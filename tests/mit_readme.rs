// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What `mit/README.md` claims muir does with MIT's drawings.
//!
//! That file is the inventory --- CLAUDE.md §4 --- so its "Read here?"
//! column is a claim about this project's own coverage, and nothing else in
//! the tree would notice it going stale. It did: it listed the DISK
//! MULTIPLEXOR among the drawings nothing reads for as long as it took
//! anyone to look, `data/DM.netlist` having been built from those very
//! sheets. This is the guard, and it is the same shape as `tests/site.rs`.
//!
//! `mit/` and `data/` are both committed, so this never skips.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The body libraries the netlist scripts pass `soap4` with `-e`. They are
/// read --- a page cannot be extracted without the library its own header
/// names --- but they are not pages, so no netlist has one and counting
/// them as unread would be wrong twice over.
const LIBRARIES: [&str; 5] =
    ["cadr/bodies", "cadr/sips", "cadrio/bodies", "cadrtv/bod2", "cadrtv/eclbod"];

/// Every `.drw` under `mit/`, as `<directory>/<name>` with the extension
/// off.
fn drawings() -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).expect("mit/ is committed and always here") {
            let p = e.expect("a directory entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "drw") {
                out.push(p);
            }
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("mit");
    let mut found = Vec::new();
    walk(&root, &mut found);
    found
        .iter()
        .map(|p| p.strip_prefix(&root).unwrap().with_extension("").to_string_lossy().into_owned())
        .collect()
}

/// Every page name any `data/*.netlist` carries, upper case. A drawing
/// becomes a page under its own file name, so this is what "read" means
/// for a sheet.
fn pages() -> BTreeSet<String> {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data");
    let mut out = BTreeSet::new();
    for e in std::fs::read_dir(&data).expect("data/ is committed") {
        let p = e.expect("a directory entry").path();
        if p.extension().is_some_and(|x| x == "netlist") {
            let text = std::fs::read_to_string(&p).expect("a netlist reads");
            out.extend(
                text.lines()
                    .filter_map(|l| l.strip_prefix("page "))
                    .map(|l| l.trim().to_ascii_uppercase()),
            );
        }
    }
    out
}

/// **The drawings no netlist is built from, by the directory they are in.**
///
/// `mit/README.md`'s closing paragraph names these, and the count is what
/// holds the naming honest: a board that starts being extracted takes its
/// sheets out of this table, and a directory that appears in it is one the
/// paragraph has to account for.
///
/// The disk multiplexor is the case this test exists for. `cadrdc` had
/// eleven `dm*` sheets here until `data/DM.netlist` was built from them,
/// and the README went on saying so afterwards; what is left in `cadrdc`
/// now is the MARKSMAN's ten and `dcud`, an earlier DISK CONTROL sheet.
#[test]
fn the_drawings_no_netlist_reads_are_the_ones_the_inventory_names() {
    let pages = pages();
    let mut unread: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for d in drawings() {
        let (dir, name) = d.rsplit_once('/').expect("a drawing is in a directory");
        if pages.contains(&name.to_ascii_uppercase()) || LIBRARIES.contains(&d.as_str()) {
            continue;
        }
        unread.entry(dir.to_string()).or_default().push(name.to_string());
    }
    let counted: BTreeMap<&str, usize> =
        unread.iter().map(|(d, v)| (d.as_str(), v.len())).collect();
    assert_eq!(
        counted,
        BTreeMap::from([
            // Pages outside `cadr.book` and `icmem.book`: earlier
            // revisions and alternates of sheets the print sets name.
            ("cadr", 27),
            // The CDC adaptor's nine `cdc*`, and `blank`, `init`, `memd1`.
            ("cadr1", 12),
            // The MARKSMAN controller's ten `mk*`, and `dcud`.
            ("cadrdc", 11),
            // The keyboard's two `newkb*` and the mouse's two `nmous*`:
            // muir models both from MIT's software rather than their
            // sheets, `io1/ukbd.lisp` and `lmio/kbd.123`.
            ("cadrio", 4),
            // The memory board's eight `mcp*` of bypass capacitors and its
            // six `pm*`.
            ("cadrm", 14),
            // `bod1`, an earlier body library than the `bod2` the sheets
            // name, and `lmram`.
            ("cadrtv", 2),
            ("cadrtv/lispm-tv", 1),
            ("cadrtv/simple-tv", 4),
            // Two of the Chaosnet library's sheets that no board's page
            // list names.
            ("chaos/lispm", 2),
        ]),
        "the drawings nothing reads; `mit/README.md`'s last paragraph names them"
    );
    // Not vacuous the other way either: most of `mit/`'s drawings *are*
    // read, and a change that stopped extracting a board would show here
    // before it showed anywhere else.
    let all = drawings().len();
    let short = unread.values().map(Vec::len).sum::<usize>();
    assert_eq!((all - short, all), (274, 351), "read, and every drawing there is");

    // And the README says the same, as `tests/site.rs` holds the manual to
    // its counts. A number in prose drifts; a number a test reads back does
    // not.
    const README: &str = include_str!("../mit/README.md");
    let said = format!("{short} of the {all}");
    assert!(README.contains(&said), "mit/README.md should say {said:?} drawings go unread");
}
