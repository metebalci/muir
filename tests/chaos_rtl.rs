// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Chaosnet interface as `rtl`'s I/O board has it, held to the netlist
//! board: the same register sequences run on both --- the `lm*` pages of
//! `data/CADRIO.netlist` under a Unibus master, and
//! `chaos::board::Interface` under the same reads and writes --- and every
//! CSR read, every word read back and every bit count must agree.  Times
//! are reported, not held: the behavioural interface takes a frame's bits
//! at the cable's rate and its turn, the board takes what its counters
//! take.

use muir::chaos::ether::{Capture, Ether, turn_byte};
use muir::chaos::interface::{self as chaos, csr};
use muir::chaos::packet::Packet;
use muir::chaos::server::{Server, op};
use muir::chaos::time::Time;
use muir::ioboard::IoBoard;
use muir::netlist::Netlist;
use muir::part::Level;
use muir::unibus::UnibusMaster;

mod support;
use support::{cadrio, quiet};

const MY_ADDRESS: u16 = 0o3050;
const SERVER: u16 = 0o3060;

fn board(n: &Netlist) -> UnibusMaster<'_> {
    let mut b = UnibusMaster::new(n, 10_000, &quiet());
    for (reference, closed) in chaos::switches(MY_ADDRESS) {
        b.chip.set_switches(reference, closed);
    }
    b.run(b.now + 2_000);
    b
}

/// The behavioural interface at the same address, with `ether` on its
/// cable or none, and a clock of its own.
struct Model {
    io: IoBoard,
    now: u64,
}

impl Model {
    fn new(ether: Option<Ether>) -> Model {
        let mut io = IoBoard::default();
        io.plug_chaos(MY_ADDRESS, ether, 0, false);
        Model { io, now: 10_000 }
    }

    /// A register cycle at `at`, the instant the board's own cycle ended:
    /// the two are held to one clock, so that what depends on the instant
    /// --- the turn timer --- can be compared.  Returns the word read.
    fn cycle(&mut self, uaddr: u32, write: Option<u16>, at: u64) -> u16 {
        self.now = at;
        match write {
            Some(v) => {
                self.io.write(uaddr, v, self.now);
                0
            }
            None => self.io.read(uaddr, self.now),
        }
    }

    fn run(&mut self, until: u64) {
        self.now = until;
        self.io.advance(self.now);
    }
}

/// One register cycle on both, the words compared when read.
fn both(b: &mut UnibusMaster, m: &mut Model, uaddr: u32, write: Option<u16>, what: &str) -> u16 {
    let (_, wb) = b.cycle(uaddr, write);
    let wm = m.cycle(uaddr, write, b.now);
    if write.is_none() {
        assert_eq!(wm, wb, "{what}: the board reads {wb:#o}, the model {wm:#o}");
    }
    wb
}

/// Waits on both until `bit` is up in the CSR, polling as CC's driver
/// would, the two polled at the same instants; the CSRs then, and the
/// time each took --- the same poll, if the model has the board's timing.
fn wait_for(
    b: &mut UnibusMaster,
    m: &mut Model,
    bit: u16,
    for_ns: u64,
) -> ((u64, u16), (u64, u16)) {
    let t0 = b.now;
    let (mut cb, mut cm) = (None, None);
    loop {
        b.run(b.now + 5_000);
        let (_, c) = b.cycle(chaos::CSR, None);
        let now = b.now;
        let c2 = m.cycle(chaos::CSR, None, now);
        if cb.is_none() && (c & bit != 0 || now >= t0 + for_ns) {
            cb = Some((now - t0, c));
        }
        if cm.is_none() && (c2 & bit != 0 || now >= t0 + for_ns) {
            cm = Some((now - t0, c2));
        }
        if let (Some(x), Some(y)) = (cb, cm) {
            return (x, y);
        }
    }
}

fn rfc_time(from: u16, to: u16) -> Vec<u16> {
    Packet {
        opcode: op::RFC,
        forward: 0,
        dest: to,
        dest_index: 0,
        source: from,
        source_index: 0o21,
        number: 1,
        ack: 0,
        data: b"TIME".to_vec(),
    }
    .to_buffer(to)
}

/// **A packet looped back comes out of both the same.** Reset, Loop Back
/// with Clear Receiver, the words in, START: Transmit Done and Receive
/// Done, no abort, no CRC error, the same bit count, the same words back
/// --- the packet, the destination, the source the interface put in, the
/// check word --- and the count `7777` once read out.
#[test]
fn a_packet_loops_back_through_both_alike() {
    let n = cadrio();
    let mut b = board(&n);
    let mut m = Model::new(None);
    let words = rfc_time(MY_ADDRESS, MY_ADDRESS);

    both(&mut b, &mut m, chaos::CSR, Some(csr::RESET), "reset");
    b.run(b.now + 2_000);
    m.run(b.now);
    let after_reset = both(&mut b, &mut m, chaos::CSR, None, "the CSR after reset");
    eprintln!("CSR after reset: {after_reset:#08o}");
    both(&mut b, &mut m, chaos::CSR, Some(csr::LOOP_BACK | csr::CLEAR_RECEIVER), "loop back");
    let c0 = both(&mut b, &mut m, chaos::CSR, None, "the CSR with loop back on");
    assert!(c0 & csr::LOOP_BACK != 0 && c0 & csr::RECEIVE_DONE == 0);
    both(&mut b, &mut m, chaos::MY_ADDRESS, None, "the address");
    for &w in &words {
        both(&mut b, &mut m, chaos::WRITE_BUFFER, Some(w), "a word in");
    }
    let c1 = both(&mut b, &mut m, chaos::CSR, None, "the CSR with a packet written");
    assert!(c1 & csr::TRANSMIT_DONE == 0, "writing clears Transmit Done: {c1:#08o}");
    let me = both(&mut b, &mut m, chaos::START, None, "START");
    assert_eq!(me, MY_ADDRESS);

    let ((tb, cb), (tm, cm)) = wait_for(&mut b, &mut m, csr::RECEIVE_DONE, 3_000_000);
    eprintln!("Receive Done: board after {tb} ns ({cb:#08o}), model after {tm} ns ({cm:#08o})");
    assert_eq!(cm, cb, "the CSR at Receive Done");
    assert_eq!(tm, tb, "Receive Done at the same poll on both: the turn timer's wait");
    assert!(
        cb & csr::TRANSMIT_DONE != 0 && cb & csr::TRANSMIT_ABORT == 0 && cb & csr::CRC_ERROR == 0
    );
    let bits = both(&mut b, &mut m, chaos::BIT_COUNT, None, "the bit count");
    assert_eq!(bits as usize, (words.len() + 2) * 16 - 1);
    let mut back = Vec::new();
    for k in 0..words.len() + 2 {
        back.push(both(&mut b, &mut m, chaos::READ_BUFFER, None, &format!("word {k} back")));
    }
    assert_eq!(&back[..words.len()], &words[..]);
    assert_eq!(back[words.len()], MY_ADDRESS, "the source both put in");
    assert_eq!(both(&mut b, &mut m, chaos::BIT_COUNT, None, "the count read out"), 0o7777);
    let c2 = both(&mut b, &mut m, chaos::CSR, None, "the CSR read out");
    assert!(c2 & csr::CRC_ERROR == 0);
    both(&mut b, &mut m, chaos::CSR, Some(csr::CLEAR_RECEIVER), "clear receiver");
    let c3 = both(&mut b, &mut m, chaos::CSR, None, "the CSR cleared");
    assert!(c3 & csr::RECEIVE_DONE == 0);
}

/// **A request for the time is answered on both.** A host at 3060 with a
/// fixed time on each cable; the interface sends `RFC TIME` and the answer
/// comes back into its receiver: the same CSR, the same bit count, the
/// same words --- an `ANS` from 3060 with the time, low byte first ---
/// and the server's address as the source.
#[test]
fn the_time_is_asked_and_answered_on_both_alike() {
    let n = cadrio();
    let mut b = board(&n);
    let ether_for = || {
        let mut server = Server::new(SERVER);
        server.serve(Box::new(Time::fixed(0x1234_5678)));
        let mut e = Ether::new();
        e.keep_log(true);
        e.attach(Box::new(server));
        e
    };
    b.plug_chaos(ether_for());
    let mut m = Model::new(Some(ether_for()));
    both(&mut b, &mut m, chaos::CSR, Some(csr::RESET), "reset");
    b.run(b.now + 2_000);
    m.run(b.now);
    both(&mut b, &mut m, chaos::CSR, Some(csr::CLEAR_RECEIVER), "clear receiver");
    let words = rfc_time(MY_ADDRESS, SERVER);
    for &w in &words {
        both(&mut b, &mut m, chaos::WRITE_BUFFER, Some(w), "a word in");
    }
    both(&mut b, &mut m, chaos::START, None, "START");
    let ((tb, cb), (tm, cm)) = wait_for(&mut b, &mut m, csr::RECEIVE_DONE, 3_000_000);
    eprintln!("the answer: board after {tb} ns ({cb:#08o}), model after {tm} ns ({cm:#08o})");
    if cm != cb {
        let e = m.io.chaos.as_ref().unwrap().ether().unwrap();
        for ev in &e.log {
            eprintln!("  model ether: {ev:?}");
        }
        eprintln!(
            "  model ether next due {:?}, sending until {:?}",
            e.next_due(),
            e.board_sending_until()
        );
    }
    assert_eq!(cm, cb, "the CSR at Receive Done");
    assert_eq!(tm, tb, "the answer at the same poll on both: both turn timers' waits");
    assert!(
        cb & csr::TRANSMIT_DONE != 0 && cb & csr::RECEIVE_DONE != 0 && cb & csr::CRC_ERROR == 0
    );
    let bits = both(&mut b, &mut m, chaos::BIT_COUNT, None, "the bit count");
    let count = (bits as usize + 1) / 16;
    let mut back = Vec::new();
    for k in 0..count {
        back.push(both(&mut b, &mut m, chaos::READ_BUFFER, None, &format!("word {k} back")));
    }
    let (ans, _) = Packet::from_buffer(&back[..count - 2]).unwrap();
    assert_eq!(ans.opcode, op::ANS);
    assert_eq!((ans.source, ans.dest, ans.dest_index), (SERVER, MY_ADDRESS, 0o21));
    assert_eq!(ans.data, [0x78, 0x56, 0x34, 0x12], "the universal time, low byte first");
    assert_eq!(back[count - 2], SERVER, "from the host");
    assert_eq!(both(&mut b, &mut m, chaos::BIT_COUNT, None, "the count read out"), 0o7777);
}

/// **A collision aborts the transmission on both alike.** Both interfaces
/// send the same packet; once both frames are on the cable, a transmitter
/// that has not heard either cable starts the same frame on each, at the
/// same instant. Both read Transmit Done with Transmit Abort, at the same
/// poll, and neither received anything.
#[test]
fn a_collision_aborts_the_transmission_on_both_alike() {
    let n = cadrio();
    let mut b = board(&n);
    let ether_for = || {
        let mut e = Ether::new();
        e.keep_log(true);
        e.attach(Box::new(Capture::new(SERVER)));
        e
    };
    b.plug_chaos(ether_for());
    let mut m = Model::new(Some(ether_for()));
    both(&mut b, &mut m, chaos::CSR, Some(csr::RESET), "reset");
    b.run(b.now + 2_000);
    m.run(b.now);
    both(&mut b, &mut m, chaos::CSR, Some(csr::CLEAR_RECEIVER), "clear receiver");
    let words = rfc_time(MY_ADDRESS, SERVER);
    for &w in &words {
        both(&mut b, &mut m, chaos::WRITE_BUFFER, Some(w), "a word in");
    }
    both(&mut b, &mut m, chaos::START, None, "START");
    let t0 = b.now;
    loop {
        b.run(b.now + 1_000);
        m.run(b.now);
        let on_b = b.chaos.as_ref().unwrap().ether.busy(b.now);
        let on_m = m.io.chaos.as_ref().unwrap().ether().unwrap().busy(b.now);
        if on_b && on_m {
            break;
        }
        assert!(b.now < t0 + 3_000_000, "both transmitted: board {on_b}, model {on_m}");
    }
    let at = b.now;
    eprintln!("both frames on the cable {} ns after START", at - t0);
    // For a third host, so that neither receiver has a say in this.
    let other = rfc_time(SERVER, 0o3070);
    b.chaos.as_mut().unwrap().ether.send_now(at, SERVER, other.clone());
    m.io.chaos.as_mut().unwrap().ether_mut().unwrap().send_now(at, SERVER, other);
    let ((tb, cb), (tm, cm)) = wait_for(&mut b, &mut m, csr::TRANSMIT_DONE, 3_000_000);
    eprintln!("Transmit Done: board after {tb} ns ({cb:#08o}), model after {tm} ns ({cm:#08o})");
    for ev in &m.io.chaos.as_ref().unwrap().ether().unwrap().log {
        eprintln!("  model ether: {ev:?}");
    }
    assert!(cb & csr::TRANSMIT_ABORT != 0, "the board's transmission was aborted: {cb:#08o}");
    // CRC Error is up on both: the receiver is in the wreckage.
    assert_eq!(cm, cb, "the CSR at Transmit Done");
    assert_eq!(tm, tb, "at the same poll on both");
    // The other frame runs out; neither took it, and the CSRs agree whole.
    b.run(b.now + 300_000);
    m.run(b.now);
    let c = both(&mut b, &mut m, chaos::CSR, None, "the CSR with the cable idle again");
    assert_eq!(
        c & (csr::RECEIVE_DONE | csr::TRANSMIT_DONE | csr::TRANSMIT_ABORT),
        csr::TRANSMIT_DONE | csr::TRANSMIT_ABORT
    );
}

/// **Wreckage addressed to the board lands on both alike.** The same two
/// frames, from transmitters that heard neither cable, go on each at the
/// same instants: one for the board, and sixty-four cells into it another
/// over it. Both boards take what arrives --- Receive Done with CRC Error
/// --- and read back the same bit count and the same words.
#[test]
fn wreckage_addressed_to_the_board_lands_on_both_alike() {
    use muir::chaos::wire;
    let n = cadrio();
    let mut b = board(&n);
    let ether_for = || {
        let mut e = Ether::new();
        e.keep_log(true);
        e.attach(Box::new(Capture::new(SERVER)));
        e
    };
    b.plug_chaos(ether_for());
    let mut m = Model::new(Some(ether_for()));
    both(&mut b, &mut m, chaos::CSR, Some(csr::RESET), "reset");
    b.run(b.now + 2_000);
    m.run(b.now);
    both(&mut b, &mut m, chaos::CSR, Some(csr::CLEAR_RECEIVER), "clear receiver");
    let at = b.now;
    let first = rfc_time(SERVER, MY_ADDRESS);
    b.chaos.as_mut().unwrap().ether.send_now(at, SERVER, first.clone());
    m.io.chaos.as_mut().unwrap().ether_mut().unwrap().send_now(at, SERVER, first);
    b.run(at + 64 * wire::CELL_NS);
    m.run(b.now);
    let over = rfc_time(0o3061, 0o3070);
    b.chaos.as_mut().unwrap().ether.send_now(b.now, 0o3061, over.clone());
    m.io.chaos.as_mut().unwrap().ether_mut().unwrap().send_now(b.now, 0o3061, over);
    let ((tb, cb), (tm, cm)) = wait_for(&mut b, &mut m, csr::RECEIVE_DONE, 1_000_000);
    eprintln!("Receive Done: board after {tb} ns ({cb:#08o}), model after {tm} ns ({cm:#08o})");
    assert!(
        cb & csr::RECEIVE_DONE != 0 && cb & csr::CRC_ERROR != 0,
        "the board took wreckage: {cb:#08o}"
    );
    assert_eq!(cm, cb, "the CSR at Receive Done");
    assert_eq!(tm, tb, "at the same poll on both");
    let bits = both(&mut b, &mut m, chaos::BIT_COUNT, None, "the bit count");
    // Every word the buffer holds, the partial one at the top included,
    // and the count as it comes down with each.
    let words = (bits as usize + 1).div_ceil(16);
    eprintln!("bit count {bits}: {words} words, the top one partial");
    let (mut from_board, mut from_model) = (Vec::new(), Vec::new());
    for k in 0..words {
        let (_, wb) = b.cycle(chaos::READ_BUFFER, None);
        from_board.push(wb);
        from_model.push(m.cycle(chaos::READ_BUFFER, None, b.now));
        both(&mut b, &mut m, chaos::BIT_COUNT, None, &format!("the count after word {k}"));
    }
    let octal = |v: &[u16]| v.iter().map(|w| format!("{w:#o}")).collect::<Vec<_>>();
    eprintln!("read back from the board: {:?}", octal(&from_board));
    eprintln!("read back from the model: {:?}", octal(&from_model));
    assert_eq!(from_model, from_board, "the words back, wreckage and all");
    assert_eq!(both(&mut b, &mut m, chaos::BIT_COUNT, None, "the count read out"), 0o7777);
    both(&mut b, &mut m, chaos::CSR, Some(csr::CLEAR_RECEIVER), "clear receiver");
    let c = both(&mut b, &mut m, chaos::CSR, None, "the CSR cleared");
    assert!(c & csr::RECEIVE_DONE == 0);
}

/// **After Reset alone, before any Clear Receiver, does a packet for this
/// address land?**  AIM-628 says the clear-receiver bit "enables the
/// receiver to receive another packet"; whether a reset interface's
/// receiver is enabled to begin with is the board's to say, and the model
/// must say the same.  A node on each cable sends the interface a packet
/// right after the reset.
#[test]
fn a_reset_interface_receives_or_not_before_clear_receiver_alike() {
    let n = cadrio();
    let mut b = board(&n);
    let words = rfc_time(SERVER, MY_ADDRESS);
    let ether_for = || {
        let mut node = Capture::new(SERVER);
        node.to_send.push_back(words.clone());
        let mut e = Ether::new();
        e.attach(Box::new(node));
        e
    };
    b.cycle(chaos::CSR, Some(csr::RESET));
    b.run(b.now + 2_000);
    let mut m = Model::new(None);
    m.cycle(chaos::CSR, Some(csr::RESET), m.now + 250);
    m.run(b.now);
    b.plug_chaos(ether_for());
    m.io.chaos.as_mut().unwrap().plug(ether_for());
    let ((tb, cb), (tm, cm)) = wait_for(&mut b, &mut m, csr::RECEIVE_DONE, 1_000_000);
    eprintln!("after reset alone: board {cb:#08o} after {tb} ns, model {cm:#08o} after {tm} ns");
    assert_eq!(cm & csr::RECEIVE_DONE, cb & csr::RECEIVE_DONE, "whether the packet landed");
    assert_eq!(cm & csr::LOST_COUNT, cb & csr::LOST_COUNT, "and whether it counted as lost");
}

/// **The turn timer loads the address difference, bit-reversed, and waits
/// that many counts and one.**  The board at four addresses sends the
/// host at 3060 an RFC and hears its answer; the 74LS164s' `SERVER ADR
/// DIFF` at each `-LOAD.MY.TURN` is read off the nets, and the counts of
/// `MY.TURN CLK^` from the cable going idle to `MY.TURN^` rising are
/// counted: for the board's own packet the load is 0 and the turn comes at
/// the first count; for the server's the low byte loaded is
/// `turn_byte(3060, me)` --- bits 14 to 7 of `3060 - me`, bit 14 at the
/// bottom --- and the turn comes that many counts and one after.
#[test]
fn the_turn_timer_loads_the_address_difference_bit_reversed() {
    let n = cadrio();
    for me in [0o1050u16, 0o2450, 0o3450, 0o3250] {
        let mut b = UnibusMaster::new(&n, 10_000, &quiet());
        for (reference, closed) in chaos::switches(me) {
            b.chip.set_switches(reference, closed);
        }
        b.run(b.now + 2_000);
        let mut server = Server::new(SERVER);
        server.serve(Box::new(Time::fixed(0x1234_5678)));
        let mut e = Ether::new();
        e.attach(Box::new(server));
        b.plug_chaos(e);
        b.cycle(chaos::CSR, Some(csr::RESET));
        b.run(b.now + 2_000);
        b.cycle(chaos::CSR, Some(csr::CLEAR_RECEIVER));
        let words = rfc_time(me, SERVER);
        for &w in &words {
            b.cycle(chaos::WRITE_BUFFER, Some(w));
        }
        let started = b.now;
        b.cycle(chaos::START, None);
        let diff = |b: &UnibusMaster| -> u16 {
            (0..12)
                .map(|k| ((b.level(&format!("'HOST ADR DIFF.{k}'")) == Level::High) as u16) << k)
                .sum()
        };
        let (mut cbl_was, mut turn_was, mut clk_was, mut load_was) = (
            b.level("-CBLBSY"),
            b.level("MY.TURN^"),
            b.level("'MY.TURN CLK^'"),
            b.level("-LOAD.MY.TURN"),
        );
        let mut loads = Vec::new();
        let mut counts: Vec<u32> = Vec::new();
        let mut idle_at = None;
        let mut busy_at = None;
        let mut rdone_was = b.level("RDONE");
        let mut counts_since_idle = 0u32;
        let mut packets = 0;
        let mut report = String::new();
        while b.now < started + 900_000 {
            b.run(b.now + 10);
            let ld = b.level("-LOAD.MY.TURN");
            if ld != load_was && ld == Level::Low {
                loads.push(diff(&b));
            }
            load_was = ld;
            let cbl = b.level("-CBLBSY");
            if cbl != cbl_was {
                if cbl == Level::High {
                    idle_at = Some(b.now);
                    counts_since_idle = 0;
                    packets += 1;
                } else {
                    busy_at = Some(b.now);
                }
                cbl_was = cbl;
            }
            let rd = b.level("RDONE");
            if rd != rdone_was
                && rd == Level::High
                && let Some(bz) = busy_at
            {
                report += &format!(" [RDONE {} ns after the frame's first edge]", b.now - bz);
            }
            rdone_was = rd;
            let ck = b.level("'MY.TURN CLK^'");
            if ck != clk_was && ck == Level::High && idle_at.is_some() {
                counts_since_idle += 1;
            }
            clk_was = ck;
            let tn = b.level("MY.TURN^");
            if tn != turn_was
                && tn == Level::High
                && let Some(i) = idle_at
            {
                report += &format!(
                    " [packet {packets}: MY.TURN^ up {} ns after idle, count {counts_since_idle}]",
                    b.now - i
                );
                counts.push(counts_since_idle);
                idle_at = None;
            }
            turn_was = tn;
        }
        let want = turn_byte(SERVER, me);
        eprintln!(
            "me {me:o} host {SERVER:o}: loads (DIFF at load) {:?}; turn_byte {want:#04x};{report}",
            loads.iter().map(|d| format!("{d:#05x}")).collect::<Vec<_>>(),
        );
        assert_eq!(loads.len(), 2, "a load for the board's own packet and one for the server's");
        assert_eq!(loads[0] & 0xff, 0, "the board's own packet loads 0");
        assert_eq!((loads[1] & 0xff) as u8, want, "the server's packet loads turn_byte");
        assert_eq!(counts, [1, want as u32 + 1], "the counts to the turn after each");
    }
}
