// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! System 1002 dev11 on QUUX revision 10 (contract Q11): the boot PROM's
//! reset devices and timer 0's period (M9), the band ticking through the
//! destination 3 alias (M10), a reboot that resets timers 1 and 2 and the
//! file device (M11, and the Q9 amendment's R3), and the old PROM on
//! revision 10 (M12, a record).
//!
//! The band is the gitignored `ref/band-1002-dev11` (muir-sys's hand-over,
//! `tests/system_1002.rs`), named by digest; without it the tests skip and
//! say so. M9, M10 and M11 run on muir's built-in PROM,
//! `data/quux-promh.mcr`, revision 10's (muir-sys `381edfb`, with contract
//! Q11's steps 2, 5 and 6); M11's discriminating run and M12 on dev11's
//! own, the hand-over's `promh.mcr`, which writes neither word 104 nor
//! word 111.

use std::path::{Path, PathBuf};

use muir::engine::Engine;
use muir::isa::Insn;
use muir::isa::asm::{ALWAYS, JUMP, N, target};
use muir::machine::{IntervalTimer, Machine, Timers};
use muir::micro::Micro;
use muir::spy;
use muir::sym::{self, Space};
use muir::tv::Board;

mod support;

const BAND: &str = "ref/band-1002-dev11";
/// LISPM-1 and OZ, as the release's `site/hosts.text` gives them.
const CHAOS: (u16, u16) = (0o177201, 0o177200);

const PAGE: u32 = 0o17377000;
const fn control(k: usize) -> u32 {
    PAGE + 0o110 + 2 * k as u32
}
const fn period(k: usize) -> u32 {
    PAGE + 0o111 + 2 * k as u32
}
const RESET_DEVICES: u32 = PAGE + 0o104;
const FDEV_CONTROL: u32 = PAGE + 0o160;
const FDEV_STATUS: u32 = PAGE + 0o161;

/// The hand-over's files this names, by their SHA-256 in its `SHA256SUMS`.
const DIGESTS: [(&str, &str); 4] = [
    ("pack-1002-dev11.vhd", "4afd8122bc8beaf2c83ec8f297e8828c805f70e2f8ddcea3dd87b33b54cc2490"),
    ("tree-1002-dev11.tar.gz", "e47f0711abc4e455c74afd7321c704a61585eba5fbc6fa798a6e3d488bd6e37b"),
    ("ucadr.sym", "b4fbb44ba9c2ca7142e18f57060c745b5cea49c178687e7b38303522567f1f92"),
    ("promh.mcr", "dba5c36fbcf5e6277d4ce5d50387980f60c72588570e0eaacfdb840a419cd396"),
];

fn from() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(BAND)
}

/// The SHA-256 of `path`, by `sha256sum`.
fn sha256(path: &Path) -> String {
    let out = std::process::Command::new("sha256sum").arg(path).output().expect("sha256sum");
    String::from_utf8(out.stdout).unwrap().split_whitespace().next().unwrap().to_string()
}

/// Whether the hand-over is here; if not, says the test is skipped.
fn present() -> bool {
    let here = from().join("pack-1002-dev11.vhd").exists();
    if !here {
        eprintln!("skipped: {} is not present", from().display());
    }
    here
}

/// A copy of the disk, which the machine writes, and the served tree, in
/// a scratch directory, after checking the hand-over's digests.
fn band(name: &str) -> Option<(support::Scratch, PathBuf, PathBuf)> {
    if !present() {
        return None;
    }
    for (file, digest) in DIGESTS {
        assert_eq!(sha256(&from().join(file)), digest, "{file}: not dev11's");
    }
    let dir = support::scratch(name);
    let pack = dir.join("pack.vhd");
    std::fs::copy(from().join("pack-1002-dev11.vhd"), &pack).unwrap();
    let untar = std::process::Command::new("tar")
        .arg("xzf")
        .arg(from().join("tree-1002-dev11.tar.gz"))
        .arg("-C")
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(untar.success(), "the tree unpacks");
    let root = dir.join("root");
    std::fs::create_dir_all(root.join("lispm")).unwrap();
    for part in ["sys", "site"] {
        std::os::unix::fs::symlink(dir.join("release-1002").join(part), root.join(part)).unwrap();
    }
    Some((dir, pack, root))
}

/// dev11's own PROM, the hand-over's `promh.mcr`, which writes neither
/// word 104 nor word 111.
fn dev11_prom() -> Vec<Insn> {
    muir::prom::parse_quux_mcr(&std::fs::read(from().join("promh.mcr")).unwrap()).unwrap()
}

/// The band's microcode's address of `name`, from the hand-over's
/// `ucadr.sym`.
fn ucadr(name: &str) -> u16 {
    let text = std::fs::read_to_string(from().join("ucadr.sym")).unwrap();
    let symbols = sym::parse(&text).unwrap();
    symbols.address(Space::IMem, name).unwrap_or_else(|| panic!("{name} in ucadr.sym")) as u16
}

fn a_mem(name: &str) -> usize {
    let text = std::fs::read_to_string(from().join("ucadr.sym")).unwrap();
    let symbols = sym::parse(&text).unwrap();
    symbols.address(Space::AMem, name).unwrap_or_else(|| panic!("{name} in ucadr.sym")) as usize
}

/// QUUX with `prom` at 36000, the disk on block-disk, MONO TV at the
/// band's 1280 by 1024, and the file device serving `files`.
fn quux(pack: &Path, prom: &[Insn], files: &Path) -> Machine {
    use muir::block_disk::{BLOCK_NS, BlockDisk};
    let mut m = Machine::new();
    m.load_prom(prom);
    let mut d = BlockDisk::new(BLOCK_NS);
    d.attach(muir::disk_image::Disk::open_rw(pack).expect("the pack"));
    m.block_disk = Some(d);
    m.geometry = muir::machine::Geometry::QUUX;
    m.tv.set_board(Board::MonoTv);
    m.tv.set_mono_tv_size(1280, 1024);
    m.file_device.mounts.add(&files.display().to_string()).unwrap();
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
    let mut m = quux(&pack, &muir::prom::quux_boot_prom(), &root);
    m.register_log = Some(Vec::new());
    let mut e = Micro::new(m);
    e.boot();
    let n = until_executed(&mut e, 6, 20_000_000);
    eprintln!("location 6 after {n} microcycles on micro");
    let log = e.machine().register_log.clone().unwrap();
    let disk = log
        .iter()
        .position(|&(a, _, _)| muir::disk_controller::register(a).is_some())
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

/// **M10, the current band on revision 10 with the new PROM** (System 1002
/// dev11, named by digest): it boots to its listener; after `BEG06` word
/// 110 reads on, periodic, interrupt enable set, through the destination 3
/// alias; over 10 s of simulated time after the listener `INTR-TICK`
/// executes 600 ± 1 times; and a mouse move reaches the cursor. (The
/// unused-codes run's values are `tests/unused_codes.rs`'s.)
#[test]
fn m10_the_band_ticks_through_the_alias() {
    let Some((_dir, pack, root)) = band("q11-m10") else { return };
    m10(&pack, &root, &muir::prom::quux_boot_prom());
}

fn m10(pack: &Path, root: &Path, prom: &[Insn]) {
    let (tick, beg06) = (ucadr("INTR-TICK"), ucadr("BEG06"));
    let mut e = Micro::new(quux(pack, prom, root));
    e.boot();
    let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root.to_path_buf(), 400_000_000);
    eprintln!("listener after {ran} microcycles");
    let _ = beg06;
    let m = e.machine_mut();
    assert_eq!(m.bus_read(control(0)) & !2, 0o401, "word 110: on, periodic, <8>");
    assert_eq!(m.bus_read(period(0)), Timers::TICK_PERIOD_US, "word 111");
    let t0 = e.machine().ns;
    let mut ticks = 0u32;
    while e.machine().ns < t0 + 10_000_000_000 {
        e.step().unwrap();
        ticks += (e.executed() == Some(tick)) as u32;
    }
    eprintln!("INTR-TICK ran {ticks} times in 10 s");
    assert!(ticks.abs_diff(600) <= 1, "{ticks} ticks in 10 s");
    // The mouse.
    use muir::quux_input::KeyboardMouse;
    let (ax, ay) = (a_mem("A-MOUSE-X"), a_mem("A-MOUSE-Y"));
    let at = |e: &Micro| (e.machine().amem[ax], e.machine().amem[ay]);
    let before = at(&e);
    e.machine_mut().quux_input.mouse_move(-40, -30);
    let t1 = e.machine().ns;
    while e.machine().ns < t1 + 100_000_000 {
        e.step().unwrap();
    }
    let after = at(&e);
    eprintln!("mouse {before:?} -> {after:?}");
    assert_ne!(after, before, "the mouse move reached the cursor");
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

/// **M11's reboot**: boot to the listener with `prom` and halt; if
/// `loaded`, turn timers 1 and 2 on under their interrupt enables (1
/// periodic at 1,000 us, 2 one-shot at 5,000 us), run 6 ms so that both
/// are up, and halt again, and leave the file device enabled with an OPEN
/// done and three READs of 64 KiB each and a CREATE-DIRECTORY queued, the
/// first due 6.4 ms after the PROM's write of word 102, its step 4, just
/// before its reset devices. Then the PC to 36000 without `-RESET`, and on
/// to the listener, within `limit` microcycles.
fn reboot(
    pack: &Path,
    root: &Path,
    files: &Path,
    prom: &[Insn],
    loaded: bool,
    limit: u64,
) -> Reboot {
    let (intr, intr_at) = (ucadr("INTR"), 6u16);
    let mut e = Micro::new(quux(pack, prom, files));
    e.boot();
    let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root.to_path_buf(), 400_000_000);
    eprintln!("listener after {ran} microcycles");
    let post = |m: &mut Machine, k: u32, words: [u32; 8]| {
        let slot = (CMD_RING + 8 * (k % 4)) as usize;
        m.main[slot..slot + 8].copy_from_slice(&words);
        m.bus_write(PAGE + 0o164, k + 1);
    };
    use muir::file_device::op;
    let mut queue: Option<Queue> = None;
    if loaded {
        let m = e.machine_mut();
        m.bus_write(period(1), 1_000);
        m.bus_write(control(1), 0o401);
        m.bus_write(period(2), 5_000);
        m.bus_write(control(2), 0o405);
        let t = m.ns;
        while e.machine().ns < t + 6_000_000 {
            e.step().unwrap();
        }
        // Halted, so that the band touches none of the device's memory.
        e.spy_write(spy::CLK, 0);
        e.step().unwrap();
        e.step().unwrap();
        let m = e.machine_mut();
        assert_eq!(m.bus_read(control(1)) & 3, 3, "timer 1 on and up");
        assert_eq!(m.bus_read(control(2)) & 3, 3, "timer 2 on and up");
        // The rings, the device enabled, and a file opened.
        m.main[CMD_RING as usize..NAME as usize + 8].fill(0);
        m.bus_write(PAGE + 0o162, CMD_RING);
        m.bus_write(PAGE + 0o163, 2);
        m.bus_write(PAGE + 0o166, RESP_RING);
        m.bus_write(PAGE + 0o167, 2);
        m.bus_write(FDEV_CONTROL, 1);
        m.main[NAME as usize] = u32::from_le_bytes(*b"/big");
        post(m, 0, [1 | op::OPEN << 16, 0, NAME, 4, 0, 0, 0, 0]);
        let t = m.ns;
        while e.machine().file_device.response_producer() == 0 {
            e.step().unwrap();
            assert!(e.machine().ns < t + 1_000_000, "the OPEN answered, the machine halted");
        }
        let m = e.machine_mut();
        let handle = m.main[RESP_RING as usize + 2];
        assert_eq!(m.main[RESP_RING as usize] >> 16 & 0xff, 0, "the OPEN answered");
        m.bus_write(PAGE + 0o171, 1);
        // Queued at the PROM's step 4, below.
        queue = Some(Box::new(move |m: &mut Machine| {
            for k in 1..4 {
                post(m, k, [(1 + k) | (op::READ << 16), handle, 0, 0, BUF, 65_536, 0, 0]);
            }
            m.main[NAME as usize..NAME as usize + 2]
                .copy_from_slice(&[u32::from_le_bytes(*b"/new"), u32::from_le_bytes(*b"dir\0")]);
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
    let at_6 = [m.bus_read(control(1)), m.bus_read(control(2)), m.bus_read(FDEV_STATUS)];
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
    let Some((base, run)) = m11("q11-m11", &muir::prom::quux_boot_prom()) else { return };
    m11_verdict(&run, &base).unwrap();
}

/// **M11's discriminating run**: the same on dev11's own PROM, which
/// writes no word 104, fails the pass criterion; its figures are printed.
/// If it passed, M11 would show nothing.
#[test]
fn m11_fails_on_dev11_s_prom() {
    if !present() {
        return;
    }
    let Some((base, run)) = m11("q11-m11-dev11", &dev11_prom()) else { return };
    let verdict = m11_verdict(&run, &base);
    eprintln!("dev11's PROM: {verdict:?}");
    assert!(verdict.is_err(), "M11 passed on dev11's PROM: it shows nothing");
}

/// **M12, a record**: the old PROM, dev11's, on revision 10 from power-on:
/// whether the listener is reached, and `INTR-TICK`'s executions over 10 s
/// after it (expected 0: timer 0's period is 0). Not a pass condition.
#[test]
fn m12_the_old_prom_on_revision_10() {
    let Some((_dir, pack, root)) = band("q11-m12") else { return };
    let tick = ucadr("INTR-TICK");
    let mut e = Micro::new(quux(&pack, &dev11_prom(), &root));
    e.boot();
    let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root.clone(), 400_000_000);
    eprintln!("M12: listener after {ran} microcycles");
    let t0 = e.machine().ns;
    let mut ticks = 0u32;
    while e.machine().ns < t0 + 10_000_000_000 {
        e.step().unwrap();
        ticks += (e.executed() == Some(tick)) as u32;
    }
    let m = e.machine_mut();
    eprintln!(
        "M12: INTR-TICK ran {ticks} times in 10 s; word 110 {:o}, word 111 {}",
        m.bus_read(control(0)),
        m.bus_read(period(0))
    );
}
