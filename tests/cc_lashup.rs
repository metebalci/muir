// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The acceptance test: CC in machine A's band debugging machine B over
//! the debug cable.  Two `rtl` machines in one process, A booting System
//! 100 from the pack with the Chaosnet and the Chaosnet server serving the
//! release as `SYS:`, B running the boot PROM with no pack.  A is typed
//! `(login 'lispm)`, `(make-system 'cc :noconfirm :nowarn)` --- CC is not
//! in the band, and its compiled files come over the FILE service --- and
//! `(cadr:cc)`, whose first act is to read B's PC, IR, O bus and error
//! status through the cable and show them.
//!
//! Ignored by default: loading CC takes some six minutes of the machine's
//! time, ten of wall time at `rtl`'s rate with two machines.  Run it with
//! `cargo test --test cc_lashup -- --ignored --nocapture`.  Needs the
//! vendored pack and file root; without them it says it was skipped.

mod cc_harness;
mod support;

use muir::engine::Engine;
use muir::spy;

#[test]
#[ignore = "loads CC over the network: minutes; run with --ignored"]
fn cc_in_machine_a_reads_machine_b_through_the_cable() {
    let Some(mut cc) = cc_harness::boot_and_load_cc() else { return };
    let b_pc_before = cc.l.debuggee.pc();

    // CC's entry reads B: PC, IR, O bus, the error status, the debug
    // status, through the cable.
    cc.type_line("(cadr:cc)");
    cc.run(100_000_000);
    let cycles = cc.l.debugger.debug_cycles();
    eprintln!(
        "after (cadr:cc): {cycles} debug cycles on the cable; B at PC {:o}{}, {}, was at {b_pc_before:o}",
        cc.l.debuggee.pc(),
        if cc.l.debuggee.machine().mode.prom_disable { "" } else { " in the PROM" },
        if cc.l.debuggee.spy_read(spy::FLAG_1) & 0x100 != 0 { "running" } else { "halted" }
    );
    cc.screenshot("screen-cc");
    cc.save_recording("cc-lashup");
    assert!(cycles > 0, "CC made no debug cycle on the cable");
    assert_eq!(cc.l.debugger.machine().bus_error, 0, "every cycle of A's was answered");
}
