// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The debug cable's debuggee in FPGA fabric, reached through a window of
//! memory-mapped registers: the third transport.
//!
//! The other two carry the cable between two simulated machines, in one
//! process ([`crate::lashup::Lashup`]) or over a byte stream
//! ([`crate::lashup::Remote`]).  This one carries it to a CADR that is not
//! simulated at all: the machine is in the programmable logic of a Zynq
//! board, Linux runs on the Arm cores beside it, muir runs under that
//! Linux and is the debugger, and the cable's 21 wires are a handful of
//! 32-bit registers muir reaches with ordinary loads and stores.  A CADR
//! is debugged by another CADR, so muir plays the second machine.
//!
//! What this end has to do is [`crate::lashup::CableEnd`]'s debuggee half,
//! and no more: take the request the debugger's DBGOUT put on the cable,
//! give back `DEBUG IN ACK` and the word on `DBD<15:0>`.
//! [`crate::lashup::FreeRunning`] is the driver that pairs it with an
//! [`Rtl`](crate::rtl::Rtl) debugger.
//!
//! **Nothing is promised in either direction.**  The other two transports
//! keep the two machines' simulated clocks in step by exchanging the
//! earliest instant each can next do anything; fabric runs on its own
//! crystal in real time and cannot be asked to wait, so
//! [`Fabric::debug_in_promise`] is `u64::MAX` --- this end never holds the
//! debugger back --- and the debugger's own timeout is what ends a cycle
//! nothing answers.  The two run free of each other, which is what two
//! real CADRs on a bench did.
//!
//! # The register window
//!
//! Sixteen words, 64 bytes, at the physical address
//! `--debug-cable-connect 0x…` gives.  Five are used and the rest read
//! [`UNMAPPED`].
//!
//! | word | | |
//! |---|---|---|
//! | 0 | [`IDENT`] | read-only, [`DBUG`] |
//! | 1 | [`CTL`] | the request, whole, in one store |
//! | 2 | [`STS`] | the answer, in one load |
//! | 3 | [`CLEAR`] | the key [`LIFT`] lifts any standing request |
//! | 4 | [`FAULTS`] | a count and three sticky faults |
//! | 5-15 | | [`UNMAPPED`] |
//!
//! **This layout is proposed and unbuilt, and so unverified.**  Nothing
//! exists in fabric yet: the word count, the bit positions, the marker's
//! position and the sequence number's width are choices and not
//! constraints, and whoever builds the adapter confirms or corrects them
//! rather than inheriting them.  What would settle them is a bitstream
//! with the DBGIN end in it and a run of muir against the board; until
//! then the only check on any of it is the one in `tests/fabric.rs`, which
//! puts a model of the window in front of the netlist's own DBGIN
//! connector.  **The identity and the clear key are the exception**: those
//! two are confirmed, see [`DBUG`].
//!
//! The whole request crosses in one store, which is the point of the
//! layout.  The debuggee decodes `DEBUG IN A<1:0>` combinationally while
//! `-DEBUG IN REQ` is down --- the 74S139 at DBGIN 0A15 --- and its
//! address and modifier latches take `DBD` at the strobe's trailing edge,
//! so a carrier that lets the request arrive before the levels decodes the
//! wrong strobe, and one that clears the levels as it lifts latches the
//! wrong word.  Storing the levels and the request bit together leaves no
//! ordering for either to get wrong, and the adapter is asked to reproduce
//! the board's own hundred nanoseconds of lead inside the fabric, where it
//! is free ([`crate::busint::DEBUG_OUT_REQUEST_NS`]).

use std::cell::{Cell, RefCell};

use crate::busint::DebugRequest;
use crate::lashup::CableEnd;

// --- The layout ------------------------------------------------------------

/// The window's words: sixteen, 64 bytes.
pub const WORDS: usize = 16;

/// Word 0, `IDENT`: read-only, and [`DBUG`] on the adapter.  The first
/// thing muir loads and the one it refuses on, because a store into a
/// window that is some other device's registers is the one failure that
/// damages something.
pub const IDENT: usize = 0;

/// Word 1, `CTL`: the request, whole, in one store --- [`REQ`], [`WR`],
/// [`A_SHIFT`], [`SEQ_SHIFT`] and [`DBD_SHIFT`] together.  It reads back
/// what the adapter is holding, which closes the question of whether a
/// store landed.  Lifting is a second store of the same word with [`REQ`]
/// cleared and every other field unchanged: the debuggee's latches take
/// `DBD` at the lift.
pub const CTL: usize = 1;

/// Word 2, `STS`: the answer, in one load, so that the acknowledgement and
/// the word cannot be read at two different instants --- [`REQ`], [`ACK`],
/// [`DRV`], [`SEQ_SHIFT`], [`DBD_SHIFT`] and [`MARK`].
pub const STS: usize = 2;

/// Word 3, `CLEAR`: a store of [`LIFT`], and of nothing else, lifts any
/// standing request and clears [`FAULTS`] and the sequence.  muir stores
/// it once as it opens the window, because a fresh muir must not inherit a
/// half-finished transaction from a run that was killed.
pub const CLEAR: usize = 3;

/// Word 4, `FAULTS`: diagnostic, cleared by [`CLEAR`], carrying the same
/// [`MARK`] as [`STS`] so that one guard serves both ---
/// [`FAULT_WATCHDOG`], [`FAULT_ON_REQUEST`], [`FAULT_IDLE_LIFT`] and
/// [`COUNT_SHIFT`].
pub const FAULTS: usize = 4;

/// What words 5 to 15 read: the complement of [`DBUG`], so that a window
/// answering with it cannot be mistaken for one of the five that mean
/// something.
pub const UNMAPPED: u32 = 0xBBBD_AAB8;

/// `CTL<0>`, the request as muir drives it, and `STS<0>`, the request the
/// adapter is holding: set is `-DEBUG IN REQ` asserted, the request down.
pub const REQ: u32 = 1;

/// `CTL<1>`: `DEBUG IN WR`.  `C1 OUT` on the debuggee's Unibus and the
/// direction its transceivers face, so a write flag that moved inside a
/// request would invert the cycle; it crosses in the one store with
/// everything else and cannot.
pub const WR: u32 = 1 << 1;

/// `CTL<3:2>`: `DEBUG IN A<1:0>`, which of the four strobes the debuggee's
/// 74S139 makes --- [`crate::busint::DEBUG_CYCLE`] to
/// [`crate::busint::DEBUG_ADDRESS`].
pub const A_SHIFT: u32 = 2;

/// `STS<1>`, `ACK`: `DEBUG IN ACK` has risen for the request now standing.
/// Latched by the adapter at the instant it first rose, with [`DBD_SHIFT`]
/// and [`DRV`] beside it, and held until the request is lifted: by the
/// time the Arm gets round to loading, the word the debuggee drove may be
/// long gone.
pub const ACK: u32 = 1 << 1;

/// `STS<3>`, `DRV`: the debuggee drove at least one data line.  On the
/// wire "drove nothing" and "drove all ones" are the same thing and muir
/// takes `None` for `0xffff`; in fabric the adapter knows which side is
/// driving, so the bit is free and is reported honestly.
pub const DRV: u32 = 1 << 3;

/// `CTL<11:8>` and `STS<11:8>`: this transaction's sequence number, four
/// bits.  muir increments it on every request and the adapter reports the
/// sequence of the request it is holding, so that an acknowledgement left
/// standing from a previous transaction --- which reads exactly like an
/// answer to the present one --- is caught.  [`crate::cable::DebugIn`]
/// guards the same hazard on the simulated side.
pub const SEQ_SHIFT: u32 = 8;

/// The sequence number's four bits, once shifted down.
pub const SEQ_MASK: u32 = 0xf;

/// `CTL<31:16>` and `STS<31:16>`: `DBD<15:0>`, as the debugger drives them
/// and as the debuggee drove them.
pub const DBD_SHIFT: u32 = 16;

/// `STS<15:14>` and `FAULTS<15:14>`: `MARK`, which reads `01` always while
/// the adapter answers.  A word of all zeros gives `00` and one of all
/// ones gives `11`, so neither can be mistaken for an answer; it has to
/// live outside `DBD`, because all ones there is what an open cable reads
/// and is a perfectly legal answer.  muir applies the guard to every load
/// and not only the first.
pub const MARK: u32 = 1 << 14;

/// The two bits [`MARK`] stands in.
pub const MARK_MASK: u32 = 3 << 14;

/// `FAULTS<0>`: the adapter's watchdog lifted a request that had stood
/// longer than its dead-man interval.  A request left standing holds `-DB
/// NEED UB` down on the debuggee, which keeps the debug master on its
/// Unibus with `-UB BBSY` asserted and the machine's own cycles waiting
/// for ever, so a muir that is killed mid-transaction has to be recovered
/// from by the fabric; on a real lashup unplugging the cable does it, the
/// SIP at DBGIN 0A22 pulling `-DEBUG IN REQ` up.
pub const FAULT_WATCHDOG: u32 = 1;

/// `FAULTS<1>`: a request was stored while one was still standing.
pub const FAULT_ON_REQUEST: u32 = 1 << 1;

/// `FAULTS<2>`: a lift was stored with no request standing.
pub const FAULT_IDLE_LIFT: u32 = 1 << 2;

/// `FAULTS<31:16>`: requests the adapter has taken, saturating at
/// `0xffff`.  It answers a question the other words cannot --- whether
/// anything is happening at all --- since a window that gives a plausible
/// identity and a plausible status but whose count never moves is a fabric
/// that is not taking the stores.
pub const COUNT_SHIFT: u32 = 16;

/// [`IDENT`]'s value: `"DBUG"` as a 32-bit register, `'D'` in the most
/// significant byte.
///
/// **Compared as a `u32`, never as bytes.**  These are 32-bit registers on
/// a 32-bit port, so a 32-bit load gives the register's value unchanged
/// and byte order never enters; it would enter only if the word were
/// stored to memory and read back a byte at a time, which nothing does.
/// On the little-endian Arm cores the same word written to memory reads
/// `GUBD`, so a comparison against a string would be wrong in a way that
/// looks right.  The four letters are printed most significant first,
/// which is the order they are meant in, and that is the only place byte
/// order appears.
///
/// **This one is confirmed, not proposed**: the fabric project presents
/// `IDENT` exactly so, and [`LIFT`] the same way.  Three of its faces
/// already use the convention --- `CONS` the console, `PACK` the disk pack
/// side, `NONE` a general-purpose port brought out with nothing behind it
/// --- and [`Fabric::open`] names them in its refusal.
pub const DBUG: u32 = 0x4442_5547;

/// [`CLEAR`]'s key: `"LIFT"`, `'L'` in the most significant byte, and
/// compared as a `u32` for the reason [`DBUG`] gives.  Four distinct
/// bytes, none of them `00` or `FF`, halves that differ and are not
/// rotations of each other, and not [`DBUG`] and not [`UNMAPPED`], so an
/// arbitrary value landing there cannot lift a live request.
pub const LIFT: u32 = 0x4C49_4654;

/// The fabric's other register faces, which identify themselves the same
/// way, so that a refusal can say which one answered instead: the letters
/// name the face and the fix is a different address.  `NONE` is the
/// default slave a general-purpose port gives when it is brought out with
/// nothing of ours behind it.
const FACES: [(u32, &str); 3] = [
    (0x434F_4E53, "the console"),
    (0x5041_434B, "the disk pack side"),
    (0x4E4F_4E45, "the default slave: a port with nothing of ours behind it"),
];

/// A window word's four bytes as letters, most significant first, which is
/// the order a four-letter identity is meant in; anything not printable is
/// a dot.
fn letters(word: u32) -> String {
    word.to_be_bytes().iter().map(|&b| if b.is_ascii_graphic() { b as char } else { '.' }).collect()
}

// --- The seam --------------------------------------------------------------

/// The register window as the loads and stores that reach it, and nothing
/// else: the seam between the layout above, which is portable, and the
/// mapping below, which is Linux's.  [`Mapped`] maps `/dev/mem`; [`Words`]
/// is an array, for a machine with no fabric under it.
///
/// **Both calls take `&self`.**  A window is a device and not memory: what
/// a load gives changes with nothing on this side touching it, since the
/// fabric behind it runs on its own crystal, and a store is a write to a
/// register this side does not own either.  `/dev/mem` hands out one raw
/// pointer for both, and [`Words`] keeps its state in a [`Cell`] for the
/// same reason.
pub trait Window {
    /// The 32-bit register `word` words into the window.
    fn load(&self, word: usize) -> u32;
    /// `value` into it.
    fn store(&self, word: usize, value: u32);
}

/// A borrowed window is a window: a device is reached through a shared
/// reference either way, and a caller that keeps the window to watch what
/// muir stored into it lends it rather than giving it up.
impl<W: Window> Window for &W {
    fn load(&self, word: usize) -> u32 {
        W::load(*self, word)
    }
    fn store(&self, word: usize, value: u32) {
        W::store(*self, word, value)
    }
}

/// The window as an array of words, for a muir with no fabric under it:
/// the seam's other side, and what `tests/fabric.rs` drives the layout
/// with.
///
/// Nothing here is read-only and no store does anything but land: the
/// adapter's own behaviour is the caller's to play, which is what makes it
/// useful --- a test can leave a stale acknowledgement standing, or change
/// the data lines after raising [`ACK`], and see what muir makes of it.
pub struct Words(Cell<[u32; WORDS]>);

impl Default for Words {
    fn default() -> Words {
        Words::new()
    }
}

impl Words {
    /// The window as the adapter reads at rest: the identity, an idle
    /// status and no faults, both marked, and [`UNMAPPED`] in the eleven
    /// words that are not registers.
    pub fn new() -> Words {
        let mut w = [UNMAPPED; WORDS];
        w[IDENT] = DBUG;
        w[CTL] = 0;
        w[STS] = MARK;
        w[CLEAR] = 0;
        w[FAULTS] = MARK;
        Words(Cell::new(w))
    }

    /// One word, as the adapter would have it.
    pub fn get(&self, word: usize) -> u32 {
        self.0.get()[word]
    }

    /// One word put where the adapter would have put it.
    pub fn set(&self, word: usize, value: u32) {
        let mut w = self.0.get();
        w[word] = value;
        self.0.set(w);
    }
}

impl Window for Words {
    fn load(&self, word: usize) -> u32 {
        self.get(word)
    }
    fn store(&self, word: usize, value: u32) {
        self.set(word, value);
    }
}

// --- The end ---------------------------------------------------------------

/// The request muir has at the window: the instant the debugger's DBGOUT
/// put it on the cable, and the word stored to [`CTL`] for it, which the
/// lift stores again with [`REQ`] cleared.
#[derive(Clone, Copy)]
struct Standing {
    at: u64,
    ctl: u32,
}

/// Something at the window muir cannot account for, which is never taken
/// as data.
#[derive(Clone, Debug)]
pub struct Fault {
    /// What it was, in the window's own numbers.
    pub what: String,
    /// Whether the run ends here.  A window that no longer answers as the
    /// adapter does end it: there is nothing left to store into and
    /// nothing it gives can be believed.  An adapter that is not holding
    /// muir's request does not: it costs the cycle, which the debugger
    /// times out as it times out a cycle nothing answers, and the next
    /// request begins again with a sequence of its own.  That is what
    /// makes the adapter's watchdog ([`FAULT_WATCHDOG`]) safe to have.
    pub fatal: bool,
}

/// The debuggee's DBGIN in FPGA fabric, as a [`CableEnd`]: the debuggee
/// half and nothing else.  [`Fabric::open`] checks the identity and clears
/// the window before any of it.
///
/// **`hold_ns` has no counterpart here and is dropped.** It exists so that
/// a simulated debuggee can schedule the lift itself; the fabric lifts
/// when muir stores the lift, which muir does the moment it has the
/// answer.
///
/// **The acknowledgement is dated at the request's own instant, and not at
/// the debugger's clock when the status was read.**  In the other two
/// transports the instant comes from the other machine, whose simulated
/// clock is kept in step with this one; fabric has no simulated clock to
/// offer, so there are only these two to choose between, and this is the
/// one place this transport parts company with the other two.
///
/// The reason is that the other instant is not a property of either
/// machine.  muir's clock does not advance while a load or a store is in
/// flight, but it does advance between polls, so dating the answer where
/// the debugger stands makes a debug cycle as long as the driver happened
/// to step while waiting --- a number that says how busy the host's core
/// was.  Dating it at the request makes the cycle take its nominal Unibus
/// time, which is the more faithful of the two, and is what the boards
/// would have taken.
///
/// **It buys nothing from the timeout, and that is worth saying because it
/// looks as though it should.**  A late answer is refused either way:
/// [`crate::busint::Busint::debug_out_answer`] refuses one with no request
/// outstanding, and the interface clears the request as its own clock
/// reaches the timeout, in the step before any later poll --- so its other
/// guard, an instant at or past the timeout, is never what decides.
/// `tests/fabric.rs` holds both ends of this: an answer twenty polls late
/// is taken and its word lands, and one past the interface's 11.05
/// microseconds is not taken at all and the cycle reads the open cable.
///
/// **What it costs** is a completion instant the interface has already
/// passed, by thousands of nanoseconds where [`crate::lashup::Lashup`] and
/// [`crate::lashup::Remote`] can be at most a generator cycle behind.
/// That is answered rather than assumed: `Granted` becomes `Acked` on a
/// plain `now >= ack`, so `-LMACK` and `-LOADMD` simply fall due at once
/// and the word lands at the next look; and the machine clock the
/// completion sets back --- the one the I/O board's clocks count ---
/// is put forward to the engine's own at the head of the next microcycle,
/// before anything is advanced with it.
///
/// The consequence either way is that no timing measured across this cable
/// means anything, and nothing should compare a debug cycle's length here
/// against the lashup's.  CC does not measure them.
pub struct Fabric<W: Window> {
    window: W,
    /// The sequence number of the last request stored, four bits.  The
    /// [`CLEAR`] at the open leaves the adapter reporting zero, so the
    /// first request is 1 and a reply that answers nothing muir asked is
    /// caught from the start.
    seq: u8,
    /// The request at the window, until it is answered or lifted.
    standing: Cell<Option<Standing>>,
    /// The answer taken for it, latched at the first load of [`STS`] that
    /// saw [`ACK`] --- so that asking twice gives the same answer and
    /// costs no second load, and so that the word reported is the one the
    /// adapter latched at the acknowledgement rather than whatever stands
    /// at some later load.
    answer: Cell<Option<(u64, Option<u16>)>>,
    /// What the window has done that muir cannot account for, kept until
    /// [`Fabric::fault`] is asked.  The first is kept: the ones after it
    /// are usually the same one seen again on the next poll.
    fault: RefCell<Option<Fault>>,
}

impl<W: Window> Fabric<W> {
    /// The window checked and cleared, or why it is no debug cable.
    ///
    /// [`IDENT`] is loaded before anything else and the window is refused
    /// unless it reads [`DBUG`], because a store into some other device's
    /// registers is the one failure that damages something.  A refusal
    /// names what it found, since each case points somewhere different:
    /// four printable letters mean another face answered and the fix is a
    /// different address, all zeros or all ones mean nothing is behind the
    /// window or nothing is driving it, and anything else is a window muir
    /// cannot account for.  In every case muir stops: it does not fall
    /// back to the network endpoint, does not retry, and does not go on to
    /// store anything.
    ///
    /// Then the key to [`CLEAR`], so that a fresh muir inherits no
    /// half-finished transaction from a run that was killed, and a load of
    /// [`STS`] to see the marker and an idle adapter.
    ///
    /// **This does not protect against the wrong address, only the wrong
    /// face.**  A load from a window that no fabric slave answers does not
    /// fault; it hangs the core, and no guard in muir can catch that.  It
    /// is measured on the board by the fabric project, which is also why
    /// the ports that are brought out with nothing behind them answer
    /// `NONE` rather than hanging.
    pub fn open(window: W) -> Result<Fabric<W>, String> {
        let ident = window.load(IDENT);
        if ident != DBUG {
            return Err(Self::refuse(ident));
        }
        let f = Fabric {
            window,
            seq: 0,
            standing: Cell::new(None),
            answer: Cell::new(None),
            fault: RefCell::new(None),
        };
        f.window.store(CLEAR, LIFT);
        let sts = f.window.load(STS);
        if sts & MARK_MASK != MARK {
            return Err(format!(
                "the window identified itself as the debug cable's and then read {sts:#010x} for \
                 STS, whose MARK bits are not 01: it is not answering as the adapter"
            ));
        }
        if sts & REQ != 0 {
            return Err(format!(
                "the window still holds a request after the clear: STS {sts:#010x}"
            ));
        }
        Ok(f)
    }

    /// Why a window whose [`IDENT`] is not [`DBUG`] is refused, in its own
    /// terms.
    fn refuse(ident: u32) -> String {
        let hex = format!("{ident:#010x}");
        if ident == 0 || ident == u32::MAX {
            let what = if ident == 0 { "all zeros" } else { "all ones" };
            return format!(
                "the window at this address reads {what} for IDENT, which is what a window reads \
                 with nothing behind it or nothing driving it, and is no identity"
            );
        }
        if let Some((_, what)) = FACES.iter().find(|(v, _)| *v == ident) {
            return format!(
                "the window at this address is {what}: IDENT reads {hex}, \"{}\", and the debug \
                 cable's adapter reads {DBUG:#010x}, \"{}\"",
                letters(ident),
                letters(DBUG)
            );
        }
        if ident.to_be_bytes().iter().all(|b| b.is_ascii_graphic()) {
            return format!(
                "another register face answered at this address: IDENT reads {hex}, \"{}\", and \
                 the debug cable's adapter reads {DBUG:#010x}, \"{}\"",
                letters(ident),
                letters(DBUG)
            );
        }
        format!(
            "the window at this address reads {hex} for IDENT, which is no face muir can account \
             for; the debug cable's adapter reads {DBUG:#010x}, \"{}\"",
            letters(DBUG)
        )
    }

    /// The window itself, for a caller that models the adapter behind it.
    pub fn window(&self) -> &W {
        &self.window
    }

    /// What the window has done that muir cannot account for, taken: a
    /// [`MARK`] that is not `01`, which says the window has died, or a
    /// status that is not the adapter holding muir's own request.  Neither
    /// is treated as data; [`Fault::fatal`] says which ends the run.
    pub fn fault(&self) -> Option<Fault> {
        self.fault.borrow_mut().take()
    }

    /// Requests the adapter says it has taken, [`COUNT_SHIFT`] of
    /// [`FAULTS`], with the fault bits beside it; `None` if [`FAULTS`]
    /// does not carry the marker.
    pub fn taken(&self) -> Option<(u32, u32)> {
        let f = self.window.load(FAULTS);
        let bits = FAULT_WATCHDOG | FAULT_ON_REQUEST | FAULT_IDLE_LIFT;
        (f & MARK_MASK == MARK).then_some((f >> COUNT_SHIFT, f & bits))
    }

    /// Keeps the first thing that went wrong.
    fn blame(&self, fatal: bool, what: String) {
        let mut held = self.fault.borrow_mut();
        if held.is_none() {
            *held = Some(Fault { what, fatal });
        }
    }
}

impl<W: Window> Drop for Fabric<W> {
    /// A request left standing holds `-DB NEED UB` down on the debuggee
    /// and its own cycles wait for ever, so a muir that stops with one at
    /// the window lifts it on the way out.  The adapter's watchdog
    /// ([`FAULT_WATCHDOG`]) is the backstop for a muir that never reaches
    /// here.
    fn drop(&mut self) {
        if let Some(s) = self.standing.take() {
            self.window.store(CTL, s.ctl & !REQ);
        }
    }
}

impl<W: Window> CableEnd for Fabric<W> {
    /// One store of [`CTL`], carrying [`REQ`], [`WR`], the strobe, the
    /// sequence and `DBD` together.  `-DEBUG OUT REQ` is one level, so a
    /// debugger has one request out at a time and a second while one
    /// stands is the caller's error.
    fn debug_request(&mut self, at: u64, request: DebugRequest) -> Result<(), String> {
        if self.standing.get().is_some() {
            return Err(format!("a debug request at {at} ns while one is already at the window"));
        }
        self.seq = (self.seq + 1) & SEQ_MASK as u8;
        let ctl = REQ
            | if request.write { WR } else { 0 }
            | ((request.strobe as u32 & 3) << A_SHIFT)
            | ((self.seq as u32) << SEQ_SHIFT)
            | ((request.dbd as u32) << DBD_SHIFT);
        self.answer.set(None);
        self.standing.set(Some(Standing { at, ctl }));
        self.window.store(CTL, ctl);
        Ok(())
    }

    /// The debugger gave up: the request word stored again with [`REQ`]
    /// cleared and every other field unchanged, since the debuggee's
    /// latches take `DBD` at the lift.  A release with nothing standing is
    /// nothing.
    fn debug_release(&mut self, _at: u64) -> Result<(), String> {
        if let Some(s) = self.standing.take() {
            self.window.store(CTL, s.ctl & !REQ);
        }
        Ok(())
    }

    /// One load of [`STS`], guarded by [`MARK`] and by the sequence, and
    /// the request lifted the moment it is answered.
    ///
    /// The marker is checked on every load and not only the first: a
    /// window that has died reads all zeros or all ones, and `DBD` alone
    /// cannot tell muir so, all ones there being what an open cable reads.
    /// A status that does not carry muir's own sequence, or that says the
    /// adapter is holding nothing, is not muir's answer --- an
    /// acknowledgement left standing from a previous transaction reads
    /// exactly like an answer to this one --- and is a fault and not data.
    ///
    /// The answer is latched here, so that the word reported is the one
    /// the adapter took at the acknowledgement and not whatever stands at
    /// a later load, and so that asking twice costs no second load.
    ///
    /// The instant it carries is the request's own and not the debugger's
    /// clock at this load; [`Fabric`] says which, why, and what it costs.
    fn debug_ack(&self) -> Option<(u64, Option<u16>)> {
        if let Some(answer) = self.answer.get() {
            return Some(answer);
        }
        let standing = self.standing.get()?;
        let sts = self.window.load(STS);
        if sts & MARK_MASK != MARK {
            self.blame(
                true,
                format!(
                    "STS read {sts:#010x}, whose MARK bits are not 01: the window has stopped \
                     answering as the adapter"
                ),
            );
            return None;
        }
        if sts & REQ == 0 || (sts >> SEQ_SHIFT) & SEQ_MASK != self.seq as u32 {
            let faults = self.window.load(FAULTS);
            self.blame(
                false,
                format!(
                    "STS read {sts:#010x}, which is not the adapter holding muir's request {}: \
                     FAULTS {faults:#010x}",
                    self.seq
                ),
            );
            return None;
        }
        if sts & ACK == 0 {
            return None;
        }
        let answer = (standing.at, (sts & DRV != 0).then_some((sts >> DBD_SHIFT) as u16));
        self.answer.set(Some(answer));
        self.window.store(CTL, standing.ctl & !REQ);
        self.standing.set(None);
        Some(answer)
    }

    /// Nothing.  The fabric runs on its own crystal in real time and
    /// cannot be asked to wait for the debugger, so this end never holds
    /// the debugger back and promises no instant it will not have passed.
    fn debug_in_promise(&self) -> u64 {
        u64::MAX
    }
}

// --- The mapping -----------------------------------------------------------

/// The window at physical address `at`, mapped, checked and cleared:
/// [`Mapped::open`] and then [`Fabric::open`].
pub fn open(at: u64) -> Result<Fabric<Mapped>, String> {
    Fabric::open(Mapped::open(at)?)
}

/// Why this build cannot reach a window at all, if it cannot: the mapping
/// is `/dev/mem`, which only Linux has, and muir is developed on macOS.
/// The command line asks this before it asks for an address, so that a
/// `--debug-cable-connect 0x…` on a machine that cannot do it is refused
/// by name.
pub fn unmappable() -> Option<&'static str> {
    #[cfg(target_os = "linux")]
    {
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        Some(NOT_LINUX)
    }
}

/// What a build with no `/dev/mem` says.
#[cfg(not(target_os = "linux"))]
const NOT_LINUX: &str = "the fabric's register window is reached through /dev/mem, which only \
                         Linux has, and this muir was not built for Linux; the debugger has to \
                         be muir running on the board's own processor";

/// The window mapped out of `/dev/mem`: the page it begins in, mapped
/// read-write and shared, with the window's first word inside it.
///
/// The mapping is four calls of the C library declared here --- `open`,
/// `mmap64`, `munmap`, `close` --- as `signal` and `localtime_r` are
/// elsewhere, muir having no dependencies.  The descriptor is closed as
/// soon as the mapping is made: the mapping outlives it.
#[cfg(target_os = "linux")]
pub struct Mapped {
    /// What `mmap` gave, for `munmap`.
    page: *mut std::ffi::c_void,
    /// How much was mapped, for `munmap`.
    len: usize,
    /// The window's word 0 inside the mapping.
    base: *mut u32,
}

#[cfg(target_os = "linux")]
mod sys {
    use std::ffi::{c_char, c_int, c_void};

    // `open` is variadic in C --- the third argument is the mode, and
    // only `O_CREAT` takes one --- and is declared so here: the compiler
    // refuses a plainer signature for a symbol the standard library also
    // uses.
    //
    // **`mmap64` rather than `mmap`.**  The board's Arm cores are 32-bit,
    // where `mmap`'s `off_t` is a signed 32-bit long: a window above 2 GB
    // could not be named at all, and the second general-purpose port ---
    // one of the two the fabric's registers sit on --- is at
    // `0x80000000`.  `mmap64` takes a 64-bit offset, and on a 64-bit
    // build it is the same call under another name.  **Unverified**:
    // nothing here has been linked against the board's own C library yet,
    // and a link that cannot find the symbol is what would say so.
    unsafe extern "C" {
        pub fn open(path: *const c_char, flags: c_int, ...) -> c_int;
        pub fn mmap64(
            addr: *mut c_void,
            len: usize,
            prot: c_int,
            flags: c_int,
            fd: c_int,
            off: i64,
        ) -> *mut c_void;
        pub fn munmap(addr: *mut c_void, len: usize) -> c_int;
        pub fn close(fd: c_int) -> c_int;
    }
}

#[cfg(target_os = "linux")]
impl Mapped {
    /// Linux's `O_RDWR`.
    const O_RDWR: std::ffi::c_int = 2;

    /// Linux's `O_SYNC`, which is `__O_SYNC | O_DSYNC` --- `04000000 |
    /// 010000` --- and not `O_DSYNC` alone.
    ///
    /// **What it buys is unverified here.**  It is asked for because a
    /// register window needs an uncached mapping and `O_SYNC` is what
    /// `/dev/mem` is said to take for one on ARM Linux; that is read
    /// rather than measured.  What would settle it is a read on the board,
    /// with the flag and without, of a register the fabric changes under
    /// muir.  Issue 95's open point 4.
    const O_SYNC: std::ffi::c_int = 0o4_010_000;

    /// `PROT_READ | PROT_WRITE` and `MAP_SHARED`, and what `mmap` returns
    /// on failure.
    const PROT_READ_WRITE: std::ffi::c_int = 1 | 2;
    const MAP_SHARED: std::ffi::c_int = 1;
    const MAP_FAILED: *mut std::ffi::c_void = usize::MAX as *mut std::ffi::c_void;

    /// The page the offset is taken in.  **4 KB, unverified**: it is the
    /// page the board's Arm Linux is expected to have, and it is not asked
    /// for at run time.  What would settle it is `getconf PAGESIZE` on the
    /// board.  A kernel with larger pages costs nothing worse than a
    /// refusal: `mmap` is the one that checks its offset, so muir would
    /// say the mapping failed rather than read the wrong words.
    const PAGE: u64 = 4096;

    /// The page holding the window at physical address `at`, mapped.
    ///
    /// The address is not required to be page-aligned, which would be a
    /// needless restriction; it is required to be word-aligned, which is
    /// not, since the window is 32-bit registers and every access to it is
    /// one 32-bit load or store.
    pub fn open(at: u64) -> Result<Mapped, String> {
        if !at.is_multiple_of(4) {
            return Err(format!("{at:#x} is not a multiple of 4, and the window is 32-bit words"));
        }
        let page = at & !(Self::PAGE - 1);
        let off = (at - page) as usize;
        let bytes = off + WORDS * 4;
        let len = bytes.div_ceil(Self::PAGE as usize) * Self::PAGE as usize;
        if page > i64::MAX as u64 {
            return Err(format!("{at:#x} does not fit the off64_t mmap64 takes"));
        }
        // SAFETY: a path that lives past the call and no mode, `open`
        // taking one only for `O_CREAT`.
        let fd = unsafe { sys::open(c"/dev/mem".as_ptr(), Self::O_RDWR | Self::O_SYNC) };
        if fd < 0 {
            return Err(format!("/dev/mem: {}", std::io::Error::last_os_error()));
        }
        // SAFETY: a mapping the kernel places, of a length that fits the
        // pages asked for, on a descriptor open for reading and writing.
        let page_at = unsafe {
            sys::mmap64(
                std::ptr::null_mut(),
                len,
                Self::PROT_READ_WRITE,
                Self::MAP_SHARED,
                fd,
                page as i64,
            )
        };
        let failed = std::io::Error::last_os_error();
        // The mapping outlives the descriptor.
        // SAFETY: a descriptor this call opened and nothing else holds.
        unsafe { sys::close(fd) };
        if page_at == Self::MAP_FAILED {
            return Err(format!("/dev/mem at {page:#x}, {len} bytes: {failed}"));
        }
        Ok(Mapped {
            page: page_at,
            len,
            // SAFETY: `off` is inside the mapping by construction, and is
            // a multiple of 4, so the window's words are aligned.
            base: unsafe { page_at.cast::<u8>().add(off).cast::<u32>() },
        })
    }
}

#[cfg(target_os = "linux")]
impl Drop for Mapped {
    fn drop(&mut self) {
        // SAFETY: the mapping this made, unmapped once.
        unsafe { sys::munmap(self.page, self.len) };
    }
}

#[cfg(target_os = "linux")]
impl Window for Mapped {
    /// One 32-bit volatile load.  The mapping is Device memory on this
    /// processor and Device accesses are not reordered against each other,
    /// so volatile is enough and no barrier is proposed --- read out of
    /// how the mapping is made rather than measured, issue 95's open point
    /// 4 again.
    fn load(&self, word: usize) -> u32 {
        assert!(word < WORDS, "word {word} is past the window");
        // SAFETY: inside the mapping, aligned, and the mapping is live for
        // as long as this is.
        unsafe { self.base.add(word).read_volatile() }
    }

    /// One 32-bit volatile store.
    fn store(&self, word: usize, value: u32) {
        assert!(word < WORDS, "word {word} is past the window");
        // SAFETY: as [`Mapped::load`].
        unsafe { self.base.add(word).write_volatile(value) }
    }
}

/// The mapped window on a machine that has no `/dev/mem`: the type is here
/// so that everything above the seam builds and is tested on macOS, and it
/// cannot be made.
#[cfg(not(target_os = "linux"))]
pub struct Mapped(std::convert::Infallible);

#[cfg(not(target_os = "linux"))]
impl Mapped {
    /// Refused by name: see [`unmappable`].
    pub fn open(_at: u64) -> Result<Mapped, String> {
        Err(NOT_LINUX.to_string())
    }
}

#[cfg(not(target_os = "linux"))]
impl Window for Mapped {
    fn load(&self, _word: usize) -> u32 {
        match self.0 {}
    }
    fn store(&self, _word: usize, _value: u32) {
        match self.0 {}
    }
}
