// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The state every engine shares: the memories, the registers, and the
//! two-level virtual memory map.
//!
//! The map geometry is read off the VMEM pages of the netlist, and the field
//! positions off `mit/cadr/ir.bits`; each is named where it is used.  Where the
//! drawings and the MIT documentation disagree, the drawings win and the
//! disagreement is named where it bites.

use crate::busint;
use crate::disk_controller::{self, Controller};
use crate::ioboard::{self, IoBoard};
use crate::isa::Insn;
use crate::spy;
use crate::tv::{self, Tv};

/// Control store size: 16K words, fourteen bits of PC.
pub const IMEM_WORDS: usize = 16 * 1024;
/// Boot PROM size: 1K words, overlaying the bottom of the control store
/// until the mode register's `PROMDISABLE` bit is set.
///
/// Twelve 74S472s, 512 by 8 each, on pages PROM0 and PROM1 of
/// `data/CADR.netlist` hold two banks of 512 words, and page PCTL decodes
/// them: `BOTTOM.1K = NOR(PC13, PC12, PC11, PC10)` at the 74S260 1D18,
/// `-PROMENABLE = NAND(BOTTOM.1K, -IDEBUG, -PROMDISABLED, -IWRITEDA)` at
/// the 74S20 1C19, and the two chip enables at the 74S32 1C18, `-PROMCE0 =
/// OR(-PROMENABLE, PC9)` for the first bank and `-PROMCE1 =
/// OR(-PROMENABLE, -PROMPC9)` for the second. `mit/cadr/ir.bits` says the
/// same of `PROMDISABLE`: "0 first 1K I memory is PROM". MIT's image fills
/// 454 words of the first bank and the boot never leaves it; the second
/// bank goes unused, and every engine reads the rest of the 1K as zero.
pub const PROM_WORDS: usize = 1024;
/// Main memory by default, in 32-bit words: thirty-two boards of 64K.
/// [`Machine::with_memory_boards`] builds a machine with another count.
/// Physical pages above the memory are devices, or nothing.
pub const MAIN_WORDS: usize = 2 * 1024 * 1024;

/// Bits of the bus error status, which the console reads over SPY and the
/// microcode tests. Three bits can be set here --- the two NXM bits and the
/// Unibus map error; the rest of the register is parity errors, which
/// cannot happen here. The wording and the bit values are MIT's own, the
/// register's description at the head of System 100's `sys/cc/ldbg.lisp`.
pub mod bus_error {
    /// "Xbus NXM Error. Set when an Xbus cycle times out for lack of
    /// response." --- `ldbg.lisp`, bit value 1.  `XB NXM ERROR` on the
    /// 74276 at the interface's REQERR 0B02, set by `-NXM TIMEOUT` with `XBUS
    /// REQUEST`, and read through the 8304 at 0B15.
    pub const XBUS_NXM: u16 = 0o1;
    /// "Unibus NXM Error. Set when a Unibus cycle times out for lack of
    /// response." --- `ldbg.lisp`, bit value 10.  `UB NXM ERROR` on the
    /// same 74276, with `UNIBUS REQUEST`, and the same 8304.
    pub const UNIBUS_NXM: u16 = 0o10;
    /// "Unibus Map Error. Set when an attempt to perform an Xbus cycle
    /// through the Unibus map is refused because the map specifies invalid
    /// or write-protected." --- the head of `ldbg.lisp`.  The 74LS74 at
    /// REQERR 0D03, clocked by `UB XBUS T100`.
    pub const UB_MAP_ERROR: u16 = 0o40;
}

/// The widths of the map and the PDL buffer: what differs between the
/// machines `--machine` chooses, `cadr` and `quux`.
///
/// The CADR's are MIT's (`SIZE-OF-HARDWARE-LEVEL-1-MAP`, `-LEVEL-2-MAP` and
/// `-PDL-BUFFER` in System 100's `sys/cold/qcom.lisp`; the netlist's RAMs,
/// `chip_and_rtl_hold_the_same_memories` in `tests/chip.rs`): a level-1
/// entry of five bits, the number of a block of 32 level-2 entries, and a
/// PDL pointer and index of ten.  A level-2 block is 32 entries on every
/// machine, `VMA<12:8>` choosing one in it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Geometry {
    /// Bits in a level-1 map entry.
    pub l1_bits: u32,
    /// Bits in the PDL buffer's pointer and index.
    pub pdl_bits: u32,
    /// What the machine answers in functional source 16, if anything: QUUX's
    /// MACHINE-ID. The CADR drives nothing there and reads all ones.
    pub machine_id: Option<u32>,
    /// Whether ALU functions 42 and 43 are QUUX's one-instruction multiply
    /// and divide ([`crate::muldiv`]) rather than the CADR's.
    pub muldiv: bool,
    /// Whether the processor has QUUX's tick ([`Tick`]): functional
    /// destinations 3 and 4, source 17.
    pub tick: bool,
    /// Whether the mode register has `SPEED1` and `SPEED0`, which choose the
    /// delay-line tap that ends the read phase (`mit/cadr/ir.bits`). The
    /// CADR's do; QUUX runs at one rate and has no such bits.
    pub speed_bits: bool,
    /// Whether a RAM read in the microcycle that writes the same word ---
    /// a `POPJ` in a dispatch memory write, the map read in the microcycle
    /// its store's write lands --- gives the word from before the write.
    /// QUUX defines it so, as an FPGA's block RAM gives it. On the CADR the
    /// RAM's output floats while written and the board races; muir takes
    /// the netlist's answer there, the word written
    /// (`tests/dispatch_write_order.rs`).
    pub old_word_while_written: bool,
    /// Whether a microcycle that reads `MD` while a read is in flight is
    /// hung, as on the CADR: its read phase and write pulses run, then
    /// `-HANG` holds the next cycle's start until `-RDFINISH`. QUUX has no
    /// hung microcycle: such a microcycle waits, as for `-WAIT`, whole
    /// microcycles with no write pulse, and runs once the word is in `MD`.
    pub hangs: bool,
}

impl Geometry {
    /// The CADR's.
    pub const CADR: Geometry = Geometry {
        l1_bits: 5,
        pdl_bits: 10,
        machine_id: None,
        muldiv: false,
        tick: false,
        speed_bits: true,
        old_word_while_written: false,
        hangs: true,
    };

    /// QUUX's, revision 4: a tick in the processor ([`Tick`]); `MUL` and
    /// `DIV` in one instruction each, ALU
    /// functions 42 and 43 ([`crate::muldiv`]); a PDL buffer of 16K words,
    /// its pointer and index 14 bits; and a level-1 entry of six bits, 64 blocks of level 2 and so 63
    /// regions of 8K words mapped at once against the CADR's 31, the last
    /// block being the invalid one. The sixth bit is carried by the two the
    /// CADR leaves spare: `MAP(MD)<29>`, which the CADR drives low (VMEMDR
    /// 1A01, `HI12` through a 74S240), and `VMA<24>`, which no map write
    /// takes. The rest of the machine is the CADR's.
    ///
    /// It says so in functional source 16, its MACHINE-ID, which no microcode of MIT's
    /// reads and nothing on the CADR drives: the signature `0x5155` in bits
    /// 31:16, the hardware revision in 15:4 --- 4: the six-bit map, then the
    /// 16K PDL buffer, then the multiply and divide, then the tick --- and
    /// the processor type, 4, in 3:0. A CADR's open bus reads all ones there,
    /// which can never carry the signature.
    pub const QUUX: Geometry = Geometry {
        l1_bits: 6,
        pdl_bits: 14,
        machine_id: Some((0x5155 << 16) | (4 << 4) | 4),
        muldiv: true,
        tick: true,
        speed_bits: false,
        old_word_while_written: true,
        hangs: false,
    };

    /// The level-1 entry a map store writes: `VMA<31:27>` on every machine
    /// (`mit/cadr/ir.bits`, "VMA<26>=1 writes the level 1 map from
    /// VMA<31-27>"), and on QUUX `VMA<24>` as its sixth bit.
    pub fn l1_from_vma(self, vma: u32) -> u32 {
        let low = (vma >> 27) & 0o37;
        if self.l1_bits > 5 { low | ((vma >> 24) & 1) << 5 } else { low }
    }

    /// A level-1 entry's bits.
    pub fn l1_mask(self) -> u32 {
        (1 << self.l1_bits) - 1
    }

    /// The level-2 entry a level-1 entry and an address select: the block
    /// the entry names, and `VMA<12:8>` in it.
    pub fn l2_index(self, l1: u32, addr: u32) -> usize {
        (((l1 & self.l1_mask()) << 5) | ((addr >> 8) & 0o37)) as usize
    }

    /// The PDL pointer's and index's bits.
    pub fn pdl_mask(self) -> u16 {
        (1 << self.pdl_bits) - 1
    }

    /// The Xbus I/O page a machine with a MACHINE-ID lists its sizes
    /// in: physical `17377000`, just below the page the display's control
    /// registers and the disk controller share. Nothing answers there on
    /// the CADR.
    pub const FEATURE_PAGE: u32 = 0o36776;

    /// The word of the feature page at physical address `phys`, if this
    /// machine has one and `phys` is on it: the MACHINE-ID, then the
    /// level-1 entry's bits, the level-2 map's entries, the PDL buffer's
    /// words, the control store's, A memory's and dispatch memory's, and
    /// which of `MUL` (bit 0) and `DIV` (bit 1) it has, and whether it has
    /// the tick (1); words 11 to 13, the main screen, are the display's
    /// ([`Machine::bus_read`]); every other word 0.
    /// Read-only.
    pub fn feature_word(self, phys: u32) -> Option<u32> {
        let id = self.machine_id?;
        if (phys >> 8) & 0o37777 != Self::FEATURE_PAGE {
            return None;
        }
        Some(match phys & 0o377 {
            0 => id,
            1 => self.l1_bits,
            2 => 32 << self.l1_bits,
            3 => 1 << self.pdl_bits,
            4 => IMEM_WORDS as u32,
            5 => 1024,
            6 => 2048,
            7 => (self.muldiv as u32) * 3,
            0o10 => self.tick as u32,
            _ => 0,
        })
    }
}

/// QUUX's tick: a periodic flag in the processor, the machine's clock in
/// place of the CADR display's vertical interrupt (revision 4).
///
/// Functional destination 3 is its control: `<0>` enables it, and a write
/// with `<1>` set clears the flag. Destination 4 is its period in
/// microseconds, `<23:0>`, 0 taken as 1. Functional source 17 reads `<0>`
/// the flag and `<1>` the enable. The flag rises a period after the tick is
/// enabled or its period written, and then every period, whether or not it
/// was cleared in between; while enabled and up it is part of
/// [`Machine::interrupt`]. The period starts at [`Tick::PERIOD_US`].
///
/// None of this is the CADR's: page SOURCE decodes no destination 3 or 4
/// and no source 17. Microcode 323 neither writes the one nor reads the
/// other, by a scan of every control-store word and by running it with
/// every executed word read as the OA registers left it
/// (`tests/unused_codes.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    pub enabled: bool,
    pub period_us: u32,
    /// When the flag next rises, or rose and has not been cleared.
    pub deadline_ns: u64,
}

impl Tick {
    /// 60 Hz, near enough: what the display's vertical interrupt was.
    pub const PERIOD_US: u32 = 16_667;

    pub const fn new() -> Tick {
        Tick { enabled: false, period_us: Self::PERIOD_US, deadline_ns: u64::MAX }
    }

    fn period_ns(self) -> u64 {
        self.period_us.max(1) as u64 * 1000
    }

    /// The flag, at `now`.
    pub fn flag(self, now: u64) -> bool {
        now >= self.deadline_ns
    }

    /// Source 17 at `now`.
    pub fn status(self, now: u64) -> u32 {
        (self.enabled as u32) << 1 | self.flag(now) as u32
    }

    /// A write of destination 3 at `now`.
    pub fn control(&mut self, now: u64, v: u32) {
        let enable = v & 1 != 0;
        if enable && !self.enabled {
            self.deadline_ns = now + self.period_ns();
        } else if v & 2 != 0 && self.flag(now) {
            // The next period boundary after `now`.
            let p = self.period_ns();
            self.deadline_ns += (now - self.deadline_ns) / p * p + p;
        }
        if !enable {
            self.deadline_ns = u64::MAX;
        }
        self.enabled = enable;
    }

    /// A write of destination 4 at `now`.
    pub fn period(&mut self, now: u64, v: u32) {
        self.period_us = v & 0o77777777;
        if self.enabled {
            self.deadline_ns = now + self.period_ns();
        }
    }

    pub fn save(self, w: &mut crate::checkpoint::Writer) {
        w.bool(self.enabled);
        w.u32(self.period_us);
        w.u64(self.deadline_ns);
    }

    pub fn load(r: &mut crate::checkpoint::Reader) -> std::io::Result<Tick> {
        Ok(Tick { enabled: r.bool()?, period_us: r.u32()?, deadline_ns: r.u64()? })
    }
}

impl Default for Tick {
    fn default() -> Tick {
        Tick::new()
    }
}

/// The PDL buffer's words on the largest machine: a QUUX with a 14-bit
/// pointer, 16K words.
pub const PDL_WORDS: usize = 16 * 1024;

/// The level-2 map's words on the largest machine, QUUX: 64 blocks of 32.
pub const L2_MAP_WORDS: usize = 2048;

/// Why a microcycle could not complete.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Halt {
    /// A functional destination an engine does not implement.  No engine
    /// raises it now: every code decodes as page SOURCE decodes it, the
    /// unassigned ones included.
    UnknownDest { pc: u16, dest: u16 },
}

/// The location counter itself, `LC<25:0>`: the 74S169 counters on page LC
/// and nothing else.
///
/// Both engines keep other things in the same word or beside it ---
/// `Machine::lc` carries NEED-FETCH in bit 31 and the interrupt-control
/// flags in 29:26, and `rtl` holds the byte-mode flags on page FLAG --- so
/// this is the mask that leaves the register the engines can be held to.
pub const LC_COUNTER: u32 = 0o377777777;

#[derive(Clone)]
pub struct Machine {
    pub prom: Vec<Insn>,
    pub imem: Vec<Insn>,
    /// The diagnostic bus's mode register, OLORD1 1A04.  `PROMDISABLE` is the
    /// bit that ends the boot.
    pub mode: spy::Mode,
    /// The clock control register, OLORD1 1A14 and 1A09: `RUN`, `STEP`,
    /// `NOP11`, `IDEBUG` and `LDSTAT`, which is how a console halts, steps
    /// and drives the machine.  See [`spy::ClockControl`].
    pub clock_control: spy::ClockControl,
    /// The OPC control register, OLORD1 1A08.  See [`spy::OpcControl`].
    pub opc_control: spy::OpcControl,
    /// The debug IR, the six 74S374s on page DEBUG: 48 bits a console loads
    /// in three halves, which `IDEBUG` puts on the I bus in place of the
    /// control store.
    pub debug_ir: u64,
    /// `-PROG.RESET` has been pulsed by a mode-register write with
    /// [`spy::MODE_RESET`] set, and the engine has not yet taken it: the
    /// pulse is asynchronous on the board, and the engines act on it at the
    /// master clock edge that ends the cycle it falls in.  `rtl` raises it
    /// from the write pulse's leading edge; `micro`, whose writes land at
    /// once, through [`Machine::spy_write`].
    pub prog_reset: bool,
    /// `PROG.BOOT` likewise, [`spy::MODE_BOOT`].
    pub prog_boot: bool,

    // The sizes of these five and of the two map levels are MIT's own
    // `SIZE-OF-HARDWARE-*` constants in System 100's `sys/cold/qcom.lisp`,
    // in octal: A memory 2000, M memory 40 on the CADR, dispatch memory 4000,
    // PDL buffer 2000, micro stack 40, level-1 map 4000, level-2 map 2000.
    // `chip_and_rtl_hold_the_same_memories` in `tests/chip.rs` holds them to
    // the netlist's RAMs.
    pub amem: [u32; 1024],
    pub mmem: [u32; 32],
    pub dmem: [u32; 2048],
    /// The PDL buffer: 1,024 words on the CADR, and room for the largest
    /// QUUX's, [`PDL_WORDS`]; [`Geometry::pdl_bits`] says how much of it the
    /// machine has.
    pub pdl: [u32; PDL_WORDS],
    pub spc: [u32; 32],

    /// 5 bits.
    pub spcptr: u8,
    /// 10 bits.
    pub pdl_pointer: u16,
    /// 10 bits.
    pub pdl_index: u16,
    pub q: u32,
    /// The PC of the instruction that just executed.
    pub opc: u16,
    /// Location counter.  The 26 bits of address [`LC_COUNTER`] leaves, plus
    /// NEED-FETCH in bit 31 and the interrupt-control flags mirrored in bits
    /// 29:26.
    pub lc: u32,
    pub vma: u32,
    pub md: u32,
    pub interrupt_control: u32,
    /// `IR<41:32>` of the last DISPATCH, readable as functional source 0.
    pub dispatch_constant: u16,

    /// The widths of the map and the PDL buffer, [`Geometry::CADR`] unless
    /// the run chose another machine.
    pub geometry: Geometry,
    /// 2048 five-bit entries, addressed by `VMA<23:13>`.
    pub l1_map: [u32; 2048],
    /// 24-bit entries, addressed by the level-1 output and `VMA<12:8>`:
    /// 1024 on the CADR, and room for the largest machine's, QUUX's 2048.
    pub l2_map: [u32; L2_MAP_WORDS],
    pub main: Vec<u32>,
    /// What the last bus cycles left in the error register; see
    /// [`bus_error`]. A write of the error status register clears it,
    /// in `Machine::interface_write`.
    pub bus_error: u16,
    /// The bus interface's interrupt status register, `766040` and
    /// `766042`: [`busint::interrupt_status`] has the bits.
    pub interrupt_status: u16,
    /// `WRITE THROUGH ENB`, bit 7 of the error status register.
    pub write_through: bool,
    /// The sixteen Unibus map registers at `766140`-`766176`: 29701s at
    /// UBMAP 0E12-0E15, read back through the 74LS244s at 0E16 and 0E17.
    /// "Bit 15 of the register signifies that mapping of that page is
    /// turned on ... Bit 14 enables write access from the unibus. The
    /// remainder of the register is the page number" --- `unaddr.text`.
    /// The one other master of the Unibus, the debug cable's
    /// ([`busint::DebugRequest`]), goes through it: [`Machine::mapped_read`]
    /// and [`Machine::mapped_write`].  The processor's own Unibus cycles do
    /// not; `busint::decode` still gives `140000`-`177777` to nothing for
    /// them.  Only the debug master's cycles are mapped.
    pub unibus_map: [u16; 16],
    /// The read buffers, the 29701s at RBUF 0D23-0D26: one per mapped
    /// Unibus page, holding `BUS<31:16>` of the last Xbus word read through
    /// that page, which the read of the page's odd Unibus word gives
    /// without another Xbus cycle.  "The bus interface actually stores half
    /// of the xbus word and usually accesses the main memory only once for
    /// each pair of unibus operations" --- `unaddr.text`.
    pub read_buffer: [u16; 16],
    /// The write buffers, the 29701s on page WBUF: the low half written to
    /// the even Unibus word, waiting for the odd one to make the Xbus word.
    pub write_buffer: [u16; 16],
    /// Whether the last mapped access was permitted.
    pub vmaok: bool,
    /// The disk controller, at `0o17377774`-`0o17377777` on the Xbus.  The
    /// board is always there; whether a drive is plugged into it is
    /// [`Controller::attach`].
    pub disk: Controller,
    /// The Chaosnet: this machine's address, and the link its cable
    /// reaches the rest of the network over.
    pub chaos: crate::chaos::Config,
    /// The display board, whichever of the two `--tv-board` named:
    /// [`crate::tv::Board`].
    pub tv: Tv,
    /// **The color TV**, the second display board, when `--color-tv`
    /// fitted one: a LISPM TV strapped to [`tv::COLOR_TV`], `17200000` and
    /// `17377750`.  `None` is a machine with one screen, which is what a
    /// CADR has unless somebody plugged a second board in, and is what
    /// `COLOR-EXISTS-P` finds when it probes.
    pub color_tv: Option<Tv>,
    /// The keyboard, the mouse and the clocks.
    pub ioboard: IoBoard,

    pub cycles: u64,
    /// Simulated nanoseconds, kept by the engine that owns this machine:
    /// `rtl` and `chip`'s cable set it to the instant a bus cycle is
    /// acknowledged, off their own clocks, and `micro`, which has no clock,
    /// to its microcycles at the nominal 145 ns. The I/O board's microsecond
    /// and 60-cycle clocks count it. Microcycles times a constant, kept
    /// inside the board, is not time at any speed but one, and is not
    /// advanced at all by the far end of `chip`'s cables.
    pub ns: u64,
    /// QUUX's tick, where the geometry has one.
    pub tick: Tick,
    /// The disk controller has been written since the engine last looked,
    /// and may have written main memory: what invalidates QUUX's memory
    /// cache ([`crate::cache`]).
    pub dma_written: bool,
    /// QUUX's block-disk, when it is fitted in the CADR controller's place
    /// ([`crate::block_disk`]).
    pub block_disk: Option<crate::block_disk::BlockDisk>,
}

impl Machine {
    /// A machine with [`MAIN_WORDS`] of main memory: thirty-two boards.
    pub fn new() -> Self {
        Self::with_memory_boards(MAIN_WORDS >> 16)
    }

    /// How many 64K-word memory boards this machine has.
    pub fn memory_boards(&self) -> usize {
        self.main.len() >> 16
    }

    /// A machine with `boards` memory boards of 64K words each, which is
    /// what `--main-memory-boards` sets on every engine. From one to
    /// [`busint::MAX_MEMORY_BOARDS`], where the Xbus I/O space begins.
    pub fn with_memory_boards(boards: usize) -> Self {
        assert!(
            (1..=busint::MAX_MEMORY_BOARDS).contains(&boards),
            "{boards} memory boards: the backplane holds 1 to {}",
            busint::MAX_MEMORY_BOARDS
        );
        Machine {
            prom: vec![Insn::new(0); PROM_WORDS],
            imem: vec![Insn::new(0); IMEM_WORDS],
            mode: spy::Mode::default(),
            clock_control: spy::ClockControl::default(),
            opc_control: spy::OpcControl::default(),
            debug_ir: 0,
            prog_reset: false,
            prog_boot: false,
            amem: [0; 1024],
            mmem: [0; 32],
            dmem: [0; 2048],
            pdl: [0; PDL_WORDS],
            spc: [0; 32],
            spcptr: 0,
            pdl_pointer: 0,
            pdl_index: 0,
            q: 0,
            opc: 0,
            lc: 0,
            vma: 0,
            md: 0,
            interrupt_control: 0,
            dispatch_constant: 0,
            geometry: Geometry::CADR,
            tick: Tick::new(),
            dma_written: false,
            block_disk: None,
            l1_map: [0; 2048],
            l2_map: [0; L2_MAP_WORDS],
            main: vec![0; boards << 16],
            bus_error: 0,
            interrupt_status: busint::interrupt_status::LOCAL_ENABLE,
            write_through: false,
            unibus_map: [0; 16],
            read_buffer: [0; 16],
            write_buffer: [0; 16],
            vmaok: true,
            disk: Controller::default(),
            chaos: crate::chaos::Config::default(),
            // The I/O board has its Chaosnet interface whether or not a
            // cable is plugged in: the switches at the configured address,
            // nothing on the cable until [`Machine::plug_chaos`].
            ioboard: {
                let mut b = ioboard::IoBoard::default();
                b.plug_chaos(crate::chaos::Config::default().address, None, 0, false);
                b
            },
            tv: Tv::default(),
            color_tv: None,
            cycles: 0,
            ns: 0,
        }
    }

    /// Puts the color TV on the backplane, which is what `--color-tv`
    /// does where an engine builds its machine.  The board is the
    /// backplane's and does not come and go under a running machine.
    pub fn fit_color_tv(&mut self) {
        self.color_tv = Some(Tv::color());
    }

    /// Loads the boot PROM.  Words past the end of the image stay zero.
    pub fn load_prom(&mut self, words: &[Insn]) {
        for (i, w) in words.iter().take(PROM_WORDS).enumerate() {
            self.prom[i] = *w;
        }
    }

    /// Fetches from the control store, honoring the PROM overlay.
    pub fn fetch(&self, pc: u16) -> Insn {
        let pc = pc as usize & (IMEM_WORDS - 1);
        if self.mode.prom_disable || pc >= PROM_WORDS { self.imem[pc] } else { self.prom[pc] }
    }

    /// LC byte mode, `interrupt_control<29>`: the 25LS2519 at FLAG 3E08
    /// takes `OB29` to `LC BYTE MODE` under `-DESTINTCTL`, beside `OB28` to
    /// `PROG.UNIBUS.RESET`, `OB27` to `INT.ENABLE` and `OB26` to
    /// `SEQUENCE.BREAK`.
    pub fn byte_mode(&self) -> bool {
        self.interrupt_control & (1 << 29) != 0
    }

    pub fn push_spc(&mut self, pc: u32) {
        self.spcptr = (self.spcptr + 1) & 0o37;
        self.spc[self.spcptr as usize] = pc;
    }

    pub fn pop_spc(&mut self) -> u32 {
        let v = self.spc[self.spcptr as usize];
        self.spcptr = self.spcptr.wrapping_sub(1) & 0o37;
        v
    }

    /// Translates a 24-bit virtual address through the two map levels.
    ///
    /// The geometry is the netlist's, read off the VMEM pages; `tests/chip.rs`
    /// drives the same RAMs from that wiring and compares them word for word:
    ///
    /// | level | pages | words | width | addressed by |
    /// |---|---|---|---|---|
    /// | 1 | VMEM0 | 2048 | 5 | `VMA<23:13>` |
    /// | 2 | VMEM1, VMEM2 | 1024 | 24 | `{VMAP<4:0>, VMA<12:8>}` |
    ///
    /// So a level-1 entry picks one block of 32 level-2 entries and
    /// `VMA<12:8>` picks the entry within it.  `VMA<7:0>` never reaches the
    /// map at all; it is the offset within the page.
    ///
    /// Both levels are wired active low --- VMEM0 reads back as `-VMAP`, level
    /// 2 as `-VMO`.  That matters to `src/chip.rs`, which holds cells; this
    /// holds values, so it does not appear here.
    pub fn translate(&self, vaddr: u32) -> Translation {
        // Only VMA<23:0> reaches MAPI; `ir.bits` writes level 2 from the same
        // 24 bits.
        let vaddr = vaddr & 0x00ff_ffff;
        let l1_data = self.l1_map[(vaddr >> 13) as usize & 0o3777] & self.geometry.l1_mask();
        let l2_data = self.l2_map[self.geometry.l2_index(l1_data, vaddr)];
        // `VMO<13:0>` is the physical page: 14 bits, which is what the boot
        // PROM's own `SET-UP-FOUR-PAGES` needs to name page 0o37766.
        let page = l2_data & 0x3fff;
        Translation {
            physical: (page << 8) | (vaddr & 0xff),
            page,
            l1_data,
            l2_data,
            // VMEMDR 1D14 latches `-VMO23` and `-VMO22`; VCTL2 1D26 makes
            // `-PFR` from the first, VCTL1 1D17 makes `-PFW` from the second
            // with `WRCYC`.
            write_permitted: l2_data & (1 << 22) != 0,
            access_permitted: l2_data & (1 << 23) != 0,
        }
    }

    /// Writes the map, as the WRITE-MAP destinations do.  `VMA<26>` enables the
    /// level-1 write from `VMA<31:27>`; `VMA<25>` enables the level-2 write
    /// from `VMA<23:0>`.  Both are indexed by MD.  Those four field positions
    /// are `mit/cadr/ir.bits` in MIT's own words.
    ///
    /// The hardware performs the write on the cycle *after* the store, and
    /// `ir.bits` warns that VMA must not be disturbed meanwhile.  That delay
    /// is the caller's: `micro` holds the write in `WMAPD` and calls this at
    /// the start of the next microcycle, `rtl` writes the two levels itself
    /// in its write phase, and `chip` has the registers.
    ///
    /// **A store with both `VMA<26>` and `VMA<25>` up writes level 2 with
    /// the level-1 bits of its address zero.**  The two write pulses are one,
    /// `-WP1` through the 74S37 at VCTL2 1D07 (`-VM0WPA/B` and `-VM1WPA/B`).
    /// Level 1 is 93425As, and "During writing, the output is held in the
    /// high impedance state" (Fairchild, 1977 Bipolar Memory Data Book,
    /// 93425/93425A, page 7-120, within 20 ns of `WE` falling); `-VMAP<4:0>`
    /// has no other driver and no pull-up, so the TTL inputs of the 74S240s
    /// at VMEM1 1D08 and VMEM2 1C10 read it high and their outputs, level
    /// 2's top five address bits, go low.  The new level-1 entry never
    /// reaches that address during the 40 ns pulse, and the old one is there
    /// for less than the part's guaranteed write, 20 ns.  `chip`, which has
    /// the RAMs and the buffers, gives the same, and
    /// `chip_rtl_and_micro_write_both_map_levels_alike` holds the three.
    /// **Unverified:** whether the old entry takes a partial write in its
    /// first nanoseconds; a CADR running the two-instruction store would
    /// settle it.  Microcode 323 writes the levels in separate stores
    /// (`LEVEL-1-MAP-MISS` in `uc-page-fault.lisp`), so the band never asks.
    pub fn write_map(&mut self, vma: u32, md: u32) {
        let l1_index = (md >> 13) as usize & 0o3777;
        if vma & (1 << 26) != 0 {
            self.l1_map[l1_index] = self.geometry.l1_from_vma(vma);
        }
        if vma & (1 << 25) != 0 {
            let l1_data = if vma & (1 << 26) != 0 { 0 } else { self.l1_map[l1_index] };
            self.l2_map[self.geometry.l2_index(l1_data, md)] = vma & 0o77777777;
        }
    }

    /// Reads virtual memory, setting [`Machine::vmaok`].  A denied access is
    /// not an error: the microcode tests for it with the page-fault jump
    /// conditions, and `-VMAOK` is one of the FLAG bits `ir.bits` lists.
    ///
    /// `VMAOK` is `(-PFR) AND (-PFW)`, so a read needs access permission and
    /// a write needs both.
    pub fn vm_read(&mut self, vaddr: u32) -> u32 {
        let t = self.translate(vaddr);
        self.vmaok = t.access_permitted;
        if !self.vmaok {
            return 0;
        }
        self.bus_read(t.physical)
    }

    pub fn vm_write(&mut self, vaddr: u32, value: u32) {
        let t = self.translate(vaddr);
        self.vmaok = t.access_permitted && t.write_permitted;
        if self.vmaok {
            self.bus_write(t.physical, value);
        }
    }

    /// One bus cycle, by physical address.
    ///
    /// **An address nothing answers is not an error.** The board times the
    /// cycle out, sets an NXM bit and carries on, and the microcode relies on
    /// that: `PAGE-0-PARITY-FIX` at `00310` deliberately runs one word past
    /// the end of its loop --- "This does one extra location, too bad" ---
    /// into the top of the Xbus I/O region, where nothing lives. Faulting
    /// there stops the machine before it ever reaches the disk.
    ///
    /// The regions are MIT's own, from the bus interface specification:
    /// Xbus memory up to page `0o35777`, Xbus I/O `0o36000`-`0o36777`, and
    /// the Unibus `0o37000`-`0o37777`. What answers is [`Machine::bus_read`]'s
    /// list: the disk controller and the display on the Xbus, the bus
    /// interface's own registers and the I/O board on the Unibus. Every
    /// other I/O address times out.
    fn device(&mut self, phys: u32) -> Option<usize> {
        // QUUX's feature page, a device of its own ([`Geometry::feature_word`]).
        if self.geometry.feature_word(phys).is_some() {
            return None;
        }
        match busint::decode_for(
            phys,
            self.main.len(),
            self.color_tv.is_some(),
            self.tv.buffer_words(),
            self.tv.control_registers(),
        ) {
            busint::Responder::Memory(_) => Some(phys as usize),
            busint::Responder::Device
            | busint::Responder::Interface
            | busint::Responder::Unibus(_) => None,
            // The debug block's word is the other machine's, over the cable;
            // `rtl` carries it.  Here, with no cable, a read is nothing and a
            // write goes nowhere, and neither is an error: the pull-up on
            // `DEBUG OUT ACK` answers.
            busint::Responder::Debug(_) => None,
            // The map's responders are the debug master's,
            // [`Machine::mapped_read`] and [`Machine::mapped_write`]; the
            // processor's `decode` never makes them.
            busint::Responder::MapBuffer
            | busint::Responder::MapXbus(_)
            | busint::Responder::MapRefused
            | busint::Responder::MapMd => None,
            busint::Responder::NoXbus => {
                self.bus_error |= bus_error::XBUS_NXM;
                None
            }
            busint::Responder::NoUnibus => {
                self.bus_error |= bus_error::UNIBUS_NXM;
                None
            }
        }
    }

    /// `INT` on the cables: the interrupt line the bus interface presents to
    /// the cpu, which the 74S175 at LCC 3E12 registers as `SINTR` and the
    /// page-fault-or-interrupt jump conditions take under `INT.ENABLE`.
    /// `LM INT` is `UB INT OR XBUS INTR IN` at UBINTC 0E04: the disk
    /// controller's request on the Xbus, or a Unibus interrupt taken or
    /// simulated. The I/O board's keyboard interrupt comes over the Unibus
    /// and is taken by [`Machine::unibus_interrupt`].
    ///
    /// On QUUX, its tick too, while enabled and up, at [`Machine::ns`].
    pub fn interrupt(&self) -> bool {
        self.xbus_interrupt()
            || self.unibus_interrupt().is_some()
            || (self.geometry.tick && self.tick.enabled && self.tick.flag(self.ns))
    }

    /// `XBUS INTR IN`: the disk controller's request, or either display's
    /// vertical interrupt, on the one Xbus line.
    ///
    /// **Both display boards drive the same wire.** On each of them the
    /// 74S08 at 0D10 ands `MODE INTR ENB` with `VERT FLAG` into `SEND
    /// INTR`, and the 26S10 at 0F14 --- an open-collector bus transceiver
    /// --- puts that on `-XBUS.INTR`; the nets are the same on both
    /// boards' netlists.  So the line is the boards ORed, and a second
    /// board fitted adds its own `SEND INTR` to it.  Nothing in
    /// System 100 turns the color board's on: `COLOR:SETUP` starts its
    /// sync with `(SI:START-SYNC 3 0 36.)`, and `CC-TV-START-SYNC` in
    /// `sys/cc/dmon.lisp` --- the same call, written out --- writes the
    /// mode as `(+ (LSH BOW 2) CLOCK)`, which is 3: the clock mode alone,
    /// with [`tv::mode::INTERRUPT_ENABLE`] clear.  That matters because
    /// `INTRX0` in microcode 323 reads `A-TV-REGS-BASE`, the normal TV's
    /// register, and clears the flag there; a color-board interrupt would
    /// have nothing to take it.
    pub fn xbus_interrupt(&self) -> bool {
        // The controller was told the time at the last bus access; a run's
        // engine keeps `ns` current between them (`rtl` every microcycle).
        self.disk.interrupt()
            || self.block_disk.as_ref().is_some_and(|d| d.interrupt_at(self.ns))
            || self.tv.interrupt(self.ns)
            || self.color_tv.as_ref().is_some_and(|tv| tv.interrupt(self.ns))
    }

    /// The Unibus interrupt the interface has taken, as `UB INT` and the
    /// vector, if it has one: what a read of `766040` shows in bits 15 and
    /// 2-9.
    ///
    /// One taken by writing the bit stays until the bit is written clear.
    /// One from a device is taken while
    /// [`busint::interrupt_status::ENABLE_UB_INTS`] is set --- the grant
    /// the interface gives a request on the bus --- and its vector is the
    /// device's. The model has no grant cycle to latch at, so the vector
    /// is read off the requesting board at the time of the read: the same
    /// value, since the request holds until the microcode has read the
    /// device's data, which is the one thing that clears `KBD READY`.
    /// Dismissal is the microcode's write of zero to `766042`,
    /// `UB-INTR-RET-0` in `uc-interrupt.lisp`, by which time the request
    /// is gone.
    pub fn unibus_interrupt(&self) -> Option<u16> {
        use busint::interrupt_status::{ENABLE_UB_INTS, UB_INT, VECTOR_MASK};
        if self.interrupt_status & UB_INT != 0 {
            return Some(UB_INT | (self.interrupt_status & VECTOR_MASK));
        }
        if self.interrupt_status & ENABLE_UB_INTS == 0 {
            return None;
        }
        self.ioboard.interrupt_request(self.ns).map(|vector| UB_INT | (vector & VECTOR_MASK))
    }

    /// Plugs the Chaosnet in as [`Machine::chaos`] describes it: the
    /// interface on the I/O board at the configured address, and on its
    /// cable the Chaosnet hosts that are not in this process, if a CHUDP
    /// link was bound. Powered at `powered_at` on the machine's clock.
    ///
    /// **Nothing else is on that cable.** A CADR carries no file or time
    /// server, so neither does this; what a band calls for its files and
    /// the date is a host on the network, reached over the link.
    /// [`Machine::attach_chaos_node`] is how a caller in this process ---
    /// the test harness, with its Chaosnet server --- puts one there
    /// instead.
    pub fn plug_chaos(&mut self, powered_at: u64) {
        let mut ether = crate::chaos::ether::Ether::new();
        if let Some(node) = self.chaos.udp_node() {
            ether.attach(node);
        }
        self.ioboard.plug_chaos(self.chaos.address, Some(ether), powered_at, self.chaos.trace);
    }

    /// Puts `node` on this machine's Chaosnet cable, beside the CHUDP
    /// link's if there is one: a station like any other, heard by the
    /// interface and taking its turn to transmit.
    ///
    /// After [`Machine::plug_chaos`], which is what makes the cable; a
    /// machine with none has nothing to attach to and this panics rather
    /// than dropping the node on the floor.
    pub fn attach_chaos_node(&mut self, node: Box<dyn crate::chaos::ether::Node>) {
        self.ioboard
            .chaos
            .as_mut()
            .and_then(|c| c.ether_mut())
            .expect("a Chaosnet cable: plug_chaos first")
            .attach(node);
    }

    /// The bus interface's own registers, as the board reads them back.
    fn interface_read(&self, r: busint::Register) -> u16 {
        use busint::{Register, error_status, interrupt_status};
        match r {
            // A diagnostic register is the cpu's own state on `SPY<15:0>`
            // under `-DBREAD`, and the engine answers it before the bus is
            // asked: [`crate::engine::Engine::spy_read`].  Here, where no
            // engine is, the bus reads as if the cpu had driven nothing.
            Register::Diagnostic(_) => spy::OPEN_READ,
            Register::InterruptControl2 | Register::Unused => 0,
            Register::InterruptControl => {
                let live = if self.xbus_interrupt() { interrupt_status::XBUS_INTR } else { 0 };
                let taken = self.unibus_interrupt().unwrap_or(0);
                let stored = self.interrupt_status
                    & !(interrupt_status::XBUS_INTR
                        | interrupt_status::UB_INT
                        | interrupt_status::VECTOR_MASK);
                stored | live | taken
            }
            // The 74LS244 at REQERR 0C16 drives eight bits of `UDO`; the
            // high byte is the Unibus pulled up, and reads as ones ---
            // **measured on the netlist board**, the processor's own read
            // of the register in `tests/chip.rs`.
            Register::ErrorStatus => {
                0xff00
                    | self.bus_error
                    | error_status::NOT_FREE
                    | if self.write_through { error_status::WRITE_THROUGH } else { 0 }
            }
            Register::Map(k) => self.unibus_map[k as usize],
        }
    }

    /// A Unibus map entry as the 29701s at UBMAP 0E12-0E15 hold it: the
    /// physical page and whether writes are allowed, or `None` if the
    /// page's `MAPVALID` is down.  "Bit 15 of the register signifies that
    /// mapping of that page is turned on; otherwise, that section of the
    /// unibus is non-existent memory.  Bit 14 enables write access from the
    /// unibus.  The remainder of the register is the page number" ---
    /// `unaddr.text`; `UDI15` to `MAPVALID`, `UDI14` to `WRITEOK`,
    /// `UDI<13:0>` to `UBMA<21:8>` on the netlist.
    pub fn map_entry(&self, page: u8) -> Option<(u32, bool)> {
        let e = self.unibus_map[page as usize & 0o17];
        (e & 0x8000 != 0).then_some(((e & 0x3fff) as u32, e & 0x4000 != 0))
    }

    /// A Unibus master's read through the map, [`busint::map_access`].  The
    /// even word is an Xbus read of the mapped word --- `-UB READ XBUS` ---
    /// whose low half is the answer and whose high half goes into the
    /// page's read buffer; the odd word is the buffer, `-UB READ BUFFER`,
    /// with no Xbus cycle.  `None` when the page is invalid, which is
    /// `UB MAP ERROR` and no answer.
    pub fn mapped_read(&mut self, access: busint::MapAccess) -> Option<u16> {
        let k = access.page as usize;
        if access.high {
            return Some(self.read_buffer[k]);
        }
        let Some((page, _)) = self.map_entry(access.page) else {
            self.bus_error |= bus_error::UB_MAP_ERROR;
            return None;
        };
        let word = self.bus_read((page << 8) | access.word);
        self.read_buffer[k] = (word >> 16) as u16;
        Some(word as u16)
    }

    /// A Unibus master's write through the map: the even word into the
    /// page's write buffer, `-UB WRITE BUFFER`; the odd word an Xbus write
    /// of the two halves, `-UB WR XBUS`, if the page is valid and writable
    /// --- else `UB MAP ERROR`, no write and no answer, `false`.  In
    /// write-through mode, bit 7 of the error status register, the even
    /// word's write on the upper eight pages is an Xbus write as well, of
    /// the Unibus word with zeros above it --- **measured on the netlist
    /// board**, `tests/chip.rs`.
    pub fn mapped_write(&mut self, access: busint::MapAccess, v: u16) -> bool {
        let k = access.page as usize;
        if !access.high {
            self.write_buffer[k] = v;
            // Write-through mode, `WRITE THROUGH ENB` at UBCYC 0B08, on the
            // upper eight pages: the low half's write goes to the Xbus at
            // once, and the 74LS244s at BUSSEL under `-UB16>BUS` put the
            // Unibus word on `BUS<15:0>` and ground on `BUS<31:16>`.
            if !(self.write_through && access.page >= 0o10) {
                return true;
            }
        }
        match self.map_entry(access.page) {
            Some((page, true)) => {
                let word = if access.high {
                    (v as u32) << 16 | self.write_buffer[k] as u32
                } else {
                    v as u32
                };
                // A page with its high five bits ones is `MD`, not the
                // Xbus: `-UB TO MD`, CC's `CC-WRITE-MD`.
                if busint::map_to_md(page) {
                    self.md = word;
                } else {
                    self.bus_write((page << 8) | access.word, word);
                }
                true
            }
            _ => {
                self.bus_error |= bus_error::UB_MAP_ERROR;
                false
            }
        }
    }

    /// The error status register's eight bits as the 8304 at REQERR 0B15
    /// puts them on the debug cable under `-DB READ STATUS`: [`bus_error`]'s
    /// bits, `-FREE` in bit 6 as it stands --- which a read of `766044` by
    /// the processor always finds up, the interface being busy with that
    /// read, and which the debugger's strobe finds as `busy` says --- and
    /// `WRITE THROUGH ENB` in bit 7.  CC's `DBG-PRINT-STATUS` names the
    /// low six and ignores the rest.
    pub fn debug_status(&self, busy: bool) -> u16 {
        use busint::error_status;
        self.bus_error
            | if busy { error_status::NOT_FREE } else { 0 }
            | if self.write_through { error_status::WRITE_THROUGH } else { 0 }
    }

    fn interface_write(&mut self, r: busint::Register, v: u16) {
        use busint::{Register, error_status, interrupt_status};
        match r {
            Register::Diagnostic(r) => self.spy_write(r, v),
            Register::Unused => {}
            Register::InterruptControl => {
                self.interrupt_status = (self.interrupt_status & !interrupt_status::CONTROL_MASK)
                    | (v & interrupt_status::CONTROL_MASK);
            }
            Register::InterruptControl2 => {
                self.interrupt_status = (self.interrupt_status & !interrupt_status::CONTROL2_MASK)
                    | (v & interrupt_status::CONTROL2_MASK);
            }
            // "Writing this location ignores the data written and clears
            // the status bits" --- all but the one the drawings clock from it.
            Register::ErrorStatus => {
                self.bus_error = 0;
                self.write_through = v & error_status::WRITE_THROUGH != 0;
            }
            Register::Map(k) => self.unibus_map[k as usize] = v,
        }
    }

    /// Loads one of the console's registers, as the trailing edge of the
    /// write strobe does --- from a Unibus write into `766000`-`766036`, or
    /// from a console that has no bus.  `eadr` is `EADR<3:0>`; only
    /// `EADR<2:0>` reach the write decoder, so 8 to 15 alias 0 to 7.  See
    /// [`crate::spy`].
    ///
    /// Two bits of a mode-register write are pulses and not settings:
    /// [`spy::MODE_RESET`] and [`spy::MODE_BOOT`] are gated with the strobe
    /// on OLORD2, so they are recorded here for the engine to act on and
    /// nothing is stored.  `RESET` clears the mode register itself, which is
    /// why CC's `CC-RESET-MACH` writes `100` and then the mode it wants.
    pub fn spy_write(&mut self, eadr: u8, v: u16) {
        match spy::write_strobe(eadr) {
            half @ (spy::IR_LOW | spy::IR_MED | spy::IR_HIGH) => {
                spy::write_debug_ir(&mut self.debug_ir, half, v)
            }
            spy::CLK => self.clock_control.write(v),
            spy::OPC_CONTROL => self.opc_control.write(v),
            spy::MODE => {
                self.mode.write(v);
                // QUUX's mode register has no speed bits: they go nowhere.
                if !self.geometry.speed_bits {
                    self.mode.speed0 = false;
                    self.mode.speed1 = false;
                }
                self.prog_reset |= v & spy::MODE_RESET != 0;
                self.prog_boot |= v & spy::MODE_BOOT != 0;
            }
            _ => {}
        }
    }

    /// What `-RESET` clears of the console's registers: the mode register,
    /// the OPC control register and the 74S175 half of the clock control
    /// register.  `RUN` is cleared by `-CLOCK RESET A`, the power-on reset,
    /// and not by this, so a console that resets the machine leaves it
    /// running or halted as it was.
    pub fn reset_console_registers(&mut self) {
        self.mode = spy::Mode::default();
        self.opc_control = spy::OpcControl::default();
        self.clock_control =
            spy::ClockControl { run: self.clock_control.run, ..Default::default() };
    }

    /// `RESET` on the bus interface, which it puts on the backplane as
    /// `-XBUS INIT` (the 26S10 at XA 0F21) and `-UB INIT` (the 8838 at
    /// 0F06).  The 74S10 at DBGIN 0A14 makes it from three inputs: `-LM
    /// UNIBUS RESET`, the processor's `PROG.UNIBUS.RESET` ---
    /// `INTERRUPT-CONTROL<28>`, the 25LS2519 at FLAG 3E08, over the cable
    /// as `-BUS.RESET` --- which microcode 323 holds for about 80
    /// microseconds at its start, "RESET THE BUS INTERFACE AND I/O DEVS";
    /// `-DEBUGEE RESET`, the debug modifier's reset bit from the debug
    /// cable; and `UNIBUS INIT IN` from another master of the Unibus.
    ///
    /// Each model board clears what its own reset pin clears and nothing
    /// more: [`Tv::xbus_init`], [`Controller::xbus_init`],
    /// [`IoBoard::unibus_init`].  The netlist boards on `chip` take the wire
    /// itself, and behind the netlist interface [`crate::buses::Buses`]
    /// calls this as the wire is asserted.  The memory boards are the
    /// engine's twins, held by the same wire in
    /// [`busint::Busint::unibus_reset`].  Power-on is the constructor:
    /// every board here is built in its reset state.
    /// The model boards clear what their drawings say on the interface's
    /// `RESET` --- `-XBUS INIT` and `-UB INIT`.  Three things drive it: the
    /// machine's own `PROG.UNIBUS.RESET`, from `Rtl` as the `INTERRUPT-
    /// CONTROL` write lands and from `Micro` at the functional destination;
    /// the debug cable's `-DEBUGEE RESET` (`Rtl::debug_cycle`); and, on the
    /// `chip` engine, `-XBUS INIT` off the netlist (`crate::buses`).
    /// Microcode 323 pulses its own only as it starts --- the boot PROM's
    /// "RESET THE BUS INTERFACE AND I/O DEVS" and `RESET-MACHINE` in
    /// `uc-cold-disk.lisp`, with nothing in flight --- and cannot do so by
    /// accident later: every other write of `INTERRUPT-CONTROL` is `IOR`
    /// or `ANDCA` of `LOCATION-COUNTER` with `1_26.`, and the `LC` source
    /// hands the flop back as `MF28` (the 74S241 at LC 1A16, pin 8 to pin
    /// 12), so bit 28 is rewritten as it stands.  Watched over a
    /// two-machine run of CC's diagnostics: three pulses at A's boot and
    /// one at B's, none after.
    pub fn bus_reset(&mut self) {
        self.tv.xbus_init(self.ns);
        // `-XBUS INIT` is a bused line and reaches every board on it, the
        // second display board with the rest.
        if let Some(tv) = self.color_tv.as_mut() {
            tv.xbus_init(self.ns);
        }
        self.disk.xbus_init();
        if let Some(d) = self.block_disk.as_mut() {
            d.xbus_init();
        }
        self.ioboard.unibus_init();
    }

    /// A read of a diagnostic register through this alone reads the open
    /// bus; the engines answer them.  See [`crate::spy`].
    pub fn bus_read(&mut self, phys: u32) -> u32 {
        if let Some(w) = self.geometry.feature_word(phys) {
            // Words 11 to 13 are the main screen, from the board fitted:
            // width in 31:16 and height in 15:0; bits a pixel in 31:16 and
            // words a line in 15:0; and the buffer's first physical address.
            let (width, height, words_per_line) = self.tv.screen();
            return match phys & 0o377 {
                0o11 => (width as u32) << 16 | height as u32,
                0o12 => 1 << 16 | words_per_line as u32,
                0o13 => tv::NORMAL_TV.buffer,
                _ => w,
            };
        }
        if let Some(r) = disk_controller::register(phys) {
            // QUUX's block-disk, when it is fitted, in the CADR
            // controller's place.
            if let Some(d) = self.block_disk.as_mut() {
                d.advance(self.ns);
                return d.read(r);
            }
            self.disk.advance(self.ns);
            return self.disk.read(r);
        }
        if let Some(off) = self.tv.buffer_offset(phys) {
            return self.tv.read_buffer(off);
        }
        if let Some(r) = self.tv_register(phys) {
            return self.tv.read_control(r, self.ns);
        }
        // The color TV, when one is fitted: the same board at the other
        // strap.  `device` has already given the NXM when it is not.
        if let Some(tv) = self.color_tv.as_ref() {
            if let Some(off) = tv::COLOR_TV.buffer_offset(phys) {
                return tv.read_buffer(off);
            }
            if let Some(r) = tv::COLOR_TV.control_register(phys) {
                return tv.read_control(r, self.ns);
            }
        }
        // The Unibus carries 16 bits, in the bottom of one Lisp machine word.
        if let Some(r) = busint::unibus_address(phys).and_then(busint::register) {
            return self.interface_read(r) as u32;
        }
        if let Some(r) = busint::unibus_address(phys).and_then(|u| ioboard::answers(u, false)) {
            return self.ioboard.read(r, self.ns) as u32;
        }
        match self.device(phys) {
            Some(a) => self.main[a],
            None => 0,
        }
    }

    /// Which of the main display's control registers a physical address
    /// names, if it is one the board has.
    fn tv_register(&self, phys: u32) -> Option<u32> {
        tv::control_register(phys).filter(|&r| self.tv.control_registers() >> r & 1 != 0)
    }

    pub fn bus_write(&mut self, phys: u32, value: u32) {
        if let Some(r) = disk_controller::register(phys) {
            // A transfer is a bus master reading and writing physical memory
            // directly, which is why the controller is handed it.
            if let Some(d) = self.block_disk.as_mut() {
                d.advance(self.ns);
                d.write(r, value, &mut self.main);
            } else {
                self.disk.advance(self.ns);
                self.disk.write(r, value, &mut self.main);
            }
            // A transfer writes main memory behind the processor's back:
            // QUUX's cache is invalidated before its next cycle.
            self.dma_written = true;
            return;
        }
        if let Some(off) = self.tv.buffer_offset(phys) {
            self.tv.write_buffer(off, value);
            return;
        }
        if let Some(r) = self.tv_register(phys) {
            self.tv.write_control(r, value, self.ns);
            return;
        }
        // The color TV, when one is fitted.
        let ns = self.ns;
        if let Some(tv) = self.color_tv.as_mut() {
            if let Some(off) = tv::COLOR_TV.buffer_offset(phys) {
                tv.write_buffer(off, value);
                return;
            }
            if let Some(r) = tv::COLOR_TV.control_register(phys) {
                tv.write_control(r, value, ns);
                return;
            }
        }
        // The Unibus carries 16 bits, in the bottom of one Lisp machine word.
        if let Some(r) = busint::unibus_address(phys).and_then(busint::register) {
            self.interface_write(r, value as u16);
            return;
        }
        if let Some(r) = busint::unibus_address(phys).and_then(|u| ioboard::answers(u, true)) {
            self.ioboard.write(r, value as u16, self.ns);
            return;
        }
        if let Some(a) = self.device(phys) {
            self.main[a] = value;
        }
    }
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

/// What the 74S373 latch at VMEMDR 1D14 holds at power-on: the map word of
/// the last memory cycle, which there has been none of. Permission bits
/// up, so that nothing faults before the first cycle, and a page number of
/// all ones. `rtl` and `micro` both start their latch from it.
///
/// The board has no answer to give. A 74S373 has no clear, and what it is
/// transparent onto is the second-level map, whose 93425As on VMEM0 to
/// VMEM2 come up holding whatever they come up holding; so this is a
/// modeling choice and not a fact about the hardware. What it has to be
/// is unobservable, and it is:
/// the only read of the map before the first memory cycle is
/// `SET-UP-THE-MAP` in `sys/ucadr/promh.text`, which takes
/// `MEMORY-MAP-DATA` and keeps `(BYTE-FIELD 5 24.)`, and AIM-528 gives
/// `MAP<28-24>` as the first-level map --- written to zero two cycles
/// earlier by the `VMA-WRITE-MAP` above it. This latch reaches only
/// `MAP<31-30>`, the fault bits of the last memory cycle, which that
/// microinstruction discards. What the constant is really for is keeping
/// `rtl` and `micro` on the same number as `chip`, and a `chip` read of
/// `MAP[MD]` before any memory cycle is what would fix it.
pub const LVMO_AT_POWER_ON: u32 = (1 << 23) | (1 << 22) | 0x3fff;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Translation {
    /// 22-bit physical address.
    pub physical: u32,
    /// 14-bit physical page number.
    pub page: u32,
    pub l1_data: u32,
    pub l2_data: u32,
    pub write_permitted: bool,
    pub access_permitted: bool,
}

// --- Checkpoints ------------------------------------------------------------

impl Machine {
    /// The machine into a checkpoint: every memory and register, the disk
    /// controller with its drives, the display, the I/O board, and its
    /// clocks.  Not [`Machine::chaos`], which is the flags' say and a
    /// resume has again.
    pub fn save(&self, w: &mut crate::checkpoint::Writer) {
        let Machine {
            prom,
            imem,
            mode,
            clock_control,
            opc_control,
            debug_ir,
            prog_reset,
            prog_boot,
            amem,
            mmem,
            dmem,
            pdl,
            spc,
            spcptr,
            pdl_pointer,
            pdl_index,
            q,
            opc,
            lc,
            vma,
            md,
            interrupt_control,
            dispatch_constant,
            geometry,
            tick,
            dma_written,
            block_disk,
            l1_map,
            l2_map,
            main,
            bus_error,
            interrupt_status,
            write_through,
            unibus_map,
            read_buffer,
            write_buffer,
            vmaok,
            disk,
            chaos: _,
            tv,
            color_tv,
            ioboard,
            cycles,
            ns,
        } = self;
        w.u64s(&prom.iter().map(|i| i.raw()).collect::<Vec<_>>());
        w.u64s(&imem.iter().map(|i| i.raw()).collect::<Vec<_>>());
        mode.save(w);
        clock_control.save(w);
        opc_control.save(w);
        w.u64(*debug_ir);
        w.bool(*prog_reset);
        w.bool(*prog_boot);
        w.u32s(amem);
        w.u32s(mmem);
        w.u32s(dmem);
        w.u32s(pdl);
        w.u32s(spc);
        w.u8(*spcptr);
        w.u16(*pdl_pointer);
        w.u16(*pdl_index);
        w.u32(*q);
        w.u16(*opc);
        w.u32(*lc);
        w.u32(*vma);
        w.u32(*md);
        w.u32(*interrupt_control);
        w.u16(*dispatch_constant);
        w.u32s(l1_map);
        w.u8(geometry.l1_bits as u8);
        w.u8(geometry.pdl_bits as u8);
        w.bool(geometry.muldiv);
        w.bool(geometry.tick);
        tick.save(w);
        w.bool(*dma_written);
        w.u32s(l2_map);
        w.u32(self.memory_boards() as u32);
        w.u32s(main);
        w.u16(*bus_error);
        w.u16(*interrupt_status);
        w.bool(*write_through);
        w.u16s(unibus_map);
        w.u16s(read_buffer);
        w.u16s(write_buffer);
        w.bool(*vmaok);
        disk.save(w);
        w.opt(block_disk.as_ref(), |w, d| d.save(w));
        tv.save(w);
        // Whether the color TV was on the backplane, and if it was, the
        // board: a resume onto a machine `--color-tv` says otherwise about
        // is refused by the flag's name.
        w.bool(color_tv.is_some());
        if let Some(tv) = color_tv {
            tv.save(w);
        }
        ioboard.save(w);
        w.u64(*cycles);
        w.u64(*ns);
    }

    /// Back from a checkpoint, into a machine built as the flags say: the
    /// same memory, the same pack under it, the same Chaosnet on it.
    pub fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        fn insns(r: &mut crate::checkpoint::Reader, into: &mut [Insn]) -> std::io::Result<()> {
            let mut raw = vec![0u64; into.len()];
            r.u64s_into(&mut raw)?;
            for (i, w) in into.iter_mut().zip(raw) {
                *i = Insn::new(w);
            }
            Ok(())
        }
        insns(r, &mut self.prom)?;
        insns(r, &mut self.imem)?;
        self.mode = spy::Mode::load(r)?;
        self.clock_control = spy::ClockControl::load(r)?;
        self.opc_control = spy::OpcControl::load(r)?;
        self.debug_ir = r.u64()?;
        self.prog_reset = r.bool()?;
        self.prog_boot = r.bool()?;
        r.u32s_into(&mut self.amem)?;
        r.u32s_into(&mut self.mmem)?;
        r.u32s_into(&mut self.dmem)?;
        r.u32s_into(&mut self.pdl)?;
        r.u32s_into(&mut self.spc)?;
        // Pointers into the SPC stack and the PDL buffer: `SPCPTR<4:0>`,
        // five bits for the 32 words of the 82S21s on page SPC, and for the
        // PDL as many bits as the machine's geometry gives it --- ten on the
        // CADR, all page PDLPTR keeps of a write to either register --- which
        // is checked once the geometry is read, below.  The engines index
        // the arrays above with them, so a wider value is a corrupt
        // checkpoint and is refused rather than loaded.
        let spcptr = r.u8()?;
        if spcptr > 0o37 {
            return Err(crate::checkpoint::bad(format!(
                "SPC pointer {spcptr:o}, wider than the five bits of SPCPTR<4:0>"
            )));
        }
        let pdl_pointer = r.u16()?;
        let pdl_index = r.u16()?;
        self.spcptr = spcptr;
        self.pdl_pointer = pdl_pointer;
        self.pdl_index = pdl_index;
        self.q = r.u32()?;
        self.opc = r.u16()?;
        self.lc = r.u32()?;
        self.vma = r.u32()?;
        self.md = r.u32()?;
        self.interrupt_control = r.u32()?;
        self.dispatch_constant = r.u16()?;
        r.u32s_into(&mut self.l1_map)?;
        let (l1_bits, pdl_bits, muldiv) = (r.u8()? as u32, r.u8()? as u32, r.bool()?);
        let tick = r.bool()?;
        self.tick = Tick::load(r)?;
        self.dma_written = r.bool()?;
        // The CADR, or a QUUX with a PDL buffer of 1K to 16K words.
        self.geometry = match (l1_bits, pdl_bits, muldiv, tick) {
            (5, 10, false, false) => Geometry::CADR,
            (6, 10..=14, true, true) => Geometry { pdl_bits, ..Geometry::QUUX },
            _ => {
                return Err(crate::checkpoint::bad(format!(
                    "a map of {l1_bits}-bit level-1 entries and a {pdl_bits}-bit PDL buffer, multiply and divide {muldiv}, tick {tick}, is no machine's"
                )));
            }
        };
        for (what, v) in [("PDL pointer", pdl_pointer), ("PDL index", pdl_index)] {
            if v > self.geometry.pdl_mask() {
                return Err(crate::checkpoint::bad(format!(
                    "{what} {v:o}, wider than the {pdl_bits} bits that address the PDL"
                )));
            }
        }
        r.u32s_into(&mut self.l2_map)?;
        let boards = r.u32()? as usize;
        if boards != self.memory_boards() {
            return Err(crate::checkpoint::bad(format!(
                "{boards} memory boards, and this machine has {}: --main-memory-boards {boards}",
                self.memory_boards()
            )));
        }
        r.u32s_into(&mut self.main)?;
        self.bus_error = r.u16()?;
        self.interrupt_status = r.u16()?;
        self.write_through = r.bool()?;
        r.u16s_into(&mut self.unibus_map)?;
        r.u16s_into(&mut self.read_buffer)?;
        r.u16s_into(&mut self.write_buffer)?;
        self.vmaok = r.bool()?;
        self.disk.load(r)?;
        match (r.bool()?, self.block_disk.as_mut()) {
            (true, Some(d)) => d.load(r)?,
            (false, None) => {}
            (saved, _) => {
                return Err(crate::checkpoint::bad(format!(
                    "a checkpoint {} block-disk, onto a machine {}",
                    if saved { "with" } else { "without" },
                    if saved { "without one" } else { "with one" }
                )));
            }
        }
        self.tv.load(r)?;
        self.color_tv = match r.bool()? {
            true => {
                let mut tv = Tv::color();
                tv.load(r)?;
                Some(tv)
            }
            false => None,
        };
        self.ioboard.load(r)?;
        self.cycles = r.u64()?;
        self.ns = r.u64()?;
        Ok(())
    }
}
