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
//! sprites, and `FUN_10024480` is the order: each page's full-screen art, then
//! the contents of the view showing (`FUN_10024c20` for the grid,
//! `FUN_10024e30` for the list), then the frame's header highlight and its
//! hovered widget. [`crate::ui::screen::Page`] is that layer.
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
//! page's art out of the run behind it. The resting grid is in the page's own
//! full-screen art; only the thumbnail under the pointer is drawn from the
//! sheet `FUN_10026350` loads.
//!
//! Hovering **either** band of the list lights the same row: `FUN_10024e30`
//! draws list record `row` when the selection is `row + 3` or `row + 0xd`, and
//! that record is the row entire rather than the band. School Days HQ's list
//! does the same thing.
//!
//! # What is not recovered here
//!
//! The list's text — the timestamps, chapters and comments the ten rows carry —
//! is rasterised into six 1024x1024 surfaces by the loops at the foot of
//! `FUN_100288a0`, and the expanded comment's two sprites (`+0xf4` and
//! `+0x4c4`, over list records 0xa to 0x13) come from the same place. **Which
//! record and which host call fill each of those is not recovered**, so this
//! module places none of them.

use crate::ui::replay::View;
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

/// The widget a point lands on among records already read out of the image.
pub fn hit_in(records: &[Widget], x: u32, y: u32) -> Option<usize> {
    records
        .iter()
        .position(|w| atlas::contains(&w.dst, x, y))
        .map(|index| index + FIRST)
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
}
