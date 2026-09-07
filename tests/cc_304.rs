// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! CC on **System 304**, the release that continues the target.
//!
//! It did not load there.  `cc/lcadrd.lisp`, `cc/diags.lisp` and
//! `cc/ldbg.lisp` called `MAKE-ARRAY` in the old positional form ---
//! `(MAKE-ARRAY NIL 'ART-Q '(8))` --- seven times between them, and System
//! 304 took that form out, so the load of `LCADRD QFASL` stopped at
//! "ART-Q is not a known MAKE-ARRAY keyword".  The three calls were
//! rewritten upstream on 7 September 2026, in the three check-ins ending
//! `1af7716f24ba298f`, which is the check-in
//! `tools/fetch-system-304.sh` now builds the sources from.
//!
//! So this is the test of that fix, and the one thing that could tell us
//! it had arrived: A boots System 304's pack with the Chaosnet server
//! serving those sources as `SYS:`, is logged in, and is asked to compile
//! and load the whole `CADR-DEBUGGER` system --- the release ships no
//! QFASL for it --- and then whether `CADR:CC` is there.  With CC loaded,
//! `(cadr:cc)` is typed and B, on the debug cable, is read through it.
//!
//! Ignored by default, and the longest test here: the sixteen files are
//! compiled on the machine itself.  Measured, with three of them already
//! compiled from an earlier run: 8,997,852,000 microcycles of A's for
//! `make-system`, about twenty-seven minutes of the machine's time, and
//! 1,424 seconds of wall clock for the whole test at `rtl`'s rate with
//! two machines.  Run it with
//!
//!     cargo test --release --test cc_304 -- --ignored --nocapture
//!
//! The compiled files are left in the release's own `cc/` --- that is
//! where the band wrote them --- so a second run loads them instead of
//! compiling again.  Needs the System 304 pack and the file root, which
//! `tools/fetch-system-304.sh` puts in place; without them it says it was
//! skipped.

mod cc_harness;
mod support;

use muir::engine::Engine;

use cc_harness::Release;

/// Microcycles of A's with neither cable moving that the compile is
/// allowed: about twenty-four minutes of the machine's time.
///
/// A file is read over the network, compiled with nothing to say to it,
/// and written back, so the quiet stretch is one file's compilation and
/// the harness's usual minute and a half would call it a stall. The
/// longest of the sixteen is `cc/cc.lisp` at 121 KB, an order above the
/// 8 KB of `cc/lcadmc.lisp`, which took about a minute of the machine's
/// time here.
const COMPILING: u64 = 8_000_000_000;

#[test]
#[ignore = "compiles CC on the machine: the better part of an hour; run with --ignored"]
fn cc_compiles_and_loads_on_system_304() {
    let Some(mut cc) = cc_harness::boot_and_login_on(Release::System304, false) else { return };

    // The login took, and the file service answers both ways: everything
    // after this is minutes long, and a run that is not logged in has
    // nowhere to compile to.
    let who = cc.ask("cc-304-login", "(princ si:user-id)", 2_000_000_000);
    eprintln!("logged in as {who:?} after {} microcycles", cc.l.steps.0);
    assert!(who.contains("LISPM"), "logged in: {who:?}");

    // `:compile` because System 304 ships the sources alone.  Its output
    // is the compiler's, kept as the run's own file.
    cc.stall = COMPILING;
    let from = cc.l.steps.0;
    let out = cc.ask(
        "cc-304-make-system",
        "(make-system 'cc :compile :noconfirm :nowarn)",
        20_000_000_000,
    );
    cc.stall = cc_harness::Cc::STALL;
    eprintln!(
        "make-system took {} microcycles and printed {} bytes",
        cc.l.steps.0 - from,
        out.len()
    );
    cc.screenshot("cc-304-make-system");

    // The error this whole test is about, in the compiler's own words.
    assert!(
        !out.contains("not a known MAKE-ARRAY keyword"),
        "the old MAKE-ARRAY form is still in CC's sources:\n{}",
        &out[out.len().saturating_sub(2000)..]
    );

    let loaded = cc.ask(
        "cc-304-loaded",
        "(princ (if (fboundp 'cadr:cc) 'cc-loaded 'cc-missing))",
        2_000_000_000,
    );
    eprintln!("CADR:CC after the load: {loaded:?}");
    assert!(loaded.contains("CC-LOADED"), "CC did not load: {loaded:?}");

    // And it runs: CC's entry reads B --- PC, IR, O bus, error status ---
    // through the debug cable, as it does on the target's band.
    let b_pc_before = cc.l.debuggee.pc();
    cc.type_line("(cadr:cc)");
    cc.run(200_000_000);
    let cycles = cc.l.debugger.debug_cycles();
    eprintln!(
        "after (cadr:cc): {cycles} debug cycles on the cable; B at PC {:o}, was at {b_pc_before:o}",
        cc.l.debuggee.pc()
    );
    cc.screenshot("cc-304");
    cc.save_recording("cc-304");
    assert!(cycles > 0, "CC made no debug cycle on the cable");
    assert_eq!(cc.l.debugger.machine().bus_error, 0, "every cycle of A's was answered");
}
