# The keyboard boot chord

On a CADR, holding both Control keys and both Meta keys and pressing Rubout
cold-boots the machine; with Return instead, it warm-boots. On muir it does
nothing, on any engine. Every board is complete pin for pin; what is
missing is in the two things that are not boards, the keyboard's own
firmware and the wiring between boards, and one link of that wiring is not
established by any file in `mit/`.

Every claim below names the file it was read from. `tests/unibus_backplane_pins.rs`
holds the wire's pins and `tests/keyboard.rs` holds the boot word's layout.

## What the hardware does

**The keyboard detects the chord, not the machine.** The keyboard has its
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
`mit/cadrio/iob.wlr` the decode is on the IOBCSR page: `BOOT` from the
74S38 at F15 pin 4, `-BOOT` from the 74S04 at E13 pin 13, and the signal
leaves the board as `-BOOT*` from the open-collector 74S38 output at F15
pin 6 onto backplane pin **`CP1`**, page HEXSPC. The HEXSPC drawing, "HEX
SPC SLOT BACKPLANE CONNECTIONS", shows the same pin. (The wire list was
made from the drawing, so that is one transcription checked twice, not
two sources.)

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

## Where muir stops

1. **The keyboard model is the terminal's, not the firmware.** The
   viewer-facing keyboard in `src/terminal/keyboard.rs` knows the boot
   word's layout and never sends it: there is no chord detection and no
   `bootflag`.
2. **The behavioural I/O board does not decode the word.** On `micro` and
   `rtl`, `IoBoard::press` stores the word and sets `KBD READY`; nothing
   tests the boot pattern, so nothing reaches the engine's boot.
3. **On `chip` the netlist I/O board decodes it** --- the IOBCSR logic is
   in `data/CADRIO.netlist` --- and the wire out is not modelled. The
   cables between boards are muir's own tables, not MIT drawings, and this
   wire is not in them: `src/unibus.rs` pulls the pin up and says so, and
   `src/cable.rs` leaves `-LM BOOT` out as a wire that touches no part on
   one board.
