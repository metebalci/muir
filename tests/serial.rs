// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The serial port as the behavioural engines have it, `src/serial.rs`
//! behind `src/ioboard.rs`: the 2651's registers against the Signetics
//! sheet, its addresses against MIT's own list, and its transmitter and
//! receiver against the frame the sheet's baud-rate table gives them.
//! `tests/serial_cable.rs` holds the netlist board to the same things.

use muir::ioboard::{self, IoBoard, csr};
use muir::serial::{self, BAUD_TENTHS, BRCLK_HZ, COMMAND, DATA, DIVISORS, Framing, MODE, STATUS};
use muir::serial::{command, mode1, mode2, status};

mod support;
use support::release;

/// The upper byte floats high on every read of the group; the low byte is
/// the chip's.
fn low(word: u16) -> u8 {
    assert_eq!(word & csr::FLOATING, csr::FLOATING, "nothing drives the upper byte");
    word as u8
}

/// A board with something plugged into J9.
fn plugged() -> IoBoard {
    let mut b = IoBoard::default();
    b.serial.cable.plug(0);
    b
}

/// `serial.lisp`'s `SERIAL-WRITE-MODE`: a read of the command register
/// to put the pointer back, then the two halves.
fn write_mode(b: &mut IoBoard, mr1: u8, mr2: u8, ns: u64) {
    b.read(COMMAND, ns);
    b.write(MODE, mr1 as u16, ns);
    b.write(MODE, mr2 as u16, ns);
}

/// `SERIAL-READ-MODE`: `(DPB (%UNIBUS-READ 764164) 1010 (%UNIBUS-READ
/// 764164))`, mode register 1 above mode register 2.
fn read_mode(b: &mut IoBoard, ns: u64) -> u16 {
    b.read(COMMAND, ns);
    let mr1 = low(b.read(MODE, ns));
    let mr2 = low(b.read(MODE, ns));
    (mr1 as u16) << 8 | mr2 as u16
}

/// 8 data bits, no parity, one stop bit, at rate `rate`: the port up
/// with both halves enabled.
fn set_up(b: &mut IoBoard, rate: u8, ns: u64) {
    write_mode(
        b,
        0o1 << mode1::STOP_SHIFT | 3 << mode1::LENGTH_SHIFT | 2,
        mode2::RX_INTERNAL | mode2::TX_INTERNAL | rate,
        ns,
    );
    b.write(
        COMMAND,
        (command::TX_ENABLE | command::RX_ENABLE | command::DTR | command::RTS) as u16,
        ns,
    );
}

/// **The addresses are MIT's own.** `sys/doc/unaddr.text` lists the four
/// with what each does; read out of the release rather than restated.
#[test]
fn the_addresses_are_mits_own() {
    let Some(text) = release("doc/unaddr.text") else { return };
    let line = |uaddr: u32| {
        text.lines()
            .find(|l| l.starts_with(&format!("{uaddr:o}")))
            .unwrap_or_else(|| panic!("unaddr.text has no line for {uaddr:o}"))
            .to_string()
    };
    assert!(line(DATA).contains("read received data, write transmit data"));
    assert!(line(STATUS).contains("read data set status"));
    assert!(line(MODE).contains("mode selection"));
    assert!(line(COMMAND).contains("command"), "{}", line(COMMAND));
    for uaddr in [DATA, STATUS, MODE, COMMAND] {
        assert_eq!(ioboard::answers(uaddr, false), Some(uaddr));
        assert_eq!(ioboard::answers(uaddr, true), Some(uaddr));
        assert_eq!(ioboard::answers(uaddr | 0o10, false), Some(uaddr | 0o10), "A3 is not decoded");
    }
}

/// **From reset, the registers read as the sheet's Table 2 says**: the
/// mode, command and status registers clear, the data bus three-stated
/// between accesses --- so the low byte is zero and the upper byte, which
/// nothing drives on this board, is ones.
#[test]
fn reset_reads_zero_below_a_floating_upper_byte() {
    let mut b = IoBoard::default();
    for uaddr in [STATUS, MODE, MODE, COMMAND, DATA] {
        assert_eq!(b.read(uaddr, 0), csr::FLOATING, "{uaddr:o} from reset");
    }
    assert_eq!(b.read(STATUS | 0o10, 0), csr::FLOATING, "the same register at 76417x");
}

/// **`serial.lisp` finds the chip.** `SERIAL-CHECK-EXISTENCE` writes 0 to
/// the command register and reads it back, then `100`: "If IOB not wired
/// for it, will read back all zero. If PCI not plugged in, will read back
/// all ones." The model reads back what was written, which is the third
/// case, the one with a port.
#[test]
fn mits_driver_finds_a_pci_on_an_iob_wired_for_it() {
    let mut b = IoBoard::default();
    b.write(COMMAND, 0, 0);
    let zeros = low(b.read(COMMAND, 0));
    b.write(COMMAND, 0o100, 0);
    let ones = low(b.read(COMMAND, 0));
    assert_ne!(ones, 0, "\"This IOB does not have serial I/O\"");
    assert_ne!(zeros, 0o377, "\"This IOB does not contain a PCI\"");
    assert_eq!((zeros, ones), (0, 0o100), "the register reads back as written");
}

/// **The mode pointer**: "The first write (or read) operation addresses
/// Mode Register 1, and a subsequent operation addresses Mode Register 2.
/// If more than the required number of accesses are made, the internal
/// sequencer recycles to point at the first register. The pointers are
/// reset to SYN1 Register and Mode Register 1 by a RESET input or by
/// performing a 'Read Command Register' operation, but are unaffected by
/// any other read or write operation." `serial.lisp` reads the command
/// register before every pair for that reason.
#[test]
fn the_mode_pointer_alternates_and_a_command_read_resets_it() {
    let mut b = IoBoard::default();
    b.write(MODE, 0o171, 0);
    b.write(MODE, 0o65, 0);
    assert_eq!(
        read_mode(&mut b, 0),
        0o171 << 8 | 0o65,
        "MR1 then MR2, as the driver combines them"
    );
    // Three reads: 1, 2, 1 again; then a command read, and 1.
    b.read(COMMAND, 0);
    assert_eq!(low(b.read(MODE, 0)), 0o171);
    assert_eq!(low(b.read(MODE, 0)), 0o65);
    assert_eq!(low(b.read(MODE, 0)), 0o171, "recycles");
    b.read(COMMAND, 0);
    assert_eq!(low(b.read(MODE, 0)), 0o171, "back to the first");
    // A status read in between leaves the pointer alone.
    b.read(STATUS, 0);
    assert_eq!(low(b.read(MODE, 0)), 0o65);
    // A write to the second, with the pointer on it, changes only it.
    b.read(COMMAND, 0);
    b.read(MODE, 0);
    b.write(MODE, 0o76, 0);
    assert_eq!(read_mode(&mut b, 0), 0o171 << 8 | 0o76);
    // A Unibus INIT resets the pointer with the rest.
    b.write(MODE, 0o11, 0);
    b.unibus_init();
    assert_eq!(read_mode(&mut b, 0), 0, "RESET clears both registers");
}

/// **The driver's own initialisation lands the registers where it
/// expects.** `(SERIAL-STREAM-MIXIN :INIT)` in `serial.lisp`: reset the
/// errors, enable both halves, `(SERIAL-WRITE-MODE 60)` for the internal
/// clocks, then asynchronous, one stop bit, even parity, seven data bits,
/// 300 baud, each through `SERIAL-READ-MODE` and back. `SERIAL-STATUS`
/// then decodes the mode as "1 stop bits, even parity, 7 data bits,
/// asynchronous" and "internal transmit clock, internal receive clock,
/// 300 baud", which is what the fields below say.
#[test]
fn mits_driver_initialises_the_port_the_way_it_reads_it_back() {
    let mut b = plugged();
    let uart_command = (command::TX_ENABLE | command::RX_ENABLE) as u16;
    b.write(COMMAND, command::RESET_ERROR as u16, 0);
    b.write(COMMAND, uart_command, 0);
    write_mode(&mut b, 0, 0o60, 0);
    // `:SYNCHRONOUS-MODE NIL`: `(DPB 1 1002 MODE)`.
    let put = |b: &mut IoBoard, value: u16, pos: u32, size: u32| {
        let mode = read_mode(b, 0);
        let mask = ((1u16 << size) - 1) << pos;
        let mode = (mode & !mask) | (value << pos) & mask;
        write_mode(b, (mode >> 8) as u8, mode as u8, 0);
    };
    put(&mut b, 1, 8, 2);
    // `:REQUEST-TO-SEND T` and `:DATA-TERMINAL-READY T`.
    b.write(COMMAND, uart_command | command::RTS as u16, 0);
    b.write(COMMAND, uart_command | command::DTR as u16, 0);
    // `:NUMBER-OF-STOP-BITS 1`, `:PARITY :EVEN`, `:NUMBER-OF-DATA-BITS 7`,
    // `:BAUD 300.`.
    put(&mut b, 1, 14, 2);
    put(&mut b, 3, 12, 2);
    put(&mut b, 7 - 5, 10, 2);
    put(&mut b, 5, 0, 4);
    let mode = read_mode(&mut b, 0);
    assert_eq!(mode >> 14 & 3, 1, "1 stop bit");
    assert_eq!(mode >> 12 & 3, 3, "even parity");
    assert_eq!(mode >> 10 & 3, 2, "7 data bits");
    assert_eq!(mode >> 8 & 3, 1, "asynchronous");
    assert_eq!(mode >> 5 & 1, 1, "internal transmit clock");
    assert_eq!(mode >> 4 & 1, 1, "internal receive clock");
    assert_eq!(mode & 0o17, 5, "300 baud");
    assert_eq!(b.serial.framing(), Framing { bits: 7, parity: Some(true), stop_halves: 2 });
    let command = low(b.read(COMMAND, 0));
    assert_eq!(command & 3, 3, "\"receiver-on transmitter-on\"");
    assert_eq!(command & command::DTR, command::DTR, "\"data-terminal-ready\"");
    // The frame the driver has set up: ten bits at 300 baud.
    assert_eq!(b.serial.framing().frame_ns(5), 10 * 16 * 1056 * 1_000_000_000 / BRCLK_HZ);
    // The status: data set ready and carrier detect from the far end,
    // the transmitter ready, and a data set change from the plug.
    let s = low(b.read(STATUS, 0));
    assert_eq!(s & (status::DSR | status::DCD), status::DSR | status::DCD, "{s:o}");
    assert_eq!(s & status::TX_READY, status::TX_READY);
}

/// **The baud-rate table is the sheet's Table 1.** Each divisor from the
/// 5.0688 MHz crystal gives a 16X clock within the sheet's stated error of
/// sixteen times the rate --- zero but for 134.5, 2000 and 19200 baud,
/// "which have errors of +0.016%, +0.235%, and +3.125% respectively".
#[test]
fn the_divisors_give_the_sheets_rates() {
    for (k, (&divisor, &tenths)) in DIVISORS.iter().zip(&BAUD_TENTHS).enumerate() {
        let actual = BRCLK_HZ as f64 / divisor as f64 / 16.0;
        let nominal = tenths as f64 / 10.0;
        let error = (actual - nominal) / nominal * 100.0;
        // 2000 baud: Table 1 prints 0.253 and the text under Table 6 says
        // +0.235%; 5,068,800 / 158 / 16 is 2005.06, which is the table's.
        let allowed = match tenths {
            1345 => 0.017,
            20000 => 0.254,
            192000 => 3.126,
            _ => 0.0005,
        };
        assert!(error.abs() <= allowed, "rate {k}: {nominal} baud comes out {actual}, {error:.3}%");
        assert_eq!(serial::bit_ns(k as u8), 16 * divisor as u64 * 1_000_000_000 / BRCLK_HZ);
    }
    assert_eq!(serial::bit_ns(14), 104_166, "9600 baud: 104.17 us a bit");
}

/// **Transmitting**: a character written goes to the shift register at
/// once and is on the cable one frame later; a second waits in the
/// holding register --- "one full character time of buffering is
/// provided" --- with `TxRDY` down until the first is out; and with both
/// out `TxEMT` comes up and stays until the next is loaded.
#[test]
fn a_character_is_on_the_cable_one_frame_after_it_is_written() {
    let mut b = plugged();
    set_up(&mut b, 14, 0);
    let frame = b.serial.framing().frame_ns(14);
    assert_eq!(frame, 10 * 16 * 33 * 1_000_000_000 / BRCLK_HZ, "ten bits at 9600 baud, 1.04 ms");
    // The generator's 16X clock, counting the crystal's edges from the
    // enable at 0.
    let tick = 33 * 1_000_000_000 / BRCLK_HZ;
    let t0: u64 = 1_000_000;
    let start = b.serial.clock_at_or_after(t0);
    assert!(start > t0 && start - t0 <= tick, "{start}");
    assert_eq!(low(b.read(STATUS, t0)) & status::TX_READY, status::TX_READY);
    b.write(DATA, b'A' as u16, t0);
    assert_eq!(
        low(b.read(STATUS, t0)) & status::TX_READY,
        0,
        "loaded, and waiting for the 16X clock"
    );
    assert_eq!(
        low(b.read(STATUS, start)) & (status::TX_READY | status::TX_EMPTY_OR_DSCHG),
        status::TX_READY,
        "in the shift register at the clock, the holding register free again"
    );
    b.write(DATA, b'B' as u16, start);
    assert_eq!(low(b.read(STATUS, start)) & status::TX_READY, 0, "the holding register is full");
    b.advance(start + frame - 1);
    assert!(b.serial.cable.take().is_none(), "not out yet");
    assert_eq!(low(b.read(STATUS, start + frame - 1)) & status::TX_READY, 0);
    b.advance(start + frame);
    assert_eq!(
        b.serial.cable.take(),
        Some((start + frame, b'A')),
        "the first, at the end of its frame"
    );
    assert_eq!(
        low(b.read(STATUS, start + frame)) & status::TX_READY,
        status::TX_READY,
        "the second moved down"
    );
    b.advance(start + 3 * frame);
    assert_eq!(
        b.serial.cable.take(),
        Some((start + 2 * frame, b'B')),
        "the second, one frame after"
    );
    let t0 = start;
    let s = low(b.read(STATUS, t0 + 3 * frame));
    assert_eq!(s & status::TX_EMPTY_OR_DSCHG, status::TX_EMPTY_OR_DSCHG, "empty: {s:o}");
    assert_eq!(
        s & status::TX_EMPTY_OR_DSCHG,
        status::TX_EMPTY_OR_DSCHG,
        "and a status read does not clear TxEMT"
    );
    b.write(DATA, b'C' as u16, t0 + 3 * frame);
    assert_eq!(
        low(b.read(STATUS, t0 + 3 * frame)) & status::TX_EMPTY_OR_DSCHG,
        0,
        "loading clears it"
    );
    // The interrupt request follows `TxRDY` with the enable up, the
    // board's ECO 10: once the character has moved down, a clock on.
    b.write(ioboard::CSR, csr::SER_INT_ENABLE, t0 + 3 * frame);
    assert_eq!(b.interrupt_request(t0 + 3 * frame), None, "the holding register still full");
    assert_eq!(b.interrupt_request(t0 + 3 * frame + tick), Some(ioboard::SERIAL_VECTOR));
}

/// **Nothing plugged in, nothing sent.** With J9 empty the MC1489 holds
/// `-CTS` off and "The 2651 is conditioned to transmit data when the -CTS
/// input is low": the character waits in the holding register, `TxRDY`
/// down, until something is plugged in, and then goes.
#[test]
fn with_nothing_on_the_cable_the_transmitter_waits_for_cts() {
    let mut b = IoBoard::default();
    set_up(&mut b, 14, 0);
    let frame = b.serial.framing().frame_ns(14);
    assert_eq!(low(b.read(STATUS, 0)) & (status::DSR | status::DCD), 0, "no data set");
    b.write(DATA, b'A' as u16, 0);
    b.advance(10 * frame);
    assert!(b.serial.cable.take().is_none());
    assert_eq!(
        low(b.read(STATUS, 10 * frame)) & status::TX_READY,
        0,
        "still in the holding register"
    );
    b.serial.cable.plug(10 * frame + 1);
    b.advance(11 * frame);
    assert_eq!(low(b.read(STATUS, 11 * frame)) & status::TX_READY, status::TX_READY);
    b.advance(12 * frame);
    let (at, c) = b.serial.cable.take().expect("out once -CTS came down");
    assert_eq!(c, b'A');
    let tick = serial::bit_ns(14) / 16;
    assert!(
        at > 11 * frame && at <= 11 * frame + tick,
        "{at}: a frame after the plug, to the 16X clock"
    );
}

/// **Receiving**: a character the far end sends is in the holding register
/// one frame after the receiver could take it, `RxRDY` up, and the read
/// takes it and puts `RxRDY` down; a second arriving before the first is
/// read is an overrun, `SR4`, which the reset-error command clears; and
/// the interrupt request follows `RxRDY` with the enable up.
#[test]
fn a_character_from_the_far_end_reads_back_one_frame_later() {
    let mut b = plugged();
    set_up(&mut b, 15, 0);
    // The receiver alone: an idle transmitter's `TxRDY` would ask for the
    // interrupt too, by ECO 10.
    b.write(COMMAND, (command::RX_ENABLE | command::DTR | command::RTS) as u16, 0);
    let frame = b.serial.framing().frame_ns(15);
    let bit = serial::bit_ns(15);
    // In the holding register at the middle of the stop bit, the ninth
    // and a half.
    let ready_at = 9 * bit + bit / 2;
    let t0 = 5_000;
    b.serial.cable.send(b'x', t0);
    assert_eq!(low(b.read(STATUS, t0)) & status::RX_READY, 0, "not yet");
    assert_eq!(b.interrupt_request(t0 + frame), None, "enable down");
    b.write(ioboard::CSR, csr::SER_INT_ENABLE, t0);
    assert_eq!(b.interrupt_request(t0 + ready_at - 1), None, "nothing ready yet");
    assert_eq!(
        b.interrupt_request(t0 + ready_at),
        Some(ioboard::SERIAL_VECTOR),
        "ready, without a read"
    );
    assert_eq!(low(b.read(STATUS, t0 + ready_at)) & status::RX_READY, status::RX_READY);
    assert_eq!(low(b.read(DATA, t0 + ready_at)), b'x');
    assert_eq!(low(b.read(STATUS, t0 + frame)) & status::RX_READY, 0, "the read took it");
    assert_eq!(b.interrupt_request(t0 + frame), None);
    // Two in a row, unread: overrun, and the later one in the register.
    b.serial.cable.send(b'y', t0 + frame);
    b.serial.cable.send(b'z', t0 + frame);
    b.advance(t0 + 4 * frame);
    let s = low(b.read(STATUS, t0 + 4 * frame));
    assert_eq!(
        s & (status::RX_READY | status::OVERRUN),
        status::RX_READY | status::OVERRUN,
        "{s:o}"
    );
    assert_eq!(low(b.read(DATA, t0 + 4 * frame)), b'z');
    assert_eq!(low(b.read(STATUS, t0 + 4 * frame)) & status::OVERRUN, status::OVERRUN, "stays");
    let cr = low(b.read(COMMAND, t0 + 4 * frame));
    b.write(COMMAND, (cr | command::RESET_ERROR) as u16, t0 + 4 * frame);
    assert_eq!(low(b.read(STATUS, t0 + 4 * frame)) & status::OVERRUN, 0, "CR4 clears it");
    assert_eq!(low(b.read(COMMAND, t0 + 4 * frame)) & command::RESET_ERROR, 0, "and resets itself");
}

/// **Seven data bits is seven**: "If the character length is less than 8
/// bits, the high order unused bits in the Holding Register are set to
/// zero", both ways.
#[test]
fn a_short_character_is_masked_both_ways() {
    let mut b = plugged();
    write_mode(
        &mut b,
        0o1 << mode1::STOP_SHIFT | 2 << mode1::LENGTH_SHIFT | 2,
        mode2::RX_INTERNAL | mode2::TX_INTERNAL | 15,
        0,
    );
    b.write(COMMAND, (command::TX_ENABLE | command::RX_ENABLE) as u16, 0);
    let frame = b.serial.framing().frame_ns(15);
    assert_eq!(frame, 9 * serial::bit_ns(15), "nine bits");
    let first = b.serial.clock_at_or_after(0);
    b.write(DATA, 0o377, 0);
    b.serial.cable.send(0o377, 0);
    b.advance(2 * frame);
    assert_eq!(b.serial.cable.take(), Some((first + frame, 0o177)), "out from the first 16X clock");
    assert_eq!(low(b.read(DATA, 2 * frame)), 0o177);
}

/// **Local loop back**: `TxD` to `RxD` inside the chip, `-DTR` for `-DCD`
/// and `-RTS` for `-CTS`, so a character written comes back on the
/// receiver with nothing on the cable at all, and the pins hold still.
#[test]
fn local_loop_back_returns_the_character_without_a_cable() {
    let mut b = IoBoard::default();
    set_up(&mut b, 15, 0);
    b.write(
        COMMAND,
        (command::LOCAL_LOOP_BACK | command::TX_ENABLE | command::DTR | command::RTS) as u16,
        0,
    );
    let frame = b.serial.framing().frame_ns(15);
    let tick = serial::bit_ns(15) / 16;
    assert!(!b.serial.dtr() && !b.serial.rts(), "\"the DTR, RTS and TxD outputs are held high\"");
    b.write(DATA, 0o252, 0);
    b.advance(frame + tick);
    assert!(b.serial.cable.take().is_none(), "nothing on the cable");
    assert_eq!(low(b.read(STATUS, frame + tick)) & status::RX_READY, status::RX_READY);
    assert_eq!(low(b.read(DATA, frame + tick)), 0o252);
}

/// **A Unibus INIT resets the chip and drops the enable.** `RESET` is the
/// 2651's own pin 21, and `-RESET` clears the 74LS74 at IOBSER 0D21 with
/// the 74LS175's four.
#[test]
fn a_unibus_init_resets_the_port() {
    let mut b = plugged();
    set_up(&mut b, 14, 0);
    b.write(ioboard::CSR, csr::SER_INT_ENABLE, 0);
    assert_eq!(b.csr() & csr::SER_INT_ENABLE, csr::SER_INT_ENABLE);
    b.unibus_init();
    assert_eq!(b.csr() & csr::SER_INT_ENABLE, 0);
    assert_eq!(low(b.read(COMMAND, 0)), 0);
    assert_eq!(read_mode(&mut b, 0), 0);
    assert!(b.serial.cable.plugged(), "the cable is still there");
}

/// **The vector, and who is named first.** IOBINT names the Chaosnet
/// before the serial port before the keyboard when more than one is up.
#[test]
fn the_board_names_the_chaosnet_before_the_serial_port_before_the_keyboard() {
    let mut b = plugged();
    set_up(&mut b, 15, 0);
    b.write(ioboard::CSR, csr::SER_INT_ENABLE | csr::KBD_INT_ENABLE, 0);
    assert_eq!(b.interrupt_request(0), Some(ioboard::SERIAL_VECTOR), "TxRDY, with the enable");
    b.press(0o123);
    assert_eq!(b.interrupt_request(0), Some(ioboard::SERIAL_VECTOR), "and not the keyboard's");
    b.write(ioboard::CSR, csr::KBD_INT_ENABLE, 0);
    assert_eq!(b.interrupt_request(0), Some(ioboard::KBD_VECTOR));
    assert_eq!(ioboard::SERIAL_VECTOR, 0o264, "serial.lisp's GET-UNIBUS-CHANNEL 264");
    assert_eq!(muir::chaos::board::VECTOR, 0o270);
}
