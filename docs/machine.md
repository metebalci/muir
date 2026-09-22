# The machine it models

Part of [the muir manual](manual.md).

The processor is not one board and the bus interface is not part of it. They
live in different backplanes and talk over five 40-wire flat cables that
carry two independent buses and the master clock, which the processor
supplies. Everything else in the machine hangs off one of those two buses.

**The diagram of the whole machine is on the front page**, at
[muir.metebalci.com/#machine](https://muir.metebalci.com/#machine): the
processor and the bus interface, the Xbus and the Unibus and every board on
them, the debug cable to a second machine, and what each of the models
reaches outside muir. What follows is what that picture says.

The `chip` engine runs every board as the netlist of MIT's own drawings for
it, held pin for pin to MIT's wire list for the same board --- every board
but the disk multiplexor, which has no wire list on the tapes and is held to
MIT's stuffing list instead, the list of which bodies are placed. Three
kinds of thing on the diagram are not boards: the Trident disk unit, a drive
cabinet on the end of the controller's cable, a behavioral model; the
Chaosnet cable itself, modeled as a medium rather than drawn as gates; and
the terminals, one to a screen. Each is pointed at something outside muir
--- the drive at a disk image file, the terminal at a VNC client, the cable
at whatever hosts its CHUDP node carries. **The file and time host a band
calls is not among them.** A CADR had no file or time server in it, so muir
has none either, and the host is a program of its own on the network,
[ozd](https://github.com/metebalci/ozd). `--chaos-address` gives this
machine its address and puts the CHUDP node on the modeled ether with it;
`--chaos-udp-peer` says where each host lives. muir is a leaf there and not
a router --- a frame goes out only when this machine put it on the cable, so
one host's is never carried on to another.

The disk multiplexor is a board, 62 parts, and the ring round it says it is
fitted only when asked for. It is what gives the *netlist* disk controller a
drive past unit 0: the three unit lines reach one of that board's receivers
and nothing on it drives them, so on a single-drive machine MIT's own
jumpers ground them and unit 0 is the consequence rather than a setting ---
and the multiplexor is where the unit number comes from, latched off the bus
and driven back. The behavioral controller wants none of it, having
addressed eight drives all along, which is why the plain cable is drawn
straight to a drive and the multiplexor stands in a ring of its own. Any
board can be run as its model instead, one at a time. The `rtl` engine runs
the models throughout, and the two engines are held to each other at every
microcycle, so where a board and its model part, one of them is wrong.

The terminal is where the machine meets a person: the picture goes out of
it, keyboard and mouse events come back in to the I/O board, and the beep
goes the other way --- the beeper is in the keyboard, not the TV. All of it
over RFB, so the screen and the keys are a VNC client --- on this machine or
anywhere else. The color screen has a terminal of its own beside that one,
`--color-terminal`, and it carries pixels and nothing else: the machine has
one keyboard and one mouse, both on the I/O board, and they stay with the
terminal that serves the main screen. Where the picture comes from depends
on the engine. `micro` and `rtl` have no video timing at all, so the
terminal reads their frame buffer; `chip` with the TV as a netlist really
scans out, and the terminal takes what a monitor takes, the differential
video pair and the two syncs. That pair is the one signal the board
deliberately leaves unterminated, because the pull-down belongs at the far
end of the cable --- so the terminal supplies it, as the far end already
holds the Chaosnet's receive pair and the keyboard's.

Two of the boxes are one board standing for a choice. The TV is either
display board, which is why the box names both: the SIMPLE TV, which is what
muir runs by default, or the LISPM TV that replaced it in December 1980, 172
parts against 171, near enough the same board, and `--tv-board` says which
on every engine. Whichever it is, the main screen is monochrome --- System
100 drives whatever is at `17000000` at one bit a pixel, 768 by 963. The
color picture is the second board's: `--color-tv` fits **the color TV**, the
same LISPM TV wrapped to the color address, and its 576 by 454 at four bits
a pixel through a sixteen-entry color map is what the second terminal
serves. On `chip` it is a second netlist on the backplane, that same
`LISPMTV.netlist` wrapped to the color addresses instead of the main
screen's; on `micro` and `rtl` it is the model, as the rest of the machine
is. And the I/O board is two machines' worth of function on one board: 94
parts on the keyboard, mouse, clock and serial pages against 84 on
[the Chaosnet's](chaosnet.md), near enough half each. They are not two
boards in disguise, though, which is why the diagram does not draw them
apart. Forty-seven signals cross between the halves --- the buffered Unibus
data and address lines, the reset, the address decode that makes the
Chaosnet's own select, and its bus reply and interrupt going back out
through the I/O half --- and the clock the I/O half counts microseconds on
is the Chaosnet half's own 32 MHz crystal. The serial port is on that half
too --- the Signetics 2651 at IOBSER 0A12, its 5.0688 MHz baud-rate can
beside it, and the MC1488 and MC1489 out to the RS-232 connector at J9 ---
and `--serial` gives it a far end outside muir: a TCP endpoint, off unless asked for, which
plugs the cable when something connects --- DSR, DCD and CTS asserted as a
device on a null-modem does with its DTR and RTS, because the 2651 is
conditioned to transmit on `-CTS` low and receive on `-DCD` low, so carrying
bytes is not enough. The far end takes its rate and framing from the port,
whatever the machine programmed into it: a far end that picks its own
produces garbage rather than an error.

The five cables carry two buses. One is the memory bus. The other is the
diagnostic bus, SPY: sixteen registers on the processor's two boards, read
and written by whoever masters the bus interface's Unibus, which is how the
console program CC halts, steps and inspects a machine. They are not a board
and are not drawn as one. Every engine has them, and the netlist is held to
`rtl` on them through the microcode's own reads and writes of its own
registers. The dashed box at the top right is not a model but a second
machine: the debug cable, 21 wires from the DBGOUT connector of one bus
interface to the DBGIN connector of another, over which one machine's Unibus
cycles run on the other's bus. Both connectors' pages are in the netlist;
the cable and the machine at its far end are the two-machine lashup, the
acceptance test. It runs on `rtl` and `chip` only: the cable is the bus
interface's timing and nothing else --- the arbitration for the debuggee's
bus, the acknowledgement at its instant, the timeout --- and `micro`, having
no timing model, has no end of it and cannot be debugged over it. And the
disk controller is a netlist like the rest of them, which costs a run that
reads a pack the drive's real milliseconds: `--disk-controller model` puts
the instant transfers back for a run that does not care about the disk.
