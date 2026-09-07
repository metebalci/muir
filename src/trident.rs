// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The disk controller's two cable tables, derived from the board.
//!
//! `data/trident-connectors.txt` and `data/trident-bus.txt` are reference for
//! the cable seam --- which line of a Trident cable is which, and what the
//! controller puts on it --- and neither is read by the simulator. They are
//! made from `data/CADRDC.netlist` and MIT's wire list `mit/cadrdc/dc.wlr`,
//! so they go stale if either changes.
//!
//! `examples/trident-tables.rs` writes them, `tools/trident-tables.sh` runs
//! that, and `tests/cadrdc_netlist.rs` holds the committed files to what
//! these functions say now.
//!
//! Only the rows are made here. The prose header of each file is written by
//! hand and kept.

use std::collections::BTreeMap;

use crate::netlist::{Netlist, plain};
use crate::wirelist;

/// The two Trident connectors pin by pin, out of MIT's wire list.
pub fn connectors(n: &Netlist, wlr: &str) -> String {
    let mut rows: BTreeMap<(String, u8), String> = BTreeMap::new();
    for s in wirelist::parse(wlr, &n.pages) {
        for c in s.pins.iter().filter(|p| p.body == "CON") {
            if c.location != "J01" && c.location != "J03" {
                continue;
            }
            // The supplies and grounds reach half the board, and listing
            // their hundreds of pins would say nothing about the cable.
            let on: Vec<String> = s
                .pins
                .iter()
                .filter(|q| q.body != "CON")
                .map(|q| format!("{} {} {} {}", q.page, q.location, q.number, q.body))
                .collect();
            let on = if s.is_pseudo() || on.is_empty() { "-".to_string() } else { on.join(", ") };
            rows.insert((c.location.clone(), c.number), format!("{} | {on}", s.name()));
        }
    }
    rows.iter().map(|((c, p), rest)| format!("{c} {p:2} | {rest}\n")).collect()
}

/// What the controller puts on each of the ten disk bus lines, out of the
/// tag multiplexers on DCDBUS.
pub fn bus(n: &Netlist) -> String {
    // Century Data's number and name for each line, by MIT's number. Their
    // Table 4-1, "Bus Definitions", reads bus 0 as `CAR512` down to bus 9
    // as `CAR001` and says "Bus 9 is the LSB", where MIT numbers by bit
    // weight, so bus n is MIT's `DBUS 9-n`. Both count forwards and they
    // count different things; `data/trident-bus.txt` has the whole of it.
    // Here it is only a table.
    const CENTURY: [(u8, &str); 10] = [
        (9, "head advance"),
        (8, "rezero"),
        (7, "head select"),
        (6, "device chk reset"),
        (5, "reset head adr"),
        (4, "address mark"),
        (3, "read"),
        (2, "write"),
        (1, "strobe early"),
        (0, "strobe late"),
    ];
    // Every gate on DCDBUS, as (output net, control nets, data nets). A pin
    // read by more than one of a part's gates is control --- the 74LS244's
    // two enables, the 74S157's select and its enable --- and a pin read by
    // exactly one gate is that gate's data.
    let mut gates: Vec<(&str, Vec<&str>, Vec<&str>)> = Vec::new();
    for p in n.parts.iter().filter(|p| p.page == "DCDBUS") {
        let Some(b) = crate::part::behaviour(&p.kind) else { continue };
        let at: BTreeMap<u8, u32> = p.pins.iter().copied().collect();
        let live: Vec<_> = b.gates.iter().filter(|g| at.contains_key(&g.out)).collect();
        let mut seen: BTreeMap<u8, usize> = BTreeMap::new();
        for g in &live {
            for pin in g.ins.iter().filter(|pin| at.contains_key(pin)) {
                *seen.entry(*pin).or_default() += 1;
            }
        }
        for g in &live {
            let (mut ctl, mut dat) = (Vec::new(), Vec::new());
            for pin in g.ins.iter().filter(|pin| at.contains_key(pin)) {
                let net = plain(n.net(at[pin]));
                if seen[pin] > 1 { &mut ctl } else { &mut dat }.push(net);
            }
            gates.push((plain(n.net(at[&g.out])), ctl, dat));
        }
    }
    // The three 74LS244s answer for DBUS0-7, one tag each. The 74S157
    // answers for DBUS8 and DBUS9: its select is CYLINDER TAG, so its B
    // input is the cylinder's and its A input the control tag's, and its
    // enable is HEAD TAG, so the head tag leaves those two lines undriven.
    let cell = |line: &str, tag: &str| -> String {
        let enable = format!("-{tag}");
        for (out, ctl, dat) in &gates {
            if *out != line {
                continue;
            }
            if ctl.contains(&enable.as_str()) && dat.len() == 1 {
                return dat[0].to_string();
            }
            if ctl.contains(&"CYLINDER TAG") && ctl.contains(&"HEAD TAG") && dat.len() == 2 {
                return match tag {
                    "CYLINDER TAG" => dat[1].to_string(),
                    "CONTROL TAG" => dat[0].to_string(),
                    _ => "-".to_string(),
                };
            }
        }
        "-".to_string()
    };
    let mut out = String::new();
    for (k, (cd, name)) in CENTURY.iter().enumerate() {
        let line = format!("DBUS{k}");
        let (cyl, head, ctl) =
            (cell(&line, "CYLINDER TAG"), cell(&line, "HEAD TAG"), cell(&line, "CONTROL TAG"));
        out.push_str(&format!("{line} | {cyl:4} | {head:17} | {ctl:17} | bus {cd}  {name}\n"));
    }
    out
}
