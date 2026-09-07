// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The display recorder: its frames read back by a GIF reader of the
//! tests' own, its clock line, and the lashup's two screens on one canvas.
//! Nothing here needs `vendor/`.

mod support;

use muir::capture::{PAIR_RULE, Recorder, TIME_H};
use muir::simpletv::{HEIGHT, SimpleTv, WIDTH};

use crate::support::{Frame, decode_gif};

/// The canvas a pair recording is laid on.
const PAIR_W: usize = 2 * WIDTH + PAIR_RULE;

/// **The recording decodes to the screens it was taken from.** Two
/// samples of a screen with a few words lit, the second changed in one
/// place; the GIF's two frames, decoded here by a plain LZW reader, are
/// the whole screen and the changed rectangle, pixel for pixel.
#[test]
fn the_recording_decodes_to_the_screens() {
    let mut tv = SimpleTv::default();
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
    let tv = SimpleTv::default();
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
    let tv = SimpleTv::default();
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
    let tv = SimpleTv::default();
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
    let (mut a, mut b) = (SimpleTv::default(), SimpleTv::default());
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
    let (mut a, mut b) = (SimpleTv::default(), SimpleTv::default());
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
    let tv = SimpleTv::default();
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
