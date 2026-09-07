// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The microcode stack as `rtl` holds it, against the drawings.

use muir::engine::Engine;
use muir::isa::Insn;
use muir::machine::Machine;
use muir::rtl::Rtl;

/// The instruction after a push reads the stack RAM, not the word being
/// pushed.
///
/// The push is written a microcycle after the instruction that made it, from
/// `SPCW` under `-SWPA = NAND(WP4C, SPUSHD)` at CONTRL 4E30, and meanwhile
/// `SPCWPASS` at 3D21 puts `SPCW` on the `SPC` bus in place of the RAM's
/// output. But that bus goes to the PC's next-address path and the parity
/// check only. The 74S373s at SPCLCH 4A07, 4A09 and 4A10 that drive
/// `M<18:0>` for `SPC-POINTER-AND-DATA` take `SPCO<18:0>`, the 82S21s' own
/// output, so the instruction in a call's delay slot sees the pointer already
/// moved and the stale word at the new slot. Handing it the pushed word
/// instead is the natural reading of the pass-around and is wrong; the board
/// shows it at microcycle 2,198,520 of the band, in a nopped word nothing
/// stored.
#[test]
fn the_word_after_a_push_reads_the_stale_slot_and_not_the_pushed_word() {
    let mut m = Machine::new();
    // What the slot the push is going to held before.
    m.spc[1] = 0o12345;
    // ALU class, ALU output onto OB, SETM, an A-memory destination, and the
    // functional M source 1, `SPC pointer and data`, as `mit/cadr/ir.bits` has
    // them. `IR<44:43>` = 1 is the jump class with the target at `IR<25:12>`,
    // `IR<8>` the push bit and `IR<5>` with `IR<2:0>` = 7 the always-true
    // condition.
    let alu = 1u64 << 12;
    let setm = (1u64 << 4) | (1 << 3);
    let a_dest = |a: u64| (1u64 << 25) | (a << 14);
    let src_spc = (1u64 << 31) | (1 << 26);
    let call_xct_next = |to: u64| (1u64 << 43) | (to << 12) | (1 << 8) | (1 << 5) | 7;
    m.load_prom(
        &[
            call_xct_next(3),
            src_spc | a_dest(0o100) | setm | alu,
            alu,
            src_spc | a_dest(0o101) | setm | alu,
            alu,
            alu,
        ]
        .map(Insn::new),
    );
    let mut e = Rtl::new(m);
    e.boot();
    // The trap, the nopped word behind it, the call, its delay slot, the
    // target, and the write-back microcycle behind that.
    for _ in 0..7 {
        e.step().unwrap();
    }
    let ptr = 1u32 << 24;
    assert_eq!(e.machine().spcptr, 1, "the pointer moved on the call's edge");
    assert_eq!(e.machine().spc[1], 2, "the return address landed a microcycle later");
    assert_eq!(e.machine().amem[0o100], ptr | 0o12345, "the delay slot read the stale slot");
    assert_eq!(e.machine().amem[0o101], ptr | 2, "the target read the pushed word");
}
