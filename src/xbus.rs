// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Xbus backplane: the bus interface and the memory boards on one set
//! of wires.
//!
//! > The XBUS is a 32-bit-wide, asynchronous bus ... The XBUS lines are
//! > open collector, terminated at both ends.
//!
//! --- `cadr1/xspec.text.3`. Every board on it drives a line by pulling it
//! low and reads what the wired-AND of all of them and the terminators
//! leaves. [`Xbus`] carries the 72 wires of `data/busint-connectors.txt`
//! between the interface netlist and every memory board the way
//! [`crate::cable::Cables`] carries the five cables between two boards:
//! what any board's own drivers hold, every other board is given, and a
//! wire nobody holds is pulled up at every end. The behavioural far end,
//! [`Buses`], asserts on the same wires for the devices that are not
//! memory.
//!
//! The memory boards are `data/CADRM.netlist`, MIT's 64K-word board of
//! four banks of 4116s, one board for each 64K of the address space the
//! machine has, each with its address switch set to its place. Every
//! board runs its own oscillator and refreshes itself, and every board is
//! the same netlist, so their timers run in step from power-on.
//!
//! Beside them are the device boards, each its own netlist at its own
//! fixed addresses: the display, `data/SIMPLETV.netlist` or
//! `data/LISPMTV.netlist`, and the disk controller, `data/CADRDC.netlist`.
//! A device board is brought up as a memory board is --- every backplane
//! wire it has pulled up, reset over `-XBUS INIT` --- plus what its
//! drawings leave to the wrapper: its address straps (`straps`) and the
//! images in its PROMs (`roms`).

use crate::buses::Buses;
use crate::chip::Chip;
use crate::disk_unit::{OnCable, Ports, Trident};
use crate::dm::Dm;
use crate::netlist::{NetId, Netlist};
use crate::part::Level;

/// One wire of the backplane: its net on the interface, and on the memory
/// board if the board has it.
struct Wire {
    name: String,
    interface: NetId,
    board: Option<NetId>,
}

/// The memory board's DIP switch, `memxba` 0F11.
const SWITCH: &str = "0F11";

/// A net by the interface's name for it, however the board spells it. The
/// memory board and the disk controller write `-XADDR0`; the display board
/// writes `-XADDR 0`, quoted for the space, and `-XBUS0` on the same page.
/// So the exact name is tried, then quoted, then with spaces disregarded.
fn find(n: &Netlist, name: &str) -> Option<NetId> {
    n.by_name_id(name).or_else(|| n.by_name_id(&format!("'{name}'"))).or_else(|| {
        let squeezed: String = name.chars().filter(|c| *c != ' ').collect();
        n.nets
            .iter()
            .position(|net| {
                net.trim_matches('\'').chars().filter(|c| *c != ' ').eq(squeezed.chars())
            })
            .map(|i| i as NetId)
    })
}

/// The backplane's wires, as `data/busint-connectors.txt` names them on
/// the interface: `-XBUS RQ`, `-XADDR0` ...
fn wire_names() -> Vec<String> {
    include_str!("../data/busint-connectors.txt")
        .lines()
        .filter_map(|line| line.split_once('|'))
        .filter(|(connector, _)| connector.trim() == "CXBUS")
        .map(|(_, wire)| wire.trim().to_string())
        .collect()
}

/// Powers a memory board on the backplane: every Xbus wire it has pulled
/// up, its address switch set to `switches`, reset over `-XBUS INIT`, and
/// settled at time 0.
fn bring_up(memory: &Netlist, switches: u8, powered_at: u64) -> Chip {
    let mut c = Chip::new_unclocked(memory);
    c.power_on();
    for name in wire_names() {
        if let Some(net) = find(memory, &name.replace(' ', ".")) {
            c.pull_up(net);
        }
    }
    // Only the memory board has a DIP switch to set: its address above 64K.
    // The display and the disk controller answer at fixed addresses.
    if memory.parts.iter().any(|p| p.reference == SWITCH && p.kind == "DIPSW") {
        c.set_switches(SWITCH, switches);
    }
    for (net, level) in straps(memory) {
        c.drive(net, level);
    }
    for (reference, kind, image) in roms(memory) {
        c.load_rom(reference, kind, image);
    }
    let init = find(memory, "-XBUS.INIT").expect("-XBUS.INIT");
    c.drive(init, Level::Low);
    c.settle_all();
    // The oscillator starts at the board's first transition: power-on.
    c.transition(powered_at);
    c.pull_up(init);
    c.settle();
    c
}

/// The wire-wrap straps that set a board's address, where it has them.
///
/// The SIMPLE TV selects its addresses with three Am25LS2521 comparators on
/// XBADR, each pair an `ADR n` off the bus against a `DEVADR n` or
/// `MAPADR n` the drawing leaves as a one-pin net: a strap, wrapped to
/// `HI` or ground when the board was built, as the memory board's DIP
/// switch is set when it is installed. `cadrtv/lmtv.order` says what the
/// normal TV was strapped to --- the buffer at `17000000`, the control
/// registers at `173777x0` with `x` 6 --- and those are
/// [`crate::simpletv::BUFFER`] and [`crate::simpletv::CONTROL`]. `MAPADR
/// 16..21` compare the buffer's top six address bits; `DEVADR 3..21` the
/// control block's, bits 0 to 2 picking the register.
///
/// `ADR BANK SEL` and `MAPADR BANK` are the two sides of one more pair, and
/// only one of them is a strap. The LISPM TV, the board that replaced this
/// one, carries the same 25LS2521 at XBADR 0F22 and its wire list settles
/// both: `cadrtv/lmtv4b.wlr` lists the net on 0F22-05 under two names,
/// `ADR 15` and `ADR BANK SEL`, off the 74LS240 at XBADR 0F17 pin 3 --- so
/// `ADR BANK SEL` is address bit 15 off the bus, not a strap --- and it
/// puts 0F22-04, `MAPADR BANK`, on the ground net, wrapped from the `BT1`
/// ground pin through 0F22-10, -08 and -06, so `MAPADR BANK`, `MAPADR 16`
/// and `MAPADR 17` are all low. That is the buffer at `17000000`, which is
/// what `lmtv.order` says: "The normal TV has x equal to 0, so the buffer
/// starts at 17000000."
///
/// The strap is therefore right here and the bus bit is not: both are
/// driven low, where `ADR BANK SEL` should follow `ADR15`. The SIMPLE TV's
/// drawings leave `ADR BANK SEL` a one-pin net (it has no wire list of its
/// own, discrepancy 35), so joining the two is a pin move in the netlist
/// builder rather than a level here. It only matters for an address in the
/// 32K words above the ones the board has, which alias onto the buffer.
fn straps(board: &Netlist) -> Vec<(NetId, Level)> {
    let mut out = Vec::new();
    let bit = |addr: u32, k: u32| if addr >> k & 1 != 0 { Level::High } else { Level::Low };
    for k in 16..22 {
        if let Some(net) = find(board, &format!("MAPADR {k}")) {
            out.push((net, bit(crate::simpletv::BUFFER, k)));
        }
    }
    for k in 3..22 {
        if let Some(net) = find(board, &format!("DEVADR {k}")) {
            out.push((net, bit(crate::simpletv::CONTROL, k)));
        }
    }
    for name in ["ADR BANK SEL", "MAPADR BANK"] {
        if let Some(net) = find(board, name) {
            out.push((net, Level::Low));
        }
    }
    // The disk controller's six lines from the DISK MULTIPLEXOR board ---
    // `UNIT0-2`, `MULTIPLE SELECT`, `ANY ATTENTION` and `SEL UNIT
    // ATTENTION` --- are not levels but wires: the one-board jumpers of
    // `cadrdc/dc.eco`, which `Netlist::HAND_JUMPERS` puts on ground and on
    // the drive's own attention as the board is parsed.
    out
}

/// The PROMs a board carries and MIT's images for them, where the board
/// has them: what [`crate::cable::busint_board`] does for the interface's
/// two.
///
/// Each display board carries two. **The sync program** starts in a 512 x 8
/// 74S472 at `0C05` --- `SYNC PROM ENB` selects it over the eight static
/// RAMs the program can later be loaded into --- and its image is
/// `cadrtv/cpt.prom`, "PROM ;for TV SYNC" of 5 May 1980, 297 words for the
/// CPT monitor of clock mode 0. **The clock phases** come out of a 32 x 8
/// 74S288 at `0D06`, addressed by the clock counter and the mode, driving
/// `RAS`, `RASW`, `CLK-64B`, `CLK-RPT` and `CLK0` --- the frame buffer's own
/// DRAM timing.
///
/// There are three images for that 74S288 and they are not
/// interchangeable, so each board takes its own: `cadrtv/lmprom.3`, "LMTV
/// Clock PROM 74S288 8/22/78", for the SIMPLE TV, and `cadrtv/lmtv4b.prom`,
/// "4B LMTV Clock PROM", for the LISPM TV in its four-bit build ---
/// `lmtv8b.prom` being the eight-bit one, which muir does not build. The
/// unqualified image is the earlier board's, and the two qualified ones
/// name the board that replaced it; their RAS patterns differ, `lmprom.3`
/// word 3 being `364` where `lmtv4b.prom`'s is `354`.
///
/// The 74S472 takes the same image on both boards, and it is MIT's own
/// pairing. `cadrtv/lmtv.book`, "Prints and Documentation for the Lisp
/// Machine TV board", is the LISPM TV's own document set --- its drawings
/// are `GEN4B`, `CAPS`, `COLOR` ... `SYNRAM` ... `XBUS`, the four-bit
/// board's --- and the texts it prints with them are `LMTV4B PRT`, `LMTV4B
/// STF`, `LMTV4B UML`, `LMTV4B WLR`, `LMTV4B WLS`, `LMTV4B WSS`, **`CPT
/// PROM`**, `LMTV4B PROM` and `LMTV ORDER`. The clock PROM in that list is
/// the board's own image and the sync PROM is `cpt.prom`. The parts list
/// says nothing either way: `cadrtv/lmtv4b.prt` gives the 74S472 at C05
/// only "PART NAME:TBP 18S42N", where it gives the 74S288 at D06
/// "PROGRAM:LMTV4B OR LMTV8B". `cpt.prom` and `cadrtv/vmi.prom` are both
/// headed "PROM ;for TV SYNC" and differ in one word of their 297 --- word
/// 41, `65` against `175` --- so that pair is a choice of monitor and not
/// of board.
pub(crate) fn roms(board: &Netlist) -> Vec<(&'static str, &'static str, &'static [u8])> {
    use std::sync::OnceLock;
    static CPT: OnceLock<Vec<u8>> = OnceLock::new();
    static CLOCK: OnceLock<Vec<u8>> = OnceLock::new();
    static LMMODU: OnceLock<Vec<u8>> = OnceLock::new();
    static LMMYNM: OnceLock<Vec<u8>> = OnceLock::new();
    let has = |page: &str, reference: &str, kind: &str| {
        board.parts.iter().any(|p| p.page == page && p.reference == reference && p.kind == kind)
    };
    let mut out: Vec<(&'static str, &'static str, &'static [u8])> = Vec::new();
    static SIMPLE_CLOCK: OnceLock<Vec<u8>> = OnceLock::new();
    static NEWDSK: [OnceLock<Vec<u8>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];

    // The sync program, on whichever board's SYNRAM page.
    if has("NSYRAM", "0C05", "74S472") || has("SYNRAM", "0C05", "74S472") {
        let image = CPT.get_or_init(|| {
            crate::prom::parse_mit(include_str!("../mit/cadrtv/cpt.prom")).expect("cpt.prom")
        });
        out.push(("0C05", "74S472", image));
    }
    // The clock PROM, each board its own image.
    if has("NSYCLK", "0D06", "74S288") {
        let image = SIMPLE_CLOCK.get_or_init(|| {
            crate::prom::parse_mit(include_str!("../mit/cadrtv/lmprom.3")).expect("lmprom.3")
        });
        out.push(("0D06", "74S288", image));
    }
    if has("SYNCLK", "0D06", "74S288") {
        let image = CLOCK.get_or_init(|| {
            crate::prom::parse_mit(include_str!("../mit/cadrtv/lmtv4b.prom")).expect("lmtv4b.prom")
        });
        out.push(("0D06", "74S288", image));
    }
    // The I/O board's Chaosnet half has two. The modulator at LMMODU 0A10
    // is a 32 x 8 state machine clocked at 8 MHz, `chaos/lmmodu.prom`,
    // "LMMODU (NORMAL VERSION)"; MIT also left the "PRECHARGE VERSION",
    // `lmmodu.prom2`, and the table both were programmed from,
    // `lmmodu.promt`, which `tests/chaos.rs` holds the images to. Without
    // it the transmitter's output clock never ticks: the PROM's `-CK` is
    // `TOCLK^`. The address comparator's PROM at LMMYNM 0D01, 256 x 4,
    // walks the incoming destination against the switches a bit at a
    // time, `chaos/lmmynm.prom` with `lmmynm.promt` beside it.
    if has("LMMODU", "0A10", "74S288") {
        let image = LMMODU.get_or_init(|| {
            crate::prom::parse_mit(include_str!("../mit/chaos/lmmodu.prom")).expect("lmmodu.prom")
        });
        out.push(("0A10", "74S288", image));
    }
    if has("LMMYNM", "0D01", "74S287") {
        let image = LMMYNM.get_or_init(|| {
            crate::prom::parse_mit(include_str!("../mit/chaos/lmmynm.prom")).expect("lmmynm.prom")
        });
        out.push(("0D01", "74S287", image));
    }
    // The disk controller's microcode: three 74S472s on DCUI, a 512 x 24
    // control store, assembled from `cadrdc/newdsk.31` by
    // `tools/newdsk-proms.sh`. See `src/dcmicro.rs`.
    for (k, reference) in ["0D03", "0D04", "0D05"].iter().enumerate() {
        if has("DCUI", reference, "74S472") {
            let image = NEWDSK[k].get_or_init(|| {
                let text = [
                    include_str!("../data/newdsk-d03.prom"),
                    include_str!("../data/newdsk-d04.prom"),
                    include_str!("../data/newdsk-d05.prom"),
                ][k];
                crate::prom::parse_mit(text).expect("newdsk image")
            });
            out.push((reference, "74S472", image));
        }
    }
    out
}

/// A master for one board on its own: the harness that measures a board
/// without the machine. It holds the Xbus wires the interface would ---
/// open collector, pulled up at rest --- puts the 220 ns master clock the
/// interface puts on `-XBUS SYNC`, and runs a cycle as the interface runs
/// one: address, direction and the word set up, `-XBUS RQ` down until
/// `-XBUS ACK`, then released. `chip` and `now` are open for a test to
/// look at any net at any time.
pub struct XbusMaster<'a> {
    pub chip: Chip,
    pub now: u64,
    netlist: &'a Netlist,
    /// `None` on a board that does not take the master clock. The SIMPLE TV
    /// is one: it runs on its own 16 MHz dot clock and works the Xbus
    /// handshake off that, so `-XBUS.SYNC` reaches no part of it.
    sync: Option<NetId>,
    addr: Vec<NetId>,
    data: Vec<NetId>,
    rq: NetId,
    ack: NetId,
    wr: NetId,
    /// `None` on a board that does not carry parity. The SIMPLE TV is one:
    /// `cadrtv/lmtv.order`, "32K x 32 bits of video buffer memory, without
    /// parity", and its data transceiver page has no `-XBUS.PAR`.
    par: Option<NetId>,
}

impl<'a> XbusMaster<'a> {
    /// Half of the 220 ns master clock the machine boots at.
    pub const SYNC_HALF_NS: u64 = 110;

    /// Brings a board up as [`Xbus::new`] does, switched to `switches`,
    /// and lets it run two microseconds.
    pub fn new(board: &'a Netlist, switches: u8) -> XbusMaster<'a> {
        let net = |name: &str| find(board, name).unwrap_or_else(|| panic!("no net {name}"));
        let mut m = XbusMaster {
            chip: bring_up(board, switches, 0),
            now: 0,
            netlist: board,
            sync: find(board, "-XBUS.SYNC"),
            addr: (0..22).map(|k| net(&format!("-XADDR{k}"))).collect(),
            data: (0..32).map(|k| net(&format!("-XBUS{k}"))).collect(),
            rq: net("-XBUS.RQ"),
            ack: net("-XBUS.ACK"),
            wr: net("-XBUS.WR"),
            par: find(board, "-XBUS.PAR"),
        };
        m.run(2_000);
        m
    }

    /// A net on the board, by name.
    pub fn net(&self, name: &str) -> NetId {
        find(self.netlist, name).unwrap_or_else(|| panic!("no net {name}"))
    }

    /// A net's level, by name.
    pub fn level(&self, name: &str) -> Level {
        self.chip.net(self.net(name))
    }

    /// Advances to `until`, stopping at every oscillator edge, one-shot
    /// fall and Xbus clock edge on the way.
    pub fn run(&mut self, until: u64) {
        while self.now < until {
            let edge = (self.now / Self::SYNC_HALF_NS + 1) * Self::SYNC_HALF_NS;
            let next = self.chip.next_tap().map_or(edge, |t| t.min(edge)).min(until);
            self.now = next.max(self.now);
            if self.now == edge {
                let level = if (edge / Self::SYNC_HALF_NS).is_multiple_of(2) {
                    Level::Low
                } else {
                    Level::High
                };
                if let Some(sync) = self.sync {
                    self.chip.drive(sync, level);
                }
            }
            self.chip.transition(self.now);
            if next >= until {
                break;
            }
        }
    }

    /// Puts a cycle on the bus at `now`: the address, `-XBUS WR` and the
    /// word with its parity for a write, and `-XBUS RQ` down.
    pub fn request(&mut self, addr: u32, write: Option<u32>) {
        for (k, &net) in self.addr.iter().enumerate() {
            if (addr >> k) & 1 != 0 {
                self.chip.drive(net, Level::Low)
            } else {
                self.chip.pull_up(net)
            }
        }
        match write {
            Some(word) => {
                self.chip.drive(self.wr, Level::Low);
                for (k, &net) in self.data.iter().enumerate() {
                    if (word >> k) & 1 != 0 {
                        self.chip.drive(net, Level::Low)
                    } else {
                        self.chip.pull_up(net)
                    }
                }
                if let Some(par) = self.par {
                    if parity(word) {
                        self.chip.drive(par, Level::Low)
                    } else {
                        self.chip.pull_up(par)
                    }
                }
            }
            None => {
                self.chip.pull_up(self.wr);
                for &net in &self.data {
                    self.chip.pull_up(net);
                }
                if let Some(par) = self.par {
                    self.chip.pull_up(par);
                }
            }
        }
        self.chip.drive(self.rq, Level::Low);
        self.chip.transition(self.now);
    }

    /// Whether the board has answered: `-XBUS ACK` is down.
    pub fn acked(&self) -> bool {
        self.chip.net(self.ack) == Level::Low
    }

    /// The word on the data wires.
    pub fn word(&self) -> u32 {
        !(self.chip.read(&self.data) as u32)
    }

    /// Lifts `-XBUS RQ` and lets go of every wire.
    pub fn release(&mut self) {
        self.chip.pull_up(self.rq);
        self.chip.pull_up(self.wr);
        if let Some(par) = self.par {
            self.chip.pull_up(par);
        }
        for &net in &self.data {
            self.chip.pull_up(net);
        }
        self.chip.transition(self.now);
    }

    /// How long after `-XBUS ACK` the interface lifts `-XBUS RQ`: its
    /// deskew and `-LMACK` back to the processor, 30 ns as measured on the
    /// two netlists in the boot. A master that lets go in the same
    /// instant as the acknowledgement is one the board never meets.
    pub const RELEASE_NS: u64 = 30;

    /// One whole cycle from `now`, watched every 5 ns: how long the board
    /// took to acknowledge, and the word on the bus when it did. The
    /// request is lifted [`Self::RELEASE_NS`] after the acknowledgement,
    /// and the bus left alone 600 ns afterwards, for the board to finish.
    pub fn cycle(&mut self, addr: u32, write: Option<u32>) -> (u64, u32) {
        let t0 = self.now;
        self.request(addr, write);
        while !self.acked() {
            assert!(self.now < t0 + 40_000, "the board never acknowledged");
            self.run(self.now + 5);
        }
        let (took, word) = (self.now - t0, self.word());
        self.run(self.now + Self::RELEASE_NS);
        self.release();
        self.run(self.now + 600);
        (took, word)
    }
}

pub struct Xbus {
    wires: Vec<Wire>,
    /// The memory boards, board `k` at physical addresses `k << 16` up.
    pub boards: Vec<Chip>,
    /// The device boards on the same backplane --- the display, the disk
    /// controller --- each its own netlist at its own fixed addresses. They
    /// come after the memory boards in every per-end vector here, and in
    /// the checkpoint after the I/O board: [`Xbus::save_devices`].
    pub devices: Vec<Chip>,
    /// Each device board's net for each wire, if it has the wire: the
    /// display has no `-XBUS SYNC` and no `-XBUS PAR`.
    device_wires: Vec<Vec<Option<NetId>>>,
    /// The device boards' netlists, kept to bring a board up again at a
    /// resume from a checkpoint that has none: [`Xbus::repower_devices`].
    device_sources: Vec<Netlist>,
    /// What each end was last given, per wire: the interface first, then
    /// the boards. `None` is a pull-up.
    given: Vec<Vec<Option<Level>>>,
    /// The 4116 holding each bit of each bank: `drams[bank][bit]`, bit 32
    /// the parity. The same instance indices on every board.
    pub drams: [[usize; 33]; 4],
    /// Which boards the last exchange changed a net on, the interface at 0.
    changed: Vec<bool>,
    /// Board transitions made, for measuring the cost of the boards.
    pub transitions: u64,
    /// Whether a board asleep on its own clock ([`Chip::asleep`]) is left
    /// to skip its oscillator edges. Off, every board is stepped at every
    /// edge, which is the slow way and the reference: `tests/cables.rs`
    /// holds the two to the same nets at every step.
    pub sleep: bool,
    /// A board built to read a checkpoint's boards into and throw away,
    /// when none are on the backplane: a checkpoint written with boards
    /// carries as many as its count says before the I/O board.
    scratch: Option<Chip>,
    /// What the interface (0) and each board last put on each wire, a
    /// level or nothing, and the [`Chip::generation`] each was read at: a
    /// chip that has not moved since is not asked again.
    contrib: Vec<Vec<Option<Level>>>,
    seen: Vec<u64>,
    /// Which wires the behavioural far end was holding low at the last
    /// exchange.
    bus_low: Vec<bool>,
    /// Which wires some end's contribution changed on in the exchange
    /// under way: the only ones combined and given. Kept for its
    /// allocation.
    wire_dirty: Vec<bool>,
    /// The next exchange carries every wire whether or not a contribution
    /// changed: an end has been built afresh and holds nothing yet.
    recombine: bool,
    /// Whether an unchanged chip's answers are reused. Off, every chip is
    /// asked about every wire at every exchange, the slow way:
    /// [`crate::cable::FarEnd::unoptimised`].
    pub skip_unchanged: bool,
    /// The drives on the disk controller's cables, and the multiplexor
    /// between them if one is fitted. `None` until something is plugged
    /// in, which is also when the controller is looked for.
    disks: Option<Disks>,
}

/// The drives on the disk controller's cables, and the DISK MULTIPLEXOR
/// between them and the controller when one is fitted.
///
/// **The multiplexor is not an Xbus device.** It hangs off the
/// controller's edge connector, which is what [`crate::dm`] carries, and
/// not the backplane: it has no address, takes no `-XBUS` wire and answers
/// no cycle. So it sits beside [`Xbus::devices`] rather than among them,
/// and every per-end vector on the backplane is the length it was.
///
/// No drive's state and no multiplexor's is in a checkpoint. A resume
/// brings them up fresh at the resume's time, spindles at the index, which
/// is what one drive on the controller's own port already did.
struct Disks {
    /// Which device board the disk controller is.
    controller: usize,
    /// The multiplexor, and the netlist it was built from so that
    /// [`Xbus::repower_devices`] can bring the board up again.
    dm: Option<(Netlist, Dm)>,
    /// The drives by unit number. With no multiplexor the controller has
    /// one port of its own and only unit 0 can be on it.
    units: [Option<OnCable>; 8],
    /// When each drive next moves of its own accord.
    next: [Option<u64>; 8],
}

impl Disks {
    /// Lets drive `unit` see what the controller has on the cable at `now`
    /// and answer, and settles the multiplexor against the controller
    /// after it. Returns the controller transitions that took, for
    /// [`Xbus::transitions`]; the multiplexor's are [`Dm::transitions`].
    fn apply(&mut self, unit: usize, controller: &mut Chip, now: u64) -> u64 {
        self.put(unit, controller, now);
        self.settle(controller, now)
    }

    /// The same for every drive. The drives write their own nets, so one
    /// settle carries all eight rather than one settle each.
    fn apply_all(&mut self, controller: &mut Chip, now: u64) -> u64 {
        for k in 0..8 {
            self.put(k, controller, now);
        }
        self.settle(controller, now)
    }

    /// One drive's answer on to the nets, with nothing settled after it.
    fn put(&mut self, unit: usize, controller: &mut Chip, now: u64) {
        let Disks { dm, units, next, .. } = self;
        let Some(cable) = units[unit].as_mut() else { return };
        // The per-unit half of the cable is on the multiplexor and the
        // shared half on the controller, so the two boards are given
        // separately; with no multiplexor both halves are the
        // controller's and this is [`OnCable::apply`].
        match dm {
            Some((_, dm)) => cable.apply_on(&mut dm.board, controller, now),
            None => cable.apply(controller, now),
        };
        next[unit] = Some(cable.next_change(now));
    }

    /// Carries the multiplexor against the controller, if there is one.
    fn settle(&mut self, controller: &mut Chip, now: u64) -> u64 {
        match self.dm.as_mut() {
            Some((_, dm)) => dm.settle(controller, now),
            None => 0,
        }
    }

    /// The drive with the earliest event of its own, and when.
    fn earliest(&self) -> Option<(usize, u64)> {
        self.next.iter().enumerate().filter_map(|(k, d)| Some((k, (*d)?))).min_by_key(|&(_, d)| d)
    }

    /// When any drive, or the multiplexor, next moves of its own accord.
    fn next_tap(&self) -> Option<u64> {
        self.earliest()
            .map(|(_, d)| d)
            .into_iter()
            .chain(self.dm.as_ref().and_then(|(_, dm)| dm.next_tap()))
            .min()
    }
}

/// Every event the drives and the multiplexor have of their own before
/// `limit`, each at its own time; `inclusive` takes those at `limit` too.
/// Returns the controller transitions they made.
///
/// A free function because the controller's board is borrowed out of
/// [`Xbus::devices`] while the drives are borrowed out of `Xbus::disks`,
/// and those are only disjoint fields at the call site.
fn run_disks(disks: &mut Option<Disks>, controller: &mut Chip, limit: u64, inclusive: bool) -> u64 {
    let Some(d) = disks.as_mut() else { return 0 };
    let due = |t: u64| if inclusive { t <= limit } else { t < limit };
    let mut n = 0;
    let mut steps = 0;
    loop {
        let drive = d.earliest().filter(|&(_, t)| due(t));
        let board = d.dm.as_ref().and_then(|(_, dm)| dm.next_tap()).filter(|&t| due(t));
        match (drive, board) {
            // Whichever is earlier goes first, and a drive first where
            // they fall together: the multiplexor's answer to an edge is
            // the same microcycle's, and its tap at that instant is not.
            (Some((k, t)), b) if b.is_none_or(|d| t <= d) => n += d.apply(k, controller, t),
            (_, Some(t)) => {
                if let Some((_, dm)) = d.dm.as_mut() {
                    dm.transition_due(t);
                }
                n += d.settle(controller, t);
            }
            // Nothing of either's is due by `limit`.
            _ => break,
        }
        steps += 1;
        assert!(steps < 1_000_000, "the drives' events never run out at {limit}");
    }
    n
}

impl Xbus {
    /// Builds `boards` memory boards on the backplane beside the interface
    /// netlist `busint`: powered, switched to their addresses, reset over
    /// `-XBUS INIT`, and settled.
    pub fn new(
        busint: &Netlist,
        memory: &Netlist,
        boards: usize,
        devices: &[&Netlist],
        powered_at: u64,
    ) -> Xbus {
        let mut wires = Vec::new();
        for name in wire_names() {
            let Some(interface) = find(busint, &name) else { continue };
            // The memory board spells `-XBUS RQ` as `-XBUS.RQ`.
            let board = find(memory, &name.replace(' ', "."));
            wires.push(Wire { name, interface, board });
        }
        // `-XBUS EXTGRANT OUT` leaves the interface and arrives at a
        // device as `-XBUS.EXTGRANT.IN`: `xspec.text.3`'s daisy chain,
        // which one device makes a wire of. A device's own `EXTGRANT.OUT`
        // would carry the grant on to the next in the chain, and with one
        // device that can take the bus it goes nowhere.
        let device_wires: Vec<Vec<Option<NetId>>> = devices
            .iter()
            .map(|d| {
                wires
                    .iter()
                    .map(|w| match w.name.as_str() {
                        "-XBUS EXTGRANT OUT" => find(d, "-XBUS.EXTGRANT.IN"),
                        name => find(d, &name.replace(' ', ".")),
                    })
                    .collect()
            })
            .collect();
        let masters = devices.iter().filter(|d| find(d, "-XBUS.EXTGRANT.IN").is_some()).count();
        assert!(masters <= 1, "the grant chain is one wire here: one device can take the bus");
        assert!(boards <= 64, "the switch has six bits");
        let mut drams = [[usize::MAX; 33]; 4];
        for (i, p) in memory.parts.iter().enumerate() {
            if p.kind != "4116VG" {
                continue;
            }
            let bank: usize =
                p.page.strip_prefix("MEM").and_then(|s| s[..1].parse().ok()).expect("a bank page");
            let din = memory.net(p.pins.iter().find(|&&(k, _)| k == 2).unwrap().1);
            let bit: usize = din
                .trim_matches('\'')
                .strip_prefix("XBI ")
                .and_then(|s| s.parse().ok())
                .expect("XBI n");
            drams[bank][bit] = i;
        }
        assert!(
            drams.iter().flatten().all(|&i| i != usize::MAX),
            "every bit of every bank has a DRAM"
        );
        let chips: Vec<Chip> = (0..boards).map(|k| bring_up(memory, k as u8, powered_at)).collect();
        let device_chips: Vec<Chip> = devices.iter().map(|d| bring_up(d, 0, powered_at)).collect();
        let scratch = (boards == 0).then(|| bring_up(memory, 0, powered_at));
        let ends = 1 + boards + devices.len();
        let given = vec![vec![None; ends]; wires.len()];
        let contrib = vec![vec![None; wires.len()]; ends];
        let bus_low = vec![false; wires.len()];
        let wire_dirty = vec![false; wires.len()];
        Xbus {
            wire_dirty,
            recombine: true,
            wires,
            boards: chips,
            devices: device_chips,
            device_wires,
            device_sources: devices.iter().map(|d| (*d).clone()).collect(),
            given,
            drams,
            changed: vec![false; ends],
            transitions: 0,
            sleep: true,
            scratch,
            contrib,
            seen: vec![u64::MAX; ends],
            bus_low,
            skip_unchanged: true,
            disks: None,
        }
    }

    /// The disk controller's drives, made when the first thing is plugged
    /// in. Panics if no device board has the drive cables.
    fn disks(&mut self) -> &mut Disks {
        if self.disks.is_none() {
            let controller = self
                .device_sources
                .iter()
                .position(OnCable::fits)
                .expect("a disk controller on the backplane to plug a drive into");
            self.disks = Some(Disks {
                controller,
                dm: None,
                units: std::array::from_fn(|_| None),
                next: [None; 8],
            });
        }
        self.disks.as_mut().expect("just made")
    }

    /// Puts a drive on the disk controller's cables at `now`, in unit 0's
    /// place: the one port the controller has of its own.
    pub fn plug(&mut self, drive: Trident, now: u64) {
        self.plug_unit(0, drive, now);
    }

    /// Puts a DISK MULTIPLEXOR on the controller's edge connector, so that
    /// the eight ports [`Xbus::plug_unit`] fills are that board's.
    ///
    /// The controller's netlist has to be the one
    /// [`crate::netlist::parse_with_multiplexor`] made --- the one whose
    /// six one-board jumpers are left off, so that the nets the
    /// multiplexor drives are its to drive rather than tied to ground and
    /// to each other. The drives go on after this, their ports being the
    /// multiplexor's.
    pub fn plug_multiplexor(&mut self, dm: &Netlist, powered_at: u64) {
        let j = self.disks().controller;
        let d = self.disks.as_ref().expect("just made");
        assert!(d.dm.is_none(), "one multiplexor on the controller's cable");
        assert!(
            d.units.iter().all(Option::is_none),
            "the multiplexor goes on before the drives: their ports are its"
        );
        // A controller parsed the ordinary way has `UNIT0` joined to
        // `GND` by the one-board jumper at `EP2 : ER2`, and five more like
        // it. The multiplexor would then be driving nets tied to ground
        // and to each other, and would go on running as if it were not:
        // the same shape of silence as indexing the wrong board.
        let dc = &self.device_sources[j];
        assert!(
            find(dc, "UNIT0") != find(dc, "GND"),
            "the controller's netlist is not the one `parse_with_multiplexor` makes: \
             its one-board jumpers are still on and the multiplexor has nothing to drive"
        );
        let cable = Dm::new(dc, dm, powered_at);
        self.disks().dm = Some((dm.clone(), cable));
        let n = self.disks.as_mut().expect("just made").settle(&mut self.devices[j], powered_at);
        self.transitions += n;
    }

    /// Puts a drive on `unit`'s port at `now`. Without a multiplexor the
    /// controller has one port and only unit 0 has one to be on.
    pub fn plug_unit(&mut self, unit: u8, drive: Trident, now: u64) {
        let unit = usize::from(unit);
        let j = self.disks().controller;
        let d = self.disks.as_ref().expect("just made");
        assert!(unit < 8, "the multiplexor's ports are units 0 to 7");
        assert!(d.units[unit].is_none(), "unit {unit} is taken");
        assert!(
            unit == 0 || d.dm.is_some(),
            "unit {unit} wants a multiplexor: the controller has one port"
        );
        let per_unit = d.dm.as_ref().map_or(&self.device_sources[j], |(n, _)| n);
        let ports = Ports { per_unit, unit: unit as u8, shared: &self.device_sources[j] };
        let cable = OnCable::on(ports, drive);
        self.disks().units[unit] = Some(cable);
        let n = self.disks.as_mut().expect("just made").apply(unit, &mut self.devices[j], now);
        self.transitions += n;
    }

    /// The drive in unit 0's place, if one is plugged in.
    pub fn trident(&self) -> Option<&Trident> {
        self.unit(0)
    }

    /// The drive on `unit`'s port, if one is plugged in.
    pub fn unit(&self, unit: u8) -> Option<&Trident> {
        self.disks.as_ref()?.units[usize::from(unit)].as_ref().map(|c| &c.drive)
    }

    /// Lets every drive see what the controller has on the cables at `now`
    /// and answer; the boards transition if that moved a net.
    fn apply_drives(&mut self, now: u64) {
        let Some(d) = self.disks.as_mut() else { return };
        let n = d.apply_all(&mut self.devices[d.controller], now);
        self.transitions += n;
    }

    /// Carries every wire across once: the wired-AND of what the interface,
    /// every board and the behavioural far end hold, given to every end.
    /// Returns whether anything changed; `Xbus::changed` says where.
    pub fn exchange(&mut self, interface: &mut Chip, buses: &Buses) -> bool {
        let mut moved = false;
        self.changed.iter_mut().for_each(|c| *c = false);
        // A chip whose generation has not moved since it was last asked
        // puts the same on every wire as it did then; one that has is
        // asked only about the wires its stamps say moved since, and only
        // a wire some end's contribution changed on is carried.
        let skip = self.skip_unchanged;
        let all = !skip || std::mem::take(&mut self.recombine);
        self.wire_dirty.iter_mut().for_each(|d| *d = all);
        let mut dirty = std::mem::take(&mut self.wire_dirty);
        let wires = &self.wires;
        refresh_end(interface, &mut self.seen[0], &mut self.contrib[0], &mut dirty, skip, |k| {
            Some(wires[k].interface)
        });
        for (i, b) in self.boards.iter().enumerate() {
            let e = i + 1;
            refresh_end(b, &mut self.seen[e], &mut self.contrib[e], &mut dirty, skip, |k| {
                wires[k].board
            });
        }
        for (j, d) in self.devices.iter().enumerate() {
            let e = 1 + self.boards.len() + j;
            let nets = &self.device_wires[j];
            refresh_end(d, &mut self.seen[e], &mut self.contrib[e], &mut dirty, skip, |k| nets[k]);
        }
        for (k, w) in self.wires.iter().enumerate() {
            let low = buses.asserting(w.interface) == Some(Level::Low);
            if low != self.bus_low[k] {
                self.bus_low[k] = low;
                dirty[k] = true;
            }
        }
        for (k, w) in self.wires.iter().enumerate() {
            if !dirty[k] {
                // Nothing has moved at any end of it since the last
                // exchange left every end given what it should be.
                continue;
            }
            let mut low = self.bus_low[k];
            let mut high = false;
            for c in &self.contrib {
                match c[k] {
                    Some(Level::Low) => low = true,
                    Some(Level::High) => high = true,
                    _ => {}
                }
            }
            // Open collector: a low anywhere is a low everywhere.
            let level = if low {
                Some(Level::Low)
            } else if high {
                Some(Level::High)
            } else {
                None
            };
            let give = |c: &mut Chip, net: NetId, to: Option<Level>| match to {
                Some(l) => c.drive(net, l),
                None => c.pull_up(net),
            };
            if self.given[k][0] != level {
                give(interface, w.interface, level);
                self.given[k][0] = level;
                self.changed[0] = true;
                moved = true;
            }
            if let Some(net) = w.board {
                for (i, b) in self.boards.iter_mut().enumerate() {
                    if self.given[k][i + 1] != level {
                        give(b, net, level);
                        self.given[k][i + 1] = level;
                        self.changed[i + 1] = true;
                        moved = true;
                    }
                }
            }
            let first_device = 1 + self.boards.len();
            for (j, d) in self.devices.iter_mut().enumerate() {
                let Some(net) = self.device_wires[j][k] else { continue };
                let e = first_device + j;
                if self.given[k][e] != level {
                    give(d, net, level);
                    self.given[k][e] = level;
                    self.changed[e] = true;
                    moved = true;
                }
            }
        }
        self.wire_dirty = dirty;
        moved
    }

    /// Whether the last exchange changed a net on the interface.
    pub fn interface_changed(&self) -> bool {
        self.changed[0]
    }

    /// Lets every board the last exchange touched respond.
    pub fn transition_changed(&mut self, now: u64) {
        for (i, b) in self.boards.iter_mut().enumerate() {
            if self.changed[i + 1] {
                b.transition(now);
                self.transitions += 1;
            }
        }
        let first_device = 1 + self.boards.len();
        let mut controller_moved = false;
        for (j, d) in self.devices.iter_mut().enumerate() {
            if self.changed[first_device + j] {
                d.transition(now);
                self.transitions += 1;
                if self.disks.as_ref().is_some_and(|d| d.controller == j) {
                    controller_moved = true;
                }
            }
        }
        if controller_moved {
            self.apply_drives(now);
        }
    }

    /// When a board next needs a transition: its next oscillator edge, or,
    /// for a board asleep on its own clock, whatever else would wake it.
    fn due(&self, b: &Chip) -> Option<u64> {
        if self.sleep && b.asleep() { b.next_wake() } else { b.next_tap() }
    }

    /// When a board, or the drive on the controller's cables, will next do
    /// something of its own accord.
    pub fn next_tap(&self) -> Option<u64> {
        self.boards
            .iter()
            .chain(&self.devices)
            .filter_map(|b| self.due(b))
            .chain(self.disks.as_ref().and_then(Disks::next_tap))
            .min()
    }

    /// Lets every board have every event due by `now`, each at its own
    /// time, as [`crate::unibus::Unibus::transition_due`] does: a board
    /// stepped late is not a sleeping board, and does not skip its edges.
    pub fn transition_due(&mut self, now: u64) {
        let sleep = self.sleep;
        let due = |b: &Chip| if sleep && b.asleep() { b.next_wake() } else { b.next_tap() };
        let boards = self.boards.len();
        let controller = self.disks.as_ref().map(|d| boards + d.controller);
        for (i, b) in self.boards.iter_mut().chain(&mut self.devices).enumerate() {
            let mut n = 0;
            while let Some(t) = due(b)
                && t <= now
            {
                // The drives' edges, and the multiplexor's, that come
                // before this tap reach the controller first, each at its
                // own time.
                if Some(i) == controller {
                    self.transitions += run_disks(&mut self.disks, b, t, false);
                }
                b.transition(t);
                self.transitions += 1;
                if Some(i) == controller
                    && let Some(d) = self.disks.as_mut()
                {
                    self.transitions += d.apply_all(b, t);
                }
                n += 1;
                assert!(
                    n < 1_000_000,
                    "{}'s events never run out at {now}: due {:?}, tap {:?}, wake {:?}, edges {:?}",
                    if i < boards {
                        format!("memory board {i}")
                    } else {
                        format!("device board {}", i - boards)
                    },
                    due(b),
                    b.next_tap(),
                    b.next_wake(),
                    b.oscillator_next()
                );
            }
        }
        // The drives' edges and the multiplexor's up to `now` that no tap
        // of the controller's came after.
        if let Some(j) = self.disks.as_ref().map(|d| d.controller) {
            self.transitions += run_disks(&mut self.disks, &mut self.devices[j], now, true);
        }
    }

    /// Whether any board has a delay-line tap in flight.
    pub fn taps_pending(&self) -> bool {
        self.boards.iter().chain(&self.devices).any(|b| b.taps_pending())
            || self.multiplexor().is_some_and(|dm| dm.board.taps_pending())
    }

    /// The multiplexor on the controller's cable, if one is fitted. Read
    /// only: what is on its ports is the board's own doing and a caller
    /// that drove them would be deciding what the drives decide.
    pub fn multiplexor(&self) -> Option<&Dm> {
        self.disks.as_ref()?.dm.as_ref().map(|(_, dm)| dm)
    }

    /// What each end of the backplane last put on every wire and was last
    /// given on it, into a checkpoint.  The boards themselves are
    /// netlists and save themselves, [`Xbus::save`]; this is what the
    /// backplane holds of them, and it has to be carried for the same
    /// reason [`crate::cable::Cables::save`] does --- a wire is driven
    /// only where what an end should be given differs from what it was,
    /// so a backplane that held nothing would drive every wire again and
    /// fire that instant's edges twice.
    pub fn save_exchange(&self, w: &mut crate::checkpoint::Writer) {
        w.u64(self.wires.len() as u64);
        w.u64(self.contrib.len() as u64);
        for end in &self.contrib {
            for &c in end {
                w.opt(c, crate::checkpoint::Writer::level);
            }
        }
        for wire in &self.given {
            for &g in wire {
                w.opt(g, crate::checkpoint::Writer::level);
            }
        }
        for &low in &self.bus_low {
            w.bool(low);
        }
        for &c in &self.changed {
            w.bool(c);
        }
        w.bool(self.recombine);
    }

    /// Back from a checkpoint, onto a backplane with the same boards on
    /// it.  Every end is marked unread --- a board's generation is not
    /// what it was when its contribution was stamped --- so all of them
    /// are asked again; asked, they answer what the checkpoint holds, and
    /// no wire needs carrying.
    pub fn load_exchange(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        let bad = crate::checkpoint::bad;
        let wires = r.u64()? as usize;
        let ends = r.u64()? as usize;
        if (wires, ends) != (self.wires.len(), self.contrib.len()) {
            return Err(bad(format!(
                "{wires} wires and {ends} ends on the backplane, and this one has {} and {}",
                self.wires.len(),
                self.contrib.len()
            )));
        }
        for end in self.contrib.iter_mut() {
            for c in end.iter_mut() {
                *c = r.opt(crate::checkpoint::Reader::level)?;
            }
        }
        for wire in self.given.iter_mut() {
            for g in wire.iter_mut() {
                *g = r.opt(crate::checkpoint::Reader::level)?;
            }
        }
        for low in self.bus_low.iter_mut() {
            *low = r.bool()?;
        }
        for c in self.changed.iter_mut() {
            *c = r.bool()?;
        }
        self.recombine = r.bool()?;
        self.seen.iter_mut().for_each(|s| *s = u64::MAX);
        Ok(())
    }

    /// Writes the device boards, for the end of a checkpoint: after the
    /// interface, the memory boards and the I/O board, so that a checkpoint
    /// from before there were any still reads.
    pub fn save_devices(&self, w: &mut impl std::io::Write) -> std::io::Result<()> {
        for d in &self.devices {
            d.save(w)?;
        }
        Ok(())
    }

    /// Reads the device boards back; see [`Xbus::save_devices`].
    pub fn load_devices(&mut self, r: &mut impl std::io::Read) -> std::io::Result<()> {
        for d in &mut self.devices {
            d.load(r)?;
        }
        Ok(())
    }

    /// Brings every device board up again, powered on at `powered_at`: what
    /// a resume does when the checkpoint was written before the boards were
    /// stored, so that a board built fresh at time 0 does not owe every
    /// edge of its oscillator since --- 26 million of them for the display
    /// at microcycle 1.4 million. The memory boards' timers come out of the
    /// checkpoint; a fresh device board's start here.
    pub fn repower_devices(&mut self, powered_at: u64) {
        let first = 1 + self.boards.len();
        self.recombine = true;
        for (j, src) in self.device_sources.iter().enumerate() {
            self.devices[j] = bring_up(src, 0, powered_at);
            let e = first + j;
            self.seen[e] = u64::MAX;
            self.changed[e] = false;
            for k in 0..self.wires.len() {
                self.given[k][e] = None;
                self.contrib[e][k] = None;
            }
        }
        if let Some(d) = self.disks.as_mut() {
            let controller = &self.device_sources[d.controller];
            if let Some((src, dm)) = d.dm.as_mut() {
                *dm = Dm::new(controller, src, powered_at);
            }
            for cable in d.units.iter_mut().flatten() {
                cable.reattach();
            }
        }
        self.apply_drives(powered_at);
    }

    /// The board, bank and cell a physical address names, if a board is
    /// there: bits 16 to 21 pick the board, 14 and 15 the bank, and 0 to
    /// 13 the row and column.
    fn locate(&self, phys: u32) -> Option<(usize, usize, usize)> {
        let board = (phys >> 16) as usize & 0x3f;
        (board < self.boards.len()).then_some((
            board,
            (phys >> 14) as usize & 3,
            phys as usize & 0x3fff,
        ))
    }

    /// Writes a word straight into the cells, as a bus master with its own
    /// path to memory would have put it there: the disk controller's DMA.
    /// The parity bit is [`parity`]'s.
    pub fn poke(&mut self, phys: u32, word: u32) {
        let Some((board, bank, a)) = self.locate(phys) else { return };
        for bit in 0..32 {
            self.boards[board].set_cell_bit(self.drams[bank][bit], a, word >> bit & 1 != 0);
        }
        self.boards[board].set_cell_bit(self.drams[bank][32], a, parity(word));
    }

    /// Fills every board from a copy of main memory: what a resume does
    /// when the checkpoint was written before the boards were stored, so
    /// the cells hold what `rtl`'s memory holds.
    pub fn load_from(&mut self, main: &[u32]) {
        let words = (self.boards.len() << 16).min(main.len());
        for (a, &word) in main[..words].iter().enumerate() {
            self.poke(a as u32, word);
        }
    }

    /// Reads a word straight out of the cells.
    pub fn peek(&self, phys: u32) -> Option<u32> {
        let (board, bank, a) = self.locate(phys)?;
        let mut word = 0;
        for bit in 0..32 {
            if self.boards[board].cell_bit(self.drams[bank][bit], a)? {
                word |= 1 << bit;
            }
        }
        Some(word)
    }

    /// The parity cell of a word, for the harness.
    pub fn peek_parity(&self, phys: u32) -> Option<bool> {
        let (board, bank, a) = self.locate(phys)?;
        self.boards[board].cell_bit(self.drams[bank][32], a)
    }

    /// The memory boards into a checkpoint: how many there are, then each
    /// board.  The count is what lets a resume read past them --- a machine
    /// may have any number from none to sixty, and the I/O board and the
    /// device boards follow them in the stream.
    pub fn save(&self, w: &mut impl std::io::Write) -> std::io::Result<()> {
        w.write_all(BOARDS_MAGIC)?;
        w.write_all(&(self.boards.len() as u32).to_le_bytes())?;
        for b in &self.boards {
            b.save(w)?;
        }
        Ok(())
    }

    /// Reads back what [`Xbus::save`] wrote.  A backplane with boards takes
    /// exactly as many as the checkpoint has and refuses another count;
    /// one whose main memory is the twins reads the checkpoint's boards
    /// past into `scratch`, however many, to reach what follows them.
    pub fn load(&mut self, r: &mut impl std::io::Read) -> std::io::Result<()> {
        let bad = |what: String| std::io::Error::new(std::io::ErrorKind::InvalidData, what);
        let mut head = [0u8; 8];
        r.read_exact(&mut head)?;
        if &head[..4] != BOARDS_MAGIC {
            return Err(bad("checkpoint: the memory boards carry no count before them".into()));
        }
        let saved = u32::from_le_bytes(head[4..].try_into().unwrap()) as usize;
        if let Some(scratch) = self.scratch.as_mut() {
            for _ in 0..saved {
                scratch.load(r)?;
            }
            return Ok(());
        }
        if saved != self.boards.len() {
            return Err(bad(format!(
                "checkpoint: {saved} memory boards, and this backplane has {}",
                self.boards.len()
            )));
        }
        for b in &mut self.boards {
            b.load(r)?;
        }
        Ok(())
    }
}

/// What [`Xbus::save`] writes before the board count.
const BOARDS_MAGIC: &[u8; 4] = b"XBUS";

/// What a chip puts on a wire: a level while it is driving, nothing while
/// it is not.
fn put((level, strong): (Level, bool)) -> Option<Level> {
    match (strong, level) {
        (true, Level::Low) => Some(Level::Low),
        (true, Level::High) => Some(Level::High),
        _ => None,
    }
}

/// Reads one end's contribution to every wire again where it may have
/// changed: not at all while the chip's generation stands where it was
/// last read, and otherwise only on the wires whose [`Chip::net_stamp`]
/// is past that reading. A wire whose contribution changed is marked in
/// `dirty`. With `skip` off every wire is read every time, the slow way.
fn refresh_end(
    chip: &Chip,
    seen: &mut u64,
    contrib: &mut [Option<Level>],
    dirty: &mut [bool],
    skip: bool,
    net_of: impl Fn(usize) -> Option<NetId>,
) {
    let now = chip.generation();
    if skip && now == *seen {
        return;
    }
    let since = (*seen != u64::MAX).then_some(*seen);
    *seen = now;
    for (k, c) in contrib.iter_mut().enumerate() {
        let Some(net) = net_of(k) else { continue };
        if skip && since.is_some_and(|s| chip.net_stamp(net) <= s) {
            continue;
        }
        let level = put(chip.board_level(net));
        if *c != level {
            *c = level;
            dirty[k] = true;
        }
    }
}

/// The parity bit the bus interface stores with a word: odd parity, the bit
/// that makes the count of ones in the 33 odd. Measured in `tests/cables.rs`
/// by letting the boot's PAGE-0-PARITY-FIX write page 0 through the
/// interface and reading the cells.
pub fn parity(word: u32) -> bool {
    word.count_ones().is_multiple_of(2)
}
