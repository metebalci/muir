// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! QUUX's MACRO-DISPATCH register, its MACRO DISPATCH MEMORY and the fused
//! return (contract H8a, revision 12), on hand-written microcode.
//!
//! Functional destination 5 writes the register (`<13:0>` the main loop's
//! address, `<31>` the enable), 6 the memory's index and 7 the entry at it.
//! With the register enabled, a return that pops the main loop's word
//! where no instruction fetch is needed goes straight to the handler the
//! memory names for the next halfword's `<15:6>`, if the entry has R and P
//! clear: the main loop's dispatch and the push of its return are not run,
//! two microcycles, and the popped word stays on the stack unless the
//! entry's N is set. Every other return runs the main loop as it always
//! has.
//!
//! The main loop here is made as microcode 2000's `QMLP` is
//! (`uc-macrocode.lisp:9-13`): the condition-6 call, `M-INST-BUFFER <- MD`,
//! `(DISPATCH-XCT-NEXT M-INST-OP OPDTB)` on the halfword's `<13:9>`, and the
//! push of `A-MAIN-DISPATCH` back.

use muir::engine::Engine;
use muir::isa::Insn;
use muir::isa::asm::{
    ALU, ALWAYS, CARRY_IN, DISPATCH, JUMP, M_PLUS_C, N, P, POPJ, R, SETA, SETM, SRC_MD, a_src,
    d_addr, d_len, filler, m_dest, m_src, rot, src, target,
};
use muir::machine::{Geometry, Machine, macro_dispatch};
use muir::micro::Micro;
use muir::rtl::Rtl;

mod support;

/// The main loop, at an address with `<1:0>` clear as the stream hardware
/// requires (`uc-macrocode.lisp:6`).
const QMLP: u64 = 0o100;
/// The opcode table in dispatch memory.
const OPDTB: u64 = 0o2300;
/// A dispatch table whose two entries return, as `QMDTBD`'s `D-PDL` does
/// for `QIMOVE1` (`uc-parameters.lisp:1283-1289`).
const RETURNS: u64 = 0o2200;
/// A-MAIN-DISPATCH, the main loop's return: `<14>` and the address.
const MAIN: u32 = 1 << 14 | QMLP as u32;
/// Where the macroinstructions are, as a word address; its page is mapped.
const CODE: u32 = 0o400;
/// Where opcode 7, the last, goes: a jump to itself.
const STOP: u16 = 0o177;
/// The handler a specialised entry names ([`specialised`]): it counts in
/// M 7.
const SPECIAL: u64 = 0o300;
/// The main loop's `<13:9>` rotate, `IR<4:0>` = 23: the field at `<9>`
/// brought to `<0>`.
const OP_ROTATE: u64 = 32 - 9;

/// A halfword: `<13:9>` the opcode and `<8:6>` the register.
const fn hw(op: u32, reg: u32) -> u32 {
    op << 9 | reg << 6
}

/// The program's halfwords in the order they run: a word's `<15:0>`, then
/// its `<31:16>`. Opcodes 1 to 4 are plain jumps, returning by a POPJ
/// (1), a jump with R (2), a dispatch with R (3), and a POPJ after pushing
/// the return back, their entry having N (4); 5's entry falls through (R
/// and P) and 6's pushes (P and N, as a trap's); 7 stops.
const PROGRAM: [u32; 22] = [
    hw(1, 0),
    hw(2, 5),
    hw(3, 1),
    hw(4, 0),
    hw(1, 2),
    hw(2, 0),
    hw(6, 0),
    hw(5, 0),
    hw(1, 0),
    hw(1, 0),
    hw(3, 0),
    hw(6, 0),
    hw(5, 0),
    hw(2, 5),
    hw(4, 3),
    hw(4, 0),
    hw(2, 5),
    hw(3, 7),
    hw(3, 0),
    hw(2, 5),
    hw(7, 0),
    hw(7, 0),
];

/// How many times each opcode 1 to 6 runs in [`PROGRAM`].
const COUNTS: [u32; 6] = [4, 5, 4, 3, 2, 2];

/// What can keep a return from fusing, put into opcode 1's POPJ.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Pop {
    /// A plain POPJ.
    Plain,
    /// The POPJ writes M 31, the same word back.
    WritesM31,
    /// The POPJ pushes the main loop's word too.
    Pushes,
    /// The POPJ writes INTERRUPT-CONTROL, as it stands.
    WritesInterruptControl,
}

/// The run's settings.
#[derive(Clone, Copy)]
struct Setup {
    geometry: Geometry,
    /// The register's word, written by destination 5 at the start.
    register: u32,
    sequence_break: bool,
    pop: Pop,
    /// The MACRO DISPATCH MEMORY holds [`specialised`]'s entry for opcode
    /// 2 with register 5.
    specialised: bool,
}

impl Setup {
    fn off() -> Setup {
        Setup {
            geometry: Geometry::QUUX,
            register: 0,
            sequence_break: false,
            pop: Pop::Plain,
            specialised: false,
        }
    }

    fn on() -> Setup {
        Setup { register: enabled(), ..Setup::off() }
    }
}

/// A functional destination, with M's address 37 as the scratch word.
fn fd(code: u64) -> u64 {
    code << 19 | 0o37 << 14
}

/// The register's word for this program's main loop, enabled.
fn enabled() -> u32 {
    macro_dispatch::word(QMLP as u16, 0, 0)
}

/// The entry `specialised` sets: opcode 2 with register 5.
fn specialised_index() -> usize {
    (hw(2, 5) >> 6) as usize
}

/// The program's control store, dispatch memory, MACRO DISPATCH MEMORY
/// (the generic handlers, OPDTB's entry for each index's opcode), A
/// memory and map.
fn machine(s: Setup) -> Machine {
    let mut m = Machine::new();
    m.geometry = s.geometry;
    let mut prom = vec![filler(); 1024];
    let put = |prom: &mut Vec<Insn>, at: u64, w: u64| prom[at as usize] = Insn::new(w);
    // Start: MACRO-DISPATCH, LC, INTERRUPT-CONTROL (the sequence break),
    // the main loop's return pushed, and a return into the main loop.
    put(&mut prom, 0, ALU | SETA | a_src(0o51) | fd(5));
    put(&mut prom, 1, ALU | SETA | a_src(0o52) | fd(1));
    put(&mut prom, 2, ALU | SETA | a_src(0o53) | fd(2));
    put(&mut prom, 3, ALU | SETA | a_src(0o50) | fd(0o15));
    put(&mut prom, 5, filler().raw() | POPJ);
    // The main loop.
    put(&mut prom, QMLP, JUMP | target(0o110) | P | 1 << 5 | 6);
    put(&mut prom, QMLP + 1, ALU | SETM | SRC_MD | m_dest(0o31));
    put(
        &mut prom,
        QMLP + 2,
        DISPATCH | m_src(0o31) | 3 << 10 | rot(OP_ROTATE) | d_len(5) | d_addr(OPDTB),
    );
    put(&mut prom, QMLP + 3, ALU | SETA | a_src(0o50) | fd(0o15));
    // Opcode 5's entry falls through (R and P): it lands here after the
    // push.
    put(&mut prom, QMLP + 4, ALU | M_PLUS_C | CARRY_IN | m_src(5) | m_dest(5) | POPJ);
    // Condition 6's call, counted in M 10: back to QMLP + 1.
    put(&mut prom, 0o110, ALU | M_PLUS_C | CARRY_IN | m_src(0o10) | m_dest(0o10) | POPJ);
    // Opcode 1: count in M 1 and return by a POPJ, the one `Pop` changes.
    // A return runs the microinstruction after it first, a filler.
    let pop = match s.pop {
        Pop::Plain => ALU | SETA | a_src(3) | m_dest(0o36),
        Pop::WritesM31 => ALU | SETM | m_src(0o31) | m_dest(0o31),
        Pop::Pushes => ALU | SETA | a_src(0o50) | fd(0o15),
        Pop::WritesInterruptControl => ALU | SETA | a_src(0o53) | fd(2),
    };
    put(&mut prom, 0o204, ALU | M_PLUS_C | CARRY_IN | m_src(1) | m_dest(1));
    put(&mut prom, 0o205, pop | POPJ);
    // Opcode 2: count in M 2 and return by a jump with R.
    put(&mut prom, 0o210, ALU | M_PLUS_C | CARRY_IN | m_src(2) | m_dest(2));
    put(&mut prom, 0o211, JUMP | R | ALWAYS);
    // Opcode 3: count in M 3 and return by a dispatch whose entry has R.
    put(&mut prom, 0o214, ALU | M_PLUS_C | CARRY_IN | m_src(3) | m_dest(3));
    put(&mut prom, 0o215, DISPATCH | m_src(2) | d_len(1) | d_addr(RETURNS));
    // Opcode 4, its entry with N: the main loop's push is not run, so it
    // pushes the return back itself before its POPJ.
    put(&mut prom, 0o220, ALU | M_PLUS_C | CARRY_IN | m_src(4) | m_dest(4));
    put(&mut prom, 0o221, ALU | SETA | a_src(0o50) | fd(0o15));
    put(&mut prom, 0o222, filler().raw() | POPJ);
    // Opcode 6's entry pushes and jumps (P and N): drop the pushed word,
    // count, and put the main loop's return back.
    put(&mut prom, 0o120, ALU | SETM | src(0o14) | m_dest(0o20));
    put(&mut prom, 0o121, ALU | M_PLUS_C | CARRY_IN | m_src(6) | m_dest(6));
    put(&mut prom, 0o122, ALU | SETA | a_src(0o50) | fd(0o15));
    put(&mut prom, 0o124, filler().raw() | POPJ);
    // The specialised handler: count in M 7 and return.
    put(&mut prom, SPECIAL, ALU | M_PLUS_C | CARRY_IN | m_src(7) | m_dest(7) | POPJ);
    // Opcode 7 stops: a jump to itself.
    put(&mut prom, STOP as u64, JUMP | target(STOP as u64) | ALWAYS | N);
    m.load_prom(&prom);
    support::prom_program_in_ram(&mut m);

    // Dispatch memory's words: `<16>` R, `<15>` P, `<14>` N, `<13:0>` the
    // address.
    m.dmem[OPDTB as usize + 1] = 0o204;
    m.dmem[OPDTB as usize + 2] = 0o210;
    m.dmem[OPDTB as usize + 3] = 0o214;
    m.dmem[OPDTB as usize + 4] = 1 << 14 | 0o220;
    m.dmem[OPDTB as usize + 5] = 1 << 16 | 1 << 15;
    m.dmem[OPDTB as usize + 6] = 1 << 15 | 1 << 14 | 0o120;
    m.dmem[OPDTB as usize + 7] = STOP as u32;
    m.dmem[RETURNS as usize] = 1 << 16;
    m.dmem[RETURNS as usize + 1] = 1 << 16;
    // The MACRO DISPATCH MEMORY: every index OPDTB's entry for its opcode,
    // `<13:9>` of the halfword being the index's `<7:3>`.
    for (k, e) in m.macro_dispatch.entries.iter_mut().enumerate() {
        *e = m.dmem[OPDTB as usize + (k >> 3 & 0o37)];
    }
    if s.specialised {
        m.macro_dispatch.entries[specialised_index()] = SPECIAL as u32;
    }

    m.amem[0o50] = MAIN;
    m.amem[0o51] = s.register;
    m.amem[0o52] = CODE * 4;
    m.amem[0o53] = if s.sequence_break { 1 << 26 } else { 0 };
    let rw = (1 << 23) | (1 << 22);
    m.l2_map[1] = rw | 1;
    for (k, pair) in PROGRAM.chunks(2).enumerate() {
        m.main[CODE as usize + k] = pair[0] | pair[1] << 16;
    }
    m
}

/// Runs an engine until opcode 7's handler has run, and gives the machine
/// back with the microcycles it took.
fn run<E: Engine>(mut e: E) -> (u64, Machine) {
    e.boot();
    for n in 0..20_000 {
        if e.machine().opc == STOP {
            return (n, e.machine().clone());
        }
        e.step().unwrap();
    }
    panic!("the program never reached its end");
}

fn both(s: Setup) -> [(&'static str, u64, Machine); 2] {
    let (n, m) = run(Micro::new(machine(s)));
    let (rn, rm) = run(Rtl::new(machine(s)));
    [("micro", n, m), ("rtl", rn, rm)]
}

/// The counts of opcodes 1 to 6, and M 7, the specialised handler's.
fn counts(m: &Machine) -> [u32; 7] {
    let mut c = [0; 7];
    c.copy_from_slice(&m.mmem[1..=7]);
    c
}

/// The architectural state a program leaves: M and A memory, the SPC
/// stack and its pointer, the location counter, the PDL's pointer and
/// index, Q, VMA, MD and the dispatch constant. Not the words that differ
/// by the register's word alone: A 51, which holds it, and M 37, where
/// every functional destination here writes too.
fn state(m: &Machine) -> impl PartialEq + std::fmt::Debug {
    let mut mmem = m.mmem;
    let mut amem = m.amem;
    mmem[0o37] = 0;
    amem[0o37] = 0;
    amem[0o51] = 0;
    (
        mmem,
        amem.to_vec(),
        m.spc,
        m.spcptr,
        m.lc,
        (m.pdl_pointer, m.pdl_index, m.q, m.vma, m.md, m.dispatch_constant),
    )
}

/// How many returns fuse in [`PROGRAM`]: a return into the second
/// halfword of a word, which needs no fetch, from a handler whose return
/// fuses (opcode 7's never returns), into an entry with R and P clear (not
/// 5 or 6).
fn fusing(returns_fuse: impl Fn(u32) -> bool) -> u64 {
    let op = |h: u32| h >> 9 & 0o37;
    (1..PROGRAM.len())
        .filter(|&k| k % 2 == 1 && op(PROGRAM[k - 1]) != 7)
        .filter(|&k| returns_fuse(op(PROGRAM[k - 1])) && !matches!(op(PROGRAM[k]), 5 | 6))
        .count() as u64
}

/// **Destinations 5 to 7 write the register, the index and the entry**
/// from revision 12, each as well as M, as every functional destination
/// does: the register keeps `<31>` and `<28:0>`, the index its ten bits
/// and the entry its eighteen. On revision 11 and on the CADR they write
/// only M.
#[test]
fn destinations_5_to_7_write_the_register_the_index_and_the_entry() {
    let prom = [
        Insn::new(ALU | SETA | a_src(0o51) | fd(5)),
        Insn::new(ALU | SETA | a_src(0o52) | fd(6)),
        Insn::new(ALU | SETA | a_src(0o53) | fd(7)),
        Insn::new(ALU | SETA | a_src(0o54) | fd(6)),
        Insn::new(ALU | SETA | a_src(0o55) | fd(7)),
    ];
    let program = |geometry: Geometry| {
        let mut m = Machine::new();
        m.geometry = geometry;
        m.load_prom(&prom);
        support::prom_program_in_ram(&mut m);
        m.amem[0o51] = !0;
        m.amem[0o52] = 0o7771234;
        m.amem[0o53] = 0o7654321;
        m.amem[0o54] = 0o17;
        m.amem[0o55] = 0o1234567;
        m
    };
    for geometry in [Geometry::QUUX, Geometry::QUUX_11, Geometry::CADR] {
        let mut e = Micro::new(program(geometry));
        e.boot();
        e.run(12);
        let mut r = Rtl::new(program(geometry));
        r.boot();
        r.run(12);
        for (name, m) in [("micro", e.machine()), ("rtl", r.machine())] {
            let d = &m.macro_dispatch;
            let written = (d.register, d.index, d.entries[0o1234], d.entries[0o17]);
            if geometry.macro_dispatch {
                let want = (1 << 31 | 0o3777777777, 0o17, 0o654321, 0o234567);
                assert_eq!(written, want, "{geometry:?}, {name}");
            } else {
                assert_eq!(written, (0, 0, 0, 0), "{geometry:?}, {name}: nothing written");
            }
            assert_eq!(m.mmem[0o37], 0o1234567, "{geometry:?}, {name}: M written too");
        }
    }
}

/// **A fused return skips the main loop's dispatch and push**, on both
/// engines: the program leaves the same state with the MACRO DISPATCH
/// MEMORY holding the generic handlers as without it, two microcycles
/// fewer for each fused return. A POPJ, a jump with R and a dispatch whose
/// entry has R all fuse; an entry with N fuses and pops the word, as the
/// main loop's nopped push would have left it popped.
#[test]
fn a_fused_return_skips_the_dispatch_and_the_push() {
    let off = both(Setup::off());
    let on = both(Setup::on());
    let fused = fusing(|_| true);
    assert!(fused >= 6, "the program fuses {fused}");
    for ((name, n_off, m_off), (_, n_on, m_on)) in off.iter().zip(on.iter()) {
        assert_eq!(counts(m_off)[..6], COUNTS, "{name}: every macroinstruction ran once");
        assert_eq!(state(m_on), state(m_off), "{name}: the same state");
        assert_eq!(m_off.macro_dispatch.fused, 0, "{name}: nothing fused while disabled");
        assert_eq!(m_on.macro_dispatch.fused, fused, "{name}");
        assert_eq!(n_off - n_on, 2 * fused, "{name}: two microcycles a fused return");
    }
}

/// **A dispatch's R return fuses** (contract H8a): the returns from
/// opcode 3, whose handler returns through a dispatch entry with R, are
/// among those fused, which a POPJ-only fused return fails.
#[test]
fn a_dispatch_return_fuses() {
    let without_3 = fusing(|op| op != 3);
    assert!(fusing(|_| true) > without_3, "the program has dispatch returns to fuse");
    for (name, _, m) in both(Setup::on()) {
        assert_eq!(counts(&m)[2], COUNTS[2], "{name}");
        assert_eq!(m.macro_dispatch.fused, fusing(|_| true), "{name}");
    }
}

/// **The fused return takes the MACRO DISPATCH MEMORY's entry**, indexed
/// by the whole `<15:6>` and not by the opcode alone: with opcode 2 and
/// register 5 given a handler of its own, its fused occurrences run it and
/// every other occurrence of opcode 2 the generic one.
#[test]
fn the_entry_for_the_halfword_is_taken() {
    // Opcode 2 with register 5 in a word's second halfword, after a
    // handler whose return fuses.
    let special: u64 =
        (1..PROGRAM.len()).filter(|&k| k % 2 == 1 && PROGRAM[k] == hw(2, 5)).count() as u64;
    assert!(special >= 2, "the program has the halfword where it fuses");
    for (name, _, m) in both(Setup { specialised: true, ..Setup::on() }) {
        let c = counts(&m);
        assert_eq!(c[6] as u64, special, "{name}: the specialised handler");
        assert_eq!(c[1] + c[6], COUNTS[1], "{name}: opcode 2 ran as often");
        assert_eq!(c[..1], COUNTS[..1], "{name}");
        assert_eq!(c[2..6], COUNTS[2..6], "{name}");
    }
    // Disabled, the entry is never read.
    for (name, _, m) in both(Setup { specialised: true, ..Setup::off() }) {
        assert_eq!(counts(&m)[6], 0, "{name}");
    }
}

/// **Each case that is not fused runs today's path**: a return that pops
/// the main loop's word while its own microinstruction writes M 31,
/// pushes, or writes INTERRUPT-CONTROL is not fused, and the run is the
/// disabled one's, microcycle for microcycle, less two for each return that
/// still fuses. The needed fetch and the entries with R or P are in every
/// run: [`fusing`] counts them out.
#[test]
fn a_return_that_writes_m31_pushes_or_writes_interrupt_control_is_not_fused() {
    for pop in [Pop::WritesM31, Pop::Pushes, Pop::WritesInterruptControl] {
        let off = both(Setup { pop, ..Setup::off() });
        let on = both(Setup { pop, ..Setup::on() });
        let fused = fusing(|op| op != 1);
        for ((name, n_off, m_off), (_, n_on, m_on)) in off.iter().zip(on.iter()) {
            assert_eq!(counts(m_off)[..6], COUNTS, "{pop:?}, {name}");
            assert_eq!(state(m_on), state(m_off), "{pop:?}, {name}: the same state");
            assert_eq!(m_on.macro_dispatch.fused, fused, "{pop:?}, {name}");
            assert_eq!(n_off - n_on, 2 * fused, "{pop:?}, {name}");
        }
    }
}

/// **Condition 6 is tested on the fetch path only** (contract H8a):
/// with the sequence break up, the main loop's call on condition 6 is
/// taken at every fetch with the fused return as without it, and the
/// returns that need no fetch fuse all the same, as today's main loop goes
/// to `QMLP+2` for them without testing it.
#[test]
fn condition_6_is_tested_on_the_fetch_path() {
    let off = both(Setup { sequence_break: true, ..Setup::off() });
    let on = both(Setup { sequence_break: true, ..Setup::on() });
    let fetches = PROGRAM.len() as u32 / 2;
    for ((name, n_off, m_off), (_, n_on, m_on)) in off.iter().zip(on.iter()) {
        assert_eq!(counts(m_off)[..6], COUNTS, "{name}");
        assert_eq!(m_off.mmem[0o10], fetches, "{name}: the call at every fetch");
        assert_eq!(state(m_on), state(m_off), "{name}");
        assert_eq!(m_on.macro_dispatch.fused, fusing(|_| true), "{name}");
        assert_eq!(n_off - n_on, 2 * fusing(|_| true), "{name}");
    }
}

/// **A return to another address is not fused**: the register naming
/// another main loop, as `DMLP`'s word is to `QMLP`'s, changes nothing.
#[test]
fn another_main_loop_is_not_fused() {
    let other = macro_dispatch::word(QMLP as u16 + 4, 0, 0);
    let off = both(Setup::off());
    let on = both(Setup { register: other, ..Setup::off() });
    for ((name, n_off, m_off), (_, n_on, m_on)) in off.iter().zip(on.iter()) {
        assert_eq!(state(m_on), state(m_off), "{name}");
        assert_eq!(m_on.macro_dispatch.fused, 0, "{name}");
        assert_eq!(n_on, n_off, "{name}");
    }
}

/// **Revision 11 has none of it, nor has the CADR**: the register written
/// with the enable changes nothing there, destination 5 writing only M.
#[test]
fn revision_11_and_the_cadr_have_none_of_it() {
    let off = both(Setup::off());
    for geometry in [Geometry::QUUX_11, Geometry::CADR] {
        let on = both(Setup { geometry, ..Setup::on() });
        for ((name, n_off, m_off), (_, n_on, m_on)) in off.iter().zip(on.iter()) {
            assert_eq!(counts(m_on)[..6], COUNTS, "{geometry:?}, {name}");
            assert_eq!(m_on.macro_dispatch.register, 0, "{geometry:?}, {name}");
            assert_eq!(m_on.macro_dispatch.fused, 0, "{geometry:?}, {name}");
            if geometry == Geometry::QUUX_11 {
                assert_eq!(state(m_on), state(m_off), "{name}");
                assert_eq!(n_on, n_off, "{name}");
            }
        }
    }
}

/// **-RESET clears the enable and keeps the rest**: the index, the
/// entries, and the register's other bits, on both engines.
#[test]
fn reset_clears_the_enable_only() {
    let check = |name: &str, m: &Machine, entries: &[u32]| {
        let d = &m.macro_dispatch;
        assert_eq!(d.register, enabled() & !macro_dispatch::ENABLE, "{name}: the enable cleared");
        assert_eq!(d.index, 0o17, "{name}: the index kept");
        assert_eq!(d.entries, entries, "{name}: the entries kept");
    };
    let mut m = machine(Setup::on());
    m.macro_dispatch.index = 0o17;
    let entries = m.macro_dispatch.entries.clone();
    let mut e = Micro::new(m.clone());
    e.boot();
    e.run(10);
    assert_eq!(e.machine().macro_dispatch.register, enabled(), "micro: written by destination 5");
    e.machine_mut().prog_reset = true;
    e.run(2);
    check("micro", e.machine(), &entries);
    let mut r = Rtl::new(m);
    r.boot();
    r.run(10);
    assert_eq!(r.machine().macro_dispatch.register, enabled(), "rtl: written by destination 5");
    r.machine_mut().prog_reset = true;
    r.run(2);
    check("rtl", r.machine(), &entries);
}

/// **A control-store write clears the enable**, wherever it lands, and
/// keeps the rest: a new microcode never runs on the entries the old one
/// left (contract H8a §3.6). The register is enabled by destination 5, and
/// a `WRITE-I-MEM` --- a jump with P and R --- follows.
#[test]
fn a_control_store_write_clears_the_enable() {
    let prom = [
        Insn::new(ALU | SETA | a_src(0o51) | fd(5)),
        Insn::new(filler().raw()),
        Insn::new(filler().raw()),
        Insn::new(JUMP | target(0o700) | P | R | ALWAYS | m_src(1) | a_src(0o52)),
        Insn::new(filler().raw()),
        Insn::new(filler().raw()),
    ];
    let program = || {
        let mut m = Machine::new();
        m.geometry = Geometry::QUUX;
        m.load_prom(&prom);
        support::prom_program_in_ram(&mut m);
        m.amem[0o51] = enabled();
        m.mmem[1] = 0o1234;
        m.amem[0o52] = 0o5670;
        m.macro_dispatch.entries[5] = 0o4321;
        m
    };
    fn check<E: Engine>(name: &str, mut e: E) {
        e.boot();
        let mut n = 0;
        while e.machine().macro_dispatch.register != enabled() {
            e.step().unwrap();
            n += 1;
            assert!(n < 20, "{name}: destination 5 never written");
        }
        e.run(8);
        let m = e.machine();
        assert_eq!(m.imem[0o700].raw(), 0o5670 << 32 | 0o1234, "{name}: the word written");
        let d = &m.macro_dispatch;
        assert_eq!(d.register, enabled() & !macro_dispatch::ENABLE, "{name}: the enable cleared");
        assert_eq!(d.entries[5], 0o4321, "{name}: the entries kept");
    }
    check("micro", Micro::new(program()));
    check("rtl", Rtl::new(program()));
}

/// **A checkpoint keeps the register, the index and the entries**, and
/// whether the machine is revision 12 or 11; the count of fused returns
/// is not the machine's.
#[test]
fn a_checkpoint_keeps_the_register_and_the_memory() {
    use muir::checkpoint::{Reader, Writer};
    for geometry in [Geometry::QUUX, Geometry::QUUX_11] {
        let mut m = machine(Setup { geometry, ..Setup::on() });
        m.macro_dispatch.register = enabled();
        m.macro_dispatch.index = 0o1001;
        m.macro_dispatch.entries[0o1777] = 0o777777;
        m.macro_dispatch.fused = 5;
        let mut w = Writer::new();
        m.save(&mut w);
        let body = w.finish();
        let mut back = Machine::new();
        back.load(&mut Reader::new(&body)).unwrap();
        assert_eq!(back.geometry, geometry);
        assert_eq!(back.macro_dispatch.register, enabled(), "{geometry:?}");
        assert_eq!(back.macro_dispatch.index, 0o1001, "{geometry:?}");
        assert_eq!(back.macro_dispatch.entries, m.macro_dispatch.entries, "{geometry:?}");
        assert_eq!(back.macro_dispatch.fused, 0, "{geometry:?}");
        assert_eq!(Machine::checkpointed_geometry(&body).unwrap(), geometry);
    }
}

/// **MACHINE-ID says revision 12, and feature word 17 the MACRO DISPATCH
/// MEMORY's 1,024 entries**; revision 11 says 11 and reads 0 there, as
/// every unused word does.
#[test]
fn revision_12_says_so() {
    let word_17 = (Geometry::FEATURE_PAGE << 8) | 0o17;
    assert_eq!(Geometry::QUUX.machine_id.unwrap() >> 4 & 0o7777, 12);
    assert_eq!(Geometry::QUUX.feature_word(word_17), Some(1024));
    assert_eq!(Geometry::QUUX_11.machine_id.unwrap() >> 4 & 0o7777, 11);
    assert_eq!(Geometry::QUUX_11.feature_word(word_17), Some(0));
    assert_eq!(Geometry::CADR.feature_word(word_17), None);
}
