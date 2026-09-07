// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The disk controller's microassembler, held to the one round trip MIT
//! left: `cadrdc/mksman.39` assembled by `MICRO 52` into `mksman.mcr` and
//! cut into `mksman.d03/d04/d05` on 25 October 1979. Every word and every
//! byte has to come out the same before the assembler is believed on
//! `newdsk.31`, for which nothing assembled survived.

use muir::dcmicro;

mod support;
use support::mit_text;

/// **Every word MIT's assembler made, this one makes.** The listing carries
/// the assembled words as `U` records; the source is what they were made
/// from.
#[test]
fn mksman_assembles_to_mits_listing() {
    let (source, listing) =
        (mit_text(&["cadrdc", "mksman.39"]), mit_text(&["cadrdc", "mksman.mcr"]));
    let asm = dcmicro::assemble(&source).unwrap_or_else(|e| panic!("mksman.39: {e}"));
    let mits = dcmicro::parse_listing(&listing).unwrap();
    assert_eq!(asm.title, "Lisp Machine Marksman Control Microcode");
    assert_eq!(mits.len(), 179, "the listing's trailer says U Words= 179");
    let mut wrong = Vec::new();
    for (&addr, &want) in &mits {
        match asm.words.get(&addr) {
            Some(&got) if got == want => {}
            Some(&got) => wrong.push(format!("{addr:o}: ours {got:08o} MIT {want:08o}")),
            None => wrong.push(format!("{addr:o}: ours none, MIT {want:08o}")),
        }
    }
    for &addr in asm.words.keys() {
        if !mits.contains_key(&addr) {
            wrong.push(format!("{addr:o}: ours {:08o}, MIT none", asm.words[&addr]));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} words differ from MIT's listing:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// **Every byte MIT burned, this one cuts.** The three PROM images beside
/// the listing are `newdsk.trans`'s output; [`dcmicro::Assembly::proms`]
/// is that cut, and [`muir::prom::parse_mit`] reads the images.
#[test]
fn mksman_cuts_to_mits_proms() {
    let source = mit_text(&["cadrdc", "mksman.39"]);
    let asm = dcmicro::assemble(&source).unwrap();
    let ours = asm.proms();
    for (k, name) in ["mksman.d03", "mksman.d04", "mksman.d05"].iter().enumerate() {
        let text = mit_text(&["cadrdc", name]);
        let theirs = muir::prom::parse_mit(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(theirs.len(), 512, "{name} has every location");
        let diffs: Vec<String> = (0..512)
            .filter(|&a| ours[k][a] != theirs[a])
            .map(|a| format!("{a:o}: ours {:o} MIT {:o}", ours[k][a], theirs[a]))
            .collect();
        assert!(diffs.is_empty(), "{name}: {} bytes differ: {diffs:?}", diffs.len());
    }
}

/// **NEWDSK 31 assembles**, to the 163 locations its source assigns, and its
/// fields sit where the DCUI drawing puts the `UIR` bits: every field
/// definition matches a drawn signal, width for width, which is what
/// says this is the board's microcode.
#[test]
fn newdsk_assembles_onto_the_boards_fields() {
    let source = mit_text(&["cadrdc", "newdsk.31"]);
    let asm = dcmicro::assemble(&source).unwrap_or_else(|e| panic!("newdsk.31: {e}"));
    assert_eq!(asm.title, "Lisp Machine Disk Control Microcode");
    assert_eq!(asm.words.len(), 163, "explicitly assigned locations");
    // Name, width, and MIT's L; the bits each occupies follow from
    // `Field::shift`. Every one of these is a signal on the DCUI drawing,
    // matched name for name and width for width against the sheet.
    for (name, width, l, bits) in [
        ("CYLINDER TAG", 1, 0, (23, 23)),
        ("HEAD TAG", 1, 1, (22, 22)),
        ("TAG ENABLE", 1, 2, (21, 21)),
        ("READ GATE", 1, 3, (20, 20)),
        ("WRITE GATE", 1, 4, (19, 19)),
        ("PRE GATE", 1, 5, (18, 18)),
        ("HEADER STROBE", 1, 6, (17, 17)),
        ("DATA FIELD", 1, 7, (16, 16)),
        ("GET DATA", 1, 8, (15, 15)),
        ("ERR IF START BLOCK", 1, 9, (14, 14)),
        ("WRITE", 2, 11, (12, 13)),
        ("ECC", 2, 11, (12, 13)),
        ("DONE TEST", 1, 12, (11, 11)),
        ("CLK", 2, 14, (9, 10)),
        ("UNUSED", 1, 15, (8, 8)),
        ("JUMP", 2, 17, (6, 7)),
        ("LOOP", 3, 20, (3, 5)),
        ("FUNC", 3, 23, (0, 2)),
    ] {
        let f = asm.field(name).unwrap_or_else(|| panic!("no field {name}"));
        assert_eq!((f.width, f.l), (width, l), "{name}: as the source defines it");
        assert_eq!((f.shift(), f.top()), bits, "{name}: which bits of the word");
    }
    // The eighteen fields tile the 24-bit word exactly: every bit claimed
    // once, `WRITE` and `ECC` being one hardware field under two names.
    let mut claimed = 0u32;
    for f in &asm.fields {
        let mask = ((1u32 << f.width) - 1) << f.shift();
        if f.name != "ECC" {
            assert_eq!(claimed & mask, 0, "{} overlaps a field before it", f.name);
            claimed |= mask;
        }
    }
    assert_eq!(claimed, (1 << muir::dcmicro::WORD_BITS) - 1, "no bit of the word is unclaimed");
    // Location 0, the read command's first step: "CYLINDER TAG,CLK/2 USEC,
    // LOOP/ON CYLINDER".
    let w = asm.words[&0];
    assert_eq!(asm.extract(w, "CYLINDER TAG"), Some(1));
    assert_eq!(asm.extract(w, "TAG ENABLE"), Some(0), "cylinder to the bus, not yet to the disk");
    assert_eq!(asm.extract(w, "CLK"), asm.field("CLK").unwrap().values.get("2 USEC").copied());
    assert_eq!(asm.extract(w, "LOOP"), Some(3), "ON CYLINDER");
    // The commands' sectors start where the comment block says.
    for sector in [0u16, 0o100, 0o200, 0o300, 0o400, 0o500, 0o600] {
        assert!(asm.words.contains_key(&sector), "sector at {sector:o} has a first word");
    }
}

/// The images in `data/` are what the assembler makes of `newdsk.31`
/// today, so that the board on the backplane and this test cannot drift.
#[test]
fn the_committed_images_are_newdsks() {
    let source = mit_text(&["cadrdc", "newdsk.31"]);
    let ours = dcmicro::assemble(&source).unwrap().proms();
    for (k, file) in [
        include_str!("../data/newdsk-d03.prom"),
        include_str!("../data/newdsk-d04.prom"),
        include_str!("../data/newdsk-d05.prom"),
    ]
    .iter()
    .enumerate()
    {
        let committed = muir::prom::parse_mit(file).unwrap();
        assert_eq!(committed, ours[k], "data/newdsk-d0{}.prom is stale", k + 3);
    }
}
