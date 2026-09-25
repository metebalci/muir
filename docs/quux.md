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
| The clocks in the processor: the 60 Hz tick, the interval timer, the microsecond clock | nothing | none enables the tick yet: the clock handler is still entered from the display's interrupt; the microsecond clock is still read on the Unibus | nothing | nothing |
| Block-disk | muir-sys's PROM 1000 for block-disk, which no longer boots the CADR controller | 1000 for block-disk: the disk routines by block number, no cylinder, head or sector | System 1002: the partitions, the band and the disk routines by block number | block-disk is QUUX's only disk, the default there; the CADR's controller is refused on QUUX |
| The memory cache (`--cache`) | nothing | nothing | nothing | `--cache`; the profile harness's `MUIR_CACHE` |
| No delay lines: `sync`, always | nothing | nothing | nothing | `--sync-cycle-ticks`; `--timing-model cadr` and `fpga` refused on QUUX |
| No hung microcycle; the old word in a RAM's write cycle | nothing | nothing: microcode 324 and 1000 never do either, counted (below) | nothing | nothing |
| No speed bits | nothing | the mode register write at boot need not set them | nothing | nothing |
| MONO TV, the display | nothing | 1000 for revision 4 (System 1002's): the run light in MONO TV's buffer, no TV vertical flag | System 1002 sizes the main screen from the feature page | the terminal, screenshots and captures show whichever screen is fitted |
| The real-time clock | nothing | nothing | does not read it yet | `--rtc` |
| The file device | nothing | nothing | does not use it yet | `--file-root` |

## What each change measured

Microcycles counted by the profile harness (`examples/profile.rs`) over its
workloads on System 1001's band, each change against the machine without
it:

| Change | Measured |
|---|---|
| Six-bit level-1 map (QUUX on microcode 1000 against the CADR on 323 rebuilt, 324) | 11.3% fewer microcycles on `micro` and 11.1% on `rtl` over thirteen workloads; `intern` 31% fewer, `print-scroll` 27%, `compile` 21%, the idle listener 20 to 30% |
| 16K-word PDL buffer (against QUUX's 1K) | 3.8% fewer on `micro` and 4.3% on `rtl` over thirteen workloads; deep recursion 29% fewer, its PDL buffer's share falling from 26% of its microcycles to 0.9% |
| `MUL` and `DIV` (microcode using them against the same without) | 20% fewer microcycles a macroinstruction on the multiply-and-divide workload (25.0 to 20.0), 7% on float, 1.5% on bignum; multiply and divide's share of the first 25.6% to 5.4% |

The time these save on a machine with the synchronous microcycle and the
cache is in [the memory cache](#the-memory-cache).

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
| 15:4 | hardware revision: 9 --- 1 the six-bit map, 2 the 16K PDL buffer, 3 the multiply and divide, 4 the tick, 5 the clocks, 6 the register page and the PROM at 36000, 7 the memory port, 8 the device registers, 9 the real-time clock and the file device | |
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
A memory in on the index wrapping at 2000 words. Microcode for QUUX has to
know the size.

## The feature page

**QUUX lists its sizes in one page of device registers**, physical `17377000`
to `17377377` (page 36776), just below the page the display's control
registers and the disk controller share. It is read-only and read like any
device register, through the map:

| Word | QUUX, revision 9 |
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
| 14 | the interval timer and the microsecond clock: 1 |
| 15 | the devices of revision 9, a bit each: 3, bit 0 the real-time clock and bit 1 the file device |
| 16-377 | 0 |

Word 15 reads 0 below revision 9, as every unused word does, so software
decides by it whether the real-time clock and the file device are there.

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
| Time | one ordinary microcycle, once its operands are ready | ten microcycles in all: nine held once its operands are ready, then its own; 400 ns at four ticks |

Each step's M operand is the output bus of the step before, as when the
microcode writes the step's result back to the same M location. Both
instructions decode on `IR<8>` and `IR<4:3>` only, as the '139 does, and
drive the output bus and load `Q` whatever the output selector `IR<13:12>` and
the Q control `IR<1:0>` say. `DIVIDE-LAST-STEP` and `DVREM` stay the CADR's
separate instructions.

**A `DIV` takes ten microcycles, its own and nine held after its operands
are ready**, QUUX's definition as ruled on 25 September 2026. A register is
ready at once, so a `DIV` of a register closes 400 ns after it entered `IR`
at four ticks; muir-fpga's fabric closes it on the same tick
(`cadr_microcycle.sv` line 2192, its trace `quux_muldiv.quux.k4.golden`
row `c4`). An
`MD` operand first waits for its read to land, through the MD interlock any
instruction reading `MD` has; the count starts in the microcycle after that
wait ends. A `MUL` of `MD` waits the same and then takes its one
microcycle. muir-fpga's measurement is what the ruling settles: since
contract Q6 a main-memory read releases at its acknowledgement, so a `DIV`
of `MD` can run in the microcycle after the word lands, and the fabric's
divider needs its operand 17 ticks before the `DIV` ends. Its program
`quux_divmd` puts a `DIV` of `MD` there; counted from the `DIV`'s entry into
`IR`, the hold runs during the wait and the new word is divided at once,
where the fabric divided the old `MD` --- at microcycle 81, an output bus of
`a850e26d` against the fabric's `11465777`. Counted from the end of the
wait, both divide the new word.

The hold is a `-WAIT` term of QUUX's own: the divider is busy while a `DIV`
stands in `IR`, not nopped, and `muldiv::DIV_CYCLES`, nine, generator cycles
have not passed since the edge that loaded `IR` or, if the MD interlock held
it, since the end of that hold. The master clock runs on, so the memory port
carries on. Nine held microcycles of QUUX's 40 ns at four ticks is 360 ns,
which covers the divider's 330 (32 quotient bits and a load at 10 ns each), and muir-fpga
finds it fits with 15 ticks to spare. The count is in microcycles, not
nanoseconds: at another `--sync-cycle-ticks` muir still holds nine, ten in
all, and a
contract that changes the microcycle's length recounts it. The time does
not depend on the operands. A halt during the hold stops the machine with
the `DIV` still in `IR`. The hold does not stop a single step, as `-WAIT`
does not; by then the divider is done.

**Unverified:** whether the fabric starts the count after `-WAIT`'s other
terms too --- a memory destination written while a cycle is busy, a fetch
--- which hold the instruction and not its operands. muir counts through
them, since the ruling counts from when the operands are ready; a `DIV`
with a memory destination behind a busy cycle, run on both, would settle
it.

`tests/muldiv.rs` holds each against the step sequence on the CADR, for many
operands and every output selector and Q control, on `micro` and `rtl`, with
the step sequence itself held to the netlist; the CADR's 42 and 43 to the
netlist; the hold's length on both engines, from a register's `DIV`'s start
and from the end of a miss's wait for a `DIV` of `MD` (the `DIV` ends nine
microcycles after a plain copy of `MD` in its place, 21 after the read's
start on `rtl` where the copy ends at 12), and at three ticks; a `MUL` of
`MD` ending with the copy; the CADR's 43 held for nothing; a halted and
single-stepped `DIV`; and a checkpoint taken during one.

## The clocks in the processor

**QUUX's processor has its clocks** (revision 5): a tick, fixed at 60 Hz,
an interval timer, and a microsecond clock. The CADR has no clock in the
processor. Its clock is the display board's vertical interrupt: microcode
323's `INTRX0` (`sys/ucadr/uc-interrupt.lisp`) reads the TV's mode register,
clears its vertical flag, and runs the "roughly-60-cycle clock" handler ---
the mouse, the disk's idle time, the Chaosnet's transmit-abort wakeup, and
the sequence-break counter the scheduler runs on. Its microsecond clock and
interval timer are on the I/O board, on the Unibus (`764120`-`764124`).

| | |
|---|---|
| Functional destination 3 | control: `<0>` the tick's enable, and a write with `<1>` set clears its flag; `<2>` the interval timer's enable, and a write with `<3>` set clears its flag |
| Functional destination 4 | the interval timer's period in microseconds, `<23:0>`; a write starts a period from then; 0 stops it |
| Functional source 17 | `<0>` the tick's flag, `<1>` its enable, `<2>` the interval timer's flag, `<3>` its enable |
| Functional source 15 | the microseconds since power-on, 32 bits, wrapping: one read gives the whole word |
| The tick's period | 16,667 µs, 60 Hz, fixed |
| Interrupt | while enabled, each flag is ORed into the interrupt pending that jump conditions 5 and 6 test, so it costs nothing until it rises |

A flag rises a period after its timer is enabled (the interval timer's
also after its period is written), then every period after, whether or not
it was cleared between; a clear takes it down until the next. `-RESET`
turns both off. A flag that rises while a microcycle waits for `MD` is up
for the jump after it: `SINTR` is registered at the edge that ends the
waiting microcycle, with the flags as they stand then
(`a_flag_rising_during_a_wait_is_seen_by_the_jump_after`, the case
muir-fpga measured). Revision 4's tick took its period from destination 4;
revision 5 fixes it at 60 Hz and gives destination 4 to the interval timer.

On the CADR, destinations 3 to 7 have no output on the 74S138 that decodes
them and write only M, and sources 15 and 17 have none either and read all
ones. Microcode 323 writes destinations 3 to 7 and reads sources 15 and 17
nowhere, by a scan of every control-store word, and running shows the same
of what the OA registers make at run time: `tests/unused_codes.rs` reads
every executed microinstruction as it stood in `IR` through a boot to the
listener. System 1001 on 323, on the CADR, runs none; System 1002 on its
microcode for revision 4 runs them at three addresses, each a control-store
word that carries them, the tick's own, and reads source 15 nowhere.

`tests/tick.rs` holds the interval timer against a 100 and a 300 µs period,
its clear and its stop at 0, the tick at 60 Hz whatever destination 4 says,
the interrupt condition taken with either on and not with both off, source
15 against each engine's time (under `sync` too) and across its wrap, the
CADR's all ones, and a checkpoint taken in the middle of an interval, on
`micro` and `rtl`.

## QUUX drops the delay lines

**A QUUX microcycle is a fixed number of 10 ns ticks, always**: `sync`,
`--sync-cycle-ticks` of them, 4 unless a board's fit says otherwise. The
CADR's clock is a string of delay-line phases: the read phase ends at the
tap the mode register's `SPEED1` and `SPEED0` choose (`mit/cadr/ir.bits`:
"00 Extra slow, 01 Slow, 10 Normal, 11 Fast"), and a 60 ns restart follows,
145 ns at normal speed, which muir-fpga's fabric replays as 15 ticks. QUUX
has none of it: no taps, no speed bits (a write of the mode register's bits
1 and 0 goes nowhere; its other bits are unchanged), and no timing but
`sync`. `--timing-model cadr` and `fpga` are refused on QUUX, and an engine
made for a QUUX machine starts on four ticks: `rtl` refuses another timing
on it, and `micro`'s clock counts the same ticks.

The ticks are a board's: the number its fit proves its longest path settles
in. **Four ticks, 40 ns, is met on the Arty Z7-20**: muir-fpga's QUUX at
four ticks, with contracts Q1-Q5 and block-disk, its commit `099507d`
(against muir at `e12749e`), has a worst setup slack of +0.461 ns and no
failing path, in 11,974 LUTs; an earlier fit of
the same design measured the longest chains as MD through both
map levels and the M bus to the control-store address, 28.0 ns of 40, and
the multiplier, 24.1 ns of the 30 a path after the scratchpads gets. Both
boards run four ticks: three, 30 ns, is out of the DE25-Nano's reach by
the divider alone, a `DIV` of `MD` needing its word 17 ticks before its
hold ends. The DE25-Nano at four ticks meets them at every corner, a worst
setup slack of +0.180 ns at `099507d`, its thinnest path the microsecond
clock's count under a constraint a tick tighter than the microcycle's (an
earlier fit's longest chain was MD through the maps to the next address at
16.1 ns); the level-1 map's MLAB write-to-read, which Quartus does not time,
is **unverified** by timing analysis, the rest of that path having 30 ns and
measuring about 16.

What the microcode sees does not change, only the time: every register is
clocked at the one edge and the late writes land one edge later, as the
single-edge contract below has it; the bus keeps its own time on the grid;
a held cycle is one microcycle long; and a `DIV` is ten microcycles, its
own and nine held, at any length. An `ILONG` instruction takes `ilong_ticks`
more, 0 unless the library says otherwise.

`tests/sync_timing.rs` holds the microcycle's length, the default and the
refusal, `ILONG`'s ticks, the grid, the divider's hold and a checkpoint;
`quux_has_no_speed_bits` in `tests/quux.rs` the speed bits;
`quux_runs_on_sync_alone` in `tests/cli.rs` the flags; and
`system_1002_runs_at_its_ticks` in `tests/system_1002.rs` System 1002 at
four ticks and at three.

## The memory port and the device registers

**QUUX's main memory and its frame buffer are on the processor's own
port**, the memory bus (contract Q6, revision 7; the frame buffer from Q7,
revision 8), and **its devices are reached by their registers alone**
(contract Q7, revision 8): there is no Xbus and no bus in its place. A
cycle to main memory or the frame buffer goes through the cache to the
memory controller; one to a device register goes to the processor's
register decode; one to any other address fails at once. QUUX has no bus
interface: the Unibus is gone (Q5) and the processor is the only
requester. Devices move bulk data to and from memory themselves ---
block-disk's transfers, the display reading its buffer --- and carry
control and small data in their registers. The CADR keeps its Xbus, Unibus
and bus interface. The boot PROM is not in the address space: it is in the
control store (Q2).

| | |
|---|---|
| Main memory | 380 ns a line fill and 290 ns a write (`MemoryTiming::NOMINAL`, the DE25-Nano's, the slower board's), one operation at a time, no setup, deskew or refresh; a floor, a board slower on an access waiting. `--memory-timing <read>,<write>`, `arty` or `de25` sets others, on `rtl` |
| The frame buffer | `17000000` up to MONO TV's buffer's end: on the memory bus with main memory, cached. The software reads it back (`BITBLT` combines with the destination, scrolling copies), and the display only reads it, which a write-through cache keeps current |
| The cache | always fitted: 4K words (`--cache <words>` another size) |
| A device register | MONO TV's at `17377760`, block-disk's at `17377774`, the feature and register page at `17377000`: never cached, taken at the edge and answered a microcycle on, two microcycles in all |
| Nothing there | past main memory's or the frame buffer's end, between the registers, the old Unibus window: fails at once, in the microcycle, reading 0 and setting word 101's NXM bit, with no timeout. A write there does not read back, which the microcode's memory-size probe, `MEM-SIZE-LOOP` in `uc-cold-disk.lisp`, relies on; nothing in System 1002 depends on how long a failed access takes (muir-sys, read) |
| Block-disk | its words move at START, and the cache is invalidated; a transfer reads the processor's writes made before START and, after DONE, no read hits a word from before it |

The bus interface's registers all have homes on QUUX already: the
diagnostic registers are the host's (Q5), `ERRSTOP` is word 102, the
interrupt control word 100 and the error status word 101 (Q2); the Unibus
map is gone (Q5).

`tests/quux_memory_port.rs` holds the port in place of the bus interface,
a miss's line fill against a hit, a register never cached, nothing past
main memory's end reading back on both engines, the disk's write never hit
stale, and a checkpoint; `tests/quux_device_registers.rs` a register a
microcycle longer than nothing, every empty range failing at once on both
engines, and the frame buffer through the cache;
`quux_s_memory_port_and_its_timing` in `tests/cli.rs` the flag and the
start's report.

## The memory cache

**QUUX's memory cache** (H2) is unified and write-through, in front of main
memory and the frame buffer, by physical address after the map. Device registers are not cached. In muir it is `rtl`'s, and
it holds tags only: `rtl` takes a word from main memory as a cycle ends,
and a write-through cache never holds a word memory does not, so the cache
changes when a cycle is answered and never what it reads.

| | |
|---|---|
| A read that hits | acknowledged `hit_ns` after the request (20 ns, two ticks of the grid, met on the Arty in muir-fpga's fit; the DE25's to follow), with no bus cycle and none of the bus's setup, deskew or release |
| A read that misses | main memory's line fill; it fills the line, the set's least recently used line going |
| A write | allocates nothing. With the write buffer it is acknowledged after `hit_ns`, or when the buffer's last write is done, and main memory runs it behind the processor; the word is memory's from the acknowledgement, as a read after it finds it |
| Shape | lines of 4 words, 2-way, `<words>` in all, a power of two |
| Coherence | a disk transfer writes main memory behind the processor, and invalidates the whole cache before the next cycle |

Behind the cache is main memory on the port. muir-fpga measured its two
boards, a 1,000,000 word array loop on System 1001 for 300 s:

| | Read, average | Write | In muir |
|---|---|---|---|
| Arty Z7-20 | 20.68 ticks of 10 ns (19 to 88) | 12 | read 220 ns, write 120 ns |
| DE25-Nano | 36.25 (33 to 228) | 29 (28 to 58) | read 380 ns, write 290 ns |

rounded up to the tick, with a tick more on a read for a line fill of four
words, two 64-bit beats: the machine has only ever made single-word
accesses, so a fill's time is **unverified**.

Measured with main memory as the CADR's memory boards on the Xbus, and as
the boards' own timings where the table says so, with the profile harness (`examples/profile.rs`, `MUIR_CACHE` and
`MUIR_SYNC_TICKS`) on `rtl`, System 1001's band on QUUX's microcode 1000
for revision 4, over ten workloads (compile, two call-heavy, cons, the
multiply-and-divide, float, array, sort, bignum, intern), in the machine's
time:

| | Total | |
|---|---|---|
| the CADR's delay lines, 145 ns, the baseline | 72.7 s | 1.00 |
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
write-and-read-back loop leaving the same words with the cache as with one
that saves nothing, and sooner, the invalidation, and a checkpoint.

## Block-disk

**QUUX's disk is block-disk**, and nothing else: a QUUX run has it
without `--disk-controller`, and the CADR's controller is refused
(`quux_s_disk_is_block_disk_only` in `tests/cli.rs`). It is the CADR
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

**Block-disk's contract is the outcome, not the time**: the status bits, the
disk address left at the last block moved or at the one that failed, the
command list pointer and memory. How long a transfer stays active, whether
it succeeds or fails, is the implementation's, and software waits for
not-active before it reads the status. muir's model does as follows, and a
fabric that walks its list through a host stays active until the walk ends,
a failure too.

The words move inside the store to START, and the controller stays busy for
a block's time each; a transfer writes main memory behind the processor, so
the memory cache is invalidated. The disk is a file in a standard format,
[below](#the-disk-file). `tests/block_disk.rs` holds the read, the write,
the end of the disk, the NXM, a command it does not do, the registers on
QUUX's bus and a checkpoint.

muir-sys's boot PROM 1000, microcode 1000 and System 1002 address it by
block: System 1002's band boots on it on `micro`, `rtl` and under `sync`,
at 1024 by 768, 1280 by 1024 and 1920 by 1080 (`tests/system_1002.rs`). Lisp checks the disk address after a transfer
against the last block it expected, and reads the command list pointer
and the disk address after one; it does not read the fourth register,
where the CADR's controller gave the ECC.

## The disk file

**QUUX's disk is a raw image, a fixed VHD or a dynamic VHD, of any size**
(contract Q8); the CADR's pack stays MIT's, a raw Trident image exactly a
T-300's size, and a file of any other size is refused on it as before
(`the_cadr_refuses_quux_s_disks_as_before` in `tests/quux_disk.rs`).
Block-disk's block `n` is the file's 512-byte sectors `2n` and `2n + 1`,
and the disk's size in blocks is the file's size, or a VHD's current size,
over 1,024; a last half block is not reachable. A VHDX, a differencing VHD
and a VHD whose footer or dynamic header does not check are refused, saying
which (`what_muir_does_not_read_is_refused`).

| | raw | fixed VHD | dynamic VHD |
|---|---|---|---|
| The file | the disk's bytes | the disk's bytes, then a 512-byte footer | a copy of the footer, a dynamic header, the block allocation table, 2 MiB blocks each after a 512-byte sector bitmap, the footer |
| Told by | no `conectix` footer | the footer at the end, disk type 2 | the footer at the end, disk type 3; or, with the one at the end lost, its copy at 0 |
| The disk's size | the file's | the footer's current size, offset 48 | the same |
| A write | in place | in place, the footer untouched | in place in an allocated block; an unallocated one is allocated where the footer was, its bitmap all ones, then the footer after it, then the table entry |

**The format is the footer's, not the name's**: a fixed VHD called `.img`
is still a fixed VHD (`the_format_is_the_footer_s_not_the_name`), and
qemu-img reports a fixed VHD as raw unless told `-f vpc`. The start says
which of the three a QUUX run's disk is, and its size in blocks
(`quux_s_disk_is_raw_or_a_vhd_of_any_size` in `tests/cli.rs`).

`data/quux-disk*` are an 8 MiB disk in all three formats and a dynamic VHD
after three writes into unallocated blocks, with its raw twin, made by
qemu-img and qemu-io (`tools/quux-disk-fixtures.sh`). Every block of each
reads through muir as its raw twin (`each_fixture_reads_as_its_raw_twin`);
muir's own writes of the same bytes into the dynamic VHD make qemu-io's file
byte for byte, the footer checking and its copy at 0 unchanged
(`a_write_grows_a_dynamic_vhd_as_qemu_does`); and where qemu-img is installed
it compares a dynamic VHD muir grew equal to the raw file given the same
writes (`qemu_reads_what_muir_grew`, skipped without it). A disk opened
`ro` keeps its writes for the run and a checkpoint, the file untouched
(`opened_read_only_nothing_reaches_the_file`); a checkpoint carries the
disk's size in blocks and refuses a disk of another
(`a_checkpoint_keeps_the_disk`).

**The partition table is a GPT.** A partition's type is one of QUUX's own
type GUIDs, whose first 32-bit words all differ:

| Partition | Type GUID |
|---|---|
| microcode, `MCRn` | `9e318cf5-a95b-4b3b-b2ad-9ae306b0e2da` |
| band, `LODn` | `a3b30470-c5d4-41c1-87a8-d26590424cb8` |
| `PAGE` | `4652bea5-06af-4bd9-b2bb-3541370151c8` |
| `FILE` | `7afa9532-75de-409f-8dc8-fef9763511d5` |
| retired: once `TEMP`; not to be used | `445976f2-34e4-4583-b750-75d28a080cba` |

- **The name** is the partition's four-character Lisp name, a space and a
  comment of up to 31 characters, `MCR1 UCADR 1000`: 36 characters, which
  is what a GPT name holds.
- **The current microcode and the current band** carry attribute bit 48,
  the first of the bits a GPT leaves to the partition type.
- **Whole blocks**: a partition's first LBA is even and its last odd, so it
  starts and ends on a block.
- **At most 8 GiB**, 2^23 blocks, because Lisp's fixnums hold block numbers
  below 2^23. Block-disk's address, `<27:0>`, reaches further, and muir
  opens a larger file; the limit is the software's.
- **No pack name and no pack comment**: a GPT has neither.
- **There is no TEMP partition**, and the fixtures have none. MIT's PROM
  saves page 0 to block 1 of its pack, which on a GPT disk is the
  partition table; QUUX's saves nothing and writes no block of the disk
  ([below](#its-boot-prom-in-its-own-addresses)). No fixture partition
  carries the retired type GUID (`the_fixtures_gpt_is_q8_s`).

`the_fixtures_gpt_is_q8_s` reads the fixtures' GPT through muir's disk
layer and holds them to all of this. muir reads no partition of QUUX's
disk itself: block-disk moves blocks, and the partitions are the machine's
software's to find. `diskpack`, MIT's label editor, is the CADR's; given a
QUUX disk it says what the file is and that its partitions are made with
sgdisk, and writes nothing (`quux_s_disk_is_named_and_left_alone` in
`tests/diskpack.rs`). The boot PROM in `data/quux-promh.mcr`, the
microcode and System 1002's band read the GPT; MIT's label in block 0 is
the CADR's.

**System 1002's band is dev11, on a GPT disk in a dynamic VHD**: muir-sys's
development band "System 1002 dev11", built from its commit `8942300`, a
T-300's 263,245 blocks with the current `MCR1` at block 17 holding
microcode 1000 and the current `LOD4` the band, no FILE and no TEMP.
QUUX boots the VHD as it is, a copy of it, the disk being written; the
tests find it in the gitignored `ref/band-1002-dev11` and skip without it.
It reaches its listener, drawn at the screen's own words a line, in 166 M
microcycles on `micro` and 187.5 M on `rtl` at 1280 by 1024
(`system_1002_runs_on_mono_tv`, measured to the half million). It
restores its own band: `(si:disk-restore 4)`, answered `yes`, reads 20,832
blocks of `LOD4` in 26 M microcycles and is back at the listener 139 M
later, on `micro` (`system_1002_restores_its_band_to_the_listener`). That
test holds, at every microcycle in `DISK-AWAIT-READY`, the disk registers'
virtual address `77377774` to their physical `17377774`, and block-disk to
moving on every million microcycles. The cold boot's `COLD-FAKE-L2-MAP`
maps the disk registers and the run light; when the two take one level-2
slot, the disk registers' virtual address reaches another word, and the
restore waits in `DISK-AWAIT-READY` for ever. On a microcode with that
collision, System 1002 dev9's, the test fails at the first check, the
address reaching `17117774`, and without it at the second (measured).

How to make a disk with standard tools is in
[the manual](manual.md#quuxs-disk).

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
| `MD`, from memory | at `-LOADMD`, when the bus says | on the CADR, a microcycle that reads `MD` before `READ IN PROGRESS` falls is held by `-HANG`; on QUUX it waits (below) |

Holds and write pulses:

- A cycle held by `-WAIT` fires no write pulse: `TPWP` is `NOR(latch,
  -MACHRUNA)` at CLOCK2 1C10. The pending writes wait for the cycle that
  runs.
- On the CADR, a cycle held by `-HANG` fires its write pulse, which takes
  its address and data as the cycle's time ends: `MD` as the bus has left
  it then. Only the next cycle's start is held (`-TPR0`, CLOCK1 1C08).
- **QUUX has no hung microcycle** (`Geometry::hangs`): a microcycle that
  reads `MD` while a read is in flight waits as for `-WAIT`, whole
  microcycles with no write pulse, and runs once, whole, when the word is
  in `MD`. Unlike `-WAIT`, a single step does not pass it. Its writes
  therefore take their addresses from the word read: a dispatch write
  addressed by `MD`, and a map store's write pending into it, land where
  the word read says, where the CADR's land at the `MD` from before
  (`on_quux_a_dispatch_write_addressed_by_md_waits_for_the_word_read`,
  `on_quux_a_map_write_pending_into_the_wait_lands_at_the_word_read`,
  `on_quux_the_wait_for_md_is_whole_microcycles` in
  `tests/dispatch_write_order.rs`).
- A memory cycle goes out at the edge ending the microcycle after its
  start, and a write carries `MD` as it stands then: an `MD` loaded in
  that microcycle is the word written, on the CADR as on QUUX; one loaded
  later waits on `MBUSY.SYNC` for the cycle to end
  (`chip_and_rtl_write_the_md_of_the_microcycle_after_the_start` in
  `tests/chip.rs`, `a_write_carries_the_md_of_the_microcycle_after_its_start`
  in `tests/quux_memory_port.rs`).
- **QUUX holds a memory start in the microcycle right after a start**, a
  `-WAIT` term of its own, `MEMSTART AND MEMOP`, until the first cycle has
  gone out and ended; both then land as written
  (`a_start_right_after_a_start_waits_for_it` in
  `tests/quux_device_registers.rs`). On the CADR nothing holds it: the
  cycle that goes out takes the second start's direction and `VMA<7:0>`,
  and the first is lost
  (`on_the_board_a_start_right_after_a_start_loses_the_first` in
  `tests/chip.rs`). **Unverified** that muir-fpga's fabric holds it.
- **QUUX's definition**: a RAM read in the cycle its own write pulse fires
  --- the dispatch word a dispatch writes, the map word right after a map
  store --- gives the word from before the write
  (`Geometry::old_word_while_written`), as an FPGA's block RAM gives it.
  On the CADR the RAM's output floats while written and the answer is a
  race; muir's CADR engines take `chip`'s answer, the new word, which rests
  on its clock cutting the pulse at the edge
  (`on_the_cadr_rtl_and_micro_take_the_word_chip_does` and the two map
  tests beside it).
- **Nothing MIT's or muir-sys's microcode runs does either.** Counted on
  `rtl` over a boot of System 1001 and the profile harness's thirteen
  workloads, on QUUX (microcode 1000) and on the CADR (System 1001's own
  microload, 324, which is 323 rebuilt): no dispatch write
  with `POPJ`, no map read in the microcycle a map store's write lands, and
  no hung microcycle whose pulse writes the dispatch memory or the map.
  The same counters fire on the test programs built to do each.

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
| Registers 1 to 3 and 5 to 7 | not there: the CADR's sync program and three that did nothing. An access fails at once and sets the NXM bit |
| Interrupt | none |

**Its size is muir's to choose**, `--mono-tv-size <width>x<height>`: the width
a multiple of 32, and at most **1920 by 1080**, the largest QUUX supports
(`a_size_is_checked` in `tests/mono_tv.rs`). That is 64,800 words, below the
color TV's buffer at `17200000` and well below the feature page at
`17377000`, the most the address space leaves below the registers (130,560 words). The feature page's
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
registers fail and leave the NXM bit set, and nothing stops over
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

## Its boot PROM, in its own addresses

**QUUX's boot PROM has control store addresses of its own**, 36000-37777,
1K words, read only and never overlaid (revision 6, contract Q2). Reset
starts the PC at 36000; the microcode lives in 0-35777, which is RAM from
the start, and there is no PROM-disable bit: the PROM loads the microcode
and jumps to 6. A reboot is a jump to 36000. The CADR keeps MIT's overlay:
its PROM covers 0-777 until `PROMDISABLE` in the mode register, written at
Unibus `766012`, lets the RAM show through.

The PROM is muir-sys's version 1000 for block-disk and a GPT
(`data/quux-promh.mcr`), MIT's `promh.text` changed so that a PDL buffer
of any width boots, QUUX's 64 level-2 blocks are cleared, the disk is read
by block number, nothing is saved, and the microcode is found through the
GPT (muir-sys's `sys/ucadr/promh.text`, handed over with System 1002 dev11,
which was built from muir-sys's commit `8942300`), assembled at 36000. It sets error stop through the register page, not `766012`, and
halts at `ERROR-MICROCODE-TOO-BIG` if a microcode reaches 36000. The
control store stays 16K words: jump targets are `IR<25:12>`, dispatch
words carry 14 address bits, and `SPC<14>` is the macroinstruction-return
flag, so 32K waits for a new microinstruction format.

**It finds the microcode through the GPT**: the first microcode partition
in the entry array carrying attribute bit 48 (muir-sys). Its own halts are
`ERROR-NO-GPT` at 36632, no GPT (or an entry array whose LBA does not fit
in 32 bits, within the 8 GiB limit, muir-sys says);
`ERROR-NO-CURRENT-MICR` at 36634, no current microcode partition; and
`ERROR-ODD-MICR-START` at 36636, one whose first LBA is odd (the
hand-over's error table `promh.tbl` and symbols `promh.sym`). On a pack
with MIT's `LABL` label and no GPT --- a T-300 label with microcode 323 in
`MCR1`, which the PROM before it booted --- it reads block 0 into page 3,
nothing else, and halts at `ERROR-NO-GPT` after 630,129 microcycles on
`micro` and 661,374 on `rtl`, having written nothing
(`quux_s_prom_reads_a_gpt_not_mit_s_label`). **Unverified**: the other two
halts, which no test here reaches.

**It saves nothing and writes no block of the disk** (contract Q8). MIT's
PROM saves main memory's page 0 to block 1 before it loads anything
(`SAVE-A-PAGE`, `mit/sys/ucadr/promh.text`), and on a GPT disk block 1 is
the partition table's entry array. QUUX's reads every block into its buffer
at physical page 3, words 1400-1777, and loads the microcode's main-memory
section --- four blocks, pages 3-6, the microcode symbol area --- last,
over the buffer. Two halts are for that: `ERROR-TWO-MAIN-MEM-SECTIONS` at
36040, a second main-memory section with blocks, and
`ERROR-BUFFER-NOT-LOADED` at 36042, a section that does not cover the
buffer. 36000 is `JUMP GO`, and `GO` is at 36043; the code ends at 36636
(`promh.locs`, `I-MEM 36637`). `tests/quux_prom_saves_nothing.rs` boots it
on both engines until the microcode's location 6 runs, on
`data/quux-disk.img` with MIT's microcode 323 in its `MCR1` and on System
1002 dev11's VHD with microcode 1000, and counts: no block written; the
only stores are to word 777, the command list word, one a block read;
every block read goes into pages 3-6; the disk file is byte for byte as it
was; and pages 3-6 hold the main-memory section's four blocks. On dev11 it
reads 115 blocks and reaches 6 after 1,020,407 microcycles on `micro` and
1,136,063 on `rtl`.

**Its file, like QUUX's microcode's, is in partition order** (contract
Q8): MIT's `.mcr` with the two 16-bit halves of every 32-bit word swapped,
so that each word is stored low byte first, as it lies in a microcode
partition and as block-disk reads it, and a whole number of 1024-byte
blocks, so that `dd` writes a microcode file into its partition with no
conversion. muir-sys's `sys/sys/qwmcr.lisp` writes it.
Swapped back, the PROM's file has MIT's `promh.mcr`'s four sections, its
dispatch and A memory word for word MIT's, only the program QUUX's
(`quux_s_prom_is_mit_s_promh_changed`); and System 1002 dev11's `MCR1`
holds the hand-over's `ucadr.mcr` block for block, as `dd` put it there
(`quux_s_prom_saves_nothing_on_system_1002_s_disk`). The CADR's `.mcr`
stays MIT's, and so does `diskpack`, which is the CADR's.

`tests/quux_prom.rs` holds the start at 36000, the PROM read only, the RAM
below live with no disable, the CADR's overlay, and the file read from
36000 in partition order; `tests/system_1002.rs` boots System 1002 on it.
`--prom` on QUUX takes a file in partition order assembled at 36000, and
refuses one in MIT's order or assembled at 0.

QUUX runs only muir-sys's latest System 1002 band. The PROMs assembled at
0, and System 1001 on QUUX, are retired with it.

## The register page

**QUUX's registers share the feature page**, `17377000`-`17377377` (revision
6, contract Q2): the address space below it is full, pages 36000-36775 being the
largest MONO TV buffer and 36777 the display's and disk's registers.

| Word | |
|---|---|
| 0-77 | the feature page, read only |
| 100 | interrupt status, read only: `<0>` the tick, `<1>` the interval timer, `<2>` block-disk's done, `<3>` the keyboard, `<4>` the mouse, `<5>` the network, `<6>` the file device, each under its own enable |
| 101 | error status: the bus errors, as `766044` gives them; a write clears them |
| 102 | mode: `<0>` error stop, which the host can set too |
| 103 | the real-time clock, read only (below) |
| 120-123 | the keyboard and the mouse (below) |
| 140-147 | the network (below) |
| 160-171 | the file device (below) |
| others | reserved: read 0, writes ignored |

On the CADR nothing answers on the page. `tests/quux_registers.rs` holds
each word, on the machine and through both engines' bus.

## The real-time clock

**QUUX keeps the real time in word 103 of the register page** (contract Q9,
revision 9): whole seconds since 1970-01-01 00:00 UTC, Unix time, as an
unsigned 32-bit number, which lasts to 2106. There are no fractions: the
microsecond clock, functional source 15, counts finer time from power-on.
Lisp's universal time counts from 1900, so it is the word plus 2,208,988,800,
a sum a 32-bit count from 1900 could not hold past 2036.

| | |
|---|---|
| Read | the seconds, `<31:0>` |
| Written | nothing: a write goes nowhere and the word reads on unchanged, as a write of any read-only word on the page does. The host keeps the time; the machine never sets it, and the time zone is not the clock's |
| The CADR | nothing answers on the page: a read times out and sets the NXM bit |

**It is live**: it gives the host's time, kept current by the host, as a
real clock keeps real time on its own crystal whatever the processor does.
muir reads the host's clock (`SystemTime`) at every read of the word, so it
never drifts from the host, however fast or slow the engine runs.

**`--rtc <s>` fixes it for runs that repeat**: the clock reads second `s`
at power-on and counts the machine's own time from there, a second for each
10^9 ns of the engine's clock, whatever the host's clock does. It holds at
2^32-1 and never wraps to 0, which would read as no clock at all; a start
past 2^32-1 is refused. `--rtc host` is the default. The start says which:
`rtc: the host's clock` or `rtc: from <s>, counting machine time`. A
checkpoint carries the setting, the start and the machine time it counts
from, so a resumed run reads the second the run that wrote it would have;
a resume under another `--rtc` is refused by the flag's name.

`tests/quux_rtc.rs` holds the live word against the host's clock, the start
and the count, the hold at 2^32-1, a write changing nothing, the CADR's
timeout, feature word 15, a checkpoint, and both engines reading a second
go by in their own time; `the_rtc_is_quux_s` in `tests/cli.rs` the flag,
its refusals and the start's report; `a_resume_has_the_checkpoint_s_rtc` in
`tests/muir_checkpoint.rs` the resume.

## The file device

**QUUX reads and writes files on its host through a file device** (contract
Q9, revision 9): folders of the host served to the machine under one
pathname host, `HOST`, with commands and responses in two rings in main
memory and the bytes moved by DMA. The registers carry control and the
rings' indexes; nothing polls memory for an index. `src/file_device.rs` is
muir's device.

| Word | | |
|---|---|---|
| 160 | control, read and written | `<0>` enable, `<8>` interrupt enable |
| 161 | status, read only | `<0>` enabled, `<1>` quiet, `<2>` configuration refused, `<3>` index fault, `<8>` a response waiting, `<23:16>` handles open |
| 162 | command ring base | `<23:0>` a physical word address, `<1:0>` 0 |
| 163 | command ring size | `<3:0>` the log2 of its entries, 0 to 8 |
| 164 | command producer, the processor's | `<15:0>` |
| 165 | command consumer, the device's, read only | `<15:0>` |
| 166 | response ring base | as 162 |
| 167 | response ring size | as 163 |
| 170 | response producer, the device's, read only | `<15:0>` |
| 171 | response consumer, the processor's | `<15:0>` |

**Configuration.** 162, 163, 166 and 167 are written while the device is
disabled and ignored while it is enabled. The enable, 160 `<0>` from 0 to 1,
checks them: a base off a 4-word line, a size over 8, or a ring reaching
past main memory is refused, status `<2>`, and the device stays disabled.
While disabled the four indexes read 0 and writes of 164 and 171 go nowhere;
the enable starts them at 0. A write of 160 clears `<2>` and `<3>`.

**Indexes.** Each counts entries, 16 bits, free-running; an entry's slot is
the index mod the ring's size. The processor writes a command into slot
`164 mod size` and then 164 with one more (or n more). A write of 164
claiming more commands than the ring holds, or fewer than are waiting, and a
write of 171 past 170, are ignored and set status `<3>`. The device answers
each command with one response, in command order, so response i answers
command i, and 165 and 170 always read the same. It takes the next command
only while the response ring has room, 170 - 171 below its size.

**Entries** are 8 words, command and response alike.

| Word | Command | Response |
|---|---|---|
| 0 | `<15:0>` tag, `<23:16>` opcode, `<31:24>` flags | `<15:0>` tag, `<23:16>` status, `<31:24>` opcode |
| 1 | handle (READ, WRITE, CLOSE) | count: bytes written to B (READ, DIRECTORY, COMPLETE), or taken from A (WRITE) |
| 2 | buffer A's address, `<23:0>` | handle (OPEN read or write) |
| 3 | buffer A's length in bytes | the file's length in bytes (OPEN, CLOSE) |
| 4 | buffer B's address | mtime, Unix seconds (OPEN, CLOSE) |
| 5 | buffer B's length | flags: `<0>` a directory, `<1>` on a read-only mount, `<2>` COMPLETE: an entry is exactly the completion, `<3>` and it is a directory |
| 6 | READ, WRITE: the offset in the file; DIRECTORY: the cookie | DIRECTORY: the next cookie, 0 at the end; COMPLETE: the matches |
| 7 | CLOSE: the date to set, Unix seconds | 0 |

A failed command's response is word 0 alone. A buffer starts on a 4-word
line, holds at most 65,536 bytes, and lies in main memory; byte k is bits
`8(k mod 4)+7:8(k mod 4)` of word k/4. A READ of n bytes writes the first
`ceil(n/4)` words of B, the bytes past n 0, and no other word.

**Commands.**

| Op | | In | Out |
|---|---|---|---|
| 1 | OPEN | A the name; flags `<1:0>` 0 read, 1 write, 2 probe; `<3:2>` if it exists (write): 0 supersede, 1 error, 2 append; `<4>` if it does not (write): 0 create, 1 error | handle (none for a probe), length, mtime, flags `<0>` `<1>` |
| 2 | READ | handle; B where, its length the bytes wanted; offset | count, the wanted or what is left |
| 3 | WRITE | handle; A the data; offset | count |
| 4 | CLOSE | handle; flags `<0>` abort, `<1>` set the date from word 7 | length and mtime as closed; 0 after an abort |
| 5 | DIRECTORY | A a directory, `/` the root; B at least 272 bytes; cookie, 0 to start | count, next cookie |
| 6 | COMPLETE | A `<directory>/<prefix>`; B | count (the completion in B), matches, flags `<2>` `<3>` |
| 7 | DELETE | A a file or an empty directory | |
| 8 | RENAME | A the old name, B the new | |
| 9 | CREATE-DIRECTORY | A, one level | |
| 10 | LOG | A a line of at most 1,024 bytes | |

- **The device moves bytes and never interprets them.** There is no
  character mode or byte size.
- **OPEN read** keeps the host file open, so a rename or delete on the host
  does not disturb the handle. There are 64 handles, numbered 1 to 64.
- **OPEN write** writes a temporary file in the target's folder, named
  `.quux-write-` and more, which DIRECTORY and COMPLETE never show. CLOSE
  renames it onto the name, so the file appears or changes whole; until
  then the name is as it was. Supersede starts it empty, which is also what
  the Lisp side sends for `:OVERWRITE` and `:TRUNCATE`: "starting at the
  beginning, and set the file's length to the length of the newly written
  data" (MIT's `sys/man/files.text`, lines 282-288). Append starts it as a
  copy of the file, and the reply's length is the copy's. Error answers FAE
  at OPEN for a name that exists, and at CLOSE for one that appeared
  meanwhile, discarding the write. A file's permissions carry over.
- **READ and WRITE are positional**: a READ past the end is short, 0 at the
  end, FOR past it; a WRITE leaving a hole, or ending past 2^32 - 1, is FOR.
- **CLOSE** with `<1>` sets the file's modification time before the rename;
  its reply gives the length and mtime the host then has, which the Lisp
  side takes as the creation date (a host may keep times to 2 s). With
  `<0>` the temporary file is removed and the name left as it was.
- **DIRECTORY** gives records, packed from B's start, whole words each:
  word 0 `<7:0>` the name's length, `<15:8>` the record's words (3 and the
  name's), `<16>` a directory, `<17>` on a read-only mount, `<18>` 2^32 bytes
  or more; word 1 the length (0 for a directory, FFFFFFFF too large); word 2
  the mtime; then the name. They are sorted bytewise; dot files are listed;
  `.` and `..`, the temporary files, a symlink that leaves its mount, a name
  the rules below refuse, and anything neither file nor directory are not.
  The cookie is the index of the next entry, and each call lists afresh.
  A missing directory is DNF and a file WKF.
- **COMPLETE** matches the entries DIRECTORY would list whose names begin
  with the prefix, case kept; B gets their longest common beginning, word 6
  their number, and `<2>` (with `<3>` for a directory) says one is exactly
  that. No match is count 0 and matches 0.
- **RENAME never overwrites** (REF), atomically on Linux
  (`renameat2`'s `RENAME_NOREPLACE`), and never across mounts (RAD).
- **CREATE-DIRECTORY** makes one level: a missing parent is DNF.
- **LOG** writes `log: ` and the line on muir's standard error, a byte
  outside 040-176 as a backslash and three octal digits.

**Names** are absolute paths of bytes, at most 1,024, a single trailing `/`
allowed; each component 1 to 255 bytes in 040-176 other than `/`, and not
`.` or `..`. Anything else is IPS, and so is a name the host's file system
refuses (EINVAL). Case is exact: a name that matches only with case ignored
is not found. muir checks the spelling against the folder when the host
finds a name's case-flipped twin as the same file, which a case-folding file
system does; **unverified** on one, muir's tests running on Linux's. A
symlink is followed when it resolves inside its mount's folder; one that
leaves it, or loops, is ACC.

**Statuses.**

| Code | | When |
|---|---|---|
| 0 | | done |
| 1 | FNF | the last component does not exist |
| 2 | DNF | a directory on the way does not exist or is a file; an unmounted name; DIRECTORY of a missing directory |
| 3 | FAE | OPEN write with if-exists error; CREATE-DIRECTORY on a file |
| 4 | REF | RENAME onto an existing name |
| 5 | ACC | the host refuses (EACCES, EPERM); a symlink leaving its mount, or a loop; a mount's own root deleted or renamed |
| 6 | ATF | a write under a read-only mount (and EROFS) |
| 7 | DAE | CREATE-DIRECTORY of an existing directory |
| 8 | DNE | DELETE of a directory not empty |
| 9 | NMR | the host is full (ENOSPC, EDQUOT) |
| 10 | IOD | OPEN read or write of a directory |
| 11 | WKF | neither file nor directory; a file of 2^32 bytes or more; DIRECTORY of a file |
| 12 | IPS | a name against the rules, or EINVAL |
| 13 | NER | all 64 handles open |
| 14 | UOP | an opcode not among the ten |
| 15 | DAT | the host's I/O error, and any error not named here |
| 16 | FOR | READ past the end; WRITE leaving a hole or past 2^32 - 1 |
| 17 | RAD | RENAME across mounts |
| 64 | | bad handle: not open, or the wrong kind |
| 65 | | bad buffer: off a line, over 65,536 bytes, or past main memory |
| 66 | | bad argument: flags out of range, a DIRECTORY buffer under 272 bytes, a LOG line over 1,024 bytes, a completion longer than B |

**Mounts** (`--file-root`). `--file-root <folder>[,ro]` is HOST's `/`, a
folder holding `sys/`, `site/` and `home/<user>/`, so that one path serves
`HOST:/sys/...`, `HOST:/site/...` and `HOST:/home/lispm/...`. `--file-root
<name>=<folder>[,ro]`, once for each name, is the top-level directory
`<name>`, over the default folder's entry of that name. A value is a named
mount when the text before its first `=` is a valid component. DIRECTORY of
`/` lists the top-level directories, each with its mount's read-only bit,
and every OPEN's reply carries it. A top-level name that is neither mounted
nor in the default folder is FNF itself and DNF below. With no default
folder `/` holds the mounts alone and is read-only (a name created there is
ATF); with no `--file-root` at all it is empty. A read-only mount answers
every OPEN write, DELETE, RENAME and CREATE-DIRECTORY with ATF, and a mount's
own root cannot be deleted or renamed (ACC). The start lists them. The CADR
refuses the flag.

**Disable is the reset.** 160 `<0>` from 1 to 0 drops the commands not yet
run, with no response and no memory written; closes every handle, a write
discarded and its temporary file removed; and sets the indexes and the
interrupt enable to 0. Quiet, 161 `<1>`, is up at once: muir's device is
never in the middle of a copy, and a command it ran stands, host effect and
all. The bases and sizes stay, and may be written before the next enable.
**Every machine reset disables it too**, `PROG.UNIBUS.RESET`, which
`RESET-MACHINE` pulses at every microcode start, clearing status `<2>` and
`<3>` with it. Power-on is disabled.

**Word 100 `<6>`** is up while the interrupt enable is on and a response is
waiting, 170 ≠ 171: a level, cleared by writing 171 up to 170 or by the
enable going off. Status `<8>` is the same ungated.

**Time.** A command completes at its due time: 20 us, and 100 us a KiB of
the two buffer lengths its entry names (a length over 65,536 counted as
65,536), rounded up to the ns, after the latest of its producer write, the
previous command's completion, and a response slot coming free. Both
constants are **unverified**: 20 us an estimate of the boards' round trip,
and 100 us a KiB block-disk's time for a block, itself an estimate, until
muir-fpga measures its path. At the due time, at the edge before the
processor's next microcycle and on both engines, the device reads the entry
and buffer A (and B for RENAME), does the host's operation, writes buffer B
and the response, and moves 165 and 170; main memory having been written
behind the processor, the whole memory cache is invalidated before the
processor's next memory cycle, as after a disk transfer. Nothing completes within the producer write,
and nothing in memory changes before 170 moves. Given the same host files a
run is the same.

**Coherence.** The device takes a new 164 only once the processor's write
buffer is empty: on `rtl` the command's time counts from when main memory
has done the last write the buffer took. `micro` has no write buffer.

**A checkpoint is refused** while a handle is open or a command is queued,
saying which: a host file and a command's host effect are outside the
machine. Otherwise it carries the registers and the rings' indexes; the
mounts are the flags'.

`tests/quux_file_device.rs` holds it: a scripted driver writing commands into
the rings against a scratch folder --- each register, the refused enable and
the index fault, the due time to the nanosecond and nothing before it, the
order, the wrap at 1, 2 and 256 entries and across 2^16, a full response
ring, the interrupt, the disable and the machine reset, every command and
its statuses, the write landing whole with its temporary file never listed,
the date, the mounts, a read-only mount unchanged, names, symlinks, LOG, the
checkpoint's refusal and its round trip --- and both engines running a
program that writes the command and its producer index and reads the
answer, `rtl`'s cache invalidated and its write buffer waited for.
`the_file_root_is_quux_s` in `tests/cli.rs` holds the flag. **Not
produced by a test:** NMR, DAT, and a WRITE past 2^32 - 1.

## The keyboard and the mouse

**QUUX's keyboard and mouse are on the register page** (contract Q3),
without the CADR's keyboard timing or the mouse's quadrature lines: the
CADR's I/O board takes its keyboard's words off a serial line at the
keyboard's clock and counts its mouse's lines on `KB CLK^`.

| Word | |
|---|---|
| 120 | keyboard status: `<0>` a key word is waiting, `<1>` the FIFO overflowed (a write clears it), `<8>` the interrupt enable |
| 121 | a read takes the oldest key word, the same 32-bit word `764100`/`764102` give together on the CADR; 0 when none is waiting |
| 122 | the mouse: `<11:0>` the X count, `<27:16>` the Y count, twelve bits each and wrapping as the CADR's counters do; `<14:12>` the buttons, as the CADR's Y register has them. A read clears 123's `<0>` |
| 123 | mouse status: `<0>` moved or a button changed since 122 was read, `<8>` the interrupt enable |

The FIFO holds 64 key words, the size of the microcode's own keyboard
buffer; a word past that is dropped and `<1>` set. A host adds its motion to
the counts, and `TRACK-MOUSE`, which takes the difference from last time and
sign-extends from bit 11, needs no change but where it reads. There is no
beeper: the CADR's `764110` is a toggle on the speaker line the microcode
flips once a half-wavelength, and QUUX leaves it out. muir's terminal
delivers to it on QUUX as it delivers to the I/O board on the CADR, and the
keyboard's boot word still boots. `tests/quux_input.rs` holds the FIFO's
order and its 64, the overflow, the counts and buttons, the interrupts, the
terminal's delivery, the boot word, a checkpoint and both engines' reads.

## No Unibus

**QUUX has no Unibus** (contract Q5). Every address of the CADR's Unibus
window, physical page 37000 and up, answers nothing on QUUX: a read or a
write fails as any empty address does, at once since Q7, the NXM bit set in
word 101, and changes nothing. With it go, on QUUX, the I/O board (its keyboard,
mouse, clocks and Chaosnet interface now QUUX's own, contracts Q1-Q4; its
serial port and general-purpose register dropped), the bus interface's
Unibus side (the adapter, the Unibus map, its buffers, WRITE-THROUGH, the
interrupt control and error status at `766040`-`766044`, the Unibus
interrupt), the diagnostic registers as the machine's own (the spy stays
the host's port: muir's prompt, the FPGA's AXI face), and the debug cable,
a Unibus master: `--debug-cable-listen`, `--debug-cable-connect` and
`--debug-in-process` are refused on QUUX, and a QUUX run has no DBGIN
connector. A QUUX debug design of its own is a later contract. The CADR
keeps all of it: CC and the two-machine lashup are its acceptance test.

muir-sys's microcode for Q5 makes no Unibus access over a boot and a while
at the listener, counted on `micro`. `tests/quux_no_unibus.rs` holds the
window's failures on the machine and through both engines' bus, the Unibus
interrupt not reaching QUUX, and the CADR's Unibus unchanged;
`quux_has_no_debug_cable` in `tests/cli.rs` the cable.

## The network

**QUUX's Chaosnet interface is on the register page** (contract Q4), off the
I/O board: its registers keep their order and their bits, word 140 + k being
the CADR's Unibus `764140` + 2k --- 140 the CSR, 141 my address (read) and
the write buffer (written), 142 the read buffer, 143 the bit count, 145
START --- sixteen bits each in the bottom of the word, and its interrupt is
word 100's `<5>`. muir's cable is the same: CHUDP to `ozd`,
`--chaos-address`, `--chaos-udp-peer`. An Ethernet interface behind the same
device is a later contract. `tests/quux_network.rs` holds each word to its
Unibus register and a STATUS request sent and answered through the page.

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

Microcode 323 knows the CADR's map, five-bit level-1 entries and 31
regions. Using QUUX's other 32 takes microcode written for QUUX. From its sources (System 1001's
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
