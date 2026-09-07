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

use muir::disk_controller::REGS;
use muir::disk_unit::{Geometry, OnCable, REVOLUTION_NS, Trident, Unit};
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
        let t0 = b.now;
        self.mine = true;
        b.request(addr, write);
        self.sample(b);
        while !b.acked() {
            assert!(b.now < t0 + 40_000, "the board never acknowledged");
            self.run(b, b.now + 5);
        }
        let word = b.word();
        self.run(b, b.now + XbusMaster::RELEASE_NS);
        b.release();
        for k in 0..22 {
            let net = b.net(&format!("-XADDR{k}"));
            b.chip.pull_up(net);
        }
        self.mine = false;
        self.run(b, b.now + 600);
        word
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
/// which is what `LOOP/BLOCK CTR EQ BLOCK` waits on. Read after every
/// pulse for a turn and a quarter.
#[test]
fn the_block_counter_follows_the_drives_sector_pulses() {
    let n = cadrdc();
    let mut b = controller(&n);
    let mut p = Probe::new(&b);
    let t0 = b.now;
    p.plug(&mut b, &n, quick_drive(t0));
    assert_eq!(p.drive().turn(t0), (0, 0), "an index pulse just beginning");
    for k in 0..21u32 {
        // 100 us into sector k: the pulse over, the counter settled.
        let at = t0 + k as u64 * REVOLUTION_NS / 17 + 100_000;
        p.run(&mut b, at);
        let status = p.cycle(&mut b, REGS, None);
        assert_eq!(p.drive().turn(b.now).0, k % 17, "the drive at {}", b.now);
        assert_eq!(status >> 24, k % 17, "the block counter at {}: status {status:o}", b.now);
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
