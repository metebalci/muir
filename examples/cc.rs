// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! CC in machine A's band, debugging machine B: the acceptance test, run by
//! hand.  Two `rtl` machines on the debug cable in one process, A with the
//! System 100 pack and the Chaosnet (the Chaosnet server serving the release as
//! `SYS:`), B with the boot PROM and no pack.  A boots to its listener,
//! is typed `(make-system 'cc :noconfirm :nowarn)` --- CC is not in the
//! band; the release's `sys/cc/*.qfasl` come over the FILE service ---
//! and then `(cadr:cc)`, or whatever `MUIR_CC_TYPE` says --- lines
//! separated by `|`, each with the microcycles to give it after an `@` ---
//! each followed by Return.  The screen goes to `vendor/run/screen-cc-<n>.png`
//! every `interval` microcycles of A's, and at the end the debug cable's
//! traffic and B's state are reported.
//!
//!     MUIR_CHAOS_TRACE=1 cargo run --release --example cc -- [microcycles] [interval]

use std::path::PathBuf;

use muir::disk_unit::{Geometry, Unit};
use muir::engine::Engine;
use muir::lashup::Lashup;
use muir::machine::Machine;
use muir::rtl::Rtl;
use muir::simpletv::SimpleTv;
use muir::spy;
use muir::terminal::keyboard::{Keyboard, keysym};

fn vendor(parts: &[&str]) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("vendor");
    p.extend(parts);
    p
}

fn dump(t: &SimpleTv, at: u64, pc: u16) {
    let p = vendor(&["run", &format!("screen-cc-{at}.png")]);
    std::fs::write(&p, t.png()).unwrap();
    println!("{}: {} pixels lit, PC {:o}", p.display(), t.lit(), pc);
}

/// Steps the lashup until A has taken `n` more microcycles.
fn run(l: &mut Lashup, n: u64) -> u64 {
    let from = l.steps.0;
    while l.steps.0 < from + n {
        if let Err(h) = l.step() {
            println!("halted: {h:?}");
            std::process::exit(1);
        }
    }
    l.steps.0
}

/// Types `text` and Return at A, a key at a time, each taken before the
/// next.
fn type_line(l: &mut Lashup, k: &mut Keyboard, text: &str) {
    println!("typing {text:?}");
    let keys: Vec<u32> = text.bytes().map(|b| b as u32).chain([keysym::RETURN]).collect();
    for sym in keys {
        k.key(sym, true);
        k.key(sym, false);
        let mut waited = 0;
        while k.pending() > 0 || l.debugger.machine().ioboard.keyboard_ready() {
            k.deliver(&mut l.debugger.machine_mut().ioboard);
            run(l, 1_000);
            waited += 1_000;
            assert!(waited < 50_000_000, "A never read the keyboard");
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let limit: u64 = args.first().map(|v| v.parse().expect("a number")).unwrap_or(400_000_000);
    let interval: u64 = args.get(1).map(|v| v.parse().expect("a number")).unwrap_or(50_000_000);
    let prom = muir::prom::boot_prom();

    let mut a = Machine::new();
    a.load_prom(&prom);
    a.disk.attach(
        0,
        Unit::open(vendor(&["run", "disk-sys-100-0.img"]), Geometry::T300)
            .expect("the System 100 pack"),
    );
    a.chaos.file_root = Some(vendor(&["run", "file-root"]));
    a.chaos.trace = std::env::var_os("MUIR_CHAOS_TRACE").is_some();
    a.plug_chaos(0);
    let mut b = Machine::new();
    b.load_prom(&prom);
    let (mut ea, mut eb) = (Rtl::new(a), Rtl::new(b));
    ea.boot();
    eb.boot();
    let mut l = Lashup::new(ea, eb);

    // A to its listener: the herald, the time from the host, the prompt.
    let settle: u64 =
        std::env::var("MUIR_CC_SETTLE").ok().and_then(|v| v.parse().ok()).unwrap_or(30_000_000);
    run(&mut l, settle);
    dump(&l.debugger.machine().simpletv, l.steps.0, l.debugger.pc());
    let mut k = Keyboard::new();
    // Each line with the microcycles to give it, `line@cycles`; without a
    // count, an equal share of the window.
    let lines: Vec<(String, Option<u64>)> = match std::env::var("MUIR_CC_TYPE") {
        Ok(v) => v
            .split('|')
            .map(|l| match l.rsplit_once('@') {
                Some((text, n)) if n.parse::<u64>().is_ok() => (text.to_string(), n.parse().ok()),
                _ => (l.to_string(), None),
            })
            .collect(),
        Err(_) => vec![
            ("(login 'lispm)".into(), Some(20_000_000)),
            ("(make-system 'cc :noconfirm :nowarn)".into(), None),
            ("(cadr:cc)".into(), Some(200_000_000)),
        ],
    };
    let mut next_dump = l.steps.0 + interval;
    for (line, budget) in &lines {
        type_line(&mut l, &mut k, line);
        // Let it work, dumping the screen as it goes, until the next line's
        // turn.
        let share = budget.unwrap_or(limit / lines.len() as u64);
        let until = l.steps.0 + share;
        while l.steps.0 < until {
            run(&mut l, 1_000_000);
            if l.steps.0 >= next_dump {
                dump(&l.debugger.machine().simpletv, l.steps.0, l.debugger.pc());
                next_dump += interval;
            }
        }
    }
    dump(&l.debugger.machine().simpletv, l.steps.0, l.debugger.pc());
    let (a, b) = (&l.debugger, &l.debuggee);
    println!(
        "A: {} microcycles, {} debug cycles on the cable, bus error {:o}",
        a.machine().cycles,
        a.debug_cycles(),
        a.machine().bus_error
    );
    println!(
        "B: {} microcycles at {} ns, PC {:o}{}, {}, bus error {:o}",
        b.machine().cycles,
        b.ns(),
        b.pc(),
        if b.machine().mode.prom_disable { "" } else { " in the PROM" },
        if b.spy_read(spy::FLAG_1) & 0x100 != 0 { "running" } else { "halted" },
        b.machine().bus_error
    );
}
