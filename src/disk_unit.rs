// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! One Trident drive: a pack, a head position, and the conditions the
//! controller reports for whichever unit is selected --- and the drive as
//! its cables see it, [`Trident`], for the controller that is a netlist.
//!
//! Split from [`crate::disk_controller`] because they are two things: the
//! controller is one board on the Xbus, and a unit is a drive cabinet on the
//! end of its cable, of which there may be eight. MIT's documentation keeps
//! them apart --- most of `sys/doc/disk.text` is the controller's registers,
//! and the drive is what those registers are *about* --- and so does this,
//! in `crate::disk_controller` and here.
//!
//! The sources are the ones `crate::disk_controller` names.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::chip::Chip;
use crate::netlist::{NetId, Netlist};
use crate::part::Level;

/// One Lisp machine page: "Each disk block contains one Lisp Machine page
/// worth of data, i.e. 256. words or 1024. bytes."
pub const BLOCK_WORDS: usize = 256;

/// A drive's geometry.  The two types the controller was used with, and
/// MIT's documentation gives the same numbers: "A T-80 has 815. cylinders,
/// each with 5 heads (tracks), each with 16. or 17. blocks depending on how
/// you feel like formatting it.  A T-300 has 19. heads."
///
/// `tests/disk.rs` checks the T-300 numbers against the disk's own label,
/// which is a third source again: the pack says how it was formatted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Geometry {
    pub cylinders: u32,
    pub heads: u32,
    pub blocks_per_track: u32,
}

impl Geometry {
    pub const T80: Geometry = Geometry { cylinders: 815, heads: 5, blocks_per_track: 17 };
    pub const T300: Geometry = Geometry { cylinders: 815, heads: 19, blocks_per_track: 17 };

    pub fn blocks_per_cylinder(&self) -> u32 {
        self.heads * self.blocks_per_track
    }

    pub fn blocks(&self) -> u32 {
        self.cylinders * self.blocks_per_cylinder()
    }
}

/// One drive: a pack, a head position, and the three conditions the status
/// register reports for whichever unit is selected.
pub struct Unit {
    pub geometry: Geometry,
    /// The pack's file. `None` is a blank pack, [`Unit::blank`], every
    /// block zero until written.
    image: Option<File>,
    /// Whether a written block goes to the file. [`Unit::open_rw`] writes
    /// through; [`Unit::open`] does not, and keeps the block
    /// in [`Unit::written`] instead, which is what the tests want of the
    /// vendored pack --- fetched material they must leave as fetched.
    writable: bool,
    written: HashMap<u32, [u32; BLOCK_WORDS]>,
    /// MIT: "the read-only switch only applies when the drive is not
    /// selected".  Nothing models the switch; a pack opened here is writable.
    pub read_only: bool,
    cylinder: u32,
    head: u32,
    block: u32,
    /// `STATUS<10>`, "Selected Unit Seek Error".
    pub seek_error: bool,
    /// `STATUS<6>`, "Selected Unit Fault".
    pub fault: bool,
    /// `STATUS<2>`, "Selected Unit Attention.  Reset using the At Ease
    /// command."
    pub attention: bool,
}

/// A second handle on the same read-only image, and the written blocks and
/// the drive's state copied: what a resumed `chip` comparison needs to give
/// the far end of its cables the drive `rtl` has been running.
impl Clone for Unit {
    fn clone(&self) -> Self {
        Unit {
            geometry: self.geometry,
            image: self.image.as_ref().map(|f| f.try_clone().expect("a second handle on the pack")),
            writable: self.writable,
            written: self.written.clone(),
            read_only: self.read_only,
            cylinder: self.cylinder,
            head: self.head,
            block: self.block,
            seek_error: self.seek_error,
            fault: self.fault,
            attention: self.attention,
        }
    }
}

impl Unit {
    /// Opens a pack read-only: the file is never written, and a written
    /// block is kept in memory for the run. The file must be exactly the
    /// size the geometry implies --- "disk pack
    /// size is not the expected one" --- and is what ties the geometry to
    /// the image.
    pub fn open(path: impl AsRef<Path>, geometry: Geometry) -> std::io::Result<Unit> {
        Self::open_with(path, geometry, false)
    }

    /// Opens a pack read-write: a written block goes to the file, as it
    /// goes to the pack in a real drive.
    pub fn open_rw(path: impl AsRef<Path>, geometry: Geometry) -> std::io::Result<Unit> {
        Self::open_with(path, geometry, true)
    }

    fn open_with(
        path: impl AsRef<Path>,
        geometry: Geometry,
        writable: bool,
    ) -> std::io::Result<Unit> {
        let image = OpenOptions::new().read(true).write(writable).open(path.as_ref())?;
        let want = geometry.blocks() as u64 * BLOCK_WORDS as u64 * 4;
        let got = image.metadata()?.len();
        if got != want {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{}: {got} bytes, expected {want}", path.as_ref().display()),
            ));
        }
        Ok(Unit { image: Some(image), writable, ..Unit::blank(geometry) })
    }

    /// Whether a written block goes to the file.
    pub fn writable(&self) -> bool {
        self.writable
    }

    /// A pack with nothing on it: every block reads as zeros until it is
    /// written. What a test formats and reads back without touching
    /// `vendor/`.
    pub fn blank(geometry: Geometry) -> Unit {
        Unit {
            geometry,
            image: None,
            writable: false,
            written: HashMap::new(),
            read_only: false,
            cylinder: 0,
            head: 0,
            block: 0,
            seek_error: false,
            fault: false,
            attention: false,
        }
    }

    /// Where the heads are: cylinder, head, block.
    pub fn position(&self) -> (u32, u32, u32) {
        (self.cylinder, self.head, self.block)
    }

    /// A block number from the start of the pack, or `None` off it.
    fn lba_of(&self, cylinder: u32, head: u32, block: u32) -> Option<u32> {
        let g = &self.geometry;
        (cylinder < g.cylinders && head < g.heads && block < g.blocks_per_track)
            .then(|| cylinder * g.blocks_per_cylinder() + head * g.blocks_per_track + block)
    }

    /// One block by address, without moving the heads: what a drive reads
    /// off the pack at a sector. `None` off the pack, or for a block the
    /// image could not deliver --- the file gone, truncated or unreadable
    /// --- which the controller reports as a drive fault either way.
    pub fn block_at(&mut self, cylinder: u32, head: u32, block: u32) -> Option<[u32; BLOCK_WORDS]> {
        let lba = self.lba_of(cylinder, head, block)?;
        if let Some(b) = self.written.get(&lba) {
            return Some(*b);
        }
        self.file_block(lba)
    }

    /// One block off the image by number: zeros off a blank pack, `None`
    /// for a block the image could not deliver.
    fn file_block(&mut self, lba: u32) -> Option<[u32; BLOCK_WORDS]> {
        let mut buf = [0u32; BLOCK_WORDS];
        let Some(image) = self.image.as_mut() else { return Some(buf) };
        let mut bytes = [0u8; BLOCK_WORDS * 4];
        if let Err(e) = image
            .seek(SeekFrom::Start(lba as u64 * bytes.len() as u64))
            .and_then(|_| image.read_exact(&mut bytes))
        {
            eprintln!("disk: reading block {lba} of the image: {e}");
            return None;
        }
        // The image holds each 32-bit word low byte first: word 0 of the
        // label reads as 0o11420440514, MIT's own `LABL`, only this way
        // round.  `tests/disk.rs` pins it.
        for (w, b) in buf.iter_mut().zip(bytes.as_chunks::<4>().0) {
            *w = u32::from_le_bytes(*b);
        }
        Some(buf)
    }

    /// One block by address, written: to the file on a pack opened
    /// read-write, else kept in memory. False off the pack, or for a
    /// block the image would not take, which is a drive fault.
    pub fn write_block_at(
        &mut self,
        cylinder: u32,
        head: u32,
        block: u32,
        data: &[u32; BLOCK_WORDS],
    ) -> bool {
        let Some(lba) = self.lba_of(cylinder, head, block) else { return false };
        match (self.writable, self.image.as_mut()) {
            (true, Some(image)) => {
                let mut bytes = [0u8; BLOCK_WORDS * 4];
                for (b, w) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(data) {
                    *b = w.to_le_bytes();
                }
                if let Err(e) = image
                    .seek(SeekFrom::Start(lba as u64 * bytes.len() as u64))
                    .and_then(|_| image.write_all(&bytes))
                {
                    eprintln!("disk: writing block {lba} of the image: {e}");
                    return false;
                }
                self.written.remove(&lba);
            }
            _ => {
                // A block written as the pack already holds it is not kept:
                // the file answers the read the same, and a checkpoint then
                // carries only what the run changed. The System 100 cold
                // boot writes twenty thousand blocks, nineteen in twenty of
                // them what was there.
                if self.file_block(lba) == Some(*data) {
                    self.written.remove(&lba);
                } else {
                    self.written.insert(lba, *data);
                }
            }
        }
        true
    }

    /// How many blocks the run has written that the pack does not hold.
    pub fn written_blocks(&self) -> usize {
        self.written.len()
    }

    /// The disk address register's view of this unit: `DCDA` reads the three
    /// counters back onto `XBO<27:0>` and the unit number onto `XBO<30:28>`,
    /// with `XBO31` grounded.
    pub fn da(&self, unit: u32) -> u32 {
        (unit & 7) << 28
            | (self.cylinder & 0o7777) << 16
            | (self.head & 0xff) << 8
            | (self.block & 0xff)
    }

    /// Moves the heads.  False means the address is off the pack, which is a
    /// seek error and stops the transfer.
    pub fn seek(&mut self, cylinder: u32, head: u32, block: u32) -> bool {
        if (cylinder, head, block) == (self.cylinder, self.head, self.block) {
            return true;
        }
        if cylinder >= self.geometry.cylinders
            || head >= self.geometry.heads
            || block >= self.geometry.blocks_per_track
        {
            self.seek_error = true;
            return false;
        }
        self.cylinder = cylinder;
        self.head = head;
        self.block = block;
        true
    }

    /// The next block in the transfer order the format defines: "0 following
    /// block on same track, 1 block 0 on next track (next head), 2 block 0 on
    /// head 0 of next cylinder".  `DCDA` does it in hardware, with `INC
    /// BLOCK`, `INC HEAD` and `INC CYL` on the counters and `CLR BLOCK` /
    /// `CLR HEAD` from the same signals.
    pub fn next_block(&mut self) -> bool {
        let (mut cylinder, mut head, mut block) = (self.cylinder, self.head, self.block + 1);
        if block == self.geometry.blocks_per_track {
            block = 0;
            head += 1;
            if head == self.geometry.heads {
                head = 0;
                cylinder += 1;
            }
        }
        self.seek(cylinder, head, block)
    }

    /// Reads the block under the heads.  False means past the end of the
    /// pack, which the controller reports as a drive fault.
    pub fn read_block(&mut self, buf: &mut [u32; BLOCK_WORDS]) -> bool {
        let (c, h, b) = self.position();
        match self.block_at(c, h, b) {
            Some(data) => {
                *buf = data;
                true
            }
            None => false,
        }
    }

    pub fn write_block(&mut self, buf: &[u32; BLOCK_WORDS]) -> bool {
        let (c, h, b) = self.position();
        self.write_block_at(c, h, b, buf)
    }
}

// ---------------------------------------------------------------------------
// The format on the pack
// ---------------------------------------------------------------------------

/// The sector as MIT's formatter lays it out, byte by byte: `sys/doc/disk.text`,
/// "The format of a block is". The ten fields add up to
/// [`format::SECTOR`], which is the sector length MIT set the drive's
/// jumpers to --- `dctrid.drw`, "Set sector length jumpers in drive to 1410
/// (octal) which is 1164. bytes" --- and `tests/disk.rs` holds the sum to
/// it. "Everything goes low-order bit first and low-order byte first."
pub mod format {
    /// "PREAMBLE - 53. bytes of ones."
    pub const PREAMBLE: usize = 53;
    /// "VFO LOCK - 8. bytes of ones."
    pub const VFO_LOCK: usize = 8;
    /// "SYNC - a byte containing octal 177": seven ones then a zero, low
    /// bit first, which is what the controller's `PREAMBLE DETECT` looks
    /// for --- `RSH0` high and `RSH7` low, the first zero after ones.
    pub const SYNC: u8 = 0o177;
    /// "HEADER - a 32-bit word": `<31:30>` the next block address code,
    /// `<27:16>` cylinder, `<15:8>` head, `<7:0>` block.
    pub const HEADER: usize = 4;
    /// "HEADER ECC - a 32-bit checkword."
    pub const ECC: usize = 4;
    /// "VFO RELOCK - 20. bytes of ones."
    pub const VFO_RELOCK: usize = 20;
    /// "PAD - a byte containing octal 377, which is here to fix a bug in
    /// the logic for read-compare. (Ugh)"
    pub const PAD: u8 = 0o377;
    /// "DATA - 1024. bytes of whatever you want."
    pub const DATA: usize = super::BLOCK_WORDS * 4;
    /// "POSTAMBLE - 44. bytes of ones."
    pub const POSTAMBLE: usize = 44;
    /// The sector, 1,164 bytes.
    pub const SECTOR: usize = 1164;
    /// Where the header's sync byte sits, from the sector pulse.
    pub const HEADER_SYNC_AT: usize = PREAMBLE + VFO_LOCK;
    /// Where the data's sync byte sits.
    pub const DATA_SYNC_AT: usize = HEADER_SYNC_AT + 1 + HEADER + ECC + VFO_RELOCK;
    /// Where the data starts, after the sync and the pad.
    pub const DATA_AT: usize = DATA_SYNC_AT + 2;

    /// "`<31:30>` next block address code: 0 following block on same track,
    /// 1 block 0 on next track (next head), 2 block 0 on head 0 of next
    /// cylinder, 3 end of disk."
    pub fn next_block_code(g: &super::Geometry, cylinder: u32, head: u32, block: u32) -> u32 {
        if block + 1 < g.blocks_per_track {
            0
        } else if head + 1 < g.heads {
            1
        } else if cylinder + 1 < g.cylinders {
            2
        } else {
            3
        }
    }
}

/// The controller's error-correcting code register, as DCECC wires it.
///
/// Thirty-two stages: `ECC.OUT` and `ECC1` to `ECC31`, four 74LS273s
/// clocked on `CLK.SR^`, every stage taking the one above it, `ECC31`
/// taking `ECC.IN`, and four stages taking the one above **exclusive-or
/// `ECC.IN`** through the 74LS86s at 0E27: `ECC29`, `ECC20`, `ECC10` and
/// `ECC8`. `ECC.IN` is the data bit exclusive-or `ECC.OUT`, gated by `ECC
/// FEEDBACK ENABLE`, which is every `ECC/` value but `NO FEEDBACK`.
///
/// So it divides by `x^32 + x^23 + x^21 + x^11 + x^2 + 1`, counting the
/// entry stage as degree 0 and `ECC.OUT` as 31. `disk.text` gives
/// `x^31+x^29+x^20+x^10+x^8+1` "if I understand this logic correctly",
/// which is the drawing's stage numbers read as exponents with `ECC.OUT`
/// left out --- one degree short, and mirrored. What
/// matters is not the name of the polynomial but that this register and
/// the board's agree bit for bit, and `tests/cadrdc_netlist.rs` reads a
/// sector serialised by this one through the board's.
///
/// The checkword is what the controller writes with `WRITE/ECC,ECC/NO
/// FEEDBACK`: the register shifted out with nothing coming in, `ECC.OUT`
/// first. Fed back in with feedback on, it leaves the register at zero,
/// which is what `TEST ECC HDR` and `JUMP/ECC` look for. The board's
/// `-ECC=ZERO`, the S133 at 0C27, reads `ECC11` to `ECC31` only, which is
/// consistent with the test being made in the last bit period of the
/// checkword's last byte, before the edge that takes the last bit, when
/// those are the stages already zero; [`Ecc::checks`] tests the whole
/// register after the last bit. What the tests enforce is the thing that
/// matters: a checkword made here passes the board's test, header and
/// data, on a read the netlist runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Ecc(u32);

impl Ecc {
    /// One `CLK.SR^`: `data` is `DISK DATA`, `feedback` is `ECC FEEDBACK
    /// ENABLE`.
    pub fn shift(&mut self, data: bool, feedback: bool) {
        let input = feedback && (data ^ self.out());
        let taps = if input { 1 << 31 | 1 << 29 | 1 << 20 | 1 << 10 | 1 << 8 } else { 0 };
        self.0 = (self.0 >> 1) ^ taps;
    }

    /// `ECC.OUT`.
    pub fn out(&self) -> bool {
        self.0 & 1 != 0
    }

    /// Runs the code over bytes, low-order bit first, feedback on.
    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            for k in 0..8 {
                self.shift(b >> k & 1 != 0, true);
            }
        }
    }

    /// The checkword the controller writes after what it has fed: 32 bits
    /// out with no feedback, as four bytes low-order bit first.
    pub fn checkword(mut self) -> [u8; 4] {
        let mut out = [0u8; 4];
        for k in 0..32 {
            if self.out() {
                out[k / 8] |= 1 << (k % 8);
            }
            self.shift(false, false);
        }
        out
    }

    /// Whether what was fed ended in its own checkword.
    pub fn checks(&self) -> bool {
        self.0 == 0
    }

    /// The checkword of some bytes from a clear register.
    pub fn over(bytes: &[u8]) -> [u8; 4] {
        let mut e = Ecc::default();
        e.feed(bytes);
        e.checkword()
    }
}

/// A sector as it lies on the pack, [`format::SECTOR`] bytes: the block's
/// data and the format around it, both checkwords computed.
pub fn sector_image(
    g: &Geometry,
    cylinder: u32,
    head: u32,
    block: u32,
    data: &[u32; BLOCK_WORDS],
) -> Vec<u8> {
    use format::*;
    let mut s = Vec::with_capacity(SECTOR);
    s.resize(PREAMBLE + VFO_LOCK, 0xff);
    s.push(SYNC);
    let header = next_block_code(g, cylinder, head, block) << 30
        | (cylinder & 0xfff) << 16
        | (head & 0xff) << 8
        | (block & 0xff);
    let hb = header.to_le_bytes();
    s.extend_from_slice(&hb);
    s.extend_from_slice(&Ecc::over(&hb));
    s.resize(s.len() + VFO_RELOCK, 0xff);
    s.push(SYNC);
    s.push(PAD);
    let mut ecc = Ecc::default();
    for w in data {
        let b = w.to_le_bytes();
        s.extend_from_slice(&b);
        ecc.feed(&b);
    }
    s.extend_from_slice(&ecc.checkword());
    s.resize(s.len() + POSTAMBLE, 0xff);
    debug_assert_eq!(s.len(), SECTOR);
    s
}

/// What a sector's bits parse back to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Sector {
    pub header: u32,
    pub header_checks: bool,
    pub data: [u32; BLOCK_WORDS],
    pub data_checks: bool,
}

/// The first bit after a sync byte at or after `from`: a zero after at
/// least sixty-four ones, which is how the controller finds one too ---
/// its `PREAMBLE DETECT` fires on the first zero after ones, and the
/// preamble is never shorter than eight bytes of them.
fn after_sync(bits: &[bool], from: usize) -> Option<usize> {
    let mut ones = 0usize;
    for (k, &bit) in bits.iter().enumerate().skip(from) {
        if bit {
            ones += 1;
        } else if ones >= 64 {
            return Some(k + 1);
        } else {
            ones = 0;
        }
    }
    None
}

fn take_bits(bits: &[bool], at: usize, n: usize) -> Option<Vec<u8>> {
    let end = at.checked_add(n)?;
    if end > bits.len() {
        return None;
    }
    let mut out = vec![0u8; n / 8];
    for (k, &bit) in bits[at..end].iter().enumerate() {
        if bit {
            out[k / 8] |= 1 << (k % 8);
        }
    }
    Some(out)
}

/// Reads a sector back off its bits, as the controller would: the header
/// after the first sync, its checkword, the data after the second sync
/// and the pad, its checkword. `None` where a sync is not there to find.
pub fn parse_sector(bits: &[bool]) -> Option<Sector> {
    let at = after_sync(bits, 0)?;
    let hb = take_bits(bits, at, 32)?;
    let hc = take_bits(bits, at + 32, 32)?;
    let mut ecc = Ecc::default();
    ecc.feed(&hb);
    ecc.feed(&hc);
    let header_checks = ecc.checks();
    let header = u32::from_le_bytes(hb.try_into().unwrap());
    let at = after_sync(bits, at + 64)?;
    let db = take_bits(bits, at + 8, format::DATA * 8)?;
    let dc = take_bits(bits, at + 8 + format::DATA * 8, 32)?;
    let mut ecc = Ecc::default();
    ecc.feed(&db);
    ecc.feed(&dc);
    let mut data = [0u32; BLOCK_WORDS];
    for (w, b) in data.iter_mut().zip(db.as_chunks::<4>().0) {
        *w = u32::from_le_bytes(*b);
    }
    Some(Sector { header, header_checks, data, data_checks: ecc.checks() })
}

// ---------------------------------------------------------------------------
// The drive as its cables see it
// ---------------------------------------------------------------------------

/// One revolution at 3,600 rpm.
///
/// **Century Data's own figure.** The specification MIT calls "the Trident
/// manual" --- `sys/doc/disk.text` and AIM-528 both send the reader to it
/// and neither repeats its numbers --- is CalComp/Century Data 76205-902,
/// *Performance Specification, Models T-25, T-50, T-80, T-200 and T-300*,
/// November 1980, on bitsavers. Its Table 2-1, "Operational
/// Specifications", gives `Rotational speed 3600 RPM` for all five models
/// and `Average latency time 8.3ms`, which is half a revolution and agrees.
///
/// The same table gives `Bytes per track 20,160` for the T-300, which is
/// what `sys/doc/disk.text` means by "A track contains (approximately)
/// 20160. bytes (on a T-80 or a T-300)" --- MIT's approximation is Century
/// Data's exact figure.
pub const REVOLUTION_NS: u64 = 16_666_667;

/// One bit on the cable, at the T-300's 1,209 KByte/s.
///
/// **Century Data's own figure**, the same Table 2-1 as [`REVOLUTION_NS`]:
/// `I/O Transfer rate ... 1209 KByte` for the T-80 and T-300, 806 KByte for
/// the T-25, T-50 and T-200. That is 103.4 ns a bit, rounded up to an even
/// number here so the clock's two halves are equal.
///
/// The rounding costs more than the figures do. 16,666,667 / 104 is 160,256
/// bits, 20,032 bytes a track against Century Data's 20,160, which is 0.6
/// per cent low; the unrounded pair gives 20,150, which is 0.05 per cent
/// low. Nothing muir runs measures a track's length, so the even clock is
/// kept, and where it shows is the track's leftover. [`SECTOR_NS`] is
/// exact --- 1,164 bytes at this rate, 968 us --- so the seventeen sector
/// pulses take 16.46 ms of the revolution and the 203 us left over are 244
/// bytes where MIT's two figures, 20,160 a track and 19,788 in seventeen
/// sectors, leave 372. The leftover is short by the rounding; that there
/// is one, and that the seventeenth pulse falls before the index rather
/// than on it, is what [`turn`] takes from those figures.
pub const BIT_NS: u64 = 104;

/// The composite sector/index pulse widths, **Century Data's own**.
///
/// The *TRIDENT T25/T50/T80 OEM Reference Manual* on the composite
/// sector/index line: "The sector pulses are 1.24 +/- .24 uS wide and the
/// index pulses are 4 +/- 1 uS wide", and the separate sector line is "a
/// 1.24 +/- .24 us low going pulse at the beginning of each sector".
///
/// These were 1,230 and 5,500 ns, read off the waveforms on Al Kossow's
/// `tridentCables.pdf`, a hand transcription of 1999. The sector figure was
/// within Century Data's tolerance; **the index figure was outside it**,
/// 5.5 us against a range of 3 to 5. Nothing turned on it, because what the
/// controller needs is only that the two fall on either side of the
/// one-shot at DCTRID 0B09 --- it tells an index from a sector by holding
/// the block counter's clear until the long pulse ends --- and 1.24 and 4
/// still straddle that one-shot's 1,991 ns as 1.23 and 5.5 did. The
/// nominal figures are taken here and the tolerance is the manual's.
pub const SECTOR_PULSE_NS: u64 = 1_240;
pub const INDEX_PULSE_NS: u64 = 4_000;

/// One sector on the cable: `format::SECTOR` bytes at [`BIT_NS`].
///
/// **The spacing is the drive's sector length jumper, not a share of the
/// revolution.** `cadrdc/dctrid.drw` carries the jumper as a note on the
/// sheet, "Set sector length jumpers in drive to 1410 (octal) which is
/// 1164. bytes", and `sys/doc/disk.text` says what that gives: "Jumpers in
/// the disk are set to give 17. sector pulses per track, or one every 1164.
/// bytes, with a little left over at the end of the track". So the pulses
/// are a sector apart from the index and the track's tail is short, which
/// is where [`turn`]'s last region comes from.
pub const SECTOR_NS: u64 = format::SECTOR as u64 * 8 * BIT_NS;

/// When the pulse that begins region `k` falls, from the index: the one
/// definition of the boundary, so that the region under the head and the
/// time to the next one always agree.
fn began_of(k: u64) -> u64 {
    k * SECTOR_NS
}

/// Where a spindle of `sectors` sectors a revolution stands at `now`, its
/// index pulse beginning at `phase` modulo a revolution: the region under
/// the head and how far into it, in nanoseconds. The drive on the cable
/// turns by this, [`Trident::turn`], and so does the block counter of the
/// behavioural controller, `disk_controller::Controller`, which has no
/// drive on a cable to count pulses from.
///
/// **A revolution is `sectors` + 1 regions, because it is `sectors` + 1
/// pulses.** The index begins region 0 and each sector pulse begins the
/// next, so the seventeen sector pulses of [`SECTOR_NS`] end the seventeen
/// blocks and the last of them opens a region the index closes. That
/// region carries no data --- [`Unit::block_at`] has nothing at block
/// `sectors`, so the head reads ones there as it does off the pack --- and
/// it is short: [`BIT_NS`] makes it 203 us where MIT's figures make it 372
/// bytes.
///
/// MIT's own two texts settle it, and both say the index stands alone.
/// `sys/doc/disk.text` puts the tail of the track after the last sector
/// pulse rather than a pulse at the index: "17. sector pulses per track, or
/// one every 1164. bytes, with a little left over at the end of the track",
/// and 17 x 1,164 is 19,788 of the 20,160 a track holds. And
/// `sys/cc/dcheck.lisp` reads the block counter for half a second in
/// `DCHECK-BLOCK-COUNTER` and expects every value from 0 to 17 and no other
/// --- "Vandals: Yes, a value of 17. can appear here". A 17 needs the
/// counter clocked seventeen times between one index and the next, which an
/// index carrying a sector pulse of its own could not give: seventeen
/// pulses in all would stop the count at 16, which is what this drive did
/// while it spaced seventeen pulses evenly.
pub fn turn(sectors: u32, phase: u64, now: u64) -> (u32, u64) {
    let t = (now + REVOLUTION_NS - phase % REVOLUTION_NS) % REVOLUTION_NS;
    let k = (t / SECTOR_NS).min(sectors as u64);
    (k as u32, t - began_of(k))
}

/// How long the region after the pulse that begins region `k` lasts: a
/// sector, or, for the last, what is left of the revolution.
fn region_ns(sectors: u64, k: u64) -> u64 {
    if k >= sectors { REVOLUTION_NS - began_of(sectors) } else { SECTOR_NS }
}

/// The width of the pulse that begins a region: the index's at region 0, a
/// sector pulse's at every other. The index stands alone --- there is no
/// sector pulse under it --- which is what leaves the block counter a
/// seventeenth sector pulse to reach 17 on.
pub fn pulse_ns(sector: u32) -> u64 {
    if sector == 0 { INDEX_PULSE_NS } else { SECTOR_PULSE_NS }
}

/// What the controller has on the cable, as the drive reads it: `true`
/// is asserted, which on every one of these lines is low.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ControllerLines {
    /// `TRIDENT.0.SELECT/`.
    pub select: bool,
    /// `TRIDENT.CYL.TAG/`, `TRIDENT.HEAD.TAG/`, `TRIDENT.CONTROL.TAG/`.
    pub cylinder_tag: bool,
    pub head_tag: bool,
    pub control_tag: bool,
    /// `TRIDENT.BUSn/` as bit `n`, MIT's numbering: `data/trident-bus.txt`.
    pub bus: u16,
    /// The data pair while the controller's 75110 drives it, or `None`.
    pub write_data: Option<bool>,
}

/// What the drive puts on the cable: `true` is asserted, low on the
/// wire, except the two pairs, which are the logic level the 75107 reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DriveLines {
    /// `TRIDENT.READY/`, which the board reads as `SEL UNIT ON CYL`.
    pub on_cylinder: bool,
    /// `TRIDENT.ON.LINE/`.
    pub on_line: bool,
    /// `TRIDENT.READ.ONLY/`.
    pub read_only: bool,
    /// `TRIDENT.DEVICE.CHECK/`, the board's `SEL UNIT FAULT`.
    pub fault: bool,
    /// `TRIDENT.SEEK.INC/`.
    pub seek_incomplete: bool,
    /// `TRIDENT.0.SELECTED/`.
    pub selected: bool,
    /// `TRIDENT.0.ATTENTION/`.
    pub attention: bool,
    /// `TRIDENT.0.COMPSECIDX/`: low for the width of a sector or index pulse.
    pub sector_index: bool,
    /// The clock pair: `true` where `CLOCK.P` is above `CLOCK.M`.
    pub clock: bool,
    /// The data pair while the drive drives it: `true` where `DATA.P` is
    /// above `DATA.M`, which the 75107 reads as a one. `None` when the
    /// read amplifier is off.
    pub data: Option<bool>,
}

/// The tags the controller can send, for the record of what the drive saw.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tag {
    Cylinder(u32),
    Head(u32),
    Control(u16),
}

/// A sector's place on the pack: cylinder, head, sector.
type SectorKey = (u32, u32, u32);

/// The Trident as its cables see it: a spindle turning from power-on, a
/// bit clock, the tags and the gates, the status lines, the composite
/// pulse, and the sector under the head serialised in MIT's format.
///
/// The one thing here that is not the drive's own is [`Unit`], the pack:
/// it serves the model controller a block at a time and serves this a
/// sector at a time, so `micro` and `rtl` on one side and the netlist
/// board on the other read the same pack.
pub struct Trident {
    pub unit: Unit,
    /// A seek's time: settling plus so much a cylinder.
    ///
    /// **Century Data's own figures**, Table 2-1 of the performance
    /// specification cited at [`REVOLUTION_NS`]: `Single track positioning
    /// time 6ms`, `Average positioning time 30ms`, `Maximum positioning
    /// time 55ms`, the same for all five Trident models. Nothing MIT wrote
    /// gives them --- the controller waits on `ON CYLINDER` in a loop with
    /// no count (`cadrdc/newdsk.31` at 203, 303, 400), and the only bound
    /// on how long that may take is the controller's own watchdog.
    ///
    /// The two constants below are chosen so that this linear model hits
    /// the first and last of those exactly: one cylinder takes 6 ms and the
    /// full stroke of 814 takes 55 ms. **The average does not come out
    /// right and cannot**, because a real drive's seek goes as roughly the
    /// square root of the distance while this goes as the distance: the
    /// linear model gives about 22 ms where Century Data says 30. Two of
    /// three published figures exact is what a straight line can do, and
    /// before this it matched none of them.
    ///
    /// Public and settable, so a test that does not want to wait can zero
    /// them.
    pub seek_settle_ns: u64,
    pub seek_ns_per_cylinder: u64,
    /// The spindle's phase: when, modulo a revolution, an index pulse
    /// begins.
    phase: u64,
    cylinder: u32,
    head: u32,
    /// `HEAD TAG` bus 6 and 7: `SERVO OFFSET PLUS`, `SERVO OFFSET`.
    pub offset: (bool, bool),
    /// A seek in progress: where to, and when it is done.
    seek: Option<(u32, u64)>,
    attention: bool,
    fault: bool,
    seek_incomplete: bool,
    prev: ControllerLines,
    read_gate: bool,
    write_gate: bool,
    /// The sector under the head serialised, and which one it is.
    image: Option<(SectorKey, Vec<u8>)>,
    /// Bits the controller has written into the sector under the head,
    /// by bit from the sector's first clock, and which sector.
    written: Option<(SectorKey, Vec<Option<bool>>)>,
    /// The last [`TAG_CAP`] tags received, for a test to read back.
    pub tags: Vec<(u64, Tag)>,
    /// Sectors written that did not parse as the format.
    pub bad_writes: usize,
}

/// How many tags [`Trident::tags`] keeps; the oldest half goes when it is
/// full.
pub const TAG_CAP: usize = 4096;

impl Trident {
    fn tag(&mut self, now: u64, tag: Tag) {
        crate::keep_recent(&mut self.tags, TAG_CAP);
        self.tags.push((now, tag));
    }

    /// A drive up to speed at `now`, on line, at cylinder 0, an index
    /// pulse just beginning.
    pub fn new(unit: Unit, now: u64) -> Trident {
        Trident {
            unit,
            // 6 ms for one cylinder and 55 ms for the full 814: a
            // per-cylinder step of (55 - 6) / 813 ms, and the settle is
            // what is left of the 6.
            seek_settle_ns: 5_939_729,
            seek_ns_per_cylinder: 60_271,
            phase: now % REVOLUTION_NS,
            cylinder: 0,
            head: 0,
            offset: (false, false),
            seek: None,
            attention: false,
            fault: false,
            seek_incomplete: false,
            prev: ControllerLines::default(),
            read_gate: false,
            write_gate: false,
            image: None,
            written: None,
            tags: Vec::new(),
            bad_writes: 0,
        }
    }

    /// The heads: cylinder and head.
    pub fn position(&self) -> (u32, u32) {
        (self.cylinder, self.head)
    }

    /// Turns the spindle so that `sector` begins `in_ns` from `now`: a
    /// test that wants a block need not wait a revolution for it.
    pub fn spin_to(&mut self, now: u64, sector: u32, in_ns: u64) {
        let began = began_of(sector as u64);
        self.phase = (now + in_ns + REVOLUTION_NS - began % REVOLUTION_NS) % REVOLUTION_NS;
        debug_assert_eq!(self.turn(now + in_ns), (sector, 0));
    }

    /// Sectors a revolution. The regions are one more, [`turn`].
    fn sectors(&self) -> u64 {
        self.unit.geometry.blocks_per_track as u64
    }

    /// Where the spindle is at `now`: the sector under the head and how
    /// far into it, in nanoseconds. [`turn`], with this drive's phase.
    pub fn turn(&self, now: u64) -> (u32, u64) {
        turn(self.unit.geometry.blocks_per_track, self.phase, now)
    }

    /// When the sector under the head at `now` began. Signed: the spindle
    /// was turning before time zero, so a sector can have begun before it.
    fn sector_began(&self, now: u64) -> i64 {
        now as i64 - self.turn(now).1 as i64
    }

    /// Which bit of the sector under the head the clock is on at `now`,
    /// counting from the first clock rise at or after the sector pulse.
    fn bit_at(&self, now: u64) -> usize {
        let began = self.sector_began(now);
        let bit = BIT_NS as i64;
        let first = -(-began).div_euclid(bit) * bit;
        let now = now as i64;
        if now < first { usize::MAX } else { ((now - first) / BIT_NS as i64) as usize }
    }

    /// The clock pair at `now`: high for the first half of every bit.
    fn clock(now: u64) -> bool {
        now % BIT_NS < BIT_NS / 2
    }

    /// The sector under the head, serialised.
    fn image(&mut self, sector: u32) -> &[u8] {
        let key = (self.cylinder, self.head, sector);
        if self.image.as_ref().is_none_or(|(k, _)| *k != key) {
            let bytes = match self.unit.block_at(key.0, key.1, key.2) {
                Some(data) => sector_image(&self.unit.geometry, key.0, key.1, key.2, &data),
                // Off the pack there is nothing formatted: ones, and no
                // sync for the controller to find.
                None => vec![0xff; format::SECTOR],
            };
            self.image = Some((key, bytes));
        }
        &self.image.as_ref().unwrap().1
    }

    /// The bit the read amplifier puts out at `now`, the sector's bits
    /// and then ones through the gap.
    fn read_bit(&mut self, now: u64) -> bool {
        let (sector, _) = self.turn(now);
        let bit = self.bit_at(now);
        let image = self.image(sector);
        if bit < image.len() * 8 { image[bit / 8] >> (bit % 8) & 1 != 0 } else { true }
    }

    /// Finishes a seek whose time has come.
    fn settle(&mut self, now: u64) {
        if let Some((target, at)) = self.seek
            && now >= at
        {
            self.cylinder = target;
            self.seek = None;
            self.attention = true;
        }
    }

    /// Starts a seek to `target`, or reports it impossible. A seek to the
    /// cylinder the heads are on is complete at once: the heads do not
    /// move, on-cylinder holds, and attention says the seek is done.
    /// **Century Data's own manual says both halves of this are right.**
    /// The *TRIDENT T25/T50/T80 OEM Reference Manual* on bitsavers, in the
    /// interface chapter:
    ///
    /// - On the attention: it "will become active at the completion of a
    ///   'First Seek', 'Rezero', 'Seek', 'Seek Incomplete', or when an
    ///   emergency retract occurs". A cylinder tag for the cylinder the
    ///   heads are on is still a Seek and still completes, so it raises
    ///   attention. This does.
    /// - On the position line: the drive asserts it "when the heads are
    ///   loaded and not moving". For a same-cylinder seek the heads do not
    ///   move, so it never drops. This holds it.
    ///
    /// **MIT's own text reads the other way and is about the controller,
    /// not the drive.** `disk.text` on `STATUS<2>` says "'Implicit' seeks
    /// do not cause attention", and under command 0004 that "the controller
    /// always initiates a seek **if necessary** at the start of a data
    /// transfer command". MIT defines "implicit" nowhere, but the "if
    /// necessary" is the reading that makes the two agree: an implicit seek
    /// is one the controller does *not* issue because the cylinder is
    /// already right, and a tag never sent raises nothing.
    ///
    /// The manual is the T-25 to T-80 book and the drive here is a T-300;
    /// the interface is the family's, and the performance specification
    /// cited at [`REVOLUTION_NS`] covers all five models in one table.
    ///
    /// MIT's controller microcode agrees, and shows where the tag is sent.
    /// Every read, write, read-all and write-all begins with a cylinder tag
    /// ---
    /// `cadrdc/newdsk.31` at 000, 100, 200 and 300, "Cylinder to DBUS,
    /// await completion of previous seek", then "Start seek", then "Hold
    /// DBUS, sync -ON CYLINDER", then "Await seek completion" --- and that
    /// wait is a loop on `ON CYLINDER` with no count in it, so a drive that
    /// did settle for milliseconds on every transfer would make the machine
    /// slow rather than wrong, until the 2.5-second timeout. Completing at
    /// once is the choice here; it is not MIT's statement.
    fn seek_to(&mut self, now: u64, target: u32) {
        if target >= self.unit.geometry.cylinders {
            self.seek_incomplete = true;
            self.attention = true;
            return;
        }
        if target == self.cylinder && self.seek.is_none() {
            self.attention = true;
            return;
        }
        let distance = target.abs_diff(self.cylinder) as u64;
        let done = now + self.seek_settle_ns + self.seek_ns_per_cylinder * distance;
        self.seek = Some((target, done));
    }

    /// Takes what the controller has on the cable at `now`. Call it at
    /// every time the lines change and at every bit clock rise while the
    /// controller is writing, which is when the drive samples the data
    /// pair.
    pub fn observe(&mut self, now: u64, c: ControllerLines) {
        self.settle(now);
        let rose = |now: bool, before: bool| now && !before;
        if c.select {
            if rose(c.cylinder_tag, self.prev.cylinder_tag) {
                let target = (c.bus & 0x3ff) as u32;
                self.tag(now, Tag::Cylinder(target));
                self.seek_to(now, target);
            }
            if rose(c.head_tag, self.prev.head_tag) {
                self.head = (c.bus & 0x3f) as u32;
                self.offset = (c.bus >> 6 & 1 != 0, c.bus >> 7 & 1 != 0);
                self.tag(now, Tag::Head(self.head));
            }
            if rose(c.control_tag, self.prev.control_tag) {
                self.tag(now, Tag::Control(c.bus));
            }
            // Under the control tag the bus is levels: the gates are on
            // while the tag and the bit both are, and the two commands
            // act on their rising edge. Recalibrate on bus 1, pre gate
            // on 2, fault clear on 3, read gate on 6, write gate on 7.
            let prev = self.prev;
            let now_on = |bit: u32| c.control_tag && c.bus >> bit & 1 != 0;
            let was_on = |bit: u32| prev.control_tag && prev.bus >> bit & 1 != 0;
            if now_on(1) && !was_on(1) {
                self.seek_to(now, 0);
                self.seek_incomplete = false;
            }
            if now_on(3) && !was_on(3) {
                self.fault = false;
            }
            let read_gate = now_on(6);
            if read_gate && !self.read_gate {
                // "Reset Attention flip flop in drive" --- `newdsk.31` at
                // 513, and the reason the at-ease command gives some read
                // gate.
                self.attention = false;
            }
            self.read_gate = read_gate;
            let write_gate = now_on(7);
            if self.write_gate && !write_gate {
                self.commit(now);
            }
            self.write_gate = write_gate;
            // "Writing while the disk is read-only causes a fault", and
            // the fault is the drive's: `DEVICE.CHECK/`, cleared by fault
            // clear. Nothing is recorded.
            if write_gate && self.unit.read_only {
                self.fault = true;
            } else if write_gate && Self::clock(now) && now.is_multiple_of(BIT_NS) {
                self.record(now, c.write_data);
            }
        } else {
            self.read_gate = false;
            if self.write_gate {
                self.commit(now);
            }
            self.write_gate = false;
        }
        self.prev = c;
    }

    /// Records one bit the controller wrote, at the clock rise it holds
    /// it for.
    fn record(&mut self, now: u64, bit: Option<bool>) {
        let (sector, _) = self.turn(now);
        let key = (self.cylinder, self.head, sector);
        if self.written.as_ref().is_some_and(|(k, _)| *k != key) {
            self.commit(now);
        }
        let at = self.bit_at(now);
        if at == usize::MAX {
            return;
        }
        let buffer = &mut self.written.get_or_insert_with(|| (key, Vec::new())).1;
        if buffer.len() <= at {
            buffer.resize(at + 1, None);
        }
        buffer[at] = bit;
    }

    /// Puts what the controller wrote over the sector it wrote it on,
    /// parses the result as the format, and writes the block it holds
    /// to the pack. A sector that does not parse is counted and dropped:
    /// that is what a real drive does with garbage, keep it.
    fn commit(&mut self, _now: u64) {
        let Some((key, buffer)) = self.written.take() else { return };
        let (cylinder, head, sector) = key;
        let image = self.image(sector).to_vec();
        let mut bits: Vec<bool> =
            (0..image.len() * 8).map(|k| image[k / 8] >> (k % 8) & 1 != 0).collect();
        for (k, bit) in buffer.iter().enumerate() {
            if let Some(bit) = bit
                && k < bits.len()
            {
                bits[k] = *bit;
            }
        }
        match parse_sector(&bits) {
            Some(s) => {
                self.unit.write_block_at(cylinder, head, sector, &s.data);
                self.image = None;
            }
            None => self.bad_writes += 1,
        }
    }

    /// What is on the cable at `now`.
    pub fn lines(&mut self, now: u64) -> DriveLines {
        self.settle(now);
        let (sector, into) = self.turn(now);
        let selected = self.prev.select;
        let data = if selected && self.read_gate { Some(self.read_bit(now)) } else { None };
        DriveLines {
            on_cylinder: self.seek.is_none(),
            on_line: true,
            read_only: self.unit.read_only,
            fault: self.fault,
            seek_incomplete: self.seek_incomplete,
            selected,
            attention: self.attention,
            sector_index: into < pulse_ns(sector),
            clock: Self::clock(now),
            data,
        }
    }

    /// The next time after `now` at which something on the cable moves
    /// of its own accord: a clock edge, a pulse edge, a seek finishing.
    pub fn next_change(&self, now: u64) -> u64 {
        let half = BIT_NS / 2;
        let clock = (now / half + 1) * half;
        let (sector, into) = self.turn(now);
        let began = now as i64 - into as i64;
        let pulse_end = began + pulse_ns(sector) as i64;
        let length = region_ns(self.sectors(), sector as u64) as i64;
        let pulse = if (now as i64) < pulse_end { pulse_end } else { began + length } as u64;
        let seek = self.seek.map_or(u64::MAX, |(_, at)| at.max(now + 1));
        clock.min(pulse).min(seek)
    }
}

// ---------------------------------------------------------------------------
// The drive on a board's connector nets
// ---------------------------------------------------------------------------

/// A [`Trident`] on the two cables of a disk controller that is a
/// [`Chip`]: the drive wired to the board's connector nets of
/// `data/trident-connectors.txt`, the controller's lines read off the
/// nets and the drive's put on them. `Xbus::plug` puts one on the
/// backplane's controller; `tests/cadrdc_netlist.rs` puts one on a
/// controller alone.
pub struct OnCable {
    pub drive: Trident,
    select: NetId,
    cyl_tag: NetId,
    head_tag: NetId,
    control_tag: NetId,
    bus: Vec<NetId>,
    data_p: NetId,
    data_m: NetId,
    ready: NetId,
    on_line: NetId,
    read_only: NetId,
    device_check: NetId,
    seek_inc: NetId,
    selected: NetId,
    attention: NetId,
    compsecidx: NetId,
    clock_p: NetId,
    clock_m: NetId,
    last: Option<DriveLines>,
}

impl OnCable {
    /// Wires `drive` to the connector nets of `n`, which must be the disk
    /// controller's netlist.
    pub fn new(n: &Netlist, drive: Trident) -> OnCable {
        let net = |name: &str| n.by_name_id(name).unwrap_or_else(|| panic!("no net {name}"));
        OnCable {
            drive,
            select: net("TRIDENT.0.SELECT/"),
            cyl_tag: net("TRIDENT.CYL.TAG/"),
            head_tag: net("TRIDENT.HEAD.TAG/"),
            control_tag: net("TRIDENT.CONTROL.TAG/"),
            bus: (0..10).map(|k| net(&format!("TRIDENT.BUS{k}/"))).collect(),
            data_p: net("TRIDENT.0.DATA.P"),
            data_m: net("TRIDENT.0.DATA.M"),
            ready: net("TRIDENT.READY/"),
            on_line: net("TRIDENT.ON.LINE/"),
            read_only: net("TRIDENT.READ.ONLY/"),
            device_check: net("TRIDENT.DEVICE.CHECK/"),
            seek_inc: net("TRIDENT.SEEK.INC/"),
            selected: net("TRIDENT.0.SELECTED/"),
            attention: net("TRIDENT.0.ATTENTION/"),
            compsecidx: net("TRIDENT.0.COMPSECIDX/"),
            clock_p: net("TRIDENT.0.CLOCK.P"),
            clock_m: net("TRIDENT.0.CLOCK.M"),
            last: None,
        }
    }

    /// Whether the netlist is a board with these cables on it.
    pub fn fits(n: &Netlist) -> bool {
        n.by_name_id("TRIDENT.0.SELECT/").is_some()
    }

    /// Forgets what was last put on the nets, for a board brought up again.
    pub fn reattach(&mut self) {
        self.last = None;
    }

    /// What the controller has on the cable now. The data pair is read
    /// only while the drive is not driving it: the 75110 sinks `DATA.M`
    /// for a one and `DATA.P` for a zero, and leaves the other to the
    /// terminator.
    pub fn controller(&self, c: &Chip) -> ControllerLines {
        let low = |net: NetId| c.net(net) == Level::Low;
        let write_data = if self.last.is_some_and(|l| l.data.is_some()) {
            None
        } else if low(self.data_m) {
            Some(true)
        } else if low(self.data_p) {
            Some(false)
        } else {
            None
        };
        ControllerLines {
            select: low(self.select),
            cylinder_tag: low(self.cyl_tag),
            head_tag: low(self.head_tag),
            control_tag: low(self.control_tag),
            bus: self.bus.iter().enumerate().fold(0, |w, (k, &n)| w | (low(n) as u16) << k),
            write_data,
        }
    }

    /// Gives the drive what the controller has on the cable at `now`, and
    /// puts what the drive answers on the nets, settled at `now`. Returns
    /// whether anything on the nets moved.
    pub fn apply(&mut self, c: &mut Chip, now: u64) -> bool {
        let seen = self.controller(c);
        self.drive.observe(now, seen);
        let l = self.drive.lines(now);
        if self.last == Some(l) {
            return false;
        }
        let lv = |on: bool| if on { Level::High } else { Level::Low };
        for (net, asserted) in [
            (self.ready, l.on_cylinder),
            (self.on_line, l.on_line),
            (self.read_only, l.read_only),
            (self.device_check, l.fault),
            (self.seek_inc, l.seek_incomplete),
            (self.selected, l.selected),
            (self.attention, l.attention),
            (self.compsecidx, l.sector_index),
        ] {
            if asserted {
                c.drive(net, Level::Low);
            } else {
                c.pull_up(net);
            }
        }
        c.drive(self.clock_p, lv(l.clock));
        c.drive(self.clock_m, lv(!l.clock));
        match l.data {
            Some(d) => {
                c.drive(self.data_p, lv(d));
                c.drive(self.data_m, lv(!d));
            }
            None => {
                c.release(self.data_p);
                c.release(self.data_m);
            }
        }
        c.transition(now);
        self.last = Some(l);
        true
    }

    /// When the drive next moves of its own accord after `now`.
    pub fn next_change(&self, now: u64) -> u64 {
        self.drive.next_change(now)
    }
}

// --- Checkpoints ------------------------------------------------------------

impl Unit {
    /// The drive into a checkpoint: the pack's geometry, every block
    /// written since the pack was loaded that the pack does not hold, the
    /// head's position and the three conditions.  Not the file, which is only ever read and a resume
    /// opens again; a pack opened read-write has its writes in the file
    /// instead, and nothing here.
    pub fn save(&self, w: &mut crate::checkpoint::Writer) {
        let Unit {
            geometry,
            image: _,
            writable: _,
            written,
            read_only,
            cylinder,
            head,
            block,
            seek_error,
            fault,
            attention,
        } = self;
        w.u32(geometry.cylinders);
        w.u32(geometry.heads);
        w.u32(geometry.blocks_per_track);
        let mut blocks: Vec<_> = written.iter().collect();
        blocks.sort_by_key(|(lba, _)| **lba);
        w.u64(blocks.len() as u64);
        for (lba, data) in blocks {
            w.u32(*lba);
            w.u32s(data);
        }
        w.bool(*read_only);
        w.u32(*cylinder);
        w.u32(*head);
        w.u32(*block);
        w.bool(*seek_error);
        w.bool(*fault);
        w.bool(*attention);
    }

    /// Back from a checkpoint, into a drive holding a pack of the same
    /// geometry.
    pub fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        let geometry =
            Geometry { cylinders: r.u32()?, heads: r.u32()?, blocks_per_track: r.u32()? };
        if geometry != self.geometry {
            return Err(crate::checkpoint::bad(format!(
                "a pack of {geometry:?}, and the drive holds one of {:?}",
                self.geometry
            )));
        }
        // The count is the file's word, so it is held to the pack --- no pack
        // has more written blocks than blocks --- and the map grows as the
        // blocks are read rather than being reserved from it.
        let n = r.u64()?;
        if n > u64::from(geometry.blocks()) {
            return Err(crate::checkpoint::bad(format!(
                "{n} written blocks on a pack of {}",
                geometry.blocks()
            )));
        }
        let mut written = HashMap::new();
        for _ in 0..n {
            let lba = r.u32()?;
            let mut data = [0u32; BLOCK_WORDS];
            r.u32s_into(&mut data)?;
            written.insert(lba, data);
        }
        self.written = written;
        self.read_only = r.bool()?;
        self.cylinder = r.u32()?;
        self.head = r.u32()?;
        self.block = r.u32()?;
        self.seek_error = r.bool()?;
        self.fault = r.bool()?;
        self.attention = r.bool()?;
        Ok(())
    }
}
