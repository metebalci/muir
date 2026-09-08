// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The numbers the documents quote about the netlists.
//!
//! `README.md`, `data/README.md` and `site/index.html` each carry a table
//! of the netlists and how many parts are on each board, and
//! `data/README.md` carries the pages and the `part` records with them.
//! **Nothing read any of them.** `tests/parts_mounted.rs` asserts the same
//! figures against the netlists and says in a comment that the documents
//! quote them, but it restates the constants rather than reading them, so
//! the netlists were held to the numbers and the documents to nobody.
//!
//! That fails worse than a document nothing checks at all. Change a
//! netlist, `parts_mounted` fails, somebody updates the constant, and three
//! documents are wrong from that moment **with a green suite saying
//! otherwise**: a second copy that nothing reconciles is worse than no
//! copy, because it looks checked.
//!
//! So the numbers are read out of the documents here and compared with the
//! netlists, which are the thing they are about. Both are committed, so
//! this never skips.

use std::collections::BTreeMap;

use muir::netlist::{self, Netlist};

mod support;

/// The netlists, by the name their row gives: `data/`'s own files, read
/// from disk rather than listed, so that a ninth is covered when it lands.
fn netlists() -> BTreeMap<String, String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
    let mut out = BTreeMap::new();
    for e in std::fs::read_dir(dir).expect("data/ is committed") {
        let p = e.expect("a directory entry").path();
        if p.extension().is_some_and(|x| x == "netlist") {
            let name = p.file_name().expect("a file").to_string_lossy().into_owned();
            out.insert(name, std::fs::read_to_string(&p).expect("a netlist"));
        }
    }
    assert!(out.len() >= 8, "data/ holds the netlists");
    out
}

/// A table read out of a document, as rows of cells keyed by the header.
///
/// **Found by its header rather than by where it is**, so that a document
/// can be rearranged, a column can move, and the padding of a Markdown
/// table can be redone by anything that formats one, without this needing
/// to know. What it cannot survive is a column being *renamed*, which is
/// the right thing to fail on: the claim being made here is about that
/// column.
struct Table {
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    /// The cell in `row` under the column called `name`.
    fn get<'a>(&self, row: &'a [String], name: &str) -> &'a str {
        let k = self.columns.iter().position(|c| c == name);
        let k = k.unwrap_or_else(|| panic!("no column {name:?} in {:?}", self.columns));
        &row[k]
    }

    /// That cell as a number, with the thousands separator a document may
    /// write and a table of numbers usually does not.
    fn number(&self, row: &[String], name: &str) -> usize {
        let cell = self.get(row, name).replace(',', "");
        cell.parse().unwrap_or_else(|_| panic!("{name} is not a number: {cell:?}"))
    }
}

/// The Markdown table whose header row names every one of `columns`.
fn markdown(text: &str, columns: &[&str]) -> Table {
    let cells = |line: &str| -> Vec<String> {
        line.trim()
            .trim_matches('|')
            .split('|')
            .map(|c| c.trim().trim_matches('`').to_string())
            .collect()
    };
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if !line.trim_start().starts_with('|') {
            continue;
        }
        let head = cells(line);
        if !columns.iter().all(|c| head.iter().any(|h| h == c)) {
            continue;
        }
        // The `|---|` rule under the header, then the rows.
        lines.next();
        let mut rows = Vec::new();
        while lines.peek().is_some_and(|l| l.trim_start().starts_with('|')) {
            rows.push(cells(lines.next().expect("peeked")));
        }
        return Table { columns: head, rows };
    }
    panic!("no Markdown table with the columns {columns:?}");
}

/// The same, out of an HTML table: `<th>` for the header and `<td>` for
/// each row, with any markup inside a cell taken off.
fn html(text: &str, columns: &[&str]) -> Table {
    let strip = |cell: &str| -> String {
        let mut out = String::new();
        let mut in_tag = false;
        for c in cell.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                c if !in_tag => out.push(c),
                _ => {}
            }
        }
        out.trim().to_string()
    };
    let cells = |row: &str, tag: &str| -> Vec<String> {
        row.split(&format!("<{tag}"))
            .skip(1)
            .filter_map(|c| c.split_once('>').map(|(_, rest)| rest))
            .filter_map(|c| c.split_once(&format!("</{tag}>")).map(|(cell, _)| strip(cell)))
            .collect()
    };
    let rows: Vec<&str> = text.split("<tr>").skip(1).collect();
    let head = rows
        .iter()
        .map(|r| cells(r, "th"))
        .find(|h| !h.is_empty() && columns.iter().all(|c| h.iter().any(|x| x == c)));
    let head = head.unwrap_or_else(|| panic!("no HTML table with the columns {columns:?}"));
    // Every `<td>` row of that table: the rows before its header belong to
    // an earlier table, and a later `<th>` row begins the next one.
    let after: Vec<&str> = rows
        .iter()
        .skip_while(|r| cells(r, "th") != head)
        .skip(1)
        .take_while(|r| cells(r, "th").is_empty())
        .copied()
        .collect();
    Table { columns: head, rows: after.iter().map(|r| cells(r, "td")).collect() }
}

/// How many devices are mounted on the board a netlist describes, which is
/// what all three documents mean by a part. `tests/parts_mounted.rs` is the
/// account of the three numbers a netlist can be counted by, and the
/// counting itself is `support::parts_on`, shared with it rather than
/// written again here.
///
/// **`CADR.netlist` is two boards in one file and they share designators**,
/// so counting the whole of it as one set of locations merges them: 691
/// where the machine has 985. The control store's pages are counted apart
/// from the processor's, which is what makes the sum right for that file
/// and changes nothing for the seven that are one board each.
fn parts(text: &str) -> usize {
    let n: Netlist = netlist::parse(text).expect("a netlist parses");
    let icmem = support::control_store_pages();
    let on = |store: bool| support::parts_on(&n, |page| icmem.contains(page) == store).len();
    on(false) + on(true)
}

/// **`data/README.md` has a row for every file in `data/`**, which is
/// `CLAUDE.md`'s rule for that directory, and the pages, records and parts
/// in each row are the netlist's.
#[test]
fn the_data_inventory_counts_every_netlist_correctly() {
    const README: &str = include_str!("../data/README.md");
    let t = markdown(README, &["File", "Pages", "Records", "Parts"]);
    let mut seen = 0;
    for netlist in t.rows.iter() {
        let file = t.get(netlist, "File").to_string();
        let Some(text) = netlists().get(&file).cloned() else { continue };
        seen += 1;
        let pages = text.lines().filter(|l| l.starts_with("page ")).count();
        let records = text.lines().filter(|l| l.starts_with("part ")).count();
        assert_eq!(t.number(netlist, "Pages"), pages, "{file}: pages");
        assert_eq!(t.number(netlist, "Records"), records, "{file}: part records");
        assert_eq!(t.number(netlist, "Parts"), parts(&text), "{file}: parts mounted");
    }
    assert_eq!(seen, netlists().len(), "a row for every netlist in data/");
}

/// **The part counts the front page and the README quote are the
/// netlists'.** Neither carries the disk multiplexor, which no engine runs,
/// so what is checked is every row that is there and not that every netlist
/// is a row --- `data/README.md` is the inventory and is held to that
/// above.
#[test]
fn the_readme_and_the_site_quote_the_netlists_own_part_counts() {
    const README: &str = include_str!("../README.md");
    const SITE: &str = include_str!("../site/index.html");
    let files = netlists();
    for (what, t) in [
        ("README.md", markdown(README, &["Netlist", "Board", "Parts"])),
        ("site/index.html", html(SITE, &["Netlist", "Board", "Parts"])),
    ] {
        let mut seen = 0;
        for row in &t.rows {
            let file = t.get(row, "Netlist");
            let text = files.get(file).unwrap_or_else(|| panic!("{what}: no data/{file}"));
            assert_eq!(t.number(row, "Parts"), parts(text), "{what}: {file}");
            seen += 1;
        }
        assert!(seen >= 7, "{what}: {seen} netlists in the table");
    }
}

/// **A count written out in words goes stale in silence**, and this one
/// had: `data/README.md` said "the seven netlists" twice and `README.md`
/// once, after the disk multiplexor made eight. Nothing spelled the number
/// wrong --- the tables were right all along --- so nothing looked.
#[test]
fn the_documents_say_how_many_netlists_there_are_in_words() {
    const WORDS: [&str; 13] = [
        "no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        "eleven", "twelve",
    ];
    let n = netlists().len();
    let right = WORDS.get(n).unwrap_or_else(|| panic!("{n} netlists is past this list"));
    for (what, text) in [
        ("README.md", include_str!("../README.md")),
        ("data/README.md", include_str!("../data/README.md")),
        ("site/index.html", include_str!("../site/index.html")),
    ] {
        for word in WORDS {
            let said = format!("the {word} netlists");
            assert!(
                !text.contains(&said) || word == *right,
                "{what} says {said:?}, and data/ holds {n}"
            );
        }
    }
}
