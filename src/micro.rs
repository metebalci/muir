// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The `micro` engine: one step per microinstruction.
//!
//! It models the architecturally visible pipeline --- the two-deep fetch, the
//! inhibited cycle after a jump, the OA register merge, the two-cycle memory
//! data delay --- but not the clock phases or any signal that only the
//! diagnostic interface can see.  Those are what `rtl` and `chip` are for.
//!
//! Field positions and the functional source and destination codes are
//! `mit/cadr/ir.bits`, MIT's own tables.  The ALU is the drawings': page ALUC4
//! decides what the 74S181s are asked to do and page ALU0-1 does it, both
//! shared with the `rtl` engine; see `crate::ttl`.  There is one carry-out and
//! it is bit 32 of the 33-bit array.
//!
//! The jump conditions are the ALU's too --- page FLAG selects `AEQM` and
//! bit 32 of the 33-bit array, which is where the signedness comes from.
//!
//! Two things here are the board's rather than the obvious reading: a
//! dispatch on the map takes the map bit *instead of* the field's bit 0, off
//! the 74S64s at 2F24/2F05/2F23, and `MAP(MD)`'s permission bits come from
//! the map word of the last memory cycle, latched at VMEMDR 1D14, rather than
//! from `MD`'s page live.  `rtl` is this engine's reference
//! (`tests/cosim.rs`). Its clock is the
//! sum of the machine's microcycle periods --- the speed bits and `ILONG`,
//! two stages behind the mode register --- and not the waits and hangs on
//! the bus, which it does not have: a word is in `MD` two instructions
//! after the cycle starts whatever the memory is doing. `rtl` counts what
//! it waited, and `micro_keeps_the_machines_periods` holds the two clocks
//! to each other with the waits taken off; over the boot they are five
//! per cent of `rtl`'s time. `memory_cycle_ns` stands in for them: the
//! band's mean wait rounded, [`MEMORY_ACCESS_NS`], charged on every memory cycle so
//! that this clock runs near the machine's; a test that wants the periods
//! alone sets it to zero.
//!
//! So this engine has a clock but no timing model: nothing in it happens
//! at an instant or waits for one. Whatever the machine does that is a
//! matter of when --- the bus's waits and hangs, the interface's
//! arbitration and timeouts, a device answering late, the debug cable,
//! which is the interface's timing and nothing else --- is `rtl`'s and
//! `chip`'s, and this engine has no end of the cable.
//!
//! The pipeline and the memory timing are the part not written from the
//! drawings, and this engine is the wrong place to settle them: `rtl` and
//! `chip` compute them from the wiring, and `tests/cosim.rs` holds the
//! engines to each other.  The multiply and divide steps are not exercised by
//! the boot PROM, so nothing yet checks them.

use crate::busint;
use crate::clock::Speed;
use crate::engine::Engine;
use crate::isa::{Insn, Op};
use crate::machine::{Halt, LVMO_AT_POWER_ON, Machine};
use crate::spy;
use crate::ttl;

/// What this engine charges its clock per memory cycle: the machine's mean
/// wait on the bus in the band, rounded. `rtl` measured 469,431,577 ns
/// stalled over 1,028,921 memory cycles from executed instruction 1,300,000
/// to 15,000,000 on the System 100 pack, 456 ns a cycle;
/// `micro_keeps_the_machines_periods` prints the figure of the run it
/// makes. The boot PROM's own mean is 545, over a run too short to matter.
pub const MEMORY_ACCESS_NS: u64 = 460;

pub struct Micro {
    pub m: Machine,

    /// The instruction being executed, and the one already fetched behind it.
    p0: Insn,
    p0_pc: u16,
    p1: Insn,
    p1_pc: u16,
    npc: u16,

    inhibit: bool,
    popj: bool,

    oal: bool,
    oah: bool,
    oa_low: u64,
    oa_high: u64,

    new_md: u32,
    new_md_delay: u8,

    aaddr: u16,
    maddr: u8,
    adata: u32,
    mdata: u32,
    alu_out: u32,
    old_q: u32,
    out: u32,
    iwr: u64,

    executed: Option<u16>,

    /// The map word of the last memory cycle, as the 74S373 at VMEMDR 1D14
    /// holds it: what `MAP(MD)`'s permission bits are read from.
    lvmo: u32,
    /// Whether that cycle was a write, which is what makes a write
    /// permission fault a fault.
    wrcyc: bool,
    /// The speed the clock runs at, and the one the mode register asks for
    /// next: OLORD1 1A01 is two stages on `-TPR60`, so a new speed takes
    /// two microcycles to reach the tap select, as in `rtl`.
    speed: Speed,
    speed_a: Speed,
    /// A fixed charge to the clock for every memory cycle this engine
    /// starts, in nanoseconds: a stand-in for the waits the machine takes on
    /// the bus, which this engine does not model. [`MEMORY_ACCESS_NS`], the
    /// band's mean; a test sets it to zero to see the periods alone.
    pub memory_cycle_ns: u64,
    /// Memory cycles started, for [`Micro::memory_cycles`].
    memory_cycles: u64,
    /// Microcycles the board spends nopped after a control-store write,
    /// still to be charged.
    nopped: u8,
    /// `SRUN`, `SSTEP` and `SSDONE`, the 74S174 at OLORD1 1A10 as `rtl` has
    /// it: the console's `RUN` and `STEP` one master clock behind, `STEP`
    /// twice.  A microcycle runs while `SRUN` is up, or for the one master
    /// clock in which `SSTEP` is up and `SSDONE` is not.
    srun: bool,
    sstep: bool,
    ssdone: bool,
    /// `HALTED`: the instruction executed asked for misc function 1,
    /// `HALT-CONS`, and `IR` holds it while the machine is halted.  Under
    /// `ERRSTOP` it stops the machine, as the 74S374 at OLORD2 1A05, the
    /// 74S133 at 1A02 and the run logic at OLORD1 1A15 do; see
    /// [`crate::rtl::Rtl`], which has the register itself.
    halted: bool,
}

impl Micro {
    pub fn new(m: Machine) -> Self {
        Micro {
            m,
            p0: Insn::new(0),
            p0_pc: 0,
            p1: Insn::new(0),
            p1_pc: 0,
            npc: 0,
            inhibit: false,
            popj: false,
            oal: false,
            oah: false,
            oa_low: 0,
            oa_high: 0,
            new_md: 0,
            new_md_delay: 0,
            aaddr: 0,
            maddr: 0,
            adata: 0,
            mdata: 0,
            alu_out: 0,
            old_q: 0,
            out: 0,
            iwr: 0,
            executed: None,
            lvmo: LVMO_AT_POWER_ON,
            wrcyc: false,
            speed: Speed::ExtraSlow,
            speed_a: Speed::ExtraSlow,
            memory_cycle_ns: MEMORY_ACCESS_NS,
            memory_cycles: 0,
            nopped: 0,
            srun: false,
            sstep: false,
            ssdone: false,
            halted: false,
        }
    }

    /// The control store address executed in the last [`Engine::step`], or
    /// `None` if that cycle was inhibited.  An inhibited cycle runs on the
    /// board and retires nothing, so it is not an executed instruction.
    pub fn executed(&self) -> Option<u16> {
        self.executed
    }

    /// The master clock edge, as far as this engine has one: the run and
    /// step synchronisers, and the two pulses a mode-register write can
    /// make.  See `Rtl::mclk_edge`, which this follows.
    fn mclk_edge(&mut self) {
        let boot = std::mem::take(&mut self.m.prog_boot);
        let reset = std::mem::take(&mut self.m.prog_reset) || boot;
        if reset {
            self.m.reset_console_registers();
        }
        if boot {
            self.m.vmaok = false;
            self.m.clock_control.run = true;
            self.npc = 0;
            self.inhibit = true;
        }
        self.ssdone = self.sstep;
        self.sstep = self.m.clock_control.step;
        self.srun = self.m.clock_control.run;
    }

    /// The speed select, two stages behind the mode register, as `rtl` has
    /// it off OLORD1 1A01.
    fn speedclk(&mut self) {
        self.speed = self.speed_a;
        self.speed_a = match (self.m.mode.speed1, self.m.mode.speed0) {
            (false, false) => Speed::ExtraSlow,
            (false, true) => Speed::Slow,
            (true, false) => Speed::Normal,
            (true, true) => Speed::Fast,
        };
    }

    /// `IR<pos+len-1:pos>` of the instruction being executed.
    fn ir(&self, pos: u32, len: u32) -> u32 {
        ((self.p0.raw() >> pos) & ((1u64 << len) - 1)) as u32
    }

    fn advance_pipeline(&mut self) {
        // `IR` loads from the debug IR instead of the control store while
        // the console holds `IDEBUG` up.
        self.p0 = if self.m.clock_control.idebug { Insn::new(self.m.debug_ir) } else { self.p1 };
        self.p0_pc = self.p1_pc;
        self.p1 = self.m.fetch(self.npc);
        self.p1_pc = self.npc;
        self.npc = if self.npc == 0o37777 { 0 } else { self.npc + 1 };
        self.m.opc = self.p0_pc;
    }

    /// Byte position for the LC byte modes, `IR<11:10> == 3`: `IR<4:3>`
    /// with the location counter's low two bits folded in, so that the
    /// instruction picks the halfword or the byte `LC` points at.
    ///
    /// The gates are on page LC of `data/CADR.netlist`, and `rtl` computes
    /// them net for net:
    ///
    /// ```text
    /// 3E11  74S00   -LC MODIFIES MROT = NAND(IR10, IR11)
    /// 2E05  74S86   INST IN LEFT HALF = NOR(-LC MODIFIES MROT, LC1 XOR LC0B)
    /// 2E05  74S86   -SH4 = INST IN LEFT HALF XOR -IR4
    /// 2E30  74S02O  INST IN 2ND OR 4TH QUARTER
    ///                 = AND(NOR(-LC MODIFIES MROT, LC0), LC BYTE MODE)
    /// 2E05  74S86   -SH3 = -IR3 XOR INST IN 2ND OR 4TH QUARTER
    /// ```
    ///
    /// with `LC0B` being `LC0 AND LC BYTE MODE`. So `SH4` is `IR4` unless
    /// `LC1 XOR LC0B` is up, and `SH3` is `IR3` unless `LC0` is *down* in
    /// byte mode: `LC` = 1 selects byte 0, 2 byte 1, 3 byte 2 and 4 byte 3,
    /// which is what CC's `CC-TEST-LC-DP` in `sys/cc/diags.lisp` expects
    /// ("Select byte (initially rightmost, LC=current+1)"). The boot never
    /// enters byte mode; `tests/cosim.rs` holds the engines to each other
    /// on it.
    fn lc_byte_mode(&self) -> u32 {
        let ir4 = self.ir(4, 1);
        let ir3 = self.ir(3, 1);
        let lc1 = (self.m.lc >> 1) & 1;
        let lc0 = self.m.lc & 1;
        if self.m.byte_mode() {
            self.ir(0, 3) | ((ir4 ^ lc1 ^ lc0 ^ 1) << 4) | ((ir3 ^ lc0 ^ 1) << 3)
        } else {
            self.ir(0, 4) | ((ir4 ^ lc1 ^ 1) << 4)
        }
    }

    /// Steps the location counter, fetching the next instruction word when
    /// `NEEDFETCH` calls for one.
    ///
    /// The hardware derives `NEEDFETCH` rather than storing it, and the
    /// netlist names every term of it on page LC:
    ///
    /// ```text
    /// 1E07  74S08   LC0B = 'LC BYTE MODE' AND LC0
    /// 3E17  74S02O  'LAST BYTE IN WORD' = NOR(LC1, LC0B)
    /// 3E09  74S32   NEEDFETCH = 'HAVE WRONG WORD' OR 'LAST BYTE IN WORD'
    /// ```
    ///
    /// This engine has no `'HAVE WRONG WORD'`: that term is `-NEWLC` NAND
    /// `-DESTLC` at 3E11, which only a write to the counter raises. So
    /// `NEEDFETCH` is carried as `LC<31>` instead --- set when the step lands
    /// on the last byte of a word, cleared by the fetch it asks for. `rtl`
    /// and `chip` compute the gates themselves, and `tests/cosim.rs` holds
    /// the three engines to one trace.
    ///
    /// Returns `ppc` with bit 1 forced when no fetch happened, which is how
    /// the caller skips the page-fault check.
    fn advance_lc(&mut self, mut ppc: u32) -> Result<u32, Halt> {
        // LC counts bytes and a word is four of them, so the word to fetch is
        // the counter *before* the step, shifted down by two. The counter is
        // `LC<25:0>` (page LC); the flags this engine keeps above it stay.
        let fetch_from = (self.m.lc & 0o377777777) >> 2;
        let inc = if self.m.byte_mode() { 1 } else { 2 };
        let lc = (self.m.lc & 0o377777777).wrapping_add(inc) & 0o377777777;
        self.m.lc = (self.m.lc & !0o377777777) | lc;

        if self.m.lc & (1 << 31) != 0 {
            self.m.lc &= !(1 << 31);
            // `IFETCH` is a term of `MEMOP` on page VCTL1 and `VMAS` is
            // `LC<25:2>` under it, so the fetch is a memory cycle like any
            // read: the map word latched, the clock charged, `MD` loaded
            // only if the map permits.
            self.m.vma = fetch_from;
            self.start_read();
        } else {
            ppc |= 2;
        }

        // 1E07 and 3E17, on the counter as stepped.
        let lc0b = self.m.byte_mode() && (self.m.lc & 1 != 0);
        let last_byte_in_word = !lc0b && (self.m.lc & 2 == 0);
        if last_byte_in_word {
            self.m.lc |= 1 << 31;
        }
        Ok(ppc)
    }

    /// Functional sources, `IR<30:26>` when `IR<31>` is set.
    ///
    /// The codes and the names below are MIT's own `FUNCTIONAL SOURCES` table
    /// in `mit/cadr/ir.bits`, which reads, in octal:
    ///
    /// ```text
    /// 0 Dispatch Constant     4 Illegal (Pdl)   10 VMA       14 SPC ptr & data, pop
    /// 1 SPC pointer and data  5 Pdl Buffer (X)  11 MAP(MD)   15 -
    /// 2 Pdl Buffer Pointer    6 OPC             12 MD        16 -
    /// 3 Pdl Buffer Index      7 Q               13 Location Counter    17 -
    ///                        24 Pdl Buffer Pop
    ///                        25 Pdl Buffer (P)
    /// ```
    ///
    /// `(X)` is the PDL addressed by the index and `(P)` by the pointer.
    /// Code 4 MIT calls illegal, and 15 to 17 it leaves unassigned.
    fn read_functional(&mut self, source: u8) -> Result<u32, Halt> {
        // The SPC word carries the pointer above the entry it selects.
        let spc_word =
            |m: &Machine| ((m.spcptr as u32) << 24) | (m.spc[m.spcptr as usize] & 0o1777777);
        // Page SOURCE decodes `IR<31>`, `IR<29>` and `IR<28:26>`: two
        // 74S138s, one under `-IR29` for sources 0 to 7 and one under `IR29`
        // for 10 to 17. `IR<30>` is not in the decode; on page PDLCTL it is
        // `PDLP` while `CLK` is up, which reads the PDL buffer by its pointer
        // rather than by its index. So 24 and 25 are 4 and 5 read by the
        // pointer, and 26 is 6, the OPC, with a bit that does nothing there.
        let by_pointer = source & 0o20 != 0;
        let pdl_at =
            |m: &Machine| if by_pointer { m.pdl_pointer as usize } else { m.pdl_index as usize };
        Ok(match source & 0o17 {
            // Dispatch Constant
            0o0 => self.m.dispatch_constant as u32,
            // SPC pointer and data
            0o1 => spc_word(&self.m),
            // Pdl Buffer Pointer, Pdl Buffer Index
            0o2 => self.m.pdl_pointer as u32 & 0o1777,
            0o3 => self.m.pdl_index as u32 & 0o1777,
            // Pdl Buffer Pop: by the pointer as 24, or by the index as 4,
            // MIT's `Illegal (Pdl)`, the pointer counting down either way
            // (`PDLCNT` on page PDLCTL).
            0o4 => {
                let v = self.m.pdl[pdl_at(&self.m)];
                self.m.pdl_pointer = self.m.pdl_pointer.wrapping_sub(1) & 0o1777;
                v
            }
            // Pdl Buffer (P) as 25, Pdl Buffer (X) as 5.
            0o5 => self.m.pdl[pdl_at(&self.m)],
            // OPC, Q
            0o6 => self.m.opc as u32,
            0o7 => self.m.q,
            // VMA
            0o10 => self.m.vma,
            // MAP(MD).  Bit 29 is **zero**: VMEMDR 1A01 drives it from `HI12`
            // through a 74S240, and the '240 inverts; read off the drawing
            // without the buffer it would be a one.
            //
            // Bits 31 and 30 are `-PFW` and `-PFR`, and on the board they
            // come off the 74S373 at VMEMDR 1D14: the map word of the *last
            // memory cycle*, not of `MD`'s page now, a write fault being one
            // only if that cycle was a write. Computing them from `MD` live
            // is the easy misreading; `rtl` and `chip` have the latch, and
            // `TRANS-OLD0`'s `DISPATCH L2-MAP-STATUS-CODE` reads exactly the
            // bits of the last cycle's page. The rest is live, as the board
            // addresses the map by `MD` outside a cycle.
            0o11 => {
                let t = self.m.translate(self.m.md);
                let pfr = (self.lvmo >> 23) & 1 != 0;
                let pfw = !((self.lvmo >> 22) & 1 == 0 && self.wrcyc);
                ((!pfw as u32) << 31)
                    | ((!pfr as u32) << 30)
                    | ((t.l1_data & 0o37) << 24)
                    | (t.l2_data & 0o77777777)
            }
            // MD
            0o12 => self.m.md,
            // Location Counter.  Bit 0 only means anything in byte mode.
            0o13 => {
                if self.m.byte_mode() {
                    self.m.lc
                } else {
                    self.m.lc & !1
                }
            }
            // SPC ptr & data, pop
            0o14 => {
                let v = spc_word(&self.m);
                self.m.spcptr = self.m.spcptr.wrapping_sub(1) & 0o37;
                v
            }
            // Functional sources 0o15, 0o16 and 0o17: the 74S138 for the
            // upper eight has those three outputs unconnected, so no part
            // drives the M bus and an undriven TTL bus reads high, as `chip`
            // shows. Microcode 323 reads 0o15 once, at 0o20535.
            _ => !0,
        })
    }

    /// Functional destinations, `dest >> 5` of `IR<25:14>`.
    ///
    /// MIT's own `FUNCTIONAL DESTINATIONS` table in `mit/cadr/ir.bits`, in octal:
    ///
    /// ```text
    ///  0 Nowhere            10 Pdl Buffer Top      20 VMA
    ///  1 Location Counter   11 Pdl Buffer Push     21 VMA, start read
    ///  2 Interrupt Control  12 Pdl Buffer (Index)  22 VMA, start write
    ///  3 -                  13 Pdl Buffer Index    23 VMA, MAP(MD)VMA
    ///  4 -                  14 Pdl Buffer Pointer  30 MD
    ///  5 -                  15 SPC, push           31 MD, start read (!)
    ///  6 -                  16 IMOD<25:0>          32 MD, start write
    ///  7 -                  17 IMOD<47:26>         33 MD, MAP(MD)VMA
    /// ```
    ///
    /// The `(!)` on 31 is MIT's, not ours.  3 to 7 and 24 to 27 are
    /// unassigned and halt here.
    fn write_functional(&mut self, dest: u16, data: u32) -> Result<(), Halt> {
        match dest >> 5 {
            // Nowhere
            0o0 => {}
            // LOCATION-COUNTER.  Writing it always sets NEED-FETCH.
            0o1 => {
                self.m.lc = (self.m.lc & !0o377777777) | (data & 0o377777777);
                if !self.m.byte_mode() {
                    self.m.lc &= !1;
                }
                self.m.lc |= 1 << 31;
            }
            // INTERRUPT-CONTROL.  Bit 28 is `PROG.UNIBUS.RESET`, the 25LS2519
            // at FLAG 3E08: as it rises the model I/O boards are reset,
            // `Machine::bus_reset`; this engine has no bus interface or
            // memory boards for it to hold.  The flag bits are mirrored
            // into LC.
            0o2 => {
                let was = self.m.interrupt_control & (1 << 28) != 0;
                self.m.interrupt_control = data;
                self.m.lc = (self.m.lc & !(0o17 << 26)) | (data & (0o17 << 26));
                if !was && data & (1 << 28) != 0 {
                    self.m.bus_reset();
                }
            }
            // Pdl Buffer Top, Push, (Index), Index, Pointer
            0o10 => self.m.pdl[self.m.pdl_pointer as usize] = data,
            0o11 => {
                self.m.pdl_pointer = (self.m.pdl_pointer + 1) & 0o1777;
                self.m.pdl[self.m.pdl_pointer as usize] = data;
            }
            0o12 => self.m.pdl[self.m.pdl_index as usize] = data,
            0o13 => self.m.pdl_index = data as u16 & 0o1777,
            0o14 => self.m.pdl_pointer = data as u16 & 0o1777,
            // SPC, push
            0o15 => self.m.push_spc(data),
            // IMOD<25:0> and IMOD<47:26>: the OA register merge into the
            // next instruction.
            0o16 => {
                self.oa_low = data as u64 & 0o377777777;
                self.oal = true;
            }
            0o17 => {
                self.oa_high = data as u64 & 0o37777777;
                self.oah = true;
            }
            // VMA, and the three that start a cycle with it
            0o20 => self.m.vma = data,
            0o21 => {
                self.m.vma = data;
                self.start_read();
            }
            0o22 => {
                self.m.vma = data;
                self.start_cycle(true);
                let md = self.m.md;
                self.m.vm_write(self.m.vma, md);
            }
            0o23 => {
                self.m.vma = data;
                let (vma, md) = (self.m.vma, self.m.md);
                self.m.write_map(vma, md);
            }
            // MD, and the three that start a cycle with it
            0o30 => self.m.md = data,
            0o31 => {
                self.m.md = data;
                self.start_read();
            }
            0o32 => {
                self.m.md = data;
                self.start_cycle(true);
                let (vma, md) = (self.m.vma, self.m.md);
                self.m.vm_write(vma, md);
            }
            0o33 => {
                self.m.md = data;
                let (vma, md) = (self.m.vma, self.m.md);
                self.m.write_map(vma, md);
            }
            _ => return Err(Halt::UnknownDest { pc: self.p0_pc, dest }),
        }
        Ok(())
    }

    /// A read, with the diagnostic block answered by this engine and not by
    /// [`Machine`]: what the cpu drives onto `SPY<15:0>` under `-DBREAD` is
    /// its own state, which `Machine` does not have.
    fn read(&mut self, vma: u32) -> u32 {
        let t = self.m.translate(vma);
        if t.access_permitted
            && let Some(eadr) = busint::unibus_address(t.physical).and_then(spy::register)
        {
            self.m.vmaok = true;
            return self.spy_read(eadr) as u32;
        }
        self.m.vm_read(vma)
    }

    /// A memory cycle starts: the latch at VMEMDR 1D14 takes the map word
    /// of the page `VMA` is on, the cycle's direction is kept for the
    /// permission bits, and the clock is charged the mean wait for a
    /// cycle. The word itself moves at once, as it always has
    /// here: this engine has no bus to wait on, only a clock to keep.
    fn start_cycle(&mut self, write: bool) {
        self.lvmo = self.m.translate(self.m.vma).l2_data;
        self.wrcyc = write;
        self.m.ns += self.memory_cycle_ns;
        self.memory_cycles += 1;
    }

    /// A read cycle starts at `VMA`: the cycle's bookkeeping as
    /// [`Micro::start_cycle`] does it, and the word two microcycles later
    /// --- but only if the map permits. On the board `MBUSY` is set from
    /// `MEMSTART AND VMAOK` (page VCTL1), so a refused read requests nothing
    /// and `MD` keeps its word; `-VMAOK` in the flags is what the microcode
    /// tests for the fault.
    fn start_read(&mut self) {
        self.start_cycle(false);
        if self.m.translate(self.m.vma).access_permitted {
            self.new_md = self.read(self.m.vma);
            self.new_md_delay = 2;
        } else {
            self.m.vmaok = false;
        }
    }

    /// How many memory cycles this engine has started.
    pub fn memory_cycles(&self) -> u64 {
        self.memory_cycles
    }

    /// `IR<25>` picks A memory, and the address is `IR<23:14>`: page ACTL's
    /// 25S09s at 3B28 and 3B29 make `WADR<9:0>` from those ten bits, and
    /// `IR24`, which `ir.bits` marks "xx", reaches nothing but the
    /// instruction register's latch at IREG 3C17 and the parity generator
    /// at IPAR 3F24. Otherwise the write goes to a functional destination
    /// *and* to M memory, which shadows the low 32 words of A.
    fn write_dest(&mut self, dest: u16) -> Result<(), Halt> {
        if dest & 0o4000 != 0 {
            self.m.amem[(dest & 0o1777) as usize] = self.out;
        } else {
            self.write_functional(dest, self.out)?;
            self.m.mmem[(dest & 0o37) as usize] = self.out;
            self.m.amem[(dest & 0o37) as usize] = self.out;
        }
        Ok(())
    }
}

/// Rotate left, the machine's only shifter primitive.
fn rol(v: u32, n: u32) -> u32 {
    v.rotate_left(n & 31)
}

impl Micro {
    fn alu(&mut self) -> Result<(), Halt> {
        let dest = self.ir(14, 12) as u16;
        // Page ALUC4 decides what the 74S181s are asked to do; page ALU0-1
        // does it.  Both are shared with the `rtl` engine, so the two cannot
        // drift, and the ALU is checked against its own gate model rather
        // than against either engine.
        let ctl = ttl::alu_control(
            self.p0.raw(),
            self.m.q & 1 != 0,
            self.adata & 0x8000_0000 != 0,
            true,
            false,
        );
        let alu = ttl::alu(self.mdata, self.adata, ctl.aluf, ctl.alumode, ctl.cin);
        self.alu_out = alu.f as u32;

        // Q control, IR<1:0>.
        self.old_q = self.m.q;
        match self.ir(0, 2) {
            1 => {
                self.m.q <<= 1;
                if self.alu_out & 0x8000_0000 == 0 {
                    self.m.q |= 1;
                }
            }
            2 => {
                self.m.q >>= 1;
                if self.alu_out & 1 != 0 {
                    self.m.q |= 0x8000_0000;
                }
            }
            3 => self.m.q = self.alu_out,
            _ => {}
        }

        // Output bus select, IR<13:12>.  The shift-right input is bit 32 of
        // the 33-bit array --- the ninth slice's sign extension --- which is
        // the one carry-out the array has.
        self.out = match self.ir(12, 2) {
            // Not the ALU: the mask and rotate network's output, pages
            // SMCTL, SHIFT0-1, MSKG4 and MO, as the BYTE class drives it.
            // The network reads its rotate from IR<4:0> and its byte length
            // from IR<9:5> whatever the class, so here it rotates and
            // masks by the ALU function, the carry and the Q control, with
            // the A source showing outside the mask.  Microcode 323 has
            // eleven such words, all zero and with no destination;
            // `tests/output_bus.rs` holds all three engines to the network's
            // word on one with a destination.
            0 => {
                let mut rotate = self.ir(0, 5);
                if self.ir(10, 2) == 3 {
                    rotate = self.lc_byte_mode();
                }
                let left = (rotate + self.ir(5, 5)) & 0o37;
                let mask = (!0u32 >> (31 - left)) & (!0u32 << rotate);
                (rol(self.mdata, rotate) & mask) | (self.adata & !mask)
            }
            1 => self.alu_out,
            2 => (alu.f >> 1) as u32,
            _ => (self.alu_out << 1) | (self.old_q >> 31),
        };

        self.write_dest(dest)
    }

    /// Page FLAG's condition mux: `IR<5>` chooses between a bit of the
    /// shifted M source and one of seven conditions, selected by `IR<2:0>`.
    ///
    /// The comparisons are the ALU's, not Rust's.  A jump asserts `ALUSUB`
    /// with no carry in, so the array computes `m - a - 1`, and the mux
    /// selects `AEQM` and bit 32 of the 33-bit result --- the ninth slice's
    /// sign extension, which is what makes the comparisons signed.  Writing
    /// them as signed `i32` comparisons reaches the same answer without the
    /// hardware, and hides where the signedness comes from.
    fn jump_condition(&mut self) -> bool {
        let r = rol(self.mdata, self.ir(0, 5));
        if self.ir(5, 1) == 0 {
            self.mdata = r;
            return r & 1 != 0;
        }
        let ctl = ttl::alu_control(
            self.p0.raw(),
            self.m.q & 1 != 0,
            self.adata & 0x8000_0000 != 0,
            false,
            true,
        );
        let alu = ttl::alu(self.mdata, self.adata, ctl.aluf, ctl.alumode, ctl.cin);
        let alu32 = alu.f >> 32 & 1 != 0;
        let int_enabled = self.m.interrupt_control & (1 << 27) != 0;
        let pending = int_enabled && self.m.interrupt();
        match self.ir(0, 3) {
            0 => r & 1 != 0,
            1 => !alu.aeqm && alu32,
            2 => alu32,
            3 => alu.aeqm,
            4 => !self.m.vmaok,
            5 => !self.m.vmaok || pending,
            6 => !self.m.vmaok || pending || (self.m.interrupt_control & (1 << 26) != 0),
            _ => true,
        }
    }

    fn jump(&mut self) -> Result<(), Halt> {
        let mut target = self.ir(12, 14) as u16;
        let r = self.ir(9, 1) != 0;
        let p = self.ir(8, 1) != 0;
        let n = self.ir(7, 1) != 0;
        let invert = self.ir(6, 1) != 0;

        // `IR<11:10>` = 1 here is `HALT-CONS`, `cadsym.lisp`'s `1_10.`,
        // which microcode 323 writes at `ZERO`, `ILLOP` and `%HALT`: the
        // jump is taken as any other, and the halt is `HALTED`'s, set after
        // the instruction ([`Micro::step`]).
        // P and R together on a JUMP is not a jump: it writes the control
        // store from the A and M sources.
        //
        // **The board pushes and then pops, and so does this.** Page CONTRL:
        // the 74S64 at 3E26 makes `-SPUSH` from four AND groups, the first
        // `IRJUMP AND -IR6 AND IR8 AND JCOND`, and **no group has `IWRITE`
        // in it** --- `IWRITE` is decoded on its own at the 74S11 3E29,
        // `IRJUMP AND IR8 AND IR9`, and never reaches 3E26. A control-store
        // write is a jump-always with P, so that first group is satisfied
        // and the machine pushes. On the next cycle the 74S175 at 3D26 has
        // `IWRITED`, the open-collector 74S08 at 3D21 gives
        // `-POPJ = -IPOPJ AND -IWRITED`, and it pops.
        //
        // So the stack pointer ends where it began with the pushed word
        // still in the slot above it, which is what a console reading the
        // stack sees. Read off `data/CADR.netlist` and confirmed pin for pin
        // against MIT's own wire list `cadrwd/cadr4.wlr`; no other
        // implementation is cited, and none is needed.
        //
        // It really costs two microcycles and this engine spends neither in
        // the pipeline: `IWRITED` drives `N` as well as `POPJ`, so the board
        // loses one cycle to this instruction's `N` and another to
        // `IWRITED`'s. Inhibiting two cycles here would kill the two
        // instructions after the write rather than returning to the first of
        // them, so the cycles are charged to the clock instead and the
        // pipeline is left alone.
        if p && r {
            self.m.imem[target as usize & (crate::machine::IMEM_WORDS - 1)] = Insn::new(self.iwr);
            if !invert && self.jump_condition() {
                let ret = if n { self.npc.wrapping_sub(1) } else { self.npc } & 0o37777;
                self.m.push_spc(ret as u32);
                self.m.pop_spc();
            }
            // The two microcycles the board spends on it, both nopped and
            // so never long, go on the clock at the next step, where the
            // board spends them: `micro_keeps_the_machines_periods` parted
            // from `rtl` by exactly two boot-speed cycles at
            // `CLEAR-I-MEMORY` without them.
            self.nopped = 2;
            return Ok(());
        }

        let cond = self.jump_condition() != invert;
        if p && cond {
            let ret = if n { self.npc.wrapping_sub(1) } else { self.npc } & 0o37777;
            self.m.push_spc(ret as u32);
        }
        if r && cond {
            let mut t = self.m.pop_spc();
            if (t >> 14) & 1 != 0 {
                t = self.advance_lc(t)?;
            }
            target = (t & 0o37777) as u16;
        }
        if cond {
            if n {
                self.inhibit = true;
            }
            self.npc = target;
            // Page CONTRL: `PCS0` is `NOT(POPJ OR ...)`, so with the POPJ bit
            // up a taken jump still takes its next address off the stack,
            // which `step` does after the instruction; a return has popped
            // it already.
            if r {
                self.popj = false;
            }
        }
        Ok(())
    }

    fn dispatch(&mut self) -> Result<(), Halt> {
        let mut pos = self.ir(0, 5);
        let len = self.ir(5, 3);
        let map = self.ir(8, 2);
        let mut addr = self.ir(12, 11);
        let use_lpc = self.ir(25, 1) != 0;
        let advance = self.ir(24, 1) != 0;

        // Misc 2 writes the dispatch memory instead of dispatching.
        //
        // Seventeen bits, not thirty-two: the DRAM pages hold `DPC<13:0>`,
        // `DN`, `DP` and `DR` and nothing else, which `tests/chip.rs` reads
        // off the netlist and enforces against `rtl`. The boot PROM's
        // `CLEAR-D-MEMORY` writes "0 with good parity" --- bit 17 --- into
        // every word, so an engine that keeps the whole word holds 2048
        // entries no board ever held.
        if self.ir(10, 2) == 2 {
            self.m.dmem[addr as usize] = self.adata & 0o377777;
            return Ok(());
        }
        if self.ir(10, 2) == 3 {
            pos = self.lc_byte_mode();
        }

        let m = rol(self.mdata, pos);
        let mask = if len == 0 { 0 } else { !0u32 >> (31 - ((len - 1) & 0o37)) };

        // Level-2 map bits.  The CADR documentation says 14 and 15; the
        // hardware uses 18 and 19 (discrepancy 3).
        //
        // A map bit takes address bit 0 *instead of* the field's: the
        // 74S64s at 2F24, 2F05 and 2F23 make `-DADR0` from `VMO18 AND IR8`,
        // `VMO19 AND IR9`, `-DMAPBENB AND DMASK0 AND R0` and `IR12`, with
        // `-DMAPBENB = NOR(IR8, IR9)` at 3F14 gating the field's bit 0 out
        // whenever a map bit is selected.  ORing the map bit into the
        // field's is the easy misreading of that NAND-OR; the bit takes
        // the place, as `rtl` and `chip` show.
        if map != 0 {
            let bits = self.m.translate(self.m.md).l2_data;
            let b18 = (bits >> 18) & 1;
            let b19 = (bits >> 19) & 1;
            addr |= (m & mask & !1)
                | match map {
                    1 => b18,
                    2 => b19,
                    _ => b18 | b19,
                };
        } else {
            addr |= m & mask;
        }

        let entry = self.m.dmem[(addr & 0o3777) as usize];
        self.m.dispatch_constant = self.ir(32, 10) as u16;

        let mut target = entry & 0o37777;
        let n = (entry >> 14) & 1 != 0;
        let p = (entry >> 15) & 1 != 0;
        let r = (entry >> 16) & 1 != 0;

        // The address a push would save --- page CONTRL's `RETA`: `PC + 1`,
        // under `N` the inhibited slot's own address, and with `IR<25>` the
        // `LPC` of the instruction-stream hardware, one behind that.
        let ret = if n {
            let pc = self.npc.wrapping_sub(1);
            if use_lpc { pc.wrapping_sub(1) } else { pc }
        } else {
            self.npc
        } & 0o37777;
        if advance {
            self.advance_lc(0)?;
        }
        if n {
            self.inhibit = true;
        }
        // `DFALL = DR AND DP` on page CONTRL: R and P together are neither,
        // and the next address is `PC + 1`, nothing pushed or popped.
        if p && r {
            return Ok(());
        }
        if p {
            self.m.push_spc(ret as u32);
        }
        if r {
            let mut t = self.m.pop_spc();
            if (t >> 14) & 1 != 0 {
                t = self.advance_lc(t)?;
            }
            target = t & 0o37777;
        }
        self.npc = target as u16;
        self.popj = false;
        Ok(())
    }

    fn byte(&mut self) -> Result<(), Halt> {
        let dest = self.ir(14, 12) as u16;
        let func = self.ir(12, 2);
        let mut pos = self.ir(0, 5);
        if self.ir(10, 2) == 3 {
            pos = self.lc_byte_mode();
        }

        let width_minus_1 = self.ir(5, 5);
        let right = if func & 2 != 0 { pos } else { 0 };
        let left = (right + width_minus_1) & 0o37;
        let mask = (!0u32 >> (31 - left)) & (!0u32 << right);

        // Page SMCTL: `IR<12>` rotates (`SR`) and `IR<13>` places the mask
        // (`MR`), so LDB and DPB rotate, selective deposit and function 0 do
        // not; and every BYTE puts the mask network's word on the bus, `OSEL`
        // being 0 on the class.
        let m = if func & 1 != 0 { rol(self.mdata, pos) } else { self.mdata };
        self.out = (m & mask) | (self.adata & !mask);
        self.write_dest(dest)
    }
}

impl Engine for Micro {
    /// The boot sequence: reset, enable the PROM, then trap to control store
    /// location 0.  The trap forces NPC to zero and inhibits the unfetched
    /// instruction still sitting in the pipeline, exactly as a parity trap
    /// would.
    fn boot(&mut self) {
        self.m.vmaok = false;
        // `-RESET` clears the console's registers, the mode register among
        // them, which is where `PROMDISABLE` lives; the PROM is back over the
        // bottom of the control store.  `-BOOT` presets `RUN`.
        self.m.reset_console_registers();
        self.m.clock_control.run = true;
        self.srun = true;
        self.npc = 0;
        self.inhibit = true;
    }
    fn save(&self, w: &mut crate::checkpoint::Writer) {
        let Micro {
            m,
            p0,
            p0_pc,
            p1,
            p1_pc,
            npc,
            inhibit,
            popj,
            oal,
            oah,
            oa_low,
            oa_high,
            new_md,
            new_md_delay,
            aaddr,
            maddr,
            adata,
            mdata,
            alu_out,
            old_q,
            out,
            iwr,
            executed,
            lvmo,
            wrcyc,
            speed,
            speed_a,
            memory_cycle_ns,
            memory_cycles,
            nopped,
            srun,
            sstep,
            ssdone,
            halted,
        } = self;
        m.save(w);
        w.u64(p0.raw());
        w.u16(*p0_pc);
        w.u64(p1.raw());
        w.u16(*p1_pc);
        w.u16(*npc);
        w.bool(*inhibit);
        w.bool(*popj);
        w.bool(*oal);
        w.bool(*oah);
        w.u64(*oa_low);
        w.u64(*oa_high);
        w.u32(*new_md);
        w.u8(*new_md_delay);
        w.u16(*aaddr);
        w.u8(*maddr);
        w.u32(*adata);
        w.u32(*mdata);
        w.u32(*alu_out);
        w.u32(*old_q);
        w.u32(*out);
        w.u64(*iwr);
        w.opt(*executed, crate::checkpoint::Writer::u16);
        w.u32(*lvmo);
        w.bool(*wrcyc);
        w.speed(*speed);
        w.speed(*speed_a);
        w.u64(*memory_cycle_ns);
        w.u64(*memory_cycles);
        w.u8(*nopped);
        w.bool(*srun);
        w.bool(*sstep);
        w.bool(*ssdone);
        w.bool(*halted);
    }

    fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        self.m.load(r)?;
        self.p0 = Insn::new(r.u64()?);
        self.p0_pc = r.u16()?;
        self.p1 = Insn::new(r.u64()?);
        self.p1_pc = r.u16()?;
        self.npc = r.u16()?;
        self.inhibit = r.bool()?;
        self.popj = r.bool()?;
        self.oal = r.bool()?;
        self.oah = r.bool()?;
        self.oa_low = r.u64()?;
        self.oa_high = r.u64()?;
        self.new_md = r.u32()?;
        self.new_md_delay = r.u8()?;
        self.aaddr = r.u16()?;
        self.maddr = r.u8()?;
        self.adata = r.u32()?;
        self.mdata = r.u32()?;
        self.alu_out = r.u32()?;
        self.old_q = r.u32()?;
        self.out = r.u32()?;
        self.iwr = r.u64()?;
        self.executed = r.opt(crate::checkpoint::Reader::u16)?;
        self.lvmo = r.u32()?;
        self.wrcyc = r.bool()?;
        self.speed = r.speed()?;
        self.speed_a = r.speed()?;
        self.memory_cycle_ns = r.u64()?;
        self.memory_cycles = r.u64()?;
        self.nopped = r.u8()?;
        self.srun = r.bool()?;
        self.sstep = r.bool()?;
        self.ssdone = r.bool()?;
        self.halted = r.bool()?;
        Ok(())
    }

    fn step(&mut self) -> Result<(), Halt> {
        self.executed = None;
        // `MACHRUN`, less the statistics halt this engine cannot raise, and
        // with the one `ERR` it can: no parity check, but `HALTED` under
        // `ERRSTOP`.  Halted, a step is one master clock cycle and no
        // microcycle.
        let errhalt = self.m.mode.errstop && self.halted;
        let machrun = (self.sstep && !self.ssdone) || (self.srun && !errhalt);
        if !machrun {
            self.speedclk();
            self.m.ns += self.speed.cycle_ns(false) as u64;
            self.mclk_edge();
            return Ok(());
        }
        // No clock phases here, but the machine's periods: each microcycle
        // is as long as the speed bits and `ILONG` make it, 220 ns through
        // the boot and 145 once microcode 323 writes the mode register, and
        // an inhibited instruction is nopped and so never long. What is not
        // here is the time the machine waits on the bus; `rtl` has that,
        // and `memory_cycle_ns` stands in for it with the band's mean.
        while self.nopped > 0 {
            self.speedclk();
            self.m.ns += self.speed.cycle_ns(false) as u64;
            self.nopped -= 1;
            self.mclk_edge();
        }
        self.speedclk();
        // `ILONG` is `IR<45> AND -NOP` on page CLOCK1, and `NOP` is the
        // inhibit or the console's `NOP11`.
        let nop = self.inhibit || self.m.clock_control.nop11;
        let ilong = !nop && self.p1.raw() >> 45 & 1 != 0;
        self.m.ns += self.speed.cycle_ns(ilong) as u64;
        self.mclk_edge();
        self.advance_pipeline();

        if self.new_md_delay > 0 {
            self.new_md_delay -= 1;
            if self.new_md_delay == 0 {
                self.m.md = self.new_md;
            }
        }

        // A jump with N set kills the instruction already in the pipeline,
        // and the console's `NOP11` kills every one.
        if self.inhibit || self.m.clock_control.nop11 {
            self.inhibit = false;
            // Nopped, the instruction's misc field decodes to nothing.
            self.halted = false;
            self.m.cycles += 1;
            return Ok(());
        }

        self.executed = Some(self.p0_pc);

        // The OA registers modify the instruction as it is executed.
        if self.oal {
            self.oal = false;
            self.p0 = Insn::new(self.p0.raw() | self.oa_low);
        }
        if self.oah {
            self.oah = false;
            self.p0 = Insn::new(self.p0.raw() | (self.oa_high << 26));
        }

        self.popj = self.p0.popj();
        self.aaddr = self.ir(32, 10) as u16;
        self.maddr = self.ir(26, 5) as u8;
        self.mdata = if self.p0.m_src_functional() {
            self.read_functional(self.maddr)?
        } else {
            self.m.mmem[self.maddr as usize]
        };
        self.adata = self.m.amem[self.aaddr as usize];
        self.iwr = ((self.adata as u64 & 0o177777) << 32) | self.mdata as u64;

        match self.p0.op() {
            Op::Alu => self.alu()?,
            Op::Jump => self.jump()?,
            Op::Dispatch => self.dispatch()?,
            Op::Byte => self.byte()?,
        }
        // `IR<11:10>` = 1 on any class is misc function 1, `HALT-CONS`:
        // `-FUNCT1` off the 74S139 at SOURCE 3D05, which the 74S374 at
        // OLORD2 1A05 registers as `HALTED` at the next edge.
        self.halted = self.ir(10, 2) == 1;

        if self.popj {
            let mut t = self.m.pop_spc();
            if (t >> 14) & 1 != 0 {
                t = self.advance_lc(t)?;
            }
            self.npc = (t & 0o37777) as u16;
        }

        self.m.cycles += 1;
        Ok(())
    }

    fn pc(&self) -> u16 {
        self.npc
    }

    /// What this engine can answer of the sixteen, which is less than
    /// `rtl`: it has no read phase apart from execution, so `OB`, `A` and
    /// `M` are the last microcycle's and not the standing instruction's;
    /// `OPC` is one register deep, the PC of the instruction just executed;
    /// there is no statistics counter, no write pipeline behind the six
    /// registered flags, and no `JCOND` or `PCS` outside a jump.  `IR` and
    /// `PC` are the instruction waiting to execute and the address after
    /// it, which is what `rtl` holds in `IR` and `PC` between microcycles.
    fn spy_read(&self, eadr: u8) -> u16 {
        let half = |v: u64, k: u8| (v >> (16 * k as u32)) as u16;
        match eadr {
            spy::IR_LOW | spy::IR_MED | spy::IR_HIGH => half(self.p1.raw(), eadr),
            spy::OPC => self.m.opc & 0x3fff,
            spy::PC => self.npc & 0x3fff,
            spy::OB_LOW => self.out as u16,
            spy::OB_HIGH => (self.out >> 16) as u16,
            spy::FLAG_1 => spy::Flag1 {
                promdisable: self.m.mode.prom_disable,
                err: self.halted,
                ssdone: self.ssdone,
                srun: self.srun,
                ..Default::default()
            }
            .word(),
            spy::FLAG_2 => spy::Flag2 {
                nop: self.inhibit || self.m.clock_control.nop11,
                vmaok: self.m.vmaok,
                ..Default::default()
            }
            .word(),
            spy::M_LOW => self.mdata as u16,
            spy::M_HIGH => (self.mdata >> 16) as u16,
            spy::A_LOW => self.adata as u16,
            spy::A_HIGH => (self.adata >> 16) as u16,
            spy::STAT_LOW | spy::STAT_HIGH => 0,
            _ => spy::OPEN_READ,
        }
    }

    fn machine(&self) -> &Machine {
        &self.m
    }

    fn machine_mut(&mut self) -> &mut Machine {
        &mut self.m
    }
}
