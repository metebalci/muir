// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The cable between the disk controller and the DISK MULTIPLEXOR.
//!
//! The multiplexor sits between one controller and up to eight Trident
//! drives.  Nine of the controller's drive-cable signals are per-unit and
//! eighteen are shared; the multiplexor fans out the nine and the shared
//! ones go past it to every drive
//! (`the_drive_ports_carry_the_controllers_own_per_unit_signals`).
//!
//! **This carries wires and decides nothing.**  Which unit is selected,
//! how `SEL UNIT ATTENTION` reaches the controller and how `ANY ATTENTION`
//! is made are the multiplexor's own gates, run as
//! [`crate::chip::Chip`] runs any board.  That is deliberate: a model that
//! chose the unit itself would be a one-board machine with a lookup in it,
//! and it would boot, and it would not be MIT's.  Nothing here knows what a
//! unit number is.
//!
//! What MIT's `lmdoc/disk.22` says of the two signals, quoted rather than
//! cited since this repository does not hold that file: Selected Unit
//! Attention is "the attention signal directly from the drive, it is not
//! separately latched in the controller", and Any Attention is "some unit
//! has an attention, you have to select them one after another to find out
//! which".  `tests/dm_cable.rs` holds both to being observably true of the
//! board through this cable, which is the check that the fan-out is the
//! board's and not this file's.

use crate::chip::Chip;
use crate::netlist::{NetId, Netlist};
use crate::part::Level;

/// The signals the cable carries: every name the controller's `DCEDGE`
/// connector and the multiplexor both have.
///
/// `cadrdc/dc.wlr` puts twenty-eight signals on `DCEDGE`
/// (`the_multiplexor_cable_is_mits_edge_connector`).  Twenty-five of them
/// are below.  **The other three are not the multiplexor's**: `XINIT` and
/// the two tags `-CYLINDER TAG` and `-HEAD TAG` have no counterpart on the
/// multiplexor at all --- it carries no net whose name holds `INIT`, `TAG`,
/// `CYL` or `HEAD` --- the tags being shared lines that reach every drive
/// without passing through it.
///
/// Two of the multiplexor's own controller-side signals come from
/// elsewhere for the same reason: `XBUS.POWER.OK` is the backplane's, and
/// `-ANY SELECT` has no post on `DCEDGE`.
pub fn wire_names() -> Vec<String> {
    let mut w: Vec<String> = ["SEL UNIT ATTENTION", "ANY ATTENTION", "UNIT 0 ATTENTION"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    w.extend(
        ["-LOAD DA", "NO SELECT", "MULTIPLE SELECT", "WRITE DATA", "WRITE GATE"]
            .iter()
            .map(|s| s.to_string()),
    );
    // The three the controller and the drive name differently, which is the
    // fan-out itself: `DR2` is `DISK.CLK^` on the controller and
    // `UNIT.0.CLOCK^` on the drive, `DS2` `READ DATA` and `UNIT 0 READ
    // DATA`, `DT2` `BLOCK.CLK^` and `UNIT.0.SECTOR^`.  One wire with one
    // drive; with a multiplexor the controller's name is the cable's and
    // the unit's name is the port's.
    w.extend(["DISK.CLK^", "READ DATA", "BLOCK.CLK^"].iter().map(|s| s.to_string()));
    w.extend((0..8).map(|k| format!("BLOCK.CTR{k}")));
    w.extend((28..31).map(|k| format!("XBI{k}")));
    w.extend((0..3).map(|k| format!("UNIT{k}")));
    w
}

fn find(n: &Netlist, name: &str) -> Option<NetId> {
    n.by_name_id(name).or_else(|| n.by_name_id(&format!("'{name}'")))
}

struct Wire {
    controller: NetId,
    board: NetId,
}

/// The multiplexor on the controller's cable: what either board's drivers
/// hold, carried to the other, exactly as [`crate::unibus::Unibus`] carries
/// the Unibus between the bus interface and the I/O board.
pub struct Dm {
    wires: Vec<Wire>,
    /// The multiplexor.
    pub board: Chip,
    /// What each end was last given, per wire: the controller first.
    given: Vec<[Option<Level>; 2]>,
    /// Board transitions made, for measuring the cost of the board.
    pub transitions: u64,
}

impl Dm {
    /// A multiplexor on the cable of a controller parsed by
    /// [`crate::netlist::parse_with_multiplexor`] --- the one whose six
    /// one-board jumpers are left off, so that these nets are the
    /// multiplexor's to drive rather than tied to ground and to each other.
    pub fn new(dc: &Netlist, dm: &Netlist, powered_at: u64) -> Dm {
        let mut wires = Vec::new();
        for name in wire_names() {
            let controller =
                find(dc, &name).unwrap_or_else(|| panic!("the controller has no {name}"));
            let board = find(dm, &name).unwrap_or_else(|| panic!("the multiplexor has no {name}"));
            wires.push(Wire { controller, board });
        }
        let mut board = Chip::new_unclocked(dm);
        board.power_on();
        for w in &wires {
            board.pull_up(w.board);
        }
        board.settle_all();
        board.transition(powered_at);
        let given = vec![[None; 2]; wires.len()];
        Dm { wires, board, given, transitions: 0 }
    }

    /// Carries every wire across once: the wired-AND of what the controller
    /// and the multiplexor hold, given to both.  Returns whether anything
    /// moved.
    pub fn exchange(&mut self, controller: &mut Chip) -> bool {
        let mut moved = false;
        for (k, w) in self.wires.iter().enumerate() {
            let (mut low, mut high) = (false, false);
            for (level, strong) in
                [controller.board_level(w.controller), self.board.board_level(w.board)]
            {
                if strong {
                    match level {
                        Level::Low => low = true,
                        Level::High => high = true,
                        _ => {}
                    }
                }
            }
            let level = if low {
                Some(Level::Low)
            } else if high {
                Some(Level::High)
            } else {
                None
            };
            for (end, (chip, net)) in [(&mut *controller, w.controller), (&mut self.board, w.board)]
                .into_iter()
                .enumerate()
            {
                if self.given[k][end] != level {
                    match level {
                        Some(l) => chip.drive(net, l),
                        None => chip.pull_up(net),
                    }
                    self.given[k][end] = level;
                    moved = true;
                }
            }
        }
        if moved {
            self.board.settle();
        }
        moved
    }

    /// When the multiplexor next has an event of its own.
    pub fn next_tap(&self) -> Option<u64> {
        self.board.next_tap()
    }

    /// Every event of the board's due by `now`, each at its own time.
    pub fn transition_due(&mut self, now: u64) {
        let mut n = 0;
        while let Some(t) = self.board.next_tap()
            && t <= now
        {
            self.board.transition(t);
            self.transitions += 1;
            n += 1;
            assert!(n < 1_000_000, "the multiplexor's events never run out at {now}");
        }
    }
}
