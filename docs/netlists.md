# Where the netlists come from

Part of [the muir manual](manual.md).

## What a netlist means here

A netlist is the board itself: every part, every pin, every wire between
them. These are not drawn by hand and not transcribed from scans. They are
extracted from MIT's own SUDS drawings --- the CAD files the machine was
designed in --- and then held, pin by pin, to MIT's own wire list for the
same board. Where the two disagree, a test fails. Six of the eight boards
are checked that way; two have no wire list on the tapes and are held to
MIT's specification instead, which is a weaker thing. [What each board is
checked against](#what-each-board-is-checked-against) below says
which board is checked which way, and the sections before it set out the
whole chain, file by file.

Underneath, a part is a behavior checked against its datasheet, nets resolve
four-valued so that an open-collector bus works the way it does on the
board, and the parts are evaluated in level order, with only the ones
something moved under recomputed.

A part is one device at one board location --- the chips, the oscillators,
the delay lines, the switches, the LED digits --- and not the bypass
capacitors, resistor packs and busbars beside them, which nothing here
computes. The files list gates rather than devices and run longer than the
boards do: the processor's 985 parts arrive as 1,243 `part` records, a quad
NAND being drawn four times under the one designator. A whole machine is
2,016 parts with one memory board on the Xbus and both display boards, the
SIMPLE TV and the color TV, and 7,224 with all thirty-two memory boards.

Six of the seven files are one card each --- the same card, six rows of
thirty positions, with 176 to 180 of them filled, which is why those boards
land within a few parts of each other --- and the processor and the control
store are the two large boards, their designators running in four sections
and in two. MIT counted these boards twice itself, by the locations it
stuffed and by the packages each part type takes, and every board but the
SIMPLE TV has one of those counts or both to answer to.

> The Chaosnet interface shares the I/O board rather than having one of
> its own, and MIT drew the two separately: fourteen pages for the board
> proper and thirteen for Chaosnet. All twenty-seven are in the netlist,
> 94 parts against 84, five chips having gates on both sides. The
> interface runs: it knows its own address off the switches, a packet
> written into its transmitter comes back out of its receiver with the
> check word right, and it trades packets with the other stations on its
> cable, taking broadcasts and passing over other hosts'. Those stations
> are off the host: `--chaos-udp` is the cable, putting the board on a
> network as Chaosnet over UDP while `--chaos-address` sets its switches,
> and a band's file and time host --- what MIT's associated machine, OZ,
> ran --- is a program of its own there,
> [ozd](https://github.com/metebalci/ozd). A CADR had no such server
> inside it, and neither has muir.

The boards this program runs are not written here. They are MIT's own
engineering files, recovered from the ITS backup tapes, extracted by a
script and then held to a second file MIT made of the same board. That chain
is the whole reason a netlist here can be called checkable rather than
plausible, and it is worth setting out, because the interesting part is not
the extraction --- it is what the extraction is answerable to.

The files are in `mit/` in the repository, committed unmodified and byte for
byte as they arrived: 682 of them, 20 MB, under the ten directory names they
were dumped from, which are MIT's own. About 380 are read by nothing here
--- two thirds of the bytes --- and are carried because they are the
machine's own record.

## What MIT left

MIT drew the CADR in SUDS, and their tooling made several other files from
the same drawings. Those are the second copies, and the reason they are
worth having is that each was made by a different program for a different
purpose, so a mistake in reading a drawing does not reproduce itself in
them.

| Extension | Files | What it is, and what it settles |
|---|---|---|
| .drw | 351 | One page of one board, as a SUDS binary rather than a picture. Every netlist here begins as these. **A drawing is the board as designed.** |
| .wlr | 9 | The wire list the board was wrapped from, wire by wire, with a direction on every pin. **This is the board as built**, and where it and the drawing disagree it is the wire list that is right and the difference that has to be named. |
| .wls | 11 | The section census: for each part type, the gates used, the packages they take, the spares left over and the current they draw. It says nothing about where any of them is, which is what makes it a cheap and independent check that a page list is complete rather than nearly so. |
| .stf | 15 | The stuffing list: every body, the card location it sits at, and the page it is drawn on. |
| .prt | 4 | The parts list: each part type, how many of it, and every location that took one. |
| .book | 6 | The print set --- the board's pages in MIT's own order, and where an extraction script takes its page list from for the boards that have one. |
| .eco | 13 | The engineering change orders in order, which is how to tell which revision a drawing is and whether a wire list is before or after a change. |
| .ray | 7 | Wire-wrap production data, and usually a *different dated revision* of the same board from the `.wlr` beside it --- so the two together date a change rather than merely confirming each other. |

> The rest --- `.wd`, `.bin`, `.uml`, `.wss`, `.augat` and the board notes
> --- are per-page wire-wrap data, a packed archive of drawings already
> held as `.drw`, and production listings. Nothing here reads them. Beside
> the boards sit MIT's own PROM images and microcode sources, and AI Memo
> 528, the CADR manual, in the 15 May 1980 printing.

## How a board becomes a netlist

Four steps, and the last is the one that matters.

**The drawings are read a page at a time.** `tools/soap4` --- the SUDS
reader from `ams/cadr4`, with fixes made here --- turns one `.drw` into
parts and nets. The pages a board gets are the ones MIT's own list for it
names, one by one, never a directory glob: its print set where there is one,
else the page list or stuffing list beside it. `cadrtv/` holds two displays
and `chaos/` five machines, several under the same page names, so a glob
mixes boards silently.

**The pages are joined.** A net the drawing left unnamed comes out of
`soap4` as `net_7`, numbered from zero on every page, so the same name means
different wires on different sheets; each is suffixed with its page before
the pages are put together.

**The wires MIT labeled twice are put back together.** A signal drawn on two
sheets under two labels arrives as two nets. `examples/reconcile.rs` reads
the board's wire list, finds the pins MIT wired to one wire, and puts them
on one net under MIT's own name for it. The wire list is the authority for
that, not the drawing.

**Then the result is held to the wire list, pin for pin.** That is
`tests/*_netlist.rs`, and it runs on every `cargo test`: each signal in
MIT's list has to reach exactly the pins the netlist gives it and no others.
Where a board has a census, the part types are counted against that too.

```text
the pages dc.book names  --soap4-->  parts and nets a page
                                            |
                                          joined
                                            |
                     mit/cadrdc/dc.wlr --reconcile--> data/CADRDC.netlist
                                                            |
                     mit/cadrdc/dc.wlr, dc.wls ----test----> pass or fail
```

The netlists are committed, so nothing has to be extracted to run the
program or the tests; the scripts are there for when a board needs changing
or re-checking. Nothing fetches anything.

**What this catches, and what it does not.** It catches a drawing misread,
and it catches the drawing and the wire list disagreeing --- different
files, made by different people at MIT, for different jobs. It does not
witness the recovery of the tape: a drawing and the wire list it is checked
against reached this project together, through the same tree, so the check
is of the two against each other and not of either against the machine that
is gone.

## What each board is checked against

**Not every board has the full set, and that is the honest half.** Six of
the eight are held to MIT's own wiring. Two are not, because no wire list
for them is on the tapes --- for those, MIT's *specification* stands in for
MIT's *wiring*, which is a weaker thing and is worth saying rather than
glossing.

| Netlist | Drawings | Wire list | And also |
|---|---|---|---|
| CADR.netlist | `cadr/` | `cadr4.wlr`, `icmem3.wlr` --- two, the processor and the control store being two boards in one file | reconciled against each in turn |
| BUSINT.netlist | `cadr1/` | `busint.wlr` | `busint.wls`, the census |
| CADRM.netlist | `cadrm/` | `mem.wlr`, with ECO 2 applied to the list, the drawings being after it | --- |
| CADRIO.netlist | `cadrio/`, `chaos/lispm/` | `iob.wlr` --- one list for both halves, the Chaosnet sharing the board | `iob.stf` for the page list |
| CADRDC.netlist | `cadrdc/` | `dc.wlr` | `dc.wls`, the census |
| LISPMTV.netlist | `cadrtv/lispm-tv/` | `lmtv4b.wlr` | `lmtv4b.wls`, the census |
| SIMPLETV.netlist | `cadrtv/simple-tv/` | **none on the tapes** | `lmtv.stf`, MIT's page list, and `lmtv.order`, MIT's register map, which the drawings have to decode the way it says |
| DM.netlist | `cadrdc/` | **none on the tapes** | `dm.stf`, the stuffing list |

> The multiplexor's missing wire list is an absence that was looked for
> rather than assumed: `cadrdc/` holds `dc.wlr` and `mk.wlr` for the two
> boards drawn beside it and no `dm.wlr`, and the tapes were searched for
> one. Its `dm.wls` stops before the census, so the check that would have
> counted its parts is not available either, and its netlist rests on
> `dm.stf` and the drawings. The SIMPLE TV is the same shape for a
> different reason: `cadrtv/` holds two displays under six shared page
> names, and only the later one's wire list survived.

## Every board a netlist, at once

**Every one of those boards ran a cold boot of System 100, all of them
netlists at once.** On 12 September 2026 a `chip` run with the processor,
the interface, main memory, the I/O board, the display and MIT's disk
controller all as netlists took the release from the pack and brought the
system up: the boot PROM read microcode 323 off the drive, the cold boot
copied all 21,342 pages of the band into the paging partition, and the
system swapped itself in and painted its who-line --- `Lisp Listener 1`, and
`Cold-booted` beside it, which is the running system's own report of where
it came from. It took 2 days 14 hours and 301 million microcycles, 51 of
those hours the copy, because the netlist disk controller spends the drive's
real milliseconds on every block where its behavioral model spends none.

That is what the netlists are for. A board held pin for pin to MIT's wire
list is a claim about the wiring; a board that carries the machine's own
software, through its own microcode, at gate level, for two and a half days
without a wrong answer, is that claim tested. The one board not in the run
is the DISK MULTIPLEXOR, which is the one board with no wire list on the
tapes to hold it to.
