// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The engines' rates on the benchmark programs of `src/benchmark.rs`.
//!
//!     cargo run --release --example benchmark -- [seconds] [--color-tv]
//!
//! Every engine on every program, six runs of five seconds each unless a
//! different number of seconds is given; `--color-tv` fits the second
//! display board as `muir --color-tv` does, to measure what it costs.  Each run is checked against the
//! count the program left in `VMA`, so a rate is only printed for a run that
//! did the work.  Nothing is read from `vendor/`.

use std::time::Duration;

use muir::benchmark::{self, Program, Run, Stop};
use muir::engine::Engine;
use muir::machine::Machine;
use muir::micro::Micro;
use muir::netlist::{self, Netlist};
use muir::rtl::Rtl;

const NETLIST: &str = include_str!("../data/CADR.netlist");
const BUSINT: &str = include_str!("../data/BUSINT.netlist");
const CADRM: &str = include_str!("../data/CADRM.netlist");
const CADRIO: &str = include_str!("../data/CADRIO.netlist");
const SIMPLETV: &str = include_str!("../data/SIMPLETV.netlist");
const LISPMTV: &str = include_str!("../data/LISPMTV.netlist");

/// The machine's own microcycle, 145 ns at normal speed.
const HARDWARE_CYCLES_PER_S: f64 = 1e9 / 145.0;

/// What the ratio does **not** include: the disk.
///
/// This is microcycles against microcycles. With the disk controller as a
/// behavioral model --- always on `micro` and `rtl`, and on `chip` when
/// it is given `--disk-controller model` --- a transfer completes inside
/// the store to `START` and a seek takes no time. The machine spent
/// milliseconds on a seek and spent them running the microcode's polling
/// loop, so a 55 ms seek is about 380,000 microcycles the hardware executes
/// and muir does not. A program that seeks therefore finishes further ahead
/// of the hardware than this says, by an amount that depends on the program.
///
/// On the netlist disk controller it inverts: the drive takes its own time,
/// the polling loop runs through every gate on the board, and a seeking
/// program comes out slower than this rather than faster.
fn report(engine: &str, p: &Program, run: &Run) {
    run.check(p);
    let ratio = run.rate() / HARDWARE_CYCLES_PER_S;
    let against = if ratio >= 0.1 {
        format!("{ratio:.2}x hardware")
    } else {
        format!("hardware/{:.0}", 1.0 / ratio)
    };
    println!(
        "{engine:6} {:9} {:>11} cycles {:8.3} s {:>12.0} cycles/s {against:>16}",
        p.name,
        run.cycles,
        run.secs,
        run.rate()
    );
}

fn on_engine<E: Engine>(
    name: &str,
    new: fn(Machine) -> E,
    boot: fn(&mut E),
    p: &Program,
    s: Stop,
    color_tv: bool,
) {
    let mut m = Machine::new();
    m.load_prom(&p.prom);
    if color_tv {
        m.fit_color_tv();
    }
    let mut e = new(m);
    boot(&mut e);
    report(name, p, &benchmark::run_engine(&mut e, p, s));
}

fn on_chip(n: &Netlist, boards: &[Netlist; 4], color_tv: Option<&Netlist>, p: &Program, s: Stop) {
    let [bus_n, mem_n, io_n, tv_n] = boards;
    let (mut c, mut clk, mut far) = benchmark::chip_with(n, bus_n, mem_n, io_n, tv_n, color_tv);
    benchmark::boot_chip(&mut c, n, &mut far, &mut clk, p);
    report("chip", p, &benchmark::run_chip(&mut c, n, &mut far, &mut clk, p, s));
}

/// `benchmark [seconds] [--color-tv]`: the seconds a run is given, and
/// whether the color TV is fitted --- the model on `micro` and `rtl`, and
/// on `chip` the LISPM TV netlist wrapped to the color addresses beside the
/// model, as `muir --color-tv` fits it.
fn main() {
    let mut secs = 5.0;
    let mut color_tv = false;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--color-tv" => color_tv = true,
            s => secs = s.parse().expect("seconds, or --color-tv"),
        }
    }
    let stop = Stop::After(Duration::from_secs_f64(secs));
    let n = netlist::parse(NETLIST).unwrap();
    let boards = [BUSINT, CADRM, CADRIO, SIMPLETV].map(|s| netlist::parse(s).unwrap());
    let color = color_tv.then(|| netlist::parse_color_tv(LISPMTV).unwrap());
    if color_tv {
        eprintln!("with the color TV fitted");
    }
    for p in [benchmark::datapath(), benchmark::control()] {
        on_engine("micro", Micro::new, Micro::boot, &p, stop, color_tv);
        on_engine("rtl", Rtl::new, Rtl::boot, &p, stop, color_tv);
        on_chip(&n, &boards, color.as_ref(), &p, stop);
    }
}
