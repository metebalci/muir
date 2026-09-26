// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! System 1002 on QUUX with MONO TV: muir-sys's development band, which
//! sizes its main screen from the feature page.
//!
//! It is in the gitignored `ref/band-1002-dev11` (muir-sys `624ad92`,
//! contract Q8): a GPT disk as a dynamic VHD, which QUUX boots as it is,
//! with microcode 1000 in its current `MCR1` and the band, "System 1002
//! dev11", in its current `LOD4`; the GPT PROM, which is muir's built-in
//! `data/quux-promh.mcr`; and the tree it was built from. No TV sync
//! program, no speed bits, and no CADR disk controller: QUUX's disk is
//! block-disk. The band takes the screen's size from the feature page at
//! every boot. Without it the tests skip and say so.

use std::path::PathBuf;

use muir::engine::Engine;
use muir::machine::{Geometry, Machine};
use muir::micro::Micro;
use muir::rtl::Rtl;
use muir::tv::Board;

mod support;

/// LISPM-1 and OZ, as the release's `site/hosts.text` gives them.
const CHAOS: (u16, u16) = (0o177201, 0o177200);

/// A copy of the disk, which the machine writes, and the served tree, in a
/// scratch directory.
fn band_1002(name: &str) -> Option<(support::Scratch, PathBuf, PathBuf)> {
    let from = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(BAND);
    if !from.join("pack-1002-dev11.vhd").exists() {
        eprintln!("skipped: {} is not present", from.display());
        return None;
    }
    let dir = support::scratch(name);
    let pack = dir.join("pack.vhd");
    std::fs::copy(from.join("pack-1002-dev11.vhd"), &pack).unwrap();
    // Booted as it is: a dynamic VHD of a T-300's 263,245 blocks, the
    // hand-over's README says.
    let (format, bytes) = muir::disk_image::probe(&pack).unwrap();
    assert_eq!((format, bytes / 1024), (muir::disk_image::Format::DynamicVhd, 263_245));
    let untar = std::process::Command::new("tar")
        .arg("xzf")
        .arg(from.join("tree-1002-dev11.tar.gz"))
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
const BAND: &str = "ref/band-1002-dev11";

/// The size `band-1002-dev11` was built at; it takes whatever size the
/// feature page says at boot ([`system_1002_sizes_its_screen_at_boot`]).
const BAND_SIZE: (usize, usize) = (1280, 1024);

fn quux(pack: &std::path::Path) -> Machine {
    quux_at(pack, BAND_SIZE)
}

/// QUUX with its own boot PROM at 36000 (`data/quux-promh.mcr`), the disk
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

/// `DISK-AWAIT-READY`, where microcode 1000 waits for block-disk to be
/// ready (dev11's `ucadr.sym`: `DISK-AWAIT-READY I-MEM 25036`).
const DISK_AWAIT_READY: u16 = 0o25036;

/// The disk registers as the microcode addresses them, virtual
/// (`DISK-REGS-ADDRESS-BASE NUMBER 77377774` in dev11's `ucadr.sym`), and
/// where they are, physical: word 774 of the register page, 17377000.
const DISK_REGS: (u32, u32) = (0o77377774, 0o17377774);

/// **System 1002 restores its own band and comes back to the listener**:
/// booted at 1280 by 1024, `(si:disk-restore 4)` answered `yes` reads LOD4
/// back in and boots it to the listener again, on `micro`. The microcode's
/// cold boot maps the disk registers and the run light with
/// `COLD-FAKE-L2-MAP`, and when the two took the same level-2 slot the
/// disk registers' virtual address reached the run light instead: the
/// restore sat in `DISK-AWAIT-READY` for ever, block-disk's disk address
/// never moving on (found by muir, fixed in muir-sys's microcode before
/// dev11). So while the band is read, every microcycle at
/// `DISK-AWAIT-READY` holds the disk registers' virtual address to their
/// physical one, and every million microcycles block-disk has to have moved
/// on. The restore takes about 26 million microcycles to read the band and
/// 165 million to the listener (measured). The test catches the collision:
/// on the band before the fix, dev9 (microcode 1000 for Q5, with the PROM
/// it booted on, which read MIT's label), the disk registers' virtual
/// address reaches 17117774 at `DISK-AWAIT-READY`; with that check taken
/// out, block-disk stands still within 2 million microcycles of the answer
/// (measured).
#[test]
fn system_1002_restores_its_band_to_the_listener() {
    use muir::terminal::keyboard::Keyboard;
    let Some((_dir, pack, root)) = band_1002("system-1002-restore") else {
        return;
    };
    let lod4 = support::gpt_partition(&mut muir::disk_image::Disk::open(&pack).unwrap(), "LOD4");
    let mut m = quux(&pack);
    m.block_disk.as_mut().unwrap().log = Some(Vec::new());
    let mut e = Micro::new(m);
    e.boot();
    let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
    eprintln!("listener after {ran} microcycles");
    let mut k = Keyboard::new();
    support::type_at(&mut e, &mut k, "(si:disk-restore 4)");
    // Time for the question, "Do you really want to reload LOD4 (System
    // 1002 dev11)? (Yes or No)", before its answer.
    for _ in 0..20_000_000 {
        e.step().unwrap();
    }
    support::type_at(&mut e, &mut k, "yes\n");
    let transfers =
        |e: &Micro| e.machine().block_disk.as_ref().unwrap().log.as_ref().unwrap().len();
    let from = transfers(&e);
    // The band read, until the screen goes dark for the boot.
    let mut n = 0u64;
    while support::lit_rows(&e, 84..130) > 400 {
        let (before, mut waiting) = (transfers(&e), 0);
        for _ in 0..1_000_000 {
            e.step().unwrap();
            if e.pc() == DISK_AWAIT_READY {
                waiting += 1;
                let at = e.machine().translate(DISK_REGS.0).physical;
                assert_eq!(
                    at, DISK_REGS.1,
                    "at DISK-AWAIT-READY the disk registers' {:o} reach {at:o}",
                    DISK_REGS.0
                );
            }
        }
        n += 1_000_000;
        assert!(
            transfers(&e) > before,
            "block-disk still after {n} microcycles, {waiting} of the last million at \
             DISK-AWAIT-READY; the screen is {}",
            shot(&e, "restore")
        );
        assert!(n < 100_000_000, "the screen never went dark for the boot");
    }
    let log = &e.machine().block_disk.as_ref().unwrap().log.as_ref().unwrap()[from..];
    let band_reads = log
        .iter()
        .filter(|t| !t.write && (lod4.first..lod4.first + lod4.blocks).contains(&t.block))
        .count();
    eprintln!("band read, {band_reads} blocks of LOD4, after {n} microcycles");
    assert!(band_reads > 0, "LOD4 read");
    let again = support::wait_for_the_prompt_within(&mut e, 400_000_000);
    eprintln!("listener again after {} microcycles more", again);
    assert!(
        drawn_at_its_words_a_line(&e),
        "the listener again, at the screen's words a line; the screen is {}",
        shot(&e, "restored")
    );
}
