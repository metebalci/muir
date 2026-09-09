// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The display's vertical interrupt on a booted System 100: the microcode's
//! "roughly-60-cycle clock", which is what runs `TRACK-MOUSE`.
//!
//! Needs the vendored pack, symbol table and file root; without them the
//! test says it was skipped.

use muir::engine::Engine;
use muir::rtl::Rtl;
use muir::simpletv::{FRAME_NS, mode};
use muir::sym::{self, Space};
use muir::terminal::keyboard::Keyboard;

mod support;
use support::{CHAOS_100, boot_to_the_prompt, machine_with_pack, type_at, vendor};

/// **The band's cold boot leaves the vertical interrupt off, and
/// `(SI:SETUP-CPT)` turns it on; then the microcode counts frames on it.**
///
/// At the listener's prompt the mode register is 0 and the sync RAM is
/// empty: `LISP-REINITIALIZE` in this release runs its `SETUP-CPT` block
/// under `(UNLESS (NOT CALLED-BY-USER) ...)`, which the cold boot's
/// `(LISP-REINITIALIZE NIL)` does not satisfy. So the PROM's sync program
/// keeps running and `MODE INTR ENB` stays clear, and with the disk idle
/// the microcode's `60CYC` never runs.
///
/// System 304 does it in the cold boot instead --- `sys/ltop.lisp` there
/// has the block as a function, `TV::INITIALIZE-RUN-LIGHT-LOCATIONS`,
/// registered `:BEFORE-COLD` --- so that band reaches the listener with
/// mode `14` and the sync RAM already in. Booting it is how this was
/// found; the state it arrives in is the state typing gets to here.
///
/// `(SI:SETUP-CPT)` typed at the listener loads the program into the sync
/// RAM through registers 1 to 3, turns the RAM in with register 3's bit
/// 7, and writes mode `14`: black-on-white and the interrupt enable. From
/// then on every [`FRAME_NS`] the flag sets, the interface presents `XBUS
/// INTR`, `INTRX0` finds bit 4, clears it and runs `60CYC`, whose first act
/// with the disk idle is to add one to `A-DISK-IDLE-TIME`. Twenty frames,
/// twenty counts, give or take the one in flight at either end.
#[test]
fn setup_cpt_enables_the_vertical_interrupt_and_the_microcode_counts_frames() {
    let (Some(pack), Some(symbols), Some(root)) = (
        support::pack_100(),
        support::release_100_file(&["ubin", "ucadr.sym"]),
        vendor(&["run", "file-root"]),
    ) else {
        return;
    };
    let symbols = sym::parse(&std::fs::read_to_string(symbols).unwrap()).unwrap();
    let idle = symbols.address(Space::AMem, "A-DISK-IDLE-TIME").expect("A-DISK-IDLE-TIME") as usize;

    let mut e = Rtl::new(machine_with_pack(&pack));
    e.boot();
    // The band is System 100's, and it calls its file and time host at
    // its own host table's numbers rather than at muir's defaults, which
    // are on the private subnet 376 and no band's.
    let ran = boot_to_the_prompt(&mut e, CHAOS_100, root);
    let tv = &e.machine().simpletv;
    eprintln!(
        "at the prompt after {ran}: mode {:o}, sync RAM enable {:o}",
        tv.mode(),
        tv.sync.enable
    );
    assert_eq!(tv.mode() & mode::INTERRUPT_ENABLE, 0, "the cold boot did not enable the interrupt");
    assert!(!tv.sync.enabled(), "nor load the sync RAM");
    let idle_before = e.machine().amem[idle] & 0o77777777;
    let t0 = e.machine().ns;
    while e.machine().ns < t0 + 20 * FRAME_NS {
        e.step().expect("halted");
    }
    let idle_still = e.machine().amem[idle] & 0o77777777;
    assert_eq!(idle_still, idle_before, "and with the disk idle, 60CYC never runs");

    let mut k = Keyboard::new();
    type_at(&mut e, &mut k, "(si:setup-cpt)\n");
    for _ in 0..10_000_000 {
        e.step().expect("halted after SETUP-CPT");
    }
    let tv = &e.machine().simpletv;
    let loaded = tv.sync.words().iter().filter(|&&w| w != 0).count();
    eprintln!(
        "after (si:setup-cpt): mode {:o}, sync RAM {loaded} words loaded, enable {:o}",
        tv.mode(),
        tv.sync.enable
    );
    assert_ne!(tv.mode() & mode::INTERRUPT_ENABLE, 0, "SETUP-CPT enabled the vertical interrupt");
    assert_ne!(tv.mode() & mode::BOW, 0, "and set black-on-white");
    assert!(tv.sync.enabled(), "and turned the sync RAM in");
    assert!(loaded > 100, "and loaded a program into it: {loaded} words");

    let before = e.machine().amem[idle] & 0o77777777;
    let t0 = e.machine().ns;
    while e.machine().ns < t0 + 20 * FRAME_NS {
        e.step().expect("halted while counting frames");
    }
    let after = e.machine().amem[idle] & 0o77777777;
    eprintln!("A-DISK-IDLE-TIME {before} -> {after} over twenty frames");
    let frames = after.wrapping_sub(before);
    assert!((18..=22).contains(&frames), "one 60CYC per frame, twenty frames: counted {frames}");
}
