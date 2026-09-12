// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where the build gets the commit it was made from, for `--version` and
//! for the line every run starts with.
//!
//! A report of a run says which muir made it, and the crate's version is
//! too coarse to answer that: it has been `0.1.0` throughout and says
//! nothing about which of several hundred commits is running, or whether
//! the tree had uncommitted work in it at the time. So the build asks git
//! for the abbreviated commit, and for whether anything was modified, and
//! `src/main.rs` prints what it finds.
//!
//! The commit and not a tag: the repository's own tags name vendored
//! release assets --- `system-100-0` is the System 100 pack and sources
//! --- so a tag is no version of muir and describing against one would
//! say something untrue.
//!
//! Git is asked, not required. A tree built from a source archive has no
//! repository, git may not be installed, and neither is an error: the
//! build says nothing and `--version` falls back to the crate's version
//! alone.

use std::process::Command;

fn main() {
    // The build has to run again when the commit moves or the tree is
    // touched, or `--version` goes stale without anything looking wrong.
    // `.git/HEAD` moves on a checkout and `.git/index` on a stage, which
    // is also what `git status` reads.
    for f in [".git/HEAD", ".git/index"] {
        if std::path::Path::new(f).exists() {
            println!("cargo:rerun-if-changed={f}");
        }
    }

    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git").args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };

    let Some(commit) = git(&["rev-parse", "--short", "HEAD"]).filter(|c| !c.is_empty()) else {
        return;
    };
    // Uncommitted work, tracked or not: a build made from a tree that is
    // not any commit says so, since the commit alone would name something
    // that was never built.
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());

    println!("cargo:rustc-env=MUIR_GIT={commit}{}", if dirty { "-dirty" } else { "" });
}
