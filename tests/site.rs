// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The numbers the site quotes about `mit/`.
//!
//! `site/manual.html` tells a reader what MIT left and how much of it there
//! is, and nothing else in the tree would notice those figures going stale.
//! A file added to `mit/` or a page taken out of a print set would leave a
//! public page saying something that used to be true, which is the one thing
//! a page about provenance cannot afford. So they are counted here.
//!
//! `mit/` is committed, so this never skips.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Every file under `mit/`, by extension, and how many there are of each.
/// `README.md` is this project's own and is not MIT's, so it is left out ---
/// which is what makes the total the site quotes MIT's files rather than the
/// directory's.
fn extensions() -> (usize, BTreeMap<String, usize>) {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).expect("mit/ is committed and always here") {
            let p = e.expect("a directory entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.file_name().is_some_and(|n| n != "README.md") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("mit"), &mut files);
    let mut by_extension = BTreeMap::new();
    for f in &files {
        let Some(e) = f.extension().and_then(|e| e.to_str()) else { continue };
        *by_extension.entry(e.to_string()).or_insert(0) += 1;
    }
    (files.len(), by_extension)
}

/// **The manual's table of MIT's file types is a count of `mit/`.**
///
/// The rows read `<td class="name">.drw</td><td class="num">351</td>`, so
/// each is an extension and how many files carry it. A row naming an
/// extension `mit/` has none of fails as loudly as a wrong count: the table
/// is meant to be the directory, not a selection from it.
#[test]
fn the_manual_counts_mits_files_correctly() {
    const MANUAL: &str = include_str!("../site/manual.html");
    let (_, have) = extensions();
    let mut rows = 0;
    for row in MANUAL.split("<td class=\"name\">.").skip(1) {
        // Only the rows of the file-type table: an extension, then a count.
        let Some((extension, rest)) = row.split_once("</td>") else { continue };
        let Some(rest) = rest.strip_prefix("<td class=\"num\">") else { continue };
        let Some((count, _)) = rest.split_once("</td>") else { continue };
        let Ok(said) = count.parse::<usize>() else { continue };
        let counted = have.get(extension).copied().unwrap_or(0);
        assert_eq!(counted, said, "the manual says {said} .{extension} files in mit/");
        rows += 1;
    }
    assert!(rows >= 8, "the file-type table was not found: {rows} rows read");
}

/// **And the total beside it.** `mit/README.md` is this project's own, so
/// MIT's files are everything else under the directory.
#[test]
fn the_manual_counts_mits_files_in_total() {
    const MANUAL: &str = include_str!("../site/manual.html");
    let (files, _) = extensions();
    let said = format!("{files} of them");
    assert!(
        MANUAL.contains(&said),
        "the manual should say {said:?}: mit/ holds that many of MIT's own files"
    );
}
