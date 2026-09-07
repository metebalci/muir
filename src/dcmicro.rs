// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! MIT's `MICRO` assembler, for the disk controller's microcode.
//!
//! The disk controller is microprogrammed: three 512 x 8 74S472s at `dcui`
//! 0D03, 0D04 and 0D05 hold a 512 x 24 control store, and the program in
//! them is `cadrdc/newdsk.31`, "Lisp Machine Disk Control Microcode". MIT
//! assembled it with `MICRO 52`, a PDP-10 program that did not survive on
//! the tapes, and burned the PROMs from the assembler's listing with
//! `cadrdc/newdsk.trans`, a thirty-line Lisp program that did. Neither the
//! assembled `NEWDSK MCR` nor the PROM images survived --- they lived in
//! Moon's own directory, which was not dumped --- so the assembler is here.
//!
//! **The language**, read off the two sources and `newdsk.31`'s own comment
//! block:
//!
//! - `NAME/=J,K,L,M` defines a field: `J` its default, `K` its width in bits,
//!   `L` "23 minus rightmost bit#, due to backwards pdp10 bit numbering"
//!   --- so the field's least significant bit is `23 - L` and it runs up
//!   from there ([`Field::shift`] says how MIT's own listing settles that
//!   against the mirror reading) --- and `M` is `D` for a field that takes
//!   its default when an instruction leaves it out. `J`, `K` and `L` are
//!   decimal, being bit counts, where everything else here is octal. Two
//!   fields may share bits: `WRITE` and `ECC` are one hardware field, `WS`.
//! - ` NAME=value`, indented, defines a value symbol for the field defined
//!   last. Values are octal, as everything numeric here is.
//! - `NAME "items"` defines a macro: a name that stands for the quoted
//!   list of items wherever an instruction uses it.
//! - `NNN: items` is an instruction at octal address `NNN`; a line ending in
//!   a comma continues on the next. An item is `FIELD/VALUE`, the value a
//!   symbol of that field's or an octal number, or a macro's name. Under
//!   `.SEQADR` an instruction with no address follows the one before it.
//! - `.TITLE "..."` names the program; `;` starts a comment; a form feed is
//!   a page break.
//!
//! **The check** is a round trip that survived for the sister program:
//! `cadrdc/mksman.39`, the Marksman controller's microcode, has its MICRO
//! listing `mksman.mcr` and its PROM images `mksman.d03/d04/d05` beside it,
//! all made by MIT on 25 October 1979. `tests/dcmicro.rs` assembles the
//! source and holds every word to the listing and every byte to the images
//! before this is pointed at `newdsk.31`, for which nothing assembled
//! survived.

use std::collections::BTreeMap;

/// The control store's word: 512 x 24, three 74S472s of eight bits.
pub const WORD_BITS: u32 = 24;

/// One field of the microword.
#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    pub default: u32,
    pub width: u32,
    /// `L` as the source writes it: "23 minus rightmost bit#, due to
    /// backwards pdp10 bit numbering". See [`Field::shift`].
    pub l: u32,
    pub defaultable: bool,
    /// The field's value symbols, in order of definition.
    pub values: BTreeMap<String, u32>,
}

impl Field {
    /// The field's least significant bit, counting the word's own least
    /// significant bit as 0.
    ///
    /// MIT's rule, and it means what it says: `L` is 23 *minus* the
    /// rightmost bit number, in the PDP-10's numbering where bit 0 is the
    /// most significant. So the rightmost bit in ordinary numbering is
    /// `23 - L`, and the field runs up from there.
    ///
    /// The mirror reading --- `L` as the field's *top* bit, running down ---
    /// tiles the 24-bit word just as exactly, with no gap and no overlap,
    /// so the definitions alone cannot choose between them. MIT's own
    /// assembled listing does. `mksman.mcr` has `10000000` at location 0
    /// for `CLK/1 USEC,AWAIT CRDY`: bit 21, which under this rule is the
    /// top bit of `LOOP` --- await-controller-ready being a loop condition,
    /// as `ON CYLINDER` and `PREAMBLE DETECT` are --- and under the mirror
    /// would be `FUNC/1`, a miscellaneous function, which that instruction
    /// does not ask for. Location 1 agrees: `00000030`, bits 3 and 4, is
    /// `MK BUS/3` for `CMD 1 TO MK BUS`, where the mirror makes it `LOOP/6`.
    pub fn shift(&self) -> u32 {
        WORD_BITS - 1 - self.l
    }

    /// The field's most significant bit.
    pub fn top(&self) -> u32 {
        self.shift() + self.width - 1
    }

    fn mask(&self) -> u32 {
        ((1u32 << self.width) - 1) << self.shift()
    }
}

/// What assembling a source gives: the words by address, and the
/// definitions they were built from.
#[derive(Clone, Debug, Default)]
pub struct Assembly {
    pub title: String,
    /// 24-bit words by octal address, only the addresses the source assigns.
    pub words: BTreeMap<u16, u32>,
    pub fields: Vec<Field>,
    pub macros: BTreeMap<String, String>,
}

impl Assembly {
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// The value of a field in a word.
    pub fn extract(&self, word: u32, field: &str) -> Option<u32> {
        let f = self.field(field)?;
        Some((word & f.mask()) >> f.shift())
    }

    /// The three PROM images as `newdsk.trans` cuts them: D03 the low
    /// eight bits, D04 the middle eight, D05 the high eight, each 512
    /// bytes with unassigned locations zero.
    ///
    /// The translator works from the listing's `HIGH12`/`LOW12` halves ---
    /// `D03 = LOW12 & 377`, `D04 = (HIGH12 & 17) << 4 + (LOW12 & 7400) >>
    /// 8`, `D05 = HIGH12 >> 4` --- which is the 24-bit word's three bytes.
    pub fn proms(&self) -> [Vec<u8>; 3] {
        let mut out = [vec![0u8; 512], vec![0u8; 512], vec![0u8; 512]];
        for (&addr, &word) in &self.words {
            let a = addr as usize;
            out[0][a] = (word & 0xff) as u8;
            out[1][a] = ((word >> 8) & 0xff) as u8;
            out[2][a] = ((word >> 16) & 0xff) as u8;
        }
        out
    }
}

/// Assembles a MICRO source.
pub fn assemble(source: &str) -> Result<Assembly, String> {
    let mut asm = Assembly::default();
    let mut seqadr = false;
    let mut next_addr: Option<u16> = None;
    // An instruction under construction: its address and its items so far,
    // carried across continuation lines.
    let mut pending: Option<(Option<u16>, Vec<String>)> = None;

    for (lineno, raw) in source.lines().enumerate() {
        let lineno = lineno + 1;
        let err = |what: String| format!("line {lineno}: {what}");
        // A form feed is a page break. Comments run to the end of the line,
        // and a macro's quoted body never holds a semicolon.
        let line = raw.replace('\x0c', "");
        let line = match line.find(';') {
            Some(k) => &line[..k],
            None => &line,
        };
        if line.trim().is_empty() {
            continue;
        }

        if let Some(rest) = line.trim_start().strip_prefix('.') {
            let mut words = rest.splitn(2, char::is_whitespace);
            match words.next() {
                Some("TITLE") => {
                    asm.title = words.next().unwrap_or("").trim().trim_matches('"').to_string();
                }
                Some("SEQADR") => seqadr = true,
                Some(other) => return Err(err(format!("unknown pseudo-op .{other}"))),
                None => {}
            }
            continue;
        }

        // A continuation: the line before ended in a comma.
        if let Some((_addr, items)) = pending.as_mut() {
            let (more, continues) = split_items(line);
            items.extend(more);
            if !continues {
                let (addr, items) = pending.take().unwrap();
                let addr = match (addr, seqadr, next_addr) {
                    (Some(a), _, _) => a,
                    (None, true, Some(a)) => a,
                    (None, true, None) => return Err(err("no address to follow".into())),
                    (None, false, _) => return Err(err("instruction without an address".into())),
                };
                let word = build(&asm, &items).map_err(err)?;
                if asm.words.insert(addr, word).is_some() {
                    return Err(err(format!("location {addr:o} assigned twice")));
                }
                next_addr = Some(addr + 1);
            }
            continue;
        }

        // A field definition: NAME/=J,K,L,M.
        if let Some((name, spec)) = line.split_once("/=") {
            let name = name.trim().to_string();
            let parts: Vec<&str> = spec.split(',').map(str::trim).collect();
            if parts.len() < 3 {
                return Err(err(format!("field {name}: want J,K,L,M")));
            }
            // J, K and L are the default, the width and the top bit ---
            // structural bit counts, decimal, so that `L is 23 minus
            // rightmost bit#` reads as MIT wrote it (`GET DATA/=0,1,8` is
            // bit 8, and 8 is not an octal digit). Field *values* below are
            // octal, the assembler's own radix.
            let dec = |s: &str| s.parse::<u32>().map_err(|e| err(format!("{s}: {e}")));
            asm.fields.push(Field {
                name,
                default: dec(parts[0])?,
                width: dec(parts[1])?,
                l: dec(parts[2])?,
                defaultable: parts.get(3).is_some_and(|m| m.trim() == "D"),
                values: BTreeMap::new(),
            });
            continue;
        }

        // A macro: NAME "items".
        if let Some(open) = line.find('"') {
            let name = line[..open].trim().to_string();
            let body = line[open + 1..].trim_end().trim_end_matches('"').to_string();
            if name.is_empty() {
                return Err(err("macro without a name".into()));
            }
            asm.macros.insert(name, body);
            continue;
        }

        // A value symbol for the field defined last: indented NAME=value.
        if line.starts_with(char::is_whitespace)
            && let Some((name, value)) = line.split_once('=')
            && !name.contains('/')
        {
            let value = u32::from_str_radix(value.trim(), 8)
                .map_err(|e| err(format!("{}: {e}", value.trim())))?;
            let field =
                asm.fields.last_mut().ok_or_else(|| err("value before any field".into()))?;
            field.values.insert(name.trim().to_string(), value);
            continue;
        }

        // An instruction, addressed or not.
        let (addr, body) = match line.split_once(':') {
            Some((a, rest)) if a.trim().chars().all(|c| c.is_digit(8)) && !a.trim().is_empty() => {
                (Some(u16::from_str_radix(a.trim(), 8).map_err(|e| err(e.to_string()))?), rest)
            }
            _ => (None, line),
        };
        let (items, continues) = split_items(body);
        if continues {
            pending = Some((addr, items));
            continue;
        }
        let addr = match (addr, seqadr, next_addr) {
            (Some(a), _, _) => a,
            (None, true, Some(a)) => a,
            (None, true, None) => return Err(err("no address to follow".into())),
            (None, false, _) => return Err(err("instruction without an address".into())),
        };
        let word = build(&asm, &items).map_err(err)?;
        if asm.words.insert(addr, word).is_some() {
            return Err(err(format!("location {addr:o} assigned twice")));
        }
        next_addr = Some(addr + 1);
    }
    if pending.is_some() {
        return Err("source ends inside an instruction".into());
    }
    Ok(asm)
}

/// Splits an instruction's items on commas, and says whether the line
/// ended in one, which continues the instruction on the next line.
fn split_items(body: &str) -> (Vec<String>, bool) {
    let trimmed = body.trim();
    let continues = trimmed.ends_with(',');
    let items =
        trimmed.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
    (items, continues)
}

/// Builds one word from an instruction's items: every defaultable field at
/// its default, then each item's field set to its value. A field set twice
/// in one instruction to two different values is an error; the same value
/// twice, as a macro and an item can produce, is not.
fn build(asm: &Assembly, items: &[String]) -> Result<u32, String> {
    let mut word = 0u32;
    for f in &asm.fields {
        if f.defaultable {
            word = (word & !f.mask()) | ((f.default << f.shift()) & f.mask());
        }
    }
    let mut set: Vec<(usize, u32)> = Vec::new();
    let mut queue: Vec<String> = items.to_vec();
    let mut expansions = 0;
    while let Some(item) = queue.first().cloned() {
        queue.remove(0);
        if let Some((field, value)) = item.split_once('/') {
            let field = field.trim();
            let value = value.trim();
            let (k, f) = asm
                .fields
                .iter()
                .enumerate()
                .find(|(_, f)| f.name == field)
                .ok_or_else(|| format!("no field {field:?} in {item:?}"))?;
            let v = match f.values.get(value) {
                Some(&v) => v,
                None => u32::from_str_radix(value, 8)
                    .map_err(|_| format!("{value:?} is not a value of {field}"))?,
            };
            if v >= 1 << f.width {
                return Err(format!("{value} does not fit {field}'s {} bits", f.width));
            }
            if let Some(&(_, before)) = set.iter().find(|&&(j, _)| j == k)
                && before != v
            {
                return Err(format!("{field} set to {before:o} and to {v:o} in one instruction"));
            }
            set.push((k, v));
            word = (word & !f.mask()) | (v << f.shift());
        } else if let Some(body) = asm.macros.get(&item) {
            expansions += 1;
            if expansions > 1000 {
                return Err(format!("macro {item:?} expands without end"));
            }
            let (more, _) = split_items(body);
            let mut rest = more;
            rest.extend(queue);
            queue = rest;
        } else {
            return Err(format!("{item:?} is neither FIELD/VALUE nor a macro"));
        }
    }
    Ok(word)
}

/// Reads the assembled words out of a MICRO listing, as `newdsk.trans`
/// does with Lisp `READ`: an object record is `U ADDR HIGH12 LOW12` in
/// octal, up to the first bare `END`.
///
/// Two things the Lisp reader does that this must. **`;` starts a comment**
/// that runs to the end of the line --- which is what makes the listing
/// readable at all, since every line carries its source text after the
/// object code (`U 0500, 1000,0000  ; 162  500:  CLK/1 USEC,...`) and the
/// pages of echoed source are wholly comment. Without that, the first `END`
/// token met is the one inside `LOOP`'s value name `END OF BYTE`, two
/// hundred lines before any object code, and nothing is read at all.
/// **A comma is whitespace**, which is what `(sstatus syntax /, 500500)` at
/// the top of `newdsk.trans` arranges.
///
/// Everything after `END` is the location index and the trailer, whose
/// `U 0200 ...` lines are line numbers and not words --- which is why the
/// translator stops where it does, and why parsing past `END` corrupts the
/// PROMs.
pub fn parse_listing(text: &str) -> Result<BTreeMap<u16, u32>, String> {
    let mut words = BTreeMap::new();
    let stripped: String = text
        .lines()
        .map(|l| match l.find(';') {
            Some(k) => &l[..k],
            None => l,
        })
        .collect::<Vec<&str>>()
        .join("\n");
    let mut tokens =
        stripped.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty());
    while let Some(t) = tokens.next() {
        match t {
            "U" => {
                let mut next = |what: &str| -> Result<u32, String> {
                    let s = tokens
                        .next()
                        .ok_or_else(|| format!("listing ends inside a U record ({what})"))?;
                    u32::from_str_radix(s, 8).map_err(|e| format!("{what} {s:?}: {e}"))
                };
                let addr = next("ADDR")? as u16;
                let high = next("HIGH12")?;
                let low = next("LOW12")?;
                words.insert(addr, (high << 12) | low);
            }
            "END" => break,
            _ => {}
        }
    }
    Ok(words)
}

/// Writes a PROM image in the form `newdsk.trans` wrote them and
/// [`crate::prom::parse_mit`] reads: a title line, a blank, `address
/// value` in octal with a tab between and a space after, and `END`.
pub fn write_prom(title: &str, image: &[u8]) -> String {
    let mut out = format!("{title}\n\n");
    for (i, &v) in image.iter().enumerate() {
        out += &format!("{i:o}\t{v:o} \n");
    }
    out += "\nEND \n";
    out
}
