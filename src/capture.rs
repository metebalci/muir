// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Recording the display as a GIF.
//!
//! A [`Recorder`] samples the screen and keeps a frame only when something
//! changed, as the rectangle that changed, laid over the frame before it,
//! and timed by the machine's own clock so the playback runs at the
//! machine's speed --- ten milliseconds of machine time a centisecond of
//! the GIF, its smallest unit.  Two colours and LZW, so it stays small
//! while the screen stays still: a blinking cursor is a few dozen bytes a
//! blink, and an idle screen one small frame a second, the clock ticking.
//!
//! The lashup's two machines record as one: [`Recorder::pair`] lays the
//! debugger's screen at the left of the canvas and the debuggee's at the
//! right, a white rule between them.  One canvas because the two are one
//! clock --- [`crate::lashup::Lashup`] steps neither past the other's
//! promise --- so a frame is one instant on both screens.  Two recordings
//! could not promise that: a frame's length is written in centiseconds,
//! rounded down, so two files of the same run drift apart by some
//! milliseconds a frame and nothing in either says when a frame was taken.
//!
//! With the clocks shown, the canvas is a line taller than the screen and
//! two times are drawn on that line below it, each `hh:mm:ss`, so they
//! hide no part of the display: at the left the machine's own, from its
//! power-on, and at the right the wall clock, the local time of day where
//! the run was made.  Each changes once its second ticks, a small frame of
//! its own.

use crate::simpletv::{HEIGHT, SimpleTv, WIDTH};

/// The height of the clock line below the screen, when it is shown: two
/// rows of margin above and below a `GLYPH_H`-tall digit doubled.
pub const TIME_H: usize = 2 * GLYPH_H + 8;

/// The rule between the two screens of a lashup recording, in columns: two
/// black screens side by side would meet with nothing to say where one
/// ends.  It is drawn down the screens and not through the clock line,
/// which belongs to the recording rather than to either machine.
pub const PAIR_RULE: usize = 2;

/// A recorder for the display, growing a GIF frame by frame.
pub struct Recorder {
    /// Whether the machine's clock and the wall clock are drawn on a line
    /// below the screen.
    show_time: bool,
    /// The canvas width: one screen, or the lashup's two and the rule
    /// between them.
    width: usize,
    /// The canvas height: the screen, and the clock line if shown.
    height: usize,
    /// The last canvas sampled, a byte a pixel, 1 white.
    prev: Option<Vec<u8>>,
    /// The instant the last frame was taken, and of the last sample, for
    /// the last frame's length.
    last_at: u64,
    sampled_at: u64,
    /// The machine second and the wall clock's second the line last
    /// showed, so it is redrawn only when one ticks.
    shown_second: Option<u64>,
    shown_wall_second: Option<u64>,
    frames: Vec<Frame>,
    samples: u64,
}

struct Frame {
    /// Nanoseconds this frame stays up.
    delay_ns: u64,
    left: u16,
    top: u16,
    width: u16,
    height: u16,
    /// LZW-coded pixels of the rectangle, row by row.
    data: Vec<u8>,
}

impl Recorder {
    /// A recorder for a screen `WIDTH` by `HEIGHT`, with the machine's
    /// clock and the wall clock on a line below it if `show_time`.
    pub fn new(show_time: bool) -> Recorder {
        Recorder::of(WIDTH, show_time)
    }

    /// A recorder for the lashup's two screens on one canvas: the
    /// debugger's at the left, the debuggee's at the right, the rule
    /// between them.  Sampled with [`Recorder::sample_pair`], and timed by
    /// the debugger's clock, which is the debuggee's to within one
    /// generator cycle.
    pub fn pair(show_time: bool) -> Recorder {
        Recorder::of(2 * WIDTH + PAIR_RULE, show_time)
    }

    fn of(width: usize, show_time: bool) -> Recorder {
        Recorder {
            show_time,
            width,
            height: HEIGHT + if show_time { TIME_H } else { 0 },
            prev: None,
            last_at: 0,
            sampled_at: 0,
            shown_second: None,
            shown_wall_second: None,
            frames: Vec::new(),
            samples: 0,
        }
    }

    pub fn frames(&self) -> usize {
        self.frames.len()
    }

    pub fn samples(&self) -> u64 {
        self.samples
    }

    /// Whether a sample at `ns` of the machine's time and `wall_ns` on the
    /// wall clock would add a frame: the screen may have changed, or a
    /// clock has ticked.  Cheap next to a sample, so a caller can skip the
    /// pixel copy when nothing is due.
    pub fn due(&self, ns: u64, wall_ns: u64) -> bool {
        self.prev.is_none()
            || (self.show_time
                && (self.shown_second != Some(ns / 1_000_000_000)
                    || self.shown_wall_second != Some(wall_ns / 1_000_000_000)))
    }

    /// The screen at `ns` of the machine's time, the wall clock reading
    /// `wall_ns` since midnight: a frame if the canvas differs from the
    /// last.
    pub fn sample(&mut self, tv: &SimpleTv, ns: u64, wall_ns: u64) {
        self.screens(&[tv], ns, wall_ns);
    }

    /// Both screens of the lashup at one instant, the debugger's at the
    /// left of the canvas and the debuggee's at the right, on a recorder
    /// made by [`Recorder::pair`].  `ns` is the debugger's clock and times
    /// the frame for both.
    pub fn sample_pair(&mut self, debugger: &SimpleTv, debuggee: &SimpleTv, ns: u64, wall_ns: u64) {
        self.screens(&[debugger, debuggee], ns, wall_ns);
    }

    /// The canvas as `screens` shows it, left to right: a frame if it
    /// differs from the last.
    fn screens(&mut self, screens: &[&SimpleTv], ns: u64, wall_ns: u64) {
        assert_eq!(
            self.width,
            screens.len() * WIDTH + (screens.len() - 1) * PAIR_RULE,
            "{} screens on a canvas {} wide",
            screens.len(),
            self.width
        );
        self.samples += 1;
        let mut cur = vec![0u8; self.width * self.height];
        for (k, tv) in screens.iter().enumerate() {
            let at = k * (WIDTH + PAIR_RULE);
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    cur[y * self.width + at + x] = tv.shows_white(x, y) as u8;
                }
            }
        }
        // The rule between one screen and the next, down the screens and
        // not through the clock line.
        for k in 1..screens.len() {
            for y in 0..HEIGHT {
                for x in 0..PAIR_RULE {
                    cur[y * self.width + k * (WIDTH + PAIR_RULE) - PAIR_RULE + x] = 1;
                }
            }
        }
        if self.show_time {
            self.shown_second = Some(ns / 1_000_000_000);
            self.shown_wall_second = Some(wall_ns / 1_000_000_000);
            draw_time(&mut cur, self.width, ns, TIME_MARGIN);
            draw_time(&mut cur, self.width, wall_ns, self.width - TIME_MARGIN - TIME_W);
        }
        let width = self.width;
        let rect = match &self.prev {
            None => Some((0, 0, width, self.height)),
            Some(prev) => {
                let (mut x0, mut y0, mut x1, mut y1) = (width, self.height, 0, 0);
                for y in 0..self.height {
                    let (a, b) =
                        (&prev[y * width..(y + 1) * width], &cur[y * width..(y + 1) * width]);
                    if a == b {
                        continue;
                    }
                    let first = (0..width).find(|&x| a[x] != b[x]).unwrap();
                    let last = (0..width).rev().find(|&x| a[x] != b[x]).unwrap();
                    x0 = x0.min(first);
                    x1 = x1.max(last + 1);
                    y0 = y0.min(y);
                    y1 = y1.max(y + 1);
                }
                (x0 < x1).then(|| (x0, y0, x1 - x0, y1 - y0))
            }
        };
        if let Some((x, y, w, h)) = rect {
            if let Some(last) = self.frames.last_mut() {
                last.delay_ns = ns - self.last_at;
            }
            let mut pixels = Vec::with_capacity(w * h);
            for row in y..y + h {
                pixels.extend_from_slice(&cur[row * self.width + x..row * self.width + x + w]);
            }
            self.frames.push(Frame {
                delay_ns: 0,
                left: x as u16,
                top: y as u16,
                width: w as u16,
                height: h as u16,
                data: lzw(&pixels),
            });
            self.last_at = ns;
            self.prev = Some(cur);
        }
        self.sampled_at = ns;
    }

    /// The recording as a GIF.
    pub fn gif(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"GIF89a");
        out.extend_from_slice(&(self.width as u16).to_le_bytes());
        out.extend_from_slice(&(self.height as u16).to_le_bytes());
        // A global table of two colours, one bit of colour resolution.
        out.extend_from_slice(&[0x80, 0, 0]);
        out.extend_from_slice(&[0, 0, 0, 255, 255, 255]);
        // Loop for ever.
        out.extend_from_slice(b"\x21\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00");
        for (k, f) in self.frames.iter().enumerate() {
            let delay_ns = if k + 1 == self.frames.len() {
                self.sampled_at - self.last_at + 1_000_000_000
            } else {
                f.delay_ns
            };
            let cs = (delay_ns / 10_000_000).clamp(2, u16::MAX as u64) as u16;
            // Graphic control: leave the frame in place, no transparency.
            out.extend_from_slice(&[0x21, 0xf9, 4, 0x04]);
            out.extend_from_slice(&cs.to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.push(0x2c);
            for v in [f.left, f.top, f.width, f.height] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.push(0);
            out.push(2);
            for block in f.data.chunks(255) {
                out.push(block.len() as u8);
                out.extend_from_slice(block);
            }
            out.push(0);
        }
        out.push(0x3b);
        out
    }
}

// --- The wall clock ---------------------------------------------------------

/// The wall clock, broken down: local date and time.  Here because the
/// clock line is drawn from it; `muir` names a checkpoint by the same
/// clock.
pub struct LocalTime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub nanos: u32,
}

/// The wall clock, local, from the C library's `localtime_r`, which alone
/// knows the machine's time zone, declared here rather than pulled in
/// through a crate for one call.  Read from the macOS SDK's `_time.h`:
/// `struct tm` is nine `int`s, `tm_sec` first, `tm_hour` third, `tm_mday`
/// fourth, `tm_mon` fifth from January as 0 and `tm_year` sixth from 1900,
/// then the offset from UTC as a `long` and the zone's name; `time_t` is a
/// `long`.  glibc and musl lay it out the same way, so the one declaration
/// serves the 64-bit Unix targets this builds for.  Should the call fail,
/// UTC.
pub fn local_time() -> LocalTime {
    use std::ffi::{c_char, c_int, c_long};
    #[repr(C)]
    struct Tm {
        sec: c_int,
        min: c_int,
        hour: c_int,
        mday: c_int,
        mon: c_int,
        year: c_int,
        wday: c_int,
        yday: c_int,
        isdst: c_int,
        gmtoff: c_long,
        zone: *const c_char,
    }
    unsafe extern "C" {
        fn localtime_r(t: *const c_long, tm: *mut Tm) -> *mut Tm;
    }
    let now =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let t = now.as_secs() as c_long;
    let mut tm = Tm {
        sec: 0,
        min: 0,
        hour: 0,
        mday: 0,
        mon: 0,
        year: 0,
        wday: 0,
        yday: 0,
        isdst: 0,
        gmtoff: 0,
        zone: std::ptr::null(),
    };
    // SAFETY: `localtime_r` reads `t` and writes the `struct tm` it is
    // handed, which `Tm` lays out as the C library expects; both live on
    // this stack for the call.
    let local = unsafe { !localtime_r(&t, &mut tm).is_null() };
    let nanos = now.subsec_nanos();
    if local {
        return LocalTime {
            year: tm.year as i64 + 1900,
            month: tm.mon as u32 + 1,
            day: tm.mday as u32,
            hour: tm.hour as u32,
            minute: tm.min as u32,
            second: tm.sec as u32,
            nanos,
        };
    }
    let secs = now.as_secs();
    // The civil date of a day count from 1970, after Howard Hinnant's
    // "chrono-Compatible Low-Level Date Algorithms", days_from_civil
    // inverted.
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + (month <= 2) as i64;
    let of_day = (secs % 86_400) as u32;
    LocalTime {
        year,
        month,
        day,
        hour: of_day / 3600,
        minute: of_day / 60 % 60,
        second: of_day % 60,
        nanos,
    }
}

/// The wall clock for a recording: the local time of day as nanoseconds
/// since midnight.
pub fn wall_clock() -> u64 {
    let t = local_time();
    (t.hour * 3600 + t.minute * 60 + t.second) as u64 * 1_000_000_000 + t.nanos as u64
}

// --- The clock line ---------------------------------------------------------

/// The digit font's cell: 5 wide, 7 tall, doubled when drawn.
const GLYPH_W: usize = 5;
const GLYPH_H: usize = 7;
const SCALE: usize = 2;

/// A clock's width on the line: eight cells, `hh:mm:ss`, each a glyph and
/// a column of gap, less the gap after the last.
const TIME_W: usize = 8 * (GLYPH_W + 1) * SCALE - SCALE;
/// The margin a clock keeps from its end of the line: the machine's from
/// the left, the wall clock from the right.
const TIME_MARGIN: usize = 6;

/// Draws `hh:mm:ss` of `ns` into the clock line at the bottom of `cur`,
/// a canvas `width` wide, from column `x`, white on the black line.  Two
/// digits of hours: the machine's past ninety-nine wrap rather than
/// stopping the recording; the wall clock's never get there.
fn draw_time(cur: &mut [u8], width: usize, ns: u64, mut x: usize) {
    let s = ns / 1_000_000_000;
    let (hh, mm, ss) = (s / 3600, s / 60 % 60, s % 60);
    let text = [
        (hh / 10 % 10) as u8,
        (hh % 10) as u8,
        10,
        (mm / 10) as u8,
        (mm % 10) as u8,
        10,
        (ss / 10) as u8,
        (ss % 10) as u8,
    ];
    let top = HEIGHT + 4;
    for &glyph in &text {
        let bits = GLYPHS[glyph as usize];
        for (row, &pattern) in bits.iter().enumerate() {
            for col in 0..GLYPH_W {
                if pattern & (1 << (GLYPH_W - 1 - col)) == 0 {
                    continue;
                }
                for dy in 0..SCALE {
                    for dx in 0..SCALE {
                        let px = x + col * SCALE + dx;
                        let py = top + row * SCALE + dy;
                        cur[py * width + px] = 1;
                    }
                }
            }
        }
        // Every glyph a cell wide and a column of gap, the colon its own
        // cell so it does not run into the next digit.
        x += (GLYPH_W + 1) * SCALE;
    }
}

/// The 5-by-7 bitmaps for `0`-`9` and, at index 10, `:`. Each row is five
/// bits, the leftmost the high one.
const GLYPHS: [[u8; GLYPH_H]; 11] = [
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110], // 0
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110], // 1
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111], // 2
    [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110], // 3
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010], // 4
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110], // 5
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110], // 6
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000], // 7
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110], // 8
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100], // 9
    [0b00000, 0b00100, 0b00100, 0b00000, 0b00100, 0b00100, 0b00000], // :
];

// --- GIF's LZW --------------------------------------------------------------

/// GIF's LZW over two-valued pixels: a minimum code size of 2, so the
/// clear code is 4 and the end code 5, codes from 6, the code width from
/// 3 bits to 12, and a clear when the table is full.
fn lzw(pixels: &[u8]) -> Vec<u8> {
    const CLEAR: u16 = 4;
    const END: u16 = 5;
    const NONE: u16 = u16::MAX;
    let mut bits = BitWriter::default();
    let mut table = vec![[NONE; 2]; 4096];
    let mut next = 6u16;
    let mut width = 3;
    bits.put(CLEAR, width);
    let Some((&first, rest)) = pixels.split_first() else {
        bits.put(END, width);
        return bits.finish();
    };
    let mut cur = first as u16;
    for &p in rest {
        let child = table[cur as usize][p as usize];
        if child != NONE {
            cur = child;
            continue;
        }
        bits.put(cur, width);
        if next < 4096 {
            table[cur as usize][p as usize] = next;
            next += 1;
            if next > (1 << width) && width < 12 {
                width += 1;
            }
        } else {
            bits.put(CLEAR, width);
            table.iter_mut().for_each(|t| *t = [NONE; 2]);
            next = 6;
            width = 3;
        }
        cur = p as u16;
    }
    bits.put(cur, width);
    bits.put(END, width);
    bits.finish()
}

#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, code: u16, width: u32) {
        self.acc |= (code as u32) << self.n;
        self.n += width;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}
