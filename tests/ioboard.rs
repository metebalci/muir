// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The I/O board, checked against microcode 323 and MIT's own sources.
//!
//! `the_addresses_are_mits_own` reads the constants out of the System 100
//! release rather than restating them, and says it was skipped when the
//! release is not vendored.

use muir::busint::{self, Responder, decode};
use muir::ioboard::{self, CYCLE_NS, IoBoard, SIXTY_CYCLE_NS, csr};
use muir::machine::{MAIN_WORDS, Machine, bus_error};

mod support;
use support::release;

/// `(ASSIGN NAME 764120)`, as microcode 323 writes it.
fn assign_octal(src: &str, name: &str) -> u32 {
    let line = src.lines().find(|l| l.contains(&format!("(ASSIGN {name} "))).unwrap();
    let rest = line.split(&format!("(ASSIGN {name} ")).nth(1).unwrap();
    let digits: String = rest.trim().chars().take_while(|c| c.is_digit(8)).collect();
    u32::from_str_radix(&digits, 8).unwrap()
}

/// The addresses are the ones microcode 323 uses.
#[test]
fn the_addresses_are_mits_own() {
    let Some(interrupt) = release("ucadr/uc-interrupt.lisp") else { return };
    assert_eq!(
        ioboard::USEC_LOW,
        assign_octal(&interrupt, "MICROSECOND-CLOCK-UNIBUS-ADDRESS"),
        "MICROSECOND-CLOCK-UNIBUS-ADDRESS"
    );
    assert_eq!(
        ioboard::CLOCK,
        assign_octal(&interrupt, "INTERVAL-TIMER-UNIBUS-ADDRESS"),
        "INTERVAL-TIMER-UNIBUS-ADDRESS"
    );
    assert_eq!(ioboard::USEC_HIGH, ioboard::USEC_LOW + 2, "the high half is the next word");
}

/// The CSR and `KBD LOW` are where `uc-cadr.lisp` loads them. The microcode
/// loads physical addresses and writes the Unibus address it means in a
/// comment beside each; both are checked because the conversion between
/// them is `busint::unibus_address`.
#[test]
fn the_csr_and_kbd_low_are_at_the_microcodes_physical_addresses() {
    let Some(cadr) = release("ucadr/uc-cadr.lisp") else { return };
    assert!(cadr.contains("17772045"), "the CSR's physical address");
    assert!(cadr.contains("17772040"), "KBD LOW's physical address");
    assert_eq!(busint::unibus_address(0o17772045), Some(ioboard::CSR), "764112 is the CSR");
    assert_eq!(busint::unibus_address(0o17772040), Some(ioboard::KBD_LOW), "764100 is KBD LOW");
}

/// Microcode 323 tests bit 5 of the CSR to decide how to boot, so where that
/// bit sits is MIT's own word and not a guess.
#[test]
fn the_keyboard_ready_bit_is_the_one_the_microcode_tests() {
    let mut b = IoBoard::default();
    assert!(!b.keyboard_ready(), "nothing has been typed");
    assert_eq!(b.read(ioboard::CSR, 0) & csr::KBD_READY, 0);

    // `(JUMP-IF-BIT-CLEAR (BYTE-FIELD 1 5) MD COLD-BOOT)`
    b.press(0o60);
    assert_eq!(b.read(ioboard::CSR, 0) & csr::KBD_READY, 1 << 5, "BYTE-FIELD 1 5");

    // `((MD) (BYTE-FIELD 6 0) MD)  ;Get keycode`
    assert_eq!(b.read(ioboard::KBD_LOW, 0) & 0o77, 0o60);
    assert!(!b.keyboard_ready(), "reading the scan code clears it again");
}

/// **The low half's read takes the word; the high half's leaves it.** `KBD
/// READY` is the 74LS74 at IOBKBD 0B30, cleared by `-READ.KBD.LOW` on its
/// pin 1 and by nothing else; the high half carries the top eight bits
/// under the floating byte.  The microcode reads the high half first, so
/// the word stands until its second read, and a keyboard handing over its
/// next word on `KBD READY` alone cannot slip one in between --- which,
/// while the model cleared ready on either half, lost a keystroke under
/// CC: the `)` of a form typed at the listener, which then waited for its
/// keyboard for ever.  `tests/keyboard_cable.rs` holds the netlist board
/// to the same.
#[test]
fn the_scan_code_is_two_sixteen_bit_halves() {
    let mut b = IoBoard::default();
    b.press(0o1234567);
    // Eight bits of scan code and the floating upper byte, as the netlist
    // board reads it.
    assert_eq!(b.read(ioboard::KBD_HIGH, 0), ioboard::csr::FLOATING | (0o1234567u32 >> 16) as u16);
    assert!(b.keyboard_ready(), "the high half leaves ready");
    assert_eq!(b.read(ioboard::KBD_LOW, 0), (0o1234567u32 & 0xffff) as u16);
    assert!(!b.keyboard_ready(), "the low half clears it");
}

/// Only the four flip-flops of the 74LS175 latch, and the serial
/// enable's beside them: the 74LS74 at IOBSER 0D21 takes `UBI7` on the
/// same clock, which `serial.lisp` counts on, writing bit 7 of `764112`
/// to turn its interrupt on. `tests/serial_cable.rs` reads the bit back
/// off the netlist board.
#[test]
fn the_status_register_writes_five_bits() {
    let mut b = IoBoard::default();
    b.write(ioboard::CSR, !0, 0);
    assert_eq!(b.csr(), csr::WRITABLE, "the quad D flip-flop, the serial enable, nothing else");
    assert_eq!(csr::WRITABLE, 0o217);
    assert_eq!(
        csr::WRITABLE & (csr::KBD_INT_ENABLE | csr::MOUSE_INT_ENABLE | csr::SER_INT_ENABLE),
        csr::KBD_INT_ENABLE | csr::MOUSE_INT_ENABLE | csr::SER_INT_ENABLE,
        "the three enables are inside the writable field"
    );

    // A write must not be able to claim the keyboard is ready.
    b.write(ioboard::CSR, csr::KBD_READY | csr::MOUSE_READY, 0);
    assert!(!b.keyboard_ready(), "ready comes from the keyboard, not from a write");
}

/// MIT: "Hardware synchronizes if you read this one first."
#[test]
fn reading_the_low_half_latches_the_microsecond_clock() {
    let mut b = IoBoard::default();
    // The clocks take the machine's nanoseconds; pick a time with a high
    // half, which is more than 65 milliseconds in.
    let ns = 700_000_000u64 * CYCLE_NS;
    // As the board counts: edges at 890 ns past each microsecond from
    // power-on, so 110 ns before a whole microsecond the count is already
    // the next one.
    let usec = ioboard::usec_at(ns);
    assert_eq!(ioboard::usec_at(889), 0);
    assert_eq!(ioboard::usec_at(890), 1);
    assert_eq!(ioboard::usec_at(462_742_655), 462_742, "the count at the band's first read");
    assert!(usec >> 16 != 0, "the test needs a time that spans both halves");

    let low = b.read(ioboard::USEC_LOW, ns);
    // Time moves on before the second read, as it does between two
    // microinstructions. The high half must still belong to the first.
    let high = b.read(ioboard::USEC_HIGH, ns + 145_000_000);
    assert_eq!(low, usec as u16);
    assert_eq!(high, (usec >> 16) as u16, "the high half is the latched one");
    assert_eq!(((high as u32) << 16) | low as u32, usec, "and the two make the whole");
}

/// The 60-cycle clock counts ticks of simulated time, not host time.
#[test]
fn the_sixty_cycle_clock_counts_at_sixty_hertz() {
    // The nanoseconds it takes to reach `n` ticks.
    let at = |n: u64| n * SIXTY_CYCLE_NS;

    let mut b = IoBoard::default();
    assert_eq!(b.read(ioboard::CLOCK, 0), 0);
    assert_eq!(b.read(ioboard::CLOCK, at(1) - 1), 0, "not quite a sixtieth of a second");
    assert_eq!(b.read(ioboard::CLOCK, at(1)), 1, "one tick");
    assert_eq!(b.read(ioboard::CLOCK, 1_000_000_000), 60, "a second is sixty ticks");
}

/// Written, the same address is the interval timer.
#[test]
fn the_same_address_reads_a_clock_and_writes_a_timer() {
    let mut b = IoBoard::default();
    b.write(ioboard::CLOCK, 0o1350, 0);
    assert_eq!(b.interval_timer(), 0o1350, "loaded, though nothing counts it down");
    assert_eq!(b.read(ioboard::CLOCK, 0), 0, "reading it is still the 60-cycle clock");
}

/// The board answers where nothing did, its Chaosnet interface with it,
/// and its decoder's holes and edges are the netlist board's
/// (`ioboard::answers`): `764130` reaches the microsecond clock, `764154`
/// nothing, the serial port's eight answer, and the block ends at
/// `764176`.
#[test]
fn the_board_answers_and_so_does_its_chaosnet_interface() {
    let d = |p| decode(p, MAIN_WORDS);
    assert_eq!(d(0o17772040), Responder::Unibus(0o764100), "KBD LOW");
    assert_eq!(d(0o17772045), Responder::Unibus(0o764112), "the CSR");
    assert_eq!(d(0o17772053), Responder::Unibus(0o764126), "the GPIO");
    assert_eq!(d(0o17772054), Responder::Unibus(0o764130), "the microsecond clock again");
    assert_eq!(d(0o17772060), Responder::Unibus(0o764140), "the Chaosnet CSR");
    assert_eq!(d(0o17772065), Responder::Unibus(0o764152), "start transmission");
    assert_eq!(d(0o17772066), Responder::NoUnibus, "the buffer read `A3` disables");
    assert_eq!(d(0o17772070), Responder::Unibus(0o764160), "the serial port");
    assert_eq!(d(0o17772077), Responder::Unibus(0o764176), "its last address");
    assert_eq!(d(0o17772100), Responder::NoUnibus, "past the block");
    assert_eq!(d(0o17772037), Responder::NoUnibus, "before the keyboard group");
    // And the diagnostic register that turns the PROM off still decodes.
    assert_eq!(d(0o17773005), Responder::Interface, "the mode register");
}

/// The whole path, as microcode 323 walks it: read the CSR at the physical
/// address the microcode loads, then the scan code.
#[test]
fn the_bus_reaches_the_board_at_the_microcodes_own_addresses() {
    let mut m = Machine::new();
    // Nothing typed, so not ready; `CLOCK READY` and the floating upper
    // byte read as ones, as they do on the netlist board.
    assert_eq!(
        m.bus_read(0o17772045),
        (ioboard::csr::CLOCK_READY | ioboard::csr::FLOATING) as u32,
        "nothing typed, so not ready"
    );
    assert_eq!(m.bus_error, 0, "the board answers, so the cycle does not time out");

    m.ioboard.press(0o15);
    assert_eq!(m.bus_read(0o17772045) & csr::KBD_READY as u32, csr::KBD_READY as u32);
    assert_eq!(m.bus_read(0o17772040) & 0o77, 0o15, "the keycode the microcode reads");
    assert_eq!(m.bus_error, 0);
}

/// The Chaosnet interface answers on the bus of a machine with nothing on
/// its cable: its address reads as the switches are set, a frame started
/// goes nowhere and Transmit Done comes back, and nothing ever arrives.
#[test]
fn the_chaosnet_interface_answers_with_nothing_on_its_cable() {
    use muir::chaos::interface::{self as chaos, csr};
    let mut m = Machine::new();
    let my = m.bus_read(busint::unibus_physical(chaos::MY_ADDRESS));
    assert_eq!(my, m.chaos.address as u32, "the switches");
    assert_eq!(m.bus_error & bus_error::UNIBUS_NXM, 0, "answered");
    m.bus_write(busint::unibus_physical(chaos::CSR), csr::RESET as u32);
    m.bus_write(busint::unibus_physical(chaos::CSR), csr::CLEAR_RECEIVER as u32);
    for w in [1 << 8, 4, 0o3060, 0, 0o3050, 1, 1, 0, 0x4954, 0x454d, 0o3060] {
        m.bus_write(busint::unibus_physical(chaos::WRITE_BUFFER), w);
    }
    let c = m.bus_read(busint::unibus_physical(chaos::CSR)) as u16;
    assert_eq!(c & csr::TRANSMIT_DONE, 0, "a word written clears Transmit Done");
    m.bus_read(busint::unibus_physical(chaos::START));
    // The turn timer runs free from power-on with nothing heard, bit 7
    // rising every 256 microseconds; the frame goes at the next.
    m.ns += 400_000;
    let c = m.bus_read(busint::unibus_physical(chaos::CSR)) as u16;
    assert_ne!(c & csr::TRANSMIT_DONE, 0, "sent into the void");
    assert_eq!(c & csr::RECEIVE_DONE, 0, "and nothing came");
}

/// **The mouse registers carry the counts, the buttons and the phases as
/// IOBMS2's read buffer lays them out.** Twelve bits of count an axis,
/// wrapping; the three switches over the Y count as tail, middle, head in
/// bits 12 to 14, which is the software's left 1, middle 2, right 4; the
/// four quadrature lines over the X count; and any change is `MOUSE
/// READY`, which reading the Y register clears.
#[test]
fn the_mouse_registers_carry_counts_buttons_and_phases() {
    use ioboard::mouse;
    let mut b = IoBoard::default();
    assert!(!b.mouse_ready());
    assert_eq!(b.read(ioboard::MOUSE_Y, 0), 0);
    assert_eq!(b.read(ioboard::MOUSE_X, 0), 0);

    b.mouse_move(5, -3);
    assert!(b.mouse_ready(), "a move is a status change");
    assert_eq!(b.read(ioboard::MOUSE_X, 0) & mouse::COUNT, 5, "five to the right");
    assert!(b.mouse_ready(), "reading X does not clear it");
    let y = b.read(ioboard::MOUSE_Y, 0);
    assert_eq!(y & mouse::COUNT, 0o7775, "three up: the count wraps in twelve bits");
    assert_eq!(y >> mouse::SHIFT, 0, "no buttons");
    assert!(!b.mouse_ready(), "reading Y clears it");
    assert_eq!(b.csr() & csr::MOUSE_READY, 0);

    // What TRACK-MOUSE does with it: the difference modulo 4096, signed
    // from bit 11.
    let delta = |now: u16, was: u16| ((now.wrapping_sub(was) & mouse::COUNT) as i16) << 4 >> 4;
    assert_eq!(delta(0o7775, 0), -3);
    assert_eq!(delta(5, 0), 5);
    b.mouse_move(-4096, 0);
    assert_eq!(b.read(ioboard::MOUSE_X, 0) & mouse::COUNT, 5, "a whole turn is no move at all");

    b.mouse_buttons(0o5);
    assert!(b.mouse_ready(), "a button is a status change");
    let y = b.read(ioboard::MOUSE_Y, 0);
    assert_eq!(
        y & (mouse::TAIL | mouse::MIDDLE | mouse::HEAD),
        mouse::TAIL | mouse::HEAD,
        "left and right"
    );
    assert_eq!(y >> 15, 0, "bit 15 is ground");
    b.mouse_buttons(0o5);
    assert!(!b.mouse_ready(), "the same buttons again change nothing");

    // The phases: each axis a Gray code of its count in bits 12-15, HORA,
    // HORB, VERA, VERB.
    let mut b = IoBoard::default();
    let phases: Vec<u16> = (0..5)
        .map(|_| {
            let p = b.read(ioboard::MOUSE_X, 0) >> mouse::SHIFT & 0o3;
            b.mouse_move(1, 0);
            p
        })
        .collect();
    assert_eq!(
        phases,
        [0b00, 0b10, 0b11, 0b01, 0b00],
        "HORA, HORB as a Gray sequence: one line moves a step"
    );
}

/// `-UB INIT` on the backplane is `RESET` on the board, and `-RESET` clears
/// the four enables --- the 74LS175 at IOBCSR 0D27 --- and the Chaosnet
/// interface's register, the 74LS174 at LMUCON 0B20. The ready flops clear
/// on their register's read and not on this; the mouse counters, the
/// interval timer and the scan code have no clear on it at all.
#[test]
fn a_unibus_init_clears_the_enables_and_the_chaosnet_interface_and_keeps_the_rest() {
    use muir::chaos::interface as chaos;
    use muir::ioboard::mouse;
    let mut b = IoBoard::default();
    b.plug_chaos(0o3040, None, 0, false);
    b.write(ioboard::CSR, csr::WRITABLE, 0);
    b.write(ioboard::CLOCK, 0o1234, 0);
    b.write(chaos::CSR, chaos::csr::LOOP_BACK | chaos::csr::RECEIVE_INT_ENABLE, 0);
    b.press(0o123);
    b.mouse_move(5, -3);
    assert!(b.interrupt_request(0).is_some(), "keyboard ready and enabled");

    b.unibus_init();
    assert_eq!(b.csr() & csr::WRITABLE, 0, "the 74LS175's four flops");
    assert_eq!(b.interrupt_request(0), None, "no enable, no request");
    assert!(b.keyboard_ready(), "KBD READY clears on a read of KBD LOW, not on INIT");
    assert!(b.mouse_ready(), "MOUSE READY clears on a read of MOUSE Y, not on INIT");
    assert_eq!(b.read(ioboard::KBD_LOW, 0), 0o123, "the shift register keeps the scan code");
    assert_eq!(b.mouse_x(), 5, "the 74LS569s have no clear on it");
    assert_eq!(b.mouse_y(), (-3i32).rem_euclid(1 << mouse::SHIFT) as u16);
    assert_eq!(b.interval_timer(), 0o1234, "nor has the timer");
    assert_eq!(
        b.read(chaos::CSR, 0) & (chaos::csr::LOOP_BACK | chaos::csr::RECEIVE_INT_ENABLE),
        0,
        "the Chaosnet interface's read/write bits are cleared, as at power-up"
    );
}
