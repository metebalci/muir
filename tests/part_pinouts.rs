// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The part models' pin tables are MIT's pinouts.
//!
//! A wire list names the pin of the body at each end of every wire, beside
//! its number: `1B19-15 ... -CE 74S472`. So MIT's own files say, for each
//! kind of part they wired, which pin is which, and `src/part.rs`'s tables
//! of address and control pins are held to that here. The names are read
//! from the raw lines: the pin's name is the tab-separated column before
//! the body's, which the list gives twice.

use std::collections::BTreeMap;

use muir::part;

mod support;

/// Every pin number and name the wire list `file` gives for `body`.
fn pinout(file: &[&str], body: &str) -> BTreeMap<u8, String> {
    let text = support::mit_text(file);
    let mut pins = BTreeMap::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        let Some(k) =
            (1..cols.len().saturating_sub(1)).find(|&k| cols[k] == body && cols[k + 1] == body)
        else {
            continue;
        };
        let Some(number) = cols.iter().find_map(|c| {
            let (_, rest) = c.trim().split_once('-')?;
            rest.get(..2)?.parse::<u8>().ok()
        }) else {
            continue;
        };
        let name = cols[k - 1].trim();
        if !name.is_empty() {
            pins.insert(number, name.to_string());
        }
    }
    assert!(!pins.is_empty(), "no {body} in {file:?}");
    pins
}

/// The pins `names` are at, by `pinout`.
fn at(pins: &BTreeMap<u8, String>, names: &[&str]) -> Vec<u8> {
    names
        .iter()
        .map(|n| {
            *pins.iter().find(|(_, v)| v == n).unwrap_or_else(|| panic!("no pin {n}: {pins:?}")).0
        })
        .collect()
}

const CADR: &[&str] = &["cadrwd", "cadr4.wlr"];

#[test]
fn the_93425a_is_mits() {
    let p = pinout(CADR, "93425A");
    let addr: Vec<String> = (0..10).map(|k| format!("A{k}")).collect();
    let addr: Vec<&str> = addr.iter().map(String::as_str).collect();
    assert_eq!(part::RAM1K_ADDR, at(&p, &addr));
    assert_eq!(&part::RAM1K_IN[..2], at(&p, &["-CE", "-WE"]));
    assert_eq!(&part::RAM1K_IN[2..], part::RAM1K_ADDR);
}

#[test]
fn the_2147_is_mits() {
    let p = pinout(&["cadrwd", "icmem3.wlr"], "2147");
    let addr: Vec<String> = (0..12).map(|k| format!("A{k}")).collect();
    let addr: Vec<&str> = addr.iter().map(String::as_str).collect();
    assert_eq!(part::RAM4K_ADDR, at(&p, &addr));
    assert_eq!(&part::RAM4K_IN[..2], at(&p, &["-CE", "-WE"]));
    assert_eq!(&part::RAM4K_IN[2..], part::RAM4K_ADDR);
}

#[test]
fn the_82s21_is_mits() {
    let p = pinout(CADR, "82S21");
    assert_eq!(part::RAM32_IN, at(&p, &["CE", "-LATCH", "A0", "A1", "A2", "A3", "A4"]));
}

#[test]
fn the_74s472_is_mits() {
    let p = pinout(&["cadrwd", "icmem3.wlr"], "74S472");
    let names = ["-CE", "A0", "A1", "A2", "A3", "A4", "A5", "A6", "A7", "A8"];
    assert_eq!(part::PROM512_IN, at(&p, &names));
}

#[test]
fn the_74s288_is_mits() {
    let p = pinout(&["cadr1", "busint.wlr"], "74S288");
    assert_eq!(part::PROM32_IN, at(&p, &["-SEL", "ADA", "ADB", "ADC", "ADD", "ADE"]));
}

/// The address as a set: a RAM cannot show which of its pins is the low bit.
#[test]
fn the_am29701_is_mits() {
    let p = pinout(&["cadr1", "busint.wlr"], "29701");
    let mut addr = part::RAM16X4_ADDR.to_vec();
    addr.sort();
    let mut theirs = at(&p, &["A0", "A1", "A2", "A3"]);
    theirs.sort();
    assert_eq!(addr, theirs);
    assert_eq!(&part::RAM16X4_IN[..2], at(&p, &["-CS", "-WE"]));
    assert_eq!(&part::RAM16X4_IN[2..], part::RAM16X4_ADDR);
}

#[test]
fn the_74s283_is_mits() {
    let p = pinout(CADR, "74S283");
    let names = ["A0", "A1", "A2", "A3", "B0", "B1", "B2", "B3", "CIN"];
    assert_eq!(part::ADD_IN, at(&p, &names));
}

#[test]
fn the_25s10_is_mits() {
    let p = pinout(CADR, "25S10");
    let names = ["-OE", "S1", "S0", "I-3", "I-2", "I-1", "I0", "I1", "I2", "I3"];
    assert_eq!(part::SHIFT_IN, at(&p, &names));
}
