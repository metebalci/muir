# Sources and attribution

Part of [the muir manual](manual.md).

Almost nothing here is a first-hand invention. The machine, the drawings,
the wire lists, the microcode and the diagnostics are MIT's, recovered by
other people's work over decades. The recovered material is also not
uniformly trustworthy --- it mixes primary MIT files with hand conversions
and reconstructions from scans --- so nothing load-bearing is taken from a
single file: anything that matters is cross-checked against a second copy
that reached us by a different route, and a test enforces the agreement.
Every netlist in the repository is held pin for pin to MIT's own wire list
for the same board, and where a fact is believed but unchecked the code that
depends on it says so.

- **The CADR** --- MIT AI Laboratory, around 1978. Tom Knight, David Moon,
  Jack Holloway and Guy Steele.

- **Tapes of Tech Square** --- The MIT project that read the AI Lab and LCS
  backup tapes, and the collection at MIT Libraries' Department of Distinctive
  Collections that holds them. System 100 was pulled from TID/671 among these.
  Recovered material is published as [MITDDC](https://github.com/MITDDC).

- **System 100** --- The restored last MIT release, from the [System 100
  release](https://tumbleweed.nu/system-100-0-release/).

- **usim** --- [A CADR emulator in C](https://tumbleweed.nu/r/usim/), by
  Brad Parker and now maintained by Alfred M. Szmidt.

- **ams/cadr4** --- [A faithful VHDL CADR](https://github.com/ams/cadr4)
  with a TTL part library, MIT's drawings and wire lists, and the SUDS
  readers. `soap4.c` from it is what reads the drawings into netlists here.

- **uhdl** --- [CADR in Verilog for FPGA](https://tumbleweed.nu/r/uhdl/). It
  carries the 2004 `CADR4.netlist`, an earlier extraction from the same MIT
  drawings.

- **cpus-caddr** --- [Brad Parker's Verilog
  CADR](https://github.com/lisper/cpus-caddr).

- **LM-3** --- [The Lisp Machine system software
  releases](https://tumbleweed.nu/lm-3/).

- **ozd** --- [The OZ daemon](https://github.com/metebalci/ozd): the file
  and time host a band calls, which muir is not --- a CADR had no such server
  in it. One host serves however many machines are on the cable.

- **muir-fpga** --- [The CADR in programmable
  logic](https://github.com/metebalci/muir-fpga), on a Zynq board with Linux
  on the Arm cores beside it. `--debug-cable-connect 0x<address>` is muir
  debugging that machine over the CADR debug cable, the cable's wires being a
  window of registers there.

- **cbridge** --- [A Chaosnet
  bridge](https://github.com/bictorv/chaosnet-bridge), for when networking
  matters here.

- **An overview of the machine** --- [CADR LISP Machine and CADR
  Processor](https://metebalci.com/blog/cadr-lisp-machine-and-cadr-processor/),
  written before this project: the processor, its pipeline and clock, virtual
  memory, the microinstruction and macroinstruction formats, and a
  bibliography of the MIT memos behind them. The shortest way in, if you are
  starting cold.

## How it was written

muir is implemented entirely by [Claude
Code](https://claude.com/claude-code), on Anthropic's Opus 5 and Fable 5.1
models.

## License

muir is [AGPL-3.0-or-later](https://www.gnu.org/licenses/agpl-3.0.html).

> "The GNU Affero General Public License is a free, copyleft license for
> software and other kinds of works, specifically designed to ensure
> cooperation with the community in the case of network server software."
> From the license's own preamble, in `LICENSE`.

MIT's own drawings and wire lists are committed in `mit/`, unmodified;
`mit/README.md` is the inventory and says what is MIT's work rather than
this project's.

## The name

muir is named for Nathan Muir, the character [Robert
Redford](https://en.wikipedia.org/wiki/Robert_Redford) plays in [*Spy
Game*](https://en.wikipedia.org/wiki/Spy_Game) (2001). In memory of Robert
Redford.
