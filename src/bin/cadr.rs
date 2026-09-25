// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `cadr`: MIT's CADR as built, on the engine the command line names.
//! [`muir::cli`] is the command line, shared with `quux`; this executable
//! is the machine, and takes only the CADR's flags.

fn main() {
    muir::cli::run(muir::machine::Geometry::CADR);
}
