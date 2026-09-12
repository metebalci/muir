// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The terminal under `muir` itself: every run serves one, since the
//! display, the keyboard and the mouse are the only way the machine is
//! worked; `--terminal` says where; and a second muir on the same host
//! takes the next display rather than stopping on the first one's port.

mod support;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use support::{Child, Run, muir, text};

/// A run long enough to be looked at, started and left running: killed
/// when the test drops it.
fn running(args: &[&str]) -> Child {
    muir().args(args).args(["--stop-after", "4000000000"]).start()
}

/// The endpoint a run says its terminal is at, read off what it wrote as
/// it started. `prefix` is `terminal: ` or `debuggee terminal: `.
fn served_at(child: &Child, prefix: &str) -> SocketAddr {
    let said = format!("{prefix}vnc://");
    let there = |t: &str| t.lines().any(|l| l.starts_with(&said));
    child.stderr().wait_until(there, &format!("the run says where its {prefix}is"));
    let wrote = child.stderr().so_far();
    let line = wrote.lines().find(|l| l.starts_with(&said)).unwrap();
    let addr = line[said.len()..].split(' ').next().unwrap();
    addr.parse().unwrap_or_else(|e| panic!("{addr}: {e}\n{wrote}"))
}

/// RFC 6143's opening twelve bytes from the endpoint: what is there is an
/// RFB server and not merely a bound port. The viewer says the version
/// back, so that the run sees an ordinary viewer come and go.
fn rfb_version(at: SocketAddr) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut s = loop {
        match TcpStream::connect(at) {
            Ok(s) => break s,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("{at}: {e}"),
        }
    };
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    let mut version = [0u8; 12];
    s.read_exact(&mut version).unwrap_or_else(|e| panic!("{at}: the version offered: {e}"));
    s.write_all(&version).unwrap();
    String::from_utf8_lossy(&version).into_owned()
}

/// **Every run serves a terminal**, asked for or not: the machine has no
/// other way to be worked, so with no flag at all the display is VNC's
/// :0 on the loopback, and an RFB viewer is answered there.
#[test]
fn every_run_serves_a_terminal() {
    let run = running(&["--micro"]);
    let at = served_at(&run, "terminal: ");
    assert!(at.ip().is_loopback(), "the loopback unless told otherwise: {at}");
    assert!((5900..=5999).contains(&at.port()), "a VNC display: {at}");
    assert_eq!(rfb_version(at), "RFB 003.008\n", "an RFB server at {at}");
}

/// **A second muir takes the next display.** Two of them on one host is an
/// ordinary thing --- the lashup over TCP is two --- so a display already
/// taken moves the next run up rather than stopping it.
#[test]
fn a_second_muir_takes_the_next_display() {
    let first = running(&["--micro"]);
    let a = served_at(&first, "terminal: ");
    let second = running(&["--micro"]);
    let b = served_at(&second, "terminal: ");
    assert_ne!(a, b, "two runs, two displays");
    assert_eq!(rfb_version(a), "RFB 003.008\n", "the first at {a}");
    assert_eq!(rfb_version(b), "RFB 003.008\n", "the second at {b}");
}

/// **A terminal that was asked for and cannot be served stops the run.**
/// The free display is looked for only when nobody named a port; a named
/// one is where a viewer is being told to look, so muir does not quietly
/// serve another instead.
#[test]
fn an_asked_for_port_that_is_taken_is_refused() {
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port().to_string();
    let out = muir().args(["--micro", "--terminal", &port, "--stop-after", "1"]).run();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(2), "a usage error:\n{t}");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .find(|l| l.starts_with("muir: "))
            .is_some_and(|l| l.contains("--terminal")),
        "the refusal names --terminal:\n{t}"
    );
}

/// **`--terminal` says where.** An endpoint whose port is the host's to
/// pick is bound there, said as it was bound, and answers a viewer.
#[test]
fn the_flag_says_where_the_terminal_is() {
    let run = running(&["--micro", "--terminal", "127.0.0.1:0"]);
    let at = served_at(&run, "terminal: ");
    assert_eq!(rfb_version(at), "RFB 003.008\n", "an RFB server at {at}");
}
