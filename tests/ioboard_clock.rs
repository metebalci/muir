// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The I/O board's clock runs on between the processor's references to
//! it, on every engine that runs the machine.
//!
//! Most of the board answers for an instant rather than accumulating:
//! `clock_ready`, `usec_at`, and the serial port's two ready bits all
//! take the instant they are asked about and work it out, so they are
//! right however long it has been since anything touched them. The
//! Chaosnet interface is not like that. A frame lands, `RECEIVE DONE`
//! sets and `Transmit Done` comes only inside
//! [`muir::chaos::board::Interface::advance`], and
//! `Interface::interrupt_request` is a bare read of the two flags. So an
//! engine that does not advance the board is an engine whose Chaosnet
//! can never interrupt --- microcode 323's `CHAOS-XMT-INTR` waits for a
//! Transmit Done that will not come, and a packet on the cable is never
//! taken.
//!
//! The check is the same for both engines and is made without the
//! processor touching a Chaosnet register, because a register access
//! advances the interface itself
//! ([`muir::chaos::board::Interface::read`] and `write` both begin with
//! it) and would hide exactly the thing being tested.

use muir::chaos::interface::{self as chaos, csr};
use muir::chaos::packet::{Packet, op};
use muir::engine::Engine;
use muir::machine::Machine;
use muir::micro::Micro;
use muir::rtl::Rtl;

/// This machine's switches, and the address the frame comes from --- the
/// band's file and time host, which is off the cable and not modelled
/// here: the frame is put on the wire directly.
const ME: u16 = 0o3050;
const OTHER: u16 = 0o3060;

/// An `RFC` for the `TIME` service, addressed to `to`, as the words go on
/// the wire.  Any frame for this machine would do; this is the one the
/// band itself sends, which makes it the frame most likely to be right.
fn rfc_time(from: u16, to: u16) -> Vec<u16> {
    Packet {
        opcode: op::RFC,
        forward: 0,
        dest: to,
        dest_index: 0,
        source: from,
        source_index: 0o21,
        number: 1,
        ack: 0,
        data: b"TIME".to_vec(),
    }
    .to_buffer(to)
}

/// A machine with the boot PROM and a Chaosnet, its receiver's interrupt
/// enabled and its transmitter's not.
///
/// Only the receiver's: `Transmit Done` is up from power-on, so a machine
/// with the transmit interrupt enabled is requesting one before anything
/// has happened, and the test would pass without the cable being read at
/// all.
///
/// No pack. The boot PROM waits on a drive that never answers, which is a
/// running machine that touches no I/O board register --- what this test
/// wants, the point being what happens while the processor is doing
/// something else.
fn machine() -> Machine {
    let mut m = Machine::new();
    m.load_prom(&muir::prom::boot_prom());
    m.chaos.address = ME;
    m.plug_chaos(0);
    let ns = m.ns;
    m.ioboard.write(chaos::CSR, csr::RECEIVE_INT_ENABLE, ns);
    m
}

/// Runs `e` until the board requests an interrupt, or for `limit`
/// microcycles.  Returns the vector, if one came.
fn until_interrupt<E: Engine>(e: &mut E, limit: u64) -> Option<u16> {
    for _ in 0..limit {
        e.step().expect("the machine halted");
        let m = e.machine();
        if let Some(v) = m.ioboard.interrupt_request(m.ns) {
            return Some(v);
        }
    }
    None
}

/// The body of the test, for one engine: nothing is asking for an
/// interrupt to begin with, a frame goes on the cable, and the board must
/// come to ask for one without the processor having touched it.
fn a_frame_lands<E: Engine>(mut e: E, name: &str) {
    e.boot();
    // Whatever the boot PROM does for a while, it does not raise this.
    assert_eq!(
        until_interrupt(&mut e, 20_000),
        None,
        "{name}: something asked for an interrupt before the frame"
    );
    let m = e.machine_mut();
    let at = m.ns;
    m.ioboard.chaos.as_mut().unwrap().ether_mut().unwrap().send_now(at, OTHER, rfc_time(OTHER, ME));
    // The frame is 16 words of 16 cells at 250 ns, so about 64 us on the
    // wire; 100,000 microcycles is a millisecond and more of the machine's
    // time on either engine.
    let vector = until_interrupt(&mut e, 100_000);
    assert_eq!(
        vector,
        Some(muir::chaos::board::VECTOR),
        "{name}: the frame put on the cable at {at} ns never landed --- \
         the I/O board's clock does not run under this engine"
    );
    eprintln!("{name}: the frame landed and the board asked for {:o}", vector.unwrap());
}

/// **A frame put on the cable lands, on `rtl`.** The engine advances the
/// whole I/O board every microcycle, so this is the behaviour the other
/// engines are held to.
#[test]
fn a_frame_lands_on_rtl() {
    a_frame_lands(Rtl::new(machine()), "rtl");
}

/// **And on `micro`.** The I/O board is `Machine`'s and not the engine's,
/// so an engine that keeps the machine's clock has to run the board on it
/// as well. This is issue #27.
#[test]
fn a_frame_lands_on_micro() {
    a_frame_lands(Micro::new(machine()), "micro");
}
