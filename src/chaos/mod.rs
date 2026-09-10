// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Chaosnet: the cable the I/O board's other half is on, and how this
//! machine reaches what is on it.
//!
//! The layers, each usable without the ones above it, so that a bridge to
//! a wider Chaosnet is a new node on the [`ether`] rather than a change to
//! any of the others:
//!
//! - [`interface`]: the Unibus interface's registers, from AIM-628 §7.
//!   What the software sees; the netlist board is the implementation.
//! - [`packet`]: a packet as the hardware carries it and as the software
//!   header lays it out, AIM-628 §2.2 and §3.5, with the check word and
//!   chapter 4's opcodes.
//! - [`wire`]: the biphase coding on the cable, and a decoder that does
//!   what the board's detector does.
//! - [`ether`]: the cable as a medium, the [`ether::Node`]s on its model
//!   side, and the etiquette of who may transmit when.
//! - [`cable`]: the netlist board on the ether, through its transceiver's
//!   four wires.
//! - [`udp`]: Chaosnet over UDP, the stations on the cable that are not
//!   in this process --- a node like any other, taking its turn.
//!
//! **What answers is not here, because it was never in the machine.** A
//! CADR has no file or time server inside it: STATUS, TIME, UPTIME and
//! FILE are the far end of the cable, what MIT's associated machine ran,
//! and a band that wants them wants a host on the network. A run reaches
//! one over [`udp`]. The tests, which cannot want a daemon running beside
//! them, put a server of their own on the modelled cable through
//! [`crate::machine::Machine::attach_chaos_node`]; it lives in
//! `tests/support/` and is still called **the Chaosnet server**.
//!
//! The authority is MIT A.I. Memo 628, *Chaosnet*, David A. Moon, 1981,
//! the scan the tree carries, read
//! beside the Lisp Machine's own NCP in `sys/network/chaos/` for what the
//! booted system actually sends.

pub mod board;
pub mod cable;
pub mod ether;
pub mod interface;
pub mod packet;
pub mod udp;
pub mod wire;

/// What a run puts on the Chaosnet: this machine's address, and the CHUDP
/// link its cable reaches the rest of the network over.
///
/// **A band wants a file and time host, and muir is not one.** A Lisp
/// Machine calls its **associated machine** --- MIT's term
/// (`si:associated-machine`, `sys/man/fd-hac.text`) for the file and time
/// server a Lisp Machine talks to, which the boot banner names: "with
/// associated machine OZ" --- for its files, and for the date unless a
/// broadcast is answered first. The CADR had no such server inside it, so
/// muir has none either: the host is another program on the network, and
/// `ozd`, `https://github.com/metebalci/ozd`, is one that boots a band.
///
/// **Which numbers a run wants are the band's, and the band is asked.** A
/// band holds a host table, and it calls its file and time host at the
/// address that table gives; a host answering anywhere else is a host it
/// never calls, and the machine then boots but stops to ask for the date
/// and reaches no files.
///
/// System 100's band is `MIT-LISPM-1` at 3050 and calls its file and time
/// host `MIT-OZ` at 3060, the `SYS` host of `site.lisp`, from
/// `vendor/system-100-0/sys/site/hosts.text`; the release's builders
/// trimmed that table to exactly those two and gave OZ that address (its
/// `README`; discrepancy 60), and the release's own configuration runs the
/// band as them, so they are what that band expects rather than MIT's
/// historical numbers. A run with that pack is given
/// `--chaos-address 3050`, and reaches its host with
/// `--chaos-udp-peer 3060@<where the host is>`.
///
/// System 304's band answers differently, and was asked at its listener:
/// `si:local-host` is `AMS-LISPM-1` at 4401, and `OZ` --- `AMS-BRIDGE-1`
/// under its other name --- is at 4403, which is where it looks for both
/// its files and its time. A run with that pack is given
/// `--chaos-address 4401` and `--chaos-udp-peer 4403@<where the host is>`.
///
/// **The default is no real host's address, deliberately**: 177001, on
/// subnet 376. muir models the CADR and not one distribution of it, so a
/// default taken from one band's host table would be the wrong default for
/// every other; and whatever is defaulted to can end up on a cable, since
/// `--chaos-udp` puts this machine on the network. Subnet 376 is the
/// Chaosnet's private, non-routable range --- its 192.168 --- so a machine
/// started with no `--chaos-address` cannot collide with an address
/// allocated on a real Chaosnet. That the range is set aside for this is
/// the Global Chaosnet's own convention and not MIT's, and it is
/// **unverified** against any MIT file: no MIT document here reserves a
/// subnet. What does not depend on the convention is the part that matters
/// --- no host table in this tree names a host on subnet 376, System 100's
/// holding exactly `MIT-LISPM-1` and `MIT-OZ` and nothing else, so no band
/// here calls it. Whether MIT ever assigned 376 is likewise
/// **unverified**: what is here is the release's trimmed table and not
/// MIT's network-wide one, and a copy of that table would settle it.
///
/// On `muir`: `--chaos-address <address>`, which is also what starts
/// Chaosnet over UDP; `--chaos-udp`, `--chaos-udp-peer` and
/// `--chaos-udp-dynamic` for the link and who is on it; `--chaos-trace`
/// prints every packet.
#[derive(Clone, Debug)]
pub struct Config {
    /// The sixteen address switches on the I/O board.
    pub address: u16,
    /// Every packet and frame on the cable, printed as it goes.
    pub trace: bool,
    /// The CHUDP link, bound: the socket this machine's cable reaches
    /// other Chaosnet hosts over, and the peers on it. None, and the cable
    /// carries this machine and whatever else this process put on it, and
    /// nothing from beyond the host. `--chaos-udp`, `--chaos-udp-peer`,
    /// `--chaos-udp-dynamic`.
    pub udp: Option<udp::Link>,
}

impl Default for Config {
    fn default() -> Config {
        Config { address: 0o177001, trace: false, udp: None }
    }
}

/// Reads an address as the flags take it: the sixteen bits in octal,
/// `3050`, or as `subnet:host` with each half in octal, `6:50`. The two
/// are one number --- the subnet is the high byte and the host the low
/// (AIM-628) --- which octal digits do not show, three bits to a digit
/// against eight to a byte; that is why `3050` reads as "subnet 6, host
/// 50" only once split. Each half must fit its byte.
///
/// **Neither half may be zero, because a zero host is not a host.** MIT's
/// own description of the interface, quoted in [`crate::chaos::interface`]:
/// the destination word is "the cable address of the destination of the
/// packet, or 0 to broadcast it", and a receiver stores the next packet
/// "addressed to this node, or is broadcast". So `6:0` names every host on
/// subnet 6 rather than one of them, and a machine configured as it would
/// take the whole subnet's traffic for its own. A zero subnet is refused
/// for the same reason, which also refuses every bare octal below `400`.
///
/// Anything else is `None`.
pub fn parse_address(s: &str) -> Option<u16> {
    let byte = |s: &str| u16::from_str_radix(s, 8).ok().filter(|&b| b <= 0o377);
    let both = |a: u16| (a >> 8 != 0 && a & 0o377 != 0).then_some(a);
    match s.split_once(':') {
        Some((subnet, host)) => both(byte(subnet)? << 8 | byte(host)?),
        None => u16::from_str_radix(s, 8).ok().and_then(both),
    }
}

impl Config {
    /// The CHUDP node for this machine's cable, if a link was bound. The
    /// address already on that cable is this machine's, which the node
    /// neither learns nor speaks for.
    pub fn udp_node(&self) -> Option<Box<dyn ether::Node>> {
        let link = self.udp.as_ref()?;
        Some(Box::new(link.node(&[self.address], self.trace)))
    }
}
