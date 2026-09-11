//! Minimal PE resource reader.
//!
//! Just enough of the PE32 format to pull a single named resource out of
//! `SCHOOLDAYS HQ.exe`. The GPK archives are encrypted with a key that ships as
//! a `CODE` / `CIPHERCODE` resource inside the game executable, so the engine
//! reads it from the user's own install rather than embedding it.

use crate::Error;

const IMAGE_DIRECTORY_ENTRY_RESOURCE: usize = 2;

fn u16le(b: &[u8], o: usize) -> Result<u16, Error> {
    b.get(o..o + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or(Error::TruncatedPe)
}

fn u32le(b: &[u8], o: usize) -> Result<u32, Error> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(Error::TruncatedPe)
}

struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_pointer: u32,
    raw_size: u32,
}

/// Maps a relative virtual address back to a file offset.
fn rva_to_offset(sections: &[Section], rva: u32) -> Option<usize> {
    sections
        .iter()
        .find(|s| {
            rva >= s.virtual_address && rva < s.virtual_address + s.virtual_size.max(s.raw_size)
        })
        .map(|s| (rva - s.virtual_address + s.raw_pointer) as usize)
}

/// A directory entry: either a name or an ID, pointing at a subdirectory or a leaf.
struct DirEntry {
    name: Option<String>,
    id: u32,
    offset: u32,
    is_directory: bool,
}

fn read_dir_entries(pe: &[u8], res_base: usize, dir_off: usize) -> Result<Vec<DirEntry>, Error> {
    let base = res_base + dir_off;
    let named = u16le(pe, base + 12)? as usize;
    let id_count = u16le(pe, base + 14)? as usize;
    let mut out = Vec::with_capacity(named + id_count);
    for i in 0..named + id_count {
        let e = base + 16 + i * 8;
        let name_or_id = u32le(pe, e)?;
        let data = u32le(pe, e + 4)?;
        let name = if name_or_id & 0x8000_0000 != 0 {
            let n = res_base + (name_or_id & 0x7fff_ffff) as usize;
            let len = u16le(pe, n)? as usize;
            let mut units = Vec::with_capacity(len);
            for k in 0..len {
                units.push(u16le(pe, n + 2 + k * 2)?);
            }
            Some(String::from_utf16_lossy(&units))
        } else {
            None
        };
        out.push(DirEntry {
            name,
            id: name_or_id & 0x7fff_ffff,
            offset: data & 0x7fff_ffff,
            is_directory: data & 0x8000_0000 != 0,
        });
    }
    Ok(out)
}

/// Extracts the bytes of the resource at `type_name` / `res_name` from a PE image.
pub fn find_resource(pe: &[u8], type_name: &str, res_name: &str) -> Result<Vec<u8>, Error> {
    if pe.get(..2) != Some(b"MZ") {
        return Err(Error::NotPe);
    }
    let pe_off = u32le(pe, 0x3c)? as usize;
    if pe.get(pe_off..pe_off + 4) != Some(b"PE\0\0") {
        return Err(Error::NotPe);
    }
    let coff = pe_off + 4;
    let n_sections = u16le(pe, coff + 2)? as usize;
    let opt_size = u16le(pe, coff + 16)? as usize;
    let opt = coff + 20;

    // PE32 (0x10b) and PE32+ (0x20b) place the data directories at different offsets.
    let magic = u16le(pe, opt)?;
    let dir_base = match magic {
        0x10b => opt + 96,
        0x20b => opt + 112,
        _ => return Err(Error::NotPe),
    };
    let res_rva = u32le(pe, dir_base + IMAGE_DIRECTORY_ENTRY_RESOURCE * 8)?;
    if res_rva == 0 {
        return Err(Error::NoResource);
    }

    let sect_base = opt + opt_size;
    let mut sections = Vec::with_capacity(n_sections);
    for i in 0..n_sections {
        let s = sect_base + i * 40;
        sections.push(Section {
            virtual_size: u32le(pe, s + 8)?,
            virtual_address: u32le(pe, s + 12)?,
            raw_size: u32le(pe, s + 16)?,
            raw_pointer: u32le(pe, s + 20)?,
        });
    }

    let res_base = rva_to_offset(&sections, res_rva).ok_or(Error::NoResource)?;

    let by_type = read_dir_entries(pe, res_base, 0)?;
    let type_entry = by_type
        .iter()
        .find(|e| e.name.as_deref() == Some(type_name))
        .ok_or(Error::NoResource)?;
    if !type_entry.is_directory {
        return Err(Error::NoResource);
    }

    let by_name = read_dir_entries(pe, res_base, type_entry.offset as usize)?;
    let name_entry = by_name
        .iter()
        .find(|e| e.name.as_deref() == Some(res_name))
        .ok_or(Error::NoResource)?;
    if !name_entry.is_directory {
        return Err(Error::NoResource);
    }

    // Language level: take whichever language is present first.
    let by_lang = read_dir_entries(pe, res_base, name_entry.offset as usize)?;
    let leaf = by_lang.first().ok_or(Error::NoResource)?;
    let _ = leaf.id;
    let data_entry = res_base + leaf.offset as usize;
    let data_rva = u32le(pe, data_entry)?;
    let data_size = u32le(pe, data_entry + 4)? as usize;
    let off = rva_to_offset(&sections, data_rva).ok_or(Error::NoResource)?;
    pe.get(off..off + data_size)
        .map(|s| s.to_vec())
        .ok_or(Error::TruncatedPe)
}
