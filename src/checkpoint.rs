// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A checkpoint: a machine's whole state in a file, for a run to be picked
//! up from where another stopped.
//!
//! The engine and everything it holds --- the processor's memories and
//! registers, main memory, the display, the I/O board, the drives with
//! their positions and every block written to their packs --- go through
//! a [`Writer`] as fixed-width little-endian fields, in the one order the
//! matching [`Reader`] takes them back.  Each type saves and loads its own
//! fields, destructured whole so that a field added later has to be
//! placed.  Not in a checkpoint: the pack's file, which is only ever read
//! and a resume opens again; the Chaosnet's cable and the server on it,
//! plugged in afresh; the terminal, which a viewer reconnects to.
//!
//! The file is a header --- the magic, the format's version, the engine's
//! name, and how many memory boards the machine had, so that a resume can
//! build one the same size before reading the rest --- and the body packed
//! as runs.  Most of a machine's two million
//! words are zero, so the body is written as pairs of counts, zeros then
//! literal bytes, each an LEB128 varint, a run of at least
//! `MIN_ZERO_RUN` zero bytes standing as its count and everything else
//! copied.  An empty memory costs almost nothing; a full one costs itself.

use std::io::{self, Error, ErrorKind};
use std::path::Path;

use crate::clock::Speed;

const MAGIC: &[u8; 16] = b"muir checkpoint\n";

/// Bumped whenever any type changes what it writes; a file from another
/// version is refused rather than read wrong.
pub const VERSION: u32 = 6;

/// The shortest run of zero bytes worth a count of its own.
const MIN_ZERO_RUN: usize = 4;

/// The error a checkpoint that cannot be read gives.
pub fn bad(what: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::InvalidData, format!("checkpoint: {what}"))
}

/// The fields of a checkpoint, in order, little-endian.
#[derive(Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    pub fn new() -> Writer {
        Writer::default()
    }

    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }

    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    pub fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }

    pub fn speed(&mut self, s: Speed) {
        self.u8(s as u8);
    }

    /// A flag, then the value if there is one.
    pub fn opt<T>(&mut self, v: Option<T>, put: impl FnOnce(&mut Writer, T)) {
        self.bool(v.is_some());
        if let Some(x) = v {
            put(self, x);
        }
    }

    /// A count, then the items.
    pub fn bytes(&mut self, v: &[u8]) {
        self.u64(v.len() as u64);
        self.0.extend_from_slice(v);
    }

    pub fn u16s(&mut self, v: &[u16]) {
        self.u64(v.len() as u64);
        for &x in v {
            self.u16(x);
        }
    }

    pub fn u32s(&mut self, v: &[u32]) {
        self.u64(v.len() as u64);
        for &x in v {
            self.u32(x);
        }
    }

    pub fn u64s(&mut self, v: &[u64]) {
        self.u64(v.len() as u64);
        for &x in v {
            self.u64(x);
        }
    }

    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

/// The fields back, in the order they were written; each read says when
/// the file ends early or holds a value no field can take.
pub struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, at: 0 }
    }

    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.data.len());
        let Some(end) = end else {
            return Err(bad(format!("the file ends early, {n} bytes wanted at {}", self.at)));
        };
        let out = &self.data[self.at..end];
        self.at = end;
        Ok(out)
    }

    pub fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    pub fn bool(&mut self) -> io::Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            v => Err(bad(format!("{v} for a flag"))),
        }
    }

    pub fn speed(&mut self) -> io::Result<Speed> {
        Ok(match self.u8()? {
            0 => Speed::ExtraSlow,
            1 => Speed::Slow,
            2 => Speed::Normal,
            3 => Speed::Fast,
            v => return Err(bad(format!("{v} for a speed"))),
        })
    }

    pub fn opt<T>(
        &mut self,
        get: impl FnOnce(&mut Reader<'a>) -> io::Result<T>,
    ) -> io::Result<Option<T>> {
        if self.bool()? { get(self).map(Some) } else { Ok(None) }
    }

    /// A count that has to be the slot's own size, then the items into it.
    fn count_for(&mut self, slot: usize) -> io::Result<usize> {
        let n = self.u64()? as usize;
        if n != slot {
            return Err(bad(format!("{n} items where {slot} belong")));
        }
        Ok(n)
    }

    pub fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let n = self.u64()? as usize;
        Ok(self.take(n)?.to_vec())
    }

    pub fn bytes_into(&mut self, into: &mut [u8]) -> io::Result<()> {
        let n = self.count_for(into.len())?;
        into.copy_from_slice(self.take(n)?);
        Ok(())
    }

    pub fn u16s(&mut self) -> io::Result<Vec<u16>> {
        let n = self.u64()? as usize;
        (0..n).map(|_| self.u16()).collect()
    }

    pub fn u16s_into(&mut self, into: &mut [u16]) -> io::Result<()> {
        self.count_for(into.len())?;
        for x in into {
            *x = self.u16()?;
        }
        Ok(())
    }

    pub fn u32s(&mut self) -> io::Result<Vec<u32>> {
        let n = self.u64()? as usize;
        (0..n).map(|_| self.u32()).collect()
    }

    pub fn u32s_into(&mut self, into: &mut [u32]) -> io::Result<()> {
        self.count_for(into.len())?;
        for x in into {
            *x = self.u32()?;
        }
        Ok(())
    }

    pub fn u64s(&mut self) -> io::Result<Vec<u64>> {
        let n = self.u64()? as usize;
        (0..n).map(|_| self.u64()).collect()
    }

    pub fn u64s_into(&mut self, into: &mut [u64]) -> io::Result<()> {
        self.count_for(into.len())?;
        for x in into {
            *x = self.u64()?;
        }
        Ok(())
    }

    /// Nothing left: every field has been taken, and no more were written.
    pub fn done(&self) -> io::Result<()> {
        match self.data.len() - self.at {
            0 => Ok(()),
            n => Err(bad(format!("{n} bytes left over"))),
        }
    }
}

// --- The file ---------------------------------------------------------------

/// A checkpoint read back: what its header says, and its body.
#[derive(Debug)]
pub struct Checkpoint {
    /// The engine that wrote it, `micro` or `rtl`.
    pub engine: String,
    /// How many 64K-word memory boards the machine had.
    pub memory_boards: usize,
    pub body: Vec<u8>,
}

/// Writes `body`, packed, under the header naming `engine` and the
/// machine's `memory_boards`, and says how big the file came.
pub fn write(path: &Path, engine: &str, memory_boards: usize, body: &[u8]) -> io::Result<u64> {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.push(engine.len() as u8);
    out.extend_from_slice(engine.as_bytes());
    out.extend_from_slice(&(memory_boards as u32).to_le_bytes());
    out.extend_from_slice(&pack(body));
    std::fs::write(path, &out)?;
    Ok(out.len() as u64)
}

/// The checkpoint at `path`.
pub fn read(path: &Path) -> io::Result<Checkpoint> {
    let file = std::fs::read(path)?;
    let mut r = Reader::new(&file);
    if r.take(MAGIC.len()).ok() != Some(MAGIC.as_slice()) {
        return Err(bad("not a muir checkpoint"));
    }
    let version = r.u32()?;
    if version != VERSION {
        return Err(bad(format!("format version {version}; this build reads {VERSION}")));
    }
    let n = r.u8()? as usize;
    let engine = std::str::from_utf8(r.take(n)?).map_err(|_| bad("the engine's name"))?;
    let memory_boards = r.u32()? as usize;
    let body = unpack(&file[r.at..])?;
    Ok(Checkpoint { engine: engine.to_string(), memory_boards, body })
}

// --- Packing ----------------------------------------------------------------

fn varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn unvarint(data: &[u8], at: &mut usize) -> io::Result<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let Some(&byte) = data.get(*at) else {
            return Err(bad("a count runs off the end"));
        };
        *at += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(bad("a count too long"))
}

/// `raw` as pairs of counts, zeros then literals, and the literal bytes.
pub fn pack(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let zeros = raw[i..].iter().take_while(|&&b| b == 0).count();
        i += zeros;
        let start = i;
        while i < raw.len() {
            if raw[i] != 0 {
                i += 1;
                continue;
            }
            let run = raw[i..].iter().take_while(|&&b| b == 0).count();
            if run >= MIN_ZERO_RUN {
                break;
            }
            i += run;
        }
        varint(&mut out, zeros as u64);
        varint(&mut out, (i - start) as u64);
        out.extend_from_slice(&raw[start..i]);
    }
    out
}

/// The most a body can be, which is what a run of zeros is held to before
/// room is made for it: the header does not say how long the body is, and
/// a corrupt count would otherwise ask for exabytes.  Four gigabytes is
/// past the largest machine the format describes --- sixty memory boards
/// are 15 MiB, and eight T300 packs with every block written since they
/// were loaded, the most the drives can carry, a little over 2 GB.
const MAX_BODY: u64 = 1 << 32;

/// The bytes [`pack`] was given.
pub fn unpack(packed: &[u8]) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < packed.len() {
        let zeros = unvarint(packed, &mut at)?;
        let literals = unvarint(packed, &mut at)? as usize;
        let Some(end) = at.checked_add(literals).filter(|&e| e <= packed.len()) else {
            return Err(bad("literals run off the end"));
        };
        if zeros > MAX_BODY.saturating_sub(out.len() as u64) {
            return Err(bad(format!(
                "a run of {zeros} zeros, past the {MAX_BODY} bytes a body can be"
            )));
        }
        out.resize(out.len() + zeros as usize, 0);
        out.extend_from_slice(&packed[at..end]);
        at = end;
    }
    Ok(out)
}
