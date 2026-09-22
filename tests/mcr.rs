// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Checks the MCR reader against the release's microcode.
//!
//! These files are AGPL and large, so they live in `vendor/` and are not part
//! of the repository.  Run `tools/fetch-system-100.sh` to get them; without
//! them these tests report that they were skipped.

use muir::isa::Op;
use muir::mcr;

mod support;
/// A file of `SYS: UBIN;` in the release this project targets.
fn ubin(name: &str) -> Option<Vec<u8>> {
    Some(std::fs::read(support::release_100_file(&["ubin", name])?).unwrap())
}

/// **The committed microcode is the release's own file.** `src/mcr.rs`
/// says `mit/sys/ubin/ucadr.mcr` is byte for byte the release's
/// `sys/ubin/ucadr.mcr`, and it is what `diskpack` puts on a pack when it
/// is given no file. Two builds of a microcode version can both parse and
/// both run, so only the release's own bytes settle which one this is.
#[test]
fn the_committed_microcode_is_the_one_the_release_ships() {
    let Some(theirs) = ubin("ucadr.mcr") else { return };
    assert_eq!(theirs.len(), mcr::UCADR_323.len(), "the two files differ in length");
    assert!(theirs.as_slice() == mcr::UCADR_323, "mit/sys/ubin/ucadr.mcr is not the release's");
}

/// **The built-in microcode is microcode 323, and parses.** Committed, so
/// this one does not skip: it is the file every pack made here loads.
#[test]
fn the_built_in_microcode_is_323() {
    let m = mcr::parse(mcr::UCADR_323).expect("mit/sys/ubin/ucadr.mcr");
    assert_eq!(m.imem_start, 0);
    assert_eq!(m.imem.len(), 12_449, "0o30241 control store words, as microcode 323 has");
    assert_eq!(m.dmem.len(), 0o4000, "dispatch memory words");
    assert_eq!(m.amem.len(), 0o2000, "A memory words");
}

/// The boot PROM as the release ships it, with the section sizes its own
/// headers give.
#[test]
fn the_release_prom_parses() {
    let Some(bytes) = ubin("promh.mcr") else {
        return;
    };
    let m = mcr::parse(&bytes).unwrap();
    assert_eq!(m.imem_start, 0);
    assert_eq!(m.imem.len(), 0o706, "control store words");
    assert_eq!(m.dmem.len(), 0o4000, "dispatch memory words");
    assert_eq!(m.amem.len(), 0o2000, "A memory words");
    assert_eq!(m.trailing_bytes, 4508, "bytes past the last section");

    // **The A memory section is all zero because that is what the file
    // holds, and the reader is right.** Settled by reading the boot PROM's
    // own loader, `mit/sys/ucadr/promh.text`, which is what the earlier
    // note here said would settle it.
    //
    // `PROCESS-A-MEM-SECTION` does not write A memory at all. It sets
    // `PDL-BUFFER-POINTER` to the section's start less one and pushes each
    // word onto the PDL buffer; when the count runs out it **falls into**
    // `DONE-LOADING` rather than returning to `PROCESS-SECTION`, and
    // `DONE-LOADING` copies the PDL buffer into M memory and then into A
    // memory. So the A memory section is terminal by construction, it must
    // be exactly start 0 and size 0o2000 to fill the 1K buffer, and the
    // PROM never reads a byte past it. Breaking out of the loop at code 4,
    // which `src/mcr.rs` does, is the PROM's own behavior.
    //
    // The zeros are the assembler's too: in `promh.text`'s `(LOCALITY
    // A-MEM)` every cell is declared `(0)` and the PROM builds its
    // constants at run time with `CLEAR-A-MEMORY` and `MAKE-CONSTANTS`.
    // Run over `sys/ubin/ucadr.mcr` instead, the same reader lands on 341
    // non-zero words of 1024, so it finds data whenever there is data.
    //
    // The 4,508 trailing bytes are the microcode symbol area, not A memory.
    // `sys/sys/qwmcr.lisp`'s `WRITE-MCR-FILE` writes the sections and then
    // pads with zero halfwords to a 1024-byte page before the symbol image:
    // the sections end at byte 15,972, 412 bytes of padding reach 16,384,
    // and the code-3 header's own fields say 4 blocks at block 16. 412 plus
    // 4,096 is 4,508, and the same arithmetic closes for `ucadr.mcr`.
    assert!(m.amem.iter().all(|&w| w == 0), "A memory read from the stream is not zero after all");

    // Hand-checked anchor: location 0 jumps to GO.
    assert_eq!(m.imem[0].raw(), 0o0200000000450247);
    assert_eq!(m.imem[0].op(), Op::Jump);
    assert_eq!(m.imem[0].jump().target, 0o45);
}

/// The whole of microcode 323 --- about 12k words --- must parse and decode.
#[test]
fn the_release_microcode_parses() {
    let Some(bytes) = ubin("ucadr.mcr") else {
        return;
    };
    let m = mcr::parse(&bytes).unwrap();
    assert_eq!(m.imem.len(), 0o30241, "control store words");

    let mut classes = [0usize; 4];
    for insn in &m.imem {
        assert_eq!(insn.raw() >> 48, 0, "word wider than 48 bits");
        classes[match insn.op() {
            Op::Alu => 0,
            Op::Jump => 1,
            Op::Dispatch => 2,
            Op::Byte => 3,
        }] += 1;
        // Every class must decode without panicking.
        match insn.op() {
            Op::Alu => drop(insn.alu()),
            Op::Jump => drop(insn.jump()),
            Op::Dispatch => drop(insn.dispatch()),
            Op::Byte => drop(insn.byte()),
        }
    }
    eprintln!(
        "microcode 323: {} alu, {} jump, {} dispatch, {} byte",
        classes[0], classes[1], classes[2], classes[3]
    );
    assert!(classes.iter().all(|&n| n > 0), "every class should occur: {classes:?}");
}

/// **A microcode file says which version it is**: A memory's word 40 is
/// `%MICROCODE-VERSION-NUMBER`, the first of the locations System 100's
/// `sys/cold/qcom.lisp` lists "IN ORDER OF CONTENTS OF A-MEMORY STARTING AT
/// 40", and `uc-parameters.lisp` assembles it as `A-VERSION`, a fixnum of
/// the "VERSION NUMBER FROM SECOND FILE NAME OF SOURCE".
#[test]
fn the_built_in_microcode_says_it_is_323() {
    let m = mcr::parse(mcr::UCADR_323).unwrap();
    assert_eq!(m.version(), Some(323));
    assert_eq!(mcr::parse(&support::mcr(&[0; 4])).unwrap().version(), None, "no A memory");
}

/// **A control store section that runs past the control store is
/// refused**, as the boot PROM refuses it: `PROCESS-I-MEM-SECTION` in
/// `mit/sys/ucadr/promh.text` takes `(BYTE-FIELD 18. 14.)` of every
/// address and jumps to `ERROR-BAD-ADDRESS` if it is not zero.
#[test]
fn a_control_store_section_past_16k_words_is_refused() {
    assert!(mcr::parse(&support::mcr(&vec![0; 0o40000])).is_ok(), "16K words fit");
    let err = mcr::parse(&support::mcr(&vec![0; 0o40001])).unwrap_err();
    assert!(err.contains("control store"), "{err}");
}

/// **An A memory section that starts past A memory is refused**:
/// `PROCESS-A-MEM-SECTION` checks `(BYTE-FIELD 22. 10.)` of its start the
/// same way. It checks the start and nothing after it, so a section that
/// starts inside A memory is the PROM's to load however long it is.
#[test]
fn an_a_memory_section_starting_past_1k_words_is_refused() {
    let mut b = support::mcr(&[0; 4]);
    // The last word of the file is the A section's size and the one before
    // it the start, in PDP-11 order: the high half first, low byte first.
    let at = b.len() - 8;
    b[at..at + 4].copy_from_slice(&[0, 0, 0, 4]);
    let err = mcr::parse(&b).unwrap_err();
    assert!(err.contains("A memory"), "{err}");
}
