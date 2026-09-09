// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The serial port: the Signetics 2651 PCI at IOBSER 0A12, its baud-rate
//! crystal at 0A15, and the RS-232 cable on J9 through the MC1488 at 0B17
//! and the MC1489 at 0B16.
//!
//! Four things are here. The facts read off the Signetics sheet
//! (`2651.pdf`, the 1978 preliminary specification) that every engine
//! shares: the register bits, the baud-rate table and the character frame.
//! [`Pci`], the chip as the behavioural engines have it, a character at a
//! time on the machine's clock, which `src/ioboard.rs` answers the bus
//! with. [`OnCable`], the far end of the cable for the netlist board,
//! a bit at a time on the EIA wires, which `tests/serial_cable.rs` holds
//! the chip's own model in `src/part.rs` to. And [`Endpoint`], the route
//! out of the process: the TCP endpoint `muir --serial` opens, which plugs
//! whatever connects to it into either far end.
//!
//! **What the board wires.** Page IOBSER of `data/CADRIO.netlist`: the
//! data bus `D0`..`D7` on `UBO0`..`UBO7`, which the 74LS244 at 0E29 copies
//! `UBI0`..`UBI7` onto whenever `READ` is low, so a write's data reaches
//! the chip and a read's leaves it on the lines the board drives the bus
//! from. `-CE` is `-SELECT.764160`, `A1` and `A0` are `UBADDR2` and
//! `UBADDR1`, and `R/-W` is `-READ`, which the sheet reads as "Read
//! command when low, write command when high". `BRCLK` is the 5.0688 MHz
//! can at 0A15 and `RESET` is the board's `RESET` off `-INIT*`. `-TxC` and
//! `-RxC` are not connected, so the internal baud-rate generator is the
//! only clock the chip can have. The RS-232 side: `TxD`, `-DTR` and `-RTS`
//! out through the MC1488, `RxD`, `-DSR`, `-CTS` and `-DCD` in through the
//! MC1489, to J9 pins 1 to 7 by `cadrio/serial.eco`. `-RxRDY` is `-SER
//! RRDY`, into the 74LS02 at 0E11 with `-SER INT ENABLE` for `SER.IREQ`,
//! and ECO 10 of `cadrio/iob.eco` puts `-TxRDY` on the same net, which is
//! why System 100's `sys/io1/serial.lisp` can run an output channel and an
//! input channel on one vector. `-TxEMT/DSCHG` goes nowhere.
//!
//! **The addresses are MIT's own.** `sys/doc/unaddr.text`: "764160 read
//! received data, write transmit data; 764162 read data set status, write
//! send weird characters in synchronous mode; 764164 mode selection;
//! 764166 command", which is the sheet's Table 4 on `A1`, `A0` as the
//! board wires them. `A3` is not decoded, so `764170` to `764176` are the
//! same four again.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};

use crate::chip::Chip;
use crate::netlist::{NetId, Netlist};
use crate::part::Level;

/// Read, the receive holding register; written, the transmit holding
/// register. Table 4: `A1 A0` = `0 0`.
pub const DATA: u32 = 0o764160;
/// Read, the status register; written, the SYN1, SYN2 and DLE registers
/// in turn. Table 4: `0 1`.
pub const STATUS: u32 = 0o764162;
/// Mode register 1, then mode register 2, read or written. Table 4: `1 0`.
pub const MODE: u32 = 0o764164;
/// The command register. Table 4: `1 1`. A read of it also puts the mode
/// and SYN pointers back to their first registers.
pub const COMMAND: u32 = 0o764166;

/// The crystal at IOBSER 0A15, as the drawing names its output net,
/// `5.0688 MHz`: the frequency the sheet's Table 1 is for, "Crystal
/// Frequency = 5.0688MHz". `src/chip.rs` runs the can at this period.
pub const BRCLK_HZ: u64 = 5_068_800;

/// Table 1, "BAUD RATE GENERATOR CHARACTERISTICS": the divisor from the
/// crystal to the 16X clock for each of the sixteen rates mode register 2
/// selects, `MR23`..`MR20` in Table 6's order --- 50, 75, 110, 134.5, 150,
/// 300, 600, 1200, 1800, 2000, 2400, 3600, 4800, 7200, 9600, 19200 baud.
/// The sheet's note under the table: "16X clock is used in asynchronous
/// mode", and under Table 5, "Factor is 16X if internal clock is selected",
/// which on this board it always is.
pub const DIVISORS: [u32; 16] =
    [6336, 4224, 2880, 2355, 2112, 1056, 528, 264, 176, 158, 132, 88, 66, 44, 33, 16];

/// The rates of [`DIVISORS`], in tenths of a baud so that 134.5 is a
/// number: Table 6, and the same list in `serial.lisp`'s `:BAUD`.
pub const BAUD_TENTHS: [u32; 16] = [
    500, 750, 1100, 1345, 1500, 3000, 6000, 12000, 18000, 20000, 24000, 36000, 48000, 72000, 96000,
    192000,
];

/// One bit time at rate `rate` (an index into [`DIVISORS`]), in
/// nanoseconds: sixteen 16X clocks, each `divisor` crystal periods.
pub fn bit_ns(rate: u8) -> u64 {
    16 * DIVISORS[rate as usize & 0xf] as u64 * 1_000_000_000 / BRCLK_HZ
}

/// Mode register 1, Table 5.
pub mod mode1 {
    /// `MR11 MR10`: `00` synchronous 1X, `01` asynchronous 1X, `10`
    /// asynchronous 16X, `11` asynchronous 64X. The factor only applies
    /// with an external clock; the mode does.
    pub const RATE_MASK: u8 = 0o3;
    pub const SYNCHRONOUS: u8 = 0;
    /// `MR13 MR12`: 5, 6, 7 or 8 bits.
    pub const LENGTH_SHIFT: u8 = 2;
    pub const LENGTH_MASK: u8 = 0o14;
    /// `MR14`: parity generated and checked.
    pub const PARITY: u8 = 0o20;
    /// `MR15`: even parity when set, odd when clear.
    pub const EVEN: u8 = 0o40;
    /// `MR17 MR16`, asynchronous: `00` invalid, `01` one stop bit, `10`
    /// one and a half, `11` two.
    pub const STOP_SHIFT: u8 = 6;
    pub const STOP_MASK: u8 = 0o300;
}

/// Mode register 2, Table 6.
pub mod mode2 {
    /// `MR23`..`MR20`: the rate, an index into [`super::DIVISORS`].
    pub const RATE_MASK: u8 = 0o17;
    /// `MR24`: the receiver runs on the internal generator, not `-RxC`.
    pub const RX_INTERNAL: u8 = 0o20;
    /// `MR25`: the transmitter runs on the internal generator, not `-TxC`.
    pub const TX_INTERNAL: u8 = 0o40;
}

/// The command register, Table 7.
pub mod command {
    /// `CR0`, TxEN.
    pub const TX_ENABLE: u8 = 0o1;
    /// `CR1`: "1 = FORCE -DTR OUTPUT LOW".
    pub const DTR: u8 = 0o2;
    /// `CR2`, RxEN.
    pub const RX_ENABLE: u8 = 0o4;
    /// `CR3`, asynchronous: force break, `TxD` held low.
    pub const BREAK: u8 = 0o10;
    /// `CR4`: clears `SR3`, `SR4` and `SR5`; "This bit resets
    /// automatically".
    pub const RESET_ERROR: u8 = 0o20;
    /// `CR5`: "1 = FORCE -RTS OUTPUT LOW".
    pub const RTS: u8 = 0o40;
    /// `CR7 CR6`, the operating mode.
    pub const MODE_MASK: u8 = 0o300;
    pub const NORMAL: u8 = 0o000;
    /// Asynchronous: received data is retransmitted, and the CPU's
    /// transmit path is cut.
    pub const AUTO_ECHO: u8 = 0o100;
    /// `TxD` to `RxD` inside the chip, `-DTR` to `-DCD` and `-RTS` to
    /// `-CTS`; the pins hold still.
    pub const LOCAL_LOOP_BACK: u8 = 0o200;
    /// Received data retransmitted and kept from the CPU.
    pub const REMOTE_LOOP_BACK: u8 = 0o300;
}

/// The status register, Table 8.
pub mod status {
    /// `SR0`: the transmit holding register is empty. Valid only with the
    /// transmitter enabled, and never set in auto echo or remote loop
    /// back.
    pub const TX_READY: u8 = 0o1;
    /// `SR1`: the receive holding register has a character.
    pub const RX_READY: u8 = 0o2;
    /// `SR2`: the transmitter has run out of characters, or `-DSR` or
    /// `-DCD` has changed. The change half clears on a read of this
    /// register.
    pub const TX_EMPTY_OR_DSCHG: u8 = 0o4;
    /// `SR3`, asynchronous: parity error.
    pub const PARITY_ERROR: u8 = 0o10;
    /// `SR4`: a character arrived before the last was read.
    pub const OVERRUN: u8 = 0o20;
    /// `SR5`, asynchronous: framing error, a stop bit that was low.
    pub const FRAMING_ERROR: u8 = 0o40;
    /// `SR6`: "1 = -DCD INPUT IS LOW".
    pub const DCD: u8 = 0o100;
    /// `SR7`: "1 = -DSR INPUT IS LOW".
    pub const DSR: u8 = 0o200;
    /// What `CR4` clears.
    pub const ERRORS: u8 = PARITY_ERROR | OVERRUN | FRAMING_ERROR;
}

/// The asynchronous character frame mode register 1 describes: a start
/// bit, five to eight data bits least significant first, a parity bit if
/// enabled, and one, one and a half or two stop bits (Table 5, and
/// "Transmitter" on the sheet's page 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Framing {
    /// Data bits, 5 to 8.
    pub bits: u8,
    /// A parity bit, and whether even.
    pub parity: Option<bool>,
    /// Stop bits, in halves: 2, 3 or 4.
    pub stop_halves: u8,
}

impl Framing {
    /// The frame mode register 1 selects.
    pub fn of(mr1: u8) -> Framing {
        Framing {
            bits: 5 + ((mr1 & mode1::LENGTH_MASK) >> mode1::LENGTH_SHIFT),
            parity: (mr1 & mode1::PARITY != 0).then_some(mr1 & mode1::EVEN != 0),
            stop_halves: match (mr1 & mode1::STOP_MASK) >> mode1::STOP_SHIFT {
                0o2 => 3,
                0o3 => 4,
                // `01`, and `00`, which the sheet calls invalid and which
                // is what the register holds from reset.
                _ => 2,
            },
        }
    }

    /// The data bits a character is masked to: "If the character length
    /// is less than 8 bits, the high order unused bits in the Holding
    /// Register are set to zero."
    pub fn mask(&self) -> u8 {
        (0xffu16 >> (8 - self.bits)) as u8
    }

    /// The parity bit for `byte`, if the frame has one.
    pub fn parity_bit(&self, byte: u8) -> Option<bool> {
        self.parity.map(|even| ((byte & self.mask()).count_ones() % 2 == 1) != even)
    }

    /// The whole frame, in half bits.
    pub fn half_bits(&self) -> u32 {
        2 + 2 * self.bits as u32
            + if self.parity.is_some() { 2 } else { 0 }
            + self.stop_halves as u32
    }

    /// The whole frame in nanoseconds at rate `rate`, to the nanosecond:
    /// eight 16X clocks a half bit.
    pub fn frame_ns(&self, rate: u8) -> u64 {
        self.half_bits() as u64 * 8 * DIVISORS[rate as usize & 0xf] as u64 * 1_000_000_000
            / BRCLK_HZ
    }

    /// The frame for `byte` as levels on the line, a bit at a time, high
    /// for a mark: the start bit low, the data least significant first,
    /// the parity bit, and the stop bits as whole bits --- one and a half
    /// rounds up to two here, which is what a far end sampling whole bits
    /// sees.
    pub fn encode(&self, byte: u8) -> Vec<bool> {
        let mut bits = vec![false];
        bits.extend((0..self.bits).map(|k| byte >> k & 1 != 0));
        bits.extend(self.parity_bit(byte));
        bits.extend(std::iter::repeat_n(true, self.stop_halves.div_ceil(2) as usize));
        bits
    }
}

/// The RS-232 cable on J9, as the behavioural engines carry it: a
/// character at a time, each way, and the three inputs the far end holds.
///
/// Nothing is plugged in until [`Cable::plug`]: then the far end asserts
/// `DSR`, `DCD` and `CTS`, as the device at the other end of a null-modem
/// cable does with its own `DTR` and `RTS`, and the port can transmit and
/// receive. Unplugged,
/// the MC1489's open inputs give the chip `-DSR`, `-DCD` and `-CTS` high
/// --- the sheet's `V_OH` row for "Input open" (`mc1489.pdf`) --- and the
/// sheet keeps the transmitter and the receiver from running: "The 2651 is
/// conditioned to transmit data when the -CTS input is low", "conditioned
/// to receive data when the -DCD input is low".
///
/// Characters the far end sends carry the time it sent them, and wait
/// here until the receiver takes them, where a line would lose what was
/// sent while the receiver was off: this far end waits for its prompt.
#[derive(Clone, Debug, Default)]
pub struct Cable {
    /// When something was plugged in, if it is.
    plugged: Option<u64>,
    /// From the far end, with when its start bit began, not yet taken by
    /// the receiver.
    inbound: VecDeque<(u64, u8)>,
    /// From the port, each with the time its last stop bit ended.
    outbound: VecDeque<(u64, u8)>,
}

impl Cable {
    /// Something is on the far end from `at`, ready: `DSR`, `DCD` and
    /// `CTS` up.
    pub fn plug(&mut self, at: u64) {
        self.plugged = Some(at);
    }

    pub fn unplug(&mut self) {
        self.plugged = None;
    }

    pub fn plugged(&self) -> bool {
        self.plugged.is_some()
    }

    /// The far end sends `byte`, its start bit beginning at `at` on the
    /// machine's clock, or when the far end's last one ends if later.
    pub fn send(&mut self, byte: u8, at: u64) {
        self.inbound.push_back((at, byte));
    }

    /// The next character the port has sent, with the time its stop bit
    /// ended, if any.
    pub fn take(&mut self) -> Option<(u64, u8)> {
        self.outbound.pop_front()
    }

    /// Everything the port has sent and the far end not yet taken.
    pub fn sent(&self) -> impl Iterator<Item = u8> + '_ {
        self.outbound.iter().map(|&(_, b)| b)
    }

    /// Characters the far end has sent and the port not yet taken.
    pub fn pending(&self) -> usize {
        self.inbound.len()
    }
}

/// Which register a Unibus address in the group reaches: `A1 A0` from
/// `UBADDR2` and `UBADDR1`.
fn select(uaddr: u32) -> u8 {
    (uaddr >> 1 & 3) as u8
}

/// The `j`-th rising edge of the crystal from power-on, as the chip
/// engine makes it: its `2j`-th toggle.
fn crystal_rise(j: u64) -> u64 {
    crate::chip::toggle_at(crate::chip::IOB_OSCILLATOR_PERIOD, 2 * j)
}

/// The first rising edge of the crystal at or after `t`.
fn crystal_rise_at_or_after(t: u64) -> u64 {
    let (num, den) = crate::chip::IOB_OSCILLATOR_PERIOD;
    let mut j = (t * den).div_ceil(num);
    while crystal_rise(j) < t {
        j += 1;
    }
    while j > 0 && crystal_rise(j - 1) >= t {
        j -= 1;
    }
    j
}

/// The 2651 as the behavioural engines have it: its registers, and its
/// transmitter and receiver a character at a time on the machine's clock.
///
/// Time enters through the `now` each access carries, and the chip's own
/// grain is kept: the baud-rate generator's 16X clock, one every
/// `divisor` crystal periods from the moment the generator comes on ---
/// the write that enables the transmitter or the receiver --- which is
/// where `src/part.rs` starts it too, so the two engines move a character
/// on the same clock. A character loaded into the transmit holding
/// register goes into the shift register at the next 16X clock if that is
/// idle and `-CTS` is down, and is on the cable, complete, one frame
/// later; a second waits in the holding register for it, which is the
/// sheet's "one full character time of buffering". The receiver takes the
/// far end's characters in order from when each was sent, or from when it
/// could first take one, and has each in the holding register at the
/// middle of its stop bit, as the chip does.
///
/// Held in reset by [`Pci::reset`] as the board's `RESET` holds it: the
/// mode, command and status registers clear, the pointers at their first
/// registers, and "the device assumes the idle state and remains there
/// until initialized with the appropriate control words" (Table 2).
///
/// **The holding registers are emptied here, and the sheet can be read
/// against that.** Four independent routes say the same thing and none
/// mentions them: the 1985 SCN2651 product specification, the same text
/// word for word in Philips' 1991 *Data Communication Products* (IC019),
/// application note TN072 "Introducing the Signetics 2651 PCI", which is a
/// block diagram of the part and no more, and the 1977 *Bipolar & MOS
/// Microprocessor* book. Each enumerates three registers and stops:
/// `RESET` "clears the Mode, Command and Status registers. The device
/// assumes the idle state and remains there until initialized with the
/// appropriate control words".
///
/// **Four sources agreeing on a list of three is not silence.** The natural
/// reading of an enumeration is that what is outside it is not cleared, so
/// a character already in the transmit holding register would survive a
/// reset and go out when the transmitter was next enabled. Against that,
/// "the device assumes the idle state" is the whole-device claim, and a
/// part with a character still queued is not obviously idle. The sheet
/// supports both readings and settles neither, and no second source
/// disambiguates it because all four carry the same paragraph.
///
/// Emptying them is the reading that makes "assumes the idle state" true,
/// and it is the choice here. It is a choice: at power-on there is no
/// previous content to keep, so the question only has force for a reset
/// during operation, and the release drives nothing through this port.
///
/// `tests/serial.rs` holds the consequence rather than the value: reset
/// leaves `RxRDY` clear, so a program following the protocol never reads a
/// register the chip has not filled. What would settle the transmit side is
/// a real part --- load the holding register, reset, enable the
/// transmitter, and see whether the character goes out.
///
/// Not modelled, and what each would need: synchronous mode (`MR1` rate
/// `00`), in which this transmits and receives nothing; the break the
/// transmitter can force and the framing and parity errors the receiver
/// can raise, which need a far end that sends bits rather than characters
/// --- the netlist board has one; and the external clocks, which the
/// board does not connect.
#[derive(Clone, Debug, Default)]
pub struct Pci {
    mode1: u8,
    mode2: u8,
    command: u8,
    /// `SR3`, `SR4`, `SR5` as latched.
    errors: u8,
    /// The mode pointer: the next access to `MODE` is mode register 2.
    second_mode: bool,
    /// The SYN pointer: the next write to `STATUS` is SYN1, SYN2 or DLE.
    next_syn: u8,
    syn: [u8; 3],
    /// When the baud-rate generator came on, while it is.
    generator_from: Option<u64>,
    /// The transmit holding register, when it holds a character, and when
    /// it was loaded.
    thr: Option<(u8, u64)>,
    /// The character in the transmit shift register and when its last
    /// stop bit ends.
    shifting: Option<(u8, u64)>,
    /// The earliest a frame can start, beyond the holding register's
    /// load: when the transmitter came on, or the far end came up.
    tx_from: u64,
    /// `TxEMT`: the shift register emptied with nothing to follow.
    tx_empty: bool,
    /// The receive holding register, and `RxRDY`.
    rhr: u8,
    rx_ready: bool,
    /// The character the receiver is assembling, when it is in the
    /// holding register, and when its frame ends.
    assembling: Option<(u8, u64, u64)>,
    /// The earliest the next character from the far end can begin: when
    /// the receiver last came up, or the last character ended.
    rx_from: u64,
    /// `DSCHG`: `-DSR` or `-DCD` moved since the status register was read.
    dschg: bool,
    /// The two as last seen, for that.
    dsr_was: bool,
    dcd_was: bool,
    pub cable: Cable,
}

impl Pci {
    /// The chip's `RESET`: the board's `RESET` net, `-INIT*` through the
    /// 8837 at IOBXCV 0F06, on pin 21.
    pub fn reset(&mut self) {
        let cable = std::mem::take(&mut self.cable);
        *self = Pci { cable, ..Pci::default() };
        self.dsr_was = self.dsr();
        self.dcd_was = self.dcd();
    }

    pub fn mode1(&self) -> u8 {
        self.mode1
    }

    pub fn mode2(&self) -> u8 {
        self.mode2
    }

    pub fn command(&self) -> u8 {
        self.command
    }

    /// The frame the mode registers select.
    pub fn framing(&self) -> Framing {
        Framing::of(self.mode1)
    }

    /// The rate mode register 2 selects, an index into [`DIVISORS`].
    pub fn rate(&self) -> u8 {
        self.mode2 & mode2::RATE_MASK
    }

    fn mode(&self) -> u8 {
        self.command & command::MODE_MASK
    }

    /// `-DSR` as the far end holds it, asserted or not.
    pub fn dsr(&self) -> bool {
        self.cable.plugged()
    }

    /// `-DCD`: the far end's, or `-DTR`'s in local loop back.
    pub fn dcd(&self) -> bool {
        if self.mode() == command::LOCAL_LOOP_BACK {
            self.command & command::DTR != 0
        } else {
            self.cable.plugged()
        }
    }

    /// `-CTS`: the far end's, or `-RTS`'s in local loop back.
    pub fn cts(&self) -> bool {
        if self.mode() == command::LOCAL_LOOP_BACK {
            self.command & command::RTS != 0
        } else {
            self.cable.plugged()
        }
    }

    /// `-DTR`, asserted or not: "the complement of Command Register bit
    /// CR1", held off in local loop back.
    pub fn dtr(&self) -> bool {
        self.command & command::DTR != 0 && self.mode() != command::LOCAL_LOOP_BACK
    }

    /// `-RTS`, asserted or not: the complement of `CR5`, held off in
    /// local loop back.
    pub fn rts(&self) -> bool {
        self.command & command::RTS != 0 && self.mode() != command::LOCAL_LOOP_BACK
    }

    /// Whether the mode registers put the port in asynchronous mode on the
    /// internal clock for the transmitter, the only way it runs here.
    fn tx_clocked(&self) -> bool {
        self.mode1 & mode1::RATE_MASK != mode1::SYNCHRONOUS && self.mode2 & mode2::TX_INTERNAL != 0
    }

    fn rx_clocked(&self) -> bool {
        self.mode1 & mode1::RATE_MASK != mode1::SYNCHRONOUS && self.mode2 & mode2::RX_INTERNAL != 0
    }

    /// Whether the transmitter is on: enabled, clocked, and in a mode that
    /// takes the CPU's characters --- auto echo and remote loop back cut
    /// "the CPU to transmitter link".
    fn tx_on(&self) -> bool {
        self.command & command::TX_ENABLE != 0
            && self.tx_clocked()
            && matches!(self.mode(), command::NORMAL | command::LOCAL_LOOP_BACK)
    }

    /// Whether the receiver is on: enabled, or in local loop back, where
    /// "CR2 (RxEN) is ignored", and clocked.
    fn rx_on(&self) -> bool {
        (self.command & command::RX_ENABLE != 0 || self.mode() == command::LOCAL_LOOP_BACK)
            && self.rx_clocked()
    }

    /// Whether the receiver runs: on, with `-DCD` down.
    fn rx_runs(&self) -> bool {
        self.rx_on() && self.dcd()
    }

    /// Whether the baud-rate generator counts: it is held while both
    /// halves are off, as `src/part.rs` holds it.
    fn generator_on(&self) -> bool {
        self.tx_on() || self.rx_on()
    }

    /// The first 16X clock at or after `t`: the generator counts the
    /// crystal's rising edges from when it came on, and every `divisor`
    /// of them is a clock. The edges are where `src/chip.rs` puts the
    /// can's, [`crate::chip::IOB_OSCILLATOR_PERIOD`] from power-on, so
    /// that the two engines' clocks fall on the same nanosecond; which
    /// edge the generator's own phase would put them on is not observable
    /// from the board. With the generator off, as if it started at `t`.
    pub fn clock_at_or_after(&self, t: u64) -> u64 {
        let divisor = DIVISORS[self.rate() as usize] as u64;
        let from = self.generator_from.unwrap_or(t);
        let j0 = crystal_rise_at_or_after(from);
        let jt = crystal_rise_at_or_after(t);
        // Clock `k` is on rise `j0 + k * divisor - 1`, the count reaching
        // `divisor`.
        let k = (jt + 1 - j0).div_ceil(divisor).max(1);
        crystal_rise(j0 + k * divisor - 1)
    }

    /// After a register write: the generator starts when it comes on,
    /// and stops when it goes off; the transmitter and receiver note when
    /// they came on.
    fn note_enables(&mut self, was: (bool, bool, bool), now: u64) {
        let (generator, tx, rx) = was;
        if !generator && self.generator_on() {
            self.generator_from = Some(now);
        } else if generator && !self.generator_on() {
            self.generator_from = None;
        }
        if !tx && self.tx_on() {
            self.tx_from = self.tx_from.max(now);
        }
        if !rx && self.rx_on() {
            self.rx_from = self.rx_from.max(now);
        }
    }

    fn enables(&self) -> (bool, bool, bool) {
        (self.generator_on(), self.tx_on(), self.rx_on())
    }

    /// When the character in the holding register starts, the shift
    /// register being free: the first 16X clock after it was loaded and
    /// the transmitter could take it.
    fn thr_start(&self) -> Option<u64> {
        let (_, loaded) = self.thr?;
        (self.tx_on() && self.cts()).then(|| self.clock_at_or_after(loaded.max(self.tx_from)))
    }

    /// `SR0` as it will stand at `now`, without moving anything: the
    /// holding register empty, or emptied into the shift register by then.
    pub fn tx_ready_at(&self, now: u64) -> bool {
        if !self.tx_on() {
            return false;
        }
        match (self.thr, self.shifting) {
            (None, _) => true,
            (Some(_), None) => self.thr_start().is_some_and(|start| start <= now),
            (Some(_), Some((_, done))) => self.cts() && done <= now,
        }
    }

    /// When the character being assembled, or else the first one waiting
    /// on the cable, is in the holding register.
    fn next_arrival(&self) -> Option<u64> {
        self.assembling.map(|(_, done, _)| done).or_else(|| {
            let (done, _) = self.rx_times(self.cable.inbound.front()?.0.max(self.rx_from));
            Some(done)
        })
    }

    /// For a character whose start bit begins at `start`: when it is in
    /// the holding register, the middle of the stop bit, and when its
    /// frame ends.
    fn rx_times(&self, start: u64) -> (u64, u64) {
        let f = self.framing();
        let bit = bit_ns(self.rate());
        let done = start + (1 + f.bits as u64 + f.parity.is_some() as u64) * bit + bit / 2;
        (done, start + f.frame_ns(self.rate()))
    }

    /// `SR1` as it will stand at `now`: a character in the holding
    /// register, or one that will have arrived by then.
    pub fn rx_ready_at(&self, now: u64) -> bool {
        self.rx_ready || (self.rx_runs() && self.next_arrival().is_some_and(|done| done <= now))
    }

    /// Time passes to `now`: characters finish leaving and arriving.
    ///
    /// A port with neither half in flight is left alone. With nothing in
    /// the shift or holding registers, nothing being assembled, nothing
    /// waiting on the cable and the modem lines where they were, there is
    /// no character for the transmitter or the receiver to move and
    /// `transmit` and `receive` can only reach their first
    /// `break`. Saying so here rather than there is what matters: the
    /// engines call this every microcycle, and working the frame's length
    /// out --- [`Framing::frame_ns`], a multiply and a divide --- to
    /// discover there is nothing to send was the largest single cost of a
    /// port with nothing plugged into it. That the skipping changes
    /// nothing is held by `tests/idle_polling.rs`, which runs a port at
    /// two grains and compares, and by `tests/serial_cable.rs`, which
    /// holds the model to the netlist board over the modem lines moving.
    pub fn advance(&mut self, now: u64) {
        if self.shifting.is_none()
            && self.thr.is_none()
            && self.assembling.is_none()
            && self.cable.inbound.is_empty()
            && self.dsr() == self.dsr_was
            && self.dcd() == self.dcd_was
        {
            return;
        }
        let (dsr, dcd) = (self.dsr(), self.dcd());
        if dsr != self.dsr_was || dcd != self.dcd_was {
            self.dschg = true;
            self.dsr_was = dsr;
            self.dcd_was = dcd;
            if let Some(at) = self.cable.plugged {
                self.rx_from = self.rx_from.max(at);
                self.tx_from = self.tx_from.max(at);
            }
        }
        self.transmit(now);
        self.receive(now);
    }

    /// The transmitter up to `now`: whatever the shift register finished
    /// by then is on the cable, and the holding register follows it in.
    fn transmit(&mut self, now: u64) {
        let frame = self.framing().frame_ns(self.rate());
        loop {
            match self.shifting {
                Some((byte, done)) if done <= now => {
                    self.shifting = None;
                    self.deliver(byte, done);
                    match self.thr {
                        Some((next, _)) if self.tx_on() && self.cts() => {
                            self.thr = None;
                            self.shifting = Some((next, done + frame));
                        }
                        Some(_) => {}
                        None => self.tx_empty = true,
                    }
                }
                Some(_) => break,
                None => match (self.thr, self.thr_start()) {
                    (Some((next, _)), Some(start)) if start <= now => {
                        self.thr = None;
                        self.shifting = Some((next, start + frame));
                    }
                    _ => break,
                },
            }
        }
    }

    /// A character the shift register has finished, at `at`: onto the
    /// cable, or in local loop back straight into the receiver, which was
    /// assembling it all along.
    fn deliver(&mut self, byte: u8, at: u64) {
        let byte = byte & self.framing().mask();
        if self.mode() == command::LOCAL_LOOP_BACK {
            self.load_rhr(byte);
        } else {
            self.cable.outbound.push_back((at, byte));
        }
    }

    /// A character into the receive holding register: an overrun if the
    /// last is still there.
    fn load_rhr(&mut self, byte: u8) {
        if self.rx_ready {
            self.errors |= status::OVERRUN;
        }
        self.rhr = byte;
        self.rx_ready = true;
    }

    /// The receiver up to `now`: the far end's characters taken one frame
    /// each, each from when it was sent or when the receiver could first
    /// take it, whichever is later.
    fn receive(&mut self, now: u64) {
        let mask = self.framing().mask();
        loop {
            if !self.rx_runs() {
                break;
            }
            match self.assembling {
                Some((byte, done, ends)) if done <= now => {
                    self.assembling = None;
                    self.rx_from = ends;
                    match self.mode() {
                        command::REMOTE_LOOP_BACK => self.cable.outbound.push_back((ends, byte)),
                        command::AUTO_ECHO => {
                            self.cable.outbound.push_back((ends, byte));
                            self.load_rhr(byte);
                        }
                        _ => self.load_rhr(byte),
                    }
                }
                Some(_) => break,
                None => match self.cable.inbound.front() {
                    Some(&(sent, next)) if sent.max(self.rx_from) <= now => {
                        let (done, ends) = self.rx_times(sent.max(self.rx_from));
                        self.cable.inbound.pop_front();
                        self.assembling = Some((next & mask, done, ends));
                    }
                    _ => break,
                },
            }
        }
    }

    /// The status register as it stands.
    pub fn status(&self) -> u8 {
        let flag = |on: bool, mask: u8| if on { mask } else { 0 };
        let tx_ready = self.tx_on() && self.thr.is_none();
        let tx_empty = self.command & command::TX_ENABLE != 0 && self.tx_empty;
        flag(tx_ready, status::TX_READY)
            | flag(self.rx_ready, status::RX_READY)
            | flag(tx_empty || self.dschg, status::TX_EMPTY_OR_DSCHG)
            | self.errors
            | flag(self.dcd(), status::DCD)
            | flag(self.dsr(), status::DSR)
    }

    /// A read of the register at `uaddr`, the low eight bits: the
    /// holding register, the status, mode register 1 or 2 by the pointer,
    /// or the command register.
    pub fn read(&mut self, uaddr: u32, now: u64) -> u8 {
        self.advance(now);
        match select(uaddr) {
            0 => {
                self.rx_ready = false;
                self.rhr
            }
            1 => {
                let s = self.status();
                self.dschg = false;
                s
            }
            2 => {
                let second = self.second_mode;
                self.second_mode = !second;
                if second { self.mode2 } else { self.mode1 }
            }
            _ => {
                // "The pointers are reset ... by performing a 'Read
                // Command Register' operation".
                self.second_mode = false;
                self.next_syn = 0;
                self.command
            }
        }
    }

    /// A write of `byte` to the register at `uaddr`.
    pub fn write(&mut self, uaddr: u32, byte: u8, now: u64) {
        self.advance(now);
        let was = self.enables();
        match select(uaddr) {
            0 => {
                self.thr = Some((byte, now));
                self.tx_empty = false;
            }
            1 => {
                self.syn[self.next_syn as usize] = byte;
                self.next_syn = (self.next_syn + 1) % 3;
            }
            2 => {
                if self.second_mode {
                    self.mode2 = byte;
                } else {
                    self.mode1 = byte;
                }
                self.second_mode = !self.second_mode;
            }
            _ => {
                if byte & command::RESET_ERROR != 0 {
                    self.errors &= !status::ERRORS;
                }
                self.command = byte & !command::RESET_ERROR;
                if !self.rx_on() {
                    // "the receiver ... will terminate operation
                    // immediately. Any character being assembled will be
                    // neglected", and `RxRDY` clears "when the receiver is
                    // disabled by CR2".
                    self.assembling = None;
                    self.rx_ready = false;
                }
                if self.command & command::TX_ENABLE == 0 {
                    self.tx_empty = false;
                }
            }
        }
        self.note_enables(was, now);
        self.transmit(now);
        self.receive(now);
    }

    /// Into a checkpoint: the registers and what is in flight, not the
    /// cable.
    pub fn save(&self, w: &mut crate::checkpoint::Writer) {
        let Pci {
            mode1,
            mode2,
            command,
            errors,
            second_mode,
            next_syn,
            syn,
            generator_from,
            thr,
            shifting,
            tx_from,
            tx_empty,
            rhr,
            rx_ready,
            assembling,
            rx_from,
            dschg,
            dsr_was,
            dcd_was,
            cable: _,
        } = self;
        w.bytes(&[*mode1, *mode2, *command, *errors, *next_syn, *rhr]);
        w.bytes(syn);
        for b in [second_mode, tx_empty, rx_ready, dschg, dsr_was, dcd_was] {
            w.bool(*b);
        }
        w.u64s(&[*tx_from, *rx_from]);
        w.bool(generator_from.is_some());
        w.u64(generator_from.unwrap_or(0));
        w.bool(thr.is_some());
        w.u8(thr.map_or(0, |(b, _)| b));
        w.u64(thr.map_or(0, |(_, t)| t));
        w.bool(shifting.is_some());
        w.u8(shifting.map_or(0, |(b, _)| b));
        w.u64(shifting.map_or(0, |(_, t)| t));
        w.bool(assembling.is_some());
        w.u8(assembling.map_or(0, |(b, _, _)| b));
        w.u64s(&assembling.map_or([0, 0], |(_, d, e)| [d, e]));
    }

    pub fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        use crate::checkpoint::bad;
        let mut regs = [0u8; 6];
        r.bytes_into(&mut regs)?;
        [self.mode1, self.mode2, self.command, self.errors, self.next_syn, self.rhr] = regs;
        r.bytes_into(&mut self.syn)?;
        self.second_mode = r.bool()?;
        self.tx_empty = r.bool()?;
        self.rx_ready = r.bool()?;
        self.dschg = r.bool()?;
        self.dsr_was = r.bool()?;
        self.dcd_was = r.bool()?;
        let froms = r.u64s()?;
        let [tx_from, rx_from] = froms[..] else { return Err(bad("the serial port's times")) };
        self.tx_from = tx_from;
        self.rx_from = rx_from;
        let some = r.bool()?;
        let at = r.u64()?;
        self.generator_from = some.then_some(at);
        let some = r.bool()?;
        let byte = r.u8()?;
        let at = r.u64()?;
        self.thr = some.then_some((byte, at));
        let some = r.bool()?;
        let byte = r.u8()?;
        let at = r.u64()?;
        self.shifting = some.then_some((byte, at));
        let some = r.bool()?;
        let byte = r.u8()?;
        let times = r.u64s()?;
        let [done, ends] = times[..] else { return Err(bad("the serial port's receiver")) };
        self.assembling = some.then_some((byte, done, ends));
        Ok(())
    }
}

// --- The far end on the netlist board ---------------------------------------

/// The seven EIA wires at J9, by net on the I/O board: what the far end
/// drives and what it reads. Their sense is the MC1488's and MC1489's as
/// `src/part.rs` has them, inverting: a high on an EIA net is the
/// positive voltage, which is a space on the data wires and an asserted
/// control; a low is negative, a mark, and off. So an idle data wire is
/// low, and a far end that is up holds the three control inputs high.
#[derive(Clone, Copy, Debug)]
pub struct Nets {
    data_in: NetId,
    dsr_in: NetId,
    cts_in: NetId,
    dcd_in: NetId,
    data_out: NetId,
    rts_out: NetId,
    dtr_out: NetId,
}

impl Nets {
    /// The nets on `n`, the I/O board.
    pub fn of(n: &Netlist) -> Option<Nets> {
        Some(Nets {
            data_in: n.by_name_id("'EIA DATA IN'")?,
            dsr_in: n.by_name_id("'EIA DSR IN'")?,
            cts_in: n.by_name_id("'EIA CTS IN'")?,
            dcd_in: n.by_name_id("'EIA DCD IN'")?,
            data_out: n.by_name_id("'EIA DATA OUT'")?,
            rts_out: n.by_name_id("'EIA RTS OUT'")?,
            dtr_out: n.by_name_id("'EIA DTR OUT'")?,
        })
    }
}

/// The far end of the serial cable on the netlist board: the device at
/// the other end of a null-modem cable, sending and receiving frames at a
/// rate of its own that has to be the port's, as the two ends of any
/// serial line have to agree. It keeps its own time:
/// [`OnCable::next_change`] says when it next moves the line or samples
/// it, and [`OnCable::apply`] does so. Not a terminal: that word is the
/// console display's far end, [`crate::terminal`].
pub struct OnCable {
    nets: Nets,
    framing: Framing,
    bit_ns: u64,
    /// Characters waiting to go.
    queue: VecDeque<u8>,
    /// The frame going out: its bits, the next to put on the line, and
    /// when.
    sending: Option<(Vec<bool>, usize, u64)>,
    /// What the data wire is held at, high for a mark.
    line: Option<bool>,
    /// Whether the control inputs are held up.
    up: bool,
    /// The frame coming in: when the next bit is sampled, how many have
    /// been, and the bits so far, least significant first.
    receiving: Option<(u64, u8, u16)>,
    /// The data wire as last read, for the start bit's edge.
    out_was: Level,
    /// Characters received from the board.
    pub received: Vec<u8>,
    /// Frames with a bad stop bit.
    pub framing_errors: usize,
    pub sent: usize,
    /// Which instance on the board is the 2651, for [`OnCable::follow`],
    /// once it has been looked for: `Some(None)` is a board with none.
    pci: Option<Option<usize>>,
}

impl OnCable {
    /// The far end at rate `rate`, an index into [`DIVISORS`], with the
    /// frame `framing`, on the nets of `n`, the I/O board.
    pub fn of(n: &Netlist, rate: u8, framing: Framing) -> Option<OnCable> {
        Some(OnCable::new(Nets::of(n)?, rate, framing))
    }

    pub fn new(nets: Nets, rate: u8, framing: Framing) -> OnCable {
        OnCable {
            nets,
            framing,
            bit_ns: bit_ns(rate),
            queue: VecDeque::new(),
            sending: None,
            line: None,
            up: false,
            receiving: None,
            out_was: Level::X,
            received: Vec::new(),
            framing_errors: 0,
            sent: 0,
            pci: None,
        }
    }

    /// Queues a character for the board.
    pub fn send(&mut self, byte: u8) {
        self.queue.push_back(byte);
    }

    /// Whether a character is on the line or waiting.
    pub fn busy(&self) -> bool {
        self.sending.is_some() || !self.queue.is_empty()
    }

    /// How many characters are waiting to go, the one on the line
    /// included.
    pub fn queued(&self) -> usize {
        self.queue.len() + self.sending.is_some() as usize
    }

    /// The far end comes up: `DSR`, `DCD` and `CTS` asserted from the
    /// next [`OnCable::apply`].
    pub fn plug(&mut self) {
        self.up = true;
    }

    pub fn unplug(&mut self) {
        self.up = false;
    }

    /// Whether the far end is up.
    pub fn plugged(&self) -> bool {
        self.up
    }

    /// Takes the port's own rate and frame off the 2651 on `board`: mode
    /// register 2's rate and mode register 1's frame, whatever the machine
    /// programmed into them.
    ///
    /// The two ends of a serial line run at one rate, and on this cable it
    /// is the port's: nothing on the far end has a say in it. A far end
    /// that picked its own would sample every stop bit in the wrong place,
    /// which is garbage rather than an error, so a run that reaches this
    /// port from outside the process calls this at every step ---
    /// `src/unibus.rs` does --- and `tests/serial_cable.rs` holds a
    /// character through two rates in one run.
    ///
    /// Between frames only: the bit time belongs to the frame in flight,
    /// and moving it under one would stretch the bits still to come.
    pub fn follow(&mut self, board: &Chip) {
        if self.sending.is_some() || self.receiving.is_some() {
            return;
        }
        let found = self.pci.get_or_insert_with(|| {
            board.instances.iter().position(|p| crate::part::strip(&p.kind).0 == "2651")
        });
        let Some(i) = *found else { return };
        let Some((mr1, mr2)) = crate::part::pci_modes(&board.instances[i].state) else { return };
        self.bit_ns = bit_ns(mr2 & mode2::RATE_MASK);
        self.framing = Framing::of(mr1);
    }

    /// `-RTS` as the board drives it through the MC1488, asserted or not.
    pub fn rts(&self, board: &Chip) -> bool {
        board.net(self.nets.rts_out) == Level::High
    }

    /// `-DTR` as the board drives it, asserted or not.
    pub fn dtr(&self, board: &Chip) -> bool {
        board.net(self.nets.dtr_out) == Level::High
    }

    /// When the far end next moves the line or samples it, if it will.
    pub fn next_change(&self, now: u64) -> Option<u64> {
        let tx = match &self.sending {
            Some((_, _, at)) => Some(*at),
            None if !self.queue.is_empty() => Some(now),
            None => None,
        };
        let rx = self.receiving.map(|(at, _, _)| at);
        [tx, rx].into_iter().flatten().min()
    }

    /// Holds the data wire at `mark`: low on the EIA wire.
    fn drive_line(&mut self, board: &mut Chip, mark: bool) -> bool {
        if self.line == Some(mark) {
            return false;
        }
        board.drive(self.nets.data_in, if mark { Level::Low } else { Level::High });
        self.line = Some(mark);
        true
    }

    /// Moves the line if a bit is due, samples the board's data wire if a
    /// sample is due, and holds the control inputs. Returns whether a net
    /// moved, in which case the board was transitioned at `now`.
    pub fn apply(&mut self, board: &mut Chip, now: u64) -> bool {
        let mut moved = false;
        let control = if self.up { Level::High } else { Level::Low };
        for net in [self.nets.dsr_in, self.nets.cts_in, self.nets.dcd_in] {
            if board.net(net) != control {
                board.drive(net, control);
                moved = true;
            }
        }
        // Out: the next bit of the frame, or a new frame, or the idle mark.
        // A frame is over when its last stop bit has had its whole bit
        // time, not when it is put on the line.
        let mark = match self.sending.take() {
            Some((bits, k, at)) if at <= now => {
                if k < bits.len() {
                    let bit = bits[k];
                    self.sending = Some((bits, k + 1, at + self.bit_ns));
                    bit
                } else {
                    self.sent += 1;
                    true
                }
            }
            Some(pending) => {
                self.sending = Some(pending);
                self.line.unwrap_or(true)
            }
            None => match self.queue.pop_front() {
                Some(byte) => {
                    let bits = self.framing.encode(byte);
                    self.sending = Some((bits, 1, now + self.bit_ns));
                    false
                }
                None => true,
            },
        };
        moved |= self.drive_line(board, mark);
        // In: the start bit's edge, then the middle of every bit.
        let out = board.net(self.nets.data_out);
        let mark_in = out != Level::High;
        match self.receiving {
            None => {
                if self.out_was == Level::Low && out == Level::High {
                    self.receiving = Some((now + self.bit_ns / 2, 0, 0));
                }
            }
            Some((at, k, bits)) if at <= now => {
                let data = self.framing.bits;
                let parity = self.framing.parity.is_some() as u8;
                let next = at + self.bit_ns;
                if k == 0 {
                    // The start bit itself: low, or a false start.
                    self.receiving = if mark_in { None } else { Some((next, 1, 0)) };
                } else if k <= data {
                    self.receiving = Some((next, k + 1, bits | (mark_in as u16) << (k - 1)));
                } else if k <= data + parity {
                    self.receiving = Some((next, k + 1, bits));
                } else {
                    if !mark_in {
                        self.framing_errors += 1;
                    }
                    self.received.push(bits as u8);
                    self.receiving = None;
                }
            }
            Some(_) => {}
        }
        self.out_was = out;
        if moved {
            board.transition(now);
        }
        moved
    }
}

// --- The route out of the process -------------------------------------------

/// How many characters the far end will hold for the port before the
/// endpoint stops reading the socket.
///
/// A serial line has no buffer at all and the port has one character of
/// one, but muir reads the socket in bursts tens of milliseconds apart, so
/// the far end holds what arrived between two of them and the receiver
/// takes them a frame at a time, each from when it was sent. Past this the
/// socket is left unread and TCP's own window holds the rest back at
/// whoever is typing --- which is what a far end that cannot keep up does.
const BACKLOG: usize = 256;

/// Where the serial port is reached from outside the process: a TCP
/// endpoint, and the one device connected to it.
///
/// **A TCP endpoint and not a pseudo-terminal.** A pty would look like a
/// serial device, so someone would attach `screen /dev/ttys004 9600` and
/// that 9600 would mean nothing: the rate is whatever the machine has
/// programmed into the 2651, and the far end of a null-modem cable has no
/// say in it. Nothing here picks a rate or a frame, and nothing here can
/// report a mismatch either --- a far end at the wrong rate produces
/// garbage, as it does on a real line.
///
/// **One device.** One thing is on the far end of a null-modem cable, so a
/// second connection is closed as it arrives rather than shouting over the
/// first.
///
/// **Connecting is plugging in.** The Signetics sheet: the chip "is
/// conditioned to transmit data when the -CTS input is low" and
/// "conditioned to receive data when the -DCD input is low", so carrying
/// bytes alone would leave the port unable to do anything with them. A
/// connection asserts `DSR`, `DCD` and `CTS`, as a device does with its own
/// `DTR` and `RTS` across a null-modem cable, and hanging up drops all
/// three.
pub struct Endpoint {
    listener: TcpListener,
    /// The device on the cable, if one is connected, and where from.
    device: Option<(TcpStream, SocketAddr)>,
    /// What the port has sent and the socket has not taken.
    outbox: VecDeque<u8>,
    /// Whether a device coming and going is printed as it happens.
    pub trace: bool,
}

/// What one turn at the socket did to the cable: a device plugged in, or
/// the one that was there hung up.
enum Change {
    PluggedIn,
    HungUp,
}

impl Endpoint {
    /// Listens on `addr`, without blocking.
    ///
    /// Port 0 asks the host for one, which is what `tests/serial_endpoint.rs`
    /// does; [`Endpoint::addr`] then says which.
    pub fn bind(addr: SocketAddr) -> std::io::Result<Endpoint> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(Endpoint { listener, device: None, outbox: VecDeque::new(), trace: false })
    }

    /// Where it is listening.
    pub fn addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Whether a device is on the cable.
    pub fn connected(&self) -> bool {
        self.device.is_some()
    }

    /// One turn at the socket: what the port sent as far as the socket will
    /// take it now, up to `room` characters typed at it, and whoever has
    /// arrived. Never blocks.
    ///
    /// The device that is there is served before a new one is accepted, so
    /// that someone who hangs up and attaches again is not turned away by
    /// the connection they have just dropped. What the cable saw is the two
    /// ends of the turn compared: a device that went and another that came
    /// in the same turn leaves the port plugged in throughout, which is one
    /// device on the cable rather than none.
    fn service(&mut self, room: usize) -> (Option<Change>, Vec<u8>) {
        let was = self.device.is_some();
        let mut gone = None;
        if let Some((stream, who)) = self.device.as_mut() {
            while !self.outbox.is_empty() {
                let (head, tail) = self.outbox.as_slices();
                let head = if head.is_empty() { tail } else { head };
                match stream.write(head) {
                    Ok(0) => {
                        gone = Some(*who);
                        break;
                    }
                    Ok(n) => {
                        self.outbox.drain(..n);
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => {
                        eprintln!("serial: {who}: {e}");
                        gone = Some(*who);
                        break;
                    }
                }
            }
        }
        let mut typed = Vec::new();
        if gone.is_none()
            && let Some((stream, who)) = self.device.as_mut()
        {
            let mut buf = [0u8; 256];
            while typed.len() < room {
                let want = buf.len().min(room - typed.len());
                match stream.read(&mut buf[..want]) {
                    Ok(0) => {
                        gone = Some(*who);
                        break;
                    }
                    Ok(n) => typed.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => {
                        eprintln!("serial: {who}: {e}");
                        gone = Some(*who);
                        break;
                    }
                }
            }
        }
        if let Some(who) = gone {
            if self.trace {
                eprintln!("serial: {who} hung up");
            }
            self.device = None;
            // What the port sent and the device never took goes with it: a
            // cable pulled out drops whatever was on the wire.
            self.outbox.clear();
        }
        loop {
            match self.listener.accept() {
                Ok((stream, who)) => {
                    if self.device.is_some() {
                        if self.trace {
                            eprintln!("serial: {who} turned away: a device is on the cable");
                        }
                        continue;
                    }
                    if let Err(e) = stream.set_nonblocking(true) {
                        eprintln!("serial: {who}: {e}");
                        continue;
                    }
                    // A character at a time is the whole traffic here;
                    // waiting to coalesce would only add latency.
                    let _ = stream.set_nodelay(true);
                    if self.trace {
                        eprintln!("serial: {who} connected");
                    }
                    self.device = Some((stream, who));
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    eprintln!("serial: {e}");
                    break;
                }
            }
        }
        let change = match (was, self.device.is_some()) {
            (false, true) => Some(Change::PluggedIn),
            (true, false) => Some(Change::HungUp),
            _ => None,
        };
        (change, typed)
    }

    /// One turn for the behavioural far end at `now` on the machine's
    /// clock: what the port has finished sending goes to the socket, and
    /// what was typed at the socket goes on the cable as sent now.
    ///
    /// The port takes its own frame time over each character either way ---
    /// [`Pci::advance`] does that --- so a burst read off the socket in one
    /// turn is still received one frame at a time.
    pub fn poll_cable(&mut self, cable: &mut Cable, now: u64) {
        while let Some((_, byte)) = cable.take() {
            self.outbox.push_back(byte);
        }
        let room = BACKLOG.saturating_sub(cable.pending());
        let (change, typed) = self.service(room);
        match change {
            Some(Change::PluggedIn) => cable.plug(now),
            Some(Change::HungUp) => cable.unplug(),
            None => {}
        }
        for byte in typed {
            cable.send(byte, now);
        }
    }

    /// The same for the netlist board's far end, which keeps its own time:
    /// [`OnCable::apply`] moves the wires, and this only hands it
    /// characters and takes the ones it has assembled.
    pub fn poll_on_cable(&mut self, far: &mut OnCable) {
        self.outbox.extend(far.received.drain(..));
        let room = BACKLOG.saturating_sub(far.queued());
        let (change, typed) = self.service(room);
        match change {
            Some(Change::PluggedIn) => far.plug(),
            Some(Change::HungUp) => far.unplug(),
            None => {}
        }
        for byte in typed {
            far.send(byte);
        }
    }
}
