# muir

A simulator of the MIT CADR Lisp Machine, down to the chips.

The documentation is a web page, kept up to date at
**[muir.metebalci.com](https://muir.metebalci.com)** --- the machine, the
engines, how the netlists are built and checked, and how to install it. This
file is the short version.

## Quick start

Install `rustup`, then open a new shell so `cargo` is on the path:

    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

Then build muir. The compiler version is pinned in `rust-toolchain.toml`,
so the first `cargo` command here installs it, and nothing else is
downloaded --- there are no crate dependencies.

    git clone https://github.com/metebalci/muir
    cd muir
    cargo build --release

Fetch the System 100 release, which is what the machine boots and is not
part of the repository. It lands in `vendor/`, every file checked against
its SHA-256 sum.

    tools/fetch-system-100.sh

Start a machine. That is the `rtl` engine running MIT's own boot PROM, with
the pack on the disk controller's cable. There is no default pack --- no
`--disk-pack` is a drive with no pack in it --- so it is named.

    target/release/muir --disk-pack vendor/run/disk-sys-100-0.img

It prints where its terminal is, and boots. Point any VNC viewer at that
address --- `vnc://127.0.0.1:5900` unless it says otherwise --- and you
have the display, keyboard and mouse, which are the machine's only way in
or out. RFB's `None` security is the only type offered, so no password.
The boot ends at a Lisp Listener.

[Install](#install) and [Running it](#running-it) below have the rest, and
so does [the install page](https://muir.metebalci.com/#install).

## What this is

The CADR is the second-generation MIT Lisp Machine, designed around 1978 by
Tom Knight, David Moon, Jack Holloway and Guy Steele. It is a 32-bit
microcoded processor with a 48-bit control store, and it became the basis for
the first commercial Lisp machines from LMI and Symbolics.

muir models it far enough down that the software written *for the hardware*
runs --- the MIT diagnostics, the console program CC, and two machines
lashed together with one debugging the other. That lashup is the acceptance
test, and it passes. Getting there meant rebuilding the machine board by board
from MIT's own drawings, which has an output of its own: netlists held pin for
pin to MIT's wire lists, part models checked against their datasheets, and a
record of which recovered files can be trusted.

Rust, no crate dependencies. The target is
[System 100](https://tumbleweed.nu/system-100-0-release/), microcode 323 ---
the restored last MIT release, recovered from TID/671 in the MIT Tapes of Tech
Square project. It will not run with earlier microcode.
[System 304](https://tumbleweed.nu/system-304-0-release/), the current release
of the line that continues it, boots here too, with its own pack and its own
Chaosnet numbers, and the console program CC runs on both: CC called
`MAKE-ARRAY` in a form System 304 removed, which kept it from loading there
until upstream rewrote the seven calls on 7 September 2026. The two-machine
lashup, the acceptance test, is run on the target's band.

## Three engines

One interface, three fidelities, the same architectural state at every
microcycle boundary. Run the fast one, check it against the slow one.

| | | Measured |
|---|---|---|
| `micro` | No timing model. It keeps a clock, so devices that read time see it pass, but nothing waits for an instant. | ~15x real |
| `rtl` | The timing model: every datapath signal on the machine's two-phase clock, and everything that is a matter of *when* --- bus waits and hangs, arbitration, timeouts, the debug cable. | ~2x real |
| `chip` | The parts themselves, resolved net by net from whatever drives each, with every board on both buses a netlist too. The reference: where it and `rtl` part, the drawings decide which is wrong. | ~1/4,000 |

Those figures are microcycles against the machine's own 145 ns microcycle,
and **the disk is outside them**. With the disk controller as a behavioural
model --- always on `micro` and `rtl`, and on `chip` unless it is given
`--disk-controller netlist` --- a transfer completes inside the store to
`START` and a seek takes no time. The hardware spent milliseconds on a seek
and spent them running the microcode's polling loop, so a 55 ms seek is
about 380,000 microcycles a CADR executes and muir does not. A program that
seeks finishes further ahead of the hardware than the table says, by an
amount that depends on the program.

On `--disk-controller netlist` it inverts: the drive takes its own time and
that polling loop runs through every gate on the board, so a seeking program
comes out slower than 1/4,000 rather than faster.

Any board can run as `rtl`'s behavioural model instead, one at a time, which
is faster: `--main-memory`, `--io-board`, `--tv` and `--disk-controller` take
`netlist` or `model`; the disk controller is the one that defaults to
`model`. `--main-memory-boards` sets how many 64K-word boards the machine
has, on every engine: 32 by default, up to 60.

## The netlists

A netlist here is the board itself --- every part, every pin, every wire. None
is drawn by hand or transcribed from a scan: each is extracted from MIT's own
SUDS drawings and then held, pin by pin, to MIT's own wire list for the same
board, and where the two disagree a test fails. Underneath, a part is a
behaviour checked against its datasheet, nets resolve four-valued so an
open-collector bus behaves as it does on the board, and parts are evaluated in
level order.

| Netlist | Board | Parts |
|---|---|---|
| `CADR.netlist` | processor and control store, two boards in the one file | 985 |
| `BUSINT.netlist` | bus interface | 176 |
| `CADRM.netlist` | main memory, 64K words a board, 32 of them by default for 2M words | 168 |
| `CADRIO.netlist` | I/O board: keyboard, mouse, clocks, the serial port, and the Chaosnet half of the same board | 173 |
| `CADRDC.netlist` | disk controller | 171 |
| `SIMPLETV.netlist` | the black-and-white TV, MIT's word for the screen | 171 |
| `LISPMTV.netlist` | colour TV, four- and eight-bit; replaced the SIMPLE TV in 1980 | 172 |

A part is one device at one board location --- the chips, the oscillators,
the delay lines, the switches, the LED digits --- and not the bypass
capacitors, resistor packs and busbars beside them, which nothing here
simulates. The whole machine with one memory board is 1,844 of them, and
7,052 with all thirty-two. The files list gates rather than devices, so they
run longer than that: the processor's 985 parts arrive as 1,243 `part`
records, a quad NAND being drawn four times under the one designator. Every
board but the SIMPLE TV has a count of MIT's own to answer to --- the parts
list of what was stuffed into each location, the census of how many packages
each type takes, or both --- and `tests/parts_mounted.rs` holds it to them.

The netlists themselves are committed in `data/` and are what the engines
read; nothing extracts them at build or test time. MIT's drawings are
committed too, in `mit/`, so re-extracting one needs no fetching --- that is
`tools/<board>-netlist.sh`, and only for changing or re-checking a file.
`data/README.md` says how to read a netlist against its sheet.

## Install

Needs a Rust toolchain and nothing else: the version is pinned in
`rust-toolchain.toml`, there are no crate dependencies, and everything the
build and the tests read is committed.

    git clone https://github.com/metebalci/muir
    cd muir
    cargo build --release
    cargo test                    # optional; passes with nothing fetched

The engines boot from a pack, which is not part of the repository:

    tools/fetch-system-100.sh     # the target
    tools/fetch-system-304.sh     # optional; the release after it

Each puts its pack and its system sources under `vendor/`, where the tests
look, and checks every file against its SHA-256 sum. They come from muir's
own GitHub releases, [`system-304-0`](https://github.com/metebalci/muir/releases/tag/system-304-0)
and [`system-100-0`](https://github.com/metebalci/muir/releases/tag/system-100-0),
so that the bytes the tests were written against stay the bytes: the packs
byte for byte as [upstream](https://tumbleweed.nu/lm-3/) publishes them, and
System 304's sources, which upstream ships no tarball of, built from the
project's own Fossil repository at the check-in that carries CC's rewritten
`MAKE-ARRAY` calls --- each script says exactly what it fetched and from
where. Everything in them is under the
AGPL, muir's own licence. Without them, everything needing a pack skips and
says so. Windows is untested; the scripts are POSIX shell, so use WSL.

The two packs are different machines and want different flags. System 100's
band is `MIT-LISPM-1`, whose host table puts it at 3050 with `MIT-OZ` at
3060, which is what `--chaos-address` defaults to; System 304's is
`AMS-LISPM-1` at 4401 with its file and time host `OZ` at 4403:

    muir --disk-pack vendor/run/disk-sys-100-0.img
    muir --disk-pack vendor/run/disk-sys-304-0.img --chaos-address 4401,4403

## Running it

The flags below are the ones a first run wants; `muir --help` lists every
one, and [the manual](https://muir.metebalci.com/manual.html) is the whole
of it --- every flag, the prompt, the file of flags, and how the engines
work.

    muir [--micro|--rtl|--chip] [--prom <file>]
         [--disk-pack <image>[,<unit>][,ro]]
         [--main-memory netlist|model] [--io-board netlist|model]
         [--tv netlist|model] [--tv-board simple-tv|lispm-tv]
         [--terminal [<endpoint>]]
         [--debug-in-process [--debuggee-disk-pack <image>[,<unit>][,ro]]
                             [--debuggee-terminal [<endpoint>]]]
         [--debug-cable-listen [<endpoint>]] [--debug-cable-connect [<endpoint>]]
         [--checkpoint <file>] [--resume <file>]
         [--stop-after <microcycles>] [--stop-at <pc>] [--stop-at-prom <pc>]

One engine at a time, `--rtl` by default. A run goes on until a stop, a
halt or ^C; `--stop-after` ends it after that many microcycles.

A machine that stops *itself* is held at the prompt and says so. `(si:%halt)`
in the band runs `HALT-CONS`, which under `ERRSTOP` drops `MACHRUN` with
`RUN` still set: the screen stops and no microcycle runs from there. Nothing
about stepping says so, so muir reads it off `FLAG-1` where a console would,
and `boot` presses the button that starts it again. `chip` holds the same
way, reading the nets those registers are buffered from.

Every engine boots MIT's own PROM, `sys/ubin/promh.mcr` --- one file across
the releases --- which is built in, so nothing under `vendor/` is needed to
start a machine.
`--prom <file>` runs another one instead, out of an MCR microcode file as
MIT's own is, and the start says how that file stands to MIT's: word for
word, or how many words apart. That last is worth having --- recovered
copies of the boot PROM are not all the same program, and nothing about a
copy announces which it is.

Every run serves a terminal: the display, keyboard and mouse over RFB, so
that any VNC viewer can work the machine, which has no other way in or out.
It is at `vnc://127.0.0.1:5900`, VNC's display :0, and the start says where
it is; a display already taken --- a second muir on the host --- moves it
up to the first free one. `--terminal` says where instead: a port, an
address or address:port. An address other than the loopback is worth
meaning: RFB's `None` security is the only type offered, so a viewer needs
no password. A port that is named there is bound as it stands, and the run
stops rather than serving a viewer somewhere it was not told to look.

The Lisp Machine keyboard is not a keyboard anyone has: 31 named keys and 11
shifting keys, against a viewer sending X11 keysyms from a PC or a Mac. What
a keysym means here is therefore a choice and not a fact, so it is a file ---
`--keyboard-mapping`, or `.muirkeys` beside `.muirrc` --- over a built-in
mapping that needs no configuration. `key <keysym> <key>` is one host key;
`prefix <keysym> <keysym> <key>` is one pressed after another, which is how
Greek, Top and the rest are reached on a keyboard with no spare keys for
them. The prompt's `keys` prints the mapping in force, and
`--keyboard-mapping-dump` writes it out as a file to edit:

    muir --keyboard-mapping-dump > my.keys
    $EDITOR my.keys
    muir --keyboard-mapping my.keys

`--debug-in-process` is the two-machine lashup, both machines in one process.
A second machine runs beside the one you started, both debug cables between
them, each machine's DBGOUT on the other's DBGIN, as MIT ran two CADRs. The
window and the stops are the first machine's; the second boots the same PROM
with no pack unless `--debuggee-disk-pack` names one, and its console belongs
to CC. Each machine has a Chaosnet of its own --- muir's cable carries one
machine, so the second gets its own cable with its own server on it, at the
first's addresses unless `--debuggee-chaos-address` gives it others. The two
cannot hear each other over it; the only wire between them is the debug
cable. The second machine's server answers `STATUS`, `TIME` and `UPTIME` but
no `FILE` unless `--debuggee-chaos-file-root` gives it a directory: two
servers rooted at one directory are two hosts sharing a filesystem, with
nothing between them to keep one from writing what the other is reading.
Both machines get a terminal, the second at the display above the first's,
or wherever `--debuggee-terminal` says. So you connect a viewer to
the machine you invoked and CC there debugs the other, and a second viewer
watches the other being debugged. `--debug-cable-listen` and `--debug-cable-connect` are the
same cable over TCP with one machine in each process, the listener being the
debuggee. Both take an endpoint the same way and meet at 127.0.0.1:7661
without one. The listener may be a `--chip` machine: the netlist board's
own DBGIN then answers the debugger, an event at a time, at the netlist's
pace.

`--tv-capture <gif>` records the display as the run goes: an animated GIF
of the rectangles that change, timed by the machine's clock, with the
machine's time at the left of a line below the screen and the wall clock,
the time of day, at the right (`--tv-capture-no-time` drops the line). In
the lashup it records both machines on one canvas, the debugger's screen at
the left and the debuggee's at the right, so that a frame is one instant on
both: in one process the two are one clock, and a GIF writes each frame's
length in centiseconds, which two files of the same run could not hold
together.

`--checkpoint <file>` writes the machine's whole state when the run stops,
and `--resume <file>` starts from it instead of booting: the engine that
wrote it, with the same pack under it. A pack is only ever read: a block
the machine writes is kept in memory for the run and goes into the
checkpoint, so the image stays as fetched and a checkpoint can be resumed
from any number of times. `micro` and `rtl` for now.

Flags that every run should have go in a file, one to a line: the flag,
then after a space whatever it takes, which is the rest of the line --- so
a path with a space in it needs no quotes --- and a line that is blank or
begins with `#` is a comment. The file is the one `-c`, `--config` names,
or `.muirrc` in the directory muir was run from, or `.muirrc` in the home
directory: the first of the three there, not all of them, so a file beside
the work is the whole of a run's flags rather than an addition to the home
one. A file named with `--config` must be there; the two looked for need
not be. It is read before the command line, so a flag given there replaces
the one in the file, and an engine named there replaces the file's. The
start says which flags came from the file, and which file it was.

The prompt is muir's own line on stdin while a machine runs on its own, on
any of the three engines, and a line typed at it is a command to muir, not
to the machine. On `chip` the registers and the memories are the parts'
own cells rather than arrays, so `reg`, the five memory dumps and
`checkpoint` say so there instead of answering; the rest work. `muir: ` is written while
the machine is held, which is when muir is waiting to be told what to do
next; a line typed while it runs is acted on all the same, there is just
no prompt in front of it. `hold` holds the machine, `continue` runs on,
`step [n]` runs so many microcycles and holds, `pc` says where it is,
`reg` writes every register, `amem`, `mmem`, `dmem`, `pdl` and `spc` dump
the machine's memories, `boot` presses the boot button, `screenshot
[file]` writes the screen as a PNG and `startcapture [file]` starts
recording the display to a GIF --- `ss` and `sc` for short, and without a
file they write `muir-yyyymmdd-hhmmss.png` and `.gif` in the current
directory --- `endcapture`, `ec`, writes the recording and stops it with
the machine left running, which the end of the run does otherwise ---
`info` says what the run is as the start did,
`checkpoint [file]` writes its state, `quit` ends the run, and `help` says
what each does; `continue`, `info`, `quit` and `help` are also `c`, `i`,
`q`, and `h` or `?`.
It reads a pipe, or a terminal muir is in the foreground of ---
down a pipe there is no `muir: `, only the answers. ^C holds the machine
at the prompt; ^C while held, or with no prompt to go on from, ends the
run as `quit` does, with the recording and the checkpoint written.
`muir --help` says the rest, with each flag's default.

A CADR comes up halted: `RUN` is clear and the clock does not reach the
datapath, and what starts it is the boot button, which presets `RUN` and
takes the machine to the bottom of the control store, where the boot PROM
is. muir presses that button for you at the start of a run.
`--no-auto-boot` leaves it unpressed --- the machine is then as it is when
the power comes on --- and the run starts held, so that the machine can be
looked at as it came up. `boot` at the prompt presses the button, and the
machine runs from that: the button is all that starts one, so `continue`
and `step` say as much and do nothing until it has been pressed. A hold
nothing can run on, stdin having ended, ends the run instead of sitting
there.

A dump writes four words to a line: the octal address, the words in hex,
and the four characters each word holds, low byte first, in the Lisp
Machine's character set. A line the same as the one above it is a `*`.
Addresses and counts are octal, as MIT writes them.

## Making a pack

`diskpack` is the second binary. A pack is a file of blocks with a label in
block 0 that says how the drive is formatted and what partitions are on it,
and the machine cannot make one: it boots from a pack, so there has to be one
before there is a machine. MIT's own program for the label is `EDIT-DISK-LABEL`
in `sys/io/dledit.lisp`, which runs on a booted machine and edits the label of
a pack already in a drive. This is that, with the making of the pack added and
the partition contents loaded and dumped.

    diskpack <image> [<command> ...]

The pack named is opened if it is there and started if it is not, and then
commands are read a line at a time; `help` lists them. Every command takes
effect when you type it --- the pack is a file with nothing running on it, so
there is no state to hold and no write to remember --- and `initialize` is
what makes the file, a T-300 being 257 MiB and sparse, so it costs what is
written into it rather than what it spans. A command whose table could not be
written is refused whole and changes nothing.

    $ diskpack mete.img
    mete.img is not there: initialize makes one
    diskpack: initialize
    ...
    mete.img: 263245 blocks, 257 MiB
    diskpack: name MIT-LISPM-2
    diskpack: load MCR1
    MCR1: 12449 control store words, 114 blocks of 148, the rest zeroed
    diskpack: load-from LOD1 vendor/run/disk-sys-100-0.img LOD1
    LOD1: 24225 blocks of 24225 from vendor/run/disk-sys-100-0.img LOD1
    diskpack: quit

That pack boots. Most commands have a short form --- `i` for `initialize`, `p` for
`partition`, `m` for `modify`, `l` for `load`, `d` for `delete`, `c` for
`current` --- and `help` lists both; `dump`, `load-from`, `drive`, `name` and
`comment` have none.

`initialize` lays out a Trident T-300, the drive a CADR's pack goes in, with
the partitions MIT's own `PACK-TYPES` gives it --- eight microcode partitions,
a paging area, eight bands and a file partition, which together fill the pack
exactly. It is the only drive there is here: `muir` attaches every pack as a
T-300, and a pack that could not be booted would be a pack for nothing.

`partition <name> <size>` adds one, on the end where there is room; `modify`
changes one that is there, its size or its comment. A size is blocks, `75c` cylinders, `25%` of the pack,
or `rest` for what is left to the end of it. Nothing shrinks and nothing is
ever moved down: a partition that grows pushes the entries after it up, and
the tool says which moved --- what moves is the entry in the table, and
nothing on the pack moves at all, which is what makes it worth saying. `delete` is how a partition gives its blocks back.

`current` prints the label's two names for its own partitions --- MIT's own
display gives them as "Current microload = MCR1, current virtual memory load
(band) = LOD2" --- and `current 2` sets the band to `LOD2`, a number meaning a
band as it does in MIT's own `SET-CURRENT-BAND`. A name works in place of the
number and says which of the two words it goes in by what it is called, so a
microload is written out: `current MCR1`. A number means a band wherever a
partition is asked for, in `delete 3` and `load 1 band.dump` as much as here.

`load` puts a file in a partition and `dump` takes one out. What an `MCR`
partition holds is microcode, so it takes a microcode file and writes it the
way the machine reads one: a microcode file holds each 32-bit word as two
16-bit pieces, the high one first, and the disk holds a word low half first,
so every word's halves are swapped on the way in and the rest of the partition
is zeroed. With no file it takes microcode 323 itself, which is built in ---
`mit/sys/ubin/ucadr.mcr`, the release's own file, committed beside the boot
PROM's --- so a pack that boots can be made from a fresh checkout with nothing
fetched. Every other partition takes its file as it stands. Either way the
partition's comment becomes what went into it --- the file's name, or
`UCADR 323` for the built-in, which is MIT's own wording in that very field
on both releases' packs --- cut to the sixteen characters a descriptor holds,
because the file is not on the pack and the comment is the only place the
pack says what a partition is. `load-from` is the same move between two
packs --- `load-from LOD1 <pack> LOD1` --- with no
file in between; ours has to be at least as big as theirs, so that all of
what is copied lands.

MIT's own editor holds the label and writes it on `^W`, and this does not,
because the reason for the two states is not here: the label MIT is editing
belongs to a drive on a running machine, where half a label is a machine that
cannot find its bands. Nothing is running on this one.

Commands can be piped in as well as typed, and a line that is no command then
stops the run rather than carrying on to the next one, so a script that builds
a pack cannot half-build it and report success. One command can also go on the
command line --- `diskpack made.img load LOD1 lod1.dump` --- which is where
the shell completes a file name for you, the prompt having no completion of
its own and muir no dependency to give it one. It does the same thing there as
it does at the prompt.

    $ diskpack made.img load MCR2
    MCR2: 12449 control store words, 114 blocks of 148, the rest zeroed

## The Chaosnet server

The machine's Chaosnet interface is real --- it is half the I/O board, and on
`chip` it is that board's netlist. What is on the other end of the cable is
not a machine but a **Chaosnet server**: an address that answers contact names, so
the band has something to talk to. It is `MIT-OZ` at `3060` and the machine is
`MIT-LISPM-1` at `3050`, octal, which are the band's own numbers from
`sys/site/hosts.text`; `--chaos-address <this>[,<server>]` changes
them.

| Contact | |
|---|---|
| `STATUS` | the host's name and its subnet meters. AIM-628 §5.1 requires every node to answer it, and it is how a machine decides another is up: `HOST-UP-P` asks for nothing else, and `(hostat)` prints what comes back. The meters are zero, which is the true count for a host with no interface |
| `TIME` | the universal time in four bytes, least significant first --- seconds since midnight GMT, 1 January 1900. The tests fix it, so two runs of a boot do the same work |
| `UPTIME` | seconds since the server came up, by the ether's clock |
| `FILE` | the file protocol the band loads `SYS:` over --- a control connection and data connections beside it. Served only when `--chaos-file-root` gives it a directory, which it serves as the server's `/` |

`FILE` is what makes the acceptance test possible: CC is not in the band, so
`sys/cc/*.qfasl` has to come over the network, the way it would have on a real
machine.

## Layout

    mit/       MIT's own files, unmodified: the drawings and wire lists of
               every board modelled here, and a snapshot of System 100's own
               sys tree, which the boot PROM comes from. mit/README.md
    data/      what is made from mit/ by a script in tools/, each
               cross-checked or labelled: the seven netlists, the disk
               controller's microcode, the cable tables. data/README.md
    src/       the simulator, the muir binary and diskpack
    tests/     the checks
    examples/  development tools; none is part of the simulator.
               examples/README.md
    tools/     fetching, and one script per board to re-extract a netlist
    site/      the front page and the manual, published by
               .github/workflows/pages.yml
    vendor/    fetched material, never committed

## On the source material

It is not trustworthy, and that shapes the whole project. The recovered CADR
files mix primary MIT originals, hand conversions made decades later, and
reconstructions from scans. Found so far: a boot PROM calling itself version 9
that is a different build from System 100's, agreeing for 0o262 words and then
diverging in 214 of 454; a PROM image parity-encoded rather than raw, so
reading its bit 46 as the statistics bit is silently wrong; and MIT
documentation giving the wrong bit positions for the DISPATCH level-2 map
bits.

So: never take a single file as authority. Anything load-bearing is
cross-checked against a second copy that arrived by a different route, and a
test enforces the agreement. Where a fact is believed but unchecked, the code
depending on it says **unverified**, and why.

## Related projects

| | |
|---|---|
| [usim](https://tumbleweed.nu/r/usim/) | the CADR emulator; C; boots Lisp |
| [uhdl](https://tumbleweed.nu/r/uhdl/) | CADR in Verilog for FPGA |
| [cpus-caddr](https://github.com/lisper/cpus-caddr) | Brad Parker's Verilog CADR |
| [ams/cadr4](https://github.com/ams/cadr4) | faithful VHDL CADR, with a TTL part library |
| [LM-3](https://tumbleweed.nu/lm-3/) | the system software releases |
| [cbridge](https://github.com/bictorv/chaosnet-bridge) | Chaosnet, for when networking matters |

## How it was written

muir is implemented entirely by [Claude Code](https://claude.com/claude-code),
on Anthropic's Opus 5 and Fable 5.1 models.

## Licence

Copyright (C) 2026 Mete Balci.

Free software under the **GNU Affero General Public License**, version 3 or,
at your option, any later version. Distributed in the hope that it will be
useful, but WITHOUT ANY WARRANTY; see `LICENSE` for the full text. AGPL
matches `ams/cadr4` and the System 100 release. The MIT licence was
considered and rejected: it would permit a closed-source derivative, and not
permitting one is the point.

Two parts of the tree are not this project's work throughout and are not
covered by that copyright. `mit/` is MIT's, written at the AI Laboratory
between 1977 and 1981, with `mit/sys/` a snapshot of the System 100 release
under the release's own AGPL; `mit/README.md` says what is claimed there,
which is nothing, and `data/README.md` says what is whose in the files made
from it. `tools/soap4/` is the reader for MIT's drawings, C that came here
from `ams/cadr4` under the AGPL: `soap4.c` follows Brad Parker's `soap.c` of
2004 and `unpack4.c` John Wilson's `unpack.c` of 1993, each keeping its
original header and naming its original author in its copyright line.
Neither original carries a licence --- not in the copies here and not in
Brad Parker's own repository they were published from --- so the AGPL covers
the work done on them and cannot make a grant for what came before it. That
is the one open licensing question in this repository, and
`tools/README.md` has the detail.
