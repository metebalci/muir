// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `--watch` and the prompt's `watch` on `chip`: the named nets recorded
//! over a range of microcycles at every instant the boards move, one line
//! on stderr per change.  What the flag refuses is `tests/cli.rs`; this is
//! what it records.  The boot PROM alone is run except where a disk read
//! is wanted, which needs the pack and says so when it is not there.

mod support;

use std::io::Write;
use std::process::Stdio;

use support::{Run, muir, text};

/// One line of the record: the time in nanoseconds, the microcycle, and
/// each net as `name=value`, in the order the spec named them.
struct Line {
    ns: u64,
    microcycle: u64,
    values: Vec<(String, String)>,
}

/// The record out of a run's output: every line prefixed `watch: `, which
/// is what the prefix is for.  A name may carry spaces --- `disk:NEW CCW`
/// --- so a value is what follows the last `=` of a word, and the name is
/// every word before it since the previous value.
fn record(t: &str) -> Vec<Line> {
    t.lines()
        .filter_map(|l| l.strip_prefix("watch: "))
        .map(|l| {
            let (when, rest) = l.split_once(": ").unwrap_or_else(|| panic!("no values: {l}"));
            let mut when = when.split(' ');
            let ns = when.next().unwrap().parse().unwrap_or_else(|_| panic!("no time: {l}"));
            assert_eq!(when.next(), Some("ns,"), "{l}");
            assert_eq!(when.next(), Some("microcycle"), "{l}");
            let microcycle = when.next().unwrap().parse().unwrap_or_else(|_| panic!("{l}"));
            let mut values = Vec::new();
            let mut name = String::new();
            for word in rest.split(' ') {
                match word.rsplit_once('=') {
                    Some((n, v)) => {
                        if !name.is_empty() {
                            name.push(' ');
                        }
                        name.push_str(n);
                        values.push((std::mem::take(&mut name), v.to_string()));
                    }
                    None => {
                        if !name.is_empty() {
                            name.push(' ');
                        }
                        name.push_str(word);
                    }
                }
            }
            assert!(name.is_empty(), "a name with no value: {l}");
            Line { ns, microcycle, values }
        })
        .collect()
}

/// Where the run said the machine was at each microcycle boundary it was
/// held at: the answer to `step`, `PC <pc> in the PROM; <n> microcycles
/// this run`, as (n, pc).
fn boundaries(t: &str) -> Vec<(u64, u64)> {
    t.lines()
        .filter(|l| l.starts_with("PC "))
        .map(|l| {
            let pc = u64::from_str_radix(l.split(' ').nth(1).unwrap(), 8).unwrap();
            let n = l.split(';').nth(1).unwrap().trim().split(' ').next().unwrap().parse().unwrap();
            (n, pc)
        })
        .collect()
}

/// **`PC/14` over microcycles 2 to 4 of a cold boot is recorded at the
/// values the same run reports at those boundaries, and nowhere else.**
///
/// The machine is stepped a microcycle at a time so that it says where it
/// is at every boundary; each `PC` line is read back and held against the
/// record rather than against a number written here.  The PC register is
/// clocked at the start of the microcycle, so the value the record shows
/// for microcycle *n* is the one the boundary *n* reports, and the boot
/// PROM's first instructions are straight-line, so it moves once a
/// microcycle: three lines, one for each microcycle in the range, their
/// times and microcycles in order.  The start of the range is printed
/// whether or not the PC moved on that very step, so a reader always has
/// the values the range began at.
#[test]
fn the_pc_is_recorded_over_the_range_at_the_values_the_run_reports() {
    let mut child = muir()
        .args(["--chip", "--no-auto-boot", "--watch", "2-4:PC/14", "--stop-after", "20"])
        .stdin(Stdio::piped())
        .start();
    let mut stdin = child.stdin();
    write!(stdin, "boot\n{}quit\n", "step 1\n".repeat(6)).unwrap();
    drop(stdin);
    let out = child.wait();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    let at = boundaries(&t);
    assert_eq!(at.len(), 7, "the boot and six steps each say where the machine is:\n{t}");
    let lines = record(&t);
    assert!(!lines.is_empty(), "nothing was recorded:\n{t}");
    assert!(
        lines.iter().all(|l| (2..=4).contains(&l.microcycle)),
        "every line is in the range:\n{t}"
    );
    assert!(lines.windows(2).all(|w| w[0].ns < w[1].ns), "time runs forward:\n{t}");
    assert!(
        lines.windows(2).all(|w| w[0].microcycle <= w[1].microcycle),
        "and so do the microcycles:\n{t}"
    );
    let recorded: Vec<(u64, u64)> = lines
        .iter()
        .map(|l| {
            assert_eq!(l.values.len(), 1, "one net was watched:\n{t}");
            assert_eq!(l.values[0].0, "PC/14", "named as the spec named it:\n{t}");
            (l.microcycle, u64::from_str_radix(&l.values[0].1, 8).unwrap())
        })
        .collect();
    let reported: Vec<(u64, u64)> =
        at.iter().copied().filter(|&(n, _)| (2..=4).contains(&n)).collect();
    assert_eq!(recorded, reported, "the record is what the boundaries reported:\n{t}");
    assert!(t.contains("quit at PC"), "the run ended by quit:\n{t}");
}

/// **`watch <n> <nets>` at the prompt records the next n microcycles from
/// where the machine stands**, so a run that has been going for hours can
/// be told to record without being started again; and a second `watch`
/// replaces the first, as does one over a `--watch` still to come.
#[test]
fn the_prompts_watch_records_the_next_microcycles_from_here() {
    let mut child = muir()
        .args(["--chip", "--no-auto-boot", "--stop-after", "30"])
        .stdin(Stdio::piped())
        .start();
    let mut stdin = child.stdin();
    write!(stdin, "boot\nstep 2\nwatch 3 PC/14,cpu:MEMRQ\nstep 6\nquit\n").unwrap();
    drop(stdin);
    let out = child.wait();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(
        t.contains("recording PC/14, cpu:MEMRQ over microcycles 2 to 4"),
        "the prompt says what it will record and when:\n{t}"
    );
    let lines = record(&t);
    let cycles: std::collections::BTreeSet<u64> = lines.iter().map(|l| l.microcycle).collect();
    assert_eq!(
        cycles.into_iter().collect::<Vec<_>>(),
        [2, 3, 4],
        "the three microcycles after the second, and no other:\n{t}"
    );
    for l in &lines {
        let names: Vec<&str> = l.values.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["PC/14", "cpu:MEMRQ"], "every value on every line:\n{t}");
        assert!(
            matches!(l.values[1].1.as_str(), "High" | "Low"),
            "a net reads as net prints it:\n{t}"
        );
    }
    assert!(t.contains("quit at PC"), "{t}");
}

/// **`watch` is chip's, as `net` is**: on the other engines the prompt
/// says so and records nothing.
#[test]
fn watch_is_the_chip_engines_at_the_prompt() {
    let mut child =
        muir().args(["--micro", "--stop-after", "1000000000"]).stdin(Stdio::piped()).start();
    let mut stdin = child.stdin();
    write!(stdin, "watch 5 PC/14\nquit\n").unwrap();
    drop(stdin);
    let out = child.wait();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(t.contains("prompt: nets are the chip engine's"), "{t}");
    assert!(!t.contains("watch: "), "nothing recorded:\n{t}");
}

/// **A pulse shorter than a microcycle is in the record**, which is what
/// the record is taken at the chip's own step for: `TPTSE` on the
/// processor is cleared at `-TPR5` and set at `-TPR25`, so it is low for
/// twenty nanoseconds at the start of every microcycle
/// ([`muir::clock`]'s `TSE_OFF_NS` and `TSE_ON_NS`), and a sample once a
/// microcycle --- 220 ns at the boot's extra-slow speed --- would never
/// land inside it.  Here it is, twice: down and up twenty nanoseconds
/// apart, inside each of the two microcycles watched, and nothing else,
/// the boot PROM having nothing that changes `TPTSE` any other way.
#[test]
fn a_pulse_inside_a_microcycle_is_recorded() {
    let out = muir().args(["--chip", "--watch", "2-3:cpu:TPTSE", "--stop-after", "10"]).run();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    let lines = record(&t);
    let levels: Vec<(u64, u64, &str)> =
        lines.iter().map(|l| (l.microcycle, l.ns, l.values[0].1.as_str())).collect();
    // The range begins at the start of microcycle 2, which is `-TPR0` and
    // before `-TPR5`, so the first line is the level the range began at,
    // high; then the drop and the rise in each microcycle.
    assert_eq!(levels.len(), 5, "the start of the range and two pulses:\n{t}");
    assert_eq!(levels[0].2, "High", "{t}");
    for (k, cycle) in [(1, 2), (3, 3)] {
        let (down, up) = (levels[k], levels[k + 1]);
        assert_eq!((down.0, down.2), (cycle, "Low"), "down in microcycle {cycle}:\n{t}");
        assert_eq!((up.0, up.2), (cycle, "High"), "and up in the same one:\n{t}");
        assert_eq!(up.1 - down.1, 20, "twenty nanoseconds later:\n{t}");
    }
}

/// **A `disk:` net is a net of the netlist disk controller, and the drive's
/// sector pulses reach it from the first microcycle.**
///
/// The net the flag was made for is `NEW CCW`, over the boot PROM's first
/// disk read; and that read is out of a test's reach. It is high from
/// reset --- the LS74 at DCCCW 0E20 comes up set --- and first falls at
/// the first fetch of a CCW, which is the read of the label at block 0
/// after `DISK-RECALIBRATE` at 0o541; and the PROM reaches 0o541 after
/// 416,736 executed instructions (`tests/boot.rs`), most of them the
/// 65,536 turns of `CLEAR-LEVEL-2-MAP`, which on `chip` is five minutes
/// at the test profile's 1,600 microcycles a second. So what is watched
/// instead is a disk net that moves with nothing asked of the board at
/// all: `-UNIT.0.SECTOR^`, the sector pulse off the drive's cable at the
/// controller's unit 0 port. A pack in the drive is a pack turning, and
/// the pulse comes once a sector, [`muir::disk_unit::SECTOR_NS`] apart
/// and [`muir::disk_unit::SECTOR_PULSE_NS`] wide --- Century Data's own
/// figures, 1.24 us --- from the first revolution on.  `TRIDENT.READY/`
/// is watched beside it, for a second `disk:` name resolving on the same
/// board.
///
/// The drive wants a pack in it, so this needs `vendor/` and skips
/// without it.
#[test]
fn the_drives_sector_pulses_reach_the_disk_controller_from_the_start() {
    use muir::disk_unit::{SECTOR_NS, SECTOR_PULSE_NS};
    let Some(pack) = support::pack_100() else { return };
    // Two sector pulses' worth of microcycles at the boot's 220 ns, and
    // some over.
    const TO: u64 = 10_000;
    let child = muir()
        .args(["--chip", "--disk-controller", "netlist", "--tv", "model", "--io-board", "model"])
        .arg("--disk-pack")
        .arg(format!("{},ro", pack.display()))
        .args(["--watch", &format!("0-{TO}:disk:-UNIT.0.SECTOR^,disk:TRIDENT.READY/")])
        .args(["--stop-after", &(TO + 1).to_string()])
        .start();
    let out = child.wait();
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    let lines = record(&t);
    assert!(!lines.is_empty(), "nothing was recorded:\n{t}");
    assert!(lines.iter().all(|l| l.microcycle <= TO), "every line is in the range:\n{t}");
    for l in &lines {
        let names: Vec<&str> = l.values.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["disk:-UNIT.0.SECTOR^", "disk:TRIDENT.READY/"], "{t}");
    }
    fn sector(l: &Line) -> &str {
        l.values[0].1.as_str()
    }
    // The pulses: each a rise and, `SECTOR_PULSE_NS` later, a fall.
    let rises: Vec<&Line> = lines
        .windows(2)
        .filter(|w| sector(&w[0]) == "Low" && sector(&w[1]) == "High")
        .map(|w| &w[1])
        .collect();
    assert!(rises.len() >= 2, "two sector pulses in {TO} microcycles:\n{t}");
    for rise in &rises {
        let fall = lines
            .iter()
            .find(|l| l.ns > rise.ns && sector(l) == "Low")
            .unwrap_or_else(|| panic!("the pulse at {} ns never ended:\n{t}", rise.ns));
        assert_eq!(fall.ns - rise.ns, SECTOR_PULSE_NS, "1.24 us wide, at {} ns:\n{t}", rise.ns);
        assert!(fall.microcycle > rise.microcycle, "several microcycles long:\n{t}");
    }
    assert_eq!(rises[1].ns - rises[0].ns, SECTOR_NS, "one a sector:\n{t}");
    eprintln!(
        "sector pulses at {} and {} ns, microcycles {} and {}",
        rises[0].ns, rises[1].ns, rises[0].microcycle, rises[1].microcycle
    );
}
