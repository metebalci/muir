// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The keyboard mapping: what a viewer's keysym means on the Lisp Machine
//! keyboard, and where a user says so.
//!
//! The mapping is the one piece of muir's keyboard that is not MIT's ---
//! the key positions and the cable word are theirs, `tests/keyboard.rs`
//! holds those --- so it has no citation to be checked against and is
//! held here to what it promises instead: that the built-in default is
//! the mapping muir has always had, that the format can name every key
//! the keyboard has, that no key of the keyboard is out of reach, and
//! that a file changes it.

use std::io::Write;
use std::process::Stdio;

use muir::terminal::keyboard::{
    self, Key, Keyboard, Mapping, Shift, keysym, named, shifting, up_down,
};

mod support;
use support::{Run, muir, scratch, text};

/// The words a fresh keyboard makes of a run of keysyms, each pressed and
/// released in turn.
fn typed(map: Mapping, keys: &[(u32, bool)]) -> Vec<u32> {
    let mut k = Keyboard::with_mapping(map);
    for &(sym, down) in keys {
        k.key(sym, down);
    }
    let mut out = Vec::new();
    while let Some(w) = k.take() {
        out.push(w);
    }
    out
}

/// A press and release of one keysym.
fn tap(sym: u32) -> [(u32, bool); 2] {
    [(sym, true), (sym, false)]
}

/// **The built-in default is the mapping muir already had.** Every keysym
/// the hard-coded `positions` and `modifier` answered answers the same
/// way, so turning the mapping into data changed no behaviour.
#[test]
fn the_default_is_the_mapping_muir_already_had() {
    let m = Mapping::default();
    // The characters, which are MIT's table and not the mapping's: a
    // printable keysym is looked for on the table's two planes.
    assert_eq!(m.positions('a' as u32), [(0o123, false)]);
    assert_eq!(m.positions('A' as u32), [(0o123, true)]);
    assert_eq!(m.positions('!' as u32), [(0o121, true)], "shift 1");
    assert_eq!(m.positions('(' as u32), [(0o71, true), (0o132, false)], "9 shifted, or its key");
    assert_eq!(m.positions(' ' as u32), [(0o134, false)]);
    // The named keys the mapping does name.
    for (sym, name) in [
        (keysym::RETURN, "Return"),
        (keysym::KP_ENTER, "Return"),
        (keysym::TAB, "Tab"),
        (keysym::BACKSPACE, "Rubout"),
        (keysym::DELETE, "Rubout"),
        (keysym::LINEFEED, "Line"),
        (keysym::ESCAPE, "Alt Mode"),
        (keysym::HELP, "Help"),
        (keysym::BREAK, "Break"),
        (keysym::CANCEL, "Abort"),
        (keysym::END, "End"),
        (keysym::PAUSE, "Hold Output"),
    ] {
        assert_eq!(m.positions(sym), [(named(name).unwrap(), false)], "{name}");
    }
    // The twelve function keys, in the order muir chose.
    for (i, name) in keyboard::FUNCTION_KEYS.iter().enumerate() {
        let sym = keysym::F1 + i as u32;
        assert_eq!(m.positions(sym), [(named(name).unwrap(), false)], "F{}", i + 1);
    }
    // The modifiers, left and right.
    for (sym, shift, side) in [
        (keysym::SHIFT_L, Shift::Shift, 0),
        (keysym::SHIFT_R, Shift::Shift, 1),
        (keysym::CONTROL_L, Shift::Control, 0),
        (keysym::CONTROL_R, Shift::Control, 1),
        (keysym::META_L, Shift::Meta, 0),
        (keysym::ALT_L, Shift::Meta, 0),
        (keysym::META_R, Shift::Meta, 1),
        (keysym::ALT_R, Shift::Meta, 1),
        (keysym::SUPER_L, Shift::Super, 0),
        (keysym::SUPER_R, Shift::Super, 1),
        (keysym::HYPER_L, Shift::Hyper, 0),
        (keysym::HYPER_R, Shift::Hyper, 1),
        (keysym::CAPS_LOCK, Shift::CapsLock, 0),
    ] {
        assert_eq!(m.modifier(sym), Some((shift, side)), "{shift:?} {side}");
    }
    assert!(m.positions(0xff50).is_empty(), "the CADR has no Home key");
    assert_eq!(m.modifier(0xff50), None);
}

/// **The default is written in the format the user writes.** It is not a
/// table in the code that a file merely adds to: the same parser reads
/// both, so anything the default says a file can say.
#[test]
fn the_default_is_written_in_the_format_a_user_writes() {
    let parsed = Mapping::parse(keyboard::DEFAULT_MAPPING).expect("the built-in default parses");
    assert_eq!(parsed, Mapping::default(), "the default is its own text, read back");
    assert!(
        keyboard::DEFAULT_MAPPING.lines().any(|l| l.trim_start().starts_with('#')),
        "and it says what it is"
    );
}

/// **The format can name every key the keyboard has.** All 31 named keys
/// and all 11 shifting keys, by MIT's own name for them, and a character
/// key by its character.
#[test]
fn the_format_can_name_every_key() {
    let mut named_keys = 0;
    let mut shifting_keys = 0;
    for (position, key) in keyboard::TABLE.iter().enumerate() {
        let (word, kind) = match key {
            Key::Named(n) => {
                named_keys += 1;
                (n.to_string(), "named")
            }
            Key::Shift(s) => {
                shifting_keys += 1;
                (keyboard::shift_name(*s).to_string(), "shifting")
            }
            _ => continue,
        };
        let m = Mapping::parse(&format!("key F13 {word}"))
            .unwrap_or_else(|e| panic!("{kind} key {word:?}: {e}"));
        let got = m.positions(0xffca);
        assert_eq!(got.len(), 1, "{kind} key {word:?} binds one position: {got:?}");
        // A shifting key has two positions and the name reaches the left
        // one; a named key has the one.
        let want = match key {
            Key::Shift(s) => shifting(*s)[0],
            _ => position as u8,
        };
        assert_eq!(got[0].0, want, "{kind} key {word:?}");
    }
    assert_eq!(named_keys, 31, "MIT's table has 31 named keys");
    assert_eq!(shifting_keys, 18, "and 18 shifting positions, 11 keys");

    // A character key by its character, on either plane.
    let m = Mapping::parse("key F13 z").unwrap();
    assert_eq!(m.positions(0xffca), [(0o124, false)]);
    let m = Mapping::parse("key F13 Z").unwrap();
    assert_eq!(m.positions(0xffca), [(0o124, true)]);
}

/// **No key of the keyboard is out of reach with the default alone.**
/// This is the issue: Greek, Top, Alt Lock, Mode Lock and Repeat had no
/// route at all, and twelve named keys with them. Every position MIT's
/// table gives a key must now be reachable from some keysym, or some
/// prefix and keysym, without the user writing a file.
#[test]
fn every_key_can_be_typed_with_the_default() {
    let m = Mapping::default();
    let reachable = m.reachable();
    let mut missing = Vec::new();
    for (position, key) in keyboard::TABLE.iter().enumerate() {
        let name = match key {
            Key::Named(n) => n.to_string(),
            Key::Shift(s) => keyboard::shift_name(*s).to_string(),
            Key::Char(..) => continue, // MIT's table finds these itself
            Key::None => continue,
        };
        // A shifting key with two positions is reachable if either is.
        let ok = match key {
            Key::Shift(s) => shifting(*s).iter().any(|p| reachable.contains(p)),
            _ => reachable.contains(&(position as u8)),
        };
        if !ok {
            missing.push(name);
        }
    }
    assert!(missing.is_empty(), "no route to {missing:?}");
    // And the five the issue names, each by name, so a regression says
    // which one went.
    for s in [Shift::Greek, Shift::Top, Shift::AltLock, Shift::ModeLock, Shift::Repeat] {
        assert!(
            shifting(s).iter().any(|p| reachable.contains(p)),
            "{s:?} is how the Lisp Machine character set is entered"
        );
    }
}

/// **A prefix reaches a key that no host key is spare for.** The prefix
/// itself sends nothing; the key after it is what the mapping says.
///
/// A named key is tapped --- pressed and released --- because the
/// sequence is over by then and nothing is being held. A shifting key is
/// held instead, for the one key that follows it and no longer, which is
/// what holding Greek and typing a letter does on the real keyboard.
#[test]
fn a_prefix_reaches_a_key_no_host_key_is_spare_for() {
    let m = Mapping::parse(
        "prefix Scroll_Lock d Delete\n\
         prefix Scroll_Lock g Greek\n",
    )
    .unwrap();
    let prefix = 0xff14;
    let delete = named("Delete").unwrap();

    // The prefix alone sends nothing.
    assert!(typed(m.clone(), &tap(prefix)).is_empty(), "the prefix is not a key");

    // Prefix then `d`: Delete tapped, and `d`'s own release swallowed.
    let words: Vec<u32> =
        typed(m.clone(), &[tap(prefix).as_slice(), tap('d' as u32).as_slice()].concat());
    assert_eq!(words, [up_down(delete, false), up_down(delete, true)], "Delete tapped");

    // Prefix then `g`: Greek held, and the next key sent inside it.
    let greek = shifting(Shift::Greek)[0];
    let a = 0o123;
    let words: Vec<u32> = typed(
        m.clone(),
        &[tap(prefix).as_slice(), tap('g' as u32).as_slice(), tap('a' as u32).as_slice()].concat(),
    );
    assert_eq!(
        words,
        [up_down(greek, false), up_down(a, false), up_down(a, true), up_down(greek, true)],
        "Greek down, the key inside it, Greek up"
    );

    // A sequence the mapping does not have does nothing, and does not
    // leave the prefix hanging.
    let words = typed(m.clone(), &[tap(prefix).as_slice(), tap('q' as u32).as_slice()].concat());
    assert!(words.is_empty(), "an unbound sequence sends nothing: {words:?}");
    let words = typed(
        m,
        &[tap(prefix).as_slice(), tap('q' as u32).as_slice(), tap('a' as u32).as_slice()].concat(),
    );
    assert_eq!(words, [up_down(a, false), up_down(a, true)], "and the key after it is itself");
}

/// **A file changes the mapping, and what it does not say the default
/// still says.** A user with a keyboard whose Super the desktop takes
/// binds another key to it and keeps everything else.
#[test]
fn a_file_changes_the_mapping_and_keeps_the_rest() {
    let m = Mapping::read("# mine\nkey F13 Super\nkey F14 Greek\n").expect("a good file");
    assert_eq!(m.modifier(0xffca), Some((Shift::Super, 0)), "F13 is Super now");
    assert_eq!(m.modifier(0xffcb), Some((Shift::Greek, 0)), "F14 is Greek");
    // Untouched: the default's own bindings are still there.
    assert_eq!(m.positions(keysym::ESCAPE), [(named("Alt Mode").unwrap(), false)]);
    assert_eq!(m.modifier(keysym::CONTROL_L), Some((Shift::Control, 0)));
    // And a binding replaces rather than adds to the default's.
    let m = Mapping::read("key Escape Break\n").unwrap();
    assert_eq!(m.positions(keysym::ESCAPE), [(named("Break").unwrap(), false)], "Escape is Break");
}

/// **A line that says nothing muir understands is refused, with the line
/// it was on.** A mapping half read is worse than none: a user who
/// mistypes a key name would otherwise find one key silently missing.
#[test]
fn a_bad_line_is_refused_and_says_where() {
    for (text, want) in [
        ("key\n", "line 1"),
        ("key F13\n", "line 1"),
        ("key Nonesuch Return\n", "Nonesuch"),
        ("key F13 No Such Key\n", "No Such Key"),
        ("prefix F13 F14\n", "line 1"),
        ("\n\nwibble F13 Return\n", "line 3"),
        // A keysym cannot be both a key of its own and a prefix: the
        // first press would have to be two things at once.
        ("key F13 Return\nprefix F13 g Greek\n", "F13"),
    ] {
        let e = Mapping::parse(text).expect_err(&format!("{text:?} is refused"));
        assert!(e.contains(want), "{text:?}: {e:?} says {want:?}");
    }
}

/// **muir reads the file and can say what is in force.** `--keyboard-mapping`
/// names one, the start line says which file a run read, and the
/// prompt's `keys` prints the mapping so that a user who cannot type a
/// key can find out what would.
#[test]
fn muir_reads_a_mapping_and_prints_it() {
    let dir = scratch("keyboard-mapping");
    let file = dir.join("keys");
    std::fs::write(&file, "key F13 Greek\n").unwrap();

    let mut child = muir()
        .args([
            "--micro",
            "--keyboard-mapping",
            file.to_str().unwrap(),
            "--stop-after",
            "1000000000",
        ])
        .stdin(Stdio::piped())
        .start();
    let mut stdin = child.stdin();
    write!(stdin, "keys\nq\n").unwrap();
    drop(stdin);
    let out = child.wait();
    let t = text(&out);
    assert!(out.status.success(), "muir failed:\n{t}");
    assert!(t.contains("keyboard: "), "the start says which mapping this run reads:\n{t}");
    assert!(t.contains(&file.display().to_string()), "and names the file:\n{t}");
    // The mapping printed: the file's own line, and the default's still
    // in force under it.
    assert!(t.contains("F13"), "keys prints what the file said:\n{t}");
    assert!(t.contains("Alt Mode"), "and what the default still says:\n{t}");

    // With no file, the run says so and still works.
    let out = muir().args(["--micro", "--stop-after", "10"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("keyboard: built in"), "the default needs no file:\n{t}");
}

/// **A mapping written out and read back is the same mapping**, which is
/// what makes `--keyboard-mapping-dump` a starting point to edit rather
/// than a report about the bindings.
///
/// Held for the built-in mapping, and for the two bindings a name alone
/// cannot carry back. A character names the **first** position of MIT's
/// table that gives it: `(` is the shifted plane of the key at 71 and the
/// unshifted plane of the key at 132, and a bare `(` reads back as 71, so
/// a binding to the one at 132 cannot be written as a character. And a
/// position the table leaves unnamed --- 0 is the first --- has no name
/// at all. Both are written `position <octal>`, with `shifted` after it
/// for the shifted plane.
#[test]
fn a_mapping_written_out_reads_back_the_same() {
    let round = |m: &Mapping, why: &str| {
        let text = m.dump();
        let back = Mapping::parse(&text).unwrap_or_else(|e| panic!("{why}: {e}\n{text}"));
        assert_eq!(&back, m, "{why}: the dump is not the mapping\n{text}");
    };
    round(&Mapping::default(), "the built-in mapping");

    // `(` on both planes, and a position nothing names.
    let unnamed = keyboard::TABLE
        .iter()
        .position(|k| matches!(k, Key::None))
        .expect("MIT's table leaves positions unfilled");
    for line in [
        "key F1 (",
        &format!("key F1 position {unnamed:o}"),
        "key F1 position 71 shifted",
        "key F1 position 132",
        // `)` on the shifted plane of the key at 171, which another key
        // gives first, and a named key carrying the shifted plane, which
        // its name cannot say at all.
        "key F1 position 171 shifted",
        "key F1 position 1 shifted",
    ] {
        let m = Mapping::parse(line).unwrap_or_else(|e| panic!("{line}: {e}"));
        round(&m, line);
    }

    // And the awkward one end to end. A bare `(` is the key at 71 on its
    // shifted plane, so that binding may be written as the character;
    // the `(` at 132 may not, and is written out.
    assert_eq!(
        Mapping::parse("key F1 (").expect("a bare character"),
        Mapping::parse("key F1 position 71 shifted").expect("is the first position giving it"),
    );
    let named = Mapping::parse("key F1 position 71 shifted").expect("that one");
    assert!(named.dump().contains("key F1 ("), "a name that reads back is used");
    let out = Mapping::parse("key F1 position 132").expect("the other one");
    assert!(out.dump().contains("key F1 position 132"), "and one that would lie is not");
    // The plane is part of the spelling, and a name that cannot carry it
    // is not used: `)` is on the shifted plane of the key at 171 and
    // another key gives it first, and `Roman II` at 1 has no plane in its
    // name at all.
    for (line, want) in [
        ("key F1 position 171 shifted", "key F1 position 171 shifted"),
        ("key F1 position 1 shifted", "key F1 position 1 shifted"),
    ] {
        let m = Mapping::parse(line).unwrap_or_else(|e| panic!("{line}: {e}"));
        assert_eq!(m.dump().lines().last(), Some(want), "{line} is written out in full");
    }
}

/// **`--keyboard-mapping-dump` writes a file `--keyboard-mapping` reads,
/// and nothing else.**
///
/// The point is the round trip through the program, not only through
/// [`Mapping::dump`]: dump, edit, feed it back, and the edit is the only
/// difference. So stdout has to carry the mapping alone --- no start
/// line, no machine --- which is why the flag stops before a terminal is
/// bound. muir's start banner is on stderr, so nothing else has to be
/// held back for this.
#[test]
fn the_dump_is_a_file_the_mapping_flag_reads() {
    let dump = |extra: &[&str]| {
        let mut args = vec!["--keyboard-mapping-dump"];
        args.extend_from_slice(extra);
        let out = muir().args(args).run();
        assert!(out.status.success(), "muir failed:\n{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).expect("the dump is text")
    };
    let built_in = dump(&[]);
    // Every line is the format's own: a comment, a key or a prefix.
    for line in built_in.lines() {
        let kind = line.split_whitespace().next().unwrap_or("");
        assert!(
            line.starts_with('#') || kind == "key" || kind == "prefix",
            "stdout carries the mapping alone, not {line:?}"
        );
    }
    assert!(built_in.lines().any(|l| l.starts_with("key ")), "and it has bindings in it");

    let dir = scratch("keyboard-dump");
    let file = dir.join("my.keys");
    std::fs::write(&file, &built_in).unwrap();
    let again = dump(&["--keyboard-mapping", file.to_str().unwrap()]);
    assert_eq!(again, built_in, "fed back unedited it says the same thing");

    // And an edit is the only difference the next time round.
    std::fs::write(&file, format!("{built_in}key F5 Terminal\n")).unwrap();
    let edited = dump(&["--keyboard-mapping", file.to_str().unwrap()]);
    let added: Vec<&str> = edited.lines().filter(|l| !built_in.lines().any(|b| b == *l)).collect();
    assert_eq!(added, ["key F5 Terminal"], "the edit, and nothing else:\n{edited}");
}
