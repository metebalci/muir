// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The disk controller, against MIT's own documentation of it.
//!
//! Every number here is quoted from one of the two sources
//! `src/disk_controller.rs` names: `sys/doc/disk.text` in the System 100
//! release, and the controller's own drawings in `mit/cadrdc`.  Tests that
//! need the disk image skip cleanly without it.

use std::path::PathBuf;

use muir::disk_controller::{Controller, REGS, TIMEOUT_NS};
use muir::disk_unit::{BLOCK_WORDS, Geometry, Unit};
use muir::machine::Machine;

mod support;

/// Status bits, by MIT's numbering.  `DCSTS` drives each of these onto the
/// Xbus line of the same number.
mod status {
    pub const READ_COMPARE_DIFFERENCE: u32 = 1 << 22;
    pub const TIMEOUT: u32 = 1 << 11;
    pub const OVERRUN: u32 = 1 << 14;
    pub const NXM: u32 = 1 << 20;
    pub const ABORTED: u32 = 1 << 13;
    pub const SEEK_ERROR: u32 = 1 << 10;
    pub const NOT_ON_LINE: u32 = 1 << 9;
    pub const NOT_ON_CYLINDER: u32 = 1 << 8;
    pub const FAULT: u32 = 1 << 6;
    pub const NO_SELECT: u32 = 1 << 5;
    pub const INTERRUPT_REQUEST: u32 = 1 << 3;
    pub const ATTENTION: u32 = 1 << 2;
    pub const ANY_ATTENTION: u32 = 1 << 1;
    pub const NOT_ACTIVE: u32 = 1;
    /// What `AWAIT-DRIVE-READY` in the boot PROM demands be clear: "Bits
    /// 4,5,6,8,9,10", which it builds as `A-3560`.
    pub const DRIVE_NOT_READY: u32 = 0o3560;
    /// What a controller with nothing on its cable reads, before any
    /// command and after a transfer's START: not active, not on line, not
    /// on cylinder, no unit selected, and transfer aborted --- the disk
    /// lossage those three make presets `STOPPED BY ERROR` while the
    /// command register holds a command with `CMD2` low, which a fresh
    /// register does.  `0o21441` is the word off the netlist board in
    /// `tests/cadrdc_netlist.rs`.
    pub const NO_DRIVE: u32 = NOT_ACTIVE | NO_SELECT | NOT_ON_CYLINDER | NOT_ON_LINE | ABORTED;
}

/// The four registers, by their offsets from `REGS`.
mod reg {
    pub const STATUS: u32 = 0;
    pub const COMMAND: u32 = 0;
    pub const MEMORY_ADDRESS: u32 = 1;
    pub const CLP: u32 = 1;
    pub const DISK_ADDRESS: u32 = 2;
    pub const START: u32 = 3;
}

fn image() -> Option<PathBuf> {
    support::pack_100()
}

/// A controller with unit 0 loaded from the System 100 pack, and 256 pages of
/// physical memory for it to transfer into --- enough for these tests, and
/// small enough that running off the end of it is easy to arrange.
fn loaded() -> Option<(Controller, Vec<u32>)> {
    let mut d = Controller::default();
    d.attach(0, Unit::open(image()?, Geometry::T300).unwrap());
    Some((d, vec![0; 1 << 16]))
}

/// Where these tests keep the command list.  The boot PROM puts it at
/// physical `0o777`, the last word of the system communication area; anywhere
/// out of the pages being transferred will do, and page 8 keeps the low pages
/// free for data.
const CLP: u32 = 0o4000;

/// Reads one block into memory at `page`, the way the boot PROM does it: a
/// one-CCW command list, then a store into START.
fn read_block(d: &mut Controller, main: &mut [u32], block: u32, page: u32) {
    // "<23:8> Main memory address of a page.  <7:1> not used.  <0> More
    // flag" --- clear, so this is the last CCW in the list.
    main[CLP as usize] = page << 8;
    d.write(reg::COMMAND, 0, main); // 0000 Read
    d.write(reg::CLP, CLP, main);
    d.write(reg::DISK_ADDRESS, block, main);
    d.write(reg::START, 0, main);
}

/// With no drive plugged in, the controller still answers: it is a board on
/// the bus.  This is the state the boot PROM hangs in --- its own error table
/// calls it `AWAIT-DISK-ON-LINE`, "Hangs near here until drive is on-line".
#[test]
fn a_controller_with_no_drive_is_ready_and_off_line() {
    let d = Controller::default();
    assert_eq!(status::NO_DRIVE, 0o21441, "the word the netlist board reads");
    assert_eq!(d.read(reg::STATUS), status::NO_DRIVE);
}

/// **With no drive, each command does what the board does.** Measured on
/// the netlist controller after a reset before each,
/// `with_no_drive_only_the_miscellaneous_command_completes` in
/// `tests/cadrdc_netlist.rs`: a read stops before it starts, the disk
/// lossage presetting `BUSY` off, and the word stays `NO_DRIVE`; at ease,
/// recalibrate and fault clear run to done with no error, `CMD2` masking
/// the lossage, so `<13>` is cleared by the START and stays clear; a seek
/// or an offset clear runs and waits at its first step for a drive that
/// never answers, `<13>` clear and the controller active, which on MIT's
/// board with the timeout jumper in ends `TIMEOUT_NS` on with the timeout
/// error and, through the transfer lossage, `<13>` again; and a reset
/// puts the word back, the empty command register letting the lossage
/// through.
#[test]
fn with_no_drive_each_command_does_what_the_board_does() {
    let mut d = Controller::default();
    let mut main = vec![0; 1 << 16];
    d.advance(1_000);
    read_block(&mut d, &mut main, 0, 2);
    assert_eq!(d.status(), status::NO_DRIVE, "a read stops before it starts");
    for cmd in [0o5, 0o1005, 0o405] {
        d.write(reg::COMMAND, cmd, &mut main);
        d.write(reg::START, 0, &mut main);
        assert_eq!(d.status(), status::NO_DRIVE & !status::ABORTED, "{cmd:o}: done, no error");
    }
    let mut t = 1_000;
    for cmd in [0o4, 0o6] {
        d.write(reg::COMMAND, cmd, &mut main);
        d.write(reg::START, 0, &mut main);
        let waiting = status::NO_DRIVE & !(status::ABORTED | status::NOT_ACTIVE);
        assert_eq!(d.status(), waiting, "{cmd:o}: waiting on the drive");
        d.advance(t + TIMEOUT_NS - 1);
        assert_eq!(d.status(), waiting, "{cmd:o}: still waiting");
        d.advance(t + TIMEOUT_NS);
        assert_eq!(d.status(), status::NO_DRIVE | status::TIMEOUT, "{cmd:o}: timed out");
        d.write(reg::COMMAND, 0o16, &mut main);
        d.write(reg::COMMAND, 0, &mut main);
        assert_eq!(d.status(), status::NO_DRIVE, "{cmd:o}: after a reset");
        t += TIMEOUT_NS + 1_000;
        d.advance(t);
    }
}

/// **Transfer Aborted follows any lossage.** `STOPPED BY ERROR` is preset
/// while a lossage stands and cleared by a START or a store into the
/// command register.  The timeout is a transfer lossage, so a reserved
/// code's timeout brings `<13>` with `<11>`, and the next command stored
/// takes both away.  A fault is a disk lossage: writing a read-only pack
/// faults the drive and `<13>` comes with `<6>`; the fault clear that
/// takes `<6>` away is a command with `CMD2` up, which masks the disk
/// lossage from the moment it is stored, so `<13>` goes with the store
/// and `<6>` with the START.
#[test]
fn transfer_aborted_follows_any_lossage() {
    let (mut d, mut main) = blank();
    d.advance(1_000);
    d.write(reg::COMMAND, 0o7, &mut main);
    d.write(reg::START, 0, &mut main);
    assert_eq!(d.status() & (status::ABORTED | status::TIMEOUT | status::NOT_ACTIVE), 0, "hung");
    d.advance(1_000 + TIMEOUT_NS);
    let s = d.status();
    assert_eq!(s & (status::ABORTED | status::TIMEOUT | status::NOT_ACTIVE), 0o24001, "{s:o}");
    d.write(reg::COMMAND, 0o5, &mut main);
    assert_eq!(d.status() & (status::ABORTED | status::TIMEOUT), 0, "the next command clears both");
    d.write(reg::START, 0, &mut main);
    assert_eq!(
        d.status() & (status::ABORTED | status::TIMEOUT),
        0,
        "and the START keeps them clear"
    );

    d.units[0].as_mut().unwrap().read_only = true;
    main[CLP as usize] = 1 << 8;
    d.write(reg::COMMAND, 0o11, &mut main);
    d.write(reg::CLP, CLP, &mut main);
    d.write(reg::DISK_ADDRESS, 5, &mut main);
    d.write(reg::START, 0, &mut main);
    let s = d.status();
    assert_eq!(s & (status::FAULT | status::ABORTED), status::FAULT | status::ABORTED, "{s:o}");
    d.write(reg::COMMAND, 0o405, &mut main);
    let s = d.status();
    assert_eq!(
        s & (status::FAULT | status::ABORTED),
        status::FAULT,
        "CMD2 masks the lossage: {s:o}"
    );
    d.write(reg::START, 0, &mut main);
    assert_eq!(
        d.status() & (status::FAULT | status::ABORTED),
        0,
        "the fault clear clears the fault"
    );
}

/// The four registers are where MIT says they are, and reach the controller
/// through the machine's bus.  "These are normally at physical addresses
/// 17377774-17377777, which is just below the Unibus."
#[test]
fn the_registers_are_just_below_the_unibus() {
    let mut m = Machine::new();
    assert_eq!(REGS, 0o17377774);
    // Status, with no drive.
    assert_eq!(m.bus_read(0o17377774), status::NO_DRIVE);
    // The disk address register is the one register that reads back what was
    // written to it.
    m.bus_write(0o17377776, 0o12345670);
    assert_eq!(m.bus_read(0o17377776), 0o12345670);
    // One word below is nobody's: it times the Xbus out.
    assert_eq!(m.bus_read(0o17377773), 0);
    assert_eq!(m.bus_error, muir::machine::bus_error::XBUS_NXM);
}

/// The pack says how it was formatted, and it says what the T-300 is.
///
/// The T-300 is generally given as 815 cylinders, 19 heads, 17
/// blocks per track; MIT's documentation gives the same; and words 2 to 5 of
/// the label --- block 0 of this pack, written by whoever formatted it ---
/// agree.  Three routes, one geometry.
#[test]
fn the_label_states_the_t300s_geometry() {
    let Some((mut d, mut main)) = loaded() else { return };
    read_block(&mut d, &mut main, 0, 1);
    let label = &main[256..256 + BLOCK_WORDS];

    // "First location of label must be ascii LABL": the PROM builds
    // 0o11420440514 to compare against, and says so in its own comment.
    assert_eq!(label[0], 0o11420440514, "label word 0 is not LABL");
    assert_eq!(label[1], 1, "label version");
    assert_eq!(label[2], Geometry::T300.cylinders, "A-NCYLS");
    assert_eq!(label[3], Geometry::T300.heads, "A-NHEADS");
    assert_eq!(label[4], Geometry::T300.blocks_per_track, "A-NBLKS");
    assert_eq!(label[5], Geometry::T300.blocks_per_cylinder(), "A-HEADS-TIMES-BLOCKS");
    // Word 6 is the name of the microload partition the PROM then searches
    // for: MCR1, in the same four-bytes-to-a-word encoding as LABL.
    assert_eq!(label[6], 0o6124441515, "microload partition name");
}

/// **The T-80 is the T-300 with five heads.** `disk.text`: "A T-80 has
/// 815. cylinders, each with 5 heads (tracks), each with 16. or 17. blocks
/// depending on how you feel like formatting it.  A T-300 has 19. heads."
/// `Geometry::T80` takes 17, the count the T-300 pack's label gives for its
/// own formatting; a 16-block T-80 is another `Geometry`. The sentence is
/// read out of the release when it is present, and so is the pack, which
/// is a T-300 and does not open as a T-80: its size is not the one that
/// geometry implies.
#[test]
fn the_t80_is_the_t300_with_five_heads() {
    let g = Geometry::T80;
    assert_eq!((g.cylinders, g.heads, g.blocks_per_track), (815, 5, 17));
    assert_eq!(g.cylinders, Geometry::T300.cylinders);
    assert_eq!(g.blocks_per_track, Geometry::T300.blocks_per_track);
    assert_eq!(g.blocks_per_cylinder(), 5 * 17);
    assert_eq!(g.blocks(), 815 * 5 * 17, "69,275 blocks");
    assert_eq!(g.blocks() as usize * BLOCK_WORDS * 4, 70_937_600, "bytes of pack");
    // The next-block code turns to the next cylinder at the fifth head,
    // where the T-300 has fourteen more to go.
    assert_eq!(format::next_block_code(&g, 0, 3, 16), 1, "block 0 on the next head");
    assert_eq!(format::next_block_code(&g, 0, 4, 16), 2, "head 4 is the last: the next cylinder");
    assert_eq!(format::next_block_code(&g, 814, 4, 16), 3, "end of disk");
    assert_eq!(format::next_block_code(&Geometry::T300, 0, 4, 16), 1);
    if let Some(doc) = support::release("doc/disk.text") {
        let text = doc.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            text.contains(
                "A T-80 has 815. cylinders, each with 5 heads (tracks), each with 16. or 17. blocks"
            ),
            "MIT's sentence"
        );
        assert!(text.contains("A T-300 has 19. heads"), "and the T-300's");
    }
    if let Some(pack) = image() {
        assert!(Unit::open(&pack, g).is_err(), "the System 100 pack is a T-300, not a T-80");
    }
}

/// The done interrupt is a level off the command register's enable.
///
/// "Done Interrupt Enable.  Enables not-active (bit 0 of the status register)
/// to cause an interrupt.  The interrupt will keep happening until you clear
/// this bit."  `START-DISK-OP-1` sets it on every transfer, and
/// `DISK-COMPLETION-OK` clears it by storing zero in the register --- "Clear
/// interrupt enable", MIT's own comment.  Taking a zero as a
/// reset-condition exit and leaving the enables alone is the easy mistake:
/// the interrupt line then stays up for ever.
#[test]
fn the_done_interrupt_is_a_level_off_the_enable() {
    let mut d = Controller::default();
    let mut main = vec![0u32; 512];
    assert!(!d.interrupt());
    d.write(reg::COMMAND, 1 << 11, &mut main);
    assert!(d.interrupt(), "the enable is set and the controller is idle");
    assert_ne!(d.status() & status::INTERRUPT_REQUEST, 0);
    d.write(reg::COMMAND, 0, &mut main);
    assert!(!d.interrupt(), "a zero clears the enable, and the request with it");
    assert_eq!(d.status() & status::INTERRUPT_REQUEST, 0);
}

/// The sequence `DISK-RECALIBRATE` runs, register by register, and the state
/// it demands at each step.  This is the boot PROM's own path from a cold
/// controller to a drive it is willing to transfer with.
#[test]
fn the_prom_recalibrate_sequence_brings_the_drive_ready() {
    let Some((mut d, mut main)) = loaded() else { return };

    // "Wait for control ready": JUMP-IF-BIT-CLEAR (BYTE-FIELD 1 0).
    assert_ne!(d.status() & status::NOT_ACTIVE, 0, "control not ready");

    // "Select unit 0" by storing zero in the disk address register.
    d.write(reg::DISK_ADDRESS, 0, &mut main);
    // JUMP-IF-BIT-SET (BYTE-FIELD 1 9.) ... ;Off-line
    assert_eq!(d.status() & status::NOT_ON_LINE, 0, "unit 0 should be on line");

    // 405 Fault Clear, then 1005 Recalibrate --- the PROM builds the second
    // as A-DISK-RECAL, 0o10001005, whose bit 21 is not a command bit at all.
    for cmd in [0o405u32, 0o10001005] {
        d.write(reg::COMMAND, cmd, &mut main);
        d.write(reg::START, 0, &mut main);
    }
    // Recalibrate "causes an attention when complete", on the selected unit
    // and so on any unit.
    assert_ne!(d.status() & status::ATTENTION, 0);
    assert_ne!(d.status() & status::ANY_ATTENTION, 0);

    // AWAIT-DRIVE-READY: (M-TEMP-2) AND READ-MEMORY-DATA A-3560, and it
    // loops until that is zero.
    assert_eq!(d.status() & status::DRIVE_NOT_READY, 0, "drive not ready");

    // "0005 At ease.  Resets attention on the selected unit."
    d.write(reg::COMMAND, 0o5, &mut main);
    d.write(reg::START, 0, &mut main);
    assert_eq!(d.status() & (status::ATTENTION | status::ANY_ATTENTION), 0);
}

/// A command list with the More flag set carries on to the next block, and
/// the disk address register ends up holding "the address of the last block
/// transferred".
#[test]
fn a_command_list_transfers_block_after_block() {
    let Some((mut d, mut main)) = loaded() else { return };
    main[CLP as usize] = (1 << 8) | 1; // page 1, More
    main[CLP as usize + 1] = 2 << 8; // page 2, last
    d.write(reg::COMMAND, 0, &mut main);
    d.write(reg::CLP, CLP, &mut main);
    d.write(reg::DISK_ADDRESS, 0, &mut main);
    d.write(reg::START, 0, &mut main);

    assert_eq!(main[256], 0o11420440514, "first page is the label");
    assert_eq!(d.read(reg::DISK_ADDRESS), 1, "left on block 1");
    // "MA should point to the last (full) memory address accessed."
    assert_eq!(d.read(reg::MEMORY_ADDRESS), 2 * 256 + 255);

    // The same two blocks read again land in the same place, so the second
    // CCW really did read block 1 and not block 0 twice.
    let mut again = vec![0u32; main.len()];
    let mut d2 = loaded().unwrap().0;
    read_block(&mut d2, &mut again, 1, 3);
    assert_eq!(&main[512..512 + BLOCK_WORDS], &again[768..768 + BLOCK_WORDS]);
}

/// Read-compare "reads from both disk and memory, and sets bit 22 of the
/// status register if they don't agree", and "this error does not stop the
/// transfer".
#[test]
fn read_compare_reports_a_difference_and_carries_on() {
    let Some((mut d, mut main)) = loaded() else { return };
    read_block(&mut d, &mut main, 0, 1);

    let compare = |d: &mut Controller, main: &mut Vec<u32>| {
        main[CLP as usize] = 1 << 8;
        d.write(reg::COMMAND, 0o10, main); // 0010 Read compare
        d.write(reg::CLP, CLP, main);
        d.write(reg::DISK_ADDRESS, 0, main);
        d.write(reg::START, 0, main);
    };

    compare(&mut d, &mut main);
    assert_eq!(d.status() & status::READ_COMPARE_DIFFERENCE, 0, "page 1 is the block");

    main[256 + 2] ^= 1;
    compare(&mut d, &mut main);
    assert_ne!(d.status() & status::READ_COMPARE_DIFFERENCE, 0, "one bit differs");
    // Memory is not touched by a compare, so the difference is still there.
    assert_eq!(main[256 + 2], 0o1457 ^ 1, "the cylinder count, one bit flipped");
}

/// A write goes to the controller's own copy and never to the image.
/// `vendor/` is fetched material; a test that modified it would corrupt the
/// only oracle this project has.
#[test]
fn a_write_is_read_back_and_the_image_is_untouched() {
    let Some(path) = image() else { return };
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    let (mut d, mut main) = loaded().unwrap();

    // Write page 1 of memory to block 5, then read it back into page 2.
    for (i, w) in main[256..256 + BLOCK_WORDS].iter_mut().enumerate() {
        *w = 0o777000 + i as u32;
    }
    main[CLP as usize] = 1 << 8;
    d.write(reg::COMMAND, 0o11, &mut main); // 0011 Write
    d.write(reg::CLP, CLP, &mut main);
    d.write(reg::DISK_ADDRESS, 5, &mut main);
    d.write(reg::START, 0, &mut main);
    assert_eq!(d.status() & status::FAULT, 0, "the write faulted");

    read_block(&mut d, &mut main, 5, 2);
    assert_eq!(&main[256..256 + BLOCK_WORDS], &main[512..512 + BLOCK_WORDS]);

    // A second controller on the same file sees the pack as it was.
    let (mut fresh, mut other) = loaded().unwrap();
    read_block(&mut fresh, &mut other, 5, 2);
    assert_ne!(&other[512..512 + BLOCK_WORDS], &main[512..512 + BLOCK_WORDS]);
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
}

/// "Nonexistent Memory Error.  Indicates that memory (or other XBUS device)
/// failed to respond within 15 microseconds.  This error stops the transfer."
#[test]
fn a_ccw_pointing_past_memory_sets_nxm_and_stops() {
    let Some((mut d, mut main)) = loaded() else { return };
    let last = (main.len() / 256 - 1) as u32;
    main[CLP as usize] = last << 8 | 1; // the last page, then one past the end
    main[CLP as usize + 1] = (last + 1) << 8;
    d.write(reg::COMMAND, 0, &mut main);
    d.write(reg::CLP, CLP, &mut main);
    d.write(reg::DISK_ADDRESS, 0, &mut main);
    d.write(reg::START, 0, &mut main);

    assert_ne!(d.status() & status::NXM, 0, "NXM not reported");
    assert_eq!(main[last as usize * 256], 0o11420440514, "the first page still transferred");
}

/// Seeking off the end of the pack is a seek error, and "Reset the error by
/// using the Recalibrate command".
#[test]
fn seeking_off_the_pack_is_a_seek_error_until_recalibrated() {
    let Some((mut d, mut main)) = loaded() else { return };
    d.write(reg::DISK_ADDRESS, Geometry::T300.cylinders << 16, &mut main);
    d.write(reg::COMMAND, 0o4, &mut main); // 0004 Seek
    d.write(reg::START, 0, &mut main);
    assert_ne!(d.status() & status::SEEK_ERROR, 0);

    d.write(reg::DISK_ADDRESS, 0, &mut main);
    d.write(reg::COMMAND, 0o1005, &mut main); // 1005 Recalibrate
    d.write(reg::START, 0, &mut main);
    assert_eq!(d.status() & status::SEEK_ERROR, 0);
    assert_eq!(d.status() & status::DRIVE_NOT_READY, 0);
}

// ---------------------------------------------------------------------------
// The format on the pack, and the drive on the cable
// ---------------------------------------------------------------------------

use muir::disk_unit::{
    BIT_NS, ControllerLines, Ecc, INDEX_PULSE_NS, REVOLUTION_NS, SECTOR_NS, SECTOR_PULSE_NS, Tag,
    Trident, format, parse_sector, sector_image,
};

/// Some words that are not all alike, from a seed.
fn words(seed: u32) -> [u32; BLOCK_WORDS] {
    let mut x = seed | 1;
    let mut out = [0u32; BLOCK_WORDS];
    for w in out.iter_mut() {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *w = x;
    }
    out
}

/// The bits of some bytes, low-order bit first.
fn bits_of(bytes: &[u8]) -> Vec<bool> {
    (0..bytes.len() * 8).map(|k| bytes[k / 8] >> (k % 8) & 1 != 0).collect()
}

/// **MIT's format adds up to the sector length on the drawing.** `disk.text`
/// gives the block's ten fields; `dctrid.drw` says the drive's jumpers are
/// set for 1,164 bytes a sector; they are the same number.
#[test]
fn the_format_adds_up_to_the_sector_length() {
    use format::*;
    let sum = PREAMBLE + VFO_LOCK + 1 + HEADER + ECC + VFO_RELOCK + 1 + 1 + DATA + ECC + POSTAMBLE;
    assert_eq!(sum, SECTOR);
    assert_eq!(SECTOR, 1164);
    assert_eq!((HEADER_SYNC_AT, DATA_SYNC_AT, DATA_AT), (61, 90, 92));
    // "A track contains (approximately) 20160. bytes ... one every 1164.
    // bytes, with a little left over at the end of the track."
    let track = 20160usize;
    assert!(17 * SECTOR < track && 18 * SECTOR > track);
    // The next-block code, by the geometry.
    let g = Geometry::T300;
    assert_eq!(format::next_block_code(&g, 0, 0, 0), 0, "following block on same track");
    assert_eq!(format::next_block_code(&g, 0, 0, 16), 1, "block 0 on next head");
    assert_eq!(format::next_block_code(&g, 0, 18, 16), 2, "block 0 on head 0 of next cylinder");
    assert_eq!(format::next_block_code(&g, 814, 18, 16), 3, "end of disk");
}

/// The register written the textbook way: a division by `x^32 + x^23 +
/// x^21 + x^11 + x^2 + 1`, feedback from the top, the remainder out top
/// bit first. If the reading of DCECC in `Ecc` is right, the two agree on
/// every checkword.
fn textbook(bytes: &[u8]) -> [u8; 4] {
    const POLY: u32 = 1 << 23 | 1 << 21 | 1 << 11 | 1 << 2 | 1;
    let mut crc = 0u32;
    for &b in bytes {
        for k in 0..8 {
            let feedback = (b >> k & 1 != 0) ^ (crc >> 31 & 1 != 0);
            crc <<= 1;
            if feedback {
                crc ^= POLY;
            }
        }
    }
    let mut out = [0u8; 4];
    for k in 0..32 {
        if crc >> (31 - k) & 1 != 0 {
            out[k / 8] |= 1 << (k % 8);
        }
    }
    out
}

/// **The checkword leaves the register at zero**, one bit wrong does not,
/// and the register is the polynomial its doc comment names.
#[test]
fn the_checkword_leaves_the_register_at_zero() {
    let mut x = 0x2545_f491u32;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x as u8
    };
    for len in [0usize, 1, 3, 4, 5, 32, 1024] {
        let bytes: Vec<u8> = (0..len).map(|_| next()).collect();
        let checkword = Ecc::over(&bytes);
        let mut e = Ecc::default();
        e.feed(&bytes);
        e.feed(&checkword);
        assert!(e.checks(), "{len} bytes");
        assert_eq!(checkword, textbook(&bytes), "{len} bytes, the textbook division");
        if len > 0 {
            let mut bad = bytes.clone();
            bad[len / 2] ^= 1 << 3;
            let mut e = Ecc::default();
            e.feed(&bad);
            e.feed(&checkword);
            assert!(!e.checks(), "{len} bytes with one bit wrong");
        }
    }
    // Nothing fed, nothing owed: a clear register's checkword is zero.
    assert_eq!(Ecc::over(&[]), [0, 0, 0, 0]);
}

/// **A sector serialised parses back**, header and data, both checkwords
/// good; and a bit flipped in the data is caught by its checkword alone.
#[test]
fn a_sector_image_parses_back() {
    let g = Geometry::T300;
    let data = words(7);
    let image = sector_image(&g, 0o1234, 3, 16, &data);
    assert_eq!(image.len(), format::SECTOR);
    assert_eq!(image[format::HEADER_SYNC_AT], format::SYNC);
    assert_eq!(image[format::DATA_SYNC_AT], format::SYNC);
    assert_eq!(image[format::DATA_SYNC_AT + 1], format::PAD);
    assert!(image[..format::HEADER_SYNC_AT].iter().all(|&b| b == 0xff), "preamble");
    let header = u32::from_le_bytes(image[62..66].try_into().unwrap());
    assert_eq!(header >> 30, 1, "the last block of a track that is not the last head");
    assert_eq!(header & 0x0fff_ffff, 0o1234 << 16 | 3 << 8 | 16);

    let s = parse_sector(&bits_of(&image)).expect("two syncs");
    assert_eq!(s.header, header);
    assert!(s.header_checks && s.data_checks);
    assert_eq!(s.data, data);

    let mut bad = image.clone();
    bad[format::DATA_AT + 500] ^= 1 << 5;
    let s = parse_sector(&bits_of(&bad)).unwrap();
    assert!(s.header_checks && !s.data_checks);
    assert_ne!(s.data, data);

    // Nothing but ones is not a sector.
    assert!(parse_sector(&vec![true; format::SECTOR * 8]).is_none());
}

/// A drive with the controller's select line down and nothing else.
fn selected() -> ControllerLines {
    ControllerLines { select: true, ..Default::default() }
}

/// **The drive pulses eighteen times a turn, the index pulse the long
/// one**, and the two widths fall on either side of the controller's
/// one-shot.
///
/// The index, then seventeen sector pulses a sector apart, and then the
/// leftover `sys/doc/disk.text` describes --- "17. sector pulses per track,
/// or one every 1164. bytes, with a little left over at the end of the
/// track" --- which the index closes.  Seventeen sectors of 1,164 bytes are
/// 19,788 of the track's 20,160, so the leftover is the shortest gap of the
/// turn and the one that gives the block counter its 17.
#[test]
fn the_drive_pulses_eighteen_times_a_turn_with_one_long_one() {
    let mut d = Trident::new(Unit::blank(Geometry::T300), 1_000);
    let mut now = 1_000;
    let (mut pulses, mut last, mut began) = (Vec::new(), false, 0);
    while now < 1_000 + REVOLUTION_NS + INDEX_PULSE_NS + 100 {
        let l = d.lines(now);
        if l.sector_index && !last {
            began = now;
        }
        if !l.sector_index && last {
            pulses.push((began, now - began));
        }
        last = l.sector_index;
        now = d.next_change(now);
    }
    assert_eq!(pulses.len(), 19, "eighteen, and the next turn's index");
    assert_eq!(pulses[0], (1_000, INDEX_PULSE_NS), "the index at power-on");
    assert_eq!(pulses[18].1, INDEX_PULSE_NS, "and one revolution later");
    assert!(pulses[1..18].iter().all(|&(_, w)| w == SECTOR_PULSE_NS), "seventeen sector pulses");
    let spacing: Vec<u64> = pulses.windows(2).map(|w| w[1].0 - w[0].0).collect();
    assert!(spacing[..17].iter().all(|&s| s == SECTOR_NS), "a sector apart: {spacing:?}");
    assert_eq!(spacing[17], REVOLUTION_NS - 17 * SECTOR_NS, "the leftover");
    assert_eq!(pulses[18].0 - pulses[0].0, REVOLUTION_NS);
    // The one-shot at DCTRID 0B09 is 2,250 ns, by the drawing; the
    // seventeen sectors leave a leftover rather than filling the turn; the
    // clock's halves are equal.
    const {
        assert!(SECTOR_PULSE_NS < 2_250 && 2_250 < INDEX_PULSE_NS);
        assert!(SECTOR_NS == format::SECTOR as u64 * 8 * BIT_NS);
        assert!(17 * SECTOR_NS < REVOLUTION_NS);
        assert!(BIT_NS.is_multiple_of(2));
    }
}

/// **A cylinder tag seeks, and the seek raises attention; a head tag
/// selects; read gate under the control tag clears attention; recalibrate
/// goes home.**
#[test]
fn the_tags_do_what_the_microcode_expects_of_them() {
    let mut d = Trident::new(Unit::blank(Geometry::T300), 0);
    d.seek_settle_ns = 1_000;
    d.seek_ns_per_cylinder = 10;
    let sel = selected();
    d.observe(0, sel);
    let l = d.lines(0);
    assert!(l.on_cylinder && l.on_line && l.selected && !l.attention && !l.fault);

    // The cylinder to the bus, then the tag: `000` then `001`.
    d.observe(100, ControllerLines { bus: 0o1234, ..sel });
    d.observe(200, ControllerLines { bus: 0o1234, cylinder_tag: true, ..sel });
    assert!(!d.lines(200).on_cylinder, "seeking");
    d.observe(300, ControllerLines { bus: 0o1234, ..sel });
    let done = 200 + 1_000 + 10 * 0o1234;
    assert!(!d.lines(done - 1).on_cylinder);
    assert!(d.lines(done).on_cylinder);
    assert!(d.lines(done).attention, "seek complete");
    assert_eq!(d.position(), (0o1234, 0));
    assert_eq!(d.tags, [(200, Tag::Cylinder(0o1234))]);

    // Head 3 with both offset bits.
    d.observe(done + 10, ControllerLines { bus: 0o303, head_tag: true, ..sel });
    assert_eq!(d.position(), (0o1234, 3));
    assert_eq!(d.offset, (true, true));

    // Read gate with pre gate under the control tag: attention off.
    d.observe(done + 20, ControllerLines { bus: 1 << 6 | 1 << 2, control_tag: true, ..sel });
    assert!(!d.lines(done + 20).attention);
    assert_eq!(d.lines(done + 20).data, Some(true), "reading ones, wherever the head is");
    d.observe(done + 30, sel);
    assert_eq!(d.lines(done + 30).data, None, "the tag off, the read amplifier off");

    // Recalibrate under the control tag.
    d.observe(done + 40, ControllerLines { bus: 1 << 1, control_tag: true, ..sel });
    d.observe(done + 50, sel);
    let home = done + 40 + 1_000 + 10 * 0o1234;
    assert!(!d.lines(home - 1).on_cylinder);
    assert!(d.lines(home).on_cylinder && d.lines(home).attention);
    assert_eq!(d.position(), (0, 3));

    // Off the pack: seek incomplete, at once, with attention.
    d.observe(home + 10, ControllerLines { bus: 0o1777, cylinder_tag: true, ..sel });
    let l = d.lines(home + 10);
    assert!(l.on_cylinder && l.seek_incomplete && l.attention);

    // Not selected, nothing is heard and nothing answers.
    d.observe(home + 20, ControllerLines { bus: 0o100, cylinder_tag: true, ..Default::default() });
    assert_eq!(d.position(), (0, 3));
    assert!(!d.lines(home + 20).selected);
}

/// When sector `k` under the head at `d` begins, on or after `from`.
fn sector_begins(d: &Trident, from: u64, k: u32) -> u64 {
    let mut now = from;
    loop {
        let (sector, into) = d.turn(now);
        if sector == k && into == 0 {
            return now;
        }
        now = if sector == k { now - into + REVOLUTION_NS } else { d.next_change(now) };
        now = now.min(from + 2 * REVOLUTION_NS);
        assert!(now < from + 2 * REVOLUTION_NS, "sector {k} never comes round");
    }
}

/// **A checkpoint holds the drive where it stood.** A drive part way
/// round its spindle, part way through a seek, and part way through
/// serialising a sector, saved and read back into a drive built fresh,
/// puts the same thing on the cable at the same times as the drive it
/// came from --- through the end of the sector and through the seek
/// completing.
///
/// The last third of this test is the reason the first two matter: the
/// same window run on a drive built fresh at the resume's instant, which
/// is what a resume did before there was anything to read back. It
/// disagrees, and the assertion is that it disagrees, so that this test
/// cannot pass by saving nothing.
#[test]
fn a_checkpoint_holds_the_drive_where_it_stood() {
    let data = words(11);
    let mut unit = Unit::blank(Geometry::T300);
    assert!(unit.write_block_at(3, 1, 2, &data));
    let mut d = Trident::new(unit.clone(), 0);
    // A seek slow enough to be still running when the save falls, which
    // is a sector or so in: settling is the whole of it here, the three
    // cylinders 30 ns more.
    d.seek_settle_ns = 3_000_000;
    d.seek_ns_per_cylinder = 10;
    let sel = selected();
    d.observe(0, sel);
    // Head 1, then a seek to cylinder 3, then read gate: the arm moving,
    // the head chosen and the gate open all at once.
    d.observe(100, ControllerLines { bus: 1, head_tag: true, ..sel });
    d.observe(200, ControllerLines { bus: 3, cylinder_tag: true, ..sel });
    let reading = ControllerLines { bus: 1 << 6, control_tag: true, ..sel };
    d.observe(300, reading);
    // Part way into a sector, and with the seek still running: settling
    // is 1,000 ns and three cylinders 30 more, from 200.
    let at = sector_begins(&d, 300, 1) + 40 * BIT_NS;
    let seek_ends = 200 + 3_000_000 + 30;
    assert!(at < seek_ends, "the save is to fall while the arm is still moving");
    let _ = d.lines(at);

    let mut w = muir::checkpoint::Writer::new();
    d.save(&mut w);
    let saved = w.finish();
    // Powered on at an instant of its own, as a resume's drive is: the
    // far end is built fresh at the join's time, not at the button. A
    // phase that did not come out of the checkpoint would be that
    // instant's, and the track would have jumped.
    let mut back = Trident::new(Unit::blank(Geometry::T300), 12_345_678);
    back.load(&mut muir::checkpoint::Reader::new(&saved)).expect("the drive back");

    // Both drives from the save onwards, stepped by the one that was
    // saved and asked at every change it makes.
    let mut fresh = Trident::new(unit, at);
    fresh.observe(at, reading);
    let mut now = at;
    let mut differed = 0;
    let end = at + 3 * SECTOR_NS;
    assert!(seek_ends < end, "and the window is to see the arm arrive");
    while now < end {
        assert_eq!(back.lines(now), d.lines(now), "the cable at {now}");
        if fresh.lines(now) != d.lines(now) {
            differed += 1;
        }
        let next = d.next_change(now);
        assert_eq!(back.next_change(now), next, "the next change at {now}");
        assert!(next > now, "time moves");
        now = next;
    }
    assert_eq!(back.unit.block_at(3, 1, 2), Some(data), "the pack came too");
    assert!(differed > 0, "a drive built fresh is not this drive: that is what the save is for");
}

/// **Under read gate the drive serialises the sector under the head**, in
/// MIT's format, one bit a clock from the sector pulse, ones in the gap.
#[test]
fn under_read_gate_the_drive_serialises_the_sector_under_the_head() {
    let data = words(11);
    let mut unit = Unit::blank(Geometry::T300);
    assert!(unit.write_block_at(0, 0, 2, &data));
    let mut d = Trident::new(unit, 0);
    let sel = selected();
    d.observe(0, sel);
    let began = sector_begins(&d, 0, 2);
    d.observe(began, ControllerLines { bus: 1 << 6 | 1 << 2, control_tag: true, ..sel });
    let first = began.div_ceil(BIT_NS) * BIT_NS;
    let bits: Vec<bool> =
        (0..format::SECTOR * 8).map(|k| d.lines(first + k as u64 * BIT_NS).data.unwrap()).collect();
    let s = parse_sector(&bits).expect("a sector");
    assert_eq!(s.header, 2, "cylinder 0, head 0, block 2, the next block following");
    assert!(s.header_checks && s.data_checks);
    assert_eq!(s.data, data);
    let gap = first + (format::SECTOR * 8 + 5) as u64 * BIT_NS;
    assert_eq!(d.lines(gap).data, Some(true), "ones in the gap");
    // The clock: high for the first half of every bit.
    assert!(d.lines(first).clock && !d.lines(first + BIT_NS / 2).clock);
    // Another sector, another block: block 3 is blank, and says so.
    let began = sector_begins(&d, gap, 3);
    let first = began.div_ceil(BIT_NS) * BIT_NS;
    let bits: Vec<bool> =
        (0..format::SECTOR * 8).map(|k| d.lines(first + k as u64 * BIT_NS).data.unwrap()).collect();
    let s = parse_sector(&bits).unwrap();
    assert_eq!((s.header, s.data), (3, [0u32; BLOCK_WORDS]));
}

/// **A checkpoint holds a sector half written.** The controller sends
/// half a sector's bits, the run is checkpointed, and a drive read back
/// from it takes the other half and drops the gate: the block lands on
/// the pack whole. Bits under the head that the pack does not hold yet
/// are the drive's alone --- nothing else in the machine has them --- so
/// a checkpoint that did not carry them would put a torn sector on the
/// pack, or none.
#[test]
fn a_checkpoint_holds_a_sector_half_written() {
    let g = Geometry::T300;
    let mut d = Trident::new(Unit::blank(g), 0);
    let sel = selected();
    d.observe(0, sel);
    let writing = ControllerLines { bus: 1 << 7 | 1 << 2, control_tag: true, ..sel };
    let data = words(3);
    let image = sector_image(&g, 0, 0, 5, &data);
    let bits = bits_of(&image);
    let began = sector_begins(&d, 0, 5);
    let first = began.div_ceil(BIT_NS) * BIT_NS;
    let half = bits.len() / 2;
    for (k, bit) in bits[..half].iter().enumerate() {
        d.observe(first + k as u64 * BIT_NS, ControllerLines { write_data: Some(*bit), ..writing });
    }

    let mut w = muir::checkpoint::Writer::new();
    d.save(&mut w);
    let saved = w.finish();
    let mut back = Trident::new(Unit::blank(g), 7_654_321);
    back.load(&mut muir::checkpoint::Reader::new(&saved)).expect("the drive back");

    for (k, bit) in bits[half..].iter().enumerate() {
        let at = first + (half + k) as u64 * BIT_NS;
        back.observe(at, ControllerLines { write_data: Some(*bit), ..writing });
    }
    back.observe(first + bits.len() as u64 * BIT_NS, sel);
    assert_eq!(back.bad_writes, 0, "the sector parsed as the format");
    assert_eq!(back.unit.block_at(0, 0, 5), Some(data), "and the block is on the pack");
}

/// **Under write gate the drive records what it is sent**, and when the
/// gate drops the sector is parsed and its block lands on the pack ---
/// the whole sector as the formatter writes it, or the data part alone
/// as the write command rewrites it over an existing header.
#[test]
fn under_write_gate_the_drive_records_what_it_is_sent() {
    let g = Geometry::T300;
    let mut d = Trident::new(Unit::blank(g), 0);
    let sel = selected();
    d.observe(0, sel);
    let writing = ControllerLines { bus: 1 << 7 | 1 << 2, control_tag: true, ..sel };

    // The whole sector, as Write All does.
    let data = words(3);
    let image = sector_image(&g, 0, 0, 5, &data);
    let began = sector_begins(&d, 0, 5);
    let first = began.div_ceil(BIT_NS) * BIT_NS;
    for (k, bit) in bits_of(&image).iter().enumerate() {
        d.observe(first + k as u64 * BIT_NS, ControllerLines { write_data: Some(*bit), ..writing });
    }
    let end = first + (format::SECTOR * 8) as u64 * BIT_NS;
    d.observe(end, sel);
    assert_eq!(d.bad_writes, 0);
    assert_eq!(d.unit.block_at(0, 0, 5), Some(data));

    // The data part alone, from the relock on, over the header just
    // written: what the write command does from `134`.
    let data2 = words(5);
    let image2 = sector_image(&g, 0, 0, 5, &data2);
    let began = sector_begins(&d, end, 5);
    let first = began.div_ceil(BIT_NS) * BIT_NS;
    let from = (format::HEADER_SYNC_AT + 1 + format::HEADER + format::ECC + 1) * 8;
    for (k, bit) in bits_of(&image2).iter().enumerate().skip(from) {
        d.observe(first + k as u64 * BIT_NS, ControllerLines { write_data: Some(*bit), ..writing });
    }
    d.observe(first + (format::SECTOR * 8) as u64 * BIT_NS, sel);
    assert_eq!(d.bad_writes, 0);
    assert_eq!(d.unit.block_at(0, 0, 5), Some(data2));

    // Garbage is dropped and counted.
    let began = sector_begins(&d, first + SECTOR_NS, 6);
    let first = began.div_ceil(BIT_NS) * BIT_NS;
    for k in 0..format::SECTOR * 8 {
        d.observe(
            first + k as u64 * BIT_NS,
            ControllerLines { write_data: Some(k % 3 == 0), ..writing },
        );
    }
    d.observe(first + (format::SECTOR * 8) as u64 * BIT_NS, sel);
    assert_eq!(d.bad_writes, 1);
    assert_eq!(d.unit.block_at(0, 0, 6), Some([0u32; BLOCK_WORDS]), "untouched");
}

/// A small pack in a scratch file: two cylinders of the T-300's tracks.
fn scratch_pack(name: &str) -> (PathBuf, Geometry) {
    let g = Geometry { cylinders: 2, heads: 2, blocks_per_track: 17 };
    let p = std::env::temp_dir().join(format!("muir-{name}-{}.img", std::process::id()));
    std::fs::write(&p, vec![0u8; g.blocks() as usize * BLOCK_WORDS * 4]).unwrap();
    (p, g)
}

/// **A pack opened read-write is written through to the file**, and one
/// opened read-only is not: its writes stay in the run.
#[test]
fn a_pack_opened_read_write_is_written_through() {
    let (p, g) = scratch_pack("rw");
    let data = words(41);
    {
        let mut u = Unit::open_rw(&p, g).unwrap();
        assert!(u.writable());
        assert!(u.write_block_at(1, 1, 5, &data));
        assert_eq!(u.block_at(1, 1, 5), Some(data));
    }
    let mut again = Unit::open(&p, g).unwrap();
    assert!(!again.writable());
    assert_eq!(again.block_at(1, 1, 5), Some(data), "on the file");
    let other = words(42);
    assert!(again.write_block_at(1, 1, 5, &other));
    assert_eq!(again.block_at(1, 1, 5), Some(other), "in the run");
    drop(again);
    let mut third = Unit::open(&p, g).unwrap();
    assert_eq!(third.block_at(1, 1, 5), Some(data), "the file untouched by a read-only unit");
    std::fs::remove_file(&p).unwrap();
}

/// **A block written as the pack already holds it is not kept**: the file
/// answers the read the same, so only what a run changed stays in it, and
/// goes into a checkpoint.
#[test]
fn a_block_written_as_the_pack_holds_it_is_not_kept() {
    let (p, g) = scratch_pack("same");
    let mut u = Unit::open(&p, g).unwrap();
    let data = words(43);
    assert!(u.write_block_at(1, 0, 3, &data));
    assert_eq!(u.written_blocks(), 1, "a changed block is kept");
    assert!(u.write_block_at(1, 0, 4, &[0u32; BLOCK_WORDS]));
    assert_eq!(u.written_blocks(), 1, "zeros over a blank block are not");
    assert!(u.write_block_at(1, 0, 3, &[0u32; BLOCK_WORDS]));
    assert_eq!(u.written_blocks(), 0, "the file's own content written back lets the block go");
    assert_eq!(u.block_at(1, 0, 3), Some([0u32; BLOCK_WORDS]));
    std::fs::remove_file(&p).unwrap();
}

/// **The drive's read-only switch makes a write a fault**: "Writing while
/// the disk is read-only causes a fault", and nothing lands on the pack.
#[test]
fn the_read_only_switch_makes_a_write_a_fault() {
    let g = Geometry::T300;
    let mut unit = Unit::blank(g);
    unit.read_only = true;
    let mut d = Trident::new(unit, 0);
    let sel = selected();
    d.observe(0, sel);
    assert!(d.lines(0).read_only && !d.lines(0).fault);
    let writing = ControllerLines { bus: 1 << 7 | 1 << 2, control_tag: true, ..sel };
    let image = sector_image(&g, 0, 0, 5, &words(3));
    let began = sector_begins(&d, 0, 5);
    let first = began.div_ceil(BIT_NS) * BIT_NS;
    for (k, bit) in bits_of(&image).iter().enumerate() {
        d.observe(first + k as u64 * BIT_NS, ControllerLines { write_data: Some(*bit), ..writing });
    }
    d.observe(first + (format::SECTOR * 8) as u64 * BIT_NS, sel);
    assert!(d.lines(first).fault, "device check");
    assert_eq!(d.unit.block_at(0, 0, 5), Some([0u32; BLOCK_WORDS]), "nothing written");
    // Fault clear under the control tag clears it.
    let t = first + (format::SECTOR * 8 + 10) as u64 * BIT_NS;
    d.observe(t, ControllerLines { bus: 1 << 3, control_tag: true, ..sel });
    assert!(!d.lines(t).fault);
}

/// A controller with a blank T-300 on unit 0, and the memory the tests
/// above use, for the commands that move no words.
fn blank() -> (Controller, Vec<u32>) {
    let mut d = Controller::default();
    d.attach(0, Unit::blank(Geometry::T300));
    (d, vec![0; 1 << 16])
}

/// **The block counter turns with the spindle.** `STATUS<31:24>`: "The
/// block-counter of the selected unit.  This tells you its current
/// rotational position."  A T-300 turns at 3,600 rpm and pulses eighteen
/// times a revolution --- the index and seventeen sector pulses a sector
/// apart --- so the count steps once every 968 us, reaches 17 in the
/// track's leftover, which holds no block, and clears at the index.  That
/// 17 is the value `DCHECK-BLOCK-COUNTER` looks for.  On the board the step
/// lands as each pulse ends and the clear as the index pulse ends, so the
/// count is one behind for a pulse's width:
/// `the_block_counter_follows_the_drives_sector_pulses` in
/// `tests/cadrdc_netlist.rs` reads the netlist controller at these same
/// offsets.  With no drive there are no pulses and the byte is zero.
#[test]
fn the_block_counter_turns_with_the_spindle() {
    let (mut d, _main) = blank();
    // The `k`th pulse of the spindle: eighteen a turn, the eighteenth
    // being the next index, which the leftover is short of a sector from.
    let pulse = |k: u64| (k / 18) * REVOLUTION_NS + (k % 18) * SECTOR_NS;
    // 100 us into each region of a turn and a quarter: the pulse over.
    for k in 0..23u64 {
        d.advance(pulse(k) + 100_000);
        assert_eq!(d.status() >> 24, (k % 18) as u32, "pulse {k}");
    }
    // 300 ns into the third sector pulse, still on: the count is still 2.
    d.advance(pulse(3) + 300);
    assert_eq!(d.status() >> 24, 2, "during a sector pulse");
    // 2.5 us into the second turn's index pulse, longer than a sector
    // pulse and not yet over: 17, the leftover's, until it ends.
    d.advance(pulse(18) + 2_500);
    assert_eq!(d.status() >> 24, 17, "during the index pulse");
    d.advance(pulse(18) + 6_000);
    assert_eq!(d.status() >> 24, 0, "after the index pulse");
    // No drive on the selected unit: nothing to count.
    let mut d = Controller::default();
    d.advance(pulse(5) + 100_000);
    assert_eq!(d.status() >> 24, 0, "no drive");
}

/// **The block counter shows every value CC asks for and no other.**
/// `DCHECK-BLOCK-COUNTER` in `sys/cc/dcheck.lisp` reads `STATUS<31:24>`
/// for half a second and holds what it saw to `'(0 1 2 ... 17)` ---
/// "Vandals: Yes, a value of 17. can appear here" --- printing "Values not
/// seen (octal)" for any of those it missed and "Erroneous values seen"
/// for anything else.  A revolution is every value the counter can show,
/// so this is that check on this model: sampled ten microseconds apart,
/// which is fine enough for the leftover, the shortest region of the turn
/// at 203 us.
#[test]
fn the_block_counter_shows_every_value_dcheck_wants() {
    let (mut d, _main) = blank();
    let mut seen = std::collections::BTreeSet::new();
    let mut now = 0;
    while now < REVOLUTION_NS {
        d.advance(now);
        seen.insert(d.status() >> 24);
        now += 10_000;
    }
    let want: std::collections::BTreeSet<u32> = (0..=17).collect();
    assert_eq!(seen, want, "not seen: {:?}", want.difference(&seen).collect::<Vec<_>>());
}

/// **Reset takes effect in the store to the command register.** MIT:
/// "0016 Reset. This stops the current transfer and resets the controller.
/// This command takes effect as soon as it is stored in the command
/// register; no store in START is required. After storing a Reset command
/// you should store 0 in the command register to turn off the reset
/// condition."
#[test]
fn a_reset_stored_in_the_command_register_takes_effect_at_once() {
    let (mut d, mut main) = blank();
    d.timed = true;
    d.advance(1_000);
    main[CLP as usize] = 2 << 8;
    d.write(reg::COMMAND, 0, &mut main);
    d.write(reg::CLP, CLP, &mut main);
    d.write(reg::DISK_ADDRESS, 0, &mut main);
    d.write(reg::START, 0, &mut main);
    assert_eq!(d.status() & status::NOT_ACTIVE, 0, "active while the transfer runs");
    d.write(reg::COMMAND, 0o16, &mut main);
    assert_ne!(d.status() & status::NOT_ACTIVE, 0, "reset stops it, with no store in START");
    d.write(reg::COMMAND, 0, &mut main);
    assert_ne!(d.status() & status::NOT_ACTIVE, 0, "0 turns the reset off and leaves it ready");
    // A store in START with the reset code still in the register starts
    // nothing, and the controller stays ready.
    d.write(reg::COMMAND, 0o16, &mut main);
    d.write(reg::START, 0, &mut main);
    assert_ne!(d.status() & status::NOT_ACTIVE, 0);
}

/// **Codes 14 and 15 are the seek and the at-ease of sectors 4 and 5.**
/// The command's `<2:0>` is the microcode sector and `<3>` only steers the
/// memory channel --- "1 means from-memory, 0 means to-memory" --- and
/// `cadrdc/newdsk.31` says "Commands 4-7 do not use the memory channel".
/// So the two combinations MIT's table leaves out of those sectors are the
/// seek and the miscellaneous command with a dead bit set, and the board
/// runs them as such: `tests/cadrdc_netlist.rs` walks MIT's board through
/// both. Here 14 seeks --- off the pack, a seek error, as 04 --- 1015
/// recalibrates as 1005 does, and 15 is at ease.
#[test]
fn codes_14_and_15_are_the_seek_and_at_ease_of_their_sectors() {
    let (mut d, mut main) = blank();
    d.write(reg::DISK_ADDRESS, 0o7777 << 16, &mut main);
    d.write(reg::COMMAND, 0o14, &mut main);
    d.write(reg::START, 0, &mut main);
    let s = d.status();
    assert_ne!(s & status::SEEK_ERROR, 0, "14 seeks, off the pack: {s:o}");
    assert_ne!(s & status::ATTENTION, 0, "and the seek raises attention: {s:o}");
    assert_eq!(s & status::TIMEOUT, 0, "no timeout: {s:o}");

    d.write(reg::COMMAND, 0o1015, &mut main);
    d.write(reg::START, 0, &mut main);
    let s = d.status();
    assert_eq!(s & status::SEEK_ERROR, 0, "1015 recalibrates: {s:o}");
    assert_ne!(s & status::ATTENTION, 0, "which raises attention: {s:o}");

    d.write(reg::COMMAND, 0o15, &mut main);
    d.write(reg::START, 0, &mut main);
    let s = d.status();
    assert_eq!(
        s & (status::ATTENTION | status::ANY_ATTENTION | status::TIMEOUT),
        0,
        "15 is at ease: {s:o}"
    );
}

/// **The two engines time out at the same instant, off the board's own
/// clock.**
///
/// [`TIMEOUT_NS`] is not a figure of its own: it is the 74LS124 VCO at
/// DCTMOT 0B04 section 1, whose period is the drawing's property on that
/// body, counted down by the 74393 at 0C03. The netlist engine runs those
/// two parts and reaches the instant by itself; this holds the behavioural
/// model to the same arithmetic, so the two cannot drift apart again.
///
/// They did drift, and the figure they drifted to was wrong as well. This
/// engine took `sys/doc/disk.text`'s 2.5 seconds and the netlist board ran
/// at 1.536; they were made to agree at 1.536, on the drawing's own
/// property, and the SN74LS124's data sheet has since shown the drawing to
/// be the stale one --- see [`TIMEOUT_NS`]. The text was right all along.
#[test]
fn both_engines_take_the_timeout_from_the_same_two_facts() {
    let (period, over) = muir::chip::DISK_TIMEOUT_VCO_PERIOD;
    assert_eq!(over, 1, "the disk timeout clock's period is a whole number of ns");
    // The SN74LS124's own data sheet: fo = 1e-4 / Cext for the LS part, and
    // the board has 2 uF on this section --- two 1 uF bodies joined BARE
    // across VCO.C1 and VCO.C2 by `dc.wlr`, both given as 1 uF by MIT's
    // parts list. 1e-4 / 2e-6 is 50 Hz.
    assert_eq!(period, 20_000_000, "2 uF through the LS124's own formula: 20 ms");
    assert_eq!(muir::disk_controller::TIMEOUT_DIVIDER, 128, "the 74393 at 0C03 divides by 128");
    assert_eq!(TIMEOUT_NS, period * 128, "the model times out on the board's own count");
    // Which is what MIT's own text says: "a disk operation took longer than
    // 2.5 seconds". The drawing's `;Period = 12 ms` is 1.2 uF, one capacitor
    // and some stray, and its `;1.5 SEC` note is that stale figure counted
    // down; the second body went on the board after the property was
    // written.
    assert_eq!(TIMEOUT_NS, 2_560_000_000, "2.56 s, which is disk.text's 2.5 seconds");
}

/// **A reserved code hangs the controller until its timer stops it.**
/// `xxx7` is sector 7, which `cadrdc/newdsk.31` leaves unwritten, so the
/// sequencer starts and never finishes; MIT: it "will currently hang the
/// controller, causing a timeout error (bit 11 in the status register)".
/// So the controller is active with no error for [`TIMEOUT_NS`], then
/// not-active with the error up; the next transfer clears it as it clears
/// the other errors; and a reset stored in the meantime stops it clean.
#[test]
fn a_reserved_code_hangs_the_controller_until_the_timeout() {
    for cmd in [0o07u32, 0o17, 0o1007] {
        let (mut d, mut main) = blank();
        d.write(reg::COMMAND, cmd, &mut main);
        d.write(reg::START, 0, &mut main);
        let s = d.status();
        assert_eq!(s & (status::NOT_ACTIVE | status::TIMEOUT), 0, "{cmd:o}: hung, no error: {s:o}");
        d.advance(TIMEOUT_NS - 1);
        let s = d.status();
        assert_eq!(s & (status::NOT_ACTIVE | status::TIMEOUT), 0, "{cmd:o}: still hung: {s:o}");
        d.advance(TIMEOUT_NS);
        let s = d.status();
        assert_eq!(
            s & (status::NOT_ACTIVE | status::TIMEOUT),
            status::NOT_ACTIVE | status::TIMEOUT,
            "{cmd:o}: stopped by the timeout: {s:o}"
        );
        d.write(reg::COMMAND, 0, &mut main);
        d.write(reg::START, 0, &mut main);
        assert_eq!(d.status() & status::TIMEOUT, 0, "{cmd:o}: the next transfer clears it");
    }
    let (mut d, mut main) = blank();
    d.write(reg::COMMAND, 0o7, &mut main);
    d.write(reg::START, 0, &mut main);
    d.advance(1_000_000);
    d.write(reg::COMMAND, 0o16, &mut main);
    let s = d.status();
    assert_eq!(
        s & (status::NOT_ACTIVE | status::TIMEOUT),
        status::NOT_ACTIVE,
        "reset mid-hang: {s:o}"
    );
}

/// **The record of tags is bounded.** It is there for a test to read back;
/// a long run's seeks would otherwise grow it for the life of the run.
#[test]
fn the_record_of_tags_is_bounded() {
    let mut d = Trident::new(Unit::blank(Geometry::T300), 0);
    let sel = selected();
    d.observe(0, sel);
    let mut last = 0;
    for k in 0..10_000u64 {
        last = 1_000 + 40 * k;
        d.observe(last, ControllerLines { bus: (k & 1) as u16, head_tag: true, ..sel });
        d.observe(last + 20, ControllerLines { bus: (k & 1) as u16, ..sel });
    }
    assert!(d.tags.len() <= 4096, "{} kept", d.tags.len());
    assert_eq!(
        d.tags.last().unwrap(),
        &(last, Tag::Head((9_999 & 1) as u32)),
        "the newest is kept"
    );
}

/// **An image that cannot be read is a drive fault, not a crash.** The
/// pack is a file, and a file can go away under a running machine ---
/// truncated here, after the drive opened it. The read fails, the
/// controller reports the fault MIT gives it for a drive that could not
/// deliver, and the run goes on.
#[test]
fn an_unreadable_image_is_a_drive_fault() {
    let g = Geometry { cylinders: 1, heads: 1, blocks_per_track: 2 };
    let path = std::env::temp_dir().join(format!("muir-disk-short-{}.img", std::process::id()));
    std::fs::write(&path, vec![0u8; 2 * BLOCK_WORDS * 4]).unwrap();
    let mut d = Controller::default();
    d.attach(0, Unit::open(&path, g).unwrap());
    std::fs::File::options().write(true).open(&path).unwrap().set_len(0).unwrap();
    let mut main = vec![0u32; 1 << 16];
    main[CLP as usize] = 2 << 8;
    d.write(reg::COMMAND, 0, &mut main);
    d.write(reg::CLP, CLP, &mut main);
    d.write(reg::DISK_ADDRESS, 0, &mut main);
    d.write(reg::START, 0, &mut main);
    assert_ne!(d.status() & status::FAULT, 0, "the read that failed is a fault");
    assert_ne!(d.status() & status::NOT_ACTIVE, 0, "and the controller is ready again");
    std::fs::remove_file(&path).unwrap();
}

/// `-XBUS INIT` clears the command register --- the 74LS175 at DCCMD 0C21
/// and the 74LS273 at 0C10 --- and the error flops through `-RESET ERR`,
/// and stops the channel through `RESET`; the disk address counters on
/// DCDA and the command list pointer's counters on DCCLP have no clear on
/// it and keep what they were given.
#[test]
fn an_xbus_init_clears_the_command_and_the_errors_and_keeps_the_addresses() {
    let mut d = Controller::default();
    d.attach(0, Unit::blank(Geometry::T80));
    let mut main = vec![0u32; 1 << 16];
    let da = (1 << 16) | (2 << 8) | 3;
    main[CLP as usize] = 3 << 8;
    // A reserved command with the done interrupt enabled, run out to the
    // timer: the timeout error up and the interrupt with it.
    d.write(reg::COMMAND, 0o17 | (1 << 11), &mut main);
    d.write(reg::CLP, CLP, &mut main);
    d.write(reg::DISK_ADDRESS, da, &mut main);
    d.write(reg::START, 0, &mut main);
    d.advance(TIMEOUT_NS);
    assert_ne!(d.status() & status::TIMEOUT, 0);
    assert_ne!(d.status() & status::INTERRUPT_REQUEST, 0);

    d.xbus_init();
    let s = d.status();
    assert_eq!(s & status::TIMEOUT, 0, "the error flops are cleared");
    assert_eq!(s & status::INTERRUPT_REQUEST, 0, "the command register, its enables with it");
    assert_ne!(s & status::NOT_ACTIVE, 0, "and the controller is ready");
    assert_eq!(d.read(reg::DISK_ADDRESS), da, "the 74LS193s keep the disk address");
    // The command list pointer kept too: a read started now goes through
    // the list at `CLP`, into page 3.
    d.write(reg::COMMAND, 0, &mut main);
    d.write(reg::START, 0, &mut main);
    assert_eq!(d.read(reg::MEMORY_ADDRESS), 3 * 256 + 255, "the 74LS569s keep the CLP");
}

/// `DCCCW` latches `XBI<21:8>` of a CCW into the two 74LS374s at 0E25 and
/// 0E26 --- the second has pins 2 to 5 unconnected --- so a CCW with bits
/// 22 or 23 set names the page its lower bits name. `disk.text` writes the
/// field as `<23:8>`; the drawing is the board.
#[test]
fn a_ccw_page_is_twenty_two_bits_wide() {
    let mut d = Controller::default();
    d.attach(0, Unit::blank(Geometry::T80));
    let mut main = vec![0u32; 1 << 16];
    for (k, w) in main[256..512].iter_mut().enumerate() {
        *w = 0o1234567 ^ k as u32;
    }
    // Block 0 written from page 1.
    main[CLP as usize] = 1 << 8;
    d.write(reg::COMMAND, 0o11, &mut main);
    d.write(reg::CLP, CLP, &mut main);
    d.write(reg::DISK_ADDRESS, 0, &mut main);
    d.write(reg::START, 0, &mut main);
    assert_eq!(d.status() & status::FAULT, 0);

    // Read back into page 2 by a CCW carrying bits 22 and 23 as well.
    main[CLP as usize] = (2 << 8) | (3 << 22);
    d.write(reg::COMMAND, 0, &mut main);
    d.write(reg::START, 0, &mut main);
    assert_eq!(d.status() & status::NXM, 0, "the two bits are not part of the address");
    assert_eq!(&main[512..768], &main[256..512], "the block landed in page 2");
    assert_eq!(d.read(reg::MEMORY_ADDRESS), 2 * 256 + 255);
}

// --- Read All and Write All -------------------------------------------------

/// A command run on the controller with a command list of `pages` CCWs
/// starting at `page`, the disk address `da`, and the timer advanced past
/// [`TIMEOUT_NS`] so that a command which hangs has shown it.
fn run(d: &mut Controller, main: &mut [u32], cmd: u32, da: u32, page: u32, pages: u32) {
    for k in 0..pages {
        main[CLP as usize + k as usize] = (page + k) << 8 | u32::from(k + 1 < pages);
    }
    d.write(reg::COMMAND, cmd, main);
    d.write(reg::CLP, CLP, main);
    d.write(reg::DISK_ADDRESS, da, main);
    d.write(reg::START, 0, main);
    d.advance(TIMEOUT_NS * 2);
}

/// **Read All hands over the track's own bytes, and does not time out.**
///
/// "0002 Read All.  Reads all bits of the disk starting at the specified
/// rotational position."  So what lands in memory is the format ---
/// preamble, sync, header, checkword, relock, sync, pad, data, checkword,
/// postamble --- and `disk_unit::parse_sector` reads it back off its bits
/// as the controller's own receiver would.  Before this the model had no
/// track format and answered the command with a timeout error, which is
/// `STATUS<11>`, a hardware fault that had not happened: issue #8.
#[test]
fn read_all_hands_over_the_tracks_own_bytes() {
    use muir::disk_unit::{format, parse_sector};
    let Some((mut d, mut main)) = loaded() else { return };
    // The first block of the pack, whose data an ordinary Read gives too.
    let mut ordinary = vec![0u32; 1 << 16];
    let Some((mut e, _)) = loaded() else { return };
    read_block(&mut e, &mut ordinary, 0, 1);
    let block0: Vec<u32> = ordinary[0o400..0o400 + BLOCK_WORDS].to_vec();

    // Read All from the same rotational position, a track's worth of pages.
    let pages = format::SECTOR.div_ceil(BLOCK_WORDS * 4) as u32 + 1;
    run(&mut d, &mut main, 0o02, 0, 16, pages);
    assert_eq!(d.status() & status::TIMEOUT, 0, "no timeout: {:o}", d.status());
    assert_eq!(d.status() & status::NOT_ACTIVE, status::NOT_ACTIVE, "and the transfer is over");

    // The bytes of the first sector, low-order byte first, parsed back.
    let words = &main[0o10000..0o10000 + pages as usize * BLOCK_WORDS];
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let bits: Vec<bool> =
        bytes.iter().flat_map(|&b| (0..8).map(move |k| b >> k & 1 != 0)).collect();
    let s = parse_sector(&bits).expect("a sector in what Read All handed over");
    assert!(s.header_checks, "the header checkword");
    assert!(s.data_checks, "the data checkword");
    assert_eq!(s.header & 0x0fff_ffff, 0, "cylinder 0, head 0, block 0");
    assert_eq!(s.data.to_vec(), block0, "the block an ordinary Read gives");
}

/// **What Write All lays down, an ordinary Read reads back.**  That is
/// what the command is for: "The format is determined by the program that
/// uses the Write All operation to format the disk."  A track image built
/// in memory, written with 0013, and then the ordinary Read of a block in
/// it gives the data the image carried.
#[test]
fn write_all_formats_a_track_an_ordinary_read_can_read() {
    use muir::disk_unit::{format, sector_image};
    let mut d = Controller::default();
    d.attach(0, Unit::blank(Geometry::T300));
    let mut main = vec![0u32; 1 << 16];

    // Two sectors of a track, each with its own data, laid out as bytes and
    // packed into memory low-order byte first.
    let g = Geometry::T300;
    let data = |n: u32| std::array::from_fn::<u32, BLOCK_WORDS, _>(|i| n * 0x10000 + i as u32);
    let mut bytes = Vec::new();
    for block in 0..2u32 {
        bytes.extend(sector_image(&g, 0, 0, block, &data(block + 1)));
    }
    assert_eq!(bytes.len(), 2 * format::SECTOR);
    let words: Vec<u32> = bytes.as_chunks::<4>().0.iter().map(|b| u32::from_le_bytes(*b)).collect();
    let pages = words.len().div_ceil(BLOCK_WORDS);
    main[0o10000..0o10000 + words.len()].copy_from_slice(&words);

    run(&mut d, &mut main, 0o13, 0, 16, pages as u32);
    assert_eq!(d.status() & status::TIMEOUT, 0, "no timeout: {:o}", d.status());

    // And the ordinary Read of each block gives what the image carried.
    for block in 0..2u32 {
        let mut back = vec![0u32; 1 << 16];
        read_block(&mut d, &mut back, block, 1);
        assert_eq!(
            back[0o400..0o400 + BLOCK_WORDS],
            data(block + 1),
            "block {block} read back after Write All"
        );
    }
}

/// **Which commands end in a timeout, measured on the netlist board.**
///
/// A timeout is `STATUS<11>`, "a disk operation took longer than 2.5
/// seconds", a hardware fault the software cannot tell from a failing
/// drive, so a model that raises it where the board does not is lying.
/// `cadrdc/newdsk.31` has the command PROM "divided into 8 sectors of 64
/// words each", so the sector is `<2:0>` alone and `<3>` only steers the
/// memory channel; every code below lands in a sector the listing fills
/// except sector 7, which it leaves empty at 700-777.
///
/// Two do time out, and only one of them was expected. Sector 7 starts
/// the sequencer in unwritten PROM, and `sys/doc/disk.text` says it "will
/// currently hang the controller, causing a timeout error". `0012` --- a
/// Read All entered with the channel reversed --- turns out to hang as
/// well: it reads eighteen words out of memory, stores nothing, and the
/// board's watchdog stops it. That is measured, not read off the
/// listing: `the_reversed_memory_channel_is_measured` in
/// `tests/cadrdc_netlist.rs`. An earlier version of this test asserted
/// `0012` completed, on the reasoning that the sector is filled so the
/// board must run it; the board runs it and it does not finish.
#[test]
fn only_sector_seven_and_the_reversed_read_all_time_out() {
    for cmd in [0o00, 0o01, 0o02, 0o03, 0o10, 0o11, 0o13, 0o04, 0o14, 0o05, 0o15, 0o06, 0o16] {
        let Some((mut d, mut main)) = loaded() else { return };
        run(&mut d, &mut main, cmd, 0, 8, 1);
        assert_eq!(d.status() & status::TIMEOUT, 0, "{cmd:o} timed out: {:o}", d.status());
    }
    for cmd in [0o07, 0o17, 0o12] {
        let Some((mut d, mut main)) = loaded() else { return };
        run(&mut d, &mut main, cmd, 0, 8, 1);
        assert_ne!(d.status() & status::TIMEOUT, 0, "{cmd:o} hangs the sequencer");
        assert_ne!(d.status() & status::ABORTED, 0, "{cmd:o}: and the transfer is aborted");
    }
}

/// **The reversed memory channel, as the board answers it.** `0001`,
/// `0003` and `0012` are the Write, Write All and Read All sectors with
/// `<3>` turning the channel round. All three were measured against the
/// netlist controller, which has the fifo this model has not
/// (`the_reversed_memory_channel_is_measured`), and the three do three
/// different things: `0001` ends clean, `0003` ends with Overrun and
/// Transfer Aborted, and `0012` hangs to the watchdog.
///
/// What is held here is the status, which is what software reads. The
/// data is not: the board disturbs the page and, for `0001`, the pack,
/// with the shift registers' own contents, and there is no fifo here to
/// produce them. Issue #34 records the measurement.
#[test]
fn the_reversed_channel_answers_as_the_board_does() {
    let Some((mut d, mut main)) = loaded() else { return };
    run(&mut d, &mut main, 0o01, 0, 8, 1);
    assert_eq!(
        d.status() & (status::TIMEOUT | status::OVERRUN | status::ABORTED | status::NXM),
        0,
        "0001 ends clean: {:o}",
        d.status()
    );

    let Some((mut d, mut main)) = loaded() else { return };
    run(&mut d, &mut main, 0o03, 0, 8, 1);
    assert_ne!(d.status() & status::OVERRUN, 0, "0003 overruns: {:o}", d.status());
    assert_ne!(d.status() & status::ABORTED, 0, "0003 aborts: {:o}", d.status());
    assert_eq!(d.status() & status::TIMEOUT, 0, "0003 does not time out: {:o}", d.status());

    // And the next command clears it, as every error is cleared by the
    // store into the command register.
    d.write(reg::COMMAND, 0, &mut main);
    assert_eq!(d.status() & status::OVERRUN, 0, "the next command clears it");
}

// --- the drive's own time ---------------------------------------------------

/// **A seek takes as long as the heads take, and the controller is busy for
/// it.** MIT: "0004 Seek. Initiates a seek to the cylinder specified in the
/// disk address register."
///
/// Century Data's figures for the T-300 are 6 ms to the next cylinder and 55
/// ms across the full 814, which is what `disk_unit::seek_ns` interpolates,
/// so the length of a seek is the distance travelled and not a constant. The
/// controller reports `STATUS<0>` clear for the whole of it --- that is the
/// thing a driver waits on.
#[test]
fn a_seek_is_as_long_as_the_heads_take() {
    use muir::disk_unit::seek_ns;
    let (mut d, mut main) = blank();
    d.timed = true;
    let mut now = 1_000;
    for (cylinder, want) in [(1u32, seek_ns(1)), (814, seek_ns(813)), (0, seek_ns(814))] {
        d.advance(now);
        d.write(reg::COMMAND, 0o4, &mut main);
        d.write(reg::DISK_ADDRESS, cylinder << 16, &mut main);
        d.write(reg::START, 0, &mut main);
        assert_eq!(d.status() & status::NOT_ACTIVE, 0, "to {cylinder}: busy at the store");
        d.advance(now + want - 1);
        assert_eq!(d.status() & status::NOT_ACTIVE, 0, "to {cylinder}: still busy a ns before");
        d.advance(now + want);
        assert_ne!(d.status() & status::NOT_ACTIVE, 0, "to {cylinder}: done after {want} ns");
        now += want + 1_000;
    }
    // Six milliseconds to the next cylinder and fifty-five across the pack,
    // which is where the two ends of the interpolation are pinned.
    assert_eq!(seek_ns(1), 6_000_000, "one cylinder");
    assert_eq!(seek_ns(814), 55_000_323, "the full stroke, to a rounding of the per-cylinder step");
    assert_eq!(seek_ns(0), 0, "and the heads already there have no move to make");
}

/// **A transfer waits for its block to come round, and then moves it.**
///
/// A T-300 turns once in 16.67 ms and lays seventeen sectors on a track, so
/// what a read costs is where the block is when the operation starts. Two
/// reads of the same block a known part of a turn apart differ by exactly
/// that part, and both take a sector's time to move the block once it is
/// under the head.
#[test]
fn a_transfer_waits_for_the_block_to_come_round() {
    use muir::disk_unit::{REVOLUTION_NS, SECTOR_NS};
    let done_at = |start: u64, block: u32| {
        let (mut d, mut main) = blank();
        d.timed = true;
        d.advance(start);
        read_block(&mut d, &mut main, block, 2);
        assert_eq!(d.status() & status::NOT_ACTIVE, 0, "busy at the store");
        // The one instant it becomes ready, found by asking either side of
        // it rather than by trusting an arithmetic of the test's own.
        let mut lo = start;
        let mut hi = start + 2 * REVOLUTION_NS;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            d.advance(mid);
            if d.status() & status::NOT_ACTIVE == 0 { lo = mid + 1 } else { hi = mid }
        }
        lo - start
    };
    // Block 0 is under the head at the index, so a read there waits for
    // nothing and takes the sector alone.
    assert_eq!(done_at(0, 0), SECTOR_NS, "block 0 at the index: no wait, one sector");
    // A third of a turn later it has to come round again, which is the rest
    // of the revolution.
    let third = REVOLUTION_NS / 3;
    assert_eq!(
        done_at(third, 0),
        REVOLUTION_NS - third + SECTOR_NS,
        "a third of a turn in, block 0 is most of a revolution away"
    );
    // And a block just ahead of the head waits least. A third of a turn is
    // 5.7 sectors in, so block 6 is the next to come round and block 5 has
    // only just gone by --- which is nearly a whole revolution away again.
    assert!(done_at(third, 6) < done_at(third, 0), "the next block round is the shortest wait");
    assert!(done_at(third, 5) > done_at(third, 6), "and the one just missed is the longest");
}

/// **A driver's wait loop goes round, which is the whole point.** Software
/// that stores into START and then polls `STATUS<0>` is what MIT's microcode
/// does in `DISK-WAIT`; before the drive had a clock the first read already
/// said done, so a loop with a bug in it would have run once and looked
/// right.
#[test]
fn a_poll_for_completion_goes_round_more_than_once() {
    let (mut d, mut main) = blank();
    d.timed = true;
    d.advance(1_000);
    read_block(&mut d, &mut main, 3, 2);
    // A microcycle a turn of the loop, which is what a microcode poll costs.
    let mut now = 1_000;
    let mut turns = 0u64;
    while d.status() & status::NOT_ACTIVE == 0 {
        now += 145;
        d.advance(now);
        turns += 1;
        assert!(turns < 1_000_000, "the transfer never finished");
    }
    assert!(turns > 1_000, "the loop spun while the disk worked: {turns} turns");

    // Off, which is how muir runs, the same read is done at the store and
    // the loop is never entered. That is a deliberate default and not an
    // oversight: see `Controller::timed`.
    let (mut d, mut main) = blank();
    d.advance(1_000);
    read_block(&mut d, &mut main, 3, 2);
    assert_ne!(d.status() & status::NOT_ACTIVE, 0, "off, the done comes with the words");
}

/// **Running off the end of the pack is Header ECC, not silence and not a
/// drive fault.**
///
/// `sys/doc/disk.text` on `<17>`: "Header ECC Error. Indicates that the
/// error-correcting code for a header failed ... Header ECC Error **also
/// happens if an attempt is made to continue a read or write operation
/// past the end of the disk**." So the second cause needs no headers on
/// the pack, and it is the one this model can have: the first wants a pack
/// that carries them, which is issue 51.
///
/// **"Continue" is the load-bearing word.** A transfer whose *first*
/// address is off the pack never gets that far: `Unit::seek` refuses it
/// and the board reports a seek error, `<10>` --- "the selected unit is
/// reporting failure of a seek operation" --- which is what this model
/// already did. `<17>` is the other case, a transfer stepping off the
/// last block with more list to go, and that used to end **saying nothing
/// at all**.
#[test]
fn running_off_the_end_of_the_pack_is_header_ecc() {
    let Some((mut d, mut main)) = loaded() else { return };
    let last = Geometry::T300.blocks() - 1;
    let da = |lba: u32| {
        let g = Geometry::T300;
        let (c, rest) = (lba / g.blocks_per_cylinder(), lba % g.blocks_per_cylinder());
        (c << 16) | ((rest / g.blocks_per_track) << 8) | (rest % g.blocks_per_track)
    };

    // The last block on its own is an ordinary read: the transfer ends
    // because the list ends, not because the pack does.
    read_block(&mut d, &mut main, da(last), 2);
    assert_eq!(d.read(reg::STATUS) & (1 << 17), 0, "the last block reads");
    assert_eq!(d.read(reg::STATUS) & (1 << 6), 0, "and the drive is not at fault");

    // Two CCWs from the last block: the second wants the block after it,
    // and there is none. `<17>`, and `<13>` with it because the error
    // stops the transfer.
    main[CLP as usize] = (2 << 8) | 1;
    main[CLP as usize + 1] = 3 << 8;
    d.write(reg::COMMAND, 0, &mut main);
    d.write(reg::CLP, CLP, &mut main);
    d.write(reg::DISK_ADDRESS, da(last), &mut main);
    d.write(reg::START, 0, &mut main);
    let s = d.read(reg::STATUS);
    assert_ne!(s & (1 << 17), 0, "past the end of the disk: {s:o}");
    assert_ne!(s & (1 << 13), 0, "and it stops the transfer: {s:o}");
    assert_eq!(s & (1 << 6), 0, "the drive is not at fault: {s:o}");

    // And a transfer that *starts* past the end is the other error: MIT
    // says "continue", and a first address off the pack is a seek the
    // drive refuses.
    read_block(&mut d, &mut main, da(last) + 1, 2);
    let s = d.read(reg::STATUS);
    assert_ne!(s & (1 << 10), 0, "starting past the end is a seek error: {s:o}");
    assert_eq!(s & (1 << 17), 0, "and not header ECC: {s:o}");
    assert_eq!(s & (1 << 6), 0, "still not the drive's fault: {s:o}");
    d.write(reg::COMMAND, 0o1005, &mut main); // recalibrate clears <10>

    // The next command clears it: `-RESET ERR` on every store into the
    // command register.
    read_block(&mut d, &mut main, 0, 2);
    assert_eq!(d.read(reg::STATUS) & (1 << 17), 0, "the next command clears it");
}
