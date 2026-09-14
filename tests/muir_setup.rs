// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What `muir` says a run is when it starts: the engine, the memory, the
//! boards, the pack, the Chaosnet, the terminal, the stops, the prompt;
//! and the prompt's `info`, which says it again.

mod support;

use std::io::Write;
use std::process::Stdio;

use support::{Run, Scratch, listening, muir, text};

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
        // Two lines, because they are two things on the board: the
        // switches, 177001 with no --chaos-address, on the private subnet
        // 376 and no band's; and the cable, which is --chaos-udp and is
        // not plugged in here.
        "chaosnet: 177001",
        "chaosnet over udp: disabled",
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
        "chaosnet: 177001",
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
    assert!(t.contains("^C holds the machine at the prompt"), "chip has the prompt too:\n{t}");
}

/// **The start says whether the run has a prompt, on every shape of run.**
/// One machine at either end of the debug cable over TCP has it, as one
/// alone does --- the netlist machine with its connector listening too,
/// which had none while it waited for the debugger and now runs from the
/// start; the lashup in one process has none and says so, with why.  Issue
/// 102 met a cable run whose start said neither, and which had none.
#[test]
fn the_start_says_whether_the_run_has_a_prompt() {
    const HAS: &str = "^C holds the machine at the prompt; help lists muir's commands";
    let mut debuggee = muir()
        .args(["--rtl", "--debug-cable-listen", "127.0.0.1:0", "--stop-after", "1000000000"])
        .stdin(Stdio::piped())
        .start();
    let addr = listening(&debuggee);
    let debugger =
        muir().args(["--rtl", "--debug-cable-connect", &addr, "--stop-after", "3000"]).start();
    let a = debugger.wait();
    let ta = text(&a);
    assert!(a.status.success(), "{ta}");
    assert!(ta.contains(HAS), "the debugger over TCP:\n{ta}");
    debuggee.stderr().wait_until(|t| t.contains(HAS), "and the debuggee");
    let mut stdin = debuggee.stdin();
    writeln!(stdin, "quit").unwrap();
    let b = debuggee.wait();
    drop(stdin);
    let tb = text(&b);
    assert!(b.status.success(), "{tb}");

    let out = muir().args(["--rtl", "--debug-in-process", "--stop-after", "10"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(
        t.contains("prompt: none; the lashup in one process runs two machines"),
        "the lashup in one process says it has none, and why:\n{t}"
    );

    // The netlist machine with its connector listening has the prompt as
    // the netlist machine alone does, and runs to its stop with nobody on
    // the cable.
    let out = muir()
        .args(["--chip", "--main-memory-boards", "1", "--debug-cable-listen", "127.0.0.1:0"])
        .args(["--stop-after", "1"])
        .run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains(HAS), "the netlist machine, its connector listening:\n{t}");
}

/// **A path under the directory muir was run from is written relative to
/// it**: the pack and the file of flags are looked for there, and their
/// whole paths say nothing a reader does not know.  A path elsewhere is
/// written whole.
#[test]
fn paths_under_the_run_directory_are_written_relative() {
    let here = std::env::current_dir().unwrap();
    // Under `target/`, the one place in the repository a test may write.
    let dir = Scratch::at(here.join("target").join(format!("muir-rc-{}", std::process::id())));
    let rc = dir.join(".muirrc");
    std::fs::write(&rc, "--stop-after 10\n").unwrap();
    let out = muir().env("MUIR_RC", &rc).args(["--rtl"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    let relative = format!("from target/muir-rc-{}/.muirrc", std::process::id());
    assert!(t.contains(&relative), "{relative}:\n{t}");
    assert!(!t.contains(&here.display().to_string()), "and not the whole path:\n{t}");

    // Elsewhere: the whole path, since there is nothing shorter to say.
    let elsewhere = Scratch::at(
        std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("muir-rc-{}", std::process::id())),
    );
    let rc = elsewhere.join(".muirrc");
    std::fs::write(&rc, "--stop-after 10\n").unwrap();
    let out = muir().env("MUIR_RC", &rc).args(["--rtl"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains(&format!("from {}", rc.display())), "{t}");
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
