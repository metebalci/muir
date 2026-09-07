// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Checks the clock generator model against the CLOCK1/CLOCK2 wiring.

use muir::clock::{Behavioural, Clock, Inputs, Outputs, Speed};

/// The eight taps the 74S151 at CLOCK1 1D08 selects between. This is the
/// solid part of the clock model: it is read directly off the mux wiring.
#[test]
fn speed_table_matches_the_mux() {
    let cases = [
        (Speed::Fast, false, 75),
        (Speed::Fast, true, 115),
        (Speed::Normal, false, 85),
        (Speed::Normal, true, 125),
        (Speed::Slow, false, 100),
        (Speed::Slow, true, 140),
        (Speed::ExtraSlow, false, 160),
        (Speed::ExtraSlow, true, 160),
    ];
    for (speed, ilong, want) in cases {
        assert_eq!(speed.read_phase_ns(ilong), want, "{speed:?} ilong={ilong}");
    }
    // ILONG adds exactly 40 ns wherever the chain has the room.
    for speed in [Speed::Fast, Speed::Normal, Speed::Slow] {
        assert_eq!(
            speed.read_phase_ns(true) - speed.read_phase_ns(false),
            40,
            "{speed:?}: ILONG should add 40 ns"
        );
    }
}

fn run_cycle(speed: Speed, ilong: bool) -> Vec<(u64, Outputs)> {
    let mut c = Behavioural::new();
    let inputs = Inputs { machrun: true, hang: false, ilong, speed, reset: false };
    let mut seen = Vec::new();
    // Two cycles' worth, so the period can be measured: ten transitions a
    // cycle, `-TPR60` and the tap selection among them.
    for _ in 0..24 {
        let (_, out) = c.advance(inputs);
        seen.push((c.time_ns(), out));
    }
    seen
}

/// One cycle in order: TPCLK rises, TPTSE drops briefly at the start and is
/// up for the rest, then at -TPREND the read phase ends, the write pulse
/// follows, and the pulse closes as the next cycle begins.
///
/// `TPTSE` is *cleared* by `-TPR5` and *set* by `-TPR25`, not the other way
/// round --- the pair at CLOCK2 1C06/1C07 is a NAND latch, and the tap
/// numbers do not say which input is set and which is reset. It matters: the
/// `-TSE1..4` it becomes enable every tri-state driver on the M and A buses,
/// so a twenty-nanosecond enable leaves those buses floating for the rest of
/// the cycle.
#[test]
fn a_cycle_runs_in_the_right_order() {
    let trace = run_cycle(Speed::Normal, false);
    let at = |t: u64| trace.iter().find(|&&(x, _)| x == t).map(|&(_, o)| o);

    assert!(at(0).unwrap().tpclk, "TPCLK should be up at -TPR0");
    assert!(!at(5).unwrap().tptse, "-TPR5 clears TPTSE");
    assert!(at(25).unwrap().tptse, "-TPR25 sets it again");
    assert!(at(85).unwrap().tptse, "and it stays up through the read phase");
    assert!(at(25).unwrap().tpclk, "still in the read phase at -TPR25");

    // -TPREND at 85 ns for normal speed.
    assert!(!at(85).unwrap().tpclk, "read phase should end at -TPREND");
    assert!(at(85).unwrap().tpwpiram, "control store write opens at -TPREND");
    assert!(at(115).unwrap().tpwp, "write pulse at -TPREND + 30");
    // -TPW60 restarts the cycle ten nanoseconds before -TPW70 would end the
    // write pulse. The pulse is cut at the boundary instead, and it ends in
    // its own step so that the parts see it close *before* the clock edge:
    // with no gate delays a pulse that outlives the boundary writes at the
    // next instruction's source address. See `WP_OFF_NS` in `src/clock.rs`.
    let both: Vec<_> = trace.iter().filter(|&&(t, _)| t == 145).map(|&(_, o)| o).collect();
    assert_eq!(both.len(), 2, "two transitions at -TPREND + 60");
    assert!(!both[0].tpwp, "the write pulse ends first");
    assert!(!both[0].tpclk, "and the clock has not risen yet");
    assert!(both[1].tpclk, "then the next cycle starts");
    assert!(!both[1].tpwp, "with no write pulse left over");
}

/// The cycle period, which is what the microcycle time actually is.
#[test]
fn cycle_period() {
    for speed in [Speed::Fast, Speed::Normal, Speed::Slow, Speed::ExtraSlow] {
        for ilong in [false, true] {
            let trace = run_cycle(speed, ilong);
            // TPCLK rises once per cycle; the gap between rises is the period.
            let rises: Vec<u64> = trace
                .windows(2)
                .filter(|w| !w[0].1.tpclk && w[1].1.tpclk)
                .map(|w| w[1].0)
                .collect();
            let period = rises.windows(2).map(|w| w[1] - w[0]).next();
            let read = speed.read_phase_ns(ilong);
            eprintln!("{speed:?} ilong={ilong}: read {read} ns, period {period:?} ns");
            // The cycle restarts at -TPW60, so the period is the read phase plus
            // sixty nanoseconds.
            //
            // `Speed::cycle_ns` is the same arithmetic as a function, and it is
            // what the `rtl` engine accumulates instead of ticking a clock, so
            // it is checked here against the generator that does tick.
            assert_eq!(period, Some(read as u64 + 60), "{speed:?}");
            assert_eq!(period, Some(speed.cycle_ns(ilong) as u64), "{speed:?} cycle_ns");
        }
    }
}

/// -HANG holds the next cycle off at its start. That is how a memory wait
/// stretches a microcycle without disturbing the phase relationships.
#[test]
fn hang_stalls_at_the_start_of_a_cycle() {
    let mut c = Behavioural::new();
    let held = Inputs { machrun: true, hang: true, speed: Speed::Normal, ..Inputs::default() };
    for _ in 0..4 {
        let (dt, _) = c.advance(held);
        assert_eq!(dt, 0, "time should not advance while hung at -TPR0");
    }
    assert_eq!(c.phase_ns(), 0);

    // Releasing -HANG emits the -TPR0 event itself, which is at zero
    // nanoseconds into the cycle, so no time passes on that step.
    let free = Inputs { hang: false, ..held };
    let (dt, out) = c.advance(free);
    assert_eq!(dt, 0, "the -TPR0 event is the start of the cycle");
    assert!(out.tpclk, "releasing -HANG should start the read phase");
    let (dt, _) = c.advance(free);
    assert!(dt > 0, "time should advance once the cycle is running");
}
