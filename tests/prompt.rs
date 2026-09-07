// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The prompt's commands, as lines.  What `muir` does with them is
//! `tests/muir_prompt.rs`.

use std::path::PathBuf;

use muir::prompt::{Command, Memory, parse};

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
