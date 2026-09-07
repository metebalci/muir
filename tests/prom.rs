// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Checks the microinstruction decoder against the boot PROM.
//!
//! The engines boot MIT's own microcode file, `mit/sys/ubin/promh.mcr`,
//! which is byte for byte what the releases ship as `sys/ubin/promh.mcr`;
//! `the_committed_prom_is_the_one_system_100_ships` holds it to that when
//! the release is vendored, and says it was skipped when not.  What is
//! checked here is that it is the *right* PROM --- two builds of "version
//! 9" exist and comparing two disassemblies of one of them proves nothing
//! --- and that the burned form derived from it is the shape the 74S472s
//! actually hold.

use muir::isa::Op;
use muir::prom::{self, PROM_WORDS};

mod support;
use support::mcr;

#[test]
fn the_prom_is_512_words() {
    assert_eq!(prom::boot_prom().len(), PROM_WORDS);
    assert_eq!(prom::boot_prom_image().len(), PROM_WORDS);
}

/// **The committed boot PROM is the release's own file.** `src/prom.rs`
/// says `mit/sys/ubin/promh.mcr` is byte for byte the release's
/// `sys/ubin/promh.mcr`; two builds of version 9 exist, so a copy that
/// drifted would still parse and still boot, and only the release's own
/// bytes settle which one the engines run.
#[test]
fn the_committed_prom_is_the_one_system_100_ships() {
    let Some(theirs) = support::release_100_file(&["ubin", "promh.mcr"]) else { return };
    let theirs = std::fs::read(&theirs).unwrap();
    let ours = include_bytes!("../mit/sys/ubin/promh.mcr");
    assert_eq!(theirs.len(), ours.len(), "the two files differ in length");
    assert!(theirs.as_slice() == ours.as_slice(), "mit/sys/ubin/promh.mcr is not the release's");
}

/// Every word of the programming image carries odd parity over bits 46:0.
///
/// This is what makes it a PROM *image* rather than raw microinstructions:
/// the 74S472 stores 48 bits, and the boot PROM's parity is checked on the
/// board by the 74S280s at PARITY.  An unburned word is zero with parity 1,
/// which is legal, so the rule holds over all 512 and not only the 0o706
/// version 9 defines.
#[test]
fn the_image_carries_odd_parity() {
    for (addr, w) in prom::boot_prom_image().iter().enumerate() {
        let stored = (w >> 47) & 1;
        let count = (w & ((1 << 47) - 1)).count_ones();
        assert_eq!(stored, 1 - (count & 1) as u64, "word {addr:o} has bad parity: {w:012x}");
    }
}

/// Location 0 is the reset entry point and jumps to the label `GO` at 0o45.
/// A named, hand-checked anchor: if the JUMP field positions drift, this trips.
#[test]
fn reset_vector_jumps_to_go() {
    let insn = prom::boot_prom()[0];
    assert_eq!(insn.op(), Op::Jump);
    let j = insn.jump();
    assert_eq!(j.target, 0o45, "reset vector target");
    assert!(j.internal_cond, "jump-always uses an internal condition");
    assert_eq!(j.cond, 7, "condition 7 is unconditional");
    assert!(!j.p && !j.r, "reset vector is a plain jump");
}

/// Every label the symbol table names is defined in the committed source.
///
/// It is what ties `mit/sys/ucadr/promh.text` to the addresses everything else
/// talks in: the comparison reports a PC, and the label is how you find it.
#[test]
fn the_committed_source_defines_every_label() {
    let syms = include_str!("../mit/sys/ubin/promh.sym");
    let source = include_str!("../mit/sys/ucadr/promh.text");
    // A label sits at the start of its own line, but the two that bracket
    // the PROM-only entry point are wrapped: `(IF PROM BEG)`. The same file
    // assembles to either the PROM or the control store, and those two lines
    // are what differ.
    let defined: std::collections::BTreeSet<&str> = source
        .lines()
        .filter(|l| !l.starts_with(char::is_whitespace) && !l.starts_with(';'))
        .map(|l| {
            let l = l.strip_prefix("(IF PROM ").unwrap_or(l);
            l.strip_prefix("(IF (NOT PROM) ").unwrap_or(l)
        })
        .filter(|l| !l.starts_with('('))
        .map(|l| l.split_whitespace().next().unwrap_or("").trim_end_matches(')'))
        .collect();
    let mut missing = Vec::new();
    let mut labels = 0;
    for line in syms.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.len() >= 3 && t[t.len() - 2] == "I-MEM" {
            labels += 1;
            if !defined.contains(t[t.len() - 3]) {
                missing.push(t[t.len() - 3].to_string());
            }
        }
    }
    eprintln!("{labels} code labels, {} not found in the source", missing.len());
    assert!(missing.is_empty(), "labels with no definition: {missing:?}");
}

/// The boot PROM source here is MIT's original, not a fast variant.
///
/// A fast variant of this source exists --- the same file with its first jump
/// changed to enter at `FUDGE-INITIAL-DISK-PARAMETERS` rather than `GO`, so
/// that memory initialisation is skipped and a boot is quicker. That is a
/// reasonable thing for an emulator to offer and the wrong thing to be
/// running here, because the memory initialisation *is* the part being
/// checked: it is where `chip` and `rtl` have been compared for half a
/// million microcycles.
///
/// The two differ in one line out of 1300, so nothing about a copy announces
/// which it is. Hence this: the entry point is pinned, and a file that
/// quietly turns into the fast variant shows up here rather than as a boot
/// that mysteriously skips half its work.
#[test]
fn the_prom_source_is_the_original_not_the_fast_variant() {
    let original = include_str!("../mit/sys/ucadr/promh.text");

    assert!(original.contains("(IF PROM (JUMP GO))"), "the original enters at GO");
    assert!(
        !original.contains("(IF PROM (JUMP FUDGE-INITIAL-DISK-PARAMETERS))"),
        "mit/sys/ucadr/promh.text is the fast variant, which skips memory initialisation"
    );
}

/// Burning a word and reading it back is the identity, and the statistics
/// bit does not survive it.
///
/// `IR<46>` has nowhere to live in a 48-bit image that spends bit 47 on
/// parity and bit 46 on `IR<47>`, so a word carrying it burns the same as
/// one without. No word of the boot PROM sets it, which is why the round
/// trip below holds over the whole PROM.
#[test]
fn a_microinstruction_is_burned_the_way_the_image_shows() {
    let insns = prom::boot_prom();
    let image = prom::boot_prom_image();
    for (k, i) in insns.iter().enumerate() {
        assert_eq!(prom::unprogramming(image[k]), *i, "word {k:o} does not read back");
        assert_eq!(i.raw() >> 46 & 1, 0, "word {k:o} sets the statistics bit");
    }
    let with_stat = muir::isa::Insn::new(1 << 46 | 1 << 47 | 0o1234567);
    let without = muir::isa::Insn::new(1 << 47 | 0o1234567);
    assert_eq!(prom::programming(with_stat), prom::programming(without));
    assert_eq!(prom::unprogramming(prom::programming(with_stat)), without);
}

/// The reader `--prom` hands a file to is the one the built-in PROM comes
/// through, so a run with `--prom mit/sys/ubin/promh.mcr` is the default
/// run and not a second path to it.
#[test]
fn the_built_in_prom_is_read_the_way_a_named_file_is() {
    let bytes = include_bytes!("../mit/sys/ubin/promh.mcr");
    assert_eq!(prom::parse_mcr(bytes).unwrap(), prom::boot_prom());
}

/// A file that is not an MCR at all comes back as an error and not a
/// panic: `--prom` names it in a usage message, and muir exits rather
/// than starting a machine on nothing.
#[test]
fn a_file_that_is_not_an_mcr_is_an_error_and_not_a_panic() {
    for bad in [b"".as_slice(), b"not an MCR file", b"\x00\x00\x00\x00"] {
        let err = prom::parse_mcr(bad).unwrap_err();
        assert!(
            err.contains("not an MCR"),
            "the message says so, rather than naming a section header: {err}"
        );
    }
}

/// A microcode file with no control store section in it holds no program,
/// whatever else it carries, and 512 unburned zero words are not a boot
/// PROM. Refused rather than run.
#[test]
fn a_file_with_no_program_in_it_is_refused() {
    let err = prom::parse_mcr(&mcr(&[])).unwrap_err();
    assert!(err.contains("control store"), "the message says what is missing: {err}");
}

/// The PROM is the 1K words page PCTL decodes and the chips hold no more, so
/// a program that overruns them is refused rather than quietly cut off at
/// the end.
#[test]
fn a_program_too_long_for_the_chips_is_refused() {
    assert!(prom::parse_mcr(&mcr(&vec![0; PROM_WORDS])).is_ok());
    let err = prom::parse_mcr(&mcr(&vec![0; PROM_WORDS + 1])).unwrap_err();
    assert!(err.contains("1025"), "the message says how long it is: {err}");
}

/// The boot PROM is fetched from address 0 --- word 0 is the reset entry
/// point --- so a file that assembles somewhere else is not one.
#[test]
fn a_program_that_does_not_start_at_zero_is_refused() {
    let mut m = mcr(&[0, 0]);
    // The control store section's start, the second header word.
    m[4..8].copy_from_slice(&[0, 0, 0o45, 0]);
    let err = prom::parse_mcr(&m).unwrap_err();
    assert!(err.contains("45"), "the message says where it starts: {err}");
}

/// **An MIT listing addresses a PROM, and an address past the part is an
/// error, not an allocation.** The listings [`prom::parse_mit`] reads are
/// burned into 74S288s, 74S287s and 74S472s, the last of them the largest
/// at 512 words, so no listing addresses past `0o777`; a corrupt one that
/// does is refused where the words would otherwise be made room for, sixty
/// bits of address being an exabyte.
#[test]
fn a_listing_addressing_past_the_part_is_refused() {
    let words = prom::parse_mit("title\n777 1\n").unwrap();
    assert_eq!(
        words.len(),
        prom::CHIP_WORDS,
        "the last word of a 74S472 sizes the image to the part"
    );
    assert_eq!(words[0o777], 1);
    let err = prom::parse_mit("title\n1000 1\n").unwrap_err();
    assert!(err.contains("1000") && err.contains("512"), "{err}");
    let err = prom::parse_mit("title\n77777777777777777777 1\n").unwrap_err();
    assert!(err.contains("512"), "{err}");
    assert!(prom::parse_mit("title\n7777777777777777777777 1\n").is_err(), "wider than usize");
}

/// A word setting the statistics bit is refused on every engine.
///
/// `IR<46>` has nowhere to live in the burned image --- bit 47 is parity
/// and bit 46 is `IR<47>`, as [`prom::programming`] says --- so `chip`,
/// which runs what the 74S472s hold, would drop it while `micro` and
/// `rtl`, which fetch the microinstruction, would keep it. That is one
/// file running as two programs, and the engines are compared against
/// each other, so it is refused at the door instead.
#[test]
fn a_word_with_the_statistics_bit_is_refused() {
    assert!(prom::parse_mcr(&mcr(&[0, 0o1234567])).is_ok());
    let err = prom::parse_mcr(&mcr(&[0, 1 << 46 | 0o1234567])).unwrap_err();
    assert!(err.contains("46"), "the message names the bit: {err}");
    assert!(err.contains('1'), "and the word: {err}");
}
