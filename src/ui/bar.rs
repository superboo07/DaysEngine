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
//! +0x10 step the gauge ramp +0x28 set widget 2's latch    (FUN_10027230)
//! +0x14 draw               +0x2c draw the play/pause widget      (FUN_100258f0)
//! +0x18 -                  +0x30 widget 2's action               (FUN_10025b90)
//! +0x1c set host pointers  +0x34 widget 0's action               (FUN_10025cf0)
//!                          +0x38/+0x3c/+0x40/+0x44 lifetime
//! ```
//!
//! The engine's own pointer is `engine + 0x330`, and the eleven slots it calls
//! on it — `+0x04`, `+0x08`, `+0x0c`, `+0x1c`, `+0x20`, `+0x24`, `+0x28`,
//! `+0x2c`, `+0x30`, `+0x34`, `+0x38` — all land inside this vtable, which is
//! what confirms the member.
//!
//! # Two slots the engine never asks for
//!
//! `+0x10` and `+0x14` are **not** among those eleven. Only a function holding
//! `engine + 0x330` can reach the object at all, and a sweep of every one of
//! them for the form this compiler emits a virtual call in — `MOV reg,[vtbl +
//! off]` and then `CALL reg`, never the one-instruction `CALL dword ptr
//! [reg+off]` — turns up the eleven above and nothing else.
//!
//! Those two come from somewhere else. The bar is **registered as a graphics
//! module**, `DXGraphicModuleList` is itself one, and its four passes hand each
//! module in turn its `+0x0c`, `+0x10`, `+0x14` and `+0x18` — `FUN_00414310`,
//! `FUN_004143f0`, `FUN_004144c0` and `FUN_00414590`, which are that list's own
//! slots of the same numbers. `FUN_0040e540` runs three of them every rendered
//! frame:
//!
//! ```text
//! FUN_004253f0   the frame step
//!   FUN_004252e0   MenuBar +0x20, the update
//!   FUN_0040e540   the render frame
//!     list +0x10   for each module: module -> +0x10  <- FUN_10024c60
//!     Clear, BeginScene
//!     list +0x14   for each module: module -> +0x14  <- FUN_10024ca0
//!     EndScene, Present
//!     list +0x18
//! ```
//!
//! So the bar is not drawn by being asked to, and the affection gauge's ramp is
//! not stepped by being asked to either — see [`gauge::Anim`], which is all
//! `FUN_10024c60` does.
//!
//! `FUN_004230b0` puts it there — `MenuBar->+0x0c(device)` and then
//! `FUN_0040e940(engine+0x330)`, the global register — and `FUN_00423650`,
//! slot `+0x08`, takes it out again. `DXGraphicModuleList` is an RTTI name, and
//! `FUN_004144c0` walks its modules and calls each one's `+0x14` **with no test
//! of any kind**.
//!
//! So the draw runs every frame for as long as playback is loaded, whatever the
//! bar is doing, and the two sprites it draws past its own `this+0xbc` and host
//! `+0x140` tests really are on the picture with the bar gone — see
//! [`Bar::compose_faded`].
//!
//! So the widget geometry is in the DLL — the `MENUBAR` table the atlas search
//! finds, one record per region plus a long trailing run of alternates — while
//! every decision is a host call back into the executable. This module is the
//! DLL's half: which widget is live, what it shows, and what it asks the host
//! for. [`Act`] is that ask; the engine answers it.
//!
//! # The widgets
//!
//! One region per control, dense from 1, so widget index `n` is region `n + 1`
//! and table record `n`. The grouping below is not a reading of the sprites:
//! it is exactly how `FUN_10024100`'s dispatch, `FUN_10023fb0`'s enabled test
//! and `FUN_100262e0`'s caption switch all three bracket the range.
//!
//! Twenty-five of them here; Shiny Days' strip has sixteen and stops after the
//! first of the transparency controls, which on that module is a knob rather
//! than the first of ten cells. Everything above it is the same widget asking
//! the same host for the same thing — see [`Layout`], which is where the two
//! modules' record tables live and what says which of them is in hand.
//!
//! ```text
//!  0        toggles the host's auto-advance flag       host +0x120
//!  1        toggles pause                              host +0xf4
//!  2        rewind: restart, then back a part           host +0x10c(0),
//!                                                        +0xfc(1) / +0xfc(2) +0x124(0)
//!  3        skip to the end of this part               host +0xfc(2)
//!  4        skip to the next choice                    host +0x12c(1), +0xfc(5)
//!  5..9     playback speed, one widget per rate        host +0x8c(0..4)
//! 10..12    open a menu: save, load, backlog           host +0xf8(4/5/3)
//! 13        open the settings menu                     host +0xf8(2)
//! 14        leave playback                             host +0x100(1)
//! 15..24    the replay indicator's transparency        FUN_10026ed0
//! ```
//!
//! The names are the game's own, off the caption strips the sheet carries for
//! each widget: `Rewind to beginning of current part`, `Skip to end of current
//! part`, `Skip to next choice`.
//!
//! Widgets 2 and 3 both ask `+0xfc(2)`, and they are **not** the same press.
//! `FUN_10025b90` is widget 2: `+0xfc(1)` the first time, and on the second
//! `+0xfc(2)` followed by `+0x124(0)` — the flag at `engine + 0x560` that
//! sends the end-of-script block to `_GetBackScriptFile@12` rather than
//! `_GetNextScriptFile@12`. So widget 3 goes forward out of the part and
//! widget 2 goes back, through one seek code and one flag. Widget 3's own
//! dispatch is `FUN_10025c90(this, 0)`, the same helper widget 4 reaches with
//! 1.
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

/// Everything about the strip that belongs to one module rather than to the
/// bar as a control.
///
/// The two games ship the same bar: the same widgets in the same order, asking
/// the host for the same things through the same dispatch. What they do not
/// share is the **record table** behind it. School Days HQ's strip has
/// twenty-five hit regions and Shiny Days' sixteen, and each lays its alternate
/// records out to match, so every index here is a raw offset into *one*
/// module's table and means nothing on the other's. Naming one of HQ's on Shiny
/// Days' strip reaches whatever happens to sit at that offset — which is how
/// its twelve caption strips, all of which share one destination, once drew
/// stacked on each other every frame.
///
/// Each field is named for the sprite object it belongs to, `this + N`, so the
/// two halves of a recovery — the function that assigns the record and the
/// function that draws the sprite — can be checked against each other.
///
/// # Where each set comes from
///
/// ```text
/// School Days HQ   SysMenuSDHQ.dll   FILM::MenuBar vtable 0x1003d804
///                  records assigned  FUN_10021c20 / FUN_10022650
///                  drawn             FUN_10024ca0
///                  hover and caption FUN_10024100 / FUN_100262e0
///
/// Shiny Days       SysMenuSD.dll     FILM::MenuBar vtable 0x1004e2bc
///                  records assigned  FUN_10031bc0 (source) / FUN_10031030 (destination)
///                  drawn             FUN_10034230
///                  hover and caption FUN_100335f0 / FUN_10035420
/// ```
///
/// Shiny Days' static `FILM::MenuBar` is `DAT_1005c270`, which is what its
/// one-line `_SetMenuBar@4` hands out and what `FUN_10030b40` — the static
/// initialiser reached through `_atexit` — constructs; its vtable is the
/// `FILM::MenuBar::vftable` symbol Ghidra recovers from the RTTI pointer that
/// precedes it at `0x1004e2b8`. Its table is at `0x100586f8` and runs
/// forty-four records, ending where `FILM::MenuBar::RTTI_Type_Descriptor`
/// starts at `0x10058b18`.
#[derive(Debug, Clone, Copy)]
pub struct Layout {
    /// Which module's bar this is, for diagnostics.
    pub module: &'static str,
    /// Hit regions on the strip, and so widgets. The count is what picks the
    /// layout: see [`Layout::of`].
    pub widgets: usize,
    /// First of the five menu buttons' resting records, or `None` on a strip
    /// whose base art already carries them.
    ///
    /// School Days HQ draws `this+0xa4[0..5]` from records `0x30 + k`
    /// unconditionally. Shiny Days has no such run — `FUN_10031bc0` gives its
    /// five buttons no sprite at all, so they are part of `MenuBar.png`.
    pub menu_buttons_first: Option<usize>,
    /// The whole rate row as one sprite, live and dead. HQ `this+0x68` and
    /// `+0x6c`; Shiny Days `this+0x50` and `+0x54`.
    pub rates_live: usize,
    pub rates_dead: usize,
    /// Widget 4, live and dead, picked by `_GetSuperSkipFlag@0` on both.
    /// HQ `this+0x74` and `+0x78`; Shiny Days `this+0x5c` and `+0x60`.
    pub skip_live: usize,
    pub skip_dead: usize,
    /// Widget 1's resting art, named for the playback state rather than for
    /// the glyph: while playback runs the button offers "pause", so
    /// [`Layout::resting_while_playing`] is the pause glyph on both modules.
    /// HQ `this+0x70`, picked by host `+0x108`; Shiny Days `this+0x58`, picked
    /// by host `+0x124` in `FUN_10034c50`.
    pub resting_while_playing: usize,
    pub resting_while_paused: usize,
    /// Widget 0's resting art while the auto flag is clear. HQ `this+0x64`;
    /// Shiny Days `this+0x4c`.
    pub auto_resting: usize,
    /// What widget 0 shows while the flag is set.
    pub auto_lit: Auto,
    /// The affection gauge, which is a different instrument on the two.
    pub gauge: Gauge,
    /// The trough the transparency control sits in, dead and live, picked by
    /// the same host answer that makes the control usable. HQ `this+0x8c`;
    /// Shiny Days `this+0x74`.
    pub slider_dead: usize,
    pub slider_live: usize,
    /// The `REPLAYMODE` indicator, which is the thing the transparency control
    /// fades.
    pub replay_mode: ReplayMode,
    /// How the player sets that transparency.
    pub knob: Knob,
    /// Widget 0's hover art while the auto flag is set; with it clear the
    /// widget's own record is used. HQ `FUN_10024100`'s special case for index
    /// 0, Shiny Days `FUN_100335f0`'s.
    pub auto_hover_on: usize,
    /// Widget 1's hover art, which the vtable `+0x2c` re-place picks — HQ
    /// `FUN_100258f0`, Shiny Days `FUN_10034c50` — off the same "is playback
    /// paused" answer that picks [`Layout::resting_while_playing`], and out of
    /// the same column of the sheet, so the resting glyph and the glyph the
    /// pointer lights are always the same one.
    ///
    /// **The re-place is the last thing the update does**, which is what makes
    /// it the recovered rule rather than the dispatch's own assignment.
    /// `FUN_10024100` and `FUN_100335f0` both end with
    /// `if (host->+0x114() == 1) this->+0x2c()` — Shiny Days' slot is `+0x130`
    /// — after the hit test, after the press dispatch, and after their own
    /// assignment of the hovered widget's record, which they make only on the
    /// frame the hovered widget *changes*. The re-place then runs every frame
    /// and re-states both sprites, so it is what the draw pass sees.
    ///
    /// That slot is the engine's state word. `SHINYDAYS.exe`'s `FUN_0041da70`
    /// returns the host subobject's `+0x228` and nothing else, and the
    /// subobject is installed at engine `+0x2c` — `mov [esi+0x2c],
    /// 0x0048e50c`, the only two occurrences of that immediate in `.text` —
    /// so the member is engine `+0x254`. 1 is the state a film plays in:
    /// `FUN_00418360`, the window message pump, gates every one of the bar's
    /// keyboard shortcuts on `engine+0x254 == 1` and asks the same question
    /// through `+0x130` a few lines further down, which ties the slot to the
    /// member a second way.
    ///
    /// The two halves of the module agree on School Days HQ and **disagree on
    /// Shiny Days**, and the sheet says which one to believe. Widget 1's four
    /// records all draw to `(10, 21, 106, 24)` and differ only in their source:
    /// resting is `x` 1 (record 20, playing) or `x` 108 (record 21, paused) on
    /// row `y` 81, and the hover row at `y` 1 holds the same two columns,
    /// record 1 at `x` 1 and record 16 at `x` 108. `FUN_10034c50` pairs column
    /// with column; `FUN_100335f0` crosses them, taking record 16 while
    /// playback runs. Its assignment is overwritten inside the same call before
    /// anything draws, so the crossed pair never reaches the screen.
    pub hover_while_paused: usize,
    pub hover_while_playing: usize,
    /// First of the twelve caption strips [`caption`] indexes.
    pub caption_first: usize,
}

/// What widget 0 shows while the auto flag is set.
#[derive(Debug, Clone, Copy)]
pub enum Auto {
    /// School Days HQ: `this+0x64` cycles a run of frames.
    ///
    /// `FUN_10024ca0` picks one with
    /// `((now - started) / (1000 / (speed + 1))) % frames + first`, so the
    /// animation runs faster the higher the playback rate.
    Animated { first: usize, frames: usize },
    /// Shiny Days: one more record, and no animation.
    ///
    /// `FUN_10035050` — widget 0's action, vtable slot `+0x34` — sets
    /// `this+0x4c` to one of two records on host `+0x150`, and `FUN_10034230`
    /// draws `this+0x4c` while that answer is clear and `this+0x6c` while it is
    /// set. Neither sprite is ever re-recorded after that.
    ///
    /// That this module has no animation is a claim, and two methods agree on
    /// it: `FUN_10034230` contains no frame arithmetic of any kind, and a raw
    /// scan of the whole class's code — `0x10030b30` to `0x100361c0` — for
    /// four-byte references into the record table finds only the records named
    /// in this [`Layout`], with no run of consecutive frames among them.
    Lit(usize),
}

/// The affection gauge a module's bar carries.
///
/// Both draw a bed under the host's "gauge raised" answer — inside the bar's
/// own visibility test while it is clear and outside it while it is set, which
/// is what puts a raised gauge on a faded bar — and both ramp what sits in it
/// over 1500ms and hold it for 2000ms. What sits in it is not the same thing.
#[derive(Debug, Clone, Copy)]
pub enum Gauge {
    /// School Days HQ: `this+0x80`, a 538-wide bed, with three pieces the bar
    /// sizes itself from the *pair* of counters `001` and `002`. See [`gauge`].
    Pieces { bed: usize },
    /// Shiny Days: `this+0x68`, a 666-wide bed, with one fill bar.
    ///
    /// `FUN_10035780` — vtable slot `+0x10`, the pass `DXGraphicModuleList`
    /// runs before the draw — asks the host for the single counter `001`
    /// through slot `+0x8` and ramps `this+0x3c` towards it. `FUN_10035670`
    /// then places `this+0x84` at record `fill`'s origin with its width set to
    /// that value, clamped to the record's own 599. There are no pieces, no
    /// second counter and no leads.
    ///
    /// [`gauge::Fill`] is the ramp and [`State::gauge_fill`] the value it
    /// leaves; [`Bar::gauge_fill_cut`] is where the bar comes out.
    Fill { bed: usize, fill: usize },
}

impl Gauge {
    /// The bed's record, which both draw the same way.
    pub fn bed(&self) -> usize {
        match *self {
            Gauge::Pieces { bed } | Gauge::Fill { bed, .. } => bed,
        }
    }
}

/// Where the `REPLAYMODE` indicator goes, which is not the same on the two.
#[derive(Debug, Clone, Copy)]
pub enum ReplayMode {
    /// School Days HQ: `this+0x94`, one record at `(697, 80)` — **below** the
    /// 800x75 strip, so it lands on the picture rather than on the bar. See
    /// [`Bar::indicator`].
    BelowStrip(usize),
    /// Shiny Days: `this+0x7c` and `this+0x80`, a pair at `(680, 20)` inside
    /// the strip.
    ///
    /// `FUN_10034230` draws the live one past both the bar's visibility test
    /// and its fade, and the dead one only while the bar is down — so the strip
    /// carries a `REPLAYMODE` sign that stays on the picture with the bar gone,
    /// the same thing HQ's below-strip record does from outside the strip.
    OnStrip { live: usize, dead: usize },
}

/// How the player sets the `REPLAYMODE` indicator's transparency.
#[derive(Debug, Clone, Copy)]
pub enum Knob {
    /// School Days HQ: ten cells, widgets 15 to 24, and a knob sprite the bar
    /// sizes itself rather than taking from a record. See [`indicator`].
    Cells,
    /// Shiny Days: one widget you take hold of and drag. See [`slider`].
    ///
    /// `record` is the knob's own record, whose `y`, width and height are used
    /// whole and whose `x` is replaced by `this+0xb0`.
    Drag { record: usize },
}

impl Layout {
    /// School Days HQ's, from `SysMenuSDHQ.dll`.
    pub const SCHOOL_DAYS_HQ: Layout = Layout {
        module: "SysMenuSDHQ.dll",
        widgets: 25,
        menu_buttons_first: Some(48),
        rates_live: 47,
        rates_dead: 66,
        skip_live: 46,
        skip_dead: 65,
        resting_while_playing: 44,
        resting_while_paused: 45,
        auto_resting: 42,
        auto_lit: Auto::Animated {
            first: 0x1d,
            frames: 13,
        },
        gauge: Gauge::Pieces { bed: 67 },
        slider_dead: 69,
        slider_live: 70,
        replay_mode: ReplayMode::BelowStrip(68),
        knob: Knob::Cells,
        auto_hover_on: 25,
        hover_while_paused: 1,
        hover_while_playing: 26,
        caption_first: 53,
    };

    /// Shiny Days', from `SysMenuSD.dll`.
    pub const SHINY_DAYS: Layout = Layout {
        module: "SysMenuSD.dll",
        widgets: 16,
        menu_buttons_first: None,
        rates_live: 23,
        rates_dead: 37,
        skip_live: 22,
        skip_dead: 36,
        resting_while_playing: 20,
        resting_while_paused: 21,
        auto_resting: 18,
        auto_lit: Auto::Lit(19),
        gauge: Gauge::Fill { bed: 43, fill: 17 },
        slider_dead: 42,
        slider_live: 41,
        replay_mode: ReplayMode::OnStrip { live: 38, dead: 39 },
        knob: Knob::Drag { record: 40 },
        auto_hover_on: 15,
        hover_while_paused: 16,
        hover_while_playing: 1,
        caption_first: 24,
    };

    /// The layout for a strip with this many hit regions, or `None` for one
    /// neither set was recovered against.
    ///
    /// The count is the whole of the test, and it is not arbitrary: a strip
    /// with a different number of regions has a different table behind it, and
    /// every index in a [`Layout`] would address the wrong art on it. A third
    /// module draws nothing but the widget under the pointer — that one sprite
    /// comes from the hit map's own run and is right on any module — rather
    /// than drawing garbage.
    pub fn of(widgets: usize) -> Option<&'static Layout> {
        match widgets {
            25 => Some(&Layout::SCHOOL_DAYS_HQ),
            16 => Some(&Layout::SHINY_DAYS),
            _ => None,
        }
    }

    /// The five menu buttons' resting records, empty where the base art carries
    /// them.
    pub fn menu_buttons(&self) -> impl Iterator<Item = usize> {
        self.menu_buttons_first
            .into_iter()
            .flat_map(|first| first..first + 5)
    }
}

impl Auto {
    /// Which record widget 0 draws from while the auto flag is set.
    pub fn frame(&self, elapsed_ms: u32, speed: usize) -> usize {
        match *self {
            Auto::Animated { first, frames } => {
                let period = 1000 / (speed as u32 + 1);
                let step = elapsed_ms.checked_div(period).unwrap_or(0);
                first + (step as usize % frames)
            }
            Auto::Lit(record) => record,
        }
    }
}

/// The five playback rates widgets 5..9 select, from the table at `0x004f99f0`
/// that host slot `+0x8c` indexes.
///
/// The last two are **12 and 24, not 16 and 32**. The English chip sheet draws
/// those two buttons as `▶×16` and `▶×32`, but the art is not the authority:
/// `FUN_00424f90` stores `DAT_004f99f0[index]` as the rate and that table is
/// `1.0, 2.0, 4.0, 12.0, 24.0`.
///
/// The two games share it, so this is not per-[`Layout`]: `SHINYDAYS.exe`'s
/// host slot `+0x98` is `FUN_004176a0`, the same routine against the table at
/// `0x004a1b3c`, and that table holds the same five rates.
pub const SPEEDS: [f32; 5] = [1.0, 2.0, 4.0, 12.0, 24.0];

/// The elapsed value widget 0's animation is placed from.
///
/// `FUN_10024ca0` computes the frame and re-places the sprite **only while
/// host `+0x108` — playback is paused — answers zero**, and draws it either
/// way. So a paused bar keeps the frame it was left on.
///
/// The clock it is measured against does not stop: the original re-reads
/// `timeGetTime` against the start stamped when the flag went up, so unpausing
/// picks the animation up where the clock has got to rather than where it left
/// off. That is why this takes the running value and the last placed one and
/// chooses between them, instead of holding a paused clock back.
///
/// Shiny Days has no animation to place ([`Auto::Lit`]), so this changes
/// nothing there.
pub fn placement_clock(paused: bool, now_ms: u32, last_ms: u32) -> u32 {
    if paused {
        last_ms
    } else {
        now_ms
    }
}

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

    /// The alpha every one of the bar's sprites is modulated by, bar the ones
    /// `FUN_10025690` skips — see [`Bar::compose_faded`], which draws those as
    /// they stand.
    pub fn alpha(&self) -> u8 {
        self.alpha
    }

    /// Puts the bar straight into its hidden state, for a fresh script.
    pub fn reset(&mut self) {
        *self = Fade::default();
    }
}

/// The frame past which widget 2's latch is dropped.
///
/// `FUN_10024100` clears the latch whenever `0x48 < param_1`, and the update's
/// argument is the **script clock** — `FUN_004252e0` passes `engine + 0x208`,
/// the same member every other part of the tick reads. So this is not "72
/// frames since the press": it is frame 72 of the script, three seconds in at
/// 24 fps.
///
/// That is a window all the same, because the press that sets the latch is the
/// one that puts the clock back to the script's first frame. A restart at
/// 20:00 leaves three seconds of replayed script in which the second press
/// means "back a part", and after that the button restarts again.
///
/// The engine drops the latch on every script change as well — vtable slot
/// `+0x28` (`FUN_10027230`, which writes `this + 0xe0` outright) called with 0
/// at the end of `FUN_00424020`, the pass that has just loaded what comes
/// next. The original needs that call because its `FILM::MenuBar` is a static
/// that outlives the script; this engine builds a [`Bar`] per script, so the
/// latch it starts with is already down.
pub const RESTART_LATCH_FRAMES: u32 = 0x48;

/// Which menu widgets 10..13 ask the host to open, by the number they pass to
/// host slot `+0xf8`.
///
/// The slot puts the engine into state 3 and hands the number to
/// `_SetReMenu@4`, so this is the DLL's own re-entry number rather than one of
/// `SystemInit`'s mode integers. `setSystemInit` is the switch that consumes
/// it, and all four the bar produces are known from it:
///
/// ```text
/// 4  the save/load module, opened to save   (its +0x94 poked to 1)
/// 5  the same module, opened to load        (+0x94 poked to 0)
/// 2  the Option screen
/// 3  the backlog screen, which `SystemInit` has no mode for
/// ```
///
/// Code 3 selects `DAT_1004ffc8`, whose static-init thunk `FUN_10038360` calls
/// the constructor `FUN_10001cb0`, which installs `MENU::BackLogView::vftable`
/// — see [`crate::ui::backlog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuRequest(pub i32);

/// Where widget 2, 3 or 4 asks the engine to move to, by the code passed to
/// host slot `+0xfc`.
///
/// `FUN_00425bf0` is the state-4 handler that consumes these, and the three
/// codes below are the ones the bar can produce. The codes travel out of here
/// unresolved: which script a code lands on belongs to the branch graph, not
/// to this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seek(pub i32);

impl Seek {
    /// Restart the current script from its first frame. `FUN_00425bf0` case 3
    /// under code 1 rewinds the timeline and replays it.
    pub const RESTART: Seek = Seek(1);
    /// End this part: widget 3, and widget 2's second press. Named for the
    /// game's own caption, `Skip to end of current part`, because the seek is
    /// not always out of the script.
    ///
    /// **Not simply "leave the script".** `FUN_00425bf0`'s case 3 under code 2
    /// asks the timeline the same question case 6 does — is the `[SkipFRAME]`
    /// target ahead of the clock and different from the `[Next]` end? — and
    /// when it is, seeks to that target less `DAT_0050c468` (24 frames) and
    /// keeps playing. Only when there is no target ahead does it seek to the
    /// end and let the end-of-script block chain. The rewind flag forces the
    /// second branch: the test is `(skip == end) || (skip < clock) ||
    /// engine + 0x560`.
    ///
    /// So the landing is [`crate::playback::stage::Stage::skip_target`]'s,
    /// which is why both widgets ask it.
    pub const END_OF_PART: Seek = Seek(2);
    /// Widget 4's code, reached only after host `+0x12c(1)`.
    ///
    /// `FUN_00425bf0`'s case 2 routes it to case 6, which jumps to the choice
    /// this script raises — or, when it raises none, chases one across the
    /// scripts that follow, which is case 7 and is reachable from nowhere
    /// else. See [`crate::playback::stage::Stage::skip_target`].
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
    /// skip request through `+0x12c(1)` first, which [`Seek::SKIP`] carries as
    /// its own code rather than as a separate action.
    ///
    /// `rewind` is host `+0x124(0)` (`FUN_0042bfd0`), which widget 2's second
    /// press makes straight after the seek. It raises `engine + 0x560` and
    /// clears the moving flag beside it, and that is what makes the
    /// end-of-script block take `_GetBackScriptFile@12` and skip the read
    /// mark — the *previous* part rather than the next one. It is on the same
    /// action as the code because the original makes both calls from one
    /// dispatch arm and the engine has to see them together.
    Seek { code: Seek, rewind: bool },
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
    /// Take hold of the transparency knob, on a module whose control is a
    /// drag rather than ten cells. See [`slider`] and [`Bar::drag`].
    ///
    /// Like [`Act::Transparency`] this asks the host for nothing — but unlike
    /// it, it is also **silent**: `FUN_100335f0`'s case `0xf` is the one arm of
    /// the dispatch that does not go through the enabled test, and that test is
    /// what plays the click. So taking hold of the knob makes no sound, which
    /// is why [`Layout::enabled`] answers false for the widget.
    GrabKnob,
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
    /// `_GetAutoDraw@0`: the `AutoDraw` setting, which decides whether widget
    /// 0's lit sprite is part of the bar at all or a sign that sits over the
    /// picture on its own.
    ///
    /// The export is not a question about drawing state. `FUN_00422170`, the
    /// `FILMENGINE.INI` parse, hands the settings store at `DAT_0050b160` to
    /// `_SystemMenuInit@4`, which calls `FUN_10006ce0` on the module's
    /// singleton; that fills `+0x1b0` with the store's
    /// `+0x10(L"AutoDraw", 1)` — the bool getter with a default of 1 — beside
    /// `TextView`, `MenVoice`, `Mute`, `Skip`, `SuperSkip` and `UseSOM`.
    /// `_GetAutoDraw@0` returns that member and nothing else writes it: exactly
    /// one instruction in the module stores at that displacement, checked with
    /// Ghidra's instruction listing and again with a raw scan of `.text`.
    /// The store's identity is forced by shape — its `+0x10` and `+0x14` are
    /// `(key, default)` getters (`FUN_0046ca70`, `FUN_0046cae0`), while the
    /// *host* interface's `+0x14` is a `void` setter, so the object the module
    /// is handed cannot be the host.
    ///
    /// So this is [`Config`]'s own `AutoDraw`, and both modules gate the same
    /// sprite on it twice over — School Days HQ's `this+0x84` in
    /// `FUN_10024ca0` and `FUN_10025690`, Shiny Days' `this+0x6c` in
    /// `FUN_10034230` and `FUN_10034a40`:
    ///
    /// ```text
    /// if (auto flag) {
    ///     if (_GetAutoDraw@0() == 0) { if (this+0xbc) draw(lit) }   // with the bar
    ///     else                         draw(lit)                    // whatever the bar does
    /// }
    /// ```
    ///
    /// and the walker that modulates every sprite at once skips that one while
    /// the export answers non-zero. Set — its default, and what both retail
    /// installs ship — the lit sprite is drawn with the bar gone and never
    /// takes the fade's alpha. Clear, it is an ordinary sprite of the strip.
    pub auto_draw: bool,
    /// Host `+0x108`: playback is paused. Swaps widget 1's sprite.
    ///
    /// Also freezes widget 0's animation: `FUN_10024ca0` re-places that sprite
    /// only while this answers zero — see [`placement_clock`].
    pub paused: bool,
    /// Host `+0x104`: the replay menu started this playback. Disables widgets
    /// 4 and 10..12.
    pub replay: bool,
    /// Host `+0x110`: `FUN_00428210`. Disables widgets 4 and 5..9.
    ///
    /// ```text
    /// if (engine+0x5c8 == 0)        return 0
    /// if (engine+0x188 is non-empty and host->+0x18(it) != 0)  return 0
    /// return 1
    /// ```
    ///
    /// So it is the draw-message flag at `engine + 0x5c8` **and** a script
    /// name at `engine + 0x188` that is empty or does not resolve in the
    /// packs. `FUN_004281a0` — host `+0x10c`, the one writer of the flag —
    /// tests the same name and drops the rate to 1x when it fails to resolve,
    /// so the pair is about a script that is about to be missing.
    ///
    /// Widget 2 calls `+0x10c(0)` before either of its presses, so a restart
    /// or a rewind takes the message off. The engine's own writes of the flag
    /// are the zero `FUN_00423130` gives it and the clear at the end of
    /// `FUN_00424020`; a scan of the executable finds no third.
    ///
    /// **What raises it is not recovered.** So this engine never sets it, and
    /// the two widgets it gates are never disabled by it.
    pub message: bool,
    /// Host `+0x88`: `FUN_00427490`. Required by widgets 5..9, and by nothing
    /// else on the strip.
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
    /// `_GetSuperSkipFlag@0`, the `SuperSkip` setting. It both gates widget 4
    /// — see [`Layout::enabled`] — and picks its resting sprite.
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
    /// `001` or `002` — the two the gauge draws. So the gauge surfaces over a
    /// faded bar exactly when the affection counters just moved. See
    /// [`crate::install::feeling`].
    ///
    /// The ramp is what puts it down again, at the end of its own three and a
    /// half seconds — see [`gauge::Anim`]. Nothing else does, short of a film
    /// run starting: a script ending leaves it alone, so a delta credited in
    /// the last seconds of a scene finishes over the next one.
    pub gauge_raised: bool,
    /// The two counters as the save holds them, `(001, 002)`, if they are
    /// known.
    ///
    /// `FUN_10026050` and [`gauge::Anim`]'s first step both ask the host for
    /// these two by name through slot `+8`. They live in the save's variable
    /// store, so a bar drawn without save state has nothing to show and this
    /// is `None`.
    ///
    /// **Not what the gauge draws.** The gauge draws its own pair, which
    /// chases this one over a ramp; [`State::gauge_leads`] is that.
    pub gauge: Option<(i32, i32)>,
    /// `this+0x48` and `this+0x4c`: the two leads the gauge's three pieces are
    /// sized from, which is the only thing `FUN_10026540` reads.
    ///
    /// [`gauge::Anim`] is what moves them, and it outlives any one script — so
    /// this comes from a value the engine keeps across scripts, not from
    /// [`State::gauge`]. A tie is the default because the static `FILM::MenuBar`
    /// starts zeroed.
    pub gauge_leads: (f32, f32),
    /// Shiny Days' `this+0x3c`: the counter its gauge draws, which is a level
    /// rather than a lead. [`gauge::Fill`] is what moves it, and
    /// [`Gauge::Fill`] what draws it — [`State::gauge_leads`] is the other
    /// module's and the two are never both in play.
    pub gauge_fill: f32,
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
            auto_draw: config.flag(Flag::AutoDraw),
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

impl Layout {
    /// Whether a widget can be activated, from `FUN_10023fb0` and its Shiny
    /// Days counterpart `FUN_100334d0`.
    ///
    /// A press on a widget that is not live is swallowed *and makes no sound*:
    /// the dispatch asks this question before playing the click, so a dead
    /// button is silent as well as inert. It is also what gates the hover art
    /// and the caption — both dispatches set their "hovered" and "caption"
    /// flags to zero for a widget this answers false for — so a dead button
    /// shows nothing under the pointer either.
    ///
    /// The two modules agree on every widget the two strips share. They differ
    /// only past the fifteenth, and only because the controls there are
    /// different: HQ's ten transparency cells are live while the host is
    /// following a record, while Shiny Days' single knob widget is **never**
    /// live — `FUN_100334d0`'s case `0xf` returns 0 outright, and its dispatch
    /// reaches the knob from outside the enabled test instead. See
    /// [`Act::GrabKnob`].
    pub fn enabled(&self, widget: usize, state: State) -> bool {
        match widget {
            0..=3 | 0xd | 0xe => true,
            // Widget 4 asks `_GetSuperSkipFlag@0` here, not host `+0x88`:
            // `FUN_10023fb0`'s case 4 is `!replay && !message &&
            // _GetSuperSkipFlag@0()`, and `FUN_100334d0`'s is the same. So
            // the skip button is dead — and, because the enabled test is what
            // gates them, silent, unhovered and captionless — for a player
            // who has `SuperSkip` off. Only the speed row asks `+0x88`.
            4 => !state.replay && !state.message && state.super_skip,
            5..=9 => !state.message && state.skippable,
            10..=12 => !state.replay,
            0xf..=0x18 => matches!(self.knob, Knob::Cells) && state.following_record,
            _ => false,
        }
    }

    /// What a widget does, from `FUN_10024100`'s dispatch and Shiny Days'
    /// `FUN_100335f0`.
    ///
    /// `latched` is the bar's own one-bit memory for widget 2, set by its first
    /// press and cleared after [`RESTART_LATCH_FRAMES`]; see [`restart_latch`].
    /// Every other widget is stateless.
    ///
    /// The two dispatches are the same switch in the same order, asking the
    /// host for the same things: the Shiny Days slot numbers are School Days
    /// HQ's shifted by `0xc` below `+0xa8` and by `0x1c` above it, without
    /// exception across the eighteen slots the bar uses, and the four the
    /// answers were chased to in `SHINYDAYS.exe` hold the members they should —
    /// `+0x13c` toggles `+0x254` and writes the `AutoMode` settings key,
    /// `+0x110` picks pause or resume off `+0x234`, `+0xa0`/`+0xa4` write and
    /// read `+0x1e8`, and `+0x170` returns `+0x79c`, which is the same member
    /// School Days HQ's `+0x154` returns.
    pub fn action(&self, widget: usize, state: State, latched: bool) -> Act {
        if let (Knob::Drag { .. }, 0xf) = (self.knob, widget) {
            // `FUN_100335f0`'s case 0xf, which is reached whether or not the
            // widget is live. Whether the pointer is actually on the knob is
            // `FUN_10035cb0`, and that is [`Bar::press`]'s to ask because it
            // needs the knob's position.
            return if state.following_record {
                Act::GrabKnob
            } else {
                Act::None
            };
        }
        if !self.enabled(widget, state) {
            return Act::None;
        }
        match widget {
            0 => Act::ToggleAuto,
            1 => Act::TogglePause,
            // `FUN_10025b90`, and Shiny Days' `FUN_10034ef0` at vtable slot
            // `+0x30`. Both are the same three-armed press:
            //
            //   unlatched            +0xfc(1), and latch unless replay or
            //                        following a record
            //   latched, allowed     +0xfc(2) then +0x124(0), and unlatch
            //   latched, refused     nothing at all
            //
            // The third arm cannot be reached by pressing: the latch is only
            // ever set with both members clear and neither turns back on
            // inside a script. It is here because the shipped dispatch asks
            // the question on the second press rather than trusting the
            // latch, and a press it refuses does nothing — it does not fall
            // back to restarting.
            2 => match (latched, state.replay || state.following_record) {
                (false, _) => Act::Seek {
                    code: Seek::RESTART,
                    rewind: false,
                },
                (true, false) => Act::Seek {
                    code: Seek::END_OF_PART,
                    rewind: true,
                },
                (true, true) => Act::None,
            },
            // `FUN_10025c90(this, 0)`; widget 4 is the same helper with 1.
            3 => Act::Seek {
                code: Seek::END_OF_PART,
                rewind: false,
            },
            4 => Act::Seek {
                code: Seek::SKIP,
                rewind: false,
            },
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
}

/// The widgets [`Layout::action`] dispatches on, by name.
///
/// The numbers are the hit map's own and belong to `FUN_10024100`'s switch;
/// these are here so an engine that presses a widget without a pointer on it —
/// a key, a controller button — presses the same widget the pointer would,
/// through the same dispatch and the same enable rules, rather than
/// short-circuiting to the [`Act`] it expects.
pub mod widget {
    /// Toggle auto-advance.
    pub const AUTO: usize = 0;
    /// Pause or resume.
    pub const PAUSE: usize = 1;
    /// Restart the script; a second press inside the latch leaves it.
    pub const RESTART: usize = 2;
    /// Leave the script for whatever follows it.
    pub const NEXT: usize = 3;
    /// Jump to the choice this script raises.
    pub const SKIP: usize = 4;
    /// Open the save screen.
    pub const SAVE: usize = 10;
    /// Open the load screen.
    pub const LOAD: usize = 11;
    /// Open the Option screen.
    pub const OPTION: usize = 13;
    /// Stop playing and go back to the title.
    pub const LEAVE: usize = 14;

    /// The widget for a rate, by its index into [`super::SPEEDS`].
    ///
    /// Clamped rather than wrapped: the two end widgets are the two ends of
    /// the list, and a request past either is the end it is past.
    pub fn speed(index: usize) -> usize {
        5 + index.min(super::SPEEDS.len() - 1)
    }
}

/// Where widget 2's latch stands after a press, from `FUN_10025b90` and Shiny
/// Days' `FUN_10034ef0`.
///
/// The first press sets it, but only while playback is neither a replay nor a
/// followed recording. The second clears it — and a second press the same two
/// members refuse leaves it **set**, because the arm that clears it is inside
/// the test rather than around it, so the button stays armed rather than
/// falling back to restarting. [`Bar::expire_latch`] and [`Bar::clear_latch`]
/// are the two ways it comes down without a press.
/// Whether the update drops widget 2's latch at this frame, from
/// `FUN_10024100`'s `0x48 < param_1`.
///
/// The argument is the script clock — see [`RESTART_LATCH_FRAMES`] — so this
/// is a position in the script and not an age.
pub fn latch_expired(frame: u32) -> bool {
    frame > RESTART_LATCH_FRAMES
}

pub fn restart_latch(latched: bool, state: State) -> bool {
    let refused = state.replay || state.following_record;
    if latched {
        refused
    } else {
        !refused
    }
}

/// The caption strip shown for the hovered widget, as an offset from
/// [`Layout::caption_first`], or `None` for a widget with no caption.
///
/// From `FUN_100262e0`, which is a switch over the same twelve groups the
/// dispatch uses: widgets 0 to 4 each have their own strip, 5..9 share one,
/// 10, 11, 12, 13 and 14 each have their own, and 15..24 share the last.
/// The record addresses run `0x1004d318` upwards at the table's 24-byte stride,
/// so the strips are consecutive records and this returns their order, not
/// their address.
///
/// **The two modules share this**, which is a reading of Shiny Days'
/// `FUN_10035420` and not an inference from its table's shape: that switch has
/// the same twelve arms with the same widgets grouped the same way, running
/// consecutively from `0x10058938`. Its twelfth arm is case `0xf` alone rather
/// than `0xf..=0x18`, because its strip stops at sixteen widgets — the arm here
/// covers both.
///
/// The caption is only drawn for a widget [`Layout::enabled`] answers true for,
/// so on Shiny Days the twelfth strip is unreachable: its widget `0xf` is never
/// live. The strip is in the sheet all the same.
pub fn caption(widget: usize) -> Option<usize> {
    Some(match widget {
        0..=4 => widget,
        5..=9 => 5,
        10..=14 => widget - 4,
        0xf..=0x18 => 11,
        _ => return None,
    })
}

/// The `REPLAYMODE` indicator, ready to draw. See [`Bar::indicator`].
pub struct Indicator {
    /// The art at display scale.
    pub art: Image,
    /// `(x, y, width, height)` in the same display space [`Bar::strip`] is in,
    /// whose origin is the picture's top-left corner. `y` is below the strip.
    pub dst: (i64, i64, u32, u32),
    /// How solid to draw it, from [`Solidity::alpha`].
    pub alpha: u8,
}

/// How solid the `REPLAYMODE` indicator is drawn, in whichever unit this
/// module's control works in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Solidity {
    /// School Days HQ's `this+0xec`: the level 0 to 10 the pressed cell stores.
    Level(usize),
    /// Shiny Days' `this+0xb0`: the knob's `x` in the strip's own units, which
    /// [`slider`] turns into an alpha.
    Knob(f32),
}

impl Solidity {
    /// The alpha the indicator is drawn at.
    pub fn alpha(self) -> u8 {
        match self {
            Solidity::Level(level) => indicator::alpha(level),
            Solidity::Knob(x) => slider::alpha(x),
        }
    }
}

/// The bar, loaded and ready to hit-test and draw.
pub struct Bar {
    screen: Screen,
    /// Widget 2's latch: `this + 0xe0` in `FUN_10025b90`, one bit and no
    /// timestamp — what expires it is the clock, not an elapsed time. See
    /// [`RESTART_LATCH_FRAMES`].
    latch: bool,
    fade: Fade,
    /// How solid the replay-mode indicator is drawn.
    solidity: Solidity,
    /// Shiny Days' `this+0xe8`: the knob has been taken hold of and follows the
    /// pointer until the button comes up. Always false on a module whose
    /// control is ten cells.
    gripped: bool,
    /// This module's record table, or `None` for a strip neither set was
    /// recovered against. See [`Layout::of`].
    layout: Option<&'static Layout>,
}

impl Bar {
    pub fn load(
        vfs: &crate::install::vfs::Vfs,
        dll: &[u8],
        resolution: Resolution,
    ) -> Result<Bar, Error> {
        let screen = Screen::load(vfs, dll, PATH, resolution)?;
        let regions = screen.atlas().widgets.len();
        let layout = Layout::of(regions);
        if layout.is_none() {
            log::warn!(
                "{PATH}: {regions} hit regions, which is neither School Days HQ's 25 nor Shiny \
                 Days' 16 — no recovered record table addresses this strip, so its resting art \
                 and its captions are not drawn"
            );
        }
        let solidity = match layout.map(|l| l.knob) {
            Some(Knob::Drag { .. }) => Solidity::Knob(slider::INITIAL_X),
            _ => Solidity::Level(indicator::INITIAL_LEVEL),
        };
        Ok(Bar {
            screen,
            latch: false,
            fade: Fade::default(),
            solidity,
            gripped: false,
            layout,
        })
    }

    /// This module's record table, or `None` for a strip neither set was
    /// recovered against.
    pub fn layout(&self) -> Option<&'static Layout> {
        self.layout
    }

    /// How many widgets this strip has, which is the hit map's own count.
    pub fn widgets(&self) -> usize {
        self.screen.atlas().widgets.len()
    }

    /// Whether a widget can be activated. Always false without a [`Layout`]:
    /// with no recovered table, nothing about the strip past its hit map is
    /// known, and a press that dispatched School Days HQ's action from an
    /// unknown module's region would be a guess.
    pub fn enabled(&self, widget: usize, state: State) -> bool {
        self.layout
            .is_some_and(|layout| layout.enabled(widget, state))
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

    /// The strip's size in the pixels it is composited at — an 800x75 band at
    /// the top of the screen in its own art set, and whatever
    /// [`Bar::set_output_width`] has since asked for.
    pub fn strip(&self) -> (u32, u32) {
        self.screen.size()
    }

    /// Composites the strip at the width it will be drawn at.
    ///
    /// The strip runs the full width of the picture, so how much it is
    /// magnified is the window's width over its art set's — which is not the
    /// stage's ladder and need not be a whole number even when the stage's is.
    /// Left to the GPU that is a bilinear stretch, or, under whole-number
    /// scaling, nearest sampling at a fractional factor, which stair-steps
    /// every edge on the strip. So the strip goes up the way the menus do:
    /// composited once, straight to the size it is seen at, through
    /// [`crate::playback::scale`]'s band-limited pixel filter, and blitted
    /// 1:1. See [`crate::ui::screen::Screen::fit_to`].
    ///
    /// A width that is already the art set's own costs nothing: `fit_to`
    /// returns without touching anything when the size has not moved.
    pub fn set_output_width(&mut self, width: u32) {
        let (map_w, map_h) = self.screen.map_size();
        if map_w == 0 || width == 0 {
            return;
        }
        let height = (f64::from(map_h) * f64::from(width) / f64::from(map_w))
            .round()
            .max(1.0) as u32;
        self.screen.fit_to(width, height);
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
        self.latch = false;
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
    /// `at` is the pointer in the strip's own units, which only Shiny Days'
    /// knob needs: `FUN_10035cb0` refuses the grip unless the pointer is inside
    /// the knob itself rather than merely inside its region.
    pub fn press(
        &mut self,
        widget: usize,
        state: State,
        frame: u32,
        at: Option<(u32, u32)>,
    ) -> Act {
        let Some(layout) = self.layout else {
            return Act::None;
        };
        self.expire_latch(frame);
        let latched = self.latch;
        let act = layout.action(widget, state, latched);
        // Widget 2 is always enabled, so the latch moves on every press of it
        // — including the one the replay members refuse, which returns
        // [`Act::None`] and leaves the latch where it was.
        if widget == 2 {
            self.latch = restart_latch(latched, state);
        }
        match act {
            // `FUN_10026ed0` stores the level in the bar itself and applies it
            // to the bar's own sprites, so this is the whole of what the press
            // does.
            Act::Transparency(level) => self.solidity = Solidity::Level(level),
            // `FUN_100335f0` case `0xf`: the grip is taken only when
            // `FUN_10035cb0` agrees the pointer is on the knob.
            Act::GrabKnob => {
                let x = at.map_or(0.0, |(x, _)| {
                    (f64::from(x) / self.screen.out_scale()) as f32
                });
                self.gripped =
                    matches!(self.solidity, Solidity::Knob(knob) if slider::on_knob(knob, x));
                if !self.gripped {
                    return Act::None;
                }
            }
            _ => {}
        }
        act
    }

    /// Moves a gripped knob, and answers whether it moved.
    ///
    /// `FUN_100359c0` runs every update: while the button is down it adds the
    /// pointer's movement since the last update to `this+0xb0` and clamps the
    /// result, and while it is up it clears the grip. `dx` is that movement in
    /// the strip's own units.
    ///
    /// **The original does this in two units at once.** It adds a movement in
    /// screen pixels to a value it stores in the strip's units, and clamps that
    /// value against bounds it has already multiplied by the display scale —
    /// so at anything but the strip's native 800x600 the knob both travels at
    /// the wrong rate and stops in the wrong place. That is a shipped bug and
    /// it is **not** reproduced: this works in the strip's own units
    /// throughout, which is what the original does at scale 1.
    pub fn drag(&mut self, held: bool, dx: f32) -> bool {
        if !held {
            self.gripped = false;
            return false;
        }
        if !self.gripped {
            return false;
        }
        match &mut self.solidity {
            Solidity::Knob(x) => {
                let was = *x;
                *x = (*x + dx).clamp(slider::MIN_X, slider::MAX_X);
                *x != was
            }
            Solidity::Level(_) => false,
        }
    }

    /// Puts a dragged knob at `x` in the strip's own units, clamped to its
    /// travel. For the headless tools, which have no pointer to drag with.
    pub fn set_knob(&mut self, x: f32) {
        if let Solidity::Knob(at) = &mut self.solidity {
            *at = x.clamp(slider::MIN_X, slider::MAX_X);
        }
    }

    /// Whether the knob is being dragged.
    pub fn gripped(&self) -> bool {
        self.gripped
    }

    /// How solid the replay-mode indicator is drawn.
    pub fn transparency(&self) -> Solidity {
        self.solidity
    }

    /// Whether anything the strip draws keeps an alpha of its own this frame.
    ///
    /// `FUN_10025690` names every sprite the bar owns but two: the rate
    /// readout, which it never touches, and the gauge bed with its three
    /// pieces, which it skips while the gauge is raised. While either is
    /// showing the strip cannot be drawn by modulating one texture — the fade
    /// has to be composited in, which is what [`Bar::compose_faded`] does.
    pub fn pinned(&self, state: State) -> bool {
        state.gauge_raised
            || self.rate_readout(state).is_some()
            // Widget 0's lit sprite under `AutoDraw`, which the draw puts on
            // the picture whether or not the bar is down — so it is also a
            // reason to composite the strip at all with the bar gone.
            || (self.layout.is_some() && state.auto && state.auto_draw)
    }

    /// Drops widget 2's latch once the clock is past
    /// [`RESTART_LATCH_FRAMES`], which is what the update does on every tick.
    pub fn expire_latch(&mut self, frame: u32) {
        if latch_expired(frame) {
            self.latch = false;
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

        // On a strip no recovered table addresses, every index would reach the
        // wrong record. Only the widget under the pointer survives, because
        // that one is the widget's own record and is right on any module.
        let Some(layout) = self.layout else {
            if !state.hidden {
                if let Some(widget) = hovered.filter(|w| *w < self.widgets()) {
                    out.push(widget);
                }
            }
            return out;
        };

        // `if (host+0x140() == 0)` — Shiny Days' `+0x15c`: with the bar hidden,
        // only the pinned gauge bed survives, and even that only under the
        // host's "gauge raised" answer.
        if !state.hidden {
            out.extend(layout.menu_buttons());
            if state.message {
                out.push(layout.rates_dead);
                out.push(layout.skip_dead);
            } else {
                out.push(if state.skippable {
                    layout.rates_live
                } else {
                    layout.rates_dead
                });
                out.push(if state.super_skip {
                    layout.skip_live
                } else {
                    layout.skip_dead
                });
                out.push(if state.following_record {
                    layout.slider_live
                } else {
                    layout.slider_dead
                });
            }
            out.push(if state.paused {
                layout.resting_while_paused
            } else {
                layout.resting_while_playing
            });
            if !state.auto {
                out.push(layout.auto_resting);
            }
            if !state.gauge_raised {
                out.push(layout.gauge.bed());
            }
            // The dead half of a strip-borne `REPLAYMODE` sign, which is up
            // only while the bar is. The live half is pinned — see
            // [`Bar::split`].
            if let ReplayMode::OnStrip { dead, .. } = layout.replay_mode {
                if !state.following_record {
                    out.push(dead);
                }
            }
        }
        if state.gauge_raised {
            out.push(layout.gauge.bed());
        }

        // The rate readout appears only once the rate is at least 2.0.
        if state.rate >= 2.0 && state.speed < SPEEDS.len() {
            out.push(state.speed + 5);
        }
        if let ReplayMode::OnStrip { live, .. } = layout.replay_mode {
            if state.following_record {
                out.push(live);
            }
        }
        if state.auto {
            out.push(layout.auto_lit.frame(elapsed_ms, state.speed));
        }

        if !state.hidden {
            // Both dispatches gate the hover sprite and the caption on the same
            // enabled test the press goes through, so a dead widget shows
            // nothing under the pointer.
            if let Some(widget) = hovered.filter(|w| layout.enabled(*w, state)) {
                out.push(match widget {
                    0 if state.auto => layout.auto_hover_on,
                    1 if state.paused => layout.hover_while_paused,
                    1 => layout.hover_while_playing,
                    other => other,
                });
                if let Some(group) = caption(widget) {
                    out.push(layout.caption_first + group);
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
        let widgets = self.widgets();
        let base = self.extras_base();
        let mut states = vec![WidgetState::Resting; widgets];
        for record in records {
            match *record {
                r if r < widgets && r < base => states[r] = WidgetState::Active,
                r => states.push(self.extra(r)),
            }
        }
        states
    }

    /// The record the alternate run starts at.
    ///
    /// Not the widget count: the run is however many records the locator
    /// actually matched against regions, and a strip can have a region with no
    /// record of its own. Shiny Days' does — its knob's region is one the table
    /// has no art for, so its sixteen regions are backed by fifteen records and
    /// the alternates begin at record 15.
    fn extras_base(&self) -> usize {
        self.screen
            .atlas()
            .segments
            .last()
            .map_or(self.widgets(), |(first, _, count)| first + count)
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
        if let Some(layout) = self.layout {
            if state.gauge_raised {
                faded.retain(|r| *r != layout.gauge.bed());
                pinned.push(layout.gauge.bed());
            }
            // A strip-borne `REPLAYMODE` sign is drawn past the bar's own
            // visibility test and is not in the list `FUN_10025690`'s
            // counterpart walks, so it keeps full alpha with the bar gone —
            // the same thing School Days HQ's below-strip record does.
            if let ReplayMode::OnStrip { live, .. } = layout.replay_mode {
                if state.following_record {
                    faded.retain(|r| *r != live);
                    pinned.push(live);
                }
            }
            // Widget 0's lit sprite is what `AutoDraw` takes out of the fade:
            // `FUN_10025690` modulates `this+0x84` only while
            // `_GetAutoDraw@0` answers zero, and `FUN_10034a40` does the same
            // for Shiny Days' `this+0x6c`. With the setting on it keeps the
            // opaque colour it was given. The hover sprite widget 0 shows
            // under the pointer is a different record and stays in the walker's
            // list, so it fades.
            if state.auto && state.auto_draw {
                let lit = layout.auto_lit.frame(elapsed_ms, state.speed);
                faded.retain(|r| *r != lit);
                pinned.push(lit);
            }
        }
        if let Some(readout) = self.rate_readout(state) {
            // Only the one `records` pushed for the readout: the same number
            // comes back as the hover sprite when that rate's widget is under
            // the pointer, and that one does fade.
            if let Some(at) = faded.iter().position(|r| *r == readout) {
                faded.remove(at);
            }
            pinned.push(readout);
        }
        (faded, pinned)
    }

    /// The record the rate readout draws from, while it is showing.
    fn rate_readout(&self, state: State) -> Option<usize> {
        (state.rate >= 2.0 && state.speed < SPEEDS.len()).then_some(state.speed + 5)
    }

    /// The sprites the bar sizes itself instead of taking whole from a record,
    /// split the way [`Bar::split`] splits the records: the gauge's three
    /// pieces, which stop fading once the gauge is raised, and the transparency
    /// knob, which does not.
    fn cuts(&self, state: State) -> (Vec<Cut>, Vec<Cut>) {
        let mut faded = self.gauge_cuts(state);
        faded.extend(self.gauge_fill_cut(state));
        let pinned = if state.gauge_raised {
            std::mem::take(&mut faded)
        } else {
            Vec::new()
        };
        // HQ's `this+0x90`, Shiny Days' `this+0x78`. Both draws put it last of
        // all, under the same three tests the trough is under plus the bar
        // being up.
        if !state.hidden && !state.message && state.following_record {
            if let Some(c) = self.knob_cut() {
                faded.push(c);
            }
        }
        (faded, pinned)
    }

    /// The transparency knob, wherever this module's control has put it.
    ///
    /// School Days HQ sizes its knob from constants rather than from a record,
    /// so [`indicator::knob`] has the whole of it. Shiny Days takes the
    /// record's `y`, width and height whole and replaces its `x` with
    /// `this+0xb0` — `FUN_100359c0` writes exactly those four into the sprite.
    fn knob_cut(&self) -> Option<Cut> {
        match (self.layout?.knob, self.solidity) {
            (Knob::Cells, Solidity::Level(level)) => {
                indicator::knob(level).map(|(src, dst)| cut(src, dst))
            }
            (Knob::Drag { record }, Solidity::Knob(x)) => {
                let art = self
                    .screen
                    .atlas()
                    .extras
                    .get(record.checked_sub(self.extras_base())?)?;
                let (w, h) = (art.dst.width as f32, art.dst.height as f32);
                Some(Cut {
                    src: (art.src_x as f32, art.src_y as f32, w, h),
                    dst: (
                        x - slider::HALF,
                        art.dst.y as f32 - slider::HALF,
                        w + slider::ONE,
                        h + slider::ONE,
                    ),
                })
            }
            _ => None,
        }
    }

    /// Turns a record index into a widget state, warning rather than drawing
    /// the wrong sprite when the recovered table is shorter than expected.
    fn extra(&self, record: usize) -> WidgetState {
        match record.checked_sub(self.extras_base()) {
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
    /// One sprite escapes it: `FUN_10025690` skips widget 0's animation while
    /// `_GetAutoDraw@0` is non-zero, so that one stays at full alpha, and
    /// `FUN_10024ca0` draws it past the `this+0xbc` test as well. That export
    /// is the `AutoDraw` setting — see [`State::auto_draw`] for the chain — and
    /// it defaults to set, so in an ordinary install the auto sign sits on the
    /// picture whether or not the bar is down. Reproduced: [`Bar::split`] pins
    /// it and [`Bar::pinned`] keeps the strip composited for it alone.
    ///
    /// Two more are absent from `FUN_10025690`'s list altogether — the rate
    /// readout at `this+0x88` and the `REPLAYMODE` indicator at `this+0x94` —
    /// and `FUN_10024ca0` draws both past the `this+0xbc` and host `+0x140`
    /// tests. That is a branch target, not a reading of indentation: the
    /// hidden branch at `0x10024f4f` is `JNZ 0x10025205`, and `0x10025205`
    /// begins the readout's own test. So a rate of 2.0 or more leaves that
    /// rate's button on the picture at full alpha, and the indicator stays on
    /// it whether or not the bar is down. Both are reproduced.
    ///
    /// The step that makes the consequence follow is that `FUN_10024ca0` runs
    /// every frame regardless of the bar — see the module docs for the chain,
    /// which ends at `DXGraphicModuleList`.
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
    ///
    /// The pieces are sized from the ramp's leads and not from
    /// [`State::gauge`], because the two disagree for the three and a half
    /// seconds a raise lasts — that is the whole of what the ramp is.
    /// **School Days HQ's gauge only.** Shiny Days' is a different instrument
    /// and comes out of [`Bar::gauge_fill_cut`].
    pub fn gauge_cuts(&self, state: State) -> Vec<Cut> {
        let Some(layout) = self.layout else {
            return Vec::new();
        };
        let Gauge::Pieces { bed } = layout.gauge else {
            return Vec::new();
        };
        if state.gauge.is_none() {
            return Vec::new();
        }
        if !self.records(None, state, 0).contains(&bed) {
            return Vec::new();
        }
        let (first, second) = state.gauge_leads;
        gauge::pieces_at(first, second)
            .drawn()
            .map(|piece| cut(piece.src, piece.dst))
            .collect()
    }

    /// Shiny Days' gauge bar, as a cut of the chip sheet.
    ///
    /// `FUN_10035670` sets only the destination, and only its width:
    /// `clamp(this+0x3c, 0, w)` where `w` is the record's own. The source is
    /// left at the whole strip `FUN_10031bc0` gave it, so the art is
    /// **squeezed** into the shorter destination rather than clipped to it.
    ///
    /// Drawn whenever the bed is, which is the pair of tests
    /// [`Bar::records`] already applies to the bed.
    fn gauge_fill_cut(&self, state: State) -> Option<Cut> {
        let Gauge::Fill { bed, fill } = self.layout?.gauge else {
            return None;
        };
        if !self.records(None, state, 0).contains(&bed) {
            return None;
        }
        let art = self
            .screen
            .atlas()
            .extras
            .get(fill.checked_sub(self.extras_base())?)?;
        let (w, h) = (art.dst.width as f32, art.dst.height as f32);
        Some(Cut {
            src: (art.src_x as f32, art.src_y as f32, w, h),
            dst: (
                art.dst.x as f32 - slider::HALF,
                art.dst.y as f32 - slider::HALF,
                state.gauge_fill.clamp(0.0, w) + slider::ONE,
                h + slider::ONE,
            ),
        })
    }

    /// The `REPLAYMODE` indicator, where it is not part of the strip's layer.
    ///
    /// School Days HQ's `this+0x94` goes at `(697, 80) 97x19` in the strip's
    /// own space — below the 800x75 strip, so it lands on the picture.
    /// `FUN_10024ca0` draws it outside the test that gates every widget, and it
    /// is not in `FUN_10025690`'s list, so it neither waits for the bar to drop
    /// down nor fades with it. The one thing it carries is the transparency the
    /// ten cells set.
    ///
    /// `None` on a module whose sign sits inside the strip: Shiny Days' is at
    /// `(680, 20)`, so it composites with everything else and [`Bar::split`]
    /// pins it instead.
    ///
    /// Returns the art at display scale, where it goes in the same display
    /// space [`Bar::strip`] is in, and the alpha to draw it at.
    pub fn indicator(&self, state: State) -> Option<Indicator> {
        if !state.following_record {
            return None;
        }
        let ReplayMode::BelowStrip(record) = self.layout?.replay_mode else {
            return None;
        };
        let n = record.checked_sub(self.extras_base())?;
        let widget = self.screen.atlas().extras.get(n)?;
        let (art, dst) = self.screen.cut_widget(widget);
        Some(Indicator {
            art,
            dst,
            alpha: self.solidity.alpha(),
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
        layer.modulate(self.fade.alpha());
        if !pinned.is_empty() || !pinned_cuts.is_empty() {
            let over = self
                .screen
                .compose_sprites(&self.states_of(&pinned), &pinned_cuts);
            // Everything in the pinned layer is opaque, whatever the fade is
            // doing, because nothing in it is a sprite `FUN_10025690` reaches:
            // a raised gauge and its bed are the four the walker skips under
            // host `+0x154`, and they were set opaque by `FUN_10026b40`'s first
            // step, which every raise begins with; the rate readout and the
            // `REPLAYMODE` sign are not in its list at all, so they keep the
            // opaque colour `FUN_10022650` gave them; and widget 0's lit sprite
            // is the one the walker skips under `_GetAutoDraw@0`.
            //
            // So the layer is blitted as it stands. Modulating it by the bar's
            // own alpha whenever the gauge happened to be down faded three of
            // those four, which is what the original never does to any of them.
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

    /// Widget 1 lights the glyph it is already showing. The vtable `+0x2c`
    /// re-place takes the resting sprite and the hover sprite out of the same
    /// column of the sheet — HQ `FUN_100258f0`, Shiny Days `FUN_10034c50` — and
    /// it is the last thing either update does, so its pairing is the one that
    /// draws. The record numbers are per module; the pairing is not. See
    /// [`Layout::hover_while_paused`].
    #[test]
    fn widget_1_lights_the_glyph_it_is_already_showing() {
        for (layout, playing, paused) in [
            (&Layout::SCHOOL_DAYS_HQ, (44, 26), (45, 1)),
            (&Layout::SHINY_DAYS, (20, 1), (21, 16)),
        ] {
            assert_eq!(
                (layout.resting_while_playing, layout.hover_while_playing),
                playing,
                "{} pairs the wrong records while playback runs",
                layout.module
            );
            assert_eq!(
                (layout.resting_while_paused, layout.hover_while_paused),
                paused,
                "{} pairs the wrong records while playback is paused",
                layout.module
            );
        }
    }
    use super::*;

    /// School Days HQ's, which is what the record numbers in these tests are.
    const HQ: &Layout = &Layout::SCHOOL_DAYS_HQ;
    const WIDGETS: usize = Layout::SCHOOL_DAYS_HQ.widgets;

    fn enabled(widget: usize, state: State) -> bool {
        HQ.enabled(widget, state)
    }

    fn action(widget: usize, state: State, latched: bool) -> Act {
        HQ.action(widget, state, latched)
    }

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

    /// A gauge raised over a bar that has already faded away is on screen at
    /// full strength: `FUN_10025690` skips those four sprites under host
    /// `+0x154`, and `FUN_10026b40`'s first step — which every raise begins
    /// with — set them opaque. The bar's own alpha is at nothing meanwhile,
    /// which is what makes the two separable at all.
    #[test]
    fn the_bar_is_at_nothing_while_a_raised_gauge_is_on_screen() {
        let mut fade = Fade::default();
        fade.update(0, false);
        assert_eq!(fade.alpha(), 0);

        // A delta lands with the bar away, and the strip stays gone.
        fade.update(1_000, false);
        fade.update(1_000 + FADE_OUT_MS, false);
        assert_eq!(fade.alpha(), 0);
        assert!(!fade.drawn(), "the strip itself is not up");
    }

    /// `FUN_10024ca0` re-places widget 0's animation only while host `+0x108`
    /// is clear, and the clock it measures against never stops — so a pause
    /// holds the frame and unpausing jumps to where the clock got to.
    #[test]
    fn a_paused_bar_holds_widget_0_s_frame_and_then_catches_up() {
        assert_eq!(placement_clock(false, 500, 0), 500, "running: place at now");
        assert_eq!(
            placement_clock(true, 900, 500),
            500,
            "paused: hold the frame"
        );
        assert_eq!(
            placement_clock(false, 4_000, 500),
            4_000,
            "unpaused: the clock ran on while the sprite did not"
        );
    }

    /// `AutoDraw` defaults to 1 and both retail installs ship it set, so widget
    /// 0's lit sprite is normally the one thing on the strip that outlives the
    /// fade.
    #[test]
    fn auto_draw_comes_from_the_setting_and_defaults_to_set() {
        assert!(
            State::from_config(&Config::parse_text("")).auto_draw,
            "an empty file leaves AutoDraw at its default of 1"
        );
        assert!(!State::from_config(&Config::parse_text("[AutoDraw]=\"0\"\n")).auto_draw);
        assert!(State::from_config(&Config::parse_text("[AutoDraw]=\"-1\"\n")).auto_draw);
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

    /// With host `+0x88` false — no script loaded — the speed row goes dead,
    /// and nothing else on the strip does. The skip button beside it answers
    /// `_GetSuperSkipFlag@0` instead.
    #[test]
    fn only_the_speed_row_goes_dead_without_a_script() {
        let no_script = State {
            skippable: false,
            ..live()
        };
        for widget in 5..=9 {
            assert!(!enabled(widget, no_script));
        }
        assert!(enabled(4, no_script));
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
    fn widget_two_restarts_then_rewinds_a_part() {
        // `FUN_10025b90` takes the second press only while neither the replay
        // menu nor the host's `+0x98` member is driving playback, so the state
        // here is not following a recording.
        let plain = State {
            following_record: false,
            ..live()
        };
        assert_eq!(
            action(2, plain, false),
            Act::Seek {
                code: Seek::RESTART,
                rewind: false,
            }
        );
        // The second press is the rewind: `+0xfc(2)` and then `+0x124(0)`.
        assert_eq!(
            action(2, plain, true),
            Act::Seek {
                code: Seek::END_OF_PART,
                rewind: true,
            }
        );
        // With the host following a record the latch never goes up, so the
        // button restarts however many times it is pressed.
        assert!(!restart_latch(false, live()));
        assert_eq!(
            action(2, live(), false),
            Act::Seek {
                code: Seek::RESTART,
                rewind: false,
            }
        );
    }

    /// A latch that is up while the replay members are set cannot be reached
    /// by pressing — they are what put it up — but the shipped dispatch asks
    /// the question again on the second press, and a press it refuses does
    /// nothing at all rather than restarting.
    #[test]
    fn a_refused_second_press_does_nothing_and_stays_armed() {
        let replay = State {
            replay: true,
            ..live()
        };
        assert!(!restart_latch(false, replay));
        assert_eq!(action(2, replay, true), Act::None);
        assert!(restart_latch(true, replay));
    }

    /// The latch is dropped by the clock passing frame 72, not by 72 frames
    /// passing since the press: `FUN_10024100`'s test is `0x48 < param_1` and
    /// `FUN_004252e0` passes `engine + 0x208`.
    #[test]
    fn the_latch_expires_on_the_clock_rather_than_on_elapsed_frames() {
        assert!(!latch_expired(0));
        assert!(!latch_expired(RESTART_LATCH_FRAMES));
        assert!(latch_expired(RESTART_LATCH_FRAMES + 1));
        // The press that arms it is the one that puts the clock back to the
        // script's first frame, so the window is the three seconds after a
        // restart however late in the script that restart was.
        assert!(!latch_expired(0));
    }

    #[test]
    fn widget_three_ends_the_part_without_rewinding() {
        let plain = State {
            following_record: false,
            ..live()
        };
        assert_eq!(
            action(3, plain, false),
            Act::Seek {
                code: Seek::END_OF_PART,
                rewind: false,
            }
        );
        // The same code as widget 2's second press, and not the same press:
        // the rewind flag is what tells them apart.
        assert_ne!(action(3, plain, false), action(2, plain, true));
    }

    /// `FUN_10023fb0` case 4 asks `_GetSuperSkipFlag@0`, where the speed row
    /// asks host `+0x88`. A player with `SuperSkip` off has a dead — and so
    /// silent, unhovered, captionless — skip button and a live speed row.
    #[test]
    fn the_skip_button_is_the_super_skip_setting_and_the_speed_row_is_not() {
        let without = State {
            super_skip: false,
            ..live()
        };
        assert!(!HQ.enabled(4, without));
        assert!(HQ.enabled(5, without));
        assert_eq!(action(4, without, false), Act::None);
        assert!(HQ.enabled(4, live()));
        assert_eq!(
            action(4, live(), false),
            Act::Seek {
                code: Seek::SKIP,
                rewind: false,
            }
        );
        // And the same on the other module's table.
        assert!(!Layout::SHINY_DAYS.enabled(4, without));
        assert!(Layout::SHINY_DAYS.enabled(5, without));
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
        let auto = HQ.auto_lit;
        let Auto::Animated { first, frames } = auto else {
            panic!("School Days HQ's widget 0 animates");
        };
        // At rate index 0 a frame lasts 1000 ms; at index 4, 200 ms.
        assert_eq!(auto.frame(0, 0), first);
        assert_eq!(auto.frame(999, 0), first);
        assert_eq!(auto.frame(1000, 0), first + 1);
        assert_eq!(auto.frame(200, 4), first + 1);
        // And it wraps after thirteen.
        assert_eq!(auto.frame(frames as u32 * 1000, 0), first);
    }

    /// The count of hit regions is the whole of what picks a layout, and a
    /// count neither set was recovered against picks none rather than the
    /// nearest one.
    #[test]
    fn the_region_count_picks_the_module() {
        assert_eq!(
            Layout::of(Layout::SCHOOL_DAYS_HQ.widgets).map(|l| l.module),
            Some("SysMenuSDHQ.dll")
        );
        assert_eq!(
            Layout::of(Layout::SHINY_DAYS.widgets).map(|l| l.module),
            Some("SysMenuSD.dll")
        );
        assert!(Layout::of(24).is_none());
        assert!(Layout::of(0).is_none());
    }

    /// The last widget is a different control on the two, and the difference
    /// runs through both the enabled test and the dispatch: School Days HQ's is
    /// the first of ten cells that store a level, and Shiny Days' is a knob
    /// that is never live and is reached from outside the enabled test.
    #[test]
    fn the_last_widget_is_a_cell_on_one_module_and_a_knob_on_the_other() {
        let hq = &Layout::SCHOOL_DAYS_HQ;
        let sd = &Layout::SHINY_DAYS;
        assert!(hq.enabled(0xf, live()));
        assert_eq!(hq.action(0xf, live(), false), Act::Transparency(0));

        assert!(!sd.enabled(0xf, live()));
        assert_eq!(sd.action(0xf, live(), false), Act::GrabKnob);
        // Dead without a record to follow, the same answer that darkens its
        // trough.
        assert_eq!(sd.action(0xf, State::default(), false), Act::None);
    }

    /// Every widget the two strips share dispatches the same way, which is
    /// what makes one [`Act`] serve both.
    #[test]
    fn the_two_strips_dispatch_their_shared_widgets_alike() {
        for widget in 0..Layout::SHINY_DAYS.widgets - 1 {
            assert_eq!(
                Layout::SCHOOL_DAYS_HQ.action(widget, live(), false),
                Layout::SHINY_DAYS.action(widget, live(), false),
                "widget {widget}"
            );
        }
    }

    /// Shiny Days' widget 0 has one lit record and no animation, so the clock
    /// and the rate make no difference to it.
    #[test]
    fn a_lit_widget_0_does_not_animate() {
        let auto = Layout::SHINY_DAYS.auto_lit;
        let Auto::Lit(record) = auto else {
            panic!("Shiny Days' widget 0 is lit, not animated");
        };
        assert_eq!(auto.frame(0, 0), record);
        assert_eq!(auto.frame(60_000, 4), record);
    }

    /// `FUN_100353b0` moves the drawn value as well as the settled one, which
    /// `FUN_10026050` does not — so Shiny Days' bar is at its counter from the
    /// first frame rather than at zero until something raises the gauge.
    #[test]
    fn settling_the_level_gauge_moves_what_is_drawn() {
        let mut fill = gauge::Fill::default();
        assert_eq!(fill.value(), 0.0);
        fill.settle(300);
        assert_eq!(fill.value(), 300.0);
    }

    /// The five steps of `FUN_10035780`, which are `FUN_10026b40`'s over one
    /// counter: read, sound, slide, hold, commit.
    #[test]
    fn the_level_gauge_slides_over_the_ramp_and_then_lowers_itself() {
        let mut fill = gauge::Fill::default();
        fill.settle(100);
        assert_eq!(fill.advance(0, 300).sound, None, "step 0 only reads");
        assert_eq!(
            fill.advance(0, 300).sound,
            Some(crate::ui::menu::SystemSe::Up)
        );
        fill.advance(gauge::RAMP_MS / 2, 300);
        assert_eq!(fill.value(), 200.0, "half way is half the change");
        fill.advance(gauge::RAMP_MS, 300);
        assert_eq!(fill.value(), 300.0);
        // The hold runs from the moment the slide ended.
        assert!(!fill.advance(gauge::RAMP_MS, 300).lowered);
        assert!(!fill.advance(gauge::RAMP_MS + gauge::HOLD_MS, 300).lowered);
        assert!(fill.advance(gauge::RAMP_MS + gauge::HOLD_MS, 300).lowered);
        // And a raise that moves nothing is silent and skips straight to it.
        let mut fill = gauge::Fill::default();
        fill.settle(100);
        fill.advance(0, 100);
        assert_eq!(fill.advance(0, 100).sound, None);
    }

    /// The knob's travel and the alpha it produces, from `FUN_100359c0`: the
    /// run reaches a full 255 where School Days HQ's ten cells stop at 250, and
    /// a fresh bar starts at the solid end.
    #[test]
    fn the_dragged_knob_runs_the_whole_alpha() {
        assert_eq!(slider::alpha(slider::MIN_X), 0);
        assert_eq!(slider::alpha(slider::MAX_X), 255);
        assert_eq!(slider::alpha(slider::INITIAL_X), 255);
        assert_eq!(indicator::alpha(indicator::INITIAL_LEVEL), 250);
        // `FUN_10035cb0` takes both ends of the knob, and nothing past them.
        assert!(slider::on_knob(700.0, 700.0));
        assert!(slider::on_knob(700.0, 700.0 + slider::KNOB_WIDTH));
        assert!(!slider::on_knob(700.0, 699.0));
        assert!(!slider::on_knob(700.0, 700.0 + slider::KNOB_WIDTH + 1.0));
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
/// Shiny Days' transparency control: one knob you take hold of and drag.
///
/// Where School Days HQ gives the `REPLAYMODE` indicator's transparency ten
/// cells to press — see [`indicator`] — Shiny Days gives it a slider. Widget
/// `0xf` is the track, `this+0xb0` is the knob's `x` in the strip's own units,
/// and the alpha is where in its travel the knob has got to.
///
/// Three functions are the whole of it. `FUN_10035cb0` answers whether the
/// pointer is on the knob, which is what a press on widget `0xf` has to pass
/// before the grip is taken; `FUN_100359c0` moves it while the button is held
/// and writes the alpha onto the indicator's sprite; and `FUN_10033270` puts
/// it at [`INITIAL_X`] when the bar loads.
pub mod slider {
    /// `_DAT_1004e320` and `_DAT_1004e318`, both doubles: the knob's travel.
    /// The bed it runs in is 114 wide at x 678, so the knob stops short of
    /// either end of it.
    pub const MIN_X: f32 = 691.0;
    pub const MAX_X: f32 = 768.0;
    /// `_DAT_1004e310`, a double: [`MAX_X`] less [`MIN_X`], which the original
    /// keeps as a constant of its own rather than subtracting.
    pub const TRAVEL: f32 = 77.0;
    /// `_DAT_1004e308`, a double: the alpha at the far end. Unlike HQ's ten
    /// cells, which reach only 250, this run reaches a full 255.
    pub const ALPHA_MAX: f32 = 255.0;
    /// `_DAT_1004e304` — a **float**, not a double like its neighbours: where
    /// `FUN_10033270` puts the knob when the bar loads. That is the far end,
    /// so a fresh bar draws the indicator solid, which is what School Days HQ's
    /// [`indicator::INITIAL_LEVEL`](super::indicator::INITIAL_LEVEL) of 10 does
    /// too.
    pub const INITIAL_X: f32 = 768.0;
    /// `_DAT_10049750` and `_DAT_100497b0`, the half-pixel inset and the `+1`
    /// every sprite on the bar gets.
    pub(super) const HALF: f32 = 0.5;
    pub(super) const ONE: f32 = 1.0;
    /// `DAT_10058ac0`: the knob record's own width, which is how far past its
    /// origin `FUN_10035cb0` will still call a point "on the knob".
    pub const KNOB_WIDTH: f32 = 11.0;

    /// The indicator's alpha with the knob at `x`, as `FUN_100359c0` computes
    /// it: `round((x - MIN_X) / TRAVEL * ALPHA_MAX)`, put in the top byte of an
    /// otherwise white ARGB.
    pub fn alpha(x: f32) -> u8 {
        (((x - MIN_X) / TRAVEL * ALPHA_MAX).round()).clamp(0.0, 255.0) as u8
    }

    /// Whether a pointer at `x` — in the strip's own units — is on a knob whose
    /// origin is at `knob`.
    ///
    /// `FUN_10035cb0`, which tests `knob <= x` and `x <= knob + KNOB_WIDTH`.
    /// Both ends are inclusive: Ghidra renders the first as
    /// `(a < b) != (a == b)`, which is `a <= b`.
    pub fn on_knob(knob: f32, x: f32) -> bool {
        knob <= x && x <= knob + KNOB_WIDTH
    }
}

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
    use crate::ui::menu::SystemSe;

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

    /// Sizes the three pieces from a settled pair of counters.
    pub fn pieces(first: i32, second: i32) -> Pieces {
        let (lead_first, lead_second) = leads(first, second);
        pieces_at(lead_first, lead_second)
    }

    /// Sizes the three pieces from the two leads, as `FUN_10026540` does.
    ///
    /// The leads are the only thing it reads — `this+0x48` and `this+0x4c` —
    /// which is what lets [`Anim`] slide the gauge between two pairs of
    /// counters without the pieces knowing anything about either pair.
    pub fn pieces_at(lead_first: f32, lead_second: f32) -> Pieces {
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

    /// How long the gauge takes to slide from the old lead to the new one.
    ///
    /// `0x5dc` is the cutoff `FUN_10026b40` compares the elapsed time against,
    /// and `_DAT_1003ab20` — a double, `1500.0` — is what it divides by, so the
    /// ramp reaches the new lead exactly as the window closes.
    pub const RAMP_MS: u32 = 1500;

    /// How long the gauge is held at the new lead before it puts itself down.
    ///
    /// The test is `1999 < elapsed`, so it takes a full two seconds to pass.
    pub const HOLD_MS: u32 = 2000;

    /// What one step of the gauge's ramp asks the engine to do.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Tick {
        /// The sound the ramp starts with, played once. `None` on every other
        /// step, and on a raise that turns out to move neither counter.
        pub sound: Option<SystemSe>,
        /// The gauge has finished and lowered itself, which the original does
        /// through host `+0x30(0)` — [`crate::install::progress::Progress::lower_gauge`]
        /// is this engine's side of that.
        pub lowered: bool,
    }

    /// Shiny Days' gauge: one counter, one bar, and the same five steps.
    ///
    /// `FUN_10035780` is School Days HQ's `FUN_10026b40` over a single value.
    /// Step 0 sets the bed and the bar opaque and asks the host for the
    /// counter; step 1 picks the rise or the fall sound, or skips straight to
    /// the hold when nothing moved; step 2 slides for [`RAMP_MS`]; step 3 holds
    /// for [`HOLD_MS`]; step 4 commits and lowers the gauge. The sounds are the
    /// same two, from the same host slot: 4 on a fall and 3 on a rise, which
    /// are [`SystemSe::Down`] and [`SystemSe::Up`].
    ///
    /// **It asks for `001` and nothing else.** Both the ramp's literal at
    /// `0x1004e2b0` and the settle's at `0x1004e2a8` are `L"001"`, so the
    /// second counter School Days HQ's gauge weighs against this one is not
    /// read at all — there is no lead here, just a level.
    #[derive(Debug, Clone, Copy, PartialEq, Default)]
    pub struct Fill {
        /// `this+0x30`: the counter the bar is drawn from once settled.
        shown: f32,
        /// `this+0x34`: the counter it is sliding towards.
        target: f32,
        /// `this+0x2c`: `target - shown`.
        change: f32,
        /// `this+0x3c`: the width the bar is actually drawn at, which is what
        /// slides.
        value: f32,
        /// `this+0x38`.
        step: u32,
        /// `this+0x40`, the tick the slide or the hold started on. `None` is
        /// the original's zero, for the same reason it is on [`Anim`].
        since: Option<u32>,
    }

    impl Fill {
        /// The width the bar is drawn at, before [`Fill`]'s own clamp.
        pub fn value(&self) -> f32 {
            self.value
        }

        /// Puts the gauge at a counter with no ramp: `FUN_100353b0`, vtable
        /// slot `+0x38`, the same slot School Days HQ settles through.
        ///
        /// Unlike [`Anim::settle`] this also moves the drawn value, so the bar
        /// is at the counter from the first frame rather than at zero until
        /// something raises it.
        pub fn settle(&mut self, first: i32) {
            self.shown = first as f32;
            self.value = self.shown;
            self.step = 0;
            self.since = None;
        }

        /// One frame of the ramp, given the wall clock and the counter as the
        /// save holds it now.
        pub fn advance(&mut self, now_ms: u32, first: i32) -> Tick {
            let mut tick = Tick::default();
            match self.step {
                0 => {
                    self.target = first as f32;
                    self.change = self.target - self.shown;
                    self.step = 1;
                }
                1 => {
                    if self.shown == self.target {
                        self.step = 3;
                    } else {
                        self.step = 2;
                        self.since = Some(now_ms);
                        // `this+0x34 <= this+0x30`, with equality already ruled
                        // out above, so this is a strict fall.
                        tick.sound = Some(if self.target < self.shown {
                            SystemSe::Down
                        } else {
                            SystemSe::Up
                        });
                    }
                }
                2 => {
                    let run = now_ms.saturating_sub(self.since.unwrap_or(now_ms));
                    if run < RAMP_MS {
                        self.value = run as f32 * self.change / RAMP_MS as f32 + self.shown;
                    } else {
                        self.value = self.shown + self.change;
                        self.since = Some(now_ms);
                        self.step = 3;
                    }
                }
                3 => {
                    let done = match self.since {
                        Some(at) => now_ms.saturating_sub(at) >= HOLD_MS,
                        None => true,
                    };
                    if done {
                        self.step = 4;
                    }
                }
                _ => {
                    self.since = None;
                    self.step = 0;
                    self.shown = self.target;
                    tick.lowered = true;
                }
            }
            tick
        }
    }

    /// The gauge's ramp: `FUN_10026b40`, and the values it slides between.
    ///
    /// # Where it runs
    ///
    /// `FILM::MenuBar` is a graphics module, and `DXGraphicModuleList`'s four
    /// passes call each module's `+0x0c`, `+0x10`, `+0x14` and `+0x18` in turn.
    /// `FUN_0040e540` — the render frame — runs the `+0x10` pass, then `+0x14`
    /// inside `BeginScene`/`EndScene`, then `+0x18`. The bar's `+0x10` is
    /// `FUN_10024c60`, three lines long:
    ///
    /// ```text
    /// if (host->+0x154())  { FUN_10026b40(this); FUN_10026540(this); }
    /// ```
    ///
    /// So while a delta has the gauge raised, this steps once per rendered
    /// frame and the pieces are re-sized from whatever it leaves in the leads.
    /// Nothing steps it while the gauge is down.
    ///
    /// # The five steps
    ///
    /// `this+0x44` is the step number, and the machine runs 0, 1, 2, 3, 4 and
    /// back to 0 across successive frames:
    ///
    /// ```text
    /// 0  make the bed and the three pieces opaque; read `001` and `002` as
    ///    the targets; keep the leads in force as the ramp's start
    /// 1  play the rise or the fall, and start the clock — or, if neither
    ///    counter moved, skip straight to the hold and play nothing
    /// 2  slide the leads for RAMP_MS, then snap to the target and restart
    ///    the clock
    /// 3  hold for HOLD_MS
    /// 4  commit the targets as the values now on screen, and lower the gauge
    /// ```
    ///
    /// # Which sound, and why those two
    ///
    /// Step 1 plays host `+0x50(3)` when the counter it is watching went up and
    /// `+0x50(4)` when it went down — [`SystemSe::Up`] and [`SystemSe::Down`],
    /// the `SeUp` and `SeDown` keys of `FILMENGINE.INI`. The counter it watches
    /// is `001` whenever `001` moved at all; only when `001` stood still does
    /// `002`'s direction decide.
    ///
    /// # The two values it slides between
    ///
    /// `this+0x34` and `this+0x3c` are the counters **as the gauge is
    /// currently drawing them**, not as the save holds them. Only two things
    /// write them: [`Anim::settle`] and step 4. That is what gives the ramp
    /// something to slide from — a gauge sized straight off the save would
    /// already be at the new lead by the time the ramp started.
    ///
    /// `FILM::MenuBar` is a static object and its constructor `FUN_100216e0`
    /// initialises none of these, so they all start at zero: before the first
    /// settle the gauge is a tie.
    #[derive(Debug, Clone, Copy, PartialEq, Default)]
    pub struct Anim {
        /// `this+0x34` / `this+0x3c`: the counters the gauge is drawing.
        shown: (f32, f32),
        /// `this+0x38` / `this+0x40`: the counters it is sliding towards, read
        /// from the save at step 0.
        target: (f32, f32),
        /// `this+0x2c` / `this+0x30`: `target - shown`, each side's move.
        change: (f32, f32),
        /// `this+0x48` / `this+0x4c`: the leads the pieces are sized from.
        leads: (f32, f32),
        /// `this+0x50` / `this+0x54`: the leads the ramp started at.
        from: (f32, f32),
        /// `this+0x44`.
        step: u32,
        /// `this+0x58`, the tick the ramp or the hold started on.
        ///
        /// `None` is the original's zero. It matters in one place: step 1's
        /// no-change arm goes to the hold without stamping this, and the hold
        /// then measures against a `timeGetTime` that is never near zero, so it
        /// expires at once. A wall clock that starts at zero would instead hold
        /// for two seconds, which is why this is an option and not a `0`.
        since: Option<u32>,
    }

    impl Anim {
        /// The leads the three pieces are sized from.
        pub fn leads(&self) -> (f32, f32) {
            self.leads
        }

        /// Puts the gauge at a pair of counters with no ramp: `FUN_10026050`.
        ///
        /// This happens **once a film run**, not once a script. The engine has
        /// two call sites for MenuBar vtable `+0x38` and only one of them can
        /// be reached: `FUN_00423a70`, which is the run starting — its first
        /// acts are `_LoadInitScript@4` and `_ZeroReset@4`. The other, in
        /// `FUN_00424020`, sits under `if (this+0x560)`, and `+0x560` is
        /// written in exactly two places, both of them `= 0`. So no script
        /// ending settles the gauge.
        ///
        /// That is what gives the ramp room to run. A choice box sits in the
        /// last seconds of its script — `01-00-B00` raises one at 00:08:12 of a
        /// script that ends at 00:13:12 — so a raise that a script boundary
        /// could cut short would usually be cut short. Instead it keeps sliding
        /// over the opening of the next scene.
        ///
        /// The caller lowers the gauge afterwards, which is the rest of
        /// `FUN_10026050`.
        pub fn settle(&mut self, first: i32, second: i32) {
            self.shown = (first as f32, second as f32);
            self.leads = leads(first, second);
            self.step = 0;
            self.since = None;
        }

        /// One frame of the ramp, given the wall clock and the counters as the
        /// save holds them now.
        pub fn advance(&mut self, now_ms: u32, first: i32, second: i32) -> Tick {
            let mut tick = Tick::default();
            match self.step {
                0 => {
                    self.target = (first as f32, second as f32);
                    self.from = self.leads;
                    self.change = (self.target.0 - self.shown.0, self.target.1 - self.shown.1);
                    self.step = 1;
                }
                1 => {
                    // `001` decides whenever it moved at all; `002` only gets
                    // to when `001` stood still. Both arms test `new <= old`,
                    // but equality has already been ruled out by the arm that
                    // was taken, so each is really a strict `<`.
                    let fell = if self.shown.0 != self.target.0 {
                        Some(self.target.0 < self.shown.0)
                    } else if self.shown.1 != self.target.1 {
                        Some(self.target.1 < self.shown.1)
                    } else {
                        None
                    };
                    match fell {
                        None => self.step = 3,
                        Some(fell) => {
                            self.step = 2;
                            self.since = Some(now_ms);
                            tick.sound = Some(if fell { SystemSe::Down } else { SystemSe::Up });
                        }
                    }
                }
                2 => {
                    let run = now_ms.saturating_sub(self.since.unwrap_or(now_ms));
                    if run < RAMP_MS {
                        let moved =
                            run as f32 * (self.change.0 - self.change.1) * SCALE / RAMP_MS as f32;
                        self.leads = (self.from.0 + moved, self.from.1 - moved);
                    } else {
                        let d = (self.target.0 - self.target.1) * SCALE;
                        self.leads = (d, -d);
                        self.since = Some(now_ms);
                        self.step = 3;
                    }
                }
                3 => {
                    let done = match self.since {
                        Some(at) => now_ms.saturating_sub(at) >= HOLD_MS,
                        None => true,
                    };
                    if done {
                        self.step = 4;
                    }
                }
                _ => {
                    self.since = None;
                    self.step = 0;
                    self.shown = self.target;
                    tick.lowered = true;
                }
            }
            tick
        }
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
    use crate::ui::menu::SystemSe;

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

    /// The gauge does not jump to the new counters; it slides. Nothing else in
    /// the bar has a value that lags its source, and getting this wrong is
    /// invisible in a still.
    #[test]
    fn a_raise_slides_from_the_old_pair_to_the_new_one() {
        let mut anim = Anim::default();
        anim.settle(60, 60);
        assert_eq!(anim.leads(), (0.0, -0.0));
        // Step 0 reads the targets, step 1 starts the clock.
        anim.advance(0, 70, 60);
        anim.advance(0, 70, 60);
        assert_eq!(anim.leads(), (0.0, -0.0), "the ramp has not moved yet");
        anim.advance(RAMP_MS / 2, 70, 60);
        assert_eq!(anim.leads(), (12.5, -12.5), "half way to a lead of ten");
        anim.advance(RAMP_MS, 70, 60);
        assert_eq!(anim.leads(), (25.0, -25.0));
    }

    /// A rise and a fall are different sounds, and which counter decides is
    /// not symmetric: `001` decides whenever it moved at all.
    #[test]
    fn the_ramp_plays_the_direction_of_the_counter_that_moved() {
        let sound = |from: (i32, i32), to: (i32, i32)| {
            let mut anim = Anim::default();
            anim.settle(from.0, from.1);
            anim.advance(0, to.0, to.1);
            anim.advance(0, to.0, to.1).sound
        };
        assert_eq!(
            sound((60, 60), (70, 60)),
            Some(crate::ui::menu::SystemSe::Up)
        );
        assert_eq!(sound((60, 60), (50, 60)), Some(SystemSe::Down));
        assert_eq!(
            sound((60, 60), (60, 70)),
            Some(crate::ui::menu::SystemSe::Up)
        );
        assert_eq!(sound((60, 60), (60, 50)), Some(SystemSe::Down));
        // `001` fell and `002` rose: the first counter's direction is the one
        // that plays, and the second's is not consulted.
        assert_eq!(sound((60, 60), (50, 70)), Some(SystemSe::Down));
        // Nothing moved, so there is no ramp and no sound.
        assert_eq!(sound((60, 60), (60, 60)), None);
    }

    /// The gauge puts itself down when its hold is over, which is what takes it
    /// off the screen in ordinary play — the end of the script is the other
    /// way, and most scripts run well past three and a half seconds.
    #[test]
    fn the_ramp_lowers_the_gauge_once_it_has_held_the_new_lead() {
        let mut anim = Anim::default();
        anim.settle(60, 60);
        anim.advance(0, 70, 60);
        anim.advance(0, 70, 60);
        // The ramp's own window, then the hold, then the step that commits.
        assert!(!anim.advance(RAMP_MS, 70, 60).lowered);
        assert!(!anim.advance(RAMP_MS + HOLD_MS - 1, 70, 60).lowered);
        assert!(!anim.advance(RAMP_MS + HOLD_MS, 70, 60).lowered);
        assert!(anim.advance(RAMP_MS + HOLD_MS, 70, 60).lowered);
        // And it is left holding the pair it slid to, so the next raise has
        // somewhere to start from.
        assert_eq!(anim.leads(), (25.0, -25.0));
        anim.advance(0, 80, 60);
        anim.advance(0, 80, 60);
        anim.advance(RAMP_MS, 80, 60);
        assert_eq!(anim.leads(), (50.0, -50.0));
    }

    /// A raise that moves neither counter skips the ramp and comes straight
    /// down: the hold measures against a stamp that was never taken.
    #[test]
    fn a_raise_that_moves_nothing_comes_straight_back_down() {
        let mut anim = Anim::default();
        anim.settle(60, 60);
        anim.advance(0, 60, 60);
        assert_eq!(anim.advance(0, 60, 60).sound, None);
        assert!(!anim.advance(0, 60, 60).lowered, "one step for the hold");
        assert!(anim.advance(0, 60, 60).lowered);
    }
}
