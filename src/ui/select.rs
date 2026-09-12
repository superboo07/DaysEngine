//! The choice box: `[SetSELECT]`, and what the player does with it.
//!
//! # It is a window on the timeline, not a stop
//!
//! `[SetSELECT]` is an ordinary statement with a start and an end timecode, and
//! nothing about it pauses the script. `FUN_00431740` — the executable's
//! per-frame playback tick — raises the box the first frame at or past the
//! start, polls it every frame while `frame + 1 < end`, and once the window is
//! spent decides without the player. So a choice the player ignores still
//! resolves, on time, and the movie under it never stops.
//!
//! This module is that whole flow: [`Choice`] holds the window and the decision,
//! [`Choice::tick`] is `FUN_00431740`'s middle, and [`Select`] is the hit map
//! and the label layout underneath it.
//!
//! # The three ways it is laid out
//!
//! The box has **no base art and no chip sheet at all** — `System/Select/`
//! ships six `.CMAP`s and nothing else. The labels are drawn with the engine's
//! own font over whatever is already on screen, and the `.CMAP` is only there to
//! say where a click lands.
//!
//! Which map, from `FUN_0044d890` and `FUN_0044da50`:
//!
//! ```text
//! [Select1] / [Select2]   the FILMENGINE.INI paths, both naming a _Full map
//! _Full -> _Note          FUN_0040f0d0, the display-size global
//! ..._H                   only for two choices, and only when both
//!                         [SelectType] and [UseEnglish] are non-zero
//! ```
//!
//! and only two of the four UI sizes are shipped, 1024x576 and 1280x720. At the
//! other two the engine has no map, which is exactly why `FUN_0044d450` has a
//! second path: it uses the map only when two display globals both read 1, and
//! otherwise splits the screen in normalised coordinates — see [`Select::hit`].
//!
//! # Whose answer it is
//!
//! Three things can answer instead of the player, and all three are in the tick:
//! the auto flag turns a timeout into a *random* pick, a replay overrides the
//! pick with what was recorded, and the host's `+0x98` member replaces it
//! outright. See [`Choice::tick`].

use crate::install::config::{Config, Flag};
use crate::install::ini::Ini;
use crate::install::vfs::Vfs;
use crate::ui::menu::SystemSe;
use crate::ui::screen::{Error, Resolution};
use days_script::Frame;
use days_ui::cmap::Cmap;

/// How the two boxes are arranged, and so which axis a click is split on.
///
/// Named for the geometry in `FUN_0044d450`, not for the `_H` in the filename:
/// the default path splits the screen on **x** and the `_H` path splits it on
/// **y**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Two boxes side by side; the split is at x = 0.5.
    Sideways,
    /// Two boxes one above the other; the split is at y = 0.5. Selected when
    /// `[SelectType]` and `[UseEnglish]` are both non-zero, which is what the
    /// shipped English install has.
    Stacked,
}

impl Layout {
    /// From `[SelectType]` and `[UseEnglish]`.
    ///
    /// `FUN_0044e2c0` returns the member `[SelectType]` was read into and host
    /// slot `+0x5c` the one `[UseEnglish]` was read into; both loaders and the
    /// hit test ask the same pair of questions in the same order.
    pub fn from_ini(film: &Ini) -> Layout {
        let select_type = film.get_bool("SelectType").unwrap_or(false);
        let english = film.get_bool("UseEnglish").unwrap_or(false);
        if select_type && english {
            Layout::Stacked
        } else {
            Layout::Sideways
        }
    }

    /// The `_H` suffix this layout appends to the two-choice map's stem.
    fn suffix(self) -> &'static str {
        match self {
            Layout::Sideways => "",
            Layout::Stacked => "_H",
        }
    }
}

/// The longest line a label is broken into, in characters.
///
/// `FUN_0044ca10` sets this three ways: 11 by default, and with `[UseEnglish]`
/// set either 33 — when `[SelectType]` is zero and this is a two-choice box —
/// or 66. Only the English path word-wraps at all; the default path takes the
/// limit and leaves the text alone.
pub fn wrap_limit(choices: usize, select_type: bool, english: bool) -> usize {
    if !english {
        return 11;
    }
    if !select_type && choices != 1 {
        33
    } else {
        66
    }
}

/// The per-character advance the layout budgets against [`wrap_limit`].
///
/// 36 by default and 16 with `[UseEnglish]`, from the same function. The line
/// height is that number over a constant, so the two scale together.
pub fn advance(english: bool) -> u32 {
    if english {
        16
    } else {
        36
    }
}

/// Where each label is anchored, in the layout's own units.
///
/// `FUN_0044d890` and `FUN_0044da50` multiply the object's scale member by one
/// of these and hand it to `FUN_0044ca10`, which treats it as **x** in the
/// sideways layout and as **y** in the stacked one, pinning x at `533.4` there.
///
/// `FUN_0044ca10` ends by setting each line's **source** rectangle — sprite slot
/// `+0x1c`, where in the shared text texture the label was drawn. The
/// **destination** is `FUN_0044ced0`, slot `+0xc`:
///
/// ```text
/// x = block->0x18[n] * scale + block->0x10
/// y = n * 48.0 * scale + 568.0 * scale + base
/// w = scale * (533.4, or 1066.8 in the stacked layout)
/// h = scale * 48.0
/// ```
///
/// with `base` centring the block vertically on the anchor —
/// `anchor - lines * 48.0 * scale / 2.0` in the stacked English case — and
/// `scale` the same 0.75/0.96/1.2 ladder [`crate::playback::text::Geometry`]
/// carries.
///
/// **This engine does not place labels by that formula yet.** It centres each
/// one in the box the shipped `.CMAP` gives, which is exact data and lines up
/// with the hit testing, but it is not the original's arithmetic.
pub const ANCHOR_ONE: f64 = 533.4;
pub const ANCHOR_TWO: [f64; 2] = [266.7, 800.0];
pub const ANCHOR_ONE_STACKED: f64 = -268.0;
pub const ANCHOR_TWO_STACKED: [f64; 2] = [-418.7, -118.0];

/// The choice box's hit map at one resolution.
pub struct Select {
    /// Path of the map actually loaded, for logging and for `days select`.
    pub path: String,
    pub layout: Layout,
    pub choices: usize,
    /// `None` when the install ships no map for this resolution, which is the
    /// normal case at 800x450 and 800x600 — [`Select::hit`] then falls back the
    /// way `FUN_0044d450` does.
    map: Option<Cmap>,
}

impl Select {
    /// Loads the map for a box with `choices` labels.
    ///
    /// The stem comes out of `FILMENGINE.INI` rather than being spelled here,
    /// because that is where the executable gets it: `FUN_00422170` reads
    /// `[Select1]` and `[Select2]` and hands both to `FUN_0044e100`, which just
    /// stores them. Both shipped values name the `_Full` map and the loaders
    /// rewrite the suffix, so this does the same rewrite on the INI's own value.
    pub fn load(
        vfs: &Vfs,
        film: &Ini,
        choices: usize,
        resolution: Resolution,
    ) -> Result<Select, Error> {
        let layout = Layout::from_ini(film);
        let key = if choices >= 2 { "Select2" } else { "Select1" };
        let stem = film
            .get(key)
            .ok_or_else(|| Error::MissingAsset(format!("FILMENGINE.INI [{key}]")))?;
        let path = rewrite(
            stem,
            resolution,
            if choices >= 2 {
                layout
            } else {
                Layout::Sideways
            },
        );

        let map = match vfs.read_path(&path) {
            Ok(bytes) => Some(Cmap::parse(&bytes)?),
            Err(_) => {
                log::info!("{path} is not in the packs; the choice box will split the screen");
                None
            }
        };
        Ok(Select {
            path,
            layout,
            choices: choices.clamp(1, 2),
            map,
        })
    }

    /// The choice under a pointer position given in **normalised** screen
    /// coordinates, `0.0..1.0` on both axes.
    ///
    /// Normalised is what the host reports: slot `+0x144` hands back a pair of
    /// floats and `FUN_0044d450` compares them against `0.0`, `0.5` and `1.0`,
    /// which are the three doubles at `0x004d13d8`, `0x004d4fb0` and
    /// `0x004d13d0`. The same pair goes to the `.CMAP` lookup, which indexes
    /// the map in whole pixels (`FUN_00465bc0`), so something scales them up in
    /// between; `FUN_00465c40` is where that happens and its decompilation
    /// loses the x87 arguments, so **the scaling step itself is not
    /// recovered**. Multiplying by the map's size is what reproduces the
    /// shipped maps' own geometry, and that is what this does.
    ///
    /// The map is used only when the display really is one of the two sizes a
    /// map is shipped for; otherwise the fallback below is the whole of the hit
    /// testing, and it is the game's own fallback, not a substitute for the map.
    pub fn hit(&self, x: f64, y: f64) -> Option<usize> {
        if let Some(map) = &self.map {
            let px = (x * f64::from(map.width())) as u32;
            let py = (y * f64::from(map.height())) as u32;
            let id = map.region_at(px.min(map.width() - 1), py.min(map.height() - 1));
            return (id != 0 && usize::from(id) <= self.choices).then(|| usize::from(id) - 1);
        }

        let inside = |v: f64| (0.0..1.0).contains(&v);
        if !inside(x) || !inside(y) {
            return None;
        }
        if self.choices == 1 {
            return Some(0);
        }
        // Two boxes: the stacked layout splits on y, the sideways one on x.
        let along = match self.layout {
            Layout::Sideways => x,
            Layout::Stacked => y,
        };
        Some(usize::from(along >= 0.5))
    }

    /// The boxes' extents in the map's own pixels, when a map was loaded.
    pub fn bounds(&self) -> Option<&[days_ui::cmap::Rect]> {
        self.map.as_ref().map(Cmap::all_bounds)
    }

    pub fn map_size(&self) -> Option<(u32, u32)> {
        self.map.as_ref().map(|m| (m.width(), m.height()))
    }
}

/// Rewrites a `[Select1]`/`[Select2]` path for a resolution and layout.
///
/// The INI names the `_Full` map; `FUN_0040f0d0` picks `_Note` instead on a
/// small display, and the two-choice loader appends `_H`. Rewriting the INI's
/// own value rather than composing a path from a hardcoded directory keeps a
/// player who has edited those keys working.
fn rewrite(stem: &str, resolution: Resolution, layout: Layout) -> String {
    let body = stem.strip_suffix(".cmap").unwrap_or(stem);
    let body = body
        .strip_suffix("_Full")
        .or_else(|| body.strip_suffix("_Note"))
        .unwrap_or(body);
    let size = match resolution {
        // `FUN_0040f0d0` is a two-way switch, so the two sizes with no map of
        // their own take the nearer one and miss, which is what puts
        // `Select::hit` on its fallback.
        Resolution::Full | Resolution::Wide => "_Full",
        Resolution::Note | Resolution::Standard => "_Note",
    };
    format!("{body}{size}{}.cmap", layout.suffix())
}

/// What the player's input has said so far this frame, from `FUN_0044de50`.
///
/// The eight buttons are host slot `+0x148`'s eight members, and the choice box
/// reads six of them. Their meaning here is the use `FUN_0044de50` makes of
/// them, which is all the DLL-facing side of the engine knows about them.
#[derive(Debug, Clone, Copy, Default)]
pub struct Input {
    /// Pointer position, normalised.
    pub pointer: (f64, f64),
    /// Slot 0: picks whatever the pointer is already on.
    pub pick: bool,
    /// Slot 1: with slot 0 up, cancels.
    pub dismiss: bool,
    /// Slot 4: moves the highlight to the previous box, wrapping.
    pub prev: bool,
    /// Slot 5: moves it to the next box, wrapping.
    pub next: bool,
    /// Slot 6: confirms the highlight.
    pub confirm: bool,
    /// Slot 7: cancels.
    pub cancel: bool,
}

/// What one frame of polling produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Poll {
    /// Nothing decided; the highlight may have moved.
    Undecided,
    /// This box was chosen.
    Chosen(usize),
    /// The player refused the choice. `FUN_00431740` treats this exactly like a
    /// timeout: a negative answer, recorded as one.
    Cancelled,
}

/// A `[SetSELECT]` in progress.
///
/// `highlight` is the box the keyboard or the pointer is on, or `None` for
/// "nothing" — `FUN_0044de50` keeps it as `-1` and the arrow keys wrap it
/// through `count - 1`.
#[derive(Debug, Clone)]
pub struct Choice {
    pub labels: Vec<String>,
    pub start: Frame,
    pub end: Frame,
    pub highlight: Option<usize>,
    /// Last pointer position, so the hit test only re-runs when it moves —
    /// `FUN_0044de50` compares against the stored pair first.
    pointer: Option<(f64, f64)>,
    /// Set the frame the box goes up, so it goes up once.
    raised: bool,
    decided: Option<Option<usize>>,
}

impl Choice {
    /// A choice from the two label fields of a `[SetSELECT]` statement.
    ///
    /// The second field is the literal `NULL` or `null` for a one-choice box —
    /// `FUN_00438de0` compares against both spellings and counts the labels it
    /// keeps, which is where the box's choice count comes from.
    pub fn new(a: &str, b: Option<&str>, start: Frame, end: Frame) -> Choice {
        let mut labels = vec![a.to_string()];
        if let Some(b) = b {
            if !b.eq_ignore_ascii_case("null") {
                labels.push(b.to_string());
            }
        }
        Choice {
            labels,
            start,
            end,
            highlight: None,
            pointer: None,
            raised: false,
            decided: None,
        }
    }

    pub fn count(&self) -> usize {
        self.labels.len()
    }

    /// Whether the box should be on screen at `frame`.
    pub fn visible(&self, frame: Frame) -> bool {
        frame >= self.start && self.decided.is_none()
    }

    /// The answer, once there is one: `Some(None)` for cancelled or timed out.
    pub fn decision(&self) -> Option<Option<usize>> {
        self.decided
    }

    /// One frame of the choice, as `FUN_00431740` runs it.
    ///
    /// `skipping` is the host's auto flag, slot `+0x134`. It is what turns a
    /// spent window from "no answer" into a *random* one: the tick does
    /// `srand(GetTickCount()); rand() % (count + 1) - 1`, over a range that
    /// includes -1, so skipping can still decline the choice.
    ///
    /// `random` supplies that draw. It is a parameter rather than a call into a
    /// generator so the behaviour can be tested; the engine passes a real one.
    pub fn tick(
        &mut self,
        frame: Frame,
        select: &Select,
        input: Input,
        skipping: bool,
        random: &mut dyn FnMut(usize) -> usize,
    ) -> Event {
        if self.decided.is_some() || frame < self.start {
            return Event::Nothing;
        }
        if !self.raised {
            self.raised = true;
            self.highlight = select.hit(input.pointer.0, input.pointer.1);
            self.pointer = Some(input.pointer);
            // `FUN_00431740` plays index 5 as the box goes up, and
            // `FUN_0044dcc0` arms the boxes and takes the first hit test.
            return Event::Raised(SystemSe::View);
        }

        if Frame(frame.0 + 1) < self.end {
            match self.poll(select, input) {
                Poll::Undecided => Event::Nothing,
                Poll::Chosen(index) => self.settle(Some(index)),
                Poll::Cancelled => self.settle(None),
            }
        } else {
            // The window is spent. Skipping draws one of `count + 1` answers,
            // the extra one being "no answer"; otherwise there is no answer.
            let picked = if skipping {
                let draw = random(self.count() + 1);
                (draw > 0).then(|| draw - 1)
            } else {
                None
            };
            self.settle(picked)
        }
    }

    /// `FUN_0044de50`: one frame of input against the current highlight.
    pub fn poll(&mut self, select: &Select, input: Input) -> Poll {
        if self.pointer != Some(input.pointer) {
            self.pointer = Some(input.pointer);
            self.highlight = select.hit(input.pointer.0, input.pointer.1);
        }

        if input.pick {
            // A press only counts on the box the pointer is genuinely over, so
            // dragging off a box and releasing chooses nothing.
            if let Some(under) = select.hit(input.pointer.0, input.pointer.1) {
                if Some(under) == self.highlight {
                    return Poll::Chosen(under);
                }
            }
        } else if input.dismiss {
            return Poll::Cancelled;
        }

        if input.prev {
            self.highlight = Some(match self.highlight {
                Some(0) | None => self.count() - 1,
                Some(n) => n - 1,
            });
        } else if input.next {
            self.highlight = Some(match self.highlight {
                Some(n) if n + 1 < self.count() => n + 1,
                _ => 0,
            });
        }

        if input.confirm {
            if let Some(index) = self.highlight {
                return Poll::Chosen(index);
            }
        }
        if input.cancel {
            return Poll::Cancelled;
        }
        Poll::Undecided
    }

    /// Records the answer and reports the sound that goes with it.
    ///
    /// `FUN_00431740` plays index 1 for a box that was chosen and index 0 for
    /// an answer of "none", which is the same sound a cancelled menu makes.
    fn settle(&mut self, picked: Option<usize>) -> Event {
        self.decided = Some(picked);
        match picked {
            Some(index) => Event::Decided(index as i32, SystemSe::Select),
            None => Event::Decided(-1, SystemSe::Cancel),
        }
    }
}

/// What a frame of [`Choice::tick`] produced, with the sound it owes.
///
/// The sound is part of the event because the tick plays it there and then, by
/// index, through host slot `+0x50`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Nothing happened this frame.
    Nothing,
    /// The box just went up.
    Raised(SystemSe),
    /// Settled on a choice, or on -1 for none.
    Decided(i32, SystemSe),
}

/// The label limits a choice box is laid out with, read off the install.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub wrap: usize,
    pub advance: u32,
    /// Whether lines are broken at spaces at all. Only the English path does.
    pub word_wrap: bool,
}

impl Metrics {
    pub fn from_ini(film: &Ini, choices: usize) -> Metrics {
        let english = film.get_bool("UseEnglish").unwrap_or(false);
        let select_type = film.get_bool("SelectType").unwrap_or(false);
        Metrics {
            wrap: wrap_limit(choices, select_type, english),
            advance: advance(english),
            word_wrap: english,
        }
    }

    /// Breaks a label into lines the way `FUN_0044ca10` does.
    ///
    /// It walks the string counting characters, and at each space looks ahead to
    /// the next space or end: if the run so far plus that next word would pass
    /// the limit, the line breaks there. So a single word longer than the limit
    /// is never broken — which is the shipped behaviour, and why the limit is 66
    /// for the wide English boxes.
    pub fn lines(&self, label: &str) -> Vec<String> {
        if !self.word_wrap {
            return vec![label.to_string()];
        }
        let mut lines = Vec::new();
        let mut line = String::new();
        let chars: Vec<char> = label.chars().collect();
        let mut count = 0usize;
        for (i, c) in chars.iter().enumerate() {
            line.push(*c);
            count += 1;
            if *c != ' ' {
                continue;
            }
            let word = chars[i + 1..].iter().take_while(|c| **c != ' ').count();
            if count + word > self.wrap {
                lines.push(std::mem::take(&mut line));
                count = 0;
            }
        }
        lines.push(line);
        lines
    }
}

/// Whether a timed-out choice is drawn at random rather than declined.
///
/// The tick asks host slot `+0x134` — the flag the control bar's first widget
/// toggles. `Config.DAT`'s `Skip` key is a different question, asked by slot
/// `+0x88`, and it gates the bar's speed row rather than this.
pub fn skipping(config: &Config, auto: bool) -> bool {
    let _ = config.flag(Flag::Skip);
    auto
}

#[cfg(test)]
mod tests {
    use super::*;

    fn film(select_type: &str, english: &str) -> Ini {
        Ini::parse(&format!(
            "[Select1]=\"System/Select/Select_1_Full.cmap\"\n\
             [Select2]=\"System/Select/Select_2_Full.cmap\"\n\
             [SelectType]=\"{select_type}\"\n\
             [UseEnglish]=\"{english}\"\n"
        ))
    }

    /// A box with no map, so the fallback split is under test.
    fn split(layout: Layout, choices: usize) -> Select {
        Select {
            path: String::new(),
            layout,
            choices,
            map: None,
        }
    }

    #[test]
    fn the_shipped_english_install_stacks_its_boxes() {
        assert_eq!(Layout::from_ini(&film("1", "1")), Layout::Stacked);
        assert_eq!(Layout::from_ini(&film("0", "1")), Layout::Sideways);
        assert_eq!(Layout::from_ini(&film("1", "0")), Layout::Sideways);
    }

    #[test]
    fn the_map_path_follows_the_resolution_and_the_layout() {
        let stem = "System/Select/Select_2_Full.cmap";
        assert_eq!(
            rewrite(stem, Resolution::Full, Layout::Stacked),
            "System/Select/Select_2_Full_H.cmap"
        );
        assert_eq!(
            rewrite(stem, Resolution::Note, Layout::Stacked),
            "System/Select/Select_2_Note_H.cmap"
        );
        assert_eq!(
            rewrite(stem, Resolution::Note, Layout::Sideways),
            "System/Select/Select_2_Note.cmap"
        );
        // The two sizes with no shipped map resolve to the nearer one; the
        // lookup then misses and the split takes over.
        assert_eq!(
            rewrite(stem, Resolution::Wide, Layout::Sideways),
            "System/Select/Select_2_Full.cmap"
        );
    }

    #[test]
    fn one_choice_covers_the_whole_screen() {
        let one = split(Layout::Sideways, 1);
        assert_eq!(one.hit(0.01, 0.99), Some(0));
        assert_eq!(one.hit(0.5, 0.5), Some(0));
        assert_eq!(one.hit(1.0, 0.5), None);
        assert_eq!(one.hit(-0.1, 0.5), None);
    }

    #[test]
    fn two_sideways_boxes_split_on_x_and_two_stacked_ones_on_y() {
        let sideways = split(Layout::Sideways, 2);
        assert_eq!(sideways.hit(0.25, 0.5), Some(0));
        assert_eq!(sideways.hit(0.75, 0.5), Some(1));
        let stacked = split(Layout::Stacked, 2);
        assert_eq!(stacked.hit(0.5, 0.25), Some(0));
        assert_eq!(stacked.hit(0.5, 0.75), Some(1));
    }

    #[test]
    fn null_in_the_second_field_means_one_choice() {
        assert_eq!(
            Choice::new("a", Some("NULL"), Frame(0), Frame(10)).count(),
            1
        );
        assert_eq!(
            Choice::new("a", Some("null"), Frame(0), Frame(10)).count(),
            1
        );
        assert_eq!(Choice::new("a", None, Frame(0), Frame(10)).count(), 1);
        assert_eq!(Choice::new("a", Some("b"), Frame(0), Frame(10)).count(), 2);
    }

    #[test]
    fn the_box_goes_up_once_at_its_start_frame() {
        let mut c = Choice::new("a", Some("b"), Frame(10), Frame(40));
        let s = split(Layout::Sideways, 2);
        let mut rng = |_: usize| 0;
        assert!(!c.visible(Frame(9)));
        assert_eq!(
            c.tick(Frame(9), &s, Input::default(), false, &mut rng),
            Event::Nothing
        );
        assert_eq!(
            c.tick(Frame(10), &s, Input::default(), false, &mut rng),
            Event::Raised(SystemSe::View)
        );
        assert!(c.visible(Frame(10)));
        assert_eq!(
            c.tick(Frame(11), &s, Input::default(), false, &mut rng),
            Event::Nothing
        );
    }

    #[test]
    fn an_ignored_choice_resolves_to_none_when_the_window_runs_out() {
        let mut c = Choice::new("a", Some("b"), Frame(0), Frame(5));
        let s = split(Layout::Sideways, 2);
        let mut rng = |_: usize| 0;
        c.tick(Frame(0), &s, Input::default(), false, &mut rng);
        // `frame + 1 < end` holds up to frame 3.
        for frame in 1..4 {
            assert_eq!(
                c.tick(Frame(frame), &s, Input::default(), false, &mut rng),
                Event::Nothing
            );
        }
        assert_eq!(
            c.tick(Frame(4), &s, Input::default(), false, &mut rng),
            Event::Decided(-1, SystemSe::Cancel)
        );
        assert_eq!(c.decision(), Some(None));
        assert!(!c.visible(Frame(4)));
    }

    #[test]
    fn skipping_draws_a_random_answer_including_none() {
        let s = split(Layout::Sideways, 2);
        // The draw is over `count + 1` values and 0 means "no answer".
        for (draw, expected) in [(0usize, -1i32), (1, 0), (2, 1)] {
            let mut c = Choice::new("a", Some("b"), Frame(0), Frame(2));
            let mut rng = |n: usize| {
                assert_eq!(n, 3);
                draw
            };
            c.tick(Frame(0), &s, Input::default(), true, &mut rng);
            let event = c.tick(Frame(1), &s, Input::default(), true, &mut rng);
            let se = if expected < 0 {
                SystemSe::Cancel
            } else {
                SystemSe::Select
            };
            assert_eq!(event, Event::Decided(expected, se));
        }
    }

    #[test]
    fn clicking_a_box_chooses_it_and_the_arrows_wrap() {
        let s = split(Layout::Stacked, 2);
        let mut c = Choice::new("a", Some("b"), Frame(0), Frame(100));
        let mut rng = |_: usize| 0;
        c.tick(Frame(0), &s, Input::default(), false, &mut rng);

        // Arrows wrap from nothing to the last box and round again.
        assert_eq!(
            c.poll(
                &s,
                Input {
                    prev: true,
                    ..Input::default()
                }
            ),
            Poll::Undecided
        );
        assert_eq!(c.highlight, Some(1));
        c.poll(
            &s,
            Input {
                next: true,
                pointer: (0.0, 0.0),
                ..Input::default()
            },
        );
        assert_eq!(c.highlight, Some(0));

        // Confirm takes the highlight.
        assert_eq!(
            c.poll(
                &s,
                Input {
                    confirm: true,
                    pointer: (0.0, 0.0),
                    ..Input::default()
                }
            ),
            Poll::Chosen(0)
        );

        // A click takes what is under the pointer.
        let mut c = Choice::new("a", Some("b"), Frame(0), Frame(100));
        c.tick(Frame(0), &s, Input::default(), false, &mut rng);
        assert_eq!(
            c.poll(
                &s,
                Input {
                    pointer: (0.5, 0.9),
                    pick: true,
                    ..Input::default()
                }
            ),
            Poll::Chosen(1)
        );
    }

    #[test]
    fn the_second_button_cancels_only_with_the_first_one_up() {
        let s = split(Layout::Sideways, 2);
        let mut c = Choice::new("a", Some("b"), Frame(0), Frame(100));
        let mut rng = |_: usize| 0;
        c.tick(Frame(0), &s, Input::default(), false, &mut rng);
        assert_eq!(
            c.poll(
                &s,
                Input {
                    dismiss: true,
                    pointer: (0.25, 0.5),
                    ..Input::default()
                }
            ),
            Poll::Cancelled
        );
        let mut c = Choice::new("a", Some("b"), Frame(0), Frame(100));
        c.tick(Frame(0), &s, Input::default(), false, &mut rng);
        assert_eq!(
            c.poll(
                &s,
                Input {
                    pick: true,
                    dismiss: true,
                    pointer: (0.25, 0.5),
                    ..Input::default()
                }
            ),
            Poll::Chosen(0)
        );
    }

    #[test]
    fn confirming_nothing_does_nothing() {
        let s = split(Layout::Sideways, 2);
        let mut c = Choice::new("a", Some("b"), Frame(0), Frame(100));
        let mut rng = |_: usize| 0;
        c.tick(Frame(0), &s, Input::default(), false, &mut rng);
        c.highlight = None;
        assert_eq!(
            c.poll(
                &s,
                Input {
                    confirm: true,
                    ..Input::default()
                }
            ),
            Poll::Undecided
        );
    }

    #[test]
    fn the_wrap_limit_has_three_values() {
        assert_eq!(wrap_limit(2, true, false), 11);
        assert_eq!(wrap_limit(1, false, false), 11);
        assert_eq!(wrap_limit(2, false, true), 33);
        assert_eq!(wrap_limit(1, false, true), 66);
        assert_eq!(wrap_limit(2, true, true), 66);
        assert_eq!(advance(true), 16);
        assert_eq!(advance(false), 36);
    }

    #[test]
    fn only_the_english_layout_breaks_lines() {
        let jp = Metrics {
            wrap: 11,
            advance: 36,
            word_wrap: false,
        };
        assert_eq!(
            jp.lines("a b c d e f g h i j k l"),
            vec!["a b c d e f g h i j k l"]
        );

        let en = Metrics {
            wrap: 10,
            advance: 16,
            word_wrap: true,
        };
        assert_eq!(en.lines("aaa bbb ccc ddd"), vec!["aaa bbb ", "ccc ddd"]);
        // A word longer than the limit is never broken.
        assert_eq!(en.lines("aaaaaaaaaaaaaaaa"), vec!["aaaaaaaaaaaaaaaa"]);
    }
}
