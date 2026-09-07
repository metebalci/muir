// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! CC's diagnostics run from machine A against machine B over the debug
//! cable, on two `rtl` engines.  Each form is typed at A with its output
//! on a file the Chaosnet server keeps, and read back over the FILE service;
//! both screens are recorded to one GIF as it goes, A's at the left and
//! B's at the right.
//!
//! Two tests:
//!
//! - `cc_quick_diagnostic` runs `cc-test-machine` --- the eighteen data
//!   paths, the fast address test of every scratchpad and control-store
//!   bank, and the SPC pointer, shifter, OA registers, dispatch and clock
//!   --- and then `cc-fast-address-test-mem`, the quick check of main
//!   memory's four banks for one dropping a bit.  Loading CC is some six
//!   minutes of the machine's time and the two diagnostics under one more,
//!   about four minutes of wall clock on `rtl`, so it is ignored by
//!   default.
//!
//! - `cc_full_diagnostics` puts `cc-other-tests` between the two --- the
//!   PC incrementer, the spy IR, arithmetic condition jumps, the gross
//!   data tests and the memory address tests, exhaustive over the
//!   scratchpads: half an hour of the machine's time and about as much
//!   wall clock on `rtl`, so it is ignored by default too.
//!
//! `cc-run-mtest`, the memory-test program CC loads into B and runs there,
//! is left out: exhaustive over B's two million words.
//!
//! Both need the vendored pack and file root; without them they say they
//! were skipped.  Each writes its screenshots, its recording and the
//! diagnostics' output under a name of its own, `cc-diags` and
//! `cc-full-diags`, and the harness runs them one at a time.
//!
//! Run one alone with
//! `cargo test --release --test cc_diags <name> -- --ignored --nocapture`.

mod cc_harness;
mod support;

use muir::engine::Engine;

/// One diagnostic form of the run `run`: run it, report its cost, save its
/// output beside the harness's screenshots, and keep the run's recording
/// up to date.
fn run_one(cc: &mut cc_harness::Cc, run: &str, name: &str, form: &str, limit: u64) {
    let from = cc.l.steps.0;
    let cycles = cc.l.debugger.debug_cycles();
    let out = cc.ask(&format!("{run}-{name}"), form, limit);
    eprintln!(
        "=== {form}: {} microcycles of A, {} debug cycles on the cable ===\n{out}\n=== end of {name} ===",
        cc.l.steps.0 - from,
        cc.l.debugger.debug_cycles() - cycles
    );
    cc.screenshot(&format!("{run}-{name}"));
    cc.save_recording(run);
    assert!(!out.trim().is_empty(), "{name} printed nothing");
    assert_eq!(cc.l.debugger.machine().bus_error, 0, "every cycle of A's was answered by {name}");
}

#[test]
#[ignore = "loads CC and runs cc-test-machine and cc-fast-address-test-mem: about four minutes; run with --ignored"]
fn cc_quick_diagnostic() {
    const RUN: &str = "cc-diags";
    let Some(mut cc) = cc_harness::boot_and_load_cc() else { return };
    cc.screenshot(&format!("{RUN}-loaded"));
    run_one(&mut cc, RUN, "test-machine", "(cadr:cc-test-machine)", 60_000_000_000);
    run_one(&mut cc, RUN, "fast-mem", "(cadr:cc-fast-address-test-mem)", 60_000_000_000);
    cc.save_recording(RUN);
}

#[test]
#[ignore = "loads CC and runs the full diagnostics: half an hour of machine time; run with --ignored"]
fn cc_full_diagnostics() {
    const RUN: &str = "cc-full-diags";
    let Some(mut cc) = cc_harness::boot_and_load_cc() else { return };
    cc.screenshot(&format!("{RUN}-loaded"));
    run_one(&mut cc, RUN, "test-machine", "(cadr:cc-test-machine)", 60_000_000_000);
    run_one(&mut cc, RUN, "other-tests", "(cadr:cc-other-tests)", 120_000_000_000);
    run_one(&mut cc, RUN, "fast-mem", "(cadr:cc-fast-address-test-mem)", 60_000_000_000);
    cc.save_recording(RUN);
}
