// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! QUUX's memory port (contract Q6, revision 7). Main memory is off the
//! Xbus: the processor reaches it through its own port, through the cache
//! --- always fitted, 4K words in lines of 4, 2-way, a hit in 20 ns, the
//! write buffer --- to main memory at one nominal timing, a line fill in
//! 380 ns and a write in 290. Device registers are never cached; an address
//! nothing answers, past main memory's end or in the old Unibus window,
//! fails with the NXM bit (at once since Q7, `tests/quux_device_registers.rs`).
//! QUUX has no bus interface; the CADR keeps its own.

use muir::cache::{CacheConfig, MemoryTiming};
use muir::engine::Engine;
use muir::isa::Insn;
use muir::isa::asm::{
    ALU, ALWAYS, JUMP, MD, N, SETA, SETM, SRC_MD, START_READ, START_WRITE, a_dest, a_src, filler,
    m_src, target,
};
use muir::machine::{Geometry, Machine, bus_error};
use muir::micro::Micro;
use muir::rtl::Rtl;

mod support;

/// A program that reads the physical words `reads` in turn into A 200 up,
/// then stops on a jump to itself at [`STOP`]. Virtual page `k + 1` maps
/// onto each word's physical page; M `k + 1` holds its virtual address.
fn reading(geometry: Geometry, reads: &[u32]) -> Machine {
    let mut prom = Vec::new();
    for k in 0..reads.len() as u64 {
        prom.push(Insn::new(ALU | SETM | m_src(1 + k) | START_READ));
        // The microinstruction after a start may not read `MD`.
        prom.push(filler());
        prom.push(Insn::new(ALU | SETM | SRC_MD | a_dest(0o200 + k)));
    }
    prom.push(Insn::new(JUMP | target(prom.len() as u64) | ALWAYS | N));
    machine(geometry, &prom, reads)
}

fn machine(geometry: Geometry, prom: &[Insn], addresses: &[u32]) -> Machine {
    let mut m = Machine::new();
    m.geometry = geometry;
    let mut words = vec![filler(); 1024];
    words[..prom.len()].copy_from_slice(prom);
    m.load_prom(&words);
    support::prom_program_in_ram(&mut m);
    let rw = (1 << 23) | (1 << 22);
    for (k, &p) in addresses.iter().enumerate() {
        m.l2_map[1 + k] = rw | (p >> 8);
        m.mmem[1 + k] = ((1 + k as u32) << 8) | (p & 0xff);
    }
    for (k, w) in m.main[..0o4000].iter_mut().enumerate() {
        *w = 0o1000000 + k as u32;
    }
    m
}

/// Boots `e` and runs it well past the program's last jump.
fn run<E: Engine>(e: &mut E) {
    e.boot();
    for _ in 0..4000 {
        e.step().unwrap();
    }
}

/// **QUUX has its memory port and no bus interface**: the cache fitted at
/// its shape and main memory at the nominal timing; the CADR keeps its bus
/// interface and has neither.
#[test]
fn quux_has_a_memory_port_and_no_bus_interface() {
    let quux = Rtl::new(reading(Geometry::QUUX, &[0o1000]));
    assert!(quux.busint().is_none(), "no bus interface");
    assert_eq!(quux.cache().map(|c| c.config), Some(CacheConfig::with_words(4096)));
    assert_eq!(quux.memory_timing(), Some(MemoryTiming::NOMINAL));
    assert_eq!(MemoryTiming::NOMINAL, MemoryTiming { read_ns: 380, write_ns: 290 });
    let cadr = Rtl::new(reading(Geometry::CADR, &[0o1000]));
    assert!(cadr.busint().is_some(), "the CADR's bus interface");
    assert!(cadr.cache().is_none() && cadr.memory_timing().is_none());
}

/// **A miss is a line fill at the nominal time, a hit the cache's**: the
/// word read first misses and waits 380 ns for its line; the next word of
/// the line hits and waits 20. Both are main memory's words.
#[test]
fn a_miss_fills_its_line_at_the_nominal_time() {
    let mut e = Rtl::new(reading(Geometry::QUUX, &[0o1000, 0o1001]));
    e.boot();
    let mut at = Vec::new();
    for _ in 0..4000 {
        e.step().unwrap();
        at.push(e.ns());
    }
    let m = e.machine();
    assert_eq!([m.amem[0o200], m.amem[0o201]], [0o1001000, 0o1001001]);
    let c = e.cache().unwrap();
    assert_eq!((c.hits, c.misses), (1, 1), "a miss, then a hit in its line");
    // The microcycles ended 40 ns apart but where one waited for MD, in
    // whole microcycles: the miss's line fill, and the hit's 20 ns.
    let waits: Vec<u64> = at.windows(2).map(|w| w[1] - w[0]).filter(|&d| d > 40).collect();
    assert_eq!(waits.len(), 2, "the miss's and the hit's: {waits:?}");
    assert!((380..=380 + 80).contains(&waits[0]), "the line fill: {waits:?}");
    assert!(waits[1] <= 80, "the hit, a microcycle at most: {waits:?}");
}

/// **A device register is never cached**: two reads of the keyboard's data word take
/// two key words out of the FIFO, and the cache sees neither.
#[test]
fn a_device_register_is_never_cached() {
    use muir::quux_input::KeyboardMouse;
    let data = 0o17377121;
    let mut m = reading(Geometry::QUUX, &[data, data]);
    m.quux_input.press(0o101);
    m.quux_input.press(0o102);
    let mut e = Rtl::new(m);
    run(&mut e);
    let m = e.machine();
    assert_eq!([m.amem[0o200], m.amem[0o201]], [0o101, 0o102], "each read reached the FIFO");
    let c = e.cache().unwrap();
    assert_eq!(c.hits + c.misses, 0, "no lookup for a register");
}

/// **Past main memory's end is nothing, and nothing is cached there**: a
/// write and a read-back at the first word past it both fail with the NXM bit,
/// and the read gives 0, not the word written --- what the microcode's
/// memory-size probe (`MEM-SIZE-LOOP`, `uc-cold-disk.lisp`) relies on. On
/// both engines.
#[test]
fn past_main_memory_s_end_nothing_reads_back() {
    let end = muir::machine::MAIN_WORDS as u32;
    let prom = [
        Insn::new(ALU | SETA | a_src(0o100) | MD),
        Insn::new(ALU | SETM | m_src(1) | START_WRITE),
        filler(),
        filler(),
        Insn::new(ALU | SETM | m_src(1) | START_READ),
        filler(),
        Insn::new(ALU | SETM | SRC_MD | a_dest(0o200)),
        Insn::new(JUMP | target(7) | ALWAYS | N),
    ];
    let setup = || {
        let mut m = machine(Geometry::QUUX, &prom, &[end]);
        m.amem[0o100] = 0o37;
        m.amem[0o200] = 0o525252;
        m
    };
    let mut r = Rtl::new(setup());
    run(&mut r);
    let mut e = Micro::new(setup());
    run(&mut e);
    for (name, m) in [("rtl", r.machine()), ("micro", e.machine())] {
        assert_eq!(m.amem[0o200], 0, "{name}: the write did not read back");
        assert_ne!(m.bus_error & bus_error::XBUS_NXM, 0, "{name}: the NXM bit");
        assert_eq!(m.bus_error & bus_error::UNIBUS_NXM, 0, "{name}");
    }
    let c = r.cache().unwrap();
    assert_eq!(c.hits + c.misses, 0, "no lookup past main memory");
    assert!(!c.holds(end));
}

/// **After the disk writes main memory, no read hits the old word**: a line
/// held, the disk's transfer changing its word, the next read misses and
/// reads the disk's word.
#[test]
fn after_the_disk_writes_no_read_hits_the_old_word() {
    let mut e = Rtl::new(reading(Geometry::QUUX, &[0o1000, 0o1000]));
    e.boot();
    // Up to the first read's word in A 200: its line is held.
    for _ in 0..200 {
        if e.machine().amem[0o200] == 0o1001000 {
            break;
        }
        e.step().unwrap();
    }
    assert_eq!(e.machine().amem[0o200], 0o1001000, "the first read");
    assert!(e.cache().unwrap().holds(0o1000));
    // The disk's transfer, as block-disk makes it: the word, and the flag.
    e.machine_mut().main[0o1000] = 0o7654321;
    e.machine_mut().dma_written = true;
    for _ in 0..200 {
        e.step().unwrap();
    }
    assert_eq!(e.machine().amem[0o201], 0o7654321, "the disk's word");
    assert_eq!(e.cache().unwrap().misses, 2, "the second read missed");
}

/// **A checkpoint on QUUX has no bus interface in it**: saved mid-run and
/// resumed, the run goes on as the one saved, cache and all.
#[test]
fn a_checkpoint_keeps_the_memory_port() {
    use muir::checkpoint::{Reader, Writer};
    let reads: Vec<u32> = (0..6).map(|k| 0o1000 + 5 * k).collect();
    let mut e = Rtl::new(reading(Geometry::QUUX, &reads));
    e.boot();
    for _ in 0..30 {
        e.step().unwrap();
    }
    let mut w = Writer::new();
    e.save(&mut w);
    let body = w.finish();
    let mut back = Rtl::new(reading(Geometry::QUUX, &reads));
    back.load(&mut Reader::new(&body)).unwrap();
    for _ in 0..300 {
        e.step().unwrap();
        back.step().unwrap();
    }
    assert_eq!(e.ns(), back.ns());
    assert_eq!(e.machine().amem[0o200..0o206], back.machine().amem[0o200..0o206]);
    let (a, b) = (e.cache().unwrap(), back.cache().unwrap());
    assert_eq!((a.hits, a.misses), (b.hits, b.misses));
}

/// One bus cycle as `rtl` shows it from the step that took it: the edge it
/// was taken at, when it was answered and when acknowledged; how many
/// later step boundaries fell before the acknowledgement, and whether
/// `-MEMGRANT` was low at all of them.
#[derive(Debug)]
struct Cycle {
    edge: u64,
    answered: u64,
    ack: u64,
    inside: u32,
    held: bool,
}

/// Runs `e` to the end of its program in steps of at most 20 ns, so that a
/// hang is seen from inside, noting each bus cycle as the step that took it
/// left it.
fn cycles(e: &mut Rtl) -> Vec<Cycle> {
    e.boot();
    let mut out: Vec<Cycle> = Vec::new();
    let mut was = false;
    for _ in 0..4000 {
        e.step_until(e.ns() + 20).unwrap();
        let granted = e.bus_granted();
        assert_eq!(granted, e.bus_ack_at().is_some(), "the acknowledgement is due while granted");
        assert_eq!(granted, e.bus_answered_at().is_some());
        if granted && !was {
            out.push(Cycle {
                edge: e.ns(),
                answered: e.bus_answered_at().unwrap(),
                ack: e.bus_ack_at().unwrap(),
                inside: 0,
                held: true,
            });
        } else if let Some(c) = out.last_mut()
            && e.ns() < c.ack
        {
            c.inside += 1;
            c.held &= granted && e.bus_ack_at() == Some(c.ack);
        }
        was = granted;
    }
    out
}

/// **`rtl` shows the running cycle's acknowledgement and grant on QUUX**:
/// `bus_granted` is `-MEMGRANT` low, `bus_ack_at` when `-MEMACK` is due,
/// forwarded from the memory port. A read miss is acknowledged as it is
/// answered, a line fill (380 ns) after the edge that took it; a hit the
/// hit time (20 ns) after; a device register is answered at the edge and
/// acknowledged a microcycle (40 ns at K=4) later; an empty address is
/// answered and acknowledged at the edge.
#[test]
fn rtl_shows_the_memory_port_s_acknowledgement_and_grant() {
    const REGISTER: u32 = 0o17377000;
    const EMPTY: u32 = 0o17377400;
    let mut e = Rtl::new(reading(Geometry::QUUX, &[0o1000, 0o1001, REGISTER, EMPTY]));
    let cs = cycles(&mut e);
    assert_eq!(cs.len(), 4, "{cs:?}");
    let m = e.machine();
    assert_eq!(m.amem[0o200..0o204], [0o1001000, 0o1001001, Geometry::QUUX.machine_id.unwrap(), 0]);
    let (miss, hit, register, empty) = (&cs[0], &cs[1], &cs[2], &cs[3]);
    for c in &cs {
        assert!(c.held, "granted until acknowledged: {c:?}");
    }
    assert!(miss.inside > 0, "the line fill's hang seen from inside: {miss:?}");
    assert_eq!((miss.answered - miss.edge, miss.ack - miss.edge), (380, 380), "miss: {miss:?}");
    assert_eq!((hit.answered - hit.edge, hit.ack - hit.edge), (20, 20), "hit: {hit:?}");
    assert_eq!(register.answered, register.edge, "register: {register:?}");
    assert_eq!(register.ack, register.answered + 40, "register: {register:?}");
    assert_eq!((empty.answered, empty.ack), (empty.edge, empty.edge), "empty: {empty:?}");
}

/// **On the CADR the same accessors are the bus interface's**: at every
/// step, `bus_ack_at` and `bus_granted` are [`muir::busint::Busint`]'s own.
#[test]
fn rtl_shows_the_bus_interface_s_acknowledgement_and_grant() {
    let mut e = Rtl::new(reading(Geometry::CADR, &[0o1000, 0o1001]));
    e.boot();
    let mut seen = 0;
    for _ in 0..4000 {
        e.step().unwrap();
        let b = e.busint().unwrap();
        assert_eq!(e.bus_ack_at(), b.ack_at());
        assert_eq!(e.bus_granted(), b.granted());
        seen += e.bus_granted() as u32;
    }
    assert!(seen > 0, "a granted cycle was seen");
}
