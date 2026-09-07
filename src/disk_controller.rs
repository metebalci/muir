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
//! Where the two disagree the drawings are the board.  Three places this model
//! is knowingly not the machine: the transfer completes inside the store to
//! START rather than taking milliseconds, so the controller is never seen
//! busy; the block counter in `STATUS<31:24>` is not modelled; and Read
//! All and Write All, which move a track's raw bits, end with the timeout
//! error rather than done, the model having no track format (see
//! `Controller::start`; the three undocumented codes that enter those
//! sectors with the channel turned round go the same way, while 14, 15 and
//! the reserved `xxx7` do what the board does).  A written block goes where
//! the drive puts it: into
//! the image file of a pack opened read-write ([`Unit::open_rw`]), and into
//! memory for the run on a pack opened read-only or blank ([`Unit::open`],
//! [`Unit::blank`]).  The interrupt request is
//! modelled, as the level MIT describes: microcode 323 enables it on every
//! transfer and takes it in `DISK-SWAP-HANDLER`.

use crate::disk_unit::{BLOCK_WORDS, Unit};

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
/// **Which of MIT's two figures is right is unsettled**, and the drawing is
/// followed because this project ranks a drawing above documentation:
///
/// - The drawing is self-consistent.  `mit/cadrdc/dctmot.drw` carries
///   `;Period = 12 ms` on the body *and* labels the net
///   `|TIMEOUT    ;1.5 SEC`; 12 ms x 128 is 1.536 s, so MIT did this
///   arithmetic on their own sheet.
/// - But it disagrees with its own other section.  The timing capacitor is
///   **2 uF, not 1**: `dc.wlr` puts C04 pins 1 *and* 2 on `VCO.C1` and 15
///   *and* 16 on `VCO.C2`, joined BARE, and MIT's parts list gives 1 uF on
///   both those body positions.  Section 2 has one 220 pF body and is drawn
///   `PERIOD = 1.8 - 2.0 usec.`.  Scaling within the one package,
///   1.9 us x (2 uF / 220 pF) is 17.3 ms, which would make the timeout
///   2.1-2.3 s and put `disk.text`'s 2.5 s nearer the mark than the
///   drawing's own note.  The two properties are inconsistent by 1.44.
///
/// So the `12 ms` may be a design estimate off a log-log curve rather than
/// the board.  Settling it needs a 74LS124 datasheet --- no LS sheet has
/// reached this project, and the S part's curve is not the LS part's, which
/// MIT says outright in `cadr1/busint.eco` ECO 5 --- or a scope.
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
    /// How long a transfer takes before the controller is not-active again,
    /// in nanoseconds. Zero, as `muir` runs it: a transfer completes inside
    /// the store to START, since a machine spinning out a drive's
    /// milliseconds of seek would only cost the run microcycles. A test
    /// sets it to see the done wait. The words move at START either way;
    /// what waits is the done.
    pub access_ns: u64,
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
    /// at once for a command the model does not do, [`TIMEOUT_NS`] on for a
    /// reserved code, which hangs the sequencer until the board's timer
    /// stops it: `Controller::start`.
    timeout: bool,
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

    /// `STATUS<0>`: no transfer in flight, or the one there is done.
    fn not_active(&self) -> bool {
        self.now >= self.done_at
    }

    /// The status word.  Every bit here is `DCSTS`'s own name for the signal
    /// on that Xbus line, and MIT's text says the same.
    ///
    /// What is not modelled: `<31:24>` the block counter, `<23>` internal
    /// parity, `<19:12>` the memory-parity, header, ECC, overrun, aborted
    /// and start-block errors, `<8>` off-cylinder, `<5>` no unit selected
    /// and `<4>` multiple units selected.  None of them can happen here,
    /// and the boot PROM's `AWAIT-DRIVE-READY` requires bits 4, 5, 6, 8, 9
    /// and 10 to be clear before it will go on.  `<11>`, the timeout error,
    /// is what a command the model does not do ends with.
    pub fn status(&self) -> u32 {
        let mut v = 0;
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
                if u.attention {
                    v |= 1 << 2;
                }
            }
            // `<9>` "Selected Unit not On-line.  The heads are not loaded, the
            // disk is not powered on, or there is no disk at the specified
            // unit number."
            None => v |= 1 << 9,
        }
        // `<1>` "Any Attention.  Some unit has an attention, you have to
        // select them one after another to find out which."
        if self.units.iter().flatten().any(|u| u.attention) {
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
        let any_attention = self.units.iter().flatten().any(|u| u.attention);
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
        if self.units[self.selected()].is_none() {
            // Start with no drive on that unit: nothing happens and the
            // controller stays ready, so the microcode's on-line wait goes on
            // reading `STATUS<9>`.
            return;
        }
        match self.cmd & 0o17 {
            0o00 => {
                self.transfer(true, false, main);
                self.done_at = self.now + self.access_ns;
            }
            0o10 => {
                self.transfer(true, true, main);
                self.done_at = self.now + self.access_ns;
            }
            0o11 => {
                // "Writing while the disk is read-only causes a fault."
                let u = self.selected();
                if self.units[u].as_ref().is_some_and(|u| u.read_only) {
                    self.units[u].as_mut().unwrap().fault = true;
                } else {
                    self.transfer(false, false, main);
                    self.done_at = self.now + self.access_ns;
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
                let u = self.units[self.selected()].as_mut().unwrap();
                u.seek(cylinder, head, block);
                u.attention = true;
            }
            0o05 | 0o15 => {
                // "0005 At ease.  Resets attention on the selected unit."
                let u = self.units[self.selected()].as_mut().unwrap();
                u.attention = false;
                // "<9> Recalibrate.  In combination with command 5, causes
                // the disk to return the heads to cylinder 0" --- "without
                // assuming the current position of the heads is correct.
                // Recalibrate resets some error conditions in the drive, and
                // causes an attention when complete."
                if self.cmd & (1 << 9) != 0 {
                    u.seek(0, 0, 0);
                    u.fault = false;
                    u.seek_error = false;
                    u.attention = true;
                }
                // "<8> Fault Clear.  In combination with command 5, resets
                // most fault conditions in the disk."
                if self.cmd & (1 << 8) != 0 {
                    u.fault = false;
                }
            }
            // "0006 Offset clear.  Take the heads out of the offset state."
            // There is no servo offset here to take them out of.
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
            0o07 | 0o17 => {
                self.timeout = true;
                self.done_at = self.now + TIMEOUT_NS;
            }
            // Read All and Write All move the bits of a track, headers and
            // all, and 01, 03 and 12 --- the other combinations MIT's table
            // leaves out --- enter the Write, Write All and Read All sectors
            // with the memory channel pointed the other way.  None of them
            // times out on the board, which runs whatever sector `<2:0>`
            // names.  This model has no track format for any of the five;
            // the netlist controller and its drive have.  They are answered
            // with a timeout at once: the error up, the words untouched, the
            // controller ready.
            _ => self.timeout = true,
        }
    }

    /// Reset: the transfer in flight stopped and the errors cleared, the
    /// registers left as they are.
    fn reset(&mut self) {
        self.done_at = self.now;
        self.read_compare_difference = false;
        self.ccw_cycle = false;
        self.nxm = false;
        self.timeout = false;
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
    fn transfer(&mut self, read: bool, compare: bool, main: &mut [u32]) {
        self.read_compare_difference = false;
        self.dma_written.clear();
        self.ccw_cycle = false;
        self.nxm = false;
        self.timeout = false;

        let i = self.selected();
        let mut unit = self.units[i].take().expect("the selected unit is online");
        let (cylinder, head, block) = decode_da(self.da);
        if unit.seek(cylinder, head, block) {
            self.command_list(&mut unit, read, compare, main);
            // "When a transfer is terminated by an error, the disk address
            // register contains the address of the block being transferred
            // when the error occurred.  When a transfer terminated normally,
            // the disk address register has the address of the last block
            // transferred."
            self.da = unit.da(i as u32);
        }
        self.units[i] = Some(unit);
    }

    /// The command list: one CCW per page, each naming where in physical
    /// memory the block goes, until one arrives with the More flag clear.
    fn command_list(&mut self, unit: &mut Unit, read: bool, compare: bool, main: &mut [u32]) {
        // "Only bits <15:0> of the CLP can count; if you attempt to carry
        // into the high 8 bits you will wrap around."  `DCCLP` is where that
        // comes from: four 74LS569s at 0D21, 0D22, 0D23 and 0D25 count
        // `XBAO<15:0>` on `-CLP CLK`, carry chained with the last carry
        // going nowhere, and the 74LS374 at 0D26 holds `XBAO<21:16>` as
        // `-LOAD CLP` latched it.
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
                return;
            }
            if page + BLOCK_WORDS > main.len() {
                self.nxm = true;
                return;
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
                        return;
                    }
                }
            }
            // Why the memory address register is not written word by word:
            // MIT has it as "the last memory reference made by the disk
            // control", so for a page transfer it is the page's last word.
            self.last_memory_address = (page + BLOCK_WORDS - 1) as u32;

            if !more {
                return;
            }
            if !unit.next_block() {
                return;
            }
            n += 1;
        }
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
            access_ns,
            done_at,
            now,
            cmd,
            clp,
            da,
            read_compare_difference,
            dma_written,
            ccw_cycle,
            nxm,
            timeout,
            last_memory_address,
            units,
        } = self;
        w.u64(*access_ns);
        w.u64(*done_at);
        w.u64(*now);
        w.u32(*cmd);
        w.u32(*clp);
        w.u32(*da);
        w.bool(*read_compare_difference);
        w.u64s(&dma_written.iter().map(|&a| a as u64).collect::<Vec<_>>());
        w.bool(*ccw_cycle);
        w.bool(*nxm);
        w.bool(*timeout);
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
        self.access_ns = r.u64()?;
        self.done_at = r.u64()?;
        self.now = r.u64()?;
        self.cmd = r.u32()?;
        self.clp = r.u32()?;
        self.da = r.u32()?;
        self.read_compare_difference = r.bool()?;
        self.dma_written = r.u64s()?.into_iter().map(|a| a as usize).collect();
        self.ccw_cycle = r.bool()?;
        self.nxm = r.bool()?;
        self.timeout = r.bool()?;
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
