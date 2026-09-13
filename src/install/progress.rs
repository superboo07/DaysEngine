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
use days_save::{FlagStore, Value};
use std::path::Path;

/// The names the position is kept under in the save's store. The route DLL
/// reads and writes both by these names and nothing else.
const ROUTE: &str = "ROUTE";
const SCENE: &str = "SCENE";

/// The version a retail slot carries, and the one the shipped executable
/// compares against.
///
/// Every slot in the player's install holds `1.0`. What
/// `_GetVersionToRoute@4` computes is **not recovered**, so a slot written
/// from nothing carries this and a slot rewritten carries whatever it had.
const RETAIL_VERSION: f32 = 1.0;

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
    /// The engine version a slot must match, from `_GetVersionToRoute@4`. Not
    /// recovered, so slots are written with what they were read with, or with
    /// the retail 1.0 for a slot written from nothing.
    version: f32,
    /// Whether the affection gauge should be showing, which the DLL raises
    /// through host slot `+0x30` after a delta that moved `001` or `002`.
    gauge_raised: bool,
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
            version: RETAIL_VERSION,
            gauge_raised: false,
        })
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
        // The engine records the answer against the script it was asked at,
        // which is what a slot carries and what replaying reads back.
        if let Some(here) = self
            .routes
            .script(route.max(0) as usize, scene.max(0) as usize)
            .filter(|_| record)
        {
            self.choices.insert(here.to_owned(), choice);
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

    /// The answer the slot recorded at the script now playing, if it has one.
    ///
    /// `FUN_00428a80` looks the current script's name up in the same store
    /// [`Progress::decide`] writes — the executable's `+0xac`, keyed by host
    /// `+0x1a4`, the script in play. The play-data list is what reads it back:
    /// a choice box that nobody answers resolves to this instead of to -1.
    pub fn recorded_choice(&self) -> Option<i32> {
        let (route, scene) = self.position();
        let here = self
            .routes
            .script(route.max(0) as usize, scene.max(0) as usize)?;
        self.choices.get(here).copied()
    }

    /// Runs the branch graph and moves to whatever it names.
    ///
    /// Returns the script to play next, or `None` when the route ends or the
    /// position is not in the graph.
    pub fn advance(&mut self) -> Option<String> {
        let (route, scene) = self.position();
        let scene = u16::try_from(scene).ok()?;
        let (acts, next) = self
            .machine
            .next(usize::try_from(route).ok()?, scene, &self.stores)?;
        let here = self
            .routes
            .script(route.max(0) as usize, scene as usize)
            .unwrap_or_default()
            .to_owned();
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
            Next::Nothing => return None,
        };
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
    pub fn to_slot(&self) -> Slot {
        let (route, scene) = self.position();
        Slot {
            script: self
                .routes
                .script(route.max(0) as usize, scene.max(0) as usize)
                .unwrap_or_default()
                .to_owned(),
            version: self.version,
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
        feeling::zero_reset(&mut self.stores.save, self.deltas.names());
        self.stores.save.set_int(ROUTE, 0);
        self.stores.save.set_int(SCENE, 0);
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
        let (route, scene) = self.position();
        match self.routes.find(&qualify(&slot.script)) {
            Some(table) if table == (route as usize, scene as usize) => {
                log::info!("{} is ROUTE {route} SCENE {scene}", slot.script);
            }
            Some((r, s)) => log::warn!(
                "{} sits at ROUTE {r} SCENE {s} in the tables, but the slot stored                  ROUTE {route} SCENE {scene}; playing on from the slot",
                slot.script
            ),
            None => log::warn!(
                "{} is in no route table; the slot puts the player at ROUTE {route} SCENE {scene}",
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
    /// Two things do it in the original: the last step of the gauge's own ramp,
    /// which is how it comes down in ordinary play, and `FUN_10026050` when the
    /// engine settles the gauge at the start of a script or the end of one.
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
