// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The I/O board as a netlist: `data/CADRIO.netlist`, the fourteen `cadrio`
//! pages `iob.stf` lists for the board proper, through
//! `tools/cadrio-netlist.sh` and MIT's wire list for the board.
//!
//! It is a Unibus slave: the keyboard, the mouse, the clocks and the serial
//! port at `0o764100`, which microcode 323 reads before it does anything
//! else. The Chaosnet interface shares the board, on thirteen more pages in
//! `chaos/`; one of them, the transmit clock, is the I/O pages' clock, and
//! all thirteen are taken with them.

use std::collections::BTreeSet;

use muir::chip::Chip;
use muir::netlist::{self, Netlist};
use muir::part::Level;
use muir::wirelist;

mod support;
use support::quiet;

const CADRIO: &str = include_str!("../data/CADRIO.netlist");

fn cadrio() -> Netlist {
    netlist::parse(CADRIO).unwrap()
}

/// The shape of the board, pinned the way the other netlists are. It is
/// **both halves**: the fourteen `cadrio` pages of the I/O board and the
/// thirteen `chaos/lispm` pages of the Chaosnet interface that shares it,
/// which is what `iob.stf` and `iob.wlr` both index. Two of the
/// twenty-seven carry no parts --- the spare gates and the jumpers.
#[test]
fn parses_to_the_expected_shape() {
    let n = cadrio();
    let pages: BTreeSet<&str> = n.parts.iter().map(|p| p.page.as_str()).collect();
    assert_eq!(pages.len(), 25, "{pages:?}");
    assert_eq!(n.parts.len(), 308);
    let chaos = pages.iter().filter(|p| p.starts_with("LM")).count();
    assert_eq!(chaos, 13, "the Chaosnet half is all there");
}

/// Every part on the board is identified, and everything that computes
/// anything has a behaviour, the oscillators, the one-shots and the delay
/// lines apart, which `src/chip.rs` runs. The 2651's is the serial port
/// itself, held to its sheet in `tests/serial_cable.rs`.
#[test]
fn every_part_is_identified() {
    let k = support::kinds(&cadrio());
    assert!(k.unknown.is_empty(), "no pinout for {:?}", k.unknown);
    assert_eq!(
        k.silent,
        ["26S02", "DIPOSC", "TD100", "TD100NC", "TD250", "TD25NC"],
        "parts with no behaviour"
    );
}

/// Two totem-pole outputs on one net is an electrical fault, so a wrongly
/// claimed output pin surfaces here.
#[test]
fn no_net_has_two_push_pull_drivers() {
    support::no_net_has_two_push_pull_drivers(&cadrio());
}

/// **MIT's wire list for the board agrees with the netlist.** `cadrio/iob.wlr`
/// (11 August 1981) is the list the board was wired from, pin by pin, and
/// reached us by a different route from the drawings; the script has
/// already put each wire the drawings label twice on one net under the
/// list's name. Every wire on the twenty-seven pages must be one net and
/// no net two wires.
///
/// **It holds over the whole board, both halves, with no correction.**
/// That is the check that the Chaosnet pages are the right revision of the
/// drawings. There is more than one copy of them on the tapes; built from an
/// earlier revision the same comparison gives 39 split wires and 14 doubled
/// nets, so a correction-free run is what says the right ones were read.
#[test]
fn matches_mits_wire_list() {
    let n = netlist::parse_wired(CADRIO).unwrap();
    let signals = support::wire_list(&n, &["cadrio", "iob.wlr"]);
    let r = wirelist::compare(&n, &signals, |slot| format!("0{slot}"));
    support::report(&signals, &r);
    assert!(r.placed > 1900, "the wire list was read");
    assert!(r.unplaced < 200, "most of the list lands on the board: {} did not", r.unplaced);
    assert!(
        r.missing.is_empty(),
        "the wire list places pins the netlist has not got: {:?}",
        r.missing
    );
    assert!(r.split.is_empty() && r.merged.is_empty(), "the wire list disagrees with the netlist");
}

/// The board's Unibus pins, as its own drawings name them: the bus-side
/// nets end in `*`.
fn unibus_wires() -> Vec<String> {
    let mut w: Vec<String> = (1..18).map(|b| format!("-A{b}*")).collect();
    w.extend((0..16).map(|b| format!("-D{b}*")));
    w.extend(
        [
            "-C1*", "-MSYN*", "-SSYN*", "-BBSY*", "-INIT*", "-INTR*", "-SACK*", "-BR*", "BG.IN*",
            "BG.OUT*", "-BOOT*",
        ]
        .map(String::from),
    );
    w
}

/// **The board comes up with its bus held up.** Powered, its Unibus pins
/// pulled up as the terminators hold them, its off-board inputs held as
/// the far end holds them, reset over `-INIT*` and left for a few
/// microseconds, no net is unknown.
///
/// The Chaosnet half made that second clause necessary. Its receiver at
/// LMLNDR A01 is a differential one, and a differential receiver with
/// nothing across its inputs has no difference to read: `src/part.rs`
/// answers with an unknown rather than a guess, and it spreads through
/// `-RCVR.DATA.IN`, `TTL.D.IN` and `COLLISION` to ten nets. An idle cable
/// is what the board actually sits on, and [`muir::unibus::IDLE_CHAOSNET`]
/// is what the far end holds it at.
#[test]
fn the_board_settles_with_its_bus_driven() {
    let n = cadrio();
    let mut c = Chip::new_unclocked(&n);
    c.power_on();
    let find = |name: &str| n.by_name_id(name).or_else(|| n.by_name_id(&format!("'{name}'")));
    for (name, level) in quiet() {
        c.drive(find(name).unwrap_or_else(|| panic!("no net {name}")), level);
    }
    let mut missing = Vec::new();
    for w in unibus_wires() {
        match find(&w) {
            Some(net) => c.pull_up(net),
            None => missing.push(w),
        }
    }
    eprintln!("bus wires the board has not got: {missing:?}");
    // The grant in is the arbiter's line, low for no grant, which the
    // board terminates to ground.
    c.drive(find("BG.IN*").expect("BG.IN*"), Level::Low);
    let init = find("-INIT*").expect("-INIT*");
    c.drive(init, Level::Low);
    c.settle_all();
    c.transition(0);
    c.pull_up(init);
    let mut now = 0;
    while now < 5_000 {
        now = c.next_tap().map_or(now + 100, |t| t.min(now + 100));
        c.transition(now);
    }
    let wired: BTreeSet<netlist::NetId> =
        n.parts.iter().flat_map(|p| p.pins.iter().map(|&(_, net)| net)).collect();
    // `NC#n` is what the reader calls a pin the drawing marks unconnected;
    // one per pin, joined to nothing. The Am26LS31's three unused drivers
    // at LMLNDR A02 have their outputs so marked, and what an unused
    // driver puts on a pin that goes nowhere is not a fact about the
    // board.
    let unknown: Vec<&str> = wired
        .iter()
        .filter(|&&id| c.net(id) == Level::X)
        .map(|&id| n.net(id))
        .filter(|name| !name.starts_with("NC#"))
        .collect();
    eprintln!("{} nets unknown after 5 µs: {unknown:?}", unknown.len());
    assert!(unknown.is_empty(), "nets at unknown with the bus driven: {unknown:?}");
}

/// **The board answers its registers on the Unibus.** Driven by a master on
/// its own, reset and left a while, it is read at each of the addresses
/// microcode 323 uses, and what it says is held against the behavioural
/// board in `src/ioboard.rs`, which was written from MIT's sources for the
/// same registers; and the status register takes its four enables back.
#[test]
fn the_registers_answer_on_the_unibus() {
    use muir::ioboard::{self, IoBoard};
    use muir::unibus::UnibusMaster;
    let n = cadrio();
    let mut b = UnibusMaster::new(&n, 10_000, &quiet());
    let mut model = IoBoard::default();
    let mut times = Vec::new();
    for (name, uaddr) in [
        ("CSR", ioboard::CSR),
        ("KBD LOW", ioboard::KBD_LOW),
        ("KBD HIGH", ioboard::KBD_HIGH),
        ("MOUSE Y", ioboard::MOUSE_Y),
        ("MOUSE X", ioboard::MOUSE_X),
        ("CLOCK", ioboard::CLOCK),
        ("GPIO", ioboard::GPIO),
    ] {
        let (took, word) = b.cycle(uaddr, None);
        let want = model.read(uaddr, b.now);
        eprintln!("{name:>9} {uaddr:o}: {word:#08o} in {took} ns; the model says {want:#08o}");
        times.push((name, took, word, want));
    }
    for &(name, _, word, want) in &times {
        assert_eq!(word, want, "{name}: the board against the behavioural model");
    }
    // A second read of each, for the timing once the board is warm.
    for (name, uaddr) in
        [("CSR", ioboard::CSR), ("KBD LOW", ioboard::KBD_LOW), ("CLOCK", ioboard::CLOCK)]
    {
        let (took, word) = b.cycle(uaddr, None);
        eprintln!("{name:>9} again: {word:#08o} in {took} ns");
    }
    // The microsecond clock: read the low half, then the high; the board
    // counts its own microseconds from reset. Twice, ten microseconds
    // apart, to place its zero.
    let (t_low, low) = b.cycle(ioboard::USEC_LOW, None);
    let at_low = b.now - t_low - 100 - 200;
    let (t_high, high) = b.cycle(ioboard::USEC_HIGH, None);
    eprintln!(
        "usec clock: {high:#08o}:{low:#08o} latched at about {at_low} ns ({t_low}, {t_high} ns to answer)"
    );
    b.run(b.now + 10_000);
    let (t_low2, low2) = b.cycle(ioboard::USEC_LOW, None);
    let at_low2 = b.now - t_low2 - 100 - 200;
    eprintln!(
        "usec clock: {low2:#08o} latched at about {at_low2} ns; {} µs for {} ns",
        low2 - low,
        at_low2 - at_low
    );
    // The four enables, written and read back.
    b.cycle(ioboard::CSR, Some(0o17));
    model.write(ioboard::CSR, 0o17, 0);
    let (_, csr) = b.cycle(muir::ioboard::CSR, None);
    eprintln!(
        "CSR after writing 17: {csr:#08o}; the model says {:#08o}",
        model.read(ioboard::CSR, b.now)
    );
    assert_eq!(csr, model.read(ioboard::CSR, b.now), "the enables written and read back");
    b.cycle(ioboard::CSR, Some(0));
    model.write(ioboard::CSR, 0, 0);
    let (_, csr) = b.cycle(muir::ioboard::CSR, None);
    assert_eq!(csr, model.read(ioboard::CSR, b.now), "the enables cleared again");
    // The microsecond counter against the model's clock at the instant
    // the board latched it.
    assert_eq!(low as u64, at_low / 1_000, "the microsecond counter counts from reset");
}

/// **How long the board takes, by register and by phase.** The clocks, the
/// GPIO and the microsecond counter answer straight through the TD250; the
/// keyboard, mouse and status registers go through a flop on the
/// microsecond clock, so their answer depends on where in that microsecond
/// the request lands. Printed for `rtl`'s twin to be written from.
#[test]
#[ignore = "a report, not a check: --ignored --nocapture"]
fn the_answer_time_by_register_and_phase() {
    use muir::ioboard;
    use muir::unibus::UnibusMaster;
    let n = cadrio();
    let mut b = UnibusMaster::new(&n, 10_000, &quiet());
    let usec = b.net("'1 USEC CLK'");
    // The microsecond clock's rising edges, as a reference for the phase.
    let mut last = b.chip.net(usec);
    let mut edge = 0;
    for _ in 0..2_000 {
        b.run(b.now + 1);
        let l = b.chip.net(usec);
        if last == Level::Low && l == Level::High {
            edge = b.now;
        }
        last = l;
    }
    eprintln!("a 1 USEC CLK rising edge at {edge}; period measured next");
    let mut edges = Vec::new();
    let mut last = b.chip.net(usec);
    for _ in 0..3_000 {
        b.run(b.now + 1);
        let l = b.chip.net(usec);
        if last == Level::Low && l == Level::High {
            edges.push(b.now);
        }
        last = l;
    }
    let periods: Vec<u64> = edges.windows(2).map(|w| w[1] - w[0]).collect();
    eprintln!(
        "1 USEC CLK rising edges {} apart: {:?}",
        periods.first().unwrap_or(&0),
        &periods[..periods.len().min(4)]
    );
    let period = periods[0];
    let mut by_phase = Vec::new();
    for k in 0..10u64 {
        // Start the cycle k tenths of a period after an edge.
        let next_edge =
            *edges.last().unwrap() + period * ((b.now - edges.last().unwrap()) / period + 2);
        b.run(next_edge + k * period / 10);
        let (took, _) = b.cycle(ioboard::CSR, None);
        by_phase.push((k * period / 10, took));
    }
    eprintln!("CSR answered, by ns after a 1 USEC CLK edge at MSYN: {by_phase:?}");
    for (name, uaddr) in [
        ("CLOCK", ioboard::CLOCK),
        ("GPIO", ioboard::GPIO),
        ("USEC LOW", ioboard::USEC_LOW),
        ("USEC HIGH", ioboard::USEC_HIGH),
        ("KBD LOW", ioboard::KBD_LOW),
        ("MOUSE X", ioboard::MOUSE_X),
        ("BEEP", ioboard::BEEP),
    ] {
        let (took, word) = b.cycle(uaddr, None);
        eprintln!("{name:>9}: {word:#08o} in {took} ns");
    }
    // The status register's writable bits.
    for w in [0u16, 0o17, 0o200, 0o377, 0] {
        b.cycle(ioboard::CSR, Some(w));
        let (_, csr) = b.cycle(muir::ioboard::CSR, None);
        eprintln!("CSR after writing {w:#o}: {csr:#08o}");
    }
}

/// **The answer rule, to the nanosecond.** For `rtl`'s twin: the status
/// register's answer against where `-MSYN*` falls relative to the
/// microsecond clock, at 20 ns steps; the microsecond clock's first edge
/// after the reset; and the straight registers' answer.
#[test]
#[ignore = "a report, not a check: --ignored --nocapture"]
fn the_answer_rule_for_the_twin() {
    use muir::ioboard;
    use muir::unibus::UnibusMaster;
    let n = cadrio();
    let mut b = UnibusMaster::new(&n, 0, &quiet());
    let usec = b.net("'1 USEC CLK'");
    // From the reset's release at 0: the first rising edges of the
    // microsecond clock.
    let mut edges = Vec::new();
    let mut last = b.chip.net(usec);
    while b.now < 6_000 {
        b.run(b.now + 1);
        let l = b.chip.net(usec);
        if last == Level::Low && l == Level::High {
            edges.push(b.now);
        }
        last = l;
    }
    eprintln!("1 USEC CLK rising edges after the reset's release at 0: {edges:?}");
    let period = edges[2] - edges[1];
    let phase0 = edges[1] % period;
    // The status register at every 20 ns of a period.
    let mut rule = Vec::new();
    for k in 0..50u64 {
        let target = (b.now / period + 3) * period + phase0 + k * 20;
        b.run(target);
        let msyn_at = b.now + 100;
        let (took, _) = b.cycle(ioboard::CSR, None);
        rule.push((k * 20, msyn_at + took - (msyn_at / period * period + phase0)));
    }
    eprintln!(
        "CSR: (ns of MSYN past a 1 USEC CLK edge, ns of the answer past that edge): {rule:?}"
    );
    // Every register, read and written, at five phases: the answer past
    // the edge before MSYN, so that a register on the microsecond clock
    // shows a constant past the next edge and a straight one a constant
    // past MSYN.
    for (name, uaddr, write) in [
        ("CSR", ioboard::CSR, None),
        ("KBD LOW", ioboard::KBD_LOW, None),
        ("KBD HIGH", ioboard::KBD_HIGH, None),
        ("MOUSE Y", ioboard::MOUSE_Y, None),
        ("MOUSE X", ioboard::MOUSE_X, None),
        ("BEEP", ioboard::BEEP, None),
        ("CLOCK", ioboard::CLOCK, None),
        ("GPIO", ioboard::GPIO, None),
        ("USEC LOW", ioboard::USEC_LOW, None),
        ("USEC HIGH", ioboard::USEC_HIGH, None),
        ("CSR w", ioboard::CSR, Some(0)),
        ("CLOCK w", ioboard::CLOCK, Some(1)),
        ("BEEP w", ioboard::BEEP, Some(0)),
        ("GPIO w", ioboard::GPIO, Some(0)),
    ] {
        let mut by_phase = Vec::new();
        for k in 0..5u64 {
            let target = (b.now / period + 3) * period + phase0 + k * 200;
            b.run(target);
            let msyn_at = b.now + 100;
            let (took, _) = b.cycle(uaddr, write);
            let edge_before = msyn_at - ((msyn_at - phase0) % period);
            by_phase.push((msyn_at - edge_before, took, msyn_at + took - edge_before));
        }
        eprintln!("{name:>9}: (MSYN past edge, took, answer past edge) {by_phase:?}");
    }
    // The setup: MSYN at 2 ns steps around the edge.
    let mut fine = Vec::new();
    for k in 0..12u64 {
        let target = (b.now / period + 3) * period + phase0 + 980 + k * 2 - 100;
        b.run(target);
        let msyn_at = b.now + 100;
        let (took, _) = b.cycle(ioboard::CSR, None);
        let edge = (b.now / period) * period + phase0;
        fine.push((msyn_at as i64 - (edge as i64), took));
    }
    eprintln!("CSR near the edge: (MSYN minus the edge, took) {fine:?}");
}

/// **The microsecond clock runs free of the reset.** `1 USEC CLK` is the
/// 74S163 at IOBCLK 0C21 counting `MCLK^` with its clear and load tied
/// high, and `MCLK^` is the 74S112 at LMTCLK 0A06 with preset and clear
/// tied high: nothing on the Unibus touches either. A reset held for a
/// microsecond and released at every phase of the crystal leaves the
/// first edge after it where it was going to be, 890 ns after power-on
/// and every 1,000 ns from there. The twin in `src/busint.rs` counts
/// from power-on for that reason; it counted from the reset's release
/// until this was measured, and the far end parted at 1,410,551.
#[test]
fn the_microsecond_clock_runs_free_of_the_reset() {
    let n = cadrio();
    let usec = n.by_name_id("'1 USEC CLK'").expect("1 USEC CLK");
    for d in (0..130).step_by(5) {
        let mut m = muir::unibus::UnibusMaster::new(&n, 2_000, &quiet());
        let init = m.net("-INIT*");
        m.chip.drive(init, Level::Low);
        m.run(3_000);
        let release = 3_000 + d;
        m.run(release);
        m.chip.pull_up(init);
        m.chip.transition(m.now);
        let mut was = m.chip.net(usec);
        let mut first = None;
        let mut t = release;
        while first.is_none() && t < release + 3_000 {
            t += 1;
            m.run(t);
            let now = m.chip.net(usec);
            if was == Level::Low && now == Level::High {
                first = Some(t);
            }
            was = now;
        }
        assert_eq!(
            first,
            Some(3_890),
            "released at {release}: the first edge after is not where power-on put it"
        );
    }
}

/// **A keyboard with nothing typed never makes `KBD READY`.** Held as
/// `IDLE_KEYBOARD` holds it, the board runs half a millisecond --- sixty
/// of its own 125 kHz keyboard clocks, two and a half words' worth ---
/// and the status register still has bit 5 clear.
#[test]
fn an_idle_keyboard_never_reads_ready() {
    use muir::unibus::UnibusMaster;
    let n = cadrio();
    let mut b = UnibusMaster::new(&n, 500_000, &quiet());
    assert_eq!(b.chip.net(b.net("'KBD READY'")), Level::Low, "KBD READY on the board");
    let (_, csr) = b.cycle(muir::ioboard::CSR, None);
    assert_eq!(csr & muir::ioboard::csr::KBD_READY, 0, "the status register reads {csr:o}");
}

/// **The far end's board is reset at power-on, and a board that was not
/// reads a key.** `Unibus::new` resets the board over `-INIT*` as the
/// memory boards are over `-XBUS INIT`: run 300 µs from power-on, its
/// `KBD READY` is clear. The same board powered on without the reset has
/// the keyboard receiver's busy flop, the 74LS109 at IOBKBD 0C26, up from
/// power-on, shifts twenty-four bits off the idle line, and has `KBD
/// READY` up 196 µs later: what the far end showed at the microcode's
/// first read of the board, which boots warm on it.
#[test]
fn the_far_ends_board_is_reset_at_power_on() {
    use muir::unibus::Unibus;
    let n = cadrio();
    let bus_n = netlist::parse(include_str!("../data/BUSINT.netlist")).unwrap();
    let mut u = Unibus::new(&bus_n, &n, 0, 0o3050);
    for t in (0..300_000).step_by(1_000) {
        u.transition_due(t);
    }
    let ready = n.by_name_id("'KBD READY'").unwrap();
    assert_eq!(u.board.net(ready), Level::Low, "KBD READY on the far end's board at 300 µs");

    // The same board, never reset.
    let mut c = Chip::new_unclocked(&n);
    c.power_on();
    for name in muir::unibus::wire_names() {
        if let Some(net) = n.by_name_id(&name) {
            c.pull_up(net);
        }
    }
    for &(name, level) in quiet().iter() {
        c.drive(n.by_name_id(name).or_else(|| n.by_name_id(&format!("'{name}'"))).unwrap(), level);
    }
    c.settle_all();
    c.transition(0);
    let mut first = None;
    while let Some(t) = c.next_tap()
        && t <= 300_000
    {
        c.transition(t);
        if first.is_none() && c.net(ready) == Level::High {
            first = Some(t);
        }
    }
    assert_eq!(first, Some(195_890), "when KBD READY rose on the board that was not reset");
}

/// **The board lets go of the data lines after a write and after a read.**
/// A Unibus device drives the data lines only while answering a read; the
/// far end found the lines held low at rest before a read of the
/// interface's own register at 2,087,325, and this is the board's side of
/// that question.
#[test]
fn the_board_lets_go_of_the_data_lines() {
    use muir::unibus::UnibusMaster;
    let n = cadrio();
    let mut b = UnibusMaster::new(&n, 10_000, &quiet());
    let d0 = b.net("-D0*");
    let d15 = b.net("-D15*");
    let strong = |b: &UnibusMaster| (b.chip.board_level(d0), b.chip.board_level(d15));
    for name in [
        "-DRIVE.UNIBUS",
        "UBO0",
        "READ",
        "'BOARD.SELECT FOR REAL'",
        "UB>TSR",
        "SSYN.OK",
        "SELECTED",
        "-BOARD.SELECT",
        "'-SER RRDY'",
        "SER.IREQ",
    ] {
        if let Some(id) = n.by_name_id(name) {
            eprintln!("  {name} = {:?}", b.chip.net(id));
        }
    }
    eprintln!("at rest: {:?}", strong(&b));
    assert!(
        !strong(&b).0.1 && !strong(&b).1.1,
        "the board drives the data lines at rest: {:?}",
        strong(&b)
    );
    let _ = b.cycle(muir::ioboard::CSR, Some(0o17));
    b.run(b.now + 2_000);
    eprintln!("after a write: {:?}", strong(&b));
    assert!(
        !strong(&b).0.1 && !strong(&b).1.1,
        "the board drives the data lines after a write: {:?}",
        strong(&b)
    );
    let _ = b.cycle(muir::ioboard::CSR, None);
    b.run(b.now + 2_000);
    eprintln!("after a read: {:?}", strong(&b));
    assert!(
        !strong(&b).0.1 && !strong(&b).1.1,
        "the board drives the data lines after a read: {:?}",
        strong(&b)
    );
}

/// What arrives from off the board: every net some part reads and no
/// part drives --- the Unibus wires the far end pulls up, and the keyboard,
/// mouse and serial lines the outside world drives. The far end has to
/// hold each at its idle level, since a floating TTL input reads high, and
/// a stray high on `UB>TSR` once had the board driving the Unibus data
/// lines at rest.
#[test]
#[ignore = "a report, not a check: --ignored --nocapture"]
fn report_what_arrives_from_off_the_board() {
    support::report_undriven_nets(&cadrio());
}

/// **Every address of the board's block, read and written, against the
/// model: who answers, with what, and when.**  A fresh netlist board and
/// a fresh model for every even address from `764076` to `764200`, and
/// `760100`, each cycle at the same instant on both: the board's decoder
/// as `ioboard::answers` has it --- the groups on `A<6:4>`, the clocks and
/// the Chaosnet ignoring `A3` but for START and the disabled buffer read,
/// writes taken only where a register takes them, the serial port's
/// eight answered with nothing behind them --- the word read the same
/// (but for the Chaosnet receive buffer, whose RAM reads its power-on
/// contents on the board), and `-SSYN` at the same nanosecond after
/// `-MSYN` by `IoBoardTiming::answer`: the keyboard group's two stages,
/// the microsecond counter's edge, the transmit buffer's and START's 350,
/// the receive buffer's `FCLK^` edge, the serial port's half-microsecond
/// edges.
#[test]
fn the_board_and_the_model_decode_the_block_alike() {
    use muir::busint::{IoBoardTiming, UNIBUS_ADDRESS_NS};
    use muir::chaos::interface as chaos;
    use muir::ioboard::{self, IoBoard};
    use muir::unibus::UnibusMaster;
    let n = cadrio();
    let timing = IoBoardTiming::default();
    let mut mismatches = Vec::new();
    let mut answered = 0;
    for write in [false, true] {
        for uaddr in (0o764076u32..=0o764200).step_by(2).chain([0o760100]) {
            let mut b = UnibusMaster::new(&n, 10_000, &quiet());
            // The switches open on this board, so the model's are too.
            let mut m = IoBoard::default();
            m.plug_chaos(0o177777, None, 0, false);
            b.run(12_000);
            let msyn = b.now + UNIBUS_ADDRESS_NS;
            let want = ioboard::answers(uaddr, write);
            let got = b.try_cycle(uaddr, write.then_some(0o1234));
            let what = if write { "write" } else { "read" };
            match (got, want) {
                (None, None) => eprintln!("{uaddr:o} {what}: unanswered on both"),
                (Some((took, word)), Some(r)) => {
                    answered += 1;
                    let at = timing.answer(r, write, msyn) - msyn;
                    let model_word = if write {
                        m.write(r, 0o1234, msyn);
                        None
                    } else {
                        Some(m.read(r, msyn))
                    };
                    eprintln!(
                        "{uaddr:o} {what}: {r:o} on the model; the board {word:#o} in {took} ns, the model {} in {at} ns",
                        model_word.map_or("-".to_string(), |w| format!("{w:#o}"))
                    );
                    if at != took {
                        mismatches.push(format!(
                            "{uaddr:o} {what}: the board answers in {took} ns, the model in {at}"
                        ));
                    }
                    if let Some(mw) = model_word
                        && r != chaos::READ_BUFFER
                        && mw != word
                    {
                        mismatches.push(format!(
                            "{uaddr:o} read: the board {word:#o}, the model {mw:#o}"
                        ));
                    }
                }
                (Some((took, _)), None) => mismatches.push(format!(
                    "{uaddr:o} {what}: the board answers in {took} ns, the model not at all"
                )),
                (None, Some(r)) => mismatches.push(format!(
                    "{uaddr:o} {what}: the model answers as {r:o}, the board not at all"
                )),
            }
        }
    }
    assert!(answered > 40, "{answered} cycles answered");
    assert!(mismatches.is_empty(), "the board against the model:\n{}", mismatches.join("\n"));
}
