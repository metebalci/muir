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
    /// The keyboard mapping in force: what a viewer's keysyms mean on the
    /// Lisp Machine keyboard, for a user who cannot type a key and wants
    /// to know what would.
    Keys,
    /// A net or a bus on one of the machine's boards, by the name the
    /// drawings give it: what the wire is doing at this instant.
    ///
    /// `chip` only. The other engines have registers and memories and no
    /// nets at all, which is the same reason [`Command::Registers`] is
    /// refused the other way round.
    Net(NetName),
    /// The nets recorded over the next so many microcycles from here, at
    /// every instant the boards move, one line on stderr per change: what
    /// `--watch` does from the command line, for a machine already
    /// running.  `chip` only, as [`Command::Net`] is.
    Watch {
        /// How many microcycles from now.
        cycles: u64,
        /// What to record, each as `net` names one.
        nets: Vec<NetName>,
    },
    /// So many words of main memory from a **physical** address.
    ///
    /// The address is the one the Xbus carries --- the board and the cell
    /// on it --- and is never translated through the map.  A program's own
    /// address is virtual and is not this; the band's CCW list, which is
    /// what wanted the command, is physical.  [`main_dump`] says why the
    /// choice is that way round and what is refused.
    Mem {
        from: usize,
        words: usize,
    },
    /// End the run, as a stop does.
    Quit,
    Help,
}

/// A net or a bus, named as the drawings name it and as `net`, `watch`
/// and `--watch` all take one: `[<board>:]<name>[/<width>]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetName {
    /// Which board, where more than one has the name; `None` searches
    /// them in order and says which one answered.
    pub board: Option<String>,
    /// The net's name, or a bus's prefix.
    pub name: String,
    /// A bus of this many bits, `NAME0` up, as `MUIR_WATCH` writes it.
    pub width: Option<u32>,
}

impl std::fmt::Display for NetName {
    /// As it was written: the board, the name and the width, which is
    /// what a record labels a value with.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(b) = &self.board {
            write!(f, "{b}:")?;
        }
        f.write_str(&self.name)?;
        if let Some(w) = self.width {
            write!(f, "/{w}")?;
        }
        Ok(())
    }
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
mem <address> [n]       main memory: the word at a physical address, or n
                        words from there. Physical, and never through the
                        map: a program's own address is not one. Answered
                        while the machine runs, as net is
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
keys                    the keyboard mapping in force: what a viewer's
                        keysyms mean on the Lisp Machine keyboard
net [board:]name[/width]
                        chip: what a net is doing at this instant, by the
                        name the drawings give it --- `net TRIDENT.READY/`
                        --- or a bus of that many bits from `name0` up,
                        as MUIR_WATCH writes one. A board where more than
                        one carries the name: cpu, busint, memory, io, tv,
                        disk
watch <n> <net>,<net>,...
                        chip: record the nets, each named as net names
                        one and comma separated, over the next n
                        microcycles from here: one line on stderr,
                        prefixed `watch:`, at every change, sampled at
                        every instant the boards move --- what --watch
                        does from the command line
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
        "keys" => bare(Command::Keys),
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
        "mem" => parse_mem(arg),
        "net" => parse_net_name(arg, "net").map(|n| Some(Command::Net(n))),
        "watch" => parse_watch(arg),
        "screenshot" | "ss" => Ok(Some(Command::Screenshot(file(arg)))),
        "startcapture" | "sc" => Ok(Some(Command::StartCapture(file(arg)))),
        "endcapture" | "ec" => bare(Command::EndCapture),
        "checkpoint" => Ok(Some(Command::Checkpoint(file(arg)))),
        other => Err(format!("{other} is no command; help lists them")),
    }
}

/// `[<board>:]<name>[/<width>]`: how `net` names a net, and how `watch`
/// and `--watch` name each of theirs.  `who` is which of them is asking,
/// for what is refused.
///
/// The name is taken whole, spaces and all, because MIT's own net names
/// have spaces in them --- `SYNC PROM ENB`, `-UNIT 0 ENB` --- and quoting
/// them at a prompt would be one more thing to get wrong. So the board and
/// the width are recognised by their punctuation and everything else is
/// the name.
pub fn parse_net_name(arg: &str, who: &str) -> Result<NetName, String> {
    let arg = arg.trim();
    if arg.is_empty() {
        return Err(format!("{who} wants a name, as the drawings write it"));
    }
    // A board prefix is a word before a colon, and a net name never has
    // one: `TRIDENT.0.SELECT/` and `-XBUS RQ` carry no colons.
    let (board, rest) = match arg.split_once(':') {
        Some((b, r)) if !b.contains(char::is_whitespace) => (Some(b.trim().to_string()), r.trim()),
        _ => (None, arg),
    };
    // A width is a number after the last slash. A name may end in one ---
    // `TRIDENT.READY/` --- and then there is nothing after the slash to
    // parse as a number, so the same arm that rejects `FOO/bar` takes it.
    let (name, width) = match rest.rsplit_once('/') {
        Some((n, w)) => match w.trim().parse::<u32>() {
            Ok(bits) if (1..=64).contains(&bits) => (n.trim(), Some(bits)),
            Ok(_) => return Err(format!("a bus is 1 to 64 bits, not {w}")),
            Err(_) => (rest, None),
        },
        _ => (rest, None),
    };
    if name.is_empty() {
        return Err(format!("{who} wants a name, as the drawings write it"));
    }
    Ok(NetName { board, name: name.to_string(), width })
}

/// `<net>,<net>,...`: the nets `watch` and `--watch` record, each as
/// [`parse_net_name`] takes one.  `who` is which of the two is asking.
///
/// A comma is the separator and nothing else, so the one kind of name the
/// list cannot carry is one with a comma in it: the memory board's `-CAS
/// 0,1 LH` and its three fellows.  They can be asked about one at a time
/// with `net`.
pub fn parse_net_names(list: &str, who: &str) -> Result<Vec<NetName>, String> {
    if list.trim().is_empty() {
        return Err(format!("{who} wants a net to record, as the drawings write it"));
    }
    list.split(',').map(|s| parse_net_name(s, who)).collect()
}

/// `watch <n> <net>,<net>,...`: a count of microcycles, then the nets,
/// which are the rest of the line.
fn parse_watch(arg: &str) -> Result<Option<Command>, String> {
    let (count, list) = arg.split_once(char::is_whitespace).unwrap_or((arg, ""));
    let cycles = match count.parse::<u64>() {
        Ok(n) if n > 0 => n,
        _ => return Err(format!("watch wants a count of microcycles, not {count}")),
    };
    let nets = parse_net_names(list, "watch")?;
    Ok(Some(Command::Watch { cycles, nets }))
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

/// `mem <address> [<words>]`: a physical address, which is required, and a
/// count of words, which is one if it is left out.  Both octal, as the
/// other dumps' are.
///
/// **The address is not optional, and the count is one.** `amem` and the
/// rest default to the whole memory because the whole of the largest of
/// them is 2048 words; main memory is two million, and a command that
/// wrote half a million lines for a mistyped line is not a default.
fn parse_mem(arg: &str) -> Result<Option<Command>, String> {
    let mut args = arg.split_whitespace();
    let octal = |what: &str, s: &str| {
        usize::from_str_radix(s, 8).map_err(|_| format!("mem wants an octal {what}, not {s}"))
    };
    let Some(from) = args.next() else {
        return Err("mem wants a physical address, in octal".to_string());
    };
    let from = octal("address", from)?;
    let words = match args.next() {
        Some(s) => octal("count", s)?,
        None => 1,
    };
    if args.next().is_some() {
        return Err("mem takes an address and a count, no more".to_string());
    }
    Ok(Some(Command::Mem { from, words }))
}

/// Words the Xbus can name: its address is 22 wires, `-XADDR0` to
/// `-XADDR21` in `data/busint-connectors.txt`, so no physical address is
/// larger than this.  A machine holds as many of them as it has memory
/// boards, 64K a board, and the top four boards' worth is the Xbus I/O
/// space the devices answer rather than memory
/// ([`crate::busint::MAX_MEMORY_BOARDS`]).
pub const PHYSICAL_WORDS: usize = 1 << 22;

/// `mem`'s answer: `words` words of main memory from the physical address
/// `from`, in [`dump`]'s shape, or why there are none there.  `fitted` is
/// how many words the machine has, and `read` gives one of them.
///
/// **The address is physical, and nothing here translates it.** The two
/// reasons are that a physical address is the only kind that names a word
/// of main memory --- on `chip` main memory is cells on a board, and it is
/// the physical address that says which board and which cell --- and that
/// a virtual one would have to be read through the map as it stands this
/// microcycle, so the same argument would name different words at
/// different instants.  A command whose meaning moved while the machine
/// ran would be worse than none.
///
/// A virtual address is refused where it can be told apart from a physical
/// one, which is above the 22 bits the Xbus carries; below that the two are
/// the same numbers and nothing can tell, which is why the help says which
/// this takes.
pub fn main_dump(
    from: usize,
    words: usize,
    fitted: usize,
    read: impl Fn(usize) -> Option<u32>,
) -> Result<String, String> {
    if from >= PHYSICAL_WORDS {
        return Err(format!(
            "the Xbus carries 22 bits of address and {from:o} is more than that; \
             mem takes a physical address and does not go through the map"
        ));
    }
    if from >= fitted {
        return Err(format!(
            "main memory here is {fitted:o} words, 0 to {:o}, and {from:o} is past its end",
            fitted.saturating_sub(1)
        ));
    }
    // A count past the end is the rest of the memory, as the other dumps'
    // is: the largest count the prompt reads would otherwise overflow the
    // address it is added to.
    let to = from.saturating_add(words).min(fitted);
    let mut all = Vec::with_capacity(to - from);
    for a in from..to {
        match read(a) {
            Some(w) => all.push(w),
            // Inside `fitted` and nothing there: the reader and the count
            // disagree about what the machine has, which is a bug in
            // whichever of them is wrong and not a word to print.
            None => return Err(format!("nothing holds {a:o}, though the machine has that word")),
        }
    }
    Ok(dump(&all, from))
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
