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
//! Where the player is standing comes from the other DLL. `_CheckScript@8`
//! answers it as a cell widget, `_GetRouteMapPage@8` turns that into the page
//! the chart opens on, and `FUN_1000ce40` and `FUN_1000c3d0` draw the "you are
//! here" marker on it. See [`Standing`].
//!
//! Recovered from `MENU::RouteMap`: `FUN_1000e8b0` (the dispatch),
//! `FUN_1000e750` (enablement), `FUN_1000d890` (the page table and the art),
//! `FUN_1000c740` (the cell states) and `FUN_1000e060` (the open); and from
//! `RouteProcSDHQ.dll`: `_GetStory@4`, `_GetRouteMapPage@8` and
//! `_CheckScript@8` with the 55 functions it dispatches to.

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
pub const FIRST_CELL: usize = 0xb;

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

/// One route's answer to `_CheckScript@8`, from the function that route
/// dispatches to.
struct Position {
    /// The `SCENE` values this route names outright, each with the cell widget
    /// it stands for.
    scenes: &'static [(i32, u32)],
    /// The episode whose story points are walked when `SCENE` is none of them.
    ///
    /// Normally the route's own episode. Two routes name another episode's
    /// scan, and that is what the binary does: `FUN_1001aa20` (ROUTE `0x0b`,
    /// episode 4) and `FUN_10032930` (ROUTE `0x1d`, episode 5) both call
    /// `FUN_10003d80`, which walks episode 3's `SP300 .. SP310`. Their own
    /// `SCENE` arms answer in their own episode's numbering, so for those two
    /// the fallback lands on a cell near the front of the episode whatever the
    /// player has actually passed.
    scan: usize,
}

/// `_CheckScript@8`'s dispatch, one entry per `ROUTE` in order.
///
/// Every one of the 55 functions has the same shape: read `SCENE` out of the
/// save's store, answer with a cell widget for the handful of scenes the route
/// names, and otherwise walk the episode's story points from the last back to
/// the first and answer with the furthest one the save carries.
const POSITIONS: [Position; 55] = [
    // ROUTE 0x00  FUN_1000e000
    Position {
        scenes: &[(0, 0xb), (3, 0xc), (14, 0xd), (20, 0xe)],
        scan: 1,
    },
    // ROUTE 0x01  FUN_100109f0
    Position {
        scenes: &[
            (0, 0xb),
            (36, 0xd),
            (53, 0xc),
            (62, 0xf),
            (77, 0x10),
            (108, 0xe),
        ],
        scan: 2,
    },
    // ROUTE 0x02  FUN_10011fa0
    Position {
        scenes: &[(2, 0x11), (35, 0x13)],
        scan: 2,
    },
    // ROUTE 0x03  FUN_100128e0
    Position {
        scenes: &[(19, 0x12)],
        scan: 2,
    },
    // ROUTE 0x04  FUN_10014d10
    Position {
        scenes: &[(0, 0xc), (41, 0x15), (45, 0x12), (57, 0x13), (90, 0x14)],
        scan: 3,
    },
    // ROUTE 0x05  FUN_100175b0
    Position {
        scenes: &[
            (0, 0xb),
            (19, 0xd),
            (39, 0xf),
            (51, 0xe),
            (87, 0x11),
            (90, 0x10),
        ],
        scan: 3,
    },
    // ROUTE 0x06  FUN_10018500
    Position {
        scenes: &[(0, 0x17)],
        scan: 4,
    },
    // ROUTE 0x07  FUN_100196c0
    Position {
        scenes: &[(1, 0x18), (34, 0x19)],
        scan: 4,
    },
    // ROUTE 0x08  FUN_10019d70
    Position {
        scenes: &[(3, 0x1a)],
        scan: 4,
    },
    // ROUTE 0x09  FUN_1001a150
    Position {
        scenes: &[],
        scan: 4,
    },
    // ROUTE 0x0a  FUN_1001a570
    Position {
        scenes: &[(2, 0x26)],
        scan: 4,
    },
    // ROUTE 0x0b  FUN_1001aa20  // episode 4, scanned by episode 3's
    Position {
        scenes: &[(3, 0x23)],
        scan: 3,
    },
    // ROUTE 0x0c  FUN_1001b250
    Position {
        scenes: &[(4, 0x28)],
        scan: 4,
    },
    // ROUTE 0x0d  FUN_1001c4e0
    Position {
        scenes: &[(21, 0x2c), (44, 0x2d)],
        scan: 4,
    },
    // ROUTE 0x0e  FUN_1001e780
    Position {
        scenes: &[(0, 0x14), (22, 0x16), (61, 0x15)],
        scan: 4,
    },
    // ROUTE 0x0f  FUN_100217f0
    Position {
        scenes: &[
            (0, 0x1c),
            (24, 0x2a),
            (53, 0x29),
            (70, 0x2b),
            (85, 0x27),
            (108, 0x27),
        ],
        scan: 4,
    },
    // ROUTE 0x10  FUN_10022080
    Position {
        scenes: &[],
        scan: 4,
    },
    // ROUTE 0x11  FUN_10023cb0
    Position {
        scenes: &[
            (0, 0x13),
            (10, 0xf),
            (16, 0x10),
            (19, 0xb),
            (36, 0xd),
            (51, 0xc),
            (55, 0xe),
            (61, 0x12),
            (67, 0x11),
        ],
        scan: 4,
    },
    // ROUTE 0x12  FUN_10026070
    Position {
        scenes: &[
            (0, 0x1b),
            (8, 0x24),
            (15, 0x1e),
            (21, 0x1d),
            (28, 0x1f),
            (44, 0x21),
            (45, 0x22),
            (48, 0x20),
            (68, 0x25),
        ],
        scan: 4,
    },
    // ROUTE 0x13  FUN_10026ef0
    Position {
        scenes: &[(0, 0x2f), (4, 0x32), (11, 0x38), (12, 0x31)],
        scan: 5,
    },
    // ROUTE 0x14  FUN_100277a0
    Position {
        scenes: &[(4, 0x2e), (5, 0x37)],
        scan: 5,
    },
    // ROUTE 0x15  FUN_10027d20
    Position {
        scenes: &[],
        scan: 5,
    },
    // ROUTE 0x16  FUN_10029200
    Position {
        scenes: &[
            (0, 0xb),
            (10, 0x10),
            (19, 0xd),
            (28, 0xe),
            (30, 0xf),
            (43, 0x11),
            (51, 0x12),
        ],
        scan: 5,
    },
    // ROUTE 0x17  FUN_1002a5c0
    Position {
        scenes: &[
            (0, 0x17),
            (1, 0x19),
            (5, 0x1b),
            (24, 0x24),
            (25, 0x23),
            (36, 0x21),
        ],
        scan: 5,
    },
    // ROUTE 0x18  FUN_1002bc70
    Position {
        scenes: &[
            (0, 0x28),
            (2, 0x2c),
            (15, 0x2d),
            (26, 0x35),
            (27, 0x34),
            (40, 0x30),
            (42, 0x33),
            (48, 0x36),
        ],
        scan: 5,
    },
    // ROUTE 0x19  FUN_1002cd50
    Position {
        scenes: &[(0, 0x3c), (1, 0x3d), (23, 0x3f), (24, 0x43), (26, 0x40)],
        scan: 5,
    },
    // ROUTE 0x1a  FUN_1002f250
    Position {
        scenes: &[
            (0, 0x16),
            (14, 0x18),
            (22, 0x1a),
            (26, 0x1d),
            (36, 0x1c),
            (40, 0x22),
            (46, 0x1e),
            (68, 0x1f),
            (78, 0x20),
            (90, 0x26),
            (94, 0x27),
            (98, 0x25),
        ],
        scan: 5,
    },
    // ROUTE 0x1b  FUN_10030a60
    Position {
        scenes: &[(0, 0xc), (23, 0x14), (43, 0x13), (44, 0x15)],
        scan: 5,
    },
    // ROUTE 0x1c  FUN_100316a0
    Position {
        scenes: &[(0, 0x2a), (16, 0x3a)],
        scan: 5,
    },
    // ROUTE 0x1d  FUN_10032930  // episode 5, scanned by episode 3's
    Position {
        scenes: &[(0, 0x3b), (3, 0x3e), (34, 0x42), (35, 0x39), (36, 0x41)],
        scan: 3,
    },
    // ROUTE 0x1e  FUN_10032f00
    Position {
        scenes: &[(0, 0x29)],
        scan: 5,
    },
    // ROUTE 0x1f  FUN_10033300
    Position {
        scenes: &[(0, 0x2b)],
        scan: 5,
    },
    // ROUTE 0x20  FUN_10033c70
    Position {
        scenes: &[(0, 0xd), (5, 0xf), (14, 0x12), (15, 0x13)],
        scan: 6,
    },
    // ROUTE 0x21  FUN_100359a0
    Position {
        scenes: &[
            (0, 0x2c),
            (6, 0x35),
            (8, 0x36),
            (17, 0x34),
            (18, 0x2f),
            (58, 0x32),
        ],
        scan: 6,
    },
    // ROUTE 0x22  FUN_10036150
    Position {
        scenes: &[],
        scan: 6,
    },
    // ROUTE 0x23  FUN_10036450
    Position {
        scenes: &[],
        scan: 6,
    },
    // ROUTE 0x24  FUN_10036750
    Position {
        scenes: &[],
        scan: 6,
    },
    // ROUTE 0x25  FUN_10036a50
    Position {
        scenes: &[],
        scan: 6,
    },
    // ROUTE 0x26  FUN_10036e80
    Position {
        scenes: &[],
        scan: 6,
    },
    // ROUTE 0x27  FUN_10037420
    Position {
        scenes: &[(5, 0x11), (6, 0x10)],
        scan: 6,
    },
    // ROUTE 0x28  FUN_10037ae0
    Position {
        scenes: &[(9, 0x2b)],
        scan: 6,
    },
    // ROUTE 0x29  FUN_10038b10
    Position {
        scenes: &[(39, 0x14), (45, 0x15)],
        scan: 6,
    },
    // ROUTE 0x2a  FUN_10039a70
    Position {
        scenes: &[(0, 0xb)],
        scan: 6,
    },
    // ROUTE 0x2b  FUN_1003acb0
    Position {
        scenes: &[(0, 0x1f), (47, 0x26)],
        scan: 6,
    },
    // ROUTE 0x2c  FUN_1003b6a0
    Position {
        scenes: &[(0, 0x22), (7, 0x25), (12, 0x28), (13, 0x27)],
        scan: 6,
    },
    // ROUTE 0x2d  FUN_1003c950
    Position {
        scenes: &[(0, 0x2e), (24, 0x31)],
        scan: 6,
    },
    // ROUTE 0x2e  FUN_1003d440
    Position {
        scenes: &[(0, 0xc)],
        scan: 6,
    },
    // ROUTE 0x2f  FUN_1003e620
    Position {
        scenes: &[(0, 0x17), (47, 0x1e)],
        scan: 6,
    },
    // ROUTE 0x30  FUN_1003f340
    Position {
        scenes: &[(0, 0x16), (12, 0x1d), (22, 0x1c)],
        scan: 6,
    },
    // ROUTE 0x31  FUN_10040110
    Position {
        scenes: &[(0, 0x20), (13, 0x37), (28, 0x2a)],
        scan: 6,
    },
    // ROUTE 0x32  FUN_10041280
    Position {
        scenes: &[(0, 0x23), (46, 0x29)],
        scan: 6,
    },
    // ROUTE 0x33  FUN_10041fe0
    Position {
        scenes: &[(0, 0xe), (1, 0x1a), (8, 0x19), (15, 0x18), (22, 0x1b)],
        scan: 6,
    },
    // ROUTE 0x34  FUN_100432e0
    Position {
        scenes: &[(0, 0x2d), (12, 0x30), (45, 0x33)],
        scan: 6,
    },
    // ROUTE 0x35  FUN_100438d0
    Position {
        scenes: &[(0, 0x24)],
        scan: 6,
    },
    // ROUTE 0x36  FUN_10043c90
    Position {
        scenes: &[(0, 0x21)],
        scan: 6,
    },
];

/// How many story points an episode charts across all of its pages.
///
/// This is the length of the scan each of `_CheckScript@8`'s six fallbacks
/// does: `FUN_10003ba0` walks `SP100 .. SP103`, `FUN_10003c40` `SP200 ..
/// SP208`, `FUN_10003d80` `SP300 .. SP310`, `FUN_10003f10` `SP400 .. SP434`,
/// `FUN_100043d0` `SP500 .. SP556` and `FUN_10004b70` `SP600 .. SP644` — 4, 9,
/// 11, 35, 57 and 45 names. [`PAGES`], recovered from a different function in
/// the other DLL, sums to the same six numbers episode by episode.
fn charted_cells(episode: usize) -> usize {
    match PAGES.get(episode) {
        Some(pages) => pages.iter().map(|page| page.cells).sum(),
        None => 0,
    }
}

/// What `RouteProcSDHQ.dll` reads out of the stores to place the player.
///
/// `_CheckScript@8` and `_GetRouteMapPage@8` both start by asking the host for
/// the save's `ROUTE`, and the first also wants `SCENE` and a flag lookup in
/// the save's own store — host `+0x10`, the same half of `FUN_1000c740` that
/// decides which cells can be picked. Gathering the three here keeps the two
/// recovered functions reading like the originals.
pub struct Standing<'a> {
    /// `ROUTE` in the save's store.
    pub route: i32,
    /// `SCENE` in the save's store.
    pub scene: i32,
    /// `EndClear` in the save's store, which is all `_GetStory@4` has to go on
    /// for a `ROUTE` past the end of its table.
    pub end_clear: bool,
    /// Whether the save's store carries a story point, by its number.
    pub passed: &'a dyn Fn(u32) -> bool,
}

impl Standing<'_> {
    /// Where the player stands in an episode, from `_CheckScript@8`.
    ///
    /// The answer is a **cell widget** — `0xb` plus the story point's index
    /// within the episode, the same index a page's `base + cell` makes — and
    /// **zero** for "not in this episode at all", which is what the function
    /// answers whenever the episode asked about is not the one `_GetStory@4`
    /// puts the route in.
    ///
    /// The zero is not a cell: widgets `0 ..= 0xa` are the tabs and the arrows,
    /// so every caller reads a value below `0xb` as no answer.
    pub fn cell_widget(&self, episode: usize) -> u32 {
        if episode_of(self.route, self.end_clear) != episode + 1 {
            return 0;
        }
        let Ok(route) = usize::try_from(self.route) else {
            return 0;
        };
        let Some(position) = POSITIONS.get(route) else {
            return 0;
        };
        if let Some((_, widget)) = position.scenes.iter().find(|(s, _)| *s == self.scene) {
            return *widget;
        }
        let first = position.scan as u32 * 100;
        (0..charted_cells(position.scan.saturating_sub(1)) as u32)
            .rev()
            .find(|i| (self.passed)(first + i))
            .map_or(0, |i| FIRST_CELL as u32 + i)
    }

    /// The page of an episode the chart opens on, from `_GetRouteMapPage@8`.
    ///
    /// Mostly a plain table over `ROUTE`. Nine routes ask [`cell_widget`] where
    /// the player is instead and pick the page from that, and those arms are
    /// transcribed as the switch has them rather than worked back out of the
    /// page table — two of route `0x0f`'s comparisons and one of route `0x1d`'s
    /// name a page that does not hold the cell they test for, and one of route
    /// `0x1d`'s cannot come up at all because that route's scan is episode 3's.
    ///
    /// [`cell_widget`]: Standing::cell_widget
    pub fn page(&self, episode: usize) -> usize {
        match self.route {
            0..=3 | 5 | 0x16 | 0x1b | 0x20 | 0x26 | 0x27 | 0x29 | 0x2a | 0x2e => 0,
            6..=9 | 0xe | 0x17 | 0x1a | 0x1e | 0x1f | 0x22 | 0x2f | 0x30 => 1,
            10..=0xc | 0x12..=0x14 | 0x18 | 0x1c | 0x2b | 0x2c | 0x32 | 0x35 | 0x36 => 2,
            0xd | 0x19 | 0x21 | 0x23..=0x25 | 0x2d | 0x34 => 3,
            4 => usize::from(self.cell_widget(episode) != 0xc),
            0xf => match self.cell_widget(episode) {
                0x1c | 0x27 | 0x2b => 2,
                _ => 3,
            },
            0x10 | 0x11 => usize::from(self.cell_widget(episode) == 0x13),
            0x15 => match self.cell_widget(episode) {
                0x29 => 2,
                _ => 3,
            },
            0x1d => match self.cell_widget(episode) {
                0x2b | 0x39 => 2,
                _ => 3,
            },
            0x28 => match self.cell_widget(episode) {
                0xc | 0x14 => 0,
                0x17 => 1,
                0x31 => 3,
                _ => 2,
            },
            0x31 => match self.cell_widget(episode) {
                0x37 => 3,
                _ => 2,
            },
            0x33 => usize::from(self.cell_widget(episode) != 0xe),
            _ => 0,
        }
    }

    /// Where the screen opens, from `FUN_1000e060`.
    ///
    /// Only when it was opened from inside a playthrough: the title-rooted
    /// screen keeps the zeroes its constructor left, which is episode 1, page 1.
    pub fn opened_at(&self) -> (usize, usize) {
        let episode = episode_of(self.route, self.end_clear).saturating_sub(1);
        (episode, self.page(episode))
    }

    /// The cell the "you are here" marker sits on, if the page showing has it.
    ///
    /// `FUN_1000d890` holds the answer in `+0x3e4`, which is
    /// `_CheckScript@8(episode)` less the page's base — the cell widget again,
    /// once the base is out of it — and recomputes it on every page and episode
    /// change. `FUN_1000ce40`
    /// cuts the marker's sprite and raises `+0x4e4` only when that is `0xb` or
    /// more **and** the page showing is the one `_GetRouteMapPage@8` names, and
    /// `FUN_1000c3d0` draws it over the cell whose widget matches. So the
    /// marker appears on one page of one episode — the player's own — and
    /// nowhere else on the chart.
    ///
    /// Both of those live behind the module's `+0x4ec`, so the chart opened
    /// from the title has no marker: see [`crate::ui::menu::Menu::pickable`].
    pub fn marker(&self, episode: usize, page: usize, of: Page) -> Option<usize> {
        if self.page(episode) != page {
            return None;
        }
        let cell = self.cell_widget(episode).checked_sub(of.base)?;
        let cell = usize::try_from(cell.checked_sub(FIRST_CELL as u32)?).ok()?;
        (cell < of.cells).then_some(cell)
    }
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

    /// A save that carries every story point up to `reached` and none after it.
    fn reached(reached: u32) -> impl Fn(u32) -> bool {
        move |story| story <= reached
    }

    /// `_CheckScript@8`'s six fallbacks walk one `SP%03d` per cell of their
    /// episode, and [`PAGES`] — recovered from the other DLL — has to agree
    /// with them or one of the two tables is wrong.
    #[test]
    fn every_episodes_pages_hold_as_many_cells_as_its_scan_has_names() {
        assert_eq!(
            (0..EPISODES).map(charted_cells).collect::<Vec<_>>(),
            [4, 9, 11, 35, 57, 45]
        );
    }

    /// A cell widget is `0xb` plus an index into the episode, so no route may
    /// name one past the end of its own episode's chart — the answer is read
    /// against a page's `base + cell`.
    #[test]
    fn no_route_names_a_cell_its_episode_does_not_have() {
        for (route, position) in POSITIONS.iter().enumerate() {
            let episode = episode_of(route as i32, false);
            let last = FIRST_CELL as u32 + charted_cells(episode - 1) as u32 - 1;
            for (scene, widget) in position.scenes {
                assert!(
                    (FIRST_CELL as u32..=last).contains(widget),
                    "ROUTE {route:#x} SCENE {scene} answers {widget:#x}, past {last:#x}"
                );
            }
        }
    }

    /// `_CheckScript@8` speaks only for the episode `_GetStory@4` puts the
    /// route in, and answers zero — which is a tab, not a cell — for the rest.
    #[test]
    fn a_route_places_the_player_in_its_own_episode_only() {
        let passed = reached(640);
        let standing = Standing {
            route: 0x34,
            scene: 45,
            end_clear: false,
            passed: &passed,
        };
        assert_eq!(standing.cell_widget(5), 0x33);
        for episode in 0..5 {
            assert_eq!(standing.cell_widget(episode), 0);
        }
    }

    /// The fallback answers with the furthest story point the save carries,
    /// walking back from the end of the episode — and with nothing at all for a
    /// save that carries none of them.
    #[test]
    fn an_unnamed_scene_falls_back_to_the_furthest_point_the_save_reached() {
        // ROUTE 0x23, episode 6, names no scene at all: pure scan.
        let passed = reached(612);
        let standing = Standing {
            route: 0x23,
            scene: 7,
            end_clear: false,
            passed: &passed,
        };
        assert_eq!(standing.cell_widget(5), FIRST_CELL as u32 + 12);
        let none = |_: u32| false;
        let standing = Standing {
            route: 0x23,
            scene: 7,
            end_clear: false,
            passed: &none,
        };
        assert_eq!(standing.cell_widget(5), 0);
    }

    /// The marker is on the page `_GetRouteMapPage@8` names and on no other,
    /// which is the test `FUN_1000ce40` makes before it cuts the sprite.
    #[test]
    fn the_marker_sits_on_one_page_of_one_episode() {
        // ROUTE 0x31 SCENE 13 answers 0x37, and that route's arm sends 0x37 to
        // the fourth page of episode 6 — cells 0x21 ..= 0x2c.
        let passed = reached(644);
        let standing = Standing {
            route: 0x31,
            scene: 13,
            end_clear: false,
            passed: &passed,
        };
        assert_eq!(standing.page(5), 3);
        assert_eq!(standing.marker(5, 3, page(5, 3).unwrap()), Some(11));
        assert_eq!(standing.marker(5, 2, page(5, 2).unwrap()), None);
        assert_eq!(standing.marker(4, 3, page(4, 3).unwrap()), None);
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
