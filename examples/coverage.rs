// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! How much of microcode 323 the `chip`-against-`rtl` comparison has covered.
//!
//! Microcycles are a poor measure of it: the band spends most of them in a
//! few loops. And `chip` is too slow to ask directly. But over the compared
//! window the two engines agree on every `PC`, so the set of control-store
//! words the comparison has exercised is exactly the set `rtl` alone
//! executes in the same number of microcycles --- and `rtl` runs a million
//! microcycles a second.
//!
//!     cargo run --release --example coverage -- [microcycles] [from]
//!
//! Counts the distinct PCs executed from microcycle `from` (default
//! 1,420,000, the first band checkpoint; before `PROM-DISABLE` the PC has
//! walked every address, so the boot PROM would count as everything) to
//! `microcycles` (default 15,000,000), against the words `ucadr.mcr`
//! carries, and prints the running count at every million. The boot PROM is
//! the committed one; `ucadr.mcr` and the pack come from the vendored
//! release.

use std::collections::BTreeSet;
use std::path::PathBuf;

use muir::disk_unit::{Geometry, Unit};
use muir::engine::Engine;
use muir::machine::Machine;
use muir::mcr;
use muir::rtl::Rtl;

fn vendor(parts: &[&str]) -> Option<PathBuf> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("vendor");
    p.extend(parts);
    p.exists().then_some(p)
}

fn main() {
    let mut args = std::env::args().skip(1).map(|v| v.parse::<usize>().expect("a number"));
    let limit = args.next().unwrap_or(15_000_000);
    let from = args.next().unwrap_or(1_420_000);
    let (Some(band), Some(pack)) = (
        vendor(&["system-100-0", "sys", "ubin", "ucadr.mcr"]),
        vendor(&["run", "disk-sys-100-0.img"]),
    ) else {
        eprintln!("skipped: the vendored release or the pack is not present");
        return;
    };
    let band = mcr::parse(&std::fs::read(band).unwrap()).unwrap();
    let words = band.imem.iter().filter(|i| i.raw() != 0).count();
    println!("microcode 323: {} control-store words from {:o}", words, band.imem_start);

    let mut m = Machine::new();
    m.load_prom(&muir::prom::boot_prom());
    m.disk.attach(0, Unit::open(pack, Geometry::T300).expect("the System 100 pack"));
    let mut e = Rtl::new(m);
    e.boot();
    let mut reached = BTreeSet::new();
    for cycle in 0..limit {
        e.step().expect("rtl halted");
        if cycle >= from
            && let Some(pc) = e.executed()
        {
            reached.insert(pc);
        }
        if cycle >= from && (cycle + 1) % 1_000_000 == 0 {
            println!(
                "to {:>10}: {:>5} distinct PCs, {:.1}% of the band",
                cycle + 1,
                reached.len(),
                100.0 * reached.len() as f64 / words as f64
            );
        }
    }
    println!(
        "from {from} to {limit}: {} distinct PCs, {:.1}% of the band",
        reached.len(),
        100.0 * reached.len() as f64 / words as f64
    );
}
