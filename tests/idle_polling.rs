// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Skipping work on a quiet port and a quiet cable must change nothing.
//!
//! The behavioural engines advance the whole I/O board every microcycle,
//! and on a machine with nothing plugged into the serial port and a quiet
//! Chaosnet most of that work discovers there is nothing to do. The engine
//! is allowed to notice and skip it: [`muir::serial::Pci::advance`]
//! returns at once when neither half of the port has anything in flight,
//! [`Interface`] remembers the earliest instant it has anything to do
//! rather than working it out again every microcycle, and the turn timer
//! takes a run of terminal counts at once when the cable is idle and
//! nothing is ready to send.
//!
//! All three are the same claim: **the state reached does not depend on
//! how finely the model was stepped to get there.** So that is what is
//! held here. Each scenario runs twice from the same start --- once a
//! microcycle at a time, once jumping straight from one observation to
//! the next --- and the two traces must be equal. A skip that skipped
//! something, or a batched run of terminal counts whose arithmetic is
//! wrong, is a difference between the two traces.
//!
//! The traces are of what the machine and the far end can see: the port's
//! status register, what reached the cable and when, the interface's CSR,
//! its bit count and its interrupt request, and the ether's log.

use muir::chaos::board::Interface;
use muir::chaos::ether::{Capture, Ether, Event};
use muir::chaos::interface::{CSR, READ_BUFFER, START, WRITE_BUFFER, csr};
use muir::ioboard::IoBoard;
use muir::serial::{COMMAND, DATA, MODE, command, mode1, mode2};

/// A microcycle, near enough: `rtl` runs about 180 ns to one, which is
/// the grain the engine actually advances the board at.
const MICROCYCLE: u64 = 180;

/// How often the reference run is observed. Every step compared against
/// it is a multiple of this, so a coarser run's observations are a
/// subsequence of the reference's and fall on the same instants.
const GRID: u64 = 1_000;

/// The steps a scenario is run at and held to the reference. The largest
/// is well over [`POLL_NS`]-sized, so a run of a hundred terminal counts
/// and several looks at the cable are taken in one jump.
const STEPS: [u64; 3] = [1_000, 5_000, 50_000];

/// The reference's observations at multiples of `grid`, to compare with a
/// run that could only be observed that often.
fn every<T: Clone>(seen: &[(u64, T)], grid: u64) -> Vec<(u64, T)> {
    seen.iter().filter(|(t, _)| t % grid == 0).cloned().collect()
}

/// Runs `b` up to `t` in `step` increments, ending on `t` exactly.
fn creep(b: &mut IoBoard, from: u64, to: u64, step: u64) {
    let mut u = from;
    while u + step < to {
        u += step;
        b.advance(u);
    }
    b.advance(to);
}

/// The same, for a bare interface.
fn creep_chaos(i: &mut Interface, from: u64, to: u64, step: u64) {
    let mut u = from;
    while u + step < to {
        u += step;
        i.advance(u);
    }
    i.advance(to);
}

// --- The serial port --------------------------------------------------------

/// 8 data bits, no parity, one stop bit, at `rate`, both halves enabled.
/// `tests/serial.rs` has the same sequence with the sheet's citations on
/// it; this is a copy because a test's helpers are private to its file.
fn set_up(b: &mut IoBoard, rate: u8, ns: u64) {
    b.read(COMMAND, ns);
    b.write(MODE, (0o1 << mode1::STOP_SHIFT | 3 << mode1::LENGTH_SHIFT | 2) as u16, ns);
    b.write(MODE, (mode2::RX_INTERNAL | mode2::TX_INTERNAL | rate) as u16, ns);
    b.write(
        COMMAND,
        (command::TX_ENABLE | command::RX_ENABLE | command::DTR | command::RTS) as u16,
        ns,
    );
}

/// One observation of the port: the status register as it stands, how many
/// of the far end's characters it has not taken yet, and everything it has
/// put on the cable so far with the instant each one's stop bit ended.
type PortSeen = (u64, (u8, usize, Vec<(u64, u8)>));

/// A port with traffic both ways and the cable pulled out and put back
/// under it, advanced in `step` increments and observed every `grid`.
///
/// The unplug is in the script on purpose: the port is quiet on either
/// side of it, which is exactly when the early-out is taken.
fn port_trace(step: u64, grid: u64) -> Vec<PortSeen> {
    let mut b = IoBoard::default();
    b.serial.cable.plug(0);
    set_up(&mut b, 15, 0);
    let mut seen = Vec::new();
    let mut sent = Vec::new();
    let mut t = 0;
    while t < 8_000_000 {
        let next = t + grid;
        creep(&mut b, t, next, step);
        t = next;
        // The script, at instants both runs land on exactly.
        match t {
            1_000_000 => b.write(DATA, b'A' as u16, t),
            1_050_000 => b.write(DATA, b'B' as u16, t),
            1_200_000 => b.serial.cable.send(b'x', t),
            2_000_000 => b.serial.cable.send(b'y', t),
            2_050_000 => b.serial.cable.send(b'z', t),
            3_000_000 => b.write(DATA, b'C' as u16, t),
            4_000_000 => b.serial.cable.unplug(),
            4_500_000 => b.write(DATA, b'D' as u16, t),
            5_000_000 => b.serial.cable.plug(t),
            6_000_000 => b.write(DATA, b'E' as u16, t),
            6_500_000 => b.serial.cable.send(b'w', t),
            _ => {}
        }
        b.advance(t);
        while let Some(c) = b.serial.cable.take() {
            sent.push(c);
        }
        seen.push((t, (b.serial.status(), b.serial.cable.pending(), sent.clone())));
    }
    seen
}

/// **A port stepped a microcycle at a time and a port jumped between
/// observations are the same port.** Characters go out at the same
/// instants, come in at the same instants, and the status register reads
/// the same all the way through, including over the stretch where the
/// cable is out.
#[test]
fn the_port_reaches_the_same_state_stepped_or_jumped() {
    let reference = port_trace(MICROCYCLE, GRID);
    for step in STEPS {
        let coarse = port_trace(step, step);
        assert_eq!(every(&reference, step), coarse, "stepped {step} ns at a time");
    }
    // The scenario has to have exercised something, or the equality above
    // is the equality of two empty traces.
    let (_, (_, _, sent)) = reference.last().unwrap();
    assert_eq!(
        sent.iter().map(|&(_, c)| c).collect::<Vec<_>>(),
        b"ABCDE",
        "five characters out, `D` waiting in the holding register for -CTS \
         until the cable went back in"
    );
    assert!(
        reference.iter().any(|&(_, (_, pending, _))| pending > 0),
        "and the far end's characters queued at some point"
    );
}

// --- The Chaosnet interface -------------------------------------------------

/// One observation of the interface: the CSR, the bit count, the interrupt
/// it is asking for, and how many events the cable has logged.
type CableSeen = (u64, (u16, u16, Option<u16>, usize));

/// A packet: the words as software writes them, cable destination last.
fn packet(to: u16, n: usize) -> Vec<u16> {
    let mut p: Vec<u16> = (0..n).map(|k| 0o1000 + k as u16).collect();
    p.push(to);
    p
}

/// An interface with a cable, sending and receiving, advanced in `step`
/// increments and observed every `grid`.
fn cable_trace(step: u64, grid: u64) -> (Vec<CableSeen>, Vec<(u64, u16)>, Vec<u16>) {
    let mut e = Ether::new();
    e.keep_log(true);
    e.attach(Box::new(Capture::new(0o3040)));
    let mut i = Interface::new(0o1440, Some(e), 0, false);
    i.write(CSR, csr::RECEIVE_INT_ENABLE | csr::TRANSMIT_INT_ENABLE, 0);
    let mut seen = Vec::new();
    let mut read_back = Vec::new();
    let mut t = 0;
    while t < 4_000_000 {
        let next = t + grid;
        creep_chaos(&mut i, t, next, step);
        t = next;
        match t {
            // A packet of this interface's own: the words, then the read
            // of START that makes it ready for the turn timer.
            500_000 => {
                for w in packet(0o3040, 4) {
                    i.write(WRITE_BUFFER, w, t);
                }
                i.read(START, t);
            }
            // One the other way, from a transmitter that does not wait for
            // its turn, while this interface has nothing going.
            1_500_000 => {
                let words = packet(0o1440, 6);
                i.ether_mut().unwrap().send_now(t, 0o3040, words);
            }
            // Read out whatever landed, and clear the receiver for the
            // next.
            2_000_000 => {
                while i.read(CSR, t) & csr::RECEIVE_DONE != 0 && read_back.len() < 32 {
                    read_back.push(i.read(READ_BUFFER, t));
                    if i.read(muir::chaos::interface::BIT_COUNT, t) == 0o7777 {
                        i.write(CSR, csr::CLEAR_RECEIVER, t);
                    }
                }
            }
            // A second of this interface's own, so the turn timer runs a
            // countdown that was loaded by a frame it heard.
            2_500_000 => {
                for w in packet(0o3040, 2) {
                    i.write(WRITE_BUFFER, w, t);
                }
                i.read(START, t);
            }
            _ => {}
        }
        i.advance(t);
        let log = i.ether().unwrap().log.len();
        seen.push((t, (i.csr(), i.bit_count(), i.interrupt_request(), log)));
    }
    let sent = i
        .ether()
        .unwrap()
        .log
        .iter()
        .filter_map(|ev| if let Event::Sent(at, s, _) = ev { Some((*at, *s)) } else { None })
        .collect();
    (seen, sent, read_back)
}

/// **An interface stepped a microcycle at a time and an interface jumped
/// between observations are the same interface.** The turn timer's
/// terminal counts fall where they fall whether they are taken one at a
/// time or in a run: the frames start at the same instant, Transmit Done
/// and Receive Done come at the same instant, and the same words are read
/// back out of the buffer.
///
/// The turn timer counts every 500 ns against a microcycle of about 180, so the
/// reference run takes every terminal count on its own and the coarsest
/// run takes a hundred of them at a time.  What a run of counts comes to
/// is held directly, against a stepped one, by the unit tests beside
/// `Turn` in `src/chaos/board.rs`.
#[test]
fn the_cable_reaches_the_same_state_stepped_or_jumped() {
    let (reference, ref_sent, ref_read) = cable_trace(MICROCYCLE, GRID);
    for step in STEPS {
        let (coarse, sent, read) = cable_trace(step, step);
        assert_eq!(every(&reference, step), coarse, "stepped {step} ns at a time");
        assert_eq!(ref_sent, sent, "the frames went at the same instants at {step} ns");
        assert_eq!(ref_read, read, "and the same words came out of the buffer at {step} ns");
    }
    // Again, the scenario has to have done something.
    assert_eq!(ref_sent.len(), 3, "two frames of this interface's own and one from the far end");
    assert_eq!(
        ref_read.len(),
        9,
        "the far end's six words and its destination, then its source and check"
    );
}

/// **A read of START brings the interface's next event forward, and it is
/// noticed.** The interface is advanced a long way with nothing to do
/// first, so that anything it remembers about when it is next due is as
/// stale as it can be; the packet handed to it after that must still go.
///
/// This is the one the cached instant can get wrong on its own, without
/// the traces above showing it: they load the buffer at instants the
/// interface was busy near anyway.
#[test]
fn a_start_after_a_long_quiet_stretch_still_goes() {
    let mut e = Ether::new();
    e.keep_log(true);
    e.attach(Box::new(Capture::new(0o3040)));
    let mut i = Interface::new(0o1440, Some(e), 0, false);
    i.advance(60_000_000);
    for w in packet(0o3040, 3) {
        i.write(WRITE_BUFFER, w, 60_000_000);
    }
    i.read(START, 60_000_000);
    assert_eq!(i.csr() & csr::TRANSMIT_DONE, 0, "the buffer is loaded and not out yet");
    // The wait is the turn timer's: the low byte counts down on every
    // second terminal count and the frame starts when it carries, so at
    // worst 256 counts, 256 us, after the buffer is ready.
    i.advance(60_400_000);
    assert_eq!(
        i.csr() & csr::TRANSMIT_DONE,
        csr::TRANSMIT_DONE,
        "and it went within a countdown of the read of START"
    );
    let sent: Vec<u64> = i
        .ether()
        .unwrap()
        .log
        .iter()
        .filter_map(|ev| if let Event::Sent(at, _, _) = ev { Some(*at) } else { None })
        .collect();
    assert_eq!(sent.len(), 1, "on the cable, once: {sent:?}");
}

/// **A reset in the middle of a transmission is noticed too.** Reset takes
/// the packet away from the turn timer and clears the pending Transmit
/// Done, and nothing may reach the cable after it.
#[test]
fn a_reset_takes_a_pending_transmission_away() {
    let mut e = Ether::new();
    e.keep_log(true);
    e.attach(Box::new(Capture::new(0o3040)));
    let mut i = Interface::new(0o1440, Some(e), 0, false);
    for w in packet(0o3040, 3) {
        i.write(WRITE_BUFFER, w, 1_000);
    }
    i.read(START, 1_000);
    i.write(CSR, csr::RESET, 2_000);
    i.advance(60_000_000);
    let sent: Vec<u64> = i
        .ether()
        .unwrap()
        .log
        .iter()
        .filter_map(|ev| if let Event::Sent(at, _, _) = ev { Some(*at) } else { None })
        .collect();
    assert!(sent.is_empty(), "nothing went after the reset: {sent:?}");
}
