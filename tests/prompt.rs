// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The prompt's commands, as lines.  What `muir` does with them is
//! `tests/muir_prompt.rs`.

use std::path::PathBuf;

use muir::prompt::{Command, Memory, NetName, parse, parse_net_names};

mod support;

/// **Each command parses, with its argument or without, whatever the
/// spacing.**
#[test]
fn each_command_parses() {
    assert_eq!(parse("boot"), Ok(Some(Command::Boot)));
    assert_eq!(parse("hold"), Ok(Some(Command::Hold)));
    assert_eq!(parse("  continue\n"), Ok(Some(Command::Continue)));
    assert_eq!(parse("pc"), Ok(Some(Command::Pc)));
    assert_eq!(parse("reg"), Ok(Some(Command::Registers)));
    assert_eq!(parse("quit"), Ok(Some(Command::Quit)));
    assert_eq!(parse("info"), Ok(Some(Command::Info)));
    assert_eq!(parse("help"), Ok(Some(Command::Help)));
    assert_eq!(parse("step"), Ok(Some(Command::Step(1))));
    assert_eq!(parse("step   12 "), Ok(Some(Command::Step(12))));
    assert_eq!(parse("checkpoint"), Ok(Some(Command::Checkpoint(None))));
    assert_eq!(parse("screenshot"), Ok(Some(Command::Screenshot(None))));
    assert_eq!(parse("ss  a shot.png"), Ok(Some(Command::Screenshot(Some("a shot.png".into())))));
    assert_eq!(parse("startcapture"), Ok(Some(Command::StartCapture(None))));
    assert_eq!(parse("sc x.gif"), Ok(Some(Command::StartCapture(Some("x.gif".into())))));
    assert_eq!(parse("endcapture"), Ok(Some(Command::EndCapture)));
    assert_eq!(parse("ec"), Ok(Some(Command::EndCapture)));
    assert_eq!(
        parse("checkpoint  a place/with space.chk "),
        Ok(Some(Command::Checkpoint(Some(PathBuf::from("a place/with space.chk")))))
    );
    assert_eq!(parse(""), Ok(None));
    assert_eq!(parse("   \n"), Ok(None));
}

/// **`c`, `i`, `q`, `h` and `?` are `continue`, `info`, `quit` and
/// `help`.**
#[test]
fn the_short_ways_say_the_same_thing() {
    assert_eq!(parse("c"), Ok(Some(Command::Continue)));
    assert_eq!(parse("i"), Ok(Some(Command::Info)));
    assert_eq!(parse("q"), Ok(Some(Command::Quit)));
    assert_eq!(parse("h"), Ok(Some(Command::Help)));
    assert_eq!(parse("?"), Ok(Some(Command::Help)));
}

/// **A memory is dumped whole, from an address, or so many words from
/// one**, and the address and the count are octal.
#[test]
fn a_dump_takes_an_octal_address_and_count() {
    let dump = |memory, from, words| Ok(Some(Command::Dump { memory, from, words }));
    assert_eq!(parse("amem"), dump(Memory::Amem, 0, None));
    assert_eq!(parse("mmem 10"), dump(Memory::Mmem, 8, None));
    assert_eq!(parse("dmem 10 4"), dump(Memory::Dmem, 8, Some(4)));
    assert_eq!(parse("pdl  1000  20 "), dump(Memory::Pdl, 512, Some(16)));
    assert_eq!(parse("spc 0 40"), dump(Memory::Spc, 0, Some(32)));
    assert!(parse("amem 8").unwrap_err().contains("octal address"));
    assert!(parse("amem 0 9").unwrap_err().contains("octal count"));
    assert!(parse("amem 0 4 0").unwrap_err().contains("no more"));
}

/// **A dump is four words to a line**: the octal address, the words in
/// hex, and the four characters each holds, the first in the low byte.
/// Lines the same as the one above them are a `*`, and the last line is
/// always written whole, so the dump ends at the address it reached.
#[test]
fn a_dump_writes_four_words_to_a_line() {
    // `LABL` low byte first, which is what block 0 of a pack holds.
    let mut words = [0u32; 20];
    words[0] = 0x4c42414c;
    words[16] = 0x0000_2020;
    let d = muir::prompt::dump(&words, 0o20);
    let lines: Vec<&str> = d.lines().collect();
    assert_eq!(lines.len(), 4, "the first, the second, a star for the third, the last:\n{d}");
    assert_eq!(lines[0], "000020  4c42414c 00000000 00000000 00000000  LABL .... .... ....");
    assert_eq!(lines[1], "000024  00000000 00000000 00000000 00000000  .... .... .... ....");
    assert_eq!(lines[2], "*");
    assert_eq!(
        lines[3],
        "000040  00002020 00000000 00000000 00000000  \u{20}\u{20}.. .... .... ...."
    );
}

/// **A short last line is padded**, so the characters stay in their
/// column.
#[test]
fn a_short_line_keeps_its_columns() {
    let full = muir::prompt::dump(&[0x4141_4141; 4], 0);
    let short = muir::prompt::dump(&[0x4141_4141; 2], 0);
    let column = |l: &str| l.find("  AAAA").unwrap();
    assert_eq!(
        column(full.lines().next().unwrap()),
        column(short.lines().next().unwrap()),
        "full:\n{full}short:\n{short}"
    );
}

/// **Registers are written in hex and as characters.**
#[test]
fn registers_are_written_in_hex_and_as_characters() {
    // 010 in the PC's low byte is lambda, which is where the Lisp
    // Machine's character set parts company with ASCII.
    let s = muir::prompt::registers(&[("PC", 0o4321), ("MD", 0x4c42414c)]);
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(
        lines,
        ["PC                 000008d1  .\u{3bb}..", "MD                 4c42414c  LABL"]
    );
}

/// **The Lisp Machine's character set is the one MIT's own character
/// table gives**: 001 to 037 the graphics, by the table's names for them,
/// 040 to 176 ASCII, and the keys and font switches from 200 up printing
/// nothing.  Needs the System 100 release, and says so when it is not
/// there.
#[test]
fn the_character_set_is_mits_own() {
    for c in 0o40..=0o176u8 {
        assert_eq!(muir::prompt::character(c), c as char, "{c:o} is ASCII");
    }
    for c in [0u8, 0o177, 0o200, 0o207, 0o215, 0o240, 0o377] {
        assert_eq!(muir::prompt::character(c), '.', "{c:o} prints nothing");
    }
    let Some(table) = support::vendor(&["system-100-0", "sys", "doc", "char.text"]) else {
        return;
    };
    let table = String::from_utf8_lossy(&std::fs::read(table).unwrap()).into_owned();
    for code in 1..=0o37u8 {
        let name = muir::prompt::graphic_name(code).unwrap();
        assert!(
            table.lines().any(|l| l.trim_start().starts_with(&format!("{code:03o} {name}"))),
            "{code:03o} {name} is what the table gives that code"
        );
    }
}

/// **A line that is no command says what was wrong with it**, and points
/// at `help`.
#[test]
fn what_is_no_command_says_so() {
    let err = parse("run").unwrap_err();
    assert!(err.contains("run") && err.contains("help"), "{err}");
    assert!(parse("step x").unwrap_err().contains("count"));
    assert!(parse("step 0").unwrap_err().contains("count"));
    assert!(parse("continue now").unwrap_err().contains("takes nothing"));
    assert!(parse("pc 3").unwrap_err().contains("takes nothing"));
}

/// **A net is named the way the drawings name it, punctuation and all.**
///
/// MIT's net names carry spaces (`SYNC PROM ENB`), leading minus signs
/// (`-XBUS RQ`), dots (`TRIDENT.0.SELECT/`) and trailing slashes, so the
/// whole of the line after the word is the name and only the two pieces of
/// punctuation that cannot appear in one are read off it: a colon after a
/// single word is the board, and a slash with digits after it is a bus's
/// width. A name that *ends* in a slash keeps it.
#[test]
fn a_net_is_named_as_the_drawings_name_it() {
    let net = |board: Option<&str>, name: &str, width: Option<u32>| {
        Ok(Some(Command::Net(NetName {
            board: board.map(str::to_string),
            name: name.to_string(),
            width,
        })))
    };
    assert_eq!(parse("net MEMRQ"), net(None, "MEMRQ", None));
    // A trailing slash is the name's: `TRIDENT.READY/` is a net, not a bus
    // of no bits.
    assert_eq!(parse("net TRIDENT.READY/"), net(None, "TRIDENT.READY/", None));
    assert_eq!(parse("net TRIDENT.0.SELECT/"), net(None, "TRIDENT.0.SELECT/", None));
    // Spaces and a leading minus are MIT's own.
    assert_eq!(parse("net -XBUS RQ"), net(None, "-XBUS RQ", None));
    assert_eq!(parse("net  SYNC PROM ENB "), net(None, "SYNC PROM ENB", None));
    // A board, where more than one carries the name.
    assert_eq!(parse("net disk:-XBUS.RQ"), net(Some("disk"), "-XBUS.RQ", None));
    assert_eq!(parse("net cpu: MEMRQ"), net(Some("cpu"), "MEMRQ", None));
    // A bus, as MUIR_WATCH writes one.
    assert_eq!(parse("net PC/14"), net(None, "PC", Some(14)));
    assert_eq!(parse("net busint:XBI/32"), net(Some("busint"), "XBI", Some(32)));
    // A colon inside a name is not a board: a board is one word.
    assert_eq!(parse("net LM UB: GRANTED"), net(None, "LM UB: GRANTED", None));
    // What is refused, including a board with no name after it.
    assert!(parse("net").is_err());
    assert!(parse("net ").is_err());
    assert!(parse("net :").is_err());
    assert!(parse("net disk:").is_err());
    assert!(parse("net disk:  ").is_err());
    assert!(parse("net PC/0").is_err());
    assert!(parse("net PC/65").is_err());
}

/// **`watch` takes a count of microcycles and then nets, comma
/// separated, each named as `net` names one** --- a board before a
/// colon, a bus's width after a slash, and the name whole between, spaces
/// and all.  The list is `--watch`'s too, and a name with a comma in it
/// --- the memory board's `-CAS 0,1 LH` --- is the one kind of name the
/// list cannot carry.
#[test]
fn watch_takes_a_count_and_nets_as_net_names_them() {
    let name = |board: Option<&str>, name: &str, width: Option<u32>| NetName {
        board: board.map(str::to_string),
        name: name.to_string(),
        width,
    };
    assert_eq!(
        parse("watch 5 PC/14"),
        Ok(Some(Command::Watch { cycles: 5, nets: vec![name(None, "PC", Some(14))] }))
    );
    assert_eq!(
        parse("watch 1 disk:CCW CLK, disk:NEW CCW ,cpu:PC/14,-XBUS RQ"),
        Ok(Some(Command::Watch {
            cycles: 1,
            nets: vec![
                name(Some("disk"), "CCW CLK", None),
                name(Some("disk"), "NEW CCW", None),
                name(Some("cpu"), "PC", Some(14)),
                name(None, "-XBUS RQ", None),
            ],
        }))
    );
    // The list, on its own, as `--watch` takes it after the range.
    assert_eq!(
        parse_net_names("disk:XBAO/22,TRIDENT.READY/", "--watch"),
        Ok(vec![name(Some("disk"), "XBAO", Some(22)), name(None, "TRIDENT.READY/", None)])
    );
    // What is refused says what was wanted: a count, a net, a bus's width.
    for line in ["watch", "watch 5", "watch x PC/14", "watch 0 PC/14", "watch 5 ,", "watch 5 PC/0"]
    {
        assert!(parse(line).is_err(), "{line:?} parsed");
    }
    assert!(parse("watch 5").unwrap_err().contains("net"));
    assert!(parse("watch x PC/14").unwrap_err().contains("count"));
    assert!(parse_net_names("PC/14,", "--watch").unwrap_err().contains("--watch"));
    assert!(parse_net_names("", "watch").unwrap_err().contains("watch"));
}

/// **`mem` takes a physical address, which it will not do without, and a
/// count of words, which is one.**
///
/// The other dumps default their address to 0 and their count to the whole
/// memory, because the largest of those memories is 2048 words.  Main
/// memory is two million, so `mem` on its own is refused rather than
/// answered with half a million lines, and a count left out is one word ---
/// which is the question that wanted the command: what is in the 512th
/// CCW.  Both numbers are octal, as MIT writes them.
#[test]
fn mem_wants_a_physical_address_and_gives_one_word() {
    assert_eq!(parse("mem 40777"), Ok(Some(Command::Mem { from: 0o40777, words: 1 })));
    assert_eq!(parse("mem  40000 1000 "), Ok(Some(Command::Mem { from: 0o40000, words: 0o1000 })));
    assert_eq!(parse("mem 0"), Ok(Some(Command::Mem { from: 0, words: 1 })));
    // Octal, so 10 is eight and 19 is not a number at all.
    assert_eq!(parse("mem 10"), Ok(Some(Command::Mem { from: 8, words: 1 })));
    assert!(parse("mem 19").unwrap_err().contains("octal"));
    assert!(parse("mem").unwrap_err().contains("physical address"));
    assert!(parse("mem x").unwrap_err().contains("octal address"));
    assert!(parse("mem 0 x").unwrap_err().contains("octal count"));
    assert!(parse("mem 0 1 2").unwrap_err().contains("no more"));
}

/// **What `mem` refuses, and why: the address is physical and nothing
/// translates it.**
///
/// A virtual address is refused where it can be told apart from a physical
/// one --- above the 22 bits `-XADDR0` to `-XADDR21` carry, which is the
/// whole of the address space the Xbus has --- and an address inside that
/// space with no board behind it is told how much memory the machine has.
/// Below 22 bits a virtual address and a physical one are the same numbers
/// and nothing can tell them apart, which is why the help says which of the
/// two this takes.
///
/// A count past the end is the rest of the memory, as it is for the
/// processor's own memories, so that the largest count the prompt can read
/// does not overflow the address it is added to.
#[test]
fn mem_refuses_what_is_not_a_word_of_this_machines_main_memory() {
    // One board of 64K words, every word holding its own address.
    let fitted = 0o200000;
    let read = |a: usize| (a < fitted).then_some(a as u32);
    let dump = |from, words| muir::prompt::main_dump(from, words, fitted, read);

    let one = dump(0o40777, 1).unwrap();
    assert_eq!(one.lines().count(), 1, "one word is one line: {one}");
    assert!(one.starts_with("040777  000041ff"), "the word at 40777: {one}");
    // Four words to a line, from the address asked for.
    assert_eq!(dump(0o40777, 4).unwrap().lines().count(), 1);
    assert_eq!(dump(0o40777, 5).unwrap().lines().count(), 2);

    // Past 22 bits is not a physical address at all.
    let err = dump(muir::prompt::PHYSICAL_WORDS, 1).unwrap_err();
    assert!(err.contains("22 bits") && err.contains("does not go through the map"), "{err}");
    // Inside the Xbus's space, past this machine's boards.
    let err = dump(fitted, 1).unwrap_err();
    assert!(err.contains("200000 words, 0 to 177777"), "{err}");
    assert!(dump(fitted - 1, 1).is_ok(), "the last word is there");

    // A count past the end is the rest of the memory, however far past.
    let rest = dump(fitted - 6, usize::MAX).unwrap();
    assert_eq!(rest.lines().count(), 2, "six words, two lines: {rest}");
    assert!(rest.contains("\n177776  0000fffe 0000ffff "), "and ends at the last word: {rest}");
}
