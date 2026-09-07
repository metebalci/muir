// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The black-and-white display, checked against MIT's own window system.
//!
//! The constants in `src/simpletv.rs` are not asserted against themselves here:
//! `the_geometry_is_mits_own` reads `sys/window/shwarm.lisp` out of the
//! System 100 release and fails if we have drifted from it. Without the
//! vendored release that one test reports that it was skipped.

use muir::busint::{self, Responder};
use muir::machine::{MAIN_WORDS, Machine, bus_error};
use muir::simpletv::{self, SimpleTv, mode};

mod support;
use support::release;

/// MIT's source, if the release is vendored.
fn shwarm() -> Option<String> {
    release("window/shwarm.lisp")
}

/// `(DEFCONST NAME #oNNN)`, as MIT writes it.
fn defconst_octal(src: &str, name: &str) -> u32 {
    let line = src.lines().find(|l| l.contains(&format!("DEFCONST {name} #o"))).unwrap();
    let digits: String =
        line.split("#o").nth(1).unwrap().chars().take_while(|c| c.is_digit(8)).collect();
    u32::from_str_radix(&digits, 8).unwrap()
}

/// The `(:CADR NNN.)` arm of the `SELECT-PROCESSOR` under `name`.
fn cadr_decimal(src: &str, name: &str) -> usize {
    let lines: Vec<&str> = src.lines().collect();
    let at = lines.iter().position(|l| l.contains(name)).unwrap();
    let arm = lines[at..at + 4].iter().find(|l| l.contains("(:CADR")).unwrap();
    let digits: String = arm
        .split("(:CADR")
        .nth(1)
        .unwrap()
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().unwrap()
}

/// Every number in `src/simpletv.rs` that MIT states, taken from MIT.
#[test]
fn the_geometry_is_mits_own() {
    let Some(src) = shwarm() else { return };

    // The control address is an Xbus I/O offset; physical is that plus the
    // base of I/O space, which is where the frame buffer starts.
    let control = defconst_octal(&src, "MAIN-SCREEN-CONTROL-ADDRESS");
    assert_eq!(control, 0o377760, "MIT's control address");
    assert_eq!(
        simpletv::CONTROL,
        simpletv::BUFFER + control,
        "control register is at the buffer base + offset"
    );

    assert_eq!(
        simpletv::BUFFER_WORDS,
        defconst_octal(&src, "MAIN-SCREEN-BUFFER-LENGTH"),
        "MAIN-SCREEN-BUFFER-LENGTH"
    );
    assert_eq!(simpletv::WIDTH, cadr_decimal(&src, "MAIN-SCREEN-WIDTH"), "MAIN-SCREEN-WIDTH");
    assert_eq!(simpletv::HEIGHT, cadr_decimal(&src, "MAIN-SCREEN-HEIGHT"), "MAIN-SCREEN-HEIGHT");
    assert_eq!(
        simpletv::WORDS_PER_LINE,
        cadr_decimal(&src, "MAIN-SCREEN-LOCATIONS-PER-LINE"),
        "MAIN-SCREEN-LOCATIONS-PER-LINE"
    );
}

/// One bit per pixel, and the line stride says so.
#[test]
fn a_line_is_the_width_in_bits() {
    assert_eq!(simpletv::WORDS_PER_LINE * 32, simpletv::WIDTH, "24 words of 32 bits is 768 pixels");
    let used = simpletv::WORDS_PER_LINE * simpletv::HEIGHT;
    assert!(used <= simpletv::BUFFER_WORDS as usize, "the screen must fit in the buffer");
    assert_eq!(used, 23_112, "what the screen actually occupies");
}

/// MIT's own `BLACK-ON-WHITE` and `WHITE-ON-BLACK`, which read the register,
/// change one bit and write it back. The register has to read back for that
/// to work at all.
#[test]
fn black_on_white_is_one_bit_read_modify_written() {
    let mut tv = SimpleTv::default();
    assert!(!tv.black_on_white(), "comes up showing one bits as white");

    // (%XBUS-WRITE control (LOGIOR 4 (%XBUS-READ control)))
    tv.write_control(0, 4 | tv.read_control(0, 0), 0);
    assert!(tv.black_on_white(), "BLACK-ON-WHITE sets MODE<2>");
    assert_eq!(tv.mode() & mode::BOW, mode::BOW);

    // (%XBUS-WRITE control (LOGAND -5 (%XBUS-READ control)))  ;1's comp of 4
    tv.write_control(0, (!4u32) & tv.read_control(0, 0), 0);
    assert!(!tv.black_on_white(), "WHITE-ON-BLACK clears it again");
}

/// Writing the mode register must not invent sync signals.
#[test]
fn the_read_only_bits_never_stick() {
    let mut tv = SimpleTv::default();
    tv.write_control(0, !0, 0);
    assert_eq!(
        tv.mode(),
        mode::CLOCK | mode::BOW | mode::INTERRUPT_ENABLE,
        "the four data pins of the 2519 land, and the read-only bits do not"
    );
    // Bits 5 to 7 come off the read buffer from nets nothing here drives.
    assert_eq!(tv.read_control(0, 0) & (mode::VSYNC | mode::HSYNC | mode::SYNC_PROM_ENABLE), 0);
}

/// **The vertical flag is a flop of its own, and a mode write loads it.**
/// `-TVMA CLR` presets it at the start of every frame, [`FRAME_NS`] apart;
/// a write of the mode register clocks the written bit 4 into it, which is
/// how `INTRX0` clears it --- read, clear bit 4, write back; and with
/// `MODE INTR ENB` it is the interrupt the microcode takes as the 60-cycle
/// clock. The drawing reads as though the bit were not writable; the 74LS74
/// at NXBCTL 0E14 is what settles it (discrepancy 26).
#[test]
fn the_vertical_flag_sets_each_frame_and_a_mode_write_clears_it() {
    use muir::simpletv::FRAME_NS;
    let mut tv = SimpleTv::default();
    assert!(!tv.vert_flag(0), "cleared by reset");
    assert!(!tv.vert_flag(FRAME_NS - 1), "and not yet set before the first frame starts");
    assert!(tv.vert_flag(FRAME_NS), "set by TVMA CLR at the frame");
    assert_eq!(tv.read_control(0, FRAME_NS) & mode::VERT, mode::VERT, "and read back in bit 4");
    assert!(!tv.interrupt(FRAME_NS), "no interrupt without the enable");

    // The window system's enable, and the microcode's clear: bit 4 written
    // zero with the rest of the register kept.
    tv.write_control(0, mode::INTERRUPT_ENABLE, FRAME_NS + 1);
    assert!(!tv.vert_flag(FRAME_NS + 1), "the write cleared it");
    assert!(!tv.interrupt(FRAME_NS + 1));
    assert!(tv.vert_flag(2 * FRAME_NS), "the next frame sets it again");
    assert!(tv.interrupt(2 * FRAME_NS), "and now it interrupts");
    assert_eq!(tv.read_control(0, 2 * FRAME_NS), mode::INTERRUPT_ENABLE | mode::VERT);
    tv.write_control(0, mode::INTERRUPT_ENABLE, 2 * FRAME_NS + 1);
    assert!(!tv.interrupt(2 * FRAME_NS + 1), "dismissed");

    // Written one, it is one: the flop takes what it is given.
    tv.write_control(0, mode::INTERRUPT_ENABLE | mode::VERT, 2 * FRAME_NS + 2);
    assert!(tv.interrupt(2 * FRAME_NS + 3), "a write of bit 4 set it");
}

#[test]
fn the_display_answers_where_nothing_did_before() {
    let d = |p| busint::decode(p, MAIN_WORDS);
    assert_eq!(d(simpletv::BUFFER), Responder::Device, "the first word of the screen");
    assert_eq!(
        d(simpletv::BUFFER + simpletv::BUFFER_WORDS - 1),
        Responder::Device,
        "the last word"
    );
    assert_eq!(d(simpletv::CONTROL), Responder::Device, "the mode register");
    assert_eq!(
        d(simpletv::CONTROL + simpletv::CONTROL_WORDS - 1),
        Responder::Device,
        "its last word"
    );

    assert_eq!(
        d(simpletv::BUFFER + simpletv::BUFFER_WORDS),
        Responder::NoXbus,
        "just past the screen"
    );
    assert_eq!(d(simpletv::CONTROL - 1), Responder::NoXbus, "just below the mode register");
    // The disk controller is four words further up the same page.
    assert_eq!(d(0o17377774), Responder::Device, "the disk still decodes");
}

/// A word written over the bus comes back, and no bus error is raised.
#[test]
fn the_bus_reaches_the_frame_buffer() {
    let mut m = Machine::new();
    let last = simpletv::BUFFER + simpletv::BUFFER_WORDS - 1;

    m.bus_write(simpletv::BUFFER, 0o12345670123);
    m.bus_write(last, 1);
    assert_eq!(m.bus_read(simpletv::BUFFER), 0o12345670123);
    assert_eq!(m.bus_read(last), 1);
    assert_eq!(m.bus_error, 0, "the display answers, so nothing times out");

    // The mode register is on the same bus and is not the buffer.
    m.bus_write(simpletv::CONTROL, mode::BOW);
    assert_eq!(m.bus_read(simpletv::CONTROL), mode::BOW);
    assert!(m.simpletv.black_on_white());
    assert_eq!(m.bus_error, 0);
}

/// Past the screen there is still nothing, and it still says so.
#[test]
fn past_the_display_the_xbus_still_times_out() {
    let mut m = Machine::new();
    m.bus_read(simpletv::BUFFER + simpletv::BUFFER_WORDS);
    assert_eq!(m.bus_error & bus_error::XBUS_NXM, bus_error::XBUS_NXM);
}

/// Bit order within a word: pixel 0 of a line is bit 0 of its first word.
#[test]
fn the_first_pixel_of_a_line_is_the_low_bit() {
    let mut tv = SimpleTv::default();
    tv.write_buffer(0, 1);
    assert!(tv.pixel(0, 0), "bit 0 is the leftmost pixel");
    assert!(!tv.pixel(1, 0));

    // The first word of line 1 is WORDS_PER_LINE in.
    tv.write_buffer(simpletv::WORDS_PER_LINE as u32, 1);
    assert!(tv.pixel(0, 1), "line 1 starts one stride along");
    assert_eq!(tv.buffer().len(), simpletv::BUFFER_WORDS as usize);
}

/// **A sync pointer past the RAM is a corrupt checkpoint, and is refused**
/// before the next register access indexes the RAM with it. The pointer
/// is the twelve-bit address of the eight 2147s, and a write to register
/// 2 keeps twelve bits, so a saved pointer is never wider.
#[test]
fn a_sync_pointer_past_the_ram_is_refused() {
    use muir::checkpoint::{Reader, Writer};
    let mut tv = SimpleTv::default();
    tv.sync.pointer = simpletv::SYNC_RAM_WORDS as u16;
    let mut w = Writer::new();
    tv.save(&mut w);
    let body = w.finish();
    let err = SimpleTv::default().load(&mut Reader::new(&body)).unwrap_err().to_string();
    assert!(err.contains("pointer"), "{err}");
    // The last word's address loads as itself.
    tv.sync.pointer = simpletv::SYNC_RAM_WORDS as u16 - 1;
    let mut w = Writer::new();
    tv.save(&mut w);
    let body = w.finish();
    let mut back = SimpleTv::default();
    let mut r = Reader::new(&body);
    back.load(&mut r).unwrap();
    r.done().unwrap();
    assert_eq!(back.sync.pointer, simpletv::SYNC_RAM_WORDS as u16 - 1);
}

/// MIT's own order sheet for the board, committed with the drawings.
///
/// `cadrtv/lmtv.order` is the LMTV's programming specification: the
/// addressable registers, the video buffer and the sync program, written by
/// the people who built it. It is a fourth route to the interface, and the
/// one that settles which address the board answers on --- `shwarm.lisp`
/// says what the software writes, `nxbctl.drw` says what the parts do, and
/// this says what the board was specified to be.
fn lmtv_order() -> &'static str {
    include_str!("../mit/cadrtv/lmtv.order")
}

/// **The addresses are MIT's, from MIT's own specification of the board.**
///
/// `lmtv.order` writes the control registers `173777x0` to `173777x7` and
/// the buffer `17x00000-17x77777`, with "for the normal TV, x is 6" and
/// "the normal TV has x equal to 0, so the buffer starts at 17000000". The
/// normal TV is the one `shwarm.lisp` declares `:CONTROLLER :SIMPLE`; the
/// colour board is the other x.
#[test]
fn the_addresses_are_mits_own() {
    let src = lmtv_order();

    assert!(src.contains("For the normal TV, x is 6"), "which x the normal TV is");
    assert_eq!(simpletv::CONTROL, 0o17377760, "173777x0 with x = 6");

    assert!(src.contains("17x00000-17x77777"), "the buffer's range");
    assert!(src.contains("32K x 32 bits of video buffer"), "its size");
    assert!(src.contains("buffer starts at 17000000"), "where the normal TV's buffer is");
    assert_eq!(simpletv::BUFFER, 0o17000000);
    assert_eq!(simpletv::BUFFER_WORDS, 0o100000, "32K words");

    // Eight control words: five that do something and three that "respond
    // but don't do anything".
    assert!(src.contains("173777x5,6,7  These addresses respond but don't do anything"));
    assert_eq!(simpletv::CONTROL_WORDS, 8);
}

/// **The mode bits are MIT's**, named and numbered on the order sheet, and
/// bit for bit what `src/simpletv.rs` has. Bit 4 is not in the register:
/// `VERT FLAG` is a flop of its own, set by `TVMA CLR`, loaded by a mode
/// write, and read back through the buffer with the two syncs.
#[test]
fn the_mode_bits_are_mits_own() {
    let src = lmtv_order();

    // The Mode register's own block, down to the next register.
    let at = src.find("173777x0  Mode").expect("the Mode register");
    let block = &src[at..src[at..].find("173777x1").unwrap() + at];
    assert!(block.contains("(read/write)"), "it reads back, which is why read_control exists");

    for (line, bits) in [
        ("1-0  Clock Mode 1-0", mode::CLOCK),
        ("2  Black on White", mode::BOW),
        ("3  Vertical Flag Interrupt Enable", mode::INTERRUPT_ENABLE),
        ("4  Vertical Flag", mode::VERT),
        ("5  Vertical Sync", mode::VSYNC),
        ("6  Horizontal Sync", mode::HSYNC),
    ] {
        assert!(block.contains(line), "{line:?} is on the sheet");
        assert_ne!(bits, 0);
    }
    assert_eq!(mode::CLOCK | mode::BOW | mode::INTERRUPT_ENABLE, mode::WRITABLE, "bits 3-0");
    assert_eq!(mode::VERT | mode::VSYNC | mode::HSYNC, mode::READ_ONLY & !mode::SYNC_PROM_ENABLE);

    // Bit 7 up the sheet calls garbage; `nxbctl.drw` has `SYNC PROM ENB` on
    // XDO 7 through the read buffer, and ECO 2 of `lmtv.eco` grounds that
    // buffer input on this board, "on old TV boards the check if TV is in
    // PROM mode (extant only on new TV boards) reads an unused input". So
    // it reads zero either way, which is what `src/simpletv.rs` gives.
    assert!(block.contains("31-7  Garbage"), "everything above bit 6");
    assert_eq!(mode::SYNC_PROM_ENABLE, 0o200);
    let mut tv = SimpleTv::default();
    tv.write_control(0, !0, 0);
    assert_eq!(tv.mode() & mode::SYNC_PROM_ENABLE, 0, "bit 7 reads zero");
}

/// `-XBUS INIT` reaches one flop on this board: the vertical flag's, the
/// 74LS74 at NXBCTL 0E14, whose clear is `-RESET` off `XBUS INIT IN`. The
/// mode register and the sync enable clear on `-POWER RESET`, a different
/// backplane wire, so they stand --- where `lmtv.order` has the sync enable
/// "cleared by Xbus reset", the drawing is followed.
#[test]
fn an_xbus_init_clears_the_vertical_flag_and_nothing_else() {
    use muir::simpletv::FRAME_NS;
    let mut tv = SimpleTv::default();
    tv.write_control(0, mode::INTERRUPT_ENABLE | mode::BOW | mode::VERT, 10);
    tv.write_control(2, 0o17, 10);
    tv.write_control(3, 0o200 | 0o5, 10);
    tv.write_control(1, 0o77, 10);
    tv.write_buffer(3, 0xdead_beef);
    assert!(tv.interrupt(11), "the flag written one, with the enable, interrupts");

    tv.xbus_init(11);
    assert!(!tv.vert_flag(11), "the flop is cleared");
    assert!(!tv.interrupt(11), "and the interrupt with it");
    assert_eq!(
        tv.mode(),
        mode::INTERRUPT_ENABLE | mode::BOW,
        "the 25LS2519 at 0F12 clears on -POWER RESET, not on -XBUS INIT"
    );
    assert!(tv.sync.enabled(), "the 74LS273 at NTVINC 0A07 likewise");
    assert_eq!(tv.sync.enable, 0o205, "spacing and all");
    assert_eq!(tv.sync.pointer, 0o17, "the 74LS374s at NSYADR have no clear");
    assert_eq!(tv.read_control(1, 11), 0o77, "the 2147s keep the program");
    assert_eq!(tv.read_buffer(3), 0xdead_beef, "the frame buffer is DRAM");
    assert!(tv.vert_flag(11 + FRAME_NS), "the next frame start presets the flop again");
}
