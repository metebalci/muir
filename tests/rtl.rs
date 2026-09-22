// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What clocks each register the `rtl` engine holds.
//!
//! `src/rtl.rs` runs on the machine's own clock: one edge per microcycle,
//! and a level either side of it. That is a claim about the board, so it is
//! made here twice over --- read off `data/CADR.netlist` through
//! `muir::part::behavior`, whose `update_ins` names the pins a stateful part
//! latches on, and then measured on `chip` while it runs.
//!
//! The board has two phases and one register clock edge a microcycle, and a
//! sequencer with more states than that gives a different answer. These are
//! the tests that say what the board does.

use muir::chip::Chip;
use muir::clock::{Behavioral, Clock};
use muir::netlist::{self, NetId, Netlist};
use muir::part::{self, Level};
use std::collections::{BTreeMap, BTreeSet};

const NETLIST: &str = include_str!("../data/CADR.netlist");

/// The pages that generate and distribute the clock. Nets driven from here
/// are timing, not data.
const CLOCK_PAGES: &[&str] = &["CLOCK1", "CLOCK2", "CLOCKD"];

/// Every net driven by a part on a clock page.
fn clock_nets(n: &Netlist) -> BTreeSet<NetId> {
    let mut out = BTreeSet::new();
    for p in &n.parts {
        if !CLOCK_PAGES.contains(&p.page.as_str()) {
            continue;
        }
        let Some(po) = part::pinout(&p.kind) else { continue };
        for &(pin, net) in &p.pins {
            if po.drives(pin) {
                out.insert(net);
            }
        }
    }
    out
}

/// One package that remembers something, with the timing nets it latches on.
struct Clocked {
    page: String,
    reference: String,
    kind: String,
    /// Pin number and net name, for each timing net the part latches on.
    timing: Vec<(u8, String)>,
}

/// Every package that remembers something, with the timing nets it latches
/// on and the pins carrying them.
fn stateful(n: &Netlist) -> Vec<Clocked> {
    let clocks = clock_nets(n);
    let mut out = Vec::new();
    for pkg in n.packages() {
        let Some(b) = part::behavior(&pkg.kind) else { continue };
        if b.update.is_none() {
            continue;
        }
        let timing: Vec<(u8, String)> = pkg
            .pins
            .iter()
            .filter(|(pin, net)| b.update_ins.contains(pin) && clocks.contains(net))
            .map(|&(pin, net)| (pin, n.net(net).to_string()))
            .collect();
        out.push(Clocked {
            page: pkg.page.clone(),
            reference: pkg.reference.clone(),
            kind: pkg.kind.clone(),
            timing,
        });
    }
    out
}

/// The table itself. `cargo test --release --test rtl -- --nocapture`.
#[test]
#[ignore = "a report, not a check: --ignored --nocapture"]
fn what_clocks_every_register() {
    let n = netlist::parse(NETLIST).unwrap();
    let parts = stateful(&n);
    let mut by_page: BTreeMap<&str, Vec<&Clocked>> = BTreeMap::new();
    for p in &parts {
        by_page.entry(&p.page).or_default().push(p);
    }
    for (page, ps) in &by_page {
        eprintln!("{page}");
        for c in ps {
            let on = c
                .timing
                .iter()
                .map(|(pin, net)| format!("p{pin}={net}"))
                .collect::<Vec<_>>()
                .join(" ");
            eprintln!("  {:6} {:10} {on}", c.reference, c.kind);
        }
    }
    eprintln!("{} stateful packages on {} pages", parts.len(), by_page.len());
}

/// Which timing nets the datapath latches on, and how many packages hang
/// off each: the set `rtl` models.
#[test]
#[ignore = "a report, not a check: --ignored --nocapture"]
fn the_datapath_latches_on_few_timing_nets() {
    let n = netlist::parse(NETLIST).unwrap();
    let mut count: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for c in stateful(&n) {
        if CLOCK_PAGES.contains(&c.page.as_str()) {
            continue;
        }
        for (_, net) in c.timing {
            count.entry(net).or_default().push(format!("{} {}", c.page, c.reference));
        }
    }
    for (net, users) in &count {
        eprintln!("{:12} {:3}  {}", net, users.len(), users.join(", "));
    }
    eprintln!("{} distinct timing nets reach the datapath", count.len());
}

/// The timing nets, as levels, measured on the running board.
///
/// The table above says which net clocks each register. It does not say
/// *when*, so this boots `chip` and watches every one of them across a
/// microcycle.
///
/// They all follow `TPCLK`: `-CLK0` is `-TPCLK AND MACHRUN` at CLOCK2 1D10
/// and `CLK1..CLK5` are `NOT(-CLK0)` through the 7428 buffers at 1D05, 1C01
/// and 1C11, so the whole of `CLK1A`, `CLK2A..C`, `CLK3A..F` and `CLK4A..F`
/// is one edge, at the cycle boundary. Measuring it says so without having
/// to trust that reading.
#[test]
fn the_datapath_has_one_clock_edge_per_microcycle() {
    let n = netlist::parse(NETLIST).unwrap();
    let image: Vec<u64> = muir::prom::boot_prom_image();
    let mut c = Chip::new(&n);
    c.power_on();
    c.load_prom(&n, &image);
    c.settle();
    let mut clk = Behavioral::new();
    let boot = n.by_name_id("-BOOT1").unwrap();
    c.set_net(boot, Level::Low);
    c.settle();
    for _ in 0..20 {
        c.tick(&mut clk);
    }
    c.set_net(boot, Level::High);
    // Well into the PROM, so the machine is running rather than starting.
    for _ in 0..150 {
        c.tick(&mut clk);
    }
    while clk.phase_ns() != 0 {
        c.tick(&mut clk);
    }

    // Every timing net the datapath latches on, plus the write pulses and
    // the tri-state enables, which gate rather than clock.
    let mut watch: Vec<String> = Vec::new();
    for c in stateful(&n) {
        if CLOCK_PAGES.contains(&c.page.as_str()) {
            continue;
        }
        for (_, net) in c.timing {
            if !watch.contains(&net) {
                watch.push(net);
            }
        }
    }
    watch.retain(|s| s != "-RESET" && s != "-LCRY3");
    watch.sort();
    for extra in ["-WP1", "-WP2", "-WP3", "-WP4", "-TSE1", "TPCLK", "TPWP"] {
        watch.push(extra.to_string());
    }
    let ids: Vec<NetId> = watch.iter().map(|s| n.by_name_id(s).unwrap()).collect();

    let mut was: Vec<Level> = ids.iter().map(|&id| c.net(id)).collect();
    let mut edges: BTreeMap<String, Vec<(u32, Level)>> = BTreeMap::new();
    for _ in 0..64 {
        c.tick(&mut clk);
        let at = clk.phase_ns();
        for (k, &id) in ids.iter().enumerate() {
            let now = c.net(id);
            if now != was[k] {
                edges.entry(watch[k].clone()).or_default().push((at, now));
                was[k] = now;
            }
        }
        if clk.phase_ns() == 0 {
            break;
        }
    }
    for (net, es) in &edges {
        let s: Vec<String> = es.iter().map(|(at, l)| format!("{at}ns->{l:?}")).collect();
        eprintln!("{:8} {}", net, s.join(" "));
    }

    // Every register clock rises at one instant, and it is the same one.
    let rises: BTreeSet<u32> = edges
        .iter()
        .filter(|(net, _)| net.starts_with("CLK") || net.starts_with("MCLK"))
        .flat_map(|(_, es)| es.iter().filter(|(_, l)| *l == Level::High).map(|&(at, _)| at))
        .collect();
    assert_eq!(rises.len(), 1, "register clocks rise at {rises:?}");
}

/// How far behind the PC the `OPC` register runs.
///
/// A single register one microcycle behind is what `OPC` looks like from the
/// microcode, and it is not what the board holds. OPCS 1F06-1F13 are 9328s
/// --- dual eight-bit shift
/// registers --- with `PC` shifted in and only the last stage brought out to
/// `OPC<13:0>`, which page OPCD drives onto `MF` for a `SRCOPC`. So the
/// depth is a fact about the board, and this measures it rather than
/// trusting the reading.
#[test]
fn how_far_behind_the_pc_opc_runs() {
    let n = netlist::parse(NETLIST).unwrap();
    let image: Vec<u64> = muir::prom::boot_prom_image();
    let mut c = Chip::new(&n);
    c.power_on();
    c.load_prom(&n, &image);
    c.settle();
    let mut clk = Behavioral::new();
    let boot = n.by_name_id("-BOOT1").unwrap();
    c.set_net(boot, Level::Low);
    c.settle();
    for _ in 0..20 {
        c.tick(&mut clk);
    }
    c.set_net(boot, Level::High);
    for _ in 0..150 {
        c.tick(&mut clk);
    }

    // The PC each microcycle, and what OPC held at the same moment.
    let mut pcs: Vec<u64> = Vec::new();
    let mut opcs: Vec<u64> = Vec::new();
    for _ in 0..40 {
        c.microcycle(&mut clk);
        pcs.push(c.bus(&n, "PC", 14));
        opcs.push(c.bus(&n, "OPC", 14));
    }
    let oct = |v: &[u64]| v.iter().map(|x| format!("{x:o}")).collect::<Vec<_>>().join(" ");
    eprintln!("PC  {}", oct(&pcs[..16]));
    eprintln!("OPC {}", oct(&opcs[..16]));

    // The lag that lines the two up over the whole window: the eight stages
    // of the 9328s, which is the depth `src/rtl.rs` holds `OPC` to.
    let lag = (1..=12).find(|&d| pcs[..pcs.len() - d].iter().zip(&opcs[d..]).all(|(p, o)| p == o));
    eprintln!("OPC lags PC by {lag:?} microcycles");
    assert_eq!(lag, Some(8), "OPC is the PC of eight microcycles ago");
}

/// **The trap cycle after a boot is long, and counted, when `IR` asks.**
/// `-ILONG` is `NAND(IR45, -NOPA)` at FLAG 3E07 and `-STATBIT` the other
/// half of it with `IR46`; `-NOPA` is `AND(-NOP11, -INOP)` at CONTRL 3E14,
/// and `NOP` is `NAND(-TRAP, -NOPA)` at 3E23 --- so the trap makes a cycle
/// nopped without making it `NOPA`, and neither the long cycle nor the
/// statistics count is suppressed in it. A machine running long, counted
/// instructions at normal speed is booted; the cycle after is 185 ns, not
/// 145, and the counter moves.
#[test]
fn the_trap_cycle_is_long_and_counted_when_ir_asks() {
    use muir::engine::Engine;
    use muir::isa::Insn;
    use muir::isa::asm::filler;
    use muir::machine::Machine;
    use muir::rtl::Rtl;
    let mut m = Machine::new();
    let long = Insn::new(filler().raw() | 1 << 45 | 1 << 46);
    m.load_prom(&vec![long; 512]);
    let mut r = Rtl::new(m);
    r.boot();
    r.machine_mut().mode.speed1 = true;
    for _ in 0..10 {
        r.step().unwrap();
    }
    let (ns, stat) = (r.ns(), r.stat());
    r.step().unwrap();
    assert_eq!((r.ns() - ns, r.stat() - stat), (185, 1), "a running cycle");
    r.boot();
    // The boot's reset clears the mode register.
    r.machine_mut().mode.speed1 = true;
    let (ns, stat) = (r.ns(), r.stat());
    r.step().unwrap();
    assert_eq!((r.ns() - ns, r.stat() - stat), (185, 1), "the trap cycle");
}
