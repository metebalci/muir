// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The instruction word's fields, where a second source settles a width.

use muir::engine::Engine;
use muir::isa::{Insn, asm};
use muir::machine::Machine;
use muir::micro::Micro;
use muir::rtl::Rtl;

/// **The JUMP condition select is three bits, `IR<2:0>`.** `ir.bits` draws
/// `COND` over the five columns of the rotate field without saying which
/// of them the condition mux reads; page FLAG does: the 74S08 at 3E14 ANDs
/// `IR0`, `IR1` and `IR2` with `IR5` for its select, and `IR3` and `IR4`
/// reach no part on the page. `micro` reads three; the decoder must too.
#[test]
fn the_jump_condition_is_ir_2_0() {
    let j = Insn::new(0b11111).jump();
    assert_eq!(j.cond, 7);
    assert_eq!(j.rotate, 0b11111, "the rotate is the five bits");
    assert_eq!(Insn::new(0b01000).jump().cond, 0, "IR<3> is not a condition bit");
    let always = Insn::new(asm::ALWAYS).jump();
    assert!(always.internal_cond && always.cond == 7, "ALWAYS is IR<5> with condition 7");
}

/// **The A memory write address is ten bits, `IR<23:14>`.** `ir.bits` marks
/// `IR<24>` "xx" beside the A destination's `IR<25>`, and on page ACTL the
/// 25S09s at 3B28 and 3B29 make `WADR<9:4>` from `IR<23:18>` (the low
/// four from `IR<17:14>`, shortened to `IR<18:14>` under `DESTM`); `IR24`
/// itself reaches nothing but the instruction register's latch at IREG
/// 3C17 and the parity generator at IPAR 3F24. So a destination with
/// `IR<24>` set writes the word `IR<23:14>` names --- on the board, and on
/// both engines.
#[test]
fn the_a_memory_destination_address_is_ir_23_14() {
    // Not 0o100: that is where `asm::filler` writes, every microcycle.
    let with_24 = asm::a_dest(0o200) | (1 << 24);
    let d = Insn::new(with_24).alu().dest;
    assert!(d.is_a_mem());
    assert_eq!(d.a_addr(), 0o200, "IR<24> is not an address bit");

    let program = || {
        let mut m = Machine::new();
        let mut prom = vec![asm::filler(); 512];
        prom[5] = Insn::new(asm::ALU | asm::SETO | with_24);
        m.load_prom(&prom);
        m
    };
    let written = |m: &Machine| -> Vec<usize> {
        m.amem.iter().enumerate().filter(|(_, w)| **w == !0).map(|(i, _)| i).collect()
    };
    let mut r = Rtl::new(program());
    r.boot();
    for _ in 0..40 {
        r.step().unwrap();
    }
    assert_eq!(written(r.machine()), [0o200], "rtl writes IR<23:14>");
    let mut e = Micro::new(program());
    e.boot();
    for _ in 0..40 {
        e.step().unwrap();
    }
    assert_eq!(written(e.machine()), [0o200], "micro writes IR<23:14>");
}
