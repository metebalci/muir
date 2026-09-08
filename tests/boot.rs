// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Runs MIT's boot PROM on the `micro` engine.
//!
//! The PROM is committed, in `mit/sys/ubin/`, and is one file across the
//! releases; the one test here that boots on to the pack needs the
//! vendored System 100 release, and says so when it is skipped.

use std::collections::BTreeSet;
use std::time::Instant;

use muir::engine::Engine;
use muir::isa::Insn;
use muir::machine::Machine;
use muir::mcr;
use muir::micro::Micro;
use muir::rtl::Rtl;

mod support;
use support::machine_with_pack;

/// Every PC the machine visits over one turn of the wait it is in.  Two
/// thousand microcycles is many times round a loop of a dozen.
fn waiting_on_the_drive(e: &mut impl Engine) -> BTreeSet<u16> {
    let mut seen = BTreeSet::new();
    for _ in 0..2_000 {
        seen.insert(e.pc());
        e.step().expect("stopped while waiting for the drive");
    }
    seen
}

fn booted(prom: &[Insn]) -> Micro {
    let mut m = Machine::new();
    m.load_prom(prom);
    let mut e = Micro::new(m);
    e.boot();
    e
}

/// The whole boot PROM runs, and gets as far as the disk.
///
/// `PAGE-0-PARITY-FIX` is the first place an engine can stop short: it reads
/// and rewrites every word of page 0 and then runs one word past the end into
/// Xbus I/O, where nothing answers. That is not the disk, it is the loop
/// overrunning, and the microcode says so --- "This does one extra location,
/// too bad". The bus times such an address out and sets an NXM bit, as the
/// board does, so the machine carries on to `DISK-RECALIBRATE` at 0o541 and
/// waits there for a drive.
///
/// **No drive is attached here**, and the controller board is, so the wait is
/// the longer of the two the PROM has: the controller answers, says it is
/// ready, and reports the selected unit off-line, so the microcode goes round
/// read status, select unit 0, read status, test bit 9 --- the hang its own
/// error table calls `AWAIT-DISK-ON-LINE`. Attaching a pack is
/// `tests/cosim.rs`, where the engines are held to each other.
#[test]
fn boot_prom_reaches_the_disk() {
    let mut e = booted(&muir::prom::boot_prom());

    // `DISK-RECALIBRATE` spins reading the status register, so the machine no
    // longer stops on its own. Long enough to get there and be seen looping.
    const CYCLES: u64 = 2_000_000;
    let started = Instant::now();
    let mut reached = false;
    let mut cycles = 0u64;
    let mut executed = 0u64;
    let mut to_disk = 0u64;
    while cycles < CYCLES {
        if let Err(h) = e.step() {
            panic!("stopped for the wrong reason at {cycles}: {h:?}");
        }
        cycles += 1;
        if let Some(pc) = e.executed() {
            executed += 1;
            if pc == 0o541 && !reached {
                reached = true;
                to_disk = executed;
            }
        }
    }
    let elapsed = started.elapsed();
    eprintln!(
        "{cycles} microcycles in {:.1?} ({:.1} M/s); reached the disk after \
         {to_disk} executed instructions; PC {:o}, bus error {:o}",
        elapsed,
        cycles as f64 / elapsed.as_secs_f64() / 1e6,
        e.pc(),
        e.machine().bus_error
    );

    assert!(reached, "never reached DISK-RECALIBRATE");
    // Pins how far the boot is. Counted in executed instructions, which mean
    // the same in both engines: `micro`'s step is a microinstruction and
    // `rtl`'s is a microcycle, so their *microcycle* counts differ by about
    // six per cent and are not comparable. Any change to the engine moves
    // it, and this says so.
    assert_eq!(to_disk, 416_736, "executed instructions to the first disk read");
    // Still going round it, and round the whole of it: `0o541`-`0o552` is
    // `DISK-RECALIBRATE` down to the off-line test, and `0o553` is the delay
    // slot behind that branch, fetched and nopped. Collecting the PCs rather
    // than sampling one says which loop the machine is in, where a single
    // sample only says it is somewhere in the region.
    assert_eq!(waiting_on_the_drive(&mut e), (0o541..=0o553).collect(), "the on-line wait");
    // The parity-fix overrun and the disk status reads are Xbus timeouts, and
    // **there is no Unibus timeout**: the one candidate is the write at
    // `PAGE-0-PARITY-FIX` that turns parity checking on, and the diagnostic
    // register it goes to answers.
    assert_eq!(
        e.machine().bus_error,
        muir::machine::bus_error::XBUS_NXM,
        "the Xbus overrun, and nothing on the Unibus"
    );
    // That write is `((MD) A-4)` --- `ERROR-STOP-ENABLE` on, and the PROM
    // emphatically still in place, which is why the machine got this far.
    assert_eq!(
        e.machine().mode,
        muir::spy::Mode { errstop: true, ..Default::default() },
        "the mode register after the PROM's first write to it"
    );
}

/// `SET-UP-FOUR-PAGES` builds the windows the PROM needs before it can talk to
/// anything: virtual page 0 onto physical page 0, page 1 onto the top of the
/// Xbus I/O region, page 2 onto the Unibus, page 3 onto physical page 1.
///
/// This is the check that the two-level map is being written correctly.  It
/// also explains the halt above: the parity-fix loop runs one word past the
/// end of page 0, and that word is in the Xbus I/O window.
#[test]
fn boot_prom_sets_up_four_pages() {
    let mut e = booted(&muir::prom::boot_prom());
    e.run(200_000_000);
    let m = e.machine();

    assert_eq!(m.l1_map.iter().filter(|&&x| x != 0).count(), 1984, "level-1 entries written");
    assert_eq!(m.l2_map.iter().filter(|&&x| x != 0).count(), 4, "level-2 entries written");

    for (vpage, physical) in [(0, 0), (1, 0o36777), (2, 0o37766), (3, 1)] {
        let t = m.translate(vpage * 256);
        assert_eq!(t.page, physical, "virtual page {vpage:o}");
        assert!(t.access_permitted && t.write_permitted, "virtual page {vpage:o} permissions");
    }
}

/// The `rtl` engine reaches the same place, and its throughput is the number
/// that matters for the fidelity/speed trade-off.
#[test]
fn rtl_engine_reaches_the_disk() {
    let mut m = Machine::new();
    m.load_prom(&muir::prom::boot_prom());
    let mut e = Rtl::new(m);
    e.boot();

    const CYCLES: u64 = 2_000_000;
    let started = Instant::now();
    let mut reached = false;
    for _ in 0..CYCLES {
        e.step().expect("rtl stopped for the wrong reason");
        if e.pc() == 0o541 {
            reached = true;
        }
    }
    let elapsed = started.elapsed();
    eprintln!(
        "rtl: {CYCLES} microcycles in {:.1?} ({:.1} M/s); PC {:o}, {} simulated ms",
        elapsed,
        CYCLES as f64 / elapsed.as_secs_f64() / 1e6,
        e.pc(),
        e.ns() / 1_000_000,
    );
    assert!(reached, "rtl never reached DISK-RECALIBRATE");
    assert_eq!(waiting_on_the_drive(&mut e), (0o541..=0o553).collect(), "the on-line wait");
    assert_eq!(
        e.machine().bus_error,
        muir::machine::bus_error::XBUS_NXM,
        "the same timeout `micro` sees, and the same silence on the Unibus"
    );
}

/// **The microcode the machine loads off the pack is the microcode in the
/// release.**
///
/// With a drive attached the whole boot PROM runs, and what it does is read
/// microcode 323 in off the disk: `READ-LABEL` decodes block 0, `SEARCH-LABEL`
/// finds the microload partition `MCR1` in the partition table, and `PROCESS-SECTION`
/// walks the sections in it --- `WRITE-I-MEM` for the control store,
/// `WRITE-DISPATCH-RAM` for the dispatch memory, `DISK-READ` for main memory
/// and the PDL buffer for A memory --- until `DONE-LOADING`.
///
/// So the control store afterwards can be compared against `sys/ubin/ucadr.mcr`,
/// the same microcode as a file. The two reach this test by entirely different
/// paths through the code: one through `src/mcr.rs`, the other through the
/// map, the bus, the disk controller, the drive, the label, the section
/// decoder and 12,449 `WRITE-I-MEM`s.
#[test]
fn the_boot_loads_microcode_323_off_the_pack() {
    let (Some(pack), Some(band)) =
        (support::pack_100(), support::release_100_file(&["ubin", "ucadr.mcr"]))
    else {
        return;
    };
    let mut e = Micro::new(machine_with_pack(&pack));
    e.boot();

    // The boot ends by writing 44 into Unibus location `766012`, which sets
    // `PROM-DISABLE` and takes the PROM out of the bottom of the control
    // store; `JUMP-TO-6` then waits for that write in a countdown loop the
    // PROM and the band share. So this runs to the write and stops there,
    // which is the last microcycle the PROM is in charge of.
    let mut executed = 0u64;
    while !e.machine().mode.prom_disable {
        e.step().expect("the PROM stopped before it could turn itself off");
        if e.executed().is_some() {
            executed += 1;
        }
    }
    assert_eq!(executed, 1_262_276, "executed instructions to PROM-DISABLE");
    assert_eq!(
        e.machine().mode,
        muir::spy::Mode { errstop: true, prom_disable: true, ..Default::default() },
        "ERROR-STOP-ENABLE and PROM-DISABLE, which is what 44 means"
    );
    // MIT put a halt at `I-MEM-LOC-10` for exactly the case where that write
    // does not take --- "Foo, should be in RAM by now" --- and the machine no
    // longer reaches it: location 6 comes from the control store, so the
    // countdown that ran was microcode 323's copy of it.
    assert_ne!(e.machine().fetch(0o10), e.machine().prom[0o10], "still fetching the PROM");

    let want = mcr::parse(&std::fs::read(band).unwrap()).unwrap();
    assert_eq!(want.imem.len(), 12_449, "microcode 323 is 0o30241 words");
    let m = e.machine();
    let at = want.imem_start as usize;
    assert!(
        m.imem[at..at + want.imem.len()] == want.imem[..],
        "the control store is not what the file holds"
    );
    // The engines keep the dispatch memory's seventeen data bits and drop
    // the parity bit the file carries above them. The board does store that
    // bit --- the 93425As at DRAM 1F16 and 1F17 --- but nothing can read it
    // back as data, so dropping it is unobservable; `src/micro.rs` has the
    // whole of it where the mask is applied.
    let at = want.dmem_start as usize;
    for (i, w) in want.dmem.iter().enumerate() {
        assert_eq!(m.dmem[at + i], w & 0o377777, "dispatch memory word {i:o}");
    }
    // A memory agrees in 1023 of its 1024 words. The odd one is `A-GARBAGE`,
    // which is where the PROM sends results it means to throw away --- it is
    // the first name in its own A-memory map --- so what stands in it at the
    // end of the boot is the PROM's leftovers and not the band's word.
    let at = want.amem_start as usize;
    for (i, w) in want.amem.iter().enumerate().skip(1) {
        assert_eq!(m.amem[at + i], *w, "A memory word {i:o}");
    }
    // The leftover is the last instruction the PROM runs: `((VMA-START-WRITE)
    // VMA)`, whose A destination field is zero, so `A-GARBAGE` catches the
    // address it wrote --- `0o1005`, the mode register's own.
    assert_eq!(m.amem[0], 0o1005, "A-GARBAGE holds the PROM's last leftover");
}
