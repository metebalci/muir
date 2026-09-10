// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Chaosnet over UDP: the frame's bytes, and the node that carries it on
//! and off the modelled cable.
//!
//! The frame is pinned here as a byte sequence rather than left implicit
//! in the packing code, because its byte order is **unverified** ---
//! `chaos::udp::PACKET_ORDER` and `chaos::udp::TRAILER_ORDER` say what
//! is believed and why --- and a correction should be a change to two
//! constants and to one test.

use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod support;

use support::Run;

use muir::chaos::ether::{Ether, Event, Node, SLOT_NS};
use muir::chaos::packet::{self, Framed, Packet, op};
use muir::chaos::udp::{self, Link};

use support::{ChaosServer, time};

/// This machine, and the Chaosnet server standing in for its band's file
/// and time host. System 100's band's pair, which is what a run of that
/// pack gives `--chaos-address` --- not what `chaos::Config::default`
/// holds, that being subnet 376's and no band's.
///
/// Under `muir` only [`ME`] is on the cable: the server is the harness's,
/// and a real run reaches its host over this very link. The tests here
/// that build an [`Ether`] of their own put both on it, which is what
/// [`Link::node`]'s `local` list is for.
const ME: u16 = 0o3050;
const SERVER: u16 = 0o3060;
/// A peer over UDP, and one this machine was never told about.
const PEER: u16 = 0o3040;
const STRANGER: u16 = 0o3041;

// --- the frame ----------------------------------------------------------

/// One known packet: an RFC for `STATUS` from [`PEER`] to [`ME`], six
/// bytes of data. Its buffer, cable destination last, as the software
/// writes it.
fn status_rfc() -> Packet {
    Packet {
        opcode: op::RFC,
        forward: 0,
        dest: ME,
        dest_index: 0,
        source: PEER,
        source_index: 0o21,
        number: 1,
        ack: 0,
        data: b"STATUS".to_vec(),
    }
}

/// The source and check word the hardware adds to `buffer`.
fn trailer(buffer: &[u16], source: u16) -> (u16, u16) {
    let mut over = buffer.to_vec();
    over.push(source);
    (source, packet::check_word(&over))
}

/// **The frame is these bytes.** Four header bytes --- version 1,
/// function 1, two arguments --- then the Chaos packet's eight header
/// words and its data, each word least significant byte first, and then
/// the hardware trailer's destination, source and check word, each most
/// significant byte first.
///
/// The mixed order is what makes this worth pinning, and the data is
/// what shows it is not arbitrary: AIM-628 §3.6 puts the first byte of a
/// pair in the word's least significant half, so `STATUS` comes out of a
/// little-endian frame as `STATUS` and out of a big-endian one as
/// `TSTASU`. Bytes 20 to 25 below read `STATUS`.
///
/// The check word `0o171007` is the CADR's own hardware CRC-16 over
/// these words, `packet::check_word`; what a CHUDP peer puts in that
/// field is unverified, and nothing here or in `chaos::udp` drops a
/// packet on it.
#[test]
fn the_frame_is_these_bytes() {
    let p = status_rfc();
    let buffer = p.to_buffer(ME);
    let (source, check) = trailer(&buffer, PEER);
    assert_eq!(check, 0o171007, "the hardware's check word for these words");
    let frame = udp::wrap(&buffer, source, check).expect("a frame");
    #[rustfmt::skip]
    let want: [u8; 32] = [
        // version, function, and two argument bytes
        0x01, 0x01, 0x00, 0x00,
        0x00, 0x01, // opcode: RFC in the high byte of the word
        0x06, 0x00, // count: no forwarding, six data bytes
        0x28, 0x06, // destination 3050
        0x00, 0x00, // destination index
        0x20, 0x06, // source 3040
        0x11, 0x00, // source index 21
        0x01, 0x00, // packet number
        0x00, 0x00, // acknowledge
        b'S', b'T', b'A', b'T', b'U', b'S',
        0x06, 0x28, // the trailer, in network order: destination 3050
        0x06, 0x20, // source 3040
        0xf2, 0x07, // check word 171007
    ];
    assert_eq!(frame, want, "the frame as it goes into the datagram");
    assert_eq!(&frame[20..26], b"STATUS", "the data reads in order, which is the packet's order");
}

/// **The frame goes out and comes back.** What `wrap` writes, `unwrap`
/// reads: the buffer with the cable destination last, the source, and
/// the check word, which is the hardware's here and so checks good.
#[test]
fn a_frame_goes_out_and_comes_back() {
    for data in [Vec::new(), b"STATUS".to_vec(), b"odd".to_vec(), vec![0xff; 488]] {
        let p = Packet { data, ..status_rfc() };
        let buffer = p.to_buffer(ME);
        let (source, check) = trailer(&buffer, PEER);
        let frame = udp::wrap(&buffer, source, check).expect("a frame");
        let back = udp::unwrap(&frame).expect("it reads back");
        assert_eq!(back, Framed { buffer, source, check, check_ok: true }, "{p:?}");
        assert_eq!(Packet::from_buffer(&back.buffer).expect("a packet").0, p);
    }
}

/// **An odd data count is taken padded or not.** The data is a whole
/// number of 16-bit words on the cable, so this pads it; the trailer is
/// found from the end of the datagram rather than from the count, so a
/// peer that does not pad is read all the same. Which a peer does is
/// **unverified**; one interoperation settles it, and until then neither
/// reading is refused.
#[test]
fn an_odd_data_count_is_taken_padded_or_not() {
    let p = Packet { data: b"odd".to_vec(), ..status_rfc() };
    let buffer = p.to_buffer(ME);
    let (source, check) = trailer(&buffer, PEER);
    let padded = udp::wrap(&buffer, source, check).expect("a frame");
    assert_eq!(padded.len(), 4 + 16 + 4 + 6, "the data padded to a word");
    // The same frame with the pad byte taken out.
    let mut unpadded = padded.clone();
    unpadded.remove(4 + 16 + 3);
    assert_eq!(unpadded.len(), 4 + 16 + 3 + 6);
    let a = udp::unwrap(&padded).expect("padded reads");
    let b = udp::unwrap(&unpadded).expect("unpadded reads too");
    assert_eq!(a.buffer, b.buffer, "and to the same words");
    assert_eq!(Packet::from_buffer(&b.buffer).expect("a packet").0.data, b"odd");
}

/// **A version this does not speak is refused, and so is a function.**
/// The protocol's author has said a version 2 may differ from version 1
/// in nothing but byte order, so a version 2 peer read as a version 1
/// one would exchange nonsense; it is refused by number instead.
#[test]
fn an_unknown_version_is_refused() {
    let p = status_rfc();
    let buffer = p.to_buffer(ME);
    let (source, check) = trailer(&buffer, PEER);
    let good = udp::wrap(&buffer, source, check).expect("a frame");
    assert!(udp::unwrap(&good).is_ok());
    for (byte, what) in [(0, "version"), (1, "function")] {
        for value in [0u8, 2, 255] {
            let mut bad = good.clone();
            bad[byte] = value;
            let err = udp::unwrap(&bad).expect_err("{what} {value} is refused");
            assert!(err.contains(what), "the refusal says which: {err}");
        }
    }
}

/// **A length that does not answer the data count is refused**, which is
/// what a wrong byte order looks like on the first packet: the count
/// word comes out of the other half and no longer accounts for the
/// datagram.
#[test]
fn a_length_that_is_not_the_count_is_refused() {
    let p = status_rfc();
    let buffer = p.to_buffer(ME);
    let (source, check) = trailer(&buffer, PEER);
    let good = udp::wrap(&buffer, source, check).expect("a frame");
    let mut short = good.clone();
    short.truncate(good.len() - 2);
    udp::unwrap(&short).expect_err("two bytes fewer than the count wants");
    let mut long = good.clone();
    long.extend([0, 0]);
    udp::unwrap(&long).expect_err("two more");
    // The count word swapped, which is the wrong order for that field.
    let mut swapped = good.clone();
    swapped.swap(6, 7);
    udp::unwrap(&swapped).expect_err("an absurd count");
    udp::unwrap(&good[..HEADER_AND_TRAILER]).expect_err("nothing but a header and a trailer");
}

/// The header and the trailer with no packet between them: shorter than
/// any frame.
const HEADER_AND_TRAILER: usize = 4 + 6;

// --- the node on the cable ----------------------------------------------

/// What a station on the cable heard whole: from whom, and the buffer
/// the software would read back.
type Log = Arc<Mutex<Vec<(u16, Vec<u16>)>>>;

/// A station on the model cable, standing in for the machine: hears
/// everything, sends what it is given, and answers the first frame it
/// hears with `reply` if it has one.
struct Host {
    address: u16,
    to_send: VecDeque<Vec<u16>>,
    reply: Option<Vec<u16>>,
    heard: Log,
}

impl Node for Host {
    fn address(&self) -> u16 {
        self.address
    }
    fn receive(&mut self, _now: u64, packet: &Framed) {
        if packet.check_ok {
            self.heard.lock().unwrap().push((packet.source, packet.buffer.clone()));
            self.to_send.extend(self.reply.take());
        }
    }
    fn transmit(&mut self, _now: u64) -> Option<Vec<u16>> {
        self.to_send.pop_front()
    }
}

fn host(address: u16, frames: Vec<Vec<u16>>) -> (Box<Host>, Log) {
    let heard: Log = Arc::new(Mutex::new(Vec::new()));
    (Box::new(Host { address, to_send: frames.into(), reply: None, heard: heard.clone() }), heard)
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

/// How far the ether is run between looks at the socket. The node takes
/// what has arrived when it is asked for a frame, and it is asked when
/// the cable is idle, so this is one ask.
const STEP: u64 = 100_000;

/// Runs the ether a [`STEP`] at a time until `done`, or gives up: a
/// datagram on the loopback is quick but not instant, and this is the
/// only place a test waits on the host's network.
fn run_until(e: &mut Ether, done: impl Fn(&Ether) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut now = 0;
    while Instant::now() < deadline {
        run(e, now, now + STEP);
        now += STEP;
        if done(e) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

/// The frames the ether put on the cable: when, from whom.
fn sent(e: &Ether) -> Vec<(u64, u16)> {
    e.log
        .iter()
        .filter_map(|ev| if let Event::Sent(t, s, _) = ev { Some((*t, *s)) } else { None })
        .collect()
}

/// A socket on the loopback standing in for a peer, and where it is.
fn peer_socket() -> (UdpSocket, SocketAddr) {
    let s = UdpSocket::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).expect("a socket");
    s.set_read_timeout(Some(Duration::from_millis(200))).expect("a timeout");
    let at = s.local_addr().expect("where it is");
    (s, at)
}

/// A link bound on the loopback at a port the host chooses, with these
/// peers.
fn link(peers: Vec<(u16, SocketAddr)>, dynamic: bool) -> Link {
    Link::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), peers, dynamic).expect("a link")
}

/// One datagram from `from` to `to`, carrying `p` addressed on the cable
/// to `cable_dest` and sourced there by `hardware_source`.
fn send_packet(from: &UdpSocket, to: SocketAddr, p: &Packet, cable_dest: u16, hardware: u16) {
    let buffer = p.to_buffer(cable_dest);
    let (source, check) = trailer(&buffer, hardware);
    let frame = udp::wrap(&buffer, source, check).expect("a frame");
    from.send_to(&frame, to).expect("it goes");
}

/// **A peer's packet reaches the cable, and waits its turn to do it.**
/// The node is a station on the model ether like any other: it is asked
/// for a frame when the cable is idle and its frame starts at its turn,
/// one slot on, rather than being pushed onto the cable whenever a
/// datagram happens to land. A transmitter that did not wait would put
/// the frame on at the instant it was asked, and the board would see a
/// transceiver that has failed.
///
/// **And it goes on as the peer**: the hardware source is 3040, the
/// address of the host whose packet it is, exactly as that host's own
/// interface would have put it there.
#[test]
fn a_peers_packet_reaches_the_cable_at_its_turn() {
    let (peer, peer_at) = peer_socket();
    let l = link(vec![(PEER, peer_at)], false);
    let mut e = Ether::new();
    e.keep_log(true);
    let (machine, heard) = host(ME, vec![]);
    e.attach(machine);
    e.attach(Box::new(l.node(&[ME, SERVER], false)));
    let p = status_rfc();
    send_packet(&peer, l.at, &p, ME, PEER);
    assert!(run_until(&mut e, |e| !sent(e).is_empty()), "the packet reached the cable");
    let sent = sent(&e);
    assert_eq!(sent.len(), 1, "once: {sent:?}");
    let (at, from) = sent[0];
    assert_eq!(from, PEER, "on the cable as the host it came from");
    assert_eq!(
        at % STEP,
        SLOT_NS,
        "at its turn, one slot after it was asked, not at the instant it was asked: {at}"
    );
    let heard = heard.lock().unwrap();
    assert_eq!(heard.len(), 1, "and the machine heard it: {heard:?}");
    assert_eq!(heard[0].0, PEER);
    assert_eq!(Packet::from_buffer(&heard[0].1).expect("a packet").0, p);
}

/// **A frame the cable carries for a peer goes out as a datagram**, and
/// arrives as the packet that was written.
#[test]
fn a_frame_for_a_peer_goes_out_as_a_datagram() {
    let (peer, peer_at) = peer_socket();
    let l = link(vec![(PEER, peer_at)], false);
    let mut e = Ether::new();
    e.keep_log(true);
    let p = Packet { source: ME, dest: PEER, ..status_rfc() };
    let (machine, _) = host(ME, vec![p.to_buffer(PEER)]);
    e.attach(machine);
    e.attach(Box::new(l.node(&[ME, SERVER], false)));
    assert!(run_until(&mut e, |e| !sent(e).is_empty()), "the machine's frame went");
    let mut buf = [0u8; 1024];
    let (n, from) = peer.recv_from(&mut buf).expect("the datagram arrives");
    assert_eq!(from, l.at, "from the link's own socket");
    let f = udp::unwrap(&buf[..n]).expect("it reads");
    assert_eq!(f.source, ME, "the hardware source the cable carried");
    assert!(f.check_ok, "and the check word the cable's hardware made");
    assert_eq!(Packet::from_buffer(&f.buffer).expect("a packet"), (p, PEER));
}

/// **muir is a leaf: a packet for a third party is dropped, not
/// forwarded.** AIM-628 chapter 6's routing is a bridge's job; a
/// `cbridge` beside muir does it. Nothing addressed to 3041 reaches this
/// cable, and nothing goes back out.
#[test]
fn a_packet_for_a_third_party_is_dropped() {
    let (peer, peer_at) = peer_socket();
    let l = link(vec![(PEER, peer_at), (STRANGER, peer_at)], false);
    let mut e = Ether::new();
    e.keep_log(true);
    let (machine, heard) = host(ME, vec![]);
    e.attach(machine);
    e.attach(Box::new(l.node(&[ME, SERVER], false)));
    // Addressed on the cable to 3041, which is not on this cable.
    let p = Packet { dest: STRANGER, ..status_rfc() };
    send_packet(&peer, l.at, &p, STRANGER, PEER);
    // And one for this machine after it, to show the node went on
    // working rather than stopping at the first.
    send_packet(&peer, l.at, &status_rfc(), ME, PEER);
    assert!(run_until(&mut e, |e| !sent(e).is_empty()), "the second reached the cable");
    let heard = heard.lock().unwrap();
    assert_eq!(heard.len(), 1, "one frame, not two: {heard:?}");
    assert_eq!(Packet::from_buffer(&heard[0].1).expect("a packet").0.dest, ME);
    assert!(peer.recv_from(&mut [0u8; 1024]).is_err(), "and nothing was forwarded back out");
}

/// **A datagram claiming an address this cable already carries is
/// dropped.** A frame whose hardware source is this machine's own
/// address is a frame the interface takes for its own --- Transmit Done
/// and all --- so the one station a peer may not be is one that is
/// already here.
#[test]
fn a_datagram_from_this_cables_own_address_is_dropped() {
    let (peer, peer_at) = peer_socket();
    let l = link(vec![(PEER, peer_at)], false);
    let mut e = Ether::new();
    e.keep_log(true);
    let (machine, heard) = host(ME, vec![]);
    e.attach(machine);
    e.attach(Box::new(l.node(&[ME, SERVER], false)));
    for claimed in [ME, SERVER, 0] {
        send_packet(&peer, l.at, &status_rfc(), ME, claimed);
    }
    // And a good one after them, so the test waits on something rather
    // than on nothing happening.
    send_packet(&peer, l.at, &status_rfc(), ME, PEER);
    assert!(run_until(&mut e, |e| !sent(e).is_empty()), "the good one reached the cable");
    let heard = heard.lock().unwrap();
    assert_eq!(heard.len(), 1, "only the one from 3040: {heard:?}");
    assert_eq!(sent(&e), [(sent(&e)[0].0, PEER)]);
}

/// A machine that answers the first frame it hears, which is what shows
/// whether the node knows where to send the answer. It answers only once
/// it has heard, so its frame and the node's are never due together and
/// never collide.
fn answering_host(to: u16) -> (Box<Host>, Log) {
    let p = Packet { opcode: op::CLS, source: ME, dest: to, data: Vec::new(), ..status_rfc() };
    let (mut h, heard) = host(ME, Vec::new());
    h.reply = Some(p.to_buffer(to));
    (h, heard)
}

/// **An endpoint is learned only when the run asked for it.** A packet
/// from a host no `--chaos-udp-peer` named is heard either way --- the
/// cable hears everything --- but the answer to it goes out only when
/// `--chaos-udp-dynamic` said to learn where the host is. Off is the
/// default because otherwise whatever can reach the port installs itself
/// in the address table under whatever Chaosnet address it claims.
#[test]
fn an_endpoint_is_learned_only_when_the_run_asked_for_it() {
    for dynamic in [false, true] {
        let (stranger, _) = peer_socket();
        let l = link(Vec::new(), dynamic);
        let mut e = Ether::new();
        e.keep_log(true);
        let (machine, heard) = answering_host(STRANGER);
        e.attach(machine);
        e.attach(Box::new(l.node(&[ME, SERVER], false)));
        let p = Packet { source: STRANGER, ..status_rfc() };
        send_packet(&stranger, l.at, &p, ME, STRANGER);
        assert!(run_until(&mut e, |e| sent(e).len() >= 2), "both frames went, dynamic {dynamic}");
        assert_eq!(heard.lock().unwrap().len(), 1, "the machine heard it either way");
        let answer = stranger.recv_from(&mut [0u8; 1024]);
        assert_eq!(
            answer.is_ok(),
            dynamic,
            "the answer goes out only when the endpoint was learned: dynamic {dynamic}"
        );
    }
}

/// **A packet does not move an endpoint a flag named.** An endpoint
/// typed on the command line is a statement about where a host is;
/// letting a packet redirect it would put the naming back in the hands
/// of whoever can reach the port, which is what leaving learning off by
/// default is for.
#[test]
fn a_packet_does_not_move_an_endpoint_a_flag_named() {
    let (named, named_at) = peer_socket();
    let (impostor, _) = peer_socket();
    let l = link(vec![(PEER, named_at)], true);
    let mut e = Ether::new();
    e.keep_log(true);
    let (machine, _) = answering_host(PEER);
    e.attach(machine);
    e.attach(Box::new(l.node(&[ME, SERVER], false)));
    // The impostor claims to be 3040, which is where the flag put it.
    let p = Packet { source: PEER, ..status_rfc() };
    send_packet(&impostor, l.at, &p, ME, PEER);
    assert!(run_until(&mut e, |e| sent(e).len() >= 2), "both frames went");
    let mut buf = [0u8; 1024];
    let (n, _) = named.recv_from(&mut buf).expect("the answer went where the flag said");
    let f = udp::unwrap(&buf[..n]).expect("it reads");
    assert_eq!(Packet::from_buffer(&f.buffer).expect("a packet").0.opcode, op::CLS);
    assert!(impostor.recv_from(&mut buf).is_err(), "and not to whoever claimed the address");
}

// --- a running machine --------------------------------------------------

/// **A datagram reaches a running muir's cable.** The whole path, in a
/// process started from the command line: `--chaos-address` starts the
/// link and says which machine this is, `--chaos-udp` says where it
/// listens, `--chaos-udp-peer` says where this test lives, the node goes
/// on the I/O board's cable, and an RFC that arrives as a datagram is put
/// on the modelled cable for the interface to hear.
///
/// **The answer is not muir's to give.** A CADR has no file or time
/// server in it, so a run carries none: nothing on that cable answers an
/// RFC but a booted band, which is minutes of machine time away, and the
/// host a band calls is `ozd` or another program on the network. So what
/// is checked here is the half muir owns --- the datagram taken in and
/// laid on the cable --- read off `--chaos-trace`, which is what the node
/// says of every frame it decides about. The other half, a frame off the
/// cable going out as a datagram, is `a_frame_for_a_peer_goes_out_as_a_datagram`
/// above, in process.
///
/// `--chaos-udp 127.0.0.1:0` lets the host choose the port and the start
/// banner says which, so the test needs no port of its own to be free.
#[test]
fn a_datagram_reaches_a_running_muirs_cable() {
    let (peer, peer_at) = peer_socket();
    let child = support::muir()
        .args(["--micro", "--stop-after", "4000000000"])
        // The address has to be given: the default is subnet 376's and no
        // band's, so a run that wants this machine at [`ME`] says so. It
        // is also what starts the link.
        .args(["--chaos-address", &format!("{ME:o}")])
        .args(["--chaos-udp", "127.0.0.1:0"])
        .args(["--chaos-udp-peer", &format!("{PEER:o}@{peer_at}")])
        .args(["--chaos-trace"])
        .start();
    child.stderr().wait_until(
        |t| t.contains("chaosnet udp: listening at"),
        "muir says where it is listening",
    );
    let banner = child.stderr().so_far();
    let line = banner.lines().find(|l| l.starts_with("chaosnet udp:")).expect("the line");
    let at: SocketAddr = line
        .split_whitespace()
        .nth(4)
        .and_then(|w| w.trim_end_matches(',').parse().ok())
        .unwrap_or_else(|| panic!("an endpoint in {line:?}"));
    // An RFC for STATUS from this test to the machine, resent until the
    // trace shows it landed: the machine is running and UDP does not
    // promise the first one arrives while the node is looking.
    let rfc = Packet { dest: ME, source: PEER, ..status_rfc() };
    let landed = format!("{PEER:o} -> {ME:o} RFC for the cable");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(
            Instant::now() < deadline,
            "the datagram never reached the cable; muir wrote:\n{}",
            child.stderr().so_far()
        );
        send_packet(&peer, at, &rfc, ME, PEER);
        if child.stderr().so_far().contains(&landed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.kill();
}

// --- who may have files -------------------------------------------------

/// A packet as the ether would hand it to a node.
fn arriving(p: &Packet, cable_dest: u16) -> Framed {
    let buffer = p.to_buffer(cable_dest);
    let (source, check) = trailer(&buffer, p.source);
    Framed { buffer, source, check, check_ok: true }
}

/// An RFC for the FILE service from `from`.
fn file_rfc(from: u16) -> Packet {
    Packet {
        opcode: op::RFC,
        forward: 0,
        dest: SERVER,
        dest_index: 0,
        source: from,
        source_index: 0o21,
        number: 1,
        ack: 0,
        data: b"FILE 1".to_vec(),
    }
}

/// What the server answers an RFC with: the opcode of the first packet
/// it sends after it.
fn answer_to(server: &ChaosServer, from: u16) -> u8 {
    let mut h = server.build(0);
    h.receive(100, &arriving(&file_rfc(from), SERVER));
    let b = h.transmit(100).expect("the server answers");
    Packet::from_buffer(&b).expect("a packet").0.opcode
}

/// **Reachability is not authorisation: FILE serves the hosts it was
/// given and refuses the rest.**
///
/// The service reads, writes, renames and deletes a real directory under
/// containment rules written for a cable with one trusted machine on it.
/// A CHUDP peer that a packet arrived from is answerable --- that is what
/// `--chaos-udp-dynamic` decides --- and being answerable is not being
/// allowed at the files. TIME, UPTIME and STATUS answer anyone; they give
/// nothing away.
///
/// Who a *run* of muir lets at its files is no longer a question muir
/// answers: the file host is another program on the network, and the
/// containment is its own. What is here is the harness's server, which is
/// the code that has to keep the rule.
#[test]
fn file_serves_the_hosts_it_was_given() {
    let root = std::env::temp_dir();
    // Named, not defaulted: [`ME`] and [`SERVER`] are what this file's
    // cable carries.
    let server = || ChaosServer::new(SERVER).serving(root.clone()).at_time(time::TEST_UNIVERSAL);
    let named = server().for_hosts(vec![ME, PEER]);
    assert_eq!(answer_to(&named, ME), op::OPN, "the machine is served, as it always was");
    assert_eq!(answer_to(&named, PEER), op::OPN, "and the peer that was named with it");
    assert_eq!(answer_to(&named, STRANGER), op::CLS, "and nobody else");
    // With the machine alone it is the machine and nobody else, which is
    // what every test that boots a band over this server has.
    let alone = server().for_hosts(vec![ME]);
    assert_eq!(answer_to(&alone, ME), op::OPN);
    assert_eq!(answer_to(&alone, PEER), op::CLS);
}
