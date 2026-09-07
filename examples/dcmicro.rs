// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Assembles the disk controller's microcode and writes its three PROM
//! images, as MIT's `MICRO` and `newdsk.trans` did between them.
//!
//!     cargo run --example dcmicro -- <source> <name>
//!
//! writes `data/<name>-d03.prom`, `-d04.prom` and `-d05.prom` in the form
//! `newdsk.trans` wrote them. `tools/newdsk-proms.sh` runs it on
//! `cadrdc/newdsk.31`.

use muir::dcmicro;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [source, name] = args.as_slice() else {
        eprintln!("usage: dcmicro <source> <name>");
        std::process::exit(2);
    };
    let text = std::fs::read(source).unwrap_or_else(|e| {
        eprintln!("{source}: {e}");
        std::process::exit(1);
    });
    let asm = dcmicro::assemble(&String::from_utf8_lossy(&text)).unwrap_or_else(|e| {
        eprintln!("{source}: {e}");
        std::process::exit(1);
    });
    let images = asm.proms();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
    for (k, image) in images.iter().enumerate() {
        let prom = format!("D0{}", k + 3);
        let path = root.join(format!("{name}-d0{}.prom", k + 3));
        let title = format!("PROM LOCATION DISK-CONTROL-{prom}");
        std::fs::write(&path, dcmicro::write_prom(&title, image)).unwrap();
        println!("wrote {}", path.display());
    }
    println!("{}: {} words, \"{}\"", source, asm.words.len(), asm.title);
}
