// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The prompt: muir's own line on stdin while a machine runs on its own.
//! A line is a command to muir itself, not to the machine --- the
//! machine's keyboard is the terminal's --- and is acted on between two
//! microcycles.  This is the commands, their parsing and what they print;
//! `muir` reads the lines, holds the machine and finds the memories.
//!
//! `muir: ` is written while the machine is held, which is when muir is
//! waiting to be told what to do next, and only to a terminal: a pipe or a
//! file gets muir's answers alone.  A line typed while the machine runs is
//! acted on just the same; there is only no prompt in front of it, because
//! muir is not waiting.

use std::fmt::Write;
use std::path::PathBuf;

/// What muir writes while the machine is held, waiting to be told what to
/// do next.
pub const PROMPT: &str = "muir: ";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// The boot button, which is what starts a CADR: it presets `RUN`,
    /// and the machine runs from there.
    Boot,
    /// No microcycle runs until `Continue` or `Step`.
    Hold,
    /// Run on.
    Continue,
    /// So many microcycles, then hold.
    Step(u64),
    /// Where the machine is.
    Pc,
    /// Every register, in hex and as characters.
    Registers,
    /// So many words of one of the machine's memories from an address, or
    /// all of it: `words` is `None` for the rest of the memory.
    Dump {
        memory: Memory,
        from: usize,
        words: Option<usize>,
    },
    /// The screen as it stands to the file, as a PNG, or to
    /// `muir-yyyymmdd-hhmmss.png` in the current directory.
    Screenshot(Option<PathBuf>),
    /// Record the display from here on to the file, as a GIF, or to
    /// `muir-yyyymmdd-hhmmss.gif` in the current directory; it is written
    /// when the run stops, as `--tv-capture`'s is, or when `EndCapture`
    /// closes it.
    StartCapture(Option<PathBuf>),
    /// Write the recording that is going and stop sampling, with the run
    /// left running.
    EndCapture,
    /// The machine's whole state to the file, or to
    /// `muir-yyyymmdd-hhmmss.chk` in the current directory.
    Checkpoint(Option<PathBuf>),
    /// What this run is: the engine, the memory, the boards, the pack,
    /// the terminal, as said at the start.
    Info,
    /// End the run, as a stop does.
    Quit,
    Help,
}

/// One of the machine's own memories, by the name the command gives it,
/// which is the name the hardware has: the A and M memories, the dispatch
/// memory, the PDL buffer and the SPC stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Memory {
    Amem,
    Mmem,
    Dmem,
    Pdl,
    Spc,
}

impl Memory {
    /// The command's word for it.
    pub fn name(self) -> &'static str {
        match self {
            Memory::Amem => "amem",
            Memory::Mmem => "mmem",
            Memory::Dmem => "dmem",
            Memory::Pdl => "pdl",
            Memory::Spc => "spc",
        }
    }
}

/// What `help` says.
pub const HELP: &str = "\
boot                    the boot button, which is what starts a machine: it
                        presets RUN, and the machine runs from the PROM at 0
hold                    no microcycle runs until continue or step
continue, c             run on
step [n]                n microcycles, one without n, then hold
pc                      where the machine is: the PC, its microcycles and
                        its clock
reg                     every register, in hex and as characters
amem [from [n]]         the A memory, all of it or n words from there
mmem [from [n]]         the M memory
dmem [from [n]]         the dispatch memory
pdl [from [n]]          the PDL buffer
spc [from [n]]          the SPC stack
screenshot, ss [file]   the screen as it stands, as a PNG, to the file or to
                        muir-yyyymmdd-hhmmss.png in the current directory
startcapture, sc [file] record the display from here on, as a GIF, to the
                        file or to muir-yyyymmdd-hhmmss.gif; it is written
                        when the run stops, or when endcapture closes it
endcapture, ec          write the recording that is going and stop
                        recording, with the machine left running
checkpoint [file]       the machine's whole state to the file, or to
                        muir-yyyymmdd-hhmmss.chk in the current directory
info, i                 what this run is, as said at the start
quit, q                 end the run, as a stop does
help, h, ?              this

An address and a count are octal, as MIT writes them.  A dump writes four
words to a line: the address, the words in hex, and the four characters
each word holds.  A line the same as the one above it is a *.
";

/// A line as a command: `Ok(None)` for a blank line, and what was wrong
/// with one that is no command.
pub fn parse(line: &str) -> Result<Option<Command>, String> {
    let line = line.trim();
    let Some(word) = line.split_whitespace().next() else { return Ok(None) };
    let arg = line[word.len()..].trim();
    let bare = |cmd: Command| {
        if arg.is_empty() { Ok(Some(cmd)) } else { Err(format!("{word} takes nothing")) }
    };
    match word {
        "boot" => bare(Command::Boot),
        "hold" => bare(Command::Hold),
        "continue" | "c" => bare(Command::Continue),
        "pc" => bare(Command::Pc),
        "reg" => bare(Command::Registers),
        "info" | "i" => bare(Command::Info),
        "quit" | "q" => bare(Command::Quit),
        "help" | "h" | "?" => bare(Command::Help),
        "step" if arg.is_empty() => Ok(Some(Command::Step(1))),
        "step" => match arg.parse::<u64>() {
            Ok(n) if n > 0 => Ok(Some(Command::Step(n))),
            _ => Err(format!("step wants a count of microcycles, not {arg}")),
        },
        "amem" => parse_dump(Memory::Amem, arg),
        "mmem" => parse_dump(Memory::Mmem, arg),
        "dmem" => parse_dump(Memory::Dmem, arg),
        "pdl" => parse_dump(Memory::Pdl, arg),
        "spc" => parse_dump(Memory::Spc, arg),
        "screenshot" | "ss" => Ok(Some(Command::Screenshot(file(arg)))),
        "startcapture" | "sc" => Ok(Some(Command::StartCapture(file(arg)))),
        "endcapture" | "ec" => bare(Command::EndCapture),
        "checkpoint" => Ok(Some(Command::Checkpoint(file(arg)))),
        other => Err(format!("{other} is no command; help lists them")),
    }
}

/// The file a command was given, if it was given one.  What is left of
/// the line is the whole name, spaces and all: a path is not several
/// arguments.
fn file(arg: &str) -> Option<PathBuf> {
    if arg.is_empty() { None } else { Some(PathBuf::from(arg)) }
}

/// `amem`, `mmem`, `dmem`, `pdl` and `spc`: an address and a count, both
/// octal, either of which may be left out.
fn parse_dump(memory: Memory, arg: &str) -> Result<Option<Command>, String> {
    let name = memory.name();
    let mut args = arg.split_whitespace();
    let octal = |what: &str, s: &str| {
        usize::from_str_radix(s, 8).map_err(|_| format!("{name} wants an octal {what}, not {s}"))
    };
    let from = match args.next() {
        Some(s) => octal("address", s)?,
        None => 0,
    };
    let words = match args.next() {
        Some(s) => Some(octal("count", s)?),
        None => None,
    };
    if args.next().is_some() {
        return Err(format!("{name} takes an address and a count, no more"));
    }
    Ok(Some(Command::Dump { memory, from, words }))
}

/// How many words a dump writes to a line.
const PER_LINE: usize = 4;

/// A memory dumped: four words to a line, the octal address of the first
/// at the left, each word in hex, and the four characters each word holds
/// at the right.  `words[0]` is at `from`.
///
/// A line the same as the one above it is written as a `*`, so that a
/// memory that is mostly one value is a few lines.  The last line is
/// always written whole, so the dump ends at the address it reached.
pub fn dump(words: &[u32], from: usize) -> String {
    let mut s = String::new();
    let last = words.len().div_ceil(PER_LINE);
    let mut above: Option<&[u32]> = None;
    let mut starred = false;
    for (i, line) in words.chunks(PER_LINE).enumerate() {
        if above == Some(line) && i + 1 < last {
            if !starred {
                s.push_str("*\n");
                starred = true;
            }
            continue;
        }
        starred = false;
        above = Some(line);
        write!(s, "{:06o} ", from + i * PER_LINE).unwrap();
        for w in line {
            write!(s, " {w:08x}").unwrap();
        }
        for _ in line.len()..PER_LINE {
            s.push_str("         ");
        }
        s.push(' ');
        for w in line {
            write!(s, " {}", characters(*w)).unwrap();
        }
        s.push('\n');
    }
    s
}

/// Registers, one to a line: the name, the value in hex, and the four
/// characters it holds.
pub fn registers(rows: &[(&str, u32)]) -> String {
    let mut s = String::new();
    for (name, v) in rows {
        writeln!(s, "{name:<18} {v:08x}  {}", characters(*v)).unwrap();
    }
    s
}

/// The four characters a word holds, the first in the low byte.  That the
/// first is the low one is the disk pack's own label: block 0 word 0 is
/// `0o11420440514`, which the boot PROM checks and which reads `LABL` low
/// byte first.
fn characters(word: u32) -> String {
    (0..4).map(|i| character((word >> (i * 8)) as u8)).collect()
}

/// The Lisp Machine graphics, 001 to 037 in order, with MIT's own names
/// for them.  See [`character`].
const GRAPHICS: [(char, &str); 31] = [
    ('↓', "down arrow"),
    ('α', "alpha"),
    ('β', "beta"),
    ('∧', "and-sign"),
    ('¬', "not-sign"),
    ('ε', "epsilon"),
    ('π', "pi"),
    ('λ', "lambda"),
    ('γ', "gamma"),
    ('δ', "delta"),
    ('↑', "uparrow"),
    ('±', "plus-minus"),
    ('⊕', "circle-plus"),
    ('∞', "infinity"),
    ('∂', "partial delta"),
    ('⊂', "left horseshoe"),
    ('⊃', "right horseshoe"),
    ('∩', "up horseshoe"),
    ('∪', "down horseshoe"),
    ('∀', "universal quantifier"),
    ('∃', "existential quantifier"),
    ('⊗', "circle-X"),
    ('↔', "double-arrow"),
    ('←', "left arrow"),
    ('→', "right arrow"),
    ('≠', "not-equals"),
    ('◊', "diamond (alt)"),
    ('≤', "less-or-equal"),
    ('≥', "greater-or-equal"),
    ('≡', "equivalence"),
    ('∨', "or"),
];

/// A byte as the Lisp Machine's character set has it: 001 to 037 the
/// graphics in `GRAPHICS`, 040 to 176 ASCII, and a dot for the rest.
///
/// The set is MIT's own, from the character table in the System 100
/// release, which gives the codes and the names in `GRAPHICS`, calls
/// 040 to 176 ASCII, and has 200 upwards as keys --- break, call, rubout,
/// return --- and font switches, which are not printing characters.  Those
/// are the dot, and so are 000 and 177, the two the table itself marks
/// with a question mark.
pub fn character(code: u8) -> char {
    match code {
        1..=0o37 => GRAPHICS[code as usize - 1].0,
        0o40..=0o176 => code as char,
        _ => '.',
    }
}

/// The name the character table gives a graphic, for the test that checks
/// `GRAPHICS` against the table itself.
pub fn graphic_name(code: u8) -> Option<&'static str> {
    match code {
        1..=0o37 => Some(GRAPHICS[code as usize - 1].1),
        _ => None,
    }
}
