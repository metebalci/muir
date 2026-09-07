// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The standard black-and-white display: a frame buffer and a mode register
//! on the Xbus.
//!
//! One bit per pixel, 768 across, 24 words to a line. MIT's own name for it
//! is the TV, and the window system asks for it by
//! `'(:VIDEO :BLACK-AND-WHITE :CONTROLLER :SIMPLE)`.
//!
//! Three sources, reaching us by three routes, and they agree:
//!
//! 1. **`sys/window/shwarm.lisp`** in the System 100 release --- the software
//!    we target, writing to this device. It gives the control address, the
//!    buffer length, the geometry and the one mode bit it uses.  **primary**
//! 2. **`cadrtv/lmtv.order`** --- MIT's own programming specification for the
//!    board, which numbers the eight control registers and the mode bits and
//!    says where the buffer is: "for the normal TV, x is 6", and the normal
//!    TV is the `:CONTROLLER :SIMPLE` one.  **primary**
//! 3. **The board itself**, `data/SIMPLETV.netlist` through
//!    `tools/simpletv-netlist.sh`: all 29 of MIT's SUDS pages, of which
//!    `nxbctl` is the mode register and the control decode. The netlist
//!    board runs --- `tests/simpletv_netlist.rs` brings it up on a bus and
//!    measures its clock --- and is on the machine's Xbus under `chip`;
//!    this model answers for it on `micro` and `rtl`, and on `chip` with
//!    `--tv model`, written from the programming interface as
//!    [`crate::disk_controller`] is.  **primary, for the mode register**
//!
//! Where this is knowingly not the machine: there
//! is no video timing, so `VSYNC` and `HSYNC` never rise; the vertical flag
//! is kept on a frame clock rather than a raster, [`FRAME_NS`], which is
//! the netlist board's own period; and the sync RAM behind registers 1 to 3
//! is a store the program is written into and read back from, not run ---
//! `SI:SETUP-CPT` loads it at every `LISP-REINITIALIZE`, and reads it back.

/// First word of the frame buffer.  `MAIN-SCREEN-BUFFER-ADDRESS` is
/// `IO-SPACE-VIRTUAL-ADDRESS`, the base of Xbus I/O space, which
/// `bus-adaptor.c` decodes as physical `017000000`.
pub const BUFFER: u32 = 0o17000000;

/// `(DEFCONST MAIN-SCREEN-BUFFER-LENGTH #o100000)` --- 32,768 words, which is
/// more than the 23,112 the screen uses.
pub const BUFFER_WORDS: u32 = 0o100000;

/// `(DEFCONST MAIN-SCREEN-CONTROL-ADDRESS #o377760)`, as an Xbus I/O offset;
/// physical is that plus [`BUFFER`].  Eight words: the mode register, then
/// the sync program's data, pointer and enable, then three that answer
/// and do nothing.
pub const CONTROL: u32 = 0o17377760;

/// Words of the sync program RAM: the eight 2147s at NSYRAM 0A01-0B04,
/// 4K by 1 each, one per bit, addressed by the twelve bits of
/// [`SyncRam::pointer`]. `cpt.prom`'s 297 words and `SET-TV-SPEED`'s
/// program both fit many times over.
pub const SYNC_RAM_WORDS: usize = 4096;

/// How many control words the board answers on.  `lmtv.order` runs the
/// registers from `173777x0` to `173777x7`, the last three of which "respond
/// but don't do anything", and NXBCTL 0F13 is a 74S138 on `ADR0..2` whose
/// top three outputs go nowhere.  `bus-adaptor.c` decodes the same eight,
/// `017377760`-`017377767`.
pub const CONTROL_WORDS: u32 = 8;

/// `(DEFVAR MAIN-SCREEN-WIDTH (SELECT-PROCESSOR (:CADR 768.)))`
pub const WIDTH: usize = 768;

/// `(:CADR 963.)`, with MIT's own comment `;was 896. for CPT`.  The older
/// value is 896, which is the CPT monitor's.
pub const HEIGHT: usize = 963;

/// `(DEFVAR MAIN-SCREEN-LOCATIONS-PER-LINE (SELECT-PROCESSOR (:CADR 24.)))`
/// --- 24 words of 32 bits is the 768 pixels of a line, one bit each.
pub const WORDS_PER_LINE: usize = 24;

/// Mode register bits, read off the board's own "SIMPLE TV / MODE REGISTER"
/// drawing and named as `lmtv.order` names them. That is the word MIT's
/// window system uses in `'(:CONTROLLER :SIMPLE)`.
///
/// MIT drew this page twice. `synmod.drw` of 28 May 1979 has the register as
/// one **74S174**, a hex D flip-flop whose data pins are `XDI 3..0` alone,
/// read back through a **74S241** octal buffer on `XDO 7..0`. `nxbctl.drw`
/// of 17 May 1980, the newer sheet and the one in `data/SIMPLETV.netlist`,
/// does the same job with an **Am25LS2519** --- a quad register with a
/// second, three-state output set that puts the four bits straight back on
/// `XDO 3..0` --- and half a **74LS244** on `XDO 7..4`. Two parts, two
/// drawings, one interface: four bits latch and four are read through a
/// buffer. `tests/simpletv_netlist.rs` holds the constants below to the
/// netlist pin by pin.
///
/// `MODE<4>` is not in the register with the four below it: it is a flop
/// of its own, clocked by the same write --- [`mode::VERT`].
pub mod mode {
    /// `MODE<1:0>`, `CLOCK MODE 0` and `CLOCK MODE 1`: which dot clock the
    /// sync PROM is addressed with.
    pub const CLOCK: u32 = 0o3;
    /// `MODE<2>`, `MODE BOW`.  `BLACK-ON-WHITE` sets it --- "display one bits
    /// as black and zeros as white" --- with `(LOGIOR 4 ...)`, and
    /// `WHITE-ON-BLACK` clears it with `(LOGAND -5 ...)`, MIT commenting that
    /// as "1's comp of 4".
    pub const BOW: u32 = 0o4;
    /// `MODE<3>`, `MODE INTR ENB`: enables the vertical interrupt.
    pub const INTERRUPT_ENABLE: u32 = 0o10;

    /// `MODE<3:0>` --- the four data pins the register has, and the whole of
    /// what a write can change.
    pub const WRITABLE: u32 = 0o17;

    /// `MODE<4>`, `VERT FLAG`, "Causes Interrupt". Not in the 2519 with the
    /// four below it but a flop of its own, the 74LS74 at NXBCTL 0E14:
    /// **preset** by `-TVMA CLR`, the sync program's start of frame ---
    /// `lmtv.order`: "this is set by TVMA CLR, not by the start of Vertical
    /// Sync" --- and **clocked by `-LOAD MODE` with `XDI 4` as its data**,
    /// so a write of the register puts the written bit 4 into it. It reads
    /// back through the 74LS244 at 0F11. Microcode 323's `INTRX0` takes
    /// the interrupt by reading the register, testing this bit, and
    /// writing it back with the bit cleared. With [`INTERRUPT_ENABLE`] it
    /// is `SEND INTR` through the 74S08 at 0D10, onto `-XBUS.INTR`.
    pub const VERT: u32 = 0o20;
    /// `MODE<5>`, `VSYNC`.  Read only, "directly from sync generator".
    pub const VSYNC: u32 = 0o40;
    /// `MODE<6>`, `HSYNC`.  Read only.
    pub const HSYNC: u32 = 0o100;
    /// `MODE<7>`, `SYNC PROM ENB`.  Read only, and zero on this board.
    ///
    /// NXBCTL 0F11 drives `XDO 7` from a net of that name, so the bit reads
    /// back; `lmtv.order` calls everything from 7 up garbage.
    /// Both are right, because ECO 2 of `cadrtv/lmtv.eco`, 18 June 1980,
    /// grounds that buffer input --- "new window system not initializing tv
    /// properly at original power-up; on old TV boards the check if TV is in
    /// PROM mode (extant only on new TV boards) reads an unused input". The
    /// window system reads this bit to tell the boards apart, and on this
    /// one it must read zero.
    pub const SYNC_PROM_ENABLE: u32 = 0o200;

    /// `MODE<7:4>`, everything the read buffer sources from somewhere other
    /// than the 2519. [`VERT`] is the flop above; the other three are
    /// undriven here and read as zero.
    pub const READ_ONLY: u32 = 0o360;
}

/// One frame of the board's raster, and so the period of `TVMA CLR` and
/// the vertical flag: 966 lines of 16.000 us, measured on the netlist
/// board in `tests/simpletv_netlist.rs` and `tests/monitor.rs`. 64.7 Hz,
/// which is what the microcode calls "the roughly-60-cycle clock".
pub const FRAME_NS: u64 = 15_456_000;

/// The word offset into the frame buffer a physical address names, if it is
/// in it.
pub fn buffer_offset(phys: u32) -> Option<u32> {
    let off = phys.wrapping_sub(BUFFER);
    (off < BUFFER_WORDS).then_some(off)
}

/// Which control register a physical address names, if it is one.
pub fn control_register(phys: u32) -> Option<u32> {
    let off = phys.wrapping_sub(CONTROL);
    (off < CONTROL_WORDS).then_some(off)
}

/// The sync program RAM and its two registers, `lmtv.order`'s `173777x1`
/// to `x3`: "Sync Program [Sync Ptr] (read/write)", "Sync Ptr (write
/// only) Address for reading and writing the Sync Program RAM", and "Sync
/// Enable (write only, cleared by Xbus reset) ... Set it to 1 after you
/// have loaded the correct sync program" over the "Vertical Spacing".
///
/// The enable and the spacing are the 74LS273 at NTVINC 0A07, whose clear
/// (pin 1) is `-POWER RESET` and not the `-RESET` that `XBUS INIT IN`
/// makes; so where `lmtv.order` has the enable "cleared by Xbus reset",
/// the drawing has it cleared by the backplane's `-XBUS POWER RESET`, and
/// the drawing is followed: a bus reset leaves both standing
/// ([`SimpleTv::xbus_init`]), and power-on --- [`SyncRam::default`] ---
/// clears them.
///
/// The program in it is not run: the board's timing here is [`FRAME_NS`]
/// whatever is loaded. What is modelled is that a program written can be
/// read back, which `SI:SETUP-CPT` does, and that the enable and the
/// spacing hold what they were given. The enable is also what selects the
/// RAM over the PROM at NSYRAM --- the 2147s' chip select is `SYNC PROM
/// ENB` and the 74S472's its complement --- so with it clear a read of
/// the data register is the PROM's word, which reads as zero here, the
/// PROM's program not being loaded on this path.
#[derive(Clone)]
pub struct SyncRam {
    words: Vec<u8>,
    /// The twelve-bit address the next data access goes to.
    pub pointer: u16,
    /// Bit 7 the sync enable, 6-0 the vertical spacing.
    pub enable: u8,
}

impl Default for SyncRam {
    fn default() -> Self {
        SyncRam { words: vec![0; SYNC_RAM_WORDS], pointer: 0, enable: 0 }
    }
}

impl SyncRam {
    /// The program as loaded.
    pub fn words(&self) -> &[u8] {
        &self.words
    }

    /// Whether the software has turned the sync outputs on and the RAM in:
    /// bit 7 of register 3.
    pub fn enabled(&self) -> bool {
        self.enable & 0o200 != 0
    }
}

/// The frame buffer, the mode register, the vertical flag, and the sync
/// program RAM.
#[derive(Clone)]
pub struct SimpleTv {
    buffer: Vec<u32>,
    mode: u32,
    /// Registers 1 to 3.
    pub sync: SyncRam,
    /// The bit the last mode write clocked into the vertical flag's flop.
    flag_written: bool,
    /// When that write was, in the machine's nanoseconds: the flag is
    /// that bit, or the frame start that has come since.
    written_at: u64,
}

impl Default for SimpleTv {
    fn default() -> Self {
        SimpleTv {
            buffer: vec![0; BUFFER_WORDS as usize],
            mode: 0,
            sync: SyncRam::default(),
            flag_written: false,
            written_at: 0,
        }
    }
}

impl SimpleTv {
    /// The whole frame buffer, for whatever draws it.
    pub fn buffer(&self) -> &[u32] {
        &self.buffer
    }

    pub fn mode(&self) -> u32 {
        self.mode
    }

    /// One bits are black when [`mode::BOW`] is set, and white otherwise.
    pub fn black_on_white(&self) -> bool {
        self.mode & mode::BOW != 0
    }

    /// Whether the pixel at `x`, `y` is lit, ignoring which way round the
    /// screen is showing them.
    pub fn pixel(&self, x: usize, y: usize) -> bool {
        let bit = y * WORDS_PER_LINE * 32 + x;
        self.buffer[bit / 32] >> (bit % 32) & 1 != 0
    }

    /// Whether the monitor shows the pixel at `x`, `y` white: a lit bit is
    /// white unless [`mode::BOW`] is set, and the other way round when it
    /// is.
    ///
    /// The one place that rule lives on this side, so that everything
    /// drawing this screen draws the same screen. `crate::terminal` states
    /// it again over its own frame, which may be the monitor's raster
    /// rather than this buffer, and `tests/terminal.rs` holds the two to
    /// each other pixel for pixel.
    pub fn shows_white(&self, x: usize, y: usize) -> bool {
        self.pixel(x, y) != self.black_on_white()
    }

    /// The screen as the monitor shows it, as a PNG: 768 by 963, one bit a
    /// pixel, a one white unless [`mode::BOW`] is set.
    ///
    /// The encoder is here rather than a crate: a 1-bit greyscale PNG is a
    /// header, the rows behind stored deflate blocks, and two checksums.
    pub fn png(&self) -> Vec<u8> {
        let mut raw = Vec::with_capacity(HEIGHT * (WIDTH / 8 + 1));
        for y in 0..HEIGHT {
            raw.push(0); // filter: none
            for x in (0..WIDTH).step_by(8) {
                let mut byte = 0u8;
                for b in 0..8 {
                    if self.shows_white(x + b, y) {
                        byte |= 0x80 >> b;
                    }
                }
                raw.push(byte);
            }
        }
        let mut z = vec![0x78, 0x01];
        let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
        for (i, b) in blocks.iter().enumerate() {
            z.push((i + 1 == blocks.len()) as u8);
            z.extend_from_slice(&(b.len() as u16).to_le_bytes());
            z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
            z.extend_from_slice(b);
        }
        z.extend_from_slice(&adler32(&raw).to_be_bytes());

        let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&(WIDTH as u32).to_be_bytes());
        ihdr.extend_from_slice(&(HEIGHT as u32).to_be_bytes());
        ihdr.extend_from_slice(&[1, 0, 0, 0, 0]); // 1 bit, greyscale, deflate, none, no interlace
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &z);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    /// How many pixels are lit.
    pub fn lit(&self) -> usize {
        self.buffer[..HEIGHT * WORDS_PER_LINE].iter().map(|w| w.count_ones() as usize).sum()
    }

    pub fn read_buffer(&self, offset: u32) -> u32 {
        self.buffer[offset as usize]
    }

    pub fn write_buffer(&mut self, offset: u32, v: u32) {
        self.buffer[offset as usize] = v;
    }

    /// `VERT FLAG` at `ns`: what the last mode write put in the flop, or
    /// set, if a frame has started since --- `-TVMA CLR` presets it once
    /// every [`FRAME_NS`], the frames counted from power-on.
    pub fn vert_flag(&self, ns: u64) -> bool {
        self.flag_written || ns / FRAME_NS > self.written_at / FRAME_NS
    }

    /// `SEND INTR`: the vertical flag with [`mode::INTERRUPT_ENABLE`] up,
    /// which the board puts on `-XBUS.INTR` and the microcode takes at
    /// `INTRX0` as the 60-cycle clock.
    pub fn interrupt(&self, ns: u64) -> bool {
        self.mode & mode::INTERRUPT_ENABLE != 0 && self.vert_flag(ns)
    }

    /// The window system read-modify-writes this register, so what it writes
    /// has to read back; `INTRX0` reads it to find the vertical flag; and
    /// `SETUP-CPT` reads the sync program back through register 1.
    pub fn read_control(&self, register: u32, ns: u64) -> u32 {
        match register {
            0 => self.mode | if self.vert_flag(ns) { mode::VERT } else { 0 },
            // The sync program's word at the pointer, eight bits, while the
            // RAM is the one selected; `lmtv.order` calls 31-8 garbage.
            1 if self.sync.enabled() => self.sync.words[self.sync.pointer as usize] as u32,
            // 2 and 3 are write only, and 5 to 7 "respond but don't do
            // anything".
            _ => 0,
        }
    }

    /// The four pins of the 2519 land, bit 4 lands in the vertical flag's
    /// flop, and the sync program's three registers take theirs;
    /// everything else the write carries has nowhere to be stored.
    pub fn write_control(&mut self, register: u32, v: u32, ns: u64) {
        match register {
            0 => {
                self.mode = v & mode::WRITABLE;
                self.flag_written = v & mode::VERT != 0;
                self.written_at = ns;
            }
            1 => self.sync.words[self.sync.pointer as usize] = v as u8,
            2 => self.sync.pointer = (v as u16) & (SYNC_RAM_WORDS as u16 - 1),
            3 => self.sync.enable = v as u8,
            _ => {}
        }
    }

    /// `-XBUS INIT` on the backplane at `ns`: `XBUS INIT IN` off the 26S10
    /// at XBDATA 0F15, inverted to `-RESET` by the 74S04 at NXBCTL 0F10,
    /// and `-RESET` clears one flop on this board --- the vertical flag's
    /// 74LS74 at NXBCTL 0E14, pin 13.  The mode register, the 25LS2519 at
    /// 0F12 (pin 19), and the sync enable and spacing, the 74LS273 at
    /// NTVINC 0A07 (pin 1), clear on `-POWER RESET` instead, the board's
    /// receiver of the backplane's `-XBUS POWER RESET`, a wire of its own
    /// that the interface drives from the cable's `-BUS.POWER.RESET`
    /// (OLORD2 1A06).  Power-on is the only processor event that raises it,
    /// so here only [`SimpleTv::default`] clears them.  `-BUS.POWER.RESET`
    /// is the 74S37 at OLORD2 1A06 inverting `POWER RESET A`, and `POWER
    /// RESET A` is `-POWER RESET` inverted by the 74S02 at 1A11 (pin 8 on
    /// ground); `-POWER RESET` comes off the 74LS14 at 1A20 from the
    /// resistor-capacitor network at 1A19, which has nothing on it but VCC,
    /// ground and that Schmitt, so nothing the microcode does reaches it.
    /// `PROG.BUS.RESET`, the other input of the 74S02 at 1A07, is the
    /// programmed reset and goes the other way, to `-BUS.RESET`: it is
    /// bit 28 of the Interrupt Control register, the 25LS2519 at FLAG 3E08
    /// where `OB28` is `PROG.UNIBUS.RESET`, which AIM-528 describes as
    /// "Bit <28>, BUS-RESET, generates a RESET signal on the Unibus (BUS
    /// INIT L) and on the Xbus (XBUS.INIT L), and resets the bus interface,
    /// when it is written 1 and then 0.  The machine also resets the busses
    /// when it is powered up."  `-BUS.RESET` is the cable wire that becomes
    /// `-LM UNIBUS RESET` at DBGIN 0A14, and `-BUS.POWER.RESET` the one
    /// that becomes `-LM POWER RESET` at XA 0B13, which the 26S10 at XA
    /// 0F21 puts on `-XBUS POWER RESET`; no other part drives it.
    /// The flag is preset again by the next frame's `-TVMA CLR`.
    pub fn xbus_init(&mut self, ns: u64) {
        self.flag_written = false;
        self.written_at = ns;
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(!0u32, |c, &b| {
        (0..8).fold(c ^ b as u32, |c, _| if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 })
    })
}

fn adler32(bytes: &[u8]) -> u32 {
    let (a, b) = bytes.iter().fold((1u32, 0u32), |(a, b), &x| {
        let a = (a + x as u32) % 65521;
        (a, (b + a) % 65521)
    });
    b << 16 | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

// --- Checkpoints ------------------------------------------------------------

impl SyncRam {
    pub fn save(&self, w: &mut crate::checkpoint::Writer) {
        let SyncRam { words, pointer, enable } = self;
        w.bytes(words);
        w.u16(*pointer);
        w.u8(*enable);
    }

    pub fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        r.bytes_into(&mut self.words)?;
        // The pointer indexes `words`, and a write to register 2 keeps the
        // twelve bits that address the 2147s, so a wider one is a corrupt
        // checkpoint: refused here, not indexed with at the next access.
        let pointer = r.u16()?;
        if pointer as usize >= SYNC_RAM_WORDS {
            return Err(crate::checkpoint::bad(format!(
                "sync pointer {pointer:o}, past the {SYNC_RAM_WORDS} words its twelve bits address"
            )));
        }
        self.pointer = pointer;
        self.enable = r.u8()?;
        Ok(())
    }
}

impl SimpleTv {
    /// The display into a checkpoint: the frame buffer, the mode, the sync
    /// RAM and the vertical flag.
    pub fn save(&self, w: &mut crate::checkpoint::Writer) {
        let SimpleTv { buffer, mode, sync, flag_written, written_at } = self;
        w.u32s(buffer);
        w.u32(*mode);
        sync.save(w);
        w.bool(*flag_written);
        w.u64(*written_at);
    }

    pub fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        r.u32s_into(&mut self.buffer)?;
        self.mode = r.u32()?;
        self.sync.load(r)?;
        self.flag_written = r.bool()?;
        self.written_at = r.u64()?;
        Ok(())
    }
}
