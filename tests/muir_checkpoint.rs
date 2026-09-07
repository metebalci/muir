// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `--checkpoint` and `--resume` under `muir` itself: a run writes its
//! state at its stop, another starts from it, and the refusals.  The boot
//! PROM alone is run, so nothing here needs `vendor/`.

mod support;

use support::{Run, muir, scratch, text};

/// `--checkpoint` writes the machine at the stop, `--resume` starts there:
/// the microcycle count carries on, the window counts from the resume.
#[test]
fn a_run_checkpoints_at_its_stop_and_another_resumes_from_it() {
    let dir = scratch("checkpoint");
    let chk = dir.join("a.chk");
    let out = muir().args(["--micro", "--stop-after", "3000", "--checkpoint"]).arg(&chk).run();
    let t = text(&out);
    assert!(out.status.success(), "the first run failed:\n{t}");
    assert!(
        t.contains(&format!("checkpoint: {} at 3000 microcycles", chk.display())),
        "the checkpoint reported:\n{t}"
    );
    assert!(chk.exists());
    let out = muir().args(["--micro", "--stop-after", "1000", "--resume"]).arg(&chk).run();
    let t = text(&out);
    assert!(out.status.success(), "the resumed run failed:\n{t}");
    assert!(
        t.contains(&format!("resumed: {} at 3000 microcycles", chk.display())),
        "the resume reported:\n{t}"
    );
    assert!(t.contains("ran out at 1000"), "the window counts from the resume:\n{t}");
    let out = muir().args(["--rtl", "--stop-after", "1", "--resume"]).arg(&chk).run();
    assert!(!out.status.success());
    assert!(text(&out).contains("a micro checkpoint"), "{}", text(&out));
}

/// A resume builds the machine with as much memory as the checkpoint's
/// had, and `--main-memory-boards` may only agree.
#[test]
fn a_resume_has_the_checkpoint_s_memory() {
    let dir = scratch("checkpoint-memory");
    let chk = dir.join("four.chk");
    let out = muir()
        .args(["--micro", "--main-memory-boards", "4", "--stop-after", "100", "--checkpoint"])
        .arg(&chk)
        .run();
    assert!(out.status.success(), "{}", text(&out));
    let out = muir().args(["--micro", "--stop-after", "10", "--resume"]).arg(&chk).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("100 microcycles") && t.contains("4 memory boards"), "{t}");
    let out = muir()
        .args(["--micro", "--main-memory-boards", "8", "--stop-after", "10", "--resume"])
        .arg(&chk)
        .run();
    assert!(!out.status.success());
    assert!(text(&out).contains("4 memory boards"), "{}", text(&out));
}

/// **A checkpoint is one machine on `micro` or `rtl`**: `chip` has no
/// checkpoint, and the lashup none of its two machines.  Both refusals are
/// run from a scratch directory, where the file named would land were
/// either run to go ahead.
#[test]
fn checkpoints_are_one_machine_on_micro_or_rtl() {
    let dir = scratch("checkpoint-refused");
    let out = muir()
        .args(["--chip", "--stop-after", "1", "--checkpoint", "x.chk"])
        .current_dir(&dir)
        .run();
    assert!(!out.status.success());
    assert!(text(&out).contains("not chip"), "{}", text(&out));
    let out = muir()
        .args(["--rtl", "--debug-in-process", "--stop-after", "1", "--resume", "x.chk"])
        .current_dir(&dir)
        .run();
    assert!(!out.status.success());
    assert!(text(&out).contains("not the lashup"), "{}", text(&out));
    assert!(!dir.join("x.chk").exists(), "neither run wrote the file");
}
