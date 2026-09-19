//! Reader for the `.GPK` archives that ship with School Days HQ and its
//! siblings (Shiny Days, Cross Days) — Overflow's "FILMEngine" packs.
//!
//! # Format
//!
//! A GPK is a blob of file data followed by an encrypted index and a 32-byte
//! footer:
//!
//! ```text
//! [ leading stub ][ entry data ... ][ encrypted index ][ 32-byte footer ]
//!
//! footer: "STKFile0PIDX"  (12 bytes)
//!         index_length    (u32 LE)
//!         "STKFile0PACKFILE" (16 bytes)
//! ```
//!
//! The index sits immediately before the footer. It is XOR'd with a repeating
//! 16-byte key, after which the first `u32` is the inflated length and the rest
//! is a raw zlib stream. The inflated index is a flat sequence of entries:
//!
//! ```text
//! name_len_utf16  u16      (in UTF-16 code units, not bytes)
//! name            [u16]    UTF-16LE, backslash-separated paths
//! reserved        [u8; 6]
//! offset          u32      absolute file offset of the entry's stored bytes
//! size            u32      total compressed size, INCLUDING the inline header
//! method          u32      FourCC, "DFLT" for deflate
//! unpacked_size   u32      inflated size, 0 if stored uncompressed
//! header_len      u8
//! header          [u8]     first `header_len` bytes of the compressed stream
//! ```
//!
//! The `header` field is the detail that trips up naive extractors. The first
//! `header_len` bytes of each entry's compressed stream live *in the index*,
//! not in the data region. So the bytes on disk at `offset` are
//! `size - header_len` long, and the real stream is `header ++ disk_bytes`.
//! Reading `size` bytes at `offset` and prepending the header — which is what
//! the GARbro-derived extractors floating around do — overruns into the next
//! entry and produces a corrupt tail.
//!
//! # Key
//!
//! The XOR key is not embedded here. It ships as a `CODE` / `CIPHERCODE`
//! resource inside the game executable, and [`Key::from_executable`] reads it
//! out of the user's own install. Some builds store a 20-byte resource whose
//! last 16 bytes are the key.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub mod pe;

const FOOTER_LEN: u64 = 32;
const MAGIC_PIDX: &[u8; 12] = b"STKFile0PIDX";
const MAGIC_PACKFILE: &[u8; 16] = b"STKFile0PACKFILE";
const KEY_LEN: usize = 16;

/// FourCC `"DFLT"`, stored little-endian, marking a deflate-compressed entry.
const METHOD_DEFLATE: u32 = u32::from_le_bytes(*b"DFLT");

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("i/o error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0} is not a GPK archive (missing STKFile0 footer)")]
    NotGpk(PathBuf),
    #[error("{path} has a {len}-byte index, larger than the archive itself")]
    IndexTooLarge { path: PathBuf, len: u64 },
    #[error("index of {0} is truncated")]
    TruncatedIndex(PathBuf),
    #[error("could not inflate {what}: {source}")]
    Inflate {
        what: String,
        #[source]
        source: miniz_oxide::inflate::DecompressError,
    },
    #[error("{what} inflated to {got} bytes, index claimed {want}")]
    SizeMismatch {
        what: String,
        got: usize,
        want: usize,
    },
    #[error("entry name in {0} is not valid UTF-16")]
    BadName(PathBuf),
    #[error("file is not a PE executable")]
    NotPe,
    #[error("PE executable is truncated")]
    TruncatedPe,
    #[error("no CODE/CIPHERCODE resource in the executable")]
    NoResource,
    #[error("CIPHERCODE resource is {0} bytes; expected 16 or 20")]
    BadKeyLength(usize),
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The 16-byte repeating XOR key guarding GPK indices.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Key([u8; KEY_LEN]);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Not secret in any meaningful sense, but printing it in logs is noise.
        write!(f, "Key(<{} bytes>)", KEY_LEN)
    }
}

impl Key {
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Key(bytes)
    }

    /// Reads the key from the `CODE` / `CIPHERCODE` resource of a game executable.
    ///
    /// This is how the engine stays legal to distribute: the key lives in the
    /// user's own `SCHOOLDAYS HQ.exe`, not in this repository.
    pub fn from_executable(exe: &Path) -> Result<Self, Error> {
        let bytes = std::fs::read(exe).map_err(io(exe))?;
        Self::from_image(&bytes)
    }

    /// Reads the key out of an executable already in memory.
    ///
    /// The same test as [`Key::from_executable`], for a caller that has the
    /// bytes rather than a path it can hand to `std::fs`. The engine reaches
    /// for this on Android, where the install is a Storage Access Framework
    /// tree and a game file has no path the C library can open.
    pub fn from_image(bytes: &[u8]) -> Result<Self, Error> {
        let res = pe::find_resource(bytes, "CODE", "CIPHERCODE")?;
        Self::from_resource(&res)
    }

    /// Interprets a raw `CIPHERCODE` resource blob.
    ///
    /// Retail builds store 16 bytes. Some store 20, where the first four are a
    /// length or version prefix and the key is the trailing 16.
    pub fn from_resource(res: &[u8]) -> Result<Self, Error> {
        let slice = match res.len() {
            KEY_LEN => res,
            20 => &res[4..20],
            n => return Err(Error::BadKeyLength(n)),
        };
        let mut key = [0u8; KEY_LEN];
        key.copy_from_slice(slice);
        Ok(Key(key))
    }

    fn decrypt(&self, data: &mut [u8]) {
        for (i, b) in data.iter_mut().enumerate() {
            *b ^= self.0[i % KEY_LEN];
        }
    }
}

/// One file inside a GPK.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Path as stored, with backslashes normalised to `/`. Case is as-stored,
    /// which is inconsistent across packs — look entries up case-insensitively.
    pub name: String,
    /// Absolute offset of this entry's stored bytes within the archive.
    pub offset: u64,
    /// Total compressed length, *including* the `header` bytes held in the index.
    pub size: u32,
    /// Inflated length, or 0 when the entry is stored uncompressed.
    pub unpacked_size: u32,
    /// Leading bytes of the compressed stream, held in the index rather than
    /// in the data region.
    pub header: Vec<u8>,
    method: u32,
}

impl Entry {
    /// True when the entry's bytes are deflate-compressed.
    pub fn is_compressed(&self) -> bool {
        self.unpacked_size != 0 && self.method == METHOD_DEFLATE
    }

    /// Bytes actually present in the data region, i.e. excluding the inline header.
    fn stored_len(&self) -> u64 {
        u64::from(self.size).saturating_sub(self.header.len() as u64)
    }

    /// Final size of this entry once decoded.
    pub fn decoded_len(&self) -> u64 {
        if self.unpacked_size != 0 {
            u64::from(self.unpacked_size)
        } else {
            u64::from(self.size)
        }
    }
}

/// An open GPK archive. Holds the index in memory and reads entry bytes lazily.
///
/// Reads go through `&self` behind a mutex so one archive can be shared with a
/// decode thread. Entries are large enough (hundreds of KB) that the lock is
/// never the bottleneck.
pub struct Archive {
    path: PathBuf,
    file: Mutex<File>,
    entries: Vec<Entry>,
    /// Lowercased name -> index into `entries`.
    by_name: HashMap<String, usize>,
}

impl Archive {
    /// Opens an archive and parses its index.
    pub fn open(path: impl AsRef<Path>, key: &Key) -> Result<Self, Error> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path).map_err(io(&path))?;
        Self::from_file(path, file, key)
    }

    /// Parses the index of an archive that is already open.
    ///
    /// `path` names it for errors and for [`Archive::path`]; it is never
    /// opened. This is the door for a caller whose files do not come from
    /// `std::fs` — on Android the install is a Storage Access Framework tree,
    /// and what a document resolves to is a file descriptor and not a path.
    /// The descriptor is still seekable, which is the whole reason a pack can
    /// be read a piece at a time there as it is everywhere else.
    pub fn from_file(path: PathBuf, file: File, key: &Key) -> Result<Self, Error> {
        let mut file = file;
        let total = file.seek(SeekFrom::End(0)).map_err(io(&path))?;
        if total < FOOTER_LEN {
            return Err(Error::NotGpk(path));
        }

        let mut footer = [0u8; FOOTER_LEN as usize];
        file.seek(SeekFrom::Start(total - FOOTER_LEN))
            .map_err(io(&path))?;
        file.read_exact(&mut footer).map_err(io(&path))?;
        if &footer[..12] != MAGIC_PIDX || &footer[16..32] != MAGIC_PACKFILE {
            return Err(Error::NotGpk(path));
        }

        let index_len = u64::from(u32::from_le_bytes([
            footer[12], footer[13], footer[14], footer[15],
        ]));
        if index_len + FOOTER_LEN > total {
            return Err(Error::IndexTooLarge {
                path,
                len: index_len,
            });
        }

        let mut raw = vec![0u8; index_len as usize];
        file.seek(SeekFrom::Start(total - FOOTER_LEN - index_len))
            .map_err(io(&path))?;
        file.read_exact(&mut raw).map_err(io(&path))?;
        key.decrypt(&mut raw);

        let entries = parse_index(&path, &raw)?;
        let by_name = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.name.to_ascii_lowercase(), i))
            .collect();

        log::debug!("opened {} with {} entries", path.display(), entries.len());
        Ok(Archive {
            path,
            file: Mutex::new(file),
            entries,
            by_name,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Looks up an entry by path, case-insensitively.
    ///
    /// Case matters here: the `.INI` files reference `System/Title/TitleBase.png`
    /// while the pack index stores `TITLE/TITLEBASE.PNG`.
    pub fn find(&self, name: &str) -> Option<&Entry> {
        let key = name.replace('\\', "/").to_ascii_lowercase();
        self.by_name.get(&key).map(|&i| &self.entries[i])
    }

    /// Reads and decodes a single entry.
    pub fn read(&self, entry: &Entry) -> Result<Vec<u8>, Error> {
        let stored = entry.stored_len();
        let mut buf = Vec::with_capacity(entry.header.len() + stored as usize);
        buf.extend_from_slice(&entry.header);
        {
            let mut file = self.file.lock().expect("archive mutex poisoned");
            file.seek(SeekFrom::Start(entry.offset))
                .map_err(io(&self.path))?;
            let mut tail = vec![0u8; stored as usize];
            file.read_exact(&mut tail).map_err(io(&self.path))?;
            buf.extend_from_slice(&tail);
        }

        if !entry.is_compressed() {
            return Ok(buf);
        }

        let out = miniz_oxide::inflate::decompress_to_vec_zlib(&buf).map_err(|source| {
            Error::Inflate {
                what: entry.name.clone(),
                source,
            }
        })?;
        if out.len() != entry.unpacked_size as usize {
            return Err(Error::SizeMismatch {
                what: entry.name.clone(),
                got: out.len(),
                want: entry.unpacked_size as usize,
            });
        }
        Ok(out)
    }

    /// Convenience: look up by name and read in one step.
    pub fn read_named(&self, name: &str) -> Option<Result<Vec<u8>, Error>> {
        let entry = self.find(name)?;
        Some(self.read(entry))
    }
}

fn parse_index(path: &Path, raw: &[u8]) -> Result<Vec<Entry>, Error> {
    if raw.len() < 4 {
        return Err(Error::TruncatedIndex(path.to_path_buf()));
    }
    let want = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
    let index = miniz_oxide::inflate::decompress_to_vec_zlib(&raw[4..]).map_err(|source| {
        Error::Inflate {
            what: format!("{} index", path.display()),
            source,
        }
    })?;
    if index.len() != want {
        return Err(Error::SizeMismatch {
            what: format!("{} index", path.display()),
            got: index.len(),
            want,
        });
    }

    let mut entries = Vec::new();
    let mut c = Cursor::new(&index, path);
    // A zero-length name terminates the index; trailing padding after it is normal.
    while c.remaining() >= 2 {
        let name_units = c.u16()? as usize;
        if name_units == 0 {
            break;
        }
        let name_bytes = c.take(name_units * 2)?;
        let units: Vec<u16> = name_bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| u16::from_le_bytes(*p))
            .collect();
        let name = String::from_utf16(&units)
            .map_err(|_| Error::BadName(path.to_path_buf()))?
            .replace('\\', "/");

        c.take(6)?; // reserved; zero in every retail pack observed
        let offset = u64::from(c.u32()?);
        let size = c.u32()?;
        let method = c.u32()?;
        let unpacked_size = c.u32()?;
        let header_len = c.u8()? as usize;
        let header = c.take(header_len)?.to_vec();

        entries.push(Entry {
            name,
            offset,
            size,
            unpacked_size,
            header,
            method,
        });
    }
    Ok(entries)
}

/// Bounds-checked forward reader over the inflated index.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    path: &'a Path,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8], path: &'a Path) -> Self {
        Cursor { data, pos: 0, path }
    }
    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let s = self
            .data
            .get(self.pos..self.pos + n)
            .ok_or_else(|| Error::TruncatedIndex(self.path.to_path_buf()))?;
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Error> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_accepts_both_resource_layouts() {
        let sixteen = [0xABu8; 16];
        assert_eq!(Key::from_resource(&sixteen).unwrap(), Key(sixteen));

        let mut twenty = vec![0x00, 0x01, 0x02, 0x03];
        twenty.extend_from_slice(&sixteen);
        assert_eq!(Key::from_resource(&twenty).unwrap(), Key(sixteen));

        assert!(matches!(
            Key::from_resource(&[0u8; 8]),
            Err(Error::BadKeyLength(8))
        ));
    }

    #[test]
    fn decrypt_is_its_own_inverse() {
        let key = Key([0x5A; 16]);
        let original = b"STKFile0PIDX and then some payload".to_vec();
        let mut buf = original.clone();
        key.decrypt(&mut buf);
        assert_ne!(buf, original);
        key.decrypt(&mut buf);
        assert_eq!(buf, original);
    }

    /// The inline header is carved out of the compressed stream, not duplicated
    /// alongside it. Getting this backwards is the bug in every ported GARbro
    /// extractor we have seen.
    #[test]
    fn stored_len_excludes_inline_header() {
        let entry = Entry {
            name: "x".into(),
            offset: 0,
            size: 70,
            unpacked_size: 83,
            header: vec![0u8; 8],
            method: METHOD_DEFLATE,
        };
        assert_eq!(entry.stored_len(), 62);
        assert!(entry.is_compressed());
    }
}
