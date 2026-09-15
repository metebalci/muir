// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Chaosnet over UDP: the frame's bytes, and the node that carries it on
//! and off the modeled cable.
//!
//! The frame is pinned here as bytes rather than left implicit in the
//! packing code, and two datagrams are the whole of the evidence for it:
//! one muir writes for a known packet, and one `cbridge` itself wrote.
//! Both were taken from a running `cbridge` on 15 September 2026, which
//! is how muir knows that every 16-bit word goes most significant byte
//! first and that the trailer's third word is the Internet checksum.

use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod support;

use support::Run;

use muir::chaos::ether::{Ether, Event, Node, ROUND_NS, SLOT_NS};
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
/// A host beyond the bridge: no peer entry names it, so a frame for it
/// goes to the default peer and its packets arrive from there.
const BEYOND: u16 = 0o3042;

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
/// words and its data, and then the trailer's destination, source and
/// checksum. **Every word most significant byte first**, in the header,
/// in the data and in the trailer.
///
/// These 32 bytes are an RFC for `STATUS` from 3040 to 3050 as a running
/// `cbridge` takes it. The data is what shows the order is not arbitrary:
/// AIM-628 §3.6 puts the first byte of a pair in the word's least
/// significant half, and the word then goes out most significant byte
/// first, so `STATUS` appears on the wire as `TSTASU`. Bytes 20 to 25
/// below read `TSTASU`.
///
/// The trailer's third word is the Internet checksum, `udp::checksum`,
/// and **not** the CADR's own CRC-16: no CADR ever sent a UDP datagram,
/// and the check word the machine's own hardware makes is the cable's.
#[test]
fn the_frame_is_these_bytes() {
    let p = status_rfc();
    let buffer = p.to_buffer(ME);
    let frame = udp::wrap(&buffer, PEER).expect("a frame");
    #[rustfmt::skip]
    let want: [u8; 32] = [
        // version 1, function 1, two zero bytes
        0x01, 0x01, 0x00, 0x00,
        0x01, 0x00, // opcode RFC (1) in the high byte
        0x00, 0x06, // forwarding count 0, six data bytes
        0x06, 0x28, // destination 3050
        0x00, 0x00, // destination index
        0x06, 0x20, // source 3040
        0x00, 0x11, // source index 21
        0x00, 0x01, // packet number 1
        0x00, 0x00, // acknowledgement 0
        b'T', b'S', b'T', b'A', b'S', b'U', // "STATUS", pairwise swapped
        0x06, 0x28, // the trailer: destination 3050
        0x06, 0x20, // source 3040
        0xea, 0x6d, // the Internet checksum
    ];
    assert_eq!(frame, want, "the frame as it goes into the datagram");
    assert_eq!(&frame[20..26], b"TSTASU", "the data pairwise swapped, which is network order");
    // The checksum is muir's own, over the words before it: the eight
    // header words, the data words, and the trailer's destination and
    // source.
    let mut over = buffer.clone();
    over.push(PEER);
    assert_eq!(udp::checksum(&over), 0xea6d, "the checksum these words make");
    // And the frame is good when every word, the checksum included, sums
    // to 0xffff --- which is to say the whole lot checksums to zero.
    over.push(0xea6d);
    assert_eq!(udp::checksum(&over), 0, "a good frame's words sum to 0xffff");
    // The CADR's own CRC-16 over the same words is another number
    // altogether, and it is the cable's and not the datagram's.
    assert_eq!(trailer(&buffer, PEER).1, 0o171007, "the hardware's check word for these words");
}

/// **`cbridge`'s own answer reads back, and muir writes it again byte for
/// byte.** 94 bytes captured from a running `cbridge` on 15 September
/// 2026: its answer to a STATUS request, from the bridge at 177020, which
/// named itself `cbtest`, to the host at 177022 that asked. Its name
/// appears on the wire as `bcetts`, pairwise swapped as any data is, and
/// its checksum verifies as the one muir computes.
#[rustfmt::skip]
const CBRIDGE_STATUS: [u8; 94] = [
    0x01, 0x01, 0x00, 0x00, 0x05, 0x00, 0x00, 0x44, 0xfe, 0x12, 0x00, 0x12, 0xfe, 0x10, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x62, 0x63, 0x65, 0x74, 0x74, 0x73, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x01, 0xfe, 0x00, 0x10, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xfe, 0x12, 0xfe, 0x10, 0xc4, 0x05,
];

/// The bridge that sent [`CBRIDGE_STATUS`], and the host it answered.
const CBRIDGE: u16 = 0o177020;
const ASKER: u16 = 0o177022;

#[test]
fn cbridges_own_answer_reads_back() {
    let f = udp::unwrap(&CBRIDGE_STATUS).expect("it reads");
    let (p, cable_dest) = Packet::from_buffer(&f.buffer).expect("a packet");
    assert_eq!(p.opcode, op::ANS, "an ANS, which is what a STATUS request is answered with");
    assert_eq!(p.forward, 0, "no forwarding count");
    assert_eq!(p.data.len(), 0o104, "68 data bytes, the count word's low twelve bits");
    assert_eq!((p.dest, p.dest_index), (ASKER, 0o22), "the host that asked, and its index");
    assert_eq!((p.source, p.source_index), (CBRIDGE, 0), "the bridge answering");
    assert_eq!((p.number, p.ack), (0, 0), "an ANS is uncontrolled: no number, no acknowledgement");
    // AIM-628 §4.4: the answer opens with the host's name in 32 bytes.
    assert_eq!(&p.data[..6], b"cbtest", "the name it called itself");
    assert_eq!(&p.data[6..32], [0u8; 26], "padded to 32 bytes");
    assert_eq!(&CBRIDGE_STATUS[20..26], b"bcetts", "and on the wire it is pairwise swapped");
    // The trailer: the next hop, the sender, and the checksum.
    assert_eq!(cable_dest, ASKER, "the trailer's destination");
    assert_eq!(f.source, CBRIDGE, "the trailer's source");
    let mut over = f.buffer.clone();
    over.push(f.source);
    assert_eq!(udp::checksum(&over), 0xc405, "the checksum it carries, which muir makes too");
    // What goes on the modeled cable carries the CADR's own check word,
    // made here for it, because that is what the interface checks.
    assert!(f.check_ok, "and it is good");
    assert_eq!(f.check, trailer(&f.buffer, f.source).1, "the hardware's, made at this edge");
    // And back out again, the same bytes.
    assert_eq!(udp::wrap(&f.buffer, f.source).expect("a frame"), CBRIDGE_STATUS);
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
        let frame = udp::wrap(&buffer, source).expect("a frame");
        let back = udp::unwrap(&frame).expect("it reads back");
        assert_eq!(back, Framed { buffer, source, check, check_ok: true }, "{p:?}");
        assert_eq!(Packet::from_buffer(&back.buffer).expect("a packet").0, p);
    }
}

/// **An odd data count is padded to a whole word, and the pad byte comes
/// first.** The data is a whole number of 16-bit words on the cable, and
/// the odd byte sits in the word's least significant half, AIM-628 §3.6,
/// so the zero that pads it is the high half --- which is the byte that
/// goes out first. `cbridge` always pads.
///
/// **A frame that is not a whole number of words is refused**, because
/// the checksum is over words: there is no reading of a lone trailing
/// byte that can be summed, so such a frame cannot be checked and is not
/// taken on faith.
#[test]
fn an_odd_data_count_is_padded_to_a_word() {
    let p = Packet { data: b"odd".to_vec(), ..status_rfc() };
    let buffer = p.to_buffer(ME);
    let padded = udp::wrap(&buffer, PEER).expect("a frame");
    assert_eq!(padded.len(), 4 + 16 + 4 + 6, "the data padded to a word");
    assert_eq!(&padded[4 + 16..4 + 16 + 4], b"do\0d", "the pad byte first in its word");
    let back = udp::unwrap(&padded).expect("padded reads");
    assert_eq!(back.buffer, buffer, "and to the words that were written");
    assert_eq!(Packet::from_buffer(&back.buffer).expect("a packet").0.data, b"odd");
    // The same frame with the pad byte taken out.
    let mut unpadded = padded.clone();
    unpadded.remove(4 + 16 + 2);
    assert_eq!(unpadded.len(), 4 + 16 + 3 + 6);
    udp::unwrap(&unpadded).expect_err("a body that is not whole words");
}

/// **A version this does not speak is refused, and so is a function.**
/// What a version 2 would lay out differently is not known here --- the
/// byte order of version 1 was itself only settled by watching `cbridge`
/// --- so a version 2 peer read as a version 1 one would exchange
/// nonsense. It is refused by number instead.
#[test]
fn an_unknown_version_is_refused() {
    let p = status_rfc();
    let buffer = p.to_buffer(ME);
    let good = udp::wrap(&buffer, PEER).expect("a frame");
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
    let good = udp::wrap(&buffer, PEER).expect("a frame");
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

/// The frame muir wrote before it spoke `cbridge`'s framing: the packet's
/// words least significant byte first, the trailer's most significant
/// first, and the CADR's own CRC-16 in the trailer's third word.
fn old_framing(buffer: &[u16], source: u16) -> Vec<u8> {
    let (&dest, words) = buffer.split_last().expect("a destination");
    let mut out = vec![1u8, 1, 0, 0];
    for &w in words {
        out.extend(w.to_le_bytes());
    }
    for w in [dest, source, trailer(buffer, source).1] {
        out.extend(w.to_be_bytes());
    }
    out
}

/// **A frame whose checksum is wrong is refused, and the refusal names
/// it** --- which is the line `--chaos-trace` prints and what
/// `cbridge` calls "Bad checksum".
///
/// The frame here is the one muir used to send in network order: the
/// trailer's third word holding the CADR's own CRC-16 rather than the
/// Internet checksum. That is exactly the frame a running `cbridge`
/// refused, printing the one's complement sum over the frame as sent.
#[test]
fn a_frame_with_a_wrong_checksum_is_refused() {
    let buffer = status_rfc().to_buffer(ME);
    let good = udp::wrap(&buffer, PEER).expect("a frame");
    assert!(udp::unwrap(&good).is_ok(), "the Internet checksum is what it wants");
    let mut crc = good.clone();
    let n = crc.len();
    crc[n - 2..].copy_from_slice(&trailer(&buffer, PEER).1.to_be_bytes());
    let err = udp::unwrap(&crc).expect_err("the CADR's CRC-16 is not the checksum");
    assert!(err.contains("checksum"), "the refusal names it: {err}");
    // And one bit anywhere in the frame is enough.
    for k in [4, 12, 20, good.len() - 3, good.len() - 1] {
        let mut bad = good.clone();
        bad[k] ^= 1;
        let err = udp::unwrap(&bad).expect_err("one bit changed is enough");
        assert!(err.contains("checksum"), "byte {k}: {err}");
    }
}

/// **A frame in the old byte order is refused as not a packet.** Its
/// count word comes out of the other half and no longer accounts for the
/// datagram, so muir can say that much; it cannot say what `cbridge` says
/// of such a frame, which is "bogus", because what it can see is a count
/// and a length that do not agree. The checksum would refuse it as well,
/// the words being byte-swapped, but the shape is what is looked at
/// first.
#[test]
fn a_frame_in_the_old_byte_order_is_refused() {
    let buffer = status_rfc().to_buffer(ME);
    let old = old_framing(&buffer, PEER);
    assert_eq!(old.len(), udp::wrap(&buffer, PEER).expect("a frame").len(), "the same length");
    let err = udp::unwrap(&old).expect_err("the old framing is not this one");
    assert!(err.contains("count"), "and the refusal says what muir can see: {err}");
}

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
/// peers and this default peer.
fn link(peers: Vec<(u16, SocketAddr)>, default_peer: Option<SocketAddr>) -> Link {
    Link::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), peers, default_peer).expect("a link")
}

/// One datagram from `from` to `to`, carrying `p` addressed on the cable
/// to `cable_dest` and sourced there by `hardware_source`.
fn send_packet(from: &UdpSocket, to: SocketAddr, p: &Packet, cable_dest: u16, hardware: u16) {
    let buffer = p.to_buffer(cable_dest);
    let frame = udp::wrap(&buffer, hardware).expect("a frame");
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
    let l = link(vec![(PEER, peer_at)], None);
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
    let l = link(vec![(PEER, peer_at)], None);
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
    assert!(f.check_ok, "and a check word made for the cable at this edge");
    assert_eq!(f.check, trailer(&f.buffer, ME).1, "which is the CADR's own CRC-16");
    assert_eq!(Packet::from_buffer(&f.buffer).expect("a packet"), (p, PEER));
}

/// **A frame for an address no peer entry names goes to the default
/// peer, and with no default peer it is dropped.** `--chaos-udp-peer`
/// says that an address lives at an endpoint, so a frame for any other
/// address has nowhere to go; `--chaos-udp-default-peer` is the route
/// of last resort, which is what lets a `cbridge` beside muir carry the
/// traffic on. The frame carries the real destination in its trailer and
/// the bridge routes on that, which is why the flag takes an endpoint
/// and no Chaosnet address.
#[test]
fn a_frame_for_an_address_no_entry_names_goes_to_the_default_peer() {
    for default_peer in [false, true] {
        let (bridge, bridge_at) = peer_socket();
        let l = link(Vec::new(), default_peer.then_some(bridge_at));
        let mut e = Ether::new();
        e.keep_log(true);
        let p = Packet { source: ME, dest: BEYOND, ..status_rfc() };
        let (machine, _) = host(ME, vec![p.to_buffer(BEYOND)]);
        e.attach(machine);
        e.attach(Box::new(l.node(&[ME, SERVER], false)));
        assert!(run_until(&mut e, |e| !sent(e).is_empty()), "the machine's frame went");
        let mut buf = [0u8; 1024];
        let got = bridge.recv_from(&mut buf);
        assert_eq!(
            got.is_ok(),
            default_peer,
            "the frame goes out only when there is a default peer: {default_peer}"
        );
        if let Ok((n, _)) = got {
            let f = udp::unwrap(&buf[..n]).expect("it reads");
            assert_eq!(f.source, ME, "the hardware source the cable carried");
            assert_eq!(
                Packet::from_buffer(&f.buffer).expect("a packet"),
                (p, BEYOND),
                "and the destination is in the frame, for the bridge to route on"
            );
        }
    }
}

/// **A frame for an address a peer entry names goes to that peer and
/// not to the default.** The default peer is where what is not named
/// goes, not a route that stands in front of the names: an endpoint a
/// flag gave is where that address is reached, bridge or no bridge.
#[test]
fn a_frame_for_a_named_peer_does_not_go_to_the_default() {
    let (peer, peer_at) = peer_socket();
    let (bridge, bridge_at) = peer_socket();
    let l = link(vec![(PEER, peer_at)], Some(bridge_at));
    let mut e = Ether::new();
    e.keep_log(true);
    let p = Packet { source: ME, dest: PEER, ..status_rfc() };
    let (machine, _) = host(ME, vec![p.to_buffer(PEER)]);
    e.attach(machine);
    e.attach(Box::new(l.node(&[ME, SERVER], false)));
    assert!(run_until(&mut e, |e| !sent(e).is_empty()), "the machine's frame went");
    let mut buf = [0u8; 1024];
    let (n, _) = peer.recv_from(&mut buf).expect("the datagram goes where the flag said");
    let f = udp::unwrap(&buf[..n]).expect("it reads");
    assert_eq!(Packet::from_buffer(&f.buffer).expect("a packet"), (p, PEER));
    assert!(bridge.recv_from(&mut buf).is_err(), "and not to the default peer as well");
}

/// **A broadcast goes to every named peer and not to the default
/// peer.** The peers are stations on this machine's cable, so a
/// broadcast is as much theirs as the board's; the default peer is the
/// way out to a wider network, and handing it a broadcast would put
/// this cable's on a network the broadcast was never meant to reach.
/// Decided, and not an oversight.
#[test]
fn a_broadcast_goes_to_the_named_peers_and_not_the_default() {
    let (one, one_at) = peer_socket();
    let (two, two_at) = peer_socket();
    let (bridge, bridge_at) = peer_socket();
    let l = link(vec![(PEER, one_at), (STRANGER, two_at)], Some(bridge_at));
    let mut e = Ether::new();
    e.keep_log(true);
    // A broadcast is destination 0 on the cable, AIM-628 §2.2.
    let p = Packet { source: ME, dest: 0, ..status_rfc() };
    let (machine, _) = host(ME, vec![p.to_buffer(0)]);
    e.attach(machine);
    e.attach(Box::new(l.node(&[ME, SERVER], false)));
    assert!(run_until(&mut e, |e| !sent(e).is_empty()), "the machine's frame went");
    let mut buf = [0u8; 1024];
    for (who, station) in [(PEER, &one), (STRANGER, &two)] {
        let (n, _) =
            station.recv_from(&mut buf).unwrap_or_else(|e| panic!("{who:o} hears it: {e}"));
        let f = udp::unwrap(&buf[..n]).expect("it reads");
        assert_eq!(Packet::from_buffer(&f.buffer).expect("a packet"), (p.clone(), 0), "{who:o}");
    }
    assert!(bridge.recv_from(&mut buf).is_err(), "and the bridge is not given a broadcast");
}

/// **muir is a leaf: a packet for a third party is dropped, not
/// forwarded.** AIM-628 chapter 6's routing is a bridge's job; a
/// `cbridge` beside muir does it. Nothing addressed to 3041 reaches this
/// cable, and nothing goes back out.
#[test]
fn a_packet_for_a_third_party_is_dropped() {
    let (peer, peer_at) = peer_socket();
    let l = link(vec![(PEER, peer_at), (STRANGER, peer_at)], None);
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

/// **muir is a leaf with a default peer too: what one peer put on the
/// cable is never sent to another.** A frame goes out over UDP only
/// when a station of this process put it on the cable, so a frame the
/// node itself laid there --- for a named peer, or relayed by the
/// default peer from an address no entry names --- stops at the cable.
/// The default peer widens where a frame may go, not which frames go:
/// routing is `cbridge`'s job, AIM-628 chapter 6's.
#[test]
fn what_a_peer_put_on_the_cable_is_never_sent_to_another() {
    let (peer, peer_at) = peer_socket();
    let (other, other_at) = peer_socket();
    let (bridge, bridge_at) = peer_socket();
    let l = link(vec![(PEER, peer_at), (STRANGER, other_at)], Some(bridge_at));
    let mut node = l.node(&[ME, SERVER], false);
    // One from a named peer and one from an address no entry names ---
    // which is what the default peer relays --- each addressed to the
    // other named peer, which is the frame a router would carry on.
    for from in [PEER, BEYOND] {
        let p = Packet { source: from, dest: STRANGER, ..status_rfc() };
        node.receive(100, &arriving(&p, STRANGER));
    }
    // And one this machine put there after them, so the test waits on
    // something rather than on nothing happening.
    let mine = Packet { source: ME, dest: STRANGER, ..status_rfc() };
    node.receive(100, &arriving(&mine, STRANGER));
    let mut buf = [0u8; 1024];
    let (n, _) = other.recv_from(&mut buf).expect("the machine's own frame went");
    let f = udp::unwrap(&buf[..n]).expect("it reads");
    assert_eq!(f.source, ME, "and it is the one the machine put on the cable");
    assert!(other.recv_from(&mut buf).is_err(), "the peers' frames were not carried on");
    assert!(peer.recv_from(&mut buf).is_err(), "nor sent back where they came from");
    assert!(bridge.recv_from(&mut buf).is_err(), "nor handed to the default peer");
}

/// **A datagram claiming an address this cable already carries is
/// dropped.** A frame whose hardware source is this machine's own
/// address is a frame the interface takes for its own --- Transmit Done
/// and all --- so the one station a peer may not be is one that is
/// already here.
#[test]
fn a_datagram_from_this_cables_own_address_is_dropped() {
    let (peer, peer_at) = peer_socket();
    let l = link(vec![(PEER, peer_at)], None);
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

/// **A frame whose checksum is wrong never reaches the cable.** The node
/// drops it where it drops any datagram it cannot read, and goes on: the
/// good frame behind it is heard.
#[test]
fn a_frame_with_a_wrong_checksum_never_reaches_the_cable() {
    let (peer, peer_at) = peer_socket();
    let l = link(vec![(PEER, peer_at)], None);
    let mut e = Ether::new();
    e.keep_log(true);
    let (machine, heard) = host(ME, vec![]);
    e.attach(machine);
    e.attach(Box::new(l.node(&[ME, SERVER], false)));
    let buffer = status_rfc().to_buffer(ME);
    let mut bad = udp::wrap(&buffer, PEER).expect("a frame");
    let n = bad.len();
    bad[n - 2..].copy_from_slice(&trailer(&buffer, PEER).1.to_be_bytes());
    peer.send_to(&bad, l.at).expect("it goes");
    // And a good one after it, so the test waits on something rather than
    // on nothing happening.
    send_packet(&peer, l.at, &status_rfc(), ME, PEER);
    assert!(run_until(&mut e, |e| !sent(e).is_empty()), "the good one reached the cable");
    let heard = heard.lock().unwrap();
    assert_eq!(heard.len(), 1, "one frame, not two: {heard:?}");
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

/// **An answer goes out only when the node knows where to send it.** A
/// packet from a host no `--chaos-udp-peer` named is heard either way
/// --- the cable hears everything --- but the answer to it goes out
/// only when there is somewhere to send it, which is that host's own
/// peer entry or `--chaos-udp-default-peer`; with neither it is
/// dropped.
///
/// **The endpoint the packet came from is not somewhere to send it.**
/// Nothing here learns where a host lives: an address table nobody
/// wrote down is state a run cannot be read back from, and it would put
/// the naming in the hands of whoever can reach the port.
#[test]
fn an_answer_goes_out_only_when_the_node_knows_where_to_send_it() {
    for default_peer in [false, true] {
        let (beyond, _) = peer_socket();
        let (bridge, bridge_at) = peer_socket();
        let l = link(Vec::new(), default_peer.then_some(bridge_at));
        let mut e = Ether::new();
        e.keep_log(true);
        let (machine, heard) = answering_host(BEYOND);
        e.attach(machine);
        e.attach(Box::new(l.node(&[ME, SERVER], false)));
        let p = Packet { source: BEYOND, ..status_rfc() };
        send_packet(&beyond, l.at, &p, ME, BEYOND);
        assert!(
            run_until(&mut e, |e| sent(e).len() >= 2),
            "both frames went, default peer {default_peer}"
        );
        assert_eq!(heard.lock().unwrap().len(), 1, "the machine heard it either way");
        assert_eq!(
            bridge.recv_from(&mut [0u8; 1024]).is_ok(),
            default_peer,
            "the answer goes out only when there is somewhere to send it: {default_peer}"
        );
        assert!(
            beyond.recv_from(&mut [0u8; 1024]).is_err(),
            "and never back where the packet came from: nothing is learned"
        );
    }
}

/// **A packet does not move an endpoint a flag named.** An endpoint
/// typed on the command line is a statement about where a host is;
/// letting a packet redirect it would put the naming back in the hands
/// of whoever can reach the port. Nothing moves an entry because
/// nothing writes one: the table is what the flags said and nothing
/// else.
#[test]
fn a_packet_does_not_move_an_endpoint_a_flag_named() {
    let (named, named_at) = peer_socket();
    let (impostor, _) = peer_socket();
    let l = link(vec![(PEER, named_at)], None);
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
/// on the modeled cable for the interface to hear.
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
    child
        .stderr()
        .wait_until(|t| t.contains("chaosnet over udp: 127."), "muir says where it is listening");
    let banner = child.stderr().so_far();
    let line = banner.lines().find(|l| l.starts_with("chaosnet over udp:")).expect("the line");
    let at: SocketAddr = line
        .split_whitespace()
        .nth(3)
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
    // **And a frame whose checksum is wrong is dropped, and said.** The
    // trailer's third word here is the CADR's own CRC-16, which is what
    // muir sent before it spoke `cbridge`'s framing and what `cbridge`
    // refuses with "Bad checksum".
    let buffer = rfc.to_buffer(ME);
    let mut bad = udp::wrap(&buffer, PEER).expect("a frame");
    let n = bad.len();
    bad[n - 2..].copy_from_slice(&trailer(&buffer, PEER).1.to_be_bytes());
    let said = format!("from {peer_at}: a checksum");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(
            Instant::now() < deadline,
            "the trace never said the checksum was wrong; muir wrote:\n{}",
            child.stderr().so_far()
        );
        peer.send_to(&bad, at).expect("it goes");
        if child.stderr().so_far().contains(&said) {
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
/// A host at the other end of the cable is answerable --- a peer a flag
/// named, or anything the default peer carries --- and being answerable
/// is not being allowed at the files. TIME, UPTIME and STATUS answer
/// anyone; they give nothing away.
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

/// **A frame a busy receiver aborts goes again, and is given up on after
/// three tries.** The node is a station on the cable, and what stands in
/// for its driver is the CADR's: `uc-chaos.lisp` loads
/// `A-CHAOS-TRANSMIT-RETRY-COUNT` with `CHAOS-NUMBER-TRANSMIT-RETRIES`,
/// which is 3 --- "Send once and retry twice if aborted" --- counts it
/// down on every Transmit Abort, and at zero is done with the packet and
/// leaves it to the transport. So a peer's packet, handed to a machine
/// whose buffer never empties, goes on the cable three times and no more,
/// each try a whole round of the turn timer after the last: the abort,
/// the host writing the buffer again --- which is what the driver does,
/// halfword by halfword through `CHAOS-XMT-2`, before reading `START`
/// --- and the first turn that comes round after that.
#[test]
fn a_frame_a_busy_receiver_aborts_is_retried_and_then_given_up_on() {
    let (peer, peer_at) = peer_socket();
    let l = link(vec![(PEER, peer_at)], None);
    let mut e = Ether::new();
    e.keep_log(true);
    // A machine whose receive buffer is full and stays full.
    e.attach_board(ME);
    e.board_receiver(true, false);
    e.attach(Box::new(l.node(&[ME, SERVER], false)));
    send_packet(&peer, l.at, &status_rfc(), ME, PEER);
    let tries = udp::TRANSMIT_TRIES as usize;
    assert!(
        run_until(&mut e, |e| sent(e).len() >= tries),
        "the frame went {tries} times: {:?}",
        sent(&e)
    );
    // And no more, however long the cable stays free.
    let after = sent(&e).last().unwrap().0 + 10 * ROUND_NS;
    run(&mut e, after - 10 * ROUND_NS, after);
    let sent = sent(&e);
    let mut lost = Vec::new();
    while let Some(x) = e.board_lost() {
        lost.push(x);
    }
    eprintln!("the frame on the cable at {sent:?}; the receiver counted {lost:?}");
    assert_eq!(sent.len(), tries, "three tries and no fourth: {sent:?}");
    assert!(sent.iter().all(|&(_, from)| from == PEER), "all of them the peer's: {sent:?}");
    assert_eq!(lost.len(), tries, "each counted in Lost Count: {lost:?}");
    assert!(lost.iter().all(|&(_, from, aborted)| from == PEER && aborted), "each aborted");
    for w in sent.windows(2) {
        let gap = w[1].0 - w[0].0;
        eprintln!("a try {gap} ns after the last, a round being {ROUND_NS} ns");
        assert!(
            (ROUND_NS..2 * ROUND_NS).contains(&gap),
            "a try comes a whole round after the last, not at once: {gap} ns"
        );
    }
}
