#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mete Balci
# SPDX-License-Identifier: AGPL-3.0-or-later
# Regenerate the disk controller's two cable tables in data/.
#
# `data/trident-connectors.txt` and `data/trident-bus.txt` are reference for
# the cable seam, derived from `data/CADRDC.netlist` and MIT's own wire list
# `mit/cadrdc/dc.wlr`. Neither is read by the simulator, and both go stale if
# the netlist is re-extracted or the wire list is re-read, so
# `tests/cadrdc_netlist.rs` holds them to what the board says now and this is
# what to run when it says they are stale.
#
# The prose header of each file is written by hand and kept.
#
# usage: tools/trident-tables.sh
set -eu
muir=$(cd "$(dirname "$0")/.." && pwd)
(cd "$muir" && cargo run --release --quiet --example trident-tables)
