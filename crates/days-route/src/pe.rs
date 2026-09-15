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

/// What `_GetVersionToRoute@4` hands back, in the form the module returns it.
///
/// Both titles' export is the same shape — ask the object at `[edx+0x34]`, then
/// pick one of two constants — and both pick between the same two values. They
/// differ in type, which is what decides how a save slot spells its tag-1
/// version field:
///
/// ```text
/// RouteProcSDHQ.dll @ 0x10006840    non-zero -> FLD [0.01]   zero -> FLD1
/// RouteProcSD.dll   @ 0x10005b30    non-zero -> L"0.01"      zero -> L"1.0"
/// ```
///
/// Recovered from the export's own prologue, so no address is written down
/// here. `None` for a module whose export does not have this shape, which is
/// a module this has not been recovered against rather than a broken one.
#[derive(Debug, Clone, PartialEq)]
pub enum RouteVersion {
    /// Returned in `ST(0)`. The zero branch is `FLD1`, so `1.0`.
    Float(f32),
    /// Returned in `EAX` as a pointer to a wide string.
    Text(String),
}

impl Image<'_> {
    /// The version the route module reports for the retail path.
    ///
    /// The export tests the result of `call [edx+0x34]` and takes one of two
    /// constants. Every slot in either install holds the **zero** branch, so
    /// that is the one recovered here: `FLD1` for the float form, and the
    /// second string pointer for the text form.
    pub fn route_version(&self) -> Option<RouteVersion> {
        let at = *self.exports.get("_GetVersionToRoute@4")?;
        // The common prologue, up to the branch: push ebp; mov ebp,esp;
        // mov eax,[ebp+8]; mov edx,[eax]; mov ecx,[ebp+8]; mov eax,[edx+0x34];
        // call eax; test eax,eax; jz short.
        const HEAD: [u8; 18] = [
            0x55, 0x8b, 0xec, 0x8b, 0x45, 0x08, 0x8b, 0x10, 0x8b, 0x4d, 0x08, 0x8b, 0x42, 0x34,
            0xff, 0xd0, 0x85, 0xc0,
        ];
        let start = self.at(at)?;
        if self.bytes.get(start..start + HEAD.len())? != HEAD {
            return None;
        }
        // `74 xx` is the jump taken when the call answered zero.
        let after = start + HEAD.len();
        if *self.bytes.get(after)? != 0x74 {
            return None;
        }
        let body = after + 2;
        match *self.bytes.get(body)? {
            // d9 05 <imm32> = FLD dword ptr [imm32]; the zero branch that
            // follows it is d9 e8 = FLD1.
            0xd9 => {
                let rest = body + 6;
                (self.bytes.get(rest..rest + 4)? == [0xeb, 0x02, 0xd9, 0xe8])
                    .then_some(RouteVersion::Float(1.0))
            }
            // b8 <imm32> = mov eax,imm32; the zero branch is the second one.
            0xb8 => {
                let rest = body + 5;
                if self.bytes.get(rest..rest + 2)? != [0xeb, 0x05] {
                    return None;
                }
                if *self.bytes.get(rest + 2)? != 0xb8 {
                    return None;
                }
                let va = u32le(self.bytes, rest + 3)?;
                Some(RouteVersion::Text(self.wide_ascii(va)?))
            }
            _ => None,
        }
    }
}

impl Image<'_> {
    /// How many patch overlays the route module says one pack may carry.
    ///
    /// A pack is not one file. `FUN_004413c0` opens `<Directory><Pack><FileExtend>`
    /// and then layers `_GetPatchMax@0()` overlays over it, each named by
    /// `_SetPackName@16` — `swprintf_s(buf, len, L"%s.%03d", name, i)` — where
    /// `name` already carries the extension, so the files are
    /// `System.GPK.000` .. `System.GPK.009`. `docs/FORMATS.md` has the whole
    /// mount sequence and the lookup order the overlays are searched in.
    ///
    /// The export is a constant return in both titles, so it is read out of
    /// its own code rather than written down:
    ///
    /// ```text
    /// 55 8b ec                push ebp; mov ebp,esp
    /// b8 0a 00 00 00          mov eax, 10
    /// 5d c3                   pop ebp; ret
    /// ```
    ///
    /// `None` for a module that does not export it, or whose export is not
    /// that shape — a module this has not been recovered against rather than
    /// a broken one.
    pub fn patch_max(&self) -> Option<u32> {
        let at = *self.exports.get("_GetPatchMax@0")?;
        let start = self.at(at)?;
        if self.bytes.get(start..start + 4)? != [0x55, 0x8b, 0xec, 0xb8] {
            return None;
        }
        if self.bytes.get(start + 8..start + 10)? != [0x5d, 0xc3] {
            return None;
        }
        u32le(self.bytes, start + 4)
    }
}

impl Image<'_> {
    /// The scenes that have a second, "Radish uniform" recording, out of
    /// `_CheckUniformBlock@4`.
    ///
    /// `RouteProcSD.dll` exports one more entry point than
    /// `RouteProcSDHQ.dll` does, and it is a plain table search:
    ///
    /// ```text
    /// _CheckUniformBlock(name):
    ///     if wcscmp(name, L"") == 0: return 0
    ///     for i in 0 .. 0x120:
    ///         if wcsstr(name, table[i]): return 1
    ///     return 0
    /// ```
    ///
    /// It is a **substring** test against 288 bare scene names —
    /// `02-22-B04`, not `02/02-22-B04` — and the executable uses the answer
    /// to decide whether to swap the script for its `Z` twin; see
    /// `Progress::uniform_block` for that half.
    ///
    /// Both the count and the table's address are read out of the export's own
    /// code — the `cmp dword ptr [ebp-4], imm32` that bounds the loop and the
    /// `mov eax, [edx*4 + imm32]` that indexes it — so no address is written
    /// down here. An empty vector for a module that does not export it, which
    /// is every `School Days HQ` install: that title ships no `Z` scripts.
    pub fn uniform_block(&self) -> Vec<String> {
        let Some(&at) = self.exports.get("_CheckUniformBlock@4") else {
            return Vec::new();
        };
        let Some(start) = self.at(at) else {
            return Vec::new();
        };
        // The whole function is 0x5e bytes of straight-line code at fixed
        // offsets. Everything below is matched; the two `call` displacements,
        // the `L""` pointer and the two short jumps are the only bytes that
        // could differ in another build, so they are skipped.
        const SHAPE: [(usize, &[u8]); 8] = [
            (0x00, &[0x55, 0x8b, 0xec, 0x51, 0x68]),
            (0x09, &[0x8b, 0x45, 0x08, 0x50, 0xe8]),
            (0x12, &[0x83, 0xc4, 0x08, 0x85, 0xc0, 0x74]),
            (0x19, &[0xc7, 0x45, 0xfc, 0x00, 0x00, 0x00, 0x00, 0xeb]),
            (
                0x22,
                &[0x8b, 0x4d, 0xfc, 0x83, 0xc1, 0x01, 0x89, 0x4d, 0xfc],
            ),
            (0x2b, &[0x81, 0x7d, 0xfc]),
            (0x32, &[0x7d]),
            (0x34, &[0x8b, 0x55, 0xfc, 0x8b, 0x04, 0x95]),
        ];
        for (off, want) in SHAPE {
            if self.bytes.get(start + off..start + off + want.len()) != Some(want) {
                return Vec::new();
            }
        }
        let (Some(count), Some(table)) = (
            u32le(self.bytes, start + 0x2e),
            u32le(self.bytes, start + 0x3a),
        ) else {
            return Vec::new();
        };
        (0..count)
            .map_while(|i| {
                let entry = self.u32(table.checked_add(i.checked_mul(4)?)?)?;
                self.wide_ascii(entry)
            })
            .collect()
    }
}
