// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The multiply and divide steps, on all three engines.
//!
//! `MUS`, `DVS1`, `DVS` and `DVREM` are ALU functions 40, 51, 41 and 45 in
//! the ALU FUNCTIONS table of `mit/cadr/ir.bits`, and page ALUC4 decides
//! what the 74S181s do on each from `Q<0>` and the A source's sign
//! (`crate::ttl::alu_control`, shared by `micro` and `rtl`). The boot PROM
//! never runs them, and microcode 323 runs them in every `MPY` and `DIV`.
//! Here a whole 32-step multiply and a whole 33-step divide, each on three
//! pairs of operands, leave the same high word and the same `Q` on `micro`,
//! on `rtl` and on the netlist.
//!
//! The steps are laid out as `uc-arith.lisp`'s loops lay them: the
//! multiply shifts the output bus and `Q` right, the divide shifts both
//! left, `DVS1` first and `DVREM` last.  Operands are built in M memory by
//! doubling and adding the carry, so that nothing but the ALU is used.

use muir::benchmark::{self, Program, Stop};
use muir::engine::Engine;
use muir::isa::Insn;
use muir::isa::asm::{
    ALU, ALWAYS, CARRY_IN, JUMP, M_PLUS_M, OB_LEFT, OB_RIGHT, Q_LEFT, Q_LOAD, Q_RIGHT, SETM, SETZ,
    SRC_Q, VMA, a_dest, a_src, filler, m_dest, m_src, target,
};
use muir::machine::Machine;
use muir::micro::Micro;
use muir::rtl::Rtl;

mod support;

const MUS: u64 = 0o40 << 3;
const DVS: u64 = 0o41 << 3;
const DVS1: u64 = 0o51 << 3;
const DVREM: u64 = 0o45 << 3;

/// `v` into M memory `r`: zero, then one doubling a bit, with the bit as the
/// carry in.
fn constant(v: u32, r: u64, out: &mut Vec<Insn>) {
    out.push(Insn::new(ALU | SETZ | m_dest(r)));
    for b in (0..32).rev() {
        let c = if (v >> b) & 1 != 0 { CARRY_IN } else { 0 };
        out.push(Insn::new(ALU | M_PLUS_M | c | m_src(r) | a_src(r) | m_dest(r)));
    }
}

/// `body`, then the result into `VMA`, then a jump to itself.
fn program(mut body: Vec<Insn>, result_in_q: bool) -> Program {
    body.push(Insn::new(ALU | SETM | if result_in_q { SRC_Q } else { m_src(5) } | VMA));
    let top = body.len() as u16;
    body.push(Insn::new(JUMP | target(top as u64) | ALWAYS));
    body.resize(512, filler());
    Program { name: "muldiv", prom: body, top, cycles_per_iteration: 1 }
}

fn multiply(x: u32, y: u32) -> Vec<Insn> {
    let mut b = Vec::new();
    constant(x, 1, &mut b);
    constant(y, 2, &mut b);
    b.push(Insn::new(ALU | SETM | m_src(2) | a_dest(0o100)));
    b.push(Insn::new(ALU | SETM | m_src(1) | Q_LOAD | m_dest(3)));
    b.push(Insn::new(ALU | SETZ | m_dest(5)));
    for _ in 0..32 {
        b.push(Insn::new(OB_RIGHT | MUS | Q_RIGHT | m_src(5) | a_src(0o100) | m_dest(5)));
    }
    b
}

fn divide(hi: u32, lo: u32, d: u32) -> Vec<Insn> {
    let mut b = Vec::new();
    constant(lo, 1, &mut b);
    constant(d, 2, &mut b);
    constant(hi, 5, &mut b);
    b.push(Insn::new(ALU | SETM | m_src(2) | a_dest(0o100)));
    b.push(Insn::new(ALU | SETM | m_src(1) | Q_LOAD | m_dest(3)));
    b.push(Insn::new(OB_LEFT | DVS1 | Q_LEFT | m_src(5) | a_src(0o100) | m_dest(5)));
    for _ in 0..31 {
        b.push(Insn::new(OB_LEFT | DVS | Q_LEFT | m_src(5) | a_src(0o100) | m_dest(5)));
    }
    b.push(Insn::new(ALU | DVREM | m_src(5) | a_src(0o100) | m_dest(5)));
    b
}

/// Every case, with its name: three multiplies and three divides, each read
/// twice, once for the high word and once for `Q`.
fn cases() -> Vec<(String, Program)> {
    let mut all = Vec::new();
    for q in [false, true] {
        let half = if q { "Q" } else { "M" };
        for (x, y) in [(12345u32, 0x9abcd), (0xffff_fff3, 7), (0x8000_0001, 0x7fff_ffff)] {
            all.push((format!("{x:x} * {y:x}, {half}"), program(multiply(x, y), q)));
        }
        for (hi, lo, d) in [(0u32, 100_000u32, 7u32), (0, 0x8765_4321, 0x1234), (!0, !0 - 15, 3)] {
            all.push((format!("{hi:x}:{lo:x} / {d:x}, {half}"), program(divide(hi, lo, d), q)));
        }
    }
    all
}

fn on_engine<E: Engine>(new: fn(Machine) -> E, boot: fn(&mut E), p: &Program) -> u32 {
    let mut m = Machine::new();
    m.load_prom(&p.prom);
    let mut e = new(m);
    boot(&mut e);
    benchmark::run_engine(&mut e, p, Stop::Cycles(4)).iterations as u32
}

/// **The steps leave the same words on `micro`, `rtl` and the board.**
#[test]
fn the_multiply_and_divide_steps_agree_on_all_three_engines() {
    let n = support::netlists();
    for (name, p) in cases() {
        let (mut c, mut clk, mut far) = support::chip(&n);
        benchmark::boot_chip(&mut c, &n.cpu, &mut far, &mut clk, &p);
        let board = benchmark::run_chip(&mut c, &n.cpu, &mut far, &mut clk, &p, Stop::Cycles(4))
            .iterations as u32;
        assert_eq!(on_engine(Rtl::new, Rtl::boot, &p), board, "rtl: {name}");
        assert_eq!(on_engine(Micro::new, Micro::boot, &p), board, "micro: {name}");
    }
}
