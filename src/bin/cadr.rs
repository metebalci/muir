// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `cadr`: MIT's CADR as built, on the engine the command line names.
//! [`muir::cli`] is the command line, shared with `quux`; this executable
//! is the machine, and takes only the CADR's flags.
//!
//! The netlists `--chip` builds the boards from are embedded here rather
//! than in the library, which has to build without them: the scripts that
//! make `data/*.netlist` link it ([`muir::cli::Netlists`]).

use muir::cli::Netlists;

const NETLISTS: Netlists = Netlists {
    cadr: include_str!("../../data/CADR.netlist"),
    busint: include_str!("../../data/BUSINT.netlist"),
    cadrm: include_str!("../../data/CADRM.netlist"),
    cadrio: include_str!("../../data/CADRIO.netlist"),
    simpletv: include_str!("../../data/SIMPLETV.netlist"),
    lispmtv: include_str!("../../data/LISPMTV.netlist"),
    cadrdc: include_str!("../../data/CADRDC.netlist"),
    dm: include_str!("../../data/DM.netlist"),
};

fn main() {
    muir::cli::run(muir::machine::Geometry::CADR, Some(&NETLISTS));
}
