// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! QUUX's clocks in the processor (contract Q1): the tick, fixed at 60 Hz,
//! the machine's clock in place of the display's vertical interrupt; an
//! interval timer; and a microsecond clock.
//!
//! Functional destination 3 is their control: `<0>` enables the tick and a
//! write with `<1>` set clears its flag; `<2>` enables the interval timer
//! and `<3>` clears its flag. Destination 4 is the interval timer's period
//! in microseconds, `<23:0>`, 0 stopping it. Functional source 17 reads
//! `<0>` the tick's flag, `<1>` its enable, `<2>` the interval timer's flag
//! and `<3>` its enable; each flag, under its enable, is one more term of
//! the interrupt the microcode already tests in jump conditions 5 and 6.
//! Functional source 15 reads the microseconds since power-on, 32 bits,
//! wrapping. On the CADR destinations 3 and 4 write only M and sources 15
//! and 17 read all ones.

use muir::engine::Engine;
use muir::isa::Insn;
use muir::isa::asm::{
    ALU, ALWAYS, JUMP, N, SETM, SETZ, START_READ, bit, filler, m_dest, m_src, src, target,
};
use muir::machine::{Geometry, Machine};
use muir::micro::Micro;
use muir::rtl::Rtl;

/// Functional destinations 2, 3 and 4, M 37 written too.
const INTERRUPT_CONTROL: u64 = (2 << 19) | (0o37 << 14);
const CLOCK_CONTROL: u64 = (3 << 19) | (0o37 << 14);
const INTERVAL_PERIOD: u64 = (4 << 19) | (0o37 << 14);
/// Jump condition 5: a page fault or an interrupt pending.
const PGF_OR_INT: u64 = (1 << 5) | 5;

/// Control words: the tick on; the tick on and its flag cleared; the
/// interval timer on; on and cleared.
const TICK_ON: u32 = 1;
const TICK_CLEAR: u32 = 3;
const INTERVAL_ON: u32 = 4;
const INTERVAL_CLEAR: u32 = 12;

/// Writes the interval period, M 1, then the control word M 2, waits for
/// status bit `flag` reading source 17 into M 3, then writes the control
/// word M 4 and reads source 17 into M 5, and stops at 7.
fn wait_and_clear(flag: u64) -> Vec<Insn> {
    vec![
        Insn::new(ALU | SETM | m_src(1) | INTERVAL_PERIOD),
        Insn::new(ALU | SETM | m_src(2) | CLOCK_CONTROL),
        Insn::new(ALU | SETM | src(0o17) | m_dest(3)),
        Insn::new(JUMP | target(5) | bit(flag) | m_src(3) | N),
        Insn::new(JUMP | target(2) | ALWAYS | N),
        Insn::new(ALU | SETM | m_src(4) | CLOCK_CONTROL),
        Insn::new(ALU | SETM | src(0o17) | m_dest(5)),
        Insn::new(JUMP | target(7) | ALWAYS | N),
    ]
}

fn machine(geometry: Geometry, prom: &[Insn], period_us: u32, control: [u32; 2]) -> Machine {
    let mut m = Machine::new();
    let mut words = vec![filler(); 512];
    words[..prom.len()].copy_from_slice(prom);
    m.load_prom(&words);
    m.geometry = geometry;
    m.mmem[1] = period_us;
    m.mmem[2] = control[0];
    m.mmem[4] = control[1];
    m.mmem[6] = 1 << 27;
    m.l2_map[0] = (1 << 23) | (1 << 22);
    m
}

/// Runs until `pc` is reached or `limit` steps; the time then, if reached.
/// A few more steps after, so that the last writes have landed: `rtl`
/// writes M a microcycle late.
fn until<E: Engine>(e: &mut E, pc: u16, limit: u64, ns: fn(&E) -> u64) -> Option<u64> {
    for _ in 0..limit {
        if e.pc() == pc {
            let t = ns(e);
            for _ in 0..4 {
                e.step().unwrap();
            }
            return Some(t);
        }
        e.step().unwrap();
    }
    None
}

fn micro_ns(e: &Micro) -> u64 {
    e.machine().ns
}

/// Both engines, the time each reached `pc` in, and their M memories.
fn both(m: impl Fn() -> Machine, pc: u16, limit: u64) -> [(Option<u64>, Vec<u32>); 2] {
    let mut e = Micro::new(m());
    e.boot();
    let te = until(&mut e, pc, limit, micro_ns);
    let mut r = Rtl::new(m());
    r.boot();
    let tr = until(&mut r, pc, limit, Rtl::ns);
    [(te, e.machine().mmem.to_vec()), (tr, r.machine().mmem.to_vec())]
}

/// **The interval timer's flag rises one period after it is enabled, and a
/// write clears it.** Timed as the difference between a 100 and a 300 µs
/// period, which leaves out the boot: 200 µs, give or take the
/// three-microcycle wait loop. Source 17 reads its flag and enable, 14, with
/// the flag up, and the enable alone, 8, once cleared.
#[test]
fn the_interval_timer_rises_each_period_and_a_write_clears_it() {
    let prom = wait_and_clear(2);
    let run = |p| both(|| machine(Geometry::QUUX, &prom, p, [INTERVAL_ON, INTERVAL_CLEAR]), 8, 10_000_000);
    let (short, long) = (run(100), run(300));
    for (k, name) in ["micro", "rtl"].into_iter().enumerate() {
        let (ts, ms) = &short[k];
        let (tl, _) = &long[k];
        let (ts, tl) = (ts.expect(name), tl.expect(name));
        assert!(ts >= 100_000, "{name}: the flag rose at {ts} ns, before a period");
        let diff = tl - ts;
        assert!(diff.abs_diff(200_000) <= 3 * 220, "{name}: 200 µs more took {diff} ns");
        assert_eq!((ms[3] & 0o17, ms[5] & 0o17), (0o14, 0o10), "{name}: source 17 up, then cleared");
    }
}

/// **An interval timer with period 0 is stopped**: enabled, its flag never
/// rises.
#[test]
fn an_interval_of_0_is_stopped() {
    let prom = wait_and_clear(2);
    for (k, (t, _)) in both(|| machine(Geometry::QUUX, &prom, 0, [INTERVAL_ON, 0]), 8, 100_000)
        .into_iter()
        .enumerate()
    {
        assert!(t.is_none(), "{}: the flag rose", ["micro", "rtl"][k]);
    }
}

/// **The tick is 60 Hz, whatever destination 4 says**: with the interval
/// period written as 100 µs and only the tick enabled, the tick's flag
/// rises 16,667 µs after the enable. Source 17 reads 3 with it up and 2
/// once cleared.
#[test]
fn the_tick_is_60_hz_whatever_destination_4_says() {
    let prom = wait_and_clear(0);
    let m = || machine(Geometry::QUUX, &prom, 100, [TICK_ON, TICK_CLEAR]);
    let t = both(m, 8, 200_000_000);
    let quick = both(m, 3, 1_000);
    for (k, name) in ["micro", "rtl"].into_iter().enumerate() {
        let enabled = quick[k].0.expect(name);
        let seen = t[k].0.expect(name) - enabled;
        // The enable lands a microcycle or two after `PC` passes 3, and the
        // wait loop is three microcycles, at the boot's 220 ns.
        assert!(seen.abs_diff(16_667_000) <= 8 * 220, "{name}: the first tick after {seen} ns");
        assert_eq!((t[k].1[3] & 3, t[k].1[5] & 3), (3, 2), "{name}: source 17 up, then cleared");
    }
}

/// **A pending tick or interval is an interrupt the microcode's jump
/// sees**: with `INT.ENABLE` up, condition 5 is taken once the flag rises,
/// and not while both are off. Condition 5 is a page fault too, and
/// `VMAOK` is down after the boot, so the program first reads a page the
/// map permits: page 0, readable and writable in level-2 entry 0.
#[test]
fn a_pending_tick_or_interval_takes_the_interrupt_condition() {
    let prom = vec![
        Insn::new(ALU | SETZ | START_READ),
        Insn::new(ALU | SETM | m_src(1) | INTERVAL_PERIOD),
        Insn::new(ALU | SETM | m_src(2) | CLOCK_CONTROL),
        Insn::new(ALU | SETM | m_src(6) | INTERRUPT_CONTROL),
        Insn::new(JUMP | target(6) | PGF_OR_INT | N),
        Insn::new(JUMP | target(4) | ALWAYS | N),
        Insn::new(JUMP | target(6) | ALWAYS | N),
    ];
    for (control, want) in [(INTERVAL_ON, true), (TICK_ON, true), (0, false)] {
        let t = both(|| machine(Geometry::QUUX, &prom, 50, [control, 0]), 7, 1_000_000);
        for (k, name) in ["micro", "rtl"].into_iter().enumerate() {
            assert_eq!(t[k].0.is_some(), want, "{name}: control {control}");
        }
    }
}

/// **Source 15 is the microseconds since power-on**, on both engines, and
/// under `sync` on `rtl`: read in a loop, it is the engine's own time over
/// a thousand, within a microsecond; and it wraps at 32 bits.
#[test]
fn source_15_counts_microseconds() {
    use muir::clock::TimingModel;
    let prom = vec![
        Insn::new(ALU | SETM | src(0o15) | m_dest(1)),
        Insn::new(JUMP | target(0) | ALWAYS | N),
    ];
    let quux = |start_ns: u64| {
        let mut m = machine(Geometry::QUUX, &prom, 0, [0, 0]);
        m.ns = start_ns;
        m
    };
    let wrap = (1u64 << 32) * 1000 - 3_000;
    for start in [0, wrap] {
        let mut e = Micro::new(quux(start));
        e.boot();
        for _ in 0..40_000 {
            e.step().unwrap();
        }
        let want = (e.machine().ns / 1000) as u32;
        assert!(want.wrapping_sub(e.machine().mmem[1]) <= 1, "micro from {start}: {} against {want}", e.machine().mmem[1]);
        for model in [TimingModel::Cadr, TimingModel::Sync { cycle_ticks: 4, ilong_ticks: 0 }] {
            let mut r = Rtl::new(quux(start));
            r.set_timing_model(model);
            r.boot();
            for _ in 0..40_000 {
                r.step().unwrap();
            }
            let want = (r.ns() / 1000) as u32;
            let got = r.machine().mmem[1];
            assert!(want.wrapping_sub(got) <= 1, "rtl {model:?} from {start}: {got} against {want}");
            if start == wrap {
                assert!(got < 100_000, "rtl {model:?}: wrapped, {got}");
            }
        }
    }
}

/// **The CADR has none of it**: sources 15 and 17 read all ones and
/// destinations 3 and 4 write only M, so nothing ever pends.
#[test]
fn the_cadr_has_no_clocks_in_the_processor() {
    let prom = {
        let mut p = wait_and_clear(0);
        p[6] = Insn::new(ALU | SETM | src(0o15) | m_dest(5));
        p
    };
    for (k, (t, m)) in both(|| machine(Geometry::CADR, &prom, 100, [TICK_ON, TICK_CLEAR]), 8, 100_000)
        .into_iter()
        .enumerate()
    {
        let name = ["micro", "rtl"][k];
        assert!(t.is_some(), "{name}: the all-ones source is a flag at once");
        assert_eq!((m[3], m[5]), (!0, !0), "{name}: sources 17 and 15");
        assert_eq!(m[0o37], TICK_CLEAR, "{name}: destination 3 wrote M 37");
    }
}

/// **A checkpoint keeps the clocks**: the tick's enable and deadline, the
/// interval timer's enable, period and deadline. Resumed halfway through an
/// interval, the flag rises at the same instant.
#[test]
fn a_checkpoint_keeps_the_clocks() {
    use muir::checkpoint::{Reader, Writer};
    let prom = wait_and_clear(2);
    let make = || {
        let mut r = Rtl::new(machine(Geometry::QUUX, &prom, 100, [INTERVAL_ON | TICK_ON, INTERVAL_CLEAR]));
        r.boot();
        r
    };
    let mut r = make();
    until(&mut r, 2, 1000, Rtl::ns).unwrap();
    for _ in 0..100 {
        r.step().unwrap();
    }
    let mut w = Writer::new();
    r.save(&mut w);
    let body = w.finish();
    let mut resumed = make();
    resumed.load(&mut Reader::new(&body)).unwrap();
    assert_eq!(until(&mut resumed, 8, 1_000_000, Rtl::ns), until(&mut r, 8, 1_000_000, Rtl::ns));
}
