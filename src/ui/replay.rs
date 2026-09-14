//! The Replay screen: which scenes exist, which are unlocked, and what plays.
//!
//! # One module, two screens
//!
//! `MENU::SceneView` is a single class switching on a member at `+0x2b0`:
//!
//! ```text
//! 0  Replay_HScene     19 widgets   a grid of scene thumbnails
//! 1  Replay_PlayData   33 widgets   the play-data list
//! ```
//!
//! This module is the grid. The list is [`crate::ui::playdata`], which is the
//! save/load screen's rows over again — see there.
//!
//! Widgets 0 and 1 are the two tab headers and widget 2 is the back button on
//! both. On the thumbnail screen widgets 3 to 6 are the four page buttons and
//! widgets 7 to 18 are the twelve thumbnails, which is [`HSCENE_PER_PAGE`] ×
//! [`HSCENE_PAGES`] = 48 slots for 41 scenes. `FUN_1001de10` is that dispatch
//! and `FUN_1001dd40` its enablement.
//!
//! `SysMenuSD.dll` lays the same screen out differently. `FUN_1002a4a0` makes
//! widget 3 a previous-page arrow, widget 4 a next-page arrow, widgets 5 to 7
//! three page buttons and widgets 8 to 0x13 the twelve thumbnails, and clamps
//! the page to 0..=2 — 36 slots for its 36 scenes, with no short last page.
//! That module ships no hit map for the grid, so its widgets are rectangles in
//! a record table instead; [`crate::ui::replay_pages`] is that half of the
//! screen and carries the dispatch for it. [`hscene_action`] and
//! [`hscene_enabled`] below are the School Days HQ layout only. The scene table
//! underneath is the same recovery for both.
//!
//! # The scene table is the user's, not ours
//!
//! A module's scenes are three tables: a run of pointers to the save-flag name
//! of each scene, a run of pointers to each scene's list of scripts, and a
//! shorter run of the flag names of the versions that the scenes which ask a
//! question offer. `SysMenuSDHQ.dll` has forty-one scenes and eight versions;
//! `SysMenuSD.dll` has thirty-six and fourteen. None of it is embedded here.
//! [`Scenes::recover`] finds the runs **by content** in the player's own DLL,
//! the way [`days_ui::atlas`] finds the widget tables — see
//! [`Scenes::recover`] for how each run is identified and checked.
//!
//! The two runs are indexed by the same scene number, from their own bases.
//! `FUN_1002ddf0` registers every name the module owns with the host and is the
//! plainest statement of it: one loop counter, `0` to `0x24`, feeds
//! `PTR_u_REP02_28_A20_10057930[scene]` for the flag and
//! `PTR_PTR_10057868[scene]` for the scripts. `FUN_1001de10` and
//! `FUN_1002a4a0` — the click dispatches — index the script table the same way,
//! from the same base. So scene zero is the **first** entry of each run, and
//! the version lists are what follows the scenes in the script run.
//!
//! # What a click plays
//!
//! `FUN_1001de10` turns a thumbnail into a scene index — `page * 12 + widget -
//! 7` — and hands the scene's **first** script to the host. The rest of the
//! list is the steps after it — up to nine for a scene and sixteen for a
//! version of one — and the engine asks the module for each in turn through
//! `FUN_1001f0d0`, which walks the scene's branch table by the choice the
//! player last made during playback.
//!
//! # How a scene walks
//!
//! `FUN_1001f270` starts a scene at step 0 (`+0x2ac`), and each time a script
//! ends `FUN_1001f0d0` is asked for the next one. It calls `FUN_1001ee20` for
//! the next **step index** and looks the name up in the scene's list, returning
//! an empty string when the index is -1 and when the name it lands on is the
//! list's NULL terminator — either way, the chain ending.
//!
//! `FUN_1001ee20` is where the branch lives:
//!
//! ```text
//! column = host->+0x4()                      // the choice made in playback
//! column = (column == -2) ? 0 : column + 1
//! switch (scene) {
//!   case 1, 6, 7, 10, 11, 12, 16, 25, 28, 29, 36:
//!       next = table[step * 3 + column]       // 0xc bytes a row, three wide
//!   default:
//!       next = step + 1                       // straight down the list
//! }
//! ```
//!
//! A row is three `i32` next-step indices, one per column. Scene 11 has four
//! such tables, one per version, picked by `+0x2b4`. See [`Run::next`] for the
//! walk and [`column`] for where the choice comes from.
//!
//! # Finding the branch tables
//!
//! A branch table is a run of small integers, and the image is full of those:
//! unlike every other table here, it cannot be found by its own contents. What
//! is distinctive is **the code that reads it**. `FUN_1001ee20` is a dense MSVC
//! switch whose arms are all the same five instructions — read `+0x2ac`,
//! multiply by twelve, index by four, load from a fixed address — which is the
//! branch rule itself, written in instructions. [`branch_switches`] scans the
//! image for a switch of that shape and takes the addresses out of its arms.
//!
//! `SysMenuSD.dll` writes the same rule without a switch, because it has only
//! one branching scene to write: `FUN_1002bd00` compares its scene member
//! `+0x610` against `0x15` and, when it matches, reads `+0x614 * 0xc +
//! column * 4` from `0x100579c0` — four rows for scene 21's four scripts, which
//! is the whole of that module's branching. [`branch_switches`] does not see
//! it, since it is a compare and a jump rather than a jump table, so scene 21
//! is walked straight down its list here and the table it should walk instead
//! is **not recovered**.
//!
//! Against the retail DLL exactly one switch matches, and it yields the eleven
//! scenes and scene 11's four version tables that the decompile shows. Each
//! table is then read as one row per script in the scene's list and refused
//! unless every entry is a step that list has — a check all fourteen pass, and
//! one whose lengths land exactly on the next table or the padding before it.
//!
//! # The three scenes that ask first
//!
//! Scenes 11, 22 and 30 do not start playing when clicked. The dispatch
//! special-cases them and raises `Pop_Replay`, whose two or four widgets are
//! versions of the scene, each with its own save flag. Whichever the player
//! picks, `FUN_1001f270` — `FUN_1002c020` in `SysMenuSD.dll` — sets the step
//! index to zero and plays element zero of that version's script list.
//!
//! What that element is, is per-scene data rather than a rule. In
//! `SysMenuSDHQ.dll` every version of a scene names the same script first, so
//! the choice there changes only what comes after it; scene 11's versions then
//! walk different tables and 22's and 30's differ only in the list. In
//! `SysMenuSD.dll` five of the seven behave the same way, and the two the
//! uniform choice reaches — `REP04_S1_B03` and `REP04_YX_A01`, see
//! [`crate::ui::dress`] — name `04/Z4-…` and `04/04-…` respectively, so for
//! those two the version picked does change what starts.
//!
//! The version flags are the scene's flag and one more letter. That is a shape
//! `SysMenuSDHQ.dll`'s own run spells out and `SysMenuSD.dll`'s code builds:
//! `FUN_1002ddf0` formats `L"%s%C"` from the scene's flag and `0x41 + (k != 0)`
//! — `A` for the first version and `B` for the second.

use days_save::FlagStore;

/// Thumbnails on one page of the scene grid: widgets 7 to 18.
pub const HSCENE_PER_PAGE: usize = 12;

/// Page buttons on the scene grid: widgets 3 to 6.
pub const HSCENE_PAGES: usize = 4;

/// First widget of the thumbnail grid.
pub const HSCENE_FIRST_THUMBNAIL: usize = 7;

/// First widget of the page row.
pub const HSCENE_FIRST_PAGE: usize = 3;

/// Which of the module's two screens is showing, as the member it switches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// `Replay_HScene`, the thumbnail grid.
    HScene,
    /// `Replay_PlayData`, the play-data list.
    PlayData,
}

impl View {
    /// What a freshly constructed module shows: `+0x2b0` is zeroed.
    pub const DEFAULT: View = View::HScene;

    pub fn variant(self) -> &'static str {
        match self {
            View::HScene => "HScene",
            View::PlayData => "PlayData",
        }
    }

    /// The view a header widget selects, for widgets 0 and 1.
    pub fn from_widget(widget: usize) -> Option<View> {
        match widget {
            0 => Some(View::HScene),
            1 => Some(View::PlayData),
            _ => None,
        }
    }
}

/// Where the grid's "this is where you are" records are.
///
/// `FUN_1001a8b0` binds the four page buttons' resting sprites to records 7 to
/// 0xa, `+0x160` to record `view + 3` and `+0x164` to record
/// `view + 5` — the tab of the view showing, and the same tab with the pointer
/// on it. `FUN_1001c1f0` binds `+0x124` to record `page + 0xb` and `+0x150` to
/// record `page + 0xf`, the same pair for the page button. Record `0x13` is the
/// first thumbnail, which is a widget rather than an alternate, so the
/// alternates are records 3 to 0x12.
const PAGE_RECORD: usize = 7;
const TAB_CURRENT: usize = 3;
const TAB_SELECTED: usize = 5;
const PAGE_CURRENT: usize = 0xb;
const PAGE_SELECTED: usize = 0xf;
const ALTERNATES: usize = TAB_CURRENT;
const THUMBNAIL_RECORD: usize = 0x13;

/// Points an atlas's alternate records at the grid's own.
///
/// The generic search reads them off the end of whatever segment it last
/// matched, which is not where this screen's are; and the grid's first seven
/// records are byte for byte the play-data list's, so the tabs alone do not say
/// which of the two tables was found. The four page buttons do — the list has
/// ten, at different x — so the anchor is all seven boxes at their own indices.
///
/// On failure the alternates are **cleared** rather than left as they were: an
/// index into a run this screen does not own would draw a sprite from another
/// screen's table, and no mark at all is the better wrong answer.
pub fn place_alternates(atlas: &mut days_ui::Atlas, dll: &[u8], boxes: &[days_ui::Rect]) {
    let anchors: Option<Vec<(usize, days_ui::Rect)>> = (0..3)
        .map(|i| Some((i, *boxes.get(i)?)))
        .chain(
            (0..HSCENE_PAGES)
                .map(|page| Some((PAGE_RECORD + page, *boxes.get(HSCENE_FIRST_PAGE + page)?))),
        )
        .collect();
    let placed = anchors
        .as_deref()
        .and_then(|anchors| days_ui::atlas::table_at(dll, anchors))
        .and_then(|base| {
            (ALTERNATES..THUMBNAIL_RECORD)
                .map(|index| days_ui::atlas::record_at(dll, base, index))
                .collect::<Option<Vec<_>>>()
        });
    match placed {
        Some(extras) => atlas.extras = extras,
        None => {
            log::warn!("no record table in the DLL marks the replay grid's tab and page");
            atlas.extras.clear();
        }
    }
}

/// Which alternate record a widget draws instead of its resting or active one,
/// from `FUN_1001a460`.
///
/// The tab of the view showing and the button of the page showing, each with a
/// second form for the pointer being on it. The answer indexes the run
/// [`place_alternates`] built.
pub fn extra_for(widget: usize, view: View, page: usize, selected: bool) -> Option<usize> {
    let record = if View::from_widget(widget) == Some(view) {
        widget + if selected { TAB_SELECTED } else { TAB_CURRENT }
    } else if widget == HSCENE_FIRST_PAGE + page && page < HSCENE_PAGES {
        page + if selected {
            PAGE_SELECTED
        } else {
            PAGE_CURRENT
        }
    } else {
        return None;
    };
    record.checked_sub(ALTERNATES)
}

/// The sprite sheet a page of thumbnails is cut from.
///
/// `FUN_1001b3f0` formats `Replay_Thm%02d.png` with the page number plus one,
/// so the four pages are `Replay_Thm01` to `Replay_Thm04`.
pub fn thumbnail_sheet(page: usize) -> String {
    format!("System/Replay/Replay_Thm{:02}.png", page + 1)
}

/// One version of a scene that asks which version to play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The save flag that says the player has seen this version. The popup only
    /// lets a version be picked once its flag is set — `FUN_100195d0`.
    pub flag: String,
    /// The scripts for this version. Only the first is ever started.
    pub scripts: Vec<String>,
    /// The branch table this version walks, or `None` to walk straight down
    /// the list. Only scene 11 has one per version.
    pub branch: Option<Branch>,
}

/// One replay scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scene {
    /// The save flag that unlocks the thumbnail. The DLL asks the host whether
    /// this name is set, and the host looks it up in the global flag store.
    pub flag: String,
    /// The scene's scripts, in sequence. Empty for the three scenes that ask
    /// first, whose scripts live on the [`Choice`]s instead.
    pub scripts: Vec<String>,
    /// The versions to choose between, or empty for a scene that just plays.
    pub choices: Vec<Choice>,
    /// The branch table this scene walks, or `None` to walk straight down the
    /// list. A scene whose versions each have their own table keeps `None`
    /// here and carries them on the [`Choice`]s.
    pub branch: Option<Branch>,
}

impl Scene {
    /// The script a click on this scene starts, before any question.
    pub fn first_script(&self) -> Option<&str> {
        self.scripts
            .first()
            .or_else(|| self.choices.first()?.scripts.first())
            .map(String::as_str)
    }

    /// Whether picking a version is required before anything plays.
    pub fn asks(&self) -> bool {
        !self.choices.is_empty()
    }

    /// What a click on this scene plays, for a scene that asks nothing.
    pub fn run(&self) -> Run {
        Run {
            scripts: self.scripts.clone(),
            branch: self.branch.clone(),
        }
    }
}

/// A scene's branch table: one row per step, three next-step indices a row.
///
/// `FUN_1001ee20` indexes it `step * 0xc + column * 4` — twelve bytes a row and
/// four a column — so a row is three `i32`s. The column is the choice the
/// player last made and the value is the step to play next, where -1 or an
/// index past the end of the scene's list ends the scene.
pub type Branch = Vec<[i32; 3]>;

/// A replay scene in flight: the scripts it plays and how it walks them.
///
/// The original keeps the same pair on the replay module — the scene at
/// `+0x2a8` and the step at `+0x2ac` — and asks `FUN_1001f0d0` for the next
/// script each time one ends. This is what that asking needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Run {
    /// The scene's scripts, indexed by step.
    pub scripts: Vec<String>,
    /// Its branch table, or `None` for the thirty scenes that have none.
    pub branch: Option<Branch>,
}

impl Run {
    /// The script a step plays.
    pub fn script(&self, step: usize) -> Option<&str> {
        self.scripts.get(step).map(String::as_str)
    }

    /// The step after `step`, or `None` when the scene ends.
    ///
    /// `FUN_1001f0d0` calls `FUN_1001ee20(this, 1, 0)`, which is the branch
    /// arm: a scene with a table reads `table[step][column]` and every other
    /// scene takes the `default` arm, `step + 1`. Either way the answer ends
    /// the scene when it is -1, and `FUN_1001f0d0` ends it too when the name it
    /// looks up is the list's NULL terminator — which is what an index equal to
    /// the list's length is.
    pub fn next(&self, step: usize, choice: i32) -> Option<usize> {
        let next = match self.branch.as_ref().and_then(|table| table.get(step)) {
            Some(row) => *row.get(column(choice))?,
            None => i32::try_from(step).ok()?.checked_add(1)?,
        };
        let next = usize::try_from(next).ok()?;
        (next < self.scripts.len()).then_some(next)
    }
}

/// What the last-choice value is before any choice box has settled.
///
/// `FUN_004388c0` writes it into the film object's `+0x1f8` when the object is
/// constructed, and nothing puts it back.
pub const NO_CHOICE: i32 = -2;

/// The column of a branch row the player's last choice selects.
///
/// `FUN_1001ee20` asks the host for it through vtable slot `+0x4`
/// (`FUN_0042c080`), which reads the film object's `+0x1f8`: `-2` until a
/// choice box has ever settled, and after that the index it settled on, which
/// is `-1` when the box was dismissed or ran out of time. The DLL maps `-2` to
/// column 0 and everything else to `choice + 1`, so a dismissed box and one
/// that was never raised share column 0.
///
/// `+0x1f8` is written in exactly two places — `FUN_004388c0` sets it to `-2`
/// once, when the film object is constructed, and `FUN_0043f330` stores the
/// settled index — so the value outlives the script it was made in and the
/// scene that reads it. A scene's first step is therefore walked by whatever
/// the player last answered, which may have been several scripts ago.
///
/// The choice box raises two options, so the column is one of the three the row
/// has; a wider one would read off the end of the row and is refused rather
/// than folded back in.
fn column(choice: i32) -> usize {
    if choice < 0 {
        0
    } else {
        choice as usize + 1
    }
}

/// What can go wrong recovering the tables.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the menu module is not a PE image this engine can read")]
    NotAPeImage,
    #[error(
        "no run of replay scene names in the menu module; the scene table cannot be recovered"
    )]
    NoSceneNames,
    #[error(
        "found {names} replay scene names but {scripts} script lists; \
         the two tables in the menu module do not describe the same scenes"
    )]
    Mismatched { names: usize, scripts: usize },
}

/// The replay scene table, recovered from the player's own menu module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenes {
    scenes: Vec<Scene>,
}

impl Scenes {
    /// Recovers the table from the bytes of the menu module.
    ///
    /// Nothing here is an address. Each table is found by what it contains, and
    /// every step is checked against the next:
    ///
    /// 1. **Scene flags.** A run of consecutive pointers, each to a
    ///    NUL-terminated UTF-16 name shaped like `REP02_2S_W03`. The longest
    ///    such run is the scene list; there is one other, which is shorter.
    /// 2. **Scripts.** The run of pointers to script-path arrays that is at
    ///    least as long as the scene list. Its **first** entries are the
    ///    scenes, one apiece and in the same order, because that is how the
    ///    module reads it: `FUN_1002ddf0` walks one counter from zero over the
    ///    flag run and the script run together, and both click dispatches index
    ///    the script run from its base by the scene number. Some entries point
    ///    into zero-initialised data and carry no list at all; those are the
    ///    scenes that ask a question.
    /// 3. **Versions.** The shorter name run holds the versions' flags, each of
    ///    which is a scene's own flag plus one trailing letter. Grouping it by
    ///    that prefix recovers which scene each belongs to and how many
    ///    versions it has, with no reliance on where any of it sits.
    /// 4. **Version scripts.** What is left of the script run after the scenes,
    ///    handed to the groups in order.
    ///
    /// Three things are then checked rather than assumed, and each one has to
    /// hold for the versions to be kept: the groups have to name exactly the
    /// scenes whose own entry is empty, in the same order; their counts have to
    /// add up to exactly what is left of the script run; and no group may name
    /// a scene that has a list of its own. Against both retail modules all
    /// three hold — 41 scenes and 4 + 2 + 2 versions in `SysMenuSDHQ.dll`, 36
    /// and seven twos in `SysMenuSD.dll` — and a module where they do not keeps
    /// its scenes and reports its versions as not recovered.
    pub fn recover(dll: &[u8]) -> Result<Scenes, Error> {
        let image = Image::parse(dll).ok_or(Error::NotAPeImage)?;

        let mut name_runs =
            image.runs(|img, va| img.wide_string(va).filter(|s| is_scene_flag(s)).map(|_| ()));
        name_runs.sort_by_key(|run| std::cmp::Reverse(run.len()));
        let scene_run = name_runs.first().cloned().ok_or(Error::NoSceneNames)?;
        let flags: Vec<String> = scene_run
            .iter()
            .map(|va| image.wide_string(*va).expect("matched above"))
            .collect();

        // A hole is a pointer into a section's zero-filled tail: real in the
        // loaded image, absent from the file. The three scenes that ask a
        // question have one where their script list would be, so a hole has to
        // continue the run rather than end it.
        let script_runs = image.runs(|img, va| {
            img.script_list(va)
                .map(|_| ())
                .or_else(|| img.points_into_zero_fill(va).then_some(()))
        });
        let table = script_runs
            .iter()
            .filter(|run| run.len() >= flags.len())
            .max_by_key(|run| run.len())
            .ok_or_else(|| Error::Mismatched {
                names: flags.len(),
                scripts: script_runs.iter().map(Vec::len).max().unwrap_or(0),
            })?;
        let (listed, extra) = table.split_at(flags.len());
        let read = |va: &u32| image.script_list(*va).unwrap_or_default();

        let mut scenes: Vec<Scene> = flags
            .into_iter()
            .zip(listed.iter().map(read))
            .map(|(flag, scripts)| Scene {
                flag,
                scripts,
                choices: Vec::new(),
                branch: None,
            })
            .collect();

        // The remaining name run, if there is one, is the versions.
        if let Some(version_flags) = name_runs.get(1) {
            let versions: Vec<String> = version_flags
                .iter()
                .map(|va| image.wide_string(*va).expect("matched above"))
                .collect();
            let groups = group_versions(&versions, &scenes);
            let counted: usize = groups.iter().map(|(_, count)| count).sum();
            let asking: Vec<usize> = scenes
                .iter()
                .enumerate()
                .filter(|(_, scene)| scene.scripts.is_empty())
                .map(|(index, _)| index)
                .collect();
            let named: Vec<usize> = groups.iter().map(|(scene, _)| *scene).collect();
            let lists: Vec<Vec<String>> = if counted == extra.len() && named == asking {
                extra.iter().map(read).collect()
            } else {
                log::warn!(
                    "the menu module's {counted} replay versions over {} scenes do not \
                     account for the {} script lists left after its {} scenes, which have \
                     {} that ask a question; treating the versions' scripts as not recovered",
                    named.len(),
                    extra.len(),
                    scenes.len(),
                    asking.len(),
                );
                Vec::new()
            };
            attach_versions(&mut scenes, &groups, &lists);
        }
        attach_branches(&mut scenes, &image);

        log::info!(
            "recovered {} replay scenes, {} of which ask which version to play",
            scenes.len(),
            scenes.iter().filter(|s| s.asks()).count()
        );
        Ok(Scenes { scenes })
    }

    /// Builds a table directly, for tests and for the offline tools.
    pub fn from_scenes(scenes: Vec<Scene>) -> Scenes {
        Scenes { scenes }
    }

    pub fn len(&self) -> usize {
        self.scenes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scenes.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&Scene> {
        self.scenes.get(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Scene> {
        self.scenes.iter()
    }

    /// How many pages the grid really needs.
    ///
    /// The screen always draws four page buttons, because the DLL's table has
    /// four; this is how many of them hold anything.
    pub fn pages(&self) -> usize {
        self.scenes.len().div_ceil(HSCENE_PER_PAGE)
    }

    /// The scene a thumbnail on a page stands for, from `FUN_1001de10`'s
    /// `page * 12 + widget - 7`.
    ///
    /// The last page is short — 41 scenes over 48 slots — and a slot past the
    /// end is not a scene at all. `FUN_1001c1f0` marks those slots dead with
    /// the same bound.
    pub fn at(&self, page: usize, slot: usize) -> Option<(usize, &Scene)> {
        let index = page * HSCENE_PER_PAGE + slot;
        self.scenes.get(index).map(|scene| (index, scene))
    }

    /// Whether the player has unlocked a scene.
    ///
    /// The DLL asks the host, whose `+0x18` (`FUN_00428770`) looks the scene's
    /// name up in the global flag store — so an unlocked scene is simply a set
    /// flag of that name.
    pub fn unlocked(&self, index: usize, flags: &FlagStore) -> bool {
        self.scenes
            .get(index)
            .is_some_and(|scene| flags.flag(&scene.flag))
    }
}

/// Whether a thumbnail widget can be chosen, from `FUN_1001dd40`.
///
/// The headers and the back button are always live, the four page buttons are
/// always live, and a thumbnail is live only when its scene exists and its flag
/// is set.
pub fn hscene_enabled(scenes: &Scenes, page: usize, widget: usize, flags: &FlagStore) -> bool {
    match widget {
        0..=2 => true,
        HSCENE_FIRST_PAGE..=6 => true,
        HSCENE_FIRST_THUMBNAIL..=0x12 => scenes
            .at(page, widget - HSCENE_FIRST_THUMBNAIL)
            .is_some_and(|(index, _)| scenes.unlocked(index, flags)),
        _ => false,
    }
}

/// What activating a widget on the thumbnail grid does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Not live, or nothing on this screen.
    None,
    /// Show the other screen.
    View(View),
    /// Leave the replay screen.
    Back,
    /// Show a page of the grid.
    Page(usize),
    /// Start this scene's first script.
    Play { scene: usize, script: String },
    /// Ask which version of this scene to play.
    Ask { scene: usize },
}

/// The thumbnail grid's dispatch, from `FUN_1001de10`.
pub fn hscene_action(scenes: &Scenes, page: usize, widget: usize) -> Act {
    if let Some(view) = View::from_widget(widget) {
        return Act::View(view);
    }
    match widget {
        2 => Act::Back,
        HSCENE_FIRST_PAGE..=6 => Act::Page(widget - HSCENE_FIRST_PAGE),
        HSCENE_FIRST_THUMBNAIL..=0x12 => match scenes.at(page, widget - HSCENE_FIRST_THUMBNAIL) {
            Some((index, scene)) if scene.asks() => Act::Ask { scene: index },
            Some((index, scene)) => match scene.first_script() {
                Some(script) => Act::Play {
                    scene: index,
                    script: script.to_string(),
                },
                None => Act::None,
            },
            None => Act::None,
        },
        _ => Act::None,
    }
}

/// The popup's art variant, from `FUN_10018db0`.
///
/// `+0xc8` chooses between the two-widget popup and the four-widget one, and
/// the DLL sets it from how many versions the scene has.
pub fn popup_variant(choices: usize) -> &'static str {
    if choices > 2 {
        "4"
    } else {
        "2"
    }
}

/// Whether a version can be picked, from `FUN_100195d0`: only one the player
/// has already seen.
pub fn popup_enabled(scene: &Scene, choice: usize, flags: &FlagStore) -> bool {
    scene
        .choices
        .get(choice)
        .is_some_and(|c| flags.flag(&c.flag))
}

/// What picking a version plays, from `FUN_10019610` and `FUN_1001f270`.
///
/// The step index is set to zero first, so this is element zero of the chosen
/// version's list — the same script whichever version is picked.
pub fn popup_action(scene: &Scene, choice: usize) -> Option<Run> {
    let picked = scene.choices.get(choice)?;
    (!picked.scripts.is_empty()).then(|| Run {
        scripts: picked.scripts.clone(),
        branch: picked.branch.clone(),
    })
}

/// The grid's thumbnails, cut from one page's `Replay_Thm%02d.png`.
///
/// The pictures are not in the chip sheet. `FUN_1001c1f0` binds the page's own
/// sheet and draws each slot from a second record table — twenty-four records,
/// the twelve slot rectangles twice over, the first twelve being what a
/// selected slot draws and the last twelve what every live slot draws.
///
/// That table is found the same way the widget table is, by content: the only
/// thing in the DLL that reproduces the twelve thumbnail boxes **twice in a
/// row** is this table. The chip sheet's own run reproduces them once and then
/// carries on into the tab headers, so the doubled shape is what tells the two
/// apart — searching for twelve boxes alone finds the chip run first and would
/// cut every thumbnail from the wrong sheet.
#[derive(Debug, Clone)]
pub struct Thumbnails {
    resting: Vec<days_ui::atlas::Widget>,
    selected: Vec<days_ui::atlas::Widget>,
}

impl Thumbnails {
    /// Recovers the table, given the twelve slot boxes from the hit map and the
    /// size of the page's sheet.
    pub fn recover(
        dll: &[u8],
        slots: &[days_ui::cmap::Rect],
        sheet: (u32, u32),
    ) -> Result<Thumbnails, days_ui::Error> {
        let doubled: Vec<days_ui::cmap::Rect> = slots.iter().chain(slots.iter()).copied().collect();
        let atlas = days_ui::atlas::find(dll, &doubled, sheet)?;
        let (selected, resting) = atlas.widgets.split_at(slots.len());
        Ok(Thumbnails {
            resting: resting.to_vec(),
            selected: selected.to_vec(),
        })
    }

    /// The sprite a slot draws: its own picture, or the brighter one when it is
    /// the slot under the pointer.
    pub fn sprite(&self, slot: usize, selected: bool) -> Option<days_ui::atlas::Widget> {
        if selected {
            self.selected.get(slot).copied()
        } else {
            self.resting.get(slot).copied()
        }
    }

    pub fn len(&self) -> usize {
        self.resting.len()
    }

    pub fn is_empty(&self) -> bool {
        self.resting.is_empty()
    }
}

/// Whether a name looks like a scene or version flag: `REP` and then digits,
/// capitals and underscores.
fn is_scene_flag(s: &str) -> bool {
    s.len() >= 8
        && s.len() <= 32
        && s.starts_with("REP")
        && s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// Whether a name looks like a script path: `NN/` and then the script stem.
fn is_script_path(s: &str) -> bool {
    let Some((dir, stem)) = s.split_once('/') else {
        return false;
    };
    !dir.is_empty()
        && dir.len() <= 4
        && dir.bytes().all(|b| b.is_ascii_alphanumeric())
        && stem.len() >= 4
        && stem.len() <= 32
        && stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Splits the version flags into one group per scene, by the scene flag each
/// one is built from.
///
/// A version flag is its scene's flag plus one trailing letter, so the group
/// boundaries come out of the names themselves rather than out of where the run
/// sits. Returns `(scene index, count)` in run order.
fn group_versions(versions: &[String], scenes: &[Scene]) -> Vec<(usize, usize)> {
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for version in versions {
        let stem = &version[..version.len().saturating_sub(1)];
        let Some(scene) = scenes.iter().position(|s| s.flag == stem) else {
            log::warn!("replay version {version} belongs to no scene; ignoring it");
            continue;
        };
        match groups.last_mut() {
            Some((last, count)) if *last == scene => *count += 1,
            _ => groups.push((scene, 1)),
        }
    }
    groups
}

/// Attaches each group of versions to its scene, with the script list that sits
/// at the matching place in the version script run.
///
/// The pairing is by order — [`Scenes::recover`] has already checked that the
/// groups name exactly the scenes with no list of their own, in that order, and
/// that their counts use up exactly the lists there are to give.
fn attach_versions(scenes: &mut [Scene], groups: &[(usize, usize)], lists: &[Vec<String>]) {
    let mut at = 0usize;
    for (scene_index, count) in groups {
        let range = at..at + count;
        at += count;
        let Some(scene) = scenes.get_mut(*scene_index) else {
            continue;
        };
        let flags: Vec<String> = (0..*count)
            .map(|k| format!("{}{}", scene.flag, (b'A' + k as u8) as char))
            .collect();
        let Some(slice) = lists.get(range) else {
            log::warn!(
                "no script lists for the {count} versions of {}; it cannot be played",
                scene.flag
            );
            scene.choices = flags
                .into_iter()
                .map(|flag| Choice {
                    flag,
                    scripts: Vec::new(),
                    branch: None,
                })
                .collect();
            continue;
        };
        if !scene.scripts.is_empty() {
            log::warn!(
                "replay scene {} has a script list of its own and versions as well; \
                 treating the versions' scripts as not recovered",
                scene.flag
            );
            scene.choices = flags
                .into_iter()
                .map(|flag| Choice {
                    flag,
                    scripts: Vec::new(),
                    branch: None,
                })
                .collect();
            continue;
        }
        scene.choices = flags
            .into_iter()
            .zip(slice.iter().cloned())
            .map(|(flag, scripts)| Choice {
                flag,
                scripts,
                branch: None,
            })
            .collect();
    }
}

/// Where a scene gets its next step, as `FUN_1001ee20`'s switch has it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Switch {
    /// One table for the whole scene.
    Table(u32),
    /// A table per version, chosen by the version index at `+0x2b4`. Only
    /// scene 11 reaches this arm.
    Versions(Vec<Option<u32>>),
}

/// Attaches the branch table each scene walks.
///
/// A table that does not read as one for its scene is dropped with a warning
/// and that scene walks straight down its list, which is what every scene
/// without a table does anyway.
fn attach_branches(scenes: &mut [Scene], image: &Image) {
    let switches = branch_switches(image);
    log::info!(
        "recovered branch tables for {} of {} replay scenes",
        switches.len(),
        scenes.len()
    );
    for (index, switch) in switches {
        let Some(scene) = scenes.get_mut(index) else {
            log::warn!("a branch table names scene {index}, which is not in the scene table");
            continue;
        };
        match switch {
            Switch::Table(va) => {
                scene.branch = read_branch(image, va, scene.scripts.len());
                if scene.branch.is_none() {
                    log::warn!(
                        "the branch table for {} does not describe its {} steps; \
                         it will play them in order",
                        scene.flag,
                        scene.scripts.len()
                    );
                }
            }
            Switch::Versions(vas) => {
                for (version, va) in vas.into_iter().enumerate() {
                    let Some(choice) = scene.choices.get_mut(version) else {
                        log::warn!(
                            "{} has a branch table for version {version}, which it does not have",
                            scene.flag
                        );
                        continue;
                    };
                    choice.branch = va.and_then(|va| read_branch(image, va, choice.scripts.len()));
                    if choice.branch.is_none() {
                        log::warn!(
                            "the branch table for {} does not describe its {} steps; \
                             it will play them in order",
                            choice.flag,
                            choice.scripts.len()
                        );
                    }
                }
            }
        }
    }
}

/// Reads `steps` rows of three next-step indices, checking each against the
/// list it walks.
///
/// The check is what makes the address believable: a row's entries are step
/// numbers in the scene's own list, so every one of them has to be -1, a step
/// the list has, or the one past its end that `FUN_1001f0d0` reads as the
/// list's NULL terminator. Against the retail DLL all fourteen tables pass, and
/// each one's length — the scene's script count — lands exactly on the next
/// table or on the alignment padding before it.
fn read_branch(image: &Image, va: u32, steps: usize) -> Option<Branch> {
    if steps == 0 {
        return None;
    }
    let mut table = Branch::with_capacity(steps);
    for step in 0..steps {
        let mut row = [0i32; 3];
        for (column, slot) in row.iter_mut().enumerate() {
            let at = va.checked_add(((step * 3 + column) * 4) as u32)?;
            let value = image.dword(at)? as i32;
            if value < -1 || value > steps as i32 {
                return None;
            }
            *slot = value;
        }
        table.push(row);
    }
    Some(table)
}

/// Finds `FUN_1001ee20`'s switch in the image and reads the table address out
/// of every arm of it.
///
/// This is the one table in this module that is not found by its own contents,
/// because it has none to be found by: a branch table is a run of small
/// integers, and the image is full of those. What is distinctive is the code
/// that reads it. `FUN_1001ee20` is a dense MSVC switch — a byte map from scene
/// index to case, and a table of case addresses — whose arms are all the same
/// five instructions:
///
/// ```text
/// MOV  r1, [EBP+this]
/// MOV  r2, [r1 + 0x2ac]          the step index
/// IMUL r2, r2, 0xc               twelve bytes a row
/// MOV  r3, [EBP+column]
/// MOV  r4, [r2 + r3*4 + table]   four bytes a column
/// ```
///
/// So the scan looks for a switch whose arms read `+0x2ac`, multiply it by
/// twelve and index by four — which is the branch rule itself, written in
/// instructions — and takes the addresses out of them. Scene 11's arm is a
/// second switch on the version at `+0x2b4` whose own arms have the same shape.
///
/// Against the retail DLL exactly one switch in the whole image matches, and it
/// yields the eleven scenes and the four version tables the decompile shows.
/// Nothing here is an address, and a build whose code does not match this
/// simply recovers no tables and plays every scene in order.
fn branch_switches(image: &Image) -> std::collections::BTreeMap<usize, Switch> {
    let mut best = std::collections::BTreeMap::new();
    for code in image.raw_sections() {
        for at in 0..code.len() {
            let Some(found) = read_switch(image, &code[at..]) else {
                continue;
            };
            if found.len() > best.len() {
                best = found;
            }
        }
    }
    best
}

/// Reads one dense switch and every branch-table arm of it, or `None` when the
/// bytes are not that switch.
fn read_switch(image: &Image, at: &[u8]) -> Option<std::collections::BTreeMap<usize, Switch>> {
    let code = &mut Code::new(at);
    // SUB r, bias / MOV [EBP+d], r / CMP [EBP+d], bound / JA default
    code.lit(&[0x83])?;
    code.range(0xe8, 0xef)?;
    let bias = usize::from(code.byte()?);
    code.lit(&[0x89])?;
    code.range(0x40, 0x7f)?;
    code.byte()?;
    code.lit(&[0x83, 0x7d])?;
    code.byte()?;
    let bound = usize::from(code.byte()?);
    code.above()?;
    // MOVZX ECX, byte [EAX + map] / JMP [ECX*4 + jump]
    code.frame()?;
    code.lit(&[0x0f, 0xb6, 0x88])?;
    let map = image.slice(code.dword()?, bound + 1)?;
    code.lit(&[0xff, 0x24, 0x8d])?;
    let jump = code.dword()?;

    let arms: Vec<u32> = (0..=usize::from(*map.iter().max()?))
        .map(|case| image.dword(jump.checked_add((case * 4) as u32)?))
        .collect::<Option<_>>()?;
    let mut out = std::collections::BTreeMap::new();
    for (index, case) in map.iter().enumerate() {
        let Some(arm) = arms.get(usize::from(*case)).and_then(|va| image.code(*va)) else {
            continue;
        };
        let switch = match read_arm(arm) {
            Some(va) => Switch::Table(va),
            None => match read_versions(image, arm) {
                Some(vas) => Switch::Versions(vas),
                // The `default` arm, `step + 1`, which every other scene takes.
                None => continue,
            },
        };
        out.insert(index + bias, switch);
    }
    (!out.is_empty()).then_some(out)
}

/// The table address one arm of the switch reads, or `None` when the arm is not
/// a branch read at all.
fn read_arm(at: &[u8]) -> Option<u32> {
    let code = &mut Code::new(at);
    code.frame()?;
    code.member(0x2ac)?;
    // IMUL r, r, 0xc
    code.lit(&[0x6b])?;
    code.range(0xc0, 0xff)?;
    code.lit(&[0x0c])?;
    code.frame()?;
    // MOV r, [base + index*4 + disp32]
    code.lit(&[0x8b])?;
    code.one_of(&[0x84, 0x8c, 0x94, 0x9c, 0xa4, 0xac, 0xb4, 0xbc])?;
    code.range(0x80, 0xbf)?;
    code.dword()
}

/// The per-version tables of the arm that switches on `+0x2b4`, for the one
/// scene whose versions each walk their own.
fn read_versions(image: &Image, at: &[u8]) -> Option<Vec<Option<u32>>> {
    let code = &mut Code::new(at);
    code.frame()?;
    code.member(0x2b4)?;
    code.lit(&[0x89])?;
    code.range(0x40, 0x7f)?;
    code.byte()?;
    code.lit(&[0x83, 0x7d])?;
    code.byte()?;
    let bound = usize::from(code.byte()?);
    code.above()?;
    code.frame()?;
    // JMP [r*4 + jump], with no byte map: the version indexes the arms directly.
    code.lit(&[0xff, 0x24])?;
    code.one_of(&[0x85, 0x8d, 0x95, 0x9d, 0xa5, 0xad, 0xb5, 0xbd])?;
    let jump = code.dword()?;
    Some(
        (0..=bound)
            .map(|version| {
                let va = image.dword(jump.checked_add((version * 4) as u32)?)?;
                read_arm(image.code(va)?)
            })
            .collect(),
    )
}

/// A cursor over instruction bytes, for matching one shape against them.
///
/// Every method consumes what it matched and gives `None` when it does not
/// match, so an arm reads as the instructions it is.
struct Code<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Code<'a> {
    fn new(bytes: &'a [u8]) -> Code<'a> {
        Code { bytes, at: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let got = self.bytes.get(self.at..self.at + n)?;
        self.at += n;
        Some(got)
    }

    fn lit(&mut self, want: &[u8]) -> Option<()> {
        (self.take(want.len())? == want).then_some(())
    }

    fn one_of(&mut self, want: &[u8]) -> Option<u8> {
        let got = self.byte()?;
        want.contains(&got).then_some(got)
    }

    fn range(&mut self, low: u8, high: u8) -> Option<u8> {
        let got = self.byte()?;
        (low..=high).contains(&got).then_some(got)
    }

    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn dword(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    /// `JA rel`, in either encoding.
    fn above(&mut self) -> Option<()> {
        match self.one_of(&[0x77, 0x0f])? {
            0x77 => self.byte().map(|_| ()),
            _ => {
                self.lit(&[0x87])?;
                self.dword().map(|_| ())
            }
        }
    }

    /// `MOV r, [EBP+d]` — a frame slot into a register.
    fn frame(&mut self) -> Option<()> {
        self.lit(&[0x8b])?;
        self.one_of(&[0x45, 0x4d, 0x55])?;
        self.byte().map(|_| ())
    }

    /// `MOV r, [r + offset]` — a member of the replay module.
    fn member(&mut self, offset: u32) -> Option<()> {
        self.lit(&[0x8b])?;
        self.range(0x80, 0xbf)?;
        self.lit(&offset.to_le_bytes())
    }
}

/// Just enough of PE32 to read the DLL's data tables.
///
/// `days_gpk::pe` walks the resource directory for the archive key and keeps
/// its section map to itself; this needs the other half — turning a pointer
/// stored in the image back into bytes — so it is a few lines of its own rather
/// than a widening of that module's surface for an unrelated caller.
struct Image<'a> {
    bytes: &'a [u8],
    base: u32,
    sections: Vec<(u32, u32, u32, u32)>,
}

impl<'a> Image<'a> {
    fn parse(bytes: &'a [u8]) -> Option<Image<'a>> {
        let pe = u32le(bytes, 0x3c)? as usize;
        if bytes.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let count = u16le(bytes, pe + 6)? as usize;
        let optional = u16le(bytes, pe + 20)? as usize;
        let base = u32le(bytes, pe + 24 + 28)?;
        let mut sections = Vec::with_capacity(count);
        for i in 0..count {
            let o = pe + 24 + optional + i * 40;
            sections.push((
                u32le(bytes, o + 12)?, // virtual address
                u32le(bytes, o + 8)?,  // virtual size
                u32le(bytes, o + 20)?, // raw pointer
                u32le(bytes, o + 16)?, // raw size
            ));
        }
        Some(Image {
            bytes,
            base,
            sections,
        })
    }

    /// The file offset a virtual address maps to, if the bytes are really in
    /// the file. A section's virtual size can run past its raw size — that tail
    /// is zero-initialised at load and has no bytes here, which is exactly what
    /// the three question-asking scenes' table entries point into.
    fn offset(&self, va: u32) -> Option<usize> {
        let rva = va.checked_sub(self.base)?;
        self.sections.iter().find_map(|&(sva, _, raw, raw_size)| {
            (rva >= sva && rva < sva + raw_size).then(|| (rva - sva + raw) as usize)
        })
    }

    /// The `len` bytes at a virtual address, if the file carries them.
    fn slice(&self, va: u32, len: usize) -> Option<&'a [u8]> {
        let at = self.offset(va)?;
        self.bytes.get(at..at + len)
    }

    /// The little-endian 32-bit word at a virtual address.
    fn dword(&self, va: u32) -> Option<u32> {
        u32le(self.bytes, self.offset(va)?)
    }

    /// Everything from a virtual address to the end of its section, which is
    /// as far as one instruction sequence can run.
    fn code(&self, va: u32) -> Option<&'a [u8]> {
        let rva = va.checked_sub(self.base)?;
        self.sections.iter().find_map(|&(sva, _, raw, raw_size)| {
            (rva >= sva && rva < sva + raw_size)
                .then(|| {
                    self.bytes
                        .get((rva - sva + raw) as usize..(raw + raw_size) as usize)
                })
                .flatten()
        })
    }

    /// The bytes of each section the file carries, for scanning.
    fn raw_sections(&self) -> impl Iterator<Item = &'a [u8]> {
        self.sections
            .iter()
            .filter_map(|&(_, _, raw, raw_size)| {
                self.bytes.get(raw as usize..(raw + raw_size) as usize)
            })
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// Whether an address lands in a section's zero-filled tail — past the
    /// bytes the file carries but inside the section the loader maps.
    fn points_into_zero_fill(&self, va: u32) -> bool {
        let Some(rva) = va.checked_sub(self.base) else {
            return false;
        };
        self.sections
            .iter()
            .any(|&(sva, vsize, _, raw_size)| rva >= sva + raw_size && rva < sva + vsize)
    }

    /// A NUL-terminated UTF-16 string at a virtual address.
    fn wide_string(&self, va: u32) -> Option<String> {
        let mut at = self.offset(va)?;
        let mut units = Vec::new();
        loop {
            let unit = u16le(self.bytes, at)?;
            if unit == 0 {
                return String::from_utf16(&units).ok();
            }
            if units.len() > 64 {
                return None;
            }
            units.push(unit);
            at += 2;
        }
    }

    /// An array of pointers to script paths, NULL- or non-pointer-terminated.
    fn script_list(&self, va: u32) -> Option<Vec<String>> {
        let mut at = self.offset(va)?;
        let mut out = Vec::new();
        while out.len() < 16 {
            let Some(target) = u32le(self.bytes, at) else {
                break;
            };
            match self.wide_string(target).filter(|s| is_script_path(s)) {
                Some(path) => out.push(path),
                None => break,
            }
            at += 4;
        }
        (!out.is_empty()).then_some(out)
    }

    /// Every maximal run of consecutive 4-byte-aligned pointers whose targets
    /// all satisfy `accept`.
    ///
    /// Runs are found rather than located: a table is whatever contiguous
    /// stretch of the image reads as one. A run may hold entries that are not
    /// real pointers at all where the table has a hole — those break the run,
    /// and the caller allows for it by matching on the run's length.
    fn runs(&self, accept: impl Fn(&Image<'a>, u32) -> Option<()>) -> Vec<Vec<u32>> {
        let mut out = Vec::new();
        for &(sva, _, raw, raw_size) in &self.sections {
            let start = raw as usize;
            let end = (start + raw_size as usize).min(self.bytes.len());
            if start >= end {
                continue;
            }
            // Only whole, aligned pointers, and only where a pointer could sit.
            let align = (self.base.wrapping_add(sva)) as usize % 4;
            let mut run: Vec<u32> = Vec::new();
            let mut at = start + (4 - align) % 4;
            while at + 4 <= end {
                let target = u32le(self.bytes, at).unwrap_or(0);
                if accept(self, target).is_some() {
                    run.push(target);
                } else if !run.is_empty() {
                    out.push(std::mem::take(&mut run));
                }
                at += 4;
            }
            if !run.is_empty() {
                out.push(run);
            }
        }
        out
    }
}

fn u16le(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32le(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

#[cfg(test)]
mod tests {

    /// A click dispatches the scene, and what the engine plays is the scene's
    /// whole list. `FUN_1001f0d0` is asked for the next script each time one
    /// ends, so the sequence is what has to be handed over, not its first name.
    #[test]
    fn a_scene_hands_back_every_step_it_plays() {
        let steps = ["02/02-2S-W00", "02/02-2S-W00b", "02/02-2S-W00c"];
        let scenes = Scenes::from_scenes(vec![scene("REP02_2S_W00", &steps)]);
        match hscene_action(&scenes, 0, HSCENE_FIRST_THUMBNAIL) {
            Act::Play { scene: at, script } => {
                assert_eq!(at, 0);
                // The dispatch names the opening script, and the scene it names
                // carries the rest — which is what the menu turns into the
                // sequence the engine walks.
                assert_eq!(script, steps[0]);
                let played = &scenes.get(at).expect("just built").scripts;
                assert_eq!(played.len(), steps.len());
                assert_eq!(played.last().map(String::as_str), Some(steps[2]));
            }
            other => panic!("a filled thumbnail should play, got {other:?}"),
        }
    }
    use super::*;
    use days_save::Value;

    fn scene(flag: &str, scripts: &[&str]) -> Scene {
        Scene {
            flag: flag.to_string(),
            scripts: scripts.iter().map(|s| s.to_string()).collect(),
            choices: Vec::new(),
            branch: None,
        }
    }

    /// A table shaped like the real one: enough scenes to fill a page and spill
    /// onto a second, with one that asks a question.
    fn table() -> Scenes {
        let mut scenes: Vec<Scene> = (0..14)
            .map(|i| {
                scene(
                    &format!("REP02_2S_W{i:02}"),
                    &[&format!("02/02-2S-W{i:02}")],
                )
            })
            .collect();
        scenes[11] = Scene {
            flag: "REP03_KB_N00".to_string(),
            scripts: Vec::new(),
            choices: (0..4)
                .map(|k| Choice {
                    flag: format!("REP03_KB_N00{}", (b'A' + k) as char),
                    scripts: vec!["03/03-KB-N00".to_string()],
                    branch: None,
                })
                .collect(),
            branch: None,
        };
        Scenes::from_scenes(scenes)
    }

    fn unlocked(names: &[&str]) -> FlagStore {
        FlagStore::from_entries(names.iter().map(|n| (n.to_string(), Value::Bool(true))))
    }

    #[test]
    fn a_page_holds_twelve_thumbnails_over_four_pages() {
        assert_eq!(HSCENE_PER_PAGE * HSCENE_PAGES, 48);
        // Forty-one scenes need four pages, the last of them short.
        let scenes = Scenes::from_scenes(
            (0..41)
                .map(|i| scene(&format!("REP02_2S_W{i:02}"), &[]))
                .collect(),
        );
        assert_eq!(scenes.pages(), HSCENE_PAGES);
        assert!(scenes.at(3, 4).is_some(), "3 * 12 + 4 = 40");
        assert!(scenes.at(3, 5).is_none(), "41 is past the end");
    }

    /// `page * 12 + widget - 7`, and a slot past the end is not a scene.
    #[test]
    fn a_thumbnail_maps_to_its_scene_through_the_page() {
        let scenes = table();
        assert_eq!(scenes.at(0, 0).unwrap().0, 0);
        assert_eq!(scenes.at(1, 1).unwrap().0, 13);
        assert!(scenes.at(1, 2).is_none());

        assert_eq!(
            hscene_action(&scenes, 1, HSCENE_FIRST_THUMBNAIL),
            Act::Play {
                scene: 12,
                script: "02/02-2S-W12".to_string()
            }
        );
        assert_eq!(
            hscene_action(&scenes, 1, HSCENE_FIRST_THUMBNAIL + 2),
            Act::None,
            "an empty slot on the last page"
        );
    }

    #[test]
    fn the_headers_the_back_button_and_the_pages_are_the_same_everywhere() {
        let scenes = table();
        assert_eq!(hscene_action(&scenes, 0, 0), Act::View(View::HScene));
        assert_eq!(hscene_action(&scenes, 0, 1), Act::View(View::PlayData));
        assert_eq!(hscene_action(&scenes, 0, 2), Act::Back);
        for page in 0..HSCENE_PAGES {
            assert_eq!(
                hscene_action(&scenes, 0, HSCENE_FIRST_PAGE + page),
                Act::Page(page)
            );
        }
    }

    /// A thumbnail is live only when its scene's own flag is set.
    #[test]
    fn a_thumbnail_needs_its_scene_flag() {
        let scenes = table();
        let none = FlagStore::default();
        let some = unlocked(&["REP02_2S_W00", "REP02_2S_W03"]);

        assert!(!hscene_enabled(&scenes, 0, HSCENE_FIRST_THUMBNAIL, &none));
        assert!(hscene_enabled(&scenes, 0, HSCENE_FIRST_THUMBNAIL, &some));
        assert!(!hscene_enabled(
            &scenes,
            0,
            HSCENE_FIRST_THUMBNAIL + 1,
            &some
        ));
        assert!(hscene_enabled(
            &scenes,
            0,
            HSCENE_FIRST_THUMBNAIL + 3,
            &some
        ));

        // The chrome never locks.
        for widget in [0, 1, 2, 3, 6] {
            assert!(hscene_enabled(&scenes, 0, widget, &none));
        }
    }

    #[test]
    fn a_scene_that_asks_raises_the_popup_instead_of_playing() {
        let scenes = table();
        // Scene 11 is slot 11 of page 0.
        let act = hscene_action(&scenes, 0, HSCENE_FIRST_THUMBNAIL + 11);
        assert_eq!(act, Act::Ask { scene: 11 });
        assert!(scenes.get(11).unwrap().asks());
        assert!(!scenes.get(0).unwrap().asks());
    }

    /// Four versions get the four-widget popup, two get the two-widget one.
    #[test]
    fn the_popup_variant_follows_how_many_versions_there_are() {
        assert_eq!(popup_variant(2), "2");
        assert_eq!(popup_variant(4), "4");
    }

    /// A version can only be picked once the player has seen it, and what it
    /// plays is its own list from element zero.
    #[test]
    fn a_version_needs_its_own_flag_and_plays_its_own_list() {
        let scenes = table();
        let scene = scenes.get(11).unwrap();
        let seen = unlocked(&["REP03_KB_N00B"]);

        assert!(!popup_enabled(scene, 0, &seen));
        assert!(popup_enabled(scene, 1, &seen));
        assert!(!popup_enabled(scene, 3, &seen));

        // `FUN_1002c020` sets the step to zero and plays element zero of the
        // chosen version's list, so a version whose list opens on a different
        // script opens on a different script. Both of `SysMenuSD.dll`'s uniform
        // scenes are that shape.
        let scene = Scene {
            flag: "REP04_S1_B03".to_string(),
            scripts: Vec::new(),
            choices: ["04/Z4-S1-B03", "04/04-S1-B03"]
                .iter()
                .enumerate()
                .map(|(k, script)| Choice {
                    flag: format!("REP04_S1_B03{}", (b'A' + k as u8) as char),
                    scripts: vec![script.to_string()],
                    branch: None,
                })
                .collect(),
            branch: None,
        };
        assert_eq!(
            popup_action(&scene, 0).and_then(|r| r.script(0).map(str::to_owned)),
            Some("04/Z4-S1-B03".to_string())
        );
        assert_eq!(
            popup_action(&scene, 1).and_then(|r| r.script(0).map(str::to_owned)),
            Some("04/04-S1-B03".to_string())
        );
        assert_eq!(popup_action(&scene, 9), None);
    }

    /// `FUN_1001ee20` + `FUN_1001f0d0`: the column is the last choice, the
    /// `default` arm is `step + 1`, and both -1 and the index that lands on the
    /// list's NULL terminator end the scene.
    #[test]
    fn a_scene_walks_its_branch_table_by_the_last_choice() {
        let scripts: Vec<String> = (0..4).map(|i| format!("05/05-KC-D{i:02}")).collect();
        // Scene 36's shape: the first step fans out three ways and every other
        // step stops. The last row ends on -1 rather than on the terminator, so
        // both endings are walked.
        let run = Run {
            scripts,
            branch: Some(vec![[1, 2, 3], [4, 0, 0], [4, 0, 0], [-1, 0, 0]]),
        };
        assert_eq!(run.next(0, NO_CHOICE), Some(1));
        assert_eq!(run.next(0, -1), Some(1));
        assert_eq!(run.next(0, 0), Some(2));
        assert_eq!(run.next(0, 1), Some(3));
        assert_eq!(run.next(2, NO_CHOICE), None);
        assert_eq!(run.next(3, NO_CHOICE), None);
        // A choice wider than the row would read off the end of it.
        assert_eq!(run.next(0, 2), None);

        // No table is the `default` arm: straight down the list, whatever the
        // player last answered, and it stops when the list runs out.
        let plain = Run {
            branch: None,
            ..run
        };
        assert_eq!(plain.next(0, 1), Some(1));
        assert_eq!(plain.next(2, 0), Some(3));
        assert_eq!(plain.next(3, 0), None);
    }

    #[test]
    fn each_page_of_thumbnails_comes_from_its_own_sheet() {
        assert_eq!(thumbnail_sheet(0), "System/Replay/Replay_Thm01.png");
        assert_eq!(thumbnail_sheet(3), "System/Replay/Replay_Thm04.png");
    }

    #[test]
    fn the_module_opens_on_the_thumbnail_grid() {
        assert_eq!(View::DEFAULT, View::HScene);
        assert_eq!(View::DEFAULT.variant(), "HScene");
        assert_eq!(View::PlayData.variant(), "PlayData");
    }

    #[test]
    fn the_name_shapes_accept_the_real_ones_and_little_else() {
        assert!(is_scene_flag("REP02_2S_W03"));
        assert!(is_scene_flag("REP05_5O_D10"));
        assert!(!is_scene_flag("REP"), "too short");
        assert!(!is_scene_flag("System/Replay/Replay_Thm01.png"));
        assert!(
            !is_scene_flag("rep02_2s_w03"),
            "the real ones are upper case"
        );

        assert!(is_script_path("02/02-2S-W03"));
        assert!(is_script_path("05/05-KC-A00"));
        assert!(!is_script_path("02-2S-W03"), "no folder");
        assert!(!is_script_path("System/Replay/Replay_Thm01.png"));
    }

    /// Version flags are grouped by the scene name they are built from, not by
    /// where they sit, so a run holding several scenes' versions splits right.
    #[test]
    fn versions_group_by_the_scene_they_name() {
        let scenes = vec![
            scene("REP03_KB_N00", &[]),
            scene("REP04_C1_A00", &[]),
            scene("REP05_5H_D00", &[]),
        ];
        let versions: Vec<String> = [
            "REP03_KB_N00A",
            "REP03_KB_N00B",
            "REP03_KB_N00C",
            "REP03_KB_N00D",
            "REP04_C1_A00A",
            "REP04_C1_A00B",
            "REP05_5H_D00A",
            "REP05_5H_D00B",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(group_versions(&versions, &scenes), [(0, 4), (1, 2), (2, 2)]);
    }

    /// Versions belong to the scenes with no list of their own — the entry in
    /// the script run that is a hole. A scene that has both is a pairing that
    /// has gone wrong somewhere, so its versions keep their flags and lose
    /// their scripts rather than being believed.
    #[test]
    fn versions_on_a_scene_that_already_has_scripts_are_refused() {
        let first = vec!["03/03-KB-N00".to_string()];
        let second = vec!["03/03-KB-N01".to_string()];

        let mut scenes = vec![scene("REP03_KB_N00", &[])];
        attach_versions(&mut scenes, &[(0, 2)], &[first.clone(), second.clone()]);
        assert_eq!(scenes[0].choices.len(), 2);
        assert_eq!(scenes[0].choices[0].flag, "REP03_KB_N00A");
        assert_eq!(scenes[0].choices[0].scripts, first);
        assert_eq!(scenes[0].choices[1].scripts, second);

        let mut scenes = vec![scene("REP03_KB_N00", &["03/03-KB-N00"])];
        attach_versions(&mut scenes, &[(0, 2)], &[first, second]);
        assert_eq!(scenes[0].choices.len(), 2, "the versions still exist");
        assert!(
            scenes[0].choices.iter().all(|c| c.scripts.is_empty()),
            "but their scripts are not recovered"
        );
    }

    #[test]
    fn a_scene_with_no_scripts_at_all_plays_nothing_rather_than_guessing() {
        let scenes = Scenes::from_scenes(vec![scene("REP02_2S_W03", &[])]);
        assert_eq!(scenes.get(0).unwrap().first_script(), None);
        assert_eq!(hscene_action(&scenes, 0, HSCENE_FIRST_THUMBNAIL), Act::None);
    }

    #[test]
    fn a_file_that_is_not_a_pe_image_is_named_rather_than_scanned() {
        assert!(matches!(
            Scenes::recover(b"not a dll at all").unwrap_err(),
            Error::NotAPeImage
        ));
    }
}
