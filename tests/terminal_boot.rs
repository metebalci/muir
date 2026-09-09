// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Typing at a booted System 100: the whole keyboard path on `rtl`, from a
//! viewer's keysyms to characters the Lisp Listener echoes.
//!
//! Needs the vendored pack and file root; without them the test says it was
//! skipped.  The boot to the listener, over the network, is some 30 million
//! microcycles, a few seconds at `rtl`'s rate.

use std::path::PathBuf;

use muir::engine::Engine;
use muir::rtl::Rtl;
use muir::terminal::keyboard::Keyboard;

mod support;
use support::{CHAOS_100, boot_to_the_prompt, lit_rows, machine_with_pack, type_at, vendor};

/// Lit pixels on the screen.
fn lit(e: &Rtl) -> usize {
    e.machine().simpletv.lit()
}

/// **A character typed at a viewer is read by the machine.** With System
/// 100 sitting in its listener, `a` goes down the keyboard path --- the
/// word into the I/O board, `KBD READY` up, the board's interrupt request,
/// the bus interface taking it as vector 260, the microcode's channel
/// service reading `764102` and `764100` into the keyboard buffer --- and
/// the register is read, which is the one thing that clears `KBD READY`.
/// Then the listener echoes it, and the screen has more on it than it had.
#[test]
fn a_key_typed_at_the_listener_is_read_and_echoed() {
    let (Some(pack), Some(root)) = (support::pack_100(), vendor(&["run", "file-root"])) else {
        return;
    };
    let mut e = Rtl::new(machine_with_pack(&pack));
    e.boot();
    // The band is System 100's, and it calls its file and time host at
    // its own host table's numbers rather than at muir's defaults, which
    // are on the private subnet 376 and no band's.
    let ran = boot_to_the_prompt(&mut e, CHAOS_100, root);
    let before = lit(&e);
    eprintln!("listener after {ran} microcycles, {before} pixels lit");
    eprintln!(
        "I/O board CSR {:o}, interval timer {} x 16 us, interrupt status {:o}",
        e.machine().ioboard.csr(),
        e.machine().ioboard.interval_timer(),
        e.machine().interrupt_status
    );

    let mut k = Keyboard::new();
    // The listener is told something it will print back: a form and its
    // value, `(+ 1 2)` and `3`.
    type_at(&mut e, &mut k, "(+ 1 2)\n");
    assert!(!e.machine().ioboard.keyboard_ready(), "every word was read by the machine");
    assert_eq!(k.pending(), 0, "every word was delivered");

    // Let the listener echo and evaluate.
    for _ in 0..10_000_000 {
        e.step().expect("halted after typing");
    }
    let after = lit(&e);
    eprintln!("after typing, {after} pixels lit");
    // The form is echoed on the line under the prompt's two, rows 128 to
    // 142, and its value printed on the next; the cursor, which blinks
    // two hundred pixels either way, is below both. The rows follow the
    // herald's height, and System 304's is a line taller.
    let echo = lit_rows(&e, 128..142);
    let value = lit_rows(&e, 142..156);
    eprintln!("the echo line has {echo} pixels lit, the value's line {value}");
    // The keyboard's Unibus channel block at 500 and its buffer at 520-600,
    // where the microcode's channel service puts what it read.
    let main = &e.machine().main;
    let block: Vec<String> = (0o500..0o520).map(|a| format!("{:o}", main[a])).collect();
    eprintln!("channel block 500-517: {}", block.join(" "));
    let buffer: Vec<String> =
        (0o520..0o600).map(|a| main[a]).filter(|&w| w != 0).map(|w| format!("{w:o}")).collect();
    eprintln!("channel buffer 520-577, non-zero words: {}", buffer.join(" "));
    // `%UNIBUS-CHANNEL-BUFFER-IN-PTR` and `-OUT-PTR`, the eighth and ninth
    // words of the block: the microcode stores at the first and Lisp takes
    // from the second, and equal means the keyboard process took every
    // word the microcode put there.
    let (in_ptr, out_ptr) = (main[0o510] & 0o77777777, main[0o511] & 0o77777777);
    eprintln!("channel in-ptr {in_ptr:o}, out-ptr {out_ptr:o}");
    assert_eq!(in_ptr, out_ptr, "Lisp took every word out of the channel buffer");
    assert_eq!(main[0o502] & 0o77777777, 0o260, "the channel is the keyboard's, vector 260");
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/run/screen-typed.png");
    std::fs::write(&p, e.machine().simpletv.png()).unwrap();
    eprintln!("screen at {}", p.display());
    assert!(echo > 60, "the screen should have the echo on it: {echo} pixels on its line");
    assert!(value > 20, "and the value under it: {value} pixels on its line");
}
