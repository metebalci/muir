// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The keyboard and the mouse on the netlist I/O board's cables: the words of
//! [`super::keyboard`] clocked down the wire, as
//! [`crate::chaos::cable::OnCable`] puts the ether on the Chaosnet's.
//!
//! **The board clocks the keyboard.** The SN75118 at IOBKBD 0E30 sends `KB
//! CLK^`, the board's own 125 kHz, out as the pair `KBDCLK+`/`KBDCLK-`,
//! and takes the pair `KBDIN+`/`KBDIN-` back through its receiver as
//! `-KBDIN`, which the 74LS14 at 0A27 turns into `KBDIN`, the cable's own
//! sense. `ukbd.lisp`, "Protocol documentation": "A character is 24 bits,
//! sent low-order bit first. There is also a 'start bit' or 'request
//! signal', which is low. Bits appear on the cable in true-high form. The
//! cable is high when idle." So the keyboard has no clock of its own: it
//! watches the board's, and moves the data line once per clock.
//!
//! **The keyboard asks, and the board clocks.** The SN75118's driver
//! sends `KB CLK^` only while the board's `DATA BITS` is up --- pins 14 and
//! 15 are ANDed onto `KBDCLK+` --- so there is no clock on the cable at
//! rest. The keyboard pulls the data line low on its own; `ukbd.lisp`
//! calls that the "start bit" and, in the same breath, the "request
//! signal", and the second name is the truer one. The 74LS109 at 0C26 goes
//! busy on it, the board starts the clock, and the three 74LS164s at
//! 0A28-0A30 take a bit on each rising edge of `KB CLK^`; after the
//! twenty-fourth the clock stops and the keyboard lets the line go high.
//! So [`OnCable::apply`] drives the request without waiting for an edge,
//! and from then on moves the line on each **falling** edge of `KBDCLK+`,
//! half a clock ahead of the edge that takes the bit. `ukbd.lisp` says as
//! much from the keyboard's side: "the falling edge is the clock in the
//! central machine. The leading edge (check this) is the clock in the
//! keyboard." `tests/keyboard_cable.rs` is what holds this to the board: a
//! word sent here reads back at `764100` and `764102`.
//!
//! What idle looks like is [`crate::unibus::IDLE_KEYBOARD`], `KBDIN+`
//! high and `KBDIN-` low, which the far end holds the pair at until a
//! keyboard is plugged in and which this holds it at between words.

use std::collections::VecDeque;

use super::mouse::Encoders;
use crate::chip::Chip;
use crate::netlist::{NetId, Netlist};
use crate::part::Level;

/// The keyboard's three wires, by net on the I/O board: the clock the
/// board sends, read; the data pair, driven.
#[derive(Clone, Copy, Debug)]
pub struct Nets {
    clock: NetId,
    data_p: NetId,
    data_m: NetId,
}

impl Nets {
    /// The nets on `n`, the I/O board.
    pub fn of(n: &Netlist) -> Option<Nets> {
        Some(Nets {
            clock: n.by_name_id("KBDCLK+")?,
            data_p: n.by_name_id("KBDIN+")?,
            data_m: n.by_name_id("KBDIN-")?,
        })
    }
}

/// Where the keyboard is in a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sending {
    /// Nothing on the line; it is high.
    Idle,
    /// The request is on the line, low, and the board's clock has not
    /// started.
    Request(u32),
    /// Data bit `k` of the word is on the line.
    Bit(u32, u8),
}

/// How long the keyboard waits for another clock edge in a word before
/// taking the clock to have stopped: three of the board's 8 us periods.
const CLOCK_GONE_NS: u64 = 24_000;

/// The keyboard on the cable.
pub struct OnCable {
    nets: Nets,
    /// Words waiting to go, oldest first.
    queue: VecDeque<u32>,
    state: Sending,
    /// The clock as last seen, for its edges, and when it last moved.
    clock_was: Level,
    clock_moved: u64,
    /// What the line is held at, to drive it only on a change.
    line: Option<bool>,
    /// Words sent end to end since the keyboard was plugged in.
    pub sent: u64,
}

impl OnCable {
    /// A keyboard for the board `n`, if `n` has the cable.
    pub fn of(n: &Netlist) -> Option<OnCable> {
        Nets::of(n).map(OnCable::new)
    }

    pub fn new(nets: Nets) -> OnCable {
        OnCable {
            nets,
            queue: VecDeque::new(),
            state: Sending::Idle,
            clock_was: Level::X,
            clock_moved: 0,
            line: None,
            sent: 0,
        }
    }

    /// Queues a word to send, if there is room: the keyboard's firmware
    /// has a shift register and a bit map and no queue, but a viewer's
    /// keys arrive in bursts and each has to reach the machine in order,
    /// so up to [`super::keyboard::BACKLOG`] words wait here.  Beyond that
    /// the word is refused --- `false` --- and the caller keeps it to offer
    /// again, as `muir` offers [`super::keyboard::Keyboard`]'s next word
    /// and takes it off that queue only once the cable has it.
    pub fn send(&mut self, word: u32) -> bool {
        if self.queue.len() >= super::keyboard::BACKLOG {
            return false;
        }
        self.queue.push_back(word);
        true
    }

    /// Whether a word is on the line or waiting.
    pub fn busy(&self) -> bool {
        self.state != Sending::Idle || !self.queue.is_empty()
    }

    /// Holds the data pair at `high`: the cable's idle and its ones are
    /// high, its start bit and its zeros low.
    fn drive(&mut self, board: &mut Chip, high: bool) -> bool {
        if self.line == Some(high) {
            return false;
        }
        let lv = |on: bool| if on { Level::High } else { Level::Low };
        board.drive(self.nets.data_p, lv(high));
        board.drive(self.nets.data_m, lv(!high));
        self.line = Some(high);
        true
    }

    /// Puts a queued word's request on the line, and on each falling edge
    /// of the board's clock moves the line to the next bit. Returns whether
    /// a net moved, in which case the board was transitioned at `now`.
    pub fn apply(&mut self, board: &mut Chip, now: u64) -> bool {
        let clock = board.net(self.nets.clock);
        let falling = self.clock_was == Level::High && clock == Level::Low;
        if clock != self.clock_was {
            self.clock_moved = now;
        }
        self.clock_was = clock;
        let (next, high) = match self.state {
            // Plugged in, or between words: the line idle, and the next
            // word's request when there is one.
            Sending::Idle => match self.queue.front().copied() {
                Some(word) if self.line == Some(true) => {
                    self.queue.pop_front();
                    (Sending::Request(word), false)
                }
                _ => (Sending::Idle, true),
            },
            Sending::Request(word) if falling => (Sending::Bit(word, 0), word & 1 != 0),
            Sending::Bit(word, k) if falling && k + 1 < 24 => {
                (Sending::Bit(word, k + 1), word >> (k + 1) & 1 != 0)
            }
            // The last bit has been taken: the clock stops, and the line
            // goes back up. Seen either as the edge after the last bit or
            // as no edge at all for a while.
            Sending::Bit(_, 23)
                if falling || now.saturating_sub(self.clock_moved) > CLOCK_GONE_NS =>
            {
                self.sent += 1;
                (Sending::Idle, true)
            }
            other => (other, self.line.unwrap_or(true)),
        };
        self.state = next;
        let moved = self.drive(board, high);
        if moved {
            board.transition(now);
        }
        moved
    }
}

/// The mouse's seven wires on the I/O board, driven: the two quadrature
/// pairs and the three switches. Each goes through an inverting Schmitt
/// trigger, the 74LS14s at IOBMSE 0A25 and 0A27 --- `*HORA` to `HORA` ---
/// and the board latches the seven on `KB CLK^` into `NEW`, the previous
/// `NEW` into `OLD`, and compares the two through the 25LS2521 at 0A21 for
/// `MOUSE STATUS CHANGE`. On IOBMS2, `OLD HORA xor NEW HORB` is the
/// direction and `(NEW HORA xor OLD HORB) xor` that the enable, into the
/// 74LS569s: one count on each step of the pair, the way a quadrature
/// encoder is read.
#[derive(Clone, Copy, Debug)]
pub struct MouseNets {
    hora: NetId,
    horb: NetId,
    vera: NetId,
    verb: NetId,
    /// The switches, in the register's order: tail, middle, head.
    switches: [NetId; 3],
}

impl MouseNets {
    pub fn of(n: &Netlist) -> Option<MouseNets> {
        Some(MouseNets {
            hora: n.by_name_id("*HORA")?,
            horb: n.by_name_id("*HORB")?,
            vera: n.by_name_id("*VERA")?,
            verb: n.by_name_id("*VERB")?,
            switches: [n.by_name_id("*TAILSW")?, n.by_name_id("*MIDSW")?, n.by_name_id("*HEADSW")?],
        })
    }
}

/// The mouse on the cable: its encoders, stepped, and the switches.
pub struct MouseOnCable {
    nets: MouseNets,
    encoders: Encoders,
    buttons: u8,
    /// Whether the lines have been driven at all yet.
    driven: bool,
    /// Steps stepped out since plugged in, for a test.
    pub steps: u64,
}

impl MouseOnCable {
    pub fn of(n: &Netlist) -> Option<MouseOnCable> {
        MouseNets::of(n).map(MouseOnCable::new)
    }

    pub fn new(nets: MouseNets) -> MouseOnCable {
        MouseOnCable { nets, encoders: Encoders::default(), buttons: 0, driven: false, steps: 0 }
    }

    /// Motion to step out and the buttons as they now stand. Motion
    /// accumulates; buttons replace.
    pub fn send(&mut self, dx: i32, dy: i32, buttons: u8) {
        self.encoders.send(dx, dy);
        self.buttons = buttons & 0o7;
    }

    pub fn busy(&self) -> bool {
        self.encoders.busy()
    }

    /// The buttons as they stand on the lines.
    pub fn buttons(&self) -> u8 {
        self.buttons
    }

    /// When the next step is due, if there is motion to step out.
    pub fn next_change(&self, now: u64) -> Option<u64> {
        self.encoders.next_change(now)
    }

    /// Puts the seven lines where the state says. Through the inverting
    /// Schmitt triggers a high on the cable is a low on the board; a
    /// switch pulls its line low when pressed, as a switch to ground does,
    /// and reads on the board as a high.
    fn drive(&mut self, board: &mut Chip) {
        let lv = |on: bool| if on { Level::High } else { Level::Low };
        let [xa, xb, ya, yb] = self.encoders.lines();
        board.drive(self.nets.hora, lv(xa));
        board.drive(self.nets.horb, lv(xb));
        board.drive(self.nets.vera, lv(ya));
        board.drive(self.nets.verb, lv(yb));
        for (k, &net) in self.nets.switches.iter().enumerate() {
            board.drive(net, lv(self.buttons >> k & 1 == 0));
        }
        self.driven = true;
    }

    /// Puts the lines on the cable if they are not yet, moves the switches
    /// as they stand, and when a step is due moves each encoder one phase
    /// toward where it is going. Returns whether a net moved, in which
    /// case the board was transitioned at `now`.
    pub fn apply(&mut self, board: &mut Chip, now: u64) -> bool {
        let mut moved = !self.driven;
        if self.encoders.step(now) {
            self.steps += 1;
            moved = true;
        }
        // The switches are level, and go out whenever they differ from
        // what is on the lines.
        let want: Vec<Level> = (0..3)
            .map(|k| if self.buttons >> k & 1 == 0 { Level::High } else { Level::Low })
            .collect();
        let have: Vec<Option<Level>> =
            self.nets.switches.iter().map(|&n| board.external_on(n)).collect();
        if want.iter().zip(&have).any(|(w, h)| Some(*w) != *h) {
            moved = true;
        }
        if moved {
            self.drive(board);
            board.transition(now);
        }
        moved
    }
}
