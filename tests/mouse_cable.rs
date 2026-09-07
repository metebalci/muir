// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The mouse on the netlist I/O board's lines: quadrature stepped down the
//! two pairs counts in the board's 74LS569s and reads back at `764106` and
//! `764104`, the switches land in the Y register's bits 12 to 14, and a
//! change raises `MOUSE READY`, which reading the Y register clears.

use muir::ioboard::{self, csr, mouse};
use muir::part::Level;
use muir::terminal::cable::MouseOnCable;
use muir::unibus::UnibusMaster;

mod support;
use support::{cadrio, quiet};

/// Runs the board to `until`, giving the mouse every tap on the way.
fn run(b: &mut UnibusMaster, m: &mut MouseOnCable, until: u64) {
    while b.now < until {
        let tap = [b.chip.next_tap(), m.next_change(b.now)]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(until)
            .clamp(b.now + 1, until);
        b.run(tap);
        m.apply(&mut b.chip, b.now);
    }
}

/// **Motion on the lines is counted by the board.** Ten steps to the right
/// and five up: X reads ten more and Y five fewer, in twelve bits.
#[test]
fn quadrature_on_the_lines_is_counted_by_the_board() {
    let n = cadrio();
    let mut b = UnibusMaster::new(&n, 500_000, &quiet());
    let mut m = MouseOnCable::of(&n).expect("the I/O board has the mouse lines");
    m.apply(&mut b.chip, b.now);
    let until = b.now + 100_000;
    run(&mut b, &mut m, until);
    let (_, x0) = b.cycle(ioboard::MOUSE_X, None);
    let (_, y0) = b.cycle(ioboard::MOUSE_Y, None);
    eprintln!("at rest: X {x0:o}, Y {y0:o}");

    m.send(10, -5, 0);
    let until = b.now + 20 * 16_000;
    run(&mut b, &mut m, until);
    assert!(!m.busy(), "every step went out");
    let (_, x) = b.cycle(ioboard::MOUSE_X, None);
    let (_, y) = b.cycle(ioboard::MOUSE_Y, None);
    let dx = ((x.wrapping_sub(x0) & mouse::COUNT) as i16) << 4 >> 4;
    let dy = ((y.wrapping_sub(y0) & mouse::COUNT) as i16) << 4 >> 4;
    eprintln!("after ten right and five up: X {x:o}, Y {y:o}: dx {dx}, dy {dy}");
    assert_eq!((dx, dy), (10, -5), "the board counted what the mouse did");
}

/// **A switch pressed is a bit in the Y register, and a status change.**
#[test]
fn the_switches_are_the_y_registers_top_bits() {
    let n = cadrio();
    let mut b = UnibusMaster::new(&n, 500_000, &quiet());
    let mut m = MouseOnCable::of(&n).unwrap();
    m.apply(&mut b.chip, b.now);
    let until = b.now + 100_000;
    run(&mut b, &mut m, until);
    let _ = b.cycle(ioboard::MOUSE_Y, None);
    let ready = b.net("'MOUSE READY'");
    let until = b.now + 50_000;
    run(&mut b, &mut m, until);
    assert_eq!(b.chip.net(ready), Level::Low, "nothing has happened");

    m.send(0, 0, 0o1);
    let until = b.now + 50_000;
    run(&mut b, &mut m, until);
    assert_eq!(b.chip.net(ready), Level::High, "the press is a status change");
    let (_, csr_word) = b.cycle(ioboard::CSR, None);
    assert_ne!(csr_word & csr::MOUSE_READY, 0, "and reads as MOUSE READY");
    let (_, y) = b.cycle(ioboard::MOUSE_Y, None);
    assert_eq!(
        y & (mouse::TAIL | mouse::MIDDLE | mouse::HEAD),
        mouse::TAIL,
        "the left button, the tail switch: {y:o}"
    );
    let until = b.now + 50_000;
    run(&mut b, &mut m, until);
    assert_eq!(b.chip.net(ready), Level::Low, "reading Y cleared it");

    m.send(0, 0, 0o6);
    let until = b.now + 50_000;
    run(&mut b, &mut m, until);
    let (_, y) = b.cycle(ioboard::MOUSE_Y, None);
    assert_eq!(
        y & (mouse::TAIL | mouse::MIDDLE | mouse::HEAD),
        mouse::MIDDLE | mouse::HEAD,
        "middle and right: {y:o}"
    );
}
