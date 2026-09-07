// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The netlist board on the ether: its transceiver's four wires read
//! and driven, as [`crate::disk_unit::OnCable`] does the drive's.
//!
//! The transceiver at LMDETC A03 has no model; what is modelled is what
//! it hands the board and takes from it. Out: `TRANS.DATA+`, the line
//! driver's true output, high while the interface puts a one on the
//! cable, and released while Loop Back holds the driver off. In: the
//! receive pair, and the interference pair. Their sense is the board's
//! own, and MIT's wiring settles it on the transmit side first:
//! `cadrio/iob.wlr` puts the active-low `-TTL.D.OUT` on the line driver's
//! input at A02-01, its true output on `TRANS.DATA-` and its complement on
//! `TRANS.DATA+`, so sending a one puts the plus above the minus. Plus
//! above minus is a one, and the receive pair is read the same way:
//! `RCVR.DATA+` is above `RCVR.DATA-` while the cable carries a one. The
//! receiver's own output is the inverse of that, because MIT crossed the
//! pair into it, which is why it is named `-RCVR.DATA.IN`; the inverting
//! 74S158 at LMLNDR 0E02 turns it back into `TTL.D.IN`, the line level.
//! `INTERFERENCE IN` high, `INTERFERE+` above `INTERFERE-`, is
//! interference.

use super::ether::Ether;
use crate::chip::Chip;
use crate::netlist::{NetId, Netlist};
use crate::part::Level;

/// The transceiver's four wires and the line driver's output, by net.
#[derive(Clone, Copy, Debug)]
pub struct Nets {
    trans_p: NetId,
    rcvr_p: NetId,
    rcvr_m: NetId,
    intf_p: NetId,
    intf_m: NetId,
}

impl Nets {
    /// The nets on `n`, the I/O board.
    pub fn of(n: &Netlist) -> Nets {
        let net = |name: &str| n.by_name_id(name).unwrap_or_else(|| panic!("no net {name}"));
        Nets {
            trans_p: net("TRANS.DATA+"),
            rcvr_p: net("RCVR.DATA+"),
            rcvr_m: net("RCVR.DATA-"),
            intf_p: net("INTERFERE+"),
            intf_m: net("INTERFERE-"),
        }
    }
}

pub struct OnCable {
    pub ether: Ether,
    nets: Nets,
    last: Option<(bool, bool)>,
}

impl OnCable {
    /// Wires `ether` to the transceiver nets of `n`, the I/O board.
    pub fn new(n: &Netlist, ether: Ether) -> OnCable {
        OnCable::from_nets(Nets::of(n), ether)
    }

    /// The same, with the nets already found.
    pub fn from_nets(nets: Nets, ether: Ether) -> OnCable {
        OnCable { ether, nets, last: None }
    }

    /// Whether the netlist is a board with the transceiver on it.
    pub fn fits(n: &Netlist) -> bool {
        n.by_name_id("TRANS.DATA+").is_some()
    }

    /// Forgets what was last put on the nets, for a board brought up again.
    pub fn reattach(&mut self) {
        self.last = None;
    }

    /// What the board is putting on the cable: a one while the line
    /// driver drives its true output high, nothing while it is off.
    pub fn board_tx(&self, c: &Chip) -> bool {
        matches!(c.board_level(self.nets.trans_p), (Level::High, true))
    }

    /// Gives the ether what the board drives at `now`, lets it do what is
    /// due, and puts the cable's level and interference on the board's
    /// receive pairs, settled at `now`. Returns whether the nets moved.
    pub fn apply(&mut self, c: &mut Chip, now: u64) -> bool {
        let tx = self.board_tx(c);
        self.ether.board(now, tx);
        self.ether.at(now);
        let want = (self.ether.level(), self.ether.interference());
        if self.last == Some(want) {
            return false;
        }
        let lv = |on: bool| if on { Level::High } else { Level::Low };
        let (level, interference) = want;
        c.drive(self.nets.rcvr_p, lv(level));
        c.drive(self.nets.rcvr_m, lv(!level));
        c.drive(self.nets.intf_p, lv(interference));
        c.drive(self.nets.intf_m, lv(!interference));
        c.transition(now);
        self.last = Some(want);
        true
    }

    /// When the ether next has something of its own to do after `now`.
    pub fn next_change(&self, now: u64) -> Option<u64> {
        self.ether.next_due().map(|t| t.max(now))
    }
}
