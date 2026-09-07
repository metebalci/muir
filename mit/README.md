# `mit/`

MIT's own files, committed here unmodified. No *file* here is generated,
edited or derived --- every one is byte for byte as it reached us: the
engineering files for the boards this project models are the AI Laboratory's,
written between 1977 and 1981, and `sys/` is a snapshot of System 100's own
tree. How they are foldered is a different question, answered below.

This is the primary source. `data/` holds what is made *from* it: the
netlists `tools/*-netlist.sh` extract, the disk controller's control store,
and the two cable tables.

## Why they are here

Everything `cargo test` and `tools/*-netlist.sh` read is in this repository.
The netlists in `data/` are committed and are what `cargo test` holds to the
wire lists here; the drawings are what `tools/*-netlist.sh` re-extracts them
from, when one needs changing or re-checking. Neither fetches anything.

They were recovered from the ITS backup tapes and reached this repository
through `ams/cadr4`'s tree, which carries them as the tapes had them; a
drawing and the wire list it is checked against reached us together. That is worth stating plainly,
because it means the check catches what it is built to catch --- the reader
misreading a drawing, the drawing and the wire list being different files made
by different people at MIT --- and does not witness the recovery of the tape
itself.

## What is here

| Directory | Board | Drawings | MIT's own second copy |
|---|---|---|---|
| `cadr/` | the CADR processor, both sections | 126 `.drw` | print sets `cadr.book`, `icmem.book`; `ir.bits`, MIT's microinstruction field diagram and register map |
| `cadrwd/` | --- | --- | the processor's wire lists: `cadr4.wlr`, `icmem3.wlr`, with `.stf` and `.wls` beside each |
| `cadr1/` | LISPM bus interface, and the CDC adaptor drawn beside it | 47 `.drw` | `busint.wlr`, `busint.wls`, `busint.stf`, print set `busint.book`; the board's two PROM images, `reqtim.prom` and `uprior.prom` |
| `cadrm/` | 64K-word memory | 29 `.drw` | `mem.wlr`, `mem.wls`, `mem.stf`, `mem.eco` |
| `cadrio/` | the I/O board | 21 `.drw` | `iob.wlr`, three `.wls`, five `.stf`, four `.eco` |
| `cadrdc/` | the disk controller | 49 `.drw` | `dc.wlr`, three `.wls`, four `.stf`, four `.eco`; also `newdsk.31`, its microcode source |
| `cadrtv/` | the two displays, in `simple-tv/` and `lispm-tv/` | 34 + 26 `.drw` | `lmtv4b.wlr`, `lmtv4b.wls`, `lmtv.order`; the sync programs `cpt.prom` and `vmi.prom` and the three clock images `lmprom.3`, `lmtv4b.prom`, `lmtv8b.prom` |
| `chaos/lispm/` | the Chaosnet half of the I/O board | 15 `.drw` | (held to `cadrio/iob.wlr`, the board they share) |
| `chaos/` | --- | --- | the Chaosnet's two PROM images with the tables they were burned from: `lmmodu.prom`, `.prom2`, `.promt`, `lmmynm.prom`, `.promt` |
| `lmdoc/` | --- | --- | `cadr.164`: **AI Memo 528**, *CADR*, by Knight, Moon, Holloway and Steele, in the 15 May 1980 printing. The machine described in MIT's own words, and the one place the mask memories' contents are written down |
| `sys/ucadr/` | --- | --- | `promh.text`, MIT's boot PROM source with their comments |
| `sys/ubin/` | --- | --- | `promh.mcr`, the boot PROM the engines run, and `promh.sym`, its symbol table; `ucadr.mcr`, microcode 323 itself, which is what `diskpack` puts on a pack |

**The top-level names are MIT's own**, the `AI:` directories these files were
dumped from --- `AI:CADR;` and its neighbours.

**The subfolders are not.** `cadrtv/simple-tv`, `cadrtv/lispm-tv` and
`chaos/lispm` file the `.drw` pages under the board whose title block names
them, because those two groups each hold more than one board *under the same
page names*. Only the drawings needed filing; the wire lists, ECOs, PROM
images and body libraries did not collide, so they stay at the group level
where the tape had them --- which is why the PROMs sit beside a folder rather
than inside it.

`chaos` is the bad case: five machines share that ITS directory and three of
them use the same page names. The Lisp Machine's `lm*` pages are the Chaosnet
half of the CADR I/O board, while a plain-named set is the PDP-10's interface,
whose `iobctl`, `iobtrm` and `iobxcv` look exactly like the I/O board's own
pages. **So never glob `chaos/*.drw` or `cadrtv/*.drw`**: it mixes machines
silently, and the netlist scripts name their pages one by one for this
reason.

**`sys/` is different: it is a snapshot of System 100's own `sys` tree**, at
the paths the release uses. `sys/ucadr/promh.text` is MIT's boot PROM source
and `sys/ubin/promh.mcr` its assembler's output, which is the PROM the engines
boot; `sys/ubin/ucadr.mcr` is microcode 323, the machine's own microcode, which
`diskpack` writes into the microload partition of a pack it makes. Each is
byte for byte the file in the release. System 100, microcode 323, is what this
project targets, so these are the target's own files rather than copies of
them from anywhere else, and `tests/prom.rs` and `tests/mcr.rs` hold each to
the release's copy whenever `vendor/` is there to compare against.

Only the CADR's own Chaosnet pages are copied: the CONS's, the CAIOS's and the
PDP-10's are other machines and are left on the tapes.

The three clock images are not interchangeable (discrepancy 40), and
`cadrtv/vmi.prom` differs from `cpt.prom` in exactly one word of 297, address
0o41 --- they are the sync program for two different monitors. `src/xbus.rs`
says which board loads which.

The PROM images are read from here rather than copied elsewhere:
`src/prom.rs` boots `sys/ubin/promh.mcr` directly and derives the burned form
itself, and `src/mcr.rs` carries `sys/ubin/ucadr.mcr` the same way, so that a
pack can be made with nothing fetched. The one thing made *from* this directory rather than read out of it is
the disk controller's control store, assembled from `cadrdc/newdsk.31` into
`data/newdsk-d0?.prom` because MIT's own assembler did not survive the
tapes.

## What the extensions are

A `.drw` is a SUDS binary, not text: `tools/soap4` reads one, and a renderer
that turns it into a picture without an emulator is what to put beside a
`page` block when reading one against the other.

The extensions are MIT's own, all of them. **Most of this directory is not
read by anything here** --- about 380 of the 682 files, two thirds of the
19 MB --- and is kept because it is the machine's own record and cheap to
carry. The table says which is which, so you can tell at a glance whether a
file is load-bearing or reference.

| Extension | What it is | Read here? |
|---|---|---|
| `.drw` | a page of a board, SUDS binary. `tools/soap4` reads one; `data/README.md` is the guide to going from a name in the netlist back to the wire on the sheet | **yes**, the pages each board's print set names |
| `.wlr` | the wire list the board was wrapped from, wire by wire | **yes** --- the second copy every netlist is held to, by `tests/*_netlist.rs` |
| `.wls` | the section census: how many bodies of each type the board carries. A cheap check that a page list is complete | **yes**, for the boards that have one |
| `.book` | the print set: the board's pages in MIT's own order. The netlist scripts take their page lists from these rather than globbing, because a directory holds more than one board | **yes** |
| `.stf` | the stuffing list, which slot holds which part | some |
| `.eco` | the engineering change orders, in order --- how to tell which revision a drawing is | some |
| `.prom`, `.mcr`, `.31`, `.39` | PROM images and microcode, MIT's own dumps and sources | some |
| `.wd` | per-page wire-wrap data, carrying the same parts and nets as the `.drw` beside it | no |
| `.bin` | a packed SUDS archive of a whole board's drawings --- the same pages as the `.drw` files, in a format `soap4` cannot read | no |
| `.ray`, `.uml`, `.augat`, `.aug`, `.wss` | wire-wrap production data: wire runs by pin, panel data, listings | no |
| `.txt`, `.fil`, `.prt`, `.memo`, `.hand` | board notes, page lists, parts lists | some, as reading rather than as input |
| `.164` | the CADR manual, an XGP typesetter source: markup lines beginning `.`, and text with control characters for the font changes. `LC_ALL=C grep` reads it | as reading, and `tests/chip.rs` carries one of its tables |

Two things in the "no" column are worth knowing about rather than forgetting.
The `.bin` archives are 6 MB of the 19, a second copy of drawings we already
hold as `.drw`; nothing is lost by ignoring them, and nothing would be lost by
deleting them either. And `.ray` looks like an independent description of the
same wiring a `.wlr` gives --- `busint.ray` lists wire runs by pin --- so if
it is, it would be a second route for the netlist checks. That is
**unverified**: nobody has read the format.

The unread `.drw` are the other boards sharing an ITS directory with one we
model: `cadr1`'s CDC adaptor, `cadrdc`'s DISK MULTIPLEXOR and MARKSMAN,
`cadrm`'s `pm*` and `mcp*` sheets, and `cadr`'s pages outside the two print
sets. The disk multiplexor is real hardware on the Trident string, so those
are not idle curiosities.

## Licence

These are MIT's files, and this project claims nothing in them: the copyright
on the rest of the repository does not extend to this directory, and no
licence of this project's choosing is put on it.

- The engineering files --- drawings, wire lists, ECOs, PROM images and the
  rest --- were written at the AI Laboratory between 1977 and 1981 and are
  the Laboratory's. They were recovered from the ITS backup tapes and reached
  this repository through `ams/cadr4`'s tree, unmodified. No statement of
  their terms came with them, and none is made up here: they are carried as
  the record of the machine they describe.
- `sys/` is a snapshot of the System 100 release, whose own README puts
  everything in it under the GNU Affero General Public License, version 3 or
  later. The files here carry those terms, as the release states them.
