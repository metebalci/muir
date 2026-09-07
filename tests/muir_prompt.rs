// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The prompt under `muir` itself: commands piped in on stdin hold the
//! machine, step it, checkpoint it and end the run.  The boot PROM alone
//! is run, so nothing here needs `vendor/`.

mod support;

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use support::{Child, Run, muir, scratch, text};

/// `hold` holds the machine, `step` moves it so far and holds again,
/// `checkpoint` writes it, a line that is no command is said to be none,
/// and `quit` ends the run before its window does.
#[test]
fn the_prompt_holds_steps_checkpoints_and_quits() {
    let dir = scratch("prompt");
    let chk = dir.join("held.chk");
    let mut child =
        muir().args(["--micro", "--stop-after", "1000000000"]).stdin(Stdio::piped()).start();
    let mut stdin = child.stdin();
    write!(stdin, "pc\nhold\nstep 7\ncheckpoint {}\nbogus\nq\n", chk.display()).unwrap();
    drop(stdin);
    let out = child.wait();
    let t = text(&out);
    assert!(out.status.success(), "muir failed:\n{t}");
    let pcs: Vec<&str> = t.lines().filter(|l| l.starts_with("PC ")).collect();
    assert_eq!(pcs.len(), 3, "pc, hold and step each say where the machine is:\n{t}");
    let after = |l: &str| {
        l.split(';').nth(1).unwrap().trim().split(' ').next().unwrap().parse::<u64>().unwrap()
    };
    assert_eq!(after(pcs[2]), after(pcs[1]) + 7, "step 7 moved it seven microcycles:\n{t}");
    assert!(t.contains("bogus is no command"), "{t}");
    assert!(chk.exists(), "the checkpoint was written");
    assert!(t.contains("quit at PC"), "the run ended by quit:\n{t}");
    assert!(!t.contains("ran out"), "not by its window:\n{t}");
    assert!(!t.contains("muir: "), "no prompt down a pipe, only the answers:\n{t}");
}

/// `reg` writes every register, and a memory command dumps that memory:
/// the boot PROM has run, so the A memory is not all zeros by the time
/// they are asked for.
#[test]
fn registers_and_memories_are_dumped() {
    let mut child =
        muir().args(["--micro", "--stop-after", "1000000000"]).stdin(Stdio::piped()).start();
    let mut stdin = child.stdin();
    write!(stdin, "hold\nreg\namem 0 4\nmmem\nspc 100\nquit\n").unwrap();
    drop(stdin);
    let out = child.wait();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    for register in ["PC ", "OPC ", "VMA ", "MD ", "SPCPTR ", "PDLIDX ", "DISPATCH CONSTANT "] {
        assert!(t.contains(register), "{register} is one of the registers:\n{t}");
    }
    // `amem 0 4`: one line, the four words at 0.
    assert!(t.contains("\n000000  "), "the A memory from 0:\n{t}");
    // `mmem`: the whole of it, thirty-two words, so eight lines ending at 34.
    assert!(t.contains("\n000034  "), "the M memory's last line:\n{t}");
    // `spc 100`: the SPC stack is 40 words, and 100 is past its end.
    assert!(t.contains("spc is 40 words, and 100 is past its end"), "{t}");
}

/// **A count past the end of a memory is the rest of it, however far
/// past.** `1777777777777777777777` is the largest count the prompt can
/// read, and added to the address it would overflow: the dump runs from
/// the address to the memory's last word, and the run goes on to its
/// `quit`.
#[test]
fn a_count_past_the_end_of_a_memory_is_the_rest_of_it() {
    let mut child =
        muir().args(["--micro", "--stop-after", "1000000000"]).stdin(Stdio::piped()).start();
    let mut stdin = child.stdin();
    write!(stdin, "hold\namem 10 1777777777777777777777\nquit\n").unwrap();
    drop(stdin);
    let out = child.wait();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("\n000010  "), "the dump begins at 10:\n{t}");
    // The A memory is 2000 words, four to a line: the last line is at 1774.
    assert!(t.contains("\n001774  "), "and reaches the A memory's last line:\n{t}");
    assert!(t.contains("quit at PC"), "the run went on to its quit:\n{t}");
}

/// **`--no-auto-boot` leaves the boot button unpressed**: the run starts
/// held with the machine halted, `continue` and `step` say that they are
/// not what starts one, and `boot` presses the button and runs it.
#[test]
fn no_auto_boot_waits_for_the_boot_command() {
    let mut child = muir()
        .args(["--micro", "--no-auto-boot", "--stop-after", "1000"])
        .stdin(Stdio::piped())
        .start();
    let mut stdin = child.stdin();
    write!(stdin, "continue\nstep 2\nboot\n").unwrap();
    drop(stdin);
    let out = child.wait();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("start: held, and the boot button not pressed"), "the start says so:\n{t}");
    assert_eq!(
        t.matches("the machine is halted, its RUN clear").count(),
        2,
        "continue and step each said why they did nothing:\n{t}"
    );
    let pcs: Vec<&str> = t.lines().filter(|l| l.starts_with("PC ")).collect();
    assert_eq!(pcs.len(), 1, "only the boot says where the machine is:\n{t}");
    assert!(
        pcs[0].starts_with("PC 0 in the PROM; 0 microcycles"),
        "the button, and nothing has run yet: {}",
        pcs[0]
    );
    assert!(t.contains("ran out at 1000"), "the button ran it to the stop:\n{t}");
}

/// **A hold nothing can run on ends the run**: stdin has ended, so no
/// `continue` will ever come, and muir stops rather than sitting there.
#[test]
fn a_hold_no_one_can_end_ends_the_run() {
    let out = muir().args(["--micro", "--no-auto-boot", "--stop-after", "1000"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("held, and stdin has ended"), "it says why it stopped:\n{t}");
    assert!(t.contains("quit at PC 0 in the PROM after 0"), "nothing ran:\n{t}");
}

/// **`screenshot` writes the screen as a PNG, `startcapture` records the
/// display and `endcapture` closes the recording**, each to the file it is
/// given; a recording still going at the stop is written there instead.
#[test]
fn the_screen_is_written_and_the_display_recorded() {
    let dir = scratch("screen");
    let png = dir.join("screen.png");
    let gif = dir.join("display.gif");
    // Run from the scratch directory: the second `startcapture` names a
    // file without a path, and were it to start a recording after all,
    // the file would land here and not in the directory the test is run
    // from.
    let mut child = muir()
        .args(["--micro", "--stop-after", "1000000000"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .start();
    let mut stdin = child.stdin();
    let second = dir.join("after.gif");
    write!(
        stdin,
        "ss {}\nsc {}\nstartcapture again.gif\nec\nsc {}\nq\n",
        png.display(),
        gif.display(),
        second.display()
    )
    .unwrap();
    drop(stdin);
    let out = child.wait();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    let shot = std::fs::read(&png).unwrap_or_else(|e| panic!("{}: {e}\n{t}", png.display()));
    assert_eq!(&shot[..8], b"\x89PNG\r\n\x1a\n", "a PNG:\n{t}");
    assert!(t.contains(&format!("screenshot: {}", png.display())), "{t}");
    assert!(t.contains("capture: recording the display to"), "{t}");
    assert!(t.contains("one is going already"), "the second startcapture:\n{t}");
    let recorded = std::fs::read(&gif).unwrap_or_else(|e| panic!("{}: {e}\n{t}", gif.display()));
    assert_eq!(&recorded[..6], b"GIF89a", "a GIF, written by endcapture:\n{t}");
    assert!(!dir.join("again.gif").exists(), "the second startcapture started nothing");
    // The one endcapture closed, and the one started after it and written
    // at the stop: two recordings out of one run.
    let after = std::fs::read(&second).unwrap_or_else(|e| panic!("{}: {e}\n{t}", second.display()));
    assert_eq!(&after[..6], b"GIF89a", "a GIF, written at the stop:\n{t}");
    assert_eq!(
        t.matches("frames of the display at").count(),
        2,
        "one recording written by endcapture and one at the stop:\n{t}"
    );
}

/// **At a terminal muir writes `muir: ` while the machine is held**, and
/// not while it runs; the prompt comes back once a `step` has run the
/// microcycles it asked for.  `script` is what gives muir a terminal here;
/// without it, or if what is typed does not reach muir through it, the
/// test says what it skipped.
#[test]
fn a_held_machine_gets_the_prompt() {
    let exe = env!("CARGO_BIN_EXE_muir");
    let args = ["--micro", "--stop-after", "1000000000"];
    let line = format!("{exe} {}", args.join(" "));
    let mut c = Command::new("script");
    if cfg!(target_os = "macos") {
        // BSD script: script [-q] file command ...
        c.arg("-q").arg("/dev/null").arg(exe).args(args);
    } else {
        // util-linux script: script [-q] -e -c "command" file
        c.args(["-q", "-e", "-c", &line, "/dev/null"]);
    }
    // muir under script has script's environment: no flags from the
    // developer's own `~/.muirrc` here either.
    c.env("MUIR_RC", "/dev/null");
    c.stdin(Stdio::piped());
    let Ok(mut child) = Child::spawn(&mut c) else {
        eprintln!("skipped: script(1) is not there to give muir a terminal");
        return;
    };
    let mut stdin = child.stdin();
    // Everything the pty carries, as it comes: what muir writes is waited
    // for rather than slept for, since a loaded machine takes its time.
    let pty = child.stdout();
    let wait_for = |what: fn(&str) -> bool| pty.wait_for(what, Duration::from_secs(30));

    // Setting the pty up can throw away what was typed ahead of it, so
    // nothing is typed until muir has said what the run is.
    let running = wait_for(|t| t.contains("^C holds the machine"));
    let prompt_while_running = pty.so_far().contains("muir: ");
    // `hold` is answered with where the machine is, prompt or no prompt:
    // the answer is what says the line reached muir, and the prompt after
    // it is what is under test.  Then a step, and the prompt comes back.
    let carried = running && writeln!(stdin, "hold").is_ok() && wait_for(|t| t.contains("PC "));
    let held = carried && wait_for(|t| t.contains("muir: "));
    let stepped =
        held && writeln!(stdin, "step 3").is_ok() && wait_for(|t| t.matches("muir: ").count() >= 2);
    writeln!(stdin, "q").ok();
    let out = child.wait();
    let t = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(running, "muir ran under script(1):\n{t}");
    if !carried {
        eprintln!("skipped: script(1) did not carry what was typed through to muir");
        return;
    }
    assert!(held, "muir at a terminal writes the prompt once the machine is held:\n{t}");
    assert!(!prompt_while_running, "no prompt while the machine was running:\n{t}");
    assert!(stepped, "the prompt comes back once the step has run:\n{t}");
    assert!(out.status.success(), "the run ended by quit:\n{t}");
    assert!(
        !t.contains("muir: PC"),
        "the prompt waits for the step\'s microcycles to run, and comes back after \
         what they left:\n{t}"
    );
}

/// `help` lists the commands, and the end of stdin ends nothing: the run
/// goes on to its window.  The window is wide enough that the line is
/// read and acted on well inside it, whatever else the machine is running
/// at the time.
#[test]
fn help_lists_the_commands_and_the_end_of_stdin_ends_nothing() {
    let mut child =
        muir().args(["--micro", "--stop-after", "20000000"]).stdin(Stdio::piped()).start();
    let mut stdin = child.stdin();
    writeln!(stdin, "help").unwrap();
    drop(stdin);
    let out = child.wait();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("hold ") && t.contains("checkpoint [file]"), "help:\n{t}");
    assert!(t.contains("ran out at 20000000"), "the window ended the run:\n{t}");
}

/// `kill -INT`, what ^C sends.
fn interrupt(child: &Child) {
    let status = Command::new("kill").args(["-INT", &child.id().to_string()]).status().unwrap();
    assert!(status.success(), "kill -INT");
}

/// How many times the machine has said where it is: the answer to `pc`,
/// and to a hold.
fn pc_lines(t: &str) -> usize {
    t.lines().filter(|l| l.starts_with("PC ")).count()
}

/// ^C with a prompt to go on from holds the machine there, `continue` runs
/// it on, and ^C while held ends the run as `quit` does, writing the
/// checkpoint the run was to write at its stop.
///
/// muir takes ^C for its own only once the run has begun; before that it
/// ends the process as ^C ends any.  Each ^C here follows an answer from
/// the prompt --- to `pc`, or to the hold before --- since a line is
/// answered only by a run that has begun and has taken ^C for its own.
#[test]
fn control_c_holds_at_the_prompt_and_again_quits() {
    let dir = scratch("interrupt");
    let chk = dir.join("at-c.chk");
    let mut child = muir()
        .args(["--micro", "--stop-after", "1000000000", "--checkpoint"])
        .arg(&chk)
        .stdin(Stdio::piped())
        .start();
    let mut stdin = child.stdin();
    let said = child.stdout();
    writeln!(stdin, "pc").unwrap();
    said.wait_until(|t| pc_lines(t) >= 1, "pc answered, so the run has begun");
    interrupt(&child);
    // The hold says where the machine is: the second PC line.
    said.wait_until(|t| t.contains("held at ^C") && pc_lines(t) >= 2, "held at the first ^C");
    writeln!(stdin, "continue\npc").unwrap();
    said.wait_until(|t| pc_lines(t) >= 3, "continue ran the machine on, and pc answered");
    interrupt(&child);
    said.wait_until(|t| t.matches("held at ^C").count() >= 2, "held at the second ^C");
    interrupt(&child);
    let out = child.wait();
    drop(stdin);
    let t = text(&out);
    assert!(out.status.success(), "the run ended as a quit does:\n{t}");
    assert_eq!(t.matches("held at ^C").count(), 2, "held twice, run on once between:\n{t}");
    assert!(t.contains("quit at PC"), "{t}");
    assert!(chk.exists(), "the checkpoint at the stop was written");
}

/// ^C with no one to type `continue` --- stdin ended --- ends the run at
/// once, as `quit` does.
#[test]
fn control_c_with_no_prompt_quits() {
    let mut child =
        muir().args(["--micro", "--stop-after", "1000000000"]).stdin(Stdio::piped()).start();
    // One line, and the end of stdin right behind it.  The answer says the
    // run has begun and takes ^C for its own, as in the test above; the end
    // of stdin, read by muir's own thread as soon as the line is, is what
    // makes this ^C a quit and not a hold.
    let mut stdin = child.stdin();
    writeln!(stdin, "pc").unwrap();
    drop(stdin);
    child.stdout().wait_until(|t| pc_lines(t) >= 1, "pc answered, so the run has begun");
    interrupt(&child);
    let out = child.wait();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(!t.contains("held"), "{t}");
    assert!(t.contains("quit at PC"), "{t}");
}
