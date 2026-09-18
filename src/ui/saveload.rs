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
//! # The list slides — on one of the two modules
//!
//! `SysMenuSDHQ.dll` changes the page on the spot: `FUN_10011ec0` refills its
//! one surface and the new ten rows are simply there. `SysMenuSD.dll` does not.
//! Its list is a strip of six page-panels stacked down the screen, and a page
//! button **slides** the strip a page at a time — twenty frames of linear
//! travel each, so page 0 to page 9 really is nine of them, 180 frames.
//! [`Strip`] is the strip and [`Slide`] is everything that moves it, with the
//! provenance for each rule in its own doc comment.
//!
//! The strip runs from y 122 to y 1627 while the screen is 450 tall, so most of
//! it is outside the list at any moment, and **nothing scissors it**: not the
//! DLL, which sets only blend and texture-stage states before each draw, and
//! not `DX9Sprite2D`, whose draw writes four vertices and calls
//! `DrawPrimitive`. What confines it is the screen's own base art.
//! `FUN_1001ad60` loads `Load.png` or `Save.png` into a full-screen sprite of
//! the module's own at `+0xd4`, and `FUN_10017e70` draws that sprite **after**
//! the list:
//!
//! ```text
//! +0x5e0   the strip's six panels                  FUN_1001a710
//! +0x80    the row under the pointer, one sprite   gated on +0x624, +0x62c
//!          the six banks' rows, ten each           FUN_1001b240
//! +0x5e4   two black bands over the letterbox      FUN_1001abb0
//! +0x5ec   the base art, full screen               FUN_1001ad60
//! +0x80    the page indicator and the rest of the hover art
//!          the expanded comment                    gated on +0x624, +0x62c
//! ```
//!
//! `Load.png` is opaque everywhere but the list — its only fully transparent
//! pixels are the window x 26..773, y 122..426 — so drawing it over the strip
//! is the clip. That is the second, non-decompiler check on `_DAT_1004bd60`:
//! the constant the panels are stacked from, 122.0, is exactly the first
//! transparent row of the player's own art, and the strip's own ink (y 3..299
//! of a 301 pitch) fits the window to the pixel at rest.
//!
//! The two black bands are the other half of the same idea. The base art is
//! only the 800x450 layout, so in 4:3 the strip runs into the letterbox
//! instead; `FUN_1001abb0` builds two `DX9Sprite2D`s coloured `0xff000000`,
//! the width of the display and the height of the letterbox plus one, at
//! `y = -0.5` and `y = height - letterbox - 0.5`, and the draw puts them
//! between the rows and the base art. It builds them only when host `+0xcc`,
//! the display setting, says 4:3 — the one display with a letterbox to spill
//! into.
//!
//! **The row highlight and the expanded comment are the two things the slide
//! takes away.** Both draws are gated on `+0x624` and `+0x62c` being clear.
//! Nothing else is: `FUN_1001ca60` gates every widget on the confirm popup and
//! on nothing else, so a page button clicked mid-slide is answered and the new
//! step starts from wherever the strip has got to. Only the keyboard waits —
//! `FUN_1001ce60` wraps its whole arrow block in the same two flags.
//!
//! ## The drag-scroll is recovered and not implemented
//!
//! The same members carry a second interaction. `FUN_1001e1f0`'s `+0x58` arm
//! drags the strip with the pointer and its `+0x62c` arm settles the drag:
//! clamped at 0 and at `pitch * 5`, it divides the scroll by a tenth of a page
//! to find the **row** it has come to rest on, keeps that row in `+0x5d0` and
//! slides to `(pitch / 10) * row`. `+0x5d0` is why [`Strip::rest`] has a
//! row-granular term and why `FUN_10018fd0` offsets the ten live rows by it.
//! That settle has a page decision of its own, `FUN_1001f130`, which is not
//! `FUN_1001f8c0` — see [`Settled::Same`].
//!
//! None of it is implemented here: it is a different interaction from the page
//! slide, and this engine always leaves `+0x5d0` at zero.
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
//! on the screen. [`Layout::surface`], [`source_rect`] and [`dest_rect`] are
//! those three facts.
//!
//! **The numbers below are School Days HQ's.** Shiny Days' `FUN_1001b240` and
//! `FUN_10018fd0` do all of the same things with a different set, and the two
//! sets are [`Layout`]. The largest difference is the order: this module puts
//! the timestamp at the left of the row and the chapter after it, and Shiny Days
//! puts the chapter in a narrow cell at the left with the timestamp beside it.
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
//! `FUN_10011ec0` accumulates. See [`comment_centre`]. Shiny Days makes the
//! same three shifts by the same two constants and centres from 226.5.
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
//! # The table cannot be found from the hit map alone
//!
//! Twenty of the thirty-two records do not look like the region they place: a
//! row is one sprite the width of the row behind two hit regions covering half
//! of it each, and a comment panel is three rows tall. Only the twelve buttons
//! in between can anchor [`days_ui::atlas::find`], and on Shiny Days that is not
//! enough — `SysMenuSD.dll` holds the play-data list's table, whose records
//! reproduce all thirty-two of this screen's hit boxes to the pixel, and the
//! generic search anchors there and draws every row a third of its width.
//! [`relocate`] is the way in: anchor on the buttons and read the rest at the
//! indices the shipped code reads them at.
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
//! the bottom of the screen. Shiny Days' `FUN_10019a40` is the same screen with
//! a buffer of its own for the lines and its own trims; both sets are
//! [`Layout::tip`].
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
use days_ui::atlas::{self, Atlas};
use days_ui::cmap::Rect;
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

/// How many widgets the screen has, from its own hit map.
pub const WIDGETS: usize = 0x20;

/// The first record of the ten page buttons, which is where this screen's table
/// can be anchored: `FUN_100135c0` and `FUN_1001b240` both place button `n`
/// from record `n + 10`, CLOSE from record 20 and the route map from record 21.
const PAGE_RECORD: usize = 10;

/// Puts the whole table into an atlas the hit map could only half place.
///
/// Every one of the thirty-two regions is record `region` of one run — rows
/// 0..9, page buttons 10..19, CLOSE 20, the route map 21, the comment panels
/// 22..31 — but twenty of those records do not look like the region they place.
/// A row is one sprite the width of the row behind two hit regions covering
/// half of it each, and a comment panel is about three rows tall because it
/// doubles as the expanded comment's panel, so
/// [`days_ui::atlas::find`] can only anchor the twelve buttons in between.
///
/// On School Days HQ it extrapolates the other twenty from those twelve at the
/// table's own stride and gets them right. On Shiny Days it does not:
/// `SysMenuSD.dll` holds a second run — the play-data list's, which splits a row
/// into the two records this screen only splits into two hit regions — that
/// reproduces all thirty-two boxes to the pixel, so the search anchors there
/// and every row is drawn a third of its width. This anchors on the buttons,
/// whose records the hit map does reproduce, and reads the rest at the indices
/// the shipped code reads them at.
///
/// Returns whether it did. A table that will not anchor leaves the screen with
/// no list to draw, so the caller refuses the screen rather than drawing one
/// from the wrong offsets.
pub fn relocate(atlas: &mut Atlas, dll: &[u8], boxes: &[Rect]) -> bool {
    if boxes.len() != WIDGETS || atlas.widgets.len() != WIDGETS {
        log::warn!(
            "the save/load screen has {} regions and {} widgets, not {WIDGETS} of each",
            boxes.len(),
            atlas.widgets.len()
        );
        return false;
    }
    // Every record between the last row and the first comment panel: the ten
    // page buttons, CLOSE and the route map.
    let anchors: Vec<(usize, Rect)> = (PAGE_RECORD..COMMENT_RECORD)
        .map(|record| (record, boxes[record]))
        .collect();
    let bands = [
        atlas::Band {
            region: 0,
            record: 0,
            count: PER_PAGE,
        },
        atlas::Band {
            region: COMMENT_RECORD,
            record: COMMENT_RECORD,
            count: PER_PAGE,
        },
    ];
    let Some(base) = atlas::relocate(atlas, dll, boxes, &anchors, &bands) else {
        log::warn!("no record table in the DLL places the save/load screen's rows");
        return false;
    };
    atlas.segments = vec![(0, base, WIDGETS)];
    // What follows the table is the page indicator — School Days HQ binds two
    // sprites to records `page + 0x20` and `page + 0x2a`, Shiny Days one to
    // `page + 0x20` — and nothing on this screen draws either yet. They are
    // carried as [`Atlas::extras`] on the same best-effort terms every other
    // screen's trailing records are.
    atlas.extras = (WIDGETS..)
        .map_while(|index| atlas::record_at(dll, base, index))
        .take(TRAILING_RECORDS)
        .collect();
    log::info!(
        "save/load: {WIDGETS} widgets and {} trailing records from the table at {base:#x}",
        atlas.extras.len()
    );
    true
}

/// The first of the ten comment panels, which is [`Column::record_of`] for row
/// zero: the same `+ 0x16` the shipped code adds.
const COMMENT_RECORD: usize = Column::Comment.record_of(0);

/// How far past the table to keep reading the page indicator's records.
const TRAILING_RECORDS: usize = 32;

/// The slot a row of a page stands for, from `FUN_10011d50`.
pub fn slot_of(page: usize, row: usize) -> u32 {
    (page * PER_PAGE + row) as u32
}

/// Where one module puts the three columns of a row, and its expanded comment.
///
/// The two titles draw this screen from the same code with different numbers,
/// and the numbers are not a variation on a theme: School Days HQ puts the
/// timestamp first and the chapter after it, Shiny Days puts the chapter in a
/// narrow cell at the left and the timestamp beside it, squeezed harder. A row
/// laid out with the wrong set is not a few pixels off — it draws the timestamp
/// across the cell divider its own art has.
///
/// Every field is the constant the module's own instruction loads, with the
/// operand width taken from the mnemonic rather than from Ghidra's
/// `(float)_DAT_...`, which narrows a double at the use site.
///
/// ```text
/// School Days HQ   SysMenuSDHQ.dll   sprites  FUN_100135c0
///                                    glyphs   FUN_10011ec0
///                                    tooltip  FUN_10012900
///
/// Shiny Days       SysMenuSD.dll     sprites  FUN_1001b240
///                                    glyphs   FUN_10018fd0
///                                    tooltip  FUN_10019a40
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    /// Which module this is, for diagnostics.
    pub module: &'static str,
    /// The off-screen surface the columns are rasterised into, from the
    /// `FrameBuffer` slot `+8` call that builds it.
    pub surface: (u32, u32),
    /// The three columns, in [`Column::ALL`] order.
    pub columns: [ColumnLayout; 3],
    /// How far down its record every column is drawn.
    pub dest_y: f32,
    /// What an English comment's centring is measured back from — see
    /// [`comment_centre`].
    pub centre_from: f32,
    /// The expanded comment.
    pub tip: Tip,
    /// The strip the list scrolls on, or `None` for a list that is one page at
    /// a time and changes page instantly.
    pub strip: Option<Strip>,
}

/// The strip of page-panels Shiny Days' list scrolls through, from
/// `FUN_1001a710`, `FUN_1001ed40` and `FUN_1001e1f0`.
///
/// The list is not redrawn when the page changes: the module keeps [`banks`]
/// pages' worth of rows stacked down a strip and **slides** the strip, one page
/// at a time. `FUN_1001a710` gives each bank a sprite of the same art, stacked
/// [`Strip::pitch`] apart, and `FUN_1001e1f0` re-places all six of them and all
/// sixty rows every frame against one scroll offset. [`Slide`] is that offset
/// and everything that moves it.
///
/// [`banks`]: Strip::banks
#[derive(Debug, Clone, Copy)]
pub struct Strip {
    /// The art behind one page of rows, loaded by `FUN_1001a710` and drawn once
    /// per bank. The engine reads its size at runtime rather than carrying
    /// numbers, because the module does: `+0x5d4` and `+0x5d8` are the image's
    /// own width and height plus one.
    pub art: &'static str,
    /// How many pages of rows the strip holds at once, from the six
    /// `FrameBuffer`s `FUN_1001b240` builds and the `local_28 < 6` loops that
    /// fill, place and draw them.
    pub banks: usize,
    /// How many pages the window keeps above the page being shown, once it has
    /// any to keep.
    ///
    /// The module's own ladder, not a formula: `FUN_10018fd0` fills bank 0 from
    /// page `0` for pages 0 and 1, from `page - 2` for pages 2 to 6, and from
    /// `page - 3`, `- 4`, `- 5` for pages 7, 8 and 9 — which is `page - 2`
    /// clamped to `0 ..= PAGES - banks`. See [`Strip::window_top`].
    pub lead: usize,
    /// Where the first panel's top edge sits down the screen, `_DAT_1004bd60` —
    /// an `faddl`, so the double 122.0.
    pub top: f32,
    /// How much of a step's travel one frame covers, `_DAT_10049738` — an
    /// `fdivl`, so the double 20.0.
    pub frames: f32,
    /// What the step's own counter climbs by each frame, `_DAT_1004a020` — an
    /// `faddl`, so the double nearest the float 0.05.
    ///
    /// The counter is what *ends* the step, at `_DAT_100497b0` (1.0), and
    /// [`Strip::frames`] is what moves it; the two agree on twenty, which is
    /// the cross-check. Both are kept because both are shipped: twenty
    /// accumulations of this constant reach 1.0000001, so the step ends on the
    /// twentieth frame and not the twenty-first.
    pub step: f32,
}

impl Strip {
    /// How far apart the panels are stacked: the art's own height plus one, the
    /// same `+ 1.0` `FUN_1001a710` adds to both of the image's dimensions.
    pub fn pitch(height: u32) -> f32 {
        height as f32 + 1.0
    }

    /// Which page bank 0 holds while `page` is the one being shown.
    ///
    /// Three independent ladders agree on this, which is what confirms it:
    /// `FUN_10018fd0` fills bank `b` from page `window_top + b`,
    /// `FUN_1001f6f0` re-seats the scroll at `pitch * (page - window_top)`, and
    /// `FUN_1001f8c0` decides the new page from the scroll measured against the
    /// same difference. See [`Strip::lead`] for the arms themselves.
    pub fn window_top(&self, page: usize) -> usize {
        window_top(page, self.lead, self.banks, PAGES)
    }

    /// Where the scroll comes to rest with `page` showing, from `FUN_1001f6f0`.
    ///
    /// The shipped function adds a row-granular term, `(pitch / 10) * +0x5d0`,
    /// which only the drag-scroll ever makes non-zero — see the module docs for
    /// why that interaction is not implemented here. Page 9's arm leaves the
    /// term out altogether.
    pub fn rest(&self, page: usize, pitch: f32) -> f32 {
        pitch * (page - self.window_top(page)) as f32
    }

    /// Where a step to `next` is heading, from `FUN_1001ed40`.
    ///
    /// The shipped code is a seven-arm switch on the page being left, the page
    /// being entered and which way the step runs, and it is transcribed here
    /// arm for arm rather than reduced. It is **identical** to
    /// `pitch * (next - window_top(page))` for all eighteen reachable
    /// transitions, which `the_step_switch_is_the_window` asserts — two ways of
    /// arriving at the same number, which is the standard a recovered rule is
    /// held to here.
    pub fn target(&self, page: usize, next: usize, forward: bool, pitch: f32) -> f32 {
        let banks = |k: usize| pitch * k as f32;
        if (next == 2 && forward) || (next == 6 && !forward) {
            banks(2)
        } else if next == 0 && !forward {
            0.0
        } else if next == 9 && forward {
            banks(5)
        } else if next == 8 {
            banks(4)
        } else if next == 7 && !forward {
            banks(3)
        } else if (next == 1 && forward) || page == next + 1 {
            banks(1)
        } else {
            banks(3)
        }
    }

    /// Which page the scroll now shows, from `FUN_1001f8c0`.
    ///
    /// Measured against `k = page - window_top(page)`, the bank the shown page
    /// occupies:
    ///
    /// ```text
    /// if (k - 1) * pitch <  scroll {
    ///     if (k + 1) * pitch <= scroll { the next page } else { neither }
    /// } else { the previous page }
    /// ```
    ///
    /// **The second compare is `<=`, not `<`.** Ghidra prints both as
    /// `a < b != (a == b)`; the instructions are `FCOMPP; FNSTSW AX` followed
    /// by `TEST AH,0x41; JP` for the `<=` at `0x1001f94f` and `TEST AH,0x1;
    /// JNZ` for the `<` at `0x1001f8fc`. Read as `<`, a step that lands exactly
    /// on `(k + 1) * pitch` — which is every step from a page button — never
    /// advances the page, and the screen looks like it ships a bug it does not.
    ///
    /// `FUN_1001ef80`'s arms for pages 0 and 9 are this function inlined with
    /// the half that cannot be reached from there dropped, and the general form
    /// gives the same answer on both, which is the second check on it.
    pub fn settled(&self, page: usize, scroll: f32, pitch: f32) -> Settled {
        let k = (page - self.window_top(page)) as f32;
        if (k - 1.0) * pitch < scroll {
            if (k + 1.0) * pitch <= scroll {
                Settled::Next
            } else {
                Settled::Same
            }
        } else {
            Settled::Previous
        }
    }
}

/// Which page a strip of `panels` over `pages` starts at, keeping `lead` pages
/// above the page showing wherever there is room for them.
///
/// Both of this module's slot lists window their strip the same way, from two
/// different functions, and each is written out one page at a time rather than
/// as arithmetic: `FUN_10018fd0` for the save/load screen's and
/// `FUN_100267c0` for the replay play-data list's — see
/// [`Strip::window_top`] and [`crate::ui::replay_pages::window_top`] for the
/// two ladders. This is the half they share.
pub fn window_top(page: usize, lead: usize, panels: usize, pages: usize) -> usize {
    page.saturating_sub(lead).min(pages - panels)
}

/// Which page a settling step has brought the strip to. See [`Strip::settled`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// The next page down the list.
    Next,
    /// The previous one.
    Previous,
    /// Neither: the scroll stopped inside the page that was already showing.
    ///
    /// `FUN_1001f8c0` leaves the direction it reports **uninitialised** on this
    /// arm, and nothing in the retail build reaches it. `FUN_1001f8c0` has
    /// four callers and they are all in `FUN_1001ef80`; `FUN_1001ef80` runs
    /// only for a step `FUN_1001ed40` started, because `+0x630` and `+0x628`
    /// are written nowhere else; and every target `FUN_1001ed40` sets is
    /// `pitch * k`, so the scroll `FUN_1001ef80` measures is always exactly on
    /// a page. The drag-release settle, which is the one thing that can leave
    /// the scroll inside a page, has its own page decision in `FUN_1001f130`
    /// and never calls this. Both caller sets were taken from Ghidra's
    /// reference index and from a raw scan of `.text` for the call
    /// displacement, which agree.
    ///
    /// So there is no direction to carry a multi-page jump on, and
    /// [`Slide::tick`] stops the jump and says so rather than choosing one.
    Same,
}

/// The slide the list makes when the page changes, from `FUN_1001e1f0`.
///
/// Every field is one of the module's own members and the arithmetic is the
/// shipped arithmetic. A step covers a page of travel in
/// [`Strip::frames`] frames, linearly, and then **snaps** onto its target
/// rather than keeping the twentieth accumulation — the same shape as
/// [`crate::ui::dress::Slide`]. A jump across more than one page is a run of
/// one-page steps back to back, so page 0 to page 9 really is nine of them.
///
/// # Output pixels against layout pixels
///
/// The module keeps the scroll at `+0xb4` in **output** pixels: every target it
/// is compared with is `pitch * k * +0x98`, the display scale already in it.
/// This engine keeps the same quantity in the 800x450 layout space the rest of
/// the screen is written in, which is the shipped value divided by that scale
/// and so the same picture — a step's travel divides by twenty either way.
/// Keeping it in layout space is also what lets the window be resized
/// mid-slide, where the shipped members would leave the strip a scale out.
#[derive(Debug, Clone)]
pub struct Slide {
    strip: Strip,
    /// The art's own size, for the panels' `+ 1`.
    art: (u32, u32),
    /// `+0x5d8`: the distance between banks, the art's height plus one.
    pitch: f32,
    /// `+0x5c4`: the page showing. Nothing outside a settling step writes it —
    /// a page button starts a slide and the page follows.
    page: usize,
    /// `+0xb4`: how far down the strip has been pulled.
    scroll: f32,
    /// `+0x620`: the scroll this step is heading for.
    target: f32,
    /// `+0x618`: the distance it has to cover.
    travel: f32,
    /// `+0x61c`: the counter the step runs on, climbing to 1.0.
    counter: f32,
    /// `+0x624`: a step is running.
    running: bool,
    /// `+0x628`, `+0x634`, `+0x638` and `+0x63c` together.
    run: Option<Run>,
}

/// A jump across more than one page, from `FUN_1001eee0`.
#[derive(Debug, Clone, Copy)]
struct Run {
    /// `+0x634`: how many one-page steps the jump is.
    steps: usize,
    /// `+0x638`: how many have settled.
    done: usize,
    /// `+0x63c`: which way they run, 1 forward.
    forward: bool,
}

impl Slide {
    /// Seats the strip under `page`, the way `FUN_1001b240` leaves it: the
    /// banks filled from [`Strip::window_top`] and the scroll at
    /// [`Strip::rest`].
    pub fn new(strip: Strip, art: (u32, u32), page: usize) -> Slide {
        let pitch = Strip::pitch(art.1);
        Slide {
            strip,
            art,
            pitch,
            page,
            scroll: strip.rest(page, pitch),
            target: 0.0,
            travel: 0.0,
            counter: 0.0,
            running: false,
            run: None,
        }
    }

    /// The page showing, which is the page the ten live rows belong to.
    pub fn page(&self) -> usize {
        self.page
    }

    /// Which page bank 0 holds.
    pub fn window_top(&self) -> usize {
        self.strip.window_top(self.page)
    }

    /// The pages the banks hold, in bank order: `FUN_10018fd0`'s own loop.
    pub fn window(&self) -> std::ops::Range<usize> {
        let top = self.window_top();
        top..top + self.strip.banks
    }

    /// Which bank the page showing occupies, and so which one's rows the ten
    /// live records and the expanded comment belong to.
    pub fn shown(&self) -> usize {
        self.page - self.window_top()
    }

    /// Whether the next tick moves the strip.
    pub fn moving(&self) -> bool {
        self.running
    }

    /// How far down one bank's rows are drawn from where their records put
    /// them, in layout pixels.
    ///
    /// `FUN_1001e1f0` adds `pitch * bank - scroll` to every one of the sixty
    /// row sprites' destinations every frame, which is why a row's quad carries
    /// where it sits **in its bank** and this is added at compose time.
    pub fn offset(&self, bank: usize) -> f32 {
        self.pitch * bank as f32 - self.scroll
    }

    /// Where one bank's panel of the strip art lands, in layout space, as
    /// `(x, y, width, height)`.
    ///
    /// `FUN_1001a710` and `FUN_1001e1f0` place it at `x = -0.5`,
    /// `y = (bank * pitch + letterbox + top) * scale - 0.5 - scroll`, sized
    /// `scale * (width + 1, height + 1)` — so the panels are `pitch` apart and
    /// `pitch` tall and tile with no seam, the art stretched by the one pixel.
    /// The half-pixel outset is the same one every sprite in this module has;
    /// the module takes it off after the display scale and this engine takes it
    /// off before, as it does everywhere else, which is less than an output
    /// pixel on a flat white panel.
    pub fn panel(&self, bank: usize) -> (f32, f32, f32, f32) {
        (
            -0.5,
            self.strip.top + self.offset(bank) - 0.5,
            self.art.0 as f32 + 1.0,
            self.art.1 as f32 + 1.0,
        )
    }

    /// Asks for a page, the way `FUN_1001cb00`'s page-button arm does.
    ///
    /// The page next to the one showing is one step; anything further is a run
    /// of them. The page already showing is nothing at all — and a click
    /// arriving mid-slide is **not** refused: `FUN_1001ca60` gates the widgets
    /// on the confirm popup and on nothing else, so the new step simply starts
    /// from wherever the strip has got to. Only the keyboard is inert while the
    /// strip moves; `FUN_1001ce60` wraps its whole arrow block in
    /// `+0x624 == 0 && +0x62c == 0`.
    pub fn go(&mut self, to: usize) {
        if to == self.page {
            return;
        }
        if to + 1 == self.page || to == self.page + 1 {
            self.step(to, to == self.page + 1);
            return;
        }
        // `FUN_1001eee0`: record the direction and the distance, then run that
        // many one-page steps.
        let forward = to > self.page;
        self.run = Some(Run {
            steps: to.abs_diff(self.page),
            done: 0,
            forward,
        });
        let next = if forward {
            self.page + 1
        } else {
            self.page - 1
        };
        self.step(next, forward);
    }

    /// Starts one step, from `FUN_1001ed40`.
    fn step(&mut self, next: usize, forward: bool) {
        self.target = self.strip.target(self.page, next, forward, self.pitch);
        self.travel = self.target - self.scroll;
        self.counter = 0.0;
        self.running = true;
    }

    /// One host tick of the slide, from `FUN_1001e1f0`'s `+0x624` arm.
    ///
    /// Answers the page the banks should now be filled from, on the tick a step
    /// settles. `FUN_1001ef80` calls `FUN_10018fd0` whether or not the page
    /// changed, so this answers on every settle.
    pub fn tick(&mut self) -> Option<usize> {
        if !self.running {
            return None;
        }
        self.counter += self.strip.step;
        self.scroll += self.travel / self.strip.frames;
        if self.counter < 1.0 {
            return None;
        }
        self.scroll = self.target;
        self.counter = 0.0;
        self.running = false;

        // The run's bookkeeping happens before the settle, so that the settle
        // sees whether there is another step to start.
        if let Some(run) = &mut self.run {
            run.done += 1;
            if run.done == run.steps {
                self.run = None;
            }
        }
        // `FUN_1001ef80`: which page the scroll shows, then re-seat it, then
        // the banks, then the next step of the run.
        match self.strip.settled(self.page, self.scroll, self.pitch) {
            Settled::Next => self.page += 1,
            Settled::Previous => self.page -= 1,
            Settled::Same => {
                if self.run.take().is_some() {
                    log::warn!(
                        "the save/load list settled inside page {} with a jump still running, \
                         which the shipped code has no direction for",
                        self.page
                    );
                }
            }
        }
        self.scroll = self.strip.rest(self.page, self.pitch);
        if let Some(run) = self.run {
            let next = if run.forward {
                self.page + 1
            } else {
                self.page - 1
            };
            self.step(next, run.forward);
        }
        Some(self.page)
    }
}

/// One column's place on the surface and on the screen.
#[derive(Debug, Clone, Copy)]
pub struct ColumnLayout {
    /// Where this column's pen starts across the surface, and how wide the
    /// sprite cuts it.
    pub span: (f32, f32),
    /// The x it is offset by inside its record, and the extra shift English
    /// adds. The comment has no shift — it is centred instead.
    pub x: f32,
    pub english_shift: f32,
    /// How wide it is drawn. Every column is cut wider than it is drawn, so
    /// every one is squeezed horizontally.
    pub width: f32,
}

/// The expanded comment's geometry.
#[derive(Debug, Clone, Copy)]
pub struct Tip {
    /// A buffer of its own, or `None` when the lines go into the same surface
    /// as the rows.
    pub surface: Option<(u32, u32)>,
    /// Where line 0 is rasterised and cut, and the step to the next.
    pub origin: (f32, f32),
    pub pitch: f32,
    /// How tall each line's slice of that surface is.
    pub height: f32,
    /// How far apart the lines are drawn, and how tall each is drawn.
    pub line_pitch: f32,
    pub line_height: f32,
    /// How many rows of the list the panel's record spans. The record is about
    /// three rows tall and the panel is that divided by the lines it needs.
    pub panel_rows: f32,
    /// What comes off the panel when it is showing fewer than three lines, and
    /// what comes off again when it opens upwards.
    pub trim: [f32; 2],
    pub deep_trim: f32,
    /// What a panel opening upwards is pushed down by, past the rows it gave
    /// back, and how far its lines move with it.
    pub deep_step: f32,
    pub deep_text: [f32; 2],
}

impl Layout {
    /// School Days HQ's, from `SysMenuSDHQ.dll`.
    ///
    /// The surface is `(0x800, 0x400)` — `FUN_10011ec0` clears it as 0x400 rows
    /// of 0x2000 bytes, which is the same 2048 pixels of 32 bits. The columns
    /// are `_DAT_1003b138` (548.0f) and `_DAT_1003b110` (986.0f) wide on it, at
    /// pen starts 0, `0x400` and 0; they are drawn at `_DAT_10039798` (1.0),
    /// `_DAT_1003b118` (262.5) and `_DAT_10039748` (2.0) into their records,
    /// `_DAT_1003b128` (252.0) and `_DAT_1003b0d0` (494.0) wide, `_DAT_1003b0c8`
    /// (4.5) down. The English shifts are `_DAT_1003b140` (5.0f) and
    /// `_DAT_1003b13c` (15.0f).
    pub const SCHOOL_DAYS_HQ: Layout = Layout {
        module: "SysMenuSDHQ.dll",
        surface: (0x800, 0x400),
        columns: [
            ColumnLayout {
                span: (0.0, 548.0),
                x: 1.0,
                english_shift: 5.0,
                width: 252.0,
            },
            ColumnLayout {
                span: (1024.0, 548.0),
                x: 262.5,
                english_shift: 15.0,
                width: 252.0,
            },
            ColumnLayout {
                span: (0.0, 986.0),
                x: 2.0,
                english_shift: 0.0,
                width: 494.0,
            },
        ],
        dest_y: 4.5,
        centre_from: 235.5,
        tip: Tip {
            surface: None,
            origin: (1024.0, 514.0),
            pitch: 64.0,
            height: 64.0,
            line_pitch: 32.0,
            line_height: 32.0,
            panel_rows: 3.0,
            trim: [2.0, 0.0],
            deep_trim: 0.0,
            deep_step: 0.0,
            deep_text: [64.0, 32.0],
        },
        // `FUN_100135c0` builds one surface and `FUN_10011ec0` refills it when
        // the page changes. Nothing on this screen scrolls.
        strip: None,
    };

    /// Shiny Days', from `SysMenuSD.dll`.
    ///
    /// Six surfaces rather than one — `FUN_1001b240` builds a `(0x400, 0x400,
    /// 0x208888)` `FrameBuffer` per page-bank for the page slide, and
    /// `FUN_10018fd0` fills each with ten rows — so a page is 1024 wide here,
    /// not 2048. The chapter's cut starts at `_DAT_1004bd78` (600.0f) and runs
    /// `_DAT_1004bd98` (548.0f), so it ends at 1148 on a surface 1024 wide; the
    /// two characters it ever holds are nowhere near that edge. The columns are
    /// drawn at `_DAT_1004bd80` (95.0), `_DAT_10049738` (20.0) and
    /// `_DAT_10049760` (2.0) into their records, `_DAT_1004bd88` (189.0),
    /// `_DAT_1004bd70` (252.0) and `_DAT_1004bd10` (494.0) wide, `_DAT_10049760`
    /// (2.0) down, with the same English shifts `_DAT_1004bda0` (5.0f) and
    /// `_DAT_1004bd9c` (15.0f).
    ///
    /// **The chapter is the left-hand column here**, at 20 into the row against
    /// the timestamp's 95, which is why the two cells of the row's hover art
    /// are one narrow and one wide. Its 252-wide slot overlaps the timestamp's
    /// and is drawn under it; only the two characters at its left edge are ever
    /// filled, so nothing collides.
    ///
    /// The same numbers place the module's other slot list — see
    /// [`crate::ui::replay_pages`], which reaches them through `FUN_10027c10`
    /// and `FUN_100267c0` instead and differs only in rasterising two pixels
    /// above its own cut.
    ///
    /// The tooltip has a buffer of its own, `(0x400, 0x100, 0x208888)`, with
    /// the pen at `(0, 2)` stepping `_DAT_1004bd30` (64.0) and the sprite
    /// cutting `_DAT_1004bd38` (986.0f) by `_DAT_1004bd3c` (64.0f) there. Its
    /// lines are drawn `_DAT_1004bd40` (29.0) apart and `_DAT_1004bd28` (32.0)
    /// tall, and a panel opening upwards gives back `_DAT_1004bd50` (3.0f) and
    /// moves its lines by `_DAT_1004bd4c` (58.0f) or `_DAT_1004bd48` (29.0f).
    pub const SHINY_DAYS: Layout = Layout {
        module: "SysMenuSD.dll",
        surface: (0x400, 0x400),
        columns: [
            ColumnLayout {
                span: (0.0, 548.0),
                x: 95.0,
                english_shift: 5.0,
                width: 189.0,
            },
            ColumnLayout {
                span: (600.0, 548.0),
                x: 20.0,
                english_shift: 15.0,
                width: 252.0,
            },
            ColumnLayout {
                span: (0.0, 986.0),
                x: 2.0,
                english_shift: 0.0,
                width: 494.0,
            },
        ],
        dest_y: 2.0,
        centre_from: 226.5,
        tip: Tip {
            surface: Some((0x400, 0x100)),
            origin: (0.0, 2.0),
            pitch: 64.0,
            height: 64.0,
            line_pitch: 29.0,
            line_height: 32.0,
            panel_rows: 3.0,
            trim: [3.0, 3.0],
            deep_trim: 3.0,
            deep_step: 3.0,
            deep_text: [58.0, 29.0],
        },
        strip: Some(Strip {
            art: "System/SaveLoad/SaveLoadList.png",
            banks: 6,
            lead: 2,
            top: 122.0,
            frames: 20.0,
            step: 0.05,
        }),
    };

    /// Which module's save/load screen this is.
    ///
    /// By the menu module's own export table, which is how
    /// [`crate::install::binaries`] recognises a menu module in the first place:
    /// `SysMenuSD.dll` publishes `_GetBGMVolume@0` and `_GetSEVolume@0` and
    /// `SysMenuSDHQ.dll` publishes neither, and those two names are the whole of
    /// the difference between the two export tables. That is a statement about
    /// the module, which is what owns the layout — the alternative, keying on
    /// the shape of the recovered table, would be reading the answer off the
    /// thing being laid out.
    ///
    /// **The test is one-sided**, and that is not a tidy answer: Shiny Days is
    /// recognised by what it publishes and School Days HQ is what is left, not
    /// what was recognised. `SysMenuSDHQ.dll`'s export table is a subset of
    /// `SysMenuSD.dll`'s, so it offers nothing to test for, and a module that is
    /// neither is laid out as School Days HQ — which is what this engine did for
    /// every module before either set existed.
    pub fn of(dll: &[u8]) -> &'static Layout {
        let shiny = days_route::pe::Image::parse(dll).is_ok_and(|image| {
            SHINY_DAYS_EXPORTS
                .iter()
                .all(|name| image.exports.contains_key(*name))
        });
        if shiny {
            &Layout::SHINY_DAYS
        } else {
            &Layout::SCHOOL_DAYS_HQ
        }
    }

    /// This column's place, indexed the way [`Column::ALL`] is ordered.
    pub fn column(&self, column: Column) -> &ColumnLayout {
        &self.columns[column as usize]
    }
}

/// The two exports `SysMenuSD.dll` adds to what `SysMenuSDHQ.dll` publishes,
/// and the only difference between the two tables. See [`Layout::of`] and
/// [`crate::install::binaries`].
const SHINY_DAYS_EXPORTS: [&str; 2] = ["_GetBGMVolume@0", "_GetSEVolume@0"];

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
    /// cuts it. The origins are the rasterising loop's own pen starts.
    pub fn surface_span(self, layout: &Layout) -> (f32, f32) {
        layout.column(self).span
    }

    /// The width of the sprite that cuts this column out of the surface.
    pub fn width(self, layout: &Layout) -> f32 {
        self.surface_span(layout).1
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
    /// `FUN_1001b240` reads `DAT_10057000` at the same two indices.
    pub const fn record_of(self, row: usize) -> usize {
        match self {
            Column::When | Column::Chapter => row,
            Column::Comment => row + 0x16,
        }
    }

    /// The x this column is offset by inside its record, with the extra shift
    /// English adds already in it. See [`Layout`] for both modules' values.
    pub fn dest_x(self, layout: &Layout, english: bool) -> f32 {
        let column = layout.column(self);
        column.x + if english { column.english_shift } else { 0.0 }
    }

    /// How wide this column is drawn.
    pub fn dest_width(self, layout: &Layout) -> f32 {
        layout.column(self).width
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

/// How tall every column is drawn, `_DAT_1003a860` — an `fmull`, so the double
/// 24.0. Half the surface's row, which is why the text is rasterised at the
/// font's own cell and comes down to size in the blit.
pub const DEST_HEIGHT: f32 = 24.0;

/// The rectangle of [`Layout::surface`] this column of this row occupies, as
/// `(x, y, width, height)`.
///
/// From `FUN_100135c0`'s `DX9Sprite2D` slot `+0x1c` calls, whose two `/ width`
/// arguments are the x pair and whose two `/ height` arguments are the y pair.
/// `FUN_10011ec0` rasterises into the same places, which is the cross-check.
pub fn source_rect(layout: &Layout, column: Column, row: usize) -> (f32, f32, f32, f32) {
    let (x, width) = column.surface_span(layout);
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
pub fn surface_pen(layout: &Layout, column: Column, row: usize) -> (i32, i32) {
    let (x, y, _, _) = source_rect(layout, column, row);
    (x as i32, y as i32)
}

/// Where this column of this row is drawn, in the 800x450 layout space the
/// widget records use, as `(x, y, width, height)`.
///
/// `record` is the row's own record — [`Column::record_of`] says which of the
/// two bands — and `centre` is [`comment_centre`], which is zero for every
/// column but an English comment.
pub fn dest_rect(
    layout: &Layout,
    column: Column,
    record: days_ui::cmap::Rect,
    english: bool,
    centre: f32,
) -> (f32, f32, f32, f32) {
    (
        record.x as f32 + column.dest_x(layout, english) + centre,
        record.y as f32 + layout.dest_y,
        column.dest_width(layout),
        DEST_HEIGHT,
    )
}

/// How far right an English comment is pushed, from `FUN_10011ec0` and
/// `FUN_10018fd0`.
///
/// `centre_from - width / 4`, clamped to zero. Both are an `fsubrl` and an
/// `fdivl` against an `fcompl` zero — `_DAT_1003b0d8` (235.5) and
/// `_DAT_1003b0e0` (4.0) in School Days HQ, `_DAT_1004bd18` (226.5) and
/// `_DAT_1004bd20` (4.0) in Shiny Days. `width` is the advance total the
/// rasterising loop accumulated, in surface pixels, and the column comes down
/// to the screen at very nearly half, so dividing by four is half the drawn
/// width on screen. Taking that from [`Layout::centre_from`] centres the
/// comment in its 494-wide column, a few pixels short of true centre. Japanese
/// comments are not moved at all.
pub fn comment_centre(layout: &Layout, width: i32, english: bool) -> f32 {
    if !english {
        return 0.0;
    }
    (layout.centre_from - width as f32 / 4.0).max(0.0)
}

/// How many lines the expanded comment can run to, from `FUN_10012900`'s own
/// clamp.
pub const TIP_LINES: usize = 3;

/// The last row whose panel can open downwards, from `FUN_10012900`'s and
/// `FUN_10019a40`'s `param_1 < 8` test.
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
    /// not in [`Bank::surface`], and it is the record's **full** height however
    /// short the panel is drawn — so a one-line panel is the same art squashed,
    /// which is what the shipped sprite does.
    pub panel: Quad,
    /// The lines, cut from [`Bank::surface`].
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
    /// wrapped line count: both modules divide the capped length by the
    /// per-line cap and switch on that. The two agree for Japanese and can
    /// differ for English, where the wrap is by word; the shipped formula is
    /// kept rather than the one that would agree.
    ///
    /// A panel showing one line is a third of its record and one showing two is
    /// two thirds, less [`Tip::trim`]; a panel opening upwards gives back
    /// [`Tip::deep_trim`] as well and is pushed down past the thirds it did not
    /// use by [`Tip::deep_step`].
    pub fn place(
        layout: &Layout,
        row: usize,
        record: days_ui::atlas::Widget,
        widths: &[i32],
        chars: usize,
        english: bool,
    ) -> Tooltip {
        let tip = &layout.tip;
        let cap = Column::Comment.cap(english);
        let height = record.dst.height as f32;
        let row_of_panel = height / tip.panel_rows;
        let deep = row > LAST_ROW_OPENING_DOWN;

        // School Days HQ's `FUN_10012900` writes the panel's shift and the
        // text's only on the branches a deep row takes, and zeroes them only on
        // the three-line one. A shallow row with one or two lines therefore
        // reads **two uninitialised floats** -- a shipped bug, and one with no
        // right value to substitute, so it is reproduced rather than fixed.
        // Zero is what the branch that does initialise them uses, and what puts
        // the panel on its own row. Shiny Days' `FUN_10019a40` zeroes all three
        // on entry and has no such branch.
        let (panel_shift, text_shift, panel_height) = match chars.min(cap * TIP_LINES) / cap {
            lines @ (0 | 1) => (
                if deep {
                    row_of_panel * (2 - lines) as f32 + tip.deep_step
                } else {
                    0.0
                },
                if deep { tip.deep_text[lines] } else { 0.0 },
                row_of_panel * (lines + 1) as f32
                    - tip.trim[lines]
                    - if deep { tip.deep_trim } else { 0.0 },
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
                    tip.origin.0 as u32,
                    (tip.origin.1 + n as f32 * tip.pitch) as u32,
                    Column::Comment.width(layout) as u32,
                    tip.height as u32,
                ),
                dst: (
                    record.dst.x as f32
                        + Column::Comment.dest_x(layout, english)
                        + comment_centre(layout, *width, english),
                    record.dst.y as f32 + layout.dest_y + n as f32 * tip.line_pitch + text_shift,
                    Column::Comment.dest_width(layout),
                    tip.line_height,
                ),
            })
            .collect();

        Tooltip { panel, lines }
    }

    /// Which row's second-band record places the panel, from `FUN_10012900`'s
    /// and `FUN_10019a40`'s `param_1 < 8 ? param_1 : param_1 - 2`.
    pub fn record_row(row: usize) -> usize {
        if row > LAST_ROW_OPENING_DOWN {
            row - 2
        } else {
            row
        }
    }
}

/// One page of rows, rasterised into a surface of its own with the rectangles
/// that put each column on the screen.
///
/// This is `FUN_10011ec0` and the sprite set-up in `FUN_100135c0` together —
/// `FUN_10018fd0` and `FUN_1001b240` on the other module: one
/// [`Layout::surface`]-sized buffer holding up to thirty columns of text, and a
/// quad per column cutting it out and placing it.
pub struct Bank {
    /// The glyph surface, RGB carrying the font's luminance plane and alpha its
    /// outline plane — the same two planes the shipped blitter writes.
    pub surface: days_ui::Image,
    /// One per drawn column, in row order. The destination is where the column
    /// sits **in this bank**; a scrolling list adds [`Slide::offset`] to it.
    pub quads: Vec<Quad>,
}

/// What the save/load list draws: one [`Bank`] per page it holds at once, and
/// the expanded comment over them.
///
/// School Days HQ holds one — `FUN_100135c0` builds a single surface and
/// `FUN_10011ec0` refills it when the page changes. Shiny Days holds
/// [`Strip::banks`] of them, because the page change is a slide and six pages
/// of rows are on screen while it runs; `FUN_1001b240` builds six
/// `FrameBuffer`s and `FUN_10018fd0` fills bank `b` from page
/// `window_top + b`.
pub struct Rows {
    /// In bank order, so bank 0 is the top of the strip.
    pub banks: Vec<Bank>,
    /// The expanded comment, when the pointer is in the second band and the
    /// row it names has one.
    pub tooltip: Option<Tooltip>,
    /// The surface [`Tooltip::lines`] are cut from, when the screen gives the
    /// expanded comment one of its own.
    ///
    /// School Days HQ does not: `FUN_10012900` rasterises the tooltip into the
    /// same buffer as the rows, clear of all three columns, so this is `None`
    /// and the lines come out of the shown page's [`Bank::surface`]. Shiny Days
    /// keeps a buffer of its own for it on both of its slot lists —
    /// `FUN_10019a40` here and `FUN_100271f0` in
    /// [`crate::ui::replay_pages`] — which is [`Tip::surface`].
    pub tip_surface: Option<days_ui::Image>,
}

/// One column's sprite: what it cuts out of the surface and where it lands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    /// The rectangle of [`Bank::surface`] this column occupies, in pixels.
    pub src: (u32, u32, u32, u32),
    /// Where it is drawn, in the 800x450 layout space the widget records use.
    pub dst: (f32, f32, f32, f32),
}

impl Rows {
    /// Rasterises every page the list holds at once.
    ///
    /// One bank for a screen that shows a page at a time, and
    /// [`Strip::banks`] of them for one that slides — bank `b` filled from page
    /// `window_top(page) + b`, which is `FUN_10018fd0`'s own loop. `slide` is
    /// what says which, so a list whose strip art would not load is rasterised
    /// as the single page it can draw rather than as a window nothing places.
    /// `page` is the page showing either way, and it is the page the tooltip
    /// reads.
    ///
    /// A row whose slot has no file is skipped entirely, which is what the
    /// shipped loop does: it only draws when the host's slot query answers 1.
    /// `records` is the screen's widget table, and a row whose record is missing
    /// is skipped rather than placed somewhere this engine chose.
    /// `hovered` is the row the pointer is expanding, if any — the selection's
    /// `widget - 0x16`, which is the only thing that opens the tooltip.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        layout: &Layout,
        font: &days_font::Font,
        slots: &Slots,
        page: usize,
        english: bool,
        records: &[days_ui::atlas::Widget],
        comments: bool,
        hovered: Option<usize>,
        slide: Option<&Slide>,
    ) -> Rows {
        let window = match slide {
            Some(slide) => slide.window(),
            None => page..page + 1,
        };
        let shown = page - window.start;
        let mut banks: Vec<Bank> = window
            .map(|page| Rows::bank(layout, font, slots, page, english, records, comments))
            .collect();

        // Shiny Days rasterises the expanded comment into a buffer of its own;
        // School Days HQ puts it in the corner of the rows', clear of all three
        // columns. `tip` is whichever the module says, and the tooltip's quads
        // cut whichever they were drawn into.
        let mut tip = layout
            .tip
            .surface
            .map(|(width, height)| days_ui::Image::empty(width, height));
        let tooltip = Rows::expand_into(
            tip.as_mut(),
            banks.get_mut(shown),
            layout,
            font,
            slots,
            page,
            english,
            records,
            comments,
            hovered,
        );
        Rows {
            banks,
            tooltip,
            tip_surface: tip,
        }
    }

    /// Re-lays the expanded comment without rasterising the rows again.
    ///
    /// Only the screen whose tooltip has a buffer of its own can do this, and
    /// that is the point of it: Shiny Days keeps six 1024x1024 banks, and
    /// pointing at a row must not cost six of those. `FUN_10017e70` calls
    /// `FUN_10019a40` from the draw and leaves `FUN_10018fd0` alone, so this is
    /// also what the shipped screen does. School Days HQ's `FUN_10012900`
    /// rasterises into the rows' own surface and its screen does go back
    /// through the whole row loop on every hover, so this answers `false` there
    /// and the caller rebuilds.
    #[allow(clippy::too_many_arguments)]
    pub fn rehover(
        &mut self,
        layout: &Layout,
        font: &days_font::Font,
        slots: &Slots,
        page: usize,
        english: bool,
        records: &[days_ui::atlas::Widget],
        comments: bool,
        hovered: Option<usize>,
    ) -> bool {
        let Some(tip) = &mut self.tip_surface else {
            return false;
        };
        tip.rgba.fill(0);
        self.tooltip = Rows::expand_into(
            Some(tip),
            None,
            layout,
            font,
            slots,
            page,
            english,
            records,
            comments,
            hovered,
        );
        true
    }

    /// Rasterises one page of rows into a surface of its own.
    fn bank(
        layout: &Layout,
        font: &days_font::Font,
        slots: &Slots,
        page: usize,
        english: bool,
        records: &[days_ui::atlas::Widget],
        comments: bool,
    ) -> Bank {
        let (width, height) = layout.surface;
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

                let drawn = draw_line(
                    &mut surface,
                    font,
                    &text,
                    surface_pen(layout, column, row),
                    &|c| text::menu_advance(c, english),
                );
                let centre = match column {
                    Column::Comment => comment_centre(layout, drawn, english),
                    _ => 0.0,
                };
                let (sx, sy, sw, sh) = source_rect(layout, column, row);
                quads.push(Quad {
                    src: (sx as u32, sy as u32, sw as u32, sh as u32),
                    dst: dest_rect(layout, column, record.dst, english, centre),
                });
            }
        }
        Bank { surface, quads }
    }

    /// Lays the expanded comment out into whichever surface holds it.
    ///
    /// `tip` is the screen's own tooltip buffer and `shown` the bank of the
    /// page being pointed at; exactly one of them is where the lines go.
    #[allow(clippy::too_many_arguments)]
    fn expand_into(
        tip: Option<&mut days_ui::Image>,
        shown: Option<&mut Bank>,
        layout: &Layout,
        font: &days_font::Font,
        slots: &Slots,
        page: usize,
        english: bool,
        records: &[days_ui::atlas::Widget],
        comments: bool,
        hovered: Option<usize>,
    ) -> Option<Tooltip> {
        let row = hovered.filter(|_| comments)?;
        let surface = match tip {
            Some(tip) => tip,
            None => &mut shown?.surface,
        };
        Rows::expand(layout, surface, font, slots, page, row, english, records)
    }

    /// Rasterises the expanded comment and lays it out, from `FUN_10012900`.
    ///
    /// Draws at the place the shipped code draws it: `[`Tip::origin`]` stepped
    /// by [`Tip::pitch`], which on School Days HQ is `(0x400, 0x202 + n * 0x40)`
    /// in the rows' own surface — clear of all three columns, since they end at
    /// x 986 above y 514 and at x 1024 below it — and on Shiny Days is
    /// `(0, 2 + n * 0x40)` in a buffer of its own.
    #[allow(clippy::too_many_arguments)]
    fn expand(
        layout: &Layout,
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
                    layout.tip.origin.0 as i32,
                    (layout.tip.origin.1 + n as f32 * layout.tip.pitch) as i32,
                );
                draw_line(surface, font, line, pen, &|c| {
                    text::menu_advance(c, english)
                })
            })
            .collect::<Vec<i32>>();

        Some(Tooltip::place(
            layout,
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

    /// Every one of these was recovered against `SysMenuSDHQ.dll`, so that is
    /// the layout they are checked at; [`Layout::SHINY_DAYS`] has its own tests.
    const HQ: &Layout = &Layout::SCHOOL_DAYS_HQ;

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
        let one = Tooltip::place(HQ, 0, panel_record(), &[0], 10, false);
        let two = Tooltip::place(HQ, 0, panel_record(), &[0, 0], 25, false);
        let three = Tooltip::place(HQ, 0, panel_record(), &[0, 0, 0], 50, false);
        assert!((one.panel.dst.3 - (97.0 / 3.0 - 2.0 + 1.0)).abs() < 0.01);
        assert!((two.panel.dst.3 - (97.0 * 2.0 / 3.0 + 1.0)).abs() < 0.01);
        assert!((three.panel.dst.3 - 98.0).abs() < 0.01);
    }

    /// The panel is always the record's full height in the chip sheet however
    /// short it is drawn, so a one-line panel is that art squashed.
    #[test]
    fn the_panel_cuts_the_whole_record_however_short_it_is_drawn() {
        let tip = Tooltip::place(HQ, 0, panel_record(), &[0], 10, false);
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
        let deep = Tooltip::place(HQ, 8, panel_record(), &[0], 10, false);
        let shallow = Tooltip::place(HQ, 0, panel_record(), &[0], 10, false);
        assert!((deep.panel.dst.1 - shallow.panel.dst.1 - 97.0 * 2.0 / 3.0).abs() < 0.01);
        assert!((deep.lines[0].dst.1 - shallow.lines[0].dst.1 - 64.0).abs() < 0.01);

        let deep = Tooltip::place(HQ, 9, panel_record(), &[0, 0], 25, false);
        let shallow = Tooltip::place(HQ, 1, panel_record(), &[0, 0], 25, false);
        assert!((deep.panel.dst.1 - shallow.panel.dst.1 - 97.0 / 3.0).abs() < 0.01);
        assert!((deep.lines[0].dst.1 - shallow.lines[0].dst.1 - 32.0).abs() < 0.01);
    }

    /// Three lines fill the record, so there is nothing to push down.
    #[test]
    fn a_full_panel_is_not_shifted_at_all() {
        let deep = Tooltip::place(HQ, 9, panel_record(), &[0, 0, 0], 60, false);
        let shallow = Tooltip::place(HQ, 1, panel_record(), &[0, 0, 0], 60, false);
        assert_eq!(deep.panel.dst.1, shallow.panel.dst.1);
        assert_eq!(deep.lines[0].dst.1, shallow.lines[0].dst.1);
    }

    /// The lines step down by half the surface pitch and cut the surface where
    /// `FUN_10012900` writes them.
    #[test]
    fn the_lines_cut_where_they_were_written() {
        let tip = Tooltip::place(HQ, 0, panel_record(), &[0, 0, 0], 60, false);
        for (n, line) in tip.lines.iter().enumerate() {
            assert_eq!(line.src, (1024, 514 + n as u32 * 64, 986, 64));
            assert!((line.dst.1 - (96.0 + HQ.dest_y + n as f32 * 32.0)).abs() < 0.01);
            assert_eq!(line.dst.2, Column::Comment.dest_width(HQ));
        }
    }

    /// The tooltip is rasterised clear of all three columns: they stop at x 986
    /// above y 514 and the tooltip starts at x 1024.
    #[test]
    fn the_tooltip_does_not_overwrite_the_rows() {
        for n in 0..TIP_LINES {
            let y = HQ.tip.origin.1 + n as f32 * HQ.tip.pitch;
            assert!(HQ.tip.origin.0 >= Column::Comment.width(HQ));
            assert!(y + HQ.tip.height <= HQ.surface.1 as f32);
            // Clear of the chapter column, which stops at y 482.
            assert!(y >= source_rect(HQ, Column::Chapter, PER_PAGE - 1).1 + SURFACE_ROW_HEIGHT);
        }
    }

    /// Pointing at a row with a comment opens it; pointing at nothing does not,
    /// and neither does a screen with `[TextInput]` off.
    #[test]
    fn the_tooltip_opens_only_on_a_hovered_row() {
        let slots = filled(3);
        let open = Rows::render(
            HQ,
            &font(),
            &slots,
            0,
            false,
            &records(),
            true,
            Some(3),
            None,
        );
        assert!(open.tooltip.is_some());
        assert!(
            Rows::render(HQ, &font(), &slots, 0, false, &records(), true, None, None)
                .tooltip
                .is_none()
        );
        // A row with no file has nothing to expand.
        assert!(Rows::render(
            HQ,
            &font(),
            &slots,
            0,
            false,
            &records(),
            true,
            Some(4),
            None
        )
        .tooltip
        .is_none());
    }

    /// `[TextInput]` off takes the comment column and the tooltip together,
    /// which is what the one host answer gates.
    #[test]
    fn text_input_off_takes_the_comment_and_its_tooltip() {
        let rows = Rows::render(
            HQ,
            &font(),
            &filled(3),
            0,
            false,
            &records(),
            false,
            Some(3),
            None,
        );
        assert!(rows.tooltip.is_none());
        assert_eq!(only(&rows).quads.len(), 2);
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
            assert_eq!(
                source_rect(HQ, Column::When, row),
                (0.0, y + 2.0, 548.0, 48.0)
            );
            assert_eq!(
                source_rect(HQ, Column::Chapter, row),
                (1024.0, y + 2.0, 548.0, 48.0)
            );
            assert_eq!(
                source_rect(HQ, Column::Comment, row),
                (0.0, y + 514.0, 986.0, 48.0)
            );
            assert_eq!(surface_pen(HQ, Column::When, row), (0, y as i32 + 2));
            assert_eq!(surface_pen(HQ, Column::Chapter, row), (1024, y as i32 + 2));
            assert_eq!(surface_pen(HQ, Column::Comment, row), (0, y as i32 + 514));
        }
    }

    /// Every column of every row has to fit, or the surface would be cut from
    /// somewhere it was never drawn.
    #[test]
    fn every_source_rect_lies_inside_the_surface() {
        for row in 0..PER_PAGE {
            for column in Column::ALL {
                let (x, y, w, h) = source_rect(HQ, column, row);
                assert!(
                    x + w <= HQ.surface.0 as f32,
                    "{column:?} row {row} runs wide"
                );
                assert!(
                    y + h <= HQ.surface.1 as f32,
                    "{column:?} row {row} runs long"
                );
            }
        }
    }

    /// The rows abut exactly: the slice is as tall as the step, so a glyph cell
    /// ends where the next row's begins.
    #[test]
    fn the_rows_abut_without_overlapping() {
        for row in 0..PER_PAGE - 1 {
            let (_, y, _, h) = source_rect(HQ, Column::When, row);
            let (_, next, _, _) = source_rect(HQ, Column::When, row + 1);
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

    /// Shiny Days' own layout, from `FUN_1001b240` and `FUN_10018fd0`.
    ///
    /// The chapter is the **left-hand** column here and the timestamp is beside
    /// it, squeezed 548 into 189 where School Days HQ squeezes it into 252 —
    /// which is why laying a Shiny Days row out with School Days HQ's set draws
    /// the timestamp from the left edge of the row, across the cell divider its
    /// own art has, and puts the chapter where the comment column starts.
    #[test]
    fn shiny_days_puts_the_chapter_first() {
        let sd = &Layout::SHINY_DAYS;
        let row = rect(30, 126);
        assert_eq!(
            dest_rect(sd, Column::Chapter, row, false, 0.0),
            (50.0, 128.0, 252.0, 24.0)
        );
        assert_eq!(
            dest_rect(sd, Column::When, row, false, 0.0),
            (125.0, 128.0, 189.0, 24.0)
        );
        // The chapter is left of the timestamp, which is the whole difference.
        assert!(Column::Chapter.dest_x(sd, false) < Column::When.dest_x(sd, false));
        assert!(Column::When.dest_x(HQ, false) < Column::Chapter.dest_x(HQ, false));
        // Its cut starts at 600, not at 1024, because the surface is half as
        // wide -- and runs past that surface's right edge, which is the shipped
        // rectangle and harmless: two characters never reach it.
        assert_eq!(
            source_rect(sd, Column::Chapter, 0),
            (600.0, 2.0, 548.0, 48.0)
        );
        assert!(600.0 + 548.0 > sd.surface.0 as f32);
        assert_eq!(sd.surface, (0x400, 0x400));
    }

    /// The English shifts are the same pair on both modules; only what the
    /// comment is centred back from differs.
    #[test]
    fn english_moves_the_same_two_columns_on_both_modules() {
        for layout in [HQ, &Layout::SHINY_DAYS] {
            assert_eq!(
                Column::When.dest_x(layout, true) - Column::When.dest_x(layout, false),
                5.0
            );
            assert_eq!(
                Column::Chapter.dest_x(layout, true) - Column::Chapter.dest_x(layout, false),
                15.0
            );
            assert_eq!(
                Column::Comment.dest_x(layout, true),
                Column::Comment.dest_x(layout, false)
            );
        }
        assert_eq!(comment_centre(HQ, 0, true), 235.5);
        assert_eq!(comment_centre(&Layout::SHINY_DAYS, 0, true), 226.5);
    }

    /// Shiny Days keeps the expanded comment in a buffer of its own, so its
    /// lines do not have to dodge the columns the way School Days HQ's do.
    #[test]
    fn shiny_days_expands_the_comment_into_its_own_buffer() {
        let sd = &Layout::SHINY_DAYS;
        assert_eq!(sd.tip.surface, Some((0x400, 0x100)));
        assert_eq!(sd.tip.origin, (0.0, 2.0));
        assert_eq!(HQ.tip.surface, None);
        let tip = Tooltip::place(sd, 0, panel_record(), &[0, 0, 0], 60, false);
        for (n, line) in tip.lines.iter().enumerate() {
            assert_eq!(line.src, (0, 2 + n as u32 * 64, 986, 64));
        }
    }

    /// A short panel gives back a third of its record on both modules, and on
    /// Shiny Days three pixels more again — and another three when it opens
    /// upwards, which `FUN_10019a40` takes off and `FUN_10012900` does not.
    #[test]
    fn a_short_panel_is_trimmed_the_way_each_module_trims_it() {
        let sd = &Layout::SHINY_DAYS;
        let third = 97.0 / 3.0;
        let height = |layout, row| {
            Tooltip::place(layout, row, panel_record(), &[0], 10, false)
                .panel
                .dst
                .3
                - 1.0
        };
        assert!((height(HQ, 0) - (third - 2.0)).abs() < 0.01);
        assert!((height(HQ, 8) - (third - 2.0)).abs() < 0.01);
        assert!((height(sd, 0) - (third - 3.0)).abs() < 0.01);
        assert!((height(sd, 8) - (third - 6.0)).abs() < 0.01);
    }

    /// [`Layout::of`] reads the module's export table, not the table it is
    /// about to lay out. A module that publishes neither of Shiny Days' two
    /// extra exports — including a byte slice that is no PE at all — is laid
    /// out as School Days HQ, which is the one-sided answer the doc records.
    #[test]
    fn a_module_that_is_not_shiny_days_is_laid_out_as_school_days_hq() {
        assert_eq!(Layout::of(&[]).module, Layout::SCHOOL_DAYS_HQ.module);
    }

    #[test]
    fn the_columns_sit_where_the_dll_puts_them() {
        let r = rect(20, 100);
        assert_eq!(
            dest_rect(HQ, Column::When, r, false, 0.0),
            (21.0, 104.5, 252.0, 24.0)
        );
        assert_eq!(
            dest_rect(HQ, Column::Chapter, r, false, 0.0),
            (282.5, 104.5, 252.0, 24.0)
        );
        assert_eq!(
            dest_rect(HQ, Column::Comment, r, false, 0.0),
            (22.0, 104.5, 494.0, 24.0)
        );
    }

    /// Host `+0x5c` moves the timestamp five right and the chapter fifteen, and
    /// leaves the comment alone -- that one is centred instead.
    #[test]
    fn english_shifts_the_stored_line_but_not_the_comment() {
        let r = rect(20, 100);
        assert_eq!(dest_rect(HQ, Column::When, r, true, 0.0).0, 26.0);
        assert_eq!(dest_rect(HQ, Column::Chapter, r, true, 0.0).0, 297.5);
        assert_eq!(dest_rect(HQ, Column::Comment, r, true, 0.0).0, 22.0);
    }

    /// `235.5 - width / 4`, clamped at zero, and nothing at all in Japanese.
    #[test]
    fn an_english_comment_is_centred_in_its_column() {
        assert_eq!(comment_centre(HQ, 0, true), 235.5);
        assert_eq!(comment_centre(HQ, 942, true), 0.0);
        // Past the point where the text fills the column, it stops moving
        // rather than going negative.
        assert_eq!(comment_centre(HQ, 4000, true), 0.0);
        assert_eq!(comment_centre(HQ, 0, false), 0.0);
        assert_eq!(comment_centre(HQ, 400, false), 0.0);
    }

    /// A comment of exactly half the column's source width centres at a quarter
    /// of the destination, which is the identity the constants encode.
    #[test]
    fn the_centring_is_half_the_column_less_half_the_drawn_width() {
        let drawn = 400;
        let column = Column::Comment.dest_width(HQ);
        let on_screen = drawn as f32 * column / Column::Comment.width(HQ);
        // 11.5 short of true centre, which is the shipped constant, not 247.
        assert!(((column - on_screen) / 2.0 - comment_centre(HQ, drawn, true) - 11.5).abs() < 0.5);
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
        assert!((Column::Comment.width(HQ) / Column::Comment.dest_width(HQ) - 1.996).abs() < 0.001);
        assert!((Column::When.width(HQ) / Column::When.dest_width(HQ) - 2.175).abs() < 0.001);
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
        assert_eq!(only(&rows).quads.len(), 3);
        assert!(only(&saveload_rows(&Slots::default(), 0)).quads.is_empty());
    }

    /// The page a row stands for moves it a whole page of slots, not a row.
    #[test]
    fn a_page_shows_its_own_ten_slots() {
        assert!(only(&saveload_rows(&filled(3), 1)).quads.is_empty());
        assert_eq!(only(&saveload_rows(&filled(13), 1)).quads.len(), 3);
    }

    fn saveload_rows(slots: &Slots, page: usize) -> Rows {
        Rows::render(
            HQ,
            &font(),
            slots,
            page,
            false,
            &records(),
            true,
            None,
            None,
        )
    }

    /// The one bank of a list that does not slide.
    fn only(rows: &Rows) -> &Bank {
        assert_eq!(rows.banks.len(), 1, "School Days HQ's list has no strip");
        &rows.banks[0]
    }

    /// The surface is the size the `FrameBuffer` is created at, and every quad
    /// cuts it rather than reaching past it.
    #[test]
    fn the_rendered_quads_cut_the_surface_they_were_drawn_into() {
        let rows = saveload_rows(&filled(7), 0);
        let bank = only(&rows);
        assert_eq!((bank.surface.width, bank.surface.height), HQ.surface);
        for quad in &bank.quads {
            let (x, y, w, h) = quad.src;
            assert!(x + w <= bank.surface.width);
            assert!(y + h <= bank.surface.height);
        }
    }

    /// A row whose record the table does not carry is skipped rather than
    /// placed somewhere this engine chose.
    #[test]
    fn a_row_with_no_record_is_left_undrawn() {
        let short: Vec<days_ui::atlas::Widget> = records().into_iter().take(10).collect();
        let rows = Rows::render(HQ, &font(), &filled(0), 0, false, &short, true, None, None);
        // The comment's record is in the second band, which this table stops
        // short of, so only the stored line's two columns are placed.
        assert_eq!(only(&rows).quads.len(), 2);
    }

    /// The strip's constants, against Shiny Days' own art: `SaveLoadList.png`
    /// is 800x300, so the banks are 301 apart and a panel is 801x301.
    const ART: (u32, u32) = (800, 300);
    const PITCH: f32 = 301.0;

    fn strip() -> Strip {
        Layout::SHINY_DAYS.strip.expect("Shiny Days' list slides")
    }

    /// The window bank 0 holds, against `FUN_10018fd0`'s own ladder: page 0 and
    /// 1 from page 0, pages 2 to 6 from `page - 2`, and pages 7, 8 and 9 all
    /// from page 4.
    #[test]
    fn the_window_is_the_modules_ladder() {
        let tops: Vec<usize> = (0..PAGES).map(|page| strip().window_top(page)).collect();
        assert_eq!(tops, [0, 0, 0, 1, 2, 3, 4, 4, 4, 4]);
    }

    /// `FUN_1001ed40`'s seven-arm switch is `pitch * (next - window_top(page))`
    /// for every transition a page button can ask for — the two methods that
    /// have to agree before either is believed. See [`Strip::target`].
    #[test]
    fn the_step_switch_is_the_window() {
        let strip = strip();
        for page in 0..PAGES {
            for (next, forward) in [(page.wrapping_sub(1), false), (page + 1, true)] {
                if next >= PAGES {
                    continue;
                }
                let want = PITCH * (next - strip.window_top(page)) as f32;
                assert_eq!(
                    strip.target(page, next, forward, PITCH),
                    want,
                    "page {page} to {next}"
                );
            }
        }
    }

    /// A step from a page button lands exactly on `(k ± 1) * pitch`, and the
    /// settle then reads the page back off the scroll. The `<=` in
    /// `FUN_1001f8c0`'s second compare is what makes the forward half of this
    /// work at all; with a `<` every forward step would leave the page alone.
    #[test]
    fn a_settled_step_reads_its_page_back_off_the_scroll() {
        let strip = strip();
        for page in 0..PAGES {
            for (next, want) in [
                (page.wrapping_sub(1), Settled::Previous),
                (page + 1, Settled::Next),
            ] {
                if next >= PAGES {
                    continue;
                }
                let scroll = strip.target(page, next, next > page, PITCH);
                assert_eq!(strip.settled(page, scroll, PITCH), want, "{page} to {next}");
            }
        }
    }

    /// Every one-page step, run to its end, reaches the page it was aimed at
    /// and leaves that page's rows exactly where their records put them.
    ///
    /// The re-seat is the part this pins down. A step's target is measured
    /// against the window the page it *left* had, so when the window itself
    /// shifts — page 2 to page 3, where bank 0 goes from page 0 to page 1 —
    /// the strip overshoots by a pitch and `FUN_1001f6f0` takes it back while
    /// `FUN_10018fd0` refills the banks one page along. The two cancel, and
    /// that is why the settle re-seats rather than just stopping.
    #[test]
    fn every_step_lands_on_the_page_it_was_aimed_at() {
        for page in 0..PAGES {
            for next in [page.wrapping_sub(1), page + 1] {
                if next >= PAGES {
                    continue;
                }
                let mut slide = Slide::new(strip(), ART, page);
                slide.go(next);
                let mut ticks = 0;
                while slide.tick().is_none() {
                    ticks += 1;
                    assert!(ticks < 100, "page {page} to {next} never settled");
                }
                assert_eq!(slide.page(), next, "page {page} to {next}");
                assert_eq!(slide.offset(next - slide.window_top()), 0.0);
            }
        }
    }

    /// A one-page step is twenty frames, linear, and the twentieth snaps onto
    /// the target rather than keeping the twentieth accumulation.
    #[test]
    fn a_step_takes_twenty_frames() {
        let mut slide = Slide::new(strip(), ART, 0);
        slide.go(1);
        for frame in 1..20 {
            assert_eq!(slide.tick(), None, "frame {frame}");
            assert!(slide.moving());
            assert_eq!(slide.page(), 0);
        }
        assert_eq!(slide.tick(), Some(1));
        assert!(!slide.moving());
        assert_eq!(slide.offset(0), -PITCH);
    }

    /// A jump across pages is a run of one-page steps, so page 0 to page 9 is
    /// nine of them and the page climbs one at a time.
    #[test]
    fn a_jump_is_one_step_per_page() {
        let mut slide = Slide::new(strip(), ART, 0);
        slide.go(9);
        let mut settled = Vec::new();
        for _ in 0..9 * 20 {
            if let Some(page) = slide.tick() {
                settled.push(page);
            }
        }
        assert_eq!(settled, [1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert!(!slide.moving());
        assert_eq!(slide.page(), 9);
    }

    /// The page showing rests where its own bank does, so the strip's offset
    /// puts that bank's rows exactly where their records say.
    #[test]
    fn the_shown_page_rests_on_its_own_bank() {
        let strip = strip();
        for page in 0..PAGES {
            let slide = Slide::new(strip, ART, page);
            assert_eq!(slide.offset(page - strip.window_top(page)), 0.0);
        }
    }

    /// The panels tile with no seam: each is `pitch` tall and `pitch` below the
    /// one before it, the art stretched by the one pixel `FUN_1001a710` adds.
    #[test]
    fn the_panels_abut() {
        let slide = Slide::new(strip(), ART, 0);
        let (_, first, width, height) = slide.panel(0);
        assert_eq!((width, height), (801.0, PITCH));
        assert_eq!(first, 122.0 - 0.5);
        for bank in 1..strip().banks {
            assert_eq!(slide.panel(bank).1, first + bank as f32 * height);
        }
    }

    /// A page click part-way through a slide is answered: `FUN_1001ca60` gates
    /// the widgets on the confirm popup and on nothing else, so the new step
    /// starts from wherever the strip has got to.
    #[test]
    fn a_click_mid_slide_redirects_the_strip() {
        let mut slide = Slide::new(strip(), ART, 0);
        slide.go(1);
        for _ in 0..10 {
            slide.tick();
        }
        let part_way = slide.offset(0);
        assert!(part_way < 0.0 && part_way > -PITCH);
        slide.go(3);
        while slide.tick() != Some(3) {}
        assert_eq!(slide.page(), 3);
        assert_eq!(slide.offset(slide.page() - slide.window_top()), 0.0);
    }

    /// Both banks of a list that slides are filled from the window, not from
    /// the page: six pages of rows are on screen while a step runs.
    #[test]
    fn a_sliding_list_rasterises_its_whole_window() {
        let rows = Rows::render(
            &Layout::SHINY_DAYS,
            &font(),
            &filled(35),
            3,
            false,
            &records(),
            true,
            None,
            Some(&Slide::new(strip(), ART, 3)),
        );
        assert_eq!(rows.banks.len(), 6);
        // Page 3's window starts at page 1, so slot 35 is row 5 of bank 2.
        let drawn: Vec<usize> = rows
            .banks
            .iter()
            .map(|bank| bank.quads.len())
            .collect::<Vec<_>>();
        assert_eq!(drawn, [0, 0, 3, 0, 0, 0]);
    }

    #[test]
    fn a_line_too_short_to_hold_a_chapter_is_left_alone() {
        assert_eq!(split_line("", false), (String::new(), String::new()));
        assert_eq!(split_line("x", false), ("x".to_owned(), String::new()));
    }
}
