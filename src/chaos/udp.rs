// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Chaosnet over UDP: the stations on this machine's cable that are not
//! in this process.
//!
//! CHUDP puts one Chaosnet packet in one UDP datagram behind a four-byte
//! header, and it is what `cbridge`, `usim`, `klh10` and the live
//! Chaosnet hosts speak to each other. Ordinary UDP: no privileges, and
//! it crosses a NAT --- which IP protocol 16, the assigned number for
//! Chaosnet, does not.
//!
//! **muir is a leaf, not a router.** A datagram whose hardware
//! destination is neither an address on this machine's cable nor the
//! broadcast address is dropped rather than forwarded; AIM-628 chapter
//! 6's routing is a bridge's job and `cbridge` is beside muir to do it.
//!
//! **The link is a [`super::ether::Node`] and nothing else.** It waits
//! its turn on the modelled cable as any other station does, so the board
//! sees a station taking its turn rather than a transceiver that has
//! failed. What comes in over UDP goes on the cable at the node's turn;
//! what the board puts on the cable for a peer goes out as a datagram.
//!
//! ## The frame
//!
//! ```text
//! offset  width  field
//!      0      1  version
//!      1      1  function
//!      2      2  two argument bytes
//! ---- the Chaos packet, AIM-628 §3.5, in 16-bit words ----
//!      4      2  operation
//!      6      2  count: 4-bit forwarding count, 12-bit data count
//!      8      2  destination address
//!     10      2  destination index
//!     12      2  source address
//!     14      2  source index
//!     16      2  packet number
//!     18      2  acknowledge
//!     20      n  data, n the byte count rounded up to a whole word
//! ---- the hardware trailer, AIM-628 §2.2 ----
//! 20 + n      2  destination
//! 22 + n      2  source
//! 24 + n      2  check
//! ```
//!
//! An odd byte count is padded to a whole word here, the cable carrying
//! whole words; [`unwrap`] finds the trailer from the end of the
//! datagram rather than from the count, so a peer that does not pad is
//! read all the same. Which a peer does is **unverified**.
//!
//! `version = 1` and `function = 1`, for "here is a Chaos packet", which
//! is described as the only function defined. The default port is 42042.
//! Read from the Wireshark dissector published at
//! `gist.github.com/ams/6bde1da514479e27c9f70c161b5537c1` and the
//! protocol page at `chaosnet.net/protocol`, cross-read against the
//! Computer History Wiki's Chaosnet page. **Not** from
//! `bictorv/chaosnet-bridge`, the reference implementation, whose author
//! forbids language models to read or process it; that is the author's
//! decision about the author's own work and it is kept here.
//!
//! Most of the frame was modelled already: [`super::packet::Packet`] is
//! the eight header words and the data, and [`super::packet::frame`]
//! adds the source and the check word to a buffer whose last word is the
//! cable destination --- which is the hardware trailer, in the trailer's
//! own order. The new work is the four-byte wrapper and the socket.
//!
//! ## Byte order, which is **unverified**
//!
//! [`PACKET_ORDER`] and [`TRAILER_ORDER`] say it, and are the only place
//! a word becomes bytes; the protocol's author has said a version 2 may
//! differ from version 1 in nothing but byte order, so that should be
//! two constants to change rather than an audit of the packing.
//! [`VERSION`] is checked on receipt for the same reason: an unknown
//! version is refused with its number rather than parsed as this one.

use super::ether::Node;
use super::packet::{Framed, MAX_DATA, Packet, check_word};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;

/// The port CHUDP is spoken on unless a flag names another.
pub const PORT: u16 = 42042;

/// The version this speaks, and the only one it takes: a datagram
/// carrying any other is refused rather than read as this one.
pub const VERSION: u8 = 1;

/// The function code for "here is a Chaos packet", described as the only
/// one defined.
pub const PACKET: u8 = 1;

/// The CHUDP header: version, function, and two argument bytes, which
/// this sends as zero and does not read.
pub const HEADER: usize = 4;

/// The hardware trailer, AIM-628 §2.2: destination, source, check.
pub const TRAILER: usize = 6;

/// The software header, AIM-628 §3.5: eight 16-bit words.
const SOFTWARE_HEADER: usize = 16;

/// The most a CHUDP frame can be: the header, the software header,
/// [`MAX_DATA`] bytes of data, and the trailer. A datagram longer than
/// this cannot be a Chaos packet, so the receive buffer is one byte
/// more --- a longer datagram then fills it, and is refused for its
/// length rather than read as a truncated packet.
pub const MAX_FRAME: usize = HEADER + SOFTWARE_HEADER + MAX_DATA + TRAILER;

/// How many datagrams are taken from the socket at one turn on the
/// cable. The cable carries one frame at a time, so the rest wait in
/// [`Chudp`]'s queue; this only bounds how long one call spends in the
/// kernel when a peer is flooding.
const DRAIN: usize = 64;

/// Which end of a 16-bit word goes first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// Least significant byte first.
    Little,
    /// Most significant byte first, which is network order.
    Big,
}

impl Order {
    /// `w` appended to `out` in this order.
    fn put(self, w: u16, out: &mut Vec<u8>) {
        out.extend(match self {
            Order::Little => w.to_le_bytes(),
            Order::Big => w.to_be_bytes(),
        });
    }

    /// The word the first two bytes of `b` make in this order.
    fn word(self, b: &[u8]) -> u16 {
        let pair = [b[0], b[1]];
        match self {
            Order::Little => u16::from_le_bytes(pair),
            Order::Big => u16::from_be_bytes(pair),
        }
    }

    /// The word a lone trailing byte makes: it is the half that comes
    /// first, and the other half is not there.
    fn odd(self, b: u8) -> u16 {
        match self {
            Order::Little => b as u16,
            Order::Big => (b as u16) << 8,
        }
    }
}

/// How the Chaos packet's own 16-bit words are laid out: least
/// significant byte first.
///
/// **Unverified.** The protocol's own documentation says so in as many
/// words, and says of it "I'm really sorry about this, and might develop
/// version 2 of the protocol with the only change being big-endian byte
/// order"; the Wireshark dissector reads its 16-bit fields in a way that
/// appears big-endian, and the two are reconciled by [`TRAILER_ORDER`]
/// below rather than by either being wrong. What it also fits: AIM-628
/// §3.6 puts "the first 8-bit byte in a 16-bit word ... in the
/// arithmetically least-significant position", so the data bytes of a
/// packet come out of a little-endian frame in the order they were
/// written and out of a big-endian one swapped in pairs.
///
/// What would settle it: a capture of a live exchange, or one
/// interoperation. The wrong order fails loudly on the first packet ---
/// an absurd 12-bit data count against the datagram's length, and
/// addresses that match nothing configured --- so it does not fail
/// quietly.
pub const PACKET_ORDER: Order = Order::Little;

/// How the hardware trailer's three words are laid out: network order.
///
/// **Unverified.** The reading is that CHUDP is a mixed frame: the
/// reference implementation was read by a person as taking the trailer
/// through `ntohs` --- `srctrailer = ntohs(tr->ch_hw_srcaddr)` --- and
/// not the packet's own words, which is what makes the mixture the
/// likely reading rather than a guess. How the packet's bytes are
/// assembled there was not traced, so this is belief and not knowledge.
///
/// What would settle it: the same capture or interoperation. One whole
/// packet's bytes are pinned in `tests/chudp.rs`, so a correction is a
/// change to these two constants and to that one test.
pub const TRAILER_ORDER: Order = Order::Big;

/// A frame as it stood on the modelled cable, as a CHUDP datagram.
///
/// `buffer` is the buffer the software wrote, its last word the cable
/// destination, and `source` and `check` are the two words the hardware
/// added --- which is exactly [`Framed`]. `None` if there is not even a
/// destination word to put in the trailer.
pub fn wrap(buffer: &[u16], source: u16, check: u16) -> Option<Vec<u8>> {
    let (&dest, words) = buffer.split_last()?;
    let mut out = Vec::with_capacity(HEADER + buffer.len() * 2 + TRAILER);
    out.extend([VERSION, PACKET, 0, 0]);
    for &w in words {
        PACKET_ORDER.put(w, &mut out);
    }
    for w in [dest, source, check] {
        TRAILER_ORDER.put(w, &mut out);
    }
    Some(out)
}

/// The frame back out of a datagram, as the cable would have handed it
/// to a node: the buffer with the cable destination last, and the source
/// and check word the far side's hardware added.
///
/// `check_ok` is that check word against the one the CADR's own
/// hardware would have made for these words, [`check_word`]. **Nothing
/// is dropped on it**: what a CHUDP peer puts in the trailer's third
/// word is unverified --- the hardware trailer's is the 9401's CRC-16,
/// and the trailer has also been described as carrying an Internet
/// checksum --- so this reports the answer and leaves the packet alone.
/// A run with `--chaos-trace` against a real peer settles it, and until
/// then the frame that goes on the modelled cable carries a check word
/// the model computes itself.
///
/// The data is a whole number of 16-bit words on the cable, and this
/// takes both a peer that pads an odd byte count to a word and one that
/// does not: the trailer is found from the end of the datagram, so where
/// it starts is not a guess, and the length is then held to one of the
/// two.
pub fn unwrap(datagram: &[u8]) -> Result<Framed, String> {
    if datagram.len() > MAX_FRAME {
        return Err(format!("{} bytes is longer than any Chaos packet", datagram.len()));
    }
    if datagram.len() < HEADER + SOFTWARE_HEADER + TRAILER {
        return Err(format!("{} bytes is too short for a packet", datagram.len()));
    }
    let version = datagram[0];
    if version != VERSION {
        return Err(format!("version {version}, and this speaks {VERSION}"));
    }
    let function = datagram[1];
    if function != PACKET {
        return Err(format!("function {function}, and only {PACKET} carries a packet"));
    }
    let body = &datagram[HEADER..datagram.len() - TRAILER];
    let count = (PACKET_ORDER.word(&body[2..]) & 0o7777) as usize;
    if count > MAX_DATA {
        return Err(format!("a data count of {count}, and the most is {MAX_DATA}"));
    }
    if body.len() != SOFTWARE_HEADER + count.next_multiple_of(2)
        && body.len() != SOFTWARE_HEADER + count
    {
        return Err(format!("{} bytes of packet against a data count of {count}", body.len()));
    }
    let mut buffer: Vec<u16> = body
        .chunks(2)
        .map(|c| match c {
            [_, _] => PACKET_ORDER.word(c),
            _ => PACKET_ORDER.odd(c[0]),
        })
        .collect();
    let trailer = &datagram[datagram.len() - TRAILER..];
    let dest = TRAILER_ORDER.word(&trailer[0..]);
    let source = TRAILER_ORDER.word(&trailer[2..]);
    let check = TRAILER_ORDER.word(&trailer[4..]);
    buffer.push(dest);
    let mut over = buffer.clone();
    over.push(source);
    let check_ok = check_word(&over) == check;
    Ok(Framed { buffer, source, check, check_ok })
}

/// A CHUDP link: the socket, and who is on the other end of it.
///
/// Bound before a machine is built, so that a port that cannot be had is
/// a refusal at the start rather than a machine that quietly reaches
/// nobody. [`Link::node`] then makes the node that goes on a cable; one
/// link belongs to one machine's cable, and the lashup's second machine
/// has none.
#[derive(Clone, Debug)]
pub struct Link {
    /// Where the socket is bound, as the host bound it: a port of 0 is
    /// the port it chose.
    pub at: SocketAddr,
    /// The peers a flag named: a Chaos address and where it lives.
    pub peers: Vec<(u16, SocketAddr)>,
    /// Whether an endpoint is learned from packets that arrive from an
    /// address no flag named.
    pub dynamic: bool,
    socket: Arc<UdpSocket>,
}

impl Link {
    /// Binds the socket at `at` and holds the peers named for it.
    pub fn bind(
        at: SocketAddr,
        peers: Vec<(u16, SocketAddr)>,
        dynamic: bool,
    ) -> std::io::Result<Link> {
        let socket = UdpSocket::bind(at)?;
        socket.set_nonblocking(true)?;
        let at = socket.local_addr().unwrap_or(at);
        Ok(Link { at, peers, dynamic, socket: Arc::new(socket) })
    }

    /// The node this link puts on a cable. `local` are the addresses
    /// already on that cable --- this machine's, and whatever else this
    /// process put there --- which are never learned, never sent out, and
    /// never spoken for.
    pub fn node(&self, local: &[u16], trace: bool) -> Chudp {
        Chudp {
            socket: self.socket.clone(),
            peers: self.peers.iter().copied().collect(),
            named: self.peers.iter().map(|&(a, _)| a).collect(),
            dynamic: self.dynamic,
            local: local.to_vec(),
            out: VecDeque::new(),
            source: 0,
            trace,
        }
    }
}

/// The peers of this cable that are reached over UDP, as one node on it.
///
/// **Its address on the cable is the address of the peer whose frame it
/// is carrying.** A CHUDP peer is a station on this machine's cable ---
/// that is what a leaf's link is --- so a packet from 3040 goes onto the
/// modelled cable with 3040 in the hardware source, exactly as 3040's
/// own interface would have put it there, and the board's turn timer
/// loads from it as it would from any other station. Between frames it
/// is 0, which is no station's address: the ether uses it only to keep a
/// node from hearing its own frame.
pub struct Chudp {
    socket: Arc<UdpSocket>,
    /// Where each peer lives: the ones a flag named, and the ones
    /// learned.
    peers: BTreeMap<u16, SocketAddr>,
    /// The ones a flag named. **A packet does not move these**: an
    /// endpoint typed on the command line is a statement, and letting a
    /// packet redirect it would put the naming back in the hands of
    /// whoever can reach the port, which is what `--chaos-udp-dynamic`
    /// is off by default to avoid.
    named: BTreeSet<u16>,
    dynamic: bool,
    /// The addresses that are on this cable in this process.
    local: Vec<u16>,
    /// Frames waiting for a turn on the cable: whose each is, and the
    /// buffer.
    out: VecDeque<(u16, Vec<u16>)>,
    /// Whose frame the node is putting on the cable, which is its
    /// address there.
    source: u16,
    trace: bool,
}

impl Chudp {
    /// Everything the socket has, up to [`DRAIN`] datagrams.
    fn drain(&mut self, now: u64) {
        let mut buf = [0u8; MAX_FRAME + 1];
        for _ in 0..DRAIN {
            let (n, from) = match self.socket.recv_from(&mut buf) {
                Ok(v) => v,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(e) => {
                    if self.trace {
                        eprintln!("chudp {now:>6}: the socket: {e}");
                    }
                    return;
                }
            };
            match unwrap(&buf[..n]) {
                Ok(f) => self.arrived(now, from, f),
                Err(why) if self.trace => eprintln!("chudp {now:>6}: from {from}: {why}"),
                Err(_) => {}
            }
        }
    }

    /// One datagram that parsed: what it says about where its sender
    /// lives, and whether it is for this cable at all.
    fn arrived(&mut self, now: u64, from: SocketAddr, f: Framed) {
        let Ok((p, dest)) = Packet::from_buffer(&f.buffer) else {
            if self.trace {
                eprintln!("chudp {now:>6}: from {from}: {} words are not a packet", f.buffer.len());
            }
            return;
        };
        // Reachability, which is not authorisation: this says where a
        // peer can be reached and nothing about what it may ask for.
        // The address learned is the packet's own source, since that is
        // where an answer would be addressed.
        if self.dynamic
            && p.source != 0
            && !self.local.contains(&p.source)
            && !self.named.contains(&p.source)
        {
            let moved = self.peers.insert(p.source, from) != Some(from);
            if moved && self.trace {
                eprintln!("chudp {now:>6}: {:o} is at {from}", p.source);
            }
        }
        // A station on this cable is in this process, so a datagram
        // claiming to be from one would put a frame on the cable that
        // the board takes for its own --- Transmit Done and all.
        if f.source == 0 || self.local.contains(&f.source) {
            if self.trace {
                eprintln!("chudp {now:>6}: from {from}: {:o} is on this cable", f.source);
            }
            return;
        }
        // muir is a leaf: what is not for this cable is dropped, never
        // forwarded.
        if dest != 0 && !self.local.contains(&dest) {
            if self.trace {
                eprintln!("chudp {now:>6}: from {from}: {dest:o} is not on this cable");
            }
            return;
        }
        if self.trace {
            eprintln!(
                "chudp {now:>6}: from {from}: {:o} -> {dest:o} {} for the cable{}",
                f.source,
                super::packet::op_name(p.opcode),
                if f.check_ok { "" } else { ", its check word not the hardware's" }
            );
        }
        self.out.push_back((f.source, f.buffer));
    }

    /// Where a frame off the cable goes: the peer it is addressed to, or
    /// every peer if it is a broadcast, since they are stations on this
    /// cable.
    fn addressed(&self, dest: u16) -> Vec<SocketAddr> {
        match dest {
            0 => self.peers.values().copied().collect(),
            d => self.peers.get(&d).copied().into_iter().collect(),
        }
    }
}

impl Node for Chudp {
    fn address(&self) -> u16 {
        self.source
    }

    fn receive(&mut self, now: u64, packet: &Framed) {
        // Wreckage: a collision's remains, or a frame whose check word
        // failed. The cable drops it as a receiver does.
        if !packet.check_ok {
            return;
        }
        // A frame this node put on the cable itself, come back around
        // while it was speaking for another peer.
        if self.peers.contains_key(&packet.source) {
            return;
        }
        let Some(&dest) = packet.buffer.last() else { return };
        let to = self.addressed(dest);
        if to.is_empty() {
            return;
        }
        // Only a whole packet goes out: the count must account for the
        // words, or the peer cannot read what it is sent.
        let Ok((p, _)) = Packet::from_buffer(&packet.buffer) else { return };
        let Some(datagram) = wrap(&packet.buffer, packet.source, packet.check) else { return };
        for addr in to {
            if self.trace {
                eprintln!(
                    "chudp {now:>6}: {:o} -> {dest:o} {} to {addr}",
                    packet.source,
                    super::packet::op_name(p.opcode)
                );
            }
            if let Err(e) = self.socket.send_to(&datagram, addr)
                && self.trace
            {
                eprintln!("chudp {now:>6}: to {addr}: {e}");
            }
        }
    }

    fn transmit(&mut self, now: u64) -> Option<Vec<u16>> {
        self.drain(now);
        let (source, buffer) = self.out.pop_front()?;
        self.source = source;
        Some(buffer)
    }

    /// A frame aborted on interference goes again at the next turn,
    /// ahead of anything that arrived since, as an interface's driver
    /// retries on Transmit Abort.
    fn aborted(&mut self, _now: u64, buffer: Vec<u16>) {
        self.out.push_front((self.source, buffer));
    }
}
