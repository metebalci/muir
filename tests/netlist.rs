// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Checks the CADR4 netlist parses to the shape we expect.
//!
//! The netlist is extracted from MIT's SUDS drawings by `tools/soap4`, and
//! SUDS is an awkward format, so the numbers here are pinned: if the file or
//! the parser changes, the tests say so rather than the chip engine quietly
//! simulating a different machine.

use muir::{netlist, wirelist};
use std::collections::BTreeMap;
use std::path::PathBuf;

mod support;
use support::mit;

const NETLIST: &str = include_str!("../data/CADR.netlist");

/// Every board muir has a netlist for, for the checks that are of all of
/// them rather than of the processor pair.
const BOARDS: [(&str, &str); 8] = [
    ("BUSINT", include_str!("../data/BUSINT.netlist")),
    ("CADR", NETLIST),
    ("CADRDC", include_str!("../data/CADRDC.netlist")),
    ("CADRIO", include_str!("../data/CADRIO.netlist")),
    ("CADRM", include_str!("../data/CADRM.netlist")),
    ("DM", include_str!("../data/DM.netlist")),
    ("LISPMTV", include_str!("../data/LISPMTV.netlist")),
    ("SIMPLETV", include_str!("../data/SIMPLETV.netlist")),
];

#[test]
fn parses_to_the_expected_shape() {
    let n = netlist::parse(NETLIST).unwrap();
    assert_eq!(n.pages.len(), 97, "page markers");
    assert_eq!(n.populated_pages().len(), 91, "pages carrying parts");
    assert_eq!(n.parts.len(), 1243, "parts");
    let kinds: std::collections::BTreeSet<&str> = n.parts.iter().map(|p| p.kind.as_str()).collect();
    assert_eq!(kinds.len(), 71, "distinct part types");
    assert!(n.nets.len() > 2000, "nets: {}", n.nets.len());
    assert!(n.parts.iter().all(|p| !p.pins.is_empty()), "every part has pins");
    eprintln!(
        "{} parts, {} nets, {} page markers ({} populated)",
        n.parts.len(),
        n.nets.len(),
        n.pages.len(),
        n.populated_pages().len()
    );
    let empty: Vec<&String> =
        n.pages.iter().filter(|p| !n.populated_pages().contains(&p.as_str())).collect();
    eprintln!("pages with no parts: {empty:?}");
}

/// Reference designators are not unique, so nothing may key parts by name.
#[test]
fn reference_designators_repeat() {
    let n = netlist::parse(NETLIST).unwrap();
    let mut seen: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for p in &n.parts {
        *seen.entry((p.page.as_str(), p.reference.as_str())).or_default() += 1;
    }
    let dupes: Vec<_> = seen.iter().filter(|&(_, &c)| c > 1).collect();
    assert!(!dupes.is_empty(), "expected repeated designators");
    assert_eq!(
        seen[&("BCTERM", "1B20")],
        2,
        "1B20 appears twice on BCTERM; it is the case that caught this"
    );
    eprintln!("{} designators repeat within their page", dupes.len());
}

/// SUDS marks open-collector variants with a trailing O.  An open-collector
/// output can only pull low, so the chip engine must not lose this.
#[test]
fn open_collector_variants_are_marked() {
    let n = netlist::parse(NETLIST).unwrap();
    let oc: Vec<&str> = {
        let mut v: Vec<&str> =
            n.parts.iter().filter(|p| p.open_collector()).map(|p| p.kind.as_str()).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    eprintln!("open-collector variants: {oc:?}");
    assert!(oc.contains(&"74S02O"), "74S02O should be marked open collector");
    assert_eq!(n.parts.iter().find(|p| p.kind == "74S02O").unwrap().base_kind(), "74S02");
}

/// A hand-checked anchor: three pins of the 74S174 at ACTL 3B26, on the
/// wires MIT's `cadrwd/cadr4.wlr` puts them on.
#[test]
fn actl_3b26_is_wired_as_expected() {
    let n = netlist::parse(NETLIST).unwrap();
    let p = n.parts.iter().find(|p| p.page == "ACTL" && p.reference == "3B26").expect("ACTL 3B26");
    assert_eq!(p.kind, "74S174");
    let by_pin: BTreeMap<u8, &str> = p.pins.iter().map(|&(k, v)| (k, n.net(v))).collect();
    assert_eq!(by_pin[&1], "-RESET");
    assert_eq!(by_pin[&9], "CLK3D");
    assert_eq!(by_pin[&15], "DESTD");
}

/// A `part` record is a gate, not a package. Anything that instantiates real
/// chips has to merge them, or a quad NAND becomes four chips with a quarter
/// of their pins each.
#[test]
fn gate_records_merge_into_packages() {
    let n = netlist::parse(NETLIST).unwrap();
    let pkgs = n.packages();
    assert_eq!(n.parts.len(), 1243, "gate records");
    assert_eq!(pkgs.len(), 1084, "packages");

    // ACTL 3B30 is a 74S37 whose four gates arrive as three records.
    let p: Vec<_> = pkgs.iter().filter(|p| p.page == "ACTL" && p.reference == "3B30").collect();
    assert_eq!(p.len(), 1, "3B30 should merge to one package");
    let pins: Vec<u8> = p[0].pins.iter().map(|&(pin, _)| pin).collect();
    assert_eq!(pins, vec![1, 2, 3, 4, 5, 6, 8, 9, 10]);

    // The three reused designators must stay separate: they really are two
    // different resistor packs sharing a name.
    for r in ["1B15", "1B20", "1B25"] {
        let c = pkgs.iter().filter(|p| p.page == "BCTERM" && p.reference == r).count();
        assert_eq!(c, 2, "BCTERM {r} is two distinct packages");
    }
}

/// **Every place a designator names two bodies, on all eight boards.**
///
/// [`Netlist::packages`] merges records that share a page, a designator and
/// a type when their pins are disjoint, and where a pin is claimed twice it
/// makes two packages instead --- a designator being a board location, and
/// MIT putting two bodies in one. This is the census of that, pinned here
/// because the count belongs to these files and not to the format: three of
/// the twenty are on the processor pair, where they are all resistor packs
/// on one page, and it was once written down as if that were the whole of
/// it. The memory board and the SIMPLE TV have none at all.
///
/// **Eight of the twenty are devices**, and that is the half worth having:
/// a reader deciding whether a citation by designator is safe must not be
/// told it is only ever resistors. Seven are pairs of 8-pin 75452 drivers
/// in one 16-pin footprint, which MIT's own files write `B05` and `B05@03`
/// and these files cannot. The eighth is not a pair at all --- see
/// `chip::wire_oscillators`, and `parts_mounted.rs`, which holds each of
/// these locations to what MIT's stuffing list stuffs there.
#[test]
fn a_designator_can_name_two_bodies() {
    let mut split = Vec::new();
    for (name, text) in BOARDS {
        let n = netlist::parse(text).unwrap();
        let mut seen: BTreeMap<(&str, &str, &str), Vec<Vec<u8>>> = BTreeMap::new();
        for part in &n.parts {
            let pins: Vec<u8> = part.pins.iter().map(|&(pin, _)| pin).collect();
            let bodies = seen.entry((&part.page, &part.reference, &part.kind)).or_default();
            match bodies.iter_mut().find(|had| !had.iter().any(|p| pins.contains(p))) {
                Some(had) => had.extend(&pins),
                None => bodies.push(pins),
            }
        }
        for ((page, reference, kind), bodies) in seen {
            if bodies.len() > 1 {
                split.push(format!("{name} {page} {reference} {kind} x{}", bodies.len()));
            }
        }
    }
    split.sort();
    assert_eq!(
        split,
        [
            "BUSINT LMDATA 0A28 SIP180/390-8 x2",
            "BUSINT LMDATA 0A29 SIP180/390-8 x2",
            "BUSINT LMDATA 0A30 SIP180/390-8 x2",
            "CADR BCTERM 1B15 SIP220/330-8 x2",
            "CADR BCTERM 1B20 SIP220/330-8 x2",
            "CADR BCTERM 1B25 SIP220/330-8 x2",
            // One 74LS124, not two: the drawing gives each of its two VCO
            // sections a body of its own and both of them the package's
            // ground and supply pins, so the records collide. `dc.stf`
            // stuffs one device at B04 and `chip::wire_oscillators` takes
            // each section from whichever record carries its pins.
            "CADRDC DCTMOT 0B04 74LS124 x2",
            "CADRDC DCTRSG 0A01 SIP100-8 x2",
            "CADRDC DCTRSG 0A02 75452 x2",
            "CADRDC DCTRSG 0B01 75452 x2",
            "CADRDC DCTRSG 0B03 75452 x2",
            "CADRIO LMMYNM 0D11 P SIP1000-10 x2",
            "DM DMIO 0B05 75452 x2",
            "DM DMIO 0B06 SIP100-8 x2",
            "DM DMIO 0B11 SIP100-8 x2",
            "DM DMIO 0B15 75452 x2",
            "DM DMIO 0B16 SIP100-8 x2",
            "DM DMSEQ 0D12 75452 x2",
            "DM DMSEQ 0D15 75452 x2",
            "LISPMTV ECLVID 0E08 RES x2",
        ]
    );
    let devices = split.iter().filter(|s| s.contains("75452") || s.contains("74LS124")).count();
    assert_eq!((split.len(), devices), (20, 8));
}

/// Every unnamed net carries a name from another end, and all of them must
/// reach the same wire.
///
/// The file names an unnamed net after a pin at the far end, so the wire
/// between ALUC4 2C15 pin 3 and 2C20 pin 13 appears as `@2C20,p13` at one end
/// and `@2C15,p3` at the other. Left alone, all 72 of them are broken apart;
/// `src/netlist.rs` joins them.
///
/// There are more names than wires: a wire of three or more pins gets one
/// name per end, and joining them end to end makes one net of it.
/// `cadrwd/icmem3.wlr` is MIT's own word on the four-pin one on OLORD2 ---
/// the unnamed wire `%1A19-09` joins 1A20-01, 1A19-12, 1A19-10 and 1A19-09.
#[test]
fn anonymous_nets_are_joined_end_to_end() {
    // Straight out of the file, before any merging.
    let written: std::collections::BTreeSet<&str> = NETLIST
        .lines()
        .filter_map(|l| l.trim().split_once('='))
        .map(|(_, net)| net.trim())
        .filter(|net| net.starts_with('@'))
        .collect();
    assert_eq!(written.len(), 72, "unnamed names in the file");

    let n = netlist::parse(NETLIST).unwrap();
    let mut wires = std::collections::BTreeSet::new();
    let mut single = Vec::new();
    for id in 0..n.nets.len() as u32 {
        if !n.net(id).starts_with('@') {
            continue;
        }
        let pins: usize =
            n.parts.iter().map(|p| p.pins.iter().filter(|&&(_, x)| x == id).count()).sum();
        match pins {
            0 => {}
            1 => single.push(n.net(id)),
            _ => {
                wires.insert(id);
            }
        }
    }
    assert!(single.is_empty(), "unnamed nets still reaching one pin: {single:?}");
    eprintln!("{} unnamed names in the file became {} wires", written.len(), wires.len());
    assert!(wires.len() < written.len(), "nothing was joined");
}

/// **The netlist against MIT's own wire lists**, pin by pin.
///
/// `cadrwd/cadr4.wlr` and `cadrwd/icmem3.wlr` are what MIT's tooling made
/// of the same drawings in 1980, one for each of the two boards this file
/// holds --- the wire lists the boards were wrapped from. Every wire there
/// must be one net here and every net here one wire there.
#[test]
fn matches_mits_wire_lists() {
    let n = netlist::parse(NETLIST).unwrap();
    for file in ["cadr4.wlr", "icmem3.wlr"] {
        let signals = support::wire_list(&n, &["cadrwd", file]);
        let r = wirelist::compare(&n, &signals, |slot| slot.to_string());
        eprint!("{file}: ");
        support::report(&signals, &r);
        assert!(r.placed > 4000, "the wire list was read");
        assert!(r.split.is_empty() && r.merged.is_empty(), "{file} disagrees with the netlist");
    }
}

/// **The two boards are joined as their cables join them.** The processor's
/// connectors `2FJ1`..`4EJ1` meet the ICMEM board's `1BJ1`..`1FJ2` pin for
/// pin, and both wire lists name every pin. Sixteen wires change name at
/// the connector. For each, the net the processor list's pins fall on must
/// be the net the ICMEM list's pins fall on. Fourteen of them, the `PC`
/// bus, the file has under the processor's names, and the parser joins the
/// other two by `EXPLICIT_ALIASES`.
#[test]
fn the_boards_are_joined_as_the_cables_join_them() {
    let (a, b) = (mit(&["cadrwd", "cadr4.wlr"]), mit(&["cadrwd", "icmem3.wlr"]));
    let n = netlist::parse(NETLIST).unwrap();
    let read = |p: &PathBuf| {
        let text = String::from_utf8_lossy(&std::fs::read(p).unwrap()).into_owned();
        wirelist::parse(&text, &n.pages)
    };
    let (cpu, icmem) = (read(&a), read(&b));
    // Connector pins by connector: pin -> wire name.
    let pins = |signals: &[wirelist::Signal]| {
        let mut by: BTreeMap<String, BTreeMap<u8, String>> = BTreeMap::new();
        for s in signals.iter().filter(|s| !s.is_pseudo()) {
            for p in s.pins.iter().filter(|p| p.body == "CON") {
                by.entry(p.location.clone()).or_default().insert(p.number, s.name().to_string());
            }
        }
        by
    };
    // The net a wire's on-board pins are on in the netlist.
    let net_of = |signals: &[wirelist::Signal], name: &str| -> Option<u32> {
        let s = signals.iter().find(|s| s.name() == name)?;
        s.pins.iter().filter(|p| p.body != "CON" && !p.page.is_empty()).find_map(|p| {
            n.parts
                .iter()
                .filter(|part| part.page == p.page && part.reference == p.slot())
                .flat_map(|part| part.pins.iter())
                .find(|&&(k, _)| k == p.number)
                .map(|&(_, net)| net)
        })
    };
    let (cpu_pins, icmem_pins) = (pins(&cpu), pins(&icmem));
    // The pairing: the processor connector, the ICMEM connector, and how
    // many pins on the ICMEM header is pin 1 of the processor's.
    let cables = [
        ("2FJ1", "1FJ2", 2u8),
        ("3CJ1", "1CJ2", 2),
        ("3DJ1", "1DJ2", 2),
        ("3EJ1", "1EJ2", 2),
        ("3FJ1", "1FJ1", 0),
        ("4BJ1", "1BJ1", 0),
        ("4CJ1", "1CJ1", 0),
        ("4DJ1", "1DJ1", 0),
        ("4EJ1", "1EJ1", 0),
    ];
    let mut same = 0;
    let mut differ = 0;
    let mut split: Vec<String> = Vec::new();
    for (c, i, off) in cables {
        for (&pin, name) in &cpu_pins[c] {
            let Some(other) = icmem_pins[i].get(&(pin + off)) else { continue };
            if name == other {
                same += 1;
                continue;
            }
            differ += 1;
            let (x, y) = (net_of(&cpu, name), net_of(&icmem, other));
            if x.is_some() && y.is_some() && x != y {
                split.push(format!("{name} / {other}"));
            }
        }
    }
    eprintln!(
        "{same} wires keep their name across the boards, {differ} change it; split: {split:?}"
    );
    assert_eq!(differ, 16, "wires that change name at the connector");
    // The fourteen are the PC bus: the processor's 74S241s at LPC buffer
    // `PC0`..`PC13` into `PC0B`..`PC13B` for the cable, and the ICMEM list
    // calls the far end `PC0`..`PC13` again. The netlist has the register's
    // net and the buffer's net, as the boards do, and the ICMEM's readers on
    // the register's by name --- the same signal through a buffer that
    // inverts nothing --- so nothing is joined here on purpose.
    let expected: Vec<String> = (0..14).map(|b| format!("PC{b}B / PC{b}")).collect();
    assert_eq!(split, expected, "wires the boards have as two nets");
}

/// **One pair of net names in the whole tree differs by case alone**, and
/// `Netlist::EXPLICIT_ALIASES` joins it by hand rather than by rule.
///
/// The pair is the display boards' dot clock: MIT typed `64 MHz CLK L` on
/// `necsip.drw`, the SIP terminator sheet, and `64 MHZ CLK` on every sheet
/// that uses it, so the terminator sat on a net of its own and the clock
/// ran with no pull-down. The alias is safe to make explicitly because
/// nothing else in any board collides --- which is what this measures. A
/// new netlist that brings a second clash fails here, and then the question
/// is whether that one is one signal too, not whether to normalise every
/// name in the tree.
///
/// Read off the files rather than through [`netlist::parse`], which has
/// joined the pair by the time it can be counted.
#[test]
fn only_the_dot_clock_is_named_twice_by_case() {
    let mut found: Vec<(&str, Vec<String>)> = Vec::new();
    for board in ["CADR", "BUSINT", "CADRM", "CADRIO", "SIMPLETV", "LISPMTV", "CADRDC"] {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data").join(format!("{board}.netlist"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e}: the netlists are committed", path.display()));
        // Every pin line is `pN=NET`, the net quoted when its name has spaces.
        let mut by_lower: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
        for line in text.lines() {
            let Some((p, net)) = line.split_once('=') else { continue };
            if !p.starts_with('p') || !p[1..].bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            by_lower.entry(net.to_lowercase()).or_default().insert(net.to_string());
        }
        for (_, names) in by_lower.into_iter().filter(|(_, v)| v.len() > 1) {
            found.push((board, names.into_iter().collect()));
        }
    }
    for (board, names) in &found {
        eprintln!("{board}: {names:?}");
    }
    let clock = ["'-64 MHZ CLK'".to_string(), "'-64 MHz CLK'".to_string()];
    assert!(
        found.iter().all(|(_, names)| names[..] == clock),
        "a net named twice by case alone that is not the dot clock: {found:?}"
    );
    assert_eq!(found.len(), 2, "the SIMPLE TV and the LISPM TV, one pair each: {found:?}");
}

/// **No board has two net names that differ only in spacing or case.**
/// MIT's draughtsmen wrote a wire's name twice on a sheet and did not
/// always write it the same way, and soap4 takes two spellings for two
/// nets --- leaving the pins on one half with no driver, which is a broken
/// board rather than a cosmetic difference. `-11CLRTDN` on the I/O board
/// was one: the 74S00 at D07 pin 11 drove `-11CLRTDN` and the 74S08 at B10
/// pin 5 waited on `- 11CLRTDN`.
///
/// `matches_mits_wire_lists` cannot catch these, which is why this is
/// separate: that comparison squeezes the spaces out of a name before
/// matching it, so the two halves look like the one wire MIT's list says
/// they are and nothing is reported. Each pair found here wants an entry
/// in `Netlist::EXPLICIT_ALIASES`, and the wire list consulted first to
/// say which spelling is the wire.
#[test]
fn no_board_spells_one_net_two_ways() {
    let mut clashes = Vec::new();
    for (name, text) in BOARDS {
        let n = netlist::parse(text).unwrap();
        let mut used = std::collections::BTreeSet::new();
        for part in &n.parts {
            for &(_, net) in &part.pins {
                used.insert(net);
            }
        }
        let mut by_squashed: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for net in 0..n.nets.len() {
            let full = n.net(net as u32);
            // A name an alias has emptied is still in the table; only nets
            // a pin still lands on are wires of the board.
            if full.starts_with('@') || !used.contains(&(net as u32)) {
                continue;
            }
            let squashed: String = full
                .trim_matches('\'')
                .chars()
                .filter(|c| *c != ' ')
                .collect::<String>()
                .to_uppercase();
            by_squashed.entry(squashed).or_default().push(full.to_string());
        }
        for (_, spellings) in by_squashed.iter().filter(|(_, v)| v.len() > 1) {
            clashes.push(format!("{name}: {spellings:?}"));
        }
    }
    for c in &clashes {
        eprintln!("{c}");
    }
    assert!(clashes.is_empty(), "{} nets are spelled two ways", clashes.len());
}
