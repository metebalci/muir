// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The I/O board's Chaosnet half, run as MIT built it: the thirteen
//! `chaos/lispm` pages of `data/CADRIO.netlist`, driven through the
//! interface's registers by a Unibus master, and held to AIM-628 §7.
//!
//! These are the checks that come before any cable model: that the board
//! knows its own address, and that a packet written into its transmitter
//! comes out of its receiver with the check word right, looped back
//! inside the board with the cable unused.

use muir::chaos::interface::{self as chaos, csr};
use muir::netlist::{self, Netlist};
use muir::part::Level;
use muir::unibus::UnibusMaster;

mod support;
use support::quiet;

const CADRIO: &str = include_str!("../data/CADRIO.netlist");

fn cadrio() -> Netlist {
    netlist::parse_wired(CADRIO).unwrap()
}

/// This machine's address in the band's host table, `MIT-LISPM-1`.
const MY_ADDRESS: u16 = 0o3050;

/// A board with its switches set to [`MY_ADDRESS`], reset and settled.
fn board(n: &Netlist) -> UnibusMaster<'_> {
    let mut b = UnibusMaster::new(n, 10_000, &quiet());
    for (reference, closed) in chaos::switches(MY_ADDRESS) {
        b.chip.set_switches(reference, closed);
    }
    b.run(b.now + 2_000);
    b
}

/// **The address switches read back as the address.** `MY ADDRESS` at
/// 764142 is "the network address of this interface (which is contained
/// in a set of DIP switches on the board)"; set for 3050, it reads 3050,
/// which also settles which way round the switches are.
#[test]
fn the_address_switches_read_back() {
    let n = cadrio();
    let mut b = board(&n);
    let (took, word) = b.cycle(chaos::MY_ADDRESS, None);
    eprintln!("MY ADDRESS reads {word:#o} in {took} ns");
    assert_eq!(word, MY_ADDRESS, "the address the switches were set to");
    // And another address, so it is not a coincidence of the bit pattern.
    for (reference, closed) in chaos::switches(0o3060) {
        b.chip.set_switches(reference, closed);
    }
    b.run(b.now + 2_000);
    let (_, word) = b.cycle(chaos::MY_ADDRESS, None);
    assert_eq!(word, 0o3060, "switched to 3060");
    // The start-transmission address reads the same address.
    // (Not read here: it would start a transmission of an empty buffer.)
}

/// The words of a packet as the software writes them: the eight header
/// words, the data, then the destination.
fn rfc_time(from: u16, to: u16) -> Vec<u16> {
    let text = b"TIME";
    let mut words = vec![
        1 << 8,            // Operation: RFC, opcode 1 in the high byte
        text.len() as u16, // Count: forwarding count 0, byte count
        to,                // Destination address
        0,                 // Destination index: none yet
        from,              // Source address
        0o21,              // Source index
        1,                 // Packet number
        0,                 // Acknowledgement
    ];
    for pair in text.chunks(2) {
        words.push(pair[0] as u16 | (pair.get(1).copied().unwrap_or(0) as u16) << 8);
    }
    words.push(to);
    words
}

/// **A packet loops back through the board.** With Loop Back set, "the
/// cable and transceiver are not used and the interface is looped back to
/// itself." A packet written into the outgoing buffer and started comes
/// out of the incoming buffer: Transmit Done and Receive Done both up, no
/// CRC error, the bit count the packet's, and every word read back as
/// written, followed by the destination, the source the hardware put in,
/// and the check word.
/// Resets the interface, sets loop back, clears the receiver, writes
/// `words` into the transmitter and starts it. Returns when it started.
fn start_loopback(b: &mut UnibusMaster, words: &[u16]) -> u64 {
    b.cycle(chaos::CSR, Some(csr::RESET));
    b.run(b.now + 2_000);
    b.cycle(chaos::CSR, Some(csr::LOOP_BACK | csr::CLEAR_RECEIVER));
    let (_, csr0) = b.cycle(chaos::CSR, None);
    eprintln!("CSR after reset, loop back and clear receiver: {csr0:#08o}");
    assert!(csr0 & csr::LOOP_BACK != 0, "loop back reads back");
    assert!(csr0 & csr::RECEIVE_DONE == 0, "nothing received yet");
    for &w in words {
        b.cycle(chaos::WRITE_BUFFER, Some(w));
    }
    let (_, csr1) = b.cycle(chaos::CSR, None);
    assert!(csr1 & csr::TRANSMIT_DONE == 0, "writing a word clears Transmit Done: {csr1:#08o}");
    let started = b.now;
    let (_, me) = b.cycle(chaos::START, None);
    assert_eq!(me, MY_ADDRESS, "START reads the interface's own address");
    started
}

#[test]
fn a_packet_loops_back_through_the_board() {
    let n = cadrio();
    let mut b = board(&n);
    let words = rfc_time(MY_ADDRESS, MY_ADDRESS);
    let started = start_loopback(&mut b, &words);

    // Wait for the transmitter, then the receiver.
    let mut done = None;
    let mut received = None;
    while b.now < started + 3_000_000 {
        b.run(b.now + 5_000);
        let (_, c) = b.cycle(chaos::CSR, None);
        if done.is_none() && c & csr::TRANSMIT_DONE != 0 {
            done = Some((b.now - started, c));
        }
        if c & csr::RECEIVE_DONE != 0 {
            received = Some((b.now - started, c));
            break;
        }
    }
    let (t_done, c_done) = done.expect("Transmit Done never came up");
    eprintln!("Transmit Done after {t_done} ns, CSR {c_done:#08o}");
    assert!(c_done & csr::TRANSMIT_ABORT == 0, "the transmission was aborted");
    let (t_rcv, c_rcv) = received.expect("Receive Done never came up");
    eprintln!("Receive Done after {t_rcv} ns, CSR {c_rcv:#08o}");
    assert!(c_rcv & csr::CRC_ERROR == 0, "CRC error on the fresh packet: {c_rcv:#08o}");

    let (_, bits) = b.cycle(chaos::BIT_COUNT, None);
    let expect_words = words.len() + 2;
    eprintln!(
        "bit count {bits} ({} words + source + check = {} bits)",
        words.len(),
        expect_words * 16
    );
    assert_eq!(bits as usize, expect_words * 16 - 1, "the bit count minus one");

    let mut back = Vec::new();
    for _ in 0..expect_words {
        let (_, w) = b.cycle(chaos::READ_BUFFER, None);
        back.push(w);
    }
    eprintln!("read back: {:?}", back.iter().map(|w| format!("{w:#o}")).collect::<Vec<_>>());
    let (_, after) = b.cycle(chaos::BIT_COUNT, None);
    let (_, c_after) = b.cycle(chaos::CSR, None);
    eprintln!("after reading out: bit count {after:#o}, CSR {c_after:#08o}");
    assert_eq!(&back[..words.len()], &words[..], "the packet as written, destination last");
    assert_eq!(back[words.len()], MY_ADDRESS, "the source address the hardware put in");
    eprintln!("check word {:#o}", back[words.len() + 1]);
    assert_eq!(
        back[words.len() + 1],
        muir::chaos::packet::check_word(&back[..words.len() + 1]),
        "the check word the board made is the one the packet layer computes"
    );
    assert_eq!(after, 0o7777, "the bit count once read out");
    assert!(c_after & csr::CRC_ERROR == 0, "CRC error after reading out: {c_after:#08o}");
}

/// A read of 764142 done by hand, printing the decode chain at each step.
/// Run alone with `--ignored --nocapture`; it is a diagnostic, not a check.
#[test]
#[ignore = "a diagnostic, not a check: --ignored --nocapture"]
fn diagnose_the_chaos_reply() {
    let n = cadrio();
    let set = std::env::var("SET_SWITCHES").is_ok();
    let mut b = UnibusMaster::new(&n, 10_000, &quiet());
    if set {
        for (reference, closed) in chaos::switches(MY_ADDRESS) {
            b.chip.set_switches(reference, closed);
        }
        b.run(b.now + 2_000);
    }
    eprintln!("switches set: {set}");
    let watch = [
        "-A1*",
        "-A5*",
        "-A6*",
        "-A11*",
        "-MSYN*",
        "-SSYN*",
        "'CHAOS SSYN'",
        "-BOARD.SELECT",
        "'BOARD.SELECT FOR REAL'",
        "-SELECT.764140",
        "UBADDR1",
        "UBADDR2",
        "UBADDR3",
        "UBADDR4",
        "UBADDR5",
        "UBADDR6",
        "-11RCSR",
        "-11WCSR",
        "-11RRBTCT",
        "-TSR.SSYN",
        "-READ.DONE",
        "'-LOAD INTERVAL'",
        "HI2",
        "-C1*",
    ];
    let show = |b: &UnibusMaster, tag: &str| {
        let s: Vec<String> = watch
            .iter()
            .filter(|w| n.by_name_id(w).is_some())
            .map(|w| format!("{}={:?}", w.trim_matches('\''), b.level(w)))
            .collect();
        eprintln!("[{tag} @ {} ns] {}", b.now, s.join(" "));
    };
    show(&b, "rest");
    let uaddr: u32 = chaos::MY_ADDRESS;
    // Address bits 1..17 on -A1*..-A17*, active low; read: -C1* released.
    for k in 1..18u32 {
        let net = b.net(&format!("-A{k}*"));
        if (uaddr >> k) & 1 != 0 { b.chip.drive(net, Level::Low) } else { b.chip.pull_up(net) }
    }
    let c1 = b.net("-C1*");
    b.chip.pull_up(c1);
    b.chip.transition(b.now);
    b.run(b.now + 100);
    show(&b, "address up");
    let msyn = b.net("-MSYN*");
    b.chip.drive(msyn, Level::Low);
    b.chip.transition(b.now);
    show(&b, "MSYN");
    for _ in 0..3 {
        b.run(b.now + 250);
        show(&b, "wait");
    }
    // the bus receivers for the address
    for p in n.parts.iter().filter(|p| p.page == "IOBXCV" && p.reference == "0F07") {
        let s: Vec<String> = p
            .pins
            .iter()
            .map(|&(pin, net)| format!("p{pin}={}={:?}", n.net(net), b.chip.net(net)))
            .collect();
        eprintln!("IOBXCV 0F07 {}: {}", p.kind, s.join(" "));
    }
    // the decoder's outputs, whatever they are called
    for pg_ref in ["0C18", "0D15"] {
        for p in n.parts.iter().filter(|p| p.page == "LMUCON" && p.reference == pg_ref) {
            let s: Vec<String> = p
                .pins
                .iter()
                .map(|&(pin, net)| format!("p{pin}={}={:?}", n.net(net), b.chip.net(net)))
                .collect();
            eprintln!("LMUCON {pg_ref} {}: {}", p.kind, s.join(" "));
        }
    }
}

/// The transmitter after START, printing every watched net that moved.
/// A diagnostic: `--ignored --nocapture`.
#[test]
#[ignore = "a diagnostic, not a check: --ignored --nocapture"]
fn diagnose_the_transmitter() {
    let n = cadrio();
    let mut b = board(&n);
    let words = rfc_time(MY_ADDRESS, MY_ADDRESS);
    let watch = [
        "LOOP.BACK",
        "-11XMT",
        "TSREMPTY",
        "-TSRLOAD",
        "-TSR.SSYN",
        "-TDONE",
        "-TRESET",
        "-RESET",
        "SR_UNIBUS",
        "CW",
        "TIWEND^",
        "TDATA",
        "-TTL.D.OUT",
        "TTL.D.IN",
        "-RCVR.DATA.IN",
        "RDATA",
        "MY.TURN^",
        "-LOAD.MY.TURN",
        "-CBLBSY",
        "'-CBLBSY A'",
        "'MATCH SO FAR'",
        "LOCKOUT",
        "-EDGE",
        "-INTERFERENCE",
        "GENCLK",
        "SAMPLE",
    ];
    let clocks = ["MCLK^", "'FCLK^'", "FCLK/2^", "TICLK^", "TOCLK^", "'MY.TURN CLK^'", "'PRICLK^'"];
    let mut last: Vec<Level> = watch.iter().map(|w| b.level(w)).collect();
    let tbct = |b: &UnibusMaster| -> u16 {
        (0..12).map(|k| ((b.level(&format!("TBCT{k}")) == Level::High) as u16) << k).sum()
    };
    let diff = |b: &UnibusMaster| -> u16 {
        (0..12)
            .map(|k| ((b.level(&format!("'HOST ADR DIFF.{k}'")) == Level::High) as u16) << k)
            .sum()
    };
    let started = start_loopback(&mut b, &words);
    eprintln!(
        "started at {started}; clocks now: {}",
        clocks.iter().map(|c| format!("{c}={:?}", b.level(c))).collect::<Vec<_>>().join(" ")
    );
    let mut edges = vec![0u32; clocks.len()];
    let mut prev_clk: Vec<Level> = clocks.iter().map(|c| b.level(c)).collect();
    let (mut last_tbct, mut last_diff) = (tbct(&b), diff(&b));
    eprintln!("[{}] TBCT={last_tbct} DIFF={last_diff}", b.now - started);
    let end = started + 120_000;
    while b.now < end {
        let next = b.chip.next_tap().map_or(b.now + 50, |t| t.max(b.now + 1)).min(b.now + 500);
        b.run(next);
        let t = b.now - started;
        let mut changes = Vec::new();
        for (k, w) in watch.iter().enumerate() {
            let l = b.level(w);
            if l != last[k] {
                changes.push(format!("{}={:?}", w.trim_matches('\''), l));
                last[k] = l;
            }
        }
        for (k, c) in clocks.iter().enumerate() {
            let l = b.level(c);
            if l == Level::High && prev_clk[k] != Level::High {
                edges[k] += 1;
            }
            prev_clk[k] = l;
        }
        let (tb, df) = (tbct(&b), diff(&b));
        if tb != last_tbct || df != last_diff {
            changes.push(format!("TBCT={tb} DIFF={df}"));
            last_tbct = tb;
            last_diff = df;
        }
        if !changes.is_empty() {
            eprintln!("[{t:>7}] {}", changes.join(" "));
        }
    }
    eprintln!(
        "rising edges seen in {} ns: {}",
        end - started,
        clocks
            .iter()
            .zip(&edges)
            .map(|(c, e)| format!("{}={e}", c.trim_matches('\'')))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let (_, c) = b.cycle(chaos::CSR, None);
    eprintln!("CSR at the end: {c:#08o}");
}

/// The receiver during a looped-back transmission: slow nets printed as
/// they move, fast ones counted. A diagnostic: `--ignored --nocapture`.
#[test]
#[ignore = "a diagnostic, not a check: --ignored --nocapture"]
fn diagnose_the_receiver() {
    let n = cadrio();
    let mut b = board(&n);
    let words = rfc_time(MY_ADDRESS, MY_ADDRESS);
    let slow = [
        "'D OUT'",
        "-TTL.D.OUT",
        "CBLBSY",
        "'-CBLBSY A'",
        "PRACT",
        "RACT",
        "START^",
        "RRESET",
        "-RDONE",
        "'RCV BUSY'",
        "-GOT.ONE",
        "ITS.ME",
        "'DEST MATCH'",
        "'MATCH SO FAR'",
        "'SRC STB'",
        "PRIWBEG",
        "CRCERR",
        "ABORTSIG",
        "ABORTDN",
        "'11 RD BF'",
        "-READ.DONE",
        "-ROWEND",
        "LOCKOUT",
        "RDATA",
        "-TDONE",
        "MATCH.ANY.DEST",
        "-CRC.PRE",
        "-RACT",
    ];
    let fast = [
        "TTL.D.IN",
        "-EDGE",
        "GENCLK",
        "'GENCLK END'",
        "SDLYD",
        "SDLYD2",
        "SAMPLE",
        "'LOCKOUT END'",
        "RICLK^",
        "'PRICLK^'",
        "ROCLK^",
        "RO.FIRST^",
        "TOCLK^",
    ];
    let counter = |b: &UnibusMaster, name: &str, bits: u32| -> u16 {
        (0..bits).map(|k| ((b.level(&format!("{name}{k}")) == Level::High) as u16) << k).sum()
    };
    // The detector's parts: every pin's net, its name, level and drivers.
    for (page, reference) in [
        ("LMDETC", "0B02"),
        ("LMDETC", "0C02"),
        ("LMDETC", "0E08"),
        ("LMDETC", "0C01"),
        ("LMDETC", "0B01"),
        ("LMDETC", "0E03"),
    ] {
        for p in n.parts.iter().filter(|p| p.page == page && p.reference == reference) {
            let s: Vec<String> = p
                .pins
                .iter()
                .map(|&(pin, net)| {
                    format!(
                        "p{pin}=[{net}]{}={:?}/{}drv",
                        n.net(net),
                        b.chip.net(net),
                        b.chip.drivers_on(net).len()
                    )
                })
                .collect();
            eprintln!("{page} {reference} {}: {}", p.kind, s.join(" "));
        }
    }
    let started = start_loopback(&mut b, &words);
    let mut last: Vec<Level> = slow.iter().map(|w| b.level(w)).collect();
    eprintln!(
        "after START: {}",
        slow.iter()
            .zip(&last)
            .map(|(w, l)| format!("{}={:?}", w.trim_matches('\''), l))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let mut prev_fast: Vec<Level> = fast.iter().map(|c| b.level(c)).collect();
    let mut rises = vec![0u32; fast.len()];
    let (mut last_rbct, mut last_prbct) = (counter(&b, "RBCT", 12), counter(&b, "PRBCT", 6));
    eprintln!("RBCT={last_rbct} PRBCT={last_prbct}");
    let mut line_edges_shown = 0;
    let end = started + 400_000;
    let mut done_at = None;
    while b.now < end {
        let next = b.chip.next_tap().map_or(b.now + 50, |t| t.max(b.now + 1)).min(b.now + 500);
        b.run(next);
        let t = b.now - started;
        let mut changes = Vec::new();
        for (k, w) in slow.iter().enumerate() {
            let l = b.level(w);
            if l != last[k] {
                changes.push(format!("{}={:?}", w.trim_matches('\''), l));
                last[k] = l;
            }
        }
        for (k, c) in fast.iter().enumerate() {
            let l = b.level(c);
            if l != prev_fast[k] {
                if l == Level::High {
                    rises[k] += 1;
                }
                if k < 8 && line_edges_shown < 60 {
                    changes.push(format!("{}={:?}", c.trim_matches('\''), l));
                    line_edges_shown += 1;
                }
            }
            prev_fast[k] = l;
        }
        let (rb, pb) = (counter(&b, "RBCT", 12), counter(&b, "PRBCT", 6));
        if rb != last_rbct || pb != last_prbct {
            changes.push(format!("RBCT={rb} PRBCT={pb}"));
            last_rbct = rb;
            last_prbct = pb;
        }
        if !changes.is_empty() {
            eprintln!("[{t:>7}] {}", changes.join(" "));
        }
        if done_at.is_none() && b.level("-TDONE") == Level::Low {
            done_at = Some(t);
        }
        if let Some(d) = done_at
            && t > d + 20_000
        {
            break;
        }
    }
    eprintln!(
        "rising edges: {}",
        fast.iter()
            .zip(&rises)
            .map(|(c, e)| format!("{}={e}", c.trim_matches('\'')))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let (_, c) = b.cycle(chaos::CSR, None);
    let (_, bits) = b.cycle(chaos::BIT_COUNT, None);
    eprintln!("CSR at the end: {c:#08o}, bit count {bits:#o}");
}

use muir::chaos::ether::{Capture, Ether, Event};

/// The board reset and its receiver cleared, with the cable in use: no
/// loop back.
fn on_the_cable(b: &mut UnibusMaster) {
    b.cycle(chaos::CSR, Some(csr::RESET));
    b.run(b.now + 2_000);
    b.cycle(chaos::CSR, Some(csr::CLEAR_RECEIVER));
}

/// Runs until `bit` is up in the CSR, or `for_ns` have passed; the CSR.
fn wait_for(b: &mut UnibusMaster, bit: u16, for_ns: u64) -> u16 {
    let until = b.now + for_ns;
    loop {
        b.run(b.now + 5_000);
        let (_, c) = b.cycle(chaos::CSR, None);
        if c & bit != 0 || b.now >= until {
            return c;
        }
    }
}

/// **The model hears what the board sends.** With a capture node at 3060
/// on the ether and no loop back, the board transmits a packet for 3060.
/// The ether decodes it off the board's own line driver --- the biphase
/// cells, the reversed word order, the check word --- and it is the
/// packet as written, from 3050, checking good. And the board, which
/// hears its own packet as every transceiver does, does not receive it:
/// it is not addressed to 3050.
#[test]
fn the_model_hears_what_the_board_sends() {
    let n = cadrio();
    let mut b = board(&n);
    on_the_cable(&mut b);
    let mut ether = Ether::new();
    ether.keep_log(true);
    ether.attach(Box::new(Capture::new(0o3060)));
    b.plug_chaos(ether);
    let words = rfc_time(MY_ADDRESS, 0o3060);
    for &w in &words {
        b.cycle(chaos::WRITE_BUFFER, Some(w));
    }
    let started = b.now;
    b.cycle(chaos::START, None);
    let c = wait_for(&mut b, csr::TRANSMIT_DONE, 3_000_000);
    eprintln!("Transmit Done after {} ns, CSR {c:#08o}", b.now - started);
    assert!(c & csr::TRANSMIT_DONE != 0, "the board transmitted");
    assert!(c & csr::TRANSMIT_ABORT == 0, "and was not aborted");
    b.run(b.now + 5_000);
    let log = &b.chaos.as_ref().unwrap().ether.log;
    eprintln!("ether log: {log:?}");
    let heard: Vec<_> = log
        .iter()
        .filter_map(|e| if let Event::Heard(t, f) = e { Some((t, f)) } else { None })
        .collect();
    assert_eq!(heard.len(), 1, "one packet went by");
    let (_, f) = heard[0];
    assert_eq!(f.buffer, words, "the packet as the board's software wrote it");
    assert_eq!(f.source, MY_ADDRESS, "from the board");
    assert!(f.check_ok, "and its check word is good: {:#o}", f.check);
    assert!(!log.iter().any(|e| matches!(e, Event::Collision(_))), "no collision");
    let (_, c) = b.cycle(chaos::CSR, None);
    assert!(c & csr::RECEIVE_DONE == 0, "not for 3050, so not received: {c:#08o}");
}

/// **The board receives what the model sends.** A packet for 3050 from a
/// node at 3060, framed and coded by the ether, arrives in the board's
/// receive buffer: Receive Done, no CRC error, the bit count, and the
/// words read back as sent, then the destination, the source 3060 and a
/// check word the board itself accepted.
#[test]
fn the_board_receives_what_the_model_sends() {
    let n = cadrio();
    let mut b = board(&n);
    on_the_cable(&mut b);
    let words = rfc_time(0o3060, MY_ADDRESS);
    let mut node = Capture::new(0o3060);
    node.to_send.push_back(words.clone());
    let mut ether = Ether::new();
    ether.attach(Box::new(node));
    let started = b.now;
    b.plug_chaos(ether);
    let c = wait_for(&mut b, csr::RECEIVE_DONE, 1_000_000);
    eprintln!("Receive Done after {} ns, CSR {c:#08o}", b.now - started);
    assert!(c & csr::RECEIVE_DONE != 0, "the board received");
    assert!(c & csr::CRC_ERROR == 0, "with its check good: {c:#08o}");
    let (_, bits) = b.cycle(chaos::BIT_COUNT, None);
    assert_eq!(bits as usize, (words.len() + 2) * 16 - 1, "the bit count");
    let mut back = Vec::new();
    for _ in 0..words.len() + 2 {
        let (_, w) = b.cycle(chaos::READ_BUFFER, None);
        back.push(w);
    }
    eprintln!("read back: {:?}", back.iter().map(|w| format!("{w:#o}")).collect::<Vec<_>>());
    assert_eq!(&back[..words.len()], &words[..], "the packet as sent");
    assert_eq!(back[words.len()], 0o3060, "from the node");
    assert_eq!(back[words.len() + 1], muir::chaos::packet::check_word(&back[..words.len() + 1]));
    let (_, c) = b.cycle(chaos::CSR, None);
    assert!(c & csr::CRC_ERROR == 0, "and still good once read out: {c:#08o}");
}

/// **The board takes broadcasts and leaves other hosts' packets alone.**
/// AIM-628 §2.5: the destination "is compared by each receiver against
/// its own address. If they match, or if the destination is zero ... the
/// packet is placed in the receive packet buffer".
#[test]
fn the_board_takes_broadcasts_and_not_others_packets() {
    let n = cadrio();
    let mut b = board(&n);
    on_the_cable(&mut b);
    let mut node = Capture::new(0o3060);
    node.to_send.push_back(rfc_time(0o3060, 0o3070));
    let mut ether = Ether::new();
    ether.keep_log(true);
    ether.attach(Box::new(node));
    b.plug_chaos(ether);
    let c = wait_for(&mut b, csr::RECEIVE_DONE, 400_000);
    assert!(c & csr::RECEIVE_DONE == 0, "a packet for 3070 is not received: {c:#08o}");
    assert_eq!(
        b.chaos.as_ref().unwrap().ether.log.iter().filter(|e| matches!(e, Event::Sent(..))).count(),
        1,
        "it did go by"
    );
    // Now a broadcast.
    let words = rfc_time(0o3060, 0);
    let mut node = Capture::new(0o3060);
    node.to_send.push_back(words.clone());
    let mut ether = Ether::new();
    ether.attach(Box::new(node));
    b.plug_chaos(ether);
    let c = wait_for(&mut b, csr::RECEIVE_DONE, 1_000_000);
    assert!(c & csr::RECEIVE_DONE != 0, "a broadcast is received: {c:#08o}");
    assert!(c & csr::CRC_ERROR == 0);
    let mut back = Vec::new();
    for _ in 0..words.len() + 2 {
        let (_, w) = b.cycle(chaos::READ_BUFFER, None);
        back.push(w);
    }
    assert_eq!(&back[..words.len()], &words[..]);
}

use muir::chaos::packet::Packet;
use muir::chaos::server::{Server, op};
use muir::chaos::time::Time;

/// **TIME, over the board.** What the Lisp Machine's `SERVER-TIME` does
/// through these registers, done through them: an RFC to `TIME` at 3060
/// written into the transmitter and started; the host on the ether
/// answers; the board receives the ANS, and its four data bytes are the
/// universal time, least significant first. The whole stack between the
/// software and the service, with MIT's board in the middle.
#[test]
fn the_board_asks_the_time_and_is_answered() {
    let n = cadrio();
    let mut b = board(&n);
    on_the_cable(&mut b);
    let mut server = Server::new(0o3060);
    server.serve(Box::new(Time::fixed(0x1234_5678)));
    let mut ether = Ether::new();
    ether.keep_log(true);
    ether.attach(Box::new(server));
    b.plug_chaos(ether);
    let ask = Packet {
        opcode: op::RFC,
        forward: 0,
        dest: 0o3060,
        dest_index: 0,
        source: MY_ADDRESS,
        source_index: 0o21,
        number: 0o1234,
        ack: 0,
        data: b"TIME".to_vec(),
    };
    let words = ask.to_buffer(0o3060);
    for &w in &words {
        b.cycle(chaos::WRITE_BUFFER, Some(w));
    }
    let started = b.now;
    b.cycle(chaos::START, None);
    let c = wait_for(&mut b, csr::RECEIVE_DONE, 3_000_000);
    eprintln!("Receive Done after {} ns, CSR {c:#08o}", b.now - started);
    assert!(c & csr::TRANSMIT_DONE != 0, "the RFC went out");
    assert!(c & csr::RECEIVE_DONE != 0, "and an answer came back: {c:#08o}");
    assert!(c & csr::CRC_ERROR == 0);
    let (_, bits) = b.cycle(chaos::BIT_COUNT, None);
    let count = (bits as usize + 1) / 16;
    let mut back = Vec::new();
    for _ in 0..count {
        let (_, w) = b.cycle(chaos::READ_BUFFER, None);
        back.push(w);
    }
    let (ans, _) = Packet::from_buffer(&back[..count - 2]).unwrap();
    eprintln!("answer: {ans:?}");
    assert_eq!(ans.opcode, op::ANS);
    assert_eq!((ans.source, ans.dest, ans.dest_index), (0o3060, MY_ADDRESS, 0o21));
    assert_eq!(ans.ack, 0o1234);
    assert_eq!(ans.data, [0x78, 0x56, 0x34, 0x12], "the universal time, low byte first");
    assert_eq!(back[count - 2], 0o3060, "from the host");
    let log = &b.chaos.as_ref().unwrap().ether.log;
    assert!(!log.iter().any(|e| matches!(e, Event::Collision(_))), "no collision: {log:?}");
}

/// **The board aborts its transmission on interference.** With its frame
/// on the cable, a transmitter that has not heard the cable starts
/// another: the model's, at its own instant. The transceiver reports
/// interference the moment both drive high, `COLLISION` is `TBUSY` with
/// it (the 74S02 at LMMODU 0B08), the `ABORT` flip-flop at 0A09 takes it
/// on `-FCLK^`, and from there the driver is off, `TABORTED` is up and
/// Transmit Done comes: the CSR reads Transmit Done and Transmit Abort.
/// The model transmitter stops too, [`ABORT_NS`] after the interference,
/// and nothing whole was on the cable. The board's nets are watched
/// through it, for the instants the behavioural interface is held to.
#[test]
fn the_board_aborts_its_transmission_on_interference() {
    use muir::chaos::ether::{ABORT_NS, Capture, Ether, Event};
    let n = cadrio();
    let mut b = board(&n);
    let mut ether = Ether::new();
    ether.keep_log(true);
    ether.attach(Box::new(Capture::new(0o3060)));
    b.plug_chaos(ether);
    on_the_cable(&mut b);
    let words = rfc_time(MY_ADDRESS, 0o3060);
    for &w in &words {
        b.cycle(chaos::WRITE_BUFFER, Some(w));
    }
    let started = b.now;
    b.cycle(chaos::START, None);
    // Until the board's frame is on the cable.
    while !b.chaos.as_ref().unwrap().ether.busy(b.now) {
        b.run(b.now + 250);
        assert!(b.now < started + 3_000_000, "the board transmitted");
    }
    let on = b.now;
    eprintln!("the board's frame on the cable {} ns after START", on - started);
    b.run(on + 5_000);
    let at = b.now;
    // For a third host, so that the board's receiver has no say in this.
    b.chaos.as_mut().unwrap().ether.send_now(at, 0o3060, rfc_time(0o3060, 0o3070));
    // The board's nets from there, as they change.
    let names = ["'INTERFERENCE IN'", "COLLISION", "ABORT", "TBUSY", "TABORTED", "TDONE"];
    let nets: Vec<_> = names.iter().map(|name| b.net(name)).collect();
    let read = |b: &UnibusMaster| -> Vec<Level> { nets.iter().map(|&id| b.chip.net(id)).collect() };
    let mut last = read(&b);
    let mut driving = b.chaos.as_ref().unwrap().board_tx(&b.chip);
    let mut seen = Vec::new();
    while b.now < at + 20_000 {
        b.run(b.now + 25);
        let now = read(&b);
        for (k, (was, is)) in last.iter().zip(&now).enumerate() {
            if was != is {
                eprintln!("  +{:>6} ns: {} {:?}", b.now - at, names[k], is);
                seen.push((b.now - at, names[k], *is));
            }
        }
        last = now;
        let d = b.chaos.as_ref().unwrap().board_tx(&b.chip);
        if d != driving {
            eprintln!("  +{:>6} ns: the driver {}", b.now - at, if d { "on" } else { "off" });
            driving = d;
        }
    }
    for ev in &b.chaos.as_ref().unwrap().ether.log {
        match ev {
            Event::Sent(t, s, _) => eprintln!("  ether: {s:o} sent at +{}", *t as i64 - at as i64),
            Event::Collision(t) => eprintln!("  ether: collision at +{}", *t as i64 - at as i64),
            Event::Heard(t, f) => {
                eprintln!("  ether: heard from {:o} at +{}", f.source, *t as i64 - at as i64)
            }
        }
    }
    let up = |name: &str| {
        seen.iter().find(|&&(_, n, l)| n == name && l == Level::High).map(|&(t, _, _)| t)
    };
    let interference = up("'INTERFERENCE IN'").expect("the transceiver reported interference");
    let abort = up("ABORT").expect("ABORT set");
    let taborted = up("TABORTED").expect("TABORTED set");
    let tdone = up("TDONE").expect("TDONE set");
    eprintln!(
        "interference at +{interference}, ABORT +{abort}, TABORTED +{taborted}, TDONE +{tdone}: \
         TDONE {} ns after ABORT",
        tdone - abort
    );
    assert!(
        abort - interference <= 16 * ABORT_NS,
        "ABORT within a few cells of the first interference: an edge of -FCLK^ found it"
    );
    let c = wait_for(&mut b, csr::TRANSMIT_DONE, 3_000_000);
    assert!(c & csr::TRANSMIT_DONE != 0, "Transmit Done: {c:#08o}");
    assert!(c & csr::TRANSMIT_ABORT != 0, "and Transmit Abort: {c:#08o}");
    let log = &b.chaos.as_ref().unwrap().ether.log;
    assert!(
        log.iter().any(|e| matches!(e, Event::Collision(_))),
        "a collision on the cable: {log:?}"
    );
    assert!(
        !log.iter().any(|e| matches!(e, Event::Heard(_, f) if f.check_ok)),
        "nothing whole was heard: {log:?}"
    );
    // The other transmitter, with no detector, runs its frame out alone.
    b.run(b.now + 200_000);
    assert!(!b.chaos.as_ref().unwrap().ether.busy(b.now), "the cable is idle again");
}

/// The turn timer's nets from START to Transmit Done under Loop Back, two
/// packets running: `MY.TURN^`, the loads, the cable-busy line and the
/// transmitter's start and end.  Run alone with `--ignored --nocapture`;
/// it is a diagnostic, not a check.
#[test]
#[ignore = "a diagnostic, not a check: --ignored --nocapture"]
fn diagnose_the_turn_timer() {
    let n = cadrio();
    let mut b = board(&n);
    b.cycle(chaos::CSR, Some(csr::RESET));
    b.run(b.now + 2_000);
    b.cycle(chaos::CSR, Some(csr::LOOP_BACK | csr::CLEAR_RECEIVER));
    let words = rfc_time(MY_ADDRESS, MY_ADDRESS);
    let watch = [
        "MY.TURN^",
        "-LOAD.MY.TURN",
        "-CBLBSY",
        "'-CBLBSY A'",
        "-TDONE",
        "TSREMPTY",
        "-11XMT",
        "LOCKOUT",
        "-INTERFERENCE",
        "-TRESET",
        "-RESET",
        "SR_UNIBUS",
    ];
    for round in 0..2 {
        for &w in &words {
            b.cycle(chaos::WRITE_BUFFER, Some(w));
        }
        let started = b.now;
        b.cycle(chaos::START, None);
        let mut last: Vec<Level> = watch.iter().map(|w| b.level(w)).collect();
        let mut out_last = b.level("-TTL.D.OUT");
        let (mut out_edges, mut out_first, mut out_final) = (0u64, None, None);
        let mut turn_clk_last = b.level("'MY.TURN CLK^'");
        let mut turn_clk = 0u64;
        let diff = |b: &UnibusMaster| -> u16 {
            (0..8)
                .map(|k| ((b.level(&format!("'HOST ADR DIFF.{k}'")) == Level::High) as u16) << k)
                .sum()
        };
        eprintln!("round {round}: START at {started}; HOST ADR DIFF {}", diff(&b));
        while b.now < started + 320_000 {
            b.run(b.now + 50);
            let now: Vec<Level> = watch.iter().map(|w| b.level(w)).collect();
            for (k, w) in watch.iter().enumerate() {
                if now[k] != last[k] {
                    eprintln!(
                        "  [{:>7}] {w} -> {:?}  (turn clocks {turn_clk}, diff {})",
                        b.now - started,
                        now[k],
                        diff(&b)
                    );
                }
            }
            last = now;
            let o = b.level("-TTL.D.OUT");
            if o != out_last {
                out_edges += 1;
                out_first.get_or_insert(b.now - started);
                out_final = Some(b.now - started);
            }
            out_last = o;
            let t = b.level("'MY.TURN CLK^'");
            if t != turn_clk_last && t == Level::High {
                turn_clk += 1;
            }
            turn_clk_last = t;
            if b.level("-TDONE") == Level::Low {
                eprintln!(
                    "  -TDONE low at {}; -TTL.D.OUT edges {out_edges} from {out_first:?} to {out_final:?}",
                    b.now - started
                );
                break;
            }
        }
        eprintln!(
            "  -TTL.D.OUT edges {out_edges} from {out_first:?} to {out_final:?}; turn clocks {turn_clk}"
        );
        b.run(b.now + 5_000);
        let (_, c) = b.cycle(chaos::CSR, None);
        eprintln!("  CSR {c:#08o}");
        let (_, bits) = b.cycle(chaos::BIT_COUNT, None);
        for _ in 0..(bits as usize + 1) / 16 {
            b.cycle(chaos::READ_BUFFER, None);
        }
        b.cycle(chaos::CSR, Some(csr::LOOP_BACK | csr::CLEAR_RECEIVER));
    }
}
