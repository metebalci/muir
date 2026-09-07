// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading a band: a disk partition holding a saved Lisp world.
//!
//! Two kinds of band exist and this module reads the system communication
//! area of both.  `make-cold` writes an *uncompressed* band, in which virtual
//! address N is word N of the partition; `DISK-SAVE` writes a *compressed*
//! one, which has no such mapping.  `%SYS-COM-BAND-FORMAT` says which:
//! `1000` octal is compressed and anything else is the plain format
//! (`cold/qcom.lisp:176-180`), and the microcode agrees --- "Non-compressed
//! band (must be a cold-load band, I think)", `ucadr/uc-cold-disk.lisp:341`.
//! Only the uncompressed mapping is implemented, so [`Band::read`] refuses on
//! a compressed band rather than returning a word that means nothing.
//!
//! The pack's label is here too --- read, laid out and written --- because
//! finding a band by name is the only thing muir itself needs it for: the
//! disk code below this works in blocks and does not know partitions exist.
//! The writing side is what `diskpack` edits a pack through, and it is here
//! rather than beside the tool so that one description of the format serves
//! both directions.
//!
//! Field positions come from `sys/cold/qcom.lisp` in the System 100 release;
//! the label's layout comes from the boot PROM, which is authority for its
//! own formats, and what a *fresh* label holds comes from MIT's own label
//! editor, `sys/io/dledit.lisp`.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::disk_unit::Geometry;

/// Words in a disk block.  The same 256 as [`crate::disk_unit::BLOCK_WORDS`].
pub const BLOCK_WORDS: usize = 256;

/// Words in a page of virtual memory.  `PAGE-SIZE`, `cold/qcom.lisp`.
pub const PAGE_WORDS: u32 = 256;

/// Width of the pointer field of a Q, from `%%Q-POINTER 0031`.
///
/// A band states its own pointer width in `%SYS-COM-POINTER-WIDTH`; both
/// bands on the vendored pack say 25, which `tests/band.rs` checks against
/// this constant.
pub const POINTER_BITS: u32 = 25;

/// One 32-bit tagged word.
///
/// The three fields are byte specifiers in `Q-FIELD-VALUES`
/// (`cold/qcom.lisp:48-58`), written there as octal position-and-size pairs:
/// `%%Q-CDR-CODE 3602`, `%%Q-DATA-TYPE 3105`, `%%Q-POINTER 0031`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Q(pub u32);

impl Q {
    /// `Q<31:30>` --- `%%Q-CDR-CODE 3602`: position 0o36, size 2.
    pub fn cdr_code(self) -> u8 {
        (self.0 >> 30) as u8 & 0b11
    }

    /// `Q<29:25>` --- `%%Q-DATA-TYPE 3105`: position 0o31, size 5.
    ///
    /// Five bits, not seven.  The seven at `Q<31:25>` are
    /// `%%Q-ALL-BUT-POINTER 3107`, which is the data type *and* the CDR code
    /// together; reading that as the type alone works only on words whose CDR
    /// code happens to be zero.
    pub fn data_type(self) -> u8 {
        (self.0 >> POINTER_BITS) as u8 & 0b1_1111
    }

    /// `Q<24:0>` --- `%%Q-POINTER 0031`: position 0, size 0o31.
    pub fn pointer(self) -> u32 {
        self.0 & ((1 << POINTER_BITS) - 1)
    }

    /// True if this Q is NIL: `DTP-SYMBOL` pointing at zero.
    ///
    /// `make-cold` builds NIL first and at the base of the symbol area, so its
    /// pointer is zero and stays zero (`coldut.lisp:1283-1291`).
    pub fn is_nil(self) -> bool {
        self.data_type() == dtp::SYMBOL && self.pointer() == 0
    }

    /// The name of this Q's data type, for messages.
    pub fn type_name(self) -> &'static str {
        dtp::NAMES.get(self.data_type() as usize).copied().unwrap_or("<out of range>")
    }
}

/// CDR codes, in the order of `Q-CDR-CODES` (`cold/qcom.lisp:41-45`):
/// "Numeric values of CDR codes, right-justified in word for %P-CDR-CODE".
pub mod cdr {
    pub const NORMAL: u8 = 0;
    pub const ERROR: u8 = 1;
    pub const NIL: u8 = 2;
    pub const NEXT: u8 = 3;

    /// Indexed by the code.
    pub const NAMES: [&str; 4] = ["CDR-NORMAL", "CDR-ERROR", "CDR-NIL", "CDR-NEXT"];
}

/// Data type codes, in the order of `Q-DATA-TYPES` (`cold/qcom.lisp:10-28`).
///
/// Twenty-nine of them, which is why the field is five bits wide.
pub mod dtp {
    pub const TRAP: u8 = 0;
    pub const NULL: u8 = 1;
    pub const FREE: u8 = 2;
    pub const SYMBOL: u8 = 3;
    pub const SYMBOL_HEADER: u8 = 4;
    pub const FIX: u8 = 5;
    pub const EXTENDED_NUMBER: u8 = 6;
    pub const HEADER: u8 = 7;
    pub const GC_FORWARD: u8 = 8;
    pub const EXTERNAL_VALUE_CELL_POINTER: u8 = 9;
    pub const ONE_Q_FORWARD: u8 = 10;
    pub const HEADER_FORWARD: u8 = 11;
    pub const BODY_FORWARD: u8 = 12;
    pub const LOCATIVE: u8 = 13;
    pub const LIST: u8 = 14;
    pub const U_ENTRY: u8 = 15;
    pub const FEF_POINTER: u8 = 16;
    pub const ARRAY_POINTER: u8 = 17;
    pub const ARRAY_HEADER: u8 = 18;
    pub const STACK_GROUP: u8 = 19;
    pub const CLOSURE: u8 = 20;
    pub const SMALL_FLONUM: u8 = 21;
    pub const SELECT_METHOD: u8 = 22;
    pub const INSTANCE: u8 = 23;
    pub const INSTANCE_HEADER: u8 = 24;
    pub const ENTITY: u8 = 25;
    pub const STACK_CLOSURE: u8 = 26;
    pub const SELF_REF_POINTER: u8 = 27;
    pub const CHARACTER: u8 = 28;

    /// Indexed by the code, so `NAMES[LOCATIVE as usize]` is `"DTP-LOCATIVE"`.
    pub const NAMES: [&str; 29] = [
        "DTP-TRAP",
        "DTP-NULL",
        "DTP-FREE",
        "DTP-SYMBOL",
        "DTP-SYMBOL-HEADER",
        "DTP-FIX",
        "DTP-EXTENDED-NUMBER",
        "DTP-HEADER",
        "DTP-GC-FORWARD",
        "DTP-EXTERNAL-VALUE-CELL-POINTER",
        "DTP-ONE-Q-FORWARD",
        "DTP-HEADER-FORWARD",
        "DTP-BODY-FORWARD",
        "DTP-LOCATIVE",
        "DTP-LIST",
        "DTP-U-ENTRY",
        "DTP-FEF-POINTER",
        "DTP-ARRAY-POINTER",
        "DTP-ARRAY-HEADER",
        "DTP-STACK-GROUP",
        "DTP-CLOSURE",
        "DTP-SMALL-FLONUM",
        "DTP-SELECT-METHOD",
        "DTP-INSTANCE",
        "DTP-INSTANCE-HEADER",
        "DTP-ENTITY",
        "DTP-STACK-CLOSURE",
        "DTP-SELF-REF-POINTER",
        "DTP-CHARACTER",
    ];
}

/// Indices into the system communication area, in the order of
/// `SYSTEM-COMMUNICATION-AREA-QS` (`cold/qcom.lisp:138-222`).
///
/// `coldut.lisp:1046` asserts this count, so a 28th name here would be a bug
/// rather than an extension.
pub mod sys_com {
    pub const AREA_ORIGIN_PNTR: usize = 0;
    pub const VALID_SIZE: usize = 1;
    pub const PAGE_TABLE_PNTR: usize = 2;
    pub const PAGE_TABLE_SIZE: usize = 3;
    pub const OBARRAY_PNTR: usize = 4;
    pub const ETHER_FREE_LIST: usize = 5;
    pub const ETHER_TRANSMIT_LIST: usize = 6;
    pub const ETHER_RECEIVE_LIST: usize = 7;
    pub const BAND_FORMAT: usize = 8;
    pub const GC_GENERATION_NUMBER: usize = 9;
    pub const UNIBUS_INTERRUPT_LIST: usize = 10;
    pub const TEMPORARY: usize = 11;
    pub const FREE_AREA_NUMBER_LIST: usize = 12;
    pub const FREE_REGION_NUMBER_LIST: usize = 13;
    pub const MEMORY_SIZE: usize = 14;
    pub const WIRED_SIZE: usize = 15;
    pub const CHAOS_FREE_LIST: usize = 16;
    pub const CHAOS_TRANSMIT_LIST: usize = 17;
    pub const CHAOS_RECEIVE_LIST: usize = 18;
    pub const DEBUGGER_REQUESTS: usize = 19;
    pub const DEBUGGER_KEEP_ALIVE: usize = 20;
    pub const DEBUGGER_DATA_1: usize = 21;
    pub const DEBUGGER_DATA_2: usize = 22;
    pub const MAJOR_VERSION: usize = 23;
    pub const DESIRED_MICROCODE_VERSION: usize = 24;
    pub const HIGHEST_VIRTUAL_ADDRESS: usize = 25;
    pub const POINTER_WIDTH: usize = 26;

    /// How many there are.
    pub const COUNT: usize = 27;

    /// Virtual address of the first of them.
    ///
    /// "LOCATIONS RELATIVE TO 400 IN CADR" (`cold/qcom.lisp:139`), so the area
    /// starts at word 0o400 of the band --- which is block 1, word 0.
    pub const ORIGIN: u32 = 0o400;
}

/// `%SYS-COM-BAND-FORMAT` for the compressed format (`cold/qcom.lisp:176-180`).
pub const BAND_FORMAT_COMPRESSED: u32 = 0o1000;

/// One entry of the label's partition table.
///
/// The numbers are whatever the label holds: a pack is a file, and a start
/// and a size read off one are not promises.  [`Partition::end`] is where
/// arithmetic on them is done.
#[derive(Clone, Debug)]
pub struct Partition {
    /// Four characters, packed one to a byte low byte first, as `LABL` is.
    pub name: String,
    /// First block of the partition, from the start of the pack.
    pub start: u32,
    pub blocks: u32,
    /// What last wrote the partition said about it.  `make-cold` sets this to
    /// `"cold "` and the date (`coldut.lisp:1215-1218`); `DISK-SAVE` sets it
    /// from `SYSTEM-VERSION-INFO` (`qmisc.lisp:1196-1201`).
    pub comment: String,
}

impl Partition {
    /// The block after the last one in it, saturated rather than wrapped: a
    /// label that says a partition of 4,294,967,295 blocks starts at block
    /// 300 is a label to refuse, not one to overflow on.
    pub fn end(&self) -> u32 {
        self.start.saturating_add(self.blocks)
    }
}

/// The pack label: block 0, which says how the pack is formatted and what
/// partitions are on it.
///
/// Every field position here is read off the boot PROM's `DECODE-LABEL` and
/// `SEARCH-LABEL` (`mit/sys/ucadr/promh.text:451-500`), which is the machine's own
/// authority for this format --- the PROM cannot boot if it is wrong about
/// where the partition table is.
#[derive(Clone, Debug)]
pub struct Label {
    pub version: u32,
    pub cylinders: u32,
    pub heads: u32,
    pub blocks_per_track: u32,
    pub blocks_per_cylinder: u32,
    /// Word 6: the partition the PROM loads microcode from.
    pub microload_partition: String,
    /// Word 7, which MIT's label editor prints as "current virtual memory
    /// load (band)" (`io/dledit.lisp:238`).  Nothing in muir reads it: the
    /// PROM is told which band to load by the console switches, and this is
    /// what the software offers as the default.
    pub current_band: String,
    /// "Brand name of drive", 32 characters at word 0o10
    /// (`io/dledit.lisp:379`).
    pub drive: String,
    /// "Name of pack", 32 characters at word 0o20 (`io/dledit.lisp:380`).
    pub pack_name: String,
    /// "Comment", 96 characters at word 0o30 (`io/dledit.lisp:381`).  MIT's
    /// own field is that wide; usim's `diskmaker` gives it 32, and the
    /// distribution pack does not settle it --- everything past its
    /// "System 100 Distribution Tape" is zero --- so MIT's own writer wins.
    pub comment: String,
    pub words_per_descriptor: u32,
    pub partitions: Vec<Partition>,
    image: PathBuf,
}

/// `LABL`, in the same four-bytes-to-a-word encoding as a partition name.
///
/// "First location of label must be ascii LABL" --- the PROM's own comment at
/// `mit/sys/ucadr/promh.text:453`.
const LABL: u32 = 0o11420440514;

/// Word 0o200 is the number of partitions, 0o201 the width of a descriptor
/// and the table starts at 0o202.  The PROM reaches 0o200 as `DPB M-ONES
/// (BYTE-FIELD 1 7) A-ZERO` --- bit 7 alone --- and MIT's editor writes the
/// three the same way round (`io/dledit.lisp:382-384`).
const COUNT_AT: usize = 0o200;
const WIDTH_AT: usize = 0o201;
const TABLE_AT: usize = 0o202;

/// The most partitions a label can hold: the table starts at 0o202 and a
/// block is 0o400 words, so seven-word descriptors leave room for exactly
/// eighteen --- which is what the System 100 pack has.
pub const MAX_PARTITIONS: usize = (BLOCK_WORDS - TABLE_AT) / 7;

/// Word 6 and word 7: the current microload and the current band, four
/// characters each (`io/dledit.lisp:377-378`).
const MICROLOAD_AT: usize = 6;
const BAND_AT: usize = 7;

/// The label's text fields: where each starts and how many characters it
/// holds, from `LE-INITIALIZE-LABEL` (`io/dledit.lisp:379-381`).  Four
/// characters to a word, so 32 characters is eight words.
const DRIVE: (usize, usize) = (0o10, 32);
const PACK_NAME: (usize, usize) = (0o20, 32);
const COMMENT: (usize, usize) = (0o30, 96);

fn name_of(word: u32) -> String {
    (0..4).map(|i| (word >> (8 * i)) as u8 as char).filter(|c| *c != '\0').collect()
}

fn text_of(words: &[u32]) -> String {
    words.iter().flat_map(|w| (0..4).map(move |i| (w >> (8 * i)) as u8 as char)).collect()
}

/// One of the label's text fields, with the padding taken off.
fn field_of(words: &[u32], (at, chars): (usize, usize)) -> String {
    text_of(&words[at..at + chars / 4]).trim_end_matches('\0').trim_end().to_string()
}

/// Four characters as a word, the first in the low byte, which is how the
/// PROM reads `LABL` and a partition name.  Longer than four, or a character
/// a byte cannot hold, and the caller is told rather than having it cut.
fn word_of(what: &str, name: &str) -> Result<u32, String> {
    let b = name.as_bytes();
    if b.len() > 4 || !name.is_ascii() {
        return Err(format!("{what} is {name}: a name is four ASCII characters or fewer"));
    }
    Ok(b.iter().enumerate().fold(0u32, |w, (i, c)| w | (*c as u32) << (8 * i)))
}

/// Text into a run of words, four characters to a word, zero-padded to the
/// end of the field.
fn put_text(w: &mut [u32], (at, chars): (usize, usize), what: &str, s: &str) -> Result<(), String> {
    if s.len() > chars || !s.is_ascii() {
        return Err(format!("the {what} is {} characters: the label holds {chars}", s.len()));
    }
    for (i, c) in s.bytes().enumerate() {
        w[at + i / 4] |= (c as u32) << (8 * (i % 4));
    }
    Ok(())
}

fn read_words(image: &Path, block: u32, words: usize) -> Result<Vec<u32>, String> {
    let mut f = File::open(image).map_err(|e| format!("{}: {e}", image.display()))?;
    f.seek(SeekFrom::Start(block as u64 * BLOCK_WORDS as u64 * 4))
        .map_err(|e| format!("{}: {e}", image.display()))?;
    let mut bytes = vec![0u8; words * 4];
    f.read_exact(&mut bytes).map_err(|e| match e.kind() {
        // What `read_exact` says of a short file is "failed to fill whole
        // buffer", which is about the buffer and not about the pack.
        std::io::ErrorKind::UnexpectedEof => {
            format!("{}: the file ends inside block {block}", image.display())
        }
        _ => format!("{}: {e}", image.display()),
    })?;
    // The image holds each 32-bit word low byte first, the same way round as
    // `disk_unit::Unit::read_block` reads it; `tests/disk.rs` pins that.
    Ok(bytes.as_chunks::<4>().0.iter().copied().map(u32::from_le_bytes).collect())
}

impl Label {
    /// Reads block 0 of a pack image.
    pub fn open(image: &Path) -> Result<Label, String> {
        let w = read_words(image, 0, BLOCK_WORDS)?;
        if w[0] != LABL {
            return Err(format!("block 0 is not a label: word 0 is {:o}, not {LABL:o}", w[0]));
        }
        if w[1] != 1 {
            return Err(format!("label version is {}, not 1", w[1]));
        }
        let count = w[COUNT_AT] as usize;
        let width = w[WIDTH_AT] as usize;
        if width < 3 {
            return Err(format!(
                "partition descriptors are {width} words, too few for name, start and size"
            ));
        }
        let end = TABLE_AT + count * width;
        if end > BLOCK_WORDS {
            return Err(format!("{count} descriptors of {width} words overrun the label"));
        }
        let partitions = (0..count)
            .map(|i| {
                let d = &w[TABLE_AT + i * width..TABLE_AT + (i + 1) * width];
                Partition {
                    name: name_of(d[0]),
                    start: d[1],
                    blocks: d[2],
                    comment: text_of(&d[3..]).trim_end_matches('\0').trim_end().to_string(),
                }
            })
            .collect();
        Ok(Label {
            version: w[1],
            cylinders: w[2],
            heads: w[3],
            blocks_per_track: w[4],
            blocks_per_cylinder: w[5],
            microload_partition: name_of(w[MICROLOAD_AT]),
            current_band: name_of(w[BAND_AT]),
            drive: field_of(&w, DRIVE),
            pack_name: field_of(&w, PACK_NAME),
            comment: field_of(&w, COMMENT),
            words_per_descriptor: w[WIDTH_AT],
            partitions,
            image: image.to_path_buf(),
        })
    }

    pub fn partition(&self, name: &str) -> Option<&Partition> {
        self.partitions.iter().find(|p| p.name == name)
    }

    /// Reads the band in the named partition.
    pub fn band(&self, name: &str) -> Result<Band, String> {
        let p = self.partition(name).ok_or_else(|| format!("no partition named {name}"))?;
        Band::open(&self.image, p.start, p.blocks)
    }

    /// The pack this label is on.
    pub fn image(&self) -> &Path {
        &self.image
    }

    /// The same label on another pack.
    ///
    /// MIT's editor has this too, and for the same reason a simulator wants
    /// it: "Copy the label from unit 0 on this machine to unit 0 of the
    /// debuggee machine.  You may want to use this right after formatting the
    /// debuggee's disk pack" (`io/dledit.lisp:163-164`).
    pub fn on(&self, image: &Path) -> Label {
        Label { image: image.to_path_buf(), ..self.clone() }
    }

    /// The drive the label describes, for the code below this which works in
    /// cylinders, heads and blocks.
    pub fn geometry(&self) -> Geometry {
        Geometry {
            cylinders: self.cylinders,
            heads: self.heads,
            blocks_per_track: self.blocks_per_track,
        }
    }

    /// Blocks on the pack, the way the label itself counts them: cylinders
    /// times the blocks-per-cylinder word, which is what MIT's editor uses
    /// for the end of the last partition (`io/dledit.lisp:270-272`).
    pub fn blocks(&self) -> u32 {
        self.cylinders * self.blocks_per_cylinder
    }

    /// The first block a partition may have: one track is reserved ---
    /// "First partition starts at block 17. (first track reserved)"
    /// (`io/dledit.lisp:346`), 17 being the T-300's blocks per track.
    pub fn first_block(&self) -> u32 {
        self.blocks_per_track
    }

    /// A fresh label for a pack of this type, as `LE-INITIALIZE-LABEL` writes
    /// one (`io/dledit.lisp:370-398`): the geometry, the drive's brand name,
    /// `MCR1` as the current microload and `LOD1` as the current band, and
    /// the type's partitions laid out from the first block after the
    /// reserved track.
    ///
    /// Nothing is written until [`Label::write`].
    pub fn initialize(image: &Path, t: &PackType) -> Label {
        let mut label = Label {
            version: 1,
            cylinders: t.geometry.cylinders,
            heads: t.geometry.heads,
            blocks_per_track: t.geometry.blocks_per_track,
            blocks_per_cylinder: t.geometry.blocks_per_cylinder(),
            microload_partition: "MCR1".to_string(),
            current_band: "LOD1".to_string(),
            drive: t.drive.to_string(),
            // MIT's own two placeholders, which say the pack is nobody's yet.
            pack_name: "(name)".to_string(),
            comment: "(comment)".to_string(),
            words_per_descriptor: 7,
            partitions: t
                .partitions
                .iter()
                .map(|(name, size)| Partition {
                    name: name.to_string(),
                    start: 0,
                    blocks: t.blocks_of(*size),
                    comment: String::new(),
                })
                .collect(),
            image: image.to_path_buf(),
        };
        // MIT's own layout, and the only place a size's cylinder-ness is
        // known: a size given in blocks is laid where it falls, and one given
        // in cylinders begins at a cylinder boundary, `(* BPC (CEILING BLOCK
        // BPC))`.  A label records neither --- it holds blocks --- so nothing
        // below can tell them apart afterwards.
        let mut block = label.first_block();
        for ((_, size), p) in t.partitions.iter().zip(&mut label.partitions) {
            if *size > 0 {
                block = block.div_ceil(t.geometry.blocks_per_cylinder());
                block *= t.geometry.blocks_per_cylinder();
            }
            p.start = block;
            block += p.blocks;
        }
        label
    }

    /// The next cylinder boundary at or after a block: `(* BPC (CEILING
    /// BLOCK BPC))`, which is where MIT's own layout puts a partition whose
    /// size it was given in cylinders (`io/dledit.lisp:391`).  It rounds up,
    /// so a block already on a boundary stays where it is and every other
    /// one moves forward, never back.
    pub fn at_cylinder(&self, block: u32) -> u32 {
        let bpc = self.blocks_per_cylinder.max(1);
        block.div_ceil(bpc) * bpc
    }

    /// Pushes the entry at `from`, and every one after it, up out of the way
    /// of the one before it.  An entry with room where it is stays where it
    /// is.
    ///
    /// Entries are all this moves.  What a partition holds sits at the blocks
    /// it sat at before, whatever the table says about it afterwards, and
    /// nothing here reads or writes one.  That is also why nothing is ever
    /// moved *down*: one that grows pushes the entries after it up, and one
    /// that shrinks or goes leaves its blocks free where they were rather
    /// than dragging a band's entry down onto blocks the band is not on.  It also leaves alone the
    /// gap MIT's own layout puts before a partition it starts at a cylinder
    /// boundary --- 91 blocks before `PAGE` on a T-300 --- which packing
    /// everything end to end would quietly close.
    ///
    /// An entry that is pushed is put where it falls and not at a cylinder
    /// boundary: the label holds blocks and does not say whether a size was
    /// once given in cylinders.  That is what the System 100 pack does with
    /// its 290 blocks of extra `PAGE`: `LOD1` follows immediately at block
    /// 66828, which is not a cylinder boundary, where a fresh table of MIT's
    /// own has it at 66538, which is.
    pub fn lay_out(&mut self, from: usize) {
        let from = from.min(self.partitions.len());
        let mut block = match from.checked_sub(1).and_then(|i| self.partitions.get(i)) {
            Some(p) => p.end(),
            None => self.first_block(),
        };
        for p in &mut self.partitions[from..] {
            p.start = p.start.max(block);
            block = p.start.saturating_add(p.blocks);
        }
    }

    /// The label as block 0 of the pack: the 0o400 words [`Label::open`]
    /// reads back.
    ///
    /// Refused rather than written short: a name or a comment too long for
    /// its field, and a partition table too long for the block.
    pub fn words(&self) -> Result<Vec<u32>, String> {
        let mut w = vec![0u32; BLOCK_WORDS];
        w[0] = LABL;
        w[1] = self.version;
        w[2] = self.cylinders;
        w[3] = self.heads;
        w[4] = self.blocks_per_track;
        w[5] = self.blocks_per_cylinder;
        w[MICROLOAD_AT] = word_of("the current microload", &self.microload_partition)?;
        w[BAND_AT] = word_of("the current band", &self.current_band)?;
        put_text(&mut w, DRIVE, "drive name", &self.drive)?;
        put_text(&mut w, PACK_NAME, "pack name", &self.pack_name)?;
        put_text(&mut w, COMMENT, "comment", &self.comment)?;

        let width = self.words_per_descriptor as usize;
        if width < 3 {
            return Err(format!(
                "partition descriptors are {width} words, too few for name, start and size"
            ));
        }
        let most = (BLOCK_WORDS - TABLE_AT) / width;
        if self.partitions.len() > most {
            return Err(format!(
                "{} partitions of {width} words do not fit in the label: it holds {most}",
                self.partitions.len()
            ));
        }
        w[COUNT_AT] = self.partitions.len() as u32;
        w[WIDTH_AT] = self.words_per_descriptor;
        for (i, p) in self.partitions.iter().enumerate() {
            let at = TABLE_AT + i * width;
            w[at] = word_of("a partition name", &p.name)?;
            w[at + 1] = p.start;
            w[at + 2] = p.blocks;
            let field = (at + 3, 4 * (width - 3));
            put_text(&mut w, field, &format!("comment on {}", p.name), &p.comment)?;
        }
        Ok(w)
    }

    /// Writes the label to block 0, making the pack's file if it is not
    /// there and sizing it to the geometry.
    ///
    /// A file that is there already has to be a pack of this geometry ---
    /// exactly the size `crate::disk_unit::Unit::open` insists on --- and
    /// anything else is refused untouched.  This is the one call that makes a
    /// 257 MiB file, the path it is given came from someone's shell, and a
    /// path that is a typo is usually a file that exists: making a pack of
    /// it, over whatever it held, is not something to do because a name was
    /// mistyped.  A new pack's file is sparse, so it costs what is written to
    /// it and not what it spans.
    pub fn write(&self) -> Result<(), String> {
        let words = self.words()?;
        if self.blocks_per_cylinder != self.heads * self.blocks_per_track {
            return Err(format!(
                "the label says {} blocks per cylinder and {} heads of {} blocks, which is {}: \
                 a pack whose label contradicts itself has no size to write",
                self.blocks_per_cylinder,
                self.heads,
                self.blocks_per_track,
                self.heads * self.blocks_per_track
            ));
        }
        let want = self.geometry().blocks() as u64 * BLOCK_WORDS as u64 * 4;
        let name = self.image.display();
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.image)
            .map_err(|e| format!("{name}: {e}"))?;
        let have = f.metadata().map_err(|e| format!("{name}: {e}"))?.len();
        if have == 0 {
            f.set_len(want).map_err(|e| format!("{name}: {e}"))?;
        } else if have != want {
            return Err(format!(
                "{name} is {have} bytes and a pack of this drive is {want}: \
                 it is not one, and this does not write over it"
            ));
        }
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        f.rewind().map_err(|e| format!("{name}: {e}"))?;
        f.write_all(&bytes).map_err(|e| format!("{name}: {e}"))
    }
}

/// A kind of drive, and what a fresh pack in it is partitioned into.
///
/// MIT's own `PACK-TYPES` (`io/dledit.lisp:339-367`), the list its label
/// editor offers: "Each element is a 4-list of Pack brand name (32 or fewer
/// chars) (as a symbol).  Number of cylinders.  Number of heads.  Number of
/// blocks per track.  Partition list: name, size (- blocks, + cylinders at
/// cyl bndry)".
pub struct PackType {
    /// The brand name, which goes into the label as the drive.
    pub drive: &'static str,
    pub geometry: Geometry,
    /// Name and size, MIT's way round: a negative size is that many blocks,
    /// a positive one that many cylinders.
    pub partitions: &'static [(&'static str, i32)],
}

impl PackType {
    /// A size from the list in blocks.
    pub fn blocks_of(&self, size: i32) -> u32 {
        if size < 0 {
            size.unsigned_abs()
        } else {
            size as u32 * self.geometry.blocks_per_cylinder()
        }
    }
}

/// The T-300, the drive a CADR's pack goes in and the only one this makes.
///
/// MIT's own `PACK-TYPES` (`io/dledit.lisp:347-367`) has three, the others
/// being a T-80 and a Fujitsu Eagle; the Eagle is a Lambda's drive, with
/// `LMC` microcode partitions to match, and a T-80 is a smaller pack of the
/// same kind with no reason to make one. What is kept here is the entry for
/// the drive the System 100 pack is on.
///
/// The sizes are MIT's, read out of a list written in octal: `-224` there is
/// 148 blocks, and `202.`, `75.` and `9.` are decimal cylinders. The table
/// fills the pack exactly, FILE ending at its last block, which is the check
/// that these numbers came across right.
///
/// The System 100 distribution pack is *not* this table: its PAGE is 65536
/// blocks rather than the 65246 that 202 cylinders come to --- 65536 pages is
/// what a 24-bit address space holds, which MIT's own comment claims for 202
/// cylinders and is 290 blocks short of --- with everything after it moved up
/// and FILE 290 blocks shorter, so that it still ends at the last block.  So
/// a pack initialized from this table is a T-300 as MIT laid one out, not a
/// copy of the pack we boot.
pub const T300: PackType = PackType {
    drive: "Trident T-300",
    geometry: Geometry::T300,
    partitions: &[
        ("MCR1", -148),
        ("MCR2", -148),
        ("MCR3", -148),
        ("MCR4", -148),
        ("MCR5", -148),
        ("MCR6", -148),
        ("MCR7", -148),
        ("MCR8", -148),
        ("PAGE", 202),
        ("LOD1", 75),
        ("LOD2", 75),
        ("LOD3", 75),
        ("LOD4", 75),
        ("LOD5", 75),
        ("LOD6", 75),
        ("LOD7", 75),
        ("LOD8", 75),
        ("FILE", 9),
    ],
};

/// A band, read far enough to answer for itself.
///
/// Always holds the system communication area.  For an uncompressed band it
/// also holds the live words, so [`Band::read`] can follow pointers.
#[derive(Clone, Debug)]
pub struct Band {
    sys_com: [Q; sys_com::COUNT],
    /// Virtual address N is `words[N]`, for N below the valid size.  Empty on
    /// a compressed band, which has no such mapping.
    words: Vec<u32>,
}

impl Band {
    /// Reads a band out of a pack image, given where its partition starts.
    pub fn open(image: &Path, start_block: u32, blocks: u32) -> Result<Band, String> {
        // The system communication area begins at word 0o400, which is block 1
        // word 0, so one block is enough to learn the format and the size.
        // The start is the label's word, and a label is data off the pack.
        let Some(head_block) = start_block.checked_add(1) else {
            return Err(format!(
                "a partition starting at block {start_block} has no block after it"
            ));
        };
        let head = read_words(image, head_block, sys_com::COUNT)?;
        let mut sys_com = [Q(0); sys_com::COUNT];
        for (q, w) in sys_com.iter_mut().zip(&head) {
            *q = Q(*w);
        }
        let mut band = Band { sys_com, words: Vec::new() };
        if !band.compressed() {
            let want = band.valid_size() as usize;
            let have = blocks as usize * BLOCK_WORDS;
            if want > have {
                return Err(format!(
                    "the band says {want} words are valid, more than the {have} in its partition"
                ));
            }
            band.words = read_words(image, start_block, want)?;
        }
        Ok(band)
    }

    /// Builds a band from words already in hand, virtual address 0 first.
    ///
    /// The path a writer's output takes to be checked by this reader.
    pub fn from_words(words: Vec<u32>) -> Result<Band, String> {
        let at = sys_com::ORIGIN as usize;
        if words.len() < at + sys_com::COUNT {
            return Err(format!(
                "only {} words, too few to hold the system communication area",
                words.len()
            ));
        }
        let mut sys_com = [Q(0); sys_com::COUNT];
        for (i, q) in sys_com.iter_mut().enumerate() {
            *q = Q(words[at + i]);
        }
        Ok(Band { sys_com, words })
    }

    /// The 27 system communication Qs, indexed by [`sys_com`].
    pub fn sys_com(&self) -> &[Q; sys_com::COUNT] {
        &self.sys_com
    }

    /// One word of the band, by virtual address.
    ///
    /// `None` past the valid size, and `None` for every address on a
    /// compressed band --- there, word N of the partition is not virtual
    /// address N, and decompression is not implemented.
    pub fn read(&self, va: u32) -> Option<Q> {
        if va >= self.valid_size() {
            return None;
        }
        self.words.get(va as usize).copied().map(Q)
    }

    /// A system communication field read as a number.
    ///
    /// The pointer field, whatever the type: `DISK-SAVE` leaves several of
    /// these untagged --- `LOD2` stores its valid size and band format as
    /// `DTP-TRAP` --- so insisting on `DTP-FIX` would refuse a band the
    /// machine itself reads happily.
    fn number(&self, field: usize) -> u32 {
        self.sys_com[field].pointer()
    }

    /// `%SYS-COM-BAND-FORMAT`: `1000` octal for the compressed format,
    /// anything else for the plain one.
    pub fn band_format(&self) -> u32 {
        self.number(sys_com::BAND_FORMAT)
    }

    pub fn compressed(&self) -> bool {
        self.band_format() == BAND_FORMAT_COMPRESSED
    }

    /// `%SYS-COM-VALID-SIZE`: "IN A SAVED BAND, NUMBER OF WORDS USED".
    pub fn valid_size(&self) -> u32 {
        self.number(sys_com::VALID_SIZE)
    }

    /// Live pages: the valid size in units of `PAGE-SIZE`.
    pub fn valid_pages(&self) -> u32 {
        self.valid_size() / PAGE_WORDS
    }

    /// `%SYS-COM-MEMORY-SIZE`: "Number of words of main memory".
    pub fn memory_size(&self) -> u32 {
        self.number(sys_com::MEMORY_SIZE)
    }

    /// `%SYS-COM-WIRED-SIZE`: "Number words of low memory wired down".
    pub fn wired_size(&self) -> u32 {
        self.number(sys_com::WIRED_SIZE)
    }

    pub fn page_table_size(&self) -> u32 {
        self.number(sys_com::PAGE_TABLE_SIZE)
    }

    pub fn highest_virtual_address(&self) -> u32 {
        self.number(sys_com::HIGHEST_VIRTUAL_ADDRESS)
    }

    /// `%SYS-COM-MAJOR-VERSION`, or `None` in a fresh cold load.
    ///
    /// `coldut.lisp:1077` writes NIL there with the comment ";I.e. fresh
    /// cold-load"; system initialization fills it in.
    pub fn major_version(&self) -> Option<u32> {
        let q = self.sys_com[sys_com::MAJOR_VERSION];
        if q.is_nil() { None } else { Some(q.pointer()) }
    }

    /// `%SYS-COM-POINTER-WIDTH`: "Either 24 or 25, as fixnum, or DTP-FREE in
    /// old sys" (`cold/qcom.lisp:220`).  `None` when it is DTP-FREE.
    pub fn pointer_width(&self) -> Option<u32> {
        let q = self.sys_com[sys_com::POINTER_WIDTH];
        if q.data_type() == dtp::FREE { None } else { Some(q.pointer()) }
    }

    /// `%SYS-COM-DESIRED-MICROCODE-VERSION`, or `None` in a fresh cold load.
    ///
    /// Read as `Q<23:0>`, not as the pointer field.  `cold/qcom.lisp:213-215`:
    /// "this word may be stored with its data type field starting at bit 24
    /// even though pointer fields are now 25 bits!"  `LOD2` on the vendored
    /// pack is such a word --- its pointer field is `0o100000503`, whose low
    /// 24 bits are 323, and 323 is the microcode `MCR1` carries.
    pub fn desired_microcode_version(&self) -> Option<u32> {
        let q = self.sys_com[sys_com::DESIRED_MICROCODE_VERSION];
        if q.is_nil() { None } else { Some(q.0 & 0xff_ffff) }
    }
}
