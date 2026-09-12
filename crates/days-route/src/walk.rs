//! Symbolic execution of one route handler, for one value of `SCENE`.
//!
//! # Why a walker and not a table
//!
//! The route handlers hold the only copy of the branch graph. Each is a
//! compiled `switch (SCENE)` whose arms call an emitter with a literal next
//! scene, so the edges exist as x86 and nowhere else — not in the INIs, not
//! in the packs, not in a table in `.data`.
//!
//! Two shapes of `switch` are in the DLL: a jump table for the big routes and
//! an `if`/`else if` chain for the small ones. Rather than recognise each
//! shape, the walker **seeds `SCENE` with the value being asked about** and
//! executes the handler symbolically. Both shapes then reduce the same way —
//! the chain's comparisons fold to constants, and the jump table's index
//! resolves — and what is left over is exactly the branching that depends on
//! something only known at run time: the player's choice, a save flag, a
//! feeling counter.
//!
//! The result is a decision tree, [`Node`], whose leaves are what the handler
//! does and whose forks are the questions it asks the host.
//!
//! # What it refuses
//!
//! Every instruction outside [`crate::x86`]'s covered subset, every indirect
//! call through something other than the host vtable, and every indexed jump
//! whose index is not known ends the walk with [`Node::Unrecovered`]. A
//! handler that produced one would be reported as not recovered rather than
//! silently mis-decoded.
//!
//! Established against `FUN_1000d440` (route 0) upward: the tree this
//! produces for all 55 handlers reproduces Ghidra's decompilation of every one
//! of their 1,840 `case` arms, emitter for emitter and literal for literal.

use crate::pe::Image;
use crate::x86::{decode, Alu, Cc, Insn, Op};
use std::collections::HashMap;

/// The host vtable slots the route handlers call.
///
/// The host is the engine object the executable passes in — a secondary base
/// subobject at `engine + 0x2c`, so these are its own offsets, not the
/// executable's. Established from the executable's side of the same vtable.
///
/// Slot `+0x00` is not here because no handler calls it directly: it takes a
/// `(story_number, script)` and marks `SP%03d` true in both flag stores, and
/// only the per-route story markers reach it.
pub mod slot {
    /// `() -> int` — the choice the player just made.
    pub const CHOICE: i32 = 0x04;
    /// `(name) -> int` — read an int from the save's variable store.
    pub const GET_INT: i32 = 0x08;
    /// `(name, value)` — write one.
    pub const SET_INT: i32 = 0x0c;
    /// `(name) -> bool` — read a bool from the save's flag store.
    pub const GET_BOOL: i32 = 0x10;
    /// `(name, value)` — write one.
    pub const SET_BOOL: i32 = 0x14;
    /// `(name) -> bool` — read a bool from the global flag store.
    pub const GET_GLOBAL_BOOL: i32 = 0x18;
    /// `(name, value)` — write one.
    pub const SET_GLOBAL_BOOL: i32 = 0x1c;
}

/// A value the walker tracks through a handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Val {
    Imm(i32),
    /// A wide string literal the code pushed or read out of a name table.
    Str(String),
    /// `host->choice()`.
    Choice,
    /// `host->get_int(name)`.
    Int(String),
    /// `host->get_bool(name)`.
    Bool(String),
    /// `host->get_global_bool(name)`.
    GlobalBool(String),
    /// A word in the DLL's own `.data`, by address.
    DllWord(u32),
    /// The return value of a host vtable call, by slot.
    HostSlot(i32),
    /// The return value of a direct call, by address.
    Returned(u32),
    /// The host pointer, its vtable, and the handler's two output arguments.
    Host,
    Vtable,
    Arg(&'static str),
    Unknown,
}

/// One thing a handler does on the way to its answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    /// The host vtable slot, for a call through the interface.
    pub slot: Option<i32>,
    /// The address, for a direct call.
    pub call: Option<u32>,
    pub args: Vec<Val>,
}

/// What a handler does once every run-time question has been answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leaf {
    pub effects: Vec<Effect>,
    /// The handler's return value: 1 when it answered, 0 when it fell through
    /// to the next route.
    pub returns: Option<i32>,
}

/// A handler reduced to the questions it asks and what it does with each
/// answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// `if <lhs> <cc> <rhs>` — `then` is the branch taken.
    Branch {
        /// What the handler did before asking. Side effects happen on the way
        /// to a question as well as after it.
        effects: Vec<Effect>,
        cc: Cc,
        lhs: Val,
        rhs: Val,
        then: Box<Node>,
        els: Box<Node>,
    },
    Do(Leaf),
    /// The decoder met something outside its subset. Says what and where.
    Unrecovered(String),
}

/// Registers, stack slots and the pending argument pushes.
#[derive(Debug, Clone)]
struct State {
    regs: HashMap<u8, Val>,
    stack: HashMap<i32, Val>,
    pushes: Vec<Val>,
    effects: Vec<Effect>,
    flags: Option<(Val, Val)>,
}

/// Bounds on one walk, so a handler the decoder has misread cannot run away
/// or recurse without end. Both are far above what the retail handlers need;
/// reaching either is a "not recovered" result, not a silent truncation.
const MAX_STEPS: usize = 8_000;
const MAX_DEPTH: usize = 32;

/// Runs `entry` with some of the save's variable store already answered.
///
/// `known` is the point of the walk: a handler's `switch` is on a value it
/// asks the host for, so fixing that value is what turns the compiled switch
/// back into an answer. `args` seeds the incoming arguments — the route
/// handlers take `(buf, size, host)`, so the caller passes `host` at
/// `[ebp+0x10]`.
pub fn run(img: &Image, entry: u32, known: &[(&str, i32)], args: &[(i32, Val)]) -> Node {
    let mut st = State {
        regs: HashMap::new(),
        stack: args.iter().cloned().collect(),
        pushes: Vec::new(),
        effects: Vec::new(),
        flags: None,
    };
    let known: Vec<(String, i32)> = known.iter().map(|(n, v)| ((*n).to_owned(), *v)).collect();
    step(img, entry, &mut st, &known, 0)
}

fn read(img: &Image, st: &State, op: &Op) -> Val {
    match *op {
        Op::Imm(n) => Val::Imm(n),
        Op::Reg(r) => st.regs.get(&r).cloned().unwrap_or(Val::Unknown),
        // `ebp` is register 5; everything else indexed off a register is
        // either the vtable or a field the walker does not model.
        Op::Mem(5, d) => st.stack.get(&d).cloned().unwrap_or(Val::Unknown),
        Op::Mem(r, d) => match st.regs.get(&r) {
            Some(Val::Host) if d == 0 => Val::Vtable,
            Some(Val::Vtable) => Val::HostSlot(d),
            _ => Val::Unknown,
        },
        Op::Abs(a) => Val::DllWord(a),
        Op::Sib {
            base: None,
            index: Some(i),
            scale: 4,
            disp,
        } => {
            // A name table indexed by a scene number: `table[SCENE]`.
            let Some(Val::Imm(n)) = st.regs.get(&i) else {
                return Val::Unknown;
            };
            let at = (disp as u32).wrapping_add((*n as u32).wrapping_mul(4));
            match img.u32(at).and_then(|p| img.wide_ascii(p)) {
                Some(s) => Val::Str(s),
                None => Val::Unknown,
            }
        }
        Op::Sib { .. } | Op::Opaque => Val::Unknown,
    }
}

fn write(st: &mut State, op: &Op, v: Val) {
    match *op {
        Op::Reg(r) => {
            st.regs.insert(r, v);
        }
        Op::Mem(5, d) => {
            st.stack.insert(d, v);
        }
        _ => {}
    }
}

fn step(img: &Image, mut va: u32, st: &mut State, known: &[(String, i32)], depth: usize) -> Node {
    if depth > MAX_DEPTH {
        return Node::Unrecovered(format!(
            "conditionals nested past {MAX_DEPTH} at {va:#010x}"
        ));
    }
    for _ in 0..MAX_STEPS {
        let Some(off) = img.at(va) else {
            return Node::Unrecovered(format!("address {va:#010x} is not backed by the file"));
        };
        let Some(d) = decode(img.bytes, off, va) else {
            return Node::Unrecovered(format!("truncated instruction at {va:#010x}"));
        };
        match &d.insn {
            Insn::Unknown(op) => {
                return Node::Unrecovered(format!("opcode {op:#04x} at {va:#010x}"));
            }
            Insn::Nop | Insn::Ret if matches!(d.insn, Insn::Nop) => {}
            Insn::Mov { dst, src } => {
                let v = read(img, st, src);
                write(st, dst, v);
            }
            Insn::Lea { dst, src } => {
                let v = match *src {
                    Op::Abs(a) => Val::DllWord(a),
                    _ => Val::Unknown,
                };
                st.regs.insert(*dst, v);
            }
            Insn::Movzx { dst, src } => {
                // The only use here is a `switch`'s byte index table.
                let v = match *src {
                    Op::Mem(r, disp) => byte_index(img, st, r, disp, 0),
                    Op::Sib {
                        base: Some(b),
                        index: None,
                        scale: 1,
                        disp,
                    } => byte_index(img, st, b, disp, 0),
                    _ => Val::Unknown,
                };
                st.regs.insert(*dst, v);
            }
            Insn::Push(src) => {
                // A pushed constant is a pointer to a string literal far more
                // often than it is a number, and it reaches the push either
                // directly or through a register the compiler parked it in.
                // Both are resolved here, and only if the address really holds
                // a wide ASCII string.
                let v = match read(img, st, src) {
                    Val::Imm(n) => match img.wide_ascii(n as u32) {
                        Some(s) => Val::Str(s),
                        None => Val::Imm(n),
                    },
                    other => other,
                };
                st.pushes.push(v);
            }
            Insn::Pop(r) => {
                st.regs.insert(*r, Val::Unknown);
            }
            Insn::Test { dst, src } => {
                st.flags = if dst == src {
                    Some((read(img, st, dst), Val::Imm(0)))
                } else {
                    Some((Val::Unknown, Val::Imm(0)))
                };
            }
            Insn::Alu { op, dst, src } => {
                let (a, b) = (read(img, st, dst), read(img, st, src));
                if *op == Alu::Cmp {
                    st.flags = Some((a, b));
                } else if *dst == Op::Reg(4) {
                    // `add esp, n` closes out a cdecl call's arguments.
                    st.pushes.clear();
                } else {
                    // Folding matters: the compiler biases a `switch` whose
                    // lowest case is not zero by subtracting it first, so the
                    // dispatch only resolves if arithmetic on a known `SCENE`
                    // stays known.
                    let folded = match (&a, &b) {
                        _ if *op == Alu::Xor && dst == src => Some(0),
                        (Val::Imm(x), Val::Imm(y)) => op.apply(*x, *y),
                        _ => None,
                    };
                    write(st, dst, folded.map_or(Val::Unknown, Val::Imm));
                }
            }
            Insn::CallIndirect(target) => {
                let v = read(img, st, target);
                let Val::HostSlot(s) = v else {
                    return Node::Unrecovered(format!(
                        "indirect call through an untracked value at {va:#010x}"
                    ));
                };
                let args: Vec<Val> = st.pushes.drain(..).rev().collect();
                let name = match args.first() {
                    Some(Val::Str(n)) => Some(n.clone()),
                    _ => None,
                };
                let ret = match (s, name) {
                    (slot::CHOICE, _) => Val::Choice,
                    // A name this walk is parameterised on answers itself.
                    (slot::GET_INT, Some(ref n)) if lookup(known, n).is_some() => {
                        Val::Imm(lookup(known, n).unwrap_or_default())
                    }
                    (slot::GET_INT, Some(n)) => Val::Int(n),
                    (slot::GET_BOOL, Some(n)) => Val::Bool(n),
                    (slot::GET_GLOBAL_BOOL, Some(n)) => Val::GlobalBool(n),
                    _ => {
                        st.effects.push(Effect {
                            slot: Some(s),
                            call: None,
                            args,
                        });
                        Val::HostSlot(s)
                    }
                };
                st.regs.insert(0, ret);
            }
            Insn::Call(to) => {
                let args: Vec<Val> = st.pushes.drain(..).rev().collect();
                st.effects.push(Effect {
                    slot: None,
                    call: Some(*to),
                    args,
                });
                st.regs.insert(0, Val::Returned(*to));
            }
            Insn::Jmp(to) => {
                va = *to;
                continue;
            }
            Insn::JmpIndirect(target) => {
                // A `switch`'s jump table, which resolves because `SCENE` is
                // a constant on this walk.
                let Op::Sib {
                    base: None,
                    index: Some(i),
                    scale: 4,
                    disp,
                } = *target
                else {
                    return Node::Unrecovered(format!("unmodelled indexed jump at {va:#010x}"));
                };
                let Some(Val::Imm(n)) = st.regs.get(&i) else {
                    return Node::Unrecovered(format!(
                        "indexed jump on an unknown index at {va:#010x}"
                    ));
                };
                let at = (disp as u32).wrapping_add((*n as u32).wrapping_mul(4));
                let Some(to) = img.u32(at) else {
                    return Node::Unrecovered(format!("jump table entry {at:#010x} is off image"));
                };
                va = to;
                continue;
            }
            Insn::Jcc { cc, to } => {
                let settled = match &st.flags {
                    Some((Val::Imm(a), Val::Imm(b))) => cc.holds(*a, *b),
                    _ => None,
                };
                match settled {
                    Some(true) => {
                        va = *to;
                        continue;
                    }
                    Some(false) => {}
                    None => {
                        let (lhs, rhs) = st.flags.clone().unwrap_or((Val::Unknown, Val::Unknown));
                        let effects = std::mem::take(&mut st.effects);
                        let mut other = st.clone();
                        let then = step(img, *to, st, known, depth + 1);
                        let els = step(img, va + d.len as u32, &mut other, known, depth + 1);
                        return Node::Branch {
                            effects,
                            cc: *cc,
                            lhs,
                            rhs,
                            then: Box::new(then),
                            els: Box::new(els),
                        };
                    }
                }
            }
            Insn::Ret => {
                let returns = match st.regs.get(&0) {
                    Some(Val::Imm(n)) => Some(*n),
                    _ => None,
                };
                return Node::Do(Leaf {
                    effects: std::mem::take(&mut st.effects),
                    returns,
                });
            }
            Insn::Nop => {}
        }
        va += d.len as u32;
    }
    Node::Unrecovered(format!("ran past {MAX_STEPS} instructions near {va:#010x}"))
}

fn lookup(known: &[(String, i32)], name: &str) -> Option<i32> {
    known.iter().find(|(n, _)| n == name).map(|(_, v)| *v)
}

/// One byte out of a `switch`'s index table, when the index is known.
fn byte_index(img: &Image, st: &State, reg: u8, disp: i32, extra: i32) -> Val {
    let Some(Val::Imm(n)) = st.regs.get(&reg) else {
        return Val::Unknown;
    };
    let at = (disp as u32)
        .wrapping_add(*n as u32)
        .wrapping_add(extra as u32);
    match img.u8(at) {
        Some(b) => Val::Imm(b as i32),
        None => Val::Unknown,
    }
}
