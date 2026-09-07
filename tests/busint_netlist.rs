// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `data/BUSINT.netlist`, and the two things that say it is the board.
//!
//! The bus interface is its own card --- MIT's `cadr1/busint.prt` heads
//! itself "LISPM Bus Interface", board type `LG684` against the processor's
//! `MPG216` --- and `tools/busint-netlist.sh` extracts it from MIT's 35 SUDS
//! drawings with `tools/soap4`, which emits the netlist format
//! `src/netlist.rs` reads.
//!
//! Two cross-checks, by routes that are not the drawings:
//!
//! 1. **MIT's own census.** `cadr1/busint.wls` counts, for every body name on
//!    the board, how many sections of it there are. That file is not the
//!    drawings; it is what MIT's own tooling made of them in 1980, and it is
//!    what [`the_census_matches_mits_own`] compares against.
//! 2. **MIT's own wire list.** `cadr1/busint.wlr` is the list the board was
//!    wrapped from, and [`matches_mits_wire_list`] holds every net of the
//!    netlist to it pin by pin.

use std::collections::{BTreeMap, BTreeSet};

use muir::chip::Chip;
use muir::netlist::{self, Netlist};
use muir::wirelist;

mod support;
use support::mit_text;

const BUSINT: &str = include_str!("../data/BUSINT.netlist");
const REQTIM: &str = include_str!("../mit/cadr1/reqtim.prom");
const UPRIOR: &str = include_str!("../mit/cadr1/uprior.prom");

fn busint() -> Netlist {
    netlist::parse(BUSINT).unwrap()
}

/// The shape of the board, pinned the way `tests/netlist.rs` pins the
/// processor's.
#[test]
fn parses_to_the_expected_shape() {
    let n = busint();
    assert_eq!(n.pages.len(), 35, "page markers: busint.book names 35 plot files");
    assert_eq!(n.parts.len(), 313, "parts, which are gates and not packages");
    let kinds: BTreeSet<&str> = n.parts.iter().map(|p| p.kind.as_str()).collect();
    assert_eq!(kinds.len(), 67, "distinct body names");
    assert!(n.parts.iter().all(|p| !p.pins.is_empty()), "every part has pins");
    eprintln!("{} parts, {} nets, {} pages", n.parts.len(), n.nets.len(), n.pages.len());
}

/// **Every section MIT counted is in the netlist.**
///
/// `busint.wls` is MIT's own summary of the board by DIP type: for each type,
/// the body names drawn with it and how many sections of each. Two entries
/// are expected not to match, and both are explained rather than tolerated:
///
/// - `BYPASS`, the six bypass capacitors, which `tools/soap4` emits no part
///   for. They are capacitors; there is nothing to simulate.
/// - `74S04A`, which MIT counts as six sections and the drawing carries as
///   **one body**: the whole 14-pin package, with all six inverters on it.
///   The test checks that body has the six input/output pairs, so nothing
///   is missing --- only the granularity differs, a `part` record being a
///   gate and this one a package.
#[test]
fn the_census_matches_mits_own() {
    let text = mit_text(&["cadr1", "busint.wls"]);

    let census = support::body_census(&text);
    assert!(census.len() > 50, "parsed {} bodies out of busint.wls", census.len());

    let n = busint();
    let mut ours: BTreeMap<String, usize> = BTreeMap::new();
    for p in &n.parts {
        *ours.entry(p.kind.clone()).or_default() += 1;
    }

    let mut wrong = Vec::new();
    for (body, want) in &census {
        let got = ours.get(body).copied().unwrap_or(0);
        match body.as_str() {
            "BYPASS" => assert_eq!(got, 0, "bypass capacitors are not parts"),
            // One body, six sections; the pairs are checked below.
            "74S04A" => assert_eq!(got, 1, "the hex inverter is drawn as one body"),
            _ if got != *want => wrong.push(format!("{body}: MIT {want}, ours {got}")),
            _ => {}
        }
    }
    for body in ours.keys() {
        if !census.contains_key(body) {
            wrong.push(format!("{body}: not in MIT's census at all"));
        }
    }
    assert!(wrong.is_empty(), "sections disagree with busint.wls: {wrong:?}");

    let sections: usize = census.values().sum();
    eprintln!(
        "busint.wls counts {sections} sections in {} bodies; the netlist has {} parts",
        census.len(),
        n.parts.len()
    );

    // The one body that carries six sections: a 74S04 hex inverter, drawn
    // whole. Its six pairs are the datasheet's.
    let hex = n.parts.iter().find(|p| p.kind == "74S04A").expect("the 74S04A at UBA 0D11");
    let pins: BTreeSet<u8> = hex.pins.iter().map(|&(p, _)| p).collect();
    for (a, y) in [(1, 2), (3, 4), (5, 6), (9, 8), (11, 10), (13, 12)] {
        assert!(pins.contains(&a) && pins.contains(&y), "inverter {a}/{y}");
    }
}

// --- the same structural checks tests/part.rs makes of the processor ------

/// Every part on the board is identified, and everything that computes
/// anything has a behaviour.
///
/// The exceptions are the analog parts, which is the same exception the
/// processor board has: twelve delay lines and the `74LS124`
/// voltage-controlled oscillator that clocks the timeout counter. A delay
/// line cannot be levelized, so `src/clock.rs` models the processor's and
/// these are the bus interface's to model the same way.
#[test]
fn every_part_is_identified() {
    let k = support::kinds(&busint());
    assert!(k.unknown.is_empty(), "no pinout for {:?}", k.unknown);
    assert_eq!(
        k.silent,
        ["74LS124", "MTD100", "TD100", "TD250"],
        "only the delay lines and the oscillator may lack a behaviour"
    );
}

/// A pinout must not claim an output on its own supply pin, and no part may
/// use one. Two of the bus interface's parts have their ground somewhere
/// other than the middle of the package --- the DM8838's is pin 7 and the
/// Am26S10 has two --- so those are declared with no package and passed
/// over, which is what `Pinout::package` being zero means.
#[test]
fn no_part_touches_its_own_supply_pins() {
    let n = busint();
    support::no_pinout_drives_its_own_supply_pin(&n);
    support::no_part_touches_its_own_supply_pins(&n);
}

/// **The strong one.** Two totem-pole outputs on one net is an electrical
/// fault, so a wrongly claimed output pin surfaces here. It is the check
/// that caught a `74S472` entry on the processor board claiming an output on
/// its ground pin.
#[test]
fn no_net_has_two_push_pull_drivers() {
    support::no_net_has_two_push_pull_drivers(&busint());
}

/// What arrives from off the board. The bus interface plugs into three
/// things --- the five cables to the cpu, the Xbus and the Unibus --- so
/// this list is the boundary the rest of the machine has to drive.
#[test]
#[ignore = "a report, not a check: --ignored --nocapture"]
fn report_what_arrives_from_off_the_board() {
    let n = busint();
    support::report_undriven_nets(&n);

    // And what the wire list says plugs in: every wire with a connector pin,
    // by the drawing the connector is on --- CLM is the five cables to the
    // processor, CUBUS the Unibus, CXBUS the Xbus, CTP the test points.
    let text = mit_text(&["cadr1", "busint.wlr"]);
    let mut by_page: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let signals = wirelist::parse(&text, &n.pages);
    for s in signals.iter().filter(|s| !s.is_pseudo()) {
        for page in s.pins.iter().filter(|p| p.body == "CON").map(|p| p.page.as_str()) {
            let v = by_page.entry(page).or_default();
            if !v.contains(&s.name()) {
                v.push(s.name());
            }
        }
    }
    for (page, names) in &by_page {
        eprintln!("{page}: {} wires reach a connector: {names:?}", names.len());
    }
}

/// **The board builds and levelizes**, and with its supplies on it half
/// comes up on its own.
///
/// [`Chip::new_unclocked`] is the bus interface's constructor: it has no
/// clock generator of its own, because "the master clock ... is supplied by
/// the cpu to the bus interface". Everything else is the processor's
/// machinery unchanged --- the same parts, the same levelizer, the same
/// four-valued net resolution.
///
/// What is still unknown after settling is what **the boundary** decides:
/// this card plugs into three things --- the five cables, the Xbus and the
/// Unibus --- and with none of them connected the address lines, the
/// strobes and every handshake sit at `X` and take their logic with them.
/// [`the_board_settles_with_its_boundary_driven`] connects them.
#[test]
fn the_board_builds_and_settles() {
    let n = busint();
    let mut c = Chip::new_unclocked(&n);
    c.power_on();
    c.settle_all();

    let groups = c.feedback();
    let looped: usize = groups.iter().map(|g| g.len()).sum();
    eprintln!("{} gates ordered, {} in {} feedback groups", c.ordered(), looped, groups.len());
    // The loops are the transceivers: each Am8304 is sixteen gates in a
    // cycle, because a bidirectional pin depends on the pin across from it
    // whichever way the direction pin is pointing. The processor board has
    // one loop and it is the console bus; this board has one per transceiver.
    assert_eq!(c.ordered(), 1423, "gates the levelizer ordered");
    assert_eq!((looped, groups.len()), (363, 37), "gates left in feedback groups");

    let settled = (0..n.nets.len() as u32).filter(|&id| c.read_driven(&[id]).is_some()).count();
    eprintln!("{settled} of {} nets at a level from cold", n.nets.len());
    assert!(settled > 0, "the supplies at least must come up");
    // 28 before the board was powered on at all --- nothing had called
    // `power_on`, so even the grounds read unknown --- and 29 once the
    // netlist was reconciled with MIT's wire list.
    assert_eq!(settled, 554, "how much of the board stands on its own supplies");
}

/// **Which revision of the drawings this is.**
///
/// MIT's `cadr1/busint.eco` lists the board's engineering changes under two
/// headings, "[WIRE LIST OF 3/4/79]" and "[WIRE LIST of 12/80]": changes 1
/// to 3 were made after the first list and are in the second. So there are
/// two revisions of these drawings, and a netlist made from the earlier
/// would be wrong silently --- it parses, and gives a plausible board.
/// `mit/cadr1` holds the later: `busint.wlr` is dated 11-DEC-80 and dates
/// thirty of the thirty-three drawings it lists 10-DEC-80. The earlier set
/// is not committed. [`the_census_matches_mits_own`] already guards against
/// it, the census being the December 1980 one; this names the revision by
/// the wire each of two changes adds. The changes give socket pins, which
/// `busint.wlr` writes beside the logical pin --- `A05-09(12)` is logical
/// pin 9 in socket pin 12.
///
/// > 2.  6/15/79  Moon   Unibus interrupt logic can get hung when using a
/// >     DL-11 [...]
/// >
/// >     INTR SSYN                      D15-14 : D14-14
/// >
/// > 3.  7/17/79  Moon   Unibus grant logic can get hung if an interrupt
/// >     request comes in while the processor is doing a unibus read [...]
/// >
/// >     -LMUB GRANT                    A6-8 : A5-12
#[test]
fn the_drawings_are_the_later_revision() {
    /// The pins on a net, as location and logical pin.
    fn pins_on<'a>(n: &'a Netlist, name: &str) -> Vec<(&'a str, u8)> {
        let Some(id) = n.by_name_id(name) else { return Vec::new() };
        n.parts
            .iter()
            .flat_map(|p| {
                p.pins
                    .iter()
                    .filter(move |&&(_, net)| net == id)
                    .map(move |&(pin, _)| (p.reference.as_str(), pin))
            })
            .collect()
    }
    let n = busint();
    // Change 2's wire joins the 74LS74s at D15 and D14 by socket pin 14 on
    // each, logical pin 11: `D15-11(14)` and `D14-11(14)` in the list.
    let ssyn = pins_on(&n, "'INTR SSYN'");
    assert!(
        ssyn.contains(&("0D15", 11)) && ssyn.contains(&("0D14", 11)),
        "the wire change 2 adds to INTR SSYN: {ssyn:?}"
    );
    assert_eq!(ssyn.len(), 4, "pins on INTR SSYN");
    // Change 3's wire joins the 74S175 at A6, socket pin 8 and logical 6,
    // to the 74S02 at A5, socket pin 12 and logical 9: `A06-06(08)` and
    // `A05-09(12)`.
    let grant = pins_on(&n, "'-LMUB GRANT'");
    assert!(
        grant.contains(&("0A06", 6)) && grant.contains(&("0A05", 9)),
        "the wire change 3 adds to -LMUB GRANT: {grant:?}"
    );
    assert_eq!(grant.len(), 3, "pins on -LMUB GRANT");

    // Two parts MIT's December 1980 census, `busint.wls`, settles too: no
    // `9S42`, and eight `74S51A` sections.
    //
    // **The earlier board is settled as well, and its wire list of 3/4/79
    // is committed**: it is `mit/cadr1/busint.ray`, which `busint.eco`
    // heads `[WIRE LIST OF 3/4/79]` and whose wires are the ones that file's
    // ECOs 1, 2, 3 and 6 delete, none of the ones they add. Its socket
    // jumpers place 16-pin bodies at C06 and C14 where the December 1980
    // list has 14-pin 74S51As, which is the package-size change.
    //
    // MIT's own parts list agrees and gives the counts outright:
    // `cadrpt/parts.64` of 16 April 1980, between the two revisions, has
    // `9S42 = 2` and `74S51 = 3` for this board against the later list's
    // five 74S51 packages. So the earlier board carried **two** 9S42
    // packages, at C06 and C14, and three 74S51s. The "four Fairchild
    // 9S42s" of the older note here is a count of four drawn gates in those
    // two packages, which is consistent and reads as four parts.
    let count = |kind: &str| n.parts.iter().filter(|p| p.kind == kind).count();
    assert_eq!(count("9S42-1"), 0, "no 9S42 on the board");
    assert_eq!(count("74S51A"), 8, "eight 74S51A sections");
}

/// **The netlist against MIT's own wire list**, pin by pin.
///
/// `cadr1/busint.wlr` is what MIT's tooling made of the same drawings, the
/// list the board was wrapped from, and it is the second route that says
/// which pins share a wire. `soap4` gave two names to a wire the drawings
/// label twice, `LMRD` and `-LMWR`, and to one spelt with and without a
/// space, `-DBUB GRANTED` and `-DB UB GRANTED`, and lost six pins to `@,p0`;
/// `tools/busint-netlist.sh` reconciles the file with the list, and this
/// says the result is the list.
#[test]
fn matches_mits_wire_list() {
    let n = netlist::parse_wired(BUSINT).unwrap();
    let mut signals = support::wire_list(&n, &["cadr1", "busint.wlr"]);
    // The list was made from the December 1980 drawing and has E09 pin 2 on
    // `LMX GRANT A` as that drawing does; the netlist is built with the pin
    // on `UBX GRANT A`, the October 1978 revision's, by
    // `examples/reconcile.rs` (discrepancy 68). The list's one
    // pin is moved the same way, and every other pin is held to the list.
    let from = signals.iter().position(|s| s.name() == "LMX GRANT A").unwrap();
    let to = signals.iter().position(|s| s.name() == "UBX GRANT A").unwrap();
    let e09_2 = signals[from]
        .pins
        .iter()
        .position(|p| p.page == "REQLM" && p.slot() == "E09" && p.number == 2)
        .expect("the wire list has E09-02 on LMX GRANT A, as the 1980 drawing does");
    let pin = signals[from].pins.remove(e09_2);
    signals[to].pins.push(pin);
    let r = wirelist::compare(&n, &signals, |slot| format!("0{slot}"));
    support::report(&signals, &r);
    assert!(r.placed > 2000, "the wire list was read");
    assert!(
        r.missing.is_empty(),
        "the wire list places pins the netlist has not got: {:?}",
        r.missing
    );
    assert!(r.split.is_empty() && r.merged.is_empty(), "the wire list disagrees with the netlist");
}

/// The board powered, with its boundary driven as the cpu and the bus
/// terminators leave it between cycles, reset, and given two microseconds
/// of master clock. The wire list names what plugs in, connector by
/// connector; `busint.erface` gives the cables' idle levels: `-MEMRQ` high,
/// `WRCYC` low, the address at rest. Every Unibus and Xbus wire is pulled up
/// as its terminator pulls it --- the grant chains among them, which the
/// board drives itself in local mode --- and the other machine's debug
/// cables pulled up too.
///
/// Returns the board and the time on it. The master clock is `-MCLK7`,
/// active on its rising edge, and [`Board::clock`] steps it.
struct Board<'a> {
    n: &'a Netlist,
    c: Chip,
    now: u64,
    /// Half the master clock's period: [`Board::HALF_NS`] unless a test
    /// asks for the machine at another speed.
    half_ns: u64,
}

impl Board<'_> {
    /// Half of a 200 ns master clock, which is the cpu at normal speed.
    const HALF_NS: u64 = 100;

    /// The netlist quotes a name with a space in it; the wire list does not.
    fn find(&self, name: &str) -> Option<muir::netlist::NetId> {
        self.n.by_name_id(name).or_else(|| self.n.by_name_id(&format!("'{name}'")))
    }

    fn id(&self, name: &str) -> muir::netlist::NetId {
        self.find(name).unwrap_or_else(|| panic!("no net {name}"))
    }

    fn level(&self, name: &str) -> muir::part::Level {
        self.c.net(self.id(name))
    }

    /// Advances to `until`, stopping at every delay-line tap and oscillator
    /// edge on the way, with the master clock running.
    fn run(&mut self, until: u64) {
        while self.now < until {
            let edge = (self.now / self.half_ns + 1) * self.half_ns;
            let next = self.c.next_tap().map_or(edge, |t| t.min(edge));
            self.now = next.min(until);
            if self.now == edge {
                let level = if (edge / self.half_ns).is_multiple_of(2) {
                    muir::part::Level::Low
                } else {
                    muir::part::Level::High
                };
                self.c.drive(self.id("-MCLK7"), level);
            }
            self.c.transition(self.now);
        }
    }
}

fn boundary<'a>(n: &'a Netlist, signals: &[wirelist::Signal]) -> Board<'a> {
    use muir::part::Level;
    let mut c = Chip::new_unclocked(n);
    // The two PROMs that are part of the board: the timeout counter's
    // table and the Unibus grant table, MIT's own listings.
    c.load_rom("0A02", "74S288", &muir::prom::parse_mit(REQTIM).unwrap());
    c.load_rom("0D09", "74S472", &muir::prom::parse_mit(UPRIOR).unwrap());
    c.power_on();
    let mut b = Board { n, c, now: 0, half_ns: Board::HALF_NS };
    for s in signals.iter().filter(|s| !s.is_pseudo()) {
        let pages: BTreeSet<&str> =
            s.pins.iter().filter(|p| p.body == "CON").map(|p| p.page.as_str()).collect();
        // A wire the list has on the connector alone is not in the netlist.
        if pages.iter().any(|p| ["CUBUS", "CXBUS", "DBGIN", "DBGOUT"].contains(p))
            && let Some(net) = b.find(s.name())
        {
            b.c.pull_up(net);
        }
    }
    for a in 0..22 {
        let net = b.id(&format!("-ADR{a}"));
        b.c.drive(net, Level::High);
    }
    for (name, level) in [
        ("-ADRPAR", Level::High),
        ("-MEMRQ", Level::High),
        ("WRCYC", Level::Low),
        ("-LM UNIBUS RESET", Level::High),
        ("-MCLK7", Level::High),
        ("-LM POWER RESET", Level::Low),
    ] {
        let net = b.id(name);
        b.c.drive(net, level);
    }
    b.c.settle_all();
    let reset = b.id("-LM POWER RESET");
    b.c.drive(reset, Level::High);
    b.c.settle();
    b.run(2_000);
    b
}

/// **The board settles from cold with its boundary driven.**
///
/// **No net is unknown.** Of the nets with a pin on them, those not at a
/// level are undriven and should be: the internal `BUS<31:0>`, the
/// diagnostic `SPY<15:0>`, and the address and data the board would put on
/// the Unibus and Xbus as a master, `UAO`, `UDO` and `XAO`, all three-state
/// and all off between cycles; six open-collector outputs whose pull-ups
/// are on the far end of a cable or on the Unibus, `-BUSINT LM RESET`,
/// `LM MEMDRIVE ENB`, `-DEBUG RESET`, `-LMXRQ`, `-WBUFWE` and `BUS READY`;
/// five unnamed wires that are open-collector outputs too, one of them
/// into a delay line, whose tap then floats with it; and the oscillator's
/// capacitor.
#[test]
fn the_board_settles_with_its_boundary_driven() {
    use muir::part::Level;

    let n = busint();
    let signals = support::wire_list(&n, &["cadr1", "busint.wlr"]);
    let b = boundary(&n, &signals);

    // Only nets with a pin on them: the parser's merges leave the other
    // name of every joined wire behind as a net with nothing on it.
    let wired: BTreeSet<muir::netlist::NetId> =
        n.parts.iter().flat_map(|p| p.pins.iter().map(|&(_, net)| net)).collect();
    let wired: Vec<muir::netlist::NetId> =
        wired.into_iter().filter(|&id| !n.net(id).starts_with("NC#")).collect();
    let settled = wired.iter().filter(|&&id| b.c.read_driven(&[id]).is_some()).count();
    let mut floating: Vec<&str> = Vec::new();
    let mut unknown: Vec<&str> = Vec::new();
    for &id in &wired {
        match b.c.net(id) {
            Level::Z => floating.push(n.net(id)),
            Level::X => unknown.push(n.net(id)),
            _ => {}
        }
    }
    floating.sort_unstable();
    unknown.sort_unstable();
    eprintln!("{settled} of {} wired nets at a level with the boundary driven", wired.len());
    eprintln!("{} undriven: {floating:?}", floating.len());
    eprintln!("{} unknown: {unknown:?}", unknown.len());
    assert!(unknown.is_empty(), "nets at unknown with the boundary driven: {unknown:?}");
    let quiet = |name: &str| {
        ["BUS", "SPY", "UAO", "UDO", "XAO", "'VCO CAP", "@"].iter().any(|p| name.starts_with(p))
            || [
                "'-BUSINT LM RESET'",
                "'LM MEMDRIVE ENB'",
                "'-DEBUG RESET'",
                "-LMXRQ",
                "-WBUFWE",
                "'BUS READY'",
                "CLK0",
                "FREE",
            ]
            .contains(&name)
    };
    let odd: Vec<&&str> = floating.iter().filter(|n| !quiet(n)).collect();
    assert!(odd.is_empty(), "undriven nets that should be driven: {odd:?}");
    assert_eq!((settled, floating.len()), (653, 117), "nets at a level, and undriven");
}

/// **A request nobody answers times out**, which is the first thing the
/// netlist bus interface does on its own.
///
/// The cpu asks for a read of Xbus address 0 --- `-MEMRQ` low --- and no
/// memory is on the bus to acknowledge it. The board should take the
/// request at a master clock, put `-XBUS RQ` on the Xbus, start `INT BUSY`
/// and with it the 74LS124, count the REQTIM PROM's table up on that
/// clock, and at its fifth state --- "10 usec NXM TIMEOUT" --- raise
/// `NXM TIMEOUT`, acknowledge the cpu with `-LM ACK`, and set the error
/// register. That is `Busint::TIMEOUT_NS` and `machine::bus_error` as
/// parts.
#[test]
fn a_request_nobody_answers_times_out() {
    use muir::part::Level;

    let n = busint();
    let signals = support::wire_list(&n, &["cadr1", "busint.wlr"]);
    let mut b = boundary(&n, &signals);

    let watch = [
        "-MEMRQ",
        "-XBUS RQ",
        "INT BUSY",
        "TIMEOUT 0",
        "TIMEOUT 1",
        "TIMEOUT 2",
        "TIMEOUT 3",
        "NXM TIMEOUT",
        "-NXM TIMEOUT",
        "HUNG TIMEOUT",
        "-LMACK",
        "XB NXM ERROR",
        "-LMXRQ",
        "LMX GRANT",
        "XBUS REQUEST",
        "-RESET ERR",
    ];
    let mut last: Vec<Level> = watch.iter().map(|w| b.level(w)).collect();
    for (w, l) in watch.iter().zip(&last) {
        eprintln!("  at rest {w}={l:?}");
    }
    let t0 = b.now;
    let memrq = b.id("-MEMRQ");
    b.c.drive(memrq, Level::Low);
    let mut timeline: Vec<(u64, String)> = Vec::new();
    let mut acked = None;
    while b.now < t0 + 14_000 {
        let next = b.now + 50;
        b.run(next);
        for (k, w) in watch.iter().enumerate() {
            let l = b.level(w);
            if l != last[k] {
                timeline.push((b.now - t0, format!("{w}={l:?}")));
                last[k] = l;
            }
        }
        if acked.is_none() && b.level("-LMACK") == Level::Low {
            acked = Some(b.now - t0);
            // The cpu drops the request when acknowledged.
            b.c.drive(memrq, Level::High);
        }
    }
    for (at, what) in &timeline {
        eprintln!("  {at:>6} ns: {what}");
    }
    let acked = acked.expect("the request was never acknowledged");
    eprintln!("acknowledged, by the timeout, after {acked} ns");
    // `-MEMRQ` falls with the master clock low, so the rising edge that
    // takes the request and starts the oscillator is half a period on. The
    // oscillator's first edge comes with its enable and its sixth, which
    // registers the NXM bit, five 2,000 ns periods later: `TIMEOUT_NS`
    // after the edge, as `src/busint.rs` counts it for `rtl`.
    assert_eq!(acked, Board::HALF_NS + muir::busint::TIMEOUT_NS, "when the NXM timeout came");
}

/// **The two PROM listings are the board's, pin for pin.**
///
/// `cadr1/reqtim.prom` and `cadr1/uprior.prom` are MIT's listings of the
/// timeout counter's table and the Unibus grant table. Each reached this
/// repository by one route only, so there is no second copy to hold them
/// to. What can be checked is that each is the part it says --- 32 words for
/// the 74S288 at REQTIM 0A02, 512 for the 74S472 at UPRIOR 0D09 --- that two
/// entries read as their own comments say, and that the address and data
/// encodings the listings' comments give land on the pins the netlist
/// wires, in the pin order `src/part.rs` gives each part.
///
/// UPRIOR's address `1` is `NPR` on A0, its `400` is `-DISABLE INT GRANT`
/// on A8 and its data `1` is `NPG` on O0, with every bit between as the
/// listing has it; the board's names carry a `D` on the requests and a `P`
/// on the grants. REQTIM's data `1`..`10` are `TIMEOUT 0`..`3` and its `20`
/// the NXM bit; its address `1`..`10` are the same four bits back through
/// the 74LS273 at 0B01, the counter's state register, and its `20`, which
/// the listing calls `DEBUG REQUEST ACTIVE`, is the registered `SELECT
/// DEBUG`. The listing's `100`, `WARNING TIMEOUT`, comes out on a pin the
/// board names `PROM UNUSED`.
#[test]
fn the_proms_are_mits_listings() {
    let reqtim = muir::prom::parse_mit(REQTIM).unwrap();
    let uprior = muir::prom::parse_mit(UPRIOR).unwrap();
    assert_eq!(reqtim.len(), 32);
    assert_eq!(uprior.len(), 512);
    // "5  26  ;10 usec  NXM TIMEOUT": next state 6 and the NXM bit.
    assert_eq!(reqtim[5], 0o26);
    // "775  41  ;NPR -> NPG, ANY GRANT".
    assert_eq!(uprior[0o775], 0o41);

    let n = busint();
    // The net on a pin of the part at a location.
    let at = |reference: &str, kind: &str, pin: u8| -> muir::netlist::NetId {
        let p = n
            .parts
            .iter()
            .find(|p| p.reference == reference && p.kind == kind)
            .unwrap_or_else(|| panic!("no {kind} at {reference}"));
        p.pins
            .iter()
            .find(|&&(k, _)| k == pin)
            .map(|&(_, net)| net)
            .unwrap_or_else(|| panic!("{reference} has no pin {pin}"))
    };
    // A listing's `NC` is a pin the netlist gives a numbered net of its own.
    let named = |net: muir::netlist::NetId, want: &str| -> bool {
        if want == "NC" { n.net(net).starts_with("NC#") } else { n.net(net) == want }
    };

    // The 74S472's address pins A0..A8 and outputs O0..O7, against the
    // listing's ADDRESS and DATA ENCODING.
    const A472: [u8; 9] = [1, 2, 3, 4, 5, 16, 17, 18, 19];
    const O472: [u8; 8] = [6, 7, 8, 9, 11, 12, 13, 14];
    let uprior_address = [
        "NPRD",
        "BR7D",
        "BR6D",
        "BR5D",
        "BR4D",
        "LEVEL0",
        "LEVEL1",
        "'-CLEAR GRANT'",
        "'-DISABLE INT GRANT'",
    ];
    let uprior_data =
        ["NPGP", "BG7P", "BG6P", "BG5P", "BG4P", "'ANY GRANT'", "'ANY INT GRANT'", "NC"];
    for (k, name) in uprior_address.iter().enumerate() {
        assert_eq!(n.net(at("0D09", "74S472", A472[k])), *name, "UPRIOR address bit {k}");
    }
    for (k, name) in uprior_data.iter().enumerate() {
        let net = at("0D09", "74S472", O472[k]);
        assert!(named(net, name), "UPRIOR data bit {k}: {} for {name}", n.net(net));
    }

    // The 74S288's outputs O0..O7 and address pins A0..A4, the same way.
    const O288: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 9];
    const A288: [u8; 5] = [10, 11, 12, 13, 14];
    let reqtim_data = [
        "'TIMEOUT 0'",
        "'TIMEOUT 1'",
        "'TIMEOUT 2'",
        "'TIMEOUT 3'",
        "'PROM NXM TIMEOUT'",
        "'PROM HUNG TIMEOUT'",
        "'PROM UNUSED'",
        "NC",
    ];
    for (k, name) in reqtim_data.iter().enumerate() {
        let net = at("0A02", "74S288", O288[k]);
        assert!(named(net, name), "REQTIM data bit {k}: {} for {name}", n.net(net));
    }
    // The address is the state register's Q pins, each fed by the D pin on
    // the data bit of the same name. The 74LS273's D pins 3, 4, 7, 8, 13,
    // 14, 17, 18 clock to Q pins 2, 5, 6, 9, 12, 15, 16, 19, as
    // `src/part.rs` has it.
    const DQ: [(u8, u8); 8] =
        [(3, 2), (4, 5), (7, 6), (8, 9), (13, 12), (14, 15), (17, 16), (18, 19)];
    let registered = |name: &str| -> muir::netlist::NetId {
        let &(_, q) = DQ
            .iter()
            .find(|&&(d, _)| n.net(at("0B01", "74LS273", d)) == name)
            .unwrap_or_else(|| panic!("no D pin of 0B01 is on {name}"));
        at("0B01", "74LS273", q)
    };
    for (k, name) in reqtim_data[..4].iter().enumerate() {
        assert_eq!(
            at("0A02", "74S288", A288[k]),
            registered(name),
            "REQTIM address bit {k} is the registered {name}"
        );
    }
    assert_eq!(
        at("0A02", "74S288", A288[4]),
        registered("'SELECT DEBUG'"),
        "REQTIM address bit 4, the listing's DEBUG REQUEST ACTIVE, is the registered SELECT DEBUG"
    );
}

/// **A Unibus cycle is arbitrated before it is run**, and what it costs.
///
/// The Xbus cycle above is granted at the master clock that samples
/// `-MEMRQ`. A Unibus cycle is not: the board is the Unibus arbiter in local
/// mode, and its own request goes through the arbitration --- `NPR` to the
/// UPRIOR PROM, `NPG` out on the grant chain and back, `SACK` --- before the
/// request synchroniser on RQSYNC lets it have the bus, and then the
/// address is deskewed, `-UB MSYN` goes out, the slave answers `-UB SSYN`,
/// and `-LMACK` follows through the TD250 at REQU 0B09. This runs three
/// such cycles against the far end in `src/buses.rs` and prints each one
/// wire by wire: a write of the mode register, which the board answers
/// itself on page DIAG; a read of the I/O board's status register, which
/// the far end answers; and a read of the Chaosnet interface, which nothing
/// answers. `src/busint.rs` carries those timings for `rtl`, and
/// [`Busint`] is run alongside each cycle here, on the same master clock,
/// to hold it to them.
#[test]
fn a_unibus_cycle_is_arbitrated_before_it_is_run() {
    unibus_cycles_at(Board::HALF_NS);
}

/// The same three cycles with the master clock at the 220 ns the machine
/// boots in, extra slow.  The synchronisers on UBMAST and RQSYNC count
/// edges and the delay lines count nanoseconds, so what a cycle costs
/// depends on the period; `rtl` and the board were found a microcycle
/// apart on a halt written from the console at this speed, in
/// `tests/chip.rs`, and the model matched the board at 200.
#[test]
fn a_unibus_cycle_is_arbitrated_the_same_way_at_220_ns() {
    unibus_cycles_at(110);
}

fn unibus_cycles_at(half_ns: u64) {
    use muir::buses::Buses;
    use muir::busint::{self, Busint};
    use muir::machine::Machine;
    use muir::part::Level;

    let n = busint();
    let signals = support::wire_list(&n, &["cadr1", "busint.wlr"]);
    let mut b = boundary(&n, &signals);
    b.half_ns = half_ns;
    // The Unibus reset the cpu gives the board at power-on, which the
    // boundary above holds released.
    let ubreset = b.id("-LM UNIBUS RESET");
    b.c.drive(ubreset, Level::Low);
    b.c.settle();
    b.c.drive(ubreset, Level::High);
    b.c.settle();
    b.run(b.now + 1_000);
    let mut buses = Buses::new(&n, Machine::new());

    let watch = [
        "-MEMRQ",
        "LM NEED UB",
        "ANY GRANT",
        "NPG1 IN",
        "LM UB GRANTED",
        "LM UB SELECTED",
        "SACK IN",
        "LMUB MASTER",
        "LMUB GRANT",
        "-UB MSYN",
        "-UB SSYN",
        "UB MD LOAD",
        "-LOADMD",
        "-LMACK",
        "NXM TIMEOUT",
        "INT BUSY",
        "-XBUS RQ",
        "-SPY WRITE",
    ];
    let mem: Vec<muir::netlist::NetId> = (0..32).map(|k| b.id(&format!("MEM{k}"))).collect();
    let addr: Vec<muir::netlist::NetId> = (0..22).map(|a| b.id(&format!("-ADR{a}"))).collect();
    let ubd: Vec<muir::netlist::NetId> = (0..16).map(|k| b.id(&format!("-UBD{k}"))).collect();
    let (memrq, wrcyc, lmack, loadmd) =
        (b.id("-MEMRQ"), b.id("WRCYC"), b.id("-LMACK"), b.id("-LOADMD"));
    // The behavioural interface, run alongside every cycle on the same
    // master clock and kept across them, as it keeps the Unibus.
    let mut model = Busint::default();

    // One cycle: the address and direction on the cables, `-MEMRQ` down
    // until `-LMACK`, and the wires watched every ten nanoseconds. Returns
    // when `-LMACK` came, the word `MEM` carried when `-LOADMD` rose, when
    // the behavioural interface acknowledged the same cycle, and --- for a
    // write of one of the board's own diagnostic registers --- when the
    // strobe's trailing edge clocked it on the board and when the model
    // says the register took the word.
    let mut cycle = |b: &mut Board,
                     phys: u32,
                     write: bool,
                     data: u32|
     -> (u64, Option<u32>, u64, Option<u64>, Option<u64>) {
        for (a, &net) in addr.iter().enumerate() {
            b.c.drive(net, Level::from((phys >> a) & 1 == 0));
        }
        b.c.drive(wrcyc, Level::from(write));
        if write {
            for (k, &net) in mem.iter().enumerate() {
                b.c.drive(net, Level::from((data >> k) & 1 != 0));
            }
        }
        b.c.settle();
        b.run(b.now + 100);
        let mut last: Vec<Level> = watch.iter().map(|w| b.level(w)).collect();
        let mut buses_were = (b.c.read_driven(&mem), b.c.read_driven(&ubd));
        let responder = busint::decode(phys, 0);
        model.request(write);
        let t0 = b.now;
        b.c.drive(memrq, Level::Low);
        let (mut acked, mut model_acked, mut word) = (None, None, None);
        let (mut strobe, mut model_answered) = (None, None);
        let mut timeline: Vec<(u64, String)> = Vec::new();
        let mut loadmd_was = b.c.net(loadmd);
        while b.now < t0 + 14_000 && (acked.is_none() || b.now < acked.unwrap() + t0 + 600) {
            let was = b.now;
            // To the board's next tap, the far end's next event --- the I/O
            // board's answer --- or ten nanoseconds on, whichever is first,
            // so that every acknowledgement is sampled when it happens.
            let stop = [Some(was + 10), b.c.next_tap(), buses.next_event()]
                .into_iter()
                .flatten()
                .filter(|&t| t > was)
                .min()
                .unwrap();
            b.run(stop);
            loop {
                if !buses.tick(&mut b.c, b.now) {
                    break;
                }
                b.c.transition(b.now);
            }
            // The rising edges of `-MCLK7` in the step, as `Board::run`
            // drives them, for the model: `-MEMRQ` was already down at
            // every one of them.
            for t in was + 1..=b.now {
                if t % b.half_ns == 0 && (t / b.half_ns) % 2 == 1 && acked.is_none() {
                    model.mclk_edge(t, responder);
                }
            }
            if model_acked.is_none()
                && let Some(ack) = model.poll(b.now, responder)
            {
                model_acked = Some(ack.at - t0);
            }
            if model_answered.is_none()
                && let Some(at) = model.answered_at()
            {
                model_answered = Some(at - t0);
            }
            for (k, w) in watch.iter().enumerate() {
                let l = b.level(w);
                if l != last[k] {
                    timeline.push((b.now - t0, format!("{w}={l:?}")));
                    if *w == "-SPY WRITE" && last[k] == Level::Low && l == Level::High {
                        strobe = Some(b.now - t0);
                    }
                    last[k] = l;
                }
            }
            let buses_are = (b.c.read_driven(&mem), b.c.read_driven(&ubd));
            if buses_are != buses_were {
                let show = |v: Option<u64>| v.map_or("undriven".to_string(), |v| format!("{v:o}"));
                timeline.push((
                    b.now - t0,
                    format!("MEM {} -UBD {}", show(buses_are.0), show(buses_are.1)),
                ));
                buses_were = buses_are;
            }
            let l = b.c.net(loadmd);
            if loadmd_was == Level::Low && l == Level::High {
                word = Some(b.c.read(&mem) as u32);
                timeline.push((b.now - t0, format!("MEM={:o}", word.unwrap())));
            }
            loadmd_was = l;
            if acked.is_none() && b.c.net(lmack) == Level::Low {
                acked = Some(b.now - t0);
                b.c.drive(memrq, Level::High);
            }
        }
        for (at, what) in &timeline {
            eprintln!("  {at:>6} ns: {what}");
        }
        if write {
            for &net in &mem {
                b.c.release(net);
            }
        }
        model.finish();
        let acked = acked.expect("the request was never acknowledged");
        eprintln!("acknowledged after {acked} ns; the model says {model_acked:?}");
        if let Some(s) = strobe {
            eprintln!(
                "the register strobe's trailing edge at {s} ns; the model says {model_answered:?}"
            );
        }
        (acked, word, model_acked.expect("the model never acknowledged"), strobe, model_answered)
    };

    eprintln!("a write of 4 to the mode register, Unibus 766012:");
    let (write_ack, _, model, strobe, model_answered) = cycle(&mut b, 0o17773005, true, 4);
    assert_eq!(model, write_ack, "the model's acknowledgement of a write the board answers itself");
    // The register takes the word at the trailing edge of `-SPY WRITE` ---
    // `-DBWRITE` on the cable, "which causes a clock" --- and the model's
    // `answered_at` is that edge: a halt or a boot written from the
    // console lands in the same microcycle on `rtl` as on the board only
    // if it is.  Found one microcycle apart in `tests/chip.rs`.
    let strobe = strobe.expect("-SPY WRITE never pulsed for a write of the mode register");
    assert_eq!(model_answered, Some(strobe), "the model's register strobe against the board's");

    eprintln!("a read of the I/O board's microsecond clock, Unibus 764120:");
    let (read_ack, word, model, _, _) = cycle(&mut b, 0o17772050, false, 0);
    assert_eq!(model, read_ack, "the model's acknowledgement of a read the I/O board answers");
    eprintln!("MEM carried {word:?} at the strobe");

    eprintln!("a read of the Chaosnet interface, Unibus 764140, which is not there:");
    let (nxm_ack, _, model, _, _) = cycle(&mut b, 0o17772060, false, 0);
    assert_eq!(model, nxm_ack, "the model's timeout of a Unibus cycle nothing answers");
}

/// **The behavioural I/O board interrupts through the interface.** Under
/// `chip --io-board model` the keyboard's word goes into the model board,
/// and its interrupt has to reach the microcode by the same road the
/// netlist board's does: `src/buses.rs` runs the device's side of the
/// Unibus interrupt cycle against the netlist interface --- `-UB BR5`,
/// the grant on `UB BG5 IN`, `-UB SACK`, then `-UB INTR` with vector 260
/// on the data lines until `INTR SSYN` --- and the interface's `UB INT`
/// sets with the vector latched, which is what a read of `766040` then
/// shows. Reading the keyboard's two registers through the interface
/// clears the request, and a write of zero to `766042` clears `UB INT`.
#[test]
fn the_behavioural_io_board_interrupts_through_the_interface() {
    use muir::buses::Buses;
    use muir::busint::interrupt_status::{ENABLE_UB_INTS, UB_INT};
    use muir::ioboard::{self, csr};
    use muir::machine::Machine;
    use muir::part::Level;
    use muir::terminal::keyboard;

    let n = busint();
    let signals = support::wire_list(&n, &["cadr1", "busint.wlr"]);
    let mut b = boundary(&n, &signals);
    let ubreset = b.id("-LM UNIBUS RESET");
    b.c.drive(ubreset, Level::Low);
    b.c.settle();
    b.c.drive(ubreset, Level::High);
    b.c.settle();
    b.run(b.now + 1_000);
    let mut buses = Buses::new(&n, Machine::new());

    let mem: Vec<muir::netlist::NetId> = (0..32).map(|k| b.id(&format!("MEM{k}"))).collect();
    let addr: Vec<muir::netlist::NetId> = (0..22).map(|a| b.id(&format!("-ADR{a}"))).collect();
    let (memrq, wrcyc, lmack, loadmd) =
        (b.id("-MEMRQ"), b.id("WRCYC"), b.id("-LMACK"), b.id("-LOADMD"));

    // Runs the board and the far end together to `until`, or until `done`.
    let step = |b: &mut Board, buses: &mut Buses, until: u64, done: &dyn Fn(&Board) -> bool| {
        while b.now < until && !done(b) {
            let was = b.now;
            let stop = [Some(was + 10), b.c.next_tap(), buses.next_event()]
                .into_iter()
                .flatten()
                .filter(|&t| t > was)
                .min()
                .unwrap();
            b.run(stop);
            while buses.tick(&mut b.c, b.now) {
                b.c.transition(b.now);
            }
        }
    };

    // A cycle from the processor: `-MEMRQ` down until `-LMACK`, the word
    // `MEM` carried at `-LOADMD` for a read.
    let cpu = |b: &mut Board, buses: &mut Buses, phys: u32, write: Option<u32>| -> u32 {
        for (a, &net) in addr.iter().enumerate() {
            b.c.drive(net, Level::from((phys >> a) & 1 == 0));
        }
        b.c.drive(wrcyc, Level::from(write.is_some()));
        if let Some(data) = write {
            for (k, &net) in mem.iter().enumerate() {
                b.c.drive(net, Level::from((data >> k) & 1 != 0));
            }
        }
        b.c.settle();
        b.run(b.now + 100);
        b.c.drive(memrq, Level::Low);
        let t0 = b.now;
        let mut word = 0u32;
        let mut loadmd_was = b.c.net(loadmd);
        let mut acked = None;
        // `MEM` as it stood before the transition that raised `-LOADMD`:
        // what MD latches. The interface's own registers let their
        // buffer go in the same instant, so `MEM` after it is the bus
        // released.
        let mut mem_before = b.c.read(&mem) as u32;
        while b.now < t0 + 20_000 && acked.is_none_or(|a| b.now < a + 600) {
            let was = b.now;
            let stop = [Some(was + 10), b.c.next_tap(), buses.next_event()]
                .into_iter()
                .flatten()
                .filter(|&t| t > was)
                .min()
                .unwrap();
            b.run(stop);
            while buses.tick(&mut b.c, b.now) {
                b.c.transition(b.now);
            }
            let l = b.c.net(loadmd);
            if loadmd_was == Level::Low && l == Level::High {
                word = mem_before;
            }
            loadmd_was = l;
            mem_before = b.c.read(&mem) as u32;
            if acked.is_none() && b.c.net(lmack) == Level::Low {
                acked = Some(b.now);
                b.c.drive(memrq, Level::High);
            }
        }
        assert!(acked.is_some(), "the cycle to {phys:o} was never acknowledged");
        if write.is_some() {
            for &net in &mem {
                b.c.release(net);
            }
        }
        b.run(b.now + 200);
        word
    };

    // The microcode's own enable, at the end of the cold boot: 6000 to
    // 766040.
    cpu(&mut b, &mut buses, 0o17773020, Some(0o6000));
    let status = cpu(&mut b, &mut buses, 0o17773020, None) as u16;
    eprintln!("766040 reads {status:o} after the enable");
    assert_ne!(status & ENABLE_UB_INTS, 0, "ENABLE UB INTS took: {status:o}");
    assert_eq!(status & UB_INT, 0, "nothing taken yet: {status:o}");

    // A key into the behavioural board, with its interrupt enabled.
    buses.machine.ioboard.write(ioboard::CSR, csr::KBD_INT_ENABLE, 0);
    let word = keyboard::up_down(0o123, false);
    buses.machine.ioboard.press(word);
    assert_eq!(buses.machine.ioboard.interrupt_request(buses.machine.ns), Some(0o260));

    let watch = [
        "-UB BR5",
        "UB BG5 IN",
        "-UB SACK",
        "-UB BBSY",
        "-UB INTR",
        "-UB SSYN",
        "INTR SSYN",
        "UB INT",
    ];
    let mut last: Vec<Level> = watch.iter().map(|w| b.level(w)).collect();
    let ub_int = b.id("UB INT");
    let t0 = b.now;
    let mut timeline = Vec::new();
    while b.now < t0 + 20_000 && b.c.net(ub_int) != Level::High {
        let until = b.now + 1;
        step(&mut b, &mut buses, until, &|_| false);
        for (k, w) in watch.iter().enumerate() {
            let l = b.level(w);
            if l != last[k] {
                timeline.push(format!("{:>6} ns: {w}={l:?}", b.now - t0));
                last[k] = l;
            }
        }
    }
    for line in &timeline {
        eprintln!("  {line}");
    }
    assert_eq!(b.c.net(ub_int), Level::High, "UB INT set within 20 us of the request");
    // The cycle ran to its end and let the bus go.
    let until = b.now + 2_000;
    step(&mut b, &mut buses, until, &|_| false);
    for w in ["-UB BR5", "-UB SACK", "-UB BBSY", "-UB INTR"] {
        assert_ne!(b.level(w), Level::Low, "{w} let go after the cycle");
    }

    // What the microcode sees, and does: the status with the vector, the
    // two halves of the word, and the dismissal.
    let status = cpu(&mut b, &mut buses, 0o17773020, None) as u16;
    eprintln!("766040 reads {status:o}");
    assert_ne!(status & UB_INT, 0, "UB INT reads set");
    assert_eq!(status & 0o1774, 0o260, "the vector, in place");
    let high = cpu(&mut b, &mut buses, 0o17772041, None);
    let low = cpu(&mut b, &mut buses, 0o17772040, None);
    assert_eq!((high & 0xff) << 16 | (low & 0xffff), word, "the word, through the interface");
    assert!(!buses.machine.ioboard.keyboard_ready(), "read, so the request is gone");
    cpu(&mut b, &mut buses, 0o17773021, Some(0));
    let until = b.now + 1_000;
    step(&mut b, &mut buses, until, &|_| false);
    assert_eq!(b.c.net(ub_int), Level::Low, "dismissed");
    let status = cpu(&mut b, &mut buses, 0o17773020, None) as u16;
    assert_eq!(status & UB_INT, 0, "and reads clear: {status:o}");
}
