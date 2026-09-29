// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! QUUX's boot PROM in its own addresses (contract Q2): 1K words at
//! control store 36000-37777, read only and never overlaid. Reset starts
//! the PC there; the microcode lives in 0-35777, which is RAM from the
//! start, and there is no PROM-disable bit. The CADR keeps MIT's overlay.

use muir::engine::Engine;
use muir::isa::Insn;
use muir::isa::asm::{ALU, ALWAYS, JUMP, N, SETO, filler, m_dest, target};
use muir::machine::{Geometry, Machine};
use muir::micro::Micro;
use muir::rtl::Rtl;

/// Where QUUX's PROM starts.
const BASE: u16 = 0o36000;

/// A QUUX with `prom` at 36000, the RAM word `ram` at 6 and at 36005.
fn quux(prom: &[Insn], ram: Insn) -> Machine {
    let mut m = Machine::new();
    m.geometry = Geometry::QUUX;
    let mut words = vec![filler(); 1024];
    words[..prom.len()].copy_from_slice(prom);
    m.load_prom(&words);
    m.imem[6] = ram;
    m.imem[BASE as usize + 5] = ram;
    m
}

/// The PROM program: M 1 set, then a jump to 36005 (the PROM's own word
/// there sets M 2), then a jump to 6 in the RAM (whose word sets M 3).
fn program() -> Vec<Insn> {
    let mut p = vec![filler(); 8];
    p[0] = Insn::new(ALU | SETO | m_dest(1));
    p[1] = Insn::new(JUMP | target(BASE as u64 + 5) | ALWAYS | N);
    p[5] = Insn::new(ALU | SETO | m_dest(2));
    p[6] = Insn::new(JUMP | target(6) | ALWAYS | N);
    p
}

/// Both engines booted and run `steps` microcycles; M 1 to M 3 and the PCs
/// each executed.
fn run(m: Machine, steps: usize) -> [([u32; 3], Vec<u16>); 2] {
    let mut e = Micro::new(m.clone());
    e.boot();
    let mut pe = Vec::new();
    for _ in 0..steps {
        pe.push(e.pc());
        e.step().unwrap();
    }
    let mut r = Rtl::new(m);
    r.boot();
    let mut pr = Vec::new();
    for _ in 0..steps {
        pr.push(r.pc());
        r.step().unwrap();
    }
    // `<31:0>`, of a 32-bit machine's words: nothing is above.
    let mm = |x: &Machine| [x.mmem[1], x.mmem[2], x.mmem[3]].map(|w| u32::try_from(w).unwrap());
    [(mm(e.machine()), pe), (mm(r.machine()), pr)]
}

/// **QUUX starts at 36000, runs its PROM there, and reaches the RAM below
/// with no PROM-disable**: the first microcycle after the boot is 36000's;
/// at 36005 it runs the PROM's word, not the RAM's word at the same
/// address; and a jump to 6 runs the RAM's word at 6, though nothing
/// turned the PROM off.
#[test]
fn quux_boots_at_36000_and_the_ram_below_is_live() {
    let ram = Insn::new(ALU | SETO | m_dest(3));
    for (k, (m, pcs)) in run(quux(&program(), ram), 40).into_iter().enumerate() {
        let name = ["micro", "rtl"][k];
        let first = pcs.iter().position(|&p| p != 0).map(|i| pcs[i]);
        assert_eq!(first, Some(BASE), "{name}: the PC after the boot, {pcs:?}");
        assert_eq!(m, [!0, !0, !0], "{name}: the PROM's 0 and 5, then the RAM's 6");
    }
}

/// **The PROM is not written by `WRITE-I-MEM`**, nor by anything else: a
/// word stored at 36005 through the control store's write path stays the
/// RAM's, which QUUX never fetches there.
#[test]
fn quux_s_prom_is_read_only() {
    let mut m = quux(&program(), Insn::new(ALU | SETO | m_dest(3)));
    m.write_imem(BASE + 5, Insn::new(0));
    assert_eq!(m.fetch(BASE + 5), program()[5], "the PROM's word, still");
    assert_eq!(m.fetch(6), Insn::new(ALU | SETO | m_dest(3)), "the RAM below");
}

/// **The PROM-disable bit does nothing on QUUX**: set, the PROM still
/// answers at 36000 and the RAM at 0.
#[test]
fn quux_has_no_prom_disable() {
    let mut m = quux(&program(), Insn::new(0));
    m.imem[0] = Insn::new(ALU | SETO | m_dest(7));
    for disable in [false, true] {
        m.mode.prom_disable = disable;
        assert_eq!(m.fetch(BASE), program()[0], "36000, disable {disable}");
        assert_eq!(m.fetch(0), m.imem[0], "0, disable {disable}");
    }
}

/// **The CADR keeps MIT's overlay**: its PROM at 0 until `PROMDISABLE`,
/// the RAM there after, and 36000 always RAM.
#[test]
fn the_cadr_keeps_the_overlay() {
    let mut m = Machine::new();
    m.load_prom(&program());
    m.imem[0] = Insn::new(ALU | SETO | m_dest(7));
    m.imem[BASE as usize] = Insn::new(ALU | SETO | m_dest(6));
    assert_eq!(m.fetch(0), program()[0]);
    assert_eq!(m.fetch(BASE), m.imem[BASE as usize]);
    m.mode.prom_disable = true;
    assert_eq!(m.fetch(0), m.imem[0]);
}

/// Where 36000's `JUMP GO` goes: `GO`, at 36043 since muir-sys's commit
/// `7c4bcb2` put the halts `ERROR-TWO-MAIN-MEM-SECTIONS` at 36040 and
/// `ERROR-BUFFER-NOT-LOADED` at 36042 before it, and still there in the GPT
/// PROM of revision 11: the hand-over's symbol table `promh.sym` says `GO
/// I-MEM 36043`
/// ([`the_built_in_quux_prom_is_the_hand_over`]).
const GO: u64 = 0o36043;

/// The PROM's last word: the revision 11 PROM's `promh.locs` says `(I-MEM
/// 36647)`, the section's size, so the code is at 36000-36646, and 36646 is
/// its last halt, `ERROR-ODD-MICR-START`, the disk routines and the halts
/// after them following the reset devices and timer 0's period that the
/// PROM writes since revision 10 (contract Q11). Revision 11's map change
/// (contract Q13) put no-ops where the two writes that went were, so no
/// address moved: its `promh.sym`, `promh.tbl` and `promh.locs` are
/// revision 10's byte for byte.
const LAST: usize = 0o36646;

/// `DISK-AWAIT-PACK`, the first of the disk routines, which the new writes
/// come before: 36600 in the revision 11 PROM's `promh.sym`.
const DISK_AWAIT_PACK: u64 = 0o36600;

/// The GPT PROM's own halts, after the disk routines, as its `promh.tbl`
/// and `promh.sym` put them: no GPT header (or its entry array's LBA past
/// 32 bits), no current microcode partition, and one whose first LBA is
/// odd.
const GPT_HALTS: [(u64, &str); 3] = [
    (0o36642, "ERROR-NO-GPT"),
    (0o36644, "ERROR-NO-CURRENT-MICR"),
    (0o36646, "ERROR-ODD-MICR-START"),
];

/// `A-DISK-REGS`, where the PROM keeps block-disk's virtual address: A
/// memory 62 in the hand-over's `promh.sym`, unchanged by revision 11.
const A_DISK_REGS: usize = 0o62;

/// **The built-in PROM maps the register page where revision 11 has it**
/// (contract Q13 §5.2): run to `DISK-AWAIT-PACK`, its first disk routine,
/// it has mapped virtual page 2 to physical page 37777, the register page
/// at `17777400`, where revision 10's PROM mapped 36776; it keeps
/// block-disk's registers at virtual 1200, words 200-203 of that page,
/// where it kept 774; and virtual page 1, which named the old disk
/// registers' page 36777, is mapped no more. A stale PROM fails this on
/// every machine, with or without the hand-over.
#[test]
fn quux_s_prom_maps_the_register_page_at_37777() {
    fn check<E: Engine>(mut e: E, name: &str) {
        e.boot();
        let mut n = 0u32;
        while e.pc() != DISK_AWAIT_PACK as u16 {
            e.step().unwrap();
            n += 1;
            assert!(n < 2_000_000, "{name}: not at DISK-AWAIT-PACK, at {:o}", e.pc());
        }
        let m = e.machine();
        let entry = |virt: u32| m.l2_map[m.geometry.l2_index(m.l1_map[0], virt)];
        assert_eq!(entry(0o1000) & 0o37777, 0o37777, "{name}: virtual page 2's physical page");
        assert_ne!(entry(0o400) & 0o37777, 0o36777, "{name}: virtual page 1, the old disk page");
        assert_eq!(m.amem[A_DISK_REGS], 0o1200, "{name}: A-DISK-REGS");
    }
    let mut m = Machine::new();
    m.geometry = Geometry::QUUX;
    m.load_prom(&muir::prom::quux_boot_prom());
    check(Micro::new(m.clone()), "micro");
    check(Rtl::new(m), "rtl");
}

/// **A QUUX PROM file is read from 36000, in partition order** (contracts
/// Q2 and Q8): the assembler writes the control store section from 0, so
/// the reader takes 36000-37777 of it and refuses a file with anything
/// assembled below 36000, or past 37777; and the file is QUUX's `.mcr`,
/// every word's two halves swapped from MIT's order, so a file in MIT's
/// order is refused saying so.
#[test]
fn a_quux_prom_file_is_read_from_36000() {
    use muir::mcr::swap_halves;
    use muir::prom::parse_quux_mcr;
    let file = include_bytes!("../data/quux-promh.mcr");
    let words = parse_quux_mcr(file).unwrap();
    assert_eq!(words.len(), 1024);
    // 36000 is `JUMP GO`.
    assert_eq!(words[0].raw() >> 43 & 3, 1, "a jump");
    assert_eq!(words[0].jump().target as u64, GO);
    let base = 0o36000;
    assert_ne!(words[LAST - base].raw(), 0, "the last word");
    assert!(words[LAST - base + 1..].iter().all(|w| w.raw() == 0), "nothing past it");
    // The same file in MIT's order, and MIT's own PROM, which is in MIT's
    // order and assembled at 0.
    let mit_order = swap_halves(file).unwrap();
    let err = parse_quux_mcr(&mit_order).unwrap_err();
    assert!(err.contains("MIT's order"), "{err}");
    let mits = include_bytes!("../mit/sys/ubin/promh.mcr");
    let err = parse_quux_mcr(mits).unwrap_err();
    assert!(err.contains("MIT's order"), "{err}");
    let err = parse_quux_mcr(&swap_halves(mits).unwrap()).unwrap_err();
    assert!(err.contains("QUUX's PROM is assembled at 36000"), "MIT's, at 0: {err}");
}

/// **QUUX's PROM file is MIT's `promh.mcr` as muir-sys changed it**, held
/// to MIT's own file through the half swap: the same four sections in the
/// same order, the dispatch memory and A memory sections word for word
/// MIT's, and the main-memory section MIT's size, four blocks. Only the
/// control store section, the program, is QUUX's.
#[test]
fn quux_s_prom_is_mit_s_promh_changed() {
    use muir::mcr::{parse, parse_partition_order};
    let quux = parse_partition_order(include_bytes!("../data/quux-promh.mcr")).unwrap();
    let mits = parse(include_bytes!("../mit/sys/ubin/promh.mcr")).unwrap();
    assert_eq!((quux.dmem_start, &quux.dmem), (mits.dmem_start, &mits.dmem), "dispatch memory");
    assert_eq!((quux.amem_start, &quux.amem), (mits.amem_start, &mits.amem), "A memory");
    assert_eq!(quux.main_memory.map(|m| m.1), Some(4), "four blocks");
    assert_eq!(mits.main_memory.map(|m| m.1), Some(4), "four blocks");
    assert_eq!(quux.imem_start, mits.imem_start, "the control store section from 0");
    assert_ne!(quux.imem, mits.imem, "the program is QUUX's");
}

/// **The built-in QUUX PROM is muir-sys's hand-over, byte for byte**, where
/// the hand-over (`ref/band-2000`, whose `promh.*` are PROM 2000, the GPT
/// PROM for revision 11, from muir-sys `02c0bb3`'s `promh.text`) is present; its
/// symbols and error table say version 2000, 3720 octal; and they put what
/// [`GO`], [`LAST`], [`DISK_AWAIT_PACK`] and [`GPT_HALTS`] say where they
/// say.
#[test]
fn the_built_in_quux_prom_is_the_hand_over() {
    let handed = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ref/band-2000");
    let Ok(bytes) = std::fs::read(handed.join("promh.mcr")) else {
        eprintln!("skipped: {} is not present", handed.display());
        return;
    };
    assert!(bytes == include_bytes!("../data/quux-promh.mcr"), "the hand-over");
    let locs = std::fs::read_to_string(handed.join("promh.locs")).unwrap();
    assert!(locs.contains(&format!("(I-MEM {:o})", LAST + 1)), "{locs}");
    let tbl = std::fs::read_to_string(handed.join("promh.tbl")).unwrap();
    let sym = std::fs::read_to_string(handed.join("promh.sym")).unwrap();
    assert!(
        tbl.contains(&format!("MICROCODE-ERROR-TABLE-VERSION-NUMBER {:o})", 2000)),
        "version 2000 in promh.tbl"
    );
    assert!(sym.contains(&format!(" VERSION-NUMBER {:o} ", 2000)), "version 2000 in promh.sym");
    assert!(sym.contains(&format!("GO I-MEM {GO:o} ")), "GO in promh.sym");
    assert!(
        sym.contains(&format!("DISK-AWAIT-PACK I-MEM {DISK_AWAIT_PACK:o} ")),
        "DISK-AWAIT-PACK in promh.sym"
    );
    let mut halts = vec![
        (0o36016, "ERROR-BAD-LABEL"),
        (GO - 3, "ERROR-TWO-MAIN-MEM-SECTIONS"),
        (GO - 1, "ERROR-BUFFER-NOT-LOADED"),
    ];
    halts.extend(GPT_HALTS);
    for (at, name) in halts {
        assert!(tbl.contains(&format!("({at:o} {name})")), "{name} at {at:o} in promh.tbl");
    }
    for (at, name) in GPT_HALTS {
        assert!(sym.contains(&format!("{name} I-MEM {at:o} ")), "{name} at {at:o} in promh.sym");
    }
}
