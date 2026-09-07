// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ALU control, checked against the parts that generate it.
//!
//! `ttl::alu_control` says what reaches the 74S181 control pins, from the
//! instruction register onwards. None of that is asserted here --- it is
//! *evaluated*, by pulling the ten packages that implement it out of
//! `data/CADR.netlist` and running them through the part behaviour in
//! `src/part.rs`. So the check runs from the drawings and the datasheets,
//! with nothing taken from another emulator on the way.

use std::collections::{BTreeMap, BTreeSet};

use muir::netlist::{self, Netlist, Package};
use muir::part::{self, Level, Pins, State};
use muir::ttl;

const NETLIST: &str = include_str!("../data/CADR.netlist");

/// The parts that make up the ALU control, by page and reference.
///
/// Everything from the instruction register to the 74S181 control pins.
const CLUSTER: &[(&str, &str)] = &[
    ("SOURCE", "3D02"), // -SPECALU
    ("SOURCE", "3D04"), // -MUL, -DIV
    ("ALUC4", "2C11"),  // the IR and A31 inverters
    ("ALUC4", "2C10"),  // -DIVPOSLASTTIME, DIVSUBCOND, DIVADDCOND
    ("ALUC4", "2D15"),  // -MULNOP
    ("ALUC4", "2C15"),  // the DIV*COND / A31 products
    ("ALUC4", "2C20"),  // ALUADD, ALUSUB
    ("ALUC4", "2B16"),  // -ALUF3, -ALUF2
    ("ALUC4", "2B17"),  // -ALUF1, -ALUF0
    ("ALUC4", "2B18"),  // -ALUMODE, -CIN0
];

/// The packages of [`CLUSTER`], in that order.
///
/// A reference can yield more than one `Package`: SUDS draws some gates of a
/// chip as the open-collector variant and its others as the plain one, and
/// `Netlist::packages` keys on the type, so it splits them. ALUC4 2C10 is one
/// of the thirteen. Both halves are needed here.
fn cluster(n: &Netlist) -> Vec<Package> {
    let all = n.packages();
    let mut out = Vec::new();
    for &(page, reference) in CLUSTER {
        let before = out.len();
        out.extend(all.iter().filter(|p| p.page == page && p.reference == reference).cloned());
        assert!(out.len() > before, "{page} {reference} is not in the netlist");
    }
    out
}

/// What the cluster is driven with: everything it reads that no part in it
/// produces.
struct Inputs {
    ir: u64,
    q0: bool,
    a31: bool,
    iralu: bool,
    irjump: bool,
}

impl Inputs {
    fn net(&self, name: &str) -> Option<bool> {
        let bit = |n: u32| self.ir >> n & 1 != 0;
        Some(match name {
            "GND" => false,
            n if n.starts_with("HI") => true,
            "IRALU" => self.iralu,
            "IRJUMP" => self.irjump,
            "-IRJUMP" => !self.irjump,
            "Q0" => self.q0,
            // A31A and A31B are buffered copies of the same bit; 2C11
            // inverts B into -A31.
            "A31A" | "A31B" => self.a31,
            n => match n.strip_prefix("IR").and_then(|d| d.parse::<u32>().ok()) {
                Some(k) if k <= 8 => bit(k),
                _ => return None,
            },
        })
    }
}

/// Evaluates the cluster by repeated sweeps until nothing changes.
///
/// The parts are not put in order by hand: a gate whose inputs are not known
/// yet reads unknown and drives unknown, so each sweep resolves one more
/// level and the loop settles. That only terminates because the cluster is
/// acyclic, which is the same property the chip engine will rely on.
///
/// Open-collector outputs are read as their logic level. That is sound here
/// because every net in the cluster has exactly one driver, which
/// `every_control_net_has_one_driver` checks.
fn evaluate(n: &Netlist, cluster: &[Package], inputs: &Inputs) -> BTreeMap<String, bool> {
    let mut known: BTreeMap<String, bool> = BTreeMap::new();
    for pkg in cluster {
        for &(_, net) in &pkg.pins {
            if let Some(v) = inputs.net(n.net(net)) {
                known.insert(n.net(net).to_string(), v);
            }
        }
    }
    for _ in 0..cluster.len() + 1 {
        let before = known.len();
        for pkg in cluster {
            let b = part::behaviour(&pkg.kind).unwrap();
            let mut pins: Pins = [Level::X; muir::part::MAX_PINS];
            for &(pin, net) in &pkg.pins {
                if let Some(&v) = known.get(n.net(net)) {
                    pins[pin as usize] = Level::from(v);
                }
            }
            let state = State::default();
            for g in b.gates {
                let Some(&(_, net)) = pkg.pins.iter().find(|&&(pin, _)| pin == g.out) else {
                    continue;
                };
                let name = n.net(net);
                if name.starts_with("NC#") {
                    continue;
                }
                match g.eval(&pins, &state) {
                    Level::High => {
                        known.insert(name.to_string(), true);
                    }
                    Level::Low => {
                        known.insert(name.to_string(), false);
                    }
                    _ => {}
                }
            }
        }
        if known.len() == before {
            break;
        }
    }
    known
}

/// What the parts produce on the 74S181's control pins, and the two nets
/// that select them.
fn from_the_netlist(n: &Netlist, cluster: &[Package], inputs: &Inputs) -> ttl::Control {
    let v = evaluate(n, cluster, inputs);
    let at = |name: &str| *v.get(name).unwrap_or_else(|| panic!("{name} never settled"));
    // 2A16, 2A17 and 2B20 invert -ALUF and -ALUMODE; pin 7 of the 74S181 is
    // -CIN0, so the carry in is its complement.
    ttl::Control {
        aluf: (!at("-ALUF3") as u8) << 3
            | (!at("-ALUF2") as u8) << 2
            | (!at("-ALUF1") as u8) << 1
            | !at("-ALUF0") as u8,
        alumode: !at("-ALUMODE"),
        cin: !at("-CIN0"),
        alusub: at("ALUSUB"),
        aluadd: at("ALUADD"),
    }
}

/// The simplification the evaluator makes --- reading an open-collector
/// output as its logic level --- holds only while nothing is wire-ANDed.
#[test]
fn every_control_net_has_one_driver() {
    let n = netlist::parse(NETLIST).unwrap();
    let inside: Vec<(String, String)> =
        CLUSTER.iter().map(|&(p, r)| (p.to_string(), r.to_string())).collect();
    let mut produced: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for pkg in cluster(&n) {
        let po = part::pinout(&pkg.kind).unwrap();
        for &(pin, net) in &pkg.pins {
            if po.drives(pin) && !n.net(net).starts_with("NC#") {
                produced.entry(n.net(net)).or_default().push(pkg.reference.clone());
            }
        }
    }
    // No net the cluster drives may be driven from outside it either.
    for part_rec in &n.parts {
        let po = match part::pinout(&part_rec.kind) {
            Some(p) => p,
            None => continue,
        };
        if inside.iter().any(|(p, r)| *p == part_rec.page && *r == part_rec.reference) {
            continue;
        }
        for &(pin, net) in &part_rec.pins {
            if po.drives(pin)
                && let Some(who) = produced.get_mut(n.net(net))
            {
                who.push(format!("{} {}", part_rec.page, part_rec.reference));
            }
        }
    }
    let shared: Vec<_> = produced.iter().filter(|(_, v)| v.len() > 1).collect();
    assert!(shared.is_empty(), "wire-ANDed control nets: {shared:?}");
}

/// The whole control path, checked against the parts that implement it.
///
/// Nothing here is asserted by hand: `ttl::alu_control` is compared against
/// ten packages pulled out of `data/CADR.netlist` and evaluated through the
/// part behaviour in `src/part.rs`, over every input that reaches them.
#[test]
fn alu_control_matches_the_hardware() {
    let n = netlist::parse(NETLIST).unwrap();
    let cluster = cluster(&n);
    let mut seen = BTreeSet::new();
    let mut checked = 0;
    for bits in 0..1u64 << 7 {
        // IR<8:2>: the only instruction bits the cluster reads.
        let ir = bits << 2;
        for q0 in [false, true] {
            for a31 in [false, true] {
                for iralu in [false, true] {
                    for irjump in [false, true] {
                        let inputs = Inputs { ir, q0, a31, iralu, irjump };
                        let want = from_the_netlist(&n, &cluster, &inputs);
                        let got = ttl::alu_control(ir, q0, a31, iralu, irjump);
                        assert_eq!(
                            got, want,
                            "ir {ir:#o} q0 {q0} a31 {a31} iralu {iralu} irjump {irjump}"
                        );
                        seen.insert((got.alusub, got.aluadd));
                        checked += 1;
                    }
                }
            }
        }
    }
    eprintln!("{checked} combinations checked against {} packages", cluster.len());
    assert_eq!(seen.len(), 4, "not every column of the table was exercised");
}
