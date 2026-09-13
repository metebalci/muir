# The keyboard boot sequence

On a CADR, holding both Control keys and both Meta keys and pressing Rubout
cold-boots the machine; with Return instead, it warm-boots. muir does the
same from the terminal's keyboard, on every engine, with the keys the
sequence needs a setting, `--keyboard-boot`. Every board is complete pin
for pin; the two things that are not boards, the keyboard's own firmware
and the wiring between boards, are modelled from the firmware's source and
from the wire lists, and one link of that wiring is not established by any
file in `mit/`.

Every claim below names the file it was read from. `tests/unibus_backplane_pins.rs`
holds the wire's pins, `tests/keyboard.rs` holds the boot word's layout and
the keyboard's part, `tests/ioboard.rs` the behavioural board's decode, and
`tests/keyboard_boot.rs` the path end to end on each engine.

## What the hardware does

**The keyboard detects the sequence, not the machine.** The keyboard has its
own microprocessor, and `sys/io1/ukbd.lisp` in the System 100 sources is
its firmware. Its `check-boot` routine runs after every key-down: if both
Metas (key positions 45 and 165) and both Controls (20 and 26) are down
along with Rubout (23), it sends the cold boot code; with Return (136),
the warm one. The firmware's own comment, at the routine: "Is request to
boot machine if both controls and both metas are held down, along with
rubout or return." The boot word is bits 15-10 ones, 9-6 zero, and 5-0
`46` octal for cold or `62` for warm, with the new keyboard's source
identifier 1 in bits 18-16. `muir::terminal::keyboard::boot` builds that
word and `tests/keyboard.rs` holds it to the firmware's description.

**Then it holds its tongue.** The firmware sets `bootflag` after the boot
word and clears it on the next key-down; while it is set no key-up codes
are sent. Its comment: "This gives the machine time to load microcode and
read the character to see whether it is a warm or cold boot, before
sending any other characters, such as up-codes."

**The I/O board decodes the word itself.** From the same file's
description of the character format: "The IOB boots the lisp machine if a
character is received with bits 10-13 = 1, bits 6-9 = 0, and bit 16 = 1
(bit 16 may or may not be looked at depending on remote mouse enable.)
The low 6 bits control whether it is a warm or cold boot." In
`mit/cadrio/iob.wlr` the decode is on the IOBCSR page: `-BOOT` is the
`-EQUAL` output of the 25LS2521 comparator at A20 pin 19, into the 74S04
at E13 pin 13, whose pin 12 makes `BOOT`, into the 74S38 at F15 pin 4
(the list's TYPE column marks E13-13 and F15-04 `TI` and `TIS`, inputs,
and A20-19 and E13-12 `TO`, outputs); and the signal leaves the board as
`-BOOT*` from the open-collector 74S38 output at F15 pin 6 onto backplane
pin **`CP1`**, page HEXSPC. The HEXSPC drawing, "HEX SPC SLOT BACKPLANE
CONNECTIONS", shows the same pin. (The wire list was made from the
drawing, so that is one transcription checked twice, not two sources.)
What the comparator compares is under "What muir does" below.

**It really did boot machines.** `mit/cadrio/iob.eco` ECO#3, of 30 January
1980, slows the keyboard clock for the new keyboards and warns that the
change "increases the chance of the old keyboard rebooting the machine
accidentally".

**The microcode reads the same word to choose warm or cold.** Microcode
323 enters at `(LOC 6)`, reads the keyboard status, and takes the cold
boot unless the keyboard is ready and holding something other than
Rubout (`src/ioboard.rs`, at the top). That is why the firmware holds the
up-codes back: a word landing in the keyboard register before the
microcode looks replaces the one there, as the three 74LS164 shift
registers on the IOBKBD page shift the next word in over the last.

## Where the wire goes, link by link

**Link 1, the I/O board to the backplane: established.** `-BOOT*` on
`CP1`, as above.

**Link 2, the backplane between the I/O board's slot and the bus
interface's: not established.** The bus interface takes its boot line on
**`CR1`**: `mit/cadr1/busint.wlr` has `-LM BOOT` with exactly two
connector pins, `CR1` on page CUBUS and `J08-12` on page CLM, and the
list's own remark, "MORE THAN 1 CONNECTOR PIN ... NO INPUTS OR OUTPUTS":
a pass-through with no part on it, which is why the BUSINT netlist has
no net for it. The CUBUS drawing, "UNIBUS SPC CONNS", shows `LM BOOT L` on
`CR1`.

`CP1` and `CR1` are different pins, and it is not a difference in naming:
all forty-two wires `muir::unibus::wire_pairs` joins between the two
boards sit on the same pin name in both lists (`-MSYN*` and `-UB MSYN` on
`EE1`, `-D0*` and `-UBD0` on `CS2`, and so on; the board's request and
grant reach the backplane through jumpers, and their pin records agree
too). Both boards use DEC's SPC lettering, with the grant chains and
`NPG` on DEC's pins, and `LM BOOT` is not a Unibus signal, so nothing in
the standard bus carries it from one slot's `CP1` to another's `CR1`. So
either MIT's card cage has a backplane wire from the I/O slot's `CP1` to
the bus interface slot's `CR1`, or one of the two lists is wrong about its
pin, or the two were never joined and keyboards booted machines some other
way. No file in `mit/` describes the cage's wiring. **Unverified.** What
would settle it: a backplane wire list, or a photograph of a cage.
`tests/unibus_backplane_pins.rs` holds the two pins as the lists give
them, so that a corrected list is noticed.

**Link 3, the bus interface to the processor: established, and it is
`-BOOT1`.** `data/cables.txt` pairs the bus interface's `J08-12` with the
processor's `1AJ1-12`, which `mit/cadrwd/icmem3.wlr` names `-BOOT1`, page
MBCPIN; and `mit/cadr/busint.erface` documents the cable signal: "-BOOT1:
Take this low to boot the machine. It has a pullup."

**`-BOOT2` is the light panel's.** `icmem3.wlr` puts `-BOOT2` on
`1AJ2-03`, and the MBCPIN drawing, "BUS INTERFACE CABLES", marks
connector `1AJ2` "TO LIGHT PANEL", beside the parity-error and run lamps,
and `1AJ1` "TO BUS INTERFACE J08". So a CADR boots three ways, and they
meet on the board: `-BOOT1` from the keyboard by way of the Unibus,
`-BOOT2` from the button on the light panel, and `PROG.BOOT` from the
other machine through the debug cable. Each of the first two goes through
a section of the 74LS14 at 1A20; `-BOOT2`'s is ORed with `PROG.BOOT` by
the 74S32 at 1C18; and both reach the 74S02 at 1A07 that makes `-BOOT`,
which presets `RUN` and forces the boot trap (`data/CADR.netlist`; the
OLORD2 page). The processor cannot tell which was pressed. The prompt's
`boot` on `chip` is the button, and presses `-BOOT2`.

## What muir does

1. **The keyboard model detects the sequence and sends the boot word**
   (`src/terminal/keyboard.rs`). After every key-down that is held, the
   keyboard runs the firmware's `check-boot`: with the Controls and Metas
   the run asks for down and Rubout down, it queues the cold boot word
   after the key-down's own word; with Return down, the warm one; Rubout
   tested first, as `check-boot` tests it. Then it does what `bootflag`
   does: no key-up word goes until the next key-down, and the keys
   released meanwhile are up on the keyboard all the same.
   `--keyboard-mapping-trace` says on the key-down's line that the boot
   word went, cold or warm, and on a held-back key-up that it was held
   back and why. The keys are read from MIT's table --- Control at 20 and
   26, Meta at 45 and 165, Rubout at 23, Return at 136 --- and
   `tests/keyboard.rs` holds them to the numbers `check-boot` names, and
   holds the words and the hold-back; `tests/keyboard_mapping.rs` holds
   the trace's wording.

   **The keys the sequence needs are a setting**, `--keyboard-boot`,
   because a host keyboard rarely has two Controls and two Metas free to
   map. `ctrl` is MIT's Control key and `meta` its Meta; one of a word is
   either key of its pair, two is both; the order does not matter.
   `ctrl,meta`, the default, is either Control and either Meta;
   `ctrl,ctrl,meta` both Controls and either Meta; `ctrl,meta,meta` either
   Control and both Metas; `ctrl,ctrl,meta,meta` both of each, the CADR
   keyboard's own sequence. Rubout and Return are never in it, and
   anything else is refused with the four named. The firmware compares
   whole bytes of its bit map, so on the keyboard itself another key down
   in the same byte as one of the four --- a Shift, at 24 or 25 beside
   the Controls --- defeats the sequence; muir counts only the keys named.

2. **The behavioural I/O board decodes the word, on `micro` and `rtl`**
   (`src/ioboard.rs`: `boot_word` and `IoBoard::take_boot`). The decode
   is read off the netlist, not the prose. On IOBCSR the 25LS2521 at 0A20
   compares `SR7`-`SR10` with ground and `SR11`-`SR14` with the pull-up
   `HI4`, and `SR<n>` is bit `n-1` of the word --- the 74LS374 at IOBKBD
   0C30 puts `SR1` on `UBO0`, the start marker riding at `SR0` --- so the
   board looks at bits 13-6 of the word and no other: ones in 13-10 over
   zeros in 9-6, which is `ukbd.lisp`'s "bits 10-13 = 1, bits 6-9 = 0"
   exactly. Its `-EQUAL` is `-BOOT`, the 74S04 at 0E13 makes `BOOT`, and
   the 74S38 at 0F15, `BOOT` on pin 4 and `HI4` on pin 5, makes `-BOOT*`
   at pin 6. **Bit 16 is not in the comparator.** What the prose calls
   "bit 16 may or may not be looked at depending on remote mouse enable"
   is `-CHAR TO MOUSE`: `CHAR FROM MOUSE` is `SR17`, bit 16, inverted by
   the 74LS14 at 0A27, ANDed with `REMOTE MOUSE ENABLE` by the 74LS08 at
   0D26 and inverted again by the 74LS14 at 0D20, one of the three inputs
   of the 74LS10 at 0C28 that makes `EOC.KBD^` with `SR0` and `-KB CLK^`.
   Under the enable a word with bit 16 clear is the mouse's and makes
   `EOC.MOUSE^` instead, so neither `KBD READY` nor the boot decode sees
   it; with the enable clear, bit 16 is not looked at. The boot word's
   bit 16 is set, source `001`. `EOC.KBD^` is also the comparator's
   enable, low while `KB CLK^` is low with the start marker at `SR0` ---
   the half clock before the rising edge that latches the word into the
   74LS374s and sets `KBD READY` --- so `-BOOT*` is a pulse of 4 us ending
   on the edge that sets `KBD READY`, and not a level held while the word
   sits in the register: measured on the netlist board in
   `tests/keyboard_boot.rs`. The model latches the match for the engine
   to take once and leaves the word in the register with `KBD READY` up,
   for `(LOC 6)`. `tests/ioboard.rs` holds the decode to the cold and
   warm words and to nothing else.

3. **The engines take the request** (`Engine::keyboard_boot` in
   `src/engine.rs`, called wherever `src/main.rs` delivers keyboard words
   to the behavioural board, the lashup's two machines included). The
   request presses the engine's boot as the button does; `boot` touches
   no board on either engine, so the word and `KBD READY` stay.
   `tests/keyboard_boot.rs` runs the PROM on `micro` and on `rtl`,
   delivers the sequence through a `Keyboard` and the board, and sees
   `RUN` preset, `PROMDISABLE` clear, the PC at 0 and then 45, and the
   boot word still readable, 46 or 62.

4. **On `chip`, the wire** (`FarEnd::boot_line` in `src/cable.rs`). The
   netlist I/O board decodes the word itself and pulses `-BOOT*` low,
   open collector, pulled up in `Unibus::new`; the far end drives the
   processor's `-BOOT1` low while `-BOOT*` is low and releases it after
   --- an undriven TTL input reads high on this engine --- and `-BOOT1`
   reaches the 74S02 at OLORD2 1A07 that makes `-BOOT`. The wire runs
   from `-BOOT*` to `-BOOT1` in one hop, the bus interface having no net
   for `-LM BOOT`. **The link between `CP1` and `CR1` is unverified**, as
   above: the wire models what ECO#3 says happened, not a wire any file
   shows. `tests/keyboard_boot.rs` sends the boot word down the keyboard
   cable into the netlist board with the whole machine on the far end and
   sees `-BOOT1` and `-BOOT` follow `-BOOT*` low for 4 us, `RUN` preset,
   and the PROM run from 0 again with `KBD READY` up on the board. Under
   `--io-board model` the behavioural board's request presses `-BOOT1`
   through the same hold as the button's `-BOOT2`.

5. **The prompt's `boot` is the button still**, `-BOOT2` on `chip`, and
   the processor cannot tell the two apart.
