// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! System 1001, the release that continues System 100, boots on muir.
//!
//! The pack and the sources are muir-sys's `release-1001`, fetched by
//! `tools/fetch-system-1001.sh`; without them these tests say they were
//! skipped. The band's site puts the machine LISPM-1 at 177201 and its
//! file and time host OZ at 177200 (`site/hosts.text` in the sources), and
//! its `SYS:` translations send `SYS: SITE;` to `/site/` and the rest to
//! `/sys/` on OZ (`site/sys.translations`), so the harness's server is given
//! a root with those two names in it and a writable `lispm` beside them.

use std::path::PathBuf;

use muir::engine::Engine;
use muir::micro::Micro;
use muir::rtl::Rtl;

mod support;
use support::{boot_to_the_prompt, boot_to_the_prompt_within, machine_with_pack};

/// LISPM-1 and OZ, as `site/hosts.text` gives them.
const CHAOS_1001: (u16, u16) = (0o177201, 0o177200);

/// A copy of the release's pack, which a run writes to, and a file root
/// serving the release's `sys` and `site`, in a scratch directory; or
/// `None` with the skip line.
fn release_1001(name: &str) -> Option<(support::Scratch, PathBuf, PathBuf)> {
    let pack = support::vendor(&["run", "release-1001-pack.img"])?;
    let sources = support::vendor(&["system-1001"])?;
    let dir = support::scratch(name);
    let copy = dir.join("pack.img");
    std::fs::copy(&pack, &copy).unwrap();
    let root = dir.join("root");
    std::fs::create_dir_all(root.join("lispm")).unwrap();
    std::os::unix::fs::symlink(sources.join("sys"), root.join("sys")).unwrap();
    std::os::unix::fs::symlink(sources.join("site"), root.join("site")).unwrap();
    Some((dir, copy, root))
}

/// The same, with the served `sys/ubin/ucadr.tbl` replaced by `table`: the
/// band asks for its running microcode's error table as `SYS: UBIN; UCADR
/// TBL <version>`, and its translations send that to `/sys/ubin/ucadr.tbl`
/// with no version in the name, so a served tree holds one microcode's
/// table.
fn serving_table(root: &std::path::Path, table: &std::path::Path) {
    let sources = support::vendor(&["system-1001"]).unwrap();
    std::fs::remove_file(root.join("sys")).unwrap();
    std::fs::create_dir_all(root.join("sys/ubin")).unwrap();
    for entry in std::fs::read_dir(sources.join("sys")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != "ubin" {
            std::os::unix::fs::symlink(entry.path(), root.join("sys").join(entry.file_name()))
                .unwrap();
        }
    }
    for entry in std::fs::read_dir(sources.join("sys/ubin")).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != "ucadr.tbl" {
            std::os::unix::fs::symlink(entry.path(), root.join("sys/ubin").join(entry.file_name()))
                .unwrap();
        }
    }
    std::fs::copy(table, root.join("sys/ubin/ucadr.tbl")).unwrap();
}

/// A microcode rebuilt from the release's sources by muir-sys and handed
/// over into the gitignored `ref/`, with its README saying from what; or
/// `None` with the skip line.
fn rebuilt_microcode(dir: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ref").join(dir);
    if p.join("ucadr.mcr").exists() {
        Some(p)
    } else {
        eprintln!("skipped: {} is not present", p.display());
        None
    }
}

/// `%MICROCODE-VERSION-NUMBER`, A memory's word 40 (`mcr::Mcr::version`
/// has where that is from), as the running machine holds it.
fn microcode_version(e: &impl Engine) -> u32 {
    e.machine().amem[0o40] & 0o77777777
}

/// **System 1001 reaches its listener on `micro`, on microcode 323.** The
/// screen at that point was looked at once: the herald says "Experimental
/// System 1001" and "Microcode 323" above ";Reading at top level in Lisp
/// Listener 1".
#[test]
fn system_1001_reaches_the_listener_on_micro() {
    let Some((_dir, pack, root)) = release_1001("system-1001-micro") else { return };
    let mut e = Micro::new(machine_with_pack(&pack));
    e.boot();
    let ran = boot_to_the_prompt(&mut e, CHAOS_1001, root);
    eprintln!("listener after {ran} microcycles");
    assert_eq!(microcode_version(&e), 323);
}

/// **And on `rtl`.**
#[test]
fn system_1001_reaches_the_listener_on_rtl() {
    let Some((_dir, pack, root)) = release_1001("system-1001-rtl") else { return };
    let mut e = Rtl::new(machine_with_pack(&pack));
    e.boot();
    let ran = boot_to_the_prompt(&mut e, CHAOS_1001, root);
    eprintln!("listener after {ran} microcycles");
    assert_eq!(microcode_version(&e), 323);
}

/// **Step 0: System 1001 runs on a microcode rebuilt with a new version
/// number, and no rebuilt band.** muir-sys's reassembly of the release's
/// sources, which differs from 323 in its version alone (`ref/ucode-324`),
/// loaded into MCR2 and made current with `diskpack`, which writes the
/// partition's comment as MIT's `LOAD-MCR-FILE` does; the band's error table
/// served for that version. The band reaches its listener on `micro` and
/// `rtl` with the new version in A memory.
#[test]
fn system_1001_runs_on_a_rebuilt_microcode() {
    use muir::diskpack::{Command, Pack};
    let Some(ucode) = rebuilt_microcode("ucode-324") else { return };
    let want = muir::mcr::parse(&std::fs::read(ucode.join("ucadr.mcr")).unwrap())
        .unwrap()
        .version()
        .expect("the microcode says its version");
    assert_ne!(want, 323, "a rebuilt version, not the release's");
    for engine in ["micro", "rtl"] {
        let Some((_dir, pack, root)) = release_1001(&format!("system-1001-rebuilt-{engine}"))
        else {
            return;
        };
        serving_table(&root, &ucode.join("ucadr.tbl"));
        let (mut p, _) = Pack::open(&pack);
        p.run(Command::Load { partition: "MCR2".to_string(), file: Some(ucode.join("ucadr.mcr")) })
            .unwrap();
        p.run(Command::Microload("MCR2".to_string())).unwrap();
        let label = muir::band::Label::open(&pack).unwrap();
        assert_eq!(label.microload_partition, "MCR2");
        assert_eq!(label.partition("MCR2").unwrap().comment, format!("UCADR {want}"));
        let m = machine_with_pack(&pack);
        let ran = match engine {
            "micro" => {
                let mut e = Micro::new(m);
                e.boot();
                let ran = boot_to_the_prompt_within(&mut e, CHAOS_1001, root, 400_000_000);
                assert_eq!(microcode_version(&e), want, "micro");
                ran
            }
            _ => {
                let mut e = Rtl::new(m);
                e.boot();
                let ran = boot_to_the_prompt_within(&mut e, CHAOS_1001, root, 400_000_000);
                assert_eq!(microcode_version(&e), want, "rtl");
                ran
            }
        };
        eprintln!("{engine}: listener on microcode {want} after {ran} microcycles");
    }
}

/// **System 1001 on microcode 323 runs on QUUX as on the CADR.** The
/// microcode knows only the CADR's map and never uses QUUX's extra blocks:
/// the band reaches its listener on both engines, and no level-1 entry the
/// microcode wrote names a block above 37.
#[test]
fn system_1001_runs_on_quux_as_on_the_cadr() {
    for engine in ["micro", "rtl"] {
        let Some((_dir, pack, root)) = release_1001(&format!("system-1001-quux-{engine}")) else {
            return;
        };
        let mut m = machine_with_pack(&pack);
        m.geometry = muir::machine::Geometry::QUUX;
        let (ran, above) = match engine {
            "micro" => {
                let mut e = Micro::new(m);
                e.boot();
                let ran = boot_to_the_prompt(&mut e, CHAOS_1001, root);
                (ran, e.machine().l1_map.iter().filter(|&&x| x > 0o37).count())
            }
            _ => {
                let mut e = Rtl::new(m);
                e.boot();
                let ran = boot_to_the_prompt(&mut e, CHAOS_1001, root);
                (ran, e.machine().l1_map.iter().filter(|&&x| x > 0o37).count())
            }
        };
        eprintln!("{engine}: listener on QUUX after {ran} microcycles");
        assert_eq!(above, 0, "{engine}: level-1 entries above block 37");
    }
}

/// A copy of the release's pack with the microcode in `ucode` loaded into
/// MCR2 and made current, and the served tree holding its error table.
fn with_microcode(pack: &std::path::Path, root: &std::path::Path, ucode: &std::path::Path) {
    use muir::diskpack::{Command, Pack};
    serving_table(root, &ucode.join("ucadr.tbl"));
    let (mut p, _) = Pack::open(pack);
    p.run(Command::Load { partition: "MCR2".to_string(), file: Some(ucode.join("ucadr.mcr")) })
        .unwrap();
    p.run(Command::Microload("MCR2".to_string())).unwrap();
}

/// An A-memory location of the microcode in `ucode`, by its symbol.
fn a_mem(ucode: &std::path::Path, name: &str) -> usize {
    let syms =
        muir::sym::parse(&std::fs::read_to_string(ucode.join("ucadr.sym")).unwrap()).unwrap();
    syms.address(muir::sym::Space::AMem, name).unwrap_or_else(|| panic!("no {name}")) as usize
}

/// **System 1001 runs on QUUX's own microcode, 1000.** muir-sys's first
/// microcode for QUUX (`ref/ucode-1000`): the six-bit level-1 entry, 63
/// level-2 blocks, and `A-PROCESSOR-TYPE-CODE` 4. On QUUX the band reaches its
/// listener on both engines with version 1000 and type 4 in A memory, on the
/// band as released: no rebuild.
#[test]
fn system_1001_runs_on_quux_microcode_1000() {
    let Some(ucode) = rebuilt_microcode("ucode-1000") else { return };
    let type_code = a_mem(&ucode, "A-PROCESSOR-TYPE-CODE");
    for engine in ["micro", "rtl"] {
        let Some((_dir, pack, root)) = release_1001(&format!("system-1001-1000-{engine}")) else {
            return;
        };
        with_microcode(&pack, &root, &ucode);
        let mut m = machine_with_pack(&pack);
        m.geometry = muir::machine::Geometry::QUUX;
        let (ran, version, code) = match engine {
            "micro" => {
                let mut e = Micro::new(m);
                e.boot();
                let ran = boot_to_the_prompt_within(&mut e, CHAOS_1001, root, 400_000_000);
                (ran, microcode_version(&e), e.machine().amem[type_code] & 0o77777777)
            }
            _ => {
                let mut e = Rtl::new(m);
                e.boot();
                let ran = boot_to_the_prompt_within(&mut e, CHAOS_1001, root, 400_000_000);
                (ran, microcode_version(&e), e.machine().amem[type_code] & 0o77777777)
            }
        };
        eprintln!("{engine}: QUUX's listener on microcode 1000 after {ran} microcycles");
        assert_eq!((version, code), (1000, 4), "{engine}: version and processor type");
    }
}

/// **Microcode 1000 will not run on a CADR.** It writes a level-1 entry of
/// 77 at boot and reads it back; on a CADR the sixth bit is not there, and it
/// stops at `QUUX-MAP-MISSING` rather than going on to mistranslate.
#[test]
fn microcode_1000_stops_on_a_cadr() {
    let Some(ucode) = rebuilt_microcode("ucode-1000") else { return };
    let Some((_dir, pack, root)) = release_1001("system-1001-1000-on-cadr") else { return };
    with_microcode(&pack, &root, &ucode);
    let syms =
        muir::sym::parse(&std::fs::read_to_string(ucode.join("ucadr.sym")).unwrap()).unwrap();
    let missing = syms.address(muir::sym::Space::IMem, "QUUX-MAP-MISSING").unwrap() as u16;
    let mut e = Micro::new(machine_with_pack(&pack));
    e.boot();
    let mut reached = false;
    for _ in 0..20_000_000 {
        e.step().unwrap();
        if e.executed().is_some_and(|pc| pc == missing) {
            reached = true;
            break;
        }
    }
    assert!(reached, "the CADR never reached QUUX-MAP-MISSING at {missing:o}");
    // And stays there: nothing past it runs.
    for _ in 0..1_000_000 {
        e.step().unwrap();
    }
    assert!((missing..=missing + 1).contains(&e.pc()), "PC {:o}", e.pc());
}
