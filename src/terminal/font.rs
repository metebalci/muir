// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The character font, and the screen read back through it.
//!
//! The machine draws characters by copying a glyph's raster into the frame
//! buffer, so a screen of text can be read back by matching each cell
//! against the same rasters. That is what makes
//! [`super::glass_tty::GlassTty`] possible without a line of the band's own
//! software: the font is the only thing needed to turn pixels into
//! characters, and it is a file.
//!
//! **There are two fonts and both are called `FONTS:CPTFONT`.**
//!
//! - `cptfon`, `sys/fonts/cptfon.qfasl`, is the one the cold load brings
//!   up: `sysdcl.lisp:462`'s `COLD-LOAD-FILE-LIST` begins `"SYS: FONTS;
//!   CPTFON QFASL >"`. Its raster is 7 wide.
//! - `cptfont`, `sys/fonts/cptfont.qfasl`, is the one the full system
//!   fasloads over it, `sysdcl.lisp:126-127`. Its raster is 8 wide.
//!
//! They are different bitmaps under one name --- the cold load's `I` has
//! full-width serifs and the system's has short ones --- so a readback
//! holding the wrong one does not fail loudly, it quietly matches nothing
//! and fills in `?`. **And the machine changes from one to the other as
//! the band boots over the cold load**, so which is in force is not
//! something a run can be told once. [`Fonts::read`] scores both against
//! the screen and takes the winner.
//!
//! The cell pitch is the same either way. `FONT-CHAR-WIDTH` is 8 in both
//! --- the 7-wide font is the same 8-pixel cell with its last column left
//! blank --- so the grid is [`COLS`] by [`ROWS`] whichever font is in
//! force, and only the glyphs change.
//!
//! The layout is MIT's, `sys/window/tvdefs.lisp:517` for the `FONT`
//! defstruct and `tvdefs.lisp:557-561` for the data: "an integral number
//! of words per character. Each word contains an integral number of rows
//! of raster, right adjusted and processed from right to left". The
//! microcode says which way that falls on this machine,
//! `sys/ucadr/uc-tv.lisp:21-39`: "LEFT ADJUSTED AND PROCESSED FROM LEFT TO
//! RIGHT. (RIGHT TO LEFT ON 32-BIT TVS)". The CADR is the 32-bit TV, so
//! row 0 is in the low bits and rows ascend upward through the word.

use std::collections::HashMap;

use super::Frame;
use super::glass_tty::Text;

/// The cell a character is drawn in: `FONT-CHAR-HEIGHT` and
/// `FONT-CHAR-WIDTH` from the leader of both fonts.
pub const CHAR_HEIGHT: usize = 12;
pub const CHAR_WIDTH: usize = 8;

/// The grid the main screen makes at that pitch: 768 by 963 pixels
/// ([`crate::tv`]) is 96 characters by 80, with three pixel rows over.
pub const COLS: usize = crate::tv::WIDTH / CHAR_WIDTH;
pub const ROWS: usize = crate::tv::HEIGHT / CHAR_HEIGHT;

/// How many characters a font defines. `FONT-FILL-POINTER` is 128 in
/// both, "1 plus highest character code defined in font"
/// (`tvdefs.lisp:518`).
const GLYPHS: usize = 128;

/// One character's raster: [`CHAR_HEIGHT`] rows, bit 0 the leftmost
/// pixel, which is the frame buffer's own convention.
pub type Glyph = [u8; CHAR_HEIGHT];

/// MIT's own files, byte for byte the release's: `mit/README.md` says
/// `mit/sys/` is a snapshot of System 100's `sys` tree at the paths the
/// release uses, and `tests/font.rs` holds each of these to the release's
/// copy whenever `vendor/` is there to compare against.
const CPTFONT_QFASL: &[u8] = include_bytes!("../../mit/sys/fonts/cptfont.qfasl");
const CPTFON_QFASL: &[u8] = include_bytes!("../../mit/sys/fonts/cptfon.qfasl");

/// The QFASL operation that introduces the raster, and the count that
/// follows it: both files carry `46 80` and then a count of 16-bit words,
/// `00 03`, which is 768 of them and so 1536 bytes --- 128 characters of
/// three 32-bit words each.
///
/// The raster is found by this rather than at a fixed offset, so that the
/// length comes out of the file instead of out of this comment, and a
/// file that does not carry the operation is refused rather than decoded
/// into nonsense.
const RASTER_OP: [u8; 2] = [0x46, 0x80];

/// The raster of a font file: the bytes after [`RASTER_OP`] and its
/// count.
///
/// **The count is what picks the operation out**, not the operation
/// alone: the two bytes occur earlier in both files as ordinary data, and
/// taking the first of them gave 196 bytes of something else. A font of
/// [`GLYPHS`] characters at [`CHAR_HEIGHT`] rows is that many bytes
/// however its rows are packed --- three 32-bit words a character either
/// way --- so the operation wanted is the one whose count says so.
fn raster(qfasl: &[u8]) -> &[u8] {
    let want = GLYPHS * CHAR_HEIGHT;
    let at = (0..qfasl.len().saturating_sub(4))
        .find(|&k| {
            qfasl[k..k + 2] == RASTER_OP
                && u16::from_le_bytes([qfasl[k + 2], qfasl[k + 3]]) as usize * 2 == want
                && k + 4 + want <= qfasl.len()
        })
        .expect("a font file carries a raster of 128 characters");
    &qfasl[at + 4..at + 4 + want]
}

/// A character font: the glyphs, and what each one means.
pub struct Font {
    /// What MIT calls the file this came from.
    pub name: &'static str,
    /// `FONT-RASTER-WIDTH`: how many of the cell's eight columns the
    /// glyphs use.
    pub raster_width: usize,
    glyphs: Vec<Glyph>,
    /// Every glyph that is not blank, by its rows, for reading a cell
    /// back. Blank is left out so that the space wins it outright: a font
    /// defines characters that are blank besides the space, and a cell of
    /// nothing is a space whichever of them it also matches.
    by_raster: HashMap<Glyph, u8>,
}

impl Font {
    /// The 8-wide system font, `cptfont.qfasl`: one row a byte, twelve
    /// rows, three 32-bit words a character.
    fn system() -> Font {
        let data = raster(CPTFONT_QFASL);
        let mut glyphs = vec![[0u8; CHAR_HEIGHT]; GLYPHS];
        for (c, glyph) in glyphs.iter_mut().enumerate() {
            let at = c * CHAR_HEIGHT;
            glyph.copy_from_slice(&data[at..at + CHAR_HEIGHT]);
        }
        Font::of("cptfont", 8, glyphs)
    }

    /// The 7-wide cold-load font, `cptfon.qfasl`: `FONT-RASTERS-PER-WORD`
    /// is 4, so four 7-bit rows are packed into each 32-bit word at bit
    /// offsets 0, 7, 14 and 21, and the top four bits of every word are
    /// unused --- `tests/font.rs` holds that nothing is ever set there.
    fn cold_load() -> Font {
        let data = raster(CPTFON_QFASL);
        let mut glyphs = vec![[0u8; CHAR_HEIGHT]; GLYPHS];
        for (c, glyph) in glyphs.iter_mut().enumerate() {
            for w in 0..CHAR_HEIGHT / 4 {
                let at = c * CHAR_HEIGHT + w * 4;
                let word = u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
                for r in 0..4 {
                    glyph[w * 4 + r] = (word >> (r * 7)) as u8 & 0x7f;
                }
            }
        }
        Font::of("cptfon", 7, glyphs)
    }

    fn of(name: &'static str, raster_width: usize, glyphs: Vec<Glyph>) -> Font {
        let mut by_raster = HashMap::new();
        for (c, glyph) in glyphs.iter().enumerate() {
            if *glyph != [0; CHAR_HEIGHT] {
                by_raster.entry(*glyph).or_insert(c as u8);
            }
        }
        Font { name, raster_width, glyphs, by_raster }
    }

    /// The raster of one character.
    pub fn glyph(&self, c: u8) -> Glyph {
        self.glyphs.get(c as usize).copied().unwrap_or([0; CHAR_HEIGHT])
    }

    /// The character a cell holds, and whether it was found inverted.
    ///
    /// A blank cell is a space. Otherwise the rows are looked for as they
    /// stand and then complemented, because the cursor is drawn by
    /// exclusive-or into the frame buffer like anything else: under it a
    /// glyph is its own complement, and a cell that matches complemented
    /// is both the character and the cursor.
    pub fn read_cell(&self, cell: &Glyph) -> Option<(char, bool)> {
        if *cell == [0; CHAR_HEIGHT] {
            return Some((' ', false));
        }
        if let Some(&c) = self.by_raster.get(cell) {
            return Some((c as char, false));
        }
        let mask = ((1u16 << self.raster_width) - 1) as u8;
        let flipped: Glyph = std::array::from_fn(|r| !cell[r] & mask);
        if flipped == [0; CHAR_HEIGHT] {
            // A solid block: the cursor with nothing under it.
            return Some((' ', true));
        }
        self.by_raster.get(&flipped).map(|&c| (c as char, true))
    }
}

/// The two fonts, and which of them a screen was last read through.
pub struct Fonts {
    system: Font,
    cold_load: Font,
    /// Which font the last screen scored highest against, and so the one
    /// a screen that tells them apart no better is read through.
    ///
    /// **The system font to begin with.** Scoring cannot separate them on
    /// a blank screen or one showing only characters they agree on, and a
    /// run watching a booted band is the common case.
    last: bool,
    /// Set by `,font=`: the readback is pinned to one font and nothing is
    /// scored. For a test that must know which table it is reading
    /// through, and for the case where the scoring turns out to be wrong
    /// --- a readback that silently picks the wrong font is hard to tell
    /// from a broken decoder at the far end of a socket.
    pinned: Option<bool>,
}

impl Default for Fonts {
    fn default() -> Fonts {
        Fonts::new()
    }
}

impl Fonts {
    pub fn new() -> Fonts {
        Fonts { system: Font::system(), cold_load: Font::cold_load(), last: true, pinned: None }
    }

    /// The name a `,font=` argument may give, and what it pins to.
    pub fn pin(&mut self, name: &str) -> Result<(), String> {
        match name {
            "cptfont" => self.pinned = Some(true),
            "cptfon" => self.pinned = Some(false),
            _ => return Err(format!("{name:?} is neither cptfont nor cptfon")),
        }
        Ok(())
    }

    fn font(&self, system: bool) -> &Font {
        if system { &self.system } else { &self.cold_load }
    }

    /// One of the two by MIT's name for its file, for a caller that must
    /// know which table it is holding: `tests/font.rs` draws with a
    /// font's own glyphs so that the round trip is against the raster and
    /// not against itself.
    pub fn named(&self, name: &str) -> Option<&Font> {
        [&self.system, &self.cold_load].into_iter().find(|f| f.name == name)
    }

    /// Which font is in force, by name.
    pub fn in_force(&self) -> &'static str {
        self.font(self.pinned.unwrap_or(self.last)).name
    }

    /// The cell at character position `x`, `y` of `frame`, as rows of
    /// pixels.
    fn cell(frame: &Frame, x: usize, y: usize) -> Glyph {
        std::array::from_fn(|r| {
            let py = y * CHAR_HEIGHT + r;
            let mut row = 0u8;
            for px in 0..CHAR_WIDTH {
                if frame.shows_white(x * CHAR_WIDTH + px, py) {
                    row |= 1 << px;
                }
            }
            row
        })
    }

    /// The screen as text, through whichever font it reads best through.
    ///
    /// Both fonts are scored by how many cells they read, and the winner
    /// is kept for the next screen: the machine changes font as the band
    /// boots over the cold load, and a run told once which font to use
    /// would be wrong for exactly the run somebody is watching.
    pub fn read(&mut self, frame: &Frame) -> Text {
        let rows = (frame.height / CHAR_HEIGHT).min(ROWS);
        let cols = (frame.width / CHAR_WIDTH).min(COLS);
        let cells: Vec<Glyph> =
            (0..rows).flat_map(|y| (0..cols).map(move |x| (x, y))).map(|(x, y)| Self::cell(frame, x, y)).collect();
        let system = match self.pinned {
            Some(pinned) => pinned,
            None => {
                let score = |f: &Font| cells.iter().filter(|c| f.read_cell(c).is_some()).count();
                let (a, b) = (score(&self.system), score(&self.cold_load));
                // Equal is the font already in force, which on a blank
                // screen is where it started.
                if a == b { self.last } else { a > b }
            }
        };
        self.last = system;
        let font = self.font(system);
        let mut text = Text::blank(cols, rows);
        let mut cursor = None;
        for (k, cell) in cells.iter().enumerate() {
            let (c, inverted) = font.read_cell(cell).unwrap_or(('?', false));
            text.cells[k] = c;
            if inverted && cursor.is_none() {
                cursor = Some((k % cols, k / cols));
            }
        }
        text.cursor = cursor;
        text
    }
}
