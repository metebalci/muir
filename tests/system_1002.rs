// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! System 1002 on QUUX with MONO TV: muir-sys's development band, which
//! sizes its main screen from the feature page.
//!
//! It is in the gitignored `ref/band-1002-dev` (muir-sys `6704553`), a pack
//! with microcode 1000 for QUUX revision 4 and the band, and the tree it was
//! built from. Without it the tests skip and say so.

use std::path::PathBuf;

use muir::engine::Engine;
use muir::machine::{Geometry, Machine};
use muir::micro::Micro;
use muir::rtl::Rtl;
use muir::tv::Board;

mod support;

/// LISPM-1 and OZ, as the release's `site/hosts.text` gives them.
const CHAOS: (u16, u16) = (0o177201, 0o177200);

/// A copy of the pack and the served tree, in a scratch directory.
fn band_1002(name: &str) -> Option<(support::Scratch, PathBuf, PathBuf)> {
    let from = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ref/band-1002-dev");
    if !from.join("pack-1002-dev.img").exists() {
        eprintln!("skipped: {} is not present", from.display());
        return None;
    }
    let dir = support::scratch(name);
    let pack = dir.join("pack.img");
    std::fs::copy(from.join("pack-1002-dev.img"), &pack).unwrap();
    let untar = std::process::Command::new("tar")
        .arg("xzf")
        .arg(from.join("tree-1002-dev.tar.gz"))
        .arg("-C")
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(untar.success(), "the tree unpacks");
    let root = dir.join("root");
    std::fs::create_dir_all(root.join("lispm")).unwrap();
    for part in ["sys", "site"] {
        std::os::unix::fs::symlink(dir.join("release-1002").join(part), root.join(part)).unwrap();
    }
    Some((dir, pack, root))
}

fn quux(pack: &std::path::Path) -> Machine {
    let mut m = support::machine_with_pack(pack);
    m.load_prom(&muir::prom::quux_boot_prom());
    m.geometry = Geometry::QUUX;
    m.tv.set_board(Board::MonoTv);
    m
}

/// Lit pixels in `rows` of a screen `words_per_line` words wide.
fn lit(e: &impl Engine, rows: std::ops::Range<usize>, words_per_line: usize) -> u32 {
    e.machine().tv.buffer()[rows.start * words_per_line..rows.end * words_per_line]
        .iter()
        .map(|w| w.count_ones())
        .sum()
}

/// **System 1002 reaches its listener on QUUX with MONO TV, drawn at 60
/// words a line**, on both engines. Its listener comes up where the harness
/// looks for it with the screen read at MONO TV's 60 words a line; read at
/// the CADR's 24, the same rows hold far less, which is the band drawing
/// for the screen it was given and not for the CADR's.
#[test]
fn system_1002_runs_on_mono_tv() {
    for engine in ["micro", "rtl"] {
        let Some((_dir, pack, root)) = band_1002(&format!("system-1002-{engine}")) else {
            return;
        };
        let m = quux(&pack);
        let (ran, at_60, at_24) = match engine {
            "micro" => {
                let mut e = Micro::new(m);
                e.boot();
                let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
                (ran, lit(&e, 84..130, 60), lit(&e, 84..130, 24))
            }
            _ => {
                let mut e = Rtl::new(m);
                e.boot();
                let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
                (ran, lit(&e, 84..130, 60), lit(&e, 84..130, 24))
            }
        };
        eprintln!("{engine}: listener after {ran} microcycles; lit {at_60} at 60, {at_24} at 24");
        assert!(at_60 > 2 * at_24, "{engine}: drawn at 60 words a line");
    }
}
