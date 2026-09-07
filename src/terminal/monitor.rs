// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The monitor on the display board's video cable: what a CPT sees.
//!
//! The board scans out for real --- `tests/simpletv_netlist.rs` measures
//! it --- and four wires leave for the monitor: the differential video
//! pair off the 10212 at ECLVID 0F03, and `HSYNC OUT` and `VSYNC OUT` off
//! the 74S37s at NSYREG 0D09. This is the far end of those, as
//! [`crate::chaos::cable`] is the far end of the Chaosnet's.
//!
//! **The monitor supplies the video pair's pull-down.** MIT's
//! `necsip.drw` terminates 26 ECL nets on the board and deliberately not
//! these two: they go down a cable and are terminated at the far end, so
//! an open-emitter output with nobody there reads high-impedance rather
//! than low (discrepancy 50). Hence
//! [`Monitor::attach`], which puts the pull-down on before anything is
//! read.
//!
//! **It paints the whole sweep, borders and all.** A monitor has no idea
//! where the picture starts: it sweeps at the rate the sync tells it and
//! paints whatever the video line is doing, and the black edges are the
//! blanking. So the raster here is every dot of every line ---
//! [`DOTS_A_LINE`] across --- and not the 768 by 912 the sync program
//! blanks it down to. Nothing internal to the board is read, which is the
//! point: this is the cable and only the cable.

use crate::chip::Chip;
use crate::netlist::{NetId, Netlist};
use crate::part::Level;

use super::Frame;

/// Dots in a line, blanking counted in: 1024 at the 64 MHz dot clock is
/// the 16.000 us line `tests/simpletv_netlist.rs` measures.
pub const DOTS_A_LINE: usize = 1024;

/// Words of the raster a line takes, at 32 dots a word --- the frame
/// buffer's own layout, so that [`Frame`] draws a raster and a frame
/// buffer the same way.
pub const WORDS_A_LINE: usize = DOTS_A_LINE / 32;

/// Lines the raster has room for. The board's frame is 966; a monitor
/// given a sync it does not expect should run out of paper rather than
/// off the end of the array.
pub const MAX_LINES: usize = 1_200;

/// The four wires that leave the board for the monitor.
#[derive(Clone, Copy, Debug)]
pub struct Nets {
    /// `MECL VIDEO OUT`, the 10212's NOR output at ECLVID 0F03 pin 3.
    pub video: NetId,
    /// `-MECL VIDEO OUT`, its OR output on pin 2: the other half of the
    /// pair, which the monitor terminates and does not read.
    pub video_bar: NetId,
    pub hsync: NetId,
    pub vsync: NetId,
    /// The dot clock, which is not on the cable --- a monitor recovers
    /// its own timing from the sync --- and is read here as the sampling
    /// clock, because a dot is a dot and this monitor does not have to
    /// guess where the cells are.
    pub clock: NetId,
}

impl Nets {
    /// The nets on `n`, a display board.
    pub fn of(n: &Netlist) -> Option<Nets> {
        let net = |name: &str| n.by_name_id(name);
        Some(Nets {
            video: net("'MECL VIDEO OUT'")?,
            video_bar: net("'-MECL VIDEO OUT'")?,
            hsync: net("'HSYNC OUT'")?,
            vsync: net("'VSYNC OUT'")?,
            clock: net("'-64 MHZ CLK'")?,
        })
    }
}

/// The monitor: a raster it paints, and where the beam is.
pub struct Monitor {
    nets: Nets,
    /// The frame being painted.
    beam: Vec<u32>,
    /// The last frame painted end to end, which is what is shown.
    shown: Vec<u32>,
    /// Lines in [`Monitor::shown`].
    lines: usize,
    x: usize,
    y: usize,
    was: (Level, Level, Level),
    /// Frames finished since the monitor was plugged in.
    pub frames: u64,
}

impl Monitor {
    /// A monitor for the board `n`, if `n` is one with a video cable.
    pub fn of(n: &Netlist) -> Option<Monitor> {
        Nets::of(n).map(Monitor::new)
    }

    pub fn new(nets: Nets) -> Monitor {
        Monitor {
            nets,
            beam: vec![0; MAX_LINES * WORDS_A_LINE],
            shown: vec![0; MAX_LINES * WORDS_A_LINE],
            lines: 0,
            x: 0,
            y: 0,
            was: (Level::X, Level::X, Level::X),
            frames: 0,
        }
    }

    /// Terminates the cable, which is the monitor's end of it, and settles
    /// the board with the terminations on. Nothing can be read off this
    /// board before it: **all four wires float**, in two different
    /// directions.
    ///
    /// The video pair is ECL, open emitter, and takes a **pull-down**: the
    /// Thevenin dividers of `necsip.drw` terminate the other 26 ECL nets
    /// on the board and deliberately not these two, because they leave for
    /// the monitor. Without it the pair reads `Z` for its low.
    ///
    /// The sync pair is TTL and takes a **pull-up**: MIT drew the 74S37 at
    /// NSYREG 0D09 with SUDS's open-collector suffix, `74S37O`, so
    /// `HSYNC OUT` and `VSYNC OUT` can pull low and never drive high.
    /// Without it they read `Z` between pulses, no edge is a falling edge
    /// from a high, and a monitor watching for flyback waits for ever ---
    /// which is what happened the first time this was run.
    ///
    /// Discrepancy 50 has the ECL half.
    pub fn attach(&self, board: &mut Chip, now: u64) {
        board.pull_down(self.nets.video);
        board.pull_down(self.nets.video_bar);
        board.pull_up(self.nets.hsync);
        board.pull_up(self.nets.vsync);
        board.transition(now);
    }

    /// Reads the cable. Call it at every transition of the board: a dot is
    /// one falling edge of the dot clock, and the monitor has to be
    /// looking when it happens.
    pub fn sample(&mut self, board: &Chip) {
        let now =
            (board.net(self.nets.hsync), board.net(self.nets.vsync), board.net(self.nets.clock));
        let (hsync, vsync, clock) = now;
        let (was_h, was_v, was_c) = self.was;
        self.was = now;

        // A dot: the video line as it stands at the clock's falling edge.
        // The shift register moves on the other edge, so this reads each
        // cell settled.
        if was_c == Level::High && clock == Level::Low {
            if self.x < DOTS_A_LINE
                && self.y < MAX_LINES
                && video_is_lit(board.net(self.nets.video))
            {
                let bit = self.y * WORDS_A_LINE * 32 + self.x;
                self.beam[bit / 32] |= 1 << (bit % 32);
            }
            self.x += 1;
        }

        // Flyback: the beam goes back to the left on horizontal sync and
        // to the top on vertical.
        if was_h == Level::High && hsync == Level::Low {
            self.x = 0;
            self.y += 1;
        }
        if was_v == Level::High && vsync == Level::Low {
            self.shown.copy_from_slice(&self.beam);
            self.lines = self.y.min(MAX_LINES);
            self.beam.fill(0);
            self.x = 0;
            self.y = 0;
            self.frames += 1;
        }
    }

    /// Lines in the last frame painted end to end.
    pub fn lines(&self) -> usize {
        self.lines
    }

    /// The last whole frame, as a screen the terminal can serve.
    ///
    /// Black is the ground and a lit dot is white; the board's `MODE BOW`
    /// has already been applied on the way out, at the 10102 that takes
    /// `MECL BOW` against the shift register, so nothing here inverts.
    pub fn frame(&self) -> Frame<'_> {
        Frame {
            words: &self.shown,
            width: DOTS_A_LINE,
            height: self.lines,
            words_per_line: WORDS_A_LINE,
            black_on_white: false,
        }
    }

    /// How many dots are lit in the last whole frame, and on how many
    /// lines --- what a test asks when it wants to know whether there is a
    /// picture at all.
    pub fn lit(&self) -> (usize, usize) {
        let mut dots = 0;
        let mut lines = 0;
        for row in 0..self.lines {
            let at = row * WORDS_A_LINE;
            let n: usize =
                self.shown[at..at + WORDS_A_LINE].iter().map(|w| w.count_ones() as usize).sum();
            dots += n;
            lines += (n > 0) as usize;
        }
        (dots, lines)
    }

    /// The lit dots of one line, as their positions across the sweep.
    pub fn dots_on(&self, row: usize) -> Vec<usize> {
        (0..DOTS_A_LINE)
            .filter(|&x| {
                let bit = row * WORDS_A_LINE * 32 + x;
                self.shown[bit / 32] >> (bit % 32) & 1 != 0
            })
            .collect()
    }
}

/// Whether a level on `MECL VIDEO OUT` is a lit dot.
///
/// The 10212 at ECLVID 0F03 is an OR/NOR whose A gate takes `MECL BLANK`
/// and `-MECL VIDEO`; its NOR output is pin 3, `MECL VIDEO OUT`, and pin
/// 4, `MECL VIDEO`, the same gate on the part's second NOR pin. `NOR` of
/// two active-low inputs is high when neither is asserted, so a high is
/// video unblanked. `tests/monitor.rs` writes a word through the Xbus and
/// finds it in the sweep, which is what settles it.
fn video_is_lit(level: Level) -> bool {
    level == Level::High
}
