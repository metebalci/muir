// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The prompt under `muir` itself: commands piped in on stdin hold the
//! machine, step it, checkpoint it and end the run.  The boot PROM alone
//! is run, so nothing here needs `vendor/`.

mod support;

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use support::{Child, Run, listening, muir, scratch, text};

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
    assert_eq!(after(pcs[2]), after(pcs[1]) + 7, "step 7 moved it seven microcycles:\n{t}");
    assert!(t.contains("bogus is no command"), "{t}");
    assert!(chk.exists(), "the checkpoint was written");
    assert!(t.contains("quit at PC"), "the run ended by quit:\n{t}");
    assert!(!t.contains("ran out"), "not by its window:\n{t}");
    assert!(!t.contains("muir: "), "no prompt down a pipe, only the answers:\n{t}");
}

/// The machine's microcycles off a line that says where it is: the second
/// field of `PC 0 in the PROM; 240 microcycles, 34800 ns; 240 this run`.
fn after(pc_line: &str) -> u64 {
    pc_line.split(';').nth(1).unwrap().trim().split(' ').next().unwrap().parse().unwrap()
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
    child.interrupt();
    // The hold says where the machine is: the second PC line.
    said.wait_until(|t| t.contains("held at ^C") && pc_lines(t) >= 2, "held at the first ^C");
    writeln!(stdin, "continue\npc").unwrap();
    said.wait_until(|t| pc_lines(t) >= 3, "continue ran the machine on, and pc answered");
    child.interrupt();
    said.wait_until(|t| t.matches("held at ^C").count() >= 2, "held at the second ^C");
    child.interrupt();
    let out = child.wait();
    drop(stdin);
    let t = text(&out);
    assert!(out.status.success(), "the run ended as a quit does:\n{t}");
    assert_eq!(t.matches("held at ^C").count(), 2, "held twice, run on once between:\n{t}");
    assert!(t.contains("quit at PC"), "{t}");
    assert!(chk.exists(), "the checkpoint at the stop was written");
}

/// **`mem` reads main memory at a physical address, and the machine is not
/// held to answer.**
///
/// Every line here is typed while the machine runs, as `net` is, and each
/// is acted on between two microcycles: no `hold` is sent and no PC line
/// comes back, which is what a hold would print.  The two refusals are the
/// interface's substance --- the address is physical, so one past the 22
/// bits the Xbus carries is not one at all, and one inside those 22 bits
/// with no board behind it is told how much memory this machine has.
#[test]
fn mem_reads_a_physical_address_while_the_machine_runs() {
    let mut child =
        muir().args(["--micro", "--stop-after", "1000000000"]).stdin(Stdio::piped()).start();
    let mut stdin = child.stdin();
    let said = child.stdout();
    let line = |head: &'static str| move |t: &str| t.lines().any(|l| l.starts_with(head));
    // The 512th CCW of a cold-load command list, which is the word issue
    // 88 wanted: one word, because a count left out is one.
    writeln!(stdin, "mem 40777").unwrap();
    said.wait_until(line("040777  "), "the word at 40777, with nothing held");
    // Sixteen words from 40000: four lines, the middle two the same as the
    // first and so a `*`, and the last written whole.
    writeln!(stdin, "mem 40000 20").unwrap();
    said.wait_until(line("040014  "), "and sixteen words from 40000");
    writeln!(stdin, "mem 20000000").unwrap();
    said.wait_until(|t| t.contains("22 bits"), "an address past the Xbus's 22 bits is refused");
    writeln!(stdin, "mem 10000000").unwrap();
    said.wait_until(|t| t.contains("past its end"), "and one past the machine's last board");
    writeln!(stdin, "quit").unwrap();
    let out = child.wait();
    drop(stdin);
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert_eq!(pc_lines(&t), 0, "the machine was never held to answer:\n{t}");
    assert!(t.contains("quit at PC"), "the run ended by quit:\n{t}");
}

/// **The same on `chip`, held**, which is what the command was built for:
/// a machine parked in a state that took hours, asked what one word of
/// main memory holds.
///
/// Main memory here is not an array but the memory boards on the
/// backplane, and the word is a bit off each of the 32 4116s of one bank
/// of one board: [`muir::cable::FarEnd::main_word`], which
/// `chip_and_rtl_read_the_same_main_memory` holds to `rtl`'s array.  One
/// board rather than the usual thirty-two, so that the run is a moment;
/// the address is on it either way.
#[test]
fn chip_reads_a_word_of_a_memory_board_at_the_prompt() {
    let mut child = muir()
        .args(["--chip", "--main-memory-boards", "1", "--stop-after", "100"])
        .stdin(Stdio::piped())
        .start();
    let mut stdin = child.stdin();
    write!(stdin, "hold\nmem 40777\nmem 200000\nquit\n").unwrap();
    drop(stdin);
    let out = child.wait();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(
        t.lines().any(|l| l.starts_with("040777  ")),
        "the word at 40777, off the board's cells:\n{t}"
    );
    assert!(
        t.contains("main memory here is 200000 words, 0 to 177777"),
        "and past the one board there is nothing to read:\n{t}"
    );
    assert!(t.contains("quit at PC"), "{t}");
}

/// ^C with no one to type `continue` --- stdin ended --- ends the run on
/// that one interrupt, as `quit` does.
///
/// **Which way it gets there is a race, and not one muir can settle.** The
/// reader thread hands over the `pc` line and sees the end of stdin just
/// after it, and the ^C can land between the two. Seen first, the ^C is a
/// quit outright; not seen, the ^C holds and the ended stdin ends the run
/// at the very next check. So what is held to here is the guarantee ---
/// one interrupt, and the run is over --- and, if it did go through a
/// hold, that the hold was resolved rather than left standing. Asserting
/// the path instead is what made this fail in CI on a loaded runner.
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
    child.interrupt();
    let out = child.wait();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("quit at PC"), "the one ^C ended the run:\n{t}");
    if t.contains("held at ^C") {
        assert!(
            t.contains("held, and stdin has ended"),
            "the ^C landed before the end of stdin was seen, so it held --- and then\n\
             the ended stdin had to end the run, rather than leave the hold standing:\n{t}"
        );
    }
}

/// The cable over TCP with the prompt at each end: the debuggee listening
/// where the host says, the debugger connected to it, stdin a pipe on
/// both and a window long enough to be typed at.
fn pair() -> (Child, Child) {
    let debuggee = muir()
        .args(["--rtl", "--debug-cable-listen", "127.0.0.1:0", "--stop-after", "1000000000"])
        .stdin(Stdio::piped())
        .start();
    let addr = listening(&debuggee);
    let debugger = muir()
        .args(["--rtl", "--debug-cable-connect", &addr, "--stop-after", "1000000000"])
        .stdin(Stdio::piped())
        .start();
    (debuggee, debugger)
}

/// **Either end of the cable over TCP has the prompt**, with the commands
/// a machine alone has: `pc`, `hold`, `step`, `info` and `quit` answer on
/// the debugger, `pc` and `quit` on the debuggee, and the two agree to
/// stop as their windows would have made them.
///
/// **What an end of the cable cannot write is refused with the reason,
/// not silently absent.** `checkpoint` and the capture commands are the
/// two: a checkpoint of a machine with the cable in its bus interface is
/// none `--resume` can take, and a recording over the cable is two clocks,
/// which is why `--checkpoint` and `--tv-capture` are refused there from
/// the command line.  Issue 102 met a `--debug-cable-connect` run that
/// answered nothing at all and could only be signaled.
#[test]
fn both_ends_of_the_cable_over_tcp_have_the_prompt() {
    let dir = scratch("cable-prompt");
    let chk = dir.join("cabled.chk");
    let (mut debuggee, mut debugger) = pair();
    let (mut to_debugger, mut to_debuggee) = (debugger.stdin(), debuggee.stdin());
    let (a, b) = (debugger.stdout(), debuggee.stdout());
    writeln!(to_debugger, "pc").unwrap();
    a.wait_until(|t| pc_lines(t) >= 1, "the debugger answered pc, so its run has begun");
    writeln!(to_debuggee, "pc").unwrap();
    b.wait_until(|t| pc_lines(t) >= 1, "and the debuggee answered pc, in step with it");
    // Held with the debuggee live at the other end, stepped seven, asked
    // for the two things an end of the cable does not write, and run on.
    write!(
        to_debugger,
        "hold\nstep 7\ncheckpoint {}\nstartcapture\nendcapture\ninfo\ncontinue\n",
        chk.display()
    )
    .unwrap();
    a.wait_until(
        |t| pc_lines(t) >= 3 && t.contains("engine: rtl"),
        "hold and step said where the debugger is, and info what the run is",
    );
    // The debuggee's quit first: it tells the debugger it is done and
    // waits for the debugger's own, which the debugger's quit sends.
    writeln!(to_debuggee, "quit").unwrap();
    writeln!(to_debugger, "quit").unwrap();
    let (a, b) = (debugger.wait(), debuggee.wait());
    drop((to_debugger, to_debuggee));
    let (ta, tb) = (text(&a), text(&b));
    assert!(a.status.success(), "the debugger:\n{ta}");
    assert!(b.status.success(), "the debuggee:\n{tb}");
    let pcs: Vec<&str> = ta.lines().filter(|l| l.starts_with("PC ")).collect();
    assert_eq!(pcs.len(), 3, "pc, hold and step each said where the debugger is:\n{ta}");
    assert_eq!(after(pcs[2]), after(pcs[1]) + 7, "step 7 moved it seven microcycles:\n{ta}");
    assert!(
        ta.contains("checkpoint: none on an end of the debug cable"),
        "checkpoint refused, with the reason:\n{ta}"
    );
    assert!(!chk.exists(), "and none was written");
    assert_eq!(
        ta.matches("capture: none on an end of the debug cable").count(),
        2,
        "startcapture and endcapture each refused, with the reason:\n{ta}"
    );
    assert!(ta.contains("quit at PC"), "the debugger ended by quit:\n{ta}");
    assert!(tb.contains("quit at PC"), "and so did the debuggee:\n{tb}");
    assert!(
        !ta.contains("muir: the debug cable") && !tb.contains("muir: the debug cable"),
        "the two agreed to stop rather than one finding the other gone:\n{ta}\n{tb}"
    );
}

/// **The debuggee runs before the debugger comes, and after it goes.** Its
/// DBGIN is a connector, not a plugged cable: the machine answers `pc` with
/// nobody on the cable and has moved between two of them; a debugger
/// connects, is answered, quits, and the debuggee says the debugger is
/// done and that it is listening again; it answers `pc` again, takes a
/// second debugger the same way, and is quit.  Neither end finds the other
/// gone.  Before this the debuggee blocked in `accept` until the debugger
/// came and its run ended when the debugger went.
#[test]
fn the_debuggee_runs_before_the_debugger_comes_and_after_it_goes() {
    let mut debuggee = muir()
        .args(["--rtl", "--debug-cable-listen", "127.0.0.1:0", "--stop-after", "1000000000"])
        .stdin(Stdio::piped())
        .start();
    let addr = listening(&debuggee);
    let mut to_debuggee = debuggee.stdin();
    let b = debuggee.stdout();
    writeln!(to_debuggee, "pc").unwrap();
    b.wait_until(|t| pc_lines(t) >= 1, "the debuggee answered pc with nobody on the cable");
    std::thread::sleep(Duration::from_millis(100));
    writeln!(to_debuggee, "pc").unwrap();
    b.wait_until(|t| pc_lines(t) >= 2, "and again");
    let mut texts = Vec::new();
    for k in 1..=2 {
        let mut debugger = muir()
            .args(["--rtl", "--debug-cable-connect", &addr, "--stop-after", "1000000000"])
            .stdin(Stdio::piped())
            .start();
        debuggee.stderr().wait_until(
            |t| t.matches("the debugger connected from").count() == k,
            "the debuggee said the debugger came",
        );
        let mut to_debugger = debugger.stdin();
        writeln!(to_debugger, "pc").unwrap();
        debugger.stdout().wait_until(|t| pc_lines(t) >= 1, "the debugger answered pc");
        writeln!(to_debugger, "quit").unwrap();
        let a = debugger.wait();
        drop(to_debugger);
        let ta = text(&a);
        assert!(a.status.success(), "debugger {k}:\n{ta}");
        assert!(ta.contains("quit at PC"), "debugger {k} ended by quit:\n{ta}");
        debuggee.stderr().wait_until(
            |t| t.matches("is done; DBGIN listening at").count() == k,
            "the debuggee said the debugger is done and it is listening again",
        );
        writeln!(to_debuggee, "pc").unwrap();
        b.wait_until(|t| pc_lines(t) >= 2 + k, "the debuggee answered pc after the cable went");
        texts.push(ta);
    }
    writeln!(to_debuggee, "quit").unwrap();
    let out = debuggee.wait();
    drop(to_debuggee);
    let tb = text(&out);
    assert!(out.status.success(), "the debuggee:\n{tb}");
    let pcs: Vec<&str> = tb.lines().filter(|l| l.starts_with("PC ")).collect();
    assert_eq!(pcs.len(), 4, "pc answered four times:\n{tb}");
    assert!(after(pcs[1]) > after(pcs[0]), "and the machine ran with nobody on the cable:\n{tb}");
    assert!(tb.contains("quit at PC"), "the debuggee ended by quit:\n{tb}");
    for t in texts.iter().chain([&tb]) {
        assert!(!t.contains("muir: the debug cable"), "nobody found the other end gone:\n{t}");
    }
}

/// **^C at an end of the cable is what it is on a machine alone**: the
/// first holds the debugger at the prompt, `continue` runs it on, and ^C
/// while held ends the run as `quit` does, the two ends agreeing to stop.
/// Before this one ^C ended a cable run outright, with nothing held.
#[test]
fn control_c_holds_the_debugger_over_tcp_and_again_quits() {
    let (mut debuggee, mut debugger) = pair();
    let (mut to_debugger, mut to_debuggee) = (debugger.stdin(), debuggee.stdin());
    let a = debugger.stdout();
    writeln!(to_debugger, "pc").unwrap();
    a.wait_until(|t| pc_lines(t) >= 1, "pc answered, so the run has begun");
    debugger.interrupt();
    a.wait_until(|t| t.contains("held at ^C") && pc_lines(t) >= 2, "held at the first ^C");
    writeln!(to_debugger, "continue\npc").unwrap();
    a.wait_until(|t| pc_lines(t) >= 3, "continue ran the debugger on, and pc answered");
    debugger.interrupt();
    a.wait_until(|t| t.matches("held at ^C").count() >= 2, "held at the second ^C");
    // The debuggee's quit first, as above, so that the debugger's own end
    // is agreed and not merely noticed.
    writeln!(to_debuggee, "quit").unwrap();
    debugger.interrupt();
    let (a, b) = (debugger.wait(), debuggee.wait());
    drop((to_debugger, to_debuggee));
    let (ta, tb) = (text(&a), text(&b));
    assert!(a.status.success(), "the debugger ended as a quit does:\n{ta}");
    assert!(b.status.success(), "the debuggee:\n{tb}");
    assert_eq!(ta.matches("held at ^C").count(), 2, "held twice, run on once between:\n{ta}");
    assert!(ta.contains("quit at PC") && tb.contains("quit at PC"), "{ta}\n{tb}");
    assert!(
        !ta.contains("muir: the debug cable") && !tb.contains("muir: the debug cable"),
        "the two agreed to stop:\n{ta}\n{tb}"
    );
}
