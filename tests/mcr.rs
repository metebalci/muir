// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Checks the MCR reader against the releases' microcode.
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

/// The same file in the System 100 release, for the cross-check below.
fn ubin_100(name: &str) -> Option<Vec<u8>> {
    let p = support::vendor(&["system-100-0", "sys", "ubin", name])?;
    Some(std::fs::read(p).unwrap())
}

/// **The committed microcode is the release's own file, and the same file
/// in both releases.** `src/mcr.rs` says `mit/sys/ubin/ucadr.mcr` is byte
/// for byte the release's `sys/ubin/ucadr.mcr`, and it is what `diskpack`
/// puts on a pack when it is given no file. Two builds of a microcode
/// version can both parse and both run, so only the release's own bytes
/// settle which one this is.
///
/// The two copies reach this test by different routes --- System 100's
/// out of the tarball published with that release, System 304's out of a
/// tarball built from the project's Fossil repository --- which is what
/// makes their agreement worth asserting: microcode 323 is one file across
/// the two, so every fact this project holds about the microcode is true
/// of both, and a release that quietly rebuilt it would fail here.
#[test]
fn the_committed_microcode_is_the_one_both_releases_ship() {
    let mut checked = 0;
    if let Some(theirs) = ubin_100("ucadr.mcr") {
        assert_eq!(theirs.len(), mcr::UCADR_323.len(), "System 100's file differs in length");
        assert!(theirs.as_slice() == mcr::UCADR_323, "mit/sys/ubin/ucadr.mcr is not System 100's");
        checked += 1;
    }
    if let Some(theirs) = ubin("ucadr.mcr") {
        assert_eq!(theirs.len(), mcr::UCADR_323.len(), "System 304's file differs in length");
        assert!(theirs.as_slice() == mcr::UCADR_323, "mit/sys/ubin/ucadr.mcr is not System 304's");
        checked += 1;
    }
    eprintln!("held against {checked} of the two releases");
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

    // **Unverified**: the A memory section the reader lands on is entirely
    // zero, and where this file keeps the PROM's A memory is not
    // established; the 4,508 bytes past its last section are the one place
    // left. What would settle it is the boot PROM's own reader,
    // `PROCESS-SECTION` in `mit/sys/ucadr/promh.text`, run over this
    // file's sections, and the words it writes to A memory compared with
    // the `A-MEM` names in `promh.sym`. Pinned so that a change in the
    // reader shows here.
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
