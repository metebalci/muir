// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The CADR netlist.
//!
//! `data/CADR.netlist` is extracted from MIT's SUDS drawings by
//! `tools/cadr-netlist.sh`, running the reader in `tools/soap4`.  It lists,
//! per schematic page, every part with its type and the net on each pin.
//!
//! It covers two of MIT's boards, not one: the CADR processor board and the
//! ICMEM control-store board, which is why the boot PROM and the control
//! store sit here alongside the ALU.  Established by comparing part counts
//! against MIT's own stockroom inventory.
//!
//! Every netlist is held to MIT's own wire list for the same board, pin by
//! pin, by `tests/*_netlist.rs`: the list is the board as it was wrapped and
//! the drawings are the board as designed, so where the two disagree the
//! reader has misread a drawing.
//!
//! Reference designators are **not** unique --- `1B20` appears twice on page
//! BCTERM, for instance --- so parts are identified by index, not by name.

use std::collections::HashMap;

/// An index into [`Netlist::nets`].
pub type NetId = u32;

#[derive(Clone, Debug)]
pub struct Part {
    /// Schematic page, e.g. `ACTL`.
    pub page: String,
    /// Reference designator, e.g. `3B26`.  Not unique.
    pub reference: String,
    /// Part type as the drawing names it, e.g. `74S174`.  A trailing `O`
    /// marks an open-collector variant; see [`Part::open_collector`].
    pub kind: String,
    /// Pin number to net, in the order the drawing lists them.
    pub pins: Vec<(u8, NetId)>,
}

impl Part {
    /// SUDS marks open-collector variants with a trailing `O`, as in `74S02O`.
    /// That distinction matters: an open-collector output can only pull low.
    pub fn open_collector(&self) -> bool {
        self.kind.ends_with('O') && self.kind.len() > 1
    }

    /// The type with any SUDS variant suffix removed.
    pub fn base_kind(&self) -> &str {
        self.kind.strip_suffix('O').unwrap_or(&self.kind)
    }
}

/// One physical package, with the pins of all its gates merged.
#[derive(Clone, Debug)]
pub struct Package {
    pub page: String,
    pub reference: String,
    pub kind: String,
    pub pins: Vec<(u8, NetId)>,
}

#[derive(Clone, Debug, Default)]
pub struct Netlist {
    pub parts: Vec<Part>,
    /// Every page marker in the file, in order.  Six of them carry no parts.
    pub pages: Vec<String>,
    /// Net names, indexed by [`NetId`].
    pub nets: Vec<String>,
    by_name: HashMap<String, NetId>,
}

impl Netlist {
    fn intern(&mut self, name: &str) -> NetId {
        if let Some(&id) = self.by_name.get(name) {
            return id;
        }
        let id = self.nets.len() as NetId;
        self.nets.push(name.to_string());
        self.by_name.insert(name.to_string(), id);
        id
    }

    /// Looks a net up by name.
    pub fn by_name_id(&self, name: &str) -> Option<NetId> {
        self.by_name.get(name).copied()
    }

    pub fn net(&self, id: NetId) -> &str {
        &self.nets[id as usize]
    }

    /// The parts grouped into physical packages.
    ///
    /// A `part` record in the file is a **gate**, not a package: a quad NAND
    /// appears as up to four records sharing one reference designator, each
    /// carrying one gate's pins. 1243 records are 1084 packages.
    ///
    /// Records sharing a designator are merged when their pins are disjoint.
    /// Where they overlap the designator has genuinely been reused for two
    /// different parts, which happens three times, all resistor packs on page
    /// BCTERM.
    pub fn packages(&self) -> Vec<Package> {
        let mut out: Vec<Package> = Vec::new();
        for part in &self.parts {
            let slot = out.iter_mut().find(|q| {
                q.page == part.page
                    && q.reference == part.reference
                    && q.kind == part.kind
                    && !part.pins.iter().any(|&(pin, _)| q.pins.iter().any(|&(p, _)| p == pin))
            });
            match slot {
                Some(q) => q.pins.extend(part.pins.iter().copied()),
                None => out.push(Package {
                    page: part.page.clone(),
                    reference: part.reference.clone(),
                    kind: part.kind.clone(),
                    pins: part.pins.clone(),
                }),
            }
        }
        for q in &mut out {
            q.pins.sort_unstable();
        }
        out
    }

    /// Every part that touches a net.
    pub fn parts_on(&self, net: NetId) -> impl Iterator<Item = &Part> {
        self.parts.iter().filter(move |p| p.pins.iter().any(|&(_, n)| n == net))
    }

    /// Pages that actually carry parts.  Fewer than [`Netlist::pages`].
    pub fn populated_pages(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.parts.iter().map(|p| p.page.as_str()).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

/// A net name without the quotes SUDS puts round one that has a space in it.
///
/// The netlist keeps the quotes, because they are how the format tells such a
/// name from two fields; they are noise anywhere the name is being read.
pub fn plain(net: &str) -> &str {
    net.trim_matches('\'')
}

/// The root of `x` in the union-find `up`, halving the path on the way: what
/// the two joins below merge nets with.
fn find(up: &mut [NetId], mut x: NetId) -> NetId {
    while up[x as usize] != x {
        up[x as usize] = up[up[x as usize] as usize];
        x = up[x as usize];
    }
    x
}

/// Parses the netlist.  Lines outside a `part`/`(`/`)` block are comments or
/// page markers.  This is the board as the model runs it: [`parse_wired`]
/// with the series resistors joined and the wires MIT added by hand after
/// wrapping, `Netlist::HAND_JUMPERS`.
pub fn parse(text: &str) -> Result<Netlist, String> {
    let mut n = parse_wired(text)?;
    n.merge_series_resistors();
    n.apply_hand_jumpers();
    Ok(n)
}

/// The netlist as wired: every alias of a net merged, but the two ends of
/// a series resistor left as the two nets they are. [`parse`] goes on to
/// join the resistors for the model. Nothing here moves a pin: where a
/// board differed from its drawings --- MIT's change orders, and the
/// drawings' own errors --- the `data/*.netlist` file already says so,
/// moved when it was built by `examples/reconcile.rs` and listed in its
/// banner.
pub fn parse_wired(text: &str) -> Result<Netlist, String> {
    let mut n = parse_raw(text)?;
    n.merge_explicit_aliases();
    n.split_by_board();
    n.merge_anonymous_nets();
    // After the anonymous merge, not before. A strap can join a name MIT
    // wrote to a wire this reader had to invent a name for, and the merge
    // picks its own representative: run first and the strap's name is buried
    // under an `@REF,pN` one, leaving the net correctly wired and impossible
    // to look up. `ECLVID`'s `-MECL VIDEO` is the case that showed it.
    n.apply_strap_pages();
    Ok(n)
}

impl Netlist {
    /// Nets the file names twice that no rule can find, because the two
    /// spellings have nothing in common. Each is justified by the same
    /// evidence: one name occurs only as a driver with no consumer, the other
    /// only as a consumer with no driver.
    ///
    /// Two are the wires between the two boards this file holds that
    /// change name at the connector, read off MIT's wire lists: the
    /// processor's `3CJ1`, `3EJ1` and `4EJ1` meet the ICMEM board's `1CJ2`,
    /// `1EJ2` and `1EJ1` pin for pin (the `J2` headers two pins on), and
    /// where the two lists name a pin differently the netlist, having no
    /// connector parts, may have two nets. It does for two of the sixteen:
    /// the other fourteen are `PC0B`..`PC13B`, the processor's buffered copy
    /// of the PC, which the ICMEM list calls `PC0`..`PC13` at its end and
    /// the file already has as one net under the processor's name.
    /// `-FUNCT1` is the 74S139 at SOURCE 3D05 pin 11 decoding misc function
    /// 1, `HALT-CONS`; it leaves the processor board on 3CJ1-19
    /// (`cadrwd/cadr4.wlr`) and is `-HALT` on the ICMEM board at 1CJ2-21,
    /// the D input of the 74S374 at OLORD2 1A05 (`cadrwd/icmem3.wlr`).
    /// `PROG.UNIBUS.RESET` is the interrupt
    /// control register's bit 28 on FLAG, and `PROG.BUS.RESET` is what
    /// OLORD2 turns into `-BUS.RESET` for the bus interface; unjoined, the
    /// Unibus stays in reset for ever, which the behavioural far end never
    /// notices and the netlist one does. `tests/netlist.rs` regenerates this
    /// list from the wire lists.
    /// `-64 MHz CLK` is the last of them, and the only one that is a
    /// difference of case alone: MIT typed the dot clock's name with a
    /// lower-case `Hz` on the display boards' SIP terminator sheet
    /// (`necsip.drw`, "MECL SIP TERMINATORS", E05 pin 6 as `64 MHz CLK L`)
    /// and with `MHZ` on every sheet that uses it. Of the 27 terminator
    /// pins on that page, 26 land on a net with parts elsewhere and this
    /// one landed on nothing, so the dot clock ran with no pull-down and
    /// the ECL parts on it read high and high-impedance where the board
    /// swings high and low.
    ///
    /// A case-insensitive rule would do the same job, and is not taken:
    /// this is the **only** pair in any of the seven netlists that differs
    /// by case --- 7,911 nets, one clash, on the SIMPLE TV and the LISPM TV
    /// alike, MIT having typed the same sheet the same way twice.
    /// `tests/netlist.rs` measures that, so a second clash arriving in some
    /// later board is a failing test and not a silent merge.
    ///
    /// One wire MIT's lists name twice on a single board needs no entry:
    /// `cadrwd/icmem3.wlr` lists `-TPW60` beside `-TPDONE` on the TD25 tap
    /// at CLOCK1 1C14 pin 4, the clock generator's cycle-restart input at
    /// 1C08 pin 2, and the file has that wire under `-TPDONE` alone.
    const EXPLICIT_ALIASES: &'static [(&'static str, &'static str)] = &[
        ("PROG.BUS.RESET", "PROG.UNIBUS.RESET"),
        // `-FUNCT1` leaves the processor on 3CJ1-19 and is `-HALT` on ICMEM
        // at 1CJ2-21: `cadrwd/cadr4.wlr` and `cadrwd/icmem3.wlr`.
        ("-HALT", "-FUNCT1"),
        ("'-64 MHZ CLK'", "'-64 MHz CLK'"),
        // `-11CLRTDN` with a space after the minus and without: two
        // spellings of one wire, which soap4 makes two nets of. MIT's
        // `cadrio/iob.wlr` has it as one, `-11CLRTDN`, with the 74S00 at
        // D07 pin 11 driving it and the 74S08 at B10 pin 5 taking it, so
        // apart the 74S08's input has no source at all. The wire-list
        // comparison cannot see this one: it matches names with the spaces
        // squeezed out, which is exactly the difference.
        ("'- 11CLRTDN'", "-11CLRTDN"),
    ];

    /// The joins a board's sheets draw as bare wire rather than through a
    /// body, which a parts-based reader takes nothing from.
    ///
    /// Two kinds, both of them wire-wrap straps as far as the board is
    /// concerned. **A whole customise page** carries no parts at all, so
    /// nothing of it reaches the netlist; that is `gen4b` below. **A stub
    /// or a strap on a sheet that does carry parts** is a second name
    /// written on a wire, or a naked line drawn between two names, and
    /// soap4 then hands the pins on either side of it two different nets.
    /// That is `NSYREG` and `NSYCLK` below, and every one of those four is
    /// a net that the sheet shows joined and the netlist has apart, with
    /// one half holding a driver and the other only inputs.
    ///
    /// Keyed by page, so a join belongs to the board whose sheet draws it:
    /// the SIMPLE TV's pages carry the `n` its May 1980 revision added
    /// (`NSYREG`, `NSYCLK`, `NRAADR`) and the LISPM TV's are the 1979
    /// names those vacated (`SYNREG`, `SYNCLK`, `RAMADR`), so no entry
    /// here can reach the other board. See `tools/simpletv-netlist.sh`.
    ///
    /// MIT built the display boards in a four-bit and an eight-bit version
    /// and put the difference on one sheet: `cadrtv/*/gen4b.drw`,
    /// "Customize for 4 bit version", which has **no reference-designator
    /// bodies at all** --- it is wire-wrap straps, drawn as connections.
    /// A parts-based reader takes nothing from it, so the wires it joins
    /// stay apart and the frame buffer's address inputs have no driver.
    ///
    /// The LISPM TV does not need this: MIT's wire list for it,
    /// `lmtv4b.wlr`, lists each of these as **one wire under both names**
    /// --- `ADR 1` and `RAM ADR IN 0` together, on A11-03 and E10-03 ---
    /// because the list describes the board as wrapped, straps included,
    /// and `examples/reconcile.rs` joins them under MIT's own name. The
    /// SIMPLE TV has no wire list at all (discrepancy 35), so its straps
    /// have to come from the sheet.
    ///
    /// The mapping is the four-bit one: the RAM address is the Xbus address
    /// shifted down one, `ADR 0` unused, `ADR 15` the bank select. It is
    /// confirmed twice over --- read off both boards' `gen4b` sheets, where
    /// the 24 nets are identical, and MIT's LISPM wire list independently
    /// pairs the same names. `gen8b.drw` shifts by two
    /// instead and is the eight-bit build, which muir does not make.
    ///
    /// `SHF IN 0`..`7` are the same sheet's other straps, `GND ---- SHF IN
    /// 0` eight times over, drawn exactly as the address ones are. They are
    /// the serial inputs of the eight 74S299s at NRASHF, so nothing the
    /// processor does reaches them and they sat outside this table while the
    /// board's read-out path did not run. It runs now, and with them
    /// floating the shifted-in bits were unknown and `MECL VIDEO OUT` never
    /// went low: the picture was one level and a high impedance.
    const STRAP_PAGES: &'static [(&'static str, &'static [(&'static str, &'static str)])] = &[
        (
            // NRAADR is the SIMPLE TV's own name for the page; its presence is
            // what says this netlist is that board.
            "NRAADR",
            &[
                ("ADR1", "RAM ADR IN 0"),
                ("ADR2", "RAM ADR IN 1"),
                ("ADR3", "RAM ADR IN 2"),
                ("ADR4", "RAM ADR IN 3"),
                ("ADR5", "RAM ADR IN 4"),
                ("ADR6", "RAM ADR IN 5"),
                ("ADR7", "RAM ADR IN 6"),
                ("ADR8", "RAM ADR IN 7"),
                ("ADR9", "RAM ADR IN 8"),
                ("ADR10", "RAM ADR IN 9"),
                ("ADR11", "RAM ADR IN 10"),
                ("ADR12", "RAM ADR IN 11"),
                ("ADR13", "RAM ADR IN 12"),
                ("ADR14", "RAM ADR IN 13"),
                ("ADR15", "ADR BANK SEL"),
                ("MAPADR15", "MAPADR BANK"),
                ("GND", "SHF IN 0"),
                ("GND", "SHF IN 1"),
                ("GND", "SHF IN 2"),
                ("GND", "SHF IN 3"),
                ("GND", "SHF IN 4"),
                ("GND", "SHF IN 5"),
                ("GND", "SHF IN 6"),
                ("GND", "SHF IN 7"),
            ],
        ),
        (
            // `nsyreg.drw`, "SYNC PROGRAM REG & REPEAT" of 17 May 1980, the
            // sheet the sync program's own control comes off. Without these
            // three the program does not loop: it counts straight up through
            // `cpt.prom` and the raster is one line and no more.
            // `tests/simpletv_netlist.rs` measures that.
            "NSYREG",
            &[
                // The 74S10 at D01 pin 6 has **two labels on its output**,
                // `SYNC RPT INC L` and `SYNC ADR LOAD L`, joined on the sheet
                // by a junction dot. One net: the repeat counter's `-CEP` at
                // C02 and C01 pin 7 and the sync address counter's `-LOAD` at
                // NSYADR pin 11 are driven by the same gate. soap4 gave the
                // gate and the 74LS669s the first name and the three 74LS569s
                // the second.
                ("-SYNC RPT INC", "-SYNC ADR LOAD"),
                // Two straps drawn as naked lines across the middle of the
                // sheet, under the decoders they qualify:
                //
                //     SYNC CYC 1 ----- TVMA INC & STEP
                //     SYNC FCN 1 ----- SYNC EOL & EOF
                //
                // Each is the top select bit of a 25LS2539 at C04 taken as a
                // signal of its own. The decoder's outputs 2 and 3 are `TVMA
                // INC L` and `TVMA STEP L` on one half and `SYNC EOL L` and
                // `SYNC EOF L` on the other, so `SEL1` high is exactly "one
                // of that pair", which is what the two names say. Both are
                // driven by the 74S374 at C03, pins 15 and 19.
                ("SYNC CYC 1", "TVMA INC & STEP"),
                ("SYNC FCN 1", "SYNC EOL & EOF"),
            ],
        ),
        (
            // `xbdata.drw`, "XBUS DATA": the 74S04 at F10 pin 12 inverts
            // `WRITE`, and its output carries two labels, `WRITE L` and
            // `READ` --- the same two the LISPM TV's copy of the sheet
            // carries, where MIT's wire list `lmtv4b.wlr` puts all seven
            // pins on one net under `READ`. soap4 gave the inverter, the
            // `-RAMWR` mux at NRAADR A11 and the control decoder at F13 the
            // first name and the `-MAP RD` decoder at RAMCAS C12 and the
            // two `-XDRIVE` gates at E13 the second, so `READ` had no
            // driver: it floated high, and the board answered a write by
            // driving its own data at the master.
            // `tests/simpletv_netlist.rs` holds it to reading only. Keyed on
            // the SIMPLE TV's `NXBCTL`, where `READ` is used, because both
            // boards have a page called `XBDATA`.
            "NXBCTL",
            &[("-WRITE", "READ")],
        ),
        (
            // `nsyclk.drw`, "MECL VIDEO" of 17 May 1980. `CLK0 64B SR` runs
            // into pin 2 of the 74S37 at D09 and carries a stub off a junction
            // dot labelled `8B SR LOAD`: one wire, two names, as the sheet's
            // own waveform says by drawing it once as `CLK0 64B/LOAD 8B`. The
            // 74S374 at E12 pin 5 drives it; unjoined, the 74S37 at D09 and
            // the 10124 translator at NECCLK 0F07 had nothing on their inputs
            // and the video shift register was never loaded.
            "NSYCLK",
            &[("CLK0 64B SR", "8B SR LOAD")],
        ),
        (
            // `eclvid.drw`, "MECL VIDEO". The two 10102 NOR outputs at F04
            // pins 3 and 14 are tied together in a wired OR and the junction
            // carries the sheet's `MECL VIDEO L`, which is `-MECL VIDEO` at
            // the 10212 at F03 pin 7. MIT draws that junction as a point
            // that is not a body pin, and soap4 does not follow it: the
            // label parses and propagates to F03 alone, leaving the two
            // outputs on an anonymous net and `-MECL VIDEO` with no driver.
            // The SIP terminator at NECSIP F05 then holds it low and the
            // video output is `NOR(MECL BLANK, low)` --- every unblanked dot
            // lit, whatever the frame buffer holds.
            //
            // **The same page of the LISPM TV needs no strap**, and that is
            // what identifies this as a reader limitation rather than a
            // difference between the boards: soap4 leaves the junction
            // anonymous there too, and `tools/lispmtv-netlist.sh` repairs it
            // by reconciling against `cadrtv/lmtv4b.wlr`, which puts F04-3,
            // F04-14 and F03-7 on one net named `-MECL VIDEO`. MIT left no
            // wire list of the SIMPLE TV, so `tools/simpletv-netlist.sh` has
            // nothing to reconcile against and the miss survives. The join
            // below is that wire list's word, applied to the board it does
            // not cover, as the `READ` join above is.
            //
            // The two boards' ECLVID sheets carry these gates identically,
            // same references and same pins, so nothing is being carried
            // across a design difference. `tests/simpletv_netlist.rs` holds
            // the picture to the frame buffer.
            "ECLVID",
            &[("-MECL VIDEO", "@0F04,p3")],
        ),
    ];

    /// The wires MIT added by hand after a board came back from wrapping,
    /// which no wire list has: the disk controller's one-board jumpers, its
    /// address jumpers and its timeout enable.
    ///
    /// `cadrdc/dc.eco` §ii, "Hand wiring after board comes back from
    /// wire-wrapping (add in red): Jumpers for 1-board version (do NOT
    /// install if this DC is to be associated with a DM board)", lists six
    /// --- `DE2 : DF2`, `DF2 : DH2`, `DN1 : DM2`, `ES2 : ET1`, `EP2 : ER2`,
    /// `ER2 : ES2`; `cadrdc/disk.hand` lists the same six, and the DCEDGE
    /// sheet notes them, "JUMPERS FOR 1-BOARD VERSION". The pins are the
    /// edge connector's, and `cadrdc/dc.wlr` says what each carries: `DE2`
    /// `SEL UNIT ATTENTION`, `DF2` `ANY ATTENTION`, `DH2` `UNIT 0 ATTENTION`
    /// --- the output of the 74LS14 at DCTRSG 0A07, the drive's own
    /// attention off the cable --- `DM2` `MULTIPLE SELECT`, `EP2`, `ER2`
    /// and `ES2` `UNIT0`, `UNIT1` and `UNIT2`, and `DN1` and `ET1` on the
    /// ground net. They are the DISK MULTIPLEXOR board's outputs; on the
    /// one-board controller they are open 74LS inputs and read high, so the
    /// unit number reads 7, `MULTIPLE SELECT` --- an input of the
    /// disk-lossage gate --- stops every transfer at START, and the status
    /// word carries both attention bits with nothing on the cable. With
    /// the jumpers the unit number and `MULTIPLE SELECT` are ground, and
    /// `STATUS<2:1>` and the attention interrupt through the 9S42 at DCCHAN
    /// 0F17 are the one drive's own attention, which is what
    /// `sys/doc/disk.text` says of `STATUS<2>`: "This is the attention
    /// signal directly from the drive, it is not separately latched in the
    /// controller".
    ///
    /// `disk.hand`'s next paragraph is the seven **address jumpers**, `J5-1
    /// : J5-2` through `J5-13 : J5-14`, and the one after that the
    /// **timeout enable**, `J5-16 : J5-41`, "the adjacent ground pin";
    /// `dc.eco` lists both again. `dc.wlr` says what the pins carry. `J5`'s
    /// odd pins 1 to 15 are all on `HI1`, the net the 330 ohm SIP at DCTRSG
    /// 0A06 pulls up, and its even pins 2 to 14 are `AD14`, `AD13`, `AD6`,
    /// `AD5`, `AD4`, `AD3` and `AD2`, the A inputs of the 25LS2521 address
    /// comparator at DCREG 0E14 whose B inputs are the bus address,
    /// `XBAI2`..`XBAI14`; its eighth pair is `GND` against `XBAI17`. So
    /// each jumper says one bit of the controller's own address is a 1, and
    /// the seven together are why the board answers at
    /// [`crate::disk_controller::REGS`], `17377774` --- what
    /// `sys/doc/disk.text` means by "The address can be changed by changing
    /// jumpers". Without them those seven are open 74LS inputs and read
    /// high anyway, so this is the address made definite rather than a
    /// change of behaviour.
    ///
    /// The timeout enable is a change of behaviour. `J5-16` is `-TIMEOUT
    /// ENB`, which is pin 6, the enable, of the 74LS124 at DCTMOT 0B04
    /// section 1 and nothing else on the board. Open, it reads high, and by
    /// the part's own sheet the output is then held high: `TIMEOUT.CLK`
    /// never falls, the 74393 at 0C03 never counts, and no operation can
    /// ever time out. The jumper grounds it, and the board has the watchdog
    /// `sys/doc/disk.text` describes,
    /// [`crate::disk_controller::TIMEOUT_NS`] long. The other section's
    /// enable, pin 11, is on ground in the wire list, so the 2 us clock
    /// needs no jumper.
    ///
    /// The fourth paragraph, the Xbus Power OK jumper `B5-3 : B5-4`, is
    /// not here. Those are socket numbers, the numbering `dc.eco`'s own
    /// wires are written in: `dc.wlr` has `B05@03-01(03)` --- logical pin 1
    /// in socket 3 --- on `XBUS.POWER.OK`, which reaches the board from the
    /// backplane at `CK1` and touches nothing else on it, and
    /// `B05@03-02(04)` on `HI7`, so the jumper puts that 75452's two inputs
    /// on the pull-up together and `TRIDENT.0.SEQUENCE/` stops waiting on
    /// the backplane. Whether a board wrapped to the 10 December 1980 list
    /// wants it is **unverified**: `dc.eco` conditions it on ECO 8 --- "the
    /// wires under this are removed by ECO #8, so put that in first, then
    /// come back and install this" --- and says in the same paragraph that
    /// ECO 8 "does not apply to boards made to the older wirelist, where
    /// part of ECO#8 is included in ECO#6", which the drawings already
    /// carry. It makes no difference to this netlist either way: with one
    /// pin on the board the net reads high with the jumper or without.
    ///
    /// Applied by [`parse`] and not by [`parse_wired`]: the wire list is the
    /// board as wrapped, before the red wires, and `tests/cadrdc_netlist.rs`
    /// holds the wired netlist to the list and the parsed one to the
    /// jumpers. Keyed by page, as [`Netlist::STRAP_PAGES`] is; DCEDGE has no
    /// parts, so the page list is what says the board is here.
    const HAND_JUMPERS: &'static [(&'static str, &'static [(&'static str, &'static str)])] = &[(
        "DCEDGE",
        &[
            ("UNIT 0 ATTENTION", "ANY ATTENTION"),      // DF2 : DH2
            ("UNIT 0 ATTENTION", "SEL UNIT ATTENTION"), // DE2 : DF2
            ("GND", "MULTIPLE SELECT"),                 // DN1 : DM2
            ("GND", "UNIT2"),                           // ES2 : ET1
            ("GND", "UNIT1"),                           // ER2 : ES2
            ("GND", "UNIT0"),                           // EP2 : ER2
            ("GND", "-TIMEOUT ENB"),                    // J5-16 : J5-41
            ("HI1", "AD14"),                            // J5-1 : J5-2
            ("HI1", "AD13"),                            // J5-3 : J5-4
            ("HI1", "AD6"),                             // J5-5 : J5-6
            ("HI1", "AD5"),                             // J5-7 : J5-8
            ("HI1", "AD4"),                             // J5-9 : J5-10
            ("HI1", "AD3"),                             // J5-11 : J5-12
            ("HI1", "AD2"),                             // J5-13 : J5-14
        ],
    )];

    /// A net by either spelling: bare, or quoted as a name with spaces is
    /// stored.
    fn id_either(&self, name: &str) -> Option<NetId> {
        self.by_name_id(name).or_else(|| self.by_name_id(&format!("'{name}'")))
    }

    /// Puts every pin of `gone` on `keep`, and `gone`'s name with them, so
    /// that looked up it is the joined net and not the half left with no
    /// pins.
    fn join(&mut self, keep: NetId, gone: NetId) {
        if keep == gone {
            return;
        }
        for part in &mut self.parts {
            for pin in &mut part.pins {
                if pin.1 == gone {
                    pin.1 = keep;
                }
            }
        }
        let name = self.nets[gone as usize].clone();
        self.by_name.insert(name, keep);
    }

    /// Applies [`Netlist::STRAP_PAGES`] for every board whose page is here.
    fn apply_strap_pages(&mut self) {
        for &(page, straps) in Self::STRAP_PAGES {
            if !self.parts.iter().any(|p| p.page == page) {
                continue;
            }
            for &(a, b) in straps {
                let (Some(keep), Some(gone)) = (self.id_either(a), self.id_either(b)) else {
                    continue;
                };
                self.join(keep, gone);
            }
        }
    }

    /// Applies [`Netlist::HAND_JUMPERS`] for every board whose page is here.
    fn apply_hand_jumpers(&mut self) {
        for &(page, jumpers) in Self::HAND_JUMPERS {
            if !self.pages.iter().any(|p| p == page) {
                continue;
            }
            for &(a, b) in jumpers {
                let (Some(keep), Some(gone)) = (self.id_either(a), self.id_either(b)) else {
                    continue;
                };
                self.join(keep, gone);
            }
        }
    }

    fn merge_explicit_aliases(&mut self) {
        for &(from_name, to) in Self::EXPLICIT_ALIASES {
            let (Some(from), Some(to)) = (self.by_name_id(from_name), self.by_name_id(to)) else {
                continue;
            };
            for part in &mut self.parts {
                for pin in &mut part.pins {
                    if pin.1 == from {
                        pin.1 = to;
                    }
                }
            }
            // The name goes with its pins: looked up, it is the joined net
            // and not the half left with none.
            self.by_name.insert(from_name.to_string(), to);
        }
    }

    /// Joins the two names every unnamed net is given.
    ///
    /// SUDS leaves some nets unnamed, and the reader that produced this file
    /// names each one after the pin at the *other* end. The wire between
    /// ALUC4 2C15 pin 3 and 2C20 pin 13 is therefore called `@2C20,p13`
    /// where 2C15 touches it and `@2C15,p3` where 2C20 does --- two names,
    /// one wire, and without this every one of them is broken in half.
    ///
    /// The rule is mechanical and needs no judgement, unlike the two aliases
    /// above: a pin carrying `@REF,pN` is on the same wire as pin `N` of
    /// `REF`, on the same page.
    ///
    fn merge_anonymous_nets(&mut self) {
        let mut at: HashMap<(&str, &str, u8), NetId> = HashMap::new();
        for part in &self.parts {
            for &(pin, net) in &part.pins {
                at.insert((part.page.as_str(), part.reference.as_str(), pin), net);
            }
        }
        // Union-find over net ids, so a wire named from three pins still
        // ends up as one net.
        let mut up: Vec<NetId> = (0..self.nets.len() as NetId).collect();
        let mut joins: Vec<(NetId, NetId)> = Vec::new();
        for part in &self.parts {
            for &(_, net) in &part.pins {
                let name = &self.nets[net as usize];
                let Some(rest) = name.strip_prefix('@') else { continue };
                let Some((reference, pin)) = rest.split_once(",p") else { continue };
                let Ok(pin) = pin.parse::<u8>() else { continue };
                if let Some(&other) = at.get(&(part.page.as_str(), reference, pin)) {
                    joins.push((net, other));
                }
            }
        }
        for (a, b) in joins {
            let (a, b) = (find(&mut up, a), find(&mut up, b));
            if a != b {
                up[b as usize] = a;
            }
        }
        for part in &mut self.parts {
            for pin in &mut part.pins {
                let mut x = pin.1;
                while up[x as usize] != x {
                    x = up[x as usize];
                }
                pin.1 = x;
            }
        }
        // Every name merged away now looks up the net its pins are on, so a
        // lookup cannot land on the half left with none. Probing such a half
        // reads `Z` for ever and looks like a signal that never asserts,
        // which is how three of the four symptoms in the display's own
        // discrepancy came to be recorded.
        for id in 0..self.nets.len() as NetId {
            let root = find(&mut up, id);
            if root != id {
                let name = self.nets[id as usize].clone();
                self.by_name.insert(name, root);
            }
        }
    }

    /// Pin `k` of these is one resistor to pin `17-k`, eight to a package.
    ///
    /// `SERRESL` is what the memory board calls its pack and `898-3-R22`
    /// what the display board calls its; the two wire the same way and
    /// [`Netlist::merge_series_resistors`] treats them alike.
    const SERIES_RESISTORS: &'static [&'static str] = &["SERRESL", "898-3-R22"];

    /// Joins the two ends of every series resistor.
    ///
    /// The memory board puts a damping resistor between each driver and
    /// the DRAM lines it drives, eight to a `SERRESL` pack, pin `k` to pin
    /// `17-k`: `-RAS 0` in at 1 comes out as `-RAS 0 LH` at 16. The display
    /// board does the same on NRABUF with `898-3-R22` packs, `RAM BFR 0` in
    /// at 1 and `RAMB A0` out at 16. Logically a resistor in series is a
    /// wire, so each pair is one net here, and the pack drives nothing.
    /// See `data/CADRM.netlist` and `data/SIMPLETV.netlist`.
    fn merge_series_resistors(&mut self) {
        // Union-find, as for the anonymous nets: one line fans out through
        // two packs, `-RAS 0` to `-RAS 0 LH` and `-RAS 0 RH`, and a rename
        // after the first pack would leave the second nothing to join.
        let mut up: Vec<NetId> = (0..self.nets.len() as NetId).collect();
        let mut joined = false;
        for part in &self.parts {
            if !Self::SERIES_RESISTORS.contains(&part.kind.as_str()) {
                continue;
            }
            for &(pin, net) in &part.pins {
                if pin <= 8
                    && let Some(&(_, other)) = part.pins.iter().find(|&&(p, _)| p == 17 - pin)
                {
                    let (a, b) = (find(&mut up, net), find(&mut up, other));
                    if a != b {
                        up[b as usize] = a;
                        joined = true;
                    }
                }
            }
        }
        if !joined {
            return;
        }
        for part in &mut self.parts {
            for pin in &mut part.pins {
                pin.1 = find(&mut up, pin.1);
            }
        }
    }

    /// Nets the file has as one because the two boards call them the same
    /// thing, that are one a board. `-RESET`: each board makes its own from
    /// the shared `RESET` with its own 74S37, CLOCKD 1B18 on the processor
    /// and OLORD2 1A06 on ICMEM, and MIT's wire lists have the two runs
    /// with no connector pin between them --- `cadrwd/cadr4.wlr` nine pins
    /// all on processor pages, `icmem3.wlr` four all on OLORD1 and OLORD2,
    /// while `RESET` on the processor list is a one-pin run with no drive,
    /// the connector bringing it from ICMEM. Read as one net, the two
    /// totem-pole outputs fight over it (discrepancy 13). Both lists spell
    /// theirs `-RESET`; the ICMEM board's is `-ICMEM RESET` here, a name of
    /// the parser's own, so that the two can be told apart.
    const EXPLICIT_SPLITS: &'static [(&'static str, &'static str, &'static [&'static str])] =
        &[("-RESET", "-ICMEM RESET", &["OLORD1", "OLORD2"])];

    fn split_by_board(&mut self) {
        for &(name, other, pages) in Self::EXPLICIT_SPLITS {
            let Some(from) = self.by_name_id(name) else { continue };
            // Only where the file has pins on those pages: another board's
            // `-RESET` is its own and stays one net.
            let moving = self
                .parts
                .iter()
                .filter(|part| pages.contains(&part.page.as_str()))
                .any(|part| part.pins.iter().any(|&(_, net)| net == from));
            if !moving {
                continue;
            }
            let to = self.nets.len() as NetId;
            self.nets.push(other.to_string());
            self.by_name.insert(other.to_string(), to);
            for part in &mut self.parts {
                if !pages.contains(&part.page.as_str()) {
                    continue;
                }
                for pin in &mut part.pins {
                    if pin.1 == from {
                        pin.1 = to;
                    }
                }
            }
        }
    }
}

fn parse_raw(text: &str) -> Result<Netlist, String> {
    let mut n = Netlist::default();
    let mut page = String::new();
    let mut pending: Option<(String, String)> = None;
    let mut pins: Vec<(u8, NetId)> = Vec::new();
    let mut in_block = false;

    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("page ") {
            page = rest.trim().to_string();
            n.pages.push(page.clone());
        } else if let Some(rest) = line.strip_prefix("part ") {
            let (reference, kind) = rest
                .split_once(',')
                .ok_or_else(|| format!("line {}: part without a type: {rest:?}", lineno + 1))?;
            pending = Some((reference.trim().to_string(), kind.trim().to_string()));
        } else if line == "(" {
            in_block = true;
            pins.clear();
        } else if line == ")" {
            let (reference, kind) = pending
                .take()
                .ok_or_else(|| format!("line {}: pin block with no part", lineno + 1))?;
            n.parts.push(Part {
                page: page.clone(),
                reference,
                kind,
                pins: std::mem::take(&mut pins),
            });
            in_block = false;
        } else if in_block {
            let (pin, net) = line
                .split_once('=')
                .ok_or_else(|| format!("line {}: bad pin line {line:?}", lineno + 1))?;
            let pin: u8 = pin
                .trim()
                .strip_prefix('p')
                .ok_or_else(|| format!("line {}: bad pin {pin:?}", lineno + 1))?
                .parse()
                .map_err(|e| format!("line {}: bad pin number: {e}", lineno + 1))?;
            // `NC` is not a net. It marks a pin as unconnected, and it appears
            // on unused pins all over the board, so interning it by name would
            // tie every unconnected pin together --- which fabricates paths
            // right across the design. Each one gets its own net instead.
            let name = net.trim();
            let id = if name == "NC" {
                let unique = format!("NC#{}", n.nets.len());
                n.intern(&unique)
            } else {
                n.intern(name)
            };
            pins.push((pin, id));
        }
    }
    Ok(n)
}
