// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Programs for timing the engines.
//!
//! The boot PROM is a poor thing to time.  What it does in a window of
//! microcycles depends on where the boot has got to; much of that is waiting
//! on the drive, which times the wait and not the machine; and none of it
//! runs without the vendored pack.  These are loops instead.  Every time
//! round is the same as the last, so the rate over one window is the rate
//! over any other; and each program checks its own work, because a machine
//! that has stalled toggles fewer nets, and so looks *faster* to a clock,
//! not slower.
//!
//! Each program starts at 0 with the PROM enabled, as the boot does, sets
//! itself up, and then goes round its loop until it is stopped.  The loop is
//! closed in every register it touches but one: it leaves `Q` and the
//! scratchpads as it found them, and both arms of every branch in it are
//! taken every time round, so one time round costs the same microcycles as
//! any other.  The register that moves is the count, one a time round,
//! written to `VMA` where every engine can be asked for it.
//!
//! That is the check every run makes: the microcycles a run took, divided by
//! what one time round costs, is the count the program should have left, and
//! [`Run::check`] holds every run to it.  Neither program reaches its count
//! by a bare add of one.  `datapath` derives the one it adds from a chain of
//! operations that has to come out at one, and `control` reaches the add
//! down a path of calls, returns and branches that has to arrive there, past
//! a spin that catches a branch going the wrong way.
//!
//! It is not a proof that every operation was right, because a chain can
//! come out at one down a wrong road --- one did while this was being
//! written.  So `tests/benchmark.rs` holds the chain to its own running
//! values as well, one instruction at a time, off the console's read of the
//! output bus: the values in the comments below are enforced, not asserted.
//!
//! There are two programs: the datapath, and the control path.  A third,
//! main memory through the map, is not written.  It needs the map set up
//! before the loop, which the boot PROM does for itself and these would have
//! to as well, and it would time the far end's memory boards more than the
//! processor; when it is written it belongs beside these, with the same
//! self-check.
//!
//! `tests/benchmark.rs` holds every engine to them; `examples/benchmark.rs`
//! times them.

use std::time::{Duration, Instant};

use crate::cable::{Boards, FarEnd};
use crate::chip::Chip;
use crate::clock::{Behavioural, Clock};
use crate::engine::Engine;
use crate::isa::Insn;
use crate::isa::asm::*;
use crate::machine::{Machine, PROM_WORDS};
use crate::netlist::Netlist;
use crate::part::Level;

pub struct Program {
    pub name: &'static str,
    /// The boot PROM the program is loaded from, as many words as it fills.
    pub prom: Vec<Insn>,
    /// The top of the loop: the first instruction the program executes over
    /// and over.  A run is measured from one arrival here to another, so
    /// what the set-up before it cost never enters the rate.
    pub top: u16,
    /// Microcycles one time round the loop takes, counting the ones a jump
    /// inhibits.
    pub cycles_per_iteration: u64,
}

/// An A memory word to write when only the side effect of the instruction is
/// wanted.  [`filler`] writes the same one.
const SCRATCH: u64 = 0o100;

/// An A memory word past the low 32 that M memory shadows, so that writing
/// it and reading it back is A memory on its own.
const FAR: u64 = 0o200;

/// Zero and one where a program can reach them, and the count at zero:
/// `M3 = A3 = 0`, `M1 = A1 = 1`, `M2 = A2 = 0`.
///
/// A write to M memory lands in the low 32 words of A memory too, which is
/// what puts each of them on both sides at once.
fn prologue() -> Vec<Insn> {
    vec![
        Insn::new(ALU | SETZ | m_dest(3)),
        Insn::new(ALU | SETZ | m_dest(1)),
        Insn::new(ALU | ADD | CARRY_IN | m_src(1) | a_src(3) | m_dest(1)),
        Insn::new(ALU | SETZ | m_dest(2)),
    ]
}

/// `Q` and `VMA` to zero, which is the state the loop is closed on and the
/// count a run starts from.  The loop begins at the address this returns.
fn start(prom: &mut Vec<Insn>) -> u16 {
    prom.push(Insn::new(ALU | SETZ | Q_LOAD | a_dest(SCRATCH)));
    prom.push(Insn::new(ALU | SETM | m_src(2) | VMA));
    prom.len() as u16
}

fn padded(mut prom: Vec<Insn>) -> Vec<Insn> {
    prom.resize(PROM_WORDS, filler());
    prom
}

/// The datapath: every one of the ALU's sixteen logic functions and four of
/// its arithmetic ones, the output bus shifted both ways, the Q register
/// loaded, shifted both ways and read back as the functional source that
/// reads it, the byte hardware's three functions, and A memory both inside
/// and outside the words M memory shadows --- thirty-three operations, each
/// taking the last one's answer.
///
/// The chain is written so that it comes out at one, which is then what the
/// count goes up by.  The running value is in the comment on each line, and
/// nothing in the chain is a constant the program did not make out of the
/// one in `M1`.
pub fn datapath() -> Program {
    let mut prom = prologue();
    let top = start(&mut prom);
    let body = prom.len();
    let mut op = |raw: u64| prom.push(Insn::new(raw));

    // The ALU functions, each through M memory and A memory and back.  `W`
    // is M5, `X` is M6, `Y` is M7, and each is written to M memory so that
    // the A side of the next operation can have it.
    op(ALU | SETO | m_dest(5)); //                             W = -1
    op(ALU | ANDCA | m_src(5) | a_src(1) | m_dest(5)); //      W = W and not 1 = -2
    op(ALU | ADD | CARRY_IN | m_src(5) | a_src(1) | m_dest(5)); // W = W + 1 + 1 = 0, and wraps
    op(ALU | SETCM | m_src(5) | m_dest(5)); //                 W = not W = -1
    op(ALU | SUB | CARRY_IN | m_src(5) | a_src(1) | m_dest(5)); // W = W - 1 = -2
    op(ALU | XOR | m_src(5) | a_src(1) | m_dest(5)); //        W = W xor 1 = -1
    op(ALU | ANDCB | m_src(5) | a_src(1) | m_dest(5)); //      W = not (W or 1) = 0
    op(ALU | ORCB | m_src(5) | a_src(1) | m_dest(5)); //       W = not (W and 1) = -1
    op(ALU | EQV | m_src(5) | a_src(1) | m_dest(5)); //        W = not (W xor 1) = 1

    // A memory past the words M memory shadows: written, and read back on
    // the A side of the next instruction.
    op(ALU | SETM | m_src(5) | a_dest(FAR)); //                A[FAR] = 1
    op(ALU | SETA | a_src(FAR) | m_dest(6)); //                X = 1
    op(ALU | M_PLUS_M | m_src(6) | m_dest(6)); //              X = X + X = 2
    op(ALU | ADD | m_src(6) | a_src(1) | m_dest(6)); //        X = X + 1 = 3
    op(ALU | ANDCM | m_src(6) | a_src(1) | m_dest(5)); //      W = not X and 1 = 0
    op(ALU | ORCA | m_src(5) | a_src(1) | m_dest(5)); //       W = W or not 1 = -2
    op(ALU | IOR | m_src(5) | a_src(1) | m_dest(5)); //        W = W or 1 = -1
    op(ALU | AND | m_src(5) | a_src(1) | m_dest(5)); //        W = W and 1 = 1
    op(ALU | SETCA | a_src(1) | m_dest(7)); //                 Y = not 1 = -2
    op(ALU | ORCM | m_src(7) | a_src(1) | m_dest(7)); //       Y = not Y or 1 = 1
    op(ALU | M_PLUS_C | CARRY_IN | m_src(7) | m_dest(7)); //   Y = Y + 1 = 2

    // The output bus, which is what the ALU output goes through: shifted
    // left and shifted right.  The left shift takes Q<31> in at the bottom,
    // and Q is zero here.
    op(OB_LEFT | SETM | m_src(7) | m_dest(6)); //              X = Y << 1 = 4
    op(OB_RIGHT | SETM | m_src(6) | m_dest(6)); //             X = X >> 1 = 2

    // Q: loaded from the ALU, shifted both ways, and read back through the
    // functional source that reads it.  A right shift takes the ALU's low
    // bit in at the top and a left shift takes the complement of its sign in
    // at the bottom, so `SETZ` and `SETO` shift zeroes in either way.  The
    // last of them leaves Q as the loop found it.
    op(ALU | SETM | Q_LOAD | m_src(6) | m_dest(6)); //         Q = X = 2
    op(ALU | SETZ | Q_RIGHT | a_dest(SCRATCH)); //             Q = Q >> 1 = 1
    op(ALU | SETO | Q_LEFT | a_dest(SCRATCH)); //              Q = Q << 1 = 2
    op(ALU | SETM | SRC_Q | m_dest(6)); //                     X = Q = 2
    op(ALU | SETZ | Q_LOAD | a_dest(SCRATCH)); //              Q = 0

    // The byte hardware: a load, a deposit, and a deposit that does not
    // rotate.  The mask is `width` bits wide at bit 0 for a load and at the
    // rotate for a deposit, and what the mask does not take comes from the A
    // source.
    op(BYTE | LDB | width(4) | rot(31) | m_src(6) | a_src(3) | m_dest(7)); // Y = 1
    op(BYTE | DPB | width(4) | rot(8) | m_src(7) | a_src(3) | m_dest(7)); //  Y = 0o400
    op(BYTE | DEP | width(4) | rot(8) | m_src(7) | a_src(1) | m_dest(7)); //  Y = 0o401
    op(BYTE | LDB | width(1) | rot(0) | m_src(7) | a_src(3) | m_dest(5)); //  W = 1

    // The count, which the chain has just made the one for, and the loop.
    op(ALU | ADD | m_src(2) | a_src(5) | m_dest(2));
    op(ALU | SETM | m_src(2) | VMA);
    op(JUMP | target(top as u64) | ALWAYS | N);

    // Straight-line but for the jump back, which inhibits one.
    let cycles = (prom.len() - body) as u64 + 1;
    Program { name: "datapath", prom: padded(prom), top, cycles_per_iteration: cycles }
}

/// Where `control`'s subroutines sit, past the loop.  The two the dispatch
/// reaches are at addresses the program can build out of the one it made
/// itself: a deposit of it at a single bit, and that doubled.
const NEST1: u64 = 0o100;
const NEST2: u64 = 0o110;
const NEST3: u64 = 0o120;
const LEAF1: u64 = 0o130;
const LEAF2: u64 = 0o140;
const POPJ_SUB: u64 = 0o150;
const DTARGET0: u64 = 0o200;
const DTARGET1: u64 = 0o400;
/// The two words of dispatch memory the program writes and dispatches
/// through.  The low bit of the address comes from the count, so the two
/// take turns.
const DMEM: u64 = 0o20;

/// Control: calls three deep and returning, two leaf calls, a call that
/// returns on the POPJ bit rather than by a jump, a dispatch through
/// dispatch memory the program wrote itself, and four conditional jumps ---
/// a bit of the M source and the ALU's own comparison, each taken and not
/// taken.  Then a jump that does not inhibit, so the instruction after it
/// runs, and that instruction is the count.
///
/// Every branch goes the same way every time round, so the loop costs the
/// same microcycles each time; the ones that never go point at a spin, so a
/// branch that goes the wrong way stops the count rather than hiding.
pub fn control() -> Program {
    let mut prom = prologue();

    // The two dispatch entries, made out of the one in M1 and deposited a
    // field at a time.  A dispatch entry is `DPC<13:0>` with N at 14, P at
    // 15 and R at 16, so 3 deposited at 15 and 14 is the N and P that make a
    // dispatch a call which inhibits the instruction after it, exactly as a
    // JUMP with P and N is.
    prom.push(Insn::new(ALU | M_PLUS_M | m_src(1) | m_dest(9))); //  M9 = 2
    prom.push(Insn::new(ALU | ADD | m_src(9) | a_src(1) | m_dest(9))); // M9 = 3
    prom.push(Insn::new(BYTE | DPB | width(1) | rot(7) | m_src(1) | a_src(3) | m_dest(8)));
    prom.push(Insn::new(BYTE | DPB | width(2) | rot(14) | m_src(9) | a_src(8) | a_dest(10)));
    prom.push(Insn::new(ALU | M_PLUS_M | m_src(8) | m_dest(8))); //  M8 = DTARGET1
    prom.push(Insn::new(BYTE | DPB | width(2) | rot(14) | m_src(9) | a_src(8) | a_dest(11)));
    prom.push(Insn::new(DISPATCH | DMEM_WRITE | d_addr(DMEM) | a_src(10)));
    prom.push(Insn::new(DISPATCH | DMEM_WRITE | d_addr(DMEM + 1) | a_src(11)));

    let top = start(&mut prom);
    let at = |i: u64| top as u64 + i;
    // The spin, which nothing reaches: the two branches that never go point
    // at it, and the jump that does not inhibit steps over it.
    let trap = at(11);
    for (i, raw) in [
        // Three deep and back, then two leaves, then one that returns on the
        // POPJ bit.
        JUMP | target(NEST1) | P | N | ALWAYS,
        JUMP | target(LEAF1) | P | N | ALWAYS,
        JUMP | target(LEAF2) | P | N | ALWAYS,
        JUMP | target(POPJ_SUB) | P | N | ALWAYS,
        // A dispatch, its address the count's low bit ored into DMEM, so the
        // two entries take turns.
        DISPATCH | d_addr(DMEM) | d_len(1) | m_src(2),
        // Bit 0 of the M source: M1 is one, so plain is taken and inverted
        // is not.
        JUMP | target(at(6)) | m_src(1) | bit(0) | N,
        JUMP | target(trap) | m_src(1) | bit(0) | INVERT | N,
        // The ALU's own comparison: A1 is the one M1 is, A3 is not.
        JUMP | target(at(8)) | m_src(1) | a_src(1) | AEQM | N,
        JUMP | target(trap) | m_src(1) | a_src(3) | AEQM | N,
        // A taken jump with no N, so the instruction after it is not
        // inhibited but run --- and that instruction is the count.
        JUMP | target(at(12)) | ALWAYS,
        ALU | ADD | m_src(2) | a_src(1) | m_dest(2),
        JUMP | target(trap) | ALWAYS | N,
        ALU | SETM | m_src(2) | VMA,
        JUMP | target(top as u64) | ALWAYS | N,
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(prom.len() as u64, at(i as u64), "the loop's addresses");
        prom.push(Insn::new(raw));
    }

    let mut prom = padded(prom);
    let mut sub = |addr: u64, raw: u64| prom[addr as usize] = Insn::new(raw);
    sub(NEST1, JUMP | target(NEST2) | P | N | ALWAYS);
    sub(NEST1 + 1, JUMP | R | N | ALWAYS);
    sub(NEST2, JUMP | target(NEST3) | P | N | ALWAYS);
    sub(NEST2 + 1, JUMP | R | N | ALWAYS);
    sub(NEST3, JUMP | R | N | ALWAYS);
    sub(LEAF1, JUMP | R | N | ALWAYS);
    sub(LEAF2, JUMP | R | N | ALWAYS);
    // POPJ does not inhibit, so the instruction after the one carrying it
    // runs before the return takes: the subroutine is two instructions, and
    // costs what a leaf call that returns by a jump costs.
    sub(POPJ_SUB, ALU | POPJ | SETA | a_src(3) | a_dest(SCRATCH));
    sub(POPJ_SUB + 1, ALU | SETA | a_src(3) | a_dest(SCRATCH));
    sub(DTARGET0, JUMP | R | N | ALWAYS);
    sub(DTARGET1, JUMP | R | N | ALWAYS);

    // A jump that is taken and has N costs two, its own microcycle and the
    // one it inhibits; one that is not taken costs one.  So: six of them for
    // the nest three deep, four for each leaf call, four for the call that
    // comes back on POPJ and the slot POPJ does not inhibit, four for the
    // dispatch and the return from it, two for each conditional jump that
    // goes and one for each that does not, one for the jump that does not
    // inhibit and one for the count it runs, one for the write to VMA and
    // two for the jump back.
    let cycles = 12 + 4 + 4 + 4 + 4 + (2 + 1 + 2 + 1) + (1 + 1) + 1 + 2;
    Program { name: "control", prom, top, cycles_per_iteration: cycles }
}

/// When to stop a run.
#[derive(Clone, Copy)]
pub enum Stop {
    /// At the first top of the loop past this much wall clock.
    After(Duration),
    /// At the first top of the loop past this many microcycles.  What the
    /// tests stop on, so that a run is the same one on every machine.
    Cycles(u64),
}

/// What a run did.
pub struct Run {
    /// Microcycles, from one top of the loop to another.
    pub cycles: u64,
    /// Times round the loop in them, as the program counted it in `VMA`.
    pub iterations: u64,
    /// Seconds the `cycles` took.
    pub secs: f64,
}

impl Run {
    pub fn rate(&self) -> f64 {
        self.cycles as f64 / self.secs
    }

    /// The program's own check: the microcycles run are the times round the
    /// loop times what one time round costs, exactly.  A machine that has
    /// stalled, or one that got an operation or a return wrong, fails it.
    pub fn check(&self, p: &Program) {
        assert!(self.iterations > 0, "{}: the loop did not go round", p.name);
        assert_eq!(
            self.cycles,
            self.iterations * p.cycles_per_iteration,
            "{}: {} microcycles against {} times round",
            p.name,
            self.cycles,
            self.iterations
        );
    }
}

/// Microcycles to run before looking at the clock again: a hundredth of what
/// has run already, so that reading the clock costs a hundredth of a run at
/// the very start and nothing worth measuring after, and a run overshoots
/// its budget by no more than a hundredth either.
fn chunk(ran: u64) -> u64 {
    (ran / 100).clamp(1, 1 << 16)
}

fn more(stop: Stop, ran: u64, t: Instant) -> u64 {
    match stop {
        Stop::Cycles(n) => n.saturating_sub(ran).min(chunk(ran)),
        Stop::After(d) => {
            if t.elapsed() >= d {
                0
            } else {
                chunk(ran)
            }
        }
    }
}

fn step<E: Engine>(e: &mut E) {
    if let Err(halt) = e.step() {
        panic!("halted at {:o}: {halt:?}", e.pc());
    }
}

/// Runs a program on an engine and times it.
///
/// The clock starts at the first arrival at the top of the loop and stops at
/// another, so the set-up before the loop is not in the rate and the run is
/// a whole number of times round.
pub fn run_engine<E: Engine>(e: &mut E, p: &Program, stop: Stop) -> Run {
    while e.pc() != p.top {
        step(e);
    }
    let t = Instant::now();
    let mut ran = 0;
    loop {
        let n = more(stop, ran, t);
        if n == 0 {
            break;
        }
        for _ in 0..n {
            step(e);
            ran += 1;
        }
    }
    while e.pc() != p.top {
        step(e);
        ran += 1;
    }
    let secs = t.elapsed().as_secs_f64();
    Run { cycles: ran, iterations: engine_vma(e) as u64, secs }
}

/// The count an engine has left in `VMA`, read after the write has landed.
///
/// The two microcycles are the one the jump back inhibits and the first of
/// the loop, neither of which writes `VMA`, so the count does not move under
/// the read.
pub fn engine_vma<E: Engine>(e: &mut E) -> u32 {
    for _ in 0..2 {
        e.step().unwrap();
    }
    e.machine().vma
}

/// The board as `muir --chip` runs it: every board a netlist but the disk
/// controller, thirty-two memory boards, and no pack.
pub fn chip(
    cpu: &Netlist,
    busint: &Netlist,
    memory: &Netlist,
    io: &Netlist,
    tv: &Netlist,
) -> (Chip, Behavioural, FarEnd) {
    let boards = Boards { memory: 32, io: Some(io), tv: Some(tv), ..Default::default() };
    let far = FarEnd::new(cpu, busint, memory, boards, 0, Machine::new());
    (Chip::new(cpu), Behavioural::new(), far)
}

/// The program into the board's PROM, and the board booted as `muir --chip`
/// boots it: the far end joined, the button, the start-up microcycles before
/// the PC moves.
pub fn boot_chip(
    c: &mut Chip,
    n: &Netlist,
    far: &mut FarEnd,
    clk: &mut Behavioural,
    program: &Program,
) {
    let image: Vec<u64> = program.prom.iter().map(|&i| crate::prom::programming(i)).collect();
    c.power_on();
    c.load_prom(n, &image);
    c.settle();
    far.join(c, clk.time_ns());
    let boot = n.by_name_id("-BOOT1").unwrap();
    c.set_net(boot, Level::Low);
    c.settle();
    for _ in 0..20 {
        c.tick(clk);
    }
    c.set_net(boot, Level::High);
    let pc = c.bus_nets(n, "PC", 14);
    let mut skipped = 0;
    while c.read(&pc) == 0 && skipped < 40 {
        c.microcycle(clk);
        skipped += 1;
    }
}

/// One microcycle of the board, as `muir --chip` runs it: however many clock
/// transitions it takes for the phase to wrap, the far end taking its own
/// turns in between.
fn chip_step(c: &mut Chip, far: &mut FarEnd, clk: &mut Behavioural) {
    let mut last = clk.phase_ns();
    for _ in 0..(1 << 16) {
        far.tick_with(c, clk);
        let p = clk.phase_ns();
        if p < last {
            return;
        }
        last = p;
    }
    panic!("the clock phase did not wrap in a microcycle's worth of transitions");
}

/// Runs a program on the board and times it, as [`run_engine`] does on an
/// engine.
pub fn run_chip(
    c: &mut Chip,
    n: &Netlist,
    far: &mut FarEnd,
    clk: &mut Behavioural,
    p: &Program,
    stop: Stop,
) -> Run {
    let pc = c.bus_nets(n, "PC", 14);
    let at_top = |c: &mut Chip| c.read(&pc) as u16 == p.top;
    while !at_top(c) {
        chip_step(c, far, clk);
    }
    let t = Instant::now();
    let mut ran = 0;
    loop {
        let n = more(stop, ran, t);
        if n == 0 {
            break;
        }
        for _ in 0..n {
            chip_step(c, far, clk);
            ran += 1;
        }
    }
    while !at_top(c) {
        chip_step(c, far, clk);
        ran += 1;
    }
    let secs = t.elapsed().as_secs_f64();
    Run { cycles: ran, iterations: chip_vma(c, n, far, clk) as u64, secs }
}

/// The count the board has left in `VMA`, read off the register's `-VMA`
/// outputs after the write has landed.
pub fn chip_vma(c: &mut Chip, n: &Netlist, far: &mut FarEnd, clk: &mut Behavioural) -> u32 {
    let vma = c.bus_nets(n, "-VMA", 32);
    for _ in 0..2 {
        chip_step(c, far, clk);
    }
    !(c.read(&vma) as u32)
}
