// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The levelizer, and the `chip` engine held to `rtl`.
//!
//! Taken pin by pin, against the per-pin dependency lists in `src/part.rs`
//! that the engine evaluates, the processor board is nearly acyclic: what
//! does not levelize is one loop, and it is hardware feedback rather than a
//! shortcoming of the model, so the engine levelizes what it can and
//! iterates the rest. The tests below pin that, and then run the board
//! beside `rtl` microcycle for microcycle.

mod support;

use muir::cable::{Boards, FarEnd};
use muir::chip::{Chip, REPLACED};
use muir::netlist;
use muir::part;

const NETLIST: &str = include_str!("../data/CADR.netlist");
const BUSINT: &str = include_str!("../data/BUSINT.netlist");
const CADRM: &str = include_str!("../data/CADRM.netlist");
const CADRIO: &str = include_str!("../data/CADRIO.netlist");
const SIMPLETV: &str = include_str!("../data/SIMPLETV.netlist");
const LISPMTV: &str = include_str!("../data/LISPMTV.netlist");
const CADRDC: &str = include_str!("../data/CADRDC.netlist");

/// How many 64K-word memory boards are on the backplane: 32 fill the 2M
/// words `rtl`'s machine has.
const MEMORY_BOARDS: usize = 32;

/// `MUIR_NO_SLEEP=1` turns every optimisation off, the slow way: every
/// board stepped at every edge of its own clock instead of sleeping,
/// every wire carried at every exchange ([`FarEnd::unoptimised`]).
/// `MUIR_MAIN_MEMORY=model` and `MUIR_IO_BOARD=model` run those from the
/// machine's models instead of as netlists, the timing being the board's
/// either way; netlists unless told. `MUIR_TV=netlist` puts the display
/// netlist on the backplane --- it answers the boot's every access --- but
/// `rtl` has no timing twin for it yet as it has for memory and the I/O
/// board, so a comparison against `rtl` diverges at the boot's first
/// display write (`chip` 870 ns, `rtl` 145); until the twin exists these
/// tests run the model display unless told, where `muir` runs the
/// netlist. `MUIR_TV_BOARD=lispm-tv` puts the LISPM TV there in place of
/// the SIMPLE TV. `MUIR_DISK_CONTROLLER=netlist` puts the disk controller
/// netlist there, with the pack in unit 0 on its cable as a drive; its
/// transfers then take the drive's time, milliseconds a block, where the
/// model's take none, so the model is the default for that one too.
fn far_end(n: &netlist::Netlist, machine: muir::machine::Machine) -> FarEnd {
    let bus_n = netlist::parse(BUSINT).unwrap();
    let mem_n = netlist::parse(CADRM).unwrap();
    let io_n = netlist::parse(CADRIO).unwrap();
    let tv_n = netlist::parse(match std::env::var("MUIR_TV_BOARD").as_deref() {
        Ok("lispm-tv") => LISPMTV,
        _ => SIMPLETV,
    })
    .unwrap();
    let disk_n = netlist::parse(CADRDC).unwrap();
    let model = |what: &str| std::env::var(what).is_ok_and(|v| v == "model");
    let netlist_asked = |what: &str| std::env::var(what).is_ok_and(|v| v == "netlist");
    let boards = if model("MUIR_MAIN_MEMORY") { 0 } else { MEMORY_BOARDS };
    let io = if model("MUIR_IO_BOARD") { None } else { Some(&io_n) };
    let tv = if netlist_asked("MUIR_TV") { Some(&tv_n) } else { None };
    let disk = if netlist_asked("MUIR_DISK_CONTROLLER") { Some(&disk_n) } else { None };
    let boards = Boards { memory: boards, io, tv, disk };
    let mut far = FarEnd::new(n, &bus_n, &mem_n, boards, 0, machine);
    if std::env::var("MUIR_NO_SLEEP").is_ok() {
        far.unoptimised();
    }
    far
}

fn build() -> Chip {
    Chip::new(&netlist::parse(NETLIST).unwrap())
}

/// A net of one of the far end's boards by name. Those netlists store a
/// name with spaces in quotes, `'UB NXM ERROR'`, so a name is looked up
/// bare and then quoted.
fn find_net(n: &netlist::Netlist, name: &str) -> Option<netlist::NetId> {
    n.by_name_id(name).or_else(|| n.by_name_id(&format!("'{name}'")))
}

/// [`find_net`], for a net that has to be there.
fn net_named(n: &netlist::Netlist, name: &str) -> netlist::NetId {
    find_net(n, name).unwrap_or_else(|| panic!("no net {name}"))
}

/// Every gate outside the clock pages is ordered exactly once.
#[test]
fn the_order_covers_every_gate() {
    let c = build();
    let outside: usize = c
        .instances
        .iter()
        .filter(|i| !REPLACED.iter().any(|&(p, r)| i.page == p && i.reference == r))
        .map(|i| i.gate_count())
        .sum();
    assert_eq!(c.ordered(), outside, "gates the clock model does not replace");
    eprintln!("{} packages, {} gates ordered", c.instances.len(), c.ordered());
}

/// Almost all of it levelizes. What does not is one loop, and it is in the
/// hardware rather than in the model.
#[test]
fn one_feedback_loop_remains() {
    let c = build();
    let groups = c.feedback();
    for g in &groups {
        let mut pages: Vec<&str> =
            g.iter().map(|id| c.instances[id.part as usize].page.as_str()).collect();
        pages.sort_unstable();
        pages.dedup();
        eprintln!("feedback group of {} gates across {} pages: {pages:?}", g.len(), pages.len());
    }
    assert_eq!(groups.len(), 1, "expected one feedback loop");
    // Under a quarter. The 74S373 latches are combinational while
    // transparent, which puts real dependencies through the A and M buses
    // into the loop; that is what makes it as large as it is.
    assert!(
        c.in_feedback * 4 < c.ordered(),
        "{} of {} gates are in feedback",
        c.in_feedback,
        c.ordered()
    );
}

/// The loop is the SPY bus: the console reads the output bus through the
/// buffers on SPY1 and SPY2, and writes the mode register back over the
/// same wires. Nothing closes it in operation, because the read enables and
/// the write strobes are never asserted together --- but a graph cannot know
/// that, so it stays a loop.
#[test]
fn the_feedback_loop_is_the_console_bus() {
    let c = build();
    let group = c.feedback()[0];
    let pages: std::collections::BTreeSet<&str> =
        group.iter().map(|id| c.instances[id.part as usize].page.as_str()).collect();
    assert!(pages.contains("SPY1") || pages.contains("SPY2"), "pages: {pages:?}");
}

/// Settling twice from the same state must give the same answer, or the
/// order is not doing its job.
#[test]
fn settling_is_deterministic() {
    let n = netlist::parse(NETLIST).unwrap();
    let mut a = Chip::new(&n);
    a.settle();
    let first: Vec<_> = (0..n.nets.len() as u32).map(|i| a.net(i)).collect();
    a.settle();
    let second: Vec<_> = (0..n.nets.len() as u32).map(|i| a.net(i)).collect();
    let differ = first.iter().zip(&second).filter(|(x, y)| x != y).count();
    assert_eq!(differ, 0, "{differ} nets moved on a second settle");
    assert!(a.unsettled.is_none(), "a feedback loop did not converge");

    let mut b = Chip::new(&n);
    b.settle();
    let other: Vec<_> = (0..n.nets.len() as u32).map(|i| b.net(i)).collect();
    assert_eq!(first, other, "two builds settled differently");
}

/// After power-on and a settle, no net may be left unknown.
///
/// This is the completeness check for the whole chain: a part whose
/// behaviour is missing, a gate that reads a pin nothing drives, or a memory
/// nobody sized would all show up here as an unknown that never resolves.
/// Undriven nets are *not* unknown --- they come out high impedance, which a
/// TTL input reads as a one.
#[test]
fn nothing_is_left_unknown() {
    let n = netlist::parse(NETLIST).unwrap();
    let mut c = Chip::new(&n);
    c.power_on();
    c.settle();
    let unknown: Vec<&str> = (0..n.nets.len() as u32)
        .filter(|&i| c.net(i) == muir::part::Level::X)
        .map(|i| n.net(i))
        .collect();
    eprintln!("{} of {} nets unknown after settling", unknown.len(), n.nets.len());
    if !unknown.is_empty() {
        eprintln!("first few: {:?}", &unknown[..unknown.len().min(20)]);
    }
    assert!(unknown.is_empty(), "{} nets never settled", unknown.len());
}

/// The feedback loop must converge, and cheaply.
///
/// It is a loop only to a dependency graph. In operation the SPY buffers are
/// high impedance unless the console has asserted a read enable, and the mode
/// register only loads on a write strobe, so nothing actually travels round
/// it --- the tri-state enables cut it. What the loop costs is sweeps, and
/// this pins how many.
#[test]
fn the_feedback_loop_converges_in_a_few_sweeps() {
    let n = netlist::parse(NETLIST).unwrap();
    let mut c = Chip::new(&n);
    c.power_on();
    c.settle();
    eprintln!("cold start: {} sweeps", c.sweeps);
    assert!(c.unsettled.is_none(), "a loop did not settle");
    let cold = c.sweeps;
    c.settle();
    eprintln!("already at rest: {} sweeps", c.sweeps);
    assert!(cold <= 8, "cold start took {cold} sweeps");
    // Not one sweep: none. Nothing is dirty, so the loop is not entered at
    // all --- which is the point of settling incrementally.
    assert_eq!(c.sweeps, 0, "an already-settled board should need no work");
}

/// Doing only the work that is needed must give the same machine as doing
/// all of it.
///
/// The engine skips two kinds of work, and both rest on marks being complete.
/// [`Chip::settle`] re-evaluates only gates whose input nets have moved, and
/// [`Chip::update_state`] asks only the parts a net has moved under. If
/// either mark were missed, a net would keep a stale level or a register
/// would miss an edge, and **nothing else in the suite would notice** --- the
/// machine would simply be quietly the wrong machine.
///
/// So this runs the real thing beside a reference that marks everything
/// before every transition, and compares the nets *and* what every part
/// holds, at every transition rather than once at the end --- a stale value
/// that is later overwritten would otherwise hide. It also settles the real
/// one from scratch each time, which catches a stale net in place.
///
/// This is the test that would have caught the pin-comparison version of the
/// update skip, which the rest of the suite let through as far as the boot.
#[test]
fn skipping_work_matches_doing_all_of_it() {
    use muir::clock::Behavioural;
    use muir::part::Level;
    let n = netlist::parse(NETLIST).unwrap();
    let image: Vec<u64> = muir::prom::boot_prom_image();
    let boot = n.by_name_id("-BOOT1").unwrap();
    let start = |n: &netlist::Netlist| {
        let mut c = Chip::new(n);
        c.power_on();
        c.load_prom(n, &image);
        c.settle();
        c.set_net(boot, Level::Low);
        c.settle();
        c
    };
    let mut c = start(&n);
    let mut reference = start(&n);
    let mut a = Behavioural::new();
    let mut b = Behavioural::new();

    for tick in 0..400 {
        c.tick(&mut a);
        // The reference does the work unconditionally.
        reference.mark_all();
        reference.tick(&mut b);
        if tick == 20 {
            for chip in [&mut c, &mut reference] {
                chip.set_net(boot, Level::High);
                chip.settle();
            }
        }

        let differ: Vec<&str> = (0..n.nets.len() as u32)
            .filter(|&i| c.net(i) != reference.net(i))
            .map(|i| n.net(i))
            .collect();
        assert!(
            differ.is_empty(),
            "tick {tick}: {} nets differ from the exhaustive reference: {:?}",
            differ.len(),
            &differ[..differ.len().min(8)]
        );
        for (i, (x, y)) in c.instances.iter().zip(&reference.instances).enumerate() {
            assert_eq!(
                (x.state.bits, &x.state.cells),
                (y.state.bits, &y.state.cells),
                "tick {tick}: {} holds something different",
                c.instances[i].name()
            );
        }

        // And nothing may be stale in place.
        let before: Vec<Level> = (0..n.nets.len() as u32).map(|i| c.net(i)).collect();
        c.settle_all();
        let moved = (0..n.nets.len() as u32).filter(|&i| c.net(i) != before[i as usize]).count();
        assert_eq!(moved, 0, "tick {tick}: a full settle moved {moved} nets");
    }
    eprintln!("400 clock transitions, nets and part state identical to the exhaustive reference");
}

/// A checkpoint must restore the machine exactly, and keep running the same.
///
/// Comparing the nets right after loading is not enough: a checkpoint that
/// forgot which gates and parts are still marked would match at that instant
/// and then quietly drift, because the engine only evaluates what is marked.
/// So this reloads into a fresh board, compares every net and everything
/// every part holds, and then runs *both* on and compares again.
#[test]
fn a_checkpoint_restores_the_machine_exactly() {
    use muir::clock::Behavioural;
    use muir::part::Level;
    let n = netlist::parse(NETLIST).unwrap();
    let image: Vec<u64> = muir::prom::boot_prom_image();
    let mut c = Chip::new(&n);
    c.power_on();
    c.load_prom(&n, &image);
    c.settle();
    let mut clk = Behavioural::new();
    c.settle();
    for _ in 0..20 {
        c.tick(&mut clk);
    }
    c.set_net(n.by_name_id("-BOOT1").unwrap(), Level::High);
    for _ in 0..300 {
        c.tick(&mut clk);
    }

    // Saved with work outstanding, on purpose. Between transitions the marks
    // are quiescent and a checkpoint that dropped them would look perfect;
    // forcing a net here leaves gates and parts marked, so the reload has to
    // carry them or it will settle to something else.
    //
    // The net has to be one a gate this chip evaluates reads, and forced to
    // the level it is not at. `-HANG` is the wrong choice that looks like the
    // right one: its only reader is on the clock generator page, which the
    // behavioural clock stands in for, so forcing it marks nothing --- and
    // the stale marks a full settle leaves behind hide that, which is what
    // `Chip::mark_every_gate` keeps honest. `-BOOT1` is read by the 74LS14
    // at OLORD2 1A20.
    let boot = n.by_name_id("-BOOT1").unwrap();
    let flipped = if c.net(boot) == Level::Low { Level::High } else { Level::Low };
    c.set_net(boot, flipped);

    let mut saved = Vec::new();
    c.save(&mut saved).unwrap();
    clk.save(&mut saved).unwrap();
    eprintln!("checkpoint is {} KB", saved.len() / 1024);

    let mut back = Chip::new(&n);
    let mut cursor = &saved[..];
    back.load(&mut cursor).unwrap();
    let mut clk_back = Behavioural::load(&mut cursor).unwrap();
    assert!(cursor.is_empty(), "{} bytes left over", cursor.len());

    let same = |a: &Chip, b: &Chip, when: &str| {
        let nets: Vec<&str> =
            (0..n.nets.len() as u32).filter(|&i| a.net(i) != b.net(i)).map(|i| n.net(i)).collect();
        assert!(
            nets.is_empty(),
            "{when}: {} nets differ: {:?}",
            nets.len(),
            &nets[..nets.len().min(6)]
        );
        for (x, y) in a.instances.iter().zip(&b.instances) {
            assert_eq!(
                (x.state.bits, &x.state.cells),
                (y.state.bits, &y.state.cells),
                "{when}: {} holds something different",
                x.name()
            );
        }
    };
    same(&c, &back, "on loading");
    // The outstanding work has to come back too, and it is compared
    // directly: leaving it out would look identical at this instant and
    // change what the next transition evaluates.
    assert_eq!(c.pending(), back.pending(), "outstanding work differs");
    // Gates only: between transitions the part marks are normally empty,
    // because `update_state` has just run and consumed them. The equality
    // above is what covers them --- a load that dropped them would leave the
    // fresh board's marks, which are all set.
    assert!(c.pending().0 > 0, "this should have been saved with work to do");
    eprintln!("{:?} gates and parts still marked at the checkpoint", c.pending());

    // And it must keep running the same, which is what catches a mark that
    // was not saved.
    for tick in 0..200 {
        c.tick(&mut clk);
        back.tick(&mut clk_back);
        same(&c, &back, &format!("tick {tick} after loading"));
    }
    eprintln!("identical on loading and over 200 further transitions");
}

/// A checkpoint from a board this is not may not be loaded.
#[test]
fn a_checkpoint_from_another_board_is_refused() {
    let n = netlist::parse(NETLIST).unwrap();
    let mut c = Chip::new(&n);
    c.power_on();
    let mut saved = Vec::new();
    c.save(&mut saved).unwrap();

    // The fingerprint sits right after the magic.
    saved[8] ^= 0xff;
    let err = c.load(&mut &saved[..]).unwrap_err();
    eprintln!("refused: {err}");
    assert!(err.to_string().contains("different board"));

    saved[8] ^= 0xff;
    saved[0] = b'X';
    assert!(c.load(&mut &saved[..]).is_err(), "bad magic should be refused");
}

/// The clock runs, and the board follows it.
///
/// The control store is still empty, so nothing is executed. What this
/// checks is that the loop closes --- the clock reads the board, moves, and
/// its outputs come back through the buffers we deliberately left structural
/// --- and that the gating those buffers carry actually gates.
///
/// The CADR comes up **halted**: `MACHRUN` is low until the console starts
/// it. So the generator runs while the machine does not, which is exactly
/// what `src/clock.rs` describes, and it is visible here:
///
/// - `TPCLK` and `TPTSE` toggle: the generator is running.
/// - `MCLK1` and `-TSE1` toggle with it: they are ungated.
/// - `CLK1` is held: `-CLK0` is `-TPCLK AND MACHRUN` at CLOCK2 1D10.
/// - `-WP1` and `-WP5` stay inactive: the write pulses are gated by
///   `MACHRUNA` at 1C10, so a halted machine writes nothing.
///
/// None of that gating is in the clock model. It is in the parts.
#[test]
fn the_clock_drives_the_board() {
    use muir::clock::Behavioural;
    use muir::part::Level;
    let n = netlist::parse(NETLIST).unwrap();
    let mut c = Chip::new(&n);
    c.power_on();
    c.settle();
    let mut clk = Behavioural::new();

    let watch = ["TPCLK", "TPTSE", "MCLK1", "-TSE1", "CLK1", "-WP1", "-WP5"];
    let ids: Vec<u32> = watch.iter().map(|w| n.by_name_id(w).unwrap()).collect();
    let mut seen: Vec<Vec<Level>> = vec![Vec::new(); watch.len()];

    let mut ns = 0u64;
    for _ in 0..120 {
        ns += c.tick(&mut clk) as u64;
        for (k, &id) in ids.iter().enumerate() {
            let level = c.net(id);
            if !seen[k].contains(&level) {
                seen[k].push(level);
            }
        }
        assert!(c.unsettled.is_none(), "the board stopped settling at {ns} ns");
    }
    for (k, name) in watch.iter().enumerate() {
        eprintln!("{name:8} {:?}", seen[k]);
    }
    eprintln!("{ns} ns of clock in 120 ticks");

    // Twelve cycles at the extra-slow 220 ns the speed bits come up at, ten
    // transitions each.
    assert!(ns > 2000, "the clock did not advance: {ns} ns");
    let moved = |name: &str| {
        let k = watch.iter().position(|w| *w == name).unwrap();
        seen[k].len() > 1
    };
    assert!(moved("TPCLK") && moved("TPTSE"), "the generator is not running");
    assert!(moved("MCLK1") && moved("-TSE1"), "the ungated phases did not follow");
    assert!(!moved("CLK1"), "CLK1 moved while the machine was halted");
    assert_eq!(seen[watch.iter().position(|w| *w == "-WP1").unwrap()], [Level::High]);
    assert_eq!(seen[watch.iter().position(|w| *w == "-WP5").unwrap()], [Level::High]);
    assert_eq!(c.net(n.by_name_id("MACHRUN").unwrap()), Level::Low, "should be halted");
}

/// The boot PROM reaches the instruction bus.
///
/// The image is split across six chips by [`Chip::load_prom`], which works
/// out each chip's slice from the netlist. This reads it back off the `I`
/// nets, which is the long way round: image to cells to pins to nets, with
/// the chip enables and the address inverters in between.
#[test]
fn the_prom_reads_back_onto_the_instruction_bus() {
    use muir::part::Level;
    let n = netlist::parse(NETLIST).unwrap();
    let image: Vec<u64> = muir::prom::boot_prom_image();
    let mut c = Chip::new(&n);
    c.power_on();
    assert_eq!(c.load_prom(&n, &image), 6, "chips in the first bank");
    c.settle();

    // Word 0 of the image, as the I bus should show it. Bit 46 is not in the
    // PROM at all, so it is left out of the comparison.
    let i_bus = |c: &Chip| {
        (0..49u32).filter(|b| *b != 46).fold(0u64, |w, b| {
            let id = n.by_name_id(&format!("I{b}")).unwrap();
            w | ((c.net(id) == Level::High) as u64) << b
        })
    };
    let want = |word: u64| {
        // The image's bits 46 and 47 sit on I47 and I48.
        (word & 0x3fff_ffff_ffff) | ((word >> 46 & 1) << 47) | ((word >> 47 & 1) << 48)
    };
    let got = i_bus(&c);
    eprintln!("PC 0: I bus {got:#015x}, image word {:#015x}", image[0]);
    assert_eq!(got, want(image[0]), "the I bus does not match image word 0");
    assert_ne!(image[0], 0, "the image is empty");
}

/// The `chip` engine boots.
///
/// Press the boot button and the machine starts: `-BOOT` is the asynchronous
/// preset of the `RUN` flip-flop at OLORD1 1A14, so no console protocol is
/// needed to start it, which is what the button is for. `SRUN` then latches
/// on `MCLK5A` and `MACHRUN` follows at 1A15.
///
/// The PC then walks the reset vector: word 0 of the PROM is
/// `jump pc 45`, and 045 is `GO`, so the machine runs `00000 00045 00046
/// 00047` --- here through the real PROM chips, the inverted address
/// decoders, the instruction register and the jump logic.
///
/// It then runs the PROM's own register self-test, 32 conditional jumps that
/// branch to `ERROR-BAD-BIT` on any failure, and passes it.
#[test]
fn chip_boots_the_prom() {
    use muir::clock::Behavioural;
    use muir::part::Level;
    let n = netlist::parse(NETLIST).unwrap();
    let image: Vec<u64> = muir::prom::boot_prom_image();
    let mut c = Chip::new(&n);
    c.power_on();
    c.load_prom(&n, &image);
    c.settle();

    let pc_nets: Vec<u32> = (0..14).map(|b| n.by_name_id(&format!("PC{b}")).unwrap()).collect();
    let pc = |c: &Chip| {
        pc_nets
            .iter()
            .enumerate()
            .fold(0u32, |w, (b, &id)| w | ((c.net(id) == Level::High) as u32) << b)
    };
    let machrun = n.by_name_id("MACHRUN").unwrap();
    let boot = n.by_name_id("-BOOT1").unwrap();

    assert_eq!(c.net(machrun), Level::Low, "should come up halted");

    let mut clk = Behavioural::new();
    c.set_net(boot, Level::Low);
    // The preset is asynchronous but shows on the flop's output once the
    // parts have been updated, which a settle alone does not do and a tick
    // does; see `part::ff`.
    c.tick(&mut clk);
    assert_eq!(c.net(n.by_name_id("RUN").unwrap()), Level::High, "-BOOT presets RUN");
    for _ in 0..19 {
        c.tick(&mut clk);
    }
    c.set_net(boot, Level::High);

    let mut seen: Vec<u32> = vec![pc(&c)];
    for _ in 0..150 {
        c.tick(&mut clk);
        let now = pc(&c);
        if *seen.last().unwrap() != now {
            seen.push(now);
        }
    }
    assert_eq!(c.net(machrun), Level::High, "the machine did not start");
    let octal: Vec<String> = seen.iter().take(8).map(|p| format!("{p:o}")).collect();
    eprintln!("first PCs: {octal:?}");
    // The 1 is the cycle nopped behind the jump, which retires nothing.
    // Past 047 is the boot PROM's own register self-test, which branches to
    // 012 (`ERROR-BAD-BIT`) on any failure --- so getting to 050 and beyond
    // means the machine passed it.
    assert_eq!(
        &seen[..8],
        &[0, 1, 0o45, 0o46, 0o47, 0o50, 0o51, 0o52],
        "the reset vector and the start of the self-test"
    );
}

/// The boot PROM's labels, from MIT's own symbol table.
///
/// Used to checkpoint a long run at named points rather than at arbitrary
/// counts: `GET-NEXT-PAGE` is a place you can mean to go back to, microcycle
/// 380,000 is not. The table is MIT's own, committed at
/// `mit/sys/ubin/promh.sym`.
///
/// Lines are `NAME SPACE ADDRESS`, sometimes with a leading number, and the
/// address is octal. Only `I-MEM` entries are microcode addresses.
fn prom_labels() -> std::collections::BTreeMap<u64, String> {
    let mut out = std::collections::BTreeMap::new();
    let text = include_str!("../mit/sys/ubin/promh.sym");
    for line in text.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.len() >= 3
            && t[t.len() - 2] == "I-MEM"
            && let Ok(a) = u64::from_str_radix(t[t.len() - 1], 8)
        {
            out.insert(a, t[t.len() - 3].to_string());
        }
    }
    out
}

/// A machine with the System 100 pack on unit 0, or `None` with the skip
/// line when the release has not been fetched. Both machines of a
/// comparison need one --- `rtl`'s own and the far end of `chip`'s cables
/// --- or the boot waits at `DISK-RECALIBRATE` for a drive, and the
/// comparison ends there; a clone is a second handle on the same read-only
/// image with its own written blocks, so one call serves both.
fn machine_with_pack() -> Option<muir::machine::Machine> {
    use muir::disk_unit::{Geometry, Unit};
    let p = support::pack_100()?;
    let mut m = muir::machine::Machine::new();
    m.disk.attach(0, Unit::open(&p, Geometry::T300).expect("the System 100 pack"));
    Some(m)
}

/// The net that says the cpu clock ran: `-CLK0` is `-TPCLK AND MACHRUN` at
/// CLOCK2 1D10, high in the write phase of a cycle the cpu runs and held low
/// through one it does not.
///
/// A `WAIT` drops `MACHRUN` while the generator runs on, so the cpu sits the
/// generator cycle out and no register is clocked at its end. `rtl` charges
/// a stall to the microcycle it delays, so a generator cycle the cpu sat out
/// is time to add and not a microcycle to compare. The first one in the boot
/// is the `WAIT` at `PAGE-0-PARITY-FIX`, microcycle 536,301, and counting
/// generator cycles instead put the two engines one microcycle out of step
/// from there on. Every comparison below runs the board until this net has
/// risen, then compares.
fn cpu_clock(n: &netlist::Netlist) -> netlist::NetId {
    n.by_name_id("-CLK0").unwrap()
}

/// How long a cpu microcycle may take before a comparison gives up on it,
/// in nanoseconds of the machine's time. The longest stall in the boot is
/// the bus timeout: ten microseconds for a device that is not there, twenty
/// for a hung one, thirty when referencing another processor
/// (`busint::TIMEOUT_NS`); this is twice the longest. A `HANG` stops the
/// generator while `FarEnd::tick_with` carries the bus and the delay lines
/// on to whatever ends it, and panics on a hang nothing can end; so this is
/// for a cycle that never comes for some other reason.
///
/// It was a count of far-end transitions, 2048, until the display board
/// went on the backplane: a 64 MHz board that never sleeps gave the far end
/// that many transitions in well under the ten microseconds the boot's
/// memory-size probe legitimately waits at `10000000`, and the guard called
/// a timeout a hang. Time is what the bound was always about.
const HANG_BOUND_NS: u64 = 60_000;

/// Writes a checkpoint: the microcycle, the processor, the clock and the
/// bus interface board, in that order.
fn checkpoint(
    p: &std::path::Path,
    cycle: usize,
    c: &Chip,
    clk: &muir::clock::Behavioural,
    far: &FarEnd,
) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(p).unwrap());
    std::io::Write::write_all(&mut f, &(cycle as u64).to_le_bytes()).unwrap();
    c.save(&mut f).unwrap();
    clk.save(&mut f).unwrap();
    far.save(&mut f).unwrap();
}

/// How many microcycles `rtl` runs before it is where `chip` is when the
/// board first leaves PC 0.
///
/// Two: the cycle the boot trap nops, and the jump at 0 that follows it.
/// `chip` spends its start-up in the same place, but it is measured there by
/// watching the PC rather than counted.
const RTL_START_STEPS: usize = 2;

/// Picks a run up from the checkpoint `MUIR_RESUME` names, if it names one,
/// and says which microcycle it resumed at. A `MUIR_RESUME` that names a
/// file which is not there is a mistake, not a cold run, and fails.
///
/// The processor, the clock and the bus interface board come from the
/// file. `rtl` catches up by being run, which is why it is not stored: it
/// has had [`RTL_START_STEPS`] more than the loop count, the alignment
/// before the loop and one at the top of each iteration. And the far end
/// of the cables is given `rtl`'s machine: past the first bus cycle it has
/// state --- main memory as the boot left it, the controller's registers
/// and the drive's attention, the pack's written blocks --- which is what
/// `rtl` holds once the two agree. A fresh far end put the disk's attention
/// flags a cycle out and looked like a divergence.
///
/// A checkpoint written before the board was stored ends after the clock,
/// and the board is built fresh instead. That is the board before its
/// first Unibus cycle --- not yet Unibus master, its registers clear ---
/// and is only right for a checkpoint taken before the boot's first, at
/// 537,841; the one such file in use is `at-535000.chk`.
fn resume_from_checkpoint(
    c: &mut Chip,
    clk: &mut muir::clock::Behavioural,
    r: &mut muir::rtl::Rtl,
    far: &mut FarEnd,
) -> Option<usize> {
    use muir::clock::Clock;
    use muir::engine::Engine;
    let p = std::path::PathBuf::from(std::env::var("MUIR_RESUME").ok()?);
    assert!(p.exists(), "MUIR_RESUME names {}, and there is no such checkpoint", p.display());
    let mut f = std::io::BufReader::new(std::fs::File::open(&p).unwrap());
    let mut at = [0u8; 8];
    std::io::Read::read_exact(&mut f, &mut at).unwrap();
    let at = u64::from_le_bytes(at) as usize;
    c.load(&mut f).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    *clk = muir::clock::Behavioural::load(&mut f).unwrap();
    let eof = |e: &std::io::Error| e.kind() == std::io::ErrorKind::UnexpectedEof;
    let board = match far.board.load(&mut f) {
        Ok(()) => "with the bus interface board as it was",
        Err(e) if eof(&e) => {
            "with a fresh bus interface board, which is only right before the boot's first Unibus cycle"
        }
        Err(e) => panic!("{}: the bus interface board: {e}", p.display()),
    };
    // The memory boards, if the checkpoint has them; else their cells are
    // filled from `rtl`'s memory once it has caught up.
    let memory = match far.xbus.load(&mut f) {
        Ok(()) => "the memory boards as they were",
        Err(e) if eof(&e) => "the memory boards filled from rtl's memory",
        Err(e) => panic!("{}: the memory boards: {e}", p.display()),
    };
    let io = match far.load_io_board(&mut f) {
        Ok(()) => "the I/O board as it was",
        Err(e) if eof(&e) => {
            "a fresh I/O board, which is only right before the boot's Unibus reset"
        }
        Err(e) => panic!("{}: the I/O board: {e}", p.display()),
    };
    let devices = match far.load_device_boards(&mut f) {
        Ok(()) => "the device boards as they were",
        Err(e) if eof(&e) => {
            // Powered on now, not at time 0, so their oscillators start here.
            far.xbus.repower_devices(clk.time_ns());
            "fresh device boards, the display's buffer empty until redrawn"
        }
        Err(e) => panic!("{}: the device boards: {e}", p.display()),
    };
    for _ in 0..at + RTL_START_STEPS {
        r.step().expect("rtl halted while catching up");
    }
    far.buses.machine = r.m.clone();
    // And its memory twins, where main memory is twins here too.
    far.buses.memory = r.busint().memory_boards().to_vec();
    if memory.starts_with("the memory boards filled") {
        far.xbus.load_from(&r.m.main);
    }
    far.join(c, clk.time_ns());
    eprintln!(
        "resumed from {} at microcycle {at}, {board}, {memory}, {io}, {devices}",
        p.display()
    );
    Some(at)
}

/// What the backplane and the device boards look like when the cpu clock
/// has stopped: the interface's Xbus address and handshake, and each device
/// board's own request path, by name. Printed before the hang assertion so
/// that a cycle no board answers can be read rather than guessed at.
fn hang_dump(far: &FarEnd) {
    use muir::part::Level;
    let bus_n = netlist::parse(BUSINT).unwrap();
    let net = |n: &netlist::Netlist, name: &str| {
        find_net(n, name).or_else(|| {
            let squeezed: String = name.chars().filter(|c| *c != ' ').collect();
            n.nets
                .iter()
                .position(|x| {
                    x.trim_matches('\'').chars().filter(|c| *c != ' ').eq(squeezed.chars())
                })
                .map(|i| i as netlist::NetId)
        })
    };
    let lv = |l: Level| match l {
        Level::Low => "0",
        Level::High => "1",
        Level::Z => "Z",
        Level::X => "X",
    };
    let mut addr = 0u32;
    for k in 0..22 {
        if let Some(id) = net(&bus_n, &format!("-XADDR{k}"))
            && far.board.net(id) == Level::Low
        {
            addr |= 1 << k;
        }
    }
    let mut line = format!("hang: interface XADDR {addr:o}");
    for name in
        ["-XBUS RQ", "-XBUS ACK", "-XBUS WR", "-XBUS INIT", "-XBUS POWER RESET", "-XBUS SYNC"]
    {
        if let Some(id) = net(&bus_n, name) {
            line += &format!(" {name}={}", lv(far.board.net(id)));
        }
    }
    eprintln!("{line}");
    let tv_n = netlist::parse(match std::env::var("MUIR_TV_BOARD").as_deref() {
        Ok("lispm-tv") => LISPMTV,
        _ => SIMPLETV,
    })
    .unwrap();
    for (j, d) in far.xbus.devices.iter().enumerate() {
        let mut line = format!("hang: device {j}");
        for name in [
            "-XBUS.RQ",
            "-XBUS.ACK",
            "-XBUS.INIT",
            "XBUS INIT IN",
            "-RESET",
            "-POWER RESET",
            "XBUS RQ IN",
            "MAP RQ",
            "CTL RQ",
            "PROC SYNC IN",
            "PROC SYNC",
            "PROC CYC",
            "-PROC CYC ACTIVE",
            "RQ T0",
            "-ACK T3",
            "SEND ACK",
            "16 MHZ CLK0",
            "SYNC ADR 0",
            "SYNC ADR 1",
            "SYNC ADR 2",
            "SYNC 4",
            "SYNC 5",
            "CLOCK MODE 0",
            "MODE BOW",
        ] {
            if let Some(id) = net(&tv_n, name) {
                line += &format!(" {name}={}", lv(d.net(id)));
            }
        }
        eprintln!("{line}");
    }
}

/// **`chip` agrees with `rtl` microcycle for microcycle**, over the window
/// `MUIR_COSIM_CYCLES` gives it, 40 by default: the flags and registers
/// the two expose, compared at every phase of every microcycle from the
/// button, and the first difference is the failure. The far end answers
/// `chip`'s cables with the bus interface netlist and the boards behind
/// it, so what is held to `rtl` is the whole machine.
///
/// `rtl` computes the whole datapath and `chip` resolves the parts that
/// implement it, so where the two disagree one of them has misread a
/// drawing. The comparison is not made at a single instant. Half the
/// datapath is tri-state buses that are only driven while their enable is
/// on, and an undriven TTL input reads high --- so at any given moment a
/// perfectly correct bus may read all ones. `rtl` has no phases at all and
/// holds one value for the whole microcycle. So the question this asks is:
/// **does `chip` take `rtl`'s value at any point in the cycle?** If it does,
/// the two agree and the difference is when it was looked at. If it never
/// does, that is a real disagreement, and it has an address on it.
///
/// This is a debugging instrument as much as a test: `MUIR_WATCH`,
/// `MUIR_TRACE_FROM`, `MUIR_CHECKPOINT_DIR`, `MUIR_CHECKPOINT_AT`,
/// `MUIR_RESUME` and `MUIR_SCREEN` are described where they are read.
#[test]
fn chip_agrees_with_rtl() {
    use muir::clock::{Behavioural, Clock};
    use muir::engine::Engine;
    use muir::part::Level;
    use muir::rtl::Rtl;

    let n = netlist::parse(NETLIST).unwrap();
    let image: Vec<u64> = muir::prom::boot_prom_image();

    let Some(pack) = machine_with_pack() else { return };
    let mut m = pack.clone();
    m.load_prom(&muir::prom::boot_prom());
    let mut r = Rtl::new(m);
    r.boot();

    let mut c = Chip::new(&n);
    c.power_on();
    c.load_prom(&n, &image);
    c.settle();
    let mut clk = Behavioural::new();
    let boot = n.by_name_id("-BOOT1").unwrap();
    // The far end of the cables, so that `chip` can finish a memory cycle at
    // all: nothing on this board drives `-MEMACK`. It is the bus interface
    // board, a netlist too, with the Xbus and the Unibus behind it answered
    // from a `Machine` that holds main memory and the devices, as `rtl`'s
    // does --- the same model, so the two answer the bus the same way.
    // Only main memory and the devices: the control store is on the board.
    let mut far = far_end(&n, pack);
    far.join(&mut c, clk.time_ns());

    // Picking a run up again. A microcycle costs about 1.3 ms, so reaching
    // the interesting part of the boot takes ten minutes.
    //
    // `MUIR_CHECKPOINT_DIR` names a directory to write one checkpoint per
    // PROM label into, the first time the machine arrives there;
    // `MUIR_RESUME` names one to start from. Only `chip` and the clock are
    // stored --- `rtl` runs the whole trace in under a second, so it is
    // replayed rather than saved.
    let labels = prom_labels();
    let dir = std::env::var("MUIR_CHECKPOINT_DIR").ok().map(std::path::PathBuf::from);
    if let Some(d) = &dir {
        std::fs::create_dir_all(d).unwrap();
    }
    // `MUIR_CHECKPOINT_AT` names microcycles to checkpoint at, comma
    // separated and ascending, into the same directory or `vendor/run/chk`,
    // for picking a run up short of a place no PROM label is near ---
    // microcode 323 has none. Each is taken at the first quiet microcycle
    // from there: a checkpoint holds the two boards and the clock and not
    // the cables, the buses or the delay lines, so a bus cycle or a tap in
    // flight would be lost, and a read hung on a `-RDFINISH` that never
    // comes stays hung.
    let mut checkpoint_at: std::collections::VecDeque<usize> = std::env::var("MUIR_CHECKPOINT_AT")
        .ok()
        .map(|v| v.split(',').map(|s| s.parse().unwrap()).collect())
        .unwrap_or_default();
    let memrq = n.by_name_id("MEMRQ").unwrap();
    let checkpoint_dir = dir
        .clone()
        .unwrap_or_else(|| [env!("CARGO_MANIFEST_DIR"), "vendor", "run", "chk"].iter().collect());
    let resumed = resume_from_checkpoint(&mut c, &mut clk, &mut r, &mut far).unwrap_or(0);

    let width = |name: &str| match name {
        "PC" => 14,
        // 48, not 49: `rtl` holds `IR<47:0>` and does not model `IR48`,
        // the control store's parity bit, which the netlist does have.
        "IR" => 48,
        // `DC0..DC9`, the dispatch constant the 25S07s at DSPCTL hold.
        "DC" => 10,
        // `OPC0..OPC13`, the last stage of the OPCS shift registers.
        "OPC" => 14,
        _ => 32,
    };
    // Resolved once. Every bus is read at every clock transition, and
    // looking each bit up by name costs a `format!` and a hash lookup ---
    // which was the majority of this test's running time.
    let watch: Vec<Vec<u32>> =
        r.signals().iter().map(|(name, _)| c.bus_nets(&n, name, width(name))).collect();
    let pc_nets = c.bus_nets(&n, "PC", 14);
    let clk0 = cpu_clock(&n);
    // `MUIR_WATCH=<from>[-<to>]:<net>,<net>,...` prints the named nets at
    // every clock transition of those microcycles, a bus as `NAME/width`,
    // with the clock's phase --- for looking inside one cycle the comparison
    // has pointed at.
    let watch_spec = std::env::var("MUIR_WATCH").ok();
    let (watch_at, watch_names) = watch_spec.as_deref().and_then(|v| v.split_once(':')).unzip();
    let watch_range = watch_at.map(|at| {
        let (from, to) = at.split_once('-').unwrap_or((at, at));
        from.parse::<usize>().unwrap()..=to.parse().unwrap()
    });
    let watch_nets: Vec<(&str, Vec<netlist::NetId>)> = watch_names
        .unwrap_or("")
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let ids = match s.split_once('/') {
                Some((name, w)) => c.bus_nets(&n, name, w.parse().unwrap()),
                None => vec![n.by_name_id(s).unwrap_or_else(|| panic!("no net {s}"))],
            };
            (s, ids)
        })
        .collect();
    // `MUIR_TRACE_FROM=<microcycle>` prints every change on the wires that
    // carry a bus cycle from that microcycle on, on both boards, with the
    // Xbus or Unibus address at each request: for looking at a cycle the
    // comparison stopped in.
    let trace_from: Option<usize> =
        std::env::var("MUIR_TRACE_FROM").ok().and_then(|v| v.parse().ok());
    let bus_n = netlist::parse(BUSINT).unwrap();
    let bus_net = |name: &str| net_named(&bus_n, name);
    // Where a traced net is: the processor, the interface, or the first
    // memory board.
    const CPU: u8 = 0;
    const INTERFACE: u8 = 1;
    const MEMORY: u8 = 2;
    const IOB: u8 = 3;
    let io_n = netlist::parse(CADRIO).unwrap();
    let io_net = |name: &str| net_named(&io_n, name);
    let mem_n = netlist::parse(CADRM).unwrap();
    let mem_net = |name: &str| net_named(&mem_n, name);
    let traced: Vec<(&str, u8, netlist::NetId)> = [
        ("cpu MEMRQ", CPU, "MEMRQ"),
        ("cpu WRCYC", CPU, "WRCYC"),
        ("cpu -MEMACK", CPU, "-MEMACK"),
        ("cpu -MEMGRANT", CPU, "-MEMGRANT"),
        ("cpu -HANG", CPU, "-HANG"),
        ("cpu -WAIT", CPU, "-WAIT"),
        ("cpu MBUSY", CPU, "MBUSY"),
        ("cpu -LOADMD", CPU, "-LOADMD"),
        ("cpu INT", CPU, "INT"),
        ("busint -XBUS RQ", INTERFACE, "-XBUS RQ"),
        ("busint -XBUS ACK", INTERFACE, "-XBUS ACK"),
        ("busint -XBUS WR", INTERFACE, "-XBUS WR"),
        ("busint -XBUS INTR", INTERFACE, "-XBUS INTR"),
        ("busint ADR=UNIBUS", INTERFACE, "ADR=UNIBUS"),
        ("busint LMX GRANT", INTERFACE, "LMX GRANT"),
        ("busint LM NEED UB", INTERFACE, "LM NEED UB"),
        ("busint LMUB MASTER", INTERFACE, "LMUB MASTER"),
        ("busint LMUB GRANT", INTERFACE, "LMUB GRANT"),
        ("busint -UB MSYN", INTERFACE, "-UB MSYN"),
        ("busint -UB SSYN", INTERFACE, "-UB SSYN"),
        ("busint NXM TIMEOUT", INTERFACE, "NXM TIMEOUT"),
        ("busint INT BUSY", INTERFACE, "INT BUSY"),
        ("busint -LOADMD", INTERFACE, "-LOADMD"),
        ("busint -LMACK", INTERFACE, "-LMACK"),
        ("busint -SPY WRITE", INTERFACE, "-SPY WRITE"),
        ("busint RESET", INTERFACE, "RESET"),
        ("busint -XBUS INIT", INTERFACE, "-XBUS INIT"),
        ("busint -UB INIT", INTERFACE, "-UB INIT"),
        ("busint -XBUS SYNC", INTERFACE, "-XBUS SYNC"),
        ("iob -INIT*", IOB, "-INIT*"),
        ("iob -MSYN*", IOB, "-MSYN*"),
        ("iob -SSYN*", IOB, "-SSYN*"),
        ("iob 1 USEC CLK", IOB, "1 USEC CLK"),
        ("iob KBD READY", IOB, "KBD READY"),
        ("iob MOUSE READY", IOB, "MOUSE READY"),
        ("iob CLOCK READY", IOB, "CLOCK READY"),
        ("iob KB CLK^", IOB, "KB CLK^"),
        ("iob KBD SR IN", IOB, "KBD SR IN"),
        ("iob EOC.KBD^", IOB, "EOC.KBD^"),
        ("busint -UBD0", INTERFACE, "-UBD0"),
        ("busint -UBD1", INTERFACE, "-UBD1"),
        ("busint -UBD14", INTERFACE, "-UBD14"),
        ("busint -UBD15", INTERFACE, "-UBD15"),
        ("iob -D0*", IOB, "-D0*"),
        ("iob -D1*", IOB, "-D1*"),
        ("iob -D14*", IOB, "-D14*"),
        ("iob -D15*", IOB, "-D15*"),
        ("memory -BUSY", MEMORY, "-BUSY"),
        ("memory REFRESH CYC", MEMORY, "REFRESH CYC"),
        ("memory REFRESH RQ", MEMORY, "REFRESH RQ"),
        ("memory TIME FOR REFRESH", MEMORY, "TIME FOR REFRESH"),
        ("memory BOARD SELECT", MEMORY, "BOARD SELECT"),
        ("memory -RESET", MEMORY, "-RESET"),
        ("memory XB RQ", MEMORY, "XB RQ"),
        ("memory IDLE", MEMORY, "IDLE"),
        ("memory -T0", MEMORY, "-T0"),
        ("memory -REFRESH NOW", MEMORY, "-REFRESH NOW"),
        ("memory REFRESH CYC", MEMORY, "REFRESH CYC"),
        ("memory XBUS CLK", MEMORY, "XBUS CLK"),
    ]
    .iter()
    .map(|&(label, on, name)| {
        let id = match on {
            CPU => n.by_name_id(name).unwrap_or_else(|| panic!("no net {name}")),
            INTERFACE => bus_net(name),
            IOB => io_net(name),
            _ => mem_net(name),
        };
        (label, on, id)
    })
    .collect();
    let xaddr: Vec<netlist::NetId> = (0..22).map(|b| bus_net(&format!("-XADDR{b}"))).collect();
    let ubaddr: Vec<netlist::NetId> = (0..18).map(|b| bus_net(&format!("-UB ADR{b}"))).collect();
    let time_for_refresh = mem_net("TIME FOR REFRESH");
    let mut traced_last: Vec<Level> = vec![Level::X; traced.len()];

    // `chip` spends a few microcycles starting up after the button, where
    // `rtl` begins executing at once.
    if resumed == 0 {
        c.set_net(boot, Level::Low);
        c.settle();
        for _ in 0..20 {
            c.tick(&mut clk);
        }
        c.set_net(boot, Level::High);
        let mut skipped = 0;
        while c.read(&pc_nets) == 0 && skipped < 40 {
            c.microcycle(&mut clk);
            skipped += 1;
        }
        let mut steps = 0;
        while r.pc() as u64 != c.read(&pc_nets) && steps < 8 {
            r.step().unwrap();
            steps += 1;
        }
        assert_eq!(steps, RTL_START_STEPS, "rtl took a different number of steps to start");
        eprintln!(
            "aligned after {skipped} start-up microcycles at PC {:o}, {} ns after the button",
            c.read(&pc_nets),
            clk.time_ns()
        );
    }

    // Long enough to be worth running, short enough for a debug build.
    // `MUIR_COSIM_CYCLES` raises it for a hunt: `cargo test --release`.
    let limit: usize =
        std::env::var("MUIR_COSIM_CYCLES").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
    let mut first: Option<(usize, String, u64)> = None;
    let mut cycles = 0;
    let mut ns_was = (clk.time_ns(), r.ns());
    // Microcycles are a poor measure of how much of the machine has been
    // exercised: the boot PROM spends 2^18 of them in one loop at 0322 and
    // 2^16 in another at 0240, so a long run can be a short program. What
    // matters is how many distinct microinstructions have actually been
    // compared.
    let mut reached = std::collections::BTreeSet::new();
    // The PC the last iteration ended on, so a label is checkpointed with
    // the state as it stands *at* the top of an iteration --- which is the
    // only point the loop can be re-entered at.
    let mut arrived: Option<u64> = None;
    // Generator cycles the cpu sat out, which are `rtl`'s waits.
    let mut sat_out = 0usize;
    for cycle in resumed..limit {
        if let (Some(d), Some(pc)) = (&dir, arrived.take())
            && let Some(label) = labels.get(&pc)
        {
            let safe: String = label
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' { ch } else { '_' })
                .collect();
            let p = d.join(format!("{pc:o}-{safe}.chk"));
            if !p.exists() && far.quiet() && c.next_tap().is_none() && c.net(memrq) != Level::High {
                checkpoint(&p, cycle, &c, &clk, &far);
                eprintln!("microcycle {cycle}: reached {label} ({pc:o}), checkpointed");
            }
        }
        if checkpoint_at.front().is_some_and(|&at| cycle >= at)
            && far.quiet()
            && c.next_tap().is_none()
            && c.net(memrq) != Level::High
        {
            checkpoint_at.pop_front();
            std::fs::create_dir_all(&checkpoint_dir).unwrap();
            let p = checkpoint_dir.join(format!("at-{cycle}.chk"));
            checkpoint(&p, cycle, &c, &clk, &far);
            eprintln!("microcycle {cycle}: checkpointed to {}", p.display());
        }
        // `rtl` runs the microinstruction first, because [`Rtl::signals`]
        // reports the microcycle it just executed rather than recomputing
        // one that has not happened. A halt is a failure, not the end of the
        // comparison: nothing would have been compared past it.
        r.step().unwrap_or_else(|h| panic!("rtl halted at microcycle {cycle}: {h:?}"));
        let want = r.signals();
        reached.insert(want[0].1);
        arrived = Some(want[0].1);
        // Every value each signal takes anywhere in this microcycle.
        // Only samples where every bit of the bus is actually driven. A
        // floating bus is not a value, and comparing one against `rtl` --- which
        // always has a value --- invents agreement or disagreement out of
        // nothing.
        let mut seen: Vec<Vec<u64>> =
            watch.iter().map(|w| c.read_driven(w).into_iter().collect()).collect();
        // Exactly one microcycle: clock transitions until the next cycle
        // begins. Sampling a fixed number of transitions instead runs the
        // machine ahead of `rtl`, which shows up as a phantom divergence.
        // And one *cpu* microcycle: see [`cpu_clock`].
        let mut ran = false;
        let hang_from = muir::clock::Clock::time_ns(&clk);
        for t in 0.. {
            far.tick_with(&mut c, &mut clk);
            ran |= c.net(clk0) == Level::High;
            if trace_from.is_some_and(|from| cycle >= from) {
                eprintln!(
                    "  rtl at {} ns cycle {cycle}: memory twin {:?}",
                    r.ns(),
                    r.busint().memory_board(0)
                );
                eprintln!("  rtl at {} ns cycle {cycle}: I/O twin {:?}", r.ns(), r.busint().io);
                if far.xbus.boards.is_empty() {
                    eprintln!(
                        "  far end at {} ns cycle {cycle}: memory twin {:?}",
                        clk.time_ns(),
                        far.buses.memory[0]
                    );
                }
                for (k, &(label, on, id)) in traced.iter().enumerate() {
                    let l = match on {
                        CPU => c.net(id),
                        INTERFACE => far.board.net(id),
                        IOB => far.unibus.as_ref().map_or(Level::X, |u| u.board.net(id)),
                        _ => far.xbus.boards.first().map_or(Level::X, |b| b.net(id)),
                    };
                    if l != traced_last[k] {
                        let addr = if label == "busint -XBUS RQ" && l == Level::Low {
                            format!(" address {:o}", !(far.board.read(&xaddr) as u32) & 0x3f_ffff)
                        } else if label == "busint -UB MSYN" && l == Level::Low {
                            format!(
                                " unibus address {:o}",
                                !(far.board.read(&ubaddr) as u32) & 0o777777
                            )
                        } else {
                            String::new()
                        };
                        eprintln!(
                            "  {} ns cycle {cycle} PC {:o}: {label}={l:?}{addr}",
                            clk.time_ns(),
                            c.read(&pc_nets)
                        );
                        traced_last[k] = l;
                    }
                }
            }
            if watch_range.as_ref().is_some_and(|r| r.contains(&cycle)) {
                let line: Vec<String> = watch_nets
                    .iter()
                    .map(|(name, ids)| match (ids.len(), c.read_driven(ids)) {
                        (1, _) => format!("{name}={:?}", c.net(ids[0])),
                        (_, Some(v)) => format!("{name}={v:o}"),
                        (_, None) => format!("{name}=Z"),
                    })
                    .collect();
                eprintln!("  {cycle} @{:>4} ns: {}", clk.phase_ns(), line.join(" "));
            }
            for (k, nets) in watch.iter().enumerate() {
                if let Some(got) = c.read_driven(nets)
                    && !seen[k].contains(&got)
                {
                    seen[k].push(got);
                }
            }
            if clk.phase_ns() == 0 {
                if ran {
                    break;
                }
                sat_out += 1;
            }
            let stalled = muir::clock::Clock::time_ns(&clk) - hang_from;
            if stalled >= HANG_BOUND_NS {
                eprintln!(
                    "hang: at {} ns, {t} far-end transitions and {stalled} ns since the cpu clock last ran",
                    clk.time_ns()
                );
                hang_dump(&far);
            }
            assert!(
                muir::clock::Clock::time_ns(&clk) - hang_from < HANG_BOUND_NS,
                "microcycle {cycle}: the cpu clock has not run in {HANG_BOUND_NS} ns ({t} far-end transitions) \
                 (chip PC {:o})",
                c.read(&pc_nets)
            );
        }

        // The memory boards' refresh timer against `rtl`'s twin of it: the
        // first board's `TIME FOR REFRESH` is up exactly when the twin says
        // the one-shot has run out, and a refresh that lands on a different
        // bus cycle in the two would otherwise only show as a wait.
        // With main memory as twins on this side too, the far end's twin,
        // clocked off the netlist interface's `-XBUS SYNC`, is held to
        // `rtl`'s, clocked off its own microcycles.
        {
            let board_time = match far.xbus.boards.first() {
                Some(b) => b.net(time_for_refresh) == Level::High,
                None => far.buses.memory[0].time_for_refresh(clk.time_ns()),
            };
            let twin_time = r.busint().memory_board(0).time_for_refresh(r.ns());
            assert_eq!(
                board_time,
                twin_time,
                "microcycle {cycle}: TIME FOR REFRESH on the board at {} ns against rtl's twin at {} ns: {:?}; the far end's twin {:?}",
                clk.time_ns(),
                r.ns(),
                r.busint().memory_board(0),
                far.buses.memory[0]
            );
        }

        // Simulated time. `rtl` accumulates it from the delay-line taps and
        // never ticks a clock; `chip` is driven by one. Over the same
        // microcycles the two must advance by the same nanoseconds, which is
        // what checks that `ILONG` and the speed bits are being read the same
        // way. Deltas, because `chip` has been running since the button.
        let ns_now = (clk.time_ns(), r.ns());
        if cycle > 0 {
            let (dc, dr) = (ns_now.0 - ns_was.0, ns_now.1 - ns_was.1);
            if dc != dr {
                // What sets a microcycle's length, so the message says which
                // of them the two engines read differently.
                let levels: Vec<String> =
                    ["SSPEED1", "SSPEED0", "SPEED1A", "SPEED1", "-ILONG", "MACHRUN"]
                        .iter()
                        .map(|name| format!("{name}={:?}", c.net(n.by_name_id(name).unwrap())))
                        .collect();
                panic!(
                    "microcycle {cycle}: chip took {dc} ns, rtl says {dr} (chip PC {:o}, rtl PC {:o} \
                     IR {:#x}; {})",
                    c.read(&pc_nets),
                    want[0].1,
                    want[1].1,
                    levels.join(" ")
                );
            }
        }
        ns_was = ns_now;

        for (k, (name, value)) in want.iter().enumerate() {
            if seen[k].contains(value) {
                continue;
            }
            // Every signal that differs in this microcycle, not just the
            // first: one wrong register usually shows up on several buses at
            // once, and which ones it reaches is the evidence.
            eprintln!(
                "  {name}: rtl {value:#x}, chip took {:?} (chip PC {:?})",
                seen[k].iter().map(|v| format!("{v:#x}")).collect::<Vec<_>>(),
                seen[0].iter().map(|v| format!("{v:o}")).collect::<Vec<_>>()
            );
            if first.is_none() {
                first = Some((cycle, name.to_string(), *value));
            }
        }
        cycles = cycle;
        if first.is_some() {
            break;
        }
    }
    eprintln!("the cpu sat out {sat_out} generator cycles on the bus");
    // `MUIR_SCREEN=<dir>` writes both screens at the end --- the far end of
    // `chip`'s cables and `rtl`'s machine --- as `screen-chip-<n>.png` and
    // `screen-rtl-<n>.png`, for comparing what the two boards drew.
    if let Ok(dir) = std::env::var("MUIR_SCREEN") {
        for (name, tv) in [("chip", &far.buses.machine.simpletv), ("rtl", &r.m.simpletv)] {
            let p = std::path::Path::new(&dir).join(format!("screen-{name}-{cycles}.png"));
            std::fs::write(&p, tv.png()).unwrap();
            eprintln!("{}: {} pixels lit", p.display(), tv.lit());
        }
    }
    match &first {
        Some((cycle, name, want)) => eprintln!(
            "microcycle {cycle}: chip never shows {name} = {want:#x} at any phase; \
             rtl PC={:o} IR={:#x}",
            r.signals()[0].1,
            r.signals()[1].1
        ),
        None => eprintln!(
            "chip and rtl agree over {} microcycles; memory board transitions {} ({} a microcycle), {} of {} asleep now",
            cycles + 1,
            far.xbus.transitions,
            far.xbus.transitions / (cycles as u64 + 1 - resumed as u64).max(1),
            far.xbus.boards.iter().filter(|b| b.asleep()).count(),
            far.xbus.boards.len()
        ),
    }
    eprintln!(
        "{} distinct PCs reached{}, last {:o}",
        reached.len(),
        if resumed > 0 { " since resuming" } else { "" },
        r.signals()[0].1
    );
    assert!(first.is_none(), "chip and rtl parted: {first:?}");
}

/// A gate must be evaluated after everything feeding it, unless the two are
/// in the same feedback group --- which is what a feedback group is.
///
/// If this is violated the sweep reads stale inputs, and a stale input on a
/// bus enable puts two drivers on one net. That resolves to unknown, and
/// unknown is absorbing: it goes round the loop and never clears. So a
/// moment of disorder becomes a permanently dead machine.
#[test]
fn every_group_comes_after_the_ones_feeding_it() {
    let n = netlist::parse(NETLIST).unwrap();
    let c = Chip::new(&n);
    let mut group_of = std::collections::HashMap::new();
    for (g, members) in c.groups().iter().enumerate() {
        for id in members {
            group_of.insert((id.part, id.gate), g);
        }
    }
    let mut wrong = Vec::new();
    for (consumer, &g) in &group_of {
        let id = muir::chip::GateId { part: consumer.0, gate: consumer.1 };
        for net in c.gate_inputs(id).into_iter().flatten() {
            for producer in c.drivers_on(net) {
                if let Some(&p) = group_of.get(&(producer.part, producer.gate))
                    && p > g
                {
                    wrong.push((
                        c.instances[producer.part as usize].name(),
                        c.instances[id.part as usize].name(),
                        p,
                        g,
                    ));
                }
            }
        }
    }
    eprintln!("{} cross-group edges point backwards", wrong.len());
    for w in wrong.iter().take(6) {
        eprintln!("   {} (group {}) feeds {} (group {})", w.0, w.2, w.1, w.3);
    }
    assert!(wrong.is_empty(), "{} edges evaluated out of order", wrong.len());
}

/// One of the board's memories: which chip holds which bit, and how a
/// logical address reaches a cell.
///
/// All of it is read off the netlist, because **every one of these memories
/// is wired differently** and a table written by hand would be a table of
/// guesses:
///
/// | memory | address pins | outputs |
/// |---|---|---|
/// | A, M, PDL | `-AADR0B` up, inverted | active high |
/// | microcode stack | `SPCPTR0` up, *not* inverted | active high |
/// | dispatch | `-DADR0A` up, inverted, `DADR10` selects the bank | active high |
/// | level-1 map | `MAPI22` down to `MAPI13`, **reversed**, `MAPI23` the bank | **active low** |
/// | level-2 map | `VMAP4A` down then `-MAPI12A` down, **reversed and half inverted** | **active low** |
///
/// So a pin's net name is parsed into a signal, a bit number and whether it
/// is inverted, and [`Mem::addr`] says what that signal's bit number means in
/// the logical address the other engine uses. Chip select is an address bit
/// too where it is not tied to a supply: the part is selected when its `CS`
/// net is low, so a chip whose `CS` is `-DADR10A` holds the half of the
/// dispatch memory with that bit set, and one whose `CS` is `DADR10A` holds
/// the other.
struct Mem {
    name: &'static str,
    pages: &'static [&'static str],
    /// Output net prefix and the data bit its number counts from. An exact
    /// match with no number is allowed, which is how the dispatch memory's
    /// `DN`, `DP` and `DR` sit above `DPC13`.
    data: &'static [(&'static str, u32)],
    /// The outputs are active low, so a stored bit is the complement of the
    /// value. Both map levels are.
    active_low: bool,
    words: usize,
    width: u32,
    /// Address signal prefix, and what to add to its bit number to get the
    /// bit of the logical address it carries.
    addr: &'static [(&'static str, i32)],
    /// The output net of the chip holding the word's parity bit, where the
    /// memory has one.  The scratchpads keep **odd parity over the word and
    /// its bit**: the 93S48s that check them --- APAR 3A28 and 4B15 for A
    /// and M, SPCPAR 4F26 for the stack --- say OK on their odd output, and
    /// the bit is written from `LPARITY`, the L register's parity off the
    /// 93S48 at L 4C09.  [`Ram::store`] writes it so; [`Ram::word`] does
    /// not read it.
    parity: Option<&'static str>,
}

const MEMS: &[Mem] = &[
    Mem {
        name: "A",
        pages: &["AMEM0", "AMEM1"],
        data: &[("AMEM", 0)],
        active_low: false,
        words: 1024,
        width: 32,
        addr: &[("AADR", 0)],
        parity: Some("AMEMPARITY"),
    },
    Mem {
        name: "M",
        pages: &["MMEM"],
        data: &[("MMEM", 0)],
        active_low: false,
        words: 32,
        width: 32,
        addr: &[("MADR", 0)],
        parity: Some("MMEMPARITY"),
    },
    Mem {
        name: "PDL",
        pages: &["PDL0", "PDL1"],
        data: &[("PDL", 0)],
        active_low: false,
        words: 1024,
        width: 32,
        addr: &[("PDLA", 0)],
        parity: Some("PDLPARITY"),
    },
    Mem {
        name: "SPC",
        pages: &["SPC"],
        data: &[("SPCO", 0)],
        active_low: false,
        words: 32,
        width: 19,
        addr: &[("SPCPTR", 0)],
        parity: Some("SPCOPAR"),
    },
    Mem {
        name: "DISPATCH",
        pages: &["DRAM0", "DRAM1", "DRAM2"],
        data: &[("DPC", 0), ("DN", 14), ("DP", 15), ("DR", 16)],
        active_low: false,
        words: 2048,
        width: 17,
        addr: &[("DADR", 0)],
        parity: None,
    },
    Mem {
        name: "L1MAP",
        pages: &["VMEM0"],
        data: &[("-VMAP", 0)],
        active_low: true,
        words: 2048,
        width: 5,
        addr: &[("MAPI", -13)],
        parity: None,
    },
    Mem {
        name: "L2MAP",
        pages: &["VMEM1", "VMEM2"],
        data: &[("-VMO", 0)],
        active_low: true,
        words: 1024,
        width: 24,
        addr: &[("MAPI", -8), ("VMAP", 5)],
        parity: None,
    },
];

/// One RAM chip's contribution: a data bit, and how to address it.
struct Slice {
    inst: usize,
    /// Which bit of the word this chip holds.
    bit: u32,
    /// Which bit of the cell byte that is --- the 82S21 holds two per cell.
    cell_bit: u32,
    /// (bit of the logical address, bit of the cell index, inverted).
    addr: Vec<(u32, u32, bool)>,
    /// The chip only answers when this bit of the logical address has this
    /// value. `None` where chip select is tied to a supply.
    bank: Option<(u32, bool)>,
}

struct Ram {
    slices: Vec<Slice>,
    /// The parity bit's chip, where [`Mem::parity`] names one.
    parity: Vec<Slice>,
    active_low: bool,
    words: usize,
}

/// Splits a net name into its sign, signal and bit number: `-MAPI12A` is
/// `MAPI` bit 12, inverted. The trailing letter is the drawing's way of
/// numbering buffered copies of one signal, as in `-AADR0B`.
fn split_net(name: &str) -> Option<(bool, &str, u32)> {
    let (inverted, rest) = match name.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, name),
    };
    let rest = match rest.as_bytes().last() {
        Some(b) if b.is_ascii_alphabetic() => &rest[..rest.len() - 1],
        _ => rest,
    };
    let at = rest.len() - rest.bytes().rev().take_while(u8::is_ascii_digit).count();
    if at == rest.len() {
        return None;
    }
    Some((inverted, &rest[..at], rest[at..].parse().ok()?))
}

impl Ram {
    fn new(c: &Chip, n: &netlist::Netlist, m: &Mem) -> Ram {
        // The 93425 is one bit per cell, out on pin 7, with chip select on
        // pin 1; the 82S21 is two, on pins 7 and 9, and its pin 1 is the
        // write enable rather than a select.
        let mut slices = Vec::new();
        let mut parity = Vec::new();
        for (i, inst) in c.instances.iter().enumerate() {
            if !m.pages.contains(&inst.page.as_str()) {
                continue;
            }
            let kind = part::strip(&inst.kind).0;
            let (outs, addr_pins, cs): (&[(u8, u32)], &[u8], Option<u8>) = match kind {
                "93425" => (&[(7, 0)], &[2, 3, 4, 5, 6, 9, 10, 11, 12, 13], Some(1)),
                "82S21" => (&[(7, 0), (9, 1)], &[13, 12, 11, 10, 4], None),
                _ => continue,
            };
            let net = |pin: u8| inst.net_on(pin).map(|x| n.net(x));
            // Where this chip's address pins take each bit of the logical
            // address, and where chip select puts it in the memory.
            let bit_of = |name: &str| -> Option<(u32, bool)> {
                let (inv, sig, k) = split_net(name)?;
                let (_, off) = m.addr.iter().find(|(p, _)| *p == sig)?;
                Some(((k as i32 + off) as u32, inv))
            };
            let mut addr = Vec::new();
            for (cell_bit, &pin) in addr_pins.iter().enumerate() {
                let name = net(pin).unwrap_or_else(|| panic!("{} pin {pin}", inst.name()));
                let (logical, inv) = bit_of(name)
                    .unwrap_or_else(|| panic!("{}: address pin {pin} is {name}", inst.name()));
                addr.push((logical, cell_bit as u32, inv));
            }
            let bank = cs.and_then(&net).and_then(|name| {
                if muir::chip::supply(name).is_some() {
                    return None;
                }
                let (held, _) = bit_of(name)
                    .unwrap_or_else(|| panic!("chip select {name} is not an address bit"));
                // Selected when the net is low: `-DADR10A` low means the
                // signal is high, a plain `DADR10A` low means it is low.
                Some((held, name.starts_with('-')))
            });
            for &(pin, cell_bit) in outs {
                let Some(name) = net(pin) else { continue };
                if m.parity == Some(name) {
                    parity.push(Slice { inst: i, bit: 0, cell_bit, addr: addr.clone(), bank });
                    continue;
                }
                let bit = m.data.iter().find_map(|&(prefix, base)| {
                    if name == prefix {
                        return Some(base);
                    }
                    name.strip_prefix(prefix)?.parse::<u32>().ok().map(|k| base + k)
                });
                if let Some(bit) = bit {
                    slices.push(Slice { inst: i, bit, cell_bit, addr: addr.clone(), bank });
                }
            }
        }
        let banks = if slices.iter().any(|s| s.bank.is_some()) { 2 } else { 1 };
        assert_eq!(
            slices.len() as u32,
            m.width * banks,
            "{}: found {} chip slices for {} bits in {banks} bank(s)",
            m.name,
            slices.len(),
            m.width
        );
        if m.parity.is_some() {
            assert_eq!(parity.len() as u32, banks, "{}: parity chip", m.name);
        }
        Ram { slices, parity, active_low: m.active_low, words: m.words }
    }

    fn word(&self, c: &Chip, a: usize) -> u32 {
        let mut w = 0;
        for s in &self.slices {
            // This chip answers only for its own half of the memory: skip
            // it when the address bit its chip select decodes is the other
            // value. Inverting this reads the wrong bank, which looks like a
            // write landing 1024 words away.
            if let Some((bit, held)) = s.bank
                && (a >> bit) & 1 != held as usize
            {
                continue;
            }
            let cell = s.addr.iter().fold(0usize, |k, &(logical, cell_bit, inv)| {
                let b = (a >> logical) & 1 == 1;
                k | ((b != inv) as usize) << cell_bit
            });
            let stored = (c.instances[s.inst].state.cells[cell] >> s.cell_bit) & 1 == 1;
            w |= ((stored != self.active_low) as u32) << s.bit;
        }
        w
    }

    fn len(&self) -> usize {
        self.words
    }
}

/// The diagnostic flags `Rtl::spy` reports, against the nets themselves.
///
/// These are what the console reads over the SPY bus, and six of them ---
/// `WMAPD`, `DESTSPCD`, `IWRITED`, `IMODD`, `PDLWRITED`, `SPUSHD` --- are the
/// ones an emulator can hold at constant `false` and still boot. `rtl` has
/// them because the write
/// pipeline cannot work without them, so this checks the datapath's own
/// registers rather than an interface bolted on beside it.
///
/// Sampled in the read phase, where `Rtl::spy` records them: `-TSE1` is
/// asserted from 25 ns and the flags are register outputs, so they are
/// settled and stable across it.
#[test]
fn chip_and_rtl_agree_on_the_spy_flags() {
    use muir::clock::{Behavioural, Clock};
    use muir::engine::Engine;
    use muir::part::Level;
    use muir::rtl::Rtl;

    let n = netlist::parse(NETLIST).unwrap();
    let image: Vec<u64> = muir::prom::boot_prom_image();

    let Some(pack) = machine_with_pack() else { return };
    let mut m = pack.clone();
    m.load_prom(&muir::prom::boot_prom());
    let mut r = Rtl::new(m);
    r.boot();

    let mut c = Chip::new(&n);
    c.power_on();
    c.load_prom(&n, &image);
    c.settle();
    let mut clk = Behavioural::new();
    let boot = n.by_name_id("-BOOT1").unwrap();
    c.set_net(boot, Level::Low);
    c.settle();
    for _ in 0..20 {
        c.tick(&mut clk);
    }
    c.set_net(boot, Level::High);
    let mut skipped = 0;
    while c.bus(&n, "PC", 14) == 0 && skipped < 40 {
        c.microcycle(&mut clk);
        skipped += 1;
    }
    // The far end of the cables, so the board can finish a memory cycle;
    // without it the first bus cycle in the boot waits for ever.
    let mut far = far_end(&n, pack);
    far.join(&mut c, clk.time_ns());
    let clk0 = cpu_clock(&n);
    // The loop steps `rtl` first, as `chip_agrees_with_rtl` does: `Rtl::spy`
    // describes the microcycle just executed, and the flags it reports are
    // the register values `chip` is holding at the boundary it is sitting on.
    for _ in 0..RTL_START_STEPS {
        r.step().unwrap();
    }

    let ids: Vec<muir::netlist::NetId> =
        r.spy().iter().map(|(name, _)| n.by_name_id(name).unwrap()).collect();
    let limit: usize =
        std::env::var("MUIR_COSIM_CYCLES").ok().and_then(|v| v.parse().ok()).unwrap_or(200);
    // How many microcycles each flag was actually seen set. A flag that never
    // rises is not being checked, whatever the comparison says.
    let mut raised = vec![0usize; ids.len()];
    let mut first: Option<String> = None;
    for cycle in 0..limit {
        r.step().unwrap_or_else(|h| panic!("rtl halted at microcycle {cycle}: {h:?}"));
        // Every value each flag takes anywhere in the microcycle, which is
        // the convention `chip_agrees_with_rtl` uses: `rtl` has one value per
        // cycle and the board moves through phases, so the question that can
        // be asked of both is whether the board ever holds it.
        // `Level::read`, not `== High`. `NOP` is an open-collector net at
        // CONTRL 3E23: driving a one means letting go, so the level a part
        // reads off it is `Z`, and comparing against `High` calls that a
        // zero.
        let mut seen: Vec<Vec<bool>> =
            ids.iter().map(|&id| vec![c.net(id).read() == Some(true)]).collect();
        let mut ran = false;
        let hang_from = muir::clock::Clock::time_ns(&clk);
        for t in 0.. {
            far.tick_with(&mut c, &mut clk);
            ran |= c.net(clk0) == Level::High;
            for (k, &id) in ids.iter().enumerate() {
                let v = c.net(id).read() == Some(true);
                if !seen[k].contains(&v) {
                    seen[k].push(v);
                }
            }
            if clk.phase_ns() == 0 && ran {
                break;
            }
            assert!(
                muir::clock::Clock::time_ns(&clk) - hang_from < HANG_BOUND_NS,
                "microcycle {cycle}: the cpu clock has not run in {HANG_BOUND_NS} ns ({t} far-end transitions) \
                 (chip PC {:o})",
                c.bus(&n, "PC", 14)
            );
        }
        if std::env::var("MUIR_SPY_TRACE").is_ok() && cycle < 10 {
            let flags: Vec<String> = r
                .spy()
                .iter()
                .enumerate()
                .map(|(k, (name, want))| {
                    format!("{name}={}/{}", *want, (c.net(ids[k]) == Level::High) as u8)
                })
                .collect();
            eprintln!(
                "cycle {cycle} chipPC {:o} rtlPC {:o} {}",
                c.bus(&n, "PC", 14),
                r.signals()[0].1,
                flags.join(" ")
            );
        }
        for (k, (name, want)) in r.spy().iter().enumerate() {
            let agrees = seen[k].contains(&(*want != 0));
            if !agrees && first.is_none() {
                first = Some(format!(
                    "microcycle {cycle}: {name} is {want} in rtl, and the board \
                     holds {:?} at every phase (chip PC {:o})",
                    seen[k],
                    c.bus(&n, "PC", 14)
                ));
            }
            if *want != 0 {
                raised[k] += 1;
            }
        }
        if first.is_some() {
            break;
        }
    }
    for ((name, _), n) in r.spy().iter().zip(&raised) {
        eprintln!("{name:10} set in {n} of {limit} microcycles");
    }
    // `JCOND` is asserted with the rest, not exempted as "ungated in `rtl`":
    // the 74S151 at FLAG 3E13 that drives it has its strobe grounded, so it
    // is not gated on the board either.
    //
    // Any divergence in the window fails. The flags agree over 540,000
    // microcycles from cold, through the branch on the disk status word at
    // `0o543`, and two microcycles in that run are where a wrong model
    // shows: 536,300 on `-VMAOK`, if the logical permission is reported
    // under the net's name, and 537,865 on that branch, if `MD` never takes
    // a word off the bus. The default window is short; `MUIR_COSIM_CYCLES`
    // reaches these.
    if let Some(why) = first {
        panic!("the flags parted company: {why}");
    }
}

/// How far into a microcycle the previous one's writes have all landed.
///
/// The write pulse ends 10 ns in; 25 ns is the next clock transition after
/// that, so it is the first phase this can be sampled at.
const WRITE_SETTLED_NS: u32 = 10;

#[test]
fn chip_and_rtl_hold_the_same_memories() {
    use muir::clock::{Behavioural, Clock};
    use muir::engine::Engine;
    use muir::part::Level;
    use muir::rtl::Rtl;

    let n = netlist::parse(NETLIST).unwrap();
    let image: Vec<u64> = muir::prom::boot_prom_image();

    let Some(pack) = machine_with_pack() else { return };
    let mut m = pack.clone();
    m.load_prom(&muir::prom::boot_prom());
    let mut r = Rtl::new(m);
    r.boot();

    let mut c = Chip::new(&n);
    c.power_on();
    c.load_prom(&n, &image);
    c.settle();
    let mut clk = Behavioural::new();
    let boot = n.by_name_id("-BOOT1").unwrap();
    // Only main memory and the devices: the control store is on the board.
    let mut far = far_end(&n, pack);
    far.join(&mut c, clk.time_ns());
    let resumed = resume_from_checkpoint(&mut c, &mut clk, &mut r, &mut far).unwrap_or(0);
    if resumed == 0 {
        c.set_net(boot, Level::Low);
        c.settle();
        for _ in 0..20 {
            c.tick(&mut clk);
        }
        c.set_net(boot, Level::High);
        let mut skipped = 0;
        while c.bus(&n, "PC", 14) == 0 && skipped < 40 {
            c.microcycle(&mut clk);
            skipped += 1;
        }
        for _ in 0..RTL_START_STEPS {
            r.step().unwrap();
        }
    }

    let rams: Vec<Ram> = MEMS.iter().map(|m| Ram::new(&c, &n, m)).collect();
    let clk0 = cpu_clock(&n);
    for (m, r) in MEMS.iter().zip(&rams) {
        eprintln!("{}: {} chip slices over {} words", m.name, r.slices.len(), r.len());
    }

    // Compared step for step, with no skew to correct. Both engines write in
    // the same microcycle because both register `DESTD` and `WADR` on
    // `CLK3D`, as the board does. An engine that records the write in the
    // cycle that computed the word is one step early, and the comparison
    // would have to be against what it held a step ago.
    let snapshot = |r: &Rtl| -> Vec<Vec<u32>> {
        vec![
            r.m.amem.to_vec(),
            r.m.mmem.to_vec(),
            r.m.pdl.to_vec(),
            r.m.spc.iter().map(|&v| v & 0o1777777).collect(),
            r.m.dmem.iter().map(|&v| v & 0o377777).collect(),
            r.m.l1_map.iter().map(|&v| v & 0o37).collect(),
            r.m.l2_map.iter().map(|&v| v & 0o77777777).collect(),
        ]
    };
    let limit: usize =
        std::env::var("MUIR_COSIM_CYCLES").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    for cycle in resumed..limit {
        // Not at the cycle boundary. The write pulse of one microcycle
        // closes ten nanoseconds into the next --- `-TPDONE` is `-TPW60` and
        // the pulse ends at `-TPW70`, which `src/clock.rs` says in as many
        // words --- so at the boundary itself the last write has not landed
        // and every comparison is one write behind.
        while clk.phase_ns() <= WRITE_SETTLED_NS {
            far.tick_with(&mut c, &mut clk);
        }
        let mut bad: Vec<String> = Vec::new();
        for ((m, ram), want) in MEMS.iter().zip(&rams).zip(&snapshot(&r)) {
            for (a, &w) in want.iter().enumerate() {
                let got = ram.word(&c, a);
                if got != w {
                    bad.push(format!("{}[{a:o}] chip {got:#x} rtl {w:#x}", m.name));
                }
            }
        }
        if !bad.is_empty() {
            eprintln!(
                "microcycle {cycle}: {} words differ, chip PC {:o} rtl PC {:o}",
                bad.len(),
                c.bus(&n, "PC", 14),
                r.signals()[0].1
            );
            for line in bad.iter().take(8) {
                eprintln!("   {line}");
            }
            panic!("microcycle {cycle}: {} words differ between chip and rtl", bad.len());
        }
        let mut ran = false;
        let hang_from = muir::clock::Clock::time_ns(&clk);
        for t in 0.. {
            far.tick_with(&mut c, &mut clk);
            ran |= c.net(clk0) == Level::High;
            if muir::clock::Clock::phase_ns(&clk) == 0 && ran {
                break;
            }
            assert!(
                muir::clock::Clock::time_ns(&clk) - hang_from < HANG_BOUND_NS,
                "microcycle {cycle}: the cpu clock has not run in {HANG_BOUND_NS} ns ({t} far-end transitions) \
                 (chip PC {:o})",
                c.bus(&n, "PC", 14)
            );
        }
        r.step().unwrap_or_else(|h| panic!("rtl halted at microcycle {cycle}: {h:?}"));
    }
    eprintln!("chip and rtl hold the same memories over {limit} microcycles");
}

// --- The diagnostic bus, from the microcode's side --------------------------

impl Ram {
    /// Puts a word into the board's memory from outside, cell by cell: the
    /// inverse of [`Ram::word`], for giving a program of one's own the
    /// constants and the map the boot PROM would otherwise have to build.
    fn store(&self, c: &mut Chip, a: usize, w: u32) {
        // Odd parity over the word and its bit; see [`Mem::parity`].
        let odd = w.count_ones().is_multiple_of(2);
        let slices = self.slices.iter().map(|s| (s, (w >> s.bit) & 1 == 1));
        let parity = self.parity.iter().map(|s| (s, odd));
        for (s, value) in slices.chain(parity) {
            if let Some((bit, held)) = s.bank
                && (a >> bit) & 1 != held as usize
            {
                continue;
            }
            let cell = s.addr.iter().fold(0usize, |k, &(logical, cell_bit, inv)| {
                let b = (a >> logical) & 1 == 1;
                k | ((b != inv) as usize) << cell_bit
            });
            let stored = value != self.active_low;
            // Through `state_mut`, which marks the part: its output gate
            // keeps a cached level while nothing it reads has moved.
            let byte = &mut c.state_mut(s.inst).cells[cell];
            let m = 1u8 << s.cell_bit;
            *byte = if stored { *byte | m } else { *byte & !m };
        }
    }
}

/// Hand-assembled microinstructions, `muir::isa::asm`'s encoding.
use muir::isa::asm as microcode;

/// A board and an `rtl` engine running the same program from the same
/// memories: the PROM as a programming image on the board's PROM parts, and
/// A memory, M memory and both map levels stored into the RAM cells from
/// the machine `rtl` gets a copy of.  Both are booted and brought to the
/// same place: `chip` measured by its PC leaving 0, `rtl` counted.
fn same_program(
    n: &netlist::Netlist,
    m: &muir::machine::Machine,
) -> (Chip, muir::clock::Behavioural, FarEnd, muir::rtl::Rtl) {
    let far = far_end(n, m.clone());
    same_program_on(n, m, far)
}

/// [`same_program`] with the far end given: the boards the test wants on
/// the backplane rather than the environment's.
fn same_program_on(
    n: &netlist::Netlist,
    m: &muir::machine::Machine,
    mut far: FarEnd,
) -> (Chip, muir::clock::Behavioural, FarEnd, muir::rtl::Rtl) {
    use muir::clock::{Behavioural, Clock};
    use muir::engine::Engine;
    use muir::part::Level;
    use muir::rtl::Rtl;

    let image: Vec<u64> = m.prom.iter().map(|&i| muir::prom::programming(i)).collect();
    let mut c = Chip::new(n);
    c.power_on();
    c.load_prom(n, &image);
    // Every word of every scratchpad, zeros included, so that each carries
    // the parity the boot PROM would have given it and no error is flagged.
    let a = Ram::new(&c, n, &MEMS[0]);
    let mm = Ram::new(&c, n, &MEMS[1]);
    let pdl = Ram::new(&c, n, &MEMS[2]);
    let spc = Ram::new(&c, n, &MEMS[3]);
    let l1 = Ram::new(&c, n, &MEMS[5]);
    let l2 = Ram::new(&c, n, &MEMS[6]);
    for (k, &v) in m.amem.iter().enumerate() {
        a.store(&mut c, k, v);
    }
    for (k, &v) in m.mmem.iter().enumerate() {
        mm.store(&mut c, k, v);
    }
    for (k, &v) in m.pdl.iter().enumerate() {
        pdl.store(&mut c, k, v);
    }
    for (k, &v) in m.spc.iter().enumerate() {
        spc.store(&mut c, k, v & 0o1777777);
    }
    for (k, &v) in m.l1_map.iter().enumerate() {
        l1.store(&mut c, k, v & 0o37);
    }
    for (k, &v) in m.l2_map.iter().enumerate() {
        l2.store(&mut c, k, v & 0o77777777);
    }
    c.settle();
    let mut clk = Behavioural::new();
    let boot = n.by_name_id("-BOOT1").unwrap();
    c.set_net(boot, Level::Low);
    c.settle();
    for _ in 0..20 {
        c.tick(&mut clk);
    }
    c.set_net(boot, Level::High);
    let mut skipped = 0;
    while c.bus(n, "PC", 14) == 0 && skipped < 40 {
        c.microcycle(&mut clk);
        skipped += 1;
    }
    far.join(&mut c, clk.time_ns());

    // On the board's own clock, not just its boundaries: the two machines'
    // memory crystals are reckoned from each clock's zero, and a mapped
    // cycle's answer is the crystal's instant.  `rtl`'s first steps from
    // the boot are at the boot's speed.
    let mut r = Rtl::new(m.clone());
    let boot_cycle = muir::clock::Speed::ExtraSlow.cycle_ns(true) as u64;
    r.set_clock(clk.time_ns() - RTL_START_STEPS as u64 * boot_cycle);
    r.boot();
    for _ in 0..RTL_START_STEPS {
        r.step().unwrap();
    }
    assert_eq!(r.ns(), clk.time_ns(), "the two clocks agree at the start");
    (c, clk, far, r)
}

/// One generator cycle of the board, and whether the cpu clock ran in it: a
/// microcycle if it did, a `WAIT` or a halt if it did not.
fn generator_cycle(
    c: &mut Chip,
    far: &mut FarEnd,
    clk: &mut muir::clock::Behavioural,
    clk0: netlist::NetId,
) -> bool {
    use muir::clock::Clock;
    use muir::part::Level;
    let from = clk.time_ns();
    let mut ran = false;
    loop {
        far.tick_with(c, clk);
        ran |= c.net(clk0) == Level::High;
        // The next boundary, not this one again: an event on a backplane
        // board that falls on the boundary is a tick in which no time
        // passes, and a cycle is time passing.  Six such ticks once put
        // the board 1320 ns behind `rtl` in what was meant to be lockstep.
        if clk.phase_ns() == 0 && clk.time_ns() > from {
            return ran;
        }
        assert!(clk.time_ns() - from < HANG_BOUND_NS, "the generator has not come round");
    }
}

/// A machine with virtual page 0 on physical page `0o37766`, which is
/// Unibus `766000`, the diagnostic block, with read and write permission:
/// a program's virtual address is then the register number. A memory 3
/// holds the word the fillers put on the A bus.
fn page_zero_on_the_diagnostic_block() -> muir::machine::Machine {
    let mut m = muir::machine::Machine::new();
    m.l1_map[0] = 0;
    m.l2_map[0] = (1 << 23) | (1 << 22) | 0o37766;
    m.amem[3] = 0o123456;
    m
}

/// [`page_zero_on_the_diagnostic_block`] with a program that writes `value`
/// into diagnostic register `register` through the processor's own Unibus
/// cycle, twenty fillers in and fillers ever after: the word in M memory 1
/// and the register's address in M memory 2, `MD` loaded from the one and
/// the write started at the other. With [`muir::spy::CLK`] and 0 it is the
/// program that halts the machine from its own microcode, `RUN` written
/// down.
fn writes_a_diagnostic_register(register: u8, value: u32) -> muir::machine::Machine {
    use microcode::*;
    use muir::isa::Insn;
    let mut m = page_zero_on_the_diagnostic_block();
    m.mmem[1] = value;
    m.mmem[2] = register as u32;
    let mut prom = vec![filler(); 512];
    prom[20] = Insn::new(ALU | SETM | m_src(1) | a_src(3) | MD);
    prom[21] = Insn::new(ALU | SETM | m_src(2) | a_src(3) | START_WRITE);
    m.load_prom(&prom);
    m
}

/// The Unibus address of diagnostic register `eadr`: a word a register
/// from [`muir::spy::BASE`].
fn unibus(eadr: u8) -> u32 {
    muir::spy::BASE + 2 * eadr as u32
}

/// **A microcode read of the diagnostic block crosses the cables both ways,
/// and the console's write registers do what `rtl` says they do.**
///
/// The cpu's own Unibus cycle into `766000`-`766036` goes out of the bus
/// interface as `-SPY READ` with `EADR<3:0>`, comes back on `SPY<15:0>`
/// through the 8304s at DIAG 0A20 and 0A21, and lands in `MD` like any
/// other word.  Five reads, each with forty identical instructions in
/// flight so that the answer does not depend on which of them the strobe
/// catches: `IR<47:32>`, the statistics counter, the open register 3, and
/// the two flag registers, whose unconnected buffer inputs read as ones.  Then
/// a write of `OPCINH` and two reads of `OPC` that must agree, and a write
/// of zero to the clock control register, after which the cpu clock never
/// runs again on either --- the microcycle in flight completing on both.
#[test]
fn chip_and_rtl_agree_on_the_diagnostic_block() {
    use microcode::*;
    use muir::engine::Engine;
    use muir::isa::Insn;
    use muir::spy;

    let n = netlist::parse(NETLIST).unwrap();
    let mut m = page_zero_on_the_diagnostic_block();
    let constants: [u32; 10] = [
        spy::IR_HIGH as u32,
        spy::STAT_LOW as u32,
        3,
        spy::FLAG_1 as u32,
        spy::FLAG_2 as u32,
        0o4,                     // OPCINH, for the OPC control register
        spy::OPC_CONTROL as u32, // its address
        spy::OPC as u32,         // the OPC read select
        0,                       // RUN down, for the clock control register
        spy::CLK as u32,         // its address
    ];
    for (k, &v) in constants.iter().enumerate() {
        m.mmem[k + 1] = v;
    }
    let mut prom = vec![filler(); 512];
    let mut at = 0;
    let park: [usize; 5] = [0o101, 0o102, 0o103, 0o104, 0o105];
    for (k, p) in park.iter().enumerate() {
        prom[at] = Insn::new(ALU | SETM | m_src(k as u64 + 1) | a_src(3) | START_READ);
        prom[at + 41] = Insn::new(ALU | SETM | SRC_MD | a_src(3) | a_dest(*p as u64));
        at += 42;
    }
    // OPC control <- 4, then OPC twice, forty apart.
    prom[at] = Insn::new(ALU | SETM | m_src(6) | a_src(3) | MD);
    prom[at + 1] = Insn::new(ALU | SETM | m_src(7) | a_src(3) | START_WRITE);
    at += 42;
    for p in [0o106, 0o107] {
        prom[at] = Insn::new(ALU | SETM | m_src(8) | a_src(3) | START_READ);
        prom[at + 41] = Insn::new(ALU | SETM | SRC_MD | a_src(3) | a_dest(p));
        at += 42;
    }
    // Clock control <- 0.
    prom[at] = Insn::new(ALU | SETM | m_src(9) | a_src(3) | MD);
    prom[at + 1] = Insn::new(ALU | SETM | m_src(10) | a_src(3) | START_WRITE);
    let halt_written_at = at + 1;
    m.load_prom(&prom);

    let (mut c, mut clk, mut far, mut r) = same_program(&n, &m);
    let clk0 = cpu_clock(&n);
    let a = Ram::new(&c, &n, &MEMS[0]);

    // Long enough for the program and the halt, and then some.
    let cycles = 700;
    let mut chip_microcycles = 0;
    let mut chip_last_ran = 0;
    for k in 0..cycles {
        if generator_cycle(&mut c, &mut far, &mut clk, clk0) {
            chip_microcycles += 1;
            chip_last_ran = k;
        }
    }
    let mut rtl_last_ran = 0;
    for k in 0..cycles {
        r.step().unwrap();
        if r.executed().is_some() {
            rtl_last_ran = k;
        }
    }

    for (k, p) in park.iter().enumerate() {
        eprintln!("read {k}: chip {:o} rtl {:o}", a.word(&c, *p), r.machine().amem[*p]);
    }
    // `IR<47:32>` of the filler is its A address, 3.  `JCOND` is off the
    // 74S151 at FLAG 3E13, which selects a bit of the M bus for an ALU
    // instruction, and the M bus is driven for only part of a microcycle;
    // a read that catches it undriven reads a one where `rtl`'s read phase
    // has the driven value.  So it is masked here, as the A, M and O buses
    // are left out altogether: on a running machine they are the phase's,
    // and CC reads them only with the machine halted.
    let want = [3u32, 0, spy::OPEN_READ as u32, 0xe900, 0xc0c0 | 0x3];
    for (k, (p, w)) in park.iter().zip(want).enumerate() {
        let chip = a.word(&c, *p);
        let rtl = r.machine().amem[*p];
        let mask = if k == 4 { !0x4 } else { !0 };
        assert_eq!(chip & mask, w, "read {k} on the board");
        assert_eq!(rtl & mask, w, "read {k} on rtl");
    }
    let (o1, o2) = (a.word(&c, 0o106), a.word(&c, 0o107));
    eprintln!(
        "OPC under OPCINH: chip {o1:o} then {o2:o}, rtl {:o} then {:o}",
        r.machine().amem[0o106],
        r.machine().amem[0o107]
    );
    assert_eq!(o1, o2, "the board's OPC history is frozen");
    assert_eq!(
        (o1, o2),
        (r.machine().amem[0o106], r.machine().amem[0o107]),
        "and rtl's is the same PC"
    );
    assert!(o1 < 0o400, "a PC in the PROM");

    eprintln!(
        "halted: chip after {chip_microcycles} microcycles at PC {:o} (last ran in generator cycle {chip_last_ran}), \
         rtl after {} at PC {:o} (last ran in step {rtl_last_ran})",
        c.bus(&n, "PC", 14),
        r.machine().cycles,
        r.pc()
    );
    assert!(chip_last_ran < cycles - 50, "the board is halted");
    assert!(rtl_last_ran < cycles - 50, "rtl is halted");
    assert_eq!(r.spy_read(spy::FLAG_1) & 0x100, 0, "SRUN down");
    assert_eq!(c.bus(&n, "PC", 14) as u16, r.pc(), "halted at the same PC");
    assert!(r.pc() as usize > halt_written_at, "past the write that halted it");
    // `rtl` counts the trap's microcycle and the nopped one behind it; the
    // board's count starts where its PC leaves 0, after both.
    assert_eq!(
        chip_microcycles as u64 + RTL_START_STEPS as u64,
        r.machine().cycles,
        "the same number of microcycles ran"
    );
}

/// **`PROG.BOOT`, bit 7 of a mode-register write, reboots the machine.**  The
/// program writes `200` into `766012` every time round, so both engines trap
/// to 0 again and again, in the same microcycle and with the same period.
/// The trap is taken inside the microcycle the write pulse falls in: the
/// pulse is up from the 50 ns tap, the 74LS109's clear is asynchronous, and
/// the cpu edge at the end of that microcycle finds `TRAP` up --- 30 ns
/// before the register itself would have loaded at the pulse's trailing
/// edge, as `MUIR_SPY_TRACE=1` shows wire by wire.  Taking it from the
/// register's own load instead is two microcycles later.
#[test]
fn chip_and_rtl_reboot_on_the_mode_registers_boot_bit() {
    use muir::engine::Engine;
    use muir::spy;

    let n = netlist::parse(NETLIST).unwrap();
    let m = writes_a_diagnostic_register(spy::MODE, spy::MODE_BOOT as u32);

    let (mut c, mut clk, mut far, mut r) = same_program(&n, &m);
    let clk0 = cpu_clock(&n);
    let mut chip_pcs = Vec::new();
    let mut rtl_pcs = Vec::new();
    // `MUIR_SPY_TRACE`: when the strobe and the boot happen on each side.
    let trace = std::env::var("MUIR_SPY_TRACE").is_ok();
    let dbwrite = n.by_name_id("-DBWRITE").unwrap();
    let nboot = n.by_name_id("-BOOT").unwrap();
    let memrq = n.by_name_id("MEMRQ").unwrap();
    let mut was = (true, true, false);
    let mut rtl_answered: Option<u64> = None;
    let mut rtl_requests = 0;
    // The interface board's side of the same cycle, by name.
    let bus_n = netlist::parse(BUSINT).unwrap();
    let bus_watch: Vec<(&str, netlist::NetId)> =
        ["-MEMRQ", "LM NEED UB", "LMUB MASTER", "LMUB GRANT", "-UB MSYN", "-SPY WRITE", "-MCLK7"]
            .iter()
            .map(|&w| (w, net_named(&bus_n, w)))
            .collect();
    let mut bus_was: Vec<muir::part::Level> =
        bus_watch.iter().map(|&(_, id)| far.board.net(id)).collect();
    let t0 = muir::clock::Clock::time_ns(&clk);
    let r0 = r.ns();
    for k in 0..120 {
        loop {
            let ran = if trace {
                // Tick by tick, so that a 100 ns strobe is seen.
                use muir::clock::Clock;
                use muir::part::Level;
                let mut ran = false;
                loop {
                    far.tick_with(&mut c, &mut clk);
                    ran |= c.net(clk0) == Level::High;
                    let now = (
                        c.net(dbwrite) != Level::Low,
                        c.net(nboot) != Level::Low,
                        c.net(memrq) == Level::High,
                    );
                    let t = clk.time_ns() - t0;
                    if k < 40 {
                        for (j, &(name, id)) in bus_watch.iter().enumerate() {
                            let l = far.board.net(id);
                            if l != bus_was[j] && (name != "-MCLK7" || (18..32).contains(&k)) {
                                eprintln!(
                                    "   busint {t} ns: {name}={l:?} (microcycle {k}, phase {})",
                                    clk.phase_ns()
                                );
                            }
                            bus_was[j] = l;
                        }
                    }
                    if now.2 && !was.2 {
                        eprintln!(
                            "chip {t} ns: MEMRQ up (microcycle {k}, phase {})",
                            clk.phase_ns()
                        );
                    }
                    if !now.0 && was.0 {
                        eprintln!("chip {t} ns: -DBWRITE low (phase {})", clk.phase_ns());
                    }
                    if now.0 && !was.0 {
                        eprintln!(
                            "chip {t} ns: -DBWRITE high, the strobe's trailing edge (phase {})",
                            clk.phase_ns()
                        );
                    }
                    if !now.1 && was.1 {
                        eprintln!("chip {t} ns: -BOOT low (phase {})", clk.phase_ns());
                    }
                    if now.1 && !was.1 {
                        eprintln!("chip {t} ns: -BOOT high (phase {})", clk.phase_ns());
                    }
                    was = now;
                    if clk.phase_ns() == 0 {
                        break;
                    }
                }
                ran
            } else {
                generator_cycle(&mut c, &mut far, &mut clk, clk0)
            };
            if ran {
                break;
            }
        }
        chip_pcs.push(c.bus(&n, "PC", 14) as u16);
        r.step().unwrap();
        rtl_pcs.push(r.pc());
        if trace {
            if r.bus_cycles() != rtl_requests {
                rtl_requests = r.bus_cycles();
                eprintln!("rtl step {k}: request at the edge {} ns", r.ns() - r0);
            }
            let a = r.busint().answered_at();
            if a != rtl_answered {
                if let Some(a) = a {
                    eprintln!(
                        "rtl step {k}: cycle granted, register strobe at {} ns (now {}), holds the Unibus: {}",
                        a - r0,
                        r.ns() - r0,
                        r.busint().holds_the_unibus()
                    );
                }
                rtl_answered = a;
            }
            if r.pc() == 0 {
                eprintln!("rtl step {k}: PC 0 at {} ns", r.ns() - r0);
            }
            if chip_pcs[k] == 0 {
                eprintln!(
                    "chip microcycle {k}: PC 0 at {} ns",
                    muir::clock::Clock::time_ns(&clk) - t0
                );
            }
        }
    }
    let traps = |pcs: &[u16]| -> Vec<usize> {
        pcs.windows(2).enumerate().filter(|(_, w)| w[1] < w[0]).map(|(k, _)| k + 1).collect()
    };
    let (ct, rt) = (traps(&chip_pcs), traps(&rtl_pcs));
    eprintln!("chip traps at microcycles {ct:?}, rtl at {rt:?}");
    eprintln!("chip PCs {:?}", &chip_pcs[..40]);
    eprintln!("rtl  PCs {:?}", &rtl_pcs[..40]);
    assert!(ct.len() >= 2 && rt.len() >= 2, "both reboot at least twice");
    assert_eq!(ct, rt, "the same traps in the same microcycles");
    assert_eq!(chip_pcs[ct[0]], 0, "the board traps to 0");
    assert_eq!(chip_pcs, rtl_pcs, "and the two PC sequences are one");
}

// --- The debug cable on DBGIN -----------------------------------------------

/// A harness debugger on the interface board's DBGIN connector: the 21
/// wires of the debug cable as `data/busint-connectors.txt` names them, and
/// `DBUB MASTER` to know when the board has let the Unibus go.
struct DebuggerOnCable {
    req: netlist::NetId,
    a0: netlist::NetId,
    a1: netlist::NetId,
    wr: netlist::NetId,
    ack: netlist::NetId,
    dbd: Vec<netlist::NetId>,
    master: netlist::NetId,
    /// Wires watched for the timeline a request prints.
    watch: Vec<(&'static str, netlist::NetId)>,
    /// Nets of the cpu board watched the same way, with the clock's phase
    /// at each change; empty unless a test asks.
    cpu_watch: Vec<(&'static str, netlist::NetId)>,
    /// `UAO<17:1>`, the address the board puts out as master, and
    /// `UBA<17:0>`, the address it receives, for the timeline.
    uao: Vec<netlist::NetId>,
    uba: Vec<netlist::NetId>,
}

impl DebuggerOnCable {
    fn new(bus_n: &netlist::Netlist) -> DebuggerOnCable {
        let net = |name: &str| net_named(bus_n, name);
        let watch = [
            "-DEBUG IN REQ",
            "-DB ADR0 CLK",
            "-DB ADR1 CLK",
            "-DB READ STATUS",
            "-DBD ENB",
            "-DEBUG > UD",
            "-DB NEED UB",
            "-UB NPR",
            "NPG1 IN",
            "DB UB GRANTED",
            "DB UB SELECTED",
            "-UB SACK",
            "LMUB MASTER",
            "DBUB MASTER",
            "-UB BBSY",
            "-UB MSYN",
            "MSYN IN",
            "-SELECT SPY",
            "UB REG CYC T0",
            "UB17-14=MAP",
            "-UB READ XBUS",
            "-UB WR XBUS",
            "-UB WRITE BUFFER",
            "-UB READ BUFFER",
            "MAPVALID",
            "WRITEOK",
            "-UB INVALID",
            "UB XBUS T0",
            "UBXRQ",
            "UBXRQS",
            "UBX GRANT",
            "-XBUS RQ",
            "-XBUS ACK",
            "XACK",
            "-UBACK",
            "NXM TIMEOUT",
            "UB MAP ERROR",
            "SSYN OUT",
            "-UB SSYN",
            "-SPY READ",
            "-SPY WRITE",
            "DEBUG IN ACK",
        ]
        .iter()
        .map(|&w| (w, net(w)))
        .collect();
        let uao = (1..18).map(|k| net(&format!("UAO{k}"))).collect();
        let uba = (0..18).map(|k| net(&format!("UBA{k}"))).collect();
        DebuggerOnCable {
            req: net("-DEBUG IN REQ"),
            a0: net("DEBUG IN A0"),
            a1: net("DEBUG IN A1"),
            wr: net("DEBUG IN WR"),
            ack: net("DEBUG IN ACK"),
            dbd: (0..16).map(|k| net(&format!("DBD{k}"))).collect(),
            master: net("DBUB MASTER"),
            watch,
            uao,
            uba,
            cpu_watch: Vec::new(),
        }
    }

    /// `DBD<15:0>` as the board's own drivers hold them: the word and which
    /// bits are driven at all --- the status strobe drives eight.
    fn dbd(&self, board: &Chip) -> (u16, u16) {
        let mut word = 0;
        let mut driven = 0;
        for (k, &net) in self.dbd.iter().enumerate() {
            let (level, strong) = board.board_level(net);
            if strong {
                driven |= 1 << k;
                if level == muir::part::Level::High {
                    word |= 1 << k;
                }
            }
        }
        (word, driven)
    }

    /// One request on the board's cable from the present instant: the wires
    /// driven, the boards run to `DEBUG IN ACK`, the request held `hold_ns`
    /// more and lifted, and the boards run on to the next generator-cycle
    /// boundary.  Returns when the request went on, when it was
    /// acknowledged, the word and driven mask on `DBD` at the
    /// acknowledgement, and how many boundaries passed --- for `rtl` to be
    /// stepped the same number of times.
    #[allow(clippy::too_many_arguments)]
    fn request(
        &self,
        c: &mut Chip,
        far: &mut FarEnd,
        clk: &mut muir::clock::Behavioural,
        strobe: u8,
        write: bool,
        dbd: u16,
        hold_ns: u64,
        trace: bool,
    ) -> (u64, u64, (u16, u16), usize) {
        use muir::clock::Clock;
        use muir::part::Level;
        let at = clk.time_ns();
        // At a boundary --- or anywhere, while the debuggee's clock ring is
        // held by the modifier's reset bit and has no boundaries.
        let held = clk.next_at(c.clock_inputs()).is_none();
        assert!(clk.phase_ns() == 0 || held, "a request goes on at a boundary");
        far.board.drive(self.a0, Level::from(strobe & 1 != 0));
        far.board.drive(self.a1, Level::from(strobe & 2 != 0));
        far.board.drive(self.wr, Level::from(write));
        for (k, &net) in self.dbd.iter().enumerate() {
            if write {
                far.board.drive(net, Level::from((dbd >> k) & 1 != 0));
            } else {
                far.board.pull_up(net);
            }
        }
        far.board.drive(self.req, Level::Low);
        far.board.transition(at);
        far.join(c, at);
        let mut last: Vec<Level> = self.watch.iter().map(|&(_, id)| far.board.net(id)).collect();
        let mut dbd_was = far.board.read(&self.dbd);
        // A boundary is counted once, when time has passed to it.
        let mut last_boundary = at;
        if trace {
            eprintln!(
                "   busint     0 ns: -DEBUG IN REQ down, A1 A0 {}{}, WR {}, DBD {dbd_was:o}",
                (strobe >> 1) & 1,
                strobe & 1,
                write as u8
            );
        }
        let mut cpu_last: Vec<Level> = self.cpu_watch.iter().map(|&(_, id)| c.net(id)).collect();
        let mut boundaries = 0;
        let mut acked = None;
        let mut word = (0, 0);
        // A tick that moves no clock is a spin: a board event that stays
        // due.  Bounded, with what each board says is next.
        let (mut spins, mut spin_at) = (0u32, u64::MAX);
        loop {
            if clk.time_ns() == spin_at {
                spins += 1;
                assert!(
                    spins < 20_000,
                    "no time passes at {} ns: clock {:?}, cpu tap {:?}, interface tap {:?}, xbus tap {:?}, buses {:?}",
                    spin_at,
                    clk.next_at(c.clock_inputs()),
                    c.next_tap(),
                    far.board.next_tap(),
                    far.xbus.next_tap(),
                    far.buses.next_event()
                );
            } else {
                spin_at = clk.time_ns();
                spins = 0;
            }
            if acked.is_none() && far.board.net(self.ack) == Level::High {
                acked = Some(clk.time_ns());
                word = self.dbd(&far.board);
            }
            if trace {
                let t = clk.time_ns() - at;
                for (k, &(name, id)) in self.watch.iter().enumerate() {
                    let l = far.board.net(id);
                    if l != last[k] {
                        eprintln!("   busint {t:>5} ns: {name}={l:?}");
                        if name == "-UB MSYN" && l == Level::Low {
                            eprintln!(
                                "   busint {t:>5} ns: UAO<17:1> {:o} out, UBA<17:0> {:o} in",
                                far.board.read(&self.uao) << 1,
                                far.board.read(&self.uba)
                            );
                        }
                        last[k] = l;
                    }
                }
                let dbd = far.board.read(&self.dbd);
                if dbd != dbd_was {
                    eprintln!("   busint {t:>5} ns: DBD {dbd:o}");
                    dbd_was = dbd;
                }
                for (k, &(name, id)) in self.cpu_watch.iter().enumerate() {
                    let l = c.net(id);
                    if l != cpu_last[k] {
                        eprintln!(
                            "   cpu    {t:>5} ns: {name}={l:?} (phase {}, clock next {:?})",
                            clk.phase_ns(),
                            clk.next_at(c.clock_inputs()).map(|x| x - at)
                        );
                        cpu_last[k] = l;
                    }
                }
            }
            if let Some(ack) = acked {
                // To the lift, an event at a time, so that a boundary on the
                // way is seen and counted --- `tick_to` would pass it.
                let lift = ack + hold_ns;
                if clk.time_ns() >= lift {
                    break;
                }
                match far.next_event(c, clk) {
                    Some(e) if e <= lift => {
                        far.tick_with(c, clk);
                    }
                    _ => {
                        clk.pass(lift, clk.next_at(c.clock_inputs()).is_none());
                        c.transition(lift);
                        far.board.transition(lift);
                        far.join(c, lift);
                    }
                }
            } else {
                assert!(clk.time_ns() < at + 40_000, "strobe {strobe} was never acknowledged");
                far.tick_with(c, clk);
            }
            if clk.phase_ns() == 0 && clk.time_ns() > last_boundary {
                boundaries += 1;
                last_boundary = clk.time_ns();
            }
        }
        let ack = acked.unwrap();
        far.board.drive(self.req, Level::High);
        far.board.transition(clk.time_ns());
        for &net in &self.dbd {
            far.board.pull_up(net);
        }
        far.board.transition(clk.time_ns());
        far.join(c, clk.time_ns());
        if trace {
            eprintln!("   busint {:>5} ns: -DEBUG IN REQ lifted", clk.time_ns() - at);
        }
        // On to the boundary, watching the release --- unless the clock is
        // held, when there is none to wait for.
        loop {
            if clk.next_at(c.clock_inputs()).is_none() {
                break;
            }
            far.tick_with(c, clk);
            if trace {
                for (k, &(name, id)) in self.watch.iter().enumerate() {
                    let l = far.board.net(id);
                    if l != last[k] {
                        eprintln!("   busint {:>5} ns: {name}={l:?}", clk.time_ns() - at);
                        last[k] = l;
                    }
                }
                for (k, &(name, id)) in self.cpu_watch.iter().enumerate() {
                    let l = c.net(id);
                    if l != cpu_last[k] {
                        eprintln!(
                            "   cpu    {:>5} ns: {name}={l:?} (phase {})",
                            clk.time_ns() - at,
                            clk.phase_ns()
                        );
                        cpu_last[k] = l;
                    }
                }
            }
            if clk.phase_ns() == 0 && clk.time_ns() > last_boundary {
                boundaries += 1;
                break;
            }
        }
        (at, ack, word, boundaries)
    }
}

/// How `rtl` is kept level with the board around a request on the cable.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Debuggee {
    /// Halted: one `rtl` step to a generator cycle; and where the clock
    /// ring is held by the modifier's reset bit and there is no boundary to
    /// step to, `rtl` brought to the board's instant by time alone.
    Halted,
    /// Running, with bus cycles: several generator cycles to one `rtl` step
    /// --- the waits --- so the two are brought to one instant by time,
    /// whichever is behind moving ([`meet`]).
    Running,
}

/// Brings `rtl` and the board to one instant: whichever is behind moves,
/// `rtl` a step and the board a generator cycle, until the two clocks read
/// the same.
fn meet(
    n: &netlist::Netlist,
    c: &mut Chip,
    far: &mut FarEnd,
    clk: &mut muir::clock::Behavioural,
    r: &mut muir::rtl::Rtl,
    clk0: netlist::NetId,
) {
    use muir::clock::Clock;
    use muir::engine::Engine;
    for _ in 0..2_000 {
        if r.ns() < clk.time_ns() {
            r.step().unwrap();
        } else if r.ns() > clk.time_ns() {
            generator_cycle(c, far, clk, clk0);
        } else {
            return;
        }
    }
    panic!(
        "rtl at {} ns (PC {:o}) and the board at {} ns (PC {:o}) never meet",
        r.ns(),
        r.pc(),
        clk.time_ns(),
        c.bus(n, "PC", 14)
    );
}

/// A harness debugger's requests made on the board and on `rtl` in
/// lockstep, and the two compared: the harness on the board's DBGIN, the
/// cpu clock net, how `rtl` is kept level, and how many debug cycles are
/// still to have their wire timeline printed.
struct Lockstep<'a> {
    n: &'a netlist::Netlist,
    cable: &'a DebuggerOnCable,
    clk0: netlist::NetId,
    debuggee: Debuggee,
    trace: std::cell::Cell<u32>,
}

impl<'a> Lockstep<'a> {
    /// The debugger lifts a request this long after the acknowledgement, as
    /// machine A's own `-UB MSYN` would.
    const HOLD_NS: u64 = muir::busint::UNIBUS_STROBE_NS;

    fn new(
        n: &'a netlist::Netlist,
        cable: &'a DebuggerOnCable,
        clk0: netlist::NetId,
        debuggee: Debuggee,
        trace: u32,
    ) -> Lockstep<'a> {
        Lockstep { n, cable, clk0, debuggee, trace: std::cell::Cell::new(trace) }
    }

    /// To a boundary where neither side is busy: `DBUB MASTER` down on the
    /// board, `rtl`'s cable quiet.
    fn idle(
        &self,
        c: &mut Chip,
        far: &mut FarEnd,
        clk: &mut muir::clock::Behavioural,
        r: &mut muir::rtl::Rtl,
    ) {
        use muir::engine::Engine;
        use muir::part::Level;
        for _ in 0..100 {
            if far.board.net(self.cable.master) == Level::Low && !r.debug_busy() {
                break;
            }
            generator_cycle(c, far, clk, self.clk0);
            match self.debuggee {
                Debuggee::Halted => r.step().unwrap(),
                Debuggee::Running => meet(self.n, c, far, clk, r, self.clk0),
            }
        }
        assert_eq!(
            far.board.net(self.cable.master),
            Level::Low,
            "DBUB MASTER still up on the board"
        );
        assert!(!r.debug_busy(), "rtl's cable still busy");
    }

    /// One request on both machines, in lockstep, and the two compared:
    /// the delay from the request to the acknowledgement, and the word on
    /// `DBD` under `mask` where `rtl` gives one --- the board must then
    /// drive something, and for a debug cycle all sixteen bits --- and
    /// nothing driven where `rtl` gives none. Returns the delay and `rtl`'s
    /// word.
    #[allow(clippy::too_many_arguments)]
    fn request(
        &self,
        c: &mut Chip,
        far: &mut FarEnd,
        clk: &mut muir::clock::Behavioural,
        r: &mut muir::rtl::Rtl,
        strobe: u8,
        write: bool,
        dbd: u16,
        mask: u16,
    ) -> (u64, Option<u16>) {
        use muir::busint::{DEBUG_CYCLE, DebugRequest};
        use muir::clock::Clock;
        use muir::engine::Engine;
        self.idle(c, far, clk, r);
        let trace = strobe == DEBUG_CYCLE && self.trace.get() > 0;
        if trace {
            self.trace.set(self.trace.get() - 1);
            eprintln!(
                "a debug {} cycle on the board, from {} ns:",
                if write { "write" } else { "read" },
                clk.time_ns()
            );
        }
        let (cat, cack, (cword, cdriven), boundaries) =
            self.cable.request(c, far, clk, strobe, write, dbd, Self::HOLD_NS, trace);
        let rat = r.ns();
        r.debug_request(rat, DebugRequest { strobe, write, dbd, hold_ns: Self::HOLD_NS });
        match self.debuggee {
            Debuggee::Halted => {
                for _ in 0..boundaries {
                    r.step().unwrap();
                }
                // With the clock held by the reset bit there was no boundary
                // to step to: bring rtl to the board's instant the same way,
                // time passing and nothing else.
                if r.ns() < clk.time_ns() {
                    r.step_until(clk.time_ns()).unwrap();
                }
                assert_eq!(r.ns(), clk.time_ns(), "in step after strobe {strobe}");
            }
            Debuggee::Running => meet(self.n, c, far, clk, r, self.clk0),
        }
        let (rack, rword) = r.debug_ack().unwrap_or_else(|| {
            panic!(
                "rtl did not acknowledge strobe {strobe} by {} ns ({boundaries} boundaries)",
                r.ns()
            )
        });
        if strobe == DEBUG_CYCLE {
            eprintln!(
                "{} at {cat} ns: board acknowledged after {} ns with {:o} (driven {:o}), rtl after {} ns with {:?}",
                if write { format!("write {dbd:o}") } else { "read".to_string() },
                cack - cat,
                cword,
                cdriven,
                rack - rat,
                rword.map(|w| format!("{w:o}"))
            );
        }
        assert_eq!(rack - rat, cack - cat, "the acknowledgement's delay, strobe {strobe}");
        match rword {
            Some(w) => {
                assert_ne!(cdriven, 0, "the board drives nothing where rtl has {w:o}");
                assert_eq!(
                    cword & cdriven & mask,
                    w & cdriven & mask,
                    "the word on DBD, strobe {strobe}"
                );
                if strobe == DEBUG_CYCLE {
                    assert_eq!(cdriven, 0xffff, "all sixteen bits driven for a read");
                }
            }
            None => assert_eq!(cdriven, 0, "the board drives DBD where rtl drives nothing"),
        }
        (cack - cat, rword)
    }

    /// CC's `DBG-READ` of Unibus address `uaddr`: the modifier with
    /// address bit 17, the address, and the read cycle. Returns the word.
    fn dbg_read(
        &self,
        c: &mut Chip,
        far: &mut FarEnd,
        clk: &mut muir::clock::Behavioural,
        r: &mut muir::rtl::Rtl,
        uaddr: u32,
    ) -> u16 {
        use muir::busint::{DEBUG_ADDRESS, DEBUG_CYCLE, DEBUG_MODIFIER};
        self.request(c, far, clk, r, DEBUG_MODIFIER, true, (uaddr >> 17) as u16 & 1, !0);
        self.request(c, far, clk, r, DEBUG_ADDRESS, true, (uaddr >> 1) as u16, !0);
        self.request(c, far, clk, r, DEBUG_CYCLE, false, 0, !0).1.expect("nothing drove DBD")
    }

    /// CC's `DBG-WRITE` of `val` to Unibus address `uaddr`.
    fn dbg_write(
        &self,
        c: &mut Chip,
        far: &mut FarEnd,
        clk: &mut muir::clock::Behavioural,
        r: &mut muir::rtl::Rtl,
        uaddr: u32,
        val: u16,
    ) {
        use muir::busint::{DEBUG_ADDRESS, DEBUG_CYCLE, DEBUG_MODIFIER};
        self.request(c, far, clk, r, DEBUG_MODIFIER, true, (uaddr >> 17) as u16 & 1, !0);
        self.request(c, far, clk, r, DEBUG_ADDRESS, true, (uaddr >> 1) as u16, !0);
        self.request(c, far, clk, r, DEBUG_CYCLE, true, val, !0);
    }

    /// [`Lockstep::dbg_write`] into diagnostic register `eadr`.
    fn spy_write(
        &self,
        c: &mut Chip,
        far: &mut FarEnd,
        clk: &mut muir::clock::Behavioural,
        r: &mut muir::rtl::Rtl,
        eadr: u8,
        val: u16,
    ) {
        self.dbg_write(c, far, clk, r, unibus(eadr), val);
    }
}

/// **The debug cable on the board: a harness debugger on DBGIN halts the
/// machine, reads its PC, steps it, loads and runs an instruction through
/// the debug IR, and reads the halted buses --- and `rtl` answers every
/// request at the same instant with the same word.**
///
/// The second master of the Unibus: `-DEBUG IN REQ` with the two address
/// bits into the 74S139 at DBGIN 0A15, `-DB NEED UB` through the
/// arbitration on UBMAST beside the processor's own --- taking the bus
/// from it, since its `SACK` clears `LMUB MASTER` --- `DBUB MASTER`, the
/// latched address out, `-UB MSYN`, the register cycle on UBCYC, and
/// `DEBUG ACK` back with `-UB SSYN`.  A read of the diagnostic block from
/// this master is the first read of it by anything but the cpu: `-DBREAD`
/// over the cables, the cpu's word on `SPY<15:0>`, and back through the
/// 8304s at DIAG and DBGOUT onto `DBD`.  With the processor halted, every
/// word compared is stable --- which is how CC reads them.  The wire
/// timeline of the first cycle is printed.
#[test]
fn chip_and_rtl_answer_the_debug_cable_alike() {
    use microcode::*;
    use muir::busint::DEBUG_STATUS;
    use muir::engine::Engine;
    use muir::part::Level;
    use muir::spy;

    let n = netlist::parse(NETLIST).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    // RUN down through the clock control register, from the microcode; A
    // memory 5 and M memory 7 for the instruction run through the debug IR.
    let mut m = writes_a_diagnostic_register(spy::CLK, 0);
    m.amem[5] = 0o12345670;
    m.mmem[7] = 0o777;

    let (mut c, mut clk, mut far, mut r) = same_program(&n, &m);
    let clk0 = cpu_clock(&n);
    let a = Ram::new(&c, &n, &MEMS[0]);
    let cable = DebuggerOnCable::new(&bus_n);

    // Both halt.
    let mut chip_ran_last = 0;
    for k in 0..60 {
        if generator_cycle(&mut c, &mut far, &mut clk, clk0) {
            chip_ran_last = k;
        }
        r.step().unwrap();
    }
    assert!(chip_ran_last < 40, "the board halted");
    assert_eq!(r.spy_read(spy::FLAG_1) & 0x100, 0, "rtl halted");
    let halted_at = c.bus(&n, "PC", 14) as u16;
    assert_eq!(halted_at, r.pc(), "halted at the same PC");
    assert!(far.board.net(cable.watch[6].1) == Level::High, "the processor holds the Unibus");
    assert!(r.busint().holds_the_unibus());
    eprintln!("both halted at PC {halted_at:o}");

    // The wire timeline of the first debug cycle is printed.
    let lock = Lockstep::new(&n, &cable, clk0, Debuggee::Halted, 1);
    let pc = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::PC));
    assert_eq!(pc, halted_at, "the PC read through the cable is the PC on the board");

    // CC-CLOCK: one step.
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, unibus(spy::CLK), 2);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, unibus(spy::CLK), 0);
    let pc = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::PC));
    assert_eq!(pc, halted_at + 1, "one step through the cable, on the board");
    assert_eq!(c.bus(&n, "PC", 14) as u16, halted_at + 1);

    // The halted buses, and the status.
    let a_low = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::A_LOW));
    assert_eq!(a_low, 0o123456, "A-LOW: the filler's A memory 3");
    let m_low = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::M_LOW));
    assert_eq!(m_low, 0, "M-LOW: the filler's M memory 0");
    let ob_low = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::OB_LOW));
    assert_eq!(ob_low, 0o123456, "OB-LOW: SETA");
    // The status.  The board's `UB MAP ERROR`, the 74LS74 at REQERR 0D03,
    // shows the error from power-on until `-RESET ERR` presets it --- a
    // write of `766044`, which nothing in this program has made --- so
    // that bit is left out of the first comparison; then CC's
    // `DBG-RESET-STATUS`, the write through the cable, and both agree on
    // every bit.  `-FREE`, bit 6, is down on both: no cycle of the
    // processor's is up.
    let map_error = 0o40;
    let status =
        lock.request(&mut c, &mut far, &mut clk, &mut r, DEBUG_STATUS, false, 0, !map_error).1;
    eprintln!("the status through the cable before DBG-RESET-STATUS: rtl {status:?}");
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o766044, 0);
    let status = lock.request(&mut c, &mut far, &mut clk, &mut r, DEBUG_STATUS, false, 0, !0).1;
    assert_eq!(
        status,
        Some(0xff00),
        "no error, the bus free, no write-through; the high byte open"
    );

    // CC-EXECUTE: `((A-MEM 101) SETA A-MEM-5)` through the debug IR ---
    // loaded under NOP11 and IDEBUG with one step, its result on the OB
    // before it runs, then run with two more.
    let insn = ALU | SETA | a_src(5) | m_src(7) | a_dest(0o101);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, unibus(spy::IR_LOW), insn as u16);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, unibus(spy::IR_MED), (insn >> 16) as u16);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, unibus(spy::IR_HIGH), (insn >> 32) as u16);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, unibus(spy::CLK), 0o16);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, unibus(spy::CLK), 0);
    let ir_high = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::IR_HIGH));
    assert_eq!(ir_high, (insn >> 32) as u16, "IR holds the debug IR on the board");
    let a_high = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::A_HIGH));
    let a_low = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::A_LOW));
    assert_eq!((a_high as u32) << 16 | a_low as u32, 0o12345670, "A memory 5 on the A bus");
    let m_low = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::M_LOW));
    assert_eq!(m_low, 0o777, "M memory 7 on the M bus");
    let ob_low = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::OB_LOW));
    assert_eq!(ob_low, (0o12345670u32 & 0xffff) as u16, "SETA on the OB, before it runs");
    assert_eq!(a.word(&c, 0o101), 0, "not executed yet on the board");
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, unibus(spy::CLK), 0o12);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, unibus(spy::CLK), 0);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, unibus(spy::CLK), 0o16);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, unibus(spy::CLK), 0);
    assert_eq!(a.word(&c, 0o101), 0o12345670, "CC-EXECUTE-W landed on the board");
    assert_eq!(r.machine().amem[0o101], 0o12345670, "and on rtl");
}

/// **`CC-SAVE-OPCS`, `CC-WRITE-STAT-COUNTER` and `CC-RESET-MACH` through
/// the cable, on the board and on `rtl` in lockstep.**  The three console
/// controls the board had not been asked for: the OPC history read out
/// under `OPCCLK`, eight reads each followed by `2` then `0` in the OPC
/// control register; the statistics counter loaded under `LDSTAT` by CC's
/// own route --- `CC-WRITE-MD` puts the word in `MD`, an `M-SRC MD`
/// instruction through the debug IR puts it on the M bus, a no-op clock
/// puts it in `IWR`, and a step with `LDSTAT` loads the counter from there
/// --- and `PROG.RESET`, the mode register written with bit 6 after
/// `DBG-RESET`'s pulse on the modifier's reset bit.  Every request's
/// acknowledgement and every word read must agree to the nanosecond.
#[test]
fn chip_and_rtl_run_the_opc_readout_the_counter_load_and_the_reset_alike() {
    use microcode::*;
    use muir::busint::{DEBUG_MODIFIER, DEBUG_STATUS, debug_modifier};
    use muir::engine::Engine;
    use muir::spy;

    let n = netlist::parse(NETLIST).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    let m = writes_a_diagnostic_register(spy::CLK, 0);
    let (mut c, mut clk, mut far, mut r) = same_program(&n, &m);
    let clk0 = cpu_clock(&n);
    let cable = DebuggerOnCable::new(&bus_n);
    for _ in 0..60 {
        generator_cycle(&mut c, &mut far, &mut clk, clk0);
        r.step().unwrap();
    }
    assert_eq!(r.spy_read(spy::FLAG_1) & 0x100, 0, "rtl halted");
    let halted_at = c.bus(&n, "PC", 14) as u16;
    assert_eq!(halted_at, r.pc(), "halted at the same PC");
    let lock = Lockstep::new(&n, &cable, clk0, Debuggee::Halted, 0);

    // CC-SAVE-OPCS: the eight PCs come out oldest first, and the shifts
    // fill the history with the halted PC.
    let history = r.opc_history();
    assert_ne!(history[0], history[7], "a history worth reading");
    let mut saved = [0u16; 8];
    for s in &mut saved {
        *s = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::OPC));
        lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::OPC_CONTROL, 2);
        lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::OPC_CONTROL, 0);
    }
    for (k, s) in saved.iter().enumerate() {
        assert_eq!(*s, history[7 - k], "read {k}: oldest first, on both");
    }
    assert_eq!(r.opc_history(), [halted_at; 8], "eight shifts fill rtl's history with PC");
    let last = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::OPC));
    assert_eq!(last, halted_at, "and the board's");
    let octal: Vec<String> = saved.iter().map(|s| format!("{s:o}")).collect();
    eprintln!("CC-SAVE-OPCS through the cable: {octal:?} on both");

    // CC-WRITE-STAT-COUNTER, CC's way.
    let value: u32 = 0o3_1415_2653;
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o766174, 0o177000);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o174000, value as u16);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o174002, (value >> 16) as u16);
    assert_eq!(r.machine().md, value, "CC-WRITE-MD");
    let insn = ALU | SETM | SRC_MD | a_src(3);
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::IR_LOW, insn as u16);
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::IR_MED, (insn >> 16) as u16);
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::IR_HIGH, (insn >> 32) as u16);
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::CLK, 0o16); // CC-NOOP-DEBUG-CLOCK
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::CLK, 0);
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::CLK, 0o6); // CC-NOOP-CLOCK: IWR gets M
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::CLK, 0);
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::CLK, 0o26); // STEP, NOP11, LDSTAT
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::CLK, 0);
    let lo = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::STAT_LOW));
    let hi = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::STAT_HIGH));
    assert_eq!((hi as u32) << 16 | lo as u32, value, "the statistics counter, read on both");
    assert_eq!(r.stat(), value);
    eprintln!("CC-WRITE-STAT-COUNTER through the cable: {value:o} on both");

    // CC-RESET-MACH: DBG-RESET's pulse on the modifier's reset bit ---
    // which holds the debuggee's clock ring for as long as the bit is up
    // --- the mode register written with RESET, whose 100 ns write pulse
    // holds RESET and restarts the ring at the register's load, then the
    // mode register proper and DBG-RESET-STATUS.  Neither reset moves the
    // PC; both leave RUN down; and RUN up runs the machine on from where
    // it stood, in step.
    let before = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::PC));
    lock.request(
        &mut c,
        &mut far,
        &mut clk,
        &mut r,
        DEBUG_MODIFIER,
        true,
        debug_modifier::RESET,
        !0,
    );
    lock.request(&mut c, &mut far, &mut clk, &mut r, DEBUG_MODIFIER, true, 0, !0);
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::MODE, spy::MODE_RESET);
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::MODE, 0);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o766044, 0);
    let pc = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::PC));
    assert_eq!(pc, before, "the PC stands where it was, on both");
    assert_eq!(c.bus(&n, "PC", 14) as u16, r.pc());
    let flags = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, unibus(spy::FLAG_1));
    assert_eq!(flags & 0x100, 0, "SRUN down on both");
    let status = lock.request(&mut c, &mut far, &mut clk, &mut r, DEBUG_STATUS, false, 0, !0).1;
    assert_eq!(status, Some(0xff00), "no error after DBG-RESET-STATUS, on both");
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::CLK, 1);
    for _ in 0..60 {
        generator_cycle(&mut c, &mut far, &mut clk, clk0);
        r.step().unwrap();
    }
    assert_ne!(r.spy_read(spy::FLAG_1) & 0x100, 0, "running again on rtl");
    assert_eq!(c.bus(&n, "PC", 14) as u16, r.pc(), "at the same PC on the board");
    assert!(r.pc() > pc, "and past where it stood");
    eprintln!(
        "CC-RESET-MACH through the cable: PC {pc:o} held, then running in step to {:o}",
        r.pc()
    );
}

/// **`LPC.HOLD` on the board, through the cable, held to `rtl`.**  With
/// both halted at the same PC, the OPC control register is written with
/// bit 0 through the debug cable and the machine single-stepped: the
/// board's fourteen `LPC` nets stay where they were while the PC moves,
/// on both; the bit cleared, one more step and `LPC` is the PC before the
/// one in `IR` again, on both.  `LPC` is read off the board's nets and off
/// `rtl`'s register, not through the cable: nothing on the diagnostic bus
/// carries it, which is why it had no test.
#[test]
fn chip_and_rtl_hold_and_release_lpc_alike() {
    use muir::engine::Engine;
    use muir::spy;

    let n = netlist::parse(NETLIST).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    let m = writes_a_diagnostic_register(spy::CLK, 0);
    let (mut c, mut clk, mut far, mut r) = same_program(&n, &m);
    let clk0 = cpu_clock(&n);
    let cable = DebuggerOnCable::new(&bus_n);
    for _ in 0..60 {
        generator_cycle(&mut c, &mut far, &mut clk, clk0);
        r.step().unwrap();
    }
    assert_eq!(r.spy_read(spy::FLAG_1) & 0x100, 0, "rtl halted");
    let halted_at = c.bus(&n, "PC", 14) as u16;
    assert_eq!(halted_at, r.pc(), "halted at the same PC");
    let lpc = |c: &Chip| c.bus(&n, "LPC", 14) as u16;
    assert_eq!(lpc(&c), r.lpc(), "LPC the same on both, halted");
    assert_eq!(r.lpc(), halted_at - 1, "the PC before the one in IR");
    let lock = Lockstep::new(&n, &cable, clk0, Debuggee::Halted, 0);

    // LPC.HOLD up, and three single steps: the PC moves, LPC does not.
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::OPC_CONTROL, 1);
    let held = lpc(&c);
    for k in 1..=3 {
        lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::CLK, 2);
        lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::CLK, 0);
        assert_eq!(c.bus(&n, "PC", 14) as u16, r.pc(), "the PC after step {k}, on both");
        assert_eq!(c.bus(&n, "PC", 14) as u16, halted_at + k, "one step each");
        assert_eq!(lpc(&c), held, "LPC held on the board through step {k}");
        assert_eq!(r.lpc(), held, "and on rtl");
    }
    eprintln!("LPC held at {held:o} through three steps to PC {:o} on both", r.pc());

    // Released: the next step loads it with the PC before the one in IR.
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::OPC_CONTROL, 0);
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::CLK, 2);
    lock.spy_write(&mut c, &mut far, &mut clk, &mut r, spy::CLK, 0);
    let pc = c.bus(&n, "PC", 14) as u16;
    assert_eq!(pc, r.pc());
    assert_eq!(lpc(&c), pc - 1, "LPC follows again on the board");
    assert_eq!(r.lpc(), pc - 1, "and on rtl");
    eprintln!("released: LPC {:o} under PC {pc:o} on both", r.lpc());
}

/// **A debug cycle against a running processor, on the board and on `rtl`
/// in lockstep.**  Every other board test halts the debuggee first, so the
/// arbitration's contended cases --- the debug master waiting for the
/// processor's Unibus cycle in flight, the processor waiting behind `DBUB
/// MASTER` and its request registered at the first edge after the release
/// --- go unexercised there.  Here
/// the debuggee runs a program of Unibus reads, the interface's error
/// status register every seven microcycles, and a debug read of its PC is
/// made twelve times at twelve phases of that program; each
/// acknowledgement's delay and each PC read must agree, and the two must
/// keep meeting at the same instants throughout, since a processor cycle
/// delayed differently on the two would put them on different clocks.
#[test]
fn chip_and_rtl_arbitrate_a_debug_cycle_against_a_running_processor_alike() {
    use microcode::*;
    use muir::busint::{self, DEBUG_ADDRESS, DEBUG_CYCLE, DEBUG_MODIFIER};
    use muir::clock::Clock;
    use muir::engine::Engine;
    use muir::isa::Insn;
    use muir::spy;

    let n = netlist::parse(NETLIST).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    let mut m = muir::machine::Machine::new();
    // Virtual page 0 on the interface's Unibus page: 766044, the error
    // status register, is virtual 22.  A register answered a fixed time
    // after `-UB MSYN`: the microsecond clock, whose answer waits for its
    // own edge, showed the board and `rtl` apart on this program before
    // any debug request.
    m.l1_map[0] = 0;
    m.l2_map[0] = (1 << 23) | (1 << 22) | 0o37766;
    m.amem[3] = 0o123456;
    m.mmem[1] = busint::unibus_physical(0o766044) & 0xff;
    m.mmem[2] = 0;
    let mut prom = vec![filler(); 512];
    // -RESET ERR first, since the board's UB MAP ERROR is up from power-on
    // and the words read back are compared.
    prom[1] = Insn::new(ALU | SETM | m_src(2) | a_src(3) | MD);
    prom[2] = Insn::new(ALU | SETM | m_src(1) | a_src(3) | START_WRITE);
    let reads = 60;
    for k in 0..reads {
        let at = 4 + 7 * k;
        prom[at] = Insn::new(ALU | SETM | m_src(1) | a_src(3) | START_READ);
        prom[at + 3] = Insn::new(ALU | SETM | SRC_MD | a_src(3) | a_dest(0o100 + k as u64));
    }
    m.load_prom(&prom);
    let (mut c, mut clk, mut far, mut r) = same_program(&n, &m);
    let clk0 = cpu_clock(&n);
    let a = Ram::new(&c, &n, &MEMS[0]);
    let cable = DebuggerOnCable::new(&bus_n);
    for _ in 0..8 {
        generator_cycle(&mut c, &mut far, &mut clk, clk0);
    }
    meet(&n, &mut c, &mut far, &mut clk, &mut r, clk0);
    assert_ne!(r.spy_read(spy::FLAG_1) & 0x100, 0, "rtl running");

    let lock = Lockstep::new(&n, &cable, clk0, Debuggee::Running, 0);
    let uaddr = unibus(spy::PC);
    let mut delays = Vec::new();
    for phase in 0..12 {
        // A different phase of the program each time: the strobes take a
        // register cycle each, the read a cycle through the arbitration,
        // and the extra cycles here move the next request along.
        for _ in 0..phase {
            generator_cycle(&mut c, &mut far, &mut clk, clk0);
        }
        meet(&n, &mut c, &mut far, &mut clk, &mut r, clk0);
        let (delay, pc) = {
            let (modifier, address) = ((uaddr >> 17) as u16 & 1, (uaddr >> 1) as u16);
            lock.request(&mut c, &mut far, &mut clk, &mut r, DEBUG_MODIFIER, true, modifier, !0);
            lock.request(&mut c, &mut far, &mut clk, &mut r, DEBUG_ADDRESS, true, address, !0);
            lock.request(&mut c, &mut far, &mut clk, &mut r, DEBUG_CYCLE, false, 0, !0)
        };
        let pc = pc.expect("nothing drove DBD");
        eprintln!(
            "phase {phase}: PC {pc:o} read {delay} ns after the request, at {} ns",
            clk.time_ns()
        );
        delays.push((delay, pc));
        assert!(pc > 4 && (pc as usize) < 4 + 7 * reads + 8, "a PC in the program: {pc:o}");
    }
    eprintln!(
        "twelve PC reads against the running processor: {}",
        delays.iter().map(|(d, pc)| format!("{d} ns -> {pc:o}")).collect::<Vec<_>>().join(", ")
    );
    assert!(delays.iter().any(|(d, _)| *d != delays[0].0), "the phases differed");
    assert_ne!(r.spy_read(spy::FLAG_1) & 0x100, 0, "rtl still running");
    // The processor's own reads: the same words parked on both.
    for _ in 0..40 {
        generator_cycle(&mut c, &mut far, &mut clk, clk0);
    }
    meet(&n, &mut c, &mut far, &mut clk, &mut r, clk0);
    let done = (0..reads).take_while(|&k| r.machine().amem[0o100 + k] != 0).count();
    assert!(done > 12, "the processor read the register {done} times");
    for k in 0..done {
        assert_eq!(a.word(&c, 0o100 + k), r.machine().amem[0o100 + k], "read {k}");
    }
    eprintln!("{done} reads of the error status parked alike on both, at {} ns", r.ns());
}

// --- The debug cable on DBGOUT -----------------------------------------------

/// The far end of the debugger's cable as a harness plays it: the DBGOUT
/// connector's wires watched for the request --- `-DEBUG OUT REQ`, `DEBUG
/// OUT A<1:0>`, `DEBUG OUT WR`, `DBD<15:0>` --- and answered on `DEBUG OUT
/// ACK` and, for a read, `DBD`.  With a cable plugged in the acknowledge
/// rests low; `busint_board`'s pull-up on it is the cable unplugged.
struct DebuggeeOnCable {
    req: netlist::NetId,
    a0: netlist::NetId,
    a1: netlist::NetId,
    wr: netlist::NetId,
    ack: netlist::NetId,
    dbd: Vec<netlist::NetId>,
    req_was: muir::part::Level,
}

impl DebuggeeOnCable {
    /// How long after the acknowledgement the board lifts its request:
    /// `SELECT DEBUG` with `-UB MSYN` at `SSYN T100`, measured in
    /// `chip_and_rtl_drive_the_debug_cable_alike` on every request.  Given
    /// to the debuggee with each request, as `rtl`'s DBGOUT gives it, so
    /// that the debuggee can run ahead of the release.
    const HOLD_NS: u64 = muir::busint::UNIBUS_STROBE_NS;

    fn new(bus_n: &netlist::Netlist, board: &mut Chip) -> DebuggeeOnCable {
        use muir::part::Level;
        let net = |name: &str| net_named(bus_n, name);
        let cable = DebuggeeOnCable {
            req: net("-DEBUG OUT REQ"),
            a0: net("DEBUG OUT A0"),
            a1: net("DEBUG OUT A1"),
            wr: net("DEBUG OUT WR"),
            ack: net("DEBUG OUT ACK"),
            dbd: (0..16).map(|k| net(&format!("DBD{k}"))).collect(),
            req_was: Level::High,
        };
        board.drive(cable.ack, Level::Low);
        board.transition(0);
        cable
    }

    /// The request's edge since the last look, if it moved: down, a
    /// [`muir::busint::CableEvent::Request`] with the wires as the board
    /// holds them --- `DBD` meaning something for a write only; up, a
    /// release.
    fn observe(&mut self, board: &Chip, now: u64) -> Option<muir::busint::CableEvent> {
        use muir::busint::{CableEvent, DebugRequest};
        use muir::part::Level;
        let req = board.net(self.req);
        let event = match (self.req_was, req) {
            (Level::High, Level::Low) => Some(CableEvent::Request {
                at: now,
                request: DebugRequest {
                    strobe: ((board.net(self.a1) == Level::High) as u8) << 1
                        | (board.net(self.a0) == Level::High) as u8,
                    write: board.net(self.wr) == Level::High,
                    dbd: board.read(&self.dbd) as u16,
                    hold_ns: Self::HOLD_NS,
                },
            }),
            (Level::Low, Level::High) => Some(CableEvent::Release { at: now }),
            _ => None,
        };
        self.req_was = req;
        event
    }

    /// `DEBUG ACK` up, with the bits of `word` under `driven` on `DBD`.
    fn answer(&self, board: &mut Chip, word: Option<u16>, driven: u16) {
        use muir::part::Level;
        if let Some(w) = word {
            for (k, &net) in self.dbd.iter().enumerate() {
                if (driven >> k) & 1 != 0 {
                    board.drive(net, Level::from((w >> k) & 1 != 0));
                }
            }
        }
        board.drive(self.ack, Level::High);
    }

    /// The far end at rest: `DEBUG ACK` down, `DBD` let go.
    fn quiet(&self, board: &mut Chip) {
        use muir::part::Level;
        board.drive(self.ack, Level::Low);
        for &net in &self.dbd {
            board.pull_up(net);
        }
    }
}

/// Every wire the two logs compare, per request, in nanoseconds from the
/// machine's start: when the request went on, the strobe, the direction,
/// the word for a write, when it was acknowledged, when it was lifted.
type CableLog = Vec<(u64, u8, bool, u16, Option<u64>, Option<u64>)>;

/// **DBGOUT on the board: the debugger's cycles into its debug block are
/// requests on the cable, timed as `rtl` times them, and its interface
/// gives up on an unanswered one when the REQTIM PROM says.**  The same
/// program on the board and on `rtl` --- CC's `DBG-WRITE` of `766006`, a
/// `DBG-READ` of `766012`, a `DBG-READ` of the debuggee's Chaosnet
/// interface --- against one scripted far end: the strobes acknowledged at
/// once, the two cycles 1010 ns after their request, the longest a
/// debuggee's DBGIN takes over a cycle
/// (`a_chip_debugger_halts_and_reads_a_chip_debuggee`), the third never.
/// Every request's instant, acknowledgement and lift agree to the
/// nanosecond.  The unanswered one is given up on by the board's timeout
/// counter and flagged `UB NXM ERROR`, and the wait is read off the
/// board's own wires: `NXM TIMEOUT` rises [`busint::DEBUG_TIMEOUT_NS`]
/// after `INT BUSY` started the counter at the grant --- the REQTIM PROM's
/// second table, 26 microseconds where the PROM's own header says 30 ---
/// and the request lifts [`busint::UNIBUS_STROBE_NS`] after that, with
/// `-UB MSYN` at `SSYN T100`.
#[test]
fn chip_and_rtl_drive_the_debug_cable_alike() {
    use muir::busint::{self, CableEvent, DEBUG_CYCLE};
    use muir::clock::Clock;
    use muir::engine::Engine;
    use muir::lashup::DebugProgram;
    use muir::machine::bus_error;
    use muir::part::Level;
    use muir::spy;

    let n = netlist::parse(NETLIST).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    let unibus = |eadr: u8| spy::BASE + 2 * eadr as u32;
    let mut p = DebugProgram::new();
    p.dbg_write(unibus(spy::CLK), 0);
    p.dbg_read(unibus(spy::PC), 0o101);
    p.dbg_read(0o764140, 0o102);
    let (mut c, mut clk, mut far, mut r) = same_program(&n, &p.finish());
    let a = Ram::new(&c, &n, &MEMS[0]);
    let mut cable = DebuggeeOnCable::new(&bus_n, &mut far.board);
    far.join(&mut c, clk.time_ns());
    r.attach_debug_cable();

    // The far end's script, by debug cycle: how long after the request the
    // acknowledgement comes and the word for a read, or never.  A register
    // strobe is acknowledged as it is made, as DBGIN acknowledges it.
    const ANSWER_NS: u64 = 1010;
    let script: [Option<(u64, Option<u16>)>; 3] =
        [Some((ANSWER_NS, None)), Some((ANSWER_NS, Some(0o1234))), None];
    let plan = |strobe: u8, cycle: usize| -> Option<(u64, Option<u16>)> {
        if strobe == DEBUG_CYCLE { script[cycle] } else { Some((0, None)) }
    };
    const REQUESTS: usize = 9;
    let done = |log: &CableLog| log.len() == REQUESTS && log[REQUESTS - 1].5.is_some();

    // The board, the far end answering on the wires; and the timeout
    // counter's own: `INT BUSY`, up from the grant, and `NXM TIMEOUT`, up
    // when the REQTIM PROM says.
    let (int_busy, nxm_timeout) = (net_named(&bus_n, "INT BUSY"), net_named(&bus_n, "NXM TIMEOUT"));
    let mut counter_was = (far.board.net(int_busy), far.board.net(nxm_timeout));
    let (mut granted_at, mut timed_out_at): (Option<u64>, Option<u64>) = (None, None);
    let t0 = clk.time_ns();
    let mut chip: CableLog = Vec::new();
    let mut due: Option<(u64, Option<u16>)> = None;
    let mut cycles = 0;
    while !done(&chip) {
        let now = clk.time_ns();
        assert!(now - t0 < 150_000, "the board's program did not finish: {chip:?}");
        let e = far.next_event(&c, &clk).expect("nothing on any board to wait for");
        if let Some((t, word)) = due
            && t <= e
        {
            assert!(t >= now, "the acknowledgement's instant has passed");
            if t > now {
                clk.pass(t, false);
                c.transition(t);
                far.board.transition(t);
            }
            cable.answer(&mut far.board, word, if word.is_some() { 0xffff } else { 0 });
            far.board.transition(t);
            far.join(&mut c, t);
            chip.last_mut().unwrap().4 = Some(t - t0);
            due = None;
            continue;
        }
        far.tick_with(&mut c, &mut clk);
        let now = clk.time_ns();
        // `INT BUSY` is an open-collector net, the 74S02O, 74S08O and
        // 74S10O at REQTIM 0A05, 0B16 and 0C13: up is the pull-up, `Z`,
        // which `Level::read` reads as a one.
        let counter = (far.board.net(int_busy), far.board.net(nxm_timeout));
        if counter.0.read() == Some(true)
            && counter_was.0.read() != Some(true)
            && timed_out_at.is_none()
        {
            granted_at = Some(now);
        }
        if counter.1 == Level::High && counter_was.1 != Level::High {
            timed_out_at.get_or_insert(now);
        }
        counter_was = counter;
        match cable.observe(&far.board, now) {
            Some(CableEvent::Request { at, request }) => {
                let dbd = if request.write { request.dbd } else { 0 };
                chip.push((at - t0, request.strobe, request.write, dbd, None, None));
                let k = cycles;
                if request.strobe == DEBUG_CYCLE {
                    cycles += 1;
                }
                if let Some((delay, word)) = plan(request.strobe, k) {
                    due = Some((at + delay, word));
                }
            }
            Some(CableEvent::Release { at }) => {
                chip.last_mut().unwrap().5 = Some(at - t0);
                cable.quiet(&mut far.board);
                far.board.transition(at);
                far.join(&mut c, at);
            }
            None => {}
        }
    }

    // `rtl`, the far end answering through the model's own hands.
    let r0 = r.ns();
    let mut rtl: CableLog = Vec::new();
    let mut cycles = 0;
    while !done(&rtl) {
        assert!(r.ns() - r0 < 150_000, "rtl's program did not finish: {rtl:?}");
        r.step().unwrap();
        match r.debug_out_take() {
            Some(CableEvent::Request { at, request }) => {
                let dbd = if request.write { request.dbd } else { 0 };
                rtl.push((at - r0, request.strobe, request.write, dbd, None, None));
                let k = cycles;
                if request.strobe == DEBUG_CYCLE {
                    cycles += 1;
                }
                if let Some((delay, word)) = plan(request.strobe, k) {
                    assert!(r.debug_out_answer(at + delay, word), "rtl was not waiting");
                    let last = rtl.last_mut().unwrap();
                    last.4 = Some(at + delay - r0);
                    // The model lifts the request `hold_ns` after the
                    // acknowledgement: `-UB MSYN` at `SSYN T100`.
                    last.5 = Some(at + delay + request.hold_ns - r0);
                }
            }
            Some(CableEvent::Release { at }) => rtl.last_mut().unwrap().5 = Some(at - r0),
            None => {}
        }
    }

    for (k, (c, r)) in chip.iter().zip(&rtl).enumerate() {
        let show = |e: &(u64, u8, bool, u16, Option<u64>, Option<u64>)| {
            format!(
                "at {} strobe {} {} {:o}, acked {:?}, lifted {:?}",
                e.0,
                e.1,
                if e.2 { "write" } else { "read" },
                e.3,
                e.4.map(|t| t - e.0),
                e.5.map(|t| t - e.0)
            )
        };
        eprintln!("request {k}: board {}; rtl {}", show(c), show(r));
    }
    assert_eq!(chip, rtl, "the cable as the board drives it against rtl");
    // The last read's word lands after the hang that waited for it.
    let clk0 = cpu_clock(&n);
    for _ in 0..20 {
        generator_cycle(&mut c, &mut far, &mut clk, clk0);
        r.step().unwrap();
    }
    // The unanswered cycle on the board's own wires: the counter runs from
    // `INT BUSY` at the grant, `NXM TIMEOUT` is up when the REQTIM PROM's
    // second table says, and the request lifts with `-UB MSYN` at `SSYN
    // T100`.
    let unanswered = chip[REQUESTS - 1];
    let granted = granted_at.expect("the unanswered cycle was never granted") - t0;
    let timed_out = timed_out_at.expect("NXM TIMEOUT never rose on the board") - t0;
    let lifted = unanswered.5.unwrap();
    eprintln!(
        "the unanswered cycle: granted at {granted} ns, requested at {} ns, NXM TIMEOUT at \
         {timed_out} ns, lifted at {lifted} ns --- {} ns after the request",
        unanswered.0,
        lifted - unanswered.0
    );
    assert_eq!(
        timed_out - granted,
        busint::DEBUG_TIMEOUT_NS,
        "the board gives a debug cycle up DEBUG_TIMEOUT_NS after INT BUSY started the counter at \
         the grant: the REQTIM PROM's second table, count 13 of its 2 microsecond intervals"
    );
    assert_eq!(
        lifted,
        timed_out + busint::UNIBUS_STROBE_NS,
        "the request lifts with -UB MSYN at SSYN T100, UNIBUS_STROBE_NS after NXM TIMEOUT"
    );

    assert_eq!(a.word(&c, 0o101), 0o1234, "the read's word on the board");
    assert_eq!(r.machine().amem[0o101], 0o1234, "and on rtl");
    eprintln!(
        "the unanswered read's word: board {:o}, rtl {:o}",
        a.word(&c, 0o102),
        r.machine().amem[0o102]
    );
    assert_eq!(a.word(&c, 0o102), r.machine().amem[0o102], "the word of the read nothing answered");
    assert_eq!(r.machine().bus_error, bus_error::UNIBUS_NXM, "rtl flags the timeout");
    let nxm = bus_n.by_name_id("'UB NXM ERROR'").unwrap();
    assert_eq!(far.board.net(nxm), Level::High, "and so does the board");
}

/// **A netlist debugger halts and steps an `rtl` debuggee over the cable:
/// rtl-chip.**  The board runs CC's `DBG-WRITE` and `DBG-READ` from its
/// boot PROM; its DBGOUT connector's wires are carried to the `rtl`
/// machine's DBGIN as they move, and the `rtl` machine's `DEBUG ACK` and
/// word are driven back at their instant.  Each machine runs only as far
/// as the other has promised, as `Lashup` runs two `rtl`s: the board
/// promises its next event, the debuggee its earliest acknowledgement.
#[test]
fn a_chip_debugger_halts_and_reads_an_rtl_debuggee() {
    use muir::busint::CableEvent;
    use muir::clock::Clock;
    use muir::engine::Engine;
    use muir::lashup::{DebugProgram, max_step_ns};
    use muir::part::Level;
    use muir::rtl::Rtl;
    use muir::spy;

    let n = netlist::parse(NETLIST).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    let unibus = |eadr: u8| spy::BASE + 2 * eadr as u32;
    let mut p = DebugProgram::new();
    p.dbg_write(unibus(spy::CLK), 0);
    p.dbg_read(unibus(spy::PC), 0o101);
    p.dbg_write(unibus(spy::CLK), 2);
    p.dbg_write(unibus(spy::CLK), 0);
    p.dbg_read(unibus(spy::PC), 0o102);
    let (mut c, mut clk, mut far, _twin) = same_program(&n, &p.finish());
    let clk0 = cpu_clock(&n);
    let a = Ram::new(&c, &n, &MEMS[0]);
    let mut cable = DebuggeeOnCable::new(&bus_n, &mut far.board);
    far.join(&mut c, clk.time_ns());

    // The debuggee: `rtl` running fillers, its PC climbing one a microcycle.
    let mut bm = muir::machine::Machine::new();
    bm.amem[3] = 0o123456;
    bm.load_prom(&vec![microcode::filler(); 512]);
    let mut b = Rtl::new(bm);
    b.boot();

    // The board's side of the protocol `Lashup` runs between two `rtl`s.
    // With nothing on the cable the board promises its next event, before
    // which no wire of its can move; with a request out it promises the
    // release at its timeout --- `DEBUG_TIMEOUT_NS` from a grant made
    // before the request --- and nothing sooner, the lift after an
    // acknowledgement being the debuggee's to know from `hold_ns`.  The
    // debuggee promises its earliest acknowledgement, and once that is
    // carried, nothing.
    let slack = max_step_ns();
    let (mut pending, mut carried) = (false, true);
    let (mut requested_at, mut acked_at) = (0, 0);
    let (mut requests, mut releases) = (0, 0);
    let (mut chip_steps, mut rtl_steps) = (0u64, 0u64);
    let t0 = clk.time_ns();
    while releases < 15 {
        let now = clk.time_ns();
        assert!(now - t0 < 200_000, "the board's program did not finish: {requests} requests");
        let e = far.next_event(&c, &clk).expect("nothing on any board to wait for");
        // The debuggee's answer, driven at its instant.
        if pending
            && !carried
            && let Some((t_ack, word)) = b.debug_ack()
            && t_ack <= e
        {
            assert!(t_ack >= now, "the acknowledgement's instant has passed on the board");
            if t_ack > now {
                clk.pass(t_ack, false);
                c.transition(t_ack);
                far.board.transition(t_ack);
            }
            cable.answer(&mut far.board, word, if word.is_some() { 0xffff } else { 0 });
            far.board.transition(t_ack);
            far.join(&mut c, t_ack);
            carried = true;
            acked_at = t_ack;
            continue;
        }
        let back = if pending && !carried { b.debug_in_promise() } else { u64::MAX };
        let out = if pending && !carried {
            requested_at + muir::busint::DEBUG_TIMEOUT_NS - 1_000
        } else {
            e
        };
        let chip_may = e <= back;
        let b_may = b.ns() + slack < out;
        if chip_may && (now <= b.ns() || !b_may) {
            far.tick_with(&mut c, &mut clk);
            chip_steps += 1;
            let now = clk.time_ns();
            match cable.observe(&far.board, now) {
                Some(CableEvent::Request { at, request }) => {
                    b.debug_request(at, request);
                    pending = true;
                    carried = false;
                    requested_at = at;
                    requests += 1;
                }
                Some(CableEvent::Release { at }) => {
                    if carried {
                        // Lifted `HOLD_NS` after the acknowledgement, which
                        // the debuggee was told; it has let go on its own.
                        assert_eq!(at, acked_at + DebuggeeOnCable::HOLD_NS, "the board's lift");
                    } else {
                        b.debug_release(at);
                    }
                    pending = false;
                    releases += 1;
                    cable.quiet(&mut far.board);
                    far.board.transition(at);
                    far.join(&mut c, at);
                }
                None => {}
            }
        } else if b_may {
            b.step_until(out - slack).unwrap();
            rtl_steps += 1;
        } else {
            panic!(
                "neither machine may move: the board at {now} ns with its next event at {e} \
                 promising {out}, the debuggee at {} ns promising {back}",
                b.ns()
            );
        }
    }
    // The last read's word lands in A memory after the fillers behind it,
    // and the debuggee, which lags the board, runs past its own release.
    for _ in 0..40 {
        generator_cycle(&mut c, &mut far, &mut clk, clk0);
        b.step().unwrap();
    }

    assert_eq!(requests, 15, "five DBG operations, three cycles each");
    assert_eq!(b.spy_read(spy::FLAG_1) & 0x100, 0, "the debuggee is halted");
    let pc = a.word(&c, 0o101);
    eprintln!(
        "the rtl debuggee halted at PC {pc:o}, stepped to {:o} by the board; {chip_steps} board \
         events, {rtl_steps} rtl steps",
        a.word(&c, 0o102)
    );
    assert!(pc > 0 && pc < 0o400, "a PC in the debuggee's PROM, past the boot: {pc:o}");
    assert_eq!(a.word(&c, 0o102), pc + 1, "one step through the cable moved it by one");
    assert_eq!(b.pc() as u32, pc + 1, "and that is where the debuggee stands");
    assert_eq!(b.machine().bus_error, 0);
    let nxm = bus_n.by_name_id("'UB NXM ERROR'").unwrap();
    assert_eq!(far.board.net(nxm), Level::Low, "no cycle of the board's timed out");
    assert!(!b.debug_busy(), "the cable is quiet");
}

// --- The Unibus map, on the board ---------------------------------------------

impl DebuggerOnCable {
    /// A request the debuggee will not acknowledge: on at a boundary, held
    /// `wait_ns`, lifted, and the boards run on to the next boundary.
    /// Returns when it went on and how many boundaries passed before and
    /// after the lift, for `rtl` to be stepped the same and released at the
    /// same instant.
    fn request_refused(
        &self,
        c: &mut Chip,
        far: &mut FarEnd,
        clk: &mut muir::clock::Behavioural,
        write: bool,
        dbd: u16,
        wait_ns: u64,
    ) -> (u64, usize, usize) {
        use muir::clock::Clock;
        use muir::part::Level;
        let at = clk.time_ns();
        assert_eq!(clk.phase_ns(), 0);
        far.board.drive(self.a0, Level::Low);
        far.board.drive(self.a1, Level::Low);
        far.board.drive(self.wr, Level::from(write));
        for (k, &net) in self.dbd.iter().enumerate() {
            if write {
                far.board.drive(net, Level::from((dbd >> k) & 1 != 0));
            } else {
                far.board.pull_up(net);
            }
        }
        far.board.drive(self.req, Level::Low);
        far.board.transition(at);
        far.join(c, at);
        let mut before = 0;
        // A boundary is counted once, when time has passed to it.
        let mut last_boundary = at;
        while clk.time_ns() < at + wait_ns {
            assert_eq!(far.board.net(self.ack), Level::Low, "the refused cycle was acknowledged");
            let lift = at + wait_ns;
            match far.next_event(c, clk) {
                Some(e) if e <= lift => {
                    far.tick_with(c, clk);
                }
                _ => {
                    clk.pass(lift, false);
                    c.transition(lift);
                    far.board.transition(lift);
                    far.join(c, lift);
                }
            }
            if clk.phase_ns() == 0 && clk.time_ns() < at + wait_ns && clk.time_ns() > last_boundary
            {
                before += 1;
                last_boundary = clk.time_ns();
            }
        }
        far.board.drive(self.req, Level::High);
        far.board.transition(clk.time_ns());
        for &net in &self.dbd {
            far.board.pull_up(net);
        }
        far.board.transition(clk.time_ns());
        far.join(c, clk.time_ns());
        let mut after = 0;
        let lifted = clk.time_ns();
        loop {
            far.tick_with(c, clk);
            if clk.phase_ns() == 0 && clk.time_ns() > lifted {
                after += 1;
                break;
            }
        }
        (at, before, after)
    }
}

/// **The Unibus map on the board, from the debug cable: a mapped word
/// written in two halves lands in the memory boards, comes back in two,
/// and a page marked read-only or invalid refuses the Xbus half without
/// answering, `UB MAP ERROR` up --- and `rtl` answers every request at
/// the same instant with the same word.**  The Xbus half's instant is the
/// memory board's: `UBXRQ` a section after `-UB MSYN`, registered at the
/// master clock, granted at the edge after, `-XBUS RQ`, the board's own
/// clock, `XACK`, and `SSYN` 100 ns on for a read; the buffer halves are
/// register cycles.  CC's `DBG-WRITE-XBUS` and `DBG-READ-XBUS` are these
/// requests.
#[test]
fn chip_and_rtl_answer_the_unibus_map_alike() {
    use muir::busint::{self, DEBUG_ADDRESS, DEBUG_CYCLE, DEBUG_MODIFIER, DEBUG_STATUS};
    use muir::clock::Clock;
    use muir::engine::Engine;
    use muir::machine::bus_error;
    use muir::spy;

    let n = netlist::parse(NETLIST).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    let m = writes_a_diagnostic_register(spy::CLK, 0);
    let (mut c, mut clk, mut far, mut r) = same_program(&n, &m);
    let clk0 = cpu_clock(&n);
    let cable = DebuggerOnCable::new(&bus_n);
    for _ in 0..60 {
        generator_cycle(&mut c, &mut far, &mut clk, clk0);
        r.step().unwrap();
    }
    assert_eq!(clk.time_ns(), r.ns(), "in step, on one clock");
    assert_eq!(r.spy_read(spy::FLAG_1) & 0x100, 0, "rtl halted");
    assert_eq!(c.bus(&n, "PC", 14) as u16, r.pc(), "both halted at the same PC");

    // `MUIR_MAP_TRACE` prints the wire timeline of every debug cycle.
    let trace = if std::env::var_os("MUIR_MAP_TRACE").is_some() { u32::MAX } else { 0 };
    let lock = Lockstep::new(&n, &cable, clk0, Debuggee::Halted, trace);
    // A cycle the map refuses, on both, lifted after `wait_ns`.
    let refused = |c: &mut Chip,
                   far: &mut FarEnd,
                   clk: &mut muir::clock::Behavioural,
                   r: &mut muir::rtl::Rtl,
                   uaddr: u32,
                   write: Option<u16>| {
        lock.request(c, far, clk, r, DEBUG_MODIFIER, true, (uaddr >> 17) as u16 & 1, !0);
        lock.request(c, far, clk, r, DEBUG_ADDRESS, true, (uaddr >> 1) as u16, !0);
        lock.idle(c, far, clk, r);
        const WAIT_NS: u64 = 5_000;
        let (_, before, after) =
            cable.request_refused(c, far, clk, write.is_some(), write.unwrap_or(0), WAIT_NS);
        let rat = r.ns();
        r.debug_request(
            rat,
            busint::DebugRequest {
                strobe: DEBUG_CYCLE,
                write: write.is_some(),
                dbd: write.unwrap_or(0),
                hold_ns: Lockstep::HOLD_NS,
            },
        );
        for _ in 0..before {
            r.step().unwrap();
        }
        assert_eq!(r.debug_ack(), None, "rtl acknowledged a refused cycle");
        r.debug_release(rat + WAIT_NS);
        for _ in 0..after {
            r.step().unwrap();
        }
    };

    // The error register cleared first: the board's UB MAP ERROR is up
    // from power-on.
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o766044, 0);
    let status = lock.request(&mut c, &mut far, &mut clk, &mut r, DEBUG_STATUS, false, 0, !0).1;
    assert_eq!(status, Some(0xff00), "no error");

    // Map 17 onto physical page 2, valid and writable; write word 1000 in
    // two halves; read it back in two.
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o766176, 0o140002);
    let low = 0o140000 + 0o17 * 0o2000;
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, low, 0o123456);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, low + 2, 0o7654);
    let word = 0o7654 << 16 | 0o123456;
    assert_eq!(r.machine().main[0o1000], word, "the word in rtl's memory");
    let on_board = far.xbus.peek(0o1000).unwrap_or(far.buses.machine.main[0o1000]);
    assert_eq!(on_board, word, "the word in the memory boards");
    let lo = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, low);
    let hi = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, low + 2);
    assert_eq!((hi as u32) << 16 | lo as u32, word, "the word read back through the map");

    // CC-WRITE-MD: map 16 loaded with 177000, whose page has its high five
    // bits ones; the low half buffered, the high half into MD by -UB TO
    // MD, no Xbus cycle --- on both, at the same instant.
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o766174, 0o177000);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o174000, 0o52525);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o174002, 0o125252);
    let md_word = 0o125252 << 16 | 0o52525;
    assert_eq!(r.machine().md, md_word, "MD on rtl");
    assert_eq!(!c.bus(&n, "-MD", 32) as u32, md_word, "MD on the board");
    assert_eq!(r.machine().main[0o1000], word, "memory untouched on rtl");
    let on_board = far.xbus.peek(0o1000).unwrap_or(far.buses.machine.main[0o1000]);
    assert_eq!(on_board, word, "and on the boards");

    // Map 16 onto the same page read-only: the write's low half is
    // buffered, its high half refused; map 15 invalid: the read's low half
    // refused.  UB MAP ERROR on both.
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o766174, 0o100002);
    let ro = 0o140000 + 0o16 * 0o2000;
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, ro, 0o777);
    refused(&mut c, &mut far, &mut clk, &mut r, ro + 2, Some(0o666));
    assert_eq!(r.machine().main[0o1000], word, "the read-only page kept its word on rtl");
    let on_board = far.xbus.peek(0o1000).unwrap_or(far.buses.machine.main[0o1000]);
    assert_eq!(on_board, word, "and on the boards");
    let status = lock.request(&mut c, &mut far, &mut clk, &mut r, DEBUG_STATUS, false, 0, !0).1;
    assert_eq!(status, Some(0xff00 | bus_error::UB_MAP_ERROR), "UB MAP ERROR, on both");
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o766172, 0);
    let bad = 0o140000 + 0o15 * 0o2000;
    refused(&mut c, &mut far, &mut clk, &mut r, bad, None);
    let hi = lock.dbg_read(&mut c, &mut far, &mut clk, &mut r, bad + 2);
    eprintln!("the read buffer of a page never read through: {hi:o} on both");

    // Write-through mode, bit 7 of the error status register: on the upper
    // eight pages the low half's write is an Xbus write at once, of the
    // Unibus word with zeros above it; the high half's write is the usual
    // one, the buffer having taken the low half too.  On the lower eight
    // pages nothing changes.  Every acknowledgement and every word in the
    // memory boards the same on both.
    let peek = |far: &FarEnd| far.xbus.peek(0o1000).unwrap_or(far.buses.machine.main[0o1000]);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o766044, 0o200);
    let status = lock.request(&mut c, &mut far, &mut clk, &mut r, DEBUG_STATUS, false, 0, !0).1;
    assert_eq!(status, Some(0xff00 | 0o200), "write-through on, no error, on both");
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o766176, 0o140002);
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, low, 0o111111);
    assert_eq!(peek(&far), 0o111111, "the low half written through, zeros above it");
    assert_eq!(r.machine().main[0o1000], 0o111111, "and on rtl");
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, low + 2, 0o22222);
    assert_eq!(peek(&far), 0o22222 << 16 | 0o111111, "the high half's write, both halves");
    assert_eq!(r.machine().main[0o1000], 0o22222 << 16 | 0o111111, "and on rtl");
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, 0o766156, 0o140002);
    let low7 = 0o140000 + 0o7 * 0o2000;
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, low7, 0o33333);
    assert_eq!(peek(&far), 0o22222 << 16 | 0o111111, "map 7: the low half only buffered");
    assert_eq!(r.machine().main[0o1000], 0o22222 << 16 | 0o111111, "and on rtl");
    lock.dbg_write(&mut c, &mut far, &mut clk, &mut r, low7 + 2, 0o44444);
    assert_eq!(peek(&far), 0o44444 << 16 | 0o33333, "map 7: the high half's write");
    assert_eq!(r.machine().main[0o1000], 0o44444 << 16 | 0o33333, "and on rtl");
    eprintln!(
        "write-through: map 17's low half written through at once, map 7's buffered, on both"
    );
}

/// **The processor's own Unibus cycle into the mapped range, on the board
/// and on `rtl`.**  The map is the bus interface's, for any master on its
/// Unibus; the processor's own cycle into `140000`-`177777` finds it too.
/// What the board does with it --- a mapped Xbus cycle while the processor
/// is the Unibus master, or a cycle nothing answers --- is what `rtl` must
/// do.  The program writes map register `17`, reads the two halves of the
/// mapped word and the error status through its own Unibus, parks them in
/// A memory, and halts; the words, the halt and its instant must agree.
#[test]
fn chip_and_rtl_answer_the_processors_own_mapped_cycle_alike() {
    use microcode::*;
    use muir::engine::Engine;
    use muir::isa::Insn;
    use muir::spy;

    let n = netlist::parse(NETLIST).unwrap();
    let mut m = muir::machine::Machine::new();
    // Virtual page 0 on Unibus 766000, virtual page 1 on Unibus 176000:
    // physical pages 37766 and 37770.
    m.l1_map[0] = 0;
    m.l2_map[0] = (1 << 23) | (1 << 22) | 0o37766;
    m.l2_map[1] = (1 << 23) | (1 << 22) | 0o37770;
    m.amem[3] = 0o123456;
    m.main[0o1000] = 0o7654 << 16 | 0o123456;
    let constants: [u32; 8] = [
        0o77,            // 1: map register 17, Unibus 766176, as a virtual address
        0o140002,        // 2: its value: physical page 2, valid, writable
        0o400,           // 3: Unibus 176000, the mapped word's low half
        0o401,           // 4: Unibus 176002, its high half
        0o22,            // 5: the error status register, Unibus 766044
        0,               // 6: RUN down, and the error register's clear
        spy::CLK as u32, // 7: the clock control register
        0,
    ];
    for (k, &v) in constants.iter().enumerate() {
        m.mmem[k + 1] = v;
    }
    let mut prom = vec![filler(); 512];
    let mut at = 4;
    // -RESET ERR first: the board's UB MAP ERROR is up from power-on.
    prom[at] = Insn::new(ALU | SETM | m_src(6) | a_src(3) | MD);
    prom[at + 1] = Insn::new(ALU | SETM | m_src(5) | a_src(3) | START_WRITE);
    at += 22;
    prom[at] = Insn::new(ALU | SETM | m_src(2) | a_src(3) | MD);
    prom[at + 1] = Insn::new(ALU | SETM | m_src(1) | a_src(3) | START_WRITE);
    at += 22;
    for (src, park) in [(3, 0o101), (4, 0o102), (5, 0o103)] {
        prom[at] = Insn::new(ALU | SETM | m_src(src) | a_src(3) | START_READ);
        prom[at + 21] = Insn::new(ALU | SETM | SRC_MD | a_src(3) | a_dest(park));
        at += 22;
    }
    prom[at] = Insn::new(ALU | SETM | m_src(6) | a_src(3) | MD);
    prom[at + 1] = Insn::new(ALU | SETM | m_src(7) | a_src(3) | START_WRITE);
    m.load_prom(&prom);

    let (mut c, mut clk, mut far, mut r) = same_program(&n, &m);
    let clk0 = cpu_clock(&n);
    let a = Ram::new(&c, &n, &MEMS[0]);
    // Long enough for three timeouts and the halt.
    let cycles = 400;
    let mut chip_last_ran = 0;
    for k in 0..cycles {
        if generator_cycle(&mut c, &mut far, &mut clk, clk0) {
            chip_last_ran = k;
        }
    }
    let mut rtl_last_ran = 0;
    for k in 0..cycles {
        r.step().unwrap();
        if r.executed().is_some() {
            rtl_last_ran = k;
        }
    }
    for (k, park) in [0o101, 0o102, 0o103].iter().enumerate() {
        eprintln!("read {k}: board {:o} rtl {:o}", a.word(&c, *park), r.machine().amem[*park]);
    }
    eprintln!(
        "halted: board in generator cycle {chip_last_ran} at PC {:o}, rtl in step {rtl_last_ran} at PC {:o}; \
         rtl bus error {:o}",
        c.bus(&n, "PC", 14),
        r.pc(),
        r.machine().bus_error
    );
    assert!(chip_last_ran < cycles - 20, "the board halted");
    assert!(rtl_last_ran < cycles - 20, "rtl halted");
    assert_eq!(c.bus(&n, "PC", 14) as u16, r.pc(), "halted at the same PC");
    for park in [0o101, 0o102, 0o103] {
        assert_eq!(a.word(&c, park), r.machine().amem[park], "the word parked at {park:o}");
    }
    // The cycles time out --- the interface is not free for a mapped Xbus
    // cycle while its own processor is the Unibus master --- and leave
    // `UB NXM ERROR` behind, `-FREE` up, the high byte the pulled-up bus.
    assert_eq!(r.machine().amem[0o103], 0xff00 | 0o110, "UB NXM ERROR and -FREE, on both");
    assert_eq!(chip_last_ran, rtl_last_ran, "halted in the same generator cycle");
}

// --- Two boards on the debug cable ------------------------------------------

/// Brings a machine that has no event before `t` to `t`: time passes on its
/// clock, its boards settle there, and it is joined against its own cables
/// --- for a wire of the debug cable that the other machine just moved.
fn bring_to(c: &mut Chip, far: &mut FarEnd, clk: &mut muir::clock::Behavioural, t: u64) {
    use muir::clock::Clock;
    if clk.time_ns() < t {
        clk.pass(t, clk.next_at(c.clock_inputs()).is_none());
        c.transition(t);
    }
    far.board.transition(t);
    far.join(c, t);
}

/// **chip-chip: two netlist machines on the debug cable, and the one halts,
/// reads and steps the other.**  Two processors, two bus interfaces, two
/// backplanes; the debugger's DBGOUT connector to the debuggee's DBGIN
/// wire for wire ([`DebugCable`]), carried whenever either board moves,
/// each machine stepped at its own next event, the earlier first, and the
/// other brought to that instant to see the wire.  The debugger runs CC's
/// `DBG-WRITE` and `DBG-READ` from its boot PROM; the debuggee runs fillers
/// until the first write takes `RUN` down.  Every strobe is acknowledged in
/// the instant it is made, every cycle within the debuggee's arbitration
/// --- three master clock edges from wherever the request fell in the
/// debuggee's cycle, then `-UB MSYN` and the register cycle --- and the PC
/// read through the cable is the PC on the debuggee's board, before and
/// after the step.
#[test]
fn a_chip_debugger_halts_and_reads_a_chip_debuggee() {
    use muir::cable::DebugCable;
    use muir::clock::Clock;
    use muir::lashup::DebugProgram;
    use muir::part::Level;
    use muir::spy;

    let n = netlist::parse(NETLIST).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    let unibus = |eadr: u8| spy::BASE + 2 * eadr as u32;
    let mut p = DebugProgram::new();
    p.dbg_write(unibus(spy::CLK), 0);
    p.dbg_read(unibus(spy::PC), 0o101);
    p.dbg_write(unibus(spy::CLK), 2);
    p.dbg_write(unibus(spy::CLK), 0);
    p.dbg_read(unibus(spy::PC), 0o102);
    let (mut ca, mut clka, mut fa, _) = same_program(&n, &p.finish());
    let mut bm = muir::machine::Machine::new();
    bm.amem[3] = 0o123456;
    bm.load_prom(&vec![microcode::filler(); 512]);
    let (mut cb, mut clkb, mut fb, _) = same_program(&n, &bm);
    let a_mem = Ram::new(&ca, &n, &MEMS[0]);
    let clk0 = cpu_clock(&n);
    let mut cable = DebugCable::new(&bus_n, &bus_n);
    let net = |name: &str| net_named(&bus_n, name);
    let (req, ack, a0, a1) =
        (net("-DEBUG IN REQ"), net("DEBUG IN ACK"), net("DEBUG IN A0"), net("DEBUG IN A1"));
    let master = net("DBUB MASTER");
    let nxm = net("UB NXM ERROR");

    // Plugged in: the cable carried at both machines' present instants.
    let t = clka.time_ns().max(clkb.time_ns());
    bring_to(&mut ca, &mut fa, &mut clka, t);
    bring_to(&mut cb, &mut fb, &mut clkb, t);
    for _ in 0..12 {
        if !cable.exchange(&mut fa.board, &mut fb.board) {
            break;
        }
        bring_to(&mut ca, &mut fa, &mut clka, t);
        bring_to(&mut cb, &mut fb, &mut clkb, t);
    }

    // The cable's story, from the debuggee's connector: when each request
    // came, its strobe, and how long to the acknowledgement and the lift.
    let mut log: Vec<(u64, u8, Option<u64>, Option<u64>)> = Vec::new();
    let (mut req_was, mut ack_was) = (fb.board.net(req), fb.board.net(ack));
    let (mut a_events, mut b_events) = (0u64, 0u64);
    let t0 = t;
    while log.len() < 15 || log[14].3.is_none() {
        let ea = fa.next_event(&ca, &clka).expect("nothing on the debugger's boards to wait for");
        let eb = fb.next_event(&cb, &clkb).expect("nothing on the debuggee's boards to wait for");
        let t = if ea <= eb {
            fa.tick_with(&mut ca, &mut clka);
            a_events += 1;
            clka.time_ns()
        } else {
            fb.tick_with(&mut cb, &mut clkb);
            b_events += 1;
            clkb.time_ns()
        };
        assert!(t - t0 < 200_000, "the debugger's program did not finish: {log:?}");
        // The cable at `t`, until neither board moves a wire.
        for _ in 0..12 {
            if !cable.exchange(&mut fa.board, &mut fb.board) {
                break;
            }
            bring_to(&mut ca, &mut fa, &mut clka, t);
            bring_to(&mut cb, &mut fb, &mut clkb, t);
        }
        let (r, k) = (fb.board.net(req), fb.board.net(ack));
        if req_was == Level::High && r == Level::Low {
            let strobe = ((fb.board.net(a1) == Level::High) as u8) << 1
                | (fb.board.net(a0) == Level::High) as u8;
            log.push((t - t0, strobe, None, None));
        }
        if ack_was == Level::Low && k == Level::High {
            let last = log.last_mut().expect("an acknowledgement with no request");
            last.2 = Some(t - t0 - last.0);
        }
        if req_was == Level::Low && r == Level::High {
            let last = log.last_mut().expect("a lift with no request");
            last.3 = Some(t - t0 - last.0);
        }
        (req_was, ack_was) = (r, k);
    }
    // The last read's word lands after the fillers behind it; the debuggee's
    // master lets go a section after the lift.
    for _ in 0..40 {
        generator_cycle(&mut ca, &mut fa, &mut clka, clk0);
    }
    let mut b_ran = false;
    for _ in 0..40 {
        b_ran |= generator_cycle(&mut cb, &mut fb, &mut clkb, clk0);
    }

    for (k, (at, strobe, acked, lifted)) in log.iter().enumerate() {
        eprintln!(
            "request {k}: at {at} ns, strobe {strobe}, acknowledged {acked:?} on, lifted {lifted:?} on"
        );
    }
    eprintln!("{a_events} events on the debugger's boards, {b_events} on the debuggee's");
    for (k, &(_, strobe, acked, lifted)) in log.iter().enumerate() {
        let acked = acked.unwrap_or_else(|| panic!("request {k} was never acknowledged"));
        let lifted = lifted.unwrap();
        if strobe == muir::busint::DEBUG_CYCLE {
            // `NPRD` at the first edge after the request, the grant at the
            // next, `SACKD` and `DBUB MASTER` at the one after the section
            // to `SACK`, `-UB MSYN` a section on, `-UB SSYN` 250 on.
            assert!(
                (790..=1010).contains(&acked),
                "request {k}, a cycle, acknowledged {acked} ns on"
            );
        } else {
            assert_eq!(acked, 0, "request {k}, a strobe, acknowledged as it is made");
        }
        assert_eq!(lifted, acked + 100, "request {k} lifted at SSYN T100");
    }
    assert!(!b_ran, "the debuggee is halted");
    let pc = a_mem.word(&ca, 0o101);
    let stepped = a_mem.word(&ca, 0o102);
    let on_board = cb.bus(&n, "PC", 14) as u32;
    eprintln!(
        "the debuggee halted at PC {pc:o}, stepped to {stepped:o}; its board says {on_board:o}"
    );
    assert!(pc > 0 && pc < 0o400, "a PC in the debuggee's PROM, past the boot: {pc:o}");
    assert_eq!(stepped, pc + 1, "one step through the cable moved it by one");
    assert_eq!(on_board, pc + 1, "and that is where the debuggee's board stands");
    assert_eq!(fb.board.net(master), Level::Low, "the debuggee's master has let go");
    assert_eq!(fa.board.net(nxm), Level::Low, "no cycle of the debugger's timed out");
    assert_eq!(fb.board.net(nxm), Level::Low, "nor of the debuggee's");
}

// --- Rounds -----------------------------------------------------------------

/// A transition asks the parts again until a round moves nothing, up to
/// `Chip::UPDATE_ROUNDS`, and one that runs out is recorded in
/// `Chip::unconverged` rather than dropped, as a loop that does not settle
/// is recorded in `unsettled`. The processor's registers all clock from
/// the clock nets and it converges in a round; the far end has registers
/// clocked from other registers' outputs and takes more; and none of them
/// runs out over the first thousand microcycles of the boot, on any board.
#[test]
fn no_transition_runs_out_of_rounds() {
    use muir::clock::Behavioural;
    use muir::part::Level;
    let n = netlist::parse(NETLIST).unwrap();
    let image: Vec<u64> = muir::prom::boot_prom_image();
    let mut c = Chip::new(&n);
    c.power_on();
    c.load_prom(&n, &image);
    c.settle();
    let mut clk = Behavioural::new();
    // The first thousand microcycles of the boot reach no device, so the
    // far end needs no pack.
    let mut far = far_end(&n, muir::machine::Machine::new());
    far.join(&mut c, muir::clock::Clock::time_ns(&clk));
    let boot = n.by_name_id("-BOOT1").unwrap();
    c.set_net(boot, Level::Low);
    c.settle();
    for _ in 0..20 {
        c.tick(&mut clk);
    }
    c.set_net(boot, Level::High);
    let mut most = 0;
    let mut last = muir::clock::Clock::phase_ns(&clk);
    let mut ran = 0;
    while ran < 1_000 {
        far.tick_with(&mut c, &mut clk);
        most = most.max(c.rounds).max(far.board.rounds);
        let p = muir::clock::Clock::phase_ns(&clk);
        if p < last {
            ran += 1;
        }
        last = p;
    }
    assert_eq!(c.unconverged, None, "the processor ran out of rounds");
    assert_eq!(far.board.unconverged, None, "the bus interface ran out of rounds");
    for (k, b) in far.xbus.boards.iter().enumerate() {
        assert_eq!(b.unconverged, None, "memory board {k} ran out of rounds");
    }
    if let Some(u) = &far.unibus {
        assert_eq!(u.board.unconverged, None, "the I/O board ran out of rounds");
    }
    eprintln!("no transition ran out of rounds in {ran} microcycles; the most any took was {most}");
}

/// **The processor's own reads of the microsecond clock, from its first
/// Unibus cycle, on both engines alike.**  Sixty reads of `764120` every
/// seven microcycles, the first of them the run's first Unibus cycle.
/// The low half's answer waits for the I/O board's microsecond edge, so
/// where each engine reckons that edge shows in when each read ends: the
/// board and `rtl` stand at the same PC at every instant they are
/// brought to, and park the same counts.  They were 563 ns apart: a far
/// end built at power-on and joined 440 ns on had folded the I/O board's
/// oscillator edges since power-on into the join's one transition, nine
/// `MCLK` edges lost, and the board's microsecond edges ran late from
/// there (`FarEnd::join`).
#[test]
fn chip_and_rtl_read_the_microsecond_clock_alike_from_the_first_cycle() {
    use microcode::*;
    use muir::busint;
    use muir::engine::Engine;
    use muir::isa::Insn;

    let n = netlist::parse(NETLIST).unwrap();
    let mut m = muir::machine::Machine::new();
    let phys = busint::unibus_physical(muir::ioboard::USEC_LOW);
    m.l1_map[0] = 0;
    m.l2_map[0] = (1 << 23) | (1 << 22) | (phys >> 8);
    m.amem[3] = 0o123456;
    m.mmem[1] = phys & 0xff;
    let reads = 60;
    let mut prom = vec![filler(); 512];
    for k in 0..reads {
        let at = 4 + 7 * k;
        prom[at] = Insn::new(ALU | SETM | m_src(1) | a_src(3) | START_READ);
        prom[at + 3] = Insn::new(ALU | SETM | SRC_MD | a_src(3) | a_dest(0o100 + k as u64));
    }
    m.load_prom(&prom);
    let (mut c, mut clk, mut far, mut r) = same_program(&n, &m);
    let clk0 = cpu_clock(&n);
    let a = Ram::new(&c, &n, &MEMS[0]);
    let mut parked = 0;
    for _ in 0..400 {
        generator_cycle(&mut c, &mut far, &mut clk, clk0);
        meet(&n, &mut c, &mut far, &mut clk, &mut r, clk0);
        assert_eq!(c.bus(&n, "PC", 14) as u64, r.pc() as u64, "the PC at {} ns", r.ns());
        parked = (0..reads).take_while(|&k| r.machine().amem[0o100 + k] != 0).count();
        for k in 0..parked {
            assert_eq!(a.word(&c, 0o100 + k), r.machine().amem[0o100 + k], "read {k}");
        }
    }
    assert!(parked >= 20, "{parked} reads parked");
    eprintln!(
        "{parked} reads of the microsecond clock parked alike, board and rtl both at {} ns",
        r.ns()
    );
}

/// **The timeout inhibit holds a cycle of the processor's that nothing
/// answers open on the board as on `rtl`, through the debug strobes that
/// set and lift it, and the cycle ends alike once it lifts.**  The
/// debuggee reads `764130`, between the I/O board's two blocks, with the
/// inhibit set through the cable; ten microseconds and more on, the cycle
/// is still open on both, no NXM flagged, both hung at the same PC.  The
/// strobe that lifts the inhibit is acknowledged in the same time on both
/// --- the debuggee's master clock is stopped by its `-HANG`, and the
/// strobes run without edges --- and from there the two stand at the same
/// PC at every instant they are brought to, the NXM flagged on both: the
/// cycle is given up on [`busint::TIMEOUT_NS`] after the first edge of the
/// timeout counter's 2 microsecond oscillator past the lift, which is why
/// the lift is made at two phases of it.
#[test]
fn chip_and_rtl_hold_an_unanswered_cycle_under_the_timeout_inhibit_alike() {
    use microcode::*;
    use muir::busint::{self, DEBUG_MODIFIER, debug_modifier};
    use muir::clock::Clock;
    use muir::engine::Engine;
    use muir::isa::Insn;
    use muir::machine::bus_error;
    use muir::part::Level;

    let n = netlist::parse(NETLIST).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    for phase in [0u64, 1_500] {
        let mut m = muir::machine::Machine::new();
        // 760100: on no board.  (764130, which the model's I/O board leaves
        // unanswered, the netlist board answers.)
        let phys = busint::unibus_physical(0o760100);
        m.l1_map[0] = 0;
        m.l2_map[0] = (1 << 23) | (1 << 22) | (phys >> 8);
        m.amem[3] = 0o123456;
        m.mmem[1] = phys & 0xff;
        let mut prom = vec![filler(); 512];
        // Fillers first: time for the inhibit to be set through the cable.
        let read_at = 30;
        prom[read_at] = Insn::new(ALU | SETM | m_src(1) | a_src(3) | START_READ);
        prom[read_at + 21] = Insn::new(ALU | SETM | SRC_MD | a_src(3) | a_dest(0o101));
        m.load_prom(&prom);
        let (mut c, mut clk, mut far, mut r) = same_program(&n, &m);
        let clk0 = cpu_clock(&n);
        let cable = DebuggerOnCable::new(&bus_n);
        let net = |name: &str| net_named(&bus_n, name);
        let (msyn, memrq, nxm) = (net("-UB MSYN"), net("-MEMRQ"), net("UB NXM ERROR"));
        // Through a hang that never ends the generator never comes round, so
        // the board is advanced event by event to an instant.
        let advance =
            |c: &mut Chip, far: &mut FarEnd, clk: &mut muir::clock::Behavioural, until: u64| {
                let mut n = 0;
                while clk.time_ns() < until {
                    far.tick_with(c, clk);
                    n += 1;
                    assert!(n < 2_000_000, "the boards spin at {} ns", clk.time_ns());
                }
            };
        const HOLD_NS: u64 = busint::UNIBUS_STROBE_NS;
        let strobe = |c: &mut Chip,
                      far: &mut FarEnd,
                      clk: &mut muir::clock::Behavioural,
                      r: &mut muir::rtl::Rtl,
                      dbd: u16|
         -> u64 {
            for _ in 0..100 {
                if far.board.net(cable.master) == Level::Low && !r.debug_busy() {
                    break;
                }
                let t = clk.time_ns() + 220;
                advance(c, far, clk, t);
            }
            // To a boundary, unless the ring is held by the hang.
            while clk.phase_ns() != 0 && clk.next_at(c.clock_inputs()).is_some() {
                far.tick_with(c, clk);
            }
            let (cat, cack, _, _) =
                cable.request(c, far, clk, DEBUG_MODIFIER, true, dbd, HOLD_NS, false);
            let rat = r.ns().max(cat);
            r.debug_request(
                rat,
                busint::DebugRequest { strobe: DEBUG_MODIFIER, write: true, dbd, hold_ns: HOLD_NS },
            );
            while r.debug_ack().is_none() {
                r.step().unwrap();
                assert!(r.ns() < rat + 20_000, "rtl did not acknowledge the strobe");
            }
            let (rack, _) = r.debug_ack().unwrap();
            assert_eq!(rack - rat, cack - cat, "the strobe's acknowledgement delay");
            cack - cat
        };
        for _ in 0..8 {
            generator_cycle(&mut c, &mut far, &mut clk, clk0);
        }
        meet(&n, &mut c, &mut far, &mut clk, &mut r, clk0);
        strobe(&mut c, &mut far, &mut clk, &mut r, debug_modifier::TIMEOUT_INHIBIT);
        meet(&n, &mut c, &mut far, &mut clk, &mut r, clk0);
        assert!((c.bus(&n, "PC", 14) as usize) < read_at, "the inhibit is set before the read");

        // Past the read and two timeouts on: the cycle is still open on both.
        let until = clk.time_ns() + read_at as u64 * 220 + 2 * busint::TIMEOUT_NS;
        advance(&mut c, &mut far, &mut clk, until);
        while r.ns() < clk.time_ns() {
            r.step().unwrap();
        }
        assert_eq!(far.board.net(msyn), Level::Low, "the board's cycle is out on its Unibus");
        assert_eq!(far.board.net(memrq), Level::Low, "and the processor's request stands");
        assert!(r.busint().busy(), "and rtl's is open");
        assert_eq!(far.board.net(nxm), Level::Low, "no NXM on the board");
        assert_eq!(r.machine().bus_error, 0, "nor on rtl");
        let hung_at = c.bus(&n, "PC", 14);
        assert_eq!(hung_at as u64, r.pc() as u64, "both hung at the same PC");
        eprintln!("hung at PC {hung_at:o} on both, {} ns on", clk.time_ns());

        // Lift the inhibit: the cycle is given up on and both run on together.
        let t = clk.time_ns() + phase;
        advance(&mut c, &mut far, &mut clk, t);
        let delay = strobe(&mut c, &mut far, &mut clk, &mut r, 0);
        let lifted = clk.time_ns();
        eprintln!("the lifting strobe acknowledged {delay} ns on, at {lifted} ns");
        meet(&n, &mut c, &mut far, &mut clk, &mut r, clk0);
        for _ in 0..120 {
            generator_cycle(&mut c, &mut far, &mut clk, clk0);
            meet(&n, &mut c, &mut far, &mut clk, &mut r, clk0);
            assert_eq!(c.bus(&n, "PC", 14) as u64, r.pc() as u64, "the PC at {} ns", r.ns());
        }
        assert!(c.bus(&n, "PC", 14) > hung_at, "the board ran on");
        assert_eq!(far.board.net(nxm), Level::High, "the NXM flagged on the board");
        assert_eq!(r.machine().bus_error, bus_error::UNIBUS_NXM, "and on rtl");
        assert_eq!(far.board.net(msyn), Level::High, "the board's cycle over");
        assert_eq!(far.board.net(memrq), Level::High, "the request answered");
        assert!(!r.busint().busy(), "and rtl's over");
        eprintln!(
            "given up on and run on alike from the lift at {lifted} ns; both at PC {:o} at {} ns",
            r.pc(),
            r.ns()
        );
    }
}

/// **Both cables between two netlist machines: each reads the other's
/// PC.**  The two boards are cabled both ways, each interface's DBGOUT to
/// the other's DBGIN; A reads B's PC through its DBGOUT while B runs, and
/// B then reads A's three times through its own while A runs on among its
/// fillers.  Every request is acknowledged, no cycle of either times out,
/// and both words are PCs in the other's PROM --- `rtl`'s
/// `both_machines_read_each_others_pc_over_the_two_cables`, on the boards.
#[test]
fn two_chip_machines_read_each_others_pc_over_two_cables() {
    use muir::cable::DebugCable;
    use muir::clock::Clock;
    use muir::lashup::DebugProgram;
    use muir::part::Level;
    use muir::spy;

    let n = netlist::parse(NETLIST).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    let unibus = |eadr: u8| spy::BASE + 2 * eadr as u32;
    let mut pa = DebugProgram::new();
    pa.dbg_read(unibus(spy::PC), 0o101);
    let mut pb = DebugProgram::new();
    pb.wait(120);
    for _ in 0..3 {
        pb.dbg_read(unibus(spy::PC), 0o101);
    }
    let (mut ca, mut clka, mut fa, _) = same_program(&n, &pa.finish());
    let (mut cb, mut clkb, mut fb, _) = same_program(&n, &pb.finish());
    let a_mem = Ram::new(&ca, &n, &MEMS[0]);
    let b_mem = Ram::new(&cb, &n, &MEMS[0]);
    let clk0 = cpu_clock(&n);
    let mut ab = DebugCable::new(&bus_n, &bus_n);
    let mut ba = DebugCable::new(&bus_n, &bus_n);
    let (ack, nxm) = (net_named(&bus_n, "DEBUG IN ACK"), net_named(&bus_n, "UB NXM ERROR"));

    let t = clka.time_ns().max(clkb.time_ns());
    bring_to(&mut ca, &mut fa, &mut clka, t);
    bring_to(&mut cb, &mut fb, &mut clkb, t);
    let exchange = |ab: &mut DebugCable,
                    ba: &mut DebugCable,
                    ca: &mut Chip,
                    fa: &mut FarEnd,
                    clka: &mut muir::clock::Behavioural,
                    cb: &mut Chip,
                    fb: &mut FarEnd,
                    clkb: &mut muir::clock::Behavioural,
                    t: u64| {
        for _ in 0..12 {
            let moved = ab.exchange(&mut fa.board, &mut fb.board);
            let moved = ba.exchange(&mut fb.board, &mut fa.board) || moved;
            if !moved {
                break;
            }
            bring_to(ca, fa, clka, t);
            bring_to(cb, fb, clkb, t);
        }
    };
    exchange(&mut ab, &mut ba, &mut ca, &mut fa, &mut clka, &mut cb, &mut fb, &mut clkb, t);

    // Acknowledgements on each DBGIN: three for A's read of B, nine for
    // B's three reads of A.
    let (mut acks_on_b, mut acks_on_a) = (0, 0);
    let (mut ack_b_was, mut ack_a_was) = (fb.board.net(ack), fa.board.net(ack));
    let t0 = t;
    while acks_on_b < 3 || acks_on_a < 9 {
        let ea = fa.next_event(&ca, &clka).expect("nothing on A's boards to wait for");
        let eb = fb.next_event(&cb, &clkb).expect("nothing on B's boards to wait for");
        let t = if ea <= eb {
            fa.tick_with(&mut ca, &mut clka);
            clka.time_ns()
        } else {
            fb.tick_with(&mut cb, &mut clkb);
            clkb.time_ns()
        };
        assert!(
            t - t0 < 200_000,
            "the programs did not finish: {acks_on_b} and {acks_on_a} acknowledgements"
        );
        exchange(&mut ab, &mut ba, &mut ca, &mut fa, &mut clka, &mut cb, &mut fb, &mut clkb, t);
        let (kb, ka) = (fb.board.net(ack), fa.board.net(ack));
        if ack_b_was == Level::Low && kb == Level::High {
            acks_on_b += 1;
        }
        if ack_a_was == Level::Low && ka == Level::High {
            acks_on_a += 1;
        }
        (ack_b_was, ack_a_was) = (kb, ka);
    }
    // The last word lands after the fillers behind the read.
    for _ in 0..40 {
        generator_cycle(&mut ca, &mut fa, &mut clka, clk0);
        generator_cycle(&mut cb, &mut fb, &mut clkb, clk0);
    }
    assert_eq!(fa.board.net(nxm), Level::Low, "no cycle of A's timed out");
    assert_eq!(fb.board.net(nxm), Level::Low, "nor of B's");
    let (pc_of_b, pc_of_a) = (a_mem.word(&ca, 0o101), b_mem.word(&cb, 0o101));
    eprintln!(
        "A read B's PC as {pc_of_b:o}; B read A's PC as {pc_of_a:o}; done at {} ns",
        clka.time_ns()
    );
    assert!(pc_of_b > 0 && pc_of_b < 0o200, "B's PC, in its wait: {pc_of_b:o}");
    assert!(
        pc_of_a > 0o102 && pc_of_a < 0o1000,
        "A's PC, past its program among the fillers: {pc_of_a:o}"
    );
}

/// **A Unibus cycle of the processor's that nothing answers is given up on
/// and the processor runs on, alike on both engines.**  The debuggee reads
/// `760100`, an address on no board; the NXM comes [`busint::TIMEOUT_NS`]
/// after the grant on both --- on the board, `NXM TIMEOUT` that long after
/// `INT BUSY` started the counter, and on `rtl` at the same instant, its
/// cycle answered by its timeout --- and from there the two stand at the
/// same PC at every generator boundary for a hundred cycles more: the
/// restart after a Unibus timeout, which the boot never makes (its probe
/// that times out is an Xbus cycle), held to the board.
#[test]
fn chip_and_rtl_run_on_alike_after_a_unibus_cycle_nothing_answers() {
    use microcode::*;
    use muir::busint;
    use muir::clock::Clock;
    use muir::engine::Engine;
    use muir::isa::Insn;
    use muir::machine::bus_error;
    use muir::part::Level;

    let n = netlist::parse(NETLIST).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    let mut m = muir::machine::Machine::new();
    let phys = busint::unibus_physical(0o760100);
    m.l1_map[0] = 0;
    m.l2_map[0] = (1 << 23) | (1 << 22) | (phys >> 8);
    m.amem[3] = 0o123456;
    m.mmem[1] = phys & 0xff;
    let mut prom = vec![filler(); 512];
    prom[10] = Insn::new(ALU | SETM | m_src(1) | a_src(3) | START_READ);
    prom[31] = Insn::new(ALU | SETM | SRC_MD | a_src(3) | a_dest(0o101));
    m.load_prom(&prom);
    let (mut c, mut clk, mut far, mut r) = same_program(&n, &m);
    // The interface's timeout counter on its own wires: `INT BUSY` up from
    // the grant, `NXM TIMEOUT` up when the REQTIM PROM says, `UB NXM
    // ERROR` registered from it.
    let (int_busy, nxm_timeout, nxm) = (
        net_named(&bus_n, "INT BUSY"),
        net_named(&bus_n, "NXM TIMEOUT"),
        net_named(&bus_n, "UB NXM ERROR"),
    );
    let sample =
        |far: &FarEnd| (far.board.net(int_busy), far.board.net(nxm_timeout), far.board.net(nxm));
    let mut was = sample(&far);
    let (mut granted_at, mut timed_out_at, mut nxm_at): (Option<u64>, Option<u64>, Option<u64>) =
        (None, None, None);
    // When `rtl` gives the cycle up: its answer is its timeout.
    let mut rtl_timed_out_at = None;
    for _ in 0..120 {
        let from = clk.time_ns();
        loop {
            far.tick_with(&mut c, &mut clk);
            let now = clk.time_ns();
            let is = sample(&far);
            // `INT BUSY` is an open-collector net, the 74S02O, 74S08O and
            // 74S10O at REQTIM 0A05, 0B16 and 0C13: up is the pull-up, `Z`,
            // which `Level::read` reads as a one.
            if is.0.read() == Some(true) && was.0.read() != Some(true) && timed_out_at.is_none() {
                granted_at = Some(now);
            }
            if is.1 == Level::High && was.1 != Level::High {
                timed_out_at.get_or_insert(now);
            }
            if is.2 != was.2 {
                eprintln!("board {now} ns: UB NXM ERROR -> {:?}", is.2);
                nxm_at.get_or_insert(now);
            }
            was = is;
            if clk.phase_ns() == 0 && now > from {
                break;
            }
            assert!(now - from < HANG_BOUND_NS);
        }
        let t = clk.time_ns();
        while r.ns() < t {
            let before = r.ns();
            r.step().unwrap();
            if rtl_timed_out_at.is_none() {
                rtl_timed_out_at = r.busint().answered_at();
            }
            if r.ns() - before > 300 {
                eprintln!("  rtl step {before} -> {} ns PC {:o}", r.ns(), r.pc());
            }
        }
        assert_eq!(
            r.ns(),
            t,
            "apart: board {t} ns PC {:o}, rtl {} ns PC {:o}",
            c.bus(&n, "PC", 14),
            r.ns(),
            r.pc()
        );
        assert_eq!(c.bus(&n, "PC", 14) as u64, r.pc() as u64, "PC at {t} ns");
    }
    eprintln!(
        "ended at board {} ns PC {:o}, rtl {} ns PC {:o}",
        clk.time_ns(),
        c.bus(&n, "PC", 14),
        r.ns(),
        r.pc()
    );
    let granted = granted_at.expect("the read was never granted on the board");
    let timed_out = timed_out_at.expect("NXM TIMEOUT never rose on the board");
    eprintln!(
        "the board granted the read at {granted} ns and gave it up at {timed_out} ns, UB NXM ERROR \
         at {nxm_at:?} ns; rtl gave it up at {rtl_timed_out_at:?} ns"
    );
    assert_eq!(
        timed_out - granted,
        busint::TIMEOUT_NS,
        "NXM TIMEOUT comes TIMEOUT_NS after INT BUSY started the counter at the grant: the REQTIM \
         PROM's first table, count 5 of its 2 microsecond intervals"
    );
    assert_eq!(
        rtl_timed_out_at,
        Some(timed_out),
        "rtl gives the cycle up at the board's instant, TIMEOUT_NS after its own grant"
    );
    assert!(
        nxm_at == Some(timed_out),
        "UB NXM ERROR, the 74276 at REQERR 0B02 clocked by -NXM TIMEOUT, registers as the \
         timeout comes: {nxm_at:?}"
    );
    assert_eq!(far.board.net(nxm), Level::High, "UB NXM ERROR stands on the board");
    assert_eq!(r.machine().bus_error, bus_error::UNIBUS_NXM, "and rtl flags the timeout");
}

/// **The netlist board is the debuggee at the end of the cable over TCP, and
/// answers CC's first operations as `rtl` does.**  The debugger is an `rtl`
/// machine in another thread, connected over the loopback and running its
/// end of the protocol ([`muir::lashup::Remote`]) on CC's first five
/// operations from its boot PROM: halt the debuggee, read its PC, step it,
/// read again.  The debuggee is the board's DBGIN with the processor and a
/// bare backplane behind it ([`muir::cable::DebugIn`]), each request going
/// on the connector at the instant the debugger's DBGOUT put it there, the
/// acknowledgement and the word coming back as of the instant the board
/// raised it.  The same debugger against `rtl` in process is the reference:
/// the PC read over the wire from the board is the PC read in process,
/// the step lands the same, and the board stands where `rtl` stands.
#[test]
fn a_chip_debuggee_answers_an_rtl_debugger_over_tcp() {
    use microcode::filler;
    use muir::cable::DebugIn;
    use muir::engine::Engine;
    use muir::lashup::{CableEnd, DebugProgram, Lashup, Remote};
    use muir::machine::Machine;
    use muir::rtl::Rtl;
    use muir::spy;
    use std::net::{TcpListener, TcpStream};

    fn unibus(eadr: u8) -> u32 {
        spy::BASE + 2 * eadr as u32
    }
    // CC's first five operations on a debuggee, as a debugger's microcode.
    let debugger = || {
        let mut a = DebugProgram::new();
        a.dbg_write(unibus(spy::CLK), 0);
        a.dbg_read(unibus(spy::PC), 0o101);
        a.dbg_write(unibus(spy::CLK), 2);
        a.dbg_write(unibus(spy::CLK), 0);
        a.dbg_read(unibus(spy::PC), 0o102);
        let mut r = Rtl::new(a.finish());
        r.boot();
        r
    };
    // The debuggee runs fillers: the PC climbs one a microcycle and nothing
    // touches the buses.
    let mut m = Machine::new();
    m.amem[3] = 0o123456;
    m.load_prom(&vec![filler(); 512]);
    let n = netlist::parse(NETLIST).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    let mem_n = netlist::parse(CADRM).unwrap();
    let bare = FarEnd::new(&n, &bus_n, &mem_n, Boards::default(), 0, m.clone());
    let (c, clk, far, r) = same_program_on(&n, &m, bare);
    const NS: u64 = 120_000;

    // The reference: the same debugger against `rtl`, in process.
    let mut reference = Lashup::new(debugger(), r);
    reference.run_until(NS).unwrap();

    // Over the wire: the debugger on its own thread, connecting; the board
    // here, listening.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let near = std::thread::spawn(move || {
        let stream = TcpStream::connect(addr).unwrap();
        stream.set_nodelay(true).unwrap();
        let reader = stream.try_clone().unwrap();
        let mut end = Remote::debugger(debugger(), reader, stream);
        end.run_until(NS).unwrap();
        end.machine
    });
    let (stream, _) = listener.accept().unwrap();
    stream.set_nodelay(true).unwrap();
    let reader = stream.try_clone().unwrap();
    let mut end = Remote::debuggee(DebugIn::new(&bus_n, c, clk, far), reader, stream);
    end.run_until(NS).unwrap();
    let a = near.join().unwrap();
    let board = end.machine;

    let (am, rm) = (a.machine(), reference.debugger.machine());
    let pc_on_the_board = board.cpu.bus(&n, "PC", 14) as u32;
    eprintln!(
        "over TCP the board's PC read {:o}, stepped to {:o}, standing at {:o}; in process rtl read {:o} and {:o}; {} microcycles on the board, {} debug cycles",
        am.amem[0o101],
        am.amem[0o102],
        pc_on_the_board,
        rm.amem[0o101],
        rm.amem[0o102],
        board.microcycles,
        board.debug_cycles
    );
    assert_eq!(am.bus_error, 0, "every cycle of the debugger's was answered");
    assert_eq!(a.bus_cycles(), 15);
    assert_eq!(board.debug_cycles, 5, "CC's five operations reached the board");
    assert_eq!(
        am.amem[0o101], rm.amem[0o101],
        "the PC read from the board is the PC read from rtl"
    );
    assert_eq!(am.amem[0o102], rm.amem[0o102], "and after the step");
    assert_eq!(am.amem[0o102], am.amem[0o101] + 1, "one step");
    assert_eq!(pc_on_the_board, am.amem[0o102], "the board stands where the debugger last read it");
    assert_eq!(pc_on_the_board as u16, reference.debuggee.pc(), "and where rtl stands");
    assert_eq!(board.debug_ack(), reference.debuggee.debug_ack(), "the last acknowledgement alike");
}

/// **`HALT-CONS` halts the netlist as it halts `rtl`, under `ERRSTOP`.**
/// The program writes `ERRSTOP` into the mode register through the Unibus
/// map --- the write `chip_and_rtl_reboot_on_the_mode_registers_boot_bit`
/// makes, with bit 2 for bit 7 --- and runs on into an instruction carrying
/// misc function 1. On the board `-FUNCT1` off the 74S139 at SOURCE 3D05
/// crosses to the ICMEM board as `-HALT` (`Netlist::EXPLICIT_ALIASES` joins
/// the two boards' names for the wire), the 74S374 at OLORD2 1A05 registers
/// it as `HALTED`, the 74S133 at 1A02 makes `ERR` of it, and `MACHRUN`
/// drops. Both machines stop at the same PC with `ERR` up, and stay there.
#[test]
fn chip_and_rtl_halt_on_halt_cons_alike() {
    use microcode::*;
    use muir::clock::Clock;
    use muir::engine::Engine;
    use muir::isa::Insn;
    use muir::part::Level;
    use muir::spy;

    let n = netlist::parse(NETLIST).unwrap();
    // `ERRSTOP`, bit 2 of the mode register: `ir.bits`, "4 ERRSTOP".
    let mut m = writes_a_diagnostic_register(spy::MODE, 1 << 2);
    // `HALT-CONS`, `1_10.`, well after the write has landed.
    m.prom[60] = Insn::new(filler().raw() | 1 << 10);
    let (mut c, mut clk, mut far, mut r) = same_program(&n, &m);
    let err = n.by_name_id("ERR").unwrap();
    let mut pcs = Vec::new();
    for k in 0..140 {
        let from = clk.time_ns();
        loop {
            far.tick_with(&mut c, &mut clk);
            if clk.phase_ns() == 0 && clk.time_ns() > from {
                break;
            }
            assert!(clk.time_ns() - from < 60_000);
        }
        let t = clk.time_ns();
        while r.ns() < t {
            r.step().unwrap();
        }
        assert_eq!(r.ns(), t, "in step at microcycle {k}");
        let (pc, rtl_pc) = (c.bus(&n, "PC", 14) as u64, r.pc() as u64);
        assert_eq!(pc, rtl_pc, "PC at microcycle {k}, {t} ns");
        pcs.push(pc);
    }
    // Halted: the PC stood still for the last forty microcycles, just past
    // the `HALT-CONS` at 60, with `ERR` up on both. The 74S133 at 1A02 is
    // the open-collector `74S133O`, so `ERR` up on the board is the output
    // let go and the pull-up, not a driven high.
    let pc = pcs[139];
    eprintln!(
        "halted at PC {pc:o}, from microcycle {}",
        pcs.iter().position(|&p| p == pc).unwrap()
    );
    assert!((60..=63).contains(&pc), "halted just past the HALT-CONS at 60: PC {pc:o}");
    assert!(pcs[100..].iter().all(|&p| p == pc), "and stayed there: {:?}", &pcs[100..]);
    assert_ne!(c.net(err), Level::Low, "the board's ERR is up");
    assert_ne!(r.spy_read(spy::FLAG_1) & (1 << 10), 0, "rtl's ERR is up");
}

/// **The mask and dispatch PROMs hold MIT's own table.** `Chip` programs the
/// eight 5600s on MSKG4 and the 5610 on DSPCTL from a formula, because the
/// drawings place those nine parts without giving their contents and no dump
/// of them survives. The CADR manual prints the table itself, under "Output
/// of mask memories": thirty-two entries, left mask and right, at line 495 of
/// `mit/lmdoc/cadr.164`, which is AI Memo 528, *CADR*, by Knight, Moon,
/// Holloway and Steele. The words are carried here rather than as a data
/// file because a transcription of a table is not one of MIT's own files,
/// and `mit/` holds only those --- the manual itself is MIT's, and is there.
///
/// The bit assignment is the schematic's: 2D11 and 2D12 drive `MSK<31:24>`,
/// 2E11 and 2E12 `MSK<23:16>`, 2D16 and 2D17 `MSK<15:8>`, 2E16 and 2E17
/// `MSK<7:0>`, with the `...11`/`...16` parts addressed by `MSKL` and the
/// `...12`/`...17` parts by `MSKR`. DSPCTL 2F22 is weaker than the other
/// eight: the manual gives its rule in prose --- the length field masks all
/// but the low `k` bits --- rather than printing an image, and only its
/// first eight words are addressed, `IR<7:5>` being all that reaches it.
#[test]
fn the_mask_proms_hold_mits_own_table() {
    // MIT's table, as the manual prints it.
    const LEFT: [u32; 32] = [
        0x00000001, 0x00000003, 0x00000007, 0x0000000f, 0x0000001f, 0x0000003f, 0x0000007f,
        0x000000ff, 0x000001ff, 0x000003ff, 0x000007ff, 0x00000fff, 0x00001fff, 0x00003fff,
        0x00007fff, 0x0000ffff, 0x0001ffff, 0x0003ffff, 0x0007ffff, 0x000fffff, 0x001fffff,
        0x003fffff, 0x007fffff, 0x00ffffff, 0x01ffffff, 0x03ffffff, 0x07ffffff, 0x0fffffff,
        0x1fffffff, 0x3fffffff, 0x7fffffff, 0xffffffff,
    ];
    const RIGHT: [u32; 32] = [
        0xffffffff, 0xfffffffe, 0xfffffffc, 0xfffffff8, 0xfffffff0, 0xffffffe0, 0xffffffc0,
        0xffffff80, 0xffffff00, 0xfffffe00, 0xfffffc00, 0xfffff800, 0xfffff000, 0xffffe000,
        0xffffc000, 0xffff8000, 0xffff0000, 0xfffe0000, 0xfffc0000, 0xfff80000, 0xfff00000,
        0xffe00000, 0xffc00000, 0xff800000, 0xff000000, 0xfe000000, 0xfc000000, 0xf8000000,
        0xf0000000, 0xe0000000, 0xc0000000, 0x80000000,
    ];
    const LENGTH: [u8; 32] = [
        0x00, 0x01, 0x03, 0x07, 0x0f, 0x1f, 0x3f, 0x7f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ];
    /// Page, reference, the word it takes its byte from, and which byte.
    const PARTS: [(&str, &str, &[u32; 32], u32); 8] = [
        ("MSKG4", "2D11", &LEFT, 24),
        ("MSKG4", "2E11", &LEFT, 16),
        ("MSKG4", "2D16", &LEFT, 8),
        ("MSKG4", "2E16", &LEFT, 0),
        ("MSKG4", "2D12", &RIGHT, 24),
        ("MSKG4", "2E12", &RIGHT, 16),
        ("MSKG4", "2D17", &RIGHT, 8),
        ("MSKG4", "2E17", &RIGHT, 0),
    ];

    // The PROMs come back with the power; before it their cells are empty.
    let mut c = build();
    c.power_on();
    let mut checked = 0;
    for inst in &c.instances {
        let (base, _) = muir::part::strip(&inst.kind);
        if !matches!(base, "5600" | "5610") {
            continue;
        }
        let want: Vec<u8> = if inst.page == "DSPCTL" {
            LENGTH.to_vec()
        } else {
            let &(_, _, word, shift) = PARTS
                .iter()
                .find(|&&(p, r, _, _)| p == inst.page && r == inst.reference)
                .unwrap_or_else(|| panic!("{} is not in the manual's table", inst.name()));
            word.iter().map(|w| (w >> shift) as u8).collect()
        };
        assert_eq!(inst.state.cells, want, "{} does not hold the manual's table", inst.name());
        checked += 1;
    }
    assert_eq!(checked, 9, "eight mask PROMs and the dispatch one");
}
