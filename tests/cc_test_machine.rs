// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `cc-test-machine` run from machine A against machine B over the debug
//! cable, both screens recorded on one canvas to `tmp/lashup.gif`: A's ---
//! the debugger's, where CC is typed and its report comes out --- at the
//! left, and B's, the machine under test, at the right.
//!
//! The lashup, the boot and the load of CC are `tests/cc_harness`, which
//! samples the pair as it steps them.  What this adds to
//! `cc_quick_diagnostic` in `tests/cc_diags.rs` is where the recording
//! goes; the diagnostic itself is the same eighteen data paths, the fast
//! address test of every scratchpad and control-store bank, and the SPC
//! pointer, shifter, OA registers, dispatch and clock.
//!
//! B boots the same pack as A --- the one image, read-only under both
//! drives --- so that the right half of the canvas is a machine and not a
//! blank screen.  B has no Chaosnet, so its boot stops in the cold-load
//! debugger asking for the date, and that is where CC finds it.
//!
//! The clocks are on: the diagnostic's report goes to the file over the
//! FILE service and not to a screen, so for the fifty-odd seconds of
//! machine time it runs neither screen changes.  The line below them ticks
//! a frame a second, which is how a still recording is told from a stopped
//! one.
//!
//! Loading CC is some six minutes of the machine's time, so it is ignored
//! by default.  Needs the vendored pack and file root; without them it
//! says it was skipped.
//!
//!     cargo test --release --test cc_test_machine -- --ignored --nocapture

mod cc_harness;
mod support;

use std::path::PathBuf;

use muir::capture::PAIR_RULE;
use muir::engine::Engine;
use muir::simpletv::WIDTH;

#[test]
#[ignore = "loads CC and runs cc-test-machine, recording both screens: minutes; run with --ignored"]
fn cc_test_machine_runs_against_b_with_both_screens_recorded() {
    let Some(mut cc) = cc_harness::boot_and_load_cc_with_debuggee_pack() else { return };
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp");
    std::fs::create_dir_all(&dir).unwrap();
    let gif = dir.join("lashup.gif");

    let from = cc.l.steps.0;
    let before = cc.l.debugger.debug_cycles();
    let out = cc.ask("test-machine", "(cadr:cc-test-machine)", 60_000_000_000);
    let cycles = cc.l.debugger.debug_cycles() - before;
    eprintln!(
        "=== (cadr:cc-test-machine): {} microcycles of A, {cycles} debug cycles on the cable ===\n{out}\n=== end of cc-test-machine ===",
        cc.l.steps.0 - from
    );
    cc.write_recording(&gif);

    assert!(!out.trim().is_empty(), "the diagnostic printed nothing");
    assert!(cycles > 0, "the diagnostic reached B over the cable");
    assert_eq!(cc.l.debugger.machine().bus_error, 0, "every cycle of A's was answered");
    assert!(cc.rec.frames() > 1, "the screens changed as the diagnostic ran");
    let bytes = std::fs::read(&gif).unwrap();
    assert!(bytes.starts_with(b"GIF89a"), "{} is a GIF", gif.display());
    let width = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;
    assert_eq!(width, 2 * WIDTH + PAIR_RULE, "both screens on one canvas");
}
