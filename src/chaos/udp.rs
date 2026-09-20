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
//! A frame goes out over UDP only when a station of this process put it
//! on the cable ([`Chudp::receive`]), so what arrives from one peer is
//! never sent to another.
//!
//! **The bridge is reached as the default peer.** A peer entry says
//! that one Chaosnet address lives at one endpoint, so naming a bridge
//! as a peer does not let muir talk *through* it: a frame for any other
//! address has nowhere to go. [`Link::default_peer`] is where such a
//! frame goes instead, which is the route of last resort and the whole
//! of muir's routing --- nothing here reads a routing packet.
//!
//! **The link is a [`super::ether::Node`] and nothing else.** It waits
//! its turn on the modeled cable as any other station does, so the board
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
//! ---- the trailer ----
//! 20 + n      2  destination
//! 22 + n      2  source
//! 24 + n      2  checksum
//! ```
//!
//! `version = 1` and `function = 1`, for "here is a Chaos packet", the
//! only function defined. The default port is 42042. [`VERSION`] is
//! checked on receipt: an unknown version is refused with its number
//! rather than read as this one.
//!
//! **Every 16-bit word goes most significant byte first** --- the
//! header's words, the data's and the trailer's. The data bytes are
//! packed into those words as AIM-628 §3.6 says, the first byte of a pair
//! in the word's least significant half, so a pair appears swapped on the
//! wire: `STATUS` goes out as `TSTASU`. An odd byte count is padded to a
//! whole word, the zero landing in the last word's high half, which is
//! the byte that goes first.
//!
//! **The trailer's first two words are the cable's**: the address on this
//! subnet the frame is for, which is the next hop and not necessarily the
//! packet's own destination, and the sender's own address. **Its third is
//! the Internet checksum**, the one's complement of the one's complement
//! sum of every word before it --- not the CADR's CRC-16. [`checksum`]
//! computes it, and [`unwrap`] refuses a frame whose own does not agree.
//!
//! ## Where the framing was read from
//!
//! **From `cbridge` running.** On 15 September 2026 a `cbridge` was run
//! with a test configuration and watched through its log and a packet
//! capture: frames in this framing were accepted and forwarded; frames
//! with their words least significant byte first were refused as "bogus",
//! the source address reported byte-swapped and the opcode as 0; and
//! frames in this byte order carrying the CADR's CRC-16 in the trailer
//! were refused with "Bad checksum", the value printed being exactly the
//! one's complement sum above. `cbridge`'s own frames verify with that
//! checksum, and `tests/chudp.rs` pins one of them whole beside the
//! datagram muir writes. The bridge's sources were **not** read: its
//! author forbids language models to read or process them, which is the
//! author's decision about the author's own work and is kept here.
//!
//! The protocol page at `chaosnet.net/protocol` §2.3 names the same
//! Internet checksum and is **wrong about the byte order**: it says
//! `cbridge` sends words least significant byte first, which the running
//! `cbridge` does not.
//!
//! What `usim` and `klh10` write is **unverified**: only `cbridge` was
//! watched, and it is the authority here because a muir on a real
//! Chaosnet reaches it through `cbridge`. Watching one of them, or one
//! interoperation, would settle it.
//!
//! **The conversion belongs here, at the edge where the machine meets
//! UDP.** The CADR's own sources and netlist decide the packet's words,
//! how data bytes go into them, and what its interface puts on its cable,
//! the 9401's CRC-16 included. They decide nothing about a UDP datagram:
//! no CADR ever sent one. So a packet leaving the machine goes out with
//! its words in network order and the Internet checksum in the trailer,
//! and a frame arriving has that checksum checked and is given the check
//! word the interface will want when it comes off the modeled cable.

use super::ether::Node;
use super::packet::{Framed, MAX_DATA, Packet, check_word};
use std::collections::{BTreeMap, VecDeque};
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

/// The trailer: destination, source, checksum. The first two words are
/// the hardware trailer's of AIM-628 §2.2; the third is not.
pub const TRAILER: usize = 6;

/// The software header, AIM-628 §3.5: eight 16-bit words.
const SOFTWARE_HEADER: usize = 16;

/// The most a CHUDP frame can be: the header, the software header,
/// [`MAX_DATA`] bytes of data, and the trailer. A datagram longer than
/// this cannot be a Chaos packet, so the receive buffer is one byte
/// more --- a longer datagram then fills it, and is refused for its
/// length rather than read as a truncated packet.
pub const MAX_FRAME: usize = HEADER + SOFTWARE_HEADER + MAX_DATA + TRAILER;

/// How many times one frame is put on the cable before it is given up
/// on: microcode 323's `uc-chaos.lisp` assigns
/// `CHAOS-NUMBER-TRANSMIT-RETRIES 3` --- "Send once and retry twice if
/// aborted" --- loads `A-CHAOS-TRANSMIT-RETRY-COUNT` with it for a fresh
/// packet, counts it down on every Transmit Abort in `CHAOS-XMT-INTR`,
/// and at zero jumps to `CHAOS-XMT-DONE`, which takes the packet off the
/// transmit list and is done with it.  The packet is then the transport's
/// to retransmit, as it is for anything else the cable loses.
///
/// The station at the far end of a CHUDP link is not modeled --- muir has
/// no model of the host at the other end of the socket --- so what stands
/// in for its driver is the one driver this project can read, the CADR's,
/// and this is its bound.
pub const TRANSMIT_TRIES: u8 = 3;

/// How many datagrams are taken from the socket at one turn on the
/// cable. The cable carries one frame at a time, so the rest wait in
/// [`Chudp`]'s queue; this only bounds how long one call spends in the
/// kernel when a peer is flooding.
const DRAIN: usize = 64;

/// The Internet checksum of `words`: the one's complement of their
/// one's complement sum, which is what a frame's trailer carries in its
/// third word. The words summed are every one before it --- the eight
/// header words, the data words, and the trailer's destination and
/// source.
///
/// A frame is good when all of its words, the checksum included, sum to
/// 0xffff, which is to say that `checksum` over the lot is 0.
pub fn checksum(words: &[u16]) -> u16 {
    let mut sum = 0u32;
    for &w in words {
        sum += w as u32;
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(((sum & 0xffff) + (sum >> 16)) as u16)
}

/// A frame as it stood on the modeled cable, as a CHUDP datagram.
///
/// `buffer` is the buffer the software wrote, its last word the cable
/// destination, and `source` the address the sending interface put in.
/// **The check word the cable carried is not wanted and is not asked
/// for**: the trailer's third word here is the Internet checksum, made
/// over the words of this frame. `None` if there is not even a
/// destination word to put in the trailer.
pub fn wrap(buffer: &[u16], source: u16) -> Option<Vec<u8>> {
    if buffer.is_empty() {
        return None;
    }
    let mut words = Vec::with_capacity(buffer.len() + 2);
    words.extend_from_slice(buffer);
    words.push(source);
    words.push(checksum(&words));
    let mut out = Vec::with_capacity(HEADER + words.len() * 2);
    out.extend([VERSION, PACKET, 0, 0]);
    for w in words {
        out.extend(w.to_be_bytes());
    }
    Some(out)
}

/// The frame back out of a datagram, as the cable would hand it to a
/// node: the buffer with the cable destination last, the source the far
/// side put in the trailer, and a check word for the modeled cable.
///
/// **The checksum is checked here and a frame that fails it is refused**,
/// as `cbridge` refuses one with "Bad checksum". What then goes on the
/// modeled cable carries the CADR's own CRC-16, [`check_word`], made for
/// these words: that is what the interface checks on receipt, and no UDP
/// peer has one to send.
///
/// The data is a whole number of 16-bit words: `cbridge` pads an odd byte
/// count and so does [`wrap`], and a body that is not whole words has no
/// words to sum, so it is refused for the length its count does not
/// answer.
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
    let count = (u16::from_be_bytes([body[2], body[3]]) & 0o7777) as usize;
    if count > MAX_DATA {
        return Err(format!("a data count of {count}, and the most is {MAX_DATA}"));
    }
    if body.len() != SOFTWARE_HEADER + count.next_multiple_of(2) {
        return Err(format!("{} bytes of packet against a data count of {count}", body.len()));
    }
    let word = |b: &[u8]| u16::from_be_bytes([b[0], b[1]]);
    // The count held the body to whole words above, so nothing is left
    // over.
    let mut buffer: Vec<u16> =
        body.as_chunks::<2>().0.iter().copied().map(u16::from_be_bytes).collect();
    let trailer = &datagram[datagram.len() - TRAILER..];
    buffer.push(word(&trailer[0..]));
    let source = word(&trailer[2..]);
    let mut over = buffer.clone();
    over.push(source);
    let want = checksum(&over);
    let carried = word(&trailer[4..]);
    if carried != want {
        return Err(format!("a checksum of {carried:#06x}, and these words make {want:#06x}"));
    }
    // What goes on the modeled cable is checked there by the interface,
    // against the 9401's CRC-16; a CHUDP peer has no such word to send,
    // so it is made here.
    Ok(Framed { buffer, source, check: check_word(&over), check_ok: true })
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
    /// Where a directed frame goes whose destination is in no peer
    /// entry: the route of last resort, and an endpoint rather than a
    /// host at an address. `--chaos-udp-default-peer`. None, and such a
    /// frame is dropped.
    pub default_peer: Option<SocketAddr>,
    socket: Arc<UdpSocket>,
}

impl Link {
    /// Binds the socket at `at` and holds the peers named for it.
    pub fn bind(
        at: SocketAddr,
        peers: Vec<(u16, SocketAddr)>,
        default_peer: Option<SocketAddr>,
    ) -> std::io::Result<Link> {
        let socket = UdpSocket::bind(at)?;
        socket.set_nonblocking(true)?;
        let at = socket.local_addr().unwrap_or(at);
        Ok(Link { at, peers, default_peer, socket: Arc::new(socket) })
    }

    /// The node this link puts on a cable. `local` are the addresses
    /// already on that cable --- this machine's, and whatever else this
    /// process put there --- which are never sent out, never spoken for,
    /// and the only sources whose frames leave over UDP.
    pub fn node(&self, local: &[u16], trace: bool) -> Chudp {
        Chudp {
            socket: self.socket.clone(),
            peers: self.peers.iter().copied().collect(),
            default_peer: self.default_peer,
            local: local.to_vec(),
            out: VecDeque::new(),
            source: 0,
            tries: 0,
            trace,
        }
    }
}

/// The peers of this cable that are reached over UDP, as one node on it.
///
/// **Its address on the cable is the address of the peer whose frame it
/// is carrying.** A CHUDP peer is a station on this machine's cable ---
/// that is what a leaf's link is --- so a packet from 3040 goes onto the
/// modeled cable with 3040 in the hardware source, exactly as 3040's
/// own interface would have put it there, and the board's turn timer
/// loads from it as it would from any other station. Between frames it
/// is 0, which is no station's address: the ether uses it only to keep a
/// node from hearing its own frame.
pub struct Chudp {
    socket: Arc<UdpSocket>,
    /// Where each peer lives, as the flags named them. **A packet does
    /// not move one and never adds one**: an endpoint typed on the
    /// command line is a statement, and a table learned from packets
    /// would put the naming in the hands of whoever can reach the port
    /// and leave a run with state nobody wrote down.
    peers: BTreeMap<u16, SocketAddr>,
    /// Where a frame goes that no entry above names. [`Link::default_peer`].
    default_peer: Option<SocketAddr>,
    /// The addresses that are on this cable in this process.
    local: Vec<u16>,
    /// Frames waiting for a turn on the cable: whose each is, the
    /// buffer, and how many times it has been put on the cable already.
    out: VecDeque<(u16, Vec<u16>, u8)>,
    /// Whose frame the node is putting on the cable, which is its
    /// address there, and how many times that frame has been sent.
    source: u16,
    tries: u8,
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
                "chudp {now:>6}: from {from}: {:o} -> {dest:o} {} for the cable",
                f.source,
                super::packet::op_name(p.opcode)
            );
        }
        self.out.push_back((f.source, f.buffer, 0));
    }

    /// Where a frame off the cable goes: the peer it is addressed to,
    /// the default peer if no entry names its destination, or every
    /// peer if it is a broadcast.
    ///
    /// **A broadcast does not go to the default peer.** The named peers
    /// are stations on this machine's cable and a broadcast is theirs;
    /// the default peer is the way out to a wider network, and handing
    /// it a broadcast would put this cable's on a network the broadcast
    /// was never meant to reach. Decided, and not an oversight.
    ///
    /// A destination already on this cable goes nowhere: a station of
    /// this process is reached on the cable, not over UDP, and it is
    /// not the bridge's business either.
    fn addressed(&self, dest: u16) -> Vec<SocketAddr> {
        match dest {
            0 => self.peers.values().copied().collect(),
            d if self.local.contains(&d) => Vec::new(),
            d => match self.peers.get(&d) {
                Some(&at) => vec![at],
                None => self.default_peer.into_iter().collect(),
            },
        }
    }
}

impl Chudp {
    /// A frame off the cable that will not go out, and why.
    ///
    /// Under `--chaos-trace` the arriving half names every reason it
    /// drops a datagram; before this the leaving half named none, so a
    /// frame that reached the cable and stopped there showed as the
    /// interface's turn followed by silence --- indistinguishable from a
    /// link that never sends. One line a frame, and only under the
    /// trace.
    fn dropped(&self, now: u64, why: std::fmt::Arguments) {
        if self.trace {
            eprintln!("chudp {now:>6}: not sent: {why}");
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
        // **muir is a leaf.** Only a frame a station of this process put
        // on the cable goes out over UDP; one this node laid there for a
        // peer stops at the cable, so what came from one peer is never
        // sent to another. Said of the source rather than of the peer
        // table because the default peer relays for addresses no entry
        // names, and those are not to be carried on either.
        //
        // **Every way out of here that is not a send says why.** A frame
        // that reaches the cable and no further leaves `--chaos-trace`
        // showing the interface's turn and then nothing at all, which
        // reads exactly like a link that is not sending --- and the
        // arriving half ([`Chudp::arrived`]) has always named its
        // reasons. A person looking at a network that carries nothing
        // needs to know which side gave up on the frame.
        if !self.local.contains(&packet.source) {
            self.dropped(now, format_args!("{:o} is not a station of this process", packet.source));
            return;
        }
        let Some(&dest) = packet.buffer.last() else {
            self.dropped(now, format_args!("an empty frame carries no destination"));
            return;
        };
        let to = self.addressed(dest);
        if to.is_empty() {
            self.dropped(
                now,
                format_args!(
                    "{dest:o} is on this cable, or no peer names it and there is no default peer"
                ),
            );
            return;
        }
        // Only a whole packet goes out: the count must account for the
        // words, or the peer cannot read what it is sent.
        let p = match Packet::from_buffer(&packet.buffer) {
            Ok((p, _)) => p,
            Err(e) => {
                self.dropped(now, format_args!("to {dest:o}: {e}"));
                return;
            }
        };
        let Some(datagram) = wrap(&packet.buffer, packet.source) else {
            self.dropped(
                now,
                format_args!("to {dest:o}: {} words will not frame", packet.buffer.len()),
            );
            return;
        };
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
        let (source, buffer, tries) = self.out.pop_front()?;
        self.source = source;
        self.tries = tries + 1;
        Some(buffer)
    }

    /// A frame aborted --- on interference, or by a receiver whose buffer
    /// was full --- goes again at the next turn, ahead of anything that
    /// arrived since, as an interface's driver retries on Transmit Abort:
    /// `CHAOS-XMT-INTR` finds `A-CHAOS-TRANSMIT-ABORTED` set, falls into
    /// `CHAOS-XMT-0`, and takes the packet still on the transmit list.
    /// It **writes the whole buffer again**, halfword by halfword through
    /// `CHAOS-XMT-2`, before reading `START`, so a retry costs a station
    /// as much as a fresh packet; the ether charges it from the abort.
    ///
    /// After [`TRANSMIT_TRIES`] the frame is dropped and its transport
    /// retransmits, which is what the driver does.
    ///
    /// **The driver's abort timeout is not waited here.**  Between two
    /// tries it sets `A-CHAOS-TRANSMIT-ABORTED` to -1, which disables
    /// transmit-done interrupts until the roughly 60-cycle clock
    /// interrupt of `uc-interrupt.lisp` wakes Chaosnet --- but it also
    /// retransmits at once "if we get woken up or receive a packet during
    /// a transmit abort delay", which on a connection that is carrying
    /// traffic is what happens.  The node takes that case: the frame goes
    /// again at the first turn its refill allows.
    fn aborted(&mut self, now: u64, buffer: Vec<u16>) {
        if self.tries >= TRANSMIT_TRIES {
            if self.trace {
                eprintln!(
                    "chudp {now:>6}: {:o}'s frame aborted {} times, given up on",
                    self.source, self.tries
                );
            }
            return;
        }
        self.out.push_front((self.source, buffer, self.tries));
    }
}
