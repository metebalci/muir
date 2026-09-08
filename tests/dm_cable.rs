// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The cable between the disk controller and the DISK MULTIPLEXOR:
//! `src/dm.rs`.
//!
//! What is being checked here is not that wires reach --- a wire that does
//! not reach fails to resolve and the cable refuses to build. It is that
//! **the fan-out is the multiplexor's own gates**. A model that chose the
//! unit itself would boot and pass everything and not be MIT's, so the two
//! signals whose behaviour MIT wrote down are exercised through the cable
//! and asserted against what MIT says they do.
//!
//! Quoted rather than cited, this repository not holding `lmdoc/disk.22`:
//! Selected Unit Attention is "the attention signal directly from the
//! drive, it is not separately latched in the controller", and Any
//! Attention is "some unit has an attention, you have to select them one
//! after another to find out which".

use muir::disk_unit::{Geometry, Trident, Unit};
use muir::dm::Dm;
use muir::netlist::{self, Netlist};
use muir::part::Level;
use muir::xbus::Xbus;

mod support;

const CADRDC: &str = include_str!("../data/CADRDC.netlist");
const DM: &str = include_str!("../data/DM.netlist");

fn boards() -> (Netlist, Netlist) {
    (netlist::parse_with_multiplexor(CADRDC).unwrap(), netlist::parse(DM).unwrap())
}

fn net(n: &Netlist, name: &str) -> muir::netlist::NetId {
    n.by_name_id(name)
        .or_else(|| n.by_name_id(&format!("'{name}'")))
        .unwrap_or_else(|| panic!("no net {name}"))
}

/// Drives the eight drive ports' attention lines and the three unit-number
/// bits, settles the board, and gives back what the controller side shows
/// for the two attention signals. The lines are active low, so an
/// attention is a `Low` on `TRIDENT.<unit>.ATTENTION/`. The two signals
/// the controller sees carry no `/` and no leading minus and are active
/// high, so an attention raised on the cable is a `Low` at the drive and a
/// `High` at the controller: the board inverts, which is what the drive's
/// open-collector line and the receiver on this side amount to.
fn attentions(dm: &mut Dm, n: &Netlist, raised: &[u8], unit: u8) -> (Level, Level) {
    for u in 0..8u8 {
        let level = if raised.contains(&u) { Level::Low } else { Level::High };
        dm.board.drive(net(n, &format!("TRIDENT.{u}.ATTENTION/")), level);
    }
    // The unit number is not the controller's to assert on the cable: it
    // is the multiplexor's to latch and report back. The 74LS175 at 0F05
    // takes the disk address's `XBI28`, `XBI29` and `XBI30` --- bits
    // <30:28>, three bits for eight units --- on the rising edge of
    // `-LOAD DA`, and its Q outputs are `UNIT0`, `UNIT1` and `UNIT2`,
    // which go both to the decoder here and back to the controller on
    // `EP2`, `ER2` and `ES2`. Its `-CLR` is `POWER OK`.
    dm.board.drive(net(n, "'POWER OK'"), Level::High);
    for (b, x) in (28..31).enumerate() {
        let level = if unit >> b & 1 != 0 { Level::High } else { Level::Low };
        dm.board.drive(net(n, &format!("XBI{x}")), level);
    }
    // The 74LS175 takes its D inputs on a clock edge, so the load is an
    // edge and not a level: the board is transitioned across it.
    dm.board.drive(net(n, "'-LOAD DA'"), Level::Low);
    dm.board.settle_all();
    dm.board.transition(100);
    dm.board.drive(net(n, "'-LOAD DA'"), Level::High);
    dm.board.settle_all();
    dm.board.transition(200);
    // `-LOAD DA` is on the 25LS2538's `E4` as well as the register's
    // clock, and AMD's sheet calls `E4` an **active HIGH** enable: "A LOW
    // on either the E3 or E4 input forces all the decoded functions to be
    // inhibited". So one signal both takes the address and turns the
    // decode on, and the decode is live once the load is over --- the
    // opposite of what a leading minus suggests to a reader who takes the
    // name for the sense.
    //
    // Then the addressed drive has to answer `SELECTED/` before the
    // 25LS2521 comparator at 0F03 gives `SELECT OK` and the 74S00 at 0D04
    // enables that unit's buffer. So the drive answers here, as a drive
    // does.
    for u in 0..8u8 {
        let level = if u == unit { Level::Low } else { Level::High };
        dm.board.drive(net(n, &format!("TRIDENT.{u}.SELECTED/")), level);
    }
    dm.board.settle_all();
    (dm.board.net(net(n, "'SEL UNIT ATTENTION'")), dm.board.net(net(n, "'ANY ATTENTION'")))
}

/// **The cable resolves on both boards.** Twenty-five signals, every one
/// of them a net the controller and the multiplexor both have; three of
/// the controller's `DCEDGE` posts are not the multiplexor's and are named
/// in `muir::dm::wire_names`. Building the cable is the check --- a name
/// on neither board panics --- so this is the assertion that it built.
#[test]
fn the_cable_is_twenty_five_signals_both_boards_have() {
    let (dc, dm) = boards();
    let names = muir::dm::wire_names();
    assert_eq!(names.len(), 25, "signals on the cable");
    for name in &names {
        assert!(
            dc.by_name_id(name).or_else(|| dc.by_name_id(&format!("'{name}'"))).is_some(),
            "the controller has {name}"
        );
        assert!(
            dm.by_name_id(name).or_else(|| dm.by_name_id(&format!("'{name}'"))).is_some(),
            "the multiplexor has {name}"
        );
    }
    let _ = Dm::new(&dc, &dm, 0);
}

/// **Any Attention is the eight units ORed, and it is the board that ORs
/// them.** "Some unit has an attention, you have to select them one after
/// another to find out which": so it must be up for a raised attention on
/// any unit whatever the unit number says, and down only when none is
/// raised. Held for every single unit alone, for none, and for all eight.
///
/// Nothing in `src/dm.rs` knows what a unit is; this is the 74S133 and the
/// gates behind it on the multiplexor's own pages doing the work.
#[test]
fn any_attention_is_the_eight_units_ored() {
    let (dc, dmn) = boards();
    let mut dm = Dm::new(&dc, &dmn, 0);

    let (_, none) = attentions(&mut dm, &dmn, &[], 0);
    assert_eq!(none, Level::Low, "no unit has an attention");

    for u in 0..8u8 {
        // The unit number is left at 0 throughout: Any Attention is not
        // allowed to depend on which unit is selected.
        let (_, any) = attentions(&mut dm, &dmn, &[u], 0);
        assert_eq!(any, Level::High, "unit {u} alone raises Any Attention");
    }
    let (_, all) = attentions(&mut dm, &dmn, &[0, 1, 2, 3, 4, 5, 6, 7], 0);
    assert_eq!(all, Level::High, "all eight raise it");
}

/// **Selected Unit Attention is the selected drive's own line, and the
/// unit number is what selects it.**
///
/// **Ignored: the multiplexor cannot choose a unit yet.** The 25LS2538 at
/// DMSECT 0E05 is the decoder that turns `UNIT<2:0>` into `ADDRESS UNIT
/// <n>`, and `src/part.rs` has no pinout for it, so its outputs float,
/// the 25LS2521 comparator at 0F03 never gives `SELECT OK`, no `-UNIT <n>
/// ENB` is ever asserted and the tri-state bus below is never driven.
///
/// The bus floating is why this test is written as it is and why it is not
/// simply deleted until then. `SEL UNIT ATTENTION` is driven by four
/// 74S240 buffers onto one net; undriven it reads `High` through its
/// pull-up, which is the same level an asserted attention gives. An
/// earlier version of this test asserted `High` for the selected unit and
/// **passed with no buffer enabled at all**. That is the failure this
/// whole piece exists to avoid, and it is only visible by reading
/// `-UNIT <n> ENB` rather than the bus. "The attention signal directly from
/// the drive, it is not separately latched in the controller": so with one
/// unit's attention raised it must follow the unit number --- present when
/// that unit is selected and absent when another is.
///
/// This is the test that would fail if the fan-out were invented. A model
/// that passed the attention of whichever drive happened to be there, or
/// that ORed the eight into this signal too, gives the same answer for
/// every unit number; the board does not.
#[test]
fn selected_unit_attention_follows_the_unit_number() {
    let (dc, dmn) = boards();
    let mut dm = Dm::new(&dc, &dmn, 0);
    let mut seen = Vec::new();
    for raised in 0..8u8 {
        for unit in 0..8u8 {
            let (sel, _) = attentions(&mut dm, &dmn, &[raised], unit);
            seen.push((raised, unit, sel));
        }
    }
    for &(raised, unit, sel) in &seen {
        if raised == unit {
            assert_eq!(sel, Level::High, "unit {unit} selected, its own attention reaches");
        } else {
            assert_eq!(sel, Level::Low, "unit {unit} selected, unit {raised}'s does not");
        }
    }
}

/// **The unit number crosses the cable to the controller.** This is what
/// [`Dm`] is for and the rest of this file does not exercise it: the tests
/// above drive the multiplexor's own nets, where this one reads the answer
/// off the *controller* after the cable has carried it.
///
/// That direction is the one the cable uniquely provides. `EP2`, `ER2` and
/// `ES2` are inputs on the controller, and on a one-board machine
/// `disk.hand` ties them to ground because there is no multiplexor to
/// report which unit it chose; with one there, they are how it reports.
///
/// **The other direction is driven on the multiplexor's side here, and
/// deliberately.** `XBI28`, `XBI29`, `XBI30` and `-LOAD DA` are the
/// controller's own outputs --- the 74LS374 at 0F06 drives the three
/// address bits and seven 74LS193s carry the load --- so a test that put
/// levels on them at the controller would be fighting its registers rather
/// than using them. Making the controller produce them means running the
/// controller, which is what putting the multiplexor on the backplane is
/// for and belongs to the issues after this one.
///
/// Held for all eight, so a cable carrying one bit or none fails as loudly
/// as one carrying nothing.
#[test]
fn the_unit_number_crosses_the_cable_to_the_controller() {
    use muir::chip::Chip;
    let (dc, dmn) = boards();
    let mut controller = Chip::new_unclocked(&dc);
    controller.power_on();
    let mut dm = Dm::new(&dc, &dmn, 0);
    dm.board.drive(net(&dmn, "'POWER OK'"), Level::High);

    let mut t = 0u64;
    for unit in 0..8u8 {
        for u in 0..8u8 {
            let level = if u == unit { Level::Low } else { Level::High };
            dm.board.drive(net(&dmn, &format!("TRIDENT.{u}.SELECTED/")), level);
        }
        for (b, x) in (28..31).enumerate() {
            let level = if unit >> b & 1 != 0 { Level::High } else { Level::Low };
            dm.board.drive(net(&dmn, &format!("XBI{x}")), level);
        }
        for load in [Level::Low, Level::High] {
            dm.board.drive(net(&dmn, "'-LOAD DA'"), load);
            dm.board.settle_all();
            t += 100;
            dm.board.transition(t);
        }
        dm.exchange(&mut controller);
        let at_controller: u8 = (0..3)
            .map(|b| ((controller.net(net(&dc, &format!("UNIT{b}"))) == Level::High) as u8) << b)
            .sum();
        assert_eq!(at_controller, unit, "the controller reads unit {unit} off the cable");
    }
}

/// **A drive on one of the multiplexor's eight ports answers the select,
/// and its cable spans two boards to do it.**
///
/// This is what [`muir::disk_unit::OnCable::on`] is for. The drive's
/// per-unit lines are its port on the multiplexor --- `TRIDENT.3.SELECT/`
/// and the rest --- and its shared lines are still the controller's, so
/// its `NetId`s come from two netlists and it is applied to two `Chip`s.
/// A cable that took both halves from one board would index the wrong one
/// without saying so.
///
/// The round trip, all of it the multiplexor's own gates. The board is
/// given unit 3 and decodes it to `ADDRESS UNIT 3`; a 75452 on page
/// `DMIO` puts `TRIDENT.3.SELECT/` down; the drive sees the select and
/// answers `SELECTED/`; the 25LS2521 at 0F03 on page `DMSEL` compares the
/// eight `ADDRESS UNIT n` against the eight `UNIT n SELECTED` and raises
/// `SELECT OK`; and an S00L gate at 0D04 takes that with `UNIT 3
/// SELECTED` to pull `-UNIT 3 ENB` down, which is what enables that
/// port's buffers on to the shared lines.
///
/// **Only the comparator is shared; the rest of that chain is unit 3's
/// own**, and the parts are spread per unit rather than being one driver
/// each --- so nothing above generalises to another unit's path. Nor does
/// a designator name one package here: the multiplexor reuses them, 19 of
/// its 79 across records whose pins collide, and `0B05` alone covers the
/// select drivers of units 0 to 3. The 75452 gate meant is the one whose
/// input is `ADDRESS UNIT 3` and whose output is `TRIDENT.3.SELECT/`.
/// MIT left no wire list of this board (`tools/dm-netlist.sh`), so there
/// is nothing that could settle the designators.
///
/// Nothing here tells the drive which unit it is: it is on port 3 and the
/// board chooses port 3.
#[test]
fn a_drive_on_a_multiplexor_port_answers_the_select() {
    use muir::chip::Chip;
    use muir::disk_unit::{OnCable, Ports};

    const UNIT: u8 = 3;
    let (dc, dmn) = boards();
    let mut controller = Chip::new_unclocked(&dc);
    controller.power_on();
    let mut dm = Dm::new(&dc, &dmn, 0);
    dm.board.drive(net(&dmn, "'POWER OK'"), Level::High);
    let mut cable = OnCable::on(
        Ports { per_unit: &dmn, unit: UNIT, shared: &dc },
        Trident::new(Unit::blank(Geometry::T300), 0),
    );

    // The multiplexor is told which unit, the way the controller tells it.
    for (b, x) in (28..31).enumerate() {
        let level = if UNIT >> b & 1 != 0 { Level::High } else { Level::Low };
        dm.board.drive(net(&dmn, &format!("XBI{x}")), level);
    }
    let mut t = 0u64;
    for load in [Level::Low, Level::High] {
        dm.board.drive(net(&dmn, "'-LOAD DA'"), load);
        dm.board.settle_all();
        t += 100;
        dm.board.transition(t);
    }
    assert_eq!(
        dm.board.net(net(&dmn, &format!("'ADDRESS UNIT {UNIT}'"))),
        Level::High,
        "the multiplexor decoded unit {UNIT}"
    );
    assert_eq!(
        dm.board.net(net(&dmn, &format!("TRIDENT.{UNIT}.SELECT/"))),
        Level::Low,
        "and put the select down on that port"
    );

    // The drive answers, over a cable whose two halves are two boards.
    for _ in 0..8 {
        t += 100;
        cable.apply_on(&mut dm.board, &mut controller, t);
        dm.exchange(&mut controller);
    }
    assert_eq!(
        dm.board.net(net(&dmn, &format!("TRIDENT.{UNIT}.SELECTED/"))),
        Level::Low,
        "the drive answered the select"
    );
    assert_eq!(
        dm.board.net(net(&dmn, &format!("'-UNIT {UNIT} ENB'"))),
        Level::Low,
        "so the comparator agrees and that port's buffers are enabled"
    );
    for other in (0..8u8).filter(|&u| u != UNIT) {
        assert_eq!(
            dm.board.net(net(&dmn, &format!("'-UNIT {other} ENB'"))),
            Level::High,
            "and no other port is"
        );
    }
}

// --- the multiplexor on the backplane ---------------------------------------

const BUSINT: &str = include_str!("../data/BUSINT.netlist");
const CADRM: &str = include_str!("../data/CADRM.netlist");

/// A backplane whose only board is the disk controller, with a DISK
/// MULTIPLEXOR on the controller's cable if `multiplexor` and a drive on
/// each of `units`. No memory boards: nothing here reads or writes one.
///
/// A drive's spindle is started at a point of the revolution that its unit
/// number picks --- 137 us per unit, which is no multiple of the 980 us
/// between sector pulses --- so that eight of them sound different from
/// one, and so that a unit sounds the same whichever others are fitted
/// beside it.
fn backplane(multiplexor: bool, units: &[u8]) -> (Netlist, Xbus) {
    let busint = netlist::parse(BUSINT).unwrap();
    let memory = netlist::parse(CADRM).unwrap();
    // With a multiplexor the controller's six one-board jumpers come off,
    // which is the whole difference between the two netlists.
    let dc = if multiplexor {
        netlist::parse_with_multiplexor(CADRDC).unwrap()
    } else {
        netlist::parse(CADRDC).unwrap()
    };
    let mut xbus = Xbus::new(&busint, &memory, 0, &[&dc], 0);
    if multiplexor {
        xbus.plug_multiplexor(&netlist::parse(DM).unwrap(), 0);
    }
    for &u in units {
        let spun = u64::from(u) * 137_000;
        xbus.plug_unit(u, Trident::new(Unit::blank(Geometry::T300), spun), 0);
    }
    (dc, xbus)
}

/// Runs such a backplane for `span` nanoseconds and counts how often
/// `BLOCK.CLK^` and `-UNIT.0.SECTOR^` move at the controller.
///
/// The step is the next event on any board, capped at a microsecond: the
/// nets are read at the step, so a pulse shorter than one could otherwise
/// pass between two reads. A sector pulse is
/// [`muir::disk_unit::SECTOR_PULSE_NS`], 1,240 ns, and the index pulse is
/// longer, so neither can.
fn spindles(multiplexor: bool, units: &[u8], span: u64) -> (usize, usize) {
    let (dc, mut xbus) = backplane(multiplexor, units);
    let block = net(&dc, "BLOCK.CLK^");
    let sector = net(&dc, "-UNIT.0.SECTOR^");
    let mut moves = [0, 0];
    let mut was = [xbus.devices[0].net(block), xbus.devices[0].net(sector)];
    let mut t = 0;
    while t < span {
        t = match xbus.next_tap() {
            Some(d) if d > t => d.min(t + 1_000),
            _ => t + 1_000,
        };
        xbus.transition_due(t);
        for (k, n) in [block, sector].into_iter().enumerate() {
            let now = xbus.devices[0].net(n);
            if now != was[k] {
                moves[k] += 1;
                was[k] = now;
            }
        }
    }
    (moves[0], moves[1])
}

/// **A drive heard through the multiplexor sounds exactly as it does on
/// the controller's own port, and a drive on any other port is silent.**
///
/// The sector pulse is the thing to listen to, because nothing has to ask
/// for it: a T-300's spindle turns from power-on and the drive puts a
/// pulse on `COMPSECIDX/` seventeen times a revolution whether or not
/// anything has selected it. It reaches the controller as `BLOCK.CLK^` ---
/// over the cable from the multiplexor when one is fitted, and off the
/// controller's own `-UNIT.0.SECTOR^` when not.
///
/// Which unit the board addresses out of reset is not this test's choice.
/// The 74LS175 at 0F05 is cleared by `POWER OK` and nothing has loaded a
/// disk address, so it holds zero and unit 0 is addressed; a drive in unit
/// 3 is inaudible, and that is the multiplexor's doing and not the
/// cable's.
#[test]
fn the_multiplexor_carries_the_addressed_units_spindle() {
    const MS: u64 = 1_000_000;
    let (alone, port) = spindles(false, &[0], MS);
    assert!(alone > 0, "the drive's spindle turns and the controller hears it");
    assert_eq!(alone, port, "on its own port, which is where a lone drive sits");

    let (through, port) = spindles(true, &[0], MS);
    assert_eq!(through, alone, "the multiplexor carries unit 0's spindle unchanged");
    assert_eq!(port, 0, "and the controller's own port is not the drive's any more");

    assert_eq!(spindles(true, &[3], MS), (0, 0), "a drive in unit 3 is not addressed");
}

/// **The multiplexor selects a port; it does not merge the eight.**
///
/// Eight drives started at different points of a revolution would, wired
/// together, give the controller eight times the pulses. They give it what
/// unit 0 alone gives, because unit 0 is what the board addresses; take
/// unit 0 away and leave the other seven turning, and the controller hears
/// nothing at all.
///
/// This is the check that the fan-in is the multiplexor's own gates, the
/// same argument `selected_unit_attention_follows_the_unit_number` makes
/// for the attention lines --- made here on the backplane and through
/// [`muir::xbus::Xbus`], which is what a machine has.
#[test]
fn the_multiplexor_selects_a_port_rather_than_merging_them() {
    const MS: u64 = 1_000_000;
    const ALL: [u8; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
    let one = spindles(true, &[0], MS);
    assert!(one.0 > 0, "unit 0 is heard");
    assert_eq!(spindles(true, &ALL, MS), one, "eight drives sound like the one addressed");
    assert_eq!(spindles(true, &ALL[1..], MS), (0, 0), "and seven with no unit 0 sound like none");
}

/// **The shared half of the cable is every drive's, addressed or not.**
///
/// Eighteen of a Trident's lines never pass through the multiplexor: the
/// three tags, the ten bus lines and the five status lines reach every
/// drive on the B-cable and are wired together there. So a drive in unit
/// 3, which the board is not addressing, still holds `TRIDENT.READY/` and
/// `TRIDENT.ON.LINE/` down at the controller. That is the cable and not a
/// selection, and it is why the controller has to select a unit before it
/// can believe what the status lines tell it.
#[test]
fn the_status_lines_are_shared_by_every_drive_on_the_cable() {
    let ready = |units: &[u8]| {
        let (dc, mut xbus) = backplane(true, units);
        xbus.transition_due(10_000);
        let low = |name: &str| xbus.devices[0].net(net(&dc, name)) == Level::Low;
        (low("TRIDENT.READY/"), low("TRIDENT.ON.LINE/"))
    };
    assert_eq!(ready(&[]), (false, false), "no drive holds the status lines down");
    assert_eq!(ready(&[0]), (true, true), "a drive in unit 0 does");
    assert_eq!(ready(&[3]), (true, true), "and so does one in unit 3, which is not addressed");
}

/// **Every drive on the multiplexor turns, not only the addressed one.**
///
/// A Trident's spindle does not wait to be selected, so all eight ports
/// carry sector pulses and choosing between them is the board's business.
/// The pulses are counted on the port itself --- the multiplexor's
/// `TRIDENT.<n>.COMPSECIDX/`, where the controller cannot hear them ---
/// because that is the only place the difference shows.
///
/// Without this the two spindle tests above pass on a machine that steps
/// unit 0 and no other: unit 0 is what the board addresses out of reset,
/// so a drive nobody stepped looks exactly like a drive nobody asked for.
#[test]
fn every_drive_on_the_multiplexor_turns() {
    let dmn = netlist::parse(DM).unwrap();
    let pulses = |units: &[u8], port: u8| {
        let (_, mut xbus) = backplane(true, units);
        let sector = net(&dmn, &format!("TRIDENT.{port}.COMPSECIDX/"));
        let board = |x: &Xbus| x.multiplexor().expect("a multiplexor is fitted").board.net(sector);
        let (mut moves, mut was) = (0, board(&xbus));
        let mut t = 0;
        while t < 1_000_000 {
            t += 1_000;
            xbus.transition_due(t);
            let now = board(&xbus);
            if now != was {
                moves += 1;
                was = now;
            }
        }
        moves
    };
    let alone = pulses(&[7], 7);
    assert!(alone > 0, "the drive in unit 7 turns, addressed or not");
    assert_eq!(pulses(&[0, 7], 7), alone, "and goes on turning beside unit 0");
    assert_eq!(pulses(&[0], 7), 0, "an empty port has nothing on it");
}
