// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! QUUX's register page (contract Q2): the feature page, `17377000`, whose
//! words 0-77 are the features and whose word 100 is the interrupt status,
//! 101 the error status and 102 the mode. Reserved words read 0 and ignore
//! writes. It takes the place of the Unibus interrupt vector (`766040`),
//! the error status (`766044`) and the mode register's error stop
//! (`766012`) on QUUX.

use muir::block_disk::{BLOCK_NS, BlockDisk};
use muir::disk_unit::{Geometry as Pack, Unit};
use muir::machine::{Geometry, Machine, bus_error};

const PAGE: u32 = 0o17377000;
const INTERRUPTS: u32 = PAGE + 0o100;
const ERRORS: u32 = PAGE + 0o101;
const MODE: u32 = PAGE + 0o102;

fn quux() -> Machine {
    let mut m = Machine::new();
    m.geometry = Geometry::QUUX;
    m
}

/// **Word 100 says who interrupted**, a bit each and each under its own
/// enable: `<0>` the tick, `<1>` the interval timer, `<2>` block-disk's
/// done. The processor's interrupt pending is their OR.
#[test]
fn word_100_says_who_interrupted() {
    let mut m = quux();
    assert_eq!(m.bus_read(INTERRUPTS), 0, "nothing");
    assert!(!m.interrupt());
    // The tick and the interval timer, 50 µs, enabled at 0.
    m.tick.period(0, 50);
    m.tick.control(0, 1 | 4);
    m.ns = 49_000;
    assert_eq!(m.bus_read(INTERRUPTS), 0, "neither yet");
    m.ns = 50_000;
    assert_eq!(m.bus_read(INTERRUPTS), 2, "the interval timer");
    assert!(m.interrupt());
    m.ns = 16_667_000;
    assert_eq!(m.bus_read(INTERRUPTS), 3, "and the tick");
    m.tick.control(m.ns, 0);
    assert_eq!(m.bus_read(INTERRUPTS), 0, "disabled, neither");
    // Block-disk's done interrupt, command <11>.
    let mut d = BlockDisk::new(BLOCK_NS);
    d.attach(Unit::blank(Pack::T300));
    m.block_disk = Some(d);
    m.main[0o100] = 0o1000;
    m.bus_write(muir::block_disk::REGS + muir::block_disk::CLP, 0o100);
    m.bus_write(muir::block_disk::REGS + muir::block_disk::COMMAND, 1 << 11);
    m.bus_write(muir::block_disk::REGS + muir::block_disk::START, 0);
    m.ns += BLOCK_NS;
    assert_eq!(m.bus_read(INTERRUPTS), 4, "block-disk done");
    assert!(m.interrupt());
}

/// **Word 101 is the error status**: the bus errors, which a write clears,
/// as a write of `766044` does.
#[test]
fn word_101_is_the_error_status() {
    let mut m = quux();
    assert_eq!(m.bus_read(ERRORS), 0);
    m.bus_read(0o17376000); // an empty Xbus I/O address
    assert_ne!(m.bus_error & bus_error::XBUS_NXM, 0, "the read timed out");
    assert_eq!(m.bus_read(ERRORS), m.bus_error as u32);
    assert_ne!(m.bus_read(ERRORS) & bus_error::XBUS_NXM as u32, 0);
    m.bus_write(ERRORS, 0);
    assert_eq!((m.bus_error, m.bus_read(ERRORS)), (0, 0), "cleared");
}

/// **Word 102 is the mode: `<0>` error stop**, the bit `(si:%halt)` relies
/// on, which the CADR has in its mode register.
#[test]
fn word_102_is_error_stop() {
    let mut m = quux();
    assert_eq!(m.bus_read(MODE), 0);
    m.bus_write(MODE, 1);
    assert!(m.mode.errstop);
    assert_eq!(m.bus_read(MODE), 1);
    m.bus_write(MODE, 0);
    assert!(!m.mode.errstop);
    // The host sets it as it sets the spy's mode register, and the word
    // shows it.
    m.mode.errstop = true;
    assert_eq!(m.bus_read(MODE), 1);
}

/// **The reserved words read 0 and ignore writes**, and nothing on the page
/// times out.
#[test]
fn the_reserved_words_read_0() {
    let mut m = quux();
    for w in [0o15, 0o77, 0o103, 0o117, 0o160, 0o377] {
        m.bus_write(PAGE + w, !0);
        assert_eq!(m.bus_read(PAGE + w), 0, "word {w:o}");
    }
    assert!(!m.mode.errstop);
    assert_eq!(m.bus_error & bus_error::XBUS_NXM, 0);
}

/// **The CADR has no such page**: a read of word 100 times out.
#[test]
fn the_cadr_has_no_register_page() {
    let mut m = Machine::new();
    m.bus_read(INTERRUPTS);
    assert_ne!(m.bus_error & bus_error::XBUS_NXM, 0);
}

/// **The microcode reaches the page through the bus on both engines**: a
/// store of 1 to word 102 sets error stop, and a read of it gives 1 back.
/// Virtual page 1 maps the register page.
#[test]
fn both_engines_write_and_read_the_page() {
    use muir::engine::Engine;
    use muir::isa::Insn;
    use muir::isa::asm::{ALU, MD, SETM, SRC_MD, START_READ, START_WRITE, a_dest, filler, m_src};
    use muir::micro::Micro;
    use muir::rtl::Rtl;
    let mut prom = vec![
        Insn::new(ALU | SETM | m_src(2) | MD),
        Insn::new(ALU | SETM | m_src(1) | START_WRITE),
    ];
    prom.extend([filler(); 12]);
    prom.push(Insn::new(ALU | SETM | m_src(1) | START_READ));
    prom.extend([filler(); 12]);
    prom.push(Insn::new(ALU | SETM | SRC_MD | a_dest(0o200)));
    let machine = || {
        let mut m = quux();
        let mut words = vec![filler(); 1024];
        words[..prom.len()].copy_from_slice(&prom);
        m.load_prom(&words);
        m.l2_map[1] = (1 << 23) | (1 << 22) | 0o36776;
        m.mmem[1] = (1 << 8) | 0o102;
        m.mmem[2] = 1;
        m
    };
    let mut e = Micro::new(machine());
    e.boot();
    let mut r = Rtl::new(machine());
    r.boot();
    for _ in 0..400 {
        e.step().unwrap();
        r.step().unwrap();
    }
    for (name, m) in [("micro", e.machine()), ("rtl", r.machine())] {
        assert!(m.mode.errstop, "{name}: error stop set");
        assert_eq!(m.amem[0o200], 1, "{name}: word 102 read back");
        assert_eq!(m.bus_error & bus_error::XBUS_NXM, 0, "{name}: no timeout");
    }
}
