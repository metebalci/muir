# QUUX

QUUX is the CADR evolved: the same processor, buses and boards, changed where
a change pays for itself. It is chosen with `--machine quux` in muir, and by
the same flag in muir-fpga and muir-sys; `--machine cadr`, the default, is
the CADR as MIT built it. Back to [the manual](manual.md).

This page says where QUUX differs from the CADR. Everything it does not
mention is the CADR's.

## What each difference reaches

A change to the hardware reaches further than the board: the boot PROM,
the microcode, the Lisp system in the band, and the tools that read a
machine's state can all depend on what changed. For each of QUUX's
differences, what it needed:

| Difference | Boot PROM | Microcode | Lisp system | Tools |
|---|---|---|---|---|
| Six-bit level-1 map entry | nothing: MIT's PROM boots it; version 1000 also clears QUUX's 64 blocks | 1000: the six-bit read, the two-deposit write, invalid block 77, the reverse first-level map moved to system communication area 640-737 and the swap-out CCWs to 440-457 | nothing: System 1001 runs unchanged | CC's remote debugger (`CADR-DEBUGGER`) still assumes the CADR's map |
| MACHINE-ID in functional source 16 | nothing | 1000 reads it at boot and runs as either machine | `PROCESSOR-TYPE-CODE` is 4 | nothing |
| The feature page | nothing | nothing: field widths are fixed when the microcode is assembled | does not read it yet | do not read it yet |
| `MUL` and `DIV` in one instruction | nothing | 1000 uses them in `MPY`, `DIV` and `BIDIV`'s quotient; the 31-step loops still step; `MULTIPLY` and `DIVIDE` named in `cadsym` | nothing | nothing |
| The processor tick | nothing | none enables it yet: the clock handler is still entered from the display's interrupt | nothing | nothing |
| MONO TV, the display | nothing | the clock from the tick, not the TV's interrupt; the run light's address in the buffer | the main screen's size from the feature page, not `shwarm.lisp`'s constants | the terminal, screenshots and captures show whichever screen is fitted |

## The map

**A level-1 entry is six bits, not five.** The level-1 map names, for each
8K-word region of virtual memory, a block of 32 level-2 entries. On the CADR
the entry is five bits, so there are 32 blocks, and the microcode keeps the
last one, 37 octal, as the invalid block: at most 31 regions are mapped at
once (`ADVANCE-SECOND-LEVEL-MAP-REUSE-POINTER` in System 100's
`sys/ucadr/uc-page-fault.lisp`). QUUX's six bits give 64 blocks, 2,048
level-2 entries, and 63 regions.

**The sixth bit travels on the two bits the CADR leaves spare.**

| | CADR | QUUX |
|---|---|---|
| Read back, `MAP(MD)` | entry in `<28:24>`, bit 29 always 0 --- VMEMDR 1A01 drives it from `HI12` through a 74S240 | entry in `<29:24>` |
| Written, a store to `WRITE-MAP` with `VMA<26>` | entry from `VMA<31:27>` (`mit/cadr/ir.bits`) | bits 4:0 from `VMA<31:27>`, bit 5 from `VMA<24>` |
| Level-2 entry chosen | `{entry, VMA<12:8>}`, 1,024 entries | the same, 2,048 entries |

Bits 31 and 30 of `MAP(MD)` are the write and read faults on both, and bit
24 of `VMA` reaches no map write on the CADR. A store that writes both levels
at once addresses level 2 with the level-1 bits zero on both machines
(`Machine::write_map` has why).

`tests/quux.rs` holds the six-bit entry written through `VMA<24>` and read
back in `MAP(MD)<29:24>`, and a translation through a block above 37, on
`micro` and `rtl`; the same store on the CADR keeps five bits and reads bit
29 as 0.

## How software tells the two apart

**QUUX answers who it is in functional source 16, its MACHINE-ID**, one
microinstruction,
no bus cycle:

| Bits | QUUX | CADR |
|---|---|---|
| 31:16 | signature `0x5155` | nothing drives the M bus: all ones |
| 15:4 | hardware revision: 4 --- 1 the six-bit map, 2 the 16K PDL buffer, 3 the multiply and divide, 4 the tick | |
| 3:0 | processor type: 4 | |

Source 16 is one MIT left unassigned: the 74S138 on page SOURCE that
decodes it has that output unconnected, and neither microcode 323 nor
microcode 1000 reads it. `IR<30>` is in no source decode, so source 36 is the
same. Source 17 is QUUX's tick (below), and open on the CADR. A machine is QUUX only if bits
31:16 hold the signature; the revision says which QUUX, each one containing
the last.

`tests/quux.rs` holds the word on both engines and the CADR's all ones;
`the_unassigned_sources_read_all_ones_on_the_board` in `tests/output_bus.rs`
holds the CADR's on the netlist.

**Unverified:** that a real CADR reads all ones there. The M bus has only
tri-state drivers and no pull-ups, so for an unassigned source it floats, and
TTL reading an open input as high is what the netlist model does and what
the parts usually do, not what a datasheet promises. The 16-bit signature is
what makes that safe: a floating bus would pass for QUUX once in 65,536 at
worst. A CADR reading source 16 would settle it.

## The PDL buffer

**QUUX's PDL buffer is 16K words**, its pointer and index 14 bits where the
CADR's are 10 (revision 2). They read back whole in functional sources 2 and
3, whose upper bits read 0 on the CADR. `quux_s_pdl_buffer_is_4k_or_16k` in
`tests/quux.rs` holds a push past word 1777 landing above it and the wrap at
the buffer's own size. It needs QUUX's boot PROM (below): MIT's stops copying
A memory in on the index wrapping at 2000 words. MIT's microcode 323 does not
run on it (`microcode_323_does_not_run_on_quux_revision_2`); microcode for
QUUX has to know the size.

## The feature page

**QUUX lists its sizes in one page of Xbus I/O space**, physical `17377000`
to `17377377` (page 36776), just below the page the display's control
registers and the disk controller share. It is read-only and read like any
device register, through the map:

| Word | QUUX, revision 4 |
|---|---|
| 0 | the MACHINE-ID, as source 16 gives it |
| 1 | level-1 entry: 6 bits |
| 2 | level-2 map: 2,048 entries |
| 3 | PDL buffer: 16,384 words |
| 4 | control store: 16,384 words |
| 5 | A memory: 1,024 words |
| 6 | dispatch memory: 2,048 words |
| 7 | multiply and divide: 3, bit 0 `MUL` and bit 1 `DIV` |
| 10 | the processor tick: 1 |
| 11 | the main screen: width in 31:16, height in 15:0 |
| 12 | the main screen: bits a pixel in 31:16, words a line in 15:0 |
| 13 | the main screen: its buffer's first physical address |
| 14-377 | 0 |

Words 11 to 13 describe whichever display is fitted: MONO TV's 1920 by 1080,
one bit a pixel, 60 words a line at `17000000`, or, on a QUUX run with a CADR
board, that board's 768 by 963, one bit, 24 words a line at the same address.

Nothing answers at that page on the CADR --- in muir's model of it the
display answers pages 36000-36177, 36400-36577 and 36777, the disk controller
36777, and nothing else in Xbus I/O space --- so a read there times out and
sets the Xbus NXM bit, as a read of any empty I/O address does. Software
reads source 16 first and the page only on QUUX, and so never waits for the
timeout. `quux_lists_its_sizes_in_its_feature_page` in `tests/quux.rs` holds
the page on QUUX and the timeout on the CADR, on both engines.

## Multiply and divide

**QUUX multiplies in one instruction and divides in one**, ALU functions 42
and 43 (revision 3). The CADR takes a step per bit: `MUS`, `DVS1`, `DVS` and
`DVREM` are ALU functions 40, 51, 41 and 45 (`mit/cadr/ir.bits`, ALU
FUNCTIONS), and microcode 323's `MPY` runs 32 multiply steps, its `DIV` a
first step, 31 steps, a last step and the remainder correction (System 1001's
`sys/ucadr/uc-arith.lisp`). On the CADR, 42 and 43 are no multiply or divide:
the 74S139 at SOURCE 3D04 that makes `-MUL` and `-DIV` from `IR<4:3>` has
outputs 2 and 3 unconnected, so they are the 74S181 functions their bits
select.

| | `MUL`, 42 | `DIV`, 43 |
|---|---|---|
| Is | 32 `MULTIPLY-STEP`s | `DIVIDE-FIRST-STEP`, then 31 `DIVIDE-STEP`s |
| M source | the high word to add into, usually 0 | the high dividend |
| A source | the multiplicand | the divisor |
| `Q` before | the multiplier | the low dividend |
| Output bus | the product's high word | the partial remainder |
| `Q` after | the product's low word | the quotient; `Q<31>` the first step's bit, set on overflow or a zero divisor |
| Time | one ordinary microcycle | held until 330 ns after it entered `IR` |

Each step's M operand is the output bus of the step before, as when the
microcode writes the step's result back to the same M location. Both
instructions decode on `IR<8>` and `IR<4:3>` only, as the '139 does, and
drive the output bus and load `Q` whatever the output selector `IR<13:12>` and
the Q control `IR<1:0>` say. `DIVIDE-LAST-STEP` and `DVREM` stay the CADR's
separate instructions.

**The divider's hold** is a `-WAIT` term of QUUX's own: the divider is busy
while a `DIV` stands in `IR`, not nopped, and 330 ns (`muldiv::DIV_NS`, 32
quotient bits and a load at 10 ns each) have not passed since the clock edge
that loaded `IR`. The master clock runs on, so the bus interface carries on,
and the microcycle starts at the first master clock edge after that: the
fewest whole generator cycles covering 330 ns: two at the boot's 220 ns, three
at the normal 145 and the fast 135. The time does not depend on the operands. A halt during the
hold stops the machine with the `DIV` still in `IR`. The hold does not stop a
single step, as `-WAIT` does not; by then the divider is done.

`tests/muldiv.rs` holds each against the step sequence on the CADR, for many
operands and every output selector and Q control, on `micro` and `rtl`, with
the step sequence itself held to the netlist; the CADR's 42 and 43 to the
netlist; the hold's length on both engines; a halted and single-stepped `DIV`;
and a checkpoint taken during one.

## The tick

**QUUX's processor has a tick of its own** (revision 4): a flag that rises
every period, part of the interrupt the microcode already tests. The CADR has
no clock in the processor. Its clock is the display board's vertical
interrupt: microcode 323's `INTRX0` (`sys/ucadr/uc-interrupt.lisp`) reads the
TV's mode register, clears its vertical flag, and runs the "roughly-60-cycle
clock" handler --- the mouse, the disk's idle time, the Chaosnet's
transmit-abort wakeup, and the sequence-break counter the scheduler runs
on.

| | |
|---|---|
| Functional destination 3 | control: `<0>` enable; a write with `<1>` set clears the flag |
| Functional destination 4 | the period in microseconds, `<23:0>`, 0 taken as 1; a write starts a period from then |
| Functional source 17 | `<0>` the flag, `<1>` the enable |
| Period at reset | 16,667 µs, 60 Hz |
| Interrupt | while enabled, the flag is ORed into the interrupt pending that jump conditions 5 and 6 test, so it costs nothing until it rises |

The flag rises a period after the tick is enabled or its period written, then
every period after, whether or not it was cleared between; a clear takes it
down until the next. `-RESET` turns the tick off and puts the period back.

On the CADR, destinations 3 to 7 have no output on the 74S138 that decodes
them and write only M, and source 17 has none either and reads all ones.
Neither microcode 323 nor QUUX's 1000 writes destinations 3 to 7 or reads
source 17, by a scan of every control-store word. **Unverified:** that no
instruction made at run time through `IMOD` does; running the band with the
tick decode trapping would settle it.

`tests/tick.rs` holds the period against a 100 and a 300 µs one, the clear,
the 60 Hz start, the interrupt condition taken with the tick on and not with
it off, the CADR's all ones, and a checkpoint taken in the middle of a
period, on `micro` and `rtl`.

## MONO TV, the display

**QUUX's display is MONO TV**, a monochrome frame buffer: 1920 by 1080 unless
`--mono-tv-size` gives another size, one bit a pixel. It is the frame buffer and one register, and nothing else: no sync
program, no color map, and no interrupt, the machine's clock being the
processor's tick. `--tv-board mono-tv`, which is QUUX's display unless another
board is named; refused on the CADR.

| | |
|---|---|
| Buffer | 64,800 words, physical `17000000`-`17176437`: 60 words a line, 1,080 lines |
| Pixel | pixel `x` of line `y` is bit `x mod 32` of word `60 y + x / 32`, the low bit leftmost, as on the CADR's TV |
| Mode register, `17377760` | bit 2, black-on-white, reads back; every other bit reads 0 and a write of it is dropped |
| Registers 1 to 7, `17377761`-`17377767` | answer, read 0, and take writes to no effect |
| Interrupt | none |

**Its size is muir's to choose**, `--mono-tv-size <width>x<height>`: the width
a multiple of 32, the buffer at most 130,560 words (up to the feature page at
`17377000`), and at most 65,536 with the color TV fitted, whose buffer starts
at `17200000`. 2560 by 1440 fits; 3840 by 2160 does not. The feature page's
words 11 to 13 give the size to the software. The table above is the default
size.

1920 bits a line is 60 whole words, which `BITBLT` needs of a screen array's
first dimension (`BITBLT-DECODE-ARRAY` in `sys/ucadr/uc-tv.lisp`). The buffer
starts where the CADR's does, so the band's `IO-SPACE-VIRTUAL-ADDRESS`
reaches it unchanged, and ends below the color TV's strap at `17200000`.

System 1001 runs on it but draws its screen wrong: `shwarm.lisp` makes the
main screen 768 by 963 at 24 words a line, and MONO TV scans 60, so each of
its lines is spread over parts of several. On QUUX's microcode 1000 with the
tick (`ref/ucode-1000-quux4`) the band reaches its listener on `micro` in
136 M microcycles, as on the CADR's board, measured by reading the rows the
listener draws in at 24 words a line. A band that sizes the main screen from
the feature page is what makes the picture right.

`tests/mono_tv.rs` holds the buffer's first and last words and the NXM past
it on both engines, the bus interface's decode of the whole buffer, the
pixel order and the terminal's frame, the register, the absence of an
interrupt over a second, and the feature page's three words.

## Its boot PROM

**QUUX boots from its own PROM, version 1000** (`data/quux-promh.mcr`),
MIT's version 9 changed so that a PDL buffer of any width from 1K up boots.
MIT's PROM loads A memory through the PDL buffer and stops copying it out
when the PDL index wraps to 0 (`FILL-A-LOOP` in `mit/sys/ucadr/promh.text`),
which on the CADR's 10-bit index is after 2000 words; on a wider index the
copy runs on and overwrites A memory with what lies above. Version 1000 stops
after 2000 words by count. The same PROM boots the CADR, whose index never
passes 1777. `--machine quux` loads it; the CADR keeps MIT's.

## Its microcode

**Microcode 1000 runs on both machines**, muir-sys's first change to MIT's
323, made from System 1001's sources. At boot it reads functional source 16.
Without the signature it is on a CADR: processor type 1, five-bit level-1
entries, invalid block 37. With it, the type is the word's bits 3:0, 4, and
from revision 1 on it reads the six-bit entry at `MAP(MD)<29:24>`, writes it
in two deposits, `VMA<31:27>` and `VMA<24>`, and keeps block 77 as the
invalid one. On both it keeps the reverse first-level map in system
communication area words 640 to 737 and the swap-out CCWs at 440 to 457. It
checks at boot that block 0's level-1 entry reads back as the invalid entry
the MACHINE-ID promised, and stops at `MAP-WIDTH-MISMATCH` if it does not.

`tests/system_1001.rs` holds it, with the microcode's files in the gitignored
`ref/ucode-1000`: System 1001, the band as released, reaches its listener on
QUUX on both engines with version 1000 and type 4, and on the CADR with
version 1000 and type 1 and no level-1 entry above block 37. The band reads
the error table for the running version at boot, `SYS: UBIN; UCADR TBL
1000`, so the served `sys/ubin/ucadr.tbl` must be microcode 1000's.

## What the microcode had to do differently

Microcode 323 knows the CADR's map, and runs on QUUX as on a CADR: System
1001 on it reaches its listener on both engines with no level-1 entry naming
a block above 37 (`system_1001_runs_on_quux_as_on_the_cadr` in
`tests/system_1001.rs`), so it uses 31 regions. Using the other 32 takes
microcode written for QUUX. From its sources (System 1001's
`sys/ucadr/`):

- `MAP-FIRST-LEVEL-MAP` and `MAP-WRITE-FIRST-LEVEL-MAP` are five-bit fields
  (`uc-page-fault.lisp`), and the write field cannot simply widen, `VMA<32>`
  not existing: the sixth bit is a second deposit, at `VMA<24>`.
- 37 octal is "no block" in three comparisons (`uc-page-fault.lisp`) and
  block 37 the invalid block, zeroed at boot (`uc-cold-disk.lisp`); on QUUX
  both are 77.
- The reverse first-level map has one word per block in the system
  communication area, words 440 to 477, and the keyboard buffer's header
  follows at 500: 64 blocks' worth does not fit there and has to move.

**Unverified:** that nothing else in the band assumes 32 blocks. What is
known is by reading the sources, and a band run on QUUX's own microcode would
settle it.

## Not modeled

`chip` is the CADR's boards, netlist for netlist, and QUUX has none: a run
that asks for both is refused.
