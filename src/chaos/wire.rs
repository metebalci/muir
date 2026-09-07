// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The cable's coding: AIM-628 §2.5, "Upright Biphase NRZI".
//!
//! "Each bit cell, which is approximately 250 nanoseconds long, begins
//! with a transition in state, from high to low or from low to high.
//! This transition marks the beginning of a bit cell and provides
//! self-clocking. 3/4 of the way through the bit cell, the state of the
//! cable is sampled; high represents a 1 and low represents a 0. If the
//! bit being represented is the same as the previous bit, there will be
//! one transition at the beginning of the bit cell and a second in the
//! middle of the bit cell. If the bit being represented is the opposite
//! of the previous bit, there will be no transition in the middle of the
//! bit cell since the clock transition will have set the cable to the
//! desired state."
//!
//! "If the ether remains low for more than about two bit cells, it is
//! considered to be not-busy. This condition marks the end of a packet."
//! "If the ether remains high for about two bit cells, this is an 'abort
//! signal'."

/// The bit cell: 250 ns, the 4 MHz `FCLK/2^` of the board.
pub const CELL_NS: u64 = 250;
/// Where in the cell the level is read. AIM-628 says "3/4 of the way
/// through", which of a 250 ns cell would be 187; the board's own chain
/// puts it at 175, and the board is followed --- see [`LOCKOUT_NS`] for the
/// chain and where its tap choices come from. `SAMPLE` is the TD25NC at
/// LMDETC 0A11's 15 ns tap: 160 ns of chain to reach that line, then 15
/// along it. It is a *sibling* of `LOCKOUT END`, not a stage before or
/// after it --- both hang off 0A11's input.
pub const SAMPLE_NS: u64 = 175;
/// How long after a cell's opening edge a further edge is the mid-cell
/// one and not the next cell's: the board's `LOCKOUT`, the 74S74 at LMDETC
/// 0E08, preset by `-EDGE` and clocked clear by `LOCKOUT END`. That is the
/// edge through three delay lines: `GENCLK`, preset by the same edge, into
/// the TD100NC at LMDETC 0B04, whose `SDLYD` feeds the TD100NC at 0B11,
/// whose `SDLYD2` feeds the TD25NC at 0A11, whose `LOCKOUT END` clocks the
/// flip-flop --- 100 + 60 + 10 ns, which falls between the mid-cell
/// transition at 125 and the next cell at 250.
///
/// **The chain forks at 0A11**, which takes `SDLYD2` in and drives two taps
/// off it: `LOCKOUT END` at 10 ns and [`SAMPLE_NS`]'s `SAMPLE` at 15. They
/// are siblings, both 160 ns into the chain; neither is downstream of the
/// other, and one cannot be derived from the other by adding a stage.
///
/// Those three tap choices are not on the drawing. The `NC` bodies carry
/// two wire-wrap posts the DIP has not got, and `cadrio/iob.eco`'s
/// consolidation of 2/18/81 straps a tap to each: MIT's own comments give
/// them as 100 ns, 60 ns and 10 ns. `src/chip.rs` lists the four straps and
/// how their post numbers map onto pins.
pub const LOCKOUT_NS: u64 = 170;
/// A line quiet this long is idle: "more than about two bit cells".
pub const IDLE_NS: u64 = 2 * CELL_NS + CELL_NS / 2;

/// The level changes that put `bits` on a line that is idle low, as
/// `(offset_ns, level)` from the first edge. The line is low again
/// after the last, which is why [`crate::chaos::packet::frame`] ends in
/// a zero.
pub fn encode(bits: &[bool]) -> Vec<(u64, bool)> {
    let mut out = Vec::with_capacity(bits.len() * 2);
    let mut level = false;
    let mut prev = false;
    for (k, &bit) in bits.iter().enumerate() {
        let start = k as u64 * CELL_NS;
        level = !level;
        out.push((start, level));
        if bit == prev {
            level = !level;
            out.push((start + CELL_NS / 2, level));
        }
        prev = bit;
    }
    out
}

/// A receiver on the line: fed each edge as it happens and asked what is
/// due, it delivers a packet's bits when the line has gone idle. It does
/// what the board's detector does with its lockout and its sample delay,
/// which is what makes the two agree on which edges are cells.
#[derive(Clone, Debug, Default)]
pub struct Decoder {
    level: bool,
    /// The opening edge of the current cell, if in a packet.
    cell: Option<u64>,
    /// When the current cell's sample is due, if not taken yet.
    sample: Option<u64>,
    bits: Vec<bool>,
    /// The last edge seen.
    last_edge: Option<u64>,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder::default()
    }

    /// Whether a packet is in progress: the line has had an edge and has
    /// not yet been idle for [`IDLE_NS`].
    pub fn busy(&self) -> bool {
        self.cell.is_some()
    }

    /// The line changed to `level` at `t`.
    pub fn edge(&mut self, t: u64, level: bool) {
        if level == self.level {
            return;
        }
        self.level = level;
        self.last_edge = Some(t);
        match self.cell {
            Some(start) if t < start + LOCKOUT_NS => {
                // The mid-cell transition: locked out.
            }
            _ => {
                self.cell = Some(t);
                self.sample = Some(t + SAMPLE_NS);
            }
        }
    }

    /// When the decoder next needs to be told the time: the sample due,
    /// or the moment the line will have been idle long enough.
    pub fn next_due(&self) -> Option<u64> {
        let idle = self.last_edge.filter(|_| self.cell.is_some()).map(|e| e + IDLE_NS);
        match (self.sample, idle) {
            (Some(s), Some(i)) => Some(s.min(i)),
            (s, i) => s.or(i),
        }
    }

    /// The time is `t`: takes a sample that is due, and ends the packet
    /// if the line has been idle since its last edge. Returns the
    /// packet's bits when it ends.
    pub fn at(&mut self, t: u64) -> Option<Vec<bool>> {
        if let Some(s) = self.sample
            && t >= s
        {
            self.bits.push(self.level);
            self.sample = None;
        }
        if let (Some(_), Some(e)) = (self.cell, self.last_edge)
            && t >= e + IDLE_NS
            && !self.level
        {
            self.cell = None;
            self.sample = None;
            return Some(std::mem::take(&mut self.bits));
        }
        None
    }
}

/// Runs a whole waveform through a [`Decoder`]: the packets it delivers.
pub fn decode(changes: &[(u64, bool)]) -> Vec<Vec<bool>> {
    let mut d = Decoder::new();
    let mut out = Vec::new();
    let mut t = 0;
    for &(at, level) in changes {
        while let Some(due) = d.next_due()
            && due <= at
        {
            t = due;
            if let Some(p) = d.at(t) {
                out.push(p);
            }
        }
        t = t.max(at);
        d.edge(at, level);
    }
    while let Some(due) = d.next_due() {
        t = due.max(t);
        if let Some(p) = d.at(t) {
            out.push(p);
        }
    }
    out
}
