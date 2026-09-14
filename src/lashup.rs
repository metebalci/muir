// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Two machines on a debug cable, in one process: the reference transport,
//! where the drawings are verified.
//!
//! The debugger's DBGOUT and the debuggee's DBGIN are both behavioural
//! here, in `src/busint.rs`; this carries the cable between them and keeps
//! the two machines' clocks honest with each other.  The cable is 21 wires
//! as timestamped levels ([`CableEvent`]), and **each machine runs only as
//! far as the other has promised**: the debugger promises the earliest
//! instant a request or a release can appear on the cable
//! ([`Rtl::debug_out_promise`]), the debuggee the earliest instant its
//! `DEBUG ACK` can rise ([`Rtl::debug_in_promise`]), and a machine is
//! stepped only when the step cannot carry it past the other's promise.
//! Simulated time and not wall time, because the debugger's interface
//! times a debug cycle out [`crate::busint::DEBUG_TIMEOUT_NS`] after the
//! first edge of its timeout clock, in *machine* time.
//!
//! A step of [`Rtl`] may run past the bound it is given by less than one
//! generator cycle at the slowest speed ([`Rtl::step_until`]), so that is
//! the slack every promise is read with.  The scheduler cannot deadlock:
//! the debuggee promises anything finite only while a request of the
//! debugger's is on the cable, during which the debugger promises the
//! release at its timeout and nothing sooner; and the debugger promises
//! anything finite always, so with nothing on the cable it is the one that
//! moves.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc;

use crate::busint::{CableEvent, DebugRequest};
use crate::clock::Speed;
use crate::isa::Insn;
use crate::isa::asm::{
    ALU, MD, SETM, SRC_MD, START_READ, START_WRITE, a_dest, a_src, filler, m_src,
};
use crate::machine::{Halt, Machine};
use crate::rtl::Rtl;

/// The most a step of either machine can run past the bound it was given:
/// one generator cycle at the slowest speed, `ILONG` or not.
pub fn max_step_ns() -> u64 {
    Speed::ExtraSlow.cycle_ns(true) as u64
}

/// The two machines and the two cables between them, as MIT wired a
/// lashup: `debugger`'s DBGOUT to `debuggee`'s DBGIN, and `debuggee`'s
/// DBGOUT to `debugger`'s DBGIN, so that either may debug the other.  The
/// names say which one CC runs on.
///
/// **The cables' clock starts when they are plugged in**, as over a stream
/// ([`Remote`]): each machine has run some way on its own clock by then ---
/// a board brought up to its first microcycle, a machine resumed --- and an
/// instant of one machine's is carried to the other through the difference
/// of the two clocks at [`Lashup::new`].  Two machines at zero, as they
/// mostly are, are carried unchanged.
pub struct Lashup {
    pub debugger: Rtl,
    pub debuggee: Rtl,
    /// Each machine's clock when the cables were plugged in.
    origin: (u64, u64),
    /// The standing answer on each cable has been carried to the machine
    /// that asked: the debuggee's to the debugger, the debugger's to the
    /// debuggee.
    carried: (bool, bool),
    /// Steps taken by each, for the curious.
    pub steps: (u64, u64),
}

/// `ns` on the clock whose origin is `from`, as the clock whose origin is
/// `to` has it; the end of time is the end of time.
fn shift(ns: u64, from: u64, to: u64) -> u64 {
    if ns == u64::MAX { ns } else { ns.saturating_sub(from).saturating_add(to) }
}

impl Lashup {
    /// Plugs both cables in: each machine's DBGOUT waits for the other
    /// from here on.
    pub fn new(mut debugger: Rtl, mut debuggee: Rtl) -> Lashup {
        debugger.attach_debug_cable();
        debuggee.attach_debug_cable();
        let origin = (debugger.ns(), debuggee.ns());
        Lashup { debugger, debuggee, origin, carried: (true, true), steps: (0, 0) }
    }

    /// What `other` promises `this` about the cables: the earliest instant
    /// its answer to `this`'s pending request can come --- an answer it
    /// holds is its instant; nothing, once carried --- and the earliest
    /// its own next request can appear on `this`'s DBGIN.
    fn promise(other: &Rtl, carried: bool) -> u64 {
        let back = if carried {
            u64::MAX
        } else {
            match other.debug_ack() {
                Some((at, _)) => at,
                None => other.debug_in_promise(),
            }
        };
        back.min(other.debug_out_promise())
    }

    /// One step of whichever machine may take one, and whatever it put on
    /// its cable carried to the other.
    pub fn step(&mut self) -> Result<(), Halt> {
        let slack = max_step_ns();
        let (oa, ob) = self.origin;
        // Each machine's promise on the other's clock, and the two clocks
        // as time since the cables were plugged in.
        let to_a = shift(Self::promise(&self.debuggee, self.carried.0), ob, oa);
        let to_b = shift(Self::promise(&self.debugger, self.carried.1), oa, ob);
        let (a, b) = (self.debugger.ns(), self.debuggee.ns());
        let (a_since, b_since) = (a.saturating_sub(oa), b.saturating_sub(ob));
        // Strictly within: a step bounded at the machine's own present would
        // make no progress.
        let a_may = a.saturating_add(slack) < to_a;
        let b_may = b.saturating_add(slack) < to_b;
        // The one behind goes first, so that neither gets a step ahead of
        // the other for longer than it must.
        if a_may && (a_since <= b_since || !b_may) {
            self.debugger.step_until(to_a.saturating_sub(slack))?;
            self.steps.0 += 1;
            Self::carry_out(&mut self.debugger, &mut self.debuggee, &mut self.carried.0, oa, ob);
        } else if b_may {
            self.debuggee.step_until(to_b.saturating_sub(slack))?;
            self.steps.1 += 1;
            Self::carry_out(&mut self.debuggee, &mut self.debugger, &mut self.carried.1, ob, oa);
        } else if a_since <= b_since {
            // Neither may: each holds the other's request, with its answer
            // due within a cycle of the other's, so that neither can reach
            // its own answer without passing the other's --- both cables
            // carrying cycles at once.  The one behind steps to the other's
            // promise, and may pass it by less than a cycle; the answer
            // then lands in its recent past, where `debug_out_answer`
            // places the acknowledgement and the cpu takes it at its next
            // look, a generator cycle late at worst.
            self.debugger.step_until(to_a)?;
            self.steps.0 += 1;
            Self::carry_out(&mut self.debugger, &mut self.debuggee, &mut self.carried.0, oa, ob);
        } else {
            self.debuggee.step_until(to_b)?;
            self.steps.1 += 1;
            Self::carry_out(&mut self.debuggee, &mut self.debugger, &mut self.carried.1, ob, oa);
        }
        Self::carry_back(&mut self.debuggee, &mut self.debugger, &mut self.carried.0, ob, oa);
        Self::carry_back(&mut self.debugger, &mut self.debuggee, &mut self.carried.1, oa, ob);
        Ok(())
    }

    /// Runs both machines to `ns` at least.
    pub fn run_until(&mut self, ns: u64) -> Result<(), Halt> {
        while self.debugger.ns() < ns || self.debuggee.ns() < ns {
            self.step()?;
        }
        Ok(())
    }

    /// `from`'s DBGOUT to `to`'s DBGIN: a request or a release carried, its
    /// instant put on `to`'s clock from `from`'s by the two origins.
    fn carry_out(
        from: &mut Rtl,
        to: &mut Rtl,
        carried: &mut bool,
        from_origin: u64,
        to_origin: u64,
    ) {
        if let Some(event) = from.debug_out_take() {
            match event {
                CableEvent::Request { at, request } => {
                    to.debug_request(shift(at, from_origin, to_origin), request);
                    *carried = false;
                }
                CableEvent::Release { at } => to.debug_release(shift(at, from_origin, to_origin)),
            }
        }
    }

    /// `from`'s acknowledgement on its DBGIN back to `to`'s DBGOUT, once,
    /// its instant on `to`'s clock likewise.
    fn carry_back(
        from: &mut Rtl,
        to: &mut Rtl,
        carried: &mut bool,
        from_origin: u64,
        to_origin: u64,
    ) {
        if !*carried && let Some((ack, word)) = from.debug_ack() {
            *carried = true;
            to.debug_out_answer(shift(ack, from_origin, to_origin), word);
        }
    }
}

/// A debugger's microcode, built up: CC's `DBG-WRITE` and `DBG-READ` as the
/// three Unibus cycles into its own debug block that `ldbg.lisp` makes them
/// --- `%UNIBUS-WRITE 766110`, `%UNIBUS-WRITE 766114`, then `%UNIBUS-READ`
/// or `%UNIBUS-WRITE 766100` --- each `MD <- value; VMA <- address, start`
/// off constants in M memory, fillers while the interface waits for the
/// other machine, and a read's word parked in A memory.  Virtual page 0 is
/// on physical page `37766`, the block's, so `766110`, `766114` and
/// `766100` are virtual `44`, `46` and `40`.  A memory 3 holds `123456`
/// for the fillers.  For the tests, on every engine; CC itself is Lisp in a
/// band.
pub struct DebugProgram {
    m: Machine,
    prom: Vec<Insn>,
    at: usize,
    next_m: usize,
    /// CC's `CC-UNIBUS-MAP-TO-MD-OK-FLAG`: map register `16` loaded once.
    md_mapped: bool,
}

impl Default for DebugProgram {
    fn default() -> Self {
        Self::new()
    }
}

impl DebugProgram {
    /// Fillers after each cycle's start: 4.4 microseconds at the boot's
    /// speed, more than a debug cycle takes the other machine to answer.
    pub const GAP: usize = 20;

    pub fn new() -> DebugProgram {
        let mut m = Machine::new();
        m.amem[3] = 0o123456;
        m.l1_map[0] = 0;
        m.l2_map[0] = (1 << 23) | (1 << 22) | 0o37766;
        // The three registers' virtual addresses; values from 4 on.
        m.mmem[1] = 0o44;
        m.mmem[2] = 0o46;
        m.mmem[3] = 0o40;
        DebugProgram {
            m,
            prom: vec![filler(); crate::machine::PROM_WORDS],
            at: 0,
            next_m: 4,
            md_mapped: false,
        }
    }

    fn constant(&mut self, v: u32) -> u64 {
        let k = self.next_m;
        assert!(k < 32, "out of M memory for constants");
        self.m.mmem[k] = v;
        self.next_m += 1;
        k as u64
    }

    fn unibus_write(&mut self, vma_m: u64, val: u32) {
        let v = self.constant(val);
        self.prom[self.at] = Insn::new(ALU | SETM | m_src(v) | a_src(3) | MD);
        self.prom[self.at + 1] = Insn::new(ALU | SETM | m_src(vma_m) | a_src(3) | START_WRITE);
        self.at += 2 + Self::GAP;
    }

    fn unibus_read(&mut self, vma_m: u64, park: u64) {
        self.prom[self.at] = Insn::new(ALU | SETM | m_src(vma_m) | a_src(3) | START_READ);
        self.prom[self.at + 1 + Self::GAP] =
            Insn::new(ALU | SETM | SRC_MD | a_src(3) | a_dest(park));
        self.at += 2 + Self::GAP;
    }

    /// `words` fillers before whatever comes next: time for the other
    /// machine.
    pub fn wait(&mut self, words: usize) {
        self.at += words;
    }

    /// `DBG-WRITE`: `val` into Unibus `uaddr` on the other machine.
    pub fn dbg_write(&mut self, uaddr: u32, val: u16) {
        self.unibus_write(1, (uaddr >> 17) & 1);
        self.unibus_write(2, (uaddr >> 1) & 0xffff);
        self.unibus_write(3, val as u32);
    }

    /// `DBG-READ`: Unibus `uaddr` on the other machine, the word into A
    /// memory `park`.
    pub fn dbg_read(&mut self, uaddr: u32, park: u64) {
        self.unibus_write(1, (uaddr >> 17) & 1);
        self.unibus_write(2, (uaddr >> 1) & 0xffff);
        self.unibus_read(3, park);
    }

    /// CC's `DBG-UNIBUS-MAP-NUMBER`: map register `17`, `766176`, "for
    /// DBG-READ-XBUS and related routines" in `unaddr.text`'s allocation.
    pub const MAP: u32 = 0o17;

    /// `DBG-SETUP-UNIBUS-MAP`: the map register loaded with the Xbus page,
    /// valid and writable --- `(+ 140000 (LDB 1016 XBUS-LOC))` --- and the
    /// Unibus address of the word's low half, `(+ 140000 (* LOC 2000) (* 4
    /// (LOGAND 377 XBUS-LOC)))`.
    fn setup_map(&mut self, xbus_loc: u32) -> u32 {
        self.dbg_write(0o766140 + 2 * Self::MAP, (0o140000 | ((xbus_loc >> 8) & 0o37777)) as u16);
        0o140000 + Self::MAP * 0o2000 + 4 * (xbus_loc & 0o377)
    }

    /// `DBG-READ-XBUS`: the other machine's memory word at `xbus_loc`
    /// through its Unibus map, low half then high, into A memory `low` and
    /// `high`.
    pub fn dbg_read_xbus(&mut self, xbus_loc: u32, low: u64, high: u64) {
        let u = self.setup_map(xbus_loc);
        self.dbg_read(u, low);
        self.dbg_read(u + 2, high);
    }

    /// `DBG-WRITE-XBUS`: `val` into the other machine's memory word at
    /// `xbus_loc`, low half then high.
    pub fn dbg_write_xbus(&mut self, xbus_loc: u32, val: u32) {
        let u = self.setup_map(xbus_loc);
        self.dbg_write(u, val as u16);
        self.dbg_write(u + 2, (val >> 16) as u16);
    }

    /// `CC-WRITE-MD`: `val` into the other machine's `MD`.  Map register
    /// `16` is loaded once with `177000` --- "VALID + WR-ENB + MAGIC HIGH 5
    /// 1'S TO ADDRESS MD" --- and the word goes as two halves through it,
    /// `174000` then `174002`; the high half's write lands in `MD` by `-UB
    /// TO MD` and makes no Xbus cycle.
    pub fn dbg_write_md(&mut self, val: u32) {
        if !self.md_mapped {
            self.dbg_write(0o766174, 0o177000);
            self.md_mapped = true;
        }
        self.dbg_write(0o174000, val as u16);
        self.dbg_write(0o174002, (val >> 16) as u16);
    }

    /// The machine with the program in its boot PROM, ready for any engine.
    pub fn finish(mut self) -> Machine {
        assert!(self.at <= crate::machine::PROM_WORDS, "the program overran the PROM");
        self.m.load_prom(&self.prom);
        self.m
    }
}

// --- The cable over a stream: the second transport ---------------------------

/// A message on the cable's stream: the same events [`Lashup`] carries by
/// hand, and the promise each side makes the other.  Fixed frames of
/// [`Message::FRAME`] bytes, a tag and little-endian fields, so that either
/// end can be another program.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Message {
    /// The debugger's `-DEBUG OUT REQ` down at `at`.
    Request { at: u64, request: DebugRequest },
    /// The debugger's request lifted at `at` unacknowledged, its timeout.
    Release { at: u64 },
    /// The debuggee's `DEBUG ACK` at `at`, with the word it drove if any.
    Ack { at: u64, word: Option<u16> },
    /// Nothing from this side before `until`: no request and no release
    /// from the debugger, no acknowledgement from the debuggee.  The other
    /// side may run to it and no further.  `seen` is how many of the other
    /// side's events --- the debuggee's acknowledgements, or the debugger's
    /// requests --- this side had taken when it made the promise: a promise
    /// made before an event the other side has since sent is stale, since
    /// the event changes what can be promised, and is ignored.
    Promise { until: u64, seen: u32 },
    /// This side has run as far as it will; nothing more comes from it.
    Done,
}

impl Message {
    pub const FRAME: usize = 24;

    pub fn encode(&self) -> [u8; Self::FRAME] {
        let mut b = [0u8; Self::FRAME];
        match *self {
            Message::Request { at, request } => {
                b[0] = 1;
                b[1..9].copy_from_slice(&at.to_le_bytes());
                b[9] = request.strobe;
                b[10] = request.write as u8;
                b[11..13].copy_from_slice(&request.dbd.to_le_bytes());
                b[13..21].copy_from_slice(&request.hold_ns.to_le_bytes());
            }
            Message::Release { at } => {
                b[0] = 2;
                b[1..9].copy_from_slice(&at.to_le_bytes());
            }
            Message::Ack { at, word } => {
                b[0] = 3;
                b[1..9].copy_from_slice(&at.to_le_bytes());
                b[9] = word.is_some() as u8;
                b[10..12].copy_from_slice(&word.unwrap_or(0).to_le_bytes());
            }
            Message::Promise { until, seen } => {
                b[0] = 4;
                b[1..9].copy_from_slice(&until.to_le_bytes());
                b[9..13].copy_from_slice(&seen.to_le_bytes());
            }
            Message::Done => b[0] = 5,
        }
        b
    }

    pub fn decode(b: &[u8; Self::FRAME]) -> io::Result<Message> {
        let u64_at = |k: usize| u64::from_le_bytes(b[k..k + 8].try_into().unwrap());
        let u16_at = |k: usize| u16::from_le_bytes(b[k..k + 2].try_into().unwrap());
        Ok(match b[0] {
            1 => Message::Request {
                at: u64_at(1),
                request: DebugRequest {
                    strobe: b[9],
                    write: b[10] != 0,
                    dbd: u16_at(11),
                    hold_ns: u64_at(13),
                },
            },
            2 => Message::Release { at: u64_at(1) },
            3 => Message::Ack { at: u64_at(1), word: (b[9] != 0).then_some(u16_at(10)) },
            4 => Message::Promise {
                until: u64_at(1),
                seen: u32::from_le_bytes(b[9..13].try_into().unwrap()),
            },
            5 => Message::Done,
            tag => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("not a cable message: tag {tag}"),
                ));
            }
        })
    }

    pub fn write_to(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(&self.encode())?;
        w.flush()
    }

    pub fn read_from(r: &mut impl Read) -> io::Result<Message> {
        let mut b = [0u8; Self::FRAME];
        r.read_exact(&mut b)?;
        Self::decode(&b)
    }
}

/// One end of the debug cable as an engine offers it to [`Remote`]: its
/// clock, a step that stops at a bound, and the cable calls of the side it
/// is --- DBGIN's for a debuggee, DBGOUT's for a debugger.  An end
/// implements the side it can be: [`Rtl`] is either, the netlist board
/// with the machine behind it is a debuggee ([`crate::cable::DebugIn`]),
/// the fabric's register window is a debuggee with no clock of muir's
/// ([`crate::fabric::Fabric`]), and a call an end cannot answer is an
/// error in the caller.
pub trait CableEnd {
    /// The machine's clock, in nanoseconds.  An end that is not a
    /// simulated machine has none: [`FreeRunning`] steps the debugger
    /// alone and asks its debuggee for neither.
    fn ns(&self) -> u64 {
        unreachable!("this end of the cable has no clock of its own")
    }
    /// One step, running no further past `limit` than [`max_step_ns`].
    fn step_until(&mut self, _limit: u64) -> Result<(), Halt> {
        unreachable!("this end of the cable has no clock of its own")
    }

    // --- DBGIN: this end as the debuggee ---

    /// `-DEBUG IN REQ` down at `at` with the wires in `request`.  `Err` is
    /// a request this end cannot take --- one while another is on the
    /// cable, or one in this end's past by more than a cycle --- which in
    /// process is the lashup's scheduling error and from a peer on a
    /// stream is the peer's, and ends the run.
    fn debug_request(&mut self, _at: u64, _request: DebugRequest) -> Result<(), String> {
        unreachable!("this end of the cable is no debuggee")
    }
    /// The request lifted at `at` unacknowledged.  `Err` as for
    /// [`CableEnd::debug_request`].
    fn debug_release(&mut self, _at: u64) -> Result<(), String> {
        unreachable!("this end of the cable is no debuggee")
    }
    /// `DEBUG ACK` up for the request on the cable: when, and the word this
    /// side drove on `DBD` if any; `None` until the machine has run to it.
    fn debug_ack(&self) -> Option<(u64, Option<u16>)> {
        unreachable!("this end of the cable is no debuggee")
    }
    /// No `DEBUG ACK` before this instant, for the request on the cable.
    fn debug_in_promise(&self) -> u64 {
        unreachable!("this end of the cable is no debuggee")
    }
    /// The cable pulled out of this end's DBGIN: `-DEBUG IN REQ` back on
    /// its pull-up now, whatever the debugger had on the connector, and
    /// nothing more coming down the wire.  A release now, for an end that
    /// holds nothing else of the debugger's.
    fn debug_unplug(&mut self) {
        let now = self.ns();
        let _ = self.debug_release(now);
    }

    // --- DBGOUT: this end as the debugger ---

    /// A cable is plugged into this end's DBGOUT.
    fn attach_debug_cable(&mut self) {
        unreachable!("this end of the cable is no debugger")
    }
    /// What this end's DBGOUT has put on the cable, if anything.
    fn debug_out_take(&mut self) -> Option<CableEvent> {
        unreachable!("this end of the cable is no debugger")
    }
    /// The other end's acknowledgement; whether this end still waited for it.
    fn debug_out_answer(&mut self, _ack_at: u64, _word: Option<u16>) -> bool {
        unreachable!("this end of the cable is no debugger")
    }
    /// No request and no release before this instant.
    fn debug_out_promise(&self) -> u64 {
        unreachable!("this end of the cable is no debugger")
    }
}

impl CableEnd for Rtl {
    fn ns(&self) -> u64 {
        Rtl::ns(self)
    }
    fn step_until(&mut self, limit: u64) -> Result<(), Halt> {
        Rtl::step_until(self, limit)
    }
    fn debug_request(&mut self, at: u64, request: DebugRequest) -> Result<(), String> {
        Rtl::try_debug_request(self, at, request)
    }
    fn debug_release(&mut self, at: u64) -> Result<(), String> {
        Rtl::try_debug_release(self, at)
    }
    fn debug_ack(&self) -> Option<(u64, Option<u16>)> {
        Rtl::debug_ack(self)
    }
    fn debug_in_promise(&self) -> u64 {
        Rtl::debug_in_promise(self)
    }
    fn attach_debug_cable(&mut self) {
        Rtl::attach_debug_cable(self)
    }
    fn debug_out_take(&mut self) -> Option<CableEvent> {
        Rtl::debug_out_take(self)
    }
    fn debug_out_answer(&mut self, ack_at: u64, word: Option<u16>) -> bool {
        Rtl::debug_out_answer(self, ack_at, word)
    }
    fn debug_out_promise(&self) -> u64 {
        Rtl::debug_out_promise(self)
    }
}

/// What stops a machine at one end of a stream: its own microcode, or the
/// stream.
#[derive(Debug)]
pub enum Error {
    Halt(Halt),
    Io(io::Error),
}

impl From<Halt> for Error {
    fn from(h: Halt) -> Error {
        Error::Halt(h)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Halt(h) => write!(f, "the machine halted: {h:?}"),
            Error::Io(e) => write!(f, "the stream: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// Which end of the cable a [`Remote`] is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Debugger,
    Debuggee,
}

/// One machine at one end of the cable over a byte stream --- a TCP
/// connection, with the other machine in another program --- running the
/// protocol [`Lashup`] runs in process, on whatever end the engine offers
/// ([`CableEnd`]).  Each side tells the other what it
/// promises, after every step that changes the promise, and steps only
/// while the step cannot carry it past the other's last promise; when it
/// cannot step it waits for a message.  The debugger's requests and
/// releases and the debuggee's acknowledgements go as they happen, with
/// their instants.
///
/// After sending a request the debugger takes the debuggee's promise to be
/// the request's own instant --- a strobe is acknowledged then --- until
/// told better; after sending its acknowledgement the debuggee takes the
/// debugger's promise to be nothing until told, since the debugger's
/// promise drops from its timeout's release to its next cycle's earliest
/// request the moment it has the answer.  A promise the other side made
/// before it had that request or that acknowledgement is stale and is
/// ignored, which is what [`Message::Promise`]'s count is for: without it
/// the debuggee once took a pre-acknowledgement promise of the debugger's
/// release, 26 microseconds off, and ran that far ahead.  Neither side ever
/// waits on the other while the other waits on it, as in process.
///
/// **The cable's clock starts at the connection.**  Each end has run some
/// way on its own clock by the time the cable is plugged in --- a debuggee
/// that has been up an hour before its debugger comes, or a debugger
/// started first --- and the instants on the wire are read against the
/// reader's own clock, so each end sends its instants as time since it was
/// plugged in and reads the other's the same way: `origin` is this end's
/// clock at the connection, taken off every instant sent and put on every
/// instant received.  Two ends plugged in at power-on have an origin of
/// zero each and the frames as they always were; a promise of the end of
/// time is one either way.
pub struct Remote<E: CableEnd> {
    pub machine: E,
    side: Side,
    /// This machine's clock when the cable was plugged in.
    origin: u64,
    writer: Box<dyn Write + Send>,
    inbox: mpsc::Receiver<io::Result<Message>>,
    /// The other side's last promise, as this side knows it.
    other_promise: u64,
    /// This side's last promise sent.
    sent_promise: Option<u64>,
    /// The debuggee's standing answer has been sent.
    carried: bool,
    other_done: bool,
    /// Events this side has sent that change the other's promises: the
    /// debugger's requests, the debuggee's acknowledgements.
    sent_events: u32,
    /// The other side's such events this side has taken.
    seen_events: u32,
    /// `DEBUG_CYCLE` requests carried, either way.
    cycles: u64,
}

impl<E: CableEnd> Remote<E> {
    /// The debugger's end: `machine`'s DBGOUT on the stream.
    pub fn debugger(
        machine: E,
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
    ) -> Remote<E> {
        let mut machine = machine;
        machine.attach_debug_cable();
        Self::new(Side::Debugger, machine, reader, writer)
    }

    /// The debuggee's end: `machine`'s DBGIN on the stream.
    pub fn debuggee(
        machine: E,
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
    ) -> Remote<E> {
        Self::new(Side::Debuggee, machine, reader, writer)
    }

    /// How many frames the reader holds for the machine.  Full, the reader
    /// reads no more until the machine takes one, and the peer's writes
    /// wait on the stream: a peer sending frames faster than the machine
    /// steps is held to the machine's pace and not buffered without end.
    /// 256 frames is 6 KB of cable.
    pub const INBOX_FRAMES: usize = 256;

    fn new(
        side: Side,
        machine: E,
        mut reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
    ) -> Remote<E> {
        let (tx, inbox) = mpsc::sync_channel(Self::INBOX_FRAMES);
        // The reader ends with the stream --- a frame that does not read
        // whole is its last --- or at its next frame once the machine is
        // gone and nothing takes what it read.
        std::thread::Builder::new()
            .name("debug cable reader".into())
            .spawn(move || {
                loop {
                    // A frame that does not read whole is the stream's
                    // end, said as that rather than as a short read.
                    let m = Message::read_from(&mut reader).map_err(|e| {
                        if e.kind() == io::ErrorKind::UnexpectedEof {
                            io::Error::new(e.kind(), "the other end of the cable went away")
                        } else {
                            e
                        }
                    });
                    let stop = matches!(m, Err(_) | Ok(Message::Done));
                    if tx.send(m).is_err() || stop {
                        break;
                    }
                }
            })
            .expect("a thread for the debug cable's reader");
        let origin = machine.ns();
        Remote {
            machine,
            side,
            origin,
            writer: Box::new(writer),
            inbox,
            // The debuggee has nothing to say until asked; the debugger may
            // ask at any time, so the debuggee waits to be told.
            other_promise: match side {
                Side::Debugger => u64::MAX,
                Side::Debuggee => 0,
            },
            sent_promise: None,
            carried: true,
            other_done: false,
            sent_events: 0,
            seen_events: 0,
            cycles: 0,
        }
    }

    /// Which end of the cable this is.
    pub fn side(&self) -> Side {
        self.side
    }

    /// The other end has said it is done: nothing more comes down the
    /// cable, and this end may run as it likes.
    pub fn peer_done(&self) -> bool {
        self.other_done
    }

    /// `DEBUG_CYCLE` requests carried over this cable, either way: the
    /// debugger's sent, or the debuggee's taken.
    pub fn debug_cycles(&self) -> u64 {
        self.cycles
    }

    /// The cable pulled out at this end: the other end is told this one is
    /// done, as [`Remote::finish`] tells it, and not waited for --- it has
    /// gone, or said it is done and waits only for this --- and the
    /// machine comes back with whatever the debugger had on its DBGIN
    /// lifted ([`CableEnd::debug_unplug`]), to run on its own.
    pub fn hang_up(mut self) -> E {
        let _ = self.send(Message::Promise { until: u64::MAX, seen: self.seen_events });
        let _ = self.send(Message::Done);
        if self.side == Side::Debuggee {
            self.machine.debug_unplug();
        }
        self.machine
    }

    /// `m` as it goes on the wire: its instants as time since the
    /// connection.  The end of time is the end of time.
    fn on_the_wire(&self, m: Message) -> Message {
        let out = |ns: u64| if ns == u64::MAX { ns } else { ns.saturating_sub(self.origin) };
        match m {
            Message::Request { at, request } => Message::Request { at: out(at), request },
            Message::Release { at } => Message::Release { at: out(at) },
            Message::Ack { at, word } => Message::Ack { at: out(at), word },
            Message::Promise { until, seen } => Message::Promise { until: out(until), seen },
            Message::Done => Message::Done,
        }
    }

    /// `m` off the wire: its instants on this machine's clock.
    fn off_the_wire(&self, m: Message) -> Message {
        let here = |ns: u64| ns.saturating_add(self.origin);
        match m {
            Message::Request { at, request } => Message::Request { at: here(at), request },
            Message::Release { at } => Message::Release { at: here(at) },
            Message::Ack { at, word } => Message::Ack { at: here(at), word },
            Message::Promise { until, seen } => Message::Promise { until: here(until), seen },
            Message::Done => Message::Done,
        }
    }

    /// Runs this machine to `ns` at least, in step with the other end, and
    /// then stays on the line until the other end is done too.
    pub fn run_until(&mut self, ns: u64) -> Result<(), Error> {
        while self.machine.ns() < ns {
            self.step()?;
        }
        self.finish()
    }

    /// One step of this machine if the other end's promise allows one, else
    /// a wait for the other end's next message.  Returns whether the
    /// machine stepped.
    pub fn step(&mut self) -> Result<bool, Error> {
        let slack = max_step_ns();
        self.drain()?;
        self.promise()?;
        if self.machine.ns().saturating_add(slack) < self.other_promise {
            let limit = self.other_promise.saturating_sub(slack);
            self.machine.step_until(limit)?;
            self.after_step()?;
            Ok(true)
        } else {
            self.wait_one()?;
            Ok(false)
        }
    }

    /// Tells the other end this machine is done and waits until it is too,
    /// so that neither end is left waiting on a promise.
    pub fn finish(&mut self) -> Result<(), Error> {
        self.send(Message::Promise { until: u64::MAX, seen: self.seen_events })?;
        self.send(Message::Done)?;
        while !self.other_done {
            self.wait_one()?;
        }
        Ok(())
    }

    fn my_promise(&self) -> u64 {
        match self.side {
            Side::Debugger => self.machine.debug_out_promise(),
            Side::Debuggee if self.carried => u64::MAX,
            // An answer this side holds and has not yet sent is what the
            // other must not run past --- a strobe's, at the request's own
            // instant, before the machine has run that far.
            Side::Debuggee => match self.machine.debug_ack() {
                Some((at, _)) => at,
                None => self.machine.debug_in_promise(),
            },
        }
    }

    /// Tells the other side this side's promise, if it has changed.
    fn promise(&mut self) -> io::Result<()> {
        let mine = self.my_promise();
        if self.sent_promise != Some(mine) {
            self.send(Message::Promise { until: mine, seen: self.seen_events })?;
            self.sent_promise = Some(mine);
        }
        Ok(())
    }

    fn send(&mut self, m: Message) -> io::Result<()> {
        self.on_the_wire(m).write_to(&mut self.writer)
    }

    /// What this side's step put on the cable.
    fn after_step(&mut self) -> io::Result<()> {
        match self.side {
            Side::Debugger => {
                if let Some(event) = self.machine.debug_out_take() {
                    match event {
                        CableEvent::Request { at, request } => {
                            self.send(Message::Request { at, request })?;
                            self.sent_events += 1;
                            if request.strobe == crate::busint::DEBUG_CYCLE {
                                self.cycles += 1;
                            }
                            // The debuggee's acknowledgement can be no
                            // sooner than the request itself --- a strobe's
                            // is then --- and it will say when it knows.
                            self.other_promise = at;
                        }
                        CableEvent::Release { at } => self.send(Message::Release { at })?,
                    }
                    self.promise()?;
                }
            }
            Side::Debuggee => {
                if !self.carried
                    && let Some((at, word)) = self.machine.debug_ack()
                {
                    self.send(Message::Ack { at, word })?;
                    self.sent_events += 1;
                    self.carried = true;
                    // The debugger's promise drops the moment it has this;
                    // wait to be told what to.
                    self.other_promise = 0;
                    self.promise()?;
                }
            }
        }
        Ok(())
    }

    /// Applies one message from the other end.  `Err` is a message the
    /// peer had no business sending --- the other side's, a request the
    /// cable cannot take --- which ends the run rather than the process:
    /// the frames are fixed so that any program may be the peer, and a
    /// wrong one is answered with an error naming what it sent.
    fn apply(&mut self, m: Message) -> io::Result<()> {
        let refused = |what: String| io::Error::new(io::ErrorKind::InvalidData, what);
        match (self.side, self.off_the_wire(m)) {
            (Side::Debugger, Message::Ack { at, word }) => {
                self.machine.debug_out_answer(at, word);
                self.seen_events += 1;
                self.other_promise = u64::MAX;
            }
            (Side::Debuggee, Message::Request { at, request }) => {
                self.machine.debug_request(at, request).map_err(refused)?;
                self.seen_events += 1;
                self.carried = false;
                if request.strobe == crate::busint::DEBUG_CYCLE {
                    self.cycles += 1;
                }
            }
            (Side::Debuggee, Message::Release { at }) => {
                self.machine.debug_release(at).map_err(refused)?;
            }
            // A promise made before the other side had this side's latest
            // event is stale; the fresh one follows.
            (_, Message::Promise { until, seen }) => {
                if seen == self.sent_events {
                    self.other_promise = until;
                }
            }
            (_, Message::Done) => {
                self.other_done = true;
                self.other_promise = u64::MAX;
            }
            (side, m) => return Err(refused(format!("the {side:?} was sent {m:?}"))),
        }
        Ok(())
    }

    /// Applies every message already arrived.
    fn drain(&mut self) -> io::Result<()> {
        while let Ok(m) = self.inbox.try_recv() {
            self.apply(m?)?;
        }
        Ok(())
    }

    /// Waits for one message and applies it.
    fn wait_one(&mut self) -> io::Result<()> {
        match self.inbox.recv() {
            Ok(m) => self.apply(m?),
            Err(_) if self.other_done => Ok(()),
            Err(_) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the other end of the cable went away",
            )),
        }
    }
}

// --- DBGIN's connector, with the debugger's cable meeting it over TCP ---------

/// DBGIN's connector with a debugger's cable meeting it over TCP: the
/// machine behind it, running on its own until a debugger connects, in
/// step with the debugger ([`Remote`]) while one is on the cable, and on
/// its own again when the cable goes.  The bus interface's DBGIN is always
/// there --- it takes the Unibus as master when a debugger drives it, and
/// nothing in the machine enables it --- so a machine has one of these
/// whether or not a debugger ever comes, with a listener at the endpoint
/// the run gave it or none at all, which is the machine alone.
///
/// The listener is looked at between microcycles ([`Connector::attend`]),
/// as the terminal is, and a debugger connecting is plugged in then: the
/// cable's clock starts at that instant ([`Remote`]).  One cable per
/// connector: a second debugger connecting while one is on is refused, its
/// connection closed at once.  A debugger that says it is done, or whose
/// stream goes away, is unplugged --- [`Connector::step_cabled`] says so
/// --- with whatever it had on DBGIN lifted and the listener open again.
///
/// The machine is stepped by whoever holds the connector: on its own with
/// no debugger on the cable, however the engine steps --- a microcycle of
/// `rtl`, a transition of the netlist --- and by [`Connector::step_cabled`]
/// while one is, which steps as the debugger's promise allows.
pub struct Connector<E: CableEnd> {
    /// Always `Some` between calls; taken to move the machine between the
    /// two ends.
    end: Option<End<E>>,
    listener: Option<TcpListener>,
    /// Where the debugger on the cable connected from, while one is on.
    debugger: Option<SocketAddr>,
    /// A handle on the cable's socket while one is on, to shut it down when
    /// the cable is pulled: the [`Remote`]'s reader thread holds a handle of
    /// its own and sits in a read, so dropping the [`Remote`] alone leaves
    /// the socket open until the other end closes it.
    socket: Option<std::net::TcpStream>,
    connections: u32,
    /// Debug cycles over the cables that have gone; the one on adds its own.
    cycles: u64,
}

enum End<E: CableEnd> {
    Free(E),
    Cabled(Remote<E>),
}

/// What [`Connector::attend`] found at the listener.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Plug {
    /// A debugger connected from here and is on the cable.
    Connected(SocketAddr),
    /// A debugger connected from here while one was on the cable, and its
    /// connection was closed: one cable per connector.
    Refused(SocketAddr),
}

/// What one turn of [`Connector::step_cabled`] did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Turn {
    /// The machine stepped.
    Stepped,
    /// The machine could not step yet, and a message was waited for.
    Waited,
    /// The cable went --- the debugger said it is done, or its stream went
    /// away, which is what the text says --- and the machine runs on its
    /// own from here, the listener open.
    Unplugged(String),
}

impl<E: CableEnd> Connector<E> {
    /// The connector on `machine`'s DBGIN, with `listener` at it; none is
    /// the machine alone.  The listener is made non-blocking here, for
    /// [`Connector::attend`].
    pub fn new(machine: E, listener: Option<TcpListener>) -> io::Result<Connector<E>> {
        if let Some(l) = &listener {
            l.set_nonblocking(true)?;
        }
        Ok(Connector {
            end: Some(End::Free(machine)),
            listener,
            debugger: None,
            socket: None,
            connections: 0,
            cycles: 0,
        })
    }

    /// Where the listener is, if there is one.
    pub fn addr(&self) -> Option<SocketAddr> {
        self.listener.as_ref().and_then(|l| l.local_addr().ok())
    }

    fn end(&self) -> &End<E> {
        self.end.as_ref().expect("the machine is at one end or the other")
    }

    fn end_mut(&mut self) -> &mut End<E> {
        self.end.as_mut().expect("the machine is at one end or the other")
    }

    pub fn machine(&self) -> &E {
        match self.end() {
            End::Free(e) => e,
            End::Cabled(r) => &r.machine,
        }
    }

    pub fn machine_mut(&mut self) -> &mut E {
        match self.end_mut() {
            End::Free(e) => e,
            End::Cabled(r) => &mut r.machine,
        }
    }

    /// Where the debugger on the cable connected from; `None` with nobody
    /// on it, when the machine is the caller's to step.
    pub fn debugger(&self) -> Option<SocketAddr> {
        self.debugger
    }

    /// Debuggers that have been on the cable, the one on it included.
    pub fn connections(&self) -> u32 {
        self.connections
    }

    /// `DEBUG_CYCLE` requests taken over the cable, every debugger's.
    pub fn debug_cycles(&self) -> u64 {
        self.cycles
            + match self.end() {
                End::Free(_) => 0,
                End::Cabled(r) => r.debug_cycles(),
            }
    }

    /// The listener looked at, once: a debugger connecting is plugged in
    /// and the machine is on the cable from here, or is refused while one
    /// is on already.  Nothing waits: with nobody connecting this returns
    /// at once, and it is for between two microcycles.
    pub fn attend(&mut self) -> Option<Plug> {
        let listener = self.listener.as_ref()?;
        let (stream, from) = match listener.accept() {
            Ok(accepted) => accepted,
            // `WouldBlock` is nobody connecting; anything else is the
            // host's for this turn, and the next turn looks again.
            Err(_) => return None,
        };
        if self.debugger.is_some() {
            drop(stream);
            return Some(Plug::Refused(from));
        }
        // The accepted stream blocks, as the cable's reader wants: whether
        // it inherits the listener's non-blocking flag differs between
        // systems, so it is set either way.  A stream that cannot be set
        // up is dropped, which the debugger sees as the cable going away.
        if stream.set_nonblocking(false).is_err() || stream.set_nodelay(true).is_err() {
            return None;
        }
        let (Ok(reader), Ok(socket)) = (stream.try_clone(), stream.try_clone()) else {
            return None;
        };
        let End::Free(machine) = self.end.take().expect("the machine is at one end or the other")
        else {
            unreachable!("a debugger on the cable and none")
        };
        self.end = Some(End::Cabled(Remote::debuggee(machine, reader, stream)));
        self.socket = Some(socket);
        self.debugger = Some(from);
        self.connections += 1;
        Some(Plug::Connected(from))
    }

    /// One turn with the debugger on the cable: a step of the machine as
    /// far as the debugger's promise allows, or a wait for its next
    /// message ([`Remote::step`]).  A debugger that has said it is done, or
    /// whose stream has gone --- or sent what it had no business sending
    /// --- is unplugged, and the turn says so; from there the machine is
    /// the caller's to step until the next debugger comes.  Not for a
    /// connector with nobody on it.
    pub fn step_cabled(&mut self) -> Result<Turn, Halt> {
        let from = self.debugger.expect("a debugger is on the cable");
        let End::Cabled(r) = self.end_mut() else { unreachable!("a debugger is on the cable") };
        let turn = match r.step() {
            Ok(true) => Turn::Stepped,
            Ok(false) => Turn::Waited,
            Err(Error::Halt(h)) => return Err(h),
            Err(Error::Io(e)) => return Ok(self.unplug(format!("the debugger at {from}: {e}"))),
        };
        if r.peer_done() {
            return Ok(self.unplug(format!("the debugger at {from} is done")));
        }
        Ok(turn)
    }

    /// The cable pulled: [`Remote::hang_up`], the socket shut down behind
    /// it so that both ends' readers see it go, and the machine on its own.
    fn unplug(&mut self, why: String) -> Turn {
        let End::Cabled(r) = self.end.take().expect("the machine is at one end or the other")
        else {
            unreachable!("a debugger is on the cable")
        };
        self.cycles += r.debug_cycles();
        self.end = Some(End::Free(r.hang_up()));
        if let Some(socket) = self.socket.take() {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
        self.debugger = None;
        Turn::Unplugged(why)
    }

    /// The run's end: a debugger on the cable is told and its own end
    /// waited for, as [`Remote::finish`]; with nobody on it, nothing.
    pub fn finish(&mut self) -> Result<(), Error> {
        match self.end_mut() {
            End::Free(_) => Ok(()),
            End::Cabled(r) => r.finish(),
        }
    }
}

// --- The cable to an end with no clock: the third transport ------------------

/// The debugger and a debuggee that keeps no clock of muir's, with the
/// cable between them: the driver for the fabric's register window
/// ([`crate::fabric::Fabric`]), and the smallest of the three.
///
/// **Only the debugger is stepped and nothing is promised either way.**
/// [`Lashup`] and [`Remote`] keep two simulated clocks in step by
/// exchanging the earliest instant each can next do anything; a debuggee
/// in fabric runs on its own crystal in real time and cannot be asked to
/// wait, so there is nothing to exchange and nothing of it to step.  The
/// debugger's own interface is what ends a cycle nothing answers: it times
/// out [`crate::busint::DEBUG_TIMEOUT_NS`] after the grant and puts the
/// release on the cable itself, which bounds the polling at about fifty
/// steps.
///
/// **The debuggee is polled as soon as the request is placed**, before the
/// next step, because the three register strobes are acknowledged
/// combinationally --- `DEBUG ACK` is `(DBUB MASTER AND SSYN T0) OR
/// NAND(-DB ADR1 CLK, -DB ADR0 CLK, -DB READ STATUS)`, the 74S08, 74S10
/// and 74S32 at DBGIN 0A12, 0A14 and 0A09 --- so their acknowledgement is
/// already up when the first load lands, and a strobe costs one store, one
/// load and one store.
///
/// Any debuggee [`CableEnd`] will do, which is what makes the transport
/// checkable with no fabric and no board: `tests/fabric.rs` runs this
/// driver against a model of the window with the netlist's own DBGIN
/// connector behind it ([`crate::cable::DebugIn`]).
pub struct FreeRunning<D: CableEnd> {
    pub debugger: Rtl,
    pub debuggee: D,
    /// The standing request's answer has been carried to the debugger, or
    /// there is no request standing.
    carried: bool,
    /// Steps of the debugger taken, for the curious.
    pub steps: u64,
}

impl<D: CableEnd> FreeRunning<D> {
    /// Plugs the cable into the debugger's DBGOUT: a cycle into its debug
    /// block waits for a real acknowledgement from here on, instead of the
    /// pull-up's.
    pub fn new(mut debugger: Rtl, debuggee: D) -> FreeRunning<D> {
        debugger.attach_debug_cable();
        FreeRunning { debugger, debuggee, carried: true, steps: 0 }
    }

    /// One step of the debugger, what it put on the cable carried to the
    /// debuggee, and the debuggee's answer carried back.
    pub fn step(&mut self) -> Result<(), Error> {
        let limit = self.debugger.ns().saturating_add(max_step_ns());
        self.debugger.step_until(limit)?;
        self.steps += 1;
        let refused = |what: String| Error::Io(io::Error::new(io::ErrorKind::InvalidData, what));
        if let Some(event) = self.debugger.debug_out_take() {
            match event {
                CableEvent::Request { at, request } => {
                    self.debuggee.debug_request(at, request).map_err(refused)?;
                    self.carried = false;
                }
                CableEvent::Release { at } => {
                    self.debuggee.debug_release(at).map_err(refused)?;
                    self.carried = true;
                }
            }
        }
        if !self.carried
            && let Some((at, word)) = self.debuggee.debug_ack()
        {
            self.carried = true;
            self.debugger.debug_out_answer(at, word);
        }
        Ok(())
    }

    /// Runs the debugger to `ns` at least.
    pub fn run_until(&mut self, ns: u64) -> Result<(), Error> {
        while self.debugger.ns() < ns {
            self.step()?;
        }
        Ok(())
    }
}
