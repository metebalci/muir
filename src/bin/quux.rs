// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `quux`: QUUX, the CADR evolved, on the engine the command line names.
//! [`muir::cli`] is the command line, shared with `cadr`; this executable
//! is the machine, and takes only QUUX's flags.

fn main() {
    muir::cli::run(muir::machine::Geometry::QUUX);
}
