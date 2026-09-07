// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What is on a pack, and what a band says about itself.
//!
//!     cargo run --example band                 # the pack's partition table
//!     cargo run --example band LOD1            # one band's system communication area
//!     cargo run --example band LOD1 3400 20    # 20 words from virtual 0o3400
//!
//! Addresses and word values print in octal, the way MIT's sources write them.
//! Needs the vendored pack.

use std::path::PathBuf;

use muir::band::{Band, Label, PAGE_WORDS, Q, cdr, sys_com};

const FIELDS: [&str; sys_com::COUNT] = [
    "AREA-ORIGIN-PNTR",
    "VALID-SIZE",
    "PAGE-TABLE-PNTR",
    "PAGE-TABLE-SIZE",
    "OBARRAY-PNTR",
    "ETHER-FREE-LIST",
    "ETHER-TRANSMIT-LIST",
    "ETHER-RECEIVE-LIST",
    "BAND-FORMAT",
    "GC-GENERATION-NUMBER",
    "UNIBUS-INTERRUPT-LIST",
    "TEMPORARY",
    "FREE-AREA/#-LIST",
    "FREE-REGION/#-LIST",
    "MEMORY-SIZE",
    "WIRED-SIZE",
    "CHAOS-FREE-LIST",
    "CHAOS-TRANSMIT-LIST",
    "CHAOS-RECEIVE-LIST",
    "DEBUGGER-REQUESTS",
    "DEBUGGER-KEEP-ALIVE",
    "DEBUGGER-DATA-1",
    "DEBUGGER-DATA-2",
    "MAJOR-VERSION",
    "DESIRED-MICROCODE-VERSION",
    "HIGHEST-VIRTUAL-ADDRESS",
    "POINTER-WIDTH",
];

fn pack() -> Option<PathBuf> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.extend(["vendor", "run", "disk-sys-100-0.img"]);
    if !p.exists() {
        eprintln!("{} is not present", p.display());
        return None;
    }
    Some(p)
}

/// One Q, as `CDR-NEXT DTP-SYMBOL 0o341`.
fn show(q: Q) -> String {
    format!("{:<10} {:<20} 0o{:o}", cdr::NAMES[q.cdr_code() as usize], q.type_name(), q.pointer())
}

fn partition_table(label: &Label) {
    println!(
        "{} cylinders, {} heads, {} blocks per track; microcode loads from {}\n",
        label.cylinders, label.heads, label.blocks_per_track, label.microload_partition
    );
    println!("{:<6} {:>8} {:>8}  comment", "name", "start", "blocks");
    for p in &label.partitions {
        println!("{:<6} {:>8} {:>8}  {}", p.name, p.start, p.blocks, p.comment);
    }
}

fn system_communication_area(name: &str, b: &Band) {
    let kind = if b.compressed() { "compressed" } else { "uncompressed" };
    println!("{name}: {kind}, {} of {} words live\n", b.valid_size(), b.valid_pages() * PAGE_WORDS);

    for (i, q) in b.sys_com().iter().enumerate() {
        println!("0o{:o}  {:<26} {}", sys_com::ORIGIN as usize + i, FIELDS[i], show(*q));
    }

    println!();
    match b.major_version() {
        Some(v) => println!("System {v}"),
        None => println!("No major version: a fresh cold load (coldut.lisp:1077)"),
    }
    match b.desired_microcode_version() {
        Some(v) => println!("Wants microcode {v}"),
        None => println!("No microcode version: set by system initialization (coldut.lisp:1078)"),
    }
    if b.compressed() {
        println!("\nCompressed, so virtual address N is not word N: no words readable.");
    }
}

fn dump(b: &Band, from: u32, count: u32) {
    for a in from..from + count {
        match b.read(a) {
            Some(q) => println!("0o{a:o}  {}", show(q)),
            None => {
                println!("0o{a:o}  unreadable (past the valid size, or a compressed band)");
                return;
            }
        }
    }
}

fn main() {
    let Some(image) = pack() else { return };
    let label = match Label::open(&image) {
        Ok(l) => l,
        Err(e) => return eprintln!("{e}"),
    };

    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(name) = args.first() else { return partition_table(&label) };

    let band = match label.band(name) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            eprintln!(
                "try one of: {}",
                label.partitions.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(" ")
            );
            return;
        }
    };

    // A bare band name prints the system communication area; an address and a
    // count print words.  Both are read octal, as MIT writes them.
    match (args.get(1), args.get(2)) {
        (Some(from), count) => {
            let Ok(from) = u32::from_str_radix(from, 8) else {
                return eprintln!("{from} is not an octal address");
            };
            let count = count.and_then(|c| c.parse().ok()).unwrap_or(16);
            dump(&band, from, count);
        }
        _ => system_communication_area(name, &band),
    }
}
