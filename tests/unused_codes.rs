// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What runs of the codes the CADR leaves unassigned and QUUX took for its
//! clocks: functional destinations 3 to 7 and functional sources 15 and 17.
//!
//! A scan of the control store finds which microinstructions carry them;
//! an instruction the OA registers modify as it loads (`IMOD`) is made at
//! run time and no scan sees it. So a band is booted on `rtl` and every
//! executed microinstruction is read as it stood in `IR`, the OA
//! substitution done: each that writes destinations 3 to 7 or reads source
//! 17 has to be a control-store word that already did.

use std::path::PathBuf;

use muir::engine::Engine;
use muir::machine::{Geometry, Machine};
use muir::rtl::Rtl;
use muir::tv::Board;

mod support;

fn octal(pcs: &[u16]) -> String {
    pcs.iter().map(|p| format!("{p:o}")).collect::<Vec<_>>().join(" ")
}

/// Whether `ir` writes functional destinations 3 to 7, or reads functional
/// source 15 or 17: an ALU or BYTE instruction with `IR<25>` clear and
/// `IR<23:19>` 3 to 7, or any class with `IR<31>` set and `IR<30:26>` 17.
fn uses_the_codes(ir: u64) -> bool {
    let class = ir >> 43 & 3;
    let dest =
        (class == 0 || class == 3) && ir >> 25 & 1 == 0 && (3..=7).contains(&(ir >> 19 & 0o37));
    // `IR<30>` is in no source decode, so 35 and 37 are 15 and 17 again.
    let src = ir >> 31 & 1 == 1 && matches!(ir >> 26 & 0o17, 0o15 | 0o17);
    dest || src
}

/// Boots `m` to its listener on `rtl`, checking every executed
/// microinstruction: the addresses whose executed word used the codes, and
/// those among them whose control-store word did not. It asserts the
/// listener came, so that a boot stuck early is not a pass.
fn run(m: Machine, chaos: (u16, u16), root: PathBuf) -> (Vec<u16>, Vec<u16>) {
    let mut e = Rtl::new(m);
    e.boot();
    let m = e.machine_mut();
    m.chaos.address = chaos.0;
    support::ChaosServer::new(chaos.1)
        .serving(root)
        .at_time(support::time::TEST_UNIVERSAL)
        .plug(m, 0);
    let (mut used, mut made) = (Vec::new(), Vec::new());
    // Up to the listener, checked every million microcycles, and two
    // million more.
    let mut until = 300_000_000u64;
    for n in 0..300_000_000u64 {
        if n == until {
            break;
        }
        if n % 1_000_000 == 0 && until == 300_000_000 && support::lit_rows(&e, 84..130) > 400 {
            until = n + 2_000_000;
        }
        let ir = e.ir();
        e.step().unwrap();
        if let Some(pc) = e.executed()
            && uses_the_codes(ir)
        {
            if !used.contains(&pc) {
                used.push(pc);
            }
            let stored = e.machine().imem[pc as usize].raw();
            if !uses_the_codes(stored) && !made.contains(&pc) {
                made.push(pc);
            }
        }
    }
    assert!(support::lit_rows(&e, 84..130) > 400, "the listener never came");
    (used, made)
}

/// **System 1002 uses the clocks' codes only where its microcode says so**,
/// through its boot to the listener and a moment after on QUUX: no
/// instruction the OA registers make writes destinations 3 to 7 or reads
/// sources 15 or 17. Its microcode, 1000 for Q8 (dev11), uses them at
/// the tick's own sites.
#[test]
fn system_1002_uses_the_tick_s_codes_only_where_its_microcode_does() {
    let from = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ref/band-1002-dev11");
    if !from.join("pack-1002-dev11.vhd").exists() {
        eprintln!("skipped: {} is not present", from.display());
        return;
    }
    // A copy of the disk, a dynamic VHD booted as it is: the machine
    // writes it.
    let dir = support::scratch("unused-codes-1002");
    let pack = dir.join("pack.vhd");
    std::fs::copy(from.join("pack-1002-dev11.vhd"), &pack).unwrap();
    let untar = std::process::Command::new("tar")
        .arg("xzf")
        .arg(from.join("tree-1002-dev11.tar.gz"))
        .arg("-C")
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(untar.success());
    let root = dir.join("root");
    std::fs::create_dir_all(root.join("lispm")).unwrap();
    for part in ["sys", "site"] {
        std::os::unix::fs::symlink(dir.join("release-1002").join(part), root.join(part)).unwrap();
    }
    let mut m = Machine::new();
    m.load_prom(&muir::prom::quux_boot_prom());
    let mut d = muir::block_disk::BlockDisk::new(muir::block_disk::BLOCK_NS);
    d.attach(muir::disk_image::Disk::open_rw(&pack).unwrap());
    m.block_disk = Some(d);
    m.geometry = Geometry::QUUX;
    m.tv.set_board(Board::MonoTv);
    m.tv.set_mono_tv_size(1280, 1024);
    let (used, made) = run(m, (0o177201, 0o177200), root);
    eprintln!("1002: the codes ran at {}", octal(&used));
    assert!(!used.is_empty(), "the tick's own sites ran");
    assert!(made.is_empty(), "made by the OA registers at {}", octal(&made));
}

/// **System 1001 on MIT's 323, on the CADR, never runs them at all**, the
/// OA registers' words included, through its boot to the listener.
#[test]
fn system_1001_on_323_never_runs_the_codes() {
    let (Some(pack), Some(sources)) =
        (support::vendor(&["run", "release-1001-pack.img"]), support::vendor(&["system-1001"]))
    else {
        return;
    };
    let dir = support::scratch("unused-codes-1001");
    let copy = dir.join("pack.img");
    std::fs::copy(&pack, &copy).unwrap();
    let root = dir.join("root");
    std::fs::create_dir_all(root.join("lispm")).unwrap();
    std::os::unix::fs::symlink(sources.join("sys"), root.join("sys")).unwrap();
    std::os::unix::fs::symlink(sources.join("site"), root.join("site")).unwrap();
    let (used, _) = run(support::machine_with_pack(&copy), (0o177201, 0o177200), root);
    assert!(used.is_empty(), "the codes ran at {}", octal(&used));
}
