// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `diskpack`: making a disk pack and editing its label, off the machine.
//!
//! MIT did this on the machine.  `EDIT-DISK-LABEL` (`io/dledit.lisp:434-466`)
//! is a screen editor for the label of a pack in a drive: initialize it from
//! a pack type, edit the fields, write it back.  What it could take for
//! granted is that the pack exists --- a real one is a stack of platters, and
//! formatting it is the drive's business, not the label editor's --- and that
//! is the one thing a simulator has to do for itself.  So this is MIT's
//! editor with the making of the pack added, and the partition contents
//! loaded and dumped, which is what usim's `diskmaker` is for.
//!
//! The commands are words on a line, as [`crate::prompt`]'s are, rather than
//! MIT's control characters: those drove a display, moving an underscore
//! from field to field, and there is no display here.
//!
//! This is the language and what each command does to a pack; `src/bin/diskpack.rs`
//! reads the lines.  The label itself --- what is in it, how a fresh one is
//! laid out and how it is written --- is in [`crate::band`], one description
//! of the format for the reader and the writer both.
//!
//! Nothing here is written atomically.  A pack is 257 MiB and a label is
//! 1024 bytes of it, so there is no temporary file and no rename: every
//! command writes where the blocks are.  A process killed part way through a
//! `load` therefore leaves half a band in a partition the label still
//! describes as whole, and nothing on the pack afterwards says so --- a
//! rename could not have helped, the bytes landing in the middle of a file
//! that has to stay the size the drive is.  Load it again, or dump the
//! partition and look at what arrived.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::band::{BLOCK_WORDS, Label, Partition, T300};
use crate::mcr;

/// What `diskpack` writes while it waits for a command.
pub const PROMPT: &str = "diskpack: ";

/// Bytes in a block, the unit everything on a pack is measured in.
const BLOCK_BYTES: u64 = BLOCK_WORDS as u64 * 4;

/// What a partition of each field is called when it is given by number.
///
/// `SET-CURRENT-BAND` builds the name as the prefix and the number, the
/// prefix being `LOD` for a band and the processor's for a microload:
/// `SELECT-PROCESSOR (:CADR "MCR") (:LAMBDA "LMC")` (`io/dledit.lisp:37-45`).
/// This is a CADR, so `MCR`.
const MICROLOAD_PREFIX: &str = "MCR";
const BAND_PREFIX: &str = "LOD";

/// What a partition's comment says when the built-in microcode goes into it.
///
/// MIT's own words, and in this very field: `MCR1` on the System 100
/// distribution pack has the comment "UCADR 323", written by whatever put the
/// microcode there.  `UCADR` is the microcode's own name --- MIT's source is
/// `ucadr.lisp` and its output `sys/ubin/ucadr.mcr` --- and 323 is the
/// version this project targets.
const BUILT_IN_COMMENT: &str = "UCADR 323";

/// How big a partition is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Blocks(u32),
    /// `75c`: whole cylinders, which is how MIT's pack types give the size of
    /// everything but a microcode partition.
    Cylinders(u32),
    /// `25%`: that much of the whole pack, rounded down to a whole cylinder.
    Percent(u32),
    /// `rest`: from where this partition starts to the last block of the pack.
    Rest,
    /// `keep`: the size it has, for changing a comment and nothing else.
    Keep,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// A fresh T-300 label, as `LE-INITIALIZE-LABEL` writes one.
    Initialize,
    /// The label and its partition table.
    Show,
    /// The drive's brand name, the pack's name, and the label's comment.
    Drive(String),
    Name(String),
    Comment(String),
    /// Words 6 and 7 as they stand.
    Current,
    /// Word 6, the partition the boot PROM loads microcode from.
    Microload(String),
    /// Word 7, the band the software offers by default.
    Band(String),
    /// Add a partition, at the end of the table where there is room.
    Partition {
        name: String,
        size: Size,
        comment: Option<String>,
    },
    /// Change one that is there --- its size, its comment, or both ---
    /// pushing the entries after it up out of its way.  Its own start does
    /// not move, and it never shrinks.  No blocks are moved by any of it.
    Modify {
        name: String,
        size: Size,
        comment: Option<String>,
    },
    Delete(String),
    /// A file into a partition: the blocks as they stand, or a microcode
    /// file the way the machine reads one.  See `Pack::load`.
    Load {
        partition: String,
        file: Option<PathBuf>,
    },
    /// A partition of another pack into one of ours, block for block.
    LoadFrom {
        partition: String,
        pack: PathBuf,
        from: String,
    },
    /// A partition's blocks out to a file, as they stand.
    Dump {
        partition: String,
        file: PathBuf,
    },
    Quit,
    Help,
}

/// What `help` says.
pub const HELP: &str = "\
initialize, i           a fresh label for a Trident T-300, the drive a
                        CADR's pack goes in: its geometry, its drive and its
                        partitions, laid out as MIT lays them out
show, s                 the label and its partition table
drive <text>            the drive's brand name
name <text>             the pack's name
comment <text>          the label's comment
current, c              the current microload and the current band
current <n>             the current band is LODn, a number being a band as it
                        is in MIT's own SET-CURRENT-BAND
current <partition>     that partition, the name saying which of the two it
                        is: MCRn is a microload and LODn a band.  A microload
                        is written out, current MCR1
partition, p <name> <size> [<comment>]
                        add a partition, on the end where there is room, the
                        name in upper case as every name on a pack is.  A
                        size is 1234 blocks, 75c cylinders, 25% of the pack
                        rounded down to a cylinder, or rest for what is left
modify, m <partition> <size> [<comment>]
                        change one that is there: grow it, or with keep
                        leave the size alone and set the comment.  Its own
                        start does not move, so what is in it stays where it
                        is; the entries after it are pushed up out of its
                        way, and what is in those partitions stays where it
                        is too, which is the thing to watch.  Nothing is
                        ever moved down and nothing shrinks: delete is how a
                        partition gives blocks back
delete, d <partition>   take it out; its blocks are free where they were
load, l <partition> [<file>]
                        a file into a partition.  An MCR one holds microcode
                        and takes a microcode file, which goes in the way the
                        machine reads one --- every word's halves swapped and
                        the rest of the partition zeroed --- and anything
                        else takes the file as it stands.  Without a file,
                        the built-in microcode 323, System 100's own.  The
                        partition's comment becomes the file's name, or
                        UCADR 323 for the built-in, cut to the sixteen
                        characters a descriptor holds
load-from <partition> <pack> <partition>
                        a partition of another pack into ours, block for
                        block.  Both sides are packs, so the words are
                        already the way the disk holds them and nothing is
                        swapped; ours must be at least as big as theirs
dump <partition> <file> a partition out to a file
quit, q                 leave
help, h, ?              this

Every command takes effect when you type it: the pack is a file with nothing
running on it, so there is no state to hold and no write to remember.  A
command whose table could not be written --- a partition off the end of the
pack, or two on the same blocks --- is refused whole and changes nothing.  So
a partition that is in the way of another has to be deleted and added again
smaller, which is two commands that say what they are doing.
";

/// A line as a command: `Ok(None)` for a blank line, and what was wrong with
/// one that is no command.
pub fn parse(line: &str) -> Result<Option<Command>, String> {
    let line = line.trim();
    let Some(word) = line.split_whitespace().next() else { return Ok(None) };
    let arg = line[word.len()..].trim();
    let bare = |cmd: Command| {
        if arg.is_empty() { Ok(Some(cmd)) } else { Err(format!("{word} takes nothing")) }
    };
    // The rest of the line, spaces and all: a comment is not several
    // arguments, and neither is a path.
    let rest = |what: &str| {
        if arg.is_empty() { Err(format!("{word} wants {what}")) } else { Ok(arg.to_string()) }
    };
    match word {
        "show" | "s" => bare(Command::Show),
        "initialize" | "i" => bare(Command::Initialize),
        "quit" | "q" => bare(Command::Quit),
        "help" | "h" | "?" => bare(Command::Help),
        "current" | "c" => parse_current(arg),
        "drive" => Ok(Some(Command::Drive(rest("the drive's brand name")?))),
        "name" => Ok(Some(Command::Name(rest("the pack's name")?))),
        "comment" => Ok(Some(Command::Comment(rest("a comment")?))),
        "delete" | "d" => Ok(Some(Command::Delete(partition_named(&rest("a partition")?)))),
        "partition" | "p" => parse_partition(arg, false),
        "modify" | "m" => parse_partition(arg, true),
        "load" | "l" => {
            let mut args = arg.splitn(2, char::is_whitespace);
            let Some(partition) = args.next().filter(|p| !p.is_empty()) else {
                return Err("load wants a partition, and a file or nothing".to_string());
            };
            Ok(Some(Command::Load {
                partition: partition_named(partition),
                file: args.next().map(|f| PathBuf::from(f.trim())),
            }))
        }
        // No short form: it is two packs and four things to get right.
        "load-from" => {
            let wants =
                || "load-from wants our partition, a pack, and a partition of it".to_string();
            let (ours, rest) = arg.split_once(char::is_whitespace).ok_or_else(wants)?;
            let (pack, theirs) = rest.trim().rsplit_once(char::is_whitespace).ok_or_else(wants)?;
            // Their partition is the last word and ours the first, so what is
            // between them is the pack, spaces in its name and all.
            Ok(Some(Command::LoadFrom {
                partition: partition_named(ours),
                pack: PathBuf::from(pack.trim()),
                from: partition_named(theirs),
            }))
        }
        // No short form: taking a partition out is a rare thing to want.
        "dump" => {
            let mut args = arg.splitn(2, char::is_whitespace);
            let (Some(partition), Some(file)) = (args.next(), args.next()) else {
                return Err("dump wants a partition and a file".to_string());
            };
            Ok(Some(Command::Dump {
                partition: partition_named(partition),
                file: PathBuf::from(file.trim()),
            }))
        }
        other => Err(format!("{other} is no command; help lists them")),
    }
}

/// A partition by number or by name: `1` is `LOD1`, the band being what a
/// number means wherever a partition is asked for, as it does in `current`
/// and in `SET-CURRENT-BAND`; anything else is the name, upper case as every
/// name on a pack is.
fn partition_named(which: &str) -> String {
    if which.bytes().all(|c| c.is_ascii_digit()) {
        format!("{BAND_PREFIX}{which}")
    } else {
        which.to_ascii_uppercase()
    }
}

/// `current <partition>`: which partition is the pack's current microload,
/// word 6, and which is its current band, word 7.
///
/// A number is the band of that number, which is how `SET-CURRENT-BAND` takes
/// one --- `(FORMAT NIL "~A~D" prefix band)` with `LOD` for the prefix when
/// `MICRO-P` is nil (`io/dledit.lisp:37-45`).  A name is that partition, and
/// says which of the two words it goes in by what it is called: `MCR` is a
/// microload and `LOD` a band, those being the two kinds of partition a CADR
/// pack has.  So a microload is always written out, `current MCR1`, and never
/// reached by a number.
fn parse_current(arg: &str) -> Result<Option<Command>, String> {
    let mut args = arg.split_whitespace();
    let Some(which) = args.next() else {
        return Ok(Some(Command::Current));
    };
    if args.next().is_some() {
        return Err("current takes one partition, by name or by number".to_string());
    }
    if which.bytes().all(|c| c.is_ascii_digit()) {
        return Ok(Some(Command::Band(format!("{BAND_PREFIX}{which}"))));
    }
    let name = which.to_ascii_uppercase();
    if name.starts_with(MICROLOAD_PREFIX) {
        Ok(Some(Command::Microload(name)))
    } else if name.starts_with(BAND_PREFIX) {
        Ok(Some(Command::Band(name)))
    } else {
        Err(format!(
            "{name} is not an {MICROLOAD_PREFIX} or a {BAND_PREFIX} partition, \
             and those are the two the label names"
        ))
    }
}

fn parse_partition(arg: &str, modify: bool) -> Result<Option<Command>, String> {
    let word = if modify { "modify" } else { "partition" };
    let mut args = arg.splitn(3, char::is_whitespace);
    let (Some(name), Some(size)) = (args.next(), args.next()) else {
        return Err(format!("{word} wants a partition and a size"));
    };
    let (name, comment) = (partition_named(name), args.next().map(|c| c.trim().to_string()));
    let size = parse_size(size)?;
    if !modify && size == Size::Keep {
        return Err("keep is the size it has, and a new partition has none".to_string());
    }
    Ok(Some(if modify {
        Command::Modify { name, size, comment }
    } else {
        Command::Partition { name, size, comment }
    }))
}

/// `1234` blocks, `75c` cylinders, `25%` of the pack, `rest`, `keep`.
pub fn parse_size(s: &str) -> Result<Size, String> {
    let number = |n: &str| {
        n.parse::<u32>().map_err(|_| format!("{s} is no size: help says what one looks like"))
    };
    match s {
        "" => Err("a partition wants a size: help says what one looks like".to_string()),
        "rest" => Ok(Size::Rest),
        "keep" => Ok(Size::Keep),
        _ => match s.split_at_checked(s.len() - 1) {
            Some((n, "c")) => Ok(Size::Cylinders(number(n)?)),
            Some((n, "%")) => match number(n)? {
                p if p > 0 && p <= 100 => Ok(Size::Percent(p)),
                p => Err(format!("{p}% of a pack is not a size")),
            },
            _ => Ok(Size::Blocks(number(s)?)),
        },
    }
}

/// A pack being made or edited: its file, and its label.
///
/// Every command that changes the label writes it, and one whose result could
/// not be written is refused whole, so what is in hand here is always what is
/// in block 0.  MIT's editor holds the label instead and writes it on `^W`,
/// because the label it is editing belongs to a drive on a running machine
/// and half a label there is a machine that cannot find its bands.  Nothing
/// is running on this one: it is a file, and the reason for the two states is
/// not here.
pub struct Pack {
    path: PathBuf,
    label: Option<Label>,
}

impl Pack {
    /// Opens a pack, or starts one that is not there yet.  The second value
    /// is what to say about it: this is the line the tool prints at the top.
    pub fn open(path: &Path) -> (Pack, String) {
        let mut pack = Pack { path: path.to_path_buf(), label: None };
        let said = if !path.exists() {
            format!("{} is not there: initialize makes one", path.display())
        } else {
            match Label::open(path) {
                Ok(label) => {
                    let said = format!("{}: {}", path.display(), one_line(&label));
                    pack.label = Some(label);
                    said
                }
                // The error names the pack where it comes off the file and
                // does not where it is about what is in block 0, so the line
                // after it always does.
                // Not "initialize makes one": it will not, over a file that
                // is there, and saying so here would be an invitation to try.
                Err(e) => {
                    format!(
                        "{e}\n{} is not a pack, and nothing here writes over it",
                        path.display()
                    )
                }
            }
        };
        (pack, said)
    }

    /// Whether there is a label to work on: a pack that is not there yet, or
    /// a file that is no pack, has none until `initialize`.
    pub fn has_label(&self) -> bool {
        self.label.is_some()
    }

    /// The label, or the complaint of someone who has not made one yet.
    fn label(&self) -> Result<&Label, String> {
        self.label.as_ref().ok_or_else(|| {
            "there is no label yet: initialize makes one where there is no file".to_string()
        })
    }

    /// A copy of the label to change, which [`Pack::commit`] puts back.
    fn taken(&self) -> Result<Label, String> {
        self.label().cloned()
    }

    /// Puts a changed label on the pack, writing it to block 0.
    ///
    /// Refused whole if the table cannot be written --- a partition off the
    /// end of the pack, or two on the same blocks --- and then the label is
    /// left exactly as it was, so that a command either happened or did not.
    /// It is why a partition has to be shrunk before the one before it is
    /// grown, rather than the table being allowed to overrun in the meantime.
    fn commit(&mut self, label: Label) -> Result<(), String> {
        if let Some(bad) = overrun(&label) {
            return Err(bad);
        }
        label.write()?;
        self.label = Some(label);
        Ok(())
    }

    /// A partition by name, and where it is in the table.
    fn partition(&self, name: &str) -> Result<(usize, &Partition), String> {
        let label = self.label.as_ref().ok_or("there is no label yet")?;
        label
            .partitions
            .iter()
            .enumerate()
            .find(|(_, p)| p.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| no_such(label, name))
    }

    /// Does what the command says, and answers with what to print.
    pub fn run(&mut self, c: Command) -> Result<String, String> {
        match c {
            Command::Help => Ok(HELP.to_string()),
            Command::Quit => Ok(String::new()),
            Command::Show => Ok(show(self.label()?)),
            Command::Current => Ok(current(self.label()?)),
            Command::Initialize => {
                // The one command that makes a 257 MiB file, given a path
                // that came out of someone's shell.  A path that is a typo is
                // usually a file that is there, and the pack it would make of
                // it is not something to do because a name was mistyped; a
                // pack that is there already is worse, since what would go is
                // the only record of where its bands are, and there is no
                // write being withheld here and no undo.  Deleting the file
                // is how someone says they mean it.
                if let Some(label) = &self.label {
                    return Err(format!(
                        "{} is a pack already: {} --- initialize makes one where there is \
                         no file, so delete it first if you mean to",
                        self.path.display(),
                        one_line(label)
                    ));
                }
                if self.path.exists() {
                    return Err(format!(
                        "{} is not a pack, and initialize would write {} MiB over it: \
                         it makes one where there is no file",
                        self.path.display(),
                        u64::from(T300.geometry.blocks()) * BLOCK_BYTES / (1024 * 1024)
                    ));
                }
                let label = Label::initialize(&self.path, &T300);
                let said = format!("{}{}", show(&label), self.made(&label));
                self.commit(label)?;
                Ok(said)
            }
            Command::Drive(text) => self.set(text, |l, t| l.drive = t),
            Command::Name(text) => self.set(text, |l, t| l.pack_name = t),
            Command::Comment(text) => self.set(text, |l, t| l.comment = t),
            Command::Microload(name) => {
                let name = self.partition(&name)?.1.name.clone();
                self.set(name, |l, t| l.microload_partition = t)
            }
            Command::Band(name) => {
                let name = self.partition(&name)?.1.name.clone();
                self.set(name, |l, t| l.current_band = t)
            }
            Command::Partition { name, size, comment } => {
                self.set_partition(&name, size, comment, false)
            }
            Command::Modify { name, size, comment } => {
                self.set_partition(&name, size, comment, true)
            }
            Command::Delete(name) => self.delete(&name),
            Command::Load { partition, file } => self.load(&partition, file),
            Command::LoadFrom { partition, pack, from } => self.load_from(&partition, &pack, &from),
            Command::Dump { partition, file } => self.dump(&partition, &file),
        }
    }

    /// How big the pack this label describes is, said where the file is made.
    fn made(&self, label: &Label) -> String {
        let blocks = label.blocks();
        format!(
            "{}: {blocks} blocks, {} MiB\n",
            self.path.display(),
            u64::from(blocks) * BLOCK_BYTES / (1024 * 1024)
        )
    }

    /// One of the label's text fields.
    fn set(&mut self, text: String, put: fn(&mut Label, String)) -> Result<String, String> {
        let mut label = self.taken()?;
        put(&mut label, text);
        self.commit(label)?;
        Ok(String::new())
    }

    /// Adds a partition, or changes one that is there, pushing the entries
    /// after it up out of its way.
    ///
    /// Two commands and not one: adding a partition to a table with room in
    /// it does nothing to what is already on the pack, and changing one moves
    /// the table under bands that do not move with it.
    ///
    /// And growing is all a change does to a size.  A partition that shrank would leave the
    /// band in it running off its end, with nothing to say so afterwards ---
    /// the label would describe a partition and the pack would hold something
    /// longer.  `delete` is how blocks are given back, which says what it is
    /// doing.
    fn set_partition(
        &mut self,
        name: &str,
        size: Size,
        comment: Option<String>,
        modify: bool,
    ) -> Result<String, String> {
        if name.len() > 4 || !name.is_ascii() {
            return Err(format!("{name} is no partition name: four ASCII characters or fewer"));
        }
        // Upper case already, from `partition_named`: the boot PROM's
        // `SEARCH-LABEL` compares the four characters as a word, so a `lod1`
        // is not the `LOD1` the machine goes looking for.  Done again here
        // because this is also reached from the library, where a caller may
        // not have come through the parser.
        let name = &name.to_ascii_uppercase();
        let mut label = self.taken()?;
        let at = label.partitions.iter().position(|p| p.name.eq_ignore_ascii_case(name));
        match (at, modify) {
            (Some(_), false) => {
                return Err(format!("{name} is on the pack already: modify changes one"));
            }
            (None, true) => return Err(no_such(&label, name)),
            _ => {}
        }
        let label = &mut label;
        // A size given in cylinders puts a new partition at a cylinder
        // boundary, as MIT's own initialize does with the sizes its pack
        // types give in cylinders; one given in blocks is laid where it
        // falls.  A partition that is already there keeps the start it has.
        let start = match at {
            Some(at) => label.partitions[at].start,
            None => {
                let end = label.partitions.last().map_or(label.first_block(), |p| p.end());
                match size {
                    Size::Cylinders(_) | Size::Percent(_) => label.at_cylinder(end),
                    _ => end,
                }
            }
        };
        let too_big = || format!("that is more than the {} blocks on the pack", label.blocks());
        let blocks = match size {
            Size::Blocks(n) => n,
            Size::Cylinders(n) => n.checked_mul(label.blocks_per_cylinder).ok_or_else(too_big)?,
            Size::Percent(p) => {
                let bpc = u64::from(label.blocks_per_cylinder.max(1));
                let want = u64::from(label.blocks()) * u64::from(p) / 100;
                ((want / bpc).max(1) * bpc) as u32
            }
            Size::Rest => label.blocks().saturating_sub(start),
            Size::Keep => label.partitions[at.expect("keep needs a partition")].blocks,
        };
        if blocks > label.blocks() {
            return Err(too_big());
        }
        if blocks == 0 {
            return Err(format!("{name} would have no blocks in it"));
        }
        if let Some(at) = at
            && blocks < label.partitions[at].blocks
        {
            return Err(format!(
                "{name} is {} blocks and this would make it {blocks}: \
                 a partition only grows here, and delete is how one gives blocks back",
                label.partitions[at].blocks
            ));
        }
        let was: Vec<(String, u32)> =
            label.partitions.iter().map(|p| (p.name.clone(), p.start)).collect();
        // A partition that is already there keeps its start: what moves when
        // it is resized is what comes after it.  That is how the System 100
        // pack is laid out --- its PAGE is 290 blocks longer than MIT's table
        // and still begins where MIT's does, with everything after it moved
        // up --- and it is what someone resizing a partition means.
        let (at, from) = match at {
            Some(at) => {
                label.partitions[at].blocks = blocks;
                (at, at + 1)
            }
            None => {
                label.partitions.push(Partition {
                    name: name.to_string(),
                    start,
                    blocks,
                    comment: String::new(),
                });
                // Laid out from the one after it: the start decided above is
                // this partition's, and nothing before it moves.
                (label.partitions.len() - 1, label.partitions.len())
            }
        };
        if let Some(comment) = comment {
            label.partitions[at].comment = comment;
        }
        label.lay_out(from);
        let label = label.clone();
        let said = format!("{}{}", line(&label, &label.partitions[at]), moved(&was, &label));
        self.commit(label)?;
        Ok(said)
    }

    fn delete(&mut self, name: &str) -> Result<String, String> {
        let at = self.partition(name)?.0;
        let mut label = self.taken()?;
        let was: Vec<(String, u32)> =
            label.partitions.iter().map(|p| (p.name.clone(), p.start)).collect();
        let gone = label.partitions.remove(at);
        label.lay_out(at);
        let said = format!(
            "{} is gone: {} blocks at {}\n{}",
            gone.name,
            gone.blocks,
            gone.start,
            moved(&was, &label)
        );
        self.commit(label)?;
        Ok(said)
    }

    /// The pack's file, for a command that reads or writes blocks.
    ///
    /// There has to be a file, and `initialize` is what makes one; the label
    /// in hand is the label in block 0, every command here having written
    /// itself as it ran.  A `dump` asks for the file read-only, so that
    /// reading a partition out of a pack cannot write to the pack --- the
    /// vendored one is fetched material and stays as fetched.
    fn image(&self, writing: bool) -> Result<File, String> {
        let label = self.label.as_ref().ok_or("there is no label yet")?;
        if !self.path.exists() {
            return Err(format!(
                "{} is not there: initialize makes it, and something has removed it",
                self.path.display()
            ));
        }
        let want = label.geometry().blocks() as u64 * BLOCK_BYTES;
        let f = OpenOptions::new()
            .read(true)
            .write(writing)
            .open(&self.path)
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        let have = f.metadata().map_err(|e| format!("{}: {e}", self.path.display()))?.len();
        if have != want {
            return Err(format!(
                "{} is {have} bytes and its label describes {want}: write it",
                self.path.display()
            ));
        }
        Ok(f)
    }

    /// A partition to put blocks in or take them out of: where it starts,
    /// how long it is, and its name as the label spells it.
    ///
    /// The label is data off a pack, and a partition it describes as running
    /// past the end of the pack is one there is nowhere to read or write ---
    /// and its size is what an allocation here would be sized by.  `overrun`
    /// keeps a table this tool writes from saying such a thing; this is for
    /// the tables it did not write.
    fn extent(&self, name: &str) -> Result<(u32, u32, String), String> {
        let blocks_on_the_pack = self.label()?.blocks();
        let (_, p) = self.partition(name)?;
        if p.end() > blocks_on_the_pack || p.blocks == 0 {
            return Err(format!(
                "{} runs past the end of the pack: {} blocks at {}, and the pack has {}",
                p.name, p.blocks, p.start, blocks_on_the_pack
            ));
        }
        Ok((p.start, p.blocks, p.name.clone()))
    }

    /// A file into a partition.
    ///
    /// A microcode file is not a copy.  It holds each 32-bit word as two
    /// 16-bit pieces, the high one first --- "Note non-standard order of
    /// 16-bit bytes", `sys/sys/qwmcr.lisp:23`, MIT's own writer of these files ---
    /// and the disk holds a word as the controller reads it, low half first,
    /// so every word's halves are swapped on the way in.  MIT says the same
    /// from the other side in `LOAD-LMC-FILE` (`io/disk.lisp:1429`): "the
    /// halfwords are IN order in a LMC file (as opposed to a MCR file)".  The
    /// System 100 pack settles it: its `MCR1` is `sys/ubin/ucadr.mcr` swapped
    /// that way, byte for byte over all 114 blocks of it, and
    /// `tests/diskpack.rs` holds this to that.  The rest of the partition is
    /// zeroed with it, so that a microload partition holds one microcode and
    /// not the tail of the last one.
    ///
    /// Anything else goes in as it stands, and what is past it is left alone:
    /// a band is written over a band, and zeroing the rest of a partition
    /// 24,225 blocks long would write 24 MB to say nothing.
    ///
    /// Which of the two it is, the partition says: an `MCR` one holds
    /// microcode and takes a microcode file, and everything else takes what
    /// it is given.  The file is read either way --- a microcode file that
    /// does not parse as one is refused rather than swapped into nonsense ---
    /// but it does not get to decide where it is going.  usim's `diskmaker`
    /// writes every file through unchanged, which is why what it restores is
    /// always a partition dump and never a microcode file.
    fn load(&mut self, name: &str, file: Option<PathBuf>) -> Result<String, String> {
        let (start, blocks, name) = self.extent(name)?;
        let room = blocks as u64 * BLOCK_BYTES;
        let mut f = self.image(true)?;
        // No file: the microcode the machine this simulates ran, which is
        // built in and needs nothing fetched.
        let (bytes, from, comment) = match file {
            Some(f) => {
                let said = f.display().to_string();
                let bytes = std::fs::read(&f).map_err(|e| format!("{said}: {e}"))?;
                // The file's own name, which is what usim's `diskmaker` puts
                // in this field too, and all a reader has to go on later:
                // the file is not on the pack, only what came out of it.
                let name = f.file_name().unwrap_or(f.as_os_str()).to_string_lossy().into_owned();
                (bytes, said, name)
            }
            None => (
                mcr::UCADR_323.to_vec(),
                "microcode 323, built in".to_string(),
                BUILT_IN_COMMENT.to_string(),
            ),
        };
        if bytes.len() as u64 > room {
            return Err(format!("{from} is {} bytes and {name} holds {room}", bytes.len()));
        }
        let blocks_of = |n: usize| n.div_ceil(BLOCK_BYTES as usize);
        let microcode = if name.starts_with(MICROLOAD_PREFIX) {
            let m = mcr::parse(&bytes).map_err(|e| {
                format!("{name} holds microcode and {from} is not a microcode file: {e}")
            })?;
            if bytes.len() % 4 != 0 {
                return Err(format!("{from} is not a whole number of words"));
            }
            Some(m)
        } else {
            None
        };
        f.seek(SeekFrom::Start(start as u64 * BLOCK_BYTES))
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        let mut said = match microcode {
            Some(m) => {
                let mut swapped = vec![0u8; room as usize];
                for (i, w) in bytes.as_chunks::<4>().0.iter().enumerate() {
                    swapped[i * 4..i * 4 + 4].copy_from_slice(&[w[2], w[3], w[0], w[1]]);
                }
                f.write_all(&swapped).map_err(|e| format!("{}: {e}", self.path.display()))?;
                // The partition's comment is left alone: a microcode file
                // does not say which version it is --- `WRITE-MCR-FILE`
                // writes one only when it is given a base version, and the
                // release's file has none --- and MIT's own "UCADR 323" is
                // something the person making the pack knows and the file
                // does not.
                format!(
                    "{name}: {} control store words, {} blocks of {blocks}, the rest zeroed\n",
                    m.imem.len(),
                    blocks_of(bytes.len())
                )
            }
            None => {
                f.write_all(&bytes).map_err(|e| format!("{}: {e}", self.path.display()))?;
                format!(
                    "{name}: {} blocks of {blocks} written, the rest left as it was\n",
                    blocks_of(bytes.len())
                )
            }
        };
        // What went in, said in the label: a partition's comment is the only
        // place a pack says what is in a partition.  Written after the
        // blocks, so that it never describes a write that did not happen.
        said.push_str(&self.describe(&name, &comment)?);
        Ok(said)
    }

    /// Puts what was loaded in the partition's comment, cut to the field.
    ///
    /// A descriptor is seven words on every pack MIT made, of which the last
    /// four are the comment, so it holds sixteen characters and a longer name
    /// arrives cut --- and characters a word cannot hold are dropped, since a
    /// file with an accent in its name is still a file that loaded.
    fn describe(&mut self, name: &str, comment: &str) -> Result<String, String> {
        let mut label = self.taken()?;
        let width = 4 * label.words_per_descriptor.saturating_sub(3) as usize;
        let comment: String =
            comment.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(width).collect();
        let Some(p) = label.partitions.iter_mut().find(|p| p.name == name) else {
            return Ok(String::new());
        };
        p.comment = comment.clone();
        self.commit(label)?;
        Ok(format!("{name} is now \"{comment}\"\n"))
    }

    /// A partition of another pack into one of ours, block for block.
    ///
    /// The way a band is moved from the pack it came on to a pack of one's
    /// own without a file in between.  Both sides are packs, so the words are
    /// already the way the controller reads them and nothing is swapped or
    /// zeroed: it is the blocks, as they are.
    ///
    /// Ours has to be at least as big as theirs, so that all of what is being
    /// copied lands: a partition that arrived cut short would be a band with
    /// no end, and nothing in the label afterwards would say so.
    fn load_from(&mut self, name: &str, pack: &Path, from: &str) -> Result<String, String> {
        let (start, blocks, name) = self.extent(name)?;

        let label = Label::open(pack)?;
        let theirs = label
            .partitions
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(from))
            .ok_or_else(|| format!("{}: {}", pack.display(), no_such(&label, from)))?;
        if theirs.blocks > blocks {
            return Err(format!(
                "{} {} is {} blocks and {name} holds {blocks}",
                pack.display(),
                theirs.name,
                theirs.blocks
            ));
        }
        let have = std::fs::metadata(pack).map_err(|e| format!("{}: {e}", pack.display()))?.len();
        let need = u64::from(theirs.end()) * BLOCK_BYTES;
        if have < need {
            return Err(format!(
                "{} is not a whole pack: {have} bytes, and its {} ends at {need}",
                pack.display(),
                theirs.name
            ));
        }
        let mut theirs_file = File::open(pack).map_err(|e| format!("{}: {e}", pack.display()))?;
        theirs_file
            .seek(SeekFrom::Start(theirs.start as u64 * BLOCK_BYTES))
            .map_err(|e| format!("{}: {e}", pack.display()))?;
        let mut ours = self.image(true)?;
        ours.seek(SeekFrom::Start(start as u64 * BLOCK_BYTES))
            .map_err(|e| format!("{}: {e}", self.path.display()))?;

        let mut left = theirs.blocks as u64 * BLOCK_BYTES;
        let mut buf = vec![0u8; BLOCK_BYTES as usize * 256];
        while left > 0 {
            let n = buf.len().min(left as usize);
            theirs_file.read_exact(&mut buf[..n]).map_err(|e| short(pack, e))?;
            ours.write_all(&buf[..n]).map_err(|e| format!("{}: {e}", self.path.display()))?;
            left -= n as u64;
        }
        Ok(format!(
            "{name}: {} blocks of {blocks} from {} {}\n",
            theirs.blocks,
            pack.display(),
            theirs.name
        ))
    }

    /// A partition out to a file, all of it.
    fn dump(&mut self, name: &str, file: &Path) -> Result<String, String> {
        let (start, blocks, name) = self.extent(name)?;
        let mut image = self.image(false)?;
        image
            .seek(SeekFrom::Start(start as u64 * BLOCK_BYTES))
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        if file.exists() {
            return Err(format!(
                "{} is there already: a dump does not write over one",
                file.display()
            ));
        }
        let mut out = File::create(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut left = blocks as u64 * BLOCK_BYTES;
        let mut buf = vec![0u8; BLOCK_BYTES as usize * 256];
        while left > 0 {
            let n = buf.len().min(left as usize);
            // A dump that stopped part way has written a file that looks like
            // a partition and is not one, so what it wrote goes with it.
            let undo = |e: String| {
                let _ = std::fs::remove_file(file);
                e
            };
            image.read_exact(&mut buf[..n]).map_err(|e| undo(short(&self.path, e)))?;
            out.write_all(&buf[..n]).map_err(|e| undo(format!("{}: {e}", file.display())))?;
            left -= n as u64;
        }
        Ok(format!("{name}: {blocks} blocks to {}\n", file.display()))
    }
}

/// A read that ran off the end of a file, said as what it is: `read_exact`
/// calls it "failed to fill whole buffer", which is about the buffer.
fn short(file: &Path, e: std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::UnexpectedEof => {
            format!("{} ends before the blocks it was read for", file.display())
        }
        _ => format!("{}: {e}", file.display()),
    }
}

/// What the pack has, for someone who named a partition it does not.
fn no_such(label: &Label, name: &str) -> String {
    let names: Vec<&str> = label.partitions.iter().map(|p| p.name.as_str()).collect();
    format!("no partition named {name}: the pack has {}", names.join(" "))
}

/// A label in a line: what the tool says about a pack it has just opened.
fn one_line(label: &Label) -> String {
    format!(
        "{}: {}, {} partitions, band {}",
        label.pack_name,
        label.drive,
        label.partitions.len(),
        label.current_band
    )
}

/// One partition, as `PRINT-DISK-LABEL-FROM-RQB` writes it
/// (`io/dledit.lisp:253-268`): a `*` on the current microload and the current
/// band, the name, where it starts, how long it is, and its comment.
fn line(label: &Label, p: &Partition) -> String {
    let current = p.name == label.microload_partition || p.name == label.current_band;
    format!(
        "{} {:<4} at block {:>8}, {:>8} blocks long, \"{}\"\n",
        if current { "*" } else { " " },
        p.name,
        p.start,
        p.blocks,
        p.comment
    )
}

/// The current microload and the current band, the line MIT's own display
/// gives them (`io/dledit.lisp:234-240`), where this tool's `current` with
/// nothing after it puts them.
fn current(label: &Label) -> String {
    format!(
        "Current microload = {}, current band = {}\n",
        label.microload_partition, label.current_band
    )
}

/// The whole label, MIT's own display of it.
fn show(label: &Label) -> String {
    let mut s = format!(
        "{}: {}, {}\nLABL version {}, {} cylinders, {} heads, {} blocks/track, \
         {} blocks/cylinder, {} blocks\nCurrent microload = {}, current band = {}\n\
         {} partitions, {}-word descriptors:\n",
        label.pack_name,
        label.drive,
        label.comment,
        label.version,
        label.cylinders,
        label.heads,
        label.blocks_per_track,
        label.blocks_per_cylinder,
        label.blocks(),
        label.microload_partition,
        label.current_band,
        label.partitions.len(),
        label.words_per_descriptor,
    );
    // The gap between each partition and the next, and after the last one to
    // the end of the pack, is MIT's own display (`io/dledit.lisp:269-276`).
    // The gap before the first and the total under them are this tool's: a
    // total that left out the blocks under the first partition would not be
    // the free space, which is the thing it is there to say.
    let mut free = label.partitions.first().map_or_else(
        || label.blocks().saturating_sub(label.first_block()),
        |p| p.start.saturating_sub(label.first_block()),
    );
    if free > 0 && !label.partitions.is_empty() {
        s.push_str(&format!("      {free} blocks free at {}\n", label.first_block()));
    }
    for (i, p) in label.partitions.iter().enumerate() {
        s.push_str(&line(label, p));
        let end = p.end();
        let next = label.partitions.get(i + 1).map_or(label.blocks(), |n| n.start);
        if next > end {
            free += next - end;
            s.push_str(&format!("      {} blocks free at {end}\n", next - end));
        } else if next < end {
            s.push_str(&format!("      {} blocks overlap\n", end - next));
        }
    }
    if label.partitions.is_empty() {
        s.push_str(&format!("      {free} blocks free at {}\n", label.first_block()));
    }
    s.push_str(&format!("{free} blocks free\n"));
    s
}

/// The partitions whose start moved, and the warning that goes with it: what
/// moved is the entry in the table, and nothing on the pack moved at all.
fn moved(was: &[(String, u32)], label: &Label) -> String {
    let mut s = String::new();
    for (name, start) in was {
        if let Some(p) = label.partitions.iter().find(|p| &p.name == name)
            && p.start != *start
        {
            s.push_str(&format!("{name} moves from {start} to {}\n", p.start));
        }
    }
    if !s.is_empty() {
        s.push_str("what is in a partition does not move with it\n");
    }
    s
}

/// What is wrong with a table that cannot be written: a partition off the end
/// of the pack, or two of them on the same blocks.
fn overrun(label: &Label) -> Option<String> {
    let end_of_pack = label.blocks();
    for (i, p) in label.partitions.iter().enumerate() {
        let end = p.end();
        if end > end_of_pack || p.start >= end_of_pack {
            return Some(format!("{} ends at block {end} and the pack has {end_of_pack}", p.name));
        }
        if let Some(next) = label.partitions.get(i + 1)
            && next.start < end
        {
            return Some(format!("{} and {} are on the same blocks", p.name, next.name));
        }
    }
    None
}
