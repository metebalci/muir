// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The LISPM TV as a netlist: `data/LISPMTV.netlist`, the 25 `cadrtv`
//! pages `lmtv4b.fil` lists, through `tools/lispmtv-netlist.sh`.
//!
//! This is the four- and eight-bit display that replaced the SIMPLE TV in
//! December 1980, and the board `--tv-board lispm-tv` puts on the Xbus.
//! Unlike the SIMPLE TV it has MIT's own wire list, `cadrtv/lmtv4b.wlr` of
//! 7 December 1980, and the section census `lmtv4b.wls` beside it, both
//! made by MIT's tooling from these drawings the same day; the netlist is
//! held to both, as the memory board and the disk controller are.

use std::collections::{BTreeMap, BTreeSet};

use muir::netlist::{self, Netlist};
use muir::wirelist;

mod support;
use support::mit_text;

const LISPMTV: &str = include_str!("../data/LISPMTV.netlist");

fn lispmtv() -> Netlist {
    netlist::parse(LISPMTV).unwrap()
}

/// The shape of the board, pinned the way the other netlists are. Two of
/// the 25 pages carry no parts: `gen4b`, the customising jumpers, and
/// `xbus`, the backplane.
///
/// **The RAM count is the frame buffer again.** RAMA to RAMD carry sixteen
/// 2118s each, 16K by 1, and 64 by 16,384 bits is the same 32K words of 32
/// bits the SIMPLE TV holds --- the 2118 being the single-supply successor
/// of the 4116 in the same pins.
#[test]
fn parses_to_the_expected_shape() {
    let n = lispmtv();
    assert_eq!(n.pages.len(), 25, "{:?}", n.pages);
    assert_eq!(n.parts.len(), 265);
    assert_eq!(n.populated_pages().len(), 23);
    let rams = n.parts.iter().filter(|p| p.kind == "2118").count();
    assert_eq!(rams, 64, "four rows of sixteen 16K DRAMs");
    assert_eq!(rams * 16_384, muir::simpletv::BUFFER_WORDS as usize * 32);
}

/// **Every page is a LISPM TV page.** The same filename collision as the
/// SIMPLE TV's, from the other side: six of these names also exist in
/// `simple-tv/`, and the title block is what says which board a sheet is.
#[test]
fn every_page_is_a_lispm_tv_page() {
    let titles: Vec<&str> =
        LISPMTV.lines().filter_map(|l| l.strip_prefix("# title 1: ")).map(str::trim).collect();
    assert_eq!(titles.len(), 25, "one title block a page");
    assert!(titles.iter().all(|&t| t == "LISPM TV"), "a page of another board: {titles:?}");
}

/// Every part on the board is identified, and everything that computes
/// anything has a behaviour, the delay line and the oscillator can apart,
/// which `src/chip.rs` runs.
#[test]
fn every_part_is_identified() {
    let k = support::kinds(&lispmtv());
    assert!(k.unknown.is_empty(), "no pinout for {:?}", k.unknown);
    assert_eq!(k.silent, ["TD100", "TTLOSC"], "parts with no behaviour");
}

/// Two totem-pole outputs on one net is an electrical fault, so a wrongly
/// claimed output pin surfaces here.
#[test]
fn no_net_has_two_push_pull_drivers() {
    support::no_net_has_two_push_pull_drivers(&lispmtv());
}

/// **Every section MIT counted is in the netlist.** `lmtv4b.wls` is MIT's
/// own summary of the board by DIP type, read by `support::body_census`.
#[test]
fn the_census_matches_mits_own() {
    let text = mit_text(&["cadrtv", "lmtv4b.wls"]);

    let census = support::body_census(&text);
    assert!(census.len() > 30, "parsed {} bodies out of lmtv4b.wls", census.len());

    let n = lispmtv();
    let mut ours: BTreeMap<String, usize> = BTreeMap::new();
    for p in &n.parts {
        *ours.entry(p.kind.clone()).or_default() += 1;
    }
    let mut wrong = Vec::new();
    for (body, want) in &census {
        let got = ours.get(body).copied().unwrap_or(0);
        match body.as_str() {
            "BYPASS" => assert_eq!(got, 0, "bypass capacitors are not parts"),
            _ if got != *want => wrong.push(format!("{body}: MIT {want}, ours {got}")),
            _ => {}
        }
    }
    for body in ours.keys() {
        if !census.contains_key(body) {
            wrong.push(format!("{body}: not in MIT's census at all"));
        }
    }
    assert!(wrong.is_empty(), "sections disagree with lmtv4b.wls: {wrong:?}");
    eprintln!(
        "lmtv4b.wls counts {} sections in {} bodies",
        census.values().sum::<usize>(),
        census.len()
    );
}

/// **MIT's wire list for the board agrees with the netlist.** `lmtv4b.wlr`
/// (7 December 1980) is the list the board was wrapped from, pin by pin;
/// every wire on the 25 pages must be one net, and no net two wires.
#[test]
fn matches_mits_wire_list() {
    let n = netlist::parse_wired(LISPMTV).unwrap();
    let signals = support::wire_list(&n, &["cadrtv", "lmtv4b.wlr"]);
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

/// **The board comes up on the bus, as far as it is known to.** Powered,
/// reset and left for two microseconds as the SIMPLE TV is --- the same
/// address straps, the same two PROM images, its own 64 MHz can --- no net
/// is unknown, and the mode register at `17377760` takes a write and reads
/// it back, and a frame-buffer word written reads back. This test pins
/// it.
#[test]
fn the_board_comes_up_on_the_bus() {
    use muir::part::Level;
    use muir::simpletv::{BUFFER, CONTROL, mode};
    use muir::xbus::XbusMaster;

    let n = lispmtv();
    let mut b = XbusMaster::new(&n, 0);
    let wired: BTreeSet<netlist::NetId> =
        n.parts.iter().flat_map(|p| p.pins.iter().map(|&(_, net)| net)).collect();
    let unknown: Vec<&str> =
        wired.iter().filter(|&&id| b.chip.net(id) == Level::X).map(|&id| n.net(id)).collect();
    assert!(unknown.is_empty(), "nets at unknown with the bus driven: {unknown:?}");

    let (took, _) = b.cycle(CONTROL, Some(mode::BOW));
    assert!(took < 1_000, "the mode register answered a write in {took} ns");
    let (took, word) = b.cycle(CONTROL, None);
    assert!(took < 1_000, "and a read in {took} ns");
    assert_eq!(word & mode::WRITABLE, mode::BOW, "and gave the bit back");

    // The frame buffer holds a word. Its 2118s are 4116s in a single-supply
    // package and stand in the part table as an alias of the 4116; a part
    // that answers to the 4116's pins and cycle must have its cells too.
    // `memory_words` once resolved the suffix but not the alias, so the
    // 2118 had none: every write went into nothing, every read gave `X`,
    // and the read register latched that as ones --- `0xffffffff` back for
    // any word.
    let (took_w, _) = b.cycle(BUFFER + 5, Some(0x1234_5678));
    let (took_r, word) = b.cycle(BUFFER + 5, None);
    eprintln!("frame buffer: write acknowledged in {took_w} ns, read in {took_r} ns");
    assert_eq!(word, 0x1234_5678, "the word written at 17000005 reads back");
}
