// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The keyboard: the word on the cable as `ukbd.lisp` lays it out, the key
//! positions as `KBD-MAKE-NEW-TABLE` has them, and the shifting a viewer's
//! keysyms need to become positions.

use muir::ioboard::{self, IoBoard, csr};
use muir::terminal::keyboard::{self, Key, Keyboard, Shift, keysym, named, shifting};

mod support;
use support::release;

/// **The word is `ukbd.lisp`'s.** Bits 23-19 ones, 18-16 the source `001`,
/// 15 clear for an up-down code, 8 the direction, 6-0 the position; the
/// all-keys-up word has 15 set and the shifts below; a boot has 15-10 set
/// and 46 or 62 in 5-0.
#[test]
fn the_word_is_laid_out_as_ukbd_says() {
    let down = keyboard::up_down(0o123, false);
    assert_eq!(down >> 19, 0o37, "bits 23-19 are ones");
    assert_eq!(down >> 16 & 0o7, 0o1, "source ID 001, the new keyboard");
    assert_eq!(down >> 15 & 1, 0, "an up-down code");
    assert_eq!(down >> 8 & 1, 0, "down");
    assert_eq!(down & 0o177, 0o123, "the position");
    assert_eq!(keyboard::up_down(0o123, true) >> 8 & 1, 1, "up");
    assert_eq!(down >> 9 & 0o77, 0, "12-9 and 13, 14 reserved zero");

    let up = keyboard::all_keys_up(1 << Shift::Control as u16 | 1 << Shift::Shift as u16);
    assert_eq!(up >> 15 & 1, 1, "not an up-down code");
    assert_eq!(up & 0o3777, 0o21, "control and shift still down");

    assert_eq!(keyboard::boot(true) & 0o77, 0o46, "cold");
    assert_eq!(keyboard::boot(false) & 0o77, 0o62, "warm");
    assert_eq!(keyboard::boot(true) >> 10 & 0o77, 0o77, "15-10 ones");
    assert_eq!(keyboard::boot(true) >> 6 & 0o17, 0, "9-6 zero");
    assert_eq!(keyboard::boot(true) >> 16 & 1, 1, "bit 16, which the I/O board also checks");
}

/// **The table agrees with `ukbd.lisp` at every position it names.** The
/// table is from the System 46 sources; the firmware is System 100's own,
/// and names positions in passing: "the bit numbers of the keys which are
/// specially known about for shifting purposes", eleven shifts with one or
/// two positions each, and "rubout is 23 and return is 136". Those are read
/// out of `sys/io1/ukbd.lisp` here and compared with the table; without the
/// release the test says it was skipped. The rest of the table's positions
/// have no second route in the repository.
#[test]
fn mits_firmware_names_the_shift_positions_and_the_table_has_them() {
    let Some(ukbd) = release("io1/ukbd.lisp") else { return };
    // The comment block: `;  <shift>\t<position>` or `;  <shift>\t<a> / <b>`,
    // octal, from the line after the sentence that opens it to the blank
    // line that closes it.
    let opening = ukbd.find("known about for shifting purposes:").expect("the comment block");
    let block = &ukbd[opening..];
    let block = &block[block.find('\n').unwrap() + 1..];
    let block = &block[..block.find("\n\n").expect("the blank line that closes it")];
    let shift = |name: &str| match name {
        "mode lock" => Shift::ModeLock,
        "caps lock" => Shift::CapsLock,
        "alt lock" => Shift::AltLock,
        "repeat" => Shift::Repeat,
        "top" => Shift::Top,
        "greek" => Shift::Greek,
        "shift" => Shift::Shift,
        "hyper" => Shift::Hyper,
        "super" => Shift::Super,
        "meta" => Shift::Meta,
        "control" => Shift::Control,
        other => panic!("the firmware names a shift the table has not got: {other}"),
    };
    let mut shifts = 0;
    for line in block.lines() {
        let line = line.strip_prefix(';').expect("a comment line").trim();
        let (name, positions) = line.split_once('\t').expect("a shift and its positions");
        let mut positions: Vec<u8> =
            positions.split('/').map(|p| u8::from_str_radix(p.trim(), 8).unwrap()).collect();
        positions.sort_unstable();
        assert_eq!(shifting(shift(name.trim())), positions, "{name}");
        shifts += 1;
    }
    assert_eq!(shifts, 11, "eleven shifts");

    // "For booting, we know that rubout is 23 and return is 136".
    let booting = ukbd
        .lines()
        .find(|l| l.contains("rubout is") && l.contains("return is"))
        .expect("the booting sentence");
    let after = |words: &str| -> u8 {
        let rest = &booting[booting.find(words).unwrap() + words.len()..];
        let digits: String = rest.trim_start().chars().take_while(|c| c.is_digit(8)).collect();
        u8::from_str_radix(&digits, 8).unwrap()
    };
    assert_eq!(named("Rubout"), Some(after("rubout is")), "rubout");
    assert_eq!(named("Return"), Some(after("return is")), "return");
}

/// **A keysym lands on the key that gives it.** Letters on their key,
/// shifted characters on the shifted plane of theirs, `(` on both the key
/// that has it unshifted and the one that has it over `9`.
#[test]
fn a_keysym_is_found_on_the_keyboard() {
    assert_eq!(keyboard::positions('a' as u32), [(0o123, false)]);
    assert_eq!(keyboard::positions('A' as u32), [(0o123, true)]);
    assert_eq!(keyboard::positions('!' as u32), [(0o121, true)], "shift 1");
    assert_eq!(
        keyboard::positions('(' as u32),
        [(0o71, true), (0o132, false)],
        "9 shifted, or its own key"
    );
    assert_eq!(keyboard::positions(' ' as u32), [(0o134, false)]);
    assert_eq!(keyboard::positions(keysym::RETURN), [(0o136, false)]);
    assert_eq!(keyboard::positions(keysym::BACKSPACE), [(0o23, false)], "rubout");
    assert_eq!(keyboard::positions(keysym::ESCAPE), [(0o143, false)], "alt mode");
    assert_eq!(keyboard::positions(keysym::F1), [(0o40, false)], "terminal");
    assert!(keyboard::positions(0xff50).is_empty(), "the CADR has no Home key");
    assert_eq!(keyboard::modifier(keysym::ALT_L), Some((Shift::Meta, 0)), "Alt is Meta");
    assert!(matches!(keyboard::TABLE[0o21], Key::None), "plus-minus is not ASCII");
}

/// The keys the machine would decode from a stream of words, as
/// `KBD-CONVERT-NEW` in `lmio/kbd.123` does it over planes 0 and 1:
/// shifts tracked from up-down codes, a character taken on a key-down from
/// the plane the shift selects.
fn decode(words: &[u32]) -> String {
    let mut shift = false;
    let mut out = String::new();
    for &w in words {
        assert_eq!(w >> 16, 0o371, "every word is the new keyboard's");
        let up = w & keyboard::UP != 0;
        let position = (w & 0o177) as usize;
        match keyboard::TABLE[position] {
            Key::Shift(Shift::Shift) => shift = !up,
            Key::Char(plain, shifted) if !up => {
                out.push(if shift { shifted } else { plain } as char)
            }
            Key::Named(n) if !up => out.push_str(&format!("<{n}>")),
            _ => {}
        }
    }
    out
}

/// **What a viewer types is what the machine decodes.** Each keysym a
/// viewer sends, with the shifts a viewer sends, comes out of MIT's own
/// decoding as the character typed --- including the ones where the
/// viewer's shift and the keyboard's disagree.
#[test]
fn what_the_viewer_types_is_what_the_machine_reads() {
    let mut k = Keyboard::new();
    let mut words = Vec::new();
    let mut type_key = |k: &mut Keyboard, sym: u32| {
        k.key(sym, true);
        k.key(sym, false);
        while let Some(w) = k.take() {
            words.push(w);
        }
    };
    // a, then shift held by the viewer and A, then shift released.
    type_key(&mut k, 'a' as u32);
    k.key(keysym::SHIFT_L, true);
    type_key(&mut k, 'A' as u32);
    // `(` with the viewer's shift held: the key over 9, no unshifting.
    type_key(&mut k, '(' as u32);
    k.key(keysym::SHIFT_L, false);
    // `!` with no shift held: shift is pressed around it.
    type_key(&mut k, '!' as u32);
    // `(` with no shift held: its own key, plane 0.
    type_key(&mut k, '(' as u32);
    // A shifted character on a key whose plane 0 the viewer wants while
    // holding shift: `9` with shift down has to let shift go.
    k.key(keysym::SHIFT_R, true);
    type_key(&mut k, '9' as u32);
    k.key(keysym::SHIFT_R, false);
    type_key(&mut k, keysym::RETURN);
    while let Some(w) = k.take() {
        words.push(w);
    }
    assert_eq!(decode(&words), "aA(!(9<Return>");
    // And every key that went down came up.
    let downs = words.iter().filter(|&&w| w & keyboard::UP == 0).count();
    let ups = words.iter().filter(|&&w| w & keyboard::UP != 0).count();
    let octal: Vec<String> = words.iter().map(|w| format!("{w:o}")).collect();
    assert_eq!(downs, ups, "every key up: {octal:?}");
}

/// **The word reaches the behavioural I/O board and reads back.** `KBD
/// READY` rises, the low half reads at `764100` and the high at `764102`
/// with the floating byte above it, and the next word waits until the
/// board has been read --- the keyboard's `DONE`.
#[test]
fn a_key_is_delivered_to_the_io_board_and_read_back() {
    let mut k = Keyboard::new();
    let mut b = IoBoard::default();
    k.key('z' as u32, true);
    k.key('z' as u32, false);
    assert_eq!(k.pending(), 2);
    assert!(k.deliver(&mut b), "the first word goes in");
    assert!(b.keyboard_ready());
    assert!(!k.deliver(&mut b), "the second waits: the board has not been read");

    let word = keyboard::up_down(0o124, false);
    let high = b.read(ioboard::KBD_HIGH, 0);
    assert_eq!(high & 0xff, (word >> 16) as u16, "bits 23-16 at 764102");
    assert_eq!(high & csr::FLOATING, csr::FLOATING, "and the floating byte above");
    assert!(b.keyboard_ready(), "the high half leaves ready: the word is not taken yet");
    assert!(!k.deliver(&mut b), "so the second still waits");
    let low = b.read(ioboard::KBD_LOW, 0);
    assert_eq!(low, word as u16, "bits 15-0 at 764100");
    assert!(!b.keyboard_ready(), "reading it cleared ready");
    assert!(k.deliver(&mut b), "and the key-up goes in");
    assert_eq!(b.read(ioboard::KBD_LOW, 0), keyboard::up_down(0o124, true) as u16);
}

/// **The board requests an interrupt only when told to, and the machine
/// takes it as vector 260.** `KBD READY` alone is not a request: the CSR's
/// `KBD INT ENABLE` has to be set, and the bus interface's `ENABLE UB
/// INTS` has to be set for the interface to take it; then `766040` reads
/// `UB INT` with the vector in bits 2-9, and a write of zero to `766042`
/// after the data has been read leaves nothing pending.
#[test]
fn the_keyboard_interrupt_is_taken_with_its_vector() {
    use muir::busint::interrupt_status::{ENABLE_UB_INTS, UB_INT};
    use muir::machine::Machine;
    // A Unibus address as the processor reaches it: the inverse of
    // `busint::unibus_address`, Unibus I/O space at physical 17400000 with
    // the 16-bit word address halved into it.
    let phys = |unibus: u32| 0o17400000 | (unibus >> 1);
    let mut m = Machine::new();
    m.ioboard.press(keyboard::up_down(0o123, false));
    assert!(m.ioboard.interrupt_request(m.ns).is_none(), "ready, but the enable is clear");
    m.ioboard.write(ioboard::CSR, csr::KBD_INT_ENABLE, 0);
    assert_eq!(m.ioboard.interrupt_request(m.ns), Some(ioboard::KBD_VECTOR));
    assert!(!m.interrupt(), "the interface has not been enabled");

    // What the microcode writes at the end of the cold boot.
    m.bus_write(phys(0o766040), 0o6000);
    assert!(m.interrupt(), "taken");
    let status = m.bus_read(phys(0o766040)) as u16;
    assert_ne!(status & UB_INT, 0, "UB INT reads set");
    assert_eq!(status & 0o1774, 0o260, "the vector, in place, in bits 2-9");
    assert_ne!(status & ENABLE_UB_INTS, 0);

    // The interrupt routine reads the data, high half first.
    let _ = m.bus_read(phys(ioboard::KBD_HIGH));
    let _ = m.bus_read(phys(ioboard::KBD_LOW));
    assert!(m.ioboard.interrupt_request(m.ns).is_none(), "the request drops with KBD READY");
    m.bus_write(phys(0o766042), 0);
    assert!(!m.interrupt(), "dismissed");
    assert_eq!(m.bus_read(phys(0o766040)) as u16 & UB_INT, 0);
}
