//! The Option screen's tab pages, on the module that ships one hit map for the
//! whole screen.
//!
//! # The screen splits in two
//!
//! School Days HQ gives each Option tab its own `.CMAP` carrying all of that
//! tab's widgets, and [`crate::ui::options`] reads them straight out of it. The
//! other module on this engine ships `System/Option/OptionBase.cmap` with four
//! regions — the three tab headers and CLOSE — and nothing for the pages that
//! scroll underneath. Those pages are rectangles in a table instead.
//!
//! The split is in the shipped code, not a decision here. `FUN_10009760`, the
//! per-frame input pump, asks `FUN_10010ed0` first, and that is only the hit
//! map: it returns the region number less one, or -1. On a page it always
//! misses, and the miss falls through to the class's own vtable slot `+0x4c`,
//! which for `MENU::ConfigMenu` is `FUN_1000bb10` — a linear scan of the tab's
//! record table returning `index + 4`. So the map answers the frame and the
//! table answers the page, and [`Pages`] is the second half.
//!
//! # One table per tab, indexed the plainest way there is
//!
//! Each tab's rectangles are a run of the usual six-float records, in widget
//! order from [`FIRST`] up, so **record `widget - 4` is widget `widget`**. Both
//! ends of the code agree on that: the draws index it that way and
//! `FUN_1000bb10` returns `index + 4`, and its per-tab record counts are the
//! live-widget counts the gates declare.
//!
//! ```text
//! tab 0  Def     10 records   widgets 4..=13   FUN_10006500   FUN_10008f20
//! tab 1  Sound    7 records   widgets 4..=10   FUN_10006a20   FUN_10008f50
//! tab 2  SomCon  14 records   widgets 4..=17   FUN_100070f0   FUN_10008fc0
//! ```
//!
//! A second run of the same widgets at a different `src_y` follows each — the
//! two alternate runs School Days HQ's Option screen also has, here once per
//! tab.
//!
//! # Finding the tables
//!
//! The Def table is anchored: it begins exactly [`DEF_AFTER_FRAME`] records
//! after the frame's, which [`days_ui::atlas::find`] already places from the
//! four regions of `OptionBase.cmap`. The frame's four widgets, the three tab
//! highlights and the Def page's ten records are one contiguous run.
//!
//! The Sound and SomCon tables are not adjacent to it or to each other, and no
//! hit map covers them, so there is nothing to anchor them against. They are
//! found by their shape instead — see [`days_ui::atlas::table_by_shape`], and
//! note that that search is ours rather than the game's. The shapes used here
//! are [`sound_shape`] and [`somcon_shape`].
//!
//! # Sliders, not levels
//!
//! The Sound tab's widgets 8, 9 and 10 are dragged sliders where School Days
//! HQ has rows of ten cells. Each is two records: the **track** is record
//! `widget - 4`, which is what [`Pages::hit`] catches the pointer with and
//! which nothing draws, and the **knob** is record `widget + 6`, which is the
//! sprite. Only the knob's `x` is live; the drag in `FUN_1000bd20` moves it and
//! clamps it to the track, and the value is where it sits along the travel.
//!
//! `FUN_10008f50` then puts `FUN_1000c280` in front of the hit test's answer,
//! narrowing the live area from the track back down to the knob — so the track
//! catches the pointer and the knob decides whether the widget answers at all.
//! [`Pages::slider_grabbed`] is that second test.
//!
//! # How a page is drawn
//!
//! A page is a layer of its own between the frame's art and the frame's
//! sprites, and `FUN_10006110` is the order: each page's full-screen art, then
//! the contents of the tab showing, then the frame's tab highlight and its
//! hovered widget. [`crate::ui::screen::Page`] is that layer.
//!
//! The three tab draws all put their contents down the same way:
//!
//! ```text
//! the value in force on each row   the row's own record, chosen by the value
//! the widget under the pointer     that widget's record in the second run
//! (Sound only) the three knobs     record `widget + 6`, moved along the track
//! ```
//!
//! So the current-value highlight is not a run of its own here: each row is two
//! records — one per button — and the tab draws whichever one the setting sits
//! on. [`values`] is that choice, and [`Pages::hover`] is the second run.
//!
//! Every page sprite is drawn at `record.x - scroll`, where the scroll is the
//! carousel's, and `FUN_10008830` sets it to `pageWidth * tab` while each page
//! sits at `pageWidth * its own index` — so at rest the two cancel and a
//! record's own coordinates are where it lands. Nothing here models the drag
//! between tabs; `docs/FORMATS.md` has the carousel.

use crate::install::config::{Channel, Config, Flag};
use crate::ui::options::{self, Act, Dir, Display, DisplayRequest, Som, Tab};
use days_ui::atlas::{self, Widget};
use days_ui::cmap::Rect;

/// The first widget of a page. Widgets 0 to 3 are the frame's, and
/// `FUN_1000bb10` returns `index + 4`.
pub const FIRST: usize = 4;

/// Records between the frame table's first record and the Def page's: the four
/// frame widgets and the three tab highlights, which `FUN_100083b0` reads and
/// `FUN_1000b3d0` re-points at `tab + 4`.
pub const DEF_AFTER_FRAME: usize = 7;

/// The first slider on the Sound tab, from `FUN_10009430`: widgets 8, 9 and 10
/// begin a drag of slider `widget - 8`.
pub const FIRST_SLIDER: usize = 8;

/// How many sliders the Sound tab has.
pub const SLIDERS: usize = 3;

/// Which volume each slider carries.
///
/// `FUN_100083b0` works the three knob positions out from `+0xc8`, `+0xcc` and
/// `+0xc4` in that order, and `FUN_100075d0` fills those three from
/// `BgmVolume`, `SeVolume` and `VoiceVolume`.
pub const SLIDER_CHANNELS: [Channel; SLIDERS] = [Channel::Bgm, Channel::Se, Channel::Voice];

/// A slider's knob, as a **record index** rather than a widget: `FUN_1000bd20`
/// reaches the track at record `widget - 4`, which is the ordinary page rule,
/// and the knob at record `widget + 6`, which is not — records 14, 15 and 16
/// for widgets 8, 9 and 10.
const KNOB_RECORD_FROM_WIDGET: usize = 6;

/// How many records each tab's table holds, from `FUN_1000bb10`.
pub fn records(tab: Tab) -> usize {
    match tab {
        Tab::Def => 10,
        Tab::Sound => 7,
        Tab::SomCon => 14,
    }
}

/// Whether a run of records can be the Sound page's.
///
/// Its four toggle widgets are two pairs on one baseline, all the same size,
/// and its three tracks are a column: one `x`, one size, an even pitch, and far
/// wider than they are tall. Nothing else in either module's data is shaped
/// like that.
fn sound_shape(run: &[Widget]) -> bool {
    let (toggles, tracks) = run.split_at(4);
    if !one_size_on_one_line(toggles) {
        return false;
    }
    let first = tracks[0];
    if !tracks.iter().all(|t| {
        (t.dst.x, t.dst.width, t.dst.height) == (first.dst.x, first.dst.width, first.dst.height)
    }) {
        return false;
    }
    let pitch = tracks[1].dst.y.saturating_sub(tracks[0].dst.y);
    pitch > 0
        && tracks.windows(2).all(|w| w[1].dst.y == w[0].dst.y + pitch)
        && first.dst.width >= first.dst.height * 4
}

/// Whether a run of records can be the SomCon page's.
///
/// Its first four are two pairs on their own baselines, and its ten port
/// buttons are one left-to-right strip: one size, one baseline, an even pitch,
/// and — because they are consecutive sprites in the chip sheet — one `src_y`
/// with `src_x` advancing by the button's width and the sheet's one-pixel
/// gutter.
fn somcon_shape(run: &[Widget]) -> bool {
    let (pairs, strip) = run.split_at(4);
    pairs.chunks(2).all(one_size_on_one_line) && button_strip(strip)
}

/// Whether these records are one size on one baseline.
fn one_size_on_one_line(run: &[Widget]) -> bool {
    let first = run[0];
    run.iter().all(|w| {
        (w.dst.y, w.dst.width, w.dst.height) == (first.dst.y, first.dst.width, first.dst.height)
    })
}

/// Whether these records are an evenly spaced row of identical buttons, packed
/// into the chip sheet left to right with a one-pixel gutter.
fn button_strip(run: &[Widget]) -> bool {
    if !one_size_on_one_line(run) || run.iter().any(|w| w.src_y != run[0].src_y) {
        return false;
    }
    let pitch = run[1].dst.x.saturating_sub(run[0].dst.x);
    let gutter = run[0].dst.width + 1;
    pitch > 0
        && run
            .windows(2)
            .all(|w| w[1].dst.x == w[0].dst.x + pitch && w[1].src_x == w[0].src_x + gutter)
}

/// Where each tab's page records begin in the DLL image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pages {
    def: usize,
    sound: usize,
    somcon: usize,
}

impl Pages {
    /// Locates all three tables, given where the frame's table starts.
    ///
    /// `frame` is [`days_ui::atlas::Atlas::offset`] for the screen's own hit
    /// map. Returns `None` when any one of the three cannot be placed
    /// unambiguously — a module with no such pages, or one this search does not
    /// fit, leaves the screen its frame rather than inventing geometry for it.
    pub fn locate(dll: &[u8], frame: usize) -> Option<Pages> {
        let def = frame.checked_add(DEF_AFTER_FRAME * 24)?;
        // The Def table is reached by arithmetic, so check it decodes as far as
        // it should before trusting the anchor it came from.
        atlas::record_at(dll, def, records(Tab::Def) - 1)?;
        let sound = atlas::table_by_shape(dll, records(Tab::Sound), sound_shape)?;
        let somcon = atlas::table_by_shape(dll, records(Tab::SomCon), somcon_shape)?;
        Some(Pages { def, sound, somcon })
    }

    /// Where a tab's records begin.
    pub fn base(&self, tab: Tab) -> usize {
        match tab {
            Tab::Def => self.def,
            Tab::Sound => self.sound,
            Tab::SomCon => self.somcon,
        }
    }

    /// A page widget's rectangle and sprite, or `None` when the widget is not
    /// one of that tab's.
    pub fn widget(&self, dll: &[u8], tab: Tab, widget: usize) -> Option<Widget> {
        let index = widget.checked_sub(FIRST)?;
        (index < records(tab)).then(|| atlas::record_at(dll, self.base(tab), index))?
    }

    /// The widget under a point in layout space, as `FUN_1000bb10` finds it.
    ///
    /// First record containing the point wins — there is no z-order and no best
    /// match — and the test is half-open at the low edge and closed at the high
    /// one, which is the shipped comparison rather than a tidied version of it.
    pub fn hit(&self, dll: &[u8], tab: Tab, x: u32, y: u32) -> Option<usize> {
        let records: Vec<Widget> = self.widgets(dll, tab);
        hit_in(&records, x, y)
    }

    /// A tab's page widgets, in widget order from [`FIRST`].
    pub fn widgets(&self, dll: &[u8], tab: Tab) -> Vec<Widget> {
        (0..records(tab))
            .map_while(|index| atlas::record_at(dll, self.base(tab), index))
            .collect()
    }

    /// The same for the run behind them: each widget's hover art.
    pub fn hovers(&self, dll: &[u8], tab: Tab) -> Vec<Widget> {
        let count = records(tab);
        (count..count * 2)
            .map_while(|index| atlas::record_at(dll, self.base(tab), index))
            .collect()
    }

    /// A Sound slider's track — the full-width record the pointer is caught by.
    pub fn track(&self, dll: &[u8], slider: usize) -> Option<Widget> {
        (slider < SLIDERS).then(|| self.widget(dll, Tab::Sound, FIRST_SLIDER + slider))?
    }

    /// A Sound slider's knob — the sprite that is drawn and dragged.
    pub fn knob(&self, dll: &[u8], slider: usize) -> Option<Widget> {
        let index = (slider < SLIDERS).then(|| FIRST_SLIDER + slider + KNOB_RECORD_FROM_WIDGET)?;
        atlas::record_at(dll, self.sound, index)
    }

    /// Whether the pointer is on slider `slider`'s knob, from `FUN_1000c280`.
    ///
    /// The test is on `x` alone: the track has already decided the pointer is
    /// on that row.
    pub fn slider_grabbed(&self, dll: &[u8], slider: usize, knob_x: f32, x: f32) -> bool {
        self.knob(dll, slider)
            .is_some_and(|knob| grabbed(&knob, knob_x, x))
    }

    /// A widget's hover sprite.
    ///
    /// Each tab's records are followed by a second run of the same rectangles
    /// at a different `src_y` — the art with the pointer on it — and the three
    /// hover draws reach it at one record per widget: `FUN_1000adc0` uses
    /// `widget + 6` on the Def tab's ten, `FUN_1000af90` `widget + 3` on the
    /// Sound tab's seven and `FUN_1000b1d0` `widget + 10` on the SomCon tab's
    /// fourteen. All three are the page rule again with the run's length added.
    ///
    /// A slider's hover art is drawn at the knob's own x rather than the
    /// track's, so `knob_x` moves it; the other widgets ignore it.
    pub fn hover(
        &self,
        dll: &[u8],
        tab: Tab,
        widget: usize,
        knob_x: Option<f32>,
    ) -> Option<Widget> {
        let index = widget.checked_sub(FIRST)?;
        if index >= records(tab) {
            return None;
        }
        let sprite = atlas::record_at(dll, self.base(tab), index + records(tab))?;
        Some(match knob_x {
            Some(x) => at_x(&sprite, x),
            None => sprite,
        })
    }
}

/// How far left of the knob a slider's hover art is drawn.
///
/// `FUN_1000af90` places that art at `knob_x - 1.0` where every other sprite on
/// the page goes at its own record's x. The hover art is 20 wide against the
/// knob's 18, so the pixel centres it on the knob it belongs to.
pub const SLIDER_HOVER_OFFSET: f32 = -1.0;

/// Whether the pointer is on a knob sitting at `knob_x`, from `FUN_1000c280`.
///
/// Closed at both ends, which the record test is not: the shipped comparison is
/// `(knob_x < px) != (knob_x == px)`, and that is `knob_x <= px`.
pub fn grabbed(knob: &Widget, knob_x: f32, x: f32) -> bool {
    knob_x <= x && x <= knob_x + knob.dst.width as f32
}

/// A sprite moved to an x, which is how the Sound tab draws anything that sits
/// on a slider.
pub fn at_x(sprite: &Widget, x: f32) -> Widget {
    Widget {
        dst: Rect {
            x: x.round().max(0.0) as u32,
            ..sprite.dst
        },
        ..*sprite
    }
}

/// A tab's own art, which is not named after the screen's stem.
///
/// A page is a full-screen 800x450 image with the frame showing through it,
/// plus a chip sheet its sprites are cut from. `FUN_10007d10` spells the three
/// backgrounds and `FUN_100081a0` the three sheets. The SomCon page swaps its
/// background — and not its sheet — on the same `UseSOM` member that lights
/// widget 4.
pub fn art(tab: Tab, som: Som) -> (&'static str, &'static str) {
    match tab {
        Tab::Def => (
            "System/Option/Default/Option_Def.png",
            "System/Option/Default/Option_Def_Chip.png",
        ),
        Tab::Sound => (
            "System/Option/Sound/Option_Sound.png",
            "System/Option/Sound/Option_Sound_Chip.png",
        ),
        Tab::SomCon => (
            if som.enabled {
                "System/Option/Somcon/Option_SomCon_Set.png"
            } else {
                "System/Option/Somcon/Option_SomCon.png"
            },
            "System/Option/Somcon/Option_SomCon_Chip.png",
        ),
    }
}

/// The first `Port number` button, and how many there are.
///
/// `FUN_100095c0` takes widgets 8 to 0x11, and `FUN_100070f0` draws the row as
/// records 4 to 13.
pub const FIRST_PORT: usize = 8;
pub const PORT_BUTTONS: usize = 10;

/// The port a `Port number` button asks for, or `None` when it asks for one
/// this engine has not got.
///
/// **The shipped handler stores `widget - 6`**, which is two past the button's
/// own place in the row: `FUN_100095c0` opens that port and keeps it in the
/// member `FUN_100070f0` draws the row's highlight from. The scan in
/// `FUN_10008760` writes the same member with a real 0-based index, so the two
/// writers disagree and the click is the one that is out by two — the first
/// button takes the third port, the first two ports cannot be clicked at all,
/// and the last two buttons ask for ports past the nine there are. That is
/// reproduced here rather than tidied: the highlight sitting two buttons right
/// of the press is what the retail screen shows, and it follows from this one
/// store. The two buttons that run off the end are refused, the way
/// [`crate::ui::options::SOM_PORTS`] refuses the other module's tenth.
pub fn port_of(widget: usize) -> Option<usize> {
    let port = widget.checked_sub(6)?;
    (FIRST_PORT..FIRST_PORT + PORT_BUTTONS)
        .contains(&widget)
        .then_some(port)
        .filter(|port| *port < options::SOM_PORTS)
}

/// Whether a widget can be chosen.
///
/// `FUN_10008e60` answers for the frame and hands the page to `FUN_10008f20`
/// (Def), `FUN_10008f50` (Sound) or `FUN_10008fc0` (SomCon).
///
/// The Sound tab's three sliders are **false here whatever the pointer is
/// doing**: `FUN_10008f50` answers for one only when `FUN_1000c280` says the
/// pointer is on its knob, and that test needs a position this cannot see. See
/// [`Pages::slider_grabbed`], which is the rest of it.
pub fn enabled(tab: Tab, widget: usize, trial: bool, som: Som) -> bool {
    match widget {
        0 | 1 | 3 => true,
        // The trial build has two tabs, so its SomCon header is not live —
        // `FUN_10008830` reads the same host answer to size the carousel.
        2 => !trial,
        _ => match tab {
            Tab::Def => (FIRST..=0xd).contains(&widget),
            Tab::Sound => (FIRST..=7).contains(&widget),
            // The two buttons that take and drop the toy are always live;
            // everything else on the tab needs a port in hand.
            Tab::SomCon => match widget {
                4 | 5 => true,
                6..=0x11 => som.enabled && som.attached,
                _ => false,
            },
        },
    }
}

/// The widget each of a tab's settings currently sits on: the buttons the
/// current-value highlight is drawn over, in the order the tab draws them.
///
/// The highlight is not a run of its own here. Each tab's draw picks one of the
/// row's **own two records** by the value in force — `if (menVoice == 0) rect =
/// &DAT_10054778; else rect = &DAT_10054760;` and so on — so the highlight is
/// the button's own rectangle and the widget is the slot. `FUN_10006500`,
/// `FUN_10006a20` and `FUN_100070f0` are the three.
///
/// The SomCon tab's last two rows are drawn only while a port is in hand, which
/// is the same gate their widgets have.
pub fn values(tab: Tab, config: &Config, display: Display, som: Som) -> Vec<usize> {
    match tab {
        Tab::Def => vec![
            if display.wide { 4 } else { 5 },
            if display.full_screen { 7 } else { 6 },
            if config.flag(Flag::Skip) { 8 } else { 9 },
            if config.flag(Flag::SuperSkip) {
                0xa
            } else {
                0xb
            },
            if config.flag(Flag::TextView) {
                0xc
            } else {
                0xd
            },
        ],
        Tab::Sound => vec![
            if config.flag(Flag::MenVoice) { 4 } else { 5 },
            if config.flag(Flag::Mute) { 6 } else { 7 },
        ],
        Tab::SomCon => {
            let mut out = vec![if som.enabled { 4 } else { 5 }];
            if som.enabled && som.attached {
                out.push(if som.testing { 6 } else { 7 });
                out.push(som.port + FIRST_PORT);
            }
            out
        }
    }
}

/// What activating a widget does, from `FUN_10009060` and the three handlers it
/// dispatches to: `FUN_10009150` (Def), `FUN_10009430` (Sound) and
/// `FUN_100095c0` (SomCon).
///
/// The caller is expected to have checked [`enabled`] first, as every arm of
/// the shipped dispatch does.
pub fn action(tab: Tab, widget: usize, display: Display) -> Act {
    if let Some(next) = Tab::from_widget(widget) {
        return Act::Tab(next);
    }
    if widget == 3 {
        return Act::Close;
    }
    match tab {
        Tab::Def => def_action(widget, display),
        Tab::Sound => sound_action(widget),
        Tab::SomCon => somcon_action(widget),
    }
}

/// `FUN_10009150`.
fn def_action(widget: usize, display: Display) -> Act {
    match widget {
        // The two display rows act only when the mode would really change: the
        // arm for widget 4 runs only while the layout is 4:3, and so on down.
        4 if !display.wide => Act::Display(DisplayRequest::Wide),
        5 if display.wide => Act::Display(DisplayRequest::Normal),
        6 if display.full_screen => Act::Display(DisplayRequest::Windowed),
        7 if !display.full_screen => Act::Display(DisplayRequest::FullScreen),
        8 => Act::SetFlag(Flag::Skip, true),
        9 => Act::SetFlag(Flag::Skip, false),
        0xa => Act::SetFlag(Flag::SuperSkip, true),
        0xb => Act::SetFlag(Flag::SuperSkip, false),
        0xc => Act::SetFlag(Flag::TextView, true),
        0xd => Act::SetFlag(Flag::TextView, false),
        _ => Act::None,
    }
}

/// `FUN_10009430`.
///
/// Widgets 8, 9 and 10 are the sliders, and they activate **nothing**: their
/// arm latches a drag of slider `widget - 8` at `+0x24c` and `+0x170` and runs
/// one step of it, and the value only ever changes from inside `FUN_1000bd20`.
/// So they are `Act::None` here and the press is what starts them — see
/// `Menu::press`.
fn sound_action(widget: usize) -> Act {
    match widget {
        4 => Act::SetFlag(Flag::MenVoice, true),
        5 => Act::SetFlag(Flag::MenVoice, false),
        6 => Act::SetFlag(Flag::Mute, true),
        7 => Act::SetFlag(Flag::Mute, false),
        _ => Act::None,
    }
}

/// `FUN_100095c0`.
fn somcon_action(widget: usize) -> Act {
    match widget {
        4 => Act::SomDetect,
        5 => Act::SomRelease,
        // `FUN_10030930(.., 0x96, ..)` starts the test and `FUN_10030a30`
        // stops it, each only when it is not already in that state.
        6 => Act::SomTest(true),
        7 => Act::SomTest(false),
        _ => match port_of(widget) {
            Some(port) => Act::SomPort(port),
            None => Act::None,
        },
    }
}

/// One step of keyboard navigation on a page module's Option screen.
///
/// Transcriptions of `FUN_100099b0` (Def), `FUN_10009f40` (Sound) and
/// `FUN_1000a330` (SomCon), which the input pump `FUN_10009760` dispatches on
/// the tab showing. Each moves `+0x168` — the cursor, which the hit test
/// writes too — itself, and each is a hand-written table rather than a rule
/// fitted to the layout, so the three disagree with each other in places.
///
/// The four direction members are `+0x60` up, `+0x64` down, `+0x68` left and
/// `+0x6c` right. Host slot `+0xb4` (`FUN_00417030`) fills them from
/// `FUN_00447c60(0..=3)`, whose bindings at `PTR_DAT_004a2850` default to the
/// virtual-key codes `0x26`, `0x28`, `0x25` and `0x27` — `VK_UP`, `VK_DOWN`,
/// `VK_LEFT`, `VK_RIGHT`, in that order. The record tables say the same thing
/// independently: the Def tab's three rows sit at y 162, 269 and 375, and the
/// `+0x60` arm is the one that steps a widget back by four.
pub fn navigate(tab: Tab, current: usize, dir: Dir, trial: bool, som: Som) -> usize {
    let c = current as i32;
    // `+0x16c`, the tab the carousel is resting on.
    let showing = tab.index() as i32;
    let next = match tab {
        Tab::Def => def_navigate(c, dir, trial, showing),
        Tab::Sound => sound_navigate(c, dir, trial, showing),
        Tab::SomCon => som_navigate(c, dir, som, showing),
    };
    next.max(0) as usize
}

/// Whether a keyboard step also opens the tab it landed on.
///
/// All three tables end their sideways arms with the same guard —
/// `if (-1 < c && c < 3 && FUN_10008e60(this, c)) FUN_1000bcb0(this, c)` — and
/// `FUN_1000bcb0` sets `+0x244` and aims the carousel at that tab. So a left or
/// right step onto a tab header opens that tab there and then, with no press.
/// The up and down arms return before the guard and never do.
pub fn opens_tab(dir: Dir, next: usize) -> Option<Tab> {
    match dir {
        Dir::Left | Dir::Right => Tab::from_widget(next),
        Dir::Up | Dir::Down => None,
    }
}

/// `FUN_100099b0`.
///
/// The Def page is three rows — widgets 4 to 7 at y 162, 8 to 0xb at y 269 and
/// 0xc to 0xd at y 375 — so up and down are a step of four and sideways wraps
/// inside the row. The bottom row is two wide, which is why down out of 0xa and
/// 0xb is a step of two rather than four.
fn def_navigate(c: i32, dir: Dir, trial: bool, showing: i32) -> i32 {
    match dir {
        Dir::Up => match c {
            4..=7 => showing,
            8..=0xd => c - 4,
            3 => 0xd,
            _ => 3,
        },
        Dir::Down => match c {
            // Each header drops into the column it sits over, and the third
            // one lands on widget 7 rather than 6.
            0 => 4,
            1 => 5,
            2 => 7,
            4..=9 => c + 4,
            0xa | 0xb => c + 2,
            3 => showing,
            _ => 3,
        },
        Dir::Left => match c {
            4..=7 => {
                if c == 4 {
                    7
                } else {
                    c - 1
                }
            }
            8..=0xb => {
                if c == 8 {
                    0xb
                } else {
                    c - 1
                }
            }
            0xc | 0xd => {
                if c == 0xc {
                    0xd
                } else {
                    c - 1
                }
            }
            0..=2 => options::header_step(c, false, trial),
            _ => 3,
        },
        Dir::Right => match c {
            4..=7 => {
                if c == 7 {
                    4
                } else {
                    c + 1
                }
            }
            8..=0xb => {
                if c == 0xb {
                    8
                } else {
                    c + 1
                }
            }
            0xc | 0xd => {
                if c == 0xd {
                    0xc
                } else {
                    c + 1
                }
            }
            0..=2 => options::header_step(c, true, trial),
            _ => 3,
        },
    }
}

/// `FUN_10009f40`.
///
/// The Sound page is one row of four toggles at y 376 with the three sliders
/// above it at y 177, 219 and 261. The keyboard never reaches a slider: down
/// out of a header lands on widget 4 and up out of the toggle row goes back to
/// the headers, so the only way onto one is the pointer, which writes the same
/// cursor.
fn sound_navigate(c: i32, dir: Dir, trial: bool, showing: i32) -> i32 {
    match dir {
        // **The shipped up arm goes to the tab header from anywhere.** Its
        // test is `if (c < 4 && c > 7)`, which no integer satisfies: at
        // `0x10009f5f` the `JGE` for `c < 4` already jumps into the block that
        // assigns the header, and the `JG` for `c > 7` below it is reached only
        // when `c < 4`, so the block it guards — `c == 3 ? 7 : 3`, at
        // `0x10009f88` — cannot be entered. `FUN_100099b0` has the same shape
        // written `||` (`JL` and `JG` to one block at `0x100099fc`), and with
        // `||` this arm would read the way every other one does: 4 to 7 up to
        // the headers, CLOSE up to 7, a header or a slider up to CLOSE. It is
        // a live difference in the two functions' machine code, not a tidying
        // of one into the other, so it is reproduced.
        Dir::Up => showing,
        Dir::Down => match c {
            3 => showing,
            0..=2 => 4,
            _ => 3,
        },
        Dir::Left => match c {
            0..=2 => options::header_step(c, false, trial),
            4..=7 => {
                if c == 4 {
                    7
                } else {
                    c - 1
                }
            }
            _ => 3,
        },
        Dir::Right => match c {
            0..=2 => options::header_step(c, true, trial),
            // A slider holds the cursor where it is. Left out of one drops it
            // on CLOSE instead; the two arms really are written differently.
            8..=0xa => c,
            4..=7 => {
                if c == 7 {
                    4
                } else {
                    c + 1
                }
            }
            _ => 3,
        },
    }
}

/// `FUN_1000a330`.
///
/// The SomCon page reads bottom-up: the enable row is widgets 4 and 5 at
/// y 327, the test row 6 and 7 at y 245, and the ten `Port number` buttons sit
/// above both at y 200. Sideways never leaves a pair — both arms are the same
/// swap — and the port row has no keyboard step at all, so the pointer is the
/// only way onto it and anything sideways from there lands on CLOSE.
fn som_navigate(c: i32, dir: Dir, som: Som, showing: i32) -> i32 {
    match dir {
        Dir::Up => match c {
            4 | 5 if !som.enabled => showing,
            // Chosen by the test state rather than by which of the pair the
            // cursor was on, and it lands on the button the value is *not*
            // showing on: `SETNZ AL; ADD EAX,0x6` at `0x1000a380`.
            4 | 5 => 6 + i32::from(som.testing),
            6 | 7 if som.enabled => showing,
            // The test row with the toy off holds the cursor where it is: the
            // arm has no else.
            6 | 7 => c,
            3 => 4,
            _ => 3,
        },
        Dir::Down => match c {
            0 | 1 if som.enabled => 6,
            0 | 1 => 4,
            2 if som.enabled => 7,
            2 => 5,
            6 | 7 if som.enabled => 4 + i32::from(c != 6),
            6 | 7 => c,
            3 => showing,
            _ => 3,
        },
        // Off the header row the two sideways arms are the same code: swap
        // within the pair, and drop everything else — CLOSE and the whole port
        // row — on CLOSE. On the header row they cycle opposite ways, and
        // neither asks the trial question the other two tabs ask.
        Dir::Left | Dir::Right => match c {
            0..=2 => match dir {
                Dir::Right if c == 2 => 0,
                Dir::Right => c + 1,
                _ if c == 0 => 2,
                _ => c - 1,
            },
            4 | 5 => 4 + i32::from(c == 4),
            6 | 7 if som.enabled => 6 + i32::from(c == 6),
            6 | 7 => c,
            _ => 3,
        },
    }
}

/// How far a knob may travel: the track less the knob's own width.
fn travel(track: &Widget, knob: &Widget) -> f32 {
    (track.dst.width.saturating_sub(knob.dst.width)) as f32
}

/// A knob position held to its track, from `FUN_1000bd20`.
///
/// The drag clamps `+0x1bc` itself, on every step, before it takes the value
/// from it: low to the track's own x and high to the last x the knob fits at.
/// So a pointer that runs off the end and comes back does not bank the
/// overshoot.
pub fn clamp_knob_x(track: &Widget, knob: &Widget, knob_x: f32) -> f32 {
    let low = track.dst.x as f32;
    knob_x.clamp(low, low + travel(track, knob))
}

/// The value a knob at `knob_x` stands for, from `FUN_1000bd20`.
///
/// Zero to one across the travel, not one of ten levels.
pub fn slider_value(track: &Widget, knob: &Widget, knob_x: f32) -> f32 {
    let span = travel(track, knob);
    if span <= 0.0 {
        return 0.0;
    }
    ((knob_x - track.dst.x as f32) / span).clamp(0.0, 1.0)
}

/// Where the knob sits for a value, which is [`slider_value`] run backwards.
///
/// `FUN_1000bd20` clamps the knob to the track on every drag, so a value
/// outside zero to one cannot place it outside either.
pub fn slider_knob_x(track: &Widget, knob: &Widget, value: f32) -> f32 {
    track.dst.x as f32 + travel(track, knob) * value.clamp(0.0, 1.0)
}

/// The widget under a point, over records already read out of a tab's table.
///
/// `FUN_1000bb10` itself, once the table is in hand: first record containing
/// the point wins, and it answers `index + 4`.
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

    /// A Sound table: four toggles, three tracks, then enough records to reach
    /// the knobs at 14, 15 and 16.
    fn sound_table() -> Vec<u8> {
        let mut dll = Vec::new();
        for x in [78.0, 215.0, 460.0, 597.0] {
            dll.extend(rec([x, 376.0, 127.0, 40.0, 1.0, 1.0]));
        }
        for y in [177.0, 219.0, 261.0] {
            dll.extend(rec([173.0, y, 529.0, 26.0, 0.0, 0.0]));
        }
        // Records 7..13: the alternates and the knob-shaped pair the draw does
        // not read. Only their count matters here.
        for i in 0..7 {
            dll.extend(rec([1.0, 1.0, 2.0, 2.0, i as f32, 9.0]));
        }
        for y in [177.0, 219.0, 261.0] {
            dll.extend(rec([507.0, y, 18.0, 26.0, 513.0, 1.0]));
        }
        dll
    }

    fn pages(base: usize) -> Pages {
        Pages {
            def: base,
            sound: base,
            somcon: base,
        }
    }

    #[test]
    fn the_hit_test_is_half_open_at_the_low_edge_and_closed_at_the_high() {
        // `FUN_1000bb10` compares `rec.x < px <= rec.x + rec.w`, so the pixel a
        // record starts on belongs to whatever is drawn before it and the pixel
        // one past its width belongs to the record. This is the shipped
        // comparison, not a tidied one, and it is what makes widget 4's left
        // edge and widget 5's right edge land where they do.
        let dll = sound_table();
        let p = pages(0);
        assert_eq!(p.hit(&dll, Tab::Sound, 78, 400), None);
        assert_eq!(p.hit(&dll, Tab::Sound, 79, 400), Some(4));
        assert_eq!(p.hit(&dll, Tab::Sound, 78 + 127, 400), Some(4));
        assert_eq!(p.hit(&dll, Tab::Sound, 78 + 128, 400), None);
    }

    #[test]
    fn a_widget_is_its_own_record_plus_four() {
        let dll = sound_table();
        let p = pages(0);
        let track = p.widget(&dll, Tab::Sound, FIRST_SLIDER).unwrap();
        assert_eq!((track.dst.x, track.dst.width), (173, 529));
        // Widget 3 is the frame's and widget 11 is past this tab's seven.
        assert_eq!(p.widget(&dll, Tab::Sound, FIRST - 1), None);
        assert_eq!(
            p.widget(&dll, Tab::Sound, FIRST + records(Tab::Sound)),
            None
        );
    }

    #[test]
    fn a_sliders_knob_is_six_records_past_its_widget_not_four_before_it() {
        // The track and the knob are both a slider's, and they are indexed
        // differently: the track by the page rule, the knob by `widget + 6`.
        // Reading the knob as if it were a page widget lands on the alternates.
        let dll = sound_table();
        let p = pages(0);
        let knob = p.knob(&dll, 0).unwrap();
        assert_eq!((knob.dst.width, knob.dst.height), (18, 26));
    }

    #[test]
    fn the_knob_travels_the_track_less_its_own_width() {
        let dll = sound_table();
        let p = pages(0);
        let (track, knob) = (p.track(&dll, 0).unwrap(), p.knob(&dll, 0).unwrap());
        // 529 wide less an 18-wide knob is 511 of travel from x = 173.
        assert_eq!(slider_knob_x(&track, &knob, 0.0), 173.0);
        assert_eq!(slider_knob_x(&track, &knob, 1.0), 684.0);
        assert_eq!(slider_value(&track, &knob, 684.0), 1.0);
        // `FUN_1000bd20` clamps the knob to the track on every drag, so a knob
        // beyond either end still reads as an end.
        assert_eq!(slider_value(&track, &knob, 9000.0), 1.0);
        assert_eq!(slider_value(&track, &knob, 0.0), 0.0);
    }

    #[test]
    fn a_drag_clamps_the_knob_before_it_takes_the_value_not_after() {
        // `FUN_1000bd20` holds `+0x1bc` to the track on every step and only
        // then divides, so a pointer that runs off the end and comes back
        // starts from the end rather than from where it would have been. A
        // clamp applied to the value instead would bank the overshoot.
        let dll = sound_table();
        let p = pages(0);
        let (track, knob) = (p.track(&dll, 0).unwrap(), p.knob(&dll, 0).unwrap());
        let rest = slider_knob_x(&track, &knob, 0.5);
        let far = clamp_knob_x(&track, &knob, rest + 9000.0);
        assert_eq!(slider_value(&track, &knob, far), 1.0);
        assert_eq!(clamp_knob_x(&track, &knob, far - 100.0), far - 100.0);
        // And the low end is the track's own x, not zero.
        assert_eq!(clamp_knob_x(&track, &knob, -9000.0), track.dst.x as f32);
    }

    #[test]
    fn a_port_button_asks_for_the_port_two_past_its_own_place() {
        // `FUN_100095c0` opens and stores `widget - 6` where the row of ten
        // starts at widget 8, so the first button takes the third port, the
        // first two ports cannot be clicked at all, and the last two buttons
        // ask for ports there are no names for.
        assert_eq!(port_of(FIRST_PORT), Some(2));
        assert_eq!(port_of(FIRST_PORT + 6), Some(8));
        assert_eq!(port_of(FIRST_PORT + 7), None);
        assert_eq!(port_of(FIRST_PORT + PORT_BUTTONS), None);
        // The row's highlight is drawn at `port + 4` records in, which is the
        // widget `port + 8`: the same store, so the lit button sits two right
        // of the pressed one. Bug and all — see `port_of`.
        let som = Som {
            enabled: true,
            attached: true,
            port: 2,
            testing: false,
        };
        let values = values(
            Tab::SomCon,
            &Config::parse_text(""),
            Display::default(),
            som,
        );
        assert!(values.contains(&(FIRST_PORT + 2)));
        // Nothing in hand draws no port highlight at all.
        let values = values_of_somcon_without_a_port();
        assert!(values.iter().all(|w| *w < FIRST_PORT));
    }

    fn values_of_somcon_without_a_port() -> Vec<usize> {
        values(
            Tab::SomCon,
            &Config::parse_text(""),
            Display::default(),
            Som::default(),
        )
    }

    #[test]
    fn a_widgets_hover_art_is_its_own_record_one_run_later() {
        // The Sound tab's seven records are followed by seven more with the
        // hover art's `src_y`, which is what `FUN_1000af90` reaches at
        // `widget + 3`. Reading it as a page widget would land on the tab's
        // own rectangles again.
        let dll = sound_table();
        let p = pages(0);
        let hovers = p.hovers(&dll, Tab::Sound);
        assert_eq!(hovers.len(), records(Tab::Sound));
        assert_eq!(p.widgets(&dll, Tab::Sound)[0].dst.x, 78);
        assert_eq!((hovers[0].src_x, hovers[0].src_y), (0, 9));
        assert_eq!(hovers[6].src_x, 6);
    }

    #[test]
    fn the_sound_tabs_up_goes_to_the_tab_header_from_everywhere() {
        // `FUN_10009f40`'s up arm tests `c < 4 && c > 7`, which nothing
        // satisfies, so the block that would step off CLOSE onto widget 7 is
        // unreachable and every widget goes to the header instead. Its sibling
        // `FUN_100099b0` writes the same test `||` and does step a row, which
        // is what makes this a difference in the two functions rather than a
        // reading of one.
        let som = Som::default();
        for c in [0, 3, 4, 7, 8, 0xa] {
            assert_eq!(navigate(Tab::Sound, c, Dir::Up, false, som), 1);
        }
        assert_eq!(navigate(Tab::Def, 3, Dir::Up, false, som), 0xd);
        assert_eq!(navigate(Tab::Def, 0xc, Dir::Up, false, som), 8);
    }

    #[test]
    fn the_def_tabs_rows_are_four_four_and_two() {
        // Down steps a whole row, which is four widgets — except out of the
        // second row's right half, where the third row is only two wide and the
        // step is two.
        let som = Som::default();
        assert_eq!(navigate(Tab::Def, 4, Dir::Down, false, som), 8);
        assert_eq!(navigate(Tab::Def, 9, Dir::Down, false, som), 0xd);
        assert_eq!(navigate(Tab::Def, 0xa, Dir::Down, false, som), 0xc);
        assert_eq!(navigate(Tab::Def, 0xd, Dir::Down, false, som), 3);
        // The third header drops onto the row's right end rather than its own
        // column's third widget.
        assert_eq!(navigate(Tab::Def, 2, Dir::Down, false, som), 7);
    }

    #[test]
    fn a_sideways_step_opens_the_header_it_lands_on_and_a_vertical_one_never_does() {
        assert_eq!(opens_tab(Dir::Right, 1), Some(Tab::Sound));
        assert_eq!(opens_tab(Dir::Left, 0), Some(Tab::Def));
        // CLOSE is widget 3 and opens nothing, and the vertical arms return
        // before the guard even on a header.
        assert_eq!(opens_tab(Dir::Right, 3), None);
        assert_eq!(opens_tab(Dir::Up, 1), None);
        assert_eq!(opens_tab(Dir::Down, 1), None);
    }

    #[test]
    fn the_somcon_tab_swaps_within_a_pair_sideways_and_leaves_the_ports_alone() {
        let on = Som {
            enabled: true,
            attached: true,
            port: 0,
            testing: false,
        };
        // Both arms are the same swap off the header row.
        for dir in [Dir::Left, Dir::Right] {
            assert_eq!(navigate(Tab::SomCon, 4, dir, false, on), 5);
            assert_eq!(navigate(Tab::SomCon, 5, dir, false, on), 4);
            assert_eq!(navigate(Tab::SomCon, 6, dir, false, on), 7);
            // The port row has no step of its own, so it lands on CLOSE.
            assert_eq!(navigate(Tab::SomCon, FIRST_PORT, dir, false, on), 3);
        }
        // The header row is the one place they differ.
        assert_eq!(navigate(Tab::SomCon, 0, Dir::Left, false, on), 2);
        assert_eq!(navigate(Tab::SomCon, 0, Dir::Right, false, on), 1);
    }

    #[test]
    fn the_somcon_tab_reads_bottom_up_and_skips_the_test_row_with_the_toy_off() {
        // The enable row is at y 327 and the test row above it at y 245, so up
        // out of 4 or 5 is a step onto the test row, not off the page.
        let mut som = Som {
            enabled: true,
            attached: true,
            port: 0,
            testing: false,
        };
        assert_eq!(navigate(Tab::SomCon, 4, Dir::Up, false, som), 6);
        // Chosen by the test state, and it lands on the button the value is
        // not showing on.
        som.testing = true;
        assert_eq!(navigate(Tab::SomCon, 5, Dir::Up, false, som), 7);
        // With the toy off the test row is dead and up goes straight to the
        // header.
        som.enabled = false;
        assert_eq!(navigate(Tab::SomCon, 4, Dir::Up, false, som), 2);
        assert_eq!(navigate(Tab::SomCon, 0, Dir::Down, false, som), 4);
    }

    #[test]
    fn a_slider_is_not_live_until_the_pointer_is_on_its_knob() {
        // `FUN_10008f50` answers for widgets 8 to 10 only through
        // `FUN_1000c280`, so the answer without a pointer has to be no: a
        // caller that took `enabled` alone would light a knob from anywhere
        // along a 529-pixel track.
        assert!(enabled(Tab::Sound, 7, false, Som::default()));
        assert!(!enabled(Tab::Sound, FIRST_SLIDER, false, Som::default()));
    }

    #[test]
    fn the_knob_gate_narrows_the_track_to_the_sprite() {
        // The track catches the pointer anywhere across 529 pixels; the knob
        // decides whether the widget answers at all.
        let dll = sound_table();
        let p = pages(0);
        assert!(p.slider_grabbed(&dll, 0, 400.0, 410.0));
        assert!(!p.slider_grabbed(&dll, 0, 400.0, 419.0));
        // Closed at both ends, which the record test is not.
        assert!(p.slider_grabbed(&dll, 0, 400.0, 400.0));
        assert!(p.slider_grabbed(&dll, 0, 400.0, 418.0));
    }
}
