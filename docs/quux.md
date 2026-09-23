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
| Six-bit level-1 map entry | nothing: MIT's PROM boots it | 1000: the six-bit read, the two-deposit write, invalid block 77, the reverse first-level map moved to system communication area 640-737 and the swap-out CCWs to 440-457 | nothing: System 1001 runs unchanged | CC's remote debugger (`CADR-DEBUGGER`) still assumes the CADR's map |
| MACHINE-ID in functional source 16 | nothing | 1000 reads it at boot and runs as either machine | `PROCESSOR-TYPE-CODE` is 4 | nothing |
| The feature page | nothing | nothing: field widths are fixed when the microcode is assembled | does not read it yet | do not read it yet |

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
| 15:4 | hardware revision: 1, the six-bit map | |
| 3:0 | processor type: 4 | |

Source 16 is one MIT left unassigned: the 74S138 on page SOURCE that
decodes it has that output unconnected, and neither microcode 323 nor
microcode 1000 reads it. `IR<30>` is in no source decode, so source 36 is the
same. Source 17 is left open on both machines. A machine is QUUX only if bits
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

## The feature page

**QUUX lists its sizes in one page of Xbus I/O space**, physical `17377000`
to `17377377` (page 36776), just below the page the display's control
registers and the disk controller share. It is read-only and read like any
device register, through the map:

| Word | QUUX, revision 1 |
|---|---|
| 0 | the MACHINE-ID, as source 16 gives it |
| 1 | level-1 entry: 6 bits |
| 2 | level-2 map: 2,048 entries |
| 3 | PDL buffer: 1,024 words |
| 4 | control store: 16,384 words |
| 5 | A memory: 1,024 words |
| 6 | dispatch memory: 2,048 words |
| 7-377 | 0 |

Nothing answers at that page on the CADR --- in muir's model of it the
display answers pages 36000-36177, 36400-36577 and 36777, the disk controller
36777, and nothing else in Xbus I/O space --- so a read there times out and
sets the Xbus NXM bit, as a read of any empty I/O address does. Software
reads source 16 first and the page only on QUUX, and so never waits for the
timeout. `quux_lists_its_sizes_in_its_feature_page` in `tests/quux.rs` holds
the page on QUUX and the timeout on the CADR, on both engines.

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
