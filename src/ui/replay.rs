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
//! Widgets 0 and 1 are the two tab headers and widget 2 is the back button on
//! both. On the thumbnail screen widgets 3 to 6 are the four page buttons and
//! widgets 7 to 18 are the twelve thumbnails, which is [`HSCENE_PER_PAGE`] ×
//! [`HSCENE_PAGES`] = 48 slots for 41 scenes. `FUN_1001de10` is that dispatch
//! and `FUN_1001dd40` its enablement.
//!
//! # The scene table is the user's, not ours
//!
//! The forty-one scenes are three tables in `SysMenuSDHQ.dll`: a run of
//! pointers to the save-flag name of each scene, a run of pointers to each
//! scene's list of scripts, and a shorter run for the scenes that ask a
//! question first. None of that is embedded here. [`Scenes::recover`] finds the
//! runs **by content** in the player's own DLL, the way
//! [`days_ui::atlas`] finds the widget tables — see
//! [`Scenes::recover`] for how each run is identified and checked.
//!
//! # What a click plays
//!
//! `FUN_1001de10` turns a thumbnail into a scene index — `page * 12 + widget -
//! 7` — and hands the scene's **first** script to the host. A scene's list can
//! hold up to six, and the rest are the steps after it: the engine asks the
//! module for the next one through `FUN_1001f0d0`, which walks a per-scene
//! branch table by the choice the player made during playback.
//!
//! # How a scene walks
//!
//! `FUN_1001f270` starts a scene at step 0 (`+0x2ac`), and each time a script
//! ends `FUN_1001f0d0` is asked for the next one. It calls `FUN_1001ee20` for
//! the next **step index** and looks the name up in the scene's list, returning
//! an empty string when the index is -1, which is the chain ending.
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
//! A row is three `i32` next-step indices, one per choice, and -1 ends the
//! scene. Scene 11 has four such tables, one per version, picked by `+0x2b4`.
//!
//! **The `default` arm is the common case, and it is what this engine does**:
//! step + 1 until the list runs out. Every scene without a table chains exactly
//! as the original does.
//!
//! # The three scenes that ask first
//!
//! Scenes 11, 22 and 30 do not start playing when clicked. The dispatch
//! special-cases them and raises `Pop_Replay`, whose two or four widgets are
//! versions of the scene, each with its own save flag. Whichever the player
//! picks, `FUN_1001f270` sets the step index to zero and plays element zero of
//! that version's script list — and element zero is the same script in every
//! version, so **the choice does not change what starts**. It selects a branch
//! for later, in the eleven scenes that have a table.
//!
//! # Still to build
//!
//! Those eleven branch on the player's choice and this engine walks them
//! linearly instead. The tables are real, they are `i32` triples in the DLL's
//! `.data`, and `FUN_1001ee20` reads them — but **finding them in the user's
//! own DLL is not recovered**. Every other table here is located by its content:
//! the widget boxes by their rectangles, the scene names by their spelling, the
//! thumbnail run by being the twelve boxes twice over. A run of small integers
//! has no such shape, so there is nothing to match on yet and nothing is
//! guessed. Until there is, a branching scene plays its steps in order.

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
}

/// What can go wrong recovering the tables.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("SysMenuSDHQ.dll is not a PE image this engine can read")]
    NotAPeImage,
    #[error(
        "no run of replay scene names in SysMenuSDHQ.dll; the scene table cannot be recovered"
    )]
    NoSceneNames,
    #[error(
        "found {names} replay scene names but {scripts} script lists; \
         the two tables in SysMenuSDHQ.dll do not describe the same scenes"
    )]
    Mismatched { names: usize, scripts: usize },
}

/// The replay scene table, recovered from the player's own `SysMenuSDHQ.dll`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenes {
    scenes: Vec<Scene>,
}

impl Scenes {
    /// Recovers the table from the bytes of `SysMenuSDHQ.dll`.
    ///
    /// Nothing here is an address. Each table is found by what it contains, and
    /// every step is checked against the next:
    ///
    /// 1. **Scene flags.** A run of consecutive pointers, each to a
    ///    NUL-terminated UTF-16 name shaped like `REP02_2S_W03`. The longest
    ///    such run is the scene list; there is one other, which is shorter.
    /// 2. **Scripts.** A window of pointers to script-path arrays, as long as
    ///    the scene list, positioned so that **every entry's first script is
    ///    the one its scene's name implies** — see [`implied_script`]. A window
    ///    has to be matched rather than simply taken, because the scene run and
    ///    the version run below sit next to each other in the image and read as
    ///    one. Three entries point into zero-initialised data and carry no list
    ///    at all; those are the scenes that ask a question, and they are
    ///    allowed to be empty rather than breaking the window.
    /// 3. **Versions.** The shorter name run holds the versions' flags, each of
    ///    which is a scene's own flag plus one trailing letter. Grouping it by
    ///    that prefix recovers which scene each belongs to and how many
    ///    versions it has, with no reliance on where any of it sits.
    /// 4. **Version scripts.** A window as long as the version run, matched the
    ///    same way against the scene each group belongs to.
    ///
    /// Steps 2 and 4 lean on [`implied_script`], which is a rule read off the
    /// data rather than out of the code, so be clear about what that does and
    /// does not buy: it places the window, which means **the first script of
    /// each list is confirmed by construction and not independently**. The rest
    /// of each list — the sequence after the first, which is the part this
    /// module reports and does not yet use — is read from the image and is not
    /// implied by anything. The rule itself holds for all forty-one names and
    /// is checked against the thirty-eight table entries that exist.
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
        let wanted: Vec<Option<String>> = flags.iter().map(|f| implied_script(f)).collect();
        let scripts =
            image
                .match_window(&script_runs, &wanted)
                .ok_or_else(|| Error::Mismatched {
                    names: flags.len(),
                    scripts: script_runs.iter().map(Vec::len).max().unwrap_or(0),
                })?;

        let mut scenes: Vec<Scene> = flags
            .into_iter()
            .zip(scripts)
            .map(|(flag, scripts)| Scene {
                flag,
                scripts,
                choices: Vec::new(),
            })
            .collect();

        // The remaining name run, if there is one, is the versions.
        if let Some(version_flags) = name_runs.get(1) {
            let versions: Vec<String> = version_flags
                .iter()
                .map(|va| image.wide_string(*va).expect("matched above"))
                .collect();
            let groups = group_versions(&versions, &scenes);
            // Every version of a scene starts the same script, so the whole
            // window is expected to read as that scene's implied script,
            // repeated once per version.
            let wanted: Vec<Option<String>> = groups
                .iter()
                .flat_map(|(scene, count)| {
                    let want = scenes.get(*scene).and_then(|s| implied_script(&s.flag));
                    std::iter::repeat_n(want, *count)
                })
                .collect();
            let lists = image
                .match_window(&script_runs, &wanted)
                .unwrap_or_default();
            attach_versions(&mut scenes, &groups, &lists);
        }

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
pub fn popup_action(scene: &Scene, choice: usize) -> Option<&[String]> {
    let scripts = &scene.choices.get(choice)?.scripts;
    (!scripts.is_empty()).then_some(scripts.as_slice())
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

/// The script stem a scene's flag implies.
///
/// Public because [`Scenes::recover`] leans on it and a caller reading that
/// method's contract needs to be able to see exactly what it leans on.
///
/// `REP02_2S_W03` names the script `02/02-2S-W03`: the two digits after `REP`
/// are the folder, and the rest of the name is the stem with underscores turned
/// into dashes. This is a *check*, not the source — the scripts come from the
/// DLL's own table. It exists because the three scenes that ask a question have
/// no entry in that table, so their versions have to be matched by order, and
/// an ordering assumption needs something to verify it against. The rule is
/// confirmed by the thirty-eight scenes that do have an entry.
pub fn implied_script(flag: &str) -> Option<String> {
    let rest = flag.strip_prefix("REP")?;
    let episode = rest.get(..2)?;
    if !episode.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("{episode}/{}", rest.replace('_', "-")))
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
/// The pairing is by order, which is why every group is checked against
/// [`implied_script`] before it is kept.
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
                })
                .collect();
            continue;
        };
        let want = implied_script(&scene.flag);
        if slice
            .iter()
            .any(|list| list.first().map(String::as_str) != want.as_deref())
        {
            log::warn!(
                "the version script lists for {} do not match the scene; \
                 treating its scripts as not recovered",
                scene.flag
            );
            scene.choices = flags
                .into_iter()
                .map(|flag| Choice {
                    flag,
                    scripts: Vec::new(),
                })
                .collect();
            continue;
        }
        scene.choices = flags
            .into_iter()
            .zip(slice.iter().cloned())
            .map(|(flag, scripts)| Choice { flag, scripts })
            .collect();
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

    /// Reads the window of script lists that matches `wanted`.
    ///
    /// `wanted[i]` is the first script entry `i` must have. An entry with no
    /// list at all — a hole — matches anything, since there is nothing there to
    /// disagree with; a window made only of holes is refused, because it would
    /// match every position equally and so identifies nothing.
    fn match_window(
        &self,
        runs: &[Vec<u32>],
        wanted: &[Option<String>],
    ) -> Option<Vec<Vec<String>>> {
        for run in runs {
            let Some(last) = run.len().checked_sub(wanted.len()) else {
                continue;
            };
            for start in 0..=last {
                let lists: Vec<Vec<String>> = run[start..start + wanted.len()]
                    .iter()
                    .map(|va| self.script_list(*va).unwrap_or_default())
                    .collect();
                let filled = lists.iter().filter(|l| !l.is_empty()).count();
                if filled == 0 {
                    continue;
                }
                let agrees = lists.iter().zip(wanted).all(|(list, want)| {
                    list.is_empty() || list.first().map(String::as_str) == want.as_deref()
                });
                if agrees {
                    return Some(lists);
                }
            }
        }
        None
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
                })
                .collect(),
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

    /// A version can only be picked once the player has seen it, and whichever
    /// they pick starts the same script.
    #[test]
    fn a_version_needs_its_own_flag_and_they_all_start_the_same_script() {
        let scenes = table();
        let scene = scenes.get(11).unwrap();
        let seen = unlocked(&["REP03_KB_N00B"]);

        assert!(!popup_enabled(scene, 0, &seen));
        assert!(popup_enabled(scene, 1, &seen));
        assert!(!popup_enabled(scene, 3, &seen));

        // Every version starts on the same script — the choice picks a branch
        // for later, not a different opening — so what differs between them is
        // the rest of the list, and the popup hands back the whole list.
        for choice in 0..scene.choices.len() {
            let scripts = popup_action(scene, choice).expect("every version plays something");
            assert_eq!(scripts.first().map(String::as_str), Some("03/03-KB-N00"));
        }
        assert_eq!(popup_action(scene, 9), None);
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

    /// The naming rule the version matching is checked against.
    #[test]
    fn a_scene_flag_implies_its_script_path() {
        assert_eq!(
            implied_script("REP02_2S_W03").as_deref(),
            Some("02/02-2S-W03")
        );
        assert_eq!(
            implied_script("REP05_5H_D00").as_deref(),
            Some("05/05-5H-D00")
        );
        assert_eq!(implied_script("NOTASCENE"), None);
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

    /// The version script lists are paired by order, so a pairing that does not
    /// match the scene is dropped rather than believed.
    #[test]
    fn version_scripts_that_do_not_match_their_scene_are_refused() {
        let mut scenes = vec![scene("REP03_KB_N00", &[]), scene("REP04_C1_A00", &[])];
        let right = vec!["03/03-KB-N00".to_string()];
        let wrong = vec!["04/04-C1-A00".to_string()];

        attach_versions(&mut scenes, &[(0, 2)], &[right.clone(), right.clone()]);
        assert_eq!(scenes[0].choices.len(), 2);
        assert_eq!(scenes[0].choices[0].flag, "REP03_KB_N00A");
        assert_eq!(scenes[0].choices[1].scripts, right);

        let mut scenes = vec![scene("REP03_KB_N00", &[])];
        attach_versions(&mut scenes, &[(0, 2)], &[right, wrong]);
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
