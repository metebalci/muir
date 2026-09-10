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
//! the CHUDP link that carries the hosts off this machine, and whatever a
//! caller in this process puts there ---
//! which see every packet and offer packets to send. The ether frames,
//! codes and decodes, and keeps the etiquette of §2.6: a node takes its
//! turn after a packet, and goes if the cable is idle then. A transmitter
//! that has committed --- the board at its turn timer's instant, a
//! transmitter that has not heard the cable --- goes whether or not the
//! cable is busy, and two driving high at once is §2.3's interference: a
//! transmitter whose clock edge finds it aborts and holds the cable high
//! for [`ABORT_HOLD_NS`], the abort signal, and what was left on the
//! cable reaches every receiver as wreckage, failing its check.

use super::packet::{Framed, Received, frame, unframe_any};
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
    /// The node's frame stopped at `now`, aborted, its transceiver having
    /// heard another: the words as given to [`Node::transmit`], back, to
    /// offer again at the next turn or not. An interface's driver retries
    /// on Transmit Abort; the default forgets the frame, which a host
    /// whose transport retransmits can afford.
    fn aborted(&mut self, _now: u64, _buffer: Vec<u16>) {}
}

/// The time-slot of the turn-taking, AIM-628 §2.6: one count of the
/// board's `MY.TURN CLK^`, measured at 1,000 ns on the netlist board
/// (`MY.TURN^`, bit 7 of the counter it clocks, toggles every 128,000 ns).
pub const SLOT_NS: u64 = 1_000;

/// How often a transmitter looks for interference: one period of the
/// board's 8 MHz `FCLK^`. On the board `COLLISION` is `TBUSY` with
/// `INTERFERENCE`, the open-collector 74S02 at LMMODU 0B08, and the
/// `ABORT` flip-flop, the 74S112 at 0A09, takes it on `-FCLK^` --- so
/// interference that is not there at an edge is missed, and the
/// transmitter goes on, and collides on, until an edge finds it; `D OUT`
/// is `TTL.D.OUT AND -ABORT` at the same 0B08, so the driver is off from
/// the edge that does. A model transmitter samples its transceiver the
/// same way, at this period from its frame's first edge, and stops at
/// the first sample that finds interference. `tests/chaos_netlist.rs`
/// watches the board's `ABORT` follow its transceiver's interference:
/// 75 ns after it, the next edge.
pub const ABORT_NS: u64 = 125;

/// How long `ABORT` stands, from the edge that sets it to `ABORTDN`, and
/// the driver holds the cable high throughout: AIM-628 §2.5's abort
/// signal, "if the ether remains high for about two bit cells". Measured
/// on the netlist board, `tests/chaos_netlist.rs`: the driver on from
/// `ABORT` to 1,000 ns after it, four cells. A model transmitter aborted
/// holds the cable as long.
pub const ABORT_HOLD_NS: u64 = 1_000;

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
    /// Two transceivers drove high at once at this time, for the first
    /// time since the cable was idle: interference, AIM-628 §2.3, which
    /// every transmitter high at one of its own clock edges stops on.
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
    /// What the board heard, as a receiver takes it.
    heard: VecDeque<(u64, Received)>,
    /// Every frame that started on the cable, the board's own included:
    /// its first edge, its source and its nominal end, for the board's
    /// turn timer, which watches the cable as the receiver does.
    frames: VecDeque<(u64, u16, u64)>,
    /// Transmitters on the cable that aborted, oldest first: when, whose
    /// frame, and when its abort signal ends and the cable is let go.
    collisions: VecDeque<(u64, u16, u64)>,
}

/// A frame a model transmitter has on the cable.
struct Transmission {
    source: u16,
    /// The words as the node gave them, to hand back if it is aborted.
    buffer: Vec<u16>,
    /// The level changes still to come, in time.
    waveform: VecDeque<(u64, bool)>,
    /// What the transmitter drives now.
    level: bool,
    /// The transmitter's next clock edge, [`ABORT_NS`] apart from the
    /// frame's first: where it looks for interference.
    tick: u64,
    /// A transmitter with no interference detector: it sends the frame
    /// whole whatever is on the cable. [`Ether::send_now`]'s.
    deaf: bool,
}

/// A frame waiting to start.
struct Waiting {
    /// When it may start.
    at: u64,
    source: u16,
    buffer: Vec<u16>,
    /// Whether `at` is the transmitter's own instant --- the board's turn
    /// timer's, or a transmitter's that has not heard the cable --- which
    /// goes then, busy or not, rather than a turn, which goes then if the
    /// cable is idle.
    committed: bool,
    deaf: bool,
}

pub struct Ether {
    nodes: Vec<Box<dyn Node>>,
    board: Option<Board>,
    /// What the board's transceiver drives, as last read.
    board_tx: bool,
    /// The model transmitters' frames on the cable, any number at once.
    sending: Vec<Transmission>,
    waiting: Vec<Waiting>,
    level: bool,
    decoder: Decoder,
    last_edge: Option<u64>,
    last_source: Option<u16>,
    /// Whether two transceivers have driven high at once since the cable
    /// was last idle: one [`Event::Collision`] an overlap.
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
            sending: Vec::new(),
            waiting: Vec::new(),
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
            collisions: VecDeque::new(),
        });
    }

    /// The behavioural board's frame to send, cable destination last, and
    /// the instant its turn timer starts it: it goes then, busy cable or
    /// not, and [`Ether::board_sending_until`] says when it ends.
    pub fn board_send(&mut self, buffer: Vec<u16>, start: u64) {
        if let Some(b) = self.board.as_mut() {
            b.pending = Some((start, buffer));
            b.sending_until = None;
        }
    }

    /// A frame put on the cable at `now`, busy or not, by a transmitter
    /// that has not heard it and has no interference detector of its own:
    /// the frame goes whole, whatever else is on the cable. It is what a
    /// test uses to try this board's detector, and what a transmitter with
    /// a length of cable between it and the rest --- a bridge to a wider
    /// Chaosnet --- looks like from here.
    pub fn send_now(&mut self, now: u64, source: u16, buffer: Vec<u16>) {
        self.waiting.push(Waiting { at: now, source, buffer, committed: true, deaf: true });
        self.at(now);
    }

    /// A frame that started on the cable, oldest first: its first edge, its
    /// source and its nominal end.
    pub fn board_cable_frame(&mut self) -> Option<(u64, u16, u64)> {
        self.board.as_mut().and_then(|b| b.frames.pop_front())
    }

    /// A transmitter on the cable that aborted, oldest first, for the
    /// behavioural board: the instant its `ABORT` set, whose frame, and
    /// when it lets the cable go, [`ABORT_HOLD_NS`] on --- the frame's
    /// end from then, in place of its nominal one. The board's own frame
    /// among them is its Transmit Abort.
    pub fn board_collision(&mut self) -> Option<(u64, u16, u64)> {
        self.board.as_mut().and_then(|b| b.collisions.pop_front())
    }

    /// When the behavioural board's frame on the cable ends, if one has
    /// started and not ended.
    pub fn board_sending_until(&self) -> Option<u64> {
        self.board.as_ref().and_then(|b| b.sending_until)
    }

    /// A frame the behavioural board heard, oldest first, as a receiver
    /// takes it: everything on the cable but its own, wreckage included,
    /// for the interface to filter as the receiver does.
    pub fn board_heard(&mut self) -> Option<(u64, Received)> {
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

    /// What the model transmitters drive: high while any of them does.
    fn model_tx(&self) -> bool {
        self.sending.iter().any(|s| s.level)
    }

    /// Whether the board's transceiver would report interference: the
    /// board is driving high and so is a model transmitter.
    pub fn interference(&self) -> bool {
        self.board_tx && self.model_tx()
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

    /// How many transceivers drive high: two or more is interference to
    /// each of them, AIM-628 §2.3.
    fn high(&self) -> usize {
        self.board_tx as usize + self.sending.iter().filter(|s| s.level).count()
    }

    /// Interference at `now`, for the record: once an overlap. The
    /// board's own detector is [`Ether::interference`]; a model
    /// transmitter's is its clock in [`Ether::play`].
    fn interfere(&mut self, now: u64) {
        if self.high() >= 2 && !self.collided {
            self.collided = true;
            self.record(Event::Collision(now));
        }
    }

    /// The board's transceiver is driving `tx` at `now`. Returns whether
    /// the cable's level changed.
    pub fn board(&mut self, now: u64, tx: bool) -> bool {
        let before = self.level;
        // What the model transmitters did up to `now`, with the board as
        // it was driving until then.
        self.play(now);
        self.board_tx = tx;
        self.interfere(now);
        self.set_level(now, self.board_tx || self.model_tx());
        self.level != before
    }

    /// Whether a model transmitter could hear interference: another
    /// transmitter is on the cable. Its clock edges matter only then.
    fn contended(&self) -> bool {
        self.board_tx || self.sending.len() >= 2
    }

    /// When the ether next has something of its own to do.
    pub fn next_due(&self) -> Option<u64> {
        let contended = self.contended();
        let edges = self.sending.iter().filter_map(|s| s.waveform.front().map(|&(t, _)| t));
        let ticks = self.sending.iter().filter_map(|s| (contended && !s.deaf).then_some(s.tick));
        let starts = self.waiting.iter().map(|w| w.at);
        let pending = self.board.as_ref().and_then(|b| b.pending.as_ref().map(|&(t, _)| t));
        edges.chain(ticks).chain(starts).chain(pending).chain(self.decoder.next_due()).min()
    }

    /// The model transmitters' edges and clock edges due by `now`, in
    /// their order, with the cable's level and any interference at each.
    /// At a transmitter's clock edge with interference standing --- it
    /// and another driving high --- it aborts, as `ABORT` aborts the
    /// board's: the frame is given up, its words handed back and the
    /// board told, and the cable held high for [`ABORT_HOLD_NS`]. A
    /// transmitter whose frame, or abort signal, is over leaves the cable.
    fn play(&mut self, now: u64) {
        loop {
            let contended = self.contended();
            let due = self
                .sending
                .iter()
                .flat_map(|s| {
                    [s.waveform.front().map(|&(t, _)| t), (contended && !s.deaf).then_some(s.tick)]
                })
                .flatten()
                .min();
            let Some(t) = due.filter(|&t| t <= now) else { break };
            // As the levels stood up to `t`: what a clock edge at `t`
            // finds.
            let high = self.high();
            let mut aborted = Vec::new();
            let mut over = Vec::new();
            for (k, s) in self.sending.iter_mut().enumerate() {
                let mut abort = false;
                while s.tick <= t {
                    if s.tick == t && contended && !s.deaf && s.level && high >= 2 {
                        abort = true;
                    }
                    s.tick += ABORT_NS;
                }
                if abort {
                    s.waveform = VecDeque::from([(t + ABORT_HOLD_NS, false)]);
                    s.level = true;
                    s.deaf = true;
                    aborted.push((s.source, std::mem::take(&mut s.buffer)));
                    continue;
                }
                while let Some(&(at, level)) = s.waveform.front()
                    && at <= t
                {
                    s.waveform.pop_front();
                    s.level = level;
                }
                if s.waveform.is_empty() {
                    over.push(k);
                }
            }
            self.interfere(t);
            self.set_level(t, self.board_tx || self.model_tx());
            for (source, buffer) in aborted {
                if let Some(b) = self.board.as_mut() {
                    b.collisions.push_back((t, source, t + ABORT_HOLD_NS));
                }
                if let Some(n) = self.nodes.iter_mut().find(|n| n.address() == source) {
                    n.aborted(t, buffer);
                }
            }
            for k in over.into_iter().rev() {
                let s = self.sending.remove(k);
                if let Some(b) = self.board.as_mut()
                    && b.address == s.source
                {
                    b.sending_until = None;
                }
            }
        }
        // A clock edge that was not looked at --- the transmitter alone on
        // the cable, nothing to hear --- is past: the next is the first
        // after `now`, on the same grid.
        for s in &mut self.sending {
            while s.tick <= now {
                s.tick += ABORT_NS;
            }
        }
    }

    /// A frame starts on the cable at `now`.
    fn start(&mut self, now: u64, me: u16, buffer: Vec<u16>, deaf: bool) {
        let bits = frame(&buffer, me);
        if self.logging {
            self.record(Event::Sent(now, me, buffer.clone()));
        }
        let mut waveform: VecDeque<(u64, bool)> =
            wire::encode(&bits).into_iter().map(|(offset, level)| (now + offset, level)).collect();
        // The last change brings the line low; then it is idle.
        let end = now + bits.len() as u64 * wire::CELL_NS;
        waveform.push_back((end, false));
        if let Some(b) = self.board.as_mut() {
            b.frames.push_back((now, me, end));
            if b.address == me {
                b.sending_until = Some(end);
            }
        }
        self.sending.push(Transmission {
            source: me,
            buffer,
            waveform,
            level: false,
            tick: now,
            deaf,
        });
    }

    /// The time is `now`: the model transmitters' edges due are made, the
    /// decoder is given the time, a packet that ended is delivered, and
    /// the frames due start --- the board's at its turn timer's instant,
    /// a node's at its turn if the cable is idle then. Returns whether the
    /// cable's level changed.
    pub fn at(&mut self, now: u64) -> bool {
        let before = self.level;
        self.play(now);
        if !self.busy(now) {
            self.collided = false;
        }
        // A run of bits over: a packet, or a collision's wreckage, which
        // every receiver takes as it takes a packet and judges by its check
        // word and bit count. The turn timers load from the source word as
        // it came, whatever the check says, as the board's does.
        if let Some(bits) = self.decoder.at(now) {
            let r = unframe_any(&bits);
            let from = r.framed.source;
            if r.bits >= 32 {
                self.last_source = Some(from);
            }
            for n in &mut self.nodes {
                if n.address() != from {
                    n.receive(now, &r.framed);
                }
            }
            if let Some(b) = self.board.as_mut()
                && b.address != from
            {
                b.heard.push_back((now, r.clone()));
            }
            self.record(Event::Heard(now, r.framed));
        }
        // The board's frame goes at its turn timer's instant, cable busy
        // or not: the timer took the cable idle at its terminal count,
        // and the transmitter is committed from there.
        if let Some(b) = self.board.as_mut()
            && b.pending.as_ref().is_some_and(|&(t, _)| t <= now)
        {
            let (at, buffer) = b.pending.take().unwrap();
            let source = b.address;
            self.waiting.push(Waiting { at, source, buffer, committed: true, deaf: false });
        }
        // A node is asked when the cable is free and no frame of the
        // nodes' is waiting or going.
        if !self.busy(now) && self.sending.is_empty() && !self.waiting.iter().any(|w| !w.committed)
        {
            for k in 0..self.nodes.len() {
                if let Some(buffer) = self.nodes[k].transmit(now) {
                    let source = self.nodes[k].address();
                    let at = now.max(self.turn(now, source));
                    self.waiting.push(Waiting {
                        at,
                        source,
                        buffer,
                        committed: false,
                        deaf: false,
                    });
                }
            }
        }
        // Frames due: a transmitter's own instant goes; a turn goes if the
        // cable is idle at it, and holds otherwise. Judged before any of
        // them starts, so frames due together start together, and
        // collide.
        let busy = self.busy(now);
        let (due, rest): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.waiting).into_iter().partition(|w| w.at <= now);
        self.waiting = rest;
        for w in due {
            if w.committed || !busy {
                self.start(now, w.source, w.buffer, w.deaf);
            } else {
                // Someone took the cable first: wait for it to clear.
                self.waiting.push(Waiting { at: now + 1, ..w });
            }
        }
        // The first edges of what just started are at `now`.
        self.play(now);
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
