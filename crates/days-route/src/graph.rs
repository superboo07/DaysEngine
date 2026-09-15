//! The branch graph: given a `(ROUTE, SCENE)` and what the player did, the
//! script the game plays next.
//!
//! # Where the answer comes from
//!
//! `_GetNextScriptFile@12` is a 55-way switch on `ROUTE`; each case is one
//! route's scene state machine. [`Machine::recover`] reads the switch out of
//! the export itself — no address is written down here — and walks each
//! handler with [`crate::walk`] to reduce it to a decision tree.
//!
//! The helpers a handler calls are classified the same way, by decoding them:
//!
//! ```text
//! sets ROUTE to a literal, and indexes a name table   an emitter
//! sets ROUTE to -1, no table                          the route is over
//! sets SCENE, writes a literal script name            a fixed script
//! formats "SP%d"                                      the story-number marker
//! formats "[End%02d]=\"" and touches EndClear         an ending
//! formats "[%s]=\"" and reads an int                  a StanderdScript gate
//! clears a run of numbered flags                      the route's flag reset
//! ```
//!
//! An **emitter carries the route it emits into**, which is what makes route
//! transitions visible: route 0's last scene calls route 1's emitter, so the
//! recovered edge is `(0, 0x14) -> (1, 0)` and not a fall-through.
//!
//! # How far this was checked
//!
//! Against a Ghidra decompilation of all 55 handlers, the trees here reproduce
//! every one of their 1,840 `case` arms exactly: same emitter, same literal
//! next scene, on every path. Three further checks, each against something
//! recovered independently:
//!
//! - 25 routes hold a comparison of two save counters against each other, and
//!   they are exactly the 25 routes the feeling work found by a different
//!   method.
//! - The 13 sites that call the StanderdScript gate resolve to 13 script
//!   names, which are exactly the 13 entries `StanderdScript.ini` declares.
//! - `_SetFeeling@8` is a second 55-way switch that works out the same
//!   destinations independently. Decoded, the two agree on 1,426 of the 1,435
//!   `(scene, choice)` pairs that credit anything; the nine that differ are
//!   three scenes where the shipped DLL has two arms swapped.
//! - The story numbers the markers assign account for every `SP***` flag in
//!   the player's real saves.
//!
//! # The ending exports
//!
//! Shiny Days' module exports three more functions, and they are decoded the
//! same way — [`Machine::end_roll_view`], [`Machine::end_roll_select`] and
//! [`Machine::change_subtitle`]. They switch on `ROUTE` as well as `SCENE`,
//! so both are seeded, and they take the host as their first argument rather
//! than their third. A module without them answers what School Days HQ's
//! engine does: the `[EndRoll]` plays, as the script wrote it.

use crate::pe::{Image, Section};
use crate::walk::{self, slot, Effect, Leaf, Node as Raw, Val};
use crate::x86::{decode, Alu, Cc, Insn, Op};
use crate::Error;
use std::collections::{BTreeMap, HashMap};

/// A value a recovered test compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    Const(i32),
    /// The choice the player just made (host slot `+0x04`).
    Choice,
    /// An int in the save's variable store (`+0x08`).
    Int(String),
    /// A flag in the save's flag store (`+0x10`).
    Flag(String),
    /// A flag in the global flag store (`+0x18`).
    GlobalFlag(String),
    /// The `StanderdScript.ini` gate for a script: true when the counter that
    /// entry names is **strictly** above the amount it declares.
    Threshold(String),
    /// A host vtable slot called with no arguments, by slot number.
    HostSlot(i32),
    /// A word in the DLL's own `.data`, by address.
    DllWord(u32),
}

/// How two terms are compared. Signed; the unsigned forms in the image are
/// all bound checks that fold away once `SCENE` is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Cmp {
    fn from(cc: Cc) -> Option<Cmp> {
        Some(match cc {
            Cc::E => Cmp::Eq,
            Cc::Ne => Cmp::Ne,
            Cc::L => Cmp::Lt,
            Cc::Le => Cmp::Le,
            Cc::G => Cmp::Gt,
            Cc::Ge => Cmp::Ge,
            _ => return None,
        })
    }

    fn holds(self, a: i32, b: i32) -> bool {
        match self {
            Cmp::Eq => a == b,
            Cmp::Ne => a != b,
            Cmp::Lt => a < b,
            Cmp::Le => a <= b,
            Cmp::Gt => a > b,
            Cmp::Ge => a >= b,
        }
    }
}

/// Something a handler does on its way to an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// `set_int(name, value)` in the save's variable store. The `BS****` names
    /// are back-bookmarks: the scene a branch was decided at, which
    /// `GetBackScriptFile` rewinds through.
    SetInt(String, i32),
    /// `set_bool(name, value)` in the save's flag store.
    SetFlag(String, bool),
    /// `set_bool(name, value)` in the global flag store.
    SetGlobalFlag(String, bool),
    /// Mark this scene's story number: `SP%03d` set true in both stores.
    Story(u32),
    /// Clear a story number's flag: `SP%d` set false in the save's store.
    ///
    /// The DLL's clear path formats the number unpadded, so it would disagree
    /// with the set path below 100 — every story number in the retail build is
    /// 100 or more, so it never does.
    ClearStory(u32),
    /// Register an ending, by its number. The route handler passes it as a
    /// literal, so which ending a scene is is recovered with the edge.
    Ending(u32),
    /// Clear this route's numbered flags. Gated by a DLL word that only a
    /// developer machine sets — see [`Machine::dll_word`].
    ClearRouteFlags,
    /// A host vtable slot the recovery does not name, by slot number.
    Host(i32),
}

/// What the game plays next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Set `ROUTE`/`SCENE` and play that route's table entry.
    Scene { route: u8, scene: u16 },
    /// Park at `SCENE` and play a name written straight into the buffer.
    /// More than one name means the DLL rotates between them.
    Named { names: Vec<String>, scene: u16 },
    /// `ROUTE` becomes -1 and the name is empty: the route is over.
    Stop,
    /// The handler returned without naming anything.
    Nothing,
}

/// One route's behaviour for one scene, as a decision tree.
///
/// The forks are the questions the handler asks the host; the leaves are what
/// it does. `L` is what a leaf carries, which is [`Next`] for the branch graph
/// and the credited script for the feeling table — the two exports have the
/// same shape and differ only in what their arms produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step<L> {
    /// `then` when the comparison holds.
    If {
        lhs: Term,
        cmp: Cmp,
        rhs: Term,
        then: Box<Step<L>>,
        els: Box<Step<L>>,
    },
    Do {
        acts: Vec<Act>,
        next: L,
    },
    /// The decoder met something outside its subset. Says what and where.
    Unrecovered(String),
}

/// What `_GetNextScriptFile@12` does for one scene.
pub type Transition = Step<Next>;

/// What `_SetFeeling@8` credits for one scene: the script whose
/// `FeelingScript.ini` deltas are applied, if any.
pub type Crediting = Step<Option<String>>;

/// What a helper called from a handler turns out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Helper {
    /// Sets `ROUTE` to `route` and copies `table[scene]`.
    Emit {
        route: u8,
    },
    /// Sets `ROUTE` to -1 and copies an empty name.
    Stop,
    /// Sets `SCENE` and copies one of these literal names.
    Named(Vec<String>),
    Story,
    Ending,
    ClearRouteFlags,
    /// The `StanderdScript.ini` gate: reads a counter and answers whether it
    /// is above the entry's amount.
    Threshold,
    /// The `FeelingScript.ini` credit: applies a script's deltas.
    Credit,
    Other,
}

/// The three exports Shiny Days' `RouteProcSD.dll` has and School Days HQ's
/// `RouteProcSDHQ.dll` does not. Each is `None` for a module without it, and
/// the answer then falls back to what the shipped HQ engine does: the
/// `[EndRoll]` plays, as written, with no card over it.
#[derive(Debug, Clone, Copy, Default)]
struct Endings {
    /// `_CheckEndRollView@4`.
    view: Option<u32>,
    /// `_CheckEndRollSelect@8`.
    select: Option<u32>,
    /// `_ChangeSubtitle@4`.
    subtitle: Option<u32>,
}

impl Endings {
    fn all(self) -> impl Iterator<Item = u32> {
        [self.view, self.select, self.subtitle]
            .into_iter()
            .flatten()
    }
}

/// The branch graph, recovered from the player's own `RouteProcSDHQ.dll`.
#[derive(Debug, Clone)]
pub struct Machine {
    dll: Vec<u8>,
    base: u32,
    sections: Vec<Section>,
    /// One handler address per `ROUTE`, out of the export's own switch.
    handlers: Vec<u32>,
    /// The same, for `_SetFeeling@8`.
    feeling: Vec<u32>,
    /// `_GetStory@4`, which is one function rather than a dispatch.
    chapters: Option<u32>,
    /// The three ending exports, each absent in a route module that does not
    /// have it. See [`Machine::end_roll_view`].
    endings: Endings,
    helpers: HashMap<u32, Helper>,
    /// `scene -> story number`, per route.
    story: Vec<BTreeMap<u16, u32>>,
}

impl Machine {
    /// Recovers the branch graph from the bytes of `RouteProcSDHQ.dll`.
    pub fn recover(dll: &[u8]) -> Result<Machine, Error> {
        let img = Image::parse(dll)?;
        let entry = *img
            .exports
            .get("_GetNextScriptFile@12")
            .ok_or(Error::NoExport("_GetNextScriptFile@12"))?;
        let handlers = dispatch(&img, entry)?;
        // `_SetFeeling@8` switches on `ROUTE` the same way. It is not fatal
        // for it to be missing: without it the engine credits nothing, which
        // is wrong but playable, where refusing to load the DLL is not.
        let feeling = match img.exports.get("_SetFeeling@8") {
            Some(&at) => dispatch(&img, at).unwrap_or_default(),
            None => Vec::new(),
        };

        let chapters = img.exports.get("_GetStory@4").copied();
        let endings = Endings {
            view: img.exports.get("_CheckEndRollView@4").copied(),
            select: img.exports.get("_CheckEndRollSelect@8").copied(),
            subtitle: img.exports.get("_ChangeSubtitle@4").copied(),
        };

        let mut m = Machine {
            dll: dll.to_vec(),
            base: img.base,
            sections: img.sections.clone(),
            handlers,
            feeling,
            chapters,
            endings,
            helpers: HashMap::new(),
            story: Vec::new(),
        };
        // Classify every helper any handler calls, then read the story maps
        // out of the markers among them.
        let mut calls = m.collect_calls();
        // The ending exports call the same `StanderdScript.ini` gate a
        // handler does, and it has to be classified for their trees to name
        // it. Their own calls are collected by decoding them straight
        // through rather than by walking every position: they hold no jump
        // table, so a linear decode does not desynchronise on them.
        for at in m.endings.all() {
            for va in calls_in(&img, at) {
                if !calls.contains(&va) {
                    calls.push(va);
                }
            }
        }
        for va in calls {
            let k = classify(&m.image(), va);
            m.helpers.insert(va, k);
        }
        m.story = (0..m.handlers.len()).map(|r| m.story_map(r)).collect();
        log::debug!(
            "recovered {} route handlers, {} story numbers",
            m.handlers.len(),
            m.story.iter().map(BTreeMap::len).sum::<usize>()
        );
        Ok(m)
    }

    fn image(&self) -> Image<'_> {
        Image {
            bytes: &self.dll,
            base: self.base,
            sections: self.sections.clone(),
            exports: BTreeMap::new(),
        }
    }

    /// How many routes the DLL dispatches. The retail build has 55.
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    /// The story number a scene marks, if it marks one.
    ///
    /// The marker runs before the scene's own branching, and sets `SP%03d`
    /// true in both flag stores. These are the flags the route map reads.
    pub fn story(&self, route: usize, scene: u16) -> Option<u32> {
        self.story.get(route)?.get(&scene).copied()
    }

    /// Every `(route, scene, story number)` the markers assign.
    pub fn stories(&self) -> impl Iterator<Item = (usize, u16, u32)> + '_ {
        self.story
            .iter()
            .enumerate()
            .flat_map(|(r, m)| m.iter().map(move |(&s, &n)| (r, s, n)))
    }

    /// The decision tree a route runs for one scene.
    pub fn step(&self, route: usize, scene: u16) -> Option<Transition> {
        let raw = self.step_raw(route, scene)?;
        Some(self.build(&raw, route, scene, Vec::new(), &|l| self.next_of(l)))
    }

    /// What `_SetFeeling@8` credits at one scene, as a decision tree.
    ///
    /// This is its own 55-way switch, not the branch graph's: each route's arm
    /// works out from `SCENE` and the choice which script the player is about
    /// to move to, and credits that script's `FeelingScript.ini` deltas. Most
    /// scenes credit nothing — only the forks do — which is why the branch
    /// graph's destination cannot stand in for it.
    ///
    /// Nor is it always the same destination. At three scenes — route 4 scene
    /// 0x16, route 5 scenes 0x15 and 0x18 — the shipped DLL has two of the
    /// arms **the other way round** from the branch graph's, so the player
    /// moves one way and is credited for the other. That is a bug in the game
    /// and it is reproduced rather than corrected: `SetFeeling` is what
    /// credits, and this is what it credits.
    pub fn crediting(&self, route: usize, scene: u16) -> Option<Crediting> {
        let entry = *self.feeling.get(route)?;
        let raw = walk::run(
            &self.image(),
            entry,
            &[("SCENE", scene as i32)],
            &[(8, Val::Host), (0xc, Val::Imm(1))],
        );
        Some(self.build(&raw, route, scene, Vec::new(), &|l| self.credit_of(l)))
    }

    /// The chapter a route belongs to — the `N` in the save screen's
    /// `第N話`.
    ///
    /// `_GetStory@4` is a 55-way switch on `ROUTE` that returns a small
    /// number, and it is decoded rather than transcribed: the walker seeds
    /// `ROUTE` and reads the constant the export returns. The retail build
    /// groups the 55 routes into six chapters.
    pub fn chapter(&self, route: usize) -> Option<u32> {
        let entry = *self.chapters.as_ref()?;
        let n = walk::run(
            &self.image(),
            entry,
            &[("ROUTE", route as i32)],
            &[(8, Val::Host)],
        );
        match n {
            Raw::Do(leaf) => u32::try_from(leaf.returns?).ok(),
            // Nothing in the export branches on anything but `ROUTE`, so a
            // fork here means the walk did not settle; take neither arm rather
            // than pick one.
            Raw::Branch { .. } | Raw::Unrecovered(_) => None,
        }
    }

    /// A word in the DLL's own `.data`, as the image initialises it.
    ///
    /// Two of these are read by the route code. Neither is a save value:
    ///
    /// - `DllMain` zeroes one on attach and only sets it after loading both
    ///   feeling INIs from the absolute path `Z:\SCHOOLDAYSHQ\Ini\`, so on any
    ///   install it stays 0 and the per-route flag reset it gates never runs.
    /// - The other is written by nothing at all — a whole-file scan finds one
    ///   reference to its address, the single `cmp` that reads it.
    pub fn dll_word(&self, at: u32) -> i32 {
        self.image().u32(at).unwrap_or(0) as i32
    }

    /// Every address any handler calls, for classification.
    ///
    /// Found by walking, not by sweeping the handlers linearly: a handler's
    /// `switch` keeps its byte index table in `.text` between the arms, so a
    /// linear decode desynchronises on it and loses every call after. The
    /// walker does not need the classification to produce its own tree, so
    /// this pass can precede it.
    fn collect_calls(&self) -> Vec<u32> {
        let mut out = Vec::new();
        {
            fn visit(n: &Raw, out: &mut Vec<u32>) {
                let effects = match n {
                    Raw::Branch {
                        effects, then, els, ..
                    } => {
                        visit(then, out);
                        visit(els, out);
                        effects
                    }
                    Raw::Do(leaf) => &leaf.effects,
                    Raw::Unrecovered(_) => return,
                };
                for e in effects {
                    if let Some(va) = e.call {
                        if !out.contains(&va) {
                            out.push(va);
                        }
                    }
                }
            }
            let img = self.image();
            for r in 0..self.handlers.len() {
                for scene in 0..=SCENE_MAX {
                    if let Some(raw) = self.step_raw(r, scene) {
                        visit(&raw, &mut out);
                    }
                    if let Some(&at) = self.feeling.get(r) {
                        visit(
                            &walk::run(
                                &img,
                                at,
                                &[("SCENE", scene as i32)],
                                &[(8, Val::Host), (0xc, Val::Imm(1))],
                            ),
                            &mut out,
                        );
                    }
                }
            }
        }
        out
    }

    /// The marker a route calls, and the story number it assigns each scene.
    fn story_map(&self, route: usize) -> BTreeMap<u16, u32> {
        let img = self.image();
        let Some(marker) = self.marker_of(route) else {
            return BTreeMap::new();
        };
        let mut out = BTreeMap::new();
        for scene in 0..=SCENE_MAX {
            // The marker takes `(host, flag, scene)`.
            let raw = walk::run(
                &img,
                marker,
                &[],
                &[
                    (8, Val::Host),
                    (0xc, Val::Imm(1)),
                    (0x10, Val::Imm(scene as i32)),
                ],
            );
            let Raw::Do(leaf) = &raw else { continue };
            for e in &leaf.effects {
                if let (Some(_), [_, Val::Imm(n), ..]) = (e.call, e.args.as_slice()) {
                    if *n > 0 {
                        out.insert(scene, *n as u32);
                    }
                }
            }
        }
        out
    }

    /// The story marker a route's handler calls before it branches.
    fn marker_of(&self, route: usize) -> Option<u32> {
        let mut n = self.step_raw(route, 0)?;
        loop {
            match n {
                Raw::Branch { effects, els, .. } => {
                    if let Some(v) = self.first_helper(&effects, Helper::Story) {
                        return Some(v);
                    }
                    n = *els;
                }
                Raw::Do(leaf) => return self.first_helper(&leaf.effects, Helper::Story),
                Raw::Unrecovered(_) => return None,
            }
        }
    }

    fn first_helper(&self, effects: &[Effect], want: Helper) -> Option<u32> {
        effects
            .iter()
            .filter_map(|e| e.call)
            .find(|va| self.helpers.get(va) == Some(&want))
    }

    fn step_raw(&self, route: usize, scene: u16) -> Option<Raw> {
        let entry = *self.handlers.get(route)?;
        Some(walk::run(
            &self.image(),
            entry,
            &[("SCENE", scene as i32)],
            &[
                (8, Val::Arg("buf")),
                (0xc, Val::Arg("size")),
                (0x10, Val::Host),
            ],
        ))
    }

    /// Turns the walker's raw tree into the public one.
    fn build<L>(
        &self,
        raw: &Raw,
        route: usize,
        scene: u16,
        mut acts: Vec<Act>,
        leaf: &dyn Fn(&Leaf) -> L,
    ) -> Step<L> {
        match raw {
            Raw::Unrecovered(why) => Step::Unrecovered(why.clone()),
            Raw::Branch {
                effects,
                cc,
                lhs,
                rhs,
                then,
                els,
            } => {
                let before: Vec<&Effect> = effects.iter().collect();
                self.acts_into(&before, route, scene, &mut acts);
                let (Some(l), Some(r)) = (self.term(&before, lhs), self.term(&before, rhs)) else {
                    return Step::Unrecovered(format!(
                        "a comparison of {lhs:?} against {rhs:?} the recovery cannot name"
                    ));
                };
                let Some(cmp) = Cmp::from(*cc) else {
                    return Step::Unrecovered(format!("the condition {cc:?} is not modelled"));
                };
                Step::If {
                    lhs: l,
                    cmp,
                    rhs: r,
                    then: Box::new(self.build(then, route, scene, acts.clone(), leaf)),
                    els: Box::new(self.build(els, route, scene, acts, leaf)),
                }
            }
            Raw::Do(done) => {
                let all: Vec<&Effect> = done.effects.iter().collect();
                self.acts_into(&all, route, scene, &mut acts);
                Step::Do {
                    next: leaf(done),
                    acts,
                }
            }
        }
    }

    /// The public name for one of the walker's values.
    fn term(&self, before: &[&Effect], v: &Val) -> Option<Term> {
        Some(match v {
            Val::Imm(n) => Term::Const(*n),
            Val::Choice => Term::Choice,
            Val::Int(n) => Term::Int(n.clone()),
            Val::Bool(n) => Term::Flag(n.clone()),
            Val::GlobalBool(n) => Term::GlobalFlag(n.clone()),
            Val::DllWord(a) => Term::DllWord(*a),
            Val::HostSlot(s) => Term::HostSlot(*s),
            Val::Returned(va) => {
                // The only call whose result is branched on is the
                // StanderdScript gate; its second argument is the script.
                if self.helpers.get(va) != Some(&Helper::Threshold) {
                    return None;
                }
                let e = before.iter().rev().find(|e| e.call == Some(*va))?;
                match e.args.get(1) {
                    Some(Val::Str(s)) => Term::Threshold(s.clone()),
                    _ => return None,
                }
            }
            _ => return None,
        })
    }

    fn acts_into(&self, effects: &[&Effect], route: usize, scene: u16, out: &mut Vec<Act>) {
        for e in effects {
            let args = e.args.as_slice();
            if let Some(s) = e.slot {
                let name = match args.first() {
                    Some(Val::Str(n)) => Some(n.clone()),
                    _ => None,
                };
                let value = match args.get(1) {
                    Some(Val::Imm(n)) => Some(*n),
                    _ => None,
                };
                out.push(match (s, name, value) {
                    (slot::SET_INT, Some(n), Some(v)) => Act::SetInt(n, v),
                    (slot::SET_BOOL, Some(n), Some(v)) => Act::SetFlag(n, v != 0),
                    (slot::SET_GLOBAL_BOOL, Some(n), Some(v)) => Act::SetGlobalFlag(n, v != 0),
                    _ => Act::Host(s),
                });
                continue;
            }
            let Some(va) = e.call else { continue };
            match self.helpers.get(&va) {
                Some(Helper::Story) => {
                    // The marker's own arguments are `(host, flag, scene)` --
                    // the story number is a literal inside it, which is what
                    // the recovered map holds. `flag` 0 clears the flag rather
                    // than setting it; the handlers only ever pass 1.
                    let Some(n) = self.story(route, scene) else {
                        continue;
                    };
                    if matches!(args.get(1), Some(Val::Imm(1))) {
                        out.push(Act::Story(n));
                    } else {
                        out.push(Act::ClearStory(n));
                    }
                }
                Some(Helper::Ending) => {
                    if let Some(Val::Imm(n)) = args.get(1) {
                        out.push(Act::Ending(u32::try_from(*n).unwrap_or(0)));
                    }
                }
                Some(Helper::ClearRouteFlags) => out.push(Act::ClearRouteFlags),
                _ => {}
            }
        }
    }

    /// The script a leaf credits, if it credits one.
    fn credit_of(&self, leaf: &Leaf) -> Option<String> {
        leaf.effects.iter().rev().find_map(|e| {
            let va = e.call?;
            if self.helpers.get(&va) != Some(&Helper::Credit) {
                return None;
            }
            match e.args.get(1) {
                Some(Val::Str(s)) => Some(s.clone()),
                _ => None,
            }
        })
    }

    /// The script a leaf's calls name, reading the last one that names any.
    fn next_of(&self, leaf: &Leaf) -> Next {
        let mut next = Next::Nothing;
        for e in &leaf.effects {
            let Some(va) = e.call else { continue };
            match self.helpers.get(&va) {
                Some(Helper::Emit { route }) => {
                    let scene = match e.args.get(3) {
                        Some(Val::Imm(n)) => *n as u16,
                        _ => continue,
                    };
                    next = Next::Scene {
                        route: *route,
                        scene,
                    };
                }
                Some(Helper::Named(names)) => {
                    let scene = e
                        .args
                        .iter()
                        .rev()
                        .find_map(|a| match a {
                            Val::Imm(n) => Some(*n as u16),
                            _ => None,
                        })
                        .unwrap_or(0);
                    next = Next::Named {
                        names: names.clone(),
                        scene,
                    };
                }
                Some(Helper::Stop) => next = Next::Stop,
                _ => {}
            }
        }
        next
    }
}

/// The largest `SCENE` the handlers distinguish. The retail dispatch bounds
/// its `switch` at 0xdc; scenes past a route's table are the sentinels the
/// handlers use for their end-of-route arms.
const SCENE_MAX: u16 = 0xdc;

/// The 55 handler addresses, read out of `_GetNextScriptFile@12`'s own switch.
///
/// The export tests `ROUTE` against a bound, jumps through a table, and each
/// arm's first `call` is that route's handler. Nothing here is written down —
/// the bound, the table and the arms all come from the image.
fn dispatch(img: &Image, entry: u32) -> Result<Vec<u32>, Error> {
    // Walking the export with an out-of-range ROUTE would take the default
    // arm, so instead find the indexed jump by decoding forward from the
    // entry, the way the walker resolves one.
    let mut va = entry;
    let mut bound = None;
    for _ in 0..80 {
        let off = img.at(va).ok_or(Error::NoDispatch)?;
        let d = decode(img.bytes, off, va).ok_or(Error::NoDispatch)?;
        match d.insn {
            Insn::Alu {
                op: Alu::Cmp,
                src: Op::Imm(n),
                ..
            } => bound = Some(n),
            Insn::JmpIndirect(Op::Sib {
                base: None,
                index: Some(_),
                scale: 4,
                disp,
            }) => {
                let n = bound.ok_or(Error::NoDispatch)?;
                if !(1..=0x100).contains(&n) {
                    return Err(Error::NoDispatch);
                }
                let mut out = Vec::with_capacity(n as usize + 1);
                for i in 0..=n {
                    let arm = img
                        .u32((disp as u32).wrapping_add(i as u32 * 4))
                        .ok_or(Error::NoDispatch)?;
                    out.push(first_call(img, arm).ok_or(Error::NoDispatch)?);
                }
                return Ok(out);
            }
            Insn::Ret | Insn::Unknown(_) => return Err(Error::NoDispatch),
            _ => {}
        }
        va += d.len as u32;
    }
    Err(Error::NoDispatch)
}

fn first_call(img: &Image, mut va: u32) -> Option<u32> {
    for _ in 0..24 {
        let d = decode(img.bytes, img.at(va)?, va)?;
        if let Insn::Call(to) = d.insn {
            return Some(to);
        }
        if matches!(d.insn, Insn::Ret | Insn::Unknown(_)) {
            return None;
        }
        va += d.len as u32;
    }
    None
}

/// What a helper is, read off its own instructions.
///
/// A flat sweep of everything reachable from its entry: enough to see which
/// store keys it writes, which string literals it holds, and whether it
/// indexes a name table. The helpers are all short and straight-line apart
/// from small `if`s, so no path tracking is needed to tell them apart.
fn classify(img: &Image, entry: u32) -> Helper {
    let mut seen = std::collections::BTreeSet::new();
    let mut todo = vec![entry];
    let mut regs: HashMap<u8, Val> = HashMap::new();
    let mut pushes: Vec<Val> = Vec::new();
    let mut sets: Vec<(i32, Vec<Val>)> = Vec::new();
    let mut strings: Vec<String> = Vec::new();
    let mut tabled = false;
    let mut calls: Vec<u32> = Vec::new();

    while let Some(mut va) = todo.pop() {
        for _ in 0..4_000 {
            if !seen.insert(va) {
                break;
            }
            let (Some(off), Some(())) = (img.at(va), Some(())) else {
                break;
            };
            let Some(d) = decode(img.bytes, off, va) else {
                break;
            };
            match &d.insn {
                Insn::Unknown(_) => break,
                Insn::Mov { dst, src } => {
                    let v = match *src {
                        Op::Imm(n) => Val::Imm(n),
                        Op::Sib { scale: 4, .. } => {
                            tabled = true;
                            Val::Unknown
                        }
                        Op::Mem(5, disp) if disp > 0 => Val::Arg("param"),
                        Op::Mem(r, disp) => match regs.get(&r) {
                            Some(Val::Host) if disp == 0 => Val::Vtable,
                            Some(Val::Vtable) => Val::HostSlot(disp),
                            _ => Val::Unknown,
                        },
                        _ => Val::Unknown,
                    };
                    if let Op::Reg(r) = *dst {
                        // A handler's first argument is the host pointer.
                        regs.insert(
                            r,
                            if matches!(v, Val::Arg(_)) {
                                Val::Host
                            } else {
                                v
                            },
                        );
                    }
                }
                Insn::Push(src) => {
                    let v = match *src {
                        Op::Imm(n) => Val::Imm(n),
                        Op::Reg(r) => regs.get(&r).cloned().unwrap_or(Val::Unknown),
                        _ => Val::Unknown,
                    };
                    // A constant reaching a push, directly or through a
                    // register, is a string pointer whenever it resolves to one.
                    pushes.push(match v {
                        Val::Imm(n) => match img.wide_ascii(n as u32) {
                            Some(s) => {
                                strings.push(s.clone());
                                Val::Str(s)
                            }
                            None => Val::Imm(n),
                        },
                        other => other,
                    });
                }
                Insn::CallIndirect(target) => {
                    let slot = match target {
                        Op::Reg(r) => match regs.get(r) {
                            Some(Val::HostSlot(s)) => Some(*s),
                            _ => None,
                        },
                        _ => None,
                    };
                    let args: Vec<Val> = pushes.drain(..).rev().collect();
                    if let Some(s) = slot {
                        sets.push((s, args));
                    }
                }
                Insn::Call(to) => {
                    calls.push(*to);
                    pushes.clear();
                }
                Insn::Jmp(to) => {
                    va = *to;
                    continue;
                }
                Insn::Jcc { to, .. } => todo.push(*to),
                Insn::Ret => break,
                _ => {}
            }
            va += d.len as u32;
        }
    }

    let named = |key: &str| {
        sets.iter().find_map(|(s, a)| {
            (*s == slot::SET_INT && matches!(a.first(), Some(Val::Str(n)) if n == key))
                .then(|| a.get(1).cloned())
                .flatten()
        })
    };
    if let Some(v) = named("ROUTE") {
        return match (v, tabled) {
            (Val::Imm(r), true) if (0..=0xff).contains(&r) => Helper::Emit { route: r as u8 },
            _ => Helper::Stop,
        };
    }
    let has = |s: &str| strings.iter().any(|x| x == s);
    if named("SCENE").is_some() {
        let names = strings
            .iter()
            .filter(|s| *s != "SCENE" && !s.is_empty())
            .cloned()
            .collect();
        return Helper::Named(names);
    }
    if has("EndClear") {
        return Helper::Ending;
    }
    // Both INI readers build the same `[<script>]="` key and search the file
    // text for it. They are told apart by what they do with the entry: the
    // gate reads a counter back through the host and answers a comparison,
    // while the credit hands the pair to the add/subtract helpers and makes no
    // host call of its own.
    if has("[%s]=\"") {
        return if sets.iter().any(|(s, _)| *s == slot::GET_INT) {
            Helper::Threshold
        } else {
            Helper::Credit
        };
    }
    if sets
        .iter()
        .filter(|(s, a)| *s == slot::SET_BOOL && matches!(a.get(1), Some(Val::Imm(0))))
        .count()
        > 4
    {
        return Helper::ClearRouteFlags;
    }
    if has("SP%d") || calls.iter().any(|&c| formats_sp(img, c)) {
        return Helper::Story;
    }
    Helper::Other
}

/// Every address a function calls directly, by decoding it from its entry.
///
/// Used for the ending exports, which are straight-line code with forward
/// jumps and no jump table, so a linear decode does not desynchronise on them
/// the way it would on a route handler's `switch`. The scan ends at the first
/// `ret` no jump already seen reaches past, which is the function's own end.
fn calls_in(img: &Image, entry: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut furthest = entry;
    let mut va = entry;
    for _ in 0..600 {
        let Some(off) = img.at(va) else { return out };
        let Some(d) = decode(img.bytes, off, va) else {
            return out;
        };
        match d.insn {
            Insn::Call(to) => {
                if !out.contains(&to) {
                    out.push(to);
                }
            }
            Insn::Jmp(to) | Insn::Jcc { to, .. } => furthest = furthest.max(to),
            Insn::Ret if va >= furthest => return out,
            Insn::Unknown(_) => return out,
            _ => {}
        }
        va += d.len as u32;
    }
    out
}

/// Whether a function holds the `SP%d` format the story marker writes through.
fn formats_sp(img: &Image, entry: u32) -> bool {
    let mut va = entry;
    for _ in 0..600 {
        let Some(off) = img.at(va) else { return false };
        let Some(d) = decode(img.bytes, off, va) else {
            return false;
        };
        if let Insn::Push(Op::Imm(n)) = d.insn {
            if img.wide_ascii(n as u32).as_deref() == Some("SP%d") {
                return true;
            }
        }
        if matches!(d.insn, Insn::Unknown(_)) {
            return false;
        }
        va += d.len as u32;
    }
    false
}

/// What the engine has to answer for the branch graph to run.
pub trait Context {
    /// The choice the player just made, or -1 when there was none.
    fn choice(&self) -> i32;
    /// An int in the save's variable store. An unset name reads 0.
    fn int(&self, name: &str) -> i32;
    /// A flag in the save's flag store.
    fn flag(&self, name: &str) -> bool;
    /// A flag in the global flag store.
    fn global_flag(&self, name: &str) -> bool;
    /// The `StanderdScript.ini` gate for a script.
    fn threshold(&self, script: &str) -> bool;
    /// A host vtable slot the recovery does not name.
    ///
    /// Only one is ever branched on, `+0x34`, and the executable's only writer
    /// of the member it returns is a function nothing in the image calls — no
    /// call, no jump, no address taken — so it is 0 in the shipped game. The
    /// default says so; the branch it selects is unreachable retail code.
    fn host_slot(&self, slot: i32) -> i32 {
        let _ = slot;
        0
    }
}

impl Machine {
    /// Runs the branch graph: what `_GetNextScriptFile@12` would answer.
    ///
    /// Returns what to play next and everything the handler did on the way,
    /// in order. The caller applies the acts — they are writes to the save's
    /// stores, which this crate does not own.
    pub fn next(&self, route: usize, scene: u16, cx: &dyn Context) -> Option<(Vec<Act>, Next)> {
        self.decide(self.step(route, scene)?, route, scene, cx)
    }

    /// Runs `_SetFeeling@8`: the script whose deltas this scene's choice
    /// credits, if it credits any.
    pub fn credited(&self, route: usize, scene: u16, cx: &dyn Context) -> Option<String> {
        let (_, credited) = self.decide(self.crediting(route, scene)?, route, scene, cx)?;
        credited
    }

    /// Whether the `[EndRoll]` at this position plays at all.
    ///
    /// `_CheckEndRollView@4` asks the host for `ROUTE` and `SCENE` and
    /// answers 0 — do not play it — at three positions, each behind its own
    /// condition: `(0x44, 0x0c)` when the save flag `878` is set, `(0x60,
    /// 0x0b)` when the `StanderdScript.ini` gate for the script named in the
    /// table at that scene passes, and `(0x64, 0x0c)` when the save flag
    /// `890` is **clear**. Everywhere else it answers 1. None of that is
    /// written down here: the export is decoded out of the player's own
    /// module with `ROUTE` and `SCENE` seeded, the same as a route handler.
    ///
    /// `FUN_0042b770`'s `[EndRoll]` arm creates the movie only when this
    /// answers non-zero, and `FUN_0042a8d0` — the pass that works out how
    /// long the script is — cuts the script's length and its skip target to
    /// the `[EndRoll]`'s own start frame when it answers 0, so a suppressed
    /// end roll ends the film where it would have begun.
    ///
    /// The executable puts one more gate in front of both: host slot `+0x120`
    /// (`FUN_0041d9f0`), which returns the member slot `+0xb0`
    /// (`FUN_00420110`) raises when a screen hands the engine a script to
    /// play. **Whether anything clears it, and so whether it can still be up
    /// when an `[EndRoll]` is reached, is not recovered.** This engine has no
    /// queued-play member at all — a menu screen's `Play` is acted on by the
    /// tick that produces it — so nothing here stands in for that gate.
    ///
    /// A module with no such export plays every `[EndRoll]`, which is what
    /// School Days HQ does.
    pub fn end_roll_view(&self, route: usize, scene: u16, cx: &dyn Context) -> bool {
        let Some(entry) = self.endings.view else {
            return true;
        };
        self.ending(entry, route, scene, cx, &|l| l.returns)
            .flatten()
            != Some(0)
    }

    /// Which of a pair of end rolls plays: `Some(true)` for `A`, `Some(false)`
    /// for `B`, and `None` at a position that has only one.
    ///
    /// `_CheckEndRollSelect@8` answers through an out-parameter at three
    /// positions — `(0x3b, 0x02)`, `(0x4d, 0x04)` and `(0x57, 0x15)` — each
    /// on one save flag, and returns 0 everywhere else. `FUN_0042b770` then
    /// **replaces the last character** of the `[EndRoll]`'s path with `A` or
    /// `B` (`erase(len - 1, 1)` at `0x0042c222`, then the literal at
    /// `0x0048ed00` or `0x0048ecfc`).
    ///
    /// The shipped data agrees from the other side. Exactly three end rolls
    /// ship as a pair — `03-M3-B00-ENDA`/`B`, `04-K2-A07-END1A`/`B` and
    /// `04-L8-E04-ENDA`/`B` — and the three scripts whose `[EndRoll]` names
    /// them sit at exactly those three positions. (A fourth pair,
    /// `03-K4-B00-ENDA`/`B`, is named by two *different* scripts, one each,
    /// so the branch graph picks between those and this export is not
    /// involved.)
    pub fn end_roll_select(&self, route: usize, scene: u16, cx: &dyn Context) -> Option<bool> {
        let entry = self.endings.select?;
        self.ending(entry, route, scene, cx, &|l| {
            // The arms that answer store first and return 1; the fall-through
            // returns 0 and stores nothing.
            if l.returns != Some(1) {
                return None;
            }
            l.effects
                .iter()
                .rev()
                .find_map(|e| match (e.store, e.args.first()) {
                    (Some("out"), Some(&Val::Imm(n))) => Some(n != 0),
                    _ => None,
                })
        })
        .flatten()
    }

    /// Whether the ending card over the end roll is swapped for the one in
    /// the `Ex01` pack.
    ///
    /// `_ChangeSubtitle@4` answers 1 at one position only, `(0x33, 0x1d)` —
    /// the script `03/03-K2-F01` — and only when the save flag `894` is set.
    /// `Ex01` holds exactly one such asset, `System/EndRoll/03-K2-F01-END.png`,
    /// for exactly that script.
    ///
    /// **What the executable does with the answer is only half recovered.**
    /// `FUN_0042b770` builds a second clip from the literal `L"Ex01/"` and
    /// runs it from the `[EndRoll]`'s start for a length chosen by host slot
    /// `+0x134` (`FUN_0041dac0`, a float at the engine's `+0x594`): 0x2d0
    /// frames at 24.0, 0x168 at 12.0 and 0x90 otherwise. Neither how the rest
    /// of that path is built nor what the float is has been recovered, and
    /// the arm is also reached when the engine's own `+0x38c` is set, which
    /// has not been recovered either — so nothing acts on this answer yet.
    pub fn change_subtitle(&self, route: usize, scene: u16, cx: &dyn Context) -> bool {
        let Some(entry) = self.endings.subtitle else {
            return false;
        };
        self.ending(entry, route, scene, cx, &|l| l.returns)
            .flatten()
            == Some(1)
    }

    /// Runs one of the ending exports for one position.
    ///
    /// They differ from a route handler in taking the host as their first
    /// argument rather than their third, and in switching on `ROUTE` as well
    /// as `SCENE`, so both are seeded. `0xc` is the out-parameter
    /// `_CheckEndRollSelect@8` answers through; the other two ignore it.
    fn ending<L>(
        &self,
        entry: u32,
        route: usize,
        scene: u16,
        cx: &dyn Context,
        leaf: &dyn Fn(&Leaf) -> L,
    ) -> Option<L> {
        let raw = walk::run(
            &self.image(),
            entry,
            &[("ROUTE", route as i32), ("SCENE", scene as i32)],
            &[(8, Val::Host), (0xc, Val::Arg("out"))],
        );
        let tree = self.build(&raw, route, scene, Vec::new(), leaf);
        self.decide(tree, route, scene, cx).map(|(_, l)| l)
    }

    /// Walks a decision tree, answering each question from the context.
    fn decide<L>(
        &self,
        mut step: Step<L>,
        route: usize,
        scene: u16,
        cx: &dyn Context,
    ) -> Option<(Vec<Act>, L)> {
        loop {
            match step {
                Step::Unrecovered(why) => {
                    log::warn!("route {route} scene {scene}: {why}");
                    return None;
                }
                Step::Do { acts, next } => return Some((acts, next)),
                Step::If {
                    lhs,
                    cmp,
                    rhs,
                    then,
                    els,
                } => {
                    let (a, b) = (self.value(&lhs, cx), self.value(&rhs, cx));
                    step = if cmp.holds(a, b) { *then } else { *els };
                }
            }
        }
    }

    fn value(&self, t: &Term, cx: &dyn Context) -> i32 {
        match t {
            Term::Const(n) => *n,
            Term::Choice => cx.choice(),
            Term::Int(n) => cx.int(n),
            Term::Flag(n) => cx.flag(n) as i32,
            Term::GlobalFlag(n) => cx.global_flag(n) as i32,
            Term::Threshold(s) => cx.threshold(s) as i32,
            Term::HostSlot(s) => cx.host_slot(*s),
            Term::DllWord(a) => self.dll_word(*a),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny assembler, enough to hand-write the shapes the DLL's handlers
    /// are made of. Labels are resolved on `finish`.
    #[derive(Default)]
    struct Asm {
        code: Vec<u8>,
        base: u32,
        labels: std::collections::HashMap<&'static str, u32>,
        fixups: Vec<(usize, &'static str)>,
    }

    impl Asm {
        fn at(&self) -> u32 {
            self.base + self.code.len() as u32
        }
        fn label(&mut self, name: &'static str) {
            let at = self.at();
            self.labels.insert(name, at);
        }
        fn put(&mut self, b: &[u8]) -> &mut Self {
            self.code.extend_from_slice(b);
            self
        }
        fn imm32(&mut self, v: u32) -> &mut Self {
            self.put(&v.to_le_bytes())
        }
        /// `mov reg, [ebp+disp]`
        fn load_arg(&mut self, reg: u8, disp: i8) -> &mut Self {
            self.put(&[0x8b, 0x45 | (reg << 3), disp as u8])
        }
        /// `mov [ebp+disp], eax`
        fn store(&mut self, disp: i8) -> &mut Self {
            self.put(&[0x89, 0x45, disp as u8])
        }
        fn push_reg(&mut self, reg: u8) -> &mut Self {
            self.put(&[0x50 + reg])
        }
        fn push_imm8(&mut self, n: i8) -> &mut Self {
            self.put(&[0x6a, n as u8])
        }
        fn push_imm32(&mut self, n: u32) -> &mut Self {
            self.put(&[0x68]).imm32(n)
        }
        /// `host->slot(...)`, with the host at `[ebp+host]`.
        fn vcall(&mut self, host: i8, slot: u8) -> &mut Self {
            self.load_arg(0, host) // mov eax,[ebp+host]
                .put(&[0x8b, 0x10]) // mov edx,[eax]
                .load_arg(1, host) // mov ecx,[ebp+host]  (this)
                .put(&[0x8b, 0x42, slot]) // mov eax,[edx+slot]
                .put(&[0xff, 0xd0]) // call eax
        }
        fn call(&mut self, to: &'static str) -> &mut Self {
            self.put(&[0xe8]);
            let at = self.code.len();
            self.fixups.push((at, to));
            self.imm32(0)
        }
        fn jcc(&mut self, cc: u8, to: &'static str) -> &mut Self {
            self.put(&[0x0f, 0x80 | cc]);
            let at = self.code.len();
            self.fixups.push((at, to));
            self.imm32(0)
        }
        /// `cmp dword [ebp+disp], imm8`
        fn cmp_local(&mut self, disp: i8, n: i8) -> &mut Self {
            self.put(&[0x83, 0x7d, disp as u8, n as u8])
        }
        fn ret(&mut self) -> &mut Self {
            self.put(&[0xc9, 0xc3]) // leave; ret
        }
        fn mov_eax(&mut self, n: u32) -> &mut Self {
            self.put(&[0xb8]).imm32(n)
        }
        fn prologue(&mut self) -> &mut Self {
            self.put(&[0x55, 0x8b, 0xec, 0x83, 0xec, 0x10])
        }
        fn finish(mut self) -> Vec<u8> {
            for (at, name) in std::mem::take(&mut self.fixups) {
                let target = self.labels[name];
                let next = self.base + at as u32 + 4;
                self.code[at..at + 4].copy_from_slice(&target.wrapping_sub(next).to_le_bytes());
            }
            self.code
        }
    }

    fn wide(s: &str) -> Vec<u8> {
        s.encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    /// A PE32 image with one section at `va`, carrying `data` and exporting
    /// `_GetNextScriptFile@12` at `entry`.
    fn image(va: u32, data: &[u8], exports_at: u32) -> Vec<u8> {
        const PE: usize = 0x80;
        const OPT: usize = 0xe0;
        const RAW: u32 = 0x400;
        let mut v = vec![0u8; RAW as usize];
        v[0..2].copy_from_slice(b"MZ");
        v[0x3c..0x40].copy_from_slice(&(PE as u32).to_le_bytes());
        v[PE..PE + 4].copy_from_slice(b"PE\0\0");
        v[PE + 6..PE + 8].copy_from_slice(&1u16.to_le_bytes());
        v[PE + 20..PE + 22].copy_from_slice(&(OPT as u16).to_le_bytes());
        v[PE + 24..PE + 26].copy_from_slice(&0x10bu16.to_le_bytes());
        v[PE + 24 + 28..PE + 24 + 32].copy_from_slice(&0u32.to_le_bytes());
        v[PE + 24 + 92..PE + 24 + 96].copy_from_slice(&1u32.to_le_bytes()); // one data directory
        v[PE + 24 + 96..PE + 24 + 100].copy_from_slice(&exports_at.to_le_bytes());
        let h = PE + 24 + OPT;
        v[h + 8..h + 12].copy_from_slice(&(data.len() as u32).to_le_bytes());
        v[h + 12..h + 16].copy_from_slice(&va.to_le_bytes());
        v[h + 16..h + 20].copy_from_slice(&(data.len() as u32).to_le_bytes());
        v[h + 20..h + 24].copy_from_slice(&RAW.to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    /// A two-route DLL shaped the way the retail one is: an export that
    /// switches on `ROUTE`, one handler each, one emitter each, and a name
    /// table each.
    ///
    /// Route 0 runs `0 -> 1`, then forks on the choice at scene 1 — choice 0
    /// stays in route 0, anything else hands over to route 1 through route
    /// 1's own emitter, which is how a route transition is spelled. Route 1
    /// forks on a save counter.
    fn two_route_dll() -> Vec<u8> {
        const VA: u32 = 0x1000;
        let mut blob: Vec<u8> = Vec::new();
        let put = |b: &[u8], blob: &mut Vec<u8>| {
            let at = VA + blob.len() as u32;
            blob.extend_from_slice(b);
            while !blob.len().is_multiple_of(4) {
                blob.push(0);
            }
            at
        };
        let s_route = put(&wide("ROUTE"), &mut blob);
        let s_scene = put(&wide("SCENE"), &mut blob);
        let s_bs = put(&wide("BS0000B00"), &mut blob);
        let names0 = ["00/00-00-A00", "00/00-00-B00", "00/00-00-C00"];
        let names1 = ["01/01-00-A00", "01/01-00-B00"];
        let p0: Vec<u32> = names0.iter().map(|n| put(&wide(n), &mut blob)).collect();
        let p1: Vec<u32> = names1.iter().map(|n| put(&wide(n), &mut blob)).collect();
        let table0 = put(
            &p0.iter().flat_map(|p| p.to_le_bytes()).collect::<Vec<_>>(),
            &mut blob,
        );
        put(&0u32.to_le_bytes(), &mut blob); // a gap, so the runs do not merge
        let table1 = put(
            &p1.iter().flat_map(|p| p.to_le_bytes()).collect::<Vec<_>>(),
            &mut blob,
        );
        put(&0u32.to_le_bytes(), &mut blob);

        // The export directory, and the dispatch jump table. Both need
        // addresses the code has not been laid out at yet, so reserve the
        // space and fill it in once the code is assembled.
        let jump_table = put(&[0u8; 8], &mut blob);
        let exports = put(&[0u8; 0x40], &mut blob);
        let s_first = put(&wide("001"), &mut blob);
        let s_second = put(&wide("002"), &mut blob);

        // Everything the code refers to is placed above: the assembler fixes
        // labels up against the address it was created at, so nothing may be
        // appended to `blob` from here until the code is laid down.
        let mut a = Asm {
            base: VA + blob.len() as u32,
            ..Default::default()
        };
        // ---- the export: switch (ROUTE) ----
        a.label("entry");
        a.prologue();
        a.push_imm32(s_route);
        a.vcall(8, 8); // get_int("ROUTE")
        a.store(-4);
        a.cmp_local(-4, 1); // cmp [ebp-4], 1
        a.jcc(0x7, "fallthrough"); // ja default
        a.put(&[0x8b, 0x55, 0xfc]); // mov edx,[ebp-4]
        a.put(&[0xff, 0x24, 0x95]).imm32(jump_table);
        a.label("arm0");
        a.load_arg(0, 8).push_reg(0);
        a.load_arg(0, 0x10).push_reg(0);
        a.load_arg(0, 0xc).push_reg(0);
        a.call("handler0");
        a.put(&[0x83, 0xc4, 0x0c]);
        a.ret();
        a.label("arm1");
        a.load_arg(0, 8).push_reg(0);
        a.load_arg(0, 0x10).push_reg(0);
        a.load_arg(0, 0xc).push_reg(0);
        a.call("handler1");
        a.put(&[0x83, 0xc4, 0x0c]);
        a.ret();
        a.label("fallthrough");
        a.mov_eax(0).ret();

        // ---- handler 0: (buf, size, host) ----
        a.label("handler0");
        a.prologue();
        a.push_imm32(s_scene);
        a.vcall(0x10, 8); // get_int("SCENE")
        a.store(-4);
        a.cmp_local(-4, 0);
        a.jcc(0x5, "h0_s1"); // jne
        emit(&mut a, "emit0", 1);
        a.mov_eax(1).ret();
        a.label("h0_s1");
        a.cmp_local(-4, 1);
        a.jcc(0x5, "h0_out"); // jne
        a.vcall(0x10, 4); // choice()
        a.put(&[0x85, 0xc0]); // test eax,eax
        a.jcc(0x5, "h0_s1_other"); // jne
        emit(&mut a, "emit0", 2);
        a.mov_eax(1).ret();
        a.label("h0_s1_other");
        // set_int("BS0000B00", 1), then hand over to route 1
        a.push_imm8(1).push_imm32(s_bs);
        a.vcall(0x10, 0xc);
        emit(&mut a, "emit1", 0);
        a.mov_eax(1).ret();
        a.label("h0_out");
        a.mov_eax(0).ret();

        // ---- handler 1: forks on a save counter ----
        a.label("handler1");
        a.prologue();
        a.push_imm32(s_scene);
        a.vcall(0x10, 8);
        a.store(-4);
        a.cmp_local(-4, 0);
        a.jcc(0x5, "h1_out");
        a.push_imm32(s_first);
        a.vcall(0x10, 8);
        a.store(-8);
        a.push_imm32(s_second);
        a.vcall(0x10, 8);
        a.put(&[0x8b, 0x4d, 0xf8]); // mov ecx,[ebp-8]   (001)
        a.put(&[0x3b, 0xc8]); // cmp ecx,eax       (001 vs 002)
        a.jcc(0xe, "h1_behind"); // jle
        emit(&mut a, "emit1", 1);
        a.mov_eax(1).ret();
        a.label("h1_behind");
        emit(&mut a, "emit1", 0);
        a.mov_eax(1).ret();
        a.label("h1_out");
        a.mov_eax(0).ret();

        // ---- the two emitters: (host, buf, size, scene, flag) ----
        for (label, route, table) in [("emit0", 0u32, table0), ("emit1", 1, table1)] {
            a.label(label);
            a.prologue();
            a.push_imm8(route as i8).push_imm32(s_route);
            a.vcall(8, 0xc); // set_int("ROUTE", route)
            a.load_arg(0, 0x14).push_reg(0).push_imm32(s_scene);
            a.vcall(8, 0xc); // set_int("SCENE", scene)
            a.load_arg(0, 0x14);
            a.put(&[0x8b, 0x04, 0x85]).imm32(table); // mov eax,[table + eax*4]
            a.ret();
        }

        let labels = a.labels.clone();
        blob.extend_from_slice(&a.finish());

        // Fill in the jump table and the export directory now that the code
        // has addresses.
        let off = |va: u32| (va - VA) as usize;
        let arms = [labels["arm0"], labels["arm1"]];
        for (i, arm) in arms.iter().enumerate() {
            blob[off(jump_table) + i * 4..off(jump_table) + i * 4 + 4]
                .copy_from_slice(&arm.to_le_bytes());
        }
        let name_at = VA + blob.len() as u32;
        blob.extend_from_slice(b"_GetNextScriptFile@12\0");
        let fn_table = VA + blob.len() as u32;
        blob.extend_from_slice(&labels["entry"].to_le_bytes());
        let name_table = VA + blob.len() as u32;
        blob.extend_from_slice(&name_at.to_le_bytes());
        let ord_table = VA + blob.len() as u32;
        blob.extend_from_slice(&0u16.to_le_bytes());
        let d = off(exports);
        blob[d + 20..d + 24].copy_from_slice(&1u32.to_le_bytes()); // NumberOfFunctions
        blob[d + 24..d + 28].copy_from_slice(&1u32.to_le_bytes()); // NumberOfNames
        blob[d + 28..d + 32].copy_from_slice(&fn_table.to_le_bytes());
        blob[d + 32..d + 36].copy_from_slice(&name_table.to_le_bytes());
        blob[d + 36..d + 40].copy_from_slice(&ord_table.to_le_bytes());

        image(VA, &blob, exports)
    }

    /// `emit(host, buf, size, scene, 1)`, the way an arm calls it.
    fn emit(a: &mut Asm, which: &'static str, scene: i8) {
        a.push_imm8(1);
        a.push_imm8(scene);
        a.load_arg(0, 0xc).push_reg(0); // size
        a.load_arg(0, 8).push_reg(0); // buf
        a.load_arg(0, 0x10).push_reg(0); // host
        a.call(which);
        a.put(&[0x83, 0xc4, 0x14]);
    }

    #[derive(Default)]
    struct Cx {
        choice: i32,
        ints: std::collections::HashMap<String, i32>,
    }

    impl Context for Cx {
        fn choice(&self) -> i32 {
            self.choice
        }
        fn int(&self, name: &str) -> i32 {
            self.ints.get(name).copied().unwrap_or(0)
        }
        fn flag(&self, _: &str) -> bool {
            false
        }
        fn global_flag(&self, _: &str) -> bool {
            false
        }
        fn threshold(&self, _: &str) -> bool {
            false
        }
    }

    #[test]
    fn finds_both_handlers_through_the_exports_own_switch() {
        let m = Machine::recover(&two_route_dll()).expect("recovers");
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn a_linear_scene_moves_to_the_next_one() {
        let m = Machine::recover(&two_route_dll()).unwrap();
        assert_eq!(
            m.step(0, 0),
            Some(Step::Do {
                acts: vec![],
                next: Next::Scene { route: 0, scene: 1 },
            })
        );
    }

    #[test]
    fn a_choice_forks_and_the_second_arm_crosses_into_another_route() {
        let m = Machine::recover(&two_route_dll()).unwrap();
        let cx = Cx {
            choice: 0,
            ..Default::default()
        };
        assert_eq!(
            m.next(0, 1, &cx),
            Some((vec![], Next::Scene { route: 0, scene: 2 }))
        );

        let cx = Cx {
            choice: 1,
            ..Default::default()
        };
        // The other arm records the back-bookmark and emits through route 1's
        // emitter, so the destination route comes out of the emitter itself.
        assert_eq!(
            m.next(0, 1, &cx),
            Some((
                vec![Act::SetInt("BS0000B00".into(), 1)],
                Next::Scene { route: 1, scene: 0 }
            ))
        );
    }

    #[test]
    fn a_counter_comparison_reads_both_counters_from_the_context() {
        let m = Machine::recover(&two_route_dll()).unwrap();
        let ahead = Cx {
            ints: [("001".into(), 69), ("002".into(), 62)].into(),
            ..Default::default()
        };
        assert_eq!(
            m.next(1, 0, &ahead),
            Some((vec![], Next::Scene { route: 1, scene: 1 }))
        );
        let behind = Cx {
            ints: [("001".into(), 62), ("002".into(), 69)].into(),
            ..Default::default()
        };
        assert_eq!(
            m.next(1, 0, &behind),
            Some((vec![], Next::Scene { route: 1, scene: 0 }))
        );
        // A tie takes the same arm as being behind: the comparison is a
        // strict one.
        let level = Cx {
            ints: [("001".into(), 5), ("002".into(), 5)].into(),
            ..Default::default()
        };
        assert_eq!(
            m.next(1, 0, &level),
            Some((vec![], Next::Scene { route: 1, scene: 0 }))
        );
    }

    #[test]
    fn a_scene_no_arm_names_falls_through() {
        let m = Machine::recover(&two_route_dll()).unwrap();
        assert_eq!(
            m.step(0, 9),
            Some(Step::Do {
                acts: vec![],
                next: Next::Nothing
            })
        );
    }
}
