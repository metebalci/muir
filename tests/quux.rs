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

/// **QUUX says what it is in functional source 16.** The word is the
/// signature `0x5155` in bits 31:16, the hardware revision in 15:4 and the
/// processor type, 4, in 3:0. On the CADR no part drives the M bus for
/// source 16 and it reads all ones, as `chip` shows
/// (`tests/output_bus.rs`), which can never carry the signature. Source 36
/// is 16, `IR<30>` being in no source decode, and source 17 is left open on
/// both machines.
#[test]
fn quux_answers_its_id_in_source_16() {
    let prom = [
        Insn::new(ALU | SETM | src(0o16) | a_dest(0o201)),
        Insn::new(ALU | SETM | src(0o36) | a_dest(0o202)),
        Insn::new(ALU | SETM | src(0o17) | a_dest(0o203)),
    ];
    let id = (0x5155 << 16) | (1 << 4) | 4;
    for (geometry, want) in [(Geometry::QUUX, [id, id, !0]), (Geometry::CADR, [!0, !0, !0])] {
        let (e, r) = both(&prom, &|m: &mut Machine| m.geometry = geometry, 30);
        for (name, m) in [("micro", e.machine()), ("rtl", r.machine())] {
            let got = [m.amem[0o201], m.amem[0o202], m.amem[0o203]];
            assert_eq!(got, want, "{geometry:?}, {name}");
        }
    }
    assert_eq!(Geometry::QUUX.id, Some(id));
    assert_eq!(Geometry::CADR.id, None);
}
