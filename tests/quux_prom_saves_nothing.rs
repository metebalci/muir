// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! QUUX's boot PROM saves nothing and writes nothing to the disk (contract
//! Q8): from the boot until the PC first reaches the microcode's location 6
//! it writes no block of the disk, and main memory only in physical pages
//! 3-6 (words 1400-3377) --- its buffer and the microcode's main-memory
//! section, which it loads last over the buffer --- and word 777, the
//! command list word of its disk transfers. On a GPT disk blocks 0 to 16
//! are the protective MBR, the primary GPT header and its entry array, and
//! the machine never writes the table (contract Q8, decided 1). MIT's PROM
//! writes page 0 of memory to block 1 before it loads anything, "In order
//! to not clobber core" (`SAVE-A-PAGE`, `mit/sys/ucadr/promh.text`);
//! muir-sys's `promh.text` for QUUX drops that save.
//!
//! The PROM is `data/quux-promh.mcr`, muir's built-in QUUX PROM, the GPT
//! PROM: it finds the microcode through the disk's GPT, the first
//! microcode partition with attribute bit 48 set, and not through MIT's
//! `LABL` label, and on a disk with no GPT it stops at `ERROR-NO-GPT`
//! ([`quux_s_prom_reads_a_gpt_not_mit_s_label`]). The microcode partition
//! holds a `.mcr` in partition order, as `dd` writes it, at the partition's
//! first block and with no conversion.
//!
//! Two disks. One is made here from committed files and always runs:
//! `data/quux-disk.img`, the GPT disk sgdisk made, with MIT's microcode
//! 323, `mit/sys/ubin/ucadr.mcr`, turned into partition order, in its
//! current `MCR1`. The PROM does not care whose microcode it loads, only
//! about its sections. The other is muir-sys's System 1002 dev11 disk,
//! `ref/band-1002-dev11/pack-1002-dev11.vhd`, a dynamic VHD whose `MCR1`
//! holds the hand-over's `ucadr.mcr` as it is; it skips when that is not
//! present.

use std::path::Path;

use muir::band::{self, Label};
use muir::block_disk::{BLOCK_NS, BlockDisk, Transfer};
use muir::engine::Engine;
use muir::machine::{Geometry, Machine};
use muir::micro::Micro;
use muir::rtl::Rtl;

mod support;

/// A block of the disk, in bytes: 256 words of four.
const BLOCK_BYTES: usize = 1024;

/// Physical pages 3-6, the PROM's buffer and the microcode's main-memory
/// section.
const PAGES_3_TO_6: std::ops::RangeInclusive<u32> = 0o1400..=0o3377;

/// The word the PROM's disk transfers take their command list from.
const CCW: u32 = 0o777;

/// Where the PC stops the count: the microcode's location 6, where the PROM
/// jumps when it is done (`JUMP-TO-6` in the PROM's own symbols).
const LOCATION_6: u16 = 6;

/// More than the PROM takes to load either microcode, which is about 1.1
/// million microcycles (measured).
const LIMIT: u64 = 20_000_000;

/// What the boot did before the PC first reached 6.
struct Run {
    microcycles: u64,
    stores: Vec<u32>,
    transfers: Vec<Transfer>,
    /// Physical memory 1400-3377 at 6.
    pages: Vec<u32>,
}

/// QUUX with its built-in PROM and `pack` on block-disk, with the bus's
/// stores and the disk's transfers recorded.
fn quux(pack: &Path) -> Machine {
    let mut m = Machine::new();
    m.geometry = Geometry::QUUX;
    m.load_prom(&muir::prom::quux_boot_prom());
    let mut d = BlockDisk::new(BLOCK_NS);
    d.attach(muir::disk_image::Disk::open_rw(pack).expect("the pack"));
    d.log = Some(Vec::new());
    m.block_disk = Some(d);
    m.store_log = Some(Vec::new());
    m
}

/// Boots `e` and steps it until the microinstruction at 6 has run and the
/// machine has gone on in the microcode, and returns what was done before
/// the step that ran it. The PC alone does not say when the PROM is done:
/// on `rtl` both `Engine::pc` and `Machine::opc` pass 6 while the PROM
/// clears the control store (`CLEAR-I-MEMORY`, 36246-36252), a control
/// store write taking its address through the PC, and the next microcycle
/// is back in the PROM (measured). After the PROM's jump to 6 the next one
/// is below 36000.
fn to_six<E: Engine>(mut e: E, name: &str) -> Run {
    e.boot();
    let mut n = 0u64;
    loop {
        let m = e.machine();
        let stores = m.store_log.as_ref().unwrap().len();
        let transfers = m.block_disk.as_ref().unwrap().log.as_ref().unwrap().len();
        e.step().unwrap();
        if e.machine().opc == LOCATION_6 && e.pc() < 0o36000 {
            let m = e.machine_mut();
            let lo = *PAGES_3_TO_6.start() as usize;
            let hi = *PAGES_3_TO_6.end() as usize;
            let mut run = Run {
                microcycles: n,
                stores: m.store_log.take().unwrap(),
                transfers: m.block_disk.as_mut().unwrap().log.take().unwrap(),
                pages: m.main[lo..=hi].to_vec(),
            };
            run.stores.truncate(stores);
            run.transfers.truncate(transfers);
            return run;
        }
        n += 1;
        assert!(n < LIMIT, "{name}: the PC is at {:o} after {n} microcycles, not at 6", e.pc());
    }
}

/// `dd if=<mcr> of=<pack> bs=1024 seek=<MCR1's first block> conv=notrunc`
/// on a pack with MIT's label: the file's bytes as they are, at the
/// partition's first block.
fn dd_into_labl_mcr1(pack: &Path, mcr: &[u8]) {
    use std::io::{Seek, SeekFrom, Write};
    let label = Label::open(pack).expect("the pack's label");
    let p = label.partition("MCR1").expect("MCR1").clone();
    assert!(mcr.len() / BLOCK_BYTES <= p.blocks as usize, "the .mcr fits MCR1");
    let mut f = std::fs::OpenOptions::new().write(true).open(pack).unwrap();
    f.seek(SeekFrom::Start(p.start as u64 * BLOCK_BYTES as u64)).unwrap();
    f.write_all(mcr).unwrap();
}

/// The main-memory section's data as the microcode partition holds it: its
/// four blocks at the relative block the section header names, as words.
fn main_memory_data(mcr: &[u8]) -> Vec<u32> {
    let m = muir::mcr::parse_partition_order(mcr).unwrap();
    let (block, blocks) = m.main_memory.expect("a main-memory section");
    assert_eq!(blocks, 4, "four blocks, pages 3-6");
    let at = block as usize * BLOCK_BYTES;
    mcr[at..at + 4 * BLOCK_BYTES]
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .map(u32::from_le_bytes)
        .collect()
}

/// Addresses in octal, for a failure message.
fn octal(a: &[u32]) -> String {
    a.iter().map(|a| format!("{a:o}")).collect::<Vec<_>>().join(" ")
}

/// Boots `pack` on both engines, each on a fresh copy of it, and holds the
/// run to the PROM's promise. `ext` names the copy for what it is; muir
/// tells a VHD by its footer, not its name.
fn holds(pack: &Path, ext: &str, mcr: &[u8], dir: &Path) {
    let before = std::fs::read(pack).unwrap();
    let want_pages = main_memory_data(mcr);
    for name in ["micro", "rtl"] {
        let copy = dir.join(format!("{name}.{ext}"));
        std::fs::copy(pack, &copy).unwrap();
        let run = match name {
            "micro" => to_six(Micro::new(quux(&copy)), name),
            _ => to_six(Rtl::new(quux(&copy)), name),
        };
        let writes = run.transfers.iter().filter(|t| t.write).count();
        let reads: Vec<&Transfer> = run.transfers.iter().filter(|t| !t.write).collect();
        let stray_stores: Vec<u32> =
            run.stores.iter().copied().filter(|a| *a != CCW && !PAGES_3_TO_6.contains(a)).collect();
        // A block read is 256 words written into memory from its page on.
        let stray_reads: Vec<u32> = reads
            .iter()
            .filter(|t| !(PAGES_3_TO_6.contains(&t.page) && PAGES_3_TO_6.contains(&(t.page + 255))))
            .map(|t| t.page)
            .collect();
        eprintln!(
            "{name}: at 6 after {} microcycles; {} blocks read, {writes} written; \
             {} stores, {} outside pages 3-6 and word 777; {} block reads outside pages 3-6",
            run.microcycles,
            reads.len(),
            run.stores.len(),
            stray_stores.len(),
            stray_reads.len(),
        );
        assert!(!reads.is_empty() && !run.stores.is_empty(), "{name}: the record is kept");
        assert_eq!(
            writes,
            0,
            "{name}: blocks written: {:?}",
            run.transfers.iter().filter(|t| t.write).collect::<Vec<_>>()
        );
        assert!(stray_stores.is_empty(), "{name}: stored at {}", octal(&stray_stores));
        assert!(stray_reads.is_empty(), "{name}: blocks read into {}", octal(&stray_reads));
        assert!(std::fs::read(&copy).unwrap() == before, "{name}: the disk file unchanged");
        assert!(
            run.pages == want_pages,
            "{name}: pages 3-6 hold the microcode's main-memory section, loaded last"
        );
    }
}

/// **QUUX's PROM writes no block and only pages 3-6 and word 777 of
/// memory**, on a GPT disk made here with MIT's microcode 323 in partition
/// order in its current `MCR1`.
#[test]
fn quux_s_prom_saves_nothing_on_a_disk_made_here() {
    let dir = support::scratch("quux-prom-saves-nothing");
    let mcr = support::ucadr_323_partition_order();
    let (pack, _) = support::quux_gpt_disk(&dir, &mcr);
    holds(&pack, "img", &mcr, &dir);
}

/// **The same on System 1002 dev11's disk**, the dynamic VHD as muir-sys
/// handed it over, whose current `MCR1`, at block 17, holds the hand-over's
/// partition-order microcode 1000 as `dd` put it there.
#[test]
fn quux_s_prom_saves_nothing_on_system_1002_s_disk() {
    let band = Path::new(env!("CARGO_MANIFEST_DIR")).join("ref/band-1002-dev11");
    let (vhd, ucode) = (band.join("pack-1002-dev11.vhd"), band.join("ucadr.mcr"));
    for p in [&vhd, &ucode] {
        if !p.exists() {
            eprintln!("skipped: {} is not present", p.display());
            return;
        }
    }
    let dir = support::scratch("quux-prom-saves-nothing-1002");
    let pack = dir.join("pack.vhd");
    std::fs::copy(&vhd, &pack).unwrap();
    let mcr = std::fs::read(&ucode).unwrap();
    let mut d = muir::disk_image::Disk::open(&pack).unwrap();
    let mcr1 = support::gpt_partition(&mut d, "MCR1");
    assert_eq!((mcr1.first, mcr1.current), (17, true), "MCR1, current, at block 17");
    for (k, block) in mcr.chunks(BLOCK_BYTES).enumerate() {
        let on_disk: Vec<u8> = d
            .read_block(mcr1.first + k as u32)
            .unwrap()
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        assert!(on_disk == block, "MCR1's block {k} is the hand-over's ucadr.mcr");
    }
    holds(&pack, "vhd", &mcr, &dir);
}

/// `ERROR-NO-GPT`, where the GPT PROM halts when block 0's second sector
/// is not a GPT header (the hand-over's `promh.tbl`, held by
/// `tests/quux_prom.rs`).
const ERROR_NO_GPT: u16 = 0o36632;

/// **QUUX's PROM finds the microcode through a GPT, not MIT's label**: on a
/// pack with MIT's `LABL` label in block 0 and no GPT --- a T-300 label of
/// MIT's own layout with MIT's microcode 323 in partition order in `MCR1`,
/// what the PROM before the GPT booted --- it reads block 0 into its
/// buffer at page 3 and halts at `ERROR-NO-GPT`, having written nothing.
#[test]
fn quux_s_prom_reads_a_gpt_not_mit_s_label() {
    let dir = support::scratch("quux-prom-labl");
    let pack = dir.join("pack.img");
    Label::initialize(&pack, &band::T300).write().unwrap();
    dd_into_labl_mcr1(&pack, &support::ucadr_323_partition_order());
    let before = std::fs::read(&pack).unwrap();
    for name in ["micro", "rtl"] {
        let copy = dir.join(format!("{name}.img"));
        std::fs::copy(&pack, &copy).unwrap();
        fn halt<E: Engine>(mut e: E, name: &str) -> (E, u64) {
            // Halted: the PC there, and staying there.
            e.boot();
            let mut there = 0;
            for n in 0..LIMIT {
                e.step().unwrap();
                there = if e.pc() == ERROR_NO_GPT { there + 1 } else { 0 };
                if there == 1000 {
                    return (e, n - 999);
                }
            }
            panic!("{name}: the PC is at {:o}, not at ERROR-NO-GPT", e.pc());
        }
        let (mut m, n) = match name {
            "micro" => {
                let (e, n) = halt(Micro::new(quux(&copy)), name);
                (e.machine().clone(), n)
            }
            _ => {
                let (e, n) = halt(Rtl::new(quux(&copy)), name);
                (e.machine().clone(), n)
            }
        };
        eprintln!("{name}: at ERROR-NO-GPT after {n} microcycles");
        let log = m.block_disk.as_mut().unwrap().log.take().unwrap();
        assert_eq!(log, [Transfer { write: false, block: 0, page: 0o1400 }], "{name}");
        assert!(m.store_log.unwrap().iter().all(|&a| a == CCW), "{name}: only the command word");
        assert!(std::fs::read(&copy).unwrap() == before, "{name}: the pack unchanged");
    }
}
