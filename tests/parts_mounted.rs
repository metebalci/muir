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
//! this file holds them to it --- and to MIT's own three counts of the same
//! boards, each made by its tooling from the drawings and reaching us by a
//! different route than the sheets did: the parts list `*.prt`, which names
//! every location that was stuffed and what went in it, the DIP census
//! `*.wls`, which counts the packages each part type takes, and the
//! stuffing list `*.stf`, which places every body at a location on a page.
//! Four boards have a parts list, seven have a census, and eight have a
//! stuffing list. The SIMPLE TV has none of the three and its count rests
//! on the drawings alone; the disk multiplexor has only the stuffing list,
//! there being no `dm.wlr` and no census in `dm.wls` (discrepancy 75).
//!
//! **A location holding two devices is the case a board total cannot see.**
//! MIT writes the second body at a location `B05@03` where the first is
//! `B05`, and the netlist has no such distinction --- not one designator in
//! the eight `data/*.netlist` files carries an `@`. So two devices at one
//! location are held apart only by the page they are drawn on, by their
//! type, or by a pin they both claim, and where none of the three separates
//! them [`Netlist::packages`] merges them: one device wired to both their
//! nets, which the `chip` engine would build and run, and which is not
//! MIT's machine. Every count above is of locations and would see nothing.
//! [`every_board_location_holds_the_devices_mit_stuffs_there`] is the count
//! that would: per location and per page, against the stuffing lists.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use muir::netlist::{self, Netlist};

mod support;
use support::{Body, control_store_pages, mit, stuffing_list};

const CADR: &str = include_str!("../data/CADR.netlist");
const BUSINT: &str = include_str!("../data/BUSINT.netlist");
const CADRM: &str = include_str!("../data/CADRM.netlist");
const CADRIO: &str = include_str!("../data/CADRIO.netlist");
const CADRDC: &str = include_str!("../data/CADRDC.netlist");
const SIMPLETV: &str = include_str!("../data/SIMPLETV.netlist");
const LISPMTV: &str = include_str!("../data/LISPMTV.netlist");
const DM: &str = include_str!("../data/DM.netlist");

/// Bypass capacitors, resistor and terminator packs, busbars, pull-up
/// networks, and the bodies a sheet carries with no device in them. MIT's
/// parts lists put these at sub-positions of a location --- `C08@01` beside
/// the chip at `C08` --- which is the shape of the thing: they are not
/// parts of the machine, and nothing simulates them.
///
/// The names are MIT's own body names, which is what the netlist's `kind`
/// and the stuffing lists' `BODY` column both carry, and one body has
/// several of them across the files: a bypass capacitor is `.1UFCAP` on the
/// memory board's sheets, `CAP1` on the multiplexor's, and `BYPASS` in
/// every stuffing list's capacitor page.
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
        || matches!(k.as_str(), "BUSBAR" | "BYPASS" | "PULLUP" | "TRITERM")
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
        ("DM", DM, 62),
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
        // The disk controller's four are not a disagreement between MIT's
        // documents. They are eight 75452 drivers in four sockets: `dc.prt`
        // and `dc.stf` list `A02` and `A02@03`, `B01` and `B01@03`, `B03`
        // and `B03@03`, `B05` and `B05@03`, and the netlist's designators
        // drop MIT's `@nn`, so a count of board sites comes to four less
        // than a count of devices. `dc.wls`'s own trailer settles it:
        // `GRAND TOTAL = 188(171)` over `NUMBER IN PARENS IS REAL (.GE.14
        // PINS) DIPS`, the same 171 `dc.prt` stuffs and the netlist has.
        // The census also gives the 75452 as "15 sections, 8 dips, 1
        // spare", which is those four sites again.
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
        // The disk controller counts eight 75452s where the netlist has four
        // sites; see the note above.
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

/// **Against MIT's own stuffing list, for the board that has no parts list
/// and no census.** `cadrdc/dm.stf` is the disk multiplexor as the
/// stockroom saw it: every location, and every body in it. It reached us
/// the same way the sheets did, which is weaker than the `*.prt` files
/// above, but it is made by different tooling for a different purpose and
/// it is the only second count this board has --- there is no `dm.wlr` and
/// `dm.wls` carries no census (discrepancy 75).
///
/// Both sides agree on 62 devices and on which 62 they are.
#[test]
fn the_multiplexors_count_agrees_with_mits_stuffing_list() {
    let bodies = stuffing_list(&["cadrdc", "dm.stf"]);
    assert_eq!(bodies.len(), 102, "bodies read out of dm.stf");
    let theirs: BTreeSet<&str> =
        bodies.iter().filter(|b| !is_passive(b.kind())).map(|b| b.location.as_str()).collect();
    let ours: BTreeSet<String> = parts(&netlist::parse(DM).unwrap())
        .iter()
        .map(|r| r.strip_prefix('0').expect("a designator here is 0-prefixed").to_string())
        .collect();
    assert_eq!(
        ours.iter().map(String::as_str).collect::<BTreeSet<_>>(),
        theirs,
        "the multiplexor's mounted devices"
    );
    assert_eq!(ours.len(), 62);
}

/// **What MIT stuffs at each board location, page by page.**
fn stuffed_devices(bodies: &[Body]) -> BTreeMap<(&str, &str), Vec<&str>> {
    let mut out: BTreeMap<(&str, &str), Vec<&str>> = BTreeMap::new();
    for body in bodies.iter().filter(|b| !is_passive(b.kind())) {
        // A body's gates may be drawn several times on one page; it is one
        // device there however often it is drawn.
        for page in body.pages() {
            out.entry((page, &body.location)).or_default().push(body.kind());
        }
    }
    out
}

/// The devices at each board location on each page, keyed by page and
/// location: for each, the pins it claims and the body names the drawing
/// gives its gates.
type Devices<'a> = BTreeMap<(&'a str, &'a str), Vec<(BTreeSet<u8>, Vec<&'a str>)>>;

/// **What the netlist has at each board location, page by page:** the
/// records at one location on one page that no pin collision separates,
/// each such group being one body's worth of gates.
///
/// This is [`Netlist::packages`] with the part type left out of the
/// grouping, and the difference is deliberate. `packages` also requires the
/// type to match, so a chip a drawing draws under two body names comes out
/// as two packages of one device: BUSINT DATCTL 0B20 is one 74S00 whose
/// gate with inverted inputs is drawn `OS00L` and whose other three are
/// drawn `74S00`. A pin is the thing a location cannot have twice, so pins
/// are what counts devices.
fn netlist_devices(n: &Netlist) -> Devices<'_> {
    let mut out: Devices = BTreeMap::new();
    for part in n.parts.iter().filter(|p| !is_passive(&p.kind)) {
        let location = part.reference.strip_prefix('0').unwrap_or(&part.reference);
        let here = out.entry((part.page.as_str(), location)).or_default();
        let pins: BTreeSet<u8> = part.pins.iter().map(|&(pin, _)| pin).collect();
        match here.iter_mut().find(|(taken, _)| taken.is_disjoint(&pins)) {
            Some((taken, kinds)) => {
                taken.extend(&pins);
                kinds.push(&part.kind);
            }
            None => here.push((pins, vec![&part.kind])),
        }
    }
    out
}

/// MIT's stuffing lists, one per board, and the netlist each is read
/// against. The other seven in `mit/` are not second copies of these:
/// `cadrio/dm.stf` and `cadrio/iob1.stf` are byte for byte the same files
/// as `cadrdc/dm.stf` and `cadrdc/iob1.stf`; `iob1.stf` is the I/O board of
/// May 1979 and `cadrio/dc.stf` the disk controller of March 1979, both
/// earlier boards than the ones modelled here; `cadrdc/mk.stf` and
/// `cadrio/mk.stf` are the Marksman controller, which muir does not model;
/// and `cadrtv/lmtv.stf` is the SIMPLE TV's, at a revision the drawings
/// have moved past. It names the pages under their pre-rename names ---
/// `SYNRAM` where the sheet is `nsyram.drw` --- puts a 74LS244 at C05 where
/// `data/SIMPLETV.netlist` has a 74S472 there and the 244 at D05, and
/// carries no 74S472 anywhere. So the SIMPLE TV is not checked here, as it
/// is not checked above.
const STUFFING_LISTS: [(&str, &str, [&str; 2]); 8] = [
    ("bus interface", "BUSINT", ["cadr1", "busint.stf"]),
    ("memory", "CADRM", ["cadrm", "mem.stf"]),
    ("I/O board", "CADRIO", ["cadrio", "iob.stf"]),
    ("disk controller", "CADRDC", ["cadrdc", "dc.stf"]),
    ("disk multiplexor", "DM", ["cadrdc", "dm.stf"]),
    ("LISPM TV", "LISPMTV", ["cadrtv", "lmtv4b.stf"]),
    ("processor", "CADR", ["cadrwd", "cadr4.stf"]),
    ("control store", "CADR", ["cadrwd", "icmem3.stf"]),
];

/// The netlist a row of [`STUFFING_LISTS`] names.
fn board(name: &str) -> Netlist {
    let text = match name {
        "BUSINT" => BUSINT,
        "CADRM" => CADRM,
        "CADRIO" => CADRIO,
        "CADRDC" => CADRDC,
        "LISPMTV" => LISPMTV,
        "CADR" => CADR,
        "DM" => DM,
        other => panic!("no netlist called {other}"),
    };
    netlist::parse(text).unwrap()
}

/// **Two devices at one board location must be two devices here.**
///
/// The count above is of locations, and a location is not a device: MIT
/// puts two 8-pin 75452 drivers in one 16-pin footprint and calls them
/// `B05` and `B05@03`. The netlist calls both `0B05`, so
/// [`Netlist::packages`] --- which is what the `chip` engine builds its
/// devices from --- keeps them apart only if they differ in page or in
/// type, or claim a pin in common, and merges them into one device wired
/// to both their nets if none of that separates them. This is the count
/// that would see it, per location and per page.
///
/// **Nothing is merged.** Across the eight boards MIT left a stuffing list
/// for, 2183 devices at 2176 locations, the two agree everywhere but at
/// `SPLIT` below, and that one is muir building two where MIT stuffs one.
/// Seven locations hold two devices on one page --- the multiplexor's four
/// 75452 pairs and the disk controller's three --- and muir builds two at
/// every one of them.
#[test]
fn every_board_location_holds_the_devices_mit_stuffs_there() {
    // One 74LS124 drawn as two bodies, not two devices: `dctmot.drw` draws
    // the chip's two VCO sections separately at 0B04 and each body repeats
    // the package's ground and supply pins, so the two records collide and
    // the netlist keeps them apart. `chip::wire_oscillators` is written for
    // exactly this and takes each section from whichever record carries its
    // pins rather than assuming section 1.
    const SPLIT: (&str, &str, &str) = ("disk controller", "DCTMOT", "B04");
    let (mut read, mut devices, mut locations, mut doubled) = (0usize, 0usize, 0usize, Vec::new());
    for (board_name, netlist_name, list) in STUFFING_LISTS {
        let n = board(netlist_name);
        assert!(
            n.parts.iter().all(|p| !p.reference.contains('@')),
            "{board_name}: a netlist designator names the device at a location, \
             and this check is built on none of them doing so"
        );
        let bodies = stuffing_list(&list);
        read += bodies.len();
        // The one thing the `@nn` is read for: it tells two bodies at one
        // location apart, and it is no use for that if it repeats.
        let mut seen: BTreeSet<(&str, u16)> = BTreeSet::new();
        for b in &bodies {
            assert!(seen.insert((&b.location, b.at)), "{board_name}: two bodies at {}", b.location);
        }
        let theirs = stuffed_devices(&bodies);
        let ours = netlist_devices(&n);
        let pages: BTreeSet<&str> = theirs.keys().map(|&(page, _)| page).collect();
        // `CADR.netlist` holds the processor and the control store both,
        // and the two share designators --- `1A01` is a 74S240 on the
        // processor's VMEMDR and a 74S174 on the control store's OLORD1 ---
        // so a count per location that was not also per board would hold
        // one location against two boards' devices. Which page is on which
        // board is what `cadr/framl.txt` says and what these two lists say,
        // and here they agree.
        if netlist_name == "CADR" {
            let icmem = control_store_pages();
            let on_the_control_store = pages.iter().filter(|p| icmem.contains(**p)).count();
            let want = if board_name == "control store" { pages.len() } else { 0 };
            assert_eq!(
                on_the_control_store, want,
                "{board_name}: the pages its stuffing list names, against `cadr/framl.txt`"
            );
        }
        let ours_here: BTreeMap<_, _> =
            ours.into_iter().filter(|((page, _), _)| pages.contains(page)).collect();
        assert_eq!(
            theirs.keys().copied().collect::<BTreeSet<_>>(),
            ours_here.keys().copied().collect::<BTreeSet<_>>(),
            "{board_name}: the locations MIT stuffs and the locations the netlist has"
        );
        for (&(page, location), mit_bodies) in &theirs {
            let built = &ours_here[&(page, location)];
            devices += mit_bodies.len();
            locations += 1;
            if mit_bodies.len() > 1 {
                doubled.push(format!("{board_name} {page} {location} {mit_bodies:?}"));
            }
            if (board_name, page, location) == SPLIT {
                assert_eq!(built.len(), 2, "{board_name} {page} {location}: the split 74LS124");
                continue;
            }
            assert_eq!(
                built.len(),
                mit_bodies.len(),
                "{board_name} {page} {location}: MIT stuffs {mit_bodies:?} and the netlist \
                 builds {:?} --- fewer here is two of MIT's devices merged into one",
                built.iter().map(|(_, kinds)| kinds).collect::<Vec<_>>()
            );
        }
    }
    assert_eq!(read, 2381, "bodies read out of the eight stuffing lists");
    assert_eq!((devices, locations), (2183, 2176));
    // Not vacuous: MIT does double-stuff a location, and where it does the
    // netlist has two devices there and not one.
    assert_eq!(doubled.len(), 7, "locations MIT stuffs twice on one page: {doubled:?}");
    assert!(doubled.iter().all(|d| d.contains("75452")), "{doubled:?}");
}

/// **And MIT's wire lists say the same, for the boards that have one.**
///
/// The stuffing list is what the stockroom filled a board from and the wire
/// list is what the board was wrapped from --- different documents by
/// different tooling, and the wire list is the board as built. Both write
/// the second device at a location `A02@03`, so both can be asked which
/// locations hold two devices, and they must answer alike.
///
/// They do: of the six boards here with a wire list, only the disk
/// controller has a location with two devices on one page, and it is the
/// same three the stuffing list gives.
#[test]
fn mits_wire_lists_agree_on_which_locations_hold_two_devices() {
    for (board_name, netlist_name, stf, wlr) in [
        ("bus interface", "BUSINT", ["cadr1", "busint.stf"], ["cadr1", "busint.wlr"]),
        ("memory", "CADRM", ["cadrm", "mem.stf"], ["cadrm", "mem.wlr"]),
        ("I/O board", "CADRIO", ["cadrio", "iob.stf"], ["cadrio", "iob.wlr"]),
        ("disk controller", "CADRDC", ["cadrdc", "dc.stf"], ["cadrdc", "dc.wlr"]),
        ("LISPM TV", "LISPMTV", ["cadrtv", "lmtv4b.stf"], ["cadrtv", "lmtv4b.wlr"]),
        ("processor", "CADR", ["cadrwd", "cadr4.stf"], ["cadrwd", "cadr4.wlr"]),
        ("control store", "CADR", ["cadrwd", "icmem3.stf"], ["cadrwd", "icmem3.wlr"]),
    ] {
        let n = board(netlist_name);
        // How many devices the wire list wires at each location on each
        // page: the `@nn` values it names there, and not the body names,
        // one chip being drawn under several of those on one sheet ---
        // BUSINT UBMAST C02 is a 74LS74 whose halves are `74LS74I` and
        // `74LS74`, both `C02`, one device. A connector pin is not a
        // device and a pin whose page was cut off cannot be placed at all.
        let mut wired: BTreeMap<(&str, &str), BTreeSet<u16>> = BTreeMap::new();
        let signals = support::wire_list(&n, &wlr);
        for pin in signals.iter().flat_map(|s| &s.pins) {
            if pin.page.is_empty() || pin.body == "CON" || is_passive(&pin.body) {
                continue;
            }
            let at: u16 = pin.location.split_once('@').map_or(0, |(_, at)| {
                at.parse()
                    .unwrap_or_else(|_| panic!("{board_name}: {} is no location", pin.location))
            });
            wired.entry((&pin.page, pin.slot())).or_default().insert(at);
        }
        let theirs: BTreeSet<(&str, &str)> =
            wired.iter().filter(|(_, at)| at.len() > 1).map(|(&k, _)| k).collect();
        let bodies = stuffing_list(&stf);
        let stuffed: BTreeSet<(&str, &str)> = stuffed_devices(&bodies)
            .into_iter()
            .filter(|(_, mit_bodies)| mit_bodies.len() > 1)
            .map(|(k, _)| k)
            .collect();
        assert_eq!(
            theirs, stuffed,
            "{board_name}: the locations the wire list wires two devices at, against the \
             locations the stuffing list stuffs two bodies at"
        );
    }
}
