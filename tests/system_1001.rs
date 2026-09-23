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
use support::{boot_to_the_prompt, machine_with_pack};

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
