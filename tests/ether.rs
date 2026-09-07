// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ether under interference. AIM-628 §2.3: the transceiver "detects
//! interference (another transceiver transmitting at the same time as
//! this one) and informs the interface". Frames due together start
//! together; a frame put on a busy cable by a transmitter that has not
//! heard it --- the board at its turn timer's instant, a transmitter with
//! cable between it and the rest --- overlaps what is there; two driving
//! high at once is interference, a transmitter whose clock edge finds it
//! aborts and holds the cable high for [`ABORT_HOLD_NS`], and what was
//! left on the cable reaches every receiver as wreckage, failing its check.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use muir::chaos::ether::{ABORT_HOLD_NS, ABORT_NS, Ether, Event, Node};
use muir::chaos::packet::Framed;
use muir::chaos::wire;

/// What happened to a host, in time.
#[derive(Debug, PartialEq, Eq)]
enum Happened {
    /// A frame heard: from whom, and whether its check word was good.
    Heard(u64, u16, bool),
    /// A frame handed back, aborted.
    Aborted(u64, Vec<u16>),
}

/// A host on the cable: sends what it is given, hears everything, and
/// offers an aborted frame again if told to, as an interface's driver
/// retries on Transmit Abort.
struct Host {
    address: u16,
    to_send: VecDeque<Vec<u16>>,
    retry: bool,
    log: Arc<Mutex<Vec<Happened>>>,
}

impl Node for Host {
    fn address(&self) -> u16 {
        self.address
    }
    fn receive(&mut self, now: u64, packet: &Framed) {
        self.log.lock().unwrap().push(Happened::Heard(now, packet.source, packet.check_ok));
    }
    fn transmit(&mut self, _now: u64) -> Option<Vec<u16>> {
        self.to_send.pop_front()
    }
    fn aborted(&mut self, now: u64, buffer: Vec<u16>) {
        self.log.lock().unwrap().push(Happened::Aborted(now, buffer.clone()));
        if self.retry {
            self.to_send.push_front(buffer);
        }
    }
}

fn host(
    address: u16,
    frames: Vec<Vec<u16>>,
    retry: bool,
) -> (Box<Host>, Arc<Mutex<Vec<Happened>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let h = Host { address, to_send: frames.into(), retry, log: log.clone() };
    (Box::new(h), log)
}

/// A packet buffer as the software writes it, `n` data words long: the
/// eight header words, the data, the cable destination last.
fn packet(from: u16, to: u16, n: usize) -> Vec<u16> {
    let mut w = vec![0o400, (2 * n) as u16, to, 0, from, 0o21, 1, 0];
    w.extend((0..n).map(|k| k as u16));
    w.push(to);
    w
}

/// Runs the ether from `now` to `until`, at every instant it has
/// something to do.
fn run(e: &mut Ether, mut now: u64, until: u64) {
    e.at(now);
    loop {
        match e.next_due() {
            Some(d) if d <= now => {
                now += 1;
                e.at(now);
            }
            Some(d) if d < until => {
                now = d;
                e.at(now);
            }
            _ => {
                e.at(until);
                return;
            }
        }
    }
}

fn sent(e: &Ether) -> Vec<(u64, u16)> {
    e.log
        .iter()
        .filter_map(|ev| if let Event::Sent(t, s, _) = ev { Some((*t, *s)) } else { None })
        .collect()
}

fn collisions(e: &Ether) -> Vec<u64> {
    e.log
        .iter()
        .filter_map(|ev| if let Event::Collision(t) = ev { Some(*t) } else { None })
        .collect()
}

fn heard(e: &Ether) -> Vec<(u64, u16, bool)> {
    e.log
        .iter()
        .filter_map(|ev| {
            if let Event::Heard(t, f) = ev { Some((*t, f.source, f.check_ok)) } else { None }
        })
        .collect()
}

/// **Two frames due together start together, and collide.** Two hosts
/// with nothing heard before them have the same turn --- one slot after
/// the cable is idle, for both --- so both start at that instant; their
/// first edges are high together, which is interference, and both stop
/// [`ABORT_NS`] on with their words handed back. Nothing came of it on
/// the cable: no frame decodes, and the cable is idle again after it.
#[test]
fn two_frames_due_together_start_together_and_collide() {
    let mut e = Ether::new();
    e.keep_log(true);
    let (a, log_a) = host(0o3060, vec![packet(0o3060, 0o3062, 4)], false);
    let (b, log_b) = host(0o3061, vec![packet(0o3061, 0o3062, 4)], false);
    let (c, log_c) = host(0o3062, vec![], false);
    e.attach(a);
    e.attach(b);
    e.attach(c);
    run(&mut e, 0, 100_000);
    let sent = sent(&e);
    eprintln!("sent: {sent:?}; collisions: {:?}", collisions(&e));
    assert_eq!(sent.len(), 2, "both went: {sent:?}");
    let t0 = sent[0].0;
    assert_eq!(sent[1].0, t0, "at the same instant");
    assert_eq!(collisions(&e), [t0], "and collided there: both first edges high");
    let stop = t0 + ABORT_NS;
    assert_eq!(
        log_a.lock().unwrap()[0],
        Happened::Aborted(stop, packet(0o3060, 0o3062, 4)),
        "3060 stopped and got its words back"
    );
    assert_eq!(
        log_b.lock().unwrap()[0],
        Happened::Aborted(stop, packet(0o3061, 0o3062, 4)),
        "and so did 3061"
    );
    // What was left on the cable reaches every receiver as wreckage, its
    // check bad, as a receiver takes it.
    let whole = |log: &[Happened]| log.iter().any(|h| matches!(h, Happened::Heard(_, _, true)));
    assert!(
        !whole(&log_c.lock().unwrap()),
        "3062 heard nothing whole: {:?}",
        log_c.lock().unwrap()
    );
    let h = heard(&e);
    assert!(h.len() == 1 && !h[0].2, "the wreckage, once, its check bad: {h:?}");
    assert!(!e.level(), "the cable is low");
    assert!(!e.busy(100_000), "and idle");
}

/// **A frame put on a busy cable collides, both stop, and the first is
/// offered again.** A host is a long way into a frame when a transmitter
/// that has not heard the cable starts another. Within a cell both are
/// high: interference, and the host's frame stops [`ABORT_NS`] on, its
/// words handed back; the wreckage decodes to nothing. The host offers
/// the frame again at its next turn, and this time it is heard whole.
#[test]
fn a_frame_put_on_a_busy_cable_collides_and_the_first_is_offered_again() {
    let mut e = Ether::new();
    e.keep_log(true);
    let long = packet(0o3060, 0o3062, 100);
    let (a, log_a) = host(0o3060, vec![long.clone()], true);
    let (c, log_c) = host(0o3062, vec![], false);
    e.attach(a);
    e.attach(c);
    run(&mut e, 0, 20_000);
    let [(s, 0o3060)] = sent(&e)[..] else { panic!("3060 went once: {:?}", sent(&e)) };
    assert!(e.busy(20_000), "a long frame is still going");

    let at = s + 50_000;
    run(&mut e, 20_000, at);
    e.send_now(at, 0o3061, packet(0o3061, 0o3062, 4));
    run(&mut e, at, at + 5_000);
    let sent_now = sent(&e);
    assert_eq!(sent_now.len(), 2, "the second went at once: {sent_now:?}");
    assert_eq!(sent_now[1], (at, 0o3061), "at its own instant, the cable busy");
    let [c] = collisions(&e)[..] else { panic!("one collision: {:?}", collisions(&e)) };
    assert!(
        (at..=at + wire::CELL_NS).contains(&c),
        "interference within a cell of the second's first edge: {c} for {at}"
    );
    assert_eq!(
        log_a.lock().unwrap()[..],
        [Happened::Aborted(c + ABORT_NS, long.clone())],
        "3060's frame stopped and came back"
    );
    // The transmitter that never heard the cable has no detector either:
    // its frame runs on alone, its head wreckage, and decodes to nothing;
    // 3060 offers its frame again at its turn once the cable is idle.
    let other_ends = at + 15 * 16 * wire::CELL_NS;
    run(&mut e, at + 5_000, other_ends);
    assert_eq!(sent(&e).len(), 2, "3060 waited for the cable: {:?}", sent(&e));
    run(&mut e, other_ends, at + 1_000_000);
    let again = sent(&e);
    assert_eq!(again.len(), 3, "3060 went again: {again:?}");
    assert_eq!(again[2].1, 0o3060);
    assert!(again[2].0 > other_ends, "after the other frame ran out: {again:?}");
    assert_eq!(collisions(&e).len(), 1, "with nothing in the way");
    let h = heard(&e);
    assert_eq!(h.len(), 2, "the wreckage was heard, then the frame: {h:?}");
    assert!(!h[0].2, "the wreckage, its check bad");
    assert_eq!((h[1].1, h[1].2), (0o3060, true), "the frame whole, its check word good");
    assert!(
        matches!(
            log_c.lock().unwrap()[..],
            [Happened::Heard(_, _, false), Happened::Heard(_, 0o3060, true)]
        ),
        "3062 took the wreckage as such, then the frame: {:?}",
        log_c.lock().unwrap()
    );
}

/// **The board's frame goes at its turn timer's instant, and the board is
/// told of the collision.** The behavioural board's turn timer decided at
/// its terminal count and its frame starts at the instant given, whether
/// or not a host has taken the cable since. Both stop; the board is told
/// when, and that its own frame was one of them, so its busy line and its
/// Transmit Abort can follow.
#[test]
fn the_boards_frame_goes_at_its_instant_and_the_board_is_told() {
    let mut e = Ether::new();
    e.keep_log(true);
    e.attach_board(0o3050);
    let (a, log_a) = host(0o3060, vec![packet(0o3060, 0o3050, 100)], false);
    e.attach(a);
    run(&mut e, 0, 20_000);
    let [(s, 0o3060)] = sent(&e)[..] else { panic!("3060 went once: {:?}", sent(&e)) };
    assert_eq!(
        e.board_cable_frame().map(|f| (f.0, f.1)),
        Some((s, 0o3060)),
        "the board heard it start"
    );

    let start = s + 50_000;
    e.board_send(packet(0o3050, 0o3060, 4), start);
    run(&mut e, 20_000, start + 5_000);
    let sent_now = sent(&e);
    assert_eq!(
        sent_now.get(1),
        Some(&(start, 0o3050)),
        "the board's frame went at its instant: {sent_now:?}"
    );
    let [c] = collisions(&e)[..] else { panic!("one collision: {:?}", collisions(&e)) };
    assert!(
        (start..=start + wire::CELL_NS).contains(&c),
        "interference within a cell: {c} for {start}"
    );
    let mut told = Vec::new();
    while let Some(x) = e.board_collision() {
        told.push(x);
    }
    told.sort();
    let (at, until) = (c + ABORT_NS, c + ABORT_NS + ABORT_HOLD_NS);
    assert_eq!(
        told,
        [(at, 0o3050, until), (at, 0o3060, until)],
        "both aborted, one of them the board's own, and hold the cable through their abort signal"
    );
    assert_eq!(e.board_sending_until(), None, "the board's frame is over");
    assert_eq!(
        e.board_cable_frame().map(|f| (f.0, f.1)),
        Some((start, 0o3050)),
        "and it started, for the turn timer"
    );
    assert!(matches!(log_a.lock().unwrap()[0], Happened::Aborted(t, _) if t == c + ABORT_NS));
    run(&mut e, start + 5_000, start + 200_000);
    let h = heard(&e);
    assert!(h.len() == 1 && !h[0].2, "the wreckage, its check bad: {h:?}");
    let (_, r) = e.board_heard().expect("the board heard the wreckage too");
    assert!(!r.framed.check_ok, "and can judge it by its check, and by its {} bits", r.bits);
}

/// **Interference, for the netlist board, is its transceiver and a model
/// transmitter driving high at once.** With a host's frame on the cable,
/// the board's transceiver driving high while the host's is high is
/// interference and a collision, and the host stops [`ABORT_NS`] on; the
/// board driving high while the host's transmitter is low, between
/// cells, is not.
#[test]
fn interference_is_two_transceivers_driving_high_at_once() {
    let mut e = Ether::new();
    e.keep_log(true);
    let (a, log_a) = host(0o3060, vec![packet(0o3060, 0o3050, 100)], false);
    e.attach(a);
    run(&mut e, 0, 1_000);
    let [(s, 0o3060)] = sent(&e)[..] else { panic!("3060 went once: {:?}", sent(&e)) };
    assert!(e.level(), "a frame's first edge takes the line high");
    // The first cell's mid-cell transition, if there is one, is at 125 ns;
    // a cell whose bit differs from the last has none. Find an instant in
    // the first few cells with the line low.
    let mut t = s;
    while e.level() {
        t += 5;
        e.at(t);
        assert!(t < s + 4 * wire::CELL_NS, "a low within four cells");
    }
    assert!(e.board(t, true), "the board driving high while the host is low: the cable goes high");
    assert!(!e.interference(), "and it is no interference");
    assert!(collisions(&e).is_empty());
    e.board(t, false);
    // Then high together.
    while !e.level() {
        t += 5;
        e.at(t);
        assert!(t < s + 8 * wire::CELL_NS, "a high within eight cells");
    }
    e.board(t, true);
    assert!(e.interference(), "both high: interference");
    assert_eq!(collisions(&e), [t]);
    run(&mut e, t, t + ABORT_NS);
    let x = match log_a.lock().unwrap()[..] {
        [Happened::Aborted(x, _)] => x,
        ref other => panic!("the host aborted once: {other:?}"),
    };
    assert!(x > t && x <= t + ABORT_NS, "at its next clock edge: {x} for {t}");
    assert!(e.interference(), "and holds the cable high through its abort signal");
    run(&mut e, t + ABORT_NS, x + ABORT_HOLD_NS);
    assert!(!e.interference(), "then lets go");
    e.board(x + ABORT_HOLD_NS, false);
    assert!(!e.level(), "and the line is low with the board's driver off too");
}
