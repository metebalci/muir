// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading MIT's microcode symbol table: the names for the band.
//!
//! `sys/ubin/ucadr.sym` in the System 100 release is microcode 323's symbol
//! table, and `sys/ubin/promh.sym` is the boot PROM's in the same format.
//! Without it a divergence past `PROM-DISABLE` is a bare octal PC; with it,
//! `0o27775` is `DISK-COPY-SECTION` and can be read in MIT's own source.
//!
//! The file is MIT's own, written by their assembler:
//!
//! ```text
//! -4
//! (I-MEM-LOC 30241 D-MEM-LOC 164 ... )
//! -2 BIGNUM-MINUS I-MEM 15120
//! TV-DRAW-TRI-INCR-1 I-MEM 21403
//! ...
//! -1
//! ```
//!
//! A leading `-4` introduces one s-expression of assembler state, `-2` the
//! symbols, and `-1` ends the file. Every symbol line is a name, a memory,
//! and an octal address. The memories are `I-MEM`, `A-MEM`, `M-MEM`, `D-MEM`
//! and `NUMBER`. A `NUMBER` is an assembly-time constant and not a location:
//! it is kept apart, and it is the only kind that can be negative --- three
//! of microcode 323's are.
//!
//! **Names are not unique per address.** Microcode 323 has 2,264 `I-MEM`
//! symbols on 2,221 distinct addresses: an entry point can be known by more
//! than one name, so a lookup returns all of them.

use std::collections::BTreeMap;

/// Which memory a symbol names a place in.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Space {
    IMem,
    AMem,
    MMem,
    DMem,
}

impl Space {
    fn parse(s: &str) -> Option<Space> {
        Some(match s {
            "I-MEM" => Space::IMem,
            "A-MEM" => Space::AMem,
            "M-MEM" => Space::MMem,
            "D-MEM" => Space::DMem,
            _ => return None,
        })
    }
}

/// One memory's symbols, by address, every name at each; and the
/// assembly-time constants, which have no address.
#[derive(Clone, Debug, Default)]
pub struct Symbols {
    by_address: BTreeMap<Space, BTreeMap<u32, Vec<String>>>,
    constants: BTreeMap<String, i64>,
}

/// Reads a symbol table.  Unknown memories are an error rather than being
/// skipped: a memory we do not know is a file we do not understand.
pub fn parse(text: &str) -> Result<Symbols, String> {
    let mut by_address: BTreeMap<Space, BTreeMap<u32, Vec<String>>> = BTreeMap::new();
    let mut constants: BTreeMap<String, i64> = BTreeMap::new();
    for line in text.lines() {
        // `-4` carries the assembler's state on the following line, and `-1`
        // ends the file.  Neither is a symbol.
        let line = line.strip_prefix("-2 ").unwrap_or(line);
        if line.is_empty() || line.starts_with('-') || line.starts_with('(') {
            continue;
        }
        let mut f = line.split_whitespace();
        let (Some(name), Some(space), Some(addr)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        if space == "NUMBER" {
            let v = i64::from_str_radix(addr, 8)
                .map_err(|e| format!("{name} NUMBER: {addr:?} is not octal: {e}"))?;
            constants.insert(name.to_string(), v);
            continue;
        }
        let Some(space) = Space::parse(space) else {
            return Err(format!("unknown memory {space} in {line:?}"));
        };
        let addr = u32::from_str_radix(addr, 8)
            .map_err(|e| format!("{name} {space:?}: {addr:?} is not octal: {e}"))?;
        by_address.entry(space).or_default().entry(addr).or_default().push(name.to_string());
    }
    Ok(Symbols { by_address, constants })
}

impl Symbols {
    /// Every name at exactly this address.
    pub fn at(&self, space: Space, addr: u32) -> &[String] {
        self.by_address
            .get(&space)
            .and_then(|m| m.get(&addr))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// The nearest symbol at or below `addr`, and how far past it `addr` is.
    /// This is what names an arbitrary PC: microcode 323 labels entry points,
    /// not every instruction.
    pub fn nearest(&self, space: Space, addr: u32) -> Option<(&str, u32)> {
        let (at, names) = self.by_address.get(&space)?.range(..=addr).next_back()?;
        Some((names.first()?.as_str(), addr - at))
    }

    /// `DISK-COPY-SECTION` for the label itself, `DISK-COPY-SECTION+1` for the
    /// instruction after it, and `None` if nothing is at or below.
    pub fn name(&self, space: Space, addr: u32) -> Option<String> {
        let (name, off) = self.nearest(space, addr)?;
        Some(if off == 0 { name.to_string() } else { format!("{name}+{off}") })
    }

    /// The address of a name in this memory, if it has one: `A-DISK-IDLE-TIME`
    /// to its word of A memory, for a test that reads the microcode's own
    /// state.
    pub fn address(&self, space: Space, name: &str) -> Option<u32> {
        self.by_address
            .get(&space)?
            .iter()
            .find(|(_, names)| names.iter().any(|n| n == name))
            .map(|(&addr, _)| addr)
    }

    /// How many addresses this memory has symbols for.
    pub fn len(&self, space: Space) -> usize {
        self.by_address.get(&space).map_or(0, BTreeMap::len)
    }

    pub fn is_empty(&self, space: Space) -> bool {
        self.len(space) == 0
    }

    /// An assembly-time constant by name.  Three of microcode 323's are
    /// negative, which is why these are signed and not addresses.
    pub fn constant(&self, name: &str) -> Option<i64> {
        self.constants.get(name).copied()
    }

    pub fn constants(&self) -> &BTreeMap<String, i64> {
        &self.constants
    }
}
