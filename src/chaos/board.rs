// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Chaosnet interface as a behaviour, for the engines whose I/O board
//! is `rtl`'s model: the registers of [`super::interface`] over an
//! [`Ether`], so that the Chaosnet server and its FILE service answer `rtl` as
//! they answer the netlist board.  The netlist board is the authority:
//! `tests/chaos_rtl.rs` runs the same register sequences on both, and
//! holds the instants.
//!
//! What the software does with it is microcode 323's `uc-chaos.lisp`, the
//! Unibus interrupt at vector 270: with Receive Done up it reads the bit
//! count, the packet's words out of the read buffer, the CSR again, and
//! writes the CSR with Clear Receiver; to transmit it writes the words and
//! reads 764152; it dismisses by writing the two interrupt enables.
//!
//! When a frame goes out is the **turn timer's**, page LMTURN of the I/O
//! board: a twelve-bit counter of 74LS193s counting down on a microsecond
//! clock that runs only while the cable is idle, loaded with the address
//! difference of every packet heard ([`turn_byte`]), and the transmitter
//! starting on the rising edge of its bit 7 --- `Turn`, with every
//! interval measured on the netlist board.

use super::ether::{Ether, turn_byte};
use super::interface::{self, csr};
use super::packet::{Framed, check_word, frame};
use super::wire;
use std::collections::VecDeque;

/// The Unibus interrupt vector, `uc-interrupt.lisp`: "JUMP-EQUAL M-B
/// (A-CONSTANT 270) CHAOS-INTR --- Chaos net has special handler".
pub const VECTOR: u16 = 0o270;

/// The bits of the CSR the software writes and reads back.
const WRITABLE: u16 = csr::TIMER_INT_ENABLE
    | csr::LOOP_BACK
    | csr::SPY
    | csr::RECEIVE_INT_ENABLE
    | csr::TRANSMIT_INT_ENABLE;

/// How often the cable is looked at while nothing is known to be due: a
/// node with a frame to send is asked only then, so this bounds the extra
/// delay before a host's answer starts, well under the board's own.
const POLL_NS: u64 = 10_000;

// --- The turn timer, page LMTURN, as the netlist board runs it -----------

/// The divider at LMTURN 0A16, a 74LS161 on `FCLK/2^` reloading 14 from
/// its own terminal count: a terminal count every 500 ns.  The 74S112 at
/// 0D14 toggles on each while the cable is idle (`J` high, `K` on
/// `-CBLBSY`), and is held set while the cable is busy and by the load, so
/// `MY.TURN CLK^` rises at every second terminal count of an idle stretch
/// and the 74LS193s count down on it.
pub const TURN_TC_NS: u64 = 500;

/// From the I/O board's power-on to its first terminal count, the divider
/// counting from its power-on state.  Measured on the netlist board:
/// `MY.TURN CLK^` rises at 4,250 ns and every 1,000 from there while the
/// cable is idle, and `MY.TURN^`, bit 7 of the counter, falls at 132,250,
/// the 129th count.
pub const TURN_FIRST_TC_NS: u64 = 4_250;

/// From a frame's first edge on the cable to `-LOAD.MY.TURN`: `SRC STB`
/// at the 33rd bit cell, the destination and the source words received.
/// Measured, 8,250.
pub const TURN_LOAD_NS: u64 = 33 * wire::CELL_NS;

/// From the count that raises bit 7 to the frame's first edge on the
/// cable, `TSTART` through the transmit clock.  Measured, 470.
pub const TURN_START_NS: u64 = 470;

/// From a frame's nominal end --- its first edge plus a cell a bit --- to
/// `-CBLBSY` lifting: the last edge 120 on, the busy one-shot 500 more.
/// Measured.  `RDONE` rises with it.
pub const CBLBSY_OFF_NS: u64 = 620;

/// Transmit Done comes this much before the frame's nominal end: `-TDONE`
/// 370 ns before the last edge, which is 120 after the nominal end.
/// Measured.
pub const TDONE_BEFORE_END_NS: u64 = 250;

/// From the read of START to `TSREMPTY`, the source and check words
/// shifted into the buffer behind the packet.  Measured, 6,350, for
/// packets of one to thirty words alike.
pub const TSR_READY_NS: u64 = 6_350;

/// The turn timer.  A terminal count at a time: the counter is loaded
/// with [`turn_byte`] of every frame's source 33 cells into the frame,
/// held while the cable is busy, counted down on every second terminal
/// count while it is idle, and a frame ready starts [`TURN_START_NS`]
/// after the count that carries the low byte from 0 to 255 --- bit 7's
/// rise, the 74S74 at 0B15 clocking `TSTART` from `TSREMPTY AND CW AND
/// -CBLBSY`.  Only the low byte matters to bit 7, so only it is kept.
#[derive(Clone, Debug)]
struct Turn {
    powered_at: u64,
    /// Terminal counts taken; the next is at
    /// `powered_at + TURN_FIRST_TC_NS + TURN_TC_NS * taken`.
    taken: u64,
    /// `MY.TURN CLK^`.
    q: bool,
    /// The counter's low byte.
    low: u8,
    /// Frames seen starting on the cable, this interface's own included:
    /// first edge, source, nominal end.  Kept while they can matter.
    frames: Vec<(u64, u16, u64)>,
    /// A packet started: `TSREMPTY` at, and its words.
    ready: Option<(u64, Vec<u16>)>,
}

impl Turn {
    fn new(powered_at: u64) -> Turn {
        Turn { powered_at, taken: 0, q: false, low: 0, frames: Vec::new(), ready: None }
    }

    fn next_tc(&self) -> u64 {
        self.powered_at + TURN_FIRST_TC_NS + TURN_TC_NS * self.taken
    }

    /// `-CBLBSY` down at `t`: a frame's first edge is past and its busy
    /// one-shot has not run out.
    fn busy_at(&self, t: u64) -> bool {
        self.frames.iter().any(|&(s, _, e)| s <= t && t < e + CBLBSY_OFF_NS)
    }

    /// Takes the terminal count at `t`, for the interface at `me`.  Returns
    /// the instant a frame starts, if this count starts one.
    fn tc(&mut self, t: u64, me: u16) -> Option<u64> {
        let prev =
            self.taken.checked_sub(1).map(|k| self.powered_at + TURN_FIRST_TC_NS + TURN_TC_NS * k);
        for &(s, source, _) in &self.frames {
            let load = s + TURN_LOAD_NS;
            if load <= t && prev.is_none_or(|p| load > p) {
                self.low = turn_byte(source, me);
                self.q = true;
            }
        }
        let mut starts = None;
        if self.busy_at(t) {
            self.q = true;
        } else {
            self.q = !self.q;
            if self.q {
                let rises = self.low == 0;
                self.low = self.low.wrapping_sub(1);
                if rises && self.ready.as_ref().is_some_and(|&(at, _)| at <= t) {
                    starts = Some(t + TURN_START_NS);
                }
            }
        }
        self.taken += 1;
        self.frames.retain(|&(_, _, e)| e + CBLBSY_OFF_NS >= t);
        starts
    }
}

pub struct Interface {
    /// The switches: [`interface::MY_ADDRESS`].
    address: u16,
    /// The read/write bits, [`WRITABLE`].
    csr: u16,
    transmit_done: bool,
    transmit_abort: bool,
    receive_done: bool,
    crc_error: bool,
    /// Packets that came while the buffer was full, four bits.
    lost: u8,
    /// The outgoing buffer, as written, destination last.
    xmit: Vec<u16>,
    /// The incoming buffer --- the words as written, destination, source,
    /// check --- and how far it has been read.
    rcv: Vec<u16>,
    rcv_at: usize,
    /// When Transmit Done comes for the frame being sent.
    tdone_at: Option<u64>,
    /// Frames off the cable, or looped back, each due at its `RDONE`.
    incoming: VecDeque<(u64, Framed)>,
    turn: Turn,
    /// When the cable was last looked at.
    polled: u64,
    /// The cable, with the Chaosnet server on it; none, and a frame sent goes
    /// nowhere and nothing ever comes.
    ether: Option<Box<Ether>>,
    /// `--chaos-trace`: say what the receiver takes and drops, and when
    /// the transmitter's turn comes.
    trace: bool,
}

impl Clone for Interface {
    /// The registers clone; the cable does not.  A copy of a machine has
    /// an interface with nothing on its cable.
    fn clone(&self) -> Interface {
        Interface {
            address: self.address,
            csr: self.csr,
            transmit_done: self.transmit_done,
            transmit_abort: self.transmit_abort,
            receive_done: self.receive_done,
            crc_error: self.crc_error,
            lost: self.lost,
            xmit: self.xmit.clone(),
            rcv: self.rcv.clone(),
            rcv_at: self.rcv_at,
            tdone_at: self.tdone_at,
            incoming: self.incoming.clone(),
            turn: self.turn.clone(),
            polled: self.polled,
            ether: None,
            trace: self.trace,
        }
    }
}

impl Interface {
    /// An interface with its switches at `address`, on `ether` if given,
    /// powered at `powered_at` --- where its turn timer's divider starts
    /// --- and as at power-up: the read/write bits zero, Transmit Done up
    /// so that the first transmission may start --- `CHAOS-XMT-INTR` waits
    /// for it --- and the receiver ready for a packet, as the netlist
    /// board's is after a reset alone (`tests/chaos_rtl.rs`): the driver's
    /// `INTERFACE-RESET-AND-ENABLE` writes Reset and the interrupt enables
    /// and nothing else.
    pub fn new(address: u16, ether: Option<Ether>, powered_at: u64, trace: bool) -> Interface {
        let mut ether = ether.map(Box::new);
        if let Some(e) = ether.as_mut() {
            e.attach_board(address);
        }
        Interface {
            address,
            csr: 0,
            transmit_done: true,
            transmit_abort: false,
            receive_done: false,
            crc_error: false,
            lost: 0,
            xmit: Vec::new(),
            rcv: Vec::new(),
            rcv_at: 0,
            tdone_at: None,
            incoming: VecDeque::new(),
            turn: Turn::new(powered_at),
            polled: powered_at,
            ether,
            trace,
        }
    }

    pub fn address(&self) -> u16 {
        self.address
    }

    pub fn ether(&self) -> Option<&Ether> {
        self.ether.as_deref()
    }

    /// The cable, to be configured: [`Ether::keep_log`] is off by default,
    /// and a test that reads the log turns it on through here.
    pub fn ether_mut(&mut self) -> Option<&mut Ether> {
        self.ether.as_deref_mut()
    }

    /// Plugs `ether` into this interface's cable, in place of whatever was
    /// there.
    pub fn plug(&mut self, mut ether: Ether) {
        ether.attach_board(self.address);
        self.ether = Some(Box::new(ether));
    }

    /// The CSR as read: AIM-628 §7's bits.
    pub fn csr(&self) -> u16 {
        self.csr
            | if self.transmit_abort { csr::TRANSMIT_ABORT } else { 0 }
            | if self.transmit_done { csr::TRANSMIT_DONE } else { 0 }
            | ((self.lost as u16) << 9) & csr::LOST_COUNT
            | if self.crc_error { csr::CRC_ERROR } else { 0 }
            | if self.receive_done { csr::RECEIVE_DONE } else { 0 }
    }

    /// "The number of bits in the incoming packet buffer, minus one.
    /// After the whole packet has been read out, it will contain 7777."
    /// Before any packet, and after a Reset or a Clear Receiver, the
    /// netlist board reads 0 (`tests/cadrio_netlist.rs`).
    pub fn bit_count(&self) -> u16 {
        if !self.receive_done {
            0
        } else if self.rcv_at < self.rcv.len() {
            (self.rcv.len() * 16 - 1) as u16
        } else {
            0o7777
        }
    }

    /// The Unibus interrupt this interface is requesting, if it is.
    pub fn interrupt_request(&self) -> Option<u16> {
        let receive = self.receive_done && self.csr & csr::RECEIVE_INT_ENABLE != 0;
        let transmit = self.transmit_done && self.csr & csr::TRANSMIT_INT_ENABLE != 0;
        (receive || transmit).then_some(VECTOR)
    }

    /// Time passes to `now`, an event at a time in their order: the
    /// cable's, the turn timer's terminal counts, frames landing, Transmit
    /// Done; then a look at the cable when one is owed.
    pub fn advance(&mut self, now: u64) {
        loop {
            let tc = self.turn.next_tc();
            let cable = self.ether.as_ref().and_then(|e| e.next_due());
            let landing = self.incoming.front().map(|&(t, _)| t);
            let due = [Some(tc), cable, landing, self.tdone_at]
                .into_iter()
                .flatten()
                .filter(|&t| t <= now)
                .min();
            let Some(t) = due else { break };
            if cable == Some(t)
                && let Some(e) = self.ether.as_mut()
            {
                e.at(t);
                self.take_from_cable();
            }
            if landing == Some(t) {
                let (_, f) = self.incoming.pop_front().unwrap();
                self.arrive(t, &f);
            }
            if self.tdone_at == Some(t) {
                self.tdone_at = None;
                self.transmit_done = true;
            }
            if tc == t
                && let Some(start) = self.turn.tc(t, self.address)
            {
                self.launch(start, t);
            }
        }
        if let Some(e) = self.ether.as_mut()
            && now >= self.polled + POLL_NS
        {
            e.at(now);
            self.polled = now;
            self.take_from_cable();
        }
    }

    /// What the cable has for this interface: frames that started, for the
    /// turn timer and for Transmit Done of its own; frames decoded, to land
    /// at their `RDONE`.  Under Loop Back the receiver is on the
    /// transmitter, not the cable.
    fn take_from_cable(&mut self) {
        let Some(e) = self.ether.as_mut() else { return };
        let loop_back = self.csr & csr::LOOP_BACK != 0;
        while let Some((start, source, end)) = e.board_cable_frame() {
            if source == self.address {
                self.tdone_at = Some(end - TDONE_BEFORE_END_NS);
                if self.trace {
                    eprintln!(
                        "chaos {start:>6}: interface {:o} put its frame on the cable, to {end}",
                        self.address
                    );
                }
            }
            if !loop_back || source == self.address {
                self.turn.frames.push((start, source, end));
            }
        }
        while let Some((at, f)) = e.board_heard() {
            if loop_back {
                continue;
            }
            // `RDONE` with `-CBLBSY` lifting, after the frame's end.
            let end = self
                .turn
                .frames
                .iter()
                .filter(|&&(s, source, _)| source == f.source && s <= at)
                .map(|&(_, _, e)| e)
                .max();
            let land = end.map_or(at, |e| e + CBLBSY_OFF_NS);
            self.incoming.push_back((land, f));
            self.incoming.make_contiguous().sort_by_key(|&(t, _)| t);
        }
    }

    /// The turn has come at terminal count `t`: the packet ready goes out
    /// from `start`, on the cable, or back to this receiver under Loop
    /// Back, or into the void with no cable.
    fn launch(&mut self, start: u64, t: u64) {
        let Some((ready_at, buffer)) = self.turn.ready.take() else { return };
        let bits = frame(&buffer, self.address).len() as u64;
        let end = start + bits * wire::CELL_NS;
        if self.trace {
            eprintln!(
                "chaos {t:>6}: interface {:o}'s turn, its frame ready since {ready_at}, {bits} bits from {start}",
                self.address
            );
        }
        if self.csr & csr::LOOP_BACK != 0 || self.ether.is_none() {
            self.turn.frames.push((start, self.address, end));
            self.tdone_at = Some(end - TDONE_BEFORE_END_NS);
            if self.csr & csr::LOOP_BACK != 0 {
                let mut over = buffer.clone();
                over.push(self.address);
                let check = check_word(&over);
                let f = Framed { buffer, source: self.address, check, check_ok: true };
                self.incoming.push_back((end + CBLBSY_OFF_NS, f));
                self.incoming.make_contiguous().sort_by_key(|&(t, _)| t);
            }
        } else if let Some(e) = self.ether.as_mut() {
            e.board_send(buffer, start);
        }
    }

    /// A frame off the cable: taken if it is for this interface --- its
    /// address, a broadcast, or anything under Spy --- and the buffer is
    /// free; counted as lost if the buffer was full.
    fn arrive(&mut self, now: u64, f: &Framed) {
        let dest = f.buffer.last().copied().unwrap_or(0);
        let mine = dest == self.address || dest == 0 || self.csr & csr::SPY != 0;
        if !mine {
            return;
        }
        if self.receive_done {
            self.lost = (self.lost + 1).min(15);
            if self.trace {
                eprintln!(
                    "chaos {now:>6}: interface {:o} lost a frame from {:o}, its buffer full ({} lost)",
                    self.address, f.source, self.lost
                );
            }
            return;
        }
        if self.trace {
            eprintln!(
                "chaos {now:>6}: interface {:o} took a frame from {:o}, {} words{}",
                self.address,
                f.source,
                f.buffer.len(),
                if self.csr & csr::RECEIVE_INT_ENABLE != 0 {
                    ", interrupting"
                } else {
                    ", no interrupt enabled"
                }
            );
        }
        self.rcv = f.buffer.clone();
        self.rcv.push(f.source);
        self.rcv.push(f.check);
        self.rcv_at = 0;
        self.crc_error = !f.check_ok;
        self.receive_done = true;
    }

    /// A read of one of the registers at `now`.
    pub fn read(&mut self, uaddr: u32, now: u64) -> u16 {
        self.advance(now);
        match uaddr {
            interface::CSR => self.csr(),
            interface::MY_ADDRESS => self.address,
            interface::READ_BUFFER => {
                let w = self.rcv.get(self.rcv_at).copied().unwrap_or(0);
                if self.rcv_at < self.rcv.len() {
                    self.rcv_at += 1;
                }
                w
            }
            interface::BIT_COUNT => self.bit_count(),
            interface::START => {
                self.start(now);
                self.address
            }
            _ => 0o177777,
        }
    }

    /// A write of one of the registers at `now`.
    pub fn write(&mut self, uaddr: u32, v: u16, now: u64) {
        self.advance(now);
        match uaddr {
            interface::CSR => {
                self.csr = v & WRITABLE;
                if v & csr::RESET != 0 {
                    self.reset();
                }
                if v & csr::CLEAR_RECEIVER != 0 {
                    self.receive_done = false;
                    self.crc_error = false;
                    self.rcv.clear();
                    self.rcv_at = 0;
                    self.lost = 0;
                }
                if v & csr::CLEAR_TRANSMITTER != 0 {
                    self.xmit.clear();
                    self.turn.ready = None;
                    self.tdone_at = None;
                    self.transmit_abort = false;
                    self.transmit_done = true;
                }
            }
            interface::WRITE_BUFFER => {
                // The outgoing buffer is the 2147 at LMTBUF 0C10, 4K by 1,
                // addressed by `TBCT<11:0>` from the three 25LS193s at
                // 0C11-0C13: 4,096 bits, 256 sixteen-bit words, and a word
                // past that has nowhere to go.  The incoming buffer is its
                // twin, the 2147 at LMRBUF 0C04 on `RBCT<11:0>`.
                if self.xmit.len() < 256 {
                    self.xmit.push(v);
                }
                self.transmit_done = false;
                self.transmit_abort = false;
            }
            _ => {}
        }
    }

    /// Reset (write only): "completely resets the interface, just as at
    /// power up and Unibus Initialize."  The turn timer is not in it: the
    /// counter's clear is grounded and the divider runs free.
    pub fn reset(&mut self) {
        self.csr = 0;
        self.transmit_done = true;
        self.transmit_abort = false;
        self.receive_done = false;
        self.crc_error = false;
        self.lost = 0;
        self.xmit.clear();
        self.rcv.clear();
        self.rcv_at = 0;
        self.tdone_at = None;
        self.incoming.clear();
        self.turn.ready = None;
    }

    /// A read of [`interface::START`]: the buffer, with this address as
    /// its source, is ready to go [`TSR_READY_NS`] on, and goes at the
    /// turn timer's next rising edge of bit 7 with the cable idle
    /// ([`Turn`]).
    fn start(&mut self, now: u64) {
        let buffer = std::mem::take(&mut self.xmit);
        self.turn.ready = Some((now + TSR_READY_NS, buffer));
    }
}

// --- Checkpoints ------------------------------------------------------------

impl Interface {
    /// The interface into a checkpoint: its switches and registers, both
    /// buffers, the frames due off the cable and the turn timer.  Not the
    /// cable, which a resume plugs in afresh with the Chaosnet server on
    /// it, and not the trace switch.
    pub fn save(&self, w: &mut crate::checkpoint::Writer) {
        let Interface {
            address,
            csr,
            transmit_done,
            transmit_abort,
            receive_done,
            crc_error,
            lost,
            xmit,
            rcv,
            rcv_at,
            tdone_at,
            incoming,
            turn,
            polled,
            ether: _,
            trace: _,
        } = self;
        w.u16(*address);
        w.u16(*csr);
        w.bool(*transmit_done);
        w.bool(*transmit_abort);
        w.bool(*receive_done);
        w.bool(*crc_error);
        w.u8(*lost);
        w.u16s(xmit);
        w.u16s(rcv);
        w.u64(*rcv_at as u64);
        w.opt(*tdone_at, crate::checkpoint::Writer::u64);
        w.u64(incoming.len() as u64);
        for (at, f) in incoming {
            w.u64(*at);
            w.u16s(&f.buffer);
            w.u16(f.source);
            w.u16(f.check);
            w.bool(f.check_ok);
        }
        turn.save(w);
        w.u64(*polled);
    }

    /// Back from a checkpoint, into an interface plugged in at the same
    /// switches.
    pub fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        let address = r.u16()?;
        if address != self.address {
            return Err(crate::checkpoint::bad(format!(
                "the Chaosnet interface's switches read {address:o}, this machine's {:o}",
                self.address
            )));
        }
        self.csr = r.u16()?;
        self.transmit_done = r.bool()?;
        self.transmit_abort = r.bool()?;
        self.receive_done = r.bool()?;
        self.crc_error = r.bool()?;
        self.lost = r.u8()?;
        self.xmit = r.u16s()?;
        self.rcv = r.u16s()?;
        self.rcv_at = r.u64()? as usize;
        self.tdone_at = r.opt(crate::checkpoint::Reader::u64)?;
        let n = r.u64()?;
        let mut incoming = VecDeque::new();
        for _ in 0..n {
            let at = r.u64()?;
            let buffer = r.u16s()?;
            let source = r.u16()?;
            let check = r.u16()?;
            let check_ok = r.bool()?;
            incoming.push_back((at, Framed { buffer, source, check, check_ok }));
        }
        self.incoming = incoming;
        self.turn.load(r)?;
        self.polled = r.u64()?;
        Ok(())
    }
}

impl Turn {
    fn save(&self, w: &mut crate::checkpoint::Writer) {
        let Turn { powered_at, taken, q, low, frames, ready } = self;
        w.u64(*powered_at);
        w.u64(*taken);
        w.bool(*q);
        w.u8(*low);
        w.u64(frames.len() as u64);
        for (a, b, c) in frames {
            w.u64(*a);
            w.u16(*b);
            w.u64(*c);
        }
        w.bool(ready.is_some());
        if let Some((at, words)) = ready {
            w.u64(*at);
            w.u16s(words);
        }
    }

    fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        self.powered_at = r.u64()?;
        self.taken = r.u64()?;
        self.q = r.bool()?;
        self.low = r.u8()?;
        let n = r.u64()?;
        self.frames =
            (0..n).map(|_| Ok((r.u64()?, r.u16()?, r.u64()?))).collect::<std::io::Result<_>>()?;
        self.ready = if r.bool()? { Some((r.u64()?, r.u16s()?)) } else { None };
        Ok(())
    }
}
