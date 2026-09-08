// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Checkpoints: the file's packing and header, and a run picked up from
//! one being the run that was never stopped.  The engine tests boot the
//! System 100 pack, so they skip and say so without `vendor/`.

use std::path::PathBuf;

use muir::checkpoint::{self, Reader, Writer};
use muir::clock::Behavioural;
use muir::disk_unit::{Geometry, Unit};
use muir::engine::Engine;
use muir::machine::Machine;
use muir::micro::Micro;
use muir::rtl::Rtl;

mod support;

/// **Packing keeps every byte and costs a run of zeros almost nothing.**
#[test]
fn packing_keeps_the_bytes_and_shrinks_the_zeros() {
    let mut mixed = vec![0u8; 10];
    mixed.extend([1, 2, 0, 3, 0, 0, 4]);
    mixed.extend([0u8; 100]);
    mixed.extend([5, 0, 0, 0, 0, 6]);
    for raw in [vec![], vec![1, 2, 3], vec![0; 10], vec![0, 0, 0, 7], vec![7, 0, 0, 0], mixed] {
        assert_eq!(checkpoint::unpack(&checkpoint::pack(&raw)).unwrap(), raw, "{raw:?}");
    }
    let empty_memory = vec![0u8; 8 << 20];
    assert!(checkpoint::pack(&empty_memory).len() < 8, "eight megabytes of zeros in a few bytes");
    assert!(checkpoint::unpack(&[3, 5, 1]).is_err(), "literals that run off the end");
}

/// **A writer's fields read back in order, and a reader says when they do
/// not fit.**
#[test]
fn the_fields_read_back_in_order() {
    let mut w = Writer::new();
    w.u8(7);
    w.u16(0x1234);
    w.u32(0xdead_beef);
    w.u64(u64::MAX - 1);
    w.bool(true);
    w.opt(Some(9u8), Writer::u8);
    w.opt(None::<u16>, Writer::u16);
    w.u32s(&[1, 2, 3]);
    w.bytes(b"pack");
    w.speed(muir::clock::Speed::Fast);
    let body = w.finish();
    let mut r = Reader::new(&body);
    assert_eq!(r.u8().unwrap(), 7);
    assert_eq!(r.u16().unwrap(), 0x1234);
    assert_eq!(r.u32().unwrap(), 0xdead_beef);
    assert_eq!(r.u64().unwrap(), u64::MAX - 1);
    assert!(r.bool().unwrap());
    assert_eq!(r.opt(Reader::u8).unwrap(), Some(9));
    assert_eq!(r.opt(Reader::u16).unwrap(), None);
    let mut three = [0u32; 3];
    r.u32s_into(&mut three).unwrap();
    assert_eq!(three, [1, 2, 3]);
    assert_eq!(r.bytes().unwrap(), b"pack");
    assert_eq!(r.speed().unwrap(), muir::clock::Speed::Fast);
    r.done().unwrap();
    assert!(r.u8().is_err(), "nothing left to read");

    let mut r = Reader::new(&[2]);
    assert!(r.bool().is_err(), "2 is no flag");
    let mut w = Writer::new();
    w.u32s(&[1, 2]);
    let body = w.finish();
    let mut into = [0u32; 3];
    assert!(Reader::new(&body).u32s_into(&mut into).is_err(), "two words where three belong");
    assert!(Reader::new(&body).done().is_err(), "bytes left over");
}

/// **The file names the engine that wrote it, and anything else is
/// refused.**
#[test]
fn the_file_names_its_engine_and_refuses_other_files() {
    let dir = std::env::temp_dir().join(format!("muir-checkpoint-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.chk");
    let body: Vec<u8> =
        (0..300u32).map(|i| if i % 7 == 0 { i } else { 0 }).flat_map(|v| v.to_le_bytes()).collect();
    let size = checkpoint::write(&path, "micro", 4, &body).unwrap();
    assert_eq!(size, std::fs::metadata(&path).unwrap().len());
    let back = checkpoint::read(&path).unwrap();
    assert_eq!(back.engine, "micro");
    assert_eq!(back.memory_boards, 4);
    assert_eq!(back.body, body);
    std::fs::write(&path, b"GIF89a not a checkpoint at all").unwrap();
    let err = checkpoint::read(&path).unwrap_err().to_string();
    assert!(err.contains("not a muir checkpoint"), "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

/// **The format is version 11, and a file of another version is refused by
/// number.** The version is bumped whenever a type changes what it writes,
/// so a file from another build is read wrong or not at all; this pins
/// which it is, and that the refusal names both versions. Version 3 added
/// the serial port's registers to the I/O board's, version 4 the instant
/// the interval timer was loaded, version 5 `micro`'s pending map write,
/// version 6 the speaker's flip-flop, version 7 the mouse interface's
/// latches and clock with the encoders on its lines, version 8 the bit
/// count of what the Chaosnet interface received or has landing, and
/// version 9 `micro`'s `NEXT INSTR` and `NEXT INSTRD`, the two stages
/// of the fetch a `POPJ` asks for, version 10 the disk controller's
/// overrun, version 11 the drives on a netlist disk controller's
/// cable and the multiplexor between them, version 12 one format for the
/// harness and the binary (`af676fa`), and version 13 the disk
/// controller's header ECC error.
#[test]
fn the_format_is_version_13_and_another_version_is_refused() {
    assert_eq!(checkpoint::VERSION, 13, "a new version needs its own tests");
    let dir = std::env::temp_dir().join(format!("muir-checkpoint-version-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.chk");
    checkpoint::write(&path, "micro", 4, &[1, 2, 3]).unwrap();
    let good = std::fs::read(&path).unwrap();
    // The version is the four bytes after the magic line.
    let at = b"muir checkpoint\n".len();
    assert_eq!(&good[at..at + 4], 13u32.to_le_bytes());
    for other in [1u32, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, u32::MAX] {
        let mut file = good.clone();
        file[at..at + 4].copy_from_slice(&other.to_le_bytes());
        std::fs::write(&path, &file).unwrap();
        let err = checkpoint::read(&path).unwrap_err().to_string();
        assert!(err.contains(&format!("format version {other}")), "{err}");
        assert!(err.contains("reads 13"), "{err}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// **A run of zeros longer than any body is refused, not allocated.** The
/// body's size is not in the header, so a count is checked against the
/// most a body can be: a corrupt `--resume` file whose count says
/// eighteen exabytes, or a terabyte, is an error before a byte of it is
/// made room for.
#[test]
fn a_run_of_zeros_longer_than_any_body_is_refused() {
    // A count as an LEB128 varint, then a count of no literals.
    let packed = |zeros: u64| {
        let mut out = Vec::new();
        let mut v = zeros;
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(byte);
                break;
            }
            out.push(byte | 0x80);
        }
        out.push(0);
        out
    };
    assert_eq!(checkpoint::unpack(&packed(8 << 20)).unwrap(), vec![0u8; 8 << 20]);
    for zeros in [u64::MAX, 1 << 40] {
        let err = checkpoint::unpack(&packed(zeros)).unwrap_err().to_string();
        assert!(err.contains("zeros"), "{err}");
    }
}

/// **A pointer wider than its register is a corrupt checkpoint, and is
/// refused** before an engine indexes a stack with it. The SPC pointer is
/// `SPCPTR<4:0>`, five bits addressing the 32-word SPC stack; the PDL
/// pointer and index are ten bits addressing the 1K-word PDL buffer; the
/// engines mask every write to them, so a saved value is never wider, and
/// a wider one has been corrupted on the way.
#[test]
fn a_pointer_wider_than_its_register_is_refused() {
    let mut spc = Machine::with_memory_boards(1);
    spc.spcptr = 0o40;
    let mut pointer = Machine::with_memory_boards(1);
    pointer.pdl_pointer = 0o2000;
    let mut index = Machine::with_memory_boards(1);
    index.pdl_index = 0o2000;
    for (what, m) in [("SPC pointer", spc), ("PDL pointer", pointer), ("PDL index", index)] {
        let mut w = Writer::new();
        m.save(&mut w);
        let body = w.finish();
        let mut back = Machine::with_memory_boards(1);
        let err = back.load(&mut Reader::new(&body)).unwrap_err().to_string();
        assert!(err.contains(what), "{what}: {err}");
    }
    // The widest value each register holds loads as itself.
    let mut m = Machine::with_memory_boards(1);
    m.spcptr = 0o37;
    m.pdl_pointer = 0o1777;
    m.pdl_index = 0o1777;
    let mut w = Writer::new();
    m.save(&mut w);
    let body = w.finish();
    let mut back = Machine::with_memory_boards(1);
    let mut r = Reader::new(&body);
    back.load(&mut r).unwrap();
    r.done().unwrap();
    assert_eq!((back.spcptr, back.pdl_pointer, back.pdl_index), (0o37, 0o1777, 0o1777));
}

/// **A count of clock events with nothing behind it is an error, not an
/// allocation.** The `chip` clock saves its pending events behind a
/// count; the count is read from the file, and a corrupt one saying four
/// billion has to fail on the first event that is not there rather than
/// make room for all of them first.
#[test]
fn a_clock_with_more_events_than_the_file_holds_is_refused() {
    let mut file = b"CADRCLK1".to_vec();
    // The time, the cycle's start, the read phase and the four outputs.
    file.extend([0u8; 8 + 8 + 4 + 4]);
    file.extend(u32::MAX.to_le_bytes());
    assert!(Behavioural::load(&mut file.as_slice()).is_err());
    // And a clock as saved loads as itself.
    let clock = Behavioural::default();
    let mut saved = Vec::new();
    clock.save(&mut saved).unwrap();
    let back = Behavioural::load(&mut saved.as_slice()).unwrap();
    let mut again = Vec::new();
    back.save(&mut again).unwrap();
    assert_eq!(again, saved);
}

/// **A checkpoint names how much memory its machine had, and loads onto
/// no other.** Four boards' worth into a machine of thirty-two is refused
/// by name, before any word of memory is read.
#[test]
fn a_checkpoint_loads_onto_a_machine_with_as_much_memory() {
    let mut small = Micro::new(Machine::with_memory_boards(4));
    small.boot();
    let mut w = Writer::new();
    small.save(&mut w);
    let body = w.finish();
    let mut big = Micro::new(Machine::with_memory_boards(32));
    big.boot();
    let err = big.load(&mut Reader::new(&body)).unwrap_err().to_string();
    assert!(err.contains("4 memory boards") && err.contains("32"), "{err}");
    let mut same = Micro::new(Machine::with_memory_boards(4));
    same.boot();
    let mut r = Reader::new(&body);
    same.load(&mut r).unwrap();
    r.done().unwrap();
}

/// A machine booting the pack, as `muir` builds one.
fn machine(pack: &PathBuf) -> Machine {
    let mut m = Machine::new();
    m.load_prom(&muir::prom::boot_prom());
    m.disk.attach(0, Unit::open(pack, Geometry::T300).expect("the System 100 pack"));
    m
}

/// Runs `straight` to `at`, checkpoints it into `resumed`, then runs both
/// `more` microcycles in step: the same PC every microcycle, and the same
/// checkpoint at the end.
fn resumes<E: Engine>(name: &str, mut straight: E, mut resumed: E, at: u64, more: u64) {
    let (ran, halt) = straight.run(at);
    assert_eq!((ran, halt), (at, None), "{name}: the straight run to the checkpoint");
    let mut w = Writer::new();
    straight.save(&mut w);
    let body = w.finish();
    let mut r = Reader::new(&body);
    resumed.load(&mut r).unwrap_or_else(|e| panic!("{name}: loading the checkpoint: {e}"));
    r.done().unwrap();
    let mut w = Writer::new();
    resumed.save(&mut w);
    assert_eq!(w.finish(), body, "{name}: the checkpoint loads and saves as itself");
    assert_eq!(resumed.machine().cycles, straight.machine().cycles);
    for n in 0..more {
        straight.step().unwrap();
        resumed.step().unwrap();
        assert_eq!(resumed.pc(), straight.pc(), "{name}: the PC {n} microcycles on");
    }
    let (mut a, mut b) = (Writer::new(), Writer::new());
    straight.save(&mut a);
    resumed.save(&mut b);
    assert_eq!(a.finish(), b.finish(), "{name}: the same state {more} microcycles on");
    assert!(
        resumed.machine().mode.prom_disable,
        "{name}: the window reaches past the PROM into the band"
    );
}

/// **`micro` picks up where the checkpoint left off.** Into the band's
/// microcode load, where the PROM has handed over and the disk has been
/// read; then a hundred thousand microcycles more, in step.
#[test]
fn micro_picks_up_where_the_checkpoint_left_off() {
    let Some(pack) = support::pack_100() else { return };
    let mut straight = Micro::new(machine(&pack));
    straight.boot();
    let mut resumed = Micro::new(machine(&pack));
    resumed.boot();
    resumes("micro", straight, resumed, 1_600_000, 100_000);
}

/// **`rtl` picks up where the checkpoint left off.** The same, with the
/// Chaosnet interface plugged in, so its state goes and comes too.
#[test]
fn rtl_picks_up_where_the_checkpoint_left_off() {
    let Some(pack) = support::pack_100() else { return };
    let build = || {
        let mut m = machine(&pack);
        m.plug_chaos(0);
        let mut e = Rtl::new(m);
        e.boot();
        e
    };
    resumes("rtl", build(), build(), 1_600_000, 100_000);
}

// --- chip -------------------------------------------------------------------

const CPU: &str = include_str!("../data/CADR.netlist");
const BUSINT: &str = include_str!("../data/BUSINT.netlist");
const CADRM: &str = include_str!("../data/CADRM.netlist");
const CADRIO: &str = include_str!("../data/CADRIO.netlist");
const SIMPLETV: &str = include_str!("../data/SIMPLETV.netlist");

/// How many memory boards the netlist machines here have. Fewer than the
/// thirty-two `muir` gives one: the round trip is the same whatever the
/// count, and each board is a netlist to build and a quarter of a megabyte
/// of cells to compare.
const BOARDS: usize = 4;

/// A netlist machine as `muir --chip` builds one, with the boot PROM in the
/// processor and the boards `--chip` runs by default on the backplane ---
/// the memory, the I/O board and the display as netlists, the disk
/// controller as the machine's model. `press` is the boot button: a run
/// from power-on presses it, and a resume does not, because a checkpoint
/// replaces everything the button and the power-on set.
fn chip_machine(press: bool) -> (muir::chip::Chip, Behavioural, muir::cable::FarEnd) {
    use muir::netlist;
    use muir::part::Level;
    let n = netlist::parse(CPU).unwrap();
    let bus_n = netlist::parse(BUSINT).unwrap();
    let mem_n = netlist::parse(CADRM).unwrap();
    let io_n = netlist::parse(CADRIO).unwrap();
    let tv_n = netlist::parse(SIMPLETV).unwrap();
    let mut c = muir::chip::Chip::new(&n);
    c.power_on();
    c.load_prom(&n, &muir::prom::boot_prom_image());
    c.settle();
    let mut clk = Behavioural::new();
    let boards = muir::cable::Boards {
        memory: BOARDS,
        io: Some(&io_n),
        tv: Some(&tv_n),
        ..Default::default()
    };
    let mut far = muir::cable::FarEnd::new(
        &n,
        &bus_n,
        &mem_n,
        boards,
        0,
        Machine::with_memory_boards(BOARDS),
    );
    far.join(&mut c, muir::clock::Clock::time_ns(&clk));
    if press {
        // The button, as `muir` presses it: `-BOOT1` held down, the board
        // settled with it down, and twenty master clocks before it rises.
        let boot = n.by_name_id("-BOOT1").unwrap();
        c.set_net(boot, Level::Low);
        c.settle();
        for _ in 0..20 {
            c.tick(&mut clk);
        }
        c.set_net(boot, Level::High);
    }
    (c, clk, far)
}

/// Runs `at_least` microcycles and then on to the first point a checkpoint
/// may be taken at: the interface between cycles, no tap in flight on any
/// board and no memory request up. Returns how many microcycles that took.
fn run_to_quiet(
    c: &mut muir::chip::Chip,
    clk: &mut Behavioural,
    far: &mut muir::cable::FarEnd,
    at_least: u64,
) -> u64 {
    use muir::clock::Clock;
    let n = muir::netlist::parse(CPU).unwrap();
    let memrq = n.by_name_id("MEMRQ").unwrap();
    let mut ran = 0;
    let mut last = clk.phase_ns();
    loop {
        far.tick_with(c, clk);
        let p = clk.phase_ns();
        let wrapped = p < last;
        last = p;
        if !wrapped {
            continue;
        }
        ran += 1;
        if ran >= at_least
            && far.quiet()
            && c.next_tap().is_none()
            && c.net(memrq) != muir::part::Level::High
        {
            return ran;
        }
        assert!(ran < at_least + 1000, "no quiet microcycle within a thousand of {at_least}");
    }
}

/// A netlist machine's whole state as a checkpoint holds it, in the four
/// pieces it is written in, each named: a difference is reported as the
/// piece it is in and how far into it, because the whole is megabytes of
/// nets and cells.
fn chip_pieces(
    c: &muir::chip::Chip,
    clk: &Behavioural,
    far: &muir::cable::FarEnd,
) -> Vec<(&'static str, Vec<u8>)> {
    let piece = |f: &dyn Fn(&mut Writer)| {
        let mut w = Writer::new();
        f(&mut w);
        w.finish()
    };
    vec![
        ("the processor", piece(&|w| c.save(w).unwrap())),
        ("the clock", piece(&|w| clk.save(w).unwrap())),
        ("the boards", piece(&|w| far.save(w).unwrap())),
        ("what is behind the buses", piece(&|w| far.buses.save(w))),
    ]
}

/// The pieces run together, which is what a checkpoint's body is.
fn chip_body(c: &muir::chip::Chip, clk: &Behavioural, far: &muir::cable::FarEnd) -> Vec<u8> {
    let mut w = Writer::new();
    c.save(&mut w).unwrap();
    clk.save(&mut w).unwrap();
    far.checkpoint(&mut w).expect("the boards this machine has are all in a checkpoint");
    w.finish()
}

/// The two machines' states piece by piece, saying which piece differs and
/// where rather than printing megabytes of them.
fn same_state(
    what: &str,
    a: (&muir::chip::Chip, &Behavioural, &muir::cable::FarEnd),
    b: (&muir::chip::Chip, &Behavioural, &muir::cable::FarEnd),
) {
    for ((name, x), (_, y)) in chip_pieces(a.0, a.1, a.2).iter().zip(chip_pieces(b.0, b.1, b.2)) {
        assert_eq!(x.len(), y.len(), "{what}: {name} is a different length");
        if let Some(at) = x.iter().zip(&y).position(|(p, q)| p != q) {
            let end = (at + 16).min(x.len());
            panic!(
                "{what}: {name} differs at byte {at} of {}: {:?} against {:?}",
                x.len(),
                &x[at..end],
                &y[at..end]
            );
        }
    }
}

/// **`chip` picks up where the checkpoint left off.** The netlist machine
/// is not an [`Engine`] and its state is not arrays: the processor's
/// scratchpads and control store are the RAM chips' own cells, and the
/// rest is every net's level, every part's bits, and the oscillators and
/// one-shots of five boards mid-pulse. So this is the same check
/// [`resumes`] makes of the other two engines, made of the pieces `muir
/// --chip` runs: saved at a quiet microcycle in the boot PROM, loaded onto
/// a machine built and not booted, and the two the same board a thousand
/// microcycles later.
///
/// The boot PROM alone, so nothing here needs `vendor/`.
#[test]
fn chip_picks_up_where_the_checkpoint_left_off() {
    use muir::clock::Clock;
    let (mut c, mut clk, mut far) = chip_machine(true);
    let at = run_to_quiet(&mut c, &mut clk, &mut far, 400);
    let body = chip_body(&c, &clk, &far);

    let (mut c2, _, mut far2) = chip_machine(false);
    let mut r = Reader::new(&body);
    c2.load(&mut r).unwrap();
    let mut clk2 = Behavioural::load(&mut r).unwrap();
    far2.resume(&mut r).unwrap();
    r.done().unwrap();
    same_state("the checkpoint loads and saves as itself", (&c, &clk, &far), (&c2, &clk2, &far2));
    assert_eq!(chip_body(&c2, &clk2, &far2), body, "the checkpoint loads and saves as itself");
    // The cables joined, as a resume joins them: each board holds what
    // the others are driving onto it, and that is the checkpoint's, so
    // this carries nothing and moves no board.
    far2.join(&mut c2, clk2.time_ns());

    // The same board, microcycle for microcycle, a thousand on. The PC is
    // read off the nets: there is no `Engine::pc` here.
    let n = muir::netlist::parse(CPU).unwrap();
    let pc_nets = c.bus_nets(&n, "PC", 14);
    let mut last = (clk.phase_ns(), clk2.phase_ns());
    let mut ran = 0;
    while ran < 1000 {
        far.tick_with(&mut c, &mut clk);
        far2.tick_with(&mut c2, &mut clk2);
        let p = (clk.phase_ns(), clk2.phase_ns());
        if p.0 < last.0 {
            ran += 1;
            assert_eq!(
                c2.read(&pc_nets),
                c.read(&pc_nets),
                "the PC {ran} microcycles past the checkpoint at {at}"
            );
        }
        assert_eq!(p, (p.0, p.0), "the two clocks in step at microcycle {ran}");
        last = p;
    }
    same_state("the same state 1000 microcycles on", (&c, &clk, &far), (&c2, &clk2, &far2));
}
