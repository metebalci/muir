# `data/`

Small, committed fixtures, every one of them **derived** from MIT's own
files in `mit/`. Nothing here is MIT's own; `mit/README.md` is the inventory
of those. How far each file here is checked differs:

- **the seven netlists** are extracted by a script in `tools/` and held pin
  for pin to MIT's own wire list for the same board by `tests/*_netlist.rs`;
- **the three disk controller images** are assembled from MIT's source by
  `tools/newdsk-proms.sh`, by an assembler first held to MIT's surviving
  round trip on its sister program, and a test re-assembles them every run so
  they cannot go stale;
- **`trident-connectors.txt` and `trident-bus.txt`** are written by
  `tools/trident-tables.sh` and checked on every run: `tests/cadrdc_netlist.rs`
  derives their rows again from the netlist and MIT's wire list and fails if
  the committed file differs;
- **`cables.txt` and `busint-connectors.txt`** were read off MIT's wire lists
  by hand, once. They are the two things here that nothing checks.

Most are compiled in: `src/` reads the seven netlists, the three disk
controller images, `cables.txt` and `busint-connectors.txt` with
`include_str!`. The two Trident tables are the exception --- nothing reads
them, they are reference for whoever works on the cable seam, and `src/`
only cites them. The scripts that make any of this do *not* run as part of
the build; they are for changing or re-checking a file.

This file is the working guide to the netlists --- what a name in one means,
and how to find it on the drawing it came from.

## What is here

### Netlists

One file per board, chip level: every part, every pin, and the net each pin
is on. `src/netlist.rs` parses them; `Chip::new` turns one into the `chip`
engine's simulated board, so these files are executable data, not only
documentation. The `rtl` engine is hand-written and is *checked* against
them instead.

| File | Board | Pages | Records | Parts | Made by |
|---|---|---|---|---|---|
| `CADR.netlist` | the CADR processor, both sections: CADR and ICMEM | 97 | 1243 | 985 | `tools/cadr-netlist.sh` |
| `BUSINT.netlist` | LISPM bus interface, board type `LG684` | 35 | 313 | 176 | `tools/busint-netlist.sh` |
| `CADRM.netlist` | 64K-word memory, four banks of 4116 | 15 | 196 | 168 | `tools/cadrm-netlist.sh` |
| `CADRIO.netlist` | I/O board, fourteen pages, and the thirteen of the Chaosnet half that shares it | 27 | 308 | 173 | `tools/cadrio-netlist.sh` |
| `CADRDC.netlist` | disk controller, the pages `dc.book` names | 27 | 294 | 171 | `tools/cadrdc-netlist.sh` |
| `SIMPLETV.netlist` | SIMPLE TV display | 29 | 384 | 171 | `tools/simpletv-netlist.sh` |
| `LISPMTV.netlist` | LISPM TV, the four- and eight-bit display that replaced it | 25 | 265 | 172 | `tools/lispmtv-netlist.sh` |

A `part` line is one gate, not one device: a quad NAND is four lines under
the one designator, which is why the records run well ahead of the parts.
A part is what is mounted at a board location --- chips, oscillators, delay
lines, switches, LED digits --- and not the bypass capacitors, resistor
packs and busbars, which are on the sheets and in MIT's parts lists but are
nothing the engines compute. `tests/parts_mounted.rs` counts both, and holds
every board but the SIMPLE TV to MIT's own count of the same one: the parts
list `*.prt`, the DIP census `*.wls`, or both.

Each is read out of MIT's SUDS drawings in `mit/<board>/*.drw` by
`tools/soap4`, and then held to MIT's own wire list for the same board ---
the list it was wrapped from --- pin by pin, by `tests/*_netlist.rs`. Where
there is no wire list, as for the SIMPLE TV, the test says so and holds the
file to what there is instead.

### Assembled images, and tables read off the wire lists

| File | What it is |
|---|---|
| `newdsk-d03.prom`, `-d04`, `-d05` | the disk controller's control store: **one** 512 x 24 program in three 512 x 8 74S472s at `dcui` 0D03, 0D04 and 0D05, each holding one byte-slice of the same word (bits 0-7, 8-15, 16-23). Not three versions. Assembled from MIT's `mit/cadrdc/newdsk.31` by `tools/newdsk-proms.sh`, MIT's own assembler and cut having not survived the tapes |
| `cables.txt` | the five cables between the processor, memory and bus interface, pin by pin, read off the wire lists, with the connector-to-header pairing the boards were cabled with |
| `busint-connectors.txt` | every bus interface wire with a pin on the Unibus, Xbus or debug cable connectors, read off `mit/cadr1/busint.wlr` |
| `trident-connectors.txt` | the disk controller's two Trident connectors pin by pin, read off `mit/cadrdc/dc.wlr`: J01 the bus cable every drive on the string sees, J03 one drive's own radial cable. The body named on each board pin says which way the line runs |
| `trident-bus.txt` | the ten disk bus lines and what each of the three tags puts on them, read off `dcdbus.drw`, with Century Data's own name for the same cable line beside MIT's. The two number the bus in opposite directions --- see discrepancy 41 |

## The netlist format

It is the format the 2004 extraction --- Brad Parker's `soap.c`, which
`tools/soap4` follows --- used, and which `src/netlist.rs` already read, so
it is what `tools/soap4`'s `-o netlist` writes:

    page ALUC4

    part 2D21,7428
    (
    p4=OSEL0A
    p5=-IR12
    p6=-IRALU
    )

A `part` block is **one body --- a gate, not a package.** The 7428 at 2D21
has four gates, so `part 2D21,7428` appears four times, once per gate, each
with the pins of that gate. `p4=OSEL0A` reads *pin 4 is on the net named
`OSEL0A`*. The name after the comma is the SUDS **body def**, which is not
always the DIP type: variant suffixes exist (`74S04A`, `74S02O`), and
`16DUMMY` is a dummy body carrying wiring that belongs to no chip.

A page's `#` header block is the drawing's own: SUDS version, board type,
who drew it, and its title lines. `page` names are upper-cased; the drawing
file is the same name in lower case, `mit/cadr/aluc4.drw`. `ams/cadr4`'s
`doc/drwtools/` renders a `.drw` to an
XGP-look PNG without an emulator, and `render_latest.py` there does the
newest readable version of every page --- which is the picture to put next
to a `page` block when reading one against the other.

## Reading a drawing against the netlist

A net name in these files is **not** always the string on the drawing.
`tools/soap4` rewrites it, in this order (`fix_signal_name` and
`netlist_name` in `tools/soap4/soap4.c`):

1. **Comments are cut off.** A name is truncated at the first ASCII 23 or
   semicolon, which SUDS uses to start a comment. `DBGIN`'s
   `RESET;RESET TO BUSSES;RESET ARBITER` is the net `RESET`.
2. **Trailing spaces go.**
3. **The right-arrow character becomes `>`.** `WRITE DATA > UB` on the sheet
   is one name, with a real arrow in it on the drawing.
4. **A non-printable character left after all that is dropped**, and soap4
   says so on stderr. `cadrio/clk60h.drw` has one inside `POWER LINE ^`.
5. **`... L` becomes `-...`** --- the active-low convention. `IWRITED L` on
   the sheet is `-IWRITED` here. A name that was already negated loses the
   negation instead of gaining a second: `-FOO L` is `FOO`.
6. **`A=M` is written `AEQM`**, because `=` separates pin from net in this
   format and would not parse.
7. **A name holding a space or a parenthesis is single quoted**:
   `'LC BYTE MODE'`, `'XB EVEN PAR'`.

So to go the other way --- netlist name to sheet --- drop the quotes, and
read a leading `-` as a trailing ` L`.

### Nets the drawing leaves unnamed

An unlabelled wire has no name to carry, so it is named **after a pin at its
other end**: `@2C20,p13` is *the wire that reaches 2C20 pin 13*. The wire
between ALUC4 2C15 pin 3 and 2C20 pin 13 is therefore written `@2C20,p13`
where 2C15 touches it and `@2C15,p3` where 2C20 does --- one wire under two
names. This is the 2004 script's convention, kept so that the same file
reads either way; `src/netlist.rs` joins the halves back into one net when
it parses, and `tests/netlist.rs::anonymous_nets_are_joined_end_to_end`
checks that none is left reaching a single pin.

**`CADR.netlist` has 72 such names.** The 2004 file had 74, and the
difference is worth knowing if you are comparing against it:

- A wire of three or more pins gets one name per end there, and can get one
  name for several ends here.
- The four-pin wire on `OLORD2` --- 1A20 pin 1 on the 74LS14, and pins 9, 10
  and 12 of the `16DUMMY` at 1A19 --- is one net here and was two pins short
  there. MIT's own wire list settles it: `cadrwd/icmem3.wlr` line 1796 has
  the unnamed wire `%1A19-09` joining all four.

### Bodies that are not parts

Not everything drawn becomes a `part`. Dropped: a body with no reference
designator, a `COMMENT` body, `TABLE` and `BYPASS` bodies, and the
placeholder designator `0@00`. So a page can be in the file with nothing
under it --- 91 of `CADR.netlist`'s 97 pages carry parts.

### `CPINS` is present and empty

`tools/soap4` cannot read `cadr/cpins.drw`: it stops with
`unexpected point bits #o001000`, a point encoding it does not know. The
page marker is written anyway, with nothing under it, and **nothing is
lost**, because the drawing holds no part to lose: `cpins.drw` is connector
connections only --- a signal entering a connector
symbol, and nothing else. MIT's own files say the same twice over:
`cadrwd/cadr4.stf` stuffs nothing on the page, and of the 181 lines naming
`CPINS` in `cadrwd/cadr4.wlr` every one but the drawing's own header is of
kind `CON`.

That also makes the empty page consistent rather than exceptional: this
netlist has **no connector parts on any page.** A connector is where a net
leaves the board, and what records that is MIT's wire list, not the netlist.
So the marker's job is to give the wire list a page to place those connector
pins on, which is how `tests/netlist.rs` checks that the two boards are
joined the way their cables join them.

## Two more passes after soap4

The file is not soap4's output alone.

**Reconciliation against MIT's wire list**, by `examples/reconcile.rs`, run
by every generating script but `tools/cadrm-netlist.sh`. A wire the drawings label twice comes out of the
reader as two nets; a name spelt with and without a space, likewise. MIT's
list is the board as it was wrapped, so where a wire of the list falls on
more than one net of the file, every pin of it is put on one net under MIT's
name. The scripts print what they moved: for `CADR.netlist` it is two pins,
`-TPDONE`/`-TPW60`. The same program then makes the board's own departures
from its drawings --- MIT's change orders, and the drawings' errors --- each
announced in the file's banner and cited at `MOVES` in the source: the
SIMPLE TV's ECO 2, the bus interface's REQLM E09 (discrepancy 68), and the
I/O board's ECO 10, which wires the 2651's `-TxRDY` onto `-SER RRDY` so
that the serial port interrupts for output as well as input.

**Two rules applied at parse time**, in `src/netlist.rs`, so they are not
visible in the file:

- `EXPLICIT_ALIASES` joins nets the file has as two that are one wire:
  `-TPDONE`/`-TPW60`, and the two wires that change name at the connector
  between the CADR and ICMEM sections, `PROG.BUS.RESET`/`PROG.UNIBUS.RESET`
  and `-HALT`/`-FUNCT1`.
- `EXPLICIT_SPLITS` does the opposite for `-RESET`, which the file has as
  one net because both sections call it that, but which each section makes
  for itself with its own 74S37 --- CLOCKD 1B18 on the CADR section, OLORD2
  1A06 on ICMEM. Read as one, the two totem-pole outputs fought over it.

## Regenerating

Each script reads MIT's drawings in `mit/`, which are committed, so a fresh
clone can regenerate every one of these:

    tools/cadr-netlist.sh
    tools/busint-netlist.sh
    tools/cadrm-netlist.sh
    tools/cadrio-netlist.sh
    tools/cadrdc-netlist.sh
    tools/simpletv-netlist.sh
    tools/lispmtv-netlist.sh

They build `tools/soap4` themselves and read nothing outside this
repository. `cargo test` then holds the result to MIT's own wire lists in
`mit/`.

Two of the four tables are not made by a script: `cables.txt` and
`busint-connectors.txt` were read off the wire lists by hand, once, and
nothing checks them.

The two Trident files have a tool of their own:

    tools/trident-tables.sh

`muir::trident` derives their rows from `data/CADRDC.netlist` and MIT's
`mit/cadrdc/dc.wlr`; `examples/trident-tables.rs` writes them.
`tests/cadrdc_netlist.rs` derives the same rows and asserts the committed
files still match, so a re-extracted netlist cannot leave them behind
quietly. The prose header of each is written by hand and kept; only the rows
below it are derived, so only those are written and only those compared.

## Licence

Everything here is made from `mit/` by this project's tools, so two things
are in each file. The extraction --- the scripts in `tools/`, the readers and
assemblers in `examples/`, the form the files take --- is this project's,
under the repository's licence. What is extracted is MIT's: the netlists are
MIT's circuits read off MIT's drawings and wire lists, the three disk
controller images are MIT's microcode `mit/cadrdc/newdsk.31` in its
assembled form, and the two Trident tables, `cables.txt` and
`busint-connectors.txt` are what MIT's wire lists say. This project claims
nothing in that content, on the footing `mit/README.md` states for the files
it came from.
