//! The backlog screen: the lines the player has already been shown.
//!
//! The control bar's third menu button asks host `+0xf8(3)`, and
//! `setSystemInit`'s case 3 selects the module object `DAT_1004ffc8`. That
//! object's static-init thunk `FUN_10038360` calls the constructor
//! `FUN_10001cb0`, which installs `MENU::BackLogView::vftable` — so case 3 is
//! this screen, and the bar's third button raises it.
//!
//! `SysMenuSD.dll` holds the same class with the same layout constants: the
//! init at `FUN_10003a60` against School Days HQ's `FUN_100039a0`, host at
//! `+0xb8` rather than `+0x90` and every slot moved by this title's shift. One
//! implementation serves both.
//!
//! # Where the lines come from
//!
//! The engine keeps them itself. `FUN_0043dbe0`'s `[PrintText]` arm pushes a
//! `FILM::BLog_LogMessage` record — `FUN_00432310`, a vtable, the type `2`, the
//! script it came from and the line's index in it — onto the vector at
//! `engine+0xac +0x08`, through `FUN_00432cc0`. `FUN_00434820` reads a record
//! back as a speaker and a text, so a record carries a line by reference rather
//! than by value; this engine stores the pair, which is the same thing.
//!
//! That vector is only ever printed lines. The same object's `+0x20` holds the
//! position record `FUN_00432a10` writes when a script starts, `+0x78` the
//! story points, and those are what a save slot serialises — which is why
//! `Save/SaveFileNNN.DAT` has tags 1, 3 and 4 and no text tag. `+0x98` is the
//! backlog's length at the last position record, and `FUN_004348e0` erases
//! everything from there on, so jumping back to a story point drops the lines
//! logged since it. Host slot `+0x2c` (`FUN_0042c0e0` → `FUN_004349f0`) empties
//! the whole thing.
//!
//! # What the screen draws
//!
//! Not into the screen's art: into a buffer of its own, which one sprite shows
//! over everything else. `FUN_100039a0` builds it `(0x800, 0x400, 0x208888)` —
//! `0x1000` wide when ruby is on, see [`RUBY_SETTING`] — and gives the sprite
//! that shows it a source rect of `(0, 0, 1280, 720)` in the buffer and a
//! destination of the whole screen, `(-0.5, -0.5, 801, 451)` in layout space.
//! So [`VIEW`] of the buffer stretches over [`DEST`], and everything below is
//! in the buffer's own pixels.
//!
//! `FUN_10003600` fills it. The entry the player is on is centred on
//! [`CENTRE`]; earlier entries are stacked upwards until one starts above the
//! buffer, later ones downwards until one starts past [`Flow::limit`]. Each
//! entry is [`height`] tall, its speaker and then its wrapped lines drawn a
//! row apart by `FUN_100022a0`.
//!
//! # Two flows
//!
//! `FILMENGINE.INI`'s `[BackLogType]` picks between two whole screens, art and
//! all: `System/BackLog/BackLog_Horizon` and `..._Vertical`. Both retail
//! installs ship `0`, the horizontal one. See [`Flow`].

use crate::playback::text::kerning;
use crate::ui::screen::{Screen, WidgetState};
use days_font::Font;
use days_ui::Image;

/// The buffer the screen draws into, `FUN_100039a0`'s `(0x800, 0x400)`.
///
/// The width is `0x1000` instead when ruby is on — see [`RUBY_SETTING`].
pub const BUFFER: (u32, u32) = (0x800, 0x400);

/// The buffer's width when ruby is on, which is the only thing that setting
/// changes about the buffer.
pub const BUFFER_WIDTH_WITH_RUBY: u32 = 0x1000;

/// The part of the buffer the sprite shows, as `FUN_100039a0` sets its source
/// rect: `texture->u(1280.0)` by `texture->v(720.0)`.
pub const VIEW: (u32, u32) = (1280, 720);

/// Where that goes on screen, in the 800x450 layout space every other screen
/// is placed in: `sprite->setDest(-0.5, -0.5, 801, 451)` scaled, with the
/// letterbox added to the y the way a widget's is.
pub const DEST: (f32, f32, f32, f32) = (-0.5, -0.5, 801.0, 451.0);

/// The buffer row the entry the player is on is centred on, `FUN_10003600`'s
/// `0x17c`.
pub const CENTRE: i32 = 0x17c;

/// An entry's height before its rows are counted, `FUN_100033c0`'s `+ 0x7e`.
pub const ENTRY_BASE: i32 = 0x7e;

/// What each row of an entry adds to its height, `FUN_100033c0`'s `0x24`. The
/// speaker is a row for this purpose and so is each wrapped line.
pub const ENTRY_ROW: i32 = 0x24;

/// The first row's offset inside an entry, `FUN_100022a0`'s `param_1 + 0x15`.
pub const FIRST_ROW: i32 = 0x15;

/// The step from one drawn row to the next, `FUN_100022a0`'s `0x39`. Larger
/// than [`ENTRY_ROW`], which is what leaves a gap between entries.
pub const ROW_STEP: i32 = 0x39;

/// How far above its row a glyph sits, `FUN_100022a0`'s `param_1 + -6`.
pub const GLYPH_LIFT: i32 = 6;

/// Where a speaker starts across the row: `FUN_100022a0` runs its pen from
/// `0x4b` and draws at `pen - 0x4b`.
pub const SPEAKER_X: i32 = 0;

/// Where a line of text starts across the row: the pen runs from `0x8c` and
/// draws at `pen - 0x1e`, so the text is indented by the difference.
pub const LINE_X: i32 = 0x8c - 0x1e;

/// How many wrapped lines an entry holds, from the array `FUN_10002a90` fills:
/// `this + line * 0x8c + 0x18c`, and the loop breaks once the count passes 7.
pub const MAX_LINES: usize = 8;

/// The setting that turns ruby on, `FILMENGINE.INI`'s `[AgateUsing]` overridden
/// by `Config.DAT`'s `UseAgate`, read back through host slot `+0x60`.
///
/// With it set, `FUN_100039a0` widens the buffer to
/// [`BUFFER_WIDTH_WITH_RUBY`] and builds 26 more sprites, and `FUN_100026b0`
/// draws the ruby `FUN_10002eb0` laid out. **That drawing is not implemented
/// here.** Both retail installs ship `[AgateUsing]="0"` and `UseAgate` `0`, and
/// no `[PrintText]` in either title carries a ruby mark — 0 of 30,485 in School
/// Days HQ and 0 of 45,015 in Shiny Days — so there is nothing shipped to draw.
/// The marks are still recognised by [`wrap`], because they change where the
/// line breaks whether or not the ruby is drawn.
pub const RUBY_SETTING: &str = "UseAgate";

/// Which way the screen runs, from `FILMENGINE.INI`'s `[BackLogType]` through
/// host slot `+0x64`.
///
/// It is not a layout switch inside one screen: `FUN_10003820` and
/// `FUN_100039a0` pick a different hit map, a different base, a different chip
/// sheet and a different widget table from it. Both retail installs ship `0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// `[BackLogType]="0"` — `System/BackLog/BackLog_Horizon`, rows running
    /// down the plate.
    Horizontal,
    /// Anything else — `System/BackLog/BackLog_Vertical`, columns running
    /// right to left.
    Vertical,
}

impl Flow {
    /// The flow `[BackLogType]` selects.
    pub fn from_setting(value: i64) -> Flow {
        match value {
            0 => Flow::Horizontal,
            _ => Flow::Vertical,
        }
    }

    /// The path stem's variant, which is also how the module spells it.
    pub fn variant(self) -> &'static str {
        match self {
            Flow::Horizontal => "Horizon",
            Flow::Vertical => "Vertical",
        }
    }

    /// How many characters fit on a line, `DAT_10050898`.
    ///
    /// `FUN_10002a90` and `FUN_100033c0` both set it, from the same two asks.
    pub fn columns(self, english: bool) -> usize {
        match (self, english) {
            (Flow::Vertical, _) => 0xd,
            (Flow::Horizontal, true) => 0x30,
            (Flow::Horizontal, false) => 0x1b,
        }
    }

    /// How far along the flow axis a row may be drawn, `DAT_100508b0`, set by
    /// `FUN_100039a0` from the same two asks.
    pub fn limit(self, english: bool) -> i32 {
        match (self, english) {
            (Flow::Vertical, _) => 0x4b6,
            (Flow::Horizontal, true) => 0x4c9,
            (Flow::Horizontal, false) => 0x2a9,
        }
    }
}

/// The advance between characters before kerning, `FUN_100022a0`'s `local_1c`:
/// `0x24`, or `0x17` under `[UseEnglish]`.
///
/// The dialogue box uses the same table over a different pitch — see
/// [`crate::playback::text::pitch`], which is `0x10` there. The kerning itself
/// is one table: `FUN_100021c0` in the menu module and `FUN_0044c660` in the
/// executable have the same arms in the same order, so [`kerning`] serves both.
pub fn pitch(english: bool) -> i32 {
    if english {
        0x17
    } else {
        0x24
    }
}

/// The advance for one character on the backlog's rows.
pub fn advance(c: char, english: bool) -> i32 {
    pitch(english) + kerning(c, english)
}

/// Characters that may not start a line, `DAT_1004300c` through
/// `FUN_10002070` — the kinsoku set the Japanese wrap tests the incoming
/// character against.
const NO_LINE_START: [char; 17] = [
    '、', '。', '？', '！', '）', '」', '』', 'ー', '\u{3000}', '・', '…', ' ', '.', ',', '!',
    '\'', '?',
];

/// Characters that may not end a line, `DAT_10043120` through `FUN_100020c0`.
const NO_LINE_END: [char; 4] = ['（', '「', '『', '―'];

/// Characters the vertical flow nudges rather than rotates, `DAT_10043128`:
/// `FUN_10002110` moves the pen and the row back by `0x12` for these.
const VERTICAL_NUDGE: [char; 2] = ['、', '。'];

/// Characters the vertical flow substitutes, `DAT_1004312c` to `DAT_10043134`.
const VERTICAL_ROTATE: [(char, char); 3] = [('ー', '｜'), ('…', '：'), ('～', '｜')];

/// One line of the backlog: who spoke and what they said.
///
/// The pair a `FILM::BLog_LogMessage` record resolves to through
/// `FUN_00434820`. A record whose type is not `2` answers nothing and measures
/// zero, but the vector this comes from holds only type `2`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Entry {
    pub speaker: String,
    pub text: String,
}

/// Breaks one entry's text the way `FUN_10002a90` breaks it.
///
/// Not the same rule as the dialogue box's [`crate::playback::text::wrap`], and
/// deliberately not shared with it: the budget is `>=` rather than `>`, the
/// look-ahead starts at the character in hand rather than one past it, `\n` is
/// dropped instead of breaking the line, and the Japanese path applies kinsoku
/// where the dialogue box never breaks a Japanese line at all.
///
/// - `｜` is a ruby anchor: consumed, and it does not take a column.
/// - `《…》` is a ruby group: consumed whole, up to and including the closing
///   mark. What it would draw is [`RUBY_SETTING`]'s business.
/// - `\n` is consumed and does **not** break the line.
/// - Under `[UseEnglish]`, a character whose predecessor was a space breaks the
///   line first if the column plus the run up to the next space reaches
///   [`Flow::columns`].
/// - Otherwise the line breaks once the column reaches the limit, unless the
///   character in hand may not start a line or its predecessor may not end one.
///
/// At most [`MAX_LINES`] lines come back.
///
/// **One divergence, written down rather than reproduced.** `FUN_10002a90`
/// does not advance its index for a `\` that is not followed by `n`, so such a
/// line spins forever. No shipped `[PrintText]` in either title contains one —
/// 0 of 30,485 in School Days HQ and 0 of 45,015 in Shiny Days, and none
/// carries a ruby mark either — so nothing in the retail data reaches it. Here
/// the backslash is consumed, which is what the dialogue box's `FUN_0043f600`
/// does with the same input.
pub fn wrap(text: &str, flow: Flow, english: bool) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let columns = flow.columns(english);
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut column = 0usize;
    let mut i = 0usize;

    while i < chars.len() {
        let c = chars[i];
        if c == '｜' {
            i += 1;
            continue;
        }
        if c == '\\' {
            // The shipped function only steps past a `\n` pair; see above.
            i += if chars.get(i + 1) == Some(&'n') { 2 } else { 1 };
            continue;
        }
        if c == '《' {
            // `FUN_10002eb0` scans to the closing mark and returns its own
            // length plus one, and the caller steps one further, so the group
            // is consumed whole.
            i += 1;
            while i < chars.len() && chars[i] != '》' {
                i += 1;
            }
            i += 1;
            continue;
        }

        let breaks = if english {
            // The test is on the *previous* character being a space, so the
            // break lands before the word rather than after the space.
            i > 0 && chars[i - 1] == ' ' && {
                let word = chars[i..].iter().take_while(|&&c| c != ' ').count();
                column + word >= columns
            }
        } else {
            // `FUN_10002a90` indexes the source by the column rather than by
            // the source index for the second of these, which is the same
            // character only on the first line. Reproduced: the two walk apart
            // once a line has broken, and there is no retail Japanese screen
            // to check a correction against.
            column >= columns
                && !NO_LINE_START.contains(&c)
                && column > 0
                && !NO_LINE_END.contains(&chars[column - 1])
        };

        if breaks {
            lines.push(std::mem::take(&mut line));
            column = 0;
            if lines.len() >= MAX_LINES {
                return lines;
            }
        }

        line.push(c);
        column += 1;
        i += 1;
    }

    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// How tall one entry is, from `FUN_100033c0`'s return: `0x7e`, plus
/// [`ENTRY_ROW`] for a non-empty speaker and [`ENTRY_ROW`] for each wrapped
/// line.
///
/// Larger than what the entry draws — [`ROW_STEP`] per row from [`FIRST_ROW`]
/// — which is the gap between one entry and the next.
pub fn height(entry: &Entry, flow: Flow, english: bool) -> i32 {
    let mut height = ENTRY_BASE;
    if !entry.speaker.is_empty() {
        height += ENTRY_ROW;
    }
    height + ENTRY_ROW * wrap(&entry.text, flow, english).len() as i32
}

/// Which entries are drawn and where, from `FUN_10003600`.
///
/// `at` is the entry the player is on: it is centred on [`CENTRE`], earlier
/// entries are stacked upwards until one would start above the buffer, and
/// later ones downwards until one would start past [`Flow::limit`] less three.
/// Each pair is an entry index and the buffer row its first row is measured
/// from.
pub fn visible(entries: &[Entry], at: usize, flow: Flow, english: bool) -> Vec<(usize, i32)> {
    if entries.is_empty() {
        return Vec::new();
    }
    let at = at.min(entries.len() - 1);
    let mut out = Vec::new();

    let own = height(&entries[at], flow, english);
    let mut top = CENTRE - own / 2;
    out.push((at, top));

    let mut above = at as isize - 1;
    while above >= 0 && top >= 0 {
        top -= height(&entries[above as usize], flow, english);
        out.push((above as usize, top));
        above -= 1;
    }

    let mut bottom = CENTRE - own / 2 + own;
    let mut below = at + 1;
    while below < entries.len() && bottom <= flow.limit(english) - 3 {
        out.push((below, bottom));
        bottom += height(&entries[below], flow, english);
        below += 1;
    }

    out.sort_unstable_by_key(|&(index, _)| index);
    out
}

/// What one press of a scroll widget does to the entry the player is on, from
/// `FUN_100042d0`'s five cases.
///
/// Widgets 0 and 3 are the double arrows and move three; 1 and 2 are the single
/// arrows and move one; 4 is Close, which is `+0x4c(0)` and leaves. Every
/// widget is live — `FUN_100042a0` answers yes for 0 through 4 and no for
/// anything else — so an arrow at the end of the list simply clamps.
pub fn scrolled(at: usize, count: usize, widget: usize) -> Option<usize> {
    let last = count.saturating_sub(1);
    let step = |back: usize| Some(at.saturating_sub(back));
    let on = |forward: usize| Some((at + forward).min(last));
    match widget {
        0 => step(3),
        1 => step(1),
        2 => on(1),
        3 => on(3),
        _ => None,
    }
}

/// The Close widget, `FUN_100042d0`'s case 4 — `+0x4c(0)`, the same way out
/// every screen the control bar opens offers.
pub const CLOSE: usize = 4;

/// The entry the screen opens on, `FUN_100039a0`'s
/// `this+0x13c = this+0x138 - 1`: the last line logged.
pub fn opening_entry(count: usize) -> usize {
    count.saturating_sub(1)
}

/// Draws the entries into the buffer, as `FUN_10003600` and `FUN_100022a0`
/// draw them.
///
/// The buffer is [`BUFFER`] and only [`VIEW`] of it is ever shown; a row
/// outside the flow's own window is skipped by the same test the original
/// applies, so what lands outside [`VIEW`] is what the original leaves there
/// too.
pub fn draw(font: &Font, entries: &[Entry], at: usize, flow: Flow, english: bool) -> Image {
    let (width, height_px) = BUFFER;
    let mut buffer = Image::empty(width, height_px);
    let limit = flow.limit(english);
    // `FUN_100022a0`'s `local_10`: the vertical flow shifts every row across by
    // this, and the horizontal one by nothing.
    let across = match flow {
        Flow::Horizontal => 0,
        Flow::Vertical => 0x15,
    };

    for (index, top) in visible(entries, at, flow, english) {
        let entry = &entries[index];
        let mut row = top + FIRST_ROW;
        let mut rows: Vec<(i32, String)> = Vec::new();
        if !entry.speaker.is_empty() {
            rows.push((SPEAKER_X, entry.speaker.clone()));
        }
        for line in wrap(&entry.text, flow, english) {
            rows.push((LINE_X, line));
        }

        for (x, text) in rows {
            if drawable(row, flow, limit) {
                place(
                    &mut buffer,
                    font,
                    &text,
                    x + across,
                    row,
                    flow,
                    english,
                    limit,
                );
            }
            row += ROW_STEP;
        }
    }
    buffer
}

/// Composites the screen with its lines over it.
///
/// The text sprite is created after the base art and after the five widget
/// sprites in `FUN_100039a0`, and they all sit on the same layer —
/// `DX9Sprite2D::vt[8](0, 1)` for every one of them — so it is drawn last and
/// covers them. [`VIEW`] of the buffer goes onto [`DEST`], placed the way a
/// widget is placed.
pub fn compose(screen: &Screen, states: &[WidgetState], buffer: &Image) -> Image {
    let mut out = screen.compose(states);
    blit(&mut out, screen, buffer);
    out
}

/// Puts [`VIEW`] of the buffer onto [`DEST`] of an already-composited frame.
///
/// The half of [`compose`] a caller that has built the frame some other way
/// still needs — [`crate::ui::menu`] composites through the screen's own page
/// and sprite layers first.
pub fn blit(out: &mut Image, screen: &Screen, buffer: &Image) {
    out.blit_downscaled(buffer, (0, 0, VIEW.0, VIEW.1), screen.place_layout(DEST));
}

/// `FUN_100022a0`'s guard on one row: the vertical flow draws every row it is
/// handed, the horizontal one only between `0x59` and `0x2ef`, and both stop at
/// the flow's own limit.
fn drawable(row: i32, flow: Flow, limit: i32) -> bool {
    let within = match flow {
        Flow::Vertical => true,
        Flow::Horizontal => row > 0x59 && row < 0x2ef,
    };
    within && row > 5 && row < limit
}

/// Draws one row's characters into the buffer.
///
/// The horizontal flow runs the pen across at `row - 6` and kerns; the vertical
/// one runs it down at `limit + 6 - row` and does not — `FUN_100022a0` only
/// asks `FUN_100021c0` on the horizontal side, so a vertical column is on the
/// flat [`pitch`].
///
/// `FUN_10002110` is the vertical flow's per-character pass: `、` and `。` are
/// drawn `0x12` back along both axes and the pen put straight again afterwards,
/// and three characters are replaced by an upright form.
#[allow(clippy::too_many_arguments)]
fn place(
    buffer: &mut Image,
    font: &Font,
    text: &str,
    start: i32,
    row: i32,
    flow: Flow,
    english: bool,
    limit: i32,
) {
    let mut pen = start;
    for c in text.chars() {
        match flow {
            Flow::Horizontal => {
                blit_glyph(buffer, font, c, pen, row - GLYPH_LIFT);
                pen += advance(c, english);
            }
            Flow::Vertical => {
                let nudge = if VERTICAL_NUDGE.contains(&c) { 0x12 } else { 0 };
                let upright = VERTICAL_ROTATE
                    .iter()
                    .find(|(from, _)| *from == c)
                    .map_or(c, |(_, to)| *to);
                blit_glyph(
                    buffer,
                    font,
                    upright,
                    limit + 6 - (row - nudge),
                    pen - nudge,
                );
                pen += pitch(english);
            }
        }
    }
}

/// One glyph into the buffer, the way `FUN_00436c10` puts it there.
///
/// The blit is `FUN_004367d0`: the alpha plane as alpha and the luminance plane
/// in all three colour channels, max-blended against what is already there so
/// the outlines of adjacent cells do not cut into each other. A character the
/// font has no glyph for is skipped and still advances, because the caller
/// advances on the character rather than on the glyph.
fn blit_glyph(buffer: &mut Image, font: &Font, c: char, x: i32, y: i32) {
    let glyph = match font.glyph(c) {
        Ok(Some(glyph)) => glyph,
        Ok(None) => {
            if c != ' ' {
                log::warn!("font has no glyph for {c:?} (U+{:04X})", u32::from(c));
            }
            return;
        }
        Err(err) => {
            log::warn!("glyph for {c:?} failed to decode: {err}");
            return;
        }
    };
    let cell = days_font::CELL;
    for gy in 0..cell {
        let dy = y + gy as i32;
        if dy < 0 || dy >= buffer.height as i32 {
            continue;
        }
        for gx in 0..cell {
            let dx = x + gx as i32;
            if dx < 0 || dx >= buffer.width as i32 {
                continue;
            }
            let alpha = glyph.alpha[gy * cell + gx];
            if alpha == 0 {
                continue;
            }
            let lum = glyph.luminance[gy * cell + gx];
            let at = (dy as usize * buffer.width as usize + dx as usize) * 4;
            for (channel, value) in buffer.rgba[at..at + 4]
                .iter_mut()
                .zip([lum, lum, lum, alpha])
            {
                *channel = (*channel).max(value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(speaker: &str, text: &str) -> Entry {
        Entry {
            speaker: speaker.to_string(),
            text: text.to_string(),
        }
    }

    /// The English break lands **before** the word that would overrun, and the
    /// space that preceded it stays on the line it was already on.
    ///
    /// A shipped line: Shiny Days `01-00-A00`, 48 columns. The look-ahead
    /// counts from the character in hand rather than one past it and the test
    /// is `>=`, both of which differ from the dialogue box's
    /// `crate::playback::text::wrap`.
    #[test]
    fn english_breaks_before_the_word_that_would_overrun() {
        let line = "I'm sick and can't really move... I'll leave it to you.";
        assert_eq!(
            wrap(line, Flow::Horizontal, true),
            [
                "I'm sick and can't really move... I'll leave it ".to_string(),
                "to you.".to_string(),
            ]
        );
    }

    /// Kinsoku: the Japanese rule holds a line open past its limit rather than
    /// start the next one with a character that may not start one.
    ///
    /// `。` is in `DAT_1004300c`, which `FUN_10002070` answers for, so the
    /// break the column count asks for at 27 does not happen until the
    /// character after it.
    #[test]
    fn japanese_does_not_start_a_line_with_a_closing_mark() {
        let text = "あ".repeat(27) + "。" + "い";
        let lines = wrap(&text, Flow::Horizontal, false);
        assert_eq!(lines[0].chars().count(), 28, "{lines:?}");
        assert!(lines[0].ends_with('。'), "{lines:?}");
        assert_eq!(lines[1], "い");
    }

    /// `\n` is dropped rather than breaking the line — the backlog's own rule,
    /// and the opposite of the dialogue box's, where it is a hard break.
    ///
    /// A `\` that is **not** followed by `n` is where this engine diverges:
    /// `FUN_10002a90` does not advance past it and spins. Nothing in either
    /// title's shipped `[PrintText]` reaches that, and here the backslash is
    /// consumed the way `FUN_0043f600` consumes it.
    #[test]
    fn escapes_are_consumed_and_never_break_the_line() {
        assert_eq!(wrap(r"one\ntwo", Flow::Horizontal, true), ["onetwo"]);
        assert_eq!(wrap(r"one\two", Flow::Horizontal, true), ["onetwo"]);
    }

    /// Ruby marks take no columns, whether or not the ruby is drawn.
    #[test]
    fn ruby_marks_are_consumed_whole() {
        assert_eq!(wrap("a｜b《ruby》c", Flow::Horizontal, true), ["abc"]);
    }

    /// The entry the player is on is centred on `CENTRE`, and its neighbours
    /// stack off its own height rather than off a fixed pitch.
    #[test]
    fn the_current_entry_is_centred_and_its_neighbours_stack_from_it() {
        let entries = vec![entry("A", "one"), entry("B", "two"), entry("C", "three")];
        let own = height(&entries[1], Flow::Horizontal, true);
        assert_eq!(own, ENTRY_BASE + ENTRY_ROW * 2);
        assert_eq!(
            visible(&entries, 1, Flow::Horizontal, true),
            [
                (0, CENTRE - own / 2 - own),
                (1, CENTRE - own / 2),
                (2, CENTRE - own / 2 + own),
            ]
        );
    }

    /// A row outside the horizontal flow's window is skipped, which is what
    /// keeps a half-scrolled entry off the title band.
    #[test]
    fn rows_outside_the_window_are_not_drawn() {
        let limit = Flow::Horizontal.limit(true);
        assert!(!drawable(0x59, Flow::Horizontal, limit));
        assert!(drawable(0x5a, Flow::Horizontal, limit));
        assert!(drawable(0x2ee, Flow::Horizontal, limit));
        assert!(!drawable(0x2ef, Flow::Horizontal, limit));
        // The vertical flow has no such window, only the shared limit.
        assert!(drawable(0x2ef, Flow::Vertical, Flow::Vertical.limit(true)));
    }

    /// The double arrows move three and the single ones one, and both clamp.
    #[test]
    fn the_arrows_move_three_and_one_and_clamp() {
        assert_eq!(scrolled(5, 10, 0), Some(2));
        assert_eq!(scrolled(1, 10, 0), Some(0));
        assert_eq!(scrolled(5, 10, 1), Some(4));
        assert_eq!(scrolled(5, 10, 2), Some(6));
        assert_eq!(scrolled(8, 10, 3), Some(9));
        assert_eq!(scrolled(5, 10, 4), None);
    }
}
