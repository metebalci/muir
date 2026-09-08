// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The cable between the disk controller and the DISK MULTIPLEXOR:
//! `src/dm.rs`.
//!
//! What is being checked here is not that wires reach --- a wire that does
//! not reach fails to resolve and the cable refuses to build. It is that
//! **the fan-out is the multiplexor's own gates**. A model that chose the
//! unit itself would boot and pass everything and not be MIT's, so the two
//! signals whose behaviour MIT wrote down are exercised through the cable
//! and asserted against what MIT says they do.
//!
//! Quoted rather than cited, this repository not holding `lmdoc/disk.22`:
//! Selected Unit Attention is "the attention signal directly from the
//! drive, it is not separately latched in the controller", and Any
//! Attention is "some unit has an attention, you have to select them one
//! after another to find out which".

use muir::dm::Dm;
use muir::netlist::{self, Netlist};
use muir::part::Level;

mod support;

const CADRDC: &str = include_str!("../data/CADRDC.netlist");
const DM: &str = include_str!("../data/DM.netlist");

fn boards() -> (Netlist, Netlist) {
    (netlist::parse_with_multiplexor(CADRDC).unwrap(), netlist::parse(DM).unwrap())
}

fn net(n: &Netlist, name: &str) -> muir::netlist::NetId {
    n.by_name_id(name)
        .or_else(|| n.by_name_id(&format!("'{name}'")))
        .unwrap_or_else(|| panic!("no net {name}"))
}

/// Drives the eight drive ports' attention lines and the three unit-number
/// bits, settles the board, and gives back what the controller side shows
/// for the two attention signals. The lines are active low, so an
/// attention is a `Low` on `TRIDENT.<unit>.ATTENTION/`. The two signals
/// the controller sees carry no `/` and no leading minus and are active
/// high, so an attention raised on the cable is a `Low` at the drive and a
/// `High` at the controller: the board inverts, which is what the drive's
/// open-collector line and the receiver on this side amount to.
fn attentions(dm: &mut Dm, n: &Netlist, raised: &[u8], unit: u8) -> (Level, Level) {
    for u in 0..8u8 {
        let level = if raised.contains(&u) { Level::Low } else { Level::High };
        dm.board.drive(net(n, &format!("TRIDENT.{u}.ATTENTION/")), level);
    }
    for b in 0..3 {
        let level = if unit >> b & 1 != 0 { Level::High } else { Level::Low };
        dm.board.drive(net(n, &format!("UNIT{b}")), level);
    }
    // The unit number alone selects nothing. `-LOAD DA` enables the
    // 25LS2538 decoder at 0E05 that makes `ADDRESS UNIT <n>` of it, and the
    // addressed drive has to answer `SELECTED/` before the 25LS2521
    // comparator at 0F03 gives `SELECT OK` and the 74S00 at 0D04 enables
    // that unit's buffer. So the drive answers here, as a drive does.
    dm.board.drive(net(n, "'-LOAD DA'"), Level::Low);
    for u in 0..8u8 {
        let level = if u == unit { Level::Low } else { Level::High };
        dm.board.drive(net(n, &format!("TRIDENT.{u}.SELECTED/")), level);
    }
    dm.board.settle_all();
    (dm.board.net(net(n, "'SEL UNIT ATTENTION'")), dm.board.net(net(n, "'ANY ATTENTION'")))
}

/// **The cable resolves on both boards.** Twenty-five signals, every one
/// of them a net the controller and the multiplexor both have; three of
/// the controller's `DCEDGE` posts are not the multiplexor's and are named
/// in `muir::dm::wire_names`. Building the cable is the check --- a name
/// on neither board panics --- so this is the assertion that it built.
#[test]
fn the_cable_is_twenty_five_signals_both_boards_have() {
    let (dc, dm) = boards();
    let names = muir::dm::wire_names();
    assert_eq!(names.len(), 25, "signals on the cable");
    for name in &names {
        assert!(
            dc.by_name_id(name).or_else(|| dc.by_name_id(&format!("'{name}'"))).is_some(),
            "the controller has {name}"
        );
        assert!(
            dm.by_name_id(name).or_else(|| dm.by_name_id(&format!("'{name}'"))).is_some(),
            "the multiplexor has {name}"
        );
    }
    let _ = Dm::new(&dc, &dm, 0);
}

/// **Any Attention is the eight units ORed, and it is the board that ORs
/// them.** "Some unit has an attention, you have to select them one after
/// another to find out which": so it must be up for a raised attention on
/// any unit whatever the unit number says, and down only when none is
/// raised. Held for every single unit alone, for none, and for all eight.
///
/// Nothing in `src/dm.rs` knows what a unit is; this is the 74S133 and the
/// gates behind it on the multiplexor's own pages doing the work.
#[test]
fn any_attention_is_the_eight_units_ored() {
    let (dc, dmn) = boards();
    let mut dm = Dm::new(&dc, &dmn, 0);

    let (_, none) = attentions(&mut dm, &dmn, &[], 0);
    assert_eq!(none, Level::Low, "no unit has an attention");

    for u in 0..8u8 {
        // The unit number is left at 0 throughout: Any Attention is not
        // allowed to depend on which unit is selected.
        let (_, any) = attentions(&mut dm, &dmn, &[u], 0);
        assert_eq!(any, Level::High, "unit {u} alone raises Any Attention");
    }
    let (_, all) = attentions(&mut dm, &dmn, &[0, 1, 2, 3, 4, 5, 6, 7], 0);
    assert_eq!(all, Level::High, "all eight raise it");
}

/// **Selected Unit Attention is the selected drive's own line, and the
/// unit number is what selects it.**
///
/// **Ignored: the multiplexor cannot choose a unit yet.** The 25LS2538 at
/// DMSECT 0E05 is the decoder that turns `UNIT<2:0>` into `ADDRESS UNIT
/// <n>`, and `src/part.rs` has no pinout for it, so its outputs float,
/// the 25LS2521 comparator at 0F03 never gives `SELECT OK`, no `-UNIT <n>
/// ENB` is ever asserted and the tri-state bus below is never driven.
///
/// The bus floating is why this test is written as it is and why it is not
/// simply deleted until then. `SEL UNIT ATTENTION` is driven by four
/// 74S240 buffers onto one net; undriven it reads `High` through its
/// pull-up, which is the same level an asserted attention gives. An
/// earlier version of this test asserted `High` for the selected unit and
/// **passed with no buffer enabled at all**. That is the failure this
/// whole piece exists to avoid, and it is only visible by reading
/// `-UNIT <n> ENB` rather than the bus. "The attention signal directly from
/// the drive, it is not separately latched in the controller": so with one
/// unit's attention raised it must follow the unit number --- present when
/// that unit is selected and absent when another is.
///
/// This is the test that would fail if the fan-out were invented. A model
/// that passed the attention of whichever drive happened to be there, or
/// that ORed the eight into this signal too, gives the same answer for
/// every unit number; the board does not.
#[test]
#[ignore = "the 25LS2538 unit decoder has no pinout in src/part.rs, so no unit is ever selected"]
fn selected_unit_attention_follows_the_unit_number() {
    let (dc, dmn) = boards();
    let mut dm = Dm::new(&dc, &dmn, 0);
    let mut seen = Vec::new();
    for raised in 0..8u8 {
        for unit in 0..8u8 {
            let (sel, _) = attentions(&mut dm, &dmn, &[raised], unit);
            seen.push((raised, unit, sel));
        }
    }
    for &(raised, unit, sel) in &seen {
        if raised == unit {
            assert_eq!(sel, Level::High, "unit {unit} selected, its own attention reaches");
        } else {
            assert_eq!(sel, Level::Low, "unit {unit} selected, unit {raised}'s does not");
        }
    }
}
