// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pinouts for the parts on the CADR's boards.
//!
//! Every entry records where it came from. Nothing here is copied from another
//! project's models: pinouts and gate structure are read off the
//! manufacturers' datasheets, TI's, AMD's and National's, and then checked
//! against the netlist itself by
//! `tests/part.rs`, which verifies that no part touches its own ground or
//! supply pin and that no net is driven by two totem-pole outputs.
//!
//! That second check is the useful one. It compares a pinout against how the
//! processor's 1243 gate records are actually wired across its 2782 nets, so a
//! wrong output pin shows up as a driver conflict rather than as silently wrong
//! logic later.

/// How a pin behaves electrically.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Drive {
    /// Push-pull. Two of these on one net is a conflict.
    Totem,
    /// Can only pull low; the net needs a pull-up. Many may share a net.
    OpenCollector,
    /// Can only pull high; the net needs a pull-down. The mirror image of
    /// open collector, and what every MECL 10K output is: an emitter
    /// follower into a terminator. Many may share a net, which MECL calls
    /// wire-OR and the display board does on ECLVID.
    OpenEmitter,
    /// Can be turned off. Many may share a net.
    TriState,
    /// A resistor. Pulls the net toward the level it drives, and any real
    /// driver wins: the packs drive high, which is what makes an
    /// open-collector net read as one, and a two-pin resistor to a rail
    /// drives that rail's level, up to VCC or down to ground.
    PullUp,
    /// Not a driver at all: connector pins and dummies.
    Passive,
}

/// How far to trust an entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// Read off the part's datasheet.
    Datasheet,
    /// Standard pinout for the family, not yet read off the datasheet.
    /// Still checked by the netlist consistency tests.
    Family,
}

#[derive(Clone, Copy, Debug)]
pub struct Pinout {
    /// DIP pin count. Ground is `package / 2` and supply is `package`.
    /// Zero means the part has no standard power pins --- resistor packs and
    /// the like --- and the supply-pin check does not apply.
    pub package: u8,
    /// Pins the part drives.
    pub outputs: &'static [u8],
    /// Drive strength of the outputs, except those in `oc_pins`.
    pub drive: Drive,
    /// Outputs that are open collector even though the rest are not. The
    /// 74S181's A=B is the case that matters: eight of them share one net.
    pub oc_pins: &'static [u8],
    pub source: Source,
}

impl Pinout {
    pub fn gnd(&self) -> u8 {
        self.package / 2
    }
    pub fn vcc(&self) -> u8 {
        self.package
    }
    pub fn drives(&self, pin: u8) -> bool {
        self.outputs.contains(&pin)
    }

    /// How a given output pin drives, or `None` if it is not an output.
    pub fn drive_of(&self, pin: u8) -> Option<Drive> {
        if !self.drives(pin) {
            return None;
        }
        Some(if self.oc_pins.contains(&pin) { Drive::OpenCollector } else { self.drive })
    }
}

const fn p(package: u8, outputs: &'static [u8], drive: Drive, source: Source) -> Pinout {
    Pinout { package, outputs, drive, oc_pins: &[], source }
}

const fn p_oc(
    package: u8,
    outputs: &'static [u8],
    drive: Drive,
    oc_pins: &'static [u8],
    source: Source,
) -> Pinout {
    Pinout { package, outputs, drive, oc_pins, source }
}

use Drive::*;
use Source::*;

/// Splits a SUDS type name into its base and whether it is open collector.
///
/// A trailing `O` marks an open-collector variant. Trailing `A` and `W` are
/// speed or process variants: same pinout, same logic. Those two only come
/// off a numbered type, `74S04A`, `74S00W`: the memory board's `DIPSW` is a
/// switch, not a `DIPS` in a W process.
pub fn strip(kind: &str) -> (&str, bool) {
    let (base, oc) = match kind.strip_suffix('O') {
        Some(b) if !b.is_empty() => (b, true),
        _ => (kind, false),
    };
    if !base.starts_with(|c: char| c.is_ascii_digit()) {
        return (base, oc);
    }
    let base = base.strip_suffix('A').unwrap_or(base);
    let base = base.strip_suffix('W').unwrap_or(base);
    (base, oc)
}

/// Looks up a part type as the netlist spells it.
/// The I/O board's drawings spell the same parts other ways: `LS00L` for
/// a 74LS00, `OLS14L` for a 74LS14, `37L` for a 74S37, `279A` for a
/// 74279, `DM8136` for the 8136. The logic families differ only in
/// speed, which the model has none of, so each is the part the table
/// already has, spelt as the table has it, before [`strip`] reads the
/// open-collector `O` off the end.
fn canonical(kind: &str) -> &str {
    match kind {
        "LS00L" | "OLS00L" => "74S00",
        "LS02L" | "OLS02L" => "74S02",
        "LS08L" | "OS08L" => "74S08",
        "LS10L" => "74S10",
        "LS14L" | "OLS14L" => "74LS14",
        "LS32L" | "OLS32L" => "74S32",
        "LS86L" => "74S86",
        "S04L" | "LS04L" | "OLS04L" => "74S04",
        "S260L" | "OS260L" => "74S260",
        "S02L" => "74S02",
        "S133L" => "74S133",
        "S11L" | "LS11L" => "74S11",
        "LS21L" | "OLS21L" => "74S21",
        "OLS08L" => "74S08",
        // Three the disk controller spells with its own family prefix, the
        // same parts as the table's: the logic differs only in speed.
        "74LS153" => "74S153",
        "74LS157" => "74S157",
        "74LS279" => "74279",
        "9S42" => "9S42-1",
        // The bus interface's one J-K flip-flop, drawn as the `-1` body
        // variant of the same part.
        "74LS112-1" => "74S112",
        "S38L" => "74S38O",
        "37L" => "74S37",
        "LS109" => "74LS109",
        "DM8136" => "8136",
        "74LS138" => "74S138",
        "74LS175" => "74S175",
        "74S163" => "74LS163",
        // Am25LS193 (`am25ls193.pdf`) is the 74LS193 pin for pin: `PL` 11,
        // `A` 15, `B` 1, `C` 10, `D` 9, `CPU` 5, `CPD` 4, `MR` 14, `QA` 3,
        // `QB` 2, `QC` 6, `QD` 7, `TCU` 12, `TCD` 13. AMD's improvements
        // are electrical --- speed, drive --- not functional.
        "25LS193" => "74LS193",
        // The Chaosnet pages spell the shift register without its family
        // letters; `74LS164` is the same body, and IOBKBD uses that name
        // for the three on the keyboard.
        "74164" => "74LS164",
        // The hex D flip-flop, likewise: the I/O board's own pages write
        // `74S174`.
        "74LS174" => "74S174",
        // `OS133L` is the 74S133, totem-pole: the `O` in these body names is
        // not open collector, as `OS00L` and `OLS14L` say --- neither the
        // 74S00 nor the 74LS14 has such a variant. Open collector is the
        // `O` at the end, which `strip` reads.
        "OS133L" => "74S133",
        "279A" => "74279",
        "279B" => "74279",
        "S112" => "74S112",
        "CAP" => "CAP1",
        "20DUMMY" => "16DUMMY",
        // The Chaosnet transceiver module at LMDETC A03: sixteen pins the
        // drawing wires and no logic of its own, as the other dummy bodies.
        "DUMMY" => "16DUMMY",
        // The display board draws one 74LS244 as two bodies on two pages,
        // `-A` and `-B`, one buffer each: NXBCTL 0F11 has the mode
        // register's read buffer on pins 1-2/4/6/8 and 12/14/16/18, NRACOL
        // 0F11 the colour buffer on 11/13/15/17/19 and 3/5/7/9. The pins
        // are the package's, so both are the whole part.
        "74LS244-A" | "74LS244-B" => "74LS244",
        // The display board's MECL bodies carry `I` and `AI` suffixes ---
        // `10102I`, `10102AI`, `10212I` --- for the drawing's inverted and
        // alternate symbols of one part. Same die, same pins.
        "10102I" | "10102AI" => "10102",
        "10212I" => "10212",
        "10105AI" => "10105",
        "10121I" => "10121",
        // The LISPM TV's spellings. Intel 2118 (`d2118.pdf`): 16K x 1
        // dynamic RAM in the 4116's package with the 4116's pins --- DIN 2,
        // WE 3, RAS 4, A0 5, A2 6, A1 7, A5 10, A4 11, A3 12, A6 13, DOUT 14,
        // CAS 15 --- on a single 5 V supply, VDD 8 and VSS 16, where the
        // 4116 has VBB on 1 and VCC on 9; the 2118 leaves those two open.
        // Same cells, same cycle, same model.
        "2118" => "4116VG",
        // Intel 2141 (`d2141.pdf`), 4K x 1 static RAM, "Industry Standard
        // 2147 Pinout" in the sheet's own words: A0..A5 on 1..6, DOUT 7,
        // -WE 8, GND 9, -CS 10, DIN 11, A6..A11 on 12..17, VCC 18, which is
        // how SYNRAM wires it. Same cells, same model.
        "2141" => "2147",
        // SN74128 (`sn74128.pdf`), quad two-input NOR line driver: the
        // 7402's pins, 1Y=1 from 2,3 and so on.
        "74128" => "74S02",
        "74128O" => "74S02O",
        "74LS299" => "74S299",
        // `OS37L` is the 74S37 as `OS133L` is the 74S133: the leading `O`
        // is a body variant, not open collector.
        "OS37L" => "74S37",
        // A two-pin pull-up resistor, as `RES` is: NRASHF 0E13 holds `HI2`
        // up to VCC.
        "PULLUP" => "RES",
        // Datasheet `sn74ls669.pdf`: the '669 is a redesign of the 'LS169
        // --- "compared to the original 'LS168 and 'LS169, they feature
        // 0-nanosecond minimum hold time" --- with its pinout and its
        // function. U/D=1, CLK=2, A..D=3..6, ENP=7, LOAD=9, ENT=10, QD=11,
        // QC=12, QB=13, QA=14, RCO=15, which is the '169 entry below pin
        // for pin, carry rule included. NSYREG 0C02 and 0C01 are the sync
        // program's repeat counter, loaded from `SYNC 3..0` and cascaded
        // through pin 15.
        "74LS669" => "74S169",
        other => other,
    }
}

pub fn pinout(kind: &str) -> Option<Pinout> {
    let (base, oc) = strip(canonical(kind));
    let mut it = table(base)?;
    if oc {
        it.drive = OpenCollector;
    }
    Some(it)
}

fn table(base: &str) -> Option<Pinout> {
    Some(match base {
        // --- 14-pin gates, GND 7, VCC 14 ---
        // Datasheet-checked: outputs of the NOR family sit on 1/4/10/13, which
        // is the pinout mistake most worth not making.
        "7428" => p(14, &[1, 4, 10, 13], Totem, Datasheet),
        "74S02" | "OS02L" => p(14, &[1, 4, 10, 13], Totem, Family),
        "74S00" | "OS00L" | "S00L" => p(14, &[3, 6, 8, 11], Totem, Family),
        "74S08" | "S08L" => p(14, &[3, 6, 8, 11], Totem, Family),

        // --- the memory board, `data/CADRM.netlist` ---
        // Every entry below is read off the drawing's own use of the part,
        // `cadrm/*.drw`, because no datasheet we have covers them; the
        // page that uses each is named, and `tests/cadrm_netlist.rs` checks
        // the wiring against these the way the processor's are checked.
        //
        // MK4116, 16K x 1 dynamic RAM: the bank pages wire pin 2 to `XBI`,
        // 3 to `-WRITE`, 4 to `-RAS`, 15 to `-CAS`, 14 to `XBO`, and the
        // address `ADR 0..6` to 5, 7, 6, 12, 11, 10, 13 --- the standard
        // pinout, with its three supplies where the standard has them: VBB
        // on 1, VDD on 8, VCC on 9, and VSS on 16, so the package's own
        // convention for ground and supply does not apply.
        "4116VG" => p(0, &[14], TriState, Family),
        // A 14-pin crystal oscillator module, output on 8: `memctl`'s
        // `CTL CLK`. Its frequency is not on any drawing; see `src/chip.rs`.
        "DIPOSC" => p(0, &[8], Totem, Family),
        // Am26S02, the Schottky 9602: dual retriggerable resettable
        // one-shot on the 9602's pinout (Am26S02 datasheet, connection
        // diagram): `Cx` 1 and 15, `Rx/Cx` 2 and 14, `-CLR` 3 and 13, the
        // rising-edge trigger `B` (`I1`) 4 and 12, the falling-edge trigger
        // `A` (`I0`) 5 and 11, `Q` 6 and 10, `-Q` 7 and 9. Section one on
        // `memctl` at 0F02 has its timing network on 1 and 2, the clear on
        // 3 held high, `B` on 4 grounded, `A` on 5 from `-REFRESH NOW`,
        // whose fall triggers it, `Q` on 6 and `-Q` on 7 as `TIME FOR
        // REFRESH`. `src/chip.rs` runs it; the second section is not used.
        "26S02" => p(16, &[6, 7, 9, 10], Totem, Family),
        // DM8136, six-bit bus comparator, open collector: `memxba` 0F21
        // compares `-XADDR21..16` on 1, 3, 5, 11, 13, 15 with the switch on
        // 2, 4, 6, 10, 12, 14, grounds 7 --- the strobe --- and the result
        // on 9 is `BOARD SELECT`, pulled up on RES20.
        "8136" => p(16, &[9], OpenCollector, Family),
        // Am25LS2539 (`am25ls2539.pdf`), dual one-of-four decoder with
        // three-state outputs; the pins and the function are in `behaviour`
        // below. `memras` 0F13 uses decoder 2 to steer `RAS` to a bank: the
        // bank number `XBAI14`, `XBAI15` on `A` 17 and `B` 18, `REFRESH
        // CYC` on `POL` 4 and `E` 16, `-OE` 5 grounded, and the four bank
        // enables out on 3, 2, 1 and 19 into the 74S37 drivers.
        "25LS2539" => p(20, &[1, 2, 3, 19, 8, 9, 11, 12], TriState, Family),
        // SN74LS569A, four-bit up/down counter with three-state outputs:
        // `memads` 0F15/0F17 count refresh rows, clocked on 2 by `-BUSY`,
        // outputs 16, 15, 14, 13 enabled on 17 by `-REFRESH CYC`, the
        // carry from 19 into the next stage's 12, 7 grounded, 8, 9 and 11
        // held high, 1 high for up.
        "74LS569" => p(20, &[13, 14, 15, 16, 18, 19], TriState, Family),
        // The I/O board's parts, each from its datasheet.
        // SN74279, quad S-R latch: Q on 4, 7, 9, 13.
        "74279" => p(16, &[4, 7, 9, 13], Totem, Datasheet),
        // SN74393, dual 4-bit binary counter: 1Q on 3..6, 2Q on 11, 10, 9, 8.
        "74393" => p(14, &[3, 4, 5, 6, 8, 9, 10, 11], Totem, Datasheet),
        // SN74LS164, 8-bit serial-in shift register: QA..QH on 3..6, 10..13.
        "74LS164" => p(14, &[3, 4, 5, 6, 10, 11, 12, 13], Totem, Datasheet),
        // SN74165 (`sn74165.pdf`), parallel-load 8-bit shift register,
        // 16-pin: `SH/-LD` 1, `CLK` 2, `E`..`H` 3 to 6, `-QH` 7, `QH` 9,
        // `SER` 10, `A`..`D` 11 to 14, `CLK INH` 15. Only 7 and 9 drive.
        "74165" => p(16, &[7, 9], Totem, Datasheet),
        // SN74LS193, 4-bit up/down counter: QA..QD on 3, 2, 6, 7; -CO 12, -BO 13.
        "74LS193" => p(16, &[2, 3, 6, 7, 12, 13], Totem, Datasheet),
        // Am25LS2521, 8-bit equal comparator: -EOUT on 19.
        "25LS2521" => p(20, &[19], Totem, Datasheet),
        // DM8837, hex unified bus receiver: OUT1..6 on 14, 12, 10, 2, 4, 6.
        "8837" => p(16, &[2, 4, 6, 10, 12, 14], Totem, Datasheet),
        // SN75118, differential line transceiver: driver DY on 4 and 3, DZ
        // on 1 and 2, receiver RY on 12 and 11, all three-state.
        "75118" => p(16, &[1, 2, 3, 4, 11, 12], TriState, Datasheet),
        // MC1488, quad RS-232 line driver: Y on 3, 6, 8, 11.
        "MC1488" => p(14, &[3, 6, 8, 11], Totem, Datasheet),
        // MC1489, quad RS-232 line receiver: Y on 3, 6, 8, 11.
        "MC1489" => p(14, &[3, 6, 8, 11], Totem, Datasheet),
        // Signetics 2651 PCI, the serial port: from the sheet's PIN
        // DESIGNATION table, the data bus D0..D7 on 27, 28, 1, 2, 5, 6, 7,
        // 8, TxD 19, -TxRDY 15, -RxRDY 14, -TxEMT/DSCHG 18, -DTR 24, -RTS
        // 23, and every other pin an input; -TxC on 9 and -RxC on 25 are
        // outputs only with the internal clock, and the board connects
        // neither. Three-state is the data bus's, driven while -CE is low
        // for a read. The three ready outputs are open drain --- Table 2
        // says so of each, and note 7 of the electrical characteristics
        // again --- which is what lets ECO 10 of `cadrio/iob.eco` put
        // -TxRDY on -RxRDY's net, the pull-up at CLK60H 0C20 holding it
        // up; see `behaviour`.
        "2651" => p_oc(
            28,
            &[27, 28, 1, 2, 5, 6, 7, 8, 19, 15, 14, 18, 24, 23],
            TriState,
            &[14, 15, 18],
            Datasheet,
        ),
        // Quad two-to-one multiplexer, three-state: `memads` 0F14/0F16 pick
        // row or column address on 1 and enable on 15, as the 74LS157 with
        // an output enable.
        // SN74LS257 (`sn74257.pdf`): select on 1, low for A,
        // output control on 15, Y on 4, 7, 9, 12.
        "74LS257" => p(16, &[4, 7, 9, 12], TriState, Datasheet),
        // A DIP switch: `memxba` 0F11 grounds 3..8 and switches them to
        // `SW 16..21` on 14..9. A closed switch pulls its line low; the
        // setting is the harness's, `Chip::set_switches`.
        "DIPSW" => p(0, &[9, 10, 11, 12, 13, 14], OpenCollector, Family),
        // The Chaosnet address switches at LMMYNM D10 and D12: a 16-pin
        // package of eight switches, each joining pin `k` to pin `17 - k`
        // as TRITERM's resistors do. Pins 1 to 8 are grounded on both
        // bodies and 9 to 16 carry `MY#0..15`, so a closed switch grounds
        // its bit against the pull-up pack and an open one leaves it high.
        "SWITCH" => p(0, &[9, 10, 11, 12, 13, 14, 15, 16], OpenCollector, Family),
        // Series damping resistors between the drivers and the DRAM
        // address, RAS and CAS lines: pin k to pin 17-k, eight of them.
        // `SERRESL` is the memory board's pack and `898-3-R22` the display
        // board's, on NRABUF between the address buffers and the four rows
        // of RAM --- a BI Technologies model 898, `beckman-898.pdf`, whose
        // `-3` is the isolated-resistor circuit; which pin pairs with which
        // is the drawing's, `RAM BFR 0` in on 1 and `RAMB A0` out on 16.
        // `src/netlist.rs` joins each pair into one net; the part drives
        // nothing.
        "SERRESL" | "898-3-R22" => p(0, &[], Passive, Family),
        // The one-shot's timing network and the bank pages' decoupling.
        "CAP1" | "RES1" | "1UFCAP" | ".1UFCAP" => p(0, &[], Passive, Family),
        "74S32" | "OS32L" => p(14, &[3, 6, 8, 11], Totem, Family),
        "74S37" | "S37L" => p(14, &[3, 6, 8, 11], Totem, Family),
        "74S86" | "S86L" => p(14, &[3, 6, 8, 11], Totem, Family),
        "74S04" | "74LS14" | "OS04L" => p(14, &[2, 4, 6, 8, 10, 12], Totem, Family),
        "74S10" | "74S11" | "OS11L" => p(14, &[12, 6, 8], Totem, Family),
        "74S20" => p(14, &[6, 8], Totem, Family),
        // `sn7451.pdf`, which covers the 'S51: "The '51 and 'S51 contain
        // two independent 2-wide 2-input AND-OR-INVERT gates. They perform
        // the Boolean function Y = AB + CD." 14-pin: `1A` 1, `2A` 2, `2B`
        // 3, `2C` 4, `2D` 5, `2Y` 6, `1Y` 8, `1C` 9, `1D` 10, `1B` 13, and
        // 11 and 12 no connection. The disk controller's `UCLK^` and the
        // I/O board's Chaosnet half both use it; the I/O board's drawings
        // still draw two of them as 9S42s.
        "74S51" => p(14, &[6, 8], Totem, Datasheet),
        "74S64" => p(14, &[8], Totem, Family),
        "74S260" => p(14, &[5, 6], Totem, Family),
        "74S280" => p(14, &[5, 6], Totem, Family),
        "74S74" | "74LS74" | "74LS74I" | "LS74" | "S74" => p(14, &[5, 6, 8, 9], Totem, Family),

        // --- 16-pin, GND 8, VCC 16 ---
        "74S133" => p(16, &[9], Totem, Family),
        "74S138" => p(16, &[7, 9, 10, 11, 12, 13, 14, 15], Totem, Family),
        "74S139" => p(16, &[4, 5, 6, 7, 9, 10, 11, 12], Totem, Family),
        "74S151" => p(16, &[5, 6], Totem, Family),
        "74S153" => p(16, &[7, 9], Totem, Family),
        "74S157" => p(16, &[4, 7, 9, 12], Totem, Family),
        // SN74S158 (`sn74s158.pdf`), the '157 with inverted outputs:
        // "the 'LS158 and 'S158 present inverted data to minimize
        // propagation delay time". Same pinout, totem-pole like the '157,
        // so the strobe forces a high rather than a high impedance.
        "74S158" => p(16, &[4, 7, 9, 12], Totem, Datasheet),
        "74S258" => p(16, &[4, 7, 9, 12], TriState, Family),
        "74S174" => p(16, &[2, 5, 7, 10, 12, 15], Totem, Family),
        "74S175" => p(16, &[2, 3, 6, 7, 10, 11, 14, 15], Totem, Family),
        "74S194" => p(16, &[12, 13, 14, 15], Totem, Family),
        "74S169" => p(16, &[11, 12, 13, 14, 15], Totem, Family),
        "74S283" => p(16, &[1, 4, 9, 10, 13], Totem, Family),
        // Datasheet: G0=3 G1=1 G2=14 G3=5 and P0=4 P1=2 P2=15 P3=6 are
        // inputs, fed from the 181s' P and G. Outputs are P=7, Cn+z=9, G=10,
        // Cn+y=11, Cn+x=12.
        "74S182" => p(16, &[7, 9, 10, 11, 12], Totem, Datasheet),
        // Datasheet: 1PRE is pin 5, an input. Outputs are 1Q=6, 1Q'=7,
        // 2Q'=9, 2Q=10.
        "74LS109" => p(16, &[6, 7, 9, 10], Totem, Datasheet),

        // --- 20-pin, GND 10, VCC 20 ---
        "74S240" | "74LS240" | "74S241" | "74LS244" => {
            p(20, &[3, 5, 7, 9, 12, 14, 16, 18], TriState, Family)
        }
        "74S373" | "74S374" | "74LS374" => p(20, &[2, 5, 6, 9, 12, 15, 16, 19], TriState, Family),
        // 512x8 PROM. Confirmed against the netlist, which is unambiguous:
        // PROM0 1E17 has -PROMPC0..4 on 1-5, I0..I7 on 6-9 and 11-14,
        // -PROMCE0 on 15 and -PROMPC5..8 on 16-19. An earlier entry here
        // claimed outputs on 10, 15, 16 and 17 --- pin 10 is ground, 15 is the
        // chip enable and 16/17 are address. It was missed because tri-state
        // parts are exempt from the two-drivers check; see the test below.
        "74S472" => p(20, &[6, 7, 8, 9, 11, 12, 13, 14], TriState, Datasheet),

        // --- Memories ---
        // Datasheet: 18-pin, GND 9, VCC 18. A0-A5 on 1-6, Dout on 7, WE on 8,
        // CS on 10, Din on 11, A6-A11 on 12-17. Three-state output.
        "2147" => p(18, &[7], TriState, Datasheet),
        // Datasheet: 16-pin, GND 8, VCC 16. CS on 1, A0-A4 on 2-6, Dout on 7,
        // A5-A9 on 9-13, WE on 14, Din on 15. Three-state output.
        "93425" => p(16, &[7], TriState, Datasheet),

        // Datasheet: 12-input parity checker, 16-pin, GND 8, VCC 16.
        // Odd parity out on 9, even on 10; everything else is an input.
        "93S48" => p(16, &[9, 10], Totem, Datasheet),
        // Datasheet: 32x2 RAM, 16-pin, GND 8, VCC 16. O0 on 7, O1 on 9, and
        // the outputs are open collector --- pulled up by RES20 packs in
        // MCTL for M memory and in SPC for the microcode stack.
        "82S21" => p(16, &[7, 9], OpenCollector, Datasheet),

        // --- AMD registers and shifter, 16-pin, GND 8, VCC 16 ---
        // Datasheet: hex D register with enable. E=1, CP=9, D on 3/4/6/11/13/14,
        // Q on 2/5/7/10/12/15.
        "25S07" => p(16, &[2, 5, 7, 10, 12, 15], Totem, Datasheet),
        // Datasheet: quad two-input register. S=1, CP=9, Q0-Q3 on 2/7/10/15.
        "25S09" => p(16, &[2, 7, 10, 15], Totem, Datasheet),
        // Datasheet: four-bit shifter. I-3..I3 on 1-7, S1=9, S0=10, OE=13,
        // Y3=11, Y2=12, Y1=14, Y0=15. Three-state outputs.
        "25S10" => p(16, &[11, 12, 14, 15], TriState, Datasheet),

        // --- 32x8 PROMs, 16-pin, GND 8, VCC 16 ---
        // Outputs on 1-7 and 9, address on 10-14, enable on 15. Confirmed
        // against the netlist: in MSKG4 the eight output pins carry MSK0-7 and
        // the address pins carry MSKL0-4. The 5600 has open-collector
        // outputs, pulled up by the RES20 packs in MSKG4; the 5610 is the
        // three-state part, used once in DSPCTL for the dispatch mask.
        // Intersil IM5600/IM5610 (`im5600.pdf`): O1..O8 on 1..7 and 9, A0..A4 on
        // 10..14, `-CE` on 15.
        "5600" => p(16, &[1, 2, 3, 4, 5, 6, 7, 9], OpenCollector, Datasheet),
        "5610" => p(16, &[1, 2, 3, 4, 5, 6, 7, 9], TriState, Datasheet),

        // Datasheet: dual 8-bit shift register, 16-pin. Register 1 has
        // Q7 on 14 and Q7' on 15; register 2 has Q7 on 3 and Q7' on 2.
        "9328" => p(16, &[2, 3, 14, 15], Totem, Datasheet),
        // Datasheet: 6-bit identity comparator, 16-pin. A=B out on 9; it
        // carries APASS1 on ACTL 3B21, the A-memory write pass-around.
        "93S46" => p(16, &[9], Totem, Datasheet),
        // Fairchild 9S42, 16-pin, two gates mirrored about the supply pins, so
        // the outputs are the two pins next to ground: 7 and 9. Confirmed
        // against the netlist: on 1A15 pin 9 carries RAMDISABLE, which is
        // computed from the other pins, so 9 is an output and 15 (IDEBUG) is
        // an input.
        "9S42-1" => p(16, &[7, 9], Totem, Datasheet),

        // --- 20-pin, GND 10, VCC 20 ---
        // Datasheet: quad register with two independently controlled
        // three-state output sets. W, on 2/5/12/15, is enabled by pin 7 and
        // inverted by pin 18; Y, on 3/6/11/14, is enabled by pin 8. Used
        // once, on page FLAG, to hold the interrupt-control flags: there pin
        // 7 is tied high and the four Y pins carry the flags, which is how
        // round the two sets are.
        "25LS2519" => p(20, &[2, 3, 5, 6, 11, 12, 14, 15], TriState, Datasheet),

        // --- Delay lines. MIT's `TDnn` is Engineered Components' TTLDM-nn,
        // --- whose sheet gives "20% taps" and draws them pin 1 in, 12 at
        // --- 20%, 4 at 40%, 10 at 60%, 6 at 80%, 8 the whole line, in a
        // --- 14-pin package with Vcc on 14 and ground on 7; it lists
        // --- TTLDM-25, -50, -100 and -250, the four the CADR uses. MIT's
        // --- own wire lists label every pin to match: the TD100 at
        // --- `cadrwd/icmem3.wlr` 1D12 reads 1:IN 12:20NS 4:40NS 10:60NS
        // --- 6:80NS 8:100NS and 14:+5.00V.
        "TD25" | "TD50" | "TD100" | "TD250" => p(14, &[4, 6, 8, 10, 12], Totem, Datasheet),
        // The `NC` bodies on the Chaosnet pages are the same modules with
        // the same five taps: `cadrio/iob.wlr` gives B04 and B11 the TD100
        // labels above and A11 the TD25's 5, 10, 15, 20 and 25 ns. What the
        // `NC` in the body name means is two extra wire-wrap posts, 3 and 5,
        // that the DIP has not got --- `cadrio/iob.wls` files A11, B04 and
        // B11 pins 3 and 5 under "BODY/DIP SOCKET MATCHING ERRORS" as
        // `UN OR NC PIN`, and prints `NO DRIVE` on the four nets the
        // drawings hang from them. Their driver arrives by hand: the
        // consolidated IOB ECO of 2/18/81 in `cadrio/iob.eco` straps each
        // dead post to a real tap ("`B4-8 : B4-11` ;taps TD100 at 100 ns"
        // and three more). So the part drives its taps and nothing else;
        // `src/chip.rs` adds the straps, and runs all of these by time.
        "TD25NC" | "TD100NC" => p(14, &[4, 6, 8, 10, 12], Totem, Datasheet),

        // A numeric display with a latch and decoder. Everything it uses is an
        // input; it drives only its own LEDs.
        "TIL309" => p(0, &[], Passive, Family),

        // --- Resistor packs. Pin 1 is the common, tied to the supply, and
        // --- every other pin is one resistor. They are what pulls an
        // --- open-collector net high: the RES20s in MCTL and SPC carry
        // --- MMEM and SPCO, and the SIPs terminate the bus.
        "RES20" => {
            p(0, &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19], PullUp, Family)
        }
        // A two-pin resistor, as the I/O board draws its pull-ups to VCC and
        // the grant chain's termination to ground: each end pulls toward
        // the other's level.
        "RES" => p(0, &[1, 2], PullUp, Family),
        "SIP220/330-8" | "SIP330/470-8" | "SIP180/390-8" => {
            p(0, &[2, 3, 4, 5, 6, 7], PullUp, Family)
        }

        // A dummy DIP: a socket carrying wire links, driving nothing.
        "16DUMMY" => p(0, &[], Passive, Family),

        // --- The bus interface's own parts -------------------------------
        // Everything below is used only by data/BUSINT.netlist. Where a
        // datasheet settled the pinout it says so, and where the wiring
        // settled it the wiring is quoted.

        // Triple three-input NOR, 14-pin: 1Y=12, 2Y=6, 3Y=8.
        "74LS27" => p(14, &[6, 8, 12], Totem, Family),
        // Quad two-input NAND *buffer*, the open-collector 74S37. The
        // netlist spells every one of the six `74S38O`, so the suffix
        // asserts what the part already is.
        "74S38" => p(14, &[3, 6, 8, 11], OpenCollector, Family),

        // SN74S112 (`sn74112.pdf`), the dual J-K: the I/O board's Chaosnet
        // clock divider at LMTCLK 0A06, and the bus interface's `XB PAR
        // ERROR` flag at REQERR 0A03, drawn `74LS112-1`.
        "74S112" => p(16, &[5, 6, 7, 9], Totem, Datasheet),
        // Datasheet SN74S124: dual voltage-controlled oscillator, 16-pin.
        // 1Y=7 and 2Y=10 are the only outputs. **It has two grounds and two
        // supplies** --- "a separate set is provided for the oscillator and
        // associated frequency-control circuits so that effective isolation
        // can be accomplished" --- with OSC GND on 8 and GND on 9, OSC VCC on
        // 15 and VCC on 16, so the package is given as 0 and the supply-pin
        // check does not apply. REQTIM 0A01 uses oscillator 1, wiring the
        // two isolated pins and leaving the other pair implicit, with the
        // timing capacitor on 1CX1/1CX2 and 1Y clocking the timeout counter.
        "74LS124" => p(0, &[7, 10], Totem, Datasheet),
        // Synchronous four-bit counter, 16-pin: QA-QD on 14-11, RCO on 15.
        // UPRIOR 0C01 uses only RCO, as the grant timeout.
        "74LS163" => p(16, &[11, 12, 13, 14, 15], Totem, Family),
        // SN74161 (`sn74161.pdf`), synchronous 4-bit binary counter with
        // **direct** clear, 16-pin: `-CLR` 1, `CLK` 2, `A`..`D` 3 to 6,
        // `ENP` 7, `-LOAD` 9, `ENT` 10, `QD` 11, `QC` 12, `QB` 13, `QA` 14,
        // `RCO` 15. Same pinout as the '163 muir already has; the '163
        // clears synchronously and the '161 does not.
        "74LS161" => p(16, &[11, 12, 13, 14, 15], Totem, Datasheet),
        // Octal D with clear, 20-pin: CLR=1, CLK=11, Q on 2/5/6/9/12/15/16/19.
        // REQTIM 0B01 is one, and the netlist confirms it exactly: its eight
        // Q pins feed the 74S288's address and its eight D pins take that
        // PROM's data back.
        "74LS273" => p(20, &[2, 5, 6, 9, 12, 15, 16, 19], Totem, Family),
        // Datasheet SN74276: quadruple J-K flip flop, 20-pin, common CLR on
        // 1 and PRE on 11, separate clocks. Outputs 1Q=5, 2Q=6, 3Q=15, 4Q=16.
        // REQERR 0B02 is the error register: `XBUS REQUEST` and
        // `UNIBUS REQUEST` are the J inputs of flip flops 3 and 4 and
        // `-NXM TIMEOUT` their clock, so whichever bus was asking when the
        // timeout came sets `XB NXM ERROR` or `UB NXM ERROR`. That is
        // `machine::bus_error`, as a part.
        "74276" => p(20, &[5, 6, 15, 16], Totem, Datasheet),
        // 32x8 PROM, 16-pin, the same pinout as the 5600/5610 above:
        // outputs 1-7 and 9, address 10-14, enable 15. The netlist settles
        // it without a datasheet --- REQTIM 0A02's outputs go to 0B01's D
        // pins and its address pins come from 0B01's Q pins. This is
        // `cadr1/reqtim.prom`, "NXM TIMEOUT PROM (74S288)".
        "74S288" => p(16, &[1, 2, 3, 4, 5, 6, 7, 9], TriState, Family),

        // Datasheet Am26S10: quad open-collector bus transceiver, 16-pin,
        // GND on 1 *and* 8. Each channel is a bus pin, a receiver output and
        // a driver input; the enable is pin 12. Reading them against the
        // wiring on XA 0F21 makes every pin say something: `RESET` drives
        // `-XBUS INIT`, `CLK0` drives `-XBUS SYNC` --- which is
        // `busint.erface`'s "this clock has to go out over the Xbus as
        // XBUS.SYNC" --- and `-XBUS ACK` is only ever received, because the
        // bus interface is a master.
        //
        //     2=B0  3=Z0  4=I0  5=I1  6=Z1  7=B1
        //     9=B2 10=Z2 11=I2 12=E  13=I3 14=Z3 15=B3
        //
        // The four bus pins are open collector; the four receiver outputs
        // are not.
        "26S10" => p_oc(16, &[2, 3, 6, 7, 9, 10, 14, 15], Totem, &[2, 7, 9, 15], Datasheet),

        // Am29701: 16x4 three-state RAM. Its datasheet is a
        // redirect --- "The Am29701 is replaced by the Am27S07
        // (three-state)" --- so the pinout is the wiring's, and RBUF 0D26
        // gives it unambiguously: four address pins (1, 13, 14, 15), CE on
        // 2, WE on 3, and four data pairs mirrored about the ground pin,
        // with the `BUS` side in and the `RBUF` side out.
        "29701" => p(16, &[5, 7, 9, 11], TriState, Family),

        // Am8304: octal bidirectional bus transceiver, 20-pin. DBGOUT 0B22
        // has `UDO<15:8>` on 1-8 and `DBD<15:8>` on 12-19, `-DBD ENB` on 9
        // and the direction on 11, so both sides are driven and both are
        // three-state.
        "8304" => {
            p(20, &[1, 2, 3, 4, 5, 6, 7, 8, 12, 13, 14, 15, 16, 17, 18, 19], TriState, Family)
        }

        // Datasheet DM8838, "quad unified bus transceiver": four
        // driver/receiver pairs with "open collector driver output allows
        // wire-OR connection", which is what the Unibus is. The two disable
        // pins are 7 and 9 and the ground is the usual 8, which the drawings
        // leave off the sheet as they leave every ground pin off, while
        // wiring both disables: pin 7 to `-DRIVE.UNIBUS` on the I/O board
        // and to ground on the bus interface, pin 9 to each board's own
        // enable. The package is given as 0, which leaves it out of the
        // supply-pin check.
        //
        //     1=BUS3  2=IN3  3=OUT3  4=BUS4  5=IN4  6=OUT4  7=DISABLE B  8=GND
        //     9=DISABLE A 10=OUT2 11=IN2 12=BUS2 13=OUT1 14=IN1 15=BUS1 16=VCC
        //
        // UBA 0F12 confirms which way round `IN` and `OUT` are: pin 2 is
        // `C1 OUT` and pin 3 `C1 IN`, so the driver takes what the board
        // sends out and the receiver produces what comes in.
        "8838" => p_oc(0, &[1, 3, 4, 6, 10, 12, 13, 15], Totem, &[1, 4, 12, 15], Datasheet),

        // Engineered Components' MTTLDL-100, whose sheet is three separate
        // 100 ns lines, each buffered in and out, in a 14-pin package: IN1 1
        // to OUT1 12, IN2 3 to OUT2 10, IN3 5 to OUT3 8, V on 14 and C on 7.
        // Not a tapped line, so it has no taps. REQU 0C07 names its pair
        // `UB XBUS T0` and `UB XBUS T100`, which is where the 100 comes from.
        "MTD100" => p(14, &[8, 10, 12], Totem, Datasheet),

        // A four-pin socket carrying a link or a capacitor: REQTIM 0B03
        // holds the VCO's timing capacitor and UPRIOR 0F13 a jumper.
        "DUMMY4" => p(0, &[], Passive, Family),

        // --- 24-pin, GND 12, VCC 24 ---
        // Datasheet: F0=9 F1=10 F2=11 F3=13, A=B is 14, P=15, Cn+4=16, G=17.
        // A=B is open collector, which is how the CADR wires eight of them
        // together onto AEQM.
        "74S181" => p_oc(24, &[9, 10, 11, 13, 14, 15, 16, 17], Totem, &[14], Datasheet),

        // --- The display board's own parts -------------------------------
        // Everything below is used only by data/SIMPLETV.netlist, and every
        // one of them is read off its datasheet.

        // Datasheet `sn74s253.pdf`, dual four-to-one selector with
        // three-state outputs, 16-pin: 1G=1, B=2, 1C3..1C0=3..6, 1Y=7,
        // 2Y=9, 2C0..2C3=10..13, A=14, 2G=15. NRAADR 0A11 multiplexes
        // `TVMA` against `RAM ADR IN` onto `RAMADR 0` at pin 7 and makes
        // `-RAMWR` at pin 9, which is the two outputs.
        "74S253" => p(16, &[7, 9], TriState, Datasheet),
        // SN74S251 (`sn74s251.pdf`), eight-input selector with three-state
        // outputs, 16-pin: `D3` 1, `D2` 2, `D1` 3, `D0` 4, `Y` 5, `W` 6,
        // `-G` 7, `C` 9, `B` 10, `A` 11, `D7` 12, `D6` 13, `D5` 14, `D4`
        // 15. LMMYNM walks the board's Chaosnet address through two of
        // them, `D0` to `MY#0` and up.
        "74S251" => p(16, &[5, 6], TriState, Datasheet),
        // Fairchild 9401 CRC generator/checker (`9401.pdf`), 14-pin: `CP`
        // 1, `-P` 2, `S0` 3, `MR` 4, `S1` 5, `S2` 8, `CWE` 10, `D` 11, `Q`
        // 12, `ER` 13, and 6 and 9 not connected. The sheet's connection
        // diagram and its logic symbol disagree over pins 3, 5 and 11; the
        // board settles it, since LMTBUF C09 takes pin 11 from the transmit
        // shift register's `QH`, which can only be the data input, leaving
        // 3 and 5 as `S0` and `S1`.
        "9401" => p(14, &[12, 13], Totem, Datasheet),
        // SN74S287 (`sn74s287.pdf`), 1,024 bits as 256 words of four with
        // three-state outputs, 16-pin: address on 5, 6, 7, 4, 3, 2, 1 and
        // 15 from the low bit up (see `PROM256_IN`), the two chip selects
        // on 13 and 14, and `DO 1`..`DO 4` on 12, 11, 10, 9, low bit
        // first. LMMYNM D01 grounds both selects and takes its eight
        // address lines from the comparison state and the two serial
        // streams.
        "74S287" => p(16, &[9, 10, 11, 12], TriState, Datasheet),
        // Am26LS31 (`am26ls31.pdf`), quadruple differential line driver,
        // 16-pin: `1A` 1, `1Y` 2, `1Z` 3, `G` 4, `2Z` 5, `2Y` 6, `2A` 7,
        // `3A` 9, `3Y` 10, `3Z` 11, `-G` 12, `4Z` 13, `4Y` 14, `4A` 15.
        // LMLNDR A02 drives the Chaosnet's transmit pair from driver 1 and
        // gates all four with `LOOP.BACK` on `-G`, `G` being grounded.
        "26LS31" => p(16, &[2, 3, 5, 6, 10, 11, 13, 14], TriState, Datasheet),
        // Am26LS33 (`am26ls33.pdf`), the matching quadruple differential
        // receiver on the same page at A01: `1B` 1, `1A` 2, `1Y` 3, `G` 4,
        // `2Y` 5, `2A` 6, `2B` 7, `3B` 9, `3A` 10, `3Y` 11, `-G` 12,
        // `4Y` 13, `4B` 14, `4A` 15 --- `A` the non-inverting input of each
        // pair and `B` the inverting one. The first receiver is the odd one
        // out: its inverting input is the lower pin number, where the other
        // three have it on the higher. It takes the receive pair on 1 and 2
        // and the interference pair on 6 and 7, with both enables tied on.
        "26LS33" => p(16, &[3, 5, 11, 13], TriState, Datasheet),
        // Datasheet `sn74257.pdf`, quadruple two-to-one selector with
        // three-state outputs, 16-pin: A/B=1, 1A=2, 1B=3, 1Y=4, 2A=5,
        // 2B=6, 2Y=7, 3Y=9, 3B=10, 3A=11, 4Y=12, 4B=13, 4A=14, G=15 ---
        // the same pins as the inverting 74S258 above. NRACOL 0F09 selects
        // `SHF 3..0` or `SHF 7..4` onto `COLOR 7..4`.
        "74S257" => p(16, &[4, 7, 9, 12], TriState, Datasheet),
        // Datasheet `sn74ls377.pdf`, octal D flip-flop with enable, 20-pin:
        // G=1, 1Q=2, 1D=3, 2D=4, 2Q=5, 3Q=6, 3D=7, 4D=8, 4Q=9, CLK=11,
        // 5Q=12, 5D=13, 6D=14, 6Q=15, 7Q=16, 7D=17, 8D=18, 8Q=19 --- the
        // 74S374's output pins with a clock enable in place of the output
        // enable, so these are totem-pole and not three-state. NSYADR 0C07
        // holds `SYNC BEG 11..8` from `SYNC ADR 11..8`.
        "74LS377" => p(20, &[2, 5, 6, 9, 12, 15, 16, 19], Totem, Datasheet),
        // Datasheet `sn74s299.pdf`, eight-bit universal shift/storage
        // register, 20-pin: S0=1, G1=2, G2=3, and the eight multiplexed
        // input/output ports on 4, 5, 6, 7, 13, 14, 15, 16, with QA'=8,
        // CLR=9, SR=11, CLK=12, QH'=17, SL=18, S1=19.
        //
        // NRASHF's eight are the 64-bit video shift register: G1 and G2 are
        // tied high, so the eight ports never drive and only QH' --- `SHF
        // 7` on 0E16 --- carries anything out. QA' and QH' are totem-pole
        // on the datasheet and three-state here because a [`Pinout`] has
        // one drive class; the two-drivers test is that much weaker on
        // those two pins alone.
        "74S299" => p(20, &[4, 5, 6, 7, 8, 13, 14, 15, 16, 17], TriState, Datasheet),

        // A TTL oscillator can, output on pin 8, as the memory board's
        // `DIPOSC` is. NECCLK 0C08 drives the TTL-to-MECL translator with
        // it. Its frequency is on no drawing we have.
        "TTLOSC" => p(0, &[8], Totem, Family),

        // The laminate strips of `cadrtv/lmtv.wwinfo`, which carry -5.2V and
        // VCC along a row of MECL sockets: ECLCAP draws one at each of the
        // fifteen locations in the decommit area, pin 1 on the -5.2V bar and
        // pin 2 on VCC. It is a note about where power goes, not a part, and
        // it joins nothing --- the two pins are on the two rails and stay
        // there.
        "BUSBAR" => p(0, &[], Passive, Family),

        // --- MECL 10K, every one off its datasheet. A MECL
        // output is an open emitter, [`Drive::OpenEmitter`]: it can only
        // pull *high*, and the SIP terminators pull the line down. Two may
        // share a net --- ECLVID 0F04 ties pins 3 and 14 together, two NOR
        // outputs wire-ORed --- which is why they are not totem-pole here.
        // Supplies on the pure-MECL parts are VCC1 on 1, VCC2 on 16, VEE
        // (-5.2 V) on 8; the two translators straddle both logic families
        // and put **ground on 16, +5 V on 9 and VEE on 8**. The package is
        // 0 for all of them and the supply-pin check does not apply.

        // Datasheet `mc10102.pdf`, quad two-input NOR, 16-pin. Gates: 4,5
        // to 2; 6,7 to 3; 10,11 to 14; 12,13 to 15, with that fourth gate's
        // OR output on 9 as well. ECLVID has four.
        "10102" => p(0, &[2, 3, 9, 14, 15], OpenEmitter, Datasheet),
        // Datasheet `mc10105.pdf`, triple 2-3-2 input OR/NOR: A in 4,5 to
        // OR 2 and NOR 3; B in 9,10,11 to OR 7 and NOR 6; C in 12,13 to OR
        // 14 and NOR 15. NECCLK 0F08's B gate ORs the three counter bits
        // into `-CLK CTR TC`.
        "10105" => p(0, &[2, 3, 6, 7, 14, 15], OpenEmitter, Datasheet),
        // Datasheet `mc10121.pdf`, four-wide OR-AND / OR-AND-invert: four
        // ORs on 4,5,6 / 7,9,10 / 10,11,12 / 13,14,15 --- pin 10 feeds two
        // of them --- ANDed onto 2, inverted onto 3. ECLVID 0F01 makes
        // `-8B SR SHIFT` from the clock mode and the counter.
        "10121" => p(0, &[2, 3], OpenEmitter, Datasheet),
        // Datasheet `mc10124.pdf`, quad TTL-to-MECL translator. TTL in on
        // 5 (A), 7 (B), 10 (C), 11 (D) with a common strobe on 6; MECL out
        // in pairs, true and complement: A on 2 and 4, B on 1 and 3, C on
        // 15 and 12, D on 14 and 13. Which of each pair is true is read
        // off the sheet's logic diagram and confirmed by NECCLK 0F07,
        // where `CLOCK MODE 0` in on 5 comes out as `MECL CLOCK MODE 0` on
        // 2 and `-MECL CLOCK MODE 0` on 4. Ground 16, VCC 9, VEE 8.
        "10124" => p(0, &[1, 2, 3, 4, 12, 13, 14, 15], OpenEmitter, Datasheet),
        // Datasheet `mc10125.pdf`, quad MECL-to-TTL translator with
        // differential inputs and **totem-pole TTL outputs**: A on 2 (bar)
        // and 3 to 4, B on 6 (bar) and 7 to 5, C on 10 (bar) and 11 to 12,
        // D on 14 (bar) and 15 to 13; VBB, the MECL threshold, out on 1
        // for single-ended use. Ground 16, VCC 9, VEE 8. NECCLK 0E02 ties
        // 1 to 11 and 15 and receives on the barred inputs: `CLK CTR 1` in
        // on 10 is `16 MHZ CLK0` out on 12, the clock the board runs on.
        "10125" => p(0, &[1, 4, 5, 12, 13], Totem, Datasheet),
        // Datasheet `mc10136.pdf`, universal hexadecimal counter: D0..D3
        // in on 12, 11, 6, 5; S1 on 9, S2 on 7, `-Cin` on 10, clock on
        // 13; Q0..Q3 out on 14, 15, 2, 3 and `-Cout` on 4. NECCLK 0F02 is
        // the dot-clock divider.
        "10136" => p(0, &[2, 3, 4, 14, 15], OpenEmitter, Datasheet),
        // Datasheet `mc10141.pdf`, four-bit universal shift register: D0..D3
        // in on 12, 11, 9, 6; S1 on 10, S2 on 7; DR on 5, DL on 13; clock
        // on 4; Q0..Q3 out on 14, 15, 2, 3. ECLVID 0E03 and 0E04 are the
        // eight-bit video shift register.
        "10141" => p(0, &[2, 3, 14, 15], OpenEmitter, Datasheet),
        // Datasheet `mc10212.pdf`, dual three-input OR/NOR with three
        // outputs a gate: A in on 5,6,7, OR out on 2, NOR out on 3 and 4;
        // B in on 9,10,11, OR out on 12, NOR out on 13 and 14. VCC1 is
        // pins 1 and 15. ECLVID has one.
        "10212" => p(0, &[2, 3, 4, 12, 13, 14], OpenEmitter, Datasheet),

        // The MECL Thevenin terminator of NECSIP: 121 ohms to VCC and 195 to
        // -5.2V on each of twelve lines, pins 2-7 and 10-15. That divider
        // holds an undriven line near -2V, below the MECL low threshold, so
        // it is a **pull-down**: the resistor class, offering a low, which
        // any open-emitter output on the line overrides by pulling high.
        "SIP-R121/195-8" => p(0, &[2, 3, 4, 5, 6, 7, 10, 11, 12, 13, 14, 15], PullUp, Family),

        // --- The disk controller's own parts -----------------------------
        // Everything below is used only by data/CADRDC.netlist.

        // Datasheet `sn7421.pdf`, dual four-input AND, 14-pin: 1A=1, 1B=2,
        // 1C=4, 1D=5, 1Y=6, 2Y=8, 2A=9, 2B=10, 2C=12, 2D=13, with 3 and 11
        // unconnected. DCUC 0E06 ANDs `-MWD FULL`, `-DATA ACK`, `RFORA` and
        // `RFORB` into `MWD_FIFO` on pin 6.
        "74S21" => p(14, &[6, 8], Totem, Datasheet),

        // Datasheet `am25ls2536.pdf`, eight-bit decoder with control
        // storage, 20-pin: CP=1, CLR=2, CE=3, A=4, B=5, C=6, POL=7, OE=8,
        // Y0=9, GND=10, Y1..Y7 on 11..17, G2=18, G1=19, VCC=20. The address,
        // the polarity and the two enables are latched; the outputs are
        // three-state. DCHDCM 0A23 decodes `RSH7:6` into `INC BLOCK^`, `INC
        // HEAD^`, `INC CYL^` and `-END OF DISK`.
        "25LS2536" => p(20, &[9, 11, 12, 13, 14, 15, 16, 17], TriState, Datasheet),

        // Datasheet `67401.pdf`, 64-by-4 first-in first-out memory, 16-pin:
        // NC=1, INPUT READY=2, SHIFT IN=3, D0..D3 on 4..7, GND=8, MASTER
        // RESET=9, O3..O0 on 10..13, OUTPUT READY=14, SHIFT OUT=15, VCC=16.
        // The two ready flags and the four data pins are the outputs. DCRBUF
        // and DCWBUF stack these into the read and write buffers.
        "67401" => p(16, &[2, 10, 11, 12, 13, 14], Totem, Datasheet),

        // Datasheet `sn75107.pdf`, dual line receiver with totem-pole
        // outputs, 14-pin: 1A=1, 1B=2, 1Y=4, 1G=5, S=6, GND=7, 2G=8, 2Y=9,
        // 2B=11, 2A=12, and **VCC- on 13** beside VCC+ on 14. DCTRID 0A10
        // receives the Trident's data and clock pairs.
        "75107" => p(14, &[4, 9], Totem, Datasheet),
        // Datasheet `sn75110.pdf`, dual line driver with constant-current
        // differential outputs, 14-pin: 1A=1, 1B=2, 1C=3, 2C=4, 2A=5, 2B=6,
        // GND=7, 2Y=8, 2Z=9, D=10, VCC-=11, 1Z=12, 1Y=13, VCC+=14. DCTRID
        // 0A08 drives the Trident's write data pair, inhibited by pin 10
        // when the unit is not selected. The outputs are constant-current
        // **sinks** --- the sheet gives `IO(on)` at `VO = 10 V` and a
        // common-mode output range to +10 V, which only a sink holds --- and
        // they switch off, so they are open collector: on pulls the line
        // down, off leaves it to the cable's terminator.
        "75110" => p(14, &[8, 9, 12, 13], OpenCollector, Datasheet),
        // Datasheet `sn75452.pdf`, dual peripheral NAND driver, 8-pin:
        // 1A=1, 1B=2, 1Y=3, GND=4, 2Y=5, 2A=6, 2B=7, VCC=8. **Open
        // collector**, which is what lets DCTRSG's several of them share the
        // Trident's control lines.
        "75452" => p(8, &[3, 5], OpenCollector, Datasheet),

        // --- The disk controller's resistor networks.
        // A four-pin Thevenin terminator on a clock: DCCLK 0E01, 0E27 and
        // DCSH 0D14 each put one across `BIT.CLK^`, `CLK.SR^` and
        // `-BIT.CLK^`, pins 1 and 2 on the line, 3 to ground and 4 to VCC.
        // A divider between the rails settles high, so it offers a high as
        // the SIP packs do.
        "RES4" => p(0, &[1, 2], PullUp, Family),
        // The Trident cable's pull-ups, 100 ohms on each of six lines, pins
        // 2 to 7 with the common on pin 1 --- the same shape as the
        // processor's SIP packs above. The Trident's control lines are
        // active low and open-collector driven, so with no drive on the
        // cable they read high, which is deasserted.
        "SIP100-8" => p(0, &[2, 3, 4, 5, 6, 7], PullUp, Family),
        // The board's own pull-up rails: DCTRSG 0A06 holds `HI1` to `HI8`
        // up on pins 2 to 9, with pin 10 the network's ground end. The
        // engine reads a net called `HIn` as a supply in any case.
        "SIP330-10" => p(0, &[2, 3, 4, 5, 6, 7, 8, 9], PullUp, Family),
        // The I/O board's pull-up packs, `P SIP1000-10` in MIT's own parts
        // list: ten pins, the first the common tied to +5 and nine
        // resistors. Two of them sit in the 20-pin footprint at LMMYNM D11
        // and hold `MY#0..15` and `BOARD.SELECT` up against the address
        // switches.
        "P SIP1000-10" => p(0, &[2, 3, 4, 5, 6, 7, 8, 9, 10], PullUp, Family),
        // The Trident signal cable's terminator, `TRITERM` being MIT's own
        // body name for it on DCTRSG, and the one part here with no
        // datasheet at all: its resistance is **unverified**, and MIT's own
        // files do not carry it either. `cadrdc/dc.prt` gives A03 and A05 a
        // DIP type of `DUMMY` with no part number and no `VALUE:`, where a
        // few rows further down the same list reads `2DUMMY 1 VALUE:.1 UF`
        // for the capacitor at A10@02; `cadrdc/dc.wlr` marks all sixteen
        // pins `RES` and every one of them 0.00 in both loading columns; and
        // the six `TRITERM` bodies on the DM board in `cadrio/dm.stf` are
        // the same valueless `DUMMY`. What would settle it is a value off a
        // surviving board, or the Trident drive's own cable-termination
        // figure. Nothing here turns on the number: only the geometry does.
        // The geometry is not unverified, because the board's wiring
        // allows only one. Sixteen pins: the four bus-cable status lines
        // arrive on 1, 3, 5, 7 with the cable's ground returns on 2, 4, 6, 8,
        // and each line leaves **across the package**, pin n to pin 17-n ---
        // 1 to 16, 3 to 14, 5 to 12, 7 to 10 --- into the 74LS14 Schmitt
        // receivers. The far ends are jumpered in pairs on the board, 9 to
        // 10 and so on, which puts the ground return's resistor beside the
        // signal's and makes each receiver a divider.
        //
        // All eight signals crossing the two packages match their receivers
        // name for name under 17-n and none does under the alternative n+8:
        // READ.ONLY to SEL UNIT READ ONLY, DEVICE.CHECK to SEL UNIT FAULT,
        // SEEK.INC to SEL UNIT SEEK ERROR, and so on. Only the signal side
        // drives; the ground side is a resistor
        // to ground and the model has nothing for it to do.
        "TRITERM" => p(0, &[10, 12, 14, 16], PullUp, Family),

        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// What the parts compute
// ---------------------------------------------------------------------------

/// A logic level on a pin or a net.
///
/// Four values: driven low, driven high, not driven at all, and driven to no
/// value we can name.  A TTL input floats high, so [`Level::Z`] reads as one;
/// see [`Level::read`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Level {
    Low,
    High,
    /// Nothing is driving.
    #[default]
    Z,
    /// Driven, but to no value we can name.
    X,
}

impl Level {
    /// What a TTL input makes of this level; `None` is unknown.
    ///
    /// An undriven input floats high. That is the TTL input's own
    /// behaviour --- an open input draws no emitter current, and the gate
    /// sees a one --- and it is what TI's *TTL Data Book* says of unused
    /// inputs when it has them tied high rather than left to float.
    pub const fn read(self) -> Option<bool> {
        self.read_open(true)
    }

    /// What an input makes of this level when an undriven one reads as
    /// `open`: high for TTL, **low for MECL**, whose inputs carry pull-down
    /// resistors to VEE --- the MC10136 sheet's "Flip-flops will toggle when
    /// all T inputs are low" is written for open T inputs. The display
    /// board leaves the 10136's `S2` and `Cin` and one 10102 input open and
    /// relies on it.
    pub const fn read_open(self, open: bool) -> Option<bool> {
        match self {
            Level::Low => Some(false),
            Level::High => Some(true),
            Level::Z => Some(open),
            Level::X => None,
        }
    }
}

/// One output of an Am25LS2539 decoder: `i` is `-OE`, `E`, `POL`, `A`,
/// `B`, and `k` which output.
fn decode4(i: &[bool], k: u8) -> Level {
    if i[0] {
        return Level::Z;
    }
    let sel = !i[1] && (i[3] as u8 | (i[4] as u8) << 1) == k;
    lv(sel != i[2])
}

const fn lv(b: bool) -> Level {
    if b { Level::High } else { Level::Low }
}

impl From<bool> for Level {
    fn from(b: bool) -> Level {
        lv(b)
    }
}

/// A part's pins by number, pin 0 unused: room for a 28-pin package, the
/// I/O board's 2651.
pub const MAX_PINS: usize = 29;
pub type Pins = [Level; MAX_PINS];

/// What a part remembers between evaluations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// What a flip-flop, register, latch or counter holds.  Each type's
    /// entry in [`behaviour`] says which bit is which; bit 0 is its first
    /// output.
    pub bits: u16,
    /// A memory array, one entry per address: RAM contents, or PROM contents
    /// loaded from outside.  Empty for everything else.
    pub cells: Vec<u8>,
}

/// The function behind a [`Gate`]: the levels on the pins it reads, and what
/// the part holds.
pub type GateFn = fn(&[bool], &State) -> Level;

/// How a part's state changes.  Called with the pin levels now and at the
/// last settled point before this clock transition, and with what the part
/// holds.
///
/// **What an edge captures is `prev`, not `now`.** A register takes the data
/// that satisfied its setup time --- the value on its inputs *before* the
/// edge --- and everything the edge itself goes on to change arrives too
/// late. Asynchronous clears and presets are level-sensitive and read `now`,
/// as does a transparent latch, which is following its input rather than
/// sampling it.
///
/// This engine has no gate delays, so the distinction is not academic: a
/// clock edge and everything it causes land in the same instant. The MD
/// register is the case that showed it. `MDCLK` rises when `-CLK2C` falls at
/// the start of a microcycle, and the same edge switches `MDSELA`, which
/// selects `OB` or the memory bus onto the register's inputs at MDS 1A28 and
/// its fellows. Reading `now`, the register captured the bus that had just
/// been switched *away* from and `MD` never loaded at all. In the hardware
/// the multiplexer is several gates behind the clock and the register sees
/// `OB`, which is what `prev` gives.
pub type Update = fn(&Pins, &Pins, &mut State);

/// What one output pin computes, and which input pins it reads.
///
/// Dependencies are recorded per output pin rather than per package because
/// that is what makes the board levelizable: taken package-wide they close
/// four loops the hardware does not have.
pub struct Gate {
    /// The pin driven.
    pub out: u8,
    /// The pins read, in the order `f` takes them.
    pub ins: &'static [u8],
    /// The *logic* level driven.  An open-collector output computing a one
    /// drives nothing, but that is [`Drive`]'s business, not this one's.
    pub f: GateFn,
    /// What an undriven input reads as: high for a TTL gate, low for a MECL
    /// one. See [`Level::read_open`].
    pub open: bool,
}

impl Gate {
    /// Evaluates the gate.
    ///
    /// An unknown on a pin it reads makes the output unknown **where the
    /// output depends on it**, and nowhere else: the gate is evaluated
    /// with its unknown inputs every way they could be, and where every
    /// way agrees, that is the answer. A NAND with one input unknown and
    /// the other high is unknown; with the other low it is high, as the
    /// part is. This matters where an unknown is real and permanent ---
    /// the disk controller's SN75107 reads its clock pair as unknown
    /// while nothing is on the cable, and the 74S51 that makes the
    /// sequencer's clock reads that on one leg and the 2 us clock on the
    /// other. The leg with the unknown is gated off by a low, so the
    /// part clocks; taking any unknown as absorbing left the controller
    /// unable to run a command until a drive was talking, which the
    /// hardware does not need.
    ///
    /// More than [`Self::MAX_UNKNOWN`] unknown inputs is unknown without
    /// trying: that is a gate reading a bus nothing has settled, and the
    /// answer would be unknown anyway.
    pub fn eval(&self, pins: &Pins, state: &State) -> Level {
        // Seventeen is the most any gate here reads: the Am25LS2521's
        // -EOUT takes eight pairs and the enable. The 74S181's F outputs
        // take fourteen, four A, four B, four S, the mode and the carry.
        let mut vals = [false; 17];
        for (slot, &pin) in vals.iter_mut().zip(self.ins) {
            match pins[pin as usize].read_open(self.open) {
                Some(b) => *slot = b,
                None => return self.eval_with_unknowns(pins, state),
            }
        }
        (self.f)(&vals[..self.ins.len()], state)
    }

    /// How many unknown inputs [`Gate::eval`] will try every way.
    pub const MAX_UNKNOWN: usize = 6;

    /// The slow path of [`Gate::eval`]: every assignment of the unknown
    /// inputs, and the one answer they share, if they share one.
    #[cold]
    fn eval_with_unknowns(&self, pins: &Pins, state: &State) -> Level {
        let mut vals = [false; 17];
        let mut unknown = [0usize; Self::MAX_UNKNOWN];
        let mut n = 0;
        for (k, &pin) in self.ins.iter().enumerate() {
            match pins[pin as usize].read_open(self.open) {
                Some(b) => vals[k] = b,
                None => {
                    if n == Self::MAX_UNKNOWN {
                        return Level::X;
                    }
                    unknown[n] = k;
                    n += 1;
                }
            }
        }
        let mut answer = None;
        for way in 0..1u32 << n {
            for (j, &k) in unknown[..n].iter().enumerate() {
                vals[k] = way >> j & 1 != 0;
            }
            let level = (self.f)(&vals[..self.ins.len()], state);
            match answer {
                None => answer = Some(level),
                Some(so_far) if so_far == level => {}
                Some(_) => return Level::X,
            }
        }
        answer.unwrap_or(Level::X)
    }
}

/// What one pin puts on a net.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Driver {
    /// The logic level the part computes.
    pub level: Level,
    /// How the pin drives it.
    pub drive: Drive,
}

/// Resolves a net from everything driving it.
///
/// Four values come out, and each means something different: [`Level::Low`]
/// and [`Level::High`] are settled, [`Level::Z`] is a net nothing is holding,
/// and [`Level::X`] is a net whose value cannot be named --- either because a
/// driver is unknown, or because two of them disagree.
///
/// The rules, in the order they apply:
///
/// - An unknown from any driver makes the net unknown, a resistor's
///   included. There is no level a part could be driving that would make
///   the answer safe.
/// - A high-impedance output is not driving, and a passive pin never was.
/// - An open-collector output can only pull down. Driving a one, it lets go,
///   which is what lets eight 74S181s share `AEQM` and what makes the M
///   memory's outputs meet the RES20 packs in MCTL.
/// - A three-state output driving [`Level::Z`] is off and contributes
///   nothing.
/// - Two strong drivers at different levels are a fault, not a value, so the
///   net comes out unknown. `tests/part.rs` shows the board has exactly one
///   place this can happen, `-RESET`, and that it is deliberate.
/// - A resistor decides the net only when nothing stronger is holding it,
///   however many resistors there are and whichever way they pull. Two
///   pulling opposite ways with nothing stronger on the net are a divider,
///   not a level: unknown.
/// - With nothing at all on the net the answer is [`Level::Z`], which a TTL
///   input then reads as a one; see [`Level::read`].
pub fn resolve(drivers: &[Driver]) -> Level {
    let mut strong: Option<bool> = None;
    // Whether any resistor pulls up, and whether any pulls down.
    let (mut up, mut down) = (false, false);
    for d in drivers {
        let level = match (d.drive, d.level) {
            (Drive::Passive, _) => continue,
            // A resistor: weak, toward the level it drives. One whose other
            // end is off drives nothing; one whose other end is unknown is
            // an unknown driver like any other.
            (Drive::PullUp, Level::Z) => continue,
            (Drive::PullUp, Level::X) => return Level::X,
            (Drive::PullUp, level) => {
                if level == Level::High {
                    up = true;
                } else {
                    down = true;
                }
                continue;
            }
            // High impedance is not driving, however it arises.
            (_, Level::Z) => continue,
            // An open-collector output computing a one lets go of the net,
            // and an open-emitter one computing a zero.
            (Drive::OpenCollector, Level::High) => continue,
            (Drive::OpenEmitter, Level::Low) => continue,
            (_, Level::X) => return Level::X,
            (_, level) => level,
        };
        let high = level == Level::High;
        match strong {
            Some(other) if other != high => return Level::X,
            _ => strong = Some(high),
        }
    }
    match (strong, up, down) {
        (Some(true), ..) => Level::High,
        (Some(false), ..) => Level::Low,
        (None, true, true) => Level::X,
        (None, true, false) => Level::High,
        (None, false, true) => Level::Low,
        (None, false, false) => Level::Z,
    }
}

/// What a part type computes and how it remembers.
#[derive(Clone, Copy)]
pub struct Behaviour {
    /// One entry per output pin.
    pub gates: &'static [Gate],
    /// `None` for a part that holds nothing.
    pub update: Option<Update>,
    /// The pins `update` reads: clocks, data, clears and enables.  Empty
    /// when there is no update.  Together with the gates' `ins` and `out`
    /// this accounts for every pin of the package, which
    /// `tests/behaviour.rs` checks against the netlist.
    pub update_ins: &'static [u8],
}

/// A part with no state.  The gate lists at the call sites are inline
/// `const` blocks, which is what gives them a `'static` lifetime; written as
/// plain array literals they would be temporaries.
const fn comb(gates: &'static [Gate]) -> Behaviour {
    Behaviour { gates, update: None, update_ins: &[] }
}

/// A part that remembers something.
const fn seq(gates: &'static [Gate], update_ins: &'static [u8], update: Update) -> Behaviour {
    Behaviour { gates, update: Some(update), update_ins }
}

const fn g(out: u8, ins: &'static [u8], f: GateFn) -> Gate {
    Gate { out, ins, f, open: true }
}

/// A MECL gate: an undriven input reads low.
const fn gm(out: u8, ins: &'static [u8], f: GateFn) -> Gate {
    Gate { out, ins, f, open: false }
}

// The gate functions shared across the small-scale types.  Arity is whatever
// the `ins` list says.
const NAND: GateFn = |i, _| lv(!i.iter().all(|&b| b));
const AND: GateFn = |i, _| lv(i.iter().all(|&b| b));
const NOR: GateFn = |i, _| lv(!i.iter().any(|&b| b));
const OR: GateFn = |i, _| lv(i.iter().any(|&b| b));
const XOR: GateFn = |i, _| lv(i.iter().fold(false, |a, &b| a ^ b));
/// A resistor pack's pin: always offering a high.
const HIGH: GateFn = |_, _| Level::High;
/// A terminator's pin: always offering a low.
const LOW: GateFn = |_, _| Level::Low;
const XNOR: GateFn = |i, _| lv(!i.iter().fold(false, |a, &b| a ^ b));
const NOT: GateFn = |i, _| lv(!i[0]);
/// The 67401's data outputs depend on no pin, only on what the queue holds,
/// but they change when a word is shifted in or out or the part is reset, so
/// those three pins are what they are ordered behind.
const FIFO_INS: &[u8] = &[3, 15, 9];

/// How many nibbles the 67401's queue holds, and the front one.
fn fifo_count(s: &State) -> usize {
    (s.bits & 0x7f) as usize
}

fn fifo_front(s: &State) -> u8 {
    if fifo_count(s) > 0 {
        s.cells.first().copied().unwrap_or(0)
    } else {
        (s.bits >> 7) as u8 & 0xf
    }
}

/// A resistor passing what is on the other end of it: TRITERM's four lines.
const PASS: GateFn = |i, _| lv(i[0]);
/// One channel of an SN75107 line receiver.  Inputs in the order `A`, `B`,
/// the channel's own strobe `G` and the package's common strobe `S`.
///
/// `sn75107.pdf` page 2: the output is low **only** where `A` is below `B`
/// and both strobes are high; every other row of the table is a high. The
/// row the sheet calls indeterminate is where the two inputs are within
/// 25 mV of each other with the strobes up, which digitally is the two
/// reading alike, and that is an `X` rather than a guess.
const R75107: GateFn = |i, _| match (i[0], i[1], i[2] && i[3]) {
    (true, false, _) => Level::High,
    (false, true, true) => Level::Low,
    (false, true, false) => Level::High,
    (_, _, true) => Level::X,
    _ => Level::High,
};
/// The `Y` output of one SN75110 driver channel, and [`Z75110`] its `Z`.
/// Inputs in the order `A`, `B`, the channel's enable `C` and the package's
/// common inhibitor `D`.
///
/// `sn75110.pdf` page 2: nothing is on unless `C` and `D` are both high, and
/// then `Z` sinks when `A` and `B` are both high and `Y` sinks otherwise.
/// The outputs are open collector here, so the level returned is the logic
/// one and a **one is off**; [`Drive::OpenCollector`] does the rest.
const Y75110: GateFn = |i, _| lv(!(i[2] && i[3] && !(i[0] && i[1])));
const Z75110: GateFn = |i, _| lv(!(i[0] && i[1] && i[2] && i[3]));
/// Two-wide AND-OR-invert, as the 74S51 has twice over.
const AOI22: GateFn = |i, _| lv(!((i[0] && i[1]) || (i[2] && i[3])));
/// Output `k` of an Am25LS2536: `i` is `-G1`, `G2`, `-OE`; the state's
/// low three bits are the select and bit 3 the polarity.
fn decode2536(i: &[bool], s: &State, k: u16) -> Level {
    if i[2] {
        return Level::Z;
    }
    let g = !i[0] && i[1];
    let selected = s.bits & 7 == k;
    let pol = s.bits >> 3 & 1 != 0;
    lv(!(g && selected) ^ pol)
}
/// The MC10121's four-wide OR-AND. Inputs in pin order 4,5,6,7,9,10,11,
/// 12,13,14,15; pin 10, at index 5, is in the second OR and the third.
fn or_and4(i: &[bool]) -> bool {
    (i[0] || i[1] || i[2])
        && (i[3] || i[4] || i[5])
        && (i[5] || i[6] || i[7])
        && (i[8] || i[9] || i[10])
}
/// One receiver of an MC10125, used single-ended: inputs are the barred
/// pin, then the true one; the output follows the true input against the
/// barred one, which for the board's wiring is the barred input inverted.
const RECEIVER: GateFn = |i, _| lv(i[1] && !i[0]);

// --- reading pins ---------------------------------------------------------

/// The value a TTL input takes from a pin.  An undriven pin floats high; so,
/// here, does an unknown one --- a driver conflict, or a receiver whose cable
/// nothing drives --- which a running board can produce.  The convention has
/// a consequence for [`rose`] and [`fell`]: a clock that goes from low to
/// unknown is taken as having risen, and one from unknown to high as not.
/// Which the part would do is not something a level model can say, and
/// nothing here resolves it.
fn v(p: &Pins, pin: u8) -> bool {
    p[pin as usize].read().unwrap_or(true)
}

/// The value a MECL input takes from a pin: an undriven one reads low.
fn vm(p: &Pins, pin: u8) -> bool {
    p[pin as usize].read_open(false).unwrap_or(false)
}

/// A low-to-high transition on `pin` between the two evaluations.
fn rose(now: &Pins, prev: &Pins, pin: u8) -> bool {
    !v(prev, pin) && v(now, pin)
}

/// A high-to-low transition on `pin` between the two evaluations.
fn fell(now: &Pins, prev: &Pins, pin: u8) -> bool {
    v(prev, pin) && !v(now, pin)
}

/// Gathers pins into a number, the first being bit 0.
fn word(p: &Pins, pins: &[u8]) -> u32 {
    pins.iter().enumerate().fold(0, |w, (i, &pin)| w | (v(p, pin) as u32) << i)
}

/// The same, read as MECL inputs.
fn word_m(p: &Pins, pins: &[u8]) -> u32 {
    pins.iter().enumerate().fold(0, |w, (i, &pin)| w | (vm(p, pin) as u32) << i)
}

/// Gathers four values out of a gate's inputs, the first being bit 0.
fn nib(i: &[bool], at: usize) -> u8 {
    (i[at] as u8) | (i[at + 1] as u8) << 1 | (i[at + 2] as u8) << 2 | (i[at + 3] as u8) << 3
}

fn bit(bits: u16, n: u32) -> bool {
    bits >> n & 1 != 0
}

fn put(bits: &mut u16, n: u32, b: bool) {
    *bits = (*bits & !(1 << n)) | (b as u16) << n;
}

// ---------------------------------------------------------------------------
// The 2651
// ---------------------------------------------------------------------------

use crate::serial::command as serial_cr;

/// What the 2651 holds, by cell: the registers of the sheet's block
/// diagram and the counters behind them. The flags are in `State::bits`,
/// `PCI_THR_FULL` and the rest.
pub const PCI_CELLS: usize = 17;
const PCI_MR1: usize = 0;
const PCI_MR2: usize = 1;
const PCI_CR: usize = 2;
/// The transmit holding register.
const PCI_THR: usize = 3;
/// The receive holding register.
const PCI_RHR: usize = 4;
/// SYN1, SYN2 and DLE, in that order.
const PCI_SYN: usize = 5;
/// The transmit shift register: the character going out.
const PCI_TXSR: usize = 8;
/// The receive shift register: the bits so far, least significant first.
const PCI_RXSR: usize = 9;
/// 16X clocks into the frame going out, low byte then high.
const PCI_TXT: usize = 10;
/// 16X clocks since the start bit of the frame coming in.
const PCI_RXT: usize = 12;
/// The baud-rate generator's count of `BRCLK` periods.
const PCI_DIV: usize = 14;
/// Bit 0: the next mode access is register 2; bits 1-2: which of SYN1,
/// SYN2, DLE the next write to the status address loads.
const PCI_PTR: usize = 16;

/// The flags, as bits of `State::bits`.
const PCI_THR_FULL: u32 = 0;
const PCI_RX_READY: u32 = 1;
/// `TxEMT`: the shift register ran out with the holding register empty.
const PCI_TX_EMPTY: u32 = 2;
/// `DSCHG`: `-DSR` or `-DCD` moved since the status register was read.
const PCI_DSCHG: u32 = 3;
const PCI_OVERRUN: u32 = 4;
const PCI_PARITY_ERROR: u32 = 5;
const PCI_FRAMING_ERROR: u32 = 6;
/// A frame is going out.
const PCI_TX_ACTIVE: u32 = 7;
/// A frame is coming in.
const PCI_RX_ACTIVE: u32 = 8;
/// After a stop bit that was low, the receiver waits for the line to
/// come back up before it hunts for a start bit again.
const PCI_RX_WAIT_HIGH: u32 = 9;
/// The received parity bit, until the stop bit is in.
const PCI_RX_PARITY: u32 = 10;
/// `-DSR` and `-DCD` as last seen, asserted or not.
const PCI_DSR_WAS: u32 = 11;
const PCI_DCD_WAS: u32 = 12;

/// The data bus, `D0`..`D7`.
const PCI_D: &[u8] = &[27, 28, 1, 2, 5, 6, 7, 8];
/// What a data output reads: `-CE`, `R/-W`, `A1`, `A0`, `-DCD`, `-DSR`.
const PCI_BUS_IN: &[u8] = &[11, 13, 10, 12, 16, 22];

fn pci_cell(s: &State, k: usize) -> u8 {
    s.cells.get(k).copied().unwrap_or(0)
}

fn pci_cr(s: &State) -> u8 {
    pci_cell(s, PCI_CR)
}

fn pci_mode(s: &State) -> u8 {
    pci_cr(s) & serial_cr::MODE_MASK
}

fn pci_local_loop(s: &State) -> bool {
    pci_mode(s) == serial_cr::LOCAL_LOOP_BACK
}

/// Whether the transmitter takes the CPU's characters: enabled, and in
/// normal operation or local loop back --- auto echo and remote loop back
/// cut "the CPU to transmitter link".
fn pci_tx_on(s: &State) -> bool {
    pci_cr(s) & serial_cr::TX_ENABLE != 0
        && matches!(pci_mode(s), serial_cr::NORMAL | serial_cr::LOCAL_LOOP_BACK)
        && pci_cell(s, PCI_MR2) & crate::serial::mode2::TX_INTERNAL != 0
        && pci_cell(s, PCI_MR1) & crate::serial::mode1::RATE_MASK
            != crate::serial::mode1::SYNCHRONOUS
}

/// Whether the receiver runs: enabled, or in local loop back, where
/// "CR2 (RxEN) is ignored".
fn pci_rx_on(s: &State) -> bool {
    (pci_cr(s) & serial_cr::RX_ENABLE != 0 || pci_local_loop(s))
        && pci_cell(s, PCI_MR2) & crate::serial::mode2::RX_INTERNAL != 0
        && pci_cell(s, PCI_MR1) & crate::serial::mode1::RATE_MASK
            != crate::serial::mode1::SYNCHRONOUS
}

/// `SR0`: the holding register empty with the transmitter on. "It is not
/// set when the Automatic Echo or Remote Loop Back modes are programmed."
fn pci_tx_ready(s: &State) -> bool {
    pci_tx_on(s) && !bit(s.bits, PCI_THR_FULL)
}

/// `-RxRDY`, asserted or not: held off in remote loop back, where "the
/// RxRDY, TxRDY and TxEMT/DSCHG outputs are held high".
fn pci_rx_ready_pin(s: &State) -> bool {
    bit(s.bits, PCI_RX_READY) && pci_mode(s) != serial_cr::REMOTE_LOOP_BACK
}

/// `SR2`: `TxEMT`, which is only "with the transmitter enabled", or
/// `DSCHG`.
fn pci_sr2(s: &State) -> bool {
    (pci_cr(s) & serial_cr::TX_ENABLE != 0 && bit(s.bits, PCI_TX_EMPTY)) || bit(s.bits, PCI_DSCHG)
}

/// `-TxEMT/DSCHG`, asserted or not: in auto echo "will reflect only the
/// data set change condition", and held off in remote loop back.
fn pci_tx_empt_pin(s: &State) -> bool {
    match pci_mode(s) {
        serial_cr::AUTO_ECHO => bit(s.bits, PCI_DSCHG),
        serial_cr::REMOTE_LOOP_BACK => false,
        _ => pci_sr2(s),
    }
}

/// The status register, Table 8, with `-DCD` and `-DSR` as the pins hold
/// them, asserted or not --- but in local loop back `-DTR` stands in for
/// `-DCD`, as the sheet connects them; whether `SR6` shows the pin or the
/// connection then is **unverified**, the sheet saying only that the
/// inputs are ignored.
fn pci_status(s: &State, dcd: bool, dsr: bool) -> u8 {
    use crate::serial::status as sr;
    let dcd = if pci_local_loop(s) { pci_cr(s) & serial_cr::DTR != 0 } else { dcd };
    let flag = |on: bool, mask: u8| if on { mask } else { 0 };
    flag(pci_tx_ready(s), sr::TX_READY)
        | flag(bit(s.bits, PCI_RX_READY), sr::RX_READY)
        | flag(pci_sr2(s), sr::TX_EMPTY_OR_DSCHG)
        | flag(bit(s.bits, PCI_PARITY_ERROR), sr::PARITY_ERROR)
        | flag(bit(s.bits, PCI_OVERRUN), sr::OVERRUN)
        | flag(bit(s.bits, PCI_FRAMING_ERROR), sr::FRAMING_ERROR)
        | flag(dcd, sr::DCD)
        | flag(dsr, sr::DSR)
}

/// The register `A1`, `A0` select for a read, Table 4.
fn pci_selected(s: &State, a1: bool, a0: bool, dcd: bool, dsr: bool) -> u8 {
    match (a1, a0) {
        (false, false) => pci_cell(s, PCI_RHR),
        (false, true) => pci_status(s, dcd, dsr),
        (true, false) => pci_cell(s, if pci_cell(s, PCI_PTR) & 1 != 0 { PCI_MR2 } else { PCI_MR1 }),
        (true, true) => pci_cr(s),
    }
}

/// Bit `k` of the data bus: the selected register while `-CE` is low for
/// a read, three-state otherwise. `i` is [`PCI_BUS_IN`].
fn pci_data(i: &[bool], s: &State, k: u32) -> Level {
    if i[0] || i[1] {
        return Level::Z;
    }
    lv(pci_selected(s, i[2], i[3], !i[4], !i[5]) >> k & 1 != 0)
}

/// The frame going out, and where in it the transmitter is: the level on
/// `TxD` at 16X clock `t` of a frame of `f` carrying `byte`, or none past
/// the end. A start bit, the data least significant first, the parity
/// bit, and the stop bits, one and a half of them being 24 clocks.
fn pci_tx_bit(f: crate::serial::Framing, byte: u8, t: u16) -> Option<bool> {
    let k = t / 16;
    let data = f.bits as u16;
    let parity = f.parity.is_some() as u16;
    if k == 0 {
        Some(false)
    } else if k <= data {
        Some(byte >> (k - 1) & 1 != 0)
    } else if k <= data + parity {
        f.parity_bit(byte)
    } else if t < 16 * (1 + data + parity) + 8 * f.stop_halves as u16 {
        Some(true)
    } else {
        None
    }
}

fn pci_u16(s: &State, k: usize) -> u16 {
    u16::from_le_bytes([pci_cell(s, k), pci_cell(s, k + 1)])
}

fn pci_put_u16(s: &mut State, k: usize, v: u16) {
    let [lo, hi] = v.to_le_bytes();
    s.cells[k] = lo;
    s.cells[k + 1] = hi;
}

/// What the transmitter has on `TxD` now: a mark unless a frame is going
/// out, or a break is forced.
fn pci_tx_level(s: &State) -> bool {
    if pci_cr(s) & serial_cr::BREAK != 0 {
        return false;
    }
    if !bit(s.bits, PCI_TX_ACTIVE) {
        return true;
    }
    let f = crate::serial::Framing::of(pci_cell(s, PCI_MR1));
    pci_tx_bit(f, pci_cell(s, PCI_TXSR), pci_u16(s, PCI_TXT)).unwrap_or(true)
}

/// `TxD` (19): the transmitter's, or in auto echo and remote loop back
/// the received data "automatically directed to the TxD line" --- the
/// `RxD` pin as it stands, `i[0]`, where the chip retimes it to its
/// clock --- and in local loop back held high.
const PCI_TXD: GateFn = |i, s| match pci_mode(s) {
    serial_cr::AUTO_ECHO | serial_cr::REMOTE_LOOP_BACK => lv(i[0]),
    serial_cr::LOCAL_LOOP_BACK => Level::High,
    _ => lv(pci_tx_level(s)),
};

/// The 2651 between two settled points: `RESET` as a level, the end of a
/// bus access on the rise of `-CE`, a change on `-DSR` or `-DCD`, and
/// one count of the baud-rate generator on the rise of `BRCLK`, which
/// every `divisor` rises is a 16X clock for the transmitter and the
/// receiver.
///
/// The transmitter moves `TxD` every sixteen clocks and loads the next
/// character when the frame is out, or goes empty; the sheet has `TxEMT`
/// rising "at the beginning of the last data bit" when nothing is
/// waiting, which is up to two and a half bits earlier than here. The
/// receiver looks for a low on `RxD` at every clock, samples eight clocks
/// later to confirm the start bit and every sixteen from there for the
/// data, the parity and the stop bit, and has the character at the stop
/// bit's middle --- the sheet's description under "Receiver". A stop bit
/// found low is a framing error, and "The RxD input must return to a
/// high condition before a search for the next start bit begins".
fn pci_update(now: &Pins, prev: &Pins, st: &mut State) {
    use crate::serial::{DIVISORS, Framing, mode2};
    if st.cells.len() < PCI_CELLS {
        st.cells.resize(PCI_CELLS, 0);
    }
    let dsr = !v(now, 22);
    let dcd_pin = !v(now, 16);
    if v(now, 21) {
        st.bits = 0;
        st.cells.iter_mut().for_each(|c| *c = 0);
        put(&mut st.bits, PCI_DSR_WAS, dsr);
        put(&mut st.bits, PCI_DCD_WAS, dcd_pin);
        return;
    }
    if rose(now, prev, 11) {
        let write = v(prev, 13);
        let a1 = v(prev, 10);
        let a0 = v(prev, 12);
        let d = word(prev, PCI_D) as u8;
        let ptr = st.cells[PCI_PTR];
        match (a1, a0, write) {
            (false, false, true) => {
                st.cells[PCI_THR] = d;
                put(&mut st.bits, PCI_THR_FULL, true);
                put(&mut st.bits, PCI_TX_EMPTY, false);
            }
            (false, false, false) => put(&mut st.bits, PCI_RX_READY, false),
            (false, true, true) => {
                let k = (ptr >> 1 & 3) as usize % 3;
                st.cells[PCI_SYN + k] = d;
                st.cells[PCI_PTR] = (ptr & 1) | (((k + 1) % 3) as u8) << 1;
            }
            (false, true, false) => put(&mut st.bits, PCI_DSCHG, false),
            (true, false, write) => {
                if write {
                    st.cells[if ptr & 1 != 0 { PCI_MR2 } else { PCI_MR1 }] = d;
                }
                st.cells[PCI_PTR] = ptr ^ 1;
            }
            (true, true, true) => {
                if d & serial_cr::RESET_ERROR != 0 {
                    for flag in [PCI_OVERRUN, PCI_PARITY_ERROR, PCI_FRAMING_ERROR] {
                        put(&mut st.bits, flag, false);
                    }
                }
                st.cells[PCI_CR] = d & !serial_cr::RESET_ERROR;
                if !pci_rx_on(st) {
                    put(&mut st.bits, PCI_RX_ACTIVE, false);
                    put(&mut st.bits, PCI_RX_READY, false);
                }
                if d & serial_cr::TX_ENABLE == 0 {
                    put(&mut st.bits, PCI_TX_EMPTY, false);
                }
            }
            (true, true, false) => st.cells[PCI_PTR] = 0,
        }
    }
    if dsr != bit(st.bits, PCI_DSR_WAS) || dcd_pin != bit(st.bits, PCI_DCD_WAS) {
        put(&mut st.bits, PCI_DSCHG, true);
        put(&mut st.bits, PCI_DSR_WAS, dsr);
        put(&mut st.bits, PCI_DCD_WAS, dcd_pin);
    }
    let tx_on = pci_tx_on(st);
    let rx_on = pci_rx_on(st);
    if !(tx_on || rx_on) {
        // The generator held, at zero: it counts from the write that
        // turns either half on, as the behavioural port counts from it.
        pci_put_u16(st, PCI_DIV, 0);
        return;
    }
    if !rose(now, prev, 20) {
        return;
    }
    let divisor = DIVISORS[(st.cells[PCI_MR2] & mode2::RATE_MASK) as usize] as u16;
    let count = pci_u16(st, PCI_DIV) + 1;
    if count < divisor {
        pci_put_u16(st, PCI_DIV, count);
        return;
    }
    pci_put_u16(st, PCI_DIV, 0);
    // A 16X clock.
    let f = Framing::of(st.cells[PCI_MR1]);
    let local = pci_local_loop(st);
    let cts = if local { st.cells[PCI_CR] & serial_cr::RTS != 0 } else { !v(now, 17) };
    let dcd = if local { st.cells[PCI_CR] & serial_cr::DTR != 0 } else { dcd_pin };
    if tx_on {
        let load = |st: &mut State| {
            st.cells[PCI_TXSR] = st.cells[PCI_THR];
            put(&mut st.bits, PCI_THR_FULL, false);
            put(&mut st.bits, PCI_TX_ACTIVE, true);
            pci_put_u16(st, PCI_TXT, 0);
        };
        if bit(st.bits, PCI_TX_ACTIVE) {
            let t = pci_u16(st, PCI_TXT) + 1;
            if pci_tx_bit(f, st.cells[PCI_TXSR], t).is_some() {
                pci_put_u16(st, PCI_TXT, t);
            } else if bit(st.bits, PCI_THR_FULL) && cts {
                load(st);
            } else {
                put(&mut st.bits, PCI_TX_ACTIVE, false);
                pci_put_u16(st, PCI_TXT, 0);
                if !bit(st.bits, PCI_THR_FULL) {
                    put(&mut st.bits, PCI_TX_EMPTY, true);
                }
            }
        } else if bit(st.bits, PCI_THR_FULL) && cts {
            load(st);
        }
    }
    if rx_on && dcd {
        let rxd = if local { pci_tx_level(st) } else { v(now, 3) };
        if !bit(st.bits, PCI_RX_ACTIVE) {
            if bit(st.bits, PCI_RX_WAIT_HIGH) {
                if rxd {
                    put(&mut st.bits, PCI_RX_WAIT_HIGH, false);
                }
            } else if !rxd {
                put(&mut st.bits, PCI_RX_ACTIVE, true);
                pci_put_u16(st, PCI_RXT, 0);
                st.cells[PCI_RXSR] = 0;
            }
            return;
        }
        let t = pci_u16(st, PCI_RXT) + 1;
        pci_put_u16(st, PCI_RXT, t);
        if t % 16 != 8 {
            return;
        }
        let k = t / 16;
        let data = f.bits as u16;
        let parity = f.parity.is_some() as u16;
        if k == 0 {
            if rxd {
                put(&mut st.bits, PCI_RX_ACTIVE, false);
            }
        } else if k <= data {
            st.cells[PCI_RXSR] |= (rxd as u8) << (k - 1);
        } else if k <= data + parity {
            put(&mut st.bits, PCI_RX_PARITY, rxd);
        } else {
            put(&mut st.bits, PCI_RX_ACTIVE, false);
            if !rxd {
                put(&mut st.bits, PCI_FRAMING_ERROR, true);
                put(&mut st.bits, PCI_RX_WAIT_HIGH, true);
            }
            let byte = st.cells[PCI_RXSR] & f.mask();
            if f.parity_bit(byte).is_some_and(|p| p != bit(st.bits, PCI_RX_PARITY)) {
                put(&mut st.bits, PCI_PARITY_ERROR, true);
            }
            if pci_mode(st) != serial_cr::REMOTE_LOOP_BACK {
                if bit(st.bits, PCI_RX_READY) {
                    put(&mut st.bits, PCI_OVERRUN, true);
                }
                st.cells[PCI_RHR] = byte;
                put(&mut st.bits, PCI_RX_READY, true);
            }
        }
    }
}

// --- the medium-scale parts, as functions -------------------------------

/// One 74S138 output.  `n` picks which of the eight.
///
/// Inputs in pin order: `sel0` p1, `sel1` p2, `sel2` p3, `-g2b` p4, `-g2a`
/// p5, `g1` p6.
fn s138(i: &[bool], n: u8) -> Level {
    let enabled = i[5] && !i[4] && !i[3];
    let sel = (i[2] as u8) << 2 | (i[1] as u8) << 1 | i[0] as u8;
    lv(!(enabled && sel == n))
}

/// One half of a 74S139.  Inputs: `-e`, `sel0`, `sel1`.
fn s139(i: &[bool], n: u8) -> Level {
    let enabled = !i[0];
    let sel = (i[2] as u8) << 1 | i[1] as u8;
    lv(!(enabled && sel == n))
}

/// The 74S151 data bit selected.  Inputs: `-ce`, `sel0`, `sel1`, `sel2`, then
/// `i0`..`i7`.  With the strobe high the part reads out low, not high
/// impedance: its outputs are totem pole.
fn s151(i: &[bool]) -> bool {
    let sel = (i[3] as usize) << 2 | (i[2] as usize) << 1 | i[1] as usize;
    !i[0] && i[4 + sel]
}

/// One half of a 74S153.  Inputs: `-enb`, `sel0`, `sel1`, then `c0`..`c3`.
/// Low when disabled, again because the outputs are totem pole.
const S153: GateFn = |i, _| {
    let sel = (i[2] as usize) << 1 | i[1] as usize;
    lv(!i[0] && i[3 + sel])
};

/// One bit of a 74S157.  Inputs: `sel` p1, `-enb` p15, `a`, `b`.
///
/// The strobe forces the output *low*: the '157 has totem-pole outputs, and
/// its function table (SN74S157 datasheet) gives `L` for strobe high where
/// the three-state part with this pinout, the '257, gives `Z`.  Every '157
/// on the board has pin 15 grounded, so nothing here can tell the two apart.
const S157: GateFn = |i, _| lv(!i[1] && if i[0] { i[3] } else { i[2] });

/// One bit of a 74S158: the '157's pinout with inverted outputs and
/// totem-pole drive, so the strobe forces a high rather than turning the
/// output off. `sn74s158.pdf`'s function table: strobe high gives `H` for
/// the '158 where it gives `L` for the '157.
/// Inputs: `sel` p1, `-strobe` p15, `a`, `b`.
const S158: GateFn = |i, _| lv(i[1] || !if i[0] { i[3] } else { i[2] });

/// One driver of an Am26LS31. Inputs: `G` p4, `-G` p12, then the
/// driver's own `A`. `am26ls31.pdf`: the enable is common to all four and
/// "offers the choice of an active-high or active-low enable", so the
/// outputs drive when `G` is high or `-G` is low. `Y` follows `A` and `Z`
/// is its complement.
fn ls31(i: &[bool], invert: bool) -> Level {
    if !(i[0] || !i[1]) {
        return Level::Z;
    }
    lv(i[2] ^ invert)
}

/// One receiver of an Am26LS33, enabled the same way. The output is high
/// where `A` is above `B`; the two reading alike is no difference to
/// read, and is an `X` rather than a guess, as the SN75107's is.
/// Inputs: `G` p4, `-G` p12, then `A` and `B`.
fn ls33(i: &[bool]) -> Level {
    if !(i[0] || !i[1]) {
        return Level::Z;
    }
    match (i[2], i[3]) {
        (true, false) => Level::High,
        (false, true) => Level::Low,
        _ => Level::X,
    }
}

/// The eight polynomials a 9401 can be told to divide by, `9401.pdf`
/// Table 1, indexed by `S2 S1 S0`: the degree, and the exponents below it
/// that are fed back. `x^16 + x^15 + x^2 + 1` is CRC-16 and is what the
/// I/O board's transmitter selects, with all three select pins grounded.
const CRC9401: [(u32, u16); 8] = [
    (16, 1 << 15 | 1 << 2 | 1), // CRC-16
    (16, 1 << 14 | 1 << 1 | 1), // CRC-16 reverse
    (16, 1 << 15 | 1 << 13 | 1 << 7 | 1 << 4 | 1 << 2 | 1 << 1 | 1),
    (12, 1 << 11 | 1 << 3 | 1 << 2 | 1 << 1 | 1), // CRC-12
    (8, 1 << 7 | 1 << 5 | 1 << 4 | 1 << 1 | 1),
    (8, 1),                     // LRC-8
    (16, 1 << 12 | 1 << 5 | 1), // CRC-CCITT
    (16, 1 << 11 | 1 << 4 | 1), // CRC-CCITT reverse
];

/// One output of a 74S251: an eight-input selector with a three-state
/// output. `sn74s251.pdf`: the strobe must be low to enable it, and `Y`
/// is the selected input where `W` is its complement.
/// Inputs: `-g` p7, `a` p11, `b` p10, `c` p9, then `d0`..`d7`.
fn s251(i: &[bool], invert: bool) -> Level {
    if i[0] {
        return Level::Z;
    }
    let sel = (i[3] as usize) << 2 | (i[2] as usize) << 1 | i[1] as usize;
    lv(i[4 + sel] ^ invert)
}

/// One bit of a 74S258: inverting, and genuinely three-state.
/// Inputs: `sel` p1, `-enb` p15, `a`, `b`.
const S258: GateFn = |i, _| {
    if i[1] { Level::Z } else { lv(!if i[0] { i[3] } else { i[2] }) }
};

/// One bit of a 74S257: the '258's pinout with the '157's polarity ---
/// non-inverting, and three-state.  Inputs: `sel` p1, `-enb` p15, `a`, `b`.
const S257: GateFn = |i, _| {
    if i[1] { Level::Z } else { lv(if i[0] { i[3] } else { i[2] }) }
};

/// One half of a 74S253: the '153 with three-state outputs, so the enable
/// turns the output off rather than forcing it low.
/// Inputs: `-enb`, `sel0`, `sel1`, then `c0`..`c3`.
const S253: GateFn = |i, _| {
    if i[0] { Level::Z } else { lv(i[3 + ((i[2] as usize) << 1 | i[1] as usize)]) }
};

/// One multiplexed input/output port of a 74S299.  Both output controls
/// must be low for it to drive, and the clear shows through at once.
/// Inputs: `-g1` p2, `-g2` p3, `-clr` p9.
fn port299(i: &[bool], q: bool) -> Level {
    if i[0] || i[1] { Level::Z } else { lv(i[2] && q) }
}

/// The 74S299's eight ports, A first: `A/QA` p7 through `H/QH` p16.
const PORTS299: &[u8] = &[7, 13, 6, 14, 5, 15, 4, 16];

/// The 74S283 sum.  Inputs: `a0`..`a3`, `b0`..`b3`, `ci`.  Returns five bits,
/// the carry out on top.
fn s283(i: &[bool]) -> u8 {
    nib(i, 0) + nib(i, 4) + i[8] as u8
}

/// The 74S181, gate for gate.
///
/// Read off the logic diagram on page 5 of `sn74181.pdf` --- TI SDLS136,
/// December 1972, revised March 1988 --- which draws the part as gates with
/// the pin numbers against them.
///
/// Each bit feeds two active-low terms into the carry network:
///
/// ```text
/// -P[i] = NOR(A[i] . B[i] . S3,  A[i] . -B[i] . S2)     propagate
/// -G[i] = NOR(A[i],  -B[i] . S1,  B[i] . S0)            generate
/// ```
///
/// `m` is high for logic and low for arithmetic; `cnb` and the returned
/// `cn4b` are the active-low carries, which is what the active-high data
/// convention gives you.  Returns `(f, cn4b, aeb, x, y)` for pins 9-13, 16,
/// 14, 15 and 17; `x` and `y` are the sheet's `P or X` and `G or Y`.
///
/// `x` and `y` depend on neither `m` nor `cnb`, and `cn4b` does not depend on
/// `m`.  That is the fact that keeps the carry-lookahead out of a loop;
/// `tests/behaviour.rs` checks it exhaustively
/// against [`crate::ttl::alu`] --- the same sheet's function table, but a
/// truth table rather than gates, so the two fail differently.
fn s181(a: u8, b: u8, s: u8, m: bool, cnb: bool) -> (u8, bool, bool, bool, bool) {
    let sel = |n: u32| if s >> n & 1 != 0 { 0xf } else { 0 };
    let bb = !b & 0xf;
    let pbar = !((a & b & sel(3)) | (a & bb & sel(2))) & 0xf;
    let gbar = !(a | (bb & sel(1)) | (b & sel(0))) & 0xf;
    let p = |n: u32| pbar >> n & 1 != 0;
    let g = |n: u32| gbar >> n & 1 != 0;
    let all_p = p(0) && p(1) && p(2) && p(3);

    let x = !all_p;
    let y = !(g(3) || (g(2) && p(3)) || (g(1) && p(2) && p(3)) || (g(0) && p(1) && p(2) && p(3)));
    let cn4b = !(y && !(all_p && cnb));

    // The carry into each bit, active low.
    let c = [
        !cnb,
        !(g(0) || (cnb && p(0))),
        !(g(1) || (g(0) && p(1)) || (cnb && p(0) && p(1))),
        !(g(2) || (g(1) && p(2)) || (g(0) && p(1) && p(2)) || (cnb && p(0) && p(1) && p(2))),
    ];
    // Mode high forces every carry, which is what turns the arithmetic
    // functions into the logic ones.
    let mut f = 0u8;
    for (n, &cn) in c.iter().enumerate() {
        let bit = p(n as u32) ^ g(n as u32) ^ (cn || m);
        f |= (bit as u8) << n;
    }
    (f, cn4b, f == 0xf, x, y)
}

/// The 74S182 carry-lookahead.
///
/// Written from the logic equations on page 1 of `sn74182.pdf` --- TI
/// SDLS206, December 1972, revised March 1988 --- in their `X`/`Y` form,
/// which is the one that applies here because CADR runs the ALU with
/// active-high data.  With `-Cn` the inverted pin 13 carry, the sheet gives:
///
/// ```text
/// -Cn+x = NOT( Y0 (X0 + -Cn) )
/// -Cn+y = NOT( Y1 (X1 + Y0 (X0 + -Cn)) )
/// -Cn+z = NOT( Y2 (X2 + Y1 (X1 + Y0 (X0 + -Cn))) )
/// Y     = Y3 (X3 + Y2) (X3 + X2 + Y1) (X3 + X2 + X1 + Y0)
/// X     = X3 + X2 + X1 + X0
/// ```
///
/// `xin` and `yin` are the four propagate and generate inputs as they arrive
/// from the 181s --- pins 4, 2, 15, 6 and 3, 1, 14, 5.  Returns
/// `(x, y, cn_x, cn_y, cn_z)` for pins 7, 10, 12, 11 and 9.
fn s182(xin: u8, yin: u8, cn: bool) -> (bool, bool, bool, bool, bool) {
    let cnb = !cn;
    let x = |n: u32| xin >> n & 1 != 0;
    let y = |n: u32| yin >> n & 1 != 0;

    // The bracketed term of each equation is the one inside the next.
    let t0 = x(0) || cnb;
    let t1 = x(1) || (y(0) && t0);
    let t2 = x(2) || (y(1) && t1);

    let x_out = x(3) || x(2) || x(1) || x(0);
    let y_out = y(3) && (x(3) || y(2)) && (x(3) || x(2) || y(1)) && (x(3) || x(2) || x(1) || y(0));
    (x_out, y_out, !(y(0) && t0), !(y(1) && t1), !(y(2) && t2))
}

/// One bit of a 25S10 shifter.  Inputs: `-ce` p13, `sel1` p9, `sel0` p10,
/// then `i-3`..`i3` in pin order p1..p7.  Output `n` is 0..3.
fn s10(i: &[bool], n: usize) -> Level {
    if i[0] {
        return Level::Z;
    }
    let shift = (i[1] as usize) << 1 | i[2] as usize;
    lv(i[6 + n - shift])
}

/// A 74S181 gate's A, B, S, mode and carry inputs, in pin order.
const ALU_IN: &[u8] = &[2, 23, 21, 19, 1, 22, 20, 18, 6, 5, 4, 3, 8, 7];
/// The same without the mode, for the carry out, which does not use it.
const ALU_CIN: &[u8] = &[2, 23, 21, 19, 1, 22, 20, 18, 6, 5, 4, 3, 7];
/// The same without mode or carry, for P and G, which use neither.
const ALU_PGIN: &[u8] = &[2, 23, 21, 19, 1, 22, 20, 18, 6, 5, 4, 3];

/// The 74S182's P, G and carry inputs, in `pb0..pb3, gb0..gb3, cn` order.
const CLA_IN: &[u8] = &[4, 2, 15, 6, 3, 1, 14, 5, 13];
/// The same without the carry, for the group P and G outputs.
const CLA_PGIN: &[u8] = &[4, 2, 15, 6, 3, 1, 14, 5];

// --- shapes the parts share ------------------------------------------------

/// A flip-flop output; `i` is `-clr` then `-pre`, and `q` the state the
/// part's update holds, which is where the asynchronous inputs are
/// applied. Both low holds both outputs high, which is the datasheet's
/// unstable state rather than an error.
///
/// **The outputs follow the state, not the pins.** Forcing the output
/// combinationally while `-pre` or `-clr` is low, so that a preset shows in
/// the same settle, is right for a preset held from outside and wrong for one
/// that clears itself: the I/O board's Chaosnet edge detector at
/// LMDETC sets a lockout flop by `-EDGE`, whose `-Q` then clears the
/// edge flop that made `-EDGE`, all through gates with no delay. Forced
/// combinationally, that preset comes and goes inside one settle before
/// the update latches it, the loop goes round again, and the settle gives
/// up on the group with the flop unset --- the second edge of every
/// packet losing `GENCLK`. Hardware latches on the pulse; so does the
/// update, one pass later, which [`crate::chip::Chip::transition`]
/// allows for. `a_packet_loops_back_through_the_board` in
/// `tests/chaos_netlist.rs` is the regression.
fn ff(i: &[bool], q: bool, complement: bool) -> Level {
    match (i[0], i[1]) {
        (false, false) => Level::High,
        _ => lv(q != complement),
    }
}

/// The next state of a J-K flip-flop.
fn jk(j: bool, k: bool, q: bool) -> bool {
    match (j, k) {
        (false, false) => q,
        (false, true) => false,
        (true, false) => true,
        (true, true) => !q,
    }
}

/// A register output with its asynchronous clear applied; `i` is `-clr`.
fn clr_q(i: &[bool], q: bool) -> Level {
    lv(i[0] && q)
}

/// The complement of one.
fn clr_qn(i: &[bool], q: bool) -> Level {
    lv(!(i[0] && q))
}

/// A three-state output; `i` is `-oe`.
fn oe(i: &[bool], q: bool) -> Level {
    if i[0] { Level::Z } else { lv(q) }
}

/// A transparent latch's output: high impedance when disabled, its input
/// while the latch is open, and what it holds once the latch closes. `i` is
/// `-oe`, the latch enable, then the data pin.
fn latch(i: &[bool], q: bool) -> Level {
    if i[0] {
        Level::Z
    } else if i[1] {
        lv(i[2])
    } else {
        lv(q)
    }
}

/// A three-state output inverted when the polarity pin is high; `i` is
/// `-oe` then `pol`.
fn oe_pol(i: &[bool], q: bool) -> Level {
    if i[0] { Level::Z } else { lv(q != i[1]) }
}

/// An open-collector bus driver, inverting: `i` is the enable (active low)
/// and the driver input.  Driving a one on an open-collector pin is not
/// driving at all, so a disabled driver and a low input read the same here
/// and [`Drive`] does the rest.
const OC_DRIVE: GateFn = |i, _| lv(i[0] || !i[1]);

/// A DM8838 driver: `i` is the two disables and the data in; the sheet's
/// one NOR of the disables enables all four drivers, so the bus is pulled
/// low only when both are low and the data is high.
const OC_BUS: GateFn = |i, _| lv(i[0] || i[1] || !i[2]);

/// One side of a three-state bidirectional transceiver.  `i` is the chip
/// disable (active low), the direction pin, and the pin at the other side;
/// `to_b` says which way the direction pin has to point for this side to
/// drive.
fn xcv(i: &[bool], to_b: bool) -> Level {
    if i[0] || i[1] != to_b {
        return Level::Z;
    }
    lv(i[2])
}

/// One bit out of a part's memory array.  Unknown if the array has not been
/// loaded, which is how a PROM nobody filled in shows up.
fn cell(s: &State, addr: usize, n: u32) -> Level {
    match s.cells.get(addr) {
        Some(&word) => lv(word >> n & 1 != 0),
        None => Level::X,
    }
}

/// An address gathered out of a gate's inputs, `at` first and lowest.
fn addr(i: &[bool], at: usize, width: usize) -> usize {
    (0..width).fold(0, |a, k| a | (i[at + k] as usize) << k)
}

/// The `D` inputs of a 74S373 or 74S374, in bit order.  The two number their
/// bits in opposite directions, but pair the same pins.
const OCTAL_D: &[u8] = &[3, 4, 7, 8, 13, 14, 17, 18];

/// One output of an 82S21.  Inputs: `ce` p5, the latch on p6, then `a0`..`a4`.
fn ram32(i: &[bool], s: &State, n: u32) -> Level {
    if !i[0] {
        return Level::Z;
    }
    if !i[1] {
        return lv(bit(s.bits, n));
    }
    cell(s, addr(i, 2, 5), n)
}

/// The output of a one-bit RAM.  Inputs: `-ce`, `-we`, then `width` address
/// pins, lowest first.  High impedance while deselected or being written.
fn ram1(i: &[bool], s: &State, width: usize) -> Level {
    if i[0] || !i[1] {
        return Level::Z;
    }
    cell(s, addr(i, 2, width), 0)
}

/// One bit of a wider RAM's output.  Inputs: `-ce`, `-we`, then `width`
/// address pins, lowest first.  High impedance while deselected or written.
fn ramn(i: &[bool], s: &State, width: usize, n: u32) -> Level {
    if i[0] || !i[1] {
        return Level::Z;
    }
    cell(s, addr(i, 2, width), n)
}

/// The write side of a one-bit RAM, taken at the end of the write pulse: the
/// datasheet lets the data arrive after the pulse starts but requires it held
/// past the end, so the pins as they were inside the pulse are the ones that
/// count.
fn ram_write(now: &Pins, prev: &Pins, st: &mut State, pins: &[u8], ce: u8, we: u8, di: u8) {
    let writing = |p: &Pins| !v(p, ce) && !v(p, we);
    if writing(prev) && !writing(now) {
        let a = word(prev, pins) as usize;
        if let Some(c) = st.cells.get_mut(a) {
            *c = v(prev, di) as u8;
        }
    }
}

/// The 4116's address pins, `A0` first: 5, 7, 6, 12, 11, 10, 13.
const DRAM_ADDR: &[u8] = &[5, 7, 6, 12, 11, 10, 13];

/// One bit of a 4116, eight to a cell.
fn dram_bit(s: &State, a: usize) -> Level {
    match s.cells.get(a / 8) {
        Some(&c) => lv(c >> (a % 8) & 1 != 0),
        None => Level::X,
    }
}

/// One output of a three-state two-to-one multiplexer. Inputs: `-oe`, the
/// select --- low for A --- then A and B.
const MUX2: GateFn = |i, _| {
    if i[0] { Level::Z } else { lv(if i[1] { i[3] } else { i[2] }) }
};

/// One output of a PROM.  Inputs: `-ce`, then `width` address pins.
fn prom(i: &[bool], s: &State, width: usize, n: u32) -> Level {
    if i[0] {
        return Level::Z;
    }
    cell(s, addr(i, 1, width), n)
}

const RAM32_IN: &[u8] = &[5, 6, 13, 12, 11, 10, 4];
const RAM4K_ADDR: &[u8] = &[1, 2, 3, 4, 5, 6, 17, 16, 15, 14, 13, 12];
const RAM4K_IN: &[u8] = &[10, 8, 1, 2, 3, 4, 5, 6, 17, 16, 15, 14, 13, 12];
const RAM1K_ADDR: &[u8] = &[2, 3, 4, 5, 6, 9, 10, 11, 12, 13];
const RAM1K_IN: &[u8] = &[1, 14, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13];
const PROM512_IN: &[u8] = &[15, 1, 2, 3, 4, 5, 16, 17, 18, 19];
const PROM32_IN: &[u8] = &[15, 10, 11, 12, 13, 14];
/// The 74S287's chip select and its eight address pins, `A0` first: 5, 6,
/// 7, 4, 3, 2, 1, 15. `sn74s287.pdf` letters them `AD C`, `AD B`, `AD A`,
/// `AD D`.. `AD H` and never says which is the low bit; MIT's own table
/// for the one at LMMYNM 0D01, `mit/chaos/lmmynm.promt`, lists the pins as
/// `15 1 2 3 4 7 6 5` and its burned image `lmmynm.prom` agrees with the
/// table only when that order is the address from its top bit down ---
/// `the_address_prom_is_its_table` in `tests/chaos.rs`. That is also the
/// 256 x 4 PROM family's pinout, the 82S129's among them.
const PROM256_IN: &[u8] = &[13, 5, 6, 7, 4, 3, 2, 1, 15];
const RAM16X4_ADDR: &[u8] = &[13, 14, 15, 1];
const RAM16X4_IN: &[u8] = &[2, 3, 13, 14, 15, 1];

/// How many words a part's memory array holds, if it has one.
///
/// PROM contents come from outside; RAM starts wherever the engine puts it.
/// No RAM datasheet defines the cells after power-up and nothing on the
/// boards clears them, so a model has to choose: the engine zeroes them,
/// and `src/chip.rs` says what a zero means in each array.
///
/// Looked up as [`pinout`] and [`behaviour`] look a kind up, alias first:
/// a 2118 is a 4116 and has its cells. Going by [`strip`] alone resolves the
/// suffix but not the alias, which gives the LISPM TV's frame buffer 64 DRAMs
/// with the 4116's pins and cycle and no cells --- every write dropped, every
/// read `X`.
pub fn memory_words(kind: &str) -> Option<usize> {
    Some(match strip(canonical(kind)).0 {
        "2147" => 4096,
        "93425" => 1024,
        "82S21" | "5600" | "5610" | "74S288" => 32,
        // 1,024 bits as 256 words of four: `sn74s287.pdf`.
        "74S287" => 256,
        // Not a memory that is addressed: the 67401 is a 64-deep queue and
        // these are its cells, one nibble each, front at index 0.
        "67401" => 64,
        // Not a memory either: the 2651's registers, holding registers,
        // shift registers and counters, [`PCI_CELLS`].
        "2651" => PCI_CELLS,
        // 16,384 bits, eight to a cell.
        "4116VG" => 2048,
        "29701" => 16,
        "74S472" => 512,
        _ => return None,
    })
}

/// Looks up what a part type computes, as the netlist spells it.
///
/// The suffix rules are [`pinout`]'s: a trailing `O` only changes the drive,
/// not the logic.  Types that share a pinout but not a function --- the
/// 74S10 and 74S11, the 74S240 and the 74S241 --- get separate entries.
///
/// [`None`] means the type has no logic here: the analog parts, which
/// cannot be levelized.  They are the delay lines --- `TD25` to `TD250`,
/// `TD25NC`, `TD100NC` and `MTD100` --- the oscillators `DIPOSC`, `TTLOSC`
/// and the 74LS124, and the Am26S02 one-shot.  `src/clock.rs` runs the
/// processor clock's delay lines and `src/chip.rs` the rest.
///
/// Every pin assignment below was taken from the part's own datasheet, and
/// each is named where it is used. `tests/behaviour.rs` holds the pinout and
/// the gates to each other: between them they must account for every pin the
/// netlist connects.
pub fn behaviour(kind: &str) -> Option<Behaviour> {
    let (base, _) = strip(canonical(kind));
    Some(match base {
        // --- gates ---------------------------------------------------------
        // Quad two-input, outputs on 1/4/10/13.
        "7428" | "74S02" | "OS02L" => comb(
            const {
                &[
                    g(1, &[2, 3], NOR),
                    g(4, &[5, 6], NOR),
                    g(10, &[8, 9], NOR),
                    g(13, &[11, 12], NOR),
                ]
            },
        ),
        // Quad two-input, outputs on 3/6/8/11.
        "74S00" | "74S37" | "74S38" | "OS00L" | "S00L" | "S37L" => comb(
            const {
                &[
                    g(3, &[1, 2], NAND),
                    g(6, &[4, 5], NAND),
                    g(8, &[9, 10], NAND),
                    g(11, &[12, 13], NAND),
                ]
            },
        ),
        "74S08" | "S08L" => comb(
            const {
                &[
                    g(3, &[1, 2], AND),
                    g(6, &[4, 5], AND),
                    g(8, &[9, 10], AND),
                    g(11, &[12, 13], AND),
                ]
            },
        ),
        "74S32" | "OS32L" => comb(
            const { &[g(3, &[1, 2], OR), g(6, &[4, 5], OR), g(8, &[9, 10], OR), g(11, &[12, 13], OR)] },
        ),
        "74S86" | "S86L" => comb(
            const {
                &[
                    g(3, &[1, 2], XOR),
                    g(6, &[4, 5], XOR),
                    g(8, &[9, 10], XOR),
                    g(11, &[12, 13], XOR),
                ]
            },
        ),
        // Hex inverter. The 74LS14 is the Schmitt-trigger version; the
        // hysteresis is a timing property, and the logic is the same.
        "74S04" | "74LS14" | "OS04L" => comb(
            const {
                &[
                    g(2, &[1], NOT),
                    g(4, &[3], NOT),
                    g(6, &[5], NOT),
                    g(8, &[9], NOT),
                    g(10, &[11], NOT),
                    g(12, &[13], NOT),
                ]
            },
        ),
        "74S10" => comb(
            const { &[g(6, &[3, 4, 5], NAND), g(8, &[9, 10, 11], NAND), g(12, &[1, 2, 13], NAND)] },
        ),
        "74S11" | "OS11L" => comb(
            const { &[g(6, &[3, 4, 5], AND), g(8, &[9, 10, 11], AND), g(12, &[1, 2, 13], AND)] },
        ),
        "74S20" => comb(const { &[g(6, &[1, 2, 4, 5], NAND), g(8, &[9, 10, 12, 13], NAND)] }),
        // Dual two-wide two-input AND-OR-invert. The second gate's fourth
        // input is pin 1, across the package from the rest of it.
        // `Y = AB + CD` twice over, `sn7451.pdf`: gate 2 is `2A`, `2B`
        // into one AND and `2C`, `2D` into the other, out on 6; gate 1 is
        // `1A`, `1B` and `1C`, `1D`, out on 8. Both inverting.
        "74S51" => comb(const { &[g(6, &[2, 3, 4, 5], AOI22), g(8, &[9, 10, 13, 1], AOI22)] }),
        // 4-2-3-2 AND-OR-invert, one gate over the whole package.
        "74S64" => comb(
            const {
                &[g(8, &[9, 10, 3, 2, 6, 5, 4, 13, 12, 11, 1], |i, _| {
                    lv(!((i[0] && i[1])
                        || (i[2] && i[3])
                        || (i[4] && i[5] && i[6])
                        || (i[7] && i[8] && i[9] && i[10])))
                })]
            },
        ),
        // Dual five-input NOR; the two gates interleave across the package.
        "74S260" => comb(const { &[g(5, &[1, 2, 3, 12, 13], NOR), g(6, &[4, 8, 9, 10, 11], NOR)] }),
        // Thirteen-input NAND, the widest gate on the board.
        "74S133" => comb(const { &[g(9, &[1, 2, 3, 4, 5, 6, 7, 10, 11, 12, 13, 14, 15], NAND)] }),
        // Fairchild 9S42: two AND-OR gates, mirrored about the supply pins.
        "9S42-1" => comb(
            const {
                &[
                    g(7, &[1, 2, 3, 4, 5, 6], |i, _| {
                        lv((i[0] && i[1]) || (i[2] && i[3] && i[4] && i[5]))
                    }),
                    g(9, &[15, 14, 13, 12, 11, 10], |i, _| {
                        lv((i[0] && i[1]) || (i[2] && i[3] && i[4] && i[5]))
                    }),
                ]
            },
        ),

        // --- parity --------------------------------------------------------
        // Nine-bit: even on 5, odd on 6.
        "74S280" => comb(
            const {
                &[
                    g(5, &[1, 2, 4, 8, 9, 10, 11, 12, 13], XNOR),
                    g(6, &[1, 2, 4, 8, 9, 10, 11, 12, 13], XOR),
                ]
            },
        ),
        // Twelve-bit: odd on 9, even on 10.
        "93S48" => comb(
            const {
                &[
                    g(9, &[1, 2, 3, 4, 5, 6, 7, 11, 12, 13, 14, 15], XOR),
                    g(10, &[1, 2, 3, 4, 5, 6, 7, 11, 12, 13, 14, 15], XNOR),
                ]
            },
        ),
        // Six-bit identity comparator, enabled by pin 7.
        "93S46" => comb(
            const {
                &[g(9, &[7, 1, 2, 3, 4, 5, 6, 10, 11, 12, 13, 14, 15], |i, _| {
                    lv(i[0] && (1..=6).all(|n| i[2 * n - 1] == i[2 * n]))
                })]
            },
        ),

        // --- decoders and multiplexers --------------------------------------
        "74S138" => comb(
            const {
                &[
                    g(7, &[1, 2, 3, 4, 5, 6], |i, _| s138(i, 7)),
                    g(9, &[1, 2, 3, 4, 5, 6], |i, _| s138(i, 6)),
                    g(10, &[1, 2, 3, 4, 5, 6], |i, _| s138(i, 5)),
                    g(11, &[1, 2, 3, 4, 5, 6], |i, _| s138(i, 4)),
                    g(12, &[1, 2, 3, 4, 5, 6], |i, _| s138(i, 3)),
                    g(13, &[1, 2, 3, 4, 5, 6], |i, _| s138(i, 2)),
                    g(14, &[1, 2, 3, 4, 5, 6], |i, _| s138(i, 1)),
                    g(15, &[1, 2, 3, 4, 5, 6], |i, _| s138(i, 0)),
                ]
            },
        ),
        "74S139" => comb(
            const {
                &[
                    g(4, &[1, 2, 3], |i, _| s139(i, 0)),
                    g(5, &[1, 2, 3], |i, _| s139(i, 1)),
                    g(6, &[1, 2, 3], |i, _| s139(i, 2)),
                    g(7, &[1, 2, 3], |i, _| s139(i, 3)),
                    g(9, &[15, 14, 13], |i, _| s139(i, 3)),
                    g(10, &[15, 14, 13], |i, _| s139(i, 2)),
                    g(11, &[15, 14, 13], |i, _| s139(i, 1)),
                    g(12, &[15, 14, 13], |i, _| s139(i, 0)),
                ]
            },
        ),
        // Eight-to-one, true on 5 and complement on 6.
        "74S151" => comb(
            const {
                &[
                    g(5, &[7, 11, 10, 9, 4, 3, 2, 1, 15, 14, 13, 12], |i, _| lv(s151(i))),
                    g(6, &[7, 11, 10, 9, 4, 3, 2, 1, 15, 14, 13, 12], |i, _| lv(!s151(i))),
                ]
            },
        ),
        // Dual four-to-one, select shared on 14 (low) and 2 (high).
        "74S153" => comb(
            const { &[g(7, &[1, 14, 2, 6, 5, 4, 3], S153), g(9, &[15, 14, 2, 10, 11, 12, 13], S153)] },
        ),
        "74S157" => comb(
            const {
                &[
                    g(4, &[1, 15, 2, 3], S157),
                    g(7, &[1, 15, 5, 6], S157),
                    g(9, &[1, 15, 11, 10], S157),
                    g(12, &[1, 15, 14, 13], S157),
                ]
            },
        ),
        "74S158" => comb(
            const {
                &[
                    g(4, &[1, 15, 2, 3], S158),
                    g(7, &[1, 15, 5, 6], S158),
                    g(9, &[1, 15, 11, 10], S158),
                    g(12, &[1, 15, 14, 13], S158),
                ]
            },
        ),
        "26LS31" => comb(
            const {
                &[
                    g(2, &[4, 12, 1], |i, _| ls31(i, false)),
                    g(3, &[4, 12, 1], |i, _| ls31(i, true)),
                    g(6, &[4, 12, 7], |i, _| ls31(i, false)),
                    g(5, &[4, 12, 7], |i, _| ls31(i, true)),
                    g(10, &[4, 12, 9], |i, _| ls31(i, false)),
                    g(11, &[4, 12, 9], |i, _| ls31(i, true)),
                    g(14, &[4, 12, 15], |i, _| ls31(i, false)),
                    g(13, &[4, 12, 15], |i, _| ls31(i, true)),
                ]
            },
        ),
        // Each receiver's non-inverting input first, then its inverting
        // one, which is pin 2 before pin 1 for the first and the other way
        // about for the rest. Getting that pair round the wrong way is
        // invisible on this board on its own, because `cadrio/iob.wlr` puts
        // `RCVR.DATA+` on the inverting input and the two errors cancel;
        // `tests/behaviour.rs` holds the pinout, the loop through the line
        // driver and [`crate::unibus::IDLE_CHAOSNET`] to each other so that
        // neither can move alone.
        "26LS33" => comb(
            const {
                &[
                    g(3, &[4, 12, 2, 1], |i, _| ls33(i)),
                    g(5, &[4, 12, 6, 7], |i, _| ls33(i)),
                    g(11, &[4, 12, 10, 9], |i, _| ls33(i)),
                    g(13, &[4, 12, 15, 14], |i, _| ls33(i)),
                ]
            },
        ),
        // SN74S287: `DO 1` on pin 12 is the low bit of the four, as the
        // '288's `DO 1` on pin 1 is the low bit of its eight --- MIT's
        // table for LMMYNM 0D01 lists the outputs `9 10 11 12` and its image
        // agrees only with 12 as the low bit, `tests/chaos.rs`. Both selects
        // must be low; the board grounds them.
        "74S287" => comb(
            const {
                &[
                    g(12, PROM256_IN, |i, s| prom(i, s, 8, 0)),
                    g(11, PROM256_IN, |i, s| prom(i, s, 8, 1)),
                    g(10, PROM256_IN, |i, s| prom(i, s, 8, 2)),
                    g(9, PROM256_IN, |i, s| prom(i, s, 8, 3)),
                ]
            },
        ),
        // Fairchild 9401 (`9401.pdf`). A 16-bit register divided by one of
        // eight polynomials, the code on `S2 S1 S0` decoded through the
        // part's own ROM. The sheet: "This data is gated with the most
        // significant output (Q) of the register, and controls the
        // Exclusive OR gates ... The Check Word Enable (CWE) must be held
        // HIGH while the data is being entered. After the last data bit is
        // entered, the CWE is brought LOW and the check bits are shifted
        // out of the register" --- so the feedback is `D` exclusive-or `Q`
        // while `CWE` is high and zero once it is low. Clocking is on the
        // **high-to-low** transition of `CP`, which the sheet says twice.
        // `MR` high clears the register at once; `-P` low sets it, and for
        // a polynomial of degree less than 16 only the top `d` bits are
        // set, the rest cleared --- the sheet's "automatic right
        // justification", which is why the register here is always the top
        // `d` stages and `Q` is always bit 15. `ER` is low when the
        // register is all zero, which after a good message it is.
        "9401" => seq(
            const { &[g(12, &[], |_, s| lv(bit(s.bits, 15))), g(13, &[], |_, s| lv(s.bits != 0))] },
            &[1, 2, 3, 4, 5, 8, 10, 11],
            |now, prev, st| {
                let code =
                    (v(now, 8) as usize) << 2 | (v(now, 5) as usize) << 1 | v(now, 3) as usize;
                let (degree, taps) = CRC9401[code];
                let low = 16 - degree;
                let active = !0u16 << low;
                if v(now, 4) {
                    st.bits = 0;
                } else if !v(now, 2) {
                    st.bits = active;
                } else if fell(now, prev, 1) {
                    let msb = bit(st.bits, 15);
                    let fb = v(prev, 10) && (v(prev, 11) ^ msb);
                    let mut next = 0u16;
                    for i in 0..degree {
                        let from = if i == 0 { fb } else { bit(st.bits, low + i - 1) };
                        let tap = fb && taps >> i & 1 != 0 && i != 0;
                        if from ^ tap {
                            next |= 1 << (low + i);
                        }
                    }
                    st.bits = next & active;
                }
            },
        ),
        // SN74S251: `Y` on 5 and its complement `W` on 6, selected by
        // `C`, `B`, `A` on 9, 10, 11 and enabled by `-G` on 7.
        "74S251" => comb(
            const {
                &[
                    g(5, &[7, 11, 10, 9, 4, 3, 2, 1, 15, 14, 13, 12], |i, _| s251(i, false)),
                    g(6, &[7, 11, 10, 9, 4, 3, 2, 1, 15, 14, 13, 12], |i, _| s251(i, true)),
                ]
            },
        ),
        "74S258" => comb(
            const {
                &[
                    g(4, &[1, 15, 2, 3], S258),
                    g(7, &[1, 15, 5, 6], S258),
                    g(9, &[1, 15, 11, 10], S258),
                    g(12, &[1, 15, 14, 13], S258),
                ]
            },
        ),

        // --- the display board's own parts ----------------------------------

        // Dual four-to-one with three-state outputs: the '153's pins, and
        // off rather than low when the enable is high.
        "74S253" => comb(
            const { &[g(7, &[1, 14, 2, 6, 5, 4, 3], S253), g(9, &[15, 14, 2, 10, 11, 12, 13], S253)] },
        ),
        // Quad two-to-one with three-state outputs, the '258's pins without
        // the inversion.
        "74S257" => comb(
            const {
                &[
                    g(4, &[1, 15, 2, 3], S257),
                    g(7, &[1, 15, 5, 6], S257),
                    g(9, &[1, 15, 11, 10], S257),
                    g(12, &[1, 15, 14, 13], S257),
                ]
            },
        ),

        // The MECL terminator offers a low on every line, as the resistor
        // packs offer a high; [`resolve`] lets any open-emitter output win.
        "SIP-R121/195-8" => comb(
            const {
                &[
                    g(2, &[], LOW),
                    g(3, &[], LOW),
                    g(4, &[], LOW),
                    g(5, &[], LOW),
                    g(6, &[], LOW),
                    g(7, &[], LOW),
                    g(10, &[], LOW),
                    g(11, &[], LOW),
                    g(12, &[], LOW),
                    g(13, &[], LOW),
                    g(14, &[], LOW),
                    g(15, &[], LOW),
                ]
            },
        ),

        // --- the display board's MECL parts ---------------------------------
        // Levels are MECL's, but a NOR is a NOR: the model has one Level
        // type and the SIP terminators are what would tell the two apart.
        // Every gate here is `gm`, an undriven input reading low.

        // Quad two-input NOR, the fourth gate with its OR on 9 as well.
        "10102" => comb(
            const {
                &[
                    gm(2, &[4, 5], NOR),
                    gm(3, &[6, 7], NOR),
                    gm(14, &[10, 11], NOR),
                    gm(15, &[12, 13], NOR),
                    gm(9, &[12, 13], OR),
                ]
            },
        ),
        // Triple 2-3-2 input OR/NOR.
        "10105" => comb(
            const {
                &[
                    gm(2, &[4, 5], OR),
                    gm(3, &[4, 5], NOR),
                    gm(7, &[9, 10, 11], OR),
                    gm(6, &[9, 10, 11], NOR),
                    gm(14, &[12, 13], OR),
                    gm(15, &[12, 13], NOR),
                ]
            },
        ),
        // Four-wide OR-AND on 2, its complement on 3; pin 10 is in two of
        // the ORs.
        "10121" => comb(
            const {
                &[
                    gm(2, &[4, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15], |i, _| lv(or_and4(i))),
                    gm(3, &[4, 5, 6, 7, 9, 10, 11, 12, 13, 14, 15], |i, _| lv(!or_and4(i))),
                ]
            },
        ),
        // TTL in, MECL out: a strobe low on 6 forces every true output low
        // and every complement high; otherwise each pair follows its input.
        // The inputs are TTL, so these are `g` and float high.
        "10124" => comb(
            const {
                &[
                    g(2, &[5, 6], AND),
                    g(4, &[5, 6], NAND),
                    g(1, &[7, 6], AND),
                    g(3, &[7, 6], NAND),
                    g(15, &[10, 6], AND),
                    g(12, &[10, 6], NAND),
                    g(14, &[11, 6], AND),
                    g(13, &[11, 6], NAND),
                ]
            },
        ),
        // MECL in, TTL out. Each receiver is differential, and the board
        // uses two of the four single-ended with the true input tied to
        // VBB and the signal on the barred one, so the output is the
        // signal inverted: "when the input pin with the bubble goes
        // positive, the output goes negative". VBB is pin 1's own output
        // and no logic level; read as a MECL input it is an undriven net
        // and comes out low, so the true input is taken through the TTL
        // reader, where it floats high --- right for this wiring, and
        // documented as such rather than a general receiver. Inputs in
        // order: the barred one, then the true one.
        "10125" => comb(
            const {
                &[
                    g(4, &[2, 3], RECEIVER),
                    g(5, &[6, 7], RECEIVER),
                    g(12, &[10, 11], RECEIVER),
                    g(13, &[14, 15], RECEIVER),
                    // VBB: an internal reference with nothing to say here.
                    gm(1, &[], |_, _| Level::Z),
                ]
            },
        ),
        // Universal hexadecimal counter. Bit 0 is Q0 on 14. On a rising
        // clock, by `S1` p9 and `S2` p7: low-low presets from D0..D3, low-high
        // counts up and high-low counts down when `-Cin` p10 is low, and
        // high-high holds. `-Cout` p4 is low on the terminal count of the
        // direction in hand, or while presetting.
        "10136" => seq(
            const {
                &[
                    gm(14, &[], |_, s| lv(bit(s.bits, 0))),
                    gm(15, &[], |_, s| lv(bit(s.bits, 1))),
                    gm(2, &[], |_, s| lv(bit(s.bits, 2))),
                    gm(3, &[], |_, s| lv(bit(s.bits, 3))),
                    gm(4, &[9, 7, 10], |i, s| {
                        let q = s.bits & 0xf;
                        let terminal = match (i[0], i[1]) {
                            (false, false) => true,
                            (false, true) => !i[2] && q == 0xf,
                            (true, false) => !i[2] && q == 0,
                            (true, true) => false,
                        };
                        lv(!terminal)
                    }),
                ]
            },
            &[5, 6, 7, 9, 10, 11, 12, 13],
            |now, prev, st| {
                if vm(now, 13) && !vm(prev, 13) {
                    let q = st.bits & 0xf;
                    st.bits = match (vm(prev, 9), vm(prev, 7)) {
                        (false, false) => word_m(prev, &[12, 11, 6, 5]) as u16,
                        (false, true) if !vm(prev, 10) => (q + 1) & 0xf,
                        (true, false) if !vm(prev, 10) => (q + 15) & 0xf,
                        _ => q,
                    };
                }
            },
        ),
        // Four-bit universal shift register. Bit 0 is Q0 on 14. On a
        // rising clock, by `S1` p10 and `S2` p7: low-low loads D0..D3,
        // low-high shifts right --- Q0 takes Q1, Q3 takes `DR` p5 ---
        // high-low shifts left, Q0 taking `DL` p13, and high-high holds.
        "10141" => seq(
            const {
                &[
                    gm(14, &[], |_, s| lv(bit(s.bits, 0))),
                    gm(15, &[], |_, s| lv(bit(s.bits, 1))),
                    gm(2, &[], |_, s| lv(bit(s.bits, 2))),
                    gm(3, &[], |_, s| lv(bit(s.bits, 3))),
                ]
            },
            &[4, 5, 6, 7, 9, 10, 11, 12, 13],
            |now, prev, st| {
                if vm(now, 4) && !vm(prev, 4) {
                    let q = st.bits & 0xf;
                    st.bits = match (vm(prev, 10), vm(prev, 7)) {
                        (false, false) => word_m(prev, &[12, 11, 9, 6]) as u16,
                        (false, true) => (q >> 1) | (vm(prev, 5) as u16) << 3,
                        (true, false) => (q << 1 & 0xf) | vm(prev, 13) as u16,
                        (true, true) => q,
                    };
                }
            },
        ),
        // Dual three-input OR/NOR, three outputs a gate: OR on 2 and 12,
        // NOR twice over on 3, 4 and 13, 14.
        "10212" => comb(
            const {
                &[
                    gm(2, &[5, 6, 7], OR),
                    gm(3, &[5, 6, 7], NOR),
                    gm(4, &[5, 6, 7], NOR),
                    gm(12, &[9, 10, 11], OR),
                    gm(13, &[9, 10, 11], NOR),
                    gm(14, &[9, 10, 11], NOR),
                ]
            },
        ),

        // --- the disk controller's own parts --------------------------------

        // Dual four-input AND, outputs on 6 and 8; pins 3 and 11 are not
        // connected.
        "74S21" => comb(const { &[g(6, &[1, 2, 4, 5], AND), g(8, &[9, 10, 12, 13], AND)] }),
        // Dual peripheral NAND driver, open collector: 1Y=3 from 1 and 2,
        // 2Y=5 from 6 and 7.
        "75452" => comb(const { &[g(3, &[1, 2], NAND), g(5, &[6, 7], NAND)] }),
        // MMI 67401, 64-by-4 fall-through FIFO, `67401.pdf`. Four of them
        // are the disk controller's read and write buffers.
        //
        // The datasheet's functional description, and the model is exactly
        // it: data enters on D0-D3 when `SHIFT IN` rises and `INPUT READY`
        // is high, SI high forces IR low, and dropping SI raises IR again
        // unless the memory is full; data is read from O0-O3 while `OUTPUT
        // READY` is high, SO high forces OR low and holds the data, and
        // dropping SO brings the next word to the output stage. `MR` is
        // overbarred on the pin configuration, so master reset is **active
        // low**; DCRBUF and DCWBUF put `MBUSY` on it, which holds the
        // buffers clear whenever the memory side is idle.
        //
        // `State::cells` is the queue, front at index 0, and `State::bits`
        // holds how many are in it and the nibble the output stage keeps
        // after the last word leaves --- the real part maintains it, and
        // OUTPUT READY low is what says not to read it. The one thing not
        // modelled is `tPT`, the fall-through time: here a word is at the
        // output as soon as it is shifted in, where the part takes it
        // through 64 stages.
        "67401" => seq(
            const {
                &[
                    g(2, &[3, 9], |i, s| lv(i[1] && !i[0] && fifo_count(s) < 64)),
                    g(14, &[15, 9], |i, s| lv(i[1] && !i[0] && fifo_count(s) > 0)),
                    g(13, FIFO_INS, |_, s| lv(bit(fifo_front(s) as u16, 0))),
                    g(12, FIFO_INS, |_, s| lv(bit(fifo_front(s) as u16, 1))),
                    g(11, FIFO_INS, |_, s| lv(bit(fifo_front(s) as u16, 2))),
                    g(10, FIFO_INS, |_, s| lv(bit(fifo_front(s) as u16, 3))),
                ]
            },
            &[3, 4, 5, 6, 7, 9, 15],
            |now, prev, st| {
                if !v(now, 9) {
                    st.bits = 0;
                    return;
                }
                // The two ports are independent, and in a zero-delay model
                // their edges can land in one step: the shift out first, on
                // the count as it was, then the shift in.
                let mut count = (st.bits & 0x7f) as usize;
                if fell(now, prev, 15) && count > 0 {
                    // The word leaving is what the output stage keeps.
                    st.bits = (st.cells[0] as u16) << 7;
                    st.cells.copy_within(1..count, 0);
                    count -= 1;
                }
                if rose(now, prev, 3) && count < 64 {
                    st.cells[count] = word(prev, &[4, 5, 6, 7]) as u8;
                    count += 1;
                }
                st.bits = (st.bits & !0x7f) | count as u16;
            },
        ),

        // The Trident cable's two ends. `dctrid.drw` puts the data pair on
        // the receiver's first channel and the clock pair on its second
        // **crossed** --- `TRIDENT.0.CLOCK.M` on `2A` and `.P` on `2B`,
        // where the data pair has `.P` on `1A` --- so `DISK.CLK^` carries
        // the opposite sense to `READ DATA`. That is the drawing's, not a
        // slip here; `tests/cadrdc_netlist.rs` has the pins.
        "75107" => comb(const { &[g(4, &[1, 2, 5, 6], R75107), g(9, &[12, 11, 8, 6], R75107)] }),
        "75110" => comb(
            const {
                &[
                    g(13, &[1, 2, 3, 10], Y75110),
                    g(12, &[1, 2, 3, 10], Z75110),
                    g(8, &[5, 6, 4, 10], Y75110),
                    g(9, &[5, 6, 4, 10], Z75110),
                ]
            },
        ),
        // The Trident cable's terminator terminates and nothing else.
        // Pin n to pin 17-n, on the four odd pins the cable arrives on. See
        // the pinout above for why that is the geometry and not the other.
        "TRITERM" => comb(
            const { &[g(16, &[1], PASS), g(14, &[3], PASS), g(12, &[5], PASS), g(10, &[7], PASS)] },
        ),
        // The clock terminators and the two SIP packs offer a high on every
        // resistor pin, as `RES20` does; [`resolve`] makes it weak enough
        // for any real driver to win.
        "RES4" => comb(const { &[g(1, &[], HIGH), g(2, &[], HIGH)] }),
        "SIP100-8" => comb(
            const {
                &[
                    g(2, &[], HIGH),
                    g(3, &[], HIGH),
                    g(4, &[], HIGH),
                    g(5, &[], HIGH),
                    g(6, &[], HIGH),
                    g(7, &[], HIGH),
                ]
            },
        ),
        "SIP330-10" => comb(
            const {
                &[
                    g(2, &[], HIGH),
                    g(3, &[], HIGH),
                    g(4, &[], HIGH),
                    g(5, &[], HIGH),
                    g(6, &[], HIGH),
                    g(7, &[], HIGH),
                    g(8, &[], HIGH),
                    g(9, &[], HIGH),
                ]
            },
        ),
        "P SIP1000-10" => comb(
            const {
                &[
                    g(2, &[], HIGH),
                    g(3, &[], HIGH),
                    g(4, &[], HIGH),
                    g(5, &[], HIGH),
                    g(6, &[], HIGH),
                    g(7, &[], HIGH),
                    g(8, &[], HIGH),
                    g(9, &[], HIGH),
                    g(10, &[], HIGH),
                ]
            },
        ),

        // --- the bus interface's own parts ----------------------------------

        // Triple three-input NOR: 1Y=12, 2Y=6, 3Y=8.
        "74LS27" => comb(
            const { &[g(12, &[1, 2, 13], NOR), g(6, &[3, 4, 5], NOR), g(8, &[9, 10, 11], NOR)] },
        ),

        // SN74S112 (`sn74112.pdf`): dual J-K, negative edge, with active-low
        // preset and clear. Flop 1 is CLK 1, K 2, J 3, -PRE 4, Q 5, -Q 6,
        // -CLR 15; flop 2 is CLK 13, K 12, J 11, -PRE 10, Q 9, -Q 7, -CLR
        // 14. J and K both high toggle, which is how LMTCLK 0A06 halves
        // the crystal. K is a true input on the '112, as on the '74S112;
        // the J-K-bar parts are the '109 and the '276. The bus interface's
        // REQERR 0A03 is drawn `74LS112-1` and comes here through
        // `canonical`: the `-1` is a body variant, as on `9S42-1`, and with K
        // grounded its `XB PAR ERROR` sets on a bad request and holds until
        // `-RESET ERR`, as the board's other error flags do.
        "74S112" => seq(
            const {
                &[
                    g(5, &[15, 4], |i, s| ff(i, bit(s.bits, 0), false)),
                    g(6, &[15, 4], |i, s| ff(i, bit(s.bits, 0), true)),
                    g(7, &[14, 10], |i, s| ff(i, bit(s.bits, 1), true)),
                    g(9, &[14, 10], |i, s| ff(i, bit(s.bits, 1), false)),
                ]
            },
            &[1, 2, 3, 4, 10, 11, 12, 13, 14, 15],
            |now, prev, st| {
                // (bit, -clr, -pre, clk, j, k)
                for &(n, clr, pre, clk, j, k) in
                    &[(0u32, 15u8, 4u8, 1u8, 3u8, 2u8), (1, 14, 10, 13, 11, 12)]
                {
                    if !v(now, clr) {
                        put(&mut st.bits, n, false);
                    } else if !v(now, pre) {
                        put(&mut st.bits, n, true);
                    } else if fell(now, prev, clk) {
                        let q = bit(st.bits, n);
                        put(&mut st.bits, n, jk(v(prev, j), v(prev, k), q));
                    }
                }
            },
        ),

        // Datasheet SN74276: quadruple J-K̄, negative edge, with a common
        // clear on 1 and a common preset on 11 and four separate clocks.
        "74276" => seq(
            const {
                &[
                    g(5, &[1, 11], |i, s| ff(i, bit(s.bits, 0), false)),
                    g(6, &[1, 11], |i, s| ff(i, bit(s.bits, 1), false)),
                    g(15, &[1, 11], |i, s| ff(i, bit(s.bits, 2), false)),
                    g(16, &[1, 11], |i, s| ff(i, bit(s.bits, 3), false)),
                ]
            },
            &[1, 2, 3, 4, 7, 8, 9, 11, 12, 13, 14, 17, 18, 19],
            |now, prev, st| {
                // (bit, clk, j, -k)
                for &(n, clk, j, kn) in
                    &[(0u32, 3u8, 2u8, 4u8), (1, 8, 9, 7), (2, 13, 12, 14), (3, 18, 19, 17)]
                {
                    if !v(now, 1) {
                        put(&mut st.bits, n, false);
                    } else if !v(now, 11) {
                        put(&mut st.bits, n, true);
                    } else if fell(now, prev, clk) {
                        let q = bit(st.bits, n);
                        put(&mut st.bits, n, jk(v(prev, j), !v(prev, kn), q));
                    }
                }
            },
        ),

        // Octal D with asynchronous clear: CLR=1, CLK=11, and the same D and
        // Q pins as the 74S374, which it is but with a clear instead of an
        // output enable.
        "74LS273" => seq(
            const {
                &[
                    g(2, &[1], |i, s| clr_q(i, bit(s.bits, 0))),
                    g(5, &[1], |i, s| clr_q(i, bit(s.bits, 1))),
                    g(6, &[1], |i, s| clr_q(i, bit(s.bits, 2))),
                    g(9, &[1], |i, s| clr_q(i, bit(s.bits, 3))),
                    g(12, &[1], |i, s| clr_q(i, bit(s.bits, 4))),
                    g(15, &[1], |i, s| clr_q(i, bit(s.bits, 5))),
                    g(16, &[1], |i, s| clr_q(i, bit(s.bits, 6))),
                    g(19, &[1], |i, s| clr_q(i, bit(s.bits, 7))),
                ]
            },
            &[1, 3, 4, 7, 8, 11, 13, 14, 17, 18],
            |now, prev, st| {
                if !v(now, 1) {
                    st.bits = 0;
                } else if rose(now, prev, 11) {
                    st.bits = word(prev, OCTAL_D) as u16;
                }
            },
        ),

        // Synchronous four-bit binary counter with synchronous clear.
        // CLR=1, CLK=2, A-D=3-6, ENP=7, LOAD=9, ENT=10, QD-QA=11-14, RCO=15.
        // UPRIOR 0C01 ties ENP, LOAD and ENT high and uses only RCO, so it
        // is a free-running divider that says when a grant has taken too
        // long.
        // SN74161 (`sn74161.pdf`). The counter is the '163's, with one
        // difference the sheet is explicit about: *"a low level at the
        // clear input sets all four of the flip-flop outputs low regardless
        // of the levels of clock, load, or enable inputs"* --- the '161's
        // clear is direct, where the '163's waits for the edge. Presetting
        // is synchronous either way, and counting needs `ENP` and `ENT`
        // both high. `RCO` is high at 15 with `ENT` high.
        "74LS161" => seq(
            const {
                &[
                    g(14, &[], |_, s| lv(bit(s.bits, 0))),
                    g(13, &[], |_, s| lv(bit(s.bits, 1))),
                    g(12, &[], |_, s| lv(bit(s.bits, 2))),
                    g(11, &[], |_, s| lv(bit(s.bits, 3))),
                    g(15, &[10], |i, s| lv(i[0] && s.bits == 0b1111)),
                ]
            },
            &[1, 2, 3, 4, 5, 6, 7, 9, 10],
            |now, prev, st| {
                if !v(now, 1) {
                    st.bits = 0;
                } else if rose(now, prev, 2) {
                    if !v(prev, 9) {
                        st.bits = word(prev, &[3, 4, 5, 6]) as u16;
                    } else if v(prev, 7) && v(prev, 10) {
                        st.bits = (st.bits + 1) & 0b1111;
                    }
                }
            },
        ),
        "74LS163" => seq(
            const {
                &[
                    g(14, &[], |_, s| lv(bit(s.bits, 0))),
                    g(13, &[], |_, s| lv(bit(s.bits, 1))),
                    g(12, &[], |_, s| lv(bit(s.bits, 2))),
                    g(11, &[], |_, s| lv(bit(s.bits, 3))),
                    g(15, &[10], |i, s| lv(i[0] && s.bits == 0b1111)),
                ]
            },
            &[1, 2, 3, 4, 5, 6, 7, 9, 10],
            |now, prev, st| {
                if rose(now, prev, 2) {
                    if !v(prev, 1) {
                        st.bits = 0;
                    } else if !v(prev, 9) {
                        st.bits = word(prev, &[3, 4, 5, 6]) as u16;
                    } else if v(prev, 7) && v(prev, 10) {
                        st.bits = (st.bits + 1) & 0b1111;
                    }
                }
            },
        ),

        // 32x8 PROM, the same shape as the 5600/5610: `-ce` on 15, address
        // on 10-14, data out on 1-7 and 9. This is `cadr1/reqtim.prom`.
        "74S288" => comb(
            const {
                &[
                    g(1, PROM32_IN, |i, s| prom(i, s, 5, 0)),
                    g(2, PROM32_IN, |i, s| prom(i, s, 5, 1)),
                    g(3, PROM32_IN, |i, s| prom(i, s, 5, 2)),
                    g(4, PROM32_IN, |i, s| prom(i, s, 5, 3)),
                    g(5, PROM32_IN, |i, s| prom(i, s, 5, 4)),
                    g(6, PROM32_IN, |i, s| prom(i, s, 5, 5)),
                    g(7, PROM32_IN, |i, s| prom(i, s, 5, 6)),
                    g(9, PROM32_IN, |i, s| prom(i, s, 5, 7)),
                ]
            },
        ),

        // Datasheet Am26S10: quad open-collector bus transceiver. Both
        // halves invert --- "input to bus is inverting on the Am26S10" ---
        // so a one on `I` pulls the bus line low, and a low bus line reads
        // back as a one on `Z`.
        //
        // **The enable on pin 12 is active low**, which the board says
        // rather than the datasheet: all seventeen tie it to ground, and XA
        // 0F21 drives `-XBUS INIT` from `RESET` and `-XBUS SYNC` from
        // `CLK0`, which it could not do with its drivers off.
        "26S10" => comb(
            const {
                &[
                    g(2, &[12, 4], OC_DRIVE),
                    g(7, &[12, 5], OC_DRIVE),
                    g(9, &[12, 11], OC_DRIVE),
                    g(15, &[12, 13], OC_DRIVE),
                    g(3, &[2], NOT),
                    g(6, &[7], NOT),
                    g(10, &[9], NOT),
                    g(14, &[15], NOT),
                ]
            },
        ),

        // Datasheet DM8838: four open-collector driver/receiver pairs, both
        // halves inverting, which is how a Unibus line works --- the line
        // idles high and a driver pulls it low.
        //
        // The two disable pins are 7 and 9: the datasheet's "one two-input
        // NOR gate ... to disable all drivers in a package simultaneously".
        // The drawings connect both --- pin 7 to `-DRIVE.UNIBUS` on the I/O
        // board and to ground on the bus interface, pin 9 to each board's
        // own enable --- and leave pin 8, the ground, off the sheet as they
        // leave every ground pin off.
        // DM8838 (`dm8838.pdf`): four open-collector bus
        // drivers, IN 14, 11, 2, 5 onto BUS 15, 12, 1, 4, enabled together
        // while both DISABLE A (9) and DISABLE B (7) are low; four
        // inverting receivers from the bus onto OUT 13, 10, 3, 6. The
        // interface grounds B and gates on A; the I/O board the other way.
        "8838" => comb(
            const {
                &[
                    g(1, &[7, 9, 2], OC_BUS),
                    g(4, &[7, 9, 5], OC_BUS),
                    g(12, &[7, 9, 11], OC_BUS),
                    g(15, &[7, 9, 14], OC_BUS),
                    g(3, &[1], NOT),
                    g(6, &[4], NOT),
                    g(10, &[12], NOT),
                    g(13, &[15], NOT),
                ]
            },
        ),

        // Datasheet Am73/8304B: octal three-state bidirectional transceiver,
        // **non-inverting** --- the Am8303 is the inverting one. Chip
        // disable on 9 is active low and Transmit/Receive on 11 "determines
        // the direction": high sends A to B.
        //
        // The A pins are 1-8 and the B pins 19 down to 12, so A0 pairs with
        // the pin at the far corner. DBGOUT 0B22 confirms it: `UDO15` on pin
        // 1 against `DBD15` on pin 19, `UDO8` on pin 8 against `DBD8` on 12.
        "8304" => comb(
            const {
                &[
                    g(1, &[9, 11, 19], |i, _| xcv(i, false)),
                    g(2, &[9, 11, 18], |i, _| xcv(i, false)),
                    g(3, &[9, 11, 17], |i, _| xcv(i, false)),
                    g(4, &[9, 11, 16], |i, _| xcv(i, false)),
                    g(5, &[9, 11, 15], |i, _| xcv(i, false)),
                    g(6, &[9, 11, 14], |i, _| xcv(i, false)),
                    g(7, &[9, 11, 13], |i, _| xcv(i, false)),
                    g(8, &[9, 11, 12], |i, _| xcv(i, false)),
                    g(19, &[9, 11, 1], |i, _| xcv(i, true)),
                    g(18, &[9, 11, 2], |i, _| xcv(i, true)),
                    g(17, &[9, 11, 3], |i, _| xcv(i, true)),
                    g(16, &[9, 11, 4], |i, _| xcv(i, true)),
                    g(15, &[9, 11, 5], |i, _| xcv(i, true)),
                    g(14, &[9, 11, 6], |i, _| xcv(i, true)),
                    g(13, &[9, 11, 7], |i, _| xcv(i, true)),
                    g(12, &[9, 11, 8], |i, _| xcv(i, true)),
                ]
            },
        ),

        // Am29701, 16x4 three-state RAM, the Unibus map's read buffer. `-ce`
        // on 2, `-we` on 3, address on 13/14/15/1 lowest first, and four
        // data pairs: in on 4/6/10/12 and out on 5/7/9/11.
        "29701" => seq(
            const {
                &[
                    g(5, RAM16X4_IN, |i, s| ramn(i, s, 4, 0)),
                    g(7, RAM16X4_IN, |i, s| ramn(i, s, 4, 1)),
                    g(9, RAM16X4_IN, |i, s| ramn(i, s, 4, 2)),
                    g(11, RAM16X4_IN, |i, s| ramn(i, s, 4, 3)),
                ]
            },
            &[1, 2, 3, 4, 6, 10, 12, 13, 14, 15],
            |now, prev, st| {
                let writing = |p: &Pins| !v(p, 2) && !v(p, 3);
                if writing(prev) && !writing(now) {
                    let a = word(prev, RAM16X4_ADDR) as usize;
                    let d = word(prev, &[4, 6, 10, 12]) as u8;
                    if let Some(c) = st.cells.get_mut(a) {
                        *c = d;
                    }
                }
            },
        ),

        // --- buffers --------------------------------------------------------
        // Octal, in two halves of four. The '240 inverts and the '241 and
        // '244 do not; the '241's second enable is active high, which the
        // netlist confirms --- ACTL feeds pin 1 -APASSENB and pin 19 APASSENB.
        "74S240" | "74LS240" => comb(
            const {
                &[
                    g(3, &[19, 17], BUF_N),
                    g(5, &[19, 15], BUF_N),
                    g(7, &[19, 13], BUF_N),
                    g(9, &[19, 11], BUF_N),
                    g(12, &[1, 8], BUF_N),
                    g(14, &[1, 6], BUF_N),
                    g(16, &[1, 4], BUF_N),
                    g(18, &[1, 2], BUF_N),
                ]
            },
        ),
        "74S241" => comb(
            const {
                &[
                    g(3, &[19, 17], BUF_HI),
                    g(5, &[19, 15], BUF_HI),
                    g(7, &[19, 13], BUF_HI),
                    g(9, &[19, 11], BUF_HI),
                    g(12, &[1, 8], BUF),
                    g(14, &[1, 6], BUF),
                    g(16, &[1, 4], BUF),
                    g(18, &[1, 2], BUF),
                ]
            },
        ),
        "74LS244" => comb(
            const {
                &[
                    g(3, &[19, 17], BUF),
                    g(5, &[19, 15], BUF),
                    g(7, &[19, 13], BUF),
                    g(9, &[19, 11], BUF),
                    g(12, &[1, 8], BUF),
                    g(14, &[1, 6], BUF),
                    g(16, &[1, 4], BUF),
                    g(18, &[1, 2], BUF),
                ]
            },
        ),

        // --- arithmetic ------------------------------------------------------
        "74S283" => comb(
            const {
                &[
                    g(1, ADD_IN, |i, _| lv(s283(i) & 2 != 0)),
                    g(4, ADD_IN, |i, _| lv(s283(i) & 1 != 0)),
                    g(9, ADD_IN, |i, _| lv(s283(i) & 16 != 0)),
                    g(10, ADD_IN, |i, _| lv(s283(i) & 8 != 0)),
                    g(13, ADD_IN, |i, _| lv(s283(i) & 4 != 0)),
                ]
            },
        ),
        "74S181" => {
            comb(
                const {
                    &[
                        g(9, ALU_IN, |i, _| lv(alu_f(i) & 1 != 0)),
                        g(10, ALU_IN, |i, _| lv(alu_f(i) & 2 != 0)),
                        g(11, ALU_IN, |i, _| lv(alu_f(i) & 4 != 0)),
                        g(13, ALU_IN, |i, _| lv(alu_f(i) & 8 != 0)),
                        g(14, ALU_IN, |i, _| {
                            lv(s181(nib(i, 0), nib(i, 4), nib(i, 8), i[12], i[13]).2)
                        }),
                        // P and G take neither the mode nor the carry, so the mode passed
                        // here is arbitrary.
                        g(15, ALU_PGIN, |i, _| {
                            lv(s181(nib(i, 0), nib(i, 4), nib(i, 8), false, true).3)
                        }),
                        g(16, ALU_CIN, |i, _| {
                            lv(s181(nib(i, 0), nib(i, 4), nib(i, 8), false, i[12]).1)
                        }),
                        g(17, ALU_PGIN, |i, _| {
                            lv(s181(nib(i, 0), nib(i, 4), nib(i, 8), false, true).4)
                        }),
                    ]
                },
            )
        }
        // The group P and G outputs take no carry, exactly as the 74S181's
        // P and G do not. Declaring one input list for all five outputs
        // closes a loop through the second level of lookahead that the
        // hardware does not have.
        "74S182" => comb(
            const {
                &[
                    g(7, CLA_PGIN, |i, _| lv(cla(i, false).0)),
                    g(9, CLA_IN, |i, _| lv(cla(i, true).4)),
                    g(10, CLA_PGIN, |i, _| lv(cla(i, false).1)),
                    g(11, CLA_IN, |i, _| lv(cla(i, true).3)),
                    g(12, CLA_IN, |i, _| lv(cla(i, true).2)),
                ]
            },
        ),
        // Four-bit shifter, three-state.
        "25S10" => comb(
            const {
                &[
                    g(11, SHIFT_IN, |i, _| s10(i, 3)),
                    g(12, SHIFT_IN, |i, _| s10(i, 2)),
                    g(14, SHIFT_IN, |i, _| s10(i, 1)),
                    g(15, SHIFT_IN, |i, _| s10(i, 0)),
                ]
            },
        ),

        // --- flip-flops, registers, counters ---------------------------------
        // Dual D, positive edge, with asynchronous preset and clear.
        // Bit 0 is the flip-flop on pins 1-6, bit 1 the one on 8-13.
        "74S74" | "74LS74" | "74LS74I" | "LS74" | "S74" => seq(
            const {
                &[
                    g(5, &[1, 4], |i, s| ff(i, bit(s.bits, 0), false)),
                    g(6, &[1, 4], |i, s| ff(i, bit(s.bits, 0), true)),
                    g(8, &[13, 10], |i, s| ff(i, bit(s.bits, 1), true)),
                    g(9, &[13, 10], |i, s| ff(i, bit(s.bits, 1), false)),
                ]
            },
            &[1, 2, 3, 4, 10, 11, 12, 13],
            |now, prev, st| {
                // (bit, -clr, -pre, clk, d)
                for &(n, clr, pre, clk, d) in &[(0u32, 1u8, 4u8, 3u8, 2u8), (1, 13, 10, 11, 12)] {
                    if !v(now, clr) {
                        put(&mut st.bits, n, false);
                    } else if !v(now, pre) {
                        put(&mut st.bits, n, true);
                    } else if rose(now, prev, clk) {
                        put(&mut st.bits, n, v(prev, d));
                    }
                }
            },
        ),
        // Dual J-K̄, positive edge, with asynchronous preset and clear.
        "74LS109" => seq(
            const {
                &[
                    g(6, &[1, 5], |i, s| ff(i, bit(s.bits, 0), false)),
                    g(7, &[1, 5], |i, s| ff(i, bit(s.bits, 0), true)),
                    g(9, &[15, 11], |i, s| ff(i, bit(s.bits, 1), true)),
                    g(10, &[15, 11], |i, s| ff(i, bit(s.bits, 1), false)),
                ]
            },
            &[1, 2, 3, 4, 5, 11, 12, 13, 14, 15],
            |now, prev, st| {
                // (bit, -clr, -pre, clk, j, -k)
                for &(n, clr, pre, clk, j, kn) in
                    &[(0u32, 1u8, 5u8, 4u8, 2u8, 3u8), (1, 15, 11, 12, 14, 13)]
                {
                    if !v(now, clr) {
                        put(&mut st.bits, n, false);
                    } else if !v(now, pre) {
                        put(&mut st.bits, n, true);
                    } else if rose(now, prev, clk) {
                        let q = bit(st.bits, n);
                        put(&mut st.bits, n, jk(v(prev, j), !v(prev, kn), q));
                    }
                }
            },
        ),
        // Hex D with asynchronous clear; bit 0 is Q1 on pin 2.
        "74S174" => seq(
            const {
                &[
                    g(2, &[1], |i, s| clr_q(i, bit(s.bits, 0))),
                    g(5, &[1], |i, s| clr_q(i, bit(s.bits, 1))),
                    g(7, &[1], |i, s| clr_q(i, bit(s.bits, 2))),
                    g(10, &[1], |i, s| clr_q(i, bit(s.bits, 3))),
                    g(12, &[1], |i, s| clr_q(i, bit(s.bits, 4))),
                    g(15, &[1], |i, s| clr_q(i, bit(s.bits, 5))),
                ]
            },
            &[1, 3, 4, 6, 9, 11, 13, 14],
            |now, prev, st| {
                if !v(now, 1) {
                    st.bits = 0;
                } else if rose(now, prev, 9) {
                    st.bits = word(prev, &[3, 4, 6, 11, 13, 14]) as u16;
                }
            },
        ),
        // Quad D with asynchronous clear, true and complement out.
        "74S175" => seq(
            const {
                &[
                    g(2, &[1], |i, s| clr_q(i, bit(s.bits, 0))),
                    g(3, &[1], |i, s| clr_qn(i, bit(s.bits, 0))),
                    g(6, &[1], |i, s| clr_qn(i, bit(s.bits, 1))),
                    g(7, &[1], |i, s| clr_q(i, bit(s.bits, 1))),
                    g(10, &[1], |i, s| clr_q(i, bit(s.bits, 2))),
                    g(11, &[1], |i, s| clr_qn(i, bit(s.bits, 2))),
                    g(14, &[1], |i, s| clr_qn(i, bit(s.bits, 3))),
                    g(15, &[1], |i, s| clr_q(i, bit(s.bits, 3))),
                ]
            },
            &[1, 4, 5, 9, 12, 13],
            |now, prev, st| {
                if !v(now, 1) {
                    st.bits = 0;
                } else if rose(now, prev, 9) {
                    st.bits = word(prev, &[4, 5, 12, 13]) as u16;
                }
            },
        ),
        // Four-bit bidirectional shift register. Bit 0 is QA on pin 15.
        //
        // The datasheet's "shift right" moves QA to QB with the pin 2 serial
        // input entering QA, and the board agrees: on Q 2C07 pin 2 carries
        // Q23, the bit below QA=Q24. The two directions are easy to swap;
        // the board's own wiring is what settles them.
        "74S194" => seq(
            const {
                &[
                    g(12, &[1], |i, s| clr_q(i, bit(s.bits, 3))),
                    g(13, &[1], |i, s| clr_q(i, bit(s.bits, 2))),
                    g(14, &[1], |i, s| clr_q(i, bit(s.bits, 1))),
                    g(15, &[1], |i, s| clr_q(i, bit(s.bits, 0))),
                ]
            },
            &[1, 2, 3, 4, 5, 6, 7, 9, 10, 11],
            |now, prev, st| {
                if !v(now, 1) {
                    st.bits = 0;
                } else if rose(now, prev, 11) {
                    let q = st.bits & 0xf;
                    st.bits = match (v(prev, 10), v(prev, 9)) {
                        (false, false) => q,
                        (false, true) => (q << 1 & 0xf) | v(prev, 2) as u16,
                        (true, false) => (q >> 1) | (v(prev, 7) as u16) << 3,
                        (true, true) => word(prev, &[3, 4, 5, 6]) as u16,
                    };
                }
            },
        ),
        // Synchronous four-bit up/down counter. Bit 0 is Q0 on pin 14.
        //
        // The ripple carry takes ENT but not ENP: the SN74S169 datasheet's
        // logic diagram gates `RCO` with `ENT` alone. Every one of the
        // nineteen here has ENP grounded, so nothing on the board can tell
        // the two apart.
        "74S169" => seq(
            const {
                &[
                    g(11, &[], |_, s| lv(bit(s.bits, 3))),
                    g(12, &[], |_, s| lv(bit(s.bits, 2))),
                    g(13, &[], |_, s| lv(bit(s.bits, 1))),
                    g(14, &[], |_, s| lv(bit(s.bits, 0))),
                    // -rco: up/down on 1, -ent on 10.
                    g(15, &[1, 10], |i, s| {
                        let terminal = if i[0] { s.bits & 0xf == 0xf } else { s.bits & 0xf == 0 };
                        let enabled = !i[1];
                        lv(!(terminal && enabled))
                    }),
                ]
            },
            &[1, 2, 3, 4, 5, 6, 7, 9, 10],
            |now, prev, st| {
                if rose(now, prev, 2) {
                    st.bits = if !v(prev, 9) {
                        word(prev, &[3, 4, 5, 6]) as u16
                    } else if !v(prev, 7) && !v(prev, 10) {
                        let step = if v(prev, 1) { 1 } else { 15 };
                        (st.bits + step) & 0xf
                    } else {
                        st.bits
                    };
                }
            },
        ),
        // Octal transparent latch, three-state. Bit 0 is the pin 3 input and
        // the pin 2 output.
        //
        // While pin 11 is high the latch is *transparent*, and a transparent
        // latch is not a latch --- it is a wire. So the data pin is one of
        // the gate's inputs and the stored bit only decides the output once
        // pin 11 goes low. Modelling the data path through the state instead
        // makes it a tick late, which on the A and M buses is the difference
        // between the machine working and not.
        "74S373" => seq(
            const {
                &[
                    g(2, &[1, 11, 3], |i, s| latch(i, bit(s.bits, 0))),
                    g(5, &[1, 11, 4], |i, s| latch(i, bit(s.bits, 1))),
                    g(6, &[1, 11, 7], |i, s| latch(i, bit(s.bits, 2))),
                    g(9, &[1, 11, 8], |i, s| latch(i, bit(s.bits, 3))),
                    g(12, &[1, 11, 13], |i, s| latch(i, bit(s.bits, 4))),
                    g(15, &[1, 11, 14], |i, s| latch(i, bit(s.bits, 5))),
                    g(16, &[1, 11, 17], |i, s| latch(i, bit(s.bits, 6))),
                    g(19, &[1, 11, 18], |i, s| latch(i, bit(s.bits, 7))),
                ]
            },
            &[3, 4, 7, 8, 11, 13, 14, 17, 18],
            |now, _prev, st| {
                if v(now, 11) {
                    st.bits = word(now, OCTAL_D) as u16;
                }
            },
        ),
        // Octal D, three-state. Same pin pairs as the '373, clocked on 11.
        "74S374" | "74LS374" => seq(
            const {
                &[
                    g(2, &[1], |i, s| oe(i, bit(s.bits, 0))),
                    g(5, &[1], |i, s| oe(i, bit(s.bits, 1))),
                    g(6, &[1], |i, s| oe(i, bit(s.bits, 2))),
                    g(9, &[1], |i, s| oe(i, bit(s.bits, 3))),
                    g(12, &[1], |i, s| oe(i, bit(s.bits, 4))),
                    g(15, &[1], |i, s| oe(i, bit(s.bits, 5))),
                    g(16, &[1], |i, s| oe(i, bit(s.bits, 6))),
                    g(19, &[1], |i, s| oe(i, bit(s.bits, 7))),
                ]
            },
            &[3, 4, 7, 8, 11, 13, 14, 17, 18],
            |now, prev, st| {
                if rose(now, prev, 11) {
                    st.bits = word(prev, OCTAL_D) as u16;
                }
            },
        ),
        // Octal D with a clock enable, totem-pole outputs: the '374's D and
        // Q pins exactly, with pin 1 an enable on the clock instead of on
        // the outputs, so a high there holds the register rather than
        // releasing the bus.
        "74LS377" => seq(
            const {
                &[
                    g(2, &[], |_, s| lv(bit(s.bits, 0))),
                    g(5, &[], |_, s| lv(bit(s.bits, 1))),
                    g(6, &[], |_, s| lv(bit(s.bits, 2))),
                    g(9, &[], |_, s| lv(bit(s.bits, 3))),
                    g(12, &[], |_, s| lv(bit(s.bits, 4))),
                    g(15, &[], |_, s| lv(bit(s.bits, 5))),
                    g(16, &[], |_, s| lv(bit(s.bits, 6))),
                    g(19, &[], |_, s| lv(bit(s.bits, 7))),
                ]
            },
            &[1, 3, 4, 7, 8, 11, 13, 14, 17, 18],
            |now, prev, st| {
                if rose(now, prev, 11) && !v(prev, 1) {
                    st.bits = word(prev, OCTAL_D) as u16;
                }
            },
        ),
        // Eight-bit universal shift/storage register with multiplexed
        // input/output ports. Bit 0 is QA, whose port is pin 7 and whose
        // buffered copy `QA'` is pin 8; QH' is pin 17. S1 on 19 and S0 on 1
        // choose hold, shift right, shift left or load, as the datasheet's
        // function table has them; the clear on 9 is asynchronous.
        //
        // Shift right moves QA into QB with `SR` p11 entering QA, and shift
        // left moves QH into QG with `SL` p18 entering QH --- the
        // datasheet's own directions, which are the '194's.
        "74S299" => seq(
            const {
                &[
                    g(7, &[2, 3, 9], |i, s| port299(i, bit(s.bits, 0))),
                    g(13, &[2, 3, 9], |i, s| port299(i, bit(s.bits, 1))),
                    g(6, &[2, 3, 9], |i, s| port299(i, bit(s.bits, 2))),
                    g(14, &[2, 3, 9], |i, s| port299(i, bit(s.bits, 3))),
                    g(5, &[2, 3, 9], |i, s| port299(i, bit(s.bits, 4))),
                    g(15, &[2, 3, 9], |i, s| port299(i, bit(s.bits, 5))),
                    g(4, &[2, 3, 9], |i, s| port299(i, bit(s.bits, 6))),
                    g(16, &[2, 3, 9], |i, s| port299(i, bit(s.bits, 7))),
                    g(8, &[9], |i, s| clr_q(i, bit(s.bits, 0))),
                    g(17, &[9], |i, s| clr_q(i, bit(s.bits, 7))),
                ]
            },
            &[1, 4, 5, 6, 7, 9, 11, 12, 13, 14, 15, 16, 18, 19],
            |now, prev, st| {
                if !v(now, 9) {
                    st.bits = 0;
                } else if rose(now, prev, 12) {
                    let q = st.bits & 0xff;
                    st.bits = match (v(prev, 19), v(prev, 1)) {
                        (false, false) => q,
                        (false, true) => (q << 1 & 0xff) | v(prev, 11) as u16,
                        (true, false) => (q >> 1) | (v(prev, 18) as u16) << 7,
                        (true, true) => word(prev, PORTS299) as u16,
                    };
                }
            },
        ),

        // Six-bit register with a clock enable on pin 1. No clear.
        "25S07" => seq(
            const {
                &[
                    g(2, &[], |_, s| lv(bit(s.bits, 0))),
                    g(5, &[], |_, s| lv(bit(s.bits, 1))),
                    g(7, &[], |_, s| lv(bit(s.bits, 2))),
                    g(10, &[], |_, s| lv(bit(s.bits, 3))),
                    g(12, &[], |_, s| lv(bit(s.bits, 4))),
                    g(15, &[], |_, s| lv(bit(s.bits, 5))),
                ]
            },
            &[1, 3, 4, 6, 9, 11, 13, 14],
            |now, prev, st| {
                if !v(prev, 1) && rose(now, prev, 9) {
                    st.bits = word(prev, &[3, 4, 6, 11, 13, 14]) as u16;
                }
            },
        ),
        // Quad register with a two-to-one select in front of every input.
        "25S09" => seq(
            const {
                &[
                    g(2, &[], |_, s| lv(bit(s.bits, 0))),
                    g(7, &[], |_, s| lv(bit(s.bits, 1))),
                    g(10, &[], |_, s| lv(bit(s.bits, 2))),
                    g(15, &[], |_, s| lv(bit(s.bits, 3))),
                ]
            },
            &[1, 3, 4, 5, 6, 9, 11, 12, 13, 14],
            |now, prev, st| {
                if rose(now, prev, 9) {
                    let pins: [[u8; 2]; 4] = [[3, 4], [6, 5], [11, 12], [14, 13]];
                    st.bits = 0;
                    for (n, pair) in pins.iter().enumerate() {
                        put(&mut st.bits, n as u32, v(prev, pair[v(prev, 1) as usize]));
                    }
                }
            },
        ),
        // Quad register with two separately enabled three-state output sets.
        // W is inverted when pin 18 is high.
        "25LS2519" => seq(
            const {
                &[
                    g(2, &[7, 18], |i, s| oe_pol(i, bit(s.bits, 0))),
                    g(3, &[8], |i, s| oe(i, bit(s.bits, 0))),
                    g(5, &[7, 18], |i, s| oe_pol(i, bit(s.bits, 1))),
                    g(6, &[8], |i, s| oe(i, bit(s.bits, 1))),
                    g(11, &[8], |i, s| oe(i, bit(s.bits, 2))),
                    g(12, &[7, 18], |i, s| oe_pol(i, bit(s.bits, 2))),
                    g(14, &[8], |i, s| oe(i, bit(s.bits, 3))),
                    g(15, &[7, 18], |i, s| oe_pol(i, bit(s.bits, 3))),
                ]
            },
            &[1, 4, 9, 13, 16, 17, 19],
            |now, prev, st| {
                if !v(now, 19) {
                    st.bits = 0;
                } else if !v(prev, 17) && rose(now, prev, 9) {
                    st.bits = word(prev, &[1, 4, 13, 16]) as u16;
                }
            },
        ),
        // Dual eight-bit shift register. Bits 0-7 are the A register, low
        // end first, so Q7 is bit 7; bits 8-15 are the B register.
        //
        // Each register is clocked by the OR of its own clock and the common
        // one on pin 9, which is how the board uses the separate clocks as
        // inhibits: `-OPCINH` sits on pins 7 and 10, `OPCCLK` on pin 9.
        // Dropping pin 9 loses the inhibit altogether.
        "9328" => seq(
            const {
                &[
                    g(2, &[1], |i, s| clr_qn(i, bit(s.bits, 7))),
                    g(3, &[1], |i, s| clr_q(i, bit(s.bits, 7))),
                    g(14, &[1], |i, s| clr_q(i, bit(s.bits, 15))),
                    g(15, &[1], |i, s| clr_qn(i, bit(s.bits, 15))),
                ]
            },
            &[1, 4, 5, 6, 7, 9, 10, 11, 12, 13],
            |now, prev, st| {
                if !v(now, 1) {
                    st.bits = 0;
                    return;
                }
                // (low bit, own clock, select, d0, d1)
                for &(base, clk, sel, d0, d1) in &[(0u32, 7u8, 4u8, 6u8, 5u8), (8, 10, 13, 11, 12)]
                {
                    let ck = |p: &Pins| v(p, clk) || v(p, 9);
                    if !ck(prev) && ck(now) {
                        let d = v(prev, if v(prev, sel) { d1 } else { d0 });
                        let reg = (st.bits >> base) & 0xff;
                        let reg = (reg << 1 & 0xff) | d as u16;
                        st.bits = (st.bits & !(0xff << base)) | reg << base;
                    }
                }
            },
        ),

        // --- memories --------------------------------------------------------
        // 32x2 RAM, open collector, with an output latch on pin 6. Cell n
        // holds bit 0 of the word in bit 0 and bit 1 in bit 1; the latch is
        // state bits 0 and 1.
        "82S21" => seq(
            const { &[g(7, RAM32_IN, |i, s| ram32(i, s, 0)), g(9, RAM32_IN, |i, s| ram32(i, s, 1))] },
            &[1, 2, 3, 4, 5, 6, 10, 11, 12, 13, 14, 15],
            |now, prev, st| {
                if fell(now, prev, 6) {
                    let addr = word(prev, &[13, 12, 11, 10, 4]) as usize;
                    st.bits = *st.cells.get(addr).unwrap_or(&0) as u16;
                }
                // Written at the end of the pulse: the datasheet lets data
                // arrive after it starts, but requires it held past the end.
                let writing = |p: &Pins, we: u8| v(p, 5) && !v(p, 1) && !v(p, we);
                for (n, we, di) in [(0, 2u8, 3u8), (1, 15, 14)] {
                    if writing(prev, we) && !writing(now, we) {
                        let a = word(prev, &[13, 12, 11, 10, 4]) as usize;
                        if let Some(cell) = st.cells.get_mut(a) {
                            *cell = (*cell & !(1 << n)) | (v(prev, di) as u8) << n;
                        }
                    }
                }
            },
        ),
        // 4096x1 RAM, three-state. The output is high impedance while the
        // part is deselected *or* being written: the 2147 datasheet's truth
        // table has `DOUT` at high Z in both rows.
        "2147" => seq(
            const { &[g(7, RAM4K_IN, |i, s| ram1(i, s, 12))] },
            &[1, 2, 3, 4, 5, 6, 8, 10, 11, 12, 13, 14, 15, 16, 17],
            |now, prev, st| ram_write(now, prev, st, RAM4K_ADDR, 10, 8, 11),
        ),
        // 1024x1 RAM, three-state.
        "93425" => seq(
            const { &[g(7, RAM1K_IN, |i, s| ram1(i, s, 10))] },
            &[1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14, 15],
            |now, prev, st| ram_write(now, prev, st, RAM1K_ADDR, 1, 14, 15),
        ),
        // 512x8 PROM, three-state. The contents come from outside.
        "74S472" => comb(
            const {
                &[
                    g(6, PROM512_IN, |i, s| prom(i, s, 9, 0)),
                    g(7, PROM512_IN, |i, s| prom(i, s, 9, 1)),
                    g(8, PROM512_IN, |i, s| prom(i, s, 9, 2)),
                    g(9, PROM512_IN, |i, s| prom(i, s, 9, 3)),
                    g(11, PROM512_IN, |i, s| prom(i, s, 9, 4)),
                    g(12, PROM512_IN, |i, s| prom(i, s, 9, 5)),
                    g(13, PROM512_IN, |i, s| prom(i, s, 9, 6)),
                    g(14, PROM512_IN, |i, s| prom(i, s, 9, 7)),
                ]
            },
        ),
        // 32x8 PROM. The 5600 is the open-collector part and the 5610 the
        // three-state one; only the drive differs, so the logic is shared.
        "5600" | "5610" => comb(
            const {
                &[
                    g(1, PROM32_IN, |i, s| prom(i, s, 5, 0)),
                    g(2, PROM32_IN, |i, s| prom(i, s, 5, 1)),
                    g(3, PROM32_IN, |i, s| prom(i, s, 5, 2)),
                    g(4, PROM32_IN, |i, s| prom(i, s, 5, 3)),
                    g(5, PROM32_IN, |i, s| prom(i, s, 5, 4)),
                    g(6, PROM32_IN, |i, s| prom(i, s, 5, 5)),
                    g(7, PROM32_IN, |i, s| prom(i, s, 5, 6)),
                    g(9, PROM32_IN, |i, s| prom(i, s, 5, 7)),
                ]
            },
        ),

        // --- passives, and the display ---------------------------------------
        // The TIL309 drives its own LEDs and nothing on the board, so it
        // computes nothing this side of the glass. Its sheet
        // (`til309.pdf`) gives a 16-pin part: latch data `A` 15, `B` 10,
        // `C` 6, `D` 7, `DP` 12; latch outputs `QA` 4, `QB` 1, `QC` 2,
        // `QD` 3, `QDP` 14; latch strobe 5, blanking 11, lamp test 13,
        // ground 8 and supply 16. The five bodies here do not use that
        // numbering --- each carries a pin 17, which a 16-pin part has not
        // got --- so SUDS numbers this body its own way and the netlist's
        // pins cannot be read against the sheet one for one. Nothing is
        // lost by it: every net on them is a digit of `PC`, a supply or a
        // status lamp (`TILT1`, `TILT0`, `DPE`, `IPE`, `PROMENABLE`) going
        // *in* to be displayed, and no net on the board is driven by one.
        // `BUSBAR` is the display board's laminate power strip and
        // `898-3-R22` its series damping pack, whose two ends
        // `src/netlist.rs` has already joined; neither computes anything.
        "TIL309" | "16DUMMY" | "DUMMY4" | "SERRESL" | "CAP1" | "RES1" | "1UFCAP" | ".1UFCAP"
        | "BUSBAR" | "898-3-R22" => comb(const { &[] }),

        // The resistor packs offer a high on every resistor pin; [`resolve`]
        // is what makes it weak enough for any real driver to win.
        "RES" => comb(const { &[g(1, &[2], |i, _| lv(i[0])), g(2, &[1], |i, _| lv(i[0]))] }),
        "RES20" => comb(
            const {
                &[
                    g(2, &[], HIGH),
                    g(3, &[], HIGH),
                    g(4, &[], HIGH),
                    g(5, &[], HIGH),
                    g(6, &[], HIGH),
                    g(7, &[], HIGH),
                    g(8, &[], HIGH),
                    g(9, &[], HIGH),
                    g(10, &[], HIGH),
                    g(11, &[], HIGH),
                    g(12, &[], HIGH),
                    g(13, &[], HIGH),
                    g(14, &[], HIGH),
                    g(15, &[], HIGH),
                    g(16, &[], HIGH),
                    g(17, &[], HIGH),
                    g(18, &[], HIGH),
                    g(19, &[], HIGH),
                ]
            },
        ),
        "SIP220/330-8" | "SIP330/470-8" | "SIP180/390-8" => comb(
            const {
                &[
                    g(2, &[], HIGH),
                    g(3, &[], HIGH),
                    g(4, &[], HIGH),
                    g(5, &[], HIGH),
                    g(6, &[], HIGH),
                    g(7, &[], HIGH),
                ]
            },
        ),

        // --- the memory board ---

        // MK4116 (`mk4116.pdf`: DIN 2, WRITE 3, RAS 4, A0..A6
        // on 5, 7, 6, 12, 11, 10, 13, DOUT 14, CAS 15; access 135 ns from
        // CAS, which the board's cycle leaves room for before `XACK`, so
        // the word is taken as there at once). `-RAS` falling latches the
        // row off `A0..A6`, `-CAS`
        // falling the column, and with `-WE` low at that edge --- the board
        // sets `-WRITE` before the cycle, an early write --- writes `DIN`;
        // otherwise the cycle is a read and `DOUT` carries the bit while
        // `-CAS` is low. A RAS-only cycle is a refresh, and the cells here
        // never forget, so it does nothing. **The access time is zero**:
        // the bit is on `DOUT` the moment `-CAS` falls, where the part
        // takes tCAC; the board's own chain paces the cycle and this is
        // the same choice `IDEAL_DEVICE_NS` was.
        // Bits: 0..6 the row, 7..13 the column, 14 set while a read is
        // open on `DOUT`.
        "4116VG" => seq(
            const {
                &[g(14, &[15], |i, s| {
                    if i[0] || !bit(s.bits, 14) {
                        return Level::Z;
                    }
                    dram_bit(s, (s.bits & 0x3fff) as usize)
                })]
            },
            &[2, 3, 4, 5, 6, 7, 10, 11, 12, 13, 15],
            |now, prev, st| {
                if fell(now, prev, 4) {
                    let row = word(prev, DRAM_ADDR) as u16;
                    st.bits = (st.bits & !0x7f) | row;
                }
                if fell(now, prev, 15) && !v(now, 4) {
                    let col = word(prev, DRAM_ADDR) as u16;
                    st.bits = (st.bits & !(0x7f << 7)) | (col << 7);
                    let a = (st.bits & 0x3fff) as usize;
                    if !v(prev, 3) {
                        if let Some(c) = st.cells.get_mut(a / 8) {
                            let m = 1u8 << (a % 8);
                            *c = if v(prev, 2) { *c | m } else { *c & !m };
                        }
                        put(&mut st.bits, 14, false);
                    } else {
                        put(&mut st.bits, 14, true);
                    }
                }
                if v(now, 15) {
                    put(&mut st.bits, 14, false);
                }
            },
        ),
        // DM8136, six-bit bus comparator with an open-collector output
        // (`dm8136.pdf`, the DM7136 sheet): while the strobe
        // on 7 is low the output on 9 lets go when the six pairs B/T match
        // --- 1/2, 3/4, 5/6, 15/14, 13/12, 11/10 --- and pulls low
        // otherwise; while the strobe is high it holds what it last showed.
        // `memads` grounds the strobe.
        "8136" => seq(
            const {
                &[g(9, &[7, 1, 2, 3, 4, 5, 6, 15, 14, 13, 12, 11, 10], |i, s| {
                    let equal = (1..13).step_by(2).all(|k| i[k] == i[k + 1]);
                    lv(if i[0] { bit(s.bits, 0) } else { equal })
                })]
            },
            &[1, 2, 3, 4, 5, 6, 7, 10, 11, 12, 13, 14, 15],
            |now, _prev, st| {
                if !v(now, 7) {
                    let pairs = [(1u8, 2u8), (3, 4), (5, 6), (15, 14), (13, 12), (11, 10)];
                    let equal = pairs.iter().all(|&(b, t)| v(now, b) == v(now, t));
                    put(&mut st.bits, 0, equal);
                }
            },
        ),
        // Am25LS2539, dual one-of-four decoder with three-state outputs and
        // polarity control (`am25ls2539.pdf`): with `-OE`
        // low the outputs are on; with `E` high all four are the polarity
        // input; else the one `B A` names is the opposite of the polarity
        // input and the rest are it. Decoder 1 is A 6, B 7, E 15, POL 13,
        // -OE 14, Y0..Y3 on 12, 11, 9, 8; decoder 2 is A 17, B 18, E 16,
        // POL 4, -OE 5, Y0..Y3 on 3, 2, 1, 19. `memras` uses decoder 2 with
        // E and POL both `REFRESH CYC`, so a refresh selects every bank.
        "25LS2539" => comb(
            const {
                &[
                    g(12, &[14, 15, 13, 6, 7], |i, _| decode4(i, 0)),
                    g(11, &[14, 15, 13, 6, 7], |i, _| decode4(i, 1)),
                    g(9, &[14, 15, 13, 6, 7], |i, _| decode4(i, 2)),
                    g(8, &[14, 15, 13, 6, 7], |i, _| decode4(i, 3)),
                    g(3, &[5, 16, 4, 17, 18], |i, _| decode4(i, 0)),
                    g(2, &[5, 16, 4, 17, 18], |i, _| decode4(i, 1)),
                    g(1, &[5, 16, 4, 17, 18], |i, _| decode4(i, 2)),
                    g(19, &[5, 16, 4, 17, 18], |i, _| decode4(i, 3)),
                ]
            },
        ),
        // Am25LS2536 (`am25ls2536.pdf`), eight-bit decoder
        // with control storage. The select `A`, `B`, `C` (4, 5, 6) and the
        // polarity `POL` (7) enter a register on the rise of `CP` (1) while
        // `-CE` (3) is low, and `-CLR` (2) low sets the register low at
        // once. `G` is `-G1` (19) low and `G2` (18) high; the selected
        // output is `-G` exclusive-or `Q_POL`, every other output is
        // `Q_POL` inverted, and `-OE` (8) high turns all eight off. Bits
        // 0..2 of the state are the select, bit 3 the polarity.
        //
        // The sheet's pad layout on its last page has `CLR` on 1 and `CP`
        // on 2, against its own connection diagram and logic symbol; MIT's
        // drawing puts `CLK.SR^` on 1 and `-XINIT` on 2, which is the
        // connection diagram's way, and the way here.
        "25LS2536" => seq(
            const {
                &[
                    g(9, &[19, 18, 8], |i, s| decode2536(i, s, 0)),
                    g(11, &[19, 18, 8], |i, s| decode2536(i, s, 1)),
                    g(12, &[19, 18, 8], |i, s| decode2536(i, s, 2)),
                    g(13, &[19, 18, 8], |i, s| decode2536(i, s, 3)),
                    g(14, &[19, 18, 8], |i, s| decode2536(i, s, 4)),
                    g(15, &[19, 18, 8], |i, s| decode2536(i, s, 5)),
                    g(16, &[19, 18, 8], |i, s| decode2536(i, s, 6)),
                    g(17, &[19, 18, 8], |i, s| decode2536(i, s, 7)),
                ]
            },
            &[1, 2, 3, 4, 5, 6, 7],
            |now, prev, st| {
                if !v(now, 2) {
                    st.bits = 0;
                } else if rose(now, prev, 1) && !v(prev, 3) {
                    st.bits = word(prev, &[4, 5, 6, 7]) as u16;
                }
            },
        ),
        // SN74LS569A (`sn74ls569.pdf`). Bits 0..3 are the
        // count. On the clock's (2) rise: clear if `-SCLR` (9) is low, load
        // `A..D` (3..6) if `-LOAD` (11) is low, count if `-CEP` (7) and
        // `-CET` (12) are both low, up with `U/-D` (1) high. `-ACLR` (8)
        // clears at once. The outputs are enabled by `-OE` (17); `-RCO`
        // (19) is low at the terminal count with `-CET` low; and `CCO`
        // (18), the clocked carry, "while counting and RCO is LOW, will
        // follow the clock HIGH-LOW-HIGH transition" --- counting being
        // `-CEP` and `-CET` low with `-LOAD` (11) high. The memory board
        // leaves it open; the disk controller's `END PAGE CLK` is the
        // CCO of the word counter at DCCCW 0E22, and its memory side is
        // busy for ever without it.
        "74LS569" => seq(
            const {
                &[
                    g(16, &[17], |i, s| if i[0] { Level::Z } else { lv(bit(s.bits, 0)) }),
                    g(15, &[17], |i, s| if i[0] { Level::Z } else { lv(bit(s.bits, 1)) }),
                    g(14, &[17], |i, s| if i[0] { Level::Z } else { lv(bit(s.bits, 2)) }),
                    g(13, &[17], |i, s| if i[0] { Level::Z } else { lv(bit(s.bits, 3)) }),
                    g(19, &[12, 1], |i, s| {
                        let q = s.bits & 0xf;
                        lv(!(!i[0] && if i[1] { q == 15 } else { q == 0 }))
                    }),
                    g(18, &[7, 12, 11, 1, 2], |i, s| {
                        let q = s.bits & 0xf;
                        let terminal = if i[3] { q == 15 } else { q == 0 };
                        lv(!(!i[0] && !i[1] && i[2] && terminal && !i[4]))
                    }),
                ]
            },
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 12],
            |now, prev, st| {
                if !v(now, 8) {
                    st.bits &= !0xf;
                } else if rose(now, prev, 2) {
                    let q = st.bits & 0xf;
                    let next = if !v(prev, 9) {
                        0
                    } else if !v(prev, 11) {
                        word(prev, &[3, 4, 5, 6]) as u16
                    } else if !v(prev, 7) && !v(prev, 12) {
                        if v(prev, 1) { (q + 1) & 0xf } else { q.wrapping_sub(1) & 0xf }
                    } else {
                        q
                    };
                    st.bits = (st.bits & !0xf) | next;
                }
            },
        ),
        // Quad two-to-one multiplexer, three-state: 1 low selects the A
        // inputs, 15 high turns the outputs off.
        "74LS257" => comb(
            const {
                &[
                    g(4, &[15, 1, 2, 3], MUX2),
                    g(7, &[15, 1, 5, 6], MUX2),
                    g(9, &[15, 1, 11, 10], MUX2),
                    g(12, &[15, 1, 14, 13], MUX2),
                ]
            },
        ),
        // The Chaosnet address switch: bit `k` of the state closes the
        // switch that joins pin `9 + k` to pin `16 - k`, which the drawings
        // ground. Open, the pin drives nothing and the pull-up pack holds
        // the bit high. `Chip::set_switches` sets the address.
        "SWITCH" => comb(
            const {
                &[
                    g(9, &[], |_, s| lv(!bit(s.bits, 0))),
                    g(10, &[], |_, s| lv(!bit(s.bits, 1))),
                    g(11, &[], |_, s| lv(!bit(s.bits, 2))),
                    g(12, &[], |_, s| lv(!bit(s.bits, 3))),
                    g(13, &[], |_, s| lv(!bit(s.bits, 4))),
                    g(14, &[], |_, s| lv(!bit(s.bits, 5))),
                    g(15, &[], |_, s| lv(!bit(s.bits, 6))),
                    g(16, &[], |_, s| lv(!bit(s.bits, 7))),
                ]
            },
        ),
        // A DIP switch: bit k of the state closes the switch on pin 9+k to
        // ground. Open, the pin drives nothing.
        "DIPSW" => comb(
            const {
                &[
                    g(9, &[], |_, s| lv(!bit(s.bits, 0))),
                    g(10, &[], |_, s| lv(!bit(s.bits, 1))),
                    g(11, &[], |_, s| lv(!bit(s.bits, 2))),
                    g(12, &[], |_, s| lv(!bit(s.bits, 3))),
                    g(13, &[], |_, s| lv(!bit(s.bits, 4))),
                    g(14, &[], |_, s| lv(!bit(s.bits, 5))),
                ]
            },
        ),
        // SN74279 (`sn74279.pdf`): a -S low sets, -R low
        // resets, both low sets, both high holds. Latches 1 and 3 have two
        // -S inputs. Bits 0..3 are the four Q.
        "74279" => seq(
            const {
                &[
                    g(4, &[], |_, s| lv(bit(s.bits, 0))),
                    g(7, &[], |_, s| lv(bit(s.bits, 1))),
                    g(9, &[], |_, s| lv(bit(s.bits, 2))),
                    g(13, &[], |_, s| lv(bit(s.bits, 3))),
                ]
            },
            &[1, 2, 3, 5, 6, 10, 11, 12, 14, 15],
            |now, _prev, st| {
                let latches: [(u32, &[u8], u8); 4] =
                    [(0, &[2, 3], 1), (1, &[6], 5), (2, &[11, 12], 10), (3, &[15], 14)];
                for (n, sets, reset) in latches {
                    if sets.iter().any(|&s| !v(now, s)) {
                        put(&mut st.bits, n, true);
                    } else if !v(now, reset) {
                        put(&mut st.bits, n, false);
                    }
                }
            },
        ),
        // SN74393 (`sn74393.pdf`): two 4-bit binary counters, each counting
        // on its clock's fall --- 1A on 1, 2A on 13 --- and cleared while
        // its CLR --- 2, 12 --- is high. Bits 0..3 and 4..7.
        "74393" => seq(
            const {
                &[
                    g(3, &[], |_, s| lv(bit(s.bits, 0))),
                    g(4, &[], |_, s| lv(bit(s.bits, 1))),
                    g(5, &[], |_, s| lv(bit(s.bits, 2))),
                    g(6, &[], |_, s| lv(bit(s.bits, 3))),
                    g(11, &[], |_, s| lv(bit(s.bits, 4))),
                    g(10, &[], |_, s| lv(bit(s.bits, 5))),
                    g(9, &[], |_, s| lv(bit(s.bits, 6))),
                    g(8, &[], |_, s| lv(bit(s.bits, 7))),
                ]
            },
            &[1, 2, 12, 13],
            |now, prev, st| {
                for (shift, clock, clear) in [(0u16, 1u8, 2u8), (4, 13, 12)] {
                    let q = (st.bits >> shift) & 0xf;
                    let next = if v(now, clear) {
                        0
                    } else if fell(now, prev, clock) {
                        (q + 1) & 0xf
                    } else {
                        q
                    };
                    st.bits = (st.bits & !(0xf << shift)) | (next << shift);
                }
            },
        ),
        // SN74165 (`sn74165.pdf`): "8-bit serial shift registers that shift
        // the data in the direction of QA toward QH when clocked. Parallel-in
        // access to each stage is made available by eight individual, direct
        // data inputs that are enabled by a low level at the shift/load
        // (SH/-LD) input." The load is direct, not clocked: "Data at the
        // parallel inputs are loaded directly into the register while SH/-LD
        // is low, independently of the levels of CLK, CLK INH, or serial
        // (SER) inputs."
        //
        // The two clock pins are a positive-NOR pair, so "holding either
        // clock input high inhibits clocking"; the board grounds `CLK INH`
        // at 15 on both bodies and clocks pin 2 from `TICLK^`, MIT's `^`
        // marking a rising edge, so this shifts on the rise of 2 while 15
        // is low. Bits 0 to 7 of the state are stages A to H, `QH` being
        // stage H, and a shift moves each stage toward H with `SER` into A.
        "74165" => seq(
            const { &[g(9, &[], |_, s| lv(bit(s.bits, 7))), g(7, &[], |_, s| lv(!bit(s.bits, 7)))] },
            &[1, 2, 3, 4, 5, 6, 10, 11, 12, 13, 14, 15],
            |now, prev, st| {
                if !v(now, 1) {
                    // SH/-LD low: A..D on 11..14, E..H on 3..6, straight in.
                    st.bits = word(now, &[11, 12, 13, 14, 3, 4, 5, 6]) as u16;
                } else if rose(now, prev, 2) && !v(now, 15) {
                    st.bits = ((st.bits << 1) & 0xff) | v(prev, 10) as u16;
                }
            },
        ),
        // SN74LS164 (`sn74164.pdf`): on the clock's (8) rise QA takes A and
        // B (1, 2) and the rest shift along; -CLR (9) low clears. QA..QH
        // are bits 0..7 on 3, 4, 5, 6, 10, 11, 12, 13.
        "74LS164" => seq(
            const {
                &[
                    g(3, &[], |_, s| lv(bit(s.bits, 0))),
                    g(4, &[], |_, s| lv(bit(s.bits, 1))),
                    g(5, &[], |_, s| lv(bit(s.bits, 2))),
                    g(6, &[], |_, s| lv(bit(s.bits, 3))),
                    g(10, &[], |_, s| lv(bit(s.bits, 4))),
                    g(11, &[], |_, s| lv(bit(s.bits, 5))),
                    g(12, &[], |_, s| lv(bit(s.bits, 6))),
                    g(13, &[], |_, s| lv(bit(s.bits, 7))),
                ]
            },
            &[1, 2, 8, 9],
            |now, prev, st| {
                if !v(now, 9) {
                    st.bits &= !0xff;
                } else if rose(now, prev, 8) {
                    let shifted = ((st.bits & 0x7f) << 1) | (v(prev, 1) && v(prev, 2)) as u16;
                    st.bits = (st.bits & !0xff) | shifted;
                }
            },
        ),
        // SN74LS193 (`sn74193.pdf`): counts up on UP's (5) rise with DOWN
        // (4) high, down on DOWN's rise with UP high; CLR (14) high clears,
        // -LOAD (11) low loads A..D (15, 1, 10, 9). QA..QD are bits 0..3
        // on 3, 2, 6, 7. -CO (12) is low at 15 while UP is low, -BO (13)
        // low at 0 while DOWN is low.
        "74LS193" => seq(
            const {
                &[
                    g(3, &[], |_, s| lv(bit(s.bits, 0))),
                    g(2, &[], |_, s| lv(bit(s.bits, 1))),
                    g(6, &[], |_, s| lv(bit(s.bits, 2))),
                    g(7, &[], |_, s| lv(bit(s.bits, 3))),
                    g(12, &[5], |i, s| lv(!(s.bits & 0xf == 15 && !i[0]))),
                    g(13, &[4], |i, s| lv(!(s.bits & 0xf == 0 && !i[0]))),
                ]
            },
            &[1, 4, 5, 9, 10, 11, 14, 15],
            |now, prev, st| {
                let q = st.bits & 0xf;
                let next = if v(now, 14) {
                    0
                } else if !v(now, 11) {
                    word(now, &[15, 1, 10, 9]) as u16
                } else if rose(now, prev, 5) && v(prev, 4) {
                    (q + 1) & 0xf
                } else if rose(now, prev, 4) && v(prev, 5) {
                    q.wrapping_sub(1) & 0xf
                } else {
                    q
                };
                st.bits = (st.bits & !0xf) | next;
            },
        ),
        // Am25LS2521 (`am25ls2521.pdf`): -EOUT (19) low when -EIN (1) is
        // low and the eight pairs A/B match --- 2/3, 4/5, 6/7, 8/9, 11/12,
        // 13/14, 15/16, 17/18.
        "25LS2521" => comb(
            const {
                &[g(19, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 12, 13, 14, 15, 16, 17, 18], |i, _| {
                    let equal = (1..17).step_by(2).all(|k| i[k] == i[k + 1]);
                    lv(!(!i[0] && equal))
                })]
            },
        ),
        // DM8837 (`dm8837.pdf`): six inverting bus receivers, each output
        // low when its input or its disable is high; disable A (9) holds
        // receivers 1 to 3, disable B (7) receivers 4 to 6.
        "8837" => comb(
            const {
                &[
                    g(14, &[15, 9], |i, _| lv(!(i[0] || i[1]))),
                    g(12, &[13, 9], |i, _| lv(!(i[0] || i[1]))),
                    g(10, &[11, 9], |i, _| lv(!(i[0] || i[1]))),
                    g(2, &[1, 7], |i, _| lv(!(i[0] || i[1]))),
                    g(4, &[3, 7], |i, _| lv(!(i[0] || i[1]))),
                    g(6, &[5, 7], |i, _| lv(!(i[0] || i[1]))),
                ]
            },
        ),
        // SN75118 (`sn75118.pdf`): the driver, enabled by DE (13), puts
        // DA and DB (14, 15) on DY (4, 3) and its complement on DZ (1, 2);
        // the receiver, enabled by RE (10), puts on RY (12, 11) whether RA
        // (5) is above RB (7). The pair's two pins are one output here.
        "75118" => comb(
            const {
                &[
                    g(4, &[13, 14, 15], |i, _| if i[0] { lv(i[1] && i[2]) } else { Level::Z }),
                    g(3, &[13, 14, 15], |i, _| if i[0] { lv(i[1] && i[2]) } else { Level::Z }),
                    g(1, &[13, 14, 15], |i, _| if i[0] { lv(!(i[1] && i[2])) } else { Level::Z }),
                    g(2, &[13, 14, 15], |i, _| if i[0] { lv(!(i[1] && i[2])) } else { Level::Z }),
                    g(12, &[10, 5, 7], |i, _| if i[0] { lv(i[1] && !i[2]) } else { Level::Z }),
                    g(11, &[10, 5, 7], |i, _| if i[0] { lv(i[1] && !i[2]) } else { Level::Z }),
                ]
            },
        ),
        // MC1488 (`mc1488.pdf`): driver 1 inverts A (2) onto 3; drivers 2,
        // 3, 4 are NANDs of (4, 5), (9, 10), (12, 13) onto 6, 8, 11.
        "MC1488" => comb(
            const {
                &[
                    g(3, &[2], |i, _| lv(!i[0])),
                    g(6, &[4, 5], |i, _| lv(!(i[0] && i[1]))),
                    g(8, &[9, 10], |i, _| lv(!(i[0] && i[1]))),
                    g(11, &[12, 13], |i, _| lv(!(i[0] && i[1]))),
                ]
            },
        ),
        // MC1489 (`mc1489.pdf`): four inverting receivers, A on 1, 4, 10,
        // 13 to Y on 3, 6, 8, 11. An open input reads low, not high as a
        // TTL gate's would: the sheet's electrical characteristics give
        // V_OH, 2.6 V minimum, for the condition "Input open", and its
        // schematic puts 10 kΩ from the input to ground. So with nothing
        // on J9 the four outputs are high --- a mark on `TTL DATA IN`, and
        // the 2651's -DSR, -CTS and -DCD off --- which is what a port with
        // no cable sees, and what keeps its transmitter and receiver from
        // running until something is plugged in.
        "MC1489" => comb(
            const {
                &[
                    gm(3, &[1], |i, _| lv(!i[0])),
                    gm(6, &[4], |i, _| lv(!i[0])),
                    gm(8, &[10], |i, _| lv(!i[0])),
                    gm(11, &[13], |i, _| lv(!i[0])),
                ]
            },
        ),
        // Signetics 2651 PCI (`2651.pdf`, the 1978 preliminary sheet),
        // the serial port's controller, in asynchronous mode on its
        // internal baud-rate generator: the registers of Tables 4 to 8,
        // and a transmitter and receiver a bit at a time on the 16X clock
        // the generator divides `BRCLK` down to. The tables and the frame
        // are `src/serial.rs`'s, shared with the behavioural port. What
        // is held where is [`PCI_CELLS`]; [`pci_update`] is the chip.
        //
        // The bus side is level and edge as the sheet's READ AND WRITE
        // timing has it: while `-CE` (11) is low with `R/-W` (13) low the
        // selected register is on `D0`..`D7`, and the rise of `-CE` ends
        // the access, taking the data for a write and doing what a read
        // does to the pointers and the ready bits.
        //
        // From `RESET` (21) high, the state the sheet's Table 2 describes:
        // the mode, command and status registers clear, so the
        // transmitter and receiver are off, `TxD` marks, and the five
        // outputs the board wires sit high --- disabling them "causes
        // -RxRDY to go high (inactive)", leaves "the TxD output ... in the
        // marking state (high)" and sends "-TxRDY and -TxEMT ... high
        // (inactive)", and `CR1` and `CR5` clear read "FORCE -DTR OUTPUT
        // HIGH" and "FORCE -RTS OUTPUT HIGH" (Table 7).
        //
        // The generator is held while neither the transmitter nor the
        // receiver is on. The chip's runs always, but its output reaches
        // nothing off the chip --- `-TxC` and `-RxC` are not connected on
        // this board --- so its phase while idle cannot be seen, and
        // holding it lets a board with an idle port sleep between bus
        // cycles as it did before the chip was modelled.
        "2651" => seq(
            const {
                &[
                    g(27, PCI_BUS_IN, |i, s| pci_data(i, s, 0)),
                    g(28, PCI_BUS_IN, |i, s| pci_data(i, s, 1)),
                    g(1, PCI_BUS_IN, |i, s| pci_data(i, s, 2)),
                    g(2, PCI_BUS_IN, |i, s| pci_data(i, s, 3)),
                    g(5, PCI_BUS_IN, |i, s| pci_data(i, s, 4)),
                    g(6, PCI_BUS_IN, |i, s| pci_data(i, s, 5)),
                    g(7, PCI_BUS_IN, |i, s| pci_data(i, s, 6)),
                    g(8, PCI_BUS_IN, |i, s| pci_data(i, s, 7)),
                    g(19, &[3], PCI_TXD),
                    g(15, &[], |_, s| lv(!pci_tx_ready(s))),
                    g(14, &[], |_, s| lv(!pci_rx_ready_pin(s))),
                    g(18, &[], |_, s| lv(!pci_tx_empt_pin(s))),
                    g(24, &[], |_, s| lv(!(pci_cr(s) & serial_cr::DTR != 0 && !pci_local_loop(s)))),
                    g(23, &[], |_, s| lv(!(pci_cr(s) & serial_cr::RTS != 0 && !pci_local_loop(s)))),
                ]
            },
            &[1, 2, 3, 5, 6, 7, 8, 10, 11, 12, 13, 16, 17, 20, 21, 22, 27, 28],
            pci_update,
        ),
        // The analog parts --- the delay lines, the oscillators and the
        // one-shot --- cannot be levelized; `src/clock.rs` and `src/chip.rs`
        // run them by time instead.
        "TD25" | "TD50" | "TD100" | "TD250" | "TD25NC" | "TD100NC" | "MTD100" | "74LS124"
        | "DIPOSC" | "TTLOSC" | "26S02" => return None,

        _ => return None,
    })
}

/// Buffer with an active-low enable: `-oe`, then the input.
const BUF: GateFn = |i, _| if i[0] { Level::Z } else { lv(i[1]) };
/// Inverting buffer with an active-low enable.
const BUF_N: GateFn = |i, _| if i[0] { Level::Z } else { lv(!i[1]) };
/// Buffer with an active-high enable; only the 74S241's second half has one.
const BUF_HI: GateFn = |i, _| if i[0] { lv(i[1]) } else { Level::Z };

const ADD_IN: &[u8] = &[5, 3, 14, 12, 6, 2, 15, 11, 7];
const SHIFT_IN: &[u8] = &[13, 9, 10, 1, 2, 3, 4, 5, 6, 7];

fn alu_f(i: &[bool]) -> u8 {
    s181(nib(i, 0), nib(i, 4), nib(i, 8), i[12], i[13]).0
}

/// `carry` says whether the gate's input list carries pin 13; the group P
/// and G outputs do not use it, so anything may be passed for it.
fn cla(i: &[bool], carry: bool) -> (bool, bool, bool, bool, bool) {
    s182(nib(i, 0), nib(i, 4), carry && i[8])
}
