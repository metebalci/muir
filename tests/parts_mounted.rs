// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! How many parts are on each board.
//!
//! Three numbers can be called the parts of a netlist, and only the last of
//! them is a count of devices:
//!
//! - a **record** is one `part` line, which is one gate. A quad NAND is
//!   drawn four times under one designator, so the processor's file holds
//!   1243 records.
//! - a **package** is [`Netlist::packages`], those records merged per
//!   schematic page, and is what the `chip` engine instantiates. There are
//!   more packages than chips: gates of one chip drawn on two sheets are
//!   built as two.
//! - a **part** is what is mounted --- one device at one board location:
//!   the chips, the oscillators, the delay lines, the switches and the LED
//!   digits, but not the bypass capacitors, resistor packs, busbars and
//!   pull-up networks, which nothing here simulates.
//!
//! The README, `data/README.md` and `site/index.html` quote the third, so
//! this file holds them to it --- and to MIT's own two counts of the same
//! boards, each made by its tooling from the drawings and reaching us by a
//! different route than the sheets did: the parts list `*.prt`, which names
//! every location that was stuffed and what went in it, and the DIP census
//! `*.wls`, which counts the packages each part type takes. Four boards
//! have a parts list and seven have a census. Only the SIMPLE TV has
//! neither, and its count rests on the drawings alone.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use muir::netlist::{self, Netlist};

mod support;
use support::mit;

const CADR: &str = include_str!("../data/CADR.netlist");
const BUSINT: &str = include_str!("../data/BUSINT.netlist");
const CADRM: &str = include_str!("../data/CADRM.netlist");
const CADRIO: &str = include_str!("../data/CADRIO.netlist");
const CADRDC: &str = include_str!("../data/CADRDC.netlist");
const SIMPLETV: &str = include_str!("../data/SIMPLETV.netlist");
const LISPMTV: &str = include_str!("../data/LISPMTV.netlist");

/// Bypass capacitors, resistor and terminator packs, busbars, pull-up
/// networks, and the bodies a sheet carries with no device in them. MIT's
/// parts lists put these at sub-positions of a location --- `C08@01` beside
/// the chip at `C08` --- which is the shape of the thing: they are not
/// parts of the machine, and nothing simulates them.
fn is_passive(kind: &str) -> bool {
    let k = kind.to_ascii_uppercase();
    k.contains("SIP")           // resistor packs: SIP180/390-8, DUAL-SIP
        || k.contains("DUMMY")  // a body on the sheet, no device in it
        || k.contains("SERRES")
        || k.starts_with("CAP")
        || k.ends_with("UFCAP")
        || k.starts_with("RES")
        || k.starts_with("DUAL-SI") // DUAL-SIP, cut to seven in a census
        || k.starts_with("898-") // a resistor network by its Bourns number
        || matches!(k.as_str(), "BUSBAR" | "PULLUP" | "TRITERM")
}

/// The board locations carrying a device, one part apiece. A location holds
/// one: where a designator appears twice it is a chip and the pack or
/// capacitor beside it, never two chips, which is what MIT's parts lists
/// say too and what [`mits_parts_lists_agree`] checks.
fn parts_on(n: &Netlist, on_board: impl Fn(&str) -> bool) -> BTreeSet<String> {
    let mut mounted: BTreeMap<&str, bool> = BTreeMap::new();
    for p in n.parts.iter().filter(|p| on_board(&p.page)) {
        *mounted.entry(p.reference.as_str()).or_insert(false) |= !is_passive(&p.kind);
    }
    mounted.into_iter().filter(|&(_, live)| live).map(|(r, _)| r.to_string()).collect()
}

fn parts(n: &Netlist) -> BTreeSet<String> {
    parts_on(n, |_| true)
}

/// The control-store board's pages, by MIT's own list of them:
/// `cadr/framl.txt`, the frame list of `icmem.book`. `CADR.netlist` holds
/// two boards and they share designators --- `1A01` is a 74S240 on the
/// processor's VMEMDR and a 74S174 on the control store's OLORD1 --- so
/// counting locations means counting them a board at a time. The list
/// names the control-store RAM pages `RAM00`..`RAM33` where the drawings
/// are `iram00.drw`, and the netlist took the file names.
fn control_store_pages() -> BTreeSet<String> {
    let text = std::fs::read_to_string(mit(&["cadr", "framl.txt"])).unwrap();
    text.split_whitespace()
        .map(|p| if p.starts_with("RAM") { format!("I{p}") } else { p.to_string() })
        .collect()
}

/// **What is mounted on each board.** The numbers the README, `data/README.md`
/// and the site quote, and the machine they add up to.
#[test]
fn every_board_has_the_parts_the_docs_claim() {
    let cadr = netlist::parse(CADR).unwrap();
    let icmem = control_store_pages();
    let processor = parts_on(&cadr, |page| !icmem.contains(page)).len();
    let control_store = parts_on(&cadr, |page| icmem.contains(page)).len();
    assert_eq!(processor, 643, "the CADR processor board");
    assert_eq!(control_store, 342, "the ICMEM control-store board");
    assert_eq!(processor + control_store, 985, "CADR.netlist, both boards");

    let mut n = BTreeMap::new();
    for (name, text, want) in [
        ("BUSINT", BUSINT, 176),
        ("CADRM", CADRM, 168),
        ("CADRIO", CADRIO, 173),
        ("CADRDC", CADRDC, 171),
        ("SIMPLETV", SIMPLETV, 171),
        ("LISPMTV", LISPMTV, 172),
    ] {
        let count = parts(&netlist::parse(text).unwrap()).len();
        assert_eq!(count, want, "{name}");
        n.insert(name, count);
    }

    // The machine the site draws: the processor pair, the bus interface,
    // the memory, the I/O board, the disk controller and the SIMPLE TV.
    let machine = processor + control_store + n["BUSINT"] + n["CADRM"] + n["CADRIO"] + n["CADRDC"];
    assert_eq!(machine + n["SIMPLETV"], 1844, "one memory board");
    assert_eq!(machine + n["LISPMTV"], 1845, "with the colour TV instead");
    assert_eq!(machine + n["SIMPLETV"] + 31 * n["CADRM"], 7052, "all 32 memory boards");
}

/// **The I/O board is two machines' worth of function on one board**, and
/// MIT drew the halves separately: fourteen pages of I/O, thirteen of
/// Chaosnet. Five chips have gates on both sides, so the halves do not add
/// up to the board.
#[test]
fn the_io_board_halves() {
    let n = netlist::parse(CADRIO).unwrap();
    let io = parts_on(&n, |page| !page.starts_with("LM"));
    let chaos = parts_on(&n, |page| page.starts_with("LM"));
    assert_eq!(io.len(), 94, "keyboard, mouse and clocks");
    assert_eq!(chaos.len(), 84, "Chaosnet");
    assert_eq!(io.intersection(&chaos).count(), 5, "chips with gates on both");
    assert_eq!(parts(&n).len(), 94 + 84 - 5);
}

/// **Against MIT's own parts lists.** A `*.prt` is the board as the
/// stockroom saw it, made by MIT's tooling from the same drawings, and it
/// reached us by a different route than the sheets did. Every location it
/// stuffs with a device must be a part here and no other location may be.
#[test]
fn mits_parts_lists_agree() {
    // The I/O board's two exceptions are the configuration switches at D10
    // and D12. The netlist has them as `SWITCH`, off `iobcsr.drw`; MIT's
    // list carries them as `DUMMY` with no part number, the switch not
    // being a stocked DIP. They are mounted, and muir reads them for the
    // Chaosnet address, so they are parts here.
    for (board, text, list, only_ours) in [
        ("bus interface", BUSINT, ["cadr1", "busint.prt"], &[][..]),
        ("disk controller", CADRDC, ["cadrdc", "dc.prt"], &[]),
        ("I/O board", CADRIO, ["cadrio", "iob.prt"], &["D10", "D12"]),
        ("LISPM TV", LISPMTV, ["cadrtv", "lmtv4b.prt"], &[]),
    ] {
        let n = netlist::parse(text).unwrap();
        let ours: BTreeSet<String> = parts(&n)
            .iter()
            .map(|r| {
                r.strip_prefix('0').expect("a designator on these boards is 0-prefixed").to_string()
            })
            .collect();
        let theirs = stuffed(&mit(&list));
        assert!(theirs.len() > 150, "{board}: the parts list was read");
        let missing: Vec<_> = theirs.difference(&ours).collect();
        assert!(
            missing.is_empty(),
            "{board}: MIT stuffs {missing:?}, the netlist has no part there"
        );
        let extra: Vec<_> = ours.difference(&theirs).map(String::as_str).collect();
        assert_eq!(extra, only_ours, "{board}: parts the netlist has and MIT's list does not");
    }
}

/// MIT's parts list. Tab separated: the part number, the type, how many,
/// an optional comment, and the locations, four to a line and running on to
/// the next line when there are more. A location is `B12`, or `B12@03`
/// where something shares the position with the device there.
fn stuffed(path: &Path) -> BTreeSet<String> {
    let text = String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned();
    let mut out = BTreeSet::new();
    let mut kind = String::new();
    let mut in_table = false;
    for line in text.lines() {
        if line.contains("PART NUMBER") {
            in_table = true;
            continue;
        }
        let cols: Vec<&str> = line.split('\t').map(str::trim).filter(|c| !c.is_empty()).collect();
        let Some(&last) = cols.last() else { continue };
        let here = trailing_locations(last);
        if !in_table || here.is_empty() {
            continue;
        }
        if cols.len() > 1 {
            kind = cols.iter().find(|c| **c != "N/A").unwrap().to_string();
        }
        if is_passive(&kind) {
            continue;
        }
        for loc in here {
            out.insert(loc.split('@').next().unwrap().to_string());
        }
    }
    out
}

/// The locations at the end of a row: `A01`, or `A22@03`, comma separated.
/// They are the last column, but not always the whole of it --- one row of
/// the disk controller's list runs a comment and the location together ---
/// so what counts is the run of positions the column ends with.
fn trailing_locations(col: &str) -> Vec<&str> {
    let position = |t: &str| {
        let b = t.as_bytes();
        let slot = |b: &[u8]| {
            b.len() == 3 && b[0].is_ascii_uppercase() && b[1..].iter().all(u8::is_ascii_digit)
        };
        match b.len() {
            3 => slot(b),
            6 => slot(&b[..3]) && b[3] == b'@' && b[4..].iter().all(u8::is_ascii_digit),
            _ => false,
        }
    };
    let t: Vec<&str> = col.split(',').map(str::trim).collect();
    let start = t.iter().rposition(|t| !position(t)).map_or(0, |i| i + 1);
    t[start..].to_vec()
}

/// **Against MIT's own DIP census.** A `*.wls` is the board by part type:
/// how many gate sections the sheets use of each, how many packages that
/// is, and what they draw. It is a different document from the parts list,
/// and the only second source for the processor and the control store,
/// neither of which has one.
///
/// The census counts a device where the netlist counts a location, so the
/// two agree part for part --- but for the switches. MIT had no part
/// number for a DIP switch and carried it as `DUMMY`: the I/O board's two
/// address switches and the memory board's one are parts here and not
/// devices there.
#[test]
fn mits_dip_censuses_agree() {
    let cadr = netlist::parse(CADR).unwrap();
    let icmem = control_store_pages();
    let mut boards = vec![
        ("processor", parts_on(&cadr, |p| !icmem.contains(p)), 0, ["cadrwd", "cadr4.wls"]),
        ("control store", parts_on(&cadr, |p| icmem.contains(p)), 0, ["cadrwd", "icmem3.wls"]),
    ];
    let rest = [
        ("bus interface", BUSINT, ["cadr1", "busint.wls"]),
        ("memory", CADRM, ["cadrm", "mem.wls"]),
        ("I/O board", CADRIO, ["cadrio", "iob.wls"]),
        ("LISPM TV", LISPMTV, ["cadrtv", "lmtv4b.wls"]),
        // The disk controller is the one board whose two MIT documents
        // disagree: its census counts four devices its parts list does not
        // stuff and the sheets do not draw. `dc.prt` and the drawings agree
        // with each other at 171, so they are what muir follows, and the
        // census is held here at its own figure rather than averaged away
        // --- discrepancy 70.
        ("disk controller", CADRDC, ["cadrdc", "dc.wls"]),
    ];
    let parsed: Vec<_> =
        rest.iter().map(|(n, t, w)| (*n, netlist::parse(t).unwrap(), *w)).collect();
    for (name, n, wls) in &parsed {
        boards.push((name, parts(n), switches(n), *wls));
    }
    for (board, ours, switches, list) in boards {
        let census = support::dip_census(&support::mit_text(&list));
        let theirs: usize =
            census.iter().filter(|(kind, _)| !is_passive(kind)).map(|(_, n)| n).sum();
        let slack = usize::from(board == "disk controller") * 4;
        assert_eq!(
            theirs + switches,
            ours.len() + slack,
            "{board}: MIT's census has {theirs} devices and {switches} switches it counts as \
             dummies, against {} parts in the netlist",
            ours.len()
        );
    }
}

/// The DIP switches among a board's parts: `SWITCH` on the I/O board,
/// `DIPSW` on the memory board. muir reads both --- the Chaosnet address
/// off one, the board's slot off the other --- so both are parts, and MIT's
/// censuses are the only place they are not devices.
fn switches(n: &Netlist) -> usize {
    n.parts
        .iter()
        .filter(|p| p.kind.contains("SW"))
        .map(|p| p.reference.as_str())
        .collect::<BTreeSet<_>>()
        .len()
}
