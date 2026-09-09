// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The serial port's route out of the process: [`muir::serial::Endpoint`],
//! the TCP endpoint `muir --serial` opens, against the behavioural far end
//! of `src/serial.rs`.
//!
//! What is held here is the plug and the characters. `tests/serial.rs`
//! holds the 2651 itself and `tests/serial_cable.rs` the netlist board's
//! far end, the endpoint on it among them; nothing here is a claim about
//! the hardware that those do not already make.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use muir::ioboard::{IoBoard, csr};
use muir::serial::{COMMAND, DATA, Endpoint, MODE, STATUS, command, mode1, mode2, status};

/// How long a check waits for the host to carry a byte across the
/// loopback, which is not instant and is not the model's business.
const PATIENCE: Duration = Duration::from_secs(10);

/// How long a check waits to be sure nothing is coming.
const BRIEFLY: Duration = Duration::from_millis(50);

/// 19,200 baud, the fastest the generator goes, so that a frame is half a
/// millisecond of the machine's time rather than two hundred.
const RATE: u8 = 15;

/// One frame of eight-N-one at [`RATE`]: a start bit, eight data bits and
/// a stop bit.
fn frame_ns() -> u64 {
    10 * muir::serial::bit_ns(RATE)
}

/// An endpoint on a port the host picks, and where it is.
fn endpoint() -> (Endpoint, SocketAddr) {
    let e = Endpoint::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).expect("a free port");
    let at = e.addr().expect("where it is");
    (e, at)
}

/// The port at [`RATE`], eight data bits, no parity, one stop bit, both
/// halves enabled with `-DTR` and `-RTS` asserted: what `serial.lisp`'s
/// `:INIT` does, at this test's rate and frame.
fn set_up(b: &mut IoBoard, ns: u64) {
    b.read(COMMAND, ns);
    b.write(MODE, (0o1 << mode1::STOP_SHIFT | 3 << mode1::LENGTH_SHIFT | 2) as u16, ns);
    b.write(MODE, (mode2::RX_INTERNAL | mode2::TX_INTERNAL | RATE) as u16, ns);
    let cr = command::TX_ENABLE | command::RX_ENABLE | command::DTR | command::RTS;
    b.write(COMMAND, cr as u16, ns);
}

/// The status register's low byte at `ns`, the upper byte floating as it
/// does on every read of the group.
fn status_at(b: &mut IoBoard, ns: u64) -> u8 {
    let word = b.read(STATUS, ns);
    assert_eq!(word & csr::FLOATING, csr::FLOATING, "nothing drives the upper byte");
    word as u8
}

/// Polls the endpoint until `done`, giving the host time to carry what was
/// written across the loopback. The machine's clock stands at `ns`: what is
/// being waited for is the socket, not the port.
fn poll_until(
    end: &mut Endpoint,
    b: &mut IoBoard,
    ns: u64,
    why: &str,
    done: impl Fn(&Endpoint, &IoBoard) -> bool,
) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        end.poll_cable(&mut b.serial.cable, ns);
        if done(end, b) {
            return;
        }
        assert!(Instant::now() < deadline, "{why}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// **A connection is a device plugging in.** The Signetics sheet: the chip
/// "is conditioned to transmit data when the -CTS input is low" and
/// "conditioned to receive data when the -DCD input is low", so bytes alone
/// would leave the port unable to do anything with them. A connection
/// asserts `DSR`, `DCD` and `CTS`, as a device on a null-modem cable does
/// with its own `DTR` and `RTS`, and hanging up drops all three.
#[test]
fn a_connection_asserts_dsr_dcd_and_cts_and_hanging_up_drops_them() {
    let (mut end, at) = endpoint();
    let mut b = IoBoard::default();
    set_up(&mut b, 0);
    end.poll_cable(&mut b.serial.cable, 0);
    assert!(!end.connected(), "nothing has connected yet");
    let s = status_at(&mut b, 0);
    assert_eq!(s & (status::DSR | status::DCD), 0, "nothing on J9: {s:o}");
    assert!(!b.serial.cts(), "and no -CTS, so the transmitter is held");

    let stream = TcpStream::connect(at).expect("the endpoint answers");
    poll_until(&mut end, &mut b, 0, "the connection is accepted", |e, _| e.connected());
    assert!(b.serial.dsr() && b.serial.dcd() && b.serial.cts(), "all three asserted");
    let s = status_at(&mut b, 0);
    assert_eq!(s & (status::DSR | status::DCD), status::DSR | status::DCD, "{s:o}");

    drop(stream);
    poll_until(&mut end, &mut b, 0, "the hangup is noticed", |e, _| !e.connected());
    assert!(!b.serial.dsr() && !b.serial.dcd() && !b.serial.cts(), "all three dropped");
    let s = status_at(&mut b, 0);
    assert_eq!(s & (status::DSR | status::DCD), 0, "{s:o}");
}

/// **A character typed at the socket arrives in the receive holding
/// register**, framed as the port was programmed and taking the port's own
/// frame time to get there: the endpoint carries characters and picks no
/// rate of its own.
#[test]
fn a_character_typed_at_the_socket_reaches_the_receive_holding_register() {
    let (mut end, at) = endpoint();
    let mut b = IoBoard::default();
    set_up(&mut b, 0);
    let mut stream = TcpStream::connect(at).expect("the endpoint answers");
    poll_until(&mut end, &mut b, 0, "the connection is accepted", |e, _| e.connected());
    stream.write_all(b"Q").expect("a character to the port");
    poll_until(&mut end, &mut b, 0, "the character reaches the cable", |_, b| {
        b.serial.cable.pending() == 1
    });
    let s = status_at(&mut b, frame_ns() / 2);
    assert_eq!(s & status::RX_READY, 0, "still on the wire half a frame in: {s:o}");
    let s = status_at(&mut b, frame_ns() + 1);
    assert_eq!(s & status::RX_READY, status::RX_READY, "a character: {s:o}");
    assert_eq!(b.read(DATA, frame_ns() + 1) as u8, b'Q');
}

/// **A character the port sends comes out of the socket**, and not before
/// the port has finished shifting it out.
#[test]
fn a_character_the_port_sends_comes_out_of_the_socket() {
    let (mut end, at) = endpoint();
    let mut b = IoBoard::default();
    set_up(&mut b, 0);
    let mut stream = TcpStream::connect(at).expect("the endpoint answers");
    poll_until(&mut end, &mut b, 0, "the connection is accepted", |e, _| e.connected());
    b.write(DATA, b'z' as u16, 0);

    // Half a frame in it is still on the wire, so nothing is on the socket.
    let half = frame_ns() / 2;
    b.serial.advance(half);
    end.poll_cable(&mut b.serial.cable, half);
    stream.set_read_timeout(Some(BRIEFLY)).unwrap();
    let mut got = [0u8; 1];
    assert!(stream.read(&mut got).is_err(), "the frame is still going out");

    // A frame later it is out, and the endpoint hands it to the socket.
    let done = frame_ns() * 2;
    b.serial.advance(done);
    end.poll_cable(&mut b.serial.cable, done);
    stream.set_read_timeout(Some(PATIENCE)).unwrap();
    stream.read_exact(&mut got).expect("the character");
    assert_eq!(got[0], b'z');
}

/// **One device is on the far end of a null-modem cable**, so a second
/// connection is closed as it arrives rather than shouting over the first.
#[test]
fn a_second_connection_is_turned_away() {
    let (mut end, at) = endpoint();
    let mut b = IoBoard::default();
    set_up(&mut b, 0);
    let first = TcpStream::connect(at).expect("the endpoint answers");
    poll_until(&mut end, &mut b, 0, "the connection is accepted", |e, _| e.connected());
    let mut second = TcpStream::connect(at).expect("the listener still accepts");
    for _ in 0..8 {
        end.poll_cable(&mut b.serial.cable, 0);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(end.connected(), "the first is still the device on the cable");
    second.set_read_timeout(Some(PATIENCE)).unwrap();
    let mut got = [0u8; 1];
    assert_eq!(second.read(&mut got).ok(), Some(0), "the second connection is closed");
    drop(first);
}
