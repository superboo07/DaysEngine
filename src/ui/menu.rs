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
//! Not every module has the same eight. `SysMenuSD.dll` adds a case 9,
//! [`Mode::DRESS_SELECT`], and spells several of the others' paths differently;
//! which screens a module has and where their art lives is read back out of
//! that module rather than written down here. See [`crate::ui::paths`].
//!
//! # What a click does
//!
//! Every screen is [`Screen`]: base art, a chip sprite sheet, and a per-pixel
//! hit map. Pointing at a widget swaps in its chip sprite; clicking runs the
//! screen's action table. Each table is the DLL's own dispatch, transcribed
//! where that screen's behaviour lives: the title's in [`Menu::confirm`], the
//! Option screen's in [`crate::ui::options`], the replay grid's and its popup's
//! in [`crate::ui::replay`] and the save/load screen's in
//! [`crate::ui::saveload`], and `Replay_PlayData` the same rows again
//! through [`crate::ui::playdata`].
//!
//! A few screens also draw things that are not widget states. Those whose
//! source is the same size as their destination go through `Menu::sprites` —
//! the Sound tab's volume bars, cut from the chip sheet, and the replay grid's
//! thumbnails. The save/load rows do not: their text is rasterised at twice the
//! size it is drawn at, the way the original rasterises it, so
//! [`Menu::load_rows`] builds that surface and [`Menu::compose`] blits it down.
//!
//! # Whose answer is it
//!
//! The DLL decides nothing about the player: it asks the host, through a
//! vtable, and picks art from the replies. Two of the three replies turn out
//! not to be save data at all — see [`SaveState::from_flags`], which is also
//! where the reasoning is written down, because getting this wrong produced a
//! title screen the real game never shows.

use crate::install::config::{Config, Sound};
use crate::install::ini::Ini;
use crate::install::vfs::Vfs;
use crate::ui::dress;
use crate::ui::option_pages;
use crate::ui::options;
use crate::ui::options::Dir;
use crate::ui::paths::Paths;
use crate::ui::playdata;
use crate::ui::replay::{self, Scenes};
use crate::ui::replay_pages;
use crate::ui::routemap;
use crate::ui::saveload::{self, Kind, Slots};
use crate::ui::screen::{Error, Resolution, Screen, WidgetState};
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

    /// Not a menu screen in every module: the dress-select screen.
    ///
    /// `_SystemInit@8` in `SysMenuSD.dll` has a case for 9 that hands the
    /// switch `DAT_1005b898`, whose static-init thunk `FUN_10048860` calls the
    /// constructor `FUN_1000c470`, which installs `MENU::DressSelect::vftable`.
    /// `SysMenuSDHQ.dll` has no case for 9 at all, so on that module
    /// [`Paths::stem`] finds no screen for it and the mode is unavailable.
    ///
    /// Its confirm popup is a second hit map inside this mode rather than a
    /// mode of its own, and what raises the mode at all is not recovered. See
    /// [`crate::ui::dress`] for both.
    pub const DRESS_SELECT: Mode = Mode(9);

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
    /// SaveLoad      +0x094  0 Load.png, else Save.png — and SystemInit
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
/// The menu modules ask the host to play one of these by a small integer index,
/// through host vtable slot `+0x50`, and the index is the key's position in
/// `FILMENGINE.INI`'s own order — the order the variants are declared in below.
///
/// `FUN_00429c80` is slot `+0x50`: a seven-arm switch reading the member at
/// `this + 0x5a4 + 0x1c * index`. `FUN_00422170` writes the seven INI values to
/// `base + 0x5a0 + 0x1c * n` in declaration order, and `base` is the host
/// subobject plus four — twice over, because the run of members lines up and
/// because the same function hands `base + 0x304` to the choice box that
/// `FUN_00431740` reaches as `engine + 0x334`, with the host subobject at
/// `engine + 0x2c`. So the two runs are the same seven strings in the same
/// order.
///
/// Three uses agree with the names: the control bar plays index 2 on a click,
/// and the choice box index 1 when a choice is taken and index 0 when it is
/// declined. Which index each *menu screen* plays is a separate question and
/// still a per-screen one; what [`Menu`] plays where is this engine's choice.
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

    /// The index host slot `+0x50` takes for this sound.
    pub fn index(self) -> usize {
        SystemSe::ALL
            .iter()
            .position(|se| *se == self)
            .expect("ALL lists every variant")
    }

    /// The sound host slot `+0x50` plays for an index, or `None` past the
    /// seventh — the switch has no default arm, so an index it does not know
    /// leaves the path pointer uninitialised and plays whatever was last there.
    pub fn from_index(index: usize) -> Option<SystemSe> {
        SystemSe::ALL.get(index).copied()
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
    /// [`crate::ui::ending`].
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

    /// The title art variant, following the DLL's own test.
    ///
    /// `Title_AC` is kept because the test is the DLL's and this is a
    /// reimplementation of it, not of its reachable subset — but nothing in
    /// the retail executable can set [`SaveState::all_clear`], so the live
    /// answers are only `Title_Clear` and `Title`.
    ///
    /// Which art each answer selects is the module's, not this test's:
    /// `SysMenuSD.dll`'s `FUN_1002f550` has only two arms, `Title.png` and
    /// `Clear/Title_Clear.png`, and no all-clear art ships in that install at
    /// all. [`Paths::title_art`] maps a non-plain variant onto whichever
    /// cleared spelling the module holds, so the unreachable all-clear arm
    /// stays unreachable there rather than naming a file that is not present.
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

/// The player's own font, for the text the menus draw themselves.
///
/// The English build ships `FONTDATA_ENG.DAT` beside the Japanese
/// `FONTDATA.DAT` and prefers it, which is the order the engine reads them in
/// everywhere else. A font that will not parse is a missing asset: it logs and
/// the text it would have drawn is left out.
fn load_font(vfs: &Vfs) -> Option<days_font::Font> {
    let bytes = vfs
        .read_path("System/System/FONTDATA_ENG.DAT")
        .or_else(|_| vfs.read_path("System/System/FONTDATA.DAT"));
    match bytes
        .map_err(|e| e.to_string())
        .and_then(|b| days_font::Font::parse(b).map_err(|e| e.to_string()))
    {
        Ok(font) => Some(font),
        Err(err) => {
            log::warn!("no menu font: {err}");
            None
        }
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Play a replay scene, as the sequence of scripts it runs through.
    ///
    /// The whole list, not just the first: `FUN_1001f0d0` is asked for the next
    /// one each time a script ends, and walks the scene's own list by a step
    /// index at `+0x2ac`, following the scene's branch table where it has one.
    PlayReplay(replay::Run),
    /// Play a save slot back following the answers it recorded.
    ///
    /// A row of the replay screen's play-data list. `FUN_1001dfe0` hands the
    /// host three things for it — `+0x48(slot)`, which is the same load the
    /// save/load screen asks for, then `+0x94(1)`, and then `+0x4c(8)` to
    /// leave. `+0x94` raises the flag `+0x98` reports, and `FUN_00431740`
    /// reads that flag at every choice box: a box nobody answers resolves to
    /// `FUN_00428a80`, the answer the slot recorded at that script, and the
    /// moment the player answers one themselves the flag comes back down.
    PlayRecorded(u32),
    /// A setting changed. The engine should re-read the volumes it mixes with.
    ///
    /// The DLL writes each setting through to the config object as it happens,
    /// which is why this is separate from [`Action::SettingsSaved`].
    SettingsChanged,
    /// The Option screen was closed: the settings were flushed to disk.
    ///
    /// `FUN_10007ef0` widget 3 flushes the config object — its `+0x2cc`
    /// vtable slot `+8` — and then asks the host to leave the menus with
    /// `+0x4c(0)`, the same code the save/load screen's Close uses. So where
    /// this lands is [`Entry`]'s question rather than this action's: the engine
    /// flushes and then leaves, which is playback when the bar opened the
    /// screen and the title when the title did.
    SettingsSaved,
    /// The Option screen asked for a display mode this engine has to apply.
    ///
    /// The DLL only records the request; who acts on it is not recovered. See
    /// [`crate::ui::options::DisplayRequest`].
    Display(options::DisplayRequest),
    /// The SOMCON tab asked the engine to do something with the peripheral.
    ///
    /// Whether a port is really there is the engine's to answer, never the
    /// DLL's: `FUN_10007850` walks the ports itself and `_GetSomFlag@0`
    /// reports what it found. The screen's job is to ask and to draw the
    /// answer, so this goes out and the engine writes the result back into
    /// [`Session::som`]. See [`crate::playback::som::Device`].
    Som(options::SomRequest),
    /// Load a save slot and play what it names.
    Load(u32),
    /// Jump to a story point the run has passed, from the route map.
    ///
    /// `FUN_1000e8b0` hands the host `+0x48(story)` and `+0x4c(8)` — the Load
    /// screen's own pair, with a number of 100 or more instead of a slot — and
    /// raises `_GetRouteLoad@0`. See [`crate::ui::routemap`].
    LoadStory(u32),
    /// Write the player's position to a save slot.
    Save(u32),
    /// Quit the game.
    Quit,
}

/// Everything the menus know about the player, carried across screens.
///
/// Each menu module in the original is a singleton that keeps its own state
/// between visits and asks the host for the rest. This is both halves: the
/// answers the host gives, and the per-screen state that survives a screen
/// being closed and reopened.
pub struct Session {
    /// The three answers the title screen asks for.
    pub save: SaveState,
    /// The global flag store, which is what the host really answers from.
    pub flags: FlagStore,
    /// The settings file, read and written by the Option screen.
    pub config: Config,
    /// Which sound model this install's settings are in, from
    /// [`crate::ui::paths::Paths::sound`].
    ///
    /// The settings file does not say: `[BgmVolume]="5"` and
    /// `[BgmVolume]="0.500000"` are both a volume at rest, and only the module
    /// that reads them settles which. See [`crate::install::config::Sound`].
    pub sound: Sound,
    /// The replay scene table, recovered from the player's own menu DLL.
    pub scenes: Scenes,
    /// What this engine can say about the display, for the Def tab's two rows.
    pub display: options::Display,
    /// What the SOMCON tab knows.
    pub som: options::Som,
    /// What the save/load screen shows for each slot.
    pub slots: Slots,
    /// The store of the playthrough that is running, when one is.
    ///
    /// The route map asks two questions about a story point and this answers
    /// the second: host `+0x18` against [`Session::flags`] is whether the
    /// player has ever seen it, and host `+0x10` against this is whether they
    /// passed it in the run they are in. There is no run from the title, and
    /// `FUN_1000c740` does not ask there either — see [`crate::ui::routemap`].
    pub run: Option<FlagStore>,
    /// Which dress the player picked, which is what host `+0x44` answers.
    ///
    /// `FUN_1000ded0` reports the choice the moment it is committed, through
    /// host `+0x48` — `FUN_0041dc50` in `SHINYDAYS.exe`, which stores the
    /// argument and raises a flag beside it. The reader is slot `+0x44`, and
    /// its callers are in `RouteProcSD.dll`: `FUN_1004da70` and `FUN_100515e0`
    /// append `A` to a scene block's name when this is non-zero and `B` when it
    /// is zero. `None` is a run in which the screen has not been answered,
    /// which is the zero the host object's constructor leaves — see
    /// [`crate::ui::dress`].
    pub dress: Option<u32>,
    /// `FILMENGINE.INI [TextInput]`, which is what host `+0xd8` answers.
    ///
    /// It gates the save row's comment column and the expanded comment behind
    /// it. `FUN_00422170` reads the key into the member host `+0xd8` returns —
    /// see [`crate::ui::saveload`], where the two-interface arithmetic that
    /// connects them is written down. The shipped INI sets it.
    pub text_input: bool,
    /// `FILMENGINE.INI [UseEnglish]`, which is what host `+0x5c` answers.
    ///
    /// The menus ask it constantly — it picks the timestamp format, the
    /// character caps on a save row and where each column sits — so it is
    /// carried here rather than threaded through every call.
    pub english: bool,
}

impl Session {
    /// A session with no save data, no settings and no scene table — enough to
    /// open the title screen, which is what a broken install leaves.
    pub fn empty(save: SaveState) -> Session {
        Session {
            save,
            flags: FlagStore::default(),
            config: Config::default(),
            sound: Sound::default(),
            scenes: Scenes::from_scenes(Vec::new()),
            display: options::Display::default(),
            som: options::Som::default(),
            slots: Slots::default(),
            run: None,
            dress: None,
            english: false,
            text_input: false,
        }
    }
}

/// Which of the original's two menu drivers is running.
///
/// This is not a flavour the engine invented to remember something: the
/// executable really does have two separate menu drivers on two different
/// objects, and which one is running is what decides where leaving a screen
/// goes.
///
/// [`Entry::Title`] is the title-rooted shell, `FUN_0041d410`, `FUN_0041d9c0`
/// and `FUN_0041dfa0`. Those three functions hold **every** call to
/// `_getNextMode@8` in the executable — 14 of them, and no others, from
/// Ghidra's reference index on the import thunk at `0x004a1ef6`. Leaving a
/// screen there walks the mode graph `getNextMode` encodes, whose sink is the
/// title.
///
/// [`Entry::Playback`] is the playback object's own menu layer, and it never
/// consults `getNextMode` at all. Host slot `+0xf8` — the control bar's "open
/// a menu" — sets that object's state member `+0x220` to 3 and stores the
/// module's `setSystemInit` code at `+0x260`; `FUN_00427300` dispatches state 3
/// to `FUN_00425550`, which runs whichever module `+0x260` names. Host slot
/// `+0x4c` is what writes `+0x260`, and the code **0** means leave: case 6 of
/// `FUN_00425550` falls through cases 7 and 8, and case 8 sets `+0x220 = 1` --
/// the state `FUN_00427300` dispatches to `FUN_004253f0`, the playback tick.
///
/// So a screen the bar opened returns to playback when it is closed, and it
/// reaches the title only if the player asks for the title.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Entry {
    /// The menus are the whole screen, entered from the title.
    #[default]
    Title,
    /// The menus are over live playback, entered from the control bar.
    Playback,
}

/// Where leaving a screen goes.
///
/// Split out from [`Menu`] so the rule can be checked without an install: the
/// destination depends only on which driver is running and, for the title
/// driver, on the mode the popup remembers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leaving {
    /// Leave the menu layer and resume playback underneath.
    Resume,
    /// Open this mode.
    To(Mode),
}

/// Where host `+0x4c(0)` — the Close button — goes.
///
/// See [`Entry`] for the evidence. Over playback `FUN_00425550` case 6 falls
/// through to case 8, which sets `+0x220 = 1`, the playback tick. From the
/// title the mode graph's sink is the title.
pub fn leaving(entry: Entry, return_to: Mode) -> Leaving {
    match entry {
        Entry::Playback => Leaving::Resume,
        Entry::Title => Leaving::To(return_to),
    }
}

/// Where backing out of `mode` goes — the Escape key, and the screens' own
/// back buttons that are not Close.
///
/// Three screens answer this themselves whichever driver is running, because
/// each was opened from another screen rather than from a driver:
/// SOMCON from the Option screen, the replay popup from the replay grid, and
/// the route map from the save/load screen — the last of those is
/// `_getNextMode@8` case 6, which answers mode 3 and never the title.
///
/// Everything else is the driver's question, and over playback the answer is
/// the same as Close's, because `+0x4c(0)` is the only way out the modules the
/// bar can open actually offer.
pub fn backing_out(entry: Entry, mode: Mode, return_to: Mode) -> Leaving {
    match mode {
        Mode::CONFIRM => Leaving::To(return_to),
        Mode::SOM_CONFIG => Leaving::To(Mode::OPTION),
        Mode::REPLAY_POPUP => Leaving::To(Mode::REPLAY),
        Mode::ROUTEMAP => Leaving::To(Mode::SAVELOAD),
        _ => match entry {
            Entry::Playback => Leaving::Resume,
            Entry::Title => Leaving::To(Mode::CONFIRM),
        },
    }
}

/// The menu, as one screen plus the pointer state over it.
pub struct Menu {
    mode: Mode,
    /// Where this install's menu module puts each screen's art. Read once from
    /// the module's own literals — see [`crate::ui::paths`].
    paths: Paths,
    variant: String,
    screen: Screen,
    /// Which widget the pointer or the keyboard is on, if any. The title starts
    /// with nothing selected: the DLL initialises its selection to -1.
    selection: Option<usize>,
    /// Where the confirm popup returns to on cancel. `SystemInit` records this
    /// for every mode except the popups themselves.
    return_to: Mode,
    /// Which driver is running, and so where leaving a screen goes. See
    /// [`Entry`].
    entry: Entry,
    resolution: Resolution,
    states: Vec<WidgetState>,
    dirty: bool,
    /// The player's state, kept across screens the way the originals' singleton
    /// modules keep theirs.
    session: Session,
    /// Which Option tab is showing: `MENU::ConfigMenu` `+0x184`.
    tab: options::Tab,
    /// Which Replay screen is showing: `MENU::SceneView` `+0x2b0`.
    view: replay::View,
    /// Which page of the replay grid: `+0x2a4`.
    page: usize,
    /// Which episode the route map is charting: `MENU::RouteMap` `+0x3d8`.
    episode: usize,
    /// Which page of it: `+0x3e0`.
    map_page: usize,
    /// The scene the replay popup is asking about: `+0x2a8`.
    asked: Option<usize>,
    /// Which job the save/load screen is doing: its `+0x94`, which
    /// `setSystemInit` pokes with 1 for code 4 and 0 for code 5.
    kind: Kind,
    /// The replay grid's thumbnail sheet for the current page, with the record
    /// table that cuts it up. Absent when the page's art will not load, which
    /// leaves the grid's empty frames showing.
    thumbnails: Option<(days_ui::Image, replay::Thumbnails)>,
    /// The Option screen's page, on a module that lays a tab's widgets out
    /// against a table rather than giving each tab its own hit map. Absent on
    /// the module that does, and on any other screen.
    option_page: Option<OptionPage>,
    replay_page: Option<ReplayPage>,
    /// Which of the dress-select screen's two hit maps is loaded:
    /// `MENU::DressSelect` `+0x140`. Meaningless on every other screen, and
    /// kept across a display-mode change because `FUN_1000ef80` keeps it.
    dress_phase: dress::Phase,
    /// That screen's art that is not the loaded map's own: the caption plate
    /// and, while the popup is up, the sheet the two dresses under it are cut
    /// from. Absent on every other screen.
    dress: Option<DressScreen>,
    /// The player's own font, for the text a screen draws itself rather than
    /// picking out of its art. Absent when the install has no readable
    /// `FONTDATA`, which leaves that text undrawn and the screen usable.
    font: Option<days_font::Font>,
    /// The save/load screen's rasterised rows for the page it is showing.
    rows: Option<saveload::Rows>,
    /// A save the screen has taken and not yet committed: the module's `+0x98`,
    /// with the comment its `+0x200` holds.
    ///
    /// While this is set the whole screen is inert — `FUN_10014910` answers
    /// false for every widget — and `Popup_Save.png` draws over it. The next
    /// tick commits, re-rasterises the rows and clears it, which is why saving
    /// leaves the player on the save screen rather than closing it.
    pending_save: Option<(u32, String)>,
    /// How much larger than its hit map this menu composites, so that the
    /// screens loaded by [`Menu::enter`] keep the size the caller asked for.
    /// See [`Menu::set_output_size`].
    out_scale: f64,
}

impl Menu {
    /// Opens the mode's screen, with the menus as the whole screen.
    pub fn open(
        vfs: &Vfs,
        dll: &[u8],
        mode: Mode,
        session: Session,
        resolution: Resolution,
    ) -> Result<Menu, Error> {
        Menu::open_with(
            vfs,
            dll,
            mode,
            session,
            resolution,
            Entry::Title,
            Kind::Load,
        )
    }

    /// Opens one screen the way the control bar opens it, over live playback.
    ///
    /// Host slot `+0xf8(code)` is a single call that does two things: it puts
    /// the playback object into its menu layer (`+0x220 = 3`) and stores the
    /// module's `setSystemInit` code at `+0x260`. There is no title screen
    /// underneath — the module the bar asked for is the first screen the layer
    /// opens — and `FUN_00425550` case 2 inits exactly that one code. So this
    /// is a menu that starts at `mode` with [`Entry::Playback`], not a title
    /// menu that then navigates.
    ///
    /// `kind` matters because the save and load screens are one module: codes 4
    /// and 5 both select it and only poke its `+0x94` differently, so the job
    /// has to be set before the art is chosen.
    pub fn open_over_playback(
        vfs: &Vfs,
        dll: &[u8],
        mode: Mode,
        kind: Kind,
        session: Session,
        resolution: Resolution,
    ) -> Result<Menu, Error> {
        Menu::open_with(vfs, dll, mode, session, resolution, Entry::Playback, kind)
    }

    #[allow(clippy::too_many_arguments)]
    fn open_with(
        vfs: &Vfs,
        dll: &[u8],
        mode: Mode,
        session: Session,
        resolution: Resolution,
        entry: Entry,
        kind: Kind,
    ) -> Result<Menu, Error> {
        let tab = options::Tab::DEFAULT;
        let view = replay::View::DEFAULT;
        let paths = Paths::from_module(dll);
        let variant = variant_for(&session, mode, tab, view, None, (0, 0));
        let screen = load_screen(
            vfs,
            dll,
            &paths,
            mode,
            &variant,
            Mode::TITLE,
            tab,
            kind,
            &session,
            dress::Phase::default(),
            resolution,
        )?;
        let mut menu = Menu {
            mode,
            paths,
            variant,
            states: vec![WidgetState::Resting; screen.widget_count()],
            screen,
            selection: None,
            return_to: Mode::TITLE,
            entry,
            resolution,
            dirty: true,
            session,
            tab,
            view,
            page: 0,
            episode: 0,
            map_page: 0,
            asked: None,
            kind,
            thumbnails: None,
            option_page: None,
            replay_page: None,
            dress_phase: dress::Phase::default(),
            dress: None,
            font: load_font(vfs),
            rows: None,
            pending_save: None,
            out_scale: 1.0,
        };
        menu.load_thumbnails(vfs, dll);
        menu.load_page(vfs, dll);
        menu.load_replay_page(vfs, dll);
        menu.load_dress(vfs, dll);
        menu.refresh();
        Ok(menu)
    }

    /// Loads `mode`'s screen into this menu, keeping the session.
    fn enter(&mut self, vfs: &Vfs, dll: &[u8], mode: Mode, return_to: Mode) -> Result<(), Error> {
        // Arriving at the dress-select screen starts it on its own hit map,
        // which is the module's `+0x140` coming up zero from the constructor.
        // Reloading the screen that is already showing — which is what a
        // display-mode change is — keeps whichever map is loaded, the way
        // `FUN_1000ef80` re-lays the popup out rather than dismissing it.
        if self.mode != mode {
            self.dress_phase = dress::Phase::default();
        }
        let variant = variant_for(
            &self.session,
            mode,
            self.tab,
            self.view,
            self.asked,
            (self.episode, self.map_page),
        );
        let screen = load_screen(
            vfs,
            dll,
            &self.paths,
            mode,
            &variant,
            return_to,
            self.tab,
            self.kind,
            &self.session,
            self.dress_phase,
            self.resolution,
        )?;
        self.states = vec![WidgetState::Resting; screen.widget_count()];
        self.screen = screen;
        self.refit();
        self.mode = mode;
        self.variant = variant;
        self.return_to = return_to;
        self.selection = None;
        self.load_thumbnails(vfs, dll);
        self.load_page(vfs, dll);
        self.load_replay_page(vfs, dll);
        self.load_dress(vfs, dll);
        self.refresh();
        Ok(())
    }

    /// Loads the dress-select screen's art that is not the loaded map's own.
    ///
    /// Two things: the caption `FUN_1000cce0` builds as a full-screen plate, and
    /// — while the popup's map is loaded — `DressSelect_Chip.png`, because
    /// `FUN_1000c740` keeps drawing the two dresses from it under the popup.
    /// Either missing leaves the rest of the screen drawn, which is the rule
    /// for every asset in this engine.
    fn load_dress(&mut self, vfs: &Vfs, dll: &[u8]) {
        self.dress = None;
        if self.mode != Mode::DRESS_SELECT {
            return;
        }
        let caption = self.paths.dress_select_text().and_then(|path| {
            let decoded = vfs
                .read_path(path)
                .map_err(|err| err.to_string())
                .and_then(|bytes| {
                    days_ui::Image::decode_png(&bytes).map_err(|err| err.to_string())
                });
            match decoded {
                Ok(image) => Some(image),
                Err(err) => {
                    log::warn!("dress select caption {path}: {err}");
                    None
                }
            }
        });
        // The dresses are only a layer of their own while the popup's map is
        // the loaded one; on the main map they are that screen's own widgets.
        // Loading the main screen again is what gets at both the records and
        // the sheet, since the popup's atlas describes the popup.
        let under = match self.dress_phase {
            dress::Phase::Choosing => None,
            dress::Phase::Confirming { .. } => {
                match self.load_dress_main(vfs, dll) {
                    Ok(screen) => Some(screen),
                    Err(err) => {
                        // The popup still draws; what is lost is the two
                        // dresses showing through it.
                        log::warn!("dress select: cannot draw the dresses under the popup: {err}");
                        None
                    }
                }
            }
        };
        self.dress = Some(DressScreen {
            caption,
            scaled: None,
            at: (0, 0),
            under,
        });
    }

    /// The dress-select screen's own map and sheet, loaded beside the popup's.
    fn load_dress_main(&self, vfs: &Vfs, dll: &[u8]) -> Result<Screen, Error> {
        let mut screen = load_screen(
            vfs,
            dll,
            &self.paths,
            Mode::DRESS_SELECT,
            &self.variant,
            self.return_to,
            self.tab,
            self.kind,
            &self.session,
            dress::Phase::Choosing,
            self.resolution,
        )?;
        screen.fit_to(self.screen.size().0, self.screen.size().1);
        Ok(screen)
    }
}

/// The dress-select screen's art that is not the loaded hit map's own.
///
/// Neither piece is a page layer: the caption goes **over** the screen's own
/// sprites and the dresses go **under** the popup's base art, which is the two
/// ends `FUN_1000c740` draws them at. See [`crate::ui::dress`].
struct DressScreen {
    /// `DressSelect_Text.png` in native layout space, absent when the module
    /// names none or it will not decode.
    caption: Option<days_ui::Image>,
    /// The same at the size the screen composites at, kept until [`Self::at`]
    /// moves — the caption covers the whole layout, so resampling it costs what
    /// the base art's own resample costs.
    scaled: Option<days_ui::Image>,
    at: (u32, u32),
    /// The screen's own map and sheet, loaded beside the popup's while the
    /// popup is up so the two dresses can still be drawn under it.
    under: Option<Screen>,
}

/// The Option screen's page for one tab: the records it draws from, read once
/// out of the module, and the two images they are drawn from.
///
/// A page is reloaded whenever the tab changes, which is what the module does —
/// `FUN_10007d10` releases the three sprites and takes the new tab's art.
struct OptionPage {
    /// The tab's widget rectangles, in widget order from
    /// [`option_pages::FIRST`].
    widgets: Vec<days_ui::atlas::Widget>,
    /// The same rectangles again with the hover art's `src_y`.
    hovers: Vec<days_ui::atlas::Widget>,
    /// The Sound tab's three knob sprites, empty on the other two.
    knobs: Vec<days_ui::atlas::Widget>,
    /// The tracks those knobs travel, which is what the pointer is caught by.
    tracks: Vec<days_ui::atlas::Widget>,
    /// The page's full-screen art in native layout space...
    art: days_ui::Image,
    /// ...and the same art at the size the screen is composited at, which is
    /// the expensive half and is kept until [`OptionPage::at`] moves.
    scaled: days_ui::Image,
    at: (u32, u32),
    /// The sheet this tab's sprites are cut from, which is not the screen's.
    chip: days_ui::Image,
    /// Where each of the three knobs sits, which is `+0x1bc` and is the live
    /// truth while the screen is up.
    ///
    /// `FUN_100083b0` fills it once from the three fractions as the screen
    /// loads and `FUN_1000bd20` moves it from there; nothing reads it back out
    /// of the settings. Deriving it from the stored fraction instead would put
    /// it through six decimals of text on every step.
    knob_x: Vec<f32>,
    /// Which slider the pointer is dragging, and where it was last seen.
    ///
    /// `FUN_10009430` latches `+0x24c` and `+0x170` on the press and
    /// `FUN_1000b3d0` re-enters `FUN_1000bd20` every frame until the button
    /// comes up. The pointer's last position is the other half: the shipped
    /// drag is **relative**, moving the knob by the frame's pointer delta
    /// rather than putting it under the pointer.
    drag: Option<Drag>,
}

/// A slider drag in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Drag {
    /// `+0x170`, the slider `FUN_1000bd20` is called for.
    slider: usize,
    /// The pointer x the next delta is measured from, in layout space.
    /// `FUN_1000bd20` reads `+0x3c` against `+0x44` for it.
    from: f32,
}

/// The Replay screen's page for one view: the records it draws from, resolved
/// once out of the module, and the art they are drawn over.
///
/// Only the module that ships one hit map for the whole Replay screen has
/// these; School Days HQ gives each view a map carrying all its widgets and
/// [`crate::ui::replay`] reads them straight out of it. A page is reloaded
/// whenever the view or the page changes, which is what `FUN_100293e0` does.
struct ReplayPage {
    /// Which view this is, because the sheet a hover sprite is cut from
    /// depends on it.
    view: replay::View,
    /// The view's widget rectangles, in widget order from
    /// [`replay_pages::FIRST`].
    widgets: Vec<days_ui::atlas::Widget>,
    /// The sprite each of those widgets lights when the pointer is on it, in
    /// the same order, resolved here so that [`Menu::compose`] never needs the
    /// module again.
    hovers: Vec<Option<days_ui::atlas::Widget>>,
    /// The page button of the page showing, drawn lit under the hover.
    page_mark: Option<days_ui::atlas::Widget>,
    /// The list run's first twenty records — the ten row bars and the ten
    /// comment panels beside them — which are what place the list's text.
    /// Empty on the grid, which has none.
    list: Vec<days_ui::atlas::Widget>,
    /// The view's background with the page's panel over it, in native layout
    /// space, and — on the grid — the player's unlocked thumbnails already
    /// blitted in the way `FUN_10027900` blits them...
    art: days_ui::Image,
    /// ...and the same art at the size the screen is composited at.
    scaled: days_ui::Image,
    at: (u32, u32),
    /// The sheet the view's own sprites are cut from, which is not the
    /// screen's and not the page's art.
    chip: days_ui::Image,
    /// The grid's second sheet, which only its thumbnails are cut from. `None`
    /// on the list, which has no second sheet.
    thumbs: Option<days_ui::Image>,
}

impl ReplayPage {
    /// The sheet one of this page's sprites comes from.
    ///
    /// `FUN_10024c20` draws the grid's arrows and page buttons from the chip
    /// sheet and the thumbnail under the pointer from the thumbnail sheet, in
    /// the one layer.
    fn sheet_for(&self, widget: usize) -> &days_ui::Image {
        let thumbnail = self.view == replay::View::HScene
            && (replay_pages::FIRST_THUMBNAIL
                ..replay_pages::FIRST_THUMBNAIL + replay_pages::THUMBNAILS)
                .contains(&widget);
        match (thumbnail, &self.thumbs) {
            (true, Some(thumbs)) => thumbs,
            _ => &self.chip,
        }
    }
}

/// Which widget of the title screen is `REPLAY`, and whether its caption is a
/// sprite in both of its states.
///
/// `SysMenuSD.dll`'s `FUN_1002f550` builds **two** sprites for it, `+0xd8` from
/// the first alternate and `+0xd4` from the second, and `FUN_1002f1c0` draws
/// one or the other on every pass: `FUN_1002fbd0(this, 2)` — not the trial
/// build and host `+0x108(1)` — picks `+0xd8`, and anything else picks `+0xd4`.
/// So on that module the base art carries no `REPLAY` caption at all and
/// leaving the widget resting draws nothing, which is what it did here.
/// `SysMenuSDHQ.dll` keeps the live caption in the base art and gives the dead
/// one the single alternate.
///
/// The two are told apart by the table rather than by the module: a screen that
/// draws both states holds two alternates over the **same rectangle**, where
/// the other module's second alternate belongs to a different row entirely.
/// `None` is a title with no alternate at all, which draws nothing either way.
const TITLE_REPLAY: usize = 2;

fn title_replay_caption(extras: &[days_ui::atlas::Widget], widget: usize) -> Option<bool> {
    debug_assert_eq!(widget, TITLE_REPLAY);
    match extras {
        [first, second, ..] => Some(first.dst == second.dst),
        [_] => Some(false),
        [] => None,
    }
}

/// Which of the two screens that list the player's slots is showing.
///
/// The save/load screen and the replay screen's play-data list read the same
/// hundred slots through the same host call, and both rasterise a page of them
/// into one off-screen surface. What differs is the records and three details
/// [`crate::ui::playdata`] sets out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum List {
    SaveLoad,
    PlayData,
    /// The same list on the module that lays it out from a record run rather
    /// than from its own hit map — see [`crate::ui::replay_pages`].
    ReplayPages,
}

impl Menu {
    /// Rasterises the save/load screen's rows for the page it is showing.
    ///
    /// Rebuilt on entering the screen and on turning a page, which is when the
    /// shipped `FUN_10011ec0` runs: the surface holds one page at a time.
    pub fn load_rows(&mut self) {
        self.rows = None;
        let list = match self.mode {
            Mode::SAVELOAD => List::SaveLoad,
            Mode::REPLAY if self.view != replay::View::PlayData => return,
            Mode::REPLAY if self.paths.replay_has_pages() => List::ReplayPages,
            Mode::REPLAY => List::PlayData,
            _ => return,
        };
        let Some(font) = &self.font else {
            log::warn!("no font, so the slot rows stay empty");
            return;
        };
        if list == List::ReplayPages {
            let Some(records) = self.replay_page.as_ref().map(|page| page.list.clone()) else {
                return;
            };
            self.rows = Some(replay_pages::render(
                font,
                &self.session.slots,
                self.page,
                &records,
                self.session.text_input,
                self.session.english,
                self.selection.and_then(replay_pages::tooltip_row),
            ));
            self.dirty = true;
            return;
        }
        if list == List::PlayData {
            let hovered = self.selection.and_then(playdata::tooltip_row);
            self.rows = Some(playdata::render(
                font,
                &self.session.slots,
                self.page,
                &self.screen.atlas().widgets,
                self.session.text_input,
                hovered,
            ));
            self.dirty = true;
            return;
        }
        // The selection is what opens the expanded comment, so this is rebuilt
        // on every hover — which is what the shipped screen does: its
        // `FUN_10012900` re-runs the whole row rasterising before laying the
        // tooltip out.
        let hovered = self
            .selection
            .filter(|w| (0x16..0x20).contains(w))
            .map(|w| w - 0x16);
        self.rows = Some(saveload::Rows::render(
            font,
            &self.session.slots,
            self.page,
            self.session.english,
            &self.screen.atlas().widgets,
            self.session.text_input,
            hovered,
        ));
        self.dirty = true;
    }

    /// Loads the thumbnail sheet for the page the replay grid is showing.
    ///
    /// This is the grid of the module that gives each view its own hit map; the
    /// other module draws its grid from [`Self::load_replay_page`] instead, out
    /// of one sheet for every page, so the two are exclusive.
    ///
    /// A page whose art or table will not load leaves the frames empty and the
    /// grid still usable, which is the same rule every other missing asset
    /// follows.
    fn load_thumbnails(&mut self, vfs: &Vfs, dll: &[u8]) {
        self.thumbnails = None;
        if self.mode != Mode::REPLAY
            || self.view != replay::View::HScene
            || self.paths.replay_has_pages()
        {
            return;
        }
        let path = replay::thumbnail_sheet(self.page);
        let sheet = match vfs
            .read_path(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| days_ui::Image::decode_png(&bytes).map_err(|e| e.to_string()))
        {
            Ok(sheet) => sheet,
            Err(err) => {
                log::warn!("replay thumbnails {path}: {err}");
                return;
            }
        };
        let slots: Vec<days_ui::cmap::Rect> = self
            .screen
            .atlas()
            .widgets
            .iter()
            .skip(replay::HSCENE_FIRST_THUMBNAIL)
            .map(|w| w.dst)
            .collect();
        match replay::Thumbnails::recover(dll, &slots, (sheet.width, sheet.height)) {
            Ok(table) => self.thumbnails = Some((sheet, table)),
            Err(err) => log::warn!("no thumbnail table for {path}: {err}"),
        }
    }

    /// Loads the Replay page for the view and page showing.
    ///
    /// Only one of the two menu modules has these; the other gives each view a
    /// hit map carrying all of its widgets. A view whose tables cannot be
    /// placed, or whose art will not load, leaves the frame drawn and the two
    /// view headers working, which is the rule every other missing asset
    /// follows.
    fn load_replay_page(&mut self, vfs: &Vfs, dll: &[u8]) {
        self.replay_page = None;
        if self.mode != Mode::REPLAY || !self.paths.replay_has_pages() {
            return;
        }
        let Some(pages) = replay_pages::Pages::locate(dll, self.screen.atlas().offset) else {
            log::warn!("no Replay page tables in this module; the views draw empty");
            return;
        };
        let view = self.view;
        let read = |path: &str| {
            vfs.read_path(path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| days_ui::Image::decode_png(&bytes).map_err(|e| e.to_string()))
        };
        let Some(panel_path) = replay_pages::panel_art(view, self.page) else {
            log::warn!("replay {} has no page {}", view.variant(), self.page);
            return;
        };
        let (mut art, chip) = match (
            read(replay_pages::base_art(view)),
            read(replay_pages::chip_art(view)),
        ) {
            (Ok(art), Ok(chip)) => (art, chip),
            (Err(err), _) | (_, Err(err)) => {
                log::warn!("Replay page art for the {} view: {err}", view.variant());
                return;
            }
        };
        // The panel goes over the background at the origin: at rest the page
        // showing is scrolled exactly over it, and a still frame is one page.
        let (panel_x, panel_y) = replay_pages::panel_origin(view);
        match read(&panel_path) {
            Ok(panel) => art.blit_scaled(
                &panel,
                (0, 0, panel.width, panel.height),
                (panel_x, panel_y, panel.width, panel.height),
            ),
            Err(err) => log::warn!("replay panel {panel_path}: {err}"),
        }
        let thumbs = match view {
            replay::View::HScene => {
                self.blit_resting_thumbnails(&mut art, &pages, dll, &read);
                match read(replay_pages::HOVER_THUMBNAILS) {
                    Ok(sheet) => Some(sheet),
                    Err(err) => {
                        log::warn!(
                            "replay thumbnails {}: {err}",
                            replay_pages::HOVER_THUMBNAILS
                        );
                        None
                    }
                }
            }
            replay::View::PlayData => None,
        };
        let scaled = self.screen.to_display(&art);
        let widgets = pages.widgets(dll, view);
        let hovers = (0..widgets.len())
            .map(|index| pages.hover(dll, view, index + replay_pages::FIRST, self.page))
            .collect();
        // The list's text is laid out against the run the hit table does not
        // hold, so it is read here with everything else the page needs.
        let list = match view {
            replay::View::PlayData => (0..replay_pages::PER_PAGE * 2)
                .map_while(|index| days_ui::atlas::record_at(dll, pages.list(), index))
                .collect(),
            replay::View::HScene => Vec::new(),
        };
        self.replay_page = Some(ReplayPage {
            view,
            widgets,
            hovers,
            list,
            page_mark: pages.page_mark(dll, view, self.page),
            art,
            scaled,
            at: self.screen.size(),
            chip,
            thumbs,
        });
        self.dirty = true;
    }

    /// Blits the player's unlocked thumbnails into the grid's page art, the way
    /// `FUN_10027900` blits them.
    ///
    /// The shipped loop walks all 36 h-scenes, asks the host whether each one's
    /// flag is set, and copies only those out of
    /// [`replay_pages::RESTING_THUMBNAILS`] into the panel of the page they
    /// belong to — so a scene the player has not seen leaves its slot bare.
    /// Here only the page showing is built, because only its panel is drawn.
    fn blit_resting_thumbnails(
        &self,
        art: &mut days_ui::Image,
        pages: &replay_pages::Pages,
        dll: &[u8],
        read: &impl Fn(&str) -> Result<days_ui::Image, String>,
    ) {
        let sheet = match read(replay_pages::RESTING_THUMBNAILS) {
            Ok(sheet) => sheet,
            Err(err) => {
                log::warn!(
                    "replay thumbnails {}: {err}",
                    replay_pages::RESTING_THUMBNAILS
                );
                return;
            }
        };
        for slot in 0..replay_pages::THUMBNAILS {
            let Some(scene) = replay_pages::scene_of(self.page, slot) else {
                continue;
            };
            if !self.session.scenes.unlocked(scene, &self.session.flags) {
                continue;
            }
            let Some(cell) = pages.thumbnail(dll, self.page, slot) else {
                continue;
            };
            art.blit_scaled(
                &sheet,
                (cell.src_x, cell.src_y, cell.dst.width, cell.dst.height),
                (
                    i64::from(cell.dst.x),
                    i64::from(cell.dst.y),
                    cell.dst.width,
                    cell.dst.height,
                ),
            );
        }
    }

    /// Loads the Option page for the tab showing.
    ///
    /// Only one of the two menu modules has pages: the other gives each tab its
    /// own hit map, and its widgets are the screen's own. A page whose table
    /// cannot be placed, or whose art will not load, leaves the frame drawn and
    /// the tab headers working, which is the rule every other missing asset
    /// follows.
    fn load_page(&mut self, vfs: &Vfs, dll: &[u8]) {
        self.option_page = None;
        if self.mode != Mode::OPTION || self.paths.option_tabs_have_own_map() {
            return;
        }
        let Some(pages) = option_pages::Pages::locate(dll, self.screen.atlas().offset) else {
            log::warn!("no Option page tables in this module; the tabs draw empty");
            return;
        };
        let (art, chip) = option_pages::art(self.tab, self.session.som);
        let read = |path: &str| {
            vfs.read_path(path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| days_ui::Image::decode_png(&bytes).map_err(|e| e.to_string()))
        };
        let (art, chip) = match (read(art), read(chip)) {
            (Ok(art), Ok(chip)) => (art, chip),
            (Err(err), _) | (_, Err(err)) => {
                log::warn!("Option page art for the {} tab: {err}", self.tab.variant());
                return;
            }
        };
        let scaled = self.screen.to_display(&art);
        self.option_page = Some(OptionPage {
            widgets: pages.widgets(dll, self.tab),
            hovers: pages.hovers(dll, self.tab),
            knobs: (0..option_pages::SLIDERS)
                .filter(|_| self.tab == options::Tab::Sound)
                .map_while(|slider| pages.knob(dll, slider))
                .collect(),
            tracks: (0..option_pages::SLIDERS)
                .filter(|_| self.tab == options::Tab::Sound)
                .map_while(|slider| pages.track(dll, slider))
                .collect(),
            art,
            scaled,
            at: self.screen.size(),
            chip,
            knob_x: Vec::new(),
            drag: None,
        });
        // `FUN_100083b0` works the three knob positions out from the three
        // fractions as the last thing it does, once the tables are in hand.
        let knob_x = (0..option_pages::SLIDERS)
            .map_while(|slider| {
                let page = self.option_page.as_ref()?;
                let channel = *option_pages::SLIDER_CHANNELS.get(slider)?;
                Some(option_pages::slider_knob_x(
                    page.tracks.get(slider)?,
                    page.knobs.get(slider)?,
                    self.session.config.fraction(channel),
                ))
            })
            .collect();
        if let Some(page) = &mut self.option_page {
            page.knob_x = knob_x;
        }
        self.dirty = true;
    }

    /// The session, for an engine that needs to read the settings.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The session, for an engine that has just changed the display mode.
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// Which Option tab is showing.
    pub fn tab(&self) -> options::Tab {
        self.tab
    }

    /// Which Replay screen is showing, and which page of it.
    pub fn replay_view(&self) -> (replay::View, usize) {
        (self.view, self.page)
    }

    /// The scene the replay popup is asking about, if it is up.
    pub fn asked(&self) -> Option<usize> {
        self.asked
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The save/load screen's rasterised rows, for a tool that wants to report
    /// what the composite would draw.
    pub fn rows(&self) -> Option<&saveload::Rows> {
        self.rows.as_ref()
    }

    /// Composites at `width` x `height` from here on. See [`Screen::fit_to`].
    ///
    /// What is remembered is the *factor*, not the size: every screen carries
    /// its own hit map — `MENUBAR`'s is 800x75 where the title's is 1280x720 —
    /// so a width that fits one screen means nothing to the next. The factor
    /// means the same thing to all of them, and it is what [`Menu::enter`]
    /// re-applies to the screen it loads.
    ///
    /// Marks the frame dirty when the size really moves, because the one
    /// already composed is the wrong size then. A call that asks for the size
    /// the screen is already at changes nothing, so that a caller can hand this
    /// the size it wants every pass without recomposing every pass.
    pub fn set_output_size(&mut self, width: u32, height: u32) {
        let (map_w, _) = self.screen.map_size();
        if map_w != 0 && width != 0 {
            self.out_scale = f64::from(width) / f64::from(map_w);
        }
        let was = self.screen.size();
        self.screen.fit_to(width, height);
        self.dirty |= self.screen.size() != was;
    }

    /// Puts the menu's output scale on the screen that is loaded.
    ///
    /// A freshly loaded [`Screen`] composites at its hit map's size, so without
    /// this every navigation would silently drop back to it — and the caller,
    /// having asked once, has no reason to ask again.
    fn refit(&mut self) {
        if self.out_scale == 1.0 {
            return;
        }
        let (map_w, map_h) = self.screen.map_size();
        let at = |v: u32| (f64::from(v) * self.out_scale).round().max(1.0) as u32;
        self.screen.fit_to(at(map_w), at(map_h));
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

    /// Composites the current frame over an optional backdrop and marks it
    /// clean. The backdrop is in display space; see [`Screen::compose_over`].
    pub fn compose(&mut self, backdrop: Option<&days_ui::Image>) -> days_ui::Image {
        self.dirty = false;
        self.refit_page();
        let page_sprites = self.page_sprites();
        let sprites = self.sprites();
        let page = self.page_art().map(|art| crate::ui::screen::Page {
            art,
            sprites: &page_sprites,
        });
        // The two dresses go *under* the popup's own base art, which is the
        // only thing on any screen that does — `FUN_1000c740` draws them from
        // the previous map's sheet before it draws the popup. So they are part
        // of what the popup is composited over rather than a layer inside it.
        let under = self.dress_under(backdrop);
        let backdrop = under.as_ref().or(backdrop);
        let mut out = self
            .screen
            .compose_over_page(backdrop, page, &self.states, &sprites);
        // The caption is the other way round: `FUN_1000c740` draws it after the
        // dresses and their highlight, and only while the main map is loaded,
        // so the popup covers it rather than sitting under it.
        if let (dress::Phase::Choosing, Some(caption)) = (
            self.dress_phase,
            self.dress.as_ref().and_then(|d| d.scaled.as_ref()),
        ) {
            let (w, h) = (caption.width, caption.height);
            out.blit_scaled(
                caption,
                (0, 0, w, h),
                (0, self.screen.letterbox().round() as i64, w, h),
            );
        }
        // The save/load rows are not widget sprites: their source is twice the
        // size of their destination, so they are blitted with the averaging
        // downscale.
        if let Some(rows) = &self.rows {
            for quad in &rows.quads {
                out.blit_downscaled(&rows.surface, quad.src, self.screen.place_layout(quad.dst));
            }
            // The expanded comment goes over the list, panel first. The panel
            // is cut from the chip sheet at the record's full height however
            // short it is drawn, so a one- or two-line panel is that art
            // squashed — the shipped sprite's own source rectangle.
            if let Some(tip) = &rows.tooltip {
                // The panel is cut from the sheet the list's own sprites come
                // from: the view's chip sheet on the module that draws its
                // views from a record run, and the screen's own otherwise.
                let chip = match &self.replay_page {
                    Some(page) if page.view == replay::View::PlayData => &page.chip,
                    _ => self.screen.chip(),
                };
                let panel = &tip.panel;
                out.blit_downscaled(chip, panel.src, self.screen.place_layout(panel.dst));
                let from = rows.tip_surface.as_ref().unwrap_or(&rows.surface);
                for line in &tip.lines {
                    out.blit_downscaled(from, line.src, self.screen.place_layout(line.dst));
                }
            }
        }
        out
    }

    /// Redraws the page's art at the size the screen is composited at.
    ///
    /// The page covers the whole layout, so resampling it is as expensive as
    /// the base art's own resample and is kept the same way: done when the size
    /// moves, not every frame.
    fn refit_page(&mut self) {
        let size = self.screen.size();
        if let Some(page) = &mut self.option_page {
            if page.at != size {
                page.scaled = self.screen.to_display(&page.art);
                page.at = size;
            }
        }
        if let Some(page) = &mut self.replay_page {
            if page.at != size {
                page.scaled = self.screen.to_display(&page.art);
                page.at = size;
            }
        }
        if let Some(dress) = &mut self.dress {
            if dress.at != size {
                dress.scaled = dress
                    .caption
                    .as_ref()
                    .map(|art| self.screen.to_display(art));
                dress.at = size;
            }
        }
    }

    /// What the dress-select popup is composited over: the two dresses, slid
    /// together, cut from the sheet the previous hit map came with.
    ///
    /// `None` on every other screen and while the main map is loaded, where the
    /// dresses are the loaded screen's own widgets instead.
    fn dress_under(&self, backdrop: Option<&days_ui::Image>) -> Option<days_ui::Image> {
        let dress::Phase::Confirming { chosen } = self.dress_phase else {
            return None;
        };
        let under = self.dress.as_ref()?.under.as_ref()?;
        let cuts = dress::committed(under.atlas(), chosen);
        let (width, height) = self.screen.size();
        let mut out = days_ui::Image::empty(width, height);
        if let Some(back) = backdrop {
            out.blit_scaled(
                back,
                (0, 0, back.width, back.height),
                (0, 0, back.width, back.height),
            );
        }
        under.draw_cuts_from(&mut out, under.chip(), &cuts);
        Some(out)
    }

    /// The art of whichever page layer this screen has, at display scale.
    ///
    /// At most one is ever loaded: [`Menu::load_page`] takes only the Option
    /// screen and [`Menu::load_replay_page`] only the Replay screen.
    fn page_art(&self) -> Option<&days_ui::Image> {
        match (&self.option_page, &self.replay_page) {
            (Some(page), _) => Some(&page.scaled),
            (_, Some(page)) => Some(&page.scaled),
            _ => None,
        }
    }

    /// What a page layer draws over its own art, each sprite with the sheet it
    /// is cut from.
    fn page_sprites(&self) -> Vec<(&days_ui::Image, days_ui::atlas::Widget)> {
        if let Some(page) = &self.option_page {
            return self
                .option_page_sprites()
                .into_iter()
                .map(|sprite| (&page.chip, sprite))
                .collect();
        }
        self.replay_page_sprites()
    }

    /// What the Replay view draws over its own art, in the order
    /// `FUN_10024c20` and `FUN_10024e30` put them down: the page button of the
    /// page showing, then the widget under the pointer.
    fn replay_page_sprites(&self) -> Vec<(&days_ui::Image, days_ui::atlas::Widget)> {
        let Some(page) = &self.replay_page else {
            return Vec::new();
        };
        let mut out: Vec<(&days_ui::Image, days_ui::atlas::Widget)> = Vec::new();
        if let Some(mark) = page.page_mark {
            out.push((&page.chip, mark));
        }
        if let Some(widget) = self.selection.filter(|w| *w >= replay_pages::FIRST) {
            if let Some(hover) = widget
                .checked_sub(replay_pages::FIRST)
                .and_then(|index| page.hovers.get(index))
                .copied()
                .flatten()
            {
                out.push((page.sheet_for(widget), hover));
            }
        }
        out
    }

    /// What the Option page draws over its own art, in the order the tab's draw
    /// puts them down.
    ///
    /// `FUN_10006500`, `FUN_10006a20` and `FUN_100070f0` all go: the value in
    /// force on each of the tab's rows, then the widget under the pointer, and
    /// the Sound tab then puts its three knobs on top of both.
    fn option_page_sprites(&self) -> Vec<days_ui::atlas::Widget> {
        let Some(page) = &self.option_page else {
            return Vec::new();
        };
        let record = |widget: usize| {
            widget
                .checked_sub(option_pages::FIRST)
                .and_then(|index| page.widgets.get(index))
                .copied()
        };
        let mut out: Vec<days_ui::atlas::Widget> = option_pages::values(
            self.tab,
            &self.session.config,
            self.session.display,
            self.session.som,
        )
        .into_iter()
        .filter_map(record)
        .collect();

        if let Some(widget) = self.selection.filter(|w| *w >= option_pages::FIRST) {
            if let Some(hover) = widget
                .checked_sub(option_pages::FIRST)
                .and_then(|index| page.hovers.get(index))
            {
                // A slider's hover art is drawn on the knob rather than on the
                // track the pointer was caught by.
                out.push(match self.slider_at(widget).and_then(|s| self.knob_x(s)) {
                    Some(x) => option_pages::at_x(hover, x + option_pages::SLIDER_HOVER_OFFSET),
                    None => *hover,
                });
            }
        }

        for slider in 0..page.knobs.len() {
            let (Some(knob), Some(x)) = (page.knobs.get(slider), self.knob_x(slider)) else {
                continue;
            };
            out.push(option_pages::at_x(knob, x));
        }
        out
    }

    /// Which Sound slider a widget is, if it is one.
    fn slider_at(&self, widget: usize) -> Option<usize> {
        let slider = widget.checked_sub(option_pages::FIRST_SLIDER)?;
        (self.tab == options::Tab::Sound && slider < option_pages::SLIDERS).then_some(slider)
    }

    /// Where a Sound slider's knob sits: `+0x1bc`.
    fn knob_x(&self, slider: usize) -> Option<f32> {
        self.option_page.as_ref()?.knob_x.get(slider).copied()
    }

    /// The sprites this screen draws that are not widget states.
    ///
    /// Three screens have them: the Sound tab's three volume bars, cut from the
    /// chip sheet, the replay grid's thumbnails, cut from the page's own sheet,
    /// and the route map's "you are here" marker, which goes over a cell rather
    /// than instead of it. See [`options::volume_bar`], [`replay::Thumbnails`]
    /// and [`routemap::Standing::marker`]. All three are drawn at their source
    /// size; the save/load rows are not, so they take the separate path in
    /// [`Menu::compose`].
    fn sprites(&self) -> Vec<(&days_ui::Image, days_ui::atlas::Widget)> {
        let mut out = Vec::new();
        // The value in force on every row of an Option tab, and the tab's own
        // header. Each is one record out of the screen's first alternate run,
        // carrying its own destination, so the highlight sits on whichever
        // button of the pair holds the value rather than on a fixed one. The
        // tab's draw function does exactly this before it looks at the pointer
        // at all — `FUN_100063c0` for the Sound tab.
        // A page module draws one of these and only one: the tab's own header,
        // which `FUN_100083b0` takes from the frame table at record `tab + 4`.
        // Everything else on its screen belongs to the page.
        // The dress under the pointer, drawn over the two resting ones.
        if self.mode == Mode::DRESS_SELECT && self.dress_phase == dress::Phase::Choosing {
            if let Some(lit) = self
                .selection
                .and_then(|widget| dress::lit(self.screen.atlas(), widget))
            {
                out.push((self.screen.chip(), lit));
            }
        }
        if self.mode == Mode::OPTION && self.option_page.is_some() {
            if let Some(header) = self.screen.atlas().extras.get(self.tab.index()) {
                out.push((self.screen.chip(), *header));
            }
        }
        if self.mode == Mode::OPTION && self.option_page.is_none() {
            let values = options::current_values(
                self.tab,
                &self.session.config,
                self.session.display,
                self.session.som,
            );
            for widget in values {
                let Some(slot) = options::highlight_slot(self.tab, widget) else {
                    continue;
                };
                if let Some(sprite) = self.screen.atlas().extras.get(slot) {
                    out.push((self.screen.chip(), *sprite));
                }
            }
        }
        match self.mode {
            Mode::OPTION if self.tab == options::Tab::Sound && self.option_page.is_none() => {
                for row in 0..options::VOLUME_ROW_COUNT {
                    let Some((channel, first)) = options::volume_row(row) else {
                        continue;
                    };
                    let cells = self
                        .screen
                        .atlas()
                        .widgets
                        .get(first..first + options::VOLUME_CELLS);
                    if let Some(bar) = cells
                        .and_then(|c| options::volume_bar(c, self.session.config.volume(channel)))
                    {
                        out.push((self.screen.chip(), bar));
                    }
                }
            }
            Mode::REPLAY if self.view == replay::View::HScene => {
                let Some((sheet, table)) = &self.thumbnails else {
                    return out;
                };
                for slot in 0..table.len() {
                    let widget = replay::HSCENE_FIRST_THUMBNAIL + slot;
                    if !self.enabled(widget) {
                        continue;
                    }
                    if let Some(sprite) = table.sprite(slot, self.selection == Some(widget)) {
                        out.push((sheet, sprite));
                    }
                }
            }
            // The "you are here" marker, which goes over the cell the player is
            // standing on rather than replacing its state sprite — so it is a
            // sprite of its own and not a `WidgetState::Extra`.
            //
            // `FUN_1000ce40` cuts it from the third of the page's four per-cell
            // bands, record `0x21 + 2 * cells + i`, and the atlas numbers its
            // alternates from the end of the second, so it is `cells + i` here.
            //
            // `FUN_1000c3d0` draws the three sprites a cell can carry in a
            // fixed order — its state, then the marker, then the hover art —
            // and all three are the same opaque 38x38 dot in the same place, so
            // the hover art wins wherever the pointer is. Here the states go
            // down in one pass before these sprites do, which puts the marker
            // on top of the hover art rather than under it; holding it back on
            // the cell the pointer is on is that order's visible half.
            Mode::ROUTEMAP => {
                let page = self.chart_page();
                if let Some(sprite) = self.marker().and_then(|cell| {
                    self.screen
                        .atlas()
                        .extras
                        .get(page.cells + cell)
                        .filter(|_| self.selection != Some(routemap::FIRST_CELL + cell))
                }) {
                    out.push((self.screen.chip(), *sprite));
                }
            }
            _ => {}
        }
        out
    }

    /// Whether a widget can be chosen.
    ///
    /// Each screen has its own rule in the DLL, and each is transcribed where
    /// that screen's behaviour lives: the title's here, the Option screen's in
    /// [`options::enabled`] (`FUN_10007ca0`), the replay grid's in
    /// [`replay::hscene_enabled`] (`FUN_1001dd40`) and the replay popup's in
    /// [`replay::popup_enabled`] (`FUN_100195d0`). A screen with no recovered
    /// rule treats every widget as live.
    pub fn enabled(&self, widget: usize) -> bool {
        // A page's widgets are numbered past the frame's four, so they are not
        // in `states` at all and the length test below would refuse every one.
        if self.option_page.is_some() && self.mode == Mode::OPTION {
            return option_pages::enabled(
                self.tab,
                widget,
                self.session.save.trial,
                self.session.som,
            );
        }
        if self.replay_page.is_some() && self.mode == Mode::REPLAY {
            return replay_pages::enabled(self.view, widget, &self.unlocked_slots());
        }
        if widget >= self.states.len() {
            return false;
        }
        match self.mode {
            Mode::TITLE => match widget {
                2 => self.session.save.replay_unlocked(),
                5 => self.session.save.all_clear,
                _ => true,
            },
            Mode::OPTION => {
                options::enabled(self.tab, widget, self.session.save.trial, self.session.som)
            }
            Mode::REPLAY if self.view == replay::View::HScene => {
                replay::hscene_enabled(&self.session.scenes, self.page, widget, &self.session.flags)
            }
            Mode::REPLAY_POPUP => match self.asked.and_then(|s| self.session.scenes.get(s)) {
                Some(scene) => replay::popup_enabled(scene, widget, &self.session.flags),
                None => false,
            },
            // Every widget, until the confirm popup goes up — which this
            // engine does not raise, so `popup_up` is always false here.
            Mode::SAVELOAD => saveload::enabled(false, widget),
            Mode::ROUTEMAP => {
                let page = self.chart_page();
                routemap::enabled(
                    self.episode,
                    self.map_page,
                    page.cells,
                    self.session.save.trial,
                    &self.pickable(),
                    widget,
                )
            }
            _ => true,
        }
    }

    /// Where the Option screen's second alternate run starts, as an index into
    /// [`Atlas::extras`](days_ui::atlas::Atlas::extras).
    ///
    /// `FUN_100076c0` puts the run's first record in `+0x190` — 27 records in
    /// for the Def tab, 51 for Sound and 35 for the SOMCON tab — and
    /// `FUN_100073a0` builds its sprites from there. The per-widget run is as
    /// long as the hit map has regions, so the alternates start that far in.
    fn option_run_b(&self) -> Option<usize> {
        let start = match self.tab {
            options::Tab::Def => 27usize,
            options::Tab::Sound => 51,
            options::Tab::SomCon => 35,
        };
        start.checked_sub(self.states.len())
    }

    /// The alternate-state sprite a widget draws instead of its hover art, if
    /// any.
    fn extra_for(&self, widget: usize) -> Option<usize> {
        match self.mode {
            Mode::TITLE if widget == TITLE_REPLAY => {
                title_replay_caption(&self.screen.atlas().extras, widget).and_then(|both| {
                    let unlocked = self.session.save.replay_unlocked();
                    match both {
                        // Both states are sprites: one of the two is always
                        // drawn.
                        true => Some(if unlocked { 0 } else { 1 }),
                        // Only the dead state is; the live one is in the
                        // base art.
                        false => (!unlocked).then_some(0),
                    }
                })
            }
            // Both replay views mark the tab and the page you are on, which no
            // resting/active pair can say. An index past the alternates the
            // screen actually placed draws nothing: see
            // [`replay::place_alternates`] for when that happens.
            Mode::REPLAY => {
                let selected = self.selection == Some(widget);
                let extra = match self.view {
                    replay::View::HScene => {
                        replay::extra_for(widget, self.view, self.page, selected)
                    }
                    replay::View::PlayData => {
                        playdata::extra_for(widget, self.view, self.page, selected)
                    }
                }?;
                (extra < self.screen.atlas().extras.len()).then_some(extra)
            }
            // A page module has no alternate widget states on this screen at
            // all. `FUN_10006110` draws the frame last and draws exactly two
            // things there: the tab highlight, which is record `tab + 4` and
            // belongs to no widget, and then — for each of the four frame
            // widgets — that widget's **own** record, if it is the one under
            // the pointer. There is no withholding and no second run, so the
            // open tab's header draws its ordinary hover art like any other
            // widget. The page's own widgets are the same: `FUN_1000adc0`,
            // `FUN_1000af90` and `FUN_1000b1d0` take one record per widget out
            // of the run behind the tab's, which is [`option_pages::Pages::hover`].
            Mode::OPTION if self.option_page.is_some() => None,
            // The value in force is not a widget state — it is a sprite of
            // its own, drawn in [`Menu::sprites`]. What a widget can carry here
            // is the *second* alternate run: the same highlight art with the
            // hover outline, which the pointer's own widget draws in place of
            // its hover sprite when it is already the value in force.
            Mode::OPTION => {
                if self.selection != Some(widget)
                    || !options::withholds_hover_art(
                        self.tab,
                        widget,
                        &self.session.config,
                        self.session.display,
                        self.session.som,
                    )
                {
                    return None;
                }
                let slot = options::highlight_slot(self.tab, widget)?;
                self.option_run_b()?.checked_add(slot)
            }
            // A cell of the route map paints its own state over the empty
            // chart in the base art. `FUN_1000ce40` builds the sprite from one
            // of two bands of the page's record table — `+0x21 + cells + i`
            // for a story point this run has not passed and `+0x21 + 3 * cells
            // + i` for one it has — and `FUN_1000c3d0` paints it only where the
            // global store says the point has ever been seen. The band the
            // regions themselves matched is the first, `+0x21 + i`, which is
            // the hover art, so a cell the pointer is on wants its own record
            // and not an alternate. The band between the two here is the "you
            // are here" marker, which is not a widget state: see
            // [`Menu::sprites`].
            Mode::ROUTEMAP => {
                let cells = self.chart_page().cells;
                let routemap::Act::Cell(cell) = routemap::action(cells, widget) else {
                    return None;
                };
                if !self.charted().get(cell).copied().unwrap_or(false) {
                    return None;
                }
                if self.selection == Some(widget) {
                    return None;
                }
                let extra = if self.pickable().get(cell).copied().unwrap_or(false) {
                    2 * cells + cell
                } else {
                    cell
                };
                (extra < self.screen.atlas().extras.len()).then_some(extra)
            }
            _ => None,
        }
    }

    /// Which widget's sprite the current selection lights.
    ///
    /// Usually the selected widget itself, but a screen is free to draw
    /// something else: the save/load screen's rows have two hit bands and one
    /// sprite between them, so pointing at either half lights the whole row.
    /// See [`saveload::highlight`] (`FUN_10011600`).
    fn lit(&self) -> Option<usize> {
        let selection = self.selection?;
        match self.mode {
            Mode::SAVELOAD => saveload::highlight(self.kind, selection),
            Mode::REPLAY if self.view == replay::View::PlayData => playdata::highlight(selection),
            _ => Some(selection),
        }
    }

    /// Recomputes every widget's sprite from the current selection.
    ///
    /// A disabled `REPLAY` draws the one alternate record that follows the
    /// title's table in the DLL — `extras[0]`, the greyed-out caption. Only
    /// that first extra belongs to this screen: the run after it is the next
    /// screen's table, which the atlas cannot see the end of.
    fn refresh(&mut self) {
        self.load_rows();
        let lit = self.lit();
        let states: Vec<WidgetState> = (0..self.states.len())
            .map(|i| match self.extra_for(i) {
                Some(extra) => WidgetState::Extra(extra),
                None if lit == Some(i) => WidgetState::Active,
                None => WidgetState::Resting,
            })
            .collect();
        self.states = states;
        // The dress-select screen's base art is the shared transparent plate,
        // so its two widgets are not revealed by their base art the way every
        // other screen's are — they are sprites `FUN_1000c740` draws every
        // frame, before it looks at the selection at all. The lit record goes
        // over the one under the pointer; see `Menu::sprites`.
        if self.mode == Mode::DRESS_SELECT && self.dress_phase == dress::Phase::Choosing {
            self.states = vec![WidgetState::Active; self.states.len()];
        }
        self.dirty = true;
    }

    /// Moves the pointer. Coordinates are in the screen's own pixel space.
    pub fn point_at(&mut self, x: u32, y: u32) -> Action {
        // A drag owns the pointer while it lasts: `FUN_1000b3d0` goes straight
        // to `FUN_1000bd20` and never reaches the hit test, so the selection
        // stays on the slider however far the pointer wanders off the track.
        if self.dragging().is_some() {
            return self.drag_to(x);
        }
        let hit = match self.screen.hit(x, y) {
            Some(widget) => Some(widget),
            None => self.page_hit(x, y),
        };
        let hit = hit.filter(|widget| self.enabled(*widget) || self.grabbing(*widget, x));
        self.select(hit)
    }

    /// The slider a drag is in progress on.
    fn dragging(&self) -> Option<usize> {
        Some(self.option_page.as_ref()?.drag?.slider)
    }

    /// The pointer went down.
    ///
    /// Only one thing on these screens cares: `FUN_10009430`'s last arm takes
    /// widgets 8 to 10, checks the gate — which for those three is
    /// `FUN_1000c280`, the knob's own hit test — and only then latches
    /// `+0x24c` and `+0x170`. It also runs one `FUN_1000bd20` immediately,
    /// which with the pointer where it already was moves nothing: the drag
    /// tests `+0x3c` against `+0x44` and returns when they match.
    ///
    /// Everything else is decided on the release, so a press elsewhere is not
    /// an activation and this says so.
    ///
    /// Only the pointer's `x` is wanted: the widget is already named by the
    /// selection, the way `FUN_10009430` is handed one, and `FUN_1000c280`
    /// tests `x` alone.
    pub fn press(&mut self, x: u32) -> Action {
        let Some(widget) = self.selection else {
            return Action::Stay;
        };
        let Some(slider) = self.slider_at(widget).filter(|_| self.grabbing(widget, x)) else {
            return Action::Stay;
        };
        let (x, _) = self.screen.to_layout(x, 0);
        if let Some(page) = &mut self.option_page {
            page.drag = Some(Drag { slider, from: x });
        }
        Action::Stay
    }

    /// The pointer came up, which is the only thing that ends a drag.
    ///
    /// `FUN_1000bd20`'s first test is `+0x58`, the held flag: with the button
    /// up it clears `+0x24c` and does nothing else, so the last value written
    /// is the one that stands.
    pub fn release(&mut self) -> Action {
        if let Some(page) = &mut self.option_page {
            page.drag = None;
        }
        Action::Stay
    }

    /// One step of a slider drag: `FUN_1000bd20`.
    ///
    /// The knob moves by the pointer's **delta**, not to the pointer — the
    /// shipped line is `+0x1bc[n] -= width * (+0x3c - +0x44)`, the two being
    /// the previous and current pointer as fractions of the client width. So a
    /// press that grabs the knob off-centre keeps its grip, and a pointer that
    /// runs past the end of the track and comes back does not bank the
    /// overshoot: the knob is clamped to the track on every step, before the
    /// value is taken from it.
    ///
    /// The value then goes to the member, to the INI key, and to the host —
    /// which this engine does by writing the config the mixer reads.
    fn drag_to(&mut self, x: u32) -> Action {
        let (x, _) = self.screen.to_layout(x, 0);
        let Some(drag) = self.option_page.as_ref().and_then(|page| page.drag) else {
            return Action::Stay;
        };
        // `FUN_1000bd20` asks host `+0xc8` whether the pointer moved at all and
        // returns when it has not, so a still pointer writes nothing.
        if x == drag.from {
            return Action::Stay;
        }
        let (Some(knob_x), Some(channel)) = (
            self.knob_x(drag.slider),
            option_pages::SLIDER_CHANNELS.get(drag.slider).copied(),
        ) else {
            return Action::Stay;
        };
        let Some(page) = self.option_page.as_ref() else {
            return Action::Stay;
        };
        let (Some(track), Some(knob)) = (page.tracks.get(drag.slider), page.knobs.get(drag.slider))
        else {
            return Action::Stay;
        };
        let moved = option_pages::clamp_knob_x(track, knob, knob_x + (x - drag.from));
        let value = option_pages::slider_value(track, knob, moved);
        if let Some(page) = &mut self.option_page {
            page.drag = Some(Drag { from: x, ..drag });
            if let Some(slot) = page.knob_x.get_mut(drag.slider) {
                *slot = moved;
            }
        }
        if value == self.session.config.fraction(channel) {
            return Action::Stay;
        }
        self.session.config.set_fraction(channel, value);
        self.dirty = true;
        Action::SettingsChanged
    }

    /// The Option page's own hit test.
    ///
    /// The shipped pump falls through to it exactly here: `FUN_10009760` asks
    /// the hit map first and the class's `+0x4c` — `FUN_1000bb10` — only when
    /// the map answers -1, which on a page it always does.
    fn page_hit(&self, x: u32, y: u32) -> Option<usize> {
        let (x, y) = self.screen.to_layout(x, y);
        let (x, y) = (x.max(0.0) as u32, y.max(0.0) as u32);
        // The two pages number their widgets from different places — the
        // Option frame has four widgets ahead of its tabs and the Replay frame
        // three — so each run is scanned by its own module.
        match (&self.option_page, &self.replay_page) {
            (Some(page), _) => option_pages::hit_in(&page.widgets, x, y),
            (_, Some(page)) => replay_pages::hit_in(&page.widgets, x, y),
            _ => None,
        }
    }

    /// Whether each of the twelve slots of the grid page showing has been seen,
    /// which is what `FUN_10028260` fills `+0x4e8` with.
    fn unlocked_slots(&self) -> Vec<bool> {
        (0..replay_pages::THUMBNAILS)
            .map(|slot| {
                replay_pages::scene_of(self.page, slot)
                    .is_some_and(|scene| self.session.scenes.unlocked(scene, &self.session.flags))
            })
            .collect()
    }

    /// Whether the pointer is on a slider's knob, which is the rest of
    /// [`option_pages::enabled`] for the Sound tab's three sliders.
    fn grabbing(&self, widget: usize, x: u32) -> bool {
        let Some(slider) = self.slider_at(widget) else {
            return false;
        };
        let (Some(page), Some(knob_x)) = (self.option_page.as_ref(), self.knob_x(slider)) else {
            return false;
        };
        let (x, _) = self.screen.to_layout(x, 0);
        page.knobs
            .get(slider)
            .is_some_and(|knob| option_pages::grabbed(knob, knob_x, x))
    }

    /// The pointer left the screen, or the window lost focus.
    pub fn point_away(&mut self) -> Action {
        // A drag keeps the selection, the same way it keeps it when the
        // pointer wanders off the track: while `+0x24c` is set
        // `FUN_1000b3d0` goes to the drag and never to the hit test.
        if self.dragging().is_some() {
            return Action::Stay;
        }
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

    /// Keyboard navigation.
    ///
    /// The title is a vertical list and uses `step`; the Option screens are
    /// not, and each tab of each module has a hand-written transition table in
    /// its DLL — see [`options::navigate`] and [`option_pages::navigate`]. A
    /// screen with no transcribed table falls back to walking its widgets in
    /// order, which is at least reachable.
    ///
    /// Both Replay views have tables too, on both modules. The module with its
    /// own per-view hit maps writes them as `FUN_1001e3a0` for the grid and
    /// `FUN_1001e7e0` for the list, dispatched on `+0x2b0` by `FUN_1001e200`;
    /// its grid's sideways guard is off by two in the shipped code and this
    /// engine fixes it, which [`replay::navigate`] carries the evidence for.
    /// The page module's are `FUN_1002abc0` and `FUN_1002b040`, dispatched on
    /// `+0x618` by `FUN_1002a9a0` — see [`replay_pages::navigate`].
    pub fn navigate(&mut self, vfs: &Vfs, dll: &[u8], dir: Dir) -> Result<Action, Error> {
        let next = match self.mode {
            Mode::OPTION if self.option_page.is_some() => Some(option_pages::navigate(
                self.tab,
                self.selection.unwrap_or(3),
                dir,
                self.session.save.trial,
                self.session.som,
            )),
            // A page module's Replay screen is a different recovery, with its
            // own pair of tables.
            Mode::REPLAY if self.replay_page.is_some() => Some(replay_pages::navigate(
                self.view,
                self.selection.unwrap_or(2),
                dir,
            )),
            Mode::REPLAY => {
                let current = self.selection.unwrap_or(2);
                Some(match self.view {
                    replay::View::HScene => replay::navigate(current, dir, self.page),
                    replay::View::PlayData => playdata::navigate(current, dir, self.page),
                })
            }
            Mode::OPTION => Some(options::navigate(
                self.tab,
                self.selection.unwrap_or(3),
                dir,
                self.session.save.trial,
                self.session.som,
            )),
            _ => {
                let delta = match dir {
                    Dir::Up | Dir::Left => -1,
                    Dir::Down | Dir::Right => 1,
                };
                step(self.wrapping_count(), self.selection, delta, |i| {
                    self.enabled(i)
                })
            }
        };
        let Some(index) = next else {
            return Ok(Action::Stay);
        };
        // A page module opens a tab the moment a sideways step lands on its
        // header, rather than waiting for a press: see
        // [`option_pages::opens_tab`].
        if self.mode == Mode::OPTION && self.option_page.is_some() && self.enabled(index) {
            if let Some(next) = option_pages::opens_tab(dir, index).filter(|t| *t != self.tab) {
                self.tab = next;
                self.enter(vfs, dll, Mode::OPTION, self.return_to)?;
                self.selection = Some(index);
                self.refresh();
                return Ok(Action::Opened(Mode::OPTION));
            }
        }
        // Every Replay table turns the page the same way, on the sideways arms
        // only and only when the button is live.
        if self.mode == Mode::REPLAY && self.enabled(index) {
            let opened = match (self.replay_page.is_some(), self.view) {
                (true, view) => replay_pages::opens_page(view, dir, index),
                (false, replay::View::HScene) => replay::opens_page(dir, index),
                (false, replay::View::PlayData) => playdata::opens_page(dir, index),
            };
            if let Some(page) = opened.filter(|page| *page != self.page) {
                self.page = page;
                self.dirty = true;
                // A page module draws the page as a layer of its own, so the
                // art has to be rebuilt for the page turned to; the screen
                // with its own hit maps redraws from the atlas it already has.
                if self.replay_page.is_some() {
                    self.load_replay_page(vfs, dll);
                }
            }
        }
        if self.selection == Some(index) {
            return Ok(Action::Stay);
        }
        self.selection = Some(index);
        self.refresh();
        Ok(Action::Sound(match dir {
            Dir::Up | Dir::Left => SystemSe::Up,
            Dir::Down | Dir::Right => SystemSe::Down,
        }))
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
    /// Every screen's table is the DLL's own dispatch: the title's here,
    /// the Option screen's in [`options::action`] (`FUN_10007e80` and the three
    /// it switches to), the replay grid's in [`replay::hscene_action`]
    /// (`FUN_1001de10`) and the replay popup's in [`replay::popup_action`]
    /// (`FUN_10019610`).
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
            Mode::OPTION => self.confirm_option(vfs, dll, widget),
            Mode::REPLAY if self.replay_page.is_some() => {
                self.confirm_replay_page(vfs, dll, widget)
            }
            Mode::REPLAY if self.view == replay::View::HScene => {
                self.confirm_replay(vfs, dll, widget)
            }
            Mode::REPLAY => self.confirm_playdata(vfs, dll, widget),
            Mode::REPLAY_POPUP => Ok(self.confirm_replay_popup(widget)),
            // `FUN_10014910` answers false for every widget while `+0x98` is
            // set, so a save that has been taken and not yet written swallows
            // clicks rather than stacking another one behind it.
            Mode::SAVELOAD if self.pending_save.is_some() => Ok(Action::Stay),
            Mode::SAVELOAD => self.confirm_saveload(vfs, dll, widget),
            Mode::ROUTEMAP => self.confirm_routemap(vfs, dll, widget),
            Mode::DRESS_SELECT => self.confirm_dress(vfs, dll, widget),
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

    /// The dress-select screen's dispatch, from `FUN_1000ded0`.
    ///
    /// Committing to a dress tells the host at once — host `+0x48`, which
    /// [`Session::dress`] is this engine's side of — and then swaps the hit map
    /// for the popup's, which is what `FUN_1000e440` phase 1 does once the
    /// slide has run. Answering the popup yes sets the module's `+0xfc`, and
    /// `getNextMode` case 9 reads that as mode 1: leave the menus and play.
    /// Answering it no puts the first map back.
    fn confirm_dress(&mut self, vfs: &Vfs, dll: &[u8], widget: usize) -> Result<Action, Error> {
        match dress::action(self.dress_phase, widget) {
            dress::Act::None => Ok(Action::Stay),
            dress::Act::Commit(chosen) => {
                self.session.dress = Some(dress::host_value(chosen));
                self.set_dress_phase(vfs, dll, dress::Phase::Confirming { chosen })
            }
            // `FUN_1000ded0` returns 0 here, which the update pump reads as
            // "this screen is finished"; the mode it leaves for is
            // `getNextMode`'s, and for mode 1 that is playback.
            dress::Act::Accept => Ok(Action::Play),
            dress::Act::Cancel => self.set_dress_phase(vfs, dll, dress::Phase::Choosing),
        }
    }

    /// Swaps the dress-select screen's hit map, keeping the mode.
    ///
    /// A map that will not load leaves the one that is up, the same answer
    /// [`Action::Unavailable`] gives for a whole screen.
    fn set_dress_phase(
        &mut self,
        vfs: &Vfs,
        dll: &[u8],
        phase: dress::Phase,
    ) -> Result<Action, Error> {
        let was = self.dress_phase;
        self.dress_phase = phase;
        let (mode, back) = (self.mode, self.return_to);
        if let Err(err) = self.enter(vfs, dll, mode, back) {
            log::warn!("dress select: cannot load the other hit map: {err}");
            self.dress_phase = was;
            return Ok(Action::Stay);
        }
        Ok(Action::Sound(SystemSe::Click))
    }

    /// The save/load screen's dispatch, from `FUN_10014990`.
    ///
    /// Picking a row names a slot; what happens to it is the engine's, because
    /// the DLL only asks the host — host `+0x48` for a load and `+0xa0` for a
    /// save. Picking an empty row on the Load screen does nothing at all,
    /// which is `FUN_10011d50` refusing before it calls.
    fn confirm_saveload(&mut self, vfs: &Vfs, dll: &[u8], widget: usize) -> Result<Action, Error> {
        match saveload::action(self.kind, widget) {
            saveload::Act::Row(row) => {
                let slot = saveload::slot_of(self.page, row);
                Ok(match self.kind {
                    Kind::Load if !self.session.slots.filled(slot) => Action::Stay,
                    Kind::Load => Action::Load(slot),
                    Kind::Save => Action::Save(slot),
                })
            }
            saveload::Act::Page(page) => {
                if page == self.page {
                    return Ok(Action::Stay);
                }
                self.page = page;
                self.refresh();
                Ok(Action::Stay)
            }
            saveload::Act::Leave => self.leave(vfs, dll),
            saveload::Act::RouteMap => self.open_routemap(vfs, dll),
            saveload::Act::None => Ok(Action::Stay),
        }
    }

    /// Which episode and page of the chart the route map is showing, and what
    /// that page holds.
    pub fn chart(&self) -> (usize, usize, routemap::Page) {
        (self.episode, self.map_page, self.chart_page())
    }

    /// The page of the chart the route map is showing.
    ///
    /// An episode can be left parked on a page it does not have — see
    /// [`routemap::page_for_episode`] — and the original reads its table with
    /// that index regardless. Here that falls back to the episode's first page,
    /// which is the page whose art the screen will have loaded.
    fn chart_page(&self) -> routemap::Page {
        routemap::page(self.episode, self.map_page)
            .or_else(|| routemap::page(self.episode, 0))
            .unwrap_or(routemap::Page { cells: 0, base: 0 })
    }

    /// Which cells of the page the player may jump to, from `FUN_1000c740`.
    ///
    /// The save's own store answers it, and only while a playthrough is
    /// running: the module computes this half at all only when its `+0x4ec` is
    /// set, and that member is 1 from `setSystemInit` — the driver over
    /// playback — and 0 from `_SystemInit@8`, the title-rooted shell.
    pub fn pickable(&self) -> Vec<bool> {
        let page = self.chart_page();
        let Some(run) = self
            .session
            .run
            .as_ref()
            .filter(|_| self.entry == Entry::Playback)
        else {
            return vec![false; page.cells];
        };
        (0..page.cells)
            .map(|cell| {
                run.flag(&routemap::story_flag(routemap::story(
                    self.episode,
                    page.base,
                    cell,
                )))
            })
            .collect()
    }

    /// Which cells of the page are drawn at all, from the other half of
    /// `FUN_1000c740`: the global store, so a story point the player has ever
    /// reached is charted whether or not this run has passed it.
    pub fn charted(&self) -> Vec<bool> {
        let page = self.chart_page();
        (0..page.cells)
            .map(|cell| {
                self.session
                    .flags
                    .flag(&routemap::story_flag(routemap::story(
                        self.episode,
                        page.base,
                        cell,
                    )))
            })
            .collect()
    }

    /// Which cell of the page the player is standing on, from `_CheckScript@8`
    /// by way of [`routemap::Standing::marker`].
    ///
    /// `None` on every page but the one the player's own story point is on, and
    /// on every page at all when the chart was opened from the title.
    pub fn marker(&self) -> Option<usize> {
        let page = self.chart_page();
        self.standing(|standing| standing.marker(self.episode, self.map_page, page))
            .flatten()
    }

    /// The route map's dispatch, from `FUN_1000e8b0`.
    ///
    /// Picking a cell leaves the menus the way the Load screen's rows do, with
    /// a story number in place of a slot. Everything else moves the chart and
    /// reloads its art, which is what `FUN_1000d890` does at the end of each
    /// arm.
    fn confirm_routemap(&mut self, vfs: &Vfs, dll: &[u8], widget: usize) -> Result<Action, Error> {
        let page = self.chart_page();
        let (episode, map_page) = (self.episode, self.map_page);
        let (episode, map_page) = match routemap::action(page.cells, widget) {
            // Not a Close like every other screen's. `FUN_1000e8b0` asks the
            // save/load module whether *it* opened the map — `FUN_10001520`
            // reads its `+0x1fc`, which `FUN_10014990` raises on the way in —
            // and answers `+0x4c(5)`, the Load screen, when it did. Its other
            // branch is `+0x4c(0)`, leave the menus, and **that branch cannot
            // be taken in the retail build**: the map has no other way in, so
            // the member is always set while it is up.
            routemap::Act::Close => {
                self.kind = Kind::Load;
                return self.advance(vfs, dll, Mode::SAVELOAD);
            }
            routemap::Act::Cell(cell) => {
                return Ok(Action::LoadStory(routemap::story(episode, page.base, cell)))
            }
            routemap::Act::Episode(tab) if tab != episode => {
                (tab, routemap::page_for_episode(episode, tab, map_page))
            }
            routemap::Act::PrevEpisode => routemap::step_back(episode, map_page),
            routemap::Act::NextEpisode => routemap::step_on(episode, map_page),
            routemap::Act::PrevPage => (episode, map_page.saturating_sub(1)),
            routemap::Act::NextPage => (episode, map_page + 1),
            _ => return Ok(Action::Stay),
        };
        self.show_chart(vfs, dll, episode, map_page)
    }

    /// Moves the chart and loads the art the new page names.
    ///
    /// A page whose art will not load leaves the chart where it was rather
    /// than on a screen it cannot draw, which is the rule every screen here
    /// follows.
    fn show_chart(
        &mut self,
        vfs: &Vfs,
        dll: &[u8],
        episode: usize,
        map_page: usize,
    ) -> Result<Action, Error> {
        let (was_episode, was_page) = (self.episode, self.map_page);
        self.episode = episode;
        self.map_page = map_page;
        let back = self.return_to;
        match self.enter(vfs, dll, Mode::ROUTEMAP, back) {
            Ok(()) => Ok(Action::Opened(Mode::ROUTEMAP)),
            Err(err) => {
                log::warn!(
                    "no route map art for episode {} page {}: {err}",
                    episode + 1,
                    map_page + 1
                );
                self.episode = was_episode;
                self.map_page = was_page;
                Ok(Action::Unavailable(Mode::ROUTEMAP))
            }
        }
    }

    /// Opens the route map, as the Load screen's widget `0x15` does.
    ///
    /// `FUN_1000e060` puts the chart on the player's own episode and page when
    /// the screen was opened from inside a playthrough, and leaves it on the
    /// first of each when it was not — the constructor's zeroes, which the
    /// title-rooted screen never overwrites.
    fn open_routemap(&mut self, vfs: &Vfs, dll: &[u8]) -> Result<Action, Error> {
        let (episode, page) = self
            .standing(|standing| standing.opened_at())
            .unwrap_or((0, 0));
        self.show_chart(vfs, dll, episode, page)
    }

    /// Where the playthrough stands, for the two `RouteProcSDHQ.dll` functions
    /// the route map asks — `_CheckScript@8` and `_GetRouteMapPage@8`.
    ///
    /// `None` unless a playthrough is running, which is the same test
    /// [`Menu::pickable`] makes and the same one the module's `+0x4ec` is: the
    /// chart reached from the title has no player to place.
    ///
    /// The `Standing` borrows the run, so it is handed to a closure rather than
    /// returned.
    fn standing<T>(&self, with: impl FnOnce(&routemap::Standing) -> T) -> Option<T> {
        let run = self
            .session
            .run
            .as_ref()
            .filter(|_| self.entry == Entry::Playback)?;
        let passed = |story: u32| run.flag(&routemap::story_flag(story));
        Some(with(&routemap::Standing {
            route: run.int("ROUTE"),
            scene: run.int("SCENE"),
            end_clear: run.flag("EndClear"),
            passed: &passed,
        }))
    }

    /// Opens the save/load screen for one of its two jobs.
    ///
    /// `setSystemInit` code 4 is the save screen and code 5 the load screen;
    /// both are this module with `+0x94` poked first, so the kind has to be
    /// set before the art is chosen.
    pub fn open_saveload(&mut self, vfs: &Vfs, dll: &[u8], kind: Kind) -> Result<Action, Error> {
        self.kind = kind;
        // The page is *not* reset. `FUN_100135c0`, the module's open, never
        // writes `+0x1f0`; the only zero it ever gets is from `FUN_100111f0`,
        // which one static-init thunk calls at load time. So the page the
        // player left the list on is the page they come back to, whether they
        // come back from the route map, from the other job, or from the title.
        self.advance(vfs, dll, Mode::SAVELOAD)
    }

    /// Takes a save the player has asked for, without committing it.
    ///
    /// This is the module's `+0x98`. Both paths into it are
    /// `FUN_10014990`'s save arm: with `FILMENGINE.INI [TextInput]` set the
    /// screen hands the host a default comment through `+0xdc`, the executable
    /// runs its dialog, and the dialog's OK calls back into `_CommentSet@4`,
    /// which stores the text at `+0x200` and raises `+0x98`. With the key clear
    /// there is no dialog at all: `+0x98` goes up straight away and
    /// `FUN_10011c30` writes `L""` as the comment.
    ///
    /// Cancelling the dialog calls nothing, so nothing is taken.
    pub fn begin_save(&mut self, slot: u32, comment: String) {
        self.pending_save = Some((slot, comment));
        self.refresh();
    }

    /// The save the screen is holding, if any.
    pub fn pending_save(&self) -> Option<(u32, &str)> {
        self.pending_save
            .as_ref()
            .map(|(slot, comment)| (*slot, comment.as_str()))
    }

    /// Commits the save the screen was holding: `FUN_10014c90`'s other arm.
    ///
    /// `FUN_10011c30` asks the host to write the slot, `FUN_10011ec0`
    /// re-rasterises the page, and `+0x98` comes down — all without leaving the
    /// screen. `line` is what the slot now reads as, so the row the player just
    /// wrote shows its new timestamp immediately instead of after a re-entry.
    pub fn finish_save(&mut self, slot: u32, line: saveload::Line) {
        self.pending_save = None;
        self.session.slots.insert(slot, line);
        self.refresh();
    }

    /// Which job the save/load screen is doing.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The page of ten slots the save/load screen is showing.
    pub fn page(&self) -> usize {
        self.page
    }

    /// The Option screen's dispatch.
    ///
    /// Every setting is written through to the config as it changes, which is
    /// what the DLL does; only the close button flushes to disk.
    fn confirm_option(&mut self, vfs: &Vfs, dll: &[u8], widget: usize) -> Result<Action, Error> {
        let act = match self.option_page {
            Some(_) => option_pages::action(self.tab, widget, self.session.display),
            None => options::action(self.tab, widget, self.session.display),
        };
        match act {
            options::Act::Tab(next) => {
                if next == self.tab {
                    return Ok(Action::Stay);
                }
                self.tab = next;
                self.enter(vfs, dll, Mode::OPTION, self.return_to)?;
                Ok(Action::Opened(Mode::OPTION))
            }
            options::Act::Close => Ok(Action::SettingsSaved),
            options::Act::Display(request) => Ok(Action::Display(request)),
            // Whether a port is really there is the engine's to answer, never
            // the screen's: the DLL asks its own serial object and reads the
            // result back through `_GetSomFlag@0`. So these four go out as
            // requests and the engine hands the answer back to
            // [`Menu::set_som`], which is what reloads the art the answer
            // chooses.
            options::Act::SomDetect => {
                options::apply(&mut self.session.config, act);
                Ok(Action::Som(options::SomRequest::Detect))
            }
            options::Act::SomRelease => {
                options::apply(&mut self.session.config, act);
                Ok(Action::Som(options::SomRequest::Release))
            }
            options::Act::SomPort(port) => Ok(Action::Som(options::SomRequest::Port(port))),
            options::Act::SomTest(on) => Ok(Action::Som(options::SomRequest::Test(on))),
            options::Act::None => Ok(Action::Stay),
            _ => {
                options::apply(&mut self.session.config, act);
                self.refresh();
                Ok(Action::SettingsChanged)
            }
        }
    }

    /// Records what the engine made of an [`Action::Som`].
    ///
    /// `FUN_100073a0` picks the tab's background from whether a port is held,
    /// and each port button's highlight from which one it is, so the answer
    /// changes what is on screen. A change to whether the toy is asked for at
    /// all swaps the base art and needs the screen loading again; anything
    /// else is a repaint.
    pub fn set_som(&mut self, vfs: &Vfs, dll: &[u8], mut som: options::Som) -> Result<(), Error> {
        // A page module keeps one answer where the other keeps two:
        // `FUN_10008760` sets `UseSOM` and "a port is open" together and clears
        // them together, and `FUN_100095c0`'s release zeroes both. So asking
        // for the toy and finding nothing leaves the tab off rather than on,
        // and the flush writes that member back to `UseSOM`.
        //
        // What the shipped screen also does with a detect that finds nothing is
        // set `+0x228`; what reads that member is **not recovered**, so the
        // message it stands for is not raised here.
        if self.option_page.is_some() {
            som.enabled = som.attached;
            self.session
                .config
                .set_flag(crate::install::config::Flag::UseSom, som.enabled);
        }
        let was = self.session.som;
        self.session.som = som;
        if was.enabled != som.enabled && self.mode == Mode::OPTION {
            self.enter(vfs, dll, Mode::OPTION, self.return_to)?;
        } else {
            self.refresh();
        }
        Ok(())
    }

    /// What a click does on a Replay view that is a page layer.
    ///
    /// [`replay_pages::action`] is the shipped dispatch, and both of its ends
    /// reach the same two places the screen with its own hit maps reaches: a
    /// thumbnail through [`Menu::play_scene`] and a row through
    /// [`Menu::play_recorded`].
    fn confirm_replay_page(
        &mut self,
        vfs: &Vfs,
        dll: &[u8],
        widget: usize,
    ) -> Result<Action, Error> {
        match replay_pages::action(self.view, widget, self.page) {
            replay_pages::Act::View(next) => {
                if next == self.view {
                    return Ok(Action::Stay);
                }
                self.view = next;
                self.page = 0;
                self.reopen(vfs, dll, Mode::REPLAY)
            }
            replay_pages::Act::Back => self.advance(vfs, dll, Mode::CONFIRM),
            replay_pages::Act::Page(page) => {
                if page == self.page || page >= replay_pages::pages(self.view) {
                    return Ok(Action::Stay);
                }
                self.page = page;
                self.selection = None;
                self.load_replay_page(vfs, dll);
                self.refresh();
                Ok(Action::Sound(SystemSe::Click))
            }
            replay_pages::Act::Scene(scene) => self.play_scene(vfs, dll, scene),
            replay_pages::Act::Row(row) => Ok(self.play_recorded(row)),
            replay_pages::Act::None => Ok(Action::Stay),
        }
    }

    /// What clicking an unlocked h-scene thumbnail does, on either Replay grid.
    ///
    /// A scene with versions raises the popup and plays nothing until one is
    /// picked; every other scene starts its own first script. Which of the two
    /// is [`replay::Scene::asks`] — the scene's own entry in the script run
    /// being a hole — and both modules' dispatches agree on it: `FUN_1001de10`
    /// and `FUN_1002a4a0` special-case exactly the scenes that have versions.
    fn play_scene(&mut self, vfs: &Vfs, dll: &[u8], index: usize) -> Result<Action, Error> {
        let Some(scene) = self.session.scenes.get(index) else {
            return Ok(Action::Stay);
        };
        if scene.asks() {
            self.asked = Some(index);
            return self.advance(vfs, dll, Mode::REPLAY_POPUP);
        }
        Ok(if scene.scripts.is_empty() {
            Action::Stay
        } else {
            Action::PlayReplay(scene.run())
        })
    }

    /// What clicking a play-data row does, on either Replay list: load the save
    /// entry the page and the row name, and nothing at all when it is empty.
    ///
    /// The shipped dispatches test the host's own answer for the row —
    /// `+0x17c + row * 4` in `SysMenuSDHQ.dll`, `+0x1cc + row * 4` in
    /// `SysMenuSD.dll` — before they call, which is this `filled`.
    fn play_recorded(&self, row: usize) -> Action {
        let slot = saveload::slot_of(self.page, row);
        if self.session.slots.filled(slot) {
            Action::PlayRecorded(slot)
        } else {
            Action::Stay
        }
    }

    fn confirm_replay(&mut self, vfs: &Vfs, dll: &[u8], widget: usize) -> Result<Action, Error> {
        match replay::hscene_action(&self.session.scenes, self.page, widget) {
            replay::Act::View(next) => {
                if next == self.view {
                    return Ok(Action::Stay);
                }
                self.view = next;
                self.page = 0;
                self.reopen(vfs, dll, Mode::REPLAY)
            }
            replay::Act::Back => self.advance(vfs, dll, Mode::CONFIRM),
            replay::Act::Page(page) => {
                if page == self.page {
                    return Ok(Action::Stay);
                }
                self.page = page;
                self.selection = None;
                self.refresh();
                Ok(Action::Sound(SystemSe::Click))
            }
            replay::Act::Play { scene } | replay::Act::Ask { scene } => {
                self.play_scene(vfs, dll, scene)
            }
            replay::Act::None => Ok(Action::Stay),
        }
    }

    /// The play-data list's dispatch, from `FUN_1001dfe0`.
    ///
    /// A row whose slot has no file does nothing, which is the shipped
    /// `+0x17c + row * 4` test; every widget stays live either way.
    fn confirm_playdata(&mut self, vfs: &Vfs, dll: &[u8], widget: usize) -> Result<Action, Error> {
        match playdata::action(widget) {
            playdata::Act::View(next) => {
                if next == self.view {
                    return Ok(Action::Stay);
                }
                self.view = next;
                self.page = 0;
                self.reopen(vfs, dll, Mode::REPLAY)
            }
            playdata::Act::Back => self.advance(vfs, dll, Mode::CONFIRM),
            playdata::Act::Page(page) => {
                if page == self.page {
                    return Ok(Action::Stay);
                }
                self.page = page;
                self.selection = None;
                self.refresh();
                Ok(Action::Sound(SystemSe::Click))
            }
            playdata::Act::Row(row) => Ok(self.play_recorded(row)),
            playdata::Act::None => Ok(Action::Stay),
        }
    }

    /// The replay popup's dispatch: pick a version and play it.
    fn confirm_replay_popup(&mut self, widget: usize) -> Action {
        let Some(scene) = self.asked.and_then(|s| self.session.scenes.get(s)) else {
            return Action::Stay;
        };
        match replay::popup_action(scene, widget) {
            Some(run) => Action::PlayReplay(run),
            None => Action::Stay,
        }
    }

    /// Backs out of the current screen.
    ///
    /// From the title, every screen but the title returns to the confirm popup,
    /// which asks whether to go back to the title; the title's own cancel asks
    /// whether to quit. The two popups differ only in their background art,
    /// chosen from the mode the popup remembers.
    ///
    /// Over playback there is no popup on this path. The modules the bar can
    /// open offer exactly one way out — host `+0x4c(0)`, the Close button --
    /// and `FUN_00425550` takes that straight back to the playback tick, so
    /// backing out over playback is the same thing as closing. **What the
    /// over-playback driver would do with a popup answer is not recovered**:
    /// `_SetReMenu@4` (`FUN_10001500`) stores the bar's own module code into
    /// the popup's `+0xa4`, and `FUN_1000a8f0` answers `+0x4c(+0xa4)` on the
    /// negative button and `+0x4c(1)` or `+0x4c(9)` on the positive one — but
    /// 1 and 9 are the popup module's own `setSystemInit` codes, so what the
    /// driver makes of that has not been established and is not guessed at
    /// here.
    ///
    /// The route map is the one screen whose way out is neither of those. It is
    /// opened from the save/load screen by `+0x4c(6)` (`FUN_10014990` widget
    /// 0x15), and `_getNextMode@8` case 6 answers mode 3 — the save/load
    /// screen — never the title. So it goes back to where it was opened from
    /// under either driver. (Case 6 reaches that arm when the module's `+0x74`
    /// is *clear*, which reads backwards for a leave flag; **which member the
    /// route map's own back button sets is not recovered**, and this engine
    /// does not depend on it.)
    pub fn cancel(&mut self, vfs: &Vfs, dll: &[u8]) -> Result<Action, Error> {
        match backing_out(self.entry, self.mode, self.return_to) {
            Leaving::Resume => Ok(Action::Play),
            Leaving::To(mode) => self.advance(vfs, dll, mode),
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

    /// Reloads the screen that is showing at a different art set.
    ///
    /// Every screen's loader picks its `.cmap` from the display mode —
    /// `FUN_10013470` is the save/load screen's copy of the rule — so changing
    /// the mode means reloading whatever is on screen at the new size. A
    /// refusal leaves the old screen up, since a menu that cannot be drawn is
    /// worse than one at the wrong size.
    pub fn set_resolution(
        &mut self,
        vfs: &Vfs,
        dll: &[u8],
        resolution: Resolution,
    ) -> Result<(), Error> {
        if self.resolution == resolution {
            return Ok(());
        }
        let was = self.resolution;
        self.resolution = resolution;
        let (mode, back) = (self.mode, self.return_to);
        if let Err(err) = self.enter(vfs, dll, mode, back) {
            log::warn!("cannot draw the menus at {}: {err}", resolution.name());
            self.resolution = was;
            self.enter(vfs, dll, mode, back)?;
        }
        Ok(())
    }

    /// Which driver is running. See [`Entry`].
    pub fn entry(&self) -> Entry {
        self.entry
    }

    /// Leaves the screen that is showing, which is host `+0x4c(0)`.
    ///
    /// The code 0 is the same call in both drivers and it means the same thing
    /// — "I am done, take me out of here" — but the two drivers take it to
    /// different places, which is the whole of this rule:
    ///
    /// * Over playback, `FUN_00425550` case 6 sees `+0x260 == 0` and falls
    ///   through cases 7 and 8; case 8 sets `+0x220 = 1`, the state
    ///   `FUN_00427300` hands to `FUN_004253f0`, the playback tick. Playback
    ///   resumes where it was paused, so this is [`Action::Play`].
    /// * From the title, the title-rooted shell walks `_getNextMode@8`'s graph
    ///   instead, and every arm of it that is not another screen ends at mode
    ///   2, the title.
    pub fn leave(&mut self, vfs: &Vfs, dll: &[u8]) -> Result<Action, Error> {
        match leaving(self.entry, self.return_to) {
            Leaving::Resume => Ok(Action::Play),
            Leaving::To(mode) => self.advance(vfs, dll, mode),
        }
    }

    fn reopen(&mut self, vfs: &Vfs, dll: &[u8], next: Mode) -> Result<Action, Error> {
        // `return_to` is only ever the popup's memory of where it was raised
        // from, and the one thing that memory decides is which popup this is:
        // the DLL picks `Popup_Title.png` or `Popup_Exit.png` off its own
        // `+0x9c`, which `setSystemInit` pokes with 0 for code 1 and 1 for code
        // 9. Raised from the title it asks about quitting; raised from anywhere
        // else it asks about the title. It is *not* where leaving a screen
        // goes — that is `leave`, and reading this member as the answer
        // to that question is what sent a bar-opened screen to the title.
        let return_to = if next == Mode::CONFIRM {
            self.mode
        } else {
            Mode::TITLE
        };
        let was = (self.mode, self.variant.clone());
        match self.enter(vfs, dll, next, return_to) {
            Ok(()) => Ok(Action::Opened(next)),
            // A screen whose art, hit map or widget table will not read
            // cannot be drawn. Staying put is the right answer: a screen this
            // engine cannot build should leave the player on a working menu,
            // not end the session.
            Err(err) => {
                log::warn!("cannot open menu mode {}: {err}", next.0);
                // Put back whatever the failed load replaced. This cannot fail:
                // it is the screen that was already up a moment ago.
                if self.enter(vfs, dll, was.0, self.return_to).is_err() {
                    log::error!("could not return to menu mode {}", was.0 .0);
                }
                Ok(Action::Unavailable(next))
            }
        }
    }

    /// The variant this screen was opened with, e.g. `Title_AC`.
    pub fn variant(&self) -> &str {
        &self.variant
    }
}

/// The variant a mode's screen loads with, given where the session has got to.
///
/// Only the title picks from save state. The rest pick from the member each
/// module keeps between visits — the Option tab, the Replay screen, and how
/// many versions the scene the popup is asking about has — and a mode with
/// neither keeps the DLL's own default.
fn variant_for(
    session: &Session,
    mode: Mode,
    tab: options::Tab,
    view: replay::View,
    asked: Option<usize>,
    chart: (usize, usize),
) -> String {
    match mode {
        Mode::TITLE => session.save.title_variant().to_string(),
        Mode::OPTION => tab.variant().to_string(),
        Mode::REPLAY => view.variant().to_string(),
        Mode::ROUTEMAP => routemap::variant(chart.0, chart.1),
        Mode::REPLAY_POPUP => {
            let choices = asked
                .and_then(|scene| session.scenes.get(scene))
                .map_or(0, |scene| scene.choices.len());
            replay::popup_variant(choices).to_string()
        }
        _ => mode.default_variant().to_string(),
    }
}

/// Loads the art for one mode.
#[allow(clippy::too_many_arguments)]
fn load_screen(
    vfs: &Vfs,
    dll: &[u8],
    paths: &Paths,
    mode: Mode,
    variant: &str,
    return_to: Mode,
    tab: options::Tab,
    kind: Kind,
    session: &Session,
    dress_phase: dress::Phase,
    resolution: Resolution,
) -> Result<Screen, Error> {
    // The dress-select screen's confirm popup is a second hit map inside mode
    // 9 rather than a mode of its own — `MENU::DressSelect` swaps the one
    // member between `FUN_1000d7f0`'s map and `FUN_1000d8c0`'s — so it is the
    // one screen whose stem does not come from the mode. Its art is named
    // after that stem like any other screen's.
    let popup =
        matches!(dress_phase, dress::Phase::Confirming { .. }) && mode == Mode::DRESS_SELECT;
    if popup {
        let stem = paths
            .dress_select_popup()
            .ok_or_else(|| Error::MissingAsset("the dress-select popup".to_string()))?;
        return Screen::load_with_art(vfs, dll, stem, None, None, resolution);
    }
    let stem_variant = match mode {
        Mode::TITLE => paths.title_stem_variant(variant),
        _ => variant,
    };
    let stem = paths
        .stem(mode.0, stem_variant)
        .ok_or_else(|| Error::MissingAsset(format!("mode {}", mode.0)))?;
    // Two screens name art that is not their stem, and which of them do
    // depends on how the module spells its paths: a module whose cleared title
    // shares the plain title's map names both that title's art and its chip
    // sheet, and a dress-select screen's background is the shared transparent
    // plate. The rest is the same either way.
    let (base, chip) = match mode {
        Mode::TITLE => match paths.title_art(variant) {
            Some((base, chip)) => (Some(base), Some(chip)),
            None => (None, None),
        },
        Mode::DRESS_SELECT => (paths.dress_select_art(), None),
        Mode::OPTION if paths.option_tabs_have_own_map() => {
            (options::base_art(tab, session.som), None)
        }
        Mode::OPTION => (None, None),
        _ => (base_art(mode, return_to, kind), None),
    };
    Screen::load_with_art(vfs, dll, &stem, base, chip, resolution)
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
fn base_art(mode: Mode, return_to: Mode, kind: Kind) -> Option<&'static str> {
    match mode {
        Mode::CONFIRM if return_to == Mode::TITLE => Some("System/Exit/Popup_Exit.png"),
        Mode::CONFIRM => Some("System/Exit/Popup_Title.png"),
        // One module, two jobs, chosen by `+0x94`. `setSystemInit` sets it:
        // code 4 opens it to save and code 5 to load.
        Mode::SAVELOAD => Some(match kind {
            Kind::Load => "System/SaveLoad/Load.png",
            Kind::Save => "System/SaveLoad/Save.png",
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {

    // Where every screen goes when it is left, from both entries. These are
    // the cases nothing covered while the menus were being written, which is
    // why a screen the control bar opened closed onto the title screen.

    /// Close on a bar-opened screen resumes playback. This is the bug.
    #[test]
    fn closing_a_bar_opened_screen_resumes_playback() {
        for mode in [Mode::SAVELOAD, Mode::OPTION, Mode::ROUTEMAP] {
            assert_eq!(
                leaving(Entry::Playback, mode),
                Leaving::Resume,
                "mode {} opened over playback must resume, not leave playback",
                mode.0
            );
        }
    }

    /// ...and Close from the title still goes where it always did.
    /// The title's `REPLAY` caption is a sprite in both states on the module
    /// that gives it two alternates over one rectangle, and only in its dead
    /// state on the module whose base art carries the live one. The shapes are
    /// the two shipped tables: `SysMenuSD.dll` puts both of its at (637, 301),
    /// `SysMenuSDHQ.dll`'s second alternate is a row 50 pixels higher.
    #[test]
    fn a_title_that_draws_both_replay_captions_holds_them_at_one_rectangle() {
        let at = |x: u32, y: u32| days_ui::atlas::Widget {
            dst: days_ui::cmap::Rect {
                x,
                y,
                width: 164,
                height: 28,
            },
            src_x: 0,
            src_y: 0,
        };
        let sd = [at(637, 301), at(637, 301)];
        let hq = [at(348, 353), at(348, 303)];
        assert_eq!(title_replay_caption(&sd, TITLE_REPLAY), Some(true));
        assert_eq!(title_replay_caption(&hq, TITLE_REPLAY), Some(false));
        assert_eq!(title_replay_caption(&hq[..1], TITLE_REPLAY), Some(false));
        assert_eq!(title_replay_caption(&[], TITLE_REPLAY), None);
    }

    #[test]
    fn closing_a_title_rooted_screen_goes_back_to_the_title() {
        assert_eq!(leaving(Entry::Title, Mode::TITLE), Leaving::To(Mode::TITLE));
    }

    /// Backing out over playback is Close, because `+0x4c(0)` is the only exit
    /// the modules the bar can open offer.
    #[test]
    fn backing_out_over_playback_resumes_rather_than_raising_the_popup() {
        for mode in [Mode::SAVELOAD, Mode::OPTION] {
            assert_eq!(
                backing_out(Entry::Playback, mode, Mode::TITLE),
                Leaving::Resume,
                "mode {} must not raise the title popup over playback",
                mode.0
            );
        }
    }

    /// Backing out of a title-rooted screen still asks the popup first.
    #[test]
    fn backing_out_from_the_title_raises_the_popup() {
        assert_eq!(
            backing_out(Entry::Title, Mode::SAVELOAD, Mode::TITLE),
            Leaving::To(Mode::CONFIRM)
        );
        assert_eq!(
            backing_out(Entry::Title, Mode::REPLAY, Mode::TITLE),
            Leaving::To(Mode::CONFIRM)
        );
    }

    /// The three screens opened from another screen answer for themselves,
    /// whichever driver is running.
    #[test]
    fn a_screen_opened_from_another_screen_goes_back_to_it() {
        for entry in [Entry::Title, Entry::Playback] {
            assert_eq!(
                backing_out(entry, Mode::SOM_CONFIG, Mode::TITLE),
                Leaving::To(Mode::OPTION)
            );
            assert_eq!(
                backing_out(entry, Mode::REPLAY_POPUP, Mode::TITLE),
                Leaving::To(Mode::REPLAY)
            );
            // `_getNextMode@8` case 6 answers mode 3, never the title.
            assert_eq!(
                backing_out(entry, Mode::ROUTEMAP, Mode::TITLE),
                Leaving::To(Mode::SAVELOAD)
            );
        }
    }

    /// The popup goes back to whatever raised it, under either driver.
    #[test]
    fn the_popup_returns_to_what_raised_it() {
        for entry in [Entry::Title, Entry::Playback] {
            assert_eq!(
                backing_out(entry, Mode::CONFIRM, Mode::REPLAY),
                Leaving::To(Mode::REPLAY)
            );
        }
    }

    /// The title driver is the default, so nothing that forgets to say which
    /// entry it is silently becomes a bar entry.
    #[test]
    fn the_default_entry_is_the_title() {
        assert_eq!(Entry::default(), Entry::Title);
    }
    use super::*;

    /// Host slot `+0x50`'s switch reads `this + 0x5a4 + 0x1c * index`, and
    /// `FUN_00422170` writes the INI's keys to the same run in declaration
    /// order — so the index is the key's position in `FILMENGINE.INI`.
    #[test]
    fn the_system_sound_index_is_the_ini_order() {
        let keys: Vec<&str> = (0..7)
            .map(|i| SystemSe::from_index(i).expect("seven arms").key())
            .collect();
        assert_eq!(
            keys,
            ["SeCancel", "SeSelect", "SeClick", "SeUp", "SeDown", "SeView", "SeOpen"]
        );
        for se in SystemSe::ALL {
            assert_eq!(SystemSe::from_index(se.index()), Some(se));
        }
        // The switch has no default arm, so there is no eighth sound.
        assert_eq!(SystemSe::from_index(7), None);
    }

    /// The three uses recovered from call sites, as a guard on the numbering:
    /// the control bar plays 2 on a live click and the choice box 1 on a pick
    /// and 0 on a decline.
    #[test]
    fn the_recovered_call_sites_line_up_with_the_names() {
        assert_eq!(SystemSe::Cancel.index(), 0);
        assert_eq!(SystemSe::Select.index(), 1);
        assert_eq!(SystemSe::Click.index(), 2);
    }

    /// The variant each module opens with is a member it zeroes, not a default
    /// this engine chose, so it is pinned against the spelling that reads it.
    /// Which stem that variant then names is [`crate::ui::paths`]'s question.
    #[test]
    fn every_modes_default_variant_is_the_one_its_module_opens_with() {
        for (mode, variant) in [
            (Mode::SAVELOAD, ""),
            (Mode::OPTION, "Def"),
            (Mode::REPLAY, "HScene"),
            (Mode::ROUTEMAP, "01"),
            (Mode::SOM_CONFIG, ""),
            (Mode::REPLAY_POPUP, "2"),
            (Mode::CONFIRM, ""),
        ] {
            assert_eq!(mode.default_variant(), variant, "mode {}", mode.0);
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

    /// Each module remembers where it was between visits, so reopening a mode
    /// has to come back to the tab or page the player left it on.
    #[test]
    fn a_screen_reopens_on_the_variant_the_session_left_it_on() {
        let session = Session::empty(SaveState {
            cleared_first: true,
            ..SaveState::default()
        });
        let def = options::Tab::DEFAULT;
        let hscene = replay::View::DEFAULT;

        assert_eq!(
            variant_for(&session, Mode::TITLE, def, hscene, None, (0, 0)),
            "Title_Clear",
            "the title is the one that picks from save state"
        );
        assert_eq!(
            variant_for(&session, Mode::OPTION, def, hscene, None, (0, 0)),
            "Def"
        );
        assert_eq!(
            variant_for(
                &session,
                Mode::OPTION,
                options::Tab::SomCon,
                hscene,
                None,
                (0, 0)
            ),
            "SomCon"
        );
        assert_eq!(
            variant_for(&session, Mode::REPLAY, def, hscene, None, (0, 0)),
            "HScene"
        );
        assert_eq!(
            variant_for(
                &session,
                Mode::REPLAY,
                def,
                replay::View::PlayData,
                None,
                (0, 0)
            ),
            "PlayData"
        );
        // A mode with no remembered state keeps the DLL's own default.
        assert_eq!(
            variant_for(&session, Mode::ROUTEMAP, def, hscene, None, (0, 0)),
            "01"
        );
    }

    /// The popup's art follows how many versions the scene it was raised for
    /// has, which is what `+0xc8` is set from.
    #[test]
    fn the_replay_popup_sizes_itself_to_the_scene_that_raised_it() {
        let two = replay::Scene {
            flag: "REP04_C1_A00".to_string(),
            scripts: Vec::new(),
            choices: (0..2)
                .map(|k| replay::Choice {
                    flag: format!("REP04_C1_A00{}", (b'A' + k) as char),
                    scripts: vec!["04/04-C1-A00".to_string()],
                    branch: None,
                })
                .collect(),
            branch: None,
        };
        let mut four = two.clone();
        four.choices.extend(two.choices.clone());

        let session = Session {
            scenes: Scenes::from_scenes(vec![two, four]),
            ..Session::empty(SaveState::default())
        };
        let def = options::Tab::DEFAULT;
        let hscene = replay::View::DEFAULT;
        assert_eq!(
            variant_for(&session, Mode::REPLAY_POPUP, def, hscene, Some(0), (0, 0)),
            "2"
        );
        assert_eq!(
            variant_for(&session, Mode::REPLAY_POPUP, def, hscene, Some(1), (0, 0)),
            "4"
        );
        // With nothing asked, the module's own zeroed member picks the small one.
        assert_eq!(
            variant_for(&session, Mode::REPLAY_POPUP, def, hscene, None, (0, 0)),
            "2"
        );
    }

    #[test]
    fn the_popup_asks_a_different_question_depending_on_where_it_opened() {
        assert_eq!(
            base_art(Mode::CONFIRM, Mode::TITLE, Kind::Load),
            Some("System/Exit/Popup_Exit.png")
        );
        assert_eq!(
            base_art(Mode::CONFIRM, Mode::OPTION, Kind::Load),
            Some("System/Exit/Popup_Title.png")
        );
    }
}
