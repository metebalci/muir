// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The instruction word's fields: where a second source settles a width,
//! and where reading one out of a running machine can be read the wrong
//! way round.

use std::collections::BTreeSet;

use muir::engine::Engine;
use muir::isa::{Insn, asm};
use muir::machine::Machine;
use muir::mcr;
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

/// `DISK-RECALIBRATE-WAIT`, `sys/ucadr/uc-disk.lisp:262` and `0o25332` in
/// microcode 323.
const WAIT: usize = 0o25332;

/// The band's wait loop in the control store, with the two instructions
/// that read the status register standing in as fillers: `MD` is planted
/// rather than fetched, so the loop can be run without a controller under
/// it.
fn wait_loop() -> Machine {
    let u = mcr::parse(mcr::UCADR_323).expect("mit/sys/ubin/ucadr.mcr");
    let mut m = Machine::new();
    let mut prom = vec![asm::filler(); 512];
    // Out of the PROM and into the loop, by the same kind of jump the
    // loop's own are: taken, with `N`.
    prom[1] = Insn::new(asm::JUMP | asm::target(WAIT as u64) | asm::N | asm::ALWAYS);
    m.load_prom(&prom);
    m.imem[WAIT] = asm::filler();
    m.imem[WAIT + 1] = asm::filler();
    for a in WAIT + 2..=WAIT + 4 {
        m.imem[a] = u.imem[a];
    }
    m
}

/// Runs `e` with `md` standing in the memory data register throughout, and
/// returns the addresses its PC took.
fn pcs_with_md(e: &mut impl Engine, md: u32) -> BTreeSet<u16> {
    let mut seen = BTreeSet::new();
    e.boot();
    for n in 0..200 {
        e.machine_mut().md = md;
        e.step().expect("stopped in the wait loop");
        // Past the run-in from the PROM's first two words.
        if n >= 4 {
            seen.insert(e.pc());
        }
    }
    seen
}

/// **`IR<7>`, `N`, puts an address in the PC that the machine never
/// executes.** The instruction behind a taken jump is fetched and nopped,
/// and the PC between two microcycles is the address after the one waiting
/// to execute --- so a loop's set of PCs runs one past its last
/// instruction, and an address caught there is not an address that ran.
///
/// The band's own wait-for-the-controller loop is the case that makes this
/// worth pinning. `sys/ucadr/uc-disk.lisp:262`, at `0o25332`:
///
/// ```text
/// DISK-RECALIBRATE-WAIT                                           ; 25332
///   ((VMA-START-READ) A-DISK-REGS-BASE)
///   (CHECK-PAGE-READ-NO-INTERRUPT)                                ; 25333
///   (JUMP-IF-BIT-CLEAR (BYTE-FIELD 1 0) MD DISK-RECALIBRATE-WAIT) ; 25334
///   (JUMP-IF-BIT-SET   (BYTE-FIELD 1 8) MD DISK-RECALIBRATE-WAIT) ; 25335
///   (POPJ)                                                        ; 25336
/// ```
///
/// The words are read out of `mit/sys/ubin/ucadr.mcr` rather than
/// assembled here, so this is MIT's own loop and not a copy of it: both
/// jumps carry `N`, and both target `25332`.
///
/// **So `25335` in the PC does not mean the bit-0 test passed.** A machine
/// spinning on bit 0 --- a controller that never reports not-active ---
/// shows exactly `25332` to `25335`, the last of them the nopped slot
/// behind the taken jump. It is `25336` that tells the two spins apart:
/// only the loop that got past bit 0 and stuck on bit 8 reaches it. Issue
/// 88 read a sighting of `25335` as evidence of the second, and it is
/// evidence of neither.
#[test]
fn the_inhibited_slot_behind_a_jump_shows_in_the_pc() {
    let u = mcr::parse(mcr::UCADR_323).unwrap();
    let (bit0, bit8) = (u.imem[WAIT + 2].jump(), u.imem[WAIT + 3].jump());
    assert!(bit0.n && bit8.n, "both jumps inhibit the instruction behind them");
    assert_eq!((bit0.target, bit8.target), (WAIT as u16, WAIT as u16), "both go back to the top");
    assert!(bit0.invert && bit0.rotate == 0, "the first tests bit 0 and jumps if clear");
    assert!(!bit8.invert && bit8.rotate == 32 - 8, "the second tests bit 8 and jumps if set");

    // Bit 0 clear: the controller is busy, the loop never reaches the
    // bit-8 test, and `25335` is in the PC all the same.
    let spinning_on_bit_0: BTreeSet<u16> = (WAIT as u16..=WAIT as u16 + 3).collect();
    // Bit 0 set with bit 8 set: round the whole loop, one address further.
    let spinning_on_bit_8: BTreeSet<u16> = (WAIT as u16..=WAIT as u16 + 4).collect();

    for (md, want, what) in [
        (0, &spinning_on_bit_0, "busy, on cylinder"),
        (1 << 8, &spinning_on_bit_0, "busy, off cylinder"),
        ((1 << 8) | 1, &spinning_on_bit_8, "not busy, off cylinder"),
    ] {
        assert_eq!(&pcs_with_md(&mut Micro::new(wait_loop()), md), want, "micro, {what}");
        assert_eq!(&pcs_with_md(&mut Rtl::new(wait_loop()), md), want, "rtl, {what}");
    }

    // And the loop does leave when both tests pass, so the two above are
    // the loop waiting and not the loop broken.
    let out = pcs_with_md(&mut Micro::new(wait_loop()), 1);
    assert!(out.contains(&(WAIT as u16 + 5)), "not busy, on cylinder: past the POPJ, {out:?}");
}
