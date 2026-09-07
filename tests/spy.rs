// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The diagnostic bus's registers, against the board that decodes them.
//!
//! Three sources reach this by three routes and agree on every bit: MIT's
//! `cadr/busint.erface` for the bus, the three 74S138s on page SPY0 of
//! `data/CADR.netlist` for the addresses, and MIT's own comments in
//! `mit/sys/ucadr/promh.text` for what two of the mode register's bits do.

use muir::busint::{Responder, decode, unibus_address};
use muir::machine::{MAIN_WORDS, Machine};
use muir::netlist::{self, Netlist, Part};
use muir::spy::{self, Mode};

const NETLIST: &str = include_str!("../data/CADR.netlist");

fn board() -> Netlist {
    netlist::parse(NETLIST).unwrap()
}

fn part<'a>(n: &'a Netlist, page: &str, reference: &str) -> &'a Part {
    n.parts
        .iter()
        .find(|p| p.page == page && p.reference == reference)
        .unwrap_or_else(|| panic!("no {reference} on page {page}"))
}

fn net_at<'a>(n: &'a Netlist, p: &Part, pin: u8) -> &'a str {
    let (_, id) = p.pins.iter().find(|(x, _)| *x == pin).expect("pin");
    n.net(*id)
}

/// The mode register is `0o766012` because `EADR<3:0>` follows the Unibus
/// address `<4:1>`, and the boot PROM gets there through a map entry of its
/// own making.
///
/// > Writing 44 in Unibus location 766012, which is at virtual address 1005,
/// > will turn on ERROR-STOP-ENABLE and PROM-DISABLE.
///
/// `SET-UP-FOUR-PAGES` maps virtual page 2 to physical page `0o37766`, so
/// virtual `0o1005` is physical `0o17773005`, which is the pair the microcode
/// writes: `((VMA) (A-CONSTANT 17773005)) ;Unibus 766012`.
#[test]
fn the_mode_register_is_at_unibus_766012() {
    assert_eq!(unibus_address(0o17773005), Some(0o766012));
    assert_eq!(spy::register(0o766012), Some(spy::MODE));

    // Below the Unibus there is no Unibus address at all.
    assert_eq!(unibus_address(0o17377777), None, "the top of Xbus I/O");
    assert_eq!(unibus_address(0o17400000), Some(0), "the bottom of the Unibus");

    // The sixteen registers, and the first address past them.
    assert_eq!(spy::register(0o766000), Some(0));
    assert_eq!(spy::register(0o766036), Some(0o17));
    assert_eq!(spy::register(0o766040), None, "the bus interface's own registers");
    assert_eq!(spy::register(0o765776), None);
}

/// The block answers on the Unibus as the bus interface's own, with the
/// interrupt block and the Unibus map behind it (`busint::register`), and
/// everything else on the Unibus still times out.
#[test]
fn the_diagnostic_registers_answer_on_the_unibus() {
    let m = MAIN_WORDS;
    assert_eq!(decode(0o17773005, m), Responder::Interface, "the mode register");
    assert_eq!(decode(0o17773000, m), Responder::Interface);
    assert_eq!(decode(0o17773017, m), Responder::Interface);
    assert_eq!(
        decode(0o17773020, m),
        Responder::Interface,
        "just past the block: the interrupt status"
    );
    assert_eq!(decode(0o17773100, m), Responder::NoUnibus, "past the interface's whole block");
    assert_eq!(decode(0o37000 << 8, m), Responder::NoUnibus, "the bottom of the Unibus");
}

/// The write decoder, read off the 74S138 at SPY0 1F03.
///
/// Its address is `EADR<2:0>` and its enables are `-DBWRITE`, `GND` and
/// `HI1`, so `EADR3` never reaches it: **a write is decoded from three
/// address bits, not four.** The two read decoders at 1F01 and 1F02 are the
/// ones `EADR3` splits, which is how one address can read the PC and write
/// the mode register --- "Read and write at the same address are
/// uncorrelated".
#[test]
fn the_write_decoder_puts_the_mode_register_at_eadr_5() {
    let n = board();
    let p = part(&n, "SPY0", "1F03");
    assert_eq!(p.kind, "74S138");
    assert_eq!(net_at(&n, p, 1), "EADR0");
    assert_eq!(net_at(&n, p, 2), "EADR1");
    assert_eq!(net_at(&n, p, 3), "EADR2");
    assert_eq!(net_at(&n, p, 4), "-DBWRITE", "G2A");
    assert_eq!(net_at(&n, p, 5), "GND", "G2B");
    assert_eq!(net_at(&n, p, 6), "HI1", "G1, and not EADR3");

    // Y0..Y7 on pins 15, 14, 13, 12, 11, 10, 9, 7.
    const WRITES: [(u8, &str); 8] = [
        (15, "-LDDBIRL"),
        (14, "-LDDBIRM"),
        (13, "-LDDBIRH"),
        (12, "-LDCLK"),
        (11, "-LDOPC"),
        (10, "-LDMODE"),
        (9, "NC"),
        (7, "NC"),
    ];
    for (eadr, (pin, name)) in WRITES.iter().enumerate() {
        // `src/netlist.rs` gives every `NC` pin a net of its own, so the two
        // strobes nothing uses are matched by prefix.
        assert!(net_at(&n, p, *pin).starts_with(name), "EADR {eadr}");
    }
    assert_eq!(WRITES[spy::MODE as usize].1, "-LDMODE");
}

/// The mode register itself, OLORD1 1A04: `SPY<5:0>` into a 74S174's six D
/// inputs, and the six outputs [`Mode`] names.
///
/// The D and Q pin orders are `muir::part::pinout`'s, not a table typed
/// here.
#[test]
fn the_mode_registers_bits_are_the_boards() {
    let n = board();
    let p = part(&n, "OLORD1", "1A04");
    assert_eq!(p.kind, "74S174");
    // The ICMEM board's own `-RESET`, one net a board since discrepancy 13
    // was settled by MIT's wire lists.
    assert_eq!(net_at(&n, p, 1), "-ICMEM RESET", "asynchronous clear");
    assert_eq!(net_at(&n, p, 9), "-LDMODE", "clock");

    // The 74S174's D pins in bit order, and the Q pins they drive. `pinout`
    // knows the outputs; the inputs are the six pins left over.
    let pinout = muir::part::pinout("74S174").expect("74S174");
    const D: [u8; 6] = [3, 4, 6, 11, 13, 14];
    const Q: [u8; 6] = [2, 5, 7, 10, 12, 15];
    assert_eq!(pinout.outputs, &Q, "the Q pins are the outputs");

    const BITS: [&str; 6] = ["SPEED0", "SPEED1", "ERRSTOP", "STATHENB", "TRAPENB", "PROMDISABLE"];
    for (bit, name) in BITS.iter().enumerate() {
        assert_eq!(net_at(&n, p, D[bit]), format!("SPY{bit}"), "D input for {name}");
        assert_eq!(net_at(&n, p, Q[bit]), *name, "Q output for SPY{bit}");
    }
}

/// MIT's own two sentences about the register, made falsifiable.
///
/// > Writing 4 in Unibus location 766012 ... turns on ERROR-STOP-ENABLE.  If
/// > we aren't really in a PROM, we write 44 which also turns on (leaves on)
/// > PROM-DISABLE.
///
/// So `ERRSTOP` is bit 2 and `PROMDISABLE` is bit 5, which is what the board
/// says above, from a completely separate source.
#[test]
fn writing_44_turns_on_error_stop_and_prom_disable() {
    let mut m = Mode::default();
    m.write(0o4);
    assert_eq!(
        m,
        Mode { errstop: true, ..Mode::default() },
        "4 turns on ERROR-STOP-ENABLE and nothing else"
    );

    m.write(0o44);
    assert_eq!(
        m,
        Mode { errstop: true, prom_disable: true, ..Mode::default() },
        "44 also turns on PROM-DISABLE"
    );

    // Six flip flops: the other ten Unibus bits go nowhere, and a later write
    // replaces all six rather than setting them.
    m.write(0o177700);
    assert_eq!(m, Mode::default(), "the top ten bits are not wired");
}

/// The write reaching the register the way the boot PROM sends it: down the
/// Unibus, by physical address, with the word in the bottom 16 bits of `MD`.
#[test]
fn a_bus_write_at_766012_takes_the_prom_away() {
    let mut m = Machine::new();
    assert!(!m.mode.prom_disable, "-RESET leaves the PROM in place");

    // What the PROM writes at `PAGE-0-PARITY-FIX`: parity checking on, and
    // the PROM emphatically still there.
    m.bus_write(0o17773005, 0o4);
    assert!(m.mode.errstop);
    assert!(!m.mode.prom_disable);
    assert_eq!(m.bus_error, 0, "something answered: no Unibus timeout");

    // What `JUMP-TO-6` writes.
    m.bus_write(0o17773005, 0o44);
    assert!(m.mode.prom_disable);

    // The fetch is the only thing that reads it.
    m.prom[0o10] = muir::isa::Insn::new(1);
    m.imem[0o10] = muir::isa::Insn::new(2);
    assert_eq!(m.fetch(0o10).raw(), 2, "location 10 now comes from the control store");
}

// --- The whole map, read off the board ---------------------------------------

/// The two read decoders at SPY0 1F01 and 1F02, split by `EADR3`: sixteen
/// selects, fifteen of them wired.  The names are the nets'; CC's symbols in
/// `lcadrd.lisp` give the same sixteen numbers the same jobs, and
/// `mit/cadr/ir.bits` lists them in the same order --- with the bit ranges of
/// `IR-LOW` and `IR-HIGH` swapped, which both the board and CC contradict.
#[test]
fn the_read_decoders_select_the_sixteen_registers() {
    let n = board();
    // Y0..Y7 on pins 15, 14, 13, 12, 11, 10, 9, 7.
    const Y: [u8; 8] = [15, 14, 13, 12, 11, 10, 9, 7];

    let low = part(&n, "SPY0", "1F01");
    assert_eq!(low.kind, "74S138");
    assert_eq!(net_at(&n, low, 4), "-DBREAD", "G2A");
    assert_eq!(net_at(&n, low, 5), "EADR3", "G2B: the low eight, EADR3 low");
    assert_eq!(net_at(&n, low, 6), "HI1", "G1");
    const LOW: [&str; 8] =
        ["-SPY.IRL", "-SPY.IRM", "-SPY.IRH", "NC", "-SPY.OPC", "-SPY.PC", "-SPY.OBL", "-SPY.OBH"];
    for (eadr, (pin, name)) in Y.iter().zip(LOW).enumerate() {
        assert!(net_at(&n, low, *pin).starts_with(name), "EADR {eadr}");
    }

    let high = part(&n, "SPY0", "1F02");
    assert_eq!(high.kind, "74S138");
    assert_eq!(net_at(&n, high, 4), "-DBREAD", "G2A");
    assert_eq!(net_at(&n, high, 5), "GND", "G2B");
    assert_eq!(net_at(&n, high, 6), "EADR3", "G1: the high eight, EADR3 high");
    const HIGH: [&str; 8] = [
        "-SPY.FLAG1",
        "-SPY.FLAG2",
        "-SPY.ML",
        "-SPY.MH",
        "-SPY.AL",
        "-SPY.AH",
        "-SPY.STL",
        "-SPY.STH",
    ];
    for (k, (pin, name)) in Y.iter().zip(HIGH).enumerate() {
        assert_eq!(net_at(&n, high, *pin), name, "EADR {}", k + 8);
    }

    // The numbers `src/spy.rs` gives them.
    assert_eq!(LOW[spy::IR_LOW as usize], "-SPY.IRL");
    assert_eq!(LOW[spy::IR_MED as usize], "-SPY.IRM");
    assert_eq!(LOW[spy::IR_HIGH as usize], "-SPY.IRH");
    assert_eq!(LOW[spy::OPC as usize], "-SPY.OPC");
    assert_eq!(LOW[spy::PC as usize], "-SPY.PC");
    assert_eq!(LOW[spy::OB_LOW as usize], "-SPY.OBL");
    assert_eq!(LOW[spy::OB_HIGH as usize], "-SPY.OBH");
    assert_eq!(HIGH[spy::FLAG_1 as usize - 8], "-SPY.FLAG1");
    assert_eq!(HIGH[spy::FLAG_2 as usize - 8], "-SPY.FLAG2");
    assert_eq!(HIGH[spy::M_LOW as usize - 8], "-SPY.ML");
    assert_eq!(HIGH[spy::M_HIGH as usize - 8], "-SPY.MH");
    assert_eq!(HIGH[spy::A_LOW as usize - 8], "-SPY.AL");
    assert_eq!(HIGH[spy::A_HIGH as usize - 8], "-SPY.AH");
    assert_eq!(HIGH[spy::STAT_LOW as usize - 8], "-SPY.STL");
    assert_eq!(HIGH[spy::STAT_HIGH as usize - 8], "-SPY.STH");
}

/// What one read select puts on `SPY<15:0>`: for every 74LS244 or 74LS240
/// enabled by it, the net on each buffer's input, by the `SPY` bit its
/// output drives.  The input-to-output pairs come from `muir::part::behaviour`
/// and not from a table typed here.  Returns the kind of buffer too, since a
/// 74LS240 inverts and a 74LS244 does not.
fn spy_sources(n: &Netlist, select: &str) -> Vec<(String, &'static str)> {
    let mut bits: Vec<Option<(String, &'static str)>> = vec![None; 16];
    for p in &n.parts {
        let enabled =
            p.pins.iter().any(|&(pin, id)| (pin == 1 || pin == 19) && n.net(id) == select);
        if !enabled {
            continue;
        }
        let kind: &'static str = match p.kind.as_str() {
            "74LS244" => "74LS244",
            "74LS240" => "74LS240",
            other => panic!("{select} enables a {other} at {} {}", p.page, p.reference),
        };
        let b = muir::part::behaviour(kind).expect("buffer behaviour");
        for g in b.gates {
            let (enable, input) = (g.ins[0], g.ins[1]);
            if net_at(n, p, enable) != select {
                continue;
            }
            let out = net_at(n, p, g.out);
            let bit: usize =
                out.strip_prefix("SPY").and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                    panic!("{} {} pin {} drives {out}, not a SPY bit", p.page, p.reference, g.out)
                });
            let src = net_at(n, p, input).to_string();
            let src = if src.starts_with("NC") { "NC".to_string() } else { src };
            assert!(bits[bit].is_none(), "{select}: SPY{bit} driven twice");
            bits[bit] = Some((src, kind));
        }
    }
    bits.into_iter()
        .enumerate()
        .map(|(b, x)| x.unwrap_or_else(|| panic!("{select}: nothing drives SPY{b}")))
        .collect()
}

/// **Every read register, net by net.**  Each of the fifteen selects enables
/// two octal buffers, and the sixteen input nets are the register CC names,
/// bit 0 of the word on `SPY0`.  This is the table `src/spy.rs` documents and
/// every engine's `spy_read` answers to, read off the board rather than off
/// `ir.bits` --- which has `IR-LOW` and `IR-HIGH` the wrong way round.
#[test]
fn every_read_register_is_the_nets_its_buffers_carry() {
    let n = board();
    let bus = |prefix: &str, from: u32| -> Vec<String> {
        (from..from + 16).map(|b| format!("{prefix}{b}")).collect()
    };
    // Fourteen bits and two grounds.
    let short = |prefix: &str| -> Vec<String> {
        (0..14).map(|b| format!("{prefix}{b}")).chain(["GND".into(), "GND".into()]).collect()
    };
    // ALATCH names the A bus's low half `AA` and drives bit 31 as `A31A`.
    let a_high: Vec<String> = (16..31).map(|b| format!("A{b}")).chain(["A31A".into()]).collect();

    let plain: [(u8, &str, Vec<String>); 13] = [
        (spy::IR_LOW, "-SPY.IRL", bus("IR", 0)),
        (spy::IR_MED, "-SPY.IRM", bus("IR", 16)),
        (spy::IR_HIGH, "-SPY.IRH", bus("IR", 32)),
        (spy::OPC, "-SPY.OPC", short("OPC")),
        (spy::PC, "-SPY.PC", short("PC")),
        (spy::OB_LOW, "-SPY.OBL", bus("OB", 0)),
        (spy::OB_HIGH, "-SPY.OBH", bus("OB", 16)),
        (spy::M_LOW, "-SPY.ML", bus("M", 0)),
        (spy::M_HIGH, "-SPY.MH", bus("M", 16)),
        (spy::A_LOW, "-SPY.AL", bus("AA", 0)),
        (spy::A_HIGH, "-SPY.AH", a_high),
        (spy::STAT_LOW, "-SPY.STL", bus("ST", 0)),
        (spy::STAT_HIGH, "-SPY.STH", bus("ST", 16)),
    ];
    for (eadr, select, want) in &plain {
        let got = spy_sources(&n, select);
        for (bit, ((src, kind), w)) in got.iter().zip(want).enumerate() {
            assert_eq!(src, w, "register {eadr} ({select}) bit {bit}");
            assert_eq!(*kind, "74LS244", "register {eadr} bit {bit}: not inverting");
        }
    }

    // FLAG-1: `ir.bits`'s order, bit 15 first, and CC's polarity note.  The
    // low byte is a 74LS240, so its eight active-low error flags read high
    // for an error.
    let flag1 = spy_sources(&n, "-SPY.FLAG1");
    const FLAG1_HIGH: [&str; 8] =
        ["-WAIT", "-V1PE", "-V0PE", "PROMDISABLE", "-STATHALT", "ERR", "SSDONE", "SRUN"];
    const FLAG1_LOW: [&str; 8] =
        ["-HIGHERR", "-MEMPE", "-IPE", "-DPE", "-SPE", "-PDLPE", "-MPE", "-APE"];
    for (k, name) in FLAG1_HIGH.iter().enumerate() {
        let (src, kind) = &flag1[15 - k];
        assert_eq!((src.as_str(), *kind), (*name, "74LS244"), "FLAG-1 bit {}", 15 - k);
    }
    for (k, name) in FLAG1_LOW.iter().enumerate() {
        let (src, kind) = &flag1[7 - k];
        assert_eq!((src.as_str(), *kind), (*name, "74LS240"), "FLAG-1 bit {}", 7 - k);
    }
    // And the word `spy::Flag1` builds from the logical senses: an error or a
    // wait reads as a zero in the high byte and a one in the low.
    let none = spy::Flag1::default().word();
    assert_eq!(none, 0xe800, "no error, no wait, not halted, no PROM disable: bits 15, 14, 13, 11");
    let all = spy::Flag1 {
        wait: true,
        v1pe: true,
        v0pe: true,
        promdisable: true,
        stathalt: true,
        err: true,
        ssdone: true,
        srun: true,
        higherr: true,
        mempe: true,
        ipe: true,
        dpe: true,
        spe: true,
        pdlpe: true,
        mpe: true,
        ape: true,
    };
    assert_eq!(all.word(), 0x17ff);

    // And `of` reads that word back, polarities and all, so a console --- or
    // muir's own run loop --- gets out what the board put in.
    assert_eq!(spy::Flag1::of(none), spy::Flag1::default(), "the quiet machine round-trips");
    assert_eq!(spy::Flag1::of(all.word()), all, "everything up round-trips");
    for bit in 0..16 {
        let w = 1u16 << bit;
        assert_eq!(spy::Flag1::of(w).word(), w, "bit {bit} alone round-trips");
    }

    // FLAG-2: four unconnected inputs read high, `-VMAOK` reads low when
    // the access was permitted, and the rest are what they say.
    let flag2 = spy_sources(&n, "-SPY.FLAG2");
    const FLAG2: [&str; 16] = [
        "PCS0",
        "PCS1",
        "JCOND",
        "-VMAOK",
        "NOP",
        "IR48",
        "NC",
        "NC",
        "SPUSHD",
        "PDLWRITED",
        "IMODD",
        "IWRITED",
        "DESTSPCD",
        "WMAPD",
        "NC",
        "NC",
    ];
    for (bit, name) in FLAG2.iter().enumerate() {
        let (src, kind) = &flag2[bit];
        assert_eq!((src.as_str(), *kind), (*name, "74LS244"), "FLAG-2 bit {bit}");
    }
    assert_eq!(spy::Flag2::default().word(), spy::Flag2::OPEN | 1 << 3, "open bits, and -VMAOK");
    assert_eq!(spy::Flag2 { vmaok: true, ..Default::default() }.word(), 0xc0c0);
    assert_eq!(
        spy::Flag2 { wmapd: true, pcs0: true, ..Default::default() }.word(),
        0xc0c0 | 1 << 13 | 1 << 3 | 1
    );
}

/// The clock control register: `RUN` in the 74S74 at OLORD1 1A14, preset by
/// `-BOOT`; `STEP`, `NOP11`, `IDEBUG` and `LDSTAT` in the 74S175 at 1A09,
/// cleared by `-RESET`; all clocked by `-LDCLK`.  The bit order is CC's ---
/// `CC-CLOCK` writes `2` for a step, `CC-DEBUG-CLOCK` `12` for a step with
/// `IDEBUG`, `CC-NOOP-DEBUG-CLOCK` `16` with `NOP` as well --- and `ir.bits`'s.
#[test]
fn the_clock_control_register_is_run_step_nop11_idebug_and_ldstat() {
    let n = board();
    let run = part(&n, "OLORD1", "1A14");
    assert_eq!(run.kind, "74S74");
    assert_eq!(net_at(&n, run, 1), "'-CLOCK RESET A'", "clear: the power-on reset, not -RESET");
    assert_eq!(net_at(&n, run, 2), "SPY0", "D");
    assert_eq!(net_at(&n, run, 3), "-LDCLK", "clock");
    assert_eq!(net_at(&n, run, 4), "-BOOT", "preset: booting starts the machine");
    assert_eq!(net_at(&n, run, 5), "RUN", "Q");

    let p = part(&n, "OLORD1", "1A09");
    assert_eq!(p.kind, "74S175");
    // The ICMEM board's own `-RESET`, as the mode register's test says.
    assert_eq!(net_at(&n, p, 1), "-ICMEM RESET", "clear");
    assert_eq!(net_at(&n, p, 9), "-LDCLK", "clock");
    // The 74S175's D pins in `muir::part`'s bit order, and the Q pins they
    // drive: D1 4 to Q1 2, D2 5 to Q2 7, D3 12 to Q3 10, D4 13 to Q4 15.
    let pinout = muir::part::pinout("74S175").expect("74S175");
    assert!([2, 7, 10, 15].iter().all(|q| pinout.outputs.contains(q)));
    const DQ: [(u8, u8, &str, &str); 4] = [
        (4, 2, "SPY4", "LDSTAT"),
        (5, 7, "SPY3", "IDEBUG"),
        (12, 10, "SPY2", "NOP11"),
        (13, 15, "SPY1", "STEP"),
    ];
    for (d, q, spy_bit, name) in DQ {
        assert_eq!(net_at(&n, p, d), spy_bit, "D for {name}");
        assert_eq!(net_at(&n, p, q), name, "Q for {spy_bit}");
    }

    let mut c = spy::ClockControl::default();
    c.write(0o12);
    assert_eq!(
        c,
        spy::ClockControl { step: true, idebug: true, ..Default::default() },
        "CC-DEBUG-CLOCK"
    );
    c.write(0o16);
    assert_eq!(
        c,
        spy::ClockControl { step: true, nop11: true, idebug: true, ..Default::default() },
        "CC-NOOP-DEBUG-CLOCK"
    );
    c.write(0o1);
    assert_eq!(c, spy::ClockControl { run: true, ..Default::default() }, "the one permitted write");
    c.write(0o20);
    assert_eq!(c, spy::ClockControl { ldstat: true, ..Default::default() });
    c.write(0o177740);
    assert_eq!(c, spy::ClockControl::default(), "the top eleven bits are not wired");
}

/// The OPC control register: three bits of the 74S175 at OLORD1 1A08, clocked
/// by `-LDOPC`, in `ir.bits`'s order.  The fourth flip flop drives nothing.
#[test]
fn the_opc_control_register_is_lpc_hold_opcclk_and_opcinh() {
    let n = board();
    let p = part(&n, "OLORD1", "1A08");
    assert_eq!(p.kind, "74S175");
    assert_eq!(net_at(&n, p, 1), "-ICMEM RESET", "clear");
    assert_eq!(net_at(&n, p, 9), "-LDOPC", "clock");
    const DQ: [(u8, u8, &str, &str); 3] =
        [(5, 7, "SPY2", "OPCINH"), (12, 10, "SPY1", "OPCCLK"), (13, 15, "SPY0", "LPC.HOLD")];
    for (d, q, spy_bit, name) in DQ {
        assert_eq!(net_at(&n, p, d), spy_bit, "D for {name}");
        assert_eq!(net_at(&n, p, q), name, "Q for {spy_bit}");
    }
    assert_eq!(net_at(&n, p, 4), "SPY3", "the fourth D");
    assert!(net_at(&n, p, 2).starts_with("NC"), "and its Q goes nowhere");

    // Where the three go.  `LPC.HOLD` is the enable of the 25S07s that hold
    // LPC; `OPCCLK` is NORed with `-CLK5` into the 9328s' common clock at
    // OPCS 1F14; `OPCINH` reaches their own clock pins through OPCS 1F10.
    for r in ["4F06", "4F07", "4F08"] {
        let lpc = part(&n, "LPC", r);
        assert_eq!(lpc.kind, "25S07");
        assert_eq!(net_at(&n, lpc, 1), "LPC.HOLD", "enable of {r}");
    }
    let nor = part(&n, "OPCS", "1F14");
    assert_eq!(nor.kind, "74S02");
    assert_eq!(net_at(&n, nor, 8), "OPCCLK");
    assert_eq!(net_at(&n, nor, 9), "-CLK5");
    assert_eq!(net_at(&n, nor, 10), "OPCCLKC");

    let mut o = spy::OpcControl::default();
    o.write(2);
    assert_eq!(o, spy::OpcControl { opcclk: true, ..Default::default() }, "CC-SAVE-OPCS's pulse");
    o.write(5);
    assert_eq!(o, spy::OpcControl { lpc_hold: true, opcinh: true, ..Default::default() });
    o.write(0o177770);
    assert_eq!(o, spy::OpcControl::default());
}

/// The debug IR: six 74S374s on page DEBUG, `SPY<15:0>` straight into
/// `I<15:0>`, `I<31:16>` and `I<47:32>` under the three strobes, and all six
/// output-enabled by `-IDEBUG`.  The D and Q pin pairs are the '374's: D1 to
/// D8 on 3, 4, 7, 8, 13, 14, 17, 18 and Q1 to Q8 on 2, 5, 6, 9, 12, 15, 16,
/// 19, which is `muir::part::pinout`'s output list.
#[test]
fn the_debug_ir_takes_spy_straight_into_the_i_bus() {
    let n = board();
    const D: [u8; 8] = [3, 4, 7, 8, 13, 14, 17, 18];
    const Q: [u8; 8] = [2, 5, 6, 9, 12, 15, 16, 19];
    assert_eq!(muir::part::pinout("74S374").expect("74S374").outputs, &Q);
    const HALVES: [(&str, u8, [&str; 2]); 3] = [
        ("-LDDBIRL", spy::IR_LOW, ["1E15", "1E14"]),
        ("-LDDBIRM", spy::IR_MED, ["1E13", "1E12"]),
        ("-LDDBIRH", spy::IR_HIGH, ["1E11", "1F15"]),
    ];
    let mut seen = 0;
    for (strobe, half, refs) in HALVES {
        for r in refs {
            let p = part(&n, "DEBUG", r);
            assert_eq!(p.kind, "74S374");
            assert_eq!(net_at(&n, p, 1), "-IDEBUG", "output enable of {r}");
            assert_eq!(net_at(&n, p, 11), strobe, "clock of {r}");
            for (d, q) in D.iter().zip(Q) {
                let spy_bit: u32 = net_at(&n, p, *d).strip_prefix("SPY").unwrap().parse().unwrap();
                let i_bit: u32 = net_at(&n, p, q).strip_prefix("I").unwrap().parse().unwrap();
                assert_eq!(i_bit, spy_bit + 16 * half as u32, "{r} D{d} to Q{q}");
                seen += 1;
            }
        }
    }
    assert_eq!(seen, 48);

    let mut ir = 0u64;
    spy::write_debug_ir(&mut ir, spy::IR_HIGH, 0o123456);
    spy::write_debug_ir(&mut ir, spy::IR_LOW, 0o7);
    spy::write_debug_ir(&mut ir, spy::IR_MED, 0o177777);
    assert_eq!(ir, 0o123456 << 32 | 0o177777 << 16 | 0o7);
    spy::write_debug_ir(&mut ir, spy::IR_MED, 0);
    assert_eq!(ir, 0o123456 << 32 | 0o7, "a half is replaced, the others kept");
}

/// Bits 6 and 7 of a mode-register write are pulses, not settings: `SPY6`
/// and `SPY7` are gated with `LDMODE` on OLORD2 into `-PROG.RESET` and
/// `PROG.BOOT`, and nothing stores them.  CC's `CC-RESET-MACH` writes `100`
/// and then the mode it wants, because the reset has just cleared the
/// register it wrote.
#[test]
fn a_mode_register_write_with_bit_6_or_7_is_a_reset_or_a_boot() {
    let n = board();
    let reset = part(&n, "OLORD2", "1C09");
    assert_eq!(reset.kind, "74S00");
    assert_eq!(net_at(&n, reset, 4), "LDMODE");
    assert_eq!(net_at(&n, reset, 5), "SPY6");
    assert_eq!(net_at(&n, reset, 6), "-PROG.RESET");
    let boot = part(&n, "OLORD2", "1D10");
    assert_eq!(boot.kind, "74S08");
    assert_eq!(net_at(&n, boot, 4), "LDMODE");
    assert_eq!(net_at(&n, boot, 5), "SPY7");
    assert_eq!(net_at(&n, boot, 6), "PROG.BOOT");
    // `-PROG.RESET` is one of the three ways `RESET` is made.
    let rst = part(&n, "OLORD2", "1C08");
    assert_eq!(rst.kind, "74S10O");
    assert_eq!(net_at(&n, rst, 9), "-BOOT");
    assert_eq!(net_at(&n, rst, 10), "'-CLOCK RESET B'");
    assert_eq!(net_at(&n, rst, 11), "-PROG.RESET");
    assert_eq!(net_at(&n, rst, 8), "RESET");
    assert_eq!(spy::MODE_RESET, 0o100);
    assert_eq!(spy::MODE_BOOT, 0o200);

    let mut m = Machine::new();
    m.spy_write(spy::MODE, 0o46);
    assert!(m.mode.prom_disable && !m.prog_reset && !m.prog_boot);
    m.spy_write(spy::MODE, 0o100);
    assert!(m.prog_reset, "CC-RESET-MACH's write");
    assert!(!m.prog_boot);
    assert_eq!(m.mode, Mode::default(), "the six bits take the write's zeros meanwhile");
    m.prog_reset = false;
    m.spy_write(spy::MODE, 0o246);
    assert!(m.prog_boot && !m.prog_reset);
    assert_eq!(
        m.mode,
        Mode { speed1: true, errstop: true, prom_disable: true, ..Default::default() }
    );
}

/// The write decoder sees `EADR<2:0>` only, so the eight high registers
/// alias the low eight for writing, and Y6 and Y7 load nothing.  Every
/// console register through `Machine::spy_write`, which is what a Unibus
/// write into the block does with its data.
#[test]
fn a_write_is_decoded_from_three_address_bits() {
    let mut m = Machine::new();
    assert_eq!(spy::write_strobe(spy::MODE), spy::MODE);
    assert_eq!(spy::write_strobe(spy::MODE + 8), spy::MODE, "0o766052 loads the mode register too");
    m.spy_write(spy::MODE + 8, 0o44);
    assert_eq!(m.mode, Mode { errstop: true, prom_disable: true, ..Default::default() });

    m.spy_write(spy::IR_LOW, 0o1);
    m.spy_write(spy::IR_MED + 8, 0o2);
    m.spy_write(spy::IR_HIGH, 0o3);
    assert_eq!(m.debug_ir, 3 << 32 | 2 << 16 | 1);
    m.spy_write(spy::CLK, 0o12);
    assert_eq!(
        m.clock_control,
        spy::ClockControl { step: true, idebug: true, ..Default::default() }
    );
    m.spy_write(spy::OPC_CONTROL, 0o4);
    assert_eq!(m.opc_control, spy::OpcControl { opcinh: true, ..Default::default() });

    let before = m.clone();
    for eadr in [6, 7, 14, 15] {
        m.spy_write(eadr, 0o177777);
    }
    assert_eq!(m.mode, before.mode);
    assert_eq!(m.debug_ir, before.debug_ir);
    assert_eq!(m.clock_control, before.clock_control);
    assert_eq!(m.opc_control, before.opc_control);

    // Down the Unibus, as `JUMP-TO-6` and CC both send them: `0o766006` is
    // virtual `0o1003` under the boot's map, physical `0o17773003`.
    let mut m = Machine::new();
    m.bus_write(0o17773003, 0o1);
    assert!(m.clock_control.run);
    m.bus_write(0o17773000, 0o4321);
    assert_eq!(m.debug_ir, 0o4321);
    // `-RESET` clears the 74S175 half and not `RUN`.
    m.spy_write(spy::CLK, 0o37);
    m.spy_write(spy::MODE, 0o46);
    m.reset_console_registers();
    assert_eq!(m.clock_control, spy::ClockControl { run: true, ..Default::default() });
    assert_eq!(m.mode, Mode::default());
}

// --- The console at work ------------------------------------------------------
//
// Hand-assembled microinstructions, in `muir::isa::asm`'s encoding.

use muir::engine::Engine;
use muir::isa::Insn;
use muir::isa::asm::{ALU, SETA, SETM, SRC_MD, START_READ, STAT, a_dest, a_src, filler, m_src};
use muir::micro::Micro;
use muir::rtl::Rtl;

/// A machine whose whole boot PROM is [`filler`], with `A3` set to a value
/// the console can recognise.
fn straight_line_machine() -> Machine {
    let mut m = Machine::new();
    m.amem[3] = 0o123456;
    m.load_prom(&vec![filler(); 512]);
    m
}

/// CC's clock protocol on an engine: write the clock control register, let
/// the synchroniser take it and the microcycle run, then write zero and let
/// `SSDONE` clear.  `SSTEP` follows `STEP` one master clock on, the one
/// microcycle runs in the next, and `SSDONE` follows `SSTEP` after that; two
/// steps a write is what the board needs, where CC's Unibus writes take
/// microseconds each.
fn cc_clock(e: &mut dyn Engine, clk: u16) {
    e.spy_write(spy::CLK, clk);
    e.step().unwrap();
    e.step().unwrap();
    e.spy_write(spy::CLK, 0);
    e.step().unwrap();
    e.step().unwrap();
}

/// Writes the debug IR in three halves, as `CC-EXECUTE-R` and `-W` do.
fn load_debug_ir(e: &mut dyn Engine, insn: u64) {
    e.spy_write(spy::IR_LOW, insn as u16);
    e.spy_write(spy::IR_MED, (insn >> 16) as u16);
    e.spy_write(spy::IR_HIGH, (insn >> 32) as u16);
}

fn word32(e: &dyn Engine, low: u8, high: u8) -> u32 {
    (e.spy_read(high) as u32) << 16 | e.spy_read(low) as u32
}

/// `FLAG-1` with nothing wrong and the machine running: the three
/// active-low no-error bits, `-STATHALT` up, and `SRUN`.
const RUNNING: u16 = 0xe800 | 0x100;
/// The same with `SRUN` down.
const HALTED: u16 = 0xe800;

/// **`RUN` down halts the machine, `STEP` runs it one microcycle, `RUN` up
/// starts it again.**  `SRUN` is one master clock behind `RUN`, so the
/// microcycle in flight when the write lands still completes; `SSTEP` and
/// `SSDONE` are one and two behind `STEP`, so the step's microcycle is the
/// second master clock after the write and `FLAG-1` shows `SSDONE` from the
/// third until two after `STEP` is lowered.
#[test]
fn rtl_halts_and_steps_under_the_clock_control_register() {
    let mut e = Rtl::new(straight_line_machine());
    e.boot();
    for _ in 0..10 {
        e.step().unwrap();
    }
    // `PC` is the next fetch; the instruction standing in `IR` is the one
    // before it, which is what the next microcycle executes.
    let pc = e.pc();
    assert!(pc > 1);
    assert_eq!(e.spy_read(spy::PC), pc);
    assert_eq!(e.spy_read(spy::FLAG_1), RUNNING);

    e.spy_write(spy::CLK, 0);
    e.step().unwrap();
    assert_eq!(e.executed(), Some(pc - 1), "the microcycle in flight completes");
    assert_eq!(e.pc(), pc + 1);
    let t = e.ns();
    for _ in 0..5 {
        e.step().unwrap();
        assert_eq!(e.executed(), None, "halted");
    }
    assert_eq!(e.pc(), pc + 1, "nothing moves");
    assert_eq!(e.ns() - t, 5 * 220, "but the master clock runs on, a generator cycle a step");
    assert_eq!(e.halted_ns(), 5 * 220);
    assert_eq!(e.spy_read(spy::FLAG_1), HALTED, "SRUN down");

    // CC-CLOCK: 2, then 0.
    e.spy_write(spy::CLK, 2);
    e.step().unwrap();
    assert_eq!(e.executed(), None, "SSTEP rises at this master clock; no microcycle yet");
    e.step().unwrap();
    assert_eq!(e.executed(), Some(pc), "the one microcycle");
    assert_eq!(e.pc(), pc + 2);
    assert_eq!(e.spy_read(spy::FLAG_1), HALTED | 0x200, "SSDONE");
    e.step().unwrap();
    e.step().unwrap();
    assert_eq!(e.pc(), pc + 2, "one, and no more while STEP stays up");
    e.spy_write(spy::CLK, 0);
    e.step().unwrap();
    assert_eq!(e.spy_read(spy::FLAG_1), HALTED | 0x200, "SSDONE is two master clocks behind STEP");
    e.step().unwrap();
    assert_eq!(e.spy_read(spy::FLAG_1), HALTED);
    cc_clock(&mut e, 2);
    assert_eq!(e.pc(), pc + 3, "and again");

    e.spy_write(spy::CLK, 1);
    e.step().unwrap();
    assert_eq!(e.executed(), None, "SRUN rises at this master clock");
    for k in 0..5 {
        e.step().unwrap();
        assert_eq!(e.executed(), Some(pc + 2 + k), "running");
    }
    assert_eq!(e.spy_read(spy::FLAG_1), RUNNING);
}

/// **`CC-EXECUTE-R` and `CC-EXECUTE-W`, as CC does them.**  With `NOP11` and
/// `IDEBUG` up, one step loads `IR` from the debug IR without executing the
/// instruction that was there; the datapath then shows the console the new
/// instruction's operands and result on the A, M and O buses without having
/// executed it either.  A step with `IDEBUG` alone executes it, and its
/// scratchpad write lands a microcycle late, which is why CC follows a
/// write with one more clock in NOP and DEBUG mode, "which finishes writes".
#[test]
fn rtl_loads_ir_from_the_debug_ir_and_shows_its_result_before_executing_it() {
    let mut m = straight_line_machine();
    m.amem[5] = 0o12345670;
    m.mmem[7] = 0o777;
    let mut e = Rtl::new(m);
    e.boot();
    for _ in 0..10 {
        e.step().unwrap();
    }
    e.spy_write(spy::CLK, 0);
    e.step().unwrap();
    e.step().unwrap();
    let pc = e.pc();

    // `((A-MEM 101) SETA A-MEM-5)`, with M memory 7 on the M bus for show.
    let insn = ALU | SETA | a_src(5) | m_src(7) | a_dest(0o101);
    load_debug_ir(&mut e, insn);
    assert_eq!(e.machine().debug_ir, insn);

    // CC-NOOP-DEBUG-CLOCK: 16, then 0.
    cc_clock(&mut e, 0o16);
    assert_eq!(e.ir(), insn, "IR holds the debug IR");
    assert_eq!(e.spy_read(spy::IR_LOW), insn as u16);
    assert_eq!(e.spy_read(spy::IR_MED), (insn >> 16) as u16);
    assert_eq!(e.spy_read(spy::IR_HIGH), (insn >> 32) as u16);
    assert_eq!(word32(&e, spy::A_LOW, spy::A_HIGH), 0o12345670, "A memory 5 on the A bus");
    assert_eq!(word32(&e, spy::M_LOW, spy::M_HIGH), 0o777, "M memory 7 on the M bus");
    assert_eq!(word32(&e, spy::OB_LOW, spy::OB_HIGH), 0o12345670, "and SETA on the OB");
    assert_eq!(e.machine().amem[0o101], 0, "not executed: NOP11 was up");
    assert_eq!(e.pc(), pc + 1, "the nopped microcycle still advanced PC");
    assert_eq!(e.spy_read(spy::FLAG_2) & 0x10, 0, "NOP is down again with NOP11");

    // CC-DEBUG-CLOCK: 12, then 0.  Executes it.
    cc_clock(&mut e, 0o12);
    assert_eq!(e.machine().amem[0o101], 0, "the write is a microcycle behind, pending in L");
    assert_eq!(e.ir(), insn, "IDEBUG loaded it again");
    cc_clock(&mut e, 0o16);
    assert_eq!(e.machine().amem[0o101], 0o12345670, "landed by the clock that finishes writes");

    // IDEBUG down: the control store is back on the I bus.
    cc_clock(&mut e, 2);
    assert_eq!(e.ir(), filler().raw());
}

/// **`CC-SAVE-OPCS`.**  The OPC read select shows the last stage of the
/// shift register, the oldest of the eight PCs; each `OPCCLK` pulse on the
/// halted machine shifts the register once, `PC` in at the front, so eight
/// reads give the history oldest first and leave the register full of the
/// halted `PC`.  `OPCINH` freezes it while the machine runs, and so does
/// `OPCCLK` held high.
#[test]
fn rtl_reads_the_opc_history_out_under_opcclk_and_freezes_it_under_opcinh() {
    let mut e = Rtl::new(straight_line_machine());
    e.boot();
    for _ in 0..20 {
        e.step().unwrap();
    }
    e.spy_write(spy::CLK, 0);
    e.step().unwrap();
    e.step().unwrap();
    let pc = e.pc();
    let history = e.opc_history();
    assert_eq!(e.spy_read(spy::OPC), history[7], "the last stage");
    assert_ne!(history[0], history[7]);

    let mut saved = [0u16; 8];
    for s in &mut saved {
        *s = e.spy_read(spy::OPC);
        e.spy_write(spy::OPC_CONTROL, 2);
        e.step().unwrap();
        e.spy_write(spy::OPC_CONTROL, 0);
        e.step().unwrap();
    }
    for (k, s) in saved.iter().enumerate() {
        assert_eq!(*s, history[7 - k], "read {k}: oldest first");
    }
    assert_eq!(e.opc_history(), [pc; 8], "eight shifts of a halted machine fill it with PC");
    assert_eq!(e.pc(), pc, "and the machine has not moved");

    e.spy_write(spy::OPC_CONTROL, 4);
    e.spy_write(spy::CLK, 1);
    for _ in 0..10 {
        e.step().unwrap();
    }
    assert!(e.pc() > pc + 5, "running");
    assert_eq!(e.opc_history(), [pc; 8], "frozen under OPCINH");
    e.spy_write(spy::OPC_CONTROL, 0);
    for _ in 0..3 {
        e.step().unwrap();
    }
    assert_ne!(e.opc_history()[0], pc, "shifting again");
    e.spy_write(spy::OPC_CONTROL, 2);
    let h = e.opc_history();
    for _ in 0..5 {
        e.step().unwrap();
    }
    assert_eq!(e.opc_history(), h, "OPCCLK held high blocks the common clock");
}

/// **`LPC.HOLD` freezes the last PC.**  `LPC`, the 25S07s at LPC
/// 4F06-4F08, loads from `PC` on every clock, so it holds the address of
/// the instruction before the one in `IR` --- what a DISPATCH with `IR<25>`
/// pushes in place of `PC`.  `ir.bits`: `LPC.HOLD` "prevents LPC from
/// changing, normally it loads from PC"; it is pin 1 of the 25S07s, their
/// enable, from the OPC control register's bit 0 at OLORD1 1A08.  Held, the
/// machine steps on and `LPC` stays; released, it follows again.
#[test]
fn lpc_hold_freezes_the_last_pc_while_the_machine_steps() {
    let mut e = Rtl::new(straight_line_machine());
    e.boot();
    for _ in 0..20 {
        e.step().unwrap();
    }
    assert_eq!(e.lpc(), e.pc() - 1, "LPC is the PC before the one in IR");
    let held = e.lpc();
    e.spy_write(spy::OPC_CONTROL, 1);
    for k in 1..=5 {
        e.step().unwrap();
        assert_eq!(e.lpc(), held, "LPC held through step {k}");
    }
    assert!(e.pc() > held + 3, "while the machine ran on");
    e.spy_write(spy::OPC_CONTROL, 0);
    e.step().unwrap();
    e.step().unwrap();
    assert_eq!(e.lpc(), e.pc() - 1, "released, LPC follows PC again");
}

/// **The statistics counter, loaded the way `ir.bits` says and halting the
/// machine the way the mode register says.**
///
/// > To load the Statistics counter, you must be the console program.  Put in
/// > DEBUG-IR an instruction whose M-source contains the data desired in the
/// > statistics counter.  Step the machine twice in NOP and DEBUG mode, once
/// > to put this instruction in IR and once to put the data into IWR.  Now
/// > step the machine in NOP, DEBUG, LDSTAT mode, which will load the
/// > statistics counter from the IWR.
///
/// The counter counts every microcycle whose instruction has `IR<46>` and is
/// not nopped; here every filler has it.  It counts *up* --- pin 1 of the
/// eight 74S169s is `HI1` --- and `STAT.OVF`, the last carry out, is
/// registered as `STATSTOP` at the edge that carries it past all ones, so
/// with `STATHENB` in the mode register the machine halts after the third
/// counted instruction from `-3`.  `ir.bits` calls it a down-counter that
/// stops at zero, which the board contradicts.
#[test]
fn rtl_loads_the_statistics_counter_from_iwr_and_halts_on_its_overflow() {
    let mut m = straight_line_machine();
    m.mmem[7] = 0xffff_fffd;
    let counted = Insn::new(filler().raw() | STAT);
    m.load_prom(&vec![counted; 512]);
    let mut e = Rtl::new(m);
    e.boot();
    for _ in 0..10 {
        e.step().unwrap();
    }
    assert_eq!(e.stat(), 8, "the trap and the word behind it are nopped and not counted");
    assert_eq!(word32(&e, spy::STAT_LOW, spy::STAT_HIGH), 8);
    e.spy_write(spy::CLK, 0);
    e.step().unwrap();
    e.step().unwrap();

    // `ir.bits`'s recipe.
    load_debug_ir(&mut e, ALU | SETM | m_src(7) | a_src(3));
    cc_clock(&mut e, 0o16);
    cc_clock(&mut e, 0o16);
    cc_clock(&mut e, 0o36);
    assert_eq!(e.stat(), 0xffff_fffd, "loaded from IWR");
    assert_eq!(word32(&e, spy::STAT_LOW, spy::STAT_HIGH), 0xffff_fffd);

    e.spy_write(spy::MODE, 0o10);
    e.spy_write(spy::CLK, 1);
    e.step().unwrap();
    let mut ran = 0;
    for _ in 0..20 {
        e.step().unwrap();
        ran += e.executed().is_some() as u32;
    }
    // The debug instruction is still standing in `IR`, uncounted; then the
    // control store's words count -3, -2, -1, and the third carries out.
    assert_eq!(ran, 4, "one uncounted, then three: STATSTOP halts the machine");
    assert_eq!(e.stat(), 0);
    assert_eq!(e.spy_read(spy::FLAG_1), RUNNING & !0x800, "SRUN up and -STATHALT down");

    // Without STATHENB the stop is a flag and not a halt.
    e.spy_write(spy::MODE, 0);
    e.step().unwrap();
    e.step().unwrap();
    assert!(e.executed().is_some(), "running again");
    assert_eq!(e.spy_read(spy::FLAG_1), RUNNING, "STATSTOP cleared at the next edge");
}

/// **All sixteen, off a known machine.**  The instruction standing in `IR` is
/// `((A-MEM 101) SETM MD)` with `A3` as its A source, so the O bus and the M
/// bus carry `MD` and the A bus carries `A3`; the OPC is the last stage of
/// the history; the flags are a running machine executing straight-line
/// code; and register 3 is the open bus.
#[test]
fn rtl_answers_the_sixteen_registers_from_its_own_state() {
    let mut m = straight_line_machine();
    m.md = 0o31415726;
    let mut prom = vec![filler(); 512];
    prom[20] = Insn::new(ALU | SETM | SRC_MD | a_src(3) | a_dest(0o101));
    m.load_prom(&prom);
    let mut e = Rtl::new(m);
    e.boot();
    // The trap, the nopped word behind it and words 0 to 19: `IR` holds
    // word 20 and `PC` is 21.
    for _ in 0..22 {
        e.step().unwrap();
    }
    assert_eq!(e.pc(), 21);
    assert_eq!(e.ir(), prom[20].raw());

    let insn = prom[20].raw();
    assert_eq!(e.spy_read(spy::IR_LOW), insn as u16);
    assert_eq!(e.spy_read(spy::IR_MED), (insn >> 16) as u16);
    assert_eq!(e.spy_read(spy::IR_HIGH), (insn >> 32) as u16);
    assert_eq!(e.spy_read(3), spy::OPEN_READ);
    assert_eq!(e.spy_read(spy::OPC), e.opc_history()[7]);
    assert_eq!(e.spy_read(spy::PC), 21);
    assert_eq!(word32(&e, spy::OB_LOW, spy::OB_HIGH), 0o31415726, "OB is MD, by SETM");
    assert_eq!(e.spy_read(spy::FLAG_1), RUNNING);
    // `JCOND` is whatever the 74S151 picks off an ALU instruction's low
    // bits; `PCS` is 3, the incremented PC; `NOP` down; the access permitted.
    assert_eq!(e.spy_read(spy::FLAG_2) & !0x4, spy::Flag2::OPEN | 0x3);
    assert_eq!(word32(&e, spy::M_LOW, spy::M_HIGH), 0o31415726, "M is MD");
    assert_eq!(word32(&e, spy::A_LOW, spy::A_HIGH), 0o123456, "A is A3");
    assert_eq!(word32(&e, spy::STAT_LOW, spy::STAT_HIGH), 0, "nothing carries the statistics bit");
}

/// **`micro` halts, steps and answers what it has.**  Its `IR` is the
/// instruction waiting to execute and its `OB`, `A` and `M` are the last
/// microcycle's; a step with `IDEBUG` executes the debug IR outright, since
/// this engine has no read phase apart from execution and no write pipeline.
#[test]
fn micro_halts_steps_and_answers_the_registers_it_has() {
    let mut m = straight_line_machine();
    m.amem[5] = 0o12345670;
    let mut e = Micro::new(m);
    e.boot();
    for _ in 0..10 {
        e.step().unwrap();
    }
    let pc = e.pc();
    assert_eq!(e.spy_read(spy::PC), pc);
    assert_eq!(e.spy_read(spy::IR_HIGH) as u64, filler().raw() >> 32);
    assert_eq!(e.spy_read(spy::A_LOW), 0o123456);
    assert_eq!(e.spy_read(spy::OB_LOW), 0o123456);
    assert_eq!(e.spy_read(spy::FLAG_1), RUNNING);
    // No mapped access yet, and this engine's `vmaok` comes up denying one.
    assert_eq!(e.spy_read(spy::FLAG_2), spy::Flag2::OPEN | 0x8, "-VMAOK up, nothing else known");
    assert_eq!(e.spy_read(3), spy::OPEN_READ);
    assert_eq!(e.spy_read(spy::STAT_LOW), 0, "no counter");

    e.spy_write(spy::CLK, 0);
    e.step().unwrap();
    let halted = e.pc();
    assert_eq!(halted, pc + 1, "the microcycle in flight completes");
    let t = e.machine().ns;
    for _ in 0..5 {
        e.step().unwrap();
        assert_eq!(e.executed(), None);
    }
    assert_eq!(e.pc(), halted);
    assert_eq!(e.machine().ns - t, 5 * 220);
    assert_eq!(e.spy_read(spy::FLAG_1), HALTED);

    cc_clock(&mut e, 2);
    assert_eq!(e.pc(), halted + 1, "one microcycle");

    load_debug_ir(&mut e, ALU | SETA | a_src(5) | a_dest(0o101));
    cc_clock(&mut e, 0o16);
    assert_eq!(e.machine().amem[0o101], 0, "NOP11: nothing executed");
    cc_clock(&mut e, 0o12);
    assert_eq!(e.machine().amem[0o101], 0o12345670, "IDEBUG: the debug IR executed, at once");
    assert_eq!(e.pc(), halted + 3);

    e.spy_write(spy::CLK, 1);
    e.step().unwrap();
    e.step().unwrap();
    assert!(e.executed().is_some(), "running again");
}

/// **A Unibus read of the block, by the microcode itself, returns the cpu's
/// own registers** --- on `rtl` through the bus interface model's timing,
/// on `micro` at once.  The map puts virtual page 0 on physical page
/// `0o37766`, which is Unibus `766000`, so virtual address `k` is register
/// `k`.  Every instruction in flight while a read is answered is the same
/// filler, so the A bus, the flags and the counter read the same whenever
/// the interface strobes them; four reads, each parked in A memory.
fn unibus_read_machine() -> Machine {
    let mut m = straight_line_machine();
    m.l1_map[0] = 0;
    m.l2_map[0] = (1 << 23) | 0o37766;
    m.mmem[1] = spy::A_LOW as u32;
    m.mmem[2] = spy::STAT_LOW as u32;
    m.mmem[3] = 3;
    m.mmem[4] = spy::FLAG_1 as u32;
    let mut prom = vec![filler(); 512];
    let mut at = 0;
    for (k, park) in [0o101, 0o102, 0o103, 0o104].iter().enumerate() {
        // VMA <- M[k+1], start a read; forty fillers; A[park] <- MD.
        prom[at] = Insn::new(ALU | SETM | m_src(k as u64 + 1) | a_src(3) | START_READ);
        prom[at + 41] = Insn::new(ALU | SETM | SRC_MD | a_src(3) | a_dest(*park));
        at += 42;
    }
    m.load_prom(&prom);
    m
}

fn runs_the_four_reads(e: &mut dyn Engine, name: &str) {
    for _ in 0..200 {
        e.step().unwrap();
    }
    let m = e.machine();
    assert_eq!(m.bus_error, 0, "{name}: every read was answered");
    assert_eq!(m.amem[0o101], 0o123456, "{name}: A-LOW, the A bus under the filler");
    assert_eq!(m.amem[0o102], 0, "{name}: STAT-LOW");
    assert_eq!(m.amem[0o103], spy::OPEN_READ as u32, "{name}: register 3, the open bus");
    assert_eq!(m.amem[0o104], RUNNING as u32, "{name}: FLAG-1, running and waiting for nothing");
}

#[test]
fn a_microcode_read_of_the_block_returns_the_cpus_own_registers() {
    let mut r = Rtl::new(unibus_read_machine());
    r.boot();
    runs_the_four_reads(&mut r, "rtl");
    assert_eq!(r.bus_cycles(), 4);

    let mut u = Micro::new(unibus_read_machine());
    u.boot();
    runs_the_four_reads(&mut u, "micro");
}
