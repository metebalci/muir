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
    // The refusal, not the first line: the version and any file of flags
    // are written before anything is parsed, so they come before it.
    assert!(
        err.lines().find(|l| l.starts_with("muir: ")).is_some_and(|l| l.contains(flag)),
        "{args:?}: the refusal names {flag}:\n{err}"
    );
}

/// A usage error whose message says something in particular, for the
/// refusals that name the engine or the other flag rather than the one
/// that was given.
fn refused_saying(args: &[&str], says: &str) {
    let out = muir().args(args).run();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{args:?}: not a usage error:\n{err}");
    assert!(err.contains(says), "{args:?}: the refusal says {says:?}:\n{err}");
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

/// **`--debug-cable-connect` takes either a debuggee on the network or a
/// debuggee in FPGA fabric, and `0x` is which.** One flag rather than two
/// for one concept --- this machine is the debugger and here is the
/// debuggee --- and the argument is self-describing where it is used: `0x`
/// is unambiguous against a port, a host name and a host with a port.
///
/// The window is `rtl`'s alone, as the endpoint already is: `micro` has no
/// timing model and on `chip` the debug cable is the board's own DBGIN.
/// It is `--debug-cable-connect`'s alone too, since what is to be built in
/// fabric is the debuggee's DBGIN end, so there is no listening at a
/// window. It must be a multiple of four, the window being 32-bit
/// registers, and it has no default: where the window sits is a property
/// of the bitstream.
///
/// **The last case is the platform.** The mapping is `/dev/mem`, which
/// only Linux has; muir is developed on macOS, where the flag is refused
/// by name rather than failing to build. Either way the run stops and the
/// first line names the flag.
#[test]
fn a_window_address_is_rtls_and_the_debuggers_and_a_multiple_of_four() {
    for engine in ["--micro", "--chip"] {
        refused_saying(
            &[engine, "--debug-cable-connect", "0x80000080", "--stop-after", "1"],
            if engine == "--micro" { "no timing model" } else { "the board's DBGIN only" },
        );
    }
    refused(
        &["--rtl", "--debug-cable-listen", "0x80000080", "--stop-after", "1"],
        "--debug-cable-listen",
    );
    refused_saying(
        &["--rtl", "--debug-cable-listen", "--debug-cable-connect", "0x80000080"],
        "one lashup at a time",
    );
    for bad in ["0x80000082", "0x80000081", "0xnothex", "0x"] {
        refused(
            &["--rtl", "--debug-cable-connect", bad, "--stop-after", "1"],
            "--debug-cable-connect",
        );
    }
    // An argument that is not 0x is still an endpoint, and a bad one is
    // refused as an endpoint.
    refused(
        &["--rtl", "--debug-cable-connect", "300000", "--stop-after", "1"],
        "--debug-cable-connect",
    );
    // And a window this build cannot map at all.
    let out =
        muir().args(["--rtl", "--debug-cable-connect", "0x80000080", "--stop-after", "1"]).run();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "a window muir cannot reach is no run:\n{err}");
    assert!(
        err.lines()
            .find(|l| l.starts_with("muir: "))
            .is_some_and(|l| l.contains("--debug-cable-connect")),
        "the refusal names the flag:\n{err}"
    );
    if !cfg!(target_os = "linux") {
        assert_eq!(out.status.code(), Some(2), "refused by name, not attempted:\n{err}");
        assert!(err.contains("/dev/mem, which only Linux has"), "and says why:\n{err}");
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

/// **The serial port is one machine's, and the lashup runs two.** The
/// endpoint would be the debugger's alone, and nothing in the run loops
/// that step two machines through the debug cable reaches the other
/// machine's port, so the flag is refused there rather than opened for one
/// of them without saying which.
#[test]
fn the_serial_port_is_not_the_lashups() {
    for lashup in ["--debug-in-process", "--debug-cable-listen"] {
        refused(&["--rtl", lashup, "--serial", "0", "--stop-after", "1"], "--serial");
    }
}

/// **`--watch` is refused for the shape of its argument**, and the first
/// line says which flag and what was wrong: no colon between the range
/// and the nets, a range that is no number or ends before it begins, a
/// bus of no bits, and no nets at all.
#[test]
fn a_watch_that_is_not_a_range_and_nets_is_refused() {
    for spec in ["PC/14", "x-3:PC/14", "9-3:PC/14", "3:", "3:PC/0", "3:PC/14,", "-:PC/14"] {
        refused(&["--chip", "--watch", spec, "--stop-after", "1"], "--watch");
    }
    refused(&["--chip", "--stop-after", "1", "--watch"], "--watch");
}

/// **A net `--watch` names has to be on a board this run has**, and the
/// refusal says which boards those are, as the prompt's `net` does: a net
/// on no board, a `disk:` net with the model controller in the machine ---
/// which has no board and so no nets --- and a bus missing its top bit.
/// This is not the command line's shape, so it is the one-line refusal
/// and exit status 1, not the usage.
#[test]
fn a_watch_on_a_net_the_machine_has_not_got_is_refused() {
    for (spec, says) in [
        ("0-1:NOSUCH", "no net NOSUCH on cpu, busint, memory, tv, io"),
        ("0-1:disk:NEW CCW", "no board called disk here; this run has cpu, busint, memory, tv, io"),
        ("0-1:cpu:PC/15", "cpu has no PC14 --- a bus is PC0 up"),
    ] {
        let out = muir().args(["--chip", "--watch", spec, "--stop-after", "1"]).run();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.code(),
            Some(1),
            "{spec}: not refused as a run that cannot start:\n{err}"
        );
        assert!(err.contains(says), "{spec}: the refusal says {says:?}:\n{err}");
        assert!(!err.contains("usage:"), "{spec}: and the usage is no help here:\n{err}");
    }
}

/// **On the other two engines `--watch` is ignored, and the start says
/// so**, as it says of every flag that is chip's alone: they have
/// registers and memories and no nets to record.
#[test]
fn a_watch_on_the_other_engines_is_said_to_be_ignored() {
    for engine in ["--micro", "--rtl"] {
        let out = muir().args([engine, "--watch", "0-1:PC/14", "--stop-after", "1"]).run();
        let t = text(&out);
        assert!(out.status.success(), "{engine}: not refused:\n{t}");
        assert!(t.contains("warning: --watch is chip, and this run is"), "{engine}: said:\n{t}");
        assert!(!t.contains("watch: "), "{engine}: and nothing was recorded:\n{t}");
    }
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
/// **The version comes first, then the file of flags, then everything
/// else --- and a run that is refused has said both before it says why.**
///
/// A report of a run says which muir made it, and a report of a *refused*
/// run wants the same two facts most of all: which muir, and which file
/// of flags it read, since a file it was not asked about is the commonest
/// reason a run will not start.  So both are written before anything is
/// parsed, and an error follows them rather than standing alone.
#[test]
fn the_version_and_the_file_are_said_before_anything_is_parsed() {
    let (_dir, rc) = muirrc("flags", "--chaos-address 3050,3060\n");
    let out = muir().env("MUIR_RC", &rc).args(["--stop-after", "1"]).run();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(2), "the run is refused:\n{t}");
    let lines: Vec<&str> = t.lines().collect();
    assert!(lines[0].starts_with("muir 0.1.0"), "the version is the first line:\n{t}");
    assert!(lines[1].contains(&rc.display().to_string()), "the file of flags is the second:\n{t}");
    let why = lines.iter().position(|l| l.starts_with("muir: ")).expect("a refusal");
    assert!(why > 1, "and the refusal comes after both:\n{t}");

    // A run that starts says them once and not twice.
    let out = muir().args(["--micro", "--stop-after", "1"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert_eq!(t.matches("muir 0.1.0").count(), 1, "the version once:\n{t}");
    assert!(!t.contains("flags:"), "and no file, this run having none:\n{t}");
}

/// **`--version` and `--help` are answered before the file of flags is
/// read at all**, because neither runs a machine and so nothing a file
/// configures applies to either.
///
/// A file outlives the flags it holds. `--chaos-address 3050,3060` was
/// the spelling until the Chaosnet server left muir, and a file still
/// holding it is refused --- rightly, for a run. But `muir --version`
/// asks what this build is, and a build that cannot say so because of a
/// file it was not asked to use leaves a person with no way to report
/// which muir they have.
#[test]
fn the_version_and_the_help_are_answered_whatever_the_file_holds() {
    let (_dir, rc) = muirrc("flags", "--chaos-address 3050,3060\n");
    // The file really is refused for a run, or this test proves nothing.
    let out = muir().env("MUIR_RC", &rc).args(["--stop-after", "1"]).run();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(2), "the file is refused for a run:\n{t}");
    assert!(t.contains("--chaos-address"), "by name:\n{t}");

    for flag in ["--version", "-V"] {
        let out = muir().env("MUIR_RC", &rc).arg(flag).run();
        let t = text(&out);
        assert!(out.status.success(), "{flag} is answered all the same:\n{t}");
        assert!(t.starts_with("muir "), "{flag} says what this build is:\n{t}");
        assert!(!t.contains("usage:"), "{flag} is no run:\n{t}");
    }
    for flag in ["--help", "-h"] {
        let out = muir().env("MUIR_RC", &rc).arg(flag).run();
        let t = text(&out);
        assert!(out.status.success(), "{flag} is answered all the same:\n{t}");
        assert!(t.contains("usage: muir"), "{flag} prints the usage:\n{t}");
    }
}

/// apart, and whatever a flag takes is the rest of the line, spaces and
/// all. The start says which flags came from the file, and where from.
#[test]
fn the_flags_in_the_file_are_the_runs() {
    // A directory with a space in its name, for the rest of the line.
    let dir = scratch("rc root");
    let gif = dir.join("a recording.gif");
    let (_rc_dir, rc) = muirrc(
        "flags",
        &format!(
            "# what every run of mine wants\n\n--rtl\n--stop-after 10\n--tv-capture {}\n",
            gif.display()
        ),
    );
    let out = muir().env("MUIR_RC", &rc).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("engine: rtl"), "the engine came from the file:\n{t}");
    assert!(t.contains("stop: after 10 microcycles"), "and the stop:\n{t}");
    assert!(
        t.contains(&format!("capture: {}", gif.display())),
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

/// **A drive the netlist controller has no port for is refused until the
/// multiplexor is asked for.**
///
/// Not because the unit number is forced to 0. `UNIT<2:0>` reach one
/// 74LS244's inputs on the controller and nothing else --- `cadrdc/dc.wlr`
/// gives a direction per pin and none of the three has a `TO` --- so the
/// board has no driver for them at all, and the six one-board jumpers
/// ground them to stop three inputs floating. Unit 0 is the consequence.
/// The DISK MULTIPLEXOR is what supplies the driver, so a second drive or
/// a drive past unit 0 wants one.
///
/// **muir does not fit it by implication**, though it could and once did:
/// a board that appears because of the way a pack was spelled is a board
/// the machine has without anybody choosing it, and which machine is
/// being simulated is the user's to say. So the run stops and names the
/// flag.
///
/// The model controller is refused nothing, because it wants no board: it
/// is behavioural and has addressed eight units all along,
/// `disk_controller::UNITS`.
#[test]
fn a_drive_past_unit_0_wants_the_multiplexor_named() {
    const FITTED: &str = "with a multiplexor";
    let start =
        |args: &[&str]| String::from_utf8_lossy(&muir().args(args).run().stderr).into_owned();
    let netlist =
        ["--chip", "--main-memory", "netlist", "--disk-controller", "netlist", "--stop-after", "1"];
    let with = |extra: &[&'static str]| -> Vec<&'static str> {
        let mut args = netlist.to_vec();
        args.extend_from_slice(extra);
        args
    };
    // The image is never opened: the start says what the machine is
    // before it is built, and a missing pack stops the run after that.
    let one = start(&with(&["--disk-pack", "nothing.img"]));
    assert!(!one.contains(FITTED), "one drive in unit 0 wants no board:\n{one}");
    // A drive the one port cannot reach, refused by name.
    for extra in [
        &["--disk-pack", "nothing.img,3"][..],
        &["--disk-pack", "a.img", "--disk-pack", "b.img,1"][..],
    ] {
        let said = start(&with(extra));
        assert!(said.contains("--disk-multiplexor"), "{extra:?} names the flag:\n{said}");
        assert!(said.contains("usage:"), "{extra:?} is refused:\n{said}");
    }
    // And with the flag, fitted and said to be.
    for extra in [
        &["--disk-multiplexor", "--disk-pack", "nothing.img"][..],
        &["--disk-multiplexor", "--disk-pack", "nothing.img,3"][..],
        &["--disk-multiplexor", "--disk-pack", "a.img", "--disk-pack", "b.img,1"][..],
    ] {
        let said = start(&with(extra));
        assert!(said.contains(FITTED), "{extra:?} fits the board, and says so:\n{said}");
        assert!(!said.contains("usage:"), "{extra:?} is not refused:\n{said}");
    }
    // The model controller takes the units and fits nothing --- asked on
    // `chip`, where the start says which boards are on the buses and so
    // can be caught saying it fitted one.
    for engine in [&["--micro"][..], &["--chip"][..]] {
        let mut args = engine.to_vec();
        args.extend(["--disk-pack", "a.img", "--disk-pack", "b.img,1", "--stop-after", "1"]);
        let said = start(&args);
        assert!(!said.contains(FITTED), "{engine:?}: the model controller wants no board:\n{said}");
        assert!(!said.contains("usage:"), "{engine:?}: and is not refused:\n{said}");
    }
}

/// **The multiplexor is a board, so it needs a board to hang off**, and
/// one pack a drive.
#[test]
fn the_multiplexor_is_the_netlist_controllers_board() {
    refused(&["--chip", "--disk-multiplexor", "--stop-after", "1"], "--disk-multiplexor");
    refused(&["--micro", "--disk-multiplexor", "--stop-after", "1"], "--disk-multiplexor");
    refused(
        &["--micro", "--disk-pack", "a.img,1", "--disk-pack", "b.img,1", "--stop-after", "1"],
        "--disk-pack",
    );
}

/// **`kill -USR1` asks a running machine where it is, and it answers and
/// goes on.**
///
/// A long `chip` run has no prompt --- the process has a terminal and
/// nothing else --- so before this the only way to know where one was was
/// to infer it from what it had touched. Issue 86 has a run whose state
/// was read from pack mtimes, then from lit pixels, then from a
/// block-by-block comparison of the pack, two of the three retracted, over
/// six hours, and `pc` unanswered the whole time.
///
/// **The answer arrives while the machine runs**, which is the point: a
/// reader that held the run would be no use for the timing runs this
/// exists for. Measured over the same 20,000-microcycle window, signalled
/// three times against not at all: both end at PC 240 having run 20,000,
/// in 8.926 s against 8.903, which is the cost of the three lines printed.
/// So asking neither changes the answer nor slows the run.
///
/// The signal is taken by every engine and acted on by `chip` alone,
/// because the default action for `SIGUSR1` is to kill the process: one
/// sent to the wrong run of a pair would otherwise end a run that had been
/// going for hours.
#[test]
fn a_running_chip_says_where_it_is_when_asked() {
    // Long enough to still be running when the signal lands, short enough
    // that the test is a second or two: `chip` does about 2,200
    // microcycles a second.
    // **Signalled once the run says it is listening, not after a guess at
    // how long that takes.**  The default action for `SIGUSR1` is to kill
    // the process, so a signal sent before the handler is installed kills
    // the run --- and building a `chip` machine takes as long as it takes,
    // which on a loaded machine is longer than any sleep worth writing.
    // The run's own start line says when it is armed.
    let child =
        muir().args(["--chip", "--main-memory-boards", "4", "--stop-after", "6000"]).start();
    child.stderr().wait_until(|t| t.contains("where: kill -USR1"), "muir says it is listening");
    let pid = child.id().to_string();
    let killed = std::process::Command::new("kill")
        .args(["-USR1", &pid])
        .status()
        .expect("kill did not run");
    assert!(killed.success(), "the signal was delivered");
    let out = child.wait();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let said = text
        .lines()
        .find(|l| l.starts_with("PC ") && l.contains("microcycles this run"))
        .unwrap_or_else(|| panic!("no answer to the signal in:\n{text}"));
    // It answered from inside the run rather than at the end of it.
    let ran: u64 = said
        .rsplit_once("; ")
        .and_then(|(_, tail)| tail.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("no microcycle count in `{said}`"));
    assert!(0 < ran && ran < 6_000, "answered mid-run, at {ran} of 6000: `{said}`");
    assert!(text.contains("IR "), "and said what it was executing:\n{text}");
    assert!(text.contains("ran out at 6000"), "and ran to its stop after answering:\n{text}");
}

/// **The CHUDP flags describe a link that has to be there.** With neither
/// `--chaos-address` nor `--chaos-udp` nothing is listening, so a peer and
/// a request to learn where one is are each a statement about a link that
/// does not exist.
#[test]
fn the_chudp_flags_need_the_link() {
    refused(&["--chaos-udp-peer", "3040@127.0.0.1:42043", "--stop-after", "1"], "--chaos-udp-peer");
    refused(&["--chaos-udp-dynamic", "--stop-after", "1"], "--chaos-udp-dynamic");
}

/// **A peer is one endpoint, and not this machine's own address.** Two
/// endpoints for one address are two answers to where one host lives; the
/// address this machine answers at is not a host over the network. Only
/// this machine's: nothing else is on the cable, the file and time host
/// being a peer like any other.
#[test]
fn a_peer_is_one_endpoint_and_not_this_machines_own() {
    let link = ["--chaos-udp", "127.0.0.1:0"];
    let twice = [
        link.as_slice(),
        &["--chaos-udp-peer", "3040@127.0.0.1:42043"],
        &["--chaos-udp-peer", "3040@127.0.0.1:42044"],
        &["--stop-after", "1"],
    ]
    .concat();
    refused(&twice, "--chaos-udp-peer");
    // The cable's own with no --chaos-address: `chaos::Config`'s default,
    // 177001, which is no band's.
    let own =
        [link.as_slice(), &["--chaos-udp-peer", "177001@127.0.0.1:42043"], &["--stop-after", "1"]]
            .concat();
    refused(&own, "--chaos-udp-peer");
    // 177002 was the old server's address, and is now a peer like any
    // other: nothing on this cable answers there.
    let out = muir()
        .args(link)
        .args(["--chaos-udp-peer", "177002@127.0.0.1:42043", "--micro", "--stop-after", "1"])
        .run();
    assert!(out.status.success(), "{}", text(&out));
    // And this machine's own address is a peer when this machine is
    // somewhere else.
    let out = muir()
        .args(link)
        .args(["--chaos-address", "4401"])
        .args(["--chaos-udp-peer", "3050@127.0.0.1:42043", "--micro", "--stop-after", "1"])
        .run();
    assert!(out.status.success(), "{}", text(&out));
}

/// **A peer's spelling is `<address>@<host>[:<port>]`**, the address in
/// octal or `subnet:host`, and the port may be left off for the
/// protocol's own. A name with no address is refused at the start rather
/// than becoming a peer that is never reached.
#[test]
fn a_peer_is_an_address_and_where_it_lives() {
    let link = ["--chaos-udp", "127.0.0.1:0"];
    for bad in [
        // no address
        "127.0.0.1:42043",
        // nowhere to live
        "3040",
        // 9 is not an octal digit
        "99@127.0.0.1:42043",
        // a name with no address
        "3040@no-such-host.invalid",
        // nor is that a port
        "3040@127.0.0.1:not-a-port",
    ] {
        let args = [link.as_slice(), &["--chaos-udp-peer", bad], &["--stop-after", "1"]].concat();
        refused(&args, "--chaos-udp-peer");
    }
    // The port may be left off, and the address may be subnet:host.
    let out = muir()
        .args(link)
        .args(["--chaos-udp-peer", "6:40@127.0.0.1"])
        .args(["--micro", "--stop-after", "1"])
        .run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("3040 at 127.0.0.1:42042"), "the protocol's own port: {t}");
}

/// **The start says the link is there and who is on it**, so that a run
/// that reaches nobody says so rather than looking as though it had.
#[test]
fn the_start_says_what_the_link_is() {
    let out = muir()
        .args(["--micro", "--stop-after", "1", "--chaos-udp", "127.0.0.1:0"])
        .args(["--chaos-udp-peer", "3060@127.0.0.1:42043", "--chaos-udp-dynamic"])
        .run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    let line = t.lines().find(|l| l.starts_with("chaosnet udp:")).unwrap_or_else(|| panic!("{t}"));
    assert!(line.contains("listening at 127.0.0.1:"), "where it listens: {line}");
    assert!(line.contains("3060 at 127.0.0.1:42043"), "and who is on it: {line}");
    assert!(line.contains("learning where others are"), "and that it learns: {line}");
    // A link with no peer named is a run that reaches no file or time
    // host, and the line says so rather than leaving it to be found out
    // at the cold-load debugger.
    let out = muir().args(["--micro", "--stop-after", "1", "--chaos-udp", "127.0.0.1:0"]).run();
    let t = text(&out);
    let line = t.lines().find(|l| l.starts_with("chaosnet udp:")).unwrap_or_else(|| panic!("{t}"));
    assert!(line.contains("no peer named, so no file or time host"), "{line}");
    // And with no link there is no line at all.
    let out = muir().args(["--micro", "--stop-after", "1"]).run();
    assert!(!text(&out).contains("chaosnet udp:"), "{}", text(&out));
}

/// **The flags of the Chaosnet server muir used to carry are refused, and
/// the refusal says where the host went.** muir has no file or time server
/// in it any more --- a CADR had none --- so `--chaos-file-root`,
/// `--chaos-file-peers` and `--debuggee-chaos-file-root` configure
/// nothing. A run whose `.muirrc` still names one would otherwise boot
/// quietly to the cold-load debugger asking for the date, so it is stopped
/// with the answer in the first line: the host is another program on the
/// network, `ozd`, named with `--chaos-udp-peer`.
#[test]
fn the_file_server_flags_are_gone_and_say_where_the_host_went() {
    for flag in ["--chaos-file-root", "--chaos-file-peers", "--debuggee-chaos-file-root"] {
        let out = muir().args([flag, ".", "--stop-after", "1"]).run();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{flag}: not a usage error:\n{err}");
        let first = err.lines().find(|l| l.starts_with("muir: ")).unwrap_or("");
        assert!(first.contains(flag), "{flag}: the refusal names it:\n{err}");
        assert!(first.contains("--chaos-udp-peer"), "{flag}: and what replaced it:\n{err}");
        assert!(first.contains("ozd"), "{flag}: and which host that is:\n{err}");
    }
}

/// **`--chaos-address` takes one address.** The second half was the
/// Chaosnet server's, and there is no Chaosnet server in muir to give an
/// address to; a run that still writes the old pair is refused rather than
/// left to find out at the cold-load debugger.
#[test]
fn the_chaos_address_is_one_address() {
    for arg in ["3050,3060", "4401,4403", "3050,"] {
        refused(&["--chaos-address", arg, "--stop-after", "1"], "--chaos-address");
    }
    // And what is refused says where the host is named instead.
    let out = muir().args(["--chaos-address", "3050,3060", "--stop-after", "1"]).run();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.lines()
            .find(|l| l.starts_with("muir: "))
            .is_some_and(|l| l.contains("--chaos-udp-peer")),
        "{err}"
    );
    // One address is taken, in octal or subnet:host.
    for arg in ["3050", "6:50"] {
        let out = muir()
            .args(["--micro", "--stop-after", "1", "--chaos-address", arg])
            .args(["--chaos-udp", "127.0.0.1:0"])
            .run();
        let t = text(&out);
        assert!(out.status.success(), "{arg}:\n{t}");
        assert!(t.contains("chaosnet: 3050,"), "{arg}: the same number either way:\n{t}");
    }
}

/// **`--chaos-address` starts Chaosnet over UDP, and a run without it
/// opens no socket.**
///
/// An address is a run saying which machine on which network this is, and
/// a network it cannot reach is no network: the file and time host a band
/// calls is another program, so the link is what the address is for. It
/// goes on the protocol's own port, 42042, unless `--chaos-udp` says
/// where.
///
/// And a plain `muir` binds nothing. That is what lets many runs go at
/// once --- this file alone starts dozens --- and a port bound by every
/// run of a simulator that mostly does not want one would be a port nobody
/// asked for.
///
/// **The default-port half skips when something else holds 42042**, and
/// says so. It is the protocol's own port, so a Chaosnet daemon on the
/// same machine --- `ozd`, a `cbridge` --- has it, and a suite that failed
/// for that would be failing about the machine it runs on rather than
/// about muir.
#[test]
fn an_address_starts_the_link_and_nothing_else_does() {
    let out = muir().args(["--micro", "--stop-after", "1", "--chaos-address", "3050"]).run();
    let t = text(&out);
    if !out.status.success() && t.contains("--chaos-udp 127.0.0.1:42042") {
        eprintln!("skipped: something on this machine already holds port 42042");
    } else {
        assert!(out.status.success(), "{t}");
        assert!(
            t.contains("chaosnet udp: listening at 127.0.0.1:42042"),
            "the protocol's own port:\n{t}"
        );
    }
    // --chaos-udp says where instead, and is still usable on its own.
    let out = muir()
        .args(["--micro", "--stop-after", "1", "--chaos-address", "3050"])
        .args(["--chaos-udp", "127.0.0.1:0"])
        .run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("chaosnet udp: listening at 127.0.0.1:"), "{t}");
    assert!(!t.contains(":42042"), "the flag had the last word:\n{t}");
    // With no address there is no socket at all.
    let out = muir().args(["--micro", "--stop-after", "1"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(!t.contains("chaosnet udp:"), "no link:\n{t}");
    assert!(t.contains("--chaos-address puts it on a network"), "and the line says so:\n{t}");
}
