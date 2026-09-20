# `docs/`

The manual, and facts about the machine that are worth more than a comment:
what a piece of the CADR does, read from MIT's own files in `mit/`, and what
muir does against it. Facts only --- no history of the project and no plans.
Each document names the file and line every claim was read from, says which
claims a test holds, and marks what is not established as **unverified**,
with what would settle it.

`manual.md` is the entry: what muir does when you run it, every flag, and
what it is doing underneath. Nine companions carry the sections that stand
on their own, and each links back to it.

| Document | What it is |
|---|---|
| `manual.md` | The manual: installing and running muir, the start block, every command-line flag, flags in a file, the prompt, the terminal, what a run can write, the Chaosnet and the two-machine lashup |
| `diskpack.md` | Making a pack: `diskpack`, the second binary --- a fresh label, partitions added, grown, zeroed and deleted, and microcode and bands loaded and dumped |
| `engines.md` | How the engines work: `micro`, `rtl` and `chip`, what each computes on the way to the next microcycle, why the level matters, what each engine models board by board, and the debug cable's wire protocol |
| `machine.md` | The machine muir models: the processor and the bus interface, the two buses and every board on them, and what each of the models reaches outside muir. The diagram is on the front page |
| `netlists.md` | Where the netlists come from: what MIT left, how a board becomes a netlist, what each board is checked against, and the cold boot with every board a netlist |
| `sources.md` | Sources and attribution: what this is built on, how it was written, the license, and the name |
| `keyboard-boot.md` | The keyboard boot chord, Control-Meta-Control-Meta-Rubout: the firmware that detects it, the I/O board that decodes it, the wire it takes to the processor link by link, and what muir has of it |
| `tv.md` | The TV board: the SIMPLE TV and the LISPM TV, the one programming interface they share, the sync program that makes the raster, the color map, the color TV as System 100 drives it, and what muir has of it |
| `glass-tty.md` | The glass TTY: the screen read back as text over telnet and typed into the keyboard, the two fonts that are both called `CPTFONT`, which part of a screen taller than the window a person is shown, and what it cannot read |
| `chaosnet.md` | The Chaosnet board: MIT's interface as it shares the I/O board, the registers the software sees, the cable and its coding, whose turn it is to transmit, the abort signal in both its uses, and what muir has of it |
