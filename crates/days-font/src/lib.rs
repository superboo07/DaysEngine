//! Reader for `FONTDATA.DAT`, the glyph store behind `FILM::FontData`.
//!
//! # Layout
//!
//! ```text
//! offsets  [u32; 65536]   file offset of each glyph, 0 = no glyph
//! glyphs   ...            RLE streams, referenced by the table above
//! ```
//!
//! The table is indexed **directly by Unicode BMP code point** and is
//! `65536 * 4 = 0x40000` bytes, so the first glyph starts at `0x40000`. Offsets
//! are absolute within the file. The retail English font defines 22,420 glyphs:
//! ASCII, kana, CJK punctuation, all of CJK Unified Ideographs, and fullwidth
//! forms.
//!
//! # Glyph encoding
//!
//! Each glyph paints into a fixed [`CELL`]x[`CELL`] cell — 48x48, the size the
//! engine passes when it allocates the planes. There are **two planes**, not
//! one: a luminance plane and an alpha plane. The alpha plane is a dilated
//! version of the shape, which is how the game draws readable text over moving
//! video: the outline is a soft dark halo around a bright core.
//!
//! The stream is a byte RLE, terminated by `0x00`:
//!
//! | Byte | Meaning |
//! |---|---|
//! | `0x00` | end of glyph; the rest of the cell stays transparent |
//! | `0x01..=0x7f` | skip that many pixels |
//! | `0x80..=0xff` | a run; a second byte follows |
//!
//! For a run, with control byte `c` and the following byte `n`:
//!
//! ```text
//! length     = (n >> 4) + 1
//! luminance  = (c << 1) & 0xff
//! alpha      = (n & 0x0f) * 0x11     // nibble scaled to 0..=255
//! ```
//!
//! A glyph that ends before filling the cell simply stops: the decoder clears
//! both planes first, so trailing pixels are transparent. That is why glyph
//! streams vary in decoded length even though the cell is fixed — the pixel
//! count is *not* a reliable way to infer the cell size from the data alone.
//!
//! All of the above is transcribed from the shipped decoder (`FUN_004368c0` in
//! `SCHOOLDAYS HQ.exe`) rather than inferred from the bytes.

#![forbid(unsafe_code)]

/// Side length of a glyph cell, in pixels.
///
/// The engine allocates the glyph planes with `FontData::resize(0x30, 0x30)`.
/// Latin glyphs are drawn at roughly half width inside this full-width cell,
/// the usual arrangement for a CJK font.
pub const CELL: usize = 48;

/// Number of code points the offset table covers (the Unicode BMP).
pub const CODEPOINTS: usize = 0x1_0000;

/// Byte length of the offset table, and therefore of the file header.
pub const TABLE_BYTES: usize = CODEPOINTS * 4;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("font file is {0} bytes, too small to hold the {TABLE_BYTES}-byte offset table")]
    TooSmall(usize),
    #[error("glyph for U+{codepoint:04X} points at offset {offset}, past the end of the file")]
    OffsetOutOfRange { codepoint: u32, offset: u32 },
    #[error("glyph for U+{codepoint:04X} is not terminated")]
    Unterminated { codepoint: u32 },
    #[error("glyph for U+{codepoint:04X} overflows its {CELL}x{CELL} cell")]
    Overflow { codepoint: u32 },
}

/// One decoded glyph: two `CELL * CELL` planes.
#[derive(Clone)]
pub struct Glyph {
    /// Greyscale value per pixel. This is the text colour channel.
    pub luminance: [u8; CELL * CELL],
    /// Coverage per pixel. Dilated relative to the luminance, giving the
    /// outline that keeps dialogue legible against video.
    pub alpha: [u8; CELL * CELL],
}

impl Glyph {
    fn blank() -> Glyph {
        Glyph {
            luminance: [0; CELL * CELL],
            alpha: [0; CELL * CELL],
        }
    }

    /// Tight bounding box of the inked pixels as `(x0, y0, x1, y1)`, exclusive
    /// on the far edge, or `None` for a blank glyph such as space.
    ///
    /// Measured on the alpha plane, since that is what is actually visible.
    pub fn ink_bounds(&self) -> Option<(usize, usize, usize, usize)> {
        let (mut x0, mut y0, mut x1, mut y1) = (CELL, CELL, 0usize, 0usize);
        for y in 0..CELL {
            for x in 0..CELL {
                if self.alpha[y * CELL + x] != 0 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        (x1 > x0).then_some((x0, y0, x1, y1))
    }

    /// Premultiplied-free RGBA, taking the luminance as the colour and the
    /// alpha plane as coverage — the composite the engine's own blit performs.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CELL * CELL * 4);
        for i in 0..CELL * CELL {
            let l = self.luminance[i];
            out.extend_from_slice(&[l, l, l, self.alpha[i]]);
        }
        out
    }
}

/// A loaded `FONTDATA.DAT`.
pub struct Font {
    data: Vec<u8>,
    offsets: Vec<u32>,
}

impl Font {
    /// Parses a font file. Only the offset table is validated up front; glyphs
    /// are decoded on demand.
    pub fn parse(data: Vec<u8>) -> Result<Font, Error> {
        if data.len() < TABLE_BYTES {
            return Err(Error::TooSmall(data.len()));
        }
        let offsets = data[..TABLE_BYTES]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b))
            .collect::<Vec<u32>>();

        let defined = offsets.iter().filter(|&&o| o != 0).count();
        log::debug!("font has {defined} glyphs in {} bytes", data.len());
        Ok(Font { data, offsets })
    }

    /// True if this font defines a glyph for `c`.
    ///
    /// Code points outside the BMP are never present: the table only covers
    /// `U+0000..=U+FFFF`.
    pub fn has_glyph(&self, c: char) -> bool {
        u32::from(c)
            .try_into()
            .ok()
            .and_then(|i: usize| self.offsets.get(i))
            .is_some_and(|&o| o != 0)
    }

    pub fn glyph_count(&self) -> usize {
        self.offsets.iter().filter(|&&o| o != 0).count()
    }

    /// Decodes the glyph for `c`, or `None` if the font does not define one.
    pub fn glyph(&self, c: char) -> Result<Option<Glyph>, Error> {
        let codepoint = u32::from(c);
        let Some(&offset) = self.offsets.get(codepoint as usize) else {
            return Ok(None);
        };
        if offset == 0 {
            return Ok(None);
        }
        let start = offset as usize;
        if start >= self.data.len() {
            return Err(Error::OffsetOutOfRange { codepoint, offset });
        }
        Ok(Some(decode(&self.data[start..], codepoint)?))
    }
}

/// The decoder, transcribed from `FUN_004368c0`.
fn decode(stream: &[u8], codepoint: u32) -> Result<Glyph, Error> {
    let mut glyph = Glyph::blank();
    let mut at = 0usize;
    let mut i = 0usize;

    loop {
        let Some(&control) = stream.get(i) else {
            return Err(Error::Unterminated { codepoint });
        };
        i += 1;

        if control == 0 {
            // Everything past here stays transparent.
            return Ok(glyph);
        }

        if control < 0x80 {
            at += control as usize;
            if at > CELL * CELL {
                return Err(Error::Overflow { codepoint });
            }
            continue;
        }

        let Some(&packed) = stream.get(i) else {
            return Err(Error::Unterminated { codepoint });
        };
        i += 1;

        let run = (packed >> 4) as usize + 1;
        // The original truncates to 8 bits here; a control byte of 0x80 yields a
        // luminance of 0, which is a legitimate black pixel with real coverage.
        let luminance = control.wrapping_shl(1);
        let alpha = (packed & 0x0f) * 0x11;

        let end = at + run;
        if end > CELL * CELL {
            return Err(Error::Overflow { codepoint });
        }
        glyph.luminance[at..end].fill(luminance);
        glyph.alpha[at..end].fill(alpha);
        at = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a font file with one glyph, for testing the decoder in isolation.
    fn font_with(codepoint: u32, stream: &[u8]) -> Font {
        let mut data = vec![0u8; TABLE_BYTES];
        let offset = TABLE_BYTES as u32;
        data[codepoint as usize * 4..codepoint as usize * 4 + 4]
            .copy_from_slice(&offset.to_le_bytes());
        data.extend_from_slice(stream);
        Font::parse(data).unwrap()
    }

    #[test]
    fn skip_runs_advance_without_inking() {
        // Skip 10, then one run of 1 pixel, then end.
        let f = font_with(0x41, &[10, 0x80, 0x0f, 0x00]);
        let g = f.glyph('A').unwrap().unwrap();
        assert!(g.alpha[..10].iter().all(|&a| a == 0));
        assert_eq!(g.alpha[10], 0xff);
        assert_eq!(g.alpha[11], 0);
    }

    /// Run length is the high nibble plus one; alpha is the low nibble times 17
    /// so that 0x0f maps to a full 0xff.
    #[test]
    fn run_length_and_alpha_come_from_the_packed_byte() {
        let f = font_with(0x41, &[0xc0, 0x3a, 0x00]);
        let g = f.glyph('A').unwrap().unwrap();
        let run = (0x3a >> 4) + 1;
        assert_eq!(run, 4);
        assert!(g.alpha[..run].iter().all(|&a| a == 0x0a * 0x11));
        assert!(g.luminance[..run]
            .iter()
            .all(|&l| l == 0xc0u8.wrapping_shl(1)));
        assert_eq!(g.alpha[run], 0);
    }

    /// A control byte of 0x80 means luminance 0 — black ink, not "no ink".
    #[test]
    fn luminance_wraps_to_zero_for_the_lowest_control_byte() {
        let f = font_with(0x41, &[0x80, 0xf0, 0x00]);
        let g = f.glyph('A').unwrap().unwrap();
        assert_eq!(g.luminance[0], 0);
        assert_eq!(g.alpha[0], 0);
        // A run of 16 with zero alpha still consumed cell space.
        assert_eq!(g.alpha.iter().filter(|&&a| a != 0).count(), 0);
    }

    #[test]
    fn a_glyph_may_end_before_filling_its_cell() {
        let f = font_with(0x41, &[0x80, 0x0f, 0x00]);
        let g = f.glyph('A').unwrap().unwrap();
        assert_eq!(g.alpha.iter().filter(|&&a| a != 0).count(), 1);
    }

    #[test]
    fn undefined_code_points_have_no_glyph() {
        let f = font_with(0x41, &[0x00]);
        assert!(f.glyph('B').unwrap().is_none());
        assert!(!f.has_glyph('B'));
        // Outside the BMP the table has no entry at all.
        assert!(f.glyph('\u{1F600}').unwrap().is_none());
    }

    #[test]
    fn unterminated_glyphs_are_rejected() {
        let f = font_with(0x41, &[10, 20]);
        assert!(matches!(f.glyph('A'), Err(Error::Unterminated { .. })));
    }

    #[test]
    fn overflowing_glyphs_are_rejected() {
        // 0x7f skips repeated enough times to run past the cell.
        let mut stream = vec![0x7f; (CELL * CELL) / 0x7f + 2];
        stream.push(0x00);
        let f = font_with(0x41, &stream);
        assert!(matches!(f.glyph('A'), Err(Error::Overflow { .. })));
    }

    #[test]
    fn rejects_a_truncated_file() {
        assert!(matches!(Font::parse(vec![0; 16]), Err(Error::TooSmall(16))));
    }
}
