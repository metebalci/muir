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
  3 or later.
- `unpack4.c` is `ams/cadr4`'s version of John Wilson's `unpack.c`, written
  in 1993 and separated from his `DUMP.C` in 1998, which reads ITS files
  stored in Alan Bawden's evacuated format; its header keeps John Wilson's.
  It came here the same way, under the AGPL through `ams/cadr4`.

**Neither original carries a licence**, and that was checked at the source
rather than assumed from the copies here. Both are published in
`lisper/cpus-cadr`, Brad Parker's own repository: `suds/soap.c` and
`suds/unpack.c` there give authorship and a description in their headers and
state no terms, and the repository holds no licence file. So for whatever
survives of the two originals in these files there is no grant to point at
--- not a permissive one, and not a refusal either. The terms were never
written down.

`ams/cadr4` released its versions under the AGPL and this repository carries
them on. That covers the work done in each --- the rewrite for cadr4, and
the changes listed in `soap4.c`'s own header --- and it cannot make a grant
for what came before it, which is why the copyright lines at the top of both
files name the original authors beside this project's. Anyone redistributing
these two files should know the position is unsettled rather than settled in
their favour. It is the one open licensing question in this repository.

Everything else in this directory is this project's, under the repository's
licence.
