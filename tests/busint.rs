// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The bus interface's timing, against MIT's two specifications of it.
//!
//! Every number here is quoted: the Xbus cycle from `cadr1/xspec.text.3`, the
//! cpu-side rules from `cadr/busint.erface`, the timeout from
//! `sys/doc/disk.text`. The one figure nobody gives --- how long main memory
//! takes to answer --- is [`muir::busint::IDEAL_DEVICE_NS`], and the tests
//! that depend on it say so.

use muir::busint::{
    Busint, MEMORY_BUSY_STAGES, MEMORY_CYCLE_STAGES, MEMORY_POWER_ON_EDGE, MemoryBoard, REFRESH_NS,
    REFRESH_TRIGGER_STAGES, Responder, SETUP_NS, STROBE_NS, TIMEOUT_NS, UNIBUS_ACK_NS,
    UNIBUS_ADDRESS_NS, UNIBUS_STROBE_NS, XBUS_ACK_NS, decode, rising_edge, rising_edge_after,
};
use muir::machine::MAIN_WORDS;

/// The four regions, at their edges. Xbus memory below page `0o36000`, Xbus
/// I/O to `0o36777`, the Unibus above that; and inside Xbus I/O, the devices
/// built are the display's frame buffer and mode register and the disk
/// controller's four registers. `tests/simpletv.rs` has the display's own edges.
#[test]
fn the_bus_is_decoded_by_page() {
    let m = MAIN_WORDS;
    assert_eq!(decode(0, m), Responder::Memory(0));
    // `MAIN_WORDS` is 2 M words, 8192 pages, so
    // the last populated page is `0o17777`. Above it the Xbus memory region
    // goes on to `0o35777` with nothing in it: a read there times out rather
    // than being a different kind of address.
    assert_eq!(
        decode(0o17777 << 8, m),
        Responder::Memory(31),
        "the last word is on the last board"
    );
    assert_eq!(decode(MAIN_WORDS as u32, m), Responder::NoXbus);
    assert_eq!(decode(0o35777 << 8, m), Responder::NoXbus);
    // Xbus I/O opens with the display: `0o36000 << 8` is `0o17000000`, the
    // first word of the frame buffer.
    assert_eq!(decode(0o36000 << 8, m), Responder::Device);
    assert_eq!(decode(0o17077777, m), Responder::Device);
    assert_eq!(decode(0o17100000, m), Responder::NoXbus);
    // The disk controller, at the top of Xbus I/O.
    assert_eq!(decode(0o17377774, m), Responder::Device);
    assert_eq!(decode(0o17377777, m), Responder::Device);
    assert_eq!(decode(0o17377773, m), Responder::NoXbus);
    // The Unibus, and on it the diagnostic registers: `0o766012` is the one
    // that turns the PROM off, and it is physical `0o17773005`. `tests/spy.rs`
    // has the block and the conversion.
    assert_eq!(decode(0o37000 << 8, m), Responder::NoUnibus);
    assert_eq!(decode(0o17773005, m), Responder::Interface, "the mode register");
    assert_eq!(decode(0o17773020, m), Responder::Interface, "the interrupt status register");
    assert_eq!(decode(0o17773022, m), Responder::Interface, "the error status register");
    assert_eq!(decode(0o17773037, m), Responder::Interface, "the interrupt block repeats");
    assert_eq!(
        decode(0o17773040, m),
        Responder::Debug(0),
        "the debug block is a request on the cable to the other machine"
    );
    assert_eq!(decode(0o17773042, m), Responder::Debug(1), "766104, the status");
    assert_eq!(decode(0o17773044, m), Responder::Debug(2), "766110, the modifier");
    assert_eq!(decode(0o17773046, m), Responder::Debug(3), "766114, the address");
    assert_eq!(decode(0o17773057, m), Responder::Debug(3), "766136, the block repeats");
    assert_eq!(decode(0o17773060, m), Responder::Interface, "the first map register");
    assert_eq!(decode(0o17773077, m), Responder::Interface, "the last map register");
    assert_eq!(decode(0o17773100, m), Responder::NoUnibus, "past the block");
}

/// A read of memory: request, grant on the master clock, then the Xbus cycle.
///
/// **The bus interface's own registers, as the board reads them back.**
///
/// The cold boot's first bus cycle after the reset is a write of zero into
/// the error status register, `766044`, "Reset bus interface status" in
/// `uc-cold-disk.lisp`; it timed out until the register was here, because
/// the sixteen diagnostic registers were the only ones the model knew and
/// the netlist bus interface answers the whole block. The semantics are
/// the drawings' --- UBINTC, REQERR, UBMAP --- and MIT's description of
/// them agrees, but for the one bit `-RESET ERR` clocks from the data.
#[test]
fn the_interface_answers_its_own_registers() {
    use muir::busint::{error_status, interrupt_status};
    let mut m = Machine::new();
    let (control, control2, error, map0) = (0o17773020, 0o17773021, 0o17773022, 0o17773060);

    // The error status: the boot's two overrunning cycles left the Xbus
    // NXM bit, and the cold boot's write clears it.
    m.bus_error = muir::machine::bus_error::XBUS_NXM;
    assert_eq!(
        m.bus_read(error),
        (0xff00 | muir::machine::bus_error::XBUS_NXM | error_status::NOT_FREE) as u32
    );
    m.bus_write(error, 0);
    assert_eq!(m.bus_error, 0, "the write clears the status bits");
    assert_eq!(
        m.bus_read(error),
        (0xff00 | error_status::NOT_FREE) as u32,
        "the interface is busy with the read"
    );
    m.bus_write(error, 0o200);
    assert!(m.write_through, "bit 7 of the data turns write-through mode on");
    assert_eq!(
        m.bus_read(error),
        (0xff00 | error_status::NOT_FREE | error_status::WRITE_THROUGH) as u32
    );

    // The interrupt status: each half of the register is written from its
    // own address, and only its own bits move.
    assert_eq!(
        m.bus_read(control),
        interrupt_status::LOCAL_ENABLE as u32,
        "local mode, nothing pending"
    );
    m.bus_write(control, 0o177777);
    assert_eq!(
        m.bus_read(control),
        (interrupt_status::CONTROL_MASK | interrupt_status::LOCAL_ENABLE) as u32
    );
    m.bus_write(control2, 0o177777);
    assert_eq!(
        m.bus_read(control),
        !interrupt_status::XBUS_INTR as u32,
        "both halves written: every bit but the live one, which the disk is not raising"
    );
    assert!(m.interrupt(), "bit 15 written is a simulated Unibus interrupt");
    m.bus_write(control2, 0);
    assert!(!m.interrupt());
    assert_eq!(m.bus_read(control2), 0, "the second address reads back nothing");

    // The map is memory.
    m.bus_write(map0, 0o140017);
    m.bus_write(map0 + 0o17, 0o5);
    assert_eq!(m.bus_read(map0), 0o140017);
    assert_eq!(m.bus_read(map0 + 0o17), 0o5);
    assert_eq!(m.bus_read(map0 + 1), 0);
}

/// "it is the responsibility of the bus master to assert good address, write,
/// and data lines 80 ns. prior to asserting -XBUS.RQ", and "masters should
/// delay 50 ns. after receiving -XBUS.ACK before dropping -XBUS.RQ and
/// strobing the data".
#[test]
fn a_read_acks_a_setup_and_a_response_after_the_grant() {
    let mut b = Busint::default();
    let r = Responder::Memory(0);
    b.request(false);
    assert!(!b.granted(), "the priority logic has not looked at MEMRQ yet");
    assert!(b.busy());

    // "MEMRQ is synchronous ... the bus interface only looks at it towards
    // the end of the cycle." Nothing happens until a master clock edge.
    assert_eq!(b.poll(1_000_000, r), None, "no grant, no cycle, however long we wait");

    b.mclk_edge(1_000, r);
    assert!(b.granted(), "-MEMGRANT is low: the processor has the bus");
    assert_eq!(b.poll(1_000 + SETUP_NS - 1, r), None, "-XBUS.RQ has not even gone down");

    // The memory board takes the request at the first rising edge of its
    // own 24 MHz clock after `-XBUS RQ` finds it idle --- it is refreshing
    // itself as it comes up, until its 34th edge, 1,416 ns --- and answers
    // eleven stages later; the interface deskews the answer 60 ns on the
    // TD100 at REQLM 0C09, ten more than the specification asks.
    let idle = rising_edge(MEMORY_POWER_ON_EDGE + MEMORY_BUSY_STAGES);
    assert_eq!(idle, 1_416);
    let answered = rising_edge(rising_edge_after(idle) + MEMORY_CYCLE_STAGES);
    assert_eq!(b.poll(answered + STROBE_NS, r), None, "the board has not finished deskewing");
    let ack = b.poll(answered + XBUS_ACK_NS, r).expect("-MEMACK");
    assert!(!ack.timed_out);
    assert_eq!(ack.at, answered + XBUS_ACK_NS);
    assert_eq!(ack.loadmd_at, ack.at, "MD takes the word with the acknowledgement");

    // "The responding device then asserts -XBUS.ACK, which remains asserted
    // until the -XBUS.RQ signal is removed by the master."
    assert!(b.poll(2_000_000, r).is_some(), "-MEMACK holds until the cpu drops -MEMRQ");
    b.finish();
    assert!(!b.busy());
    assert!(!b.granted());
}

/// "Nonexistent Memory Error.  Indicates that memory (or other XBUS device)
/// failed to respond within 15 microseconds.  This error stops the transfer."
///
/// The cycle still ends --- the board times out and carries on, which is what
/// lets the boot PROM run one word past the end of page 0 and survive.
#[test]
fn nothing_answering_times_out_and_still_acks() {
    let r = Responder::NoXbus;
    let mut b = Busint::default();
    b.request(false);
    b.mclk_edge(0, r);
    assert_eq!(b.poll(TIMEOUT_NS - 1, r), None, "too early to give up");
    let ack = b.poll(TIMEOUT_NS, r).expect("the timeout ends the cycle");
    assert!(ack.timed_out);
    assert_eq!(ack.responder, r);
    b.finish();

    // On the Unibus the counter starts at the grant, after the arbitration,
    // and the timeout is acknowledged as an `SSYN` would be.
    let r = Responder::NoUnibus;
    let mut b = Busint::default();
    b.request(false);
    let granted = clock_until_granted(&mut b, r, 0);
    assert_eq!(b.poll(granted + TIMEOUT_NS + UNIBUS_ACK_NS - 1, r), None, "too early to give up");
    let ack = b.poll(granted + TIMEOUT_NS + UNIBUS_ACK_NS, r).expect("the timeout ends the cycle");
    assert!(ack.timed_out);
    assert_eq!(ack.responder, r);
}

/// Master clocks at the 220 ns the machine boots in, from the edge the
/// request rose on --- which the interface does not take it at --- until
/// `-MEMGRANT` is low. Returns the time of the granting edge.
fn clock_until_granted(b: &mut Busint, r: Responder, rose_at: u64) -> u64 {
    let mut edge = rose_at;
    while !b.granted() {
        edge += 220;
        b.mclk_edge(edge, r);
        assert!(edge < rose_at + 3_000, "never granted");
    }
    edge
}

/// **A Unibus cycle is arbitrated first, and the board keeps the bus.**
///
/// Read off the netlist bus interface and measured against it in
/// `tests/busint_netlist.rs`: the request goes through the priority PROM,
/// the grant chain and `SACK` before the request synchroniser grants it,
/// six master clocks at 220 ns, and the handshake with the board's own
/// mode register takes 500 ns more, so the boot PROM's first Unibus write
/// costs 1.8 microseconds where an Xbus write costs 300. The board is then
/// Unibus master and stays so, and the next cycle is granted in three
/// clocks. A Unibus word lands in `MD` 100 ns after `-UB SSYN`, 50 before
/// `-LMACK`.
#[test]
fn a_unibus_cycle_is_arbitrated_first_and_the_board_keeps_the_bus() {
    let mut b = Busint::default();
    let r = decode(0o17773005, MAIN_WORDS);
    assert_eq!(r, Responder::Interface, "the mode register is the board's own");
    b.request(true);
    b.mclk_edge(220, r);
    assert!(!b.granted(), "the arbitration has only started");
    assert_eq!(b.ack_at(), None);
    let granted = clock_until_granted(&mut b, r, 220);
    assert_eq!(granted, 6 * 220);
    let ack = b.poll(granted + 500, r).expect("-LMACK");
    assert_eq!(ack.at, 1_820);
    assert!(!ack.timed_out);
    b.finish();

    // The next one skips the arbitration.
    let r = decode(0o17772050, MAIN_WORDS);
    assert_eq!(r, Responder::Unibus(0o764120), "the I/O board's microsecond clock");
    b.request(false);
    let granted = clock_until_granted(&mut b, r, 10_000);
    assert_eq!(granted, 10_000 + 3 * 220);
    // The I/O board latches the count on the microsecond edge after
    // `-MSYN` and answers 315 ns on: `IoBoardTiming`, measured on the
    // netlist board.
    let msyn = granted + UNIBUS_ADDRESS_NS;
    let ssyn = b.io.answer(0o764120, false, msyn);
    assert_eq!(
        ssyn,
        890 + 10_000 + muir::busint::IOB_USEC_LOW_NS,
        "the edge after MSYN at {msyn}, plus 315"
    );
    assert_eq!(b.poll(ssyn + UNIBUS_ACK_NS - 1, r), None);
    let ack = b.poll(ssyn + UNIBUS_ACK_NS, r).expect("-LMACK");
    assert_eq!(
        ack.loadmd_at,
        ssyn + UNIBUS_STROBE_NS,
        "the word is in MD before the acknowledgement"
    );
    assert!(ack.loadmd_at < ack.at);
}

/// A write goes through the same handshake: "Write requests proceed
/// identically, except that the master asserts -XBUS.WR and the data to be
/// written on the -XBUS lines along with the address lines."
#[test]
fn a_write_takes_the_same_cycle() {
    let mut b = Busint::default();
    let r = Responder::Memory(0);
    b.request(true);
    b.mclk_edge(0, r);
    assert!(b.poll(SETUP_NS - 1, r).is_none());
    // The board takes the write at its edge after `-XBUS RQ` finds it idle,
    // which after the button is the edge after its power-on refreshes, and
    // acknowledges it eleven stages on; the interface passes a write's
    // `XACK` straight through, with no deskew.
    let idle = rising_edge(MEMORY_POWER_ON_EDGE + MEMORY_BUSY_STAGES);
    let ack = rising_edge(rising_edge_after(idle) + MEMORY_CYCLE_STAGES);
    assert!(b.poll(ack - 1, r).is_none());
    assert!(b.poll(ack, r).is_some());
}

/// The disk controller's registers answer like any other Xbus slave, so the
/// microcode's status reads cost a bus cycle each rather than nothing.
#[test]
fn the_disk_registers_answer_as_a_device() {
    let mut b = Busint::default();
    let r = decode(0o17377774, MAIN_WORDS);
    assert_eq!(r, Responder::Device);
    b.request(false);
    b.mclk_edge(0, r);
    let ack = b.poll(SETUP_NS + muir::busint::IDEAL_DEVICE_NS + XBUS_ACK_NS, r).expect("-MEMACK");
    assert!(!ack.timed_out, "the controller is there");
}

// --- the cpu on the other end of the cables ---

use muir::engine::Engine;
use muir::isa::Insn;
use muir::machine::Machine;
use muir::rtl::Rtl;

/// Runs `rtl` to the first microcycle at `pc` after `after` microcycles, and
/// returns it. The `after` is because `CLEAR-I-MEMORY` walks the PC through
/// every address in the control store, so any PROM address is "reached" early
/// and meaninglessly.
fn at(prom: &[muir::isa::Insn], pc: u16, after: u64) -> (Rtl, u64) {
    let mut m = Machine::new();
    m.load_prom(prom);
    let mut e = Rtl::new(m);
    e.boot();
    let mut n = 0;
    while n < after || e.pc() != pc {
        e.step().unwrap();
        n += 1;
    }
    (e, n)
}

/// **A stall costs time, not a microcycle.**
///
/// `WAIT` stops the cpu clock and `HANG` stops both, and neither runs a
/// microcycle that would not otherwise have run --- the cycle happens once,
/// later. So the bus's timing moves no instruction and no microcycle in the
/// boot: `tests/cosim.rs` holds the engines to the same instruction and
/// `tests/boot.rs` counts the same 416,736 to the disk. This pins the
/// microcycle count that goes with them, and the nanoseconds.
///
/// The nanoseconds are the microcycles at the 220 ns the machine comes up
/// in, plus what the boot spends stalled on the bus. Four rules of the
/// interface set the stalled part:
///
/// - the diagnostic bus's mode register answers, so `PAGE-0-PARITY-FIX`'s
///   write to `766012` costs one bus cycle and not a 10 microsecond
///   timeout;
/// - a bus cycle completes at the end of the microcycle rather than the
///   next time the cpu stalls on it, which is what MIT specifies --- "Once a
///   cycle is started it goes to completion even if the machine is then
///   stopped";
/// - a `HANG` is charged for the stretch past the cycle's own end and not
///   from its start, as `-HANG` holds off the edge that ends the cycle;
/// - the write of the mode register is a Unibus cycle, and the bus
///   interface arbitrates the Unibus before it runs one: six master clocks
///   at 220 ns, read off the netlist bus interface, 1,320 ns.
#[test]
fn the_boot_takes_the_same_microcycles_and_more_nanoseconds() {
    let mut m = Machine::new();
    m.load_prom(&muir::prom::boot_prom());
    let mut e = Rtl::new(m);
    e.boot();
    // Counted to the microcycle that *executes* `0o541`, which is where
    // `tests/boot.rs` counts its 416,736 instructions to.
    let mut cycles = 0u64;
    loop {
        e.step().unwrap();
        cycles += 1;
        if e.executed() == Some(0o541) {
            break;
        }
    }
    eprintln!("to the first disk read: {cycles} microcycles, {} ns", e.ns());
    assert_eq!(cycles, 537_848, "microcycles to the first disk read");
    // 537,848 microcycles at the 220 ns the machine comes up in is
    // 118,326,560 ns. The rest is the bus: the parity-fix loop's two
    // overrunning cycles, which nothing answers, at the 10 microseconds
    // `reqtim.prom` gives, the Unibus arbitration before the mode
    // register's write, and the loop's 256 turns each held for the memory
    // board's cycle. It is 0.2% of the boot, because the boot PROM spends
    // almost all of its time clearing memories it can reach without the
    // bus. `chip_agrees_with_rtl` holds the netlist to `rtl` cycle for
    // cycle over the window it is given.
    assert_eq!(e.ns(), 118_613_440, "nanoseconds to the first disk read");
    assert_eq!(e.ns() - 537_848 * 220, 286_880, "spent stalled on the bus");
}

/// One turn of `PAGE-0-PARITY-FIX`, which is the boot's only pair of
/// back-to-back memory cycles: a read at `0o310` and a write at `0o312`.
///
/// MIT computes the same thing by hand and gets three clocks --- "the next
/// clock (1) clocks MBUSY.SYNC from MEMRQ ... (2) loads VMA for the new
/// operation ... (3) terminates MEMSTART and starts XBUSRQ out in the bus
/// interface.  So there is 400-600 ns delay between back to back cycles" ---
/// which at the 135 to 200 ns of the three faster speeds is exactly that
/// band, and 660 ns at the 220 ns this machine boots in.
///
/// The read costs the loop one `WAIT` and no `HANG`, which is MIT's own
/// prediction: `READ IN PROGRESS` "clears 150 ns after MEMACK arrives, which
/// may be just barely in time to avoid a HANG".
#[test]
fn the_parity_loop_waits_once_a_turn() {
    let (mut e, start) = at(&muir::prom::boot_prom(), 0o310, 500_000);
    let (c0, t0) = (start, e.ns());
    e.step().unwrap();
    let mut n = start + 1;
    while e.pc() != 0o310 {
        e.step().unwrap();
        n += 1;
    }
    let (cycles, ns) = (n - c0, e.ns() - t0);
    eprintln!("one turn of PAGE-0-PARITY-FIX: {cycles} microcycles, {ns} ns");
    // Five instructions and the nopped delay slot behind the branch.
    assert_eq!(cycles, 6, "microcycles in the loop");
    // Six microcycles at 220, plus the wait for the word: the memory
    // board's 440 ns cycle, the edge before it and the deskew after, which
    // comes to three microcycles' worth of clock stopped.
    assert_eq!(ns, 9 * 220, "the read holds the loop three microcycles");
}

/// **`-LOADMD` is asynchronous with the clock**, so the word is in `MD` from
/// the strobe on, whether or not the clock ever hangs for it.
///
/// "Loads MD from MEM, asynchronous with clock.  This is a pulse which goes
/// low then high, the high-going edge loads MD" --- `busint.erface`. On the
/// board `MD`'s clock is the 74S51 at MD 1D16, `(DESTMDR AND -CLK2C) OR
/// LOADMD`.
///
/// The case that tells the two apart is a read whose word is used three
/// instructions after it starts, not two. `MEMSTART` is the microcycle after
/// the `VMA-START-READ`, `-MEMRQ` goes out on the edge that ends it, and at
/// the 220 ns the machine comes up in the acknowledgement at 80 ns and
/// `-RDFINISH` at 80 + 140 both fall inside the next microcycle. So `READ IN
/// PROGRESS` is down by the time the instruction that reads `MD` arrives,
/// nothing hangs, and what it reads is whatever `-LOADMD` has put there ---
/// and not what the clock edge after the strobe would give, which is one
/// microcycle late. `GET-AREA-ORIGINS` in microcode 323 is the shape that
/// shows it, filling its table one entry behind.
#[test]
fn md_takes_the_word_at_the_strobe_and_not_at_the_next_edge() {
    // Virtual page 0 onto physical page 0, readable; and a word to find there.
    let mut m = Machine::new();
    m.l1_map[0] = 0;
    m.l2_map[0] = 1 << 23;
    m.main[0] = 0o1234567;
    // Four instructions, all ALU class with A and M sources both zero:
    //   0  ((VMA-START-READ) SETZ)     functional destination 0o21
    //   1  ((A-MEM 100) SETZ)          the page-fault check's slot
    //   2  ((A-MEM 100) SETZ)          -MEMACK and -RDFINISH both fall here
    //   3  ((A-MEM 101) SETM MD)       reads MD without hanging
    // `IR<13:12>` = 1 selects the ALU output; `IR<8:2>` = 0 is SETZ, and
    // `IR<4:3>` set with `IR<7:5>` clear is SETM. `IR<31>`, `IR<29>` and
    // `IR<27>` make the M source functional source 2, which is MD.
    let alu = 1u64 << 12;
    let setm = (1u64 << 4) | (1 << 3);
    let a_dest = |a: u64| (1u64 << 25) | (a << 14);
    let start_read = (0o21u64 << 19) | (0o37 << 14);
    let src_md = (1u64 << 31) | (1 << 29) | (1 << 27);
    m.load_prom(&[
        Insn::new(start_read | alu),
        Insn::new(a_dest(0o100) | alu),
        Insn::new(a_dest(0o100) | alu),
        Insn::new(src_md | a_dest(0o101) | setm | alu),
    ]);
    let mut e = Rtl::new(m);
    e.boot();
    // The trap, the nopped word behind it, the four, and the write-back
    // microcycle that lands the last one's result in A memory.
    for _ in 0..8 {
        e.step().unwrap();
    }
    assert_eq!(e.machine().md, 0o1234567, "MD");
    assert_eq!(e.machine().amem[0o101], 0o1234567, "what the read of MD stored");
    // Eight microcycles at 220 ns, and the 796 ns the clock stops while
    // the memory board finishes refreshing itself after the button and
    // takes its eleven stages over the word; `chip_agrees_with_rtl` holds
    // the netlist to the same hangs over its window.
    assert_eq!(e.ns(), 8 * 220 + 796, "the clock stopped once, for the word");
}

/// The speed bits are read, so the machine leaves extra slow when told to.
///
/// `SSPEED1, SSPEED0` come out of the mode register's 74S174 at OLORD1 1A01
/// and pick the delay-line tap that ends the read phase: 160 ns at extra
/// slow, 85 at normal, plus the 60 ns restart. Microcode 323 writes `46` ---
/// `SPEED1`, `ERROR-STOP-ENABLE` and `PROM-DISABLE` --- as soon as it is in
/// charge, so an engine that stays at 220 ns is wrong from there on.
#[test]
fn the_speed_bits_pick_the_microcycle() {
    let mut m = Machine::new();
    m.load_prom(&[Insn::new(0); 4]);
    let mut e = Rtl::new(m);
    e.boot();
    e.step().unwrap();
    let t0 = e.ns();
    e.step().unwrap();
    assert_eq!(e.ns() - t0, 220, "extra slow, as the machine comes up");
    e.machine_mut().mode = muir::spy::Mode { speed1: true, ..Default::default() };
    // The 74S174 at OLORD1 1A01 is two stages on `SPEEDCLK`, which is
    // `-TPR60`: the first cycle after the register changes shifts it into
    // the first stage and runs at the old speed, and the second shifts it
    // into `SSPEED` sixty nanoseconds in, before the tap, and runs at the
    // new.
    e.step().unwrap();
    let t1 = e.ns();
    assert_eq!(t1 - t0, 220 + 220, "the change is in the first stage only");
    e.step().unwrap();
    assert_eq!(e.ns() - t1, 145, "normal, which is what 46 asks for");
}

// --- the board's own side of it ---

/// The two delay lines on VCTL1 are what ends a memory cycle, and `chip` did
/// not have them: a delay line computes nothing, so [`muir::part::behaviour`]
/// has none for it and the engine skipped the package altogether. `-MEMACK`
/// then arrived and nothing happened.
///
///     1D28  74S08O  -MFINISH = -RESET AND -MEMACK
///     1D23  TD50    -MFINISHD at the third tap, 30 ns; the fourth, 40 ns,
///                   feeds the next line
///     1D22  TD250   -RDFINISH at the second tap, 100 ns
///
/// So `MBUSY` clears 30 ns after the acknowledgement and `READ IN PROGRESS`
/// 140 ns after it --- MIT's "about 150 ns", from the drawings.
#[test]
fn the_delay_lines_carry_memack_to_mfinishd_and_rdfinish() {
    use muir::chip::Chip;
    use muir::clock::{Behavioural, Clock};
    use muir::netlist;
    use muir::part::Level;

    let n = netlist::parse(include_str!("../data/CADR.netlist")).unwrap();
    let mut c = Chip::new(&n);
    c.power_on();
    c.settle();
    let mut clk = Behavioural::new();
    let net = |name: &str| n.by_name_id(name).unwrap();
    let (memack, mfinishd, rdfinish) = (net("-MEMACK"), net("-MFINISHD"), net("-RDFINISH"));

    // Settle the lines with the acknowledgement high, as it is when nothing
    // is answering.
    c.drive(memack, Level::High);
    while clk.time_ns() < 1_000 {
        c.tick(&mut clk);
    }
    assert_eq!(c.read(&[mfinishd]), 1, "-MFINISHD follows -MFINISH high");
    assert_eq!(c.read(&[rdfinish]), 1);

    // The memory answers.
    c.drive(memack, Level::Low);
    let at = clk.time_ns();
    let mut fell = (None, None);
    while clk.time_ns() < at + 400 {
        c.tick(&mut clk);
        if fell.0.is_none() && c.read(&[mfinishd]) == 0 {
            fell.0 = Some(clk.time_ns() - at);
        }
        if fell.1.is_none() && c.read(&[rdfinish]) == 0 {
            fell.1 = Some(clk.time_ns() - at);
        }
    }
    let (a, b) = (fell.0.expect("-MFINISHD never fell"), fell.1.expect("-RDFINISH never fell"));
    eprintln!("-MEMACK to -MFINISHD {a} ns, to -RDFINISH {b} ns");
    // Sampled at clock transitions, so each is the first tick at or after the
    // tap. The taps themselves are 30 and 140.
    assert!((30..=30 + 90).contains(&a), "-MFINISHD fell after {a} ns");
    assert!((140..=140 + 90).contains(&b), "-RDFINISH fell after {b} ns");
    assert!(a < b, "the second line is downstream of the first");
}

/// A cycle on the Unibus releases no memory board: the board of the last
/// memory cycle is not held by it. Measured in the band at 2,545,496, where
/// a refresh on board 0 waited, in the twin, for a cycle to the I/O board.
#[test]
fn a_unibus_cycle_holds_no_memory_board() {
    let mut b = Busint::default();
    // A memory cycle on board 0, released 30 ns after its acknowledgement;
    // the edge that grants it is the first the board's synchroniser sees
    // of its first refresh, due long before.
    b.request(false);
    b.mclk_edge(100_000, Responder::Memory(0));
    let ack = b.poll(200_000, Responder::Memory(0)).expect("acknowledged").at;
    b.released(ack + 30);
    b.finish();
    // A Unibus cycle: its grant edge makes the refresh a request, and its
    // release, far off, must not be what the refresh waits for.
    b.request(false);
    b.mclk_edge(101_000, Responder::Unibus(0o764112));
    b.released(300_000);
    b.mclk_edge(101_220, Responder::Unibus(0o764112));
    assert!(
        !b.memory_board(0).time_for_refresh(102_000),
        "board 0 refreshed at once, not after the Unibus cycle's release"
    );
}

/// The memory board's timing as the netlist board measured it in
/// `tests/cadrm_netlist.rs` and in the boot, on the 24 MHz oscillator's
/// rising edges, 125/3 ns apart: a request is taken at the first edge
/// strictly after it and acknowledged eleven stages on; a cycle holds the
/// board twelve stages from its edge, and a request that lands in a refresh
/// is taken at the edge after the refresh's twelfth.
#[test]
fn the_memory_board_answers_on_its_own_clock_and_refreshes_between() {
    let mut m = MemoryBoard::default();
    // The board is refreshing itself twice as it comes up; a request made
    // meanwhile waits for the edge after the second one's busy.
    m.xbus_clock(220);
    m.xbus_clock(440);
    let first = m.request(450);
    let idle = rising_edge(MEMORY_POWER_ON_EDGE + MEMORY_BUSY_STAGES);
    assert_eq!(
        first,
        rising_edge(rising_edge_after(idle) + MEMORY_CYCLE_STAGES),
        "taken at the edge after the board comes up"
    );

    // A request at each phase of the clock: taken at the next edge.
    for k in 0..8u64 {
        let edge = rising_edge_after(100_000 + 1_000 * k);
        let rq = rising_edge(edge) + 5 * k;
        let taken = rising_edge_after(rq);
        assert_eq!(
            taken,
            if 5 * k < rising_edge(edge + 1) - rising_edge(edge) { edge + 1 } else { edge + 2 }
        );
        assert_eq!(m.request(rq), rising_edge(taken + MEMORY_CYCLE_STAGES));
    }

    // Back to back: the second request waits for the first cycle's busy
    // to end and is taken at the edge after that.
    let a = m.request(200_000);
    let taken = rising_edge_after(200_000);
    let b = m.request(200_010);
    assert_eq!(a, rising_edge(taken + MEMORY_CYCLE_STAGES));
    assert_eq!(b, rising_edge(taken + MEMORY_BUSY_STAGES + 1 + MEMORY_CYCLE_STAGES));

    // The first refresh is due REFRESH_NS after the one-shot first ran,
    // an edge into the refresh cycle the board came up with, synchronised
    // by two clock edges; a request landing after its edge is acknowledged
    // at the edge after its busy, plus a cycle.
    let refresh_time = rising_edge(MEMORY_POWER_ON_EDGE + REFRESH_TRIGGER_STAGES) + REFRESH_NS;
    for k in [0u64, 100, 200, 300, 400] {
        let mut m = MemoryBoard::default();
        let e1 = ((refresh_time / 220) + 1) * 220;
        m.xbus_clock(e1);
        m.xbus_clock(e1 + 220);
        let refresh = rising_edge_after(e1 + 220);
        let rq = rising_edge(refresh) + k;
        assert_eq!(
            m.request(rq),
            rising_edge(refresh + MEMORY_BUSY_STAGES + 1 + MEMORY_CYCLE_STAGES),
            "landing {k} ns into the refresh"
        );
    }

    // An Xbus clock edge in the same instant as the one-shot's end is not
    // seen by the synchroniser, which samples the line as it was: the
    // refresh request then comes one edge later.
    {
        let e = ((refresh_time / 220) + 1) * 220;
        let mut coincident = MemoryBoard::default();
        coincident.xbus_clock(refresh_time);
        coincident.xbus_clock(e);
        let mut after = MemoryBoard::default();
        after.xbus_clock(refresh_time + 1);
        after.xbus_clock(e);
        // Past the busy of a refresh requested at `e`.
        let probe = rising_edge(rising_edge_after(e) + MEMORY_BUSY_STAGES) + 100;
        after.xbus_clock(probe);
        coincident.xbus_clock(probe);
        assert!(
            !after.time_for_refresh(probe),
            "an edge a nanosecond after the one-shot's end counts, and the refresh has run"
        );
        assert!(
            coincident.time_for_refresh(probe),
            "the coincident edge counted for nothing, and the refresh is only now requested"
        );
    }

    // A refresh whose request comes up during a cpu cycle waits for `IDLE`,
    // which returns when `-BUSY` lifts at the twelfth edge and the master
    // has lifted its request: 30 ns after the acknowledgement for a
    // write, which is before the twelfth edge, so the refresh is taken at
    // the thirteenth; 90 for a read, the interface's deskew and all, which
    // is after it, so at the fourteenth.
    for (release, edge) in [(30u64, 13u64), (90, 14)] {
        let mut m = MemoryBoard::default();
        let e1 = ((refresh_time / 220) + 1) * 220;
        let cpu = rising_edge_after(e1 - 100);
        let ack = m.request(e1 - 100);
        assert_eq!(ack, rising_edge(cpu + MEMORY_CYCLE_STAGES));
        m.released(ack + release);
        m.xbus_clock(e1);
        m.xbus_clock(e1 + 220);
        assert!(
            rising_edge(cpu + MEMORY_BUSY_STAGES) > e1 + 220,
            "the refresh request is up while the cycle runs"
        );
        m.xbus_clock(e1 + 440);
        m.xbus_clock(e1 + 660);
        m.xbus_clock(e1 + 880);
        // `TIME FOR REFRESH` falls with `-REFRESH NOW`, the edge after the
        // one that takes the refresh.
        let taken = cpu + edge;
        assert!(
            m.time_for_refresh(rising_edge(taken + REFRESH_TRIGGER_STAGES) - 1),
            "released {release} ns after the ack"
        );
        assert!(
            !m.time_for_refresh(rising_edge(taken + REFRESH_TRIGGER_STAGES)),
            "released {release} ns after the ack: the one-shot runs from the refresh's first stage"
        );
    }
}

/// `Busint` clocks the thirty-two twins only at an edge that can move one
/// of them --- clocking them all at every edge was a quarter of an `rtl`
/// run --- and the skip has to be invisible. So a `Busint` is driven
/// through a long run of cycles on boards that walk the backplane, with
/// idle stretches, `ILONG` cycles and the Unibus reset, beside a set of
/// twins clocked by hand at every edge, and the two must be the same
/// twins after every step.
#[test]
fn the_twins_are_the_same_whether_or_not_every_edge_clocks_them() {
    use muir::busint::MEMORY_BOARDS;
    let mut b = Busint::default();
    let mut reference = vec![MemoryBoard::default(); MEMORY_BOARDS];
    let mut now = 0u64;
    // The cycle in flight: its board, and when the request lifts once the
    // acknowledgement has come.
    let mut inflight: Option<(u8, Option<u64>)> = None;
    let mut cycles = 0u64;
    let reset = 130_000..131_000;
    for step in 0..400_000u64 {
        now += if step % 11 == 5 { 260 } else { 220 };
        if step == reset.start {
            b.unibus_reset(now, true);
            reference.iter_mut().for_each(|m| m.unibus_reset(now, true));
        }
        if step == reset.end {
            b.unibus_reset(now, false);
            reference.iter_mut().for_each(|m| m.unibus_reset(now, false));
        }
        // Busy for two thousand edges, idle for one, and nothing during
        // the reset.
        let busy = (step / 1_000) % 3 != 2 && !reset.contains(&step);
        let board = ((step / 3) % MEMORY_BOARDS as u64) as u8;
        let requesting = inflight.is_none() && busy && step % 3 == 0;
        if requesting {
            b.request(step % 2 == 0);
            inflight = Some((board, None));
        }
        let responder = Responder::Memory(inflight.map_or(board, |(k, _)| k));
        b.mclk_edge(now, responder);
        // The reference: every twin clocked, then the request taken, in
        // the order `mclk_edge` does it.
        reference.iter_mut().for_each(|m| m.xbus_clock(now));
        if requesting {
            reference[board as usize].request(now + SETUP_NS);
            cycles += 1;
        }
        // The cpu's side: the acknowledgement, `-MFINISHD` thirty
        // nanoseconds on, and the request lifted there.
        if let Some((k, lifts)) = inflight {
            match lifts {
                None => {
                    if let Some(ack) = b.poll(now, responder) {
                        b.released(ack.at + 30);
                        reference[k as usize].released(ack.at + 30);
                        inflight = Some((k, Some(ack.at + 30)));
                    }
                }
                Some(at) if now >= at => {
                    b.finish();
                    inflight = None;
                }
                Some(_) => {}
            }
        }
        assert_eq!(b.memory_boards(), &reference[..], "step {step}, {now} ns");
    }
    eprintln!("{cycles} memory cycles over {now} ns, twins identical throughout");
    assert!(cycles > 20_000, "the run should have made many cycles: {cycles}");
}

/// **Main memory is as many boards as the machine is given.** The count
/// `--main-memory-boards` sets sizes the model memory and the interface's
/// twins alike, and the decode calls everything from its end to the Xbus
/// I/O space nonexistent. Sixty is the most: the I/O space begins at the
/// sixty-first board's first word, and is answered first whatever the
/// memory's size.
#[test]
fn main_memory_is_as_many_boards_as_the_machine_is_given() {
    use muir::busint::{MAX_MEMORY_BOARDS, decode};
    use muir::machine::Machine;
    use muir::rtl::Rtl;

    assert_eq!(Machine::new().memory_boards(), MAIN_WORDS >> 16, "thirty-two by default");
    let m = Machine::with_memory_boards(4);
    assert_eq!(m.main.len(), 4 << 16);
    assert_eq!(decode((3 << 16) + 5, m.main.len()), Responder::Memory(3));
    assert_eq!(decode(4 << 16, m.main.len()), Responder::NoXbus, "the fifth board is not there");
    let r = Rtl::new(m);
    assert_eq!(r.machine().memory_boards(), 4);

    assert_eq!(MAX_MEMORY_BOARDS, 60);
    let sixty = Machine::with_memory_boards(60);
    assert_eq!(decode((59 << 16) + 1, sixty.main.len()), Responder::Memory(59));
    assert_ne!(
        decode(60 << 16, 61 << 16),
        Responder::Memory(60),
        "the sixty-first board's place is the Xbus I/O space, whatever the memory's size"
    );
}

// --- the netlist interface, and the model boards behind it ---

/// The bus interface netlist, powered and settled with every connector wire
/// pulled up and the processor's cables at rest, as `tests/busint_netlist.rs`
/// brings it up.
fn interface() -> (muir::netlist::Netlist, muir::chip::Chip) {
    use muir::part::Level;
    let n = muir::netlist::parse(include_str!("../data/BUSINT.netlist")).unwrap();
    let find = |name: &str| n.by_name_id(name).or_else(|| n.by_name_id(&format!("'{name}'")));
    let net = |name: &str| find(name).unwrap_or_else(|| panic!("no net {name}"));
    let mut c = muir::chip::Chip::new_unclocked(&n);
    let reqtim = muir::prom::parse_mit(include_str!("../mit/cadr1/reqtim.prom")).unwrap();
    let uprior = muir::prom::parse_mit(include_str!("../mit/cadr1/uprior.prom")).unwrap();
    c.load_rom("0A02", "74S288", &reqtim);
    c.load_rom("0D09", "74S472", &uprior);
    c.power_on();
    for line in include_str!("../data/busint-connectors.txt").lines() {
        if let Some((_, wire)) = line.split_once('|')
            && let Some(id) = find(wire.trim())
        {
            c.pull_up(id);
        }
    }
    for a in 0..22 {
        c.drive(net(&format!("-ADR{a}")), Level::High);
    }
    for (name, level) in [
        ("-ADRPAR", Level::High),
        ("-MEMRQ", Level::High),
        ("WRCYC", Level::Low),
        ("-LM UNIBUS RESET", Level::High),
        ("-MCLK7", Level::High),
        ("-LM POWER RESET", Level::Low),
    ] {
        c.drive(net(name), level);
    }
    c.settle_all();
    c.drive(net("-LM POWER RESET"), Level::High);
    c.settle();
    (n, c)
}

/// The processor's Unibus reset comes over the cable as `-LM UNIBUS RESET`,
/// is `RESET` at DBGIN 0A14, and goes out on the backplane as `-XBUS INIT`
/// (XA 0F21) and `-UB INIT` (0F06). Behind the netlist interface under
/// `chip`, the model boards are [`Buses`]'s, and its far end resets them
/// when the interface asserts the wire --- each board what its own reset
/// line clears: the display's vertical flag, the disk controller's command
/// register, the I/O board's interrupt enables.
#[test]
fn a_bus_reset_through_the_interface_reaches_the_model_boards() {
    use muir::buses::Buses;
    use muir::disk_controller::REGS;
    use muir::ioboard::{self, csr};
    use muir::part::Level;
    use muir::simpletv::{CONTROL, mode};

    let (n, mut c) = interface();
    let mut m = Machine::new();
    m.ns = 1_000;
    m.bus_write(CONTROL, mode::INTERRUPT_ENABLE | mode::VERT);
    m.bus_write(muir::busint::unibus_physical(ioboard::CSR), csr::WRITABLE as u32);
    m.bus_write(REGS, 1 << 11);
    assert!(m.simpletv.interrupt(1_000) && m.disk.interrupt());
    assert_eq!(m.ioboard.csr() & csr::WRITABLE, csr::WRITABLE);
    let mut buses = Buses::new(&n, m);
    let ubreset = n.by_name_id("'-LM UNIBUS RESET'").unwrap();

    buses.tick(&mut c, 2_000);
    assert!(buses.machine.simpletv.interrupt(2_000), "nothing has reset yet");

    c.drive(ubreset, Level::Low);
    c.settle();
    buses.tick(&mut c, 3_000);
    let m = &buses.machine;
    assert!(!m.simpletv.vert_flag(3_000), "the display's flag");
    assert_eq!(m.simpletv.mode(), mode::INTERRUPT_ENABLE, "and not its mode register");
    assert!(!m.disk.interrupt(), "the controller's command register");
    assert_eq!(m.ioboard.csr() & csr::WRITABLE, 0, "the I/O board's enables");

    // Released: nothing more happens, and a write lands again.
    c.drive(ubreset, Level::High);
    c.settle();
    buses.tick(&mut c, 4_000);
    buses.machine.bus_write(CONTROL, mode::INTERRUPT_ENABLE | mode::VERT);
    buses.tick(&mut c, 5_000);
    assert!(buses.machine.simpletv.interrupt(5_000));
}

/// **The processor's own `PROG.UNIBUS.RESET` reaches the model boards on
/// the engines that run them.** A PROM that writes `INTERRUPT-CONTROL`
/// with bit 28 --- the boot PROM's own "RESET THE BUS INTERFACE AND I/O
/// DEVS" --- clears the same three things on `rtl` and on `micro` as the
/// wire does under `chip` above: the display's vertical flag, the disk
/// controller's command register, the I/O board's interrupt enables.
#[test]
fn the_processors_own_bus_reset_reaches_the_model_boards() {
    use muir::disk_controller::REGS;
    use muir::engine::Engine;
    use muir::ioboard::{self, csr};
    use muir::isa::Insn;
    use muir::isa::asm::{ALU, SETA, a_src, filler};
    use muir::micro::Micro;
    use muir::rtl::Rtl;
    use muir::simpletv::{CONTROL, mode};

    /// Functional destination 2, `INTERRUPT-CONTROL`: `IR<23:19>` with
    /// `IR<25>` clear, as `asm::MD` spells destination 30.
    const INTERRUPT_CONTROL: u64 = (0o2 << 19) | (0o37 << 14);

    fn armed() -> Machine {
        let mut m = Machine::new();
        m.amem[3] = 1 << 28;
        let mut prom = vec![filler(); 512];
        prom[5] = Insn::new(ALU | SETA | a_src(3) | INTERRUPT_CONTROL);
        m.load_prom(&prom);
        m.ns = 1_000;
        m.bus_write(CONTROL, mode::INTERRUPT_ENABLE | mode::VERT);
        m.bus_write(muir::busint::unibus_physical(ioboard::CSR), csr::WRITABLE as u32);
        m.bus_write(REGS, 1 << 11);
        assert!(m.simpletv.interrupt(1_000) && m.disk.interrupt());
        assert_eq!(m.ioboard.csr() & csr::WRITABLE, csr::WRITABLE);
        m
    }

    fn reset_by(e: &mut impl Engine, what: &str) {
        e.boot();
        for _ in 0..20 {
            e.step().unwrap();
        }
        let m = e.machine();
        assert!(!m.simpletv.vert_flag(m.ns), "{what}: the display's flag");
        assert_eq!(m.simpletv.mode(), mode::INTERRUPT_ENABLE, "{what}: and not its mode register");
        assert!(!m.disk.interrupt(), "{what}: the controller's command register");
        assert_eq!(m.ioboard.csr() & csr::WRITABLE, 0, "{what}: the I/O board's enables");
    }

    reset_by(&mut Rtl::new(armed()), "rtl");
    reset_by(&mut Micro::new(armed()), "micro");
}
