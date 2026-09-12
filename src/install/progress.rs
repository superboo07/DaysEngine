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
//! the save's variable store   ROUTE, SCENE, the five counters, the BS****
//!                             back-bookmarks    (host +0x08 / +0x0c)
//! the save's flag store       the numbered gate flags, SP***
//!                             (host +0x10 / +0x14)
//! the global flag store       SP***, EndClear, End%02d
//!                             (host +0x18 / +0x1c)
//! ```
//!
//! # What this does not do
//!
//! **None of it is written back to a save.** `Save/SaveFileNNN.DAT` is only
//! decoded as far as its head and the flag store embedded in it, so a session's
//! progress lives in memory and is gone when the game closes. Loading a save
//! would seed these stores; writing one is not implemented.

use crate::install::feeling::{Deltas, Feeling, Thresholds};
use crate::install::vfs::Vfs;
use days_route::{Act, Context, Machine, Next, Routes};
use std::collections::BTreeSet;

/// The name the position is kept under in the save's variable store. The route
/// DLL reads and writes both by these names and nothing else.
const ROUTE: &str = "ROUTE";
const SCENE: &str = "SCENE";

/// The stores the route DLL questions, and the tables it questions them
/// against.
///
/// Split out from [`Progress`] so the branch graph can be asked a question
/// while it holds a reference to these: the DLL is the thing doing the asking,
/// and the answers come from here.
#[derive(Debug, Default)]
struct Stores {
    /// The save's variable store. One map holds `ROUTE`, `SCENE`, the five
    /// feeling counters and the `BS****` back-bookmarks alike —
    /// `FUN_00460810` looks all of them up the same way, creating a missing
    /// name on demand, which is why an unset counter reads zero.
    vars: Feeling,
    flags: BTreeSet<String>,
    global: BTreeSet<String>,
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
        self.vars.get(name)
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }

    fn global_flag(&self, name: &str) -> bool {
        self.global.contains(name)
    }

    fn threshold(&self, script: &str) -> bool {
        self.vars.passes(&self.thresholds, script)
    }
}

/// The player's position in the branch graph.
#[derive(Debug)]
pub struct Progress {
    routes: Routes,
    machine: Machine,
    deltas: Deltas,
    stores: Stores,
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
    pub fn load(vfs: &Vfs, route_dll: &[u8]) -> Result<Progress, days_route::Error> {
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
                thresholds: Thresholds::parse(&read("Ini/StanderdScript.ini")),
                choice: -1,
                ..Default::default()
            },
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
                self.stores.vars.set(ROUTE, route as i32);
                self.stores.vars.set(SCENE, scene as i32);
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
        (self.stores.vars.get(ROUTE), self.stores.vars.get(SCENE))
    }

    /// Records a decided choice and credits what it earns.
    ///
    /// The engine calls `_SetFeeling@8(host, 1)` the moment a choice box
    /// settles — `FUN_00431740` does it right after storing the index, and
    /// before the script has ended — so the deltas land here and not when the
    /// transition is taken. `_SetFeeling@8` works the destination out for
    /// itself rather than being told it, and most scenes credit nothing.
    pub fn decide(&mut self, choice: i32) {
        self.stores.choice = choice;
        let (route, scene) = self.position();
        let Ok(scene) = u16::try_from(scene) else {
            return;
        };
        let Some(script) = self
            .machine
            .credited(route.max(0) as usize, scene, &self.stores)
        else {
            return;
        };
        if self.stores.vars.apply(&self.deltas, &script) {
            self.gauge_raised = true;
        }
        log::info!("choice {choice} credits the deltas of {script}");
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
        for act in &acts {
            self.apply(act);
        }
        let played = match next {
            Next::Scene { route, scene } => {
                self.stores.vars.set(ROUTE, route as i32);
                self.stores.vars.set(SCENE, scene as i32);
                self.routes
                    .script(route as usize, scene as usize)?
                    .to_owned()
            }
            // The handler writes a name straight into the buffer instead of
            // taking it from a table, and parks `SCENE` on a sentinel. More
            // than one name means the DLL rotates between them; nothing here
            // keeps that counter, so the first is played.
            Next::Named { names, scene } => {
                self.stores.vars.set(SCENE, scene as i32);
                names.first()?.clone()
            }
            Next::Stop => {
                self.stores.vars.set(ROUTE, -1);
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

    /// The two values the gauge draws, and whether it should be showing.
    pub fn gauge(&self) -> ((i32, i32), bool) {
        (self.stores.vars.gauge(), self.gauge_raised)
    }

    /// Hides the gauge, which `FUN_10026050` does through slot `+0x30`.
    pub fn lower_gauge(&mut self) {
        self.gauge_raised = false;
    }

    fn apply(&mut self, act: &Act) {
        match act {
            Act::SetInt(name, value) => self.stores.vars.set(name, *value),
            Act::SetFlag(name, on) => set(&mut self.stores.flags, name, *on),
            Act::SetGlobalFlag(name, on) => set(&mut self.stores.global, name, *on),
            // The marker sets `SP%03d` in both stores; the clear path formats
            // the number unpadded, which only differs below 100 and no story
            // number is.
            Act::Story(n) => {
                let name = format!("SP{n:03}");
                set(&mut self.stores.flags, &name, true);
                set(&mut self.stores.global, &name, true);
            }
            Act::ClearStory(n) => set(&mut self.stores.flags, &format!("SP{n}"), false),
            // Registering an ending and clearing a route's flags both need
            // more of the save format than is decoded; they are logged so a
            // session shows where they would have happened.
            Act::Ending => log::info!("the route registered an ending"),
            Act::ClearRouteFlags => log::info!("the route cleared its flags"),
            Act::Host(slot) => log::debug!("the route called host slot {slot:#04x}"),
        }
    }
}

fn set(store: &mut BTreeSet<String>, name: &str, on: bool) {
    if on {
        store.insert(name.to_owned());
    } else {
        store.remove(name);
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

    #[test]
    fn a_flag_set_false_is_absent_rather_than_present_and_zero() {
        let mut s = BTreeSet::new();
        set(&mut s, "972", true);
        assert!(s.contains("972"));
        set(&mut s, "972", false);
        assert!(s.is_empty());
    }
}
