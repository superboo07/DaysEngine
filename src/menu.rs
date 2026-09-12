//! The menu: which screen is up, what the pointer is on, and where a click goes.
//!
//! # The mode number is the state machine
//!
//! FILMEngine's menus are not a tree the engine walks. The executable asks the
//! menu DLL for a *mode* — a small integer — and `SystemInit` turns that number
//! into one of eight screen modules, so the whole graph is one `switch`:
//!
//! ```text
//! mode  2  Title          mode  6  RouteMap        mode -1  Exit/Popup
//! mode  3  SaveLoad       mode  7  Option/Pop_Som  mode  1  (not a menu: play)
//! mode  4  Option         mode  8  Replay/Pop_Replay
//! mode  5  Replay
//! ```
//!
//! A screen reports where to go next and the engine opens that module. Mode 1
//! is the one value `SystemInit` has no case for, which is exactly why it means
//! "stop showing menus and start playing"; -1 is the confirm popup, and it
//! remembers the mode it was opened from so it can draw the right question and
//! return there on cancel. [`Mode`] and [`Menu::advance`] are that switch.
//!
//! # What a click does
//!
//! Every screen is [`Screen`]: base art, a chip sprite sheet, and a per-pixel
//! hit map. Pointing at a widget swaps in its chip sprite; clicking runs the
//! screen's action table. Only the title's table is recovered so far, out of
//! the DLL's own dispatch — see [`Menu::confirm`].
//!
//! # Whose answer is it
//!
//! The DLL decides nothing about the player: it asks the host, through a
//! vtable, and picks art from the replies. Two of the three replies turn out
//! not to be save data at all — see [`SaveState::from_flags`], which is also
//! where the reasoning is written down, because getting this wrong produced a
//! title screen the real game never shows.

use crate::ini::Ini;
use crate::screen::{Error, Resolution, Screen, WidgetState};
use crate::vfs::Vfs;
use days_save::FlagStore;

/// A menu screen id, as the game itself numbers them.
///
/// These are the exact integers the DLL's `SystemInit` switches on, kept rather
/// than renamed to an enum of our own: they cross the engine/menu boundary in
/// the original and every table below is written in terms of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode(pub i32);

impl Mode {
    /// Not a menu at all: leave the menus and play the script.
    pub const PLAY: Mode = Mode(1);
    pub const TITLE: Mode = Mode(2);
    pub const SAVELOAD: Mode = Mode(3);
    pub const OPTION: Mode = Mode(4);
    pub const REPLAY: Mode = Mode(5);
    pub const ROUTEMAP: Mode = Mode(6);
    /// The peripheral-config popup, reachable only from [`Mode::OPTION`].
    pub const SOM_CONFIG: Mode = Mode(7);
    /// The "play this replay?" popup, reachable only from [`Mode::REPLAY`].
    pub const REPLAY_POPUP: Mode = Mode(8);
    /// The confirm popup. Asks to quit when opened from the title and to return
    /// to the title when opened from anywhere else.
    pub const CONFIRM: Mode = Mode(-1);

    /// The screen's path stem, or `None` for [`Mode::PLAY`], which has no art.
    ///
    /// Several modules share one class across several stems and choose at load
    /// time, so `variant` is whichever one this screen was opened with. See
    /// [`Mode::default_variant`] for where those come from.
    pub fn stem(self, variant: &str) -> Option<String> {
        Some(match self {
            Mode::TITLE => format!("System/Title/{variant}"),
            Mode::SAVELOAD => "System/SaveLoad/SaveLoad".to_string(),
            Mode::OPTION => format!("System/Option/Option_{variant}"),
            Mode::REPLAY => format!("System/Replay/Replay_{variant}"),
            // Episodes 1 and 2 are one page; 3 upwards add a `-N` suffix the
            // screen pages through, so the variant carries both parts.
            Mode::ROUTEMAP => format!("System/RouteMap/{variant}/RouteMap{variant}"),
            Mode::SOM_CONFIG => "System/Option/Pop_Som".to_string(),
            Mode::REPLAY_POPUP => format!("System/Replay/Pop_Replay_{variant}"),
            Mode::CONFIRM => "System/Exit/Popup".to_string(),
            _ => return None,
        })
    }

    /// The variant a module opens with the first time it is entered.
    ///
    /// These are not defaults this engine picked. Each module is a singleton
    /// whose art is chosen from one zero-initialised member, so the value a
    /// freshly constructed module selects is the value below:
    ///
    /// ```text
    /// Option        +0x184  0 Def      1 Sound     2 SomCon
    /// Replay        +0x2b0  0 HScene   else PlayData
    /// RouteMap      +0x3d8  0 episode 1, single page
    /// ReplayPopup   +0x0c8  0 the two-widget popup, else the four-widget one
    /// SaveLoad      +0x094  0 Load.png, else Save.png -- and SystemInit
    ///                       explicitly pokes 0 when it opens the module
    /// ```
    ///
    /// The title is the exception: it picks from save state rather than from a
    /// member, in [`SaveState::title_variant`].
    pub fn default_variant(self) -> &'static str {
        match self {
            Mode::OPTION => "Def",
            Mode::REPLAY => "HScene",
            Mode::ROUTEMAP => "01",
            Mode::REPLAY_POPUP => "2",
            _ => "",
        }
    }
}

/// The seven system sounds `FILMENGINE.INI` names.
///
/// The menu modules ask the host to play one of these by a small integer index.
/// Which index is which is **not recovered**: the executable stores the seven
/// paths as separate members rather than an array, so there is no stride to
/// read off, and no dispatch on the index was found in the menu-host code. The
/// bindings [`Menu`] uses are therefore this engine's choice, not the game's —
/// the names and the INI keys are the game's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemSe {
    Cancel,
    Select,
    Click,
    Up,
    Down,
    View,
    Open,
}

impl SystemSe {
    /// The `FILMENGINE.INI` key naming this sound.
    pub fn key(self) -> &'static str {
        match self {
            SystemSe::Cancel => "SeCancel",
            SystemSe::Select => "SeSelect",
            SystemSe::Click => "SeClick",
            SystemSe::Up => "SeUp",
            SystemSe::Down => "SeDown",
            SystemSe::View => "SeView",
            SystemSe::Open => "SeOpen",
        }
    }

    pub const ALL: [SystemSe; 7] = [
        SystemSe::Cancel,
        SystemSe::Select,
        SystemSe::Click,
        SystemSe::Up,
        SystemSe::Down,
        SystemSe::View,
        SystemSe::Open,
    ];

    /// Resolves the sound's asset path out of `FILMENGINE.INI`.
    pub fn path(self, film: &Ini) -> Option<&str> {
        film.get(self.key())
    }
}

/// What the player has unlocked.
///
/// The title screen asks the host three questions — all-clear, trial build,
/// and whether a given route is cleared — and picks its art and its enabled
/// widgets from the answers. See [`SaveState::from_flags`] for where each
/// answer really comes from, because only one of the three is a save flag.
#[derive(Debug, Clone, Copy, Default)]
pub struct SaveState {
    /// The host's all-clear answer. **Always false in the retail executable**
    /// — see [`SaveState::from_flags`]. It is not the `AllClear` save flag.
    pub all_clear: bool,
    /// The host's trial answer. Always false in the retail executable.
    pub trial: bool,
    /// Route 0 cleared: switches the title to `Title_Clear`. Needs both the
    /// `EndClear` flag and `STARTSCRIPT.INI [EndBGView]`.
    pub cleared_first: bool,
    /// Route 1 cleared: unlocks `REPLAY`. The `EndClear` flag alone.
    pub cleared_replay: bool,
}

impl SaveState {
    /// Answers the title screen's three questions the way the host does.
    ///
    /// The DLL only asks; the executable answers. The host interface it is
    /// handed is a **secondary base subobject**, installed at `[object+0x2c]`,
    /// so each slot's member sits `0x2c` above the offset the DLL sees:
    ///
    /// | Question | Slot | Reads | Object member |
    /// |---|---|---|---|
    /// | all-clear | `+0xe8` | `FUN_0042c2f0` | `+0x7a4` |
    /// | trial | `+0x34` | `FUN_0042c120` | `+0x7a8` |
    /// | route *n* cleared | `+0xec` | `FUN_0042baf0` | `+0x21c` and a flag |
    ///
    /// **Neither all-clear nor trial is a save flag.** Both are plain members,
    /// and only two functions in the whole executable write either. The live
    /// constructor `FUN_004217e0` — the only one anything calls — zeroes both.
    /// The other, `FUN_00421b30`, sets trial to 1 and all-clear to the result
    /// of asking for `L"CrossDays"`, and **has no callers at all**: it is the
    /// sibling-title build, dead code here.
    ///
    /// So in the retail executable all-clear is always false, which makes
    /// `Title_AC` — and the audio-commentary entry painted into it — a screen
    /// the game can never reach. Trial is always false too. This engine
    /// answers both the way the shipped code does rather than inventing a
    /// condition for them, and [`SaveState::from_flags`] therefore ignores the
    /// `AllClear` flag entirely. That flag is real and is read, but by
    /// `FUN_0041fee0` in the executable, to choose the *backdrop* — a
    /// different question with a confusingly similar name. See
    /// [`crate::ending`].
    ///
    /// Route 0 and route 1 both come from the one `EndClear` flag
    /// (`FUN_0042baf0`), so clearing the game once changes the title art and
    /// unlocks `REPLAY`. Route 0 carries one further condition, object
    /// `+0x21c`, which `FUN_0041f600` sets from `STARTSCRIPT.INI
    /// [EndBGView]` — the same key that gates loading the ending list at all.
    pub fn from_flags(flags: &FlagStore, start: &Ini) -> Self {
        let cleared = flags.flag("EndClear");
        SaveState {
            all_clear: false,
            trial: false,
            cleared_first: cleared && end_bg_view(start),
            cleared_replay: cleared,
        }
    }

    /// The title art variant, following the DLL's own three-way test.
    ///
    /// `Title_AC` is kept because the test is the DLL's and this is a
    /// reimplementation of it, not of its reachable subset — but nothing in
    /// the retail executable can set [`SaveState::all_clear`], so the live
    /// answers are only `Title_Clear` and `Title`.
    pub fn title_variant(self) -> &'static str {
        if self.all_clear {
            "Title_AC"
        } else if self.trial {
            "Title"
        } else if self.cleared_first {
            "Title_Clear"
        } else {
            "Title"
        }
    }

    /// Whether `REPLAY` is selectable.
    pub fn replay_unlocked(self) -> bool {
        !self.trial && self.cleared_replay
    }
}

/// `STARTSCRIPT.INI [EndBGView]`, the ending-backdrop switch.
///
/// `FUN_0041f600` reads this key into the startup config and does two things
/// with it: it hands it to the host as the extra condition on route 0, and it
/// skips loading the ending list entirely when the key is clear. So one key
/// turns off both the ending backdrops and the cleared title art. The shipped
/// value is `"1"`; a key that is absent altogether reads as off, which is what
/// the executable's zero-initialised member does.
pub fn end_bg_view(start: &Ini) -> bool {
    start.get_bool("EndBGView").unwrap_or(false)
}

/// What the engine should do after handing the menu an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Nothing changed that the engine needs to act on.
    Stay,
    /// Play a system sound.
    Sound(SystemSe),
    /// Open another screen. The menu has already switched to it.
    Opened(Mode),
    /// The screen exists in the game but this engine cannot draw it yet, so
    /// the menu stayed where it was.
    Unavailable(Mode),
    /// Leave the menus and play the script.
    Play,
    /// Quit the game.
    Quit,
}

/// The menu, as one screen plus the pointer state over it.
pub struct Menu {
    mode: Mode,
    variant: String,
    screen: Screen,
    /// Which widget the pointer or the keyboard is on, if any. The title starts
    /// with nothing selected: the DLL initialises its selection to -1.
    selection: Option<usize>,
    /// Where the confirm popup returns to on cancel. `SystemInit` records this
    /// for every mode except the popups themselves.
    return_to: Mode,
    save: SaveState,
    resolution: Resolution,
    states: Vec<WidgetState>,
    dirty: bool,
}

impl Menu {
    /// Opens the mode's screen.
    pub fn open(
        vfs: &Vfs,
        dll: &[u8],
        mode: Mode,
        save: SaveState,
        resolution: Resolution,
    ) -> Result<Menu, Error> {
        let variant = if mode == Mode::TITLE {
            save.title_variant().to_string()
        } else {
            mode.default_variant().to_string()
        };
        Menu::open_variant(vfs, dll, mode, &variant, save, resolution, Mode::TITLE)
    }

    fn open_variant(
        vfs: &Vfs,
        dll: &[u8],
        mode: Mode,
        variant: &str,
        save: SaveState,
        resolution: Resolution,
        return_to: Mode,
    ) -> Result<Menu, Error> {
        let stem = mode
            .stem(variant)
            .ok_or_else(|| Error::MissingAsset(format!("mode {}", mode.0)))?;
        let base = base_art(mode, return_to);
        let screen = Screen::load_with_base(vfs, dll, &stem, base, resolution)?;
        let states = vec![WidgetState::Resting; screen.widget_count()];
        let mut menu = Menu {
            mode,
            variant: variant.to_string(),
            screen,
            selection: None,
            return_to,
            save,
            resolution,
            states,
            dirty: true,
        };
        menu.refresh();
        Ok(menu)
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    pub fn selection(&self) -> Option<usize> {
        self.selection
    }

    /// True when the composited frame is out of date.
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Composites the current frame over an optional backdrop and marks it clean.
    pub fn compose(&mut self, backdrop: Option<&days_ui::Image>) -> days_ui::Image {
        self.dirty = false;
        self.screen.compose_over(backdrop, &self.states)
    }

    /// Whether a widget can be chosen.
    ///
    /// Only the title's rule is recovered, from the DLL's own enablement switch:
    /// everything is selectable except `REPLAY`, which needs a cleared route,
    /// and the all-clear-only sixth widget. Other screens have no recovered
    /// table yet and treat every widget as live.
    pub fn enabled(&self, widget: usize) -> bool {
        if self.mode != Mode::TITLE {
            return true;
        }
        match widget {
            2 => self.save.replay_unlocked(),
            5 => self.save.all_clear,
            _ => widget < self.states.len(),
        }
    }

    /// Recomputes every widget's sprite from the current selection.
    ///
    /// A disabled `REPLAY` draws the one alternate record that follows the
    /// title's table in the DLL — `extras[0]`, the greyed-out caption. Only
    /// that first extra belongs to this screen: the run after it is the next
    /// screen's table, which the atlas cannot see the end of.
    fn refresh(&mut self) {
        for (i, state) in self.states.iter_mut().enumerate() {
            *state = if self.mode == Mode::TITLE && i == 2 && !self.save.replay_unlocked() {
                WidgetState::Extra(0)
            } else if self.selection == Some(i) {
                WidgetState::Active
            } else {
                WidgetState::Resting
            };
        }
        self.dirty = true;
    }

    /// Moves the pointer. Coordinates are in the screen's own pixel space.
    pub fn point_at(&mut self, x: u32, y: u32) -> Action {
        let hit = self.screen.hit(x, y).filter(|i| self.enabled(*i));
        self.select(hit)
    }

    /// The pointer left the screen, or the window lost focus.
    pub fn point_away(&mut self) -> Action {
        self.select(None)
    }

    fn select(&mut self, next: Option<usize>) -> Action {
        if next == self.selection {
            return Action::Stay;
        }
        self.selection = next;
        self.refresh();
        match next {
            Some(_) => Action::Sound(SystemSe::Select),
            None => Action::Stay,
        }
    }

    /// Keyboard navigation: `delta` is -1 for up and +1 for down.
    ///
    /// The DLL wraps over the five title entries and steps past a disabled
    /// `REPLAY` in whichever direction it was already moving, so the locked
    /// entry is never landed on. With nothing selected yet, a first keypress
    /// lands on the first entry rather than moving from an imaginary one.
    pub fn navigate(&mut self, delta: i32) -> Action {
        let count = self.wrapping_count();
        let Some(index) = step(count, self.selection, delta, |i| self.enabled(i)) else {
            return Action::Stay;
        };
        self.selection = Some(index);
        self.refresh();
        Action::Sound(if delta < 0 {
            SystemSe::Up
        } else {
            SystemSe::Down
        })
    }

    /// How many widgets keyboard navigation wraps over.
    ///
    /// The title's sixth widget is a mouse-only shortcut: the DLL's navigation
    /// wraps 0..=4 even on the all-clear screen, where it instead jumps to
    /// index 5 from outside that range.
    fn wrapping_count(&self) -> usize {
        if self.mode == Mode::TITLE {
            self.states.len().min(5)
        } else {
            self.states.len()
        }
    }

    /// Activates the selected widget.
    ///
    /// The title's widget-to-mode table is the DLL's own dispatch: `START` and
    /// the all-clear shortcut both begin play, `LOAD`, `OPTION` and `REPLAY`
    /// each open their screen, and `EXIT` opens the confirm popup. No other
    /// screen's table is recovered, so elsewhere a click confirms nothing and
    /// only the cancel path is live.
    pub fn confirm(&mut self, vfs: &Vfs, dll: &[u8]) -> Result<Action, Error> {
        let Some(widget) = self.selection else {
            return Ok(Action::Stay);
        };
        if !self.enabled(widget) {
            return Ok(Action::Stay);
        }
        match self.mode {
            Mode::TITLE => {
                let next = match widget {
                    0 | 5 => Mode::PLAY,
                    1 => Mode::SAVELOAD,
                    2 => Mode::REPLAY,
                    3 => Mode::OPTION,
                    4 => Mode::CONFIRM,
                    _ => return Ok(Action::Stay),
                };
                self.advance(vfs, dll, next)
            }
            // The popup's two widgets are YES then NO, from its own dispatch:
            // widget 0 records the affirmative answer, widget 1 records the
            // negative one and sends the player back to the mode the popup
            // remembers.
            Mode::CONFIRM => match widget {
                0 => self.confirm_popup(vfs, dll),
                1 => {
                    let back = self.return_to;
                    self.advance(vfs, dll, back)
                }
                _ => Ok(Action::Stay),
            },
            _ => Ok(Action::Stay),
        }
    }

    /// Backs out of the current screen.
    ///
    /// Every screen but the title returns to the confirm popup, which asks
    /// whether to go back to the title; the title's own cancel asks whether to
    /// quit. The two popups differ only in their background art, chosen from
    /// the mode the popup remembers.
    pub fn cancel(&mut self, vfs: &Vfs, dll: &[u8]) -> Result<Action, Error> {
        match self.mode {
            Mode::CONFIRM => {
                let back = self.return_to;
                self.advance(vfs, dll, back)
            }
            Mode::SOM_CONFIG => self.advance(vfs, dll, Mode::OPTION),
            Mode::REPLAY_POPUP => self.advance(vfs, dll, Mode::REPLAY),
            _ => self.advance(vfs, dll, Mode::CONFIRM),
        }
    }

    /// Switches to `next`, loading its screen.
    ///
    /// [`Mode::PLAY`] has no screen — it is the one mode `SystemInit` has no
    /// case for — and quitting is the confirm popup's answer when it was opened
    /// from the title.
    pub fn advance(&mut self, vfs: &Vfs, dll: &[u8], next: Mode) -> Result<Action, Error> {
        if next == Mode::PLAY {
            return Ok(Action::Play);
        }
        self.reopen(vfs, dll, next)
    }

    /// Answers the confirm popup with "yes".
    pub fn confirm_popup(&mut self, vfs: &Vfs, dll: &[u8]) -> Result<Action, Error> {
        if self.mode != Mode::CONFIRM {
            return Ok(Action::Stay);
        }
        if self.return_to == Mode::TITLE {
            Ok(Action::Quit)
        } else {
            self.reopen(vfs, dll, Mode::TITLE)
        }
    }

    fn reopen(&mut self, vfs: &Vfs, dll: &[u8], next: Mode) -> Result<Action, Error> {
        // The popup remembers where it came from; every other screen resets it,
        // matching SystemInit, which records the outgoing mode for all but the
        // popups themselves.
        let return_to = if next == Mode::CONFIRM {
            self.mode
        } else {
            Mode::TITLE
        };
        let variant = if next == Mode::TITLE {
            self.save.title_variant().to_string()
        } else {
            next.default_variant().to_string()
        };
        match Menu::open_variant(
            vfs,
            dll,
            next,
            &variant,
            self.save,
            self.resolution,
            return_to,
        ) {
            Ok(opened) => {
                *self = opened;
                Ok(Action::Opened(next))
            }
            // Two screens lay their rows out with a runtime loop instead of a
            // table, so the atlas search correctly refuses them and they cannot
            // be drawn yet. Staying put is the right answer: an unbuilt screen
            // should leave the player on a working menu, not end the session.
            Err(err) => {
                log::warn!("cannot open menu mode {}: {err}", next.0);
                Ok(Action::Unavailable(next))
            }
        }
    }

    /// The variant this screen was opened with, e.g. `Title_AC`.
    pub fn variant(&self) -> &str {
        &self.variant
    }
}

/// One step of keyboard navigation over `count` entries.
///
/// The DLL wraps the title's five entries and, when it lands on a locked one,
/// keeps moving the way it was already going rather than stopping — so a
/// disabled `REPLAY` is stepped over in both directions and never selected.
/// With nothing selected yet a first keypress enters the list from its end
/// rather than moving off an imaginary position. Returns `None` when there is
/// nothing selectable to move to.
fn step(
    count: usize,
    current: Option<usize>,
    delta: i32,
    enabled: impl Fn(usize) -> bool,
) -> Option<usize> {
    if count == 0 || delta == 0 {
        return None;
    }
    let n = count as i32;
    let mut index = match current {
        Some(at) => (at as i32 + delta).rem_euclid(n),
        None if delta > 0 => 0,
        None => n - 1,
    };
    for _ in 0..count {
        if enabled(index as usize) {
            return Some(index as usize);
        }
        index = (index + delta.signum()).rem_euclid(n);
    }
    None
}

/// Background art for screens that do not name theirs after the stem.
///
/// The confirm popup shares one chip sheet and hit map between two questions
/// and picks the background from where it was opened: quitting from the title,
/// returning to the title from anywhere else.
fn base_art(mode: Mode, return_to: Mode) -> Option<&'static str> {
    match mode {
        Mode::CONFIRM if return_to == Mode::TITLE => Some("System/Exit/Popup_Exit.png"),
        Mode::CONFIRM => Some("System/Exit/Popup_Title.png"),
        Mode::SAVELOAD => Some("System/SaveLoad/Load.png"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_is_the_mode_with_no_screen() {
        assert_eq!(Mode::PLAY.stem(""), None);
        assert!(Mode::TITLE.stem("Title").is_some());
    }

    /// Every mode's default has to name art that actually ships, so a stem is
    /// never a plausible-looking path nothing resolves. The packs are not in
    /// this repository, so this pins the spellings; `days menu` checks them
    /// against a real install.
    #[test]
    fn every_mode_opens_a_stem_the_game_ships() {
        let expected = [
            (Mode::TITLE, "Title", "System/Title/Title"),
            (Mode::SAVELOAD, "", "System/SaveLoad/SaveLoad"),
            (Mode::OPTION, "Def", "System/Option/Option_Def"),
            (Mode::REPLAY, "HScene", "System/Replay/Replay_HScene"),
            (Mode::ROUTEMAP, "01", "System/RouteMap/01/RouteMap01"),
            (Mode::SOM_CONFIG, "", "System/Option/Pop_Som"),
            (Mode::REPLAY_POPUP, "2", "System/Replay/Pop_Replay_2"),
            (Mode::CONFIRM, "", "System/Exit/Popup"),
        ];
        for (mode, variant, stem) in expected {
            assert_eq!(mode.stem(variant).as_deref(), Some(stem));
            // The variant used above is the one the module actually opens with,
            // except the title's, which comes from save state instead.
            if mode != Mode::TITLE {
                assert_eq!(mode.default_variant(), variant, "mode {}", mode.0);
            }
        }
    }

    /// The DLL's three-way test, exercised on its own terms — including the
    /// branch the retail executable can never take.
    #[test]
    fn the_title_variant_follows_the_dlls_three_way_test() {
        let fresh = SaveState::default();
        assert_eq!(fresh.title_variant(), "Title");
        assert!(!fresh.replay_unlocked());

        let cleared = SaveState {
            cleared_first: true,
            ..fresh
        };
        assert_eq!(cleared.title_variant(), "Title_Clear");

        let all = SaveState {
            all_clear: true,
            cleared_first: true,
            ..fresh
        };
        assert_eq!(all.title_variant(), "Title_AC");

        let trial = SaveState {
            trial: true,
            cleared_first: true,
            cleared_replay: true,
            ..fresh
        };
        assert_eq!(trial.title_variant(), "Title");
        assert!(!trial.replay_unlocked());
    }

    fn started(end_bg_view: &str) -> Ini {
        Ini::parse(&format!("[EndBGView]=\"{end_bg_view}\""))
    }

    fn cleared_save() -> FlagStore {
        FlagStore::from_entries([("EndClear".to_string(), days_save::Value::Bool(true))])
    }

    /// A finished save gets `Title_Clear`, and **not** `Title_AC`: the host
    /// member behind the all-clear question is zeroed by the one constructor
    /// anything calls and never written again. Confirmed against the real
    /// game, which shows `Title_Clear` on a save with every ending seen.
    #[test]
    fn a_finished_save_gets_the_cleared_title_and_never_the_all_clear_one() {
        let save = SaveState::from_flags(&cleared_save(), &started("1"));
        assert_eq!(save.title_variant(), "Title_Clear");
        assert!(save.replay_unlocked());
        assert!(!save.all_clear, "no executable path sets this");
        assert!(!save.trial, "nor this");
    }

    /// The `AllClear` save flag is real, but it chooses the backdrop, not the
    /// title art. Setting it must not move the title screen.
    #[test]
    fn the_all_clear_save_flag_does_not_reach_the_title_art() {
        let flags = FlagStore::from_entries([
            ("EndClear".to_string(), days_save::Value::Bool(true)),
            ("AllClear".to_string(), days_save::Value::Bool(true)),
        ]);
        let save = SaveState::from_flags(&flags, &started("1"));
        assert!(!save.all_clear);
        assert_eq!(save.title_variant(), "Title_Clear");
    }

    /// `[EndBGView]` is the extra condition on route 0, so clearing it leaves
    /// the plain title however far the player has got — while `REPLAY`, which
    /// asks about route 1, stays unlocked.
    #[test]
    fn end_bg_view_gates_the_cleared_title_but_not_replay() {
        let save = SaveState::from_flags(&cleared_save(), &started("0"));
        assert_eq!(save.title_variant(), "Title");
        assert!(save.replay_unlocked());

        // An absent key reads as clear, matching the zeroed member.
        let save = SaveState::from_flags(&cleared_save(), &Ini::parse(""));
        assert_eq!(save.title_variant(), "Title");
    }

    #[test]
    fn a_fresh_save_gets_the_plain_title() {
        let save = SaveState::from_flags(&FlagStore::default(), &started("1"));
        assert_eq!(save.title_variant(), "Title");
        assert!(!save.replay_unlocked());
    }

    #[test]
    fn every_system_sound_has_an_ini_key() {
        let film = Ini::parse(
            r#"[SeCancel]="SysSe/NewSys/SeCancel_01"
               [SeSelect]="SysSe/NewSys/SeSelect"
               [SeClick]="SysSe/NewSys/SeClick_01"
               [SeUp]="SysSe/NewSys/SeUp"
               [SeDown]="SysSe/NewSys/SeDown"
               [SeView]="SysSe/NewSys/SeView"
               [SeOpen]="SysSe/NewSys/SeOpen""#,
        );
        for se in SystemSe::ALL {
            assert!(se.path(&film).is_some(), "{} is unresolved", se.key());
        }
    }

    #[test]
    fn navigation_wraps_and_steps_over_a_locked_entry() {
        // The title with REPLAY locked: five entries, index 2 unselectable.
        let live = |i: usize| i != 2;
        assert_eq!(step(5, None, 1, live), Some(0));
        assert_eq!(step(5, None, -1, live), Some(4));
        assert_eq!(step(5, Some(1), 1, live), Some(3), "down skips the lock");
        assert_eq!(step(5, Some(3), -1, live), Some(1), "up skips it too");
        assert_eq!(step(5, Some(4), 1, live), Some(0), "wraps past the end");
        assert_eq!(step(5, Some(0), -1, live), Some(4), "and past the start");
    }

    #[test]
    fn navigation_lands_on_a_locked_entry_once_it_is_unlocked() {
        let live = |_: usize| true;
        assert_eq!(step(5, Some(1), 1, live), Some(2));
        assert_eq!(step(5, Some(3), -1, live), Some(2));
    }

    #[test]
    fn navigation_gives_up_rather_than_spinning_when_everything_is_locked() {
        assert_eq!(step(5, Some(0), 1, |_| false), None);
        assert_eq!(step(0, None, 1, |_| true), None);
    }

    #[test]
    fn the_popup_asks_a_different_question_depending_on_where_it_opened() {
        assert_eq!(
            base_art(Mode::CONFIRM, Mode::TITLE),
            Some("System/Exit/Popup_Exit.png")
        );
        assert_eq!(
            base_art(Mode::CONFIRM, Mode::OPTION),
            Some("System/Exit/Popup_Title.png")
        );
    }
}
