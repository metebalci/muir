# How the engines work

Part of [the muir manual](manual.md).

The three engines differ in what they actually compute on the way to the
next microcycle --- no timing model, the timing model, the full netlist.
This is what that means in each case, and it is the part of the project most
easily mistaken for something it is not. Last is the wire protocol that runs
two of them against each other over a network, which is a scheduling problem
before it is a networking one.

## The three engines

An **engine** is an implementation of the machine at one fidelity level. All
three sit behind one interface and reach the same architectural state at
every microcycle boundary, so you can run the fast one and check it against
the slow one. They differ in how much of the hardware they compute on the
way, and in where that state sits: `micro` and `rtl` hold the scratchpads
and the control store as arrays, `chip` holds them in the RAM chips' own
cells, and the cosimulation compares the two word for word.

The shortest way to tell them apart is by time.

**`micro` has no timing model.** It has a clock --- it adds up the
microcycle periods, so that the devices that read time see it pass --- but
nothing in it happens at an instant or waits for one. A bus cycle completes
at once; the word is in `MD` two instructions later whatever the memory is
doing. It is the engine for what needs no timing: unless a program reaches
the hardware directly, or the debugger and the diagnostic bus are in play,
`micro` is enough. It is also much the fastest.

**`rtl` is the timing model.** Every datapath signal on the machine's own
two-phase clock, and everything that is a matter of *when*: the bus's waits
and hangs, the interface's arbitration and timeouts, a device answering
late, the debug cable to a second machine. The boards behind the cables are
behavioral models, timed to the nanosecond against the netlist boards.
Anything `micro` cannot do, `rtl` can; it is slower than `micro` and still
faster than the machine itself.

**`chip` is the full netlist.** The parts themselves, resolved net by net
from what is actually driving each, and every board on both buses a netlist
too. It is the reference: where it and `rtl` disagree, the drawings decide
which is wrong, and it is usually `rtl`. It is also much the slowest ---
approximately 5,000 times slower than the hardware.

**Those figures count microcycles, and the disk is not in them.** With the
disk controller as a behavioral model --- which is what `micro` and `rtl`
always use, and what `chip` uses when told `--disk-controller model` --- a
transfer completes inside the instruction that starts it and a seek takes no
time at all. A real CADR spent milliseconds on a seek, and spent them
executing the microcode's polling loop: a 55 ms seek is some 380,000
microcycles that produce nothing. muir does not execute them. So on anything
that touches the disk muir gains twice over the hardware, once per
microcycle and once by having far fewer of them, and by how much depends
entirely on how much the program seeks. It is not in the numbers above and
it is not worth guessing at.

`chip` runs the netlist controller, and it goes the other way. There the
drive takes its own time over every block, so those 380,000 polling
microcycles are executed for real, through every gate on the board, and a
program that waits on the disk is slower than the 5,000 figure rather than
faster. **A run that touches no pack pays about 3% for that; a boot pays
days.** System 100 came up this way on 12 September 2026, after 2 days 14
hours. `--disk-controller model` is how a `chip` run that does not care
about the disk is made quick --- and `rtl`, which runs the models throughout
at about twice the hardware, is the engine for ordinary use.

```text
micro    +---------------------------------------------+
         | one evaluation: decode, execute, write back |
         +---------------------------------------------+
         60 M/s --- about 9x the machine

rtl      +---------------------------+    +---------------------------+
         | read phase                |--->| write phase               |
         | sources drive, the ALU    |    | destinations take what    |
         | settles                   |    | they caught               |
         +---------------------------+    +---------------------------+
         16 M/s --- about 2x the machine

chip     +------------------+    +-----------------------------------+
         | clock generator  |--->| 985 parts, settled in level order |
         +------------------+    +-----------------------------------+
         1,400/s --- about 1/5,000 of the machine
```

*One microinstruction through three engines: micro computes one evaluation,
rtl the read and write phases of every datapath signal, chip the 985 parts
in level order.*

The same microinstruction, through each engine. The rates are the worst
case, with everything fitted: the color TV on every engine, and on `chip`
MIT's disk controller with its multiplexor as well, every board a netlist
--- `cargo run --release --example benchmark -- --everything` on one core
of an Apple M4, rounded down. Run it yourself and you should see about
these; without the second display board `chip` runs about 2,000/s and the
other two the same. The machine's own microcycle is 145
ns at normal speed, read off the delay-line taps, so the hardware runs 6.9 M
of them a second.

| Engine | What it computes | Rate |
|---|---|---|
| micro | No timing model. One step per microinstruction, on the machine's own periods but without its bus waits. Fast enough to get somewhere and see what happens. | 60 M/s |
| rtl | The timing model. Every datapath signal, on the machine's own two-phase clock, with the real memory cycle in nanoseconds: `MEMSTART`, `MEMRQ`, `MBUSY`, the acknowledgement back, and the clock held off by `WAIT` and `HANG` meanwhile. | 16 M/s |
| chip | The full netlist. The parts themselves: 985 of them on the processor and control-store boards, sorted into levels, on four-valued nets, with the bus interface, main memory, the I/O board and the TV as netlists behind the cables. This is the reference the other two are checked against. | 1,400/s |

> The 1,400/s above is the `chip` engine with every board a netlist ---
> the processor, the interface, main memory, the I/O board, both display
> boards, and MIT's disk controller with the multiplexor on its cable ---
> on two programs that touch no pack, so the disk boards only sit on the
> bus. Without the color TV it is about 2,000/s; a run that reads a pack
> pays the drive's real milliseconds on top. Any board can be run instead
> as the behavioral model the `rtl` engine uses, one at a time, which is
> faster. Which model each engine uses for each part of the machine, and
> which flag changes it, is [the table
> below](#what-each-engine-models).

## What each engine models

The engine chooses how the processor is computed; it also chooses what
stands in for every other board on the backplane. A netlist board is MIT's
own wire list run package by package. A model is a behavioral one written
from the drawings --- on `rtl` those are timed to the nanosecond and held
against the netlist board, and on `micro` they run without the clock.

**The choice between the two exists only on `chip`.** `--main-memory`,
`--io-board`, `--tv` and `--disk-controller` are refused to the other
engines with a warning rather than obeyed quietly, so `micro` and `rtl` have
one answer a row and `chip` has two. **A bare `chip` run is an all-netlist
machine**: main memory, the I/O board, the display and the disk controller
all come up as MIT's boards. Each row below marks which side it starts on.

**`--tv-board` is not one of those:** it says which display board the
machine has, and it is every engine's. One model serves either board, so the
flag chooses the netlist `chip` builds the backplane with and the board the
model answers as everywhere else. `--color-tv` is every engine's too, and it
takes the same two words as the board flags: `model` anywhere, `netlist` on
`chip` alone, and the bare flag the netlist on `chip` and the model
elsewhere.

**The mixed machines are instruments, not configurations.** A run meant to
work the machine wants `rtl`, which runs the models throughout at about
twice the hardware and is the engine for ordinary use; whoever asks for
`chip` is asking for the gates, and asking for one board as its model is how
that board is taken out of the picture while something else is under
investigation --- the two engines are held to each other at every
microcycle, so where a board and its model part one of them is wrong, and
running the model in its place is how you find out which. The other use is
time: `--disk-controller model` gives a `chip` run the model's instant
transfers in place of the drive's real milliseconds, which is nearly five
times fewer microcycles through the boot PROM alone, measured above.

A row that gives one answer for more than one of the columns writes it
once, and the columns after it read *as at left*.

| Part | micro | rtl | chip, model board | chip, netlist board |
|---|---|---|---|---|
| Processor and control store | Model, one step a microinstruction. The architecturally visible pipeline --- the two-deep fetch, the inhibited cycle after a jump, the `OA` merge, the two-cycle memory data delay --- and not the clock phases. | Model, two phases a microcycle on the board's own clock, with the delay-line taps that end the read phase. | `CADR.netlist`, always. There is no flag that makes the processor a model, which is why parity below is real on `chip` whatever the other boards are. | as at left |
| Bus interface | None. A physical address is answered by the machine directly, and a memory cycle is charged a flat 460 ns. | Model, timed: the arbitration, `MEMSTART` to `-LOADMD`, the Unibus and Xbus cycles, and the timeout. | `BUSINT.netlist`, always. | as at left |
| Main memory | Words, no timing. | The same words, with a twin of each board carrying its refresh and its answer time. | `--main-memory model` puts `rtl`'s model behind the same bus. | **Default.** `CADRM.netlist`. `--main-memory-boards` sets how many, on every engine. |
| I/O board | Model, advanced every microcycle on both: the keyboard and its cable word, the mouse's quadrature, the microsecond clock, the 2651 serial port a character at a time, and the Chaosnet interface with its turn timer. | as at left | `--io-board model`, for the same model. | **Default.** `CADRIO.netlist`. The mains line the 60 Hz counter runs on is undriven here, where the model counts. |
| Disk controller | Model: the command PROM's sectors, the CCW chain and the status word. | as at left | `--disk-controller model`, for the same model as at left. How a run that does not care about the disk is made quick, and what `--main-memory model` leaves. | **Default.** `CADRDC.netlist`, which takes the drive's real milliseconds over every block: about 3% on a run that touches no pack, and days on one that boots. |
| Disk drive | Model: one Trident, its pack, head position and the conditions the controller reports. | as at left | The same model. | The drive as its cables carry it --- the bus and tag lines, index and sector pulses, and the read and write clocks. |
| Display | Model: a frame buffer, a mode register and the sync program the board runs, which is where the frame, the vertical flag and the sync bits come from. Either board, as `--tv-board` says. | as at left | `--tv model`, for the same model, on the board `--tv-board` names. | **Default.** `SIMPLETV.netlist`, or `--tv-board lispm-tv` for `LISPMTV.netlist`, the four- and eight-bit board that replaced it in 1980. |
| Color TV | Not fitted unless `--color-tv` asks for it, and then the same model at the color addresses: the buffer at `17200000` and the registers at `17377750`. | as at left | `--color-tv model`, for that same model. | `--color-tv`, for `LISPMTV.netlist` again --- one more board on the backplane, wrapped to the color addresses instead of the main screen's. The model is fitted beside it and every write is mirrored in, so the picture is read off the model whichever board drew it. |
| Debug cable | Neither end: no timing model, so no instants to promise. | Either end. | The debuggee, on the interface board's own DBGIN connector. | as at left |
| Disk multiplexor | Not fitted, and not wanted: the model controller has addressed eight units all along. | as at left | as at left | `--disk-multiplexor`, for `DM.netlist`, which gives the netlist controller eight drive ports instead of its one; without it, unit 0 alone. Off by default. There is no model of it: the drive cable it switches has no second source to check one against. |
| Parity | Not modeled. The ten parity-error flags and the memory-parity trap never fire. | as at left | Real either way, the processor being a netlist on this engine: the parity RAMs and the 74S280 and 93S48 checkers are packages like any other, so the board computes parity whether or not anything is wrong with it. | as at left |

> Parity is left out of the two models deliberately. A parity bit detects
> a chip having failed, and the words these engines hold are values that
> cannot fail; a check written over them could not fire, and MIT's own
> writers --- `CC-WRITE-D-MEM`, PRAID's `P-D-MEM-D` --- always compute the
> bit correctly. What CC would do with a flag, it can still do on `chip`.

## Why the level matters

A model that steps once per microinstruction is self-consistent, agrees with
itself, and boots Lisp. It is still not the machine, and one instruction
shows the gap.

```text
micro --- one step per microinstruction

+------------------+------------------+
| microcycle 1     | 2                |
+------------------+------------------+
| WRITE-I-MEM      | next instruction |
+------------------+------------------+

rtl, chip --- what the board actually spends

+------------------+------------------+------------------+------------------+
| microcycle 1     | 2                | 3                | 4                |
+------------------+------------------+------------------+------------------+
| WRITE-I-MEM      | nop              | nop              | next instruction |
|                  | the              | IWRITED's N,     |                  |
|                  | instruction's N  | POPJ             |                  |
+------------------+------------------+------------------+------------------+
```

*A control-store write costs one microcycle in a model that special-cases
it, and three on the board: the instruction plus two dead cycles.*

`WRITE-I-MEM` --- a JUMP with both `IR<8>` and `IR<9>` set --- writes the
control store. On the board it costs two microcycles beyond itself:
`IWRITED` is the registered `IWRITE`, and in the following microcycle it
drives both `N` and `POPJ`, so one cycle dies for the instruction's own `N`
and another for `IWRITED`'s, with the `POPJ` bringing the PC back.

The boot PROM notices. `CLEAR-I-MEMORY` writes all 16,384 words of the
control store, so the shortcut is taken 16,384 times, and two microcycles
apiece is 32,768 of them --- six per cent of the boot, out of one
instruction.

| To the first disk read | Executed instructions | Microcycles |
|---|---|---|
| `micro`, one step per microinstruction | 416,736 | 505,079 |
| `rtl`, following the drawings | 416,736 | 537,848 |

> The same instructions either way, so nothing an instruction trace can
> ever see. A cycle count built on the first number is simply wrong, and
> an instruction trace cannot reveal it, because the cycles that were
> inhibited leave no instruction behind.

## What register-transfer level means here

RTL is register-transfer level: the machine described as registers and the
logic that moves data between them, clocked. The `rtl` engine is that
written out directly --- what drives each bus, what the ALU computes, which
register takes what. It has no notion of a package, a pin or a net.

A microcycle there is two phases, because that is what the board has.
`-CLK0` is `-TPCLK AND MACHRUN` at CLOCK2 1D10, so every edge-triggered
register takes one edge per microcycle, at the cycle boundary. Between edges
the clock is a level and the board reads it as one. With `CLK` high the
source multiplexers select the instruction's own source fields, the 74S373
latches follow the memories, `-TSE1..4` let the sources drive, and the ALU
settles. With `CLK` low the multiplexers select `WADR`, the latches hold
what they caught, and `-WP1..4` write.

Time is real even though the parts are not. A memory cycle runs `MEMSTART`,
`MEMRQ`, `MBUSY`, the acknowledgement back and `-LOADMD` 50 ns after it,
with the clock held off by `WAIT` and `HANG` meanwhile --- so a stall costs
nanoseconds without costing a microcycle.

## The chip engine

Every other engine collapses something. This one does not: it takes the
packages out of the netlist, gives each one the behavior recorded for its
type, and resolves every net from what is actually driving it. It is the
slowest of the three and the one the other two are checked against, so it is
allowed no opinions.

A pin there is electrical rather than logical --- totem-pole, open
collector, open emitter, tri-state, a resistor pack, or passive. That is
what lets an open-collector net behave like one, with the pull-up packs as
the weak drivers a real one needs, and it is what makes a wrong pinout
surface as two totem-pole outputs fighting over a net instead of as quietly
wrong logic much later. The pinouts are read off the manufacturers'
datasheets, and a test checks all 1,243 gate records across the processor's
2,782 nets for exactly that conflict.

The ALU shows the difference most clearly. `rtl` computes the whole 33-bit
array at once from the datasheet's function table. `chip` runs the nine
74S181 slices and the three 74S182 carry-lookaheads as parts, gate for gate.
The two have to agree.

## Why this is not a VHDL simulator

A VHDL CADR exists --- `ams/cadr4` --- and running it under a simulator such
as nvc is a different thing from what `chip` does.

A VHDL simulator is general. It elaborates any design into processes and
signals, drives them from an event queue, advances time to the next event,
and resolves each signal through a resolution function. `chip` simulates one
netlist and exploits it. Taken pin by pin --- which output each input
actually reaches --- the CADR is acyclic, so its gates are sorted into
levels once and then evaluated in that order, with no event queue at all.
Taken package-wide the same dependencies appear to close loops the hardware
does not have; the only real cycles are in the clock generator, which is
modeled directly instead. Each pass recomputes only the gates something
moved under.

Both run zero-delay: neither charges a gate for its propagation time, and in
muir the nanoseconds come from the clock and bus models instead. So the two
should agree, and that has been measured rather than assumed. Running
cadr4's VHDL of the same three boards under nvc against `chip`, on the same
PROM, every named net at every instruction: over the boot's first
millisecond 2,014 nets agree at every sample, 830 differ only while the VHDL
is still uninitialized, and 15 are the clock taps the behavioral clock
leaves undriven. None agrees for a while and then parts. Over the whole boot
PROM the two are on the same PC at every microcycle, and `chip` gets there
about four times faster in wall time and ten times per core.

## The debug cable over TCP

Two machines on a debug cable in one process are one clock, and the cable is
a function call. `--debug-cable-listen` and `--debug-cable-connect` put the
two machines in separate programs on a TCP connection, and then they are two
clocks that have to be made into one. The protocol below is how.

**The only time on the wire is machine time.** Every instant sent is the
sending machine's own nanosecond count, and no message carries a wall clock,
a heartbeat or a timeout of its own. It could not: the debugger's interface
gives a debug cycle up 11,050 ns after the grant --- thirteen intervals of
the 74LS124 at REQTIM 0A01, the REQTIM PROM's second table --- and that
instant has to fall in the same place whether the other machine answered in
a microsecond of wall time or a minute of it. A slow peer must make the
debugger wait, not time out.

So neither end is ever allowed to run into time the other has not accounted
for. **Each side tells the other the earliest instant it could do anything,
and runs only as far as the other's last such promise.** The debugger
promises the earliest a request or a release can appear on the cable ---
with a cycle out, that is the timeout's release; idle, 440 ns. The debuggee
promises the earliest `DEBUG ACK` can rise for the request it is holding,
and promises nothing at all when the cable is quiet, having nothing to say
until it is asked. A side steps only while the step cannot carry it past the
other's promise, and blocks on a message when it cannot step. Since the
debugger's promise is always a finite instant and the debuggee's is only
finite while a request is out, a quiet cable always leaves exactly one side
free to move, and the two cannot sit waiting on each other.

The messages are fixed 24-byte frames, a one-byte tag then little-endian
fields. Fixed, because a frame that is always one length needs neither a
length prefix nor a parser: **either end may be a program that is not
muir**, and that is the point of writing the protocol down rather than
leaving it as whatever two copies of the same binary happen to agree on.

| Tag | Message | Sent by | What it says |
|---|---|---|---|
| 1 | Request | debugger | `-DEBUG IN REQ` down at an instant, with the strobe, the direction, `DBD<15:0>`, and how long after the acknowledgement the debugger will lift it |
| 2 | Release | debugger | the request lifted unacknowledged: the debugger's own timeout, which crosses the wire as an ordinary event and not as a failure |
| 3 | Ack | debuggee | `DEBUG ACK` up at an instant, with the word the board drove on `DBD` if it drove one |
| 4 | Promise | either | nothing from this side before this instant --- and how many of the other side's events it had taken when it said so |
| 5 | Done | either | this side has run as far as it will |

The cable carries the request, the release and the acknowledgement, and not
the lift of the request. That is why a request says how long it will be
held: the debuggee's end raises `DEBUG ACK`, then lifts `-DEBUG IN REQ`
itself at the instant the debugger's Unibus cycle would have.

A promise can go stale, and that is what the count in it is for. Each side
runs ahead on two assumptions that save a round trip --- having sent a
request, the debugger takes the answer to be due no sooner than the
request's own instant, a register strobe being acknowledged as it arrives;
having sent an acknowledgement, the debuggee stops trusting the debugger's
last promise, since it collapses from the timeout's release to the next
cycle's earliest request the moment the answer lands. A promise the other
side made before it had the event that changed it is therefore wrong, and
arrives after it. Each side counts the events it has sent that move the
other's promise --- the debugger's requests, the debuggee's acknowledgements
--- and every promise carries the count it was made against; one whose count
is behind is dropped, and the fresh one is a frame away.

Opening is a bare TCP connection with no handshake, no version and no magic
number: the listener is the debuggee, the connecting side is the debugger,
and the first frame either sends is a promise. Each end reads on a thread of
its own into a queue of 256 frames, about 6 KB of cable; full, it stops
reading and the peer's writes wait on the socket, so a peer running faster
is held to the slower machine's pace rather than buffered without end.
Closing is symmetric: each side sends an unbounded promise and then `Done`,
and waits for the other's, so that neither is left blocked on a promise that
will never be revised.

A frame the cable cannot take ends the run with an error naming what was
sent, rather than killing the process or trusting it: a message from the
wrong side, a second request while one is already on the cable, or an
instant behind the receiver's own by more than one generator cycle. An
instant *ahead* is not an error --- it is held and put on the connector when
the receiving machine reaches it, which is what lets an `rtl` debugger drive
a netlist debuggee that runs thousands of times slower.

`micro` can be neither end, having no timing model and so no instants to
promise. `rtl` can be either. `chip` can be the debuggee, where the request
goes on the interface board's real DBGIN connector at its instant and the
acknowledgement comes back as of the instant the board raised it. The
default endpoint is `127.0.0.1:7661`, the port chosen for DBGOUT's Unibus
address `766100`.

> There is no authentication and no encryption on this connection, and a
> peer that speaks it can halt the machine at the other end, read its
> memory and reset it --- that is what a debug cable is for. Hence the
> loopback default; an address other than the loopback is worth meaning.
> The protocol is held to the in-process cable rather than to itself: the
> same two machines are run both ways, and the debuggee has to halt at the
> same PC and the debugger read back the same words.
