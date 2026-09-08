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

/// **A checkpoint is one machine**: the lashup has two and writes none.
/// Run from a scratch directory, where the file named would land were the
/// run to go ahead.
#[test]
fn a_checkpoint_is_one_machine() {
    let dir = scratch("checkpoint-refused");
    let out = muir()
        .args(["--rtl", "--debug-in-process", "--stop-after", "1", "--resume", "x.chk"])
        .current_dir(&dir)
        .run();
    assert!(!out.status.success());
    assert!(text(&out).contains("not the lashup"), "{}", text(&out));
    assert!(!dir.join("x.chk").exists(), "the run wrote no file");
}

/// **A `chip` checkpoint carries the netlist controller's drives.** They
/// are on the controller's own cable rather than in the machine's model,
/// so before this the pairing was refused outright; now the two runs
/// together are the run that was never stopped, as they are with the model
/// controller.
///
/// The pack is made here by `diskpack`, so nothing in this test needs
/// `vendor/`: a fresh T-300 label on a sparse file, which the boot PROM
/// finds no microload on and does not need to. What matters is that there
/// is a drive on the cable at all --- a spindle turning, an arm, and a
/// controller talking to it --- for the checkpoint to have something to
/// carry.
#[test]
fn chip_checkpoints_the_netlist_controllers_drives() {
    let dir = scratch("checkpoint-drives");
    let pack = dir.join("t300.img");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_diskpack"))
        .arg(&pack)
        .arg("initialize")
        .run();
    assert!(out.status.success(), "the pack was not made:\n{}", text(&out));
    let chk = dir.join("drives.chk");
    let chip = [
        "--chip",
        "--main-memory-boards",
        "4",
        "--disk-controller",
        "netlist",
        "--disk-pack",
        pack.to_str().unwrap(),
    ];
    let out = muir().args(chip).args(["--stop-after", "300", "--checkpoint"]).arg(&chk).run();
    let t = text(&out);
    assert!(out.status.success(), "the first run failed:\n{t}");
    assert!(chk.exists(), "the checkpoint was written:\n{t}");
    let out = muir().args(chip).args(["--stop-after", "200", "--resume"]).arg(&chk).run();
    let resumed = text(&out);
    assert!(out.status.success(), "the resumed run failed:\n{resumed}");
    let out = muir().args(chip).args(["--stop-after", "500"]).run();
    let straight = text(&out);
    assert!(out.status.success(), "the straight run failed:\n{straight}");
    let stop = |t: &str| {
        t.lines()
            .find_map(|l| l.trim().strip_prefix("ran out at ").map(|r| r.to_string()))
            .unwrap_or_else(|| panic!("no stop line in:\n{t}"))
    };
    assert_eq!(
        stop(&resumed).split_once("; ").map(|(_, at)| at),
        stop(&straight).split_once("; ").map(|(_, at)| at),
        "the resumed run is where the straight one is"
    );
}

/// **`chip` checkpoints and resumes like the others**, and the two runs
/// together are the run that was never stopped: the same PC at the same
/// microcycle as a straight run of both windows.  The boot PROM alone, so
/// nothing here needs `vendor/`, and four memory boards rather than
/// thirty-two to keep the boards to build down.  Then the two refusals a
/// checkpoint's own header cannot make: another engine's, and the display
/// board the backplane was built with.
#[test]
fn chip_checkpoints_at_its_stop_and_another_resumes_from_it() {
    let dir = scratch("checkpoint-chip");
    let chk = dir.join("chip.chk");
    let chip = ["--chip", "--main-memory-boards", "4"];
    let out = muir().args(chip).args(["--stop-after", "300", "--checkpoint"]).arg(&chk).run();
    let t = text(&out);
    assert!(out.status.success(), "the first run failed:\n{t}");
    assert!(
        t.contains(&format!("checkpoint: {} at 300 microcycles", chk.display())),
        "the checkpoint reported at the microcycle it was taken at:\n{t}"
    );
    let out = muir().args(chip).args(["--stop-after", "200", "--resume"]).arg(&chk).run();
    let resumed = text(&out);
    assert!(out.status.success(), "the resumed run failed:\n{resumed}");
    assert!(
        resumed.contains(&format!("resumed: {} at 300 microcycles", chk.display())),
        "the resume reported:\n{resumed}"
    );
    assert!(resumed.contains("ran out at 200"), "the window counts from the resume:\n{resumed}");
    // The same place a run that was never stopped is at.
    let out = muir().args(chip).args(["--stop-after", "500"]).run();
    let straight = text(&out);
    assert!(out.status.success(), "the straight run failed:\n{straight}");
    let pc = |t: &str| {
        t.lines()
            .find_map(|l| l.trim().strip_prefix("ran out at ").map(|r| r.to_string()))
            .unwrap_or_else(|| panic!("no stop line in:\n{t}"))
    };
    assert_eq!(
        pc(&resumed).split_once("; ").map(|(_, at)| at),
        pc(&straight).split_once("; ").map(|(_, at)| at),
        "the resumed run is where the straight one is"
    );
    // A checkpoint of one machine does not load onto another engine, and
    // the engine it names is what it says.
    let out = muir().args(["--rtl", "--stop-after", "1", "--resume"]).arg(&chk).run();
    assert!(!out.status.success());
    assert!(text(&out).contains("a chip checkpoint"), "{}", text(&out));
    let out = muir()
        .args(chip)
        .args(["--tv-board", "lispm-tv", "--stop-after", "1", "--resume"])
        .arg(&chk)
        .run();
    assert!(!out.status.success());
    assert!(text(&out).contains("--tv-board"), "{}", text(&out));
}
