//! The slice of 32-bit x86 that `RouteProcSDHQ.dll`'s route handlers are made
//! of.
//!
//! This is not a general disassembler and must not become one. The route
//! handlers are unoptimised MSVC output — a `switch` over `SCENE`, calls
//! through the host vtable, comparisons against small constants — and the
//! decoder covers exactly the forms that appear there. Anything else is an
//! [`Insn::Unknown`], which the walker above turns into a "not recovered"
//! result rather than a guess.
//!
//! Refusing to decode is the point: a decoder that muddled through an
//! instruction it did not understand would produce a branch graph that looked
//! complete and was wrong.

/// A memory or register operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Reg(u8),
    /// `[reg + disp]`
    Mem(u8, i32),
    /// `[disp]`, an absolute address.
    Abs(u32),
    /// `[base + index*scale + disp]`, with `base` absent for the jump-table
    /// form the compiler emits.
    Sib {
        base: Option<u8>,
        index: Option<u8>,
        scale: u8,
        disp: i32,
    },
    Imm(i32),
    /// A value the decoder does not track. Reading one yields an unknown; it
    /// never compares equal to anything the walker can fold.
    Opaque,
}

/// The arithmetic and logic group the `0x80`-series opcodes select from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alu {
    Add,
    Or,
    Adc,
    Sbb,
    And,
    Sub,
    Xor,
    Cmp,
}

/// A decoded condition, named for the jump that carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cc {
    E,
    Ne,
    L,
    Ge,
    Le,
    G,
    B,
    Ae,
    Be,
    A,
    S,
    Ns,
    Other,
}

impl Alu {
    /// The result, when both operands are known. `Adc` and `Sbb` are not
    /// folded: they read a carry the decoder does not track.
    pub fn apply(self, a: i32, b: i32) -> Option<i32> {
        Some(match self {
            Alu::Add => a.wrapping_add(b),
            Alu::Sub => a.wrapping_sub(b),
            Alu::And => a & b,
            Alu::Or => a | b,
            Alu::Xor => a ^ b,
            Alu::Cmp | Alu::Adc | Alu::Sbb => return None,
        })
    }
}

impl Cc {
    fn from_low(n: u8) -> Cc {
        match n {
            0x2 => Cc::B,
            0x3 => Cc::Ae,
            0x4 => Cc::E,
            0x5 => Cc::Ne,
            0x6 => Cc::Be,
            0x7 => Cc::A,
            0x8 => Cc::S,
            0x9 => Cc::Ns,
            0xc => Cc::L,
            0xd => Cc::Ge,
            0xe => Cc::Le,
            0xf => Cc::G,
            _ => Cc::Other,
        }
    }

    /// Whether the jump is taken when comparing two known values.
    pub fn holds(self, a: i32, b: i32) -> Option<bool> {
        let (ua, ub) = (a as u32, b as u32);
        Some(match self {
            Cc::E => a == b,
            Cc::Ne => a != b,
            Cc::L => a < b,
            Cc::Ge => a >= b,
            Cc::Le => a <= b,
            Cc::G => a > b,
            Cc::B => ua < ub,
            Cc::Ae => ua >= ub,
            Cc::Be => ua <= ub,
            Cc::A => ua > ub,
            Cc::S => a.wrapping_sub(b) < 0,
            Cc::Ns => a.wrapping_sub(b) >= 0,
            Cc::Other => return None,
        })
    }
}

/// One instruction, in the terms the walker reasons about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Insn {
    Mov {
        dst: Op,
        src: Op,
    },
    /// `movzx` — only ever used here to index a `switch`'s byte table.
    Movzx {
        dst: u8,
        src: Op,
    },
    Lea {
        dst: u8,
        src: Op,
    },
    Alu {
        op: Alu,
        dst: Op,
        src: Op,
    },
    Test {
        dst: Op,
        src: Op,
    },
    Push(Op),
    Pop(u8),
    /// `call rel32`.
    Call(u32),
    /// `call r/m32` — a host vtable call.
    CallIndirect(Op),
    Jmp(u32),
    /// `jmp r/m32` — a `switch`'s indexed jump.
    JmpIndirect(Op),
    Jcc {
        cc: Cc,
        to: u32,
    },
    Ret,
    /// Something with no effect the walker needs to model.
    Nop,
    /// Not in the covered subset. The walker refuses to continue past one.
    Unknown(u8),
}

/// A decoded instruction and the bytes it occupied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub insn: Insn,
    pub len: usize,
}

fn i8at(b: &[u8], o: usize) -> Option<i32> {
    Some(*b.get(o)? as i8 as i32)
}

fn i32at(b: &[u8], o: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// Decodes a ModR/M byte and whatever displacement follows it.
///
/// Returns the `reg` field — which is either a register or an opcode
/// extension — the operand the `r/m` field names, and how many bytes the
/// whole thing took.
fn modrm(b: &[u8], o: usize) -> Option<(u8, Op, usize)> {
    let m = *b.get(o)?;
    let (md, reg, rm) = (m >> 6, (m >> 3) & 7, m & 7);
    let mut n = 1;
    if md == 3 {
        return Some((reg, Op::Reg(rm), n));
    }
    if rm == 4 {
        let sib = *b.get(o + n)?;
        n += 1;
        let (scale, index, base) = (1u8 << (sib >> 6), (sib >> 3) & 7, sib & 7);
        let (disp, base) = match md {
            0 if base == 5 => {
                let d = i32at(b, o + n)?;
                n += 4;
                (d, None)
            }
            0 => (0, Some(base)),
            1 => {
                let d = i8at(b, o + n)?;
                n += 1;
                (d, Some(base))
            }
            _ => {
                let d = i32at(b, o + n)?;
                n += 4;
                (d, Some(base))
            }
        };
        let index = (index != 4).then_some(index);
        return Some((
            reg,
            Op::Sib {
                base,
                index,
                scale,
                disp,
            },
            n,
        ));
    }
    let op = match md {
        0 if rm == 5 => {
            let d = i32at(b, o + n)?;
            n += 4;
            Op::Abs(d as u32)
        }
        0 => Op::Mem(rm, 0),
        1 => {
            let d = i8at(b, o + n)?;
            n += 1;
            Op::Mem(rm, d)
        }
        _ => {
            let d = i32at(b, o + n)?;
            n += 4;
            Op::Mem(rm, d)
        }
    };
    Some((reg, op, n))
}

const ALU: [Alu; 8] = [
    Alu::Add,
    Alu::Or,
    Alu::Adc,
    Alu::Sbb,
    Alu::And,
    Alu::Sub,
    Alu::Xor,
    Alu::Cmp,
];

/// Decodes the instruction at file offset `o`, which is loaded at `va`.
///
/// `va` is needed because the relative branches encode their targets against
/// the address of the following instruction.
pub fn decode(b: &[u8], o: usize, va: u32) -> Option<Decoded> {
    let op = *b.get(o)?;
    let d = |insn, len| Some(Decoded { insn, len });
    // A segment override. The only ones here are the `fs:` accesses of the
    // SEH prologue, whose operand is thread-local rather than an address in
    // the image, so the instruction is decoded for its length and its memory
    // operand is discarded.
    if matches!(op, 0x26 | 0x2e | 0x36 | 0x3e | 0x64 | 0x65) {
        let inner = decode(b, o + 1, va)?;
        let blind = |x: Op| match x {
            Op::Abs(_) | Op::Mem(..) | Op::Sib { .. } => Op::Opaque,
            other => other,
        };
        let insn = match inner.insn {
            Insn::Mov { dst, src } => Insn::Mov {
                dst: blind(dst),
                src: blind(src),
            },
            Insn::Push(x) => Insn::Push(blind(x)),
            Insn::Alu { op, dst, src } => Insn::Alu {
                op,
                dst: blind(dst),
                src: blind(src),
            },
            other => other,
        };
        return d(insn, inner.len + 1);
    }
    match op {
        0x0f => {
            let op2 = *b.get(o + 1)?;
            match op2 {
                0x80..=0x8f => {
                    let rel = i32at(b, o + 2)?;
                    d(
                        Insn::Jcc {
                            cc: Cc::from_low(op2 & 0xf),
                            to: va.wrapping_add(6).wrapping_add(rel as u32),
                        },
                        6,
                    )
                }
                0xb6 | 0xb7 => {
                    let (reg, rm, n) = modrm(b, o + 2)?;
                    d(Insn::Movzx { dst: reg, src: rm }, 2 + n)
                }
                0x90..=0x9f => {
                    // setcc: the walker only needs its width and that the
                    // destination becomes unknown.
                    let (_, rm, n) = modrm(b, o + 2)?;
                    d(
                        Insn::Mov {
                            dst: rm,
                            src: Op::Opaque,
                        },
                        2 + n,
                    )
                }
                0xaf => {
                    let (reg, _, n) = modrm(b, o + 2)?;
                    d(
                        Insn::Mov {
                            dst: Op::Reg(reg),
                            src: Op::Opaque,
                        },
                        2 + n,
                    )
                }
                0x1f => {
                    let (_, _, n) = modrm(b, o + 2)?;
                    d(Insn::Nop, 2 + n)
                }
                _ => d(Insn::Unknown(op2), 2),
            }
        }
        0x50..=0x57 => d(Insn::Push(Op::Reg(op - 0x50)), 1),
        0x58..=0x5f => d(Insn::Pop(op - 0x58), 1),
        0x40..=0x4f => d(
            Insn::Alu {
                op: if op < 0x48 { Alu::Add } else { Alu::Sub },
                dst: Op::Reg(op & 7),
                src: Op::Imm(1),
            },
            1,
        ),
        0x6a => d(Insn::Push(Op::Imm(i8at(b, o + 1)?)), 2),
        0x68 => d(Insn::Push(Op::Imm(i32at(b, o + 1)?)), 5),
        0x8b => {
            let (reg, rm, n) = modrm(b, o + 1)?;
            d(
                Insn::Mov {
                    dst: Op::Reg(reg),
                    src: rm,
                },
                1 + n,
            )
        }
        0x89 => {
            let (reg, rm, n) = modrm(b, o + 1)?;
            d(
                Insn::Mov {
                    dst: rm,
                    src: Op::Reg(reg),
                },
                1 + n,
            )
        }
        0x8d => {
            let (reg, rm, n) = modrm(b, o + 1)?;
            d(Insn::Lea { dst: reg, src: rm }, 1 + n)
        }
        0xa1 => d(
            Insn::Mov {
                dst: Op::Reg(0),
                src: Op::Abs(i32at(b, o + 1)? as u32),
            },
            5,
        ),
        0xa3 => d(
            Insn::Mov {
                dst: Op::Abs(i32at(b, o + 1)? as u32),
                src: Op::Reg(0),
            },
            5,
        ),
        0xc7 => {
            let (_, rm, n) = modrm(b, o + 1)?;
            d(
                Insn::Mov {
                    dst: rm,
                    src: Op::Imm(i32at(b, o + 1 + n)?),
                },
                1 + n + 4,
            )
        }
        0xc6 => {
            let (_, rm, n) = modrm(b, o + 1)?;
            d(
                Insn::Mov {
                    dst: rm,
                    src: Op::Imm(i8at(b, o + 1 + n)?),
                },
                1 + n + 1,
            )
        }
        0xb8..=0xbf => d(
            Insn::Mov {
                dst: Op::Reg(op - 0xb8),
                src: Op::Imm(i32at(b, o + 1)?),
            },
            5,
        ),
        0x83 => {
            let (reg, rm, n) = modrm(b, o + 1)?;
            d(
                Insn::Alu {
                    op: ALU[reg as usize],
                    dst: rm,
                    src: Op::Imm(i8at(b, o + 1 + n)?),
                },
                1 + n + 1,
            )
        }
        0x81 => {
            let (reg, rm, n) = modrm(b, o + 1)?;
            d(
                Insn::Alu {
                    op: ALU[reg as usize],
                    dst: rm,
                    src: Op::Imm(i32at(b, o + 1 + n)?),
                },
                1 + n + 4,
            )
        }
        0x01 | 0x09 | 0x11 | 0x19 | 0x21 | 0x29 | 0x31 | 0x39 => {
            let (reg, rm, n) = modrm(b, o + 1)?;
            d(
                Insn::Alu {
                    op: ALU[(op >> 3) as usize],
                    dst: rm,
                    src: Op::Reg(reg),
                },
                1 + n,
            )
        }
        0x03 | 0x0b | 0x13 | 0x1b | 0x23 | 0x2b | 0x33 | 0x3b => {
            let (reg, rm, n) = modrm(b, o + 1)?;
            d(
                Insn::Alu {
                    op: ALU[(op >> 3) as usize],
                    dst: Op::Reg(reg),
                    src: rm,
                },
                1 + n,
            )
        }
        0x05 | 0x0d | 0x15 | 0x1d | 0x25 | 0x2d | 0x35 | 0x3d => d(
            Insn::Alu {
                op: ALU[(op >> 3) as usize],
                dst: Op::Reg(0),
                src: Op::Imm(i32at(b, o + 1)?),
            },
            5,
        ),
        0x85 => {
            let (reg, rm, n) = modrm(b, o + 1)?;
            d(
                Insn::Test {
                    dst: rm,
                    src: Op::Reg(reg),
                },
                1 + n,
            )
        }
        0xe8 => d(
            Insn::Call(va.wrapping_add(5).wrapping_add(i32at(b, o + 1)? as u32)),
            5,
        ),
        0xe9 => d(
            Insn::Jmp(va.wrapping_add(5).wrapping_add(i32at(b, o + 1)? as u32)),
            5,
        ),
        0xeb => d(
            Insn::Jmp(va.wrapping_add(2).wrapping_add(i8at(b, o + 1)? as u32)),
            2,
        ),
        0x70..=0x7f => d(
            Insn::Jcc {
                cc: Cc::from_low(op & 0xf),
                to: va.wrapping_add(2).wrapping_add(i8at(b, o + 1)? as u32),
            },
            2,
        ),
        0xff => {
            let (reg, rm, n) = modrm(b, o + 1)?;
            match reg {
                0 | 1 => d(
                    Insn::Alu {
                        op: if reg == 0 { Alu::Add } else { Alu::Sub },
                        dst: rm,
                        src: Op::Imm(1),
                    },
                    1 + n,
                ),
                2 => d(Insn::CallIndirect(rm), 1 + n),
                4 => d(Insn::JmpIndirect(rm), 1 + n),
                6 => d(Insn::Push(rm), 1 + n),
                _ => d(Insn::Unknown(op), 1 + n),
            }
        }
        0x99 | 0x90 => d(Insn::Nop, 1),
        0xc9 => d(Insn::Nop, 1),
        0xc3 => d(Insn::Ret, 1),
        0xc2 => d(Insn::Ret, 3),
        _ => d(Insn::Unknown(op), 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(bytes: &[u8], va: u32) -> Decoded {
        decode(bytes, 0, va).expect("decodes")
    }

    #[test]
    fn reads_the_forms_a_handler_prologue_is_made_of() {
        // push ebp / mov ebp,esp / sub esp,0xc
        assert_eq!(one(&[0x55], 0).insn, Insn::Push(Op::Reg(5)));
        assert_eq!(
            one(&[0x8b, 0xec], 0).insn,
            Insn::Mov {
                dst: Op::Reg(5),
                src: Op::Reg(4)
            }
        );
        assert_eq!(
            one(&[0x83, 0xec, 0x0c], 0).insn,
            Insn::Alu {
                op: Alu::Sub,
                dst: Op::Reg(4),
                src: Op::Imm(0xc)
            }
        );
        // mov eax,[ebp+0x10] — the host argument
        assert_eq!(
            one(&[0x8b, 0x45, 0x10], 0).insn,
            Insn::Mov {
                dst: Op::Reg(0),
                src: Op::Mem(5, 0x10)
            }
        );
        // mov eax,[edx+8] — a vtable slot
        assert_eq!(
            one(&[0x8b, 0x42, 0x08], 0).insn,
            Insn::Mov {
                dst: Op::Reg(0),
                src: Op::Mem(2, 8)
            }
        );
        assert_eq!(one(&[0xff, 0xd0], 0).insn, Insn::CallIndirect(Op::Reg(0)));
    }

    #[test]
    fn resolves_branch_targets_against_the_following_instruction() {
        assert_eq!(one(&[0xeb, 0x10], 0x1000).insn, Insn::Jmp(0x1012));
        assert_eq!(one(&[0xeb, 0xfe], 0x1000).insn, Insn::Jmp(0x1000));
        assert_eq!(
            one(&[0x74, 0x05], 0x1000).insn,
            Insn::Jcc {
                cc: Cc::E,
                to: 0x1007
            }
        );
        assert_eq!(
            one(&[0x0f, 0x87, 0x2a, 0x04, 0x00, 0x00], 0x1000).insn,
            Insn::Jcc {
                cc: Cc::A,
                to: 0x1430
            }
        );
        assert_eq!(
            one(&[0xe8, 0xb6, 0xfe, 0xff, 0xff], 0x1000).insn,
            Insn::Call(0xebb)
        );
    }

    #[test]
    fn reads_a_switchs_index_table_and_indexed_jump() {
        // movzx edx,[ecx + 0x1000d930]
        assert_eq!(
            one(&[0x0f, 0xb6, 0x91, 0x30, 0xd9, 0x00, 0x10], 0).insn,
            Insn::Movzx {
                dst: 2,
                src: Op::Mem(1, 0x1000d930u32 as i32)
            }
        );
        // jmp dword [0x1000d8d0 + edx*4]
        assert_eq!(
            one(&[0xff, 0x24, 0x95, 0xd0, 0xd8, 0x00, 0x10], 0).insn,
            Insn::JmpIndirect(Op::Sib {
                base: None,
                index: Some(2),
                scale: 4,
                disp: 0x1000d8d0u32 as i32
            })
        );
    }

    /// The SEH prologue is the only place a segment override appears. It has
    /// to decode, or the scan stops at the top of the function and misses
    /// everything below — which is exactly what hid the StanderdScript gate.
    #[test]
    fn decodes_past_the_seh_prologue() {
        // mov eax, fs:[0]
        let d = one(&[0x64, 0xa1, 0x00, 0x00, 0x00, 0x00], 0);
        assert_eq!(d.len, 6);
        assert_eq!(
            d.insn,
            Insn::Mov {
                dst: Op::Reg(0),
                src: Op::Opaque
            }
        );
        // mov fs:[0], esp
        let d = one(&[0x64, 0x89, 0x25, 0x00, 0x00, 0x00, 0x00], 0);
        assert_eq!(d.len, 7);
    }

    #[test]
    fn refuses_what_it_does_not_cover() {
        assert!(matches!(one(&[0xd9, 0x00], 0).insn, Insn::Unknown(0xd9)));
    }

    #[test]
    fn folds_the_conditions_a_known_scene_settles() {
        assert_eq!(Cc::E.holds(3, 3), Some(true));
        assert_eq!(Cc::A.holds(0x14, 0xdc), Some(false));
        assert_eq!(Cc::Le.holds(69, 62), Some(false));
        assert_eq!(Cc::Other.holds(0, 0), None);
    }
}
