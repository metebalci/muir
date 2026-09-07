// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Checks the part pinouts against the netlist.
//!
//! These are not truth-table tests; they are consistency tests. A pinout is
//! claimed in `src/part.rs`, and the netlist says how 1243 parts are actually
//! wired. If an output pin is wrong, it shows up here as a part reaching for
//! its own supply pin, or as two push-pull outputs fighting over one net.

use std::collections::BTreeMap;

use muir::netlist::{self, Netlist};
use muir::part;

mod support;
use support::is_power;

const NETLIST: &str = include_str!("../data/CADR.netlist");

fn load() -> Netlist {
    netlist::parse(NETLIST).unwrap()
}

/// A pinout must not claim an output on its own supply pin. The netlist
/// check cannot catch this on a tri-state or open-collector part, since those
/// are exempt from the two-drivers rule --- which is exactly how a `74S472`
/// entry claiming an output on pin 10, its ground, survived for a while.
#[test]
fn no_pinout_drives_its_own_supply_pin() {
    support::no_pinout_drives_its_own_supply_pin(&load());
}

/// No part has a pin on its own ground or supply, which a wrong pinout or a
/// wrong package size would give it.
#[test]
fn modelled_parts_never_touch_their_own_supply_pins() {
    let checked = support::no_part_touches_its_own_supply_pins(&load());
    eprintln!("supply-pin check covered {checked} parts");
    assert!(checked > 400, "only {checked} parts modelled");
}

/// The strong one: two totem-pole outputs on one net is an electrical fault,
/// so if a claimed output pin is wrong this is where it surfaces.
#[test]
fn no_net_has_two_push_pull_drivers() {
    // Nothing is excepted, so a new conflict fails the test. `-RESET`, the
    // one that would be --- two 74S37s, totem pole where 7438s would be
    // expected --- is not here: MIT's wire lists show it as two nets, one
    // per board, and `src/netlist.rs` splits them.
    support::no_net_has_two_push_pull_drivers(&load());
}

/// Every part on the board has a pinout, so the `chip` engine builds all of
/// it. The kinds without one are listed, most used first.
#[test]
fn every_part_has_a_pinout() {
    let n = load();
    let mut missing: BTreeMap<&str, usize> = BTreeMap::new();
    let mut modelled = 0;
    for part in &n.parts {
        if part::pinout(&part.kind).is_some() {
            modelled += 1;
        } else {
            *missing.entry(part.kind.as_str()).or_default() += 1;
        }
    }
    eprintln!("{modelled}/{} parts modelled", n.parts.len());
    let mut rest: Vec<_> = missing.into_iter().collect();
    rest.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
    assert!(rest.is_empty(), "parts with no pinout: {rest:?}");
}

/// A net driven but never consumed, paired with one consumed but never
/// driven, is the signature of one physical net split under two names. MIT's
/// own lists have such wires --- `cadrwd/icmem3.wlr` lists `-TPW60` as a
/// second name on `-TPDONE`'s wire --- and a reader that kept both names
/// would show exactly this pair. `src/netlist.rs` joins the ones it knows by
/// `EXPLICIT_ALIASES`.
///
/// This reports both lists so the pairing can be done by hand; a rule cannot
/// find them, since the two names need have nothing in common.
#[test]
#[ignore = "a report, not a check: --ignored --nocapture"]
fn report_split_net_candidates() {
    let n = load();
    let mut driven = std::collections::BTreeMap::new();
    let mut used = std::collections::BTreeMap::new();
    for part in &n.parts {
        let Some(po) = part::pinout(&part.kind) else { continue };
        for &(pin, net) in &part.pins {
            *(if po.drives(pin) { &mut driven } else { &mut used }).entry(net).or_insert(0) += 1;
        }
    }
    let interesting =
        |name: &str| !is_power(name) && !name.starts_with("NC#") && !name.starts_with('@');
    let dangling: Vec<&str> = driven
        .keys()
        .filter(|id| !used.contains_key(id))
        .map(|&id| n.net(id))
        .filter(|s| interesting(s))
        .collect();
    let orphan: Vec<&str> = used
        .keys()
        .filter(|id| !driven.contains_key(id))
        .map(|&id| n.net(id))
        .filter(|s| interesting(s))
        .collect();
    eprintln!("driven but never consumed ({}): {dangling:?}", dangling.len());
    eprintln!("consumed but never driven ({}): {orphan:?}", orphan.len());
}

/// The complementary report to the one above. A net with no driver at all is
/// either something that arrives from off the board, a supply, or a pin
/// wrongly called an input. Every part being modelled, the list should be
/// short and recognisable.
#[test]
#[ignore = "a report, not a check: --ignored --nocapture"]
fn report_undriven_nets() {
    support::report_undriven_nets(&load());
}

/// A part the table knows only by alias --- the LISPM TV's 2118 for the
/// 4116, its 2141 for the 2147 --- is the whole of its target: the pins,
/// the behaviour and, for a memory, the cells. `memory_words` once resolved
/// the suffixes but not the alias, and the LISPM TV's frame buffer was 64
/// DRAMs with no cells: every read `X`.
#[test]
fn an_aliased_memory_has_its_targets_cells() {
    for (alias, target) in [("2118", "4116VG"), ("2141", "2147")] {
        assert!(part::memory_words(target).is_some(), "{target} is a memory");
        assert_eq!(part::memory_words(alias), part::memory_words(target), "{alias} is a {target}");
    }
}
