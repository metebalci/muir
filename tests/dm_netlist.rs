// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The disk multiplexor as a netlist: `data/DM.netlist`, MIT's `cadrdc`
//! `dm*` drawings through `tools/dm-netlist.sh`.
//!
//! The board is the DISK MULTIPLEXOR, which sits between one disk
//! controller and up to eight Trident drives. **Nothing runs it yet.**
//! This file holds the extraction; the cable between the board and the
//! controller's edge is issue #1, blocked on #37, and
//! `the_cable_signals_reach_one_pin_and_stop` is what marks the seam.
//!
//! **The second source here is MIT's specification, not MIT's wiring**, as
//! it is for the SIMPLE TV. There is no `dm.wlr` on the tapes --- `dc.wlr`
//! and `mk.wlr` are, for the two boards beside this one --- and `dm.wls`
//! is a page list with no section census in it, naming ten of the eleven
//! pages; discrepancy 75. What stands behind the extraction instead is
//! `cadrdc/dm.stf`, MIT's stuffing list, which places every body on the
//! board by location, and `cadrdc/dm.txt`, its page list.
//!
//! So this file cannot make the pin-for-pin check a board with a wire list
//! gets, and says so rather than making a weaker one quietly.

use std::collections::{BTreeMap, BTreeSet};

use muir::netlist::{self, Netlist};

mod support;
use support::mit_text;

const DM: &str = include_str!("../data/DM.netlist");

fn dm() -> Netlist {
    netlist::parse(DM).unwrap()
}

/// MIT's page list read back.  `dm.txt` is a SUDS plot command file: each
/// drawing is a line `IDMBKC0`, and every line of it begins with an ASCII
/// STX that has to come off first.
fn pages_mit_names() -> Vec<String> {
    mit_text(&["cadrdc", "dm.txt"])
        .lines()
        .map(|l| l.trim_matches(|c: char| c.is_control() || c.is_whitespace()))
        .filter_map(|l| l.strip_prefix('I'))
        .filter(|p| p.starts_with("DM"))
        .map(str::to_string)
        .collect()
}

/// Every body `dm.stf` places, as `(location, body, page)`.
///
/// The columns are `PART NUMBER / DIPTYPE / LOC / BODY / FILE / POS`, and
/// the list is written the way a person reads it: a row with a `LOC(  )`
/// in it opens a slot, rows under it with three fields are further bodies
/// in that same slot, and a part number or a diptype is left blank when it
/// is the same as the row above.  A slot holding two bodies --- a package
/// and the bypass capacitor beside it --- writes the second `A07@02`.
fn stuffing() -> Vec<(String, String, String)> {
    let text = mit_text(&["cadrdc", "dm.stf"]);
    let mut out = Vec::new();
    let mut at: Option<String> = None;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let loc = f.iter().position(|w| {
            w.ends_with('(')
                && w[..w.len() - 1].len() >= 3
                && w.starts_with(|c: char| c.is_ascii_uppercase())
        });
        match loc {
            Some(k) => {
                at = Some(f[k][..f[k].len() - 1].to_string());
                let rest: Vec<&str> = f[k + 1..].iter().copied().filter(|w| *w != ")").collect();
                if let (Some(slot), [body, page, ..]) = (&at, &rest[..]) {
                    out.push((slot.clone(), body.to_string(), page.to_string()));
                }
            }
            // A further body in the slot above: body, file, position.
            None if f.len() == 3 && f[1].starts_with("DM") => {
                if let Some(slot) = &at {
                    out.push((slot.clone(), f[0].to_string(), f[1].to_string()));
                }
            }
            None => {}
        }
    }
    out
}

/// Every body the netlist has, as `(reference, kind, page)`, with the
/// board's `0` prefix taken off the reference so that it reads as MIT's
/// slot does.
fn extracted() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut page = String::new();
    for line in DM.lines() {
        if let Some(p) = line.strip_prefix("page ") {
            page = p.trim().to_string();
        }
        if let Some(rest) = line.strip_prefix("part ")
            && let Some((reference, kind)) = rest.split_once(',')
        {
            let slot = reference.trim().strip_prefix('0').unwrap_or(reference.trim());
            out.push((slot.to_string(), kind.trim().to_string(), page.clone()));
        }
    }
    out
}

/// **The netlist is the eleven pages MIT's own page list names, in its
/// order.** `dm.txt` names them; `dm.fil` and `dm.stf` name the same
/// eleven. `dm.wls` names ten, leaving `DMSEQ` out, and is not followed:
/// discrepancy 75.
#[test]
fn the_pages_are_the_eleven_mit_names() {
    let n = dm();
    let mit = pages_mit_names();
    assert_eq!(mit.len(), 11, "dm.txt names eleven pages: {mit:?}");
    assert_eq!(n.pages, mit, "the netlist's pages, in dm.txt's order");
    // And every one is this board's: `cadrdc/` holds three boards.
    let titles: BTreeSet<&str> =
        DM.lines().filter_map(|l| l.strip_prefix("# title 1: ")).map(str::trim).collect();
    assert_eq!(titles, BTreeSet::from(["DISK MULTIPLEXOR"]), "a page of another board");
}

/// **Every body MIT's stuffing list places is in the netlist, in its slot
/// and on its page --- but for four bypass capacitors.** This stands in
/// for the section census a board with a `.wls` gets.
///
/// The four are `BYPASS` bodies at `B07@01`, `B17@01`, `D12@01` and
/// `E04@01`, all on `DMCAPS`. `dm.stf` places them; the CAPACITORS drawing
/// does not draw them, and the twelve `CAP1` bodies it does draw are all
/// in the netlist. A bypass capacitor decouples a supply pin and carries
/// no signal, so nothing this netlist is for depends on which file is
/// right --- but the two MIT files do disagree, and the disagreement is
/// named here rather than left as a count that does not add up.
/// Discrepancy 76.
#[test]
fn every_body_mit_stuffs_is_in_the_netlist() {
    let stf = stuffing();
    let netlist = extracted();
    assert!(stf.len() > 140, "parsed {} bodies out of dm.stf", stf.len());

    // A slot's second body is `A07@02` in the list and `0A07` in the
    // netlist, so the comparison is on the slot without the suffix.
    let plain = |s: &str| s.split('@').next().unwrap_or(s).to_string();
    let mut want: BTreeMap<(String, String, String), usize> = BTreeMap::new();
    for (slot, body, page) in &stf {
        *want.entry((plain(slot), body.clone(), page.clone())).or_default() += 1;
    }
    for (slot, kind, page) in &netlist {
        if let Some(c) = want.get_mut(&(slot.clone(), kind.clone(), page.clone())) {
            *c -= 1;
            if *c == 0 {
                want.remove(&(slot.clone(), kind.clone(), page.clone()));
            }
        }
    }
    let short: BTreeSet<String> = want.keys().map(|(s, b, p)| format!("{s} {b} on {p}")).collect();
    eprintln!("dm.stf places {} bodies; the netlist has {}", stf.len(), netlist.len());
    assert_eq!(
        short,
        BTreeSet::from([
            "B07 BYPASS on DMCAPS".to_string(),
            "B17 BYPASS on DMCAPS".to_string(),
            "D12 BYPASS on DMCAPS".to_string(),
            "E04 BYPASS on DMCAPS".to_string(),
        ]),
        "bodies dm.stf places and the drawings do not draw"
    );

    // And nothing the other way round: every body the drawings carry is
    // one MIT stuffed.
    let placed: BTreeSet<(String, String, String)> =
        stf.iter().map(|(s, b, p)| (plain(s), b.clone(), p.clone())).collect();
    let extra: BTreeSet<&(String, String, String)> =
        netlist.iter().filter(|k| !placed.contains(*k)).collect();
    assert!(extra.is_empty(), "bodies the drawings carry and dm.stf does not place: {extra:?}");
}

/// **No net has two push-pull drivers**, the electrical check every board
/// gets: a wrongly claimed output pin in `src/part.rs` surfaces here.
#[test]
fn no_net_has_two_push_pull_drivers() {
    support::no_net_has_two_push_pull_drivers(&dm());
}

/// **The signals that leave the board reach one pin and stop, and that is
/// exactly the shape of what is missing.**
///
/// `dmcabl`, the CABLES page, and `dmedge`, the EDGE CONNECTIONS page,
/// yield no bodies and no points --- as `dcedge` and `xbus` do on the
/// controller, where the same connector knowledge is committed by hand in
/// `data/cables.txt` and `data/busint-connectors.txt` and checked against
/// `dc.wlr`. This board has no wire list to check such a file against, so
/// it has no such file: issue #37.
///
/// What is left dangling says what the board is for. `TRIDENT.0.SEQUENCE/`
/// through `TRIDENT.7.SEQUENCE/` are the eight drives; `ANY ATTENTION`,
/// `MULTIPLE SELECT`, `NO SELECT` and `-ANY SELECT` are the four the
/// controller's one-board jumpers stand in for today, `cadrdc/disk.hand`
/// and `dc.eco`, "Jumpers for 1-board version (use red wire)";
/// `READ DATA` and `WRITE GATE` are the drive's own; and `XBI28` to
/// `XBI30` and `XBUS.POWER.OK` are the backplane's.
///
/// Held exactly, so that an extraction which gains any of them shows up as
/// a failure rather than passing unnoticed.
#[test]
fn the_cable_signals_reach_one_pin_and_stop() {
    let n = dm();
    let mut pins: BTreeMap<&str, usize> = BTreeMap::new();
    for p in &n.parts {
        for &(_, net) in &p.pins {
            *pins.entry(n.net(net)).or_default() += 1;
        }
    }
    let alone: BTreeSet<&str> = pins
        .iter()
        .filter(|&(name, &c)| c == 1 && !support::is_power(name) && !name.starts_with('@'))
        .map(|(&name, _)| name)
        .collect();
    let mut want = BTreeSet::from([
        "'-ANY SELECT'",
        "'ANY ATTENTION'",
        "'MULTIPLE SELECT'",
        "'NO SELECT'",
        "'READ DATA'",
        "'WRITE GATE'",
        "BLOCK.CLK^",
        "DISK.CLK^",
        "XBI28",
        "XBI29",
        "XBI30",
        "XBUS.POWER.OK",
    ]);
    let drives: Vec<String> = (0..8).map(|k| format!("TRIDENT.{k}.SEQUENCE/")).collect();
    want.extend(drives.iter().map(String::as_str));
    assert_eq!(alone, want, "the signals with nothing on the other end");
}

/// The `TRIDENT.<unit>.<signal>` nets a netlist carries, by unit.
fn per_unit(n: &Netlist) -> BTreeMap<u8, BTreeSet<String>> {
    let mut by: BTreeMap<u8, BTreeSet<String>> = BTreeMap::new();
    for net in 0..n.nets.len() {
        let name = n.net(net as u32);
        if let Some(rest) = name.strip_prefix("TRIDENT.")
            && let Some((unit, signal)) = rest.split_once('.')
            && let Ok(unit) = unit.parse::<u8>()
        {
            by.entry(unit).or_default().insert(signal.to_string());
        }
    }
    by
}

/// The `TRIDENT.*` nets that carry no unit number: the lines every drive
/// shares.
fn shared(n: &Netlist) -> BTreeSet<String> {
    (0..n.nets.len())
        .filter_map(|net| n.net(net as u32).strip_prefix("TRIDENT.").map(str::to_string))
        .filter(|rest| !rest.starts_with(|c: char| c.is_ascii_digit()))
        .collect()
}

/// **The drive end is eight ports of exactly the signals the controller
/// puts on its own drive cable --- and none of the ones it shares.**
///
/// This is the check the drive end can have, and it is worth being precise
/// about what it is. There is no `dm.wlr`: `dm.stf` places bodies on the
/// board without saying what is wired to what, so nothing here can be held
/// pin for pin the way `data/CADRDC.netlist` is held to `cadrdc/dc.wlr`.
/// What *can* be held, and is, is the **signal set**, against a source
/// that is wire-listed --- the controller's own Trident cable.
///
/// The controller's `TRIDENT.` nets fall into two groups. Nine carry a
/// unit number, `TRIDENT.0.SELECT/` and its eight fellows, and eighteen do
/// not: the ten `TRIDENT.BUS<n>/`, the three tags, and `READY/`,
/// `ON.LINE/`, `READ.ONLY/`, `DEVICE.CHECK/` and `SEEK.INC/`. The
/// multiplexor carries the first nine for every one of eight units and not
/// one of the eighteen, which is what a multiplexor is: the per-drive
/// lines fan out through it and the shared ones go straight past it to
/// every drive.
///
/// So a signal appearing on one board's drive cable and not the other's
/// fails here, in either direction, without either board's drawings being
/// taken on trust.
#[test]
fn the_drive_ports_carry_the_controllers_own_per_unit_signals() {
    let dm = dm();
    let dc = netlist::parse(include_str!("../data/CADRDC.netlist")).unwrap();
    let (dmp, dcp) = (per_unit(&dm), per_unit(&dc));

    assert_eq!(dmp.keys().copied().collect::<Vec<_>>(), (0..8).collect::<Vec<_>>(), "eight ports");
    assert_eq!(dcp.keys().copied().collect::<Vec<_>>(), vec![0], "the controller has one drive");

    let want = BTreeSet::from(
        [
            "ATTENTION/",
            "CLOCK.M",
            "CLOCK.P",
            "COMPSECIDX/",
            "DATA.M",
            "DATA.P",
            "SELECT/",
            "SELECTED/",
            "SEQUENCE/",
        ]
        .map(String::from),
    );
    assert_eq!(dcp[&0], want, "the controller's per-unit cable");
    for (unit, signals) in &dmp {
        assert_eq!(*signals, want, "the multiplexor's port {unit}");
    }

    // And the eighteen the drives share are the controller's alone.
    assert!(shared(&dm).is_empty(), "the multiplexor carries no shared line: {:?}", shared(&dm));
    assert_eq!(shared(&dc).len(), 18, "the controller's shared lines");
}

/// **The eight ports are two banks of four, which is how `dm.eco` sees
/// them too.** MIT's change order lists the eight drive connectors with
/// pins to decommit and remove before wire-wrapping, and the eight are
/// four patterns twice over: `J1` and `J5` alike, `J2` and `J6`, `J3` and
/// `J7`, `J4` and `J8`. The board is built the same way --- units 0 to 3
/// on one set of drivers, receivers and terminators, units 4 to 7 on
/// another, with no part shared between the halves.
///
/// The parts are the controller's own cable parts, which is the other half
/// of the corroboration: the 75107 and 75110 differential pairs, the 75452
/// drivers, `TRITERM` the cable terminator, and the 100 ohm `SIP100-8`
/// pull-ups. `src/part.rs` has each from its datasheet.
#[test]
fn the_drive_ports_are_two_banks_of_four() {
    let n = dm();
    let mut by_unit: BTreeMap<u8, BTreeSet<&str>> = BTreeMap::new();
    let mut kinds: BTreeSet<&str> = BTreeSet::new();
    for p in &n.parts {
        for &(_, net) in p.pins.iter() {
            if let Some(rest) = n.net(net).strip_prefix("TRIDENT.")
                && let Some((unit, _)) = rest.split_once('.')
                && let Ok(unit) = unit.parse::<u8>()
            {
                by_unit.entry(unit).or_default().insert(p.reference.as_str());
                kinds.insert(p.kind.as_str());
            }
        }
    }
    let low: BTreeSet<&str> = (0..4).flat_map(|u| by_unit[&u].iter().copied()).collect();
    let high: BTreeSet<&str> = (4..8).flat_map(|u| by_unit[&u].iter().copied()).collect();
    assert!(!low.is_empty() && !high.is_empty(), "both banks carry parts");
    let both: Vec<&str> = low.intersection(&high).copied().collect();
    assert_eq!(both, ["0B11"], "one part spans the banks: the COMPSECIDX/ pull-ups");
    let pack = n.parts.iter().find(|p| p.reference == "0B11").expect("0B11");
    assert_eq!(pack.kind, "SIP100-8", "and it is a pack of eight resistors");
    let pulled: BTreeSet<&str> = pack
        .pins
        .iter()
        .filter_map(|&(_, net)| n.net(net).strip_prefix("TRIDENT."))
        .filter_map(|rest| rest.split_once('.').map(|(_, sig)| sig))
        .collect();
    assert_eq!(
        pulled,
        BTreeSet::from(["ATTENTION/", "COMPSECIDX/"]),
        "pulling up lines from both banks"
    );
    assert_eq!(
        kinds,
        BTreeSet::from(["16DUMMY", "75107", "75110", "75452", "SIP100-8", "TRITERM"]),
        "the drive end is built of the controller's own cable parts"
    );
}
