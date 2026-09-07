// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `HALT-CONS`, misc function 1, and what the CADR does with it.
//!
//! MIT's assembler defines it as `1_10.` in `sys/sys/cadsym.lisp`, beside
//! `WRITE-DISPATCH-RAM` at `2_10.` and `INSTRUCTION-STREAM` at `3_10.`, so
//! `IR<11:10>` is the misc function on every instruction class, and
//! microcode 323 writes it at `ZERO`, `ILLOP` and `%HALT`. On the board,
//! page SOURCE decodes the field through the 74S139 at 3D05 into
//! `-FUNCT0..3`; `-FUNCT1` leaves the processor board on connector pin
//! 3CJ1-19 and is `-HALT` on the ICMEM board at 1CJ2-21 (`cadrwd/cadr4.wlr`
//! and `icmem3.wlr`), the D input of the 74S374 at OLORD2 1A05 whose Q is
//! `-HALTED`, one of the inputs of the 74S133 at 1A02 that makes `ERR`.
//! `ERR` shows in `FLAG-1` bit 10, and under `ERRSTOP` in the mode register
//! it drops `MACHRUN`: the machine halts after the instruction, and runs on
//! without `ERRSTOP`. `micro` and `rtl` are held to that here.

use muir::engine::Engine;
use muir::isa::Insn;
use muir::isa::asm::{JUMP, filler};
use muir::machine::Machine;
use muir::micro::Micro;
use muir::rtl::Rtl;
use muir::spy;

/// `HALT-CONS`, `1_10.`.
const HALT_CONS: u64 = 1 << 10;
/// `ERR` in `FLAG-1`, bit 10.
const ERR: u16 = 1 << 10;

/// A straight-line program whose PC climbs one a microcycle, with `insn`
/// carrying `HALT-CONS` at address 5.
fn program(insn: Insn) -> Machine {
    let mut m = Machine::new();
    let mut prom = vec![filler(); 512];
    prom[5] = Insn::new(insn.raw() | HALT_CONS);
    m.load_prom(&prom);
    m
}

/// Boots `e`, sets `ERRSTOP` as asked --- after the boot, which resets the
/// console's registers --- and runs forty microcycles.
fn run(e: &mut impl Engine, errstop: bool) {
    e.machine_mut().mode.errstop = errstop;
    for _ in 0..40 {
        e.step().expect("no halt this engine raises");
    }
}

/// **Under `ERRSTOP`, `HALT-CONS` halts the machine after the
/// instruction, on either engine and on either class.**
#[test]
fn halt_cons_halts_under_errstop() {
    for (what, insn) in
        [("an ALU instruction", filler()), ("a JUMP to itself", Insn::new(JUMP | 5 << 12))]
    {
        let mut r = Rtl::new(program(insn));
        r.boot();
        run(&mut r, true);
        assert!(r.pc() <= 7, "rtl, {what}: halted, PC {:o}", r.pc());
        assert_ne!(r.spy_read(spy::FLAG_1) & ERR, 0, "rtl, {what}: ERR up");
        assert!(r.halted_ns() > 0, "rtl, {what}: the cpu clock held off");

        let mut e = Micro::new(program(insn));
        e.boot();
        run(&mut e, true);
        assert!(e.pc() <= 7, "micro, {what}: halted, PC {:o}", e.pc());
        assert_ne!(e.spy_read(spy::FLAG_1) & ERR, 0, "micro, {what}: ERR up");
    }
}

/// **Without `ERRSTOP`, `HALT-CONS` does nothing**: `HALTED` sets, `ERR`
/// is up for the microcycle, and the machine runs on.
#[test]
fn halt_cons_runs_on_without_errstop() {
    let mut r = Rtl::new(program(filler()));
    r.boot();
    run(&mut r, false);
    assert!(r.pc() > 20, "rtl ran on, PC {:o}", r.pc());
    assert_eq!(r.spy_read(spy::FLAG_1) & ERR, 0, "and ERR is down again");

    let mut e = Micro::new(program(filler()));
    e.boot();
    run(&mut e, false);
    assert!(e.pc() > 20, "micro ran on, PC {:o}", e.pc());
    assert_eq!(e.spy_read(spy::FLAG_1) & ERR, 0, "and ERR is down again");
}

/// **A machine stopped by `HALT-CONS` under `ERRSTOP` stays stopped**, and
/// says so in `FLAG-1` where a console reads it: `SRUN` still set, so this
/// is not the halt a cleared `RUN` is, with `ERR` up and no `WAIT`.
///
/// This is what `(si:%halt)` does in System 100, and it is why the run loop
/// has to watch `FLAG-1` rather than step on: [`Engine::step`] goes on
/// returning `Ok` for as long as it is called, advancing the master clock
/// and running no microcycle, so nothing about the call says the machine
/// has stopped.
fn stays_halted<E: Engine>(what: &str, e: &mut E) {
    e.boot();
    run(e, true);
    let pc = e.pc();
    let f = spy::Flag1::of(e.spy_read(spy::FLAG_1));
    assert!(f.srun, "{what}: RUN is still set, so a cleared RUN is not what stopped it");
    assert!(f.err, "{what}: ERR is up");
    assert!(!f.wait, "{what}: and it is not a wait on the bus, which it would come out of");
    for _ in 0..10_000 {
        e.step().expect("no halt this engine raises");
    }
    assert_eq!(e.pc(), pc, "{what}: stepped 10,000 times more and never moved");
}

#[test]
fn an_errstop_halt_stays_halted_and_shows_in_flag_1() {
    stays_halted("rtl", &mut Rtl::new(program(filler())));
    stays_halted("micro", &mut Micro::new(program(filler())));
}

/// **`MACHRUN` is made where the run loop thinks it is**, which is what
/// the `chip` engine's reading of a self-halt rests on.
///
/// `Chip` is not an [`Engine`] and has no spy registers to read, so
/// `time_chip` goes to the nets instead: `SRUN`, `-ERRHALT` and
/// `-STATHALT` on the 9S42 at OLORD1 1A15, leaving out `-WAIT`, which is a
/// stall the machine comes out of by itself. That only works while those
/// are the names and the wiring, so this holds them --- a rename or a
/// rewire fails here rather than quietly making the engine blind to a
/// halt it can no longer see.
#[test]
fn machrun_is_made_from_the_nets_the_chip_engine_reads() {
    let n = muir::netlist::parse(include_str!("../data/CADR.netlist")).unwrap();
    // A quad gate is drawn as more than one group under the one
    // designator, so these are found by the net they carry and then held
    // to where they are, rather than found by position.
    let pin = |p: &muir::netlist::Part, want: u8| {
        p.pins.iter().find(|(x, _)| *x == want).map(|&(_, id)| n.net(id).to_string())
    };
    let carries =
        |p: &muir::netlist::Part, net: &str| p.pins.iter().any(|&(_, id)| n.net(id) == net);

    // `-ERRHALT` is `NAND(ERRSTOP, ERR)`: low exactly when an error is up
    // and `ERRSTOP` says to stop for one.
    let nand = n
        .parts
        .iter()
        .find(|p| p.kind == "74S00" && carries(p, "-ERRHALT"))
        .expect("the NAND that makes -ERRHALT");
    assert_eq!((nand.page.as_str(), nand.reference.as_str()), ("OLORD2", "1C09"));
    assert_eq!(pin(nand, 11).as_deref(), Some("-ERRHALT"), "its output");
    let mut ins = [pin(nand, 12), pin(nand, 13)];
    ins.sort();
    assert_eq!(ins, [Some("ERR".into()), Some("ERRSTOP".into())], "its two inputs");

    // And the 9S42 that makes `MACHRUN` from six.
    let gate = n
        .parts
        .iter()
        .find(|p| p.kind.starts_with("9S42") && pin(p, 7).as_deref() == Some("MACHRUN"))
        .expect("the gate that makes MACHRUN");
    assert_eq!((gate.page.as_str(), gate.reference.as_str()), ("OLORD1", "1A15"));
    let ins: Vec<String> = (1..=6).map(|k| pin(gate, k).expect("an input")).collect();
    assert_eq!(ins, ["SSTEP", "-SSDONE", "SRUN", "-ERRHALT", "-WAIT", "-STATHALT"]);
}
