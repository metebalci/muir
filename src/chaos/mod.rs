// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Chaosnet: the cable the I/O board's other half is on, and what is
//! on it besides this machine.
//!
//! The layers, each usable without the ones above it, so that another
//! service is a new [`server::Service`] and a bridge to a wider Chaosnet is
//! a new node on the [`ether`] rather than a change to either:
//!
//! - [`interface`]: the Unibus interface's registers, from AIM-628 §7.
//!   What the software sees; the netlist board is the implementation.
//! - [`packet`]: a packet as the hardware carries it and as the software
//!   header lays it out, AIM-628 §2.2 and §3.5, with the check word.
//! - [`wire`]: the biphase coding on the cable, and a decoder that does
//!   what the board's detector does.
//! - [`ether`]: the cable as a medium, the [`ether::Node`]s on its model
//!   side, and the etiquette of who may transmit when.
//! - [`cable`]: the netlist board on the ether, through its transceiver's
//!   four wires.
//! - [`server`]: a host on the ether, the transport protocol of chapters 3
//!   and 4, and the [`server::Service`] trait its services are written to.
//! - [`time`]: the TIME and UPTIME services, §5.8.
//! - [`mod@file`]: the FILE service, the band's file server: it reads, writes,
//!   renames and deletes under the root `--chaos-file-root` names.
//! - [`udp`]: Chaosnet over UDP, the stations on the cable that are not
//!   in this process --- a node like any other, taking its turn.
//!
//! The authority is MIT A.I. Memo 628, *Chaosnet*, David A. Moon, 1981,
//! the scan the tree carries, read
//! beside the Lisp Machine's own NCP in `sys/network/chaos/` for what the
//! booted system actually sends.

pub mod board;
pub mod cable;
pub mod ether;
pub mod file;
pub mod interface;
pub mod packet;
pub mod server;
pub mod status;
pub mod time;
pub mod udp;
pub mod wire;

/// What a run puts on the Chaosnet: this machine's address, and the
/// **Chaosnet server** on the other end of the cable --- muir's
/// implementation of the server side of STATUS, TIME, UPTIME and FILE,
/// standing in for the servers that ran on the machine's **associated
/// machine**, MIT's term (`si:associated-machine`, `sys/man/fd-hac.text`)
/// for the file and time server a Lisp Machine talks to, which the boot
/// banner names: "with associated machine OZ". The server is not a model
/// of that machine, only of its services; and TIME need not come from it
/// at all, the band asking the whole network by broadcast.
///
/// **Which numbers a run wants is the band's, and the band is asked.** A
/// band holds a host table, and it calls its file and time host at the
/// address that table gives; a server answering anywhere else is a server
/// it never calls, and the machine then boots but stops to ask for the
/// date and reaches no files.
///
/// System 100's band is `MIT-LISPM-1` at 3050 and calls its file and time
/// host `MIT-OZ` at 3060, the `SYS` host of `site.lisp`, from
/// `vendor/system-100-0/sys/site/hosts.text`; the release's builders
/// trimmed that table to exactly those two and gave OZ that address (its
/// `README`; discrepancy 60), and the release's own configuration runs the
/// band as them, so they are what that band expects rather than MIT's
/// historical numbers. A run with that pack is given
/// `--chaos-address 3050,3060`.
///
/// System 304's band answers differently, and was asked at its listener:
/// `si:local-host` is `AMS-LISPM-1` at 4401, and `OZ` --- `AMS-BRIDGE-1`
/// under its other name --- is at 4403, which is where it looks for both
/// its files and its time. A run with that pack is given
/// `--chaos-address 4401,4403`.
///
/// **The defaults are neither band's, and are deliberately no real
/// host's**: this machine 177001 and the server 177002, subnet 376. muir
/// models the CADR and not one distribution of it, so a default taken from
/// one band's host table would be the wrong default for every other; and
/// whatever is defaulted to can end up on a cable, since `--chaos-udp`
/// puts this machine on the network. Subnet 376 is the Chaosnet's private,
/// non-routable range --- its 192.168 --- so a machine started with no
/// `--chaos-address` cannot collide with an address allocated on a real
/// Chaosnet. That the range is set aside for this is the Global Chaosnet's
/// own convention and not MIT's, and it is **unverified** against any MIT
/// file: no MIT document here reserves a subnet. What does not depend on
/// the convention is the part that matters --- no host table in this tree
/// names a host on subnet 376, System 100's holding exactly `MIT-LISPM-1`
/// and `MIT-OZ` and nothing else, so no band here calls it. Whether MIT
/// ever assigned 376 is likewise **unverified**: what is here is the
/// release's trimmed table and not MIT's network-wide one, and a copy of
/// that table would settle it. So the pair is a working one,
/// this machine and its server hearing each other on the modelled cable as
/// they always did, and it is a pair no band goes looking for: a run that
/// wants its band to reach the server names the band's own.
///
/// On `muir`: `--chaos-address <this>[,<server>]`, `--chaos-file-root`;
/// `--chaos-trace` prints every packet.
#[derive(Clone, Debug)]
pub struct Config {
    /// The sixteen address switches on the I/O board.
    pub address: u16,
    /// The associated machine's address: where the server answers.
    pub server_address: u16,
    /// The name the server answers STATUS with. `MIT-OZ` by default,
    /// which is System 100's own name in `sys/site/hosts.text` for the
    /// file and time host it calls at 3060. The default address is not
    /// that band's, so the name is no longer that band's name for that
    /// address; it is what STATUS prints, and `(hostat)` is what prints
    /// it. There is no flag for it: a run that wants another sets the
    /// field, as `tests/cc_harness` sets `OZ` for System 304.
    pub server_name: String,
    /// The directory the server's FILE service serves as its `/`; none,
    /// and there is no FILE service.
    pub file_root: Option<std::path::PathBuf>,
    /// Every packet the server handles or sends, printed as it goes.
    pub trace: bool,
    /// The universal time the server's TIME service answers with and its
    /// FILE service dates new files by, fixed; none, and both use the
    /// machine's clock.  The tests fix it ([`time::TEST_UNIVERSAL`])
    /// so a boot over the model network does the same work every run.
    pub time: Option<u32>,
    /// The CHUDP link, bound: the socket this machine's cable reaches
    /// other Chaosnet hosts over, and the peers on it. None, and the
    /// cable carries this machine and the Chaosnet server and nothing
    /// else. `--chaos-udp`, `--chaos-udp-peer`, `--chaos-udp-dynamic`.
    pub udp: Option<udp::Link>,
    /// The Chaosnet addresses besides this machine's that FILE serves,
    /// `--chaos-file-peers`.
    ///
    /// **Reachability and authorisation are separate.** A peer that a
    /// packet reached this machine from can be answered; being answerable
    /// is not being allowed to read and write the file root, which is a
    /// real directory served under containment rules written for a cable
    /// with one trusted machine on it. So a peer is named here or it is
    /// refused at the RFC. TIME, UPTIME and STATUS answer anyone: they
    /// give nothing away.
    pub file_peers: Vec<u16>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            address: 0o177001,
            server_address: 0o177002,
            server_name: "MIT-OZ".to_string(),
            file_root: None,
            trace: false,
            time: None,
            udp: None,
            file_peers: Vec::new(),
        }
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
    /// The Chaosnet server the configuration describes, with its services,
    /// at `powered_at` on the ether's clock.
    pub fn server(&self, powered_at: u64) -> server::Server {
        let mut h = server::Server::new(self.server_address);
        h.trace = self.trace;
        h.serve(Box::new(match self.time {
            Some(t) => time::Time::fixed(t),
            None => time::Time::new(),
        }));
        h.serve(Box::new(time::Uptime::new(powered_at)));
        // STATUS, so that `(hostat)` on the machine sees the server at all:
        // the name is the band's for this address, and the subnet its high
        // byte.
        h.serve(Box::new(
            status::Status::new(&self.server_name).on_subnet((self.server_address >> 8) as u8),
        ));
        if let Some(root) = &self.file_root {
            // Who may have files: this machine, always, and whoever
            // `--chaos-file-peers` names. A CHUDP peer that was only
            // learned is reachable and not authorised.
            let mut hosts = vec![self.address];
            hosts.extend(&self.file_peers);
            h.serve(Box::new(file::File::new(root.clone()).with_time(self.time).serving(hosts)));
        }
        h
    }

    /// The CHUDP node for this machine's cable, if a link was bound. The
    /// addresses already on that cable are this machine's and the
    /// Chaosnet server's, which the node neither learns nor speaks for.
    pub fn udp_node(&self) -> Option<Box<dyn ether::Node>> {
        let link = self.udp.as_ref()?;
        Some(Box::new(link.node(&[self.address, self.server_address], self.trace)))
    }
}
