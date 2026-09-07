// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The two-machine lashup under `muir` itself: the flags that put a second
//! machine on the debug cable, in one process and over TCP.  The machines
//! boot the boot PROM and run in step for a short window; nothing writes
//! the debug block, so the cable carries promises and no cycles.  What is
//! checked is that both ends run, agree to stop, and report.  The cable's
//! traffic itself is `tests/lashup.rs`.

mod support;

use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use muir::capture::PAIR_RULE;
use muir::simpletv::WIDTH;

use support::{Child, Run, muir, scratch, text};

/// `--debug-in-process`: two `rtl` machines in one process, the second on the
/// first's debug cable, both run for the window and both reported.
#[test]
fn a_debuggee_runs_beside_the_debugger_in_one_process() {
    let out = muir().args(["--rtl", "--debug-in-process", "--stop-after", "3000"]).run();
    let t = text(&out);
    assert!(out.status.success(), "muir --debug-in-process failed:\n{t}");
    assert!(t.contains("rtl, debugger"), "the debugger reported:\n{t}");
    assert!(t.contains("debuggee:") && t.contains("microcycles to"), "the debuggee reported:\n{t}");
    assert!(t.contains("0 debug cycles on the cable"), "no debug cycles without CC:\n{t}");
    assert!(t.contains("ran out at 3000"), "the window was the debugger's:\n{t}");
    // Both machines are served a terminal, neither being workable without
    // one; where they are is `the_debuggees_terminal_defaults_to_one_port_above`.
    assert!(t.contains("\nterminal: vnc://"), "the debugger's display:\n{t}");
    assert!(t.contains("\ndebuggee terminal: vnc://"), "the debuggee's display:\n{t}");
}

/// The address a debuggee's DBGIN listens on, said on its stderr once it
/// is bound.  The debuggee is given `--debug-cable-listen 127.0.0.1:0` and
/// the host picks the port, so there is none to guess at; and the
/// debugger is started once the address has been said, so there is none to
/// lose in between either.
fn listening(debuggee: &Child) -> String {
    const SAID: &str = "debug cable: DBGIN listening on ";
    debuggee.stderr().wait_until(|t| t.contains(SAID), "the debuggee said where it listens");
    let t = debuggee.stderr().so_far();
    t.lines().find_map(|l| l.trim().strip_prefix(SAID)).unwrap().to_string()
}

/// `--debug-cable-listen` and `--debug-cable-connect`: the same two machines in two
/// processes, the debugger connecting to the debuggee's DBGIN, both ending
/// their windows and telling each other so.
#[test]
fn a_debuggee_and_a_debugger_meet_over_tcp() {
    let debuggee = muir()
        .args(["--rtl", "--debug-cable-listen", "127.0.0.1:0", "--stop-after", "3000"])
        .start();
    let addr = listening(&debuggee);
    let debugger =
        muir().args(["--rtl", "--debug-cable-connect", &addr, "--stop-after", "3000"]).start();
    let a = debugger.wait();
    let b = debuggee.wait();
    let (ta, tb) = (text(&a), text(&b));
    assert!(a.status.success(), "the debugger failed:\n{ta}");
    assert!(b.status.success(), "the debuggee failed:\n{tb}");
    assert!(ta.contains("DBGOUT connected") && ta.contains("rtl, debugger"), "the debugger:\n{ta}");
    assert!(
        tb.contains("the debugger connected") && tb.contains("rtl, debuggee"),
        "the debuggee:\n{tb}"
    );
    assert!(
        ta.contains("ran out at 3000") && tb.contains("ran out at 3000"),
        "both windows ran out:\n{ta}\n{tb}"
    );
}

/// A viewer of our own: RFC 6143's opening exchange as far as `ServerInit`,
/// whose first four bytes are the screen's width and height.
fn rfb_screen(addr: &str) -> (u16, u16) {
    use std::io::{Read, Write};
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut s = loop {
        match TcpStream::connect(addr) {
            Ok(s) => break s,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => panic!("{addr}: {e}"),
        }
    };
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut version = [0u8; 12];
    s.read_exact(&mut version).unwrap();
    assert_eq!(&version, b"RFB 003.008\n", "{addr}: the version offered");
    s.write_all(&version).unwrap();
    let mut types = [0u8; 2];
    s.read_exact(&mut types).unwrap();
    assert_eq!(types, [1, 1], "{addr}: one security type, None");
    s.write_all(&[1]).unwrap();
    let mut result = [0u8; 4];
    s.read_exact(&mut result).unwrap();
    assert_eq!(result, [0; 4], "{addr}: SecurityResult ok");
    s.write_all(&[1]).unwrap();
    let mut init = [0u8; 24];
    s.read_exact(&mut init).unwrap();
    (u16::from_be_bytes([init[0], init[1]]), u16::from_be_bytes([init[2], init[3]]))
}

/// `--debug-in-process --terminal --debuggee-terminal`: both displays are
/// served, each a whole RFB server answering a viewer with the screen's
/// size.
#[test]
fn the_lashup_serves_both_displays() {
    // Both displays on ports the host picks, each said on stderr as it is
    // bound: port 0 twice is two ports, and `muir` takes it so.  The window
    // is long; the lashup is killed once both have answered, or by the
    // drop if either does not.
    let lashup = muir()
        .args([
            "--rtl",
            "--debug-in-process",
            "--terminal",
            "127.0.0.1:0",
            "--debuggee-terminal",
            "127.0.0.1:0",
            "--stop-after",
            "400000000",
        ])
        .start();
    let endpoints = |t: &str| {
        let endpoint = |rest: &str| rest.split(' ').next().unwrap().to_string();
        let (mut first, mut second) = (None, None);
        for line in t.lines() {
            if let Some(rest) = line.trim().strip_prefix("debuggee terminal: vnc://") {
                second = Some(endpoint(rest));
            } else if let Some(rest) = line.trim().strip_prefix("terminal: vnc://") {
                first = Some(endpoint(rest));
            }
        }
        first.zip(second)
    };
    lashup.stderr().wait_until(|t| endpoints(t).is_some(), "both displays said where they are");
    let (first, second) = endpoints(&lashup.stderr().so_far()).unwrap();
    assert_ne!(first, second, "two displays, two ports:\n{}", lashup.stderr().so_far());
    let a = rfb_screen(&first);
    let b = rfb_screen(&second);
    lashup.kill();
    let screen = (muir::simpletv::WIDTH as u16, muir::simpletv::HEIGHT as u16);
    assert_eq!(a, screen, "the debugger's screen");
    assert_eq!(b, screen, "the debuggee's screen");
}

/// `--debuggee-terminal` outside the lashup is refused, and so is the same
/// endpoint as `--terminal`.  The endpoint is a port the host has free,
/// bound and let go: every run serves a display, so a fixed port is
/// another run's as often as not, and `--terminal` names its port and is
/// refused when it is taken, before the two are compared.  Taken again in
/// between, the attempt is made on another.
#[test]
fn a_debuggee_terminal_wants_the_lashup() {
    let out = muir().args(["--rtl", "--debuggee-terminal", "--stop-after", "1"]).run();
    assert!(!out.status.success());
    assert!(text(&out).contains("needs --debug-in-process"), "{}", text(&out));
    for _ in 0..5 {
        let p = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port().to_string();
        let out = muir()
            .args(["--rtl", "--debug-in-process", "--terminal", &p, "--debuggee-terminal", &p])
            .run();
        let t = text(&out);
        if t.contains("Address already in use") {
            continue;
        }
        assert!(!out.status.success(), "{t}");
        assert!(t.contains("the same endpoint as --terminal"), "{t}");
        return;
    }
    panic!("no free port in five tries");
}

/// **The other machine has a Chaosnet of its own**: its own cable with its
/// own server on it, at the debugger's addresses unless it is given
/// others, and with no file service unless one is named for it --- two
/// servers rooted at one directory being two hosts sharing a filesystem.
/// Both flags are the other machine's, so both want the lashup.
#[test]
fn the_debuggee_has_a_chaosnet_of_its_own() {
    let out = muir().args(["--rtl", "--debug-in-process", "--stop-after", "100"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(
        t.contains("debuggee chaosnet: 3050, its own server at 3060, no file root"),
        "the debugger's addresses, and no FILE:\n{t}"
    );

    let out = muir()
        .args(["--rtl", "--debug-in-process", "--debuggee-chaos-address", "3051,3061"])
        .args(["--stop-after", "100"])
        .run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("debuggee chaosnet: 3051, its own server at 3061"), "{t}");
    assert!(t.contains("chaosnet: 3050, the server at 3060"), "this machine's are its own:\n{t}");

    for (flag, value) in [("--debuggee-chaos-address", "3051"), ("--debuggee-chaos-file-root", ".")]
    {
        let out = muir().args(["--rtl", flag, value, "--stop-after", "1"]).run();
        assert!(!out.status.success(), "{flag}: {}", text(&out));
        assert!(text(&out).contains("needs --debug-in-process"), "{flag}: {}", text(&out));
    }
}

/// `--chip --debug-cable-listen`: the netlist board is the debuggee over
/// TCP, an `rtl` debugger in the other process; both end their windows
/// and tell each other so.
#[test]
fn a_chip_debuggee_and_an_rtl_debugger_meet_over_tcp() {
    let debuggee = muir()
        .args(["--chip", "--main-memory", "model", "--io-board", "model", "--tv", "model"])
        .args(["--debug-cable-listen", "127.0.0.1:0", "--stop-after", "300"])
        .start();
    let addr = listening(&debuggee);
    let debugger =
        muir().args(["--rtl", "--debug-cable-connect", &addr, "--stop-after", "300"]).start();
    let a = debugger.wait();
    let b = debuggee.wait();
    let (ta, tb) = (text(&a), text(&b));
    assert!(a.status.success(), "the debugger failed:\n{ta}");
    assert!(b.status.success(), "the debuggee failed:\n{tb}");
    assert!(
        ta.contains("rtl, debugger") && ta.contains("ran out at 300"),
        "the debugger reported:\n{ta}"
    );
    assert!(
        tb.contains("chip, debuggee") && tb.contains("ran out at 300"),
        "the debuggee reported:\n{tb}"
    );
    assert!(tb.contains("0 debug cycles on the cable"), "no debug cycles without CC:\n{tb}");
}

/// `--tv-capture` in the lashup: one recording of both machines, the
/// canvas two screens and the rule between them wide, and the run says
/// where it went.  There is no flag for the debuggee's display of its own:
/// in one process the two machines are one clock, and one file is the only
/// way to keep them on it.
#[test]
fn the_lashup_records_both_displays_on_one_canvas() {
    let dir = scratch("lashup-capture");
    let gif = dir.join("lashup.gif");
    let out = muir()
        .args(["--rtl", "--debug-in-process", "--stop-after", "5000", "--tv-capture"])
        .arg(&gif)
        .run();
    let t = text(&out);
    assert!(out.status.success(), "muir failed:\n{t}");
    let bytes =
        std::fs::read(&gif).unwrap_or_else(|e| panic!("the recording {}: {e}", gif.display()));
    assert!(bytes.starts_with(b"GIF89a"), "the recording is a GIF");
    let width = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;
    assert_eq!(width, 2 * WIDTH + PAIR_RULE, "both screens on one canvas");
    assert!(
        t.contains(&format!("frames of the display at {}", gif.display())),
        "the recording reported:\n{t}"
    );
}

/// `--debug-in-process --terminal <port>` alone: the debuggee's display
/// is served one port above the debugger's. The pair is picked by binding
/// and releasing it, and the run is one microcycle, so the window in
/// which another program could take one of the two is small; taken all
/// the same --- the debugger's port refused, or the debuggee's moved up
/// to the next free display --- the attempt is made again on another
/// pair.
#[test]
fn the_debuggees_terminal_defaults_to_one_port_above() {
    for _ in 0..5 {
        let p = {
            let a = TcpListener::bind("127.0.0.1:0").unwrap();
            let p = a.local_addr().unwrap().port();
            if p == u16::MAX || TcpListener::bind(("127.0.0.1", p + 1)).is_err() {
                continue;
            }
            p
        };
        let out = muir()
            .args([
                "--rtl",
                "--debug-in-process",
                "--terminal",
                &p.to_string(),
                "--stop-after",
                "1",
            ])
            .run();
        let t = text(&out);
        if !out.status.success() && t.contains("Address already in use") {
            continue;
        }
        assert!(out.status.success(), "the lashup failed:\n{t}");
        assert!(t.contains(&format!("terminal: vnc://127.0.0.1:{p} ")), "the debugger's:\n{t}");
        if !t.contains(&format!("debuggee terminal: vnc://127.0.0.1:{} ", p + 1)) {
            continue;
        }
        return;
    }
    panic!("no free pair of ports in five tries");
}
