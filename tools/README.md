# `tools/`

Scripts that make the committed fixtures in `data/` from MIT's files in
`mit/`, and one that fetches what is not committed. None of them runs as part
of the build; `data/README.md` says which test holds each output to its
source.

| Script | Makes |
|---|---|
| `fetch-system-100.sh` | `vendor/`: the System 100 release, and a directory for muir to serve it from. Nothing in `vendor/` is committed |
| `cadr-netlist.sh`, `busint-netlist.sh`, `cadrm-netlist.sh`, `cadrio-netlist.sh`, `cadrdc-netlist.sh`, `simpletv-netlist.sh`, `lispmtv-netlist.sh` | `data/<BOARD>.netlist`, one board each: MIT's drawings read with `soap4`, then reconciled with MIT's wire list by `examples/reconcile.rs` |
| `newdsk-proms.sh` | `data/newdsk-d0?.prom`, the disk controller's control store, assembled from `mit/cadrdc/newdsk.31` by `examples/dcmicro.rs` |
| `trident-tables.sh` | `data/trident-connectors.txt` and `data/trident-bus.txt`, by `examples/trident-tables.rs` |

## `soap4/`

The reader for MIT's drawings, which are SUDS binaries. It is C, built by the
netlist scripts as they run, and it is the one part of this repository that
is not this project's own work throughout:

- `soap4.c` was written by Mete Balci in 2026 for `ams/cadr4`, following Brad
  Parker's `soap.c` of October 2004, whose header it keeps. It came here from
  `ams/cadr4`, which is under the GNU Affero General Public License, version
  3 or later. The terms Brad Parker released the original under are not
  stated in the copy that reached this repository.
- `unpack4.c` is `ams/cadr4`'s version of John Wilson's `unpack.c` of 1993,
  which reads ITS files stored in Alan Bawden's evacuated format; its header
  keeps John Wilson's. The same holds: it came under the AGPL through
  `ams/cadr4`, and the terms of the 1993 original are not stated in the copy
  that reached this repository.

The copyright lines at the top of the two files name the original authors
beside this project's, and the changes made here are listed in `soap4.c`'s
own header. Everything else in this directory is this project's, under the
repository's licence.
