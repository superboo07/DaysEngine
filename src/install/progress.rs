//! Where the player is in the branch graph, and the stores the graph reads.
//!
//! The route DLL decides everything about what plays next, but it decides it
//! by asking the host questions: what did the player just choose, what is
//! `SCENE`, what is counter `001`, is flag `972` set. This is the host side of
//! that conversation — the two variable stores a save carries, plus the
//! position in the graph, plus the two affection tables the questions are
//! answered against.
//!
//! ```text
//! the save's store     ROUTE, SCENE, the five counters, the numbered gate
//!                      flags, SP***, the BS**** back-bookmarks
//!                      (host +0x08/+0x0c for ints, +0x10/+0x14 for flags --
//!                       both reach the same member, host + 0x14)
//! the global store     SP***, EndClear, EndNo, [EndNN], the save screen's
//!                      display lines    (host +0x18/+0x1c)
//! ```
//!
//! There are **two** stores and not three: the integer and boolean accessors
//! differ only in the `VARIANT` coercion they apply, and a player's own save
//! has `001` as `VT_I4` sitting beside `946` as `VT_BOOL` in one map.
//!
//! # Saving and loading
//!
//! A slot is a log rather than a snapshot — see [`days_save::slot`] — so
//! writing one is a matter of handing over the store, the story points reached
//! and the choices made, and loading one is putting them back. The global
//! store carries what the save screen shows and everything the player has
//! unlocked, and is written alongside.

use crate::install::feeling::{self, Deltas, Thresholds};
use crate::install::ini::Ini;
use crate::install::save::{self, Mark, Slot};
use crate::install::vfs::Vfs;
use days_route::{Act, Context, Machine, Next, Routes};
use days_save::{FlagStore, Value, Version};
use std::path::Path;

/// The names the position is kept under in the save's store. The route DLL
/// reads and writes both by these names and nothing else.
const ROUTE: &str = "ROUTE";
const SCENE: &str = "SCENE";

/// The save flag that turns the Radish-uniform recordings on.
/// `FUN_0041b830` reads it through host slot `+0x10`, which is the save's own
/// store — see [`Progress::uniform_block`].
const NEW_RADISH: &str = "NewRadish";

/// The version a retail slot carries, and the one the shipped executable
/// compares against.
///
/// Every slot in the player's install holds `1.0`, and both titles' route
/// module returns it from the branch the retail path takes. A slot rewritten
/// carries whatever it had; a slot written from nothing carries what the
/// player's own route module reports, which is also what says whether this
/// title spells the field as a float or as text.
///
/// What makes the export take its *other* branch — `0.01` in both titles — is
/// **not recovered**; it asks the object at `[edx+0x34]` a question this engine
/// has not followed.
fn retail_version(route_dll: &[u8]) -> Version {
    let form = days_route::pe::Image::parse(route_dll)
        .ok()
        .and_then(|img| img.route_version());
    match form {
        Some(days_route::pe::RouteVersion::Text(v)) => Version::Text(v),
        Some(days_route::pe::RouteVersion::Float(v)) => Version::Number(v),
        None => {
            log::warn!(
                "the route module's _GetVersionToRoute@4 is not a shape this \
                 engine has recovered; a slot written from nothing will carry \
                 the School Days HQ form"
            );
            Version::default()
        }
    }
}

/// The stores the route DLL questions, and the tables it questions them
/// against.
///
/// Split out from [`Progress`] so the branch graph can be asked a question
/// while it holds a reference to these: the DLL is the thing doing the asking,
/// and the answers come from here.
#[derive(Debug, Default)]
struct Stores {
    /// The save's own store, which is the whole of a slot's state.
    save: FlagStore,
    /// The global store, out of `GlobalFlag.DAT`.
    global: FlagStore,
    thresholds: Thresholds,
    /// The choice the player last made, or -1 when the box timed out. The
    /// engine stores it and the DLL reads it through host slot `+0x04`.
    choice: i32,
}

impl Context for Stores {
    fn choice(&self) -> i32 {
        self.choice
    }

    fn int(&self, name: &str) -> i32 {
        self.save.int(name)
    }

    fn flag(&self, name: &str) -> bool {
        self.save.flag(name)
    }

    fn global_flag(&self, name: &str) -> bool {
        self.global.flag(name)
    }

    fn threshold(&self, script: &str) -> bool {
        feeling::passes(&self.save, &self.thresholds, script)
    }
}

/// The player's position in the branch graph.
#[derive(Debug)]
pub struct Progress {
    routes: Routes,
    machine: Machine,
    deltas: Deltas,
    stores: Stores,
    /// The story points the player has reached, for the slot to carry.
    marks: std::collections::BTreeMap<String, Mark>,
    /// The choice made at each script, likewise.
    choices: std::collections::BTreeMap<String, i32>,
    /// The engine version a slot must match, from `_GetVersionToRoute@4`, in
    /// whichever form this title's route module reports it. Slots are written
    /// with what they were read with, or with what that module reports for a
    /// slot written from nothing.
    version: Version,
    /// Whether the affection gauge should be showing, which the DLL raises
    /// through host slot `+0x30` after a delta that moved `001` or `002`.
    gauge_raised: bool,
    /// The script now playing, as the engine keeps it at `engine + 0x1a0`.
    ///
    /// This is the name **after** the uniform-block swap, which is what the
    /// rest of the engine reads: the file that is opened, the key the recorded
    /// choices are filed under, and the name a slot carries. See
    /// [`Progress::uniform_block`].
    script: String,
    /// The name [`Progress::script`] was swapped **from**, as the engine keeps
    /// it at `engine + 0x1d8`, or empty when the swap did not fire.
    ///
    /// `FUN_0041b830` writes it: where `_CheckUniformBlock@4` matches, it puts
    /// the original name here and hands the `Z` spelling to host `+0x144`, so
    /// the engine plays one name while remembering the other. Both are then
    /// marked read at the end of the script — see [`Progress::mark_read`].
    ///
    /// The same function writes it empty on every script the swap does not
    /// fire on, so it never outlives the scene it belongs to.
    twin: String,
    /// The scenes that have a second recording, out of `_CheckUniformBlock@4`.
    /// Empty for a title whose route module does not export it.
    uniform: Vec<String>,
    /// The dress the player chose, `engine + 0x7fc`.
    ///
    /// This is what `NewRadish` is: the dress-select screen commits through
    /// host slot `+0x48`, which is `FUN_0041dc50` — it stores the argument at
    /// the subobject's `+0x7d0`, the object's `+0x7fc`, and raises `+0x7d4`
    /// beside it. `FUN_1000ded0` passes 1 for widget 0 and 0 for widget 1, so
    /// the flag is set for the left dress. See [`crate::ui::dress`].
    ///
    /// The engine then moves it between that member and the save's store.
    /// `FUN_0041eb10` writes it into the store as `NewRadish` with host
    /// `+0x14` at the start of a film run — after `_ZeroReset@4` has emptied
    /// the store, so the choice outlives the reset where nothing else in the
    /// store does — and `FUN_0041c440` writes it again after a rewind. The
    /// same function reads the flag back out with `+0x10` into `+0x7fc`
    /// whenever a slot or a story point is put back, so a loaded save decides
    /// which dress the run is wearing.
    new_radish: bool,
}

/// What the route module says about a script's `[EndRoll]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndRoll {
    /// Whether the statement plays at all.
    pub plays: bool,
    /// The letter the movie path's last character becomes, at a position
    /// whose end roll ships as a pair.
    pub letter: Option<char>,
    /// Whether the `Ex01` episode title card is laid over the start of the
    /// end roll. See [`days_route::Machine::change_subtitle`].
    pub card: bool,
}

impl Progress {
    /// Reads the route DLL and the two affection tables out of the install.
    ///
    /// The tables come from the packs, so a missing one is logged and left
    /// empty rather than refused: a script with no entry credits nothing and
    /// passes no gate, which is what the DLL's own substring search does with
    /// a file it could not load.
    /// `global` is the player's own `GlobalFlag.DAT`. It is a parameter and
    /// not something this fills in later because saving writes the store back
    /// over that file: a `Progress` built without it would erase everything
    /// the player has unlocked the first time they saved.
    pub fn load(
        vfs: &Vfs,
        route_dll: &[u8],
        global: FlagStore,
    ) -> Result<Progress, days_route::Error> {
        let read = |path: &str| match vfs.read_path(path) {
            Ok(bytes) => bytes,
            Err(err) => {
                log::warn!("reading {path}: {err}");
                Vec::new()
            }
        };
        Ok(Progress {
            routes: Routes::recover(route_dll)?,
            machine: Machine::recover(route_dll)?,
            deltas: Deltas::parse(&read("Ini/FeelingScript.ini")),
            stores: Stores {
                global,
                thresholds: Thresholds::parse(&read("Ini/StanderdScript.ini")),
                choice: -1,
                ..Default::default()
            },
            marks: Default::default(),
            choices: Default::default(),
            version: retail_version(route_dll),
            gauge_raised: false,
            script: String::new(),
            twin: String::new(),
            uniform: days_route::pe::Image::parse(route_dll)
                .map(|img| img.uniform_block())
                .unwrap_or_default(),
            new_radish: false,
        })
    }

    /// The dress the player committed to, as host slot `+0x48` is told it.
    ///
    /// `FUN_0041dc50` stores it at `engine + 0x7fc`; [`Progress::film_start`]
    /// is what puts it into the save's store. Non-zero is the left dress,
    /// which is the one the `Z` recordings are of — see
    /// [`Progress::uniform_block`] and [`crate::ui::dress::host_value`].
    pub fn set_dress(&mut self, dress: u32) {
        self.new_radish = dress != 0;
    }

    /// Swaps a script for its Radish-uniform recording, the way
    /// `FUN_0041b830` does.
    ///
    /// Every name the route module hands over goes through this before
    /// anything else sees it — `FUN_0041cb60` case 7 after
    /// `_GetNextScriptFile@12`, `FUN_0041c440` after `_GetBackScriptFile@12`,
    /// `FUN_0041eb10` after `_LoadInitScript@4`. All three store the name at
    /// `engine + 0x1a0` and then call `FUN_0041b830` with it. It swaps when
    /// three things hold:
    ///
    /// ```text
    /// host slot +0x10 says the save's flag NewRadish is set
    /// the name is not the empty string
    /// _CheckUniformBlock@4 finds one of its 288 names inside it
    /// ```
    ///
    /// and the swap is one character: `replace(3, 1, L"Z")`, so
    /// `02/02-22-B04` becomes `02/Z2-22-B04`. The result goes back over
    /// `engine + 0x1a0` through host slot `+0x144` (`FUN_0041a4f0`, which
    /// assigns it to the subobject's `+0x174`), and the name it replaced is
    /// kept at `engine + 0x1d8`.
    ///
    /// So the `Z` name is the script that plays, the key the recorded choices
    /// are filed under, and the name the slot's tag-1 record carries. The
    /// player's own saves say so from the other side: across all 77 Shiny
    /// slots a script is spelled with `Z` exactly when
    /// `_CheckUniformBlock@4`'s table matches it, and 376 of the recorded
    /// choices are filed under `Z` names.
    ///
    /// `School Days HQ` never swaps: its route module exports no
    /// `_CheckUniformBlock@4`, so the table is empty, and its packs hold no
    /// `Z` scripts to swap to.
    ///
    /// `NewRadish` is the dress-select screen's answer — see
    /// [`Progress::new_radish`] — so the 288 second recordings are the left
    /// dress, and the two scenes `RouteProcSD.dll` suffixes `A`/`B` through
    /// host `+0x44` branch on the same choice.
    fn uniform_block(&self, name: &str) -> String {
        uniform_block(&self.uniform, self.stores.save.flag(NEW_RADISH), name)
    }

    /// Puts the player at a named script, the way `searchRoot` does.
    ///
    /// Returns whether the name was in any route's table. One that is not
    /// leaves the position alone: the script still plays, it just does not
    /// chain, which is the right answer for a script named on the command line.
    pub fn enter(&mut self, script: &str) -> bool {
        // Both spellings are in use: the tables store `00/00-00-A00` and the
        // rest of the engine passes the trailing name around.
        let full = qualify(script);
        self.script = full.clone();
        self.twin.clear();
        match self.routes.find(&full) {
            Some((route, scene)) => {
                self.stores.save.set_int(ROUTE, route as i32);
                self.stores.save.set_int(SCENE, scene as i32);
                log::info!("{full} is ROUTE {route} SCENE {scene}");
                true
            }
            None => {
                log::info!("{full} is in no route table, so nothing follows it");
                false
            }
        }
    }

    /// What the route module answers about the `[EndRoll]` of the script
    /// playing here.
    ///
    /// Shiny Days' `RouteProcSD.dll` exports `_CheckEndRollView@4`,
    /// `_CheckEndRollSelect@8` and `_ChangeSubtitle@4`; `RouteProcSDHQ.dll`
    /// exports none of them, so on School Days HQ this always says "plays, as
    /// written, with no card over it", which is what
    /// that engine does. See [`days_route::Machine::end_roll_view`] for how
    /// both are decoded, and [`crate::playback::stage::apply_end_roll`] for
    /// what the executable does with the answers.
    pub fn end_roll(&self) -> EndRoll {
        let (route, scene) = self.position();
        let (route, scene) = (route.max(0) as usize, scene.max(0) as u16);
        EndRoll {
            plays: self.machine.end_roll_view(route, scene, &self.stores),
            letter: self
                .machine
                .end_roll_select(route, scene, &self.stores)
                .map(|a| if a { 'A' } else { 'B' }),
            card: self.machine.change_subtitle(route, scene, &self.stores),
        }
    }

    /// The current `(ROUTE, SCENE)`.
    pub fn position(&self) -> (i32, i32) {
        (self.stores.save.int(ROUTE), self.stores.save.int(SCENE))
    }

    /// Records a decided choice and credits what it earns.
    ///
    /// The engine calls `_SetFeeling@8(host, 1)` the moment a choice box
    /// settles — `FUN_00431740` does it right after storing the index, and
    /// before the script has ended — so the deltas land here and not when the
    /// transition is taken. `_SetFeeling@8` works the destination out for
    /// itself rather than being told it, and most scenes credit nothing.
    ///
    /// `record` is false while the play-data list is following a slot's own
    /// answers: `FUN_00431740` calls `FUN_00428a50` — the write — only when
    /// host `+0x98` is clear, so a followed playthrough credits its deltas
    /// without writing over the recording it is reading.
    pub fn decide(&mut self, choice: i32, record: bool) {
        self.stores.choice = choice;
        let (route, scene) = self.position();
        // The engine records the answer against the script it was asked at --
        // host `+0x1a4`, so the uniform-block swap's name and not the table's.
        if record && !self.script.is_empty() {
            self.choices.insert(self.script.clone(), choice);
        }
        let Ok(scene) = u16::try_from(scene) else {
            return;
        };
        let Some(script) = self
            .machine
            .credited(route.max(0) as usize, scene, &self.stores)
        else {
            return;
        };
        if feeling::credit(&mut self.stores.save, &self.deltas, &script) {
            self.gauge_raised = true;
        }
        log::info!("choice {choice} credits the deltas of {script}");
    }

    /// The script now playing, in the spelling the route tables use.
    ///
    /// `engine + 0x188` in `SCHOOLDAYS HQ.exe` and `engine + 0x1a0` in
    /// `SHINYDAYS.exe`, after the uniform-block swap. This is the name the
    /// read record is filed under — see [`Progress::mark_read`].
    pub fn script(&self) -> &str {
        &self.script
    }

    /// The answer the slot recorded at the script now playing, if it has one.
    ///
    /// `FUN_00428a80` looks the current script's name up in the same store
    /// [`Progress::decide`] writes — the executable's `+0xac`, keyed by host
    /// `+0x1a4`, the script in play. The play-data list is what reads it back:
    /// a choice box that nobody answers resolves to this instead of to -1.
    pub fn recorded_choice(&self) -> Option<i32> {
        self.choices.get(&self.script).copied()
    }

    /// Records that the player has read the script now playing.
    ///
    /// This is the per-script read record: one global flag per script, named
    /// by its route-table spelling — `00/00-00-A00`, the same name the tables
    /// hold — and set to `true`. The player's own stores carry 1,854 of them
    /// in School Days HQ and 2,585 in Shiny Days, and it is what the route map
    /// and the replay list read a scene's "seen" state out of.
    ///
    /// **Who writes it.** Host slot `+0x1c`, the global store's boolean
    /// setter (`FUN_00428800`, a `FUN_0045d180` on the map at host `+0x10`;
    /// `+0x18` is the matching getter). The end-of-script block calls it with
    /// the name the engine was told to play, held at `engine + 0x188` in
    /// `SCHOOLDAYS HQ.exe` and `engine + 0x1a0` in `SHINYDAYS.exe`, and only
    /// then asks the route module what follows. HQ does it in `FUN_00424020`;
    /// Shiny in `FUN_0041c440` and in `FUN_0041cb60`'s state 7. So a script is
    /// marked when it **ends**, not when it starts, and the last script of a
    /// route is marked even though nothing follows it.
    ///
    /// **When it is not written.** Two gates sit above the call, and both are
    /// reproduced by where this is called from rather than by a flag here:
    ///
    /// - `vt[0x104]` (HQ) / `vt[0x120]` (Shiny) — `engine + 0x150`, raised by
    ///   `vt[0xa4]`/`FUN_004280b0` when a named script is played on its own.
    ///   That is the replay path, which takes `_GetNextChapter@8` instead and
    ///   never reaches the mark. A replay here returns before
    ///   [`Progress::advance`] is ever called.
    /// - `engine + 0x560` (HQ) / `+0x5b8` (Shiny), raised by `vt[0x124]`
    ///   together with a reset of the moving flag. That is a rewind or a
    ///   route-map jump, which goes to `_GetBackScriptFile@12` or `searchRoot`
    ///   and, again, past the mark. Neither path here runs through `advance`.
    ///
    /// **The twin.** Shiny marks a second name when the uniform-block swap has
    /// fired: `engine + 0x7fc` set and the string at `engine + 0x1d8`
    /// non-empty, at `0x0041c642` and `0x0041cf87`. `FUN_0041b830` is what
    /// fills it — under the `NewRadish` flag it hands the `Z` spelling to host
    /// `+0x144` and keeps the original here — so the pair is credited
    /// together, which is the same rule `_GetReadScriptCount@4` counts by.
    /// School Days HQ has no such second call.
    ///
    /// The player's own `GlobalFlag.DAT` agrees: Shiny's carries both
    /// `01/01-00-A01` and `01/Z1-00-A01`, and the `Z` sits at character 3,
    /// exactly where `FUN_0041a090(name, 3, 1, L"Z", 1)` puts it.
    ///
    /// Nothing is written to disk here. The shipped engine flushes the store
    /// at three boundaries and no others — saving a slot (host `vt[0xa0]`,
    /// `FUN_0042aea0`), the film run ending (`FUN_004236f0`, off the run
    /// thread in `FUN_00427780`) and the engine tearing down (`FUN_00422e10`)
    /// — all three through `FUN_0042b5f0`, the writer for the `FlagFileName`
    /// the film INI names. See [`Progress::save_to`].
    /// **The blank entry.** Both players' stores carry one entry whose name is
    /// the empty string, set `true`, and this engine does not write it:
    /// [`mark_read`] takes an empty name as nothing to record. The shipped
    /// call is unconditional — neither `0x0041c633` nor `0x0041cf5e` checks
    /// the string first — so the original does write it whenever the name is
    /// empty at an end of script. Every mark site marks *before* it replaces
    /// the member, so the blank is written by a tick that arrives with the
    /// member already empty, never by the pass that empties it; which tick
    /// that is remains unrecovered. See `docs/FORMATS.md`.
    ///
    /// The empty name is the **route module's own end-of-route sentinel**, not
    /// an uninitialised buffer. `FUN_10005a50` is the emitter each route
    /// handler's `default:` arm calls: it sets `ROUTE` to `-1` through host
    /// slot `+0xc` and then `wcscpy_s(buf, len, L"")`. `days_route` reaches
    /// the same function from the other side without reading it — that is
    /// `Helper::Stop`, and the `Next::Stop` that [`Progress::advance`] already
    /// turns into `ROUTE = -1`. Some arms call
    /// the emitter and still return 1, so even a *successful*
    /// `_GetNextScriptFile@12` can hand back an empty name.
    ///
    /// Three paths carry it into the member the mark reads: `FUN_00425bf0`
    /// case 7 assigns an empty string outright, `FUN_00430f60`'s load retry
    /// hands one to `vt[0x128]`, and `FUN_0043d4c0`'s `[Next]` arm leaves the
    /// script object's `+0x114` (`+0x11c` in Shiny) as constructed — or copies
    /// the emitted `L""` — for the end-of-script block to copy up. It is
    /// **not** `[Exit]`, which parks an empty name too but appears in none of
    /// School Days HQ's 1,857 or Shiny Days' 2,587 scripts.
    ///
    /// **Which tick marks it is still not recovered.** The block's only latch
    /// on that branch is set either by the `ROUTE == -1` test below the mark
    /// or at the end of the load-the-next-script branch that the emptiness
    /// guard skips, and both should latch on the pass that empties the member.
    /// See `docs/FORMATS.md`.
    fn mark_read(&mut self) {
        let (script, twin) = (self.script.clone(), self.twin.clone());
        mark_read(&mut self.stores.global, &script, &twin);
    }

    /// Runs the branch graph and moves to whatever it names.
    ///
    /// Returns the script to play next, or `None` when the route ends or the
    /// position is not in the graph.
    pub fn advance(&mut self) -> Option<String> {
        // Before the graph is asked anything: the shipped order is mark, then
        // `_GetNextScriptFile@12`, so a script that ends a route is recorded
        // even though the call below answers `None`.
        self.mark_read();
        let (route, scene) = self.position();
        let scene = u16::try_from(scene).ok()?;
        let (acts, next) = self
            .machine
            .next(usize::try_from(route).ok()?, scene, &self.stores)?;
        self.take(acts, next)
    }

    /// Runs the rewind and moves to whatever it names: the control bar's
    /// second press of widget 2.
    ///
    /// The same shape as [`Progress::advance`] with two differences, both of
    /// them the shipped block's. `FUN_00424020` takes this branch when the
    /// engine's rewind flag (`engine + 0x560`, raised by host `+0x124`) is
    /// set, and there it calls `_GetBackScriptFile@12` **instead of** the
    /// mark: a screen the player is rewinding out of is not recorded as read.
    /// The acts the rewind's handlers carry include taking the scene's
    /// feeling deltas back off the counters — see `days_route::Act::Uncredit`.
    ///
    /// `None` is a rewind that cannot move: the route module exports no
    /// `_GetBackScriptFile@12`, the position is not in the graph, or the
    /// handler named nothing. The shipped block answers the same way by
    /// leaving `engine + 0x188` alone, so nothing is loaded.
    pub fn back(&mut self) -> Option<String> {
        let (route, scene) = self.position();
        let scene = u16::try_from(scene).ok()?;
        let (acts, next) = self
            .machine
            .back(usize::try_from(route).ok()?, scene, &self.stores)?;
        self.take(acts, next)
    }

    /// Applies what a handler did and moves to what it named, for either
    /// direction. Everything below the export is the same on both.
    fn take(&mut self, acts: Vec<Act>, next: Next) -> Option<String> {
        let here = self.script.clone();
        for act in &acts {
            self.apply(act, &here);
        }
        let played = match next {
            Next::Scene { route, scene } => {
                self.stores.save.set_int(ROUTE, route as i32);
                self.stores.save.set_int(SCENE, scene as i32);
                self.routes
                    .script(route as usize, scene as usize)?
                    .to_owned()
            }
            // The handler writes a name straight into the buffer instead of
            // taking it from a table, and parks `SCENE` on a sentinel. More
            // than one name means the DLL rotates between them; nothing here
            // keeps that counter, so the first is played.
            Next::Named { names, scene } => {
                self.stores.save.set_int(SCENE, scene as i32);
                names.first()?.clone()
            }
            Next::Stop => {
                self.stores.save.set_int(ROUTE, -1);
                log::info!("the route ended");
                return None;
            }
            // `Machine::back` has already read the save for these, and the
            // forward handlers never pass anything but a literal, so this is
            // unreachable from either export.
            Next::Bookmark { name, .. } => {
                log::warn!("the graph named the bookmark {name}, which is not resolved here");
                return None;
            }
            Next::Nothing => return None,
        };
        // Every name the route module produces goes through the swap before
        // anything else sees it, and the name it was swapped from is kept
        // beside it — `FUN_0041b830` writes both, or writes the second empty.
        let swapped = self.uniform_block(&played);
        self.twin = match swapped == played {
            true => String::new(),
            false => played,
        };
        let played = swapped;
        self.script = played.clone();
        self.stores.choice = -1;
        log::info!("{}", {
            let (r, s) = self.position();
            format!("moved to ROUTE {r} SCENE {s}: {played}")
        });
        Some(played)
    }

    /// Everything a slot carries, as it stands.
    ///
    /// A slot is a log: where the player is, the store, the story points and
    /// the choices. The version is the one the slot was loaded with, so a slot
    /// written back still matches what the shipped executable compares it
    /// against and the original game still reads it.
    ///
    /// The script is the one in play — `engine + 0x1a0` — and not a lookup of
    /// `(ROUTE, SCENE)` in the tables. The two differ whenever the
    /// uniform-block swap has fired: the tables hold `02/02-22-B04` and the
    /// slot has to carry `02/Z2-22-B04`, which is the file the original
    /// reopens. See [`Progress::uniform_block`].
    pub fn to_slot(&self) -> Slot {
        Slot {
            script: self.script.clone(),
            version: self.version.clone(),
            store: self.stores.save.clone(),
            marks: self.marks.clone(),
            choices: self.choices.clone(),
        }
    }

    /// Clears the save state, the way the start of a film run does.
    ///
    /// A run is a thread — `FUN_00427850` begins one on `FUN_00427780` — and
    /// it opens with `FUN_00423130`, which empties both halves of the save
    /// state: the store at `engine + 0x40` through `FUN_0045f690`, and the
    /// marks and recorded choices at `engine + 0xac` through `FUN_00432850`.
    /// `FUN_00423a70` then calls `_ZeroReset@4`, which walks the counter names
    /// the `FeelingScript.ini` head declares setting each to 0, and finishes
    /// with `ROUTE` and `SCENE` in `FUN_10006660`.
    ///
    /// So nothing one playthrough sets reaches the next: a New Game after a
    /// finished route starts with that route's gate flags down, and the five
    /// counters present and zero rather than absent. The player's own saves
    /// show the seeding — every slot the original wrote carries `000` and
    /// `003` and `004` at zero.
    ///
    /// The global store is untouched. It is the other store, and everything
    /// the player has unlocked is meant to outlive a run.
    ///
    /// Loading does not need this: `FUN_004336c0` empties the same two halves
    /// again — host `+0x28` for the store — before it reads the slot, so what
    /// the slot carries is the whole of the state either way.
    pub fn film_start(&mut self) {
        self.stores.save = FlagStore::default();
        self.marks.clear();
        self.choices.clear();
        self.stores.choice = -1;
        self.gauge_raised = false;
        self.script.clear();
        self.twin.clear();
        feeling::zero_reset(&mut self.stores.save, self.deltas.names());
        self.stores.save.set_int(ROUTE, 0);
        self.stores.save.set_int(SCENE, 0);
        // The dress the player picked goes in after the reset, not before:
        // it is the one thing a run carries across `_ZeroReset@4`. See
        // `Progress::new_radish`. Only where the title has the mechanism at
        // all — the write-back is `SHINYDAYS.exe`'s, and `SCHOOLDAYS HQ.exe`
        // holds the name nowhere, so an HQ slot must not grow it.
        if !self.uniform.is_empty() {
            self.stores.save.set_flag(NEW_RADISH, self.new_radish);
        }
    }

    /// Puts a slot back, as loading one does.
    ///
    /// **The position comes out of the store, not out of the script name.**
    /// `FUN_004336c0` reads the slot's first record — the script name, the
    /// version and a variable map — and hands that map to `FUN_00428a20`,
    /// which puts it into the engine's own store at `engine + 0x40`. That is
    /// the same member the route DLL questions through host `+0x08`/`+0x0c`,
    /// and `ROUTE` and `SCENE` are two of its names, so restoring the store
    /// *is* restoring the position. The script name is only the file to open:
    /// `FUN_0042a760` takes it to `FUN_00430d20` and nothing on that path
    /// touches either name.
    ///
    /// So a slot whose store disagrees with its script keeps its store. That
    /// cannot arise from a save this game wrote — every one of the 1857 script
    /// names across the 55 route tables is unique, so a name resolves to one
    /// scene — and a disagreement is logged rather than corrected.
    pub fn from_slot(&mut self, slot: Slot) -> Option<String> {
        self.version = slot.version;
        self.stores.save = slot.store;
        self.marks = slot.marks;
        self.choices = slot.choices;
        self.stores.choice = -1;
        self.gauge_raised = false;
        self.script = slot.script.clone();
        // A slot carries the name the swap already produced, which is in no
        // table, so `FUN_0041b830` finds nothing to match and writes the twin
        // empty. It fills again at the next script. See [`Progress::mark_read`].
        self.twin.clear();
        self.new_radish = self.stores.save.flag(NEW_RADISH);
        let (route, scene) = self.position();
        // The store is the position; this only says whether the name agrees
        // with it. A slot written while the uniform-block swap was on carries
        // the `Z` name, which is in no table, so the table's name is put
        // through the same swap before the two are compared.
        let here = self
            .routes
            .script(route.max(0) as usize, scene.max(0) as usize)
            .map(|name| self.uniform_block(name));
        match here {
            Some(name) if name == slot.script => {
                log::info!("{} is ROUTE {route} SCENE {scene}", slot.script);
            }
            Some(name) => log::warn!(
                "the slot puts the player at ROUTE {route} SCENE {scene}, which is {name}, \
                 but its script is {}; playing on from the slot",
                slot.script
            ),
            None => log::warn!(
                "the slot puts the player at ROUTE {route} SCENE {scene}, which is in no \
                 route table; playing on from {}",
                slot.script
            ),
        }
        Some(slot.script)
    }

    /// Jumps to a story point the run has passed, as the route map does.
    ///
    /// `FUN_00428400` formats the number into `SP%03d` and hands it to
    /// `FUN_004331a0`, which finds that mark among the ones the run has
    /// recorded and restores it through `FUN_00432670`: the mark's own store
    /// goes to `FUN_00428a20` — the same install a slot's store gets — and its
    /// script to `FUN_0042a760`, which opens the file. So a jump is a load
    /// whose state came from memory rather than from a file, and the position
    /// rides in the store exactly as it does for a slot.
    ///
    /// **The marks from the picked one onward are erased.** `FUN_004331a0`
    /// destroys every entry from the one it found to the end of the map and
    /// then erases that range, which is right for a rewind: the story points
    /// after this one have not been reached any more. The map is keyed by the
    /// `SP%03d` name, and those sort the same way their numbers do.
    ///
    /// `None` is a story point this run never passed, which the route map does
    /// not offer — it greys a cell whose flag the save's store does not carry.
    pub fn from_story(&mut self, story: u32) -> Option<String> {
        let name = format!("SP{story:03}");
        let mark = self.marks.get(&name)?;
        let script = mark.script.clone();
        self.stores.save = mark.store.clone();
        self.script = script.clone();
        self.twin.clear();
        self.new_radish = self.stores.save.flag(NEW_RADISH);
        self.marks.split_off(&name);
        self.stores.choice = -1;
        self.gauge_raised = false;
        let (route, scene) = self.position();
        log::info!("{name} puts the player at ROUTE {route} SCENE {scene}: {script}");
        Some(script)
    }

    /// The story points the run has reached, which is what the route map lights
    /// its cells from.
    pub fn marks(&self) -> impl Iterator<Item = &str> {
        self.marks.keys().map(String::as_str)
    }

    /// The chapter the player is in — the `N` the save line spells `第N話`.
    pub fn chapter(&self) -> u32 {
        let (route, _) = self.position();
        self.machine.chapter(route.max(0) as usize).unwrap_or(1)
    }

    /// The save's own store, which is the whole of a slot's state.
    pub fn store(&self) -> &days_save::FlagStore {
        &self.stores.save
    }

    /// The global store, which is what the save screen's display lines and
    /// everything the player has unlocked live in.
    pub fn global(&self) -> &days_save::FlagStore {
        &self.stores.global
    }

    pub fn global_mut(&mut self) -> &mut days_save::FlagStore {
        &mut self.stores.global
    }

    /// Writes the player's position into a slot, and the display line the save
    /// screen shows for it into the global store.
    ///
    /// Both files are written: the slot, and `GlobalFlag.DAT`, which is where
    /// the line lives. A slot with no line would show as empty on the screen
    /// even though its file is there, because that is how the shipped screen
    /// decides what to draw.
    pub fn save_to(
        &mut self,
        game: &Path,
        film: &Ini,
        slot: u32,
        english: bool,
        comment: Option<&str>,
    ) -> std::io::Result<()> {
        use crate::install::clock;
        use crate::ui::saveload;

        let (head, tail) = saveload::display_line(clock::now(), self.chapter(), english);
        let (key, sub) = save::slot_keys(film, slot);
        // The two halves are stored joined; the reader splits the chapter back
        // off by character count.
        let comment = comment
            .map(str::to_owned)
            .or_else(|| {
                self.stores
                    .global
                    .get(&sub)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        self.stores
            .global
            .set(&key, Value::Str(format!("{head}{tail}")));
        self.stores.global.set(&sub, Value::Str(comment));

        save::write_slot(game, film, slot, &self.to_slot())?;
        save::write_flags(game, film, &self.stores.global)
    }

    /// Writes the global store out, without touching a slot.
    ///
    /// The read record [`Progress::mark_read`] keeps is in memory until one of
    /// the shipped engine's three flush points, and two of them are not saves:
    /// `FUN_004236f0` runs when the film run ends and `FUN_00422e10` when the
    /// engine tears down, both calling the same `FUN_0042b5f0` that
    /// [`Progress::save_to`] reaches through host `vt[0xa0]`. Leaving playback
    /// is where this engine has that boundary.
    pub fn flush_flags(&self, game: &Path, film: &Ini) -> std::io::Result<()> {
        save::write_flags(game, film, &self.stores.global)
    }

    /// Reads a slot back, returning the script to play.
    pub fn load_from(&mut self, game: &Path, film: &Ini, slot: u32) -> Option<String> {
        let data = save::load_slot(game, film, slot)?;
        log::info!(
            "loading slot {slot}: {} with {} story points",
            data.script,
            data.marks.len()
        );
        self.from_slot(data)
    }

    /// The two counters as the save holds them, and whether the gauge should
    /// be showing.
    ///
    /// These are what the gauge is chasing, not what it is drawing: the ramp
    /// takes about three and a half seconds to reach them. See
    /// [`crate::ui::bar::gauge::Anim`].
    pub fn gauge(&self) -> ((i32, i32), bool) {
        (feeling::gauge(&self.stores.save), self.gauge_raised)
    }

    /// Hides the gauge, through the same slot `+0x30` that raised it.
    ///
    /// The last step of the gauge's own ramp is what does it in ordinary play.
    /// The only other way is a film run starting, which settles the gauge
    /// through `FUN_10026050`. **A script ending does not**: the engine's other
    /// `+0x38` call is under a member that is never set. See
    /// [`crate::ui::bar::gauge::Anim::settle`].
    pub fn lower_gauge(&mut self) {
        self.gauge_raised = false;
    }

    fn apply(&mut self, act: &Act, script: &str) {
        match act {
            Act::SetInt(name, value) => self.stores.save.set_int(name, *value),
            Act::SetFlag(name, on) => self.stores.save.set_flag(name, *on),
            Act::SetGlobalFlag(name, on) => self.stores.global.set_flag(name, *on),
            // The marker sets `SP%03d` in both stores, and records the story
            // point in the slot's own log along with the store as it stands.
            Act::Story(n) => {
                let name = format!("SP{n:03}");
                self.stores.save.set_flag(&name, true);
                self.stores.global.set_flag(&name, true);
                let order = self.marks.len() as i32;
                self.marks.entry(name.clone()).or_insert_with(|| Mark {
                    script: script.to_owned(),
                    story: name,
                    order,
                    store: self.stores.save.clone(),
                });
            }
            // The DLL's clear path formats the number unpadded, which would
            // disagree below 100; no story number is.
            Act::ClearStory(n) => self.stores.save.set_flag(&format!("SP{n}"), false),
            Act::Ending(n) => self.register_ending(*n),
            // Gated in the DLL by a word only a developer machine sets, so
            // this never runs in a shipped game -- see `days_route`.
            Act::ClearRouteFlags => log::info!("the route asked to clear its flags"),
            Act::Host(slot) => log::debug!("the route called host slot {slot:#04x}"),
            // The rewind's half of the feeling table: every
            // `_GetBackScriptFile@12` handler takes the scene it is leaving
            // off the counters again before it names the one before it.
            Act::Uncredit(name) => {
                if feeling::uncredit(&mut self.stores.save, &self.deltas, name) {
                    self.gauge_raised = true;
                }
                log::info!("the rewind takes back the deltas of {name}");
            }
        }
    }

    /// What `FUN_10006590` does when a route reaches an ending.
    ///
    /// The flag's name really is spelled with the trailing `]="`: the DLL
    /// formats `[End%02d]="` and uses the whole thing as a key, and the
    /// player's own `GlobalFlag.DAT` carries names of exactly that shape. The
    /// number comes from the route handler as a literal.
    ///
    /// `EndNo` goes to the global store as an integer through host slot
    /// `+0x24`; `EndClear` goes to both stores.
    fn register_ending(&mut self, no: u32) {
        self.stores
            .global
            .set_flag(&format!("[End{no:02}]=\""), true);
        self.stores.global.set_flag("EndClear", true);
        self.stores.global.set_int("EndNo", no as i32);
        self.stores.save.set_flag("EndClear", true);
        log::info!("ending {no} registered");
    }
}

/// The marking itself, out of [`Progress::mark_read`].
///
/// `script` is the name the engine was told to play and `twin` the name the
/// uniform-block swap fired from, empty when it did not fire. Both go in as
/// `VT_BOOL` true, which is what host `+0x1c` writes.
fn mark_read(global: &mut FlagStore, script: &str, twin: &str) {
    if script.is_empty() {
        return;
    }
    global.set_flag(script, true);
    if twin.is_empty() {
        log::info!("{script} is read");
    } else {
        global.set_flag(twin, true);
        log::info!("{script} is read, and so is {twin} beside it");
    }
}

/// The swap itself, out of [`Progress::uniform_block`].
fn uniform_block(table: &[String], on: bool, name: &str) -> String {
    // `wcsstr`, so a table entry matches anywhere in the name: the table holds
    // `02-22-B04` and the engine passes `02/02-22-B04`.
    if !on || name.len() < 4 || !table.iter().any(|scene| name.contains(scene)) {
        return name.to_owned();
    }
    let mut out = name.to_owned();
    out.replace_range(3..4, "Z");
    log::info!("{name} has a Radish-uniform recording, so {out} plays instead");
    out
}

/// A bare script name as the route tables spell it: `A00` stays, `00-00-A00`
/// gains its chapter directory.
fn qualify(name: &str) -> String {
    if name.contains('/') || name.len() < 2 {
        name.to_owned()
    } else {
        format!("{}/{name}", &name[..2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualifies_a_bare_script_name_the_way_the_tables_spell_it() {
        assert_eq!(qualify("00-00-A00"), "00/00-00-A00");
        assert_eq!(qualify("00/00-00-A00"), "00/00-00-A00");
        assert_eq!(qualify(""), "");
    }

    /// The table `_CheckUniformBlock@4` searches holds bare scene names and
    /// the engine passes qualified ones, so the match has to be the `wcsstr`
    /// the export does; and the swap is a single character at index 3, which
    /// is the chapter digit the qualified name repeats.
    #[test]
    fn the_uniform_swap_matches_a_bare_name_inside_a_qualified_one() {
        let table = [String::from("02-22-B04")];
        assert_eq!(uniform_block(&table, true, "02/02-22-B04"), "02/Z2-22-B04");
        // The flag is what turns it on; without it the scene plays as shot.
        assert_eq!(uniform_block(&table, false, "02/02-22-B04"), "02/02-22-B04");
        // A scene with no second recording is left alone, and so is a name
        // too short to have an index 3 -- the empty name `FUN_0041b830`
        // checks for first.
        assert_eq!(uniform_block(&table, true, "02/02-22-B05"), "02/02-22-B05");
        assert_eq!(uniform_block(&table, true, ""), "");
    }

    /// A scene the uniform swap fired on is read under both spellings, which
    /// is the pair `_GetReadScriptCount@4` counts as one. A scene it did not
    /// fire on has no second name to credit, and School Days HQ never has one
    /// because its route module exports no `_CheckUniformBlock@4`.
    #[test]
    fn a_swapped_scene_is_read_under_both_of_its_names() {
        let mut store = FlagStore::default();
        mark_read(&mut store, "02/Z2-22-B04", "02/02-22-B04");
        assert!(store.flag("02/Z2-22-B04"));
        assert!(store.flag("02/02-22-B04"));

        mark_read(&mut store, "00/00-00-A00", "");
        assert!(store.flag("00/00-00-A00"));
        // Nothing else went in: an empty twin is not a name.
        assert_eq!(store.iter().count(), 3);
    }

    /// The counters, the numbered gate flags and the bookmarks are one map,
    /// and a name can hold either type — which is what a player's own save
    /// has, `001` as `VT_I4` beside `946` as `VT_BOOL`.
    #[test]
    fn the_save_store_holds_both_types_under_one_namespace() {
        let mut store = FlagStore::default();
        store.set_int("001", 69);
        store.set_flag("946", true);
        assert_eq!(store.int("001"), 69);
        assert!(store.flag("946"));
        // A flag read as an integer is zero rather than an error, which is
        // what a typed getter against the wrong tag gives.
        assert_eq!(store.int("946"), 0);
        assert!(!store.flag("001"));
        // A name never written reads as zero: the store creates it on demand.
        assert_eq!(store.int("never written"), 0);
    }
}
