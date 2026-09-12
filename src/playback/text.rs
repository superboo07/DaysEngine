//! Dialogue text layout and rasterisation using the game's own font.
//!
//! Glyphs come from `FONTDATA.DAT` as two planes — a luminance plane and a
//! dilated alpha plane. Compositing alpha-as-coverage over luminance-as-colour
//! reproduces the outlined look the original uses to keep dialogue readable
//! over moving video, which is why we do not just threshold a single bitmap.

use days_font::{Font, Glyph, CELL};

/// Horizontal gap left after each glyph's ink, in cell pixels.
///
/// **Provisional.** The shipped blit takes an explicit `x` per character, so
/// the advance is decided by a caller we have not traced yet. Until then,
/// advance is measured from the glyph's own ink plus this gap, which spaces
/// proportionally and looks right, but is not guaranteed to match the original
/// pixel for pixel.
const GAP: usize = 2;

/// Advance used for a space, which has no ink to measure.
const SPACE_ADVANCE: usize = CELL / 4;

/// A laid-out, rasterised line of text as RGBA.
pub struct TextImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Measures a glyph's advance from its luminance plane.
///
/// The alpha plane is deliberately not used: it is dilated to form the outline,
/// so measuring it would space text several pixels too wide per character.
fn advance_of(glyph: &Glyph) -> usize {
    let mut right = 0usize;
    for y in 0..CELL {
        for x in (right..CELL).rev() {
            if glyph.luminance[y * CELL + x] != 0 {
                right = right.max(x + 1);
                break;
            }
        }
    }
    if right == 0 {
        SPACE_ADVANCE
    } else {
        right + GAP
    }
}

/// Renders one line of text into a tightly sized RGBA image.
///
/// Characters the font does not define are skipped, with a warning: the font
/// covers ASCII, kana and CJK but not Latin-1, and dropping a character is
/// better than refusing to draw the line.
pub fn render_line(font: &Font, text: &str, colour: [u8; 3]) -> TextImage {
    let mut glyphs: Vec<(usize, Glyph)> = Vec::new();
    let mut width = 0usize;

    for c in text.chars() {
        match font.glyph(c) {
            Ok(Some(glyph)) => {
                let advance = advance_of(&glyph);
                glyphs.push((width, glyph));
                width += advance;
            }
            Ok(None) => {
                if c == ' ' {
                    width += SPACE_ADVANCE;
                } else {
                    log::warn!("font has no glyph for {c:?} (U+{:04X})", u32::from(c));
                }
            }
            Err(err) => log::warn!("glyph for {c:?} failed to decode: {err}"),
        }
    }

    // Leave room for the last glyph's full cell, since the outline extends past
    // the advance.
    let width = width + CELL;
    let height = CELL;
    let mut rgba = vec![0u8; width * height * 4];

    for (x0, glyph) in &glyphs {
        for y in 0..CELL {
            for x in 0..CELL {
                let alpha = glyph.alpha[y * CELL + x];
                if alpha == 0 {
                    continue;
                }
                let luminance = glyph.luminance[y * CELL + x];
                let dst = ((y * width) + x0 + x) * 4;
                if dst + 4 > rgba.len() {
                    continue;
                }
                // The luminance plane is the shape; scale the requested colour
                // by it so the dark outline stays dark and the core takes the
                // text colour.
                let shade = |c: u8| ((u16::from(c) * u16::from(luminance)) / 255) as u8;
                let src = [shade(colour[0]), shade(colour[1]), shade(colour[2]), alpha];
                // `max` rather than `over`: adjacent cells overlap because the
                // outline is wider than the advance, and the shipped blit
                // composites overlapping glyphs by taking the larger value.
                for i in 0..4 {
                    rgba[dst + i] = rgba[dst + i].max(src[i]);
                }
            }
        }
    }

    TextImage {
        width,
        height,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_font() -> Font {
        // One glyph for 'A': a single fully opaque, fully bright pixel.
        let mut data = vec![0u8; days_font::TABLE_BYTES];
        let offset = days_font::TABLE_BYTES as u32;
        data[0x41 * 4..0x41 * 4 + 4].copy_from_slice(&offset.to_le_bytes());
        data.extend_from_slice(&[0xff, 0x0f, 0x00]);
        Font::parse(data).unwrap()
    }

    #[test]
    fn renders_ink_in_the_requested_colour() {
        let img = render_line(&test_font(), "A", [255, 0, 0]);
        assert_eq!(img.height, CELL);
        assert_eq!(&img.rgba[..4], &[0xfe, 0, 0, 0xff]);
    }

    #[test]
    fn missing_glyphs_are_skipped_rather_than_fatal() {
        let img = render_line(&test_font(), "AZA", [255, 255, 255]);
        assert!(img.width > CELL);
    }

    #[test]
    fn spaces_advance_without_ink() {
        let font = test_font();
        let narrow = render_line(&font, "A", [255, 255, 255]);
        let wide = render_line(&font, "A A", [255, 255, 255]);
        assert!(wide.width > narrow.width);
    }
}
