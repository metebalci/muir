// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `diskpack`: the label it writes, the table it lays out, and what it puts
//! in a partition.
//!
//! The checks that need no pack come first, and they are the ones that hold
//! the tool to MIT's own software: a fresh label is `LE-INITIALIZE-LABEL`'s,
//! partition for partition.  The ones after them need the vendored pack, and
//! they are the ones that hold it to a pack MIT's own software wrote.

mod support;

use std::path::{Path, PathBuf};

use muir::band::{BLOCK_WORDS, Label, T300};
use muir::diskpack::{Command, Pack, Size, parse, parse_size};
use muir::engine::Engine;
use support::{scratch, vendor};

/// Bytes in a block, for the sizes the tests work in.
const BLOCK_BYTES: u64 = BLOCK_WORDS as u64 * 4;

/// The table a fresh label gets: name, first block, blocks.
fn table() -> Vec<(String, u32, u32)> {
    let label = Label::initialize(Path::new("nowhere.img"), &T300);
    label.partitions.iter().map(|p| (p.name.clone(), p.start, p.blocks)).collect()
}

fn rows(want: &[(&str, u32, u32)]) -> Vec<(String, u32, u32)> {
    want.iter().map(|(n, s, b)| (n.to_string(), *s, *b)).collect()
}

/// A pack of the tool's own, in a directory of the test's own.
fn pack_at(dir: &Path, name: &str) -> (Pack, PathBuf) {
    let path = dir.join(name);
    let (pack, _) = Pack::open(&path);
    (pack, path)
}

/// **A fresh label is the one MIT's editor writes.**
///
/// `LE-INITIALIZE-LABEL` lays the partitions out from block 17 --- one track
/// reserved --- with a size in blocks taken as it stands and a size in
/// cylinders starting at a cylinder boundary.  The numbers here are that
/// table worked out by hand from `PACK-TYPES`, and the check that they came
/// across right is at the end of it: `FILE` ends at block 263,245, the last
/// block of a T-300, so the table fills the pack exactly.
#[test]
fn a_fresh_label_is_mits_own_table() {
    let want = rows(&[
        ("MCR1", 17, 148),
        ("MCR2", 165, 148),
        ("MCR3", 313, 148),
        ("MCR4", 461, 148),
        ("MCR5", 609, 148),
        ("MCR6", 757, 148),
        ("MCR7", 905, 148),
        ("MCR8", 1053, 148),
        // 202 cylinders, and the start rounded up to a cylinder boundary:
        // 1201 is in cylinder 3, so PAGE begins at 4 * 323.
        ("PAGE", 1292, 65246),
        ("LOD1", 66538, 24225),
        ("LOD2", 90763, 24225),
        ("LOD3", 114988, 24225),
        ("LOD4", 139213, 24225),
        ("LOD5", 163438, 24225),
        ("LOD6", 187663, 24225),
        ("LOD7", 211888, 24225),
        ("LOD8", 236113, 24225),
        ("FILE", 260338, 2907),
    ]);
    assert_eq!(table(), want);

    let label = Label::initialize(Path::new("nowhere.img"), &T300);
    let last = label.partitions.last().unwrap();
    assert_eq!(last.start + last.blocks, label.blocks(), "the table fills a T-300 exactly");
    assert_eq!(label.blocks(), 815 * 19 * 17);
    assert_eq!(label.microload_partition, "MCR1", "what LE-INITIALIZE-LABEL puts in word 6");
    assert_eq!(label.current_band, "LOD1", "and in word 7");
    assert_eq!(label.drive, "Trident T-300");
    assert_eq!(label.words_per_descriptor, 7);
}

/// **A label written and read back is the same label.**
///
/// Every field, through the words of block 0: the writer and the reader are
/// each other's check, and the fields MIT's editor writes that muir had never
/// read before --- the current band, the drive, the pack's name and the
/// comment --- come back with the rest.
#[test]
fn a_label_written_is_the_label_read_back() {
    let dir = scratch("diskpack-round-trip");
    let path = dir.join("pack.img");
    let mut label = Label::initialize(&path, &T300);
    label.pack_name = "MIT-LISPM-2".to_string();
    label.comment = "a pack of one's own".to_string();
    label.current_band = "LOD2".to_string();
    label.partitions[0].comment = "UCADR 323".to_string();
    label.write().unwrap();

    let back = Label::open(&path).unwrap();
    assert_eq!(back.pack_name, "MIT-LISPM-2");
    assert_eq!(back.comment, "a pack of one's own");
    assert_eq!(back.drive, "Trident T-300");
    assert_eq!(back.microload_partition, "MCR1");
    assert_eq!(back.current_band, "LOD2");
    assert_eq!(back.cylinders, 815);
    assert_eq!(back.heads, 19);
    assert_eq!(back.blocks_per_track, 17);
    assert_eq!(back.blocks_per_cylinder, 323);
    assert_eq!(back.partitions[0].comment, "UCADR 323");
    assert_eq!(
        back.partitions.iter().map(|p| (p.name.clone(), p.start, p.blocks)).collect::<Vec<_>>(),
        table()
    );

    // The file is the size the geometry says, which is what `Unit::open`
    // insists on before it will attach a pack to a drive.
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 815 * 19 * 17 * BLOCK_BYTES);
}

/// **A size is blocks, cylinders, a percentage, the rest of the pack, or the
/// size it already has.**
#[test]
fn a_size_is_read_five_ways() {
    assert_eq!(parse_size("1234").unwrap(), Size::Blocks(1234));
    assert_eq!(parse_size("75c").unwrap(), Size::Cylinders(75));
    assert_eq!(parse_size("25%").unwrap(), Size::Percent(25));
    assert_eq!(parse_size("rest").unwrap(), Size::Rest);
    assert_eq!(parse_size("keep").unwrap(), Size::Keep);
    for no in ["", "-1", "101%", "0%", "12x", "c", "%"] {
        assert!(parse_size(no).is_err(), "{no} is no size");
    }
    // `keep` is the size it has, and a partition being added has none.
    assert!(parse("partition LOD9 keep").is_err());
    assert!(parse("modify LOD1 keep").is_ok());
}

/// **A command is the whole rest of the line where it takes text.**
///
/// A comment has spaces in it and so does a path, and neither is several
/// arguments.  `load-from` is the one with a path in the middle: their
/// partition is the last word and ours the first, so what is between them is
/// the pack, spaces and all.
#[test]
fn a_command_takes_the_rest_of_the_line() {
    assert_eq!(
        parse("comment System 100, again").unwrap().unwrap(),
        Command::Comment("System 100, again".to_string())
    );
    assert_eq!(
        parse("partition LOD1 75c cold 3/23/23").unwrap().unwrap(),
        Command::Partition {
            name: "LOD1".to_string(),
            size: Size::Cylinders(75),
            comment: Some("cold 3/23/23".to_string()),
        }
    );
    assert_eq!(
        parse("dump LOD1 /tmp/a band.img").unwrap().unwrap(),
        Command::Dump { partition: "LOD1".to_string(), file: PathBuf::from("/tmp/a band.img") }
    );
    assert_eq!(
        parse("load-from 1 /tmp/a pack.img LOD2").unwrap().unwrap(),
        Command::LoadFrom {
            partition: "LOD1".to_string(),
            pack: PathBuf::from("/tmp/a pack.img"),
            from: "LOD2".to_string(),
        }
    );
    assert_eq!(parse("   ").unwrap(), None);
    assert!(parse("frobnicate").is_err());
    assert!(parse("show me").is_err(), "show takes nothing");
    assert!(parse("load-from LOD1 pack.img").is_err(), "three things or none");
}

/// **Every command that has a short form means the same thing by it, and
/// `current` is the label's own two fields.**
///
/// MIT's display gives them as "Current microload = MCR1, current virtual
/// memory load (band) = LOD2", and `SET-CURRENT-BAND` takes the number first
/// and then whether it is a microload, so `current 2` is the band `LOD2` and
/// `current 2 mcr` the microload `MCR2`.
#[test]
fn every_short_form_means_the_same_thing() {
    for (long, short) in [
        ("initialize", "i"),
        ("show", "s"),
        ("current", "c"),
        ("current 2", "c 2"),
        ("partition LOD1 75c", "p LOD1 75c"),
        ("modify LOD1 75c", "m LOD1 75c"),
        ("load LOD1 band.dump", "l LOD1 band.dump"),
        ("delete LOD1", "d LOD1"),
        ("quit", "q"),
        ("help", "h"),
    ] {
        let (a, b) = (parse(long).unwrap(), parse(short).unwrap());
        assert_eq!(a, b, "{long} and {short} are not the same command");
        assert!(a.is_some(), "{long} is no command");
    }
    assert_eq!(parse("?").unwrap().unwrap(), Command::Help);
    assert_eq!(parse("c").unwrap().unwrap(), Command::Current, "c is current, not comment");

    // Four have no short form: the two that are short words already, the one
    // that is two packs at once, and the one that is rare enough not to want
    // a second letter for.
    for no in ["dr Trident T-300", "n MIT-LISPM-2", "lf LOD1 p.img LOD1", "du LOD1 band.dump"] {
        assert!(parse(no).is_err(), "{no} is no command");
    }
    // `d` is delete and `du` is dump, which are next to each other on the
    // keyboard and one of them cannot be undone: a dump that is typed `d`
    // names a partition and a file, and delete takes the whole rest of the
    // line as one name, so it finds no partition of that name rather than
    // deleting the one at the front of it.
    assert_eq!(parse("d LOD1").unwrap().unwrap(), Command::Delete("LOD1".to_string()));
    assert_eq!(
        parse("d LOD1 band.dump").unwrap().unwrap(),
        Command::Delete("LOD1 BAND.DUMP".to_string())
    );

    let band = |n: &str| Command::Band(n.to_string());
    let microload = |n: &str| Command::Microload(n.to_string());
    assert_eq!(parse("current").unwrap().unwrap(), Command::Current);
    assert_eq!(parse("current 2").unwrap().unwrap(), band("LOD2"));
    // A name says which of the two words it goes in by what it is called,
    // those being the two kinds of partition a CADR pack has.  So a microload
    // is always written out and never reached by a number.
    assert_eq!(parse("current MCR3").unwrap().unwrap(), microload("MCR3"));
    assert_eq!(parse("current lod1").unwrap().unwrap(), band("LOD1"));
    assert!(parse("current PAGE").unwrap_err().contains("is not an MCR or a LOD partition"));
    assert!(parse("current 2 mcr").is_err(), "no keyword after it: MCR2 is written out");
    assert!(parse("current LOD1 LOD2").is_err(), "one partition at a time");

    // A number is a band wherever a partition is asked for, as it is there.
    assert_eq!(
        parse("load 1 band.dump").unwrap().unwrap(),
        Command::Load { partition: "LOD1".to_string(), file: Some(PathBuf::from("band.dump")) }
    );
    assert_eq!(parse("d 3").unwrap().unwrap(), Command::Delete("LOD3".to_string()));
    assert_eq!(
        parse("dump 2 band.dump").unwrap().unwrap(),
        Command::Dump { partition: "LOD2".to_string(), file: PathBuf::from("band.dump") }
    );
    assert_eq!(
        parse("m 4 75c").unwrap().unwrap(),
        Command::Modify { name: "LOD4".to_string(), size: Size::Cylinders(75), comment: None }
    );
    assert_eq!(
        parse("p 9 75c").unwrap().unwrap(),
        Command::Partition { name: "LOD9".to_string(), size: Size::Cylinders(75), comment: None }
    );
    // And a name is the name, upper case as it is on the pack.
    assert_eq!(parse("delete mcr2").unwrap().unwrap(), Command::Delete("MCR2".to_string()));
    assert_eq!(
        parse("l 1").unwrap().unwrap(),
        Command::Load { partition: "LOD1".to_string(), file: None }
    );
}

/// **A partition is added once, and changed after that.**
///
/// Two commands, because they are two things: adding one to a table with room
/// in it does nothing to what is already on the pack, and growing one moves
/// the table under bands that do not move with it.
#[test]
fn a_partition_is_added_once_and_changed_after_that() {
    let dir = scratch("diskpack-add");
    let (mut pack, _) = pack_at(&dir, "pack.img");
    pack.run(Command::Initialize).unwrap();

    let add = |size| Command::Partition { name: "MCR1".to_string(), size, comment: None };
    let e = pack.run(add(Size::Blocks(148))).unwrap_err();
    assert!(e.contains("MCR1 is on the pack already: modify changes one"), "{e}");

    let grow = |name: &str, size| Command::Modify { name: name.to_string(), size, comment: None };
    let e = pack.run(grow("LOD9", Size::Blocks(1))).unwrap_err();
    assert!(e.contains("no partition named LOD9"), "{e}");

    // And it only grows: a partition that shrank would leave the band in it
    // running off the end, with nothing in the label to say so.
    let e = pack.run(grow("FILE", Size::Blocks(2617))).unwrap_err();
    assert!(e.contains("FILE is 2907 blocks and this would make it 2617"), "{e}");
    assert!(e.contains("delete is how one gives blocks back"), "{e}");
}

/// **Growing a partition moves the ones after it, and says so; one that would
/// not fit changes nothing.**
///
/// The table moves and the blocks do not, which is the whole hazard of
/// editing a label under a band that is already there, so the tool says which
/// partitions moved rather than leaving it to be discovered.  And since every
/// command is written as it runs, one whose table could not be written has to
/// leave the label exactly as it was.
#[test]
fn growing_moves_what_follows_and_says_so() {
    let dir = scratch("diskpack-grow");
    let (mut pack, path) = pack_at(&dir, "pack.img");
    pack.run(Command::Initialize).unwrap();

    // The System 100 pack's PAGE is 65536 blocks, 290 more than the 65246
    // that MIT's 202 cylinders come to.  Growing PAGE into a full table
    // pushes FILE off the end of the pack, so it is refused and nothing moves.
    let grow_page =
        || Command::Modify { name: "PAGE".to_string(), size: Size::Blocks(65536), comment: None };
    let e = pack.run(grow_page()).unwrap_err();
    assert!(e.contains("FILE ends at block 263535 and the pack has 263245"), "{e}");
    assert_eq!(
        Label::open(&path).unwrap().partition("PAGE").unwrap().blocks,
        65246,
        "the refused command left the pack as it was"
    );

    // FILE gives the blocks back the way blocks are given back, and comes
    // again 290 blocks shorter; then PAGE takes them, and the table is the
    // System 100 pack's own.
    pack.run(Command::Delete("FILE".to_string())).unwrap();
    pack.run(Command::Partition {
        name: "FILE".to_string(),
        size: Size::Blocks(2617),
        comment: None,
    })
    .unwrap();
    let said = pack.run(grow_page()).unwrap();
    assert!(said.contains("LOD1 moves from 66538 to 66828"), "{said}");
    assert!(said.contains("what is in a partition does not move with it"), "{said}");

    let back = Label::open(&path).unwrap();
    let file = back.partition("FILE").unwrap();
    assert_eq!((file.start, file.blocks), (260628, 2617));
    assert_eq!(file.start + file.blocks, back.blocks(), "and it ends at the last block");
}

/// **A percentage is of the whole pack, and the rest is what is left.**
#[test]
fn a_percentage_and_the_rest_fill_the_pack() {
    let dir = scratch("diskpack-percent");
    let path = dir.join("pack.img");
    // A T-300 with no partitions on it: MIT's table fills the pack, so a
    // table of one's own starts from an empty one.
    let mut empty = Label::initialize(&path, &T300);
    empty.partitions.clear();
    empty.write().unwrap();

    let (mut pack, _) = Pack::open(&path);
    let add = |name: &str, size| Command::Partition { name: name.to_string(), size, comment: None };
    pack.run(add("MCR1", Size::Blocks(148))).unwrap();
    pack.run(add("PAGE", Size::Percent(50))).unwrap();
    pack.run(add("LOD1", Size::Rest)).unwrap();

    let back = Label::open(&path).unwrap();
    let table: Vec<(String, u32, u32)> =
        back.partitions.iter().map(|p| (p.name.clone(), p.start, p.blocks)).collect();
    assert_eq!(
        table,
        rows(&[
            // The first block after the reserved track, and no rounding: a
            // size in blocks is laid where it falls.
            ("MCR1", 17, 148),
            // Half of 263245 is 131622, which is 407 cylinders and 161 blocks
            // over; a percentage is whole cylinders, so 131461 --- and one
            // that is whole cylinders starts on a cylinder boundary, so PAGE
            // begins at 323 rather than at the 165 MCR1 ends on.
            ("PAGE", 323, 131461),
            ("LOD1", 131784, 131461),
        ])
    );
    let last = back.partitions.last().unwrap();
    assert_eq!(last.start + last.blocks, back.blocks(), "the rest reaches the last block");
}

/// **Deleting a partition frees its blocks where they are and moves
/// nothing.**
///
/// The table is not the pack: moving `MCR3` down onto the blocks `MCR2` used
/// would not move what is in `MCR3` with it.  So a delete leaves a hole, and
/// the hole is shown.
#[test]
fn deleting_frees_its_blocks_and_moves_nothing() {
    let dir = scratch("diskpack-delete");
    let (mut pack, _) = pack_at(&dir, "pack.img");
    pack.run(Command::Initialize).unwrap();
    let said = pack.run(Command::Delete("MCR2".to_string())).unwrap();
    assert!(said.contains("MCR2 is gone: 148 blocks at 165"), "{said}");
    assert!(!said.contains("moves from"), "nothing moved: {said}");
    let shown = pack.run(Command::Show).unwrap();
    assert!(shown.contains("MCR3 at block      313"), "{shown}");
    assert!(shown.contains("148 blocks free at 165"), "the hole: {shown}");
    assert!(pack.run(Command::Delete("MCR2".to_string())).is_err(), "gone twice");
}

/// **Changing a comment moves nothing.**
///
/// `keep` is the size it has, and a partition whose size did not change
/// cannot push the one after it anywhere --- including the 91 blocks MIT's
/// own table leaves before `PAGE`, which is there because `PAGE` starts at a
/// cylinder boundary and which an edit somewhere else must not close.
#[test]
fn a_comment_moves_nothing() {
    let dir = scratch("diskpack-comment");
    let (mut pack, _) = pack_at(&dir, "pack.img");
    pack.run(Command::Initialize).unwrap();
    let said = pack
        .run(Command::Modify {
            name: "MCR1".to_string(),
            size: Size::Keep,
            comment: Some("UCADR 323".to_string()),
        })
        .unwrap();
    assert!(said.contains("\"UCADR 323\""), "{said}");
    assert!(!said.contains("moves from"), "nothing moved: {said}");
    assert!(pack.run(Command::Show).unwrap().contains("91 blocks free at 1201"));
}

/// **A partition the pack does not have is refused, and so is a pack that has
/// no label.**
#[test]
fn what_is_not_there_is_refused() {
    let dir = scratch("diskpack-refused");
    let path = dir.join("pack.img");
    let (mut pack, said) = Pack::open(&path);
    assert!(said.contains("is not there"), "{said}");

    // Nothing to show, and nowhere to put blocks, until there is a label.
    assert!(pack.run(Command::Show).unwrap_err().contains("no label yet"));
    let e = pack
        .run(Command::Load { partition: "LOD1".to_string(), file: Some(PathBuf::from("nothing")) })
        .unwrap_err();
    assert!(e.contains("there is no label yet"), "{e}");

    pack.run(Command::Initialize).unwrap();
    let e = pack.run(Command::Microload("MCR9".to_string())).unwrap_err();
    assert!(e.contains("no partition named MCR9"), "{e}");
    assert!(e.contains("MCR1 MCR2 MCR3"), "it says what there is: {e}");

    // A name the label cannot hold is refused where it is typed rather than
    // at the write, which is where the label would have had to cut it.
    let e = pack
        .run(Command::Partition {
            name: "SWAPPING".to_string(),
            size: Size::Blocks(1),
            comment: None,
        })
        .unwrap_err();
    assert!(e.contains("four ASCII characters or fewer"), "{e}");

    // And a dump does not write over what is already there.
    let there = dir.join("already");
    std::fs::write(&there, b"not a band").unwrap();
    let e = pack.run(Command::Dump { partition: "LOD1".to_string(), file: there }).unwrap_err();
    assert!(e.contains("is there already"), "{e}");
}

/// **A microcode partition takes a microcode file and nothing else.**
///
/// What goes into an `MCR` partition is written the way the machine reads it,
/// every word's halves swapped, so a file that is not microcode would be
/// swapped into nonsense.  The partition says what it holds, and the file is
/// read to see that it is that.
#[test]
fn a_microcode_partition_takes_a_microcode_file() {
    let dir = scratch("diskpack-not-microcode");
    let (mut pack, _) = pack_at(&dir, "pack.img");
    pack.run(Command::Initialize).unwrap();

    let not_microcode = dir.join("band.dump");
    std::fs::write(&not_microcode, [0u8; 1024]).unwrap();
    let e = pack
        .run(Command::Load { partition: "MCR1".to_string(), file: Some(not_microcode.clone()) })
        .unwrap_err();
    assert!(e.contains("MCR1 holds microcode and"), "{e}");

    // The same file into a partition that holds anything is fine.
    let said = pack
        .run(Command::Load { partition: "LOD1".to_string(), file: Some(not_microcode) })
        .unwrap();
    assert!(said.contains("1 blocks of 24225 written"), "{said}");
}

// --- what needs the vendored pack -------------------------------------------

/// **The label of the System 100 pack, written back, is the same 1024
/// bytes.**
///
/// The distribution pack's label was written by MIT's own software.  Reading
/// it and writing it out again word for word is the strongest check there is
/// that this writer lays a label out where the machine expects to find it:
/// every field, the partition table, the padding between them.
#[test]
fn the_system_100_label_written_back_is_the_same_bytes() {
    let Some(pack) = vendor(&["run", "disk-sys-100-0.img"]) else { return };
    let dir = scratch("diskpack-same-bytes");
    let path = dir.join("copy.img");

    let label = Label::open(&pack).unwrap();
    label.on(&path).write().unwrap();

    let want = std::fs::read(&pack).unwrap()[..1024].to_vec();
    let got = std::fs::read(&path).unwrap()[..1024].to_vec();
    assert!(got == want, "block 0 of the copy is not block 0 of the pack");

    // And it is the distribution pack's own table, not MIT's T-300 one.
    assert_eq!(label.pack_name, "MIT-LISPM-1");
    assert_eq!(label.comment, "System 100 Distribution Tape");
    assert_eq!(label.current_band, "LOD2");
    assert_eq!(label.partition("PAGE").unwrap().blocks, 65536, "not the 65246 of 202 cylinders");
}

/// **The microcode this puts in a partition is the microcode the pack has.**
///
/// A fresh pack, the built-in microcode written into `MCR1`, and the result
/// compared with `MCR1` on the pack MIT's own software made: all 148 blocks
/// of it, byte for byte.  That is what settles the halving ---
/// a microcode file holds each word's two 16-bit pieces high one first, and
/// the disk holds a word low half first --- and it settles the zeroing of the
/// rest of the partition with it.
#[test]
fn the_microcode_written_is_the_microcode_on_the_pack() {
    let Some(pack) = vendor(&["run", "disk-sys-100-0.img"]) else { return };
    let dir = scratch("diskpack-microcode");
    let (mut ours, path) = pack_at(&dir, "fresh.img");
    ours.run(Command::Initialize).unwrap();
    let said = ours
        .run(Command::Load { partition: "MCR1".to_string(), file: None })
        .expect("the built-in microcode into MCR1");
    assert!(said.contains("12449 control store words"), "{said}");

    // MCR1 is at block 17 on both: MIT's own table and this one agree there,
    // which is what makes the comparison a comparison of the contents.
    let theirs = Label::open(&pack).unwrap();
    let mcr1 = theirs.partition("MCR1").unwrap();
    assert_eq!((mcr1.start, mcr1.blocks), (17, 148));
    let at = mcr1.start as usize * BLOCK_WORDS * 4;
    let len = mcr1.blocks as usize * BLOCK_WORDS * 4;
    let want = &std::fs::read(&pack).unwrap()[at..at + len];
    let got = &std::fs::read(&path).unwrap()[at..at + len];
    assert!(got == want, "MCR1 is not what the System 100 pack has in MCR1");

    // And the same file named explicitly is the same write again.
    let named = dir.join("ucadr.mcr");
    std::fs::write(&named, muir::mcr::UCADR_323).unwrap();
    ours.run(Command::Load { partition: "MCR2".to_string(), file: Some(named) })
        .expect("the same microcode, named rather than built in");
    let got = std::fs::read(&path).unwrap();
    let mcr2 = theirs.partition("MCR2").unwrap();
    let at2 = mcr2.start as usize * BLOCK_WORDS * 4;
    assert!(got[at2..at2 + len] == got[at..at + len], "MCR2 is not MCR1");
}

/// **A band dumped out of the pack and loaded into a fresh one arrives.**
///
/// `LOD1` is 24,225 blocks, so this moves 24 MB twice; it is the path a pack
/// of one's own is made by, and the check is that the blocks are the blocks.
#[test]
fn a_band_dumped_and_loaded_arrives() {
    let Some(pack) = vendor(&["run", "disk-sys-100-0.img"]) else { return };
    let dir = scratch("diskpack-band");
    let dump = dir.join("lod1.dump");

    let (mut theirs, _) = Pack::open(&pack);
    theirs.run(Command::Dump { partition: "LOD1".to_string(), file: dump.clone() }).unwrap();
    assert_eq!(std::fs::metadata(&dump).unwrap().len(), 24225 * BLOCK_BYTES, "the whole partition");

    let (mut ours, path) = pack_at(&dir, "fresh.img");
    ours.run(Command::Initialize).unwrap();
    ours.run(Command::Load { partition: "LOD1".to_string(), file: Some(dump) }).unwrap();

    // The band reads as a band: this is the same reader `examples/band.rs`
    // uses, so what it says of the copy is what it says of the pack.
    let ours = Label::open(&path).unwrap();
    let band = ours.band("LOD1").unwrap();
    assert!(!band.compressed(), "LOD1 is the uncompressed format");
    assert_eq!(band.valid_size(), Label::open(&pack).unwrap().band("LOD1").unwrap().valid_size());
}

/// **A band copied straight out of another pack arrives, and one too big for
/// where it is going is refused.**
///
/// The same move as a dump and a load, without the 24 MB file in between:
/// both sides are packs, so the words are already the way the controller
/// reads them and nothing is swapped.
#[test]
fn a_band_copied_from_another_pack_arrives() {
    let Some(theirs) = vendor(&["run", "disk-sys-100-0.img"]) else { return };
    let dir = scratch("diskpack-load-from");
    let (mut ours, path) = pack_at(&dir, "fresh.img");
    ours.run(Command::Initialize).unwrap();

    let said = ours
        .run(Command::LoadFrom {
            partition: "LOD1".to_string(),
            pack: theirs.clone(),
            from: "LOD1".to_string(),
        })
        .unwrap();
    assert!(said.contains("24225 blocks of 24225"), "{said}");

    let band = Label::open(&path).unwrap().band("LOD1").unwrap();
    assert_eq!(band.valid_size(), Label::open(&theirs).unwrap().band("LOD1").unwrap().valid_size());

    // Their PAGE is 65536 blocks and our LOD1 holds 24225: a partition that
    // arrived cut short would be a band with no end.
    let e = ours
        .run(Command::LoadFrom {
            partition: "LOD1".to_string(),
            pack: theirs.clone(),
            from: "PAGE".to_string(),
        })
        .unwrap_err();
    assert!(e.contains("PAGE is 65536 blocks and LOD1 holds 24225"), "{e}");

    let e = ours
        .run(Command::LoadFrom {
            partition: "LOD1".to_string(),
            pack: theirs,
            from: "LOD9".to_string(),
        })
        .unwrap_err();
    assert!(e.contains("no partition named LOD9"), "{e}");
}

/// **A pack made here boots.**
///
/// The whole point of the tool, and the one check that goes through the
/// machine rather than around it: a fresh pack, the built-in microcode in
/// `MCR1`, the System 100 pack's `LOD1` in `LOD1`, and then a machine booted
/// off it.  The boot PROM finds this label with its own `DECODE-LABEL` and
/// `SEARCH-LABEL`, walks the sections of the microload partition it names,
/// and turns itself off --- which it cannot do unless the label, the
/// partition table and the microcode in `MCR1` are all as the hardware
/// expects them.
///
/// What it does not do is run the band to a Lisp listener; that is ten
/// million microcycles.  `examples/screen.rs` with this pack under it is that
/// check, by hand.
#[test]
fn a_pack_made_here_boots() {
    let Some(theirs) = vendor(&["run", "disk-sys-100-0.img"]) else { return };
    let dir = scratch("diskpack-boots");
    let (mut made, path) = pack_at(&dir, "made.img");
    for c in [
        Command::Initialize,
        Command::Name("MIT-LISPM-2".to_string()),
        Command::Comment("made by diskpack".to_string()),
        Command::Load { partition: "MCR1".to_string(), file: None },
        Command::LoadFrom { partition: "LOD1".to_string(), pack: theirs, from: "LOD1".to_string() },
    ] {
        made.run(c).unwrap();
    }

    let mut e = muir::micro::Micro::new(support::machine_with_pack(&path));
    e.boot();
    let mut microcycles = 0u64;
    while !e.machine().mode.prom_disable {
        e.step().expect("the PROM stopped before it could turn itself off");
        microcycles += 1;
        assert!(microcycles < 20_000_000, "the PROM never finished loading microcode");
    }
    // The same check `tests/boot.rs` makes of the vendored pack: what the
    // machine loaded off this one is the microcode file, word for word.
    let want = muir::mcr::parse(muir::mcr::UCADR_323).unwrap();
    let m = e.machine();
    let at = want.imem_start as usize;
    assert!(m.imem[at..at + want.imem.len()] == want.imem[..], "the control store is not the file");
    assert_ne!(m.fetch(0o10), m.prom[0o10], "the band's own words are being fetched");
}

// --- the tool as a program --------------------------------------------------

/// **The tool builds a pack from a script, and stops when a line of it
/// fails.**
///
/// The prompt is for a person, and a pipe is for a test or a `tools/` script;
/// both go through the same commands, and a script that fails part way must
/// not report success.
#[test]
fn a_script_builds_a_pack_and_a_bad_line_stops_it() {
    let dir = scratch("diskpack-script");
    let path = dir.join("scripted.img");

    let out = diskpack(&path, "initialize\nname MIT-LISPM-2\nquit\n");
    assert!(out.status.success(), "{}", support::text(&out));
    let label = Label::open(&path).unwrap();
    assert_eq!(label.pack_name, "MIT-LISPM-2");
    assert_eq!(label.drive, "Trident T-300");

    // A line that is no command stops the run: what ran before it stands, and
    // what comes after it never runs.
    let out = diskpack(&path, "name MIT-LISPM-3\nfrobnicate\ncomment never\n");
    assert_eq!(out.status.code(), Some(1), "{}", support::text(&out));
    assert!(support::text(&out).contains("frobnicate is no command"));
    let label = Label::open(&path).unwrap();
    assert_eq!(label.pack_name, "MIT-LISPM-3", "the line before it took effect");
    assert_eq!(label.comment, "(comment)", "the line after it never ran");
}

/// **A command on the command line does what it does at the prompt.**
///
/// The prompt has no completion of its own --- that would be a readline and
/// muir has no dependencies --- so a command that takes a path is given on
/// the command line, where the shell completes it.  A word that is no command
/// there is a usage error, like a flag muir does not have; a command that ran
/// and failed is not.
#[test]
fn a_command_on_the_command_line_runs() {
    let dir = scratch("diskpack-argv");
    let path = dir.join("argv.img");

    let out = args(&path, &["initialize"]);
    assert!(out.status.success(), "{}", support::text(&out));
    assert_eq!(Label::open(&path).unwrap().drive, "Trident T-300", "initialize wrote the label");

    let out = args(&path, &["name", "MIT-LISPM-2"]);
    assert!(out.status.success(), "{}", support::text(&out));
    assert_eq!(Label::open(&path).unwrap().pack_name, "MIT-LISPM-2", "and so did name");
    assert!(support::text(&out).is_empty(), "and said nothing: {}", support::text(&out));

    let out = args(&path, &["show"]);
    assert!(
        support::text(&out).contains("LABL version 1, 815 cylinders"),
        "{}",
        support::text(&out)
    );

    // A name with a space in it survives the shell's quoting: every command
    // that takes a path takes the whole of what is left of the line as it.
    let spaced = dir.join("a band.dump");
    let out = args(&path, &["dump", "MCR2", spaced.to_str().unwrap()]);
    assert!(out.status.success(), "{}", support::text(&out));
    assert_eq!(std::fs::metadata(&spaced).unwrap().len(), 148 * BLOCK_BYTES);

    let out = args(&path, &["frobnicate"]);
    assert_eq!(out.status.code(), Some(2), "no command is a usage error");
    assert!(support::text(&out).contains("frobnicate is no command"));

    // And one that ran and failed is not a usage error but a failure.
    let out = args(&path, &["current", "MCR9"]);
    assert_eq!(out.status.code(), Some(1), "{}", support::text(&out));
}

/// The tool with a command on its command line.
fn args(image: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_diskpack"))
        .arg(image)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("diskpack")
}

/// The tool with a script on its stdin.
fn diskpack(image: &Path, script: &str) -> std::process::Output {
    use std::io::Write;
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_diskpack"))
        .arg(image)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("diskpack");
    child.stdin.take().unwrap().write_all(script.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

// --- what must not happen ---------------------------------------------------

/// **`initialize` does not write over a file that is not a pack.**
///
/// It is the one command that makes a file, it makes a 257 MiB one, and the
/// path it is given comes from a shell where a typo is a different file that
/// exists.  Reported by a review of this file: a 20-byte note, `diskpack
/// notes.txt initialize`, and the note was gone.
#[test]
fn initialize_does_not_write_over_a_file() {
    let dir = scratch("diskpack-not-over-a-file");
    let path = dir.join("notes.txt");
    let note = b"a note I care about\n";
    std::fs::write(&path, note).unwrap();

    let (mut pack, said) = Pack::open(&path);
    assert!(said.contains("is not a pack"), "{said}");
    let e = pack.run(Command::Initialize).unwrap_err();
    assert!(e.contains("is not a pack"), "{e}");
    assert_eq!(std::fs::read(&path).unwrap(), note, "the file is as it was");
}

/// **`initialize` does not replace a pack that is already there.**
///
/// The blocks would survive and the only record of where the bands are would
/// not, and there is no write to withhold and no undo: every command here
/// takes effect as it runs.  So the one irreversible act is not spelled the
/// same as the harmless one.
#[test]
fn initialize_does_not_replace_a_pack() {
    let dir = scratch("diskpack-not-over-a-pack");
    let (mut pack, path) = pack_at(&dir, "pack.img");
    pack.run(Command::Initialize).unwrap();
    pack.run(Command::Name("MIT-LISPM-2".to_string())).unwrap();

    let e = pack.run(Command::Initialize).unwrap_err();
    assert!(e.contains("is a pack already"), "{e}");
    assert!(e.contains("MIT-LISPM-2"), "it says what it found: {e}");
    assert_eq!(Label::open(&path).unwrap().pack_name, "MIT-LISPM-2", "the label is as it was");
}

/// **A size no pack could hold is refused, and does not overflow.**
///
/// `20000000c` is 6,460,000,000 blocks, which is not a `u32`; `13300000c`
/// wraps to 932,704, which is a size a pack could hold and is not the size
/// that was asked for.  Both are arithmetic on numbers that came off a
/// command line, so both are checked rather than trusted.
#[test]
fn a_size_no_pack_could_hold_is_refused() {
    let dir = scratch("diskpack-huge");
    let (mut pack, path) = pack_at(&dir, "pack.img");
    pack.run(Command::Initialize).unwrap();
    let before = std::fs::read(&path).unwrap()[..1024].to_vec();

    for size in [Size::Cylinders(20_000_000), Size::Cylinders(13_300_000), Size::Blocks(u32::MAX)] {
        let e = pack
            .run(Command::Partition { name: "NEW".to_string(), size, comment: None })
            .unwrap_err();
        assert!(e.contains("263245"), "{size:?}: says what the pack holds: {e}");
        assert_eq!(std::fs::read(&path).unwrap()[..1024], before, "{size:?}: the label stands");
    }
}

/// **A label with nonsense in it is read and refused, not run into.**
///
/// A label is data off a pack and the numbers in it are whatever is there.
/// `show` must print what it found rather than overflowing on it, and a
/// partition that runs off the end of the pack is not somewhere blocks can be
/// put --- an allocation sized by it would be sized by the file's own claim.
#[test]
fn a_label_with_nonsense_in_it_is_refused() {
    let dir = scratch("diskpack-nonsense");
    let path = dir.join("pack.img");
    let mut label = Label::initialize(&path, &T300);
    label.write().unwrap();
    // Written under the tool's back, as a pack from somewhere else would be.
    label.partitions[9].start = u32::MAX - 100;
    label.partitions[9].blocks = u32::MAX - 100;
    label.write().unwrap();

    let (mut pack, _) = Pack::open(&path);
    let shown = pack.run(Command::Show).expect("a label is printed, whatever is in it");
    assert!(shown.contains("LOD1"), "{shown}");

    let file = dir.join("band.dump");
    std::fs::write(&file, [0u8; 1024]).unwrap();
    let e =
        pack.run(Command::Load { partition: "LOD1".to_string(), file: Some(file) }).unwrap_err();
    assert!(e.contains("past the end of the pack"), "{e}");
    let e = pack
        .run(Command::Dump { partition: "LOD1".to_string(), file: dir.join("out.dump") })
        .unwrap_err();
    assert!(e.contains("past the end of the pack"), "{e}");
}

/// **A source too short for what its label promises is refused before
/// anything is written.**
///
/// `load-from` reads another pack's partition, and that pack is a file like
/// any other: it can be truncated.  Copying until the read fails would leave
/// ours half-filled with no way to tell.
#[test]
fn load_from_refuses_a_source_that_is_too_short() {
    let dir = scratch("diskpack-short-source");
    let (mut ours, our_path) = pack_at(&dir, "ours.img");
    ours.run(Command::Initialize).unwrap();

    // A pack whose label is a T-300's and whose file stops after the label.
    let theirs = dir.join("theirs.img");
    Label::initialize(&theirs, &T300).write().unwrap();
    std::fs::OpenOptions::new().write(true).open(&theirs).unwrap().set_len(1024).unwrap();

    let before = std::fs::read(&our_path).unwrap();
    let e = ours
        .run(Command::LoadFrom {
            partition: "LOD1".to_string(),
            pack: theirs,
            from: "LOD1".to_string(),
        })
        .unwrap_err();
    assert!(e.contains("is not a whole pack") || e.contains("bytes"), "{e}");
    assert!(std::fs::read(&our_path).unwrap() == before, "ours was not written to");
}

/// **The free space is all of it, including the blocks before the first
/// partition.**
///
/// The gap lines between partitions are MIT's own display; the total under
/// them is this tool's, and a total that leaves out the reserved track and
/// whatever else is under the first partition is not the free space.
#[test]
fn the_free_space_is_all_of_it() {
    let dir = scratch("diskpack-free");
    let path = dir.join("pack.img");
    let mut label = Label::initialize(&path, &T300);
    label.partitions.retain(|p| p.name == "FILE");
    label.write().unwrap();

    let (mut pack, _) = Pack::open(&path);
    let shown = pack.run(Command::Show).unwrap();
    let file = Label::open(&path).unwrap().partition("FILE").unwrap().clone();
    // Everything but the reserved track and FILE itself: the track is not
    // free, it is reserved.
    let free = label.blocks() - file.blocks - 17;
    assert!(shown.contains(&format!("{free} blocks free")), "all of it: {shown}");
    assert!(shown.contains(&format!("{} blocks free at 17", file.start - 17)), "the head: {shown}");
}

/// **What was loaded is recorded in the partition's comment.**
///
/// The file is not on the pack --- only what came out of it is --- and the
/// comment is the only place a pack says what a partition holds.  MIT's own
/// software wrote that field the same way: `MCR1` on the System 100 pack says
/// "UCADR 323" and `LOD1` says "cold 3/23/23", which is how anyone reading
/// the label knows which band is which.
#[test]
fn what_was_loaded_is_in_the_comment() {
    let dir = scratch("diskpack-comments");
    let (mut pack, path) = pack_at(&dir, "pack.img");
    pack.run(Command::Initialize).unwrap();

    // The built-in microcode writes MIT's own words for it, in the field MIT
    // put them in.
    let said = pack.run(Command::Load { partition: "MCR1".to_string(), file: None }).unwrap();
    assert!(said.contains("MCR1 is now \"UCADR 323\""), "{said}");
    assert_eq!(Label::open(&path).unwrap().partition("MCR1").unwrap().comment, "UCADR 323");

    // A file writes its own name, and its name only: the directory it was in
    // is not something the pack can hold or a later reader can use.
    let band = dir.join("a-band-with-a-very-long-name.dump");
    std::fs::write(&band, [0u8; 1024]).unwrap();
    pack.run(Command::Load { partition: "LOD1".to_string(), file: Some(band) }).unwrap();
    assert_eq!(
        Label::open(&path).unwrap().partition("LOD1").unwrap().comment,
        "a-band-with-a-ve",
        "cut to the sixteen characters a seven-word descriptor holds"
    );

    // And it is the comment as it stands afterwards, not an addition to what
    // was there: a partition says what is in it now.
    let short = dir.join("lod1.dump");
    std::fs::write(&short, [0u8; 1024]).unwrap();
    pack.run(Command::Load { partition: "LOD1".to_string(), file: Some(short) }).unwrap();
    assert_eq!(Label::open(&path).unwrap().partition("LOD1").unwrap().comment, "lod1.dump");
}
