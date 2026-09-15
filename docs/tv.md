# The TV board

The CADR's display is a board on the Xbus: a 32K-word frame buffer, eight
control registers, and a program in the board's own memory that makes the
raster. MIT built two of them --- the **SIMPLE TV**, the black-and-white
board System 100 drives, and the **LISPM TV** that replaced it in December
1980 --- and wrote one programming specification, `mit/cadrtv/lmtv.order`,
which describes both. A machine can carry two boards at once, strapped to
two sets of addresses: the **normal TV** at `17000000`, whichever of the
two boards it is, and the **color TV** at `17200000`, which is MIT's own
spelling for the second board.

Every claim below names the file it was read from. `tests/tv.rs` holds the
model's interface to `lmtv.order`, `tests/sync_program.rs` holds the
programs MIT's own software runs on the board, `tests/simpletv_netlist.rs`
and `tests/lispmtv_netlist.rs` hold what is read here off the boards
themselves, and `tests/color_screen.rs` holds a System 100 cold boot with
the color TV fitted.

## One programming interface

`lmtv.order` is the LISPM TV's order sheet and it numbers the registers
from a strap: "173777x0 Mode (read/write)" to "173777x7", with the note
that settles which board is which:

> Note: For the normal TV, x is 6.  For the color TV, x is 5.

and, of the buffer:

> Locations 17x00000-17x77777 are 32K x 32 bits of video buffer memory,
> without parity.  Some portion of this beginning at 17x00000 is
> displayed.  The first bit sent to the TV is the low-order bit of a word
> (the reverse of the old (pdp11) TV system).  The normal TV has x equal
> to 0, so the buffer starts at 17000000.  The color TV has x equal to 2,
> and so the buffer starts at 17200000.

Two xs, because the registers and the buffer are two decodes; one strap,
because one board carries both. So the normal TV answers `17000000` and
`17377760`, and the color TV `17200000` and `17377750`.
`tests/tv.rs::the_addresses_are_mits_own` reads the numbers out of the
committed `lmtv.order` and holds the model's constants to them.

The eight registers:

| Register | What `lmtv.order` says it is |
|---|---|
| `173777x0` | Mode, read/write |
| `173777x1` | Sync Program `[Sync Ptr]`, read/write, eight bits |
| `173777x2` | Sync Ptr, write only, twelve bits |
| `173777x3` | bit 7 Sync Enable, bits 6-0 Vertical Spacing, write only |
| `173777x4` | Colour: 15-8 value, 7-6 channel, 3-0 colour, write only |
| `173777x5,6,7` | "These addresses respond but don't do anything" |

The board decodes exactly those eight. On the SIMPLE TV the write decode is
the 74S138 at NXBCTL `0F13` on `ADR0..2`, whose outputs are `-LOAD MODE`,
`-LOAD SYNC`, `-LOAD SYNC PTR`, `-LOAD VERT SPACING` and `-LOAD COLOR`,
with the three above them wired to nothing (`data/SIMPLETV.netlist`) ---
which is what "respond but don't do anything" is on the board.

**The mode register's bits**, as `lmtv.order` names and numbers them: `1-0`
Clock Mode, "00 = CPT 64 MHz, 01 = Motorola 4408 32 MHz, 10 = Standard
video 12 MHz, 11 = Color 12 MHz"; `2` Black on White; `3` Vertical Flag
Interrupt Enable; `4` Vertical Flag, "(Causes Interrupt) This is set by
TVMA CLR, not by the start of Vertical Sync"; `5` Vertical Sync, "(Directly
from sync generator) (read only)"; `6` Horizontal Sync; and `31-7`
"Garbage". `tests/tv.rs::the_mode_bits_are_mits_own` reads that block out
of the file and holds the model's bit values to it.

Only bits 3-0 are a register. On the SIMPLE TV they are the 25LS2519 at
NXBCTL `0F12`, whose four data pins are `XDI0..3` and whose clear, pin 19,
is `-POWER RESET`; bits 7-4 are read back through the 74LS244-A at `0F11`,
whose inputs are `VERT FLAG`, `VSYNC`, `HSYNC` and, on this board, `GND`
(`data/SIMPLETV.netlist`). The vertical flag is a flop of its own, the
74LS74 at `0E14`: **preset** by `-TVMA CLR`, **clocked** by `-LOAD MODE`
with `XDI4` as its data, and **cleared** by `-RESET`, which the 74S04 at
`0F10` makes from `XBUS INIT IN`. So a mode write puts its own bit 4 into
the flag, and `-XBUS INIT` clears it.

**Where `lmtv.order` and the drawings disagree about resets, the drawings
are followed, and which was followed is said here.** The order sheet has
the mode register "cleared by Xbus Reset" and the sync enable "cleared by
Xbus reset"; on the board the mode register's 25LS2519 clears on
`-POWER RESET` and so does the 74LS273 at NTVINC `0A07` that holds the sync
enable and the vertical spacing (pin 1 on both). `-XBUS POWER RESET` is a
backplane wire of its own --- ECO 1 of `mit/cadrtv/lmtv.eco`, 3 June 1979,
"Annoying screen popping.  Reset from power-on, not from xbus init", moved
the receiver on the 28 May 1979 wire list from `XBUS.INIT L` to
`XBUS POWER RESET L` and added the backplane wire that carries it ---
and the processor raises it only at power-on.
`tests/tv.rs::an_xbus_init_clears_the_vertical_flag_and_nothing_else` holds
that: the flag goes, the mode, the sync enable, the spacing, the
pointer, the program and the frame buffer stay.

## The sync program

`lmtv.order`: "The Sync Program is a 4K x 8 program which has overall
control of the TV.  It controls the length of raster lines, generates the
horizontal and vertical sync pulses, etc." An instruction's eight bits are
`0` Horizontal Sync to TV, `1` Vertical Sync to TV, `2` Composite Sync
(marked ";Obsolete - H"), `3` Blank, `5-4` the video buffer cycle type ---
processor, refresh, normal video, or end-of-line video, which steps `TVMA`
by the vertical spacing instead of by one --- and `7-6` the special
function: none, `TVMA CLR` "(i.e. start next field)", End of Loop, End of
Program.

The repeat feature is the whole of the control flow:

> The Sync Program is structured as a series of loops.  Each loop is
> executed a fixed number of times between 1 and 256.  A loop starts with a
> word containing the number of times it is to be executed.  This word is
> never executed as an instruction, and does not cause a time delay ...
> The second to last instruction of a loop contains Special Function 2 or
> 3; one more instruction is executed (JUMP-XCT-NEXT) and then control
> returns to the first instruction of the loop, unless the repeat counter
> has counted out.
>
> If the repeat counter has counted out, then if the function was End of
> Program control returns to location 0 of the Sync Program (by a
> JUMP-XCT-NEXT), which is expected to contain a repeat count. ...
>
> If the repeat counter has counted out and the function was End of Loop,
> then the location after next is taken as the repeat count of the next
> loop and the location after that is the first instruction of that loop.

A count of zero is the counter's 256, the count being eight bits and the
range "between 1 and 256". The loops "need not correspond to raster lines,
although they usually will", and in every program MIT wrote they do.

**How long an instruction takes** is the one number the sheet does not
give: "The Sync Program executes an instruction every (32, 16, 8, 32) bits
of video (indexed by Mode<1-0>) or roughly every 1/2 microsecond.  This is
also the rate at which the video buffer RAM cycles." What that comes to in
nanoseconds is the clock PROM's business, so it was measured on the netlist
LISPM TV with `cpt.prom` running, off `HSYNC OUT`, 32 instructions to a
line:

| Clock mode | `lmtv.order`'s name | Line | An instruction |
|---|---|---|---|
| 0 | CPT 64 MHz | 16.000 us | 500 ns |
| 1 | Motorola 4408 32 MHz | 16.000 us | 500 ns |
| 2 | Standard video 12 MHz | 20.000 us | 625 ns |
| 3 | Color 12 MHz | 20.000 us | 625 ns |

500 ns and 625 ns are 32 and 40 periods of the 64 MHz can.
`tests/lispmtv_netlist.rs::an_instruction_of_the_sync_program_in_each_clock_mode`
holds all four, and the SIMPLE TV is measured at 16.000 us a line the same
way in `tests/simpletv_netlist.rs`.

**The mode register's sync bits are the program's own bits, one instruction
late, and the monitor sees the same polarity.** Measured on the netlist
SIMPLE TV over the first sixty lines of `cpt.prom`
(`tests/simpletv_netlist.rs::the_sync_bits_the_mode_register_reads_are_the_programs`):
`HSYNC` follows the program's `SYNC 0` at every edge 500 ns later, and
`VSYNC` follows `SYNC 1` the same way --- the 74LS175 at NSYREG `0D02`
latches them on `-CLK`, which is one latch an instruction. `HSYNC OUT` is
`HSYNC` and `VSYNC OUT` is `VSYNC`, open collector through the 74S37 at
`0D09` and not inverted, so the pulse to the monitor is where the program
bit is one. And `-TVMA CLR` pulses low for one instruction as the
instruction carrying that special function completes: in `cpt.prom` that is
the 32nd instruction of the first line, 16.000 us after the program starts,
and not again for the frame.

**Control reaches location 0 at End of Program, and nothing else on the
board sends it there.** The three 74LS569 address counters at NSYADR take
`-SYNC ADR CLR` on their `-SCLR` pins, and `-SYNC ADR CLR` is the 74S10 at
NSYREG `0D01` pin 8 with three inputs: `-SYNC EOL` and `-SYNC NEW LINE`,
and the 74S08 at `0D10` that ands `RPT DONE` with `SYNC EOL & EOF`. Both
netlists have those three, and MIT's own wire list for the LISPM TV,
`mit/cadrtv/lmtv4b.wlr`, gives the same node its three pins --- `D01-08` as
the driver of `-SYNC ADR CLR` into the three counters, and `D10-11` driving
`D01-09` --- so the drawing and the board as wrapped agree. `-RESET` is on
neither, which is where `lmtv.order`'s "Control also gets to location 0
when the Xbus is reset" is **unverified**; see below.

## What MIT's three programs make

Three sync programs reached us, and running them by the rules above gives
three rasters (`tests/sync_program.rs`).

**`mit/cadrtv/cpt.prom`**, "PROM ;for TV SYNC" of 5 May 1980, is the
program the board runs from power-on, before the software loads the RAM and
selects it. In clock mode 0 it is nine loops of 1, 53, 8, 255, 255, 255,
131, 7 and 1 lines of 32 instructions: **966 lines of 16.000 us**, a frame
of 15.456 ms and so 64.7 Hz, which is what the microcode calls the
roughly-60-cycle clock. 896 of those lines fetch 12 video cycles of 64 bits
--- the 768-dot picture --- 54 are blanked end to end and 912 are not, and
one `TVMA CLR` fires as the first line's 32nd instruction completes. The
netlist SIMPLE TV is measured to make that same raster, line for line, in
`tests/simpletv_netlist.rs`.

**`SET-TV-SPEED`** in `sys/window/shwarm.lisp` is what the window system
loads for the main screen. At its default of 60.5 Hz it computes
`N-LINES` as `(- (FIX (// 1E6 (* 16. ARG))) 70.)`, which is 963, over the
comment "Here each horizontal line is 32. sync clocks, or 16.0 microseconds
with a 64 MHz clock.  The number of lines per frame is 70. overhead lines
plus enough display lines to give the desired rate." Built as the Lisp
builds it, the program walks to **1033 lines**, 963 of them fetching the
picture, a frame of 16.528 ms and 60.5 Hz to the whole line; the 70
overhead lines are its fixed loops of 1, 53, 8, 7 and 1.

**`COLOR:SYNC`** in `sys/window/color.lisp`, under MIT's comment "This is
really NTSC standard video", is the colour board's. In clock mode 3 it is
**525 lines in two fields** of 102 instructions each --- 63.75 us a line
against NTSC's 63.56, a frame of 33.47 ms --- with 454 lines fetching 37
video cycles each and a `TVMA CLR` at the top of each field. 36 video
cycles of 64 bits is 576 pixels of four bits, the colour screen's width,
and 454 lines is its height; the 37th is the blanked end-of-line cycle that
steps `TVMA` by the vertical spacing.

A program that never ends makes no frame: an unwritten RAM is a count of
256 and then no End of Loop before the memory runs out, and one loaded half
way is what it is. `lmtv.order` bounds a real program at "1356 loops of 256
iterations each, or 349440 cycles, or about .2 seconds.  Maybe 3 times
this, actually?", and three times that is where the walk gives up.

## Mode bit 7, the one bit the two boards differ in

On the **LISPM TV** the mode register's read buffer, the 74LS244 at XBCTL
`0F11` section A, takes its bit 7 input, pin 8, from the net
`-SYNC PROM ENB`, which is pin 19 of the 74LS273 at TVINC `0A07` off `XDI7`
--- register 3's Sync Enable. So the bit reads the enable back: one while
the sync RAM is selected, zero while the 74S472 PROM is, the RAM's chip
select being the complement of the same net.
`tests/lispmtv_netlist.rs::the_prom_mode_bit_is_the_sync_enable_read_back`
reads it through bus cycles as the software would.

On the **SIMPLE TV** that pin is `GND`, by ECO 2 of `mit/cadrtv/lmtv.eco`,
18 June 1980: "New window system not initializing tv properly at original
power-up; on old TV boards the check if TV is in PROM mode (extant only on
new TV boards) reads an unused input", with the wire `GND` to `F11-8`. The
ECO is applied when `data/SIMPLETV.netlist` is built, and
`tests/simpletv_netlist.rs::the_prom_mode_bit_reads_zero_whatever_the_sync_enable_is`
holds the board to reading zero with the sync RAM selected and with it not.

**No file the release ships reads the bit.** System 100's only reads of the
mode register are `shwarm.lisp`'s three read-modify-writes of `MODE BOW`
and `color.lisp`'s waits on `VSYNC` and `HSYNC`. `SI:SETUP-CPT`, which the
ECO blames for the check, is called from `sys/sys/ltop.lisp` and
`shwarm.lisp` and exported by `sys/cold/export.lisp`, and is defined in no
file the release ships; nor are `SI:STOP-SYNC`, `SI:FILL-SYNC` and
`SI:START-SYNC`, which `color.lisp` calls. The band has them compiled. So
what the software does with the bit is the ECO's word and not a line of
code we have, and both boards are modelled as the boards read.

## The colour register

`lmtv.order`'s `173777x4`: "15-8 Value to write into color map, 7-6 Select
which color map (up to 4 channels), 3-0 Color (i.e. address into color
map)", over "The color map is a 64x9 RAM for each channel, with a D-A on
it.  However, we only use a 16x8 subset of it.  The address into the color
map RAMs, i.e. the color, comes from successive 4-bit pixels of the video
buffer at a 12 MHz rate."

**Both boards carry it, and the circuit is the same one.** The page is
`COLOR` on the LISPM TV and `NRACOL` on the SIMPLE TV --- `RAMCOL.DRW`,
titled "SIMPLE TV / COLOR MAP" in MIT's own page list `mit/cadrtv/lmtv.stf`
of 28 May 1979 and revised to `nracol` in May 1980. On each: the 74LS244 at
`0D13`, turned on by `-LOAD COLOR`, puts `XDI0..7` on `COLOR 0..7`; the
74S241 at `0E09`, both halves permanently enabled, puts `XDI8..15` on
`COLOR VALUE 0..7`; and the 74S139 at `0E10`, enabled through the 74S32 at
`0E11` when `-ACK WRITE` comes, decodes `XDI6` and `XDI7` into
`-LOAD COLOR 0`, `-LOAD COLOR 1` and `-LOAD COLOR 2`, **its fourth output
unconnected**, so a write naming the fourth channel strobes nothing and the
register acknowledges all the same. The two pages differ in the 74S257's
select and in the name of the pull-up rail on the 241's second enable,
neither of which is this write. Measured on both boards, tap by tap through
a bus write:
`tests/lispmtv_netlist.rs::the_colour_register_strobes_one_map_with_the_colour_and_the_value`
and
`tests/simpletv_netlist.rs::the_colour_register_strobes_one_map_here_as_well`.

`XDI4` and `XDI5` leave the board too, on `COLOR 4` and `COLOR 5`, where
the 64-entry map would take them as address; `lmtv.order` gives the colour
four bits and `WRITE-COLOR-MAP` writes `(LOGAND LOC 17)`, so MIT's own
software never sets them. The map RAMs and their D-As are off the board.

## The color TV as System 100 drives it

`COLOR:MAKE-SCREEN` in `sys/window/color.lisp` defines the screen
`:BITS-PER-PIXEL 4 :BUFFER -600000 :HEIGHT 454. :WIDTH 576.
:CONTROL-ADDRESS 377750`, which is `lmtv.order`'s x = 2 and x = 5, and an
`(ADD-INITIALIZATION "Color Make Screen" ... '(ONCE))` puts it among the
screens `WINDOW-INITIALIZE` exposes at cold boot.

**The release finds out whether there is a board by writing to it.** The
`(COLOR-SCREEN :EXPOSE)` wrapper asks `COLOR-EXISTS-P`, "T if this machine
has color screen hardware", which is `XBUS-LOCATION-EXISTS-P` on the first
word of the colour buffer: `(%XBUS-WRITE XBUS-ADDR BITS)` and then
`(BIT-TEST BITS (XBUS-READ-NO-PARITY XBUS-ADDR))` --- a write of 1 with the
error stop off, read back. A machine without the board has to answer that
with an NXM.

With a board there, `COLOR:SETUP` runs `SI:STOP-SYNC`, `SI:FILL-SYNC` with
`COLOR:SYNC`, and `(SI:START-SYNC 3 0 36. TV-COLOR-ADR)` --- which
`CC-TV-START-SYNC` in `sys/cc/dmon.lisp` writes out as the mode
`(+ (LSH BOW 2) CLOCK)` and register 3 as `(+ 200 VSP)`: clock mode 3, the
sync RAM in, vertical spacing 36, MIT's "36. for color". Then
`R-G-B-COLOR-MAP` sets "color 0 to black, and the remaining colors to
alternating red, green, blue" through sixteen `WRITE-COLOR-MAP`s of three
writes each.

**Those writes wait on the sync bits.** `WRITE-COLOR-MAP` writes register 4
through `(%XBUS-WRITE-SYNC (+ TV-ADR 4) ... TV-ADR 100 100)`, and
`%XBUS-WRITE-SYNC` is `XXBWS` in `sys/ucadr/uc-cadr.lisp`: it reads the
mode register under the mask `100` --- bit 6, `HSYNC` --- until the value
differs from `100`, then until it matches, and only then writes. Its own
`SYNCHRONIZE` argument spins on `(LOGAND 40 (%XBUS-READ TV-ADR))`, bit 5,
`VSYNC`; `BLT-COLOR-MAP` does the same twice over, "We wait for vertical
retrace to tell the hardware". So the colour map is written at all only on
a board whose sync program is running.

**The map is stored inverted.** `WRITE-COLOR-MAP`'s arguments are "numbers
from 0 to 377 that together say how pixels containing LOC should appear on
the screen", and what it sends the board is `(- 377 (LOGAND (FIX R) 377))`
for each of the three; the band's own `HARDWARE-COLOR-MAP` array keeps
`377 - stored` again, because "the hardware does not allow reading back of
the color map".

`tests/color_screen.rs` runs that cold boot on `rtl` with the System 100
pack and the board fitted --- about 13 million microcycles to the listener
--- and holds what the release left on the board: clock mode 3, `MODE BOW`
clear, the interrupt enable clear, the sync RAM in with spacing 36, a
loaded program that makes NTSC's 525 lines with a `TVMA CLR` a field and
454 picture lines, the main board untouched, and the sixteen colours
`R-G-B-COLOR-MAP` wrote, stored inverted.

**Nothing in the release enables the colour board's interrupt.** Both
boards drive the one `-XBUS.INTR`: on each, the 74S08 at `0D10` ands
`MODE INTR ENB` with `VERT FLAG` into `SEND INTR` and the 26S10 at `0F14`
puts that on the line, the same nets in both netlists, so the line is the
boards ORed. `START-SYNC` writes the clock mode alone, which matters:
`INTRX0` in microcode 323 reads `A-TV-REGS-BASE` and has nothing to clear a
second board's flag with. `tests/tv.rs::both_boards_are_on_the_one_interrupt_line`
holds the OR and holds `-XBUS INIT` reaching both flags.

## What muir does

**One model serves both boards and both straps** (`src/tv.rs`): the frame
buffer, the mode register, the vertical flag, the sync RAM with its pointer
and enable, and the 16 x 3 colour map. `--tv-board simple-tv|lispm-tv` says
which board, on every engine, and the only thing it changes is what mode
bit 7 reads. `--color-tv` fits a second board, a LISPM TV at the color TV's
strap; without it the bus decode gives those addresses an NXM, which is the
answer `COLOR-EXISTS-P` needs.

**On `chip` either display is a netlist board or the model**, and the
second one is `--color-tv [netlist|model]`: `netlist` is the board itself,
`model` is this model at the colour addresses, and the bare flag is the
netlist on `chip` and the model on the other two engines, where `netlist`
is refused by the engine's name. The colour board's netlist is
`data/LISPMTV.netlist` again, whatever `--tv-board` put at the main
screen's addresses, brought up by `netlist::parse_color_tv` and wrapped by
`xbus::straps` to `tv::COLOR_TV`.

**Three wire-wrap straps are the whole difference between the two boards**
(`src/xbus.rs`). `MAPADR 16` goes high, moving the buffer from `17000000`
to `17200000`; `DEVADR 3` goes high and `DEVADR 4` goes low, moving the
registers from `17377760` to `17377750`. On MIT's own board those are
XBADR 0F22 pin 06, and XBADR 0F19 pins 13 and 15, and `cadrtv/lmtv4b.wlr`
has all three wrapped the normal TV's way: 0F22-06 and 0F19-13 on the
ground net --- 0F22-06 in the run wrapped from the `BT1` ground pin and
0F19-13 in the one from `CF1` --- and 0F19-15 on the
pull-up at XBADR 0E14, the net that list heads with `DEVADR 4` through
`DEVADR 21`, `MAPADR 18` through `MAPADR 21` and `HI1` at once --- one net
where the SIMPLE TV's drawings leave each strap a net of its own. So
`DEVADR 4` is not a net muir can wrap: `netlist::parse_color_tv` moves
0F19-15 onto a net of its own first, and `xbus::straps` refuses a display
board handed the colour strap without it rather than leave it answering at
the main screen's address.
`tests/lispmtv_netlist.rs::the_colour_wrap_moves_three_pins` holds the
three, `::the_board_wrapped_as_the_color_tv_answers_at_the_other_addresses`
holds that the wrapped board answers at `17200000` and `17377750` and at
neither of the normal TV's blocks, and
`tests/chip.rs::two_display_boards_answer_at_their_own_straps` holds the
two boards on one backplane, each answering its own cycles while the models
behind the buses hold both pictures.

**A write to a netlist display board is mirrored into its model**
(`src/buses.rs`), the main screen's into `machine.tv` and the colour one's
into `machine.color_tv`, so the picture is read off the model whichever
board drew it; and a model whose netlist board is on the backplane does not
put its own vertical interrupt on `-XBUS INTR`, the board driving that
wire itself.

**The sync program is run** (`src/tv/sync.rs`). The program in the RAM
while the enable selects it, and `cpt.prom` otherwise, is executed by the
rules above into a timeline of one run --- its period, the instants the
sync bits change, the instants `TVMA CLR` fires, and the lines --- and the
model reads that timeline by time, the program being periodic. So the
vertical flag is preset where the running program's `TVMA CLR` falls, and
the mode register's `VSYNC` and `HSYNC` are the program's bits 1 and 0
latched an instruction late, as measured on the board. A change of program,
of the RAM's selection, or of the clock mode runs the program afresh from
location 0.

**What muir does not do is scan.** No dot is fetched, no shift register is
loaded and no monitor is driven, so **the picture is the frame buffer as it
stands**: a program that fetches part of the buffer, or none of it, shows
the whole of it all the same, and `MODE BOW` is applied to the
black-and-white picture by inverting it. The colour picture is 576 by 454,
a pixel being a nibble of the buffer with the low nibble of a word first
--- `COLOR:MAKE-SCREEN` displaces an `ART-4B` array onto it,
`sys/cold/qcom.lisp` gives `ART-4B` eight elements a word, and
`XCOLOR-TRANSFORM` in `sys/ucadr/uc-hacks.lisp`, MIT's own microcode
walking such an array over this screen, takes element `k` from bit
`4 * (k mod 8)` of word `k / 8` --- and that four-bit value indexes the
map. A gun is rendered as `255 - stored`. `MODE BOW` is not applied to it:
on the board that bit reaches the 10124 at `0F06` and nothing but the 10102
sections at `0F04` that exclusive-or `8B SR 0` into `-MECL VIDEO`, which is
the one-bit video path, and `COLOR:SETUP` leaves it clear in any case.

The main screen is served over RFB on `--terminal` and the colour screen on
`--color-terminal`, pixels only: the machine has one keyboard and one
mouse, both on the I/O board, and they stay with the terminal that serves
the main screen.

## Unverified

**The color TV's own wrap.** `lmtv.order` gives the board's two addresses
and MIT left no wire list of a board strapped to them: `cadrtv/lmtv4b.wlr`
is a normal TV. So the three pins above are read off that list wrapped the
other way, and that they are the three --- and that a colour board was a
LISPM TV and not a board of its own --- is inference from the addresses.
**What would settle it:** a wire list of a second board, or an installation
note naming the pins the colour strap moves.

**What the off-board D-A makes of a stored map byte.** muir renders a gun
as `255 - stored`, and the only reference for that is the software's own
model of the RAM: `WRITE-COLOR-MAP` sends the board `377 - value`, and the
array the band keeps beside it --- because "the hardware does not allow
reading back of the color map" --- holds `377 -` what the board holds,
which is what `READ-COLOR-MAP` gives back. `lmtv.order` says only "a 64x9
RAM for each channel, with a D-A on it"; the RAMs and the D-As are past the
paddle connections that ECO 2 of `mit/cadrtv/lmtv4b.eco` rewires ("these
wires are from paddles to old or new Outs"), and `mit/cadrtv/lmtv.book`,
the board's own print list, is the sheets and text files of the board
itself and nothing of the paddles. **What would settle it:** a drawing or a
parts list of those paddles.

**The sync program's phase after the software changes the program or the
clock mode.** muir runs the program afresh from location 0 at every change
of the RAM's contents while it is selected, of the enable, or of the clock
mode. The board's address counters are cleared only by `-SYNC ADR CLR`,
which none of those writes reaches, so the board may instead go on fetching
from wherever its counter stood. Nothing in MIT's software depends on the
phase. **What would settle it:** the phase of `-TVMA CLR` measured on a
netlist board across a `SETUP-CPT`, which is a program loaded and then
selected under a running generator.

**Whether an Xbus init sends the program back to location 0.**
`lmtv.order` says it does --- "Control also gets to location 0 when the
Xbus is reset" --- and the boards show no wire for it: `-SYNC ADR CLR`,
the counters' only clear, is the 74S10 at `0D01` on `-SYNC EOL`,
`-SYNC NEW LINE` and the 74S08 at `0D10` that ands `RPT DONE` with
`SYNC EOL & EOF`, all three of them the program's own sequencing, in both
netlists and in `mit/cadrtv/lmtv4b.wlr`. `-RESET` reaches one flop on the
board, the vertical flag's. muir follows the drawings and leaves the phase
alone, so a bus reset here clears the flag and nothing else. **What would
settle it:** a reset into the sync sequencer on a page the extraction did
not carry --- neither board's netlist has one --- or MIT's own word that
the sentence describes what the board was meant to do rather than what was
wrapped.
