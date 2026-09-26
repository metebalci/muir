// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! QUUX's clocks in the processor: the tick, timer 0 of the interval timers
//! (contract Q11, `tests/interval_timers.rs`) as the machine's 60-cycle
//! clock in place of the display's vertical interrupt, reached through
//! functional destination 3, the destination 3 alias; and the microsecond
//! clock (contract Q1), functional source 15.
//!
//! Destination 3 is timer 0's control as Q1's tick control was: `<0>` on,
//! and a write with `<1>` set clears its flag; a turn-on makes it periodic
//! and sets its interrupt enable. Its period is the register page's word
//! 111, which the boot PROM writes. Source 15 reads the microseconds since
//! power-on, 32 bits, wrapping. Destination 4 writes only M and source 17
//! reads all ones, on QUUX as on the CADR, where destination 3 too writes
//! only M and source 15 reads all ones.

use muir::engine::Engine;
use muir::isa::Insn;
use muir::isa::asm::{ALU, ALWAYS, JUMP, N, SETM, bit, filler, m_dest, m_src, src, target};
use muir::machine::{Geometry, Machine};
use muir::micro::Micro;
use muir::rtl::Rtl;

mod support;

/// Functional destinations 3 and 4, M 37 written too.
const CLOCK_CONTROL: u64 = (3 << 19) | (0o37 << 14);
/// Q1's interval period, which on QUUX at revision 10 writes only M, as on
/// the CADR.
const INTERVAL_PERIOD: u64 = (4 << 19) | (0o37 << 14);

/// Destination 3's words: the tick on; on and its flag cleared.
const TICK_ON: u32 = 1;
const TICK_CLEAR: u32 = 3;

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
    support::prom_program_in_ram(&mut m);
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

/// Timer 0's period word on the register page, through virtual page 1.
const TIMER_0_PERIOD: u32 = (1 << 8) | 0o111;

/// **The tick through the destination 3 alias**: with timer 0's period
/// written on the page as the boot PROM writes it, 16,667 µs, a write of
/// 1 to destination 3 turns it on and its flag rises 16,667 µs later,
/// read through word 110 `<1>`; a write of 3 clears it. Word 110 reads on,
/// periodic and its interrupt enable set, 0o403 with the flag up, 0o401
/// cleared.
#[test]
fn the_tick_through_the_destination_3_alias() {
    use muir::isa::asm::{MD, SRC_MD, START_READ, START_WRITE};
    let prom = vec![
        // Word 111 gets 16,667, the period.
        Insn::new(ALU | SETM | m_src(1) | MD),
        Insn::new(ALU | SETM | m_src(7) | START_WRITE),
        filler(),
        filler(),
        filler(),
        // The tick on; word 110 read into M 3 until its flag is up.
        Insn::new(ALU | SETM | m_src(2) | CLOCK_CONTROL),
        Insn::new(ALU | SETM | m_src(8) | START_READ),
        filler(),
        filler(),
        Insn::new(ALU | SETM | SRC_MD | m_dest(3)),
        Insn::new(JUMP | target(12) | bit(1) | m_src(3) | N),
        Insn::new(JUMP | target(6) | ALWAYS | N),
        // Cleared; read again into M 5.
        Insn::new(ALU | SETM | m_src(4) | CLOCK_CONTROL),
        Insn::new(ALU | SETM | m_src(8) | START_READ),
        filler(),
        filler(),
        Insn::new(ALU | SETM | SRC_MD | m_dest(5)),
        Insn::new(JUMP | target(17) | ALWAYS | N),
    ];
    let m = || {
        let mut m = machine(Geometry::QUUX, &prom, 16_667, [TICK_ON, TICK_CLEAR]);
        m.l2_map[1] = (1 << 23) | (1 << 22) | 0o36776;
        m.mmem[7] = TIMER_0_PERIOD;
        m.mmem[8] = (1 << 8) | 0o110;
        m
    };
    // `pc()` is the address being fetched, one ahead of the instruction
    // executing: 6 is the turn-on executing, 13 the clear, 18 the end.
    let t = both(m, 13, 200_000_000);
    let quick = both(m, 6, 1_000);
    let done = both(m, 18, 200_000_000);
    for (k, name) in ["micro", "rtl"].into_iter().enumerate() {
        let on = quick[k].0.expect(name);
        let seen = t[k].0.expect(name) - on;
        // The poll loop is six microcycles with a page read.
        assert!(seen.abs_diff(16_667_000) <= 30 * 60, "{name}: the first tick after {seen} ns");
        assert_eq!(
            (done[k].1[3], done[k].1[5]),
            (0o403, 0o401),
            "{name}: word 110 up, then cleared"
        );
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
        assert!(
            want.wrapping_sub(e.machine().mmem[1]) <= 1,
            "micro from {start}: {} against {want}",
            e.machine().mmem[1]
        );
        for model in [
            TimingModel::Sync { cycle_ticks: 4, ilong_ticks: 0 },
            TimingModel::Sync { cycle_ticks: 3, ilong_ticks: 0 },
        ] {
            let mut r = Rtl::new(quux(start));
            r.set_timing_model(model);
            r.boot();
            for _ in 0..40_000 {
                r.step().unwrap();
            }
            let want = (r.ns() / 1000) as u32;
            let got = r.machine().mmem[1];
            assert!(
                want.wrapping_sub(got) <= 1,
                "rtl {model:?} from {start}: {got} against {want}"
            );
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
    for (k, (t, m)) in
        both(|| machine(Geometry::CADR, &prom, 100, [TICK_ON, TICK_CLEAR]), 8, 100_000)
            .into_iter()
            .enumerate()
    {
        let name = ["micro", "rtl"][k];
        assert!(t.is_some(), "{name}: the all-ones source is a flag at once");
        assert_eq!((m[3], m[5]), (!0, !0), "{name}: sources 17 and 15");
        assert_eq!(m[0o37], TICK_CLEAR, "{name}: destination 3 wrote M 37");
    }
}
