// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The display recorder: its frames read back by a GIF reader of the
//! tests' own, its clock line, the lashup's two screens on one canvas, and
//! the colour screen's recording with the map it was taken through.
//! Nothing here needs `vendor/`.

mod support;

use muir::capture::{ColorRecorder, PAIR_RULE, Recorder, TIME_H};
use muir::machine::Machine;
use muir::tv::{
    CHANNELS, COLOR_HEIGHT, COLOR_WIDTH, COLOR_WORDS_PER_LINE, COLORS, HEIGHT, Tv, WIDTH,
};

use crate::support::{Frame, decode_gif, decode_gif_fully};

/// The canvas a pair recording is laid on.
const PAIR_W: usize = 2 * WIDTH + PAIR_RULE;

/// **The recording decodes to the screens it was taken from.** Two
/// samples of a screen with a few words lit, the second changed in one
/// place; the GIF's two frames, decoded here by a plain LZW reader, are
/// the whole screen and the changed rectangle, pixel for pixel.
#[test]
fn the_recording_decodes_to_the_screens() {
    let mut tv = Tv::default();
    tv.write_buffer(3, 0xa5a5_a5a5);
    tv.write_buffer(24 * 100 + 7, 0x0000_ffff);
    let mut rec = Recorder::new(false);
    rec.sample(&tv, 1_000, 0);
    tv.write_buffer(24 * 100 + 7, 0xffff_0000);
    tv.write_buffer(24 * 101 + 8, 1);
    rec.sample(&tv, 501_000_000, 0);
    rec.sample(&tv, 1_001_000_000, 0);
    assert_eq!(rec.frames(), 2, "a frame for the first screen and one for the change");
    let gif = rec.gif();
    let frames = decode_gif(&gif);
    assert_eq!(frames.len(), 2);
    let (rect, px) = &frames[0];
    assert_eq!(*rect, (0, 0, WIDTH, HEIGHT));
    for (x, &got) in px.iter().enumerate().take(WIDTH) {
        let want =
            ((96..128).contains(&x) && (0xa5a5_a5a5u32 >> (x.wrapping_sub(96))) & 1 != 0) as u8;
        assert_eq!(got, want, "row 0, x {x}");
    }
    let (rect, px) = &frames[1];
    // The change spans word 7 of row 100 to bit 0 of word 8 of row 101:
    // 33 pixels wide, two rows.
    assert_eq!(*rect, (224, 100, 33, 2), "the changed rectangle");
    let mut want = vec![0u8; 66];
    want[16..32].fill(1);
    want[65] = 1;
    assert_eq!(px, &want, "the rectangle's pixels, row by row");
    assert!(gif.len() < 20_000, "a blank screen and a small change: {} bytes", gif.len());
}

/// The clock line of a decoded frame: whether each of its columns holds a
/// white pixel below the screen.
fn lit_columns((rect, px): &Frame) -> Vec<bool> {
    let (_, top, w, h) = *rect;
    assert!(top + h > HEIGHT, "the frame reaches the clock line");
    (0..WIDTH).map(|x| (HEIGHT.max(top)..top + h).any(|y| px[(y - top) * w + x] == 1)).collect()
}

/// **The machine's clock is at the left of the line and the wall clock at
/// the right.** A first sample at power-on and midnight: the line's lit
/// columns fall in two groups, one in each half, the same pattern twice,
/// both reading `00:00:00`.
#[test]
fn the_clocks_sit_at_either_end_of_the_line() {
    let tv = Tv::default();
    let mut rec = Recorder::new(true);
    rec.sample(&tv, 0, 0);
    let frames = decode_gif(&rec.gif());
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].0, (0, 0, WIDTH, HEIGHT + TIME_H));
    let lit = lit_columns(&frames[0]);
    let first = lit.iter().position(|&l| l).expect("the machine's clock");
    let last = lit.iter().rposition(|&l| l).expect("the wall clock");
    assert!(first < WIDTH / 2 && last >= WIDTH / 2, "one at each end: {first}..={last}");
    let left = &lit[first..WIDTH / 2];
    let right = &lit[WIDTH / 2..=last];
    let width = left.iter().rposition(|&l| l).unwrap() + 1;
    let start = right.iter().position(|&l| l).unwrap();
    assert_eq!(left[..width], right[start..], "both read 00:00:00");
    assert!(width < WIDTH / 4, "each clock well within its half: {width} wide");
}

/// **Each clock ticks a frame of its own.** With the screen still, a
/// machine second makes a frame of the left clock alone, a wall-clock
/// second one of the right clock alone, and a sample with neither ticked
/// makes none.
#[test]
fn each_clock_ticks_a_frame_of_its_own() {
    const S: u64 = 1_000_000_000;
    let tv = Tv::default();
    let mut rec = Recorder::new(true);
    rec.sample(&tv, 0, 0);
    assert!(!rec.due(S / 2, S / 2));
    rec.sample(&tv, S / 2, S / 2);
    assert_eq!(rec.frames(), 1, "no second ticked");
    assert!(rec.due(S, S / 2));
    rec.sample(&tv, S, S / 2);
    assert_eq!(rec.frames(), 2, "the machine's second ticked");
    assert!(rec.due(S, S));
    rec.sample(&tv, S + 1, S);
    assert_eq!(rec.frames(), 3, "the wall second ticked");
    let frames = decode_gif(&rec.gif());
    let (left, top, w, _) = frames[1].0;
    assert!(top >= HEIGHT && left + w <= WIDTH / 2, "the left clock alone: {:?}", frames[1].0);
    let (left, top, _, _) = frames[2].0;
    assert!(top >= HEIGHT && left >= WIDTH / 2, "the right clock alone: {:?}", frames[2].0);
}

/// **Past ninety-nine hours the machine's hours wrap, and the recording
/// goes on.** Two digits hold them: a hundred hours from power-on reads
/// `00:00:00`, as power-on does, and a run of days is a run, a frame at
/// every tick, not a stop at the hundredth hour.
#[test]
fn the_hours_wrap_at_a_hundred() {
    const H: u64 = 3600 * 1_000_000_000;
    let tv = Tv::default();
    let mut long = Recorder::new(true);
    long.sample(&tv, 100 * H, 0);
    let mut fresh = Recorder::new(true);
    fresh.sample(&tv, 0, 0);
    assert_eq!(long.gif(), fresh.gif(), "00:00:00 at a hundred hours as at none");

    let mut days = Recorder::new(true);
    for h in [0u64, 9, 99, 100, 110, 1234] {
        let ns = h * H + 5_000_000_000;
        days.sample(&tv, ns, ns);
    }
    assert_eq!(days.frames(), 6, "a frame at every tick, past the hundredth hour");
}

/// **The lashup records on one canvas, the debugger's screen at the left
/// and the debuggee's at the right.** A word lit at the top of each, the
/// debuggee's a row lower so the two cannot be confused: the first frame
/// is the whole canvas and holds each screen where it belongs, with the
/// rule down between them.
#[test]
fn the_lashup_records_two_screens_side_by_side() {
    let (mut a, mut b) = (Tv::default(), Tv::default());
    // A word is 32 pixels and a row 24 words: word 0 is row 0 from the
    // left, word 24 the same of row 1.
    a.write_buffer(0, 0xffff_ffff);
    b.write_buffer(24, 0xffff_ffff);
    let mut r = Recorder::pair(false);
    r.sample_pair(&a, &b, 0, 0);
    let frames = decode_gif(&r.gif());
    assert_eq!(frames.len(), 1);
    let (rect, px) = &frames[0];
    assert_eq!(*rect, (0, 0, PAIR_W, HEIGHT), "the first frame is the whole canvas");
    assert!((0..32).all(|x| px[x] == 1), "the debugger's word on row 0 at the left");
    assert!(
        (0..32).all(|x| px[PAIR_W + WIDTH + PAIR_RULE + x] == 1),
        "the debuggee's word on row 1 at the right"
    );
    assert!(
        (0..HEIGHT).all(|y| (0..PAIR_RULE).all(|k| px[y * PAIR_W + WIDTH + k] == 1)),
        "the rule down between the two"
    );
}

/// **A change on either screen is a rectangle on its own side.** The
/// debuggee's screen written alone makes a frame wholly in the right half,
/// the debugger's one wholly in the left, each on the row written.
#[test]
fn a_change_is_a_rectangle_on_its_own_side() {
    let (mut a, mut b) = (Tv::default(), Tv::default());
    let mut r = Recorder::pair(false);
    r.sample_pair(&a, &b, 0, 0);
    b.write_buffer(24 * 40 + 2, 0xff);
    r.sample_pair(&a, &b, 1_000_000, 0);
    a.write_buffer(24 * 10 + 1, 0xff);
    r.sample_pair(&a, &b, 2_000_000, 0);
    let rects: Vec<_> = decode_gif(&r.gif()).iter().map(|f| f.0).collect();
    assert_eq!(rects.len(), 3);
    let (left, top, width, height) = rects[1];
    assert!(left >= WIDTH + PAIR_RULE, "the debuggee's change at the right: {:?}", rects[1]);
    assert_eq!((top, width, height), (40, 8, 1), "the eight pixels written: {:?}", rects[1]);
    let (left, top, width, height) = rects[2];
    assert!(left + width <= WIDTH, "the debugger's change at the left: {:?}", rects[2]);
    assert_eq!((top, width, height), (10, 8, 1), "the eight pixels written: {:?}", rects[2]);
}

/// **One clock line under the pair.** The machine's clock keeps to the
/// left of the canvas and the wall clock to the right of the whole of it,
/// not of the debugger's screen; the rule between the screens stops at the
/// line.
#[test]
fn the_clock_line_spans_the_pair() {
    let tv = Tv::default();
    let mut r = Recorder::pair(true);
    r.sample_pair(&tv, &tv, 0, 0);
    let frames = decode_gif(&r.gif());
    let (rect, px) = &frames[0];
    assert_eq!(*rect, (0, 0, PAIR_W, HEIGHT + TIME_H));
    let lit: Vec<bool> =
        (0..PAIR_W).map(|x| (HEIGHT..HEIGHT + TIME_H).any(|y| px[y * PAIR_W + x] == 1)).collect();
    let first = lit.iter().position(|&l| l).expect("the machine's clock");
    let last = lit.iter().rposition(|&l| l).expect("the wall clock");
    assert!(first < WIDTH / 2, "the machine's clock at the left: {first}");
    assert!(last > PAIR_W - WIDTH / 2, "the wall clock at the right of the canvas: {last}");
    assert!(
        !(WIDTH..WIDTH + PAIR_RULE).any(|x| lit[x]),
        "the rule does not run through the clock line"
    );
}

// --- The colour screen ------------------------------------------------------

/// One write of the colour register, `lmtv.order`'s `173777x4`: the byte
/// at `15-8`, the channel at `7-6`, the colour at `3-0`, as
/// `WRITE-COLOR-MAP` lays them out.
fn write_map(m: &mut Machine, colour: u32, channel: u32, value: u32) {
    m.bus_write(0o17377754, value << 8 | channel << 6 | colour);
}

/// The whole map in one go: colour `c`'s three guns from `f`.
fn write_whole_map(m: &mut Machine, f: impl Fn(usize, usize) -> u32) {
    for colour in 0..COLORS {
        for channel in 0..CHANNELS {
            write_map(m, colour as u32, channel as u32, f(colour, channel));
        }
    }
}

/// The colour screen as the recorder should have it: a byte a pixel, the
/// four-bit index of [`Tv::pixel4`], row by row.
fn colour_canvas(tv: &Tv) -> Vec<u8> {
    (0..COLOR_HEIGHT)
        .flat_map(|y| (0..COLOR_WIDTH).map(move |x| (x, y)))
        .map(|(x, y)| tv.pixel4(x, y))
        .collect()
}

/// The sixteen colours as the monitor shows them: [`Tv::rgb`] of each.
fn colours(tv: &Tv) -> Vec<[u8; 3]> {
    (0..COLORS).map(|c| tv.rgb(c)).collect()
}

/// **The colour recording is the four-bit picture through the map that was
/// in force when the frame was taken.** Two samples of the colour screen
/// with the map written between them: each frame's pixels are
/// [`Tv::pixel4`] and the colour table a viewer resolves them through is
/// [`Tv::rgb`] of the sixteen colours as they stood at that frame --- the
/// second frame carrying a table of its own, the map having changed, and
/// covering the whole canvas, since a map that has changed recolours
/// everything already on it.
#[test]
fn the_colour_recording_carries_the_map_it_was_taken_through() {
    let mut m = Machine::new();
    m.fit_color_tv();
    // Line 0's first eight pixels are the nibbles of word 0, low one
    // first: 8, 9, ..., 15.  Line 1's are word `COLOR_WORDS_PER_LINE`.
    m.bus_write(0o17200000, 0xfedc_ba98);
    m.bus_write(0o17200000 + COLOR_WORDS_PER_LINE as u32, 0x0000_0123);
    write_whole_map(&mut m, |colour, channel| (16 * colour + 5 * channel) as u32);

    let mut rec = ColorRecorder::new(false);
    rec.sample(m.color_tv.as_ref().unwrap(), 1_000, 0);
    let first =
        (colours(m.color_tv.as_ref().unwrap()), colour_canvas(m.color_tv.as_ref().unwrap()));

    // A pixel and the map both changed: one frame, the whole canvas, its
    // own table.
    m.bus_write(0o17200000 + 2 * COLOR_WORDS_PER_LINE as u32, 0x0000_000f);
    write_whole_map(&mut m, |colour, channel| (3 * colour + 80 * channel) as u32);
    rec.sample(m.color_tv.as_ref().unwrap(), 501_000_000, 0);
    let second =
        (colours(m.color_tv.as_ref().unwrap()), colour_canvas(m.color_tv.as_ref().unwrap()));
    assert_ne!(first.0, second.0, "the map changed");
    assert_eq!(rec.frames(), 2, "a frame for the first picture and one for the change");

    let (canvas, frames) = decode_gif_fully(&rec.gif());
    assert_eq!(canvas, (COLOR_WIDTH, COLOR_HEIGHT), "COLOR:MAKE-SCREEN's 576 by 454");
    assert_eq!(frames.len(), 2);
    for (k, (want_colours, want_pixels)) in [first, second].into_iter().enumerate() {
        let f = &frames[k];
        assert_eq!(f.rect, (0, 0, COLOR_WIDTH, COLOR_HEIGHT), "frame {k} is the whole canvas");
        assert_eq!(f.colors.len(), COLORS, "frame {k}: sixteen colours");
        assert_eq!(f.colors, want_colours, "frame {k}: the map as Tv::rgb shows it");
        assert_eq!(f.pixels.len(), want_pixels.len(), "frame {k}: the canvas");
        let differs = f.pixels.iter().zip(&want_pixels).position(|(a, b)| a != b);
        assert_eq!(differs, None, "frame {k}: the pixels are Tv::pixel4's indices");
    }
    assert!(!frames[0].local, "the first frame's colours are the file's global table");
    assert!(frames[1].local, "and the second carries its own, the map having changed");
}

/// **The clocks below the colour picture are drawn past the map's sixteen
/// colours**, so the line reads the same whatever the machine wrote into
/// the map: the canvas is [`TIME_H`] taller, the picture's pixels are the
/// map's indices and the line's are not, and the table is long enough to
/// hold both.
#[test]
fn the_colour_clock_line_is_not_drawn_in_the_machines_colours() {
    let mut m = Machine::new();
    m.fit_color_tv();
    // Every colour of the map black, which is what the line would be
    // drawn in if it were drawn in the machine's colours.
    write_whole_map(&mut m, |_, _| 0o377);
    let mut rec = ColorRecorder::new(true);
    rec.sample(m.color_tv.as_ref().unwrap(), 0, 0);
    let (canvas, frames) = decode_gif_fully(&rec.gif());
    assert_eq!(canvas, (COLOR_WIDTH, COLOR_HEIGHT + TIME_H), "a line below the picture");
    let f = &frames[0];
    assert_eq!(f.rect, (0, 0, COLOR_WIDTH, COLOR_HEIGHT + TIME_H));
    assert!(f.colors.len() > COLORS, "the table holds the line's colours too");
    assert!(
        f.pixels[..COLOR_WIDTH * COLOR_HEIGHT].iter().all(|&p| (p as usize) < COLORS),
        "the picture is the map's sixteen"
    );
    let line = &f.pixels[COLOR_WIDTH * COLOR_HEIGHT..];
    assert!(line.iter().all(|&p| (p as usize) >= COLORS), "and the line is not");
    let ink = *line.iter().max().unwrap();
    assert_eq!(f.colors[ink as usize], [255, 255, 255], "the digits white");
    let ground = *line.iter().min().unwrap();
    assert_eq!(f.colors[ground as usize], [0, 0, 0], "on black");
    assert_ne!(ink, ground, "the clocks are drawn on the line");
}
