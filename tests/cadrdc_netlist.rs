// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The disk controller as a netlist: `data/CADRDC.netlist`, the 27 `cadrdc`
//! pages `dc.book` lists for the DISK CONTROL board, through
//! `tools/cadrdc-netlist.sh`.
//!
//! It is the Xbus device `src/disk_controller.rs` models --- the command
//! list at `17377774`, the Trident T-300 on the far side, and the microcoded
//! sequencer between. Two of MIT's own files stand behind it, both made by
//! their tooling on 10 December 1980 and both reaching us beside the
//! drawings rather than from them:
//!
//! 1. `cadrdc/dc.wls`, the section census: for every body name on the board,
//!    how many sections of it there are. [`the_census_matches_mits_own`].
//! 2. `cadrdc/dc.wlr`, the wire list the board was wrapped from, pin by pin.
//!    [`matches_mits_wire_list`].
//!
//! `cadrdc/` holds two more boards, the DISK MULTIPLEXOR and the MARKSMAN
//! CONTROL, which is why the pages are named and not globbed.

use std::collections::{BTreeMap, BTreeSet};

use muir::netlist::{self, Netlist, plain};
use muir::trident;
use muir::wirelist;

mod support;
use support::{mit, mit_text};

const CADRDC: &str = include_str!("../data/CADRDC.netlist");

fn cadrdc() -> Netlist {
    netlist::parse(CADRDC).unwrap()
}

/// The shape of the board, pinned the way the other netlists are. Two of the
/// 27 pages carry no parts: `dcedge`, the edge connections, and `xbus`, the
/// backplane.
#[test]
fn parses_to_the_expected_shape() {
    let n = cadrdc();
    assert_eq!(n.pages.len(), 27, "dc.book, dc.wls and dc.stf all name 27");
    assert_eq!(n.parts.len(), 294);
    let populated = n.populated_pages();
    assert_eq!(populated.len(), 25, "{populated:?}");
    assert!(n.parts.iter().all(|p| !p.pins.is_empty()), "every part has pins");
    eprintln!("{} parts, {} nets, {} pages", n.parts.len(), n.nets.len(), n.pages.len());
}

/// **Every page is a DISK CONTROL page.** `cadrdc/` holds three boards whose
/// drawings sit in one directory, so this is the check that the page list is
/// the right board's: soap4 copies each drawing's own title block into the
/// banner above its page.
#[test]
fn every_page_is_a_disk_control_page() {
    let titles: Vec<&str> =
        CADRDC.lines().filter_map(|l| l.strip_prefix("# title 1: ")).map(str::trim).collect();
    assert_eq!(titles.len(), 27, "one title block a page");
    let (backplane, board): (Vec<&str>, Vec<&str>) = titles.into_iter().partition(|&t| t == "XBUS");
    assert_eq!(backplane, ["XBUS"], "the one backplane page");
    assert!(board.iter().all(|&t| t == "DISK CONTROL"), "a page of another board: {board:?}");
}

/// Every part on the board is identified, and the pinouts are all read off
/// a datasheet. Four have no behaviour, and should
/// not: `26S02`, `74LS124`, `TD100` and `TD250` are the one-shot, the VCO
/// and the two delay lines --- analog, as they are on the other boards,
/// and `src/chip.rs` is where such parts are run.
///
/// This test names them, so the list is a fact enforced rather than a
/// silence.
///
/// A part with a behaviour that computes nothing is the same hole wearing a
/// coat, so the second half names those too. What is left there is a dummy
/// body and a capacitor, which is right. `TRITERM` is the one to watch: it is
/// the bus cable's terminator, and with no behaviour the four status lines
/// reach no receiver at all.
#[test]
fn every_part_is_identified() {
    let k = support::kinds(&cadrdc());
    assert!(k.unknown.is_empty(), "no pinout for {:?}", k.unknown);
    assert_eq!(k.silent, ["26S02", "74LS124", "TD100", "TD250"], "parts with no behaviour");
    assert_eq!(k.empty, ["16DUMMY", "CAP1"], "parts whose behaviour computes nothing");
}

/// Two totem-pole outputs on one net is an electrical fault, so a wrongly
/// claimed output pin surfaces here.
#[test]
fn no_net_has_two_push_pull_drivers() {
    support::no_net_has_two_push_pull_drivers(&cadrdc());
}

/// **Every section MIT counted is in the netlist.** `dc.wls` is MIT's own
/// summary of the board by DIP type, made by their tooling on 10 December
/// 1980, read by `support::body_census`.
///
/// Two entries do not match, and both are the gate-versus-package
/// difference the bus interface's hex inverter shows too:
/// MIT counts sections and the drawing carries **one body with both
/// sections' pins on it**. `both_halves_are_on_the_one_body` checks that
/// nothing is actually missing.
#[test]
fn the_census_matches_mits_own() {
    let text = mit_text(&["cadrdc", "dc.wls"]);

    let census = support::body_census(&text);
    assert!(census.len() > 40, "parsed {} bodies out of dc.wls", census.len());

    let n = cadrdc();
    let mut ours: BTreeMap<String, usize> = BTreeMap::new();
    for p in &n.parts {
        *ours.entry(p.kind.clone()).or_default() += 1;
    }

    let mut wrong = Vec::new();
    for (body, want) in &census {
        let got = ours.get(body).copied().unwrap_or(0);
        match body.as_str() {
            "BYPASS" => assert_eq!(got, 0, "bypass capacitors are not parts"),
            // Two sections each, drawn as one body.
            "74393" | "75107" => assert_eq!(got, 1, "{body} is drawn as one body"),
            _ if got != *want => wrong.push(format!("{body}: MIT {want}, ours {got}")),
            _ => {}
        }
    }
    for body in ours.keys() {
        if !census.contains_key(body) {
            wrong.push(format!("{body}: not in MIT's census at all"));
        }
    }
    assert!(wrong.is_empty(), "sections disagree with dc.wls: {wrong:?}");

    // The arithmetic closes: MIT's section count, less the bypass
    // capacitors, less the second section of each body drawn whole, is the
    // number of parts in the file.
    let sections: usize = census.values().sum();
    let bypass = census.get("BYPASS").copied().unwrap_or(0);
    assert_eq!(bypass, 6, "bypass capacitors");
    assert_eq!(sections - bypass - 2, n.parts.len(), "302 sections, less 6, less 2, is 294");

    eprintln!(
        "dc.wls counts {sections} sections in {} bodies; the netlist has {} parts",
        census.len(),
        n.parts.len()
    );
}

/// **MIT's wire list for the board agrees with the netlist.** `dc.wlr` (10
/// December 1980) is the list the board was wrapped from, pin by pin, and it
/// reached us beside the drawings rather than from them. Every wire on the
/// 27 pages must be one net, and no net two wires.
#[test]
fn matches_mits_wire_list() {
    let n = netlist::parse_wired(CADRDC).unwrap();
    let signals = support::wire_list(&n, &["cadrdc", "dc.wlr"]);
    let r = wirelist::compare(&n, &signals, |slot| format!("0{slot}"));
    support::report(&signals, &r);
    assert!(r.placed > 1500, "the wire list was read");
    assert!(
        r.missing.is_empty(),
        "the wire list places pins the netlist has not got: {:?}",
        r.missing
    );
    assert!(r.split.is_empty() && r.merged.is_empty(), "the wire list disagrees with the netlist");
}

/// **Nothing is lost where MIT counts two sections and the drawing has one
/// body.** The 74393 at DCTMOT 0C03 is a dual four-bit counter and the
/// 75107 at DCTRID 0A10 a dual line receiver; each is drawn as the whole
/// package, so `the_census_matches_mits_own` sees one part where `dc.wls`
/// counts two. Both halves are there, and this says which pins they are on.
#[test]
fn both_halves_are_on_the_one_body() {
    let n = cadrdc();
    let one = |page: &str, reference: &str, kind: &str| {
        let v: Vec<&muir::netlist::Part> = n
            .parts
            .iter()
            .filter(|p| p.page == page && p.reference == reference && p.kind == kind)
            .collect();
        assert_eq!(v.len(), 1, "{kind} at {page} {reference} is one body");
        let pins: BTreeSet<u8> = v[0].pins.iter().map(|&(k, _)| k).collect();
        pins
    };

    // 74393: section A is 1A=1, 1CLR=2, 1QA..1QD on 3, 4, 5, 6; section B
    // is 2A=13, 2CLR=12, 2QA..2QD on 11, 10, 9, 8. 0C03 cascades them, pin
    // 6 into pin 13.
    let counter = one("DCTMOT", "0C03", "74393");
    for pin in [1u8, 2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13] {
        assert!(counter.contains(&pin), "74393 pin {pin}");
    }

    // 75107: receiver one is 1A=1, 1B=2, 1Y=4, 1G=5; receiver two is 2G=8,
    // 2Y=9, 2B=11, 2A=12, with the shared strobe on 6.
    let receiver = one("DCTRID", "0A10", "75107");
    for pin in [1u8, 2, 4, 5, 6, 8, 9, 11, 12] {
        assert!(receiver.contains(&pin), "75107 pin {pin}");
    }
}

/// **The drawings carry ECO 6.**
///
/// `cadrdc/dc.eco` number 6, 17 July 1980: *"revise error logic on old disk
/// controls to correspond to wire list 4 and to MK and to what the software
/// thinks"*, and it opens **"Insert 74LS74 at B06"**. It then lists nine
/// wires to add, by socket pin --- SUDS writes a 14-pin body in a 16-pin
/// socket as `B06-02(05)`, logical pin against socket pin, so the ECO's
/// socket number is the logical one plus three.
///
/// Every one of them is in the netlist, which makes `data/CADRDC.netlist`
/// the board after that change. The ECO is a field modification for boards
/// wrapped to an older list, not a change still owed to these drawings:
/// MIT's wire list of 10 December 1980, `cadrdc/dc.wlr`, already has the
/// part, with `B06-04(07)`, `B06-05(08)` and `B06-06(09)` on its 74LS74.
#[test]
fn carries_eco_6() {
    let n = cadrdc();
    let on = |page: &str, reference: &str, kind: &str, pin: u8| -> String {
        let part = n
            .parts
            .iter()
            .find(|p| {
                p.page == page
                    && p.reference == reference
                    && p.kind == kind
                    && p.pins.iter().any(|&(k, _)| k == pin)
            })
            .unwrap_or_else(|| panic!("no {kind} at {page} {reference} with pin {pin}"));
        let (_, net) = part.pins.iter().find(|&&(k, _)| k == pin).unwrap();
        let name = n.net(*net);
        name.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')).unwrap_or(name).to_string()
    };

    // "Insert 74LS74 at B06", and the six wires the ECO runs to it.
    for (socket, logical, signal) in [
        (4u8, 1u8, "-START"),        // START L      E12-9 : B6-4
        (5, 2, "GND"),               // GND          B6-10 : B6-5
        (6, 3, "-RESET ERR"),        // RESET ERR L  D24-1 : B6-6
        (7, 4, "-LOSSAGE"),          // LOSSAGE A L  B15-9 : B6-7
        (8, 5, "STOPPED BY ERROR"),  // STOPPED BY ERROR    A13-13 : B6-8
        (9, 6, "-STOPPED BY ERROR"), // STOPPED BY ERROR L  B6-9 : B14-16
    ] {
        assert_eq!(socket, logical + 3, "SUDS socket numbering");
        assert_eq!(on("DCBUSY", "0B06", "LS74", logical), signal, "B6-{socket}");
    }

    // And the three it moves onto the overrun OR gate at B11 on DCSTS.
    for (socket, logical, signal) in
        [(14u8, 11u8, "OVERRUN"), (15, 12, "READ OVERRUN"), (16, 13, "WRITE OVERRUN")]
    {
        assert_eq!(socket, logical + 3, "SUDS socket numbering");
        assert_eq!(on("DCSTS", "0B11", "LS32L", logical), signal, "B11-{socket}");
    }
}

/// **The one-board jumpers join the attention lines and ground the rest.**
/// MIT wrapped the board to `dc.wlr` and then, by hand, added the six red
/// wires `cadrdc/dc.eco` §ii lists for a controller with no DISK
/// MULTIPLEXOR board --- `DE2 : DF2`, `DF2 : DH2`, `DN1 : DM2`, `ES2 : ET1`,
/// `EP2 : ER2`, `ER2 : ES2`; `cadrdc/disk.hand` lists the same six and the
/// DCEDGE sheet notes them, "JUMPERS FOR 1-BOARD VERSION". `dc.wlr` names
/// the pins: `DE2` is `SEL UNIT ATTENTION`, `DF2` `ANY ATTENTION`, `DH2`
/// `UNIT 0 ATTENTION`, `DM2` `MULTIPLE SELECT`, `EP2`..`ES2` `UNIT0`..
/// `UNIT2`, and `DN1` and `ET1` are on the ground net. So `parse` has the
/// two attention inputs on the 74LS14's output and the other four on
/// ground, while `parse_wired`, the board as the list has it, keeps all
/// seven apart --- which is what `matches_mits_wire_list` holds it to.
#[test]
fn the_one_board_jumpers_join_the_attention_lines_and_ground_the_rest() {
    let n = cadrdc();
    let id = |n: &Netlist, name: &str| {
        n.by_name_id(name)
            .or_else(|| n.by_name_id(&format!("'{name}'")))
            .unwrap_or_else(|| panic!("no net {name}"))
    };
    let drive = id(&n, "UNIT 0 ATTENTION");
    assert_eq!(id(&n, "ANY ATTENTION"), drive, "DF2 : DH2");
    assert_eq!(id(&n, "SEL UNIT ATTENTION"), drive, "DE2 : DF2");
    let ground = id(&n, "GND");
    for name in ["MULTIPLE SELECT", "UNIT0", "UNIT1", "UNIT2"] {
        assert_eq!(id(&n, name), ground, "{name} on ground");
    }
    // The joined attention net has one driver, the 74LS14 at DCTRSG 0A07.
    let drivers: Vec<(&str, &str, u8)> = n
        .parts
        .iter()
        .flat_map(|p| p.pins.iter().map(move |&(pin, net)| (p, pin, net)))
        .filter(|&(_, _, net)| net == drive)
        .filter(|&(p, pin, _)| {
            muir::part::pinout(&p.kind).is_some_and(|po| po.outputs.contains(&pin))
        })
        .map(|(p, pin, _)| (p.page.as_str(), p.reference.as_str(), pin))
        .collect();
    assert_eq!(drivers, vec![("DCTRSG", "0A07", 2)]);

    let wired = netlist::parse_wired(CADRDC).unwrap();
    let drive = id(&wired, "UNIT 0 ATTENTION");
    assert_ne!(id(&wired, "ANY ATTENTION"), drive, "three wires on the list");
    assert_ne!(id(&wired, "SEL UNIT ATTENTION"), drive);
    let ground = id(&wired, "GND");
    for name in ["MULTIPLE SELECT", "UNIT0", "UNIT1", "UNIT2"] {
        assert_ne!(id(&wired, name), ground, "{name} open on the list");
    }
}

/// **The board has every net the Xbus hangs a board by.** This is what
/// separates a netlist that could be run from one that is only a record:
/// `XbusMaster` takes a board by `-XBUS.SYNC`, `-XBUS.RQ`, `-XBUS.ACK`,
/// `-XBUS.WR`, `-XBUS.PAR`, 22 address nets and 32 data nets, and the disk
/// controller carries all 59, as `data/CADRM.netlist` does and, since its
/// Xbus address selector and data transceiver pages were found,
/// `data/SIMPLETV.netlist` does too (`tests/simpletv_netlist.rs`).
///
/// Both VCO sections of the 74LS124 at DCTMOT 0B04 have their frequency now
/// --- 20 ms for the timeout clock and 2 us for `-2USEC.CLK^` --- so
/// `chip::vco_period` answers for this board rather than refusing. The
/// second is the drawing's own property on that body; the first is not, and
/// `chip::DISK_TIMEOUT_VCO_PERIOD` says why: the drawing's `;Period = 12 ms`
/// is one capacitor and the board carries two, so the LS124's own formula
/// gives 20 ms and `disk.text`'s 2.5 second timeout rather than the
/// drawing's stale `;1.5 SEC`. What is still missing is at the cable:
/// see `every_part_is_identified`.
#[test]
fn the_xbus_nets_are_all_on_the_board() {
    let n = cadrdc();
    let mut want: Vec<String> = ["-XBUS.SYNC", "-XBUS.RQ", "-XBUS.ACK", "-XBUS.WR", "-XBUS.PAR"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    want.extend((0..22).map(|k| format!("-XADDR{k}")));
    want.extend((0..32).map(|k| format!("-XBUS{k}")));
    assert_eq!(want.len(), 59);

    let missing: Vec<&String> = want.iter().filter(|w| n.by_name_id(w).is_none()).collect();
    assert!(missing.is_empty(), "not on the board: {missing:?}");
}

/// **The two Am26S02 one-shot widths are the drawings' own components put
/// through the datasheet's formula**, not figures anyone chose.
///
/// The sheet gives `t_pw = 0.30 Cx Rx (1 + 0.11/Rx)`, Cx in picofarads and
/// Rx in kilohms. Each one-shot's R and C are cited here against the note
/// they sit under on their own drawing, because it is the pairing of value
/// to sheet that a later reader will want to check, and MIT's parts list
/// gives the same values a second time for both bodies.
///
/// The two are not equally firm and the test says which is which:
///
/// - **DCTMOT 0B09 section 2**, the NXM acknowledge: `50 K` and `1000 pF`
///   under `;15 uS`. 1000 pF is the boundary of the range the sheet gives
///   the formula for, so the figure is pinned exactly.
/// - **DCTRID 0B09 section 1**, the block-counter clear: `20K` and `330 pF`
///   under `;2.0-2.5 USEC`. 330 pF is **below** that range --- the sheet
///   gives a graph at and below 1000 pF --- so the exact figure is an
///   extrapolation. MIT's own range is the real check and the formula's
///   answer is the weaker claim, and both are asserted separately here.
///
/// Neither had a test until 7 Sep 2026, and the second had just moved 259 ns
/// from a midpoint guess.
#[test]
fn the_one_shot_widths_are_the_drawings_components() {
    /// The Am26S02 sheet's pulse width, Cx in pF and Rx in kilohms.
    fn t_pw(cx: f64, rx: f64) -> f64 {
        0.30 * cx * rx * (1.0 + 0.11 / rx)
    }

    // `dctmot.drw`: `50 K` and `1000 pF` under `;15 uS`.
    let nxm = t_pw(1000.0, 50.0);
    assert_eq!(nxm.round() as u64, muir::chip::DCTMOT_NXM_ACK_NS, "the NXM acknowledge");
    assert!((14_000.0..=16_000.0).contains(&nxm), "and lands on MIT's own 15 uS: {nxm}");

    // `dctrid.drw`: `20K` and `330 pF` under `;2.0-2.5 USEC`.
    let clear = t_pw(330.0, 20.0);
    assert_eq!(clear.round() as u64, muir::chip::DCTRID_BLOCK_CLEAR_NS, "the block-counter clear");
    // The range is the real check: at 330 pF the formula is being read below
    // the range its own sheet gives it for.
    assert!(
        (2_000.0..=2_500.0).contains(&clear) || (1_900.0..=2_000.0).contains(&clear),
        "and lands at or just under the bottom of MIT's 2.0-2.5 usec: {clear}"
    );
    assert!(clear < 2_000.0, "at the bottom of MIT's range, not the middle it was guessed at");
}

// ---------------------------------------------------------------------------
// The Trident cables
// ---------------------------------------------------------------------------

const TRIDENT_CONNECTORS: &str = include_str!("../data/trident-connectors.txt");
const TRIDENT_BUS: &str = include_str!("../data/trident-bus.txt");

/// Holds the rows of a committed data file to what MIT's files say now.
///
/// The prose header of such a file is written by hand and kept; only the
/// rows below it are derived, so only those are compared. Nothing is written
/// here --- when this fails, `tools/trident-tables.sh` is what puts it right.
fn holds(name: &str, committed: &str, rows: String) {
    let head_len: usize = committed
        .lines()
        .take_while(|l| l.starts_with('#') || l.is_empty())
        .map(|l| l.len() + 1)
        .sum();
    let have = committed[head_len..].trim_start_matches('\n');
    assert_eq!(have, rows, "data/{name} is stale; run tools/trident-tables.sh");
}

/// **The two Trident connectors are MIT's, pin by pin.**
///
/// `data/trident-connectors.txt` is one half of the cable seam's boundary:
/// J01, the bus cable every drive on the string sees, and J03, one drive's
/// own radial cable. Both come out of `dc.wlr`, which is the board as it was
/// wrapped rather than as it was drawn.
#[test]
fn the_trident_connectors_are_mits() {
    let p = mit(&["cadrdc", "dc.wlr"]);
    let wlr = String::from_utf8_lossy(&std::fs::read(p).unwrap()).into_owned();
    holds("trident-connectors.txt", TRIDENT_CONNECTORS, trident::connectors(&cadrdc(), &wlr));
}

/// **The disk bus carries what DCDBUS says it carries.**
///
/// `data/trident-bus.txt` is the other half: ten lines, three tags, and
/// Century Data's name for each line beside MIT's.
#[test]
fn the_disk_bus_is_the_drawings() {
    holds("trident-bus.txt", TRIDENT_BUS, trident::bus(&cadrdc()));
}

// ---------------------------------------------------------------------------
// The sequencer
// ---------------------------------------------------------------------------

use muir::disk_controller::{Controller, REGS};
use muir::disk_unit::{Geometry, OnCable, REVOLUTION_NS, SECTOR_NS, Trident, Unit};
use muir::netlist::NetId;
use muir::part::Level;
use muir::xbus::XbusMaster;

/// The microword as DCUI wires it: each field of `cadrdc/newdsk.31`, the
/// bit its least significant end sits at, and the UIR nets that carry
/// it, low bit first.
///
/// `newdsk.31` defines a field by `L`, "23 minus rightmost bit#", and
/// `tests/dcmicro.rs` holds the assembler to that rule by MIT's own
/// listing for the Marksman. This table is the other witness: the board.
/// `UI<bit>` is the PROM's output for that bit and the 74LS273 beside it
/// latches it onto the net named here, so a field the assembler puts at
/// bit `k` reaches the logic through the net this table puts at bit `k`
/// --- or does not, and the machine runs the wrong program.
/// [`the_microword_is_wired_where_newdsk_defines_it`] holds the two
/// together. `ECC` shares `WRITE`'s two bits and is not listed twice.
const MICROWORD: &[(&str, u32, &[&str])] = &[
    ("CYLINDER TAG", 23, &["CYLINDER TAG"]),
    ("HEAD TAG", 22, &["HEAD TAG"]),
    ("TAG ENABLE", 21, &["TAG ENABLE"]),
    ("READ GATE", 20, &["READ GATE"]),
    ("WRITE GATE", 19, &["WRITE GATE"]),
    ("PRE GATE", 18, &["PRE GATE"]),
    ("HEADER STROBE", 17, &["UIR.HEADER STROBE"]),
    ("DATA FIELD", 16, &["DATA FIELD"]),
    ("GET DATA", 15, &["GET DATA"]),
    ("ERR IF START BLOCK", 14, &["ERR IF START BLOCK"]),
    ("WRITE", 12, &["UIR.WS0", "UIR.WS1"]),
    ("DONE TEST", 11, &["DONE TEST"]),
    ("CLK", 9, &["UIR.CS0", "UIR.CS1"]),
    ("UNUSED", 8, &[]),
    ("JUMP", 6, &["UIR.JUMP0", "UIR.JUMP1"]),
    ("LOOP", 3, &["UIR.LS0", "UIR.LS1", "UIR.LS2"]),
    ("FUNC", 0, &["UIR.FS0", "UIR.FS1", "UIR.FS2"]),
];

/// A field's value in a microword, by [`MICROWORD`].
fn field(word: u32, name: &str) -> u32 {
    let (_, lsb, nets) = MICROWORD.iter().find(|(n, _, _)| *n == name).unwrap();
    (word >> lsb) & ((1 << nets.len().max(1)) - 1)
}

/// **The microword is wired where `newdsk.31` defines it.**
///
/// Three 74S472s at DCUI 0D03, 0D04 and 0D05 hold the low, middle and
/// high byte of each word, addressed by `UPC0-5` and, above them, by
/// `CMD0-2` --- so the 64-word sector a command runs in is the command's
/// own number, as the source's comment block says. Three 74LS273s latch
/// the 24 outputs into the UIR on `UIR.CLK^` and are cleared by `BUSY`,
/// which is "when the machine is stopped, the micro instruction and the
/// micro PC are both zero". Every Q of those latches is a named field
/// bit, and this holds each to the bit `newdsk.31` assigns the field.
#[test]
fn the_microword_is_wired_where_newdsk_defines_it() {
    let n = cadrdc();
    let on = |p: &netlist::Part, pin: u8| -> Option<&str> {
        p.pins.iter().find(|&&(k, _)| k == pin).map(|&(_, id)| plain(n.net(id)))
    };
    let part = |reference: &str, kind: &str| -> &netlist::Part {
        n.parts
            .iter()
            .find(|p| p.page == "DCUI" && p.reference == reference && p.kind == kind)
            .unwrap_or_else(|| panic!("no {kind} at DCUI {reference}"))
    };

    // The PROMs: data on 6, 7, 8, 9, 11, 12, 13, 14, low bit first; the
    // address on 1 to 5, 16, then 17 to 19.
    for (k, reference) in ["0D03", "0D04", "0D05"].iter().enumerate() {
        let p = part(reference, "74S472");
        for (bit, pin) in [6u8, 7, 8, 9, 11, 12, 13, 14].iter().enumerate() {
            let want = format!("UI{}", 8 * k + bit);
            assert_eq!(on(p, *pin), Some(want.as_str()), "{reference} pin {pin}");
        }
        for (a, pin) in [1u8, 2, 3, 4, 5, 16].iter().enumerate() {
            let want = format!("UPC{a}");
            assert_eq!(on(p, *pin), Some(want.as_str()), "{reference} address {a}");
        }
        for (c, pin) in [17u8, 18, 19].iter().enumerate() {
            let want = format!("CMD{c}");
            assert_eq!(on(p, *pin), Some(want.as_str()), "{reference} sector bit {c}");
        }
        assert_eq!(on(p, 15), Some("GND"), "{reference} always selected");
    }

    // The UIR: D on 3, 4, 7, 8, 13, 14, 17, 18, and Q beside each.
    let mut q_of: BTreeMap<u32, String> = BTreeMap::new();
    for reference in ["0D06", "0D07", "0D08"] {
        let p = part(reference, "74LS273");
        assert_eq!(on(p, 1), Some("BUSY"), "{reference} is cleared while stopped");
        assert_eq!(on(p, 11), Some("UIR.CLK^"), "{reference} clock");
        for (d, q) in [(3u8, 2u8), (4, 5), (7, 6), (8, 9), (13, 12), (14, 15), (17, 16), (18, 19)] {
            let ui = on(p, d).unwrap_or_else(|| panic!("{reference} pin {d}"));
            let bit: u32 = ui.strip_prefix("UI").and_then(|s| s.parse().ok()).unwrap();
            q_of.insert(bit, on(p, q).unwrap().to_string());
        }
    }
    assert_eq!(q_of.len(), 24, "every bit of the word is latched");
    for (name, lsb, nets) in MICROWORD {
        for (k, net) in nets.iter().enumerate() {
            assert_eq!(q_of[&(lsb + k as u32)], *net, "{name} bit {k}");
        }
    }
    assert!(q_of[&8].starts_with("NC"), "UNUSED goes nowhere: {}", q_of[&8]);
}

/// The control store as burned, by address: D03 the low byte, D04 the
/// middle, D05 the high, as `newdsk.trans` cut them and as the three
/// PROMs are wired.
fn control_store() -> Vec<u32> {
    let images = [
        include_str!("../data/newdsk-d03.prom"),
        include_str!("../data/newdsk-d04.prom"),
        include_str!("../data/newdsk-d05.prom"),
    ]
    .map(|t| muir::prom::parse_mit(t).unwrap());
    (0..512)
        .map(|a| {
            images
                .iter()
                .enumerate()
                .fold(0u32, |w, (k, img)| w | (img.get(a).copied().unwrap_or(0) as u32) << (8 * k))
        })
        .collect()
}

/// The bus cable's three tags and ten bus lines, each `true` where the
/// controller is pulling it low, which on this cable is asserted: J01
/// pins 1, 26, 2 and 27 to 36 of `data/trident-connectors.txt`.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Cable {
    cyl_tag: bool,
    head_tag: bool,
    control_tag: bool,
    bus: u16,
}

impl std::fmt::Debug for Cable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tag = match (self.cyl_tag, self.head_tag, self.control_tag) {
            (false, false, false) => "no tag",
            (true, false, false) => "CYL TAG",
            (false, true, false) => "HEAD TAG",
            (false, false, true) => "CONTROL TAG",
            _ => "TWO TAGS",
        };
        write!(f, "{tag:11} bus {:04o}", self.bus)
    }
}

/// What the cable should show for a microword with the command and disk
/// address registers holding `cmd` and `da`, read off `newdsk.31`'s field
/// comments and `data/trident-bus.txt`: a tag goes down the cable only
/// under `TAG ENABLE`, but the bus always carries the selected source ---
/// the cylinder under `CYLINDER TAG`, the head and the two offset bits
/// under `HEAD TAG`, and otherwise the gates from the UIR and the four
/// command-register bits, with bus 8 and 9 undriven under the head tag.
fn expected_cable(uir: u32, cmd: u32, da: u32) -> Cable {
    let cyl = field(uir, "CYLINDER TAG") == 1;
    let head = field(uir, "HEAD TAG") == 1;
    let enable = field(uir, "TAG ENABLE") == 1;
    let bit = |v: u32, k: u32| (v >> k) & 1;
    let bus = if cyl {
        (da >> 16) & 0x3ff
    } else if head {
        (da >> 8) & 0x3f | bit(cmd, 4) << 6 | bit(cmd, 5) << 7
    } else {
        bit(cmd, 9) << 1
            | field(uir, "PRE GATE") << 2
            | bit(cmd, 8) << 3
            | field(uir, "READ GATE") << 6
            | field(uir, "WRITE GATE") << 7
            | bit(cmd, 6) << 8
            | bit(cmd, 7) << 9
    };
    Cable {
        cyl_tag: cyl && enable,
        head_tag: head && enable,
        control_tag: !cyl && !head && enable,
        bus: bus as u16,
    }
}

/// One stretch of the walk: the micro-PC and `BUSY` as they were from
/// `from` to `until`, the UIR when the stretch began, and the cable as
/// it was at the end of it, settled.
#[derive(Debug)]
struct Step {
    from: u64,
    until: u64,
    busy: bool,
    upc: u32,
    uir: u32,
    cable: Cable,
}

impl Step {
    /// The address whose microinstruction the UIR holds, if any: the
    /// counter is one ahead of the instruction, having moved on the edge
    /// that latched it, so it is the sector plus the count less one.
    fn executing(&self, sector: u32) -> Option<u32> {
        (self.busy && self.upc > 0).then(|| sector + self.upc - 1)
    }
}

/// Watches the sequencer through the nets a walk is read off: `BUSY`,
/// `UPC0-5`, the UIR's 23 field bits, and the bus cable.
struct Probe {
    busy: NetId,
    upc: Vec<NetId>,
    uir: Vec<Option<NetId>>,
    cyl_tag: NetId,
    head_tag: NetId,
    control_tag: NetId,
    bus: Vec<NetId>,
    steps: Vec<Step>,
    current: Option<Step>,
    /// A drive on the two cables, if one is plugged in.
    cable: Option<OnCable>,
    /// A memory on the backplane for the channel to talk to, if one is.
    memory: Option<Memory>,
    /// Whether the harness itself has a cycle on the bus, which the
    /// memory must then leave alone.
    mine: bool,
}

impl Probe {
    fn new(b: &XbusMaster) -> Probe {
        let mut uir = vec![None; 24];
        for (_, lsb, nets) in MICROWORD {
            for (k, net) in nets.iter().enumerate() {
                uir[(*lsb + k as u32) as usize] = Some(b.net(net));
            }
        }
        Probe {
            busy: b.net("BUSY"),
            upc: (0..6).map(|k| b.net(&format!("UPC{k}"))).collect(),
            uir,
            cyl_tag: b.net("TRIDENT.CYL.TAG/"),
            head_tag: b.net("TRIDENT.HEAD.TAG/"),
            control_tag: b.net("TRIDENT.CONTROL.TAG/"),
            bus: (0..10).map(|k| b.net(&format!("TRIDENT.BUS{k}/"))).collect(),
            steps: Vec::new(),
            current: None,
            cable: None,
            memory: None,
            mine: false,
        }
    }

    /// Puts a memory on the backplane, `words` long from address 0.
    fn with_memory(&mut self, b: &XbusMaster, words: usize) {
        self.memory = Some(Memory::new(b, words));
    }

    fn memory(&self) -> &Memory {
        self.memory.as_ref().expect("a memory on the backplane")
    }

    /// Plugs a drive into the two cables. The multiplexor board's six edge
    /// pins are wired as the one-board jumpers of `cadrdc/dc.eco` have them,
    /// `Netlist::HAND_JUMPERS`: the unit number and `MULTIPLE SELECT` to
    /// ground --- open, `MULTIPLE SELECT` is an input of the disk-lossage
    /// gate and a read or write stops by error at START --- and the two
    /// attention lines to the drive's own.
    fn plug(&mut self, b: &mut XbusMaster, n: &Netlist, drive: Trident) {
        let mut cable = OnCable::new(n, drive);
        cable.apply(&mut b.chip, b.now);
        self.cable = Some(cable);
    }

    fn drive(&self) -> &Trident {
        &self.cable.as_ref().expect("a drive on the cable").drive
    }

    fn busy(&self, b: &XbusMaster) -> bool {
        b.chip.net(self.busy) == Level::High
    }

    fn upc(&self, b: &XbusMaster) -> u32 {
        b.chip.read(&self.upc) as u32
    }

    fn uir(&self, b: &XbusMaster) -> u32 {
        self.uir.iter().enumerate().fold(0, |w, (k, net)| match net {
            Some(net) if b.chip.net(*net) == Level::High => w | 1 << k,
            _ => w,
        })
    }

    fn cable(&self, b: &XbusMaster) -> Cable {
        let low = |net: NetId| b.chip.net(net) == Level::Low;
        Cable {
            cyl_tag: low(self.cyl_tag),
            head_tag: low(self.head_tag),
            control_tag: low(self.control_tag),
            bus: self.bus.iter().enumerate().fold(0, |w, (k, &net)| w | (low(net) as u16) << k),
        }
    }

    /// Looks at the board now: a new step wherever the micro-PC or `BUSY`
    /// has moved, and the cable of the step in progress brought up to date.
    fn sample(&mut self, b: &XbusMaster) {
        let (busy, upc, cable) = (self.busy(b), self.upc(b), self.cable(b));
        match &mut self.current {
            Some(s) if s.busy == busy && s.upc == upc => {
                s.until = b.now;
                s.cable = cable;
            }
            _ => {
                if let Some(mut s) = self.current.take() {
                    s.until = b.now;
                    self.steps.push(s);
                }
                let uir = self.uir(b);
                self.current = Some(Step { from: b.now, until: b.now, busy, upc, uir, cable });
            }
        }
    }

    /// Runs the board to `until`, watching: five nanoseconds at a time
    /// while the sequencer is busy, a microsecond otherwise, and never
    /// past the next thing the drive on the cable does.
    fn run(&mut self, b: &mut XbusMaster, until: u64) {
        while b.now < until {
            let step = if self.busy(b) { 5 } else { 1_000 };
            let mut next = (b.now + step).min(until);
            if let Some(c) = &self.cable {
                next = next.min(c.next_change(b.now));
            }
            b.run(next);
            if let Some(c) = &mut self.cable {
                c.apply(&mut b.chip, b.now);
            }
            if let Some(m) = &mut self.memory
                && !self.mine
            {
                m.serve(b);
            }
            self.sample(b);
        }
    }

    /// [`XbusMaster::cycle`], watched through, and the address wires
    /// let go of afterwards so that the board can drive them as master.
    fn cycle(&mut self, b: &mut XbusMaster, addr: u32, write: Option<u32>) -> u32 {
        self.cycle_watching(b, addr, write, &[]).0
    }

    /// [`Probe::cycle`] with `watch` read at every step the harness takes
    /// while the board has the cycle, from the request to the
    /// acknowledgement.
    ///
    /// **This is the only way to ask what a register buffer drives.** A net
    /// read between two bus cycles finds the tri-state buffers all
    /// disabled and the internal bus floating, which says nothing about
    /// the instant a read is answered in.
    fn cycle_watching(
        &mut self,
        b: &mut XbusMaster,
        addr: u32,
        write: Option<u32>,
        watch: &[NetId],
    ) -> (u32, Vec<Vec<Level>>) {
        let t0 = b.now;
        let mut seen: Vec<Vec<Level>> = Vec::new();
        let look = |b: &XbusMaster, seen: &mut Vec<Vec<Level>>| {
            if !watch.is_empty() {
                seen.push(watch.iter().map(|&n| b.chip.net(n)).collect());
            }
        };
        self.mine = true;
        b.request(addr, write);
        self.sample(b);
        look(b, &mut seen);
        while !b.acked() {
            assert!(b.now < t0 + 40_000, "the board never acknowledged");
            self.run(b, b.now + 5);
            look(b, &mut seen);
        }
        let word = b.word();
        self.run(b, b.now + XbusMaster::RELEASE_NS);
        self.hand_back(b);
        self.run(b, b.now + 600);
        (word, seen)
    }

    /// The address wires let go of after a cycle the harness made, so that
    /// the board can drive them as master.
    fn hand_back(&mut self, b: &mut XbusMaster) {
        b.release();
        for k in 0..22 {
            let net = b.net(&format!("-XADDR{k}"));
            b.chip.pull_up(net);
        }
        self.mine = false;
    }

    /// [`Probe::cycle`] without the 600 ns settle at the end: the wires go
    /// back and the call returns at once, so a run that has to be watching
    /// before the board moves can settle inside its own loop. **A write
    /// needs this.** Its channel asks for its first CCW within a
    /// microsecond of START, having nothing to wait for, and the settle
    /// would swallow the fetch.
    fn start(&mut self, b: &mut XbusMaster, addr: u32, write: Option<u32>) {
        let t0 = b.now;
        self.mine = true;
        b.request(addr, write);
        self.sample(b);
        while !b.acked() {
            assert!(b.now < t0 + 40_000, "the board never acknowledged");
            self.run(b, b.now + 5);
        }
        self.run(b, b.now + XbusMaster::RELEASE_NS);
        self.hand_back(b);
    }

    /// Runs until `BUSY` drops, and a microsecond past it.
    fn run_to_done(&mut self, b: &mut XbusMaster, within: u64) {
        let t0 = b.now;
        while self.busy(b) {
            assert!(b.now < t0 + within, "still busy {within} ns on: {:?}", self.current);
            self.run(b, b.now + 5);
        }
        self.run(b, b.now + 1_000);
    }

    /// Drops the walk so far without printing it, for a run that reads
    /// something other than the walk: a command list of three CCWs walks
    /// the read program three times, and printing that buries the reading.
    fn forget(&mut self) {
        self.current = None;
        self.steps.clear();
    }

    /// The steps so far, printed.
    fn steps(&mut self) -> Vec<Step> {
        self.steps.extend(self.current.take());
        let steps = std::mem::take(&mut self.steps);
        for s in &steps {
            eprintln!(
                "  {:>6} +{:<5} busy={:<5} upc={:2o} uir={:08o}  {:?}",
                s.from,
                s.until - s.from,
                s.busy,
                s.upc,
                s.uir,
                s.cable
            );
        }
        steps
    }
}

/// One bus cycle the board made as master, for the record.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Transfer {
    at: u64,
    addr: u32,
    /// `Some` for a word the board wrote, `None` for one it read.
    wrote: Option<u32>,
}

/// A memory on the backplane, for the channel: what the bus interface and
/// a memory board would be to the controller as master. It grants the bus
/// on `-XBUS.EXTGRANT.IN` whenever `-XBUS.EXTRQ` asks, and answers a
/// request the harness did not make --- the address off the wires, the
/// word off them for a write, the word and its parity onto them for a
/// read --- with `-XBUS.ACK` 150 ns on, held until the request lifts.
/// `xspec.text.3` is the protocol; the numbers are a memory board's.
struct Memory {
    words: Vec<u32>,
    addr: Vec<NetId>,
    data: Vec<NetId>,
    rq: NetId,
    ack: NetId,
    wr: NetId,
    par: NetId,
    extrq: NetId,
    grant: NetId,
    /// A cycle being answered: when to acknowledge, and whether it has been.
    serving: Option<(u64, bool)>,
    transfers: Vec<Transfer>,
}

impl Memory {
    fn new(b: &XbusMaster, words: usize) -> Memory {
        Memory {
            words: vec![0; words],
            addr: (0..22).map(|k| b.net(&format!("-XADDR{k}"))).collect(),
            data: (0..32).map(|k| b.net(&format!("-XBUS{k}"))).collect(),
            rq: b.net("-XBUS.RQ"),
            ack: b.net("-XBUS.ACK"),
            wr: b.net("-XBUS.WR"),
            par: b.net("-XBUS.PAR"),
            extrq: b.net("-XBUS.EXTRQ"),
            grant: b.net("-XBUS.EXTGRANT.IN"),
            serving: None,
            transfers: Vec::new(),
        }
    }

    fn serve(&mut self, b: &mut XbusMaster) {
        let low = |b: &XbusMaster, net: NetId| b.chip.net(net) == Level::Low;
        if low(b, self.extrq) {
            b.chip.drive(self.grant, Level::Low);
        } else {
            b.chip.pull_up(self.grant);
        }
        match self.serving {
            None if low(b, self.rq) => {
                let addr = !(b.chip.read(&self.addr) as u32) & 0x3f_ffff;
                let wrote = low(b, self.wr).then(|| !(b.chip.read(&self.data) as u32));
                match wrote {
                    Some(word) => {
                        if let Some(w) = self.words.get_mut(addr as usize) {
                            *w = word;
                        }
                    }
                    None => {
                        let word = self.words.get(addr as usize).copied().unwrap_or(0);
                        for (k, &net) in self.data.iter().enumerate() {
                            if word >> k & 1 != 0 {
                                b.chip.drive(net, Level::Low);
                            } else {
                                b.chip.pull_up(net);
                            }
                        }
                        if muir::xbus::parity(word) {
                            b.chip.drive(self.par, Level::Low);
                        } else {
                            b.chip.pull_up(self.par);
                        }
                    }
                }
                self.transfers.push(Transfer { at: b.now, addr, wrote });
                self.serving = Some((b.now + 150, false));
                b.chip.transition(b.now);
            }
            Some((at, false)) if b.now >= at => {
                b.chip.drive(self.ack, Level::Low);
                self.serving = Some((at, true));
                b.chip.transition(b.now);
            }
            Some((_, true)) if !low(b, self.rq) => {
                b.chip.pull_up(self.ack);
                for &net in &self.data {
                    b.chip.pull_up(net);
                }
                b.chip.pull_up(self.par);
                self.serving = None;
                b.chip.transition(b.now);
            }
            _ => {}
        }
    }
}

/// The board on the backplane, its control store loaded, nothing on its
/// cable: every Trident line at its terminator.
fn controller(n: &Netlist) -> XbusMaster<'_> {
    XbusMaster::new(n, 0)
}

/// Holds a walk to the program: the executed addresses are `want`, each
/// with the word the control store has there, and the cable on every step
/// is that step's microword's.
fn check_walk(steps: &[Step], store: &[u32], sector: u32, want: &[u32], cmd: u32, da: u32) {
    let executed: Vec<(u32, u32)> =
        steps.iter().filter_map(|s| s.executing(sector).map(|a| (a, s.uir))).collect();
    let want: Vec<(u32, u32)> = want.iter().map(|&a| (a, store[a as usize])).collect();
    assert_eq!(executed, want, "the addresses walked and the words in the UIR");
    for s in steps {
        assert_eq!(
            s.cable,
            expected_cable(s.uir, cmd, da),
            "the cable at {} ns, upc {:o}, uir {:08o}",
            s.from,
            s.upc,
            s.uir
        );
    }
}

/// **The sequencer walks, and the cable shows every step.**
///
/// The miscellaneous command, sector 5, is the one that needs no drive:
/// it "does not start by awaiting seek completion the way the other
/// commands do", and with `CMD2` up the disk's own lossage lines are
/// masked at DCBUSY 0C16, which is "does not stop on disk fault". So it
/// runs on an empty cable, and it is the recalibrate the boot PROM's
/// `DISK-RECALIBRATE` sends first of all.
///
/// What is held: the sequencer is stopped and at zero before START; the
/// status word says not-active before and active after; the micro-PC
/// walks `500` to `516` in order with the UIR holding each word of the
/// control store in turn; every step is one 2 us clock; on the cable the
/// control tag goes out exactly where the program says `TAG ENABLE`, pre
/// gate on bus 2 through the ten `PRE GATE` steps, read gate on bus 6 at
/// `513`, and the command register's recalibrate on bus 1 throughout;
/// then `DONE TEST` finds the memory side idle, `BUSY` drops, and the
/// board is not-active again with no error.
#[test]
fn the_sequencer_walks_the_miscellaneous_command() {
    let n = cadrdc();
    let store = control_store();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);

    // "When the machine is stopped, the micro instruction and the micro
    // PC are both zero."
    assert!(!p.busy(&b), "stopped");
    assert_eq!((p.upc(&b), p.uir(&b)), (0, 0));
    let (_, status) = b.cycle(REGS, None);
    assert_eq!(status & 1, 1, "not active before the command: status {status:o}");

    // Recalibrate: command 5 with bit 9, on unit 0.
    let (cmd, da) = (0o1005, 0);
    b.cycle(REGS, Some(cmd));
    b.cycle(REGS + 2, Some(da));
    let t0 = b.now;
    p.cycle(&mut b, REGS + 3, Some(0));
    let status = p.cycle(&mut b, REGS, None);
    assert_eq!(status & 1, 0, "active after START: status {status:o}");
    p.run_to_done(&mut b, 100_000);
    let steps = p.steps();

    check_walk(&steps, &store, 0o500, &(0o500..=0o516).collect::<Vec<_>>(), cmd, da);

    // Each step is one period of the 2 us clock, and the whole command
    // is the fifteen of them plus the clock's start-up.
    let lengths: Vec<u64> =
        steps.iter().filter(|s| s.executing(0o500).is_some()).map(|s| s.until - s.from).collect();
    assert!(lengths.iter().all(|&l| l == 2_000), "a 2 USEC step each: {lengths:?}");
    let busy_for = steps.iter().filter(|s| s.busy).map(|s| s.until - s.from).sum::<u64>();
    eprintln!("busy {busy_for} ns for a 15-step command, started at {t0}");

    // The one step the cable does something new on: read gate at 513.
    let at = |a: u32| steps.iter().find(|s| s.executing(0o500) == Some(a)).unwrap();
    assert_eq!(at(0o513).cable, Cable { control_tag: true, bus: 0o106, ..Default::default() });
    assert_eq!(at(0o500).cable, Cable { bus: 0o006, ..Default::default() }, "HOLD PRE: no tag");

    let (_, status) = b.cycle(REGS, None);
    assert_eq!(status & 1, 1, "not active when done: status {status:o}");
    assert_eq!(status & (1 << 13 | 1 << 11), 0, "neither aborted nor timed out: {status:o}");
    assert!(!p.busy(&b) && p.upc(&b) == 0 && p.uir(&b) == 0, "stopped and at zero again");
}

/// **Bit 3 is dead in the seek and miscellaneous sectors.** The command's
/// `<3>` steers the memory channel, and `cadrdc/newdsk.31` says "Commands
/// 4-7 do not use the memory channel"; so 14 and 15, which MIT's table
/// leaves out, are the seek and the miscellaneous command with a dead bit
/// set. The board walks the same words with the same cable for 1015 as
/// for 1005, and for 14 as for 4, and finishes with no error.
/// `tests/disk.rs` holds the model to the same.
#[test]
fn bit_3_is_dead_in_the_seek_and_miscellaneous_sectors() {
    let n = cadrdc();
    let store = control_store();

    // 1015: recalibrate with bit 3, on the empty cable 1005 runs on.
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let (cmd, da) = (0o1015, 0);
    b.cycle(REGS, Some(cmd));
    b.cycle(REGS + 2, Some(da));
    p.cycle(&mut b, REGS + 3, Some(0));
    p.run_to_done(&mut b, 100_000);
    let steps = p.steps();
    check_walk(&steps, &store, 0o500, &(0o500..=0o516).collect::<Vec<_>>(), cmd, da);
    let (_, status) = b.cycle(REGS, None);
    assert_eq!(status & (1 | 1 << 13 | 1 << 11), 1, "done, no error: {status:o}");

    // 14: the seek, with `READY/` held down as a drive on cylinder holds it.
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let cylinder = 0o1234;
    let (cmd, da) = (0o14, cylinder << 16);
    b.cycle(REGS, Some(cmd));
    b.cycle(REGS + 2, Some(da));
    let ready = b.net("TRIDENT.READY/");
    b.chip.drive(ready, Level::Low);
    p.cycle(&mut b, REGS + 3, Some(0));
    p.run_to_done(&mut b, 40_000);
    let steps = p.steps();
    check_walk(&steps, &store, 0o400, &[0o400, 0o401, 0o402], cmd, da);
    let at = |a: u32| steps.iter().find(|s| s.executing(0o400) == Some(a)).unwrap();
    assert_eq!(
        at(0o401).cable,
        Cable { cyl_tag: true, bus: cylinder as u16, ..Default::default() },
        "the cylinder tag, as 4 sends it"
    );
    let (_, status) = b.cycle(REGS, None);
    assert_eq!(status & (1 | 1 << 13 | 1 << 11), 1, "done, no error: {status:o}");
}

/// **A seek waits for on-cylinder, then sends the cylinder tag.**
///
/// Sector 4: `400` puts the cylinder on the bus and loops `ON CYLINDER`,
/// `401` sends the cylinder tag, `402` holds the bus and stops. The
/// on-cylinder condition is the drive's `READY/` on J01 pin 42, through
/// TRITERM and a 74LS14 to `SEL UNIT ON CYL` and a 2 us synchroniser.
/// With the cable empty it is never on cylinder, so the micro-PC stands
/// at 1 with the first word in the UIR and the cylinder on the bus, and
/// the status word says not-on-cylinder. When `READY/` is pulled down,
/// as a drive would, the loop lets go within two clocks, the tag goes
/// out with the cylinder written into the disk address register on the
/// bus behind it, and the command completes.
#[test]
fn a_seek_waits_for_on_cylinder_then_sends_the_cylinder_tag() {
    let n = cadrdc();
    let store = control_store();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);

    let cylinder = 0o1234;
    let (cmd, da) = (0o4, cylinder << 16 | 3 << 8 | 5);
    b.cycle(REGS, Some(cmd));
    b.cycle(REGS + 2, Some(da));
    // `<30:28>`, the unit number, is `UNIT0-2` on DCDA 0B17, three nets
    // that go to edge pins and nowhere else on the board: the multiplexor
    // board's, which the one-board jumpers of `cadrdc/dc.eco` put on ground
    // (`Netlist::HAND_JUMPERS`) --- MIT's "in the 1-unit version, always
    // zero".
    let (_, back) = b.cycle(REGS + 2, None);
    assert_eq!(back, da, "the disk address register reads back, unit 0");
    let (_, status) = b.cycle(REGS, None);
    assert_ne!(status & 1 << 8, 0, "not on cylinder with nothing on the cable: {status:o}");

    let t0 = b.now;
    p.cycle(&mut b, REGS + 3, Some(0));
    p.run(&mut b, t0 + 40_000);
    assert!(p.busy(&b), "still busy");
    assert_eq!(p.upc(&b), 1, "waiting at the first word");
    assert_eq!(p.uir(&b), store[0o400], "LOOP/ON CYLINDER");
    assert_eq!(p.cable(&b), Cable { bus: cylinder as u16, ..Default::default() });
    let status = p.cycle(&mut b, REGS, None);
    assert_eq!(status & (1 | 1 << 8), 1 << 8, "active, not on cylinder: {status:o}");

    // The drive reaches the cylinder.
    let ready = b.net("TRIDENT.READY/");
    b.chip.drive(ready, Level::Low);
    let t1 = b.now;
    p.run_to_done(&mut b, 10_000);
    let steps = p.steps();
    check_walk(&steps, &store, 0o400, &[0o400, 0o401, 0o402], cmd, da);
    let at = |a: u32| steps.iter().find(|s| s.executing(0o400) == Some(a)).unwrap();
    assert!(at(0o400).until - at(0o400).from >= 30_000, "held through the wait");
    assert!(at(0o401).from - t1 <= 4_000, "let go within two clocks of READY/");
    assert_eq!(
        at(0o401).cable,
        Cable { cyl_tag: true, bus: cylinder as u16, ..Default::default() }
    );
    assert_eq!(at(0o402).cable, Cable { bus: cylinder as u16, ..Default::default() }, "held");

    let (_, status) = b.cycle(REGS, None);
    assert_eq!(status & (1 | 1 << 8 | 1 << 13), 1, "done, on cylinder, no error: {status:o}");
}

/// **An offset sends the head tag.**
///
/// Sector 6, the one command that sends a head tag on its own: `600`
/// puts the head on the bus and waits on cylinder, `601` sends the tag,
/// `602` holds and stops. Under the head tag the bus carries `HEAD0-5`
/// on 0 to 5 and the command register's two servo-offset bits on 6 and
/// 7, and bus 8 and 9 are left to their terminators --- the 74S157 at
/// DCDBUS 0C06 is disabled by `HEAD TAG`. The drive is on cylinder from
/// the start here, so the wait is one clock.
#[test]
fn an_offset_sends_the_head_tag() {
    let n = cadrdc();
    let store = control_store();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let ready = b.net("TRIDENT.READY/");
    b.chip.drive(ready, Level::Low);
    b.run(b.now + 5_000);

    // Offset forward, on head 3.
    let (cmd, da) = (0o6 | 1 << 5 | 1 << 4, 0o1234 << 16 | 3 << 8 | 5);
    b.cycle(REGS, Some(cmd));
    b.cycle(REGS + 2, Some(da));
    p.cycle(&mut b, REGS + 3, Some(0));
    p.run_to_done(&mut b, 40_000);
    let steps = p.steps();
    check_walk(&steps, &store, 0o600, &[0o600, 0o601, 0o602], cmd, da);
    let at = |a: u32| steps.iter().find(|s| s.executing(0o600) == Some(a)).unwrap();
    assert_eq!(at(0o601).cable, Cable { head_tag: true, bus: 0o303, ..Default::default() });
    assert_eq!(at(0o600).until - at(0o600).from, 2_000, "on cylinder already: no wait");

    let (_, status) = b.cycle(REGS, None);
    assert_eq!(status & (1 | 1 << 13), 1, "done, no error: {status:o}");
}

/// **A transfer with no drive stops by error before it starts.**
///
/// A read is command 0, and with `CMD2` down the disk's lossage lines
/// count: `-SEL UNIT ON LINE` and `NO SELECT` are both up with nothing
/// on the cable, DCBUSY 0B13 makes `-DISK LOSSAGE` of them, and
/// `-LOSSAGE` presets the `BUSY` flop off and the ECO 6 flop to STOPPED
/// BY ERROR. So START leaves the sequencer at zero and the status word
/// reads not-active with transfer-aborted, not-on-line and no-unit-
/// selected --- which is what the boot sees at the disk with
/// `--disk-controller netlist` and why the drive is the next thing.
#[test]
fn a_transfer_with_no_drive_stops_by_error() {
    let n = cadrdc();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);

    b.cycle(REGS, Some(0));
    b.cycle(REGS + 1, Some(0o777));
    b.cycle(REGS + 2, Some(0));
    p.cycle(&mut b, REGS + 3, Some(0));
    let until = b.now + 20_000;
    p.run(&mut b, until);
    let steps = p.steps();
    // `-START` clears the `BUSY` flop while `-LOSSAGE` presets it, and a
    // 74S74 with both down has both outputs up: `BUSY` is high for the
    // 200 ns strobe and falls with it, the clock never having come.
    let busy: Vec<u64> = steps.iter().filter(|s| s.busy).map(|s| s.until - s.from).collect();
    assert!(busy.iter().all(|&ns| ns <= 200), "busy only for the strobe: {busy:?}");
    assert!(steps.iter().all(|s| s.upc == 0), "never ran");
    assert!(!p.busy(&b));

    let (_, status) = b.cycle(REGS, None);
    eprintln!("status {status:o}");
    for (bit, name) in [(0, "not active"), (5, "no select"), (9, "not on line"), (13, "aborted")] {
        assert_ne!(status & 1 << bit, 0, "{name}: status {status:o}");
    }
}

/// **With no drive only the miscellaneous command completes.** Each command
/// code's low three bits pick a sector of the sequencer's PROM, and what
/// an empty cable does to each is `CMD2`'s to say: with it low the disk
/// lossage --- `NO SELECT` and `-SEL UNIT ON LINE` both up --- presets
/// `BUSY` off before the sequencer runs, and `STOPPED BY ERROR` with it,
/// so a read leaves the word as it found it, `0o21441`; with it high the
/// lossage is masked at DCBUSY 0C16 and the START clears the flop, so at
/// ease, recalibrate and fault clear (sector 5) run to done with no error,
/// `0o1441`, while a seek (4) and an offset clear (6) run to their first
/// step and wait there for a drive that never answers, and the reserved
/// sector 7 walks into unwritten PROM and stays at `31`. A reset between
/// each --- `16` then `0` into the command register --- puts the word
/// back to `0o21441`, the empty command register letting the lossage
/// through again. The behavioural controller is held to the same words in
/// `tests/disk.rs`.
///
/// The three waits end at the watchdog, 2.56 seconds on --- the hand
/// jumper `J5-16 : J5-41` of `cadrdc/disk.hand` grounds `-TIMEOUT ENB`
/// (`Netlist::HAND_JUMPERS`) and the 74LS124 section at DCTMOT 0B04 clocks
/// the 74393 at 0C03 --- which is four orders of magnitude past the 50 us
/// this reads at. `a_hung_command_times_out` is where they are waited out.
#[test]
fn with_no_drive_only_the_miscellaneous_command_completes() {
    let n = cadrdc();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    // (command, the status word 50 us after START, busy, micro-PC)
    let cases = [
        (0o0, 0o21441, false, 0),
        (0o4, 0o1440, true, 1),
        (0o5, 0o1441, false, 0),
        (0o1005, 0o1441, false, 0),
        (0o405, 0o1441, false, 0),
        (0o6, 0o1440, true, 1),
        (0o7, 0o1440, true, 0o31),
    ];
    for (cmd, word, busy, upc) in cases {
        b.cycle(REGS, Some(0o16));
        b.cycle(REGS, Some(0));
        let until = b.now + 5_000;
        p.run(&mut b, until);
        let (_, status) = b.cycle(REGS, None);
        assert_eq!(status, 0o21441, "after the reset before {cmd:o}");
        assert!(!p.busy(&b) && p.upc(&b) == 0, "stopped before {cmd:o}");
        b.cycle(REGS, Some(cmd));
        b.cycle(REGS + 1, Some(0o777));
        b.cycle(REGS + 2, Some(100 << 16));
        p.cycle(&mut b, REGS + 3, Some(0));
        let until = b.now + 50_000;
        p.run(&mut b, until);
        let (_, status) = b.cycle(REGS, None);
        assert_eq!(status, word, "{cmd:o} with no drive: status {status:o}");
        assert_eq!((p.busy(&b), p.upc(&b)), (busy, upc), "{cmd:o} with no drive");
    }
}

/// **The timeout enable jumper starts the watchdog clock.** `-TIMEOUT ENB`
/// is `J5-16` and pin 6, the enable, of the 74LS124 at DCTMOT 0B04 section
/// 1, and nothing else on the board. `cadrdc/disk.hand` grounds it by hand
/// --- "Timeout Enable jumper (use red wire): Add: J5-16 : J5-41 (the
/// adjacent ground pin)", and `dc.eco` says the same --- and
/// `Netlist::HAND_JUMPERS` does that, so the board as wrapped and the board
/// as built differ here: on the list the enable is an open 74LS input,
/// reads high, and by the part's own sheet holds the output high, which is
/// a watchdog that cannot tick.
///
/// With the jumper the output follows the section's internal oscillator,
/// which free-runs whatever the board is doing: `TIMEOUT.CLK` idles high
/// and toggles every half of `chip::DISK_TIMEOUT_VCO_PERIOD`. The 74393 at
/// 0C03 divides that by `disk_controller::TIMEOUT_DIVIDER` to reach
/// `TIMEOUT`, which `a_hung_command_times_out` waits out in full.
#[test]
fn the_timeout_enable_jumper_starts_the_watchdog_clock() {
    let n = cadrdc();
    let ground = n.by_name_id("GND").expect("a ground net");
    assert_eq!(n.by_name_id("'-TIMEOUT ENB'"), Some(ground), "J5-16 : J5-41");
    let wired = netlist::parse_wired(CADRDC).unwrap();
    assert_ne!(
        wired.by_name_id("'-TIMEOUT ENB'"),
        wired.by_name_id("GND"),
        "the wire list is the board before the red wire"
    );

    let mut b = controller(&n);
    let clk = b.net("TIMEOUT.CLK");
    assert_eq!(b.chip.net(clk), Level::High, "the output idles high");

    // A quarter of a period at a time is 45 ms of board, which is four
    // edges; the counter needs 128 of them, and the resolution here is the
    // step, a microsecond.
    let (step, start) = (1_000, b.now);
    let mut edges: Vec<(u64, Level)> = Vec::new();
    let mut last = Level::High;
    let mut t = start;
    while t < start + 45_000_000 {
        t += step;
        b.run(t);
        let level = b.chip.net(clk);
        if level != last {
            edges.push((t, level));
            last = level;
        }
    }

    let (num, den) = muir::chip::DISK_TIMEOUT_VCO_PERIOD;
    let half = num / den / 2;
    let levels: Vec<Level> = edges.iter().map(|&(_, l)| l).collect();
    assert_eq!(levels, [Level::Low, Level::High, Level::Low, Level::High], "{edges:?}");
    assert!(edges[0].0 - half < step, "the first fall half a period on: {edges:?}");
    for w in edges.windows(2) {
        assert_eq!(w[1].0 - w[0].0, half, "half a period apart: {edges:?}");
    }
}

/// **A hung command times out.** Command 7 walks into the sequencer PROM's
/// unwritten eighth sector and stops there, which `sys/doc/disk.text` says
/// of it: it "will currently hang the controller, causing a timeout error".
/// So the watchdog is what ends it. `-ACTIVE` holds both halves of the
/// 74393 at DCTMOT 0C03 clear while the board is idle, and the count runs
/// from the first fall of `TIMEOUT.CLK` after the START. That clock is
/// free-running, so the first fall is anywhere in a period and the timeout
/// lands between 127 and 128 periods after the command, 2.54 to 2.56
/// seconds: still busy at 2.5, over by 2.6.
///
/// `TIMEOUT` is inverted at DCSTS 0A16 and latched as `TIMEOUT ERROR`,
/// which is `STATUS<11>` through the 74LS273 at DCSTS 0C12 and an input of
/// the 74S260 at 0B13, so it stops the sequencer as the other errors do:
/// `STATUS<13>`, `STOPPED BY ERROR`, comes up with it and `BUSY` drops.
/// The behavioural controller reaches the same word from
/// `disk_controller::TIMEOUT_NS`, in `tests/disk.rs`.
#[test]
#[ignore = "2.56 seconds of board, twenty seconds of wall clock; run with --ignored"]
fn a_hung_command_times_out() {
    let n = cadrdc();
    let mut b = controller(&n);
    let busy = b.net("BUSY");
    let busy = move |b: &XbusMaster| b.chip.net(busy) == Level::High;

    b.cycle(REGS, Some(0o7));
    b.cycle(REGS + 1, Some(0o777));
    b.cycle(REGS + 2, Some(100 << 16));
    b.cycle(REGS + 3, Some(0));
    let started = b.now;
    assert!(busy(&b), "the command is running");

    b.run(started + 2_500_000_000);
    let (_, status) = b.cycle(REGS, None);
    assert!(busy(&b), "still busy at 2.5 s: status {status:o}");
    assert_eq!(status & (1 << 11 | 1 << 13), 0, "no error yet: status {status:o}");

    b.run(started + 2_600_000_000);
    let (_, status) = b.cycle(REGS, None);
    assert!(!busy(&b), "stopped by 2.6 s: status {status:o}");
    assert_ne!(status & 1 << 11, 0, "timeout error: status {status:o}");
    assert_ne!(status & 1 << 13, 0, "stopped by error: status {status:o}");
}

/// A drive on the cable that seeks fast: the settling and the stroke
/// scaled down so a test that waits on a seek waits microseconds.
fn quick_drive(now: u64) -> Trident {
    let mut d = Trident::new(Unit::blank(Geometry::T300), now);
    d.seek_settle_ns = 100_000;
    d.seek_ns_per_cylinder = 1_000;
    d
}

/// **The block counter follows the drive's sector pulses.** `STATUS<31:24>`
/// is the block counter, two 74LS569s on DCTRID clocked by the composite
/// pulse's trailing edge and cleared, synchronously, on the trailing edge
/// of a pulse the 2.25 us one-shot has timed out on --- the index. So with
/// a drive turning, the status word says which sector is under the head,
/// which is what `LOOP/BLOCK CTR EQ BLOCK` waits on.
///
/// The drive pulses eighteen times a turn, the index and then seventeen
/// sector pulses a sector apart, so the count runs 0 to 17 and the 17 is
/// the track's leftover, which holds no block. Read during and after every
/// pulse for a turn and a quarter: while a pulse is on the count is still
/// the region before it, and the index pulse, the long one, holds that 17
/// past a sector pulse's width and clears as it ends. The behavioural
/// controller is held to the same readings in `tests/disk.rs`.
#[test]
fn the_block_counter_follows_the_drives_sector_pulses() {
    let n = cadrdc();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    p.plug(&mut b, &n, quick_drive(t0));
    assert_eq!(p.drive().turn(t0), (0, 0), "an index pulse just beginning");
    for k in 0..23u32 {
        let began = t0 + (k as u64 / 18) * REVOLUTION_NS + (k as u64 % 18) * SECTOR_NS;
        // 300 ns in, the pulse still on: the count before it. Not at the
        // first pulse, which the counter meets holding its power-up zero.
        if k > 0 {
            p.run(&mut b, began + 300);
            let status = p.cycle(&mut b, REGS, None);
            assert_eq!(
                status >> 24,
                (k + 17) % 18,
                "during the pulse at {}: status {status:o}",
                b.now
            );
        }
        if k == 18 {
            // 2.5 us into the index pulse: a sector pulse would be over,
            // this one is not, and the clear has not landed.
            p.run(&mut b, began + 2_500);
            let status = p.cycle(&mut b, REGS, None);
            assert_eq!(status >> 24, 17, "during the index pulse at {}: status {status:o}", b.now);
            p.run(&mut b, began + 6_000);
            let status = p.cycle(&mut b, REGS, None);
            assert_eq!(status >> 24, 0, "after the index pulse at {}: status {status:o}", b.now);
        }
        // 100 us in: the pulse over, the counter settled.
        p.run(&mut b, began + 100_000);
        let status = p.cycle(&mut b, REGS, None);
        assert_eq!(p.drive().turn(b.now).0, k % 18, "the drive at {}", b.now);
        assert_eq!(status >> 24, k % 18, "the block counter at {}: status {status:o}", b.now);
    }
    // And the drive is on line, on cylinder, selected: none of bits 9, 8, 5.
    let status = p.cycle(&mut b, REGS, None);
    assert_eq!(status & (1 << 9 | 1 << 8 | 1 << 5), 0, "status {status:o}");
}

/// **A seek with a drive on the cable moves the heads, and its attention
/// reaches the status word and the interrupt.** Command 4 to cylinder 100:
/// the sequencer sends the cylinder tag and stops, the drive takes its
/// time, the status word says not-on-cylinder until it is done, and then
/// the drive's attention is up on `UNIT 0 ATTENTION` --- which the
/// one-board jumpers of `cadrdc/dc.eco` tie to `ANY ATTENTION` and `SEL
/// UNIT ATTENTION` (`Netlist::HAND_JUMPERS`), so `STATUS<2:1>` are both
/// up, as `sys/doc/disk.text` has them, "the attention signal directly
/// from the drive, ... not separately latched in the controller"; and
/// with the command register's attention interrupt enable set, the 9S42 at
/// DCCHAN 0F17 raises `INTR`, `STATUS<3>`, and the 26S10 at 0F18 pulls
/// `-XBUS.INTR`. The at-ease command's read gate then clears the drive's
/// attention, and the bits and the interrupt go with it, the enable still
/// set.
#[test]
fn a_seek_with_a_drive_moves_the_heads() {
    let n = cadrdc();
    let store = control_store();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    p.plug(&mut b, &n, quick_drive(t0));
    p.run(&mut b, t0 + 10_000);

    let (cmd, da) = (0o4, 100 << 16);
    b.cycle(REGS, Some(cmd));
    b.cycle(REGS + 2, Some(da));
    p.cycle(&mut b, REGS + 3, Some(0));
    p.run_to_done(&mut b, 40_000);
    let steps = p.steps();
    check_walk(&steps, &store, 0o400, &[0o400, 0o401, 0o402], cmd, da);
    let sent = p.drive().tags.clone();
    assert_eq!(sent.len(), 1, "one cylinder tag: {sent:?}");
    assert!(matches!(sent[0].1, muir::disk_unit::Tag::Cylinder(100)), "{sent:?}");

    // Seeking: 100 us and 100 cylinders at 1 us.
    let status = p.cycle(&mut b, REGS, None);
    assert_eq!(status & (1 | 1 << 8), 1 | 1 << 8, "done, not on cylinder: {status:o}");
    assert_eq!(b.level("'UNIT 0 ATTENTION'"), Level::Low, "no attention yet");
    p.run(&mut b, sent[0].0 + 250_000);
    let status = p.cycle(&mut b, REGS, None);
    assert_eq!(status & (1 | 1 << 8), 1, "on cylinder again: {status:o}");
    assert_eq!(p.drive().position(), (100, 0));
    assert_eq!(b.level("'UNIT 0 ATTENTION'"), Level::High, "attention from the drive");
    assert_eq!(
        status & (1 << 2 | 1 << 1),
        1 << 2 | 1 << 1,
        "STATUS<2:1> off the same wire: {status:o}"
    );
    assert_eq!(status & 1 << 3, 0, "no interrupt without the enable: {status:o}");
    assert_eq!(b.level("-XBUS.INTR"), Level::High);

    // The attention interrupt enable, `<10>` of the command register, with
    // the controller idle: the interrupt comes up.
    b.cycle(REGS, Some(0o4 | 1 << 10));
    let settled = b.now + 10_000;
    p.run(&mut b, settled);
    let status = p.cycle(&mut b, REGS, None);
    assert_eq!(status & 1 << 3, 1 << 3, "interrupt request: {status:o}");
    assert_eq!(b.level("-XBUS.INTR"), Level::Low, "and asserted on the backplane");

    // At ease: command 5, whose read gate at 513 resets the attention. The
    // enable stays set, so it is the attention going that ends the
    // interrupt.
    b.cycle(REGS, Some(0o5 | 1 << 10));
    p.cycle(&mut b, REGS + 3, Some(0));
    p.run_to_done(&mut b, 100_000);
    assert_eq!(b.level("'UNIT 0 ATTENTION'"), Level::Low, "attention reset by read gate");
    let status = p.cycle(&mut b, REGS, None);
    assert_eq!(status & (1 << 3 | 1 << 2 | 1 << 1), 0, "bits and interrupt gone: {status:o}");
    assert_eq!(b.level("-XBUS.INTR"), Level::High);
    let seen: Vec<_> = p.drive().tags.iter().map(|(_, t)| *t).collect();
    assert!(seen.len() >= 2, "the control tags of the at-ease command: {seen:?}");
}

/// Some words that are not all alike, from a seed.
fn words(seed: u32) -> [u32; muir::disk_unit::BLOCK_WORDS] {
    let mut x = seed | 1;
    let mut out = [0u32; muir::disk_unit::BLOCK_WORDS];
    for w in out.iter_mut() {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *w = x;
    }
    out
}

/// The status bits System 100 retries a transfer on: `%DISK-STATUS-HIGH-
/// ERROR` and `%DISK-STATUS-LOW-ERROR` from `sys/cold/qcom.lisp`, the
/// high half above bit 16.
const ERRORS: u32 = 0o237 << 16 | LOW_ERRORS;
const LOW_ERRORS: u32 = 0o177560;

/// Where a test keeps its command list, and the pages it transfers.
const CLP: u32 = 0o4000;
const PAGE: u32 = 0o20000;
const PAGE2: u32 = 0o30000;

/// One read command from START to not-active, with the drive and the
/// memory plugged in: the page written, the words in order, the walk,
/// and the status word when it is over.
fn read_block(
    p: &mut Probe,
    b: &mut XbusMaster,
    store: &[u32],
    block: u32,
    page: u32,
) -> (u32, Vec<(u32, u32)>) {
    p.memory.as_mut().unwrap().words[CLP as usize] = page;
    p.memory.as_mut().unwrap().transfers.clear();
    let (cmd, da) = (0o0, block);
    b.cycle(REGS, Some(cmd));
    b.cycle(REGS + 1, Some(CLP));
    b.cycle(REGS + 2, Some(da));
    p.cycle(b, REGS + 3, Some(0));
    p.run_to_done(b, 3 * REVOLUTION_NS / 17);
    // The memory side may still be emptying the FIFO.
    let settled = b.now + 20_000;
    p.run(b, settled);
    let steps = p.steps();
    let (_, status) = b.cycle(REGS, None);
    let transfers = p.memory().transfers.clone();
    assert_eq!(transfers[0].wrote, None, "the CCW fetch first");
    assert_eq!(transfers[0].addr, CLP);
    let written: Vec<(u32, u32)> =
        transfers.iter().filter_map(|t| t.wrote.map(|w| (t.addr, w))).collect();
    assert_eq!(written.len(), 256, "one page of words");
    assert!(written.iter().enumerate().all(|(k, &(a, _))| a == page + k as u32), "in order");
    let program: Vec<u32> = (0..=0o47).collect();
    let executed: Vec<u32> = steps.iter().filter_map(|s| s.executing(0)).collect();
    assert_eq!(executed, program, "the read program, start to done");
    check_walk(&steps, store, 0, &program, cmd, da);
    let (_, back) = b.cycle(REGS + 2, None);
    assert_eq!(back & 0x0fff_ffff, da, "the disk address of the last block transferred");
    (status, written)
}

/// **A read with a drive on the cable puts the block in memory.**
///
/// Everything the seam has to carry, in one command: the channel fetches
/// its CCW from memory as bus master; the sequencer seeks, selects the
/// head, waits for the block's sector pulse, spends its 20 us of pre gate,
/// finds the sync in the preamble, compares the four header bytes against
/// the disk address register, checks the header's checkword against its
/// own ECC register, finds the data sync, takes the pad byte, moves 1,024
/// bytes through the FIFO into 256 words the channel writes to the page,
/// checks the data checkword, and stops with no error.
///
/// "No error" is what System 100 means by it: `%DISK-STATUS-HIGH-ERROR`
/// and `%DISK-STATUS-LOW-ERROR` in `sys/cold/qcom.lisp`, the masks its
/// disk code retries on. Bit 22, read-compare difference, is outside them
/// and reads set after a plain read --- "This bit is undefined unless the
/// command is read-compare" --- and bit 23, internal parity, is inside
/// them and reads clear, for a block ending in a zero bit and for one
/// ending in a one.
#[test]
fn a_read_with_a_drive_puts_the_block_in_memory() {
    let n = cadrdc();
    let store = control_store();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    let mut drive = quick_drive(t0);
    let mut ending_in_zero = words(21);
    ending_in_zero[255] &= !(1 << 31);
    let mut ending_in_one = words(22);
    ending_in_one[255] |= 1 << 31;
    assert!(drive.unit.write_block_at(0, 0, 2, &ending_in_zero));
    assert!(drive.unit.write_block_at(0, 0, 5, &ending_in_one));
    p.plug(&mut b, &n, drive);
    p.with_memory(&b, 1 << 15);
    p.run(&mut b, t0 + 10_000);

    for (block, page, data) in [(2, PAGE, ending_in_zero), (5, PAGE2, ending_in_one)] {
        let (status, _) = read_block(&mut p, &mut b, &store, block, page);
        eprintln!("block {block}: status {status:o}");
        assert_eq!(&p.memory().words[page as usize..page as usize + 256], &data[..]);
        assert_eq!(status & 1, 1, "not active: {status:o}");
        assert_eq!(status & ERRORS, 0, "no error: {status:o}");
        assert_eq!(status >> 24, block, "the block counter, the sector just read");
    }
}

/// **A spurious sector pulse raises `STATUS<12>`, start block error.**
///
/// MIT's own description is of a drive at fault rather than a mark gone
/// missing: `<12>` "indicates that a start-of-block (sector pulse)
/// happened at a time when it should not have. Either the disk is
/// incorrectly formatted or it is generating spurious sector pulses." The
/// board carries the detection --- the LS74 two-stage synchroniser at
/// DCHDCM 0D15 clocked off the drive's composite pulse, `BAD START BLOCK`
/// at the LS08 0D16, and `ERR IF START BLOCK` asserted through nearly all
/// of the read in `cadrdc/newdsk.31` --- and until
/// [`Trident::spurious_pulse`] nothing could give it one to catch: the
/// spindle's pulses come from a fixed revolution and are right by
/// construction. Issue 81.
///
/// **The same read three times, the drive the only difference.** Clean,
/// `<12>` is clear. With a pulse halfway through the sector being read,
/// `<12>` comes up and `<13>` with it --- "Transfer Aborted", the error
/// stopping the transfer, which is what an error is meant to do. And with
/// the same pulse in a sector the read never reaches, the status comes
/// back **bit for bit the clean one**: the board is detecting a pulse
/// during its read rather than being upset by an injected fault, which is
/// the difference between this test and one that would pass on any
/// disturbance at all.
///
/// `<23>`, internal parity, also comes up on the faulty read. That is not
/// asserted here and has not been chased: a read torn open mid-sector has
/// no reason to leave the parity chain happy, but nobody has shown that is
/// what happens.
///
/// One offset is left out deliberately, and where it goes instead says
/// why: a pulse in the first microseconds of the block's own sector falls
/// in steps `010` and `011`, which carry no `ERR IF START BLOCK`, so it is
/// taken as a block boundary rather than reported.
/// [`the_start_block_check_begins_where_the_microcode_starts_it`] has the
/// boundary and MIT's listing for it.
#[test]
fn a_spurious_sector_pulse_raises_start_block_error() {
    const START_BLOCK: u32 = 1 << 12;
    const ABORTED: u32 = 1 << 13;
    let n = cadrdc();
    let data = words(23);

    let read = |fault: Option<u64>| -> u32 {
        let mut b = controller(&n);
        let mut p = Probe::new(&b);
        let t0 = b.now;
        let mut drive = quick_drive(t0);
        assert!(drive.unit.write_block_at(0, 0, 2, &data));
        drive.spurious_pulse = fault;
        p.plug(&mut b, &n, drive);
        p.with_memory(&b, 1 << 15);
        p.run(&mut b, t0 + 10_000);
        p.memory.as_mut().unwrap().words[CLP as usize] = PAGE;
        b.cycle(REGS, Some(0o0));
        b.cycle(REGS + 1, Some(CLP));
        b.cycle(REGS + 2, Some(2));
        p.cycle(&mut b, REGS + 3, Some(0));
        p.run_to_done(&mut b, 3 * REVOLUTION_NS / 17);
        let settled = b.now + 20_000;
        p.run(&mut b, settled);
        b.cycle(REGS, None).1
    };

    let clean = read(None);
    assert_eq!(clean & ERRORS, 0, "no fault, no error: {clean:o}");
    assert_eq!(clean & START_BLOCK, 0, "and <12> clear: {clean:o}");

    let torn = read(Some(2 * SECTOR_NS + SECTOR_NS / 2));
    assert_ne!(torn & START_BLOCK, 0, "a pulse mid-read raises <12>: {torn:o}");
    assert_ne!(torn & ABORTED, 0, "and <13>, the transfer aborted: {torn:o}");

    let elsewhere = read(Some(8 * SECTOR_NS + SECTOR_NS / 2));
    assert_eq!(
        elsewhere, clean,
        "a pulse the read never reaches changes nothing: {elsewhere:o} against {clean:o}"
    );
}

/// **The error check starts at step 012, and MIT's listing says why.**
///
/// `cadrdc/newdsk.31` asserts `ERR IF START BLOCK` on every step of the
/// read from `012` on. It does not on `010` or `011`:
///
/// ```text
/// 010:  CLK/START BLOCK,HOLD PRE,        ;Initialize DBUS,
///           LOOP/BLOCK CTR EQ BLOCK      ; find start of right block
/// 011:  PRE GATE,CLK/2 USEC              ;Start head select, delay 20 usec
/// 012:  PRE GATE,CLK/2 USEC,ERR IF START BLOCK
/// ```
///
/// `010` is **clocked by** the start block and loops until the block
/// counter matches, so a sector pulse there is the thing it is waiting
/// for and cannot be an error. `011` is the one 2 us step after it.
///
/// So a spurious pulse ([`Trident::spurious_pulse`]) in the first
/// microseconds of the block's own sector is taken as a block boundary
/// rather than reported, and one after that window raises `<12>`.
/// Measured across the boundary, at 500 ns steps into block 2's sector:
/// through 2,500 ns the outcomes vary with where the pulse falls against
/// the counter's clocking --- a clean read at 500, 2,000 and 2,500, a
/// header compare error `<18>` at 1,000, a clean read a revolution late at
/// 1,500 --- and **none of them raises `<12>`**. From 3,000 ns on, every
/// one does. That is a race with no error detection, which is what a step
/// carrying no `ERR IF START BLOCK` is.
///
/// **Nothing hangs.** Issue 81's comment said a pulse here hung the
/// controller to the watchdog; it does not. The longest of these takes
/// 1.17 revolutions --- the 1,500 ns case, which loses a revolution and
/// reads correctly on the next --- against the `3 * REVOLUTION_NS / 17`
/// that [`read_block`] allows, and it was that allowance running out
/// rather than the drive's. Issue 83.
#[test]
fn the_start_block_check_begins_where_the_microcode_starts_it() {
    const START_BLOCK: u32 = 1 << 12;
    let n = cadrdc();
    let data = words(23);

    let read = |at: u64| -> u32 {
        let mut b = controller(&n);
        let mut p = Probe::new(&b);
        let t0 = b.now;
        let mut drive = quick_drive(t0);
        assert!(drive.unit.write_block_at(0, 0, 2, &data));
        drive.spurious_pulse = Some(2 * SECTOR_NS + at);
        p.plug(&mut b, &n, drive);
        p.with_memory(&b, 1 << 15);
        p.run(&mut b, t0 + 10_000);
        p.memory.as_mut().unwrap().words[CLP as usize] = PAGE;
        b.cycle(REGS, Some(0o0));
        b.cycle(REGS + 1, Some(CLP));
        b.cycle(REGS + 2, Some(2));
        p.cycle(&mut b, REGS + 3, Some(0));
        // Two revolutions is room for the slowest of these, which loses
        // one; a command still busy after it would be a hang, and the
        // assertion below says none is.
        p.run_to_done(&mut b, 2 * REVOLUTION_NS);
        let settled = b.now + 20_000;
        p.run(&mut b, settled);
        b.cycle(REGS, None).1
    };

    for at in [500, 1_000, 1_500, 2_000, 2_500] {
        let v = read(at);
        assert_eq!(v & START_BLOCK, 0, "at +{at} the pulse is inside 010/011: {v:o}");
    }
    for at in [3_000, 4_000, 6_000, 8_000] {
        let v = read(at);
        assert_ne!(v & START_BLOCK, 0, "at +{at} the check is running: {v:o}");
    }
}

/// **`STATUS<23>` is a parity comparison, not an abort flag**, so a read
/// torn part way through raises it or does not according to what had gone
/// by --- which is the board's own arithmetic and not a fault of the
/// model.
///
/// `INTERNAL PARITY ERROR` is the LS86 at DCSTS 0A16 taking `MEM SIDE
/// PAR` against `DISK SIDE PAR`, and it drives `XBO23`. The two are
/// running accumulators in 74LS273s --- `PAR IN = PAR XOR <incoming>`
/// latched back into itself --- cleared together by `-RESET ERR` and
/// clocked apart. The disk's takes `DISK DATA` gated by `DATA FIELD` at
/// the LS08 0E05, data bits off the cable and not the header or preamble,
/// latched at 0C12 on `BIT.CLK^`; the memory's takes `XB ODD PAR` gated by
/// `-NEW CCW` at the LS08 0D14, each data word's parity over the Xbus and
/// not the CCW fetch, latched at 0D24 on `CHAN.ACK.T1`. The parity of
/// every word's parity is the parity of every bit in those words, so the
/// two compute one quantity by two routes and agree when the same data has
/// been through both sides. A transfer stopped in the middle leaves them
/// wherever it stopped.
///
/// **That is what distinguishes the two readings, and it is measurable.**
/// A bit the abort sets would come up on every torn read, with `<12>`. A
/// parity comparison comes up on the tears where the accumulated parities
/// differ and not on the others. Measured over tears every 40 us across
/// the sector: `<12>` on all of them, `<23>` on some. So it is the second,
/// and it is fidelity --- MIT's board computes this same XOR from these
/// same two latches and would report the same. Issue 85.
///
/// The clean read is the other half: both sides see the whole block, the
/// accumulators agree, and `<23>` is clear. That is asserted by
/// [`a_spurious_sector_pulse_raises_start_block_error`], whose clean
/// status is `220000001`.
#[test]
fn internal_parity_is_a_comparison_and_not_an_abort_flag() {
    const START_BLOCK: u32 = 1 << 12;
    const PARITY: u32 = 1 << 23;
    let n = cadrdc();
    let data = words(23);

    let torn_at = |at: u64| -> u32 {
        let mut b = controller(&n);
        let mut p = Probe::new(&b);
        let t0 = b.now;
        let mut drive = quick_drive(t0);
        assert!(drive.unit.write_block_at(0, 0, 2, &data));
        drive.spurious_pulse = Some(2 * SECTOR_NS + at);
        p.plug(&mut b, &n, drive);
        p.with_memory(&b, 1 << 15);
        p.run(&mut b, t0 + 10_000);
        p.memory.as_mut().unwrap().words[CLP as usize] = PAGE;
        b.cycle(REGS, Some(0o0));
        b.cycle(REGS + 1, Some(CLP));
        b.cycle(REGS + 2, Some(2));
        p.cycle(&mut b, REGS + 3, Some(0));
        p.run_to_done(&mut b, 2 * REVOLUTION_NS);
        let settled = b.now + 20_000;
        p.run(&mut b, settled);
        b.cycle(REGS, None).1
    };

    // Well inside the data, so every one of these is a torn read rather
    // than a pulse absorbed at the block boundary (issue 83).
    let tears: Vec<(u64, u32)> =
        [90_000u64, 130_000, 210_000, 250_000, 330_000, 530_000, 610_000, 650_000]
            .iter()
            .map(|&at| (at, torn_at(at)))
            .collect();
    for &(at, v) in &tears {
        assert_ne!(v & START_BLOCK, 0, "the read at +{at} was torn: {v:o}");
    }
    let with = tears.iter().filter(|(_, v)| v & PARITY != 0).count();
    assert!(
        with > 0 && with < tears.len(),
        "<23> follows the parities and not the abort: {with} of {} tears raise it, and a \
         flag the abort set would be all of them --- {:?}",
        tears.len(),
        tears.iter().map(|&(at, v)| (at, v >> 23 & 1)).collect::<Vec<_>>()
    );
}

/// **`STATUS<8>`, "not on cylinder", is `-ON CYL SYNC` through one buffer,
/// and nothing else can reach the bus pin while the status register is
/// being read.**
///
/// The path, read off `data/CADRDC.netlist` and measured here from the
/// drive's end to the backplane's:
///
/// - `TRIDENT.READY/` off the signal cable reaches the terminator at
///   DCTRSG 0A03 pin 1 and leaves across the package on pin 16;
/// - the Schmitt inverter at DCTRSG 0A04 takes it on pin 1 and puts
///   `SEL UNIT ON CYL` on pin 2;
/// - that is the D input, pin 12, of the 74LS74 at DCCLK 0F16, clocked on
///   `2USEC.CLK^` at pin 11 with both asynchronous inputs tied high; its
///   `Q` on pin 9 is `ON CYL SYNC` and its `Q/` on pin 8 is
///   `-ON CYL SYNC`;
/// - `-ON CYL SYNC` is pin 2 of the 74LS244 at DCSTS 0A13, whose pin 18
///   is `XBO8`, both halves enabled by `-READ STS` on pins 1 and 19;
/// - and the 26S10 at DCXBUS 0F27 takes `XBO8` on pin 4 and pulls
///   `-XBUS8`, pin 2, low for a one, enabled by `-DRIVE XBUS` on pin 12.
///
/// **`XBO8` is a shared internal bus with five drivers**, so "and nothing
/// else" is the other half of the claim. Four are register read-backs
/// selected one at a time by the 74S138 at DCREG 0E12 --- `-READ STS`,
/// `-READ MA` (the 74LS374 at DCCLP 0C25), `-READ DA` (the 74LS244 at
/// DCDA 0B19, `HEAD0`) and `-READ ECC` (the 74LS244 at DCPOSC 0B25,
/// `POSC8`) --- and the fifth is outside the decoder: the 74LS374 at
/// DCRBUF 0F09 puts `RBUF0` on `XBO8` whenever `-CHAN.MASTER` is low,
/// which is how a word read off the disk reaches memory. This asserts
/// that the four that are not the status buffer stay disabled for the
/// whole of a status read.
///
/// Both states of the bit, taken at every step the harness makes inside
/// the cycle rather than between two of them. Issue 88.
#[test]
fn status_8_is_the_on_cylinder_synchroniser() {
    const OFF_CYLINDER: u32 = 1 << 8;
    let n = cadrdc();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    p.plug(&mut b, &n, quick_drive(t0));
    p.run(&mut b, t0 + 10_000);

    const NAMES: [&str; 8] = [
        "-READ STS",
        "-READ MA",
        "-READ DA",
        "-READ ECC",
        "-CHAN.MASTER",
        "-ON CYL SYNC",
        "XBO8",
        "-XBUS8",
    ];
    let watch: Vec<NetId> = NAMES.iter().map(|s| b.net(s)).collect();
    let (sts, ma, da, ecc, master, sync, xbo8, xbus8) = (0, 1, 2, 3, 4, 5, 6, 7);

    let check = |what: &str, status: u32, seen: &[Vec<Level>]| {
        let read: Vec<&Vec<Level>> = seen.iter().filter(|s| s[sts] == Level::Low).collect();
        assert!(!read.is_empty(), "{what}: the status buffer was never enabled");
        for s in &read {
            for (k, name) in [(ma, "-READ MA"), (da, "-READ DA"), (ecc, "-READ ECC")] {
                assert_eq!(s[k], Level::High, "{what}: {name} with -READ STS, on {s:?}");
            }
            assert_eq!(s[master], Level::High, "{what}: -CHAN.MASTER low, on {s:?}");
            assert_eq!(s[xbo8], s[sync], "{what}: XBO8 is not -ON CYL SYNC, on {s:?}");
            assert_eq!(
                s[xbus8],
                match s[xbo8] {
                    Level::High => Level::Low,
                    _ => Level::High,
                },
                "{what}: -XBUS8 is not XBO8 inverted, on {s:?}"
            );
        }
        eprintln!("{what}: status {status:o}, {} samples inside the read", read.len());
    };

    // On cylinder: `READY/` low, the synchroniser holding it, and the bit
    // down all the way to the backplane.
    let (status, seen) = p.cycle_watching(&mut b, REGS, None, &watch);
    assert!(seen.iter().all(|s| s[sync] == Level::Low), "-ON CYL SYNC while on cylinder");
    check("on cylinder", status, &seen);
    assert_eq!(status & OFF_CYLINDER, 0, "status {status:o}");

    // Off cylinder: a seek to 100, read while the heads are moving.
    b.cycle(REGS, Some(0o4));
    b.cycle(REGS + 2, Some(100 << 16));
    p.cycle(&mut b, REGS + 3, Some(0));
    p.run_to_done(&mut b, 40_000);
    let (status, seen) = p.cycle_watching(&mut b, REGS, None, &watch);
    assert!(seen.iter().all(|s| s[sync] == Level::High), "-ON CYL SYNC while seeking");
    check("seeking", status, &seen);
    assert_ne!(status & OFF_CYLINDER, 0, "status {status:o}");

    // And back down when the heads arrive, so that the bit is the
    // drive's line and not a latch that stays where a command put it.
    let arrived = b.now + 400_000;
    p.run(&mut b, arrived);
    let (status, seen) = p.cycle_watching(&mut b, REGS, None, &watch);
    assert!(seen.iter().all(|s| s[sync] == Level::Low), "-ON CYL SYNC once the heads arrive");
    check("arrived", status, &seen);
    assert_eq!(status & OFF_CYLINDER, 0, "status {status:o}");
}

/// **A write with a drive on the cable puts the page on the pack.**
///
/// The other direction, sector 1 of the control store: the channel
/// fetches the CCW and then reads the page from memory a word at a time
/// into the write FIFO; the sequencer seeks, waits for the block, reads
/// and checks the header as a read does, then switches to writing at
/// `134` --- nineteen bytes of ones, the sync, the pad, 1,024 bytes out
/// of the shift register with the ECC register following, the checkword,
/// four guard bytes --- and stops at `174` when the memory side is done.
/// The drive records what the 75110 puts on the data pair at every clock,
/// and when write gate drops it parses the sector it was given and
/// writes the block it holds to the pack: the same 256 words the page
/// held, under a header the drive already had.
#[test]
fn a_write_with_a_drive_puts_the_page_on_the_pack() {
    let n = cadrdc();
    let store = control_store();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    p.plug(&mut b, &n, quick_drive(t0));
    p.with_memory(&b, 1 << 15);
    let data = words(31);
    let m = p.memory.as_mut().unwrap();
    m.words[PAGE as usize..PAGE as usize + 256].copy_from_slice(&data);
    m.words[CLP as usize] = PAGE;
    p.run(&mut b, t0 + 10_000);

    let (cmd, da) = (0o11, 2);
    b.cycle(REGS, Some(cmd));
    b.cycle(REGS + 1, Some(CLP));
    b.cycle(REGS + 2, Some(da));
    p.cycle(&mut b, REGS + 3, Some(0));
    p.run_to_done(&mut b, 3 * REVOLUTION_NS / 17);
    let settled = b.now + 20_000;
    p.run(&mut b, settled);
    let steps = p.steps();
    let (_, status) = b.cycle(REGS, None);
    eprintln!("status {status:o}");

    let transfers = p.memory().transfers.clone();
    assert_eq!((transfers[0].addr, transfers[0].wrote), (CLP, None), "the CCW fetch first");
    let read: Vec<u32> = transfers[1..]
        .iter()
        .map(|t| {
            assert_eq!(t.wrote, None, "the channel only reads");
            t.addr
        })
        .collect();
    assert_eq!(read, (PAGE..PAGE + 256).collect::<Vec<_>>(), "the page, word by word");
    assert_eq!(status & 1, 1, "not active: {status:o}");
    assert_eq!(status & ERRORS, 0, "no error: {status:o}");
    let program: Vec<u32> = (0o100..=0o174).collect();
    let executed: Vec<u32> = steps.iter().filter_map(|s| s.executing(0o100)).collect();
    assert_eq!(executed, program, "the write program, start to done");
    check_walk(&steps, &store, 0o100, &program, cmd, da);

    let drive = &mut p.cable.as_mut().unwrap().drive;
    assert_eq!(drive.bad_writes, 0, "what was written parsed as the format");
    assert_eq!(drive.unit.block_at(0, 0, 2), Some(data), "the page, on the pack");
    assert_eq!(drive.unit.block_at(0, 0, 1), Some([0u32; 256]), "and no other block");
}

/// **The System 100 pack reads through the board.** The release's own image,
/// `vendor/run/disk-sys-100-0.img`, in the same `Unit` the
/// model controller reads it through, on the netlist's cable: block 0 is
/// the label, and its first word is `LABL` as the boot PROM checks it.
#[test]
fn the_system_100_pack_reads_through_the_board() {
    let Some(image) = support::pack_100() else { return };
    let n = cadrdc();
    let store = control_store();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    let mut drive = Trident::new(Unit::open(&image, Geometry::T300).unwrap(), t0);
    drive.seek_settle_ns = 100_000;
    drive.seek_ns_per_cylinder = 1_000;
    // The label's sector comes round 400 us from now: the seek settling
    // and the head selection take about 120.
    drive.spin_to(t0, 0, 400_000);
    let want = drive.unit.block_at(0, 0, 0).unwrap();
    assert_eq!(want[0], 0o11420440514, "the pack's label, off the image");
    p.plug(&mut b, &n, drive);
    p.with_memory(&b, 1 << 15);
    p.run(&mut b, t0 + 10_000);

    let (status, written) = read_block(&mut p, &mut b, &store, 0, PAGE);
    eprintln!("block 0: status {status:o}");
    assert_eq!(status & 1, 1, "not active: {status:o}");
    assert_eq!(status & ERRORS, 0, "no error: {status:o}");
    assert_eq!(written[0].1, 0o11420440514, "LABL, through the board");
    assert_eq!(&p.memory().words[PAGE as usize..PAGE as usize + 256], &want[..], "the label block");
}

/// One read command of `blocks` at a CCW list of as many pages, with the
/// drive and memory plugged in; the pages written and the status word.
fn read_blocks(
    p: &mut Probe,
    b: &mut XbusMaster,
    first: u32,
    pages: &[u32],
) -> (u32, Vec<(u32, u32)>) {
    let m = p.memory.as_mut().unwrap();
    for (k, &page) in pages.iter().enumerate() {
        // "<0> More flag. If this bit is 0, this is the last CCW in the
        // list."
        m.words[CLP as usize + k] = page | (k + 1 < pages.len()) as u32;
    }
    m.transfers.clear();
    b.cycle(REGS, Some(0));
    b.cycle(REGS + 1, Some(CLP));
    b.cycle(REGS + 2, Some(first));
    p.cycle(b, REGS + 3, Some(0));
    // A block at the end of the track is most of a revolution away.
    p.run_to_done(b, REVOLUTION_NS + 4 * REVOLUTION_NS / 17);
    let settled = b.now + 20_000;
    p.run(b, settled);
    let _ = p.steps();
    let (_, status) = b.cycle(REGS, None);
    let written =
        p.memory().transfers.iter().filter_map(|t| t.wrote.map(|w| (t.addr, w))).collect();
    (status, written)
}

/// **A read of two blocks increments the disk address between them.**
///
/// The command list has two CCWs, the first with its More flag. After the
/// first block `047` finds the memory side still busy, `JUMP/START` takes
/// the sequencer through `050`, `FUNC/INCREMENT ADDRESS`, and round again;
/// the Am25LS2536 at DCHDCM 0A23, holding the next-block code it latched
/// off the header's top two bits, pulses `INC BLOCK^`, and the second
/// header compares against block 3. Both pages land, and the disk address
/// register reads back "the address of the last block transferred".
#[test]
fn a_read_of_two_blocks_increments_the_address() {
    let n = cadrdc();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    let mut drive = quick_drive(t0);
    let (two, three) = (words(51), words(52));
    assert!(drive.unit.write_block_at(0, 0, 2, &two));
    assert!(drive.unit.write_block_at(0, 0, 3, &three));
    p.plug(&mut b, &n, drive);
    p.with_memory(&b, 1 << 15);
    p.run(&mut b, t0 + 10_000);

    let (status, written) = read_blocks(&mut p, &mut b, 2, &[PAGE, PAGE2]);
    eprintln!("blocks 2 and 3: status {status:o}");
    assert_eq!(status & 1, 1, "not active: {status:o}");
    assert_eq!(status & ERRORS, 0, "no error: {status:o}");
    assert_eq!(written.len(), 512, "two pages of words");
    assert_eq!(&p.memory().words[PAGE as usize..PAGE as usize + 256], &two[..]);
    assert_eq!(&p.memory().words[PAGE2 as usize..PAGE2 as usize + 256], &three[..]);
    let (_, back) = b.cycle(REGS + 2, None);
    assert_eq!(back, 3, "the last block transferred");
}

/// **A read across the end of a cylinder seeks to the next one.** The last
/// block of the last head is next-block code 2, "block 0 on head 0 of next
/// cylinder", and the Am25LS2536 at DCHDCM 0A23 pulses `INC CYL^` for it:
/// on DCDA that steps the three cylinder counters and, through the
/// inverter at 0C23 and the NAND at 0A28, clears the head and the block.
/// The sequencer's `JUMP/START` then re-enters the read program at `000`,
/// whose first four steps are a cylinder tag and a wait on `-ON CYL SYNC`,
/// so the crossing is a real seek and not just a counter.
///
/// **The band crosses cylinders and the boot PROM never does**: the PROM
/// transfers one page a command, while `COLD-DISK-READ` hands
/// `START-DISK-N-PAGES` up to 512 of them in one command list. So this is
/// on the band's path and off the PROM's, which is the shape issue 88 is
/// looking for. It works: both pages land, and the disk address register
/// reads back cylinder 1, head 0, block 0.
#[test]
fn a_read_across_a_cylinder_seeks_to_the_next() {
    let n = cadrdc();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    let mut drive = quick_drive(t0);
    let g = Geometry::T300;
    let (last, head, block) = (g.blocks_per_track - 1, g.heads - 1, 0);
    let (end, next) = (words(91), words(92));
    assert!(drive.unit.write_block_at(0, head, last, &end));
    assert!(drive.unit.write_block_at(1, 0, block, &next));
    p.plug(&mut b, &n, drive);
    p.with_memory(&b, 1 << 15);
    p.run(&mut b, t0 + 10_000);

    let first = head << 8 | last;
    let (status, written) = read_blocks(&mut p, &mut b, first, &[PAGE, PAGE2]);
    eprintln!("cylinder 0 head {head} block {last}, then cylinder 1: status {status:o}");
    assert_eq!(status & 1, 1, "not active: {status:o}");
    assert_eq!(status & ERRORS, 0, "no error: {status:o}");
    assert_eq!(written.len(), 512, "two pages of words");
    assert_eq!(&p.memory().words[PAGE as usize..PAGE as usize + 256], &end[..]);
    assert_eq!(&p.memory().words[PAGE2 as usize..PAGE2 as usize + 256], &next[..]);
    let (_, back) = b.cycle(REGS + 2, None);
    assert_eq!(back, 1 << 16, "cylinder 1, head 0, block 0: the last block transferred");
    assert_eq!(p.drive().position(), (1, 0), "the heads on the next cylinder");
}

/// **Both busy lines low with `-LAST CCW` high is the middle of a chained
/// transfer, not a fault.** Issue 88 read five nets off a stuck machine ---
/// `-ACTIVE`, `-MBUSY` and `-BUSY` low, `-LAST CCW` and `END PAGE CLK`
/// high --- and took them for two halves that had each failed to finish.
/// This is the same five nets watched through a healthy two-page read, and
/// they stand in exactly that state for the whole of the first page.
///
/// What the run shows, and none of it is assumed:
///
/// - `-MBUSY` rises **once**, at the end of the last page, and `-LAST CCW`
///   is low at that edge --- the second CCW's More flag, clear;
/// - `-LAST CCW` is high from the first CCW fetch until the second, which
///   is the whole of the first page, with both busy lines low throughout;
/// - `-BUSY` rises once, and after `-MBUSY`: for a read `DONE`'s two terms
///   are `-MBUSY` and `LAST CCW`, and the channel gets there first;
/// - the channel makes no request of its own before the disk gives it a
///   word --- `CHAN.RQ` for a read is `CLK MWD4` at the 9S42 on DCCHAN
///   0D20 --- so on a read **the memory side is downstream of the
///   sequencer**, and both halves busy is one stall, not two.
#[test]
fn both_busy_lines_low_is_the_middle_of_a_chained_read() {
    const NAMES: [&str; 6] = ["-BUSY", "-MBUSY", "-ACTIVE", "-LAST CCW", "END PAGE CLK", "NEW CCW"];
    let (busy, mbusy, active, last_ccw, end_page, new_ccw) = (0, 1, 2, 3, 4, 5);
    let n = cadrdc();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    let mut drive = quick_drive(t0);
    let (two, three) = (words(81), words(82));
    assert!(drive.unit.write_block_at(0, 0, 2, &two));
    assert!(drive.unit.write_block_at(0, 0, 3, &three));
    p.plug(&mut b, &n, drive);
    p.with_memory(&b, 1 << 15);
    p.run(&mut b, t0 + 10_000);

    let watch: Vec<NetId> = NAMES.iter().map(|s| b.net(s)).collect();
    let read = |b: &XbusMaster| -> Vec<Level> { watch.iter().map(|&x| b.chip.net(x)).collect() };
    let idle = read(&b);
    assert_eq!(idle[busy], Level::High, "not busy before the command");
    assert_eq!(idle[mbusy], Level::High);
    assert_eq!(idle[active], Level::High);
    assert_eq!(idle[last_ccw], Level::Low, "no CCW fetched yet: the flop's power-up zero");

    // Two CCWs, the first with MIT's More flag.
    let m = p.memory.as_mut().unwrap();
    m.words[CLP as usize] = PAGE | 1;
    m.words[CLP as usize + 1] = PAGE2;
    m.transfers.clear();
    b.cycle(REGS, Some(0));
    b.cycle(REGS + 1, Some(CLP));
    b.cycle(REGS + 2, Some(2));
    p.cycle(&mut b, REGS + 3, Some(0));

    // Every transition of the six, from START until both halves are idle.
    let deadline = b.now + REVOLUTION_NS + 4 * REVOLUTION_NS / 17;
    let mut seen: Vec<(u64, Vec<Level>)> = vec![(b.now, read(&b))];
    loop {
        let now = read(&b);
        if now != seen[seen.len() - 1].1 {
            seen.push((b.now, now.clone()));
        }
        if now[busy] == Level::High && now[mbusy] == Level::High {
            break;
        }
        assert!(b.now < deadline, "still busy at {} ns: {now:?}", b.now - t0);
        let next = b.now + 5;
        p.run(&mut b, next);
    }
    let _ = p.steps();

    let rises = |k: usize| -> Vec<usize> {
        (1..seen.len())
            .filter(|&i| seen[i].1[k] == Level::High && seen[i - 1].1[k] == Level::Low)
            .collect()
    };
    let mrise = rises(mbusy);
    assert_eq!(mrise.len(), 1, "-MBUSY rises once: {mrise:?}");
    let mrise = mrise[0];
    assert_eq!(seen[mrise].1[last_ccw], Level::Low, "the last CCW's More flag is clear");
    assert_eq!(seen[mrise].1[end_page], Level::High, "on END PAGE CLK's rise");
    let brise = rises(busy);
    assert_eq!(brise.len(), 1, "-BUSY rises once: {brise:?}");
    assert!(
        brise[0] > mrise,
        "the channel finishes first: -MBUSY at {} ns, -BUSY at {} ns",
        seen[mrise].0 - t0,
        seen[brise[0]].0 - t0
    );

    // The state the stuck machine was read in, and how long a healthy read
    // spends in it.
    let stuck = |s: &[Level]| {
        s[busy] == Level::Low
            && s[mbusy] == Level::Low
            && s[active] == Level::Low
            && s[last_ccw] == Level::High
            && s[end_page] == Level::High
    };
    let (mut held, mut run, mut run_at) = (0u64, 0u64, 0u64);
    for i in 1..seen.len() {
        if stuck(&seen[i - 1].1) {
            run += seen[i].0 - seen[i - 1].0;
            if run > held {
                held = run;
                run_at = seen[i].0 - run;
            }
        } else {
            run = 0;
        }
    }
    let whole = seen[seen.len() - 1].0 - seen[0].0;
    assert!(held > 500_000, "{held} ns unbroken in the read's five readings, of {whole} ns");
    eprintln!(
        "issue 88's five readings held unbroken for {held} ns from {} ns into a {whole} ns \
         two-page read",
        run_at - seen[0].0
    );

    // The first CCW's More flag is what puts -LAST CCW up, and NEW CCW is
    // down for the whole of the page that follows it.
    let first = (1..seen.len())
        .find(|&i| seen[i].1[last_ccw] == Level::High && seen[i - 1].1[last_ccw] == Level::Low)
        .expect("-LAST CCW goes up on the first CCW fetch");
    assert!(first < mrise, "and stays up until the second fetch");
    assert_eq!(seen[first].1[new_ccw], Level::High, "on the fetch cycle itself");

    let written: Vec<(u32, u32)> =
        p.memory().transfers.iter().filter_map(|t| t.wrote.map(|w| (t.addr, w))).collect();
    assert_eq!(written.len(), 512, "two pages of words");
    let (_, status) = b.cycle(REGS, None);
    assert_eq!(status & 1, 1, "not active: {status:o}");
    assert_eq!(status & ERRORS, 0, "no error: {status:o}");
}

/// The nets one chained transfer is watched through, in the order a row of
/// [`Chain::seen`] carries them; the constants below name the columns. The
/// last four are the write path's: what the channel asks for, and the
/// write buffer it is filling.
const CHAIN_NETS: [&str; 19] = [
    "-LAST CCW",
    "LAST CCW",
    "NEW CCW",
    "CCW CLK",
    "CHAN.ACK.T0",
    "-CHAN.ACK.T1",
    "XBI0",
    "-CHAN.MASTER",
    "END PAGE CLK",
    "-MBUSY",
    "-BUSY",
    "DONE",
    "DONE TEST",
    "-CMD1",
    "-CMD.FROM.MEMORY",
    "CHAN.RQ",
    "MRD FULL",
    "WFIRA",
    "WFORA",
];
const NOT_LAST_CCW: usize = 0;
const LAST_CCW: usize = 1;
const NEW_CCW: usize = 2;
const CCW_CLK: usize = 3;
const ACK_T0: usize = 4;
const NOT_ACK_T1: usize = 5;
const XBI0: usize = 6;
const NOT_CHAN_MASTER: usize = 7;
const END_PAGE_CLK: usize = 8;
const NOT_MBUSY: usize = 9;
const NOT_BUSY: usize = 10;
const DONE: usize = 11;
const DONE_TEST: usize = 12;
const NOT_CMD1: usize = 13;
const NOT_CMD_FROM_MEMORY: usize = 14;
const CHAN_RQ: usize = 15;
const MRD_FULL: usize = 16;
const WFIRA: usize = 17;
const WFORA: usize = 18;

/// The block a chained transfer starts on.
const FIRST_BLOCK: u32 = 2;

/// `sys/cold/qcom.lisp`'s `%DISK-COMMAND-READ` and `%DISK-COMMAND-WRITE`,
/// which `sys/ucadr/uc-cadr.lisp` gives the microcode as
/// `DISK-READ-COMMAND 0` and `DISK-WRITE-COMMAND 11`. Bits `<2:0>` are
/// `newdsk.31`'s sector --- 0 Read, 1 Write --- and bit 3 is the board's
/// `CMD.FROM.MEMORY`, set on exactly the commands whose data comes out of
/// memory: `%DISK-COMMAND-WRITE 11`, `%DISK-COMMAND-WRITE-ALL 13` and
/// `%DISK-COMMAND-READ-COMPARE 10`, and clear on `%DISK-COMMAND-READ 0`
/// and `%DISK-COMMAND-READ-ALL 2`.
const DISK_READ_COMMAND: u32 = 0o0;
const DISK_WRITE_COMMAND: u32 = 0o11;

/// One chained transfer, run to the end and watched at five nanoseconds: every
/// transition of [`CHAIN_NETS`] and of the micro-PC, the command list
/// pointer off `XBAO/22` at each CCW fetch, the bus cycles the board made
/// as master, and the status word after it stopped.
struct Chain {
    /// When, the nineteen nets, and `UPC/6`.
    seen: Vec<(u64, Vec<Level>, u32)>,
    /// `XBAO/22` read while `CCW CLK` is up --- which is inside a fetch,
    /// so `NEW CCW` is high and the address wires carry the command list
    /// pointer. This is the reading issue 88 took off the parked machine.
    clp: Vec<u32>,
    /// The bus cycles the board made as master.
    transfers: Vec<Transfer>,
    status: u32,
    /// The board was still busy when the run's budget ran out.
    stuck: bool,
    /// START, so times read as offsets from it.
    t0: u64,
}

impl Chain {
    /// The rows where a net rose.
    fn rises(&self, net: usize) -> Vec<usize> {
        (1..self.seen.len())
            .filter(|&i| {
                self.seen[i].1[net] == Level::High && self.seen[i - 1].1[net] == Level::Low
            })
            .collect()
    }

    /// The More flag the LS74 at DCCCW 0E20 latched at each CCW fetch, read
    /// at the edge that latched it: `-LAST CCW` high is another CCW to come.
    fn more(&self) -> Vec<bool> {
        self.rises(CCW_CLK).iter().map(|&i| self.seen[i].1[NOT_LAST_CCW] == Level::High).collect()
    }

    /// The addresses the board read as master. **On a Read** those are its
    /// CCW fetches and nothing else, the channel only ever writing the
    /// data; on a Write every cycle is a read and this is not the fetches
    /// --- [`Chain::clp`] is, being taken at `CCW CLK`.
    fn fetches(&self) -> Vec<u32> {
        self.transfers.iter().filter(|t| t.wrote.is_none()).map(|t| t.addr).collect()
    }

    /// The pages the board wrote, in the order it touched them, with how
    /// many words went into each. A Read's, therefore: a Write writes no
    /// memory at all.
    fn pages(&self) -> Vec<(u32, usize)> {
        let mut out: Vec<(u32, usize)> = Vec::new();
        for t in self.transfers.iter().filter(|t| t.wrote.is_some()) {
            let page = t.addr & !0xff;
            match out.last_mut() {
                Some((p, n)) if *p == page => *n += 1,
                _ => out.push((page, 1)),
            }
        }
        out
    }

    /// The rows where the sequencer entered `addr` of `sector`. `UPC/6` is
    /// the six bits below the sector and it is one ahead of the
    /// microinstruction in the UIR, having moved on the edge that latched
    /// it, so `addr` is being executed while the counter reads
    /// `addr - sector + 1`.
    fn entered(&self, sector: u32, addr: u32) -> Vec<usize> {
        let upc = addr - sector + 1;
        (1..self.seen.len())
            .filter(|&i| self.seen[i].2 == upc && self.seen[i - 1].2 != upc)
            .collect()
    }

    /// A net's level while the sequencer stood at the row's micro-PC.
    fn during(&self, row: usize, net: usize) -> Vec<Level> {
        let upc = self.seen[row].2;
        self.seen[row..].iter().take_while(|(_, _, u)| *u == upc).map(|(_, s, _)| s[net]).fold(
            Vec::new(),
            |mut out, l| {
                if out.last() != Some(&l) {
                    out.push(l);
                }
                out
            },
        )
    }
}

/// Addresses in octal, the way the drawings and `newdsk.31` write them.
fn octal(xs: &[u32]) -> String {
    xs.iter().map(|x| format!("{x:o}")).collect::<Vec<_>>().join(" ")
}

/// The board with a quick drive and a memory, and `blocks` blocks written
/// on the pack from [`FIRST_BLOCK`]: what a chained read needs in place
/// before it starts. The words written are returned so the pages can be
/// checked against them.
fn chain_bench(
    n: &Netlist,
    blocks: usize,
) -> (XbusMaster<'_>, Probe, Vec<[u32; muir::disk_unit::BLOCK_WORDS]>) {
    let mut b = controller(n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    let mut drive = quick_drive(t0);
    let written: Vec<[u32; muir::disk_unit::BLOCK_WORDS]> =
        (0..blocks).map(|k| words(100 + k as u32)).collect();
    for (k, w) in written.iter().enumerate() {
        assert!(drive.unit.write_block_at(0, 0, FIRST_BLOCK + k as u32, w));
    }
    p.plug(&mut b, n, drive);
    p.with_memory(&b, 1 << 15);
    p.run(&mut b, t0 + 10_000);
    (b, p, written)
}

/// The command list at `clp` in the harness memory: one CCW a page, the
/// More flag set on all but the last, and `after` in the word immediately
/// past it.
fn command_list(p: &mut Probe, clp: u32, pages: &[u32], after: u32) {
    let m = p.memory.as_mut().unwrap();
    for (k, &page) in pages.iter().enumerate() {
        // "<0> More flag. If this bit is 0, this is the last CCW in the
        // list."
        m.words[clp as usize + k] = page | (k + 1 < pages.len()) as u32;
    }
    m.words[clp as usize + pages.len()] = after;
    m.transfers.clear();
}

/// One [`DISK_READ_COMMAND`] over a command list of `pages.len()` CCWs with
/// `after` past it, watched from START until the board is idle.
fn chained_read(p: &mut Probe, b: &mut XbusMaster, clp: u32, pages: &[u32], after: u32) -> Chain {
    command_list(p, clp, pages, after);
    chained(p, b, DISK_READ_COMMAND, clp, pages.len())
}

/// One [`DISK_WRITE_COMMAND`] over the same list, with the pages already
/// filled by the caller. On a write the channel *reads* memory for both
/// the CCWs and the pages, so a fetch is told from a page read by
/// `NEW CCW` rather than by the direction of the cycle.
fn chained_write(p: &mut Probe, b: &mut XbusMaster, clp: u32, pages: &[u32], after: u32) -> Chain {
    command_list(p, clp, pages, after);
    chained(p, b, DISK_WRITE_COMMAND, clp, pages.len())
}

/// The command written to the four registers and then run, watched.
fn chained(p: &mut Probe, b: &mut XbusMaster, cmd: u32, clp: u32, pages: usize) -> Chain {
    b.cycle(REGS, Some(cmd));
    b.cycle(REGS + 1, Some(clp));
    b.cycle(REGS + 2, Some(FIRST_BLOCK));
    p.start(b, REGS + 3, Some(0));

    let t0 = b.now;
    let watch: Vec<NetId> = CHAIN_NETS.iter().map(|s| b.net(s)).collect();
    let upc: Vec<NetId> = (0..6).map(|k| b.net(&format!("UPC{k}"))).collect();
    let xbao: Vec<NetId> = (0..22).map(|k| b.net(&format!("XBAO{k}"))).collect();
    // A block at the end of the track is most of a revolution away, and
    // then one sector a page.
    let deadline = t0 + REVOLUTION_NS + (pages as u64 + 1) * REVOLUTION_NS / 17;
    let mut seen: Vec<(u64, Vec<Level>, u32)> = Vec::new();
    let (mut clp, mut pulse, mut stopped) = (Vec::new(), false, None);
    while b.now < deadline {
        let now: Vec<Level> = watch.iter().map(|&x| b.chip.net(x)).collect();
        let pc = b.chip.read(&upc) as u32;
        if seen.last().is_none_or(|(_, s, u)| *s != now || *u != pc) {
            seen.push((b.now, now.clone(), pc));
        }
        if now[CCW_CLK] == Level::High {
            if !pulse {
                clp.push(b.chip.read(&xbao) as u32);
            }
            pulse = true;
        } else {
            pulse = false;
        }
        // `-BUSY` rises a `UCLK^` after `DONE`, so the run goes five
        // microseconds past the stop to have both edges in the record.
        if !p.busy(b) {
            let at = *stopped.get_or_insert(b.now);
            if b.now >= at + 5_000 {
                break;
            }
        }
        let next = b.now + 5;
        p.run(b, next);
    }
    let stuck = p.busy(b);
    // The memory side may still be emptying the FIFO.
    let settled = b.now + 20_000;
    p.run(b, settled);
    // Three pages walk the read program three times; printing it would
    // bury the reading this test is here for.
    p.forget();
    let transfers = p.memory().transfers.clone();
    let (_, status) = b.cycle(REGS, None);
    Chain { seen, clp, transfers, status, stuck, t0 }
}

/// **The channel fetches one CCW a page and never the word past the end of
/// the list.** Issue 88 read the command list pointer off a parked machine
/// marching to `0o41000`, one word past a 512-entry list, with `-LAST CCW`
/// high at every sample; and the hypothesis put to this bench was that the
/// board runs one CCW ahead, so that while it finishes the last real CCW it
/// has already fetched the word past the list and latched that word's bit
/// 0. A bench of zeros past the list could not show it, because zero has
/// bit 0 clear, where the band's own memory has data there.
///
/// **It does not happen.** Three CCWs, the third with its More flag clear,
/// and the word immediately past the list a valid-looking page address with
/// bit 0 **set**: the board fetches three CCWs and stops. The command list
/// pointer, `XBAO/22` at each `CCW CLK`, reads `4000`, `4001`, `4002` and
/// goes no further; `-LAST CCW` follows the fetch it is in --- high, high,
/// low; three pages are filled and the page the word past the list names is
/// never written.
///
/// **There is no prefetch to have**, and the wiring says why. `-END OF
/// PAGE` is `-RCO`, pin 19, of the 74LS569 word counter at DCCCW 0E22, and
/// it is the D, pin 12, of the LS74 at DCCCW 0E20 clocked by `-CHAN.MASTER`
/// on pin 11, one edge a channel bus cycle. So `NEW CCW` comes up as a
/// page's 256th word goes by, and the channel's *next* cycle is a fetch ---
/// of the CCW that governs the page after it. The list is walked one CCW
/// behind the pages, never ahead; and the channel asks for that next cycle
/// only once the disk has given it another word, so a run whose last page
/// is full never asks at all.
///
/// The controls are here because without them the test proves nothing: the
/// same chain with zeros past the list stops in exactly the same place; a
/// list of one CCW --- whose only entry is the last one --- stops after one
/// page; and the same three CCWs at the cold boot's own geometry, where the
/// word past the list is [`COPY_BUFFER`], the first word of the first page,
/// stop too --- there the read plants the trap itself, writing band data
/// with its bit 0 set into the word its own list ends against.
#[test]
fn the_word_past_the_ccw_list_is_never_fetched() {
    const PAGE3: u32 = 0o40000;
    const PAST: u32 = 0o50000;
    let n = cadrdc();
    let list = [PAGE, PAGE2, PAGE3];

    let (mut b, mut p, written) = chain_bench(&n, list.len());
    let c = chained_read(&mut p, &mut b, CLP, &list, PAST | 1);
    assert!(!c.stuck, "the read never stopped: fetches {}", octal(&c.fetches()));
    assert_eq!(c.fetches(), [CLP, CLP + 1, CLP + 2], "one CCW a page and no more");
    assert_eq!(c.clp, c.fetches(), "XBAO/22 at each CCW CLK is the address of that cycle");
    assert_eq!(c.more(), [true, true, false], "-LAST CCW at each fetch's own CCW CLK");
    assert_eq!(c.pages(), [(PAGE, 256), (PAGE2, 256), (PAGE3, 256)], "three pages, in order");
    assert_eq!(c.status & 1, 1, "not active: {:o}", c.status);
    assert_eq!(c.status & ERRORS, 0, "no error: {:o}", c.status);
    for (page, w) in list.iter().zip(&written) {
        let at = *page as usize;
        assert_eq!(&p.memory().words[at..at + 256], &w[..], "page {page:o}");
    }
    assert!(
        p.memory().words[PAST as usize..PAST as usize + 256].iter().all(|&w| w == 0),
        "the page the word past the list names is untouched"
    );
    eprintln!(
        "{:o} past the list: CLP {}, -LAST CCW {:?}, {} words in {} pages, status {:o}",
        PAST | 1,
        octal(&c.clp),
        c.more(),
        c.pages().iter().map(|&(_, n)| n).sum::<usize>(),
        c.pages().len(),
        c.status
    );

    // The control the hypothesis says would hide it. It does not differ.
    let (mut b, mut p, _) = chain_bench(&n, list.len());
    let zeros = chained_read(&mut p, &mut b, CLP, &list, 0);
    assert!(!zeros.stuck, "zeros past the list: the read never stopped");
    assert_eq!(zeros.fetches(), c.fetches(), "the same fetches as with the word set");
    assert_eq!(zeros.more(), c.more());
    assert_eq!(zeros.pages(), c.pages());
    assert_eq!(zeros.status & ERRORS, 0, "no error: {:o}", zeros.status);

    // And a list of one, whose only CCW is the last one.
    let (mut b, mut p, _) = chain_bench(&n, 1);
    let one = chained_read(&mut p, &mut b, CLP, &[PAGE], PAST | 1);
    assert!(!one.stuck, "a list of one: the read never stopped");
    assert_eq!(one.fetches(), [CLP], "one CCW fetched");
    assert_eq!(one.more(), [false], "and it is the last");
    assert_eq!(one.pages(), [(PAGE, 256)]);
    assert_eq!(one.status & 1, 1, "not active: {:o}", one.status);
    assert_eq!(one.status & ERRORS, 0, "no error: {:o}", one.status);

    // And at the cold boot's own geometry, [`COPY_BUFFER`], where the word
    // past the list is the first word of the first page: the read plants
    // the trap itself, out of the band, and still stops.
    let cold = [COPY_BUFFER, COPY_BUFFER + 0o400, COPY_BUFFER + 0o1000];
    let clp = cold_clp(cold.len());
    let (mut b, mut p, _) = chain_bench(&n, 0);
    let blocks = pages_to_write(cold.len());
    for (k, w) in blocks.iter().enumerate() {
        let drive = &mut p.cable.as_mut().unwrap().drive;
        assert!(drive.unit.write_block_at(0, 0, FIRST_BLOCK + k as u32, w));
    }
    let c2 = chained_read(&mut p, &mut b, clp, &cold, 0);
    assert!(!c2.stuck, "the cold geometry: the read never stopped");
    assert_eq!(c2.fetches(), [clp, clp + 1, clp + 2], "three fetches, and 41000 not among them");
    assert_eq!(c2.more(), [true, true, false]);
    assert_eq!(c2.pages(), [(cold[0], 256), (cold[1], 256), (cold[2], 256)]);
    assert_eq!(c2.status & ERRORS, 0, "no error: {:o}", c2.status);
    assert_eq!(
        p.memory().words[COPY_BUFFER as usize] & 1,
        1,
        "and the read left the word past the list carrying a More flag"
    );
}

/// **`CCW CLK` latches the More flag of the fetch it is in.** The other
/// half of the prefetch hypothesis: if the flop took its D from the
/// *following* fetch, the last real CCW's More flag would be the word past
/// the list's. It does not.
///
/// `CCW CLK` is the LS11 at DCCCW 0D09, pins 1, 2 and 13 into pin 12:
/// `CHAN.ACK.T0` and `NEW CCW` and `-CHAN.ACK.T1`. The two taps are one
/// delay line, the TD250 at DCCHAN 0C20 --- `CHAN.ACK.T0` its 100 ns tap on
/// pin 4, `CHAN.ACK.T1` its 150 ns tap on pin 10 --- started on the
/// acknowledgement of the channel's own bus cycle, the NOR pair at DCCHAN
/// 0E16 taking `-CHAN.XREQ` with `XACK` or `NXM ACK`. So `CCW CLK` is a
/// 50 ns window opening 100 ns after that cycle is acknowledged, and it is
/// gated by `NEW CCW`, which is up only during a CCW fetch.
///
/// Measured at five nanoseconds, for every fetch of a three-CCW chain and
/// the last one especially: the window opens 250 ns after the harness
/// memory sees the request and is 50 ns wide --- the memory's own 150 ns to
/// acknowledge plus the delay line's 100 --- and it closes on the same edge
/// that ends the cycle, `-CHAN.MASTER` rising. `XBI0`, the 26S10 receiver
/// at DCXBUS 0F25 pin 3 off `-XBUS0` on pin 2, is settled at the fetched
/// word's bit 0 from the moment the cycle begins --- the whole 250 ns
/// before the edge, and for a word whose bit 0 is clear since long before
/// that --- and `-LAST CCW` takes it there. No later fetch has begun: the
/// next request is a page away.
#[test]
fn ccw_clk_latches_the_fetch_it_is_in() {
    const PAGE3: u32 = 0o40000;
    const PAST: u32 = 0o50000;
    let n = cadrdc();
    let list = [PAGE, PAGE2, PAGE3];
    let (mut b, mut p, _) = chain_bench(&n, list.len());
    let c = chained_read(&mut p, &mut b, CLP, &list, PAST | 1);
    assert!(!c.stuck, "the read never stopped");

    let opens = c.rises(CCW_CLK);
    let reads: Vec<&Transfer> = c.transfers.iter().filter(|t| t.wrote.is_none()).collect();
    assert_eq!(opens.len(), reads.len(), "one CCW CLK a fetch");
    assert_eq!(opens.len(), list.len(), "and one fetch a page");

    for (k, (&row, cycle)) in opens.iter().zip(&reads).enumerate() {
        let (at, _, _) = c.seen[row];
        assert_eq!(at, cycle.at + 250, "fetch {k}: CCW CLK opens 250 ns into the cycle");
        assert_eq!(c.seen[row].1[NEW_CCW], Level::High, "fetch {k}: inside a CCW fetch");
        assert_eq!(c.seen[row].1[ACK_T0], Level::High, "fetch {k}: the 100 ns tap opens it");
        assert_eq!(c.seen[row].1[NOT_ACK_T1], Level::High, "fetch {k}: the 150 ns tap has not");
        let shut = (row + 1..c.seen.len())
            .find(|&i| c.seen[i].1[CCW_CLK] == Level::Low)
            .expect("CCW CLK comes back down");
        assert_eq!(c.seen[shut].0, at + 50, "fetch {k}: 50 ns wide, the two taps");
        assert_eq!(c.seen[shut].1[NOT_ACK_T1], Level::Low, "fetch {k}: the 150 ns tap shuts it");
        assert_eq!(
            c.seen[shut].1[NOT_CHAN_MASTER],
            Level::High,
            "fetch {k}: and shuts as -CHAN.MASTER ends the cycle"
        );
        // The word on the bus at the edge, and how long it had been there.
        let want = if k + 1 < list.len() { Level::High } else { Level::Low };
        assert_eq!(c.seen[row].1[XBI0], want, "fetch {k}: XBI0 is the CCW's own bit 0");
        assert_eq!(c.seen[row].1[NOT_LAST_CCW], want, "fetch {k}: and -LAST CCW takes it");
        let settled =
            (0..row).rev().find(|&i| c.seen[i].1[XBI0] != want).map_or(c.t0, |i| c.seen[i + 1].0);
        assert!(
            settled <= cycle.at,
            "fetch {k}: XBI0 settled at {} ns, the cycle began at {} ns",
            settled - c.t0,
            cycle.at - c.t0
        );
        // No later fetch is in flight: the next request is a page away.
        if let Some(next) = reads.get(k + 1) {
            assert!(next.at > c.seen[shut].0, "fetch {k}: the next cycle has not begun");
        }
        eprintln!(
            "fetch {k} at {:o}: cycle at {} ns, CCW CLK {} to {} ns, XBI0 {want:?} since {} ns",
            cycle.addr,
            cycle.at - c.t0,
            at - c.t0,
            c.seen[shut].0 - c.t0,
            settled - c.t0
        );
    }
}

/// **`DONE` at `047` is what ends a chained read, and only the last
/// block's visit finds it true.** `DONE` is the second gate of the 9S42 at
/// DCUC 0D20, its output on pin 9 and its six inputs on 15 down to 10:
///
///     DONE = (-MBUSY and DONE TEST)
///         or (LAST CCW and -CMD.FROM.MEMORY and -CMD1 and DONE TEST)
///
/// `DONE TEST` is UIR bit 12 and `newdsk.31`'s read program sets it at
/// `047` alone, so the sequencer can only finish there; arriving with
/// `DONE` false, its `JUMP/START` takes it round again with `050`'s
/// `FUNC/INCREMENT ADDRESS` in the delay slot. That much is
/// [`the_two_busy_flip_flops_are_one_74s74`]'s reading of the wiring; this
/// is the gate watched through a three-page read.
///
/// **The second term is live on a Read, not dead.** The command register is
/// the 74LS175 at DCCMD 0C21: `XBI1` on pin 12 into `CMD1` on pin 10 and
/// `-CMD1` on pin 11, `XBI3` on pin 4 into `CMD.FROM.MEMORY` on pin 2 and
/// `-CMD.FROM.MEMORY` on pin 3. Read is command 0, so both those D inputs
/// take a zero and both complements stand **high** for the whole of the
/// command --- measured here at every transition of the run. The four-input
/// term is therefore `LAST CCW and DONE TEST`, and a read has two ways to
/// end, not one.
///
/// **What keeps the second term from ending a read a page early is when the
/// last CCW is fetched.** `LAST CCW` is Q̄, pin 6, of the LS74 at DCCCW
/// 0E20, up when the CCW last fetched had its More flag clear; whenever it
/// is up, the next `047` finishes the command whatever the channel is
/// doing. Over three pages it comes up at the third CCW fetch, and that
/// fetch falls **after** the second block's `047` --- measured, 3.94 ms
/// against 3.82 ms. The reason is that the channel asks for the bus only
/// when the disk has given it a word, so the fetch that ends the list waits
/// for the first word of the last block, which the sequencer does not reach
/// until it has left `047` and walked the read program's header again.
///
/// So the first two visits to `047` find both terms false and go round
/// again, and at the third both are true at once: the channel's last word
/// of the last page clocks `END PAGE CLK`, which takes `-LAST CCW` --- low,
/// the last CCW's More flag --- into the memory side of the 74S74 at DCBUSY
/// 0B14 and puts `-MBUSY` up; the sequencer, still walking the block out,
/// reaches `047` 3.5 us later and finds `DONE`; `-BUSY` follows at the next
/// `UCLK^` and the sequencer stops with its counter at zero.
#[test]
fn done_at_047_ends_a_chained_read() {
    const PAGE3: u32 = 0o40000;
    const PAST: u32 = 0o50000;
    let n = cadrdc();
    let list = [PAGE, PAGE2, PAGE3];
    let (mut b, mut p, _) = chain_bench(&n, list.len());
    let c = chained_read(&mut p, &mut b, CLP, &list, PAST | 1);
    assert!(!c.stuck, "the read never stopped");
    assert_eq!(c.pages().len(), list.len(), "every page moved");

    // The second term of the gate, over the whole run.
    assert!(
        c.seen.iter().all(|(_, s, _)| s[NOT_CMD1] == Level::High),
        "-CMD1 stands high for a Read: command 0 puts a zero on the 74LS175's D"
    );
    assert!(
        c.seen.iter().all(|(_, s, _)| s[NOT_CMD_FROM_MEMORY] == Level::High),
        "-CMD.FROM.MEMORY stands high too, so the term is LAST CCW and DONE TEST"
    );

    // `047` once a block, and `DONE TEST` up there and nowhere else.
    let visits = c.entered(0, 0o47);
    assert_eq!(visits.len(), list.len(), "047 once a block");
    assert_eq!(c.rises(DONE_TEST), visits, "DONE TEST comes up at 047 and nowhere else");

    // `LAST CCW` starts up --- the flop's power-up zero, no CCW fetched ---
    // and the first fetch puts it down before the sequencer ever reaches
    // `047`, which is what stops the run ending with nothing moved.
    assert_eq!(c.seen[0].1[LAST_CCW], Level::High, "no CCW fetched yet");
    let first = c.transfers.iter().find(|t| t.wrote.is_none()).expect("a CCW fetch");
    assert!(first.at < c.seen[visits[0]].0, "the first CCW is fetched before the first 047");

    // And it comes back up at the last CCW's fetch, after the block before
    // it has left `047`.
    let up = c.rises(LAST_CCW);
    assert_eq!(up.len(), 1, "LAST CCW comes up once");
    let up = up[0];
    assert_eq!(c.seen[up].1[CCW_CLK], Level::High, "at a CCW CLK, which is inside a fetch");
    assert!(
        c.seen[visits[1]].0 < c.seen[up].0,
        "the second block's 047 at {} ns, LAST CCW at {} ns",
        c.seen[visits[1]].0 - c.t0,
        c.seen[up].0 - c.t0
    );

    // `DONE` at each visit.
    let done: Vec<Vec<Level>> = visits.iter().map(|&i| c.during(i, DONE)).collect();
    assert_eq!(done[0], [Level::Low], "the first block's 047: not done, round again");
    assert_eq!(done[1], [Level::Low], "the second block's 047: not done either");
    assert_eq!(done[2], [Level::High], "the last block's 047: done");
    assert_eq!(c.rises(DONE), [visits[2]], "DONE rises once, as 047 is entered");

    // The channel finishes first, on the last page's 256th word.
    let mbusy = c.rises(NOT_MBUSY);
    assert_eq!(mbusy.len(), 1, "-MBUSY rises once");
    let mbusy = mbusy[0];
    assert_eq!(c.seen[mbusy].1[NOT_LAST_CCW], Level::Low, "taking the last CCW's More flag");
    assert_eq!(c.seen[mbusy].1[END_PAGE_CLK], Level::High, "on END PAGE CLK's rise");
    let last = c.transfers.iter().rev().find(|t| t.wrote.is_some()).expect("a page written");
    assert!(c.seen[mbusy].0 > last.at, "-MBUSY at the end of the last page's last word");
    assert!(c.seen[mbusy].0 < c.seen[visits[2]].0, "and before the 047 that reads it");

    // `-BUSY` follows a `UCLK^` later, and the sequencer stops at zero.
    let busy = c.rises(NOT_BUSY);
    assert_eq!(busy.len(), 1, "-BUSY rises once");
    let busy = busy[0];
    assert!(c.seen[busy].0 > c.seen[visits[2]].0, "-BUSY after DONE");
    assert_eq!(c.seen[busy].2, 0, "the sequencer stopped and its counter cleared");
    assert_eq!(c.status & 1, 1, "not active: {:o}", c.status);
    assert_eq!(c.status & ERRORS, 0, "no error: {:o}", c.status);
    eprintln!(
        "last word {} ns, -MBUSY +{} ns, 047 with DONE +{} ns, -BUSY +{} ns; 047 at {} ns and \
         LAST CCW at {} ns",
        last.at - c.t0,
        c.seen[mbusy].0 - last.at,
        c.seen[visits[2]].0 - c.seen[mbusy].0,
        c.seen[busy].0 - c.seen[visits[2]].0,
        c.seen[visits[1]].0 - c.t0,
        c.seen[up].0 - c.t0
    );
}

/// The net a pin carries. Compared by [`NetId`] rather than by name
/// because [`netlist::parse`] joins MIT's hand jumpers, so a joined net
/// answers to either of its names and reports whichever it kept.
fn pin_net(n: &Netlist, page: &str, reference: &str, pin: u8) -> NetId {
    let mut found = n
        .parts
        .iter()
        .filter(|p| p.page == page && p.reference == reference)
        .filter_map(|p| p.pins.iter().find(|&&(k, _)| k == pin).map(|&(_, net)| net));
    let net =
        found.next().unwrap_or_else(|| panic!("no pin {pin} on the part at {page} {reference}"));
    assert!(found.next().is_none(), "{page} {reference} pin {pin} is on two records");
    net
}

/// A net by name, quoted or not.
fn net_id(n: &Netlist, name: &str) -> NetId {
    n.by_name_id(name)
        .or_else(|| n.by_name_id(&format!("'{name}'")))
        .unwrap_or_else(|| panic!("no net {name}"))
}

/// **What clears `-BUSY`, and what clears `-MBUSY`: the two halves of one
/// 74S74, and what each is waiting for.** Issue 88.
///
/// `-ACTIVE`, `STATUS<0>`, is the LS08 at DCBUSY 0D14: pin 9 `-MBUSY`,
/// pin 10 `-BUSY`, pin 8 `-ACTIVE`. Both halves have to be idle before
/// the band's wait loop can leave, and both halves are one part, the
/// 74S74 at DCBUSY **0B14**.
///
/// **The sequencer's half**, pins 1 to 6: `-START` on 1 is the clear, so a
/// write to register 3 makes the board busy; `-LOSSAGE` on 4 is the
/// preset, so any error ends the command at once; `DONE` on 2 is the D and
/// `UCLK^` on 3 the clock; `Q` on 5 is `-BUSY` and `Q/` on 6 is `BUSY`.
/// So **`-BUSY` waits for `DONE` at a `UCLK^` edge**, and `DONE` is the
/// second gate of the 9S42 at DCUC 0D20, output on pin 9:
///
///     DONE = (-MBUSY and DONE TEST)
///         or (LAST CCW and -CMD.FROM.MEMORY and -CMD1 and DONE TEST)
///
/// `DONE TEST` is a microcode bit --- UIR bit 12, the 74LS273 at DCUI 0D07
/// pin 12 --- and the read program of `cadrdc/newdsk.31` sets it at one
/// address only, `047`. So the sequencer can only finish there, and if
/// `DONE` is false when it arrives the `JUMP/START` on that instruction
/// takes it round the whole program again with `050`'s
/// `FUNC/INCREMENT ADDRESS` in the delay slot. **A read that is not done
/// does not stop: it reads the next block.**
///
/// **The memory channel's half**, pins 8 to 13: `-MSTART` on 10 is the
/// preset --- the OR at DCCMD 0C16, `-START` gated by `CMD2`, so commands
/// 4 to 7 never start the channel --- `-STOPPED BY ERROR` on 13 the clear,
/// `-LAST CCW` on 12 the D and `END PAGE CLK` on 11 the clock; `Q` on 9 is
/// `MBUSY` and `Q/` on 8 is `-MBUSY`. `END PAGE CLK` is `CCO`, pin 18, of
/// the 74LS569 at DCCCW 0E22, the high half of the eight-bit word counter,
/// which follows the clock only at the terminal count; the clock is
/// `-CHAN.MASTER`, one edge per channel bus cycle. So **`-MBUSY` waits for
/// the end of the 256th word of a page**, and takes `-LAST CCW` there.
///
/// `-LAST CCW` is `Q`, pin 5, of the LS74 at DCCCW 0E20, whose D on pin 2
/// is `XBI0` and whose clock on pin 3 is `CCW CLK`: bit 0 of the last
/// channel command word fetched from memory, MIT's More flag, 1 for
/// another CCW to come. **Both of its asynchronous pins are tied high, so
/// nothing resets it**: its value is the last CCW the channel read, and
/// before the first fetch of a run it is the model's power-up zero, which
/// reads `-LAST CCW` low.
#[test]
fn the_two_busy_flip_flops_are_one_74s74() {
    let n = cadrdc();
    let is = |page: &str, reference: &str, pin: u8, name: &str| {
        assert_eq!(
            pin_net(&n, page, reference, pin),
            net_id(&n, name),
            "{page} {reference} pin {pin} is not {name}"
        );
    };

    // -ACTIVE is the AND of the two.
    is("DCBUSY", "0D14", 8, "-ACTIVE");
    is("DCBUSY", "0D14", 9, "-MBUSY");
    is("DCBUSY", "0D14", 10, "-BUSY");
    is("DCSTS", "0A12", 2, "-ACTIVE");
    is("DCSTS", "0A12", 18, "XBO0");

    // The sequencer's half.
    for (pin, name) in
        [(1, "-START"), (2, "DONE"), (3, "UCLK^"), (4, "-LOSSAGE"), (5, "-BUSY"), (6, "BUSY")]
    {
        is("DCBUSY", "0B14", pin, name);
    }
    for (pin, name) in [
        (9, "DONE"),
        (15, "-MBUSY"),
        (14, "DONE TEST"),
        (13, "LAST CCW"),
        (12, "-CMD.FROM.MEMORY"),
        (11, "-CMD1"),
        (10, "DONE TEST"),
    ] {
        is("DCUC", "0D20", pin, name);
    }
    is("DCUI", "0D07", 12, "DONE TEST");
    is("DCUI", "0D07", 13, "UI11");

    // The memory channel's half.
    for (pin, name) in [
        (8, "-MBUSY"),
        (9, "MBUSY"),
        (10, "-MSTART"),
        (11, "END PAGE CLK"),
        (12, "-LAST CCW"),
        (13, "-STOPPED BY ERROR"),
    ] {
        is("DCBUSY", "0B14", pin, name);
    }
    is("DCCMD", "0C16", 8, "-MSTART");
    is("DCCMD", "0C16", 9, "CMD2");
    is("DCCMD", "0C16", 10, "-START");

    // END PAGE CLK is the word counter's clocked carry and goes nowhere
    // else, so nothing but a full page can clear the channel's half.
    is("DCCCW", "0E22", 18, "END PAGE CLK");
    is("DCCCW", "0E22", 2, "-CHAN.MASTER");
    let end_page = net_id(&n, "END PAGE CLK");
    let on: Vec<(&str, &str, u8)> = n
        .parts
        .iter()
        .flat_map(|p| p.pins.iter().map(move |&(pin, net)| (p, pin, net)))
        .filter(|&(_, _, net)| net == end_page)
        .map(|(p, pin, _)| (p.page.as_str(), p.reference.as_str(), pin))
        .collect();
    assert_eq!(on, vec![("DCBUSY", "0B14", 11), ("DCCCW", "0E22", 18)]);

    // And -LAST CCW is the More flag of the CCW, with nothing to reset it.
    is("DCCCW", "0E20", 5, "-LAST CCW");
    is("DCCCW", "0E20", 6, "LAST CCW");
    is("DCCCW", "0E20", 2, "XBI0");
    is("DCCCW", "0E20", 3, "CCW CLK");
    let high = pin_net(&n, "DCCCW", "0E20", 1);
    assert_eq!(high, pin_net(&n, "DCCCW", "0E20", 4), "clear and preset on one net");
    assert_eq!(netlist::plain(n.net(high)), "HI1", "and that net is a pull-up");

    // And on a read the channel asks for nothing until the disk has given
    // it a word: the 9S42 at DCCHAN 0D20's first gate is
    // `(CLK MWD4 and -CMD.FROM.MEMORY) or (-MRD FULL and CMD.FROM.MEMORY
    // and MBUSY)`, whose second term is the write direction's. So on a
    // read the memory side is downstream of the sequencer, and both
    // halves busy is one stall rather than two.
    for (pin, name) in [
        (7, "CHAN.RQ"),
        (1, "CLK MWD4"),
        (2, "-CMD.FROM.MEMORY"),
        (3, "-MRD FULL"),
        (4, "CMD.FROM.MEMORY"),
        (5, "MBUSY"),
    ] {
        is("DCCHAN", "0D20", pin, name);
    }
    is("DCRBUF", "0E08", 15, "CLK MWD4");
}

/// **Nothing but `-ACTIVE` clears the watchdog**, so a command that hangs
/// silently cannot outlive it: the 74393 at DCTMOT 0C03 has `-ACTIVE` on
/// both of its clears, pins 2 and 12, and no other reset. It counts
/// `TIMEOUT.CLK` from the moment the board goes busy and reaches `TIMEOUT`
/// 128 periods later whatever the sequencer is doing in between ---
/// activity does not postpone it, only finishing does.
///
/// That is what makes the pair of readings in issue 88 --- both halves
/// busy, no error --- a statement about the *last* 2.56 seconds and not
/// about the whole run. [`a_hung_command_times_out`] measures the time;
/// this holds the wiring that makes the time unavoidable.
#[test]
fn only_not_active_clears_the_watchdog() {
    let n = cadrdc();
    let active = net_id(&n, "-ACTIVE");
    assert_eq!(pin_net(&n, "DCTMOT", "0C03", 2), active, "the first half's clear");
    assert_eq!(pin_net(&n, "DCTMOT", "0C03", 12), active, "the second half's clear");
    assert_eq!(pin_net(&n, "DCTMOT", "0C03", 1), net_id(&n, "TIMEOUT.CLK"), "the clock");
    assert_eq!(pin_net(&n, "DCTMOT", "0C03", 8), net_id(&n, "TIMEOUT"), "128 periods on");
    // The clock is the 74LS124's own section, enabled by the hand jumper.
    assert_eq!(pin_net(&n, "DCTMOT", "0B04", 7), net_id(&n, "TIMEOUT.CLK"));
    assert_eq!(pin_net(&n, "DCTMOT", "0B04", 6), net_id(&n, "GND"), "-TIMEOUT ENB grounded");
}

/// **A read across the end of a track increments the head and clears the
/// block.** Block 16 is the last on a track, its header's next-block code
/// is 1, and the 2536 pulses `INC HEAD^` instead: `CLR BLOCK` follows from
/// it on DCDA, the head counter steps, and the second block read is head
/// 1 block 0, whose sector is the next one round.
#[test]
fn a_read_across_a_track_steps_the_head() {
    let n = cadrdc();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    let mut drive = quick_drive(t0);
    let (last, next) = (words(61), words(62));
    assert!(drive.unit.write_block_at(0, 0, 16, &last));
    assert!(drive.unit.write_block_at(0, 1, 0, &next));
    // The drive starts at its index pulse, which is what clears the block
    // counter; block 16 then comes round sixteen sector pulses later.
    p.plug(&mut b, &n, drive);
    p.with_memory(&b, 1 << 15);
    p.run(&mut b, t0 + 10_000);

    let (status, written) = read_blocks(&mut p, &mut b, 16, &[PAGE, PAGE2]);
    eprintln!("block 16 then head 1 block 0: status {status:o}");
    assert_eq!(status & 1, 1, "not active: {status:o}");
    assert_eq!(status & ERRORS, 0, "no error: {status:o}");
    assert_eq!(written.len(), 512, "two pages of words");
    assert_eq!(&p.memory().words[PAGE as usize..PAGE as usize + 256], &last[..]);
    assert_eq!(&p.memory().words[PAGE2 as usize..PAGE2 as usize + 256], &next[..]);
    let (_, back) = b.cycle(REGS + 2, None);
    assert_eq!(back, 1 << 8, "head 1, block 0: the last block transferred");
    assert_eq!(p.drive().position(), (0, 1), "the drive's head selected");
}

/// One of 01, 03 and 12 run on the board with a drive and a memory: the
/// channel's transfers, the pack, and the status word.
///
/// `within` bounds the run; a command that is still busy at the end of it
/// comes back as `busy`.
struct Reversed {
    /// Still busy when the run ran out.
    busy: bool,
    status: u32,
    /// The addresses the channel stored into, and the ones it fetched.
    stored: Vec<u32>,
    fetched: Vec<u32>,
    block2: Option<[u32; muir::disk_unit::BLOCK_WORDS]>,
    /// Sectors the drive took that did not parse as the format.
    bad: usize,
}

fn reversed_channel(n: &Netlist, cmd: u32, within: u64) -> Reversed {
    let mut b = controller(n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    p.plug(&mut b, n, quick_drive(t0));
    p.with_memory(&b, 1 << 15);
    let data = words(31);
    let m = p.memory.as_mut().unwrap();
    m.words[PAGE as usize..PAGE as usize + 256].copy_from_slice(&data);
    m.words[CLP as usize] = PAGE;
    p.run(&mut b, t0 + 10_000);

    b.cycle(REGS, Some(cmd));
    b.cycle(REGS + 1, Some(CLP));
    b.cycle(REGS + 2, Some(2));
    p.cycle(&mut b, REGS + 3, Some(0));
    // Not `run_to_done`: one of the three does not finish, and that is
    // one of the things being measured.
    let deadline = b.now + within;
    while p.busy(&b) && b.now < deadline {
        let to = b.now + 5;
        p.run(&mut b, to);
    }
    let busy = p.busy(&b);
    let settled = b.now + 20_000;
    p.run(&mut b, settled);
    let (_, status) = b.cycle(REGS, None);
    let transfers = p.memory().transfers.clone();
    let stored: Vec<u32> = transfers.iter().filter(|t| t.wrote.is_some()).map(|t| t.addr).collect();
    let fetched: Vec<u32> =
        transfers.iter().filter(|t| t.wrote.is_none()).map(|t| t.addr).collect();
    let drive = &mut p.cable.as_mut().unwrap().drive;
    Reversed {
        busy,
        status,
        stored,
        fetched,
        block2: drive.unit.block_at(0, 0, 2),
        bad: drive.bad_writes,
    }
}

/// **The reversed memory channel stores where it should fetch.**
///
/// `<3>` of the command steers the channel and reaches no part of the
/// command PROM --- `cadrdc/newdsk.31`, "Commands 4-7 do not use the
/// memory channel", and the PROM "is divided into 8 sectors of 64 words
/// each", so `<2:0>` alone picks the sector. MIT's table leaves 01 and 03
/// out: they are the Write and Write All sectors with the channel pointed
/// the other way, so the fifo is filled from the end it is normally
/// emptied at.
///
/// Both run their sector and both store 256 words into the page rather
/// than fetching them --- one CCW word is the only thing read. They part
/// on the outcome: **01 ends with no error at all** and puts a well-formed
/// block on the pack, where **03 ends with Overrun and Transfer Aborted**
/// and puts something on the disk that does not parse, so no block lands.
///
/// This is the measurement issue #34 asked for, and
/// `src/disk_controller.rs` follows it.
#[test]
fn the_reversed_memory_channel_stores_where_it_should_fetch() {
    let n = cadrdc();
    let two_turns = 2 * REVOLUTION_NS;

    let Reversed { busy, status, stored, fetched, block2, bad } =
        reversed_channel(&n, 0o01, two_turns);
    assert!(!busy, "0001 finishes");
    assert_eq!(status & ERRORS, 0, "0001 ends with no error: {status:o}");
    assert_eq!(fetched, [CLP], "0001 reads the CCW and nothing else");
    assert_eq!(stored.len(), 256, "0001 stores a page: {}", stored.len());
    assert_eq!(bad, 0, "0001 puts a well-formed block on the pack");
    assert_ne!(block2, Some([0; muir::disk_unit::BLOCK_WORDS]), "0001 writes block 2");

    let Reversed { busy, status, stored, fetched, block2, bad } =
        reversed_channel(&n, 0o03, two_turns);
    assert!(!busy, "0003 finishes");
    assert_ne!(status & (1 << 14), 0, "0003 overruns: {status:o}");
    assert_ne!(status & (1 << 13), 0, "0003 aborts: {status:o}");
    assert_eq!(status & (1 << 11), 0, "0003 does not time out: {status:o}");
    assert_eq!(fetched, [CLP], "0003 reads the CCW and nothing else");
    assert_eq!(stored.len(), 256, "0003 stores a page: {}", stored.len());
    assert_eq!(bad, 1, "0003 puts something on the disk that does not parse");
    assert_eq!(block2, Some([0; muir::disk_unit::BLOCK_WORDS]), "0003 lands no block");
}

/// **And the reversed Read All hangs to the watchdog.**
///
/// `0012` is the Read All sector with the channel pointed at memory
/// instead of away from it. It reads eighteen words out of memory, stores
/// nothing, and never finishes: the board's own timer stops it with
/// **Timeout and Transfer Aborted**, `STATUS<11>` and `<13>`.
///
/// So of the three codes MIT's table leaves out of the transfer sectors,
/// this is the one that behaves like a reserved code, and
/// `src/disk_controller.rs` gives it [`muir::disk_controller`]'s `hang`.
/// It was expected to be the harmless one of the three; it is not.
#[test]
#[ignore = "2.56 seconds of board, minutes of wall clock; run with --ignored"]
fn the_reversed_read_all_hangs_to_the_watchdog() {
    let n = cadrdc();
    let Reversed { busy, status, stored, fetched, block2, bad } =
        reversed_channel(&n, 0o12, muir::disk_controller::TIMEOUT_NS + REVOLUTION_NS);
    assert!(!busy, "the watchdog stopped it");
    assert_ne!(status & (1 << 11), 0, "0012 times out: {status:o}");
    assert_ne!(status & (1 << 13), 0, "0012 aborts: {status:o}");
    assert!(stored.is_empty(), "0012 stores nothing: {stored:?}");
    assert_eq!(fetched.len(), 18, "0012 reads eighteen words: {}", fetched.len());
    assert_eq!(bad, 0, "and puts nothing on the disk");
    assert_eq!(block2, Some([0; muir::disk_unit::BLOCK_WORDS]), "block 2 untouched");
}

/// **The cable to the DISK MULTIPLEXOR, as MIT's wire list has it.** The
/// controller's edge connector is the `DCEDGE` page of `cadrdc/dc.wlr`:
/// fifty-three posts, of which twenty-four are ground, one is `NC`, and
/// the twenty-eight below are the cable. This is the controller's half of issue
/// #1's cable, pinned here so that the multiplexor is wired to something
/// checked rather than to a reading of the drawings.
///
/// Nothing else in this project reads `DCEDGE`, so without this the six
/// one-board jumpers in `src/netlist.rs` would be the only record of what
/// these posts carry, and they name six of the twenty-eight.
///
/// **Three posts carry two labels, and they are the fan-out.** `DR2` is
/// `DISK.CLK^` and `UNIT.0.CLOCK^`, `DS2` is `READ DATA` and `UNIT 0 READ
/// DATA`, `DT2` is `BLOCK.CLK^` and `UNIT.0.SECTOR^`. On a one-board
/// machine those are the same wire because there is one drive; a
/// multiplexor is the thing that makes them different, taking the
/// controller's single `DISK.CLK^`, `READ DATA` and `BLOCK.CLK^` and
/// choosing which of eight units they come from. The names are asserted
/// here in full, both labels, so that the pair is on the record before
/// anything is wired between them.
///
/// Four copies of `dc.wlr` are on the tapes and agree on every post they
/// share; the two older ones lack `DD2` and `EL2`/`EM2`/`EN2`, which were
/// added later. The copy in `mit/` is the newest, and is the one with all
/// fifty-three.
#[test]
fn the_multiplexor_cable_is_mits_edge_connector() {
    let n = netlist::parse(CADRDC).unwrap();
    let signals = support::wire_list(&n, &["cadrdc", "dc.wlr"]);
    let mut posts: BTreeMap<String, String> = BTreeMap::new();
    for s in &signals {
        for p in s.pins.iter().filter(|p| p.body == "CON" && p.page == "DCEDGE") {
            posts.insert(p.location.clone(), s.names.join(" = "));
        }
    }
    assert_eq!(posts.len(), 53, "posts on the DCEDGE connector");
    let grounds = posts.values().filter(|v| *v == "GND").count();
    assert_eq!(grounds, 24, "of them ground");
    let signal_posts: BTreeMap<&str, &str> = posts
        .iter()
        .filter(|(_, v)| *v != "GND" && *v != "NC")
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let want: BTreeMap<&str, &str> = BTreeMap::from([
        ("DD2", "XINIT"),
        ("DE2", "SEL UNIT ATTENTION"),
        ("DF2", "ANY ATTENTION"),
        ("DH2", "UNIT 0 ATTENTION"),
        ("DJ2", "-LOAD DA"),
        ("DL2", "NO SELECT"),
        ("DM2", "MULTIPLE SELECT"),
        ("DN2", "WRITE DATA"),
        ("DP2", "WRITE GATE"),
        ("DR2", "DISK.CLK^ = UNIT.0.CLOCK^"),
        ("DS2", "READ DATA = UNIT 0 READ DATA"),
        ("DT2", "BLOCK.CLK^ = UNIT.0.SECTOR^"),
        ("DU2", "BLOCK.CTR0"),
        ("DV2", "BLOCK.CTR1"),
        ("ED2", "BLOCK.CTR2"),
        ("EE2", "BLOCK.CTR3"),
        ("EF2", "BLOCK.CTR4"),
        ("EH2", "BLOCK.CTR5"),
        ("EJ2", "BLOCK.CTR6"),
        ("EK2", "BLOCK.CTR7"),
        ("EL2", "XBI28"),
        ("EM2", "XBI29"),
        ("EN2", "XBI30"),
        ("EP2", "UNIT0"),
        ("ER2", "UNIT1"),
        ("ES2", "UNIT2"),
        ("ET2", "-CYLINDER TAG"),
        ("EU2", "-HEAD TAG"),
    ]);
    assert_eq!(signal_posts, want, "the cable's posts and what each carries");
}

/// **The six one-board jumpers are the multiplexor's nets, and only those
/// six.** `cadrdc/disk.hand` heads them "not to be installed if this DC is
/// associated with a DM board", so a controller with a multiplexor beside
/// it must have them off and the multiplexor drives those nets instead ---
/// which is what `netlist::parse_with_multiplexor` is for.
///
/// Held from both sides: with the jumpers the three attention nets are one
/// and the unit number and `MULTIPLE SELECT` are ground; without them all
/// six stand apart, ready for the cable. The address and timeout jumpers
/// are not the multiplexor's and stay either way, so the board answers at
/// its own address in both.
#[test]
fn a_multiplexor_leaves_the_one_board_jumpers_off() {
    let one = netlist::parse(CADRDC).unwrap();
    let dm = netlist::parse_with_multiplexor(CADRDC).unwrap();
    let id = |n: &Netlist, name: &str| n.by_name_id(name).unwrap_or_else(|| panic!("{name}"));

    // Jumpered: the three attentions are one net, and the unit number and
    // MULTIPLE SELECT are on ground.
    let attention = id(&one, "'UNIT 0 ATTENTION'");
    assert_eq!(id(&one, "'ANY ATTENTION'"), attention, "ANY ATTENTION is unit 0's");
    assert_eq!(id(&one, "'SEL UNIT ATTENTION'"), attention, "and so is SEL UNIT ATTENTION");
    let gnd = id(&one, "GND");
    for net in ["'MULTIPLE SELECT'", "UNIT0", "UNIT1", "UNIT2"] {
        assert_eq!(id(&one, net), gnd, "{net} is grounded on the one-board controller");
    }

    // With a multiplexor: six nets of their own, for it to drive.
    let mut apart = BTreeSet::new();
    for net in [
        "'UNIT 0 ATTENTION'",
        "'ANY ATTENTION'",
        "'SEL UNIT ATTENTION'",
        "'MULTIPLE SELECT'",
        "UNIT0",
        "UNIT1",
        "UNIT2",
    ] {
        let net = id(&dm, net);
        assert_ne!(net, id(&dm, "GND"), "no multiplexor net is grounded");
        apart.insert(net);
    }
    assert_eq!(apart.len(), 7, "the six jumpers leave seven nets standing apart");

    // The address and timeout jumpers are not the multiplexor's.
    assert_eq!(id(&dm, "'-TIMEOUT ENB'"), id(&dm, "GND"), "the timeout enable stays");
    assert_eq!(id(&dm, "AD14"), id(&dm, "HI1"), "and the address jumpers stay");
}

// --- the model and the board, against each other ----------------------------

/// **Which bits of the status word the two implementations are held to.**
///
/// The disk controller is modelled twice --- behaviourally in
/// `src/disk_controller.rs` and gate-for-gate in `data/CADRDC.netlist` ---
/// and until this section nothing put the same commands to both. Four
/// closed issues were that gap: #8, #34, #36 and #69, each found by a
/// person reading the two side by side rather than by anything failing.
///
/// **Held**: the status bits below, and the words the channel moves.
///
/// **Free, and measured rather than assumed**:
///
/// - `<31:24>`, the block counter, which is "the current rotational
///   position" of the drive. The board's drive here has its seek and
///   settle times cut to nothing so the tests run; the model charges
///   [`muir::disk_unit::seek_ns`]. So the two are at different points of
///   the turn when the transfer ends --- 2 against 17 on the read below
///   --- and the number is right in both.
/// - `<22>`, read-compare difference. MIT: "This bit is undefined unless
///   the command is read-compare." The board leaves it **set** after a
///   plain read and the model leaves it **clear**; undefined is undefined,
///   and holding either to the other would be inventing a rule MIT does
///   not give.
///
/// Timing is free throughout, which is the declaration `busint`'s
/// `IDEAL_DEVICE_NS` already makes: the model answers in no time where the
/// board takes gate delays.
const PAIRED: u32 = !(0xffu32 << 24) & !(1 << 22);

/// The behavioural controller with the same pack under it as the board's
/// drive, charging the drive's own time.
fn model_with(block: u32, data: &[u32; muir::disk_unit::BLOCK_WORDS]) -> (Controller, Vec<u32>) {
    let mut unit = Unit::blank(Geometry::T300);
    assert!(unit.write_block_at(0, 0, block, data), "the pack takes the block");
    let mut d = Controller::default();
    d.attach(0, unit);
    d.timed = true;
    (d, vec![0; 1 << 16])
}

/// Puts one command to the model and runs it to done: the four register
/// writes the board is given, in the same order.
fn model_command(d: &mut Controller, main: &mut [u32], at: u64, cmd: u32, clp: u32, da: u32) {
    d.advance(at);
    d.write(0, cmd, main);
    d.write(1, clp, main);
    d.write(2, da, main);
    d.write(3, 0, main);
    // Timing is free, so this only has to be past the drive's own worst
    // case: a full stroke is 55 ms and a revolution 16.67.
    d.advance(at + 200_000_000);
}

/// **The two implementations move the same words and answer the same
/// status.** A read of one block through the board and through the model,
/// from the same pack, compared bit for bit under [`PAIRED`].
///
/// This is the first thing that puts one command to both. What it holds is
/// small on purpose --- one command, one block --- because the value is
/// the seam existing rather than its coverage: #69 was an attention raised
/// at the wrong instant on one of the two, and nothing but a person
/// reading both would have seen it.
#[test]
fn the_model_and_the_board_read_a_block_alike() {
    let block = 2;
    let data = words(21);

    let n = cadrdc();
    let store = control_store();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    let mut drive = quick_drive(t0);
    assert!(drive.unit.write_block_at(0, 0, block, &data));
    p.plug(&mut b, &n, drive);
    p.with_memory(&b, 1 << 15);
    p.run(&mut b, t0 + 10_000);
    let (board, written) = read_block(&mut p, &mut b, &store, block, PAGE);

    let (mut d, mut main) = model_with(block, &data);
    main[CLP as usize] = PAGE;
    model_command(&mut d, &mut main, t0, 0, CLP, block);
    let model = d.status();

    let page = PAGE as usize;
    eprintln!("board {board:o}, model {model:o}, differing outside PAIRED {:o}", board ^ model);
    assert_eq!(main[page..page + 256], data[..], "the model put the block in memory");
    let moved: Vec<u32> = written.iter().map(|&(_, w)| w).collect();
    assert_eq!(moved, data.to_vec(), "the board put the same words in memory");
    assert_eq!(board & PAIRED, model & PAIRED, "board {board:o} against model {model:o}");
    // And what is free is free because it differs, not because nobody
    // looked: the block counter is the drive's rotational position and
    // these two drives are not at the same point of the turn.
    assert_ne!(board >> 24, model >> 24, "the block counters are the free half");
}

/// **A controller with nothing on its cable answers the same word on both.**
///
/// `tests/disk.rs` asserts the model reads `0o21441` and its comment calls
/// that "the word the netlist board reads" --- **while reading nothing**.
/// That is the restated-constant shape: two copies that agree because one
/// person wrote both. This reads it off the board.
#[test]
fn the_model_and_the_board_agree_with_no_drive() {
    let n = cadrdc();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let settled = b.now + 10_000;
    p.run(&mut b, settled);
    let board = p.cycle(&mut b, REGS, None);

    let mut d = Controller::default();
    d.advance(0);
    let model = d.status();

    eprintln!("no drive: board {board:o}, model {model:o}");
    assert_eq!(board & PAIRED, model & PAIRED, "board {board:o} against model {model:o}");
    assert_eq!(board & PAIRED, 0o21441, "not active, no select, off line, off cylinder, aborted");
}

/// **A seek ends with the same bits up on both.** The command MIT
/// describes as "Initiates a seek to the cylinder specified in the disk
/// address register.  An attention will occur when the seek completes":
/// run past the heads' arrival on each, the attention, the any-attention
/// and the not-active must be up together.
///
/// **And this is where the paired seam's reach ends, which is worth
/// knowing.** #69 was the model raising this attention as the command was
/// stored, where the board's drive raises it when the heads arrive ---
/// and *this test would not have caught it*, measured: with #69 reverted
/// it still passes, because an attention raised early is still up at the
/// end. What catches it is
/// `tests/disk.rs::the_attention_comes_when_the_seek_completes` on one
/// side and `a_seek_with_a_drive_moves_the_heads` on the other, each
/// holding its own implementation to an instant.
///
/// That is not a gap to close but the consequence of timing being free
/// between the two: the board's drive here has its seek cut to
/// microseconds and the model charges the real
/// [`muir::disk_unit::seek_ns`], so there is no instant to compare. What
/// pairing adds is the end state --- that both arrive at the same three
/// bits --- and a divergence in *which* bits, which is what #8, #34 and
/// #36 were.
#[test]
fn the_model_and_the_board_end_a_seek_alike() {
    let cylinder = 100u32;
    let da = cylinder << 16;

    let n = cadrdc();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    p.plug(&mut b, &n, quick_drive(t0));
    p.run(&mut b, t0 + 10_000);
    b.cycle(REGS, Some(0o4));
    b.cycle(REGS + 2, Some(da));
    p.cycle(&mut b, REGS + 3, Some(0));
    p.run_to_done(&mut b, 40_000);
    // The board's drive is the quick one, so its heads are there in
    // microseconds; the wait is the drive's and not the board's.
    let arrived = b.now + 250_000;
    p.run(&mut b, arrived);
    let board = p.cycle(&mut b, REGS, None);

    let (mut d, mut main) = model_with(0, &words(1));
    model_command(&mut d, &mut main, t0, 0o4, 0, da);
    let model = d.status();

    eprintln!("seek: board {board:o}, model {model:o}");
    assert_eq!(board & PAIRED, model & PAIRED, "board {board:o} against model {model:o}");
    // And the bits themselves, so that the two agreeing on nothing would
    // not pass: the seek is done and the drive is asking.
    assert_eq!(board & 0o7, 0o7, "not active, any attention, attention: {board:o}");
    assert_eq!(model & 0o7, 0o7, "and the same three on the model: {model:o}");
}

/// The cold boot's own copy buffer, from `sys/ucadr/uc-cold-disk.lisp`.
/// `DISK-COPY-SECTION` copies a band in chunks, each chunk a
/// `COLD-DISK-READ` and then a `COLD-DISK-WRITE` through
/// `START-DISK-N-PAGES`: `COPY-BUFFER-CCW-ORIGIN` 40000 holds the command
/// list, `COPY-BUFFER-CCW-BLOCK-LENGTH` 1000 is the most CCWs it can hold
/// --- two pages of them --- and `COPY-BUFFER-PAGE-ORIGIN` 102 is the
/// first page of the data buffer, 41000. **So a full list ends at 40777
/// and the word immediately past it is the first word the transfer
/// touches**, which is the adjacency issue 88's parked machine was read
/// in: its command list pointer walked to 41000 and kept going.
const COPY_BUFFER_CCW_ORIGIN: u32 = 0o40000;
const COPY_BUFFER_CCW_BLOCK_LENGTH: u32 = 0o1000;
const COPY_BUFFER: u32 = 0o41000;

/// A command list of `pages` CCWs ending where the cold boot's does, so
/// that the word past it is [`COPY_BUFFER`].
fn cold_clp(pages: usize) -> u32 {
    COPY_BUFFER - pages as u32
}

/// The band's own command word: `START-DISK-OP-1` sets bit 11, the done
/// interrupt enable, over whatever `A-DISK-COMMAND` holds, and the boot
/// PROM's working path never does.
const DONE_INTR_ENB: u32 = 1 << 11;

/// The three pages a chained write sends, the first with its first word's
/// bit 0 **set** so that it can also serve as a live word past the list.
fn pages_to_write(n: usize) -> Vec<[u32; muir::disk_unit::BLOCK_WORDS]> {
    let mut out: Vec<[u32; muir::disk_unit::BLOCK_WORDS]> =
        (0..n).map(|k| words(200 + k as u32)).collect();
    out[0][0] |= 1;
    out
}

/// **The one bit that makes a write a write, pin by pin.** Bit 3 of the
/// command register is the board's `CMD.FROM.MEMORY`, and it is the input
/// the sequencer's `DONE` gate and the channel's request gate both take.
///
/// The register is the 74LS175 at DCCMD 0C21 --- `-XINIT` on pin 1 clears
/// it, `-LOAD CMD` on pin 9 clocks it, and its four D inputs are `XBI3`,
/// `XBI2`, `XBI1` and `XBI0` on pins 4, 5, 12 and 13. `XBI3` comes out as
/// `CMD.FROM.MEMORY` on pin 2 and `-CMD.FROM.MEMORY` on pin 3, and `XBI1`
/// as `CMD1` and `-CMD1` on pins 10 and 11.
///
/// **MIT's wire list gives that net a second name and it settles what the
/// bit means**: `dc.wlr` prints `-CMD.FROM.MEMORY` and `CMD.TO.MEMORY`
/// over one pin list, off `C21-03(05)`, the `-1Q`. So the wire says which
/// way the data goes, and `sys/cold/qcom.lisp` sets bit 3 on exactly the
/// commands whose data comes out of memory --- `%DISK-COMMAND-WRITE 11`,
/// `%DISK-COMMAND-WRITE-ALL 13`, `%DISK-COMMAND-READ-COMPARE 10` --- and
/// leaves it clear on `%DISK-COMMAND-READ 0` and `%DISK-COMMAND-READ-ALL
/// 2`. Drawing and wire list agree here; there is nothing to choose
/// between.
#[test]
fn bit_3_of_the_command_is_cmd_from_memory() {
    let n = cadrdc();
    let is = |page: &str, reference: &str, pin: u8, name: &str| {
        assert_eq!(
            pin_net(&n, page, reference, pin),
            net_id(&n, name),
            "{page} {reference} pin {pin} is not {name}"
        );
    };

    // The command register.
    for (pin, name) in [
        (1, "-XINIT"),
        (9, "-LOAD CMD"),
        (4, "XBI3"),
        (2, "CMD.FROM.MEMORY"),
        (3, "-CMD.FROM.MEMORY"),
        (5, "XBI2"),
        (7, "CMD2"),
        (12, "XBI1"),
        (10, "CMD1"),
        (11, "-CMD1"),
        (13, "XBI0"),
        (15, "CMD0"),
    ] {
        is("DCCMD", "0C21", pin, name);
    }

    // The channel's request, the 9S42 at DCCHAN 0D20: output on pin 7,
    // inputs 1 to 6, so `(CLK MWD4 and -CMD.FROM.MEMORY) or (-MRD FULL and
    // CMD.FROM.MEMORY and MBUSY and HI4)`. A read is clocked by the disk
    // and a write by the memory-read register having room.
    for (pin, name) in [
        (7, "CHAN.RQ"),
        (1, "CLK MWD4"),
        (2, "-CMD.FROM.MEMORY"),
        (3, "-MRD FULL"),
        (4, "CMD.FROM.MEMORY"),
        (5, "MBUSY"),
        (6, "HI4"),
    ] {
        is("DCCHAN", "0D20", pin, name);
    }

    // And the same input on the `DONE` gate, the 9S42 at DCUC 0D20.
    for (pin, name) in [
        (9, "DONE"),
        (15, "-MBUSY"),
        (14, "DONE TEST"),
        (13, "LAST CCW"),
        (12, "-CMD.FROM.MEMORY"),
        (11, "-CMD1"),
        (10, "DONE TEST"),
    ] {
        is("DCUC", "0D20", pin, name);
    }
}

/// **A chained write stops at the last CCW, with the cold boot's own
/// geometry and a live word past the list.** Issue 88's parked machine was
/// in the write half of `DISK-COPY-SECTION`, not a read: `CMD/3` read 1,
/// which is Write by `newdsk.31`'s sector list, and `UPC/6` 63 is `162` or
/// `163`, "Write out the data bytes". The board's write had been benched
/// one page at a time --- [`a_write_with_a_drive_puts_the_page_on_the_pack`]
/// --- and never chained.
///
/// This is one. Three CCWs at `40775`, `40776`, `40777` --- the More flag
/// set on the first two --- with the pages at `41000`, `41400` and `42000`,
/// so the word immediately past the list **is** the transfer's own first
/// word, and it has bit 0 set. That is the exact shape the cold boot hands
/// the board, and the exact trap the read half was tested against.
///
/// **The board stops.** Three CCW fetches, the command list pointer
/// `40775 40776 40777` off `XBAO/22` at each `CCW CLK`, `-LAST CCW` high,
/// high, low, three pages read out of memory, three blocks on the pack
/// equal to them, not-active with no error. `41000` is read once, as page
/// data, and never as a CCW.
///
/// The controls: the same list away from the buffer stops the same way,
/// and so does the band's own command word --- `%DISK-COMMAND-WRITE 11`
/// with `START-DISK-OP-1`'s bit 11 on top, which the boot PROM's working
/// path never sets and which the issue kept in view as a difference.
#[test]
fn a_chained_write_stops_at_the_last_ccw() {
    let n = cadrdc();
    let list = [COPY_BUFFER, COPY_BUFFER + 0o400, COPY_BUFFER + 0o1000];
    let clp = cold_clp(list.len());
    assert_eq!(clp, 0o40775, "the list ends at 40777, where the cold boot's does");

    let out = pages_to_write(list.len());
    let (mut b, mut p, _) = chain_bench(&n, 0);
    for (page, w) in list.iter().zip(&out) {
        let at = *page as usize;
        p.memory.as_mut().unwrap().words[at..at + 256].copy_from_slice(w);
    }
    // The word past the list is the first page's own first word, put back
    // as it was so the page is undisturbed.
    let c = chained_write(&mut p, &mut b, clp, &list, out[0][0]);
    assert_eq!(out[0][0] & 1, 1, "and it carries the More flag");
    assert!(!c.stuck, "the write never stopped: CLP {}", octal(&c.clp));
    assert_eq!(c.clp, [clp, clp + 1, clp + 2], "XBAO/22 at each CCW CLK");
    assert_eq!(c.more(), [true, true, false], "-LAST CCW at each fetch");
    assert_eq!(c.status & 1, 1, "not active: {:o}", c.status);
    assert_eq!(c.status & ERRORS, 0, "no error: {:o}", c.status);

    // On a write every cycle is a read, so the CCW fetches are told from
    // the page reads by their addresses --- and `41000` appears once,
    // among the page's own 256 words.
    let at = |a: u32| c.transfers.iter().filter(|t| t.addr == a).count();
    assert_eq!(at(clp), 1, "the first CCW, once");
    assert_eq!(at(clp + 1), 1);
    assert_eq!(at(clp + 2), 1);
    assert_eq!(at(COPY_BUFFER), 1, "the word past the list, once: as page data");
    assert!(c.transfers.iter().all(|t| t.wrote.is_none()), "a write only reads memory");
    assert_eq!(c.transfers.len(), 3 + 3 * 256, "three fetches and three pages, and nothing else");

    // And the pack has what memory had.
    for (k, w) in out.iter().enumerate() {
        let block = FIRST_BLOCK + k as u32;
        let drive = &mut p.cable.as_mut().unwrap().drive;
        assert_eq!(drive.bad_writes, 0, "what was written parsed as the format");
        assert_eq!(drive.unit.block_at(0, 0, block), Some(*w), "block {block} on the pack");
    }
    eprintln!(
        "cold geometry: CLP {}, -LAST CCW {:?}, {} cycles, status {:o}",
        octal(&c.clp),
        c.more(),
        c.transfers.len(),
        c.status
    );

    // Away from the buffer, and with the band's own command word.
    for (what, cmd, clp, list) in [
        ("a list away from the buffer", DISK_WRITE_COMMAND, CLP, [PAGE, PAGE2, 0o40000]),
        ("the band's own command word", DISK_WRITE_COMMAND | DONE_INTR_ENB, clp, list),
    ] {
        let (mut b, mut p, _) = chain_bench(&n, 0);
        for (page, w) in list.iter().zip(&out) {
            let at = *page as usize;
            p.memory.as_mut().unwrap().words[at..at + 256].copy_from_slice(w);
        }
        command_list(&mut p, clp, &list, out[0][0]);
        let c = chained(&mut p, &mut b, cmd, clp, list.len());
        assert!(!c.stuck, "{what}: the write never stopped");
        assert_eq!(c.clp, [clp, clp + 1, clp + 2], "{what}: the command list pointer");
        assert_eq!(c.more(), [true, true, false], "{what}: -LAST CCW at each fetch");
        assert_eq!(c.status & 1, 1, "{what}: not active: {:o}", c.status);
        assert_eq!(c.status & ERRORS, 0, "{what}: no error: {:o}", c.status);
        for (k, w) in out.iter().enumerate() {
            let block = FIRST_BLOCK + k as u32;
            let drive = &mut p.cable.as_mut().unwrap().drive;
            assert_eq!(drive.unit.block_at(0, 0, block), Some(*w), "{what}: block {block}");
        }
    }
}

/// **On a write the channel runs ahead of the disk, and `DONE` at `174`
/// has one term where a read has two.**
///
/// The two differences are one bit of the command register. `DISK-WRITE-
/// COMMAND` is `11`, and bit 3 is the board's `CMD.FROM.MEMORY` --- MIT's
/// wire list gives the same wire a second name, `CMD.TO.MEMORY`, off
/// `C21-03(05)`, the `-1Q` of the 74LS175 at DCCMD 0C21 --- set on exactly
/// the commands whose data comes out of memory: `%DISK-COMMAND-WRITE 11`,
/// `%DISK-COMMAND-WRITE-ALL 13` and `%DISK-COMMAND-READ-COMPARE 10`.
///
/// **What it does to the channel's clock.** `CHAN.RQ` is the 9S42 at
/// DCCHAN 0D20, pin 7 out of pins 1 to 6:
///
///     CHAN.RQ = (CLK MWD4 and -CMD.FROM.MEMORY)
///            or (-MRD FULL and CMD.FROM.MEMORY and MBUSY and HI4)
///
/// On a read the first term stands and the channel asks for the bus only
/// when the disk has given it a word. On a write the second stands and it
/// asks whenever the memory-read register has room --- **nothing to wait
/// for**. Measured: the first CCW is fetched 230 ns after START, where a
/// read's is a seek and a sector away --- 2.00 ms, which
/// [`ccw_clk_latches_the_fetch_it_is_in`] prints --- and for every row of
/// the run `CHAN.RQ` is up exactly when `MRD FULL` is down and `MBUSY`
/// up. The
/// write buffer is what throttles it: the 67401s at DCWBUF 0F01 and 0F02
/// fill and `WFIRA` goes down.
///
/// **What it does to `DONE`.** `-CMD.FROM.MEMORY` is low for the whole
/// command, so the four-input term of the 9S42 at DCUC 0D20 is dead and
/// `DONE` at `174` is `-MBUSY and DONE TEST` alone. That matters here in a
/// way it does not on a read: because the channel runs ahead, `LAST CCW`
/// comes up at the last CCW's fetch **before** the block before it reaches
/// `174` --- measured 3.77 ms against 3.83 ms, the opposite order from a
/// read --- so a live term would end the write a page early. It is dead,
/// and all three pages go to the pack.
#[test]
fn the_write_channel_runs_ahead_of_the_disk() {
    let n = cadrdc();
    let list = [COPY_BUFFER, COPY_BUFFER + 0o400, COPY_BUFFER + 0o1000];
    let clp = cold_clp(list.len());
    let out = pages_to_write(list.len());
    let (mut b, mut p, _) = chain_bench(&n, 0);
    for (page, w) in list.iter().zip(&out) {
        let at = *page as usize;
        p.memory.as_mut().unwrap().words[at..at + 256].copy_from_slice(w);
    }
    let c = chained_write(&mut p, &mut b, clp, &list, out[0][0]);
    assert!(!c.stuck, "the write never stopped");

    // The command register, over the whole run.
    assert!(
        c.seen.iter().all(|(_, s, _)| s[NOT_CMD_FROM_MEMORY] == Level::Low),
        "-CMD.FROM.MEMORY is low for a Write, so the four-input term is dead"
    );
    assert!(
        c.seen.iter().all(|(_, s, _)| s[NOT_CMD1] == Level::High),
        "-CMD1 is high, as it is on a Read: it is not what kills the term here"
    );

    // The channel's clock: `CHAN.RQ` is the FIFO's, not the disk's.
    assert!(
        c.seen.iter().all(|(_, s, _)| {
            (s[CHAN_RQ] == Level::High) == (s[MRD_FULL] == Level::Low && s[NOT_MBUSY] == Level::Low)
        }),
        "CHAN.RQ is -MRD FULL and MBUSY, the second term of the 9S42, and nothing else"
    );
    assert!(
        c.seen.iter().any(|(_, s, _)| s[WFIRA] == Level::Low),
        "the write buffer fills: WFIRA goes down"
    );
    assert!(c.seen.iter().any(|(_, s, _)| s[WFORA] == Level::High), "and has bytes for the disk");
    let first = c.transfers.first().expect("a CCW fetch");
    assert_eq!(first.addr, clp, "the first cycle is the first CCW");
    assert!(
        first.at - c.t0 < 1_000,
        "and it is a microsecond after START, not a seek away: {} ns",
        first.at - c.t0
    );

    // `DONE` at `174`, one visit a block.
    let visits = c.entered(0o100, 0o174);
    assert_eq!(visits.len(), list.len(), "174 once a block");
    assert_eq!(c.rises(DONE_TEST), visits, "DONE TEST comes up at 174 and nowhere else");
    let done: Vec<Vec<Level>> = visits.iter().map(|&i| c.during(i, DONE)).collect();
    assert_eq!(done[0], [Level::Low], "the first block's 174: not done");
    assert_eq!(done[1], [Level::Low], "the second block's 174: not done");
    assert_eq!(done[2], [Level::High], "the last block's 174: done");
    assert_eq!(c.rises(DONE), [visits[2]], "DONE rises once, as 174 is entered");

    // And `LAST CCW` was already up at the second block's `174`, which on a
    // read it is not.
    let up = c.rises(LAST_CCW);
    assert_eq!(up.len(), 1, "LAST CCW comes up once");
    let up = up[0];
    assert_eq!(c.seen[up].1[CCW_CLK], Level::High, "at a CCW CLK");
    assert!(
        c.seen[up].0 < c.seen[visits[1]].0,
        "LAST CCW at {} ns, the second block's 174 at {} ns",
        c.seen[up].0 - c.t0,
        c.seen[visits[1]].0 - c.t0
    );

    // The channel is done well before the disk is.
    let mbusy = c.rises(NOT_MBUSY);
    assert_eq!(mbusy.len(), 1, "-MBUSY rises once");
    let mbusy = mbusy[0];
    assert_eq!(c.seen[mbusy].1[NOT_LAST_CCW], Level::Low, "taking the last CCW's More flag");
    assert_eq!(c.seen[mbusy].1[END_PAGE_CLK], Level::High, "on END PAGE CLK's rise");
    assert!(c.seen[mbusy].0 < c.seen[visits[2]].0, "and before the 174 that reads it");
    eprintln!(
        "first CCW +{} ns; LAST CCW +{} ns, second 174 +{} ns; -MBUSY +{} ns, DONE +{} ns",
        first.at - c.t0,
        c.seen[up].0 - c.t0,
        c.seen[visits[1]].0 - c.t0,
        c.seen[mbusy].0 - c.t0,
        c.seen[visits[2]].0 - c.t0
    );
}

/// **The cold boot's whole chunk: 512 CCWs at 40000, the buffer from
/// 41000, written.** `DISK-COPY-SECTION` hands the board at most
/// `COPY-BUFFER-CCW-BLOCK-LENGTH` = 1000 octal pages at a time, and issue
/// 88's parked machine was in one of those. This is that command, entire.
///
/// The board makes 512 CCW fetches and 512 pages of reads --- 131,584
/// cycles, and nothing else --- walks the command list pointer from `40000`
/// to `40777` and stops there, with the More flag set on the first 511 and
/// clear on the last. It does not read `41000` as a CCW.
///
/// It is `#[ignore]`d for its length: half a second of board is two minutes
/// of wall clock at five nanoseconds a step.
#[test]
#[ignore = "512 pages: half a second of board, two minutes of wall clock; run with --ignored"]
fn the_cold_boots_own_chunk_stops_at_the_last_ccw() {
    let n = cadrdc();
    let pages = COPY_BUFFER_CCW_BLOCK_LENGTH as usize;
    let list: Vec<u32> = (0..pages as u32).map(|k| COPY_BUFFER + k * 0o400).collect();
    let clp = COPY_BUFFER_CCW_ORIGIN;
    assert_eq!(clp + pages as u32, COPY_BUFFER, "a full list ends where the buffer begins");

    let (mut b, mut p, _) = chain_bench(&n, 0);
    // The pages the CCWs name run to 441000, so the memory has to reach it.
    p.with_memory(&b, 1 << 19);
    let out = pages_to_write(1);
    let at = COPY_BUFFER as usize;
    p.memory.as_mut().unwrap().words[at..at + 256].copy_from_slice(&out[0]);
    let c = chained_write(&mut p, &mut b, clp, &list, out[0][0]);
    assert!(!c.stuck, "the write never stopped: last CLP {:o}", c.clp.last().copied().unwrap_or(0));
    assert_eq!(c.clp.len(), pages, "one CCW fetch a page");
    assert_eq!(c.clp.first(), Some(&clp), "from the origin");
    assert_eq!(c.clp.last(), Some(&(COPY_BUFFER - 1)), "to 40777, and no further");
    assert!(c.clp.windows(2).all(|w| w[1] == w[0] + 1), "one word at a time");
    let more = c.more();
    assert_eq!(more.len(), pages);
    assert!(more[..pages - 1].iter().all(|&m| m), "More on all but the last");
    assert!(!more[pages - 1], "and clear on the last");
    assert_eq!(c.transfers.len(), pages * 257, "512 fetches and 512 pages, and nothing else");
    assert_eq!(c.transfers.iter().filter(|t| t.addr == COPY_BUFFER).count(), 1, "41000 once");
    assert_eq!(c.status & 1, 1, "not active: {:o}", c.status);
    assert_eq!(c.status & ERRORS, 0, "no error: {:o}", c.status);
    eprintln!(
        "512 CCWs from {clp:o} to {:o}: {} cycles, status {:o}",
        c.clp.last().unwrap(),
        c.transfers.len(),
        c.status
    );
}
