// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Writes that land inside the microcycle that reads the same memory, and
//! writes whose microcycle is held, on the board, `rtl` and `micro`.
//!
//! Each test is a small boot-PROM program, run on `chip` (MIT's netlist),
//! `rtl` and `micro` from the same memories, and the end states compared.
//! The programs end in a jump to themselves, so the end state does not
//! depend on how many microcycles each engine is given past that point.

mod support;

use muir::cable::FarEnd;
use muir::chip::{Chip, MEMS, Ram};
use muir::clock::{Behavioral, Clock};
use muir::engine::Engine;
use muir::isa::Insn;
use muir::isa::asm::*;
use muir::machine::Machine;
use muir::part::Level;

/// `v` into M memory `r` (and the A memory word it shadows): zero, then one
/// doubling a bit, with the bit as the carry in. Nothing but the ALU.
fn constant(v: u32, r: u64, out: &mut Vec<Insn>) {
    out.push(Insn::new(ALU | SETZ | m_dest(r)));
    for b in (0..32).rev() {
        let c = if (v >> b) & 1 != 0 { CARRY_IN } else { 0 };
        out.push(Insn::new(ALU | M_PLUS_M | c | m_src(r) | a_src(r) | m_dest(r)));
    }
}

/// `M[to] <- M[from]`.
fn copy(from: u64, to: u64) -> Insn {
    Insn::new(ALU | SETM | m_src(from) | a_src(3) | m_dest(to))
}

/// A jump to itself.
fn halt_here(at: usize) -> Insn {
    Insn::new(JUMP | target(at as u64) | ALWAYS)
}

/// Functional destination 23, `VMA-WRITE-MAP` (`IR<23:19>`), also into M 37.
const WRITE_MAP: u64 = (0o23 << 19) | (0o37 << 14);
/// Functional source 11, `MAP(MD)`: the map's output for `MAPI`.
const SRC_MAP: u64 = src(0o11);

/// What a run leaves behind, read off whichever engine ran it.
#[derive(Debug, PartialEq, Eq, Clone)]
struct End {
    mmem: Vec<u32>,
    dmem: Vec<u32>,
    l2: Vec<u32>,
    pdl: Vec<u32>,
    spc: Vec<u32>,
    spcptr: u8,
}

/// The PC in each microcycle the processor ran, for the failure message.
type Trace = Vec<u16>;

/// One generator cycle of the board, as `tests/chip.rs` steps it: whether
/// the cpu clock ran in it, and how many times each of `-WP1`..`-WP5` went
/// low in it.
fn generator_cycle(
    c: &mut Chip,
    far: &mut FarEnd,
    clk: &mut Behavioral,
    clk0: muir::netlist::NetId,
    wps: &[muir::netlist::NetId],
    pulses: &mut [u32],
) -> bool {
    let from = clk.time_ns();
    let mut ran = false;
    let mut was: Vec<Level> = wps.iter().map(|&w| c.net(w)).collect();
    loop {
        far.tick_with(c, clk);
        ran |= c.net(clk0) == Level::High;
        for (k, &w) in wps.iter().enumerate() {
            let now = c.net(w);
            if now == Level::Low && was[k] != Level::Low {
                pulses[k] += 1;
            }
            was[k] = now;
        }
        if clk.phase_ns() == 0 && clk.time_ns() > from {
            return ran;
        }
        assert!(clk.time_ns() - from < 60_000, "the generator has not come round");
    }
}

/// What the board did, generator cycle by generator cycle: the PC, whether
/// the cpu clock ran, and the write pulses that fired.
#[derive(Debug, Clone)]
struct Cycle {
    pc: u16,
    ran: bool,
    pulses: [u32; 5],
    /// Nanoseconds from this generator cycle's start to the next's: longer
    /// than the others under `-HANG`, which stops the generator.
    ns: u64,
}

/// The program and memories of `m` on the board --- the PROM as a
/// programming image, every scratchpad and both map levels stored into the
/// RAM cells, `main` put on the memory boards --- booted and run for
/// `cycles` generator cycles.
fn on_chip(m: &Machine, main: &[(u32, u32)], cycles: usize) -> (End, Vec<Cycle>) {
    let n = support::netlists();
    let (mut c, mut clk, mut far) = support::chip(&n);
    let image: Vec<u64> = m.prom.iter().map(|&i| muir::prom::programming(i)).collect();
    c.power_on();
    c.load_prom(&n.cpu, &image);
    let ram = |k: usize, c: &Chip| Ram::new(c, &n.cpu, &MEMS[k]);
    let (a, mm, pdl, spc, dm, l1, l2) =
        (ram(0, &c), ram(1, &c), ram(2, &c), ram(3, &c), ram(4, &c), ram(5, &c), ram(6, &c));
    for (k, &v) in m.amem.iter().enumerate() {
        a.store(&mut c, k, v);
    }
    for (k, &v) in m.mmem.iter().enumerate() {
        mm.store(&mut c, k, v);
    }
    for (k, &v) in m.pdl[..pdl.len()].iter().enumerate() {
        pdl.store(&mut c, k, v);
    }
    for (k, &v) in m.spc.iter().enumerate() {
        spc.store(&mut c, k, v & 0o1777777);
    }
    for (k, &v) in m.dmem.iter().enumerate() {
        dm.store(&mut c, k, v & 0o377777);
    }
    for (k, &v) in m.l1_map.iter().enumerate() {
        l1.store(&mut c, k, v & 0o37);
    }
    for (k, &v) in m.l2_map[..l2.len()].iter().enumerate() {
        l2.store(&mut c, k, v & 0o77777777);
    }
    for &(p, w) in main {
        far.xbus.poke(p, w);
    }
    c.settle();
    far.join(&mut c, clk.time_ns());
    let boot = n.cpu.by_name_id("-BOOT2").unwrap();
    c.set_net(boot, Level::Low);
    c.settle();
    for _ in 0..20 {
        c.tick(&mut clk);
    }
    c.set_net(boot, Level::High);
    let mut skipped = 0;
    while c.bus(&n.cpu, "PC", 14) == 0 && skipped < 40 {
        c.microcycle(&mut clk);
        skipped += 1;
    }
    let clk0 = n.cpu.by_name_id("-CLK0").unwrap();
    let wps: Vec<_> = ["-WP1", "-WP2", "-WP3", "-WP4", "-WP5"]
        .iter()
        .map(|w| n.cpu.by_name_id(w).unwrap())
        .collect();
    let mut log = Vec::new();
    for _ in 0..cycles {
        let pc = c.bus(&n.cpu, "PC", 14) as u16;
        let mut pulses = [0; 5];
        let t0 = clk.time_ns();
        let ran = generator_cycle(&mut c, &mut far, &mut clk, clk0, &wps, &mut pulses);
        log.push(Cycle { pc, ran, pulses, ns: clk.time_ns() - t0 });
    }
    let (mm, dm, l2, pdl, spc) = (ram(1, &c), ram(4, &c), ram(6, &c), ram(2, &c), ram(3, &c));
    let end = End {
        mmem: (0..32).map(|k| mm.word(&c, k)).collect(),
        dmem: (0..dm.len()).map(|k| dm.word(&c, k)).collect(),
        l2: (0..l2.len()).map(|k| l2.word(&c, k)).collect(),
        pdl: (0..pdl.len()).map(|k| pdl.word(&c, k)).collect(),
        spc: (0..32).map(|k| spc.word(&c, k) & 0o1777777).collect(),
        spcptr: c.bus(&n.cpu, "SPCPTR", 5) as u8,
    };
    (end, log)
}

/// The same on an engine, `steps` microcycles.
fn on_engine<E: Engine>(
    new: fn(Machine) -> E,
    boot: fn(&mut E),
    m: &Machine,
    main: &[(u32, u32)],
    steps: usize,
) -> (End, Trace) {
    let mut m = m.clone();
    for &(p, w) in main {
        m.main[p as usize] = w;
    }
    let mut e = new(m);
    boot(&mut e);
    let mut trace = Vec::new();
    for _ in 0..steps {
        trace.push(e.pc());
        e.step().unwrap();
    }
    let mm = e.machine();
    let end = End {
        mmem: mm.mmem.to_vec(),
        dmem: mm.dmem.iter().map(|&w| w & 0o377777).collect(),
        l2: mm.l2_map[..1024].iter().map(|&w| w & 0o77777777).collect(),
        pdl: mm.pdl[..1024].to_vec(),
        spc: mm.spc.iter().map(|&w| w & 0o1777777).collect(),
        spcptr: mm.spcptr,
    };
    (end, trace)
}

fn rtl(m: &Machine, main: &[(u32, u32)], steps: usize) -> (End, Trace) {
    on_engine(muir::rtl::Rtl::new, muir::rtl::Rtl::boot, m, main, steps)
}

fn micro(m: &Machine, main: &[(u32, u32)], steps: usize) -> (End, Trace) {
    on_engine(muir::micro::Micro::new, muir::micro::Micro::boot, m, main, steps)
}

/// PCs in octal, for a failure message.
fn octal(pcs: &[u16]) -> String {
    pcs.iter().map(|p| format!("{p:o}")).collect::<Vec<_>>().join(" ")
}

/// The PCs of the microcycles the board ran.
fn ran_pcs(log: &[Cycle]) -> Vec<u16> {
    log.iter().filter(|c| c.ran).map(|c| c.pc).collect()
}

/// The three engines' ends, and the board's generator cycles and `rtl`'s PCs.
struct Three {
    chip: End,
    log: Vec<Cycle>,
    rtl: End,
    rtl_pcs: Trace,
    micro: End,
}

fn run3(m: &Machine, main: &[(u32, u32)], cycles: usize) -> Three {
    let (chip, log) = on_chip(m, main, cycles);
    let (rtl, rtl_pcs) = rtl(m, main, cycles);
    let (micro, _) = micro(m, main, cycles);
    Three { chip, log, rtl, rtl_pcs, micro }
}

fn program(p: Vec<Insn>) -> Machine {
    let mut p = p;
    p.resize(512, filler());
    let mut m = Machine::new();
    m.load_prom(&p);
    m
}

// --- Q3: a dispatch memory write in the instruction that also pops ---------

/// The dispatch memory word written and read.
const D: u64 = 0o1200;
/// The dispatch word's `R`, `P` and `N` bits, `DR`, `DP`, `DN` (the data
/// nets of pages DRAM0-2 as `muir::chip::MEMS` reads them).
const DR: u32 = 1 << 16;
const DP: u32 = 1 << 15;
const DN: u32 = 1 << 14;
/// What `M5` ends holding: the subroutine returned, or it went to the old
/// word's `DPC`, or to the new word's.
const RETURNED: u32 = 0o1111;
const AT_OLD_DPC: u32 = 0o2222;
const AT_NEW_DPC: u32 = 0o3333;
/// Where the subroutine is, and the two words' `DPC`.
const SUB: usize = 0o400;
const OLD_DPC: u32 = 0o440;
const NEW_DPC: u32 = 0o460;

/// The program: the dispatch word at [`D`] set to `old` by a plain write, a
/// call to [`SUB`], and there **one** instruction that both writes `new`
/// over that word (`DISPATCH` with `IR<11:10>` = 2, `DISPWR`) and has
/// `POPJ` (`IR<42>`), the write's address being the dispatch's.
///
/// `IGNPOPJ` is `DISPATCH AND NOT DR` on page CONTRL. With `DR` set the
/// `POPJ` is taken and the caller's return sets `M5` to [`RETURNED`]. With
/// it clear the pop is not taken, `PCS1` stays up and `PCS0` goes down
/// (`POPJ` is in `PCS0`'s NOR and, suppressed, not in `PCS1`'s), which
/// selects `DPC`: the machine goes to the dispatch word's PC field, where
/// [`OLD_DPC`] sets [`AT_OLD_DPC`] and [`NEW_DPC`] sets [`AT_NEW_DPC`]. So
/// `M5` and the stack pointer say both which `R` and which `DPC` were used.
fn popj_program(old: u32, new: u32) -> Machine {
    let mut p = vec![filler()];
    constant(old, 1, &mut p);
    constant(new, 2, &mut p);
    constant(RETURNED, 6, &mut p);
    constant(AT_OLD_DPC, 7, &mut p);
    constant(AT_NEW_DPC, 8, &mut p);
    p.push(Insn::new(ALU | SETZ | m_dest(5)));
    p.push(Insn::new(DISPATCH | DMEM_WRITE | a_src(1) | d_addr(D)));
    p.push(filler());
    p.push(filler());
    p.push(Insn::new(JUMP | P | ALWAYS | target(SUB as u64)));
    // The call's delay slot runs before the subroutine and sets nothing; the
    // return comes to the instruction after it.
    p.push(filler());
    p.push(copy(6, 5));
    let here = p.len();
    p.push(halt_here(here));
    assert!(p.len() < SUB);
    p.resize(512, filler());
    p[SUB] = Insn::new(DISPATCH | DMEM_WRITE | POPJ | a_src(2) | d_addr(D));
    for (at, from) in [(OLD_DPC as usize, 7), (NEW_DPC as usize, 8)] {
        p[at] = copy(from, 5);
        p[at + 1] = halt_here(at + 1);
    }
    program(p)
}

const POPJ_CYCLES: usize = 240;

/// `(M5, SPCPTR)` on the board, `rtl` and `micro` for `old` then `new`, the
/// new word being in place afterwards on all three.
fn popj_case(old: u32, new: u32) -> [(u32, u8); 3] {
    let t = run3(&popj_program(old, new), &[], POPJ_CYCLES);
    let tail = |t: &[u16]| octal(&t[t.len().saturating_sub(6)..]);
    eprintln!(
        "old {old:6o} new {new:6o}: chip M5 {:o} SPCPTR {} [{}]; rtl M5 {:o} SPCPTR {} [{}]; micro M5 {:o} SPCPTR {}",
        t.chip.mmem[5],
        t.chip.spcptr,
        tail(&ran_pcs(&t.log)),
        t.rtl.mmem[5],
        t.rtl.spcptr,
        tail(&t.rtl_pcs),
        t.micro.mmem[5],
        t.micro.spcptr
    );
    for (name, e) in [("chip", &t.chip), ("rtl", &t.rtl), ("micro", &t.micro)] {
        assert_eq!(e.dmem[D as usize], new, "{name}: the new word is written");
    }
    [
        (t.chip.mmem[5], t.chip.spcptr),
        (t.rtl.mmem[5], t.rtl.spcptr),
        (t.micro.mmem[5], t.micro.spcptr),
    ]
}

/// The four cases and what the board does in each: old and new word, and
/// the board's `(M5, SPCPTR)`. The stack pointer starts at 0 and the call
/// takes it to 1; a pop takes it back.
const POPJ_CASES: [(u32, u32, (u32, u8)); 4] = [
    (DR | OLD_DPC, NEW_DPC, (AT_NEW_DPC, 1)),
    (OLD_DPC, DR | NEW_DPC, (RETURNED, 0)),
    (OLD_DPC, NEW_DPC, (AT_NEW_DPC, 1)),
    (DR | DP | OLD_DPC, DR | DN | NEW_DPC, (RETURNED, 0)),
];

/// **On `chip`, a `DISPATCH` that writes the dispatch memory and has `POPJ`
/// takes `R`, and the `DPC` it goes to when `R` is clear, from the word it
/// is writing** --- the new word, not the one the RAM held when the
/// microcycle began. This is the netlist model's answer and not a fact
/// about the board: it rests on the clock ending the write pulse before the
/// edge ([`on_chip_the_dispatch_ram_floats_during_its_write_and_the_pulse_ends_at_the_edge`]),
/// where the board's pulse runs ten nanoseconds past it and the RAM's
/// output floats while written, a race. **Unverified** on a CADR. QUUX
/// defines the old word ([`rtl_and_micro_take_the_old_word_as_quux_defines`]).
///
/// The dispatch memory is 93425A 1K x 1 RAMs (pages DRAM0-DRAM2), written
/// by `-DWEA`/`-DWEB`, `NAND(WP2, DISPWR)` at DRAM0 2F03: live `DISPWR`, so
/// the pulse is in this instruction's own microcycle, and the address,
/// `DADR`, is this instruction's throughout. The 93425A's output is high
/// impedance while written and shows the cell it addresses after, so by the
/// clock edge that ends the microcycle --- the edge that registers `PC`
/// and steps the stack pointer --- `DR` and `DPC` are the new word's.
///
/// `P` and `N` do nothing here: `DISPENB` is `DISPATCH AND NOT DISPWR`, and
/// the fourth case, `R` set in both and `P`, `N` and `DPC` all different,
/// returns.
#[test]
fn on_chip_a_popj_that_writes_its_own_dispatch_word_uses_the_new_word() {
    for (old, new, want) in POPJ_CASES {
        let [chip, _, _] = popj_case(old, new);
        assert_eq!(chip, want, "chip, old {old:o} new {new:o}");
    }
}

/// **QUUX defines the word a `POPJ` in a dispatch write sees as the old
/// one**: `R` and `DPC` from the word standing before the write, on `rtl`
/// (`Rtl::read_phase` reads it, `Rtl::write_phase` writes after) and on
/// `micro` (`Micro::dispatch`). With `R` set in the old word it returns;
/// with it clear it goes to the old word's `DPC` and pops nothing.
#[test]
fn rtl_and_micro_take_the_old_word_as_quux_defines() {
    for (old, new, _) in POPJ_CASES {
        let want = if old & DR != 0 { (RETURNED, 0) } else { (AT_OLD_DPC, 1) };
        let [_, rtl, micro] = popj_case(old, new);
        assert_eq!(rtl, want, "rtl, old {old:o} new {new:o}");
        assert_eq!(micro, want, "micro, old {old:o} new {new:o}");
    }
}

/// **What the board's answer rests on**, measured on `chip` in the
/// `DISPWR`-with-`POPJ` microcycle: while `-DWEA` is low the dispatch RAM's
/// `DR` output is high impedance (the 93425A's output is off while written),
/// and the pulse ends at the same nanosecond as the clock edge that ends the
/// microcycle, ordered before it. That order is `src/clock.rs`'s: it cuts
/// `TPWP` at the cycle boundary (`WP_OFF_NS` limited to
/// `RESTART_AFTER_READ_NS`) and schedules the end first. On the board the
/// pulse runs to `-TPW70`, ten nanoseconds past `-TPW60`/`-TPDONE` where the
/// next `-TPR0` starts, and the 93425A needs time after `-WE` rises before
/// its output is valid; so on the physical machine the registers at that
/// edge may see the RAM's output still floating. **Unverified**: settled
/// only by gate-level timing (the 93425A's write recovery against the
/// `-TPR0` to register-clock path), which `chip` does not model.
#[test]
fn on_chip_the_dispatch_ram_floats_during_its_write_and_the_pulse_ends_at_the_edge() {
    let m = popj_program(DR | OLD_DPC, NEW_DPC);
    let n = support::netlists();
    let (mut c, mut clk, mut far) = support::chip(&n);
    let image: Vec<u64> = m.prom.iter().map(|&i| muir::prom::programming(i)).collect();
    c.power_on();
    c.load_prom(&n.cpu, &image);
    c.settle();
    far.join(&mut c, clk.time_ns());
    let boot = n.cpu.by_name_id("-BOOT2").unwrap();
    c.set_net(boot, Level::Low);
    c.settle();
    for _ in 0..20 {
        c.tick(&mut clk);
    }
    c.set_net(boot, Level::High);
    let net = |s: &str| n.cpu.by_name_id(s).unwrap();
    let (clk0, dwea, dr) = (net("-CLK0"), net("-DWEA"), net("DR"));
    // (time, -CLK0, -DWEA, DR) at every tick while `IR` holds the
    // subroutine's instruction, which is while `PC` is SUB + 1.
    let mut seen = Vec::new();
    for _ in 0..400_000 {
        far.tick_with(&mut c, &mut clk);
        let pc = c.bus(&n.cpu, "PC", 14) as usize;
        if pc == SUB + 1 {
            seen.push((clk.time_ns(), c.net(clk0), c.net(dwea), c.net(dr)));
        } else if !seen.is_empty() {
            seen.push((clk.time_ns(), c.net(clk0), c.net(dwea), c.net(dr)));
            break;
        }
    }
    let during: Vec<_> = seen.iter().filter(|s| s.2 == Level::Low).collect();
    assert!(!during.is_empty(), "the dispatch write pulse fired");
    assert!(during.iter().all(|s| s.3 == Level::Z), "DR floats while written: {during:?}");
    let end = seen.iter().position(|s| s.2 == Level::Low).unwrap() + during.len();
    let (after, edge) = (seen[end], seen[end + 1]);
    assert_eq!(after.2, Level::High, "the pulse has ended");
    assert_eq!(after.1, Level::High, "before the edge: -CLK0 still high");
    assert_eq!(after.3, Level::Low, "DR shows the new word's R, clear");
    assert_eq!(edge.1, Level::Low, "then the edge");
    assert_eq!(after.0, edge.0, "at the same nanosecond");
}

// --- Q3b: a map write, and the instruction straight after it ---------------

/// `MD` for the map writes: level-1 index 100, level-2 low bits 3. Level 1
/// holds 0 there, so level 2 is addressed at 3. Its low three bits are 2.
const MAP_MD: u32 = (0o100 << 13) | (3 << 8) | 2;
/// Level 2's word 3 before and after: bit 18 clear before and set after,
/// for the dispatch on it.
const OLD_L2: u32 = 0o1234567 & !(1 << 18);
const NEW_L2: u32 = 0o7654321;
/// The store: `VMA<25>`, level 2 only, and the new word.
const MAP_STORE: u32 = (1 << 25) | NEW_L2;

/// `MD` set to [`MAP_MD`], `VMA-WRITE-MAP` of [`MAP_STORE`], and `then`
/// straight after it; level 2's word 3 holds [`OLD_L2`] to begin with.
fn map_write_then(then: Vec<Insn>) -> Machine {
    assert_eq!(OLD_L2 & (1 << 18), 0);
    assert_ne!(NEW_L2 & (1 << 18), 0);
    let mut p = vec![filler()];
    constant(MAP_MD, 1, &mut p);
    constant(MAP_STORE, 2, &mut p);
    constant(AT_OLD_DPC, 7, &mut p);
    constant(AT_NEW_DPC, 8, &mut p);
    p.push(Insn::new(ALU | SETZ | m_dest(5)));
    p.push(Insn::new(ALU | SETM | m_src(1) | a_src(3) | MD));
    p.push(filler());
    p.push(filler());
    p.push(Insn::new(ALU | SETM | m_src(2) | a_src(3) | WRITE_MAP));
    p.extend(then);
    let here = p.len();
    p.push(halt_here(here));
    assert!(p.len() < OLD_DPC as usize);
    p.resize(512, filler());
    for (at, from) in [(OLD_DPC as usize, 7), (NEW_DPC as usize, 8)] {
        p[at] = copy(from, 5);
        p[at + 1] = halt_here(at + 1);
    }
    let mut m = program(p);
    m.l2_map[3] = OLD_L2;
    m
}

/// Reads `MAP(MD)` into `M10` in the instruction after the store and into
/// `M11` in the one after that.
fn map_source_after_a_map_write() -> Three {
    let m = map_write_then(vec![
        Insn::new(ALU | SETM | SRC_MAP | a_src(3) | m_dest(10)),
        Insn::new(ALU | SETM | SRC_MAP | a_src(3) | m_dest(11)),
    ]);
    let t = run3(&m, &[], 240);
    for (name, e) in [("chip", &t.chip), ("rtl", &t.rtl), ("micro", &t.micro)] {
        eprintln!("{name}: M10 {:o} M11 {:o} l2[3] {:o}", e.mmem[10], e.mmem[11], e.l2[3]);
    }
    t
}

/// **On `chip`, the instruction right after a map write reads the new word
/// from `MAP(MD)`** --- the netlist model's answer, resting on the same
/// pulse timing as the dispatch RAM's, and **unverified** on a CADR. `WMAPD` puts the level-2 pulse, `-VM1WPA/B`
/// = `NAND(MAPWR1D, WP1B)` at VCTL2 1D07, in the next microcycle's write
/// phase, and that microcycle is the reader; functional source 11 goes
/// through VMEMDR onto `MF` and the M bus unlatched (MLATCH's 74S373s latch
/// only M memory), so the word the edge registers is the one the RAM shows
/// after the pulse. Same caveat as the dispatch RAM:
/// [`on_chip_the_dispatch_ram_floats_during_its_write_and_the_pulse_ends_at_the_edge`].
#[test]
fn on_chip_the_instruction_after_a_map_write_reads_the_new_map_word() {
    let t = map_source_after_a_map_write();
    assert_eq!(t.chip.l2[3], NEW_L2, "chip: the map is written");
    assert_eq!(
        t.chip.mmem[10] & 0o77777777,
        NEW_L2,
        "chip: the next instruction reads the new word"
    );
    assert_eq!(t.chip.mmem[11], t.chip.mmem[10], "chip: and so does the one after");
}

/// **QUUX defines it as the old word**: the instruction right after the
/// store reads the level-2 word from before the write, on `rtl` and on
/// `micro`, and the one after that reads the new one.
#[test]
fn rtl_and_micro_read_the_old_map_word_after_a_map_write_as_quux_defines() {
    let t = map_source_after_a_map_write();
    for (name, e) in [("rtl", &t.rtl), ("micro", &t.micro)] {
        assert_eq!(e.mmem[10] & 0o77777777, OLD_L2, "{name}: the next instruction");
        assert_eq!(e.mmem[11], t.chip.mmem[11], "{name}: the one after");
    }
}

/// A dispatch on map bit 18 in the instruction right after the store:
/// dispatch words 1300 and 1301 go to [`OLD_DPC`] and [`NEW_DPC`], and
/// `IR<8>` makes `DADR<0>` `VMO18` (the 74S64s at DSPCTL 2F24, 2F05 and
/// 2F23). The old level-2 word has bit 18 clear, the new one set.
fn map_dispatch_after_a_map_write() -> Three {
    const E: u64 = 0o1300;
    let mut m = map_write_then(vec![
        Insn::new(DISPATCH | (1 << 8) | a_src(3) | m_src(3) | d_addr(E)),
        filler(),
    ]);
    m.dmem[E as usize] = OLD_DPC;
    m.dmem[E as usize + 1] = NEW_DPC;
    let t = run3(&m, &[], 240);
    for (name, e) in [("chip", &t.chip), ("rtl", &t.rtl), ("micro", &t.micro)] {
        eprintln!("{name}: M5 {:o}, l2[3] {:o}", e.mmem[5], e.l2[3]);
    }
    t
}

/// **On `chip` the dispatch takes the new word's bit 18**, the netlist
/// model's answer (see above).
#[test]
fn on_chip_a_dispatch_on_a_map_bit_after_a_map_write_takes_the_new_word() {
    let t = map_dispatch_after_a_map_write();
    assert_eq!(t.chip.l2[3], NEW_L2, "chip: the map is written");
    assert_eq!(t.chip.mmem[5], AT_NEW_DPC, "chip: dispatched on the new word's bit 18");
}

/// **QUUX defines it as the old word's bit 18**, on `rtl` and `micro`.
#[test]
fn rtl_and_micro_dispatch_on_the_old_map_bit_as_quux_defines() {
    let t = map_dispatch_after_a_map_write();
    assert_eq!((t.rtl.mmem[5], t.micro.mmem[5]), (AT_OLD_DPC, AT_OLD_DPC));
}

// --- Q4: writes pending across a held microcycle ----------------------------

/// Virtual word `(1 << 8) | 5`: level-2 entry 1, physical page 100.
const VADDR: u32 = (1 << 8) | 5;
const PHYS: u32 = (0o100 << 8) | 5;
/// The word the read brings into `MD`: level-1 index 100 and level-2 low
/// bits 7 for a map write, low three bits 5 for a dispatch.
const READ_WORD: u32 = (0o100 << 13) | (7 << 8) | 5;
/// `MD` before the read lands: level-2 low bits 3, low three bits 2.
const MD_BEFORE: u32 = MAP_MD;
const HELD_CYCLES: usize = 400;
/// Functional destinations 20 (`VMA`), 14 (PDL pointer), 10 (PDL at the
/// pointer), 15 (SPC push), each with M 37.
const PDL_POINTER: u64 = (0o14 << 19) | (0o37 << 14);
const PDL_TOP: u64 = (0o10 << 19) | (0o37 << 14);
const SPC_PUSH: u64 = (0o15 << 19) | (0o37 << 14);

/// `M1` = [`MD_BEFORE`] into `MD`, `M12` = [`VADDR`], the `(value,
/// register)` constants of `setup`, the PDL pointer set to 20, then
/// `VMA-START-READ` of [`VADDR`] and `then`, fillers and a jump to itself.
/// [`PHYS`] holds [`READ_WORD`].
fn read_then(setup: &[(u32, u64)], then: Vec<Insn>) -> Machine {
    let mut p = vec![filler()];
    constant(MD_BEFORE, 1, &mut p);
    constant(VADDR, 12, &mut p);
    constant(0o20, 14, &mut p);
    for &(v, r) in setup {
        constant(v, r, &mut p);
    }
    p.push(Insn::new(ALU | SETM | m_src(1) | a_src(3) | MD));
    p.push(Insn::new(ALU | SETM | m_src(14) | a_src(3) | PDL_POINTER));
    p.push(filler());
    p.push(Insn::new(ALU | SETM | m_src(12) | a_src(3) | START_READ));
    p.extend(then);
    for _ in 0..4 {
        p.push(filler());
    }
    let here = p.len();
    p.push(halt_here(here));
    let mut m = program(p);
    m.l2_map[1] = (1 << 23) | (1 << 22) | 0o100;
    m
}

/// The board's generator cycles in which the cpu clock did not run
/// (`-WAIT`), the write pulses fired in them, and the generator cycles
/// stretched past 220 ns (`-HANG`, which stops the generator itself).
fn holds(log: &[Cycle]) -> (usize, [u32; 5], usize) {
    let mut sum = [0; 5];
    let mut k = 0;
    for c in log.iter().filter(|c| !c.ran) {
        k += 1;
        for (s, p) in sum.iter_mut().zip(c.pulses) {
            *s += p;
        }
    }
    (k, sum, log.iter().filter(|c| c.ns > 220).count())
}

/// **No write pulse fires in a generator cycle `-WAIT` holds, and a late
/// write pending across one lands once, alike on all three.** `TPWP` is
/// `NOR(latch, -MACHRUNA)` at CLOCK2 1C10, `-MACHRUNA` being `NOT MACHRUN`
/// at 1C10 too, and `TPWPIRAM` is gated the same way: so `-WP1..-WP5`, the
/// 7428s at 1C02 and 1C11, are all gated by `MACHRUN`, the 9S42-1 at OLORD1
/// 1A15 that takes `-WAIT`. The generator runs on through a wait; the pulses
/// do not.
///
/// The program: a PDL write (destination 10, at the pointer, 20) or an SPC
/// push (destination 15) right after a `VMA-START-READ`, then a `VMA` store,
/// which `-WAIT` holds (`DESTMEM AND MBUSY.SYNC`) while the read lands in
/// `MD`, then `MD` into `M13`. The PDL or SPC write is pending --- its pulse
/// is in the microcycle after its own --- across the held cycles. Its
/// address is a register and its data `L`, which the stopped cpu clock
/// holds, so even a repeated pulse would land on the same word; what the
/// test holds is that none fired and the engines agree.
///
/// A **map** write cannot be pending across a `-WAIT` on the CADR: the
/// store is `DESTMEM` and waits for `MBUSY.SYNC` itself, and nothing but a
/// memory cycle raises `MBUSY` again before the next instruction. The
/// store's own wait is reached here instead
/// ([`a_map_store_held_by_wait_writes_at_the_md_it_ends_with`]).
#[test]
fn pdl_and_spc_writes_pending_across_a_wait_land_once_alike() {
    for (what, dest) in [("PDL", PDL_TOP), ("SPC", SPC_PUSH)] {
        let m = read_then(
            &[(0o765432, 4)],
            vec![
                Insn::new(ALU | SETM | m_src(4) | a_src(3) | dest),
                Insn::new(ALU | SETM | m_src(12) | a_src(3) | VMA),
                Insn::new(ALU | SETM | SRC_MD | a_src(3) | m_dest(13)),
            ],
        );
        let t = run3(&m, &[(PHYS, READ_WORD)], HELD_CYCLES);
        let (k, pulses, _) = holds(&t.log);
        eprintln!("{what}: chip {k} cycles held by -WAIT, write pulses in them {pulses:?}");
        for (name, e) in [("chip", &t.chip), ("rtl", &t.rtl), ("micro", &t.micro)] {
            eprintln!(
                "  {name}: M13 {:o} pdl[20] {:o} spcptr {} spc[1] {:o}",
                e.mmem[13], e.pdl[0o20], e.spcptr, e.spc[1]
            );
        }
        assert!(k > 0, "{what}: chip: the VMA store was held by -WAIT");
        assert_eq!(pulses, [0; 5], "{what}: chip: no write pulse in a held cycle");
        assert_eq!(t.chip.mmem[13], READ_WORD, "{what}: chip: the read landed");
        let (pdl, spc) = if what == "PDL" { (0o765432, 0) } else { (0, 0o765432) };
        assert_eq!((t.chip.pdl[0o20], t.chip.spc[1]), (pdl, spc), "{what}: chip");
        for (name, e) in [("rtl", &t.rtl), ("micro", &t.micro)] {
            assert_eq!(
                (e.mmem[13], e.pdl[0o20], e.spcptr, e.spc.clone()),
                (t.chip.mmem[13], t.chip.pdl[0o20], t.chip.spcptr, t.chip.spc.clone()),
                "{what}: {name}"
            );
        }
    }
}

/// **A map store held by `-WAIT` while the read lands writes at the `MD` it
/// ends with**, alike on all three. The store comes one instruction after
/// the `VMA-START-READ`, `-WAIT` holds it, `MD` goes from [`MD_BEFORE`]
/// (level-2 index 3) to [`READ_WORD`] (7) meanwhile, and the level-2 write
/// in the microcycle after lands at 7, once.
#[test]
fn a_map_store_held_by_wait_writes_at_the_md_it_ends_with() {
    let m = read_then(
        &[(MAP_STORE, 2)],
        vec![
            filler(),
            Insn::new(ALU | SETM | m_src(2) | a_src(3) | WRITE_MAP),
            Insn::new(ALU | SETM | SRC_MD | a_src(3) | m_dest(13)),
        ],
    );
    let t = run3(&m, &[(PHYS, READ_WORD)], HELD_CYCLES);
    let (k, pulses, _) = holds(&t.log);
    eprintln!("chip: {k} cycles held by -WAIT, write pulses in them {pulses:?}");
    assert!(k > 0, "chip: the store was held by -WAIT");
    assert_eq!(pulses, [0; 5], "chip: no write pulse in a held cycle");
    assert_eq!(t.chip.mmem[13], READ_WORD, "chip: the read landed");
    assert_eq!((t.chip.l2[3], t.chip.l2[7]), (0, NEW_L2), "chip: written at the new MD, once");
    for (name, e) in [("rtl", &t.rtl), ("micro", &t.micro)] {
        assert_eq!((e.mmem[13], e.l2.clone()), (t.chip.mmem[13], t.chip.l2.clone()), "{name}");
    }
}

/// A map store straight after the `VMA-START-READ` (not held: `MBUSY.SYNC`
/// is not yet up), then an instruction using `MD`, which `-HANG` holds.
/// The store's pending level-2 write is in that hung microcycle. The store
/// also rewrites `VMA` under the read in flight, and the read brings back
/// 0 on the board and `rtl` alike.
fn map_write_pending_into_a_hang() -> Three {
    let m = read_then(
        &[(MAP_STORE, 2)],
        vec![
            Insn::new(ALU | SETM | m_src(2) | a_src(3) | WRITE_MAP),
            Insn::new(ALU | SETM | SRC_MD | a_src(3) | m_dest(13)),
        ],
    );
    let t = run3(&m, &[(PHYS, READ_WORD)], HELD_CYCLES);
    let (k, pulses, hung) = holds(&t.log);
    eprintln!("chip: {k} cycles held by -WAIT, pulses {pulses:?}; {hung} hung");
    for (name, e) in [("chip", &t.chip), ("rtl", &t.rtl), ("micro", &t.micro)] {
        let nz: Vec<_> =
            e.l2.iter()
                .enumerate()
                .filter(|&(_, &w)| w != 0)
                .map(|(k, &w)| format!("{k:o}:{w:o}"))
                .collect();
        eprintln!("  {name}: M13 {:o} l2 {nz:?}", e.mmem[13]);
    }
    t
}

/// **`-HANG` is not `-WAIT`: a hung microcycle's write pulses fire before
/// the hold, at `MD` as it was.** `-TPR0` is `NAND(-HANG, -CLOCK RESET B,
/// CYCLECOMPLETED)` at CLOCK1 1C08, so `-HANG` holds off the *next*
/// cycle's start; `CYCLECOMPLETED` is set by `-TPDONE` (`-TPW60`), after the
/// write pulse has begun at `-TPW30`. So the hung microcycle's read phase
/// and write pulses run, then the generator stops until `-RDFINISH`, and
/// the edge that ends the cycle registers the new `MD`. On the board the
/// pending map write lands at level-2 index 3, [`MD_BEFORE`]'s; `M13` gets
/// the read's word, 0.
#[test]
fn a_map_write_pending_into_a_hang_lands_at_the_md_before_the_hang_on_the_board() {
    let t = map_write_pending_into_a_hang();
    let (_, pulses, hung) = holds(&t.log);
    assert_eq!(pulses, [0; 5], "chip: nothing held by -WAIT fired");
    assert!(hung > 0, "chip: a microcycle was hung");
    assert_eq!(t.chip.mmem[13], 0, "chip: what the read brought back");
    assert_eq!((t.chip.l2[0], t.chip.l2[3]), (0, NEW_L2), "chip: written at MD before the hang");
}

/// `rtl` resolves a `Stall::Hang` before running the microcycle at all
/// (`Rtl::step`: `stall_for`, then `read_phase` again, then `write_phase`),
/// which wrote the pending map write at `MD` after the read had landed.
/// It now fires the hung cycle's write pulse as the cycle ends, as the
/// board does, and agrees.
#[test]
fn rtl_lands_a_map_write_pending_into_a_hang_as_the_board_does() {
    let t = map_write_pending_into_a_hang();
    assert_eq!((t.rtl.mmem[13], t.rtl.l2.clone()), (t.chip.mmem[13], t.chip.l2.clone()), "rtl");
}

/// Dispatch word 1300 plus `MD<2:0>` written with [`DISPATCH_WORD`] by a
/// `DISPWR` whose `M` source is `MD` (three bits, rotate 0), `gap` fillers
/// after a `VMA-START-READ`.
const DISPATCH_WORD: u32 = 0o123456;
fn dispatch_write_on_md(gap: usize) -> Three {
    const E: u64 = 0o1300;
    let mut then = vec![filler(); gap];
    then.push(Insn::new(DISPATCH | DMEM_WRITE | SRC_MD | d_len(3) | a_src(2) | d_addr(E)));
    let m = read_then(&[(DISPATCH_WORD, 2)], then);
    let t = run3(&m, &[(PHYS, READ_WORD)], HELD_CYCLES);
    let (k, pulses, hung) = holds(&t.log);
    eprintln!("gap {gap}: chip {k} cycles held by -WAIT, pulses {pulses:?}; {hung} hung");
    for (name, e) in [("chip", &t.chip), ("rtl", &t.rtl), ("micro", &t.micro)] {
        let w: Vec<_> = (0..8).map(|k| format!("{:o}", e.dmem[E as usize + k])).collect();
        eprintln!("  {name}: dmem 1300..1307 {w:?}");
    }
    t
}

/// Where each gap's write lands on the board: 1302 is `MD<2:0>` = 2,
/// [`MD_BEFORE`]; 1305 is 5, [`READ_WORD`]. Gap 0 is not held at all and
/// reads the old `MD`. Gaps 1 and 2 are hung, and the pulse fires before
/// the hang ends, at the old `MD`. Gap 3 is hung too, for less time, and
/// writes at the new `MD`: the word is in `MD` before its pulse, though
/// `READ IN PROGRESS` is still up. `rtl` agrees on gaps 0 and 3.
const DISPATCH_GAPS: [(usize, usize); 4] = [(0, 0o1302), (1, 0o1302), (2, 0o1302), (3, 0o1305)];

/// **A dispatch write addressed by `MD` in a hung microcycle writes at `MD`
/// as it was before the hang, on the board** --- the same order as
/// [`a_map_write_pending_into_a_hang_lands_at_the_md_before_the_hang_on_the_board`],
/// for a write that is this instruction's own.
#[test]
fn a_dispatch_write_addressed_by_md_in_a_hang_lands_at_the_md_before_it_on_the_board() {
    for (gap, at) in DISPATCH_GAPS {
        let t = dispatch_write_on_md(gap);
        let (_, pulses, _) = holds(&t.log);
        assert_eq!(pulses, [0; 5], "gap {gap}: chip: nothing held by -WAIT fired");
        let written: Vec<_> = (0..2048).filter(|&k| t.chip.dmem[k] != 0).collect();
        assert_eq!(written, vec![at], "gap {gap}: chip");
        assert_eq!(t.chip.dmem[at], DISPATCH_WORD, "gap {gap}: chip");
    }
}

/// `rtl` fires a hung cycle's write pulse as the cycle ends, with `MD` as
/// the bus has left it (`Rtl::step`), and agrees at every gap.
#[test]
fn rtl_lands_a_dispatch_write_in_a_hang_as_the_board_does() {
    for (gap, _) in DISPATCH_GAPS {
        let t = dispatch_write_on_md(gap);
        assert_eq!(t.rtl.dmem, t.chip.dmem, "gap {gap}: rtl");
    }
}

/// **`micro` writes a dispatch word where the dispatch would read**,
/// the `M` source's bits in the address as `DADR` has them. It has no bus
/// timing --- a read lands at once and nothing hangs --- so it matches the
/// board where nothing is hung, gap 0, and writes at the word read after.
#[test]
fn micro_writes_a_dispatch_word_at_the_address_the_dispatch_reads() {
    for (gap, _) in DISPATCH_GAPS {
        let t = dispatch_write_on_md(gap);
        assert_eq!(t.micro.dmem[0o1300], 0, "gap {gap}: not at the field alone");
        if gap == 0 {
            assert_eq!(t.micro.dmem, t.chip.dmem, "gap {gap}: micro");
        }
    }
}
