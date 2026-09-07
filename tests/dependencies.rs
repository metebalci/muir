// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! muir has no crate dependencies, and this is what says so.
//!
//! The README opens with it and `Cargo.toml` carries an empty
//! `[dependencies]`, neither of which fails if one appears. `Cargo.lock` does
//! know: it lists every package in the build, transitive ones included, so
//! one entry means one crate.
//!
//! Cargo rewrites the lock to match `Cargo.toml` before it builds, so an
//! entry cannot simply be edited in or out. That leaves two ways a dependency
//! could arrive, and one check each: CI builds with `--locked`, which fails
//! when the committed lock would have to change, and this test fails when the
//! lock has been regenerated and committed with a new package in it.

/// The lock names exactly one package: this one.
#[test]
fn muir_has_no_crate_dependencies() {
    let lock = include_str!("../Cargo.lock");
    let names: Vec<&str> = lock
        .lines()
        .filter_map(|l| l.strip_prefix("name = "))
        .map(|n| n.trim_matches('"'))
        .collect();
    assert_eq!(names, ["muir"], "muir is meant to have no crate dependencies");
}
