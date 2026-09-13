//! The route map — mode 6, `MENU::RouteMap`, `System/RouteMap`.
//!
//! A chart of one episode's story points, opened from the Load screen's own
//! widget `0x15`. Each cell on it is a story point, `SP%03d`, and the two
//! stores answer two different questions about it:
//!
//! ```text
//! host +0x18  the global store   has the player ever seen it -> it is drawn
//! host +0x10  the save's store   did they pass it this run   -> it can be picked
//! ```
//!
//! `FUN_1000c740` is that pair, and the second half of it runs only when the
//! module's `+0x4ec` is set. That member is the whole difference between the
//! screen's two lives: `_SystemInit@8` case 6 opens it with **0** and
//! `setSystemInit` case 6 with **1**, and those are the title-rooted shell and
//! the driver that runs over playback. So the route map reached from the title
//! is a chart to look at — every cell inert — and the same screen reached from
//! the control bar during a playthrough is how the player jumps back to a story
//! point they have passed.
//!
//! Picking a cell hands the host `+0x48(story)` and `+0x4c(8)` — the same pair
//! the Load screen's rows use, with a number of 100 or more rather than a slot
//! — and raises `_GetRouteLoad@0`. The engine's `FUN_00423a70` sends a number
//! that size to `FUN_00428400`, and the flag is what stops `FUN_00432850` from
//! emptying the story points on the way, since it is those the jump reads.
//!
//! Recovered from `MENU::RouteMap`: `FUN_1000e8b0` (the dispatch),
//! `FUN_1000e750` (enablement), `FUN_1000d890` (the page table and the art),
//! `FUN_1000c740` (the cell states) and `FUN_1000e060` (the open).

/// What a widget of the route map does, from `FUN_1000e8b0`.
///
/// The table is fixed except for its tail: the cells run from `0xb` for as many
/// as the page holds, and everything at or past that is nothing at all.
///
/// ```text
/// 0x00 .. 0x05   the six episode tabs
/// 0x06           close, back to the Load screen
/// 0x07  0x08     previous episode, next episode
/// 0x09  0x0a     previous page, next page
/// 0x0b ..        the story points of this page, in order
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// Go to this episode, 0-based.
    Episode(usize),
    /// Back to the Load screen.
    Close,
    PrevEpisode,
    NextEpisode,
    PrevPage,
    NextPage,
    /// The story point at this index into the page.
    Cell(usize),
    None,
}

/// How many episodes the chart has. `FUN_1000e750` clamps the tabs to this.
pub const EPISODES: usize = 6;

/// The widget the cells start at.
const FIRST_CELL: usize = 0xb;

/// What a widget does, given how many cells the page showing has.
pub fn action(cells: usize, widget: usize) -> Act {
    match widget {
        0..=5 => Act::Episode(widget),
        6 => Act::Close,
        7 => Act::PrevEpisode,
        8 => Act::NextEpisode,
        9 => Act::PrevPage,
        10 => Act::NextPage,
        w if w >= FIRST_CELL && w < FIRST_CELL + cells => Act::Cell(w - FIRST_CELL),
        _ => Act::None,
    }
}

/// One page of one episode: how many story points it charts, and the number
/// the first of them carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    /// `+0x3f0`, the number of cells.
    pub cells: usize,
    /// `+0x3ec`, added to a cell's index to make the story number.
    pub base: u32,
}

/// The page table, straight out of `FUN_1000d890`'s switch.
///
/// Episodes 1 and 2 are a single page; the rest chart too much for one and are
/// paged. The count in the third column is what `+0x3e8` is set to — **zero**
/// for the single-page episodes, which is why `FUN_1000e750` disables the page
/// buttons by episode rather than by that count.
///
/// Every entry is confirmed twice over against the player's own install. A
/// page's record table is always `0x21 + 4 * cells` records long — 33 fixed
/// ones and four bands of one per cell — and its hit map has `11 + cells`
/// regions, the eleven fixed widgets and one per story point. All sixteen
/// pages agree with both.
const PAGES: [&[Page]; EPISODES] = [
    &[Page { cells: 4, base: 0 }],
    &[Page { cells: 9, base: 0 }],
    &[Page { cells: 7, base: 0 }, Page { cells: 4, base: 7 }],
    &[
        Page { cells: 8, base: 0 },
        Page { cells: 8, base: 8 },
        Page {
            cells: 15,
            base: 0x10,
        },
        Page {
            cells: 4,
            base: 0x1f,
        },
    ],
    &[
        Page { cells: 11, base: 0 },
        Page {
            cells: 18,
            base: 0xb,
        },
        Page {
            cells: 19,
            base: 0x1d,
        },
        Page {
            cells: 9,
            base: 0x30,
        },
    ],
    &[
        Page { cells: 11, base: 0 },
        Page {
            cells: 9,
            base: 0xb,
        },
        Page {
            cells: 13,
            base: 0x14,
        },
        Page {
            cells: 12,
            base: 0x21,
        },
    ],
];

/// `+0x3e8` for an episode: 0 for the two that are a single page.
fn page_count(episode: usize) -> usize {
    match episode {
        0 | 1 => 0,
        2 => 2,
        _ => 4,
    }
}

/// The page an episode is showing, or `None` when the pair names no page.
///
/// The paging quirks below can leave an episode on a page it does not have —
/// tabbing from episode 4's last page to episode 1 parks the page at 1 — and
/// the original reads its table with that index anyway. Here that is `None`,
/// and the screen falls back to the episode's first page.
pub fn page(episode: usize, page: usize) -> Option<Page> {
    PAGES.get(episode)?.get(page).copied()
}

/// The story point a cell stands for: `(episode + 1) * 100 + base + cell`.
///
/// `FUN_1000c740` formats exactly this into `SP%03d` to ask the stores about a
/// cell, and `FUN_1000e8b0` hands exactly this to host `+0x48` when one is
/// picked. The two being the same number is what makes the chart and the jump
/// agree.
pub fn story(episode: usize, base: u32, cell: usize) -> u32 {
    (episode as u32 + 1) * 100 + base + cell as u32
}

/// The `SP%03d` name a story number carries in either store.
pub fn story_flag(story: u32) -> String {
    format!("SP{story:03}")
}

/// The art variant for an episode and page, as [`crate::ui::menu::Mode::stem`]
/// spells it.
///
/// `FUN_1000d890` builds two different names: episodes 1 and 2 are one page and
/// are named without one, and the rest carry the page as a `-N` suffix.
pub fn variant(episode: usize, page: usize) -> String {
    if episode < 2 {
        format!("{:02}", episode + 1)
    } else {
        format!("{:02}-{}", episode + 1, page + 1)
    }
}

/// Whether a widget can be used, from `FUN_1000e750`.
///
/// `trial` is host `+0x34`, the same answer the Option screen's tabs are gated
/// on: with it set only the first episode is reachable and the chart cannot be
/// paged forward past it.
///
/// A cell is enabled when the **save's** store carries its story point, which
/// is `clickable` here — never when the screen was opened from the title, where
/// `FUN_1000c740` leaves that array as `FUN_1000c850` zeroed it.
pub fn enabled(
    episode: usize,
    page: usize,
    cells: usize,
    trial: bool,
    clickable: &[bool],
    widget: usize,
) -> bool {
    match action(cells, widget) {
        Act::Close => true,
        Act::Episode(tab) => !trial || tab == 0,
        Act::PrevEpisode => episode != 0,
        Act::NextEpisode => !trial && episode != EPISODES - 1,
        Act::PrevPage => page != 0,
        // The first two episodes are one page, and this is the test that says
        // so: `+0x3e8` is zero for them and would make the comparison below
        // nonsense, so the episode is checked instead.
        Act::NextPage => episode >= 2 && page + 1 != page_count(episode),
        Act::Cell(cell) => clickable.get(cell).copied().unwrap_or(false),
        Act::None => false,
    }
}

/// Where the page lands when the episode changes, from `FUN_1000e8b0`.
///
/// Episodes 3 to 6 have four pages and the two before them have one or two, so
/// crossing between the two groups carries the reader to roughly where they
/// were rather than to the front. Nothing clamps the result to the episode
/// being entered, which is the quirk [`page`] absorbs.
pub fn page_for_episode(from: usize, to: usize, page: usize) -> usize {
    if from < 3 && to > 2 {
        if page == 0 {
            1
        } else {
            2
        }
    } else if from >= 3 && to <= 2 {
        if page < 2 {
            0
        } else {
            1
        }
    } else {
        page
    }
}

/// Stepping back an episode with the `<` button, which adjusts the page only
/// when it lands on episode 3.
pub fn step_back(episode: usize, page: usize) -> (usize, usize) {
    let episode = episode.saturating_sub(1);
    let page = if episode == 2 {
        if page < 2 {
            0
        } else {
            1
        }
    } else {
        page
    };
    (episode, page)
}

/// Stepping on an episode with the `>` button, which adjusts the page only
/// when it lands on episode 4.
pub fn step_on(episode: usize, page: usize) -> (usize, usize) {
    let episode = (episode + 1).min(EPISODES - 1);
    let page = if episode == 3 {
        if page == 0 {
            1
        } else {
            2
        }
    } else {
        page
    };
    (episode, page)
}

/// The episode a `ROUTE` is in, from `_GetStory@4`.
///
/// A plain table over the 55 routes, and the answer for a route past the end is
/// the sixth episode once anything has been cleared and **zero** before that —
/// zero being "no episode at all", which [`opened_at`] turns into the first.
pub fn episode_of(route: i32, end_clear: bool) -> usize {
    match route {
        0 => 1,
        1..=3 => 2,
        4..=5 => 3,
        6..=0x12 => 4,
        0x13..=0x1f => 5,
        0x20..=0x36 => 6,
        _ if end_clear => 6,
        _ => 0,
    }
}

/// The page `_GetRouteMapPage@8` opens on for a route, where it is a table.
///
/// Eight of the 55 routes answer this by asking `_CheckScript@8` where the
/// player is within the episode, and **`_CheckScript@8` is not recovered** — it
/// dispatches to one function per route, 55 of them, each reading the position
/// out of the graph. Those eight are `None` here and the screen opens on the
/// first page of the episode instead of on the player's own.
pub fn opening_page(route: i32) -> Option<usize> {
    Some(match route {
        0..=3 | 5 | 0x16 | 0x1b | 0x20 | 0x26 | 0x27 | 0x29 | 0x2a | 0x2e => 0,
        6..=9 | 0xe | 0x17 | 0x1a | 0x1e | 0x1f | 0x22 | 0x2f | 0x30 => 1,
        10..=0xc | 0x12..=0x14 | 0x18 | 0x1c | 0x2b | 0x2c | 0x32 | 0x35 | 0x36 => 2,
        0xd | 0x19 | 0x21 | 0x23..=0x25 | 0x2d | 0x34 => 3,
        // `_CheckScript@8` decides these.
        4 | 0xf | 0x10 | 0x11 | 0x15 | 0x1d | 0x28 | 0x31 | 0x33 => return None,
        _ => 0,
    })
}

/// Where the screen opens, from `FUN_1000e060`.
///
/// Only when it was opened from inside a playthrough: the title-rooted screen
/// keeps the zeroes its constructor left, which is episode 1, page 1.
pub fn opened_at(route: i32, end_clear: bool) -> (usize, usize) {
    let episode = episode_of(route, end_clear).saturating_sub(1);
    let page = opening_page(route).unwrap_or_else(|| {
        log::info!(
            "which page of episode {} route {route} opens on is not recovered",
            episode + 1
        );
        0
    });
    (episode, page)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four bands of per-cell records plus the 33 fixed ones are what a
    /// page's record table holds, so a page whose cell count disagrees with its
    /// table length is a page read wrong. These are the shipped lengths.
    #[test]
    fn every_page_matches_the_length_of_its_record_table() {
        let lengths = [
            [0x31, 0, 0, 0],
            [0x45, 0, 0, 0],
            [0x3d, 0x31, 0, 0],
            [0x41, 0x41, 0x5d, 0x31],
            [0x4d, 0x69, 0x6d, 0x45],
            [0x4d, 0x45, 0x55, 0x51],
        ];
        for (episode, pages) in PAGES.iter().enumerate() {
            for (n, page) in pages.iter().enumerate() {
                assert_eq!(0x21 + 4 * page.cells, lengths[episode][n]);
            }
        }
    }

    /// The story numbers of an episode have to run without a gap or an overlap
    /// across its pages, because the chart is one episode's points split over
    /// pages rather than a fresh numbering per page.
    #[test]
    fn an_episodes_pages_number_its_story_points_consecutively() {
        for pages in PAGES {
            let mut next = 0;
            for page in pages {
                assert_eq!(page.base, next);
                next += page.cells as u32;
            }
        }
    }

    /// Tabbing between the short episodes and the long ones carries the page
    /// across, and the original never clamps it to the episode it lands on.
    #[test]
    fn tabbing_out_of_a_long_episode_can_park_on_a_page_that_is_not_there() {
        assert_eq!(page_for_episode(4, 0, 3), 1);
        assert_eq!(page(0, 1), None);
        // Which is the fallback's whole job.
        assert_eq!(page(0, 0), Some(Page { cells: 4, base: 0 }));
    }

    /// The page buttons are gated by the episode and not by the page count,
    /// which is zero for the two single-page episodes.
    #[test]
    fn the_single_page_episodes_cannot_be_paged() {
        let none: [bool; 0] = [];
        assert!(!enabled(0, 0, 4, false, &none, 10));
        assert!(!enabled(1, 0, 9, false, &none, 10));
        assert!(enabled(2, 0, 7, false, &none, 10));
        assert!(!enabled(2, 1, 4, false, &none, 10));
    }

    /// A cell is pickable only when the save's store carries its story point,
    /// so the chart opened from the title — where that array is all false —
    /// has nothing to pick.
    #[test]
    fn a_cell_is_pickable_only_where_the_run_has_been() {
        let passed = [true, false, true, false];
        assert!(enabled(0, 0, 4, false, &passed, 0xb));
        assert!(!enabled(0, 0, 4, false, &passed, 0xc));
        assert!(enabled(0, 0, 4, false, &passed, 0xd));
        assert!(!enabled(0, 0, 4, false, &[false; 4], 0xb));
    }

    /// The number a cell carries is the one both the chart and the jump use.
    #[test]
    fn a_cell_names_the_story_point_the_jump_asks_for() {
        assert_eq!(story(0, 0, 0), 100);
        assert_eq!(story(3, 0x1f, 3), 434);
        assert_eq!(story_flag(story(3, 0x1f, 3)), "SP434");
        assert_eq!(story_flag(story(2, 7, 3)), "SP310");
    }

    /// Episodes 1 and 2 are named without a page and the rest with one.
    #[test]
    fn only_the_paged_episodes_carry_a_page_in_their_art() {
        assert_eq!(variant(0, 0), "01");
        assert_eq!(variant(1, 0), "02");
        assert_eq!(variant(2, 1), "03-2");
        assert_eq!(variant(5, 3), "06-4");
    }
}
