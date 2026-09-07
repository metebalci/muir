// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ether: the cable as a medium, and the nodes on its model side.
//!
//! AIM-628 §2.3: the transceiver "impresses [the interface's signal] onto
//! the cable as a level of about 8 volts for a 1, or 0 volts (open
//! circuit) for a 0 ... When the cable is idle it is held at 0 volts by
//! the terminations." So the cable's level is the OR of everything
//! driving it, every transceiver hears the whole cable including its own
//! transmission, and one "detects interference (another transceiver
//! transmitting at the same time as this one) and informs the
//! interface."
//!
//! One side of this ether is the netlist board, through
//! [`super::cable::OnCable`]; the other is any number of [`Node`]s ---
//! the Chaosnet server with its services, and one day a bridge to a wider Chaosnet ---
//! which see every packet and offer packets to send. The ether frames,
//! codes and decodes, and keeps the etiquette of §2.6: nothing goes out
//! while the cable is busy, and after a packet each node waits its turn.

use super::packet::{Framed, frame, unframe};
use super::wire::{self, Decoder};
use std::collections::VecDeque;

/// Something on the model side of the cable.
pub trait Node: Send {
    /// The node's network address.
    fn address(&self) -> u16;
    /// A packet taken off the cable at `now`, whoever it was for. Nodes
    /// filter by [`Framed::buffer`]'s destination as the hardware does.
    fn receive(&mut self, now: u64, packet: &Framed);
    /// A packet to put on the cable, asked when the cable is free and it
    /// is the node's turn: the words as the software would write them,
    /// cable destination last. The ether adds the source and the check
    /// word.
    fn transmit(&mut self, now: u64) -> Option<Vec<u16>>;
}

/// The time-slot of the turn-taking, AIM-628 §2.6: one count of the
/// board's `MY.TURN CLK^`, measured at 1,000 ns on the netlist board
/// (`MY.TURN^`, bit 7 of the counter it clocks, toggles every 128,000 ns).
pub const SLOT_NS: u64 = 1_000;

/// What the I/O board's turn counter is loaded with when it hears a
/// packet from `source`: page LMMYNM computes `source - me` bit-serially
/// as the source word goes by, low bit first, in the 74S287 at 0D01 ---
/// `MATCH SO FAR` the difference bit, `RS2` the borrow --- and the
/// 74LS164s at 0B18 and 0B19 shift the difference bits in as they are
/// made, so that at `SRC STB`, when the word is over, they hold bits 14
/// down to 3 of the difference, the newest first; the low byte the
/// counter takes is bits 14 to 7, bit 14 at the bottom.  The turn comes
/// that many counts and one after the cable goes idle.  Measured on the
/// netlist board for four addresses against a host at 3060: `16`, `64`,
/// `127`, `255` loaded and 17, 65, 128, 256 counts waited
/// against the netlist board.
pub fn turn_byte(source: u16, me: u16) -> u8 {
    let d = source.wrapping_sub(me);
    (0..8).map(|m| (((d >> (14 - m)) & 1) as u8) << m).sum()
}

/// One event on the ether, for the record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A packet went by, taken off the cable at this time.
    Heard(u64, Framed),
    /// A node put a packet on the cable, starting at this time.
    Sent(u64, u16, Vec<u16>),
    /// The model was transmitting while the board was: a collision.
    Collision(u64),
}

/// A behavioural interface on the board side of the cable ---
/// [`crate::chaos::board::Interface`], `rtl`'s I/O board --- which sends
/// and receives whole frames.  Its turn and its time on the cable are the
/// ether's, as a node's are; what it hears is kept for it to take.
struct Board {
    address: u16,
    /// A frame given to send, and the instant its turn timer starts it.
    pending: Option<(u64, Vec<u16>)>,
    /// When the frame being sent ends, once it has started.
    sending_until: Option<u64>,
    heard: VecDeque<(u64, Framed)>,
    /// Every frame that started on the cable, the board's own included:
    /// its first edge, its source and its nominal end, for the board's
    /// turn timer, which watches the cable as the receiver does.
    frames: VecDeque<(u64, u16, u64)>,
}

pub struct Ether {
    nodes: Vec<Box<dyn Node>>,
    board: Option<Board>,
    /// What the board's transceiver drives, as last read.
    board_tx: bool,
    /// What the model transmitter drives.
    model_tx: bool,
    /// The model transmitter's level changes still to come, in time.
    waveform: VecDeque<(u64, bool)>,
    /// A packet waiting for its turn: when it may start, and its bits.
    waiting: Option<(u64, u16, Vec<u16>)>,
    level: bool,
    decoder: Decoder,
    last_edge: Option<u64>,
    last_source: Option<u16>,
    collided: bool,
    /// Whether [`Ether::log`] is kept.
    logging: bool,
    /// The last [`LOG_CAP`] events on the cable, for a test to read back.
    pub log: Vec<Event>,
}

/// How many events [`Ether::log`] keeps; the oldest half goes when it is
/// full.  Enough for the acceptance test's load of CC over the network to
/// stay whole, and small enough that a day's traffic is not.
pub const LOG_CAP: usize = 1 << 16;

impl Ether {
    fn record(&mut self, event: Event) {
        // A normal run keeps nothing: the log holds whole packet buffers,
        // tens of megabytes over a day's traffic, and only a test reads
        // it back. It is kept when [`Ether::keep_log`] asks.
        if !self.logging {
            return;
        }
        crate::keep_recent(&mut self.log, LOG_CAP);
        self.log.push(event);
    }

    pub fn new() -> Ether {
        Ether {
            nodes: Vec::new(),
            board: None,
            board_tx: false,
            model_tx: false,
            waveform: VecDeque::new(),
            waiting: None,
            level: false,
            decoder: Decoder::new(),
            last_edge: None,
            last_source: None,
            collided: false,
            // Kept by default so a board or the machine's own ether can be
            // read back by a test as it always could; a long-running,
            // run that wants it turns it on with [`Ether::keep_log`].
            logging: false,
            log: Vec::new(),
        }
    }

    /// Whether to keep [`Ether::log`]. Off for a normal run, which reads it
    /// back never: the log holds whole packet buffers, tens of megabytes
    /// over a day's traffic.
    pub fn keep_log(&mut self, keep: bool) {
        self.logging = keep;
    }

    pub fn attach(&mut self, node: Box<dyn Node>) {
        self.nodes.push(node);
    }

    /// Puts a behavioural interface at `address` on the board side of the
    /// cable.  One at most; the netlist board drives the cable itself.
    pub fn attach_board(&mut self, address: u16) {
        self.board = Some(Board {
            address,
            pending: None,
            sending_until: None,
            heard: VecDeque::new(),
            frames: VecDeque::new(),
        });
    }

    /// The behavioural board's frame to send, cable destination last, and
    /// the instant its turn timer starts it: it goes then, if the cable
    /// is free, and [`Ether::board_sending_until`] says when it ends.
    pub fn board_send(&mut self, buffer: Vec<u16>, start: u64) {
        if let Some(b) = self.board.as_mut() {
            b.pending = Some((start, buffer));
            b.sending_until = None;
        }
    }

    /// A frame that started on the cable, oldest first: its first edge, its
    /// source and its nominal end.
    pub fn board_cable_frame(&mut self) -> Option<(u64, u16, u64)> {
        self.board.as_mut().and_then(|b| b.frames.pop_front())
    }

    /// When the behavioural board's frame on the cable ends, if one has
    /// started and not ended.
    pub fn board_sending_until(&self) -> Option<u64> {
        self.board.as_ref().and_then(|b| b.sending_until)
    }

    /// A frame the behavioural board heard, oldest first: everything on
    /// the cable but its own, for the interface to filter as the receiver
    /// does.
    pub fn board_heard(&mut self) -> Option<(u64, Framed)> {
        self.board.as_mut().and_then(|b| b.heard.pop_front())
    }

    pub fn nodes(&self) -> &[Box<dyn Node>] {
        &self.nodes
    }

    pub fn nodes_mut(&mut self) -> &mut [Box<dyn Node>] {
        &mut self.nodes
    }

    /// The cable's level: high while anything drives it.
    pub fn level(&self) -> bool {
        self.level
    }

    /// Whether the board's transceiver would report interference: the
    /// board is transmitting and so is the model.
    pub fn interference(&self) -> bool {
        self.board_tx && self.model_tx
    }

    /// Whether the cable is busy: something has been on it within the
    /// idle time.
    pub fn busy(&self, now: u64) -> bool {
        self.level || self.last_edge.is_some_and(|e| now < e + wire::IDLE_NS)
    }

    fn set_level(&mut self, now: u64, level: bool) {
        if level != self.level {
            self.level = level;
            self.last_edge = Some(now);
            self.decoder.edge(now, level);
        }
    }

    /// The board's transceiver is driving `tx` at `now`. Returns whether
    /// the cable's level changed.
    pub fn board(&mut self, now: u64, tx: bool) -> bool {
        let before = self.level;
        self.board_tx = tx;
        if tx && self.model_tx && !self.collided {
            self.collided = true;
            self.record(Event::Collision(now));
        }
        self.set_level(now, self.board_tx || self.model_tx);
        self.level != before
    }

    /// When the ether next has something of its own to do.
    pub fn next_due(&self) -> Option<u64> {
        [
            self.waveform.front().map(|&(t, _)| t),
            self.waiting.as_ref().map(|&(t, _, _)| t),
            self.board.as_ref().and_then(|b| b.pending.as_ref().map(|&(t, _)| t)),
            self.decoder.next_due(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// The time is `now`: the model transmitter's edges due are made, the
    /// decoder is given the time, a packet that ended is delivered, and a
    /// node with something to send is given the cable when it is free
    /// and its turn has come. Returns whether the cable's level changed.
    pub fn at(&mut self, now: u64) -> bool {
        let before = self.level;
        while let Some(&(t, level)) = self.waveform.front()
            && t <= now
        {
            self.waveform.pop_front();
            self.model_tx = level;
            self.set_level(t, self.board_tx || self.model_tx);
        }
        if let Some(bits) = self.decoder.at(now) {
            match unframe(&bits) {
                Ok(f) => {
                    self.last_source = Some(f.source);
                    for n in &mut self.nodes {
                        if n.address() != f.source {
                            n.receive(now, &f);
                        }
                    }
                    if let Some(b) = self.board.as_mut()
                        && b.address != f.source
                    {
                        b.heard.push_back((now, f.clone()));
                    }
                    self.record(Event::Heard(now, f));
                }
                Err(_) => {
                    // Noise, or a collision's wreckage: the hardware would
                    // fail the check and drop it too.
                }
            }
        }
        if self.waveform.is_empty() && self.model_tx {
            self.model_tx = false;
            if let Some(b) = self.board.as_mut()
                && b.sending_until.is_some_and(|t| t <= now)
            {
                b.sending_until = None;
            }
        }
        // The board's turn is its own timer's; its frame goes at the
        // instant given.  Were the cable busy then, the hardware would
        // collide; here it waits for the cable to clear.
        if self.waiting.is_none()
            && self.waveform.is_empty()
            && let Some(b) = self.board.as_mut()
            && b.pending.as_ref().is_some_and(|&(t, _)| t <= now)
        {
            let (start, buffer) = b.pending.take().unwrap();
            self.waiting = Some((start, b.address, buffer));
        }
        if self.waiting.is_none() && self.waveform.is_empty() && !self.busy(now) {
            for k in 0..self.nodes.len() {
                if let Some(buffer) = self.nodes[k].transmit(now) {
                    let me = self.nodes[k].address();
                    let start = now.max(self.turn(now, me));
                    self.waiting = Some((start, me, buffer));
                    break;
                }
            }
        }
        if let Some((start, me, buffer)) = self.waiting.take() {
            if now >= start && !self.busy(now) {
                let bits = frame(&buffer, me);
                self.record(Event::Sent(now, me, buffer));
                self.collided = false;
                for (offset, level) in wire::encode(&bits) {
                    self.waveform.push_back((now + offset, level));
                }
                // The last change brings the line low; then it is idle.
                let end = now + bits.len() as u64 * wire::CELL_NS;
                self.waveform.push_back((end, false));
                if let Some(b) = self.board.as_mut() {
                    b.frames.push_back((now, me, end));
                    if b.address == me {
                        b.sending_until = Some(end);
                    }
                }
            } else if self.busy(now) {
                // Someone took the cable first: wait for it to clear.
                self.waiting = Some((start.max(now + 1), me, buffer));
            } else {
                self.waiting = Some((start, me, buffer));
            }
        }
        self.level != before
    }

    /// When a node `me` may next transmit: after the cable has been idle,
    /// its turn comes [`turn_byte`] counts and one later, as the board's
    /// timer would have it --- the nodes stand in for interfaces built to
    /// the same rule, without a timer's phase of their own.
    fn turn(&self, now: u64, me: u16) -> u64 {
        let idle_at = self.last_edge.map_or(now, |e| e + wire::IDLE_NS);
        let slots = turn_byte(self.last_source.unwrap_or(me), me) as u64 + 1;
        idle_at.max(now) + slots * SLOT_NS
    }
}

impl Default for Ether {
    fn default() -> Self {
        Ether::new()
    }
}

/// A node that keeps what it hears and sends what it is given: the
/// harness's end of the cable.
#[derive(Default)]
pub struct Capture {
    pub address: u16,
    pub heard: Vec<(u64, Framed)>,
    pub to_send: VecDeque<Vec<u16>>,
}

impl Capture {
    pub fn new(address: u16) -> Capture {
        Capture { address, ..Default::default() }
    }
}

impl Node for Capture {
    fn address(&self) -> u16 {
        self.address
    }
    fn receive(&mut self, now: u64, packet: &Framed) {
        self.heard.push((now, packet.clone()));
    }
    fn transmit(&mut self, _now: u64) -> Option<Vec<u16>> {
        self.to_send.pop_front()
    }
}
