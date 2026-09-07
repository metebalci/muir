// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! MIT's microcode symbol table, and the names it gives the band.
//!
//! Needs the vendored System 100 release; without it these say they were
//! skipped.

use muir::sym::{Space, Symbols, parse};

mod support;
use support::{mit_text, release};

fn symbols(file: &str) -> Option<Symbols> {
    let s = release(&format!("ubin/{file}"))?;
    Some(parse(&s).unwrap())
}

/// The whole of microcode 323's table parses, and the counts are the file's.
#[test]
fn the_band_has_names() {
    let Some(s) = symbols("ucadr.sym") else { return };
    // Distinct addresses, not symbol lines: 2,264 I-MEM symbols share 2,222
    // addresses, and 382 A-MEM symbols share 373.
    assert_eq!(s.len(Space::IMem), 2_222, "control store addresses named");
    assert_eq!(s.len(Space::AMem), 373, "A memory");
    assert_eq!(s.len(Space::MMem), 30, "M memory");
    assert_eq!(s.len(Space::DMem), 142, "dispatch memory");
    assert!(!s.is_empty(Space::IMem));
}

/// The addresses a comparison between engines reports, named.
///
/// A comparison reports an octal PC; the symbol table turns it into a place
/// in MIT's own source. The addresses here are the ones a comparison of the
/// boot reports --- the disk copy loop, the swap handler, the page-fault
/// handlers, the doors into `INTR` and the transport dispatch --- and the
/// table names each.
///
/// `0o27775` is `DISK-COPY-SECTION` in `uc-cold-disk.lisp`, whose first
/// instruction is `(POPJ-EQUAL M-J A-ZERO)  ;If done.` --- a conditional
/// return. Falling through it instead of taking it is what a stale `MD`
/// looks like from outside, on an `M-J` read one microcycle early.
#[test]
fn the_divergence_has_a_name() {
    let Some(s) = symbols("ucadr.sym") else { return };
    assert_eq!(s.at(Space::IMem, 0o27775), ["DISK-COPY-SECTION"], "the conditional return");
    assert_eq!(s.name(Space::IMem, 0o27775).unwrap(), "DISK-COPY-SECTION");
    assert_eq!(s.name(Space::IMem, 0o27776).unwrap(), "DISK-COPY-SECTION+1", "falling through");
    assert_eq!(s.name(Space::IMem, 0o27262).unwrap(), "DISK-RR-1+17", "taking the return");

    // The swap handler and the page-fault write.
    assert_eq!(s.name(Space::IMem, 0o25107).unwrap(), "DISK-SWAP-HANDLER+9");
    assert_eq!(s.at(Space::IMem, 0o23547), ["PGF-W-I"], "the page-fault write");

    // The doors into `INTR`. A disk interrupt that never clears sends the
    // machine through one of these at every interruptible check, so a
    // divergence that lands here is that and not a datapath fault.
    assert_eq!(s.at(Space::IMem, 0o25455), ["INTR"]);
    assert_eq!(s.at(Space::IMem, 0o23531), ["PGF-R-I"]);
    assert_eq!(s.at(Space::IMem, 0o170), ["QMLP-P-OR-I-OR-SB"]);
    assert_eq!(s.name(Space::IMem, 0o23507).unwrap(), "PGF-R-SB+2");
    assert_eq!(s.name(Space::IMem, 0o23734).unwrap(), "ADVANCE-SECOND-LEVEL-MAP-REUSE-POINTER");

    // The transport dispatch, where taking the map bit and ORing it in part
    // company: `XSET2`'s dispatch goes to the 0 entry or the 1 entry of
    // `D-TRANSPORT` for a `ONE-Q-FORWARD` by whether address bit 0 is the
    // map bit alone or the map bit ORed with pointer bit 24.
    assert_eq!(s.at(Space::IMem, 0o324), ["XSET2"]);
    assert_eq!(s.at(Space::IMem, 0o17557), ["TRANS-OLD0"], "the 0 entry: check for old space");
    assert_eq!(s.at(Space::IMem, 0o17706), ["TRANS-OQF"], "the 1 entry: follow the forward");
}

/// An address between labels is named by the one before it.
#[test]
fn a_pc_with_no_label_is_named_by_the_one_above() {
    let Some(s) = symbols("ucadr.sym") else { return };
    let (name, off) = s.nearest(Space::IMem, 0o27776).unwrap();
    assert_eq!(name, "DISK-COPY-SECTION");
    assert_eq!(off, 1);
    // ILLOP is at 0o23, the lowest labelled place the microcode halts at.
    assert_eq!(s.at(Space::IMem, 0o23), ["ILLOP"]);
}

/// Every `I-MEM` line of `mit/sys/ubin/promh.sym`, read by hand: `NAME
/// I-MEM ADDRESS`, the address octal, one line with a leading `-2`. It is
/// the reading `tests/prom.rs` and `tests/chip.rs` make of the same file.
fn prom_labels(text: &str) -> Vec<(&str, u32)> {
    text.lines()
        .map(|l| l.strip_prefix("-2 ").unwrap_or(l))
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let (name, space, addr) = (f.next()?, f.next()?, f.next()?);
            (space == "I-MEM").then(|| (name, u32::from_str_radix(addr, 8).unwrap()))
        })
        .collect()
}

/// The boot PROM's table is the same format. `mit/sys/ubin/promh.sym` is
/// committed, so this never skips: every label line of the file is in the
/// parsed table at its address, and the table has nothing else.
#[test]
fn the_boot_proms_table_reads_too() {
    let text = mit_text(&["sys", "ubin", "promh.sym"]);
    let s = parse(&text).unwrap();
    let labels = prom_labels(&text);
    assert!(labels.len() > 50, "the PROM has labels: {}", labels.len());
    for &(name, addr) in &labels {
        assert!(s.at(Space::IMem, addr).iter().any(|n| n == name), "{name} at {addr:o}");
    }
    let addresses: std::collections::BTreeSet<u32> = labels.iter().map(|&(_, a)| a).collect();
    assert_eq!(s.len(Space::IMem), addresses.len(), "an entry an address, and no more");
    // `GO` is the reset vector's target; `tests/prom.rs` pins the vector.
    assert_eq!(s.at(Space::IMem, 0o45), ["GO"]);
}

/// The release carries the PROM's table too, and it names the same
/// addresses as the committed copy: two routes to one table.
#[test]
fn the_releases_prom_table_is_the_committed_one() {
    let Some(r) = symbols("promh.sym") else { return };
    let text = mit_text(&["sys", "ubin", "promh.sym"]);
    let s = parse(&text).unwrap();
    for &(name, addr) in &prom_labels(&text) {
        assert_eq!(r.at(Space::IMem, addr), s.at(Space::IMem, addr), "{name}");
    }
    for space in [Space::IMem, Space::AMem, Space::MMem, Space::DMem] {
        assert_eq!(r.len(space), s.len(space), "{space:?}");
    }
}

/// Aliases: more names than addresses, and every name at an address is kept.
#[test]
fn one_address_can_have_several_names() {
    let Some(text) = release("ubin/ucadr.sym") else { return };
    let s = parse(&text).unwrap();
    let lines = text.lines().filter(|l| l.contains(" I-MEM ")).count();
    assert_eq!(lines, 2_264, "symbol lines in MIT's file");
    assert_eq!(lines - s.len(Space::IMem), 42, "addresses carrying more than one name");
}

/// An unknown memory is a file the reader does not understand.
#[test]
fn an_unknown_memory_is_an_error() {
    assert!(parse("FOO I-MEM 100\n").is_ok());
    let e = parse("FOO Q-MEM 100\n").unwrap_err();
    assert!(e.contains("Q-MEM"), "{e}");
    let e = parse("FOO I-MEM 99\n").unwrap_err();
    assert!(e.contains("not octal"), "{e}");
}

/// A `NUMBER` is a constant and not a location, and three of MIT's are
/// negative --- which is why they are kept apart from the addresses.
#[test]
fn the_constants_are_signed_and_are_not_addresses() {
    let Some(s) = symbols("ucadr.sym") else { return };
    assert_eq!(s.constant("%MAPPING-TABLE-FLAVOR"), Some(-3));
    assert_eq!(s.constant("%HASH-TABLE-MODULUS"), Some(-6));
    assert_eq!(s.constant("NEGATIVE-SETZ"), Some(-0o100000000));
    assert_eq!(s.constants().len(), 133, "NUMBER lines in MIT's file");
    // COPY-BUFFER-CCW-BLOCK-LENGTH is the count DISK-COPY-SECTION compares
    // against, two instructions after its conditional return.
    assert!(s.constant("COPY-BUFFER-CCW-BLOCK-LENGTH").is_some());
}
