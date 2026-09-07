// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The I/O board: the keyboard, the mouse, the clocks and their status
//! register, on the Unibus.
//!
//! MIT's own name for the card is the I/O BOARD, and `iob.wls` is its section
//! census. **`IOB` in this codebase is the bus that merges `I` with `OB` into
//! the instruction register** --- `IOB<47:0>` on IREG --- and stays that; the
//! card is the I/O board.
//!
//! Three sources, reaching us by three routes:
//!
//! 1. **`sys/ucadr/uc-cadr.lisp`** in the System 100 release --- microcode
//!    323 itself, reading this board. It names Unibus `764112` "KBD CSR" and
//!    `764100` "KBD LOW", gives their physical addresses, and tests the
//!    keyboard-ready bit as `(BYTE-FIELD 1 5)`.  **primary**
//! 2. **`sys/ucadr/uc-interrupt.lisp` and `sys/io1/time.lisp`**, for
//!    `MICROSECOND-CLOCK-UNIBUS-ADDRESS 764120`,
//!    `INTERVAL-TIMER-UNIBUS-ADDRESS 764124`, and MIT's own note that the
//!    hardware latches the clock when the low half is read first.  **primary**
//! 3. **`cadrio/iobcsr.drw` in `mit/`**, the status register's own sheet
//!    (`data/CADRIO.netlist`'s IOBCSR page is made from it): the
//!    four interrupt enables are one **74LS175**, a quad D flip-flop, and
//!    `KBD READY`, `MOUSE READY` and `CLOCK READY` come back through a
//!    **74LS244**.  **primary**
//!
//! This is the first thing microcode 323 touches after the boot PROM lets go.
//! `uc-cadr.lisp` enters at `(LOC 6)`, reads the CSR, and takes the cold boot
//! unless the keyboard is ready and holding something other than RUBOUT. With
//! nothing answering there the read is an NXM and the machine cold-boots by
//! accident rather than by decision.
//!
//! Where this is knowingly not the machine: the mouse's counters count what
//! they are told rather than a quadrature pair ([`IoBoard::mouse_move`]);
//! and the clocks are driven from simulated time rather than the
//! computer's, which makes a run reproducible.

/// Keyboard, low sixteen bits of the scan code.  Reading either half clears
/// [`csr::KBD_READY`].
pub const KBD_LOW: u32 = 0o764100;
/// Keyboard, high sixteen bits.
pub const KBD_HIGH: u32 = 0o764102;
/// Mouse Y, with the three buttons above it.  Reading it clears
/// [`csr::MOUSE_READY`].
///
/// The word, off the 74LS244 at IOBMS2 0C24 and the three 74LS569s at
/// 0B24-0B26: bits 11-0 the Y count, bit 12 `MOUSE TAILSW`, 13 `MOUSE
/// MIDSW`, 14 `MOUSE HEADSW`, 15 ground. MIT's `io1/mouse.text` reads the
/// same three as `10000`, `20000` and `40000`, and its `doc/mouse.text`
/// gives the software's mask as "the 1-bit applies to the left button,
/// the 2-bit to the middle button, and the 4-bit to the right button" ---
/// so the tail switch is the left button and the head switch the right.
/// Microcode 323's `TRACK-MOUSE` takes the three as `(BYTE-FIELD 3 12.)`.
pub const MOUSE_Y: u32 = 0o764104;
/// Mouse X, with the raw quadrature above it.
///
/// Bits 11-0 the X count off the 74LS569s at 0B27-0B29; then, through the
/// other half of the 74LS244, bit 12 `NEW HORA`, 13 `NEW HORB`, 14 `NEW
/// VERA`, 15 `NEW VERB`: the four quadrature lines as last latched.
pub const MOUSE_X: u32 = 0o764106;

/// Bits of the two mouse registers.
pub mod mouse {
    /// The count in each register, twelve bits: three 74LS569s an axis.
    /// `TRACK-MOUSE` takes the difference from last time modulo this and
    /// sign-extends from bit 11.
    pub const COUNT: u16 = 0o7777;
    /// Where the buttons and the quadrature sit, above the count.
    pub const SHIFT: u32 = 12;
    /// `MOUSE TAILSW`, the left button, in the Y register.
    pub const TAIL: u16 = 1 << 12;
    /// `MOUSE MIDSW`, the middle button.
    pub const MIDDLE: u16 = 1 << 13;
    /// `MOUSE HEADSW`, the right button.
    pub const HEAD: u16 = 1 << 14;
    /// The three, as the software's mask: left 1, middle 2, right 4.
    pub const BUTTONS: u8 = 0o7;
}
/// The beep.  Written by `%BEEP`, and read by older code to the same effect.
///
/// **The register has no value in it.**  `-CLICK.AUDIO` is `Y4` of the
/// 74LS138 at IOBKBD 0C22, whose enables are `-SELECT.764100` and
/// `-SELECT KBD OR MOUSE` and neither is `-WRITE`, so a read clicks as a
/// write does; it clocks the 74LS74 at 0C27 wired as a toggle, `Q` back to
/// its own `D`, and that `Q` is `AUDIO` into the 75118 at 0F30 and off the
/// board as `AUDIO+`/`AUDIO-`.  So one reference is one edge of a square
/// wave, and the tone is the rate the microcode references it at:
/// `uc-hacks.lisp`'s `XBEEP` is that loop, "First argument is
/// half-wavelength, second is duration.  Both are in microseconds",
/// writing `BEEP-HARDWARE-VIRTUAL-ADDRESS` once every half-wavelength.
/// `tests/cadrio_netlist.rs` holds this model to the netlist board.
pub const BEEP: u32 = 0o764110;

/// How long the speaker must have been quiet for the next click to count
/// as a new beep rather than more of the last one, for
/// [`IoBoard::take_beep`]: 50 milliseconds.
///
/// This is the far end's arithmetic and not the board's --- the board has
/// only the flip-flop --- but it is made here, where the machine's own
/// clock is.  The lowest note anyone can hear is about 20 Hz, which is 25
/// milliseconds a half cycle, so no audible tone is broken into two beeps;
/// MIT's own `BEEP-WAVELENGTH` of `1350` octal is 744 microseconds, sixty
/// times under it.
pub const AUDIO_QUIET_NS: u64 = 50_000_000;
/// The status register.  `uc-cadr.lisp`: "Unibus address 764112 (KBD CSR)".
pub const CSR: u32 = 0o764112;
/// Microsecond clock, low sixteen bits.  `MICROSECOND-CLOCK-UNIBUS-ADDRESS`.
/// MIT: "Hardware synchronizes if you read this one first."
pub const USEC_LOW: u32 = 0o764120;

/// From power-on to the first rising edge of the board's microsecond
/// clock, `1 USEC CLK`, the 74S163 at IOBCLK 0C21 counting the 16 MHz
/// `MCLK^` from its power-on state; the edges are every 1,000 ns from
/// there, and no Unibus reset moves them. Measured on the netlist
/// (`tests/cadrio_netlist.rs`).
pub const FIRST_USEC_EDGE_NS: u64 = 890;

/// The microsecond counter as it stands at `ns`: how many edges of the
/// microsecond clock have come by then, since power-on.
pub fn usec_at(ns: u64) -> u32 {
    ((ns + 1_000 - FIRST_USEC_EDGE_NS) / 1_000) as u32
}
/// Microsecond clock, high sixteen bits.
pub const USEC_HIGH: u32 = 0o764122;
/// Written, the interval timer: `INTERVAL-TIMER-UNIBUS-ADDRESS 764124`.
/// Read, the sixty-cycle clock.  The 74LS138 at CLK60H 0B21 decodes the
/// group under `-SELECT.764120`: `UBADDR1` and `UBADDR2` on `A` and `B`,
/// `-WRITE` on `C`, so `Y2` is the write of `764124`, `-LOAD INTERVAL`, and
/// `Y6` is the read of it, `-READ SCL`.  `-READ SCL` gates the two 74LS244s
/// at CLKTOD 0D28 and 0B22, which put `SCL0`..`SCL15` on `UBO0`..`UBO15` in
/// order: the sixteen bits of the two 74393 counters at CLKTOD 0D22 and
/// 0D23, clocked by `60 Hz` with their clears on ground, so it is a
/// free-running count of mains cycles since power-on --- which is what the
/// model returns.
pub const CLOCK: u32 = 0o764124;
/// General purpose I/O.  Nothing is wired to it.
pub const GPIO: u32 = 0o764126;

/// The Unibus interrupt vector the board puts on the bus for the
/// keyboard: `260`. From the channel MIT's keyboard driver sets up ---
/// `lmio/kbd.123` in the System 46 sources, `(%P-DPB 260 ...
/// %UNIBUS-CHANNEL-VECTOR-ADDRESS)`, with `764112` as its CSR, `40` as
/// the CSR bit to test, which is [`csr::KBD_READY`], and `764100` as a
/// two-register data address --- the System 100 driver being in the band
/// only. The board's own vector is on its Unibus interrupt cycle, which
/// the netlist runs for real under `chip`; that the microcode finds the
/// channel there is the cross-check on this number.
pub const KBD_VECTOR: u16 = 0o260;

/// The serial port's registers, `764160`-`764176`: the 2651 at IOBSER 0A12
/// on `A<2:1>` under `-SELECT.764160`, every address of the group answered
/// through the two 74LS74s at 0F29 on the half-microsecond clock, read or
/// written.  The four registers are [`crate::serial`]'s, `DATA`, `STATUS`,
/// `MODE` and `COMMAND`, and `A3` is not decoded, so the upper four
/// addresses are the same again.  The chip drives `UBO0`..`UBO7` on a
/// read and nothing drives the upper byte, which floats high through
/// the 8838s as the status register's does.
pub const SERIAL_FIRST: u32 = 0o764160;
pub const SERIAL_LAST: u32 = 0o764176;

/// The serial port's Unibus interrupt vector, `264`, and the board's
/// other three.  Page IOBINT: the 74S175 at 0F14 latches `KBD/MOUSE.IREQ`,
/// `SER.IREQ`, `CHAOS.IREQ` and `CLOCK.IREQ` on the grant, `BG.IN`, and
/// the 74LS00s at 0E12 make two vector bits from its outputs, `V2 = (SER
/// AND NOT CHAOS) OR CLOCK` and `V3 = CLOCK OR CHAOS`, which the 74S38s
/// at 0F16 put on `-D2*` and `-D3*` under `MASTER B` beside `-D4*`,
/// `-D5*` and `-D7*`, held down for every vector.  So `260` for the
/// keyboard and mouse, `264` for the serial port, `270` for the Chaosnet
/// and `274` for the clock, and with more than one latched the clock is
/// named before the Chaosnet before the serial port before the keyboard.
/// System 100's `sys/io1/serial.lisp` sets its channels up on `264`, and
/// [`crate::chaos::board::VECTOR`] is the `270`.
pub const SERIAL_VECTOR: u16 = 0o264;

/// The interval timer's Unibus interrupt vector, `274`: microcode 323's own
/// `(ASSIGN INTERVAL-TIMER-VECTOR 274)` in `uc-interrupt.lisp`, which is
/// also what page IOBINT's two vector bits come to with `CLOCK.IREQ`
/// latched --- see [`SERIAL_VECTOR`] for the two equations. `V2` and `V3`
/// are both one under the clock, so it is named before all three others.
pub const CLOCK_VECTOR: u16 = 0o274;

/// One count of the interval timer, in nanoseconds: `16 USEC CLK` drives
/// the counters' count-down input, and `iob.wlr` puts `F21-04 CNT DW` on
/// that net with `F21-05 CNT UP` on `HI3`.
pub const INTERVAL_TICK_NS: u64 = 16_000;

/// Bits of the status register.
///
/// `iobcsr.drw` names the signals, and `data/CADRIO.netlist` places them:
/// the 74LS244 at 0D29 puts `REMOTE MOUSE ENABLE`, `MOUSE INT ENABLE`,
/// `KBD INT ENABLE`, `CLOCK INT ENABLE`, `MOUSE READY`, `KBD READY`,
/// `CLOCK READY` and `SER INT ENABLE` on `UBO0` to `UBO7` in that order,
/// and nothing drives `UBO8` to `UBO15` on a read of this register, so
/// they float high through the 8838s and the upper byte reads as ones.
/// The netlist board says the same in `tests/cadrio_netlist.rs`.
pub mod csr {
    /// `REMOTE MOUSE ENABLE`, the first of the 74LS175's four.
    pub const REMOTE_MOUSE_ENABLE: u16 = 1 << 0;
    /// `MOUSE INT ENABLE`.
    pub const MOUSE_INT_ENABLE: u16 = 1 << 1;
    /// `KBD INT ENABLE`.
    pub const KBD_INT_ENABLE: u16 = 1 << 2;
    /// `CLOCK INT ENABLE`, the last of the four.
    pub const CLOCK_INT_ENABLE: u16 = 1 << 3;
    /// `MOUSE READY`.  Cleared when the mouse Y register is read.
    pub const MOUSE_READY: u16 = 1 << 4;
    /// `KBD READY`.  Microcode 323 tests exactly this bit ---
    /// `(JUMP-IF-BIT-CLEAR (BYTE-FIELD 1 5) MD ...)`, "If keyboard is not
    /// ready" --- so this one is MIT's own, and the netlist agrees.
    pub const KBD_READY: u16 = 1 << 5;
    /// `CLOCK READY`.  Not the sixty-cycle clock at all: it is the
    /// interval timer's, one latch of the 74LS279 at CLKTIM 0D09, set by
    /// `-INTERVAL OVER` on `1S2` and cleared by `-LOAD INTERVAL` on `1R`.
    /// `-INTERVAL OVER` is the borrow out of the four 74LS193s at CLKTIM
    /// 0F21, 0E21, 0F22 and 0E22, loaded from `UBI0`..`UBI15` by the write
    /// of [`super::CLOCK`] and counting down on `16 USEC CLK` at pin 4 (the
    /// `DOWN` input, `sn74193.pdf`), their clears on ground.  So the bit
    /// comes up when the loaded interval runs out and goes down when a new
    /// one is loaded, which is [`super::IoBoard::clock_ready`].  Before any
    /// load the latch reads set, which is what the netlist board reads from
    /// reset.
    ///
    /// **Down from what was written, not up to zero.**  `iob.wlr`, the
    /// board as wrapped, puts `16 USEC CLK` on `F21-04 CNT DW` and
    /// `F21-05 CNT UP` on `HI3`, and takes `F21-13 BORROW` up the chain
    /// while `F21-12 CARRY` goes nowhere; MIT's `doc/iob.text` agrees ---
    /// storing `n` "turns off clock ready CSR<6>, delays 16 x `n`
    /// microseconds, then turns clock ready back on".  Microcode 323 says
    /// the opposite of its own board and writes the two's complement:
    /// `((MD) (A-CONSTANT 1_20))` then `((MD) SUB MD A-T)` under the
    /// comment ";Timer counts up, not down", and `doc/unaddr.text` says
    /// increments too.  Discrepancy 74.  The wire list is followed here,
    /// which costs nothing on System 100: that microcode is `INTR-OUTDEV`,
    /// reached only with `A-UNIBUS-TIMED-OUTPUT-CSR-ADDRESS` set up, and
    /// nothing in the release sets `CLOCK INT ENABLE` at all.
    pub const CLOCK_READY: u16 = 1 << 6;
    /// `SER INT ENABLE`: not one of the 74LS175's four but the second
    /// half of the 74LS74 at IOBSER 0D21, with `UBI7` on its `D`, the
    /// same `KBD/MOUSE.CSR.CLK^` on its clock and `-RESET` on its clear,
    /// so a write of this register sets it as it sets the other four.
    /// `serial.lisp` writes bit 7 of `764112` to turn its interrupt on and
    /// off.  With it up, `SER.IREQ` is the 74LS02 at 0E11 taking
    /// `-SER RRDY`, which is the 2651's `-RxRDY` and, by ECO 10 of
    /// `cadrio/iob.eco`, its `-TxRDY` on the same net.
    pub const SER_INT_ENABLE: u16 = 1 << 7;
    /// What the upper byte reads as: nothing drives it.
    pub const FLOATING: u16 = 0o177400;

    /// The four flip-flops of the 74LS175 at IOBCSR 0D27 and the serial
    /// enable's at IOBSER 0D21: the whole of what a write can change.
    pub const WRITABLE: u16 = 0o217;
}

/// The nominal microcycle: 145 ns, the period at normal speed, which
/// `tests/clock.rs` pins.  `micro`, the engine with no clock, multiplies its
/// microcycles by this to give the clocks here a time, and keeps worse time
/// than the board would at any other speed or stalled on memory; `rtl` and
/// `chip` give them their own nanoseconds.
pub const CYCLE_NS: u64 = 145;

/// One 60-cycle tick, in nanoseconds.
pub const SIXTY_CYCLE_NS: u64 = 1_000_000_000 / 60;

/// Whether a Unibus address is one of this board's registers.
pub fn register(uaddr: u32) -> Option<u32> {
    answers(uaddr, false)
}

/// What the board's decoder makes of Unibus address `uaddr` for a read
/// (`write` false) or a write: the register the cycle reaches, by its own
/// address, or none --- no `-SSYN`, and the master times out.  Page
/// IOBADR: the DM8136s at 0F08 and 0F09 select the block `764000`-`764176`
/// on `A<17:7>`, the 74LS138 at 0E20 splits it into eight groups on
/// `A<6:4>`, and each group decodes `A<3:1>` its own way.  Read off the
/// netlist and measured on it, address by address
/// (`the_board_and_the_model_decode_the_block_alike` in
/// `tests/cadrio_netlist.rs`):
///
/// - `764000`-`764076`: the four selects go nowhere.
/// - `764100`-`764116`, keyboard and mouse: eight registers on `A<3:1>`,
///   every one answered read or written; `764114` and `764116` have nothing
///   behind them.
/// - `764120`-`764136`, the clocks and the GPIO: `A<2:1>` alone, so
///   `76413x` is `76412x`; the microsecond counter's halves take no write.
/// - `764140`-`764156`, the Chaosnet: the 74LS138 at LMUCON 0C18 decodes
///   `A<2:1>` and read against write, and `A3` only makes a read of the
///   address START (`764152`) and disables the receive buffer's read
///   (`764154`, unanswered); writes reach the CSR and the transmit buffer,
///   at `764150` and `764152` as at `764140` and `764142`, and nothing else.
/// - `764160`-`764176`, the serial port: every address answered.
pub fn answers(uaddr: u32, write: bool) -> Option<u32> {
    use crate::chaos::interface as chaos;
    if uaddr & 1 != 0 || !(0o764000..=0o764176).contains(&uaddr) {
        return None;
    }
    let a3 = uaddr & 0o10 != 0;
    match (uaddr >> 4) & 7 {
        4 => Some(uaddr),
        5 => {
            let r = uaddr & !0o10;
            (!write || (r != USEC_LOW && r != USEC_HIGH)).then_some(r)
        }
        6 => match (uaddr & 0o6, write) {
            (0, _) => Some(chaos::CSR),
            (2, true) => Some(chaos::WRITE_BUFFER),
            (2, false) => Some(if a3 { chaos::START } else { chaos::MY_ADDRESS }),
            (4, false) if !a3 => Some(chaos::READ_BUFFER),
            (6, false) => Some(chaos::BIT_COUNT),
            _ => None,
        },
        7 => Some(uaddr),
        _ => None,
    }
}

/// Whether `uaddr` is in the Chaosnet interface's group of the block,
/// `764140`-`764156`.
pub fn chaos_register(uaddr: u32) -> bool {
    (0o764140..=0o764156).contains(&uaddr) && uaddr & 1 == 0
}

/// The board.
#[derive(Default, Clone)]
pub struct IoBoard {
    /// The four enables and the two readies; the rest of the register is
    /// made up on a read.
    csr: u16,
    /// The scan code the keyboard last delivered.
    scancode: u32,
    /// The microsecond clock, latched when the low half is read.
    usec: u32,
    /// What the interval timer was last loaded with, in units of 16 us.
    interval: u16,
    /// When it was loaded, on the machine's clock, or `None` before any
    /// write of [`CLOCK`]: what [`IoBoard::clock_ready`] counts from.
    interval_loaded_at: Option<u64>,
    /// The mouse's two counters, twelve bits each.
    mouse_x: u16,
    mouse_y: u16,
    /// The three switches, as the software's mask: left 1, middle 2,
    /// right 4.
    mouse_buttons: u8,
    /// `AUDIO`, the 74LS74 at IOBKBD 0C27: the speaker's own flip-flop,
    /// toggled by every reference to [`BEEP`].
    audio: bool,
    /// When it last toggled, on the machine's clock.
    audio_click_ns: Option<u64>,
    /// Whether the speaker has started up since [`IoBoard::take_beep`] was
    /// last asked.
    beep_started: bool,
    /// The Chaosnet interface, when one is plugged in.
    pub chaos: Option<crate::chaos::board::Interface>,
    /// The serial port's 2651, with its cable.
    pub serial: crate::serial::Pci,
}

impl IoBoard {
    pub fn csr(&self) -> u16 {
        self.csr
    }

    /// What the interval timer was last loaded with.  It counts down from
    /// there at [`INTERVAL_TICK_NS`] a count; [`IoBoard::clock_ready`] is
    /// where it has got to.
    pub fn interval_timer(&self) -> u16 {
        self.interval
    }

    /// `CLOCK READY` at `ns`: the 74LS279 at CLKTIM 0D09, set from reset and
    /// again by `-INTERVAL OVER`, cleared by `-LOAD INTERVAL`.  So it is
    /// down from the write of [`CLOCK`] until the counters have taken the
    /// loaded number of `16 USEC CLK` edges.  See [`csr::CLOCK_READY`] for
    /// the direction, which is discrepancy 74.
    pub fn clock_ready(&self, ns: u64) -> bool {
        match self.interval_loaded_at {
            None => true,
            Some(at) => ns.saturating_sub(at) >= self.interval as u64 * INTERVAL_TICK_NS,
        }
    }

    /// A word came in off the keyboard.  Sets `KBD READY`, which is what
    /// microcode 323 tests at `(LOC 6)` to decide between a warm and a cold
    /// boot; a word landing on one not yet read replaces it, as the three
    /// 74LS164s at IOBKBD shift the next word in over the last.
    pub fn press(&mut self, scancode: u32) {
        self.scancode = scancode;
        self.csr |= csr::KBD_READY;
    }

    pub fn keyboard_ready(&self) -> bool {
        self.csr & csr::KBD_READY != 0
    }

    /// The vector of the Unibus interrupt the board is requesting at
    /// `now`, if it is.  Three 74LS08s at IOBKBD 0D26 make `KBD.IREQ`,
    /// `MOUSE.IREQ` and `CLOCK.IREQ`, each a ready bit with its enable, and
    /// the 74LS32 at 0D25 ors the first two into `KBD/MOUSE.IREQ`;
    /// `SER.IREQ` is the serial port's ready with `SER INT ENABLE` through
    /// the 74LS02 at IOBSER 0E11.  All four reach the bus request through
    /// the 74S260 at 0E14.
    ///
    /// With more than one up, the vector is the one the board's latch would
    /// name --- see [`SERIAL_VECTOR`] for the two equations that make it:
    /// the clock's before the Chaosnet's before the serial port's before
    /// the keyboard and mouse's, which they share.
    pub fn interrupt_request(&self, now: u64) -> Option<u16> {
        (self.csr & csr::CLOCK_INT_ENABLE != 0 && self.clock_ready(now))
            .then_some(CLOCK_VECTOR)
            .or_else(|| self.chaos.as_ref().and_then(|c| c.interrupt_request()))
            .or_else(|| {
                (self.csr & csr::SER_INT_ENABLE != 0
                    && (self.serial.rx_ready_at(now) || self.serial.tx_ready_at(now)))
                .then_some(SERIAL_VECTOR)
            })
            .or_else(|| {
                let kbd = self.csr & csr::KBD_READY != 0 && self.csr & csr::KBD_INT_ENABLE != 0;
                let mouse =
                    self.csr & csr::MOUSE_READY != 0 && self.csr & csr::MOUSE_INT_ENABLE != 0;
                (kbd || mouse).then_some(KBD_VECTOR)
            })
    }

    /// Plugs a Chaosnet interface in, with its switches at `address` and
    /// the cable `ether` on it --- or no cable --- powered at `powered_at`
    /// on the machine's clock, where its turn timer starts.
    pub fn plug_chaos(
        &mut self,
        address: u16,
        ether: Option<crate::chaos::ether::Ether>,
        powered_at: u64,
        trace: bool,
    ) {
        self.chaos = Some(crate::chaos::board::Interface::new(address, ether, powered_at, trace));
    }

    /// Time passes to `now` for whatever on the board keeps time between
    /// the processor's cycles: the Chaosnet cable, and the serial port's
    /// characters going out and coming in.
    pub fn advance(&mut self, now: u64) {
        if let Some(c) = self.chaos.as_mut() {
            c.advance(now);
        }
        self.serial.advance(now);
    }

    /// `-UB INIT` on the Unibus: `-INIT*` into the 8837 at IOBXCV 0F06 is
    /// `RESET`, which is the 2651's own `RESET` on its pin 21, and
    /// `-RESET` off the 74S37 at 0E07 clears the five interrupt enables
    /// --- the 74LS175 at IOBCSR 0D27, pin 1, and the 74LS74 at IOBSER
    /// 0D21, pin 13 --- and the
    /// Chaosnet interface's register, the 74LS174 at LMUCON 0B20, pin 1;
    /// `-INIT` also sets its Transmit Done (the 74S08 at LMMODU 0B10) and
    /// clears its receiver's busy (the 74S08 at LMRCTL 0E10), which is
    /// AIM-628's "just as at power up and Unibus Initialize",
    /// [`crate::chaos::board::Interface::reset`].  Nothing else on the board
    /// has a pin on it: `KBD READY` (the 74LS74 at IOBKBD 0B30) clears on
    /// `-READ.KBD.LOW`, `MOUSE READY` (the 74LS109 at IOBCSR 0C26) on
    /// `-READ.MOUSE.Y`, and the mouse counters (the 74LS569s at IOBMS2
    /// 0B24-0B29), the interval timer (CLKTIM) and the microsecond counter
    /// (IOBCLK) count on.
    pub fn unibus_init(&mut self) {
        self.csr &= !csr::WRITABLE;
        if let Some(c) = self.chaos.as_mut() {
            c.reset();
        }
        self.serial.reset();
    }

    /// The mouse moved: `dx` counts to the right and `dy` down, into the
    /// two twelve-bit counters. On the board the 74LS569s count one on
    /// each valid change of a quadrature pair, sampled on `KB CLK^`; here
    /// the counts arrive whole. `TRACK-MOUSE` adds the difference to its
    /// internal position without inversion, so a count up is the screen's
    /// x to the right and y down --- `io1/mouse.text`'s "NOTE Y-COORD
    /// INVERTED" is that older program's own convention.
    ///
    /// Any change is `MOUSE STATUS CHANGE` off the 25LS2521 at IOBMSE
    /// 0A21, which sets `MOUSE READY`.
    pub fn mouse_move(&mut self, dx: i32, dy: i32) {
        if dx == 0 && dy == 0 {
            return;
        }
        self.mouse_x = (self.mouse_x as i32 + dx).rem_euclid(1 << mouse::SHIFT) as u16;
        self.mouse_y = (self.mouse_y as i32 + dy).rem_euclid(1 << mouse::SHIFT) as u16;
        self.csr |= csr::MOUSE_READY;
    }

    /// The buttons as they now stand, as the software's mask. A change
    /// sets `MOUSE READY` as a move does.
    pub fn mouse_buttons(&mut self, mask: u8) {
        let mask = mask & mouse::BUTTONS;
        if mask != self.mouse_buttons {
            self.mouse_buttons = mask;
            self.csr |= csr::MOUSE_READY;
        }
    }

    pub fn mouse_x(&self) -> u16 {
        self.mouse_x
    }

    pub fn mouse_y(&self) -> u16 {
        self.mouse_y
    }

    pub fn mouse_ready(&self) -> bool {
        self.csr & csr::MOUSE_READY != 0
    }

    /// `AUDIO`, the level the speaker's pair is being driven to.
    pub fn audio(&self) -> bool {
        self.audio
    }

    /// Whether the speaker has started up since this was last asked ---
    /// handed out once, as the keys and the mouse's motion are.
    ///
    /// A run of clicks is one beep and a click after [`AUDIO_QUIET_NS`] of
    /// silence starts another, because that is what a far end can use: the
    /// board itself has no notion of a beep, only of edges.  RFC 6143's
    /// `Bell` has no duration and no pitch in it either, so one bell for
    /// one beep is the whole of what reaches a viewer.
    ///
    /// `(%BEEP 0 duration)` is reported as a beep though MIT means it for
    /// silence: with no half-wavelength `XBEEP` clicks as fast as it can,
    /// which on the real speaker is too high to hear.  Telling that from a
    /// tone would mean modelling what the speaker can reproduce, which is
    /// further than this goes.
    pub fn take_beep(&mut self) -> bool {
        std::mem::take(&mut self.beep_started)
    }

    /// `-CLICK.AUDIO`: one reference to [`BEEP`], read or write.
    fn click_audio(&mut self, ns: u64) {
        self.audio = !self.audio;
        let quiet = self.audio_click_ns.is_none_or(|last| ns - last >= AUDIO_QUIET_NS);
        self.beep_started |= quiet;
        self.audio_click_ns = Some(ns);
    }

    /// The buttons as the board holds them, as the software's mask. A
    /// look, not a read: the register read is what clears `MOUSE READY`.
    pub fn mouse_buttons_held(&self) -> u8 {
        self.mouse_buttons
    }

    /// The four quadrature lines as the board would have latched them
    /// for these counts: each axis a two-bit Gray code of its count, `A`
    /// then `B`, which is what an encoder gives as it turns.
    fn quadrature(&self) -> u16 {
        let gray = |count: u16| {
            let p = count & 3;
            p ^ (p >> 1)
        };
        let (gx, gy) = (gray(self.mouse_x), gray(self.mouse_y));
        // HORA, HORB, VERA, VERB in bits 12 to 15.
        (gx >> 1 & 1) << 12 | (gx & 1) << 13 | (gy >> 1 & 1) << 14 | (gy & 1) << 15
    }

    /// `ns` is the machine's simulated time, which the clocks here count:
    /// [`crate::machine::Machine::ns`].
    pub fn read(&mut self, uaddr: u32, ns: u64) -> u16 {
        let Some(r) = answers(uaddr, false) else { return 0o177777 };
        if chaos_register(r) {
            return match self.chaos.as_mut() {
                Some(c) => c.read(r, ns),
                None => 0o177777,
            };
        }
        match r {
            // `KBD READY` is the 74LS74 at IOBKBD 0B30, clocked by `EOC.KBD^`
            // and cleared by `-READ.KBD.LOW` on its pin 1, by nothing else:
            // the low half's read takes the word and the high half's leaves
            // it.  Microcode 323's Unibus channel reads the high half first
            // (`uc-interrupt.lisp`, "needs to read the high-order word
            // first"), so the word stands until both halves are in, and a
            // keyboard that hands over its next word on `KBD READY` alone
            // cannot put one in between.
            KBD_LOW => {
                self.csr &= !csr::KBD_READY;
                self.scancode as u16
            }
            // Eight bits of scan code, and a floating upper byte.
            KBD_HIGH => csr::FLOATING | ((self.scancode >> 16) as u16 & 0xff),
            MOUSE_Y => {
                self.csr &= !csr::MOUSE_READY;
                (self.mouse_buttons as u16) << mouse::SHIFT | (self.mouse_y & mouse::COUNT)
            }
            MOUSE_X => self.quadrature() | (self.mouse_x & mouse::COUNT),
            CSR => {
                let clock = if self.clock_ready(ns) { csr::CLOCK_READY } else { 0 };
                self.csr | clock | csr::FLOATING
            }
            // "Hardware synchronizes if you read this one first": the low
            // half latches the whole thing, so the high half cannot be from a
            // later microsecond than the low one.
            // `ns` is `-MSYN`'s instant: the netlist latches the count as it
            // stands then, the edge that answers the read not yet counted.
            USEC_LOW => {
                self.usec = usec_at(ns);
                self.usec as u16
            }
            USEC_HIGH => (self.usec >> 16) as u16,
            CLOCK => (ns / SIXTY_CYCLE_NS) as u16,
            // The serial port: the 2651's register on the low byte, and
            // nothing driving the upper.
            SERIAL_FIRST..=SERIAL_LAST => csr::FLOATING | self.serial.read(r, ns) as u16,
            // A read of the beep clicks it: the decoder that makes
            // `-CLICK.AUDIO` is not gated by `-WRITE`.  It, the GPIO and the
            // two unnamed slots of the keyboard group answer with nothing
            // behind them, and nothing driving the data lines reads as ones.
            BEEP => {
                self.click_audio(ns);
                0o177777
            }
            GPIO | 0o764114 | 0o764116 => 0o177777,
            _ => 0,
        }
    }

    pub fn write(&mut self, uaddr: u32, v: u16, ns: u64) {
        let Some(r) = answers(uaddr, true) else { return };
        if chaos_register(r) {
            if let Some(c) = self.chaos.as_mut() {
                c.write(r, v, ns);
            }
            return;
        }
        match r {
            CSR => self.csr = (self.csr & !csr::WRITABLE) | (v & csr::WRITABLE),
            CLOCK => {
                self.interval = v;
                self.interval_loaded_at = Some(ns);
            }
            // The 2651 takes `D0`..`D7`, which are `UBI0`..`UBI7` through
            // the 74LS244 at IOBSER 0E29.
            SERIAL_FIRST..=SERIAL_LAST => self.serial.write(r, v as u8, ns),
            BEEP => self.click_audio(ns),
            // The keyboard and mouse registers are inputs.
            _ => {}
        }
    }
}

// --- Checkpoints ------------------------------------------------------------

impl IoBoard {
    /// The board into a checkpoint: its registers, the serial port's, and
    /// the Chaosnet interface if one is plugged in.
    pub fn save(&self, w: &mut crate::checkpoint::Writer) {
        let IoBoard {
            csr,
            scancode,
            usec,
            interval,
            interval_loaded_at,
            mouse_x,
            mouse_y,
            mouse_buttons,
            audio,
            audio_click_ns,
            beep_started,
            chaos,
            serial,
        } = self;
        w.u16(*csr);
        w.u32(*scancode);
        w.u32(*usec);
        w.u16(*interval);
        w.bool(interval_loaded_at.is_some());
        w.u64(interval_loaded_at.unwrap_or(0));
        w.u16(*mouse_x);
        w.u16(*mouse_y);
        w.u8(*mouse_buttons);
        w.bool(*audio);
        w.opt(*audio_click_ns, crate::checkpoint::Writer::u64);
        w.bool(*beep_started);
        serial.save(w);
        w.bool(chaos.is_some());
        if let Some(c) = chaos {
            c.save(w);
        }
    }

    /// Back from a checkpoint, into a board with the same interface plugged
    /// in, or none, as the checkpoint had: the cable is not in the file.
    pub fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        use crate::checkpoint::bad;
        self.csr = r.u16()?;
        self.scancode = r.u32()?;
        self.usec = r.u32()?;
        self.interval = r.u16()?;
        self.interval_loaded_at = {
            let loaded = r.bool()?;
            let at = r.u64()?;
            loaded.then_some(at)
        };
        self.mouse_x = r.u16()?;
        self.mouse_y = r.u16()?;
        self.mouse_buttons = r.u8()?;
        self.audio = r.bool()?;
        self.audio_click_ns = r.opt(crate::checkpoint::Reader::u64)?;
        self.beep_started = r.bool()?;
        self.serial.load(r)?;
        match (r.bool()?, self.chaos.as_mut()) {
            (true, Some(c)) => c.load(r),
            (false, None) => Ok(()),
            (true, None) => {
                Err(bad("a Chaosnet interface on the I/O board, and this machine has none"))
            }
            (false, Some(_)) => Err(bad("no Chaosnet interface, and this machine has one")),
        }
    }
}
