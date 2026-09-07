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
use support::vendor;

/// Status bits, by MIT's numbering.  `DCSTS` drives each of these onto the
/// Xbus line of the same number.
mod status {
    pub const READ_COMPARE_DIFFERENCE: u32 = 1 << 22;
    pub const TIMEOUT: u32 = 1 << 11;
    pub const NXM: u32 = 1 << 20;
    pub const SEEK_ERROR: u32 = 1 << 10;
    pub const NOT_ON_LINE: u32 = 1 << 9;
    pub const FAULT: u32 = 1 << 6;
    pub const INTERRUPT_REQUEST: u32 = 1 << 3;
    pub const ATTENTION: u32 = 1 << 2;
    pub const ANY_ATTENTION: u32 = 1 << 1;
    pub const NOT_ACTIVE: u32 = 1;
    /// What `AWAIT-DRIVE-READY` in the boot PROM demands be clear: "Bits
    /// 4,5,6,8,9,10", which it builds as `A-3560`.
    pub const DRIVE_NOT_READY: u32 = 0o3560;
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
    vendor(&["run", "disk-sys-100-0.img"])
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
    assert_eq!(d.read(reg::STATUS), status::NOT_ACTIVE | status::NOT_ON_LINE);
}

/// **A transfer takes the disk's time when a run asks for it.** With
/// `access_ns` set, a read started at one instant leaves the controller
/// active, with no done interrupt, until that many nanoseconds later; the
/// words are in memory from the start, as before. With it zero the
/// controller is never active, as every run has had it.
#[test]
fn a_transfer_takes_the_access_time_when_asked() {
    let Some((mut d, mut main)) = loaded() else { return };
    d.access_ns = 38_300_000;
    d.advance(1_000);
    d.write(reg::COMMAND, 1 << 11, &mut main); // read, done interrupt enabled
    d.write(reg::CLP, CLP, &mut main);
    main[CLP as usize] = 2 << 8;
    d.write(reg::DISK_ADDRESS, 0, &mut main);
    d.write(reg::START, 0, &mut main);
    assert_eq!(d.status() & status::NOT_ACTIVE, 0, "active while the transfer runs");
    assert!(!d.interrupt(), "no done while active");
    d.advance(1_000 + 38_299_999);
    assert_eq!(d.status() & status::NOT_ACTIVE, 0, "still active a nanosecond short");
    d.advance(1_000 + 38_300_000);
    assert_ne!(d.status() & status::NOT_ACTIVE, 0, "done at the access time");
    assert!(d.interrupt(), "and the done interrupt with it");
    assert_ne!(main[2 << 8], 0, "the words were there from the start");
}

/// The four registers are where MIT says they are, and reach the controller
/// through the machine's bus.  "These are normally at physical addresses
/// 17377774-17377777, which is just below the Unibus."
#[test]
fn the_registers_are_just_below_the_unibus() {
    let mut m = Machine::new();
    assert_eq!(REGS, 0o17377774);
    // Status, with no drive.
    assert_eq!(m.bus_read(0o17377774), status::NOT_ACTIVE | status::NOT_ON_LINE);
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
    BIT_NS, ControllerLines, Ecc, INDEX_PULSE_NS, REVOLUTION_NS, SECTOR_PULSE_NS, Tag, Trident,
    format, parse_sector, sector_image,
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

/// **The drive pulses seventeen times a turn, the index pulse the long one**,
/// and the two widths fall on either side of the controller's one-shot.
#[test]
fn the_drive_pulses_seventeen_times_a_turn_with_one_long_one() {
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
    assert_eq!(pulses.len(), 18, "seventeen, and the next turn's index");
    assert_eq!(pulses[0], (1_000, INDEX_PULSE_NS), "the index at power-on");
    assert_eq!(pulses[17].1, INDEX_PULSE_NS, "and one revolution later");
    assert!(pulses[1..17].iter().all(|&(_, w)| w == SECTOR_PULSE_NS));
    let spacing: Vec<u64> = pulses.windows(2).map(|w| w[1].0 - w[0].0).collect();
    assert!(spacing.iter().all(|&s| s.abs_diff(REVOLUTION_NS / 17) <= 1), "{spacing:?}");
    assert_eq!(pulses[17].0 - pulses[0].0, REVOLUTION_NS);
    // The one-shot at DCTRID 0B09 is 2,250 ns, by the drawing; a sector's
    // bits fit between its pulse and the next; the clock's halves are equal.
    const {
        assert!(SECTOR_PULSE_NS < 2_250 && 2_250 < INDEX_PULSE_NS);
        assert!(format::SECTOR as u64 * 8 * BIT_NS < REVOLUTION_NS / 17);
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
    let began = sector_begins(&d, first + REVOLUTION_NS / 17, 6);
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

/// **Reset takes effect in the store to the command register.** MIT:
/// "0016 Reset. This stops the current transfer and resets the controller.
/// This command takes effect as soon as it is stored in the command
/// register; no store in START is required. After storing a Reset command
/// you should store 0 in the command register to turn off the reset
/// condition."
#[test]
fn a_reset_stored_in_the_command_register_takes_effect_at_once() {
    let (mut d, mut main) = blank();
    d.access_ns = 38_300_000;
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
/// They did drift. This engine took `sys/doc/disk.text`'s 2.5 seconds and
/// the netlist board ran at 1.536, and nothing in the code said which muir
/// meant. Which of MIT's two figures is right is still open --- see
/// [`TIMEOUT_NS`]'s own comment --- but they now disagree in one place
/// instead of two.
#[test]
fn both_engines_take_the_timeout_from_the_same_two_facts() {
    let (period, over) = muir::chip::DISK_TIMEOUT_VCO_PERIOD;
    assert_eq!(over, 1, "the disk timeout clock's period is a whole number of ns");
    assert_eq!(period, 12_000_000, "`dctmot.drw`'s own property on the body: 12 ms");
    assert_eq!(muir::disk_controller::TIMEOUT_DIVIDER, 128, "the 74393 at 0C03 divides by 128");
    assert_eq!(TIMEOUT_NS, period * 128, "the model times out on the board's own count");
    // MIT wrote the answer on the same sheet: `|TIMEOUT    ;1.5 SEC`.
    assert_eq!(TIMEOUT_NS, 1_536_000_000, "1.536 s, which is the drawing's `;1.5 SEC`");
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

/// **The track commands end with a timeout error, and the simulator goes
/// on.** Read All and Write All move a track's raw bits, headers and all,
/// and 01, 03 and 12 enter the Write, Write All and Read All sectors with
/// the memory channel turned round; this model has no track format for
/// any of them, where the netlist controller and its drive have. Each is
/// answered with the timeout error at once, the words untouched, and the
/// next transfer clears it as it clears the other errors.
#[test]
fn the_track_commands_end_with_a_timeout_error() {
    for cmd in [0o02u32, 0o13, 0o01, 0o03, 0o12] {
        let (mut d, mut main) = blank();
        main[CLP as usize] = 2 << 8;
        main[2 << 8] = 0o525252;
        d.write(reg::COMMAND, cmd, &mut main);
        d.write(reg::CLP, CLP, &mut main);
        d.write(reg::DISK_ADDRESS, 0, &mut main);
        d.write(reg::START, 0, &mut main);
        let s = d.status();
        assert_ne!(s & status::TIMEOUT, 0, "command {cmd:o}: the timeout error");
        assert_ne!(s & status::NOT_ACTIVE, 0, "command {cmd:o}: ready again");
        assert_eq!(main[2 << 8], 0o525252, "command {cmd:o}: no words moved");
        d.write(reg::COMMAND, 0, &mut main);
        d.write(reg::START, 0, &mut main);
        assert_eq!(d.status() & status::TIMEOUT, 0, "command {cmd:o}: the next transfer clears it");
    }
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
