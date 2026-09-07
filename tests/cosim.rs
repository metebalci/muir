// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The engines against each other, microinstruction by microinstruction.
//!
//! `micro` models the architecturally visible pipeline; `rtl` computes the
//! whole datapath on the machine's own clock; `chip` resolves the 1243 parts
//! of `data/CADR.netlist`.  Three readings of the same drawings by three
//! different roads, so where two of them agree about something neither was
//! told, that is evidence.
//!
//! The weight is not equal.  `micro` and `rtl` share the ALU and the jump
//! conditions (`crate::ttl`), so those two agreeing about an ALU result says
//! less than either agreeing with `chip`, which computes it from the 74S181s
//! themselves.  `tests/chip.rs` is where that comparison lives; this file is
//! `micro` against `rtl`, which is what catches a pipeline modelled wrong.
//!
//! Only executed cycles are compared.  A cycle the pipeline inhibits runs on
//! the board and retires nothing, so counting it would make the two engines
//! disagree about a number neither is wrong about.

use std::path::{Path, PathBuf};

use muir::engine::Engine;
use muir::isa::Insn;
use muir::machine::Machine;
use muir::micro::Micro;
use muir::rtl::Rtl;

mod support;
use support::machine_with_pack;

/// The System 100 pack, which these comparisons need for a reason worth
/// stating: they run past `PROM-DISABLE`, so the disk must have loaded
/// microcode 323 by then. Without a pack both engines would spin in
/// `DISK-RECALIBRATE` in step with each other and agree about nothing.
fn pack() -> Option<PathBuf> {
    support::pack_100()
}

/// A machine with the System 100 pack, `pack`, on unit 0.
///
/// Both engines must have the same pack on the same unit, or they diverge on
/// the first thing the boot PROM reads off it rather than on anything either
/// computes.
fn booted<E>(make: impl Fn(Machine) -> E, boot: impl Fn(&mut E), pack: &Path) -> E {
    let mut e = make(machine_with_pack(pack));
    boot(&mut e);
    e
}

/// **The strongest thing here.** After the same number of executed
/// instructions, both engines must hold the same memories.  Agreeing about
/// which instruction runs next only shows control flow agreeing; this shows
/// the data does too, and control flow reaches data only where something
/// branches on it.
///
/// It runs to just short of `PROM-DISABLE`, so what it compares is the
/// machine the boot PROM is about to hand over --- every word of the band
/// read off the pack a block at a time and written by `WRITE-I-MEM`,
/// `WRITE-DISPATCH-RAM` and the A-memory loop. Two engines agreeing on 12,449
/// control store words that came through the disk controller is the strongest
/// thing the disk work has to stand on.
///
/// **`M-GARBAGE` is what stops the count being pushed further.** It is where
/// the microcode throws results away, so it holds whatever the last
/// instruction discarded --- and the two engines are one microcycle apart in
/// when they store it. Every other memory agrees at any count; that one
/// agrees only where the offset happens to line up.
#[test]
fn engines_agree_on_memory() {
    let Some(p) = pack() else { return };
    // Just short of `PROM-DISABLE` at 1,262,276, by which point everything
    // the boot PROM loads off the pack is in place.
    const N: usize = 1_262_000;

    let mut a = booted(Micro::new, Micro::boot, &p);
    let mut b = booted(Rtl::new, Rtl::boot, &p);
    let mut n = 0;
    while n < N {
        a.step().expect("micro halted early");
        if a.executed().is_some() {
            n += 1;
        }
    }
    let mut n = 0;
    while n < N {
        b.step().expect("rtl halted early");
        if b.executed().is_some() {
            n += 1;
        }
    }

    let (x, y) = (a.machine(), b.machine());
    assert!(x.imem == y.imem, "control store");
    assert_eq!(x.mmem, y.mmem, "M memory");
    assert_eq!(x.amem, y.amem, "A memory");
    assert_eq!(x.pdl, y.pdl, "PDL buffer");
    // The live stack must agree.  One dead slot above the pointer does not.
    // `WRITE-I-MEM` is a JUMP with both `IR<9>` and `IR<8>` set, so it pushes
    // the return address and the `POPJ` in the cycle CONTRL 3D26's `IWRITED`
    // drives pops it straight back.  The slot above the pointer is left
    // holding that dead return address, and which microcycle each engine
    // writes it in decides what is in it.  Same control flow, one stale slot.
    assert_eq!(x.spcptr, y.spcptr, "microcode stack pointer");
    assert_eq!(x.spc[x.spcptr as usize], y.spc[y.spcptr as usize], "top of microcode stack");
    assert_eq!(x.dmem, y.dmem, "dispatch memory");
    assert_eq!(x.l1_map, y.l1_map, "level-1 map");
    assert_eq!(x.l2_map, y.l2_map, "level-2 map");
    assert_eq!(x.q, y.q, "Q register");
    assert_eq!(x.vma, y.vma, "VMA");
    assert_eq!(x.md, y.md, "MD");
    assert!(x.main == y.main, "main memory");
}

/// **`micro`'s clock is the machine's periods.** Each of its microcycles is
/// as long as the speed bits and `ILONG` make it, 220 ns through the boot
/// and 145 after microcode 323 writes the mode register, and nothing more:
/// it has no bus to wait on. `rtl`'s clock is the same periods plus its
/// waits and hangs, which it counts, so at every executed instruction the
/// two agree once `rtl`'s stalls are taken off. Held from the button to
/// well past the mode register write.
#[test]
fn micro_keeps_the_machines_periods() {
    let Some(p) = pack() else { return };
    const N: usize = 1_300_000;
    let mut a = booted(Micro::new, Micro::boot, &p);
    // The periods alone: off with the mean wait `micro` charges a memory
    // cycle, which `micro_charges_the_mean_memory_wait` checks by itself.
    a.memory_cycle_ns = 0;
    let mut b = booted(Rtl::new, Rtl::boot, &p);
    // From the first executed instruction: the two start up differently,
    // `rtl` charging one more cycle before it, and that is the button, not
    // the clock.
    let (mut a0, mut b0) = (0, 0);
    let mut n = 0;
    let mut speeds = 0;
    while n < N {
        a.step().expect("micro halted early");
        if a.executed().is_none() {
            continue;
        }
        n += 1;
        loop {
            b.step().expect("rtl halted early");
            if b.executed().is_some() {
                break;
            }
        }
        if n == 1 {
            (a0, b0) = (a.machine().ns, b.ns() - b.stalled_ns());
        }
        let (x, y) = (a.machine().ns - a0, b.ns() - b.stalled_ns() - b0);
        assert_eq!(
            x,
            y,
            "instruction {n} at micro PC {:o}, rtl PC {:o}: micro's clock {x} ns, rtl's less its stalls {y}",
            a.pc(),
            b.pc()
        );
        if a.machine().mode.speed1 {
            speeds += 1;
        }
    }
    assert!(speeds > 0, "the window never saw the speed change");
    eprintln!(
        "{N} instructions, clocks agree: micro {} ns, rtl {} ns of which {} stalled over {} memory cycles, {} ns a cycle",
        a.machine().ns,
        b.ns(),
        b.stalled_ns(),
        b.bus_cycles(),
        b.stalled_ns() / b.bus_cycles().max(1)
    );
    // And the band's mean, from `rtl` alone: the number `micro` charges a
    // memory cycle is the band's, not the boot's.
    let (s0, c0, n0) = (b.stalled_ns(), b.bus_cycles(), n);
    while n < 15_000_000 {
        b.step().expect("rtl halted in the band");
        if b.executed().is_some() {
            n += 1;
        }
    }
    let (s, c) = (b.stalled_ns() - s0, b.bus_cycles() - c0);
    eprintln!(
        "band, instructions {n0} to {n}: {s} ns stalled over {c} memory cycles, {} ns a cycle",
        s / c.max(1)
    );
}

/// **`micro` charges the measured mean wait per memory cycle and nothing
/// else.** Two `micro` runs to the same instruction, one with the charge
/// off, differ by exactly the cycles started times `MEMORY_ACCESS_NS`.
#[test]
fn micro_charges_the_mean_memory_wait() {
    let Some(p) = pack() else { return };
    // Past the boot PROM's self-tests, which touch no memory for the first
    // 536,000 microcycles, and into its disk reads.
    const N: usize = 1_000_000;
    let run = |charge: u64| {
        let mut e = booted(Micro::new, Micro::boot, &p);
        e.memory_cycle_ns = charge;
        let mut n = 0;
        while n < N {
            e.step().expect("micro halted early");
            if e.executed().is_some() {
                n += 1;
            }
        }
        (e.machine().ns, e.memory_cycles())
    };
    let (plain, cycles) = run(0);
    let (charged, cycles2) = run(muir::micro::MEMORY_ACCESS_NS);
    assert_eq!(cycles, cycles2, "the charge changes no cycle");
    assert!(cycles > 0);
    assert_eq!(
        charged - plain,
        cycles * muir::micro::MEMORY_ACCESS_NS,
        "the charge is the mean per cycle, {cycles} cycles"
    );
}

/// What the location counter does to the shifter in the two byte modes,
/// on `micro` and `rtl` alike, against what CC's diagnostics expect.
///
/// `CC-TEST-LC-DP` in `sys/cc/diags.lisp` writes `LC` and reads the
/// shifter's output of an all-ones word with `IR<11:10>` = 3: "Select byte
/// (initially rightmost, LC=current+1)", `LC` = 1 selecting byte 0 and
/// `LC` = 4 byte 3, and in word mode `LC` = 2 the low halfword and 4 the
/// high one. That is the drawings' `SH4` and `SH3` on page LC, which `rtl`
/// computes gate by gate. `micro` had a formula that inverts both bits in
/// byte mode, and nothing saw it because the boot never leaves word mode:
/// `CC-TEST-LC-DP` is the first thing that asks.
#[test]
fn the_lc_shift_selects_the_byte_the_diagnostics_expect() {
    fn shifter_output<E: Engine>(
        make: impl Fn(Machine) -> E,
        boot: impl Fn(&mut E),
        byte_mode: bool,
        lc: u32,
        width: u32,
    ) -> u32 {
        let mut m = Machine::new();
        // The constants the program takes from A memory, and the all-ones
        // word it shifts, in M memory 2.
        m.amem[10] = (byte_mode as u32) << 29;
        m.amem[11] = lc;
        m.amem[12] = 0;
        m.mmem[2] = !0;
        m.amem[2] = !0;
        // `tests/chip.rs`'s encoding: `IR<13:12>` = 1 puts the ALU on OB,
        // `IR<8:3>` = 5 is SETA, the A address is `IR<41:32>` and a
        // functional destination is `IR<23:19>` with `IR<25>` clear.
        let alu = 1u64 << 12;
        let seta = 5u64 << 3;
        let a_src = |a: u64| a << 32;
        let int_control = (0o2u64 << 19) | (0o37 << 14);
        let lc_dest = (0o1u64 << 19) | (0o37 << 14);
        // A BYTE-class word with `IR<11:10>` = 3, MR and SR (`IR<13:12>`),
        // `width` bits from position 0, M memory 2 against A memory 12,
        // into A memory 200.
        let byte_op = (3u64 << 43)
            | (1 << 25)
            | (0o200 << 14)
            | a_src(12)
            | (2 << 26)
            | (3 << 12)
            | (3 << 10)
            | ((width as u64 - 1) << 5);
        m.load_prom(
            &[a_src(10) | int_control | seta | alu, a_src(11) | lc_dest | seta | alu, byte_op]
                .map(Insn::new),
        );
        let mut e = make(m);
        boot(&mut e);
        // The trap and its delay slot, the three words, and the write-back
        // microcycle behind the last of them, on either engine.
        for _ in 0..8 {
            e.step().unwrap();
        }
        e.machine().amem[0o200]
    }
    let cases = [
        (true, 1, 8, 0xffu32),
        (true, 2, 8, 0xff00),
        (true, 3, 8, 0xff_0000),
        (true, 4, 8, 0xff00_0000),
        (false, 2, 16, 0xffff),
        (false, 4, 16, 0xffff_0000),
    ];
    for (byte_mode, lc, width, want) in cases {
        let mode = if byte_mode { "byte" } else { "word" };
        let r = shifter_output(Rtl::new, Rtl::boot, byte_mode, lc, width);
        assert_eq!(r, want, "rtl: LC {lc} in {mode} mode, {width} bits");
        let u = shifter_output(Micro::new, Micro::boot, byte_mode, lc, width);
        assert_eq!(u, want, "micro: LC {lc} in {mode} mode, {width} bits");
    }
}

// --- micro against rtl, one construct at a time ------------------------------

use muir::isa::asm::{
    ALU, ALWAYS, BYTE, DISPATCH, JUMP, MD, P, POPJ, R, SETM, SETO, SETZ, SRC_MD, START_READ,
    a_dest, a_src, d_addr, filler, m_src, src, target, width,
};

/// The functional destination `LOCATION-COUNTER`, 0o1, with the harmless
/// M word every functional destination here also writes.
const LC: u64 = (0o1 << 19) | (0o37 << 14);

/// Both engines on the same boot PROM --- `prom` at word 0, fillers behind
/// it --- with `set` having prepared the machine, run `steps` microcycles
/// each.
fn both(prom: &[Insn], set: &dyn Fn(&mut Machine), steps: usize) -> (Micro, Rtl) {
    let make = || {
        let mut m = Machine::new();
        let mut words = vec![filler(); 512];
        words[..prom.len()].copy_from_slice(prom);
        m.load_prom(&words);
        set(&mut m);
        m
    };
    let mut e = Micro::new(make());
    e.boot();
    for _ in 0..steps {
        e.step().unwrap();
    }
    let mut r = Rtl::new(make());
    r.boot();
    for _ in 0..steps {
        r.step().unwrap();
    }
    (e, r)
}

/// **A control-store write pushes and pops, leaving the pointer alone and
/// a word above it.**
///
/// `P` and `R` together on a JUMP write the control store rather than
/// jumping, and the board does not special-case the push. Page CONTRL: the
/// 74S64 at 3E26 makes `-SPUSH` from four AND groups, the first
/// `IRJUMP AND -IR6 AND IR8 AND JCOND`, and none of the four has `IWRITE`
/// in it --- `IWRITE` is decoded alone at the 74S11 3E29 as
/// `IRJUMP AND IR8 AND IR9`. A write is a jump-always with `P`, so the
/// group is satisfied and the machine pushes; on the next cycle `IWRITED`
/// off the 74S175 at 3D26 makes `POPJ` through the open-collector 74S08 at
/// 3D21, and it pops.
///
/// So the stack pointer ends where it began with the pushed word still in
/// the slot above it. That word is what a console reading the stack sees,
/// and it is the whole observable difference. Read off `data/CADR.netlist`
/// and confirmed against MIT's own wire list `cadrwd/cadr4.wlr`.
///
/// `micro` used to neither push nor pop here, which left a different word
/// above the pointer from the one the board leaves.
#[test]
fn a_control_store_write_pushes_and_pops_on_both() {
    // Write word 0o250 of the control store, then read the stack back.
    let prom =
        [filler(), Insn::new(JUMP | ALWAYS | P | R | target(0o250)), filler(), filler(), filler()];
    let (e, r) = both(&prom, &|_| {}, 20);
    let (em, rm) = (e.machine(), r.machine());
    assert_eq!(em.spcptr, rm.spcptr, "the stack pointer ends in the same place");
    let above = |p: u8| (p as usize + 1) & 0o37;
    assert_eq!(
        em.spc[above(em.spcptr)],
        rm.spc[above(rm.spcptr)],
        "and both leave the same word in the slot above it"
    );
    assert_eq!(em.imem[0o250].raw(), rm.imem[0o250].raw(), "both wrote the same word");
}

/// **A read the map refuses leaves MD alone.** On the board a cycle starts
/// only under `VMAOK`: `MBUSY` is set from `MEMSTART AND VMAOK` on page
/// VCTL1, so nothing is requested and `MD` keeps its word, and `-VMAOK` in
/// the flags is how the microcode learns of the fault. With no map entry
/// every page is refused, so the read below fetches nothing on either engine.
#[test]
fn a_refused_read_leaves_md_alone() {
    let prom = [
        Insn::new(ALU | SETO | MD),
        Insn::new(ALU | SETZ | START_READ),
        filler(),
        filler(),
        filler(),
        Insn::new(ALU | SETM | SRC_MD | a_dest(0o200)),
    ];
    let (e, r) = both(&prom, &|_| {}, 40);
    assert_eq!(r.machine().amem[0o200], !0, "rtl: MD kept its word");
    assert_eq!(e.machine().amem[0o200], !0, "micro: MD kept its word");
}

/// **An instruction fetch is a memory cycle.** `IFETCH` is one of `MEMOP`'s
/// terms on page VCTL1, so the fetch the location counter asks for goes
/// through the map like any read: the latch at VMEMDR 1D14 takes the page's
/// map word, and the `-PFR` and `-PFW` bits of `MAP(MD)` --- functional
/// source 0o11, off that latch through the 74S240 at VMEMDR 1A01 --- show
/// the fetched page's permissions afterwards. Here a permitted read of page
/// 1 is followed by a fetch from page 2, which no map entry permits, and
/// the fault bit is up on both engines.
#[test]
fn an_instruction_fetch_goes_through_the_map() {
    let prom = [
        Insn::new(ALU | SETM | m_src(1) | LC),
        Insn::new(ALU | SETM | m_src(2) | START_READ),
        filler(),
        filler(),
        filler(),
        Insn::new(DISPATCH | d_addr(7) | (1 << 24)),
        filler(),
        filler(),
        Insn::new(ALU | SETM | src(0o11) | a_dest(0o200)),
    ];
    let set = |m: &mut Machine| {
        // LC counts bytes: the first word of page 2, and page 1's first word
        // for the read. Level 1 is all zeros, so page 1 is level-2 entry 1.
        m.mmem[1] = (2 << 8) << 2;
        m.mmem[2] = 1 << 8;
        m.l2_map[1] = (1 << 23) | (1 << 22) | 0o100;
        // The dispatch lands on word 8.
        m.dmem[7] = 8;
    };
    let (e, r) = both(&prom, &set, 40);
    let map = r.machine().amem[0o200];
    assert_ne!(map & (1 << 30), 0, "rtl: -PFR up, the fetched page refused");
    assert_eq!(e.machine().amem[0o200], map, "micro reads MAP(MD) as rtl does");
}

/// **A dispatch entry with R, P and N together falls through.** Page
/// CONTRL: `DFALL = DR AND DP` takes the return-and-push entry out of both
/// `PCS` selects, so the next address is `PC + 1`; nothing is pushed or
/// popped, and `N` still inhibits the instruction behind. That holds with
/// `IR<25>`, which only chooses `LPC` as the address a push would take.
#[test]
fn a_dispatch_to_a_return_and_push_entry_falls_through() {
    let prom = [
        filler(),
        Insn::new(DISPATCH | d_addr(7) | (1 << 25)),
        Insn::new(ALU | SETO | a_dest(0o201)),
        Insn::new(ALU | SETO | a_dest(0o202)),
        Insn::new(ALU | SETO | a_dest(0o203)),
    ];
    let set = |m: &mut Machine| m.dmem[7] = (1 << 16) | (1 << 15) | (1 << 14) | 0o100;
    let (e, r) = both(&prom, &set, 40);
    let got = |m: &Machine| [m.amem[0o201], m.amem[0o202], m.amem[0o203]];
    assert_eq!(got(r.machine()), [0, !0, !0], "rtl: the slot inhibited, then straight on");
    assert_eq!(got(e.machine()), [0, !0, !0], "micro likewise");
}

/// **The POPJ bit wins over a taken jump.** Page CONTRL's next-address
/// select: `PCS0` is `NOT(POPJ OR ...)` and `PCS1` is low for a taken jump,
/// so with `IR<42>` up a taken JUMP without `R` still takes its next address
/// off the microcode stack and not from `IR<25:12>`.
#[test]
fn the_popj_bit_wins_over_a_taken_jump() {
    let prom = [
        filler(),
        Insn::new(JUMP | target(4) | ALWAYS | POPJ),
        filler(),
        filler(),
        Insn::new(ALU | SETO | a_dest(0o201)),
        filler(),
        Insn::new(ALU | SETO | a_dest(0o202)),
    ];
    let set = |m: &mut Machine| {
        m.spcptr = 1;
        m.spc[1] = 6;
    };
    let (e, r) = both(&prom, &set, 40);
    let got = |m: &Machine| (m.amem[0o201], m.amem[0o202]);
    assert_eq!(got(r.machine()), (0, !0), "rtl pops to word 6");
    assert_eq!(got(e.machine()), (0, !0), "micro pops to word 6");
}

/// **The functional sources MIT leaves unassigned read as all ones, and
/// `IR<30>` is not in the decode.** Page SOURCE decodes `IR<31>`, `IR<29>`
/// and `IR<28:26>` through two 74S138s, and the one for sources 10 to 17
/// has its outputs for 0o15, 0o16 and 0o17 unconnected: no part drives the
/// M bus for them, and an undriven TTL bus reads high. `IR<30>` reaches only
/// page PDLCTL, where it reads the PDL buffer by pointer rather than index,
/// so source 0o26 is source 6, the OPC. Microcode 323 reads 0o15 at 0o20535
/// and 0o26 at 0o20705.
#[test]
fn an_unassigned_functional_source_reads_all_ones() {
    // The OPC runs eight microcycles behind the PC on the board, so the
    // reads come after ten fillers, once it has moved off its power-on zero.
    let mut prom = vec![filler(); 10];
    prom.extend([
        Insn::new(ALU | SETM | src(0o15) | a_dest(0o201)),
        Insn::new(ALU | SETM | src(0o16) | a_dest(0o202)),
        Insn::new(ALU | SETM | src(0o17) | a_dest(0o203)),
        Insn::new(ALU | SETM | src(0o26) | a_dest(0o204)),
        Insn::new(ALU | SETM | src(0o6) | a_dest(0o205)),
    ]);
    let (e, r) = both(&prom, &|_| {}, 40);
    let ones = |m: &Machine| [m.amem[0o201], m.amem[0o202], m.amem[0o203]];
    assert_eq!(ones(r.machine()), [!0; 3], "rtl");
    assert_eq!(ones(e.machine()), [!0; 3], "micro");
    // The OPC moves with every microcycle, so the two reads of it are one
    // apart on each engine; the engines differ in how far behind the PC the
    // OPC runs, which is not what is checked here.
    for (name, m) in [("rtl", r.machine()), ("micro", e.machine())] {
        assert_ne!(m.amem[0o204], !0, "{name}: source 26 is driven");
        assert_eq!(m.amem[0o205], m.amem[0o204] + 1, "{name}: 26 is the OPC, as 6 is");
    }
}

/// **A BYTE with function 0 deposits without rotating.** Page SMCTL: `MR`
/// is `IR<13>` and `SR` is `IR<12>` on a BYTE, so with both clear the
/// rotate is zero and the mask starts at bit 0; and the output bus carries
/// the mask network's word on every BYTE, `OSEL` being 0 --- the M source's
/// low `IR<9:5>` + 1 bits laid into the A source.
#[test]
fn a_byte_with_function_zero_deposits_without_rotating() {
    let prom = [Insn::new(BYTE | width(4) | m_src(1) | a_src(2) | a_dest(0o201))];
    let set = |m: &mut Machine| {
        m.mmem[1] = 0xdead_beef;
        m.amem[2] = 0x1111_1111;
    };
    let (e, r) = both(&prom, &set, 40);
    assert_eq!(r.machine().amem[0o201], 0x1111_111f, "rtl");
    assert_eq!(e.machine().amem[0o201], 0x1111_111f, "micro");
}

/// **The map write lands on the cycle after the store, in both engines.**
///
/// `mit/cadr/ir.bits`, on destination `23`, which MIT writes `VMA, MAP(MD)VMA`:
/// "The write actually occurs on the cycle following the store into
/// destination WRITE-MAP, and the VMA must not be disturbed during this cycle
/// for proper operation."
///
/// `rtl` has always had the delay --- `WMAPD` is registered at VCTL2 1C15 and
/// the write is one of `Rtl::write_phase`'s, a microcycle behind the level
/// that asked for it. `micro` wrote at the store, so the entry appeared to
/// the microcycle in between, which on the board still reads the old map.
///
/// Two instructions in the boot PROM, so this needs no pack: one to put the
/// virtual address in `MD`, one to store the map word through `VMA`. The
/// store is the microcycle `VMA` becomes the stored word; the write is the
/// microcycle the level-2 entry appears. They must be one apart.
#[test]
fn the_map_write_lands_the_cycle_after_the_store() {
    // ALU class, `SETA`, A source, and the two functional destinations.
    // `ir.bits`: the ALU function table is "shift left 3", so `SETA` is 5 at
    // `IR<8:3>`; `IR<13:12>` = 1 takes the ALU output; the functional
    // destination is `IR<23:19>` with `IR<25>` clear, and `IR<18:14>` is the
    // M address a functional write also lands in.
    let alu = 1u64 << 12;
    let seta = 5u64 << 3;
    let a_src = |a: u64| a << 32;
    let functional = |d: u64| (d << 19) | (0o37 << 14);

    // Virtual address `0o2000000`: `MD<23:13>` is `0o100`, the level-1 index,
    // and `MD<12:8>` is 0, so with `1` in the level-1 entry the level-2 index
    // is `1 << 5`. The word stored has `VMA<25>` set, which is the level-2
    // write enable, and `VMA<26>` clear, so the level-1 entry is left alone.
    const VIRTUAL: u32 = 0o2000000;
    const MAP_WORD: u32 = (1 << 25) | 0o12345;
    const L1: usize = 0o100;
    const L2: usize = 1 << 5;

    let machine = || {
        let mut m = Machine::new();
        m.l1_map[L1] = 1;
        m.amem[0o100] = MAP_WORD;
        m.amem[0o101] = VIRTUAL;
        m.load_prom(&[
            // ((MD) SETA A-MEM 101)
            Insn::new(alu | seta | a_src(0o101) | functional(0o30)),
            // ((VMA-WRITE-MAP) SETA A-MEM 100), MIT's `VMA, MAP(MD)VMA`
            Insn::new(alu | seta | a_src(0o100) | functional(0o23)),
            Insn::new(0),
            Insn::new(0),
            Insn::new(0),
            Insn::new(0),
        ]);
        m
    };

    /// The executed microcycle the store lands in, and the one the map entry
    /// appears in.
    fn watch(e: &mut dyn Engine) -> (usize, usize) {
        let (mut stored, mut written) = (None, None);
        let mut n = 0;
        while written.is_none() {
            assert!(n < 64, "the map entry never appeared");
            e.step().unwrap();
            n += 1;
            if stored.is_none() && e.machine().vma == MAP_WORD {
                stored = Some(n);
            }
            if e.machine().l2_map[L2] == (MAP_WORD & 0o77777777) {
                written = Some(n);
            }
        }
        (stored.expect("VMA never took the map word"), written.unwrap())
    }

    let mut a = Micro::new(machine());
    a.boot();
    let mut b = Rtl::new(machine());
    b.boot();
    let (a_stored, a_written) = watch(&mut a);
    let (b_stored, b_written) = watch(&mut b);

    assert_eq!(a_written, a_stored + 1, "micro: one microcycle after the store");
    assert_eq!(b_written, b_stored + 1, "rtl: one microcycle after the store");
    assert_eq!(a.machine().l2_map, b.machine().l2_map, "and the same entry");
    assert_eq!(a.machine().l1_map, b.machine().l1_map, "with the level-1 map untouched");
    assert_eq!(a.machine().l1_map[L1], 1, "VMA<26> was clear");
}
