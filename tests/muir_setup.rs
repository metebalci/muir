// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What `muir` says a run is when it starts: the engine, the memory, the
//! boards, the pack, the Chaosnet, the terminal, the stops, the prompt;
//! and the prompt's `info`, which says it again.

mod support;

use std::io::Write;
use std::process::Stdio;

use support::{Run, Scratch, muir, text};

/// **The start says what the run is**, one line a thing, on every engine.
#[test]
fn the_start_says_what_the_run_is() {
    let out = muir().args(["--micro", "--main-memory-boards", "4", "--stop-after", "10"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    for line in [
        "engine: micro",
        "memory: 4 boards, 256 KW",
        // The Chaosnet is the machine's and not the engine's, so `micro`
        // has one too: `tests/micro_chaos.rs` boots the band over it.
        // With no --chaos-address these are the defaults, on the private
        // subnet 376 and no band's; a run that boots a band names the
        // band's own pair.
        "chaosnet: 177001, the server at 177002, ",
        "terminal: vnc://127.0.0.1:59",
        "stop: after 10 microcycles",
        "^C holds the machine at the prompt",
    ] {
        assert!(t.contains(line), "{line}:\n{t}");
    }

    let out = muir().args(["--rtl", "--stop-after", "10", "--stop-at", "23731"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    for line in [
        "engine: rtl",
        "memory: 32 boards, 2 MW",
        "chaosnet: 177001, the server at 177002, ",
        "stop: after 10 microcycles, at PC 23731",
    ] {
        assert!(t.contains(line), "{line}:\n{t}");
    }
    assert!(t.contains("pack: "), "the pack, or none:\n{t}");

    let out = muir()
        .args([
            "--chip",
            "--main-memory",
            "model",
            "--io-board",
            "model",
            "--tv",
            "model",
            "--stop-after",
            "10",
        ])
        .run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    for line in [
        "engine: chip",
        "memory: 32 boards, 2 MW, model",
        "boards: I/O board model, TV model simple-tv, disk controller model",
    ] {
        assert!(t.contains(line), "{line}:\n{t}");
    }
    assert!(!t.contains("^C holds"), "no prompt on chip yet:\n{t}");
}

/// **A path under the directory muir was run from is written relative to
/// it**: the pack and the Chaosnet server's file root are looked for
/// there, and their whole paths say nothing a reader does not know.  A
/// path elsewhere is written whole.
#[test]
fn paths_under_the_run_directory_are_written_relative() {
    let here = std::env::current_dir().unwrap();
    // Under `target/`, the one place in the repository a test may write.
    let root =
        Scratch::at(here.join("target").join(format!("muir-file-root-{}", std::process::id())));
    let out = muir().args(["--rtl", "--stop-after", "10", "--chaos-file-root"]).arg(&*root).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    let relative = format!("file root target/muir-file-root-{}", std::process::id());
    assert!(t.contains(&relative), "{relative}:\n{t}");
    assert!(!t.contains(&here.display().to_string()), "and not the whole path:\n{t}");

    // Elsewhere: the whole path, since there is nothing shorter to say.
    let elsewhere = std::env::temp_dir().canonicalize().unwrap();
    let out =
        muir().args(["--rtl", "--stop-after", "10", "--chaos-file-root"]).arg(&elsewhere).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains(&format!("file root {}", elsewhere.display())), "{t}");
}

/// **`info` says it again**, on stdout, as the start did on stderr.
///
/// `--no-auto-boot` rather than a cycle count: the run then starts held at
/// the prompt and waits to be told what to do, so the line is certainly
/// read. Timed against a `--stop-after` instead, this raced --- 20,000
/// microcycles is about a millisecond, and on a loaded machine the run
/// finished before the prompt's reader thread had the line, which is a
/// test failing for want of a scheduler rather than for a reason.
#[test]
fn info_says_it_again() {
    let mut child = muir().args(["--micro", "--no-auto-boot"]).stdin(Stdio::piped()).start();
    let mut stdin = child.stdin();
    writeln!(stdin, "info").unwrap();
    drop(stdin);
    let out = child.wait();
    assert!(out.status.success(), "{}", text(&out));
    let err = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(err.contains("engine: micro"), "at the start:\n{err}");
    assert!(
        stdout.contains("engine: micro") && stdout.contains("memory: 32 boards, 2 MW"),
        "info:\n{stdout}"
    );
}

/// **The start says which boot PROM the machine runs**, and how it stands
/// to MIT's own.
///
/// Recovered copies of the boot PROM are not all the same program ---
/// two builds of "version 9" exist that differ in 214 of their 454 words
/// --- so a run on a file of one's own says whether it is MIT's, and a
/// run on the built-in one says that it is.
#[test]
fn the_start_says_which_prom_the_machine_runs() {
    let out = muir().args(["--micro", "--stop-after", "10"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("prom: built in"), "the built-in PROM says so:\n{t}");

    let out =
        muir().args(["--micro", "--prom", "mit/sys/ubin/promh.mcr", "--stop-after", "10"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(
        t.contains("prom: mit/sys/ubin/promh.mcr, MIT's own word for word"),
        "the named file, and that it is MIT's:\n{t}"
    );
}
