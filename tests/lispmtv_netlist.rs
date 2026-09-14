// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The LISPM TV as a netlist: `data/LISPMTV.netlist`, the 25 `cadrtv`
//! pages `lmtv4b.fil` lists, through `tools/lispmtv-netlist.sh`.
//!
//! This is the four- and eight-bit display that replaced the SIMPLE TV in
//! December 1980, and the board `--tv-board lispm-tv` puts on the Xbus.
//! Unlike the SIMPLE TV it has MIT's own wire list, `cadrtv/lmtv4b.wlr` of
//! 7 December 1980, and the section census `lmtv4b.wls` beside it, both
//! made by MIT's tooling from these drawings the same day; the netlist is
//! held to both, as the memory board and the disk controller are.

use std::collections::{BTreeMap, BTreeSet};

use muir::netlist::{self, Netlist};
use muir::wirelist;

mod support;
use support::mit_text;

const LISPMTV: &str = include_str!("../data/LISPMTV.netlist");

fn lispmtv() -> Netlist {
    netlist::parse(LISPMTV).unwrap()
}

/// The shape of the board, pinned the way the other netlists are. Two of
/// the 25 pages carry no parts: `gen4b`, the customising jumpers, and
/// `xbus`, the backplane.
///
/// **The RAM count is the frame buffer again.** RAMA to RAMD carry sixteen
/// 2118s each, 16K by 1, and 64 by 16,384 bits is the same 32K words of 32
/// bits the SIMPLE TV holds --- the 2118 being the single-supply successor
/// of the 4116 in the same pins.
#[test]
fn parses_to_the_expected_shape() {
    let n = lispmtv();
    assert_eq!(n.pages.len(), 25, "{:?}", n.pages);
    assert_eq!(n.parts.len(), 265);
    assert_eq!(n.populated_pages().len(), 23);
    let rams = n.parts.iter().filter(|p| p.kind == "2118").count();
    assert_eq!(rams, 64, "four rows of sixteen 16K DRAMs");
    assert_eq!(rams * 16_384, muir::tv::BUFFER_WORDS as usize * 32);
}

/// **Every page is a LISPM TV page.** The same filename collision as the
/// SIMPLE TV's, from the other side: six of these names also exist in
/// `simple-tv/`, and the title block is what says which board a sheet is.
#[test]
fn every_page_is_a_lispm_tv_page() {
    let titles: Vec<&str> =
        LISPMTV.lines().filter_map(|l| l.strip_prefix("# title 1: ")).map(str::trim).collect();
    assert_eq!(titles.len(), 25, "one title block a page");
    assert!(titles.iter().all(|&t| t == "LISPM TV"), "a page of another board: {titles:?}");
}

/// Every part on the board is identified, and everything that computes
/// anything has a behaviour, the delay line and the oscillator can apart,
/// which `src/chip.rs` runs.
#[test]
fn every_part_is_identified() {
    let k = support::kinds(&lispmtv());
    assert!(k.unknown.is_empty(), "no pinout for {:?}", k.unknown);
    assert_eq!(k.silent, ["TD100", "TTLOSC"], "parts with no behaviour");
}

/// Two totem-pole outputs on one net is an electrical fault, so a wrongly
/// claimed output pin surfaces here.
#[test]
fn no_net_has_two_push_pull_drivers() {
    support::no_net_has_two_push_pull_drivers(&lispmtv());
}

/// **Every section MIT counted is in the netlist.** `lmtv4b.wls` is MIT's
/// own summary of the board by DIP type, read by `support::body_census`.
#[test]
fn the_census_matches_mits_own() {
    let text = mit_text(&["cadrtv", "lmtv4b.wls"]);

    let census = support::body_census(&text);
    assert!(census.len() > 30, "parsed {} bodies out of lmtv4b.wls", census.len());

    let n = lispmtv();
    let mut ours: BTreeMap<String, usize> = BTreeMap::new();
    for p in &n.parts {
        *ours.entry(p.kind.clone()).or_default() += 1;
    }
    let mut wrong = Vec::new();
    for (body, want) in &census {
        let got = ours.get(body).copied().unwrap_or(0);
        match body.as_str() {
            "BYPASS" => assert_eq!(got, 0, "bypass capacitors are not parts"),
            _ if got != *want => wrong.push(format!("{body}: MIT {want}, ours {got}")),
            _ => {}
        }
    }
    for body in ours.keys() {
        if !census.contains_key(body) {
            wrong.push(format!("{body}: not in MIT's census at all"));
        }
    }
    assert!(wrong.is_empty(), "sections disagree with lmtv4b.wls: {wrong:?}");
    eprintln!(
        "lmtv4b.wls counts {} sections in {} bodies",
        census.values().sum::<usize>(),
        census.len()
    );
}

/// **MIT's wire list for the board agrees with the netlist.** `lmtv4b.wlr`
/// (7 December 1980) is the list the board was wrapped from, pin by pin;
/// every wire on the 25 pages must be one net, and no net two wires.
#[test]
fn matches_mits_wire_list() {
    let n = netlist::parse_wired(LISPMTV).unwrap();
    let signals = support::wire_list(&n, &["cadrtv", "lmtv4b.wlr"]);
    let r = wirelist::compare(&n, &signals, |slot| format!("0{slot}"));
    support::report(&signals, &r);
    assert!(r.placed > 1500, "the wire list was read");
    assert!(
        r.missing.is_empty(),
        "the wire list places pins the netlist has not got: {:?}",
        r.missing
    );
    assert!(r.split.is_empty() && r.merged.is_empty(), "the wire list disagrees with the netlist");
}

/// **The board comes up on the bus, as far as it is known to.** Powered,
/// reset and left for two microseconds as the SIMPLE TV is --- the same
/// address straps, the same two PROM images, its own 64 MHz can --- no net
/// is unknown, and the mode register at `17377760` takes a write and reads
/// it back, and a frame-buffer word written reads back. This test pins
/// it.
#[test]
fn the_board_comes_up_on_the_bus() {
    use muir::part::Level;
    use muir::tv::{BUFFER, CONTROL, mode};
    use muir::xbus::XbusMaster;

    let n = lispmtv();
    let mut b = XbusMaster::new(&n, 0);
    let wired: BTreeSet<netlist::NetId> =
        n.parts.iter().flat_map(|p| p.pins.iter().map(|&(_, net)| net)).collect();
    let unknown: Vec<&str> =
        wired.iter().filter(|&&id| b.chip.net(id) == Level::X).map(|&id| n.net(id)).collect();
    assert!(unknown.is_empty(), "nets at unknown with the bus driven: {unknown:?}");

    let (took, _) = b.cycle(CONTROL, Some(mode::BOW));
    assert!(took < 1_000, "the mode register answered a write in {took} ns");
    let (took, word) = b.cycle(CONTROL, None);
    assert!(took < 1_000, "and a read in {took} ns");
    assert_eq!(word & mode::WRITABLE, mode::BOW, "and gave the bit back");

    // The frame buffer holds a word. Its 2118s are 4116s in a single-supply
    // package and stand in the part table as an alias of the 4116; a part
    // that answers to the 4116's pins and cycle must have its cells too.
    // `memory_words` once resolved the suffix but not the alias, so the
    // 2118 had none: every write went into nothing, every read gave `X`,
    // and the read register latched that as ones --- `0xffffffff` back for
    // any word.
    let (took_w, _) = b.cycle(BUFFER + 5, Some(0x1234_5678));
    let (took_r, word) = b.cycle(BUFFER + 5, None);
    eprintln!("frame buffer: write acknowledged in {took_w} ns, read in {took_r} ns");
    assert_eq!(word, 0x1234_5678, "the word written at 17000005 reads back");
}

/// **How long an instruction of the sync program takes in each clock
/// mode, measured on the board.** `lmtv.order` says "an instruction every
/// (32, 16, 8, 32) bits of video (indexed by Mode<1-0>) or roughly every
/// 1/2 microsecond" and names the modes CPT 64 MHz, Motorola 4408 32 MHz,
/// standard video 12 MHz and color 12 MHz; what that comes to in
/// nanoseconds is the clock PROM's business, `lmtv4b.prom`, and this
/// reads it off `HSYNC OUT` with `cpt.prom` running: 32 instructions a
/// line, so a line's period over 32 is the instruction's.
#[test]
fn an_instruction_of_the_sync_program_in_each_clock_mode() {
    use muir::part::Level;
    use muir::tv::CONTROL;
    use muir::xbus::XbusMaster;

    let n = lispmtv();
    for mode in 0..4u32 {
        let mut b = XbusMaster::new(&n, 0);
        b.cycle(CONTROL, Some(mode));
        let h = b.net("HSYNC OUT");
        let mut was = b.chip.net(h);
        let mut falls = Vec::new();
        let until = b.now + 400_000;
        while b.now < until && falls.len() < 6 {
            let tap = b.chip.next_tap().unwrap_or(until).clamp(b.now + 1, until);
            b.run(tap);
            let now = b.chip.net(h);
            if now != was {
                if now == Level::Low {
                    falls.push(b.now);
                }
                was = now;
            }
        }
        let periods: Vec<u64> = falls.windows(2).map(|w| w[1] - w[0]).collect();
        eprintln!(
            "mode {mode}: HSYNC OUT fell at {falls:?}; line periods {periods:?}; an instruction {:?} ns",
            periods.first().map(|p| *p as f64 / 32.0)
        );
        assert!(periods.len() >= 2, "mode {mode}: lines seen {falls:?}");
        assert!(
            periods.windows(2).all(|w| w[0] == w[1]),
            "mode {mode}: a steady line: {periods:?}"
        );
        assert_eq!(
            periods[0],
            32 * muir::tv::sync::INSTRUCTION_NS[mode as usize],
            "mode {mode}: 32 instructions a line"
        );
    }
}

/// The one part of that type at that place, as `tests/simpletv_netlist.rs`
/// finds one: every reference used below is unique on its page, and the
/// assertion says so.
fn at<'a>(n: &'a Netlist, page: &str, reference: &str, kind: &str) -> &'a muir::netlist::Part {
    let mut it =
        n.parts.iter().filter(|p| p.page == page && p.reference == reference && p.kind == kind);
    let part = it.next().unwrap_or_else(|| panic!("no {kind} at {page} {reference}"));
    assert!(it.next().is_none(), "more than one {kind} at {page} {reference}");
    part
}

/// The net on a pin, as MIT names it, with the quotes a name with a space
/// in it keeps in the netlist stripped off.
fn on<'a>(n: &'a Netlist, part: &muir::netlist::Part, pin: u8) -> &'a str {
    let &(_, net) = part.pins.iter().find(|&&(p, _)| p == pin).expect("that pin is wired");
    n.net(net).trim_matches('\'')
}

/// **Mode bit 7 is this board's PROM-mode bit, and it is the sync enable
/// read back.**
///
/// XBCTL 0F11 section A is the mode register's read buffer, a 74LS244
/// turned on by `-RD MODE`; its fourth input, pin 8, is the net
/// `-SYNC PROM ENB`, and its output pin 12 is `XDO 7`. `-SYNC PROM ENB` is
/// pin 19 of the 74LS273 at TVINC 0A07, the Q of the D that `XDI7` feeds
/// --- register 3's bit 7, `lmtv.order`'s "Sync Enable" --- so the bit is
/// one while the sync RAM is selected and zero while the 74S472 PROM is.
/// The 74S37 at SYNRAM 0C11 makes the complement, `SYNC PROM ENB`, which
/// is the 2141s' chip select, and the PROM's is `-SYNC PROM ENB` itself:
/// the RAM is in exactly when the bit reads one.
///
/// This is the bit ECO 2 of `cadrtv/lmtv.eco` grounds on the SIMPLE TV,
/// "the check if TV is in PROM mode (extant only on new TV boards)". Read
/// here through bus cycles, as the software would read it.
#[test]
fn the_prom_mode_bit_is_the_sync_enable_read_back() {
    use muir::tv::{CONTROL, mode};
    use muir::xbus::XbusMaster;

    let n = lispmtv();
    let buf = at(&n, "XBCTL", "0F11", "74LS244-A");
    assert_eq!(on(&n, buf, 1), "-RD MODE", "the buffer turns on for a mode read");
    // Datasheet pin pairs: 1A1..1A4 on 2/4/6/8, 1Y1..1Y4 on 18/16/14/12.
    assert_eq!(on(&n, buf, 8), "-SYNC PROM ENB", "bit 7 in");
    assert_eq!(on(&n, buf, 12), "XDO 7", "bit 7 out");
    let enable = at(&n, "TVINC", "0A07", "74LS273");
    assert_eq!(on(&n, enable, 18), "XDI7", "register 3's bit 7 in");
    assert_eq!(on(&n, enable, 19), "-SYNC PROM ENB", "and out again");

    let mut b = XbusMaster::new(&n, 0);
    let (_, word) = b.cycle(CONTROL, None);
    assert_eq!(word & mode::SYNC_PROM_ENABLE, 0, "the PROM is selected at power-on");

    b.cycle(CONTROL + 3, Some(0o200));
    // The 2141s' chip select is low with the enable set, which is the RAM
    // in and the 74S472 at SYNRAM 0C05 --- whose select is
    // `-SYNC PROM ENB` --- out.
    assert_eq!(b.level("SYNC PROM ENB"), muir::part::Level::Low, "the 2141s are selected");
    let (_, word) = b.cycle(CONTROL, None);
    assert_eq!(
        word & mode::SYNC_PROM_ENABLE,
        mode::SYNC_PROM_ENABLE,
        "with the sync enable set the bit reads one"
    );

    b.cycle(CONTROL + 3, Some(0));
    let (_, word) = b.cycle(CONTROL, None);
    assert_eq!(word & mode::SYNC_PROM_ENABLE, 0, "and zero with the PROM back in");
}

/// **The colour register, measured on the board**: `lmtv.order`'s
/// "173777x4 Color (write only), 15-8 Value to write into color map, 7-6
/// Select which color map (up to 4 channels), 3-0 Color (i.e. address
/// into color map)", page COLOR part by part.
///
/// The 74LS244 at 0D13, turned on by `-LOAD COLOR`, puts `XDI0..7` on
/// `COLOR 0..7`; the 74S241 at 0E09, whose two halves are enabled by `GND`
/// and `HI3` and so are never off, puts `XDI8..15` on
/// `COLOR VALUE 0..7`; and the 74S139 at 0E10, enabled by `-LOAD COLOR`
/// through the 74S32 at 0E11 when `-ACK WRITE` comes, decodes `XDI6` and
/// `XDI7` into `-LOAD COLOR 0`, `-LOAD COLOR 1` and `-LOAD COLOR 2`, its
/// fourth output unconnected. The map RAMs and their D-As are off the
/// board, past the paddle connections ECO 2 of `cadrtv/lmtv4b.eco`
/// rewires, so `COLOR`, `COLOR VALUE` and the strobe are the whole of what
/// the board does with the write.
///
/// **The SIMPLE TV does the same thing**, its `NRACOL` page being this one
/// part for part:
/// `tests/simpletv_netlist.rs::the_colour_register_strobes_one_map_here_as_well`
/// measures it there, and `src/tv.rs` writes the map on either board.
#[test]
fn the_colour_register_strobes_one_map_with_the_colour_and_the_value() {
    use muir::tv::CONTROL;
    use muir::xbus::XbusMaster;

    let n = lispmtv();
    let dec = at(&n, "COLOR", "0E10", "74S139");
    assert_eq!(on(&n, dec, 14), "XDI6", "the channel's low bit");
    assert_eq!(on(&n, dec, 13), "XDI7", "and its high bit");
    for (channel, pin) in [(0, 12), (1, 11), (2, 10)] {
        assert_eq!(on(&n, dec, pin), format!("-LOAD COLOR {channel}"));
    }
    assert!(on(&n, dec, 9).starts_with("NC#"), "the fourth channel goes nowhere");

    let mut b = XbusMaster::new(&n, 0);
    // (DPB value 1010 (DPB channel 0602 colour)), as `WRITE-COLOR-MAP` writes.
    for (value, channel, colour) in [(0o252u32, 0u32, 5u32), (0o123, 1, 0o17), (0o077, 2, 0)] {
        let (low, sampled) = support::colour_write(&mut b, value << 8 | channel << 6 | colour);
        let (on_colour, on_value) = sampled
            .unwrap_or_else(|| panic!("channel {channel}: no -LOAD COLOR n fell during the cycle"));
        eprintln!(
            "value {value:o} channel {channel} colour {colour:o}: \
             -LOAD COLOR {low:?} low, COLOR {on_colour:o}, COLOR VALUE {on_value:o}"
        );
        assert_eq!(low, vec![channel as usize], "one map's strobe, and one only");
        assert_eq!(on_colour & 0o17, colour as u64, "the colour is XDI3..0 on COLOR 3..0");
        assert_eq!(on_colour >> 6 & 3, channel as u64, "and the channel rides out on COLOR 7..6");
        assert_eq!(on_value, value as u64, "the value is XDI15..8 on COLOR VALUE 7..0");
    }

    // The fourth channel the field can name strobes nothing, and the
    // register answers all the same.
    let (low, _) = support::colour_write(&mut b, 0o377 << 8 | 3 << 6 | 5);
    assert!(low.is_empty(), "channel 3 decodes to the 74S139's unconnected output: {low:?}");
    let (took, _) = b.cycle(CONTROL + 4, Some(0));
    assert!(took < 1_000, "and the register acknowledges a write in {took} ns");
}
