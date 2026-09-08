// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The disk controller: four registers on the Xbus, and the transfers they
//! start.
//!
//! Four 32-bit registers on the Xbus at physical `0o17377774`-`0o17377777`,
//! just below the Unibus.  That is where `SET-UP-FOUR-PAGES` maps virtual
//! page 1, so the boot PROM's `A-DISK-REGS` at virtual `0o774` reaches them.
//!
//! Two sources, reaching us by two routes, and they agree on every bit:
//!
//! 1. **`sys/doc/disk.text`** in the System 100 release --- MIT's own
//!    "PROGRAMMING THE DISK CONTROLLER", shipped with the software we target.
//!    Every register and every bit, in their words.  **primary**
//! 2. **The controller's own drawings**, `mit/cadrdc/`, and the netlist
//!    `data/CADRDC.netlist` read out of them.  `DCSTS` names the signal
//!    driving each of `XBO0..31` for the status register, `DCCMD` the command
//!    register's bits and MIT's command table, `DCDA` the disk address
//!    counters, and `DCCCW` the channel command word.  **primary**
//!
//! Where the two disagree the drawings are the board.  Two places this model
//! is knowingly not the machine: the transfer completes inside the store to
//! START rather than taking milliseconds, so the controller is never seen
//! busy; and Read All and Write All, which move a track's raw bits, end
//! with the timeout error rather than done, the model having no track format (see
//! `Controller::start`; the three undocumented codes that enter those
//! sectors with the channel turned round go the same way, while 14, 15 and
//! the reserved `xxx7` do what the board does).  A written block goes where
//! the drive puts it: into
//! the image file of a pack opened read-write ([`Unit::open_rw`]), and into
//! memory for the run on a pack opened read-only or blank ([`Unit::open`],
//! [`Unit::blank`]).  The interrupt request is
//! modelled, as the level MIT describes: microcode 323 enables it on every
//! transfer and takes it in `DISK-SWAP-HANDLER`.

use crate::disk_unit::{self, BLOCK_WORDS, Unit, format};

/// The divider between the timeout clock and `TIMEOUT`, the 74393 at
/// DCTMOT 0C03.
///
/// Read off `data/CADRDC.netlist` against the part's own datasheet: `p1` is
/// `TIMEOUT.CLK`, `p2` and `p12` are both `-ACTIVE` --- the two clears ---
/// `p6` (`1QD`) clocks `p13` (`2A`), and `TIMEOUT` is `p8` (`2QD`).  Both
/// counters count on the falling edge, so `1QD` falls every 16 input
/// periods and `2QD` first rises on the second counter's count of 8.
pub const TIMEOUT_DIVIDER: u64 = 128;

/// How long the board lets an operation run before stopping it with the
/// timeout error, `STATUS<11>`.
///
/// **The board's own clock, so that this engine and the netlist agree.**
/// The timer is the 74LS124 VCO at DCTMOT 0B04 section 1, whose period is
/// [`crate::chip::DISK_TIMEOUT_VCO_PERIOD`] from the drawing's property on
/// that body, counted down by [`TIMEOUT_DIVIDER`].  Both engines take the
/// same two facts and reach the same instant; before, this was
/// `sys/doc/disk.text`'s "a disk operation took longer than 2.5 seconds"
/// and the netlist board ran at 1.536 s, and nothing said which muir meant.
///
/// **Settled by the part's own data sheet, and MIT's prose was right.**
/// The SN74LS124 sheet --- TI's *TTL Data Book*, 2nd edition of 1976, pages
/// 7-123 to 7-128, a part deleted in 1981 and replaced by the 'LS629, which
/// is why no standalone sheet for it survives --- gives the LS part
/// `fo = 1e-4 / Cext`, its own constant and not the S part's `5e-4`.
///
/// The board carries **2 uF** on this section: `dc.wlr` puts C04 pins 1
/// *and* 2 on `VCO.C1` and 15 *and* 16 on `VCO.C2`, joined BARE, and MIT's
/// parts list gives 1 uF on both those positions.  That is 50 Hz, a 20 ms
/// period, and 20 ms x 128 is 2.56 s --- `disk.text`'s "longer than 2.5
/// seconds".
///
/// The drawing's `;Period = 12 ms` corresponds to 1.2 uF, one capacitor and
/// a little stray, and its `|TIMEOUT ;1.5 SEC` note is that figure counted
/// down.  Both were written before the second body went on.
///
/// An earlier reading here took the drawing over the text, on the rule that
/// a drawing outranks documentation.  The rule holds.  What it does not
/// cover is the part's own data sheet, which outranks both and agrees with
/// the text.
///
/// The timeout is also optional hardware: `cadrdc/disk.hand` and `dc.eco`
/// make it the hand jumper `J5-16 : J5-41`, and without it `-TIMEOUT ENB`
/// is high and the section is disabled outright.
pub const TIMEOUT_NS: u64 = crate::chip::DISK_TIMEOUT_VCO_PERIOD.0 * TIMEOUT_DIVIDER
    / crate::chip::DISK_TIMEOUT_VCO_PERIOD.1;

/// Physical address of the first of the four registers.  MIT: "These are
/// normally at physical addresses 17377774-17377777, which is just below the
/// Unibus.  The address can be changed by changing jumpers."
pub const REGS: u32 = 0o17377774;

/// The controller talks to at most eight drives; the unit number is
/// `DA<30:28>`, three bits.  A drive is [`crate::disk_unit::Unit`].
pub const UNITS: usize = 8;

/// Which register a physical address names, if it is one of the four.
pub fn register(phys: u32) -> Option<u32> {
    let off = phys.wrapping_sub(REGS);
    (off < 4).then_some(off)
}

/// The controller: the three registers a program can write, the flags that
/// make up its half of the status word, and the drives it has.
#[derive(Default, Clone)]
pub struct Controller {
    /// Whether an operation takes the drive's own time before the
    /// controller reports itself not-active again.
    ///
    /// **Off, as `muir` runs it**, and that is a choice rather than an
    /// oversight. A drive's milliseconds are microcycles of `DISK-WAIT` on
    /// every engine, and **every count this project quotes was measured
    /// without them**: the boot reaches `PROM-DISABLE` in 1,352,364
    /// microcycles on `micro` with this off and 1,636,112 with it on, 21%
    /// more, and 19 tests fail on the difference. Turning it on is a
    /// decision about all of those numbers rather than about this model.
    ///
    /// On, an operation takes what the drive takes: a seek is
    /// [`crate::disk_unit::seek_ns`] of the distance travelled, and a
    /// transfer waits for its block to come round and then spends a
    /// sector's time on each. So software that starts an operation and
    /// polls sees the controller busy, which is what a driver's wait loop
    /// is for --- and what the netlist controller does either way.
    ///
    /// The words move inside the store to START whichever it is. What
    /// waits is the done.
    pub timed: bool,
    /// When the transfer in flight is done, on the machine's clock.
    done_at: u64,
    /// The machine's clock as of the last time it spoke to the controller:
    /// [`Controller::advance`].
    now: u64,
    /// Write-only: "Note that the command register cannot be read back."
    cmd: u32,
    /// Write-only, and a *physical* address: the PROM writes 0o777 into it
    /// and says so --- "Phys Addr 777 = Virt Addr 1777".
    clp: u32,
    da: u32,
    /// `STATUS<17>`, "Header ECC Error", for the one of its two causes
    /// this model has: `sys/doc/disk.text` says it "also happens if an
    /// attempt is made to continue a read or write operation past the end
    /// of the disk". The other cause is a header whose checkword fails,
    /// which wants a pack that carries headers --- issue 51.
    header_ecc: bool,
    /// `STATUS<22>`, "Read Compare Difference".
    read_compare_difference: bool,
    /// The first word of every page the last transfer put into memory,
    /// for a far end whose memory is elsewhere than `main`: the netlist
    /// memory boards get the same pages poked into their cells.
    pub dma_written: Vec<usize>,
    /// `STATUS<21>`, "CCW Cycle": set while the memory reference being made
    /// is a CCW fetch rather than data.
    ccw_cycle: bool,
    /// `STATUS<20>`, "Nonexistent Memory Error.  Indicates that memory (or
    /// other XBUS device) failed to respond within 15 microseconds.  This
    /// error stops the transfer."
    nxm: bool,
    /// `STATUS<11>`, "Timeout Error.  Indicates that a disk operation took
    /// longer than 2.5 seconds.  This error stops the transfer."  Set at
    /// START for an operation that ends so, and shown once it has ended ---
    /// at once for a command the model does not do, [`TIMEOUT_NS`] on for
    /// one that hangs the sequencer until the board's timer stops it, a
    /// reserved code or a wait on a drive that is not there:
    /// [`Controller::hang`].
    timeout: bool,
    /// `STATUS<14>`, "Overrun.  Indicates that data arrived from the disk
    /// faster than it could be stored into memory, or that memory did not
    /// supply data fast enough for the disk.  This error stops the
    /// transfer."  One command raises it here, `0003`; see
    /// [`Controller::start`], where the measurement is.
    overrun: bool,
    /// Read back as register 1.  MIT puts it on the controller --- "Address
    /// of the last memory reference made by the disk control" --- so it is
    /// one register and not one per drive.  Nothing reads it before the end
    /// of the PROM.
    last_memory_address: u32,
    pub units: [Option<Unit>; UNITS],
}

impl Controller {
    /// Plugs a drive in at a unit number.
    pub fn attach(&mut self, unit: usize, drive: Unit) {
        self.units[unit] = Some(drive);
    }

    /// "Many bits in these registers refer to the 'selected unit', which is
    /// that disk unit whose number is currently in bits `<30:28>` of the
    /// disk-address register."
    fn selected(&self) -> usize {
        (self.da >> 28) as usize & 7
    }

    /// The machine's clock, told before the controller is read, written or
    /// asked for its interrupt, so that a transfer's done can be timed.
    pub fn advance(&mut self, now: u64) {
        self.now = now;
    }

    /// The operation is done `ns` from now, or at once where the run has
    /// not asked for the drive's time: [`Controller::timed`].
    fn done_in(&mut self, ns: u64) {
        self.done_at = self.now + if self.timed { ns } else { 0 };
    }

    /// Raises the selected unit's attention `ns` from now.
    ///
    /// **MIT has the attention at the arrival and not at the store**: of
    /// the seek, "An attention will occur when the seek completes", and of
    /// the recalibrate, it "causes an attention when complete". This model
    /// raised it as the command was stored, which is early by the whole of
    /// the head move --- 6 ms to the next cylinder and 55 across the pack,
    /// [`crate::disk_unit::seek_ns`].
    ///
    /// **The drive on the netlist controller's cable already had it
    /// right**, which is the second source: [`crate::disk_unit::Trident`]
    /// raises it where the seek settles, and `tests/disk.rs` holds it
    /// there. So this is the behavioural model catching up with the
    /// gate-level one.
    ///
    /// Charged like [`Controller::done_in`], so a model that is not
    /// charging the drive's time raises it at once.
    fn attention_in(&mut self, ns: u64) {
        let at = self.now + if self.timed { ns } else { 0 };
        if let Some(u) = self.units[self.selected()].as_mut() {
            u.raise_attention(at);
        }
    }

    /// How long an operation reaching `blocks` blocks takes: the heads'
    /// move to the cylinder, then the wait for the addressed block to come
    /// round, then a sector's time a block.
    ///
    /// The three are the drive's, not the controller's ---
    /// [`crate::disk_unit::seek_ns`], [`crate::disk_unit::until`] and
    /// [`crate::disk_unit::SECTOR_NS`] --- so this model and the drive on
    /// the netlist controller's cable are timed off one set of numbers.
    /// What is left out is the board's own sequencer, tens of microseconds
    /// against the drive's milliseconds.
    fn access_ns(&self, from: u32, to: u32, block: u32, blocks: u32) -> u64 {
        let seek = crate::disk_unit::seek_ns(from.abs_diff(to));
        let latency = crate::disk_unit::until(block, self.now + seek);
        seek + latency + u64::from(blocks) * crate::disk_unit::SECTOR_NS
    }

    /// Read All and Write All go round the whole track, so what they take
    /// is the heads' move, the wait for the addressed block, and then a
    /// revolution.
    fn track_ns(&self, from: u32, to: u32, block: u32) -> u64 {
        let seek = crate::disk_unit::seek_ns(from.abs_diff(to));
        seek + crate::disk_unit::until(block, self.now + seek) + crate::disk_unit::REVOLUTION_NS
    }

    /// Which cylinder the heads are over, for the seek an operation begins
    /// with. No drive is cylinder 0, and a command that needs one has
    /// already returned by the time this is asked.
    fn heads_at(&self) -> u32 {
        self.units[self.selected()].as_ref().map_or(0, Unit::cylinder)
    }

    /// `STATUS<0>`: no transfer in flight, or the one there is done.
    fn not_active(&self) -> bool {
        self.now >= self.done_at
    }

    /// The status word.  Every bit here is `DCSTS`'s own name for the signal
    /// on that Xbus line, and MIT's text says the same.
    ///
    /// What is not modelled: `<23>` internal parity, `<19:18>` the
    /// memory-parity and header-compare errors, `<16:15>` the data ECC
    /// ones, and of `<17>` only the failing checkword --- its other cause,
    /// running off the end of the pack, is here; `<12>` the start-block error
    /// and `<4>` multiple units selected.  None of them can happen here,
    /// and the boot PROM's `AWAIT-DRIVE-READY` requires bits 4, 5, 6, 8, 9
    /// and 10 to be clear before it will go on.  `<14>`, the overrun, one
    /// command does raise --- `0003`, the Write All sector entered with
    /// the memory channel reversed --- measured on the netlist board by
    /// `the_reversed_memory_channel_stores_where_it_should_fetch` in
    /// `tests/cadrdc_netlist.rs`.
    pub fn status(&self) -> u32 {
        let mut v = self.block_counter() << 24;
        if self.read_compare_difference {
            v |= 1 << 22;
        }
        if self.ccw_cycle {
            v |= 1 << 21;
        }
        if self.nxm {
            v |= 1 << 20;
        }
        // Shown once the operation it ends has ended: a reserved code hangs
        // for `TIMEOUT_NS` first.
        if self.timeout && self.not_active() {
            v |= 1 << 11;
        }
        if self.overrun {
            v |= 1 << 14;
        }
        if self.header_ecc {
            v |= 1 << 17;
        }
        // `<13>` "Transfer Aborted": `STOPPED BY ERROR`, preset while any
        // lossage stands, `Controller::lossage`.
        if self.lossage() {
            v |= 1 << 13;
        }
        // `<3>` "Interrupt Request.  1 means the disk controller is asserting
        // -XBUS.INTR."
        if self.interrupt() {
            v |= 1 << 3;
        }
        match &self.units[self.selected()] {
            Some(u) => {
                if u.seek_error {
                    v |= 1 << 10;
                }
                if u.read_only {
                    v |= 1 << 7;
                }
                if u.fault {
                    v |= 1 << 6;
                }
                if u.attention(self.now) {
                    v |= 1 << 2;
                }
            }
            // Nothing on the cable: `<9>` "Selected Unit not On-line.  The
            // heads are not loaded, the disk is not powered on, or there is
            // no disk at the specified unit number"; `<8>` not on cylinder,
            // `-ON CYL SYNC` never coming; and `<5>` "No Unit Selected ...
            // Happens if no disk is plugged into the selected unit number",
            // `NO SELECT` being `UNIT 0 SELECTED` through the 74LS14 at
            // DCTRID 0A07.  All three are levels, there before any command:
            // `a_transfer_with_no_drive_stops_by_error` in
            // `tests/cadrdc_netlist.rs` reads them off the board.
            None => v |= 1 << 9 | 1 << 8 | 1 << 5,
        }
        // `<1>` "Any Attention.  Some unit has an attention, you have to
        // select them one after another to find out which."
        if self.units.iter().flatten().any(|u| u.attention(self.now)) {
            v |= 1 << 1;
        }
        // `<0>` "Not Active.  0 means the controller is busy, 1 means it is
        // ready to accept a command."  Ready unless a transfer is in flight
        // and the run asked for the disk's time: a transfer's words move
        // inside the store to START, where the board takes milliseconds and
        // the microcode spins in `DISK-WAIT`, and by default the done comes
        // with them.
        if self.not_active() {
            v |= 1;
        }
        v
    }

    /// `STATUS<31:24>`: "The block-counter of the selected unit.  This
    /// tells you its current rotational position.  Reading of this register
    /// is not synchronized to its incrementation, so you must read it twice
    /// and check that it came out the same both times."
    ///
    /// On the board it is the two 74LS569s at DCTRID 0B07 and 0B08, clocked
    /// by `BLOCK.CLK^` at the trailing edge of each pulse on the drive's
    /// composite sector/index line and cleared, synchronously, by `-UNIT 0
    /// CLR BC` off the one-shot at 0B09, which only the index pulse
    /// outlasts.  So the count steps to `k` as sector `k`'s pulse ends,
    /// [`crate::disk_unit::SECTOR_PULSE_NS`] in, and holds the last sector's
    /// number through the index pulse until that ends,
    /// [`crate::disk_unit::INDEX_PULSE_NS`] in.  `tests/cadrdc_netlist.rs` reads
    /// the board at both edges and `tests/disk.rs` holds this to the same
    /// readings.  The spindle is the one [`crate::disk_unit::turn`] gives the drive
    /// on the cable, with an index pulse at time zero of the machine's
    /// clock: this controller has one drive, the multiplexor not being
    /// modelled, so one spindle is the machine.  With no drive there are no
    /// pulses; the board's counter holds whatever it last had, and here
    /// that is zero.
    ///
    /// What reads it is CC's `DCHECK-BLOCK-COUNTER` in `sys/cc/dcheck.lisp`:
    /// half a second of reads, expecting every value from 0 to 17 and no
    /// other --- "Vandals: Yes, a value of 17. can appear here".  A 17 needs
    /// an eighteenth pulse in the revolution, a seventeenth sector pulse in
    /// the track's leftover after the index, which is what
    /// `sys/doc/disk.text` describes: "17. sector pulses per track, or one
    /// every 1164. bytes, with a little left over at the end of the track".
    /// [`crate::disk_unit::turn`] spaces the pulses that way, a sector apart
    /// from the index with the leftover last, so the count runs 0 to 17 and
    /// the 17 is the leftover, which holds no block.  Through a pulse the
    /// count is still the region before it, and the region before the index
    /// is that same 17.
    fn block_counter(&self) -> u32 {
        let Some(u) = &self.units[self.selected()] else { return 0 };
        let n = u.geometry.blocks_per_track;
        let (k, into) = crate::disk_unit::turn(n, 0, self.now);
        if into >= crate::disk_unit::pulse_ns(k) {
            k
        } else if k == 0 {
            n
        } else {
            k - 1
        }
    }

    /// `-LOSSAGE`, the level behind `STATUS<13>`: "Transfer Aborted.  This
    /// bit comes on for any error that stops the operation prematurely.
    /// Normally some other bit will also be on, but if this bit is the only
    /// error bit on, some error condition came on then went away again."
    ///
    /// On the board the bit is `STOPPED BY ERROR`, the 74LS74 at DCBUSY
    /// 0B06: `-LOSSAGE` on its preset, `-START` on its clear and `-RESET
    /// ERR` clocking in a zero, [`Controller::reset_errors`].  So it is a
    /// level while lossage stands, and a latch after the lossage goes,
    /// until the next START or store into the command register --- MIT's
    /// "came on then went away again".  `-LOSSAGE` is the 74LS21 at 0B15
    /// over four: the transfer lossage, the 74S260 at 0B13 over `TIMEOUT
    /// ERROR`, `NXM ERROR`, the two overruns and `MEM PARITY ERROR`; the
    /// format and ECC lossages, which need a track format; and the disk
    /// lossage, the other 74S260 at 0B13 over `NO SELECT`, `MULTIPLE
    /// SELECT`, `SEL UNIT FAULT`, `SEL UNIT SEEK ERROR` and `-SEL UNIT ON
    /// LINE`, which the 74LS32 at 0C16 lets through only with `CMD2` low:
    /// a seek or a miscellaneous command runs with the drive in any state,
    /// and a command that uses the memory channel does not start.
    ///
    /// Only the level is kept.  Every lossage this model can raise goes
    /// away by a store into the command register --- the timeout and the
    /// NXM cleared by it, the fault and the seek error by the recalibrate
    /// or fault clear it stores, whose `CMD2` masks the disk lossage from
    /// the store on --- and that store clocks the flop clear too, so the
    /// latch never outlives the level here.
    fn lossage(&self) -> bool {
        let transfer =
            (self.timeout && self.not_active()) || self.nxm || self.overrun || self.header_ecc;
        let disk = self.cmd & 0o4 == 0
            && match &self.units[self.selected()] {
                None => true,
                Some(u) => u.fault || u.seek_error,
            };
        transfer || disk
    }

    /// `-XBUS.INTR`, which the bus interface carries to the cpu as `INT`.
    ///
    /// A level, off the two enables in the command register.  "Done
    /// Interrupt Enable.  Enables not-active (bit 0 of the status register)
    /// to cause an interrupt.  The interrupt will keep happening until you
    /// clear this bit.  (This is really an idle interrupt rather than a done
    /// interrupt.)"  And "Attention Interrupt Enable.  Enables any-attention
    /// (bit 1 of the status register) to cause an interrupt.  (The interrupt
    /// will only happen if the controller is not active ...)".
    ///
    /// The controller here is never active, so the done interrupt is up from
    /// the moment the enable is.  `START-DISK-OP-1` sets it on every
    /// transfer, and `DISK-SWAP-HANDLER`'s `CHECK-PAGE-WRITE`, four
    /// instructions after the start, takes it.  It rises when the transfer
    /// completes, which is the same instant here, and drops when a command
    /// clears both enables.
    pub fn interrupt(&self) -> bool {
        let not_active = self.not_active();
        let done_enable = self.cmd & (1 << 11) != 0;
        let attention_enable = self.cmd & (1 << 10) != 0;
        let any_attention = self.units.iter().flatten().any(|u| u.attention(self.now));
        not_active && (done_enable || (attention_enable && any_attention))
    }

    pub fn read(&self, register: u32) -> u32 {
        match register & 3 {
            0 => self.status(),
            // 1: MEMORY ADDRESS.  `<23:22>` is the controller type, 0 for a
            // Trident, so the register is the address alone.
            1 => self.last_memory_address,
            2 => self.da,
            // 3: ERROR CORRECTION.  No ECC error is ever generated, so there
            // is never a pattern to report.
            _ => 0,
        }
    }

    /// `main` is physical memory, which a transfer reads and writes directly:
    /// the controller is a bus master and does not go through the map.
    pub fn write(&mut self, register: u32, v: u32, main: &mut [u32]) {
        match register & 3 {
            // "Writing the command register does NOT initiate a transfer,
            // unlike most disk controllers.  Use register 3 (START) to
            // initiate a transfer, after setting up the other registers."
            // The one exception is MIT's too: "0016 Reset.  This stops the
            // current transfer and resets the controller.  This command
            // takes effect as soon as it is stored in the command register;
            // no store in START is required.  After storing a Reset command
            // you should store 0 in the command register to turn off the
            // reset condition."
            0 => {
                self.cmd = v;
                self.reset_errors();
                if v & 0o17 == 0o16 {
                    self.reset();
                }
            }
            1 => self.clp = v,
            // "Storing into the Disk Address register momentarily deselects
            // the current unit so that the drive can update its read-only
            // status from the switch."  Nothing models the switch.
            2 => self.da = v,
            _ => self.start(main),
        }
    }

    /// "Writing anything at this address initiates the operation specified in
    /// the command, disk address, and command list pointer registers."
    ///
    /// The command codes are `DCCMD`'s own table, which is MIT's text again:
    /// 00 read, 10 read compare, 11 write, 02 read all, 13 write all, 04
    /// seek, 05 at ease, 1005 recalibrate, 405 fault clear, 06 offset clear,
    /// 16 stop/reset.
    fn start(&mut self, main: &mut [u32]) {
        if self.units[self.selected()].is_none() && self.cmd & 0o4 == 0 {
            // A command that uses the memory channel, with no drive on the
            // unit: the disk lossage presets `BUSY` off before the sequencer
            // runs, so nothing happens and the controller stays ready ---
            // the microcode's on-line wait goes on reading `STATUS<9>`.
            return;
        }
        // Where the heads are before the operation moves them, which is
        // what the length of its seek is measured from.
        let from = self.heads_at();
        let (to, _, block) = decode_da(self.da);
        match self.cmd & 0o17 {
            0o00 => {
                let n = self.transfer(true, false, main);
                self.done_in(self.access_ns(from, to, block, n));
            }
            0o10 => {
                let n = self.transfer(true, true, main);
                self.done_in(self.access_ns(from, to, block, n));
            }
            0o11 => {
                // "Writing while the disk is read-only causes a fault."
                let u = self.selected();
                if self.units[u].as_ref().is_some_and(|u| u.read_only) {
                    self.units[u].as_mut().unwrap().fault = true;
                } else {
                    let n = self.transfer(false, false, main);
                    self.done_in(self.access_ns(from, to, block, n));
                }
            }
            // "Initiates a seek to the cylinder specified in the disk address
            // register.  An attention will occur when the seek completes."
            // `<3>` "means from-memory" and steers the memory channel, which
            // sectors 4 to 7 do not use --- `cadrdc/newdsk.31`, "Commands
            // 4-7 do not use the memory channel" --- so 14 is this seek and
            // 15 the at-ease below, the two combinations MIT's table leaves
            // out of those sectors; `tests/cadrdc_netlist.rs` walks the board
            // through both.
            0o04 | 0o14 => {
                let (cylinder, head, block) = decode_da(self.da);
                let heads_moved = match self.units[self.selected()].as_mut() {
                    Some(u) => {
                        u.seek(cylinder, head, block);
                        // The heads take the drive's own time to get
                        // there, and the controller is busy for it.
                        true
                    }
                    // Sector 4 waits for on-cylinder, which no drive
                    // gives: the board sits at its first step, busy, with
                    // `CMD2` masking the empty cable's lossage.
                    None => false,
                };
                if heads_moved {
                    let ns = crate::disk_unit::seek_ns(from.abs_diff(to));
                    self.done_in(ns);
                    // "An attention will occur when the seek completes",
                    // so it arrives with the heads and not with the
                    // command: the same instant the done is charged at.
                    self.attention_in(ns);
                } else {
                    self.hang();
                }
            }
            0o05 | 0o15 => {
                // "0005 At ease.  Resets attention on the selected unit."
                // Sector 5 "does not start by awaiting seek completion the
                // way the other commands do" and runs on an empty cable to
                // done with no error; with no drive there is nothing to
                // reset.
                let (recalibrate, fault_clear) =
                    (self.cmd & (1 << 9) != 0, self.cmd & (1 << 8) != 0);
                let Some(u) = self.units[self.selected()].as_mut() else { return };
                u.clear_attention();
                // "<9> Recalibrate.  In combination with command 5, causes
                // the disk to return the heads to cylinder 0" --- "without
                // assuming the current position of the heads is correct.
                // Recalibrate resets some error conditions in the drive, and
                // causes an attention when complete."
                let home = recalibrate.then(|| {
                    let was = u.position().0;
                    u.seek(0, 0, 0);
                    u.fault = false;
                    u.seek_error = false;
                    crate::disk_unit::seek_ns(was)
                });
                // "<8> Fault Clear.  In combination with command 5, resets
                // most fault conditions in the disk."
                if fault_clear {
                    u.fault = false;
                }
                // "causes an attention when complete", and the heads come
                // home from wherever they were.
                if let Some(home) = home {
                    self.attention_in(home);
                }
            }
            // "0006 Offset clear.  Take the heads out of the offset state."
            // There is no servo offset here to take them out of.  With no
            // drive, sector 6 waits at its first step as the seek does.
            0o06 if self.units[self.selected()].is_none() => self.hang(),
            0o06 => {}
            // Reset took effect in the store to the command register; a
            // START with it still there starts nothing.
            0o16 => {}
            // `xxx7` is sector 7, and MIT's listing assigns nothing at all in
            // 700-777 --- `cadrdc/newdsk.31` gives the sectors as "0 Read, 1
            // Write, 2 Read All, 3 Write All, 4 Seek, 5 Miscellaneous, e.g.
            // Recalibrate, 6 Offset, 7 not used" and writes 000-050 and
            // 070-074, 100-175, 200-223, 300-315, 400-402, 500-516 and
            // 600-602.  The sequencer starts in unwritten PROM and never
            // finishes, which is why `sys/doc/disk.text` says the code
            // "will currently hang the controller, causing a timeout error
            // (bit 11 in the status register.)": active, with no error,
            // until the board's timer stops it `TIMEOUT_NS` on.
            0o07 | 0o17 => self.hang(),
            // "0002 Read All.  Reads all bits of the disk starting at the
            // specified rotational position" --- the format and the data
            // both, headers, checkwords, preambles and all, which is what
            // [`Controller::track_bytes`] lays out.  "It will not
            // automatically advance heads and cylinders", so it is the one
            // track, round and round.
            0o02 => {
                self.transfer_all(true, main);
                self.done_in(self.track_ns(from, to, block));
            }
            // "0013 Write All.  Writes all bits of the disk starting at the
            // specified rotational position.  This is intended for
            // formatting the disk."
            0o13 => {
                let u = self.selected();
                if self.units[u].as_ref().is_some_and(|u| u.read_only) {
                    self.units[u].as_mut().unwrap().fault = true;
                } else {
                    self.transfer_all(false, main);
                    self.done_in(self.track_ns(from, to, block));
                }
            }
            // 01, 03 and 12: the Write, Write All and Read All sectors
            // entered with the memory channel pointed the other way.
            // MIT's table leaves them out, but the command PROM "is
            // divided into 8 sectors of 64 words each" and `<3>` reaches
            // none of it --- `cadrdc/newdsk.31`, "Commands 4-7 do not use
            // the memory channel" --- so the sequencer runs the sector
            // `<2:0>` names and `<3>` only turns the channel round, so
            // that the fifo is filled from the end it is normally emptied
            // at.
            //
            // **Measured on the netlist board**, which has the fifo:
            // `the_reversed_memory_channel_is_measured` in
            // `tests/cadrdc_netlist.rs` runs all three against it with a
            // drive and a memory and reads off the channel's transfers,
            // the pack and the status register. The three do three
            // different things, and none of them is what this model did
            // before:
            //
            // - **01** runs the whole 61-microword Write program and ends
            //   with **no error at all**. The channel stores 256 words
            //   into the page rather than fetching them, and a
            //   well-formed block lands on the pack --- the drive counts
            //   no bad write.
            // - **03** ends with **Overrun and Transfer Aborted**,
            //   `STATUS<14>` and `<13>`: fourteen microwords, 256 words
            //   stored into the page, and what went to the disk did not
            //   parse, so no block lands.
            // - **12** does not finish. It reads eighteen words out of
            //   memory, stores nothing, and the board's own watchdog
            //   stops it: **Timeout and Transfer Aborted**. So it is
            //   [`Controller::hang`], the same as a reserved code.
            //
            // What is modelled here is the status, which is what software
            // reads. What is **not** modelled is the data: the words the
            // channel stores are the shift registers' own contents
            // cycling --- 3, c000000, 0, 300000, 0, c000 on the board ---
            // and there is no fifo here to produce them. So 01 and 03
            // leave the page and the pack alone where the board disturbs
            // both. That is a measured gap and no longer a guess; it
            // wants the fifo, and the fifo is not here.
            //
            // **And it is left there deliberately, because no software
            // this machine runs can reach it.** `<3>` is the channel's
            // direction, and every command the release issues picks the
            // direction its own sector wants. The whole vocabulary is nine
            // codes: `cold/qcom.lisp` gives READ 0, READ-COMPARE 10, WRITE
            // 11, READ-ALL 2, WRITE-ALL 13, SEEK on 4, RECALIBRATE and
            // FAULT-CLEAR on 5; `ucadr/uc-cadr.lisp` gives the microcode's
            // own three of those; and the disk diagnostic `cc/dcheck.lisp`
            // names the same set with AT-EASE 5, OFFSET-CLEAR 6 and STOP
            // 16, passing a named constant at all twelve of its `DC-EXEC`
            // calls rather than sweeping. **01, 03 and 12 are among the
            // codes nothing writes**, so the status being right for them
            // is already more than anything asks for.
            //
            // Modelling the data would mean writing `3, c000000, 0,
            // 300000, 0, c000` in here as a constant, and that is one
            // board's fifo holding what one prior state left in it ---
            // not a value this model could derive from anything it has.
            // Which is why the netlist test that measured it asserts the
            // count, the status and the pack and **not** the words. Our
            // own measurement is not a reason to invent a mechanism
            // (CLAUDE.md 2), and the invented mechanism would be a
            // constant standing where a fifo belongs.
            0o12 => self.hang(),
            0o03 => {
                self.overrun = true;
                self.done_in(self.access_ns(from, to, block, 1));
            }
            0o01 => self.done_in(self.access_ns(from, to, block, 1)),
            // `& 0o17` leaves four bits, and all sixteen are above.
            0o20.. => unreachable!("a command code is four bits"),
        }
    }

    /// A command the sequencer never finishes: active, with no error, until
    /// the board's timer stops it [`TIMEOUT_NS`] on, with the timeout error
    /// and, through the transfer lossage, `STOPPED BY ERROR`.  Which waits
    /// the netlist board sits in with no drive is measured in
    /// `tests/cadrdc_netlist.rs`; whether it then times out is the hand
    /// jumper `J5-16 : J5-41` of `cadrdc/disk.hand`, "Timeout Enable
    /// jumper", which this model has in, as MIT's text has it.
    fn hang(&mut self) {
        self.timeout = true;
        self.done_at = self.now + TIMEOUT_NS;
    }

    /// `-RESET ERR`: the 74LS08 at DCCMD 0D14 pulls it for `-LOAD CMD` as
    /// for `-XINIT`, so every store into the command register clears the
    /// error flops --- the 74LS273s at 0C12 and 0D24 (NXM, CCW cycle, read
    /// compare difference, memory parity, the overruns, header compare,
    /// start block, ECC soft), the 74LS279 at 0B12 (timeout, header ECC,
    /// ECC hard) and, clocked with a zero, `STOPPED BY ERROR` at 0B06.
    /// `BUSY` is not among them: a hung sequencer stays hung and its timer
    /// keeps counting, so a timeout still to come is kept.
    fn reset_errors(&mut self) {
        self.header_ecc = false;
        self.read_compare_difference = false;
        self.ccw_cycle = false;
        self.nxm = false;
        self.overrun = false;
        if self.not_active() {
            self.timeout = false;
        }
    }

    /// Reset: the transfer in flight stopped and the errors cleared, the
    /// registers left as they are.
    fn reset(&mut self) {
        self.done_at = self.now;
        self.reset_errors();
    }

    /// `-XBUS INIT` on the backplane, `-XINIT` off the 26S10 at DCCHAN
    /// 0F19.  It clears the command register --- the 74LS175 at DCCMD 0C21
    /// and the 74LS273 at 0C10, pin 1 of each --- and is an input of two
    /// gates on DCCMD: `RESET` at the 74LS00 0A28, beside `-CMD.RESET`,
    /// which stops the channel (DCCHAN 0E16, DCBUSY 0C26); and `-RESET
    /// ERR` at the 74LS08 0D14, beside `-LOAD CMD`, which clears the error
    /// flops (the 74LS273s at DCSTS 0C12 and 0D24, the 74LS279 at 0B12, the
    /// 74LS74s at DCBUSY 0B06 and DCRBUF 0E02).  So it is the Reset command
    /// with the command register cleared as well.  The disk address
    /// counters (the 74LS193s at DCDA 0A17-0A21), the command list pointer
    /// (DCCLP) and the CCW latches (DCCCW) have no pin on it and stand.
    pub fn xbus_init(&mut self) {
        self.reset();
        self.cmd = 0;
    }

    /// One read, read-compare or write, from the disk address register
    /// through the command list.
    fn transfer(&mut self, read: bool, compare: bool, main: &mut [u32]) -> u32 {
        self.read_compare_difference = false;
        self.dma_written.clear();
        self.ccw_cycle = false;
        self.nxm = false;
        self.timeout = false;
        self.overrun = false;
        self.header_ecc = false;

        let i = self.selected();
        let mut unit = self.units[i].take().expect("the selected unit is online");
        let (cylinder, head, block) = decode_da(self.da);
        let mut blocks = 0;
        if unit.seek(cylinder, head, block) {
            blocks = self.command_list(&mut unit, read, compare, main);
            // "When a transfer is terminated by an error, the disk address
            // register contains the address of the block being transferred
            // when the error occurred.  When a transfer terminated normally,
            // the disk address register has the address of the last block
            // transferred."
            self.da = unit.da(i as u32);
        }
        self.units[i] = Some(unit);
        blocks
    }

    /// Read All and Write All: the same command list, with a track's bytes
    /// on the disk side instead of a block's words.
    ///
    /// The two commands are the format itself --- "The format is
    /// determined by the program that uses the Write All operation to
    /// format the disk" --- so what crosses the channel is the bytes as
    /// they lie under the head, low-order byte first, as everything on
    /// this disk goes.
    fn transfer_all(&mut self, read: bool, main: &mut [u32]) {
        self.read_compare_difference = false;
        self.dma_written.clear();
        self.ccw_cycle = false;
        self.nxm = false;
        self.timeout = false;
        self.overrun = false;
        self.header_ecc = false;

        let i = self.selected();
        let mut unit = self.units[i].take().expect("the selected unit is online");
        let (cylinder, head, block) = decode_da(self.da);
        if unit.seek(cylinder, head, block) {
            if read {
                let bytes = track_bytes(&mut unit, cylinder, head, block);
                self.read_all(&bytes, main);
            } else {
                let bytes = self.write_all_bytes(main);
                lay_down_track(&mut unit, cylinder, head, block, &bytes);
            }
            self.da = unit.da(i as u32);
        }
        self.units[i] = Some(unit);
    }

    /// Read All: the track's bytes into the pages the command list names,
    /// four bytes to a word, low-order byte first.  The track is read
    /// round and round --- the command does not advance the head --- so a
    /// list longer than a track comes back to where it started.
    fn read_all(&mut self, bytes: &[u8], main: &mut [u32]) {
        let mut at = 0usize;
        self.each_ccw(main, |d, page, main| {
            if page + BLOCK_WORDS > main.len() {
                d.nxm = true;
                return false;
            }
            for w in &mut main[page..page + BLOCK_WORDS] {
                let mut b = [0u8; 4];
                for byte in &mut b {
                    *byte = bytes[at % bytes.len()];
                    at += 1;
                }
                *w = u32::from_le_bytes(b);
            }
            d.dma_written.push(page);
            true
        })
    }

    /// Write All: the pages the command list names, back into bytes.
    fn write_all_bytes(&mut self, main: &mut [u32]) -> Vec<u8> {
        let mut bytes = Vec::new();
        self.each_ccw(main, |d, page, main| {
            if page + BLOCK_WORDS > main.len() {
                d.nxm = true;
                return false;
            }
            bytes.extend(main[page..page + BLOCK_WORDS].iter().flat_map(|w| w.to_le_bytes()));
            true
        });
        bytes
    }

    /// The command list walked, `page` by `page`, until a CCW arrives with
    /// the More flag clear or `each` says to stop.  [`Controller::command_list`]
    /// walks the same list a block at a time; this one has no block to
    /// advance to, the track being one stream.
    fn each_ccw(
        &mut self,
        main: &mut [u32],
        mut each: impl FnMut(&mut Controller, usize, &mut [u32]) -> bool,
    ) {
        let mut n = 0u32;
        loop {
            let clp = self.clp & !0xffff | (self.clp.wrapping_add(n)) & 0xffff;
            self.last_memory_address = clp;
            self.ccw_cycle = true;
            let Some(&ccw) = main.get(clp as usize) else {
                self.nxm = true;
                return;
            };
            self.ccw_cycle = false;
            let page = (ccw & 0x003f_ff00) as usize;
            let more = ccw & 1 != 0;
            if !each(self, page, main) {
                return;
            }
            self.last_memory_address = (page + BLOCK_WORDS - 1) as u32;
            if !more {
                return;
            }
            n += 1;
        }
    }

    /// The command list: one CCW per page, each naming where in physical
    /// memory the block goes, until one arrives with the More flag clear.
    fn command_list(
        &mut self,
        unit: &mut Unit,
        read: bool,
        compare: bool,
        main: &mut [u32],
    ) -> u32 {
        // "Only bits <15:0> of the CLP can count; if you attempt to carry
        // into the high 8 bits you will wrap around."  `DCCLP` is where that
        // comes from: four 74LS569s at 0D21, 0D22, 0D23 and 0D25 count
        // `XBAO<15:0>` on `-CLP CLK`, carry chained with the last carry
        // going nowhere, and the 74LS374 at 0D26 holds `XBAO<21:16>` as
        // `-LOAD CLP` latched it.
        let mut n = 0u32;
        // Blocks that reached the pack or the page, which is what the
        // operation's length is measured in; `n` is the CCW the list is on,
        // which is one behind until the first block has moved.
        let mut moved = 0u32;
        loop {
            let clp = self.clp & !0xffff | (self.clp.wrapping_add(n)) & 0xffff;
            self.last_memory_address = clp;
            self.ccw_cycle = true;
            let Some(&ccw) = main.get(clp as usize) else {
                self.nxm = true;
                return moved;
            };
            self.ccw_cycle = false;

            // "<23:8> Main memory address of a page.  <0> More flag."
            // `DCCCW` latches `XBI<21:8>` into two 74LS374s --- 0E25 takes
            // `XBI<15:8>`, 0E26 `XBI<21:16>` with its pins 2 to 5 left
            // unconnected, and `dc.wlr` has the same part in that place ---
            // and clocks `XBI0` into the LAST CCW flag.  So the page address
            // is the 22 bits the Xbus has, and bits 22 and 23 of the CCW go
            // nowhere: `disk.text`'s `<23:8>` is two bits wider than the
            // board, and the board is followed.
            let page = (ccw & 0x003f_ff00) as usize;
            let more = ccw & 1 != 0;

            let mut from_disk = [0u32; BLOCK_WORDS];
            if read && !unit.read_block(&mut from_disk) {
                unit.fault = true;
                return moved;
            }
            if page + BLOCK_WORDS > main.len() {
                self.nxm = true;
                return moved;
            }
            let in_memory = &mut main[page..page + BLOCK_WORDS];
            match (read, compare) {
                // "Read-compare.  Reads from both disk and memory, and sets
                // bit 22 of the status register if they don't agree."  "This
                // error does not stop the transfer."
                (true, true) => {
                    if in_memory != from_disk {
                        self.read_compare_difference = true;
                    }
                }
                (true, false) => {
                    in_memory.copy_from_slice(&from_disk);
                    self.dma_written.push(page);
                }
                (false, _) => {
                    let words: &[u32; BLOCK_WORDS] = (&*in_memory).try_into().unwrap();
                    if !unit.write_block(words) {
                        unit.fault = true;
                        return moved;
                    }
                }
            }
            // Why the memory address register is not written word by word:
            // MIT has it as "the last memory reference made by the disk
            // control", so for a page transfer it is the page's last word.
            self.last_memory_address = (page + BLOCK_WORDS - 1) as u32;
            moved += 1;

            if !more {
                return moved;
            }
            // "Header ECC Error also happens if an attempt is made to
            // continue a read or write operation past the end of the
            // disk" --- `sys/doc/disk.text`. `next_block` stepping off
            // the pack is that attempt, and it used to end the transfer
            // saying nothing at all.
            if !unit.next_block() {
                self.header_ecc = true;
                return moved;
            }
            n += 1;
        }
    }
}

/// The track under the heads as it lies on the pack, from `block` round to
/// itself: each block's [`disk_unit::sector_image`] end to end, and the
/// [`format::LEFTOVER`] at the end of the track as the ones every gap in
/// this format is written with.
///
/// A block the pack has no data for reads as zeros, which is what a blank
/// image gives; the format around it is there either way, because the
/// format is what the drive would be carrying.
fn track_bytes(unit: &mut Unit, cylinder: u32, head: u32, block: u32) -> Vec<u8> {
    let g = unit.geometry;
    let mut bytes = Vec::with_capacity(format::TRACK);
    for k in 0..g.blocks_per_track {
        let b = (block + k) % g.blocks_per_track;
        let data = unit.block_at(cylinder, head, b).unwrap_or([0; BLOCK_WORDS]);
        // The header the sector carries, which is not always the one its
        // address implies: a Write All wrote whatever the program had.
        let header = unit
            .header_at(cylinder, head, b)
            .unwrap_or_else(|| disk_unit::header_of(&g, cylinder, head, b));
        bytes.extend(disk_unit::sector_image_with_header(header, &data));
        if b + 1 == g.blocks_per_track {
            bytes.resize(bytes.len() + format::LEFTOVER, 0xff);
        }
    }
    bytes
}

/// Write All: every sector the written bytes carry, put where its own
/// header says.
///
/// The header is what decides: "HEADER - a 32-bit word ... `<27:16>`
/// cylinder number, used to verify that the disk is positioned to the
/// correct cylinder.  `<15:8>` head number ... `<7:0>` block number".  A
/// formatter lays out a track and writes it, and the addresses it wrote
/// into the headers are the addresses the blocks then have.
///
/// A sector whose bytes run out is not written: "it doesn't really write
/// quite all of the last page; somewhere between zero and seventeen words
/// will be lost", so the tail of the stream is expected to be short.
fn lay_down_track(unit: &mut Unit, cylinder: u32, head: u32, from: u32, bytes: &[u8]) {
    let mut at = 0usize;
    let mut k = 0u32;
    while at + format::SECTOR <= bytes.len() {
        let bits: Vec<bool> = bytes[at..at + format::SECTOR]
            .iter()
            .flat_map(|&b| (0..8).map(move |k| b >> k & 1 != 0))
            .collect();
        let Some(s) = disk_unit::parse_sector(&bits) else { break };
        // **The sector goes where the heads are, and its header goes in
        // it.** That is what a formatter does: the drive writes the track
        // under the heads, and the addresses in the headers are the
        // program's to choose. Until issue 51 this placed the block at the
        // address its *header* named, because `Unit` had nowhere to put a
        // header --- so a formatted pack could never disagree with itself
        // and `STATUS<18>`, Header Compare, could not fire.
        let block = (from + k) % unit.geometry.blocks_per_track;
        k += 1;
        // An address the geometry has no room for stops the track here.
        //
        // **The board raises nothing.** This comment used to say the board
        // would give `STATUS<18>`, Header Compare Error, and that is wrong:
        // Write All never compares a header. `cadrdc/newdsk.31` strobes
        // one only in the Read sector, `024` to `027`, and the Write
        // sector, `124` to `127`, each "Compare first header byte" and so
        // on against the disk address register; the Write All sector from
        // `300` seeks, selects the head, finds the index pulse at `313`
        // and writes the track out, and no word of it carries `HEADER
        // STROBE`. Which is what formatting means: the header a Write All
        // lays down is whatever the program put in memory, and the drive
        // writes it where the heads are. A header naming an address that
        // fits nowhere is simply written, and becomes a pack that a later
        // Read or Write fails to compare against.
        //
        // So stopping the track is this model's own, and the board would
        // have written it. What the board cannot do is what `Unit` cannot
        // represent: it stores blocks by the address they are asked for,
        // so there is nowhere to put a block whose header disagrees with
        // its place. That is issue #8's missing track format rather than a
        // missing status bit.
        if !unit.write_sector_at(cylinder, head, block, s.header, &s.data) {
            return;
        }
        at += format::SECTOR;
    }
}

/// "`<27:16>` Cylinder number.  `<15:8>` Head number.  `<7:0>` Block number."
/// `DCDA` wires exactly those three fields as 74LS193 counters.
fn decode_da(da: u32) -> (u32, u32, u32) {
    ((da >> 16) & 0o7777, (da >> 8) & 0xff, da & 0xff)
}

// --- Checkpoints ------------------------------------------------------------

impl Controller {
    /// The controller into a checkpoint: its registers, the command in
    /// progress, and each unit's drive.
    pub fn save(&self, w: &mut crate::checkpoint::Writer) {
        let Controller {
            timed,
            done_at,
            now,
            cmd,
            clp,
            da,
            header_ecc,
            read_compare_difference,
            dma_written,
            ccw_cycle,
            nxm,
            timeout,
            overrun,
            last_memory_address,
            units,
        } = self;
        // A word, where a byte would do, so that the field keeps the
        // width `access_ns` had here: a checkpoint written before this was
        // a switch carries the flat time, and any of it means timed.
        w.u64(u64::from(*timed));
        w.u64(*done_at);
        w.u64(*now);
        w.u32(*cmd);
        w.u32(*clp);
        w.u32(*da);
        w.bool(*header_ecc);
        w.bool(*read_compare_difference);
        w.u64s(&dma_written.iter().map(|&a| a as u64).collect::<Vec<_>>());
        w.bool(*ccw_cycle);
        w.bool(*nxm);
        w.bool(*timeout);
        w.bool(*overrun);
        w.u32(*last_memory_address);
        for u in units {
            w.bool(u.is_some());
            if let Some(u) = u {
                u.save(w);
            }
        }
    }

    /// Back from a checkpoint, into a controller with a drive on every unit
    /// the checkpoint has one on, and no other.
    pub fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        use crate::checkpoint::bad;
        self.timed = r.u64()? != 0;
        self.done_at = r.u64()?;
        self.now = r.u64()?;
        self.cmd = r.u32()?;
        self.clp = r.u32()?;
        self.da = r.u32()?;
        self.header_ecc = r.bool()?;
        self.read_compare_difference = r.bool()?;
        self.dma_written = r.u64s()?.into_iter().map(|a| a as usize).collect();
        self.ccw_cycle = r.bool()?;
        self.nxm = r.bool()?;
        self.timeout = r.bool()?;
        self.overrun = r.bool()?;
        self.last_memory_address = r.u32()?;
        for (unit, slot) in self.units.iter_mut().enumerate() {
            match (r.bool()?, slot) {
                (true, Some(u)) => u.load(r)?,
                (false, None) => {}
                (true, None) => {
                    return Err(bad(format!(
                        "a drive on unit {unit}, and this machine has none there: give it the pack"
                    )));
                }
                (false, Some(_)) => {
                    return Err(bad(format!(
                        "no drive on unit {unit}, and this machine has one there"
                    )));
                }
            }
        }
        Ok(())
    }
}
