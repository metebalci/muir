// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The five flat cables, and what is at the far end of them.
//!
//! > The bus interface plugs into the Xbus and the Unibus, and is connected
//! > to the cpu by 5 40-wire (alternating grounds) flat cables, which carry
//! > two independent busses and the master clock, which is supplied by the
//! > cpu to the bus interface.
//!
//! --- `cadr/busint.erface`. `data/CADR.netlist` is the processor board and
//! the control-store board, so it stops at the connector: `-MEMACK`,
//! `-LOADMD`, `-MEMGRANT` and `-IGNPAR` arrive on pins that nothing on the
//! board drives, and the SIP330/470-8 at BCTERM 2C25 pulls them up. Without
//! something at the far end, `MBUSY` never clears and the machine stops on
//! the boot PROM's first pair of bus cycles --- measured, at `0o313`.
//!
//! The far end is the bus interface board, `data/BUSINT.netlist`, a netlist
//! too: [`Cables`] carries the 92 wires of `data/cables.txt` between the two
//! boards, [`busint_board`] builds the interface powered and plugged into
//! its buses, and [`FarEnd`] is the three together with the Xbus and the
//! Unibus behind them answered from [`Machine`] by [`Buses`]. `rtl` runs on
//! a behavioural model of the same interface, `src/busint.rs`, and the two
//! are held to each other by `chip_agrees_with_rtl`: where the boards and
//! the model part, one of them is the board, and every time so far it has
//! been the netlist.
//!
//! The signals on the cables, and what the two netlists call them:
//!
//! | MIT's name | processor | bus interface | direction |
//! |---|---|---|---|
//! | `-MEMRQ` | `MEMRQ` | `-MEMRQ` | from the cpu, a level while it wants a cycle |
//! | `WRCYC` | `WRCYC` | `WRCYC` | from the cpu, high to write |
//! | `-ADR<21:0>` | `-PMA21`..`-PMA8`, `-VMA7`..`-VMA0` | `-ADR21`..`-ADR0` | from the cpu |
//! | `MEM<31:0>` | `MEM0`..`MEM31` | `MEM0`..`MEM31` | both ways, active high |
//! | `-MEMACK` | `-MEMACK` | `-LMACK` | to the cpu |
//! | `-LOADMD` | `-LOADMD` | `-LOADMD` | to the cpu, a pulse |
//! | `-MEMGRANT` | `-MEMGRANT` | `-LM GRANT` | to the cpu |
//! | `-IGNPAR` | `-IGNPAR` | `-LM IGNPAR` | to the cpu |
//! | `MCLK7` | `MCLK7` | `-MCLK7` | from the cpu |
//!
//! The address arrives already translated: `-PMA21..8` is the map's output
//! held by the 74S373s at VMEMDR 1D14 and 1D15, and `-VMA7..0` the offset the
//! map never sees. `data/cables.txt` has the other 55 wires: the SPY bus and
//! the diagnostic strobes, the grant, `WRCYC`, `INT`, and the resets.

use std::collections::VecDeque;

use crate::buses::Buses;
use crate::busint::{DEBUG_CYCLE, DEBUG_MSYN_NS, DebugRequest};
use crate::chip::Chip;
use crate::clock::{Behavioural, Clock, Speed};
use crate::lashup::CableEnd;
use crate::machine::{Halt, Machine};
use crate::netlist::{NetId, Netlist};
use crate::part::Level;
use crate::unibus::Unibus;
use crate::xbus::Xbus;

/// The five cables joining the processor board to the bus interface board,
/// both as netlists.
///
/// `data/cables.txt` gives the 92 wires pin by pin, read off MIT's wire
/// lists, and each end is found by a part pin on it rather than a name,
/// because the two readers spelt some of them differently. A wire is
/// carried the way it is driven: whichever board's own drivers hold it ---
/// [`Chip::board_level`], which leaves the cable's driver out --- the other
/// board is given that level; a wire neither drives is released at both
/// ends, and one both drive is left to each board's resolution, which is
/// what a bus conflict is.
pub struct Cables {
    /// The processor's net, the bus interface's net, and the wire's name.
    pub wires: Vec<(NetId, NetId, String)>,
    /// Wires of the table whose anchor names no pin on one of the boards.
    pub missing: Vec<String>,
    /// What each end was last given, so a step changes only what moved.
    given: Vec<(Option<Level>, Option<Level>)>,
    /// The two boards' [`Chip::generation`]s as of the last exchange:
    /// while neither has moved, no end of any wire has, and the exchange
    /// is skipped.
    seen: (u64, u64),
    /// Whether it is. Off, every wire is carried at every exchange, the
    /// slow way: [`FarEnd::unoptimised`].
    pub skip_unchanged: bool,
}

impl Cables {
    /// Joins the two boards by `data/cables.txt`. A wire that touches no
    /// part on one board --- `-LM BOOT`, which the bus interface only passes
    /// through --- is left out.
    pub fn new(cpu: &Netlist, busint: &Netlist) -> Cables {
        let find = |n: &Netlist, anchor: &str| -> Option<NetId> {
            let mut f = anchor.split_whitespace();
            let (page, reference, pin) = (f.next()?, f.next()?, f.next()?.parse::<u8>().ok()?);
            // A package's sections are separate parts under one designator.
            n.parts
                .iter()
                .filter(|p| p.page == page && p.reference == reference)
                .flat_map(|p| p.pins.iter())
                .find(|&&(k, _)| k == pin)
                .map(|&(_, net)| net)
        };
        let mut wires = Vec::new();
        let mut missing = Vec::new();
        for line in include_str!("../data/cables.txt").lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let f: Vec<&str> = line.split('|').map(str::trim).collect();
            let [_, cpu_wire, cpu_anchor, busint_wire, busint_anchor] = f.as_slice() else {
                panic!("cables.txt: {line:?}");
            };
            match (find(cpu, cpu_anchor), find(busint, busint_anchor)) {
                (Some(a), Some(b)) => wires.push((a, b, format!("{cpu_wire} / {busint_wire}"))),
                (a, b) => missing.push(format!(
                    "{cpu_wire} at {cpu_anchor}{} / {busint_wire} at {busint_anchor}{}",
                    if a.is_none() { " (none)" } else { "" },
                    if b.is_none() { " (none)" } else { "" }
                )),
            }
        }
        let given = vec![(None, None); wires.len()];
        Cables { wires, missing, given, seen: (u64::MAX, u64::MAX), skip_unchanged: true }
    }

    /// Keeps only the wires `keep` says to, by name: an instrument for
    /// finding which wire a difference rides in on.
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        let kept: Vec<bool> = self.wires.iter().map(|w| keep(&w.2)).collect();
        let mut k = kept.iter();
        self.wires.retain(|_| *k.next().unwrap());
        let mut k = kept.iter();
        self.given.retain(|_| *k.next().unwrap());
    }

    /// Carries every wire across once, and says whether any end changed.
    pub fn exchange(&mut self, cpu: &mut Chip, busint: &mut Chip) -> bool {
        let mut changed = false;
        // Taken before the wires are carried: what carrying them changes on
        // a board moves its generation, and the next exchange reads it.
        let gens = (cpu.generation(), busint.generation());
        if self.skip_unchanged && gens == self.seen {
            return false;
        }
        let since = self.seen;
        self.seen = gens;
        for (k, &(a, b, _)) in self.wires.iter().enumerate() {
            // Only a wire an end's drivers have moved on since the last
            // exchange, by the boards' own stamps; the first exchange reads
            // every wire.
            if self.skip_unchanged
                && since != (u64::MAX, u64::MAX)
                && cpu.net_stamp(a) <= since.0
                && busint.net_stamp(b) <= since.1
            {
                continue;
            }
            let (la, sa) = cpu.board_level(a);
            let (lb, sb) = busint.board_level(b);
            // What each end should be given: the other end's level when the
            // other end is driving, and nothing otherwise.
            let to_b = sa.then_some(la);
            let to_a = sb.then_some(lb);
            if self.given[k].1 != to_b {
                match to_b {
                    Some(l) => busint.drive(b, l),
                    None => busint.release(b),
                }
                self.given[k].1 = to_b;
                changed = true;
            }
            if self.given[k].0 != to_a {
                match to_a {
                    Some(l) => cpu.drive(a, l),
                    None => cpu.release(a),
                }
                self.given[k].0 = to_a;
                changed = true;
            }
        }
        changed
    }

    /// What each end was last given into a checkpoint.  It is the cables'
    /// only state, and it has to be carried: an exchange drives a wire
    /// only where what the end should be given differs from what it was
    /// given, so a resume whose cables held nothing would drive every wire
    /// again, and a board transitioned to carry a drive it already had
    /// fires the edges of that instant a second time --- a 20 ns tap on
    /// the bus interface's `XBUS ACK` delay line, in the run this was
    /// found in.
    pub fn save(&self, w: &mut crate::checkpoint::Writer) {
        w.u64(self.given.len() as u64);
        for &(a, b) in &self.given {
            w.opt(a, crate::checkpoint::Writer::level);
            w.opt(b, crate::checkpoint::Writer::level);
        }
    }

    /// Back from a checkpoint, onto cables joining the same two netlists.
    /// The boards' generations are not the ones the stamps were taken at
    /// --- a load moves a board on --- so both ends are marked unread and
    /// every wire is looked at again; with what each end was given back,
    /// none of them needs carrying.
    pub fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        let n = r.u64()? as usize;
        if n != self.given.len() {
            return Err(crate::checkpoint::bad(format!(
                "{n} wires between the boards, and these cables have {}",
                self.given.len()
            )));
        }
        for g in self.given.iter_mut() {
            let a = r.opt(crate::checkpoint::Reader::level)?;
            let b = r.opt(crate::checkpoint::Reader::level)?;
            *g = (a, b);
        }
        self.seen = (u64::MAX, u64::MAX);
        Ok(())
    }

    /// Settles the two boards against each other at `now`, with nothing
    /// behind the interface: carries the wires, lets each board respond,
    /// and again, until a pass moves nothing. [`FarEnd::join`] does this
    /// with the buses behind it.
    pub fn settle(&mut self, cpu: &mut Chip, busint: &mut Chip, now: u64) {
        for _ in 0..12 {
            if !self.exchange(cpu, busint) {
                break;
            }
            busint.transition(now);
            cpu.transition(now);
        }
    }
}

/// The debug cable between two interface boards: the debugger's DBGOUT
/// connector to the debuggee's DBGIN, the 21 wires of
/// `data/busint-connectors.txt` --- `-DEBUG OUT REQ` to `-DEBUG IN REQ`,
/// `DEBUG OUT A<1:0>` and `WR` to `DEBUG IN A<1:0>` and `WR`, `DEBUG IN ACK`
/// back to `DEBUG OUT ACK`, and `DBD<15:0>` to `DBD<15:0>`, one bus on each
/// board and both connectors on it.  Carried as [`Cables`] carries the
/// processor's: whichever board's own drivers hold a wire, the other is
/// given that level; a wire neither drives is left to each connector's
/// pull-up, which is what an open `DBD` reads as through the 8304s; one
/// both drive is left to each board's resolution.
pub struct DebugCable {
    /// The debugger's net, the debuggee's net, and the wire's name.
    pub wires: Vec<(NetId, NetId, String)>,
    given: Vec<(Option<Level>, Option<Level>)>,
    seen: (u64, u64),
}

impl DebugCable {
    /// Joins `debugger`'s DBGOUT to `debuggee`'s DBGIN, both interface
    /// netlists --- the same netlist, on two boards.
    pub fn new(debugger: &Netlist, debuggee: &Netlist) -> DebugCable {
        let find = |n: &Netlist, name: &str| {
            n.by_name_id(name)
                .or_else(|| n.by_name_id(&format!("'{name}'")))
                .unwrap_or_else(|| panic!("the interface has no {name}"))
        };
        let mut wires = Vec::new();
        for (out, into) in [
            ("-DEBUG OUT REQ", "-DEBUG IN REQ"),
            ("DEBUG OUT A0", "DEBUG IN A0"),
            ("DEBUG OUT A1", "DEBUG IN A1"),
            ("DEBUG OUT WR", "DEBUG IN WR"),
            ("DEBUG OUT ACK", "DEBUG IN ACK"),
        ] {
            wires.push((find(debugger, out), find(debuggee, into), format!("{out} / {into}")));
        }
        for k in 0..16 {
            let name = format!("DBD{k}");
            wires.push((find(debugger, &name), find(debuggee, &name), name));
        }
        let given = vec![(None, None); wires.len()];
        DebugCable { wires, given, seen: (u64::MAX, u64::MAX) }
    }

    /// Carries every wire across once, and says whether any end changed.
    pub fn exchange(&mut self, debugger: &mut Chip, debuggee: &mut Chip) -> bool {
        let mut changed = false;
        let gens = (debugger.generation(), debuggee.generation());
        if gens == self.seen {
            return false;
        }
        let since = self.seen;
        self.seen = gens;
        for (k, &(a, b, _)) in self.wires.iter().enumerate() {
            if since != (u64::MAX, u64::MAX)
                && debugger.net_stamp(a) <= since.0
                && debuggee.net_stamp(b) <= since.1
            {
                continue;
            }
            let (la, sa) = debugger.board_level(a);
            let (lb, sb) = debuggee.board_level(b);
            let to_b = sa.then_some(la);
            let to_a = sb.then_some(lb);
            if self.given[k].1 != to_b {
                match to_b {
                    Some(l) => debuggee.drive(b, l),
                    None => debuggee.pull_up(b),
                }
                self.given[k].1 = to_b;
                changed = true;
            }
            if self.given[k].0 != to_a {
                match to_a {
                    Some(l) => debugger.drive(a, l),
                    None => debugger.pull_up(a),
                }
                self.given[k].0 = to_a;
                changed = true;
            }
        }
        changed
    }
}

/// The bus interface board, powered, with its PROMs and with everything
/// but the processor's cables plugged in at rest: every Unibus, Xbus and
/// debug-cable wire of `data/busint-connectors.txt` pulled up as its
/// terminator pulls it. The master clock and the rest of the cables are the
/// processor's, through [`Cables`].
pub fn busint_board(n: &Netlist) -> Chip {
    let mut c = Chip::new_unclocked(n);
    c.load_rom(
        "0A02",
        "74S288",
        &crate::prom::parse_mit(include_str!("../mit/cadr1/reqtim.prom")).unwrap(),
    );
    c.load_rom(
        "0D09",
        "74S472",
        &crate::prom::parse_mit(include_str!("../mit/cadr1/uprior.prom")).unwrap(),
    );
    c.power_on();
    let find = |name: &str| n.by_name_id(name).or_else(|| n.by_name_id(&format!("'{name}'")));
    for line in include_str!("../data/busint-connectors.txt").lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let (_, wire) = line.split_once('|').unwrap();
        if let Some(net) = find(wire.trim()) {
            c.pull_up(net);
        }
    }
    // The grant chains are not held low as an idle daisy chain would be: in
    // local mode --- `LOCAL ENABLE`, pulled up on the board --- this board
    // is the Unibus arbiter, and its own grant goes out on `UB NPG IN` and
    // comes back to it through the 8838 at UPRIOR 0F06. Held low from
    // outside, the grant never arrived and the arbiter re-granted every
    // three microseconds for ever.
    // The reset the cpu gave it at power-on, which a board powered beside a
    // processor already running never sees on the cable: asserted, settled,
    // released. What comes up as [`Chip::power_on`] leaves it is the
    // convention every register of every board here shares; what comes up
    // as the reset leaves it is the board's.
    let resets = ["-LM POWER RESET", "-LM UNIBUS RESET"].map(|name| find(name).unwrap());
    for net in resets {
        c.drive(net, Level::Low);
    }
    c.settle_all();
    for net in resets {
        c.release(net);
    }
    c.settle();
    c
}

/// The far end of the cables as netlists: the bus interface board on the
/// five cables, the memory boards on the Xbus behind it, and the rest of
/// the Xbus and the Unibus answered from [`Machine`] by [`Buses`].
///
/// This is what `chip` runs against. `rtl` runs against the behavioural
/// interface in `src/busint.rs`, and `tests/cables.rs` holds the two to
/// each other through the boot PROM and into microcode 323.
pub struct FarEnd {
    /// The bus interface board, as [`busint_board`] builds it.
    pub board: Chip,
    pub cables: Cables,
    pub buses: Buses,
    /// The Xbus backplane and the memory boards on it.
    pub xbus: Xbus,
    /// The Unibus and the I/O board on it, if the board is a netlist;
    /// else [`Buses`] answers its registers.
    pub unibus: Option<Unibus>,
    /// `INT BUSY`: high while the interface has a cycle in flight.
    int_busy: NetId,
    /// Whether [`FarEnd::join`] has run: the first join replays what the
    /// boards owe from before it.
    joined: bool,
}

/// The boards behind the bus interface, as netlists or not: how many
/// memory boards are on the backplane, and whether the I/O board, the
/// display and the disk controller are netlists on their buses. `None` for
/// a board is [`Buses`] answering it from the machine's model.
#[derive(Clone, Copy, Default)]
pub struct Boards<'a> {
    /// Memory boards on the backplane, each 64K words; 0 is the twins.
    pub memory: usize,
    pub io: Option<&'a Netlist>,
    pub tv: Option<&'a Netlist>,
    pub disk: Option<&'a Netlist>,
    /// The DISK MULTIPLEXOR on the disk controller's cable, which is not
    /// on a bus at all: it hangs off the controller's edge connector, and
    /// with it the controller has eight drive ports instead of one. When
    /// it is here, `disk` has to be the netlist
    /// [`crate::netlist::parse_with_multiplexor`] made, and
    /// [`crate::xbus::Xbus::plug_multiplexor`] refuses it if it is not.
    pub multiplexor: Option<&'a Netlist>,
}

impl<'a> Boards<'a> {
    /// `memory` boards and nothing else as a netlist.
    pub fn memory(memory: usize) -> Boards<'a> {
        Boards { memory, ..Default::default() }
    }
}

impl FarEnd {
    /// Builds the interface board, powered and reset, `boards.memory` memory
    /// boards on the backplane, joins the cables to `cpu`'s netlist and
    /// puts `machine` behind the buses. With no boards, [`Buses`] answers
    /// memory from `machine` through the same [`crate::busint::MemoryBoard`]
    /// twins `rtl` runs, one a board, so that the timing is the board's
    /// either way; with no I/O board netlist, its registers likewise from
    /// the behavioural board on the twin's times. `chip` runs the
    /// netlists unless told otherwise (`--main-memory model`,
    /// `--io-board model`, `--tv model`, `--disk-controller model` on
    /// `muir`; `MUIR_MAIN_MEMORY=model`, `MUIR_IO_BOARD=model`,
    /// `MUIR_TV=model`, `MUIR_DISK_CONTROLLER=model` on the tests). `tv` and `disk`
    /// are the display and the disk controller as netlists on the
    /// backplane; without them [`Buses`] answers each from the machine's
    /// model. With a display board, [`Buses`] still mirrors every write to
    /// it into the model, so the screen can be read off either. The boards
    /// are powered on at `powered_at`, where their oscillators start: 0 for
    /// a machine run from the button, the join's time for a far end built
    /// fresh at a checkpoint, so that its boards do not owe every edge
    /// since power-on. Nothing has been settled against the processor yet:
    /// [`FarEnd::join`] does that.
    pub fn new(
        cpu: &Netlist,
        busint: &Netlist,
        memory: &Netlist,
        boards: Boards,
        powered_at: u64,
        machine: Machine,
    ) -> FarEnd {
        // The packs go on the netlist controller's cable as drives, the
        // same packs the model controller reads. Without a multiplexor
        // the controller has one port and only unit 0 has one to be on.
        let packs: Vec<Option<crate::disk_unit::Unit>> = match boards.disk {
            Some(_) => machine.disk.units.iter().map(Clone::clone).collect(),
            None => Vec::new(),
        };
        let chaos = machine.chaos.clone();
        let mut buses = Buses::new(busint, machine);
        buses.memory_boards = boards.memory > 0;
        buses.memory_twins = boards.memory == 0;
        buses.io_board = boards.io.is_some();
        buses.tv_board = boards.tv.is_some();
        buses.disk_board = boards.disk.is_some();
        // The device boards on the backplane, the display first.
        let devices: Vec<&Netlist> = boards.tv.into_iter().chain(boards.disk).collect();
        let mut xbus = Xbus::new(busint, memory, boards.memory, &devices, powered_at);
        if let Some(dm) = boards.multiplexor {
            xbus.plug_multiplexor(dm, powered_at);
        }
        for (u, unit) in packs.into_iter().enumerate() {
            let Some(unit) = unit else { continue };
            let drive = crate::disk_unit::Trident::new(unit, powered_at);
            xbus.plug_unit(u as u8, drive, powered_at);
        }
        // The I/O board's Chaosnet cable, carrying the hosts that are not
        // in this process if a CHUDP link was bound: one node, taking its
        // turn. Nothing else is on it --- a CADR has no file or time
        // server in it --- and [`FarEnd::attach_chaos_node`] is how a
        // caller in this process puts one there.
        let unibus = boards.io.map(|io| {
            let mut u = Unibus::new(busint, io, powered_at, chaos.address);
            let mut ether = crate::chaos::ether::Ether::new();
            if let Some(node) = chaos.udp_node() {
                ether.attach(node);
            }
            u.plug(ether, powered_at);
            // And a keyboard with nothing typed on the keyboard cable,
            // which the terminal types at.
            u.plug_keyboard(powered_at);
            u.plug_mouse(powered_at);
            u
        });
        FarEnd {
            board: busint_board(busint),
            cables: Cables::new(cpu, busint),
            buses,
            xbus,
            unibus,
            int_busy: busint.by_name_id("'INT BUSY'").expect("INT BUSY"),
            joined: false,
        }
    }

    /// Puts `node` on the I/O board's Chaosnet cable, beside the CHUDP
    /// link's if there is one: a station like any other, heard by the
    /// board and taking its turn to transmit. What
    /// [`crate::machine::Machine::attach_chaos_node`] is on the other two
    /// engines.
    ///
    /// The cable is the netlist I/O board's, so a far end built without
    /// one has none, and this panics rather than dropping the node on the
    /// floor.
    pub fn attach_chaos_node(&mut self, node: Box<dyn crate::chaos::ether::Node>) {
        self.unibus
            .as_mut()
            .and_then(|u| u.ether_mut())
            .expect("an I/O board netlist with a Chaosnet cable")
            .attach(node);
    }

    /// Settles everything against everything at `now`: the cables between
    /// the processor and the interface, the backplane between the
    /// interface and the memory boards, the behavioural far end on the
    /// buses, and again, until a pass moves nothing. After a build and
    /// after a load, before the first step; and at every step.
    pub fn join(&mut self, cpu: &mut Chip, now: u64) {
        // The first join of a far end built at power-on and joined after
        // the processor's first cycles: the boards owe every oscillator
        // edge since, and get them each at its own time, as
        // [`FarEnd::tick_with`] gives them, not folded into the join's one
        // transition --- folded, the I/O board's microsecond counter lost
        // nine `MCLK` edges and answered 563 ns off `rtl` for the rest of
        // the run.  Only what falls strictly before `now`; and only the
        // first time, the joins [`FarEnd::tick_with`] makes owing nothing.
        if !self.joined {
            self.joined = true;
            if now > 0 {
                self.xbus.transition_due(now - 1);
                if let Some(u) = self.unibus.as_mut() {
                    u.transition_due(now - 1);
                }
            }
        }
        for _ in 0..12 {
            let mut moved = self.cables.exchange(cpu, &mut self.board);
            let backplane = self.xbus.exchange(&mut self.board, &self.buses);
            let unibus = self.unibus.as_mut().is_some_and(|u| u.exchange(&mut self.board));
            if moved
                || (backplane && self.xbus.interface_changed())
                || (unibus && self.unibus.as_ref().unwrap().interface_changed())
            {
                self.board.transition(now);
            }
            if backplane {
                self.xbus.transition_changed(now);
                moved = true;
            }
            if unibus {
                self.unibus.as_mut().unwrap().transition_changed(now);
                moved = true;
            }
            if self.buses.tick(&mut self.board, now) {
                self.board.transition(now);
                moved = true;
            }
            self.dma();
            if !moved {
                break;
            }
            cpu.transition(now);
        }
    }

    /// Pages the disk controller has just put into `main` go into the
    /// memory boards' cells the same way: the controller is behavioural and
    /// so is its path to memory.
    fn dma(&mut self) {
        let pages: Vec<usize> = std::mem::take(&mut self.buses.machine.disk.dma_written);
        for page in pages {
            for a in page..page + crate::disk_unit::BLOCK_WORDS {
                self.xbus.poke(a as u32, self.buses.machine.main[a]);
            }
        }
    }

    /// When the next event is due on any board or the clock, if one is.
    pub fn next_event(&self, cpu: &Chip, clk: &dyn crate::clock::Clock) -> Option<u64> {
        let clock_next = clk.next_at(cpu.clock_inputs());
        [clock_next, self.other_next(cpu)].into_iter().flatten().min()
    }

    /// The next tap, oscillator edge or one-shot on any board.
    fn other_next(&self, cpu: &Chip) -> Option<u64> {
        [
            cpu.next_tap(),
            self.board.next_tap(),
            self.xbus.next_tap(),
            self.unibus.as_ref().and_then(|u| u.next_tap()),
            self.buses.next_event(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// One step of all the boards together: to the processor's next clock
    /// transition, or to a delay-line tap, oscillator edge or one-shot on
    /// any board if one comes first.
    pub fn tick_with(&mut self, cpu: &mut Chip, clk: &mut dyn crate::clock::Clock) -> u32 {
        let now = clk.time_ns();
        let inputs = cpu.clock_inputs();
        // `RESET` up between transitions --- the debug cable's reset bit
        // holds it --- clears the ring; the generator is then held, and
        // starts a cycle from `-TPR0` the instant the reset lifts.
        if inputs.reset {
            clk.advance(inputs);
        }
        let clock_next = clk.next_at(inputs);
        let other = self.other_next(cpu);
        let clock_first = match (clock_next, other) {
            (Some(t), Some(e)) => e >= t,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if clock_first {
            let ns = cpu.tick(clk);
            let at = clk.time_ns();
            self.join(cpu, at);
            ns
        } else if let Some(e) = other {
            let e = e.max(now);
            clk.pass(e, clock_next.is_none());
            if cpu.next_tap().is_some_and(|t| t <= e) {
                cpu.transition(e);
            }
            if self.board.next_tap().is_some_and(|t| t <= e) {
                self.board.transition(e);
            }
            self.join(cpu, e);
            // A memory board whose one-shot runs out in the instant a
            // clock edge reaches it must see both in one transition, so
            // that its synchroniser samples the line as it was, as a flop
            // does. A hang's release comes through a tap, restarts the
            // clock, and the clock's edge in that same instant is the next
            // tick's; so while one is due the boards' taps wait for it,
            // and go with it. The other way round the tap's transition
            // came first and the edge found the line already up, at
            // 1,084,441 in the band. What a board's tap raises, an
            // acknowledgement, crosses in the same instant, so the wires
            // are exchanged again after.
            if clk.next_at(cpu.clock_inputs()).is_none_or(|t| t > e) {
                self.xbus.transition_due(e);
                if let Some(u) = self.unibus.as_mut() {
                    u.transition_due(e);
                }
                self.join(cpu, e);
            }
            (e - now) as u32
        } else {
            panic!("-HANG or RESET at {now} ns with nothing on any board to end it")
        }
    }

    /// Turns every optimisation off: every board stepped at every edge of
    /// its own clock, every wire carried at every exchange. The slow way,
    /// and the reference the optimisations are held to in
    /// `tests/cables.rs`.
    pub fn unoptimised(&mut self) {
        self.cables.skip_unchanged = false;
        self.xbus.sleep = false;
        self.xbus.skip_unchanged = false;
        if let Some(u) = self.unibus.as_mut() {
            u.sleep = false;
            u.skip_unchanged = false;
        }
    }

    /// Whether the interface is between cycles: where a checkpoint can be
    /// taken.
    ///
    /// **A delay-line tap in flight is not asked about, because it is
    /// saved.** It was until format 20, when the format had no field for
    /// one; `CADRCHK7` carries them, [`Chip::save`], and a board's
    /// oscillators and one-shots were already carried before that. So
    /// what is left here is the interface's own cycle, whose answer would
    /// be owed by boards a checkpoint does not describe.
    ///
    /// **Unverified**: that the cycle is what is left is read from the
    /// refusal as it was written, not measured --- every microcycle
    /// boundary `tests/checkpoint.rs` samples in the boot PROM has `INT
    /// BUSY` low, so a checkpoint across a cycle in flight has never been
    /// tried. What would settle it is that test reaching a boundary with
    /// `INT BUSY` high and the machine loaded from it still being the
    /// machine that was never stopped.
    pub fn quiet(&self) -> bool {
        self.board.net(self.int_busy) == Level::Low
    }

    /// One word of main memory by its **physical** address, as it stands:
    /// the prompt's `mem` on `chip`.  `None` where this machine has no
    /// memory at that address.
    ///
    /// Main memory is the netlist boards' own cells where boards are on
    /// the backplane and the twins' array where they are not, so the word
    /// comes from whichever of the two is memory here.  A board's word is
    /// bits 16 to 21 of the address picking the board, 14 and 15 the bank
    /// and 0 to 13 the cell, and then a bit off each of the 32 4116s of
    /// that bank: [`crate::xbus::Xbus::peek`], the reader beside the
    /// `poke` the disk controller's DMA writes through.
    ///
    /// **Asking does not disturb a running machine.** Nothing here drives
    /// a net, advances a clock or marks a part: it is a read of cells that
    /// are already what the last write left, and not a bus cycle, so the
    /// machine is not asked for the bus and never learns it was read.
    /// What it cannot promise is a word caught in the middle of being
    /// written --- the cells are the cells, mid-write as at any other
    /// instant --- which is the caveat any probe carries.
    pub fn main_word(&self, phys: u32) -> Option<u32> {
        if self.xbus.boards.is_empty() {
            self.buses.machine.main.get(phys as usize).copied()
        } else {
            self.xbus.peek(phys)
        }
    }

    /// How many words of main memory this machine has: the boards on the
    /// backplane, or the twins' array where there are none.
    pub fn main_words(&self) -> usize {
        if self.xbus.boards.is_empty() {
            self.buses.machine.main.len()
        } else {
            self.xbus.boards.len() << 16
        }
    }

    /// Writes the interface board and then the memory boards after the
    /// processor and the clock in a checkpoint: a board built fresh is not
    /// the board that has been on the bus. The interface holds the Unibus
    /// once it has had it and its error and interrupt registers are whatever
    /// the microcode left in them; the memory holds the memory.
    pub fn save(&self, w: &mut impl std::io::Write) -> std::io::Result<()> {
        self.board.save(w)?;
        self.xbus.save(w)?;
        if let Some(u) = &self.unibus {
            u.save(w)?;
        }
        self.xbus.save_devices(w)?;
        // Last, so that a checkpoint from before there were device boards
        // or drives still reads up to here. The drives are fielded rather
        // than raw board bytes, so they go through a [`checkpoint::Writer`]
        // of their own and then in as a counted blob, self-delimiting as
        // every other field here is.
        let mut disks = crate::checkpoint::Writer::new();
        self.xbus.save_disks(&mut disks)?;
        let bytes = disks.finish();
        w.write_all(&(bytes.len() as u64).to_le_bytes())?;
        w.write_all(&bytes)
    }

    /// Reads the drives on the disk controller's cable, and the
    /// multiplexor between them, back from a checkpoint that has them;
    /// they come last, after the device boards. See [`FarEnd::save`].
    pub fn load_disks(&mut self, r: &mut impl std::io::Read) -> std::io::Result<()> {
        let mut len = [0u8; 8];
        r.read_exact(&mut len)?;
        let mut bytes = vec![0u8; u64::from_le_bytes(len) as usize];
        r.read_exact(&mut bytes)?;
        self.xbus.load_disks(&mut crate::checkpoint::Reader::new(&bytes))
    }

    /// Reads the device boards --- the display, the disk controller ---
    /// back from a checkpoint that has them; they come after the I/O
    /// board. A checkpoint from before they were stored leaves them fresh,
    /// which for the display means an empty frame buffer until the
    /// software draws again, the behavioural twin in [`Buses`] keeping the
    /// picture meanwhile.
    pub fn load_device_boards(&mut self, r: &mut impl std::io::Read) -> std::io::Result<()> {
        self.xbus.load_devices(r)
    }

    /// Reads the boards back from a checkpoint that has them; see
    /// [`Chip::load`] for what is refused. The I/O board is read last, and
    /// separately: [`FarEnd::load_io_board`].
    pub fn load(&mut self, r: &mut impl std::io::Read) -> std::io::Result<()> {
        self.board.load(r)?;
        self.xbus.load(r)
    }

    /// Reads the I/O board back from a checkpoint that has it. A checkpoint
    /// from before the board was stored leaves a fresh one, which is right
    /// for a checkpoint taken before the boot's Unibus reset at 535,764,
    /// since that reset leaves the board fresh anyway.
    pub fn load_io_board(&mut self, r: &mut impl std::io::Read) -> std::io::Result<()> {
        match &mut self.unibus {
            Some(u) => u.load(r),
            None => Ok(()),
        }
    }

    /// **The whole far end into a checkpoint**, which [`FarEnd::save`] is
    /// not: that writes the boards, and a machine picked up from it needs
    /// what is behind them too --- the machine's model with the drive and
    /// the display on it, the memory twins, and the cycle in flight,
    /// [`Buses::save`].  `muir --checkpoint` on `chip` writes this;
    /// `tests/chip.rs` writes the boards alone, because there the machine
    /// behind them is `rtl`'s and is replayed rather than stored.
    ///
    /// **Taken where [`FarEnd::quiet`] says it may be**, between cycles
    /// with nothing in flight: the caller runs on to such a point first.
    ///
    /// **A netlist disk controller comes too.** Its drives are on its own
    /// cable rather than in the machine's model, and the multiplexor
    /// between them is on the connector and not the backplane, so neither
    /// is an Xbus device and both are written after the boards:
    /// [`crate::xbus::Xbus::save_disks`].
    pub fn checkpoint(&self, w: &mut crate::checkpoint::Writer) -> std::io::Result<()> {
        self.save(w)?;
        self.cables.save(w);
        self.xbus.save_exchange(w);
        if let Some(u) = &self.unibus {
            u.save_exchange(w);
        }
        self.buses.save(w);
        Ok(())
    }

    /// The far end back from what [`FarEnd::checkpoint`] wrote, onto one
    /// built as the flags say. The boards come first, in the order they
    /// were written, and then what is behind them.
    pub fn resume(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        self.load(r)?;
        self.load_io_board(r)?;
        self.load_device_boards(r)?;
        self.load_disks(r)?;
        self.cables.load(r)?;
        self.xbus.load_exchange(r)?;
        if let Some(u) = self.unibus.as_mut() {
            u.load_exchange(r)?;
        }
        self.buses.load(r)
    }
}

// --- A netlist machine's checkpoint ------------------------------------------

/// **A netlist machine's whole state, in the one format both roads
/// write.** `muir --chip --checkpoint` writes it and `--resume` reads it;
/// `tests/chip.rs` writes it at the microcycles `MUIR_CHECKPOINT_AT`
/// names and reads it back at `MUIR_RESUME`; `tests/cables.rs` reads the
/// front of one. Before this there were two shapes --- the harness's
/// carried a bare microcycle count and the two boards, with no magic and
/// no version, and was handed `rtl`'s machine on resume --- so a file
/// either road made was no use to the other, and
/// `vendor/run/chk/at-535000.chk` had to be made by driving the harness by
/// hand.
///
/// The body is the microcycles run, the display board the backplane
/// carries, and then the processor, its clock and the far end, each
/// saving itself. It is a stream and a reader takes as much of it as it
/// wants: `muir` and the harness take all of it, `tests/cables.rs` takes
/// the processor and the clock and stops, because the far end it builds
/// is not the one the checkpoint holds.
///
/// The display board goes in by name because it is the one thing about
/// the backplane the far end does not carry: which netlist a board was
/// built from shows only as [`Chip::fingerprint`], so a resume onto the
/// other one would be refused as "a different board or a different build"
/// rather than as the flag it is. How many memory boards, and whether
/// each of main memory, the I/O board, the display and the disk
/// controller is a netlist, is [`Buses`]'s own and is refused there.
pub fn write_checkpoint(
    path: &std::path::Path,
    ran: u64,
    tv_board: &str,
    cpu: &Chip,
    clk: &Behavioural,
    far: &FarEnd,
) -> std::io::Result<u64> {
    let mut w = crate::checkpoint::Writer::new();
    w.u64(ran);
    w.bytes(tv_board.as_bytes());
    cpu.save(&mut w)?;
    clk.save(&mut w)?;
    far.checkpoint(&mut w)?;
    let boards = far.buses.machine.memory_boards();
    crate::checkpoint::write(path, ENGINE, boards, &w.finish())
}

/// The name a netlist machine's checkpoint carries in its header, which is
/// the engine that made it.
const ENGINE: &str = "chip";

/// A netlist machine's checkpoint opened: what its header and its first
/// fields say, and the rest of the body ready to read in the order
/// [`write_checkpoint`] wrote it.
pub struct Resuming<'a> {
    /// The microcycles the run had made when it was taken.
    pub ran: u64,
    /// The display board the backplane carried, `simple-tv` or `lispm-tv`.
    pub tv_board: String,
    /// How many 64K-word memory boards the machine had, off the header.
    pub memory_boards: usize,
    body: crate::checkpoint::Reader<'a>,
}

impl<'a> Resuming<'a> {
    /// The processor, which comes first.
    pub fn processor(&mut self, cpu: &mut Chip) -> std::io::Result<()> {
        cpu.load(&mut self.body)
    }

    /// Its clock, which comes after the processor.
    pub fn clock(&mut self) -> std::io::Result<Behavioural> {
        Behavioural::load(&mut self.body)
    }

    /// The far end, which comes last and is the rest of the body: the
    /// boards, what each end of each bus was given, and the machine
    /// behind them. A reader that wants only the processor and the clock
    /// simply does not call this.
    pub fn far_end(&mut self, far: &mut FarEnd) -> std::io::Result<()> {
        far.resume(&mut self.body)?;
        self.body.done()
    }
}

/// A netlist machine's checkpoint read back from the file `c` came from,
/// or why it is not one: a checkpoint of another engine is refused by
/// name here, before a board is asked to load anything.
pub fn read_checkpoint(c: &crate::checkpoint::Checkpoint) -> std::io::Result<Resuming<'_>> {
    if c.engine != ENGINE {
        return Err(crate::checkpoint::bad(format!(
            "a {} checkpoint, and this is {ENGINE}",
            c.engine
        )));
    }
    let mut body = crate::checkpoint::Reader::new(&c.body);
    let ran = body.u64()?;
    let tv_board = String::from_utf8(body.bytes()?)
        .map_err(|_| crate::checkpoint::bad("the display board's name"))?;
    Ok(Resuming { ran, tv_board, memory_boards: c.memory_boards, body })
}

// --- The debuggee's DBGIN with the debugger elsewhere ------------------------

/// The debuggee's end of a debug cable whose debugger is elsewhere: the
/// interface board's DBGIN connector with the processor, its clock and
/// the far end behind it, run as [`crate::lashup::Remote`] runs an end.
/// The debugger's request arrives as the wires at an instant and goes on
/// the connector then, the boards run an event at a time, and `DEBUG IN
/// ACK` and the word on `DBD<15:0>` come back as of the instant the board
/// raised it, exactly as the board-side harness in `tests/chip.rs` reads
/// them and `rtl` is held to.  The wires are [`DebugCable`]'s, from
/// `data/busint-connectors.txt`; the debugger's lift of the request,
/// `hold_ns` after the acknowledgement, is this end's to make, since the
/// cable carries requests, releases and acknowledgements and not the lift.
///
/// **An instant the boards have not reached is waited for, not run to.**
/// A debugger that is an `rtl` engine runs ahead of a netlist debuggee by
/// as much as the two speeds differ, and dates its requests where it
/// stands; a request dated ahead of the boards is held as `Due` and goes
/// on the connector when [`DebugIn::step_until`] brings the boards to its
/// instant, one quantum a step with the wire read between steps, and a
/// release likewise.  What is held is bounded by the protocol and not by a
/// count: the release of the request on the connector, one request after
/// it, and that request's release, three at the most, and a frame that
/// would be a fourth --- a second request while one is unanswered, or a
/// frame dated behind the one before it --- is refused as the peer's
/// error, as [`crate::lashup::Remote`] refuses the other side's messages.
/// [`DebugIn::debug_in_promise`] promises the debugger nothing
/// before a held request's own instant, so the debugger waits at it.
pub struct DebugIn {
    pub cpu: Chip,
    pub clk: Behavioural,
    pub far: FarEnd,
    /// Microcycles run: the processor's clock phase wrapping.
    pub microcycles: u64,
    /// Debug cycles taken: `DEBUG_CYCLE` requests.
    pub debug_cycles: u64,
    req: NetId,
    a0: NetId,
    a1: NetId,
    wr: NetId,
    ack: NetId,
    master: NetId,
    dbd: Vec<NetId>,
    /// The request on the connector, and when it went on.
    pending: Option<(u64, DebugRequest)>,
    /// `DEBUG IN ACK` up: when, and the word the board drove.
    answer: Option<(u64, Option<u16>)>,
    /// When the debugger lifts the request, `hold_ns` after the ack.
    lift_at: Option<u64>,
    /// What the wire has brought for instants the boards have not reached,
    /// in the order it came.
    due: VecDeque<Due>,
    phase: u32,
    spins: u32,
    spin_at: u64,
}

/// An event of the debugger's for an instant the boards have not reached:
/// its request, or the lift of one it gave up on.
#[derive(Clone, Copy, Debug)]
enum Due {
    Request(u64, DebugRequest),
    Release(u64),
}

impl Due {
    fn at(&self) -> u64 {
        match *self {
            Due::Request(at, _) | Due::Release(at) => at,
        }
    }
}

impl DebugIn {
    /// The connector on `busint`'s board, with the machine behind it.
    pub fn new(busint: &Netlist, cpu: Chip, clk: Behavioural, far: FarEnd) -> DebugIn {
        let net = |name: &str| {
            busint
                .by_name_id(name)
                .or_else(|| busint.by_name_id(&format!("'{name}'")))
                .unwrap_or_else(|| panic!("the interface has no {name}"))
        };
        let phase = clk.phase_ns();
        DebugIn {
            cpu,
            clk,
            far,
            microcycles: 0,
            debug_cycles: 0,
            req: net("-DEBUG IN REQ"),
            a0: net("DEBUG IN A0"),
            a1: net("DEBUG IN A1"),
            wr: net("DEBUG IN WR"),
            ack: net("DEBUG IN ACK"),
            master: net("DBUB MASTER"),
            dbd: (0..16).map(|k| net(&format!("DBD{k}"))).collect(),
            pending: None,
            answer: None,
            lift_at: None,
            due: VecDeque::new(),
            phase,
            spins: 0,
            spin_at: u64::MAX,
        }
    }

    /// `DBD<15:0>` as the debugger sees them: the bits the board drives as
    /// it drives them, the rest high on the cable's pull-ups; `None` when
    /// the board drives none of them.
    fn dbd(&self) -> Option<u16> {
        let (mut word, mut driven) = (0u16, 0u16);
        for (k, &net) in self.dbd.iter().enumerate() {
            let (level, strong) = self.far.board.board_level(net);
            if strong {
                driven |= 1 << k;
                if level == Level::High {
                    word |= 1 << k;
                }
            }
        }
        (driven != 0).then_some(word | !driven)
    }

    /// The board's `DEBUG IN ACK`, seen for the request on the connector.
    fn observe(&mut self) {
        // Once for the request on the connector: `lift_at` is set here and
        // cleared only by the lift or the next placement, so the
        // acknowledgement that stays up until the lift is not read again
        // --- not as this request's, and not as the held next request's
        // after `answer` has been cleared for it.
        if self.pending.is_some()
            && self.lift_at.is_none()
            && self.far.board.net(self.ack) == Level::High
        {
            let now = self.clk.time_ns();
            self.answer = Some((now, self.dbd()));
            let hold = self.pending.map(|(_, r)| r.hold_ns).unwrap_or(0);
            // The hold comes off the wire; one the clock cannot reach is a
            // lift at the end of time, and the request stays on.
            self.lift_at = Some(now.saturating_add(hold));
        }
    }

    /// The wires as the debugger leaves them between requests: the
    /// request up, `DBD` released to the pull-ups.
    fn lift(&mut self) {
        let now = self.clk.time_ns();
        self.far.board.drive(self.req, Level::High);
        for &net in &self.dbd {
            self.far.board.pull_up(net);
        }
        self.far.board.transition(now);
        self.far.join(&mut self.cpu, now);
        self.pending = None;
        self.lift_at = None;
    }

    /// `request` on the connector now: `-DEBUG IN REQ` down with the wires
    /// as the debugger drives them, `DBD` driven for a write and released
    /// to the pull-ups for a read.
    fn place(&mut self, request: DebugRequest) {
        let now = self.clk.time_ns();
        let board = &mut self.far.board;
        board.drive(self.a0, Level::from(request.strobe & 1 != 0));
        board.drive(self.a1, Level::from(request.strobe & 2 != 0));
        board.drive(self.wr, Level::from(request.write));
        for (k, &net) in self.dbd.iter().enumerate() {
            if request.write {
                board.drive(net, Level::from((request.dbd >> k) & 1 != 0));
            } else {
                board.pull_up(net);
            }
        }
        board.drive(self.req, Level::Low);
        board.transition(now);
        self.far.join(&mut self.cpu, now);
        if request.strobe == DEBUG_CYCLE {
            self.debug_cycles += 1;
        }
        self.pending = Some((now, request));
        self.answer = None;
        self.lift_at = None;
        // A register strobe is acknowledged in the instant it is made.
        self.observe();
    }

    /// Whatever has fallen due at the boards' present instant, in order: a
    /// lift whose time has come, then the wire's events from the front of
    /// the queue while their instants are now or past.  A release ahead of
    /// a held request is made before the request goes on, so the connector
    /// is free for it.
    fn settle_due(&mut self) {
        loop {
            let now = self.clk.time_ns();
            if let Some(lift) = self.lift_at
                && now >= lift
            {
                self.lift();
            }
            match self.due.front().copied() {
                Some(d) if d.at() <= now => {
                    self.due.pop_front();
                    match d {
                        Due::Request(_, request) => self.place(request),
                        Due::Release(_) => {
                            self.lift();
                            self.answer = None;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    /// The next instant something of the debugger's falls due, if one does:
    /// a lift, or the front of the queue.
    fn next_due(&self) -> u64 {
        self.lift_at.unwrap_or(u64::MAX).min(self.due.front().map_or(u64::MAX, Due::at))
    }

    /// The instant of the last event the wire brought, for the order the
    /// next one must keep: nothing yet is the boards' present.
    fn last_due(&self) -> u64 {
        self.due.back().map_or(self.clk.time_ns(), Due::at)
    }

    /// One event of the boards, or time passed to `target` when the next
    /// event is beyond it, as the board-side harness runs to a lift.
    fn advance_to(&mut self, target: u64) {
        let now = self.clk.time_ns();
        // A tick that moves no clock is a spin: a board event that stays
        // due.  Bounded, with what each board says is next.
        if now == self.spin_at {
            self.spins += 1;
            assert!(
                self.spins < 20_000,
                "no time passes at {now} ns: clock {:?}, cpu tap {:?}, interface tap {:?}",
                self.clk.next_at(self.cpu.clock_inputs()),
                self.cpu.next_tap(),
                self.far.board.next_tap()
            );
        } else {
            self.spin_at = now;
            self.spins = 0;
        }
        match self.far.next_event(&self.cpu, &self.clk) {
            Some(e) if e <= target => {
                self.far.tick_with(&mut self.cpu, &mut self.clk);
            }
            _ => {
                let held = self.clk.next_at(self.cpu.clock_inputs()).is_none();
                self.clk.pass(target, held);
                self.cpu.transition(target);
                self.far.board.transition(target);
                self.far.join(&mut self.cpu, target);
            }
        }
        let p = self.clk.phase_ns();
        if p < self.phase {
            self.microcycles += 1;
        }
        self.phase = p;
    }

    /// The boards to `limit`: a lift, a request or a release on the way is
    /// made at its instant.
    fn run_to(&mut self, limit: u64) {
        loop {
            self.settle_due();
            if self.clk.time_ns() >= limit {
                break;
            }
            let target = limit.min(self.next_due());
            self.advance_to(target);
            self.observe();
        }
    }

    /// `at` if it is not in the past, now if it is by less than a slack,
    /// as `rtl` allows the lashup's scheduling, and a refusal beyond.
    fn recent(&self, at: u64, what: &str) -> Result<u64, String> {
        let now = self.clk.time_ns();
        let slack = Speed::ExtraSlow.cycle_ns(true) as u64;
        if at.saturating_add(slack) < now {
            return Err(format!(
                "a debug {what} at {at} ns, before the board's {now} ns by more than a cycle"
            ));
        }
        Ok(at.max(now))
    }
}

impl CableEnd for DebugIn {
    fn ns(&self) -> u64 {
        self.clk.time_ns()
    }

    /// One quantum of the boards --- a generator cycle at the slowest
    /// speed, the most `rtl`'s step runs past its bound --- and never past
    /// `limit`: the boards stop at an instant, so there is no overrun.
    fn step_until(&mut self, limit: u64) -> Result<(), Halt> {
        let quantum = Speed::ExtraSlow.cycle_ns(true) as u64;
        let to = limit.min(self.clk.time_ns().saturating_add(quantum));
        self.run_to(to);
        Ok(())
    }

    /// Held for its instant and made then (`settle_due`); at
    /// once when the instant is now.  `-DEBUG OUT REQ` is one level, so a
    /// debugger has one request out at a time: one is refused while a
    /// held request waits, or while the request on the connector is
    /// neither released nor to be lifted by `at`; and one dated behind the
    /// event before it is refused as well, the wire carrying a debugger's
    /// events in its clock's order.
    fn debug_request(&mut self, at: u64, request: DebugRequest) -> Result<(), String> {
        let at = self.recent(at, "request")?;
        let busy = format!("a debug request at {at} ns while one is already on the cable");
        if self.due.iter().any(|d| matches!(d, Due::Request(..))) {
            return Err(busy);
        }
        if self.pending.is_some() {
            let released = self.due.iter().any(|d| matches!(d, Due::Release(_)));
            let lifted = self.lift_at.is_some_and(|lift| lift <= at);
            if !released && !lifted {
                return Err(busy);
            }
        }
        if at < self.last_due() {
            return Err(format!(
                "a debug request at {at} ns, behind the debugger's last event at {} ns",
                self.last_due()
            ));
        }
        self.due.push_back(Due::Request(at, request));
        // The answer to come is this request's; the last one's has gone.
        self.answer = None;
        self.settle_due();
        Ok(())
    }

    /// Held and made as [`DebugIn::debug_request`] is.  A release with no
    /// request on the connector or held is nothing on the wires, and is
    /// taken as nothing; one dated behind the event before it is refused.
    fn debug_release(&mut self, at: u64) -> Result<(), String> {
        let at = self.recent(at, "release")?;
        let outstanding = match self.due.back() {
            Some(Due::Request(..)) => true,
            Some(Due::Release(_)) => false,
            None => self.pending.is_some(),
        };
        if !outstanding {
            return Ok(());
        }
        if at < self.last_due() {
            return Err(format!(
                "a debug release at {at} ns, behind the debugger's last event at {} ns",
                self.last_due()
            ));
        }
        self.due.push_back(Due::Release(at));
        self.settle_due();
        Ok(())
    }

    fn debug_ack(&self) -> Option<(u64, Option<u16>)> {
        self.answer
    }

    /// A held request is answered no sooner than its own instant: a strobe
    /// then, a cycle `-UB MSYN` after a mastership no earlier than that.
    /// On the connector: for a cycle still short of `DBUB MASTER`, `-UB
    /// MSYN` is at least that long after the mastership, itself at a clock
    /// edge to come; a master may be answered any time; a strobe
    /// unacknowledged is not a case the board makes, and promises nothing.
    fn debug_in_promise(&self) -> u64 {
        let now = self.clk.time_ns();
        if let Some(Due::Request(at, r)) = self.due.iter().find(|d| matches!(d, Due::Request(..))) {
            return if r.strobe == DEBUG_CYCLE { at.saturating_add(DEBUG_MSYN_NS) } else { *at };
        }
        match self.pending {
            None => u64::MAX,
            Some(_) if self.answer.is_some() => self.answer.map(|(at, _)| at).unwrap_or(now),
            Some((_, r)) if r.strobe != DEBUG_CYCLE => now,
            Some(_) if self.far.board.net(self.master) == Level::High => now,
            Some(_) => now + DEBUG_MSYN_NS,
        }
    }
}
