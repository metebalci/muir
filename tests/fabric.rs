// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The debug cable to a debuggee in FPGA fabric, checked with no fabric
//! and no board.
//!
//! Nothing exists in the programmable logic yet, so the register window
//! [`muir::fabric`] proposes is modelled here in two ways.  [`Adapter`] is
//! the window with **the netlist's own DBGIN connector behind it** ---
//! [`muir::cable::DebugIn`], the interface board and a processor, MIT's
//! own logic --- which is what the driver and the transport are run
//! against: the three register strobes, a whole debug cycle, and the
//! hazard that a request split across two stores decodes the wrong strobe
//! on the real 74S139.  [`muir::fabric::Words`] is the window as a bare
//! array, which is how the layout's own rules are checked: the identity,
//! the marker, the sequence number and the latching of the answer.
//!
//! Neither is evidence about the fabric, which is unbuilt.  They are
//! evidence about muir's side of it, which is the half this repository
//! has.

use std::cell::{Cell, RefCell};

use muir::busint::{
    DEBUG_ADDRESS, DEBUG_CYCLE, DEBUG_MODIFIER, DEBUG_STATUS, DebugRequest, debug_modifier,
};
use muir::cable::DebugIn;
use muir::engine::Engine;
use muir::fabric::{
    self, A_SHIFT, ACK, CLEAR, CTL, DBD_SHIFT, DBUG, DRV, FAULTS, Fabric, IDENT, LIFT, MARK, REQ,
    SEQ_SHIFT, STS, UNMAPPED, WR, Window, Words,
};
use muir::lashup::{CableEnd, DebugProgram, FreeRunning};
use muir::machine::Machine;
use muir::netlist::Netlist;
use muir::rtl::Rtl;

// --- The window with MIT's own DBGIN behind it -------------------------------

/// The netlist board as the debuggee: the processor with the boot PROM,
/// started from the button as `muir --chip` starts it, the interface board
/// on its cables and a bare backplane behind it, as
/// [`muir::cable::DebugIn`] runs them --- `tests/lashup.rs` builds the
/// same board for the cable over TCP.
fn board() -> (DebugIn, Netlist) {
    use muir::cable::{Boards, FarEnd};
    use muir::chip::Chip;
    use muir::clock::{Behavioural, Clock};
    use muir::part::Level;

    let n = muir::netlist::parse(include_str!("../data/CADR.netlist")).unwrap();
    let bus_n = muir::netlist::parse(include_str!("../data/BUSINT.netlist")).unwrap();
    let mem_n = muir::netlist::parse(include_str!("../data/CADRM.netlist")).unwrap();
    let mut c = Chip::new(&n);
    c.power_on();
    c.load_prom(&n, &muir::prom::boot_prom_image());
    c.settle();
    let mut clk = Behavioural::new();
    let mut far = FarEnd::new(&n, &bus_n, &mem_n, Boards::default(), 0, Machine::new());
    far.join(&mut c, clk.time_ns());
    let boot = n.by_name_id("-BOOT1").unwrap();
    c.set_net(boot, Level::Low);
    c.settle();
    for _ in 0..20 {
        c.tick(&mut clk);
    }
    c.set_net(boot, Level::High);
    (DebugIn::new(&bus_n, c, clk, far), n)
}

/// A model of the fabric's adapter with the netlist's own DBGIN connector
/// behind it: the register layout of [`muir::fabric`] on one side and
/// MIT's own 21 wires on the other.
///
/// A store to `CTL` with `REQ` set puts the whole request on the connector
/// at once, which is what the layout is for; a store with `REQ` clear
/// lifts it, and the levels stay driven until then, the request going on
/// with `hold_ns` of `u64::MAX` so that nothing but the lift takes it off.
/// `ACK` and the word are latched at the instant the board first raises
/// `DEBUG IN ACK` and held until the lift, as the layout asks the fabric
/// to do.
///
/// **The boards run on every load of `STS`, one quantum a load.**  That is
/// the model's version of the fabric's own crystal: the debuggee runs
/// because time passes on its side and not because muir stepped it, and
/// muir's driver never steps it at all.
struct Adapter {
    board: RefCell<DebugIn>,
    ctl: Cell<u32>,
    /// The request on the connector, and its sequence.
    standing: Cell<bool>,
    seq: Cell<u32>,
    /// `DEBUG IN ACK` seen for it: whether the board drove `DBD`, and what.
    answer: Cell<Option<(bool, u16)>>,
    count: Cell<u32>,
    faults: Cell<u32>,
}

impl Adapter {
    fn new(board: DebugIn) -> Adapter {
        Adapter {
            board: RefCell::new(board),
            ctl: Cell::new(0),
            standing: Cell::new(false),
            seq: Cell::new(0),
            answer: Cell::new(None),
            count: Cell::new(0),
            faults: Cell::new(0),
        }
    }

    /// The request lifted, if one stands.
    fn lift(&self) {
        let mut board = self.board.borrow_mut();
        let now = board.ns();
        board.debug_release(now).unwrap();
        self.standing.set(false);
        self.answer.set(None);
    }

    /// One quantum of the boards, and `DEBUG IN ACK` latched if it has
    /// risen for the request standing.
    fn tick(&self) {
        let mut board = self.board.borrow_mut();
        board.step_until(u64::MAX).unwrap();
        if self.standing.get()
            && self.answer.get().is_none()
            && let Some((_, word)) = board.debug_ack()
        {
            self.answer.set(Some((word.is_some(), word.unwrap_or(0xffff))));
        }
    }

    /// Debug cycles the board has taken --- `-DB NEED UB` strobes, the
    /// only one of the four that runs a cycle on the debuggee's Unibus.
    fn debug_cycles(&self) -> u64 {
        self.board.borrow().debug_cycles
    }
}

impl Window for Adapter {
    fn load(&self, word: usize) -> u32 {
        match word {
            IDENT => DBUG,
            CTL => self.ctl.get(),
            STS => {
                self.tick();
                let (drove, dbd) = self.answer.get().unwrap_or((false, 0));
                MARK | (self.seq.get() << SEQ_SHIFT)
                    | if self.standing.get() { REQ } else { 0 }
                    | if self.answer.get().is_some() { ACK } else { 0 }
                    | if drove { DRV } else { 0 }
                    | ((dbd as u32) << DBD_SHIFT)
            }
            CLEAR => MARK | (LIFT & 0xffff_0000) | if self.standing.get() { REQ } else { 0 },
            FAULTS => MARK | self.faults.get() | (self.count.get() << fabric::COUNT_SHIFT),
            _ => UNMAPPED,
        }
    }

    fn store(&self, word: usize, value: u32) {
        match word {
            CTL if value & REQ != 0 => {
                if self.standing.get() {
                    self.faults.set(self.faults.get() | fabric::FAULT_ON_REQUEST);
                    self.lift();
                }
                let request = DebugRequest {
                    strobe: ((value >> A_SHIFT) & 3) as u8,
                    write: value & WR != 0,
                    dbd: (value >> DBD_SHIFT) as u16,
                    // The levels stay on the connector until muir stores
                    // the lift: nothing else takes the request off.
                    hold_ns: u64::MAX,
                };
                let mut board = self.board.borrow_mut();
                let now = board.ns();
                board.debug_request(now, request).unwrap();
                drop(board);
                self.ctl.set(value);
                self.standing.set(true);
                self.seq.set((value >> SEQ_SHIFT) & fabric::SEQ_MASK);
                self.answer.set(None);
                self.count.set(self.count.get() + 1);
                // A register strobe is acknowledged in the instant it is
                // made, so look before any time passes.
                if let Some((_, w)) = self.board.borrow().debug_ack() {
                    self.answer.set(Some((w.is_some(), w.unwrap_or(0xffff))));
                }
            }
            CTL => {
                if self.standing.get() {
                    self.lift();
                } else {
                    self.faults.set(self.faults.get() | fabric::FAULT_IDLE_LIFT);
                }
                self.ctl.set(value);
            }
            CLEAR if value == LIFT => {
                if self.standing.get() {
                    self.lift();
                }
                self.faults.set(0);
                self.seq.set(0);
            }
            _ => {}
        }
    }
}

/// The strobe of a request as [`Adapter`] decodes it, for the tests that
/// build one by hand.
fn ctl(strobe: u8, write: bool, dbd: u16, seq: u32) -> u32 {
    REQ | if write { WR } else { 0 }
        | ((strobe as u32) << A_SHIFT)
        | (seq << SEQ_SHIFT)
        | ((dbd as u32) << DBD_SHIFT)
}

// --- The driver against the netlist's own DBGIN ------------------------------

/// **The driver reads the debuggee's PC through the window, and it is the
/// PC the board is standing at.**  The debugger is an `rtl` machine
/// running CC's own two first operations from its boot PROM --- halt the
/// debuggee, read its PC ([`DebugProgram`]) --- and the debuggee is the
/// netlist's DBGIN connector with a processor behind it, reached only
/// through the proposed register window.  [`FreeRunning`] steps the
/// debugger and nothing else: the boards run because the model's crystal
/// ticks on every load of `STS`, which is what the fabric's own crystal
/// will do.
///
/// This is the strongest check the transport can have without a board.
/// Every one of the six requests is a store, a poll and a store, the two
/// clocks are kept in step by nothing at all, and what comes back is
/// MIT's logic answering, not a model of it.
#[test]
fn the_driver_reads_the_boards_pc_through_the_window() {
    let (end, n) = board();
    let unibus = |eadr: u8| muir::spy::BASE + 2 * eadr as u32;
    let mut program = DebugProgram::new();
    program.dbg_write(unibus(muir::spy::CLK), 0);
    program.dbg_read(unibus(muir::spy::PC), 0o101);
    let mut debugger = Rtl::new(program.finish());
    debugger.boot();

    let window = Fabric::open(Adapter::new(end)).expect("the model window identifies itself");
    let mut run = FreeRunning::new(debugger, window);
    run.run_until(80_000).unwrap();

    let (count, faults) = run.debuggee.taken().expect("FAULTS carries the marker");
    let read = run.debugger.machine().amem[0o101];
    let board = run.debuggee.window().board.borrow();
    eprintln!(
        "the window took {count} requests, faults {faults:#x}; the debugger read PC {read:o} and \
         the board stands at {:o} after {} microcycles and {} debug cycles",
        board.cpu.bus(&n, "PC", 14),
        board.microcycles,
        board.debug_cycles
    );
    assert_eq!(faults, 0, "the adapter faulted at nothing");
    assert_eq!(count, 6, "two operations of three Unibus cycles each reached the window");
    assert_eq!(board.debug_cycles, 2, "and two of the six were cycles on the debuggee's Unibus");
    assert_eq!(
        run.debugger.machine().bus_error,
        0,
        "every cycle of the debugger's was answered: nothing timed out"
    );
    assert_eq!(
        read,
        board.cpu.bus(&n, "PC", 14) as u32,
        "the PC read through the window is where the halted board stands"
    );
    assert!(run.debuggee.fault().is_none(), "and nothing at the window went unaccounted for");
}

/// **Each of the three register strobes is acknowledged at the window, and
/// only the status strobe drives the data lines back.**  The `DRV` bit
/// says which: `-DB READ STATUS` enables the 8304 at REQERR 0B15 onto
/// `DBD<7:0>`, and the two latch strobes drive nothing, which on the wire
/// is indistinguishable from driving all ones.  Each costs one store, one
/// load and one store, the acknowledgement being combinational and already
/// up when the load lands.
#[test]
fn each_register_strobe_is_answered_at_the_window() {
    let (end, _) = board();
    let mut window = Fabric::open(Adapter::new(end)).unwrap();
    for (strobe, what) in [
        (DEBUG_MODIFIER, "the modifier register"),
        (DEBUG_ADDRESS, "the address latches"),
        (DEBUG_STATUS, "the error status"),
    ] {
        let write = strobe != DEBUG_STATUS;
        let dbd = if write { debug_modifier::ADDRESS_17 } else { 0 };
        window.debug_request(0, DebugRequest { strobe, write, dbd, hold_ns: 0 }).unwrap();
        let (at, word) = window.debug_ack().unwrap_or_else(|| panic!("{what} was acknowledged"));
        assert_eq!(at, 0, "{what}: the acknowledgement is dated at the request's own instant");
        if strobe == DEBUG_STATUS {
            let status = word.unwrap_or_else(|| panic!("{what}: the board drove DBD"));
            assert_eq!(
                status & 0xff00,
                0xff00,
                "{what}: the 8304 at REQERR 0B15 drives DBD<7:0> and no more, and the high byte \
                 is the open cable's pull-ups reading ones"
            );
        } else {
            assert_eq!(word, None, "{what}: nothing on the debuggee drives DBD for a latch");
        }
        assert_eq!(window.window().debug_cycles(), 0, "{what}: no cycle on the debuggee's Unibus");
    }
    let (count, faults) = window.taken().unwrap();
    assert_eq!((count, faults), (3, 0), "three requests taken and nothing faulted");
    assert!(window.fault().is_none());
}

/// **A request split across two stores decodes the wrong strobe**, which
/// is the hazard the one-store layout exists to close.  The 74S139 at
/// DBGIN 0A15 decodes `DEBUG IN A<1:0>` combinationally while `-DEBUG IN
/// REQ` is down, so a carrier that puts the levels on the cable in one
/// store and the request in another gives the decoder whatever the second
/// store carried: here the address-latch strobe becomes `-DB NEED UB` and
/// the board runs a Unibus cycle at whatever address was last latched.
/// That is not a corrupt read but a write to a register nobody asked for.
///
/// The whole request in one store is the same wires and the right strobe:
/// acknowledged in the instant it is made, with no cycle on the debuggee's
/// Unibus at all.
#[test]
fn a_request_split_across_two_stores_decodes_the_wrong_strobe() {
    let (end, _) = board();
    let whole = Adapter::new(end);
    whole.store(CTL, ctl(DEBUG_ADDRESS, true, 0o1234, 1));
    assert_eq!(whole.load(STS) & ACK, ACK, "the address latch is strobed and answers at once");
    assert_eq!(whole.debug_cycles(), 0, "and nothing ran on the debuggee's Unibus");
    whole.store(CTL, 0);

    let (end, _) = board();
    let split = Adapter::new(end);
    // The levels first, without the request: nothing goes on the cable,
    // and the adapter counts it as a lift with nothing standing.
    split.store(CTL, ctl(DEBUG_ADDRESS, true, 0o1234, 1) & !REQ);
    assert_eq!(
        split.load(FAULTS) & fabric::FAULT_IDLE_LIFT,
        fabric::FAULT_IDLE_LIFT,
        "the adapter saw a lift with no request standing"
    );
    assert_eq!(split.debug_cycles(), 0);
    // Then the request on its own, as a carrier that writes the wires
    // piecewise would: the strobe bits are gone.
    split.store(CTL, REQ | (1 << SEQ_SHIFT));
    assert_eq!(
        split.debug_cycles(),
        1,
        "the 74S139 decoded -DB NEED UB and the board took a cycle nobody asked for"
    );
    assert_eq!(
        split.load(STS) & ACK,
        0,
        "and it is not acknowledged as a strobe is: a cycle has the debuggee's bus to win first"
    );
}

// --- An adapter that answers late --------------------------------------------

/// An adapter with nothing behind it that holds every request for a fixed
/// number of polls and then answers it with one word: the layout's rules
/// on top of the array, and no debuggee at all.
///
/// It is for one question --- how late an answer muir can take --- so the
/// only thing it models faithfully is the timing of its own bookkeeping:
/// `STS` shows the request standing from the store that placed it, as the
/// adapter's does, and `ACK` and the word appear only after `polls` loads.
struct Late {
    polls: u32,
    word: u16,
    /// The request standing: its sequence, and the polls it has had.
    held: Cell<Option<(u32, u32)>>,
}

impl Late {
    fn new(polls: u32, word: u16) -> Late {
        Late { polls, word, held: Cell::new(None) }
    }
}

impl Window for Late {
    fn load(&self, word: usize) -> u32 {
        match word {
            IDENT => DBUG,
            STS => match self.held.get() {
                None => MARK,
                Some((seq, seen)) => {
                    self.held.set(Some((seq, seen + 1)));
                    let answered = seen + 1 >= self.polls;
                    MARK | REQ
                        | (seq << SEQ_SHIFT)
                        | if answered { ACK | DRV | ((self.word as u32) << DBD_SHIFT) } else { 0 }
                }
            },
            FAULTS => MARK,
            _ => UNMAPPED,
        }
    }

    fn store(&self, word: usize, value: u32) {
        if word == CTL {
            self.held
                .set((value & REQ != 0).then_some(((value >> SEQ_SHIFT) & fabric::SEQ_MASK, 0)));
        }
    }
}

/// **An answer dated at the request's own instant is taken, though the
/// debugger has run a long way past it.**
///
/// This is the one place this transport parts company with the other two.
/// [`muir::lashup::Lashup`] and [`muir::lashup::Remote`] date the
/// acknowledgement by the debuggee's own simulated clock, which is kept in
/// step with the debugger's, so the instant they hand `debug_out_answer`
/// is at worst a generator cycle in its past.  Fabric has no simulated
/// clock to offer, and muir's own does not advance while a load is in
/// flight, so the choice is between the request's instant and the
/// debugger's clock at the moment the status was read.  muir takes the
/// former: a debug cycle then takes its nominal Unibus time instead of a
/// length that says how busy the Arm core was, and no answer the fabric
/// really gave can be thrown away by
/// `Busint::debug_out_answer`'s timeout, which refuses an
/// acknowledgement at or past the instant the interface gave up.
///
/// What that costs is an instant the interface has already passed, by far
/// more than the other two transports can produce.  Here the window
/// answers only on its twentieth poll, so the stamp is thousands of
/// nanoseconds behind the debugger's clock --- many generator cycles ---
/// and the cycle must still complete: `-LMACK` and `-LOADMD` fall due at
/// once rather than later, the word lands in `MD` and is parked in A
/// memory, nothing is flagged as a bus error, and the debugger's own clock
/// never goes backwards.
#[test]
fn an_answer_dated_in_the_debuggers_past_is_taken_and_the_word_lands() {
    const POLLS: u32 = 20;
    const WORD: u16 = 0o123456;
    let mut program = DebugProgram::new();
    program.dbg_read(0o766012, 0o101);
    let mut debugger = Rtl::new(program.finish());
    debugger.boot();
    let mut run = FreeRunning::new(debugger, Fabric::open(Late::new(POLLS, WORD)).unwrap());

    // How far behind the debugger's clock each answer was dated, and that
    // the clock itself only ever went forwards.
    let mut behind: Vec<u64> = Vec::new();
    let mut was = 0;
    let mut answered = 0;
    while run.debugger.ns() < 200_000 {
        let before = run.debuggee.window().held.get();
        run.step().unwrap();
        assert!(run.debugger.ns() >= was, "the debugger's own clock never goes backwards");
        was = run.debugger.ns();
        // The poll that answered is the one that cleared the request; the
        // answer it took stands until the next request is placed.
        if before.is_some() && run.debuggee.window().held.get().is_none() {
            let (at, _) = run.debuggee.debug_ack().expect("the answer it took stands");
            answered += 1;
            behind.push(run.debugger.ns() - at);
        }
    }
    let worst = behind.iter().copied().max().unwrap_or(0);
    eprintln!(
        "{answered} answers, the latest stamped {worst} ns behind the debugger's clock; \
         A memory holds {:o}",
        run.debugger.machine().amem[0o101]
    );
    assert_eq!(answered, 3, "the modifier, the address and the cycle were each answered once");
    assert!(
        worst > muir::lashup::max_step_ns(),
        "the point of the test is a stamp further back than the other two transports can make: \
         {worst} ns against a generator cycle of {}",
        muir::lashup::max_step_ns()
    );
    assert_eq!(
        run.debugger.machine().amem[0o101] as u16,
        WORD,
        "the word of an answer dated in the past still lands in MD and is parked"
    );
    assert_eq!(run.debugger.machine().bus_error, 0, "and no cycle was flagged as unanswered");
}

/// **An answer later than the debugger's own patience is not taken,
/// whichever instant it carries.**  The debugger's interface gives up
/// [`muir::busint::DEBUG_TIMEOUT_NS`] after the grant and puts the release
/// on the cable itself; the driver carries it, muir stores the lift, and
/// the window's answer after that is nobody's.
///
/// This is the limit of the choice the test above describes.  Dating the
/// answer at the request's own instant does **not** buy a cycle back from
/// the timeout: `Busint::poll` clears the pending request as the clock
/// reaches the timeout, in the same step, so `Busint::debug_out_answer`
/// refuses a late answer for having nothing outstanding long before its
/// other guard --- an instant at or past the timeout --- could come into
/// it.  The two stampings accept and refuse exactly the same answers.
/// What they differ in is the length a debug cycle appears to take.
#[test]
fn an_answer_later_than_the_debuggers_patience_is_not_taken() {
    let mut program = DebugProgram::new();
    program.dbg_read(0o766012, 0o101);
    let mut debugger = Rtl::new(program.finish());
    debugger.boot();
    // Far more polls than the interface's 11.05 microseconds allow.
    let mut run = FreeRunning::new(debugger, Fabric::open(Late::new(1_000, 0o123456)).unwrap());
    while run.debugger.ns() < 200_000 {
        run.step().unwrap();
    }
    eprintln!(
        "A memory holds {:o}, bus error {:#o}",
        run.debugger.machine().amem[0o101],
        run.debugger.machine().bus_error
    );
    assert_eq!(
        run.debugger.machine().bus_error,
        muir::machine::bus_error::UNIBUS_NXM,
        "the cycle nothing answered in time is flagged, as any timed-out Unibus cycle is"
    );
    assert_eq!(
        run.debugger.machine().amem[0o101],
        0xffff,
        "and the word parked is the open cable's, not the one the window gave too late"
    );
    assert!(
        run.debuggee.fault().is_none(),
        "the window itself did nothing muir cannot account for: the release was carried to it \
         and the lift stored, so its late answer was never looked at"
    );
}

// --- The layout's own rules, on the array ------------------------------------

/// A window that identifies itself and answers idle, ready for a test to
/// play the adapter's side by hand.
fn array() -> Fabric<Words> {
    Fabric::open(Words::new()).expect("a window at rest identifies itself")
}

/// The refusal a window earns, or a panic saying it was taken for the
/// adapter when it should not have been.
fn refusal<W: Window>(window: W) -> String {
    match Fabric::open(window) {
        Ok(_) => panic!("the window was taken for the debug cable's adapter"),
        Err(e) => e,
    }
}

/// One request placed, and the `CTL` word the window took for it.
fn request<W: Window>(window: &mut Fabric<W>, strobe: u8, dbd: u16) -> u32 {
    window.debug_request(1_000, DebugRequest { strobe, write: false, dbd, hold_ns: 0 }).unwrap();
    window.window().load(CTL)
}

/// **A stale acknowledgement is not muir's answer, and the sequence number
/// is what catches it.**  An acknowledgement left standing from one
/// transaction reads exactly like an answer to the next: the same `ACK`
/// bit, the same marker, a perfectly plausible word.  muir increments the
/// sequence on every request and the adapter reports the sequence of the
/// request it is holding, so a status that carries the last one is a fault
/// and not data --- and muir goes on waiting for its own answer until its
/// interface times out.
#[test]
fn a_stale_acknowledgement_is_caught_by_the_sequence_number() {
    let mut window = array();
    let first = request(&mut window, DEBUG_STATUS, 0);
    let seq = (first >> SEQ_SHIFT) & fabric::SEQ_MASK;
    window.window().set(STS, MARK | REQ | ACK | DRV | (seq << SEQ_SHIFT) | (0x00ff << DBD_SHIFT));
    assert_eq!(window.debug_ack(), Some((1_000, Some(0x00ff))), "its own answer is taken");

    // The next request, with the last transaction's answer left standing
    // at the window: the same bits, the same word, the old sequence.
    let next = request(&mut window, DEBUG_CYCLE, 0);
    assert_eq!((next >> SEQ_SHIFT) & fabric::SEQ_MASK, seq + 1, "the sequence moved on");
    assert_eq!(window.debug_ack(), None, "the stale acknowledgement is not this request's answer");
    let fault = window.fault().expect("and it is a fault, not data");
    assert!(fault.what.contains("not the adapter holding muir's request"), "{}", fault.what);
    assert!(
        !fault.fatal,
        "a lost request costs the cycle and not the run: the debugger times it out and the next \
         request begins again, which is what makes the adapter's watchdog safe to have"
    );
    assert!(window.window().get(CTL) & REQ != 0, "the request still stands: nothing was lifted");
}

/// **The word is the one driven at the acknowledgement, not the one
/// standing at the load.**  The adapter latches `DBD` and `ACK` together
/// at the instant `DEBUG IN ACK` first rises and holds them until the
/// request is lifted, because by the time the Arm gets round to loading,
/// the word the debuggee drove may be long gone.  muir's half of that is
/// this: the answer is taken at the load that first saw `ACK` and never
/// read again, so a window whose data lines move afterwards cannot change
/// what the debugger was told.
#[test]
fn the_word_is_the_one_driven_at_the_acknowledgement() {
    let mut window = array();
    let placed = request(&mut window, DEBUG_CYCLE, 0);
    let seq = (placed >> SEQ_SHIFT) & fabric::SEQ_MASK;
    let sts = MARK | REQ | ACK | DRV | (seq << SEQ_SHIFT);
    // Nothing yet: the adapter is holding the request and has no answer.
    window.window().set(STS, MARK | REQ | (seq << SEQ_SHIFT));
    assert_eq!(window.debug_ack(), None);
    // The acknowledgement, with the word the debuggee drove.
    window.window().set(STS, sts | (0o123456u32 << DBD_SHIFT));
    let answered = window.debug_ack();
    assert_eq!(answered, Some((1_000, Some(0o123456))), "the word at the acknowledgement");
    // And the data lines move on, as a debuggee's do once the cycle is
    // over: the answer does not.
    window.window().set(STS, sts | (0xffff << DBD_SHIFT));
    assert_eq!(window.debug_ack(), answered, "the answer is the one latched, not the one standing");
    assert_eq!(
        window.window().get(CTL) & REQ,
        0,
        "and the request was lifted when it was answered"
    );
}

/// **A status word without the marker is the window having died**, and is
/// never data.  All zeros and all ones are what a window reads with
/// nothing behind it or nothing driving it, and `DBD` alone cannot tell
/// muir so: all ones there is what an open cable reads and is a perfectly
/// legal answer.  The guard is applied to every load and not only the
/// first.
#[test]
fn a_status_without_the_marker_is_a_dead_window() {
    for dead in [0, u32::MAX] {
        let mut window = array();
        request(&mut window, DEBUG_CYCLE, 0);
        window.window().set(STS, dead);
        assert_eq!(window.debug_ack(), None, "{dead:#x} is no answer");
        let fault = window.fault().expect("and it is a fault");
        assert!(fault.what.contains("MARK bits are not 01"), "{}", fault.what);
        assert!(fault.fatal, "and the run ends: there is nothing left to believe or to store into");
    }
}

/// **The window is refused by what its identity reads, and the refusal
/// says which case it is**, because each points somewhere different: four
/// printable letters mean another face answered and the fix is a different
/// address, all zeros or all ones mean nothing is behind the window or
/// nothing is driving it, and anything else is a window muir cannot
/// account for.  `NONE` is the one that is not a mistake at all: a
/// general-purpose port brought out with nothing of ours behind it answers
/// with it rather than hanging the core.
///
/// In every case muir stops.  It stores nothing --- a store into some
/// other device's registers is the one failure that damages something ---
/// and it does not fall back to the network endpoint.
#[test]
fn a_window_that_is_not_the_adapter_is_refused_by_what_it_reads() {
    for (ident, says) in [
        (0x4E4F_4E45, "the default slave"),
        (0x434F_4E53, "the console"),
        (0x5041_434B, "the disk pack side"),
        (0x4142_4344, "another register face answered"),
        (0, "all zeros"),
        (u32::MAX, "all ones"),
        (0x1234_5678, "no face muir can account for"),
    ] {
        let words = Words::new();
        words.set(IDENT, ident);
        let refused = refusal(words);
        assert!(refused.contains(says), "{ident:#010x}: {refused}");
        assert!(
            !refused.contains("endpoint"),
            "{ident:#010x}: no fallback to the network is offered: {refused}"
        );
    }
    // And the identity is read before anything is stored: a window that is
    // some other device's registers is left exactly as it was.
    let words = Words::new();
    words.set(IDENT, 0x434F_4E53);
    let before: Vec<u32> = (0..fabric::WORDS).map(|w| words.get(w)).collect();
    refusal(&words);
    let after: Vec<u32> = (0..fabric::WORDS).map(|w| words.get(w)).collect();
    assert_eq!(before, after, "nothing was stored into a window muir refused");
}

/// **The identity is a `u32` comparison and never a string one.**  These
/// are 32-bit registers on a 32-bit port, so a load gives the register's
/// value unchanged; the same word stored to memory on a little-endian core
/// and read back a byte at a time reads `GUBD`, which is why a comparison
/// against the letters would be wrong in a way that looks right.  The
/// bytes appear in exactly one place, the refusal's message, where they
/// are printed most significant first because that is the order they are
/// meant in.
#[test]
fn the_identity_and_the_key_are_the_words_the_fabric_presents() {
    assert_eq!(DBUG, 0x4442_5547, "\"DBUG\", 'D' in the most significant byte");
    assert_eq!(LIFT, 0x4C49_4654, "\"LIFT\", 'L' in the most significant byte");
    assert_eq!(UNMAPPED, !DBUG, "the unmapped words are the identity's complement");
    let words = Words::new();
    words.set(IDENT, DBUG.swap_bytes());
    let refused = refusal(words);
    assert!(refused.contains("GUBD"), "and the refusal prints the bytes as they came: {refused}");
    // The key muir stores, as it opens the window.
    let window = array();
    assert_eq!(window.window().get(CLEAR), LIFT, "the key is stored once at the open");
}

/// **A muir that stops with a request standing lifts it.**  A request left
/// down holds `-DB NEED UB` on the debuggee, which keeps the debug master
/// on its Unibus with `-UB BBSY` asserted and the machine's own cycles
/// waiting for ever.  The adapter's watchdog is the backstop for a muir
/// that is killed outright; this is the ordinary way out.
#[test]
fn a_request_standing_is_lifted_when_the_window_is_dropped() {
    let words = Words::new();
    {
        let mut window = Fabric::open(&words).unwrap();
        request(&mut window, DEBUG_CYCLE, 0o777);
        assert_eq!(words.get(CTL) & REQ, REQ, "the request stands");
    }
    assert_eq!(words.get(CTL) & REQ, 0, "and is lifted on the way out");
    assert_eq!(
        words.get(CTL) >> DBD_SHIFT,
        0o777,
        "with the levels unchanged, since the latches take DBD at the lift"
    );
}
