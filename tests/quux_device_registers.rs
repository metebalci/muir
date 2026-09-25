// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! QUUX's device registers (contract Q7, revision 8). There is no device
//! bus: the processor's register decode reaches the device registers ---
//! MONO TV's, block-disk's and the feature and register page --- at their
//! addresses, never cached, a register access taking one microcycle more
//! than a failed one. An address nothing answers fails at once, reads 0 and
//! sets word 101's NXM bit: no timeout. The frame buffer is on the memory
//! bus with main memory, through the cache.

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

/// Where the program stops: a jump to itself.
const STOP: u16 = 0o77;

/// A program that reads each of `reads`' physical words into A 200 up, a
/// filler after each start, then stops at [`STOP`]; virtual page `k + 1`
/// maps each word's page, and M `k + 1` holds its virtual address.
fn reading(reads: &[u32]) -> Machine {
    let mut prom = Vec::new();
    for k in 0..reads.len() as u64 {
        prom.push(Insn::new(ALU | SETM | m_src(1 + k) | START_READ));
        prom.push(filler());
        prom.push(Insn::new(ALU | SETM | SRC_MD | a_dest(0o200 + k)));
    }
    machine(&prom, reads)
}

fn machine(prom: &[Insn], addresses: &[u32]) -> Machine {
    let mut m = Machine::new();
    m.geometry = Geometry::QUUX;
    m.tv.set_board(muir::tv::Board::MonoTv);
    let mut words = vec![filler(); 1024];
    words[..prom.len()].copy_from_slice(prom);
    words[STOP as usize] = Insn::new(JUMP | target(STOP as u64) | ALWAYS | N);
    words[prom.len()] = Insn::new(JUMP | target(STOP as u64) | ALWAYS | N);
    m.load_prom(&words);
    support::prom_program_in_ram(&mut m);
    let rw = (1 << 23) | (1 << 22);
    for (k, &p) in addresses.iter().enumerate() {
        m.l2_map[1 + k] = rw | (p >> 8);
        m.mmem[1 + k] = ((1 + k as u32) << 8) | (p & 0xff);
    }
    for k in 0..6 {
        m.amem[0o200 + k] = 0o525252;
    }
    m
}

/// Runs an engine to [`STOP`]: the machine, and the instant it got there.
fn run<E: Engine>(mut e: E, ns: impl Fn(&E) -> u64) -> (Machine, u64) {
    e.boot();
    for _ in 0..4000 {
        if e.machine().opc == STOP {
            return (e.machine().clone(), ns(&e));
        }
        e.step().unwrap();
    }
    panic!("the program never reached its end");
}

fn rtl(m: Machine) -> (Machine, u64) {
    run(Rtl::new(m), Rtl::ns)
}

/// The feature page's word 0, the MACHINE-ID: a register that answers.
const REGISTER: u32 = 0o17377000;
/// Inside the register page's page, reserved: a register that answers 0.
const RESERVED: u32 = 0o17377377;

/// Addresses nothing answers: past main memory's end, between the frame
/// buffer and the register page, after the register page, between MONO
/// TV's registers and block-disk's, and the old Unibus window.
const EMPTY: [u32; 5] = [0o10000000, 0o17200000, 0o17377400, 0o17377770, 0o17400000];

/// **A register access takes a microcycle more than a failed one, and a
/// failed one is at once**: the same program, reading the MACHINE-ID or an
/// empty address, ends one microcycle later for the register; and the
/// empty address costs no timeout, the whole program ending well inside
/// the CADR's 4.25 us.
#[test]
fn a_register_takes_a_microcycle_more_than_nothing() {
    let (m, answered) = rtl(reading(&[REGISTER]));
    assert_eq!(m.amem[0o200], Geometry::QUUX.machine_id.unwrap(), "the register's word");
    assert_eq!(m.bus_error & bus_error::XBUS_NXM, 0);
    for empty in EMPTY {
        let (m, failed) = rtl(reading(&[empty]));
        assert_eq!(m.amem[0o200], 0, "{empty:o} reads 0");
        assert_ne!(m.bus_error & bus_error::XBUS_NXM, 0, "{empty:o}: the NXM bit");
        assert_eq!(answered - failed, 40, "{empty:o}: a microcycle more for the register");
        assert!(failed < 1_000, "{empty:o}: no timeout, the program ended at {failed} ns");
    }
}

/// **Every empty address fails the same on both engines**: 0 read, the NXM
/// bit set, a write going nowhere.
#[test]
fn nothing_answers_on_either_engine() {
    for empty in EMPTY {
        let prom = [
            Insn::new(ALU | SETA | a_src(0o100) | MD),
            Insn::new(ALU | SETM | m_src(1) | START_WRITE),
            filler(),
            filler(),
            Insn::new(ALU | SETM | m_src(1) | START_READ),
            filler(),
            Insn::new(ALU | SETM | SRC_MD | a_dest(0o200)),
        ];
        let setup = || {
            let mut m = machine(&prom, &[empty]);
            m.amem[0o100] = 0o37;
            m
        };
        let (r, _) = rtl(setup());
        let (e, _) = run(Micro::new(setup()), |_| 0);
        for (name, m) in [("rtl", r), ("micro", e)] {
            assert_eq!(m.amem[0o200], 0, "{name}, {empty:o}: the write did not read back");
            assert_ne!(m.bus_error & bus_error::XBUS_NXM, 0, "{name}, {empty:o}");
        }
    }
}

/// **A reserved register reads 0 and answers**: no NXM there.
#[test]
fn a_reserved_register_answers_0() {
    let (m, _) = rtl(reading(&[RESERVED]));
    assert_eq!(m.amem[0o200], 0);
    assert_eq!(m.bus_error & bus_error::XBUS_NXM, 0);
}

/// **The frame buffer is on the memory bus, through the cache**: a word
/// written reaches the display, and read twice it misses once and hits
/// once --- where a register is never looked up.
#[test]
fn the_frame_buffer_is_cached() {
    let fb = muir::tv::BUFFER + 0o100;
    let prom = [
        Insn::new(ALU | SETA | a_src(0o100) | MD),
        Insn::new(ALU | SETM | m_src(1) | START_WRITE),
        filler(),
        filler(),
        Insn::new(ALU | SETM | m_src(1) | START_READ),
        filler(),
        Insn::new(ALU | SETM | SRC_MD | a_dest(0o200)),
        Insn::new(ALU | SETM | m_src(1) | START_READ),
        filler(),
        Insn::new(ALU | SETM | SRC_MD | a_dest(0o201)),
        Insn::new(ALU | SETM | m_src(2) | START_READ),
        filler(),
        Insn::new(ALU | SETM | SRC_MD | a_dest(0o202)),
    ];
    let mut m = machine(&prom, &[fb, REGISTER]);
    m.amem[0o100] = 0o123456;
    let mut e = Rtl::new(m);
    e.boot();
    while e.machine().opc != STOP {
        e.step().unwrap();
    }
    let m = e.machine();
    assert_eq!(m.tv.read_buffer(0o100), 0o123456, "the display has the word");
    assert_eq!([m.amem[0o200], m.amem[0o201]], [0o123456, 0o123456]);
    let c = e.cache().unwrap();
    assert_eq!((c.misses, c.hits), (1, 1), "the buffer's two reads; the register none");
}
