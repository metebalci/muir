// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! QUUX's block-disk: a disk of numbered blocks behind the CADR disk
//! controller's own programming interface, `--disk-controller block-disk`.
//!
//! What stays the CADR's (`sys/doc/disk.text`, and `crate::disk_controller`,
//! which models MIT's board): the four registers at Xbus `17377774` to
//! `17377777` --- status and command, the command list pointer, the disk
//! address, and START --- and the command list itself, one command word a
//! 256-word block, `<23:8>` the page's physical address and `<0>` More; the
//! done interrupt enable, command `<11>`; and the disk address left at the
//! last block moved, or at the one that failed.
//!
//! What goes: cylinders, heads and sectors, seeks, the ECC, Read All and
//! Write All, the drive's own states. The disk address is a block number
//! from the start of the disk, `<27:0>`; the commands are read, 0, and
//! write, 11; anything else stops by error, and so does a transfer that
//! runs past the end of the disk. The disk is a file of any size, raw or a
//! VHD, [`crate::disk_image::Disk`]: block `n` is its 512-byte sectors `2n`
//! and `2n + 1` (contract Q8).
//!
//! The words move inside the store to START, as the CADR model's do; the
//! controller then stays busy for [`BLOCK_NS`] a block moved, which is when
//! it goes not-active and the done interrupt comes.

use crate::disk_image::Disk;
use crate::disk_unit::BLOCK_WORDS;

/// The registers' first physical address, the CADR controller's.
pub const REGS: u32 = crate::disk_controller::REGS;
/// The four registers, by number. Status reads and command writes are the
/// first; START is written, and reads 0.
pub const STATUS: u32 = 0;
pub const COMMAND: u32 = 0;
pub const CLP: u32 = 1;
pub const DA: u32 = 2;
pub const START: u32 = 3;

/// A block's time, the default: 100 us, what an SD card in four-bit mode
/// at 25 MHz takes for a kilobyte and its command, near enough.
/// **Unverified**: an estimate until muir-fpga measures its disk path.
pub const BLOCK_NS: u64 = 100_000;

/// One block moved, as [`BlockDisk::log`] records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transfer {
    /// A write of the disk; a read otherwise, which writes main memory.
    pub write: bool,
    /// The block, from the start of the disk.
    pub block: u32,
    /// The first of the 256 words of physical memory it moved to or from.
    pub page: u32,
}

/// The block-disk: its registers, its error flags, and its disk.
#[derive(Clone)]
pub struct BlockDisk {
    cmd: u32,
    clp: u32,
    da: u32,
    /// The last address the command list made the disk read or write.
    last_memory_address: u32,
    /// When the transfer in flight is done.
    done_at: u64,
    now: u64,
    /// A block's time.
    pub block_ns: u64,
    past_end: bool,
    nxm: bool,
    bad_command: bool,
    disk: Option<Disk>,
    /// Every block moved, in order, when a test or a trace asks for the
    /// record by setting it to `Some`. Not kept in a checkpoint.
    pub log: Option<Vec<Transfer>>,
}

impl BlockDisk {
    pub fn new(block_ns: u64) -> BlockDisk {
        BlockDisk {
            cmd: 0,
            clp: 0,
            da: 0,
            last_memory_address: 0,
            done_at: 0,
            now: 0,
            block_ns,
            past_end: false,
            nxm: false,
            bad_command: false,
            disk: None,
            log: None,
        }
    }

    pub fn attach(&mut self, disk: Disk) {
        self.disk = Some(disk);
    }

    pub fn disk_mut(&mut self) -> Option<&mut Disk> {
        self.disk.as_mut()
    }

    /// The machine's clock, told before the disk is read, written or asked
    /// for its interrupt.
    pub fn advance(&mut self, now: u64) {
        self.now = now;
    }

    fn not_active(&self) -> bool {
        self.now >= self.done_at
    }

    fn error(&self) -> bool {
        self.past_end || self.nxm || self.bad_command
    }

    /// The CADR's done interrupt: not active with command `<11>` set.
    pub fn interrupt(&self) -> bool {
        self.interrupt_at(self.now)
    }

    /// The done interrupt at `now`, for a machine whose clock has moved on
    /// since it last spoke to the disk.
    pub fn interrupt_at(&self, now: u64) -> bool {
        now >= self.done_at && self.cmd & (1 << 11) != 0
    }

    /// The status word: `<0>` not active, `<3>` interrupt request, `<9>` no
    /// pack, `<13>` stopped by error, `<17>` past the end of the pack,
    /// `<20>` NXM, the CADR's bits for the same things.
    pub fn status(&self) -> u32 {
        let mut v = 0;
        if self.not_active() {
            v |= 1;
        }
        if self.interrupt() {
            v |= 1 << 3;
        }
        if self.disk.is_none() {
            v |= 1 << 9;
        }
        if self.error() {
            v |= 1 << 13;
        }
        if self.past_end {
            v |= 1 << 17;
        }
        if self.nxm {
            v |= 1 << 20;
        }
        v
    }

    pub fn read(&self, register: u32) -> u32 {
        match register & 3 {
            STATUS => self.status(),
            CLP => self.last_memory_address,
            DA => self.da,
            _ => 0,
        }
    }

    /// `main` is physical memory, which a transfer reads and writes
    /// directly, the disk being a bus master.
    pub fn write(&mut self, register: u32, v: u32, main: &mut [u32]) {
        match register & 3 {
            COMMAND => {
                self.cmd = v;
                self.past_end = false;
                self.nxm = false;
                self.bad_command = false;
            }
            CLP => self.clp = v,
            DA => self.da = v & 0o1777777777,
            _ => self.start(main),
        }
    }

    fn start(&mut self, main: &mut [u32]) {
        self.past_end = false;
        self.nxm = false;
        self.bad_command = false;
        let read = match self.cmd & 0o17 {
            0o00 => true,
            0o11 => false,
            _ => {
                self.bad_command = true;
                return;
            }
        };
        let Some(mut disk) = self.disk.take() else { return };
        let mut moved = 0u64;
        let mut n = 0u32;
        let mut block = self.da;
        loop {
            // "Only bits <15:0> of the CLP can count", as on the CADR.
            let clp = self.clp & !0xffff | (self.clp.wrapping_add(n)) & 0xffff;
            self.last_memory_address = clp;
            let Some(&ccw) = main.get(clp as usize) else {
                self.nxm = true;
                break;
            };
            let page = (ccw & 0x003f_ff00) as usize;
            if page + BLOCK_WORDS > main.len() {
                self.nxm = true;
                break;
            }
            let ok = if read {
                match disk.read_block(block) {
                    Some(b) => {
                        main[page..page + BLOCK_WORDS].copy_from_slice(&b);
                        true
                    }
                    None => false,
                }
            } else {
                let b: [u32; BLOCK_WORDS] = main[page..page + BLOCK_WORDS].try_into().unwrap();
                disk.write_block(block, &b)
            };
            if !ok {
                self.past_end = true;
                break;
            }
            if let Some(log) = self.log.as_mut() {
                log.push(Transfer { write: !read, block, page: page as u32 });
            }
            self.last_memory_address = (page + BLOCK_WORDS - 1) as u32;
            moved += 1;
            if ccw & 1 == 0 {
                break;
            }
            n += 1;
            block += 1;
        }
        // The last block moved, or the one that failed.
        self.da = block;
        self.disk = Some(disk);
        self.done_at = self.now + moved * self.block_ns;
    }

    /// `-XBUS INIT`: the command and the errors cleared, as a reset does.
    pub fn xbus_init(&mut self) {
        self.cmd = 0;
        self.past_end = false;
        self.nxm = false;
        self.bad_command = false;
        self.done_at = self.now;
    }

    pub fn save(&self, w: &mut crate::checkpoint::Writer) {
        for v in [self.cmd, self.clp, self.da, self.last_memory_address] {
            w.u32(v);
        }
        w.u64(self.done_at);
        w.u64(self.now);
        w.u64(self.block_ns);
        w.bool(self.past_end);
        w.bool(self.nxm);
        w.bool(self.bad_command);
        w.opt(self.disk.as_ref(), |w, d| d.save(w));
    }

    /// Back from a checkpoint, onto a block-disk whose disk is already
    /// attached.
    pub fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        self.cmd = r.u32()?;
        self.clp = r.u32()?;
        self.da = r.u32()?;
        self.last_memory_address = r.u32()?;
        self.done_at = r.u64()?;
        self.now = r.u64()?;
        self.block_ns = r.u64()?;
        self.past_end = r.bool()?;
        self.nxm = r.bool()?;
        self.bad_command = r.bool()?;
        let has_disk = r.bool()?;
        match (has_disk, self.disk.as_mut()) {
            (true, Some(d)) => d.load(r)?,
            (false, None) => {}
            _ => return Err(crate::checkpoint::bad("block-disk: a disk in one and not the other")),
        }
        Ok(())
    }
}
