// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The display board as a netlist: `data/SIMPLETV.netlist`, MIT's `cadrtv`
//! drawings through `tools/simpletv-netlist.sh`.
//!
//! The board is the SIMPLE TV, the one `src/simpletv.rs` models. Two things
//! stand behind it: `cadrtv/lmtv.stf`, MIT's own page list for the board,
//! which names 29 drawings and titles every one of them `SIMPLE TV`, and
//! `cadrtv/lmtv.order`, MIT's register map for it, which the drawings have to
//! decode the way it says. There is no wire list for this board on the
//! tapes, so the second source is MIT's specification rather than MIT's
//! wiring.
//!
//! `cadrtv/` holds two boards, the SIMPLE TV and the LISPM TV that replaced
//! it, six of whose pages share filenames; `every_page_is_a_simple_tv_page`
//! is the check that the right one was taken every time.

use std::collections::BTreeSet;

use muir::netlist::{self, Netlist, Part};
use muir::simpletv::mode;

mod support;

const SIMPLETV: &str = include_str!("../data/SIMPLETV.netlist");

fn simpletv() -> Netlist {
    netlist::parse(SIMPLETV).unwrap()
}

/// The one part of that type at that place. Every reference used below is
/// unique on its page, and the assertion says so.
fn at<'a>(n: &'a Netlist, page: &str, reference: &str, kind: &str) -> &'a Part {
    let mut it =
        n.parts.iter().filter(|p| p.page == page && p.reference == reference && p.kind == kind);
    let found = it.next().unwrap_or_else(|| panic!("no {kind} at {page} {reference}"));
    assert!(it.next().is_none(), "{kind} at {page} {reference} is not unique");
    found
}

/// The net on a pin, as MIT names it. A name with a space in it is quoted
/// in the file and keeps its quotes in the netlist; they are stripped here
/// so that the assertions below read as the drawing does.
fn on<'a>(n: &'a Netlist, part: &Part, pin: u8) -> &'a str {
    let (_, net) = part.pins.iter().find(|&&(k, _)| k == pin).expect("pin is wired");
    let name = n.net(*net);
    name.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')).unwrap_or(name)
}

/// The shape of the board, pinned the way the other netlists are: MIT's 29
/// pages, `synmod` and `xbctl` of the May 1979 list having been folded into
/// one sheet, `nxbctl`, by May 1980, and `rampwr` added.
///
/// **The RAM count is the frame buffer.** NRAMA to NRAMD carry sixteen
/// 16K-by-1 DRAMs each, and 64 by 16,384 bits is exactly the 32,768 words of
/// 32 bits `shwarm.lisp` reserves --- which is `lmtv.order`'s "the video
/// buffer is really 64 bits wide internally; the processor accesses 32 bit
/// half-words", one row of DRAMs to sixteen of those 64 bits.
#[test]
fn parses_to_the_expected_shape() {
    let n = simpletv();
    assert_eq!(n.pages.len(), 29, "{:?}", n.pages);
    assert_eq!(n.parts.len(), 384);
    let rams = n.parts.iter().filter(|p| p.kind == "4116VG").count();
    assert_eq!(rams, 64, "four rows of sixteen 16K DRAMs: 64 by 16K by 1");
    assert_eq!(
        rams * 16_384,
        muir::simpletv::BUFFER_WORDS as usize * 32,
        "the DRAMs hold exactly MAIN-SCREEN-BUFFER-LENGTH words of 32 bits"
    );
}

/// Every part on the board is identified, and everything that computes
/// anything has a behaviour. Two parts have a pinout and no behaviour, and
/// both are analog: the RAS/CAS delay line at RAMCAS 0D11 and the 64 MHz
/// oscillator can at NECCLK 0C08, which `src/chip.rs` runs as it runs the
/// other boards'.
#[test]
fn every_part_is_identified() {
    let k = support::kinds(&simpletv());
    assert!(k.unknown.is_empty(), "no pinout for {:?}", k.unknown);
    assert_eq!(k.silent, ["TD100", "TTLOSC"], "parts with no behaviour");
}

/// Two totem-pole outputs on one net is an electrical fault, so a wrongly
/// claimed output pin surfaces here.
#[test]
fn no_net_has_two_push_pull_drivers() {
    support::no_net_has_two_push_pull_drivers(&simpletv());
}

/// **Every page is a SIMPLE TV page.** This board's drawings share their
/// names with the LISPM TV's, which replaced them in December 1980, so the
/// risk `tools/simpletv-netlist.sh` runs is picking up a sheet of the wrong
/// board. soap4 copies each drawing's own title block into the banner above
/// its page, and MIT titled every sheet of this board `SIMPLE TV` --- the
/// backplane page excepted, which is titled after the bus it wires.
#[test]
fn every_page_is_a_simple_tv_page() {
    let titles: Vec<&str> =
        SIMPLETV.lines().filter_map(|l| l.strip_prefix("# title 1: ")).map(str::trim).collect();
    assert_eq!(titles.len(), 29, "one title block a page");
    let (backplane, board): (Vec<&str>, Vec<&str>) = titles.into_iter().partition(|&t| t == "XBUS");
    assert_eq!(backplane, ["XBUS"], "the one backplane page, NXBUS");
    assert!(board.iter().all(|&t| t == "SIMPLE TV"), "a page of another board: {board:?}");
}

/// **The mode register is the one `src/simpletv.rs` models.** NXBCTL 0F12
/// is an Am25LS2519, a quad register with two three-state output sets:
/// `XDI0..3` go in, the `W` outputs are the four bits the board acts on, and
/// the `Y` outputs put the same four back on `XDO 0..3` for a read. Four
/// data pins is the whole of what a write can change, which is
/// [`mode::WRITABLE`].
#[test]
fn the_writable_mode_bits_are_the_four_the_register_has() {
    let n = simpletv();
    let reg = at(&n, "NXBCTL", "0F12", "25LS2519");

    // Datasheet pin pairs: D on 1/4/13/16, W on 2/5/12/15, Y on 3/6/11/14.
    let bits = [
        (0, 1, 2, 3, "CLOCK MODE 0"),
        (1, 4, 5, 6, "CLOCK MODE 1"),
        (2, 13, 12, 11, "MODE BOW"),
        (3, 16, 15, 14, "MODE INTR ENB"),
    ];
    for (bit, d, w, y, name) in bits {
        assert_eq!(on(&n, reg, d), format!("XDI{bit}"), "bit {bit} in");
        assert_eq!(on(&n, reg, w), name, "bit {bit} out to the board");
        assert_eq!(on(&n, reg, y), format!("XDO {bit}"), "bit {bit} read back");
        assert!(mode::WRITABLE & 1 << bit != 0, "bit {bit} is writable");
    }
    assert_eq!(mode::WRITABLE, 0o17, "the register has four data pins and no more");
    assert_eq!(mode::CLOCK, 0o3, "CLOCK MODE 0 and 1");
    assert_eq!(mode::BOW, 0o4, "MODE BOW");
    assert_eq!(mode::INTERRUPT_ENABLE, 0o10, "MODE INTR ENB");

    // The register is cleared by power reset, not by Xbus init: ECO 1 of
    // `cadrtv/lmtv.eco`, 3 June 1979, "annoying screen popping ... reset
    // from power-on, not from xbus init", which this drawing is after.
    assert_eq!(on(&n, reg, 19), "-POWER RESET");
}

/// **The four bits above the register are the read buffer's, and nothing
/// latches them.** NXBCTL 0F11 is half a 74LS244 with `VERT FLAG`, `VSYNC`,
/// `HSYNC` and `SYNC PROM ENB` on its inputs and `XDO 4..7` on its outputs,
/// which is [`mode::READ_ONLY`], and the drawing is why `src/simpletv.rs`
/// says they cannot be written.
#[test]
fn the_read_only_mode_bits_come_off_a_buffer() {
    let n = simpletv();
    let buf = at(&n, "NXBCTL", "0F11", "74LS244-A");
    assert_eq!(on(&n, buf, 1), "-RD MODE", "the buffer turns on for a mode read");

    // Datasheet pin pairs: 1A1..1A4 on 2/4/6/8, 1Y1..1Y4 on 18/16/14/12.
    // Bit 7's input is `GND`, not the `SYNC PROM ENB` the drawing has: ECO
    // 2 of `lmtv.eco`, which `examples/reconcile.rs` applies when the
    // netlist is built, so that the window system's PROM-mode check reads
    // zero on this board as it did on MIT's.
    let bits = [
        (4, 2, 18, "VERT FLAG", mode::VERT),
        (5, 4, 16, "VSYNC", mode::VSYNC),
        (6, 6, 14, "HSYNC", mode::HSYNC),
        (7, 8, 12, "GND", mode::SYNC_PROM_ENABLE),
    ];
    for (bit, a, y, name, m) in bits {
        assert_eq!(on(&n, buf, a), name, "bit {bit} in");
        assert_eq!(on(&n, buf, y), format!("XDO {bit}"), "bit {bit} out");
        assert_eq!(m, 1 << bit, "{name} is bit {bit}");
        assert!(mode::READ_ONLY & m != 0 && mode::WRITABLE & m == 0, "{name} is read only");
    }
    assert_eq!(mode::READ_ONLY, 0o360);
}

/// **The control registers decode as MIT's own order sheet says.** NXBCTL
/// 0F13 is a 74S138 on `ADR0..2` and `CTL RQ`, and its first five outputs
/// are the five writes `cadrtv/lmtv.order` lists at `173777x0` to
/// `173777x4`: mode, sync program, sync pointer, vertical spacing, colour.
/// `src/simpletv.rs` models the first and answers the rest without storing
/// them.
#[test]
fn the_control_registers_decode_in_mits_order() {
    let n = simpletv();
    let dec = at(&n, "NXBCTL", "0F13", "74S138");
    for (pin, net) in [(1, "ADR0"), (2, "ADR1"), (3, "ADR2"), (5, "-WRITE"), (6, "CTL RQ")] {
        assert_eq!(on(&n, dec, pin), net, "decoder input");
    }
    // Datasheet: Y0..Y7 on 15, 14, 13, 12, 11, 10, 9, 7.
    let loads = [
        (0, 15, "-LOAD MODE"),
        (1, 14, "-LOAD SYNC"),
        (2, 13, "-LOAD SYNC PTR"),
        (3, 12, "-LOAD VERT SPACING"),
        (4, 11, "-LOAD COLOR"),
    ];
    for (register, pin, name) in loads {
        assert_eq!(on(&n, dec, pin), name, "register {register}");
    }
    // Registers 5, 6 and 7 answer and do nothing, which is why the board
    // takes eight words and `src/simpletv.rs` with it. `src/netlist.rs`
    // gives every unconnected pin a net of its own, numbered, so that they
    // do not tie together.
    for pin in [10, 9, 7] {
        assert!(on(&n, dec, pin).starts_with("NC#"), "the top three decode to nothing");
    }
    assert_eq!(muir::simpletv::CONTROL_WORDS, 8);
}

/// **The board has the Xbus nets it needs, and not the two it does not.**
/// `XbusMaster` hangs a board by 22 address nets, 32 data nets, `-XBUS.RQ`,
/// `-XBUS.ACK` and `-XBUS.WR`, and this board has all 57, on its XBADR and
/// XBDATA pages. Two nets the memory board and the disk controller
/// carry are absent here **by design**: `-XBUS.PAR`, because the buffer is
/// "without parity" (`cadrtv/lmtv.order`), and `-XBUS.SYNC`, because the board
/// works its handshake off its own dot clock. `XbusMaster` takes both as
/// optional for that reason.
#[test]
fn the_xbus_nets_are_the_boards_own() {
    let n = simpletv();
    // This board's own spellings: the address lines with a space, `-XADDR 0`,
    // and the data lines without, `-XBUS0`, on the same two pages. The
    // memory board and the disk controller write both without. `XbusMaster`
    // finds a net either way; here the board is held to what it says.
    let mut want: Vec<String> =
        ["-XBUS.RQ", "-XBUS.ACK", "-XBUS.WR"].iter().map(|s| s.to_string()).collect();
    want.extend((0..22).map(|k| format!("'-XADDR {k}'")));
    want.extend((0..32).map(|k| format!("-XBUS{k}")));
    assert_eq!(want.len(), 57);
    let missing: Vec<&String> = want.iter().filter(|w| n.by_name_id(w).is_none()).collect();
    assert!(missing.is_empty(), "not on the board: {missing:?}");

    assert!(n.by_name_id("-XBUS.PAR").is_none(), "no parity: lmtv.order says so");
    assert!(n.by_name_id("-XBUS.SYNC").is_none(), "no master clock: the board has its own");
    assert!(n.by_name_id("-XBUS.INIT").is_some(), "but it is reset from the bus");
}

/// **The board comes up on the bus, running.**
///
/// Powered, reset and left for two microseconds as the memory board is,
/// **no net is at an unknown level** and the Xbus side is at rest:
/// `-XBUS.ACK` released, the mode register clear. That takes the dot clock:
/// the 64 MHz can at NECCLK 0C08 through the 10124 translator, the 10136
/// divider counting down 3, 2, 1, 0 and reloading on its own terminal
/// count, and `CLK CTR 1` back through the 10125 as `16 MHZ CLK0`, the
/// clock the handshake shift register at NXBCTL 0E12 runs on. Before those
/// parts had pinouts the register never clocked, its last stage `-ACK T3`
/// stayed low, and that held the `SEND ACK` flop at 0E14 in the 74LS74's
/// forbidden state --- preset and clear both low --- with `-XBUS.ACK`
/// driven low: the board acknowledging a request nobody made. Four ticks
/// of the clock clear it, and now the board gives itself thousands.
///
/// The clock is measured rather than assumed: `16 MHZ CLK0` must make
/// thirty-two edges in a microsecond, which is 64 MHz divided by four.
#[test]
fn the_board_comes_up_on_the_bus() {
    use muir::part::Level;
    use muir::xbus::XbusMaster;

    let n = simpletv();
    let mut b = XbusMaster::new(&n, 0);

    let wired: BTreeSet<netlist::NetId> =
        n.parts.iter().flat_map(|p| p.pins.iter().map(|&(_, net)| net)).collect();
    let unknown: Vec<&str> =
        wired.iter().filter(|&&id| b.chip.net(id) == Level::X).map(|&id| n.net(id)).collect();
    eprintln!("{} nets wired, {} unknown", wired.len(), unknown.len());
    assert!(unknown.is_empty(), "nets at unknown with the bus driven: {unknown:?}");
    for bit in ["CLOCK MODE 0", "CLOCK MODE 1", "MODE BOW", "MODE INTR ENB"] {
        assert_eq!(b.level(bit), Level::Low, "{bit} clear after -POWER RESET");
    }
    for name in
        ["-ACK T3", "SEND ACK", "-SEND ACK", "-XBUS.ACK", "16 MHZ CLK0", "CLK CTR 0", "CLK CTR 1"]
    {
        eprintln!("  {name:14} {:?}", b.level(name));
    }

    // The handshake chain has clocked itself clear.
    assert_eq!(b.level("-ACK T3"), Level::High, "the shift register has clocked");
    assert_eq!(b.level("SEND ACK"), Level::Low, "nothing to acknowledge");
    assert_eq!(b.level("-XBUS.ACK"), Level::High, "and the bus is released");

    // The dot clock, measured: edges of 16 MHZ CLK0 over the next microsecond.
    let clk = b.net("16 MHZ CLK0");
    let (mut edges, mut last) = (0, b.chip.net(clk));
    let until = b.now + 1_000;
    while b.now < until {
        b.run(b.now + 1);
        let now = b.chip.net(clk);
        if now != last {
            edges += 1;
            last = now;
        }
    }
    eprintln!("16 MHZ CLK0: {edges} edges in 1000 ns");
    assert!((30..=34).contains(&edges), "16 MHz is 32 edges a microsecond, saw {edges}");
}

/// The raster `cpt.prom` programs, from the static walk of the sync
/// program --- `cadrtv/lmtv.order` lines 88-135
/// give the program's semantics, so it can be executed by hand --- read
/// beside `lmtv.order` line 79's "an instruction every 32 bits of video
/// ... or roughly every 1/2 microsecond": 32 dots an instruction, 32
/// instructions a line, 1024 dots a line, 966 lines a frame, and 896 of
/// those lines carrying the 768 dots the window system draws.
///
/// **These are whole nanoseconds, not rounded ones.** The can is
/// `chip::TV_OSCILLATOR_PERIOD`, 125/8 ns, so a half-period is 125/16 and
/// [`muir::chip::toggle_at`] puts the 2048th of them --- a line --- at
/// exactly 16,000 ns. Every instruction boundary is whole for the same
/// reason, 64 half-periods being 500 ns on the nose.
const LINE_NS: u64 = 16_000;

/// Lines a frame, sync and blanking counted in: 966 at [`LINE_NS`] is
/// 15.456 ms, 64.7 Hz.
const LINES_A_FRAME: u64 = 966;

/// Runs the board to `until`, stopping at every tap on the way --- the 64
/// MHz can's edges, and the RAS/CAS delay line's --- and says when
/// `HSYNC OUT` and `VSYNC OUT` each fell.
///
/// [`XbusMaster::run`] on its own steps by taps too, but only tells the
/// caller where it ended up; a raster is in the times of the edges
/// between, so the stepping is done here and `run` is asked for one tap
/// at a time.
fn sync_falls(b: &mut muir::xbus::XbusMaster, until: u64) -> (Vec<u64>, Vec<u64>) {
    use muir::part::Level;
    let (h, v) = (b.net("HSYNC OUT"), b.net("VSYNC OUT"));
    let (mut was_h, mut was_v) = (b.chip.net(h), b.chip.net(v));
    let (mut hsync, mut vsync) = (Vec::new(), Vec::new());
    while b.now < until {
        let tap = b.chip.next_tap().unwrap_or(until).clamp(b.now + 1, until);
        b.run(tap);
        let (now_h, now_v) = (b.chip.net(h), b.chip.net(v));
        if now_h != was_h {
            if now_h == Level::Low {
                hsync.push(b.now);
            }
            was_h = now_h;
        }
        if now_v != was_v {
            if now_v == Level::Low {
                vsync.push(b.now);
            }
            was_v = now_v;
        }
    }
    (hsync, vsync)
}

/// **The sync program repeats its line.** The program is a loop: 32
/// instructions carry one line, and the 74LS569 counter at NSYADR is
/// reloaded from the `SYNC BEG` latches to run the same 32 again for the
/// next line of the band. So `HSYNC OUT` must fall once every
/// [`LINE_NS`], for ever.
///
/// `-SYNC ADR LOAD` has to reach pin 11 of all three 74LS569s from a
/// driver on this board for that to happen, and on the netlist as first
/// extracted it did not: the program counted straight up through the PROM
/// and `HSYNC OUT` fell once and never returned. This is the check that
/// the loop closes.
#[test]
fn the_sync_program_repeats_the_line() {
    use muir::xbus::XbusMaster;

    let n = simpletv();
    let mut b = XbusMaster::new(&n, 0);
    let window = 7 * LINE_NS;
    let until = b.now + window;
    let (hsync, vsync) = sync_falls(&mut b, until);
    eprintln!("over {window} ns, {} line times:", window / LINE_NS);
    eprintln!("  HSYNC OUT fell at {hsync:?}");
    eprintln!("  VSYNC OUT fell at {vsync:?}");

    assert!(
        hsync.len() >= 6,
        "seven line times should carry at least six HSYNC falls, saw {}: {hsync:?}",
        hsync.len()
    );
    let periods: Vec<u64> = hsync.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(
        periods.iter().all(|&p| p == LINE_NS),
        "every line is {LINE_NS} ns, 1024 dots of the 64 MHz can; saw {periods:?}"
    );
}

/// Lines of the frame the sync program leaves unblanked, at
/// [`ACTIVE_DOTS`] each, and the lines it blanks end to end. **Measured on
/// the board.**
///
/// **912 and 896 are both right and count different things**, which is
/// what a hand walk of `cpt.prom` settled: the program's nine loops give
/// 966 lines a frame and 16.000 us a line, of which 54 are blanked end to
/// end, 912 are unblanked, and only the four loops with `255 + 255 + 255 +
/// 131` iterations issue video cycles. So 912 is the unblanked raster and
/// **896 is the picture**, with 8 unblanked lines above it and 8 below
/// that fetch nothing: a border. Confirmed on the board by writing single
/// buffer lines around the boundary --- buffer line 895 appears and 896
/// never does.
///
/// MIT says 896 twice in its own window system. `sys/window/shwarm.lisp`
/// has `(DEFVAR MAIN-SCREEN-HEIGHT ... (:CADR 963.) ;was 896. for CPT`,
/// and its `SET-TV-SPEED` computes display lines as the frame's total less
/// 70 overhead lines, which for this program's 966 is 896.
const ACTIVE_LINES: usize = 912;
/// The rest of the frame: 966 - 912.
const BLANKED_LINES: usize = 54;
/// Dots a line the program leaves unblanked, and `shwarm.lisp`'s
/// `MAIN-SCREEN-WIDTH`. The walk of `cpt.prom` gives the same, by a
/// different route: 12 video cycles a line at 64 bits a load.
const ACTIVE_DOTS: u32 = 768;

/// One frame as the monitor takes it: `VSYNC OUT` round to `VSYNC OUT`,
/// and for each line between, how many dots the program left unblanked ---
/// falling edges of the dot clock with `BLANKING` low, which is the sync
/// program's own bit 3 (`gen4b.drw` names the eight).
fn frame_dots(b: &mut muir::xbus::XbusMaster) -> Vec<u32> {
    use muir::part::Level;
    let (h, v) = (b.net("HSYNC OUT"), b.net("VSYNC OUT"));
    let (blank, clk) = (b.net("BLANKING"), b.net("-64 MHZ CLK"));
    let (mut was_h, mut was_v, mut was_c) = (b.chip.net(h), b.chip.net(v), b.chip.net(clk));
    let mut in_frame = false;
    let (mut dots, mut lines) = (0u32, Vec::new());
    // Enough for a VSYNC to come round and a whole frame to follow it.
    let deadline = b.now + 2 * LINE_NS * LINES_A_FRAME + LINE_NS;
    while b.now < deadline {
        let tap = b.chip.next_tap().unwrap_or(deadline).clamp(b.now + 1, deadline);
        b.run(tap);
        let now_c = b.chip.net(clk);
        if now_c != was_c {
            if now_c == Level::Low && in_frame && b.chip.net(blank) == Level::Low {
                dots += 1;
            }
            was_c = now_c;
        }
        let now_h = b.chip.net(h);
        if now_h != was_h {
            if now_h == Level::Low && in_frame {
                lines.push(dots);
                dots = 0;
            }
            was_h = now_h;
        }
        let now_v = b.chip.net(v);
        if now_v != was_v {
            if now_v == Level::Low {
                if in_frame {
                    break;
                }
                in_frame = true;
            }
            was_v = now_v;
        }
    }
    lines
}

/// **The frame is 966 lines, and 912 of them carry 768 dots.** `VSYNC OUT`
/// falls once every 966 lines, 15.456 ms apart, and between two of them the
/// sync program blanks 54 lines end to end and leaves 912 unblanked for
/// exactly 768 dots each --- no line part way between, which is what says
/// the count is the program's and not an artefact of where the measurement
/// starts.
///
/// Ignored because it runs two frames of board time --- two, so that a
/// VSYNC first seen part way through one still has a whole frame after
/// it --- which is twenty-four seconds against a suite that takes two:
///
///     cargo test --test simpletv_netlist -- --ignored --nocapture
#[test]
#[ignore = "two frames of board time, twenty-four seconds: cargo test --test simpletv_netlist -- --ignored --nocapture"]
fn the_sync_program_makes_a_frame() {
    use muir::xbus::XbusMaster;

    let n = simpletv();
    let mut b = XbusMaster::new(&n, 0);
    let window = 2 * LINE_NS * LINES_A_FRAME;
    let until = b.now + window;
    let (hsync, vsync) = sync_falls(&mut b, until);
    eprintln!("over {window} ns, two frame times:");
    eprintln!("  {} HSYNC falls, {} VSYNC falls at {vsync:?}", hsync.len(), vsync.len());

    assert!(
        vsync.len() >= 2,
        "two frame times should carry at least two VSYNC falls, saw {}: {vsync:?}",
        vsync.len()
    );
    let frames: Vec<u64> = vsync.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(
        frames.iter().all(|&f| f == LINE_NS * LINES_A_FRAME),
        "a frame is {LINES_A_FRAME} lines of {LINE_NS} ns; saw {frames:?}"
    );
    let lines = hsync.iter().filter(|&&t| t > vsync[0] && t <= vsync[1]).count() as u64;
    assert_eq!(lines, LINES_A_FRAME, "lines between one VSYNC and the next");

    // The same frame again, counted as the monitor would take it.
    let mut b = XbusMaster::new(&n, 0);
    let dots = frame_dots(&mut b);
    let mut how_many: BTreeSet<(u32, usize)> = BTreeSet::new();
    for &d in &dots {
        let n = dots.iter().filter(|&&x| x == d).count();
        how_many.insert((d, n));
    }
    eprintln!("  dots a line, and lines at that count: {how_many:?}");
    assert_eq!(dots.len(), LINES_A_FRAME as usize, "lines in the frame");
    assert_eq!(
        dots.iter().filter(|&&d| d == ACTIVE_DOTS).count(),
        ACTIVE_LINES,
        "lines of exactly {ACTIVE_DOTS} dots"
    );
    assert_eq!(dots.iter().filter(|&&d| d == 0).count(), BLANKED_LINES, "lines blanked end to end");
}

/// **The board drives the bus only to read.** `READ` enables the read
/// register onto the bus: it is pin 18 of the 25LS2539 at RAMCAS 0C12,
/// which makes `-MAP RD0` and `-MAP RD1`, and an input of both 74S00 gates
/// at 0E13 that make `-XDRIVE`. It is `-XBUS.WR` inverted, and the sheets
/// write that signal `-WRITE` where the 74S04 at 0F10 makes it and `READ`
/// where these use it: one wire, two names, which MIT's wire list joins on
/// the LISPM TV (`lmtv4b.wlr` has the seven pins on one net) and nothing
/// joins on this board, which has no wire list. Unjoined, `READ` floats
/// high, and the board answers a **write** by driving its own data at the
/// master.
#[test]
fn the_board_drives_the_bus_only_to_read() {
    use muir::part::Level;
    use muir::simpletv::BUFFER;
    use muir::xbus::XbusMaster;

    let n = simpletv();
    let mut b = XbusMaster::new(&n, 0);
    b.request(BUFFER + 5, Some(0x1234_5678));
    let until = b.now + 100;
    b.run(until);
    assert_eq!(b.level("READ"), Level::Low, "a write is not a read");
    // The 74S08 at 0D10 is open collector: not driving is letting go.
    assert_ne!(b.level("-XDRIVE"), Level::Low, "and the board leaves the bus to the master");
    let until = b.now + 1_000;
    b.run(until);
    b.release();
    let until = b.now + 600;
    b.run(until);
    b.request(BUFFER + 5, None);
    let until = b.now + 100;
    b.run(until);
    assert_eq!(b.level("READ"), Level::High, "a read is one");
    assert_eq!(b.level("-XDRIVE"), Level::Low, "and the board drives the bus");
}

/// **What the board takes to answer an Xbus cycle is not a number.**
///
/// The frame buffer is one RAM with three users --- the processor over the
/// Xbus, the refresh, and the video shifter reading the picture out --- and
/// the board hands it to them a slot at a time. `PROC CYC` at NRAADR 0D12
/// is what switches the RAM address multiplexers at 0A11-0B12 between the
/// processor's `RAM ADR IN` and the shifter's `TVMA`, so a processor cycle
/// waits for a slot of its own.
///
/// **The slots are every 500 ns**, which is `cadrtv/lmtv.order`'s "an
/// instruction every 32 bits of video ... or roughly every 1/2
/// microsecond", **and every acknowledgement this board ever gives lands
/// on that grid**, 257 ns into each one, counted from power-on. A request
/// is answered at the **second** slot strictly after it: the slot it
/// arrives in sees it, and the next one serves it. So the answer takes
/// between 501 and 1000 ns depending on where in the grid the request
/// falls, and a single figure for it is a sample rather than a constant.
///
/// **The refresh takes one slot a line.** `-REFRESH CYC` is low for
/// exactly one slot, from 16,757 ns and every 16,000 after --- a line of
/// the raster, [`LINE_NS`] --- and a cycle that would have been served
/// then waits one slot more. **Every 16,000 while the frame is blank**:
/// once the picture is on, the refresh competes with the shifter as the
/// processor does and its own spacing slides, measured at 15,500 as well
/// as 16,000 past 864,757 ns. So this is the blanking's rule too, and the
/// window watched here ends well inside it.
///
/// **And from the first line of the picture the shifter takes slots too,
/// which this does not model.** The rule above is exact through the 54
/// lines of the frame that carry no dots (966 lines against the 912 that
/// carry 768 dots, both counted by `the_sync_program_makes_a_frame`), and
/// the first cycle it fails to predict is in line 54. That is measured and
/// asserted here, so that this test says where the rule ends as well as
/// where it holds: a timing twin for this board in `rtl` --- issue 66 ---
/// needs the shifter's own fetches, and the grid and the refresh are not
/// enough for one.
#[test]
fn the_board_answers_on_its_own_slots() {
    use muir::part::Level;
    use muir::xbus::XbusMaster;

    /// The slots the RAM is handed out in, and the offset into each at
    /// which the board acknowledges. Measured here.
    const SLOT_NS: u64 = 500;
    const ACK_INTO_SLOT_NS: u64 = 257;
    /// One line of the raster: [`muir::simpletv::FRAME_NS`] over the 966
    /// lines `the_sync_program_makes_a_frame` counts.
    const LINE_NS: u64 = 16_000;
    /// When the first refresh cycle begins, and every [`LINE_NS`] after
    /// it. The slot it takes is the one that would have been answered
    /// [`SLOT_NS`] later.
    const FIRST_REFRESH_NS: u64 = 16_757;
    /// The first line of the picture: 966 lines less the 912 that carry
    /// dots.
    const PICTURE_LINE: u64 = 54;

    let n = simpletv();

    // The refresh, off the board rather than inferred: `-REFRESH CYC` low
    // for one slot, once a line.
    let mut b = XbusMaster::new(&n, 0);
    let cyc = b.net("-REFRESH CYC");
    let (mut last, mut edges) = (b.chip.net(cyc), Vec::new());
    while b.now < 100_000 {
        let next = b.chip.next_tap().filter(|&t| t > b.now).unwrap_or(b.now + 1);
        b.run(next.min(100_000));
        let now = b.chip.net(cyc);
        if now != last {
            edges.push((b.now, now));
            last = now;
        }
    }
    let falls: Vec<u64> = edges.iter().filter(|(_, l)| *l == Level::Low).map(|&(t, _)| t).collect();
    let rises: Vec<u64> =
        edges.iter().filter(|(_, l)| *l == Level::High).map(|&(t, _)| t).collect();
    assert_eq!(falls[0], FIRST_REFRESH_NS, "when the first refresh cycle begins");
    assert!(falls.windows(2).all(|w| w[1] - w[0] == LINE_NS), "one refresh a line: {falls:?}");
    assert!(
        falls.iter().zip(&rises).all(|(f, r)| r - f == SLOT_NS),
        "and it is one slot long: {edges:?}"
    );

    // Where the acknowledgement lands, for a write and for a read, at
    // every phase of the grid the requests happen to fall on.
    // The acknowledgements the refresh takes: the slot it holds the RAM
    // for would have been answered at its end.
    let blocked = FIRST_REFRESH_NS + SLOT_NS;
    let refresh_slot = |s: u64| s >= blocked && (s - blocked).is_multiple_of(LINE_NS);
    let predict = |t: u64| {
        let mut seen = (t / SLOT_NS) * SLOT_NS + ACK_INTO_SLOT_NS;
        if seen <= t {
            seen += SLOT_NS;
        }
        let serve = seen + SLOT_NS;
        if refresh_slot(serve) { serve + SLOT_NS } else { serve }
    };
    let mut b = XbusMaster::new(&n, 0);
    let (mut blanking, mut first_miss) = (0, None);
    // Two lines past the first of the picture, which is where the rule is
    // to stop holding.
    while b.now < (PICTURE_LINE + 2) * LINE_NS {
        b.run(b.now + 1 + (b.now * 53) % (SLOT_NS - 1));
        let t0 = b.now;
        let write = t0.is_multiple_of(2);
        b.request(muir::simpletv::BUFFER + 21491, write.then_some(0o525252));
        let mut guard = 0;
        while !b.acked() {
            let next = b.chip.next_tap().filter(|&t| t > b.now).unwrap_or(b.now + 1);
            b.run(next);
            guard += 1;
            assert!(guard < 100_000, "the board never acknowledged a cycle from {t0}");
        }
        let ack = b.now;
        assert_eq!(
            ack % SLOT_NS,
            ACK_INTO_SLOT_NS,
            "every acknowledgement is on the grid: {ack} for a request at {t0}"
        );
        assert!(
            (SLOT_NS + 1..=2 * SLOT_NS).contains(&(ack - t0)) || refresh_slot(ack - SLOT_NS),
            "between one slot and two, or three across a refresh: {} from {t0}",
            ack - t0
        );
        if ack == predict(t0) {
            blanking += u64::from(t0 / LINE_NS < PICTURE_LINE);
        } else if first_miss.is_none() {
            first_miss = Some((t0, t0 / LINE_NS, ack, predict(t0)));
        }
        b.run(b.now + XbusMaster::RELEASE_NS);
        b.release();
    }
    let (at, line, ack, wanted) =
        first_miss.expect("the shifter takes slots once the picture is on");
    eprintln!(
        "{blanking} cycles through the blanking predicted; first miss at {at}, line {line}:          acknowledged at {ack} where the grid and the refresh alone say {wanted}"
    );
    assert!(blanking > 500, "the blanking was sampled: {blanking} cycles");
    assert_eq!(line, PICTURE_LINE, "the rule holds until the picture starts, and stops there");
}

/// **The board runs MIT's sync program out of reset**, which nothing here
/// checked: the raster test above walks `cpt.prom` by hand, and a hand walk
/// says what the program means rather than that the board executes it.
///
/// Out of reset the 74LS273 at NTVINC 0A07 is cleared, so its `-SYNC PROM
/// ENB` on pin 19 is low: the 74S472 at NSYRAM 0C05 is enabled and the
/// board fetches MIT's program. The eight 2147s that hold a program the
/// software loads instead take their chip select from `SYNC PROM ENB`, the
/// **other** net --- the 74S37O at NTVINC 0C11 inverts the first into the
/// second --- so with the PROM enabled the RAMs are deselected, which is
/// the state this checks.
///
/// **`SYNC PROM ENB` has no pull-up and does not need one.** Its only
/// driver is that open-collector inverter, so the net is either pulled low
/// or released, and a released open collector is `Level::Z`, which every
/// TTL input reads as a one ([`muir::part::Level::read_open`]). A pull-up
/// would give exactly that. The net is therefore `Z` for the whole of this
/// test and the RAMs stay deselected, which is right rather than a float
/// that happens not to bite.
///
/// What this does **not** say is what the board does in a booted machine.
/// `-SYNC PROM ENB` is a software-written register bit, so microcode that
/// writes it moves the board on to the program in its RAMs, and what runs
/// then is whatever was loaded there.
#[test]
fn the_sync_prom_is_fetched_out_of_reset() {
    use muir::part::Level;
    use muir::xbus::XbusMaster;

    let n = simpletv();
    let id = |name: &str| n.by_name_id(&format!("'{name}'")).or_else(|| n.by_name_id(name));
    let id = |name: &str| id(name).unwrap_or_else(|| panic!("no net {name}"));
    let address: Vec<_> = (0..9).map(|k| id(&format!("SYNC ADR {k}"))).collect();
    let data: Vec<_> = (0..8).map(|k| id(&format!("SYNC {k}"))).collect();

    // Who is on the RAMs' chip select: the eight 2147s that read it and the
    // one open-collector inverter that drives it, and nothing else. That is
    // the whole of the net, and it is why `Z` below is the driver releasing
    // rather than the driver missing.
    let on: BTreeSet<String> = n
        .parts
        .iter()
        .flat_map(|p| p.pins.iter().map(move |&(pin, net)| (p, pin, net)))
        .filter(|&(_, _, net)| net == id("SYNC PROM ENB"))
        .map(|(p, pin, _)| format!("{} {} p{pin}", p.reference, p.kind))
        .collect();
    let want: BTreeSet<String> = (1..=4)
        .flat_map(|k| [format!("0A0{k} 2147 p10"), format!("0B0{k} 2147 p10")])
        .chain(["0C11 74S37O p6".to_string()])
        .collect();
    assert_eq!(on, want, "the chip select's own parts");

    let mut b = XbusMaster::new(&n, 0);
    assert_eq!(b.chip.net(id("-SYNC PROM ENB")), Level::Low, "the PROM is enabled out of reset");

    let word = |b: &XbusMaster, nets: &[netlist::NetId]| -> Option<u32> {
        nets.iter().enumerate().try_fold(0, |w, (k, &net)| {
            b.chip.net(net).read_open(true).map(|bit| w | (bit as u32) << k)
        })
    };
    // MIT's own program, to compare what comes off the PROM against.
    let image = muir::prom::parse_mit(include_str!("../mit/cadrtv/cpt.prom")).expect("cpt.prom");
    let (mut addresses, mut words, mut codes) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    // A millisecond, sampled every 250 ns, which is finer than the
    // program's instruction every 500 ns. A frame is sixteen of these, so
    // what this sees is the start of the program rather than all 297
    // words of it.
    for k in 1..=4_000u64 {
        b.run(k * 250);
        assert_eq!(b.chip.net(id("SYNC PROM ENB")), Level::Z, "the RAMs stay deselected");
        let at = word(&b, &address).expect("the address is driven");
        let w = word(&b, &data).expect("the PROM answers");
        // **The word is MIT's own at that address**, which is what makes
        // this the board running `cpt.prom` rather than the board running
        // something. An address line that did not reach the PROM would
        // still give a program, and would give the wrong words.
        assert_eq!(
            Some(w as u8),
            image.get(at as usize).copied(),
            "address {at} answered {w:#04x}"
        );
        addresses.insert(at);
        words.insert(w);
        codes.insert((w >> 4) & 3);
    }

    // The program is fetched rather than the address standing still, and
    // what comes back is a program rather than one word over and over.
    assert!(addresses.len() > 64, "the sync counter walks: {} addresses", addresses.len());
    assert!(addresses.contains(&4), "and starts where the program does");
    assert!(words.len() > 8, "the PROM answers with a program: {} words", words.len());
    // `lmtv.order` gives the slot owner as bits 5 and 4 of the instruction,
    // and MIT's program uses all four: the processor, the refresh, the
    // display and the idle slot.
    assert_eq!(codes, BTreeSet::from([0, 1, 2, 3]), "every slot owner appears");
}

/// **The board answers its own control registers, and not only its frame
/// buffer.** `cadrtv/lmtv.order` runs them from `173777x0` with `x` 6,
/// which is [`muir::simpletv::CONTROL`]; the 25LS2521 comparators at XBADR
/// match `ADR3..21` against the `DEVADR` straps to decide that a cycle is
/// the control block's, and the 74S138 at NXBCTL 0F13 picks the register
/// out of `ADR0..2`.
///
/// **This is issue 60.** `ADR BANK SEL` is address bit 15 off the bus, not
/// a strap --- `crate::netlist`'s NRAADR page joins it to `ADR15`, which
/// the 74LS240 at XBADR 0F17 drives --- and `xbus::straps` drove it low as
/// well. Two drivers on one bit, agreeing at idle and fighting whenever
/// the bus put bit 15 the other way, which a cycle to the control block
/// does: `ADR15` went unknown, the comparator could not match `DEVADR 15`,
/// and the board answered nothing here at all. The microcode's `INTRX0`
/// reads the TV control register, finds the vertical flag set and writes
/// it back to clear it, and `muir --chip` halted on that write at
/// 2,340,964 microcycles.
///
/// So the frame buffer is checked beside the registers: it was answered
/// throughout, because its own comparator at 0F22 matches `MAPADR 16..21`
/// and bit 15 is no part of it. A test on the buffer alone saw nothing
/// wrong for as long as this bug existed.
#[test]
fn the_board_answers_its_control_registers() {
    use muir::xbus::XbusMaster;

    let n = simpletv();
    // Long enough that a board which never answers has plainly not: the
    // bus interface gives a device 4.7 to 5.5 us before it calls the
    // address missing, `busint::TIMEOUT_NS`.
    const PATIENCE_NS: u64 = 20_000;
    let answer = |addr: u32, write: Option<u32>| -> Option<u64> {
        let mut b = XbusMaster::new(&n, 0);
        b.run(3_000);
        let at = b.now;
        b.request(addr, write);
        while b.now < at + PATIENCE_NS {
            let next = b.chip.next_tap().filter(|&t| t > b.now).unwrap_or(b.now + 1);
            b.run(next.min(at + PATIENCE_NS));
            if b.acked() {
                return Some(b.now - at);
            }
        }
        None
    };
    for k in 0..muir::simpletv::CONTROL_WORDS {
        for write in [None, Some(0o525252)] {
            let addr = muir::simpletv::CONTROL + k;
            let took = answer(addr, write).unwrap_or_else(|| {
                panic!(
                    "the board never answered {addr:o}, register {k} of its control block, \
                     in {PATIENCE_NS} ns"
                )
            });
            assert!(took < 1_000, "register {k} answered in {took} ns");
        }
    }
    // And the frame buffer still does, on its own slower path: the RAM is
    // handed out a slot at a time and `the_board_answers_on_its_own_slots`
    // is what says how.
    let buffer = muir::simpletv::BUFFER + 21_491;
    assert!(answer(buffer, None).is_some(), "the frame buffer answered a read");
    assert!(answer(buffer, Some(0o525252)).is_some(), "the frame buffer answered a write");
}

/// **What the board takes to answer in the picture**, where
/// [`the_board_answers_on_its_own_slots`] stops. `rtl`
/// charges every device [`muir::busint::IDEAL_DEVICE_NS`], which is zero,
/// so a display access costs it the protocol's deskew and nothing else.
/// Issue 66 asked which of two figures was wrong; `cadr1/xspec.text.3`
/// cannot say, constraining the master at every turn and giving a slave
/// **no response time at all**. So the board's own answer has to be
/// measured.
///
/// **The grid is not this test's finding.**
/// [`the_board_answers_on_its_own_slots`] established it first: the RAM is
/// handed out in 500 ns slots, every acknowledgement lands 257 ns into one,
/// and a request is served at the second slot strictly after it. That test
/// measures **through the blanking**, and says where it stops --- from the
/// first line of the picture the shifter takes slots of its own and the
/// grid and the refresh alone stop predicting.
///
/// **This one measures the picture**, the other 912 lines of the 966, and
/// finds two things.
///
/// **The grid survives it.** Every acknowledgement is still 257 ns into a
/// slot on a picture line, so a twin can keep the grid whatever else it
/// has to do.
///
/// **The second-slot rule does not.** Sampled across picture lines at every
/// phase, requests are still served at the second slot most of the time and
/// wait a third or a fourth often enough to matter --- the shifter's own
/// fetches taking the slot the processor wanted. **How often is not the
/// board's number**: the same twenty lines gave 41 late of 282 and 81 of
/// 167 depending only on when the master chose to ask. So a twin charging
/// two slots is right most of the time and **cannot be made right by a
/// constant**, which is the answer issue 66 was looking for; what it needs
/// is the shifter.
///
/// **Inside one line the answer is a sawtooth**, `base + (next slot - t)`
/// with the base one slot: walking the phase 100 ns a step gives
/// `757, 657, 557, 957, 857`, -100 each step and +400 across the boundary.
/// **A read and a write share that base.** Each sample carries the time
/// its request went out, and for a sawtooth `answer + request` is the
/// grid's own offset modulo the period --- so the directions can be
/// compared without knowing where the grid sits, and both give 257, as do
/// two read sweeps on two different lines.
///
/// The control registers do not wait for a slot --- they address no RAM
/// --- but they are not flat either: about 130 to 185 over the phase of
/// the board's own clock, the shape the memory board's 470--500 has.
///
/// **Five ways this measurement lies, all of them met here.**
///
/// - **Back-to-back accesses look constant.** `XbusMaster::cycle` advances
///   the clock by the cycle's own duration, so a request issued at
///   `b.now`, or on a stride shorter than a cycle, always lands at the same
///   phase relative to the last one. That reads as a constant and is an
///   artefact of the stride. **The stride must exceed the longest cycle**,
///   which is why it is 2,100 ns here. What walks the phase is its
///   remainder over the slot, 100 ns a step, not the stride itself.
/// - **A blanked line fetches nothing.** The board runs MIT's sync program
///   out of reset (`the_sync_prom_is_fetched_out_of_reset`) and blanks 54
///   lines end to end before the first unblanked one
///   (`the_sync_program_makes_a_frame`), so a sweep in the first
///   microseconds is of a board that is scanning and fetching nothing.
///   Contention cannot appear there whatever the stride.
/// - **A read is not a write --- until both are swept.** Sweeping one says
///   nothing about the other, and a three-cycle sweep said they differed by
///   hundreds of nanoseconds. Swept properly they do not differ at all:
///   same base, same period. The trap is real; the answer it seemed to
///   give was not.
/// - **A stride longer than one cycle is not a stride longer than three.**
///   Issuing a read, a control access and a write at each step puts the
///   second and third back to back behind the first, so only the first is
///   ever on the grid --- the artefact above, surviving inside the fix for
///   it. That is what made the control registers look constant. **One
///   access per step**, and one sweep per kind.
/// - **`XbusMaster::cycle` watches every 5 ns**, so what it returns is the
///   true figure rounded up to the next 5. Against a grid whose offset is
///   257 that reads as 260 and puts the base at 560 rather than 557: a
///   shape it can measure, a grid it cannot. Every access here waits on the
///   netlist's own taps instead, as [`the_board_answers_on_its_own_slots`]
///   does.
///
/// And one more, which is not a lie but a limit: an access takes up to
/// 2 us where a line's unblanked stretch is 12, so a request that starts
/// while the board is fetching can finish after it has stopped. Blanking
/// is checked **after** each access as well as before, and a sample that
/// spans the end of a line is dropped rather than explained.
///
/// Each of those produced a different wrong answer before this one, and
/// each was flat or varying for a reason that had nothing to do with the
/// board.
#[test]
fn what_the_board_takes_to_answer_in_the_picture() {
    use muir::part::Level;
    use muir::xbus::XbusMaster;

    /// The slot the RAM is handed out in and the offset into it at which
    /// the board acknowledges, both measured by
    /// [`the_board_answers_on_its_own_slots`] through the blanking. What
    /// is checked here is that they still hold in the picture.
    const SLOT_NS: u64 = 500;
    const ACK_INTO_SLOT_NS: u64 = 257;
    /// The first line of the picture: 966 lines less the 912 that carry
    /// dots, as `the_sync_program_makes_a_frame` counts them.
    const PICTURE_LINE: u64 = 54;
    /// Between one request of a sweep and the next. Long enough that no
    /// access overlaps the following one --- the longest measured here is
    /// 1,981 ns --- and its remainder over [`SLOT_NS`] is the phase the
    /// request gains each step, 100 ns. A line carries 768 dots, 12 us,
    /// and holds five or six accesses at this stride, so five samples walk
    /// 400 ns of the 500 and a sweep may or may not cross a boundary; the
    /// three sweeps together must. A finer walk needs more lines than one,
    /// and the grid's phase is a property of the board rather than of the
    /// line, so that would be a different measurement, not a better one.
    const STRIDE_NS: u64 = 2_100;

    /// One whole access on the netlist's own taps: when the request went
    /// out, and the instant of the acknowledgement. The request is lifted
    /// [`XbusMaster::RELEASE_NS`] after the acknowledgement and the bus
    /// left alone from there, as [`the_board_answers_on_its_own_slots`]
    /// does --- not `XbusMaster::cycle`'s further 600 ns, which at the
    /// longest answers would carry `b.now` past the next stride point and
    /// quietly cost the sweep its uniform step.
    fn access(b: &mut XbusMaster, addr: u32, write: Option<u32>) -> (u64, u64) {
        let t = b.now;
        b.request(addr, write);
        let mut guard = 0;
        while !b.acked() {
            let next = b.chip.next_tap().filter(|&x| x > b.now).unwrap_or(b.now + 1);
            b.run(next);
            guard += 1;
            assert!(guard < 100_000, "the board never acknowledged a cycle from {t}");
        }
        let ack = b.now;
        b.run(b.now + XbusMaster::RELEASE_NS);
        b.release();
        (t, ack)
    }

    let n = simpletv();
    let mut b = XbusMaster::new(&n, 0);
    let buffer = muir::simpletv::BUFFER + 0o51763;
    let control = muir::simpletv::CONTROL;
    let blanking = b.net("BLANKING");

    // **A line of its own for each sweep.** A sweep stops when the line
    // does, so `b.now` is then inside the blanked stretch that stopped it
    // and the next sweep would take no samples at all.
    let unblanked = |b: &mut XbusMaster| {
        let deadline = b.now + 2 * LINE_NS * LINES_A_FRAME;
        while b.chip.net(blanking) != Level::Low && b.now < deadline {
            b.run(b.now + 500);
        }
        assert!(b.now < deadline, "no unblanked line in two frames");
    };

    // **One access per step**, on a stride longer than the longest of them,
    // stopping when the line does: the fetch's phase is continuous while
    // the board is fetching, and a blanked stretch fetches nothing, so a
    // sample the far side of one is not on the same ramp.
    let sweep = |b: &mut XbusMaster, write: Option<u32>| -> Vec<(u64, u64)> {
        let origin = b.now + 500;
        let mut out = Vec::new();
        for step in 0..8u64 {
            let at = origin + step * STRIDE_NS;
            if b.now < at {
                b.run(at);
            }
            if b.chip.net(blanking) != Level::Low {
                break;
            }
            let (t, ack) = access(b, buffer, write);
            // **Blanking is checked after as well as before.** An access
            // takes up to 2 us and a line's unblanked stretch is 12, so a
            // request issued while the board is fetching can finish after
            // it has stopped --- and that sample is not on the ramp.
            if b.chip.net(blanking) != Level::Low {
                break;
            }
            out.push((t, ack));
        }
        out
    };
    unblanked(&mut b);
    let read = sweep(&mut b, None);
    unblanked(&mut b);
    let write = sweep(&mut b, Some(0o525252));
    unblanked(&mut b);
    let read_again = sweep(&mut b, None);

    // **A sawtooth, and this is what says the mechanism is the slot.** The
    // stride is 2,100 ns and the slot 500, so each request lands 100 ns
    // later in the slot than the last and waits 100 ns less --- until it
    // crosses a boundary and waits a whole slot more. So consecutive
    // samples differ by -100, or by +400, and by nothing else.
    let walk = -(STRIDE_NS as i64 % SLOT_NS as i64);
    let mut crossings = 0;
    for (what, samples) in [("read", &read), ("write", &write), ("read again", &read_again)] {
        let ns: Vec<u64> = samples.iter().map(|&(t, ack)| ack - t).collect();
        let line = samples[0].0 / LINE_NS;
        assert!(line >= PICTURE_LINE, "{what} swept line {line}, which carries no dots");
        assert!(ns.len() >= 5, "too few {what} samples inside one line: {ns:?}");
        let steps: Vec<i64> = ns.windows(2).map(|w| w[1] as i64 - w[0] as i64).collect();
        for (k, &d) in steps.iter().enumerate() {
            assert!(
                d == walk || d == walk + SLOT_NS as i64,
                "{what} sample {k} moved {d} ns; a slot sawtooth moves {walk} or {}: {ns:?}",
                walk + SLOT_NS as i64
            );
        }
        // Whether *this* sweep crossed a slot boundary is where in the
        // slot its line happened to start: five samples walk 400 ns of a
        // 500 ns slot, so a sweep can sit inside one. The crossing is
        // required of the three together, below.
        let jumps = steps.iter().filter(|&&d| d != walk).count();
        crossings += jumps;

        // **The sawtooth as an equation, and this is the whole claim.**
        // `answer = base + ((B - t) mod P)` for a grid at offset `B`, so
        // `answer + t` --- the acknowledgement itself --- is congruent to
        // `B` modulo `P` for every sample, whatever the stride and whatever
        // boundaries it crosses. The step check above assumes a uniform
        // stride and can only be read off a picture; this holds pointwise,
        // and it is the grid the blanking test measured.
        let residues: Vec<u64> = samples.iter().map(|&(_, ack)| ack % SLOT_NS).collect();
        assert!(
            residues.iter().all(|&r| r == ACK_INTO_SLOT_NS),
            "{what} is off the grid on line {line}: {residues:?}"
        );
        eprintln!("{what} on line {line}: {ns:?}, {jumps} crossings, {walk} ns a step");
    }

    // That check, made of all three sweeps, is also what says **the grid
    // keeps its offset from one line to the next**: the sweeps are on three
    // different lines and every acknowledgement in them is at the same 257.
    // A line is 16,000 ns and a slot 500, so a line is exactly 32 slots, and
    // the grid crosses the blanked stretch between sweeps without moving.
    assert!(crossings > 0, "no sweep crossed a slot boundary, so the slot is not shown");

    // **A read and a write share one base, and that was worth sweeping both
    // to find out.** With the grid common the answers differ by exactly
    // what the bases do; congruent is not equal, so the pooled span settles
    // the rest --- every sample of every sweep inside one slot means the
    // bases coincide rather than sitting a slot apart.
    let all: Vec<u64> =
        [&read, &write, &read_again].iter().flat_map(|s| s.iter().map(|&(t, a)| a - t)).collect();
    let (lo, hi) = (*all.iter().min().expect("samples"), *all.iter().max().expect("samples"));
    assert!(
        hi - lo < SLOT_NS,
        "the sweeps span {} ns, so a base is a whole slot out: {lo} to {hi}",
        hi - lo
    );
    // And the phase really was walked: a sweep that never moved would pass
    // every check above and show nothing.
    assert!(hi - lo > 3 * walk.unsigned_abs(), "the phase barely moved: {lo} to {hi}");
    assert!(hi < muir::busint::TIMEOUT_NS, "{hi} ns is longer than the bus waits");
    eprintln!("read and write share one base; all {} samples in {lo}-{hi} ns", all.len());

    // **And now the picture at every phase**, twenty lines of it, stepping
    // by `1 + (now * 53) mod 499` as the blanking test does so that no
    // phase of the grid is favoured. Two questions: does the grid hold, and
    // does the second-slot rule hold with it.
    let until = b.now + 20 * LINE_NS;
    let (mut served, mut beyond, mut worst) = (0u64, 0u64, 0u64);
    while b.now < until {
        b.run(b.now + 1 + (b.now * 53) % (SLOT_NS - 1));
        let (t, ack) = access(&mut b, buffer, None);
        assert_eq!(
            ack % SLOT_NS,
            ACK_INTO_SLOT_NS,
            "off the grid in the picture: acknowledged at {ack} for a request at {t}"
        );
        let took = ack - t;
        served += 1;
        worst = worst.max(took);
        beyond += u64::from(took > 2 * SLOT_NS);
    }
    eprintln!(
        "{served} requests over 20 picture lines, {beyond} past the second slot, worst {worst} ns"
    );
    assert!(served > 100, "the picture was sampled: {served} requests");
    // **The shifter takes slots, and this is what says a constant will not
    // do.** `the_board_answers_on_its_own_slots` predicts the second slot
    // from the grid and the refresh alone and is exact through the
    // blanking; here that rule fails, so the twin issue 66 wants needs the
    // shifter's own fetches and cannot be a number.
    //
    // **How often it fails is not asserted, because it is not the board's
    // number alone.** The same twenty lines gave 41 of 282 when the master
    // let go at the acknowledgement and 81 of 167 when it idled a further
    // 600 ns first: the same board, a different set of instants asked. What
    // the board owns is that the rule fails at all and by how much at
    // worst; the rate belongs to whoever is driving the bus.
    assert!(beyond > 0, "nothing waited past the second slot in the picture");
    assert!(worst <= 4 * SLOT_NS, "an answer took {worst} ns, more than four slots");

    // The control registers address no RAM and wait for no slot.
    let mut control_ns = Vec::new();
    for k in 0..8 {
        b.run(b.now + 500 + k * 5);
        let (t, ack) = access(&mut b, control, None);
        control_ns.push(ack - t);
    }
    // **Not flat --- but not the buffer's shape either.** It tracks the
    // phase of the board's own clock over tens of nanoseconds, the way the
    // memory board's 470--500 does, and never by a whole slot. Saying
    // "flat" here was an artefact of measuring it back to back behind
    // another access.
    let (lo, hi) =
        (*control_ns.iter().min().expect("samples"), *control_ns.iter().max().expect("samples"));
    assert!(
        hi - lo < SLOT_NS,
        "the control registers wait for a slot, which they should not: {control_ns:?}"
    );
    eprintln!("control {lo}-{hi} ns over {} offsets", control_ns.len());
}
