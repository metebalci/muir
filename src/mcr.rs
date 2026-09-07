// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading MCR microcode files.
//!
//! The format is the one MIT's microassembler writes and the boot PROM
//! reads, and both are in the System 100 release.  `sys/sys/qwmcr.lisp`
//! writes it: `WRITE-MCR-FILE` puts out the I memory as section 1, the D
//! memory as 2, one section 3 for the microcode symbol area and the A
//! memory as 4 (`WRITE-I-MEM I-MEM 1`, `WRITE-D-MEM D-MEM 2`,
//! `WRITE-MICRO-CODE-SYMBOL-AREA-PART-1`, `WRITE-A-MEM A-MEM 4`), each
//! section opening with `OUT32` of the code, a start address and a size.
//! `sys/ucadr/promh.text` reads it, `PROCESS-SECTION`: "Each section starts
//! with three words: The section type, the initial address, and the number
//! of locations ... Section codes are: 1 = I-MEM, 2 = D-MEM, 3 = MAIN-MEM,
//! 4 = A-M-MEM", and its `PROCESS-A-MEM-SECTION` runs into `DONE-LOADING`,
//! so the A-memory section is the last one read.  What follows it in the
//! file is the symbol area, padded to a page boundary
//! (`WRITE-MICRO-CODE-SYMBOL-AREA-PART-2`): [`Mcr::trailing_bytes`].
//!
//! Every word is 32 bits put out by `OUT32` as two 16-bit halves, the high
//! half first --- "Note non-standard order of 16-bit bytes" --- each half a
//! PDP-11 word, low byte first: `Reader::u32_pdp`.
//!
//! The main-memory section's size field is not a count of words in the
//! file; the note under [`parse`] says what it is.

use crate::isa::Insn;

/// System 100's own microcode as a file: `sys/ubin/ucadr.mcr` from the
/// release, which is microcode 323, the version this project targets.
///
/// Byte for byte what the release ships, committed in `mit/` beside the boot
/// PROM's own file so that what a pack made here loads is the target's own
/// microcode and not a copy of it from somewhere else.  `tests/mcr.rs` holds
/// the committed copy to the release's, the way `tests/prom.rs` does for the
/// PROM: two builds of a microcode version can both parse and both run, and
/// only the release's bytes settle which one this is.
///
/// It is 12,449 control store words --- `0o30241`, the count `tests/boot.rs`
/// finds in the `MCR1` partition of the System 100 pack after the boot PROM
/// has loaded it.
pub const UCADR_323: &[u8] = include_bytes!("../mit/sys/ubin/ucadr.mcr");

#[derive(Clone, Debug, Default)]
pub struct Mcr {
    /// Bytes after the last section header's data.  Not zero in practice: the
    /// A-memory section is the last one and the file runs on past it.
    pub trailing_bytes: usize,
    pub imem_start: u32,
    /// Control store words, in address order from `imem_start`.
    pub imem: Vec<Insn>,
    pub dmem_start: u32,
    /// Dispatch memory: 18 bits used --- parity in 17, R/P/N in 16:14,
    /// address in 13:0 (`ir.bits`).  `WRITE-D-MEM` puts each word out as
    /// two halves, the high one carrying bit 16 in its bit 0 and odd parity
    /// in its bit 1, the low one bits 15-0.
    pub dmem: Vec<u32>,
    pub amem_start: u32,
    pub amem: Vec<u32>,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).ok_or("length overflow")?;
        let s = self.b.get(self.at..end).ok_or_else(|| {
            format!("short read of {n} at offset {}, file is {} bytes", self.at, self.b.len())
        })?;
        self.at = end;
        Ok(s)
    }

    /// 32 bits in PDP-11 word order: the two 16-bit halves are swapped, each
    /// stored little-endian.
    fn u32_pdp(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok((b[1] as u32) << 24 | (b[0] as u32) << 16 | (b[3] as u32) << 8 | b[2] as u32)
    }

    fn u16_le(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok((b[1] as u16) << 8 | b[0] as u16)
    }

    /// One control store word: four 16-bit little-endian halves, most
    /// significant first --- `WRITE-I-MEM` puts out `(LDB 6020)`, `4020`,
    /// `2020` and `0020` of each word, "A high", "A low", "M high", "M low".
    /// The top 16 bits are unused; a microinstruction is 48 bits.
    fn insn(&mut self) -> Result<Insn, String> {
        let w1 = self.u16_le()? as u64;
        let w2 = self.u16_le()? as u64;
        let w3 = self.u16_le()? as u64;
        let w4 = self.u16_le()? as u64;
        if w1 != 0 {
            return Err(format!("control store word has bits above 48 set: {w1:#x}"));
        }
        Ok(Insn::new(w2 << 32 | w3 << 16 | w4))
    }
}

/// Parses a whole MCR file.
///
/// The section headers form a sequential stream.  A main-memory section is
/// four words and no data: `WRITE-MICRO-CODE-SYMBOL-AREA-PART-1` puts out
/// the code 3, the number of blocks, the relative disk block and the
/// physical memory address --- the second and third of which arrive here as
/// `start` and `size` --- and `WRITE-MCR-FILE` opens a file built on a base
/// version with the same shape, `3, 0, 0, BASE-VERSION-NUMBER`.  The
/// section's data is on the disk, not in the file, so one more word is
/// consumed and nothing else, which is what the boot PROM's
/// `PROCESS-MAIN-MEM-SECTION` does before it reads the blocks off the pack.
///
/// Sections do not consume the file exactly; [`Mcr::trailing_bytes`] reports
/// what is left.
pub fn parse(bytes: &[u8]) -> Result<Mcr, String> {
    let mut r = Reader { b: bytes, at: 0 };
    let mut mcr = Mcr::default();
    loop {
        let code = r.u32_pdp()?;
        let start = r.u32_pdp()?;
        let size = r.u32_pdp()? as usize;
        match code {
            1 => {
                mcr.imem_start = start;
                mcr.imem = (0..size).map(|_| r.insn()).collect::<Result<_, _>>()?;
            }
            2 => {
                if size != 0o4000 {
                    return Err(format!("dispatch memory is {size:o} words, expected 4000"));
                }
                mcr.dmem_start = start;
                mcr.dmem = (0..size).map(|_| r.u32_pdp()).collect::<Result<_, _>>()?;
            }
            // Main memory: the physical address word, and `size` --- the
            // relative disk block --- not a count of anything in the file.
            3 => {
                r.u32_pdp()?;
            }
            4 => {
                mcr.amem_start = start;
                mcr.amem = (0..size).map(|_| r.u32_pdp()).collect::<Result<_, _>>()?;
                break;
            }
            _ => return Err(format!("unknown section code {code:o} at offset {}", r.at - 12)),
        }
    }
    mcr.trailing_bytes = bytes.len() - r.at;
    Ok(mcr)
}
