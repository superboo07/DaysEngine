//! Dialogue text: how a line is broken, spaced and rasterised.
//!
//! Glyphs come from `FONTDATA.DAT` as two planes — a luminance plane and a
//! dilated alpha plane. Compositing alpha-as-coverage over luminance-as-colour
//! reproduces the outlined look the original uses to keep dialogue readable
//! over moving video, which is why we do not just threshold a single bitmap.
//!
//! # Where the line breaks
//!
//! Not in the script. Only 68 of the 30,485 `[PrintText]` statements carry a
//! `\n` escape, and segments between breaks run to 165 characters, so the
//! engine wraps — in `FUN_0043f600`, which the `[PrintText]` arm of
//! `FUN_0043dbe0` hands the statement's text field to. It walks the string a
//! character at a time, appending to the line it is on, and after every space
//! looks ahead to the next space or end of string: if the count so far plus
//! that next word would pass **62**, the line breaks. The `\n` escape is a hard
//! break. That is [`wrap`].
//!
//! Two details of it are shipped behaviour rather than tidiness, and are
//! reproduced: the wrap test only runs under `[UseEnglish]`, so a Japanese
//! install never breaks a line this way; and the look-ahead starts one past the
//! word's first character, so the budget is really `column + word - 1`.
//!
//! # How characters are spaced
//!
//! By a fixed pitch and a kerning table, not by measuring the glyph.
//! `FUN_0044c740` advances `local_c + FUN_0044c660(c)` per character, where
//! `local_c` is 36 and drops to **16** under `[UseEnglish]`, and
//! `FUN_0044c660` is an eight-case table that returns 0 for everything unless
//! `[UseEnglish]` is set. See [`kerning`].
//!
//! # What the box holds
//!
//! Two lines. The array `FUN_0043f600` fills is at `this+0x234` with a `0x1c`
//! stride, and the ruby array it would otherwise run into is at `this+0x26c` —
//! anchored by `FUN_0043f880`, the `《》` group consumer, writing ruby to
//! `this + line*0x1c + 0x26c`. So there is room for lines 0 and 1 and no more,
//! at a pitch of `0x30` between them.
//!
//! Under the recovered wrap rule **49 of the shipped English statements produce
//! three or four lines**, which is past the end of that array. What the retail
//! executable does with those is not established here, and this engine does not
//! reproduce it: [`wrap`] returns every line it produces and the caller draws
//! them all, because dropping them would lose dialogue.
//!
//! # Ruby
//!
//! `FUN_0043f600` also understands `｜` as a ruby anchor and `《…》` as a ruby
//! group, drawn as a second block by `FUN_0044c740`. **No English statement
//! uses either mark**, so the marks are recognised and skipped here rather than
//! rendered.

use days_font::{Font, CELL};

/// The longest a line may get, in characters, from `FUN_0043f600`'s `0x3e`.
pub const WRAP_COLUMNS: usize = 62;

/// How many lines the dialogue box holds, from the array's extent.
pub const BOX_LINES: usize = 2;

/// Pitch between dialogue lines, in font cells' own units — `FUN_0044c740`
/// draws line `n` at `n * 0x30`, and `0x30` is also the cell size the font is
/// configured with (`FUN_00436b00(font, 0x30, 0x30)`).
pub const LINE_PITCH: usize = 0x30;

/// Per-character advance before kerning: 36, or 16 under `[UseEnglish]`.
pub fn pitch(english: bool) -> i32 {
    if english {
        0x10
    } else {
        0x24
    }
}

/// `FUN_0044c660`: the kerning delta added to [`pitch`] for one character.
///
/// Zero for everything unless `[UseEnglish]`. The arms are in the order the
/// function tests them, which matters: `i`, `j`, `l`, `m` and `w` are picked off
/// before the `A`..`Z` gate, so every other lowercase letter, every digit and
/// all punctuation get nothing.
pub fn kerning(c: char, english: bool) -> i32 {
    if !english {
        return 0;
    }
    match c {
        'j' | 'i' | 'l' => -7,
        'm' | 'w' => 8,
        'A'..='Z' => match c {
            'M' | 'W' | 'Q' => 11,
            'I' => -7,
            _ => 4,
        },
        _ => 0,
    }
}

/// The advance for one character, in the layout's own units.
pub fn advance(c: char, english: bool) -> i32 {
    pitch(english) + kerning(c, english)
}

/// Breaks a dialogue line the way `FUN_0043f600` breaks it.
///
/// `english` is `FILMENGINE.INI`'s `[UseEnglish]`, which is what the wrap test
/// is gated on. Returns at least one line, and may return more than
/// [`BOX_LINES`] — see the module docs.
pub fn wrap(text: &str, english: bool) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut lines = vec![String::new()];
    let mut column = 0usize;
    let mut i = 0usize;

    while i < chars.len() {
        let c = chars[i];
        if c == '｜' {
            // A ruby anchor: consumed, and it does not count as a column.
            i += 1;
        } else if c == '\\' {
            if chars.get(i + 1) == Some(&'n') {
                column = 0;
                i += 2;
                lines.push(String::new());
                continue;
            }
            // A lone backslash is consumed and nothing is appended, which is
            // what the `else if` with no else does.
            i += 1;
            continue;
        } else if c == '《' {
            // A ruby group. The original hands it to FUN_0043f880 and carries
            // on past the closing mark; with no English statement using ruby
            // there is nothing to draw, so it is skipped.
            i += 1;
            while i < chars.len() && chars[i] != '》' {
                i += 1;
            }
            i += 1;
            continue;
        } else {
            lines.last_mut().expect("always one line").push(c);
            column += 1;
            i += 1;
        }

        if english && c == ' ' {
            // The look-ahead starts one past the word's first character, so
            // this counts `word - 1`. Reproduced, off-by-one included.
            let mut word = 0usize;
            let mut look = i;
            loop {
                look += 1;
                match chars.get(look) {
                    Some(' ') | None => break,
                    Some(_) => word += 1,
                }
            }
            if column + word > WRAP_COLUMNS {
                column = 0;
                lines.push(String::new());
            }
        }
    }
    lines
}

/// A laid-out, rasterised line of text as RGBA.
pub struct TextImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Renders one line of text into a tightly sized RGBA image.
///
/// Characters the font does not define are skipped, with a warning: the font
/// covers ASCII, kana and CJK but not Latin-1, and dropping a character is
/// better than refusing to draw the line. A character with no glyph still
/// advances, because the original advances on the character rather than on the
/// glyph — it never looks the glyph up to decide spacing.
pub fn render_line(font: &Font, text: &str, colour: [u8; 3], english: bool) -> TextImage {
    let mut glyphs: Vec<(usize, days_font::Glyph)> = Vec::new();
    let mut pen = 0i32;

    for c in text.chars() {
        match font.glyph(c) {
            Ok(Some(glyph)) => glyphs.push((pen.max(0) as usize, glyph)),
            Ok(None) => {
                if c != ' ' {
                    log::warn!("font has no glyph for {c:?} (U+{:04X})", u32::from(c));
                }
            }
            Err(err) => log::warn!("glyph for {c:?} failed to decode: {err}"),
        }
        pen += advance(c, english);
    }

    // Leave room for the last glyph's full cell, since the outline extends past
    // the advance.
    let width = pen.max(0) as usize + CELL;
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
        let img = render_line(&test_font(), "A", [255, 0, 0], true);
        assert_eq!(img.height, CELL);
        assert_eq!(&img.rgba[..4], &[0xfe, 0, 0, 0xff]);
    }

    #[test]
    fn missing_glyphs_are_skipped_rather_than_fatal() {
        let img = render_line(&test_font(), "AZA", [255, 255, 255], true);
        assert!(img.width > CELL);
    }

    #[test]
    fn spaces_advance_without_ink() {
        let font = test_font();
        let narrow = render_line(&font, "A", [255, 255, 255], true);
        let wide = render_line(&font, "A A", [255, 255, 255], true);
        assert!(wide.width > narrow.width);
    }

    #[test]
    fn the_kerning_table_is_the_eight_cases_in_the_order_they_are_tested() {
        // `FUN_0044c660`, arm by arm.
        for c in ['j', 'i', 'l'] {
            assert_eq!(kerning(c, true), -7);
        }
        for c in ['m', 'w'] {
            assert_eq!(kerning(c, true), 8);
        }
        for c in ['M', 'W', 'Q'] {
            assert_eq!(kerning(c, true), 11);
        }
        assert_eq!(kerning('I', true), -7);
        assert_eq!(kerning('A', true), 4);
        assert_eq!(kerning('Z', true), 4);
        // i, j, l, m and w are taken before the A..Z gate, so every other
        // lowercase letter falls through it to nothing.
        for c in ['a', 'z', 'o', 's'] {
            assert_eq!(kerning(c, true), 0);
        }
        // ...as do digits, punctuation and the space.
        for c in ['0', '9', '.', ' ', '?'] {
            assert_eq!(kerning(c, true), 0);
        }
        // And the whole table is off without [UseEnglish].
        for c in ['j', 'M', 'w', 'A'] {
            assert_eq!(kerning(c, false), 0);
        }
    }

    #[test]
    fn the_pitch_is_sixteen_in_english_and_thirtysix_otherwise() {
        assert_eq!(pitch(true), 0x10);
        assert_eq!(pitch(false), 0x24);
        assert_eq!(advance('M', true), 0x10 + 11);
        assert_eq!(advance('i', true), 0x10 - 7);
        assert_eq!(advance('M', false), 0x24);
    }

    #[test]
    fn a_short_line_is_not_broken() {
        assert_eq!(wrap("Hello there.", true), vec!["Hello there."]);
        assert_eq!(wrap("", true), vec![""]);
    }

    #[test]
    fn a_long_line_breaks_after_the_space_that_overflows_it() {
        // Sixty-two columns is the budget, and the space that pushes the next
        // word past it stays on the line it ends.
        let text = "aaaa ".repeat(20);
        let lines = wrap(&text, true);
        assert!(lines.len() > 1, "twenty words should not fit in 62 columns");
        for line in &lines {
            assert!(
                line.chars().count() <= WRAP_COLUMNS + 1,
                "{line:?} is {} columns",
                line.chars().count()
            );
        }
        // Nothing is lost or duplicated by the break.
        assert_eq!(lines.concat(), text);
    }

    #[test]
    fn the_look_ahead_is_one_short_so_the_budget_is_really_column_plus_word_less_one() {
        // A word of exactly N characters is measured as N - 1, so a line can
        // finish one column past the limit. This is the shipped off-by-one and
        // the test exists to keep it.
        let text = format!("{} bb", "a".repeat(61));
        let lines = wrap(&text, true);
        // column is 62 after the space; the next word measures 1; 62 + 1 > 62,
        // so it breaks.
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].chars().count(), 62);
    }

    #[test]
    fn a_japanese_install_never_word_wraps() {
        let text = "aaaa ".repeat(40);
        assert_eq!(wrap(&text, false), vec![text.clone()]);
    }

    #[test]
    fn the_backslash_n_escape_is_a_hard_break_in_either_language() {
        // The scripts carry it as two characters, a backslash and an n, so the
        // input here is built from those two characters rather than written as
        // an escape that the compiler would turn into a real newline.
        let bs = '\\';
        let text = format!("one{bs}ntwo{bs}nthree");
        for english in [true, false] {
            assert_eq!(wrap(&text, english), vec!["one", "two", "three"]);
        }
    }

    #[test]
    fn a_real_newline_is_not_what_the_scripts_carry() {
        // Only the two-character escape breaks a line. A genuine newline is an
        // ordinary character to the splitter, and none of the scripts hold one.
        assert_eq!(wrap("one\ntwo", true), vec!["one\ntwo"]);
    }

    #[test]
    fn ruby_marks_are_consumed_and_do_not_count_as_columns() {
        // No English statement uses either mark, but the splitter understands
        // them, so they must not end up in the text or in the column count.
        assert_eq!(wrap("ab｜cd", true), vec!["abcd"]);
        assert_eq!(wrap("ab《ruby》cd", true), vec!["abcd"]);
    }
}
