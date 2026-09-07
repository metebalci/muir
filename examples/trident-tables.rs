// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Writes `data/trident-connectors.txt` and `data/trident-bus.txt`.
//!
//! Both are derived from `data/CADRDC.netlist` and MIT's wire list for the
//! board, `mit/cadrdc/dc.wlr`, by `muir::trident`. Run this after either
//! changes; `tests/cadrdc_netlist.rs` is what notices that you have not.
//!
//! The prose header of each file is written by hand and kept: only the rows
//! below it are replaced.
//!
//!     tools/trident-tables.sh
//!     cargo run --release --example trident-tables

use std::path::PathBuf;

use muir::{netlist, trident};

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let n = netlist::parse(include_str!("../data/CADRDC.netlist")).expect("data/CADRDC.netlist");
    let wlr = std::fs::read(root.join("mit/cadrdc/dc.wlr")).expect("mit/cadrdc/dc.wlr");
    let wlr = String::from_utf8_lossy(&wlr).into_owned();

    for (name, rows) in [
        ("trident-connectors.txt", trident::connectors(&n, &wlr)),
        ("trident-bus.txt", trident::bus(&n)),
    ] {
        let path = root.join("data").join(name);
        let old = std::fs::read_to_string(&path).unwrap_or_default();
        // Keep the hand-written header: everything down to the last `#` line.
        let head: String = old
            .lines()
            .take_while(|l| l.starts_with('#') || l.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, format!("{head}\n{rows}")).expect("write");
        println!("wrote data/{name}: {} rows", rows.lines().count());
    }
}
