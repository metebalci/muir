// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The serial port under `muir` itself: `--serial` opens the endpoint and
//! nothing else does, a named port that is taken stops the run, and a
//! connection is the device on the null-modem cable plugging in.
//!
//! `tests/serial_endpoint.rs` holds what the endpoint does with the
//! characters; this holds the flag.

mod support;

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use support::{Child, Run, muir, text};

/// A run long enough to be looked at, started and left running: killed
/// when the test drops it.
fn running(args: &[&str]) -> Child {
    muir().args(args).args(["--stop-after", "4000000000"]).start()
}

/// The endpoint a run says its serial port is at, read off what it wrote
/// as it started.
fn served_at(child: &Child) -> SocketAddr {
    let said = "serial: tcp://";
    let there = |t: &str| t.lines().any(|l| l.starts_with(said));
    child.stderr().wait_until(there, "the run says where its serial port is");
    let wrote = child.stderr().so_far();
    let line = wrote.lines().find(|l| l.starts_with(said)).unwrap();
    let addr = line[said.len()..].split(' ').next().unwrap();
    addr.parse().unwrap_or_else(|e| panic!("{addr}: {e}\n{wrote}"))
}

/// A connection to `at`, waiting for the run to get its listener up.
fn connect(at: SocketAddr) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match TcpStream::connect(at) {
            Ok(s) => return s,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("{at}: {e}"),
        }
    }
}

/// **The port is off unless the flag is given.** It costs something to
/// have one --- on `chip` a port the machine has opened counts the
/// baud-rate crystal and the I/O board stops idling --- and the machine
/// is worked through the terminal, not through this. So nothing opens it
/// but `--serial`, and a run without the flag says nothing about a serial
/// endpoint because it has none.
#[test]
fn the_port_is_off_unless_the_flag_is_given() {
    let out = muir().args(["--micro", "--stop-after", "1"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(!t.contains("serial: tcp://"), "no endpoint was asked for:\n{t}");
}

/// **`--serial` says where the port is, and a connection plugs the device
/// in.** An endpoint whose port is the host's to pick is bound there, said
/// as it was bound, and answers.
#[test]
fn the_flag_says_where_the_port_is_and_a_connection_plugs_in() {
    let run = running(&["--micro", "--serial", "127.0.0.1:0"]);
    let at = served_at(&run);
    assert!(at.ip().is_loopback(), "the loopback unless told otherwise: {at}");
    let _device = connect(at);
    let said = format!("serial: {}", at.ip());
    run.stderr().wait_until(
        |t| t.lines().any(|l| l.starts_with("serial: ") && l.ends_with("connected")),
        &format!("the run says a device connected to {said}"),
    );
}

/// **A port that is taken stops the run.** The endpoint is where someone
/// is being told to attach, so muir does not quietly open another.
#[test]
fn a_port_that_is_taken_is_refused() {
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port().to_string();
    let out = muir().args(["--micro", "--serial", &port, "--stop-after", "1"]).run();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(2), "a usage error:\n{t}");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .find(|l| l.starts_with("muir: "))
            .is_some_and(|l| l.contains("--serial")),
        "the refusal names --serial:\n{t}"
    );
}

/// **The endpoint wants a port, since there is no default one to fall back
/// on.** A bare address names no port, and inventing one would put the
/// machine's serial line somewhere nobody was told about.
#[test]
fn the_endpoint_must_name_a_port() {
    for arg in ["127.0.0.1", "nowhere", ""] {
        let out = muir().args(["--micro", "--serial", arg, "--stop-after", "1"]).run();
        let t = text(&out);
        assert_eq!(out.status.code(), Some(2), "--serial {arg:?}:\n{t}");
        assert!(
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .find(|l| l.starts_with("muir: "))
                .is_some_and(|l| l.contains("--serial")),
            "the first line names --serial:\n{t}"
        );
    }
}
