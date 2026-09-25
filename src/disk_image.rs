// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! QUUX's disk file (contract Q8): raw, a fixed VHD or a dynamic VHD, read
//! and written as block-disk's blocks of 1,024 bytes. Block `n` is the
//! disk's 512-byte sectors `2n` and `2n + 1`.
//!
//! The CADR's pack is not this: it is MIT's raw Trident image, exactly a
//! T-300's size, and `crate::disk_unit::Unit` reads it.
//!
//! **The VHD layout is Microsoft's *Virtual Hard Disk Image Format
//! Specification*** (October 2006), every field big-endian:
//!
//! - the **footer**, 512 bytes at the file's end: cookie `conectix` at 0,
//!   data offset at 16, current size at 48, disk type at 60 (2 fixed,
//!   3 dynamic, 4 differencing), and at 64 a checksum, the one's
//!   complement of the sum of the footer's bytes with the field as zero;
//! - a **fixed** VHD is the raw disk and then the footer;
//! - a **dynamic** VHD has a copy of the footer at 0, its dynamic header
//!   (cookie `cxsparse`, 1,024 bytes, the same kind of checksum at 36) at
//!   the footer's data offset, the block allocation table at the header's
//!   table offset (16), its entry count at 28 and the block size at 32; a
//!   table entry is the sector where a block's sector bitmap begins, the
//!   block's data following the bitmap, or `FFFFFFFF` for a block not
//!   allocated, which reads as zeros.
//!
//! `tests/quux_disk.rs` holds each of these to files made by qemu-img and
//! qemu-io: every block of each reads as its raw twin, and a dynamic VHD
//! grown here is qemu-io's own file byte for byte.
//!
//! **What a write to an unallocated block does** is the specification's
//! and qemu's: the block goes where the footer was, its bitmap all ones
//! and its data zeros but for what is written, then the footer after it,
//! then the table entry. The footer's bytes, checksum included, are the
//! ones read at open; only where they are moves. In that order a write cut
//! short leaves at worst a block no entry names, and a footer missing at
//! the end is what the copy at 0 is for: such a file opens by the copy.
//!
//! **Unverified**: a sector whose bit is clear in an allocated block's
//! bitmap reads what the block holds, not zeros; and a write into an
//! allocated block sets the sector's bit taking the bitmap's first sector
//! as the high bit of its first byte. qemu and muir set every bit when
//! they allocate, so neither matters to a file either of them made and
//! nothing here has exercised either. A file made by a tool that leaves
//! bits clear would settle both.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::disk_unit::BLOCK_WORDS;

/// A sector: the VHD's unit and the GPT's.
pub const SECTOR: u64 = 512;

/// A block of block-disk, two sectors.
pub const BLOCK_BYTES: usize = BLOCK_WORDS * 4;

/// Block-disk's disk address is a block number, `<27:0>`
/// (`crate::block_disk`), so this many blocks are all it can reach.
pub const MAX_BLOCKS: u64 = 1 << 28;

/// What a disk file is, told by its footer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Raw,
    FixedVhd,
    DynamicVhd,
}

impl Format {
    /// The words the start line uses.
    pub fn name(self) -> &'static str {
        match self {
            Format::Raw => "raw",
            Format::FixedVhd => "a fixed VHD",
            Format::DynamicVhd => "a dynamic VHD",
        }
    }
}

const COOKIE: &[u8; 8] = b"conectix";
const DYNAMIC_COOKIE: &[u8; 8] = b"cxsparse";
const FOOTER_CHECKSUM: usize = 64;
const HEADER_CHECKSUM: usize = 36;
const UNALLOCATED: u32 = u32::MAX;

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

fn be64(b: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(b[at..at + 8].try_into().unwrap())
}

/// The VHD checksum of a footer or dynamic header, its own field at `at`
/// counted as zero.
fn checksum(b: &[u8], at: usize) -> u32 {
    let sum = b
        .iter()
        .enumerate()
        .filter(|(k, _)| !(at..at + 4).contains(k))
        .fold(0u32, |s, (_, &x)| s.wrapping_add(u32::from(x)));
    !sum
}

fn invalid(what: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, what)
}

fn read_at(f: &mut File, at: u64, buf: &mut [u8]) -> std::io::Result<()> {
    f.seek(SeekFrom::Start(at))?;
    f.read_exact(buf)
}

fn write_at(f: &mut File, at: u64, buf: &[u8]) -> std::io::Result<()> {
    f.seek(SeekFrom::Start(at))?;
    f.write_all(buf)
}

/// A footer, if these 512 bytes are one: `None` without the cookie, an
/// error for a cookie whose checksum does not check.
fn footer(b: &[u8; 512]) -> std::io::Result<Option<(u32, u64)>> {
    if &b[..8] != COOKIE {
        return Ok(None);
    }
    let (want, got) = (be32(b, FOOTER_CHECKSUM), checksum(b, FOOTER_CHECKSUM));
    if want != got {
        return Err(invalid(format!(
            "a VHD footer whose checksum does not check: {want:08x}, the bytes sum to {got:08x}"
        )));
    }
    Ok(Some((be32(b, 60), be64(b, 48))))
}

/// The file behind a disk.
enum Image {
    /// The disk's bytes from 0, and nothing else that is the disk's.
    Flat {
        file: File,
        format: Format,
    },
    Dynamic(Box<Dynamic>),
}

/// A dynamic VHD's file and what locating a sector in it takes.
struct Dynamic {
    file: File,
    /// The block allocation table, as in the file.
    table: Vec<u32>,
    table_offset: u64,
    /// A block's data, and the sector bitmap before it.
    block_bytes: u64,
    bitmap_bytes: u64,
    /// The footer as read, written again after each block allocated.
    footer: [u8; 512],
    /// Where the next block goes: where the footer is.
    end: u64,
}

impl Dynamic {
    fn open(mut file: File, footer_bytes: [u8; 512], end: u64) -> std::io::Result<Dynamic> {
        let at = be64(&footer_bytes, 16);
        let mut h = [0u8; 1024];
        read_at(&mut file, at, &mut h)?;
        if &h[..8] != DYNAMIC_COOKIE {
            return Err(invalid(format!("no dynamic header at {at}: a dynamic VHD has one")));
        }
        let (want, got) = (be32(&h, HEADER_CHECKSUM), checksum(&h, HEADER_CHECKSUM));
        if want != got {
            return Err(invalid(format!(
                "a dynamic header whose checksum does not check: {want:08x}, the bytes sum to {got:08x}"
            )));
        }
        let table_offset = be64(&h, 16);
        let entries = be32(&h, 28) as u64;
        let block_bytes = u64::from(be32(&h, 32));
        if block_bytes == 0 || !block_bytes.is_multiple_of(SECTOR) {
            return Err(invalid(format!("a block of {block_bytes} bytes, not whole sectors")));
        }
        let size = be64(&footer_bytes, 48);
        if entries * block_bytes < size {
            return Err(invalid(format!(
                "{entries} blocks of {block_bytes} bytes, short of the disk's {size}"
            )));
        }
        // A bit a sector, padded to whole sectors.
        let bitmap_bytes = (block_bytes / SECTOR).div_ceil(8).next_multiple_of(SECTOR);
        let mut raw = vec![0u8; entries as usize * 4];
        read_at(&mut file, table_offset, &mut raw)?;
        let table = raw.as_chunks::<4>().0.iter().map(|b| u32::from_be_bytes(*b)).collect();
        Ok(Dynamic {
            file,
            table,
            table_offset,
            block_bytes,
            bitmap_bytes,
            footer: footer_bytes,
            end: end.next_multiple_of(SECTOR),
        })
    }

    /// Where in the file the disk's byte `at` is, or `None` in a block not
    /// allocated. `at` and the span after it lie in one block.
    fn locate(&self, at: u64) -> Option<(usize, u64)> {
        let k = (at / self.block_bytes) as usize;
        let entry = self.table[k];
        (entry != UNALLOCATED)
            .then(|| (k, u64::from(entry) * SECTOR + self.bitmap_bytes + at % self.block_bytes))
    }

    fn read(&mut self, at: u64, buf: &mut [u8]) -> std::io::Result<()> {
        match self.locate(at) {
            Some((_, off)) => read_at(&mut self.file, off, buf),
            None => {
                buf.fill(0);
                Ok(())
            }
        }
    }

    /// Writes one sector of the disk, allocating its block if need be.
    fn write_sector(&mut self, at: u64, buf: &[u8]) -> std::io::Result<()> {
        if let Some((k, off)) = self.locate(at) {
            write_at(&mut self.file, off, buf)?;
            // The sector's bit in the bitmap, set if it is not.
            let sector = at % self.block_bytes / SECTOR;
            let bit_at = u64::from(self.table[k]) * SECTOR + sector / 8;
            let mut byte = [0u8];
            read_at(&mut self.file, bit_at, &mut byte)?;
            let bit = 0x80 >> (sector % 8);
            if byte[0] & bit == 0 {
                write_at(&mut self.file, bit_at, &[byte[0] | bit])?;
            }
            return Ok(());
        }
        let k = (at / self.block_bytes) as usize;
        let sector = u32::try_from(self.end / SECTOR).map_err(|_| {
            invalid(format!("a block at byte {}, past what a table entry holds", self.end))
        })?;
        let mut block = vec![0u8; (self.bitmap_bytes + self.block_bytes) as usize];
        block[..self.bitmap_bytes as usize].fill(0xff);
        let into = (self.bitmap_bytes + at % self.block_bytes) as usize;
        block[into..into + buf.len()].copy_from_slice(buf);
        // The data, the footer after it, and only then the entry.
        write_at(&mut self.file, self.end, &block)?;
        let end = self.end + block.len() as u64;
        write_at(&mut self.file, end, &self.footer)?;
        write_at(&mut self.file, self.table_offset + 4 * k as u64, &sector.to_be_bytes())?;
        self.table[k] = sector;
        self.end = end;
        Ok(())
    }
}

impl Image {
    /// Opens a disk file and tells what it is: its format and its size in
    /// bytes, which for a VHD is the footer's current size.
    fn open(path: &Path, writable: bool) -> std::io::Result<(Image, u64)> {
        let mut file = OpenOptions::new().read(true).write(writable).open(path)?;
        let len = file.metadata()?.len();
        let mut first = [0u8; 512];
        if len >= 8 {
            read_at(&mut file, 0, &mut first[..len.min(512) as usize])?;
            if &first[..8] == b"vhdxfile" {
                return Err(invalid("a VHDX, which is not a VHD: muir reads VHD".into()));
            }
        }
        let mut last = [0u8; 512];
        let trailing = if len >= SECTOR {
            read_at(&mut file, len - SECTOR, &mut last)?;
            footer(&last)?
        } else {
            None
        };
        let (kind, size, footer_bytes, end) = match trailing {
            Some((kind, size)) => (kind, size, last, len - SECTOR),
            None => match footer(&first)? {
                // A dynamic VHD's copy at 0, its footer at the end lost.
                Some((3, size)) if len >= SECTOR => (3, size, first, len),
                _ => return Ok((Image::Flat { file, format: Format::Raw }, len)),
            },
        };
        match kind {
            2 => Ok((Image::Flat { file, format: Format::FixedVhd }, size)),
            3 => Ok((Image::Dynamic(Box::new(Dynamic::open(file, footer_bytes, end)?)), size)),
            4 => Err(invalid("a differencing VHD, which muir does not read".into())),
            k => Err(invalid(format!("a VHD of disk type {k}, which is none muir reads"))),
        }
    }

    fn format(&self) -> Format {
        match self {
            Image::Flat { format, .. } => *format,
            Image::Dynamic(_) => Format::DynamicVhd,
        }
    }

    fn read_block(&mut self, n: u32) -> std::io::Result<[u8; BLOCK_BYTES]> {
        let mut b = [0u8; BLOCK_BYTES];
        let at = u64::from(n) * BLOCK_BYTES as u64;
        match self {
            Image::Flat { file, .. } => read_at(file, at, &mut b)?,
            Image::Dynamic(d) => {
                for (k, s) in b.chunks_mut(SECTOR as usize).enumerate() {
                    d.read(at + k as u64 * SECTOR, s)?;
                }
            }
        }
        Ok(b)
    }

    fn write_block(&mut self, n: u32, b: &[u8; BLOCK_BYTES]) -> std::io::Result<()> {
        let at = u64::from(n) * BLOCK_BYTES as u64;
        match self {
            Image::Flat { file, .. } => write_at(file, at, b),
            Image::Dynamic(d) => {
                for (k, s) in b.chunks(SECTOR as usize).enumerate() {
                    d.write_sector(at + k as u64 * SECTOR, s)?;
                }
                Ok(())
            }
        }
    }

    fn try_clone(&self) -> std::io::Result<Image> {
        Ok(match self {
            Image::Flat { file, format } => {
                Image::Flat { file: file.try_clone()?, format: *format }
            }
            Image::Dynamic(d) => Image::Dynamic(Box::new(Dynamic {
                file: d.file.try_clone()?,
                table: d.table.clone(),
                footer: d.footer,
                ..**d
            })),
        })
    }
}

/// What a disk file is and how many bytes of disk it holds, without
/// opening it as a disk: for the start line and for `diskpack`, which
/// refuses QUUX's disks by it.
pub fn probe(path: &Path) -> std::io::Result<(Format, u64)> {
    let (image, size) = Image::open(path, false)?;
    Ok((image.format(), size))
}

/// What a file is if it is QUUX's disk and not a CADR pack: a VHD, or a
/// disk with a GPT --- `EFI PART` at the start of sector 1, block 0's second
/// half, UEFI's header signature --- in words for a refusal. `None` for
/// anything else, a CADR pack among it.
pub fn quux_disk(path: &Path) -> Option<String> {
    let mut d = Disk::open(path).ok()?;
    let block = d.read_block(0)?;
    let bytes: Vec<u8> = block.iter().flat_map(|w| w.to_le_bytes()).collect();
    let gpt = &bytes[512..520] == b"EFI PART";
    match (d.format()?, gpt) {
        (Format::Raw, false) => None,
        (f, true) => Some(format!("{} with a GPT", f.name())),
        (f, false) => Some(f.name().to_string()),
    }
}

/// QUUX's disk: the file, its size in blocks, and the blocks written this
/// run that the file does not hold.
pub struct Disk {
    blocks: u32,
    /// `None` is a blank disk, [`Disk::blank`], every block zero until
    /// written.
    image: Option<Image>,
    /// Whether a written block goes to the file. [`Disk::open_rw`] writes
    /// through; [`Disk::open`] keeps it in [`Disk::written`] instead, for
    /// the run and a checkpoint.
    writable: bool,
    written: HashMap<u32, [u32; BLOCK_WORDS]>,
}

/// A second handle on the same file, and the written blocks copied.
impl Clone for Disk {
    fn clone(&self) -> Self {
        Disk {
            blocks: self.blocks,
            image: self.image.as_ref().map(|i| i.try_clone().expect("a second handle on the disk")),
            writable: self.writable,
            written: self.written.clone(),
        }
    }
}

impl Disk {
    /// Opens a disk read-only: the file is never written, and a written
    /// block is kept in memory for the run.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Disk> {
        Self::open_with(path.as_ref(), false)
    }

    /// Opens a disk read-write: a written block goes to the file, growing
    /// a dynamic VHD where it has to.
    pub fn open_rw(path: impl AsRef<Path>) -> std::io::Result<Disk> {
        Self::open_with(path.as_ref(), true)
    }

    fn open_with(path: &Path, writable: bool) -> std::io::Result<Disk> {
        let (image, size) =
            Image::open(path, writable).map_err(|e| invalid(format!("{}: {e}", path.display())))?;
        // Whole blocks: a last half block, a sector the size leaves over,
        // is not reachable by block number.
        let blocks = size / BLOCK_BYTES as u64;
        if blocks > MAX_BLOCKS {
            return Err(invalid(format!(
                "{}: {blocks} blocks, and block-disk's address reaches {MAX_BLOCKS}",
                path.display()
            )));
        }
        Ok(Disk { blocks: blocks as u32, image: Some(image), writable, written: HashMap::new() })
    }

    /// A disk of `blocks` with nothing on it.
    pub fn blank(blocks: u32) -> Disk {
        Disk { blocks, image: None, writable: false, written: HashMap::new() }
    }

    /// The disk's size in blocks.
    pub fn blocks(&self) -> u32 {
        self.blocks
    }

    /// The file's format, `None` for a blank disk.
    pub fn format(&self) -> Option<Format> {
        self.image.as_ref().map(Image::format)
    }

    /// Whether a written block goes to the file.
    pub fn writable(&self) -> bool {
        self.writable
    }

    /// How many blocks the run has written that the file does not hold.
    pub fn written_blocks(&self) -> usize {
        self.written.len()
    }

    /// Block `n` off the file: zeros off a blank disk, `None` for one the
    /// file could not deliver.
    fn file_block(&mut self, n: u32) -> Option<[u32; BLOCK_WORDS]> {
        let mut words = [0u32; BLOCK_WORDS];
        let Some(image) = self.image.as_mut() else { return Some(words) };
        match image.read_block(n) {
            // Each 32-bit word low byte first, as the CADR's pack has it
            // (`crate::disk_unit`).
            Ok(b) => {
                for (w, b) in words.iter_mut().zip(b.as_chunks::<4>().0) {
                    *w = u32::from_le_bytes(*b);
                }
                Some(words)
            }
            Err(e) => {
                eprintln!("disk: reading block {n} of the image: {e}");
                None
            }
        }
    }

    /// Block `n`: `None` past the end, or for a block the file could not
    /// deliver.
    pub fn read_block(&mut self, n: u32) -> Option<[u32; BLOCK_WORDS]> {
        if n >= self.blocks {
            return None;
        }
        if let Some(b) = self.written.get(&n) {
            return Some(*b);
        }
        self.file_block(n)
    }

    /// Block `n` written: to the file on a disk opened read-write, else
    /// kept in memory. False past the end, or for a block the file would
    /// not take.
    pub fn write_block(&mut self, n: u32, data: &[u32; BLOCK_WORDS]) -> bool {
        if n >= self.blocks {
            return false;
        }
        match (self.writable, self.image.as_mut()) {
            (true, Some(image)) => {
                let mut b = [0u8; BLOCK_BYTES];
                for (b, w) in b.as_chunks_mut::<4>().0.iter_mut().zip(data) {
                    *b = w.to_le_bytes();
                }
                if let Err(e) = image.write_block(n, &b) {
                    eprintln!("disk: writing block {n} of the image: {e}");
                    return false;
                }
                self.written.remove(&n);
            }
            _ => {
                // A block written as the file holds it is not kept, so a
                // checkpoint carries only what the run changed.
                if self.file_block(n) == Some(*data) {
                    self.written.remove(&n);
                } else {
                    self.written.insert(n, *data);
                }
            }
        }
        true
    }

    /// Into a checkpoint: the size in blocks and every block written that
    /// the file does not hold. Not the file, which a resume opens again.
    pub fn save(&self, w: &mut crate::checkpoint::Writer) {
        w.u32(self.blocks);
        let mut blocks: Vec<_> = self.written.iter().collect();
        blocks.sort_by_key(|(n, _)| **n);
        w.u64(blocks.len() as u64);
        for (n, data) in blocks {
            w.u32(*n);
            w.u32s(data);
        }
    }

    /// Back from a checkpoint, onto a disk of the same size.
    pub fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        let blocks = r.u32()?;
        if blocks != self.blocks {
            return Err(crate::checkpoint::bad(format!(
                "a disk of {blocks} blocks, and this one is {}",
                self.blocks
            )));
        }
        let n = r.u64()?;
        if n > u64::from(blocks) {
            return Err(crate::checkpoint::bad(format!(
                "{n} written blocks on a disk of {blocks}"
            )));
        }
        let mut written = HashMap::new();
        for _ in 0..n {
            let at = r.u32()?;
            let mut data = [0u32; BLOCK_WORDS];
            r.u32s_into(&mut data)?;
            written.insert(at, data);
        }
        self.written = written;
        Ok(())
    }
}
