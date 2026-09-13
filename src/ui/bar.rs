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
//! +0x0c load and lay out   +0x24 dirty the rate readout (FUN_10027210)
//! +0x10 re-place on resize +0x28 set widget 2's latch    (FUN_10027230)
//! +0x14 draw               +0x2c draw the play/pause widget      (FUN_100258f0)
//! +0x18 -                  +0x30 widget 2's action               (FUN_10025b90)
//! +0x1c set host pointers  +0x34 widget 0's action               (FUN_10025cf0)
//!                          +0x38/+0x3c/+0x40/+0x44 lifetime
//! ```
//!
//! The engine's own pointer is `engine + 0x330`, and the eleven slots it calls
//! on it — `+0x04`, `+0x08`, `+0x0c`, `+0x1c`, `+0x20`, `+0x24`, `+0x28`,
//! `+0x2c`, `+0x30`, `+0x34`, `+0x38` — all land inside this vtable, which is
//! what confirms the member. **`+0x14`, the draw, is not among them**, and no
//! call site for it was found: `FUN_10024ca0` is referenced only from the
//! vtable slot, and a raw scan of `.text` finds no `CALL [reg+0x14]` outside
//! the CRT. So what invokes the draw is **not recovered**. That it runs
//! whether or not the bar is dropped down is read from the function itself,
//! which tests `this+0xbc` and host `+0x140` internally and draws two sprites
//! past both — see [`Bar::compose_faded`].
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
//! 15..24    the replay indicator's transparency        FUN_10026ed0
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
use crate::ui::screen::{Cut, Error, Resolution, Screen, WidgetState};
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
    /// The alpha the gauge bed and the gauge's three pieces carry, which is not
    /// always the bar's own.
    ///
    /// `FUN_10025690` sets one ARGB on every sprite the bar owns **except**
    /// `this+0x80` and `this+0x98..0xa0` — the bed and the three pieces — which
    /// it skips whenever host `+0x154` answers non-zero. Skipping is not the
    /// same as setting them opaque: they keep whatever they last held. So a
    /// gauge raised while the bar is up stays on screen at full alpha after the
    /// bar has faded away, and one raised while the bar is already gone is
    /// pinned at nothing and does not appear at all.
    gauge_alpha: u8,
}

impl Fade {
    /// One frame of the fade, given the clock and whether the pointer is over
    /// the strip at all.
    pub fn update(&mut self, now_ms: u32, over_strip: bool, gauge_raised: bool) {
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
        // The one place `FUN_10025690`'s exception lands: while the gauge is
        // raised it is not called for the bed or the pieces at all, so their
        // alpha stops following the bar's.
        if !gauge_raised {
            self.gauge_alpha = self.alpha;
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

    /// The alpha every one of the bar's sprites is modulated by, bar the
    /// gauge's — see [`Fade::gauge_alpha`].
    pub fn alpha(&self) -> u8 {
        self.alpha
    }

    /// The alpha the gauge bed and the gauge's three pieces are modulated by.
    pub fn gauge_alpha(&self) -> u8 {
        self.gauge_alpha
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
/// 3  an object `SystemInit` has no case for — not recovered
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
    ///
    /// The only code that is not "this script is over": `FUN_00425bf0`'s case 2
    /// routes it to case 6, which jumps to the choice this script raises — or,
    /// when it raises none, chases one across the scripts that follow. See
    /// [`crate::playback::stage::Stage::skip_target`].
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
    /// Set the replay-mode indicator's transparency, to the level the pressed
    /// cell stands for. See [`indicator`].
    ///
    /// Nothing is asked of the host: `FUN_10026ed0` keeps the level in the bar
    /// and applies it to the bar's own sprites, so [`Bar::press`] has already
    /// done it by the time this comes back. It is an [`Act`] so the caller
    /// still plays the click the original plays for any enabled widget.
    Transparency(usize),
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
    /// Host `+0x88`: `FUN_00427490`. Required by widgets 4 and 5..9.
    ///
    /// Two branches, and the second is the one that matters:
    ///
    /// ```text
    /// if (_GetSkipFlag@0() == 0) return host->+0x18(engine + 0x188);
    /// else                       return 1;
    /// ```
    ///
    /// Host `+0x18` is `FUN_00428770`, a lookup of the given path in the pack
    /// index the engine holds at `+0x3c`. `engine + 0x188` is **the script
    /// being played**: `FUN_00423a70` takes the next entry off the pending
    /// queue at `+0x154`, stores it there, and hands that same string to the
    /// timeline object's loader `FUN_00430d20`; `FUN_00425bf0` state 7 refills
    /// it from `_GetNextScriptFile@12` when a script chains.
    ///
    /// So while a script is playing the path resolves, `+0x18` answers
    /// non-zero, and `+0x88` is true **whatever the `Skip` setting says**. The
    /// setting is only a short circuit ahead of the lookup, not a gate on the
    /// speed row.
    pub skippable: bool,
    /// Host `+0x98`: playback is following a save's recorded answers.
    ///
    /// `FUN_0042bef0` returns the film object's `+0x1e0` and `FUN_0042bf10`
    /// (host `+0x94`) is the only thing that writes it — nothing in the
    /// executable names that member otherwise. Its two callers are both in the
    /// menu DLL: `FUN_1001dfe0`, a row of the replay screen's play-data list,
    /// passes 1, and `FUN_1001d380`, an ordinary load, passes 0. It is the same
    /// member an unanswered choice box consults to take the slot's own answer.
    ///
    /// On the bar it is what the whole right-hand box is for: it lights the
    /// slider, makes its ten cells pressable, and puts the `REPLAYMODE`
    /// indicator on screen.
    pub following_record: bool,
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
    /// The state a bar starts in over a script that is playing.
    ///
    /// One member comes from `Config.DAT`: widget 4 asks `_GetSuperSkipFlag@0`
    /// directly, and that export returns a key the settings screen writes.
    ///
    /// [`State::skippable`] does **not**. Host `+0x88` short-circuits on
    /// `_GetSkipFlag@0` and otherwise asks whether the playing script resolves
    /// in the packs, which it does — a bar only exists over a loaded script.
    /// So it is true here, and the speed widgets are live for every player
    /// rather than only for one who has turned `Skip` on. Everything else on
    /// [`State`] is a live member of the running engine and comes from the
    /// caller.
    pub fn from_config(config: &Config) -> State {
        State {
            skippable: true,
            super_skip: config.flag(Flag::SuperSkip),
            rate: SPEEDS[0],
            ..State::default()
        }
    }

    /// Applies host `+0x8c(index)`, the speed widgets' own call.
    ///
    /// Answers whether the clock has to be re-based, which is the part of
    /// `FUN_00424f90` the engine has to act on rather than store:
    ///
    /// * Pressing the rate that is already in force does nothing but re-store
    ///   the lit index. The original compares `index` against its `+0x504`
    ///   before anything else and, when they match, writes only `+0x508` — so
    ///   pressing 1x twice must not disturb a running clock.
    /// * Any other index folds the frames run so far into the base and
    ///   restarts from there (`FUN_00424910`, then `FUN_00424a10`), so the new
    ///   rate applies from now on and the frame never jumps backwards.
    /// * An index outside the table is clamped to 1x at index 0, not ignored:
    ///   `(param_1 < 0) || (4 < param_1)` stores `0x3f800000` and rewrites the
    ///   index to 0.
    ///
    /// The rate itself is [`SPEEDS`], which the original keeps as a float at
    /// `+0x538` and this engine keeps in [`State::rate`].
    pub fn set_speed(&mut self, index: usize) -> bool {
        let index = if index < SPEEDS.len() { index } else { 0 };
        let changed = index != self.speed;
        self.speed = index;
        self.rate = SPEEDS[index];
        changed
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
        0xf..=0x18 => state.following_record,
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
            if latched && !state.replay && !state.following_record {
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
        0xf..=0x18 => indicator::level_for(widget).map_or(Act::None, Act::Transparency),
        _ => Act::None,
    }
}

/// How widget 2's latch moves when it is pressed or time passes.
///
/// `FUN_10025b90` sets the latch on the first press but only when playback is
/// neither a replay nor a followed recording, and clears it on the second;
/// `FUN_10024100`
/// clears it once its frame argument passes [`RESTART_LATCH_FRAMES`].
pub fn restart_latch(latched: bool, state: State) -> bool {
    !latched && !state.replay && !state.following_record
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
    /// `this+0x8c`, the trough the ten transparency cells sit in, picked by
    /// host `+0x98` — the same question that makes them pressable. The dead
    /// one is grey and the live one a white-to-cyan gradient.
    pub const SLIDER_DEAD: usize = 69;
    pub const SLIDER_LIVE: usize = 70;
    /// `this+0x80`, a 538-wide strip drawn alongside the gauge.
    pub const GAUGE_BED: usize = 67;
    /// `this+0x94`, the `REPLAYMODE` indicator, drawn while host `+0x98` is
    /// set. Its destination is at y = 80, **below the 800x75 strip**, so it
    /// lands on the picture rather than on the bar and is not part of the
    /// strip's layer — [`Bar::indicator`] is where it comes out.
    pub const REPLAY_MODE: usize = 68;
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

/// The `REPLAYMODE` indicator, ready to draw. See [`Bar::indicator`].
pub struct Indicator {
    /// The art at display scale.
    pub art: Image,
    /// `(x, y, width, height)` in the same display space [`Bar::strip`] is in,
    /// whose origin is the picture's top-left corner. `y` is below the strip.
    pub dst: (i64, i64, u32, u32),
    /// What the ten cells set: `round(level * 25.0)`.
    pub alpha: u8,
}

/// The bar, loaded and ready to hit-test and draw.
pub struct Bar {
    screen: Screen,
    /// Widget 2's latch, and the frame it was set on.
    latch: Option<u32>,
    fade: Fade,
    /// `this+0xec`: how solid the replay-mode indicator is drawn, 0 to 10.
    /// See [`indicator`].
    transparency: usize,
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
            transparency: indicator::INITIAL_LEVEL,
        })
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    /// Reloads the strip's art at a new display size, keeping the drop-down and
    /// the latch where they are.
    ///
    /// The Option screen is one of the menus the bar itself opens, so the
    /// display mode can change underneath a bar that is already on screen.
    /// `FUN_10013470` picks the art set from the mode, and this screen is no
    /// exception to it. A size the packs have no art for leaves the strip as it
    /// was rather than losing the bar mid-script.
    pub fn set_resolution(
        &mut self,
        vfs: &crate::install::vfs::Vfs,
        dll: &[u8],
        resolution: Resolution,
    ) -> Result<(), Error> {
        if self.screen.resolution == resolution {
            return Ok(());
        }
        self.screen = Screen::load(vfs, dll, PATH, resolution)?;
        Ok(())
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
    pub fn point_at(
        &mut self,
        over: Option<(u32, u32)>,
        now_ms: u32,
        gauge_raised: bool,
    ) -> Option<usize> {
        self.fade.update(now_ms, over.is_some(), gauge_raised);
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
        // `FUN_10026ed0` stores the level in the bar itself and applies it to
        // the bar's own sprites, so this is the whole of what the press does.
        if let Act::Transparency(level) = act {
            self.transparency = level;
        }
        act
    }

    /// How solid the replay-mode indicator is drawn, 0 to 10.
    pub fn transparency(&self) -> usize {
        self.transparency
    }

    /// Whether anything the strip draws keeps an alpha of its own this frame.
    ///
    /// Only the raised gauge, which the fade skips. While it is showing, the
    /// strip cannot be drawn by modulating one texture — the fade has to be
    /// composited in, which is what [`Bar::compose_faded`] does.
    ///
    /// The rate readout at `this+0x88` is **not** here, though `FUN_10025690`
    /// does not name it either. See [`Bar::compose_faded`] for why it is left
    /// fading with the rest.
    pub fn pinned(&self, state: State) -> bool {
        state.gauge_raised
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
                out.push(if state.following_record {
                    record::SLIDER_LIVE
                } else {
                    record::SLIDER_DEAD
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
        self.states_of(&self.records(hovered, state, elapsed_ms))
    }

    /// As [`Bar::states`], for a record list already in hand.
    fn states_of(&self, records: &[usize]) -> Vec<WidgetState> {
        let mut states = vec![WidgetState::Resting; WIDGETS];
        for record in records {
            if *record < WIDGETS {
                states[*record] = WidgetState::Active;
            } else {
                states.push(self.extra(*record));
            }
        }
        states
    }

    /// This frame's records split by whether the fade reaches them.
    ///
    /// The second list is empty unless the gauge is raised, which is the only
    /// thing that takes a sprite out of `FUN_10025690`'s reach — and when it is
    /// raised the bed goes with the gauge, since the original skips that too.
    fn split(
        &self,
        hovered: Option<usize>,
        state: State,
        elapsed_ms: u32,
    ) -> (Vec<usize>, Vec<usize>) {
        let mut faded = self.records(hovered, state, elapsed_ms);
        let mut pinned = Vec::new();
        if state.gauge_raised {
            faded.retain(|r| *r != record::GAUGE_BED);
            pinned.push(record::GAUGE_BED);
        }
        (faded, pinned)
    }

    /// The sprites the bar sizes itself instead of taking whole from a record,
    /// split the way [`Bar::split`] splits the records: the gauge's three
    /// pieces, which stop fading once the gauge is raised, and the transparency
    /// knob, which does not.
    fn cuts(&self, state: State) -> (Vec<Cut>, Vec<Cut>) {
        let mut faded = self.gauge_cuts(state);
        let pinned = if state.gauge_raised {
            std::mem::take(&mut faded)
        } else {
            Vec::new()
        };
        // `this+0x90`. `FUN_10024ca0` draws it last of all, under the same
        // three tests the trough is under plus the bar being up.
        if !state.hidden && !state.message && state.following_record {
            if let Some((src, dst)) = indicator::knob(self.transparency) {
                faded.push(cut(src, dst));
            }
        }
        (faded, pinned)
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
    ///
    /// Two more are absent from `FUN_10025690`'s list altogether — the rate
    /// readout at `this+0x88` and the `REPLAYMODE` indicator at `this+0x94` —
    /// and `FUN_10024ca0` draws both past the `this+0xbc` and host `+0x140`
    /// tests, which is a branch target and not a reading of indentation. Taken
    /// at face value that would leave one rate button on the picture for as
    /// long as the rate is 2.0 or more.
    ///
    /// **The readout is faded with the rest here anyway.** The step that would
    /// make the consequence follow — that `FUN_10024ca0` runs at all while the
    /// bar is down — is **not recovered**: nothing found calls the draw, so
    /// whether it is reached in that state is an inference from the function
    /// testing `this+0xbc` itself. Against that inference stands the game as
    /// played, where no such button is on screen. The indicator is the one
    /// exception, and only because the ten-cell slider that sets its
    /// transparency is evidence in its own right that it is meant to be seen
    /// without the bar — there would be nothing to adjust otherwise.
    pub fn compose(&self, hovered: Option<usize>, state: State, elapsed_ms: u32) -> Image {
        let records = self.records(hovered, state, elapsed_ms);
        let (faded, pinned) = self.cuts(state);
        let cuts: Vec<Cut> = pinned.into_iter().chain(faded).collect();
        self.screen
            .compose_layer_cuts(&self.states_of(&records), &cuts)
    }

    /// The affection gauge's pieces, as cuts of the chip sheet.
    ///
    /// Not part of [`Bar::records`], because they are not chip records:
    /// `FUN_10026540` sizes them itself from the two counters, and
    /// `FUN_10024ca0` draws whichever are up straight after the bed record
    /// they sit in — under the same two tests the bed itself is under, which is
    /// why this asks [`Bar::records`] for the bed rather than repeating them.
    ///
    /// Empty while the counters are unknown: the pieces' visibility flags are
    /// only ever set by `FUN_10026540`, which nothing has run.
    pub fn gauge_cuts(&self, state: State) -> Vec<Cut> {
        let Some((first, second)) = state.gauge else {
            return Vec::new();
        };
        if !self.records(None, state, 0).contains(&record::GAUGE_BED) {
            return Vec::new();
        }
        gauge::pieces(first, second)
            .drawn()
            .map(|piece| cut(piece.src, piece.dst))
            .collect()
    }

    /// The `REPLAYMODE` indicator, which is not part of the strip's layer.
    ///
    /// `this+0x94` goes at `(697, 80) 97x19` in the strip's own space — below
    /// the 800x75 strip, so it lands on the picture. `FUN_10024ca0` draws it
    /// outside the test that gates every widget, and it is not in
    /// `FUN_10025690`'s list, so it neither waits for the bar to drop down nor
    /// fades with it. The one thing it carries is the transparency the ten
    /// cells set.
    ///
    /// Returns the art at display scale, where it goes in the same display
    /// space [`Bar::strip`] is in, and the alpha to draw it at.
    pub fn indicator(&self, state: State) -> Option<Indicator> {
        if !state.following_record {
            return None;
        }
        let n = record::REPLAY_MODE.checked_sub(WIDGETS)?;
        let widget = self.screen.atlas().extras.get(n)?;
        let (art, dst) = self.screen.cut_widget(widget);
        Some(Indicator {
            art,
            dst,
            alpha: indicator::alpha(self.transparency),
        })
    }

    /// As [`Bar::compose`], with the fade already multiplied in.
    ///
    /// The SDL path does not use this: it caches the layer on its record list
    /// and modulates the texture's alpha instead, so a ramp does not
    /// recomposite the strip every frame. This is for the headless path, where
    /// there is one image and no texture to modulate.
    pub fn compose_faded(&self, hovered: Option<usize>, state: State, elapsed_ms: u32) -> Image {
        let (records, pinned) = self.split(hovered, state, elapsed_ms);
        let (faded_cuts, pinned_cuts) = self.cuts(state);
        let mut layer = self
            .screen
            .compose_layer_cuts(&self.states_of(&records), &faded_cuts);
        modulate(&mut layer, self.fade.alpha());
        if !pinned.is_empty() || !pinned_cuts.is_empty() {
            let mut over = self
                .screen
                .compose_sprites(&self.states_of(&pinned), &pinned_cuts);
            // The gauge is the only part whose alpha the raise pins; the rate
            // readout is simply never in `FUN_10025690`'s list, so it keeps the
            // opaque colour `FUN_10022650` gave it.
            modulate(&mut over, self.fade.gauge_alpha());
            let (w, h) = (over.width, over.height);
            layer.blit_scaled(&over, (0, 0, w, h), (0, 0, w, h));
        }
        layer
    }
}

/// A [`Cut`] from a source and destination rectangle in the bar's own units.
fn cut(src: gauge::Rect, dst: gauge::Rect) -> Cut {
    Cut {
        src: (src.x, src.y, src.w, src.h),
        dst: (dst.x, dst.y, dst.w, dst.h),
    }
}

/// Multiplies a layer's alpha through, for the headless path that has no
/// texture to modulate.
fn modulate(layer: &mut Image, alpha: u8) {
    if alpha == 255 {
        return;
    }
    let alpha = u32::from(alpha);
    for px in layer.rgba.as_chunks_mut::<4>().0 {
        px[3] = (u32::from(px[3]) * alpha / 255) as u8;
    }
}

#[cfg(test)]
mod tests {

    /// The speed row and the skip button are live for every player, not only
    /// one who has turned `Skip` on. Host `+0x88` short-circuits on
    /// `_GetSkipFlag@0` and otherwise asks whether the playing script resolves
    /// in the packs, which over a loaded script it does.
    #[test]
    fn the_speed_row_is_live_whatever_the_skip_setting_says() {
        let config = Config::parse_text("[Skip]=\"0\"\n[SuperSkip]=\"0\"\n");
        let state = State::from_config(&config);
        assert!(state.skippable, "+0x88 is true over a playing script");
        for widget in 5..=9 {
            assert!(
                enabled(widget, state),
                "speed widget {widget} must be live with Skip off"
            );
        }
    }

    /// Pressing the rate already in force must not disturb the clock:
    /// `FUN_00424f90` compares against `+0x504` before doing anything else.
    #[test]
    fn pressing_the_rate_already_in_force_does_not_rebase_the_clock() {
        let mut state = State::default();
        assert!(!state.set_speed(0), "1x is the rate a session starts at");
        assert!(state.set_speed(3), "12x is a change");
        assert!(!state.set_speed(3), "12x again is not");
        assert_eq!(state.speed, 3);
    }

    /// The index and the rate move together, because the bar draws from one
    /// and the clock runs on the other.
    #[test]
    fn setting_a_speed_sets_the_rate_beside_it() {
        let mut state = State::default();
        for (index, rate) in SPEEDS.iter().enumerate() {
            state.set_speed(index);
            assert_eq!(state.speed, index);
            assert_eq!(state.rate, *rate);
        }
    }

    /// Out of range is clamped to 1x at index 0, not ignored: the original
    /// stores `0x3f800000` and rewrites the index.
    #[test]
    fn an_out_of_range_speed_falls_back_to_1x() {
        let mut state = State::default();
        state.set_speed(2);
        state.set_speed(SPEEDS.len());
        assert_eq!(state.speed, 0);
        assert_eq!(state.rate, 1.0);
    }
    use super::*;

    /// A state in which everything the bar can ask about is available.
    fn live() -> State {
        State {
            skippable: true,
            following_record: true,
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
        fade.update(0, false, false);
        fade.update(10_000, false, false);
        assert!(!fade.drawn());

        // The pointer arriving makes it draw at once, at alpha 0, and it ramps.
        fade.update(0, true, false);
        assert!(fade.drawn());
        assert_eq!(fade.alpha(), 0);
        fade.update(FADE_IN_MS / 2, true, false);
        assert_eq!(fade.alpha(), 127);
        fade.update(FADE_IN_MS, true, false);
        assert_eq!(fade.alpha(), 255);
    }

    /// `FUN_10025690` skips the gauge while it is raised, which is not the
    /// same as holding it up: what it keeps is the alpha it had when the raise
    /// happened, so a gauge raised over a bar that is already gone stays gone.
    #[test]
    fn a_raised_gauge_holds_the_alpha_it_was_raised_at() {
        let mut fade = Fade::default();
        fade.update(0, true, false);
        fade.update(FADE_IN_MS, true, false);
        assert_eq!(fade.gauge_alpha(), 255);

        // Raised with the bar up, the gauge survives the whole ramp out.
        fade.update(1_000, false, true);
        fade.update(1_000 + FADE_OUT_MS, false, true);
        assert_eq!(fade.alpha(), 0);
        assert_eq!(fade.gauge_alpha(), 255);

        // Lowering lets it catch up, and raising it again from there pins it at
        // nothing.
        fade.update(3_000, false, false);
        assert_eq!(fade.gauge_alpha(), 0);
        fade.update(4_000, false, true);
        assert_eq!(fade.gauge_alpha(), 0);
    }

    #[test]
    fn the_pointer_leaving_ramps_out_and_then_takes_the_bar_away() {
        let mut fade = Fade::default();
        fade.update(0, true, false);
        fade.update(FADE_IN_MS, true, false);
        assert_eq!(fade.alpha(), 255);

        // The ramp out is over the longer of the two windows, and the bar keeps
        // drawing all the way through it.
        fade.update(1_000, false, false);
        assert!(fade.drawn());
        fade.update(1_000 + FADE_OUT_MS / 2, false, false);
        assert_eq!(fade.alpha(), 128);
        assert!(fade.drawn());
        fade.update(1_000 + FADE_OUT_MS, false, false);
        assert_eq!(fade.alpha(), 0);
        // It is still "up" on the frame the alpha hits zero; the next frame is
        // the one that takes it away, which is the order `FUN_10024100` does it
        // in — the test comes before the assignment.
        fade.update(3_000, false, false);
        assert!(!fade.drawn());
    }

    #[test]
    fn a_reversal_mid_ramp_does_not_restart_the_clock() {
        // `FUN_100255c0` clears its start tick only when a ramp completes, so
        // flipping direction part way through keeps the old start and the alpha
        // jumps. Shipped behaviour, reproduced rather than smoothed over.
        let mut fade = Fade::default();
        fade.update(0, true, false);
        fade.update(FADE_IN_MS / 2, true, false);
        assert_eq!(fade.alpha(), 127);
        // Now leave. The out ramp measures from tick 0, not from now, so half
        // of FADE_IN_MS into a 1000ms out ramp is barely any fall at all.
        fade.update(FADE_IN_MS / 2, false, false);
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
        fade.update(0, true, false);
        fade.update(FADE_IN_MS, true, false);
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

    /// With host `+0x88` false — no script loaded — the skip button and the
    /// speed row go dead together, and nothing else does.
    #[test]
    fn the_speed_row_and_skip_go_dead_together_without_a_script() {
        let no_script = State {
            skippable: false,
            ..live()
        };
        for widget in 4..=9 {
            assert!(!enabled(widget, no_script));
        }
        assert!(enabled(3, no_script));
    }

    #[test]
    fn the_transparency_cells_need_a_recording_to_be_following() {
        let idle = State {
            following_record: false,
            ..live()
        };
        for widget in 0xf..=0x18 {
            assert!(!enabled(widget, idle));
            assert!(enabled(widget, live()));
        }
        assert_eq!(action(0xf, live(), false), Act::Transparency(0));
        assert_eq!(action(0x18, live(), false), Act::Transparency(10));
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
        // here is not following a recording.
        let plain = State {
            following_record: false,
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
        // ...so the second press restarts again rather than jumping to the end.
        assert_eq!(action(2, replay, true), Act::Seek(Seek::RESTART));
    }

    #[test]
    fn widget_three_asks_for_the_same_code_as_a_second_press_of_widget_two() {
        let plain = State {
            following_record: false,
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

/// The replay-mode indicator, and the ten-cell slider that sets how solid it is.
///
/// # What the box on the right is
///
/// Widgets 15 to 24 are ten 12x17 cells in a row at x 676..796, sitting inside
/// a 124x19 trough at `this+0x8c`. The trough has two sprites and host `+0x98`
/// picks between them: grey while nothing is following a recording, a
/// white-to-cyan gradient while something is. The game's own caption for all
/// ten cells — record 64, the twelfth caption strip — reads
/// `Change transparency of replay mode indicator`.
///
/// The indicator itself is the `REPLAYMODE` sprite at `this+0x94`, drawn while
/// the same `+0x98` is set. Its destination is `(697, 80) 97x19`, **below** the
/// 800x75 strip, so it sits on the picture and stays there whether or not the
/// bar is dropped down — `FUN_10024ca0` draws it outside the `this+0xbc` test
/// that gates every widget, and `FUN_10025690` never touches its colour, so it
/// does not fade with the bar either. What it does carry is this setting.
///
/// # The level
///
/// `FUN_10026ed0` is the whole of it. A press on cell `c` stores a level in
/// `this+0xec`, sets the indicator's alpha to `round(level * 25.0)`, and calls
/// `FUN_10027030` to re-place the knob at `this+0x90`:
///
/// ```text
/// widget   15  16  17  18  19  20  21  22  23  24
/// level     0   2   3   4   5   6   7   8   9  10
/// ```
///
/// **Level 1 is not reachable**, and that is not a transcription slip: cell 0
/// stores 0 and cell 1 stores 2, and `FUN_10027030`'s switch has no case 1
/// either — it leaves its local uninitialised, which is what a level of 1 would
/// place the knob from. `FUN_10023d50` starts the bar at 10, so the indicator
/// is opaque until the player moves it.
///
/// Nothing saves the level. It is a member of the bar, gone with the bar.
pub mod indicator {
    use super::gauge::Rect;

    /// `DAT_1004d4c8`, where the knob sits at level 8 — the middle of the run.
    pub const BASE_X: f32 = 760.0;
    /// `DAT_1004d4cc` / `DAT_1004d4d0` / `DAT_1004d4d4`: the knob's top edge
    /// and its size, one cell.
    pub const TOP: f32 = 5.0;
    pub const WIDTH: f32 = 12.0;
    pub const HEIGHT: f32 = 17.0;
    /// `DAT_1004d4d8` / `DAT_1004d4dc`, the knob's art in `MenuBar_Chip.png`.
    /// `FUN_10022650` sets that source once and nothing moves it: the knob is
    /// one sprite that slides, not a record.
    pub const SRC_X: f32 = 1.0;
    pub const SRC_Y: f32 = 379.0;
    /// `_DAT_1003d880`, a float and not a double — `flds`: the alpha one level
    /// is worth. Ten levels reach 250, not 255, which is the shipped ceiling.
    pub const ALPHA_STEP: f32 = 25.0;
    /// `FUN_10023d50`'s `*(this+0xec) = 10`.
    pub const INITIAL_LEVEL: usize = 10;
    /// The first of the ten cells.
    pub const FIRST_WIDGET: usize = 0xf;
    pub const CELLS: usize = 10;
    /// `_DAT_10039738` and `_DAT_10039798`, the half-pixel inset and the `+1`
    /// every sprite on the bar gets.
    const HALF: f32 = 0.5;
    const ONE: f32 = 1.0;

    /// The level a press on `widget` stores, or `None` for a widget that is not
    /// one of the ten cells.
    pub fn level_for(widget: usize) -> Option<usize> {
        let cell = widget.checked_sub(FIRST_WIDGET).filter(|c| *c < CELLS)?;
        // Cell 0 stores 0 and cell 1 stores 2, so the run skips 1.
        Some(if cell == 0 { 0 } else { cell + 1 })
    }

    /// The indicator's alpha at `level`, as `FUN_10026ed0` computes it.
    pub fn alpha(level: usize) -> u8 {
        (level as f32 * ALPHA_STEP).round().clamp(0.0, 255.0) as u8
    }

    /// Where the knob goes at `level`, cut from the sheet.
    ///
    /// `None` for the unreachable level 1, whose case `FUN_10027030` does not
    /// have: the original would place the sprite from an uninitialised local,
    /// so there is no rectangle to transcribe.
    pub fn knob(level: usize) -> Option<(Rect, Rect)> {
        if level == 1 || level > 10 {
            return None;
        }
        // Cases 2..10 are `BASE_X - WIDTH * n` with n counting down from 6 to
        // -2, which is `BASE_X + (level - 8) * WIDTH`; case 0's multiplier is
        // the double 7.0, one short of the 8.0 that pattern would give it,
        // because the run has no level 1 to occupy the step in between.
        let steps = if level == 0 { -7.0 } else { level as f32 - 8.0 };
        let src = Rect {
            x: SRC_X,
            y: SRC_Y,
            w: WIDTH,
            h: HEIGHT,
        };
        let dst = Rect {
            x: BASE_X + steps * WIDTH - HALF,
            y: TOP - HALF,
            w: WIDTH + ONE,
            h: HEIGHT + ONE,
        };
        Some((src, dst))
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
/// # Where the art comes from
///
/// All three pieces cut `MenuBar_Chip.png`, which is the texture the bar keeps
/// at `this+0x1c` — `FUN_10023d50` loads `System/MenuBar/MenuBar.png` into
/// `+0x20` and `System/MenuBar/MenuBar_Chip.png` into `+0x24`, and
/// `FUN_10023aa0` renders those two into the textures at `+0x18` and `+0x1c`
/// in that order. The sheet carries one 9-pixel-tall strip per piece: the
/// first counter's bar at y 57 and the second's at y 369, each 485 long and
/// each a flat colour — orange for the first, green for the second — and at
/// y 399 the level strip, 798 long, green until x 370 and orange from x 420
/// with the blend between them centred on 395.
///
/// So the level piece is not a neutral bar. It is a **window** 417 wide onto
/// that strip, and its origin slides by [`SCALE`] pixels for every point of
/// lead, which walks the green-to-orange edge across the gauge: at a tie the
/// edge sits one and a half pixels left of centre, and by the time either side
/// is far enough ahead for its own piece to take over the edge has reached the
/// end. That slide is what the gauge shows in ordinary play.
///
/// # Reading the rectangles out
///
/// `FUN_10026540` sets each piece's destination through `DX9Sprite2D` slot
/// `+0xc` and its source through slot `+0x1c`, and the source goes in as four
/// separate calls on the texture — `DX9Texture` slot `+8` is `x / width` and
/// slot `+0xc` is `y / height`. Ghidra renders those four as a chain of
/// `float10` results, in which the order is lost; the disassembly's push order
/// is what gives it, and it is self-checking, since the two `/ width` values
/// have to pair with each other and the two `/ height` values with each other.
///
/// The source rectangles are a pixel smaller than their destinations in each
/// axis, and the destinations start half a pixel back, which is the half-texel
/// offset every other sprite the bar draws gets as well.
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
    /// `_DAT_1003d860`, a double: the level piece's fixed width on screen.
    pub const LEVEL_WIDTH: f32 = 418.0;
    /// `DAT_1004d0bc`, the second side's strip in the sheet.
    pub const SECOND_SRC_Y: f32 = 369.0;
    /// `DAT_1004d0b8`, added to the second side's source origin. Its bar is
    /// cut from the **far** end of the strip, so the origin walks right as the
    /// length shrinks and the art's own right edge stays put.
    pub const SECOND_SRC_END: f32 = 1.0;
    /// `DAT_1004d0d0` / `DAT_1004d0d4`, the first side's strip in the sheet.
    /// This one is cut from its left edge, so the origin is fixed.
    pub const FIRST_SRC_X: f32 = 1.0;
    pub const FIRST_SRC_Y: f32 = 57.0;
    /// `DAT_1004d4f4`, the level strip in the sheet.
    pub const LEVEL_SRC_Y: f32 = 399.0;
    /// `_DAT_1003d868`, a float here and not a double: how much of the level
    /// strip the window shows, one pixel under [`LEVEL_WIDTH`].
    pub const LEVEL_SRC_WIDTH: f32 = 417.0;
    /// `DAT_1004d4e0` and `DAT_1004d4f0`, which the level piece's source origin
    /// is [`LEFT`] less the first and plus the second, before the lead slides
    /// it.
    pub const LEVEL_SRC_BACK: f32 = 1.0;
    pub const LEVEL_SRC_FORWARD: f32 = 1.0;
    /// `DAT_1004d0b4` / `DAT_1004d0cc` / `DAT_1004d4ec`: every piece cuts nine
    /// rows, which is [`HEIGHT`] before the `+1` its destination gets.
    pub const SRC_HEIGHT: f32 = HEIGHT;
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

    /// One piece: where its art is cut from, and where it goes.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Piece {
        /// In `MenuBar_Chip.png`'s own pixels.
        pub src: Rect,
        /// In the strip's 800x75 space, before the display scale.
        pub dst: Rect,
    }

    /// Which of the three pieces is up, and where each goes.
    ///
    /// `FUN_10024ca0` draws them in this order, which is the order the fields
    /// are in: the flags it walks are `this+0xc4`, `+0xc8` and `+0xcc`.
    #[derive(Debug, Clone, Copy, PartialEq, Default)]
    pub struct Pieces {
        /// `this+0x98`, up under `this+0xc4`: the second counter is far ahead.
        pub second: Option<Piece>,
        /// `this+0x9c`, up under `this+0xc8`: the first counter is far ahead.
        pub first: Option<Piece>,
        /// `this+0xa0`, up under `this+0xcc`: neither is, which is the usual
        /// case.
        pub level: Option<Piece>,
    }

    impl Pieces {
        /// The pieces that are up, in the order the original draws them.
        pub fn drawn(&self) -> impl Iterator<Item = Piece> {
            [self.second, self.first, self.level].into_iter().flatten()
        }
    }

    /// The lead each side has, scaled: `(first, second)`.
    pub fn leads(first: i32, second: i32) -> (f32, f32) {
        let d = (first - second) as f32 * SCALE;
        (d, -d)
    }

    /// Sizes the three pieces from the two counters, as `FUN_10026540` does.
    pub fn pieces(first: i32, second: i32) -> Pieces {
        let (lead_first, lead_second) = leads(first, second);

        // `this+0x98`: driven by the second side's lead, anchored at LEFT, and
        // cut from the far end of its strip.
        let second_piece = bar(lead_second).map(|len| Piece {
            src: Rect {
                x: (MAX_LEN - len) + SECOND_SRC_END,
                y: SECOND_SRC_Y,
                w: len,
                h: SRC_HEIGHT,
            },
            dst: Rect {
                x: LEFT - HALF,
                y: TOP - HALF,
                w: len + ONE,
                h: HEIGHT + ONE,
            },
        });

        // `this+0x9c`: the original computes the remaining width *before*
        // clamping the length, so a lead past MAX_LEN pushes the origin
        // negative rather than pinning it. That ordering is reproduced.
        let first_piece = bar(lead_first).map(|len| {
            let remaining = MAX_LEN - (lead_first + BIAS);
            Piece {
                src: Rect {
                    x: FIRST_SRC_X,
                    y: FIRST_SRC_Y,
                    w: len,
                    h: SRC_HEIGHT,
                },
                dst: Rect {
                    x: remaining + RIGHT_OFFSET - HALF,
                    y: TOP - HALF,
                    w: len + ONE,
                    h: HEIGHT + ONE,
                },
            }
        });

        // `this+0xa0`: up only while *neither* side's piece is. Its window into
        // the level strip slides against the second side's lead, so the art's
        // green-to-orange edge moves towards whichever counter is ahead.
        let level = (second_piece.is_none() && first_piece.is_none()).then_some(Piece {
            src: Rect {
                x: (LEFT - LEVEL_SRC_BACK) + LEVEL_SRC_FORWARD - lead_second,
                y: LEVEL_SRC_Y,
                w: LEVEL_SRC_WIDTH,
                h: SRC_HEIGHT,
            },
            dst: Rect {
                x: LEFT - HALF,
                y: TOP - HALF,
                w: LEVEL_WIDTH,
                h: HEIGHT + ONE,
            },
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
mod indicator_tests {
    use super::indicator::*;

    /// The run of levels skips 1: cell 0 stores 0 and cell 1 stores 2, so no
    /// press can reach the case `FUN_10027030` does not have.
    #[test]
    fn no_cell_can_ask_for_the_level_that_has_no_case() {
        let levels: Vec<usize> = (FIRST_WIDGET..).take(CELLS).filter_map(level_for).collect();
        assert_eq!(levels, vec![0, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert!(knob(1).is_none());
        assert!(levels.iter().all(|l| knob(*l).is_some()));
    }

    /// The knob lands on its own cell, which is the check that the switch's
    /// odd multipliers were read right: cell 0's is 7 where the pattern would
    /// give 8, and that is what makes the two ends line up.
    #[test]
    fn the_knob_lands_on_the_cell_that_was_pressed() {
        for cell in 0..CELLS {
            let widget = FIRST_WIDGET + cell;
            let level = level_for(widget).unwrap();
            let (_, dst) = knob(level).unwrap();
            // Widget 15 is at x 676 and they step by 12; the sprite sits half a
            // pixel back, as every sprite on the bar does.
            assert_eq!(dst.x, 676.0 + cell as f32 * WIDTH - 0.5, "cell {cell}");
        }
    }

    /// Ten levels of 25 reach 250, not 255. The constant is a float, and the
    /// ceiling is the shipped one.
    #[test]
    fn full_is_two_hundred_and_fifty() {
        assert_eq!(alpha(0), 0);
        assert_eq!(alpha(INITIAL_LEVEL), 250);
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
        assert!(p.level.is_some());
    }

    /// What the gauge actually shows in ordinary play: the window onto the
    /// level strip slides towards whichever counter is ahead, carrying the
    /// art's green-to-orange edge with it, and sits still at a tie.
    #[test]
    fn the_level_window_slides_towards_the_leading_counter() {
        let at = |a, b| pieces(a, b).level.unwrap().src.x;
        let tie = at(60, 60);
        assert_eq!(tie, LEFT);
        assert_eq!(at(70, 60), tie + 25.0, "the first counter ahead by ten");
        assert_eq!(at(60, 70), tie - 25.0, "the second counter ahead by ten");
    }

    /// The second side's bar is cut from the far end of its strip, so growing
    /// it extends leftwards in the sheet while its right edge stays put. The
    /// first side's is cut from a fixed origin instead.
    #[test]
    fn the_two_sides_are_cut_from_opposite_ends() {
        for counter in [90, 150, 400] {
            let second = pieces(0, counter).second.unwrap().src;
            assert_eq!(second.x + second.w, MAX_LEN + SECOND_SRC_END);
            assert_eq!(pieces(counter, 0).first.unwrap().src.x, FIRST_SRC_X);
        }
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
        assert_eq!(p.dst.w, MAX_LEN + 1.0);
        assert_eq!(p.src.w, MAX_LEN);
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
