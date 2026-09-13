// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The boot sequence from the keyboard's boot word to the processor's
//! `-BOOT`: on `micro` and `rtl` the behavioural I/O board's decode
//! pressing the engine's boot, and on `chip` the netlist board's `-BOOT*`
//! carried to the processor's `-BOOT1` by the far end. `tests/keyboard.rs`
//! holds the keyboard's side --- the keys, the word, the held-back key-ups
//! --- and `tests/ioboard.rs` the behavioural board's decode;
//! `docs/keyboard-boot.md` has the whole path with its citations.

use muir::engine::Engine;
use muir::ioboard::{self, IoBoard};
use muir::machine::Machine;
use muir::micro::Micro;
use muir::part::Level;
use muir::rtl::Rtl;
use muir::spy;
use muir::terminal::cable::OnCable;
use muir::terminal::keyboard::{self, Keyboard, RUBOUT, all_keys_up, keysym, up_down};
use muir::unibus::UnibusMaster;

mod support;

/// A machine with the boot PROM in it and no pack: booted, the PROM runs
/// its register self-test and waits on the drive.
fn prom_machine() -> Machine {
    let mut m = Machine::new();
    m.load_prom(&muir::prom::boot_prom());
    m
}

/// Reads the keyboard word as the microcode's channel does, high half
/// first, which the boot PROM never does --- `promh.text` has no keyboard
/// read in it --- so the test plays that part.
fn read_word(b: &mut IoBoard) -> u32 {
    let high = b.read(ioboard::KBD_HIGH, 0);
    let low = b.read(ioboard::KBD_LOW, 0);
    (high as u32 & 0xff) << 16 | low as u32
}

/// The distinct PCs an engine passes through in `steps` microcycles.
fn pcs<E: Engine>(e: &mut E, steps: usize) -> Vec<u16> {
    let mut seen = vec![e.pc()];
    for _ in 0..steps {
        e.step().expect("no halt this engine raises");
        if *seen.last().unwrap() != e.pc() {
            seen.push(e.pc());
        }
    }
    seen
}

/// **The PROM's own start is in `seen`**: word 0 is `jump 45`, and the PC
/// goes 0, then 45 --- with the 1 of the cycle nopped behind the jump
/// between them where an engine counts it, as `chip_boots_the_prom` in
/// `tests/chip.rs` notes.
fn starts_the_prom(what: &str, seen: &[u64]) {
    let octal: Vec<String> = seen.iter().map(|p| format!("{p:o}")).collect();
    let at = seen
        .iter()
        .position(|&p| p == 0)
        .unwrap_or_else(|| panic!("{what}: never at 0: {octal:?}"));
    let after = &seen[at..];
    assert!(
        after.starts_with(&[0, 0o45]) || after.starts_with(&[0, 1, 0o45]),
        "{what}: the PROM's own start, 0 then 45: {octal:?}"
    );
}

/// **The boot sequence re-enters the boot on `micro` and on `rtl`.** The
/// PROM is running with the keyboard's three key-downs read; the boot
/// word lands, the board's decode raises its request, and
/// [`Engine::keyboard_boot`] presses the boot as the button does: `RUN`
/// preset, the mode register cleared so the PROM is back over the control
/// store, and the PROM's first words, 0 and 45, run again. The word is
/// left in the register with `KBD READY` up, which is what microcode 323
/// reads at `(LOC 6)` to choose cold from warm.
fn boots_again<E: Engine>(what: &str, e: &mut E, cold: bool) {
    e.boot();
    for _ in 0..2_000 {
        e.step().unwrap_or_else(|h| panic!("{what}: halted in the PROM: {h:?}"));
    }
    assert_ne!(e.pc(), 0, "{what}: the PROM is running");
    let mut k = Keyboard::new();
    k.key(keysym::CONTROL_L, true);
    k.key(keysym::ALT_L, true);
    k.key(if cold { keysym::BACKSPACE } else { keysym::RETURN }, true);
    assert_eq!(k.pending(), 4, "{what}: three key-downs and the boot word");
    for n in 0..3 {
        assert!(k.deliver(&mut e.machine_mut().ioboard), "{what}: word {n} goes in");
        assert!(!e.keyboard_boot(), "{what}: word {n} is a key-down, not the boot word");
        let word = read_word(&mut e.machine_mut().ioboard);
        assert_eq!(word & keyboard::UP, 0, "{what}: a key-down, {word:o}");
        for _ in 0..100 {
            e.step().expect("no halt this engine raises");
        }
    }
    assert!(k.deliver(&mut e.machine_mut().ioboard), "{what}: the boot word goes in");
    assert!(e.keyboard_boot(), "{what}: and presses the boot");
    assert!(!e.keyboard_boot(), "{what}: once");
    let b = &mut e.machine_mut().ioboard;
    assert!(b.keyboard_ready(), "{what}: the word is still there for (LOC 6)");
    assert_eq!(read_word(b) & 0o77, if cold { 0o46 } else { 0o62 }, "{what}: and says which");
    let f = spy::Flag1::of(e.spy_read(spy::FLAG_1));
    assert!(f.srun, "{what}: RUN preset");
    assert!(!f.promdisable, "{what}: the PROM back over the control store");
    let seen: Vec<u64> = pcs(e, 12).into_iter().map(u64::from).collect();
    starts_the_prom(what, &seen);
}

#[test]
fn the_boot_sequence_re_enters_the_boot_on_micro_and_rtl() {
    boots_again("micro, cold", &mut Micro::new(prom_machine()), true);
    boots_again("micro, warm", &mut Micro::new(prom_machine()), false);
    boots_again("rtl, cold", &mut Rtl::new(prom_machine()), true);
    boots_again("rtl, warm", &mut Rtl::new(prom_machine()), false);
}

/// Runs the board to `until`, letting the keyboard see every edge of the
/// clock on the way, and calls `seen` after every step.
fn run(b: &mut UnibusMaster, k: &mut OnCable, until: u64, mut seen: impl FnMut(&UnibusMaster)) {
    while b.now < until {
        let tap = b.chip.next_tap().unwrap_or(until).clamp(b.now + 1, until);
        b.run(tap);
        k.apply(&mut b.chip, b.now);
        seen(b);
    }
}

/// **The netlist I/O board pulses `-BOOT*` low for the boot word and for
/// no other word, and holds the word for the microcode.** The 25LS2521 at
/// IOBCSR 0A20 is enabled by `EOC.KBD^`, the 74LS10 at IOBKBD 0C28 ---
/// `NAND(SR0, -CHAR TO MOUSE, -KB CLK^)` --- which is low while `KB CLK^`
/// is low with the start marker at `SR0`: the half clock before the
/// rising edge that latches the word into the 74LS374s and sets `KBD
/// READY`. So `-BOOT*` is a pulse of that width, ending on the edge that
/// sets `KBD READY`, and not a level: the machine is booted and let go, as
/// by the button. Measured here, on the board.
#[test]
fn the_netlist_board_pulses_boot_star_for_the_boot_word_alone() {
    let n = support::cadrio();
    let mut b = UnibusMaster::new(&n, 500_000, &support::quiet());
    let mut k = OnCable::of(&n).expect("the I/O board has the keyboard cable");
    k.apply(&mut b.chip, b.now);
    let boot_star = b.net("-BOOT*");
    let ready = b.net("'KBD READY'");
    assert_eq!(b.chip.net(boot_star), Level::High, "pulled up at rest");
    for (word, what, boots) in [
        (up_down(RUBOUT, false), "Rubout down", false),
        (keyboard::boot(true), "the cold boot word", true),
        (all_keys_up(1 << 4 | 1 << 5), "all keys up but Control and Meta", false),
        (keyboard::boot(false), "the warm boot word", true),
    ] {
        assert!(k.send(word));
        let t0 = b.now;
        // The pulse: when `-BOOT*` first read low, and when it was next
        // read high after that; and when `KBD READY` first read high.
        let mut low: Option<(u64, Option<u64>)> = None;
        let mut rose: Option<u64> = None;
        run(&mut b, &mut k, t0 + 400_000, |b| {
            match (b.chip.net(boot_star), low) {
                (Level::Low, None) => low = Some((b.now, None)),
                (Level::High, Some((from, None))) => low = Some((from, Some(b.now))),
                _ => {}
            }
            if rose.is_none() && b.chip.net(ready) == Level::High {
                rose = Some(b.now);
            }
        });
        assert_eq!(b.chip.net(ready), Level::High, "{what}: KBD READY up, the word taken");
        assert!(!k.busy(), "{what}: the keyboard is idle again");
        match low {
            Some((from, Some(to))) => {
                let width = to - from;
                eprintln!(
                    "{what}: -BOOT* low {} us after the word was queued, for {width} ns",
                    (from - t0) / 1_000
                );
                assert!(boots, "{what}: -BOOT* pulsed for a word that is not the boot word");
                assert!((1_000..=8_000).contains(&width), "{what}: a half-clock pulse, {width} ns");
                assert_eq!(
                    rose,
                    Some(to),
                    "{what}: the edge that ends the pulse is the one that sets KBD READY"
                );
            }
            Some((from, None)) => panic!("{what}: -BOOT* low from {from} and never released"),
            None => assert!(!boots, "{what}: -BOOT* never went low"),
        }
        assert_eq!(b.chip.net(boot_star), Level::High, "{what}: high again");
        // The word is still in the register, whole, for the microcode.
        let (_, high) = b.cycle(ioboard::KBD_HIGH, None);
        let (_, low_half) = b.cycle(ioboard::KBD_LOW, None);
        assert_eq!((high as u32 & 0xff) << 16 | low_half as u32, word, "{what}: reads back");
        assert_eq!(b.chip.net(ready), Level::Low, "{what}: and reading it cleared ready");
    }
}

/// **Through the far end, the I/O board's `-BOOT*` reaches the processor's
/// `-BOOT1` and the netlist machine boots again.** The far end drives
/// `-BOOT1` low while the board holds `-BOOT*` low and releases it after;
/// on the processor `-BOOT1` reaches the 74S02 at OLORD2 1A07 that makes
/// `-BOOT`, which presets `RUN` and forces the boot trap, and the PROM
/// runs from its first word again with the boot word waiting in the
/// board's register. The link between the board's pin `CP1` and the
/// interface's `CR1` is what no file establishes (`docs/keyboard-boot.md`,
/// `tests/unibus_backplane_pins.rs`); this holds the model's wire.
#[test]
fn the_boot_word_boots_the_netlist_machine_through_the_far_end() {
    use muir::clock::Clock;
    let n = support::netlists();
    let (mut c, mut clk, mut far) = support::chip(&n);
    c.power_on();
    c.load_prom(&n.cpu, &muir::prom::boot_prom_image());
    c.settle();
    far.join(&mut c, clk.time_ns());
    let net = |name: &str| n.cpu.by_name_id(name).unwrap_or_else(|| panic!("no net {name}"));
    let (boot1, boot, boot2, run) = (net("-BOOT1"), net("-BOOT"), net("-BOOT2"), net("RUN"));
    let pc = c.bus_nets(&n.cpu, "PC", 14);
    let ready = n.cadrio.by_name_id("'KBD READY'").expect("the I/O board's KBD READY");
    assert!(far.unibus.is_some(), "a netlist I/O board on the far end");
    assert_ne!(c.net(boot1), Level::Low, "nothing on the keyboard's boot line");
    assert_eq!(c.net(boot), Level::High);

    // The button, as muir presses it, and the PROM running off it.
    c.set_net(boot2, Level::Low);
    c.settle();
    for _ in 0..20 {
        c.tick(&mut clk);
    }
    c.set_net(boot2, Level::High);
    let until = clk.time_ns() + 300_000;
    while clk.time_ns() < until {
        far.tick_with(&mut c, &mut clk);
    }
    assert_ne!(c.read(&pc), 0, "the PROM is running");
    assert_eq!(c.net(boot), Level::High);

    // The boot word down the keyboard cable; 25 clocks at 8 us is 200 us.
    far.unibus
        .as_mut()
        .unwrap()
        .keyboard()
        .expect("a keyboard on the cable")
        .send(keyboard::boot(true));
    let t0 = clk.time_ns();
    while c.net(boot1) != Level::Low && clk.time_ns() < t0 + 600_000 {
        far.tick_with(&mut c, &mut clk);
    }
    let pressed = clk.time_ns();
    assert_eq!(c.net(boot1), Level::Low, "-BOOT1 low within 600 us");
    assert!(far.unibus.as_ref().unwrap().boot_low(), "because the board holds -BOOT* low");
    assert_eq!(c.net(boot), Level::Low, "and -BOOT with it");
    assert_eq!(c.net(run), Level::High, "RUN preset");
    while c.net(boot) != Level::High && clk.time_ns() < pressed + 50_000 {
        far.tick_with(&mut c, &mut clk);
    }
    let released = clk.time_ns();
    assert_eq!(c.net(boot), Level::High, "-BOOT released within 50 us");
    assert_ne!(c.net(boot1), Level::Low, "-BOOT1 let go");
    assert!(!far.unibus.as_ref().unwrap().boot_low(), "as the board let -BOOT* go");
    eprintln!(
        "-BOOT1 low {} us after the word was queued, for {} ns",
        (pressed - t0) / 1_000,
        released - pressed
    );
    assert_eq!(
        far.unibus.as_ref().unwrap().board.net(ready),
        Level::High,
        "the word waits for (LOC 6)"
    );

    // The PROM from its first word: 0, then 45.
    let mut seen = vec![c.read(&pc)];
    let until = clk.time_ns() + 5_000;
    while clk.time_ns() < until {
        far.tick_with(&mut c, &mut clk);
        let now = c.read(&pc);
        if *seen.last().unwrap() != now {
            seen.push(now);
        }
    }
    starts_the_prom("chip", &seen);
}
