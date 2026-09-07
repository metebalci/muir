// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The monitor on the display board's video cable.
//!
//! `tests/simpletv_netlist.rs` measures the raster from inside the board,
//! off `HSYNC OUT`, `VSYNC OUT` and the sync program's own `BLANKING`.
//! This measures it from **outside**, off the four wires that leave for
//! the monitor and nothing else, which is the check that a monitor has
//! everything it needs: the board hands over a picture, not a set of
//! internal signals.

use muir::netlist::{self, Netlist};
use muir::part::Level;
use muir::terminal::monitor::{DOTS_A_LINE, Monitor};
use muir::xbus::XbusMaster;

const SIMPLETV: &str = include_str!("../data/SIMPLETV.netlist");

fn simpletv() -> Netlist {
    netlist::parse(SIMPLETV).unwrap()
}

/// Runs the board with a monitor on it until `frames` are painted, or the
/// deadline passes, and gives back the monitor.
fn paint(b: &mut XbusMaster, n: &Netlist, frames: u64, ns: u64) -> Monitor {
    let mut m = Monitor::of(n).expect("a display board has a video cable");
    m.attach(&mut b.chip, b.now);
    let deadline = b.now + ns;
    while b.now < deadline && m.frames < frames {
        let tap = b.chip.next_tap().unwrap_or(deadline).clamp(b.now + 1, deadline);
        b.run(tap);
        m.sample(&b.chip);
    }
    m
}

/// **All four wires of the cable float, and the monitor terminates them.**
/// Two directions: the video pair is ECL and open emitter, so it takes a
/// pull-down, and MIT's `necsip.drw` terminates the board's other 26 ECL
/// nets and not these two. The sync pair is the 74S37 at NSYREG 0D09,
/// which MIT drew with SUDS's open-collector suffix, so it can pull low
/// and never drive high and takes a pull-up.
///
/// Unterminated, `VSYNC OUT` never falls *from a high* and a monitor
/// waiting for flyback waits for ever, which is what happened the first
/// time this was run.
#[test]
fn the_monitor_terminates_the_cable() {
    let n = simpletv();
    let net = |s: &str| n.by_name_id(s).unwrap();
    let wires = ["'HSYNC OUT'", "'VSYNC OUT'", "'MECL VIDEO OUT'", "'-MECL VIDEO OUT'"];

    // Every level each wire takes over a line, with a monitor and without.
    let levels = |attached: bool| {
        let mut b = XbusMaster::new(&n, 0);
        let m = Monitor::of(&n).unwrap();
        if attached {
            m.attach(&mut b.chip, b.now);
        }
        let ids = wires.map(net);
        let mut seen: [Vec<Level>; 4] = Default::default();
        let end = b.now + 16_000;
        while b.now < end {
            let tap = b.chip.next_tap().unwrap_or(end).clamp(b.now + 1, end);
            b.run(tap);
            for (k, &id) in ids.iter().enumerate() {
                let level = b.chip.net(id);
                if !seen[k].contains(&level) {
                    seen[k].push(level);
                }
            }
        }
        seen
    };

    let bare = levels(false);
    for (k, name) in wires.iter().enumerate() {
        eprintln!("  no monitor: {name:20} {:?}", bare[k]);
    }
    // **Nothing on the cable can swing without a monitor.** Each of the
    // four drives one way only --- the sync pair is open collector and
    // pulls low, the video pair is open emitter and pulls high --- so the
    // level each of them is missing is the monitor's to supply, and until
    // it is there no wire ever takes both. Which of the two it is depends
    // on where in the frame this window falls; that none of them swings
    // does not.
    for (k, name) in wires.iter().enumerate() {
        assert!(
            !(bare[k].contains(&Level::High) && bare[k].contains(&Level::Low)),
            "{name} swings with nothing terminating it: {:?}",
            bare[k]
        );
    }

    let held = levels(true);
    for (k, name) in wires.iter().enumerate() {
        eprintln!("  a monitor:  {name:20} {:?}", held[k]);
        assert!(
            !held[k].contains(&Level::Z),
            "{name} is terminated and never floats: {:?}",
            held[k]
        );
    }
    // And the one that matters most: flyback. `VSYNC OUT` pulses once a
    // frame, so a window of one line need not show it swing; `HSYNC OUT`
    // is once a line and must.
    let hsync = &held[wires.iter().position(|&w| w == "'HSYNC OUT'").unwrap()];
    assert!(
        hsync.contains(&Level::High) && hsync.contains(&Level::Low),
        "HSYNC OUT swings once a monitor is pulling it up: {hsync:?}"
    );
}

/// **The board hands the monitor a picture, and the picture is the frame
/// buffer.** 966 lines between flybacks, arrived at without reading one net
/// that does not leave the board.
///
/// **This test used to assert 912 lit lines with nothing written**, and it
/// passed for the wrong reason: `-MECL VIDEO` had no driver, its terminator
/// held it low, and the video output was the blanking signal alone, so every
/// unblanked dot was lit whatever the buffer held. With
/// [`muir::netlist::Netlist::STRAP_PAGES`]'s `ECLVID` join in place the video
/// follows the buffer, an empty buffer is a dark screen, and the old
/// assertion measured the raster rather than the picture.
///
/// So write first and then look. Two lines of the buffer are filled and the
/// two raster lines they land on must carry a full line of dots while every
/// other line stays dark. That is the property the old test was reaching for
/// and could not have caught being broken.
///
/// The offset is the border: of the 912 unblanked lines only 896 fetch, 8
/// above and 8 below fetching nothing, so buffer line 0 is raster line 9.
/// `tests/simpletv_netlist.rs` measures both counts from the inside.
///
/// Ignored for the same reason the frame test there is: a frame of board
/// time is about half a minute.
///
///     cargo test --test monitor -- --ignored --nocapture
#[test]
#[ignore = "a frame of board time is about half a minute: cargo test --test monitor -- --ignored --nocapture"]
fn the_monitor_is_handed_the_boards_picture() {
    use muir::simpletv::BUFFER;
    /// 768 dots at 32 bits a word.
    const WORDS_A_LINE: u32 = 24;
    /// Buffer line 0 is raster line 9: eight border lines and the dark one.
    const FIRST: usize = 9;

    let n = simpletv();
    let mut b = XbusMaster::new(&n, 0);
    // A request has to be run out and released before the next one: issued
    // back to back only the last of them reaches the DRAMs.
    // Long enough that a cycle delayed by a DRAM refresh still completes:
    // at 1,100 ns one word in twenty-four was abandoned mid-line, which
    // showed up as a 32-dot gap in an otherwise whole line.
    let write = |b: &mut XbusMaster, address: u32| {
        b.request(address, Some(0xffff_ffff));
        let until = b.now + 20_000;
        b.run(until);
        b.release();
        let until = b.now + 600;
        b.run(until);
    };
    // The first request a freshly built master makes is lost however long
    // the board is left to settle first, so the first word is written twice
    // and the duplicate is harmless. Without it the first line written comes
    // out 32 dots short and the second is whole, which is what showed it.
    let first = BUFFER + 100 * WORDS_A_LINE;
    write(&mut b, first);
    for line in [100u32, 200] {
        for w in 0..WORDS_A_LINE {
            write(&mut b, BUFFER + line * WORDS_A_LINE + w);
        }
    }
    let m = paint(&mut b, &n, 2, 16_000 * 2_100);

    let (dots, lines) = m.lit();
    eprintln!("{} frames, {} lines, {dots} dots lit on {lines} lines", m.frames, m.lines());
    for row in [FIRST + 100, FIRST + 200] {
        let d = m.dots_on(row);
        eprintln!(
            "  raster line {row} carries {} dots, first {:?} last {:?}",
            d.len(),
            d.first(),
            d.last()
        );
    }
    assert_eq!(m.frames, 2, "two flybacks");
    assert_eq!(m.lines(), 966, "lines between one vertical flyback and the next");
    assert_eq!(lines, 2, "only the two lines written are lit: {dots} dots on {lines} lines");
    // 768, `MAIN-SCREEN-WIDTH`, not [`DOTS_A_LINE`]: the sweep is 1024 dots
    // and the picture is the unblanked 768 of them, starting at 40.
    for row in [FIRST + 100, FIRST + 200] {
        let d = m.dots_on(row);
        assert_eq!(d.len(), 768, "raster line {row} carries the whole buffer line");
        assert_eq!((d[0], d[767]), (40, 807), "and carries it where the sweep is unblanked");
        assert!(DOTS_A_LINE > d[767], "a line's dots fit in the sweep");
    }
    assert!(m.dots_on(0).is_empty(), "the first line of the frame is dark");
    assert!(m.dots_on(FIRST + 150).is_empty(), "a line nothing was written to is dark");
}
