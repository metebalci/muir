// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What is on the screen.
//!
//! Runs `rtl` --- or `micro`, with `micro` as the third argument --- with
//! the pack and writes the frame buffer as a PNG every `interval`
//! microcycles, at the end, and at the microcycle before a halt, into
//! `vendor/run/screen-<engine>-<microcycle>.png`.
//!
//!     cargo run --release --example screen -- [microcycles] [interval] [micro]

use std::path::PathBuf;

use muir::disk_unit::{Geometry, Unit};
use muir::engine::Engine;
use muir::machine::Machine;
use muir::micro::Micro;
use muir::rtl::Rtl;
use muir::simpletv::SimpleTv;

fn vendor(parts: &[&str]) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("vendor");
    p.extend(parts);
    p
}

fn dump(name: &str, t: &SimpleTv, at: usize, pc: u16) {
    let p = vendor(&["run", &format!("screen-{name}-{at}.png")]);
    std::fs::write(&p, t.png()).unwrap();
    println!("{}: {} pixels lit, mode {:o}, PC {:o}", p.display(), t.lit(), t.mode(), pc);
}

fn run<E: Engine>(name: &str, mut e: E, limit: usize, interval: usize) {
    // The screen a thousand microcycles back, for the frame before a halt.
    let mut previous = (e.machine().simpletv.clone(), e.pc());
    for cycle in 1..=limit {
        if cycle % 1000 == 0 {
            previous = (e.machine().simpletv.clone(), e.pc());
        }
        if let Err(h) = e.step() {
            println!("halted at microcycle {cycle}: {h:?}");
            dump(name, &previous.0, cycle - cycle % 1000, previous.1);
            return;
        }
        if cycle % interval == 0 {
            dump(name, &e.machine().simpletv, cycle, e.pc());
        }
    }
    if !limit.is_multiple_of(interval) {
        dump(name, &e.machine().simpletv, limit, e.pc());
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let limit: usize = args.first().map(|v| v.parse().expect("a number")).unwrap_or(100_000_000);
    let interval: usize = args.get(1).map(|v| v.parse().expect("a number")).unwrap_or(10_000_000);
    let mut m = Machine::new();
    m.load_prom(&muir::prom::boot_prom());
    m.disk.attach(
        0,
        Unit::open(vendor(&["run", "disk-sys-100-0.img"]), Geometry::T300)
            .expect("the System 100 pack"),
    );
    if args.get(2).is_some_and(|v| v == "micro") {
        let mut e = Micro::new(m);
        e.boot();
        run("micro", e, limit, interval);
    } else {
        let mut e = Rtl::new(m);
        e.boot();
        run("rtl", e, limit, interval);
    }
}
