//! The save/load screen — mode 3, `System/SaveLoad`.
//!
//! One screen serves both jobs. Which one it is doing is a member the DLL
//! keeps at `+0x94`, and everything else follows from it: the base art is
//! `Load.png` or `Save.png`, picking a row either loads that slot or writes
//! one, and the route-map button is live on the Load screen only.
//!
//! # The list
//!
//! Ten slots to a page, ten pages, so a hundred slots. The widget table has
//! two bands of ten for the rows, the left of the row and the right of it, and
//! clicking either picks the same slot:
//!
//! ```text
//! 0x00 .. 0x09   the ten rows
//! 0x0a .. 0x13   the ten page buttons
//! 0x14           leave
//! 0x15           the route map, Load screen only
//! 0x16 .. 0x1f   the ten rows again, the other band
//! ```
//!
//! `FUN_10014990` is that dispatch and [`action`] is its transcription. The
//! slot a row stands for is `page * 10 + row` (`FUN_10011d50`), which is why
//! the shipped `[SaveFileName]="Save/SaveFile00%d.DAT"` puts slot 14 in
//! `SaveFile0014.DAT` rather than in a padded name.
//!
//! Every widget is live unless the confirm popup is up (`FUN_10014910`): the
//! screen does not grey out an empty slot, it just does nothing when the row
//! it names has no file.
//!
//! # The display line
//!
//! A slot's file says nothing about itself. The line the screen shows lives in
//! the **global** store, under the key `[SaveConfig]="FILMEngine/SaveFile00%d"`
//! formats, with the player's comment under the same name plus `_Sub`.
//!
//! `FUN_10011b40` builds it from the clock and the chapter number, as one
//! string with the chapter appended:
//!
//! ```text
//! Japanese   "%4d年%2d月%2d日(%s)%02d:%02d"  +  "第%d話"
//! English    "%2d/%2d/%4d(%s)%02d:%02d"      +  "%02d"
//! ```
//!
//! The reader splits the chapter back off by character count — three for
//! Japanese, two for English (`FUN_0042a980`) — which is exactly the length
//! each of those tails has, and is why the split is a constant rather than a
//! search.
//!
//! # Where a row's text sits
//!
//! No column is drawn to the screen directly. `FUN_100135c0` builds a
//! 2048x1024 off-screen ARGB surface — a `FrameBuffer` at `+0x10c` wrapped by
//! a `DX9Texture` at `+0x110`, with the buffer's pixels cached at `+0x114` and
//! its pitch at `+0x118` — and `FUN_10011ec0` clears it and rasterises all
//! thirty columns into it at once. Each column then has its own sprite, whose
//! source rectangle cuts that surface and whose destination rectangle puts it
//! on the screen. [`SURFACE`], [`source_rect`] and [`dest_rect`] are those
//! three facts.
//!
//! The classes are RTTI names, not inferences: host `+0xac` allocates by type
//! code, and codes 0, 3 and 4 construct `DX9Texture`, `DX9Sprite2D` and
//! `FrameBuffer`. That matters because the sprite calls Ghidra renders as a
//! chain of `float10` results are really `DX9Texture` slot `+8` (`x / width`)
//! and slot `+0xc` (`y / height`) normalising pixels to texture coordinates,
//! and `DX9Sprite2D` slot `+0x1c`, which takes an origin **and a size** and
//! forwards `(u, v, u + du, v + dv)`. Reading that chain as written gives four
//! coordinates in the wrong order; the disassembly gives the real one, and it
//! pairs every `/ width` value with the other `/ width` value.
//!
//! On the surface, per row `r` (`FUN_10011ec0` agrees with `FUN_100135c0` on
//! all three, which is the cross-check):
//!
//! ```text
//! timestamp   x 0      y r * 48 + 2     548 x 48
//! chapter     x 1024   y r * 48 + 2     548 x 48
//! comment     x 0      y r * 48 + 514   986 x 48
//! ```
//!
//! On screen, in the 800x450 layout space the widget records use, where
//! `record` is the row's own record — the first band for the timestamp and
//! chapter, the second for the comment:
//!
//! ```text
//! timestamp   record[r].x        + 1.0    record[r].y + 4.5   252 x 24
//! chapter     record[r].x        + 262.5  record[r].y + 4.5   252 x 24
//! comment     record[r + 0x16].x + 2.0    record[.].y + 4.5   494 x 24
//! ```
//!
//! So the two bands are not two hit regions over one drawing: each band is
//! where one of the columns lands. Every one of those constants was read out
//! of the player's own DLL with its operand width taken from the instruction
//! (`flds` for a 4-byte float, `fmull`/`faddl` for an 8-byte double) rather
//! than from Ghidra's `(float)_DAT_...`, which narrows doubles at the use site
//! and has already turned a 2.5 into a 0.0 on this project once.
//!
//! The surface is about twice the screen, so the text is rasterised at the
//! font's own 48-pixel cell and comes down to size in the blit. Only the height
//! halves exactly, 48 to 24. The widths do not: the comment goes 986 to 494 and
//! the stored line 548 to 252, so both columns are squeezed horizontally, the
//! stored line's noticeably. These are the shipped constants and they are
//! transcribed rather than rounded to the clean ratio they nearly are.
//!
//! English shifts three things, all off host `+0x5c`: the timestamp moves right
//! by 5.0, the chapter by 15.0, and the comment is centred in its column by
//! `235.5 - width / 4` clamped at zero, measured on the advance total
//! `FUN_10011ec0` accumulates. See [`comment_centre`].
//!
//! # The comment column's gate is `[TextInput]`
//!
//! The comment sprites are built, and the comment rasterised, only when host
//! `+0xd8` answers non-zero. That slot is `FUN_0042bad0`, returning member
//! `+0x74` of the interface it is called on — and the host object carries
//! **two** interfaces, one at `+0x2c` (`movl $0x4d2894,0x2c(%edx)` in
//! `FUN_004217e0`) and one at `+0x30` (`0x4d2864`). The DLL holds the first, so
//! the member is object `+0xa0`.
//!
//! Nothing writes `0xa0(reg)` anywhere in the executable, which is not the same
//! as nothing writing the member: the writer holds the **second** interface, so
//! it stores at `0x70(reg)`. It is slot `+0x20` of the `+0x30` vtable,
//! `FUN_00422170`, and it is the `FILMENGINE.INI` reader:
//!
//! ```text
//! [TextInput]    -> this+0x70  -> object +0xa0 -> host +0xd8
//! [UseEnglish]   -> this+0x74  -> object +0xa4 -> host +0x5c
//! ```
//!
//! The second line is the check on the first. `+0x5c` is the English question,
//! already recovered from the other side and used all over this module, and it
//! lands on the member the key next to `[TextInput]` writes. Two interfaces,
//! two different deltas, one member, and the meaning agrees.
//!
//! So the column is on when the player's `FILMENGINE.INI` says
//! `[TextInput]="1"`, which the shipped INI does.
//!
//! # One sprite between the two bands
//!
//! Only the first band of ten has hover art. `FUN_10011600` draws it in a loop
//! over the rows that lights `+0xa8 + row * 4` when
//! `row == selection || row + 0x16 == selection`, so pointing anywhere in a row
//! highlights the row entire, from the first band's full-width record.
//! [`highlight`] is that mapping, and the second band's own records are not
//! hover art at all — see below.
//!
//! # The expanded comment, which is what the tall records are for
//!
//! The second band's records are about three rows tall, and that is not an
//! error in the table: they size the panel of a tooltip. When the selection is
//! in that band, `FUN_10011600` calls `FUN_10012900`, which re-wraps the
//! slot's whole comment over up to three lines and shows it over the list —
//! the panel sprite at `+0x108` cut from that record, and the lines at
//! `+0x1bc + n * 4` rasterised into the same surface at
//! `(0x400, 0x202 + n * 0x40)`. The panel's height grows with the line count,
//! and rows 8 and 9 borrow rows 6 and 7's record so three lines cannot run off
//! the bottom of the screen.
//!
//! [`Tooltip`] is that, and [`wrap_comment`] is its wrapping rule: Japanese breaks
//! every twenty characters and English wraps on whole words at forty, both
//! measured in characters and both cut at three lines' worth.
//!
//! # Saving does not leave this screen
//!
//! Clicking a row on the save screen does not write anything. `FUN_10014990`
//! records the row at `+0x1f4` and then asks the host `+0xd8` — which is
//! `FILMENGINE.INI [TextInput]` — what to do next:
//!
//! * key set: hand `+0xdc` a default comment, which opens the executable's own
//!   dialog. Its OK calls back into `_CommentSet@4`, which stores the text at
//!   `+0x200` and raises `+0x98`. Cancel calls nothing, so nothing is taken.
//! * key clear: raise `+0x98` on the spot, with no dialog at all. The comment
//!   `FUN_10011c30` then writes is `L""`.
//!
//! **Overwriting is silent and unconditional.** Nothing anywhere on that path
//! asks, and three separate places say so: `FUN_10014990`'s save arm branches
//! only on `+0xd8` and never tests whether the row is occupied; `FUN_10014910`
//! gates the row on nothing but the popup state, where the *load* arm goes
//! through `FUN_10011d50` and refuses a row whose `+0x11c` entry is zero; and
//! the host's own writer `FUN_0042aea0` builds the path out of
//! `[SaveFileName]`, hands it to `FUN_00457240` with mode 1 — "Open
//! WritableFile", create and truncate — and writes, with no existence test and
//! no backup. So the asymmetry is the whole rule: loading refuses an empty
//! slot, saving accepts any slot and overwrites it without asking.
//!
//! `+0x98` is the save waiting to be written. While it is up `FUN_10014910`
//! answers false for every widget, so the screen is inert, and the next tick
//! of `FUN_10014c90` takes the other arm: `FUN_10011c30` asks the host to write
//! the slot through `+0xa0`, `FUN_10011ec0` re-rasterises the page, and `+0x98`
//! comes down. **The player is left on the save screen**, with the row they
//! just wrote showing its new timestamp.
//!
//! # Still to build
//!
//! `Popup_Save.png` draws while `+0x98` is up — `FUN_100135c0` loads it
//! whenever the module opens for the save job, and `FUN_1000ffe0` places its
//! two sprites as bands 604x18 at (150, 371) and 604x17 at (150, 398), read out
//! of the DLL's own `DAT_1004ae10` and `DAT_1004ae28`. **Which part of the
//! image each band shows is not recovered** — the place call carries only the
//! destination — so the notice is not drawn yet. The save itself behaves
//! correctly without it.
//!
//! Keyboard navigation through the list (`FUN_10014c90`) is a transition table
//! that is **not recovered**; the pointer works, and the arrow keys fall back
//! to the generic order.
//!
//! The two sprites at `+0xf8` and `+0xfc`, placed from records
//! `(page + 0x20) * 0x18` and `(page + 0x2a) * 0x18`, are the current page's
//! indicator and sit past the thirty-two records the atlas recovers for this
//! screen.

use crate::install::clock::Civil;
use crate::install::ini::Ini;
use crate::install::save;
use crate::playback::text;
use days_save::{FlagStore, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// Which job the screen is doing, the DLL's `+0x94`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `+0x94 == 0`: `Load.png`, and picking a row loads it.
    Load,
    /// Anything else: `Save.png`, and picking a row writes it.
    Save,
}

impl Kind {
    /// The base art the screen loads, from the DLL's own `+0x94` test.
    pub fn base(self) -> &'static str {
        match self {
            Kind::Load => "Load.png",
            Kind::Save => "Save.png",
        }
    }
}

/// Slots to a page, and pages. Ten rows are drawn per page and there are ten
/// page buttons, both from `FUN_10011ec0`'s loop and the widget bands.
pub const PER_PAGE: usize = 10;
pub const PAGES: usize = 10;

/// What activating a widget asks for, from `FUN_10014990`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// The widget is not one the screen answers.
    None,
    /// Use this row of the page in whichever way the screen is for.
    Row(usize),
    /// Show this page of ten slots.
    Page(usize),
    /// Leave the screen: host `+0x4c(0)`.
    Leave,
    /// Open the route map: host `+0x4c(6)`. Only on the Load screen.
    RouteMap,
}

/// The screen's dispatch, from `FUN_10014990`.
pub fn action(kind: Kind, widget: usize) -> Act {
    match widget {
        0..PER_PAGE => Act::Row(widget),
        0x0a..0x14 => Act::Page(widget - 0x0a),
        0x14 => Act::Leave,
        // Guarded by `+0x94 == 0` in the DLL, so the Save screen's copy of
        // this widget does nothing at all.
        0x15 if kind == Kind::Load => Act::RouteMap,
        0x16..0x20 => Act::Row(widget - 0x16),
        _ => Act::None,
    }
}

/// Which widget's sprite a selection lights, from `FUN_10011600`.
///
/// Not the selected widget. The draw loop runs over the ten rows and tests
/// `row == selection || row + 0x16 == selection`, lighting `+0xa8 + row * 4`
/// either way — the sprite built from the **first** band's record, which spans
/// the whole row. The second band has no sprite of its own at all, so pointing
/// anywhere in a row highlights the row entire.
///
/// The route map is the other departure: its sprite is drawn behind one
/// condition more than [`enabled`] carries, `+0x94 == 0`, so on the Save screen
/// the widget is still pointable and still does nothing, and now also lights
/// nothing. Anything past the table lights nothing either.
pub fn highlight(kind: Kind, widget: usize) -> Option<usize> {
    match widget {
        0..PER_PAGE => Some(widget),
        0x0a..0x15 => Some(widget),
        0x15 => (kind == Kind::Load).then_some(0x15),
        0x16..0x20 => Some(widget - 0x16),
        _ => None,
    }
}

/// Whether a widget can be chosen, from `FUN_10014910`.
///
/// Every widget the screen has, until the confirm popup goes up and takes them
/// all at once. An empty slot is still live — picking it is what does nothing,
/// not the pointing.
pub fn enabled(popup_up: bool, widget: usize) -> bool {
    !popup_up && widget < 0x20
}

/// The slot a row of a page stands for, from `FUN_10011d50`.
pub fn slot_of(page: usize, row: usize) -> u32 {
    (page * PER_PAGE + row) as u32
}

/// The off-screen surface every column is rasterised into, from
/// `FUN_100135c0`: `FrameBuffer` slot `+8` is called with `(0x800, 0x400,
/// 0x208888)`, and `FUN_10011ec0` clears it as 0x400 rows of 0x2000 bytes,
/// which is the same 2048 pixels of 32 bits.
pub const SURFACE: (u32, u32) = (0x800, 0x400);

/// Which of the three things a row shows.
///
/// Each is a separate `DX9Sprite2D` with its own source and destination rect —
/// `+0x144 + row * 4`, `+0x16c + row * 4` and `+0x194 + row * 4`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    /// The timestamp half of the stored line.
    When,
    /// The chapter half.
    Chapter,
    /// The player's own comment.
    Comment,
}

impl Column {
    pub const ALL: [Column; 3] = [Column::When, Column::Chapter, Column::Comment];

    /// Where in the surface this column's glyphs start, and how wide the sprite
    /// cuts it.
    ///
    /// The x origins are `FUN_10011ec0`'s own pen starts — 0, `0x400` and 0 —
    /// and the widths are `_DAT_1003b138` (a `flds`, so 548.0f) and
    /// `_DAT_1003b110` (986.0f).
    pub const fn surface_span(self) -> (f32, f32) {
        match self {
            Column::When => (0.0, 548.0),
            Column::Chapter => (1024.0, 548.0),
            Column::Comment => (0.0, 986.0),
        }
    }

    /// The width of the sprite that cuts this column out of [`SURFACE`].
    pub const fn width(self) -> f32 {
        self.surface_span().1
    }

    /// How many characters of the column are drawn, from `FUN_10011ec0`.
    ///
    /// A cap, not a measurement: the loop stops at this many characters however
    /// wide they are. English buys the timestamp one more character and the
    /// comment twenty more; the chapter is always twenty.
    pub const fn cap(self, english: bool) -> usize {
        match self {
            Column::When => 0x14 + if english { 1 } else { 0 },
            Column::Chapter => 0x14,
            Column::Comment => 0x14 + if english { 0x14 } else { 0 },
        }
    }

    /// Which of the row's two widget records places this column.
    ///
    /// `FUN_100135c0` reads `DAT_1004b048 + row * 0x18` for the timestamp and
    /// the chapter and `DAT_1004b048 + (row + 0x16) * 0x18` for the comment, so
    /// the second band of ten is not a duplicate: it is where the comment goes.
    pub const fn record_of(self, row: usize) -> usize {
        match self {
            Column::When | Column::Chapter => row,
            Column::Comment => row + 0x16,
        }
    }

    /// The x this column is offset by inside its record, and the extra shift
    /// English adds.
    ///
    /// `_DAT_10039798` (1.0), `_DAT_1003b118` (262.5) and `_DAT_10039748` (2.0),
    /// all `faddl` so all doubles; the English shifts are `_DAT_1003b140`
    /// (5.0f) and `_DAT_1003b13c` (15.0f), and the comment has none here —
    /// it is centred instead, in [`comment_centre`].
    pub const fn dest_x(self, english: bool) -> f32 {
        let shift = if english { 1.0 } else { 0.0 };
        match self {
            Column::When => 1.0 + 5.0 * shift,
            Column::Chapter => 262.5 + 15.0 * shift,
            Column::Comment => 2.0,
        }
    }

    /// How wide this column is drawn: `_DAT_1003b128` (252.0) for the two
    /// halves of the stored line and `_DAT_1003b0d0` (494.0) for the comment,
    /// both `fmull` so both doubles.
    pub const fn dest_width(self) -> f32 {
        match self {
            Column::When | Column::Chapter => 252.0,
            Column::Comment => 494.0,
        }
    }
}

/// One row of the surface, `_DAT_1003b130` — an `fmull`, so the double 48.0.
pub const SURFACE_ROW_PITCH: f32 = 48.0;

/// How tall each column's slice of the surface is, `_DAT_1003977c` — a `flds`,
/// so the float 48.0. The same as the pitch, so the rows abut exactly.
pub const SURFACE_ROW_HEIGHT: f32 = 48.0;

/// Where in the row the glyphs of the stored line go, `_DAT_10039748` (2.0),
/// and where the comment's go, `_DAT_1003b108` (514.0). Both `faddl`.
const SURFACE_LINE_Y: f32 = 2.0;
const SURFACE_COMMENT_Y: f32 = 514.0;

/// How far down its record every column is drawn, `_DAT_1003b0c8` — an `faddl`,
/// so the double 4.5.
pub const DEST_Y: f32 = 4.5;

/// How tall every column is drawn, `_DAT_1003a860` — an `fmull`, so the double
/// 24.0. Half the surface's row, which is why the text is rasterised at the
/// font's own cell and comes down to size in the blit.
pub const DEST_HEIGHT: f32 = 24.0;

/// The rectangle of [`SURFACE`] this column of this row occupies, as
/// `(x, y, width, height)`.
///
/// From `FUN_100135c0`'s `DX9Sprite2D` slot `+0x1c` calls, whose two `/ width`
/// arguments are the x pair and whose two `/ height` arguments are the y pair.
/// `FUN_10011ec0` rasterises into the same places, which is the cross-check.
pub fn source_rect(column: Column, row: usize) -> (f32, f32, f32, f32) {
    let (x, width) = column.surface_span();
    let base = match column {
        Column::When | Column::Chapter => SURFACE_LINE_Y,
        Column::Comment => SURFACE_COMMENT_Y,
    };
    (
        x,
        row as f32 * SURFACE_ROW_PITCH + base,
        width,
        SURFACE_ROW_HEIGHT,
    )
}

/// Where in the glyph surface a column's pen starts for a row.
///
/// The same origin [`source_rect`] cuts from, offset by nothing: `FUN_10011ec0`
/// draws each glyph at the top of its slice.
pub fn surface_pen(column: Column, row: usize) -> (i32, i32) {
    let (x, y, _, _) = source_rect(column, row);
    (x as i32, y as i32)
}

/// Where this column of this row is drawn, in the 800x450 layout space the
/// widget records use, as `(x, y, width, height)`.
///
/// `record` is the row's own record — [`Column::record_of`] says which of the
/// two bands — and `centre` is [`comment_centre`], which is zero for every
/// column but an English comment.
pub fn dest_rect(
    column: Column,
    record: days_ui::cmap::Rect,
    english: bool,
    centre: f32,
) -> (f32, f32, f32, f32) {
    (
        record.x as f32 + column.dest_x(english) + centre,
        record.y as f32 + DEST_Y,
        column.dest_width(),
        DEST_HEIGHT,
    )
}

/// How far right an English comment is pushed, from `FUN_10011ec0`.
///
/// `_DAT_1003b0d8 - width / _DAT_1003b0e0`, clamped to zero below
/// `_DAT_10039758` — an `fsubrl`, an `fdivl` and an `fcompl`, so 235.5, 4.0 and
/// 0.0. `width` is the advance total the rasterising loop accumulated, in
/// surface pixels, and the column comes down to the screen at very nearly half,
/// so dividing by four is half the drawn width on screen. Taking that from
/// 235.5 centres the comment in its 494-wide column, 11.5 short of true centre.
/// Japanese comments are not moved at all.
pub fn comment_centre(width: i32, english: bool) -> f32 {
    if !english {
        return 0.0;
    }
    let shift = 235.5 - width as f32 / 4.0;
    if shift < 0.0 {
        0.0
    } else {
        shift
    }
}

/// How many lines the expanded comment can run to, from `FUN_10012900`'s own
/// clamp.
pub const TIP_LINES: usize = 3;

/// Where the expanded comment's lines are rasterised, and how they are cut.
///
/// `FUN_10012900` starts its pen at `(0x400, 0x202)` and steps down by `0x40`,
/// and `FUN_100135c0` cuts the sprites at `_DAT_1003b120` (1024.0f),
/// `_DAT_1003b108` (514.0) + n * `_DAT_1003b100` (64.0), `_DAT_1003b110`
/// (986.0f) wide and `_DAT_1003b0f4` (64.0f) tall. The two agree, which is the
/// cross-check.
const TIP_SURFACE_X: f32 = 1024.0;
const TIP_SURFACE_Y: f32 = 514.0;
const TIP_SURFACE_PITCH: f32 = 64.0;
const TIP_SURFACE_HEIGHT: f32 = 64.0;

/// How far apart the lines are drawn and how tall each is, `_DAT_1003b0e8` —
/// an `fmull`, so the double 32.0. Half the surface pitch, the same halving the
/// rows get.
const TIP_LINE_HEIGHT: f32 = 32.0;

/// How many rows of the list the panel's record spans, `_DAT_1003b0f8` — an
/// `fdivl`, so the double 3.0. The record is about three rows tall and the
/// panel is that divided by the lines it needs.
pub const PANEL_ROWS: f32 = 3.0;

/// The last row whose panel can open downwards, from `FUN_10012900`'s
/// `param_1 < 8` test.
pub const LAST_ROW_OPENING_DOWN: usize = 7;

/// Splits a comment the way `FUN_10012900` splits it.
///
/// Two different rules, chosen by host `+0x5c`. Japanese breaks every twenty
/// characters with no regard for what it cuts. English looks ahead at each
/// space to the end of the next word and breaks if that word would not finish
/// inside forty. Both are counted in characters, never in pixels, and both stop
/// after three lines' worth of input.
///
/// The empty line a break at exactly the cap leaves behind is **kept**: the
/// shipped loop increments its line count there, and that count is what sizes
/// the panel.
pub fn wrap_comment(text: &str, english: bool) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let cap = Column::Comment.cap(english);
    let total = chars.len().min(cap * TIP_LINES);
    if total == 0 {
        return Vec::new();
    }

    let mut lines = vec![String::new()];
    let mut on_line = 0usize;
    let mut i = 0usize;
    while i < total {
        if let Some(line) = lines.last_mut() {
            line.push(chars[i]);
        }
        i += 1;
        on_line += 1;
        if !english {
            if i.is_multiple_of(cap) {
                lines.push(String::new());
            }
        } else if chars[i - 1] == ' ' {
            // The look-ahead runs over the whole comment, not the capped part,
            // so a word past the cut still decides the break before it.
            let word = chars[i..].iter().take_while(|c| **c != ' ').count();
            if on_line + word >= cap {
                lines.push(String::new());
                on_line = 0;
            }
        }
    }
    lines.truncate(TIP_LINES);
    lines
}

/// The expanded comment shown over the list, from `FUN_10012900`.
#[derive(Debug, Clone)]
pub struct Tooltip {
    /// The panel behind the lines. Its `src` is in the screen's `_CHIP` sheet,
    /// not in [`Rows::surface`], and it is the record's **full** height however
    /// short the panel is drawn — so a one-line panel is the same art squashed,
    /// which is what the shipped sprite does.
    pub panel: Quad,
    /// The lines, cut from [`Rows::surface`].
    pub lines: Vec<Quad>,
}

impl Tooltip {
    /// Lays out the tooltip for a row, given the lines already wrapped and the
    /// advance each consumed.
    ///
    /// `record` is the row's second-band record — but not always its own: a row
    /// past [`LAST_ROW_OPENING_DOWN`] borrows the record two rows up and the
    /// panel grows upwards instead, so three lines cannot run off the bottom of
    /// the screen. [`Tooltip::record_row`] is that swap.
    ///
    /// The panel's height comes from the **character count**, not from the
    /// wrapped line count: `FUN_10012900` divides the capped length by the
    /// per-line cap and switches on that. The two agree for Japanese and can
    /// differ for English, where the wrap is by word; the shipped formula is
    /// kept rather than the one that would agree.
    pub fn place(
        row: usize,
        record: days_ui::atlas::Widget,
        widths: &[i32],
        chars: usize,
        english: bool,
    ) -> Tooltip {
        let cap = Column::Comment.cap(english);
        let height = record.dst.height as f32;
        let row_of_panel = height / PANEL_ROWS;
        let deep = row > LAST_ROW_OPENING_DOWN;

        // `FUN_10012900` writes the panel's shift and the text's only on the
        // branches a deep row takes, and zeroes them only on the three-line
        // one. A shallow row with one or two lines therefore reads **two
        // uninitialised floats** -- a shipped bug. Zero is what the branch that
        // does initialise them uses, and what puts the panel on its own row.
        let (panel_shift, text_shift, panel_height) = match chars.min(cap * TIP_LINES) / cap {
            0 => (
                if deep { row_of_panel * 2.0 } else { 0.0 },
                if deep { TIP_SURFACE_HEIGHT } else { 0.0 },
                row_of_panel - 2.0,
            ),
            1 => (
                if deep { row_of_panel } else { 0.0 },
                if deep { TIP_LINE_HEIGHT } else { 0.0 },
                row_of_panel * 2.0,
            ),
            _ => (0.0, 0.0, height),
        };

        let panel = Quad {
            src: (
                record.src_x,
                record.src_y,
                record.dst.width,
                record.dst.height,
            ),
            // The half-pixel outset is the DLL's, on this sprite as on every
            // other: origin back by 0.5 and size out by 1.0.
            dst: (
                record.dst.x as f32 - 0.5,
                record.dst.y as f32 - 0.5 + panel_shift,
                record.dst.width as f32 + 1.0,
                panel_height + 1.0,
            ),
        };

        let lines = widths
            .iter()
            .take(TIP_LINES)
            .enumerate()
            .map(|(n, width)| Quad {
                src: (
                    TIP_SURFACE_X as u32,
                    (TIP_SURFACE_Y + n as f32 * TIP_SURFACE_PITCH) as u32,
                    Column::Comment.width() as u32,
                    TIP_SURFACE_HEIGHT as u32,
                ),
                dst: (
                    record.dst.x as f32
                        + Column::Comment.dest_x(english)
                        + comment_centre(*width, english),
                    record.dst.y as f32 + DEST_Y + n as f32 * TIP_LINE_HEIGHT + text_shift,
                    Column::Comment.dest_width(),
                    TIP_LINE_HEIGHT,
                ),
            })
            .collect();

        Tooltip { panel, lines }
    }

    /// Which row's second-band record places the panel, from
    /// `FUN_10012900`'s `param_1 < 8 ? param_1 : param_1 - 2`.
    pub fn record_row(row: usize) -> usize {
        if row > LAST_ROW_OPENING_DOWN {
            row - 2
        } else {
            row
        }
    }
}

/// The ten rows of a page, rasterised into one surface with the rectangles that
/// put each column on the screen.
///
/// This is `FUN_10011ec0` and the sprite set-up in `FUN_100135c0` together: one
/// [`SURFACE`]-sized buffer holding up to thirty columns of text, and a quad per
/// column cutting it out and placing it.
pub struct Rows {
    /// The glyph surface, RGB carrying the font's luminance plane and alpha its
    /// outline plane — the same two planes the shipped blitter writes.
    pub surface: days_ui::Image,
    /// One per drawn column, in row order.
    pub quads: Vec<Quad>,
    /// The expanded comment, when the pointer is in the second band and the
    /// row it names has one.
    pub tooltip: Option<Tooltip>,
    /// The surface [`Tooltip::lines`] are cut from, when the screen gives the
    /// expanded comment one of its own.
    ///
    /// This screen does not: `FUN_10012900` rasterises the tooltip into the
    /// same buffer as the rows, clear of all three columns, so this is `None`
    /// and the lines come out of [`Rows::surface`]. The Shiny Days list keeps a
    /// seventh buffer for it — see [`crate::ui::replay_pages`].
    pub tip_surface: Option<days_ui::Image>,
}

/// One column's sprite: what it cuts out of the surface and where it lands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    /// The rectangle of [`Rows::surface`] this column occupies, in pixels.
    pub src: (u32, u32, u32, u32),
    /// Where it is drawn, in the 800x450 layout space the widget records use.
    pub dst: (f32, f32, f32, f32),
}

impl Rows {
    /// Rasterises a page's rows.
    ///
    /// A row whose slot has no file is skipped entirely, which is what the
    /// shipped loop does: it only draws when the host's slot query answers 1.
    /// `records` is the screen's widget table, and a row whose record is missing
    /// is skipped rather than placed somewhere this engine chose.
    /// `hovered` is the row the pointer is expanding, if any — the selection's
    /// `widget - 0x16`, which is the only thing that opens the tooltip.
    pub fn render(
        font: &days_font::Font,
        slots: &Slots,
        page: usize,
        english: bool,
        records: &[days_ui::atlas::Widget],
        comments: bool,
        hovered: Option<usize>,
    ) -> Rows {
        let (width, height) = SURFACE;
        let mut surface = days_ui::Image::empty(width, height);
        let mut quads = Vec::new();

        for row in 0..PER_PAGE {
            let Some(line) = slots.get(slot_of(page, row)) else {
                continue;
            };
            for column in Column::ALL {
                if column == Column::Comment && !comments {
                    continue;
                }
                let text: String = match column {
                    Column::When => &line.when,
                    Column::Chapter => &line.chapter,
                    Column::Comment => &line.comment,
                }
                .chars()
                .take(column.cap(english))
                .collect();
                if text.is_empty() {
                    continue;
                }
                let Some(record) = records.get(column.record_of(row)) else {
                    continue;
                };

                let drawn = draw_line(&mut surface, font, &text, surface_pen(column, row), &|c| {
                    text::menu_advance(c, english)
                });
                let centre = match column {
                    Column::Comment => comment_centre(drawn, english),
                    _ => 0.0,
                };
                let (sx, sy, sw, sh) = source_rect(column, row);
                quads.push(Quad {
                    src: (sx as u32, sy as u32, sw as u32, sh as u32),
                    dst: dest_rect(column, record.dst, english, centre),
                });
            }
        }

        let tooltip = hovered
            .filter(|_| comments)
            .and_then(|row| Rows::expand(&mut surface, font, slots, page, row, english, records));
        Rows {
            surface,
            quads,
            tooltip,
            tip_surface: None,
        }
    }

    /// Rasterises the expanded comment and lays it out, from `FUN_10012900`.
    ///
    /// Draws into the same surface the rows use, at the same place the shipped
    /// code draws it: `(0x400, 0x202 + n * 0x40)`, which is clear of all three
    /// columns — they end at x 986 above y 514 and at x 1024 below it.
    fn expand(
        surface: &mut days_ui::Image,
        font: &days_font::Font,
        slots: &Slots,
        page: usize,
        row: usize,
        english: bool,
        records: &[days_ui::atlas::Widget],
    ) -> Option<Tooltip> {
        let comment = &slots.get(slot_of(page, row))?.comment;
        let lines = wrap_comment(comment, english);
        if lines.is_empty() {
            return None;
        }
        let record = *records.get(Column::Comment.record_of(Tooltip::record_row(row)))?;

        let widths = lines
            .iter()
            .enumerate()
            .map(|(n, line)| {
                let pen = (
                    TIP_SURFACE_X as i32,
                    (TIP_SURFACE_Y + n as f32 * TIP_SURFACE_PITCH) as i32,
                );
                draw_line(surface, font, line, pen, &|c| {
                    text::menu_advance(c, english)
                })
            })
            .collect::<Vec<i32>>();

        Some(Tooltip::place(
            row,
            record,
            &widths,
            comment.chars().count(),
            english,
        ))
    }
}

/// Draws one line into the glyph surface and returns the advance it consumed.
///
/// The shipped blitter writes `alpha << 24 | lum << 16 | lum << 8 | lum` and
/// keeps whichever value is larger (`FUN_004367d0`), so overlapping cells take
/// the brighter pixel rather than compositing. [`text::render_line_with`]
/// already produces exactly those two planes when asked for white, and already
/// combines glyphs with `max` for the same reason, so the line goes in as one
/// piece.
///
/// `advance` is the per-character step the screen's own rasterising loop uses,
/// because the two modules do not share one: this module and
/// [`crate::ui::playdata`] call [`text::menu_advance`], while
/// [`crate::ui::replay_pages`] has a flat rule of its own.
pub(crate) fn draw_line(
    surface: &mut days_ui::Image,
    font: &days_font::Font,
    text: &str,
    (x, y): (i32, i32),
    advance: &dyn Fn(char) -> i32,
) -> i32 {
    let line = text::render_line_with(font, text, [0xff, 0xff, 0xff], advance);
    for sy in 0..line.height {
        let dy = y + sy as i32;
        if dy < 0 || dy >= surface.height as i32 {
            continue;
        }
        for sx in 0..line.width {
            let dx = x + sx as i32;
            if dx < 0 || dx >= surface.width as i32 {
                continue;
            }
            let src = (sy * line.width + sx) * 4;
            let dst = (dy as usize * surface.width as usize + dx as usize) * 4;
            for i in 0..4 {
                surface.rgba[dst + i] = surface.rgba[dst + i].max(line.rgba[src + i]);
            }
        }
    }
    text.chars().map(advance).sum()
}

/// The weekday names the timestamp uses.
///
/// Two tables of seven in the DLL, indexed by `SYSTEMTIME.wDayOfWeek`, which
/// counts from Sunday.
const WEEKDAYS_JP: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];
const WEEKDAYS_EN: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// The two halves of a slot's display line, from `FUN_10011b40`.
///
/// Returned separately because the game stores them joined and splits them
/// back apart by length; `story` is the chapter number `_GetStory@4` reports.
pub fn display_line(now: Civil, story: u32, english: bool) -> (String, String) {
    let weekday = |names: [&'static str; 7]| names[(now.weekday as usize).min(6)];
    if english {
        (
            format!(
                "{:2}/{:2}/{:4}({}){:02}:{:02}",
                now.month,
                now.day,
                now.year,
                weekday(WEEKDAYS_EN),
                now.hour,
                now.minute
            ),
            format!("{story:02}"),
        )
    } else {
        (
            format!(
                "{:4}年{:2}月{:2}日({}){:02}:{:02}",
                now.year,
                now.month,
                now.day,
                weekday(WEEKDAYS_JP),
                now.hour,
                now.minute
            ),
            format!("第{story}話"),
        )
    }
}

/// How many characters of a stored line are the chapter, from `FUN_0042a980`.
///
/// A constant, not a search: the Japanese tail is always `第N話` and the
/// English one always two digits.
pub fn tail_len(english: bool) -> usize {
    if english {
        2
    } else {
        3
    }
}

/// Splits a stored display line back into its timestamp and its chapter.
///
/// A line too short to hold a chapter comes back whole, with no tail, rather
/// than being cut into nonsense — the shipped reader only splits when the
/// string is longer than one character.
pub fn split_line(line: &str, english: bool) -> (String, String) {
    let chars: Vec<char> = line.chars().collect();
    let tail = tail_len(english);
    if chars.len() <= 1 || chars.len() < tail {
        return (line.to_owned(), String::new());
    }
    let at = chars.len() - tail;
    (chars[..at].iter().collect(), chars[at..].iter().collect())
}

/// What the screen shows for one slot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line {
    /// The timestamp half of the stored line.
    pub when: String,
    /// The chapter half, `第N話` or two digits.
    pub chapter: String,
    /// The player's own comment, from the `_Sub` key.
    pub comment: String,
}

impl Line {
    /// One slot's line, out of the global store the screen reads it from.
    ///
    /// Split out of [`Slots::read`] so a slot just written can be put back
    /// without re-reading all two hundred: `FUN_10011ec0` re-rasterises the
    /// page straight after `FUN_10011c30` writes, and the row has to show the
    /// new timestamp on that same pass.
    pub fn read(film: &Ini, flags: &FlagStore, slot: u32, english: bool) -> Line {
        let (key, sub) = save::slot_keys(film, slot);
        let stored = flags.get(&key).and_then(Value::as_str).unwrap_or_default();
        let (when, chapter) = split_line(stored, english);
        Line {
            when,
            chapter,
            comment: flags
                .get(&sub)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        }
    }
}

/// What the save/load screen knows about the slots, as host `+0x9c` answers.
///
/// `FUN_0042a980` reports a slot as present only when **the file opens** — it
/// tests that and nothing else, then fills the three strings from the global
/// store. So a slot whose file is gone shows as empty even with its line still
/// in the store, and a slot whose line is missing shows as present with
/// nothing written on it.
#[derive(Debug, Clone, Default)]
pub struct Slots {
    rows: BTreeMap<u32, Line>,
}

impl Slots {
    /// Reads every slot the screen can show.
    pub fn read(game: &Path, film: &Ini, flags: &FlagStore, english: bool) -> Slots {
        let mut rows = BTreeMap::new();
        for slot in 0..(PAGES * PER_PAGE) as u32 {
            if !save::slot_path(game, film, slot).is_file() {
                continue;
            }
            rows.insert(slot, Line::read(film, flags, slot, english));
        }
        Slots { rows }
    }

    /// Whether a slot has a file, which is the only thing the screen tests.
    pub fn filled(&self, slot: u32) -> bool {
        self.rows.contains_key(&slot)
    }

    /// The line a slot shows, if it has one.
    pub fn get(&self, slot: u32) -> Option<&Line> {
        self.rows.get(&slot)
    }

    /// How many slots have a file.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Records a slot the player has just written, so the screen shows it
    /// without re-reading the install.
    pub fn insert(&mut self, slot: u32, line: Line) {
        self.rows.insert(slot, line);
    }

    /// The ten rows of a page, with the slot each stands for.
    pub fn page(&self, page: usize) -> Vec<(u32, Option<&Line>)> {
        (0..PER_PAGE)
            .map(|row| {
                let slot = slot_of(page, row);
                (slot, self.get(slot))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {

    /// Loading refuses an empty slot and saving accepts any slot. The save
    /// screen never asks before overwriting — `FUN_0042aea0` opens the file
    /// create-and-truncate and writes, and nothing on the way there tests
    /// whether the slot was occupied.
    #[test]
    fn saving_overwrites_without_asking_and_loading_refuses_an_empty_slot() {
        // Row 2 of page 0 is the only filled one.
        let mut slots = Slots::default();
        slots.insert(
            2,
            Line {
                when: "9/12/2026(Sat)15:17".to_string(),
                chapter: "01".to_string(),
                comment: String::new(),
            },
        );
        for row in 0..PER_PAGE {
            let slot = slot_of(0, row);
            let widget = 0x16 + row;
            // Both jobs dispatch the row; what differs is what the engine does
            // with it, which is why both are `Act::Row` here.
            assert_eq!(action(Kind::Save, widget), Act::Row(row));
            assert_eq!(action(Kind::Load, widget), Act::Row(row));
            assert_eq!(slots.filled(slot), slot == 2);
        }
    }

    /// A slot just written is put back without re-reading the install, which
    /// is what lets the page re-rasterise in place after `FUN_10011c30`.
    #[test]
    fn a_written_slot_goes_back_into_the_page_in_place() {
        let mut slots = Slots::default();
        assert!(!slots.filled(3));
        slots.insert(
            3,
            Line {
                when: "9/12/2026(Sat)15:17".to_string(),
                chapter: "01".to_string(),
                comment: "just saved".to_string(),
            },
        );
        assert!(slots.filled(3));
        assert_eq!(slots.get(3).map(|l| l.comment.as_str()), Some("just saved"));
        // And the page the screen draws sees it without another read.
        let page = slots.page(0);
        assert_eq!(
            page[3].1.map(|l| l.comment.as_str()),
            Some("just saved"),
            "row 3 of page 0 is slot 3"
        );
    }
    use super::*;

    fn at(year: i32, month: u32, day: u32, weekday: u32, hour: u32, minute: u32) -> Civil {
        Civil {
            year,
            month,
            day,
            weekday,
            hour,
            minute,
            second: 0,
        }
    }

    #[test]
    fn the_two_row_bands_pick_the_same_row() {
        for row in 0..PER_PAGE {
            assert_eq!(action(Kind::Load, row), Act::Row(row));
            assert_eq!(action(Kind::Load, row + 0x16), Act::Row(row));
        }
    }

    #[test]
    fn the_page_buttons_and_the_exits() {
        assert_eq!(action(Kind::Load, 0x0a), Act::Page(0));
        assert_eq!(action(Kind::Load, 0x13), Act::Page(9));
        assert_eq!(action(Kind::Save, 0x14), Act::Leave);
    }

    /// The route map is guarded by `+0x94 == 0`, so the Save screen's copy of
    /// the widget answers nothing.
    #[test]
    fn the_route_map_is_the_load_screens_alone() {
        assert_eq!(action(Kind::Load, 0x15), Act::RouteMap);
        assert_eq!(action(Kind::Save, 0x15), Act::None);
    }

    #[test]
    fn a_widget_past_the_table_asks_for_nothing() {
        assert_eq!(action(Kind::Load, 0x20), Act::None);
        assert!(!enabled(false, 0x20));
    }

    #[test]
    fn the_popup_takes_every_widget_at_once() {
        assert!(enabled(false, 0));
        assert!(enabled(false, 0x15));
        assert!(!enabled(true, 0));
    }

    #[test]
    fn a_row_names_a_slot_the_way_the_file_name_is_numbered() {
        assert_eq!(slot_of(0, 0), 0);
        assert_eq!(slot_of(1, 4), 14);
        assert_eq!(slot_of(9, 9), 99);
    }

    /// The line in the player's own `GlobalFlag.DAT` for slot 0 reads
    /// `2012年 1月28日(土)18:02第6話` — the space before the `1` is `%2d`
    /// padding, which is why this is a format and not a join.
    #[test]
    fn builds_the_line_a_real_save_carries() {
        let (head, tail) = display_line(at(2012, 1, 28, 6, 18, 2), 6, false);
        assert_eq!(head, "2012年 1月28日(土)18:02");
        assert_eq!(tail, "第6話");
        assert_eq!(format!("{head}{tail}"), "2012年 1月28日(土)18:02第6話");
    }

    #[test]
    fn the_english_line_is_the_other_order_and_a_two_digit_chapter() {
        let (head, tail) = display_line(at(2012, 1, 28, 6, 18, 2), 6, true);
        assert_eq!(head, " 1/28/2012(Sat)18:02");
        assert_eq!(tail, "06");
    }

    #[test]
    fn a_line_splits_back_into_what_built_it() {
        for english in [false, true] {
            let (head, tail) = display_line(at(2012, 1, 28, 6, 18, 2), 6, english);
            let joined = format!("{head}{tail}");
            assert_eq!(split_line(&joined, english), (head, tail));
        }
    }

    /// The draw loop lights the row's own sprite for either band, so pointing
    /// at a comment highlights the whole row rather than the three rows the
    /// second band's record covers.
    #[test]
    fn both_bands_light_the_rows_own_sprite() {
        for row in 0..PER_PAGE {
            assert_eq!(highlight(Kind::Load, row), Some(row));
            assert_eq!(highlight(Kind::Load, row + 0x16), Some(row));
        }
    }

    #[test]
    fn the_page_buttons_and_leave_light_themselves() {
        for widget in 0x0a..=0x14 {
            assert_eq!(highlight(Kind::Save, widget), Some(widget));
        }
    }

    /// One condition further than `enabled`: the widget is live on both
    /// screens, but only the Load screen draws its sprite.
    #[test]
    fn the_route_map_lights_on_the_load_screen_alone() {
        assert_eq!(highlight(Kind::Load, 0x15), Some(0x15));
        assert_eq!(highlight(Kind::Save, 0x15), None);
        assert!(enabled(false, 0x15));
    }

    #[test]
    fn a_widget_past_the_table_lights_nothing() {
        assert_eq!(highlight(Kind::Load, 0x20), None);
        assert_eq!(highlight(Kind::Load, 0x99), None);
    }

    /// Twenty characters to a line and no regard for what it cuts.
    #[test]
    fn a_japanese_comment_breaks_on_the_count_alone() {
        let text: String = std::iter::repeat_n('あ', 25).collect();
        let lines = wrap_comment(&text, false);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].chars().count(), 20);
        assert_eq!(lines[1].chars().count(), 5);
    }

    /// A break at exactly the cap leaves an empty line behind, and the shipped
    /// loop counts it -- which is what sizes the panel, so it is kept.
    #[test]
    fn a_break_at_the_cap_leaves_the_empty_line_it_makes() {
        let text: String = std::iter::repeat_n('あ', 20).collect();
        let lines = wrap_comment(&text, false);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1], "");
    }

    /// English breaks before a word that would not finish inside forty.
    #[test]
    fn an_english_comment_breaks_on_whole_words() {
        let lines = wrap_comment("the quick brown fox jumps over the lazy dog again", true);
        assert!(lines.len() >= 2);
        assert!(lines[0].chars().count() <= 40);
        // No word is cut in half.
        for line in &lines {
            assert!(!line.trim_end().ends_with(char::is_alphabetic) || line.len() <= 40);
        }
        assert_eq!(
            lines.concat(),
            "the quick brown fox jumps over the lazy dog again"
        );
    }

    /// Three lines' worth of input and no more, whichever language.
    #[test]
    fn a_comment_is_cut_at_three_lines_of_input() {
        let long: String = std::iter::repeat_n('あ', 200).collect();
        assert_eq!(wrap_comment(&long, false).len(), TIP_LINES);
        let words = "alpha ".repeat(60);
        assert_eq!(wrap_comment(&words, true).len(), TIP_LINES);
    }

    #[test]
    fn an_empty_comment_has_no_tooltip() {
        assert!(wrap_comment("", false).is_empty());
        assert!(wrap_comment("", true).is_empty());
    }

    fn panel_record() -> days_ui::atlas::Widget {
        days_ui::atlas::Widget {
            dst: days_ui::cmap::Rect {
                x: 328,
                y: 96,
                width: 472,
                height: 97,
            },
            src_x: 1,
            src_y: 442,
        }
    }

    /// One line takes a third of the record, two take two thirds and three
    /// take all of it.
    #[test]
    fn the_panel_grows_with_the_comment() {
        let one = Tooltip::place(0, panel_record(), &[0], 10, false);
        let two = Tooltip::place(0, panel_record(), &[0, 0], 25, false);
        let three = Tooltip::place(0, panel_record(), &[0, 0, 0], 50, false);
        assert!((one.panel.dst.3 - (97.0 / 3.0 - 2.0 + 1.0)).abs() < 0.01);
        assert!((two.panel.dst.3 - (97.0 * 2.0 / 3.0 + 1.0)).abs() < 0.01);
        assert!((three.panel.dst.3 - 98.0).abs() < 0.01);
    }

    /// The panel is always the record's full height in the chip sheet however
    /// short it is drawn, so a one-line panel is that art squashed.
    #[test]
    fn the_panel_cuts_the_whole_record_however_short_it_is_drawn() {
        let tip = Tooltip::place(0, panel_record(), &[0], 10, false);
        assert_eq!(tip.panel.src, (1, 442, 472, 97));
        assert!(tip.panel.dst.3 < 97.0);
    }

    /// Rows past the eighth borrow the record two rows up, so three lines
    /// cannot run off the bottom of the screen.
    #[test]
    fn the_last_two_rows_open_upwards() {
        for row in 0..=LAST_ROW_OPENING_DOWN {
            assert_eq!(Tooltip::record_row(row), row);
        }
        assert_eq!(Tooltip::record_row(8), 6);
        assert_eq!(Tooltip::record_row(9), 7);
    }

    /// Having borrowed it, a short panel is pushed back down so it still lands
    /// on the row the pointer is on, and the text with it.
    #[test]
    fn a_borrowed_record_is_pushed_back_down() {
        let deep = Tooltip::place(8, panel_record(), &[0], 10, false);
        let shallow = Tooltip::place(0, panel_record(), &[0], 10, false);
        assert!((deep.panel.dst.1 - shallow.panel.dst.1 - 97.0 * 2.0 / 3.0).abs() < 0.01);
        assert!((deep.lines[0].dst.1 - shallow.lines[0].dst.1 - 64.0).abs() < 0.01);

        let deep = Tooltip::place(9, panel_record(), &[0, 0], 25, false);
        let shallow = Tooltip::place(1, panel_record(), &[0, 0], 25, false);
        assert!((deep.panel.dst.1 - shallow.panel.dst.1 - 97.0 / 3.0).abs() < 0.01);
        assert!((deep.lines[0].dst.1 - shallow.lines[0].dst.1 - 32.0).abs() < 0.01);
    }

    /// Three lines fill the record, so there is nothing to push down.
    #[test]
    fn a_full_panel_is_not_shifted_at_all() {
        let deep = Tooltip::place(9, panel_record(), &[0, 0, 0], 60, false);
        let shallow = Tooltip::place(1, panel_record(), &[0, 0, 0], 60, false);
        assert_eq!(deep.panel.dst.1, shallow.panel.dst.1);
        assert_eq!(deep.lines[0].dst.1, shallow.lines[0].dst.1);
    }

    /// The lines step down by half the surface pitch and cut the surface where
    /// `FUN_10012900` writes them.
    #[test]
    fn the_lines_cut_where_they_were_written() {
        let tip = Tooltip::place(0, panel_record(), &[0, 0, 0], 60, false);
        for (n, line) in tip.lines.iter().enumerate() {
            assert_eq!(line.src, (1024, 514 + n as u32 * 64, 986, 64));
            assert!((line.dst.1 - (96.0 + DEST_Y + n as f32 * 32.0)).abs() < 0.01);
            assert_eq!(line.dst.2, Column::Comment.dest_width());
        }
    }

    /// The tooltip is rasterised clear of all three columns: they stop at x 986
    /// above y 514 and the tooltip starts at x 1024.
    #[test]
    fn the_tooltip_does_not_overwrite_the_rows() {
        for n in 0..TIP_LINES {
            let y = TIP_SURFACE_Y + n as f32 * TIP_SURFACE_PITCH;
            assert!(TIP_SURFACE_X >= Column::Comment.width());
            assert!(y + TIP_SURFACE_HEIGHT <= SURFACE.1 as f32);
            // Clear of the chapter column, which stops at y 482.
            assert!(y >= source_rect(Column::Chapter, PER_PAGE - 1).1 + SURFACE_ROW_HEIGHT);
        }
    }

    /// Pointing at a row with a comment opens it; pointing at nothing does not,
    /// and neither does a screen with `[TextInput]` off.
    #[test]
    fn the_tooltip_opens_only_on_a_hovered_row() {
        let slots = filled(3);
        let open = Rows::render(&font(), &slots, 0, false, &records(), true, Some(3));
        assert!(open.tooltip.is_some());
        assert!(
            Rows::render(&font(), &slots, 0, false, &records(), true, None)
                .tooltip
                .is_none()
        );
        // A row with no file has nothing to expand.
        assert!(
            Rows::render(&font(), &slots, 0, false, &records(), true, Some(4))
                .tooltip
                .is_none()
        );
    }

    /// `[TextInput]` off takes the comment column and the tooltip together,
    /// which is what the one host answer gates.
    #[test]
    fn text_input_off_takes_the_comment_and_its_tooltip() {
        let rows = Rows::render(&font(), &filled(3), 0, false, &records(), false, Some(3));
        assert!(rows.tooltip.is_none());
        assert_eq!(rows.quads.len(), 2);
    }

    fn rect(x: u32, y: u32) -> days_ui::cmap::Rect {
        days_ui::cmap::Rect {
            x,
            y,
            width: 500,
            height: 30,
        }
    }

    /// The two functions that agree on this are `FUN_100135c0`, which cuts the
    /// surface, and `FUN_10011ec0`, which draws into it. Both put the stored
    /// line two pixels down its row and the comment at `0x202`, and both step
    /// by `0x30`.
    #[test]
    fn the_surface_rows_are_where_the_rasteriser_writes_them() {
        for row in 0..PER_PAGE {
            let y = row as f32 * 48.0;
            assert_eq!(source_rect(Column::When, row), (0.0, y + 2.0, 548.0, 48.0));
            assert_eq!(
                source_rect(Column::Chapter, row),
                (1024.0, y + 2.0, 548.0, 48.0)
            );
            assert_eq!(
                source_rect(Column::Comment, row),
                (0.0, y + 514.0, 986.0, 48.0)
            );
            assert_eq!(surface_pen(Column::When, row), (0, y as i32 + 2));
            assert_eq!(surface_pen(Column::Chapter, row), (1024, y as i32 + 2));
            assert_eq!(surface_pen(Column::Comment, row), (0, y as i32 + 514));
        }
    }

    /// Every column of every row has to fit, or the surface would be cut from
    /// somewhere it was never drawn.
    #[test]
    fn every_source_rect_lies_inside_the_surface() {
        for row in 0..PER_PAGE {
            for column in Column::ALL {
                let (x, y, w, h) = source_rect(column, row);
                assert!(x + w <= SURFACE.0 as f32, "{column:?} row {row} runs wide");
                assert!(y + h <= SURFACE.1 as f32, "{column:?} row {row} runs long");
            }
        }
    }

    /// The rows abut exactly: the slice is as tall as the step, so a glyph cell
    /// ends where the next row's begins.
    #[test]
    fn the_rows_abut_without_overlapping() {
        for row in 0..PER_PAGE - 1 {
            let (_, y, _, h) = source_rect(Column::When, row);
            let (_, next, _, _) = source_rect(Column::When, row + 1);
            assert_eq!(y + h, next);
        }
    }

    /// The timestamp and the chapter come off the first band of ten records and
    /// the comment off the second, so the two bands are not duplicates.
    #[test]
    fn the_comment_is_placed_by_the_other_band() {
        for row in 0..PER_PAGE {
            assert_eq!(Column::When.record_of(row), row);
            assert_eq!(Column::Chapter.record_of(row), row);
            assert_eq!(Column::Comment.record_of(row), row + 0x16);
        }
    }

    #[test]
    fn the_columns_sit_where_the_dll_puts_them() {
        let r = rect(20, 100);
        assert_eq!(
            dest_rect(Column::When, r, false, 0.0),
            (21.0, 104.5, 252.0, 24.0)
        );
        assert_eq!(
            dest_rect(Column::Chapter, r, false, 0.0),
            (282.5, 104.5, 252.0, 24.0)
        );
        assert_eq!(
            dest_rect(Column::Comment, r, false, 0.0),
            (22.0, 104.5, 494.0, 24.0)
        );
    }

    /// Host `+0x5c` moves the timestamp five right and the chapter fifteen, and
    /// leaves the comment alone -- that one is centred instead.
    #[test]
    fn english_shifts_the_stored_line_but_not_the_comment() {
        let r = rect(20, 100);
        assert_eq!(dest_rect(Column::When, r, true, 0.0).0, 26.0);
        assert_eq!(dest_rect(Column::Chapter, r, true, 0.0).0, 297.5);
        assert_eq!(dest_rect(Column::Comment, r, true, 0.0).0, 22.0);
    }

    /// `235.5 - width / 4`, clamped at zero, and nothing at all in Japanese.
    #[test]
    fn an_english_comment_is_centred_in_its_column() {
        assert_eq!(comment_centre(0, true), 235.5);
        assert_eq!(comment_centre(942, true), 0.0);
        // Past the point where the text fills the column, it stops moving
        // rather than going negative.
        assert_eq!(comment_centre(4000, true), 0.0);
        assert_eq!(comment_centre(0, false), 0.0);
        assert_eq!(comment_centre(400, false), 0.0);
    }

    /// A comment of exactly half the column's source width centres at a quarter
    /// of the destination, which is the identity the constants encode.
    #[test]
    fn the_centring_is_half_the_column_less_half_the_drawn_width() {
        let drawn = 400;
        let column = Column::Comment.dest_width();
        let on_screen = drawn as f32 * column / Column::Comment.width();
        // 11.5 short of true centre, which is the shipped constant, not 247.
        assert!(((column - on_screen) / 2.0 - comment_centre(drawn, true) - 11.5).abs() < 0.5);
    }

    #[test]
    fn english_buys_the_timestamp_one_character_and_the_comment_twenty() {
        assert_eq!(Column::When.cap(false), 0x14);
        assert_eq!(Column::When.cap(true), 0x15);
        assert_eq!(Column::Chapter.cap(false), 0x14);
        assert_eq!(Column::Chapter.cap(true), 0x14);
        assert_eq!(Column::Comment.cap(false), 0x14);
        assert_eq!(Column::Comment.cap(true), 0x28);
    }

    /// The surface is twice the screen, so the comment halves exactly; the
    /// stored line's columns are the shipped squash and are checked as such
    /// rather than rounded to a clean ratio.
    #[test]
    fn the_surface_comes_down_to_the_screen_at_the_shipped_ratios() {
        // The height is the only exact halving.
        assert_eq!(SURFACE_ROW_HEIGHT / DEST_HEIGHT, 2.0);
        // Neither width is: 494 doubled is 988, not 986.
        assert!((Column::Comment.width() / Column::Comment.dest_width() - 1.996).abs() < 0.001);
        assert!((Column::When.width() / Column::When.dest_width() - 2.175).abs() < 0.001);
    }

    fn font() -> days_font::Font {
        days_font::Font::parse(vec![0u8; days_font::TABLE_BYTES]).unwrap()
    }

    fn records() -> Vec<days_ui::atlas::Widget> {
        (0..0x20)
            .map(|i| days_ui::atlas::Widget {
                dst: rect(10, 20 * i as u32),
                src_x: 0,
                src_y: 0,
            })
            .collect()
    }

    fn filled(slot: u32) -> Slots {
        let mut slots = Slots::default();
        slots.insert(
            slot,
            Line {
                when: "2012年 1月28日(土)18:02".into(),
                chapter: "第6話".into(),
                comment: "a comment".into(),
            },
        );
        slots
    }

    /// A row with no file is not drawn at all, which is what the shipped loop
    /// does: it only draws when the host's slot query answers 1.
    #[test]
    fn only_the_slots_with_a_file_get_quads() {
        let rows = saveload_rows(&filled(3), 0);
        assert_eq!(rows.quads.len(), 3);
        assert!(saveload_rows(&Slots::default(), 0).quads.is_empty());
    }

    /// The page a row stands for moves it a whole page of slots, not a row.
    #[test]
    fn a_page_shows_its_own_ten_slots() {
        assert!(saveload_rows(&filled(3), 1).quads.is_empty());
        assert_eq!(saveload_rows(&filled(13), 1).quads.len(), 3);
    }

    fn saveload_rows(slots: &Slots, page: usize) -> Rows {
        Rows::render(&font(), slots, page, false, &records(), true, None)
    }

    /// The surface is the size the `FrameBuffer` is created at, and every quad
    /// cuts it rather than reaching past it.
    #[test]
    fn the_rendered_quads_cut_the_surface_they_were_drawn_into() {
        let rows = saveload_rows(&filled(7), 0);
        assert_eq!((rows.surface.width, rows.surface.height), SURFACE);
        for quad in &rows.quads {
            let (x, y, w, h) = quad.src;
            assert!(x + w <= rows.surface.width);
            assert!(y + h <= rows.surface.height);
        }
    }

    /// A row whose record the table does not carry is skipped rather than
    /// placed somewhere this engine chose.
    #[test]
    fn a_row_with_no_record_is_left_undrawn() {
        let short: Vec<days_ui::atlas::Widget> = records().into_iter().take(10).collect();
        let rows = Rows::render(&font(), &filled(0), 0, false, &short, true, None);
        // The comment's record is in the second band, which this table stops
        // short of, so only the stored line's two columns are placed.
        assert_eq!(rows.quads.len(), 2);
    }

    #[test]
    fn a_line_too_short_to_hold_a_chapter_is_left_alone() {
        assert_eq!(split_line("", false), (String::new(), String::new()));
        assert_eq!(split_line("x", false), ("x".to_owned(), String::new()));
    }
}
