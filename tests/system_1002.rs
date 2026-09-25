// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! System 1002 on QUUX with MONO TV: muir-sys's development band, which
//! sizes its main screen from the feature page.
//!
//! It is in the gitignored `ref/band-1002-dev9` (muir-sys `0bddeb0`, contract Q5, no Unibus): boot
//! PROM 1000 and microcode 1000 for block-disk, a pack whose Lisp addresses
//! the disk by block, and the tree it was built from. No TV sync program,
//! no speed bits, and no CADR disk controller: QUUX's disk is block-disk.
//! The band takes the screen's size from the feature page at every boot.
//! Without it the tests skip and say so.

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
    let from = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(BAND);
    if !from.join("pack-1002-dev9.img").exists() {
        eprintln!("skipped: {} is not present", from.display());
        return None;
    }
    let dir = support::scratch(name);
    let pack = dir.join("pack.img");
    std::fs::copy(from.join("pack-1002-dev9.img"), &pack).unwrap();
    let untar = std::process::Command::new("tar")
        .arg("xzf")
        .arg(from.join("tree-1002-dev9.tar.gz"))
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

/// The band, muir-sys's hand-over.
const BAND: &str = "ref/band-1002-dev9";

/// The size `band-1002-dev9` was built at; it takes whatever size the
/// feature page says at boot ([`system_1002_sizes_its_screen_at_boot`]).
const BAND_SIZE: (usize, usize) = (1280, 1024);

fn quux(pack: &std::path::Path) -> Machine {
    quux_at(pack, BAND_SIZE)
}

/// QUUX with its own boot PROM at 36000 (`data/quux-promh.mcr`), the pack
/// on block-disk, and MONO TV at `w` by `h`.
fn quux_at(pack: &std::path::Path, (w, h): (usize, usize)) -> Machine {
    use muir::block_disk::{BLOCK_NS, BlockDisk};
    let mut m = Machine::new();
    m.load_prom(&muir::prom::quux_boot_prom());
    let mut d = BlockDisk::new(BLOCK_NS);
    d.attach(muir::disk_image::Disk::open_rw(pack).expect("the pack"));
    m.block_disk = Some(d);
    m.geometry = Geometry::QUUX;
    m.tv.set_board(Board::MonoTv);
    m.tv.set_mono_tv_size(w, h);
    m
}

/// Whether the listener is framed at the screen's own size, and not at any
/// other width a band might have drawn at: its border lights the first and
/// the last pixel of every row through the middle half of the screen, read
/// at the screen's words a line, and read at any other it does not.
fn drawn_at_its_words_a_line(e: &impl Engine) -> bool {
    let (_, h, own) = e.machine().tv.screen();
    framed(e, own, h)
        && [24, 40, 60, 80].into_iter().filter(|&w| w != own).all(|w| !framed(e, w, h))
}

/// Whether, read `words_per_line` words a line, the first and the last
/// pixel of every row in the middle half of `h` rows are lit.
fn framed(e: &impl Engine, words_per_line: usize, h: usize) -> bool {
    let buf = e.machine().tv.buffer();
    let lit = |bit: usize| buf.get(bit / 32).is_some_and(|w| w >> (bit % 32) & 1 != 0);
    let width = words_per_line * 32;
    (h / 4..3 * h / 4).all(|y| lit(y * width) && lit(y * width + width - 1))
}

/// The screen as a GIF in the temporary directory, for a failure message.
fn shot(e: &impl Engine, name: &str) -> String {
    let path = std::env::temp_dir().join(format!("muir-system-1002-{name}.gif"));
    let mut rec = muir::capture::Recorder::new(false);
    rec.sample(&e.machine().tv, 0, 0);
    std::fs::write(&path, rec.gif()).unwrap();
    path.display().to_string()
}

/// **System 1002 reaches its listener on QUUX with MONO TV, drawn at the
/// screen's words a line**, on both engines, at the size the band was built
/// for ([`BAND_SIZE`]): its listener is framed at MONO TV's words a line and
/// at no other width, the CADR's 24 among them, which is the band drawing
/// for the screen it was given.
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

/// **System 1002 sizes its screen at boot**: the same band, built at
/// [`BAND_SIZE`], booted at other sizes, draws its listener at each size's
/// own words a line. 1920 by 1080 is the largest MONO TV QUUX supports.
#[test]
fn system_1002_sizes_its_screen_at_boot() {
    for size in [(1024, 768), (1920, 1080)] {
        let Some((_dir, pack, root)) = band_1002(&format!("system-1002-{}x{}", size.0, size.1))
        else {
            return;
        };
        let mut e = Micro::new(quux_at(&pack, size));
        e.boot();
        let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
        eprintln!("{size:?}: listener after {ran} microcycles");
        assert_eq!(e.machine().tv.screen().2, size.0 / 32, "{size:?}: the screen's words a line");
        assert!(
            drawn_at_its_words_a_line(&e),
            "{size:?}: drawn at the screen's words a line; the screen is {}",
            shot(&e, &format!("{}x{}", size.0, size.1))
        );
    }
}

/// **System 1002 runs at its ticks**: QUUX drops the delay lines, and its
/// microcycle is `sync`'s K ticks of 10 ns. The same microcode and band
/// reach the same listener at four ticks and at three, and the time to it
/// is shorter at three by less than the microcycles' ratio, the bus keeping
/// its own time.
#[test]
fn system_1002_runs_at_its_ticks() {
    use muir::clock::TimingModel;
    let mut times = Vec::new();
    for ticks in [4, 3] {
        let model = TimingModel::Sync { cycle_ticks: ticks, ilong_ticks: 0 };
        let Some((_dir, pack, root)) = band_1002(&format!("system-1002-sync-{ticks}")) else {
            return;
        };
        let mut e = Rtl::new(quux(&pack));
        e.set_timing_model(model);
        e.boot();
        let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
        assert!(
            drawn_at_its_words_a_line(&e),
            "{ticks} ticks: at the screen's words a line; the screen is {}",
            shot(&e, &format!("sync-{ticks}"))
        );
        eprintln!("{ticks} ticks: listener after {ran} microcycles, {} ns", e.ns());
        times.push(e.ns());
    }
    let ratio = times[0] as f64 / times[1] as f64;
    assert!(ratio > 1.0 && ratio < 4.0 / 3.0, "{ratio:.3} times faster at three ticks");
}
