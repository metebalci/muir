// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The `chip` engine: the board as its parts.
//!
//! Every other engine collapses something.  This one does not: it takes the
//! packages out of [`crate::netlist`], gives each the behaviour
//! [`crate::part`] records for its type, and resolves every net from what is
//! actually driving it.  It is the slowest of the three and the one the other
//! two are checked against, so it is allowed no opinions.
//!
//! The design is acyclic once dependencies are taken pin by pin --- which
//! output each input actually reaches --- so the gates can be sorted once and
//! evaluated in order, with no event queue.  Taken package-wide the same
//! dependencies close loops the hardware does not have.  The real cycles
//! are few: the clock generator, which `src/clock.rs` models instead, and
//! the combinational feedback `levelize` finds and iterates --- the trap and
//! NOP interlock, and the SPY bus, which is read and written on the same
//! wires.

use std::collections::HashMap;

use crate::clock;
use crate::netlist::{NetId, Netlist};
use crate::part::{self, Behaviour, Drive, Driver, Level, Pins, State};

/// One gate of one package: which package, and which of its gates.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct GateId {
    pub part: u32,
    pub gate: u16,
}

/// One package on the board.
pub struct Instance {
    pub page: String,
    pub reference: String,
    pub kind: String,
    behaviour: Behaviour,
    /// Net on each pin, indexed by pin number.
    net_of: [Option<NetId>; crate::part::MAX_PINS],
    /// How each pin drives, indexed by pin number, from the part's pinout
    /// overrides already applied. Worked out once: [`part::pinout`] matches
    /// on the type name, and [`Chip::resolve_net`] would otherwise do that
    /// string match every time it evaluated a gate.
    drive_of: [Drive; crate::part::MAX_PINS],
    /// What the part holds.
    pub state: State,
}

impl Instance {
    /// How many gates this package has.
    pub fn gate_count(&self) -> usize {
        self.behaviour.gates.len()
    }

    /// The net on a pin, if it is wired.
    pub fn net_on(&self, pin: u8) -> Option<NetId> {
        self.net_of[pin as usize]
    }

    /// The output pin of one of this package's gates.
    pub fn gate_out(&self, gate: u16) -> u8 {
        self.behaviour.gates[gate as usize].out
    }

    /// The input pins of one of this package's gates.
    pub fn gate_ins(&self, gate: u16) -> &'static [u8] {
        self.behaviour.gates[gate as usize].ins
    }

    pub fn name(&self) -> String {
        format!("{} {} ({})", self.page, self.reference, self.kind)
    }
}

pub struct Chip {
    pub instances: Vec<Instance>,
    /// What each net starts at: a supply level, or high impedance.
    supplies: Vec<Level>,
    /// The nets the clock model drives, and which of its outputs each takes.
    clock_nets: Vec<(NetId, ClockOut)>,
    /// The nets the clock model reads back off the board.
    /// [`None`] on a board with no clock generator of its own. The bus
    /// interface is one: its clock arrives over the cables, and the only
    /// oscillator on it is the 74LS124 that times the NXM timeout out.
    clock_in: Option<ClockIn>,
    /// The PROMs that are part of the machine, and what they hold; see
    /// [`fixed_proms`].
    fixed: Vec<(usize, Vec<u8>)>,
    /// What each RAM's cells come up holding; see [`power_on_fill`].
    fill: Vec<(usize, u8)>,
    /// The level on every net, indexed by [`NetId`].
    nets: Vec<Level>,
    /// Every gate driving each net.
    drivers: Vec<Vec<GateId>>,
    /// Every gate *reading* each net, by flat index. The other half of
    /// `drivers`, and what makes [`Chip::settle`] incremental.
    readers: Vec<Vec<u32>>,
    /// Where each instance's gates start in the flat numbering.
    gate_base: Vec<u32>,
    /// Which gates need evaluating. A gate is dirty when a net it reads has
    /// moved, or when the part it belongs to has changed what it holds.
    dirty: Vec<bool>,
    /// How many of `dirty` are set among the gates settling evaluates, so
    /// that it can stop when none is.
    dirty_count: usize,
    /// Which gates settling evaluates: those levelization placed. A gate
    /// it left out --- a delayed gate --- is never counted dirty.
    evaluated: Vec<bool>,
    /// The delay lines on VCTL1; see [`DELAY_LINES`]. Their pending
    /// transitions are not saved in a checkpoint: a line is at most 250 ns
    /// long, and a checkpoint is taken between microcycles.
    delays: Vec<Delay>,
    /// The board's one-shots; see [`OneShot`].
    one_shots: Vec<OneShot>,
    /// The board's own oscillators; see [`Oscillator`]. None on the
    /// processor.
    oscillators: Vec<Oscillator>,
    /// What the cables are driving onto the board.
    ///
    /// The bus interface is a different board, so its drivers are not in this
    /// netlist: `-MEMACK` and its fellows arrive on the connector and are
    /// pulled up by the SIP pack at BCTERM 2C25, and `MEM<31:0>` is driven
    /// from the far end during a read. An external driver joins the
    /// resolution like any other, which is what makes it survive a settle
    /// where [`Chip::set_net`] does not.
    external: Vec<Option<Driver>>,
    /// Every net as it stood before this clock transition; see
    /// [`Chip::snapshot`].
    prev_nets: Vec<Level>,
    /// No transition has happened since power-on: the first one has nothing
    /// to find edges against, so it takes the board as it stands before it
    /// moves the clock. A checkpoint carries its own `prev_nets`.
    fresh: bool,
    /// Moves whenever a net or a part's state changes: the key by which
    /// whatever reads the board from outside --- the cables and the
    /// backplanes --- knows it has nothing new to read. See
    /// [`Chip::generation`].
    generation: u64,
    /// The net states the last few transitions left, each with its hash,
    /// as a ring of [`Chip::RECENT`] with `recent_next` the slot the next
    /// one takes; and how many transitions in a row have ended in one of
    /// them: a board whose clocks are ticking over static logic cycles
    /// among a few states. Kept only for a board with an oscillator. See
    /// [`Chip::asleep`]. The hash is compared first and the levels only
    /// against a slot whose hash matches, so the answer is exact and a
    /// transition that brings something new costs one pass over the nets
    /// rather than sixteen: `Vec<Level>` equality compiles to a loop, not
    /// a `memcmp`, and it was a quarter of a display board's transition.
    recent: Vec<(u64, Vec<Level>)>,
    recent_next: usize,
    quiet: u32,
    /// A hash of every net's level, kept up to date as the nets move
    /// rather than computed over them: `zobrist[net][level]` is a random
    /// word for each net at each level, and the hash is the exclusive-or
    /// of the words for the levels the nets stand at, so a net moving
    /// costs two exclusive-ors. What [`Chip::remember`] compares first.
    state_hash: u64,
    zobrist: Vec<[u64; 4]>,
    /// The generation at which each net's contribution to the outside
    /// last changed --- its level, or the level of any gate driving it ---
    /// so that a cable or a backplane reads back only the wires that
    /// moved since it last looked; see [`Chip::net_stamp`].
    net_stamp: Vec<u64>,
    /// Nets with a driver the evaluator does not place --- the clock
    /// generator's replaced parts --- whose level is computed wherever it
    /// is read and so has no stamp to trust.
    net_live: Vec<bool>,
    /// For each net, each of its drivers by flat gate index with how its
    /// pin drives: `drivers` again, in the form the resolution loops read.
    driver_info: Vec<Vec<(u32, Drive)>>,
    /// Every part that remembers and touches each net, by index. The
    /// equivalent of `readers` for [`Chip::update_state`].
    touchers: Vec<Vec<u32>>,
    /// Which parts need asking whether they have seen an edge.
    part_dirty: Vec<bool>,
    /// The gates in evaluation order, grouped so that every group comes
    /// after the ones feeding it. A group of one gate is evaluated once; a
    /// larger one is a feedback loop and is iterated to a fixpoint.
    order: Vec<Vec<GateId>>,
    /// How many gates are in groups larger than one.
    pub in_feedback: usize,
    /// A loop that did not settle within [`Chip::SWEEP_LIMIT`] sweeps, if
    /// any did.
    pub unsettled: Option<Vec<GateId>>,
    /// The most sweeps any feedback group needed in the last settle. One
    /// means it was already at rest; the cost of a loop is this number.
    pub sweeps: usize,
    /// The level each placed gate last computed, by flat index. Valid
    /// while the gate is not dirty: every net it reads marks it when it
    /// moves, and so does its part when its state changes, so a clean
    /// gate's inputs are what they were when this was computed.
    /// [`Chip::resolve_net`] takes the other drivers of a net from here
    /// and evaluates only the dirty one, and [`Chip::board_level`] takes
    /// them all, so a bus with `k` drivers costs one evaluation when one
    /// of them moves and not `k`, and a wire is read off a settled board
    /// without evaluating anything. A gate the evaluator does not place is
    /// evaluated wherever it is read, as before.
    gate_level: Vec<Level>,
    /// `order` flattened: every gate of every group in turn with its flat
    /// index, and where each group starts, so that settling walks one
    /// vector and looks nothing up.
    order_flat: Vec<(GateId, u32)>,
    group_start: Vec<u32>,
    /// Which group each gate is in, by flat index; `u32::MAX` for a gate
    /// the evaluator does not place.
    group_of: Vec<u32>,
    /// One bit a group: which groups hold a dirty gate. What settling
    /// visits, instead of every group in turn.
    group_dirty: Vec<u64>,
    /// The parts flagged in `part_dirty`, so that [`Chip::update_state`]
    /// asks those and not every part.
    part_dirty_list: Vec<u32>,
    /// The drivers of one net, kept between resolutions so that a net
    /// driven from outside costs no allocation.
    scratch: Vec<Driver>,
    /// A transition that was still moving nets after
    /// [`Chip::UPDATE_ROUNDS`] rounds of asking the parts, if any was:
    /// when. The rounds it would have needed were dropped, and the board
    /// is not the board it would have been. Cleared by nothing, so a run
    /// can check it at the end as it checks [`Chip::unsettled`].
    pub unconverged: Option<u64>,
    /// How many rounds the last transition took: one on a board whose
    /// registers all clock from the clock nets, more where a register is
    /// clocked from another's output.
    pub rounds: usize,
}

/// FNV-1a, for [`Chip::fingerprint`]. Nothing here is security-sensitive;
/// it only has to change when the board does.
fn fnv(h: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(h, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// SplitMix64, for the words of the state hash: one well-mixed word per
/// net and level, so that the exclusive-or of a board's worth of them
/// tells its states apart.
fn splitmix(x: u64) -> u64 {
    let z = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn level_byte(l: Level) -> u8 {
    match l {
        Level::Low => 0,
        Level::High => 1,
        Level::Z => 2,
        Level::X => 3,
    }
}

fn byte_level(b: u8) -> Result<Level, ()> {
    match b {
        0 => Ok(Level::Low),
        1 => Ok(Level::High),
        2 => Ok(Level::Z),
        3 => Ok(Level::X),
        _ => Err(()),
    }
}

/// The level a net's name fixes it at.
///
/// The supplies have no driver, so without this they resolve to high
/// impedance --- and a TTL input reads that as a one, which would put every
/// grounded pin on the board at one. `HI` followed by digits is how the
/// drawings name a pull-up to the supply; `HIGHOK` and the like are signals
/// and must not match.
pub fn supply(name: &str) -> Option<Level> {
    // The processor calls its pull-up rails `HI1`, `HI2`, `HI12`; the bus
    // interface calls its two `HI 1-14` and `HI 15-30`, after the pins they
    // reach, and `busint.wls` lists both under "RUNS WITH NO OUTPUTS" ---
    // nothing drives them because they are the supply. A quoted name keeps
    // its quotes, so they are stripped first.
    let bare = name.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')).unwrap_or(name);
    let hi = bare.strip_prefix("HI").is_some_and(|rest| {
        !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit() || b == b' ' || b == b'-')
    });
    match bare {
        "GND" => Some(Level::Low),
        "VCC" | "+5" => Some(Level::High),
        _ if hi => Some(Level::High),
        _ => None,
    }
}

/// The packages `src/clock.rs` replaces.
///
/// Not the whole of CLOCK1 and CLOCK2 --- only what is genuinely analog or
/// genuinely cyclic. The delay lines drop out on their own, having no
/// behaviour to give. What is listed is the speed multiplexer, the
/// `CYCLECOMPLETED` latch, and the three cross-coupled pairs that make
/// `TPCLK`, `TPTSE` and the two write pulses.
///
/// Everything else on those pages stays structural, which matters: `-CLK0`
/// is gated by `MACHRUN` at CLOCK2 1D10 and the write pulses by `MACHRUNA`
/// at 1C10, so the halted machine stops writing because the hardware says
/// so, not because the clock model was told to. `src/clock.rs` takes
/// `machrun` as an input and does not use it, for exactly this reason.
pub const REPLACED: &[(&str, &str)] = &[
    ("CLOCK1", "1C08"), // CYCLECOMPLETED, cross-coupled with 1C09
    ("CLOCK1", "1C09"),
    ("CLOCK1", "1C10"),
    ("CLOCK1", "1D08"), // the speed multiplexer
    ("CLOCK2", "1C06"), // TPCLK and TPTSE, cross-coupled with 1C07
    ("CLOCK2", "1C07"),
    ("CLOCK2", "1C13"), // the control-store write-pulse latch
];

/// The delay lines the clock generator does not own.
///
/// Eight of the ten on the board are the clock's, and `src/clock.rs` supplies
/// their taps directly. These two are on VCTL1 and they are what ends a
/// memory cycle: `-MFINISH` through the TD50 at 1D23 gives `-MFINISHD`, which
/// clears `MBUSY`, and on through the TD250 at 1D22 gives `-RDFINISH`, which
/// clears `READ IN PROGRESS`. Without them `-MEMACK` arrives and nothing
/// happens --- the board waits for a cycle that can never finish.
pub const DELAY_LINES: &[(&str, &str)] = &[("VCTL1", "1D23"), ("VCTL1", "1D22")];

/// Gates whose propagation delay the design relies on: page, reference,
/// output pin, input pin, nanoseconds, and whether the gate inverts.
///
/// The memory board clocks its refresh flag, the 74S74 at `memctl` 0F06,
/// from `T5`: `-T0` through one section of the 74S240 at 0F07, named for
/// the five nanoseconds it takes. `-T0` falls the moment `REFRESH RQ`
/// rises, and the flag's data is `REFRESH RQ` itself, so with no delay
/// the flop sees its clock in the instant its data changes and samples
/// the old value: the refresh cycle ran as a memory cycle to bank 0. In
/// the hardware the buffer is what gives the flop its setup time. So this
/// gate is left out of the evaluation and its output is a tap five
/// nanoseconds behind its input, inverting, like a delay line's.
pub const DELAYED_GATES: &[(&str, &str, u8, u8, u32, bool)] = &[("MEMCTL", "0F07", 9, 11, 5, true)];

/// Whether a gate is one of [`DELAYED_GATES`].
fn delayed(inst: &Instance, gate: usize) -> bool {
    let out = inst.behaviour.gates[gate].out;
    DELAYED_GATES
        .iter()
        .any(|&(p, r, o, _, _, _)| inst.page == p && inst.reference == r && o == out)
}

/// A delay line: an input, and taps at fifths of its nominal delay.
///
/// The tap order is the drawings': pins 12, 4, 10, 6, 8 carry one fifth to
/// five fifths, which is how the TD100 at CLOCKD 1D12 comes to carry `-TPR20`
/// through `-TPR100`. `src/part.rs` records the same order.
struct Delay {
    input: NetId,
    /// Tap net and how long after the input it follows it.
    taps: Vec<(NetId, u32)>,
    /// The taps carry the input's complement: a delayed inverting gate.
    invert: bool,
    last: Option<Level>,
    /// Transitions on their way down the line: when, which tap, to what.
    /// Never more than a handful; a line is 250 ns long at most.
    pending: Vec<(u64, usize, Level)>,
}

/// An oscillator on the board: a section of a 74LS124, or a DIP can.
///
/// **The 74LS124 at REQTIM 0A01, whose first VCO section clocks the
/// timeout counter.** `INT BUSY` rises when a bus cycle is granted and is
/// inverted at the 74S04 0B03 onto pin 6, the section's enable, which is
/// active low --- a low there is only worth an inverter if a low is what
/// runs it. Pin 7, the output, goes to pin 11 of the 74LS273 at REQTIM 0B01
/// with nothing in between, and that is the register's clock; the
/// register's clear, pin 1, is `INT BUSY` through the 74S08 at 0B16, so it
/// is held at zero until the grant and counts from the output's first
/// rising edge after.
///
/// What the register holds is a state machine, not a counter. `TIMEOUT<3:0>`
/// address the 74S288 at 0A02, MIT's `cadr1/reqtim.prom`, whose data carries
/// the *next* state in its low four bits and the timeout flags above them.
/// The flags are registered by the same 74LS273 --- `PROM HUNG TIMEOUT` on
/// pin 4 comes back out as `HUNG TIMEOUT` on pin 5, and `NXM TIMEOUT` on pin
/// 6 the same way --- so a flag the PROM raises for a state appears at the
/// board's output one clock after that state is entered, and the machine
/// walks one state per rising edge for as long as the cycle is unanswered.
/// State 5 is where the first table raises `PROM NXM TIMEOUT`, so `NXM
/// TIMEOUT` is up on the sixth edge; state 10 raises `PROM HUNG TIMEOUT`,
/// up on the eleventh; and in the half the `DEBUG REQUEST ACTIVE` address
/// bit selects, state 13 raises the NXM flag, up on the fourteenth.
///
/// **What the part does, from its own sheet.** The 'LS124 is on pages
/// 7-123 to 7-125 of Texas Instruments' *The TTL Data Book for Design
/// Engineers*, 2nd edition, 1976. TI deleted the part in 1981, so no later
/// sheet exists, and the SN74S124 sheet that is easy to find describes the
/// Schottky part, which this one says differs on the point that matters
/// here. Three things it says:
///
/// - "The internal oscillator of the 'LS124 runs continuously even while
///   the output is disabled, whereas the internal oscillator of the 'S124
///   is itself started and stopped by the enable input."
/// - "While the enable input is low, the output is enabled. While the
///   enable input is high, the output is high."
/// - The one delay it specifies from the enable is `tPHL`, "high-to-low-
///   level output from enable", which "will typically be 30 ns plus up to
///   one period of one cycle ... depending upon the timing of the enable
///   pulse with respect to the signal generated by the internal
///   oscillator"; and "the pulse synchronization-gating section ensures
///   that the first output pulse is neither clipped nor extended".
///
/// So an enabled section's output is not a clock started at the grant: it
/// is a window opened onto a clock that has run since power-on. The output
/// idles high; when the enable drops it stays high until the internal
/// oscillator's next falling edge, follows the oscillator from there, and
/// goes back high when the enable rises. Where the counter's edges fall
/// against the grant depends on the phase of a clock the board has no say
/// in, and the wait for the sixth edge is anywhere between five and a half
/// periods and six and a half. `tests/busint_netlist.rs` measures that
/// band on the board at five phases.
///
/// **What is modelled, and what is chosen.** The internal oscillator toggles
/// every half period from `t = 0`, high first, at [`toggle_at`]. That phase
/// is a choice --- on the machine it is whatever the part did when the
/// power came up --- and it is fixed against absolute time rather than the
/// board's power-on so that `rtl`, which has no board, reckons the same
/// edges through [`gated_rise`]: a far end built at a checkpoint powers its
/// boards on at the join's time, and this section still has the phase it
/// would have had from the button. [`gated_fall`] is where the output first
/// falls after an enable, and an enable that lands exactly on a falling edge
/// of the internal oscillator misses it and waits for the next: the sheet's
/// 30 ns says the part cannot pass an edge it has not yet seen. The 30 ns
/// itself is left out, as every gate delay in `part` is. The section wakes
/// the board at each internal toggle while enabled and not at all while
/// disabled, when its output is constant.
///
/// A DIP can, and a 74LS124 section with its enable grounded --- the disk
/// controller's `-2USEC.CLK^` --- run from the board's first transition,
/// high first, and toggle every half period; for the grounded enable the
/// two rules give the same output.
///
/// **This is the board of December 1980, with the LS part; a later one has
/// an S.** `cadr1/busint.eco` item 5, of 21 February 1981, replaces this
/// part with a 74S124 and its capacitor with 1000 pF --- [`VCO_PERIOD`] has
/// the whole of it --- and the S part's oscillator starts at the enable,
/// its first fall coming 1.4 periods after by the same sheet. The netlist is
/// built from the drawings of 10 December 1980 and the wire list of 11
/// December 1980, and MIT's parts list printed on 25 March 1981,
/// `cadr1/busint.prt`, still names a 74LS124 at A01: the drawings were not
/// brought up to the change, and it is the drawings' board that is built.
struct Oscillator {
    output: NetId,
    /// Low to run; `None` for one that always runs.
    enable: Option<NetId>,
    /// The period in nanoseconds, as a fraction, so that a crystal whose
    /// period is not a whole number of nanoseconds keeps time over a
    /// second of it.
    period: (u64, u64),
    /// When the internal oscillator started, high: the `k`-th toggle is at
    /// `origin + toggle_at(period, k)`. `Some(0)` for a 74LS124 section,
    /// which runs from power-on whether enabled or not; a DIP can's is the
    /// board's first transition, and `None` before it.
    origin: Option<u64>,
    /// How many times it had toggled at the last transition: a cache of
    /// what `origin` and the time give, so that catching up is one step.
    edges: u64,
    /// Since when the enable has been low, for a section that has one: the
    /// output is high until the internal oscillator's first fall after
    /// this, [`gated_fall`], and follows it from there.
    enabled_at: Option<u64>,
    /// When the output next needs the board's attention, while it can move.
    next: Option<u64>,
}

impl Oscillator {
    /// What a checkpoint keeps of the phase: a DIP can's origin once it has
    /// started, a gated section's enable instant while it is enabled.
    fn phase(&self) -> Option<u64> {
        if self.enable.is_some() { self.enabled_at } else { self.origin }
    }
}

/// When an oscillator of `period` makes its `k`-th toggle after starting:
/// the output rises as it starts, and toggles every half-period from
/// there, to the nanosecond below.
pub fn toggle_at(period: (u64, u64), k: u64) -> u64 {
    k * period.0 / (2 * period.1)
}

/// The index of the internal oscillator's first falling edge strictly
/// after `t`: the odd toggles are the falls, the oscillator being high at
/// `t = 0`.
fn first_fall(period: (u64, u64), t: u64) -> u64 {
    let mut k = t * 2 * period.1 / period.0;
    while toggle_at(period, k) <= t {
        k += 1;
    }
    while k > 0 && toggle_at(period, k - 1) > t {
        k -= 1;
    }
    if k.is_multiple_of(2) { k + 1 } else { k }
}

/// Where a 74LS124 section's output first falls after its enable drops at
/// `enabled_at`: the internal oscillator's first falling edge strictly
/// after that instant, the oscillator toggling at [`toggle_at`] from
/// `t = 0`, high first. The output is high until then; the note on
/// `Oscillator` has the sheet.
pub fn gated_fall(period: (u64, u64), enabled_at: u64) -> u64 {
    toggle_at(period, first_fall(period, enabled_at))
}

/// The `k`-th rising edge, `k` from 1, of a 74LS124 section's output after
/// its enable drops at `enabled_at`: the output follows the internal
/// oscillator from [`gated_fall`], so its rises are the even toggles after
/// that.
pub fn gated_rise(period: (u64, u64), enabled_at: u64, k: u64) -> u64 {
    toggle_at(period, first_fall(period, enabled_at) + 2 * k - 1)
}

/// The first rising edge of the output strictly after `t`, the enable
/// having dropped at `enabled_at`, no later than `t`.
pub fn gated_rise_after(period: (u64, u64), enabled_at: u64, t: u64) -> u64 {
    let first = first_fall(period, enabled_at) + 1;
    let mut k = (t * 2 * period.1 / period.0).max(first);
    while toggle_at(period, k) <= t {
        k += 1;
    }
    while k > first && toggle_at(period, k - 1) > t {
        k -= 1;
    }
    if !k.is_multiple_of(2) {
        k += 1;
    }
    toggle_at(period, k)
}

/// The bus interface's request-timing oscillator, `reqtim` 0A01, section 1:
/// 850 ns.
///
/// **From the part's own sheet and the board's own capacitor, not from
/// MIT's prose.** Texas Instruments' *The TTL Data Book for Design
/// Engineers*, 2nd edition, 1976, page 7-123, gives the 'LS124 its own
/// constant, distinct from the 'S124's:
///
/// ```text
///     fo = 1 x 10^-4 / Cext   for 'LS124      fo = 5 x 10^-4 / Cext   for 'S124
/// ```
///
/// The capacitor is `cadr1/reqtim.drw`'s `DCAP` of `100 pf`, in the
/// `DUMMY4` at B03@01 that `cadr1/busint.wlr` hangs across `VCO CAP1` and
/// `VCO CAP2`, pins 4 and 5 of the part; the same wire list straps the
/// range input on pin 3 and the frequency-control input on pin 2 to
/// `+5.0V`, which the drawing does not show because +5 is not a page
/// signal. 1e-4 over 1e-10 is 1 MHz, a 1,000 ns period at the formula's
/// conditions.
///
/// **The figure is a band, 0.7 to 1.0 us, and 850 sits near its centre
/// with one correction applied and another noted.** The formula holds
/// "under the conditions used in Figure 3", both control inputs at 2 V, and
/// this board straps both to +5, so 1,000 measures a bias point the board
/// does not have. Figure 4, the frequency normalised against the two
/// control voltages, is drawn for the S part and only to 4.5 V on the range
/// input; every curve on it converges near 1.17 at 5 V on the frequency
/// input, and the sheet says of the LS only that "the concept also
/// applies". 1,000 over 1.17 is 855, rounded to 850, the precision a band
/// of about fifteen per cent supports; that is the correction applied. The
/// other is a calibration of the formula itself: on the disk
/// controller, where all four control pins sit at the formula's own 2 V,
/// it gives 2.2 us for the 220 pF section MIT drew as `PERIOD = 1.8 - 2.0
/// usec.` --- [`DISK_2USEC_VCO_PERIOD`] --- 10 to 22 per cent slow. That
/// points the same way, and being a single point from another board it is
/// noted rather than multiplied in: the two stacked would say about 750,
/// the band's fast end, and two soft corrections multiplied is precision
/// the sheet does not give. All three lines of evidence put the true period
/// below 1,000 and none above it. The cadr4 project reached the same
/// figure by an independent reading of the same page, one project through
/// Figure 4 and the other partly through the disk controller's
/// calibration, which is the agreement by separate routes this project's
/// rules ask for; the two clocks matching also makes the comparison between
/// the projects easier where they disagree elsewhere. Nothing in the
/// simulation turns on where in the band the board sits: the `rtl` boot's
/// nanosecond count to the first disk read is the same at 850 as at 855,
/// its two NXM cycles ending in the same microcycles either way, and it
/// moved by 7.7 microseconds of 118 milliseconds when the period came
/// down from 2,000.
///
/// **MIT says "roughly 2 uSec", and the board MIT measured is not this
/// one.** `cadr1/reqtim.prom`'s header says "This Assumes Roughly 2 uSec
/// clock intervals" and labels its states 2, 4, 6 ... microseconds apart.
/// `cadr1/busint.eco` item 5, of 21 February 1981, "To make the clock less
/// marginal": "Replace the 74LS124 at A-1 with a 74S124 and replace the
/// capacitor in the 2-dummy at B-3@2 with a 1000pf (the intrinsic clock
/// rates of the two chips are different)." Through the S part's constant,
/// 1000 pF is 500 kHz, 2,000 ns on the nose. So the PROM's labels describe
/// the board after that change, or the clock its designer meant to have,
/// and the netlist is the board before it: `data/BUSINT.netlist` comes from
/// the drawings of 10 December 1980 reconciled to the wire list of 11
/// December 1980, and MIT's parts list printed on 25 March 1981,
/// `cadr1/busint.prt`, still names a 74LS124 at A01 and no two-pin dummy at
/// B03@02 --- its dummy at B03 is the four-pin one --- so the drawings were
/// not brought up to the change. One reading of the ECO's heading, offered
/// as a reading: a clock at half its assumed interval fires every timeout
/// at half its label, which is what "marginal" would look like in service,
/// and the change would then be the hardware being corrected to the PROM
/// rather than the PROM written for new hardware. On this board the PROM's
/// 10 microsecond NXM timeout is between 5.5 and 6.5 periods, 4.7 to 5.5
/// us; its 20 microsecond hung timeout 8.9 to 9.8; and its 26 microsecond
/// debug timeout 11.5 to 12.3 --- the note on `Oscillator` has why each is
/// a range ---
/// which the microcode never notices, because nothing it does is timed
/// against them. Discrepancy 73.
///
/// **Unverified** to a point: what would settle the period within the band
/// is a scope on a board built to the December 1980 list, or an LS sheet
/// with its own normalised curve. The labels need nothing: they are right
/// for the other board.
pub const VCO_PERIOD: (u64, u64) = (850, 1);

/// The disk controller's other VCO, `dctmot` 0B04 section 2, whose output
/// is the net MIT calls `-2USEC.CLK^`: 2,000 ns.
///
/// The drawing's own property on that body is `PERIOD = 1.8 - 2.0 usec.`,
/// a range rather than a figure, and the top of it is taken because MIT
/// named the net for it. `tests/cadrdc_netlist.rs` walks the sequencer on
/// this clock and is written to that number.
///
/// **Separate from the bus interface's `VCO_PERIOD` although both were
/// once one constant.**
/// They are different circuits: this one has 220 pF and both control pins
/// on a 2 V divider, MIT annotates it directly, and the bus interface's has
/// 100 pF with both control pins at +5 and no annotation of its own. Held
/// together, a correction to either moved the other, which is why an
/// earlier attempt to change the bus interface's broke this board's tests.
pub const DISK_2USEC_VCO_PERIOD: (u64, u64) = (2_000, 1);

/// The period of the memory board's DIP oscillator, `memctl` 0E08, which
/// clocks the two 74S374 shift chains the board's timing is made of: a
/// 24 MHz crystal, 125/3 ns, by MIT's own ECO record for the board,
/// `cadrm/mem.eco` ("Add DIPS: 24 MHz crystal E08", ECO 1 of 10 June
/// 1979). The chain's taps are named `-T0`, `-T40`, `-T80` ... `-T640` as
/// if a stage were 40 ns; a stage is 41 2/3.
pub const DIP_OSCILLATOR_PERIOD: (u64, u64) = (125, 3);

/// The period of the I/O board's DIP oscillator, `iobser` 0A15: 5.0688
/// MHz, the serial port's baud-rate crystal, which the drawing names on
/// its output net, 78125/396 ns.
pub const IOB_OSCILLATOR_PERIOD: (u64, u64) = (78_125, 396);

/// The Chaosnet transmit clock's crystal, LMTCLK 0A05: `32 MHZ`, as
/// `lmtclk.drw` writes beside it. Two things are made from it. The
/// second section of the 74S112 at 0A06 halves it to `MCLK^`, 16 MHz,
/// which the I/O pages' select flop and microsecond chain run on: the
/// 74S163 at CLKTIM 0C21 counts it, and its third bit is the `1/2 USEC
/// CLK` and its carry the `1 USEC CLK` the drawing names, which is the
/// check on the figure. And the 74S163 at 0B03 divides it by four to
/// `FCLK^`, 8 MHz, reloading itself from its own carry with the load
/// value the speed jumpers set --- [`crate::unibus::chaosnet_speed_jumpers`]
/// --- which the first section of the 74S112 halves again to `FCLK/2^`,
/// the 4 MHz bit rate of AIM-628 §2.5. `cadrio/iob.eco`'s "8 MHz
/// Operation" is `FCLK^`; its "cable rate is 4 MHz" is `FCLK/2^`.
pub const CHAOS_CRYSTAL_PERIOD: (u64, u64) = (125, 4);

/// The display boards' dot clock, the TTL oscillator can at C08 on both ---
/// the SIMPLE TV's `necclk` 0C08 and the LISPM TV's `eclclk` 0C08: 64 MHz,
/// 125/8 ns. The SIMPLE TV's drawing says so itself, the body carrying the
/// SUDS property `VALUE = MOTOROLA K114A 64 MHZ TTL`, and `lmtv4b.prt`, the
/// LISPM TV's own stuffing list, names the same can at its C08. It is also
/// what `lmtv.order` means by "CPT 64 MHz" for clock mode 0, and the net it
/// drives through the 10124 is called `-64 MHZ CLK` on both boards.
///
/// **MIT's parts lists say the can they bought was 63.5 MHz**, a Motorola
/// K1114A, in five files under `cadrpt`. This constant is the design number
/// rather than the stocked one, and the choice costs nothing: the raster is
/// a pure division of this clock --- 1024 dots a line, 966 lines a frame ---
/// so either figure gives the same 768 by 896 picture and only scales the
/// whole of it by 0.78%. The CPT locks to the sync the board hands it and
/// there is no broadcast standard for the timing to miss, which is why no
/// simulation can tell the two apart. 64 MHz is kept because it is what
/// makes the design come out round, `lmtv.order`'s "roughly every 1/2
/// microsecond" being exactly half a microsecond at 64 and 0.504 at 63.5.
/// Discrepancy 44 has both sides; if the physically stocked
/// part is ever what is wanted, it is one edit here.
pub const TV_OSCILLATOR_PERIOD: (u64, u64) = (125, 8);

/// The period of one section of a 74LS124, which is set by a capacitor the
/// drawing carries and so differs per instance, as a DIP can's does.
///
/// The bus interface's `reqtim` 0A01 uses section 1 alone, at
/// [`VCO_PERIOD`] from its own capacitor through the part's sheet, where
/// MIT's PROM listing says "roughly 2 uSec" of a later board. The disk
/// controller's `dctmot` 0B04 uses both,
/// drawn as two bodies at one location, and each body carries its period
/// as a SUDS property: the one with section 2 --- output pin 10, the net
/// `-2USEC.CLK^` --- says `PERIOD = 1.8 - 2.0 usec.`, and the one with
/// section 1 --- output pin 7, `TIMEOUT.CLK` --- says `;Period = 12 ms`,
/// which the 74393s of the timeout counter then divide. MIT's own stuffing
/// list `dc.prt` prints that comment as `PERIOD = 12 MEGS`, which is
/// wrong by six orders of magnitude; the drawing is the authority.
fn vco_period(page: &str, reference: &str, section: u8) -> (u64, u64) {
    match (page, reference, section) {
        ("REQTIM", "0A01", 1) => VCO_PERIOD,
        ("DCTMOT", "0B04", 1) => DISK_TIMEOUT_VCO_PERIOD,
        ("DCTMOT", "0B04", 2) => DISK_2USEC_VCO_PERIOD,
        _ => panic!("no frequency known for VCO {section} of the 74LS124 at {page} {reference}"),
    }
}

/// The disk controller's timeout clock, `dctmot` 0B04 section 1: 20 ms.
///
/// **Not the drawing's `;Period = 12 ms`, and the difference is a second
/// capacitor.** The SN74LS124's own data sheet --- TI's *TTL Data Book for
/// Design Engineers*, 2nd edition of 1976, pages 7-123 to 7-128, a part TI
/// deleted in 1981 and replaced with the 'LS629, which is why no separate
/// sheet for it survives --- gives the LS its own constant, distinct from
/// the S part's:
///
/// ```text
///     fo = 1 x 10^-4 / Cext   for 'LS124      fo = 5 x 10^-4 / Cext   for 'S124
/// ```
///
/// The board as wrapped has **2 uF** on this section, not 1: `dc.wlr` puts
/// C04 pins 1 *and* 2 on `VCO.C1` and 15 *and* 16 on `VCO.C2`, joined BARE,
/// and MIT's parts list gives 1 uF on both those body positions. Two
/// capacitors in parallel. 1e-4 / 2e-6 is 50 Hz, a 20 ms period.
///
/// The drawing's 12 ms corresponds to 1.2 uF, which is one capacitor and a
/// little stray: the property was written for the section before the second
/// body went on, and its `|TIMEOUT ;1.5 SEC` note is that figure counted
/// down. With the board as built, 20 ms through the 74393's divide by 128
/// is **2.56 s**, and `sys/doc/disk.text` says the timeout error means "a
/// disk operation took longer than 2.5 seconds". MIT's prose was right and
/// the drawing's two notes are stale.
///
/// This reverses an earlier reading which took the drawing over the text on
/// the rule that a drawing outranks documentation. The rule holds; what
/// changed is that the part's own data sheet outranks both, and it agrees
/// with the text.
pub const DISK_TIMEOUT_VCO_PERIOD: (u64, u64) = (20_000_000, 1);

fn dip_oscillator_period(page: &str, reference: &str) -> (u64, u64) {
    match (page, reference) {
        ("MEMCTL", "0E08") => DIP_OSCILLATOR_PERIOD,
        ("IOBSER", "0A15") => IOB_OSCILLATOR_PERIOD,
        ("LMTCLK", "0A05") => CHAOS_CRYSTAL_PERIOD,
        ("NECCLK", "0C08") | ("ECLCLK", "0C08") => TV_OSCILLATOR_PERIOD,
        _ => panic!("no frequency known for the oscillator at {page} {reference}"),
    }
}

/// A retriggerable one-shot on the board: the Am26S02 at `memctl` 0F02 that
/// times the memory board's refresh.
///
/// From the Am26S02 sheet (`am26s02.pdf`): the clear on pin
/// 3 resets it while low; with `I1` (pin 4) low, a high-to-low edge on
/// `I0` (pin 5) triggers it, and with `I0` high a low-to-high edge on `I1`
/// does; `Q` (pin 6) is high for the pulse and `-Q` (pin 7) low. The
/// drawing holds the clear high, grounds `I1` and drives `I0` from
/// `-REFRESH NOW`, so the pulse starts when a refresh cycle begins, and
/// pin 7, `TIME FOR REFRESH`, is high before the first pulse and after
/// each one ends, which is when the synchroniser at 0F01 requests a
/// refresh cycle. The pulse width is the drawing's timing components at
/// 0F02 through the sheet's formula: [`MEMCTL_REFRESH_NS`].
struct OneShot {
    i0: NetId,
    i1: NetId,
    clear: NetId,
    q: NetId,
    not_q: NetId,
    width_ns: u64,
    /// When `Q` falls, while the pulse runs.
    fall: Option<u64>,
    /// `I0` and `I1` at the last transition, for their edges.
    last: Option<(Level, Level)>,
}

/// The refresh one-shot's pulse. `memctl.drw` gives its timing components
/// as `RES1 40 K` and `CAP1 1000 pF` and notes `12 US` beside the part;
/// the Am26S02 sheet's t_pw = 0.30 Cx Rx (1 + 0.11/Rx), Cx in pF and Rx
/// in kilohms, makes that 12,033 ns. The sheet gives that formula for Cx
/// above 1000 pF and a graph at or below, so the last two digits are the
/// formula's; MIT's own figure is the 12 µs. A 4116 row every 12 µs
/// refreshes all 128 in 1.5 ms of the 2 the part allows.
pub const MEMCTL_REFRESH_NS: u64 = 12_033;

/// The disk controller's NXM-acknowledge one-shot, `dctmot` 0B09 section 2.
///
/// `dctmot.drw` carries `50 K` and `1000 pF` under MIT's own note `;15 uS`,
/// and MIT's parts list gives the same two values independently for that
/// body. Through the Am26S02 sheet's t_pw = 0.30 Cx Rx (1 + 0.11/Rx), Cx in
/// picofarads and Rx in kilohms, that is 15,033 ns. 1000 pF is the boundary
/// of the range the sheet gives the formula for, so this one can be taken
/// at the figure; MIT's own note is the 15 us.
pub const DCTMOT_NXM_ACK_NS: u64 = 15_033;

/// The disk controller's block-counter clear, `dctrid` 0B09 section 1.
///
/// `dctrid.drw` carries `20K` and `330 pF` under MIT's own note
/// `;2.0-2.5 USEC`, and MIT's parts list gives the same two values
/// independently. The same formula makes that 1,991 ns.
///
/// **330 pF is below the range the sheet gives that formula for** --- it
/// gives a graph at and below 1000 pF and the formula above it --- so the
/// exact figure is an extrapolation and MIT's own range is the real check.
/// It was 2,250 until 7 Sep 2026, the middle of MIT's range chosen for want
/// of the components; the components are now known.
pub const DCTRID_BLOCK_CLEAR_NS: u64 = 1_991;

/// Where a clock output joins the board: by net name where the drawing gives
/// one, and by the pin it leaves on where it does not.
#[derive(Debug)]
pub enum At {
    Net(&'static str),
    Pin(&'static str, &'static str, u8),
}

/// Which clock signal a net carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClockOut {
    /// `TPCLK`, the read phase, at CLOCK2 1C07 pin 3.
    Tpclk,
    /// Its complement, at 1C06 pin 12, which 1D10 gates with `MACHRUN`.
    NotTpclk,
    /// `TPTSE`, the tri-state enable, at 1C06 pin 6.
    Tptse,
    /// The write-pulse latch at 1C06 pin 8, active low into 1C10.
    NotTpwp,
    /// The control-store write latch at 1C13 pin 8, active low into 1C10.
    NotTpwpiram,
    /// `-TPR60`, a read tap OLORD1 turns into `SPEEDCLK`. Active low, and
    /// sixty nanoseconds behind the forty-nanosecond `-TPR0` pulse.
    NotTpr60,
}

const CLOCK_OUTPUTS: &[(At, ClockOut)] = &[
    (At::Net("TPCLK"), ClockOut::Tpclk),
    (At::Net("-TPCLK"), ClockOut::NotTpclk),
    (At::Net("TPTSE"), ClockOut::Tptse),
    (At::Net("-TPR60"), ClockOut::NotTpr60),
    // The two write-pulse latches drive nets the drawing never named.
    (At::Pin("CLOCK2", "1C06", 8), ClockOut::NotTpwp),
    (At::Pin("CLOCK2", "1C13", 8), ClockOut::NotTpwpiram),
];

/// The nets the clock generator reads back off the rest of the board.
struct ClockIn {
    machrun: NetId,
    hang: NetId,
    ilong: NetId,
    sspeed0: NetId,
    sspeed1: NetId,
    reset: NetId,
}

impl Chip {
    /// Builds the board. Parts whose type has no behaviour --- the delay
    /// lines, the oscillators and the one-shots --- are not instances:
    /// `build` skips them, and `wire_delays`, `wire_oscillators` and
    /// `wire_one_shots` run them by time from the netlist instead.
    pub fn new(n: &Netlist) -> Chip {
        Chip::build(n, true)
    }

    /// Builds a board that has no clock generator of its own.
    ///
    /// [`Chip::new`] expects the processor's: `TPCLK` and its fellows out,
    /// `MACHRUN`, `-HANG`, `-ILONG`, the speed bits and `RESET` back. The
    /// bus interface has none of them --- "the master clock ... is supplied
    /// by the cpu to the bus interface" --- so it is built without, and
    /// [`Chip::tick`] cannot be called on it. Everything else is the same:
    /// the same parts, the same levelizer, the same settling.
    pub fn new_unclocked(n: &Netlist) -> Chip {
        Chip::build(n, false)
    }

    fn build(n: &Netlist, clocked: bool) -> Chip {
        let mut instances = Vec::new();
        for pkg in n.packages() {
            let Some(behaviour) = part::behaviour(&pkg.kind) else { continue };
            let mut net_of = [None; crate::part::MAX_PINS];
            for &(pin, net) in &pkg.pins {
                if (pin as usize) < net_of.len() {
                    net_of[pin as usize] = Some(net);
                }
            }
            let mut drive_of = [Drive::Passive; crate::part::MAX_PINS];
            for (pin, drive) in drive_of.iter_mut().enumerate() {
                *drive = part::pinout(&pkg.kind)
                    .and_then(|po| po.drive_of(pin as u8))
                    .unwrap_or(Drive::Passive);
            }
            instances.push(Instance {
                page: pkg.page,
                reference: pkg.reference,
                kind: pkg.kind,
                behaviour,
                net_of,
                drive_of,
                state: State::default(),
            });
        }

        let mut drivers: Vec<Vec<GateId>> = vec![Vec::new(); n.nets.len()];
        for (i, inst) in instances.iter().enumerate() {
            for (g, gate) in inst.behaviour.gates.iter().enumerate() {
                if delayed(inst, g) {
                    continue;
                }
                if let Some(net) = inst.net_of[gate.out as usize] {
                    drivers[net as usize].push(GateId { part: i as u32, gate: g as u16 });
                }
            }
        }

        let mut gate_base = Vec::with_capacity(instances.len());
        let mut gates_total = 0u32;
        for inst in &instances {
            gate_base.push(gates_total);
            gates_total += inst.behaviour.gates.len() as u32;
        }
        let mut readers: Vec<Vec<u32>> = vec![Vec::new(); n.nets.len()];
        let mut touchers: Vec<Vec<u32>> = vec![Vec::new(); n.nets.len()];
        for (i, inst) in instances.iter().enumerate() {
            for (g, gate) in inst.behaviour.gates.iter().enumerate() {
                for &pin in gate.ins {
                    if let Some(net) = inst.net_of[pin as usize] {
                        readers[net as usize].push(gate_base[i] + g as u32);
                    }
                }
            }
            // Every pin, not only the ones the update names: a part that
            // remembers may read any of them, and marking too often is safe
            // where marking too seldom is not.
            if inst.behaviour.update.is_some() {
                for net in inst.net_of.iter().flatten() {
                    touchers[*net as usize].push(i as u32);
                }
            }
        }
        for r in readers.iter_mut().chain(touchers.iter_mut()) {
            r.sort_unstable();
            r.dedup();
        }

        let supplies: Vec<Level> =
            (0..n.nets.len()).map(|i| supply(n.net(i as u32)).unwrap_or(Level::Z)).collect();
        let mut clock_nets = Vec::new();
        for (at, which) in if clocked { CLOCK_OUTPUTS } else { &[][..] } {
            let found = match *at {
                At::Net(name) => n.by_name_id(name),
                At::Pin(page, reference, pin) => instances
                    .iter()
                    .find(|i| i.page == page && i.reference == reference)
                    .and_then(|i| i.net_of[pin as usize]),
            };
            match found {
                Some(net) => clock_nets.push((net, *which)),
                None => panic!("the clock output at {at:?} is not in the netlist"),
            }
        }
        let net = |name: &str| n.by_name_id(name).unwrap_or_else(|| panic!("no net {name}"));
        let clock_in = clocked.then(|| ClockIn {
            machrun: net("MACHRUN"),
            hang: net("-HANG"),
            ilong: net("-ILONG"),
            sspeed0: net("SSPEED0"),
            sspeed1: net("SSPEED1"),
            reset: net("RESET"),
        });
        let mut chip = Chip {
            fixed: fixed_proms(&instances, n),
            fill: power_on_fill(&instances, n),
            clock_nets,
            clock_in,
            nets: vec![Level::X; n.nets.len()],
            supplies,
            drivers,
            readers,
            gate_base,
            dirty: vec![true; gates_total as usize],
            dirty_count: 0,
            evaluated: vec![false; gates_total as usize],
            delays: Vec::new(),
            one_shots: Vec::new(),
            oscillators: Vec::new(),
            external: vec![None; n.nets.len()],
            prev_nets: vec![Level::Z; n.nets.len()],
            fresh: true,
            generation: 0,
            recent: Vec::new(),
            recent_next: 0,
            quiet: 0,
            state_hash: 0,
            zobrist: (0..n.nets.len() as u64)
                .map(|net| [0, 1, 2, 3].map(|l| splitmix(net * 4 + l + 1)))
                .collect(),
            net_stamp: vec![0; n.nets.len()],
            net_live: vec![false; n.nets.len()],
            driver_info: Vec::new(),
            touchers,
            part_dirty: vec![true; instances.len()],
            part_dirty_list: (0..instances.len() as u32).collect(),
            order: Vec::new(),
            in_feedback: 0,
            unsettled: None,
            sweeps: 0,
            gate_level: vec![Level::Z; gates_total as usize],
            order_flat: Vec::new(),
            group_start: Vec::new(),
            group_of: vec![u32::MAX; gates_total as usize],
            group_dirty: Vec::new(),
            scratch: Vec::with_capacity(16),
            unconverged: None,
            rounds: 0,
            instances,
        };
        chip.levelize();
        for (k, group) in chip.order.iter().enumerate() {
            chip.group_start.push(chip.order_flat.len() as u32);
            for &id in group {
                let f = chip.gate_base[id.part as usize] as usize + id.gate as usize;
                chip.evaluated[f] = true;
                chip.group_of[f] = k as u32;
                chip.order_flat.push((id, f as u32));
            }
        }
        chip.group_start.push(chip.order_flat.len() as u32);
        chip.group_dirty = vec![0; chip.order.len().div_ceil(64)];
        for net in 0..chip.drivers.len() {
            let info: Vec<(u32, Drive)> = chip.drivers[net]
                .iter()
                .map(|&id| (chip.flat(id) as u32, chip.drive_of(id)))
                .collect();
            chip.net_live[net] = info.iter().any(|&(f, _)| !chip.evaluated[f as usize]);
            chip.driver_info.push(info);
        }
        chip.mark_every_gate();
        chip.nets_replaced();
        chip.delays = wire_delays(n, clocked);
        chip.oscillators = wire_oscillators(n, clocked);
        chip.one_shots = wire_one_shots(n, clocked);
        chip
    }

    /// How many sweeps a feedback group gets before we call it stuck.
    pub const SWEEP_LIMIT: usize = 64;

    /// Groups the gates so that every group comes after the ones feeding it.
    ///
    /// Tarjan's algorithm over gates, not packages. Most groups come out with
    /// one gate in them and are evaluated once. The rest are genuine
    /// combinational feedback --- the trap and NOP interlock, and the SPY
    /// bus, which is read and written on the same wires --- and those are
    /// iterated instead. Taking dependencies package-wide rather than pin by
    /// pin would put nearly the whole board in one loop.
    fn levelize(&mut self) {
        let skip = |inst: &Instance| {
            REPLACED.iter().any(|&(page, r)| inst.page == page && inst.reference == r)
        };
        let gates: Vec<GateId> = self
            .instances
            .iter()
            .enumerate()
            .filter(|(_, inst)| !skip(inst))
            .flat_map(|(i, inst)| {
                (0..inst.behaviour.gates.len())
                    .filter(move |&g| !delayed(inst, g))
                    .map(move |g| GateId { part: i as u32, gate: g as u16 })
            })
            .collect();

        let mut at: HashMap<(u32, u16), usize> = HashMap::new();
        for (k, id) in gates.iter().enumerate() {
            at.insert((id.part, id.gate), k);
        }
        // feeds[k] are the gates that read what gate k drives.
        let mut feeds: Vec<Vec<usize>> = vec![Vec::new(); gates.len()];
        for (k, id) in gates.iter().enumerate() {
            let inst = &self.instances[id.part as usize];
            for &pin in inst.behaviour.gates[id.gate as usize].ins {
                let Some(net) = inst.net_of[pin as usize] else { continue };
                for d in &self.drivers[net as usize] {
                    if let Some(&j) = at.get(&(d.part, d.gate)) {
                        feeds[j].push(k);
                    }
                }
            }
        }

        for group in tarjan(&feeds).into_iter().rev() {
            if group.len() > 1 {
                self.in_feedback += group.len();
            }
            let group = order_within(group, &feeds);
            self.order.push(group.into_iter().map(|k| gates[k]).collect());
        }
    }

    /// Zeroes every register and sizes every memory array.
    ///
    /// The CADR's own reset clears only the registers the drawings wire it
    /// to; every other flip-flop and every RAM comes up in a state no
    /// datasheet defines, so a model has to start somewhere. Everything
    /// starts at zero, and the PROMs are then loaded over the top.
    pub fn power_on(&mut self) {
        self.generation += 1;
        for inst in &mut self.instances {
            inst.state.bits = 0;
            inst.state.cells = match part::memory_words(&inst.kind) {
                Some(n) => vec![0; n],
                None => Vec::new(),
            };
        }
        // Every memory comes up reading zero, which for the two map levels
        // means filling their cells with ones; see [`power_on_fill`].
        for k in 0..self.fill.len() {
            let (i, byte) = self.fill[k];
            self.instances[i].state.cells.fill(byte);
        }
        // The mask PROMs are wiring, not program: they come back with the
        // power. Left empty they read as all zeros, the field mask is zero,
        // and every byte instruction returns its A operand unchanged --- a
        // machine that runs and is quietly wrong.
        for k in 0..self.fixed.len() {
            let i = self.fixed[k].0;
            self.instances[i].state.cells.clone_from(&self.fixed[k].1);
        }
        for (id, level) in self.nets.iter_mut().enumerate() {
            *level = self.supplies[id];
        }
        self.nets_replaced();
        self.prev_nets.fill(Level::Z);
        self.fresh = true;
        self.mark_all();
        // The clock's outputs are all low out of reset, and they have to be
        // on the board before anything is evaluated. Left undriven, `TPTSE`
        // reads high --- an undriven TTL input does --- and the 7428s at
        // CLOCK2 1D04 turn that into `-TSE1..4` *asserted*, which enables
        // every source on the M bus at once. Four tri-state drivers then
        // fight over `M24` and the bus resolves to unknown. The machine has
        // no defined state until the clock is holding the enables off.
        self.apply_clock(clock::Outputs::default(), 0);
    }

    /// Loads a PROM that is part of the machine --- the bus interface's
    /// REQTIM 0A02 and UPRIOR 0D09 --- with its contents by address, and
    /// keeps them across [`Chip::power_on`] as the mask PROMs are kept.
    /// The part is named by reference designator and kind.
    pub fn load_rom(&mut self, reference: &str, kind: &str, contents: &[u8]) {
        let i = self
            .instances
            .iter()
            .position(|inst| inst.reference == reference && inst.kind == kind)
            .unwrap_or_else(|| panic!("no {kind} at {reference}"));
        let words = part::memory_words(kind).unwrap_or_else(|| panic!("{kind} is not a memory"));
        assert!(contents.len() <= words, "{} words into a {kind}", contents.len());
        let mut cells = vec![0u8; words];
        cells[..contents.len()].copy_from_slice(contents);
        self.instances[i].state.cells.clone_from(&cells);
        self.fixed.push((i, cells));
        self.part_dirty[i] = true;
        self.mark_all();
    }

    /// Loads the boot PROM.
    ///
    /// `image` is the programming image --- what was burned, not
    /// microinstructions; see [`crate::prom`]. Each chip holds eight bits of
    /// each word, and which eight is read off the netlist rather than
    /// tabulated: the net on a chip's first data pin is `I<n>`, and `n` is
    /// where its slice starts. That is also why the top chip needs no special
    /// case despite carrying `I47` and `I48` with `I46` missing --- the
    /// image's bits 46 and 47 are the ones that belong there.
    ///
    /// **The address bus is inverted.** PCTL 1D19 is a 74S04A turning `PC0`
    /// into `-PROMPC0` and so on, so the chip decoding word `k` sees `!k`,
    /// and word `k` of the image has to be burned at `!k`. That is the same
    /// inversion the burned images carry, where
    /// the last chip word is the PROM's word 0; the netlist says it directly.
    ///
    /// Only the bank `-PROMCE0` enables is loaded. The second bank is on the
    /// drawing and we have no image for it.
    ///
    /// Returns how many chips were filled.
    pub fn load_prom(&mut self, n: &Netlist, image: &[u64]) -> usize {
        self.generation += 1;
        /// The 74S472's eight data pins, low bit first.
        const DATA: [u8; 8] = [6, 7, 8, 9, 11, 12, 13, 14];
        let mut filled = 0;
        for inst in &mut self.instances {
            if part::strip(&inst.kind).0 != "74S472" {
                continue;
            }
            let enable = inst.net_of[15].map(|net| n.net(net));
            if enable != Some("-PROMCE0") {
                continue;
            }
            let base: u32 = match inst.net_of[DATA[0] as usize].map(|net| n.net(net)) {
                Some(name) => match name.strip_prefix('I').and_then(|d| d.parse().ok()) {
                    Some(b) => b,
                    None => panic!("{} first data pin carries {name}", inst.name()),
                },
                None => panic!("{} has no net on its first data pin", inst.name()),
            };
            let words = part::memory_words(&inst.kind).unwrap();
            let mut cells = vec![0u8; words];
            for (k, word) in image.iter().enumerate().take(words) {
                cells[!k & (words - 1)] = (word >> base) as u8;
            }
            inst.state.cells = cells;
            filled += 1;
        }
        self.mark_all();
        filled
    }

    /// Every net's level, for holding two runs of a board to each other.
    pub fn nets(&self) -> &[Level] {
        &self.nets
    }

    /// The nets the board's oscillators drive: stale on a sleeping board
    /// between the transitions that catch them up.
    pub fn oscillator_outputs(&self) -> Vec<NetId> {
        self.oscillators.iter().map(|o| o.output).collect()
    }

    pub fn net(&self, net: NetId) -> Level {
        self.nets[net as usize]
    }

    /// A counter that moves whenever a net or a part's state changes, and
    /// otherwise stays: two reads of [`Chip::board_level`] at the same
    /// generation give the same answer, so a board that has not moved need
    /// not be asked again. It is not part of the board and not saved; a
    /// load moves it.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Forces a net, for the signals that arrive from off the board.
    pub fn set_net(&mut self, net: NetId, level: Level) {
        if self.put_net(net, level) {
            self.mark_readers(net);
        }
    }

    /// Puts a level on a net, and says whether that moved it. Every write
    /// to a net goes through here: the generation, the net's stamp and the
    /// state hash all follow.
    fn put_net(&mut self, net: NetId, level: Level) -> bool {
        let old = self.nets[net as usize];
        if old == level {
            return false;
        }
        self.nets[net as usize] = level;
        let z = &self.zobrist[net as usize];
        self.state_hash ^= z[level_byte(old) as usize] ^ z[level_byte(level) as usize];
        self.generation += 1;
        self.net_stamp[net as usize] = self.generation;
        true
    }

    /// The nets have been replaced wholesale --- power-on, a checkpoint
    /// --- so the hash is computed afresh and every wire counts as moved.
    fn nets_replaced(&mut self) {
        self.generation += 1;
        self.state_hash = self
            .nets
            .iter()
            .enumerate()
            .fold(0, |h, (net, &l)| h ^ self.zobrist[net][level_byte(l) as usize]);
        self.net_stamp.fill(self.generation);
    }

    /// The generation at which this net's contribution to the outside
    /// last changed: its level, or the level of a gate driving it, which
    /// is what [`Chip::board_level`] resolves. A reader that noted the
    /// generation when it last looked need not look again while this is
    /// no greater. `u64::MAX` for a net that must always be read: one with
    /// a driver the evaluator does not place, or one with a driver still
    /// marked, whose live evaluation may not be what was stamped.
    pub fn net_stamp(&self, net: NetId) -> u64 {
        if self.net_live[net as usize]
            || self.driver_info[net as usize].iter().any(|&(f, _)| self.dirty[f as usize])
        {
            return u64::MAX;
        }
        self.net_stamp[net as usize]
    }

    /// Drives a net from off the board, as the cables do.
    ///
    /// Unlike [`Chip::set_net`] this survives: it is a driver in the
    /// resolution, so a settle that re-evaluates the net's on-board drivers
    /// sees it too. `MEM<31:0>` needs that --- the 74S240s at MDS drive the
    /// same nets outward on a write --- and `-MEMACK` and its fellows would
    /// be content with either, having no on-board driver at all.
    pub fn drive(&mut self, net: NetId, level: Level) {
        let d = Driver { level, drive: Drive::TriState };
        if self.external[net as usize] != Some(d) {
            self.external[net as usize] = Some(d);
            self.resolve_from_outside(net);
        }
    }

    /// What the board's own drivers make of a net, leaving out whatever is
    /// driving it from off the board: the level, and whether anything
    /// stronger than a pull-up is holding it there. A cable joining two
    /// boards asks this of each end to know which is talking.
    pub fn board_level(&self, net: NetId) -> (Level, bool) {
        // On the stack: this is asked about every wire of every board at
        // every exchange, and a heap allocation per call was a quarter of a
        // run.
        const ON_STACK: usize = 16;
        let drivers = &self.drivers[net as usize];
        let mut stack = [Driver { level: Level::Z, drive: Drive::Passive }; ON_STACK];
        let mut heap: Vec<Driver> = Vec::new();
        for (k, &(f, drive)) in self.driver_info[net as usize].iter().enumerate() {
            let f = f as usize;
            let level = if self.evaluated[f] && !self.dirty[f] {
                self.gate_level[f]
            } else {
                self.eval_gate(drivers[k])
            };
            let d = Driver { level, drive };
            if drivers.len() <= ON_STACK {
                stack[k] = d;
            } else {
                heap.push(d);
            }
        }
        let on_it: &[Driver] =
            if drivers.len() <= ON_STACK { &stack[..drivers.len()] } else { &heap };
        let strong = on_it.iter().any(|d| {
            !matches!(d.drive, Drive::Passive | Drive::PullUp)
                && !matches!(d.level, Level::Z)
                && !matches!(
                    (d.drive, d.level),
                    (Drive::OpenCollector, Level::High) | (Drive::OpenEmitter, Level::Low)
                )
        });
        (part::resolve(on_it), strong)
    }

    /// What is driving a net from off the board, if anything is.
    pub fn external_on(&self, net: NetId) -> Option<Level> {
        self.external[net as usize].and_then(|d| (d.drive != Drive::PullUp).then_some(d.level))
    }

    /// Pulls a net up from off the board, as a bus terminator does: an
    /// open-collector line the board is not pulling low reads high.
    pub fn pull_up(&mut self, net: NetId) {
        let d = Driver { level: Level::High, drive: Drive::PullUp };
        if self.external[net as usize] != Some(d) {
            self.external[net as usize] = Some(d);
            self.resolve_from_outside(net);
        }
    }

    /// Pulls a net down from off the board, as a MECL line terminator
    /// does: the Thevenin divider of `SIP-R121/195-8` holds an open
    /// emitter near -2 V, below the low threshold, and any output on the
    /// line pulling high wins. The same weak class as [`Chip::pull_up`],
    /// the other way up.
    ///
    /// The display board's video pair is terminated at the monitor and
    /// nowhere else --- MIT's `necsip.drw` terminates the other 26 ECL
    /// nets and not those two --- so `crate::terminal::monitor` is what
    /// puts this on, and without it the pair reads high-impedance for its
    /// low (discrepancy 50).
    pub fn pull_down(&mut self, net: NetId) {
        let d = Driver { level: Level::Low, drive: Drive::PullUp };
        if self.external[net as usize] != Some(d) {
            self.external[net as usize] = Some(d);
            self.resolve_from_outside(net);
        }
    }

    /// Stops driving it, which for the data bus is most of the time.
    pub fn release(&mut self, net: NetId) {
        if self.external[net as usize].is_some() {
            self.external[net as usize] = None;
            self.resolve_from_outside(net);
        }
    }

    /// How many gates are in the evaluation order.
    pub fn ordered(&self) -> usize {
        self.order.iter().map(|g| g.len()).sum()
    }

    /// The evaluation order, as groups.
    pub fn groups(&self) -> &[Vec<GateId>] {
        &self.order
    }

    /// Every gate driving a net.
    pub fn drivers_on(&self, net: NetId) -> &[GateId] {
        &self.drivers[net as usize]
    }

    /// The pins a gate reads, and the nets they are on.
    pub fn gate_inputs(&self, id: GateId) -> Vec<Option<NetId>> {
        let inst = &self.instances[id.part as usize];
        inst.behaviour.gates[id.gate as usize]
            .ins
            .iter()
            .map(|&p| inst.net_of[p as usize])
            .collect()
    }

    /// The feedback groups, largest first.
    pub fn feedback(&self) -> Vec<&[GateId]> {
        let mut v: Vec<&[GateId]> =
            self.order.iter().filter(|g| g.len() > 1).map(|g| g.as_slice()).collect();
        v.sort_by_key(|g| std::cmp::Reverse(g.len()));
        v
    }

    /// Brings every net to rest.
    ///
    /// Groups are evaluated in order. A group of one gate needs evaluating
    /// once, because everything feeding it has already settled. A feedback
    /// group is swept until its nets stop changing, which is what the real
    /// gates do with their own delays.
    ///
    /// Only the groups holding a dirty gate are visited, off `group_dirty`:
    /// a word of it at a time, the lowest set bit first. A group marks only
    /// groups after it --- that is what the order is --- so a bit set while
    /// a group is evaluated is either ahead, and reached in this pass, or
    /// the group's own, which is left to the next settle as it always was.
    /// Walking every group instead, and asking each whether it was dirty,
    /// was a fifth of a `chip` run.
    pub fn settle(&mut self) {
        self.unsettled = None;
        self.sweeps = 0;
        let mut on_it = std::mem::take(&mut self.scratch);
        let groups = self.order.len();
        let mut w = 0;
        let mut passed = 0u64;
        // Nothing left to evaluate: an idle board's transition ends here.
        while w < self.group_dirty.len() && self.dirty_count > 0 {
            let word = self.group_dirty[w] & !passed;
            if word == 0 {
                w += 1;
                passed = 0;
                continue;
            }
            let bit = word.trailing_zeros();
            passed = (2u64 << bit).wrapping_sub(1);
            self.group_dirty[w] &= !(1u64 << bit);
            let k = w * 64 + bit as usize;
            if k >= groups {
                break;
            }
            let (start, end) = (self.group_start[k] as usize, self.group_start[k + 1] as usize);
            if end - start == 1 {
                let (id, f) = self.order_flat[start];
                let f = f as usize;
                if !self.dirty[f] {
                    continue;
                }
                if let Some(net) = self.out_net(id) {
                    let before = self.nets[net as usize];
                    self.resolve_net(net, &mut on_it);
                    self.clean(f);
                    if self.nets[net as usize] != before {
                        self.mark_readers(net);
                    }
                } else {
                    self.clean(f);
                }
                continue;
            }
            if !(start..end).any(|i| self.dirty[self.order_flat[i].1 as usize]) {
                continue;
            }
            let mut settled = false;
            for sweep in 1..=Self::SWEEP_LIMIT {
                self.sweeps = self.sweeps.max(sweep);
                let mut changed = false;
                for i in start..end {
                    let (id, f) = self.order_flat[i];
                    let f = f as usize;
                    if !self.dirty[f] {
                        continue;
                    }
                    let Some(net) = self.out_net(id) else {
                        self.clean(f);
                        continue;
                    };
                    let before = self.nets[net as usize];
                    self.resolve_net(net, &mut on_it);
                    self.clean(f);
                    if self.nets[net as usize] == Level::X && before != Level::X {
                        // A loop cannot be swept in dependency order --- that
                        // is what makes it a loop --- so some gate always
                        // reads an input the sweep has not reached yet. A
                        // stale read on a bus enable turns two drivers on at
                        // once, and [`part::resolve`] rightly calls that
                        // unknown. But unknown is absorbing: it goes round
                        // the loop, returns as an unknown input, and the
                        // machine never recovers.
                        //
                        // In the hardware that moment is a transient current
                        // between two drivers, not a value, and the gates
                        // settle. So a *new* unknown inside a loop holds its
                        // previous level and the sweep goes round again. A
                        // conflict that is real survives every sweep, and
                        // the group is then reported through
                        // [`Chip::unsettled`] rather than quietly resolving
                        // to a wrong value.
                        self.put_net(net, before);
                        // Still dirty, so the sweep comes back to it. A
                        // conflict that is real survives every sweep and the
                        // group is reported through `unsettled`.
                        self.soil(f);
                        changed = true;
                        continue;
                    }
                    if self.nets[net as usize] != before {
                        self.mark_readers(net);
                        changed = true;
                    }
                }
                if !changed {
                    settled = true;
                    break;
                }
            }
            if settled {
                // Its own gates marked it while it was swept, and none of
                // them is dirty now.
                self.group_dirty[w] &= !(1u64 << bit);
            } else if self.unsettled.is_none() {
                self.unsettled = Some(self.order[k].clone());
            }
        }
        self.scratch = on_it;
    }

    /// Writes everything the engine holds, so a run can be picked up again.
    ///
    /// A microcycle of `chip` costs about 0.6 ms, so reaching anything
    /// interesting in the boot takes minutes; this is what makes iterating
    /// on a divergence deep in a run practical. Only what the engine
    /// *remembers* is written --- every net, and what every part holds ---
    /// because the rest is derived from the netlist and comes back from
    /// [`Chip::new`].
    ///
    /// The other half of resuming a comparison is not here and does not need
    /// to be: `rtl` runs the whole boot in under a second, so it is
    /// cheaper to replay it than to store it.
    ///
    /// A checkpoint carries [`Chip::fingerprint`] and is refused if it does
    /// not match, because loading one from a different board would be wrong
    /// in exactly the quiet way this project keeps finding. The magic carries
    /// a version with it: a `CADRCHK1` file holds `prev_nets` as the board
    /// before the last transition rather than after it, so reading one here
    /// would clock every register twice on the first tick, and it is refused
    /// rather than converted.
    pub fn save(&self, w: &mut impl std::io::Write) -> std::io::Result<()> {
        w.write_all(b"CADRCHK5")?;
        w.write_all(&self.fingerprint().to_le_bytes())?;
        for levels in [&self.nets, &self.prev_nets] {
            w.write_all(&(levels.len() as u32).to_le_bytes())?;
            let bytes: Vec<u8> = levels.iter().map(|&l| level_byte(l)).collect();
            w.write_all(&bytes)?;
        }
        for flags in [&self.dirty, &self.part_dirty] {
            w.write_all(&(flags.len() as u32).to_le_bytes())?;
            let bytes: Vec<u8> = flags.iter().map(|&b| b as u8).collect();
            w.write_all(&bytes)?;
        }
        w.write_all(&(self.instances.len() as u32).to_le_bytes())?;
        for inst in &self.instances {
            w.write_all(&inst.state.bits.to_le_bytes())?;
            w.write_all(&(inst.state.cells.len() as u32).to_le_bytes())?;
            w.write_all(&inst.state.cells)?;
        }
        // CADRCHK3: the timers. An oscillator's next edge and a one-shot's
        // pulse run across any checkpoint of the memory board, whose
        // oscillator never stops and whose refresh timer is always running.
        let time = |t: Option<u64>| t.unwrap_or(u64::MAX).to_le_bytes();
        // CADRCHK4: an oscillator is where it started and how many times it
        // has toggled since, or not running; a 74LS124 section, whose
        // internal oscillator runs from `t = 0`, is since when its enable
        // has been low, or disabled.
        for o in &self.oscillators {
            w.write_all(&time(o.phase()))?;
            w.write_all(&o.edges.to_le_bytes())?;
        }
        for o in &self.one_shots {
            w.write_all(&time(o.fall))?;
            let (a, b) = o.last.map_or((255, 255), |(a, b)| (level_byte(a), level_byte(b)));
            w.write_all(&[a, b])?;
        }
        Ok(())
    }

    /// Reads back what [`Chip::save`] wrote, onto a board built from the
    /// same netlist. The board does not have to have been powered on: this
    /// replaces everything power-on would set.
    pub fn load(&mut self, r: &mut impl std::io::Read) -> std::io::Result<()> {
        self.generation += 1;
        let bad = |what: &str| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("checkpoint: {what}"))
        };
        let mut magic = [0u8; 8];
        r.read_exact(&mut magic)?;
        // CADRCHK5 stores the timers as they are now; CADRCHK3 stored an
        // oscillator as its next edge alone, which cannot place a running
        // one now, and CADRCHK2 stored no timers. Both are fine for a
        // board with none running, which the bus interface's is at any
        // quiet point. CADRCHK4 stored a one-shot by the wrong edge.
        let (timers, edges) = match &magic {
            b"CADRCHK5" => (true, true),
            b"CADRCHK3" => (true, false),
            b"CADRCHK2" => (false, false),
            _ => return Err(bad("not one of ours, or a format this build does not write")),
        };
        let mut word = [0u8; 8];
        r.read_exact(&mut word)?;
        if u64::from_le_bytes(word) != self.fingerprint() {
            return Err(bad("saved from a different board or a different build"));
        }
        let count = |r: &mut dyn std::io::Read| -> std::io::Result<usize> {
            let mut n = [0u8; 4];
            r.read_exact(&mut n)?;
            Ok(u32::from_le_bytes(n) as usize)
        };
        for which in 0..2 {
            let n = count(r)?;
            if n != self.nets.len() {
                return Err(bad("wrong number of nets"));
            }
            let mut bytes = vec![0u8; n];
            r.read_exact(&mut bytes)?;
            let levels: Result<Vec<Level>, _> = bytes.iter().map(|&b| byte_level(b)).collect();
            let levels = levels.map_err(|_| bad("a net level is not one of the four"))?;
            if which == 0 {
                self.nets = levels;
            } else {
                self.prev_nets = levels;
                self.fresh = false;
            }
        }
        for which in 0..2 {
            let n = count(r)?;
            let want = if which == 0 { self.dirty.len() } else { self.part_dirty.len() };
            if n != want {
                return Err(bad("wrong number of gates or parts"));
            }
            let mut bytes = vec![0u8; n];
            r.read_exact(&mut bytes)?;
            let flags: Vec<bool> = bytes.iter().map(|&b| b != 0).collect();
            if which == 0 {
                self.dirty_count =
                    flags.iter().zip(&self.evaluated).filter(|(d, e)| **d && **e).count();
                self.dirty = flags;
            } else {
                self.part_dirty = flags;
            }
        }
        if count(r)? != self.instances.len() {
            return Err(bad("wrong number of parts"));
        }
        for inst in &mut self.instances {
            let mut bits = [0u8; 2];
            r.read_exact(&mut bits)?;
            inst.state.bits = u16::from_le_bytes(bits);
            let mut n = [0u8; 4];
            r.read_exact(&mut n)?;
            // Sized from the file rather than required to match, so a
            // board straight out of `Chip::new` can be loaded into without
            // powering it on first. The fingerprint has already agreed
            // about every part's type, which is what fixes the size.
            inst.state.cells = vec![0u8; u32::from_le_bytes(n) as usize];
            r.read_exact(&mut inst.state.cells)?;
        }
        if timers {
            let mut t = [0u8; 8];
            for i in 0..self.oscillators.len() {
                r.read_exact(&mut t)?;
                let phase = Some(u64::from_le_bytes(t)).filter(|&v| v != u64::MAX);
                if !edges {
                    if phase.is_some() {
                        return Err(bad("a running oscillator saved before its phase was"));
                    }
                    let o = &mut self.oscillators[i];
                    o.next = None;
                    o.enabled_at = None;
                    if o.enable.is_none() {
                        o.origin = None;
                    }
                    continue;
                }
                r.read_exact(&mut t)?;
                let (period, count) = (self.oscillators[i].period, u64::from_le_bytes(t));
                let o = &mut self.oscillators[i];
                o.edges = count;
                if o.enable.is_some() {
                    // A file written when a section's clock was reckoned
                    // from its enable kept that instant here too, so it
                    // restores the same.
                    o.enabled_at = phase;
                    o.next = phase.map(|_| toggle_at(period, count + 1));
                } else {
                    o.origin = phase;
                    o.next = phase.map(|at| at + toggle_at(period, count + 1));
                }
            }
            for i in 0..self.one_shots.len() {
                r.read_exact(&mut t)?;
                self.one_shots[i].fall = Some(u64::from_le_bytes(t)).filter(|&v| v != u64::MAX);
                let mut b = [0u8; 2];
                if edges {
                    r.read_exact(&mut b)?;
                } else {
                    r.read_exact(&mut b[..1])?;
                    b[1] = b[0];
                }
                self.one_shots[i].last = if b[0] == 255 {
                    None
                } else {
                    Some((
                        byte_level(b[0]).map_err(|_| bad("a one-shot level"))?,
                        byte_level(b[1]).map_err(|_| bad("a one-shot level"))?,
                    ))
                };
            }
        }
        self.rebuild_derived();
        self.unsettled = None;
        self.sweeps = 0;
        Ok(())
    }

    /// A fingerprint of the board as this build describes it: the netlist's
    /// wiring, and every part's type, pinout and gate list.
    ///
    /// It is there to stop a checkpoint being loaded onto a board it does
    /// not describe. **It does not cover what a gate computes** --- editing
    /// the body of a function in [`crate::part`] leaves it unchanged --- so
    /// a checkpoint is only as good as the build that wrote it, and one kept
    /// across a change to part behaviour will be silently stale.
    pub fn fingerprint(&self) -> u64 {
        let mut h = fnv(0xcbf2_9ce4_8422_2325, &(self.nets.len() as u64).to_le_bytes());
        for inst in &self.instances {
            h = fnv(h, inst.kind.as_bytes());
            for slot in &inst.net_of {
                h = fnv(h, &slot.unwrap_or(u32::MAX).to_le_bytes());
            }
            for gate in inst.behaviour.gates {
                h = fnv(h, &[gate.out]);
                h = fnv(h, gate.ins);
            }
            h = fnv(h, inst.behaviour.update_ins);
        }
        h
    }

    /// How much work is outstanding: gates still to evaluate, and parts
    /// still to ask for an edge.
    ///
    /// Both are zero on a settled board between clock transitions, which is
    /// what makes them worth exposing --- it is the difference between "this
    /// board has caught up" and "it happens to look right at the moment".
    pub fn pending(&self) -> (usize, usize) {
        (self.dirty.iter().filter(|&&d| d).count(), self.part_dirty.iter().filter(|&&d| d).count())
    }

    /// A gate's index in the flat numbering the dirty set uses.
    fn flat(&self, id: GateId) -> usize {
        self.gate_base[id.part as usize] as usize + id.gate as usize
    }

    /// Marks a gate for evaluation.
    fn soil(&mut self, f: usize) {
        if !self.dirty[f] && self.evaluated[f] {
            self.dirty[f] = true;
            self.dirty_count += 1;
            let g = self.group_of[f] as usize;
            self.group_dirty[g / 64] |= 1u64 << (g % 64);
        }
    }

    /// Marks every gate of a part, for a change to what it holds that did
    /// not come through its update.
    fn soil_part(&mut self, i: usize) {
        let base = self.gate_base[i] as usize;
        for f in base..base + self.instances[i].behaviour.gates.len() {
            self.soil(f);
        }
    }

    /// Takes a gate off the evaluation list.
    fn clean(&mut self, f: usize) {
        if self.dirty[f] {
            self.dirty[f] = false;
            self.dirty_count -= 1;
        }
    }

    /// Records that a net has moved: every gate reading it must be
    /// re-evaluated, and every part that remembers and touches it must be
    /// asked again whether it has seen an edge.
    fn mark_readers(&mut self, net: NetId) {
        for k in 0..self.readers[net as usize].len() {
            let f = self.readers[net as usize][k] as usize;
            self.soil(f);
        }
        for k in 0..self.touchers[net as usize].len() {
            let i = self.touchers[net as usize][k] as usize;
            if !self.part_dirty[i] {
                self.part_dirty[i] = true;
                self.part_dirty_list.push(i as u32);
            }
        }
    }

    /// Marks the whole board, for when something outside the net graph has
    /// changed: power-on, or a PROM being loaded.
    ///
    /// Public because it is the reference the incremental machinery is
    /// checked against: a chip that marks everything before every tick does
    /// the work unconditionally, and must reach the same state as one that
    /// does not. See `tests/chip.rs`.
    pub fn mark_all(&mut self) {
        self.mark_every_gate();
        self.part_dirty.fill(true);
        self.part_dirty_list = (0..self.instances.len() as u32).collect();
    }

    /// Flags every gate the evaluator visits, and keeps the count in step.
    ///
    /// The count is what [`Chip::settle`] stops on, so a fill that left it
    /// behind was a trap: the settle consumed that many gates and stopped,
    /// the rest stayed flagged, and a flagged gate cannot be flagged again,
    /// so a net feeding one kept its stale level for ever after. Setting
    /// the I/O board's Chaosnet address switches on a settled board did
    /// exactly that, and its Unibus receivers stopped following the bus.
    /// `the_address_switches_read_back` in `tests/chaos_netlist.rs` is the
    /// regression; the constructor's seeding is the rule followed.
    fn mark_every_gate(&mut self) {
        for f in 0..self.dirty.len() {
            self.dirty[f] = self.evaluated[f];
        }
        self.dirty_count = self.evaluated.iter().filter(|&&e| e).count();
        self.group_dirty.fill(!0);
        let tail = self.order.len() % 64;
        if tail != 0
            && let Some(last) = self.group_dirty.last_mut()
        {
            *last = (1u64 << tail) - 1;
        }
    }

    /// Rebuilds what is derived from the flags and the nets after a load:
    /// the dirty groups, the list of dirty parts, and the cached level of
    /// every placed gate, evaluated from the nets as loaded. A clean gate's
    /// inputs are what they were when it was last evaluated, so evaluating
    /// it again gives the level it computed then.
    fn rebuild_derived(&mut self) {
        self.nets_replaced();
        self.group_dirty.fill(0);
        for f in 0..self.dirty.len() {
            if self.dirty[f] && self.evaluated[f] {
                let g = self.group_of[f] as usize;
                self.group_dirty[g / 64] |= 1u64 << (g % 64);
            }
        }
        self.part_dirty_list =
            (0..self.part_dirty.len()).filter(|&i| self.part_dirty[i]).map(|i| i as u32).collect();
        for i in 0..self.order_flat.len() {
            let (id, f) = self.order_flat[i];
            let level = self.eval_gate(id);
            self.gate_level[f as usize] = level;
        }
    }

    /// Settles from scratch, evaluating every gate rather than only the ones
    /// whose inputs moved.
    ///
    /// The two must agree. `settle` skips work on the strength of the reader
    /// map being complete and the group order being right; if either were
    /// wrong a net would quietly keep a stale level, and nothing else would
    /// say so. `tests/chip.rs` runs the machine and then checks that a full
    /// settle moves nothing.
    pub fn settle_all(&mut self) {
        // Gates only. Marking the parts too would hand the caller a board
        // that is no longer running incrementally, which quietly disarms any
        // test using this to check that it is.
        self.mark_every_gate();
        self.settle();
    }

    /// What the clock generator sees of the board.
    ///
    /// `-HANG` and `-ILONG` are active low on the drawings and active high
    /// in [`crate::clock::Inputs`], so they are inverted here.
    pub fn clock_inputs(&self) -> clock::Inputs {
        let clock_in = self.clock_in.as_ref().expect("this board has no clock generator");
        let high = |net: NetId| self.nets[net as usize].read().unwrap_or(true);
        let speed = match (high(clock_in.sspeed1), high(clock_in.sspeed0)) {
            (false, false) => clock::Speed::ExtraSlow,
            (false, true) => clock::Speed::Slow,
            (true, false) => clock::Speed::Normal,
            (true, true) => clock::Speed::Fast,
        };
        clock::Inputs {
            machrun: high(clock_in.machrun),
            hang: !high(clock_in.hang),
            ilong: !high(clock_in.ilong),
            speed,
            reset: high(clock_in.reset),
        }
    }

    /// Puts the clock's outputs onto the board.
    fn apply_clock(&mut self, out: clock::Outputs, phase_ns: u32) {
        let tpr60 = (60..60 + clock::TPR_PULSE_NS).contains(&phase_ns);
        for k in 0..self.clock_nets.len() {
            let (net, which) = self.clock_nets[k];
            let level = Level::from(match which {
                ClockOut::Tpclk => out.tpclk,
                ClockOut::NotTpclk => !out.tpclk,
                ClockOut::Tptse => out.tptse,
                ClockOut::NotTpwp => !out.tpwp,
                ClockOut::NotTpwpiram => !out.tpwpiram,
                ClockOut::NotTpr60 => !tpr60,
            });
            if self.put_net(net, level) {
                self.mark_readers(net);
            }
        }
    }

    /// Advances to the clock's next transition and brings the board with it.
    ///
    /// The clock is asked what the board looks like, moves to its own next
    /// event, and its outputs are put back on the board. Then the gates
    /// settle, the parts that remember are given the chance to see an edge,
    /// and the gates settle again --- because a register that has just
    /// latched is driving something new.
    ///
    /// Returns the nanoseconds that passed.
    pub fn tick(&mut self, clk: &mut dyn clock::Clock) -> u32 {
        if self.fresh {
            self.snapshot();
            self.fresh = false;
        }
        let inputs = self.clock_inputs();
        let (ns, out) = clk.advance(inputs);
        self.apply_clock(out, clk.phase_ns());
        self.transition(clk.time_ns());
        ns
    }

    /// Everything that follows a change on the board, at `now`: the
    /// delay-line taps due by then fire, every part that remembers gets to
    /// see an edge, the gates settle, and the snapshot the next edges are
    /// found against is taken.
    ///
    /// [`Chip::tick`] runs this after moving the clock, and the cables run
    /// it at their own events --- an acknowledgement, the strobe after it,
    /// a tap releasing `-HANG` --- which fall between clock transitions, and
    /// are settled at their own time rather than at the next one. Two things
    /// the order settles: a tap
    /// due now acts before the registers are asked, so an asynchronous
    /// clear off a delay line lands at the tap's time and not a transition
    /// later; and the delay lines are looked at again once the gates have
    /// settled, so a change the cable just made is on its way down a line
    /// from `now`.
    ///
    /// The snapshot is taken here, at the end, and not at the start of the
    /// next tick. What the cables drive between transitions ---
    /// [`Chip::drive`] --- has to be a change `Chip::update_state` can
    /// see, and taken at the start of the next tick the snapshot already
    /// held it. `-LOADMD`'s rising edge was invisible that way, and `MD`
    /// never took a word off the bus.
    pub fn transition(&mut self, now: u64) {
        if self.fresh {
            self.snapshot();
            self.fresh = false;
        }
        self.advance_delays(now);
        self.settle();
        // A register clocked from another register's output --- the 74276
        // at REQERR 0B02, clocked by `-NXM TIMEOUT` off the 74LS273 at
        // REQTIM 0B01 --- sees its edge only after the first has latched
        // and the gates between have settled. So the parts are asked again,
        // against a fresh snapshot so that nothing is clocked twice, until
        // a round moves no net. The processor clocks everything from the
        // clock nets and never needs a second round.
        //
        // The snapshot is taken right after the parts are asked, before
        // what they latched has propagated, so that the next round sees
        // those changes as edges against it; and the last round, which
        // moves nothing, leaves the snapshot at the board's final state.
        //
        // A round that moved no net is the last. Whatever a settle or a tap
        // moves bumps the generation, so a generation standing still over
        // the round says the nets stand where the snapshot has them, with
        // no need to compare the two. A net that moved and moved back
        // inside a loop's sweeps bumps it as well, and costs one more
        // round, which finds nothing.
        self.rounds = 0;
        let mut converged = false;
        while self.rounds < Self::UPDATE_ROUNDS {
            self.rounds += 1;
            self.update_state();
            self.snapshot();
            let before = self.generation;
            self.settle();
            self.advance_delays(now);
            self.settle();
            if self.generation == before {
                converged = true;
                break;
            }
        }
        if !converged {
            self.unconverged = Some(now);
        }
        if !self.oscillators.is_empty() {
            self.remember();
        }
    }

    /// How many rounds of asking the parts a transition gets before it is
    /// given up as [`Chip::unconverged`]. The bus interface's longest chain
    /// of registers clocked from registers is three deep.
    pub const UPDATE_ROUNDS: usize = 8;

    /// Records the board's state among the recent ones, and whether it was
    /// already there, for [`Chip::asleep`].
    fn remember(&mut self) {
        let hash = self.state_hash;
        let seen = self.recent.iter().any(|(h, nets)| *h == hash && *nets == self.nets);
        self.quiet = if seen { self.quiet + 1 } else { 0 };
        if self.recent.len() < Self::RECENT {
            self.recent.push((hash, self.nets.clone()));
        } else {
            let slot = &mut self.recent[self.recent_next];
            slot.0 = hash;
            slot.1.copy_from_slice(&self.nets);
            self.recent_next = (self.recent_next + 1) % Self::RECENT;
        }
    }

    /// How many net states [`Chip::asleep`] looks back over: more than a
    /// bus clock period of oscillator edges and bus clock edges, since a
    /// static board only returns to the state a bus clock edge leaves it in
    /// a period later, thirteen transitions on.
    const RECENT: usize = 16;

    /// How many transitions in a row must end in a recent state before the
    /// board is asleep: more than a bus clock half-period of oscillator
    /// edges, so every phase of both clocks has been seen to bring nothing.
    const QUIET: u32 = 16;

    /// Whether the board's clocks are ticking over static logic: every
    /// transition for a while has only returned it to a state it was in a
    /// few transitions before, so the oscillator edges to come will do the
    /// same, until something else reaches the board --- a wire, a
    /// one-shot's fall, a delay tap. A sleeping board can skip its edges;
    /// its oscillator catches up, keeping the phase, at the next
    /// transition. The memory board is asleep between cycles and
    /// refreshes, which is nearly always.
    pub fn asleep(&self) -> bool {
        self.quiet >= Self::QUIET && !self.taps_pending()
    }

    /// When each oscillator's next edge is due, for diagnosis.
    pub fn oscillator_next(&self) -> Vec<Option<u64>> {
        self.oscillators.iter().map(|o| o.next).collect()
    }

    /// When something other than an oscillator edge is due: a delay tap or
    /// a one-shot's fall. What wakes a sleeping board.
    pub fn next_wake(&self) -> Option<u64> {
        let taps = self.delays.iter().flat_map(|d| d.pending.iter().map(|&(at, _, _)| at));
        let falls = self.one_shots.iter().filter_map(|o| o.fall);
        taps.chain(falls).min()
    }

    /// When the next delay-line tap is due, if one is on its way.
    pub fn next_tap(&self) -> Option<u64> {
        let taps = self.delays.iter().flat_map(|d| d.pending.iter().map(|&(at, _, _)| at));
        let edges = self.oscillators.iter().filter_map(|o| o.next);
        let falls = self.one_shots.iter().filter_map(|o| o.fall);
        taps.chain(edges).chain(falls).min()
    }

    /// Whether a delay line has a tap in flight: the one kind of pending
    /// event a board should not be checkpointed across. An oscillator's
    /// next edge and a one-shot's fall are saved with the board.
    pub fn taps_pending(&self) -> bool {
        self.delays.iter().any(|d| !d.pending.is_empty())
    }

    /// One bit of a part's memory array, eight to a cell, as the 4116
    /// keeps them; `None` if the part has no such cell.
    pub fn cell_bit(&self, instance: usize, a: usize) -> Option<bool> {
        self.instances[instance].state.cells.get(a / 8).map(|&c| c >> (a % 8) & 1 != 0)
    }

    /// What a part holds, to change from outside the board: a harness
    /// filling a memory with a program, or the disk's transfer landing.
    /// The part's gates are marked, since what they drive may depend on
    /// it and they keep a cached level while they are clean; writing
    /// through `instances[i].state` directly does not mark them.
    pub fn state_mut(&mut self, instance: usize) -> &mut State {
        self.generation += 1;
        self.soil_part(instance);
        &mut self.instances[instance].state
    }

    /// Sets one such bit, from outside the board.
    pub fn set_cell_bit(&mut self, instance: usize, a: usize, bit: bool) {
        if let Some(c) = self.instances[instance].state.cells.get_mut(a / 8) {
            let m = 1u8 << (a % 8);
            *c = if bit { *c | m } else { *c & !m };
            // What the part drives may depend on the cell, and its gates
            // keep a cached level while they are clean.
            self.soil_part(instance);
        }
    }

    /// Closes the switches of the DIP switch at `reference`: bit `k` of
    /// `closed` is the switch on pin `9 + k`. The memory board's sets the
    /// six address bits above 64K that select the board; the I/O board's
    /// two `SWITCH` bodies hold the Chaosnet address,
    /// [`crate::chaos::interface::switches`].
    pub fn set_switches(&mut self, reference: &str, closed: u8) {
        let i = self
            .instances
            .iter()
            .position(|inst| {
                inst.reference == reference && (inst.kind == "DIPSW" || inst.kind == "SWITCH")
            })
            .unwrap_or_else(|| panic!("no DIPSW or SWITCH at {reference}"));
        self.instances[i].state.bits = closed as u16;
        self.generation += 1;
        self.part_dirty[i] = true;
        self.mark_all();
    }

    /// Carries the VCTL1 delay lines forward to `now`.
    ///
    /// A tap follows its input by a fixed time, so a transition is scheduled
    /// when the input moves and applied when the clock reaches it. Nothing
    /// else drives a tap, so these are ordinary net writes.
    fn advance_delays(&mut self, now: u64) {
        for i in 0..self.delays.len() {
            // A delay line's input is a TTL input: a net nothing is driving
            // --- `INT BUSY`, an open-collector NAND that lets go --- reads
            // high, and the taps carry the level read, not the float.
            let level = match self.nets[self.delays[i].input as usize] {
                Level::Z => Level::High,
                l => l,
            };
            if self.delays[i].last != Some(level) {
                self.delays[i].last = Some(level);
                for t in 0..self.delays[i].taps.len() {
                    let at = now + self.delays[i].taps[t].1 as u64;
                    self.delays[i].pending.push((at, t, level));
                }
            }
            let mut fired: Vec<(usize, Level)> = Vec::new();
            self.delays[i].pending.retain(|&(at, t, l)| {
                if at <= now {
                    fired.push((t, l));
                    false
                } else {
                    true
                }
            });
            for (t, l) in fired {
                let net = self.delays[i].taps[t].0;
                let l = match (self.delays[i].invert, l) {
                    (true, Level::High) => Level::Low,
                    (true, Level::Low) => Level::High,
                    (_, l) => l,
                };
                self.set_net(net, l);
            }
        }
        for i in 0..self.one_shots.len() {
            let o = &self.one_shots[i];
            let (i0, i1, clear, q, not_q, width) = (o.i0, o.i1, o.clear, o.q, o.not_q, o.width_ns);
            let pulled = |l: Level| if l == Level::Z { Level::High } else { l };
            let (a, b, cd) = (
                pulled(self.nets[i0 as usize]),
                pulled(self.nets[i1 as usize]),
                pulled(self.nets[clear as usize]),
            );
            let triggered = match self.one_shots[i].last {
                Some((a0, b0)) => {
                    (a0 == Level::High && a == Level::Low && b == Level::Low)
                        || (b0 == Level::Low && b == Level::High && a == Level::High)
                }
                None => false,
            };
            self.one_shots[i].last = Some((a, b));
            let up = if cd == Level::Low {
                self.one_shots[i].fall = None;
                false
            } else if triggered {
                self.one_shots[i].fall = Some(now + width);
                true
            } else if self.one_shots[i].fall.is_some_and(|at| at <= now) {
                self.one_shots[i].fall = None;
                false
            } else {
                self.one_shots[i].fall.is_some()
            };
            self.set_net(q, if up { Level::High } else { Level::Low });
            self.set_net(not_q, if up { Level::Low } else { Level::High });
        }
        for i in 0..self.oscillators.len() {
            let o = &self.oscillators[i];
            let (output, enable, period) = (o.output, o.enable, o.period);
            // A DIP can starts at the board's first transition; a 74LS124
            // section has run since `t = 0`. See [`Oscillator`].
            let origin = *self.oscillators[i].origin.get_or_insert(now);
            let edges = self.oscillators[i].edges;
            // Every toggle due by now, in one step rather than one at a
            // time: the `k`-th is at `origin + k num / 2 den`, so the last
            // due is `(now - origin) 2 den / num` but for the rounding,
            // which the two loops put right. A board asleep for a
            // millisecond owes tens of thousands of them.
            let mut due = (now.saturating_sub(origin) * 2 * period.1 / period.0).max(edges);
            while origin + toggle_at(period, due + 1) <= now {
                due += 1;
            }
            while due > edges && origin + toggle_at(period, due) > now {
                due -= 1;
            }
            self.oscillators[i].edges = due;
            // High at the origin, so an even count of toggles leaves it high.
            let internal = if due.is_multiple_of(2) { Level::High } else { Level::Low };
            let level = match enable {
                None => internal,
                Some(e) if self.nets[e as usize] != Level::Low => {
                    // Disabled: the output is high, and the board need not
                    // wake for the toggles the oscillator makes meanwhile.
                    self.oscillators[i].enabled_at = None;
                    self.oscillators[i].next = None;
                    self.set_net(output, Level::High);
                    continue;
                }
                Some(_) => {
                    // Enabled: high until the internal oscillator's first
                    // fall after the enable, following it from there.
                    debug_assert_eq!(origin, 0, "a gated section runs from t = 0");
                    let since = *self.oscillators[i].enabled_at.get_or_insert(now);
                    if gated_fall(period, since) <= now { internal } else { Level::High }
                }
            };
            self.oscillators[i].next = Some(origin + toggle_at(period, due + 1));
            self.set_net(output, level);
        }
    }

    /// Runs one microcycle: clock transitions until the next cycle begins.
    ///
    /// The cycle boundary is the only phase-aligned point the two engines
    /// share. `rtl` has no phases at all, so comparing them anywhere else is
    /// comparing a settled value against one that has not happened yet.
    pub fn microcycle(&mut self, clk: &mut dyn clock::Clock) -> u32 {
        let mut ns = 0;
        for _ in 0..64 {
            ns += self.tick(clk);
            if clk.phase_ns() == 0 {
                break;
            }
        }
        ns
    }

    /// The net carrying one bit of a numbered bus.
    ///
    /// `A31` is why this is not just a `format!`. There is no such net:
    /// ALATCH drives bit 31 of the A bus onto two wires, `A31A` and `A31B`,
    /// to split the load, which is the same suffix convention the drawings
    /// use for `-AADR0A` and `-AADR0B` or for `WP3A`. So a bit is looked up
    /// under its plain name and then under the first of the split pair.
    ///
    /// **Missing is a panic, not a zero.** A bus read that quietly skips the
    /// bit it cannot find reports a value the board never carried, and then
    /// a comparison against another engine blames the wrong thing. `A31` is
    /// the one to think about: the split pair is why a lookup that gives up
    /// quietly would return a 31-bit bus and disagree with `rtl` in exactly
    /// one place.
    fn bus_net(&self, n: &Netlist, prefix: &str, bit: u32) -> NetId {
        n.by_name_id(&format!("{prefix}{bit}"))
            .or_else(|| n.by_name_id(&format!("{prefix}{bit}A")))
            .unwrap_or_else(|| panic!("the netlist has no {prefix}{bit}"))
    }

    /// The nets carrying a numbered bus, bit 0 first.
    ///
    /// Worth resolving once and keeping. Looking a bus up by name costs a
    /// `format!` and a hash lookup per bit, and anything watching the
    /// datapath reads several buses at every clock transition.
    pub fn bus_nets(&self, n: &Netlist, prefix: &str, bits: u32) -> Vec<NetId> {
        (0..bits).map(|b| self.bus_net(n, prefix, b)).collect()
    }

    /// Reads a bus off the nets, taking anything that is not a settled low
    /// as a one --- which is what a TTL input does with an undriven net.
    pub fn read(&self, nets: &[NetId]) -> u64 {
        nets.iter()
            .enumerate()
            .fold(0, |w, (b, &id)| w | (self.nets[id as usize].read().unwrap_or(true) as u64) << b)
    }

    /// The same, but `None` unless every bit is actually being driven.
    ///
    /// Half the datapath is tri-state buses that are undriven for part of
    /// the cycle. Reading those as zeroes --- or as ones --- invents a value
    /// that nothing on the board is asserting, and comparing it against
    /// another engine's is comparing against a fiction.
    pub fn read_driven(&self, nets: &[NetId]) -> Option<u64> {
        let mut w = 0;
        for (b, &id) in nets.iter().enumerate() {
            match self.nets[id as usize] {
                Level::High => w |= 1u64 << b,
                Level::Low => {}
                _ => return None,
            }
        }
        Some(w)
    }

    /// [`Chip::read`] by name, for a caller that reads a bus once.
    pub fn bus(&self, n: &Netlist, prefix: &str, bits: u32) -> u64 {
        self.read(&self.bus_nets(n, prefix, bits))
    }

    /// [`Chip::read_driven`] by name.
    pub fn bus_driven(&self, n: &Netlist, prefix: &str, bits: u32) -> Option<u64> {
        self.read_driven(&self.bus_nets(n, prefix, bits))
    }

    /// Records the settled board as what every part will see as `prev`.
    ///
    /// Taken at the end of [`Chip::transition`], once the registers have
    /// updated and their outputs propagated, so it is the last *fully*
    /// settled state. That is the value a register's inputs had when its
    /// setup time expired, which is what [`part::Update`] captures on an
    /// edge. A snapshot at the top of the next tick would already hold what
    /// the cables drove in between, and one in the middle of a tick is one
    /// settle stale.
    ///
    /// Kept as one copy of the nets rather than a pin array per part. The
    /// board has 1084 packages of [`part::MAX_PINS`] pins and 2782 nets, so
    /// the pin-array form wrote eleven times as much on every clock
    /// transition to say the same thing.
    fn snapshot(&mut self) {
        self.prev_nets.copy_from_slice(&self.nets);
    }

    /// Gives every part that remembers a chance to see an edge.
    ///
    /// Every part is asked, on every transition, and the ones that changed
    /// something have their gates marked for [`Chip::settle`] --- a register
    /// that has just latched drives something new, and no net it reads has
    /// to have moved for that to be true.
    ///
    /// Only parts a net has moved under are asked, which is 82% of the work
    /// gone: on a settled board almost nothing has an edge to see.
    ///
    /// The mark is what makes that safe, and **comparing the pins instead
    /// does not work**, which is worth recording because it looks obviously
    /// equivalent. It holds for an edge-triggered part, but not for a
    /// level-sensitive one: a 74S373 that is transparent follows its inputs
    /// whether or not they have just moved, and at power-on its state is
    /// zero while its pins already carry a value. The ALATCH at 3B02 comes
    /// up holding zero instead of `0x3f` and the machine never starts. A
    /// mark, unlike a comparison, survives until the update actually runs.
    fn update_state(&mut self) {
        let mut list = std::mem::take(&mut self.part_dirty_list);
        for &i in &list {
            let i = i as usize;
            if !self.part_dirty[i] {
                continue;
            }
            self.part_dirty[i] = false;
            let Some(update) = self.instances[i].behaviour.update else { continue };
            let mut now: Pins = [Level::Z; crate::part::MAX_PINS];
            let mut was: Pins = [Level::Z; crate::part::MAX_PINS];
            for (pin, slot) in self.instances[i].net_of.iter().enumerate() {
                if let Some(net) = slot {
                    now[pin] = self.nets[*net as usize];
                    was[pin] = self.prev_nets[*net as usize];
                }
            }
            update(&now, &was, &mut self.instances[i].state);
            self.generation += 1;
            self.soil_part(i);
        }
        // Nothing marks a part while they are asked; the allocation is
        // kept, and anything that did mark one is kept with it.
        list.clear();
        list.append(&mut self.part_dirty_list);
        self.part_dirty_list = list;
    }

    /// The net a gate drives, if it is wired.
    fn out_net(&self, id: GateId) -> Option<NetId> {
        let inst = &self.instances[id.part as usize];
        inst.net_of[inst.behaviour.gates[id.gate as usize].out as usize]
    }

    /// Recomputes one net from every gate driving it.
    ///
    /// `on_it` is the caller's scratch buffer, because this is the innermost
    /// loop of the engine and it runs a few thousand times per settle.
    fn resolve_net(&mut self, net: NetId, on_it: &mut Vec<Driver>) {
        // A rail is a rail: the I/O board's termination resistor from the
        // grant line to ground would otherwise pull ground up, taking the
        // address comparators' fixed inputs with it.
        if self.supplies[net as usize] != Level::Z {
            self.put_net(net, self.supplies[net as usize]);
            return;
        }
        on_it.clear();
        // The dirty driver is evaluated, and any other dirty one with it;
        // a clean one has not had an input move since it was last
        // evaluated, and its level is what it computed then. A driver
        // whose level moves is a change the outside can see whether or
        // not the net's level does, so the net is stamped for it.
        for k in 0..self.driver_info[net as usize].len() {
            let (f, drive) = self.driver_info[net as usize][k];
            let f = f as usize;
            let level = if self.evaluated[f] && !self.dirty[f] {
                self.gate_level[f]
            } else {
                let level = self.eval_gate(self.drivers[net as usize][k]);
                if self.gate_level[f] != level {
                    self.gate_level[f] = level;
                    self.generation += 1;
                    self.net_stamp[net as usize] = self.generation;
                }
                level
            };
            on_it.push(Driver { level, drive });
        }
        if let Some(d) = self.external[net as usize] {
            on_it.push(d);
        }
        let level = part::resolve(on_it);
        self.put_net(net, level);
    }

    /// Re-resolves a net whose driver from off the board has changed, and
    /// marks what reads it.
    fn resolve_from_outside(&mut self, net: NetId) {
        let mut on_it = std::mem::take(&mut self.scratch);
        self.resolve_net(net, &mut on_it);
        self.scratch = on_it;
        self.mark_readers(net);
    }

    /// Evaluates a gate from the nets as they stand.
    fn eval_gate(&self, id: GateId) -> Level {
        let inst = &self.instances[id.part as usize];
        let gate = &inst.behaviour.gates[id.gate as usize];
        // Only the pins this gate reads. A pin with no net is not wired
        // to anything, and a TTL input with nothing on it floats high.
        // OLORD2 1A20 is why that matters: it is the power-on reset, a
        // Schmitt inverter chain whose first input is the RC network,
        // and the resistor and capacitor are not parts in the drawing.
        let mut pins: Pins = [Level::Z; crate::part::MAX_PINS];
        for &pin in gate.ins {
            if let Some(n) = inst.net_of[pin as usize] {
                pins[pin as usize] = self.nets[n as usize];
            }
        }
        gate.eval(&pins, &inst.state)
    }

    /// How a gate's output pin drives.
    fn drive_of(&self, id: GateId) -> Drive {
        let inst = &self.instances[id.part as usize];
        inst.drive_of[inst.behaviour.gates[id.gate as usize].out as usize]
    }
}

/// The name a net ends in, split from the number: `MSK24` is bit 24 of the
/// `MSK` bus.
fn split_number(name: &str) -> (&str, Option<u32>) {
    let at = name.len() - name.bytes().rev().take_while(u8::is_ascii_digit).count();
    (&name[..at], name[at..].parse().ok())
}

/// Finds the delay lines of [`DELAY_LINES`] and their taps.
///
/// They are not [`Instance`]s: a delay line computes nothing, so
/// [`part::behaviour`] has nothing for it and [`Chip::new`] skips it. It is
/// still a part on the board, and these two are wired here from the netlist.
fn wire_delays(n: &Netlist, clocked: bool) -> Vec<Delay> {
    // Pins 12, 4, 10, 6 and 8 are one fifth to five fifths of the line. MIT
    // writes `TDnn` for Engineered Components' TTLDM-nn, whose sheet gives
    // "20% taps" and draws them in that order --- 12 at 20%, 4 at 40%, 10 at
    // 60%, 6 at 80%, 8 the whole line, with the input on 1 --- and lists
    // TTLDM-25, -50, -100 and -250, the four the CADR uses. MIT's own wire
    // lists label every tap to match: the TD100 at `icmem3` 1D12 reads
    // 12:20NS 4:40NS 10:60NS 6:80NS 8:100NS, and the TD250 at `cadr4` 1D22
    // reads 50, 100, 150, 200 and 250 on the same pins.
    const TAPS: [u8; 5] = [12, 4, 10, 6, 8];
    // The `NC` bodies on the Chaosnet pages are the same modules with the
    // same five taps. What they add is two wire-wrap posts, 3 and 5, that
    // the DIP itself has not got --- `cadrio/iob.wls` files them under
    // "BODY/DIP SOCKET MATCHING ERRORS" as `UN OR NC PIN`, and MIT's own
    // checker prints `NO DRIVE` on the nets hung from them. They are not
    // taps and nothing on the part drives them.
    //
    // The driver arrives by hand. `cadrio/iob.eco`'s consolidation of
    // 2/18/81 --- "then add 6 jumpers to set timing" --- straps a real tap
    // post to the dead post, and the drawing hangs the downstream net
    // there, which leaves the tap choice in the ECO where it can be changed
    // without redrawing. The posts are numbered three above the pins, so
    // MIT's `B4-8 : B4-11` is pin 8 to pin 5. Its four straps, with MIT's
    // own comments:
    //
    // | strap | pins | tap | net |
    // |---|---|---|---|
    // | `B4-8 : B4-11` | 8 to 5 | 100 ns | `SDLYD` |
    // | `B11-6 : B11-13` | 10 to 3 | 60 ns | `SDLYD2` |
    // | `A11-6 : A11-13` | 10 to 3 | 15 ns | `SAMPLE` |
    // | `A11-7 : A11-8` | 4 to 5 | 10 ns | `LOCKOUT END` |
    //
    // `GENCLK END` needs none: ECO 5 of 8/3/80 strapped it the same way and
    // the 11-AUG-81 wire list has it back in the drawing, on pin 10.
    const TAP_STRAPS: [(&str, &str, u8, u8); 4] = [
        ("LMDETC", "0B04", 5, 8),
        ("LMDETC", "0B11", 3, 10),
        ("LMDETC", "0A11", 3, 10),
        ("LMDETC", "0A11", 5, 4),
    ];
    let mut out = Vec::new();
    for pkg in n.packages() {
        // On the processor, [`DELAY_LINES`] is the two the clock generator
        // does not own. The bus interface has no clock generator and its
        // twelve delay sections are all wired: three TD100, two TD250, and
        // the seven sections of three MTD100s. The MTD100 is not a tapped
        // line at all: it is three independent 100 ns delays in one package,
        // pins 1 to 12, 3 to 10 and 5 to 8 --- `UB XBUS T0` in at 5 and `UB
        // XBUS T100` out at 8. MIT's own wire list names those pins IN1,
        // OUT1, IN2, OUT2, IN3 and OUT3, and its master parts list gives the
        // part as an Engineered Components MTTLDL-100, whose sheet is three
        // separately buffered lines on exactly those pins.
        if clocked && !DELAY_LINES.iter().any(|&(p, r)| pkg.page == p && pkg.reference == r) {
            continue;
        }
        if pkg.kind == "MTD100" {
            let on = |pin: u8| pkg.pins.iter().find(|&&(p, _)| p == pin).map(|&(_, net)| net);
            for (input, tap) in [(1, 12), (3, 10), (5, 8)] {
                if let (Some(input), Some(tap)) = (on(input), on(tap)) {
                    out.push(Delay {
                        input,
                        taps: vec![(tap, 100)],
                        invert: false,
                        last: None,
                        pending: Vec::new(),
                    });
                }
            }
            continue;
        }
        if !pkg.kind.starts_with("TD") {
            continue;
        }
        // `TD100NC` and its fellow are the same modules with two dead posts
        // added; the suffix is not part of the number.
        let name = pkg.kind.trim_start_matches("TD");
        let (name, nc) = match name.strip_suffix("NC") {
            Some(n) => (n, true),
            None => (name, false),
        };
        let ns: u32 = name
            .parse()
            .unwrap_or_else(|_| panic!("{} {} is not a delay line", pkg.page, pkg.reference));
        let on = |pin: u8| pkg.pins.iter().find(|&&(p, _)| p == pin).map(|&(_, net)| net);
        let input = on(1).unwrap_or_else(|| panic!("{} {} has no input", pkg.page, pkg.reference));
        let fifth = |pin: u8| TAPS.iter().position(|&p| p == pin).map(|k| ns * (k as u32 + 1) / 5);
        let mut taps: Vec<_> =
            TAPS.iter().filter_map(|&pin| Some((on(pin)?, fifth(pin)?))).collect();
        if nc {
            for (page, reference, dead, tap) in TAP_STRAPS {
                if pkg.page == page
                    && pkg.reference == reference
                    && let (Some(net), Some(at)) = (on(dead), fifth(tap))
                {
                    taps.push((net, at));
                }
            }
        }
        out.push(Delay { input, taps, invert: false, last: None, pending: Vec::new() });
    }
    if clocked {
        assert_eq!(out.len(), DELAY_LINES.len(), "a delay line is missing from the netlist");
    }
    for &(page, reference, out_pin, in_pin, ns, invert) in DELAYED_GATES {
        for pkg in n.packages().iter().filter(|pkg| pkg.page == page && pkg.reference == reference)
        {
            let on = |pin: u8| pkg.pins.iter().find(|&&(p, _)| p == pin).map(|&(_, net)| net);
            if let (Some(input), Some(tap)) = (on(in_pin), on(out_pin)) {
                out.push(Delay {
                    input,
                    taps: vec![(tap, ns)],
                    invert,
                    last: None,
                    pending: Vec::new(),
                });
            }
        }
    }
    out
}

/// Finds the oscillators: both VCOs of every 74LS124 and every DIP can.
///
/// The 74LS124 is a **dual** VCO --- section 1 enabled on pin 6 with its
/// output on 7, section 2 on 11 and 10 --- and a drawing may use one section
/// or both. It may also draw the two sections as **two bodies at one
/// location**, each repeating the supply pins, which is what `cadrdc/dctmot`
/// does at `0B04`; those do not merge into one package, because
/// [`Netlist::packages`] merges only records whose pins are disjoint. So
/// each section is wired from whichever record carries its pins, and a
/// record that carries neither is not an oscillator.
///
/// Taking pins 6 and 7 unconditionally is right for the bus interface ---
/// `reqtim` 0A01 is one body using section 1 alone --- and wrong for any
/// drawing that splits the package, which is why the section is found from
/// the pins the record actually carries.
fn wire_oscillators(n: &Netlist, clocked: bool) -> Vec<Oscillator> {
    if clocked {
        return Vec::new();
    }
    n.packages()
        .iter()
        .flat_map(|pkg| {
            let on = |pin: u8| pkg.pins.iter().find(|&&(p, _)| p == pin).map(|&(_, net)| net);
            let mut out = Vec::new();
            match pkg.kind.as_str() {
                "74LS124" => {
                    for (section, enable_pin, output_pin) in [(1u8, 6u8, 7u8), (2, 11, 10)] {
                        if let (Some(enable), Some(output)) = (on(enable_pin), on(output_pin)) {
                            out.push(Oscillator {
                                output,
                                enable: Some(enable),
                                period: vco_period(&pkg.page, &pkg.reference, section),
                                origin: Some(0),
                                edges: 0,
                                enabled_at: None,
                                next: None,
                            });
                        }
                    }
                }
                "DIPOSC" | "TTLOSC" => out.push(Oscillator {
                    output: on(8).unwrap_or_else(|| {
                        panic!("{} {} has nothing on pin 8", pkg.page, pkg.reference)
                    }),
                    enable: None,
                    period: dip_oscillator_period(&pkg.page, &pkg.reference),
                    origin: None,
                    edges: 0,
                    enabled_at: None,
                    next: None,
                }),
                _ => {}
            }
            out
        })
        .collect()
}

/// Finds the one-shots: both sections of every Am26S02 that are wired.
///
/// Section 1 is the left half of the package --- timing on 1 and 2, clear
/// on 3, `I1` on 4, `I0` on 5, `Q` on 6 and `-Q` on 7 --- and section 2
/// the mirror image, timing on 15 and 14, clear on 13, `I1` on 12, `I0` on
/// 11, `Q` on 10 and `-Q` on 9. A section whose trigger pins the drawing
/// leaves unconnected is not wired. The pulse width is per instance, from
/// the drawing.
fn wire_one_shots(n: &Netlist, clocked: bool) -> Vec<OneShot> {
    if clocked {
        return Vec::new();
    }
    n.packages()
        .iter()
        .filter(|pkg| pkg.kind == "26S02")
        .flat_map(|pkg| {
            let on = |pin: u8| pkg.pins.iter().find(|&&(p, _)| p == pin).map(|&(_, net)| net);
            let mut out = Vec::new();
            for (section, i0, i1, clear, q, not_q) in
                [(1u8, 5u8, 4u8, 3u8, 6u8, 7u8), (2, 11, 12, 13, 10, 9)]
            {
                let (Some(i0), Some(i1), Some(clear)) = (on(i0), on(i1), on(clear)) else {
                    continue;
                };
                if on(q).is_none() && on(not_q).is_none() {
                    continue;
                }
                let width_ns = one_shot_width(&pkg.page, &pkg.reference, section);
                out.push(OneShot {
                    i0,
                    i1,
                    clear,
                    // An unused output still needs a net to be driven on: an
                    // NC pin has one of its own.
                    q: on(q).unwrap_or_else(|| on(not_q).unwrap()),
                    not_q: on(not_q).unwrap_or_else(|| on(q).unwrap()),
                    width_ns,
                    fall: None,
                    last: None,
                });
            }
            out
        })
        .collect()
}

/// The pulse width of one section of an Am26S02, from the drawing's own
/// timing components through the sheet's t_pw = 0.30 Cx Rx (1 + 0.11/Rx),
/// Cx in picofarads and Rx in kilohms --- the same reading as
/// [`MEMCTL_REFRESH_NS`], and each one lands on MIT's own note beside the
/// part. `dctmot.drw`, which times the NXM acknowledge, carries `50 K` and
/// `1000 pF` under `;15 uS`: 15,033 ns. `dctrid.drw`, which clears the block
/// counter, carries `20K` and `330 pF` under `;2.0-2.5 USEC`: 1,991 ns, the
/// bottom of MIT's own range. The sheet gives that formula for Cx above
/// 1000 pF and a graph at or below, so on the two below it the last digits
/// are the formula's and MIT's note is the figure.
fn one_shot_width(page: &str, reference: &str, section: u8) -> u64 {
    match (page, reference, section) {
        ("MEMCTL", "0F02", 1) => MEMCTL_REFRESH_NS,
        ("DCTMOT", "0B09", 2) => DCTMOT_NXM_ACK_NS,
        ("DCTRID", "0B09", 1) => DCTRID_BLOCK_CLEAR_NS,
        // The DISK MULTIPLEXOR's eight, one a unit: `UNIT.<n>.SECTOR^` in
        // and `-UNIT.<n>.BC.CLR` out, which is the controller's `DCTRID`
        // 0B09 section 1 job done eight times over. `dmsect.drw` carries
        // `20K` and `330 pF` against every one of them, under the same two
        // notes as `dctrid.drw` --- `CLEARS BLOCK CTR` and `;2.0-2.5 USEC`
        // --- so they are the same components, the same formula and the
        // same width. Four Am26S02 packages, two sections each.
        ("DMSECT", "0C05" | "0C07" | "0B10" | "0B20", 1 | 2) => DCTRID_BLOCK_CLEAR_NS,
        // The Chaosnet half's two. `chaos/lispm/lmlndr.drw` carries their
        // components too, as discrete bodies in the sixteen-pin `DUMMY`
        // socket at A03@02: `cadrio/iob.wlr` wires that socket across the
        // package, pin n to pin 17-n, which puts a capacitor from `R.C` (14)
        // to `R.RC` (3) and a resistor from `R.RC` (4) to VCC for LMRCTL's
        // section, and the same shape on 12/5 and 6/11 for LMTBFC's. Its
        // eight bodies are two 470 and two 180 --- the receiver's two
        // differential pairs and the two `HI` rails --- and `6.8 K`, `4700`,
        // `82 pF`, `22 pF`, which are the four the one-shots take: no other
        // resistance on the page is within reach of the sheet's 5 kilohm
        // minimum Rx.
        //
        // **Which resistor pairs with which capacitor is settled**, and the
        // sentence that used to stand here was wrong twice. The drawing's
        // *list order* is grouped by type and does not pair them, but its
        // *drawn geometry* does: the eight bodies sit on the eight rows of
        // the DUMMY socket, each value on the row of the socket pair it
        // belongs to. And MIT's parts list does give the values ---
        // `cadrpt/parts.64`, Penny's list of 16 April 1980, draws every
        // dummy pin by pin and assigns them per board, `IOB dummies: 1 type
        // E, 1 type F, 1 type G, 1 type H`. Type E is the sixteen-pin one:
        //
        //     1:|180 ohm|16:      5:| 82 pF |12:
        //     2:|180 ohm|15:      6:|4.7Kohm|11:
        //     3:| 22 pF |14:      7:|470 ohm|10:
        //     4:|6.8Kohm|13:      8:|470 ohm|9:
        //
        // With the socket wiring above --- a capacitor from `R.C` (14) to
        // `R.RC` (3) and a resistor from `R.RC` (4) to VCC for LMRCTL's
        // section, the same shape on 12/5 and 6/11 for LMTBFC's --- LMRCTL
        // takes 22 pF and 6.8 kilohms and LMTBFC 82 pF and 4.7 kilohms.
        // Through the Am26S02 formula that is about 46 ns and 118 ns.
        //
        // **They are left at 100 all the same**, because nothing muir runs
        // turns on it: both drawings want an edge and not a width ---
        // LMRCTL A04 under "THIS PRE-DECREMENTS THE SILO POINTER WHICH THEN
        // POINTS TO THE FIRST BIT", LMTBFC A04 under "THIS MAKES IT WIN
        // WITH PDP11S" --- and 100 sits inside the band either pairing
        // gives. Changing them would be motion without a check behind it.
        ("LMRCTL", "0A04", 1) | ("LMTBFC", "0A04", 1) | ("LMTBFC", "0A04", 2) => 100,
        _ => panic!("no timing components known for one-shot {section} at {page} {reference}"),
    }
}

/// What each RAM's cells hold at power-on, so that **every memory reads
/// zero**.
///
/// Real RAM comes up undefined and a model has to choose; this one zeroes,
/// and says so. The subtlety is *what* is zeroed. For
/// most of the board a zero in the cell is a zero on the bus, but the two map
/// levels are wired active low --- VMEM0 stores `-VMA31` and reads it back as
/// `-VMAP4`, VMEM1 and VMEM2 store `-VMA23` and read `-VMO23` --- so a cell
/// full of zeros there reads as **all ones**.
///
/// Left that way the maps come up granting every access, and `chip` and
/// `rtl`, which zeroes the values rather than the cells, start from different
/// machines. That is not academic: they ran 535,836 microcycles before the
/// difference surfaced, on a `MAP[MD]` read of a page the microcode had not
/// written. So the cells of an active-low RAM are filled with ones, and
/// "comes up zero" means the same thing in both engines.
///
/// Which RAMs those are is read off the netlist, not tabulated: a chip whose
/// data output carries a net named `-something` stores the complement.
fn power_on_fill(instances: &[Instance], n: &Netlist) -> Vec<(usize, u8)> {
    let mut out = Vec::new();
    for (i, inst) in instances.iter().enumerate() {
        // The 82S21 holds two bits per cell, on pins 7 and 9; the others one,
        // on pin 7.
        let outs: &[(u8, u32)] = match part::strip(&inst.kind).0 {
            "93425" | "2147" => &[(7, 0)],
            "82S21" => &[(7, 0), (9, 1)],
            _ => continue,
        };
        let mut byte = 0u8;
        for &(pin, bit) in outs {
            let low = inst.net_of[pin as usize].is_some_and(|net| n.net(net).starts_with('-'));
            byte |= (low as u8) << bit;
        }
        if byte != 0 {
            out.push((i, byte));
        }
    }
    out
}

/// The contents of the nine 32x8 PROMs that are part of the machine rather
/// than of a program.
///
/// Eight on MSKG4 make the field mask and one on DSPCTL makes the dispatch
/// mask. MIT burned them once and never changed them, so they are as much
/// the machine as the wiring is, and [`Chip::power_on`] puts them back
/// rather than anything loading them from outside.
///
/// Which bits a chip holds and which table it answers from are read off the
/// netlist rather than tabulated, as in [`Chip::load_prom`]: the net on its
/// first output pin gives the bit its slice starts at, and the bus on its
/// address pins says which table. An address pin tied to ground fixes that
/// bit at zero, which is why DSPCTL 2F22 answers only its first eight words.
///
/// The three tables, at address `k`:
///
/// | bus | address | holds |
/// |---|---|---|
/// | `MSK` | `MSKL<4:0>` | bits `k:0` set |
/// | `MSK` | `MSKR<4:0>` | bits `31:k` set |
/// | `DMASK` | `IR<7:5>` | bits `k-1:0` set |
///
/// The `MSK` chips are open collector and the two banks share the bus, so
/// the mask is the AND of the two --- the field from bit `MSKR` to bit
/// `MSKL`.
///
/// **The formula is MIT's own table.** `mit/cadr/mskg4.drw` places the eight
/// 5600s at 2D11, 2D12, 2D16, 2D17, 2E11, 2E12, 2E16 and 2E17 and DSPCTL the
/// 5610 at 2F22, and no drawing lists their contents, nor did any dump of
/// the nine parts survive. What did survive is the table itself, printed in
/// MIT's own AI Memo 528, *CADR*, under "Output of mask memories", all
/// thirty-two entries of both masks: `mit/lmdoc/cadr.164`, line 495.
/// `the_mask_proms_hold_mits_own_table` in `tests/chip.rs` carries that
/// table and holds what this computes to it, word for word.
/// DSPCTL 2F22 is weaker, the manual giving its rule in prose --- the length
/// field masks all but the low `k` bits --- rather than an image; the words
/// follow from that rule and the test checks them the same way.
fn fixed_proms(instances: &[Instance], n: &Netlist) -> Vec<(usize, Vec<u8>)> {
    /// The 5600 and 5610's eight output pins, low bit first.
    const OUT: u8 = 1;
    /// Their five address pins, `A0` first.
    const ADDR: [u8; 5] = [10, 11, 12, 13, 14];

    let mut out = Vec::new();
    for (i, inst) in instances.iter().enumerate() {
        if !matches!(part::strip(&inst.kind).0, "5600" | "5610") {
            continue;
        }
        let name = |pin: u8| match inst.net_of[pin as usize] {
            Some(net) => n.net(net),
            None => panic!("{} has nothing on pin {pin}", inst.name()),
        };
        let (bus, base) = split_number(name(OUT));
        let base = base.unwrap_or_else(|| panic!("{} drives {}", inst.name(), name(OUT)));
        let table: fn(u32) -> u32 = match (bus, split_number(name(ADDR[0])).0) {
            ("DMASK", _) => |k| (1 << k) - 1,
            ("MSK", "MSKL") => |k| !0u32 >> (31 - k),
            ("MSK", "MSKR") => |k| !0u32 << k,
            _ => panic!("{} drives {} from {}", inst.name(), name(OUT), name(ADDR[0])),
        };
        let grounded = ADDR.iter().enumerate().fold(0u32, |m, (b, &pin)| match supply(name(pin)) {
            Some(Level::Low) => m | 1 << b,
            _ => m,
        });
        let words = part::memory_words(&inst.kind).expect("a PROM has words");
        let mut cells = vec![0u8; words];
        for (k, cell) in cells.iter_mut().enumerate() {
            if k as u32 & grounded == 0 {
                *cell = (table(k as u32) >> base) as u8;
            }
        }
        out.push((i, cells));
    }
    out
}

/// Puts a feedback group in an order that respects as many of its own
/// dependencies as it can.
///
/// Tarjan emits a component's members in no useful order, and sweeping them
/// that way computes gates from inputs that have not been updated yet. Those
/// stale reads are only transient --- another sweep would correct them ---
/// except that a transient can put two drivers on one bus at once, and a bus
/// conflict resolves to unknown. Unknown then propagates round the loop,
/// comes back as an unknown input, and never clears: it is absorbing, so a
/// moment of disorder becomes permanent.
///
/// So the members are sorted greedily: repeatedly take a gate all of whose
/// in-group feeders have already been taken, and when none qualifies --- which
/// is what being in a cycle means --- take whichever is waiting on fewest.
/// Only the genuine back-edges are then read stale, which is far fewer.
fn order_within(group: Vec<usize>, feeds: &[Vec<usize>]) -> Vec<usize> {
    if group.len() == 1 {
        return group;
    }
    let inside: HashMap<usize, usize> = group.iter().enumerate().map(|(i, &g)| (g, i)).collect();
    let mut waiting = vec![0usize; group.len()];
    let mut fed: Vec<Vec<usize>> = vec![Vec::new(); group.len()];
    for (i, &g) in group.iter().enumerate() {
        for &to in &feeds[g] {
            if let Some(&j) = inside.get(&to) {
                fed[i].push(j);
                waiting[j] += 1;
            }
        }
    }
    let mut taken = vec![false; group.len()];
    let mut out = Vec::with_capacity(group.len());
    for _ in 0..group.len() {
        let next = (0..group.len())
            .filter(|&i| !taken[i])
            .min_by_key(|&i| waiting[i])
            .expect("a group member is left");
        taken[next] = true;
        out.push(group[next]);
        for &j in &fed[next] {
            waiting[j] = waiting[j].saturating_sub(1);
        }
    }
    out
}

/// Tarjan's strongly-connected components, iterative.
///
/// `feeds[u]` lists the nodes `u` feeds. Components come back in reverse
/// topological order, so reversing them puts producers before consumers.
fn tarjan(feeds: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = feeds.len();
    let (mut index, mut low) = (vec![usize::MAX; n], vec![0usize; n]);
    let mut on_stack = vec![false; n];
    let (mut stack, mut out, mut counter) = (Vec::new(), Vec::new(), 0usize);
    for start in 0..n {
        if index[start] != usize::MAX {
            continue;
        }
        let mut work = vec![(start, 0usize)];
        while let Some((v, next)) = work.pop() {
            if next == 0 {
                index[v] = counter;
                low[v] = counter;
                counter += 1;
                stack.push(v);
                on_stack[v] = true;
            }
            let mut descended = false;
            for (i, &w) in feeds[v].iter().enumerate().skip(next) {
                if index[w] == usize::MAX {
                    work.push((v, i + 1));
                    work.push((w, 0));
                    descended = true;
                    break;
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            }
            if descended {
                continue;
            }
            if low[v] == index[v] {
                let mut group = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    group.push(w);
                    if w == v {
                        break;
                    }
                }
                out.push(group);
            }
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[v]);
            }
        }
    }
    out
}
