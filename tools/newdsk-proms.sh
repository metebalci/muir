#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mete Balci
# SPDX-License-Identifier: AGPL-3.0-or-later
# Assemble the disk controller's microcode and write its three PROM images.
#
# The DC board is microprogrammed: three 512x8 74S472s at `dcui` 0D03, 0D04
# and 0D05 are a 512x24 control store, and `cadrdc/newdsk.31` is the program
# in them. MIT assembled it with `MICRO 52` on the PDP-10 and cut the images
# with `cadrdc/newdsk.trans`; neither the assembler nor its output survived
# the tapes --- they lived in Moon's directory, which was not dumped --- so
# `src/dcmicro.rs` is the assembler and `examples/dcmicro.rs` the cut.
#
# The check that they are right is the sister program, whose whole round
# trip did survive: `mksman.39` -> `mksman.mcr` -> `mksman.d03/d04/d05`, all
# MIT's, all of 25 October 1979. `tests/dcmicro.rs` holds the assembler to
# every word of the listing and every byte of the images before believing it
# here.
#
# usage: tools/newdsk-proms.sh
set -eu
muir=$(cd "$(dirname "$0")/.." && pwd)
mit=$muir/mit
src=$mit/cadrdc/newdsk.31
[ -f "$src" ] || { echo "no $src" >&2; exit 1; }
(cd "$muir" && cargo run --release --quiet --example dcmicro -- "$src" newdsk)
