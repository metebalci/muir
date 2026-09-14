// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The sync program, run: the 4K x 8 program that "has overall control of
//! the TV", executed as the board executes it and laid out as a timeline
//! the model reads by time.
//!
//! Everything here is `cadrtv/lmtv.order`, `>Sync Program`, which is the
//! whole of what MIT wrote down about it:
//!
//! > The Sync Program executes an instruction every (32, 16, 8, 32) bits
//! > of video (indexed by Mode<1-0>) or roughly every 1/2 microsecond.
//! > This is also the rate at which the video buffer RAM cycles.
//!
//! and then the eight bits of an instruction ([`Instruction`]), and the
//! repeat feature:
//!
//! > The Sync Program is structured as a series of loops.  Each loop is
//! > executed a fixed number of times between 1 and 256.  A loop starts
//! > with a word containing the number of times it is to be executed.
//! > This word is never executed as an instruction, and does not cause a
//! > time delay ... The second to last instruction of a loop contains
//! > Special Function 2 or 3; one more instruction is executed
//! > (JUMP-XCT-NEXT) and then control returns to the first instruction of
//! > the loop, unless the repeat counter has counted out.
//! >
//! > If the repeat counter has counted out, then if the function was End
//! > of Program control returns to location 0 of the Sync Program (by a
//! > JUMP-XCT-NEXT), which is expected to contain a repeat count.  Control
//! > also gets to location 0 when the Xbus is reset.
//! >
//! > If the repeat counter has counted out and the function was End of
//! > Loop, then the location after next is taken as the repeat count of
//! > the next loop and the location after that is the first instruction
//! > of that loop.
//!
//! **Held to the boards.** A hand walk of MIT's own `cpt.prom` under these
//! rules gives nine loops of 1, 53, 8, 255, 255, 255, 131, 7 and 1 lines of
//! 32 instructions, 966 in all, which is the frame the netlist SIMPLE TV
//! is measured to make in `tests/simpletv_netlist.rs`: 966 lines of
//! 16.000 us, 912 of them unblanked and 896 fetching the picture.
//! `tests/sync_program.rs` holds this walk to those numbers, and to the
//! program `SET-TV-SPEED` loads and the one `COLOR:SETUP` loads. What
//! "roughly every 1/2 microsecond" is in each clock mode was read off the
//! netlist LISPM TV, [`INSTRUCTION_NS`]. How the mode register's sync bits
//! relate to the program's was read off the netlist SIMPLE TV: they are the
//! program's own bits 1 and 0, latched at the instruction boundary after
//! the instruction that carries them, and `-TVMA CLR` pulses as the
//! instruction carrying its special function completes ---
//! `the_sync_bits_the_mode_register_reads_are_the_programs`.

/// MIT's own sync PROM image, `cadrtv/cpt.prom`, "PROM ;for TV SYNC" of 5
/// May 1980: the program the board runs from power-on until the software
/// loads the RAM and selects it, [`super::SyncRam::enabled`]. 297 words for
/// the CPT monitor in clock mode 0.
pub const PROM_IMAGE: &str = include_str!("../../mit/cadrtv/cpt.prom");

/// [`PROM_IMAGE`] parsed, once.
pub fn prom() -> &'static [u8] {
    static PROM: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    PROM.get_or_init(|| crate::prom::parse_mit(PROM_IMAGE).expect("cadrtv/cpt.prom"))
}

/// Nanoseconds an instruction takes, by clock mode `MODE<1:0>`, **measured
/// on the netlist LISPM TV** with `cpt.prom` running: 32 instructions a
/// line, and `HSYNC OUT` 16.000 us apart in modes 0 and 1 and 20.000 us
/// apart in modes 2 and 3 --- 32 and 40 periods of the 64 MHz can
/// (`tests/lispmtv_netlist.rs`,
/// `an_instruction_of_the_sync_program_in_each_clock_mode`). Mode 0 is
/// the SIMPLE TV's too, measured there the same way.
pub const INSTRUCTION_NS: [u64; 4] = [500, 500, 625, 625];

/// The most instructions a program is walked for before it is taken to
/// make no frame: `lmtv.order`'s "Maximum execution time is 1356 loops of
/// 256 iterations each, or 349440 cycles, or about .2 seconds. Maybe 3
/// times this, actually?" --- three times that.
pub const MOST_INSTRUCTIONS: u64 = 3 * 349_440;

/// One word of the program, `lmtv.order`'s eight bits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Instruction(pub u8);

/// Bits 5-4, "Video Buffer Cycle Type".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cycle {
    /// 00, "Processor Cycle (Buffer available to processor if it has a
    /// pending reference to the video buffer.)"
    Processor,
    /// 01, "Refresh Cycle (refresh address comes from an automatic counter)"
    Refresh,
    /// 10, "Normal Video Cycle, 64-bit shift register gets Buffer[TVMA] and
    /// TVMA gets TVMA + 1."
    Video,
    /// 11, "End-of-line Video Cycle, 64-bit shift register gets
    /// Buffer[TVMA] and TVMA gets TVMA + Vertical Spacing"
    EndOfLine,
}

/// Bits 7-6, "Special Function".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Special {
    /// 00, "No special function"
    None,
    /// 01, "TVMA CLR (i.e. start next field)" --- and the vertical flag's
    /// preset: "This is set by TVMA CLR, not by the start of Vertical
    /// Sync."
    TvmaClr,
    /// 10, "End of Loop (EOL)"
    EndOfLoop,
    /// 11, "End of Program (EOF)"
    EndOfProgram,
}

impl Instruction {
    /// Bit 0, "Horizontal Sync to TV".
    pub fn hsync(self) -> bool {
        self.0 & 1 != 0
    }
    /// Bit 1, "Vertical Sync to TV".
    pub fn vsync(self) -> bool {
        self.0 & 2 != 0
    }
    /// Bit 2, "Composite Sync (used if making composite video output)".
    pub fn composite(self) -> bool {
        self.0 & 4 != 0
    }
    /// Bit 3, "Blank".
    pub fn blank(self) -> bool {
        self.0 & 8 != 0
    }
    pub fn cycle(self) -> Cycle {
        match (self.0 >> 4) & 3 {
            0 => Cycle::Processor,
            1 => Cycle::Refresh,
            2 => Cycle::Video,
            _ => Cycle::EndOfLine,
        }
    }
    pub fn special(self) -> Special {
        match (self.0 >> 6) & 3 {
            0 => Special::None,
            1 => Special::TvmaClr,
            2 => Special::EndOfLoop,
            _ => Special::EndOfProgram,
        }
    }
}

/// One iteration of one loop of the program: `lmtv.order`'s "the loops
/// need not correspond to raster lines, although they usually will", and
/// in every program MIT wrote they do --- 32 instructions of `cpt.prom`
/// and of `SET-TV-SPEED`'s program, 102 of `COLOR:SYNC`'s --- so this is
/// a line of the raster, counted as the program counts it rather than by
/// its sync pulses, which NTSC doubles up in the equalizing lines.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Line {
    /// Where in the period the line starts.
    pub at_ns: u64,
    pub instructions: u32,
    /// Video cycles fetching the buffer, `Cycle::Video` and
    /// `Cycle::EndOfLine`, each 64 bits of it.
    pub video_cycles: u32,
    /// Blanked from end to end.
    pub blanked: bool,
}

/// A program run through once, from location 0 to its End of Program:
/// how long that takes, when the sync bits change along the way, when
/// `TVMA CLR` fires, and the lines it makes. Periodic: the program returns
/// to location 0 and does it again.
#[derive(Clone, Debug)]
pub struct Timeline {
    /// One run of the program, in nanoseconds.
    pub period_ns: u64,
    /// Instructions executed in one run.
    pub instructions: u64,
    /// The sync bits as the mode register reads them, `(offset, hsync,
    /// vsync)`, at every instant one of them changes; the first at 0. An
    /// instruction's bits are latched at the boundary after it, which is
    /// where these are dated.
    changes: Vec<(u64, bool, bool)>,
    /// Offsets at which `-TVMA CLR` fires: the instant the instruction
    /// carrying it completes.
    pub tvma_clr: Vec<u64>,
    pub lines: Vec<Line>,
}

impl Timeline {
    /// `program` run in clock mode `mode` (`MODE<1:0>`; the rest of the
    /// word is ignored). `None` for a program that makes no frame: one
    /// that runs off the end of its memory without an End of Loop, or
    /// past [`MOST_INSTRUCTIONS`] without an End of Program --- the RAM
    /// before the software has loaded it, or half way through loading.
    pub fn of(program: &[u8], mode: u32) -> Option<Timeline> {
        let step = INSTRUCTION_NS[(mode & 3) as usize];
        let mut t: u64 = 0;
        let mut executed: u64 = 0;
        let mut changes: Vec<(u64, bool, bool)> = Vec::new();
        let mut tvma_clr = Vec::new();
        let mut lines: Vec<Line> = Vec::new();
        // The bits as latched: what the register shows now.
        let (mut hsync, mut vsync) = (false, false);
        let mut pc = 0usize;
        loop {
            // The loop's repeat count: 1 to 256, and a zero is the
            // counter's 256, the count being eight bits.
            let count = *program.get(pc)? as u32;
            let iterations = if count == 0 { 256 } else { count };
            let first = pc + 1;
            let mut ended = Special::None;
            let mut after = first;
            for _ in 0..iterations {
                let mut p = first;
                let mut line = Line { at_ns: t, instructions: 0, video_cycles: 0, blanked: true };
                // The loop's instructions, the one that ends it, and the
                // one executed after that.
                let mut one_more = false;
                loop {
                    let word = *program.get(p)?;
                    let insn = Instruction(word);
                    executed += 1;
                    if executed > MOST_INSTRUCTIONS {
                        return None;
                    }
                    t += step;
                    // The bits land at the boundary after the instruction.
                    let (h, v) = (insn.hsync(), insn.vsync());
                    if (h, v) != (hsync, vsync) || changes.is_empty() {
                        changes.push((t, h, v));
                    }
                    (hsync, vsync) = (h, v);
                    line.instructions += 1;
                    if matches!(insn.cycle(), Cycle::Video | Cycle::EndOfLine) {
                        line.video_cycles += 1;
                    }
                    if !insn.blank() {
                        line.blanked = false;
                    }
                    match insn.special() {
                        Special::TvmaClr => tvma_clr.push(t),
                        Special::None => {}
                        s @ (Special::EndOfLoop | Special::EndOfProgram) if !one_more => {
                            ended = s;
                            one_more = true;
                            // The next instruction goes too, and is where
                            // the loop is left from.
                            after = p + 1;
                        }
                        // A special function on the one-more instruction
                        // itself is not a second end: the loop is already
                        // ending, and this is the instruction after it.
                        _ => {}
                    }
                    if one_more && p == after {
                        break;
                    }
                    p += 1;
                }
                lines.push(line);
            }
            match ended {
                Special::EndOfProgram => break,
                Special::EndOfLoop => pc = after + 1,
                // Unreachable: a loop leaves only through one of the two.
                _ => return None,
            }
        }
        // The first change is dated where the first instruction lands, and
        // what the register shows before it is what it showed at the end
        // of the previous run --- the program being periodic, that is the
        // last change's bits, which `sync_at` reads.
        Some(Timeline { period_ns: t, instructions: executed, changes, tvma_clr, lines })
    }

    /// The sync bits `(hsync, vsync)` the mode register shows at `offset`
    /// into the period.
    pub fn sync_at(&self, offset: u64) -> (bool, bool) {
        let offset = offset % self.period_ns;
        // The change at or before the offset, else the period's last.
        match self.changes.partition_point(|c| c.0 <= offset) {
            0 => self.changes.last().map_or((false, false), |c| (c.1, c.2)),
            k => (self.changes[k - 1].1, self.changes[k - 1].2),
        }
    }

    /// How many times `-TVMA CLR` has fired from the program's start to
    /// `since_start` inclusive.
    pub fn tvma_clrs_by(&self, since_start: u64) -> u64 {
        let runs = since_start / self.period_ns;
        let into = since_start % self.period_ns;
        runs * self.tvma_clr.len() as u64 + self.tvma_clr.partition_point(|&c| c <= into) as u64
    }
}
