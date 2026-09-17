//! The clothing-selection screen, which only one of the two titles has.
//!
//! `_SystemInit@8` in `SysMenuSD.dll` has a case for mode 9 that hands over the
//! static object at `DAT_1005b898`; its static-init thunk `FUN_10048860` calls
//! the constructor `FUN_1000c470`, which installs `MENU::DressSelect::vftable`
//! at `0x1004a6ec`. `SysMenuSDHQ.dll` has no case 9 at all, so on that module
//! [`crate::ui::paths::Paths::stem`] finds no screen and the mode is
//! unavailable — see [`crate::ui::menu::Mode::DRESS_SELECT`].
//!
//! # One object, two hit maps
//!
//! The screen is not two modes. `MENU::DressSelect` keeps one hit map at a time
//! and swaps it: `FUN_1000d7f0` loads `System/DressSelect/DressSelect*.cmap` and
//! `FUN_1000d8c0` loads `System/DressSelect/Popup/Popup_Select*.cmap` into the
//! same member, each picking its widescreen variant from host `+0xcc`, `+0xd0`
//! and `+0xe4` the way every other screen does. [`Phase`] is the module's
//! `+0x140`, which says whether the popup is up.
//!
//! # A click does not land on the popup, it slides there
//!
//! Committing does not raise the popup: it starts [`Slide`], and the two
//! dresses travel to the middle of the screen over [`SLIDE_FRAMES`] frames
//! before `FUN_1000e440` arm 1 takes the popup's art and hit map. Answering it
//! no runs the same travel backwards. [`Slide::drawn`] is what the screen shows
//! at each point of that, and [`Slide::tick`] is the arm that moves it.
//!
//! # The records, from `FUN_1000d980`, `FUN_1000ded0` and `FUN_1000ef80`
//!
//! One table of six 24-byte records at `DAT_10054920`, `x y w h src_x src_y` as
//! floats:
//!
//! ```text
//! rec 0   ( 64, 0) 273x450  src (  1,   1)   left dress, resting
//! rec 1   (463, 0) 273x450  src (275,   1)   right dress, resting
//! rec 2   ( 64, 0) 273x450  src (  1, 452)   left dress, lit
//! rec 3   (463, 0) 273x450  src (275, 452)   right dress, lit
//! rec 4   (274, 398) 115x26 src (  1,   1)   popup, widget 0
//! rec 5   (411, 398) 115x26 src (117,   1)   popup, widget 1
//! ```
//!
//! The first four are cut from `DressSelect_Chip.png` and the last two from
//! `Popup/Popup_Select_Chip.png`. Records 2 and 3 hold the *same destination
//! rectangle* as 0 and 1 and differ only in `src_y`, which is why the three
//! functions above can disagree about which record they anchor a slide to and
//! still place the sprite identically — `FUN_1000e440` slides against records 0
//! and 1 where `FUN_1000ded0` sets the same sprites up from records 1 and 2.
//! The player's own sheets confirm the split: `DressSelect_Chip.png` is 548x902,
//! exactly two columns of 273 and two rows of 450, and
//! `Popup_Select_Chip.png` is 232x27, a single row with no lit variant at all.
//!
//! # The dress is drawn, not revealed
//!
//! Every other screen in these modules has opaque base art and lights a widget
//! by drawing its chip sprite over it. This one's base is
//! `System/Screen/Transparence.png`, the shared full-screen transparent plate
//! (`FUN_1000d980` hands it to `FUN_10010620` paired with the chip sheet), so
//! **both dresses are sprites that are always drawn** — `FUN_1000c740` walks
//! `+0xc8` and `+0xcc` unconditionally — and the lit record goes over the one
//! under the pointer. The moving background is `STARTSCRIPT.INI`'s `[DressBG]`
//! playing behind the plate; see [`background`].
//!
//! `DressSelect_Text.png` is a caption over that, loaded by `FUN_1000cce0` as a
//! full-screen plate and drawn by `FUN_1000c740` only while the dresses are at
//! rest — it goes the moment they start moving, and the popup covers it rather
//! than sitting under it. See [`Drawn`].
//!
//! # The dispatch, from `FUN_1000ded0`
//!
//! `FUN_1000dea0` is the availability test and is `0 <= widget <= 1`: neither
//! dress is ever locked, on either hit map. What a click does depends on
//! `+0x140` rather than on which map it came through, which is the whole of
//! [`action`]:
//!
//! * While choosing, either widget commits. The module tells the host the
//!   choice at once — host `+0x48(1)` for widget 0 and `+0x48(0)` for widget 1
//!   — and starts the slide; `FUN_1000e440` arm 1 loads the popup's art and hit
//!   map and raises `+0x140` once it has run.
//! * While confirming, widget 0 sets `+0xfc` and widget 1 clears `+0x140` and
//!   sets `+0x154` to 3, which is `FUN_1000e440`'s arm that slides the dresses
//!   back apart and reloads the main hit map.
//!
//! `+0xfc` is what `getNextMode` case 9 reads through `FUN_10001a10`, and it
//! answers mode 1 — leave the menus and play. `+0x88` answers mode -1, the
//! confirm popup, and anything else falls through to mode 2, the title.
//!
//! # What the choice reaches
//!
//! Host slot `+0x48` is `FUN_0041dc50` in `SHINYDAYS.exe`, which stores the
//! argument and raises a flag beside it. The interface is a secondary base
//! subobject installed at `[object + 0x2c]` — `FUN_0041d660` writes the vtable
//! `0x0048e50c` there, and `FUN_004167e0` hands the DLL `this + 0x2c` — so the
//! member the setter spells `+0x7d0` is the object's `+0x7fc`.
//!
//! Slot `+0x44` is the matching reader, `FUN_0041dc40`, and its callers are in
//! `RouteProcSD.dll`: `FUN_1004da70` and `FUN_100515e0` each play a scene block
//! and then append a letter to its name from this value — `A` when it is
//! non-zero, `B` when it is zero.
//!
//! ```text
//! FUN_1004da70   REP04_S1_B03  ->  REP04_S1_B03A / REP04_S1_B03B
//! FUN_100515e0   REP04_YX_A01  ->  REP04_YX_A01A / REP04_YX_A01B
//! ```
//!
//! So widget 0 is the `A` dress and widget 1 the `B` dress, and exactly two
//! scenes in the shipped route branch on it. That the interface is the one
//! `RouteProcSD` holds is confirmed by its own use of the neighbouring slots:
//! `+0x8(wstr)`, `+0xc(wstr, int)`, `+0x10(wstr) -> int` and `+0x1c(wstr, int)`
//! match the signatures at `0x0048e50c` exactly.
//!
//! # How the screen is entered
//!
//! The title screen's `START` widget asks for it. `FUN_1002fc60` case 0 — the
//! title module's dispatch — writes **9** into its next-mode member `+0xe0`,
//! and `_getNextMode@8` case 2 returns that member verbatim through
//! `FUN_100019b0`, a plain load with no clamp. `SysMenuSDHQ.dll`'s matching
//! arm, `FUN_100207a0` case 0, writes 1 instead, which is the whole of the
//! difference between the two titles here; see [`crate::ui::menu::Mode`].
//!
//! The executable takes it from there without ever naming the number.
//! `FUN_004158c0` is the mode pump: it loads `+0x2d8`, adds one, and jumps
//! through a four-entry table for modes -1 through 2 — every other mode,
//! 9 among them, falls to the **default arm**, which is
//! `FUN_00413250(mode)` with the mode passed as a parameter and its return
//! stored back into `+0x2d8`. That handler switches on its own phase counter
//! `+0x2dc` and, in phase 1, tests the mode twice: `mode == -1` and
//! `mode == 9` both skip `FUN_0041aa10`, the call that silences what is
//! already playing, and `mode == 9` additionally takes the arm described under
//! [`BACKGROUND_KEY`] before handing the mode to `_SystemInit@8` through
//! `FUN_004167e0`.
//!
//! So nothing writes a literal 9 in the executable at all: it arrives as a
//! parameter from the module, which is why a scan of the exe for the constant
//! finds nothing.

use days_ui::atlas::{Atlas, Widget};

use crate::install::ini::Ini;
use crate::ui::screen::Cut;

/// Whether the confirm popup is up: `MENU::DressSelect` `+0x140`.
///
/// `FUN_1000e440` arm 1 raises it in the same breath as it takes the popup's
/// art and hit map, once the commit slide has run; `FUN_1000ded0` drops it the
/// moment the popup is answered no, thirty frames before arm 3 puts the main
/// hit map back. So it is the screen's own state and not a name for which map
/// is loaded — for the length of the slide back apart, the popup's map is
/// still the loaded one. Neither map can be told from the other by its widget
/// count: both carry two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// The main hit map: the two dresses.
    #[default]
    Choosing,
    /// The popup, over the dress the player committed to.
    Confirming {
        /// The widget that was clicked, which is also the index the two
        /// dresses are drawn in — not the value the host was told. See
        /// [`host_value`].
        chosen: usize,
    },
}

/// What a click on the screen does, from `FUN_1000ded0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// The widget is not one of the two, or the click is not live.
    None,
    /// Commit to this dress and raise the popup. The host is told at once.
    Commit(usize),
    /// The popup's yes: `+0xfc`, which `getNextMode` answers with mode 1.
    Accept,
    /// The popup's no: back to the two dresses.
    Cancel,
}

/// `STARTSCRIPT.INI`'s key for what plays behind the transparent plate.
///
/// The shipped file sets it to `System/DressSelect/sentakuBG_03.wmv`, and the
/// install ships `sentakuBG_03.png` beside it. See [`background`] for which of
/// the two arms the value takes.
pub const BACKGROUND_KEY: &str = "DressBG";

/// What the mode-9 arm does with [`BACKGROUND_KEY`]'s value.
///
/// **The key chooses between the two by its own spelling**, in
/// `FUN_00413250`'s mode-9 arm: it hands the value and the literal `L".png"`
/// at `0x0048df38` to `wcsstr` — the import at IAT slot `0x0048c228`,
/// `MSVCR90.dll!wcsstr`, reached through the thunk at `0x0048330a` — and a hit
/// takes the still arm while a miss takes the movie arm. It is `wcsstr` and
/// not a suffix test, so what it asks is whether `.png` appears anywhere in
/// the value at all. Either way `FUN_00408da0` starts what was set up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Background {
    /// One picture, loaded straight into a texture and held: `FUN_00420df0`
    /// on the object at `this + 0x74`, with `+0x304` cleared so the per-frame
    /// arm below never runs.
    Still(String),
    /// A clip, played on a loop from its first frame: `FUN_004212a0` opens it
    /// on the `MENU::MovieView` at `this + 0x178` — the class `FUN_00421b20`
    /// names — `FUN_00421110` sets its frame to 0, and `+0x304` is set so the
    /// pump advances it. See [`BACKGROUND_FPS`].
    Movie(String),
}

impl Background {
    /// The asset path, as the INI spells it.
    pub fn path(&self) -> &str {
        match self {
            Background::Still(path) | Background::Movie(path) => path,
        }
    }
}

/// What goes behind the transparent plate, from `STARTSCRIPT.INI`.
///
/// `None` when the key is absent or empty — the arm still runs in the
/// original, on an empty string, and both loaders report a file they cannot
/// open; here that is one more missing asset, so the screen draws over what is
/// already behind it.
pub fn background(start: &Ini) -> Option<Background> {
    let value = start.get(BACKGROUND_KEY)?;
    if value.is_empty() {
        return None;
    }
    Some(if value.contains(".png") {
        Background::Still(value.to_string())
    } else {
        Background::Movie(value.to_string())
    })
}

/// The clock the movie arm advances its frame on: 24 frames a second.
///
/// The frame is a member of its own — `MENU::MovieView` `+0x3c`, which
/// `FUN_00421b20` zeroes and only `FUN_00421110` ever writes — and the mode
/// pump sets it from the wall clock: `FUN_00413250` stores `timeGetTime()` in
/// `+0x30c` as the screen comes up (case 2) and its two running phases (cases
/// 3 and 8) set the frame to `round(elapsed_ms * DAT_004b31e0) / 1000`.
/// `DAT_004b31e0` is written once, `FUN_00437e00`'s `mov dword ptr
/// [0x004b31e0], 0x18` — 24, the same frame clock `.ORS` timelines are
/// scheduled on.
///
/// # The clip loops
///
/// That frame number only ever goes up, and it is **not** evidence that the
/// clip is played once: the loader rebases its timestamps instead of rewinding
/// the count. `wmvLoader`'s pump `FUN_004532f0` asks
/// `IWMSyncReader::GetNextSample` for a sample, and on
/// `NS_E_NO_MORE_SAMPLES` — `FUN_00451820`'s `0xc00d0bcf` — it raises the
/// play counter `+0xfc`, adds the clip's whole length `+0x88` to the running
/// offset `+0x100` that every later sample's timestamp is taken from, and
/// calls `FUN_00451fe0`. That seeks the reader back to the start —
/// `IWMSyncReader::SetRange(0, 0)` through `FUN_004511a0`, vtable slot
/// `+0x14` — and says so in its own debug line, `L"先頭へシーク\n"`.
///
/// It stops only when it has played a set number of times: `FUN_00451fe0`
/// returns without seeking when `+0xf8` is non-zero and has been reached.
/// `+0xf8` is that play count, `FUN_004523c0` sets it, and `FUN_004212a0`
/// passes `(its second argument == 0)` — `setz cl` at `0x004213de`. The
/// mode-9 arm's second argument is 1 (`push ebx` at `0x00413379`, with `ebx`
/// 1 throughout `FUN_00413250`), so the count is zero and **the background
/// loops for as long as the screen is up**.
pub const BACKGROUND_FPS: f64 = 24.0;

/// How many ticks the slide runs for: `_DAT_1004a750`, a double.
///
/// `FUN_1000e440` adds `_DAT_100497b0` — 1.0, also a double — to `+0x12c` on
/// each tick of the arm and finishes when it reaches this, moving each dress a
/// thirtieth of its travel per tick.
///
/// # The tick is a presented frame
///
/// Nothing paces the module's update but the picture: `FUN_004011e0`'s message
/// loop calls `FUN_004158c0` once round, whose default arm runs
/// `FUN_00413250`, whose case 3 calls the module's update and then
/// `FUN_00409690` — which ends in `IDirect3DDevice9::Present`, device vtable
/// `+0x44`. The device is created by `FUN_00408cd0` from parameters
/// `FUN_00408ac0` builds, and that `memset`s the 0x38-byte structure and never
/// writes `+0x34`, so `PresentationInterval` is `D3DPRESENT_INTERVAL_DEFAULT`:
/// one vertical retrace. So the slide is thirty refreshes of the player's
/// display, and this engine gives it thirty passes round its own loop, which
/// vsync paces the same way. See [`crate::ui::menu::Menu::tick`].
pub const SLIDE_FRAMES: f32 = 30.0;

/// Where the left dress ends up once the slide finishes: `_DAT_1004a740`.
///
/// The two are ten pixels apart and both sprites are 273 wide, so the dresses
/// finish stacked in the middle of the 800-wide layout with the chosen one on
/// top — this is the "moving to the centre" the screen does, not two dresses
/// side by side.
pub const SLID_LEFT_X: f32 = 258.5;

/// Where the right dress ends up: `_DAT_1004a748`.
pub const SLID_RIGHT_X: f32 = 268.5;

/// Where one of the two dresses lands, whichever of them was chosen.
///
/// `FUN_1000ded0` sets each sprite's travel to the difference between its
/// resting `x` and the constant for the side it came from, not the side it was
/// clicked on: the left dress always ends at [`SLID_LEFT_X`] and the right at
/// [`SLID_RIGHT_X`]. Only which of them is lit, and so which is drawn on top,
/// changes.
fn slid_x(widget: usize) -> f32 {
    if widget == 0 {
        SLID_LEFT_X
    } else {
        SLID_RIGHT_X
    }
}

/// How many widgets each of the screen's two hit maps carries.
pub const WIDGETS: usize = 2;

/// The dispatch, from `FUN_1000ded0`.
///
/// `FUN_1000dea0` rejects anything outside `0..=1` before the switch, on both
/// maps, so an out-of-range widget is [`Act::None`] rather than a no-op click
/// that still plays a sound.
pub fn action(phase: Phase, widget: usize) -> Act {
    if widget >= WIDGETS {
        return Act::None;
    }
    match phase {
        Phase::Choosing => Act::Commit(widget),
        // The popup's widget 0 sets `+0xfc` and returns 0 — the update pump
        // reads that as "this screen is finished" — and widget 1 drops
        // `+0x140` and sets the slide-back phase.
        Phase::Confirming { .. } => match widget {
            0 => Act::Accept,
            _ => Act::Cancel,
        },
    }
}

/// What the host is told for a chosen widget: host `+0x48`'s argument.
///
/// `FUN_1000ded0` calls it with 1 for widget 0 and 0 for widget 1, and
/// `RouteProcSD.dll` reads the value back through slot `+0x44` to pick the `A`
/// variant of a scene block when it is non-zero and the `B` variant when it is
/// zero. So widget 0 is `A`.
pub fn host_value(widget: usize) -> u32 {
    u32::from(widget == 0)
}

/// The lit sprite for one of the two dresses: records 2 and 3.
///
/// These follow the two the hit map covers, so they are the leading entries of
/// [`Atlas::extras`]. They are looked up rather than assumed: a module whose
/// table does not carry them yields `None` and the dress is drawn resting,
/// which is a screen missing its highlight rather than a screen that will not
/// draw.
pub fn lit(atlas: &Atlas, widget: usize) -> Option<Widget> {
    if widget >= WIDGETS {
        return None;
    }
    atlas.extras.get(widget).copied()
}

/// Every sprite the commit slide can draw, in their resting rectangles.
///
/// The two dresses and the two lit records behind them: which pair a slide
/// uses depends on which dress is clicked, and all four are the same four
/// however it goes. They are here so that a caller can bring them to the size
/// they are drawn at before the click that starts the slide — what identifies
/// a realized cut is its source and its size, and neither moves while the
/// slide runs; only where it lands does. See [`Slide::cuts`] and
/// [`crate::ui::menu::Menu::warm_layers`].
pub fn slide_cuts(atlas: &Atlas) -> Vec<Cut> {
    (0..WIDGETS)
        .flat_map(|widget| [atlas.widgets.get(widget).copied(), lit(atlas, widget)])
        .flatten()
        .map(|source| {
            let (w, h) = (source.dst.width as f32, source.dst.height as f32);
            Cut {
                src: (source.src_x as f32, source.src_y as f32, w, h),
                dst: (source.dst.x as f32, source.dst.y as f32, w, h),
            }
        })
        .collect()
}

/// The two dresses' resting sprites, which this screen draws itself.
///
/// `FUN_1000c740` walks `+0xc8` and `+0xcc` before anything else and draws both
/// every frame, because the base art under them is the transparent plate. Every
/// other screen leaves a resting widget to its base art.
pub fn resting(atlas: &Atlas) -> Vec<Widget> {
    atlas.widgets.iter().take(WIDGETS).copied().collect()
}

/// What the screen draws this frame, from `FUN_1000c740`'s two gates.
///
/// Everything after the two dresses is drawn under one of two conditions:
/// `+0x13c == 0 && +0x140 == 0` for the dress under the pointer and the
/// caption, and `+0x13c != 0 && +0x140 != 0` for the popup and its own hovered
/// widget. The two flags disagree for exactly as long as a slide is running,
/// and then neither block draws — which is why the caption goes as soon as the
/// dresses start moving rather than when the popup arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drawn {
    /// Both dresses at rest, the one under the pointer lit, and the caption.
    Dresses,
    /// The two dresses wherever the slide has them, and nothing else.
    Sliding,
    /// The popup over the two dresses, which stay where the slide left them.
    Popup,
}

/// Which arm of `FUN_1000e440` the slide is in: `MENU::DressSelect` `+0x154`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Step {
    /// Arm 0, the dresses moving together.
    #[default]
    Together,
    /// Arm 1, which takes the popup's art and hit map and raises `+0x140`. It
    /// is a tick of its own: arm 0 only sets `+0x154` to 1 when it reaches the
    /// end, so the popup comes up the frame after the dresses have met.
    Raise,
    /// Arm 2, which the module has no code for: the popup is up and the slide
    /// sits where it left off.
    Held,
    /// Arm 3, the dresses moving back apart, which `FUN_1000ded0` selects when
    /// the popup is answered no.
    Apart,
}

/// The slide the two dresses make on the way to the popup and back.
///
/// The fields are the module's own members, and the arithmetic is
/// `FUN_1000e440`'s: each of the two sprites carries how far it has to travel
/// and how far it has gone, and a tick adds a thirtieth of the first to the
/// second. Both sprites are indexed in **draw order** — `FUN_1000c740` draws
/// `+0xc8` and then `+0xcc`, and `FUN_1000ded0` puts the unchosen dress's
/// resting record in the first and the chosen dress's lit record in the second,
/// so the chosen one is always on top.
#[derive(Debug, Clone, Default)]
pub struct Slide {
    /// `+0x158`: the widget that was clicked, which is the one drawn lit.
    chosen: usize,
    /// `+0x144` and `+0x148`: how far each sprite travels, signed.
    travel: [f32; 2],
    /// `+0x14c` and `+0x150`: how far each has gone.
    offset: [f32; 2],
    /// `+0x12c`: how many ticks the running arm has had.
    frame: f32,
    /// `+0x154`.
    step: Step,
    /// `+0x13c`: whether the screen is showing the slide's two sprites rather
    /// than the loaded hit map's widgets. `FUN_1000d980` clears it on the way
    /// into the screen along with `+0x140`, `+0x12c` and `+0x154`.
    running: bool,
}

/// What a tick of the slide asks the screen to do once an arm finishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slid {
    /// The dresses have met: take the popup's art and hit map and raise
    /// [`Phase::Confirming`]. `FUN_1000e440` arm 1.
    Popup,
    /// They are back where they started: load the main hit map again. The tail
    /// of arm 3, which re-lays both sprites from records 0 and 1 and calls
    /// `FUN_1000d7f0`.
    Dresses,
}

impl Slide {
    /// Which widget each of the two sprites carries, in draw order.
    fn slots(&self) -> [usize; WIDGETS] {
        if self.chosen == 0 {
            [1, 0]
        } else {
            [0, 1]
        }
    }

    /// `FUN_1000ded0`'s commit arm: the travel for a dress that has just been
    /// clicked, from the records the screen's own table holds.
    ///
    /// It writes every member but `+0x154`, which is why a click during the
    /// slide back apart keeps that arm and carries the new dress's travel into
    /// it. A module whose table has no record for a widget leaves that sprite
    /// with nowhere to go, and it stays where it is rather than the screen
    /// refusing to commit.
    pub fn commit(&mut self, atlas: &Atlas, chosen: usize) {
        self.chosen = chosen;
        for (slot, widget) in self.slots().into_iter().enumerate() {
            let from = atlas
                .widgets
                .get(widget)
                .map_or(slid_x(widget), |w| w.dst.x as f32);
            self.travel[slot] = slid_x(widget) - from;
            self.offset[slot] = 0.0;
        }
        self.frame = 0.0;
        self.running = true;
    }

    /// The popup's no: `FUN_1000ded0` sets `+0x154` to 3 and leaves the rest of
    /// the members alone, so the slide runs back from wherever it stopped.
    pub fn cancel(&mut self) {
        self.step = Step::Apart;
    }

    /// One host tick of `FUN_1000e440`'s slide, before it looks at the pointer.
    pub fn tick(&mut self) -> Option<Slid> {
        if !self.running {
            return None;
        }
        match self.step {
            Step::Together => {
                self.advance(1.0);
                if self.frame >= SLIDE_FRAMES {
                    // The arm snaps each sprite onto its travel rather than
                    // leaving it on the thirtieth accumulation, and the tick
                    // that does so is still drawn.
                    self.offset = self.travel;
                    self.frame = 0.0;
                    self.step = Step::Raise;
                }
                None
            }
            Step::Raise => {
                self.step = Step::Held;
                Some(Slid::Popup)
            }
            Step::Held => None,
            Step::Apart => {
                self.advance(-1.0);
                if self.frame >= SLIDE_FRAMES {
                    // The arm re-lays both sprites from records 0 and 1 with
                    // no offset at all rather than from where thirty
                    // subtractions left them, which is a clear here — thirty
                    // thirtieths of 194.5 come back a fraction short.
                    self.offset = [0.0; WIDGETS];
                    self.frame = 0.0;
                    self.step = Step::Together;
                    self.running = false;
                    return Some(Slid::Dresses);
                }
                None
            }
        }
    }

    /// One tick of travel on both sprites, in the direction the arm runs.
    fn advance(&mut self, direction: f32) {
        for slot in 0..WIDGETS {
            self.offset[slot] += direction * self.travel[slot] / SLIDE_FRAMES;
        }
        self.frame += 1.0;
    }

    /// What the screen draws while this slide is where it is: `+0x13c` against
    /// `+0x140`. See [`Drawn`].
    pub fn drawn(&self, phase: Phase) -> Drawn {
        match (self.running, phase) {
            (true, Phase::Confirming { .. }) => Drawn::Popup,
            (false, Phase::Choosing) => Drawn::Dresses,
            _ => Drawn::Sliding,
        }
    }

    /// Whether the next tick moves something, which is every part of the slide
    /// but the wait while the popup is up.
    pub fn moving(&self) -> bool {
        self.running && self.step != Step::Held
    }

    /// The widget the slide is carrying to the middle.
    pub fn chosen(&self) -> usize {
        self.chosen
    }

    /// The two dresses as the screen draws them, in draw order.
    ///
    /// `FUN_1000ef80` re-derives exactly these when the display mode changes
    /// mid-commit, from the same records and the same offsets, so a resolution
    /// change during the slide does not move them.
    pub fn cuts(&self, atlas: &Atlas) -> Vec<Cut> {
        self.slots()
            .into_iter()
            .enumerate()
            .filter_map(|(slot, widget)| {
                let source = if widget == self.chosen {
                    // A table without the lit records draws the chosen dress
                    // resting rather than not at all, the same answer [`lit`]
                    // gives the highlight.
                    lit(atlas, widget).or_else(|| atlas.widgets.get(widget).copied())?
                } else {
                    atlas.widgets.get(widget).copied()?
                };
                let (w, h) = (source.dst.width as f32, source.dst.height as f32);
                Some(Cut {
                    src: (source.src_x as f32, source.src_y as f32, w, h),
                    dst: (
                        source.dst.x as f32 + self.offset[slot],
                        source.dst.y as f32,
                        w,
                        h,
                    ),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use days_ui::Rect;

    fn widget(x: u32, y: u32, width: u32, height: u32, src_x: u32, src_y: u32) -> Widget {
        Widget {
            dst: Rect {
                x,
                y,
                width,
                height,
            },
            src_x,
            src_y,
        }
    }

    /// The six records `DAT_10054920` holds, as the atlas splits them: the two
    /// the hit map covers, then the two lit ones behind them.
    fn atlas() -> Atlas {
        Atlas {
            widgets: vec![
                widget(64, 0, 273, 450, 1, 1),
                widget(463, 0, 273, 450, 275, 1),
            ],
            extras: vec![
                widget(64, 0, 273, 450, 1, 452),
                widget(463, 0, 273, 450, 275, 452),
            ],
            offset: 0,
            segments: vec![(0, 0, 2)],
            matched: 2,
        }
    }

    /// The retail key names a clip, so the arm `wcsstr` misses is the one
    /// taken: `sentakuBG_03.wmv` plays rather than being held as a picture.
    #[test]
    fn the_shipped_key_is_a_clip() {
        let start = Ini::parse("[DressBG]=\"System/DressSelect/sentakuBG_03.wmv\"");
        assert_eq!(
            background(&start),
            Some(Background::Movie(
                "System/DressSelect/sentakuBG_03.wmv".to_string()
            ))
        );
    }

    /// Pointing the key at the `.png` the install ships beside the clip takes
    /// the still arm instead. `wcsstr` is a substring test, not a suffix one,
    /// so a value that only mentions `.png` takes it too.
    #[test]
    fn a_png_anywhere_in_the_value_takes_the_still_arm() {
        let start = Ini::parse("[DressBG]=\"System/DressSelect/sentakuBG_03.png\"");
        assert_eq!(
            background(&start),
            Some(Background::Still(
                "System/DressSelect/sentakuBG_03.png".to_string()
            ))
        );
        let odd = Ini::parse("[DressBG]=\"System/.png/clip.wmv\"");
        assert!(matches!(background(&odd), Some(Background::Still(_))));
    }

    /// A key the file does not carry is a missing asset, not a screen that
    /// refuses to draw.
    #[test]
    fn no_key_is_no_background() {
        assert_eq!(background(&Ini::parse("[TitleBGM]=\"x\"")), None);
        assert_eq!(background(&Ini::parse("[DressBG]=\"\"")), None);
    }

    /// Widget 0 is the `A` dress: `FUN_1000ded0` calls host `+0x48` with 1 for
    /// it, and `RouteProcSD.dll` takes non-zero to mean `A`.
    #[test]
    fn widget_zero_is_the_a_dress() {
        assert_eq!(host_value(0), 1);
        assert_eq!(host_value(1), 0);
    }

    /// The popup's two widgets are yes then no, and while choosing the same two
    /// indices commit instead — one hit map's widget 1 is the other's cancel.
    #[test]
    fn the_same_widget_means_different_things_on_each_map() {
        assert_eq!(action(Phase::Choosing, 1), Act::Commit(1));
        assert_eq!(action(Phase::Confirming { chosen: 1 }, 1), Act::Cancel);
        assert_eq!(action(Phase::Confirming { chosen: 1 }, 0), Act::Accept);
    }

    /// `FUN_1000dea0` bounds the widget before the switch on both maps.
    #[test]
    fn a_third_widget_does_nothing() {
        assert_eq!(action(Phase::Choosing, 2), Act::None);
        assert_eq!(action(Phase::Confirming { chosen: 0 }, 2), Act::None);
    }

    /// Records 2 and 3 share records 0 and 1's destination and differ only in
    /// `src_y` — the lit row of the sheet.
    #[test]
    fn the_lit_records_only_move_in_the_sheet() {
        let atlas = atlas();
        for widget in 0..WIDGETS {
            let lit = lit(&atlas, widget).expect("the lit record");
            assert_eq!(lit.dst, atlas.widgets[widget].dst);
            assert_eq!(lit.src_x, atlas.widgets[widget].src_x);
            assert_eq!(lit.src_y, 452);
        }
    }

    /// A committed slide, run until it has asked for the popup.
    fn slid(atlas: &Atlas, chosen: usize) -> Slide {
        let mut slide = Slide::default();
        slide.commit(atlas, chosen);
        while slide.tick() != Some(Slid::Popup) {}
        slide
    }

    /// Both dresses land on the same two `x` positions whichever was chosen,
    /// and the chosen one is drawn last so it is on top.
    #[test]
    fn committing_slides_both_dresses_together() {
        let atlas = atlas();
        for chosen in 0..WIDGETS {
            let cuts = slid(&atlas, chosen).cuts(&atlas);
            assert_eq!(cuts.len(), 2);
            let xs: Vec<f32> = cuts.iter().map(|cut| cut.dst.0).collect();
            assert!(xs.contains(&SLID_LEFT_X) && xs.contains(&SLID_RIGHT_X));
            assert_eq!(cuts.last().expect("the top sprite").src.1, 452.0);
        }
    }

    /// The slide takes [`SLIDE_FRAMES`] ticks and the popup comes up on the one
    /// after, which is `FUN_1000e440` arm 0 handing over to arm 1.
    #[test]
    fn the_popup_is_one_tick_behind_the_end_of_the_slide() {
        let atlas = atlas();
        let mut slide = Slide::default();
        slide.commit(&atlas, 0);
        for tick in 1..SLIDE_FRAMES as usize {
            assert_eq!(slide.tick(), None, "tick {tick}");
            // Moving, and not yet arrived: a thirtieth of the way per tick.
            let x = slide.cuts(&atlas)[1].dst.0;
            assert!(x > 64.0 && x < SLID_LEFT_X, "tick {tick} put it at {x}");
        }
        // The thirtieth tick lands them, and is drawn there.
        assert_eq!(slide.tick(), None);
        assert_eq!(slide.cuts(&atlas)[1].dst.0, SLID_LEFT_X);
        assert_eq!(slide.tick(), Some(Slid::Popup));
        assert_eq!(slide.tick(), None);
    }

    /// Nothing but the two dresses is drawn while they are moving, and the
    /// popup only once `+0x140` is up: `FUN_1000c740`'s two gates.
    #[test]
    fn the_caption_and_the_highlight_go_the_moment_the_dresses_move() {
        let atlas = atlas();
        let mut slide = Slide::default();
        assert_eq!(slide.drawn(Phase::Choosing), Drawn::Dresses);
        slide.commit(&atlas, 0);
        assert_eq!(slide.drawn(Phase::Choosing), Drawn::Sliding);
        assert!(slide.moving());
        while slide.tick() != Some(Slid::Popup) {}
        assert_eq!(slide.drawn(Phase::Confirming { chosen: 0 }), Drawn::Popup);
        assert!(!slide.moving(), "nothing moves while the popup is up");
    }

    /// The popup's no runs the same travel backwards and ends where it began:
    /// `FUN_1000ded0` sets arm 3 and touches nothing else.
    #[test]
    fn cancelling_slides_them_back_to_their_records() {
        let atlas = atlas();
        let mut slide = slid(&atlas, 1);
        slide.cancel();
        assert_eq!(slide.drawn(Phase::Choosing), Drawn::Sliding);
        let mut ticks = 0;
        while slide.tick() != Some(Slid::Dresses) {
            ticks += 1;
            assert!(ticks < 100, "the slide back never finished");
        }
        assert_eq!(slide.drawn(Phase::Choosing), Drawn::Dresses);
        let cuts = slide.cuts(&atlas);
        assert_eq!(cuts[0].dst.0, 64.0);
        assert_eq!(cuts[1].dst.0, 463.0);
    }

    /// What a slide can draw is the four records, whichever dress is clicked,
    /// and each of them at the source and size a realized cut is keyed by —
    /// which is what makes warming them before the click work.
    #[test]
    fn the_cuts_to_warm_are_every_sprite_a_slide_can_draw() {
        let atlas = atlas();
        let warm = slide_cuts(&atlas);
        assert_eq!(warm.len(), 4);
        for chosen in 0..WIDGETS {
            let mut slide = Slide::default();
            slide.commit(&atlas, chosen);
            for cut in slide.cuts(&atlas) {
                assert!(
                    warm.iter()
                        .any(|w| w.src == cut.src && w.dst.2 == cut.dst.2 && w.dst.3 == cut.dst.3),
                    "the slide draws a sprite nothing warmed: {cut:?}"
                );
            }
        }
    }

    /// A table without the lit records has nothing to warm for them either,
    /// and warms what it does have rather than refusing.
    #[test]
    fn the_cuts_to_warm_follow_the_table() {
        let mut atlas = atlas();
        atlas.extras.clear();
        assert_eq!(slide_cuts(&atlas).len(), WIDGETS);
    }

    /// A module whose table stops at the two the map covers still draws: the
    /// dresses come up resting rather than the screen refusing.
    #[test]
    fn a_table_without_the_lit_records_still_draws() {
        let mut atlas = atlas();
        atlas.extras.clear();
        assert_eq!(lit(&atlas, 0), None);
        assert_eq!(resting(&atlas).len(), WIDGETS);
        let cuts = slid(&atlas, 0).cuts(&atlas);
        assert_eq!(cuts.len(), WIDGETS, "both dresses, the chosen one unlit");
        assert_eq!(cuts[1].src.1, 1.0);
    }
}
