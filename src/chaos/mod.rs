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
/// The defaults are the band's own, from `vendor/system-100-0/sys/site/
/// hosts.text`: this machine is `MIT-LISPM-1` at 3050, and its associated
/// machine is `MIT-OZ` at 3060, the `SYS` host of `site.lisp`. The
/// release's builders trimmed that table to exactly those two and gave OZ
/// that address (its `README`; discrepancy 60), and the release's own
/// configuration runs the band as them, so they are what this band
/// expects rather than MIT's historical numbers. On `muir`:
/// `--chaos-address <this>[,<server>]`, `--chaos-file-root`;
/// `--chaos-trace` prints every packet.
#[derive(Clone, Debug)]
pub struct Config {
    /// The sixteen address switches on the I/O board.
    pub address: u16,
    /// The associated machine's address: where the server answers.
    pub server_address: u16,
    /// The name the server answers STATUS with. `MIT-OZ` by default, the
    /// band's own name for 3060 in `sys/site/hosts.text`.
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
}

impl Default for Config {
    fn default() -> Config {
        Config {
            address: 0o3050,
            server_address: 0o3060,
            server_name: "MIT-OZ".to_string(),
            file_root: None,
            trace: false,
            time: None,
        }
    }
}

/// Reads an address as the flags take it: the sixteen bits in octal,
/// `3050`, or as `subnet:host` with each half in octal, `6:50`. The two
/// are one number --- the subnet is the high byte and the host the low
/// (AIM-628) --- which octal digits do not show, three bits to a digit
/// against eight to a byte; that is why `3050` reads as "subnet 6, host
/// 50" only once split. Each half must fit its byte; anything else is
/// `None`.
pub fn parse_address(s: &str) -> Option<u16> {
    let byte = |s: &str| u16::from_str_radix(s, 8).ok().filter(|&b| b <= 0o377);
    match s.split_once(':') {
        Some((subnet, host)) => Some(byte(subnet)? << 8 | byte(host)?),
        None => u16::from_str_radix(s, 8).ok(),
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
            h.serve(Box::new(file::File::new(root.clone()).with_time(self.time)));
        }
        h
    }
}
