# Making a pack

Part of [the muir manual](manual.md).

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

    $ diskpack pack.img
    pack.img is not there: initialize makes one
    diskpack: initialize
    ...
    pack.img: 263245 blocks, 257 MiB
    diskpack: name MIT-LISPM-2
    diskpack: load MCR1
    MCR1: 12449 control store words, 114 blocks of 148, the rest zeroed
    diskpack: load-from LOD1 vendor/run/disk-sys-100-0.img LOD2
    LOD1: 24225 blocks of 24225 from vendor/run/disk-sys-100-0.img LOD2
    diskpack: quit

That pack boots. Most commands have a short form --- `i` for `initialize`, `p` for
`partition`, `m` for `modify`, `l` for `load`, `d` for `delete`, `c` for
`current` --- and `help` lists both; `dump`, `load-from`, `zero`, `drive`, `name`
and `comment` have none.

`initialize` lays out a Trident T-300, the drive a CADR's pack goes in:
`initialize <mcrs> <lods> <file MB>`, each count optional. Microcode partitions
are MIT's 148 blocks each, two if not given, and the paging area is MIT's 202
cylinders, which is already every page the machine can use. The bands share
what is left in whole cylinders, four if not given, and FILE is that many
megabytes at the end of the pack, none if not given. So a bare `initialize`
is two microloads and four bands of 153 cylinders, about 48 MiB each, filling
the pack exactly. A band is never bigger than the paging area, since a cold
boot copies the band into it; a layout that would make one bigger, or ask for
more than nine of either, is refused and no pack is made. It is the only drive
there is here: `muir` attaches every pack as a T-300, and a pack that could
not be booted would be a pack for nothing.

`partition <name> <size>` adds one, on the end where there is room; `modify`
changes one that is there, its size or its comment. A size is blocks, `75c` cylinders, `25%` of the pack,
or `rest` for what is left to the end of it. Nothing shrinks and nothing is
ever moved down: a partition that grows pushes the entries after it up, and
the tool says which moved --- what moves is the entry in the table, and
nothing on the pack moves at all, which is what makes it worth saying. `delete` is how a partition gives its blocks back:
it zeros them, then takes the entry out. `zero <partition>` zeros a partition
and leaves its entry where it is, with its comment emptied.

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
fetched. Every other partition takes its file as it stands. Either way the rest
of the partition is zeroed, and the
partition's comment becomes what went into it --- the file's name, or
`UCADR 323` for the built-in, which is MIT's own wording in that very field
on the release's own pack --- cut to the sixteen characters a descriptor holds,
because the file is not on the pack and the comment is the only place the
pack says what a partition is. `load-from` is the same move between two
packs --- `load-from LOD1 <pack> LOD2` --- with no
file in between; ours has to be at least as big as theirs, so that all of
what is copied lands, and what is past it in ours is zeroed.

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
