// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The black-and-white display, checked against MIT's own window system.
//!
//! Both boards are here: one model serves the SIMPLE TV and the LISPM TV,
//! and what `--tv-board` changes is mode bit 7. What the boards do is read
//! off them in `tests/simpletv_netlist.rs` and `tests/lispmtv_netlist.rs`;
//! this holds the model to it.
//!
//! The constants in `src/tv.rs` are not asserted against themselves here:
//! `the_geometry_is_mits_own` reads `sys/window/shwarm.lisp` out of the
//! System 100 release and fails if we have drifted from it. Without the
//! vendored release that one test reports that it was skipped.

use muir::busint::{self, Responder};
use muir::machine::{MAIN_WORDS, Machine, bus_error};
use muir::tv::{self, Board, FRAME_NS, Tv, mode};

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

/// Every number in `src/tv.rs` that MIT states, taken from MIT.
#[test]
fn the_geometry_is_mits_own() {
    let Some(src) = shwarm() else { return };

    // The control address is an Xbus I/O offset; physical is that plus the
    // base of I/O space, which is where the frame buffer starts.
    let control = defconst_octal(&src, "MAIN-SCREEN-CONTROL-ADDRESS");
    assert_eq!(control, 0o377760, "MIT's control address");
    assert_eq!(
        tv::CONTROL,
        tv::BUFFER + control,
        "control register is at the buffer base + offset"
    );

    assert_eq!(
        tv::BUFFER_WORDS,
        defconst_octal(&src, "MAIN-SCREEN-BUFFER-LENGTH"),
        "MAIN-SCREEN-BUFFER-LENGTH"
    );
    assert_eq!(tv::WIDTH, cadr_decimal(&src, "MAIN-SCREEN-WIDTH"), "MAIN-SCREEN-WIDTH");
    assert_eq!(tv::HEIGHT, cadr_decimal(&src, "MAIN-SCREEN-HEIGHT"), "MAIN-SCREEN-HEIGHT");
    assert_eq!(
        tv::WORDS_PER_LINE,
        cadr_decimal(&src, "MAIN-SCREEN-LOCATIONS-PER-LINE"),
        "MAIN-SCREEN-LOCATIONS-PER-LINE"
    );
}

/// One bit per pixel, and the line stride says so.
#[test]
fn a_line_is_the_width_in_bits() {
    assert_eq!(tv::WORDS_PER_LINE * 32, tv::WIDTH, "24 words of 32 bits is 768 pixels");
    let used = tv::WORDS_PER_LINE * tv::HEIGHT;
    assert!(used <= tv::BUFFER_WORDS as usize, "the screen must fit in the buffer");
    assert_eq!(used, 23_112, "what the screen actually occupies");
}

/// MIT's own `BLACK-ON-WHITE` and `WHITE-ON-BLACK`, which read the register,
/// change one bit and write it back. The register has to read back for that
/// to work at all.
#[test]
fn black_on_white_is_one_bit_read_modify_written() {
    let mut tv = Tv::default();
    assert!(!tv.black_on_white(), "comes up showing one bits as white");

    // (%XBUS-WRITE control (LOGIOR 4 (%XBUS-READ control)))
    tv.write_control(0, 4 | tv.read_control(0, 0), 0);
    assert!(tv.black_on_white(), "BLACK-ON-WHITE sets MODE<2>");
    assert_eq!(tv.mode() & mode::BOW, mode::BOW);

    // (%XBUS-WRITE control (LOGAND -5 (%XBUS-READ control)))  ;1's comp of 4
    tv.write_control(0, (!4u32) & tv.read_control(0, 0), 0);
    assert!(!tv.black_on_white(), "WHITE-ON-BLACK clears it again");
}

/// Writing the mode register must not invent sync signals: bits 5 and 6
/// are the sync program's, read where it has them clear, and bit 7 is the
/// sync enable read back, grounded on the SIMPLE TV this `Tv` is.
#[test]
fn the_read_only_bits_never_stick() {
    let mut tv = Tv::default();
    // Clock mode 0 kept, so that the program's timing is the PROM's.
    tv.write_control(0, !mode::CLOCK, 0);
    assert_eq!(
        tv.mode(),
        mode::BOW | mode::INTERRUPT_ENABLE,
        "the four data pins of the 2519 land, and the read-only bits do not"
    );
    // Five milliseconds in: a picture line, sixteen instructions along it,
    // where `cpt.prom` has neither sync bit up.
    assert_eq!(
        tv.read_control(0, 5_000_000) & (mode::VSYNC | mode::HSYNC | mode::SYNC_PROM_ENABLE),
        0
    );
    assert_eq!(tv.read_control(0, 500) & mode::SYNC_PROM_ENABLE, 0, "grounded, whenever");
}

/// **The vertical flag is a flop of its own, and a mode write loads it.**
/// `-TVMA CLR` presets it where the sync program has it --- for the PROM
/// program from power-on, as the first line's last instruction completes,
/// 16.000 us in, and then every [`FRAME_NS`]; a write of the mode register
/// clocks the written bit 4 into it, which is how `INTRX0` clears it ---
/// read, clear bit 4, write back; and with `MODE INTR ENB` it is the
/// interrupt the microcode takes as the 60-cycle clock. The drawing reads
/// as though the bit were not writable; the 74LS74 at NXBCTL 0E14 is what
/// settles it (discrepancy 26).
#[test]
fn the_vertical_flag_sets_each_frame_and_a_mode_write_clears_it() {
    use muir::tv::FRAME_NS;
    const CLR: u64 = 16_000;
    let mut tv = Tv::default();
    assert!(!tv.vert_flag(0), "cleared by reset");
    assert!(!tv.vert_flag(CLR - 1), "and not yet set before the first line ends");
    assert!(tv.vert_flag(CLR), "set by TVMA CLR as it does");
    assert_eq!(tv.read_control(0, CLR) & mode::VERT, mode::VERT, "and read back in bit 4");
    assert!(!tv.interrupt(CLR), "no interrupt without the enable");

    // The window system's enable, and the microcode's clear: bit 4 written
    // zero with the rest of the register kept.
    tv.write_control(0, mode::INTERRUPT_ENABLE, CLR + 1);
    assert!(!tv.vert_flag(CLR + 1), "the write cleared it");
    assert!(!tv.interrupt(CLR + 1));
    assert!(!tv.vert_flag(CLR + FRAME_NS - 1), "and it stays clear for the frame");
    assert!(tv.vert_flag(CLR + FRAME_NS), "the next frame sets it again");
    assert!(tv.interrupt(CLR + FRAME_NS), "and now it interrupts");
    assert_eq!(
        tv.read_control(0, CLR + FRAME_NS) & !(mode::HSYNC | mode::VSYNC),
        mode::INTERRUPT_ENABLE | mode::VERT
    );
    tv.write_control(0, mode::INTERRUPT_ENABLE, CLR + FRAME_NS + 1);
    assert!(!tv.interrupt(CLR + FRAME_NS + 1), "dismissed");

    // Written one, it is one: the flop takes what it is given.
    tv.write_control(0, mode::INTERRUPT_ENABLE | mode::VERT, CLR + FRAME_NS + 2);
    assert!(tv.interrupt(CLR + FRAME_NS + 3), "a write of bit 4 set it");
}

#[test]
fn the_display_answers_where_nothing_did_before() {
    let d = |p| busint::decode(p, MAIN_WORDS);
    assert_eq!(d(tv::BUFFER), Responder::Device, "the first word of the screen");
    assert_eq!(d(tv::BUFFER + tv::BUFFER_WORDS - 1), Responder::Device, "the last word");
    assert_eq!(d(tv::CONTROL), Responder::Device, "the mode register");
    assert_eq!(d(tv::CONTROL + tv::CONTROL_WORDS - 1), Responder::Device, "its last word");

    assert_eq!(d(tv::BUFFER + tv::BUFFER_WORDS), Responder::NoXbus, "just past the screen");
    assert_eq!(d(tv::CONTROL - 1), Responder::NoXbus, "just below the mode register");
    // The disk controller is four words further up the same page.
    assert_eq!(d(0o17377774), Responder::Device, "the disk still decodes");
}

/// A word written over the bus comes back, and no bus error is raised.
#[test]
fn the_bus_reaches_the_frame_buffer() {
    let mut m = Machine::new();
    let last = tv::BUFFER + tv::BUFFER_WORDS - 1;

    m.bus_write(tv::BUFFER, 0o12345670123);
    m.bus_write(last, 1);
    assert_eq!(m.bus_read(tv::BUFFER), 0o12345670123);
    assert_eq!(m.bus_read(last), 1);
    assert_eq!(m.bus_error, 0, "the display answers, so nothing times out");

    // The mode register is on the same bus and is not the buffer; its sync
    // bits are the program's, wherever it stands.
    m.bus_write(tv::CONTROL, mode::BOW);
    assert_eq!(m.bus_read(tv::CONTROL) & !(mode::HSYNC | mode::VSYNC), mode::BOW);
    assert!(m.tv.black_on_white());
    assert_eq!(m.bus_error, 0);
}

/// Past the screen there is still nothing, and it still says so.
#[test]
fn past_the_display_the_xbus_still_times_out() {
    let mut m = Machine::new();
    m.bus_read(tv::BUFFER + tv::BUFFER_WORDS);
    assert_eq!(m.bus_error & bus_error::XBUS_NXM, bus_error::XBUS_NXM);
}

/// Bit order within a word: pixel 0 of a line is bit 0 of its first word.
#[test]
fn the_first_pixel_of_a_line_is_the_low_bit() {
    let mut tv = Tv::default();
    tv.write_buffer(0, 1);
    assert!(tv.pixel(0, 0), "bit 0 is the leftmost pixel");
    assert!(!tv.pixel(1, 0));

    // The first word of line 1 is WORDS_PER_LINE in.
    tv.write_buffer(tv::WORDS_PER_LINE as u32, 1);
    assert!(tv.pixel(0, 1), "line 1 starts one stride along");
    assert_eq!(tv.buffer().len(), tv::BUFFER_WORDS as usize);
}

/// **A sync pointer past the RAM is a corrupt checkpoint, and is refused**
/// before the next register access indexes the RAM with it. The pointer
/// is the twelve-bit address of the eight 2147s, and a write to register
/// 2 keeps twelve bits, so a saved pointer is never wider.
#[test]
fn a_sync_pointer_past_the_ram_is_refused() {
    use muir::checkpoint::{Reader, Writer};
    let mut tv = Tv::default();
    tv.sync.pointer = tv::SYNC_RAM_WORDS as u16;
    let mut w = Writer::new();
    tv.save(&mut w);
    let body = w.finish();
    let err = Tv::default().load(&mut Reader::new(&body)).unwrap_err().to_string();
    assert!(err.contains("pointer"), "{err}");
    // The last word's address loads as itself.
    tv.sync.pointer = tv::SYNC_RAM_WORDS as u16 - 1;
    let mut w = Writer::new();
    tv.save(&mut w);
    let body = w.finish();
    let mut back = Tv::default();
    let mut r = Reader::new(&body);
    back.load(&mut r).unwrap();
    r.done().unwrap();
    assert_eq!(back.sync.pointer, tv::SYNC_RAM_WORDS as u16 - 1);
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
/// color board is the other x.
#[test]
fn the_addresses_are_mits_own() {
    let src = lmtv_order();

    assert!(src.contains("For the normal TV, x is 6"), "which x the normal TV is");
    assert_eq!(tv::CONTROL, 0o17377760, "173777x0 with x = 6");

    assert!(src.contains("17x00000-17x77777"), "the buffer's range");
    assert!(src.contains("32K x 32 bits of video buffer"), "its size");
    assert!(src.contains("buffer starts at 17000000"), "where the normal TV's buffer is");
    assert_eq!(tv::BUFFER, 0o17000000);
    assert_eq!(tv::BUFFER_WORDS, 0o100000, "32K words");

    // Eight control words: five that do something and three that "respond
    // but don't do anything".
    assert!(src.contains("173777x5,6,7  These addresses respond but don't do anything"));
    assert_eq!(tv::CONTROL_WORDS, 8);
}

/// **The mode bits are MIT's**, named and numbered on the order sheet, and
/// bit for bit what `src/tv.rs` has. Bit 4 is not in the register:
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
    // on the SIMPLE TV it reads zero whatever the sync enable is doing,
    // which is what `src/tv.rs` gives; the LISPM TV reads the enable back
    // there, and `the_prom_mode_bit_reads_the_sync_enable_on_the_lispm_tv_alone`
    // is the pair of boards.
    assert!(block.contains("31-7  Garbage"), "everything above bit 6");
    assert_eq!(mode::SYNC_PROM_ENABLE, 0o200);
    let mut tv = Tv::default();
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
    use muir::tv::FRAME_NS;
    let mut tv = Tv::default();
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
    // The RAM selected above holds one word and no program, so nothing
    // presets the flop; the PROM back in, its program's next TVMA CLR does.
    assert!(!tv.vert_flag(11 + FRAME_NS), "a RAM with no program in it makes no frame");
    tv.write_control(3, 0o5, 12);
    assert!(tv.vert_flag(12 + 16_000), "the PROM program's TVMA CLR presets the flop again");
}

/// **Mode bit 7 is the one bit of the interface the two boards differ in.**
///
/// On the LISPM TV the read buffer's fourth input, XBCTL 0F11 pin 8, is
/// the net `-SYNC PROM ENB`, which the 74LS273 at TVINC 0A07 drives from
/// `XDI7` --- register 3's bit 7, the sync enable --- so the bit reads back
/// the enable: one while the sync RAM is selected, zero while MIT's PROM
/// is. On the SIMPLE TV that same pin is `GND`, ECO 2 of `cadrtv/lmtv.eco`
/// grounding it, so it reads zero whatever the enable says.
///
/// `tests/lispmtv_netlist.rs` and `tests/simpletv_netlist.rs` read both
/// boards back through a bus cycle; this is the model beside them.
#[test]
fn the_prom_mode_bit_reads_the_sync_enable_on_the_lispm_tv_alone() {
    for (board, selected) in [(Board::SimpleTv, 0), (Board::LispmTv, mode::SYNC_PROM_ENABLE)] {
        let mut tv = Tv::default();
        tv.set_board(board);
        assert_eq!(tv.board(), board);
        assert_eq!(
            tv.read_control(0, 0) & mode::SYNC_PROM_ENABLE,
            0,
            "{}: the PROM is selected at power-on",
            board.name()
        );

        // Register 3 bit 7 up: the RAM is in, and the PROM out.
        tv.write_control(3, 0o200, 10);
        assert!(tv.sync.enabled());
        assert_eq!(
            tv.read_control(0, 20) & mode::SYNC_PROM_ENABLE,
            selected,
            "{}: with the sync RAM selected",
            board.name()
        );

        // And back to the PROM.
        tv.write_control(3, 0, 30);
        assert_eq!(
            tv.read_control(0, 40) & mode::SYNC_PROM_ENABLE,
            0,
            "{}: with the PROM selected again",
            board.name()
        );
        assert_eq!(tv.mode() & mode::SYNC_PROM_ENABLE, 0, "the bit is in no register");
    }
}

/// **Register 4 is the color map's write port, on either board.**
///
/// `lmtv.order`: "173777x4 Color (write only) 15-8 Value to write into
/// color map, 7-6 Select which color map (up to 4 channels), 3-0 Color
/// (i.e. address into color map)", and `WRITE-COLOR-MAP` in
/// `sys/window/color.lisp` writes exactly that --- `(DPB R 1010 LOC)`,
/// then the same with `(DPB 1 0602 LOC)` and `(DPB 2 0602 LOC)`, so
/// channel 0 is red, 1 green and 2 blue, each stored as `377 - value`.
/// The fourth channel the field can name is not wired: the 74S139 at
/// COLOR 0E10 decodes `XDI7`, `XDI6` into `-LOAD COLOR 0`, `1` and `2`,
/// and its fourth output is NC.
///
/// **Both boards do this.** The page is `COLOR` on the LISPM TV and
/// `NRACOL` --- `lmtv.stf`'s "SIMPLE TV / COLOR MAP" --- on the SIMPLE TV,
/// the same parts wired the same way, and each board is measured strobing
/// the map in its own netlist test. So the color register is not what
/// tells the two apart; mode bit 7 is.
#[test]
fn the_color_register_writes_the_map_on_either_board() {
    // (DPB value 1010 (DPB channel 0602 color)), as MIT's own writes are.
    let write = |value: u32, channel: u32, color: u32| value << 8 | channel << 6 | color;

    for board in [Board::SimpleTv, Board::LispmTv] {
        let mut tv = Tv::default();
        tv.set_board(board);
        assert_eq!(tv.color_map(), &[[0; 3]; tv::COLORS], "nothing is written at power-on");

        tv.write_control(4, write(0o252, 0, 5), 0);
        tv.write_control(4, write(0o123, 1, 5), 0);
        tv.write_control(4, write(0o077, 2, 5), 0);
        assert_eq!(
            tv.color_map()[5],
            [0o252, 0o123, 0o077],
            "{}: red, green and blue of color 5",
            board.name()
        );

        // The fourth channel decodes to the 74S139's unconnected output.
        tv.write_control(4, write(0o377, 3, 5), 0);
        assert_eq!(tv.color_map()[5], [0o252, 0o123, 0o077], "channel 3 strobes nothing");
        for (color, entry) in tv.color_map().iter().enumerate() {
            assert!(color == 5 || entry == &[0; 3], "color {color} was not written");
        }

        // Sixteen colors, `(LOGAND LOC 17)` in MIT's own write.
        tv.write_control(4, write(0o11, 0, 0o17), 0);
        assert_eq!(tv.color_map()[0o17][0], 0o11, "the last color");

        // Write only: `lmtv.order` gives the register no read, and
        // `color.lisp` keeps `HARDWARE-COLOR-MAP` in the band because "the
        // hardware does not allow reading back of the color map".
        assert_eq!(tv.read_control(4, 0), 0);
    }
}

/// **A checkpoint carries the board and its color map**, so that a
/// resumed run is the machine that was stopped and not another one with
/// the same buffer in it.
#[test]
fn the_board_and_its_color_map_go_through_a_checkpoint() {
    use muir::checkpoint::{Reader, Writer};

    let mut tv = Tv::default();
    tv.set_board(Board::LispmTv);
    tv.write_control(4, 0o252 << 8 | 5, 0);
    tv.write_control(3, 0o200, 0);
    let mut w = Writer::new();
    tv.save(&mut w);
    let body = w.finish();

    let mut back = Tv::default();
    let mut r = Reader::new(&body);
    back.load(&mut r).unwrap();
    r.done().unwrap();
    assert_eq!(back.board(), Board::LispmTv);
    assert_eq!(back.color_map()[5], [0o252, 0, 0]);
    assert_eq!(back.read_control(0, 0) & mode::SYNC_PROM_ENABLE, mode::SYNC_PROM_ENABLE);

    // There are two boards, so a third is a corrupt checkpoint and is
    // refused rather than taken for one of them.
    let mut wrong = body.clone();
    wrong[0] = 2;
    let err = Tv::default().load(&mut Reader::new(&wrong)).unwrap_err().to_string();
    assert!(err.contains("display board 2"), "{err}");
}

// --- The color TV, the second board ----------------------------------------

/// **The color TV's strap is MIT's other x**, and neither board answers
/// the other's addresses.
///
/// `lmtv.order`: "Note: For the normal TV, x is 6.  For the color TV, x is
/// 5", and of the buffer, "The normal TV has x equal to 0, so the buffer
/// starts at 17000000.  The color TV has x equal to 2, and so the buffer
/// starts at 17200000." System 100 asks for the same two ---
/// `COLOR:MAKE-SCREEN` in `sys/window/color.lisp` defines the screen
/// `:BUFFER -600000 :CONTROL-ADDRESS 377750`, whose `(LOGAND ... 377777)`
/// is `200000` as an Xbus I/O offset --- so the strap is settled twice
/// over.
#[test]
fn the_color_tv_is_strapped_to_mits_other_x() {
    let src = lmtv_order();
    assert!(src.contains("For the color TV, x is 5"), "which x the color TV is");
    assert!(src.contains("has x equal to 2, and so the buffer starts at 17200000"));
    assert_eq!(tv::COLOR_TV.control, 0o17377750, "173777x0 with x = 5");
    assert_eq!(tv::COLOR_TV.buffer, 0o17200000);

    // Each strap answers its own eight registers and 32K buffer words, and
    // nothing of the other's.
    let color = Tv::color();
    let normal = Tv::default();
    assert_eq!(color.strap(), tv::COLOR_TV);
    assert_eq!(normal.strap(), tv::NORMAL_TV);
    for (what, phys) in [("the buffer", 0o17200000), ("the last buffer word", 0o17277777)] {
        assert!(tv::COLOR_TV.buffer_offset(phys).is_some(), "the color {what}");
        assert!(
            tv::NORMAL_TV.buffer_offset(phys).is_none(),
            "and the normal board is not at {what}"
        );
    }
    assert_eq!(tv::COLOR_TV.buffer_offset(0o17277777), Some(0o77777));
    assert!(tv::COLOR_TV.buffer_offset(0o17300000).is_none(), "past the color buffer");
    assert_eq!(tv::COLOR_TV.control_register(0o17377754), Some(4), "the color register");
    assert_eq!(tv::COLOR_TV.control_register(0o17377757), Some(7), "the last of the eight");
    assert!(tv::COLOR_TV.control_register(0o17377760).is_none(), "which is the normal board's 0");
    assert_eq!(tv::NORMAL_TV.control_register(0o17377760), Some(0));
    assert!(tv::NORMAL_TV.control_register(0o17377750).is_none(), "the color board's 0");
    assert!(!tv::NORMAL_TV.answers(0o17200000) && !tv::NORMAL_TV.answers(0o17377750));
    assert!(!tv::COLOR_TV.answers(0o17000000) && !tv::COLOR_TV.answers(0o17377760));

    // The color board is a LISPM TV: the four-bit picture and its map are
    // that board's, and `--tv-board` is the normal TV's flag alone.
    assert_eq!(color.board(), Board::LispmTv);
}

/// **The color picture's geometry is MIT's own**, read out of
/// `COLOR:MAKE-SCREEN`.
///
/// Read from the vendored release; skipped without it.
#[test]
fn the_color_geometry_is_mits_own() {
    let Some(src) = release("window/color.lisp") else { return };
    let at = src.find("(DEFUN MAKE-SCREEN").expect("COLOR:MAKE-SCREEN");
    let block = &src[at..at + 600];
    assert!(block.contains("':BITS-PER-PIXEL 4"), "four bits a pixel:\n{block}");
    assert!(block.contains("':HEIGHT 454."), "the height:\n{block}");
    assert!(block.contains("':WIDTH 576."), "the width:\n{block}");
    assert!(block.contains("':CONTROL-ADDRESS CONTROL-ADR"), "the control address:\n{block}");
    assert!(block.contains("(CONTROL-ADR 377750)"), "and where that is:\n{block}");
    assert!(block.contains("(XBUS-ADR -600000)"), "and where the buffer is:\n{block}");
    assert_eq!(tv::COLOR_WIDTH, 576);
    assert_eq!(tv::COLOR_HEIGHT, 454);
    assert_eq!(tv::COLOR_BITS_PER_PIXEL, 4);
    assert_eq!(tv::COLOR_WORDS_PER_LINE, 72, "576 pixels of 4 bits is 72 words of 32");

    // `(LOGAND (TV:SCREEN-BUFFER SCREEN) 377777)` of `-600000`, which is
    // what `COLOR-EXISTS-P` writes to as an Xbus I/O offset.
    let offset = (-0o600000i32) as u32 & 0o377777;
    assert_eq!(tv::BUFFER + offset, tv::COLOR_TV.buffer, "COLOR-EXISTS-P's own address");
}

/// **A four-bit pixel is a nibble of the buffer, the low one first.**
///
/// `COLOR:MAKE-SCREEN` displaces an `ART-4B` array onto the buffer.
/// `sys/cold/qcom.lisp` gives `ART-4B` eight elements a word of four bits
/// each, and `XCOLOR-TRANSFORM` in `sys/ucadr/uc-hacks.lisp` --- MIT's own
/// microcode walking such an array over this screen --- takes element `k`
/// from bit `4 * (k mod 8)` of word `k / 8`: `((M-K) DPB M-J (BYTE-FIELD 3
/// 2) A-ZERO) ;Rotation amount in bits`, and the word offset
/// `(BYTE-FIELD (DIFFERENCE Q-POINTER-WIDTH 3) 3) M-Q`. So the low nibble
/// is the leftmost pixel, as `lmtv.order` has the low-order bit of a word
/// sent to the TV first.
#[test]
fn a_four_bit_pixel_is_a_nibble_low_one_first() {
    let mut tv = Tv::color();
    // The first eight pixels of line 0, in a word: 0, 1, 2 ... 7.
    tv.write_buffer(0, 0x7654_3210);
    for x in 0..8 {
        assert_eq!(tv.pixel4(x, 0), x as u8, "pixel {x} of the first word");
    }
    // The ninth is the next word's low nibble, and a line is 72 words.
    tv.write_buffer(1, 0x0000_000f);
    assert_eq!(tv.pixel4(8, 0), 0o17);
    tv.write_buffer(tv::COLOR_WORDS_PER_LINE as u32, 0x0000_00a0);
    assert_eq!(tv.pixel4(0, 1), 0, "line 1 begins a word later");
    assert_eq!(tv.pixel4(1, 1), 0o12);
    // The last pixel of the last line is inside the buffer.
    let last = (tv::COLOR_HEIGHT - 1) * tv::COLOR_WORDS_PER_LINE + tv::COLOR_WORDS_PER_LINE - 1;
    assert!((last as u32) < tv::BUFFER_WORDS, "454 lines of 72 words fit the 32K buffer");
    tv.write_buffer(last as u32, 0x9000_0000);
    assert_eq!(tv.pixel4(tv::COLOR_WIDTH - 1, tv::COLOR_HEIGHT - 1), 9);
}

/// **The map is shown inverted**, because that is the software's own model
/// of it: `WRITE-COLOR-MAP` stores `377 - value` and `READ-COLOR-MAP`
/// hands back `377 - stored`. What the D-A off the board makes of a stored
/// byte is undocumented, so the inversion is the only reference there is,
/// and `Tv::rgb` is where that decision lives.
#[test]
fn the_map_is_shown_inverted_as_the_software_stores_it() {
    let mut tv = Tv::color();
    // A map fresh from power-on holds zeros, which show as full white ---
    // the inversion, applied to a board nobody has written.
    assert_eq!(tv.rgb(0), [255, 255, 255], "the unwritten map");

    // What `WRITE-COLOR-MAP` puts in the register: `377 - value`, so
    // `COLOR-MAP-ON`, 377, is written as 0 and `COLOR-MAP-OFF`, 0, as 377.
    let (on, off): (u32, u32) = (0, 0o377);

    // `R-G-B-COLOR-MAP`'s color 0: `COLOR-MAP-OFF` on every channel.
    for channel in 0..3u32 {
        tv.write_control(4, off << 8 | channel << 6, 0);
    }
    assert_eq!(tv.color_map()[0], [0o377, 0o377, 0o377], "stored inverted");
    assert_eq!(tv.rgb(0), [0, 0, 0], "and shown black");

    // Color 1, full green: `COLOR-MAP-ON` on channel 1 and off on the
    // others.
    tv.write_control(4, off << 8 | 1, 0);
    tv.write_control(4, on << 8 | 1 << 6 | 1, 0);
    tv.write_control(4, off << 8 | 2 << 6 | 1, 0);
    assert_eq!(tv.color_map()[1], [0o377, 0, 0o377]);
    assert_eq!(tv.rgb(1), [0, 255, 0], "full green");

    // The color is four bits: `WRITE-COLOR-MAP` writes `(LOGAND LOC 17)`.
    assert_eq!(tv.rgb(0o21), tv.rgb(1), "the address is four bits wide");
}

/// **`COLOR-EXISTS-P`'s probe: the board answers when it is fitted and the
/// bus faults when it is not.**
///
/// `sys/window/color.lisp`: `XBUS-LOCATION-EXISTS-P` writes `BITS` into
/// the address and reads it back with the error stop off
/// (`XBUS-READ-NO-PARITY`), and `COLOR-EXISTS-P` calls it with `1` at
/// `(LOGAND (TV:SCREEN-BUFFER SCREEN) 377777)`, the first word of the
/// color buffer. That is how the release finds out whether there is a
/// color screen, so a machine without the board has to give it the NXM.
#[test]
fn color_exists_p_finds_the_board_only_when_it_is_fitted() {
    const PROBE: u32 = 0o17200000;
    const CONTROL: u32 = 0o17377750;
    let m = MAIN_WORDS;

    // The decode, which is what says whether anything is there at all.
    for phys in [PROBE, PROBE + 0o77777, CONTROL, CONTROL + 7] {
        assert_eq!(busint::decode(phys, m), Responder::NoXbus, "{phys:o} with no board");
        assert_eq!(busint::decode_with(phys, m, false), Responder::NoXbus);
        assert_eq!(busint::decode_with(phys, m, true), Responder::Device, "{phys:o} with one");
    }
    // The normal board is there either way, and the gap between the two
    // control blocks is nobody's.
    assert_eq!(busint::decode_with(0o17377760, m, false), Responder::Device, "the normal TV");
    assert_eq!(busint::decode_with(0o17300000, m, true), Responder::NoXbus, "past the buffer");

    // The probe itself, on a machine with no board: the write goes
    // nowhere and the read faults.
    let mut without = Machine::new();
    assert!(without.color_tv.is_none());
    without.bus_write(PROBE, 1);
    assert_eq!(without.bus_read(PROBE), 0);
    assert_ne!(without.bus_error & bus_error::XBUS_NXM, 0, "COLOR-EXISTS-P gets the NXM");

    // And with the board: the word reads back, which is what
    // `(BIT-TEST BITS ...)` wants.
    let mut with = Machine::new();
    with.fit_color_tv();
    with.bus_write(PROBE, 1);
    assert_eq!(with.bus_read(PROBE), 1, "the board answers");
    assert_eq!(with.bus_error & bus_error::XBUS_NXM, 0, "and no bus error");
    // The main screen is untouched by it: two boards, two buffers.
    assert_eq!(with.tv.read_buffer(0), 0, "the main screen's first word");
    // The color board's own registers, at the other strap.
    with.bus_write(CONTROL + 4, 0o252 << 8 | 5);
    assert_eq!(with.color_tv.as_ref().unwrap().color_map()[5], [0o252, 0, 0]);
    assert_eq!(with.tv.color_map()[5], [0, 0, 0], "and not the main board's map");
}

/// **Both display boards drive the one interrupt line, and `-XBUS INIT`
/// reaches both.**
///
/// `-XBUS.INTR` is a bused line with one open-collector driver a board, so
/// the line is the boards ORed; `-XBUS INIT` likewise reaches every board
/// on the backplane. Nothing in System 100 enables the color board's
/// interrupt --- `COLOR:SETUP` starts its sync with `(SI:START-SYNC 3 0
/// 36.)`, which `CC-TV-START-SYNC` writes as the clock mode alone --- but
/// the wire is the wire.
#[test]
fn both_boards_are_on_the_one_interrupt_line() {
    let mut m = Machine::new();
    m.fit_color_tv();
    // A mode write puts its own bit 4 into the vertical flag's flop, so
    // one that enables the interrupt leaves the flag down; a frame later
    // the program's `TVMA CLR` has preset it again. Both boards run the
    // PROM's program from power-on, so a frame is `FRAME_NS` for each.
    m.ns = FRAME_NS;
    assert!(!m.xbus_interrupt(), "neither enable is up");

    // The color board alone, through its own registers.
    m.bus_write(0o17377750, mode::INTERRUPT_ENABLE);
    assert!(!m.xbus_interrupt(), "the write cleared the flag");
    m.ns += FRAME_NS;
    assert!(m.xbus_interrupt(), "the color board's SEND INTR is on the line");
    assert!(m.color_tv.as_ref().unwrap().interrupt(m.ns));
    assert!(!m.tv.interrupt(m.ns), "and it is not the main board's");

    // The main board alone.
    m.bus_write(0o17377750, 0);
    assert!(!m.xbus_interrupt());
    m.bus_write(0o17377760, mode::INTERRUPT_ENABLE);
    m.ns += FRAME_NS;
    assert!(m.xbus_interrupt(), "the main board's");
    assert!(!m.color_tv.as_ref().unwrap().interrupt(m.ns), "with the color board's enable down");

    // `-XBUS INIT` clears the vertical flag on both: `Machine::bus_reset`
    // is the wire.
    m.bus_write(0o17377750, mode::INTERRUPT_ENABLE);
    m.ns += FRAME_NS;
    assert!(m.tv.vert_flag(m.ns) && m.color_tv.as_ref().unwrap().vert_flag(m.ns));
    m.bus_reset();
    assert!(!m.tv.vert_flag(m.ns), "the main board's flag cleared");
    assert!(!m.color_tv.as_ref().unwrap().vert_flag(m.ns), "and the color board's");
    assert!(!m.xbus_interrupt(), "so nothing is on the line");
}

/// **A checkpoint carries the second board, and whether there was one.**
/// A machine with a color screen is not the machine without one, so a
/// resume that disagrees is refused rather than run --- `--color-tv` by
/// name, as `--tv-board` is --- and this is the half of that the format
/// holds.
#[test]
fn the_color_tv_goes_through_a_machine_checkpoint() {
    use muir::checkpoint::{Reader, Writer};

    let mut m = Machine::new();
    m.fit_color_tv();
    m.bus_write(0o17200000, 0x1234_5678);
    m.bus_write(0o17377750, 3); // clock mode 3, as COLOR:SETUP starts it
    m.bus_write(0o17377754, 0o252 << 8 | 1 << 6 | 5);
    let mut w = Writer::new();
    m.save(&mut w);
    let body = w.finish();

    let mut back = Machine::new();
    back.fit_color_tv();
    back.load(&mut Reader::new(&body)).unwrap();
    let tv = back.color_tv.as_ref().expect("the board came back");
    assert_eq!(tv.strap(), tv::COLOR_TV);
    assert_eq!(tv.board(), Board::LispmTv);
    assert_eq!(tv.read_buffer(0), 0x1234_5678);
    assert_eq!(tv.mode() & tv::mode::CLOCK, 3);
    assert_eq!(tv.color_map()[5], [0, 0o252, 0]);

    // A machine built without the board takes the same file and comes back
    // with one, so that the flag's refusal has something to compare.
    let mut plain = Machine::new();
    plain.load(&mut Reader::new(&body)).unwrap();
    assert!(plain.color_tv.is_some(), "the checkpoint says there was a board");

    // And the other way: a machine with no board writes none.
    let mut w = Writer::new();
    Machine::new().save(&mut w);
    let mut had = Machine::new();
    had.fit_color_tv();
    had.load(&mut Reader::new(&w.finish())).unwrap();
    assert!(had.color_tv.is_none(), "and none when there was none");
}
