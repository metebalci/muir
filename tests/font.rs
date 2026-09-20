// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The character font, and the screen read back through it.
//!
//! The committed fonts are held to the release's own copies when
//! `vendor/` is there, and say they were skipped when not --- the same
//! bargain `tests/prom.rs` makes for the boot PROM. What is checked
//! besides is that the *two* fonts are two: both are called
//! `FONTS:CPTFONT`, they are different bitmaps, and a readback holding
//! the wrong one matches nothing rather than failing.
//!
//! The rest is a round trip: a glyph drawn into a frame buffer the way
//! the machine draws one, and read back out again.

use muir::terminal::Frame;
use muir::terminal::font::{CHAR_HEIGHT, CHAR_WIDTH, Fonts, Glyph};
use muir::tv::{self, Tv};

mod support;

// --- The committed files are the release's -----------------------------------

/// **The committed fonts are the release's own files.** `mit/README.md`
/// says `mit/sys/` is a snapshot of System 100's `sys` tree at the paths
/// the release uses, byte for byte. A font that drifted would still
/// decode and still render --- it would just read a screen the machine
/// drew with a different bitmap --- so only the release's own bytes
/// settle it.
#[test]
fn the_committed_fonts_are_the_ones_system_100_ships() {
    for (name, ours) in [
        ("cptfont.qfasl", include_bytes!("../mit/sys/fonts/cptfont.qfasl").as_slice()),
        ("cptfon.qfasl", include_bytes!("../mit/sys/fonts/cptfon.qfasl").as_slice()),
    ] {
        let Some(theirs) = support::release_100_file(&["fonts", name]) else {
            eprintln!("skipped: no vendored release to hold {name} to");
            return;
        };
        let theirs = std::fs::read(&theirs).unwrap();
        assert_eq!(theirs.len(), ours.len(), "{name} differs in length from the release's");
        assert!(theirs.as_slice() == ours, "mit/sys/fonts/{name} is not the release's");
    }
}

// --- The fonts declare their own shape ---------------------------------------

/// **The grid comes out of MIT's files and not out of muir.**
///
/// Every round trip below draws a glyph and reads it back on the same
/// pitch, so all of them would pass just as well if that pitch were
/// wrong: they are circular about the one number that decides how a
/// screen is cut into cells. This is the check that is not. The leader of
/// each font is read --- `tvdefs.lisp:517`'s fields, which the file
/// carries --- and held to the constants the readback uses.
#[test]
fn the_fonts_declare_the_grid_the_readback_uses() {
    let fonts = Fonts::new();
    for (name, raster_width) in [("cptfont", 8usize), ("cptfon", 7)] {
        let l = fonts.named(name).expect("one of the two fonts").leader;
        assert_eq!(l.char_height, CHAR_HEIGHT, "{name}: FONT-CHAR-HEIGHT");
        // The pitch, and so the 96 columns the screen is cut into. The
        // two fonts differ in raster width and agree here, which is why
        // a change of font does not change the grid.
        assert_eq!(l.char_width, CHAR_WIDTH, "{name}: FONT-CHAR-WIDTH");
        assert_eq!(l.raster_width, raster_width, "{name}: FONT-RASTER-WIDTH");
        // MIT's own definitions of the two packing fields,
        // `tvdefs.lisp:531` and `:533`.
        assert_eq!(l.rasters_per_word, 32 / l.raster_width, "{name}: FLOOR 32 / RASTER-WIDTH");
        assert_eq!(
            l.words_per_char,
            l.char_height.div_ceil(l.rasters_per_word),
            "{name}: CEILING RASTER-HEIGHT / RASTERS-PER-WORD"
        );
        // The rasters have to reach every row of the cell.
        assert!(
            l.rasters_per_word * l.words_per_char >= CHAR_HEIGHT,
            "{name}: {} rows packed for a cell {CHAR_HEIGHT} tall",
            l.rasters_per_word * l.words_per_char
        );
    }
    assert_eq!(muir::terminal::font::COLS, tv::WIDTH / CHAR_WIDTH, "the columns of the screen");
}

// --- The two fonts are two ---------------------------------------------------

/// **`FONTS:CPTFONT` names two different fonts.** `cptfon.qfasl` is what
/// the cold load brings up and `cptfont.qfasl` is what the full system
/// loads over it, and they are not the same bitmap: the cold load's `I`
/// carries full-width serifs and the system's short ones. This is the
/// check that nothing ever quietly makes them one.
#[test]
fn the_cold_load_and_system_fonts_are_different_bitmaps() {
    let mut pinned = Fonts::new();
    pinned.pin("cptfon").unwrap();
    assert_eq!(pinned.in_force(), "cptfon");
    let mut other = Fonts::new();
    other.pin("cptfont").unwrap();
    assert_eq!(other.in_force(), "cptfont");

    // Drawn from each font's own table, the same letter differs.
    let cold = drawn_with("cptfon", "I");
    let system = drawn_with("cptfont", "I");
    assert_ne!(cold, system, "the two fonts draw `I` the same way");
}

/// **A name that is neither is refused**, rather than silently leaving
/// the readback on whatever it had.
#[test]
fn a_font_that_is_neither_is_refused() {
    let mut fonts = Fonts::new();
    assert!(fonts.pin("tr12").is_err());
    assert!(fonts.pin("cptfont").is_ok());
}

// --- The round trip ----------------------------------------------------------

/// Draws `text` at the top left of a screen, each character's raster
/// taken from the font `name`, the way the machine draws one, and gives
/// back the frame buffer's words.
fn drawn_with(name: &str, text: &str) -> Vec<u32> {
    let mut fonts = Fonts::new();
    fonts.pin(name).unwrap();
    let mut tv = Tv::default();
    for (col, ch) in text.chars().enumerate() {
        let glyph = glyph_of(name, ch as u8);
        draw(&mut tv, col, 0, &glyph);
    }
    tv.buffer().to_vec()
}

/// One character's raster from the named font: what the machine would
/// copy into the frame buffer to draw it.
fn glyph_of(name: &str, c: u8) -> Glyph {
    Fonts::new().named(name).expect("one of the two fonts").glyph(c)
}

/// Sets the pixels of one character cell, bit 0 of a row the leftmost
/// pixel, which is how the frame buffer holds a line.
fn draw(tv: &mut Tv, col: usize, row: usize, glyph: &Glyph) {
    for (r, bits) in glyph.iter().enumerate() {
        let y = row * CHAR_HEIGHT + r;
        for px in 0..CHAR_WIDTH {
            if bits >> px & 1 == 0 {
                continue;
            }
            let x = col * CHAR_WIDTH + px;
            let bit = y * tv::WORDS_PER_LINE * 32 + x;
            let (word, at) = (bit / 32, bit % 32);
            let was = tv.buffer()[word];
            tv.write_buffer(word as u32, was | 1 << at);
        }
    }
}

/// A screen with `text` drawn at the top left in the named font.
fn screen_of(name: &str, text: &str) -> Tv {
    let mut tv = Tv::default();
    for (col, ch) in text.chars().enumerate() {
        draw(&mut tv, col, 0, &glyph_of(name, ch as u8));
    }
    tv
}

/// **What the machine drew reads back as what it drew.** Every printable
/// character of the system font, laid across the screen and read again.
#[test]
fn what_was_drawn_reads_back_as_itself() {
    let line: String = (0x20u8..0x7f).map(|c| c as char).collect();
    let tv = screen_of("cptfont", &line);
    let mut fonts = Fonts::new();
    fonts.pin("cptfont").unwrap();
    let text = fonts.read(&Frame::of(&tv));
    let read: String = text.line(0).iter().collect();
    assert_eq!(read.trim_end(), line.trim_end(), "every printable character");
}

/// The same for the cold-load font, whose rows are seven bits packed four
/// to a word rather than one to a byte: the packing is the thing this
/// catches.
#[test]
fn the_cold_load_font_reads_back_as_itself() {
    let line = "PRAID 3.2 HALTED";
    let tv = screen_of("cptfon", line);
    let mut fonts = Fonts::new();
    fonts.pin("cptfon").unwrap();
    let text = fonts.read(&Frame::of(&tv));
    let read: String = text.line(0).iter().collect();
    assert_eq!(read.trim_end(), line);
}

// --- Which font is in force --------------------------------------------------

/// **A screen is read through the font that drew it.** The machine comes
/// up on the cold-load font and changes to the system one as the band
/// boots over it, so nothing can be told once which to use: both are
/// scored against the screen and the winner is taken.
#[test]
fn the_font_that_drew_the_screen_is_the_one_it_is_read_through() {
    let line = "MIT CADR Lisp Machine";
    for name in ["cptfont", "cptfon"] {
        let tv = screen_of(name, line);
        let mut fonts = Fonts::new();
        let text = fonts.read(&Frame::of(&tv));
        assert_eq!(fonts.in_force(), name, "the font {name} drew it with");
        let read: String = text.line(0).iter().collect();
        assert_eq!(read.trim_end(), line, "read through {name}");
    }
}

/// **A blank screen leaves the font where it was**, because nothing on it
/// tells the two apart. It starts on the system font, which is the common
/// case: a run watching a band that has booted.
#[test]
fn a_blank_screen_leaves_the_font_where_it_was() {
    let mut fonts = Fonts::new();
    assert_eq!(fonts.in_force(), "cptfont", "the system font to begin with");
    let tv = Tv::default();
    fonts.read(&Frame::of(&tv));
    assert_eq!(fonts.in_force(), "cptfont", "nothing on a blank screen separates them");
}

/// **The machine changing font is followed.** A cold-load screen read,
/// then a system-font screen: the readback moves with it rather than
/// filling the second with `?`.
#[test]
fn a_change_of_font_under_the_readback_is_followed() {
    let line = "LISP MACHINE ONE";
    let mut fonts = Fonts::new();
    let cold = screen_of("cptfon", line);
    let text = fonts.read(&Frame::of(&cold));
    assert_eq!(fonts.in_force(), "cptfon", "the cold load's font");
    assert_eq!(text.line(0).iter().collect::<String>().trim_end(), line);

    let system = screen_of("cptfont", line);
    let text = fonts.read(&Frame::of(&system));
    assert_eq!(fonts.in_force(), "cptfont", "the band has booted over it");
    assert_eq!(text.line(0).iter().collect::<String>().trim_end(), line);
}

// --- The cursor --------------------------------------------------------------

/// **The cursor is a cell found inverted.** It is drawn by exclusive-or
/// into the frame buffer like anything else, so under it a glyph is its
/// own complement: the character is still read, and where it was read
/// inverted is where the cursor is.
#[test]
fn the_cursor_is_a_cell_read_inverted() {
    // `A` drawn plainly, and `B` drawn into an empty cell as its own
    // complement, which is what the cursor standing on it leaves.
    let mut tv = screen_of("cptfont", "A");
    let glyph = glyph_of("cptfont", b'B');
    let flipped: Glyph = std::array::from_fn(|r| !glyph[r]);
    draw(&mut tv, 1, 0, &flipped);
    let mut fonts = Fonts::new();
    fonts.pin("cptfont").unwrap();
    let text = fonts.read(&Frame::of(&tv));
    assert_eq!(text.line(0).iter().collect::<String>().trim_end(), "AB", "both characters read");
    assert_eq!(text.cursor, Some((1, 0)), "the cursor is on the second cell");
}
