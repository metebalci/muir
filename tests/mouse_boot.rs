// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The mouse on a booted System 100: counts into the I/O board, and the
//! microcode's `TRACK-MOUSE` moving its position.
//!
//! Needs the vendored pack and symbol table; without them the test says it
//! was skipped. `TRACK-MOUSE` runs from the display's vertical interrupt,
//! which the band's cold boot leaves off and `(SI:SETUP-CPT)` turns on
//! (`tests/vertical.rs`), so that is typed first.

use std::path::PathBuf;

use muir::engine::Engine;
use muir::rtl::Rtl;
use muir::simpletv::FRAME_NS;
use muir::sym::{self, Space};
use muir::terminal::keyboard::Keyboard;
use muir::terminal::mouse::Mouse;

mod support;
use support::{boot_to_the_prompt, machine_with_pack, type_at, vendor};

/// A fixnum out of A memory, signed from its 24-bit pointer field.
fn fixnum(word: u32) -> i32 {
    ((word & 0o77777777) as i32) << 8 >> 8
}

/// **A viewer's pointer moves the machine's mouse.** The boot runs over
/// the Chaosnet server to the listener, running the mouse's
/// initialization on the way --- the speed tables of `MOUSE-SPEED-HACK`
/// and the mouse screen. `(SI:SETUP-CPT)`, which the
/// band's cold boot skips, then turns the vertical interrupt on
/// (`tests/vertical.rs`). From there `TRACK-MOUSE` reads the counters
/// every frame, scales the difference by speed and adds it to `A-MOUSE-X`
/// and `A-MOUSE-Y`: counts to the right and down move them right and
/// down, and left and up back.
#[test]
fn the_pointer_moves_the_machines_mouse() {
    let (Some(pack), Some(symbols), Some(root)) = (
        support::pack_100(),
        support::release_100_file(&["ubin", "ucadr.sym"]),
        vendor(&["run", "file-root"]),
    ) else {
        return;
    };
    let symbols = sym::parse(&std::fs::read_to_string(symbols).unwrap()).unwrap();
    let a = |name: &str| {
        symbols.address(Space::AMem, name).unwrap_or_else(|| panic!("{name}")) as usize
    };
    let (ax, ay, width) = (a("A-MOUSE-X"), a("A-MOUSE-Y"), a("A-MOUSE-SCREEN-WIDTH"));

    let mut e = Rtl::new(machine_with_pack(&pack));
    e.boot();
    let ran = boot_to_the_prompt(&mut e, root);
    eprintln!("at the prompt after {ran}: mouse screen width {}", fixnum(e.machine().amem[width]));

    // The boot finished over the network; then the sync program and the
    // interrupt, which the cold boot leaves off.
    let mut k = Keyboard::new();
    type_at(&mut e, &mut k, "(si:setup-cpt)\n");
    for _ in 0..10_000_000 {
        e.step().expect("halted");
    }
    assert_eq!(
        fixnum(e.machine().amem[width]),
        768,
        "MOUSE-INITIALIZE ran: the mouse screen is the main screen"
    );
    assert_ne!(
        e.machine().simpletv.mode() & muir::simpletv::mode::INTERRUPT_ENABLE,
        0,
        "SETUP-CPT enabled the vertical interrupt"
    );
    let (x0, y0) = (fixnum(e.machine().amem[ax]), fixnum(e.machine().amem[ay]));
    eprintln!("mouse at ({x0}, {y0}) before");
    assert!(x0 > 0 && y0 > 0, "the mouse was warped somewhere on the screen");

    // A viewer's pointer, from the middle, up and to the left, so that the
    // clip at the right edge is not in the way.
    let mut mouse = Mouse::new();
    mouse.pointer(0, 400, 400);
    mouse.pointer(0, 300, 350);
    mouse.deliver(&mut e.machine_mut().ioboard);
    assert!(e.machine().ioboard.mouse_ready());
    let t0 = e.machine().ns;
    while e.machine().ns < t0 + 5 * FRAME_NS {
        e.step().expect("halted while the mouse moved");
    }
    let (x1, y1) = (fixnum(e.machine().amem[ax]), fixnum(e.machine().amem[ay]));
    eprintln!("mouse at ({x1}, {y1}) after 100 counts left and 50 up");
    assert!(!e.machine().ioboard.mouse_ready(), "TRACK-MOUSE read the registers");
    assert!(x1 < x0, "moved left: {x0} -> {x1}");
    assert!(y1 < y0, "moved up: {y0} -> {y1}");

    // And back, right and down.
    mouse.pointer(0, 500, 450);
    mouse.deliver(&mut e.machine_mut().ioboard);
    let t0 = e.machine().ns;
    while e.machine().ns < t0 + 5 * FRAME_NS {
        e.step().expect("halted");
    }
    let (x2, y2) = (fixnum(e.machine().amem[ax]), fixnum(e.machine().amem[ay]));
    eprintln!("mouse at ({x2}, {y2}) after 200 counts right and 100 down");
    assert!(x2 > x1, "moved right: {x1} -> {x2}");
    assert!(y2 > y1, "moved down: {y1} -> {y2}");
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/run/screen-mouse.png");
    std::fs::write(&p, e.machine().simpletv.png()).unwrap();
    eprintln!("screen at {}", p.display());
}
