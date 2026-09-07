// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The serial port on the netlist I/O board: the 2651 of `src/part.rs`
//! at IOBSER 0A12, programmed over the Unibus and driven a bit at a time
//! over the EIA wires by the far end of `src/serial.rs`. The behavioural
//! port of `src/ioboard.rs` is run alongside and has to say the same:
//! every register read is compared, and the frame times are held to a
//! bit of each other.
//!
//! A frame at 19,200 baud is half a millisecond of board time, which is
//! what keeps these tests short: the crystal at 0A15 keeps the board awake
//! while the port is on.

use muir::busint::{UNIBUS_ADDRESS_NS, UNIBUS_STROBE_NS};
use muir::ioboard::{self, IoBoard, csr};
use muir::part::Level;
use muir::serial::{
    self, COMMAND, DATA, Framing, MODE, OnCable, STATUS, command, mode1, mode2, status,
};
use muir::unibus::UnibusMaster;

mod support;
use support::{cadrio, quiet};

/// 19,200 baud, the fastest the generator goes.
const RATE: u8 = 15;

/// Eight data bits, no parity, one stop bit.
fn eight_n_one() -> u8 {
    0o1 << mode1::STOP_SHIFT | 3 << mode1::LENGTH_SHIFT | 2
}

/// Runs the board to `until`, letting the far end see every step, and
/// stops early when `done` says so.
fn run(
    b: &mut UnibusMaster,
    far: &mut OnCable,
    until: u64,
    done: impl Fn(&UnibusMaster, &OnCable) -> bool,
) {
    while b.now < until && !done(b, far) {
        let tap = [b.chip.next_tap(), far.next_change(b.now)]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(until)
            .clamp(b.now + 1, until);
        b.run(tap);
        far.apply(&mut b.chip, b.now);
    }
}

/// A read on the board and on the model at the same instant --- the
/// model's at the moment the board's word was taken, `-SSYN*` --- which
/// have to agree; the low byte.
fn read_both(b: &mut UnibusMaster, m: &mut IoBoard, uaddr: u32, what: &str) -> u8 {
    let t = b.now;
    let (took, word) = b.cycle(uaddr, None);
    let want = m.read(uaddr, t + UNIBUS_ADDRESS_NS + took);
    assert_eq!(word, want, "{what}: the board {word:#o}, the model {want:#o}");
    assert_eq!(word & csr::FLOATING, csr::FLOATING, "{what}: the upper byte floats");
    word as u8
}

/// A write on both: the model's at the rise of `-CE`, where the chip
/// takes it, which is `-MSYN*` lifting a strobe after `-SSYN*`.
fn write_both(b: &mut UnibusMaster, m: &mut IoBoard, uaddr: u32, v: u16) {
    let t = b.now;
    let (took, _) = b.cycle(uaddr, Some(v));
    m.write(uaddr, v, t + UNIBUS_ADDRESS_NS + took + UNIBUS_STROBE_NS);
}

/// The port set up at [`RATE`], eight-N-one, both halves on, `-DTR` and
/// `-RTS` asserted, on the board and on the model.
fn set_up(b: &mut UnibusMaster, m: &mut IoBoard) {
    read_both(b, m, COMMAND, "command, to reset the mode pointer");
    write_both(b, m, MODE, eight_n_one() as u16);
    write_both(b, m, MODE, (mode2::RX_INTERNAL | mode2::TX_INTERNAL | RATE) as u16);
    let cr = command::TX_ENABLE | command::RX_ENABLE | command::DTR | command::RTS;
    write_both(b, m, COMMAND, cr as u16);
}

/// **The registers read back on the board as on the model**, from reset
/// and through what `serial.lisp` does to them: the existence check, the
/// mode pointer, the driver's own initialisation, the status with and
/// without a far end, and the interrupt enable, which is bit 7 of the
/// status register at `764112` on this board and reads back.
#[test]
fn the_registers_read_back_on_the_board_as_on_the_model() {
    let n = cadrio();
    let mut b = UnibusMaster::new(&n, 10_000, &quiet());
    let mut m = IoBoard::default();
    for (uaddr, what) in [
        (STATUS, "status"),
        (MODE, "mode 1"),
        (MODE, "mode 2"),
        (COMMAND, "command"),
        (DATA, "data"),
    ] {
        assert_eq!(read_both(&mut b, &mut m, uaddr, what), 0, "{what} from reset");
    }
    // `SERIAL-CHECK-EXISTENCE`.
    write_both(&mut b, &mut m, COMMAND, 0);
    let zeros = read_both(&mut b, &mut m, COMMAND, "command after 0");
    write_both(&mut b, &mut m, COMMAND, 0o100);
    let ones = read_both(&mut b, &mut m, COMMAND, "command after 100");
    assert_eq!((zeros, ones), (0, 0o100), "the board has a PCI, and is wired for it");
    // The mode pointer, and the aliases above `764170`.
    write_both(&mut b, &mut m, COMMAND, 0);
    read_both(&mut b, &mut m, COMMAND, "command");
    write_both(&mut b, &mut m, MODE, 0o171);
    write_both(&mut b, &mut m, MODE | 0o10, 0o65);
    read_both(&mut b, &mut m, COMMAND | 0o10, "command, at 764176");
    assert_eq!(read_both(&mut b, &mut m, MODE, "mode 1 back"), 0o171);
    assert_eq!(read_both(&mut b, &mut m, MODE | 0o10, "mode 2 back, at 764174"), 0o65);
    assert_eq!(read_both(&mut b, &mut m, MODE, "mode 1 again"), 0o171, "recycled");
    read_both(&mut b, &mut m, STATUS, "status");
    assert_eq!(read_both(&mut b, &mut m, MODE, "mode 2 after a status read"), 0o65);
    // The status with nothing on J9: no data set, no carrier.
    set_up(&mut b, &mut m);
    let s = read_both(&mut b, &mut m, STATUS, "status, unplugged");
    assert_eq!(s & (status::DSR | status::DCD), 0, "{s:o}");
    assert_eq!(s & status::TX_READY, status::TX_READY, "enabled and empty: {s:o}");
    // The far end comes up: both, and a data set change, which the read
    // clears.
    let mut far = OnCable::of(&n, RATE, Framing::of(eight_n_one())).expect("the board has J9");
    far.plug();
    far.apply(&mut b.chip, b.now);
    m.serial.cable.plug(b.now);
    let s = read_both(&mut b, &mut m, STATUS, "status, plugged");
    assert_eq!(
        s & (status::DSR | status::DCD | status::TX_EMPTY_OR_DSCHG),
        status::DSR | status::DCD | status::TX_EMPTY_OR_DSCHG,
        "{s:o}"
    );
    let s = read_both(&mut b, &mut m, STATUS, "status read again");
    assert_eq!(s & status::TX_EMPTY_OR_DSCHG, 0, "the change was read: {s:o}");
    // The interrupt enable: the 74LS74 at 0D21 takes bit 7.
    write_both(&mut b, &mut m, ioboard::CSR, csr::SER_INT_ENABLE | csr::KBD_INT_ENABLE);
    let (_, word) = b.cycle(ioboard::CSR, None);
    assert_eq!(word, m.read(ioboard::CSR, b.now), "the status register, board against model");
    assert_eq!(word & csr::WRITABLE, csr::SER_INT_ENABLE | csr::KBD_INT_ENABLE, "{word:o}");
    assert_eq!(b.level("'SER INT ENABLE'"), Level::High);
    // And a Unibus INIT clears it, and the chip.
    let init = b.net("-INIT*");
    b.chip.drive(init, Level::Low);
    b.chip.transition(b.now);
    b.run(b.now + 1_000);
    b.chip.pull_up(init);
    b.chip.transition(b.now);
    b.run(b.now + 5_000);
    m.unibus_init();
    assert_eq!(b.level("'SER INT ENABLE'"), Level::Low);
    assert_eq!(read_both(&mut b, &mut m, COMMAND, "command after INIT"), 0);
}

/// **A character written goes out on the EIA wire and the far end reads
/// it**, one frame after the write, and the model's frame is the board's
/// to within a bit. `TxRDY` is up again as soon as the character is in
/// the shift register, and `TxEMT` once it is out with nothing behind it.
#[test]
fn a_character_written_reaches_the_far_end_one_frame_later() {
    let n = cadrio();
    let mut b = UnibusMaster::new(&n, 10_000, &quiet());
    let mut m = IoBoard::default();
    let mut far = OnCable::of(&n, RATE, Framing::of(eight_n_one())).unwrap();
    far.plug();
    far.apply(&mut b.chip, b.now);
    m.serial.cable.plug(b.now);
    set_up(&mut b, &mut m);
    let bit = serial::bit_ns(RATE);
    let frame = Framing::of(eight_n_one()).frame_ns(RATE);
    assert_eq!(frame, 10 * bit);
    // The plug's data set change, read and cleared.
    read_both(&mut b, &mut m, STATUS, "status after the plug");

    write_both(&mut b, &mut m, DATA, b'A' as u16);
    let t0 = b.now;
    // Whether the character has moved down by the read's instant depends
    // on where the 16X clock falls; the board and the model must agree
    // on it either way, and two clocks on it has.
    read_both(&mut b, &mut m, STATUS, "status just after the write");
    let until = t0 + 2 * bit / 16;
    run(&mut b, &mut far, until, |_, _| false);
    let s = read_both(&mut b, &mut m, STATUS, "status two 16X clocks later");
    assert_eq!(
        s & (status::TX_READY | status::TX_EMPTY_OR_DSCHG),
        status::TX_READY,
        "in the shift register: {s:o}"
    );
    run(&mut b, &mut far, t0 + 3 * frame, |_, f| f.received.len() == 1);
    assert_eq!(far.received, [b'A'], "the far end has the character");
    let took = b.now - t0;
    eprintln!(
        "the far end had the character {took} ns after the write; a frame is {frame}, a bit {bit}"
    );
    // The far end has it at the middle of the stop bit, half a bit before
    // the model's frame ends; the write's own cycle and the start of the
    // shift register's clock add less than a bit.
    assert!(took + bit / 2 >= frame && took < frame + bit, "{took} against {frame}");
    assert_eq!(far.framing_errors, 0);
    let until = b.now + bit;
    run(&mut b, &mut far, until, |_, _| false);
    m.advance(b.now);
    assert_eq!(m.serial.cable.take().map(|(_, c)| c), Some(b'A'), "and so does the model's");
    let s = read_both(&mut b, &mut m, STATUS, "status once it is out");
    assert_eq!(
        s & (status::TX_READY | status::TX_EMPTY_OR_DSCHG),
        status::TX_READY | status::TX_EMPTY_OR_DSCHG,
        "{s:o}"
    );

    // Two back to back: the second waits in the holding register.
    write_both(&mut b, &mut m, DATA, 0o252);
    write_both(&mut b, &mut m, DATA, 0o125);
    let t1 = b.now;
    let s = read_both(&mut b, &mut m, STATUS, "status with both loaded");
    assert_eq!(s & status::TX_READY, 0, "the holding register is full: {s:o}");
    run(&mut b, &mut far, t1 + 4 * frame, |_, f| f.received.len() == 3);
    assert_eq!(far.received, [b'A', 0o252, 0o125]);
    assert_eq!(far.framing_errors, 0);
    let took = b.now - t1;
    assert!(took < 2 * frame + bit, "{took}: the second followed the first without a gap");
}

/// **A character from the far end reads back, and interrupts on vector
/// 264.** `RxRDY` pulls `-SER RRDY`; with `SER INT ENABLE` up the board
/// pulls `-BR*`, takes the grant with `-SACK*`, and puts `264` on the
/// data lines in its interrupt cycle --- the board's own word on the
/// number `serial.lisp` sets its channels up with. The read of the data
/// register takes the character and drops the request.
#[test]
fn a_character_from_the_far_end_reads_back_and_names_its_vector() {
    let n = cadrio();
    let mut b = UnibusMaster::new(&n, 10_000, &quiet());
    let mut m = IoBoard::default();
    let mut far = OnCable::of(&n, RATE, Framing::of(eight_n_one())).unwrap();
    far.plug();
    far.apply(&mut b.chip, b.now);
    m.serial.cable.plug(b.now);
    set_up(&mut b, &mut m);
    // The receiver alone, so that an idle transmitter's `TxRDY` does not
    // ask for the interrupt first.
    write_both(&mut b, &mut m, COMMAND, (command::RX_ENABLE | command::DTR | command::RTS) as u16);
    write_both(&mut b, &mut m, ioboard::CSR, csr::SER_INT_ENABLE);
    let rrdy = b.net("'-SER RRDY'");
    let br = b.net("-BR*");
    let sack = b.net("-SACK*");
    let intr = b.net("-INTR*");
    let bg_in = b.net("BG.IN*");
    let data: Vec<_> = (0..16).map(|k| b.net(&format!("-D{k}*"))).collect();
    // The pull-up pack at CLK60H 0C20 has no model, so the net floats
    // with both open-drain outputs off; the 74LS02 reads that as a high.
    assert_ne!(b.chip.net(rrdy), Level::Low, "nothing received");
    assert_eq!(b.chip.net(br), Level::High, "no request");

    let frame = Framing::of(eight_n_one()).frame_ns(RATE);
    let bit = serial::bit_ns(RATE);
    far.send(b'x');
    let t0 = b.now;
    m.serial.cable.send(b'x', t0);
    run(&mut b, &mut far, t0 + 3 * frame, |b, _| b.chip.net(rrdy) == Level::Low);
    assert_eq!(b.chip.net(rrdy), Level::Low, "-RxRDY within three frames");
    let took = b.now - t0;
    eprintln!("-RxRDY {took} ns after the far end began the start bit; a frame is {frame}");
    assert!(took + bit >= frame && took < frame + bit, "{took} against {frame}");
    assert!(m.serial.rx_ready_at(b.now), "the model has it by then too");
    let until = b.now + 5_000;
    run(&mut b, &mut far, until, |b, _| b.chip.net(br) == Level::Low);
    assert_eq!(b.chip.net(br), Level::Low, "the board pulls -BR* for the port");
    assert_eq!(m.interrupt_request(b.now), Some(ioboard::SERIAL_VECTOR));

    b.chip.drive(bg_in, Level::High);
    b.chip.transition(b.now);
    let until = b.now + 5_000;
    run(&mut b, &mut far, until, |b, _| b.chip.net(sack) == Level::Low);
    assert_eq!(b.chip.net(sack), Level::Low, "the board takes the grant with -SACK*");
    b.chip.drive(bg_in, Level::Low);
    b.chip.transition(b.now);
    let until = b.now + 5_000;
    run(&mut b, &mut far, until, |b, _| b.chip.net(intr) == Level::Low);
    assert_eq!(b.chip.net(intr), Level::Low, "and runs an interrupt cycle, -INTR*");
    let vector: u16 =
        data.iter().enumerate().map(|(k, &d)| ((b.chip.net(d) == Level::Low) as u16) << k).sum();
    eprintln!("vector on the bus: {vector:o}");
    assert_eq!(vector, ioboard::SERIAL_VECTOR, "the serial port's vector, from the board itself");
    // Acknowledge it as the interface would, `-SSYN*`, and let the cycle
    // end.
    let ssyn = b.net("-SSYN*");
    b.chip.drive(ssyn, Level::Low);
    b.chip.transition(b.now);
    let until = b.now + 2_000;
    run(&mut b, &mut far, until, |b, _| b.chip.net(intr) == Level::High);
    b.chip.pull_up(ssyn);
    b.chip.transition(b.now);
    let until = b.now + 2_000;
    run(&mut b, &mut far, until, |_, _| false);

    let s = read_both(&mut b, &mut m, STATUS, "status with a character in");
    assert_eq!(s & status::RX_READY, status::RX_READY, "{s:o}");
    assert_eq!(read_both(&mut b, &mut m, DATA, "the character"), b'x');
    assert_ne!(b.chip.net(rrdy), Level::Low, "the read took it");
    let s = read_both(&mut b, &mut m, STATUS, "status after the read");
    assert_eq!(s & status::RX_READY, 0, "{s:o}");
    // Two more, unread between them: an overrun, and the later one.
    far.send(b'y');
    far.send(b'z');
    m.serial.cable.send(b'y', b.now);
    m.serial.cable.send(b'z', b.now);
    let until = b.now + 4 * frame;
    run(&mut b, &mut far, until, |_, _| false);
    let s = read_both(&mut b, &mut m, STATUS, "status after two");
    assert_eq!(
        s & (status::RX_READY | status::OVERRUN),
        status::RX_READY | status::OVERRUN,
        "{s:o}"
    );
    assert_eq!(read_both(&mut b, &mut m, DATA, "the later"), b'z');
}

/// **With nothing on J9 the transmitter waits**: the MC1489's open input
/// holds `-CTS` off, the character stays in the holding register with
/// `TxRDY` down, and it goes once the far end comes up.
#[test]
fn with_nothing_on_j9_the_transmitter_waits_for_cts() {
    let n = cadrio();
    let mut b = UnibusMaster::new(&n, 10_000, &quiet());
    let mut m = IoBoard::default();
    let mut far = OnCable::of(&n, RATE, Framing::of(eight_n_one())).unwrap();
    assert_eq!(b.level("'TTL CTS IN'"), Level::High, "-CTS off with J9 empty");
    assert_eq!(b.level("'TTL DATA IN'"), Level::High, "and RxD marking");
    set_up(&mut b, &mut m);
    write_both(&mut b, &mut m, DATA, b'A' as u16);
    let frame = Framing::of(eight_n_one()).frame_ns(RATE);
    let until = b.now + 3 * frame;
    run(&mut b, &mut far, until, |_, _| false);
    assert!(far.received.is_empty(), "nothing went out");
    let s = read_both(&mut b, &mut m, STATUS, "status, waiting");
    assert_eq!(s & status::TX_READY, 0, "{s:o}");
    far.plug();
    m.serial.cable.plug(b.now);
    let t0 = b.now;
    run(&mut b, &mut far, t0 + 3 * frame, |_, f| f.received.len() == 1);
    assert_eq!(far.received, [b'A'], "out once -CTS came down");
    // The far end has it at the middle of the stop bit; the model's
    // frame ends half a bit on.
    let until = b.now + serial::bit_ns(RATE);
    run(&mut b, &mut far, until, |_, _| false);
    m.advance(b.now);
    assert_eq!(m.serial.cable.take().map(|(_, c)| c), Some(b'A'));
}

/// **Seven bits with even parity, as the driver defaults to**: the far
/// end and the port agree on the frame, and a character with the top bit
/// set loses it.
#[test]
fn seven_bits_even_parity_both_ways() {
    let n = cadrio();
    let mut b = UnibusMaster::new(&n, 10_000, &quiet());
    let mut m = IoBoard::default();
    let mr1 = 0o1 << mode1::STOP_SHIFT | mode1::EVEN | mode1::PARITY | 2 << mode1::LENGTH_SHIFT | 1;
    let framing = Framing::of(mr1);
    assert_eq!(framing, Framing { bits: 7, parity: Some(true), stop_halves: 2 });
    let mut far = OnCable::of(&n, RATE, framing).unwrap();
    far.plug();
    far.apply(&mut b.chip, b.now);
    m.serial.cable.plug(b.now);
    b.cycle(COMMAND, None);
    m.read(COMMAND, b.now);
    write_both(&mut b, &mut m, MODE, mr1 as u16);
    write_both(&mut b, &mut m, MODE, (mode2::RX_INTERNAL | mode2::TX_INTERNAL | RATE) as u16);
    write_both(
        &mut b,
        &mut m,
        COMMAND,
        (command::TX_ENABLE | command::RX_ENABLE | command::DTR | command::RTS) as u16,
    );
    let frame = framing.frame_ns(RATE);
    write_both(&mut b, &mut m, DATA, 0o345);
    far.send(0o263);
    m.serial.cable.send(0o263, b.now);
    let until = b.now + 3 * frame;
    run(&mut b, &mut far, until, |_, _| false);
    assert_eq!(far.received, [0o145], "the eighth bit is not sent");
    assert_eq!(far.framing_errors, 0);
    let s = read_both(&mut b, &mut m, STATUS, "status");
    assert_eq!(
        s & (status::RX_READY | status::PARITY_ERROR | status::FRAMING_ERROR),
        status::RX_READY,
        "{s:o}"
    );
    assert_eq!(
        read_both(&mut b, &mut m, DATA, "the character"),
        0o63,
        "the eighth bit is not received"
    );
}
