// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reads the bands on the vendored System 100 pack.
//!
//! Needs `vendor/run/disk-sys-100-0.img`.  Without it these
//! tests report that they were skipped.
//!
//! The pack carries two bands built by different programs --- `LOD1`, a fresh
//! cold load written by `make-cold`, and `LOD2`, a full world written by
//! `DISK-SAVE` --- so each is a check on the other.  A field layout that only
//! fits one of them is wrong.

use std::io::{Read, Seek, SeekFrom};

use muir::band::{Band, Label, Q, cdr, dtp, sys_com};

mod support;
use support::vendor;

fn pack() -> Option<Label> {
    let p = support::pack_100()?;
    Some(Label::open(&p).unwrap())
}

fn band(name: &str) -> Option<Band> {
    let label = pack()?;
    Some(label.band(name).unwrap())
}

/// The label names the bands, and the comments say what wrote them.
///
/// `make-cold` closes by setting the comment to `"cold "` and the date
/// (`coldut.lisp:1215-1218`); `DISK-SAVE` sets it from `SYSTEM-VERSION-INFO`
/// (`qmisc.lisp:1196-1201`).  So the comments are the partition table's own
/// statement of which band is which, independent of anything inside them.
#[test]
fn the_label_names_the_bands_and_says_what_wrote_them() {
    let Some(label) = pack() else { return };

    assert_eq!(label.partitions.len(), 18);
    assert_eq!(label.words_per_descriptor, 7);

    let lod1 = label.partition("LOD1").expect("no LOD1 on the pack");
    assert_eq!((lod1.start, lod1.blocks), (66_828, 24_225));
    assert_eq!(lod1.comment, "cold 3/23/23");

    let lod2 = label.partition("LOD2").expect("no LOD2 on the pack");
    assert_eq!((lod2.start, lod2.blocks), (91_053, 24_225));
    assert_eq!(lod2.comment, "Exp 100.0");

    // A band is the whole partition, and both are the same size: how much of
    // one is live is %SYS-COM-VALID-SIZE's business, not the label's.
    assert_eq!(lod1.blocks, lod2.blocks);

    // The microload partition the boot PROM searches for, carrying muir's
    // target microcode.
    assert_eq!(label.microload_partition, "MCR1");
    assert_eq!(label.partition("MCR1").unwrap().comment, "UCADR 323");
}

/// Every one of the 27 system-communication Qs decodes to something sensible.
///
/// This is the check that the word layout is right.  `SYSTEM-COMMUNICATION-
/// AREA-QS` (`qcom.lisp:138-222`) is 27 long and `coldut.lisp:1046` asserts
/// that count; a wrong split of the Q would show up as data types outside the
/// 29 that exist, or as pointers past the end of the band.
#[test]
fn every_system_communication_q_decodes() {
    for name in ["LOD1", "LOD2"] {
        let Some(band) = band(name) else { return };
        for (i, q) in band.sys_com().iter().enumerate() {
            assert!(
                (q.data_type() as usize) < dtp::NAMES.len(),
                "{name} sys-com {i} has data type {}, past the {} that exist",
                q.data_type(),
                dtp::NAMES.len()
            );
        }
    }
}

/// `LOD1` is a fresh cold load, and says so three separate ways.
///
/// Every value here was read off the band before this test was written.
#[test]
fn lod1_is_a_fresh_uncompressed_cold_load() {
    let Some(b) = band("LOD1") else { return };

    // Uncompressed: 1000 octal is the compressed format and anything else is
    // the plain one (`qcom.lisp:176-180`).  `coldut.lisp:1060` writes 0
    // deliberately --- ";not compressed format".
    assert_eq!(b.band_format(), 0, "LOD1 should be the uncompressed format");
    assert!(!b.compressed());

    assert_eq!(b.valid_size(), 835_584, "words used");
    assert_eq!(b.valid_size() / 256, 3_264, "pages used");

    // Fresh, fingerprint 1: `coldut.lisp:1069` writes 32K with the comment
    // ";assume 32K, fixed later".  A booted world would have fixed it.
    assert_eq!(b.memory_size(), 32_768);
    // Fresh, fingerprint 2: `coldut.lisp:1077` writes NIL with ";I.e. fresh
    // cold-load".  NIL is DTP-SYMBOL pointing at 0.
    let major = b.sys_com()[sys_com::MAJOR_VERSION];
    assert_eq!(major.data_type(), dtp::SYMBOL);
    assert_eq!(major.pointer(), 0, "MAJOR-VERSION is not NIL");
    // Fresh, fingerprint 3: no microcode version --- `coldut.lisp:1078`
    // writes NIL, "Set by system initialization".
    assert_eq!(b.desired_microcode_version(), None);

    assert_eq!(b.wired_size(), 44_032);
    assert_eq!(b.page_table_size(), 32_768);
}

/// `LOD2` is a saved world, and differs from `LOD1` in exactly the places a
/// boot fills in.
///
/// Reading both with one set of accessors is what makes this a test of the
/// layout rather than of `LOD1`.
#[test]
fn lod2_is_a_saved_world_of_system_100() {
    let Some(b) = band("LOD2") else { return };

    // 1000 octal: the compressed format.  So virtual address N is *not* word
    // N of this partition, and the reader must refuse to pretend otherwise.
    assert_eq!(b.band_format(), 0o1000);
    assert!(b.compressed());
    assert_eq!(b.read(0o400), None, "a compressed band has no flat mapping");

    // The three fields LOD1 leaves as placeholders, all filled in.
    assert_eq!(b.sys_com()[sys_com::MAJOR_VERSION].pointer(), 100, "System 100");
    assert_eq!(b.memory_size(), 2_097_152, "8 MB of real memory, not the 32K placeholder");
    assert!(b.highest_virtual_address() > 0, "set in the new band format");

    // Microcode 323 --- muir's target --- stored in the layout `qcom.lisp`
    // warns about: type field at bit 24, so the version is Q<23:0>.
    assert_eq!(b.desired_microcode_version(), Some(323));
}

/// The band states the pointer width that was used to read it.
///
/// `%SYS-COM-POINTER-WIDTH` is "either 24 or 25, as fixnum, or DTP-FREE in old
/// sys" (`qcom.lisp:220`).  Both bands say 25, which is the width
/// `%%Q-POINTER` gives, so the field that describes the layout was itself read
/// correctly by that layout.
#[test]
fn the_bands_state_the_pointer_width_they_were_read_with() {
    for name in ["LOD1", "LOD2"] {
        let Some(b) = band(name) else { return };
        assert_eq!(b.pointer_width(), Some(25), "{name}");
        assert_eq!(b.pointer_width(), Some(muir::band::POINTER_BITS), "{name}");
    }
}

/// The two table pointers are locatives, and they land inside the live band.
///
/// Note this does *not* pin the width of the data-type field: every Q in the
/// system communication area has a zero CDR code, so a reading that swallowed
/// those two bits would pass here unchanged.
/// `nil_is_built_the_way_make_t_and_nil_builds_it` is what pins the width.
#[test]
fn the_area_and_page_tables_are_locatives_inside_the_band() {
    let Some(b) = band("LOD1") else { return };

    for (what, q) in [
        ("AREA-ORIGIN-PNTR", b.sys_com()[sys_com::AREA_ORIGIN_PNTR]),
        ("PAGE-TABLE-PNTR", b.sys_com()[sys_com::PAGE_TABLE_PNTR]),
    ] {
        assert_eq!(q.data_type(), dtp::LOCATIVE, "{what} is not a locative");
        assert!(q.pointer() > 0, "{what} is null");
        assert!(q.pointer() < b.valid_size(), "{what} points past the live band");
        assert!(b.read(q.pointer()).is_some(), "{what} is not readable");
    }

    // And they are where the reference table says.
    assert_eq!(b.sys_com()[sys_com::AREA_ORIGIN_PNTR].pointer(), 0o3400);
    assert_eq!(b.sys_com()[sys_com::PAGE_TABLE_PNTR].pointer(), 0o5400);
}

/// **A partition whose first block is the last block number there is has
/// no block after it, and says so.** The label is data read off the pack,
/// and [`Band::open`] reads the block after a partition's first for the
/// system communication area; on a start of `u32::MAX` that block has no
/// number, which is an error and not an overflow.
#[test]
fn a_partition_starting_at_the_last_block_number_is_refused() {
    let dir = std::env::temp_dir().join(format!("muir-band-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hostile.img");
    // Block 0 as `DECODE-LABEL` reads it: `LABL`, version 1, and at 0o200
    // one descriptor of seven words --- name, start, blocks, comment.
    let mut label = vec![0u32; 256];
    label[0] = u32::from_le_bytes(*b"LABL");
    label[1] = 1;
    label[0o200] = 1;
    label[0o201] = 7;
    label[0o202] = u32::from_le_bytes(*b"LOD1");
    label[0o203] = u32::MAX;
    label[0o204] = 1;
    let bytes: Vec<u8> = label.iter().flat_map(|w| w.to_le_bytes()).collect();
    std::fs::write(&path, &bytes).unwrap();
    let label = Label::open(&path).unwrap();
    let p = label.partition("LOD1").expect("the one partition");
    assert_eq!((p.start, p.blocks), (u32::MAX, 1));
    let err = label.band("LOD1").unwrap_err();
    assert!(err.contains(&u32::MAX.to_string()), "the error names the block: {err}");
    std::fs::remove_dir_all(&dir).ok();
}

/// Reading past the live part of the band is refused, not silently zero.
#[test]
fn reads_outside_the_valid_size_are_refused() {
    let Some(b) = band("LOD1") else { return };
    assert!(b.read(b.valid_size() - 1).is_some());
    assert_eq!(b.read(b.valid_size()), None);
    assert_eq!(b.read(u32::MAX), None);
}

/// NIL is built where and how `make-t-and-nil` builds it.
///
/// This is the test that pins the width of the data-type field, because these
/// are the first Qs on the band with a CDR code that is not zero.
///
/// `coldut.lisp:1283-1291` allocates NIL first, at the base of
/// `RESIDENT-SYMBOL-AREA`, so its pointer is 0 --- which is what makes
/// [`Q::is_nil`] a pointer comparison --- and writes all five words of the
/// symbol head with `CDR-NEXT`.  Read with the five bits at `Q<29:25>` those
/// words are a symbol header, a value cell, a function cell, a property list
/// and a package cell.  Read with seven bits they are types 100, 99, 97, 110
/// and 99, none of which exist: `Q-DATA-TYPES` stops at 28.
#[test]
fn nil_is_built_the_way_make_t_and_nil_builds_it() {
    let Some(b) = band("LOD1") else { return };

    let head: Vec<Q> = (0..5).map(|a| b.read(a).unwrap()).collect();
    for (i, q) in head.iter().enumerate() {
        assert_eq!(q.cdr_code(), cdr::NEXT, "NIL word {i} was written with CDR-NEXT");
        assert!((q.data_type() as usize) < dtp::NAMES.len(), "NIL word {i} has no such data type");
    }

    // `(vwrite-cdr qnil sym:cdr-next (vmake-pointer sym:dtp-symbol-header
    //   (store-string 'sym:p-n-string "NIL")))`
    assert_eq!(head[0].data_type(), dtp::SYMBOL_HEADER);
    // The value cell of NIL is NIL, and so is its package cell.
    assert!(head[1].is_nil(), "NIL's value cell");
    assert!(head[4].is_nil(), "NIL's package cell");
    // `(vmake-pointer sym:dtp-null qnil)`: undefined, not NIL.
    assert_eq!(head[2].data_type(), dtp::NULL, "NIL's function cell");
    assert_eq!(head[2].pointer(), 0);

    // The print name the symbol header points at.  Its array header is one
    // word, so the characters start at the next --- packed four to a word, low
    // byte first, the same way round as everything else on the pack.
    let name = b.read(head[0].pointer() + 1).unwrap().0.to_le_bytes();
    assert_eq!(&name[..3], b"NIL", "NIL's print name");
}

/// Almost every word of the live band has a data type that exists.
///
/// The band is 835,584 words and 28 of the 32 codes the field can hold are
/// defined, so a wrong field width shows up as a flood of impossible types.
/// Read at `Q<29:25>`, 1,192 words are out of range --- 0.14%, and those are
/// the unboxed ones: string characters, `FEF` macrocode, numeric arrays, which
/// carry no tag at all.  Read at `Q<31:25>` it is 237,598 words, 28%.
#[test]
fn nearly_every_word_of_the_band_has_a_data_type_that_exists() {
    let Some(b) = band("LOD1") else { return };

    let total = b.valid_size();
    let out_of_range =
        (0..total).filter(|&a| b.read(a).unwrap().data_type() as usize >= dtp::NAMES.len()).count();

    assert!(
        out_of_range * 100 < total as usize,
        "{out_of_range} of {total} words have no such data type, over 1%: \
         the data-type field is being read at the wrong width or position"
    );
}

/// The band this file reads out of the pack is the band the release ships.
///
/// `vendor/run/disk-sys-100-0.img` is not pristine: a pack is opened
/// read-write and a booted machine writes to it, so it carries whatever the
/// runs made against it wrote.  Every test above would still pass on a pack
/// whose `LOD1` had been scribbled on.
///
/// So this is the same band by a second route --- `LOD1-cold-3-23-23.gz`
/// straight from the release --- and it is bit-for-bit identical, which says
/// both that the fixture is intact and that no run has written to `LOD1`.
#[test]
fn the_pack_carries_the_bands_the_release_ships() {
    let (Some(pack), Some(archive)) =
        (support::pack_100(), vendor(&["system-100-0", "LOD1-cold-3-23-23.gz"]))
    else {
        return;
    };

    let label = Label::open(&pack).unwrap();
    let p = label.partition("LOD1").unwrap();

    // No gzip in the tree --- muir has no dependencies --- so unpack with the
    // system's, and skip rather than fail if it is not there.
    let Ok(out) = std::process::Command::new("gunzip").arg("-c").arg(&archive).output() else {
        eprintln!("skipped: no gunzip");
        return;
    };
    assert!(out.status.success(), "gunzip failed on {}", archive.display());

    let mut f = std::fs::File::open(&pack).unwrap();
    f.seek(SeekFrom::Start(p.start as u64 * 1024)).unwrap();
    let mut from_pack = vec![0u8; p.blocks as usize * 1024];
    f.read_exact(&mut from_pack).unwrap();

    assert_eq!(out.stdout.len(), from_pack.len(), "the archive is not one partition long");
    assert!(out.stdout == from_pack, "LOD1 on the pack is not the LOD1 the release ships");
}
