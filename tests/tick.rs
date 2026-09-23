// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! QUUX's processor tick, revision 4: a periodic flag in the processor that
//! takes the place of the display's vertical interrupt as the machine's
//! clock.
//!
//! Functional destination 3 is its control: bit 0 enables it, and a write
//! with bit 1 set clears the flag. Destination 4 is its period in
//! microseconds, `<23:0>`, and a write starts a period from then. Functional
//! source 17 reads bit 0 the flag and bit 1 the enable. While enabled and
//! set, the flag is one more term of the interrupt the microcode already
//! tests in jump conditions 5 and 6. On the CADR destinations 3 and 4 write
//! only M and source 17 reads all ones.

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
const TICK_CONTROL: u64 = (3 << 19) | (0o37 << 14);
const TICK_PERIOD: u64 = (4 << 19) | (0o37 << 14);
/// Jump condition 5: a page fault or an interrupt pending.
const PGF_OR_INT: u64 = (1 << 5) | 5;

/// Sets the period to M 1 (unless `period` is false), enables, waits for
/// the flag reading source 17 into M 3, then clears it and reads source 17
/// into M 5, and stops at 7.
fn wait_and_clear(period: bool) -> Vec<Insn> {
    vec![
        Insn::new(
            ALU | SETM
                | m_src(if period { 1 } else { 0o37 })
                | if period { TICK_PERIOD } else { 0 },
        ),
        Insn::new(ALU | SETM | m_src(2) | TICK_CONTROL),
        Insn::new(ALU | SETM | src(0o17) | m_dest(3)),
        Insn::new(JUMP | target(5) | bit(0) | m_src(3) | N),
        Insn::new(JUMP | target(2) | ALWAYS | N),
        Insn::new(ALU | SETM | m_src(4) | TICK_CONTROL),
        Insn::new(ALU | SETM | src(0o17) | m_dest(5)),
        Insn::new(JUMP | target(7) | ALWAYS | N),
    ]
}

fn machine(geometry: Geometry, prom: &[Insn], period_us: u32) -> Machine {
    let mut m = Machine::new();
    let mut words = vec![filler(); 512];
    words[..prom.len()].copy_from_slice(prom);
    m.load_prom(&words);
    m.geometry = geometry;
    m.mmem[1] = period_us;
    m.mmem[2] = 1;
    m.mmem[4] = 3;
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
fn both(
    geometry: Geometry,
    prom: &[Insn],
    period_us: u32,
    pc: u16,
    limit: u64,
) -> [(Option<u64>, Vec<u32>); 2] {
    let mut e = Micro::new(machine(geometry, prom, period_us));
    e.boot();
    let te = until(&mut e, pc, limit, micro_ns);
    let mut r = Rtl::new(machine(geometry, prom, period_us));
    r.boot();
    let tr = until(&mut r, pc, limit, Rtl::ns);
    [(te, e.machine().mmem.to_vec()), (tr, r.machine().mmem.to_vec())]
}

/// **The flag rises one period after the tick is enabled, and a write
/// clears it.** Timed as the difference between a 100 and a 300 µs period,
/// which leaves out the boot: 200 µs, give or take the three-microcycle
/// wait loop. Source 17 reads 3 with the flag up, and 2 once cleared.
#[test]
fn the_flag_rises_each_period_and_a_write_clears_it() {
    let prom = wait_and_clear(true);
    let short = both(Geometry::QUUX, &prom, 100, 8, 10_000_000);
    let long = both(Geometry::QUUX, &prom, 300, 8, 10_000_000);
    for (k, name) in ["micro", "rtl"].into_iter().enumerate() {
        let (ts, ms) = &short[k];
        let (tl, _) = &long[k];
        let (ts, tl) = (ts.expect(name), tl.expect(name));
        assert!(ts >= 100_000, "{name}: the flag rose at {ts} ns, before a period");
        let diff = tl - ts;
        assert!(diff.abs_diff(200_000) <= 3 * 220, "{name}: 200 µs more took {diff} ns");
        assert_eq!((ms[3], ms[5]), (3, 2), "{name}: source 17 up, then cleared");
    }
}

/// **The period starts at 16,667 µs, 60 Hz**, until the microcode writes
/// one.
#[test]
fn the_period_starts_at_60_hz() {
    let prom = wait_and_clear(false);
    let t = both(Geometry::QUUX, &prom, 0, 8, 200_000_000);
    let quick = both(Geometry::QUUX, &prom, 0, 3, 1_000);
    for (k, name) in ["micro", "rtl"].into_iter().enumerate() {
        let enabled = quick[k].0.expect(name);
        let seen = t[k].0.expect(name) - enabled;
        // The enable lands a microcycle or two after `PC` passes 3, and the
        // wait loop is three microcycles, at the boot's 220 ns.
        assert!(seen.abs_diff(16_667_000) <= 8 * 220, "{name}: the first tick after {seen} ns");
    }
}

/// **A pending tick is an interrupt the microcode's jump sees**: with
/// `INT.ENABLE` up, condition 5 is taken once the flag rises, and not while
/// the tick is off. Condition 5 is a page fault too, and `VMAOK` is down
/// after the boot, so the program first reads a page the map permits:
/// page 0, readable and writable in level-2 entry 0.
#[test]
fn a_pending_tick_takes_the_interrupt_condition() {
    let prom = |enable: u64| {
        vec![
            Insn::new(ALU | SETZ | START_READ),
            Insn::new(ALU | SETM | m_src(1) | TICK_PERIOD),
            Insn::new(ALU | SETM | m_src(enable) | TICK_CONTROL),
            Insn::new(ALU | SETM | m_src(6) | INTERRUPT_CONTROL),
            Insn::new(JUMP | target(6) | PGF_OR_INT | N),
            Insn::new(JUMP | target(4) | ALWAYS | N),
            Insn::new(JUMP | target(6) | ALWAYS | N),
        ]
    };
    // M 2 holds 1, enabling; M 0 holds 0.
    let on = both(Geometry::QUUX, &prom(2), 50, 7, 1_000_000);
    let off = both(Geometry::QUUX, &prom(0), 50, 7, 1_000_000);
    for (k, name) in ["micro", "rtl"].into_iter().enumerate() {
        assert!(on[k].0.is_some(), "{name}: the tick never took condition 5");
        assert!(off[k].0.is_none(), "{name}: condition 5 taken with the tick off");
    }
}

/// **The CADR has no tick**: source 17 reads all ones and destinations 3
/// and 4 write only M, so nothing ever pends.
#[test]
fn the_cadr_has_no_tick() {
    let prom = wait_and_clear(true);
    for (k, (t, m)) in both(Geometry::CADR, &prom, 100, 8, 100_000).into_iter().enumerate() {
        let name = ["micro", "rtl"][k];
        assert!(t.is_some(), "{name}: the all-ones source is a flag at once");
        assert_eq!((m[3], m[5]), (!0, !0), "{name}: source 17");
        assert_eq!(m[0o37], 3, "{name}: destination 3 wrote M 37");
    }
}

/// **A checkpoint keeps the tick**: its enable, period and next deadline,
/// checkpoint format 32. Resumed halfway through a period, the flag rises at
/// the same instant.
#[test]
fn a_checkpoint_keeps_the_tick() {
    use muir::checkpoint::{Reader, Writer};
    let prom = wait_and_clear(true);
    let make = || {
        let mut r = Rtl::new(machine(Geometry::QUUX, &prom, 100));
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
