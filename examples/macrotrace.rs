// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The macroinstructions a band retires, as it boots.
//!
//! muir models the hardware and MIT's microcode implements the instruction
//! set, so muir has no macroinstruction decoder of its own and this does
//! not add one. What it does is watch the one place the microcode passes
//! through between instructions --- **`QMLP`, the macrocode main loop** ---
//! and report the location counter and the memory word it points at, each
//! time it arrives there.
//!
//! `QMLP` is `I-MEM 164` octal in `sys/ubin/ucadr.sym`, microcode 323's
//! symbol table. `uc-macrocode.lisp` describes it as the main loop and
//! says "QMLP MUST BE AT LOC WITH BIT 1=0", which `src/micro.rs` quotes
//! where it models the hardware that skips it.
//!
//! **Which half of the word is the instruction.** The location counter is
//! a byte address; `LC<25:2>` is the word a fetch puts on `VMA`, and
//! `LC<1>` picks the half. The gates on page LC say
//! `INST IN LEFT HALF = NOR(-LC MODIFIES MROT, LC1 XOR LC0B)`, so outside
//! the byte modes it is `LC<1>` alone: 0 is the left half and 1 the right.
//! **Which 16 bits MIT's drawings call "left" is not settled here**, so
//! both halves are printed and the reader may align them. Nothing in this
//! program depends on the answer.
//!
//! The machine is not disturbed. The word is read through
//! [`Machine::translate`] and [`Machine::bus_read`] rather than
//! [`Machine::vm_read`], because that one sets `VMAOK`, which the
//! microcode reads --- a tracer that used it would change the run it is
//! watching. A word whose page is not resident is reported as `-` and
//! never as a value.
//!
//!     cargo run --release --example macrotrace -- <pack> [count] [limit]
//!
//! `count` is how many to print, 20 by default; `limit` bounds the
//! microcycles spent looking for them.

use std::path::PathBuf;

use muir::disk_unit::{Geometry, Unit};
use muir::engine::Engine;
use muir::machine::{LC_COUNTER, Machine};
use muir::micro::Micro;

/// The macrocode main loop, `sys/ubin/ucadr.sym`: `QMLP I-MEM 164`.
const QMLP: u16 = 0o164;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(pack) = args.first() else {
        eprintln!("usage: macrotrace <pack> [count] [limit]");
        std::process::exit(2);
    };
    let count: usize = args.get(1).and_then(|v| v.parse().ok()).unwrap_or(20);
    let limit: u64 = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(4_000_000_000);

    let mut m = Machine::new();
    m.load_prom(&muir::prom::boot_prom());
    m.disk.attach(0, Unit::open(PathBuf::from(pack), Geometry::T300).expect("the pack opens"));
    let mut e = Micro::new(m);
    e.boot();

    println!("#  {:>12}  {:>10}  {:>6} {:>6}  word", "microcycle", "lc", "left", "right");
    let mut seen = 0usize;
    let mut cycles = 0u64;
    let mut was_at = false;
    while seen < count && cycles < limit {
        if e.step().is_err() {
            eprintln!("halted after {cycles} microcycles with {seen} instructions seen");
            break;
        }
        cycles += 1;
        // The loop is arrived at, not sat in: one report each time the PC
        // reaches it, and not one for every microcycle it spends there.
        let at = e.pc() == QMLP;
        if !at || was_at {
            was_at = at;
            continue;
        }
        was_at = true;
        let lc = e.lc() & LC_COUNTER;
        let word_addr = lc >> 2;
        let m = e.machine_mut();
        let t = m.translate(word_addr);
        let (left, right, word) = if t.access_permitted {
            let w = m.bus_read(t.physical);
            (format!("{:04x}", w >> 16), format!("{:04x}", w & 0xffff), format!("{w:08x}"))
        } else {
            ("-".into(), "-".into(), "not resident".into())
        };
        seen += 1;
        println!("{seen:<3}{cycles:>12}  {lc:>10o}  {left:>6} {right:>6}  {word}");
    }
}
