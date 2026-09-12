//! Just enough of PE32 to read the DLL: the section table, the export
//! directory, and the strings a virtual address points at.
//!
//! Nothing here is specific to the route tables — it is the mapping every
//! other module in this crate needs to turn an address in the image into
//! bytes in the file. The image base and every section bound are read from
//! the file rather than assumed, so a differently based build still resolves.

use crate::Error;

/// One PE section, reduced to the mapping this crate needs.
#[derive(Debug, Clone, Copy)]
pub struct Section {
    /// Virtual address of the section's first byte.
    pub va: u32,
    /// Bytes the section occupies once loaded.
    pub vsize: u32,
    /// Offset of the section's first byte in the file.
    pub raw: u32,
    /// Bytes the section occupies in the file, which can be less than `vsize`.
    pub rsize: u32,
}

pub fn u16le(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

pub fn u32le(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// A 32-bit PE image, with the lookups the decoder does constantly.
#[derive(Debug, Clone)]
pub struct Image<'a> {
    pub bytes: &'a [u8],
    pub base: u32,
    pub sections: Vec<Section>,
    /// The export directory, by decorated name.
    pub exports: std::collections::BTreeMap<String, u32>,
}

impl<'a> Image<'a> {
    /// Reads the image base, section table and export directory.
    pub fn parse(dll: &'a [u8]) -> Result<Image<'a>, Error> {
        if dll.get(..2) != Some(b"MZ") {
            return Err(Error::NotPe("no MZ signature"));
        }
        let pe = u32le(dll, 0x3c).ok_or(Error::NotPe("truncated at e_lfanew"))? as usize;
        if dll.get(pe..pe + 4) != Some(b"PE\0\0") {
            return Err(Error::NotPe("no PE signature"));
        }
        let count = u16le(dll, pe + 6).ok_or(Error::NotPe("truncated file header"))? as usize;
        let opt_size = u16le(dll, pe + 20).ok_or(Error::NotPe("truncated file header"))? as usize;
        let opt = pe + 24;
        if u16le(dll, opt) != Some(0x10b) {
            return Err(Error::NotPe("not a 32-bit PE32 image"));
        }
        let base = u32le(dll, opt + 28).ok_or(Error::NotPe("truncated optional header"))?;

        let table = opt + opt_size;
        let mut sections = Vec::with_capacity(count);
        for i in 0..count {
            let h = table + i * 40;
            sections.push(Section {
                va: base + u32le(dll, h + 12).ok_or(Error::NotPe("truncated section table"))?,
                vsize: u32le(dll, h + 8).ok_or(Error::NotPe("truncated section table"))?,
                rsize: u32le(dll, h + 16).ok_or(Error::NotPe("truncated section table"))?,
                raw: u32le(dll, h + 20).ok_or(Error::NotPe("truncated section table"))?,
            });
        }

        let mut img = Image {
            bytes: dll,
            base,
            sections,
            exports: Default::default(),
        };
        img.exports = img.read_exports(opt);
        Ok(img)
    }

    /// The export directory, if the image has one. An image without exports is
    /// not an error here — only the callers that need a named export care.
    fn read_exports(&self, opt: usize) -> std::collections::BTreeMap<String, u32> {
        let mut out = std::collections::BTreeMap::new();
        let dirs = u32le(self.bytes, opt + 92).unwrap_or(0);
        if dirs == 0 {
            return out;
        }
        let Some(rva) = u32le(self.bytes, opt + 96).filter(|&r| r != 0) else {
            return out;
        };
        let Some(dir) = self.at(self.base + rva) else {
            return out;
        };
        let (Some(names), Some(fns), Some(name_ptrs), Some(ordinals)) = (
            u32le(self.bytes, dir + 24),
            u32le(self.bytes, dir + 28),
            u32le(self.bytes, dir + 32),
            u32le(self.bytes, dir + 36),
        ) else {
            return out;
        };
        for i in 0..names as usize {
            let entry = || {
                let np = u32le(self.bytes, self.at(self.base + name_ptrs)? + i * 4)?;
                let at = self.at(self.base + np)?;
                let end = self.bytes[at..].iter().position(|&c| c == 0)? + at;
                let name = std::str::from_utf8(&self.bytes[at..end]).ok()?;
                let ord = u16le(self.bytes, self.at(self.base + ordinals)? + i * 2)? as usize;
                let va = u32le(self.bytes, self.at(self.base + fns)? + ord * 4)?;
                Some((name.to_owned(), self.base + va))
            };
            if let Some((n, va)) = entry() {
                out.insert(n, va);
            }
        }
        out
    }

    /// Where a virtual address lands in the file, if it is backed by file
    /// bytes.
    ///
    /// A section's virtual size can exceed what the file holds — the tail is
    /// zero filled at load — so an address past `rsize` has no bytes to read
    /// and is refused rather than read from the next section.
    pub fn at(&self, va: u32) -> Option<usize> {
        self.sections.iter().find_map(|s| {
            let off = va.checked_sub(s.va)?;
            (off < s.vsize && off < s.rsize).then_some((s.raw + off) as usize)
        })
    }

    pub fn u8(&self, va: u32) -> Option<u8> {
        self.bytes.get(self.at(va)?).copied()
    }

    pub fn u32(&self, va: u32) -> Option<u32> {
        u32le(self.bytes, self.at(va)?)
    }

    /// The NUL-terminated UTF-16LE string at `va`, if it is printable ASCII.
    ///
    /// Every string the route code passes around — key names, script paths —
    /// is ASCII, so refusing anything else is what keeps the neighbouring
    /// arrays of narrow strings from being read as wide ones.
    pub fn wide_ascii(&self, va: u32) -> Option<String> {
        self.wide_ascii_at(self.at(va)?)
    }

    pub fn wide_ascii_at(&self, at: usize) -> Option<String> {
        /// The longest a string is allowed to run before we stop believing it
        /// is one of the DLL's. The real names are twelve characters.
        const MAX: usize = 32;
        let mut out = String::new();
        let mut o = at;
        loop {
            let u = u16le(self.bytes, o)?;
            if u == 0 {
                return Some(out);
            }
            if !(0x20..0x7f).contains(&u) || out.len() == MAX {
                return None;
            }
            out.push(u as u8 as char);
            o += 2;
        }
    }
}
