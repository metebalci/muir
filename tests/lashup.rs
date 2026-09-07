// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The debug cable on DBGIN, the first half of the two-machine lashup: a
//! harness plays the debugger --- CC's four registers by hand, as the
//! PDP-11 once did --- against an `rtl` debuggee, and what it reads back
//! through the cable is what is on the machine.
//!
//! The cable is 21 wires, read off MIT's wire list for the board:
//! `DBD<15:0>`, the request, the acknowledge, the write flag and
//! two address bits.  [`DebugRequest`] is those wires as the debugger drives
//! them; the debuggee's DBGIN page decodes the two address bits into CC's
//! four registers --- `766100` the cycle, `766104` the status, `766110` the
//! modifier, `766114` the address --- and runs the cycle as a second master
//! on its own Unibus.

use muir::busint::{
    self, DEBUG_ADDRESS, DEBUG_CYCLE, DEBUG_MODIFIER, DEBUG_STATUS, DebugRequest, debug_modifier,
};
use muir::engine::Engine;
use muir::isa::Insn;
use muir::machine::{Machine, bus_error};
use muir::rtl::Rtl;
use muir::spy;

use muir::ioboard;
use muir::isa::asm::{ALU, SETM, SRC_MD, START_READ, a_dest, a_src, filler, m_src};
use muir::lashup::{CableEnd, DebugProgram, Error, Lashup, Message, Remote};

/// An `rtl` engine on `m`, booted.
fn booted(m: Machine) -> Rtl {
    let mut r = Rtl::new(m);
    r.boot();
    r
}

/// A machine running fillers: the PC climbs one a microcycle and nothing
/// touches the buses.
fn straight_line() -> Rtl {
    let mut m = Machine::new();
    m.amem[3] = 0o123456;
    m.load_prom(&vec![filler(); 512]);
    let mut r = Rtl::new(m);
    r.boot();
    r
}

/// A machine that reads four Unibus locations forty-two microcycles apart
/// --- `A-LOW`, `STAT-LOW`, register 3 and `FLAG-1` of its own diagnostic
/// block --- and parks each in A memory, as `tests/spy.rs` has it; and one
/// more read, of `extra`, parked at `0o105`.
fn unibus_reader(extra: u32) -> Rtl {
    let mut m = Machine::new();
    m.amem[3] = 0o123456;
    m.l1_map[0] = 0;
    m.l2_map[0] = (1 << 23) | 0o37766;
    m.mmem[1] = spy::A_LOW as u32;
    m.mmem[2] = spy::STAT_LOW as u32;
    m.mmem[3] = 3;
    m.mmem[4] = spy::FLAG_1 as u32;
    // Virtual page 1 on the physical page the Unibus location `extra` is
    // reached through; the offset within it.
    let phys = busint::unibus_physical(extra);
    m.l2_map[1] = (1 << 23) | (phys >> 8);
    m.mmem[5] = 0o400 | (phys & 0xff);
    let mut prom = vec![filler(); 512];
    let mut at = 0;
    for (k, park) in [0o101, 0o102, 0o103, 0o104, 0o105].iter().enumerate() {
        prom[at] = Insn::new(ALU | SETM | m_src(k as u64 + 1) | a_src(3) | START_READ);
        prom[at + 41] = Insn::new(ALU | SETM | SRC_MD | a_src(3) | a_dest(*park));
        at += 42;
    }
    m.load_prom(&prom);
    let mut r = Rtl::new(m);
    r.boot();
    r
}

/// The harness debugger: puts one request on the cable at the debuggee's
/// present instant, once the last has cleared it, and runs the debuggee
/// until the request is acknowledged.  Returns when the request went on,
/// when `DEBUG IN ACK` rose, and the word on `DBD<15:0>`.  The request is
/// lifted [`busint::UNIBUS_STROBE_NS`] after the acknowledgement, as machine
/// A's own `-UB MSYN` would lift it.
fn request(r: &mut Rtl, strobe: u8, write: bool, dbd: u16) -> (u64, u64, Option<u16>) {
    for _ in 0..100 {
        if !r.debug_busy() {
            break;
        }
        r.step().unwrap();
    }
    assert!(!r.debug_busy(), "the last request never cleared the cable");
    let at = r.ns();
    r.debug_request(at, DebugRequest { strobe, write, dbd, hold_ns: busint::UNIBUS_STROBE_NS });
    let mut answer = None;
    for _ in 0..1_000 {
        if answer.is_none() {
            answer = r.debug_ack();
        }
        // On until the request has been lifted and the bus let go, so that
        // a latch has taken what the strobe gave it.
        if let Some((ack, word)) = answer
            && !r.debug_busy()
        {
            return (at, ack, word);
        }
        r.step().unwrap();
    }
    panic!("strobe {strobe} at {at} ns was never acknowledged and lifted");
}

/// CC's `DBG-READ`: the modifier with bit 17 of the address, the address's
/// bits 1 to 16, then the cycle.
fn dbg_read(r: &mut Rtl, uaddr: u32) -> (u64, u16) {
    request(r, DEBUG_MODIFIER, true, (uaddr >> 17) as u16 & 1);
    request(r, DEBUG_ADDRESS, true, (uaddr >> 1) as u16);
    let (_, ack, word) = request(r, DEBUG_CYCLE, false, 0);
    (ack, word.unwrap_or_else(|| panic!("nothing drove DBD for the read of {uaddr:o}")))
}

/// CC's `DBG-WRITE`.
fn dbg_write(r: &mut Rtl, uaddr: u32, val: u16) -> u64 {
    request(r, DEBUG_MODIFIER, true, (uaddr >> 17) as u16 & 1);
    request(r, DEBUG_ADDRESS, true, (uaddr >> 1) as u16);
    request(r, DEBUG_CYCLE, true, val).1
}

/// Unibus addresses of the console's registers, `766000` plus twice the
/// register number.
fn unibus(eadr: u8) -> u32 {
    spy::BASE + 2 * eadr as u32
}

/// **A read of `766012` through the cable is the PC on the machine, and a
/// write of the clock control register through it halts and steps the
/// machine as CC's does.**
///
/// What CC does first to a debuggee: `DBG-WRITE` of `0` to `SPY-CLK`, which
/// takes `RUN` down; then reads of `SPY-PC`, which is what `CC-REGISTER-
/// EXAMINE` shows for the PC, and single steps by `2` then `0` in
/// `SPY-CLK`.  The halted machine's PC is stable, so the read and the
/// machine can be compared exactly, and one step moves it by one.
#[test]
fn a_debug_read_of_766012_returns_the_debuggees_pc() {
    let mut r = straight_line();
    for _ in 0..30 {
        r.step().unwrap();
    }
    assert_ne!(r.spy_read(spy::FLAG_1) & 0x100, 0, "running before the halt");

    let acked = dbg_write(&mut r, unibus(spy::CLK), 0);
    for _ in 0..4 {
        r.step().unwrap();
    }
    assert_eq!(r.spy_read(spy::FLAG_1) & 0x100, 0, "SRUN down: halted through the cable");
    let halted_at = r.pc();
    eprintln!("halted at PC {halted_at:o}; the halt's cycle was acknowledged at {acked} ns");

    let (at, pc) = dbg_read(&mut r, unibus(spy::PC));
    eprintln!("PC read through the cable: {pc:o}, acknowledged at {at} ns");
    assert_eq!(pc, halted_at, "the PC read through the cable is the PC on the machine");
    assert_eq!(pc, r.pc(), "and the machine has not moved");

    // A single step, CC's `CC-CLOCK`: `2` then `0`.
    dbg_write(&mut r, unibus(spy::CLK), 2);
    dbg_write(&mut r, unibus(spy::CLK), 0);
    let (_, pc2) = dbg_read(&mut r, unibus(spy::PC));
    assert_eq!(pc2, halted_at + 1, "one step through the cable moved the PC by one");
    assert_eq!(pc2, r.pc());

    // The other halted registers CC examines.
    let (_, a_low) = dbg_read(&mut r, unibus(spy::A_LOW));
    assert_eq!(a_low, 0o123456, "A-LOW, the A bus under the filler");
    let (_, ir_high) = dbg_read(&mut r, unibus(spy::IR_HIGH));
    assert_eq!(ir_high, 3, "IR-HIGH, the filler's A address");
    let (_, open) = dbg_read(&mut r, unibus(3));
    assert_eq!(open, spy::OPEN_READ, "register 3, the open bus");
}

/// **The three register strobes are acknowledged at once, and the status
/// strobe drives the error register.**  `DEBUG ACK` is the NAND of the
/// three strobes at DBGIN 0A14 ORed with the cycle's, so a write of the
/// address or the modifier costs the debugger no arbitration; and the 8304
/// at REQERR 0B15 puts the same eight bits on `DBD` that a read of `766044`
/// gives, `-FREE` as it stands.
#[test]
fn the_register_strobes_answer_at_once_and_the_status_is_the_error_register() {
    let mut r = straight_line();
    for _ in 0..10 {
        r.step().unwrap();
    }
    let (at, ack, word) = request(&mut r, DEBUG_ADDRESS, true, 0o123456);
    assert_eq!(ack, at, "the address strobe is acknowledged the instant it is made");
    assert_eq!(word, None, "and drives nothing back");
    assert_eq!(r.busint().debug_unibus_address(), 0o123456 << 1, "the latch took it when lifted");

    let (at, ack, word) = request(&mut r, DEBUG_MODIFIER, true, debug_modifier::ADDRESS_17);
    assert_eq!(ack, at, "the modifier strobe likewise");
    assert_eq!(word, None);
    assert_eq!(r.busint().debug_unibus_address(), (1 << 17) | (0o123456 << 1));

    let (at, ack, status) = request(&mut r, DEBUG_STATUS, false, 0);
    assert_eq!(ack, at, "and the status strobe");
    assert_eq!(
        status,
        Some(0xff00),
        "no error, the bus free, no write-through; the high byte open"
    );

    r.machine_mut().bus_error = bus_error::UNIBUS_NXM;
    let (_, _, status) = request(&mut r, DEBUG_STATUS, false, 0);
    assert_eq!(status, Some(0xff00 | bus_error::UNIBUS_NXM), "the Unibus NXM bit, as CC prints it");
    request(&mut r, DEBUG_MODIFIER, true, 0);
}

/// **A debug cycle takes the Unibus from the processor and gives it back.**
/// The processor keeps `LMUB MASTER` after its first Unibus cycle and
/// skips the arbitration thereafter; the debug master's `SACK` clears it
/// --- `NAND(-LM NEED UB, SACK IN)` at UBMAST 0D05 --- so the processor's
/// next cycle is arbitrated from the start, and it still gets its word.
#[test]
fn a_debug_cycle_takes_the_unibus_from_the_processor_and_gives_it_back() {
    let mut r = unibus_reader(0o764120);
    while r.bus_cycles() < 1 || r.busint().busy() {
        r.step().unwrap();
    }
    assert!(r.busint().holds_the_unibus(), "the processor keeps the Unibus after its first cycle");

    let (_, pc) = dbg_read(&mut r, unibus(spy::PC));
    assert!(!r.busint().holds_the_unibus(), "the debug master's SACK took it away");
    assert!(pc < 0o400, "a PC in the PROM: {pc:o}");

    for _ in 0..250 {
        r.step().unwrap();
    }
    let m = r.machine();
    assert_eq!(m.bus_error, 0, "every read of the processor's was answered");
    assert_eq!(m.amem[0o101], 0o123456, "A-LOW");
    assert_eq!(m.amem[0o102], 0, "STAT-LOW");
    assert_eq!(m.amem[0o103], spy::OPEN_READ as u32, "register 3");
    assert_eq!(m.amem[0o104], 0xe900, "FLAG-1, running");
    assert_eq!(r.bus_cycles(), 5);
    assert!(r.busint().holds_the_unibus(), "and the processor has the Unibus again");
}

/// **A debug cycle at an address nothing answers is never acknowledged.**
/// `INT BUSY`, which starts the timeout counter on REQTIM, is made from
/// the processor's grants alone at RQSYNC 0C13; the debug master's cycle
/// waits until the debugger lifts the request, which its own interface
/// does at thirty microseconds.  Lifted, `DBUB MASTER` clears and the
/// processor's Unibus cycles go on.
#[test]
fn a_debug_cycle_nothing_answers_waits_for_the_debugger_to_give_up() {
    let mut r = straight_line();
    for _ in 0..10 {
        r.step().unwrap();
    }
    // An address on no board.
    let uaddr = 0o760100;
    request(&mut r, DEBUG_MODIFIER, true, (uaddr >> 17) as u16 & 1);
    request(&mut r, DEBUG_ADDRESS, true, (uaddr >> 1) as u16);
    let at = r.ns();
    r.debug_request(at, DebugRequest { strobe: DEBUG_CYCLE, write: false, dbd: 0, hold_ns: 100 });
    while r.ns() < at + 3 * busint::TIMEOUT_NS {
        r.step().unwrap();
    }
    assert_eq!(r.debug_ack(), None, "no acknowledgement three timeouts on");
    assert!(r.busint().debug_master(), "DBUB MASTER holds the bus meanwhile");
    r.debug_release(r.ns());
    for _ in 0..3 {
        r.step().unwrap();
    }
    assert!(!r.debug_busy(), "lifted, the request is gone");
    assert!(!r.busint().debug_master());
}

/// **The modifier's reset bit resets the debuggee and halts it.**
/// `-DEBUGEE RESET` is an input of the interface's `RESET` --- `-UB INIT`
/// and `-XBUS INIT` --- and of `-BUSINT LM RESET`, which OLORD2 makes
/// `-CLOCK RESET A` and `-CLOCK RESET B` of on the cpu: the power-on reset,
/// which clears `RUN`.  CC's `DBG-RESET` writes `2` then `0`; afterwards a
/// `1` in the clock control register starts the machine again.
#[test]
fn the_modifiers_reset_bit_resets_the_debuggee_and_halts_it() {
    let mut r = straight_line();
    for _ in 0..20 {
        r.step().unwrap();
    }
    r.spy_write(spy::MODE, 0o44);
    for _ in 0..3 {
        r.step().unwrap();
    }
    assert!(r.machine().mode.prom_disable, "PROMDISABLE set before the reset");
    assert_ne!(r.spy_read(spy::FLAG_1) & 0x100, 0, "running");

    request(&mut r, DEBUG_MODIFIER, true, debug_modifier::RESET);
    for _ in 0..3 {
        r.step().unwrap();
    }
    assert_eq!(r.spy_read(spy::FLAG_1) & 0x100, 0, "SRUN down: the power-on reset cleared RUN");
    assert!(!r.machine().mode.prom_disable, "and the mode register");
    assert!(!r.machine().clock_control.run);
    request(&mut r, DEBUG_MODIFIER, true, 0);
    let pc = r.pc();
    for _ in 0..5 {
        r.step().unwrap();
    }
    assert_eq!(r.pc(), pc, "halted: the PC does not move");

    dbg_write(&mut r, unibus(spy::CLK), 1);
    for _ in 0..5 {
        r.step().unwrap();
    }
    assert_ne!(r.spy_read(spy::FLAG_1) & 0x100, 0, "RUN up again: running");
    assert!(r.pc() > pc, "and the PC moves");
}

/// **The modifier's reset bit clears the debuggee's model I/O boards.**
/// `-DEBUGEE RESET` is an input of the interface's `RESET`, which goes out
/// as `-XBUS INIT` and `-UB INIT`, so the debuggee's disk controller,
/// display and Chaosnet interface clear what their drawings say ---
/// `Machine::bus_reset`. Here the debuggee is left with a disk timeout
/// error, and a reset over the cable clears it. (The machine's own
/// `PROG.UNIBUS.RESET` does the same, `tests/busint.rs`.)
#[test]
fn the_modifiers_reset_bit_clears_the_debuggees_boards() {
    use muir::disk_unit::{Geometry, Unit};
    // The disk controller's command and start registers and its
    // nonexistent-memory error bit, as `tests/disk.rs` reads them off MIT's
    // `disk.text`.
    const COMMAND: u32 = 0;
    const START: u32 = 3;
    const NXM: u32 = 1 << 20;

    let mut r = straight_line();
    for _ in 0..20 {
        r.step().unwrap();
    }
    // A read whose command word points past memory, with the done interrupt
    // enabled, started: the controller raises the NXM error flop.
    let mut scratch = vec![0u32; 1 << 16];
    scratch[0] = 0x3fff << 8;
    r.machine_mut().disk.attach(0, Unit::blank(Geometry::T80));
    r.machine_mut().disk.write(COMMAND, 1 << 11, &mut scratch);
    r.machine_mut().disk.write(START, 0, &mut scratch);
    assert_ne!(r.machine().disk.status() & NXM, 0, "the error is up to be cleared");

    request(&mut r, DEBUG_MODIFIER, true, debug_modifier::RESET);
    for _ in 0..3 {
        r.step().unwrap();
    }
    assert_eq!(r.machine().disk.status() & NXM, 0, "the cable reset cleared the error flop");
    request(&mut r, DEBUG_MODIFIER, true, 0);
}

/// **The modifier's timeout inhibit holds the processor's own cycle
/// open.**  The 74LS273 at REQTIM 0B01 that counts the timeout is held
/// clear unless `INT BUSY AND -DEBUG TIMEOUT INH`, so a cycle of the
/// processor's that nothing answers waits, ten microseconds and more, until
/// the debugger lifts the inhibit; then the counter runs and the cycle is
/// given up on as usual.
#[test]
fn the_modifiers_timeout_inhibit_holds_the_processors_nxm_cycle() {
    // The fifth read is of an address between the I/O board's two blocks,
    // which nothing answers.
    let mut r = unibus_reader(0o760100);
    request(&mut r, DEBUG_MODIFIER, true, debug_modifier::TIMEOUT_INHIBIT);
    while r.bus_cycles() < 5 {
        r.step().unwrap();
    }
    let started = r.ns();
    while r.ns() < started + 2 * busint::TIMEOUT_NS {
        r.step().unwrap();
    }
    assert!(r.busint().busy(), "the fifth cycle is still open, two timeouts on");
    assert_eq!(r.machine().bus_error, 0, "and no NXM has been flagged");

    request(&mut r, DEBUG_MODIFIER, true, 0);
    let lifted = r.ns();
    while r.busint().busy() {
        r.step().unwrap();
        assert!(r.ns() < lifted + 2 * busint::TIMEOUT_NS, "the cycle never ended");
    }
    assert_eq!(r.machine().bus_error, bus_error::UNIBUS_NXM, "given up on once the inhibit lifted");
    eprintln!("the cycle ended {} ns after the inhibit was lifted", r.ns() - lifted);
}

// --- Two machines: the debugger's DBGOUT to the debuggee's DBGIN ------------

/// **The debugger halts the debuggee over the cable, reads its PC, steps it
/// and reads it again: CC's `DBG-WRITE` and `DBG-READ`, machine to
/// machine, on two `rtl` engines under one scheduler.**  Every cycle into
/// the debugger's debug block is a request on the cable, answered by the
/// debuggee's DBGIN as the harness's were; the debugger's cpu waits on its
/// own bus meanwhile, as it would for any slow slave.
#[test]
fn a_debugger_halts_and_reads_the_debuggee_over_the_cable() {
    let mut a = DebugProgram::new();
    a.dbg_write(unibus(spy::CLK), 0);
    a.dbg_read(unibus(spy::PC), 0o101);
    a.dbg_write(unibus(spy::CLK), 2);
    a.dbg_write(unibus(spy::CLK), 0);
    a.dbg_read(unibus(spy::PC), 0o102);
    let mut lashup = Lashup::new(booted(a.finish()), straight_line());
    lashup.run_until(120_000).unwrap();

    let a = lashup.debugger.machine();
    let b = &lashup.debuggee;
    assert_eq!(b.spy_read(spy::FLAG_1) & 0x100, 0, "the debuggee is halted");
    assert_eq!(a.bus_error, 0, "every cycle of the debugger's was answered");
    let pc = a.amem[0o101];
    eprintln!(
        "the debuggee halted at PC {pc:o}, stepped to {:o}; steps {:?}",
        a.amem[0o102], lashup.steps
    );
    assert!(pc > 0 && pc < 0o400, "a PC in the debuggee's PROM, past the boot: {pc:o}");
    assert_eq!(a.amem[0o102], pc + 1, "one step through the cable moved it by one");
    assert_eq!(b.pc() as u32, pc + 1, "and that is where the debuggee stands");
    assert_eq!(lashup.debugger.bus_cycles(), 15, "five DBG operations, three cycles each");
    assert!(!b.debug_busy(), "the cable is quiet");
}

/// **A debug cycle the debuggee never answers times out on the debugger,
/// at the REQTIM PROM's 26 microseconds and not its header's 30.**  The
/// debuggee's DBGIN runs a cycle to its Chaosnet interface, which is not
/// there and which no timeout of the debuggee's watches; the debugger's
/// interface gives up, flags `UB NXM ERROR`, lifts the request, and the
/// debuggee's `DBUB MASTER` lets go.
#[test]
fn a_debug_cycle_the_debuggee_never_answers_times_out_on_the_debugger() {
    let mut a = DebugProgram::new();
    a.dbg_read(0o760100, 0o101);
    let mut lashup = Lashup::new(booted(a.finish()), straight_line());

    // The two strobes go by; the cycle's request goes out and waits.
    while !lashup.debugger.busint().debug_out_pending() {
        lashup.step().unwrap();
        assert!(lashup.debugger.ns() < 30_000, "the request never went out");
    }
    let granted = lashup.debugger.ns();
    eprintln!("the debugger's cycle was granted at about {granted} ns");
    lashup.run_until(granted + 3_000).unwrap();
    assert!(lashup.debugger.busint().debug_out_pending(), "the request is out, unanswered");
    assert!(lashup.debuggee.busint().debug_master(), "DBUB MASTER holds the debuggee's Unibus");
    // The debugger's timeout counter registers its NXM on the fourteenth
    // edge of a clock that has run since power-on, between 13.5 and 14.5
    // microseconds after the grant (`busint::debug_timeout_at`): not yet at
    // 13, and by 15, where the PROM's 26 --- the header's 30 --- would not
    // have come, those being a later board's microseconds.
    lashup.run_until(granted + 13_000).unwrap();
    assert!(lashup.debugger.busint().debug_out_pending(), "still waiting 13 microseconds on");
    assert_eq!(lashup.debugger.machine().amem[0o101], 0, "no word yet");
    lashup.run_until(granted + 15_000).unwrap();
    assert!(!lashup.debugger.busint().debug_out_pending(), "given up on by 15 microseconds");
    let a = lashup.debugger.machine();
    assert_eq!(a.bus_error, bus_error::UNIBUS_NXM, "the debugger's own NXM timeout");
    assert_eq!(a.amem[0o101], 0xffff, "the word of a cycle nothing answered: the open bus");
    assert!(!lashup.debuggee.debug_busy(), "the request lifted, the debuggee's master released");
    assert_eq!(lashup.debuggee.debug_ack(), None, "and it never acknowledged");
    assert_eq!(lashup.debuggee.machine().bus_error, 0, "nothing was flagged on the debuggee");
}

/// **With no cable plugged in, a cycle into the debug block is acknowledged
/// at once.**  `DEBUG OUT ACK` is pulled up by the SIP at DBGIN 0A22, so
/// `DEBUG SSYN` rises with `SELECT DEBUG`; the word is the open `DBD`
/// through the 8304s, all ones, and no error is flagged.  The model before
/// this timed the block out as an address nothing answers.
#[test]
fn with_no_cable_a_debug_cycle_is_acknowledged_at_once() {
    let mut a = DebugProgram::new();
    a.dbg_read(unibus(spy::PC), 0o101);
    let mut a = booted(a.finish());
    for _ in 0..120 {
        a.step().unwrap();
    }
    assert_eq!(a.bus_cycles(), 3);
    assert_eq!(a.machine().bus_error, 0, "no timeout");
    assert_eq!(a.machine().amem[0o101], 0xffff, "the open bus");
}

/// **The debugger reads a running debuggee, and neither loses a cycle.**
/// The debuggee's own microcode is reading its Unibus every forty-two
/// microcycles while the debugger's requests come in: the debug master
/// takes the bus between the processor's cycles or waits behind one, the
/// processor re-arbitrates behind the debug master, and every read on both
/// machines gets its word --- the debuggee's four diagnostic reads and its
/// read of the I/O board, the debugger's five reads of a climbing PC.
#[test]
fn the_debugger_reads_a_running_debuggee_and_neither_loses_a_cycle() {
    let mut a = DebugProgram::new();
    for park in [0o101, 0o102, 0o103, 0o104, 0o105] {
        a.dbg_read(unibus(spy::PC), park);
    }
    let mut lashup = Lashup::new(booted(a.finish()), unibus_reader(0o764120));
    lashup.run_until(120_000).unwrap();

    let b = lashup.debuggee.machine();
    assert_eq!(b.bus_error, 0, "every read of the debuggee's was answered");
    assert_eq!(b.amem[0o101], 0o123456, "A-LOW");
    assert_eq!(b.amem[0o102], 0, "STAT-LOW");
    assert_eq!(b.amem[0o103], spy::OPEN_READ as u32, "register 3");
    assert_eq!(b.amem[0o104], 0xe900, "FLAG-1, running");
    assert_eq!(lashup.debuggee.bus_cycles(), 5);

    let a = lashup.debugger.machine();
    assert_eq!(a.bus_error, 0, "every read of the debugger's was answered");
    let pcs: Vec<u32> = (0o101..=0o105).map(|k| a.amem[k]).collect();
    eprintln!("the running debuggee's PC, five reads: {pcs:?}; steps {:?}", lashup.steps);
    assert!(pcs.windows(2).all(|w| w[1] > w[0]), "a climbing PC: {pcs:?}");
    assert!(pcs.iter().all(|&p| p < 512), "in the PROM: {pcs:?}");
    assert_eq!(lashup.debugger.bus_cycles(), 15);
}

// --- The cable over TCP -------------------------------------------------------

/// CC's first five operations on a debuggee, as a debugger's microcode:
/// halt it, read its PC, step it, read again.
fn halt_read_step_read() -> Machine {
    let mut a = DebugProgram::new();
    a.dbg_write(unibus(spy::CLK), 0);
    a.dbg_read(unibus(spy::PC), 0o101);
    a.dbg_write(unibus(spy::CLK), 2);
    a.dbg_write(unibus(spy::CLK), 0);
    a.dbg_read(unibus(spy::PC), 0o102);
    a.finish()
}

/// The two ends of the cable on a TCP connection over the loopback, each on
/// its own thread as it would be in its own program: the debuggee
/// listening, the debugger connecting.  Returns both machines run to `ns`.
fn over_tcp(debugger: Rtl, debuggee: Rtl, ns: u64) -> (Rtl, Rtl) {
    use std::net::{TcpListener, TcpStream};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let far = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream.set_nodelay(true).unwrap();
        let reader = stream.try_clone().unwrap();
        let socket = stream.try_clone().unwrap();
        let mut end = Remote::debuggee(debuggee, reader, stream);
        let ran = end.run_until(ns);
        // The reader's clone of the socket would otherwise stay open after
        // an error here, and the debugger wait for ever for an end that has
        // gone: shut, the debugger's run ends and the test fails instead.
        if ran.is_err() {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
        ran.unwrap();
        end.machine
    });
    let stream = TcpStream::connect(addr).unwrap();
    stream.set_nodelay(true).unwrap();
    let reader = stream.try_clone().unwrap();
    let mut near = Remote::debugger(debugger, reader, stream);
    near.run_until(ns).unwrap();
    (near.machine, far.join().unwrap())
}

/// **The cable over TCP is the cable in process.**  The same two machines
/// run twice --- under `Lashup`, and as two ends on a connection, each on
/// its own thread with the protocol's promises going over the wire as
/// messages --- and the debuggee halts at the same PC, the debugger reads
/// the same words, and the step lands the same.  Machine time is the only
/// time either end keeps.
#[test]
fn the_cable_over_tcp_is_the_cable_in_process() {
    let mut reference = Lashup::new(booted(halt_read_step_read()), straight_line());
    reference.run_until(120_000).unwrap();
    let (a, b) = over_tcp(booted(halt_read_step_read()), straight_line(), 120_000);

    let (am, rm) = (a.machine(), reference.debugger.machine());
    eprintln!(
        "over TCP the debuggee halted at PC {:o}, stepped to {:o}; in process {:o} and {:o}",
        am.amem[0o101], am.amem[0o102], rm.amem[0o101], rm.amem[0o102]
    );
    assert_eq!(b.spy_read(spy::FLAG_1) & 0x100, 0, "the debuggee is halted");
    assert_eq!(am.bus_error, 0, "every cycle of the debugger's was answered");
    assert_eq!(a.bus_cycles(), 15);
    assert_eq!(am.amem[0o101], rm.amem[0o101], "the PC read over TCP is the PC read in process");
    assert_eq!(am.amem[0o102], rm.amem[0o102], "and after the step");
    assert_eq!(am.amem[0o102], am.amem[0o101] + 1);
    assert_eq!(b.pc(), reference.debuggee.pc(), "the debuggee stands where the reference does");
    assert_eq!(b.pc() as u32, am.amem[0o102]);
    assert!(!b.debug_busy(), "the cable is quiet");
}

/// **A timeout crosses the wire as a release.**  The debugger's read of an
/// address nothing on the debuggee answers is given up on by the
/// debugger's own interface; its release goes over the connection and the
/// debuggee's `DBUB MASTER` lets go, nothing flagged there.
#[test]
fn a_timeout_over_tcp_releases_the_debuggee() {
    let mut a = DebugProgram::new();
    a.dbg_read(0o760100, 0o101);
    let (a, b) = over_tcp(booted(a.finish()), straight_line(), 60_000);
    assert_eq!(a.machine().bus_error, bus_error::UNIBUS_NXM, "the debugger's own timeout");
    assert_eq!(a.machine().amem[0o101], 0xffff, "the open bus");
    assert!(!b.debug_busy(), "the debuggee's master let go");
    assert_eq!(b.debug_ack(), None);
    assert_eq!(b.machine().bus_error, 0);
}

// --- The Unibus map, for the debug master --------------------------------------

/// **A debug master reads and writes the debuggee's memory through its
/// Unibus map, two Unibus words to the Xbus word, and the map refuses what
/// it should.**  CC's `DBG-SETUP-UNIBUS-MAP` loads map register `17` with
/// the Xbus page, valid and writable; `DBG-WRITE-XBUS` writes the low half
/// into the page's write buffer and the high half as the Xbus write;
/// `DBG-READ-XBUS` reads the low half as the Xbus read, the high half from
/// the read buffer.  A page marked read-only refuses the Xbus write, an
/// invalid page refuses the Xbus read, each without answering and with `UB
/// MAP ERROR` in the status; the buffer halves answer regardless.
/// `CC-WRITE-MD`: map register `16` loaded with `177000`, and a write
/// through it lands in the halted debuggee's `MD` and nowhere in memory.
#[test]
fn a_debug_master_writes_the_debuggees_md() {
    let mut r = straight_line();
    for _ in 0..10 {
        r.step().unwrap();
    }
    dbg_write(&mut r, unibus(spy::CLK), 0);
    dbg_write(&mut r, 0o766174, 0o177000);
    assert_eq!(r.machine().unibus_map[0o16], 0o177000, "valid, writable, page 37000");
    let md = r.machine().md;
    let main = r.machine().main.clone();
    dbg_write(&mut r, 0o174000, 0o123456);
    assert_eq!(r.machine().md, md, "the low half waits in the write buffer");
    let acked = dbg_write(&mut r, 0o174002, 0o7654);
    assert_eq!(r.machine().md, 0o7654 << 16 | 0o123456, "MD");
    assert_eq!(r.machine().bus_error, 0, "nothing refused, no Xbus cycle to time out");
    assert!(r.machine().main == main, "memory untouched");
    eprintln!("CC-WRITE-MD's high half was acknowledged at {acked} ns");
}

/// The same over the cable, as the debugger's microcode: `DebugProgram`'s
/// `dbg_write_md` is CC's three writes, the map register set once.
#[test]
fn a_debugger_writes_the_debuggees_md_over_the_cable() {
    let mut a = DebugProgram::new();
    a.dbg_write(unibus(spy::CLK), 0);
    a.dbg_write_md(0o1234_5670);
    a.dbg_write_md(0o76_543_210);
    let mut lashup = Lashup::new(booted(a.finish()), straight_line());
    lashup.run_until(120_000).unwrap();
    let (a, b) = (lashup.debugger.machine(), lashup.debuggee.machine());
    assert_eq!(a.bus_error, 0, "every cycle of the debugger's was answered");
    assert_eq!(b.bus_error, 0, "the map refused nothing");
    assert_eq!(b.unibus_map[0o16], 0o177000, "map 16 addresses MD");
    assert_eq!(b.md, 0o76_543_210, "the second word in the debuggee's MD");
    assert_eq!(lashup.debugger.bus_cycles(), 3 * (1 + 1 + 2 + 2), "map once, four halves");
}

/// **Both cables: each machine reads the other's PC.**  MIT's lashup ran a
/// cable each way, so that whichever machine was up could debug the
/// other.  Here both run the debugger's microcode: A `DBG-READ`s B's PC
/// through its DBGOUT while B runs, and later B reads A's three times
/// through its own while A runs; both reads land, both are PCs in the
/// other's program, and no cycle of either times out.  Not at the same
/// instant: two machines each in a debug read of the other hold each
/// other's Unibus until both time out, which is the next test.
#[test]
fn both_machines_read_each_others_pc_over_the_two_cables() {
    let mut a = DebugProgram::new();
    a.dbg_read(unibus(spy::PC), 0o101);
    let mut b = DebugProgram::new();
    b.wait(120);
    for _ in 0..3 {
        b.dbg_read(unibus(spy::PC), 0o101);
    }
    let mut lashup = Lashup::new(booted(a.finish()), booted(b.finish()));
    lashup.run_until(120_000).unwrap();
    let (a, b) = (lashup.debugger.machine(), lashup.debuggee.machine());
    assert_eq!(a.bus_error, 0, "every cycle of A's was answered");
    assert_eq!(b.bus_error, 0, "and every cycle of B's");
    let (pc_of_b, pc_of_a) = (a.amem[0o101] as u16, b.amem[0o101] as u16);
    eprintln!("A read B's PC as {pc_of_b:o}; B read A's PC as {pc_of_a:o}");
    assert!(pc_of_b > 0 && pc_of_b < 0o200, "B's PC, in its wait");
    assert!(pc_of_a > 0o102 && pc_of_a < 0o1000, "A's PC, past its program among the fillers");
    assert_eq!(lashup.debugger.debug_cycles(), 1, "A's one read");
    assert_eq!(lashup.debuggee.debug_cycles(), 3, "B's three");
}

/// **Both cables at the same instant: the two reads hold each other up.**
/// Each processor is master of its own Unibus for its read of its DBGOUT,
/// and each DBGIN wants the other's Unibus to answer; neither gets it until
/// both reads time out.  The 26 microsecond timeouts release both cables
/// at once, the reads go on with no word (a Unibus timeout each), and the
/// reads that follow, one machine at a time, land.
#[test]
fn simultaneous_reads_over_the_two_cables_time_out_and_the_next_ones_land() {
    let mut a = DebugProgram::new();
    a.dbg_read(unibus(spy::PC), 0o101);
    a.dbg_read(unibus(spy::PC), 0o102);
    let mut b = DebugProgram::new();
    b.dbg_read(unibus(spy::PC), 0o101);
    b.wait(140);
    b.dbg_read(unibus(spy::PC), 0o102);
    let mut lashup = Lashup::new(booted(a.finish()), booted(b.finish()));
    lashup.run_until(120_000).unwrap();
    let (a, b) = (lashup.debugger.machine(), lashup.debuggee.machine());
    assert_eq!(a.bus_error, bus_error::UNIBUS_NXM, "A's first read timed out");
    assert_eq!(b.bus_error, bus_error::UNIBUS_NXM, "and so did B's");
    assert_eq!(lashup.debugger.debug_cycles(), 2, "A put both reads on the cable");
    assert_eq!(lashup.debuggee.debug_cycles(), 2, "and so did B");
    let (pc_of_b, pc_of_a) = (a.amem[0o102] as u16, b.amem[0o102] as u16);
    eprintln!("A read B's PC as {pc_of_b:o}; B read A's PC as {pc_of_a:o}");
    assert!(pc_of_b > 0o26 && pc_of_b < 0o240, "B's PC, in its wait");
    assert!(pc_of_a > 0o204 && pc_of_a < 0o1000, "A's PC, past its program among the fillers");
}

#[test]
fn a_debug_master_reads_and_writes_memory_through_the_unibus_map() {
    let mut r = straight_line();
    for _ in 0..10 {
        r.step().unwrap();
    }
    dbg_write(&mut r, unibus(spy::CLK), 0);
    // Map 17 onto physical page 2, valid and writable, as CC does.
    dbg_write(&mut r, 0o766176, 0o140002);
    let low = 0o140000 + 0o17 * 0o2000;
    dbg_write(&mut r, low, 0o123456);
    assert_eq!(r.machine().main[0o1000], 0, "the low half waits in the write buffer");
    let acked = dbg_write(&mut r, low + 2, 0o7654);
    assert_eq!(r.machine().main[0o1000], 0o7654 << 16 | 0o123456, "the Xbus write, both halves");
    eprintln!("the mapped Xbus write was acknowledged at {acked} ns");
    r.machine_mut().main[0o1001] = 0o1234_5670;
    let (_, w) = dbg_read(&mut r, low + 4);
    assert_eq!(w, (0o12345670u32 & 0xffff) as u16, "the low half of word 1001, an Xbus read");
    let (_, w) = dbg_read(&mut r, low + 6);
    assert_eq!(w, (0o1234_5670u32 >> 16) as u16, "the high half, from the read buffer");
    assert_eq!(r.machine().bus_error, 0, "nothing refused so far");

    // Map 16 onto page 2 read-only: the buffer takes the low half, the
    // Xbus write is refused and never acknowledged.
    dbg_write(&mut r, 0o766174, 0o100002);
    let ro = 0o140000 + 0o16 * 0o2000;
    dbg_write(&mut r, ro, 0o777);
    request(&mut r, DEBUG_MODIFIER, true, (ro >> 17) as u16 & 1);
    request(&mut r, DEBUG_ADDRESS, true, ((ro + 2) >> 1) as u16);
    let at = r.ns();
    r.debug_request(
        at,
        DebugRequest { strobe: DEBUG_CYCLE, write: true, dbd: 0o666, hold_ns: 100 },
    );
    for _ in 0..40 {
        r.step().unwrap();
    }
    assert_eq!(r.debug_ack(), None, "the refused write is not acknowledged");
    assert_eq!(r.machine().main[0o1000], 0o7654 << 16 | 0o123456, "and wrote nothing");
    assert_eq!(r.machine().bus_error, bus_error::UB_MAP_ERROR, "UB MAP ERROR");
    r.debug_release(r.ns());
    let (_, _, status) = request(&mut r, DEBUG_STATUS, false, 0);
    assert_eq!(status, Some(0xff00 | bus_error::UB_MAP_ERROR), "as CC's DBG-PRINT-STATUS shows it");

    // Map 15 invalid: the Xbus read is refused; the buffer read still
    // answers, with whatever the buffer holds.
    dbg_write(&mut r, 0o766172, 0);
    let bad = 0o140000 + 0o15 * 0o2000;
    request(&mut r, DEBUG_MODIFIER, true, (bad >> 17) as u16 & 1);
    request(&mut r, DEBUG_ADDRESS, true, (bad >> 1) as u16);
    let at = r.ns();
    r.debug_request(at, DebugRequest { strobe: DEBUG_CYCLE, write: false, dbd: 0, hold_ns: 100 });
    for _ in 0..40 {
        r.step().unwrap();
    }
    assert_eq!(r.debug_ack(), None, "the refused read is not acknowledged");
    r.debug_release(r.ns());
    let (_, w) = dbg_read(&mut r, bad + 2);
    assert_eq!(w, 0, "the read buffer of a page never read through");
}

/// **`DBG-WRITE-XBUS` and `DBG-READ-XBUS` from the debugger's microcode,
/// machine to machine.**  Eighteen cycles: the map register, the two
/// halves written, the map register again, the two halves read back ---
/// the word lands in the debuggee's memory and comes back whole.
#[test]
fn a_debugger_writes_and_reads_the_debuggees_memory_over_the_cable() {
    let mut a = DebugProgram::new();
    a.dbg_write(unibus(spy::CLK), 0);
    a.dbg_write_xbus(0o1000, 0o1234_5670);
    a.dbg_read_xbus(0o1000, 0o101, 0o102);
    let mut lashup = Lashup::new(booted(a.finish()), straight_line());
    lashup.run_until(150_000).unwrap();
    let (a, b) = (lashup.debugger.machine(), lashup.debuggee.machine());
    assert_eq!(a.bus_error, 0, "every cycle of the debugger's was answered");
    assert_eq!(b.bus_error, 0, "the map refused nothing");
    assert_eq!(b.unibus_map[0o17], 0o140000 | 2, "map 17 on page 2, valid and writable");
    assert_eq!(b.main[0o1000], 0o1234_5670, "the word in the debuggee's memory");
    assert_eq!(a.amem[0o101], 0o1234_5670 & 0xffff, "the low half read back");
    assert_eq!(a.amem[0o102], 0o1234_5670 >> 16, "and the high");
    assert_eq!(lashup.debugger.bus_cycles(), 21);
}

/// A debuggee on the wire with this test as its peer: the connected
/// stream, on which the test writes frames as a debugger would --- or as
/// no debugger would --- and the thread running the debuggee to a
/// millisecond, which returns what its run came to.
fn debuggee_on_the_wire() -> (std::net::TcpStream, std::thread::JoinHandle<Result<(), Error>>) {
    use std::net::{TcpListener, TcpStream};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let far = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream.set_nodelay(true).unwrap();
        let reader = stream.try_clone().unwrap();
        let socket = stream.try_clone().unwrap();
        let ran = Remote::debuggee(straight_line(), reader, stream).run_until(1_000_000);
        // Shut for the peer, whose reads would otherwise wait on the
        // reader's clone of the socket after the run has ended.
        let _ = socket.shutdown(std::net::Shutdown::Both);
        ran
    });
    let stream = TcpStream::connect(addr).unwrap();
    stream.set_nodelay(true).unwrap();
    (stream, far)
}

/// The run's own result; a panic on the thread --- the process aborted,
/// were this `muir` --- is what none of the runs below may end in.
fn refused(far: std::thread::JoinHandle<Result<(), Error>>) -> Error {
    match far.join().expect("the debuggee's thread panicked on a peer's message") {
        Err(e) => e,
        Ok(()) => panic!("the run ended as if nothing were wrong"),
    }
}

fn is_invalid_data(e: &Error) -> bool {
    matches!(e, Error::Io(e) if e.kind() == std::io::ErrorKind::InvalidData)
}

/// **A message for the other side ends the run with an error, not the
/// process.** The frames are fixed and any program may be the peer, so a
/// debuggee sent a debuggee's message --- an acknowledgement --- is a
/// peer that is not a debugger, and the run ends saying so.
#[test]
fn a_debuggee_sent_an_acknowledgement_ends_with_an_error() {
    let (mut peer, far) = debuggee_on_the_wire();
    Message::Ack { at: 0, word: None }.write_to(&mut peer).unwrap();
    let e = refused(far);
    assert!(is_invalid_data(&e), "{e:?}");
}

/// **A second request while one is on the cable is a peer's error, not
/// the debuggee's.** `-DEBUG OUT REQ` is one level: a debugger cannot make
/// a request before its last was acknowledged and lifted, or released.
/// A peer that sends one anyway is answered with the run's end.
#[test]
fn a_second_request_on_a_busy_cable_ends_with_an_error() {
    let (mut peer, far) = debuggee_on_the_wire();
    let request = DebugRequest {
        strobe: DEBUG_ADDRESS,
        write: true,
        dbd: 0,
        hold_ns: busint::UNIBUS_STROBE_NS,
    };
    Message::Request { at: 0, request }.write_to(&mut peer).unwrap();
    Message::Request { at: 0, request }.write_to(&mut peer).unwrap();
    let e = refused(far);
    assert!(is_invalid_data(&e), "{e:?}");
}

/// **A request in the debuggee's past by more than a cycle is refused.**
/// The lashup's scheduling lets an event fall less than a cycle behind the
/// machine it is for, and such an event is taken as of now; one further
/// back cannot have happened, and a peer that dates one so is wrong.
#[test]
fn a_request_in_the_debuggees_past_ends_with_an_error() {
    let (mut peer, far) = debuggee_on_the_wire();
    // Let the debuggee run its millisecond: a promise, and its Done back.
    Message::Promise { until: u64::MAX, seen: 0 }.write_to(&mut peer).unwrap();
    while Message::read_from(&mut peer).unwrap() != Message::Done {}
    let request = DebugRequest { strobe: DEBUG_ADDRESS, write: true, dbd: 0, hold_ns: 100 };
    Message::Request { at: 0, request }.write_to(&mut peer).unwrap();
    let e = refused(far);
    assert!(is_invalid_data(&e), "{e:?}");
}

/// **A debug read of a Unibus slave has the slave's side effects.** The
/// word a slave on the Unibus proper answers never reaches the debugger
/// --- `UDI` goes to `BUS` alone --- but the slave saw `-MSYN` and
/// answered, and the I/O board's keyboard register clears `KBD READY` on
/// being read whoever the master was. `chip` runs the netlist board and
/// has it so; `rtl` must agree.
#[test]
fn a_debug_read_of_the_keyboard_register_clears_kbd_ready() {
    let mut r = straight_line();
    r.machine_mut().ioboard.press(0o101);
    assert!(r.machine().ioboard.keyboard_ready());
    let uaddr = ioboard::KBD_LOW;
    request(&mut r, DEBUG_MODIFIER, true, (uaddr >> 17) as u16 & 1);
    request(&mut r, DEBUG_ADDRESS, true, (uaddr >> 1) as u16);
    let (_, _, word) = request(&mut r, DEBUG_CYCLE, false, 0);
    assert_eq!(word, None, "the slave's word stays on the Unibus");
    assert!(!r.machine().ioboard.keyboard_ready(), "the read cleared KBD READY on the board");
}

// --- What a peer may put on the wire ------------------------------------------

/// How long a test waits on the wire before calling the debuggee stuck.
const WIRE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(120);

/// Lets a debuggee on the wire run out its window, as a debugger with
/// nothing more to ask would: a promise of nothing for ever, renewed after
/// each acknowledgement the debuggee sends --- a promise made before that
/// is stale to it --- until it says it is done, and then the same back.
/// Returns everything it sent.
fn let_it_run(peer: &mut std::net::TcpStream) -> Vec<Message> {
    let mut acks = 0;
    Message::Promise { until: u64::MAX, seen: acks }.write_to(peer).unwrap();
    let mut got = Vec::new();
    loop {
        let m = Message::read_from(peer)
            .unwrap_or_else(|e| panic!("the debuggee said nothing for {WIRE_PATIENCE:?}: {e}"));
        got.push(m);
        match m {
            Message::Done => break,
            Message::Ack { .. } => {
                acks += 1;
                Message::Promise { until: u64::MAX, seen: acks }.write_to(peer).unwrap();
            }
            _ => {}
        }
    }
    Message::Done.write_to(peer).unwrap();
    got
}

/// The netlist board as a debuggee: the processor with the boot PROM,
/// started from the button as `muir --chip` starts it, the interface
/// board on its cables and a bare backplane behind it, as
/// [`muir::cable::DebugIn`] runs them.
fn board() -> muir::cable::DebugIn {
    use muir::cable::{Boards, DebugIn, FarEnd};
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
    DebugIn::new(&bus_n, c, clk, far)
}

/// [`board`] as the debuggee on the wire.  Returns the connected stream,
/// the board's clock as the run starts, and the thread running it for
/// `window` nanoseconds, which returns the debug cycles the board took.
fn board_on_the_wire(
    window: u64,
) -> (std::net::TcpStream, u64, std::thread::JoinHandle<Result<u64, Error>>) {
    use std::net::{TcpListener, TcpStream};

    let end = board();
    let start = end.ns();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let far = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream.set_nodelay(true).unwrap();
        let reader = stream.try_clone().unwrap();
        let socket = stream.try_clone().unwrap();
        let mut remote = Remote::debuggee(end, reader, stream);
        let ran = remote.run_until(start + window);
        let _ = socket.shutdown(std::net::Shutdown::Both);
        ran.map(|()| remote.machine.debug_cycles)
    });
    let stream = TcpStream::connect(addr).unwrap();
    stream.set_nodelay(true).unwrap();
    stream.set_read_timeout(Some(WIRE_PATIENCE)).unwrap();
    (stream, start, far)
}

/// **A hold of `u64::MAX` is a request never lifted, not an overflow.**
/// `hold_ns` comes off the wire; the instant of the lift is the
/// acknowledgement's plus it, and a sum that does not fit is the end of
/// time.  On `rtl`, the debuggee of `tests/lashup.rs`'s other wire tests.
#[test]
fn a_request_held_for_ever_is_taken_without_overflow() {
    let (mut peer, far) = debuggee_on_the_wire();
    peer.set_read_timeout(Some(WIRE_PATIENCE)).unwrap();
    let request = DebugRequest { strobe: DEBUG_ADDRESS, write: true, dbd: 0, hold_ns: u64::MAX };
    Message::Request { at: 0, request }.write_to(&mut peer).unwrap();
    let got = let_it_run(&mut peer);
    assert!(
        got.iter().any(|m| matches!(m, Message::Ack { at: 0, word: None })),
        "the strobe was acknowledged at its own instant: {got:?}"
    );
    far.join().expect("the debuggee's thread panicked on a peer's message").unwrap();
}

/// **The same on the netlist board.**  The board's end makes the lift
/// itself, `hold_ns` after `DEBUG IN ACK`, and a hold to the end of time is
/// a request still on the connector when the window runs out.
#[test]
fn the_board_takes_a_request_held_for_ever_without_overflow() {
    let (mut peer, start, far) = board_on_the_wire(50_000);
    let request = DebugRequest { strobe: DEBUG_ADDRESS, write: true, dbd: 0, hold_ns: u64::MAX };
    let at = start + 1_000;
    Message::Request { at, request }.write_to(&mut peer).unwrap();
    let got = let_it_run(&mut peer);
    assert!(
        got.iter().any(|m| matches!(m, Message::Ack { at: t, word: None } if *t == at)),
        "the strobe was acknowledged at its own instant: {got:?}"
    );
    far.join().expect("the board's thread panicked on a peer's message").unwrap();
}

/// **The next request is held while the last is still on the connector,
/// and the last is answered once.**  A debugger that has the board's
/// acknowledgement lifts its request `hold_ns` on and asks again later,
/// dating the next request where it stands; the boards, behind, still
/// have the last request on the connector with `DEBUG IN ACK` up.  The
/// next request is taken and held for its instant --- not refused as a
/// second request on a busy cable --- and until the boards reach it
/// there is no answer standing: the acknowledgement still up on the
/// connector is the last request's, already given, and is not read
/// again as the next one's.  At its instant the held request goes on and
/// is answered then.
#[test]
fn the_board_holds_the_next_request_and_answers_each_once() {
    let mut end = board();
    let t0 = end.ns();
    let first = DebugRequest { strobe: DEBUG_ADDRESS, write: true, dbd: 0o123, hold_ns: 100 };
    end.debug_request(t0, first).unwrap();
    assert_eq!(end.debug_ack(), Some((t0, None)), "the strobe is answered as it is made");

    // The boards not stepped further; the debugger, told, asks again well
    // after the lift at t0 + 100.
    let t1 = t0 + 5_000;
    let next = DebugRequest { strobe: DEBUG_MODIFIER, write: true, dbd: 0, hold_ns: 100 };
    end.debug_request(t1, next).expect("held, the last request's lift being before it");
    assert_eq!(end.debug_ack(), None, "no answer stands for a request the boards have not reached");
    assert_eq!(end.debug_in_promise(), t1, "and none is promised before its instant");

    // Stepping the boards short of it: still none, though the first
    // request's acknowledgement is up on the connector until its lift.
    while end.ns() < t1 - 1_000 {
        end.step_until(t1 - 1_000).unwrap();
        assert_eq!(
            end.debug_ack(),
            None,
            "at {} ns, no answer before the held request's instant",
            end.ns()
        );
    }
    // And at its instant the held request goes on, and is answered then.
    while end.ns() < t1 {
        end.step_until(t1).unwrap();
    }
    assert_eq!(end.debug_ack(), Some((t1, None)), "the held request, answered at its own instant");

    // A genuine second request during an unanswered one is still refused:
    // a cycle to an address nothing answers, then another request before
    // any release.
    let t2 = t1 + 5_000;
    while end.ns() < t2 {
        end.step_until(t2).unwrap();
    }
    let cycle = DebugRequest { strobe: DEBUG_CYCLE, write: false, dbd: 0, hold_ns: 100 };
    end.debug_request(t2, cycle).unwrap();
    while end.ns() < t2 + 3_000 {
        end.step_until(t2 + 3_000).unwrap();
    }
    assert_eq!(end.debug_ack(), None, "the cycle to the open bus is not acknowledged");
    let e = end.debug_request(t2 + 4_000, next).expect_err("a second request on an unanswered one");
    assert!(e.contains("already on the cable"), "{e}");
}

/// **A request far ahead of the board does not run the board to it.**  A
/// debugger that is an `rtl` engine runs ahead of a netlist debuggee by as
/// much as the two speeds differ, and its request is dated where it stands;
/// the board takes the request for its instant and runs to it a step at a
/// time, reading the wire between steps, so that a request a thousand
/// seconds ahead is one frame held and not a thousand seconds of netlist
/// inside one message.  Here the window runs out first and the request
/// never reaches the connector.
#[test]
fn a_request_far_ahead_does_not_run_the_board_to_it() {
    let (mut peer, start, far) = board_on_the_wire(50_000);
    let request = DebugRequest { strobe: DEBUG_ADDRESS, write: true, dbd: 0, hold_ns: 100 };
    Message::Request { at: start + 1_000_000_000_000, request }.write_to(&mut peer).unwrap();
    let got = let_it_run(&mut peer);
    assert!(!got.iter().any(|m| matches!(m, Message::Ack { .. })), "nothing acknowledged: {got:?}");
    let cycles = far.join().expect("the board's thread panicked on a peer's message").unwrap();
    assert_eq!(cycles, 0, "the request never reached the connector");
}

/// **A peer sending frames faster than the machine steps is held back, not
/// buffered without end.**  Twenty thousand promises at once --- more than
/// the reader keeps --- are taken as the machine gets to them, the peer's
/// writes waiting meanwhile, and the run ends as it would have.
#[test]
fn a_flood_of_promises_is_taken_as_the_machine_gets_to_them() {
    use std::io::Write;
    let (mut peer, far) = debuggee_on_the_wire();
    peer.set_read_timeout(Some(WIRE_PATIENCE)).unwrap();
    let promise = Message::Promise { until: u64::MAX, seen: 0 }.encode();
    let burst: Vec<u8> = promise.iter().copied().cycle().take(promise.len() * 20_000).collect();
    peer.write_all(&burst).unwrap();
    let_it_run(&mut peer);
    far.join().expect("the debuggee's thread panicked on a peer's message").unwrap();
}

/// **A request long after the last finds the cable idle, however still the
/// debuggee has stood.** The master's life on the cable is time: the request
/// is lifted `hold_ns` after the acknowledgement and `DBUB MASTER` clears
/// `DEBUG_RELEASE_NS` after that. A debuggee that has not run since it
/// acknowledged --- over TCP, one whose thread is late --- is asked at an
/// instant past both and must find the cable idle: the state is brought to
/// the instant asked through every step on the way, not one step towards it.
#[test]
fn a_request_long_after_the_last_finds_the_cable_idle() {
    let mut r = straight_line();
    for _ in 0..10 {
        r.step().unwrap();
    }
    // The PC, `766012`, which the tests above read the same way.
    let pc = unibus(5);
    request(&mut r, DEBUG_MODIFIER, true, (pc >> 17) as u16 & 1);
    request(&mut r, DEBUG_ADDRESS, true, (pc >> 1) as u16);
    let at = r.ns();
    let cycle = DebugRequest { strobe: DEBUG_CYCLE, write: false, dbd: 0, hold_ns: 100 };
    r.debug_request(at, cycle);
    // Run to the acknowledgement and no further: the request is still on
    // the cable, to be lifted 100 ns on and the bus let go 100 ns after.
    let mut ack = None;
    for _ in 0..1_000 {
        if let Some((a, _)) = r.debug_ack() {
            ack = Some(a);
            break;
        }
        r.step().unwrap();
    }
    let ack = ack.expect("the read of the PC is acknowledged");
    assert!(r.debug_busy(), "still on the cable at the acknowledgement");
    // The debuggee stands still; the debugger asks again five microseconds
    // on, past the lift and the release both.
    let later = ack + 100 + busint::DEBUG_RELEASE_NS + 5_000;
    r.try_debug_request(later, cycle).expect("the cable is idle by then");
}
