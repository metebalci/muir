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
| Block-disk | needed: address the disk by block number | needed: the disk routines by block number, no cylinder, head or sector | needed: `io/disk.lisp` and the label editor by block number | `--disk-controller block-disk` |
| The memory cache (`--cache`) | nothing | nothing | nothing | `--cache`; the profile harness's `MUIR_CACHE` |
| The synchronous microcycle (`sync`) | nothing | nothing | nothing | `--timing-model sync`, `--sync-cycle-ticks` |
| No speed bits | nothing | the mode register write at boot need not set them | nothing | nothing |
| MONO TV, the display | nothing | 1000 for revision 4 (System 1002's): the run light in MONO TV's buffer, no TV vertical flag | System 1002 sizes the main screen from the feature page | the terminal, screenshots and captures show whichever screen is fitted |

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

Words 11 to 13 describe whichever display is fitted: MONO TV's 1280 by 1024,
one bit a pixel, 40 words a line at `17000000`, or, on a QUUX run with a CADR
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
fewest whole generator cycles covering 330 ns: three of QUUX's 145 ns. The time does not depend on the operands. A halt during the
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
Microcode 323 writes destinations 3 to 7 and reads source 17 nowhere, by
a scan of every control-store word, and running shows the same of what
the OA registers make at run time: `tests/unused_codes.rs` reads every
executed microinstruction as it stood in `IR` through a boot to the
listener. System 1001 on 323, on the CADR, runs none; System 1002 on its
microcode for revision 4 runs them at three addresses, each a
control-store word that carries them, the tick's own.

`tests/tick.rs` holds the period against a 100 and a 300 µs one, the clear,
the 60 Hz start, the interrupt condition taken with the tick on and not with
it off, the CADR's all ones, and a checkpoint taken in the middle of a
period, on `micro` and `rtl`.

## One rate

**QUUX has no speed bits.** On the CADR the mode register's `SPEED1` and
`SPEED0` choose the delay-line tap that ends the read phase, from extra slow
to fast (`mit/cadr/ir.bits`: "00 Extra slow, 01 Slow, 10 Normal, 11 Fast"),
and reset leaves them at extra slow, so the CADR boots at 220 ns a
microcycle until its microcode asks for normal. QUUX's mode register has no
such bits: a write of bits 1 and 0 goes nowhere, and every microcycle is the
same length from the boot on --- for now the CADR's normal, 145 ns, a
normal read tap and the 60 ns restart. The register's other bits are
unchanged. `quux_has_no_speed_bits` in `tests/quux.rs` holds it on both
engines.

## The synchronous microcycle

**Under `--timing-model sync`, a QUUX microcycle is a fixed number of 10 ns
ticks**, `--sync-cycle-ticks`, in place of the CADR's delay-line taps. On
the CADR a microcycle is the read tap and a 60 ns restart, 145 ns at normal
speed, which muir-fpga's fabric replays as 15 ticks. The ticks are a
board's: the number its fit proves its longest path settles in. muir-fpga's
routed fits put that path, from MD or `MEMSTART` through the map and the M
bus to the dispatch address and the next PC, at 37.35 ns on the Arty Z7-20
and 20.2 ns on the DE25-Nano; the default, 4 ticks, is the Arty's.
**Unverified** until a fit at that deadline meets it.

Only the length of a microcycle changes. Every register is clocked at the
one edge and the late writes land one edge later, as before; the bus keeps
its own time on the grid; a held cycle is one microcycle long; and the
divider's 330 ns hold is the fewest microcycles that cover it. The speed
synchronizer's instant 60 ns into a cycle goes: QUUX has no speed bits, and
a write lands at the edge. An `ILONG` instruction takes `ilong_ticks` more,
0 unless the library says otherwise.

System 1002 reaches its listener in the same 11 M microcycles under `sync`
of 4 ticks as at QUUX's one rate, in 0.95 s of the machine's time against
2.02 s: 2.1 times faster, the bus keeping its own time
(`system_1002_runs_under_sync` in `tests/system_1002.rs`).
`tests/sync_timing.rs` holds the microcycle's length, `ILONG`'s ticks, the
grid, the divider's hold and a checkpoint.

## The memory cache

**QUUX can have a memory cache** (`--cache <words>`, H2): unified and
write-through, in front of main memory, by physical address after the map.
Xbus I/O space and the Unibus are not cached. In muir it is `rtl`'s, and
it holds tags only: `rtl` takes a word from main memory as a cycle ends,
and a write-through cache never holds a word memory does not, so the cache
changes when a cycle is answered and never what it reads.

| | |
|---|---|
| A read that hits | acknowledged `hit_ns` after the request (20 ns, two ticks of the grid, **unverified** until a fit), with no bus cycle and none of the bus's setup, deskew or release |
| A read that misses | the memory board's cycle, as without the cache; it fills the line, the set's least recently used line going |
| A write | allocates nothing. With the write buffer it is acknowledged after `hit_ns`, or when the buffer's last write is done, and the board runs it behind the processor; the word is memory's from the acknowledgement, as a read after it finds it |
| Shape | lines of 4 words, 2-way, `<words>` in all, a power of two |
| Coherence | a disk transfer writes main memory behind the processor, and invalidates the whole cache before the next cycle |

Behind the cache is the CADR's memory board, eleven stages of its 24 MHz
chain, 458 ns, from the request to `XACK` (`MEMORY_CYCLE_STAGES` in
`src/busint.rs`), unless QUUX's own memory timing is fitted
(`MemoryTiming`, `Rtl::set_memory_timing`): a read, which behind the cache
is a line fill, and a write each answered a fixed time after the request,
off the Xbus, one at a time. muir-fpga measured its two boards, a 1,000,000
word array loop on System 1001 for 300 s:

| | Read, average | Write | In muir |
|---|---|---|---|
| Arty Z7-20 | 20.68 ticks of 10 ns (19 to 88) | 12 | read 220 ns, write 120 ns |
| DE25-Nano | 36.25 (33 to 228) | 29 (28 to 58) | read 380 ns, write 290 ns |

rounded up to the tick, with a tick more on a read for a line fill of four
words, two 64-bit beats: the machine has only ever made single-word
accesses, so a fill's time is **unverified**.

Measured with the profile harness (`examples/profile.rs`, `MUIR_CACHE` and
`MUIR_SYNC_TICKS`) on `rtl`, System 1001's band on QUUX's microcode 1000
for revision 4, over ten workloads (compile, two call-heavy, cons, the
multiply-and-divide, float, array, sort, bignum, intern), in the machine's
time:

| | Total | |
|---|---|---|
| QUUX's one rate, 145 ns | 72.7 s | 1.00 |
| the same, cache of 4K words | 58.6 s | 1.24 |
| `sync` of 4 ticks, no cache | 33.7 s | 2.15 |
| `sync`, cache of 1K words | 19.6 s | 3.70 |
| `sync`, cache of 4K words, no write buffer | 18.9 s | 3.84 |
| `sync`, cache of 16K words, no write buffer | 18.6 s | 3.90 |
| `sync`, cache of 4K words and the write buffer | 17.7 s | 4.11 |
| `sync`, the Arty's memory, no cache | 21.9 s | 3.32 |
| `sync`, the Arty's memory, cache of 4K words and the buffer | 15.5 s | 4.70 |
| `sync`, the DE25's memory, no cache | 26.4 s | 2.75 |
| `sync`, the DE25's memory, cache of 4K words and the buffer | 16.2 s | 4.48 |

Read hits run from 75 to 99 per cent of reads at 1K words and from 81 to 99
at 4K; past 4K words little more is gained. The cons workload is written
far more than read --- over four in five of its memory cycles are writes
--- and the write buffer is most of what the cache does for it.

`tests/cache.rs` holds the lines and the replacement, a read loop and a
write-and-read-back loop leaving the same words with the cache as without
and sooner, the invalidation, and a checkpoint.

## Block-disk

**QUUX's disk is block-disk** (`--disk-controller block-disk`): the CADR
disk controller's programming interface with the drive's geometry taken
out. Blocks are numbered from the start of the pack, and each is 256 words,
a page.

| | CADR controller | block-disk |
|---|---|---|
| Registers, `17377774`-`17377777` | status and command, command list pointer, disk address, START | the same |
| Command list | one word a block, `<23:8>` the page's physical address, `<0>` More | the same |
| Disk address | cylinder `<27:16>`, head `<15:8>`, sector `<7:0>`, unit `<30:28>` | the block number, `<27:0>`; one pack, unit 0 |
| Commands | read, read compare, write, read all, write all, seek, at ease, recalibrate, offset clear, reset | read, 0, and write, 11; any other stops by error |
| Status | not active, attention, errors of the drive, the ECC and the transfer | `<0>` not active, `<3>` interrupt request, `<9>` no pack, `<13>` stopped by error, `<17>` past the end of the pack, `<20>` NXM |
| After a transfer | the disk address at the last block moved, or the one that failed | the same |
| Interrupt | done, command `<11>`; attention, `<10>` | done, command `<11>` |
| Time | seeks and rotation, when timed | 100 us a block moved, **unverified**: an estimate until muir-fpga measures its disk path |

The words move inside the store to START, and the controller stays busy for
a block's time each; a transfer writes main memory behind the processor, so
the memory cache is invalidated. The pack image is the CADR's file, its
blocks in the same order, and the label's format is unchanged, its partition
starts and lengths already block numbers. `tests/block_disk.rs` holds the
read, the write, the end of the pack, the NXM, a command it does not do, the
registers on QUUX's bus and a checkpoint.

Nothing runs on it yet: the boot PROM, the microcode and Lisp address the
disk by cylinder, head and sector from the label's geometry, and a QUUX
boot PROM, microcode and band that address it by block are muir-sys's to
make.

## The single-edge contract

**What a single-edge core must keep**, for H1b: in which microcycle each
resource's new value is seen, counted from the microcycle whose
instruction produced it (cycle *n*). It is `rtl`'s read phase, write
phase and edge, which `tests/cosim.rs`, `tests/chip.rs` and
`tests/dispatch_write_order.rs` hold to the netlist; an FPGA core that
keeps every row, and stalls where a row needs it, runs the microcode
unchanged.

| Resource | Written | Seen by |
|---|---|---|
| `IR` | at the edge ending *n*, from the I bus: the control store at `PC`, the boot PROM, the debug IR, or `IWR` in the cycle after `WRITE-I-MEM` | the instruction executed in *n*+1. The instruction at a jump's target runs in *n*+2; the one after the jump runs in *n*+1 unless `N` inhibits it |
| `PC`, `LC`, `Q`, `VMA`, `MD` (from the processor), `INTERRUPT-CONTROL`, the PDL pointer and index, the SPC pointer, the flags | at the edge ending *n* | *n*+1 |
| A memory, M memory | in *n*+1's write pulse, from `WADR` and `L` registered at the edge ending *n* | *n*+1, through the pass-around (ACTL 3B21/3B27, MCTL 4B18: a source address equal to the pending `WADR` reads `L`); the memory itself from *n*+2 |
| PDL buffer | in *n*+1's write pulse, at the pointer or index registered with it (`PWIDX`) | *n*+2: no pass-around, so *n*+1 reads the word as it was (`pdl_read_right_after_a_push_on_the_board`, `tests/cosim.rs`) |
| SPC stack, a push | in *n*+1's write pulse, at the pointer the edge ending *n* moved to | the next-address path in *n*+1 (`SPCWPASS` puts the word on the `SPC` bus); an M-source read of the stack in *n*+1 reads the RAM's old word at the new pointer; *n*+2 reads the new |
| Map, a `WRITE-MAP` store | in *n*+1's write pulse, both levels, addressed by `MAPI` then (`VMA` while `MEMSTART`, else `MD`) | `MAP(MD)` and a dispatch on map bits read the old word in *n*+1 (QUUX's definition; below) and the new from *n*+2. A memory cycle an instruction in *n*+1 starts is translated at the edge ending *n*+2, through the new word: `PHYS-MEM-READ` stores the map and starts a read in the next instruction |
| Dispatch memory, a dispatch write | in *n*'s own write pulse, at its own `DADR` | the dispatch in *n* reads the old word (QUUX's definition); from *n*+1 the new |
| Control store, `WRITE-I-MEM` | in *n*+1's write pulse, at the `PC` it moved to | *n*+1's `IR` takes the word from `IWR` directly, not from the RAM; later fetches the RAM |
| OA registers, `IMOD` | at the edge ending *n* | or'd into `IR` as it loads at that same edge: the instruction executed in *n*+1 |
| `MD`, from memory | at `-LOADMD`, when the bus says | a microcycle that reads `MD` before `READ IN PROGRESS` falls is held by `-HANG` |

Holds and write pulses:

- A cycle held by `-WAIT` fires no write pulse: `TPWP` is `NOR(latch,
  -MACHRUNA)` at CLOCK2 1C10. The pending writes wait for the cycle that
  runs.
- A cycle held by `-HANG` fires its write pulse, which takes its address
  and data as the cycle's time ends: `MD` as the bus has left it then.
  Only the next cycle's start is held (`-TPR0`, CLOCK1 1C08).
- **QUUX's definition**: a RAM read in the cycle its own write pulse fires
  --- the dispatch word a dispatch writes, the map word right after a map
  store --- gives the word from before the write. On the CADR the RAM's
  output floats while written and the answer is a race; `chip`'s answer,
  the new word, rests on its clock cutting the pulse at the edge. Microcode
  323 and 1000 never read either (muir-sys's search of their sources).

## MONO TV, the display

**QUUX's display is MONO TV**, a monochrome frame buffer: 1280 by 1024 unless
`--mono-tv-size` gives another size, one bit a pixel. It is the frame buffer and one register, and nothing else: no sync
program, no color map, and no interrupt, the machine's clock being the
processor's tick. `--tv-board mono-tv`: QUUX's only display, the CADR's two
boards being refused on QUUX, and refused on the CADR.

| | |
|---|---|
| Buffer | 40,960 words, physical `17000000`-`17117777`: 40 words a line, 1,024 lines |
| Pixel | pixel `x` of line `y` is bit `x mod 32` of word `40 y + x / 32` (at the default size), the low bit leftmost, as on the CADR's TV |
| Mode register, `17377760` | bit 2, black-on-white, reads back; every other bit reads 0 and a write of it is dropped |
| Register 4, `17377764` | the color map's write, kept for a color display to come: answers, reads 0, and takes no writes yet |
| Registers 1 to 3 and 5 to 7 | not there: the CADR's sync program and three that did nothing. An access times out and sets the Xbus NXM bit |
| Interrupt | none |

**Its size is muir's to choose**, `--mono-tv-size <width>x<height>`: the width
a multiple of 32, the buffer at most 130,560 words (up to the feature page at
`17377000`), and at most 65,536 with the color TV fitted, whose buffer starts
at `17200000`. 2560 by 1440 fits; 3840 by 2160 does not. The feature page's
words 11 to 13 give the size to the software. The table above is the default
size.

1280 bits a line is 40 whole words, which `BITBLT` needs of a screen array's
first dimension (`BITBLT-DECODE-ARRAY` in `sys/ucadr/uc-tv.lisp`). The buffer
starts where the CADR's does, so the band's `IO-SPACE-VIRTUAL-ADDRESS`
reaches it unchanged, and ends below the color TV's strap at `17200000`.

System 1001 runs on it but draws its screen wrong: `shwarm.lisp` makes the
main screen 768 by 963 at 24 words a line, and MONO TV scans 40, so each of
its lines is spread over parts of several. On QUUX's microcode 1000 with the
tick (`ref/ucode-1000-quux4`) the band reaches its listener on `micro` in
136 M microcycles, as on the CADR's board, measured by reading the rows the
listener draws in at 24 words a line; its writes of the sync program's
registers time out and leave the Xbus NXM bit set, and nothing stops over
it. System 1002 sizes the main screen from the feature page's words 11 to 13,
and draws it right: muir-sys's development band (`ref/band-1002-dev2`,
muir-sys `5427570`, microcode 1000 for revision 4, no sync program and no
speed bits) reaches its listener in
11 M microcycles on both engines, its herald, listener and who line drawn at
the screen's own words a line (`system_1002_runs_on_mono_tv` in
`tests/system_1002.rs`) --- at 1920 by 1080, the size it was built at. A
band fixes its screen's size when its window system loads, so that one
run at another size draws 60-word lines into the raster anyway: at 1280 by
1024 its lines spill into the next, which the test's check catches.
**Unverified** until a band sizes its screen at boot: that System 1002
runs right at every size the feature page can give.

`tests/mono_tv.rs` holds the buffer's first and last words and the NXM past
it on both engines, the bus interface's decode of the whole buffer, the
pixel order and the terminal's frame, the registers there and not there,
the absence of an interrupt over a second, and the feature page's three words.

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
