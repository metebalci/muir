// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! System 2000 on QUUX revision 11 (contracts Q11 and Q13): the boot
//! PROM's reset devices and timer 0's period (M9), the band ticking and
//! saying it is System 2000 on microcode 2000 (M10), and a reboot that
//! resets timers 1 and 2 and the file device (M11, and the Q9 amendment's
//! R3). Microcode 2000 reaches timer 0 through the register page, writing
//! its period at `RESET-MACHINE` and turning it on at `BEG06`, and writes
//! reset devices at every start of the microcode (`uc-cold-disk.lisp` in
//! the release's sources).
//!
//! The band is QUUX's release, `release-2000`, which
//! `tools/fetch-system-for-quux.sh` fetches into the gitignored `vendor/`
//! and pins by digest (`tests/system_2000.rs`); without it the tests skip
//! and say so. M9, M10 and M11 run on muir's built-in PROM,
//! `data/quux-promh-2000.mcr`, PROM 2000, the release's `release-2000-promh.mcr`
//! byte for byte (with contract Q11's steps 2, 5 and 6). The runs on the PROM before
//! Q11, M11's discriminating run and M12, booted revision 10's register
//! page and cannot run on revision 11; their figures stay in
//! `docs/quux.md` (Interval timers).

use std::path::{Path, PathBuf};

use muir::engine::Engine;
use muir::isa::Insn;
use muir::isa::asm::{ALWAYS, JUMP, N, target};
use muir::machine::{IntervalTimer, Machine, Timers};
use muir::micro::Micro;
use muir::spy;
use muir::sym::{self, Space};
use muir::terminal::keyboard::Keyboard;
use muir::tv::Board;

mod support;

/// LISPM-1 and OZ, as the release's `site/hosts.text` gives them.
const CHAOS: (u16, u16) = (0o177201, 0o177200);

const PAGE: u32 = 0o17777400;
const fn control(k: usize) -> u32 {
    PAGE + 0o110 + 2 * k as u32
}
const fn period(k: usize) -> u32 {
    PAGE + 0o111 + 2 * k as u32
}
const RESET_DEVICES: u32 = PAGE + 0o104;
const FDEV_CONTROL: u32 = PAGE + 0o160;
const FDEV_STATUS: u32 = PAGE + 0o161;

/// A copy of the release's disk, which the machine writes, and the served
/// tree, in a scratch directory.
fn band(name: &str) -> Option<(support::Scratch, PathBuf, PathBuf)> {
    support::quux_release_band(name)
}

/// The release's microcode's symbols, its `sys/ubin/ucadr.sym`.
fn symbols() -> sym::Symbols {
    let path = support::quux_release(&["sys", "ubin", "ucadr.sym"]).expect("the release");
    sym::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The band's microcode's address of `name`, from the release's
/// `ucadr.sym`.
fn ucadr(name: &str) -> u16 {
    symbols().address(Space::IMem, name).unwrap_or_else(|| panic!("{name} in ucadr.sym")) as u16
}

fn a_mem(name: &str) -> usize {
    symbols().address(Space::AMem, name).unwrap_or_else(|| panic!("{name} in ucadr.sym")) as usize
}

/// QUUX with `prom` at 36000, the disk on block-disk, the video controller at the
/// band's 1280 by 1024, and the file device serving `files` as HOST's `/`
/// and `root`'s `sys` and `site`, the tree's, as `/sys` and `/site`, where
/// the band's `SYS:` is (`site/sys.translations`).
fn quux(pack: &Path, prom: &[Insn], files: &Path, root: &Path) -> Machine {
    use muir::block_disk::{BLOCK_NS, BlockDisk};
    let mut m = Machine::new();
    m.load_prom(prom);
    let mut d = BlockDisk::new(BLOCK_NS);
    d.attach(muir::disk_image::Disk::open_rw(pack).expect("the pack"));
    m.block_disk = Some(d);
    m.geometry = muir::machine::Geometry::QUUX;
    m.tv.set_board(Board::Video);
    m.tv.set_video_size(1280, 1024);
    m.file_device.mounts.add(&files.display().to_string()).unwrap();
    for part in ["sys", "site"] {
        m.file_device.mounts.add(&format!("{part}={}", root.join(part).display())).unwrap();
    }
    m
}

fn listening(e: &impl Engine) -> bool {
    support::lit_rows(e, 84..130) > 400
}

/// Steps `e` until the listener has been drawn afresh --- the screen dark
/// first, then the listener --- within `limit` microcycles: how many it
/// took, or `None`, calling `each` on every microcycle.
fn to_a_new_listener(e: &mut Micro, limit: u64, mut each: impl FnMut(&Micro)) -> Option<u64> {
    let mut dark = !listening(e);
    let mut n = 0u64;
    while n < limit {
        for _ in 0..100_000 {
            e.step().expect("halted");
            each(e);
        }
        n += 100_000;
        if !dark {
            dark = !listening(e);
        } else if listening(e) {
            return Some(n);
        }
    }
    None
}

/// The console's way to a PC without `-RESET`: halt, a `JUMP` through the
/// debug IR --- loaded with `NOP11`, run with `IDEBUG` --- and run again.
fn jump_to(e: &mut Micro, pc: u16) {
    let clock = |e: &mut Micro, clk: u16| {
        e.spy_write(spy::CLK, clk);
        e.step().unwrap();
        e.step().unwrap();
        e.spy_write(spy::CLK, 0);
        e.step().unwrap();
        e.step().unwrap();
    };
    e.spy_write(spy::CLK, 0);
    e.step().unwrap();
    e.step().unwrap();
    let insn = JUMP | target(pc as u64) | ALWAYS | N;
    e.spy_write(spy::IR_LOW, insn as u16);
    e.spy_write(spy::IR_MED, (insn >> 16) as u16);
    e.spy_write(spy::IR_HIGH, (insn >> 32) as u16);
    clock(e, 0o16);
    clock(e, 0o12);
    e.spy_write(spy::CLK, 1);
    for _ in 0..8 {
        e.step().unwrap();
        if e.executed() == Some(pc) {
            return;
        }
    }
    panic!("the jump to {pc:o} never ran");
}

/// Runs `e` until `pc` executes, within `limit` microcycles: how many it
/// took.
fn until_executed(e: &mut Micro, pc: u16, limit: u64) -> u64 {
    for n in 0..limit {
        e.step().expect("halted");
        if e.executed() == Some(pc) {
            return n + 1;
        }
    }
    panic!("{pc:o} never ran in {limit} microcycles");
}

/// **M9, the new PROM**: from power-on, its writes recorded by address and
/// microcycle show word 104 with `<0>` set, then word 111 with 16,667, and
/// no other timer word, before its first block-disk command; at the
/// microcode's location 6 timer 0 is off, periodic, interrupt enable 0,
/// period 16,667, and timers 1 and 2 in their reset state. Its
/// microcycles to location 6 are recorded. From power-on every timer is in
/// its reset state before the PROM runs, so the last clause holds only that
/// the PROM turns nothing on; its reset of timers 1 and 2 is M11's.
#[test]
fn m9_the_prom_resets_the_devices_and_writes_timer_0_s_period() {
    let Some((_dir, pack, root)) = band("q11-m9") else { return };
    let mut m = quux(&pack, &muir::prom::quux_12_boot_prom(), &root, &root);
    m.register_log = Some(Vec::new());
    let mut e = Micro::new(m);
    e.boot();
    let n = until_executed(&mut e, 6, 20_000_000);
    eprintln!("location 6 after {n} microcycles on micro");
    let log = e.machine().register_log.clone().unwrap();
    let disk = log
        .iter()
        .position(|&(a, _, _)| (muir::block_disk::REGS..muir::block_disk::REGS + 4).contains(&a))
        .expect("a block-disk command");
    let timers: Vec<_> = log[..disk]
        .iter()
        .filter(|&&(a, _, _)| a == RESET_DEVICES || (control(0)..=period(2)).contains(&a))
        .map(|&(a, v, c)| (a - PAGE, v, c))
        .collect();
    eprintln!("before the first disk command: {timers:?}");
    assert_eq!(timers.len(), 2, "word 104 and word 111 alone: {timers:?}");
    assert_eq!((timers[0].0, timers[0].1 & 1), (0o104, 1), "reset devices first");
    assert_eq!((timers[1].0, timers[1].1), (0o111, Timers::TICK_PERIOD_US), "then the period");
    assert!(log[disk..].iter().all(|&(a, _, _)| !(control(0)..=period(2)).contains(&a)));
    let t = e.machine().timers.timer;
    assert_eq!(
        t[0],
        IntervalTimer { period_us: Timers::TICK_PERIOD_US, ..IntervalTimer::RESET },
        "timer 0 at location 6"
    );
    assert_eq!([t[1], t[2]], [IntervalTimer::RESET; 2], "timers 1 and 2 at location 6");
}

/// Types `text` a character at a time, 3 ms of simulated time after
/// each.
fn type_slow(e: &mut Micro, k: &mut Keyboard, text: &str) {
    for ch in text.chars() {
        support::type_at(e, k, &ch.to_string());
        run_ns(e, 3_000_000);
    }
}

fn run_ns(e: &mut Micro, ns: u64) {
    let t = e.machine().ns;
    while e.machine().ns < t + ns {
        e.step().unwrap();
    }
}

/// What the band says it is: logged in at the listener, it writes to the
/// file device, as `HOST://home//lispm//versions`, 3, its microcode's
/// version, its machine type and the herald's line, then the count
/// `(time)` moved over a `process-sleep` of 60; what it wrote, within 60 s
/// of simulated time. `(time)` is bits 31:14 of the microsecond clock
/// (muir-sys `sys/sys/qrand.lisp:1291-1318`, `%microsecond-clock-ldb`
/// `#o1622`), 1e6/16384 = 61.04 counts a second; the tick does not move it.
fn says_what_it_is(e: &mut Micro, k: &mut Keyboard, root: &Path) -> Option<String> {
    type_slow(e, k, "(login \"LISPM\" \"HOST\" t)\n");
    run_ns(e, 2_000_000_000);
    type_slow(
        e,
        k,
        "(let ((t0 (time))) (process-sleep 60.) (with-open-file (s \"HOST://home//lispm//versions\" \
         :direction :output) (format s \"~S ~S END~%\" (list (+ 1 2) %microcode-version-number \
         (si:machine-type) (si:system-version-info)) (time-difference (time) t0))))\n",
    );
    let file = root.join("home/lispm/versions");
    let t = e.machine().ns;
    while e.machine().ns < t + 60_000_000_000 {
        run_ns(e, 50_000_000);
        if let Ok(s) = std::fs::read_to_string(&file)
            && s.contains("END")
        {
            return Some(s);
        }
    }
    None
}

/// **M10, the band ticks on revision 11 and says it is System 2000 on
/// microcode 2000**, on the new PROM: it boots to its listener; there word
/// 110 reads 401, timer 0 on, periodic, under its interrupt enable (its
/// flag, `<1>`, masked, as a tick may be pending at the read), turned on at
/// `BEG06` through the register page; word 111 reads 16,667; and over 10 s
/// of simulated time after the listener `INTR-TICK` executes 600 times, give
/// or take one. Then, logged in, the band writes
/// `(3 2000 "QUUX" "Experimental System 2000, microcode 2000")` and a
/// `(time)` that moved over its second's sleep ([`says_what_it_is`]).
/// Whether a mouse move reaches the cursor is recorded. (The unused-codes
/// run's values are `tests/unused_codes.rs`'s.)
#[test]
fn m10_the_band_ticks_and_says_it_is_system_2000() {
    let Some((_dir, pack, root)) = band("q11-m10") else { return };
    let tick = ucadr("INTR-TICK");
    let mut e = Micro::new(quux(&pack, &muir::prom::quux_12_boot_prom(), &root, &root));
    e.boot();
    let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root.clone(), 400_000_000);
    eprintln!("listener after {ran} microcycles");
    let m = e.machine_mut();
    let (c0, p0) = (m.bus_read(control(0)), m.bus_read(period(0)));
    let t0 = e.machine().ns;
    let mut ticks = 0u32;
    while e.machine().ns < t0 + 10_000_000_000 {
        e.step().unwrap();
        ticks += (e.executed() == Some(tick)) as u32;
    }
    eprintln!("word 110 {c0:o}, word 111 {p0}; INTR-TICK ran {ticks} times in 10 s");
    assert_eq!(c0 & !2, 0o401, "word 110: timer 0 on, periodic, interrupt enable");
    assert_eq!(p0, Timers::TICK_PERIOD_US.into(), "word 111");
    assert!(ticks.abs_diff(600) <= 1, "INTR-TICK ran {ticks} times in 10 s");
    // The mouse, a record.
    use muir::quux_input::KeyboardMouse;
    let (ax, ay) = (a_mem("A-MOUSE-X"), a_mem("A-MOUSE-Y"));
    let at = |e: &Micro| (e.machine().amem[ax], e.machine().amem[ay]);
    let before = at(&e);
    e.machine_mut().quux_input.mouse_move(-40, -30);
    let t1 = e.machine().ns;
    while e.machine().ns < t1 + 100_000_000 {
        e.step().unwrap();
    }
    eprintln!("mouse {before:?} -> {:?}", at(&e));
    let said = says_what_it_is(&mut e, &mut Keyboard::new(), &root);
    eprintln!("the band says {said:?}");
    let said = said.expect("the band wrote what it is");
    let (what, moved) = said.trim_end().strip_suffix(" END").unwrap().rsplit_once(' ').unwrap();
    assert_eq!(what, r#"(3 2000 "QUUX" "Experimental System 2000, microcode 2000")"#);
    let moved: u32 = moved.parse().unwrap();
    assert!((60..90).contains(&moved), "(time) moved {moved} over a second's sleep");
}

/// Commands to queue on a machine, later.
type Queue = Box<dyn Fn(&mut Machine)>;

/// What M11 measures of one reboot.
#[derive(Debug)]
struct Reboot {
    /// Microcycles from 36000 to the listener, if it came within the limit.
    to_listener: Option<u64>,
    /// `INTR`'s executions over the same.
    intr: u64,
    /// At the microcode's location 6: words 112 and 114, and 161.
    at_6: [u32; 3],
    /// A queued command ran: a response written by location 6, or the
    /// folder the last one creates, by the end.
    ran: bool,
}

/// The file device's rings and buffers for M11, high in main memory.
const CMD_RING: u32 = 0o7000000;
const RESP_RING: u32 = 0o7000100;
const NAME: u32 = 0o7000200;
const BUF: u32 = 0o7100000;

/// **M11's reboot**: boot to the listener with `prom`; if `loaded`, halt,
/// turn timers 1 and 2 on under their interrupt enables (1 periodic at
/// 1,000 us, 2 one-shot at 5,000 us), let 6 ms pass so that both are up,
/// and leave the file device enabled with an OPEN done and three READs of
/// 64 KiB each and a CREATE-DIRECTORY queued, the first due 6.4 ms after
/// the PROM's write of word 102, its step 4, just before its reset
/// devices. Then the PC to 36000 without `-RESET`, and on to the listener,
/// within `limit` microcycles. The timers are turned on with the machine
/// halted because microcode 2000 turns off a timer 1 or 2 that interrupts
/// (`INTR-TIMER-1-STRAY` and `INTR-TIMER-2-STRAY` in `uc-interrupt.lisp`),
/// which a running band would do before the reboot.
fn reboot(
    pack: &Path,
    root: &Path,
    files: &Path,
    prom: &[Insn],
    loaded: bool,
    limit: u64,
) -> Reboot {
    let (intr, intr_at) = (ucadr("INTR"), 6u16);
    let mut e = Micro::new(quux(pack, prom, files, root));
    e.boot();
    let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root.to_path_buf(), 400_000_000);
    eprintln!("listener after {ran} microcycles");
    let post = |m: &mut Machine, k: u32, words: [u32; 8]| {
        let slot = (CMD_RING + 8 * (k % 4)) as usize;
        for (m, w) in m.main[slot..slot + 8].iter_mut().zip(words) {
            *m = w.into();
        }
        m.bus_write(PAGE + 0o164, (k + 1).into());
    };
    use muir::file_device::op;
    let mut queue: Option<Queue> = None;
    if loaded {
        // Halted, so that the band touches neither the timers nor the
        // device's memory.
        e.spy_write(spy::CLK, 0);
        e.step().unwrap();
        e.step().unwrap();
        let m = e.machine_mut();
        m.bus_write(period(1), 1_000);
        m.bus_write(control(1), 0o401);
        m.bus_write(period(2), 5_000);
        m.bus_write(control(2), 0o405);
        let t = m.ns;
        while e.machine().ns < t + 6_000_000 {
            e.step().unwrap();
        }
        let m = e.machine_mut();
        assert_eq!(m.bus_read(control(1)) & 3, 3, "timer 1 on and up");
        assert_eq!(m.bus_read(control(2)) & 3, 3, "timer 2 on and up");
        // The rings, the device enabled, and a file opened.
        m.main[CMD_RING as usize..NAME as usize + 8].fill(0);
        m.bus_write(PAGE + 0o162, CMD_RING.into());
        m.bus_write(PAGE + 0o163, 2);
        m.bus_write(PAGE + 0o166, RESP_RING.into());
        m.bus_write(PAGE + 0o167, 2);
        m.bus_write(FDEV_CONTROL, 1);
        m.main[NAME as usize] = u64::from(u32::from_le_bytes(*b"/big"));
        post(m, 0, [1 | op::OPEN << 16, 0, NAME, 4, 0, 0, 0, 0]);
        let t = m.ns;
        while e.machine().file_device.response_producer() == 0 {
            e.step().unwrap();
            assert!(e.machine().ns < t + 1_000_000, "the OPEN answered, the machine halted");
        }
        let m = e.machine_mut();
        let handle = support::low(m.main[RESP_RING as usize + 2]);
        assert_eq!(m.main[RESP_RING as usize] >> 16 & 0xff, 0, "the OPEN answered");
        m.bus_write(PAGE + 0o171, 1);
        // Queued at the PROM's step 4, below.
        queue = Some(Box::new(move |m: &mut Machine| {
            for k in 1..4 {
                post(m, k, [(1 + k) | (op::READ << 16), handle, 0, 0, BUF, 65_536, 0, 0]);
            }
            m.main[NAME as usize..NAME as usize + 2].copy_from_slice(&[
                u32::from_le_bytes(*b"/new").into(),
                u32::from_le_bytes(*b"dir\0").into(),
            ]);
            post(m, 4, [5 | op::CREATE_DIRECTORY << 16, 0, NAME, 7, 0, 0, 0, 0]);
        }));
    }
    e.machine_mut().main[RESP_RING as usize..RESP_RING as usize + 32].fill(0);
    e.machine_mut().register_log = Some(Vec::new());
    jump_to(&mut e, 0o36000);
    if let Some(queue) = queue {
        let mut n = 0;
        while !e.machine().register_log.as_ref().unwrap().iter().any(|w| w.0 == PAGE + 0o102) {
            e.step().unwrap();
            n += 1;
            assert!(n < 5_000_000, "the PROM never wrote word 102");
        }
        eprintln!("the PROM's step 4 after {n} microcycles; the commands queued");
        queue(e.machine_mut());
    }
    let mut intrs = (e.executed() == Some(intr)) as u64;
    let to_6 = {
        let mut n = 0u64;
        loop {
            e.step().unwrap();
            n += 1;
            intrs += (e.executed() == Some(intr)) as u64;
            if e.executed() == Some(intr_at) {
                break;
            }
            assert!(n < 50_000_000, "location 6 never ran");
        }
        n
    };
    let m = e.machine_mut();
    let at_6 =
        [m.bus_read(control(1)), m.bus_read(control(2)), m.bus_read(FDEV_STATUS)].map(support::low);
    let answered = m.main[RESP_RING as usize..RESP_RING as usize + 32].iter().any(|&w| w != 0);
    let to_listener = to_a_new_listener(&mut e, limit, |e| {
        intrs += (e.executed() == Some(intr)) as u64;
    })
    .map(|n| n + to_6);
    let created = files.join("newdir").exists();
    Reboot { to_listener, intr: intrs, at_6, ran: answered || created }
}

/// M11's pass criterion, against the baseline's figures.
fn m11_verdict(run: &Reboot, base: &Reboot) -> Result<(), String> {
    let mut why = Vec::new();
    let (Some(b), Some(r)) = (base.to_listener, run.to_listener) else {
        return Err(format!("no listener: {run:?} against {base:?}"));
    };
    if run.intr as f64 > 1.1 * base.intr as f64 + 10.0 {
        why.push(format!("INTR ran {} times against the baseline's {}", run.intr, base.intr));
    }
    if r as f64 > 1.05 * b as f64 {
        why.push(format!("{r} microcycles to the listener against the baseline's {b}"));
    }
    if run.at_6[0] & 1 != 0 || run.at_6[1] & 1 != 0 {
        why.push(format!("timers 1 and 2 at location 6: {:o} {:o}", run.at_6[0], run.at_6[1]));
    }
    if run.at_6[2] & 1 != 0 || run.at_6[2] >> 16 & 0xff != 0 {
        why.push(format!("161 at location 6: {:x}", run.at_6[2]));
    }
    if run.ran {
        why.push("a queued command ran".into());
    }
    if why.is_empty() { Ok(()) } else { Err(why.join("; ")) }
}

fn m11(name: &str, prom: &[Insn]) -> Option<(Reboot, Reboot)> {
    let (dir, pack, root) = band(&format!("{name}-base"))?;
    let files = dir.join("files");
    std::fs::create_dir_all(&files).unwrap();
    std::fs::write(files.join("big"), vec![5u8; 300_000]).unwrap();
    let base = reboot(&pack, &root, &files, prom, false, 600_000_000);
    eprintln!("baseline: {base:?}");
    let (dir, pack, root) = band(&format!("{name}-loaded"))?;
    let files = dir.join("files");
    std::fs::create_dir_all(&files).unwrap();
    std::fs::write(files.join("big"), vec![5u8; 300_000]).unwrap();
    let limit = 3 * base.to_listener.expect("the baseline reached the listener");
    let run = reboot(&pack, &root, &files, prom, true, limit);
    eprintln!("loaded: {run:?}");
    Some((base, run))
}

/// **M11 and R3, the reboot**: timers 1 and 2 left on and up under their
/// interrupt enables, and the file device enabled with commands queued due
/// after the PROM's reset devices; a jump to 36000 without `-RESET` then
/// reaches the listener as a reboot with neither does --- `INTR` at most
/// 1.1 times the baseline's executions and 10, the microcycles at most
/// 1.05 times --- with timers 1 and 2 off and the device disabled, no
/// handle open, at location 6, and no queued command run.
#[test]
fn m11_a_reboot_resets_the_timers_and_the_file_device() {
    let Some((base, run)) = m11("q11-m11", &muir::prom::quux_12_boot_prom()) else { return };
    m11_verdict(&run, &base).unwrap();
}
