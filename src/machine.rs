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
use crate::simpletv::{self, SimpleTv};
use crate::spy;

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
/// cannot happen here. The wording is MIT's own, from the bus interface
/// specification.
pub mod bus_error {
    /// "Xbus NXM Error. Set when an Xbus cycle times out for lack of
    /// response."
    pub const XBUS_NXM: u16 = 0o1;
    /// "Unibus NXM Error. Set when a Unibus cycle times out for lack of
    /// response."
    pub const UNIBUS_NXM: u16 = 0o10;
    /// "Unibus Map Error. Set when an attempt to perform an Xbus cycle
    /// through the Unibus map is refused because the map specifies invalid
    /// or write-protected." --- the head of `ldbg.lisp`.  The 74LS74 at
    /// REQERR 0D03, clocked by `UB XBUS T100`.
    pub const UB_MAP_ERROR: u16 = 0o40;
}

/// Why a microcycle could not complete.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Halt {
    /// A functional destination we do not implement.
    UnknownDest { pc: u16, dest: u16 },
}

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

    pub amem: [u32; 1024],
    pub mmem: [u32; 32],
    pub dmem: [u32; 2048],
    pub pdl: [u32; 1024],
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
    /// Location counter.  26 bits of address, plus NEED-FETCH in bit 31 and
    /// the interrupt-control flags mirrored in bits 29:26.
    pub lc: u32,
    pub vma: u32,
    pub md: u32,
    pub interrupt_control: u32,
    /// `IR<41:32>` of the last DISPATCH, readable as functional source 0.
    pub dispatch_constant: u16,

    /// 2048 five-bit entries, addressed by `VMA<23:13>`.
    pub l1_map: [u32; 2048],
    /// 1024 24-bit entries, addressed by the level-1 output and `VMA<12:8>`.
    pub l2_map: [u32; 1024],
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
    /// The Chaosnet: this machine's address and the host across the cable.
    pub chaos: crate::chaos::Config,
    /// The standard black-and-white display.
    pub simpletv: SimpleTv,
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
            pdl: [0; 1024],
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
            l1_map: [0; 2048],
            l2_map: [0; 1024],
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
            simpletv: SimpleTv::default(),
            cycles: 0,
            ns: 0,
        }
    }

    /// Loads the boot PROM.  Words past the end of the image stay zero.
    pub fn load_prom(&mut self, words: &[Insn]) {
        for (i, w) in words.iter().take(PROM_WORDS).enumerate() {
            self.prom[i] = *w;
        }
    }

    /// Fetches from the control store, honouring the PROM overlay.
    pub fn fetch(&self, pc: u16) -> Insn {
        let pc = pc as usize & (IMEM_WORDS - 1);
        if self.mode.prom_disable || pc >= PROM_WORDS { self.imem[pc] } else { self.prom[pc] }
    }

    /// LC byte mode, `interrupt_control<29>`.
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
        let l1_data = self.l1_map[(vaddr >> 13) as usize & 0o3777] & 0o37;
        let l2_data = self.l2_map[((l1_data << 5) | ((vaddr >> 8) & 0o37)) as usize];
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
    /// `ir.bits` warns that VMA must not be disturbed meanwhile.  Nothing here
    /// models that delay; the engines call this at the store.
    pub fn write_map(&mut self, vma: u32, md: u32) {
        let l1_index = (md >> 13) as usize & 0o3777;
        if vma & (1 << 26) != 0 {
            self.l1_map[l1_index] = (vma >> 27) & 0o37;
        }
        if vma & (1 << 25) != 0 {
            let l1_data = self.l1_map[l1_index] & 0o37;
            let l2_index = (l1_data << 5) | ((md >> 8) & 0o37);
            self.l2_map[l2_index as usize] = vma & 0o77777777;
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
        match busint::decode(phys, self.main.len()) {
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
    pub fn interrupt(&self) -> bool {
        self.xbus_interrupt() || self.unibus_interrupt().is_some()
    }

    /// `XBUS INTR IN`: the disk controller's request, or the display's
    /// vertical interrupt, on the one Xbus line.
    pub fn xbus_interrupt(&self) -> bool {
        // The controller was told the time at the last bus access; a run's
        // engine keeps `ns` current between them (`rtl` every microcycle).
        self.disk.interrupt() || self.simpletv.interrupt(self.ns)
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
    /// cable the Chaosnet server with its services, powered at `powered_at` on
    /// the machine's clock.
    pub fn plug_chaos(&mut self, powered_at: u64) {
        let mut ether = crate::chaos::ether::Ether::new();
        ether.attach(Box::new(self.chaos.server(powered_at)));
        self.ioboard.plug_chaos(self.chaos.address, Some(ether), powered_at, self.chaos.trace);
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
    /// more: [`SimpleTv::xbus_init`], [`Controller::xbus_init`],
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
        self.simpletv.xbus_init(self.ns);
        self.disk.xbus_init();
        self.ioboard.unibus_init();
    }

    /// A read of a diagnostic register through this alone reads the open
    /// bus; the engines answer them.  See [`crate::spy`].
    pub fn bus_read(&mut self, phys: u32) -> u32 {
        if let Some(r) = disk_controller::register(phys) {
            self.disk.advance(self.ns);
            return self.disk.read(r);
        }
        if let Some(off) = simpletv::buffer_offset(phys) {
            return self.simpletv.read_buffer(off);
        }
        if let Some(r) = simpletv::control_register(phys) {
            return self.simpletv.read_control(r, self.ns);
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

    pub fn bus_write(&mut self, phys: u32, value: u32) {
        if let Some(r) = disk_controller::register(phys) {
            // A transfer is a bus master reading and writing physical memory
            // directly, which is why the controller is handed it.
            self.disk.advance(self.ns);
            self.disk.write(r, value, &mut self.main);
            return;
        }
        if let Some(off) = simpletv::buffer_offset(phys) {
            self.simpletv.write_buffer(off, value);
            return;
        }
        if let Some(r) = simpletv::control_register(phys) {
            self.simpletv.write_control(r, value, self.ns);
            return;
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
/// modelling choice and not a fact about the hardware. What it has to be
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
            simpletv,
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
        simpletv.save(w);
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
        // five bits for the 32 words of the 82S21s on page SPC, and ten
        // bits for the 1K words of the PDL, which is all page PDLPTR keeps
        // of a write to either register.  The engines index the arrays
        // above with them, so a wider value is a corrupt checkpoint and is
        // refused rather than loaded.
        let spcptr = r.u8()?;
        if spcptr > 0o37 {
            return Err(crate::checkpoint::bad(format!(
                "SPC pointer {spcptr:o}, wider than the five bits of SPCPTR<4:0>"
            )));
        }
        let pdl_pointer = r.u16()?;
        let pdl_index = r.u16()?;
        for (what, v) in [("PDL pointer", pdl_pointer), ("PDL index", pdl_index)] {
            if v > 0o1777 {
                return Err(crate::checkpoint::bad(format!(
                    "{what} {v:o}, wider than the ten bits that address the PDL"
                )));
            }
        }
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
        self.simpletv.load(r)?;
        self.ioboard.load(r)?;
        self.cycles = r.u64()?;
        self.ns = r.u64()?;
        Ok(())
    }
}
