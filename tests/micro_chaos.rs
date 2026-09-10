// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The band boots over the Chaosnet and reads a file over it, on `micro`
//! as on `rtl`.
//!
//! The Chaosnet is the machine's and not the engine's:
//! [`muir::machine::Machine::plug_chaos`] puts the interface on the I/O
//! board, and the board's registers answer any engine through
//! `Machine::read_physical` and `write_physical`. What the interface
//! wants from an engine is a clock, and `micro` has one --- the machine's
//! periods with [`muir::micro::MEMORY_ACCESS_NS`] a memory cycle, which
//! is five per cent off `rtl`'s measured waits over the boot and is the
//! accuracy this engine offers everywhere else.
//!
//! Whether that is close enough for the interface's own timing --- the
//! turn timer's 500 ns terminal counts, a frame's edges on the cable,
//! `RDONE` --- was the open question of issue #28, and this file is the
//! answer: the two engines have the same conversation with the server.
//!
//! `micro` cannot do any of it without the I/O board's clock, which is
//! issue #27 and `tests/ioboard_clock.rs`.

use std::sync::Mutex;

use muir::chaos::ether::Event;
use muir::chaos::packet::{Packet, op};
use muir::engine::Engine;
use muir::micro::Micro;
use muir::rtl::Rtl;
use muir::terminal::keyboard::Keyboard;

mod support;
use support::{
    CHAOS_100, ChaosServer, file_root, lit_rows, machine_with_pack, pack_100, time, type_at,
    wait_for_the_prompt,
};

/// The two machines here serve one directory, so they take turns: two
/// Chaosnet servers rooted at one tree are two hosts sharing a
/// filesystem, with nothing between them. These two only read, so it
/// would do no harm --- but a run under one root is a run at a time.
static SHARED_ROOT: Mutex<()> = Mutex::new(());

/// Every packet that crossed the machine's cable, either way.
fn packets<E: Engine>(e: &E) -> Vec<Packet> {
    let ether = e.machine().ioboard.chaos.as_ref().unwrap().ether().unwrap();
    ether
        .log
        .iter()
        .filter_map(|ev| match ev {
            Event::Sent(_, _, b) => Some(&b[..]),
            Event::Heard(_, f) => Some(&f.buffer[..]),
            Event::Collision(_) => None,
        })
        .filter_map(|b| Packet::from_buffer(b).ok().map(|(p, _)| p))
        .collect()
}

/// A data packet's text: the FILE service's commands are lines of ASCII.
fn text(p: &Packet) -> String {
    String::from_utf8_lossy(&p.data).into_owned()
}

/// **The band boots over the Chaosnet and reads a file, on `micro`.**
/// This is what settles issue #28.
#[test]
fn the_band_boots_over_the_chaosnet_on_micro() {
    let (Some(pack), Some(root)) = (pack_100(), file_root()) else {
        return;
    };
    boots_and_reads_a_file(Micro::new(machine_with_pack(&pack)), root, "micro");
}

/// **And on `rtl`**, which is the engine the behaviour is defined by.
/// It is here so that the two are held to the same conversation, rather
/// than `micro` being held to what someone thought it ought to say.
#[test]
fn the_band_boots_over_the_chaosnet_on_rtl() {
    let (Some(pack), Some(root)) = (pack_100(), file_root()) else {
        return;
    };
    boots_and_reads_a_file(Rtl::new(machine_with_pack(&pack)), root, "rtl");
}

fn boots_and_reads_a_file<E: Engine>(mut e: E, root: std::path::PathBuf, name: &str) {
    let _held = SHARED_ROOT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    e.boot();
    let m = e.machine_mut();
    // The band is System 100's, and it calls its file and time host at its
    // own host table's numbers. muir's defaults are on the private subnet
    // 376 and are no band's, so the pair is named here; without it the
    // band calls an address nothing answers at, and nothing this test
    // looks for goes on the cable at all.
    m.chaos.address = CHAOS_100.0;
    ChaosServer::new(CHAOS_100.1).serving(root).at_time(time::TEST_UNIVERSAL).plug(m, 0);
    // After the plug, so the log is this interface's own.
    m.ioboard.chaos.as_mut().unwrap().ether_mut().unwrap().keep_log(true);

    // Reaching the listener is the first half of the check: a band that
    // gets neither the time nor its files stops in the cold-load debugger
    // to ask for the date instead of reaching one.
    let t = std::time::Instant::now();
    let ran = wait_for_the_prompt(&mut e);
    let secs = t.elapsed().as_secs_f64();
    let ps = packets(&e);
    eprintln!(
        "{name}: the listener after {ran} microcycles in {secs:.1} s ({:.1} M/s), \
         {} packets on the cable",
        ran as f64 / secs / 1e6,
        ps.len()
    );

    let time = ps
        .iter()
        .find(|p| p.opcode == op::RFC && p.data.starts_with(b"TIME"))
        .expect("the band asked its time host for the time");
    assert_eq!(
        (time.source, time.dest),
        CHAOS_100,
        "{name}: from this machine to its file and time host"
    );
    assert!(
        ps.iter().any(|p| p.opcode == op::ANS && p.source == CHAOS_100.1),
        "{name}: and the server answered it"
    );
    assert_eq!(e.machine().bus_error, 0, "{name}: every cycle of the machine's was answered");

    // The second half, and the reason to want the Chaosnet on `micro` at
    // all: `FILE`, a stream protocol over many packets and many turns of
    // the cable, where `TIME` is one packet each way.
    //
    // Logged in first --- MIT's file service wants a user, and the login
    // is what opens the `FILE` connection; `tests/cc_harness` logs in
    // before asking for a file for the same reason.
    let mut k = Keyboard::new();
    type_at(&mut e, &mut k, "(login 'lispm)\n");
    for _ in 0..30_000_000 {
        e.step().expect("the machine halted");
    }
    let from = packets(&e).len();
    let before = lit_rows(&e, 0..muir::simpletv::HEIGHT);
    // No Return after it: the listener runs a form as soon as its last
    // parenthesis is in, and a Return typed after would sit in the
    // keyboard buffer as typeahead. `PROBE-FILE` is
    // `sys/io/file/open.lisp` --- the truename if the file is there,
    // `NIL` if it is not --- which is the smallest thing that is a whole
    // file operation.
    type_at(&mut e, &mut k, "(princ (probe-file \"SYS: CC; CCGSYL LISP\"))");
    let mut ran_more = 0u64;
    while ran_more < 200_000_000 {
        for _ in 0..1_000_000 {
            e.step().expect("the machine halted");
        }
        ran_more += 1_000_000;
        if packets(&e).len() > from && lit_rows(&e, 0..muir::simpletv::HEIGHT) != before {
            break;
        }
    }
    let ps = packets(&e);
    let after = &ps[from.min(ps.len())..];
    eprintln!("{name}: {} packets for the probe, {ran_more} microcycles on", after.len());
    for p in after {
        eprintln!("  {:o} -> {:o} opcode {:o}: {:?}", p.source, p.dest, p.opcode, text(p));
    }
    // The command as `sys/doc/chfile.text` has it: a transaction id, the
    // handle, the operation and its options.
    assert!(
        after.iter().any(|p| {
            op::is_data(p.opcode) && p.source == CHAOS_100.0 && text(p).contains("OPEN PROBE")
        }),
        "{name}: the band sent an OPEN PROBE over the file service"
    );
    assert!(
        after.iter().any(|p| op::is_data(p.opcode) && p.source == CHAOS_100.1),
        "{name}: and the service answered it"
    );
    assert_ne!(
        lit_rows(&e, 0..muir::simpletv::HEIGHT),
        before,
        "{name}: and the band printed what it found"
    );
    let png = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("vendor/run/micro-chaos-{name}.png"));
    std::fs::write(&png, e.machine().simpletv.png()).unwrap();
    eprintln!("{name}'s screen at {}", png.display());
    assert_eq!(e.machine().bus_error, 0, "{name}: every cycle of the machine's was answered");
}
