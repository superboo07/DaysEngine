//! The in-game control bar: `System/MenuBar`, the strip that sits over playback.
//!
//! # Whose screen this is
//!
//! The bar is not one of `SystemInit`'s eight menu modes. The executable holds
//! it directly: `_SetMenuBar@4` is a one-line export that writes the address of
//! a static `FILM::MenuBar` into the pointer the caller passes
//! (`FUN_00422170` passes its own `+0x300`), and from then on the engine drives
//! the bar through that object's vtable at `0x1003d804`:
//!
//! ```text
//! +0x08 release textures   +0x20 update: hit test and dispatch  (FUN_10024100)
//! +0x0c load and lay out   +0x24/+0x28 -
//! +0x10 re-place on resize +0x2c draw the play/pause widget      (FUN_100258f0)
//! +0x14 draw               +0x30 widget 2's action               (FUN_10025b90)
//! +0x18 -                  +0x34 widget 0's action               (FUN_10025cf0)
//! +0x1c set host pointers  +0x38/+0x3c/+0x40/+0x44 lifetime
//! ```
//!
//! So the widget geometry is in the DLL — the `MENUBAR` table the atlas search
//! finds, 25 records plus a long trailing run of alternates — while every
//! decision is a host call back into the executable. This module is the DLL's
//! half: which widget is live, what it shows, and what it asks the host for.
//! [`Act`] is that ask; the engine answers it.
//!
//! # The 25 widgets
//!
//! One region per control, dense from 1, so widget index `n` is region `n + 1`
//! and table record `n`. The grouping below is not a reading of the sprites:
//! it is exactly how `FUN_10024100`'s dispatch, `FUN_10023fb0`'s enabled test
//! and `FUN_100262e0`'s caption switch all three bracket the range.
//!
//! ```text
//!  0        toggles the host's auto-advance flag       host +0x120
//!  1        toggles pause                              host +0xf4
//!  2        restart, then step back on a second press   host +0xfc(1) / (2)
//!  3        jump to the end of this script             host +0xfc(2)
//!  4        skip                                       host +0x12c(1), +0xfc(5)
//!  5..9     playback speed, one widget per rate        host +0x8c(0..4)
//! 10..12    open a menu: save, load, backlog           host +0xf8(4/5/3)
//! 13        open the settings menu                     host +0xf8(2)
//! 14        leave playback                             host +0x100(1)
//! 15..24    ten steps of one setting                   FUN_10026ed0
//! ```
//!
//! Note widgets 3 and 2's second press make the *same* host call. That is the
//! shipped dispatch, not a transcription slip: widget 2 goes through
//! `FUN_10025b90`, which asks for `+0xfc(1)` the first time and `+0xfc(2)` the
//! second, and widget 3 asks for `+0xfc(2)` outright.
//!
//! # It is a drop-down, and it is translucent
//!
//! The bar is not a fixture along the top of the screen. It is on screen only
//! while the pointer is inside the 800x75 strip, ramping in over 300ms and out
//! over 1000ms once the pointer leaves — see [`Fade`], which is where the
//! evidence for that lives, because the trigger is a value the hit map returns
//! rather than anything the bar decides.
//!
//! And `MENUBAR.PNG` is RGBA: the strip's panels are semi-transparent and the
//! frame underneath shows through them. So [`Bar::compose`] returns a *layer*,
//! not a picture — flattening it onto black first would put a black band across
//! the top of the screen, which is not what the original shows.
//!
//! # What the bar does not decide
//!
//! Where `+0xfc` actually lands is the executable's state machine
//! (`FUN_00427300`, state 4 in `FUN_00425bf0`), and that is script-chaining
//! territory rather than menu territory — see [`Act::Seek`].

use crate::install::config::{Config, Flag};
use crate::ui::screen::{Error, Resolution, Screen, WidgetState};
use days_ui::Image;

/// The screen's path stem, as the DLL spells it.
pub const PATH: &str = "System/MenuBar/MenuBar";

/// Number of hit regions, and so of widgets.
pub const WIDGETS: usize = 25;

/// The five playback rates widgets 5..9 select, from the table at `0x004f99f0`
/// that host slot `+0x8c` indexes.
///
/// The last two are **12 and 24, not 16 and 32**. The English chip sheet draws
/// those two buttons as `▶×16` and `▶×32`, but the art is not the authority:
/// `FUN_00424f90` stores `DAT_004f99f0[index]` as the rate and that table is
/// `1.0, 2.0, 4.0, 12.0, 24.0`.
pub const SPEEDS: [f32; 5] = [1.0, 2.0, 4.0, 12.0, 24.0];

/// How long the bar takes to fade in and out, in milliseconds.
///
/// `FUN_10024100` calls the fade helper `FUN_100255c0(this, 1, 300)` while the
/// pointer is over the bar and `FUN_100255c0(this, 0, 1000)` once it leaves, and
/// the helper ramps a 0..255 alpha linearly over that many milliseconds of
/// `timeGetTime`.
pub const FADE_IN_MS: u32 = 300;
pub const FADE_OUT_MS: u32 = 1000;

/// The bar's visibility: it is a drop-down, not a fixture.
///
/// `FUN_10024100` stores `lookup(pointer) - 1` at `this+0xd4` and then branches
/// on it being **-2**:
///
/// ```text
/// if (this+0xd4 == -2) {                       // pointer is off the strip
///     if (DAT_100508c8 == 0) this+0xbc = 0;    // faded right out: bar is off
///     else FUN_100255c0(this, 0, 1000);        // ramp out
/// } else {
///     if (DAT_100508c8 != 0xff) FUN_100255c0(this, 1, 300);
///     this+0xbc = 1;
/// }
/// ```
///
/// -2 is what makes this a drop-down, and it is the hit map's doing rather than
/// a sentinel the bar invents. The map object is the executable's `ClickableMap`
/// (vtable `0x004d70c4`, stored by its constructor `FUN_00465830`, which the DLL
/// obtains through host factory slot `+0xac` case 5), and its lookup
/// `FUN_00465bc0` returns **-1 for a point outside the map's own rectangle** and
/// the region id — 0 for no region — for one inside it. So off the strip gives
/// `-1 - 1 = -2` and the bar goes away, while anywhere on the strip, widget or
/// not, gives -1 or better and it stays. The strip is 800x75 at the top of the
/// screen, because `ClickableMap`'s origin members are zeroed by that
/// constructor and nothing in the bar's path ever sets them.
///
/// `this+0xbc` gates every resting sprite in `FUN_10024ca0`, and `DAT_100508c8`
/// is a 0..255 alpha `FUN_10025690` applies to all of them at once as an ARGB
/// modulation — so the whole strip fades as one.
#[derive(Debug, Clone, Copy, Default)]
pub struct Fade {
    /// `DAT_100508c8`.
    alpha: u8,
    /// `DAT_100508c4`: the tick the ramp in progress started on, or `None` for
    /// no ramp. The original clears it **only when a ramp completes**, so the
    /// pointer leaving mid-fade-in does not restart the clock — the direction
    /// flips against the old start and the alpha jumps. That is reproduced.
    started: Option<u32>,
    /// `this+0xbc`.
    up: bool,
}

impl Fade {
    /// One frame of the fade, given the clock and whether the pointer is over
    /// the strip at all.
    pub fn update(&mut self, now_ms: u32, over_strip: bool) {
        if over_strip {
            self.up = true;
            if self.alpha != 0xff {
                self.ramp(now_ms, true, FADE_IN_MS);
            }
        } else if self.alpha == 0 {
            self.up = false;
        } else {
            self.ramp(now_ms, false, FADE_OUT_MS);
        }
    }

    /// `FUN_100255c0`.
    fn ramp(&mut self, now_ms: u32, rising: bool, over_ms: u32) {
        let started = *self.started.get_or_insert(now_ms);
        let elapsed = now_ms.saturating_sub(started);
        if elapsed < over_ms {
            let ramp = (elapsed * 255 / over_ms.max(1)) as u8;
            self.alpha = if rising { ramp } else { 255 - ramp };
        } else {
            self.alpha = if rising { 255 } else { 0 };
            self.started = None;
        }
    }

    /// Whether the bar draws at all this frame.
    pub fn drawn(&self) -> bool {
        self.up
    }

    /// The alpha every one of the bar's sprites is modulated by.
    pub fn alpha(&self) -> u8 {
        self.alpha
    }

    /// Puts the bar straight into its hidden state, for a fresh script.
    pub fn reset(&mut self) {
        *self = Fade::default();
    }
}

/// Frames within which a second press of widget 2 means "step back".
///
/// `FUN_10024100` clears the latch when its frame argument exceeds `0x48`, so
/// the window is 72 frames — three seconds at the script's 24 fps.
pub const RESTART_LATCH_FRAMES: u32 = 0x48;

/// Which menu widgets 10..13 ask the host to open, by the number they pass to
/// host slot `+0xf8`.
///
/// The slot puts the engine into state 3 and hands the number to
/// `_SetReMenu@4`, so this is the DLL's own re-entry number rather than one of
/// `SystemInit`'s mode integers. `setSystemInit` is the switch that consumes
/// it, and three of the four the bar produces are known from it:
///
/// ```text
/// 4  the save/load module, opened to save   (its +0x94 poked to 1)
/// 5  the same module, opened to load        (+0x94 poked to 0)
/// 2  the Option screen
/// 3  an object `SystemInit` has no case for -- not recovered
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuRequest(pub i32);

/// Where widget 2, 3 or 4 asks the engine to move to, by the code passed to
/// host slot `+0xfc`.
///
/// `FUN_00425bf0` is the state-4 handler that consumes these, and the three
/// codes below are the ones the bar can produce. Code 1 restarts the script in
/// place; codes 2 and 5 both end up at the "this script is finished" path,
/// which loads whatever `_GetNextScriptFile@12` names. The codes travel out of
/// here unresolved: which script that is belongs to the branch graph, not to
/// this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seek(pub i32);

impl Seek {
    /// Restart the current script from its first frame. `FUN_00425bf0` case 3
    /// under code 1 rewinds the timeline and replays it.
    pub const RESTART: Seek = Seek(1);
    /// Leave the current script. Widget 3, and widget 2's second press.
    pub const END_OF_SCRIPT: Seek = Seek(2);
    /// Widget 4's code, reached only after host `+0x12c(1)`.
    pub const SKIP: Seek = Seek(5);
}

/// What activating a widget asks the host for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// The widget is not live, so the press is swallowed.
    None,
    /// Flip the auto-advance flag and write the settings out. Host `+0x120`,
    /// which also stamps `timeGetTime` so the widget's animation has a start.
    ToggleAuto,
    /// Pause if playing, resume if paused. Host `+0xf4`.
    TogglePause,
    /// Move the timeline. Host `+0xfc`; widget 4 additionally sets the host's
    /// skip request through `+0x12c(1)` first, which [`Act::Seek`] carries as
    /// its own code rather than as a separate action.
    Seek(Seek),
    /// Select a playback rate by index into [`SPEEDS`]. Host `+0x8c`.
    Speed(usize),
    /// Open a menu over playback. Host `+0xf8`.
    Menu(MenuRequest),
    /// Leave playback. Host `+0x100(1)`.
    Leave,
    /// One of ten steps of a setting, `0..=9`, from `FUN_10026ed0`. **Which
    /// setting is not recovered**: the ten widgets are live only while host
    /// `+0x98` is set, and nothing in the menu DLL says what that member means.
    Step(usize),
}

/// What the engine has to tell the bar about itself for it to draw and dispatch.
///
/// Each field is one host slot the bar calls, named for the member the slot
/// returns rather than for the widget it happens to drive.
#[derive(Debug, Clone, Copy, Default)]
pub struct State {
    /// Host `+0x134`: the flag widget 0 toggles. While it is set, widget 0
    /// draws from the alternate run and animates, and the choice box stops
    /// following the pointer (`FUN_0044dcc0`).
    pub auto: bool,
    /// Host `+0x108`: playback is paused. Swaps widget 1's sprite.
    pub paused: bool,
    /// Host `+0x104`: the replay menu started this playback. Disables widgets
    /// 4 and 10..12.
    pub replay: bool,
    /// Host `+0x110`: `FUN_00428210`, which answers false unless the host's
    /// own draw-message flag is set and the asset it names resolves. Disables
    /// widgets 4 and 5..9.
    pub message: bool,
    /// Host `+0x88`: `FUN_00427490` — the `Skip` setting, or else the same
    /// asset test. Required by widgets 5..9.
    pub skippable: bool,
    /// Host `+0x98`: the member host `+0x94` sets. Required by widgets 15..24.
    pub stepping: bool,
    /// Host `+0x11c`: the rate index currently in force, an index into
    /// [`SPEEDS`].
    pub speed: usize,
    /// Host `+0x13c`: while set, `FUN_10024100` returns before doing anything,
    /// so the bar neither hovers nor dispatches.
    pub frozen: bool,
    /// Host `+0x140`: while set, `FUN_10024ca0` draws none of the bar's own
    /// widgets.
    pub hidden: bool,
    /// `_GetSuperSkipFlag@0`, the `SuperSkip` setting. Required by widget 4,
    /// and it picks widget 4's resting sprite.
    pub super_skip: bool,
    /// Host `+0x118`: the rate in force as a number rather than an index. The
    /// rate readout is drawn only once it is at least 2.0 — the threshold is
    /// the double at `0x10039748`.
    pub rate: f32,
    /// Host `+0x154`: while set, `FUN_10024ca0` draws the gauge even when the
    /// bar itself is faded out — the gauge block appears twice, once inside
    /// the "bar is up" test and once outside it under this flag.
    ///
    /// The member is `engine + 0x79c`, and what raises it is a feeling
    /// change: `FUN_10005c60` in `RouteProcSDHQ.dll` sets it through host slot
    /// `+0x30` after crediting a delta, but only when the counter it moved was
    /// `001` or `002` — the two the gauge draws. `FUN_10026050` clears it
    /// again through the same slot once it has read the two values. So the
    /// gauge surfaces over a faded bar exactly when the affection counters
    /// just moved. See [`crate::install::feeling`].
    pub gauge_raised: bool,
    /// The two counters the gauge draws, `(001, 002)`, if they are known.
    ///
    /// `FUN_10026050` asks the host for these two by name through slot `+8`.
    /// They live in the save's variable store, so a bar drawn without save
    /// state has nothing to show and this is `None`.
    pub gauge: Option<(i32, i32)>,
}

impl State {
    /// Reads the parts of the state that live in `Config.DAT`.
    ///
    /// Two do: host `+0x88` is `FUN_00427490`, whose first act is
    /// `_GetSkipFlag@0`, and `_GetSuperSkipFlag@0` is asked for directly.
    /// Both exports return keys the settings screen writes. Everything else on
    /// [`State`] is a live member of the running engine and has to come from
    /// the caller.
    pub fn from_config(config: &Config) -> State {
        State {
            skippable: config.flag(Flag::Skip),
            super_skip: config.flag(Flag::SuperSkip),
            rate: SPEEDS[0],
            ..State::default()
        }
    }
}

/// Whether a widget can be activated, from `FUN_10023fb0`.
///
/// A press on a widget that is not live is swallowed *and makes no sound*: the
/// dispatch asks this question before playing the click, so a dead button is
/// silent as well as inert.
pub fn enabled(widget: usize, state: State) -> bool {
    match widget {
        0..=3 | 0xd | 0xe => true,
        4 => !state.replay && !state.message && state.skippable,
        5..=9 => !state.message && state.skippable,
        10..=12 => !state.replay,
        0xf..=0x18 => state.stepping,
        _ => false,
    }
}

/// What a widget does, from `FUN_10024100`'s dispatch.
///
/// `latched` is the bar's own one-bit memory for widget 2, set by its first
/// press and cleared after [`RESTART_LATCH_FRAMES`]; see [`restart_latch`].
/// Every other widget is stateless.
pub fn action(widget: usize, state: State, latched: bool) -> Act {
    if !enabled(widget, state) {
        return Act::None;
    }
    match widget {
        0 => Act::ToggleAuto,
        1 => Act::TogglePause,
        // `FUN_10025b90`. The second press is refused outright while the
        // replay menu is driving playback or the host's `+0x98` member is set,
        // so in those cases the button restarts every time.
        2 => {
            if latched && !state.replay && !state.stepping {
                Act::Seek(Seek::END_OF_SCRIPT)
            } else {
                Act::Seek(Seek::RESTART)
            }
        }
        3 => Act::Seek(Seek::END_OF_SCRIPT),
        4 => Act::Seek(Seek::SKIP),
        5..=9 => Act::Speed(widget - 5),
        10 => Act::Menu(MenuRequest(4)),
        11 => Act::Menu(MenuRequest(5)),
        12 => Act::Menu(MenuRequest(3)),
        13 => Act::Menu(MenuRequest(2)),
        14 => Act::Leave,
        0xf..=0x18 => Act::Step(widget - 0xf),
        _ => Act::None,
    }
}

/// How widget 2's latch moves when it is pressed or time passes.
///
/// `FUN_10025b90` sets the latch on the first press but only when playback is
/// neither a replay nor stepping, and clears it on the second; `FUN_10024100`
/// clears it once its frame argument passes [`RESTART_LATCH_FRAMES`].
pub fn restart_latch(latched: bool, state: State) -> bool {
    !latched && !state.replay && !state.stepping
}

/// The caption strip shown for the hovered widget, as an index into the
/// alternate run, or `None` for a widget with no caption.
///
/// From `FUN_100262e0`, which is a switch over the same twelve groups the
/// dispatch uses: widgets 0 to 4 each have their own strip, 5..9 share one,
/// 10, 11, 12, 13 and 14 each have their own, and 15..24 share the last.
/// The record addresses run `0x1004d318` upwards at the table's 24-byte stride,
/// so the strips are consecutive records and this returns their order, not
/// their address.
pub fn caption(widget: usize) -> Option<usize> {
    Some(match widget {
        0..=4 => widget,
        5..=9 => 5,
        10..=14 => widget - 4,
        0xf..=0x18 => 11,
        _ => return None,
    })
}

/// First of the twelve caption records. `0x1004d318` is
/// `(0x1004d318 - 0x1004ce20) / 0x18` records past the start of the table.
pub const CAPTION_FIRST_RECORD: usize = 53;

/// The records the bar's resting art comes from.
///
/// `MENUBAR.PNG` is nearly empty — two arrow buttons — so the bar cannot be
/// composited from widget hover states alone. `FUN_10024ca0` draws about
/// fifteen sprite objects every frame from the chip sheet, and `FUN_10021c20`
/// is where each of those objects is given a record. Both halves are named
/// here by the object they belong to, `this + N`, so the two can be checked
/// against each other.
mod record {
    /// `this+0xa4[0..5]`, placed by `FUN_10026100` from records `0x30 + k`:
    /// the five menu buttons' resting art, drawn unconditionally.
    pub const MENU_BUTTONS: std::ops::RangeInclusive<usize> = 48..=52;
    /// `this+0x68` and `this+0x6c`, one 216-wide sprite covering the whole
    /// rate row. The live one is taken when host `+0x88` answers non-zero,
    /// which is the same question that makes widgets 5..9 pressable.
    pub const RATES_LIVE: usize = 47;
    pub const RATES_DEAD: usize = 66;
    /// `this+0x74` and `this+0x78`, widget 4, picked by `_GetSuperSkipFlag@0`
    /// — again the same question that makes the widget pressable.
    pub const SKIP_LIVE: usize = 46;
    pub const SKIP_DEAD: usize = 65;
    /// `this+0x70`, widget 1's resting art, picked by host `+0x108`.
    pub const PLAY_RESTING: usize = 44;
    pub const PAUSE_RESTING: usize = 45;
    /// `this+0x64`, widget 0's resting art, drawn only while the auto flag is
    /// clear — with it set, the animation takes over.
    pub const AUTO_RESTING: usize = 42;
    /// `this+0x8c`, the strip above the ten step widgets, picked by host
    /// `+0x98` — the same question that makes them pressable.
    pub const STEPS_DEAD: usize = 69;
    pub const STEPS_LIVE: usize = 70;
    /// `this+0x80`, a 538-wide strip drawn alongside the gauge.
    pub const GAUGE_BED: usize = 67;
    /// `this+0x94`, drawn while host `+0x98` is set. Its destination is at
    /// y = 80, **below the 800x75 hit map**, so composing at the map's size
    /// clips it away; that is the shipped geometry, not a placement mistake.
    pub const STEPS_OVERFLOW: usize = 68;
    /// `this+0x60` while widget 0 is hovered, from `FUN_10024100`'s special
    /// case for index 0: its own record with the flag clear, record 25 with it
    /// set.
    pub const AUTO_HOVER_ON: usize = 25;
    /// `this+0x60` while widget 1 is hovered. Inverted on purpose: the button
    /// shows what pressing it will do, so it offers "pause" while playing.
    /// `FUN_10024100` and `FUN_100258f0` pick the same pair independently.
    pub const PAUSE_HOVER: usize = 26;
    pub const PLAY_HOVER: usize = 1;
}

/// The thirteen frames widget 0 cycles through while the auto flag is set.
///
/// `FUN_10024ca0` picks one with
/// `((now - started) / (1000 / (speed + 1))) % 13 + 0x1d`, where `0x1d` is a
/// record index into the same table the widgets come from — so the frames are
/// records 29 to 41 and the animation runs faster the higher the playback rate.
pub const AUTO_FRAMES: usize = 13;
pub const AUTO_FIRST_RECORD: usize = 0x1d;

/// Which of widget 0's animation frames is showing.
pub fn auto_frame(elapsed_ms: u32, speed: usize) -> usize {
    let period = 1000 / (speed as u32 + 1);
    let step = elapsed_ms.checked_div(period).unwrap_or(0);
    AUTO_FIRST_RECORD + (step as usize % AUTO_FRAMES)
}

/// The bar, loaded and ready to hit-test and draw.
pub struct Bar {
    screen: Screen,
    /// Widget 2's latch, and the frame it was set on.
    latch: Option<u32>,
    fade: Fade,
}

impl Bar {
    pub fn load(
        vfs: &crate::install::vfs::Vfs,
        dll: &[u8],
        resolution: Resolution,
    ) -> Result<Bar, Error> {
        Ok(Bar {
            screen: Screen::load(vfs, dll, PATH, resolution)?,
            latch: None,
            fade: Fade::default(),
        })
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    /// The strip's size in display pixels — an 800x75 band at the top of the
    /// screen, scaled like any other UI art.
    pub fn strip(&self) -> (u32, u32) {
        self.screen.size()
    }

    /// One frame of the drop-down, given the pointer in the strip's own pixels.
    ///
    /// `over` is `None` when the pointer is not in the strip's rectangle at all,
    /// which is the `-2` case that fades the bar out. A pointer inside the strip
    /// but on no widget keeps it up, so this takes the position rather than the
    /// hovered widget.
    pub fn point_at(&mut self, over: Option<(u32, u32)>, now_ms: u32) -> Option<usize> {
        self.fade.update(now_ms, over.is_some());
        over.and_then(|(x, y)| self.hit(x, y))
    }

    pub fn fade(&self) -> Fade {
        self.fade
    }

    /// Hides the bar again, for the start of a script.
    pub fn reset(&mut self) {
        self.fade.reset();
        self.latch = None;
    }

    /// The widget under a point in the bar's own space, or `None`.
    ///
    /// The bar's hit map is an 800x75 strip rather than a full screen, so a
    /// point below it simply misses — the caller passes coordinates relative to
    /// the strip's top-left corner.
    pub fn hit(&self, x: u32, y: u32) -> Option<usize> {
        self.screen.hit(x, y)
    }

    /// Activates the widget under the pointer and reports what it asks for.
    ///
    /// `frame` is the script frame, which is what expires widget 2's latch.
    pub fn press(&mut self, widget: usize, state: State, frame: u32) -> Act {
        self.expire_latch(frame);
        let latched = self.latch.is_some();
        let act = action(widget, state, latched);
        if widget == 2 && act != Act::None {
            self.latch = if restart_latch(latched, state) {
                Some(frame)
            } else {
                None
            };
        }
        act
    }

    /// Drops widget 2's latch once its window has passed.
    pub fn expire_latch(&mut self, frame: u32) {
        if let Some(set) = self.latch {
            if frame.saturating_sub(set) > RESTART_LATCH_FRAMES {
                self.latch = None;
            }
        }
    }

    /// Everything the bar draws this frame, as records in draw order.
    ///
    /// This is `FUN_10024ca0` in order, which matters: the records overlap, and
    /// the hover sprite and the caption go on last so they cover the resting
    /// art underneath. Records outside the widget run are alternates; the
    /// widget run's own records are the hover art.
    ///
    /// The gauge's three moving pieces are not in this list and cannot be:
    /// they are not chip records at all. `FUN_10026540` sets their
    /// destination and source rectangles itself, from the two counters — see
    /// [`gauge`] for that geometry. `this+0x90` is also absent, because
    /// `FUN_10021c20` assigns it no record, so where its art comes from is
    /// **not recovered**.
    pub fn records(&self, hovered: Option<usize>, state: State, elapsed_ms: u32) -> Vec<usize> {
        let mut out = Vec::new();

        // `if (host+0x140() == 0)`: with the bar hidden, only the pinned gauge
        // bed survives, and even that only under host +0x154.
        if !state.hidden {
            out.extend(record::MENU_BUTTONS);
            if state.message {
                out.push(record::RATES_DEAD);
                out.push(record::SKIP_DEAD);
            } else {
                out.push(if state.skippable {
                    record::RATES_LIVE
                } else {
                    record::RATES_DEAD
                });
                out.push(if state.super_skip {
                    record::SKIP_LIVE
                } else {
                    record::SKIP_DEAD
                });
                out.push(if state.stepping {
                    record::STEPS_LIVE
                } else {
                    record::STEPS_DEAD
                });
            }
            out.push(if state.paused {
                record::PAUSE_RESTING
            } else {
                record::PLAY_RESTING
            });
            if !state.auto {
                out.push(record::AUTO_RESTING);
            }
            if !state.gauge_raised {
                out.push(record::GAUGE_BED);
            }
        }
        if state.gauge_raised {
            out.push(record::GAUGE_BED);
        }

        // The rate readout appears only once the rate is at least 2.0.
        if state.rate >= 2.0 && state.speed < SPEEDS.len() {
            out.push(state.speed + 5);
        }
        if state.stepping {
            out.push(record::STEPS_OVERFLOW);
        }
        if state.auto {
            out.push(auto_frame(elapsed_ms, state.speed));
        }

        if !state.hidden {
            if let Some(widget) = hovered.filter(|w| *w < WIDGETS) {
                out.push(match widget {
                    0 if state.auto => record::AUTO_HOVER_ON,
                    1 if state.paused => record::PLAY_HOVER,
                    1 => record::PAUSE_HOVER,
                    other => other,
                });
                if let Some(group) = caption(widget) {
                    out.push(CAPTION_FIRST_RECORD + group);
                }
            }
        }
        out
    }

    /// The records of [`Bar::records`] as states the screen can composite.
    ///
    /// A record inside the widget run becomes that widget's own state; one past
    /// it becomes an appended alternate, which is how [`Screen::compose_over`]
    /// takes sprites that are not any widget's state.
    pub fn states(
        &self,
        hovered: Option<usize>,
        state: State,
        elapsed_ms: u32,
    ) -> Vec<WidgetState> {
        let mut states = vec![WidgetState::Resting; WIDGETS];
        for record in self.records(hovered, state, elapsed_ms) {
            if record < WIDGETS {
                states[record] = WidgetState::Active;
            } else {
                states.push(self.extra(record));
            }
        }
        states
    }

    /// Turns a record index into a widget state, warning rather than drawing
    /// the wrong sprite when the recovered table is shorter than expected.
    fn extra(&self, record: usize) -> WidgetState {
        match record.checked_sub(WIDGETS) {
            Some(n) if n < self.screen.atlas().extras.len() => WidgetState::Extra(n),
            _ => {
                log::warn!("{PATH}: no alternate record {record} in the recovered table");
                WidgetState::Resting
            }
        }
    }

    /// Composites the strip as a transparent layer, faded.
    ///
    /// `MENUBAR.PNG` is RGBA and the engine draws the strip's sprites over
    /// whatever frame is underneath, so this returns a layer for the caller to
    /// blend rather than a picture with a black bar in it. The fade is applied
    /// as one modulation over the whole layer, which is what `FUN_10025690`
    /// does — it walks every sprite the bar owns and sets the same ARGB on each.
    ///
    /// One sprite escapes it in the original: `FUN_10025690` skips widget 0's
    /// animation while `_GetAutoDraw@0` is non-zero, so that one stays at full
    /// alpha. **What that export returns is not recovered**, so the exception is
    /// not reproduced and the whole strip fades together.
    pub fn compose(&self, states: &[WidgetState]) -> Image {
        self.screen.compose_layer(states)
    }

    /// As [`Bar::compose`], with the fade already multiplied in.
    ///
    /// The SDL path does not use this: it caches the layer on its record list
    /// and modulates the texture's alpha instead, so a ramp does not
    /// recomposite the strip every frame. This is for the headless path, where
    /// there is one image and no texture to modulate.
    pub fn compose_faded(&self, states: &[WidgetState]) -> Image {
        let mut layer = self.compose(states);
        let alpha = u32::from(self.fade.alpha());
        if alpha < 255 {
            for px in layer.rgba.as_chunks_mut::<4>().0 {
                px[3] = (u32::from(px[3]) * alpha / 255) as u8;
            }
        }
        layer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state in which everything the bar can ask about is available.
    fn live() -> State {
        State {
            skippable: true,
            stepping: true,
            super_skip: true,
            rate: SPEEDS[0],
            ..State::default()
        }
    }

    #[test]
    fn the_bar_is_hidden_until_the_pointer_reaches_the_strip() {
        let mut fade = Fade::default();
        assert!(!fade.drawn());
        assert_eq!(fade.alpha(), 0);

        // Off the strip it stays away however long it is left.
        fade.update(0, false);
        fade.update(10_000, false);
        assert!(!fade.drawn());

        // The pointer arriving makes it draw at once, at alpha 0, and it ramps.
        fade.update(0, true);
        assert!(fade.drawn());
        assert_eq!(fade.alpha(), 0);
        fade.update(FADE_IN_MS / 2, true);
        assert_eq!(fade.alpha(), 127);
        fade.update(FADE_IN_MS, true);
        assert_eq!(fade.alpha(), 255);
    }

    #[test]
    fn the_pointer_leaving_ramps_out_and_then_takes_the_bar_away() {
        let mut fade = Fade::default();
        fade.update(0, true);
        fade.update(FADE_IN_MS, true);
        assert_eq!(fade.alpha(), 255);

        // The ramp out is over the longer of the two windows, and the bar keeps
        // drawing all the way through it.
        fade.update(1_000, false);
        assert!(fade.drawn());
        fade.update(1_000 + FADE_OUT_MS / 2, false);
        assert_eq!(fade.alpha(), 128);
        assert!(fade.drawn());
        fade.update(1_000 + FADE_OUT_MS, false);
        assert_eq!(fade.alpha(), 0);
        // It is still "up" on the frame the alpha hits zero; the next frame is
        // the one that takes it away, which is the order `FUN_10024100` does it
        // in — the test comes before the assignment.
        fade.update(3_000, false);
        assert!(!fade.drawn());
    }

    #[test]
    fn a_reversal_mid_ramp_does_not_restart_the_clock() {
        // `FUN_100255c0` clears its start tick only when a ramp completes, so
        // flipping direction part way through keeps the old start and the alpha
        // jumps. Shipped behaviour, reproduced rather than smoothed over.
        let mut fade = Fade::default();
        fade.update(0, true);
        fade.update(FADE_IN_MS / 2, true);
        assert_eq!(fade.alpha(), 127);
        // Now leave. The out ramp measures from tick 0, not from now, so half
        // of FADE_IN_MS into a 1000ms out ramp is barely any fall at all.
        fade.update(FADE_IN_MS / 2, false);
        assert_eq!(
            fade.alpha(),
            255 - (FADE_IN_MS / 2 * 255 / FADE_OUT_MS) as u8
        );
    }

    #[test]
    fn a_pointer_on_the_strip_but_on_no_widget_keeps_the_bar_up() {
        // -1 from the hit map, not -2: the bar stays. This is the distinction
        // the whole drop-down turns on.
        let mut fade = Fade::default();
        fade.update(0, true);
        fade.update(FADE_IN_MS, true);
        assert!(fade.drawn());
        assert_eq!(fade.alpha(), 255);
    }

    #[test]
    fn every_widget_is_covered_by_the_enabled_test() {
        // `FUN_10023fb0`'s switch has an arm for each of 0..=0x18 and a
        // default of 0, so nothing in range should fall through to the
        // default and nothing past it should be live.
        for widget in 0..WIDGETS {
            assert!(
                enabled(widget, live()),
                "widget {widget} fell through to the default arm"
            );
        }
        assert!(!enabled(WIDGETS, live()));
    }

    #[test]
    fn replay_disables_skip_and_the_three_save_menus() {
        let replay = State {
            replay: true,
            ..live()
        };
        assert!(!enabled(4, replay));
        for widget in 10..=12 {
            assert!(!enabled(widget, replay));
        }
        // The settings menu and leaving are not conditional on replay.
        assert!(enabled(0xd, replay));
        assert!(enabled(0xe, replay));
    }

    #[test]
    fn the_speed_row_needs_the_skip_setting() {
        let no_skip = State {
            skippable: false,
            ..live()
        };
        for widget in 4..=9 {
            assert!(!enabled(widget, no_skip));
        }
        assert!(enabled(3, no_skip));
    }

    #[test]
    fn the_ten_steps_need_the_hosts_own_member() {
        let idle = State {
            stepping: false,
            ..live()
        };
        for widget in 0xf..=0x18 {
            assert!(!enabled(widget, idle));
            assert!(enabled(widget, live()));
        }
        assert_eq!(action(0xf, live(), false), Act::Step(0));
        assert_eq!(action(0x18, live(), false), Act::Step(9));
    }

    #[test]
    fn the_speed_row_maps_one_widget_per_rate() {
        for (index, _) in SPEEDS.iter().enumerate() {
            assert_eq!(action(index + 5, live(), false), Act::Speed(index));
        }
    }

    #[test]
    fn the_two_fastest_rates_are_twelve_and_twentyfour() {
        // The chip sheet's captions say 16 and 32; `DAT_004f99f0` says this.
        assert_eq!(SPEEDS, [1.0, 2.0, 4.0, 12.0, 24.0]);
    }

    #[test]
    fn widget_two_restarts_then_steps_back() {
        // `FUN_10025b90` takes the second press only while neither the replay
        // menu nor the host's `+0x98` member is driving playback, so the state
        // here has `stepping` clear.
        let plain = State {
            stepping: false,
            ..live()
        };
        assert_eq!(action(2, plain, false), Act::Seek(Seek::RESTART));
        assert_eq!(action(2, plain, true), Act::Seek(Seek::END_OF_SCRIPT));
        // With it set, the button restarts however many times it is pressed.
        assert_eq!(action(2, live(), true), Act::Seek(Seek::RESTART));
    }

    #[test]
    fn widget_two_never_latches_during_a_replay() {
        let replay = State {
            replay: true,
            ..live()
        };
        assert!(!restart_latch(false, replay));
        // ...so the second press restarts again rather than stepping back.
        assert_eq!(action(2, replay, true), Act::Seek(Seek::RESTART));
    }

    #[test]
    fn widget_three_asks_for_the_same_code_as_a_second_press_of_widget_two() {
        let plain = State {
            stepping: false,
            ..live()
        };
        assert_eq!(action(3, plain, false), action(2, plain, true));
    }

    #[test]
    fn the_menu_buttons_ask_for_four_different_numbers() {
        let asked: Vec<Act> = (10..=14).map(|w| action(w, live(), false)).collect();
        assert_eq!(
            asked,
            vec![
                Act::Menu(MenuRequest(4)),
                Act::Menu(MenuRequest(5)),
                Act::Menu(MenuRequest(3)),
                Act::Menu(MenuRequest(2)),
                Act::Leave,
            ]
        );
    }

    #[test]
    fn a_dead_widget_does_nothing() {
        let idle = State::default();
        assert_eq!(action(5, idle, false), Act::None);
        assert_eq!(action(0xf, idle, false), Act::None);
    }

    #[test]
    fn captions_group_the_widgets_the_way_the_dispatch_does() {
        // Twelve strips, consecutive, with 5..9 and 15..24 sharing.
        let groups: Vec<Option<usize>> = (0..WIDGETS).map(caption).collect();
        assert_eq!(groups[0], Some(0));
        assert_eq!(groups[4], Some(4));
        for group in &groups[5..=9] {
            assert_eq!(*group, Some(5));
        }
        assert_eq!(groups[10], Some(6));
        assert_eq!(groups[14], Some(10));
        for group in &groups[0xf..=0x18] {
            assert_eq!(*group, Some(11));
        }
        assert_eq!(caption(WIDGETS), None);
    }

    #[test]
    fn the_auto_animation_speeds_up_with_the_playback_rate() {
        // At rate index 0 a frame lasts 1000 ms; at index 4, 200 ms.
        assert_eq!(auto_frame(0, 0), AUTO_FIRST_RECORD);
        assert_eq!(auto_frame(999, 0), AUTO_FIRST_RECORD);
        assert_eq!(auto_frame(1000, 0), AUTO_FIRST_RECORD + 1);
        assert_eq!(auto_frame(200, 4), AUTO_FIRST_RECORD + 1);
        // And it wraps after thirteen.
        assert_eq!(auto_frame(13_000, 0), AUTO_FIRST_RECORD);
    }
}

/// The affection gauge the bar carries, and the geometry that sizes it.
///
/// # What it shows
///
/// Two counters, `001` and `002`, out of the save's variable store —
/// [`crate::install::feeling`] is where they come from and what they mean.
/// `FUN_10026050` reads exactly those two by name through host slot `+8`,
/// keeps them as floats, and derives a signed **lead** for each side:
///
/// ```text
/// lead_first  = (first  - second) * 2.5      this+0x48
/// lead_second = (second - first ) * 2.5      this+0x4c
/// ```
///
/// so the two are always negatives of each other and only their difference is
/// ever drawn — the gauge shows which counter is ahead and by how much, never
/// either total.
///
/// # Reading the constants
///
/// The scale is `2.5`, and getting that right needed care: Ghidra renders
/// `_DAT_1003d858` as `(float)`, and read as a float its bytes are `0.0` —
/// a value that is self-consistent (a zero-width gauge) and wrong. It is a
/// **double**, narrowed at the use site. The same applies to [`BIAS`] and
/// [`FLOOR`]. The `.data` constants next to them really are floats.
///
/// # The three pieces
///
/// `FUN_10026540` sizes three sprites and sets a visibility flag for each.
/// Which one is up is decided by the same biased comparison each time: a
/// side's piece appears only once its lead passes [`FLOOR`] once [`BIAS`] is
/// added, which needs a lead of `(417.0 - 208.5) / 2.5`, a little over 83
/// points. Below that neither side's piece is up and the third, level piece is
/// drawn instead — so in ordinary play, where the two counters run within a
/// few points of each other, the level piece is the one on screen.
///
/// The destination rectangles below are `FUN_10026540`'s, exactly. The
/// **source** rectangles are not: the original builds them through four
/// chained calls on the object at `this+0x1c` whose vtable this module has not
/// identified, so which of those four values is x, y, width and height is
/// **not recovered**, and the pieces are therefore not composed yet.
pub mod gauge {
    /// `_DAT_1003d858`, a double: points of lead to pixels.
    pub const SCALE: f32 = 2.5;
    /// `_DAT_1003d878`, a double: added to a lead before it is tested.
    pub const BIAS: f32 = 208.5;
    /// `_DAT_1003d870`, a double: a biased lead at or below this hides the
    /// side's piece. The comparison is `<=`, so exactly `FLOOR` is hidden.
    pub const FLOOR: f32 = 417.0;
    /// `DAT_1004d0b0` / `DAT_1004d0c8`, the longest either side's bar is drawn.
    pub const MAX_LEN: f32 = 485.0;
    /// `DAT_1004d0a8`, where the first side's bar starts.
    pub const LEFT: f32 = 188.0;
    /// `DAT_1004d0c0`, the second side's bar's offset from its computed end.
    pub const RIGHT_OFFSET: f32 = 118.0;
    /// `DAT_1004d0ac` / `DAT_1004d0c4`, the bars' top edge.
    pub const TOP: f32 = 9.0;
    /// `DAT_1004d0b4` / `DAT_1004d0cc`, their height before the `+1`.
    pub const HEIGHT: f32 = 9.0;
    /// `_DAT_1003d860`, a double: the level piece's fixed width.
    pub const LEVEL_WIDTH: f32 = 418.0;
    /// `_DAT_10039738`, a double: taken off both origins.
    const HALF: f32 = 0.5;
    /// `_DAT_10039798`, a double: added to both extents.
    const ONE: f32 = 1.0;

    /// A destination rectangle, before the display scale is applied.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Rect {
        pub x: f32,
        pub y: f32,
        pub w: f32,
        pub h: f32,
    }

    /// Which of the three pieces is up, and where each goes.
    #[derive(Debug, Clone, Copy, PartialEq, Default)]
    pub struct Pieces {
        /// `this+0x98`, up under `this+0xc4`: the second counter is far ahead.
        pub second: Option<Rect>,
        /// `this+0x9c`, up under `this+0xc8`: the first counter is far ahead.
        pub first: Option<Rect>,
        /// `this+0xa0`, up under `this+0xcc`: neither is, which is the usual
        /// case.
        pub level: Option<Rect>,
    }

    /// The lead each side has, scaled: `(first, second)`.
    pub fn leads(first: i32, second: i32) -> (f32, f32) {
        let d = (first - second) as f32 * SCALE;
        (d, -d)
    }

    /// Sizes the three pieces from the two counters, as `FUN_10026540` does.
    pub fn pieces(first: i32, second: i32) -> Pieces {
        let (lead_first, lead_second) = leads(first, second);

        // `this+0x98`: driven by the second side's lead, anchored at LEFT.
        let second_piece = bar(lead_second).map(|len| Rect {
            x: LEFT - HALF,
            y: TOP - HALF,
            w: len + ONE,
            h: HEIGHT + ONE,
        });

        // `this+0x9c`: the original computes the remaining width *before*
        // clamping the length, so a lead past MAX_LEN pushes the origin
        // negative rather than pinning it. That ordering is reproduced.
        let first_piece = bar(lead_first).map(|len| {
            let remaining = MAX_LEN - (lead_first + BIAS);
            Rect {
                x: remaining + RIGHT_OFFSET - HALF,
                y: TOP - HALF,
                w: len + ONE,
                h: HEIGHT + ONE,
            }
        });

        // `this+0xa0`: up only while *neither* side's piece is.
        let level = (second_piece.is_none() && first_piece.is_none()).then_some(Rect {
            x: LEFT - HALF,
            y: TOP - HALF,
            w: LEVEL_WIDTH,
            h: HEIGHT + ONE,
        });

        Pieces {
            second: second_piece,
            first: first_piece,
            level,
        }
    }

    /// One side's bar length, or `None` while its piece is down.
    ///
    /// `_DAT_10039758` is `0.0`, so the "below zero" clamp only fires for a
    /// negative bias-adjusted length, which the `FLOOR` test has already
    /// excluded — it is transcribed because it is there, not because it can
    /// be reached.
    fn bar(lead: f32) -> Option<f32> {
        let len = lead + BIAS;
        if len <= FLOOR {
            return None;
        }
        Some(len.clamp(0.0, MAX_LEN))
    }
}

#[cfg(test)]
mod gauge_tests {
    use super::gauge::*;

    /// The two leads are one number and its negation, so the gauge can only
    /// ever show a difference.
    #[test]
    fn the_two_leads_are_opposite() {
        assert_eq!(leads(89, 78), (27.5, -27.5));
        assert_eq!(leads(78, 89), (-27.5, 27.5));
        assert_eq!(leads(40, 40), (0.0, -0.0));
    }

    /// The counters in a real save run a few points apart, which is nowhere
    /// near the threshold, so the level piece is what is on screen.
    #[test]
    fn ordinary_counters_show_the_level_piece_alone() {
        let p = pieces(69, 62);
        assert!(p.first.is_none());
        assert!(p.second.is_none());
        assert_eq!(
            p.level,
            Some(Rect {
                x: 187.5,
                y: 8.5,
                w: 418.0,
                h: 10.0
            })
        );
    }

    /// A lead has to clear `(FLOOR - BIAS) / SCALE` before its side's piece
    /// appears at all — 83.4 points, which is most of the game's range.
    #[test]
    fn a_side_needs_a_large_lead_before_its_piece_appears() {
        assert!(pieces(83, 0).first.is_none());
        assert!(pieces(84, 0).first.is_some());
        assert!(pieces(0, 84).second.is_some());
    }

    /// The moment one side's piece comes up the level piece goes down: the
    /// three are never on together.
    #[test]
    fn the_level_piece_yields_to_a_leading_side() {
        let p = pieces(120, 0);
        assert!(p.first.is_some());
        assert!(p.level.is_none());
        assert!(p.second.is_none());
    }

    /// The bar stops growing at MAX_LEN.
    #[test]
    fn a_runaway_lead_is_clamped_to_the_bar_length() {
        let p = pieces(0, 10_000).second.unwrap();
        assert_eq!(p.w, MAX_LEN + 1.0);
    }

    /// The `<=` in the visibility test means exactly FLOOR is still hidden.
    #[test]
    fn a_lead_landing_exactly_on_the_floor_stays_hidden() {
        let exact = (FLOOR - BIAS) / SCALE;
        assert_eq!(exact, 83.4);
        // 83.4 is not reachable from integer counters, so the boundary is
        // checked through the same arithmetic the real values take.
        assert!(pieces(83, 0).first.is_none());
    }
}
