// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The sync program run by the model, `src/tv/sync.rs`, held to
//! what the boards make of the same programs: MIT's PROM program against
//! the netlist SIMPLE TV's measured raster, the program the window system
//! loads for the main screen against its own arithmetic, and the program
//! the color software loads against NTSC. Nothing here needs `vendor/`
//! but the color program, whose test says so when it is skipped.

use muir::tv::sync::{INSTRUCTION_NS, Timeline, prom};
use muir::tv::{FRAME_NS, Tv, mode};

mod support;

/// `SI:FILL-SYNC`'s list form, as `CC-TV-FILL-SYNC` in `cc/dmon.lisp`
/// writes it: an atom is one word, and a sublist `(N w...)` is its words
/// `N` times over.
#[derive(Clone, Debug)]
enum Item {
    Word(u8),
    Repeat(u32, Vec<Item>),
}

fn flatten(items: &[Item], out: &mut Vec<u8>) {
    for it in items {
        match it {
            Item::Word(w) => out.push(*w),
            Item::Repeat(n, body) => {
                for _ in 0..*n {
                    flatten(body, out);
                }
            }
        }
    }
}

/// A number as MIT's Lisp reads it with `Base: 8`: octal unless it ends
/// in a point, which makes it decimal.
fn number(tok: &str) -> u32 {
    match tok.strip_suffix('.') {
        Some(d) => d.parse().unwrap(),
        None => u32::from_str_radix(tok, 8).unwrap(),
    }
}

/// The items of one list, from the text after its opening paren to the
/// matching close.
fn read_list(src: &str) -> (Vec<Item>, &str) {
    let mut items = Vec::new();
    let mut rest = src;
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix(')') {
            return (items, r);
        }
        if let Some(r) = rest.strip_prefix('(') {
            let (body, r) = read_list(r);
            // The sublist's first number is how many times the rest goes.
            let Some(Item::Word(n)) = body.first() else {
                panic!("a repeat count first: {body:?}")
            };
            items.push(Item::Repeat(*n as u32, body[1..].to_vec()));
            rest = r;
            continue;
        }
        if let Some(r) = rest.strip_prefix(';') {
            rest = r.split_once('\n').map_or("", |(_, t)| t);
            continue;
        }
        let end = rest.find(|c: char| c.is_whitespace() || c == ')' || c == '(').unwrap();
        let tok = &rest[..end];
        items.push(Item::Word(number(tok) as u8));
        rest = &rest[end..];
    }
}

/// The program `SET-TV-SPEED` in `sys/window/shwarm.lisp` loads for the
/// main screen at its default of 60.5 Hz, built as the Lisp builds it:
/// `N-LINES` display lines in loops of at most 255, between the vertical
/// sync, retrace and margin loops MIT wrote out.
fn set_tv_speed_program(n_lines: u32) -> Vec<u8> {
    let w = |s: &str| -> Vec<Item> {
        let closed = format!("{s})");
        let (items, rest) = read_list(&closed);
        assert!(rest.is_empty());
        items
    };
    let mut items = Vec::new();
    items.extend(w("1.  (1 33) (5 13) 12 12 (11. 12 12) 212 113"));
    items.extend(w("53. (1 33) (5 13) 12 12 (11. 12 12) 212 13"));
    items.extend(w("8.  (1 31)  (5 11) 11 10 (11. 0 0) 200 21"));
    let mut n = n_lines;
    while n > 0 {
        let dn = n.min(255);
        items.push(Item::Word(dn as u8));
        items.extend(w("(1 31) (5 11) 11 50 (11. 0 40) 200 21"));
        n -= dn;
    }
    items.extend(w("7. (1 31) (5 11) 11 10 (11. 0 0) 200 21"));
    items.extend(w("1. (1 31) (5 11) 11 10 (11. 0 0) 300 23"));
    let mut out = Vec::new();
    flatten(&items, &mut out);
    out
}

/// **MIT's PROM program makes the frame the netlist board makes.** The
/// walk of `cpt.prom` in clock mode 0 gives 966 lines of 32 instructions
/// at 500 ns --- the 966 lines of 16.000 us `tests/simpletv_netlist.rs`
/// measures on the board --- one `TVMA CLR` a frame as the first line's
/// last instruction completes, and 896 lines fetching 12 video cycles of
/// 64 bits, the 768-dot picture, with 54 lines blanked end to end and 912
/// not, which is what the board's raster counts.
#[test]
fn mits_prom_program_makes_the_frame_the_board_makes() {
    let t = Timeline::of(prom(), 0).expect("cpt.prom makes a frame");
    assert_eq!(t.period_ns, FRAME_NS);
    assert_eq!(t.instructions, 966 * 32);
    assert_eq!(t.lines.len(), 966, "lines a frame");
    assert!(t.lines.iter().all(|l| l.instructions == 32), "32 instructions each");
    assert_eq!(t.lines.iter().filter(|l| l.video_cycles == 12).count(), 896, "the picture");
    assert_eq!(t.lines.iter().filter(|l| l.blanked).count(), 54, "blanked end to end");
    assert_eq!(t.lines.iter().filter(|l| !l.blanked).count(), 912, "unblanked");
    assert_eq!(
        t.tvma_clr,
        vec![32 * 500],
        "TVMA CLR as the first line's 32nd instruction completes"
    );
    // The sync bits: vertical for the first 54 lines, horizontal for the
    // first six instructions of each line and the last, each latched an
    // instruction late.
    assert_eq!(t.sync_at(500), (true, true), "the first instruction's bits, once latched");
    assert_eq!(t.sync_at(3_499), (true, true));
    assert_eq!(t.sync_at(3_500), (false, true), "the seventh instruction has no horizontal bit");
    assert_eq!(t.sync_at(16_000), (true, true), "the 32nd raises it for the next line");
    assert_eq!(t.sync_at(54 * 16_000 + 499), (true, true), "the 54th line's last instruction");
    assert_eq!(t.sync_at(54 * 16_000 + 500), (true, false), "vertical sync over after 54 lines");
    assert_eq!(t.sync_at(5_000_000), (false, false), "mid-picture, mid-line");
    assert_eq!(
        t.sync_at(0),
        (true, true),
        "an offset before the first change: the run's last, the program being periodic"
    );
    // That is the steady state, from the second run on. The first run
    // after the program is started is the one run where no instruction of
    // it has landed yet, and there the register still holds what the
    // program before it left: `sync_at_since_start` is handed those bits.
    let held = (false, true);
    assert_eq!(t.sync_at_since_start(0, held), held, "held at the start");
    assert_eq!(t.sync_at_since_start(499, held), held, "and until the first instruction lands");
    assert_eq!(t.sync_at_since_start(500, held), (true, true), "which is this program's own");
    assert_eq!(
        t.sync_at_since_start(FRAME_NS, held),
        (true, true),
        "a run later, the last's again"
    );
    assert_eq!(t.tvma_clrs_by(15_999), 0);
    assert_eq!(t.tvma_clrs_by(16_000), 1);
    assert_eq!(t.tvma_clrs_by(FRAME_NS + 15_999), 1);
    assert_eq!(t.tvma_clrs_by(FRAME_NS + 16_000), 2);
}

/// **The program the window system loads at 60.5 Hz is 1033 lines with 963
/// of picture.** `SET-TV-SPEED` computes `N-LINES` as `1e6 / (16 * 60.5)`
/// less 70 overhead lines, 963, and says "each horizontal line is 32. sync
/// clocks, or 16.0 microseconds with a 64 MHz clock"; the walk agrees with
/// its arithmetic, and the 70 are the 1 + 53 + 8 + 7 + 1 lines of its
/// fixed loops.
#[test]
fn the_window_systems_program_at_sixty_hertz() {
    let n_lines = (1_000_000.0 / (16.0 * 60.5)) as u32 - 70;
    assert_eq!(n_lines, 963, "MAIN-SCREEN-HEIGHT");
    let program = set_tv_speed_program(n_lines);
    let t = Timeline::of(&program, 0).expect("makes a frame");
    assert_eq!(t.lines.len(), 1033);
    assert_eq!(t.period_ns, 1033 * 16_000);
    assert!(t.lines.iter().all(|l| l.instructions == 32));
    assert_eq!(t.lines.iter().filter(|l| l.video_cycles == 12).count(), 963, "display lines");
    assert_eq!(t.tvma_clr.len(), 1);
    let hz = (t.period_ns as f64 / 1e9).recip();
    assert!((hz - 60.5).abs() < 0.01, "the refresh rate asked for, to the whole line: {hz}");
}

/// **The color program is two fields of NTSC.** `COLOR:SYNC` in
/// `sys/window/color.lisp`, "This is really NTSC standard video": 525
/// lines in two fields of 262 and 263, each field's 227 picture lines
/// fetching 36 video cycles of the picture --- 36 x 64 bits is 576 pixels
/// of 4 bits, the color screen's width --- and a 37th, blanked,
/// end-of-line cycle that steps the address by the vertical spacing; 454
/// picture lines in all, the color screen's height, and a `TVMA CLR` at
/// the top of each field. In clock mode 3 an instruction is 625 ns, so a
/// line of 102 is 63.75 us and the frame 33.47 ms: NTSC's 63.56 us line
/// to within the program's whole instructions.
///
/// Read from the vendored release; skipped without it.
#[test]
fn the_color_program_is_two_fields_of_ntsc() {
    let Some(src) = support::release("window/color.lisp") else {
        eprintln!("skipped: needs vendor/system-100-0 (tools/fetch-system-100.sh)");
        return;
    };
    let at = src.find("(DEFCONST SYNC '(").expect("COLOR:SYNC");
    let (items, _) = read_list(&src[at + "(DEFCONST SYNC '(".len()..]);
    let mut program = Vec::new();
    flatten(&items, &mut program);
    let t = Timeline::of(&program, 3).expect("makes a frame");
    assert_eq!(INSTRUCTION_NS[3], 625);
    assert_eq!(t.lines.len(), 525, "NTSC's lines");
    assert!(t.lines.iter().all(|l| l.instructions == 102), "102 instructions a line");
    assert_eq!(t.period_ns, 525 * 102 * 625);
    assert_eq!(
        t.lines.iter().filter(|l| l.video_cycles == 37).count(),
        454,
        "the picture, both fields"
    );
    // The rest fetch nothing, but for the one line MIT comments
    // "step-tvma", whose end-of-line cycle steps the address between the
    // fields.
    assert_eq!(t.lines.iter().filter(|l| l.video_cycles == 1).count(), 1, "step-tvma");
    assert_eq!(t.lines.iter().filter(|l| l.video_cycles == 0).count(), 525 - 454 - 1);
    assert_eq!(t.tvma_clr.len(), 2, "a field start each");
    let line_us = 102.0 * 0.625;
    assert!((line_us - 63.56f64).abs() < 0.25, "a line of {line_us} us against NTSC's 63.56");
}

/// **A program that never ends makes no frame.** The RAM before the
/// software has loaded it is zeros --- a count of 256 and then no End of
/// Loop before the memory runs out --- and one loaded part way is what it
/// is; neither is walked for ever.
#[test]
fn a_program_that_never_ends_makes_no_frame() {
    assert!(Timeline::of(&[0; 4096], 0).is_none(), "zeros");
    let mut half = prom().to_vec();
    half.truncate(0o200);
    half.resize(4096, 0);
    assert!(Timeline::of(&half, 0).is_none(), "half of cpt.prom");
    assert!(Timeline::of(&[], 0).is_none(), "nothing at all");
}

/// **The board runs its PROM from power-on and the RAM once it is loaded
/// and selected**, and the model's register bits follow: the vertical
/// flag is preset where the running program's `TVMA CLR` falls, and the
/// sync bits read as the program has them. Loading the window system's
/// 60.5 Hz program as `SETUP-CPT` does --- the RAM selected, the words
/// written, the mode and the spacing set --- moves the frame from
/// `cpt.prom`'s 15.456 ms to 16.528 ms.
#[test]
fn the_model_runs_the_prom_and_then_the_ram() {
    let mut tv = Tv::default();
    assert_eq!(tv.timeline().map(|t| t.period_ns), Some(FRAME_NS), "the PROM from power-on");
    assert_eq!(tv.read_control(0, 500) & (mode::HSYNC | mode::VSYNC), mode::HSYNC | mode::VSYNC);
    assert_eq!(tv.read_control(0, 5_000_000) & (mode::HSYNC | mode::VSYNC), 0);
    assert!(!tv.vert_flag(15_999));
    assert!(tv.vert_flag(16_000), "TVMA CLR at the end of the first line");

    // SETUP-CPT: the RAM in, a word at a time through the pointer, the
    // mode, then the enable with the spacing.
    let program = set_tv_speed_program(963);
    let mut now = 20_000_000;
    tv.write_control(3, 0o200, now);
    assert!(tv.timeline().is_none(), "the RAM selected, holding no program");
    for (k, &w) in program.iter().enumerate() {
        now += 1_000;
        tv.write_control(2, k as u32, now);
        tv.write_control(1, w as u32, now);
    }
    tv.write_control(0, 0, now);
    tv.write_control(3, 0o200 | 9, now);
    let t = tv.timeline().expect("the program loaded makes a frame");
    assert_eq!(t.lines.len(), 1033);
    assert_eq!(tv.origin(), now, "run from its start at the last change");
    assert!(!tv.vert_flag(now + 15_999));
    assert!(tv.vert_flag(now + 16_000), "the first line's end again");
    tv.write_control(0, mode::INTERRUPT_ENABLE, now + 16_001);
    assert!(!tv.interrupt(now + 16_001));
    assert!(!tv.interrupt(now + 16_000 + 1033 * 16_000 - 1));
    assert!(tv.interrupt(now + 16_000 + 1033 * 16_000), "and a 60.5 Hz frame later");
    // The PROM again when the RAM is deselected, from its start.
    tv.write_control(3, 0, now + 30_000_000);
    assert_eq!(tv.timeline().map(|t| t.period_ns), Some(FRAME_NS));
    assert_eq!(tv.origin(), now + 30_000_000);
}
