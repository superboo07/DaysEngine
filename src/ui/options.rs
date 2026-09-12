//! The Option screen: three tabs, and what every widget on them does.
//!
//! # One module, three screens
//!
//! `MENU::ConfigMenu` is a single class that draws one of three sets of art and
//! switches on a member at `+0x184` to decide which:
//!
//! ```text
//! 0  Option_Def      14 widgets   display and text settings
//! 1  Option_Sound    44 widgets   three volumes, voices, mute
//! 2  Option_SomCon   18 widgets   SOMCON peripheral setup
//! ```
//!
//! The first four widgets mean the same thing on all three — widgets 0, 1 and 2
//! are the tab headers and widget 3 is the close button — and everything after
//! that is per-tab. [`Tab`] is that member and [`action`] is the module's own
//! dispatch, which lives in `FUN_10007ef0` (Def), `FUN_10008260` (Sound) and
//! `FUN_100089a0` (SomCon), reached through the switch in `FUN_10007e80`.
//!
//! # Where the settings go
//!
//! Every change is written straight through to `Config.DAT` — see
//! [`crate::install::config`] — and the close button flushes it. The DLL does exactly
//! that: each dispatch arm calls the config object's setter with the key name
//! before it touches anything else, and widget 3 calls the flush.
//!
//! # The current-value marks
//!
//! Each setting draws a mark over whichever of its two buttons is the value in
//! force. That is not derived from the art or from the table: it is a switch
//! per tab, `FUN_10009fd0`, `FUN_1000a190` and `FUN_1000a250`, transcribed in
//! [`shows_current_value`]. One of them has a shipped bug, noted there.
//!
//! # Keyboard navigation
//!
//! The arrow keys do not walk the widget list. Each tab has a hand-written
//! transition table in the DLL — `FUN_10008da0`, `FUN_100092d0` and
//! `FUN_100098a0` — with its own wrap rules, and the Sound tab's never visits
//! the ten level cells at all, which is why those cells have no sprite of their
//! own. [`navigate`] is those three tables.

use crate::install::config::{Channel, Config, Flag};

/// Which set of art the Option module is showing, as the member it switches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Def,
    Sound,
    SomCon,
}

impl Tab {
    /// The tab a freshly constructed module opens with: `+0x184` is zeroed.
    pub const DEFAULT: Tab = Tab::Def;

    /// The path variant, as `FUN_100073a0` spells it.
    pub fn variant(self) -> &'static str {
        match self {
            Tab::Def => "Def",
            Tab::Sound => "Sound",
            Tab::SomCon => "SomCon",
        }
    }

    /// The tab a header widget selects, for widgets 0, 1 and 2.
    pub fn from_widget(widget: usize) -> Option<Tab> {
        match widget {
            0 => Some(Tab::Def),
            1 => Some(Tab::Sound),
            2 => Some(Tab::SomCon),
            _ => None,
        }
    }

    fn index(self) -> usize {
        match self {
            Tab::Def => 0,
            Tab::Sound => 1,
            Tab::SomCon => 2,
        }
    }

    pub const ALL: [Tab; 3] = [Tab::Def, Tab::Sound, Tab::SomCon];
}

/// The base art for a tab.
///
/// `Option_SomCon` is the one that swaps: `FUN_100073a0` draws
/// `Option_SomCon_Set.png` instead once a port has been opened, and the chip
/// sheet stays the same either way.
pub fn base_art(tab: Tab, som: Som) -> Option<&'static str> {
    match tab {
        Tab::SomCon if som.enabled => Some("System/Option/Option_SomCon_Set.png"),
        _ => None,
    }
}

/// What this engine can answer about the display, for the two rows on the Def
/// tab that ask.
///
/// The DLL asks the host rather than the settings file — `+0xb8` for the aspect
/// and `+0xbc` for the window mode — so these come from the running engine, not
/// from `Config.DAT`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Display {
    /// The host's `+0xb8` answer: the layout is widescreen rather than 4:3.
    pub wide: bool,
    /// The host's `+0xbc` answer, which is **full screen, not windowed**.
    ///
    /// `FUN_0040e830` returns a member and says nothing about its sense; the
    /// screen does. `FUN_10009fd0` marks widget 6 when the answer is zero and
    /// widget 7 when it is not, and rendering the real art shows widget 6 is
    /// `WINDOW` and widget 7 is `FULL`. Naming this the other way round put the
    /// mark on `FULL` for a windowed engine, which is exactly the kind of
    /// plausible-looking inversion a screenshot catches and a test does not.
    pub full_screen: bool,
}

/// What the SOMCON tab knows.
///
/// SOMCON is the peripheral toy the game can drive. It is **not** a gamepad —
/// the tab's own art says `Port number` and `SOMCON test`, and the DLL imports
/// no input API at all. What it imports is `CreateFileA`, `GetCommState`,
/// `SetCommState`, `SetCommTimeouts`, `SetCommMask`, `WaitCommEvent`,
/// `ReadFile` and `WriteFile`: the toy is a **serial device**, and a "port
/// number" is a COM port. See [`SOM_PORTS`] for the protocol.
///
/// `+0x328` is the `UseSOM` setting, `+0x31c` is whether a port was actually
/// opened, `+0x324` is which one, and `+0x320` is whether the test is running.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Som {
    /// `UseSOM`: the player has asked for the toy.
    pub enabled: bool,
    /// A port was found and opened.
    pub attached: bool,
    /// Which port index is in use, when one is. Zero-based, so the screen's
    /// `Port number` 1 is index 0.
    pub port: usize,
    /// The `SOMCON test` is running.
    pub testing: bool,
}

/// How many `Port number` buttons the tab shows: widgets 6 to 15.
///
/// Ten buttons, but the DLL's port-name table has only [`SOM_PORTS`] entries —
/// see there.
pub const SOM_PORT_BUTTONS: usize = 10;

/// How many serial ports the DLL can actually open.
///
/// `FUN_10021070` selects from a table of nine ASCII names, `COM1` to `COM9`,
/// and opens the chosen one with `CreateFileA` at 9600 baud, 8 data bits, no
/// parity, one stop bit, 500 ms timeouts and `EV_RXCHAR`. It then talks a
/// two-command ASCII protocol: `FUN_100214d0` writes `s%02x` — `s` and a level
/// in hex, the test button sending `0x96` — and `FUN_100215d0` writes `b` to
/// stop. Each is followed by a read of up to 256 bytes of reply.
///
/// **This engine drives nothing.** The tab is a working screen and the `UseSOM`
/// setting is stored, but no port is opened and no command is sent: the
/// protocol above is proprietary to one discontinued device, and if this engine
/// ever moves a toy it should do it through Intiface rather than reimplement
/// it. So [`Som::attached`] and [`Som::testing`] are whatever the engine says
/// they are, and the screen honestly shows "no port" until something sets them.
///
/// The mismatch with [`SOM_PORT_BUTTONS`] is the DLL's own: the tenth button
/// selects index 9 of a nine-entry table. The string that follows it in
/// `.rdata` is the `s%02x` format itself, so the shipped build would ask
/// `CreateFileA` to open a file called `s%02x` and fail. This engine refuses
/// the tenth button instead of reproducing an out-of-bounds read.
pub const SOM_PORTS: usize = 9;

/// Which way a volume arrow moves a level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// `FUN_10007140` with a non-zero flag: one quieter, floored at 0.
    Quieter,
    /// One louder, capped at 10.
    Louder,
}

/// The display change a Def-tab row asks for.
///
/// The DLL does **not** change the mode itself. Each of these four widgets
/// checks that the mode really would change and then sets a flag: `+0xac` for
/// full screen and `+0xb0` for wide.
///
/// The executable reads them back through the module's **exports**, which is
/// why a sweep of the DLL's own code finds writes and no reads. `_GetFullFlag@0`
/// returns `+0xac` and `_GetWideFlag@0` returns `+0xb0`; `_SetFullFlag@4` and
/// `_SetWideFlag@4` write them. Each has exactly one reader in the executable
/// and it is a one-shot applier polled from the main loop:
///
/// ```text
/// FUN_004279e0:  if (_GetFullFlag@0()) { tear down, toggle the window with
///                FUN_0040e860(!FUN_0040e830()), rebuild, _SetFullFlag@4(0) }
/// FUN_00427a90:  if (_GetWideFlag@0()) { FUN_0040ed10(), rebuild,
///                _SetWideFlag@4(0) }
/// ```
///
/// So the flag is a **request**, not a setting: the engine acts on it once and
/// clears it. Both appliers toggle rather than assign, which is safe because
/// the widgets only fire when the mode would really change — see
/// [`def_action`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayRequest {
    Wide,
    Normal,
    FullScreen,
    Windowed,
}

/// What activating a widget does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// The widget is not live, or does nothing on this tab.
    None,
    /// Show another tab. The module reloads its art.
    Tab(Tab),
    /// Flush the settings and leave the menus. Widget 3 on every tab.
    Close,
    /// Nudge a volume one step.
    StepVolume(Channel, Step),
    /// Set a volume outright, from one of the ten cells.
    SetVolume(Channel, i32),
    /// Set a boolean setting.
    SetFlag(Flag, bool),
    /// Ask the engine to change the display mode.
    Display(DisplayRequest),
    /// Look for the toy and take the first port that opens.
    SomDetect,
    /// Let the port go.
    SomRelease,
    /// Take the port at this index.
    SomPort(usize),
    /// Start or stop the `SOMCON test`.
    SomTest(bool),
}

/// Whether a widget can be chosen, from `FUN_10007ca0`.
///
/// The three tab headers and the close button are common: everything is live
/// except the SOMCON header, which the trial build hides — the DLL asks the
/// host's `+0x34` trial question, and that member is zeroed by the only
/// constructor anything calls, so in the retail build the header is always
/// live. See [`crate::ui::menu::SaveState::from_flags`] for the same question on
/// the title screen.
pub fn enabled(tab: Tab, widget: usize, trial: bool, som: Som) -> bool {
    match widget {
        0 | 1 | 3 => true,
        2 => !trial,
        _ => match tab {
            Tab::Def => (4..=0xd).contains(&widget),
            Tab::Sound => (4..=0x2b).contains(&widget),
            // `FUN_10007dd0`: the two buttons that find and drop the port are
            // always live; everything else on the tab needs a port in hand.
            Tab::SomCon => match widget {
                4 | 5 => true,
                6..=0x11 => som.enabled && som.attached,
                _ => false,
            },
        },
    }
}

/// Whether this widget is the value currently in force, and so draws its mark.
///
/// Transcribed from `FUN_10009fd0` (Def), `FUN_1000a190` (Sound) and
/// `FUN_1000a250` (SomCon).
pub fn shows_current_value(
    tab: Tab,
    widget: usize,
    config: &Config,
    display: Display,
    som: Som,
) -> bool {
    match tab {
        Tab::Def => match widget {
            4 => display.wide,
            5 => !display.wide,
            6 => !display.full_screen,
            7 => display.full_screen,
            8 => config.flag(Flag::Skip),
            9 => !config.flag(Flag::Skip),
            0xa => config.flag(Flag::SuperSkip),
            0xb => !config.flag(Flag::SuperSkip),
            0xc => config.flag(Flag::TextView),
            0xd => !config.flag(Flag::TextView),
            _ => false,
        },
        // `FUN_1000a190` tests `widget == 10` in both of its first two arms, so
        // the mark for male voice is always on widget 10 and widget 11 never
        // gets one. That is the shipped behaviour, bug and all; writing the
        // arm the author meant would make this screen differ from the game.
        Tab::Sound => match widget {
            0xa => true,
            0xc => config.flag(Flag::Mute),
            0xd => !config.flag(Flag::Mute),
            _ => false,
        },
        Tab::SomCon => match widget {
            4 => som.enabled,
            5 => !som.enabled,
            0x10 => som.testing,
            0x11 => !som.testing,
            // Past those four, `FUN_1000a250` marks whichever port button is
            // the one in hand.
            _ => som.attached && widget == som.port + 6,
        },
    }
}

/// The ten cells of a volume row, as `(channel, first widget)`.
///
/// From the Sound tab's dispatch: widgets 14 to 23 set the music volume to
/// `widget - 13`, 24 to 33 the effects volume to `widget - 23`, and 34 to 43
/// the voice volume to `widget - 33` — so each row runs 1 to 10 and only the
/// arrows can reach 0.
const VOLUME_ROWS: [(Channel, usize); 3] =
    [(Channel::Bgm, 14), (Channel::Se, 24), (Channel::Voice, 34)];

/// How many cells a volume row has.
pub const VOLUME_CELLS: usize = 10;

/// What a widget does, from the module's own dispatch.
///
/// The caller is expected to have checked [`enabled`] first, exactly as every
/// arm of the DLL's dispatch does.
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
        Tab::SomCon => som_action(widget),
    }
}

/// `FUN_10007ef0`.
fn def_action(widget: usize, display: Display) -> Act {
    match widget {
        // The two display rows act only when the mode would really change:
        // "wide" is live only while the layout is 4:3, and so on. A click on
        // the value already in force falls through and does nothing.
        4 if !display.wide => Act::Display(DisplayRequest::Wide),
        5 if display.wide => Act::Display(DisplayRequest::Normal),
        // `WINDOW` is live only while the engine is full screen, and `FULL`
        // only while it is not.
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

/// `FUN_10008260`.
fn sound_action(widget: usize) -> Act {
    match widget {
        // The left arrow of each row is the lower widget number and passes a
        // non-zero flag to `FUN_10007140`, which decrements.
        4 => Act::StepVolume(Channel::Bgm, Step::Quieter),
        5 => Act::StepVolume(Channel::Bgm, Step::Louder),
        6 => Act::StepVolume(Channel::Se, Step::Quieter),
        7 => Act::StepVolume(Channel::Se, Step::Louder),
        8 => Act::StepVolume(Channel::Voice, Step::Quieter),
        9 => Act::StepVolume(Channel::Voice, Step::Louder),
        0xa => Act::SetFlag(Flag::MenVoice, true),
        0xb => Act::SetFlag(Flag::MenVoice, false),
        0xc => Act::SetFlag(Flag::Mute, true),
        0xd => Act::SetFlag(Flag::Mute, false),
        _ => {
            for (channel, first) in VOLUME_ROWS {
                if (first..first + VOLUME_CELLS).contains(&widget) {
                    return Act::SetVolume(channel, (widget - first) as i32 + 1);
                }
            }
            Act::None
        }
    }
}

/// `FUN_100089a0`.
fn som_action(widget: usize) -> Act {
    match widget {
        4 => Act::SomDetect,
        5 => Act::SomRelease,
        // The screen has ten buttons and the DLL nine port names; see
        // `SOM_PORTS` for what the tenth does in the shipped build.
        6..=0xf if widget - 6 < SOM_PORTS => Act::SomPort(widget - 6),
        0x10 => Act::SomTest(true),
        0x11 => Act::SomTest(false),
        _ => Act::None,
    }
}

/// The bar that shows how full a volume row is.
///
/// `FUN_100063c0` does not light the ten cells one by one: it draws a single
/// sprite whose destination is the first cell's record, stretched to the width
/// `FUN_100070e0` works out — the distance from the first cell's left edge to
/// the right edge of the cell at the current level. A level of zero draws
/// nothing, and because the source rect keeps the first cell's coordinates the
/// bar is one cell's art repeated across.
///
/// `cells` are the records for the row's ten cells, which is
/// [`Atlas::widgets`](days_ui::atlas::Atlas::widgets) sliced at the row's first
/// widget.
pub fn volume_bar(cells: &[days_ui::atlas::Widget], level: i32) -> Option<days_ui::atlas::Widget> {
    if level <= 0 {
        return None;
    }
    let first = cells.first()?;
    let last = cells.get((level as usize).min(cells.len()) - 1)?;
    let width = (last.dst.x + last.dst.width).checked_sub(first.dst.x)?;
    Some(days_ui::atlas::Widget {
        dst: days_ui::cmap::Rect { width, ..first.dst },
        ..*first
    })
}

/// The three volume rows of the Sound tab, as `(channel, first widget)`.
pub fn volume_row(row: usize) -> Option<(Channel, usize)> {
    VOLUME_ROWS.get(row).copied()
}

/// How many volume rows the Sound tab has.
pub const VOLUME_ROW_COUNT: usize = VOLUME_ROWS.len();

/// Applies an action's effect on the settings.
///
/// Only the arms that change `Config.DAT` are handled here; the rest — closing,
/// switching tabs, the display request and everything about the toy — are
/// the caller's, because they touch the engine rather than the file.
pub fn apply(config: &mut Config, act: Act) {
    match act {
        Act::StepVolume(channel, step) => {
            let level = config.volume(channel);
            config.set_volume(
                channel,
                match step {
                    Step::Quieter => level - 1,
                    Step::Louder => level + 1,
                },
            );
        }
        Act::SetVolume(channel, level) => config.set_volume(channel, level),
        Act::SetFlag(flag, value) => config.set_flag(flag, value),
        Act::SomDetect => config.set_flag(Flag::UseSom, true),
        Act::SomRelease => config.set_flag(Flag::UseSom, false),
        _ => {}
    }
}

/// Which arrow key moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// One step of keyboard navigation, from each tab's own transition table.
///
/// These are transcriptions of `FUN_10008da0`, `FUN_100092d0` and
/// `FUN_100098a0` rather than a rule fitted to them: the tables are hand
/// written in the DLL and each has its own oddities — the Sound tab never
/// reaches the ten level cells, the SOMCON tab's horizontal moves ignore the
/// trial question its siblings ask, and several transitions depend on state
/// rather than position.
pub fn navigate(tab: Tab, current: usize, dir: Dir, trial: bool, som: Som) -> usize {
    let c = current as i32;
    let next = match tab {
        Tab::Def => def_navigate(c, dir, trial),
        Tab::Sound => sound_navigate(c, dir, trial),
        Tab::SomCon => som_navigate(c, dir, tab, som),
    };
    next.max(0) as usize
}

/// The header row wraps over three tabs, or two when the trial build hides the
/// SOMCON one. Def and Sound share this; the SOMCON tab does not ask.
fn header_step(c: i32, forward: bool, trial: bool) -> i32 {
    let last = if trial { 1 } else { 2 };
    if forward {
        if c == last {
            0
        } else {
            c + 1
        }
    } else if c == 0 {
        last
    } else {
        c - 1
    }
}

/// `FUN_10008da0`.
fn def_navigate(c: i32, dir: Dir, trial: bool) -> i32 {
    match dir {
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
            0..=2 => header_step(c, true, trial),
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
            0..=2 => header_step(c, false, trial),
            _ => 3,
        },
        Dir::Down => match c {
            0 => 4,
            1 => 5,
            2 => 7,
            4..=9 => c + 4,
            0xa | 0xb => c + 2,
            3 => 0,
            _ => 3,
        },
        Dir::Up => match c {
            4..=6 => c - 4,
            7 => c - 5,
            8..=0xd => c - 4,
            3 => 0xd,
            _ => 3,
        },
    }
}

/// `FUN_100092d0`. The ten level cells are not in any arm: the Sound tab is
/// navigated by its arrows, and the cells are mouse-only.
fn sound_navigate(c: i32, dir: Dir, trial: bool) -> i32 {
    match dir {
        Dir::Right => match c {
            0..=2 => header_step(c, true, trial),
            4 | 5 => {
                if c == 5 {
                    4
                } else {
                    c + 1
                }
            }
            6 | 7 => {
                if c == 7 {
                    6
                } else {
                    c + 1
                }
            }
            8 | 9 => {
                if c == 9 {
                    8
                } else {
                    c + 1
                }
            }
            0xa..=0xd => {
                if c == 0xd {
                    0xa
                } else {
                    c + 1
                }
            }
            _ => 3,
        },
        Dir::Left => match c {
            0..=2 => header_step(c, false, trial),
            4 | 5 => {
                if c == 4 {
                    5
                } else {
                    c - 1
                }
            }
            6 | 7 => {
                if c == 6 {
                    7
                } else {
                    c - 1
                }
            }
            8 | 9 => {
                if c == 8 {
                    9
                } else {
                    c - 1
                }
            }
            0xa..=0xd => {
                if c == 0xa {
                    0xd
                } else {
                    c - 1
                }
            }
            _ => 3,
        },
        Dir::Down => match c {
            0 | 1 => 4,
            2 => 5,
            4..=8 => c + 2,
            9 => 0xd,
            3 => 0,
            _ => 3,
        },
        Dir::Up => match c {
            4 => 0,
            5 => 2,
            6..=9 => c - 2,
            0xa | 0xb => 8,
            0xc | 0xd => 9,
            3 => 0xd,
            _ => 3,
        },
    }
}

/// `FUN_100098a0`.
///
/// Two transitions land on the tab's own index — `+0x184`, which on this screen
/// is always 2 — so they are written as such rather than as a constant.
fn som_navigate(c: i32, dir: Dir, tab: Tab, som: Som) -> i32 {
    let home = tab.index() as i32;
    match dir {
        // Neither horizontal arm consults the trial question, unlike the other
        // two tabs': the header wraps over all three headers here.
        Dir::Right => match c {
            0..=2 => {
                if c == 2 {
                    0
                } else {
                    c + 1
                }
            }
            4 | 5 => 4 + i32::from(c == 4),
            0x10 | 0x11 => {
                if som.enabled {
                    0x10 + i32::from(c == 0x10)
                } else {
                    c
                }
            }
            _ => 3,
        },
        Dir::Left => match c {
            0..=2 => {
                if c == 0 {
                    2
                } else {
                    c - 1
                }
            }
            4 | 5 => 4 + i32::from(c == 4),
            0x10 | 0x11 => {
                if som.enabled {
                    0x10 + i32::from(c == 0x10)
                } else {
                    c
                }
            }
            _ => 3,
        },
        Dir::Down => match c {
            0 | 1 => {
                if som.enabled {
                    0x10
                } else {
                    4
                }
            }
            2 => {
                if som.enabled {
                    0x11
                } else {
                    5
                }
            }
            0x10 | 0x11 => {
                if som.enabled {
                    4 + i32::from(c != 0x10)
                } else {
                    c
                }
            }
            3 => home,
            _ => 3,
        },
        Dir::Up => match c {
            4 | 5 => {
                if som.enabled {
                    0x10 + i32::from(som.testing)
                } else {
                    home
                }
            }
            0x10 | 0x11 => {
                if som.enabled {
                    home
                } else {
                    c
                }
            }
            3 => 4,
            _ => 3,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        Config::default()
    }

    fn cells(first_x: u32) -> Vec<days_ui::atlas::Widget> {
        (0..VOLUME_CELLS as u32)
            .map(|i| days_ui::atlas::Widget {
                dst: days_ui::cmap::Rect {
                    x: first_x + i * 52,
                    y: 130,
                    width: 52,
                    height: 31,
                },
                src_x: 1,
                src_y: 111,
            })
            .collect()
    }

    /// The bar is one sprite stretched over however many cells are filled, not
    /// ten sprites, and silence draws nothing at all.
    #[test]
    fn a_volume_bar_spans_the_cells_up_to_the_level() {
        let cells = cells(175);
        assert_eq!(volume_bar(&cells, 0), None, "silence draws no bar");

        let one = volume_bar(&cells, 1).expect("a bar");
        assert_eq!((one.dst.x, one.dst.width), (175, 52));
        // The source stays the first cell's, so the bar repeats one cell's art.
        assert_eq!((one.src_x, one.src_y), (1, 111));

        let full = volume_bar(&cells, 10).expect("a bar");
        assert_eq!((full.dst.x, full.dst.width), (175, 520));

        // A level past the last cell cannot read off the end.
        let over = volume_bar(&cells, 99).expect("a bar");
        assert_eq!(over.dst.width, 520);
        assert_eq!(volume_bar(&[], 5), None);
    }

    #[test]
    fn every_volume_row_is_reachable_by_index() {
        assert_eq!(VOLUME_ROW_COUNT, 3);
        assert_eq!(volume_row(0), Some((Channel::Bgm, 14)));
        assert_eq!(volume_row(2), Some((Channel::Voice, 34)));
        assert_eq!(volume_row(3), None);
    }

    #[test]
    fn a_freshly_opened_module_shows_the_def_tab() {
        assert_eq!(Tab::DEFAULT, Tab::Def);
        assert_eq!(Tab::DEFAULT.variant(), "Def");
    }

    /// The headers and the close button mean the same thing on every tab.
    #[test]
    fn the_first_four_widgets_are_common_to_all_three_tabs() {
        for tab in Tab::ALL {
            assert_eq!(action(tab, 0, Display::default()), Act::Tab(Tab::Def));
            assert_eq!(action(tab, 1, Display::default()), Act::Tab(Tab::Sound));
            assert_eq!(action(tab, 2, Display::default()), Act::Tab(Tab::SomCon));
            assert_eq!(action(tab, 3, Display::default()), Act::Close);
        }
    }

    #[test]
    fn the_def_tab_toggles_the_three_text_settings() {
        let d = Display::default();
        assert_eq!(action(Tab::Def, 8, d), Act::SetFlag(Flag::Skip, true));
        assert_eq!(action(Tab::Def, 9, d), Act::SetFlag(Flag::Skip, false));
        assert_eq!(
            action(Tab::Def, 0xa, d),
            Act::SetFlag(Flag::SuperSkip, true)
        );
        assert_eq!(
            action(Tab::Def, 0xb, d),
            Act::SetFlag(Flag::SuperSkip, false)
        );
        assert_eq!(action(Tab::Def, 0xc, d), Act::SetFlag(Flag::TextView, true));
        assert_eq!(
            action(Tab::Def, 0xd, d),
            Act::SetFlag(Flag::TextView, false)
        );
    }

    /// A display row acts only when the mode would really change — the DLL
    /// guards each of the four with the matching host answer.
    #[test]
    fn a_display_row_only_acts_when_it_would_change_something() {
        let four_three = Display {
            wide: false,
            full_screen: true,
        };
        assert_eq!(
            action(Tab::Def, 4, four_three),
            Act::Display(DisplayRequest::Wide)
        );
        assert_eq!(action(Tab::Def, 5, four_three), Act::None, "already 4:3");
        assert_eq!(
            action(Tab::Def, 6, four_three),
            Act::Display(DisplayRequest::Windowed)
        );
        assert_eq!(action(Tab::Def, 7, four_three), Act::None, "already full");

        let wide_window = Display {
            wide: true,
            full_screen: false,
        };
        assert_eq!(action(Tab::Def, 4, wide_window), Act::None);
        assert_eq!(
            action(Tab::Def, 5, wide_window),
            Act::Display(DisplayRequest::Normal)
        );
        assert_eq!(action(Tab::Def, 6, wide_window), Act::None);
        assert_eq!(
            action(Tab::Def, 7, wide_window),
            Act::Display(DisplayRequest::FullScreen)
        );
    }

    /// Each row's left arrow is the lower widget number and makes it quieter.
    #[test]
    fn the_sound_arrows_step_the_right_channel_the_right_way() {
        let d = Display::default();
        let expected = [
            (4, Channel::Bgm, Step::Quieter),
            (5, Channel::Bgm, Step::Louder),
            (6, Channel::Se, Step::Quieter),
            (7, Channel::Se, Step::Louder),
            (8, Channel::Voice, Step::Quieter),
            (9, Channel::Voice, Step::Louder),
        ];
        for (widget, channel, step) in expected {
            assert_eq!(
                action(Tab::Sound, widget, d),
                Act::StepVolume(channel, step),
                "widget {widget}"
            );
        }
    }

    /// Thirty cells, three rows of ten, each row 1 to 10.
    #[test]
    fn every_volume_cell_maps_to_a_level_from_one_to_ten() {
        let d = Display::default();
        for (channel, first) in VOLUME_ROWS {
            for cell in 0..VOLUME_CELLS {
                assert_eq!(
                    action(Tab::Sound, first + cell, d),
                    Act::SetVolume(channel, cell as i32 + 1),
                    "{channel:?} cell {cell}"
                );
            }
        }
        // The last cell of the last row is the last widget on the screen.
        assert_eq!(
            action(Tab::Sound, 0x2b, d),
            Act::SetVolume(Channel::Voice, 10)
        );
        assert_eq!(action(Tab::Sound, 0x2c, d), Act::None);
    }

    /// Only the arrows reach silence: the cells start at 1.
    #[test]
    fn stepping_reaches_zero_but_the_cells_do_not() {
        let mut config = cfg();
        config.set_volume(Channel::Bgm, 1);
        apply(&mut config, Act::StepVolume(Channel::Bgm, Step::Quieter));
        assert_eq!(config.volume(Channel::Bgm), 0);
        apply(&mut config, Act::StepVolume(Channel::Bgm, Step::Quieter));
        assert_eq!(config.volume(Channel::Bgm), 0, "floors rather than wraps");

        config.set_volume(Channel::Bgm, 10);
        apply(&mut config, Act::StepVolume(Channel::Bgm, Step::Louder));
        assert_eq!(config.volume(Channel::Bgm), 10, "and caps");
    }

    #[test]
    fn applying_an_action_writes_the_setting_through() {
        let mut config = cfg();
        apply(&mut config, action(Tab::Sound, 0xc, Display::default()));
        assert!(config.flag(Flag::Mute));
        apply(&mut config, action(Tab::Sound, 0x12, Display::default()));
        assert_eq!(config.volume(Channel::Bgm), 5);
        apply(&mut config, action(Tab::Sound, 0x17, Display::default()));
        assert_eq!(config.volume(Channel::Bgm), 10);
    }

    #[test]
    fn the_somcon_tab_finds_a_port_and_runs_the_test() {
        let d = Display::default();
        assert_eq!(action(Tab::SomCon, 4, d), Act::SomDetect);
        assert_eq!(action(Tab::SomCon, 5, d), Act::SomRelease);
        for port in 0..SOM_PORTS {
            assert_eq!(action(Tab::SomCon, 6 + port, d), Act::SomPort(port));
        }
        // The tenth button has no port behind it: the DLL's table stops at
        // COM9, and reproducing its overrun would open a file named `s%02x`.
        assert_eq!(SOM_PORT_BUTTONS, SOM_PORTS + 1);
        assert_eq!(action(Tab::SomCon, 6 + SOM_PORTS, d), Act::None);
        assert_eq!(action(Tab::SomCon, 0x10, d), Act::SomTest(true));
        assert_eq!(action(Tab::SomCon, 0x11, d), Act::SomTest(false));
    }

    /// The SOMCON tab's port list is dead until a port is in hand.
    #[test]
    fn the_somcon_port_list_needs_a_port_before_it_is_live() {
        let none = Som::default();
        let held = Som {
            enabled: true,
            attached: true,
            port: 2,
            testing: false,
        };
        assert!(
            enabled(Tab::SomCon, 4, false, none),
            "detect is always live"
        );
        assert!(enabled(Tab::SomCon, 5, false, none));
        assert!(!enabled(Tab::SomCon, 6, false, none));
        assert!(!enabled(Tab::SomCon, 0x10, false, none));
        assert!(enabled(Tab::SomCon, 6, false, held));
        assert!(enabled(Tab::SomCon, 0x11, false, held));

        // Asking for the toy without a port being found is not enough.
        let asked = Som {
            enabled: true,
            attached: false,
            ..none
        };
        assert!(!enabled(Tab::SomCon, 6, false, asked));
    }

    /// The SOMCON header is the only widget the trial build takes away.
    #[test]
    fn the_trial_build_hides_only_the_somcon_header() {
        for tab in Tab::ALL {
            assert!(enabled(tab, 2, false, Som::default()));
            assert!(!enabled(tab, 2, true, Som::default()));
            for widget in [0, 1, 3] {
                assert!(enabled(tab, widget, true, Som::default()));
            }
        }
    }

    #[test]
    fn each_tab_stops_at_its_own_last_widget() {
        let som = Som::default();
        assert!(enabled(Tab::Def, 0xd, false, som));
        assert!(!enabled(Tab::Def, 0xe, false, som));
        assert!(enabled(Tab::Sound, 0x2b, false, som));
        assert!(!enabled(Tab::Sound, 0x2c, false, som));
    }

    #[test]
    fn the_def_marks_follow_the_display_and_the_settings() {
        let mut config = cfg();
        // The mark on the second row sits on WINDOW when the engine is not
        // full screen, which is the pairing a screenshot of the real art
        // settled.
        let wide_window = Display {
            wide: true,
            full_screen: false,
        };
        let som = Som::default();
        assert!(shows_current_value(Tab::Def, 4, &config, wide_window, som));
        assert!(!shows_current_value(Tab::Def, 5, &config, wide_window, som));
        assert!(shows_current_value(Tab::Def, 6, &config, wide_window, som));
        assert!(!shows_current_value(Tab::Def, 7, &config, wide_window, som));

        // TextView defaults on, so the mark starts on widget 12.
        let d = Display::default();
        assert!(shows_current_value(Tab::Def, 0xc, &config, d, som));
        config.set_flag(Flag::TextView, false);
        assert!(shows_current_value(Tab::Def, 0xd, &config, d, som));
        assert!(!shows_current_value(Tab::Def, 0xc, &config, d, som));
    }

    /// The mark for male voice is stuck on widget 10 in the shipped build,
    /// because `FUN_1000a190` tests the same widget number in both arms. This
    /// pins that rather than quietly fixing it.
    #[test]
    fn the_sound_tabs_male_voice_mark_is_stuck_where_the_dll_leaves_it() {
        let mut config = cfg();
        let d = Display::default();
        let som = Som::default();
        assert!(shows_current_value(Tab::Sound, 0xa, &config, d, som));
        assert!(!shows_current_value(Tab::Sound, 0xb, &config, d, som));
        config.set_flag(Flag::MenVoice, false);
        assert!(
            shows_current_value(Tab::Sound, 0xa, &config, d, som),
            "still on widget 10: the DLL never tests widget 11"
        );
        assert!(!shows_current_value(Tab::Sound, 0xb, &config, d, som));

        // Mute's pair, in the same function, does work.
        assert!(shows_current_value(Tab::Sound, 0xd, &config, d, som));
        config.set_flag(Flag::Mute, true);
        assert!(shows_current_value(Tab::Sound, 0xc, &config, d, som));
    }

    #[test]
    fn the_somcon_tab_marks_the_port_in_hand() {
        let config = cfg();
        let d = Display::default();
        let held = Som {
            enabled: true,
            attached: true,
            port: 3,
            testing: false,
        };
        assert!(shows_current_value(Tab::SomCon, 4, &config, d, held));
        assert!(
            shows_current_value(Tab::SomCon, 9, &config, d, held),
            "6 + 3"
        );
        assert!(!shows_current_value(Tab::SomCon, 8, &config, d, held));
        assert!(shows_current_value(Tab::SomCon, 0x11, &config, d, held));
        assert!(!shows_current_value(Tab::SomCon, 0x10, &config, d, held));
    }

    /// The Def tab's five rows are pairs, and left/right walks each pair.
    #[test]
    fn def_navigation_walks_pairs_and_wraps_within_a_row() {
        let som = Som::default();
        let nav = |c, d| navigate(Tab::Def, c, d, false, som);
        assert_eq!(nav(4, Dir::Right), 5);
        assert_eq!(nav(7, Dir::Right), 4, "wraps inside the row of four");
        assert_eq!(nav(4, Dir::Left), 7);
        assert_eq!(nav(0xc, Dir::Right), 0xd);
        assert_eq!(nav(0xd, Dir::Right), 0xc, "the last row is a pair");
        // Down from a header lands on that column's first setting.
        assert_eq!(nav(0, Dir::Down), 4);
        assert_eq!(nav(1, Dir::Down), 5);
        assert_eq!(nav(2, Dir::Down), 7);
        assert_eq!(nav(4, Dir::Down), 8);
        assert_eq!(nav(8, Dir::Up), 4);
        assert_eq!(nav(7, Dir::Up), 2, "seven steps back five, not four");
        // The close button is the fallback from anywhere unexpected.
        assert_eq!(nav(3, Dir::Down), 0);
        assert_eq!(nav(3, Dir::Up), 0xd);
    }

    /// The header row is two wide in a trial build and three otherwise.
    #[test]
    fn header_navigation_skips_the_somcon_tab_in_a_trial_build() {
        let som = Som::default();
        assert_eq!(navigate(Tab::Def, 2, Dir::Right, false, som), 0);
        assert_eq!(navigate(Tab::Def, 1, Dir::Right, false, som), 2);
        assert_eq!(navigate(Tab::Def, 1, Dir::Right, true, som), 0);
        assert_eq!(navigate(Tab::Def, 0, Dir::Left, true, som), 1);
        assert_eq!(navigate(Tab::Def, 0, Dir::Left, false, som), 2);
    }

    /// The arrows never reach the thirty level cells.
    #[test]
    fn sound_navigation_never_lands_on_a_volume_cell() {
        let som = Som::default();
        for start in 0..=0xd {
            for dir in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
                let to = navigate(Tab::Sound, start, dir, false, som);
                assert!(to <= 0xd, "{start} {dir:?} reached cell {to}");
            }
        }
        assert_eq!(navigate(Tab::Sound, 4, Dir::Right, false, som), 5);
        assert_eq!(navigate(Tab::Sound, 5, Dir::Right, false, som), 4);
        assert_eq!(navigate(Tab::Sound, 0xd, Dir::Right, false, som), 0xa);
        assert_eq!(navigate(Tab::Sound, 9, Dir::Down, false, som), 0xd);
        assert_eq!(navigate(Tab::Sound, 4, Dir::Up, false, som), 0);
        assert_eq!(navigate(Tab::Sound, 5, Dir::Up, false, som), 2);
    }

    /// Without the toy the tab's lower half is unreachable by keyboard too.
    #[test]
    fn somcon_navigation_opens_up_once_the_toy_is_asked_for() {
        let none = Som::default();
        assert_eq!(navigate(Tab::SomCon, 0, Dir::Down, false, none), 4);
        assert_eq!(navigate(Tab::SomCon, 2, Dir::Down, false, none), 5);
        // The test pair holds still while it is dead.
        assert_eq!(navigate(Tab::SomCon, 0x10, Dir::Right, false, none), 0x10);

        let on = Som {
            enabled: true,
            attached: true,
            ..none
        };
        assert_eq!(navigate(Tab::SomCon, 0, Dir::Down, false, on), 0x10);
        assert_eq!(navigate(Tab::SomCon, 2, Dir::Down, false, on), 0x11);
        assert_eq!(navigate(Tab::SomCon, 0x10, Dir::Right, false, on), 0x11);
        assert_eq!(navigate(Tab::SomCon, 0x11, Dir::Down, false, on), 5);
        // Up from the pair returns to this tab's own header.
        assert_eq!(navigate(Tab::SomCon, 0x10, Dir::Up, false, on), 2);
    }

    /// Unlike the other two, this tab's horizontal arms never ask about trial.
    #[test]
    fn somcon_header_navigation_ignores_the_trial_question() {
        let som = Som::default();
        assert_eq!(navigate(Tab::SomCon, 1, Dir::Right, true, som), 2);
        assert_eq!(navigate(Tab::SomCon, 2, Dir::Right, true, som), 0);
    }

    /// Only the SOMCON tab swaps its background, and only once the toy is on.
    #[test]
    fn only_the_somcon_tab_changes_its_background() {
        let off = Som::default();
        let on = Som {
            enabled: true,
            ..off
        };
        assert_eq!(base_art(Tab::Def, on), None);
        assert_eq!(base_art(Tab::Sound, on), None);
        assert_eq!(base_art(Tab::SomCon, off), None);
        assert_eq!(
            base_art(Tab::SomCon, on),
            Some("System/Option/Option_SomCon_Set.png")
        );
    }
}
