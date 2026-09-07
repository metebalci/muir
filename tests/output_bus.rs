// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The output bus select's fourth value.  `IR<13:12>` on an ALU
//! instruction chooses what reaches the output bus: 1 the ALU, 2 and 3 the
//! ALU shifted right and left --- and 0 is not the ALU at all but the mask
//! and rotate network the BYTE class drives, pages SMCTL, SHIFT0-1, MSKG4
//! and MO.  The network reads its rotate from `IR<4:0>` and its byte
//! length from `IR<9:5>` whatever the class, `mit/cadr/ir.bits`; in an ALU
//! instruction those bits are the ALU function, the carry and the Q
//! control, and the network rotates and masks by them all the same.  So
//! the output is the M source rotated left by `IR<4:0>`, under a mask of
//! `IR<9:5>` + 1 bits starting at bit `IR<4:0>`, with the A source
//! outside the mask --- and only when that mask is all ones is it the
//! bare rotate.
//!
//! Microcode 323 has eleven ALU words with this select, every one the
//! all-zero word with no destination, so the boot never sees the
//! difference.  A benchmark program did: `micro` gave the bare rotate
//! where `rtl`, from the drawings, and the netlist, from the wire lists,
//! gave the network's output.  All three are held to it here, with the
//! result written to VMA, which each can be read back from.

use muir::benchmark::{self, Program, Stop};
use muir::engine::Engine;
use muir::isa::Insn;
use muir::isa::asm::{ALU, ALWAYS, JUMP, SETO, SETZ, VMA, a_dest, a_src, filler, m_dest, m_src};
use muir::machine::Machine;
use muir::micro::Micro;
use muir::rtl::Rtl;

mod support;

/// `M5` and `A100` set to all ones or all zeros, then the instruction with
/// output bus select 0, `IR<4:0>` = `rotate` and `IR<9:5>` = `length`,
/// M source 5, A source 100, into VMA; then a jump to itself, which is
/// where a run is measured from.
fn program(m_ones: bool, a_ones: bool, rotate: u64, length: u64) -> Program {
    let fill = |ones: bool| if ones { SETO } else { SETZ };
    let mut prom = vec![
        Insn::new(ALU | fill(m_ones) | m_dest(5)),
        Insn::new(ALU | fill(a_ones) | a_dest(0o100)),
        // `IR<13:12>` = 0: no `ALU` bit.
        Insn::new(m_src(5) | a_src(0o100) | VMA | length << 5 | rotate),
        Insn::new(JUMP | 3 << 12 | ALWAYS),
    ];
    prom.resize(512, filler());
    Program { name: "output bus select 0", prom, top: 3, cycles_per_iteration: 1 }
}

/// What the network gives, from the field definitions: the M source
/// rotated left by `rotate`, under `length` + 1 bits of mask from bit
/// `rotate` up, the A source elsewhere.  A mask whose top would pass bit
/// 31 wraps to nothing, as the two mask PROMs on MSKG4 ANDed give.
fn network(m: u32, a: u32, rotate: u32, length: u32) -> u32 {
    let left = (rotate + length) & 31;
    let mask = (!0u32 >> (31 - left)) & (!0u32 << rotate);
    (m.rotate_left(rotate) & mask) | (a & !mask)
}

fn on_engine<E: Engine>(new: fn(Machine) -> E, boot: fn(&mut E), p: &Program) -> u32 {
    let mut m = Machine::new();
    m.load_prom(&p.prom);
    let mut e = new(m);
    boot(&mut e);
    benchmark::run_engine(&mut e, p, Stop::Cycles(4)).iterations as u32
}

fn on_chip(p: &Program) -> u32 {
    let n = support::netlists();
    let (mut c, mut clk, mut far) = support::chip(&n);
    benchmark::boot_chip(&mut c, &n.cpu, &mut far, &mut clk, p);
    benchmark::run_chip(&mut c, &n.cpu, &mut far, &mut clk, p, Stop::Cycles(4)).iterations as u32
}

/// The cases: a mask in the middle, a mask that wraps to nothing, the
/// other source showing through, and the one mask a bare rotate gets
/// right, all ones.
const CASES: [(bool, bool, u32, u32); 4] =
    [(true, false, 3, 7), (true, false, 30, 5), (false, true, 3, 7), (true, false, 0, 31)];

/// **Output bus select 0 is the mask and rotate network on `micro` and
/// `rtl`**, case for case what the field definitions give.
#[test]
fn output_bus_select_0_is_the_network_on_micro_and_rtl() {
    for (m_ones, a_ones, rotate, length) in CASES {
        let (m, a) = (if m_ones { !0 } else { 0 }, if a_ones { !0 } else { 0 });
        let want = network(m, a, rotate, length);
        let p = program(m_ones, a_ones, rotate as u64, length as u64);
        let case = format!("M {m:#x}, A {a:#x}, rotate {rotate}, length {length}");
        assert_eq!(on_engine(Rtl::new, Rtl::boot, &p), want, "rtl: {case}");
        assert_eq!(on_engine(Micro::new, Micro::boot, &p), want, "micro: {case}");
    }
}

/// **The netlist agrees**: the board itself, run on the first case, gives
/// the network's output and not the bare rotate.
#[test]
fn output_bus_select_0_is_the_network_on_the_board() {
    let (m_ones, a_ones, rotate, length) = CASES[0];
    let want = network(!0, 0, rotate, length);
    assert_ne!(want, (!0u32).rotate_left(rotate), "the case tells the two apart");
    assert_eq!(on_chip(&program(m_ones, a_ones, rotate as u64, length as u64)), want);
}
