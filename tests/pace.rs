// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **`--pace` runs the machine at the machine's own speed**, end to end:
//! muir started with the flag takes something like the time the machine it
//! is simulating would have taken, instead of getting there nine times
//! faster.
//!
//! The rule the pacing follows is [`Pace`]'s and is held by the unit tests
//! beside it in `src/main.rs`, which pass the instants in and need no
//! clock. What is left for here is the one thing those cannot see: that
//! the run loop asks and then actually waits.

mod support;

use std::time::{Duration, Instant};

use support::{Run, muir, muir_default, text};

/// **A paced run takes about the machine's own time**, and this holds it to
/// a third of that.
///
/// The bound is loose on purpose, and in one direction only. Pacing can
/// only ever make a run *longer* --- it waits, and it never sprints --- so
/// the failure worth catching is a paced run that did not wait at all, and
/// against `micro`'s nine times hardware a third of the machine's time is
/// as good a catch as nine tenths would be. A tight bound would be a
/// measurement of the host rather than of muir: a loaded machine, a
/// scheduler that rounds a sleep up, another test's run beside this one.
/// So nothing here says how *close* to the machine's speed the run came.
///
/// 4 M microcycles is the machine's 145 ns each at the very least. This run
/// spends all of them in the boot PROM with no pack, where the mode
/// register has not been written and a microcycle is 220 ns, so the
/// machine's own time is nearer 0.9 s than the 0.58 s the bound is taken
/// from --- conservative twice over.
#[test]
fn a_paced_run_waits_for_the_machine_it_is_running_ahead_of() {
    const MICROCYCLES: u64 = 4_000_000;
    let least = Duration::from_nanos(MICROCYCLES * 145) / 3;

    let began = Instant::now();
    let out = muir().args(["--micro", "--pace", "--stop-after", "4000000"]).run();
    let took = began.elapsed();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("pace: the machine's own speed"), "the start says it is paced:\n{t}");
    assert!(
        took >= least,
        "a paced run of {MICROCYCLES} microcycles took {took:?}, less than the {least:?} \
         that is a third of the machine's own time:\n{t}"
    );
}

/// **`rtl` and `chip` run at the machine's own speed unless told not to,
/// and `micro` as fast as the host takes it unless told to.**
///
/// The machine's own nanoseconds are what the band's clock counts, so a run
/// that got ahead of them would keep the band's time ahead of the day. `rtl`
/// is the engine for working the machine; `chip` is slower than the machine
/// and never waits, so pacing it costs nothing; `micro` is the fast engine.
/// `--pace` and `--no-pace` say otherwise, and of the two the last given
/// wins. The start says which a run is.
#[test]
fn micro_is_the_engine_that_is_not_paced_unless_told_to() {
    let models = ["--main-memory", "model", "--io-board", "model", "--disk-controller", "model"];
    let paced = |args: &[&str]| {
        let out = muir_default()
            .arg("--no-debug-cable-listen")
            .args(args)
            .args(["--stop-after", "1"])
            .run();
        let t = text(&out);
        assert!(out.status.success(), "{args:?}:\n{t}");
        t.contains("pace: the machine's own speed")
    };
    assert!(paced(&[]), "rtl, the engine a bare run is, paces");
    assert!(paced(&["--rtl"]), "rtl paces");
    assert!(!paced(&["--rtl", "--no-pace"]), "unless told not to");
    assert!(!paced(&["--micro"]), "micro does not");
    assert!(paced(&["--micro", "--pace"]), "unless told to");
    let chip: Vec<&str> = ["--chip"].into_iter().chain(models).collect();
    assert!(paced(&chip), "chip paces, and never waits");
    assert!(paced(&["--rtl", "--no-pace", "--pace"]), "the last given wins");
    assert!(!paced(&["--rtl", "--pace", "--no-pace"]), "either way round");
}
