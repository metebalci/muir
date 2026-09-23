// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! QUUX, the evolved CADR, `--machine quux`: where it differs from the CADR,
//! on both engines that model it.
//!
//! Its level-1 map entry is six bits, not five: 64 blocks of 32 level-2
//! entries, so 63 regions of 8K words can be mapped at once, block 77 being
//! the invalid one, against the CADR's 31. The two bits the CADR leaves spare
//! carry the sixth: `MAP(MD)<29>`, which reads 0 on the CADR, and `VMA<24>`,
//! which a map store on the CADR ignores. Everything else is the CADR's.

use muir::engine::Engine;
use muir::isa::Insn;
use muir::isa::asm::{ALU, MD, SETM, a_dest, filler, m_src, src};
use muir::machine::{Geometry, Machine};
use muir::micro::Micro;
use muir::rtl::Rtl;

/// Both engines on the same boot PROM, the machine prepared by `set`.
fn both(prom: &[Insn], set: &dyn Fn(&mut Machine), steps: usize) -> (Micro, Rtl) {
    let make = || {
        let mut m = Machine::new();
        let mut words = vec![filler(); 512];
        words[..prom.len()].copy_from_slice(prom);
        m.load_prom(&words);
        set(&mut m);
        m
    };
    let mut e = Micro::new(make());
    e.boot();
    let mut r = Rtl::new(make());
    r.boot();
    for _ in 0..steps {
        e.step().unwrap();
        r.step().unwrap();
    }
    (e, r)
}

/// `WRITE-MAP`, functional destination 23.
const WRITE_MAP: u64 = (0o23 << 19) | (0o37 << 14);

/// A level-1 write of `entry` at the level-1 index of `MD`, as QUUX takes
/// it: bits 4:0 in `VMA<31:27>`, bit 5 in `VMA<24>`, `VMA<26>` enabling.
fn level_1_store(entry: u32) -> u32 {
    ((entry & 0o37) << 27) | (1 << 26) | (((entry >> 5) & 1) << 24)
}

/// **QUUX writes a six-bit level-1 entry and reads it back in
/// `MAP(MD)<29:24>`.** The CADR, given the same store, keeps five bits and
/// reads bit 29 as 0.
#[test]
fn a_six_bit_level_1_entry_is_written_and_read_back() {
    let prom = [
        Insn::new(ALU | SETM | m_src(1) | MD),
        Insn::new(ALU | SETM | m_src(2) | WRITE_MAP),
        filler(),
        filler(),
        Insn::new(ALU | SETM | src(0o11) | a_dest(0o200)),
    ];
    for (geometry, want) in [(Geometry::QUUX, 0o41), (Geometry::CADR, 0o01)] {
        let set = |m: &mut Machine| {
            m.geometry = geometry;
            // Level-1 index 100.
            m.mmem[1] = 0o100 << 13;
            m.mmem[2] = level_1_store(0o41);
        };
        let (e, r) = both(&prom, &set, 30);
        for (name, m) in [("micro", e.machine()), ("rtl", r.machine())] {
            assert_eq!(m.l1_map[0o100], want, "{geometry:?}, {name}: the entry stored");
            assert_eq!((m.amem[0o200] >> 24) & 0o77, want, "{geometry:?}, {name}: MAP(MD)<29:24>");
        }
    }
}

/// **A translation goes through a block above 37.** Level-1 entry 41, a
/// level-2 entry in block 41, and a read through it lands on the physical
/// page that entry names, on both engines.
#[test]
fn a_translation_goes_through_a_block_above_37() {
    use muir::isa::asm::{SRC_MD, START_READ, m_dest};
    let prom = [
        Insn::new(ALU | SETM | m_src(1) | START_READ),
        filler(),
        filler(),
        filler(),
        filler(),
        filler(),
        filler(),
        filler(),
        filler(),
        filler(),
        Insn::new(ALU | SETM | SRC_MD | m_dest(3)),
    ];
    let set = |m: &mut Machine| {
        m.geometry = Geometry::QUUX;
        // Virtual address: level-1 index 7, level-2 slot 2, word 5.
        m.mmem[1] = (7 << 13) | (2 << 8) | 5;
        m.l1_map[7] = 0o41;
        m.l2_map[(0o41 << 5) | 2] = (1 << 23) | (1 << 22) | 0o100;
        m.main[(0o100 << 8) | 5] = 0o1234567;
    };
    let (e, r) = both(&prom, &set, 60);
    assert_eq!(e.machine().mmem[3], 0o1234567, "micro");
    assert_eq!(r.machine().mmem[3], 0o1234567, "rtl");
}

/// **QUUX says what it is in functional source 16, its MACHINE-ID.** The
/// word is the
/// signature `0x5155` in bits 31:16, the hardware revision in 15:4 and the
/// processor type, 4, in 3:0. On the CADR no part drives the M bus for
/// source 16 and it reads all ones, as `chip` shows
/// (`tests/output_bus.rs`), which can never carry the signature. Source 36
/// is 16, `IR<30>` being in no source decode. Source 17 is QUUX's tick
/// (`tests/tick.rs`), 0 while it is off, and open on the CADR.
#[test]
fn quux_answers_its_id_in_source_16() {
    let prom = [
        Insn::new(ALU | SETM | src(0o16) | a_dest(0o201)),
        Insn::new(ALU | SETM | src(0o36) | a_dest(0o202)),
        Insn::new(ALU | SETM | src(0o17) | a_dest(0o203)),
    ];
    let id = (0x5155 << 16) | (4 << 4) | 4;
    for (geometry, want) in [(Geometry::QUUX, [id, id, 0]), (Geometry::CADR, [!0, !0, !0])] {
        let (e, r) = both(&prom, &|m: &mut Machine| m.geometry = geometry, 30);
        for (name, m) in [("micro", e.machine()), ("rtl", r.machine())] {
            let got = [m.amem[0o201], m.amem[0o202], m.amem[0o203]];
            assert_eq!(got, want, "{geometry:?}, {name}");
        }
    }
    assert_eq!(Geometry::QUUX.machine_id, Some(id));
    assert_eq!(Geometry::CADR.machine_id, None);
}

/// **QUUX lists its sizes in its feature page**, the Xbus I/O page at
/// physical `17377000`, just below the page the display and the disk
/// controller share: word 0 the MACHINE-ID again, then the level-1
/// entry's bits, the level-2 map's entries, the PDL buffer's words, and the
/// control store's, A memory's and dispatch memory's, then which of the
/// multiply and divide it has (bit 0 `MUL`, bit 1 `DIV`), and whether it has
/// the tick; the rest reads 0. On
/// the CADR nothing answers there, and a read times out as any read of an
/// empty I/O address does, the Xbus NXM bit set.
#[test]
fn quux_lists_its_sizes_in_its_feature_page() {
    use muir::isa::asm::{SRC_MD, START_READ, filler};
    use muir::machine::bus_error;
    // Virtual page 1 on the feature page; M 1 to M 8 the addresses of words
    // 0 to 10 and 100 of it, each read into A 200 up.
    let words = [0u32, 1, 2, 3, 4, 5, 6, 7, 0o10, 0o100];
    let mut prom = Vec::new();
    for (k, _) in words.iter().enumerate() {
        prom.push(Insn::new(ALU | SETM | m_src(1 + k as u64) | START_READ));
        prom.extend([filler(); 12].map(|f| f));
        prom.push(Insn::new(ALU | SETM | SRC_MD | a_dest(0o200 + k as u64)));
    }
    let set = |geometry: Geometry| {
        move |m: &mut Machine| {
            m.geometry = geometry;
            m.l2_map[1] = (1 << 23) | (1 << 22) | 0o36776;
            for (k, &w) in words.iter().enumerate() {
                m.mmem[1 + k] = (1 << 8) | w;
            }
        }
    };
    let id = Geometry::QUUX.machine_id.unwrap();
    let want = [id, 6, 2048, 16384, 16384, 1024, 2048, 3, 1, 0];
    let (e, r) = both(&prom, &set(Geometry::QUUX), 400);
    for (name, m) in [("micro", e.machine()), ("rtl", r.machine())] {
        let got: Vec<u32> = (0..words.len()).map(|k| m.amem[0o200 + k]).collect();
        assert_eq!(got, want, "QUUX, {name}");
        assert_eq!(m.bus_error & bus_error::XBUS_NXM, 0, "QUUX, {name}: no NXM");
    }
    let (e, r) = both(&prom[..14], &set(Geometry::CADR), 400);
    for (name, m) in [("micro", e.machine()), ("rtl", r.machine())] {
        assert_ne!(m.bus_error & bus_error::XBUS_NXM, 0, "CADR, {name}: the read timed out");
    }
}

/// **QUUX's PDL buffer can be 4K or 16K words**, its pointer and index 12
/// or 14 bits where the CADR's are 10: a push past word 1777 lands above
/// it rather than wrapping to 0, the pointer reads back whole in source 2,
/// and it wraps at the buffer's own size. The CADR, on the same program,
/// wraps at 1,024.
#[test]
fn quux_s_pdl_buffer_is_4k_or_16k() {
    use muir::isa::asm::{SETO, SETZ};
    // A functional destination, with the harmless M word 31.
    let fdest = |d: u64| (d << 19) | (0o37 << 14);
    // Pointer to M 1, push ones, read the pointer and the word at it.
    let prom = [
        Insn::new(ALU | SETM | m_src(1) | fdest(0o14)),
        Insn::new(ALU | SETO | fdest(0o11)),
        filler(),
        Insn::new(ALU | SETM | src(0o2) | a_dest(0o201)),
        Insn::new(ALU | SETM | src(0o25) | a_dest(0o202)),
        Insn::new(ALU | SETZ | a_dest(0o203)),
    ];
    for (bits, start, after) in [
        (10u32, 0o1777u32, 0u32),
        (12, 0o1777, 0o2000),
        (12, 0o7777, 0),
        (14, 0o7777, 0o10000),
        (14, 0o37777, 0),
    ] {
        let geometry =
            if bits == 10 { Geometry::CADR } else { Geometry { pdl_bits: bits, ..Geometry::QUUX } };
        let set = |m: &mut Machine| {
            m.geometry = geometry;
            m.mmem[1] = start;
        };
        let (e, r) = both(&prom, &set, 30);
        for (name, m) in [("micro", e.machine()), ("rtl", r.machine())] {
            assert_eq!(m.amem[0o201], after, "{bits} bits from {start:o}, {name}: the pointer");
            assert_eq!(m.amem[0o202], !0, "{bits} bits from {start:o}, {name}: the word pushed");
            assert_eq!(
                m.pdl[after as usize], !0,
                "{bits} bits from {start:o}, {name}: where it went"
            );
        }
    }
}

/// **QUUX's boot PROM is MIT's up to its changes.** `data/quux-promh.mcr`
/// is PROM version 1000, built by muir-sys from MIT's `promh.text`: its
/// `FILL-A-LOOP` stops after 2000 words rather than on the PDL index
/// wrapping to 0, `CLEAR-PDL-BUFFER` starts from a pointer of all ones, and
/// `CLEAR-LEVEL-2-MAP` clears all 64 blocks. Every word before the first of
/// those is MIT's version 9 word for word, and it is four words longer.
#[test]
fn quux_s_boot_prom_is_mits_up_to_its_changes() {
    use muir::prom::{boot_prom, quux_boot_prom};
    let mits = muir::mcr::parse(
        &std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/mit/sys/ubin/promh.mcr")).unwrap(),
    )
    .unwrap();
    let ours = muir::mcr::parse(
        &std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/data/quux-promh.mcr")).unwrap(),
    )
    .unwrap();
    assert_eq!((mits.imem.len(), ours.imem.len()), (0o706, 0o712), "the words each defines");
    let first = mits.imem.iter().zip(&ours.imem).position(|(a, b)| a.raw() != b.raw());
    assert_eq!(first, Some(0o223), "MIT's word for word up to the first change");
    assert_eq!(quux_boot_prom()[..0o223], boot_prom()[..0o223]);
}
