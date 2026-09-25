// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The terminal under `muir` itself: every run serves one, since the
//! display, the keyboard and the mouse are the only way the machine is
//! worked; `--terminal` says where; and a second muir on the same host
//! takes the next display rather than stopping on the first one's port.

mod support;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::Stdio;
use std::time::{Duration, Instant};

use support::{Child, Run, cadr, text};

/// A run long enough to be looked at, started and left running: killed
/// when the test drops it.
fn running(args: &[&str]) -> Child {
    cadr().args(args).args(["--stop-after", "4000000000"]).start()
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
    let mut s = dial(at);
    let mut version = [0u8; 12];
    s.read_exact(&mut version).unwrap_or_else(|e| panic!("{at}: the version offered: {e}"));
    s.write_all(&version).unwrap();
    String::from_utf8_lossy(&version).into_owned()
}

/// A connection to a run's terminal, waited for: the port is bound before
/// the run says where it is, but a run that has not got there yet is
/// still starting.
fn dial(at: SocketAddr) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(30);
    let s = loop {
        match TcpStream::connect(at) {
            Ok(s) => break s,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("{at}: {e}"),
        }
    };
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    s
}

/// A viewer through RFC 6143's opening exchange as 3.8, so that what it
/// sends after it is read as messages: the version, `None` of the
/// security types offered, `ClientInit`, and `ServerInit` read back.
fn viewer(at: SocketAddr) -> TcpStream {
    let mut s = dial(at);
    let mut version = [0u8; 12];
    s.read_exact(&mut version).unwrap();
    s.write_all(&version).unwrap();
    let mut types = [0u8; 2];
    s.read_exact(&mut types).unwrap();
    assert_eq!(types, [1, 1], "one security type on offer, and it is None");
    s.write_all(&[1]).unwrap();
    let mut result = [0u8; 4];
    s.read_exact(&mut result).unwrap();
    assert_eq!(result, [0, 0, 0, 0], "SecurityResult, and it is ok");
    // ClientInit's shared flag, then ServerInit: 24 bytes and a name.
    s.write_all(&[1]).unwrap();
    let mut head = [0u8; 24];
    s.read_exact(&mut head).unwrap();
    let name = u32::from_be_bytes(head[20..24].try_into().unwrap()) as usize;
    s.read_exact(&mut vec![0u8; name]).unwrap();
    s
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
    let out = cadr().args(["--micro", "--terminal", &port, "--stop-after", "1"]).run();
    let t = text(&out);
    assert_eq!(out.status.code(), Some(2), "a usage error:\n{t}");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .find(|l| l.starts_with("cadr: "))
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

/// **A run says when its terminal loses typing, without being asked to.**
/// What a viewer types waits for the machine's next look ---
/// [`muir::terminal::INPUT_BACKLOG`] events of it, and the run looks every
/// 33 ms --- and beyond that the oldest keystroke goes. A character that
/// did not type looks exactly like a key with no binding, so the run says
/// so rather than leaving the loss to be guessed at; one line, and
/// `--keyboard-mapping-trace` says it again as more go.
#[test]
fn a_run_says_when_its_terminal_loses_typing() {
    let run = running(&["--micro", "--terminal", "127.0.0.1:0"]);
    let at = served_at(&run, "terminal: ");
    let mut v = viewer(at);
    // Four thousand key-downs in one write, which crosses the loopback
    // inside one of the run's polls: a burst smaller than the queue, or
    // one spread over several polls, is a queue the run kept up with and
    // nothing is lost.
    let mut burst = Vec::new();
    for _ in 0..4000 {
        burst.extend_from_slice(&[4u8, 1, 0, 0]);
        burst.extend_from_slice(&('a' as u32).to_be_bytes());
    }
    v.write_all(&burst).unwrap();
    let said = |t: &str| t.lines().any(|l| l.starts_with("terminal: the input queue was full: "));
    run.stderr().wait_until(said, "the run says what its terminal lost");
    // That there is one such line a run, and what the trace makes of the
    // count after it, is `tests/terminal.rs`'s to hold; what this holds is
    // that a run prints it at all, and names the flag that says the rest.
    let wrote = run.stderr().so_far();
    let line = wrote.lines().find(|l| l.starts_with("terminal: the input queue")).unwrap();
    assert!(line.ends_with("--keyboard-mapping-trace says as more go"), "{line}");
}

/// **^C ends the serving of the last screen, and the run ends as its stop
/// would have.** A run whose viewer is still looking when it stops keeps
/// serving the screen and says `^C to stop`, and that has to be so on
/// every run function: the ^C returns from the serving, so the run comes
/// back the way it does from any stop and what it holds is dropped ---
/// the process is not ended under it, which a `kill -KILL`, the way out
/// before this, would do. Held on `micro`, `time_engine`, and on `chip`,
/// `time_chip`, which was one of the five run functions that reached the
/// serving with ^C still only counted (issue 103).
///
/// The run starts held with the button unpressed, so that the viewer is
/// on the terminal before the machine runs its ten microcycles and stops;
/// `boot` down stdin runs it. The viewer stays until the run has ended,
/// so that it is the ^C and not its leaving that ends the serving.
#[test]
fn control_c_ends_the_serving_of_the_last_screen() {
    for engine in [&["--micro"][..], &["--chip", "--main-memory-boards", "1"][..]] {
        let mut run = cadr()
            .args(engine)
            .args(["--no-auto-boot", "--stop-after", "10", "--terminal", "127.0.0.1:0"])
            .stdin(Stdio::piped())
            .start();
        let at = served_at(&run, "terminal: ");
        let v = viewer(at);
        let mut stdin = run.stdin();
        writeln!(stdin, "boot").unwrap();
        let serving =
            |t: &str| t.contains("serving the last screen while a viewer is on it; ^C to stop");
        run.stderr().wait_until(serving, "the run stopped with the viewer on its terminal");
        run.interrupt();
        let out = run.wait();
        drop(v);
        drop(stdin);
        let t = text(&out);
        assert!(out.status.success(), "{engine:?}: the ^C ended the run as its stop does:\n{t}");
        assert!(t.contains("ran out at 10"), "{engine:?}: and the stop was the run's own:\n{t}");
    }
}
