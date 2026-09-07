// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The benchmark programs do what they say, on every engine.
//!
//! Three things are held.  That one time round the loop costs the
//! microcycles `src/benchmark.rs` says it costs, which is what says no wait
//! state is hiding in it and what every run's own check rests on.  That the
//! datapath's chain of operations passes through the values its comments
//! give, one instruction at a time, so the comments are enforced rather than
//! believed.  And that the engines agree: `micro`, `rtl` and the board reach
//! the same count in the same microcycles.
//!
//! `chip` is booted as `muir --chip` boots it, every board a netlist but the
//! disk controller, and needs nothing from `vendor/`.

use muir::benchmark::{self, Program, Run, Stop};
use muir::engine::Engine;
use muir::machine::Machine;
use muir::micro::Micro;
use muir::rtl::Rtl;
use muir::spy;

mod support;

/// Enough microcycles for several times round either loop, and the same
/// number on every machine.
const CYCLES: u64 = 10_000;

fn booted<E: Engine>(new: fn(Machine) -> E, boot: fn(&mut E), p: &Program) -> E {
    let mut m = Machine::new();
    m.load_prom(&p.prom);
    let mut e = new(m);
    boot(&mut e);
    e
}

fn on_micro(p: &Program) -> Run {
    benchmark::run_engine(&mut booted(Micro::new, Micro::boot, p), p, Stop::Cycles(CYCLES))
}

fn on_rtl(p: &Program) -> Run {
    benchmark::run_engine(&mut booted(Rtl::new, Rtl::boot, p), p, Stop::Cycles(CYCLES))
}

fn on_chip(p: &Program) -> Run {
    let n = support::netlists();
    let (mut c, mut clk, mut far) = support::chip(&n);
    benchmark::boot_chip(&mut c, &n.cpu, &mut far, &mut clk, p);
    benchmark::run_chip(&mut c, &n.cpu, &mut far, &mut clk, p, Stop::Cycles(CYCLES))
}

/// The count and the microcycles agree: the run went round the loop a whole
/// number of times and did a loop's worth of work each time.
fn holds(run: Run, p: &Program) -> Run {
    run.check(p);
    assert!(run.cycles >= CYCLES, "{}: the run stopped short", p.name);
    assert!(run.cycles < CYCLES + p.cycles_per_iteration, "{}: the run ran on", p.name);
    run
}

#[test]
fn datapath_counts_on_micro_and_rtl() {
    let p = benchmark::datapath();
    let a = holds(on_micro(&p), &p);
    let b = holds(on_rtl(&p), &p);
    assert_eq!((a.cycles, a.iterations), (b.cycles, b.iterations), "micro and rtl part");
}

#[test]
fn control_counts_on_micro_and_rtl() {
    let p = benchmark::control();
    let a = holds(on_micro(&p), &p);
    let b = holds(on_rtl(&p), &p);
    assert_eq!((a.cycles, a.iterations), (b.cycles, b.iterations), "micro and rtl part");
}

#[test]
fn datapath_counts_on_chip() {
    let p = benchmark::datapath();
    let a = holds(on_chip(&p), &p);
    let b = on_micro(&p);
    assert_eq!((a.cycles, a.iterations), (b.cycles, b.iterations), "the board and micro part");
}

#[test]
fn control_counts_on_chip() {
    let p = benchmark::control();
    let a = holds(on_chip(&p), &p);
    let b = on_micro(&p);
    assert_eq!((a.cycles, a.iterations), (b.cycles, b.iterations), "the board and micro part");
}

/// What `datapath`'s thirty-three operations put on the output bus, in
/// order: the running value in the comment on each line of
/// `benchmark::datapath`.  A chain that arrives at one down a different road
/// passes the count's own check and fails this one.
const CHAIN: [u32; 33] = [
    0xffff_ffff, // SETO, and the ALU functions begin
    0xffff_fffe, // ANDCA
    0,           // ADD, and it wraps
    0xffff_ffff, // SETCM
    0xffff_fffe, // SUB
    0xffff_ffff, // XOR
    0,           // ANDCB
    0xffff_ffff, // ORCB
    1,           // EQV
    1,           // into A memory past the words M memory shadows
    1,           // and back off it
    2,           // M+M
    3,           // ADD
    0,           // ANDCM
    0xffff_fffe, // ORCA
    0xffff_ffff, // IOR
    1,           // AND
    0xffff_fffe, // SETCA
    1,           // ORCM
    2,           // M+c, and the ALU functions are done
    4,           // the output bus shifted left
    2,           // and shifted right
    2,           // into Q
    0,           // Q shifted right, the output bus zero
    0xffff_ffff, // Q shifted left, the output bus all ones
    2,           // Q back out through the functional source that reads it
    0,           // and zero into Q, where the loop found it
    1,           // the byte hardware: a load
    0o400,       // a deposit
    0o401,       // a deposit that does not rotate
    1,           // and a load of one bit
    1,           // the count, at its first time round
    1,           // and into VMA
];

/// The chain instruction by instruction, off the console's own read of the
/// output bus.  `micro` has no read phase apart from execution, so the `OB`
/// it shows between two microcycles is the one just computed.
#[test]
fn datapath_chain_on_micro() {
    let p = benchmark::datapath();
    let mut e = booted(Micro::new, Micro::boot, &p);
    while e.pc() != p.top {
        e.step().unwrap();
    }
    // The stop is two microcycles before the top of the loop executes; the
    // first of them is the last instruction before it.
    e.step().unwrap();
    for (i, want) in CHAIN.iter().enumerate() {
        e.step().unwrap();
        let ob = e.spy_read(spy::OB_LOW) as u32 | (e.spy_read(spy::OB_HIGH) as u32) << 16;
        assert_eq!(ob, *want, "datapath operation {i}");
    }
}

/// The same chain seen from the other end: what it leaves in the
/// scratchpads, which `micro` and `rtl` must agree on.
#[test]
fn datapath_leaves_the_same_on_micro_and_rtl() {
    let p = benchmark::datapath();
    let state = |e: &mut dyn Engine| {
        while e.pc() != p.top {
            e.step().unwrap();
        }
        for _ in 0..p.cycles_per_iteration + 1 {
            e.step().unwrap();
        }
        let m = e.machine();
        // W, X and Y, the A memory word past the shadowed ones, Q as the
        // loop put it back, and the count.
        (m.mmem[5], m.mmem[6], m.mmem[7], m.amem[0o200], m.q, m.vma)
    };
    let want = (1, 2, 0o401, 1, 0, 1);
    assert_eq!(state(&mut booted(Micro::new, Micro::boot, &p)), want, "micro");
    assert_eq!(state(&mut booted(Rtl::new, Rtl::boot, &p)), want, "rtl");
}
