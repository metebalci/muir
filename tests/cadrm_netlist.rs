// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The memory board as a netlist: `data/CADRM.netlist`, MIT's `cadrm`
//! drawings through `tools/cadrm-netlist.sh`.
//!
//! MIT's wire list for the board, `cadrm/mem.wlr`, is the second copy,
//! one ECO apart (`matches_mits_wire_list`); beside it stand the
//! regularity of the drawings themselves --- 132 DRAMs that must be wired
//! the same way bank for bank --- and the board doing on its own what a
//! memory does: taking a word, giving it back, and refreshing itself.

use std::collections::BTreeSet;

use muir::busint::{rising_edge, rising_edge_after};
use muir::netlist::{self, Netlist};
use muir::part::Level;
use muir::wirelist;
use muir::xbus::XbusMaster;

mod support;

const CADRM: &str = include_str!("../data/CADRM.netlist");

fn cadrm() -> Netlist {
    netlist::parse(CADRM).unwrap()
}

/// The shape of the board, pinned the way the other two netlists are.
#[test]
fn parses_to_the_expected_shape() {
    let n = cadrm();
    let pages: BTreeSet<&str> = n.parts.iter().map(|p| p.page.as_str()).collect();
    assert_eq!(pages.len(), 15, "{pages:?}");
    assert_eq!(n.parts.len(), 196);
    let drams = n.parts.iter().filter(|p| p.kind == "4116VG").count();
    assert_eq!(drams, 132, "four banks of 33");
}

/// Every part on the board is identified, and everything that computes
/// anything has a behaviour. The exceptions are the two analog parts, the
/// DIP oscillator and the one-shot, which `src/chip.rs` runs as it runs
/// the bus interface's oscillator.
#[test]
fn every_part_is_identified() {
    let k = support::kinds(&cadrm());
    assert!(k.unknown.is_empty(), "no pinout for {:?}", k.unknown);
    assert_eq!(k.silent, ["26S02", "DIPOSC"], "parts with no behaviour");
}

/// Two totem-pole outputs on one net is an electrical fault, so a wrongly
/// claimed output pin surfaces here.
#[test]
fn no_net_has_two_push_pull_drivers() {
    support::no_net_has_two_push_pull_drivers(&cadrm());
}

/// **The banks are regular.** Each of the four banks is 33 DRAMs --- 32
/// data bits and parity --- drawn as a left half and a right half. The
/// halves share the bank's `-RAS`; each half has its own `-CAS`, `-WRITE`
/// and seven address lines, driven for it through its own series
/// resistors from its own distribution page, and every DRAM in a half is
/// on those; and each DRAM has a data bit of its own on `XBI` and `XBO`.
/// This compares nets, not names, and it is the wire list this board does
/// not have.
#[test]
fn the_banks_are_regular() {
    let n = cadrm();
    let mut ras_of_bank: Vec<netlist::NetId> = Vec::new();
    for bank in 0..4 {
        let mut bits: BTreeSet<u32> = BTreeSet::new();
        let mut ras: BTreeSet<netlist::NetId> = BTreeSet::new();
        for half in ["LH", "RH"] {
            let mut shared: Option<Vec<netlist::NetId>> = None;
            let page = format!("MEM{bank}{half}");
            let mut count = 0;
            for p in n.parts.iter().filter(|p| p.kind == "4116VG" && p.page == page) {
                let on = |pin: u8| p.pins.iter().find(|&&(k, _)| k == pin).unwrap().1;
                let lines: Vec<netlist::NetId> =
                    [4u8, 15, 3, 5, 7, 6, 12, 11, 10, 13].iter().map(|&k| on(k)).collect();
                match &shared {
                    None => shared = Some(lines),
                    Some(want) => assert_eq!(
                        &lines, want,
                        "{} {}: RAS, CAS, WRITE or an address line",
                        p.page, p.reference
                    ),
                }
                ras.insert(on(4));
                let din = n.net(on(2)).trim_matches('\'').to_string();
                let dout = n.net(on(14)).trim_matches('\'').to_string();
                let bit =
                    din.strip_prefix("XBI ").unwrap_or_else(|| panic!("{}: {din}", p.reference));
                assert_eq!(dout, format!("XBO {bit}"), "{} {}", p.page, p.reference);
                let bit: u32 = bit.parse().unwrap_or(32);
                assert!(bits.insert(bit), "bank {bank} has bit {bit} twice");
                count += 1;
            }
            assert!(count == 16 || count == 17, "{page}: {count} DRAMs");
        }
        assert_eq!(bits.len(), 33, "bank {bank}: {bits:?}");
        assert_eq!(ras.len(), 1, "bank {bank}: one RAS for both halves");
        let ras = ras.into_iter().next().unwrap();
        assert!(!ras_of_bank.contains(&ras), "bank {bank} shares its RAS with another");
        ras_of_bank.push(ras);
    }
}

/// The board with its Xbus wires driven as the backplane leaves them: every
/// line pulled up, the address for a board at the bottom of memory on the
/// switch, powered, reset over `-XBUS.INIT`, and running on its own
/// oscillator. Returns the board and the time on it.
fn board(n: &Netlist) -> XbusMaster<'_> {
    // Every switch open: the board answers addresses whose six top bits
    // are zero, which is the bottom 64K.
    XbusMaster::new(n, 0)
}

/// **The board comes up on its own clock.** Powered, reset and left for two
/// microseconds, no net is unknown, and the timing chain, the refresh timer
/// and the bank lines are where a resting board has them.
#[test]
fn the_board_settles_with_its_bus_driven() {
    let n = cadrm();
    let b = board(&n);
    let wired: BTreeSet<netlist::NetId> =
        n.parts.iter().flat_map(|p| p.pins.iter().map(|&(_, net)| net)).collect();
    let unknown: Vec<&str> =
        wired.iter().filter(|&&id| b.chip.net(id) == Level::X).map(|&id| n.net(id)).collect();
    assert!(unknown.is_empty(), "nets at unknown with the bus driven: {unknown:?}");
    for (name, want) in [
        ("-RAS", Level::High),
        ("-CAS", Level::High),
        ("-BUSY", Level::High),
        ("-T0", Level::High),
        ("BOARD SELECT", Level::High),
    ] {
        assert_eq!(b.level(name), want, "{name} at rest");
    }
    assert_eq!(b.level("-XBUS.ACK"), Level::High, "nothing asked");
    assert!(b.chip.next_tap().is_some(), "the oscillator runs");
}

/// **The board takes a word and gives it back.**
///
/// A write of one word and a read of it at the same address, driven as the
/// bus interface drives the Xbus: address, direction and data set up, then
/// `-XBUS.RQ` low until `-XBUS.ACK` answers, then released. What is
/// measured is the time from the request to the acknowledgement and the
/// word that comes back, and beside it the refresh: `REFRESH CYC` runs on
/// its own at the one-shot's period, and a request that lands in one waits.
#[test]
fn the_board_takes_a_word_and_gives_it_back() {
    let n = cadrm();
    let mut b = board(&n);
    let (t_write, _) = b.cycle(0o1234, Some(0o12345670123));
    eprintln!("write acknowledged after {t_write} ns");
    let (t_read, word) = b.cycle(0o1234, None);
    eprintln!("read acknowledged after {t_read} ns, word {word:o}");
    assert_eq!(word, 0o12345670123, "the word read back");
    let (_, other) = b.cycle(0o1235, None);
    assert_ne!(other, 0o12345670123, "the next word is not this one");

    // Refresh: let the board run and count the cycles it makes by itself,
    // each a `-BUSY` with nothing asking.
    let mut refreshes = 0;
    let mut was = b.level("-BUSY");
    let t0 = b.now;
    while b.now < t0 + 100_000 {
        b.run(b.now + 20);
        let l = b.level("-BUSY");
        if was != Level::Low && l == Level::Low {
            refreshes += 1;
        }
        was = l;
    }
    eprintln!("{refreshes} refresh cycles in 100 µs");
    assert!((7..=8).contains(&refreshes), "a refresh every 12.5 µs: {refreshes} in 100 µs");
}

/// **The numbers `rtl` needs.** A request lands on the board's own 24 MHz
/// clock, so the acknowledgement depends on where in a period it arrives;
/// the refresh cycles come on the one-shot's period plus the synchroniser's
/// and hold the board for a cycle; a request that arrives during one waits.
/// Printed, and pinned where they are stable.
#[test]
fn the_cycle_and_the_refresh_are_timed() {
    let n = cadrm();
    let mut b = board(&n);
    // A read at address 0 requested at each phase of the oscillator, well
    // clear of any refresh: the one-shot's first pulse ends at 12 µs.
    let mut acks = Vec::new();
    let mut starts = Vec::new();
    for phase in [0u64, 10, 20, 30] {
        // On a rising edge of the oscillator, 125/3 ns apart from power-on,
        // plus the phase.
        let start = rising_edge(rising_edge_after(b.now) + 25) + phase;
        b.run(start);
        starts.push((phase, start));
        acks.push((phase, b.cycle(0, None).0));
    }
    eprintln!("acknowledgement by request phase: {acks:?}");
    // The first rising edge strictly after the request, and eleven stages
    // from it: 500, 490, 480, 470 here, and the arithmetic `MemoryBoard` in
    // `src/busint.rs` runs `rtl` on.
    let expect: Vec<(u64, u64)> = starts
        .iter()
        .map(|&(phase, start)| (phase, rising_edge(rising_edge_after(start) + 11) - start))
        .collect();
    assert_eq!(acks, expect);
    assert_eq!(acks, [(0, 500), (10, 490), (20, 480), (30, 470)]);
    let busy_low = |b: &XbusMaster| b.level("-BUSY") == Level::Low;
    // The refresh cycles over 200 µs, with nothing else asking: every
    // `-BUSY` is one. `REFRESH CYC` is not the cycle but a mode flag, set
    // at the cycle's T5 and cleared at the next cycle's, whichever it is.
    let mut cycles: Vec<(u64, u64)> = Vec::new();
    let mut began: Option<u64> = None;
    let t0 = b.now;
    while b.now < t0 + 200_000 {
        b.run(b.now + 5);
        match (began, busy_low(&b)) {
            (None, true) => began = Some(b.now),
            (Some(at), false) => {
                cycles.push((at, b.now - at));
                began = None;
            }
            _ => {}
        }
    }
    let periods: Vec<u64> = cycles.windows(2).map(|w| w[1].0 - w[0].0).collect();
    eprintln!(
        "refresh cycles: {} in 200 µs; first at {} ns after the last request; busy {:?} ns; periods {:?}",
        cycles.len(),
        cycles[0].0 - t0,
        cycles.iter().map(|c| c.1).collect::<Vec<_>>(),
        periods
    );
    // The one-shot's 12,033 ns from the cycle's start, plus the
    // synchroniser's two Xbus clocks and the edge: 12.3 to 12.6 µs at a
    // 220 ns Xbus clock, the pulse being no multiple of it.
    assert!((15..=17).contains(&cycles.len()), "{} refresh cycles in 200 µs", cycles.len());
    // Eleven stages of 125/3 ns, less the 5 ns `T5` runs `-T0`'s fall
    // ahead of the chain's first stage, as the sampling sees it.
    assert!(
        cycles.iter().all(|c| (455..=460).contains(&c.1)),
        "a refresh cycle holds the board 455 ns: {cycles:?}"
    );
    assert!(
        periods.iter().all(|&p| (12_300..=12_600).contains(&p)),
        "one refresh every 12.3 to 12.6 µs: {periods:?}"
    );
    // A read after the refresh flag has been set: the flag must not steer
    // the cycle's address.
    assert_eq!(b.level("REFRESH CYC"), Level::High, "the flag is set between refreshes");
    b.cycle(0o4567, Some(0o31415726535));
    assert_eq!(b.cycle(0o4567, None).1, 0o31415726535, "a word written and read with the flag set");
    // A request that lands inside a refresh cycle waits for it: ask at
    // every 100 ns into the next one.
    let mut waits = Vec::new();
    for offset in [0u64, 100, 200, 300, 400] {
        while !busy_low(&b) {
            b.run(b.now + 5);
        }
        let began = b.now;
        b.run(began + offset);
        waits.push((offset, b.cycle(0, None).0));
    }
    eprintln!("a request landing this far into a refresh cycle is acknowledged after: {waits:?}");
    // The refresh's twelve stages, the edge after them, and eleven of its
    // own, less the request's offset into the refresh: 958 - k, as the
    // 5 ns sampling sees it.
    for &(k, wait) in &waits {
        assert!((955 - k..=960 - k).contains(&wait), "landing {k} ns in: {waits:?}");
    }
}

/// **MIT's wire list for the board agrees with the netlist, one ECO apart.**
/// `cadrm/mem.wlr` (4 March 1980) reached us by a different route from the
/// drawings: it is the list the board was wired from, pin by pin. Beside it
/// is `cadrm/mem.eco`, MIT's ECO record: ECO 1 (10 June 1979) replaced the
/// delay lines with the 74S374 chain and a 24 MHz crystal, and ECO 2 (4 July
/// 1979, "ECO #1 screwed up the timing") moved three of the chain's taps.
/// The wire list is before ECO 2 and the drawings after it, so ECO 2's five
/// moves are applied to the list here, and then every wire must be one net
/// and no net two wires, as `matches_mits_wire_list` in
/// `tests/busint_netlist.rs` holds for the interface. The netlist is read as
/// wired, the two ends of a series resistor apart, since the list has them
/// apart.
#[test]
fn matches_mits_wire_list() {
    let n = netlist::parse_wired(CADRM).unwrap();
    let mut signals = support::wire_list(&n, &["cadrm", "mem.wlr"]);
    // ECO 2, from `mem.eco`: "-T40 delete F6-13:F4-4; -T80 delete F8-14:F4-7,
    // add F4-5:F6-13; -T120 add F4-6:F8-14; -T320 delete F8-6:F5-3; -T400
    // add F8-6:F5-5". The list writes a 74S74's pins by both numberings,
    // `F06-10(13)`, and keeps the first.
    let moves =
        [("-T40", "-T80", "F06", 10u8), ("-T80", "-T120", "F08", 11), ("-T320", "-T400", "F08", 3)];
    for (from, to, slot, number) in moves {
        let at = signals
            .iter()
            .position(|s| s.name() == from)
            .unwrap_or_else(|| panic!("no wire {from}"));
        let k = signals[at]
            .pins
            .iter()
            .position(|p| p.slot() == slot && p.number == number)
            .unwrap_or_else(|| panic!("{from} has no pin {slot}-{number}: {:?}", signals[at].pins));
        let pin = signals[at].pins.remove(k);
        let at =
            signals.iter().position(|s| s.name() == to).unwrap_or_else(|| panic!("no wire {to}"));
        signals[at].pins.push(pin);
    }
    let r = wirelist::compare(&n, &signals, |slot| format!("0{slot}"));
    support::report(&signals, &r);
    assert!(r.placed > 2000, "the wire list was read");
    // What is left is the drawing being later than ECO 2: it also moves the
    // 74S74 at F09's two clocks a stage on, `CAS MPX TIME`'s from `-T40` to
    // `-T80` and `-RAS`'s from `-T320` to `-T400`, one pin each; and it
    // draws the one-shot's timing capacitor and resistor at F02 the other
    // way round between the 26S02's pins 1 and 2, which the model, taking
    // the pulse as a constant, does not see. The drawing is taken as the
    // board.
    let later = ["-T40", "-T320", "%F02@02-01", "%F02@02-02"];
    let unexplained: Vec<_> = r
        .split
        .iter()
        .filter(|(names, nets)| {
            !(later.contains(&names[0].as_str())
                && (names[0].starts_with('%') || nets.iter().any(|(_, k)| *k == 1)))
        })
        .collect();
    assert!(
        unexplained.is_empty() && r.merged.is_empty(),
        "the wire list disagrees with the netlist: {unexplained:?}"
    );
    assert_eq!(r.split.len(), 4, "the four differences the drawing's later revision makes");
}

/// **Which edge takes a cycle that waited.** A request that finds the board
/// busy is taken some edges after the edge that took the cycle in flight,
/// and the count depends on which followed which: measured here for a cpu
/// request behind a refresh, a cpu request behind a cpu cycle, and a
/// refresh behind a cpu cycle, in rising edges of the oscillator, which
/// `MemoryBoard` in `src/busint.rs` must reproduce.
#[test]
fn the_edge_that_takes_a_waiting_cycle() {
    let n = cadrm();
    let mut b = board(&n);
    let busy_low = |b: &XbusMaster| b.level("-BUSY") == Level::Low;
    // Runs to the next fall of `-BUSY` and returns the edge it fell at,
    // which is one after the edge that took the cycle.
    let next_busy = |b: &mut XbusMaster| -> u64 {
        while busy_low(b) {
            b.run(b.now + 1);
        }
        while !busy_low(b) {
            b.run(b.now + 1);
        }
        assert_eq!(rising_edge(rising_edge_after(b.now - 1)), b.now, "-BUSY falls on an edge");
        rising_edge_after(b.now - 1)
    };
    // A cpu request 100 ns into a refresh cycle.
    let r = next_busy(&mut b) - 1;
    b.run(rising_edge(r + 1) + 100);
    b.request(0, None);
    while !b.acked() {
        b.run(b.now + 1);
    }
    let taken_after_refresh = rising_edge_after(b.now - 1) - 11;
    b.run(b.now + XbusMaster::RELEASE_NS);
    b.release();
    b.run(b.now + 600);
    eprintln!("a cpu request behind a refresh taken at R+{}", taken_after_refresh - r);
    assert_eq!(taken_after_refresh, r + 13);

    // A cpu request 10 ns after another's acknowledgement, before its busy ends.
    b.run(b.now + 3_000);
    let e = rising_edge_after(b.now + 100);
    b.run(rising_edge(e) - 10);
    b.request(0, None);
    while !b.acked() {
        b.run(b.now + 1);
    }
    let ack_a = b.now;
    assert_eq!(ack_a, rising_edge(e + 11));
    b.run(ack_a + XbusMaster::RELEASE_NS);
    b.release();
    b.run(ack_a + 60);
    b.request(0, None);
    while !b.acked() {
        b.run(b.now + 1);
    }
    let ack_b = b.now;
    b.run(b.now + XbusMaster::RELEASE_NS);
    b.release();
    let taken_b = rising_edge_after(ack_b - 1) - 11;
    eprintln!("a cpu request behind a cpu cycle taken at E+{}", taken_b - e);
    assert_eq!(taken_b, e + 13);
    b.run(b.now + 600);

    // A refresh behind a cpu cycle: the next `TIME FOR REFRESH` is known,
    // so a read is started just after it, and the refresh request, up two
    // Xbus clocks later, finds the cycle in flight; the refresh's `-BUSY`
    // is watched for.
    let r2 = next_busy(&mut b) - 1;
    // The one-shot runs from `-REFRESH NOW`'s fall, `T5` after the edge.
    let time_rise = rising_edge(r2) + 5 + muir::busint::REFRESH_NS;
    b.run(time_rise + 30);
    let e2 = rising_edge_after(b.now);
    let watch = [
        "-XBUS.RQ",
        "XB RQ",
        "MEM RQ",
        "-T0",
        "IDLE",
        "IDLE A",
        "-BUSY",
        "REFRESH RQ",
        "REFRESH CYC",
        "TIME FOR REFRESH",
        "XACK",
        "-XBUS.ACK",
        "-T440",
        "-T480",
        "-T520",
        "-T560",
        "-T600",
        "-T640",
    ];
    let mut last: Vec<Level> = watch.iter().map(|w| b.level(w)).collect();
    let t0 = b.now;
    let show = |b: &XbusMaster, last: &mut Vec<Level>| {
        for (k, w) in watch.iter().enumerate() {
            let l = b.level(w);
            if l != *last.get(k).unwrap() {
                eprintln!(
                    "    {:>5} ns (E{:+}): {w}={l:?}",
                    b.now - t0,
                    (b.now as i64 - rising_edge(e2) as i64) * 3 / 125
                );
                last[k] = l;
            }
        }
    };
    b.request(0, None);
    show(&b, &mut last);
    while !b.acked() {
        b.run(b.now + 1);
        show(&b, &mut last);
    }
    assert_eq!(b.now, rising_edge(e2 + 11));
    b.run(b.now + XbusMaster::RELEASE_NS);
    b.release();
    show(&b, &mut last);
    // Through the cycle's busy and to the refresh's own `-BUSY`.
    while busy_low(&b) {
        b.run(b.now + 1);
        show(&b, &mut last);
    }
    while !busy_low(&b) {
        b.run(b.now + 1);
        show(&b, &mut last);
    }
    let refresh = rising_edge_after(b.now - 1) - 1;
    eprintln!("a refresh behind a cpu cycle taken at E+{}", refresh - e2);
    // With the request lifted 30 ns after the acknowledgement, before
    // `-BUSY` lifts at the twelfth edge; lifted after it, `IDLE` waits for
    // that instead, and the refresh goes an edge later, as the band shows
    // behind every read.
    assert_eq!(refresh, e2 + 13);
}
