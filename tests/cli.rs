// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `muir`'s command line: what it refuses, and that it says why rather
//! than starting a machine it cannot build; the boot PROM a flag puts in
//! the machine; and the flags it reads from a file before it.

mod support;

use std::path::{Path, PathBuf};

use support::{Run, Scratch, muir, scratch, text};

/// A usage error: exit status 2, and the flag named on stderr.
fn refused(args: &[&str], flag: &str) {
    let out = muir().args(args).run();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{args:?}: not a usage error:\n{err}");
    assert!(
        err.lines().next().is_some_and(|l| l.contains(flag)),
        "{args:?}: the first line names {flag}:\n{err}"
    );
}

/// **The memory board count is one to sixty, on every engine.** The
/// Xbus I/O space begins where the sixty-first board would, so sixty is
/// the backplane's most; zero is no memory at all --- the model memory on
/// `chip` is `--main-memory model`. The count sizes main memory on
/// `micro` and `rtl` as it does the backplane on `chip`.
#[test]
fn the_memory_board_count_is_one_to_sixty() {
    for n in ["0", "61", "65"] {
        refused(
            &["--chip", "--main-memory-boards", n, "--stop-after", "1"],
            "--main-memory-boards",
        );
    }
    for (engine, n) in [("--micro", "4"), ("--rtl", "60"), ("--micro", "1")] {
        let out = muir().args([engine, "--main-memory-boards", n, "--stop-after", "1"]).run();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{engine} with {n} boards:\n{err}");
    }
}

/// **A debuggee's pack needs a debuggee.** Like `--debuggee-terminal`,
/// the flag is the other machine's, and without the lashup there is no
/// other machine to give it to.
#[test]
fn a_debuggee_pack_needs_the_lashup() {
    refused(
        &["--rtl", "--debuggee-disk-pack", "nothing.img", "--stop-after", "1"],
        "--debuggee-disk-pack",
    );
}

/// A `.muirrc` of its own for one test, in a directory of its own: the
/// directory, and the file's path under it.
fn muirrc(name: &str, text: &str) -> (Scratch, PathBuf) {
    let dir = scratch(&format!("rc-{name}"));
    let path = dir.join(".muirrc");
    std::fs::write(&path, text).unwrap();
    (dir, path)
}

/// **The flags in the file are the run's**, comments and blank lines
/// apart, and whatever a flag takes is the rest of the line, spaces and
/// all. The start says which flags came from the file, and where from.
#[test]
fn the_flags_in_the_file_are_the_runs() {
    // A directory with a space in its name, for the rest of the line.
    let dir = scratch("rc root");
    // --chaos-file-root resolves what it is given, and the temporary
    // directory is behind a symbolic link on macOS.
    let root = dir.canonicalize().unwrap();
    let (_rc_dir, rc) = muirrc(
        "flags",
        &format!(
            "# what every run of mine wants\n\n--rtl\n--stop-after 10\n--chaos-file-root {}\n",
            dir.display()
        ),
    );
    let out = muir().env("MUIR_RC", &rc).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("engine: rtl"), "the engine came from the file:\n{t}");
    assert!(t.contains("stop: after 10 microcycles"), "and the stop:\n{t}");
    assert!(
        t.contains(&format!("file root {}", root.display())),
        "the rest of the line is one word, spaces and all:\n{t}"
    );
    assert!(t.contains(&format!("from {}", rc.display())), "the start says where from:\n{t}");
    assert!(!t.contains("# what every run"), "the comment is not a flag:\n{t}");
}

/// **What the command line gives replaces what the file gives**, the
/// engine among it: `--micro`, `--rtl` and `--chip` are exclusive of one
/// another, so the file's is dropped rather than refused.
#[test]
fn the_command_line_replaces_the_file() {
    let (_dir, rc) = muirrc("replaced", "--rtl\n--stop-after 10\n");
    let out = muir().env("MUIR_RC", &rc).args(["--micro", "--stop-after", "3"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("engine: micro"), "the command line's engine:\n{t}");
    assert!(t.contains("stop: after 3 microcycles"), "and its stop:\n{t}");
    assert!(!t.contains("flags:"), "nothing was left to come from the file:\n{t}");
}

/// **A file that is not there is no error**: muir runs as if there were
/// none, which is what most runs have.
#[test]
fn no_file_is_no_error() {
    let out =
        muir().env("MUIR_RC", "/no/such/.muirrc").args(["--micro", "--stop-after", "3"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(!t.contains("flags:"), "{t}");
}

/// **`--config` names the file**, `-c` for short, and one named that is
/// not there is a usage error: a file asked for by name is one the run is
/// meant to have.
#[test]
fn config_names_the_file() {
    let (_dir, rc) = muirrc("named", "--micro\n--stop-after 7\n");
    for flag in ["--config", "-c"] {
        let out = muir().args([flag]).arg(&rc).run();
        let t = text(&out);
        assert!(out.status.success(), "{flag}:\n{t}");
        assert!(t.contains("stop: after 7 microcycles"), "{flag}:\n{t}");
        assert!(t.contains(&format!("from {}", rc.display())), "{flag}:\n{t}");
    }
    refused(&["--config", "/no/such/.muirrc"], "--config");
    refused(&["--config"], "--config");
}

/// **The directory muir was run from comes before the home directory, and
/// `--config` before both** --- the first of the three there, not all of
/// them.
#[test]
fn the_run_directory_comes_before_the_home_one() {
    let (home, _) = muirrc("home", "--micro\n--stop-after 11\n");
    let (here, _) = muirrc("here", "--micro\n--stop-after 22\n");
    let (_named_dir, named) = muirrc("named-first", "--micro\n--stop-after 33\n");
    let run = |args: &[&Path]| {
        let mut c = muir();
        c.env_remove("MUIR_RC").env("HOME", home.path()).current_dir(here.path());
        for a in args {
            c.arg("--config").arg(a);
        }
        text(&c.run())
    };
    let t = run(&[]);
    assert!(t.contains("stop: after 22 microcycles"), "the directory muir was run from:\n{t}");
    let t = run(&[&named]);
    assert!(t.contains("stop: after 33 microcycles"), "--config before either:\n{t}");
    // With nothing in the run directory, the home one is what is left.
    let empty = scratch("rc-empty");
    let mut c = muir();
    c.env_remove("MUIR_RC").env("HOME", home.path()).current_dir(&empty);
    let t = text(&c.run());
    assert!(t.contains("stop: after 11 microcycles"), "the home directory:\n{t}");
}

/// **A file of flags cannot name another.** Which file to read is the
/// command line's to say, and a file that could point at another could
/// point at itself.
#[test]
fn a_file_cannot_name_another() {
    let (_dir, rc) = muirrc("recursive", "--micro\n--config /somewhere/else\n");
    let out = muir().arg("--config").arg(&rc).run();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "a usage error:\n{err}");
    assert!(err.contains("cannot name another"), "{err}");
}

/// **A PROM muir cannot read stops the run before it starts.** The file
/// is the program the machine is about to execute, so a run on a file
/// that is missing, or that is not MIT's MCR, is no run at all: it is
/// refused with the flag named, not begun on 512 zero words.
#[test]
fn a_prom_that_cannot_be_read_is_refused() {
    refused(&["--rtl", "--prom", "nothing.mcr", "--stop-after", "1"], "--prom");
    refused(&["--rtl", "--prom", "README.md", "--stop-after", "1"], "--prom");
}

/// **A checkpoint carries the PROM it ran**, all 512 words of it, so
/// `--resume` and `--prom` would be two answers to the same question and
/// the checkpoint's would win silently. Refused instead.
#[test]
fn a_prom_and_a_checkpoint_to_resume_do_not_go_together() {
    refused(&["--rtl", "--prom", "mit/sys/ubin/promh.mcr", "--resume", "nothing.chk"], "--prom");
}

/// **The PROM the machine runs is the file `--prom` names**, on all three
/// engines. One file has to reach all three, and they take it by two
/// different routes: `micro` and `rtl` fetch the microinstruction, `chip`
/// fetches what the 74S472s hold, which is the burned image derived from
/// it.
///
/// The file is MIT's own PROM with word 0 --- the reset vector, a jump to
/// `GO` at 45 --- retargeted at 400. Word 0 is the machine's first fetch,
/// so `--stop-at-prom 400` is reached at once; MIT's own never goes near
/// 400 and runs the window out instead. Which of the two happened is what
/// the end of the run says.
#[test]
fn the_prom_the_machine_runs_is_the_file_named() {
    let mut words: Vec<u64> = muir::prom::boot_prom().iter().map(|i| i.raw()).collect();
    assert_eq!(muir::isa::Insn::new(words[0]).jump().target, 0o45, "the reset vector jumps to GO");
    // `IR<25:12>` is the jump's target, `muir::isa::Insn::jump`.
    words[0] = words[0] & !(0o37777 << 12) | 0o400 << 12;
    let dir = scratch("prom");
    let path = dir.join("retargeted.mcr");
    std::fs::write(&path, support::mcr(&words)).unwrap();

    // `chip` runs its boards as models: what is under test is the PROM
    // reaching the chips, not the rest of the backplane.
    let models = ["--main-memory", "model", "--io-board", "model", "--tv", "model"];
    for engine in ["--micro", "--rtl", "--chip"] {
        let mut c = muir();
        c.args([engine, "--prom", path.to_str().unwrap()]);
        if engine == "--chip" {
            c.args(models);
        }
        let out = c.args(["--stop-after", "100", "--stop-at-prom", "400"]).run();
        let t = text(&out);
        assert!(out.status.success(), "{engine}:\n{t}");
        assert!(t.contains("stopped at PC 400 in the PROM"), "{engine} ran the named PROM:\n{t}");
    }

    // And MIT's own, which the flag is standing in for, does not.
    let out = muir().args(["--micro", "--stop-after", "100", "--stop-at-prom", "400"]).run();
    let t = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(t.contains("stop not reached in 100"), "MIT's own does not go to 400:\n{t}");
}

/// **`--version` says what this build is**, and says it on stdout so a
/// script can read it. The name, the crate's version, and whether it was
/// built with optimisations off --- a run says the same line first, so a
/// report of a run says which muir made it.
#[test]
fn version_says_what_this_build_is() {
    for flag in ["--version", "-V"] {
        let out = muir().arg(flag).run();
        assert!(out.status.success(), "{}", text(&out));
        let said = String::from_utf8_lossy(&out.stdout);
        let said = said.trim();
        let want = format!("muir {}-", env!("CARGO_PKG_VERSION"));
        assert!(said.starts_with(&want), "{flag}: {said:?}");
        assert!(
            said.ends_with("-dev") || said.ends_with("-release"),
            "{flag}: the build kind: {said:?}"
        );
    }
}

/// **A drive on a unit other than 0 wants the multiplexor's board.**
///
/// Not because the unit number is forced to 0. `UNIT<2:0>` reach one
/// 74LS244's inputs on the controller and nothing else --- `cadrdc/dc.wlr`
/// gives a direction per pin and none of the three has a `TO` --- so the
/// board has no driver for them at all, and the six one-board jumpers
/// ground them to stop three inputs floating. Unit 0 is the consequence.
/// The DISK MULTIPLEXOR is what supplies the driver, and
/// `--disk-use-multiplexor` fits it.
///
/// The model controller is behavioural and wants no board: it has
/// addressed eight units all along, `disk_controller::UNITS`, so a pack in
/// unit 3 is its business and it takes one.
#[test]
fn a_drive_past_unit_0_wants_the_multiplexor() {
    let netlist =
        ["--chip", "--main-memory", "netlist", "--disk-controller", "netlist", "--stop-after", "1"];
    let with = |extra: &[&'static str]| -> Vec<&'static str> {
        let mut args = netlist.to_vec();
        args.extend_from_slice(extra);
        args
    };
    refused(&with(&["--disk-pack", "nothing.img,3"]), "--disk-pack");
    // With the board fitted the flag is taken, and the run then stops on
    // the image, which is the next thing wrong with it.
    let out = muir().args(with(&["--disk-use-multiplexor", "--disk-pack", "nothing.img,3"])).run();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_ne!(out.status.code(), Some(2), "the flag is taken:\n{err}");

    // The model controller needs no board and takes the unit.
    let out = muir().args(["--micro", "--disk-pack", "nothing.img,3", "--stop-after", "1"]).run();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_ne!(out.status.code(), Some(2), "the model controller takes unit 3:\n{err}");
}

/// **The multiplexor is a board, so it needs a board to hang off**, and
/// one pack a drive.
#[test]
fn the_multiplexor_is_the_netlist_controllers_board() {
    refused(&["--chip", "--disk-use-multiplexor", "--stop-after", "1"], "--disk-use-multiplexor");
    refused(&["--micro", "--disk-use-multiplexor", "--stop-after", "1"], "--disk-use-multiplexor");
    refused(
        &["--micro", "--disk-pack", "a.img,1", "--disk-pack", "b.img,1", "--stop-after", "1"],
        "--disk-pack",
    );
}
