// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Checks what the parts compute.
//!
//! Two kinds of check, in rising order of how much they can catch:
//!
//! - **Consistency.** The gates and the pinout in `src/part.rs` must agree,
//!   and between them they must account for every pin the netlist connects.
//!   That is what catches a mux whose select was left out of its `ins`.
//! - **Behaviour.** The 74S181 array is checked against [`muir::ttl::alu`],
//!   which is written from the datasheet function table rather than from the
//!   gate model, so the two are genuinely independent.

use std::collections::{BTreeMap, BTreeSet};

use muir::netlist::{self, Netlist};
use muir::part::{self, Behaviour, Level, Pins, State};
use muir::ttl;

const NETLIST: &str = include_str!("../data/CADR.netlist");

fn load() -> Netlist {
    netlist::parse(NETLIST).unwrap()
}

/// Every part type the netlist actually uses.
fn kinds(n: &Netlist) -> Vec<String> {
    let mut v: Vec<String> = n.parts.iter().map(|p| p.kind.clone()).collect();
    v.sort();
    v.dedup();
    v
}

fn pins() -> Pins {
    [Level::Z; muir::part::MAX_PINS]
}

/// Drives the named pins, everything else left undriven.
fn drive(vals: &[(u8, bool)]) -> Pins {
    let mut p = pins();
    for &(pin, b) in vals {
        p[pin as usize] = Level::from(b);
    }
    p
}

fn out(b: &Behaviour, pin: u8, p: &Pins, s: &State) -> Level {
    let gate = b.gates.iter().find(|g| g.out == pin).expect("no such output pin");
    gate.eval(p, s)
}

fn behaviour(kind: &str) -> Behaviour {
    part::behaviour(kind).unwrap_or_else(|| panic!("{kind} has no behaviour"))
}

// ---------------------------------------------------------------------------
// Consistency
// ---------------------------------------------------------------------------

/// Every type the netlist uses must compute something --- except the delay
/// lines, which are analog and belong to `src/clock.rs`.
#[test]
fn every_part_type_has_a_behaviour() {
    let n = load();
    const DELAY: &[&str] = &["TD25", "TD50", "TD100", "TD250"];
    let missing: Vec<String> = kinds(&n)
        .into_iter()
        .filter(|k| part::behaviour(k).is_none() && !DELAY.contains(&part::strip(k).0))
        .collect();
    assert!(missing.is_empty(), "no behaviour for {missing:?}");
}

/// The gates and the pinout are two statements about the same thing.
#[test]
fn gates_drive_exactly_the_pinout_outputs() {
    let n = load();
    for kind in kinds(&n) {
        let Some(b) = part::behaviour(&kind) else { continue };
        let Some(po) = part::pinout(&kind) else { panic!("{kind} has a behaviour but no pinout") };
        let gated: BTreeSet<u8> = b.gates.iter().map(|g| g.out).collect();
        let claimed: BTreeSet<u8> = po.outputs.iter().copied().collect();
        assert_eq!(gated, claimed, "{kind}: gates drive {gated:?}, pinout says {claimed:?}");
        assert_eq!(gated.len(), b.gates.len(), "{kind} has two gates on one pin");
    }
}

/// No gate may read a pin its own package drives: that would be a loop
/// inside one part, and the levelizer could not order it.
#[test]
fn no_gate_reads_its_own_packages_output() {
    let n = load();
    for kind in kinds(&n) {
        let Some(b) = part::behaviour(&kind) else { continue };
        let driven: BTreeSet<u8> = b.gates.iter().map(|g| g.out).collect();
        for gate in b.gates {
            for pin in gate.ins {
                assert!(
                    !driven.contains(pin),
                    "{kind}: gate on {} reads driven pin {pin}",
                    gate.out
                );
            }
        }
    }
}

/// Every pin the board connects must have a role: driven by a gate, read by
/// one, or read by the state update. A pin with no role is a pin left out of
/// somebody's `ins` list.
///
/// The two parts that compute nothing are skipped, not excused: the dummy
/// socket is a set of wire links, and the TIL309's inputs go only to its own
/// display.
#[test]
fn every_connected_pin_has_a_role() {
    const NO_LOGIC: &[&str] = &["TIL309", "16DUMMY"];
    let n = load();
    let mut unaccounted: BTreeMap<String, BTreeSet<u8>> = BTreeMap::new();
    for pkg in n.packages() {
        if NO_LOGIC.contains(&part::strip(&pkg.kind).0) {
            continue;
        }
        let Some(b) = part::behaviour(&pkg.kind) else { continue };
        let po = part::pinout(&pkg.kind).unwrap();
        let mut known: BTreeSet<u8> = b.gates.iter().map(|g| g.out).collect();
        known.extend(b.gates.iter().flat_map(|g| g.ins.iter().copied()));
        known.extend(b.update_ins.iter().copied());
        for &(pin, net) in &pkg.pins {
            if n.net(net).starts_with("NC#")
                || (po.package != 0 && (pin == po.gnd() || pin == po.vcc()))
            {
                continue;
            }
            if !known.contains(&pin) {
                unaccounted.entry(pkg.kind.clone()).or_default().insert(pin);
            }
        }
    }
    assert!(unaccounted.is_empty(), "pins with no role: {unaccounted:?}");
}

// ---------------------------------------------------------------------------
// Gates
// ---------------------------------------------------------------------------

/// An unknown a gate reads makes its output unknown where the output
/// depends on it, and nothing else does.
#[test]
fn unknown_propagates_only_through_what_is_read() {
    let b = behaviour("74S00");
    let mut p = drive(&[(1, true), (2, true), (4, true), (5, true)]);
    assert_eq!(out(&b, 3, &p, &State::default()), Level::Low);
    p[1] = Level::X;
    assert_eq!(out(&b, 3, &p, &State::default()), Level::X);
    // The other three gates in the package are untouched.
    assert_eq!(out(&b, 6, &p, &State::default()), Level::Low);
    // A NAND with its other input low is high whatever the unknown is.
    p[2] = Level::Low;
    assert_eq!(out(&b, 3, &p, &State::default()), Level::High);
}

/// **An unknown a gate's output does not depend on is not an unknown
/// output.** The 74S51 at DCCLK 0C18 makes the disk controller's
/// sequencer clock of two legs: the 2 us clock under its enable, and the
/// disk's bit clock under its enable. With nothing on the cable the bit
/// clock is unknown --- the 75107 says so honestly --- but its enable is
/// low, and the part's output is the other leg's. Under a rule that
/// took any unknown input as absorbing, the sequencer never clocked.
#[test]
fn an_unknown_on_a_gated_off_leg_is_not_an_unknown_output() {
    let b = behaviour("74S51");
    // 2 = enable, 3 = -2USEC.CLK^, 4 = -BIT.CLK^, 5 = the bit clock's enable.
    let mut p = drive(&[(2, true), (3, false), (4, true), (5, false)]);
    assert_eq!(out(&b, 6, &p, &State::default()), Level::High);
    p[4] = Level::X;
    assert_eq!(out(&b, 6, &p, &State::default()), Level::High, "leg gated off");
    p[3] = Level::High;
    assert_eq!(out(&b, 6, &p, &State::default()), Level::Low, "the other leg");
    p[5] = Level::High;
    assert_eq!(out(&b, 6, &p, &State::default()), Level::Low, "the AND-OR-INVERT is low anyway");
    p[3] = Level::Low;
    assert_eq!(out(&b, 6, &p, &State::default()), Level::X, "now it depends on the unknown");
    // Past the limit, a gate does not try: seventeen unknowns on a
    // 74S181 F output is a bus nothing has settled.
    let alu = behaviour("74S181");
    let all = [Level::X; muir::part::MAX_PINS];
    assert_eq!(out(&alu, 9, &all, &State::default()), Level::X);
}

/// An undriven TTL input floats high.
#[test]
fn undriven_inputs_read_high() {
    let b = behaviour("74S00");
    assert_eq!(out(&b, 3, &pins(), &State::default()), Level::Low);
}

/// The small gates, exhaustively, against their names.
#[test]
fn gate_truth_tables() {
    let s = State::default();
    /// A part type, one of its output pins, the pins that gate reads, and
    /// what the datasheet says it computes.
    type Case = (&'static str, u8, &'static [u8], fn(&[bool]) -> bool);
    let cases: &[Case] = &[
        ("74S00", 3, &[1, 2], |i| !(i[0] && i[1])),
        ("74S02", 1, &[2, 3], |i| !(i[0] || i[1])),
        ("7428", 1, &[2, 3], |i| !(i[0] || i[1])),
        ("74S04", 2, &[1], |i| !i[0]),
        ("74S08", 3, &[1, 2], |i| i[0] && i[1]),
        ("74S10", 12, &[1, 2, 13], |i| !(i[0] && i[1] && i[2])),
        ("74S11", 12, &[1, 2, 13], |i| i[0] && i[1] && i[2]),
        ("74S20", 6, &[1, 2, 4, 5], |i| !(i[0] && i[1] && i[2] && i[3])),
        ("74S32", 3, &[1, 2], |i| i[0] || i[1]),
        ("74S37", 3, &[1, 2], |i| !(i[0] && i[1])),
        ("74S51", 6, &[2, 3, 4, 5], |i| !((i[0] && i[1]) || (i[2] && i[3]))),
        ("74S51", 8, &[9, 10, 13, 1], |i| !((i[0] && i[1]) || (i[2] && i[3]))),
        ("74S86", 3, &[1, 2], |i| i[0] ^ i[1]),
        ("74S260", 5, &[1, 2, 3, 12, 13], |i| !i.iter().any(|&b| b)),
        ("74S133", 9, &[1, 2, 3, 4, 5, 6, 7, 10, 11, 12, 13, 14, 15], |i| !i.iter().all(|&b| b)),
        ("74S64", 8, &[9, 10, 3, 2, 6, 5, 4, 13, 12, 11, 1], |i| {
            !((i[0] && i[1])
                || (i[2] && i[3])
                || (i[4] && i[5] && i[6])
                || (i[7] && i[8] && i[9] && i[10]))
        }),
        ("9S42-1", 7, &[1, 2, 3, 4, 5, 6], |i| (i[0] && i[1]) || (i[2] && i[3] && i[4] && i[5])),
        ("93S48", 9, &[1, 2, 3, 4, 5, 6, 7, 11, 12, 13, 14, 15], |i| {
            i.iter().filter(|&&b| b).count() % 2 == 1
        }),
        ("74S280", 6, &[1, 2, 4, 8, 9, 10, 11, 12, 13], |i| {
            i.iter().filter(|&&b| b).count() % 2 == 1
        }),
    ];
    for &(kind, pin, ins, want) in cases {
        let b = behaviour(kind);
        for n in 0..1u32 << ins.len() {
            let vals: Vec<bool> = (0..ins.len()).map(|k| n >> k & 1 != 0).collect();
            let p = drive(&ins.iter().copied().zip(vals.iter().copied()).collect::<Vec<_>>());
            assert_eq!(
                out(&b, pin, &p, &s),
                Level::from(want(&vals)),
                "{kind} pin {pin} with {vals:?}"
            );
        }
    }
}

/// Six-bit identity comparator, enabled by pin 7.
#[test]
fn comparator_93s46() {
    let b = behaviour("93S46");
    let s = State::default();
    let a = [1u8, 3, 5, 10, 12, 14];
    let bp = [2u8, 4, 6, 11, 13, 15];
    for x in 0..64u8 {
        for y in 0..64u8 {
            let mut vals = vec![(7u8, true)];
            for k in 0..6 {
                vals.push((a[k], x >> k & 1 != 0));
                vals.push((bp[k], y >> k & 1 != 0));
            }
            let p = drive(&vals);
            assert_eq!(out(&b, 9, &p, &s), Level::from(x == y), "{x} vs {y}");
            vals[0] = (7, false);
            let p = drive(&vals);
            assert_eq!(out(&b, 9, &p, &s), Level::Low, "disabled");
        }
    }
}

/// One output low at a time, and only when enabled.
#[test]
fn decoders_select_one_line() {
    let s = State::default();
    let b = behaviour("74S138");
    let outs = [15u8, 14, 13, 12, 11, 10, 9, 7];
    for sel in 0..8u8 {
        for g1 in [false, true] {
            for g2a in [false, true] {
                let p = drive(&[
                    (1, sel & 1 != 0),
                    (2, sel & 2 != 0),
                    (3, sel & 4 != 0),
                    (4, false),
                    (5, g2a),
                    (6, g1),
                ]);
                let enabled = g1 && !g2a;
                for (n, &pin) in outs.iter().enumerate() {
                    let want = !(enabled && n as u8 == sel);
                    assert_eq!(out(&b, pin, &p, &s), Level::from(want), "sel {sel} pin {pin}");
                }
            }
        }
    }
    let b = behaviour("74S139");
    for sel in 0..4u8 {
        for enabled in [false, true] {
            let p = drive(&[(1, !enabled), (2, sel & 1 != 0), (3, sel & 2 != 0)]);
            for (n, pin) in [4u8, 5, 6, 7].iter().enumerate() {
                let want = !(enabled && n as u8 == sel);
                assert_eq!(out(&b, *pin, &p, &s), Level::from(want));
            }
        }
    }
}

/// The multiplexers, exhaustively over select and data.
#[test]
fn multiplexers_pick_the_right_input() {
    let s = State::default();
    let b = behaviour("74S151");
    let data = [4u8, 3, 2, 1, 15, 14, 13, 12];
    for sel in 0..8u8 {
        for word in 0..256u32 {
            let mut vals =
                vec![(7u8, false), (11, sel & 1 != 0), (10, sel & 2 != 0), (9, sel & 4 != 0)];
            for (n, &pin) in data.iter().enumerate() {
                vals.push((pin, word >> n & 1 != 0));
            }
            let p = drive(&vals);
            let want = word >> sel & 1 != 0;
            assert_eq!(out(&b, 5, &p, &s), Level::from(want), "sel {sel} data {word:#x}");
            assert_eq!(out(&b, 6, &p, &s), Level::from(!want));
        }
    }
    // The strobe forces the true output low, not high impedance.
    let p = drive(&[(7, true), (4, true), (11, false), (10, false), (9, false)]);
    assert_eq!(out(&b, 5, &p, &s), Level::Low);
    assert_eq!(out(&b, 6, &p, &s), Level::High);

    // The '157 is totem pole and forces low; the '258 inverts and goes high
    // impedance.
    let b157 = behaviour("74S157");
    let b258 = behaviour("74S258");
    for sel in [false, true] {
        for a in [false, true] {
            for bb in [false, true] {
                let p = drive(&[(1, sel), (15, false), (14, a), (13, bb)]);
                let want = if sel { bb } else { a };
                assert_eq!(out(&b157, 12, &p, &s), Level::from(want));
                assert_eq!(out(&b258, 12, &p, &s), Level::from(!want));
                let p = drive(&[(1, sel), (15, true), (14, a), (13, bb)]);
                assert_eq!(out(&b157, 12, &p, &s), Level::Low);
                assert_eq!(out(&b258, 12, &p, &s), Level::Z);
            }
        }
    }
}

/// The four-bit adder, exhaustively.
#[test]
fn adder_74s283() {
    let b = behaviour("74S283");
    let s = State::default();
    let ap = [5u8, 3, 14, 12];
    let bp = [6u8, 2, 15, 11];
    let sp = [4u8, 1, 13, 10];
    for x in 0..16u8 {
        for y in 0..16u8 {
            for ci in [false, true] {
                let mut vals = vec![(7u8, ci)];
                for k in 0..4 {
                    vals.push((ap[k], x >> k & 1 != 0));
                    vals.push((bp[k], y >> k & 1 != 0));
                }
                let p = drive(&vals);
                let want = x as u16 + y as u16 + ci as u16;
                for (k, &pin) in sp.iter().enumerate() {
                    assert_eq!(out(&b, pin, &p, &s), Level::from(want >> k & 1 != 0));
                }
                assert_eq!(out(&b, 9, &p, &s), Level::from(want & 16 != 0), "{x}+{y}+{ci}");
            }
        }
    }
}

/// The four-bit shifter, exhaustively over its ten inputs and both enables.
#[test]
fn shifter_25s10() {
    let b = behaviour("25S10");
    let s = State::default();
    // Pins 1 to 7 carry i-3 through i3.
    for word in 0..128u32 {
        for shift in 0..4u8 {
            let mut vals = vec![(13u8, false), (9, shift & 2 != 0), (10, shift & 1 != 0)];
            for k in 0..7u8 {
                vals.push((k + 1, word >> k & 1 != 0));
            }
            let p = drive(&vals);
            for n in 0..4u32 {
                let src = 3 + n - shift as u32;
                let want = word >> src & 1 != 0;
                let pin = [15u8, 14, 12, 11][n as usize];
                assert_eq!(out(&b, pin, &p, &s), Level::from(want), "{word:#x} >> {shift}");
            }
        }
    }
    let p = drive(&[(13, true)]);
    assert_eq!(out(&b, 15, &p, &s), Level::Z);
}

/// The buffers: which enable belongs to which half, and which type inverts.
#[test]
fn buffers_enable_the_right_half() {
    let s = State::default();
    for (kind, invert, b_active_high) in
        [("74S240", true, false), ("74S241", false, true), ("74LS244", false, false)]
    {
        let b = behaviour(kind);
        for data in [false, true] {
            // A half: input 2, output 18, enable 1 active low.
            let p = drive(&[(1, false), (19, !b_active_high), (2, data), (17, data)]);
            assert_eq!(out(&b, 18, &p, &s), Level::from(data != invert), "{kind} A on");
            assert_eq!(out(&b, 3, &p, &s), Level::Z, "{kind} B off");
            // B half: input 17, output 3.
            let p = drive(&[(1, true), (19, b_active_high), (2, data), (17, data)]);
            assert_eq!(out(&b, 18, &p, &s), Level::Z, "{kind} A off");
            assert_eq!(out(&b, 3, &p, &s), Level::from(data != invert), "{kind} B on");
        }
    }
}

// ---------------------------------------------------------------------------
// The ALU
// ---------------------------------------------------------------------------

/// One 74S181 slice, wired the way the board does.
fn slice(a: u8, b: u8, sel: u8, logic: bool, cnb: bool) -> (u8, bool, bool, bool, bool) {
    let bh = behaviour("74S181");
    let s = State::default();
    let mut vals = Vec::new();
    for (k, &pin) in [2u8, 23, 21, 19].iter().enumerate() {
        vals.push((pin, a >> k & 1 != 0));
    }
    for (k, &pin) in [1u8, 22, 20, 18].iter().enumerate() {
        vals.push((pin, b >> k & 1 != 0));
    }
    for (k, &pin) in [6u8, 5, 4, 3].iter().enumerate() {
        vals.push((pin, sel >> k & 1 != 0));
    }
    vals.push((8, logic));
    vals.push((7, cnb));
    let p = drive(&vals);
    let f = [9u8, 10, 11, 13]
        .iter()
        .enumerate()
        .fold(0u8, |w, (k, &pin)| w | ((out(&bh, pin, &p, &s) == Level::High) as u8) << k);
    let got = |pin| out(&bh, pin, &p, &s) == Level::High;
    (f, got(16), got(14), got(15), got(17))
}

/// P and G take neither the mode nor the carry, and the carry out does not
/// take the mode. That is what keeps the carry-lookahead out of a loop, and
/// what lets the levelizer work, so it is checked here over every input.
#[test]
fn alu_propagate_and_generate_ignore_mode_and_carry() {
    for a in 0..16u8 {
        for b in 0..16u8 {
            for sel in 0..16u8 {
                let (_, cn4, _, x, y) = slice(a, b, sel, false, false);
                for logic in [false, true] {
                    for cnb in [false, true] {
                        let got = slice(a, b, sel, logic, cnb);
                        assert_eq!((got.3, got.4), (x, y), "P/G moved: {a} {b} {sel}");
                        if !cnb {
                            assert_eq!(got.1, cn4, "carry out took the mode: {a} {b} {sel}");
                        }
                    }
                }
            }
        }
    }
}

/// The 74S181 array, checked against [`muir::ttl::alu`].
///
/// Nine slices with the carry rippled through, which is what the board has
/// once the carry-lookahead has settled. `ttl::alu` is written from the
/// datasheet function table and this from the gate model, so agreement is
/// worth something.
fn ripple(m: u32, a: u32, aluf: u8, logic: bool, cin: bool) -> (u64, bool) {
    let mut f = 0u64;
    let mut cnb = !cin;
    let mut aeqm = true;
    for n in 0..9 {
        // The ninth slice sees bit 31 of both operands again, which is how
        // the board sign-extends to 33 bits.
        let (x, y) = if n == 8 {
            ((m >> 31 & 1) as u8 * 0xf, (a >> 31 & 1) as u8 * 0xf)
        } else {
            ((m >> (4 * n) & 0xf) as u8, (a >> (4 * n) & 0xf) as u8)
        };
        let (nibble, cn4b, aeb, _, _) = slice(x, y, aluf, logic, cnb);
        f |= (nibble as u64) << (4 * n);
        if n < 8 {
            aeqm &= aeb;
        }
        cnb = cn4b;
    }
    (f & ((1 << 33) - 1), aeqm)
}

#[test]
fn alu_array_agrees_with_the_function_table() {
    let words: [u32; 8] =
        [0, 0xffff_ffff, 0xa5a5_a5a5, 0x5a5a_5a5a, 1, 0x8000_0000, 0x7fff_ffff, 0x0123_4567];
    for &m in &words {
        for &a in &words {
            for aluf in 0..16u8 {
                for logic in [false, true] {
                    for cin in [false, true] {
                        let want = ttl::alu(m, a, aluf, logic, cin);
                        let got = ripple(m, a, aluf, logic, cin);
                        assert_eq!(
                            got.0, want.f,
                            "f: m={m:#x} a={a:#x} s={aluf:x} logic={logic} cin={cin}"
                        );
                        assert_eq!(got.1, want.aeqm, "aeqm: m={m:#x} a={a:#x} s={aluf:x}");
                    }
                }
            }
        }
    }
}

/// The 74S182 must produce the carries the ripple chain would, for every
/// combination of the P and G a group of four slices can present.
#[test]
fn lookahead_agrees_with_ripple() {
    let bh = behaviour("74S182");
    let s = State::default();
    for a in 0..16u32 {
        for b in 0..16u32 {
            for sel in 0..16u8 {
                for cin in [false, true] {
                    // Four slices, rippled, recording each slice's carry in.
                    let mut cnb = !cin;
                    let mut want = Vec::new();
                    let mut pb = 0u8;
                    let mut gb = 0u8;
                    for n in 0..4 {
                        let x = (a >> n & 1) as u8 * 0xf;
                        let y = (b >> n & 1) as u8 * 0xf;
                        let (_, cn4b, _, p, g) = slice(x, y, sel, false, cnb);
                        pb |= (p as u8) << n;
                        gb |= (g as u8) << n;
                        cnb = cn4b;
                        want.push(cn4b);
                    }
                    let vals: Vec<(u8, bool)> = [4u8, 2, 15, 6]
                        .iter()
                        .enumerate()
                        .map(|(n, &pin)| (pin, pb >> n & 1 != 0))
                        .chain(
                            [3u8, 1, 14, 5]
                                .iter()
                                .enumerate()
                                .map(|(n, &pin)| (pin, gb >> n & 1 != 0)),
                        )
                        .chain(std::iter::once((13u8, !cin)))
                        .collect();
                    let p = drive(&vals);
                    let got = |pin| out(&bh, pin, &p, &s) == Level::High;
                    assert_eq!(got(12), want[0], "cn+x: a={a} b={b} sel={sel} cin={cin}");
                    assert_eq!(got(11), want[1], "cn+y");
                    assert_eq!(got(9), want[2], "cn+z");
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The parts that remember
// ---------------------------------------------------------------------------

/// Runs an update: `now` becomes the pins, `prev` what they were.
fn step(b: &Behaviour, st: &mut State, prev: &Pins, now: &Pins) {
    (b.update.unwrap())(now, prev, st);
}

#[test]
fn register_74s374_latches_on_the_rising_edge() {
    let b = behaviour("74S374");
    let mut st = State::default();
    let d = [3u8, 4, 7, 8, 13, 14, 17, 18];
    let q = [2u8, 5, 6, 9, 12, 15, 16, 19];
    let set = |word: u8, clk: bool| {
        let mut vals = vec![(1u8, false), (11, clk)];
        for (n, &pin) in d.iter().enumerate() {
            vals.push((pin, word >> n & 1 != 0));
        }
        drive(&vals)
    };
    let low = set(0xa5, false);
    let high = set(0xa5, true);
    step(&b, &mut st, &low, &high);
    for (n, &pin) in q.iter().enumerate() {
        assert_eq!(out(&b, pin, &high, &st), Level::from(0xa5u8 >> n & 1 != 0));
    }
    // Nothing moves without an edge.
    let other = set(0x5a, true);
    step(&b, &mut st, &high, &other);
    assert_eq!(st.bits, 0xa5);
    // Output enable high floats every pin.
    let mut off = other;
    off[1] = Level::High;
    assert_eq!(out(&b, 19, &off, &st), Level::Z);
}

#[test]
fn latch_74s373_is_transparent_while_pin_11_is_high() {
    let b = behaviour("74S373");
    let mut st = State::default();
    let open = drive(&[(1, false), (11, true), (3, true), (4, true)]);
    step(&b, &mut st, &open, &open);
    assert_eq!(st.bits & 3, 3);
    let shut = drive(&[(1, false), (11, false), (3, false), (4, false)]);
    step(&b, &mut st, &open, &shut);
    assert_eq!(st.bits & 3, 3, "held");
}

#[test]
fn flip_flop_74s74_clears_and_presets() {
    let b = behaviour("74S74");
    let mut st = State::default();
    let idle = drive(&[(1, true), (4, true), (3, false), (2, true)]);
    let clk = drive(&[(1, true), (4, true), (3, true), (2, true)]);
    step(&b, &mut st, &idle, &clk);
    assert_eq!(out(&b, 5, &clk, &st), Level::High);
    assert_eq!(out(&b, 6, &clk, &st), Level::Low);
    // Clear and preset are asynchronous: no clock edge, only a level, and
    // the update latches them. The outputs follow the latched state, not
    // the pin --- `part::ff` says why --- so the state is stepped first.
    let clear = drive(&[(1, false), (4, true), (3, false), (2, true)]);
    step(&b, &mut st, &clk, &clear);
    assert_eq!(st.bits & 1, 0, "clear is asynchronous");
    assert_eq!(out(&b, 5, &clear, &st), Level::Low);
    let preset = drive(&[(1, true), (4, false), (3, false), (2, true)]);
    step(&b, &mut st, &clear, &preset);
    assert_eq!(st.bits & 1, 1, "preset is asynchronous");
    assert_eq!(out(&b, 5, &preset, &st), Level::High);
    // And it stays latched once the preset is released.
    let released = drive(&[(1, true), (4, true), (3, false), (2, false)]);
    step(&b, &mut st, &preset, &released);
    assert_eq!(out(&b, 5, &released, &st), Level::High, "latched after the pulse");
    let both = drive(&[(1, false), (4, false)]);
    assert_eq!(out(&b, 5, &both, &st), Level::High);
    assert_eq!(out(&b, 6, &both, &st), Level::High, "both outputs high");
}

/// **The 74LS112 is a J-K flip-flop with a true K.** The SN74LS112A's
/// function table: on the falling clock edge J high with K low sets, both
/// low hold, J low with K high resets, both high toggle; preset and clear
/// are asynchronous. The one on the bus interface is REQERR 0A03, the `XB
/// PAR ERROR` flag (`data/BUSINT.netlist`): its second flop has K, pin 12,
/// grounded, J from the 74S11 at 0B05 and `XBUS REQUEST` for its clock,
/// with `-RESET ERR` on its clear. With a true K the flag sets on a bad
/// request and holds through good ones until `-RESET ERR`, as the board's
/// other error flags do; with a complemented K it would clear itself on the
/// next request.
#[test]
fn flip_flop_74ls112_has_a_true_k_and_holds_with_both_low() {
    let b = behaviour("74LS112-1");
    let mut st = State::default();
    // Flop 2: CLK 13, K 12, J 11, -PRE 10, Q 9, -Q 7, -CLR 14.
    let at =
        |clk: bool, j: bool, k: bool| drive(&[(13, clk), (11, j), (12, k), (10, true), (14, true)]);
    step(&b, &mut st, &at(true, true, false), &at(false, true, false));
    assert_eq!(out(&b, 9, &at(false, true, false), &st), Level::High, "J alone sets");
    step(&b, &mut st, &at(true, false, false), &at(false, false, false));
    assert_eq!(out(&b, 9, &at(false, false, false), &st), Level::High, "both low hold");
    step(&b, &mut st, &at(true, false, true), &at(false, false, true));
    assert_eq!(out(&b, 9, &at(false, false, true), &st), Level::Low, "K alone resets");
    step(&b, &mut st, &at(true, true, true), &at(false, true, true));
    assert_eq!(out(&b, 9, &at(false, true, true), &st), Level::High, "both high toggle");
    step(&b, &mut st, &at(true, true, true), &at(false, true, true));
    assert_eq!(out(&b, 9, &at(false, true, true), &st), Level::Low, "and toggle again");
    assert_eq!(out(&b, 7, &at(false, true, true), &st), Level::High, "-Q is the complement");
    // The clear is asynchronous: set, then clear with the clock still.
    step(&b, &mut st, &at(true, true, false), &at(false, true, false));
    let cleared = drive(&[(13, false), (11, true), (12, false), (10, true), (14, false)]);
    step(&b, &mut st, &at(false, true, false), &cleared);
    assert_eq!(out(&b, 9, &cleared, &st), Level::Low, "-CLR clears");
    // Flop 1 the same way: CLK 1, K 2, J 3, -PRE 4, Q 5, -CLR 15.
    let one =
        |clk: bool, j: bool, k: bool| drive(&[(1, clk), (3, j), (2, k), (4, true), (15, true)]);
    step(&b, &mut st, &one(true, true, false), &one(false, true, false));
    assert_eq!(out(&b, 5, &one(false, true, false), &st), Level::High, "flop 1 sets");
    step(&b, &mut st, &one(true, false, false), &one(false, false, false));
    assert_eq!(out(&b, 5, &one(false, false, false), &st), Level::High, "flop 1 holds");
}

#[test]
fn counter_74s169_counts_both_ways_and_carries() {
    let b = behaviour("74S169");
    let mut st = State::default();
    // Load 14, then count up: 15 raises the carry, then it wraps to 0.
    let load = |clk: bool| {
        drive(&[
            (1, true),
            (2, clk),
            (9, false),
            (7, false),
            (10, false),
            (3, false),
            (4, true),
            (5, true),
            (6, true),
        ])
    };
    step(&b, &mut st, &load(false), &load(true));
    assert_eq!(st.bits & 0xf, 14);
    let count = |clk: bool| drive(&[(1, true), (2, clk), (9, true), (7, false), (10, false)]);
    step(&b, &mut st, &count(false), &count(true));
    assert_eq!(st.bits & 0xf, 15);
    assert_eq!(out(&b, 15, &count(true), &st), Level::Low, "carry at terminal count");
    step(&b, &mut st, &count(false), &count(true));
    assert_eq!(st.bits & 0xf, 0);
    // Counting down from zero wraps and raises the carry there instead.
    let down = |clk: bool| drive(&[(1, false), (2, clk), (9, true), (7, false), (10, false)]);
    assert_eq!(out(&b, 15, &down(false), &st), Level::Low);
    step(&b, &mut st, &down(false), &down(true));
    assert_eq!(st.bits & 0xf, 15);
}

#[test]
fn shift_register_74s194_shifts_the_datasheet_way() {
    let b = behaviour("74S194");
    let mut st = State::default();
    // Load 0b0001 --- QA set, QB to QD clear.
    let load = |clk: bool| {
        drive(&[
            (1, true),
            (11, clk),
            (9, true),
            (10, true),
            (3, true),
            (4, false),
            (5, false),
            (6, false),
        ])
    };
    step(&b, &mut st, &load(false), &load(true));
    assert_eq!(st.bits & 0xf, 1);
    assert_eq!(out(&b, 15, &load(true), &st), Level::High, "QA is pin 15");
    // Shift right moves QA into QB, with pin 2 entering QA.
    let right = |clk: bool| drive(&[(1, true), (11, clk), (9, true), (10, false), (2, false)]);
    step(&b, &mut st, &right(false), &right(true));
    assert_eq!(st.bits & 0xf, 0b0010);
    assert_eq!(out(&b, 14, &right(true), &st), Level::High, "QB is pin 14");
    // Shift left moves it back, with pin 7 entering QD.
    let left = |clk: bool| drive(&[(1, true), (11, clk), (9, false), (10, true), (7, false)]);
    step(&b, &mut st, &left(false), &left(true));
    assert_eq!(st.bits & 0xf, 0b0001);
}

/// Where the board takes the '194's serial inputs from is what fixes which
/// direction is which: on the Q register the pin 2 input of one slice is the
/// pin 12 output of the slice below, never its pin 15.
#[test]
fn the_194_serial_input_comes_from_the_slice_below() {
    let n = load();
    let drivers: BTreeMap<u32, (String, u8)> = n
        .parts
        .iter()
        .filter(|p| part::strip(&p.kind).0 == "74S194")
        .flat_map(|p| {
            p.pins.iter().filter_map(move |&(pin, net)| {
                [12u8, 13, 14, 15].contains(&pin).then_some((net, (p.reference.clone(), pin)))
            })
        })
        .collect();
    let mut checked = 0;
    for p in n.parts.iter().filter(|p| part::strip(&p.kind).0 == "74S194") {
        let Some(&(_, net)) = p.pins.iter().find(|&&(pin, _)| pin == 2) else { continue };
        if let Some((who, pin)) = drivers.get(&net) {
            assert_eq!(
                *pin, 12,
                "{} takes its shift-right input from {who} pin {pin}",
                p.reference
            );
            checked += 1;
        }
    }
    assert!(checked >= 4, "only {checked} slices chained");
}

/// The 9328's two clocks are ORed, which is how the board uses the separate
/// clock as an inhibit and the common one on pin 9 as the clock.
#[test]
fn shift_register_9328_uses_the_common_clock() {
    let b = behaviour("9328");
    let mut st = State::default();
    // Inhibit low, common clock rising: the A register shifts a one in.
    let a = |common: bool| drive(&[(1, true), (7, false), (9, common), (4, false), (6, true)]);
    step(&b, &mut st, &a(false), &a(true));
    assert_eq!(st.bits & 0xff, 1);
    // Inhibit high: the OR is already high, so the common clock does nothing.
    let held = |common: bool| drive(&[(1, true), (7, true), (9, common), (4, false), (6, true)]);
    step(&b, &mut st, &held(false), &held(true));
    assert_eq!(st.bits & 0xff, 1, "inhibited");
    // Eight shifts put the first bit on Q7.
    for _ in 0..7 {
        step(&b, &mut st, &a(false), &a(true));
    }
    assert_eq!(out(&b, 3, &a(true), &st), Level::High, "Q7");
    assert_eq!(out(&b, 2, &a(true), &st), Level::Low, "-Q7");
    let clear = drive(&[(1, false)]);
    assert_eq!(out(&b, 3, &clear, &st), Level::Low, "clear is asynchronous");
}

/// Every 9328 on the board has a live signal on pin 9, so a model that drops
/// it cannot be right.
#[test]
fn every_9328_has_a_signal_on_its_common_clock() {
    let n = load();
    let mut seen = 0;
    for p in n.parts.iter().filter(|p| p.kind == "9328") {
        let &(_, net) = p.pins.iter().find(|&&(pin, _)| pin == 9).expect("pin 9 unconnected");
        let name = n.net(net);
        assert!(
            !matches!(name, "GND" | "VCC") && !name.starts_with("HI") && !name.starts_with("NC#"),
            "{} pin 9 is tied to {name}",
            p.reference
        );
        assert_ne!(
            net,
            p.pins.iter().find(|&&(pin, _)| pin == 7).unwrap().1,
            "{} shares its clocks",
            p.reference
        );
        seen += 1;
    }
    assert_eq!(seen, 7);
}

#[test]
fn ram_2147_reads_back_what_was_written() {
    let b = behaviour("2147");
    let mut st = State { bits: 0, cells: vec![0; 4096] };
    let addr = [1u8, 2, 3, 4, 5, 6, 17, 16, 15, 14, 13, 12];
    let at = |a: u32, ce: bool, we: bool, di: bool| {
        let mut vals = vec![(10u8, !ce), (8, !we), (11, di)];
        for (k, &pin) in addr.iter().enumerate() {
            vals.push((pin, a >> k & 1 != 0));
        }
        drive(&vals)
    };
    // A write pulse: the cell takes the value the pins had inside it.
    let during = at(0x123, true, true, true);
    let after = at(0x123, true, false, true);
    step(&b, &mut st, &during, &after);
    assert_eq!(st.cells[0x123], 1);
    assert_eq!(out(&b, 7, &after, &st), Level::High);
    // Deselected, and mid-write, the output floats.
    assert_eq!(out(&b, 7, &at(0x123, false, false, false), &st), Level::Z);
    assert_eq!(out(&b, 7, &during, &st), Level::Z, "high impedance while writing");
    assert_eq!(out(&b, 7, &at(0x124, true, false, false), &st), Level::Low);
}

#[test]
fn ram_82s21_is_two_bits_wide_and_open_collector() {
    let b = behaviour("82S21");
    let mut st = State { bits: 0, cells: vec![0; 32] };
    let addr = [13u8, 12, 11, 10, 4];
    let at = |a: u32, write: bool, d0: bool, d1: bool| {
        let mut vals =
            vec![(5u8, true), (6, true), (1, !write), (2, false), (15, false), (3, d0), (14, d1)];
        for (k, &pin) in addr.iter().enumerate() {
            vals.push((pin, a >> k & 1 != 0));
        }
        drive(&vals)
    };
    let during = at(9, true, true, false);
    let after = at(9, false, true, false);
    step(&b, &mut st, &during, &after);
    assert_eq!(st.cells[9], 1);
    // A one is a logic one here; the drive record turns it into nothing
    // driven, and the RES20 packs pull the net up.
    assert_eq!(out(&b, 7, &after, &st), Level::High);
    assert_eq!(out(&b, 9, &after, &st), Level::Low);
    let mut off = after;
    off[5] = Level::Low;
    assert_eq!(out(&b, 7, &off, &st), Level::Z, "deselected");
}

#[test]
fn prom_74s472_reads_its_contents() {
    let b = behaviour("74S472");
    let mut cells = vec![0u8; 512];
    cells[0x1ff] = 0x81;
    let st = State { bits: 0, cells };
    let addr = [1u8, 2, 3, 4, 5, 16, 17, 18, 19];
    let at = |a: u32, ce: bool| {
        let mut vals = vec![(15u8, !ce)];
        for (k, &pin) in addr.iter().enumerate() {
            vals.push((pin, a >> k & 1 != 0));
        }
        drive(&vals)
    };
    let p = at(0x1ff, true);
    assert_eq!(out(&b, 6, &p, &st), Level::High, "d0");
    assert_eq!(out(&b, 7, &p, &st), Level::Low, "d1");
    assert_eq!(out(&b, 14, &p, &st), Level::High, "d7");
    assert_eq!(out(&b, 6, &at(0x1ff, false), &st), Level::Z);
    // An unloaded PROM reads unknown rather than zero.
    assert_eq!(out(&b, 6, &p, &State::default()), Level::X);
}

/// A count, so the README can quote one that is checked.
#[test]
fn how_much_is_modelled() {
    let n = load();
    let all = kinds(&n);
    let with = all.iter().filter(|k| part::behaviour(k).is_some()).count();
    let logic =
        all.iter().filter(|k| part::behaviour(k).is_some_and(|b| !b.gates.is_empty())).count();
    let gates: usize = all.iter().filter_map(|k| part::behaviour(k)).map(|b| b.gates.len()).sum();
    eprintln!(
        "{} part types, {with} with a behaviour, {logic} of them logic, {gates} gates",
        all.len()
    );
    assert_eq!(all.len(), 71);
    assert_eq!(with, 67);
    assert_eq!(logic, 65);
}

// ---------------------------------------------------------------------------
// Net resolution
// ---------------------------------------------------------------------------

fn d(level: Level, drive: part::Drive) -> part::Driver {
    part::Driver { level, drive }
}

/// The rules in [`part::resolve`], one case each.
#[test]
fn nets_resolve_four_ways() {
    use Level::{High, Low, X, Z};
    use part::Drive::*;
    let r = part::resolve;

    // Nothing on the net at all.
    assert_eq!(r(&[]), Z);
    assert_eq!(r(&[d(High, Passive), d(Low, Passive)]), Z, "passive pins never drive");
    assert_eq!(r(&[d(Z, TriState), d(Z, TriState)]), Z, "all outputs off");

    // One driver decides it.
    assert_eq!(r(&[d(High, Totem)]), High);
    assert_eq!(r(&[d(Low, Totem)]), Low);
    assert_eq!(r(&[d(Low, TriState), d(Z, TriState)]), Low, "one of two enabled");

    // Open collector pulls down or lets go, so a pull-up decides the rest.
    assert_eq!(r(&[d(High, OpenCollector)]), Z, "letting go, with no pull-up");
    assert_eq!(r(&[d(High, OpenCollector), d(High, PullUp)]), High);
    assert_eq!(r(&[d(Low, OpenCollector), d(High, PullUp)]), Low, "pulled down wins");
    assert_eq!(
        r(&[d(High, OpenCollector), d(Low, OpenCollector), d(High, PullUp)]),
        Low,
        "wire-AND: one low takes the net"
    );

    // A pull-up loses to anything real.
    assert_eq!(r(&[d(Low, Totem), d(High, PullUp)]), Low);
    assert_eq!(r(&[d(High, PullUp)]), High);

    // Faults and unknowns.
    assert_eq!(r(&[d(High, Totem), d(Low, Totem)]), X, "two strong drivers disagree");
    assert_eq!(r(&[d(High, Totem), d(High, Totem)]), High, "agreeing is not a fault");
    assert_eq!(r(&[d(X, Totem)]), X);
    assert_eq!(r(&[d(Low, Totem), d(X, TriState)]), X, "unknown is not overridden");
    assert_eq!(r(&[d(X, Totem), d(High, PullUp)]), X, "a pull-up cannot rescue it");
}

/// The resistor rules in [`part::resolve`]: a driver decides the net over
/// any number of resistors whichever way they pull and wherever they come
/// in the list, resistors alone that disagree are a divider and not a
/// level, and an unknown on a resistor is an unknown on the net.
#[test]
fn resistors_yield_to_drivers_and_unknowns_win() {
    use Level::{High, Low, X, Z};
    use part::Drive::*;
    let r = part::resolve;

    // Opposing resistors with nothing stronger on the net.
    assert_eq!(r(&[d(High, PullUp), d(Low, PullUp)]), X, "a divider is not a level");
    assert_eq!(r(&[d(Low, PullUp), d(High, PullUp)]), X);

    // A driver wins over the divider, before or after it in the list.
    assert_eq!(r(&[d(High, PullUp), d(Low, PullUp), d(High, Totem)]), High);
    assert_eq!(r(&[d(High, PullUp), d(Low, PullUp), d(Low, TriState)]), Low);
    assert_eq!(r(&[d(Low, Totem), d(High, PullUp), d(Low, PullUp)]), Low);
    assert_eq!(r(&[d(High, PullUp), d(Low, PullUp), d(Low, OpenCollector)]), Low);
    assert_eq!(r(&[d(High, PullUp), d(Low, PullUp), d(High, OpenEmitter)]), High);

    // Resistors that agree are one resistor, and one whose far end is off
    // pulls nothing.
    assert_eq!(r(&[d(High, PullUp), d(High, PullUp)]), High);
    assert_eq!(r(&[d(Low, PullUp), d(Low, PullUp)]), Low);
    assert_eq!(r(&[d(Z, PullUp)]), Z);
    assert_eq!(r(&[d(Z, PullUp), d(Low, PullUp)]), Low);

    // An unknown resistor makes the net unknown like any other driver.
    assert_eq!(r(&[d(X, PullUp)]), X);
    assert_eq!(r(&[d(X, PullUp), d(High, Totem)]), X);
    assert_eq!(r(&[d(High, Totem), d(X, PullUp)]), X);
    assert_eq!(r(&[d(X, PullUp), d(High, PullUp)]), X);
}

/// Every net an open-collector output pulls down, with nothing pushing it
/// back up, needs a pull-up to ever settle at a one --- and MIT fitted them:
/// the RES20 packs in MCTL and SPC, and the SIPs on BCTERM.
///
/// The nets that have none are all driven by a single gate carrying the SUDS
/// `O` suffix, and MIT's parts list has no open-collector variant of any of
/// those types, so the plain part was fitted. With one
/// driver it makes no difference which we model: an open-collector one
/// leaves the net high impedance, and a TTL input reads that as a one.
#[test]
fn open_collector_nets_have_pull_ups_or_a_single_driver() {
    let n = load();
    let mut drives: BTreeMap<u32, Vec<part::Drive>> = BTreeMap::new();
    for p in &n.parts {
        let Some(po) = part::pinout(&p.kind) else { continue };
        for &(pin, net) in &p.pins {
            if let Some(drive) = po.drive_of(pin) {
                drives.entry(net).or_default().push(drive);
            }
        }
    }
    let (mut pulled, mut single) = (0, 0);
    let mut floating = Vec::new();
    for (net, on_it) in &drives {
        let oc = on_it.contains(&part::Drive::OpenCollector);
        let strong = on_it.iter().any(|&x| matches!(x, part::Drive::Totem | part::Drive::TriState));
        if !oc || strong {
            continue;
        }
        if on_it.contains(&part::Drive::PullUp) {
            pulled += 1;
        } else if on_it.len() == 1 {
            single += 1;
        } else {
            floating.push(n.net(*net));
        }
    }
    eprintln!("{pulled} open-collector nets pulled up, {single} with a single driver");
    assert!(pulled > 50, "expected the M memory and the stack to be pulled up");
    assert!(floating.is_empty(), "wire-ANDed with nothing to pull them up: {floating:?}");
}

// ---------------------------------------------------------------------------
// The Trident cable
// ---------------------------------------------------------------------------

/// **The SN75107's function table, row for row.**
///
/// `sn75107.pdf`, SLLS069D, page 2: the output is low **only** when the
/// differential input is below its threshold the wrong way round and both
/// strobes are high. Everything else on the table is a high, and the one
/// row the sheet calls indeterminate --- the inputs within 25 mV of each
/// other with both strobes up --- is what [`Level::X`] is for.
///
/// Pin numbers are the sheet's: 1A 1, 1B 2, 1Y 4, 1G 5, S 6, 2G 8, 2Y 9,
/// 2B 11, 2A 12.
#[test]
fn line_receiver_75107() {
    let b = behaviour("75107");
    let s = State::default();
    let y = |a: bool, bb: bool, g: bool, st: bool| {
        out(&b, 4, &drive(&[(1, a), (2, bb), (5, g), (6, st)]), &s)
    };
    // A above B: high whatever the strobes do.
    for (g, st) in [(false, false), (false, true), (true, false), (true, true)] {
        assert_eq!(y(true, false, g, st), Level::High, "A>B, G={g} S={st}");
    }
    // A below B: low, and only, when both strobes are up.
    assert_eq!(y(false, true, true, true), Level::Low, "A<B, strobed");
    assert_eq!(y(false, true, false, true), Level::High, "A<B, G down");
    assert_eq!(y(false, true, true, false), Level::High, "A<B, S down");
    // The sheet's indeterminate row, and its two strobed-off neighbours.
    assert_eq!(y(true, true, true, true), Level::X, "no difference to read");
    assert_eq!(y(false, false, true, true), Level::X, "no difference to read");
    assert_eq!(y(true, true, false, true), Level::High, "G down");

    // The second channel is the same gate on 2A 12, 2B 11, 2G 8 and the
    // common S 6, which is what the disk controller reads its clock with.
    let y2 = |a: bool, bb: bool| out(&b, 9, &drive(&[(12, a), (11, bb), (8, true), (6, true)]), &s);
    assert_eq!(y2(true, false), Level::High);
    assert_eq!(y2(false, true), Level::Low);
}

/// **The SN75110's function table, row for row.**
///
/// `sn75110.pdf`, SLLS106G, page 2. The outputs are constant-current
/// **sinks** --- the sheet specifies `IO(on)` at `VO = 10 V`, and the
/// common-mode output range runs to +10 V, which only a sink can hold ---
/// so "on" pulls its line down and "off" leaves it to the terminator. That
/// is an open-collector output, and the gates below return the logic level
/// with [`part::Drive::OpenCollector`] doing the rest: a one is off.
///
/// Pin numbers are the sheet's: 1A 1, 1B 2, 1C 3, 2C 4, 2A 5, 2B 6, 2Y 8,
/// 2Z 9, D 10, 1Z 12, 1Y 13.
#[test]
fn line_driver_75110() {
    let b = behaviour("75110");
    let s = State::default();
    // Channel 2, the one the disk controller uses: A 5, B 6, C 4, D 10.
    let yz = |a: bool, bb: bool, c: bool, d: bool| {
        let p = drive(&[(5, a), (6, bb), (4, c), (10, d)]);
        (out(&b, 8, &p, &s), out(&b, 9, &p, &s))
    };
    let (on, off) = (Level::Low, Level::High);
    // C low, then D low: both off, whatever the logic inputs say.
    for (a, bb) in [(false, false), (false, true), (true, false), (true, true)] {
        assert_eq!(yz(a, bb, false, true), (off, off), "A={a} B={bb}, C down");
        assert_eq!(yz(a, bb, true, false), (off, off), "A={a} B={bb}, D down");
    }
    // Enabled: Y sinks unless both logic inputs are high, and then Z does.
    assert_eq!(yz(false, false, true, true), (on, off));
    assert_eq!(yz(false, true, true, true), (on, off), "A low");
    assert_eq!(yz(true, false, true, true), (on, off), "B low");
    assert_eq!(yz(true, true, true, true), (off, on));

    // Channel 1 is the same on A 1, B 2, C 3, the same common D, Y 13, Z 12.
    let p = drive(&[(1, true), (2, true), (3, true), (10, true)]);
    assert_eq!((out(&b, 13, &p, &s), out(&b, 12, &p, &s)), (off, on));
}

/// **A bit put on the Trident's data pair reads back as itself.**
///
/// The controller's write driver and its read receiver sit on the same two
/// nets, `TRIDENT.0.DATA.P` and `.M`, so the pair closes a loop through the
/// cable and the loop must not invert. This is the check that the two
/// function tables were read the same way up: the 75110's `2Y` is the `P`
/// line and its `2Z` the `M`, and the 75107 takes `P` on `1A` and `M` on
/// `1B`, as `dctrid.drw` wires them.
///
/// It also fixes the one thing the two sheets cannot settle between them:
/// an open-collector output driving a one is not driving, and the line is
/// then held up by its terminator, so the level a receiver reads is the
/// driver's logic level either way.
#[test]
fn the_trident_data_pair_is_a_loop() {
    let (drv, rcv) = (behaviour("75110"), behaviour("75107"));
    let s = State::default();
    for bit in [false, true] {
        // WRITE DATA B on 2B, the other logic input strapped high, write
        // gate and unit-selected both up.
        let w = drive(&[(5, true), (6, bit), (4, true), (10, true)]);
        let (p, m) = (out(&drv, 8, &w, &s), out(&drv, 9, &w, &s));
        assert_ne!(p, m, "the pair is differential");
        let r = drive(&[(1, p == Level::High), (2, m == Level::High), (5, true), (6, true)]);
        assert_eq!(out(&rcv, 4, &r, &s), Level::from(bit), "wrote {bit}, read it back");
    }
}

/// **TRITERM passes pin n to pin 17-n**, which is the only geometry the
/// board's own wiring allows.
///
/// No datasheet for it has reached us, and nothing names a vendor or a
/// value for it, so what settles it is that all eight of
/// the signals crossing it match their receivers name for name under 17-n
/// and none does under the alternative, n+8.
/// Without this the four bus-cable status lines reach no receiver at all.
#[test]
fn trident_terminator_passes_across_the_package() {
    let b = behaviour("TRITERM");
    let s = State::default();
    for (signal, receiver) in [(1u8, 16u8), (3, 14), (5, 12), (7, 10)] {
        for level in [false, true] {
            assert_eq!(
                out(&b, receiver, &drive(&[(signal, level)]), &s),
                Level::from(level),
                "pin {signal} to pin {receiver}"
            );
        }
    }
    // The even pins are the cable's ground returns and drive nothing: the
    // ground-side resistor only makes the divider, and the board jumpers its
    // far end to the signal's.
    let po = part::pinout("TRITERM").unwrap();
    assert_eq!(po.outputs, &[10, 12, 14, 16], "only the signal side drives");
}

// ---------------------------------------------------------------------------
// The Chaosnet cable
// ---------------------------------------------------------------------------

/// **The Am26LS33's differential inputs, and which of each pair is which.**
///
/// `am26ls33.pdf`, SLLS115G, the pin table: `1B` 1, `1A` 2, `1Y` 3, `G` 4,
/// `2Y` 5, `2A` 6, `2B` 7, `3B` 9, `3A` 10, `3Y` 11, `-G` 12, `4Y` 13,
/// `4B` 14, `4A` 15 --- `A` the non-inverting input of each pair and `B`
/// the inverting one, the output high where `A` is above `B`. Note the
/// first receiver: its inverting input is the **lower** pin number, and
/// the second receiver's is the higher.
///
/// MIT says the same thing in its own words for the two receivers the I/O
/// board uses. `cadrio/iob.wlr` gives A01-01 the `USE` code `-IN` and
/// A01-02 `IN`, A01-06 `IN` and A01-07 `-IN`. The two sources reached us
/// by different routes and agree pin for pin.
#[test]
fn line_receiver_26ls33() {
    let b = behaviour("26LS33");
    let s = State::default();
    // Both enables on, as LMLNDR A01 ties them: `G` 4 high, `-G` 12 low.
    let y = |o: u8, a: u8, bb: u8, above: bool| {
        out(&b, o, &drive(&[(a, above), (bb, !above), (4, true), (12, false)]), &s)
    };
    // Each receiver as (output, A, B).
    for (o, a, bb) in [(3u8, 2u8, 1u8), (5, 6, 7), (11, 10, 9), (13, 15, 14)] {
        assert_eq!(y(o, a, bb, true), Level::High, "pin {a} above pin {bb}");
        assert_eq!(y(o, a, bb, false), Level::Low, "pin {a} below pin {bb}");
    }
    // Nothing across the inputs is no difference to read, not a guess.
    for v in [false, true] {
        let p = drive(&[(1, v), (2, v), (4, true), (12, false)]);
        assert_eq!(out(&b, 3, &p, &s), Level::X, "both inputs at {v}");
    }
    // Neither enable on: the outputs are off.
    let off = drive(&[(2, true), (1, false), (4, false), (12, true)]);
    assert_eq!(out(&b, 3, &off, &s), Level::Z, "G low and -G high is disabled");
}

/// **A bit put on the Chaosnet's transmit pair reads back as itself.**
///
/// The driver and the receiver do not share nets the way the Trident's do:
/// the transmit pair goes out to the transceiver at LMDETC A03 and the
/// receive pair comes back from it. But a transceiver hears its own cable,
/// so with one board talking on a quiet ether the two pairs carry the same
/// bit, and the loop must not invert. This is the check that the driver's
/// pinout and the receiver's were read the same way up, and it is what
/// fails if either input of a pair is transposed.
///
/// MIT crossed both pairs on purpose, and `cadrio/iob.wlr` shows it twice.
/// `-TTL.D.OUT`, the active-low transmit data, is on A02-01 as `IN`, with
/// `TRANS.DATA-` on A02-02 as `OUT` and `TRANS.DATA+` on A02-03 as `-OUT`:
/// sending a one drives the input low, the true output low and the
/// complement high, which puts the plus above the minus. Plus above minus
/// is therefore a one, on this board, by MIT's own wiring. The receive pair
/// is crossed the same way --- `RCVR.DATA+` on the receiver's inverting
/// input --- so a one comes back as a low, which is why `-RCVR.DATA.IN` is
/// named active-low.
///
/// The last leg is the inverting 74S158 at LMLNDR 0E02: `-RCVR.DATA.IN` on
/// `4A` 14, `TTL.D.IN` on `4Y` 12, `LOOP.BACK` low on the select and the
/// strobe grounded. `TTL.D.IN` is the line level, active high.
#[test]
fn the_chaosnet_data_pair_is_a_loop() {
    let (drv, rcv, sel) = (behaviour("26LS31"), behaviour("26LS33"), behaviour("74S158"));
    let s = State::default();
    for bit in [false, true] {
        // `-TTL.D.OUT` on `1A` 1; `G` 4 is grounded and `-G` 12 is
        // `LOOP.BACK`, low while the board is on the cable.
        let t = drive(&[(1, !bit), (4, false), (12, false)]);
        let (minus, plus) = (out(&drv, 2, &t, &s), out(&drv, 3, &t, &s));
        assert_ne!(plus, minus, "the pair is differential");
        assert_eq!(plus == Level::High, bit, "a one puts the plus above the minus");
        // The transceiver hands them back: plus on pin 1, minus on pin 2.
        let r =
            drive(&[(1, plus == Level::High), (2, minus == Level::High), (4, true), (12, false)]);
        let back = out(&rcv, 3, &r, &s);
        assert_eq!(back == Level::Low, bit, "-RCVR.DATA.IN is asserted by a one");
        let d = drive(&[(14, back == Level::High), (13, !bit), (1, false), (15, false)]);
        assert_eq!(out(&sel, 12, &d, &s), Level::from(bit), "sent {bit}, read it back");
    }
}

/// **An idle Chaosnet reads as no data and no interference.**
///
/// [`muir::unibus::IDLE_CHAOSNET`] is what the far end holds the two
/// receive pairs at while nothing is on the cable, and it only means
/// anything alongside the receiver's pinout: the pair of them says what
/// the board hears. AI Memo 628 §2.3, "When the cable is idle it is held
/// at 0 volts by the terminations", and §2.5, "if no transceivers are
/// active, the terminations will hold the ether low". A low ether is a
/// zero, so `-RCVR.DATA.IN` must be unasserted and `INTERFERENCE IN` low.
///
/// This is the test that holds the two together. Either one alone can be
/// turned round and the board still comes up quiet, because the two errors
/// cancel; it is the pair that has to be right, and it is the pair that is
/// checked here.
#[test]
fn an_idle_chaosnet_reads_as_quiet() {
    let b = behaviour("26LS33");
    let s = State::default();
    let held = |name: &str| {
        muir::unibus::IDLE_CHAOSNET
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| panic!("IDLE_CHAOSNET does not hold {name}"))
            .1
            == Level::High
    };
    // LMLNDR A01 as `cadrio/iob.wlr` wires it, both enables on.
    let p = drive(&[
        (1, held("RCVR.DATA+")),
        (2, held("RCVR.DATA-")),
        (6, held("INTERFERE+")),
        (7, held("INTERFERE-")),
        (4, true),
        (12, false),
    ]);
    assert_eq!(out(&b, 3, &p, &s), Level::High, "-RCVR.DATA.IN not asserted");
    assert_eq!(out(&b, 5, &p, &s), Level::Low, "INTERFERENCE IN not asserted");
}

/// **The 67401 FIFO, by its datasheet's functional description.**
///
/// `67401.pdf`, "Functional Description": data goes in on D0-D3 when
/// `SHIFT IN` is brought high and `INPUT READY` is high; SI high forces IR
/// low, and dropping SI raises it again unless the memory is full. Data is
/// read from O0-O3 while `OUTPUT READY` is high; SO high forces OR low and
/// holds the data, and dropping SO brings the next word to the output stage,
/// OR staying low if there is none. `MR` is overbarred on the pin
/// configuration, so master reset is **active low**, which is also the only
/// reading that makes sense of the board: DCRBUF and DCWBUF put `MBUSY`
/// there, holding the buffers clear whenever the memory side is idle.
///
/// Four of these are the read and write buffers, and every byte of a
/// transfer crosses one.
#[test]
fn fifo_67401_is_first_in_first_out() {
    let b = behaviour("67401");
    let mut st = State { cells: vec![0; 64], ..State::default() };
    let (si, so, mr) = (3u8, 15u8, 9u8);
    let d = [4u8, 5, 6, 7];
    let o = [13u8, 12, 11, 10];
    let pins = |nibble: u8, shift_in: bool, shift_out: bool, reset: bool| {
        let mut vals = vec![(si, shift_in), (so, shift_out), (mr, reset)];
        for (k, &pin) in d.iter().enumerate() {
            vals.push((pin, nibble >> k & 1 != 0));
        }
        drive(&vals)
    };
    let idle = pins(0, false, false, true);
    let read = |st: &State, p: &Pins| -> u8 {
        o.iter()
            .enumerate()
            .fold(0, |w, (k, &pin)| w | ((out(&b, pin, p, st) == Level::High) as u8) << k)
    };
    let ready = |st: &State, p: &Pins| {
        (out(&b, 2, p, st) == Level::High, out(&b, 14, p, st) == Level::High)
    };

    // Master reset, active low: empty, so ready to take and nothing to give.
    step(&b, &mut st, &idle, &pins(0, false, false, false));
    assert_eq!(ready(&st, &idle), (true, false), "reset leaves it empty");

    // Three nibbles in, and the order they come out in is the order they
    // went in. The data is put up first and SI pulsed after it, which is
    // what the board does --- the shift register's outputs settle and
    // `RBUF.ICLK` follows --- and what `Update`'s "an edge captures `prev`"
    // means.
    for (k, &word) in [0b1010u8, 0b0101, 0b1111].iter().enumerate() {
        let (low, high) = (pins(word, false, false, true), pins(word, true, false, true));
        assert!(ready(&st, &low).0, "room before word {k}");
        step(&b, &mut st, &low, &high);
        assert!(!ready(&st, &high).0, "SI high pulls INPUT READY down");
        step(&b, &mut st, &high, &low);
    }
    assert_eq!(ready(&st, &idle), (true, true), "three in, room for more");
    assert_eq!(read(&st, &idle), 0b1010, "the first word is at the output");

    for (k, &want) in [0b1010u8, 0b0101, 0b1111].iter().enumerate() {
        assert_eq!(read(&st, &idle), want, "word {k} at the output stage");
        assert!(ready(&st, &idle).1, "OUTPUT READY before word {k}");
        let high = pins(0, false, true, true);
        step(&b, &mut st, &idle, &high);
        assert!(!ready(&st, &high).1, "SO high pulls OUTPUT READY down");
        assert_eq!(read(&st, &high), want, "data is held while SO is high");
        step(&b, &mut st, &high, &idle);
    }
    assert!(!ready(&st, &idle).1, "emptied, OUTPUT READY stays low");

    // Sixty-four deep, and the sixty-fifth finds no room.
    for k in 0..64u8 {
        let (low, high) = (pins(k & 0xf, false, false, true), pins(k & 0xf, true, false, true));
        assert!(ready(&st, &low).0, "room for word {k}");
        step(&b, &mut st, &low, &high);
        step(&b, &mut st, &high, &low);
    }
    assert_eq!(ready(&st, &idle), (false, true), "full: INPUT READY stays low");
    let (low, high) = (pins(0xf, false, false, true), pins(0xf, true, false, true));
    step(&b, &mut st, &low, &high);
    step(&b, &mut st, &high, &low);
    assert_eq!(read(&st, &idle), 0, "the first of the sixty-four is still first");

    // And master reset empties it again.
    step(&b, &mut st, &idle, &pins(0, false, false, false));
    assert_eq!(ready(&st, &idle), (true, false), "reset empties a full one");
}

/// **The Am25LS2536 is its function table.** The select and polarity
/// enter the register on the clock's rise while `-CE` is low, `-CLR`
/// sets it low at once, `G` is `-G1` low with `G2` high, the selected
/// output is `-G` exclusive-or the polarity bit and the others are the
/// polarity bit inverted, and `-OE` high turns all eight off. On the disk
/// controller the four low outputs are `INC BLOCK^`, `INC HEAD^`, `INC
/// CYL^` and `-END OF DISK`, and the polarity is grounded, so the one
/// selected by the header's next-block code pulses low under
/// `FUNC/INCREMENT ADDRESS`.
#[test]
fn the_2536_is_its_function_table() {
    let b = behaviour("25LS2536");
    let outs = [9u8, 11, 12, 13, 14, 15, 16, 17];
    let ys =
        |p: &Pins, s: &State| -> Vec<Level> { outs.iter().map(|&y| out(&b, y, p, s)).collect() };
    let h = Level::High;
    let l = Level::Low;
    let mut st = State::default();
    // Cleared: register low. With G true the selected output, 0, is low.
    let idle = drive(&[
        (1, false),
        (2, false),
        (3, true),
        (4, false),
        (5, false),
        (6, false),
        (7, false),
        (8, false),
        (19, true),
        (18, true),
    ]);
    step(&b, &mut st, &idle, &idle);
    assert_eq!(st.bits, 0);
    assert_eq!(ys(&idle, &st), vec![h; 8], "G false: all high");
    let g = drive(&[(8, false), (19, false), (18, true)]);
    assert_eq!(ys(&g, &st), vec![l, h, h, h, h, h, h, h], "G true selects 0");
    // Select 5 with POL low: -CE low, clock rise.
    let load = |ce: bool, a: bool, bb: bool, c: bool, pol: bool, cp: bool| {
        drive(&[
            (1, cp),
            (2, true),
            (3, ce),
            (4, a),
            (5, bb),
            (6, c),
            (7, pol),
            (8, false),
            (19, false),
            (18, true),
        ])
    };
    step(
        &b,
        &mut st,
        &load(false, true, false, true, false, false),
        &load(false, true, false, true, false, true),
    );
    assert_eq!(st.bits, 5);
    assert_eq!(ys(&g, &st), vec![h, h, h, h, h, l, h, h]);
    // Held with -CE high, whatever the inputs do.
    step(
        &b,
        &mut st,
        &load(true, false, true, false, true, false),
        &load(true, false, true, false, true, true),
    );
    assert_eq!(st.bits, 5, "clock enable off");
    // POL high inverts all eight.
    step(
        &b,
        &mut st,
        &load(false, false, true, false, true, false),
        &load(false, false, true, false, true, true),
    );
    assert_eq!(st.bits, 2 | 8);
    assert_eq!(ys(&g, &st), vec![l, l, h, l, l, l, l, l], "select 2, polarity high");
    // The four ways of G: only -G1 low with G2 high.
    for (g1, g2, on) in
        [(false, true, true), (true, true, false), (false, false, false), (true, false, false)]
    {
        let p = drive(&[(8, false), (19, g1), (18, g2)]);
        let want = if on { vec![l, l, h, l, l, l, l, l] } else { vec![l; 8] };
        assert_eq!(ys(&p, &st), want, "G1 {g1} G2 {g2}");
    }
    // -OE high: off.
    assert_eq!(ys(&drive(&[(8, true), (19, false), (18, true)]), &st), vec![Level::Z; 8]);
    // Clear at once, with no clock.
    step(
        &b,
        &mut st,
        &load(true, true, true, true, true, false),
        &drive(&[(1, false), (2, false), (3, true)]),
    );
    assert_eq!(st.bits, 0);
}

/// **The SN74165 is its datasheet.** `sn74165.pdf`: a parallel-load
/// 8-bit shift register that shifts "in the direction of QA toward QH",
/// loads directly while `SH/-LD` is low "independently of the levels of
/// CLK, CLK INH, or serial inputs", and is inhibited while either clock
/// pin is high. Only `QH` at 9 and `-QH` at 7 come out. The I/O board's
/// Chaosnet half uses two at LMTBUF B12 and B13 to put sixteen Unibus
/// bits on the wire a bit at a time.
#[test]
fn the_74165_is_its_datasheet() {
    let b = behaviour("74165");
    let mut st = State::default();
    // Pins: 1 SH/-LD, 2 CLK, 3..6 E..H, 10 SER, 11..14 A..D, 15 CLK INH.
    let at = |load: bool, clk: bool, ser: bool, inh: bool, data: u8| -> Pins {
        let bits = |k: u32| data >> k & 1 != 0;
        drive(&[
            (1, load),
            (2, clk),
            (15, inh),
            (10, ser),
            (11, bits(0)),
            (12, bits(1)),
            (13, bits(2)),
            (14, bits(3)),
            (3, bits(4)),
            (4, bits(5)),
            (5, bits(6)),
            (6, bits(7)),
        ])
    };
    let qh = |st: &State| out(&b, 9, &drive(&[]), st) == Level::High;
    let nqh = |st: &State| out(&b, 7, &drive(&[]), st) == Level::High;

    // The load is direct, with no clock edge at all.
    let low = at(false, false, false, false, 0b1000_0001);
    step(&b, &mut st, &low, &low);
    assert_eq!(st.bits, 0b1000_0001, "A..D on 11..14, E..H on 3..6");
    assert!(qh(&st) && !nqh(&st), "QH is stage H, and -QH its complement");

    // Shifting toward QH, with SER entering stage A. The serial input is
    // sampled at the setup instant, so it is held across the edge.
    let hold = at(true, false, false, false, 0);
    let ser_low = at(true, false, true, false, 0);
    let tick = at(true, true, true, false, 0);
    step(&b, &mut st, &ser_low, &tick);
    assert_eq!(st.bits, 0b0000_0011, "each stage moves toward H, SER into A");
    assert!(!qh(&st) && nqh(&st));

    // Either clock pin high inhibits it.
    let inhibited = at(true, true, true, true, 0);
    step(&b, &mut st, &ser_low, &inhibited);
    assert_eq!(st.bits, 0b0000_0011, "CLK INH high: no shift");

    // And the load overrides a clock edge.
    let both = at(false, true, false, false, 0b0101_0101);
    step(&b, &mut st, &hold, &both);
    assert_eq!(st.bits, 0b0101_0101, "SH/-LD low wins over the edge");
}

/// **The 9401 divides by the polynomial its select code names.**
/// `9401.pdf`: a 16-bit register, the code on `S2 S1 S0` picking one of
/// eight polynomials from Table 1, data gated with `Q` and entered on the
/// **high-to-low** transition of `CP` while `CWE` is high, then `CWE`
/// low to shift the check word out. `ER` is low when the register is all
/// zero. The I/O board's Chaosnet half has two, at LMTBUF C09 sending and
/// LMRBUF C07 receiving.
///
/// The check is the one the part is for: feed a message with `CWE` high,
/// shift the sixteen check bits out with `CWE` low, then feed message and
/// check bits back through a second register and see it end at zero, with
/// `ER` low. A single flipped bit must not.
#[test]
fn the_9401_check_word_divides_out() {
    let b = behaviour("9401");
    // Pins: 1 CP, 2 -P, 3 S0, 4 MR, 5 S1, 8 S2, 10 CWE, 11 D.
    let at = |clk: bool, d: bool, cwe: bool, mr: bool| {
        drive(&[
            (1, clk),
            (2, true),
            (3, false),
            (4, mr),
            (5, false),
            (8, false),
            (10, cwe),
            (11, d),
        ])
    };
    // One bit in, on the falling edge, holding the inputs across it.
    let feed = |st: &mut State, d: bool, cwe: bool| {
        let high = at(true, d, cwe, false);
        let low = at(false, d, cwe, false);
        step(&b, st, &high, &low);
    };
    let er = |st: &State| out(&b, 13, &drive(&[]), st) == Level::High;
    let q = |st: &State| out(&b, 12, &drive(&[]), st) == Level::High;

    // A master reset clears it, and a cleared register reports no error.
    let mut st = State::default();
    let reset = at(true, false, true, true);
    step(&b, &mut st, &reset, &reset);
    assert_eq!(st.bits, 0);
    assert!(!er(&st), "all zero is no error");

    let message: Vec<bool> = (0..40u32).map(|k| (k * 7 + 3) % 5 < 2).collect();

    // Encode: the message with CWE high, then sixteen bits out with it low.
    let mut enc = State::default();
    step(&b, &mut enc, &reset, &reset);
    for &d in &message {
        feed(&mut enc, d, true);
    }
    assert_ne!(enc.bits, 0, "the message left a remainder");
    let mut check = Vec::new();
    for _ in 0..16 {
        check.push(q(&enc));
        feed(&mut enc, false, false);
    }
    assert_eq!(enc.bits, 0, "shifting the check word out empties the register");

    // Check: message and check bits back through, CWE high throughout.
    let mut dec = State::default();
    step(&b, &mut dec, &reset, &reset);
    for &d in message.iter().chain(&check) {
        feed(&mut dec, d, true);
    }
    assert_eq!(dec.bits, 0, "a good message divides out");
    assert!(!er(&dec), "ER low after a good message");

    // One bit wrong, and it does not.
    let mut bad = State::default();
    step(&b, &mut bad, &reset, &reset);
    let mut wrong = message.clone();
    wrong[9] = !wrong[9];
    for &d in wrong.iter().chain(&check) {
        feed(&mut bad, d, true);
    }
    assert_ne!(bad.bits, 0, "a flipped bit leaves a remainder");
    assert!(er(&bad), "ER high on a detectable error");

    // The select code picks a different polynomial, so a different word.
    let ccitt = |clk: bool, d: bool, cwe: bool| {
        drive(&[
            (1, clk),
            (2, true),
            (3, false),
            (4, false),
            (5, true),
            (8, true),
            (10, cwe),
            (11, d),
        ])
    };
    let mut other = State::default();
    step(&b, &mut other, &reset, &reset);
    for &d in &message {
        let (h, l) = (ccitt(true, d, true), ccitt(false, d, true));
        step(&b, &mut other, &h, &l);
    }
    assert_ne!(other.bits, enc_remainder(&b, &message), "CRC-CCITT is not CRC-16");
}

/// The remainder CRC-16 leaves on a message, for the comparison above.
fn enc_remainder(b: &Behaviour, message: &[bool]) -> u16 {
    let at = |clk: bool, d: bool| {
        drive(&[
            (1, clk),
            (2, true),
            (3, false),
            (4, false),
            (5, false),
            (8, false),
            (10, true),
            (11, d),
        ])
    };
    let mut st = State::default();
    let reset =
        drive(&[(1, true), (2, true), (3, false), (4, true), (5, false), (8, false), (10, true)]);
    step(b, &mut st, &reset, &reset);
    for &d in message {
        step(b, &mut st, &at(true, d), &at(false, d));
    }
    st.bits
}

/// **A shift in and a shift out in the same instant both happen.** The
/// FIFO's two clocks are the board's own two --- `RBUF.ICLK` from the
/// disk's bit clock, the other from the memory side --- and in a
/// zero-delay model their edges can land in one step. Then the word at
/// the output leaves and the word on the inputs enters, as the part with
/// its two independent ports has it, and the count stays.
#[test]
fn fifo_67401_takes_a_word_in_and_out_in_one_step() {
    let b = behaviour("67401");
    let mut st = State { cells: vec![0; 64], ..State::default() };
    let (si, so, mr) = (3u8, 15u8, 9u8);
    let d = [4u8, 5, 6, 7];
    let o = [13u8, 12, 11, 10];
    let pins = |nibble: u8, shift_in: bool, shift_out: bool, reset: bool| {
        let mut vals = vec![(si, shift_in), (so, shift_out), (mr, reset)];
        for (k, &pin) in d.iter().enumerate() {
            vals.push((pin, nibble >> k & 1 != 0));
        }
        drive(&vals)
    };
    let idle = pins(0, false, false, true);
    let read = |st: &State, p: &Pins| -> u8 {
        o.iter()
            .enumerate()
            .fold(0, |w, (k, &pin)| w | ((out(&b, pin, p, st) == Level::High) as u8) << k)
    };
    let output_ready = |st: &State, p: &Pins| out(&b, 14, p, st) == Level::High;
    step(&b, &mut st, &idle, &pins(0, false, false, false));

    // A and B in.
    for word in [0b0001u8, 0b0010] {
        let (low, high) = (pins(word, false, false, true), pins(word, true, false, true));
        step(&b, &mut st, &low, &high);
        step(&b, &mut st, &high, &low);
    }
    assert_eq!(read(&st, &idle), 0b0001, "A at the output");

    // SO up, holding A at the output, with C on the inputs; then in one
    // step SO drops and SI rises.
    let holding = pins(0b0100, false, true, true);
    step(&b, &mut st, &idle, &holding);
    let both = pins(0b0100, true, false, true);
    step(&b, &mut st, &holding, &both);
    step(&b, &mut st, &both, &idle);
    assert_eq!(read(&st, &idle), 0b0010, "A left: B at the output");

    // B and C out, and then nothing: two words, not one or three.
    for want in [0b0010u8, 0b0100] {
        assert!(output_ready(&st, &idle), "a word to give");
        assert_eq!(read(&st, &idle), want);
        let high = pins(0, false, true, true);
        step(&b, &mut st, &idle, &high);
        step(&b, &mut st, &high, &idle);
    }
    assert!(!output_ready(&st, &idle), "empty after B and C");
}

// ---------------------------------------------------------------------------
// The serial port's parts
// ---------------------------------------------------------------------------

/// **The MC1489's open input gives a high**: `mc1489.pdf` specifies
/// `V_OH` for "Input open", and its schematic has 10 kΩ from the input to
/// ground. A TTL gate's open input reads high and would give the opposite,
/// which is what an empty J9 turned into `-CTS`, `-DSR` and `-DCD` asserted
/// and `RxD` in a break.
#[test]
fn receiver_mc1489_reads_an_open_input_as_low() {
    let b = behaviour("MC1489");
    assert_eq!(out(&b, 3, &pins(), &State::default()), Level::High);
    assert_eq!(out(&b, 3, &drive(&[(1, true)]), &State::default()), Level::Low);
    assert_eq!(out(&b, 3, &drive(&[(1, false)]), &State::default()), Level::High);
}

/// The 2651's pins: `-CE` 11, `R/-W` 13, `A1` 10, `A0` 12, `RESET` 21,
/// `-DCD` 16, `-DSR` 22, `-CTS` 17, `RxD` 3, `BRCLK` 20, and `D0`..`D7` on
/// 27, 28, 1, 2, 5, 6, 7, 8.
fn pci_pins(ce: bool, write: bool, a: u8, d: u8, brclk: bool) -> Pins {
    let mut vals = vec![
        (11u8, ce),
        (13, write),
        (10, a & 2 != 0),
        (12, a & 1 != 0),
        (21, false),
        (16, true),
        (22, true),
        (17, true),
        (3, true),
        (20, brclk),
    ];
    for (k, &pin) in [27u8, 28, 1, 2, 5, 6, 7, 8].iter().enumerate() {
        vals.push((pin, d >> k & 1 != 0));
    }
    drive(&vals)
}

/// One bus access to register `a`: `-CE` down with the data, and up
/// again, which is where the chip takes it.
fn pci_access(b: &Behaviour, st: &mut State, write: bool, a: u8, d: u8) {
    let idle = pci_pins(true, write, a, d, false);
    let active = pci_pins(false, write, a, d, false);
    step(b, st, &idle, &active);
    step(b, st, &active, &idle);
}

/// **The 2651 takes a write on the rise of `-CE`, answers a read while it
/// is low, and holds still from reset**: the mode registers through their
/// pointer, the command register, and the data bus three-stated
/// otherwise; the three ready outputs high, being open drain and not
/// ready.
#[test]
fn pci_2651_registers_by_the_sheets_table_4() {
    let b = behaviour("2651");
    let mut st = State::default();
    // Reset, as a level.
    let mut reset = pci_pins(true, false, 0, 0, false);
    reset[21] = Level::High;
    step(&b, &mut st, &reset, &reset);
    let idle = pci_pins(true, false, 0, 0, false);
    step(&b, &mut st, &reset, &idle);
    for pin in [27u8, 28, 1, 2, 5, 6, 7, 8] {
        assert_eq!(out(&b, pin, &idle, &st), Level::Z, "the bus is three-stated between accesses");
    }
    for pin in [19u8, 15, 14, 18, 24, 23] {
        assert_eq!(out(&b, pin, &idle, &st), Level::High, "pin {pin} high from reset");
    }
    // Mode 1, mode 2 through the pointer; the command register read puts
    // it back.
    pci_access(&b, &mut st, true, 2, 0o171);
    pci_access(&b, &mut st, true, 2, 0o77);
    pci_access(&b, &mut st, true, 3, 0o47);
    pci_access(&b, &mut st, false, 3, 0);
    let read = |st: &State, a: u8| -> u8 {
        let p = pci_pins(false, false, a, 0, false);
        [27u8, 28, 1, 2, 5, 6, 7, 8]
            .iter()
            .enumerate()
            .map(|(k, &pin)| ((out(&b, pin, &p, st) == Level::High) as u8) << k)
            .sum()
    };
    assert_eq!(read(&st, 2), 0o171, "mode register 1");
    pci_access(&b, &mut st, false, 2, 0);
    assert_eq!(read(&st, 2), 0o77, "mode register 2 after the first was read");
    assert_eq!(read(&st, 3), 0o47, "the command register");
    // Enabled and empty: `-TxRDY` down, `-RxRDY` up, `-DTR` and `-RTS` as
    // CR1 and CR5 say.
    assert_eq!(out(&b, 15, &idle, &st), Level::Low, "-TxRDY");
    assert_eq!(out(&b, 14, &idle, &st), Level::High, "-RxRDY");
    assert_eq!(out(&b, 24, &idle, &st), Level::Low, "-DTR from CR1");
    assert_eq!(out(&b, 23, &idle, &st), Level::Low, "-RTS from CR5");
    let sr = read(&st, 1);
    assert_eq!(sr, 1, "the status register: TxRDY alone, no data set: {sr:o}");
    // A write to the holding register takes `-TxRDY` up until the
    // generator moves it into the shift register: sixteen crystal
    // rises at rate 15, and the start bit is then on `TxD`.
    pci_access(&b, &mut st, true, 0, 0o125);
    assert_eq!(out(&b, 15, &idle, &st), Level::High, "-TxRDY with the holding register full");
    assert_eq!(out(&b, 19, &idle, &st), Level::High, "TxD still marking");
    let mut low = pci_pins(true, false, 0, 0, false);
    low[17] = Level::Low;
    let mut high = low;
    high[20] = Level::High;
    for _ in 0..16 {
        step(&b, &mut st, &low, &high);
        step(&b, &mut st, &high, &low);
    }
    assert_eq!(out(&b, 19, &low, &st), Level::Low, "the start bit");
    assert_eq!(
        out(&b, 15, &low, &st),
        Level::Low,
        "-TxRDY again, the character in the shift register"
    );
    // Sixteen 16X clocks on: the first data bit, a one.
    for _ in 0..16 * 16 {
        step(&b, &mut st, &low, &high);
        step(&b, &mut st, &high, &low);
    }
    assert_eq!(out(&b, 19, &low, &st), Level::High, "D0 of 125");
}
