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

use crate::ui::options::Tab;
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
        (0..records(tab))
            .find(|index| {
                atlas::record_at(dll, self.base(tab), *index)
                    .is_some_and(|w| contains(&w.dst, x, y))
            })
            .map(|index| index + FIRST)
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
            .is_some_and(|k| knob_x <= x && x <= knob_x + k.dst.width as f32)
    }
}

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
    fn the_knob_gate_narrows_the_track_to_the_sprite() {
        // The track catches the pointer anywhere across 529 pixels; the knob
        // decides whether the widget answers at all.
        let dll = sound_table();
        let p = pages(0);
        assert!(p.slider_grabbed(&dll, 0, 400.0, 410.0));
        assert!(!p.slider_grabbed(&dll, 0, 400.0, 419.0));
    }
}
