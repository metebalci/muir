# The glass TTY

`--glass-tty` serves the machine's screen as text over telnet, and puts
what is typed there back into the keyboard. **Nothing in the band knows it
is there.** It is not a device the software drives; it is a reader of the
frame buffer and a writer of the keyboard's own cable, both of which the
machine already has.

Back to [the manual](manual.md).

## What it is, and what `--serial` is

The two are easy to confuse and they are opposites.

`--serial` is the 2651 at J9, a real part on the I/O board. For anything to
come out of it the band must have a driver, the rate and the frame must be
programmed into the chip, and the software must choose to write there. MIT's
driver defaults to 300 baud, seven data bits and even parity, and nothing in
muir sets them.

`--glass-tty` needs none of that. It reads the frame buffer the machine has
already drawn into, decodes each character cell against the machine's own
character font, and writes the result to a socket as ANSI. What a client
types goes the other way, onto the 24-bit shift register the I/O board reads
MIT's keyboard from. **Both halves are paths the machine already has**, so
the glass TTY works where the band's software cannot help at all:

- on a cold machine, before any driver exists;
- in the boot PROM;
- in PRAID;
- in the MIT diagnostics.

Those are the cases muir exists for, and they are exactly the ones a serial
port and a network file transfer cannot reach.

## The limit

It reads a bitmap back through one font. A cell that is not a character of
that font reads as `?`.

For a cold-load stream, for PRAID and for the diagnostics, that costs
nothing: those screens are text. **Under the window system it costs most of
the screen** --- much of what a booted System 100 draws is not text in the
character font, and will not read. The glass TTY is not a replacement for
`--terminal`; it is what to use when a VNC viewer is not what you want, or
when there is nothing on the screen a window system drew.

## The two fonts

**There are two different fonts, both called `FONTS:CPTFONT`.**

- `sys/fonts/cptfon.qfasl` is the one the cold load brings up.
  `sys/sys/sysdcl.lisp:462`'s `COLD-LOAD-FILE-LIST` begins `"SYS: FONTS;
  CPTFON QFASL >"`. Its raster is **7** columns wide, and its rows are
  packed four to a 32-bit word at bit offsets 0, 7, 14 and 21.
- `sys/fonts/cptfont.qfasl` is the one the full system fasloads over it,
  `sys/sys/sysdcl.lisp:126-127`. Its raster is **8** wide, one row to a
  byte.

They are not the same bitmap. The cold load's `I` carries full-width serifs
and the system's short ones:

```
    cptfont (8-wide)      cptfon (7-wide)
 I  ..###...              .#####.
 H  .#....#.              .#....#
```

A readback holding the wrong one does not fail loudly. It matches nothing
and fills the screen with `?`.

**The cell pitch is the same for both.** `FONT-CHAR-WIDTH` is 8 either way
--- the 7-wide font is the same 8-pixel cell with its last column left blank
--- so the grid is 96 characters by 80 whichever font is in force, and only
the glyphs change.

**And the machine changes from one to the other as the band boots over the
cold load.** So nothing can be told once which font to use. muir scores both
against the screen on every poll and takes whichever reads more cells; the
system font is where it starts, and where it stays on a screen that does not
tell them apart, because a run watching a booted band is the common case.
`tests/font.rs` holds all of this: that the two are different bitmaps, that
each reads back what it drew, that the readback follows a change of font, and
that a blank screen leaves it where it was.

The layout is MIT's own. `sys/window/tvdefs.lisp:517` is the `FONT`
defstruct --- fill-pointer, name, char-height, char-width, raster-height,
raster-width, rasters-per-word, words-per-char, baseline --- and
`tvdefs.lisp:557-561` gives the data: "an integral number of words per
character. Each word contains an integral number of rows of raster, right
adjusted and processed from right to left". Which way that falls on this
machine is settled by the microcode, `sys/ucadr/uc-tv.lisp:21-39`: "LEFT
ADJUSTED AND PROCESSED FROM LEFT TO RIGHT. (RIGHT TO LEFT ON 32-BIT TVS)".
The CADR is the 32-bit TV, so row 0 is in the low bits.

**None of those numbers is written down in muir.** Each font declares its
own shape --- char height and width, raster width, rasters per word, words
per character --- and the decoder reads the leader rather than assuming
any of it, so one path serves both fonts and a third would need no new
code. `the_fonts_declare_the_grid_the_readback_uses` holds what the files
declare to the grid the readback cuts a screen into. That test earns its
place: every other test here draws a glyph and reads it back on the same
pitch, so all of them would pass just as well if that pitch were wrong.

**Both fonts are committed**, in `mit/sys/fonts/`, byte for byte the
release's own files, as `mit/sys/ubin/promh.mcr` is for the boot PROM.
`the_committed_fonts_are_the_ones_system_100_ships` holds each to the
release's copy whenever `vendor/` is there, and says it was skipped when
not. Only the rasters are read: `src/terminal/font.rs` takes the bytes
after the QFASL operation whose count says 128 characters of 12 rows, so
nothing here is a QFASL reader.

## The cursor

The cursor is drawn by exclusive-or into the frame buffer like anything
else, so under it a glyph is its own complement. A cell that matches no
glyph is looked for complemented, and one that matches that way is both the
character and the cursor's position. A solid block with nothing under it is
the cursor on an empty cell.

## What goes on the wire

Telnet, RFC 854. The server offers `WILL ECHO` (RFC 857) and `WILL
SUPPRESS-GO-AHEAD` (RFC 858), which together put a real client into
character-at-a-time mode, and asks `DO NAWS` (RFC 1073) for the window size.
**Echo is the server's because the machine echoes**: a key goes to the
keyboard and what comes back is whatever the machine chose to draw, which is
the screen this paints. A client echoing for itself would show characters
the machine never took.

The painting is ECMA-48: `CUP` to place the cursor, `ED` to clear, `EL` to
erase to the end of a line. Nothing more exotic, so any telnet client will
do.

A client is sent **the whole screen when it connects**, and only the lines
that changed after that. A person attaching to a machine that has been
sitting at its prompt for an hour needs to see the prompt.

### Which part of the screen a person sees

The machine's screen is 80 character rows and a terminal window is usually
24, so something has to choose. **Only rows with anything on them are
painted** --- painting all 80 scrolls everything interesting off the top of
the window, and the bytes that do it are perfectly correct.

The window follows **the line that changed most recently**, not the cursor.
The cases the glass TTY exists for are the ones where the cursor is the
wrong anchor: the diagnostics and PRAID print without moving a cursor the
way a terminal means it, and a cold-load stream paints lines while the
hardware cursor sits wherever it was left. Where the machine last wrote is
what a person is trying to watch, and it costs nothing to know, because
which lines changed is already what decides what to send.

Three details, each of which is a test in `tests/glass_tty.rs`:

- the lowest changed line is the anchor, because this screen fills downward;
- the window is left alone until the anchor leaves it or comes within three
  lines of its bottom, so output walking down the screen does not move the
  window on every line;
- **a screen where nothing changed leaves the window where it is.** The
  cursor is the anchor only before any line has ever changed. Falling back
  to it on every still screen drags the window off the line the person is
  watching and back to wherever the cursor was left --- which is the failure
  the rule exists to avoid, and one muir had for the length of an afternoon.

### What a client types

A printable character is its own keysym, which is how X11 numbers it and
what `src/terminal/keyboard.rs` takes. Carriage return and carriage
return-line feed both type one Return.

A control character is the letter with Control held around it: Control-A
arrives from a terminal as the byte 1, and the machine, which decodes from
the stream of key positions, has to see Control go down, `a`, and Control
come up --- which is what a typist would have done.

Everything typed reaches the machine through the one
`terminal::keyboard::Keyboard` a run has, which is the same object a VNC
viewer's keys go through. **The machine has one keyboard**, on the I/O
board, and everything that types shares it.

## The flag

    --glass-tty [<endpoint>][,ro]

The endpoint is nothing, a port, an address, or address:port. **Bound to the
loopback unless an address says otherwise**, for the reason `--terminal` is:
telnet offers no authentication at all, and a glass TTY on a routable
address hands the machine's keyboard to whoever can reach it. `ro` is a
client that may watch and not type, spelled as `--disk-pack` spells the
drive's read-only switch because it means the same thing.

The flag may be given more than once, and several clients may attach to each
--- every client sees the same screen, and each keeps its own copy of what it
was last sent, so one joining late is not shown a difference against
something it never saw. A port that was named is bound as it stands and a
run that cannot have it stops; a port that was not named moves up until it
finds one free, so the flag twice is two glass TTYs rather than a collision.

The default port is **10023**, and it is **muir's own number and no
convention**: telnet's own port is 23, and a server on it needs privilege
this has no business asking for.

There is no default at all for whether there is one. A run gets no glass TTY
unless the flag is given, and a run without one says nothing about it.

## Unverified

**That the screen's character grid begins at the top left pixel.** muir
reads cells on an 8-by-12 grid from pixel (0, 0). The window system
positions its lines by sheet margins rather than on that grid, so a window
whose origin is not a multiple of the cell will read as `?` even in the
character font. Nothing here establishes where System 100 puts its windows.
What would settle it: reading a known string off a booted band's screen and
finding the offset that makes it decode. The cold-load stream, PRAID and the
diagnostics all write from the top left, which is why this has not bitten.
