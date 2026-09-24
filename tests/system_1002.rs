// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! System 1002 on QUUX with MONO TV: muir-sys's development band, which
//! sizes its main screen from the feature page.
//!
//! It is in the gitignored `ref/band-1002-dev2` (muir-sys `5427570`), a
//! pack with microcode 1000 for QUUX revision 4 and the band, and the tree
//! it was built from: no TV sync program and no speed bits. Without it the tests skip and say so.

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
    let from = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ref/band-1002-dev2");
    if !from.join("pack-1002-dev2.img").exists() {
        eprintln!("skipped: {} is not present", from.display());
        return None;
    }
    let dir = support::scratch(name);
    let pack = dir.join("pack.img");
    std::fs::copy(from.join("pack-1002-dev2.img"), &pack).unwrap();
    let untar = std::process::Command::new("tar")
        .arg("xzf")
        .arg(from.join("tree-1002-dev2.tar.gz"))
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

/// The size the band's window system was loaded at: a band fixes its
/// screen then, and `band-1002-dev2` was built at 1920 by 1080. Run at
/// another size it draws 60-word lines into the raster anyway.
const BAND_SIZE: (usize, usize) = (1920, 1080);

fn quux(pack: &std::path::Path) -> Machine {
    let mut m = support::machine_with_pack(pack);
    m.load_prom(&muir::prom::quux_boot_prom());
    m.geometry = Geometry::QUUX;
    m.tv.set_board(Board::MonoTv);
    m.tv.set_mono_tv_size(BAND_SIZE.0, BAND_SIZE.1);
    m
}

/// Whether the listener's rows read as text at the screen's own words a
/// line and worse at every other width a band might have drawn at: lit
/// pixels in the rows are most where the lines are read as drawn.
fn drawn_at_its_words_a_line(e: &impl Engine) -> bool {
    let own = e.machine().tv.screen().2;
    let at_own = lit(e, 84..130, own);
    [24, 40, 60, 80].into_iter().filter(|&w| w != own).all(|w| at_own > lit(e, 84..130, w))
}

/// Lit pixels in `rows` of a screen `words_per_line` words wide.
fn lit(e: &impl Engine, rows: std::ops::Range<usize>, words_per_line: usize) -> u32 {
    e.machine().tv.buffer()[rows.start * words_per_line..rows.end * words_per_line]
        .iter()
        .map(|w| w.count_ones())
        .sum()
}

/// **System 1002 reaches its listener on QUUX with MONO TV, drawn at the
/// screen's words a line**, on both engines, at the size the band was built
/// for ([`BAND_SIZE`]). Its listener comes up where the harness looks for
/// it with the screen read at MONO TV's words a line, and reads worse at
/// any other width; read at
/// the CADR's 24, the same rows hold far less, which is the band drawing
/// for the screen it was given and not for the CADR's.
#[test]
fn system_1002_runs_on_mono_tv() {
    for engine in ["micro", "rtl"] {
        let Some((_dir, pack, root)) = band_1002(&format!("system-1002-{engine}")) else {
            return;
        };
        let m = quux(&pack);
        let (ran, drawn) = match engine {
            "micro" => {
                let mut e = Micro::new(m);
                e.boot();
                let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
                (ran, drawn_at_its_words_a_line(&e))
            }
            _ => {
                let mut e = Rtl::new(m);
                e.boot();
                let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
                (ran, drawn_at_its_words_a_line(&e))
            }
        };
        eprintln!("{engine}: listener after {ran} microcycles");
        assert!(drawn, "{engine}: drawn at the screen's words a line");
    }
}

/// **System 1002 runs under `sync`**, QUUX's 40 ns microcycle: the same
/// microcode and band reach the same listener, and the time to it is
/// shorter than at QUUX's one rate of 145 ns by less than the microcycle
/// ratio, the bus keeping its own time.
#[test]
fn system_1002_runs_under_sync() {
    use muir::clock::TimingModel;
    let mut times = Vec::new();
    for model in [TimingModel::Cadr, TimingModel::Sync { cycle_ticks: 4, ilong_ticks: 0 }] {
        let Some((_dir, pack, root)) = band_1002(&format!("system-1002-{}", model.name())) else {
            return;
        };
        let mut e = Rtl::new(quux(&pack));
        e.set_timing_model(model);
        e.boot();
        let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
        assert!(drawn_at_its_words_a_line(&e), "{model:?}: at the screen's words a line");
        eprintln!("{}: listener after {ran} microcycles, {} ns", model.name(), e.ns());
        times.push(e.ns());
    }
    let ratio = times[0] as f64 / times[1] as f64;
    assert!(ratio > 1.5 && ratio < 145.0 / 40.0, "{ratio:.2} times faster");
}
