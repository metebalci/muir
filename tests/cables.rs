// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The processor board and the bus interface board on the five cables, both
//! as netlists.
//!
//! Every test here resumes the boot from `vendor/run/chk/at-535000.chk`,
//! a checkpoint of `chip_agrees_with_rtl` taken just short of the boot's
//! first bus cycle, which nothing fetches: it is written by a run of ten
//! minutes or so, and the tests are ignored until it is there.

mod support;

use muir::cable::{Boards, FarEnd};
use muir::chip::Chip;
use muir::clock::{Behavioural, Clock};
use muir::machine::Machine;
use muir::netlist;
use muir::part::Level;

const CPU: &str = include_str!("../data/CADR.netlist");
const BUSINT: &str = include_str!("../data/BUSINT.netlist");
const CADRM: &str = include_str!("../data/CADRM.netlist");
const CADRIO: &str = include_str!("../data/CADRIO.netlist");

/// A net of either board by name. The bus interface's netlist stores a
/// name with spaces in quotes, `'UB NXM ERROR'`, so a name is looked up
/// bare and then quoted.
fn net_named(n: &netlist::Netlist, name: &str) -> netlist::NetId {
    n.by_name_id(name)
        .or_else(|| n.by_name_id(&format!("'{name}'")))
        .unwrap_or_else(|| panic!("no net {name}"))
}

/// The processor as a checkpoint holds it, with its clock and the
/// microcycle it was taken at; `None`, with the skip line and how to make
/// the file, when the checkpoint is not there.
///
/// **Only the front of the file is read.** A `chip` checkpoint is the
/// processor, its clock and then the whole far end --- boards, buses and
/// the machine behind them, [`muir::cable::write_checkpoint`] --- and the
/// far end each test here builds is not that one: they run 32 memory
/// boards, or 2, or 4 with an I/O board, against a fresh
/// [`Machine`]. The body is a stream, so this takes the two pieces it
/// wants and stops rather than building a backplane to match.
///
/// `at-535000.chk` is from before the boot's first Unibus cycle, so a
/// board built fresh here is the right one: not yet Unibus master, its
/// error register clear.
fn resume(label: &str) -> Option<(Chip, Behavioural, u64)> {
    let Some(p) = support::vendor(&["run", "chk", &format!("{label}.chk")]) else {
        eprintln!("  {MAKES_IT}");
        return None;
    };
    let file = muir::checkpoint::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let mut it =
        muir::cable::read_checkpoint(&file).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let n = netlist::parse(CPU).unwrap();
    // As `tests/chip.rs` builds the board before loading into it: powered,
    // with the boot PROM, settled.
    let image: Vec<u64> = muir::prom::boot_prom_image();
    let mut c = Chip::new(&n);
    c.power_on();
    c.load_prom(&n, &image);
    c.settle();
    it.processor(&mut c).unwrap_or_else(|e| panic!("{}: the processor: {e}", p.display()));
    let clk = it.clock().unwrap_or_else(|e| panic!("{}: the clock: {e}", p.display()));
    Some((c, clk, it.ran))
}

/// What writes `vendor/run/chk/at-535000.chk`, which no fetch produces:
/// an ordinary run of the binary, with the System 100 pack fetched, in
/// about five minutes. `--tv model` because that is the one board this
/// harness and `muir --chip` differ over, and a checkpoint carries which
/// boards were on the backplane.
const MAKES_IT: &str = "`cargo run --release -- --chip --tv model --disk-pack \
     vendor/run/disk-sys-100-0.img,ro --stop-after 535000 --checkpoint \
     vendor/run/chk/at-535000.chk` writes it, in about five minutes";

/// **The boot PROM's first bus cycle crosses the cables.**
///
/// Resumed just before `PAGE-0-PARITY-FIX`, whose first bus cycle is the
/// boot's first: a read of main memory at address 0, which the far end
/// answers from `Machine`. What is watched: the request on the processor's
/// side, its arrival at the interface as an Xbus request, the far end's
/// acknowledgement, `-LOADMD` and `-MEMACK` back on the cables. With
/// `MUIR_TRACE_WRITE` set, a read's acknowledgement is passed over and the
/// watch goes on to the first write's --- the writes of `PAGE-0-PARITY-FIX`
/// --- printed the same way; what is asserted, that a request was made and
/// acknowledged within the window, is the same.
#[test]
#[ignore = "needs vendor/run/chk/at-535000.chk, which no fetch produces: `cargo run --release \
            -- --chip --tv model --disk-pack vendor/run/disk-sys-100-0.img,ro --stop-after 535000 \
            --checkpoint vendor/run/chk/at-535000.chk` writes it, in about five minutes, with the \
            System 100 pack fetched"]
fn the_first_bus_cycles_cross_the_cables() {
    let cpu_n = netlist::parse(CPU).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    let Some((mut cpu, mut clk, at)) = resume("at-535000") else { return };
    eprintln!("resumed at microcycle {at}, {} ns", clk.time_ns());
    let mem_n = netlist::parse(CADRM).unwrap();
    let mut far =
        FarEnd::new(&cpu_n, &bus_n, &mem_n, Boards::memory(32), clk.time_ns(), Machine::new());
    // One wire of `data/cables.txt` is not joined: `-LM BOOT`, which the
    // bus interface only passes through, so its anchor names no pin there.
    assert!(
        matches!(far.cables.missing.as_slice(), [wire] if wire.contains("-LM BOOT")),
        "unjoined: {:?}",
        far.cables.missing
    );
    far.join(&mut cpu, clk.time_ns());

    let cpu_net = |name: &str| net_named(&cpu_n, name);
    let bus_net = |name: &str| net_named(&bus_n, name);
    let pc = cpu.bus_nets(&cpu_n, "PC", 14);
    let watch: Vec<(&str, bool, muir::netlist::NetId)> = vec![
        ("cpu MEMRQ", true, cpu_net("MEMRQ")),
        ("cpu -MEMACK", true, cpu_net("-MEMACK")),
        ("cpu -MEMGRANT", true, cpu_net("-MEMGRANT")),
        ("cpu -DBWRITE", true, cpu_net("-DBWRITE")),
        ("cpu -LDMODE", true, cpu_net("-LDMODE")),
        ("busint -MEMRQ", false, bus_net("-MEMRQ")),
        ("busint ADR=UNIBUS", false, bus_net("ADR=UNIBUS")),
        ("busint -LMUB GRANT", false, bus_net("-LMUB GRANT")),
        ("busint -UB MSYN", false, bus_net("-UB MSYN")),
        ("busint -UB SSYN", false, bus_net("-UB SSYN")),
        ("busint -SELECT SPY", false, bus_net("-SELECT SPY")),
        ("busint -SPY WRITE", false, bus_net("-SPY WRITE")),
        ("busint -LMACK", false, bus_net("-LMACK")),
        ("busint NXM TIMEOUT", false, bus_net("NXM TIMEOUT")),
        ("busint -XBUS RQ", false, bus_net("-XBUS RQ")),
        ("busint -XBUS ACK", false, bus_net("-XBUS ACK")),
        ("cpu WRCYC", true, cpu_net("WRCYC")),
        ("busint WRCYC", false, bus_net("WRCYC")),
        ("busint XWR", false, bus_net("XWR")),
        ("busint -XBUS WR", false, bus_net("-XBUS WR")),
        ("busint -XDRIVE", false, bus_net("-XDRIVE")),
        ("busint WRITE THROUGH", false, bus_net("WRITE THROUGH")),
        ("busint -WBUFWE", false, bus_net("-WBUFWE")),
        ("busint BUS READY", false, bus_net("BUS READY")),
        ("busint LMX GRANT", false, bus_net("LMX GRANT")),
        ("busint XBUS REQUEST", false, bus_net("XBUS REQUEST")),
        ("cpu -LOADMD", true, cpu_net("-LOADMD")),
        ("cpu MBUSY", true, cpu_net("MBUSY")),
        ("cpu -PMA21", true, cpu_net("-PMA21")),
        ("cpu -VMA0", true, cpu_net("-VMA0")),
        ("busint -ADR21", false, bus_net("-ADR21")),
        ("busint -ADR0", false, bus_net("-ADR0")),
    ];
    let cpu_adr: Vec<muir::netlist::NetId> = (8..22)
        .map(|b| cpu_net(&format!("-PMA{b}")))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let cpu_vma: Vec<muir::netlist::NetId> = (0..8).map(|b| cpu_net(&format!("-VMA{b}"))).collect();
    let bus_adr: Vec<muir::netlist::NetId> =
        (0..22).map(|b| bus_net(&format!("-ADR{b}"))).collect();
    let level =
        |cpu: &Chip, busint: &Chip, (_, on_cpu, id): &(&str, bool, muir::netlist::NetId)| {
            if *on_cpu { cpu.net(*id) } else { busint.net(*id) }
        };
    let mut last: Vec<Level> = watch.iter().map(|w| level(&cpu, &far.board, w)).collect();
    let t0 = clk.time_ns();
    let (mut requested, mut acked): (Option<u64>, Option<u64>) = (None, None);
    let mut steps = 0;
    while acked.is_none() && steps < 400_000 {
        far.tick_with(&mut cpu, &mut clk);
        steps += 1;
        let now = clk.time_ns() - t0;
        for (k, w) in watch.iter().enumerate() {
            let l = level(&cpu, &far.board, w);
            if l != last[k] {
                eprintln!("  {now:>7} ns PC {:o}: {}={l:?}", cpu.read(&pc), w.0);
                last[k] = l;
            }
        }
        if requested.is_none()
            && cpu.net(cpu_net("MEMRQ")) == Level::High
            && far.board.net(bus_net("-MEMRQ")) == Level::Low
        {
            requested = Some(now);
            let pma: Vec<String> = cpu_adr
                .iter()
                .map(|&id| format!("{:?}/{:?}", cpu.net(id), cpu.board_level(id)))
                .collect();
            let vma: Vec<String> = cpu_vma
                .iter()
                .map(|&id| format!("{:?}/{:?}", cpu.net(id), cpu.board_level(id)))
                .collect();
            eprintln!("cpu -PMA21..8 {}", pma.join(" "));
            eprintln!("cpu -VMA7..0 {}", vma.join(" "));
            eprintln!(
                "busint -ADR21..0 {:?}",
                bus_adr.iter().rev().map(|&id| far.board.net(id)).collect::<Vec<_>>()
            );
        }
        if requested.is_some() && acked.is_none() && cpu.net(cpu_net("-MEMACK")) == Level::Low {
            if std::env::var("MUIR_TRACE_WRITE").is_ok() && cpu.net(cpu_net("WRCYC")) != Level::High
            {
                // Not this one: wait for a write.
                requested = None;
                last.fill(Level::X);
            } else {
                acked = Some(now);
            }
        }
    }
    let requested = requested.expect("the boot never asked for the bus");
    let acked = acked.expect("the request was never acknowledged");
    eprintln!(
        "requested at {requested} ns, acknowledged at {acked} ns, {} ns later",
        acked - requested
    );
}

/// The parity the bus interface writes with a word is the parity
/// `xbus::parity` stores when the memory boards are filled from behind:
/// after the boot's PAGE-0-PARITY-FIX has read and written back every word
/// of page 0 through the interface, every parity cell on the board is what
/// `parity` says of the word beside it.
#[test]
#[ignore = "needs vendor/run/chk/at-535000.chk, which no fetch produces: `cargo run --release \
            -- --chip --tv model --disk-pack vendor/run/disk-sys-100-0.img,ro --stop-after 535000 \
            --checkpoint vendor/run/chk/at-535000.chk` writes it, in about five minutes, with the \
            System 100 pack fetched"]
fn the_interface_writes_the_parity_the_boards_are_filled_with() {
    let cpu_n = netlist::parse(CPU).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    let Some((mut cpu, mut clk, at)) = resume("at-535000") else { return };
    let mem_n = netlist::parse(CADRM).unwrap();
    // Two boards are enough: page 0 is on the first.
    let mut far =
        FarEnd::new(&cpu_n, &bus_n, &mem_n, Boards::memory(2), clk.time_ns(), Machine::new());
    far.join(&mut cpu, clk.time_ns());
    // Words of both parities, and every parity cell wrong for half of them
    // whichever way the interface counts, so a written cell shows.
    let bit = far.xbus.drams[0][32];
    for a in 0..256u32 {
        far.xbus.poke(a, a);
        far.xbus.boards[0].set_cell_bit(bit, a as usize, false);
    }
    // The loop begins 1,301 microcycles in and its 256 turns take nine
    // microcycles each, 800 µs in all; 900 µs is that with room, and ends
    // before the first Unibus cycle at 537,841.
    let t0 = clk.time_ns();
    while clk.time_ns() - t0 < 900_000 {
        far.tick_with(&mut cpu, &mut clk);
    }
    eprintln!("ran from microcycle {at} for {} ns", clk.time_ns() - t0);
    let (mut written, mut agree) = (0, 0);
    for a in 0..256u32 {
        assert_eq!(far.xbus.peek(a), Some(a), "the word at {a} came back as it went");
        let stored = far.xbus.peek_parity(a).unwrap();
        if stored {
            written += 1;
        }
        if stored == muir::xbus::parity(a) {
            agree += 1;
        }
    }
    eprintln!("{written} of page 0's parity cells set by the loop, {agree} as `parity` has them");
    assert_eq!(written, 128, "the loop wrote every word of page 0, half of them with the bit set");
    assert_eq!(agree, 256, "the interface's parity is `parity`'s");
}

/// **The optimisations are on the same nets as the slow way.**
///
/// A memory board asleep on its own clock skips its oscillator edges
/// (`Chip::asleep`), and a board or the processor that has not moved since
/// the last exchange is not asked about its wires again
/// (`Chip::generation`). Stepped at every edge and asked about every wire
/// at every exchange instead, the slow way (`FarEnd::unoptimised`), every
/// board must be on the same nets at every step. Two far ends on the same
/// processor checkpoint run in lockstep through the boot's Unibus reset,
/// its parity loop and a few dozen refreshes, one optimised and one not,
/// with the I/O board on both, whose microsecond clock never lets it
/// sleep.
#[test]
#[ignore = "needs vendor/run/chk/at-535000.chk, which no fetch produces: `cargo run --release \
            -- --chip --tv model --disk-pack vendor/run/disk-sys-100-0.img,ro --stop-after 535000 \
            --checkpoint vendor/run/chk/at-535000.chk` writes it, in about five minutes, with the \
            System 100 pack fetched"]
fn the_optimisations_are_on_the_same_nets_as_the_slow_way() {
    let cpu_n = netlist::parse(CPU).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    let mem_n = netlist::parse(CADRM).unwrap();
    let io_n = netlist::parse(CADRIO).unwrap();
    let Some((mut cpu_a, mut clk_a, at)) = resume("at-535000") else { return };
    let (mut cpu_b, mut clk_b, _) = resume("at-535000").unwrap();
    let mut a = FarEnd::new(
        &cpu_n,
        &bus_n,
        &mem_n,
        Boards { memory: 4, io: Some(&io_n), ..Default::default() },
        clk_a.time_ns(),
        Machine::new(),
    );
    let mut b = FarEnd::new(
        &cpu_n,
        &bus_n,
        &mem_n,
        Boards { memory: 4, io: Some(&io_n), ..Default::default() },
        clk_b.time_ns(),
        Machine::new(),
    );
    b.unoptimised();
    a.join(&mut cpu_a, clk_a.time_ns());
    b.join(&mut cpu_b, clk_b.time_ns());
    // The optimised run's steps are a subset of the slow run's, so the slow
    // run is brought to each time the optimised run stops at, and both are
    // let finish everything due at that time before they are compared.
    let t0 = clk_a.time_ns();
    let mut steps = 0u64;
    while clk_a.time_ns() - t0 < 700_000 {
        a.tick_with(&mut cpu_a, &mut clk_a);
        steps += 1;
        let t = clk_a.time_ns();
        while a.next_event(&cpu_a, &clk_a).is_some_and(|e| e <= t) {
            a.tick_with(&mut cpu_a, &mut clk_a);
        }
        while clk_b.time_ns() < t || b.next_event(&cpu_b, &clk_b).is_some_and(|e| e <= t) {
            b.tick_with(&mut cpu_b, &mut clk_b);
        }
        assert_eq!(
            clk_b.time_ns(),
            t,
            "the slow run has no step at the optimised run's step {steps}"
        );
        // A sleeping board's own clock net is stale until it wakes; nothing
        // on the board reads it meanwhile, and every other net must agree.
        let same = |x: &Chip, y: &Chip, n: &netlist::Netlist, what: &str| {
            let clocks = x.oscillator_outputs();
            let differing: Vec<&str> = (0..x.nets().len())
                .filter(|&i| x.nets()[i] != y.nets()[i] && !clocks.contains(&(i as netlist::NetId)))
                .map(|i| n.net(i as netlist::NetId))
                .collect();
            assert!(
                differing.is_empty(),
                "{what} at {t} ns, step {steps}: the optimised and the slow run differ on {differing:?}"
            );
        };
        same(&cpu_a, &cpu_b, &cpu_n, "the processor");
        same(&a.board, &b.board, &bus_n, "the interface");
        for (k, (x, y)) in a.xbus.boards.iter().zip(&b.xbus.boards).enumerate() {
            same(x, y, &mem_n, &format!("memory board {k}"));
        }
        same(
            &a.unibus.as_ref().unwrap().board,
            &b.unibus.as_ref().unwrap().board,
            &io_n,
            "the I/O board",
        );
    }
    eprintln!(
        "from microcycle {at}, {steps} steps over {} ns: {} memory board and {} I/O board transitions optimised, {} and {} the slow way",
        clk_a.time_ns() - t0,
        a.xbus.transitions,
        a.unibus.as_ref().unwrap().transitions,
        b.xbus.transitions,
        b.unibus.as_ref().unwrap().transitions,
    );
    // The window is mostly bus traffic and the reset, which keep the boards
    // awake; the saving is in the band, where a board is idle for tens of
    // microseconds at a time.
    assert!(a.xbus.transitions < b.xbus.transitions, "the optimised run did less work");
}
