// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The keyboard on the netlist I/O board's cable: a word clocked down the
//! wire by the board's own 125 kHz reads back at `764100` and `764102`,
//! and with the enable set the board asks the bus for an interrupt and,
//! granted, puts its vector on the data lines.

use muir::ioboard::{self, csr};
use muir::part::Level;
use muir::terminal::cable::OnCable;
use muir::terminal::keyboard;
use muir::unibus::UnibusMaster;

mod support;
use support::{cadrio, quiet};

/// Runs the board to `until`, letting the keyboard see every edge of the
/// clock on the way, and stops early when `done` says so.
fn run(b: &mut UnibusMaster, k: &mut OnCable, until: u64, done: impl Fn(&UnibusMaster) -> bool) {
    while b.now < until && !done(b) {
        let tap = b.chip.next_tap().unwrap_or(until).clamp(b.now + 1, until);
        b.run(tap);
        k.apply(&mut b.chip, b.now);
    }
}

/// **A word sent down the cable is the word the board holds.** The start
/// bit sets the 74LS109 busy, twenty-four falling edges of `KB CLK^` later
/// the three 74LS164s hold the word, `KBD READY` rises, and the two halves
/// read back as the word sent, the high half's read leaving `KBD READY` and
/// the low half's taking it --- the 74LS74 at IOBKBD 0B30 clears on
/// `-READ.KBD.LOW` alone, which is what the behavioural board does in
/// `tests/ioboard.rs`. 25 clocks at 8 us is 200 us; the board is given 400.
#[test]
fn a_word_down_the_cable_reads_back_from_the_board() {
    let n = cadrio();
    let mut b = UnibusMaster::new(&n, 500_000, &quiet());
    let mut k = OnCable::of(&n).expect("the I/O board has the keyboard cable");
    k.apply(&mut b.chip, b.now);
    let ready = b.net("'KBD READY'");
    assert_eq!(b.chip.net(ready), Level::Low, "nothing typed yet");

    let word = keyboard::up_down(0o123, false);
    assert!(k.send(word));
    let t0 = b.now;
    run(&mut b, &mut k, t0 + 400_000, |b| b.chip.net(ready) == Level::High);
    eprintln!("KBD READY rose {} us after the word was queued", (b.now - t0) / 1_000);
    assert_eq!(b.chip.net(ready), Level::High, "KBD READY within 400 us");
    // The clock has stopped; the keyboard notices after three periods.
    let until = b.now + 40_000;
    run(&mut b, &mut k, until, |_| false);
    assert_eq!(k.sent, 1, "the keyboard finished the word");
    assert!(!k.busy(), "and is idle");

    let (_, high) = b.cycle(ioboard::KBD_HIGH, None);
    assert_eq!(b.chip.net(ready), Level::High, "the high half's read leaves KBD READY");
    let (_, low) = b.cycle(ioboard::KBD_LOW, None);
    let got = (high as u32 & 0xff) << 16 | low as u32;
    eprintln!("sent {word:o}, read back {got:o}");
    assert_eq!(got, word, "the word the board holds is the word sent");
    assert_eq!(b.chip.net(ready), Level::Low, "reading it cleared ready");
}

/// **With the enable set, the board requests an interrupt and puts vector
/// 260 on the bus when granted.** `KBD.IREQ` pulls `-BR*`; a grant on
/// `BG.IN*` is answered with `-SACK*`; and the board's interrupt cycle
/// asserts `-INTR*` with the vector on the data lines. That number is
/// otherwise known only from the System 46 keyboard driver's channel
/// setup, so this is the board's own word on it.
#[test]
fn the_board_requests_an_interrupt_and_names_its_vector() {
    let n = cadrio();
    let mut b = UnibusMaster::new(&n, 500_000, &quiet());
    let mut k = OnCable::of(&n).unwrap();
    k.apply(&mut b.chip, b.now);
    let ready = b.net("'KBD READY'");
    let br = b.net("-BR*");
    let sack = b.net("-SACK*");
    let intr = b.net("-INTR*");
    let bg_in = b.net("BG.IN*");
    let data: Vec<_> = (0..16).map(|k| b.net(&format!("-D{k}*"))).collect();

    // The enable, as the keyboard driver sets it up.
    let _ = b.cycle(ioboard::CSR, Some(csr::KBD_INT_ENABLE));
    assert_eq!(b.chip.net(br), Level::High, "no request with nothing typed");

    k.send(keyboard::up_down(0o136, false));
    let until = b.now + 400_000;
    run(&mut b, &mut k, until, |b| b.chip.net(ready) == Level::High);
    assert_eq!(b.chip.net(ready), Level::High);
    let until = b.now + 5_000;
    run(&mut b, &mut k, until, |b| b.chip.net(br) == Level::Low);
    assert_eq!(b.chip.net(br), Level::Low, "the board pulls -BR* for the keyboard");

    // Grant it, as the arbiter would.
    b.chip.drive(bg_in, Level::High);
    b.chip.transition(b.now);
    let until = b.now + 5_000;
    run(&mut b, &mut k, until, |b| b.chip.net(sack) == Level::Low);
    assert_eq!(b.chip.net(sack), Level::Low, "the board takes the grant with -SACK*");
    b.chip.drive(bg_in, Level::Low);
    b.chip.transition(b.now);
    let until = b.now + 5_000;
    run(&mut b, &mut k, until, |b| b.chip.net(intr) == Level::Low);
    assert_eq!(b.chip.net(intr), Level::Low, "and runs an interrupt cycle, -INTR*");

    let vector: u16 =
        data.iter().enumerate().map(|(k, &d)| ((b.chip.net(d) == Level::Low) as u16) << k).sum();
    eprintln!("vector on the bus: {vector:o}");
    assert_eq!(vector, ioboard::KBD_VECTOR, "the keyboard's vector, from the board itself");
}
