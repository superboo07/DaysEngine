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
//! # The speaker is not drawn
//!
//! `[PrintText]` carries a speaker field, and it never reaches the text layer.
//! `FUN_0043dbe0`'s arm hands only the *text* field to `FUN_0043f600` for
//! wrapping, and `FUN_00431740`'s tail hands `FUN_0044c740` three things: the
//! line, its ruby, and the ruby flag. The speaker goes somewhere else entirely
//! — `FUN_00432cc0` wraps speaker and text into a `0x10`-byte record and pushes
//! it onto the list at `engine+0xac`, which is what the control bar's backlog
//! button asks for. `FUN_0044bf30` draws the lines, the ruby and the choice
//! blocks and nothing else, so there is no name box to miss.
//!
//! # Where the block sits, and how big
//!
//! Centred, along the bottom, and scaled — see [`place`] and [`Geometry`], and
//! note that none of that is a choice this engine made: the x divisor
//! `_DAT_004d13c0` is 2.0, which is what makes it a centring.
//!
//! The whole block is behind one gate: `FUN_0044bf30` draws it only when
//! `_GetDrawMessage@0` answers non-zero, and that export reads `+0xa4` of the
//! settings module — the member whose setter `FUN_10007070` persists under the
//! key `TextView`. So subtitles are the `TextView` setting, off included.
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

/// A laid-out line's width in layout units, from `FUN_0044c740`.
///
/// The twelve is the function's own: it records `local_14 + 0xc` as the line's
/// width after summing the advances.
pub fn line_width(text: &str, english: bool) -> i32 {
    text.chars().map(|c| advance(c, english)).sum::<i32>() + 0xc
}

/// Destination step between dialogue lines, `_DAT_004d6740`.
pub const LINE_STEP: f32 = 39.0;

/// Destination height of one dialogue line, `_DAT_004d6770`. The source row is
/// [`LINE_PITCH`] tall, so a line is squashed vertically by 42/48 before the
/// resolution scale is applied.
pub const LINE_HEIGHT: f32 = 42.0;

/// Gap left below the last line before scaling, `_DAT_004d6788`.
pub const BOTTOM_MARGIN: f32 = 40.0;

/// Where the dialogue block goes, per resolution.
///
/// `FUN_0044bc90` sets the four numbers together, so they travel together. The
/// scales are `0.75`, `0.96` and `1.2` — which is `0.75 * screen_width / 800`,
/// the same 1.0/1.28/1.6 ladder the rest of the UI scales by, taken off a
/// design width of 800/0.75.
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    /// `this+0x1dc` and `this+0x1e0`: the size the block is placed within.
    pub screen: (f32, f32),
    /// `this+0x1e4`: layout units to screen pixels.
    pub scale: f32,
    /// `this+0x1ec`: `-0.5` widescreen, `+74.5` in 4:3 — which is the 75-pixel
    /// letterbox of the 800x600 mode, so the block sits at the bottom of the
    /// *picture* rather than of the window.
    pub y_offset: f32,
    /// `this+0x1e8`: `48.0` when full screen and not `[UseEnglish]`, else 0.
    pub margin: f32,
    /// `FUN_0044e2e0`, the member `FILMENGINE.INI`'s `[LeftArrangement]` is
    /// read into. With it clear — the shipped value — every line is centred on
    /// its own width. With it set they all take the widest line's left edge,
    /// so the block is left-aligned as a unit.
    pub left_arrangement: bool,
}

impl Geometry {
    /// The windowed 800x450 case, which is the one this engine presents at.
    pub fn native(left_arrangement: bool) -> Geometry {
        Geometry {
            screen: (800.0, 450.0),
            scale: 0.75,
            // Widescreen; the 4:3 mode takes +74.5 instead.
            y_offset: -0.5,
            // `FUN_0044bc90` sets the margin to 48 only on the full-screen
            // path, and only outside `[UseEnglish]`; the windowed path zeroes
            // it either way.
            margin: 0.0,
            left_arrangement,
        }
    }

    /// The 4:3 800x600 case: the same 800x450 block pushed down by the
    /// letterbox.
    pub fn standard(left_arrangement: bool) -> Geometry {
        Geometry {
            y_offset: 74.5,
            ..Geometry::native(left_arrangement)
        }
    }

    /// Full screen. `small` selects 1024x576 over 1280x720, which is what
    /// `FUN_0040f0d0` answers.
    pub fn full_screen(small: bool, english: bool, left_arrangement: bool) -> Geometry {
        let (screen, scale) = if small {
            ((1024.0, 576.0), 0.96)
        } else {
            ((1280.0, 720.0), 1.2)
        };
        Geometry {
            screen,
            scale,
            y_offset: -0.5,
            margin: if english { 0.0 } else { 48.0 },
            left_arrangement,
        }
    }
}

/// Where one line of dialogue is drawn, in screen pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Places a block of dialogue lines, from `FUN_0044bf30`.
///
/// Each line's x is
///
/// ```text
/// (screen_width - line_width * scale) / 2.0 - 0.5
/// ```
///
/// — a horizontal centring, and it is one because `_DAT_004d13c0`, the divisor,
/// is 2.0. The y runs up from the bottom: the *last* line sits at
/// `screen_height - (margin + 40.0) * scale + y_offset` and each earlier line is
/// `39.0 * scale` above it, because the draw loop counts down from the last line
/// and lifts as it goes.
pub fn place(lines: &[String], english: bool, geometry: Geometry) -> Vec<Placement> {
    let Geometry {
        screen: (w, h),
        scale,
        y_offset,
        margin,
        left_arrangement,
    } = geometry;

    let widths: Vec<f32> = lines
        .iter()
        .map(|l| line_width(l, english) as f32)
        .collect();
    let centred = |width: f32| (w - width * scale) / 2.0 - 0.5;

    // With `[LeftArrangement]` set the loop keeps the running minimum of those
    // x values and gives every line the last one, so the block shares the
    // widest line's left edge.
    let x_for = |width: f32| {
        if left_arrangement {
            widths.iter().copied().map(centred).fold(w, f32::min)
        } else {
            centred(width)
        }
    };

    let bottom = h - (margin + BOTTOM_MARGIN) * scale + y_offset;
    let last = lines.len().saturating_sub(1) as f32;
    widths
        .iter()
        .enumerate()
        .map(|(n, width)| Placement {
            x: x_for(*width),
            y: bottom - (last - n as f32) * LINE_STEP * scale,
            width: width * scale,
            height: LINE_HEIGHT * scale,
        })
        .collect()
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
    fn a_line_is_centred_on_its_own_width() {
        // The x formula's divisor `_DAT_004d13c0` is 2.0, which is the whole
        // reason this is a centring rather than a margin.
        let g = Geometry::native(false);
        let lines = vec!["aa".to_string()];
        let places = place(&lines, true, g);
        let width = line_width("aa", true) as f32 * g.scale;
        assert_eq!(places[0].width, width);
        assert!((places[0].x - ((800.0 - width) / 2.0 - 0.5)).abs() < 1e-3);
        // A wider line starts further left, and the midpoints agree.
        let wide = vec!["aaaaaaaaaa".to_string()];
        let wider = place(&wide, true, g);
        assert!(wider[0].x < places[0].x);
        let mid = |p: &Placement| p.x + p.width / 2.0;
        assert!((mid(&places[0]) - mid(&wider[0])).abs() < 1e-3);
    }

    #[test]
    fn the_block_is_anchored_to_the_bottom_and_stacks_upwards() {
        let g = Geometry::native(false);
        let one = place(&["a".to_string()], true, g);
        let three = place(
            &["a".to_string(), "b".to_string(), "c".to_string()],
            true,
            g,
        );
        // The bottom line lands in the same place however many there are: the
        // draw counts down from the last line and lifts as it goes.
        assert_eq!(one[0].y, three[2].y);
        assert_eq!(one[0].y, 450.0 - BOTTOM_MARGIN * g.scale - 0.5);
        // And each earlier line is one step higher.
        let step = LINE_STEP * g.scale;
        assert!((three[1].y - (three[2].y - step)).abs() < 1e-3);
        assert!((three[0].y - (three[2].y - 2.0 * step)).abs() < 1e-3);
        // A line is drawn 42 units tall, not the 48 of its source row.
        assert_eq!(one[0].height, LINE_HEIGHT * g.scale);
    }

    #[test]
    fn left_arrangement_gives_every_line_the_widest_lines_edge() {
        let lines = vec!["a".to_string(), "aaaaaaaaaaaaaaaa".to_string()];
        let centred = place(&lines, true, Geometry::native(false));
        let aligned = place(&lines, true, Geometry::native(true));
        // Centred, the short line starts further right than the long one.
        assert!(centred[0].x > centred[1].x);
        // Left-arranged, both take the minimum, which is the long line's x.
        assert_eq!(aligned[0].x, aligned[1].x);
        assert!((aligned[0].x - centred[1].x).abs() < 1e-3);
    }

    #[test]
    fn the_scales_are_the_same_ladder_the_rest_of_the_ui_uses() {
        // 0.75, 0.96, 1.2 is 0.75 x 1.0 / 1.28 / 1.6.
        assert_eq!(Geometry::native(false).scale, 0.75);
        assert_eq!(Geometry::full_screen(true, true, false).scale, 0.96);
        assert_eq!(Geometry::full_screen(false, true, false).scale, 1.2);
        assert_eq!(
            Geometry::full_screen(true, true, false).screen,
            (1024.0, 576.0)
        );
        assert_eq!(
            Geometry::full_screen(false, true, false).screen,
            (1280.0, 720.0)
        );
        // The margin is 48 only full screen and only outside [UseEnglish].
        assert_eq!(Geometry::full_screen(false, false, false).margin, 48.0);
        assert_eq!(Geometry::full_screen(false, true, false).margin, 0.0);
        assert_eq!(Geometry::native(false).margin, 0.0);
        // 4:3 pushes the block down by the 800x600 letterbox.
        assert_eq!(Geometry::standard(false).y_offset, 74.5);
        assert_eq!(Geometry::native(false).y_offset, -0.5);
    }

    #[test]
    fn a_line_width_is_the_advances_plus_twelve() {
        assert_eq!(line_width("", true), 0xc);
        assert_eq!(line_width("M", true), 0x10 + 11 + 0xc);
        assert_eq!(line_width("ii", true), 2 * (0x10 - 7) + 0xc);
    }

    #[test]
    fn ruby_marks_are_consumed_and_do_not_count_as_columns() {
        // No English statement uses either mark, but the splitter understands
        // them, so they must not end up in the text or in the column count.
        assert_eq!(wrap("ab｜cd", true), vec!["abcd"]);
        assert_eq!(wrap("ab《ruby》cd", true), vec!["abcd"]);
    }
}
