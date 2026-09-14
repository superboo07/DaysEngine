//! The Replay screen's two views, on the module that ships one hit map for the
//! whole screen.
//!
//! # The screen splits in two, the way the Option screen does
//!
//! School Days HQ gives each Replay view its own `.CMAP` carrying all of that
//! view's widgets, and [`crate::ui::replay`] reads them straight out of it. The
//! other module on this engine ships `System/Replay/ReplayBase.cmap` with three
//! regions — the two view headers and CLOSE — and nothing for the grid or the
//! list underneath. Those are rectangles in a table instead.
//!
//! `MENU::SceneView`'s vtable slot `+0x4c` is `FUN_1002cf00`, the same shape as
//! `MENU::ConfigMenu`'s `FUN_1000bb10`: a linear scan of the view's record
//! table returning `index + 3`, reached when the hit map misses. So the map
//! answers the frame and the table answers the view, and [`Pages`] is the
//! second half. See [`crate::ui::option_pages`] for the same split written out
//! at length.
//!
//! # The widgets, from `FUN_1002cf00`, `FUN_1002a330` and `FUN_1002a390`
//!
//! ```text
//! HScene    17 records   widgets 3..=0x13
//!   3,  4     the two arrows either side of the grid
//!   5 ..  7   the three page buttons
//!   8 .. 0x13 the twelve thumbnails
//!
//! PlayData  30 records   widgets 3..=0x20
//!   3 .. 0xc  the ten rows, left band
//!   0xd..0x16 the ten rows, right band — the comment column
//!   0x17..0x20 the ten page buttons
//! ```
//!
//! The two bands and the page buttons are the same three runs School Days HQ's
//! play-data list has ([`crate::ui::playdata`]) **in a different order**: there
//! the page buttons are widgets 0xd to 0x16 and the comment column 0x17 to
//! 0x20. Both ends of this module agree on the order written above — the table
//! is laid out that way and `FUN_10024e30` raises the expanded comment for
//! `selection - 0xd`, where School Days HQ's `FUN_1001a060` uses
//! `selection - 0x17`.
//!
//! # Finding the tables
//!
//! Neither is anchored: no hit map covers them and neither sits at a fixed
//! distance from one that does. They are found by their shape instead — see
//! [`days_ui::atlas::table_by_shape`], and note that that search is ours rather
//! than the game's. The shapes are [`hscene_shape`] and [`playdata_shape`].
//!
//! The play-data **list run** is anchored, though: `FUN_100288a0` reaches it at
//! `DAT_10057a68`, which is exactly [`LIST_AFTER_FRAME`] records past the
//! frame's table — the frame's three widgets and the two header highlights, one
//! per view. See [`Pages::list`].
//!
//! # How a view is drawn
//!
//! A view is a layer of its own between the frame's art and the frame's
//! sprites, and `FUN_10024480` is the order: the view's background and the
//! panels stacked over it, then the contents of the view showing
//! (`FUN_10024c20` for the grid, `FUN_10024e30` for the list), then the frame's
//! header highlight and its hovered widget. [`crate::ui::screen::Page`] is that
//! layer.
//!
//! The background is one full-screen image per view ([`base_art`]) and the
//! panels are a strip of pages over it ([`panel_art`]) — three side by side for
//! the grid, scrolled horizontally by the page buttons, and six one above
//! another for the list, scrolled vertically. At rest the page showing sits
//! exactly over the background, so a still frame of one page is the background
//! with that page's panel on top.
//!
//! What each view puts down, from the two content draws and the two refits that
//! bind their sprites to records — `FUN_10028260` for the grid and
//! `FUN_100288a0` for the list:
//!
//! ```text
//! HScene    hovered arrow or page button   its own record, from the chip sheet
//!           the page button of the page    record 0x29 + page
//!           the hovered thumbnail          record 5 + page * 12 + slot
//!
//! PlayData  the hovered row                list record `row`, 740 wide
//!           the hovered page button        list record 0x14 + button
//!           the page button of the page    list record 0x1e + page
//! ```
//!
//! A thumbnail's rectangle does not move between pages — records 5 to 0x10,
//! 0x11 to 0x1c and 0x1d to 0x28 hold the same twelve rectangles at three
//! `src_y` — so the hit table covers page 0's and [`Pages::thumbnail`] picks the
//! page's art out of the run behind it.
//!
//! The resting grid is not in the panel art as it ships. `FUN_10027900` builds
//! it: for each of the 36 h-scenes it asks the host whether that scene's flag
//! is set, and only then copies the scene's rectangle out of
//! [`RESTING_THUMBNAILS`] into the panel its page belongs to, at the same
//! record [`Pages::thumbnail`] returns. A scene the player has not seen is
//! never copied, so a locked slot shows the bare panel. The thumbnail under the
//! pointer is a sprite over that, cut from [`HOVER_THUMBNAILS`] — the second
//! sheet of the same 36 rectangles, which `FUN_10026350` loads.
//!
//! Hovering **either** band of the list lights the same row: `FUN_10024e30`
//! draws list record `row` when the selection is `row + 3` or `row + 0xd`, and
//! that record is the row entire rather than the band. School Days HQ's list
//! does the same thing.
//!
//! # The list's text
//!
//! `FUN_10027c10` fills a panel and then places what it filled: it calls
//! `FUN_100267c0` to rasterise the panel's ten rows, and then binds the three
//! sprites per row that cut them back out. Each of the six panels keeps a
//! surface of its own — `FUN_100296a0` builds six `FrameBuffer`s at
//! `(0x400, 0x400, 0x208888)` and `FUN_100267c0` clears its own as 0x400 rows
//! of 0x1000 bytes, which is the same 1024 pixels of 32 bits.
//!
//! `FUN_100267c0` asks the host `+0xa8` for every entry of the panel —
//! `FUN_0041af00`, which formats `[SaveFileName]` and `[SaveConfig]` with the
//! entry number and answers with the timestamp, the chapter and the player's
//! comment, the same three strings School Days HQ's `FUN_0042a980` gives
//! through `+0x9c` — and lays them down through host `+0x64`, twenty
//! characters apiece, at pen `(0, row * 0x30)` for the timestamp,
//! `(600, row * 0x30)` for the chapter and `(0, row * 0x30 + 0x202)` for the
//! comment. Its advance is a flat `0x18` below U+0080 and `0x2d` at or above
//! it, with none of the per-character kerning
//! [`crate::playback::text::menu_advance`] adds under English — see
//! [`advance`].
//!
//! What `FUN_10027c10` then cuts and where it puts it:
//!
//! ```text
//!            record        cut from the panel surface   drawn at
//! timestamp  list[row]     (0,   row*48 + 2,  548, 48)  (+95 +5en, +2, 189, 24)
//! chapter    list[row]     (600, row*48 + 2,  548, 48)  (+20 +15en, +2, 252, 24)
//! comment    list[row+10]  (0,   row*48 + 514, 986, 48) (+2 +centre, +2, 494, 24)
//! ```
//!
//! — the offsets being against that record's own origin. So the two halves of
//! the stored line share the row bar's record and the comment takes the
//! 454x87 panel record beside it, which is the same split
//! [`crate::ui::saveload`] and [`crate::ui::playdata`] make between their two
//! bands.
//!
//! The timestamp and the chapter are rasterised two pixels above where they are
//! cut, exactly as School Days HQ's play-data list does; the comment's pen and
//! cut agree, because its pen carries the `0x202` its cut does.
//!
//! The chapter's cut runs from 600 to 1148 across a surface 1024 wide. That is
//! the shipped rectangle and it is kept: a chapter is `第N話` or two digits, so
//! the glyphs are long finished by the edge.
//!
//! Two things come off the host. The comment column is drawn only when `+0xf4`
//! — `FILMENGINE.INI [TextInput]`, reached the way [`crate::ui::saveload`] sets
//! out — answers true. `+0x68` is the English question: `FUN_10018c40` picks
//! `L"%4d年%2d月%2d日(%s)%02d:%02d"` and `L"第%d話"` when it answers zero and
//! `L"%2d/%2d/%4d(%s)%02d:%02d"` and `L"%02d"` when it does not, which is what
//! a save line in an English install reads like. It moves the timestamp 5 right
//! and the chapter 15, the same two shifts the save/load screen makes.
//!
//! # The comment is not centred, and the reason is worth keeping
//!
//! `FUN_100267c0` measures each comment's advance and works out a centre for it
//! — `226.5 - width / 4` clamped at zero, School Days HQ's rule 9 short of its
//! 235.5 — and stores it at `+0x518 + panel * 0x28 + row * 4`. But it **zeroes
//! all six panels' ten centres on entry** and refills only the panel it was
//! given, and `FUN_100288a0` is the one and only caller of `FUN_10027c10`,
//! which is the one and only caller of `FUN_100267c0`: six panels in order,
//! every pass. So when the last of them returns, the array holds centres for
//! panel 5 and zeroes for the other five.
//!
//! That would not matter if the sprite kept the position it was given — but
//! `FUN_1002c1f0`, the screen's own update, re-places all three sprites of all
//! six panels out of the same array, so the comment's x is re-read from it
//! whenever the list is showing. The five panels whose centres were wiped draw
//! their comments at the column's left edge.
//!
//! [`comment_centred`] is which page keeps its centring, and it is the tenth:
//! the page showing sits in panel `page - window_top(page)`, and that is panel
//! 5 only at page 9.
//!
//! `FUN_100267c0` places the comment sprite once itself on the way past, out
//! of the **hit** table at `0x10058248` rather than the list run. Nothing is
//! ever drawn from it: `FUN_10027c10` places the same sprite again the moment
//! it returns, and the update places it again every frame after that.
//!
//! # The expanded comment
//!
//! `FUN_10024e30` raises it while the selection is past `0xc`, handing
//! `FUN_100271f0` `selection - 0xd`. That is the comment band; the page buttons
//! above it index a row the panel never filled, so they find nothing and draw
//! nothing.
//!
//! It has a **seventh** buffer of its own, `+0x1a4`, built `(0x400, 0x100,
//! 0x208888)` by `FUN_100296a0` and cleared 0x100 rows of 0x1000 bytes. The
//! row's whole comment is laid into it cut at `0x3c` characters: the pen starts
//! at `(0, 0)`, steps `0x18`/`0x2d` a glyph, and every twenty characters drops
//! `0x3a` and goes back to `0x400`.
//!
//! `0x400` is the right edge of a buffer 1024 pixels wide, and the blitter
//! takes a pitch rather than a width, so a glyph put there lands one whole
//! scanline on: `y * 0x1000 + 0x400 * 4` is `(y + 1) * 0x1000`. The second and
//! third lines therefore come out at x 0, one pixel below where the `0x3a`
//! steps put them, and all three are inside the 986-wide rectangle the sprite
//! cuts. See [`tip_pen`].
//!
//! The panel behind it is the list run's `row + 10` record — `row - 2` for the
//! last two rows, so three lines cannot run off the bottom — sized by the
//! character count over twenty: a third of the record less 3 for one line, two
//! thirds less 3 for two, the whole record for three. A row opening upwards
//! takes another 3 off and pushes the panel down by what it did not use. None
//! of those shifts is read before it is written, where the save/load screen's
//! pair are.

use crate::ui::replay::View;
use crate::ui::saveload::{self, Column, Line, Quad, Rows, Slots, Tooltip};
use days_ui::atlas::{self, Widget};

/// The first widget of a view. Widgets 0 to 2 are the frame's, and
/// `FUN_1002cf00` returns `index + 3`.
pub const FIRST: usize = 3;

/// Records between the frame table's first record and the play-data list run:
/// the three frame widgets and the two header highlights, one per view.
pub const LIST_AFTER_FRAME: usize = 5;

/// The grid's two arrows, from `FUN_1002a330`: widget 3 back, widget 4 forward.
pub const FIRST_ARROW: usize = 3;
pub const ARROWS: usize = 2;

/// The grid's page buttons.
pub const FIRST_HSCENE_PAGE: usize = 5;

/// How many pages the grid has, from the five sprites `FUN_10024480` lights for
/// widgets 3 to 7 less the two arrows.
pub const HSCENE_PAGES: usize = 3;

/// The grid's thumbnails, from `FUN_1002a330` and `FUN_10024c20`.
pub const FIRST_THUMBNAIL: usize = 8;
pub const THUMBNAILS: usize = 12;

/// How many h-scenes the grid can reach, which is the bound `FUN_10028260`
/// tests `page * 12 + slot` against before it looks a flag up.
pub const SCENES: usize = THUMBNAILS * HSCENE_PAGES;

/// The list's left band, its right band — the comment column — and its page
/// buttons, from `FUN_1002a390` and `FUN_10024e30`.
pub const FIRST_ROW: usize = 3;
pub const FIRST_COMMENT: usize = 0xd;
pub const FIRST_PLAYDATA_PAGE: usize = 0x17;
pub const PER_PAGE: usize = 10;

/// Where each view's records begin, from `FUN_1002cf00`.
pub fn records(view: View) -> usize {
    match view {
        View::HScene => 0x11,
        View::PlayData => 0x1e,
    }
}

/// The view's full-screen background.
///
/// `FUN_100293e0` hands `FUN_10029020` one of two literals on a view change:
/// `System/Replay/Replay_HScene.png` for the grid and
/// `System/Replay/Replay_PlayData.png` for the list. Neither is the screen's
/// own base art — `FUN_10029550` takes that from the stem like every other
/// screen, as `System/Replay/ReplayBase.png` — so both of these are drawn
/// inside the frame rather than under it. Holding both is what
/// [`crate::ui::paths::Paths::replay_has_pages`] recognises a one-map module
/// by.
pub fn base_art(view: View) -> &'static str {
    match view {
        View::HScene => "System/Replay/Replay_HScene.png",
        View::PlayData => "System/Replay/Replay_PlayData.png",
    }
}

/// How many pages a view's buttons reach.
///
/// The grid's three are `FUN_1002a4a0`'s own clamp — the back arrow is refused
/// at 0 and the forward arrow at 2. The list's ten are the ten page buttons
/// `FUN_1002a390` and `FUN_1002a780` answer for, widgets 0x17 to 0x20.
pub fn pages(view: View) -> usize {
    match view {
        View::HScene => HSCENE_PAGES,
        View::PlayData => PLAYDATA_PAGES,
    }
}

/// The list's ten page buttons. See [`pages`].
pub const PLAYDATA_PAGES: usize = 10;

/// How many panels of list the strip holds, from the 6 `FUN_100293e0` stores to
/// `+0x628` on the list's arm — where the grid's arm stores its three pages.
///
/// Six is **fewer than the list's ten pages**, because the strip is a window
/// over them rather than the whole of them. See [`window_top`].
pub const PLAYDATA_PANELS: usize = 6;

/// Which page the list's six-panel strip starts at, from `FUN_100267c0`.
///
/// That function takes the panel to rasterise and fills it from entry
/// `(window_top(page) + panel) * 10`, so panel `p` carries page
/// `window_top(page) + p` and the six slide as the page moves. The shipped
/// chain is written out one page at a time — 0 and 1 give themselves and
/// themselves less one, 7, 8 and 9 give themselves less three, four and five,
/// everything between gives itself less two — which is `page - 2` held inside
/// `0 ..= PLAYDATA_PAGES - PLAYDATA_PANELS`. So the page showing sits third of
/// the six wherever there is room either side, and at the end the strip stops
/// rather than running past the last page.
pub fn window_top(page: usize) -> usize {
    page.saturating_sub(2).min(PLAYDATA_PAGES - PLAYDATA_PANELS)
}

/// The panel art for one page, or `None` for a page the view has not got.
///
/// The grid's three pages are three sheets: `FUN_10025770` formats
/// `System/Replay/HScene/ReplayThum%d.png` with `page + 1` — the `ADD EAX,0x1`
/// ahead of the `_vswprintf_p_l` call at `0x10025849` — so they are
/// `ReplayThum1` to `ReplayThum3`, and the three are laid side by side in one
/// strip the page buttons scroll horizontally.
///
/// The list's six pages are six copies of **one** sheet: `FUN_10025e70` loads
/// `System/Replay/PlayData/ReplayList.png` once and gives it six sprites, each
/// one panel height further down a strip scrolled vertically. So every page of
/// the list rests on the same art and only its text differs.
pub fn panel_art(view: View, page: usize) -> Option<String> {
    if page >= pages(view) {
        return None;
    }
    Some(match view {
        View::HScene => format!("System/Replay/HScene/ReplayThum{}.png", page + 1),
        View::PlayData => "System/Replay/PlayData/ReplayList.png".to_string(),
    })
}

/// Where the page showing sits over the view's background, in layout space.
///
/// Each view stacks its panels in one strip and scrolls the strip, so at rest
/// the page showing lands at the strip's own origin. `FUN_10025770` puts the
/// grid's panel `i` at `x = pitch * i` with no `y` of its own, so page 0 is the
/// origin; `FUN_10025e70` puts the list's panel `i` at
/// `x = _DAT_1004978c, y = i * panel_height + _DAT_1004bd60`, and those two are
/// **doubles** — `-0.5` and `122.0`, not the zeroes Ghidra's f32 view of
/// `.rdata` shows. The `-0.5` is this engine's half-pixel convention for 0, so
/// the list's first page sits at `(0, 122)`, four pixels above the first row
/// bar at y = 126.
pub fn panel_origin(view: View) -> (i64, i64) {
    match view {
        View::HScene => (0, 0),
        View::PlayData => (0, 122),
    }
}

/// The sheet a view's own sprites are cut from, which is neither the frame's
/// chip sheet nor the page's art.
///
/// `FUN_10028260` loads the grid's and `FUN_100288a0` the list's.
pub fn chip_art(view: View) -> &'static str {
    match view {
        View::HScene => "System/Replay/HScene/ReplayThum_Chip.png",
        View::PlayData => "System/Replay/PlayData/ReplayList_Chip.png",
    }
}

/// The sheet the grid's resting thumbnails are blitted out of, and the sheet
/// the one under the pointer is drawn from.
///
/// Two sheets of the same 36 rectangles. `FUN_10027900` formats
/// `System/Replay/HScene/Replay_ThmBase%02d.png` and `FUN_10026350`
/// `System/Replay/HScene/Replay_Thm%02d.png`, and **both pass a literal 1** —
/// neither takes the page — so each title ships exactly one of each however
/// many pages it has.
pub const RESTING_THUMBNAILS: &str = "System/Replay/HScene/Replay_ThmBase01.png";
pub const HOVER_THUMBNAILS: &str = "System/Replay/HScene/Replay_Thm01.png";

/// Where the grid's thumbnail art starts, from `FUN_10028260`: record
/// `5 + page * 12 + slot`.
const THUMBNAIL_RECORD: usize = 5;

/// Where the grid's lit page button is, from `FUN_10028260`: record
/// `0x29 + page`.
const HSCENE_PAGE_MARK_RECORD: usize = 0x29;

/// Where the list's page buttons are in the list run, from `FUN_100288a0`:
/// the hovered button at record `0x14 + button`, the lit one at `0x1e + page`.
const PLAYDATA_PAGE_RECORD: usize = 0x14;
const PLAYDATA_PAGE_MARK_RECORD: usize = 0x1e;

/// Whether a run of records can be the grid's.
///
/// Two arrows flank the grid — one size, one baseline, taller than they are
/// wide — then three page buttons share a baseline left to right, then twelve
/// cells of one size make three rows of four: each row on its own baseline at
/// the four `x` of the row above, the rows going down the screen. Neither the
/// columns nor the rows are evenly pitched, so nothing here asks them to be.
fn hscene_shape(run: &[Widget]) -> bool {
    let (arrows, rest) = run.split_at(ARROWS);
    let (pages, grid) = rest.split_at(HSCENE_PAGES);
    if !one_size_on_one_line(arrows) || arrows[1].dst.x <= arrows[0].dst.x {
        return false;
    }
    if arrows[0].dst.height <= arrows[0].dst.width {
        return false;
    }
    if !(on_one_line(pages) && rightwards(pages)) {
        return false;
    }
    let cell = grid[0].dst;
    if !grid
        .iter()
        .all(|w| (w.dst.width, w.dst.height) == (cell.width, cell.height))
    {
        return false;
    }
    let rows: Vec<&[Widget]> = grid.chunks(THUMBNAILS / HSCENE_PAGES).collect();
    rows.iter().all(|row| {
        on_one_line(row)
            && rightwards(row)
            && row.iter().zip(rows[0]).all(|(w, top)| w.dst.x == top.dst.x)
    }) && rows.windows(2).all(|w| w[1][0].dst.y > w[0][0].dst.y)
}

/// Whether a run of records can be the list's.
///
/// Two bands of ten rows, each band one `x` and one size at an even pitch and
/// the second band to the right of the first, sharing the first's ten
/// baselines; then ten page buttons left to right on one baseline. The buttons
/// are neither one width nor one pitch — the first and last are a pixel wider
/// than the eight between them — so this asks only that they advance.
fn playdata_shape(run: &[Widget]) -> bool {
    let (rows, pages) = run.split_at(PER_PAGE * 2);
    let (left, right) = rows.split_at(PER_PAGE);
    if !(is_column(left) && is_column(right)) {
        return false;
    }
    if right[0].dst.x <= left[0].dst.x {
        return false;
    }
    if !left.iter().zip(right).all(|(l, r)| l.dst.y == r.dst.y) {
        return false;
    }
    on_one_line(pages) && rightwards(pages)
}

/// Whether these records are one size at an even vertical pitch down one `x`.
fn is_column(run: &[Widget]) -> bool {
    let first = run[0].dst;
    if !run
        .iter()
        .all(|w| (w.dst.x, w.dst.width, w.dst.height) == (first.x, first.width, first.height))
    {
        return false;
    }
    let pitch = run[1].dst.y.saturating_sub(first.y);
    pitch > 0 && run.windows(2).all(|w| w[1].dst.y == w[0].dst.y + pitch)
}

/// Whether these records share a baseline and a height.
fn on_one_line(run: &[Widget]) -> bool {
    let first = run[0].dst;
    run.iter()
        .all(|w| (w.dst.y, w.dst.height) == (first.y, first.height))
}

/// Whether these records share a baseline, a height and a width.
fn one_size_on_one_line(run: &[Widget]) -> bool {
    on_one_line(run) && run.iter().all(|w| w.dst.width == run[0].dst.width)
}

/// Whether these records advance left to right.
fn rightwards(run: &[Widget]) -> bool {
    run.windows(2).all(|w| w[1].dst.x > w[0].dst.x)
}

/// Where each view's records begin in the DLL image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pages {
    hscene: usize,
    playdata: usize,
    list: usize,
}

impl Pages {
    /// Locates both tables and the list run, given where the frame's table
    /// starts.
    ///
    /// `frame` is [`days_ui::atlas::Atlas::offset`] for the screen's own hit
    /// map. Returns `None` when either table cannot be placed unambiguously —
    /// a module with no such views, or one this search does not fit, leaves the
    /// screen its frame rather than inventing geometry for it.
    pub fn locate(dll: &[u8], frame: usize) -> Option<Pages> {
        let hscene = atlas::table_by_shape(dll, records(View::HScene), hscene_shape)?;
        let playdata = atlas::table_by_shape(dll, records(View::PlayData), playdata_shape)?;
        let list = frame.checked_add(LIST_AFTER_FRAME * 24)?;
        // The list run is reached by arithmetic, so check it decodes as far as
        // the lit page buttons before trusting the anchor it came from.
        atlas::record_at(dll, list, PLAYDATA_PAGE_MARK_RECORD + PER_PAGE - 1)?;
        Some(Pages {
            hscene,
            playdata,
            list,
        })
    }

    /// Where a view's records begin.
    pub fn base(&self, view: View) -> usize {
        match view {
            View::HScene => self.hscene,
            View::PlayData => self.playdata,
        }
    }

    /// Where the play-data list's sprite records begin — the run the hit table
    /// does not hold, from `DAT_10057a68`.
    pub fn list(&self) -> usize {
        self.list
    }

    /// A view widget's rectangle, or `None` when the widget is not one of that
    /// view's.
    pub fn widget(&self, dll: &[u8], view: View, widget: usize) -> Option<Widget> {
        let index = widget.checked_sub(FIRST)?;
        (index < records(view)).then(|| atlas::record_at(dll, self.base(view), index))?
    }

    /// A view's widgets, in widget order from [`FIRST`].
    pub fn widgets(&self, dll: &[u8], view: View) -> Vec<Widget> {
        (0..records(view))
            .map_while(|index| atlas::record_at(dll, self.base(view), index))
            .collect()
    }

    /// The widget under a point in layout space, as `FUN_1002cf00` finds it.
    ///
    /// First record containing the point wins — there is no z-order and no best
    /// match — and the test is half-open at the low edge and closed at the high
    /// one, which is the shipped comparison rather than a tidied version of it.
    pub fn hit(&self, dll: &[u8], view: View, x: u32, y: u32) -> Option<usize> {
        hit_in(&self.widgets(dll, view), x, y)
    }

    /// A thumbnail's sprite on a given page, from `FUN_10028260`.
    ///
    /// The rectangle is the same on every page; the page moves the art down the
    /// sheet `FUN_10026350` loads.
    pub fn thumbnail(&self, dll: &[u8], page: usize, slot: usize) -> Option<Widget> {
        if page >= HSCENE_PAGES || slot >= THUMBNAILS {
            return None;
        }
        atlas::record_at(
            dll,
            self.hscene,
            THUMBNAIL_RECORD + page * THUMBNAILS + slot,
        )
    }

    /// The page button of the page showing, drawn lit over the resting row.
    ///
    /// `FUN_10028260` binds the grid's to record `0x29 + page` in its own table
    /// and `FUN_100288a0` the list's to record `0x1e + page` in the list run.
    pub fn page_mark(&self, dll: &[u8], view: View, page: usize) -> Option<Widget> {
        match view {
            View::HScene => (page < HSCENE_PAGES)
                .then(|| atlas::record_at(dll, self.hscene, HSCENE_PAGE_MARK_RECORD + page))?,
            View::PlayData => (page < PER_PAGE)
                .then(|| atlas::record_at(dll, self.list, PLAYDATA_PAGE_MARK_RECORD + page))?,
        }
    }

    /// The sprite drawn for the widget under the pointer, from `FUN_10024480`,
    /// `FUN_10024c20` and `FUN_10024e30`, or `None` for a widget that has none.
    ///
    /// `page` is the page showing, which only the grid's thumbnails need.
    pub fn hover(&self, dll: &[u8], view: View, widget: usize, page: usize) -> Option<Widget> {
        match view {
            // The arrows and page buttons are drawn from their own records,
            // over the chip sheet rather than the page's art.
            View::HScene => match widget {
                FIRST_ARROW..FIRST_THUMBNAIL => self.widget(dll, view, widget),
                FIRST_THUMBNAIL..=0x13 => self.thumbnail(dll, page, widget - FIRST_THUMBNAIL),
                _ => None,
            },
            // Either band lights the whole row, from the list run's full-width
            // record rather than from the band's own.
            View::PlayData => match widget {
                FIRST_ROW..FIRST_PLAYDATA_PAGE => {
                    let row = (widget - FIRST_ROW) % PER_PAGE;
                    atlas::record_at(dll, self.list, row)
                }
                FIRST_PLAYDATA_PAGE..=0x20 => atlas::record_at(
                    dll,
                    self.list,
                    PLAYDATA_PAGE_RECORD + widget - FIRST_PLAYDATA_PAGE,
                ),
                _ => None,
            },
        }
    }
}

/// Which h-scene a thumbnail carries, from `FUN_10028260`: `page * 12 + slot`,
/// refused past [`SCENES`].
///
/// The index is into the run of scene flag names [`crate::ui::replay::Scenes`]
/// already recovers — `FUN_10028260` reads the same run, at
/// `PTR_u_REP02_28_A20_10057930`, and asks the host `+0x18` whether that flag
/// is set.
pub fn scene_of(page: usize, slot: usize) -> Option<usize> {
    (slot < THUMBNAILS)
        .then(|| page.checked_mul(THUMBNAILS)?.checked_add(slot))?
        .filter(|scene| *scene < SCENES)
}

/// Whether a widget can be chosen.
///
/// `FUN_1002a2d0` answers for the frame and hands the view to `FUN_1002a330`
/// (grid) or `FUN_1002a390` (list). Nothing on the list is ever greyed out, and
/// on the grid only the thumbnails are: `FUN_1002a330` returns the per-slot
/// member `FUN_10028260` fills from the scene's own flag, so a thumbnail is
/// live exactly when its h-scene has been seen. `unlocked` is that answer for
/// the twelve slots of the page showing.
pub fn enabled(view: View, widget: usize, unlocked: &[bool]) -> bool {
    match widget {
        0..FIRST => true,
        _ => match view {
            View::HScene => match widget {
                FIRST_ARROW..FIRST_THUMBNAIL => true,
                FIRST_THUMBNAIL..=0x13 => unlocked
                    .get(widget - FIRST_THUMBNAIL)
                    .copied()
                    .unwrap_or(false),
                _ => false,
            },
            View::PlayData => (FIRST..=0x20).contains(&widget),
        },
    }
}

/// What activating a widget on a Replay view does.
///
/// `FUN_1002a3e0` takes the frame — widgets 0 and 1 switch view, widget 2 is
/// CLOSE — and hands everything else to `FUN_1002a4a0` for the grid or
/// `FUN_1002a780` for the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// Nothing, or a widget this view has not got.
    None,
    /// Show the other view. Both dispatchers reset the page to 0 on the way —
    /// `FUN_100293e0` zeroes `+0x60c` whenever the view it is given differs
    /// from the one in force.
    View(View),
    /// Leave the Replay screen.
    Back,
    /// Show a page of the view.
    Page(usize),
    /// An h-scene thumbnail the player has unlocked, by its index into the
    /// scene run — see [`action`] for what it plays.
    Scene(usize),
    /// A row of the play-data list, by its index within the page. The save
    /// entry it loads is [`crate::ui::saveload::slot_of`] of the page and the
    /// row — see [`action`].
    Row(usize),
}

/// What activating a widget does, from `FUN_1002a4a0` and `FUN_1002a780`.
///
/// `page` is the page showing. The two arrows are the grid's only widgets that
/// depend on it: `FUN_1002a4a0` refuses the back arrow at page 0 and the
/// forward arrow at page 2, so neither wraps.
///
/// # What a thumbnail plays
///
/// A thumbnail sets the scene `+0x610` to `page * 12 + slot` and the step
/// `+0x614` to zero, then switches on the scene. Seven of them raise the
/// version popup through `FUN_100018b0(0, n)` — scenes 6, 8, 9, 10, 0x10, 0x11
/// and 0x19, for n = 0 to 6 in that order — and every other scene hands the
/// host `+0xb0` the script at `PTR_PTR_10057868[scene][step]`, which with the
/// step just zeroed is the scene's **first** script.
///
/// That is School Days HQ's `FUN_1001de10` over again: it zeroes the same pair,
/// raises the same popup for its own three scenes and calls host `+0xa4` — the
/// same slot, `0xc` lower, on the interface this title shifts — with element
/// zero of the same run. So a thumbnail is [`crate::ui::replay::Act::Play`] or
/// [`crate::ui::replay::Act::Ask`] exactly as it is there, and the scene run
/// [`crate::ui::replay::Scenes`] recovers is the one being indexed:
/// `FUN_1002ddf0` registers both tables from the same counter.
///
/// **Which seven ask is not written down here.** The seven the switch names are
/// exactly the seven whose own script list is a hole and whose flags carry `A`
/// and `B` versions — the scenes [`crate::ui::replay::Scene::asks`] already
/// answers for — so the caller asks the recovered table instead of the DLL's
/// case labels.
///
/// All seven pass `FUN_100018b0`'s first argument as 0, which is Pop_Replay's
/// two-widget variant: `FUN_10022ee0` reads it at `+0x110` and picks `L"2"`
/// with two widgets over `L"4"` with four. Those are the only seven callers —
/// Ghidra's reference index and a raw scan of `.text` for calls to
/// `0x100018b0` both find exactly them, the byte scan reading `PUSH 0` for the
/// variant at every one — so **the four-widget popup is unreachable in the
/// retail Shiny Days build**, which is what having two versions everywhere
/// means. `FUN_100238e0` says the same from the other side: its arms name
/// `REP03_3O_A06A`/`B` through `REP04_YX_A01A`/`B`, the fourteen version flags
/// of those seven scenes and nothing else.
///
/// # What a list row loads
///
/// A row is refused unless `+0x1cc[row]` is set, and otherwise hands the host
/// `+0x54` the entry `page * 10 + +0x624 + row`, then `+0xa0(1)` and
/// `+0x58(8)`. School Days HQ's `FUN_1001dfe0` makes the same three calls with
/// the same literals at `+0x48`, `+0x94` and `+0x4c` — the same slots `0xc`
/// lower — so the row loads that save entry, which is what
/// [`crate::ui::menu::Action::PlayRecorded`] already is.
///
/// `+0x1cc` is the ten rows on screen: `FUN_100267c0` fills each from the
/// host's answer for that entry, which is why an empty row does nothing.
/// `+0x624` is how many rows the list has been flicked past the page's own top
/// — `FUN_1002c1f0` sets it from the settled scroll divided by a tenth of the
/// panel height and holds it under ten, and every page button zeroes it through
/// `FUN_1002d060`. This engine has no flick, so it is zero and the entry is the
/// page and the row.
pub fn action(view: View, widget: usize, page: usize) -> Act {
    match widget {
        0 => return Act::View(View::HScene),
        1 => return Act::View(View::PlayData),
        2 => return Act::Back,
        _ => {}
    }
    match view {
        View::HScene => match widget {
            // The arrows step one page and stop at the ends.
            3 => page.checked_sub(1).map_or(Act::None, Act::Page),
            4 if page + 1 < HSCENE_PAGES => Act::Page(page + 1),
            FIRST_HSCENE_PAGE..FIRST_THUMBNAIL => Act::Page(widget - FIRST_HSCENE_PAGE),
            FIRST_THUMBNAIL..=0x13 => {
                scene_of(page, widget - FIRST_THUMBNAIL).map_or(Act::None, Act::Scene)
            }
            _ => Act::None,
        },
        // Either band activates the row it belongs to, the same way both light
        // it. The page buttons animate to an adjacent page and jump to a
        // distant one (`FUN_1002d060` against `FUN_1002d260`), which is one
        // move either way from here.
        View::PlayData => match widget {
            FIRST_ROW..FIRST_PLAYDATA_PAGE => Act::Row((widget - FIRST_ROW) % PER_PAGE),
            FIRST_PLAYDATA_PAGE..=0x20 => Act::Page(widget - FIRST_PLAYDATA_PAGE),
            _ => Act::None,
        },
    }
}

/// The widget a point lands on among records already read out of the image.
pub fn hit_in(records: &[Widget], x: u32, y: u32) -> Option<usize> {
    records
        .iter()
        .position(|w| atlas::contains(&w.dst, x, y))
        .map(|index| index + FIRST)
}

/// The surface one panel's ten rows are rasterised into, from `FUN_100296a0`:
/// six `FrameBuffer`s built `(0x400, 0x400, 0x208888)`, one per panel, and
/// `FUN_100267c0` clears its own as 0x400 rows of 0x1000 bytes.
pub const LIST_SURFACE: (u32, u32) = (0x400, 0x400);

/// How many characters of any column are drawn, from `FUN_100267c0`'s
/// `if (0x14 < n) n = 0x14`. The same cap on all three, English or not.
pub const CAP: usize = 0x14;

/// One row of the surface, `_DAT_1004bd90` — an `fmull`, so the double 48.0 —
/// and how tall each column's slice of it is, `_DAT_10049794`, an `flds` so the
/// float 48.0. The same number, so the rows abut exactly.
const SURFACE_ROW_PITCH: f32 = 48.0;
const SURFACE_ROW_HEIGHT: f32 = 48.0;

/// Where `FUN_100267c0`'s pen starts down a row for the two halves of the
/// stored line and for the comment — `0` and `0x202` — against the cut
/// `FUN_10027c10` makes at `_DAT_10049760` (2.0) and `_DAT_1004bd68` (514.0),
/// both `faddl` so both doubles.
const PEN_LINE_Y: f32 = 0.0;
const PEN_COMMENT_Y: f32 = 514.0;
const CUT_LINE_Y: f32 = 2.0;
const CUT_COMMENT_Y: f32 = 514.0;

/// How far down its record every column is drawn, `_DAT_10049760` (2.0), and
/// how tall, `_DAT_1004b288` (24.0) — half the surface row, which is why the
/// text is rasterised at the font's own cell and comes down to size in the
/// blit.
const DEST_Y: f32 = 2.0;
const DEST_HEIGHT: f32 = 24.0;

/// The advance for one character, from `FUN_100267c0` and `FUN_100271f0`.
///
/// Both spell it out in full — `0x18` below U+0080 and `0x2d` at or above —
/// with no kerning table and no language test, where School Days HQ's lists go
/// through `FUN_10011dc0` and pick up
/// [`crate::playback::text::menu_advance`]'s English kerning. The two agree on
/// everything but an English ASCII character.
pub fn advance(c: char) -> i32 {
    if u32::from(c) >= 0x80 {
        0x2d
    } else {
        0x18
    }
}

/// Where in the surface a column's glyphs start, and how wide the sprite cuts
/// it.
///
/// The x origins are `FUN_100267c0`'s own pen starts — 0, 600 and 0 — the
/// chapter's being `_DAT_1004bd78` (600.0f) again on the cutting side, and the
/// widths are `_DAT_1004bd98` (548.0f) and `_DAT_1004bd38` (986.0f). The
/// chapter's cut therefore ends at 1148 on a surface 1024 wide; see the module
/// doc.
fn surface_span(column: Column) -> (f32, f32) {
    match column {
        Column::When => (0.0, 548.0),
        Column::Chapter => (600.0, 548.0),
        Column::Comment => (0.0, 986.0),
    }
}

/// The x this column is offset by inside its record, and the extra shift
/// English adds.
///
/// `_DAT_1004bd80` (95.0), `_DAT_10049738` (20.0) and `_DAT_10049760` (2.0),
/// all `faddl` so all doubles; the English shifts are `_DAT_1004bda0` (5.0f)
/// and `_DAT_1004bd9c` (15.0f), the same pair the save/load screen uses. The
/// comment has none — it is centred instead, in [`comment_centre`].
fn dest_x(column: Column, english: bool) -> f32 {
    let shift = if english { 1.0 } else { 0.0 };
    match column {
        Column::When => 95.0 + 5.0 * shift,
        Column::Chapter => 20.0 + 15.0 * shift,
        Column::Comment => 2.0,
    }
}

/// How wide this column is drawn: `_DAT_1004bd88` (189.0), `_DAT_1004bd70`
/// (252.0) and `_DAT_1004bd10` (494.0), all `fmull` so all doubles. Each is
/// cut wider than it is drawn, so every column is squeezed horizontally — the
/// timestamp's 548 into 189 hardest of the three.
fn dest_width(column: Column) -> f32 {
    match column {
        Column::When => 189.0,
        Column::Chapter => 252.0,
        Column::Comment => 494.0,
    }
}

/// How far right an English comment is pushed, from `FUN_100267c0`.
///
/// `_DAT_1004bd18 - width / _DAT_1004bd20`, clamped to zero below
/// `_DAT_10049770` — an `fsubrl`, an `fdivl` and an `fcompl`, so 226.5, 4.0 and
/// 0.0. `width` is the advance total the rasterising loop accumulated, in
/// surface pixels, and the column comes down to the screen at half, so dividing
/// by four is half the drawn width. Japanese comments are not moved at all.
///
/// The clamp is unreachable here: [`CAP`] characters at the widest advance
/// total 900, which leaves 1.5. It is kept because it is what the function
/// does, not because anything reaches it.
pub fn comment_centre(width: i32, english: bool) -> f32 {
    if !english {
        return 0.0;
    }
    (226.5 - width as f32 / 4.0).max(0.0)
}

/// Whether the page showing keeps the centring worked out for its comments.
///
/// `FUN_100267c0` zeroes all six panels' centres on entry and refills only its
/// own, `FUN_100288a0` runs it over panels 0 to 5 in order and is the only
/// thing that runs it, and `FUN_1002c1f0` re-reads the array every frame. So
/// the centres that survive belong to panel `PLAYDATA_PANELS - 1`, and the
/// page showing is in that panel only at the end of the strip — see the module
/// doc.
pub fn comment_centred(page: usize) -> bool {
    page.checked_sub(window_top(page)) == Some(PLAYDATA_PANELS - 1)
}

/// Where the expanded comment's line `n` is really rasterised, from
/// `FUN_100271f0` and the shape of the buffer it writes into.
///
/// The pen is `(0, 0)` and every twentieth character drops it `0x3a` and sends
/// it back to `0x400`. The buffer is 1024 pixels wide and the blitter is given
/// a pitch rather than a width, so `0x400` is the first pixel of the next
/// scanline: every line after the first lands at x 0, one pixel below its own
/// `0x3a` step.
fn tip_pen(n: usize, base: f32) -> (i32, i32) {
    let folded = if n == 0 { 0.0 } else { 1.0 };
    (0, (base + n as f32 * TIP_PEN_PITCH + folded) as i32)
}

/// Which record of the list run places a column of a row.
///
/// `FUN_10027c10` reads `row` for the timestamp and the chapter and `row + 10`
/// for the comment — the row bar and the comment panel beside it.
pub fn record_of(column: Column, row: usize) -> usize {
    match column {
        Column::When | Column::Chapter => row,
        Column::Comment => row + PER_PAGE,
    }
}

/// Where in the surface a column of a row is rasterised, from `FUN_100267c0`.
fn surface_pen(column: Column, row: usize) -> (i32, i32) {
    let base = match column {
        Column::When | Column::Chapter => PEN_LINE_Y,
        Column::Comment => PEN_COMMENT_Y,
    };
    (
        surface_span(column).0 as i32,
        (row as f32 * SURFACE_ROW_PITCH + base) as i32,
    )
}

/// The rectangle of the surface a column of a row is cut from, from
/// `FUN_10027c10`.
fn source_rect(column: Column, row: usize) -> (f32, f32, f32, f32) {
    let (x, width) = surface_span(column);
    let base = match column {
        Column::When | Column::Chapter => CUT_LINE_Y,
        Column::Comment => CUT_COMMENT_Y,
    };
    (
        x,
        row as f32 * SURFACE_ROW_PITCH + base,
        width,
        SURFACE_ROW_HEIGHT,
    )
}

/// Where a column of a row is drawn, in the 800x450 layout space the records
/// use, from `FUN_10027c10`.
fn dest_rect(
    column: Column,
    record: days_ui::cmap::Rect,
    english: bool,
    centre: f32,
) -> (f32, f32, f32, f32) {
    (
        record.x as f32 + dest_x(column, english) + centre,
        record.y as f32 + DEST_Y,
        dest_width(column),
        DEST_HEIGHT,
    )
}

/// The row whose comment the pointer expands, from `FUN_10024e30`'s
/// `0xc < selection` and its `selection - 0xd`.
///
/// The page buttons above the band pass that test too, and ask for a row from
/// ten up. `FUN_100267c0` only ever files the ten rows it drew, so those find
/// nothing and nothing is drawn — which is what this refuses outright.
pub fn tooltip_row(widget: usize) -> Option<usize> {
    (FIRST_COMMENT..FIRST_COMMENT + PER_PAGE)
        .contains(&widget)
        .then(|| widget - FIRST_COMMENT)
}

/// The expanded comment's own surface, from `FUN_100271f0`: a seventh buffer at
/// `+0x1a4`, cleared 0x100 rows of 0x1000 bytes.
const TIP_SURFACE: (u32, u32) = (0x400, 0x100);

/// How many characters of the comment it lays out, `0x3c` — three lines of
/// [`CAP`].
const TIP_CAP: usize = 0x3c;

/// How far the pen drops at each twentieth character, `0x3a`. Where it goes
/// back to is [`tip_pen`].
const TIP_PEN_PITCH: f32 = 58.0;

/// How far down the surface a deep row's lines are rasterised so the fixed cut
/// picks them up in the right slot, from `FUN_100271f0`: `0x7e` for one line
/// and `0x3b` for two. Three lines fill the slice and are not moved.
const TIP_DEEP_PEN: [f32; 2] = [126.0, 59.0];

/// What the expanded comment's one sprite cuts — `_DAT_1004bdbc` (2.0f),
/// `_DAT_1004bd38` (986.0f) and `_DAT_1004dabc` (192.0f), three lines' worth in
/// one piece — and how tall it is drawn, `_DAT_1004dac0` (96.0), so the three
/// line slots are 32 screen pixels each.
const TIP_CUT: (f32, f32, f32, f32) = (0.0, 2.0, 986.0, 192.0);
const TIP_DEST_HEIGHT: f32 = 96.0;

/// How many rows of the list the panel's record spans, `_DAT_1004bd58` — an
/// `fdivl`, so the double 3.0 — and how much a shortened panel gives back,
/// which is that same 3.0 and `_DAT_1004bd50` (3.0f) again for a row opening
/// upwards.
const PANEL_ROWS: f32 = 3.0;
const PANEL_INSET: f32 = 3.0;

/// The last row whose panel can open downwards, from `FUN_100271f0`'s
/// `7 < param_1` test.
const LAST_ROW_OPENING_DOWN: usize = 7;

/// Rasterises a page of the list.
///
/// A row whose slot has no file is skipped entirely, which is what
/// `FUN_100267c0` does: it only draws when the host's entry query answers
/// non-zero. `list` is the list run [`Pages::list`] anchors, `comments` is host
/// `+0xf4` — `FILMENGINE.INI [TextInput]` — and `english` is host `+0x68`.
/// `hovered` is the row the pointer is expanding, from [`tooltip_row`].
pub fn render(
    font: &days_font::Font,
    slots: &Slots,
    page: usize,
    list: &[Widget],
    comments: bool,
    english: bool,
    hovered: Option<usize>,
) -> Rows {
    let (width, height) = LIST_SURFACE;
    let mut surface = days_ui::Image::empty(width, height);
    let mut quads = Vec::new();

    for row in 0..PER_PAGE {
        let Some(line) = slots.get(saveload::slot_of(page, row)) else {
            continue;
        };
        for column in Column::ALL {
            if column == Column::Comment && !comments {
                continue;
            }
            let text: String = text_of(line, column).chars().take(CAP).collect();
            if text.is_empty() {
                continue;
            }
            let Some(record) = list.get(record_of(column, row)) else {
                continue;
            };
            let drawn = saveload::draw_line(
                &mut surface,
                font,
                &text,
                surface_pen(column, row),
                &advance,
            );
            let centre = match column {
                Column::Comment if comment_centred(page) => comment_centre(drawn, english),
                _ => 0.0,
            };
            let (sx, sy, sw, sh) = source_rect(column, row);
            quads.push(Quad {
                src: (sx as u32, sy as u32, sw as u32, sh as u32),
                dst: dest_rect(column, record.dst, english, centre),
            });
        }
    }

    let (tooltip, tip_surface) = match hovered
        .filter(|_| comments)
        .and_then(|row| expand(font, slots, page, row, list))
    {
        Some((tip, surface)) => (Some(tip), Some(surface)),
        None => (None, None),
    };
    Rows {
        surface,
        quads,
        tooltip,
        tip_surface,
    }
}

fn text_of(line: &Line, column: Column) -> &str {
    match column {
        Column::When => &line.when,
        Column::Chapter => &line.chapter,
        Column::Comment => &line.comment,
    }
}

/// How far the expanded comment's panel is moved, how tall it is drawn and how
/// far down the surface its lines are rasterised, from `FUN_100271f0`.
///
/// The line count is the **character count** over [`CAP`], not the wrap's own:
/// the shipped switch divides the capped length and takes three arms off it.
/// A row past [`LAST_ROW_OPENING_DOWN`] opens upwards — its panel starts as far
/// down the borrowed record as the lines it does not need, and its text is
/// rasterised into the matching slot of the one fixed cut. All three are zeroed
/// before the branches that set them, so a shallow row reads no uninitialised
/// float here where the save/load screen's pair do.
fn panel_geometry(row: usize, chars: usize, height: f32) -> (f32, f32, f32) {
    let deep = row > LAST_ROW_OPENING_DOWN;
    let row_of_panel = height / PANEL_ROWS;
    let given_back = if deep { PANEL_INSET } else { 0.0 };
    match chars.min(TIP_CAP) / CAP {
        0 => (
            if deep {
                row_of_panel * 2.0 + PANEL_INSET
            } else {
                0.0
            },
            row_of_panel - PANEL_INSET - given_back,
            if deep { TIP_DEEP_PEN[0] } else { 0.0 },
        ),
        1 => (
            if deep {
                row_of_panel + PANEL_INSET
            } else {
                0.0
            },
            row_of_panel * 2.0 - PANEL_INSET - given_back,
            if deep { TIP_DEEP_PEN[1] } else { 0.0 },
        ),
        _ => (0.0, height, 0.0),
    }
}

/// Rasterises the expanded comment into a surface of its own and lays it out,
/// from `FUN_100271f0`.
///
/// The wrap is a hard break every [`CAP`] characters over the first [`TIP_CAP`]
/// of the comment, with no language test at all — which is
/// [`saveload::wrap_comment`]'s Japanese arm. Only the first line lands inside
/// the rectangle the sprite cuts; see the module doc.
fn expand(
    font: &days_font::Font,
    slots: &Slots,
    page: usize,
    row: usize,
    list: &[Widget],
) -> Option<(Tooltip, days_ui::Image)> {
    let comment = &slots.get(saveload::slot_of(page, row))?.comment;
    let lines = saveload::wrap_comment(comment, false);
    if lines.is_empty() {
        return None;
    }
    let record = *list.get(record_of(Column::Comment, Tooltip::record_row(row)))?;

    let (panel_shift, panel_height, pen_shift) =
        panel_geometry(row, comment.chars().count(), record.dst.height as f32);

    let (width, tip_height) = TIP_SURFACE;
    let mut surface = days_ui::Image::empty(width, tip_height);
    for (n, line) in lines.iter().enumerate() {
        saveload::draw_line(&mut surface, font, line, tip_pen(n, pen_shift), &advance);
    }

    let panel = Quad {
        // The half-pixel outset is the DLL's, on this sprite as on every other.
        src: (
            record.src_x,
            record.src_y,
            record.dst.width,
            record.dst.height,
        ),
        dst: (
            record.dst.x as f32 - 0.5,
            record.dst.y as f32 - 0.5 + panel_shift,
            record.dst.width as f32 + 1.0,
            panel_height + 1.0,
        ),
    };
    // One sprite, three line slots. Which slot the text lands in was decided by
    // where it was rasterised, not by where this is drawn.
    let text = Quad {
        src: (
            TIP_CUT.0 as u32,
            TIP_CUT.1 as u32,
            TIP_CUT.2 as u32,
            TIP_CUT.3 as u32,
        ),
        dst: (
            record.dst.x as f32 + dest_x(Column::Comment, false),
            record.dst.y as f32 + DEST_Y,
            dest_width(Column::Comment),
            TIP_DEST_HEIGHT,
        ),
    };
    Some((
        Tooltip {
            panel,
            lines: vec![text],
        },
        surface,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(v: [f32; 6]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    /// A list run whose ten row bars carry a recognisable `src_y`, followed by
    /// enough records to reach the page buttons at 0x14 and their lit run at
    /// 0x1e.
    fn list_run() -> Vec<u8> {
        let mut dll = Vec::new();
        for row in 0..PER_PAGE {
            let y = 126.0 + row as f32 * 30.0;
            dll.extend(rec([30.0, y, 740.0, 27.0, 1.0, y]));
        }
        for index in PER_PAGE..PLAYDATA_PAGE_MARK_RECORD + PER_PAGE {
            dll.extend(rec([0.0, 0.0, 1.0, 1.0, index as f32, 0.0]));
        }
        dll
    }

    /// The six-panel strip is a window over the ten pages, from
    /// `FUN_100267c0`'s page-at-a-time chain: the page showing is third of the
    /// six once there is room either side, and the window stops at the last
    /// page rather than running past it.
    #[test]
    fn the_list_strip_is_a_window_over_the_ten_pages() {
        let tops: Vec<usize> = (0..PLAYDATA_PAGES).map(window_top).collect();
        assert_eq!(tops, [0, 0, 0, 1, 2, 3, 4, 4, 4, 4]);
        // Every page falls inside its own window, and the last window ends on
        // the last page.
        for (page, top) in tops.iter().enumerate() {
            assert!((*top..top + PLAYDATA_PANELS).contains(&page));
        }
        assert_eq!(tops[PLAYDATA_PAGES - 1] + PLAYDATA_PANELS, PLAYDATA_PAGES);
    }

    /// Both bands of the list light the same bar: `FUN_10024e30` draws list
    /// record `row` for a selection of `row + 3` **or** `row + 0xd`, and that
    /// record is the row entire rather than the band the pointer is in.
    #[test]
    fn either_band_lights_the_whole_row() {
        let dll = list_run();
        let pages = Pages {
            hscene: 0,
            playdata: 0,
            list: 0,
        };
        for row in 0..PER_PAGE {
            let left = pages.hover(&dll, View::PlayData, FIRST_ROW + row, 0);
            let right = pages.hover(&dll, View::PlayData, FIRST_COMMENT + row, 0);
            assert_eq!(left, right);
            assert_eq!(left.unwrap().dst.width, 740);
        }
    }

    /// The page buttons are reached in the same run at `0x14 + button`, so they
    /// are not the rows continued.
    #[test]
    fn page_buttons_are_a_run_of_their_own() {
        let dll = list_run();
        let pages = Pages {
            hscene: 0,
            playdata: 0,
            list: 0,
        };
        let button = pages
            .hover(&dll, View::PlayData, FIRST_PLAYDATA_PAGE, 0)
            .expect("the run reaches 0x14");
        assert_eq!(button.src_x, PLAYDATA_PAGE_RECORD as u32);
        assert_eq!(
            pages.page_mark(&dll, View::PlayData, 0).unwrap().src_x,
            PLAYDATA_PAGE_MARK_RECORD as u32
        );
    }

    /// `FUN_1002cf00`'s containment test is half-open at the low edge and
    /// closed at the high one, and it answers `index + 3`.
    #[test]
    fn a_record_owns_its_far_edge_and_not_its_near_one() {
        let widgets = [Widget {
            dst: days_ui::cmap::Rect {
                x: 56,
                y: 119,
                width: 162,
                height: 92,
            },
            src_x: 1,
            src_y: 1,
        }];
        assert_eq!(hit_in(&widgets, 56 + 162, 119 + 92), Some(FIRST));
        assert_eq!(hit_in(&widgets, 56, 119 + 92), None);
        assert_eq!(hit_in(&widgets, 56 + 162, 119), None);
    }

    /// A thumbnail is live only while its own h-scene has been seen; the arrows
    /// and page buttons beside it never are greyed, and neither is anything on
    /// the list.
    #[test]
    fn only_the_thumbnails_are_greyed_out() {
        let mut unlocked = [false; THUMBNAILS];
        unlocked[3] = true;
        assert!(enabled(View::HScene, FIRST_THUMBNAIL + 3, &unlocked));
        assert!(!enabled(View::HScene, FIRST_THUMBNAIL, &unlocked));
        assert!(enabled(View::HScene, FIRST_ARROW, &unlocked));
        assert!(enabled(View::HScene, FIRST_HSCENE_PAGE, &unlocked));
        for widget in FIRST..=0x20 {
            assert!(enabled(View::PlayData, widget, &[]));
        }
    }

    /// The grid reaches thirty-six scenes and stops: `FUN_10028260` tests
    /// `page * 12 + slot` against 0x24 before it looks a flag up, so a page past
    /// the third would have none.
    #[test]
    fn the_grid_stops_at_the_scene_run() {
        assert_eq!(scene_of(HSCENE_PAGES - 1, THUMBNAILS - 1), Some(SCENES - 1));
        assert_eq!(scene_of(HSCENE_PAGES, 0), None);
        assert_eq!(scene_of(0, THUMBNAILS), None);
    }

    /// Neither arrow wraps. `FUN_1002a4a0` takes the back arrow only while the
    /// page is above zero and the forward arrow only while it is below two, so
    /// the ends of the strip are dead rather than circular.
    #[test]
    fn the_grid_arrows_stop_at_the_ends() {
        assert_eq!(action(View::HScene, 3, 0), Act::None);
        assert_eq!(action(View::HScene, 3, 1), Act::Page(0));
        assert_eq!(action(View::HScene, 4, HSCENE_PAGES - 1), Act::None);
        assert_eq!(action(View::HScene, 4, 0), Act::Page(1));
    }

    /// Both bands of the list are the same ten rows. `FUN_1002a780` runs the
    /// identical body for widgets 3 to 0xc and 0xd to 0x16, each less its own
    /// base, so the comment column activates the row beside it rather than
    /// anything of its own.
    #[test]
    fn either_band_activates_the_same_row() {
        for row in 0..PER_PAGE {
            assert_eq!(
                action(View::PlayData, FIRST_ROW + row, 0),
                action(View::PlayData, FIRST_COMMENT + row, 0)
            );
            assert_eq!(action(View::PlayData, FIRST_ROW + row, 0), Act::Row(row));
        }
    }

    /// Only the comment band opens the expanded comment, and the page buttons
    /// that pass `FUN_10024e30`'s `0xc < selection` do not: they name a row the
    /// panel never filled.
    #[test]
    fn only_the_comment_band_expands_a_row() {
        for row in 0..PER_PAGE {
            assert_eq!(tooltip_row(FIRST_COMMENT + row), Some(row));
            assert_eq!(tooltip_row(FIRST_ROW + row), None);
        }
        for button in 0..PLAYDATA_PAGES {
            assert_eq!(tooltip_row(FIRST_PLAYDATA_PAGE + button), None);
        }
    }

    /// The centring survives on one page only. `FUN_100267c0` wipes all six
    /// panels' centres every time it fills one, `FUN_100288a0` fills them 0 to
    /// 5 in order and nothing else fills any, and `FUN_1002c1f0` re-places the
    /// comment sprites from what is left — so the page sitting in the last
    /// panel is the only one that keeps it, and that is the tenth.
    #[test]
    fn the_last_page_is_the_only_one_whose_comments_are_centred() {
        let centred: Vec<usize> = (0..PLAYDATA_PAGES)
            .filter(|p| comment_centred(*p))
            .collect();
        assert_eq!(centred, [PLAYDATA_PAGES - 1]);
        // And on that page it is School Days HQ's rule 9 short of its 235.5.
        // Its clamp cannot be reached: twenty of the widest characters advance
        // 900, which is 1.5 short of turning the shift negative.
        assert_eq!(comment_centre(CAP as i32 * 0x18, true), 106.5);
        assert_eq!(comment_centre(CAP as i32 * 0x2d, true), 1.5);
        assert_eq!(comment_centre(CAP as i32 * 0x18, false), 0.0);
    }

    /// Every line of the expanded comment after the first is sent back to
    /// `0x400`, which on a buffer 1024 wide is the first pixel of the next
    /// scanline — so it lands at x 0, one pixel below its own `0x3a` step, and
    /// inside the rectangle the sprite cuts.
    #[test]
    fn the_expanded_comments_lines_fold_onto_the_next_scanline() {
        assert_eq!(tip_pen(0, 0.0), (0, 0));
        assert_eq!(tip_pen(1, 0.0), (0, 59));
        assert_eq!(tip_pen(2, 0.0), (0, 117));
        // All three sit inside the cut, which starts two pixels down and runs
        // 192 deep.
        for n in 0..saveload::TIP_LINES {
            let (x, y) = tip_pen(n, 0.0);
            assert!(x >= TIP_CUT.0 as i32 && (x as f32) < TIP_CUT.0 + TIP_CUT.2);
            assert!((y as f32) + SURFACE_ROW_HEIGHT <= TIP_CUT.1 + TIP_CUT.3);
        }
    }

    /// A deep row's panel opens upwards and its text is rasterised into the
    /// slot that opening leaves, from `FUN_100271f0`'s three arms. A shallow
    /// row never moves either.
    #[test]
    fn a_deep_rows_panel_opens_upwards_into_the_record_above() {
        let height = 87.0;
        let third = height / PANEL_ROWS;
        assert_eq!(panel_geometry(0, 5, height), (0.0, third - 3.0, 0.0));
        assert_eq!(panel_geometry(0, 25, height), (0.0, third * 2.0 - 3.0, 0.0));
        assert_eq!(panel_geometry(0, 45, height), (0.0, height, 0.0));
        assert_eq!(
            panel_geometry(9, 5, height),
            (third * 2.0 + 3.0, third - 6.0, 126.0)
        );
        assert_eq!(
            panel_geometry(9, 25, height),
            (third + 3.0, third * 2.0 - 6.0, 59.0)
        );
        // Three lines fill the record whichever way the row opens, and the row
        // that borrows a record borrows the one two above it.
        assert_eq!(panel_geometry(9, 45, height), (0.0, height, 0.0));
        assert_eq!(saveload::Tooltip::record_row(7), 7);
        assert_eq!(saveload::Tooltip::record_row(8), 6);
    }

    /// The list's ten pages all rest on one sheet, where the grid's three each
    /// have their own: `FUN_10025e70` loads `ReplayList.png` once and gives it
    /// six sprites, while `FUN_10025770` formats a new path per page.
    #[test]
    fn the_list_pages_share_one_panel() {
        let list: Vec<Option<String>> = (0..PLAYDATA_PAGES)
            .map(|page| panel_art(View::PlayData, page))
            .collect();
        assert!(list.iter().all(|path| *path == list[0]));
        assert_eq!(panel_art(View::PlayData, PLAYDATA_PAGES), None);

        let grid: Vec<Option<String>> = (0..HSCENE_PAGES)
            .map(|page| panel_art(View::HScene, page))
            .collect();
        assert_eq!(
            grid.iter().collect::<std::collections::HashSet<_>>().len(),
            HSCENE_PAGES
        );
        assert_eq!(panel_art(View::HScene, HSCENE_PAGES), None);
    }
}
