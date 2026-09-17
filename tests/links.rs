// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every link between this repository's own documents leads somewhere.
//!
//! `README.md` is the way in and `docs/` is where the detail is, so most of
//! what a reader follows is a link from one file to another, often to a
//! heading inside it. **A link that leads nowhere fails in silence**: GitHub
//! renders it, the reader lands at the top of a file or on a 404, and no
//! build notices. A heading renamed in `docs/manual.md` breaks every
//! `#anchor` that pointed at it, in whichever file.
//!
//! So each committed Markdown file is read here, and each relative link in
//! it is held to a file that exists and, when it names one, to a heading that
//! file has. The front page links into `docs/` by its GitHub address, and
//! those are held the same way. Links out of the repository are not
//! followed: whether another site is up is not something a test here can
//! settle. Everything read is committed, so this never skips.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// This project's own Markdown: every `.md` file outside `mit/sys/`, which is
/// the release's tree and not this project's writing, `target/`, and the
/// directories never committed, hidden ones among them.
fn documents() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).expect("a readable directory") {
            let p = e.expect("a directory entry").path();
            let rel = p.strip_prefix(root()).expect("under the root");
            let skip = ["target", "vendor", "notes", "mit/sys"];
            let hidden = p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
            if hidden || skip.iter().any(|s| rel == Path::new(s)) {
                continue;
            }
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "md") && rel != Path::new("CLAUDE.md") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(root(), &mut out);
    out.sort();
    out
}

/// The lines of a Markdown file that are prose rather than code: a fenced
/// block is skipped whole, and a line indented four spaces is an indented
/// code block. Inline code spans are blanked out, so that `[a](b)` written
/// as an example in backticks is not taken for a link.
fn prose(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced || line.starts_with("    ") || line.starts_with('\t') {
            continue;
        }
        let mut kept = String::new();
        for (k, part) in line.split('`').enumerate() {
            kept.push_str(if k % 2 == 0 { part } else { " " });
        }
        out.push(kept);
    }
    out
}

/// The anchor GitHub gives a heading: its text as rendered, lower case,
/// with everything but letters, digits, hyphens, underscores and spaces
/// taken out and each space made a hyphen. A second heading with the same
/// anchor gets `-1`, the third `-2`.
fn anchors(text: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        let hashes = line.chars().take_while(|&c| c == '#').count();
        if fenced || !(1..=6).contains(&hashes) || !line[hashes..].starts_with(' ') {
            continue;
        }
        // A link in a heading renders as its text.
        let mut heading = String::new();
        let mut rest = &line[hashes..];
        while let Some(open) = rest.find("](") {
            let close = rest[open..].find(')').map(|c| open + c + 1).unwrap_or(rest.len());
            heading.push_str(&rest[..open]);
            rest = &rest[close..];
        }
        heading.push_str(rest);
        let slug: String = heading
            .trim()
            .to_lowercase()
            .chars()
            .filter(|&c| c.is_alphanumeric() || c == '-' || c == '_' || c == ' ')
            .map(|c| if c == ' ' { '-' } else { c })
            .collect();
        let mut name = slug.clone();
        let mut n = 0;
        while seen.contains(&name) {
            n += 1;
            name = format!("{slug}-{n}");
        }
        seen.insert(name);
    }
    seen
}

/// The target of every inline link on a line, `[text](target)`, with a
/// title after it dropped.
fn targets(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find("](") {
        rest = &rest[at + 2..];
        let end = rest.find(')').unwrap_or(rest.len());
        let target = rest[..end].split_whitespace().next().unwrap_or("");
        out.push(target.to_string());
        rest = &rest[end..];
    }
    out
}

/// Whether `target`, written in the file at `from`, leads to something: the
/// file or directory, and the heading when an anchor names one. `None` when
/// it does, and what is wrong when it does not.
fn broken(from: &Path, target: &str) -> Option<String> {
    let (path, anchor) = match target.split_once('#') {
        Some((p, a)) => (p, Some(a)),
        None => (target, None),
    };
    let file = if path.is_empty() {
        from.to_path_buf()
    } else {
        from.parent().expect("a file is in a directory").join(path)
    };
    if !file.exists() {
        return Some(format!("no {path}"));
    }
    let anchor = anchor?;
    if file.extension().is_none_or(|x| x != "md") {
        return None;
    }
    let text = std::fs::read_to_string(&file).expect("a readable document");
    if anchors(&text).contains(anchor) { None } else { Some(format!("no heading #{anchor}")) }
}

fn external(target: &str) -> bool {
    ["http://", "https://", "mailto:"].iter().any(|s| target.starts_with(s))
}

/// **The anchor rule is GitHub's**, checked on headings whose anchors the
/// documents already link to, so that the test below is not passing because
/// every anchor it computes is wrong in the same way.
#[test]
fn a_heading_anchor_is_the_one_github_makes() {
    let text = "# muir manual\n\
                ## Running it\n\
                ### `--watch <from>[-<to>]:<net>,<net>,...`\n\
                ### `continue, c`\n\
                ```text\n# not a heading\n```\n\
                ## Running it\n";
    let want = ["muir-manual", "running-it", "--watch-from-tonetnet", "continue-c", "running-it-1"];
    assert_eq!(anchors(text), want.iter().map(|s| s.to_string()).collect());
}

/// **Every relative link in this project's Markdown leads to a file that is
/// there, and to a heading that file has.**
#[test]
fn every_link_between_the_documents_leads_somewhere() {
    let mut wrong = Vec::new();
    let mut checked = 0;
    for doc in documents() {
        let text = std::fs::read_to_string(&doc).expect("a readable document");
        for line in prose(&text) {
            for target in targets(&line) {
                if target.is_empty() || external(&target) {
                    continue;
                }
                checked += 1;
                if let Some(why) = broken(&doc, &target) {
                    let rel = doc.strip_prefix(root()).expect("under the root");
                    wrong.push(format!("{}: ({target}): {why}", rel.display()));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "links that lead nowhere:\n{}", wrong.join("\n"));
    assert!(checked > 20, "{checked} links read: the reader is missing them");
}

/// **The front page's links into the repository lead somewhere too.** It
/// is served on its own site, so it names each file by its GitHub address;
/// the part after `blob/main/` or `tree/main/` is a path here, and an
/// anchor on the repository's own address is a heading of `README.md`,
/// which is the page GitHub shows there.
#[test]
fn the_front_page_links_into_the_repository_lead_somewhere() {
    let page = root().join("pages/index.html");
    let text = std::fs::read_to_string(&page).expect("the front page is committed");
    let mut wrong = Vec::new();
    let mut checked = 0;
    for (prefix, from, lead) in [
        ("https://github.com/metebalci/muir/blob/main/", "index", ""),
        ("https://github.com/metebalci/muir/tree/main/", "index", ""),
        ("https://github.com/metebalci/muir#", "README.md", "#"),
    ] {
        for piece in text.split(prefix).skip(1) {
            let target = format!("{lead}{}", &piece[..piece.find('"').expect("an attribute ends")]);
            checked += 1;
            // Resolved as if written in a file at the root.
            if let Some(why) = broken(&root().join(from), &target) {
                wrong.push(format!("pages/index.html: {prefix}: {target}: {why}"));
            }
        }
    }
    assert!(wrong.is_empty(), "links that lead nowhere:\n{}", wrong.join("\n"));
    assert!(checked > 5, "{checked} links read: the reader is missing them");
}
