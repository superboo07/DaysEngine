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
use crate::ui::options::{self, Act, Display, DisplayRequest, Som, Tab};
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
/// Widgets 8, 9 and 10 begin a drag of slider `widget - 8` and are **not
/// wired**: this title keeps its three volumes as floats — see [`volume`] — and
/// what its executable makes of one is not recovered, so there is nothing to
/// write a dragged value into. The sliders draw, hover and read back; they do
/// not yet move.
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

/// The volume a slider shows, as a fraction of its travel.
///
/// `FUN_100075d0` reads `BgmVolume`, `SeVolume` and `VoiceVolume` through the
/// settings object's float getter, defaulting to **0.5**, and clamps anything
/// above 1.0 back to 1.0 — so this title's volumes are not the 0 to 10 levels
/// [`crate::install::config::Config::volume`] reads for the other one.
///
/// **What the engine does with the number is not recovered.** This is what the
/// screen draws the knob from, and nothing else reads it yet.
pub fn volume(config: &Config, channel: Channel) -> f32 {
    config.r4_or(channel.key(), DEFAULT_VOLUME).min(1.0)
}

/// The volume a missing key stands for, from `FUN_100075d0`.
pub const DEFAULT_VOLUME: f32 = 0.5;

/// How far a knob may travel: the track less the knob's own width.
fn travel(track: &Widget, knob: &Widget) -> f32 {
    (track.dst.width.saturating_sub(knob.dst.width)) as f32
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
        .position(|w| contains(&w.dst, x, y))
        .map(|index| index + FIRST)
}

/// The shipped containment test: `rec.x < x <= rec.x + rec.w`, and the same in
/// `y`.
fn contains(rect: &Rect, x: u32, y: u32) -> bool {
    x > rect.x && x <= rect.x + rect.width && y > rect.y && y <= rect.y + rect.height
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
