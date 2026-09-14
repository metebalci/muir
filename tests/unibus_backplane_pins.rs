//! **The Unibus between the I/O board and the bus interface, pin by pin,
//! held to MIT's two wire lists** --- and the one wire they do not agree on.
//!
//! `muir::unibus::wire_pairs` joins the two boards by *name*: the bus
//! interface's `-UB MSYN` to the I/O board's `-MSYN*`, and so on. Each
//! board's wire list also says which backplane pin the wire sits on ---
//! `cadr1/busint.wlr` on its CUBUS page, `cadrio/iob.wlr` on its HEXSPC
//! page, both in DEC's SPC lettering, `EE1` for `MSYN` --- and a pairing
//! by name is only right if the two boards put the wire on the same pin.
//! So that is held here, for every pair, and it is how the boot line was
//! found to be the exception. MIT's own list for the backplane itself,
//! `cadr1/dubspc.wires`, is held to the same pins, and has neither of the
//! boot line's.

mod support;

use std::collections::BTreeMap;

use muir::netlist;
use muir::wirelist::Signal;
use support::{mit_text, wire_list};

/// The backplane pins a signal sits on: its connector pins whose name is a
/// DEC SPC pin, two letters and a row --- `EE1`, `CS2` --- as against a
/// cable header's `J08-12`.
fn backplane_pins(s: &Signal) -> Vec<String> {
    s.pins
        .iter()
        .filter(|p| p.body == "CON")
        .map(|p| p.location.clone())
        .filter(|l| {
            let b = l.as_bytes();
            b.len() == 3
                && b[0].is_ascii_uppercase()
                && b[1].is_ascii_uppercase()
                && b[2].is_ascii_digit()
        })
        .collect()
}

fn signal<'a>(list: &'a [Signal], name: &str) -> &'a Signal {
    list.iter()
        .find(|s| s.names.iter().any(|n| n == name))
        .unwrap_or_else(|| panic!("no wire {name}"))
}

fn the_two_lists() -> (Vec<Signal>, Vec<Signal>) {
    let busint = netlist::parse(include_str!("../data/BUSINT.netlist")).unwrap();
    let io = netlist::parse(include_str!("../data/CADRIO.netlist")).unwrap();
    (wire_list(&busint, &["cadr1", "busint.wlr"]), wire_list(&io, &["cadrio", "iob.wlr"]))
}

/// **Every wire `wire_pairs` joins is on the same backplane pin in both
/// lists.** Two things are not disagreements: the I/O board's request and
/// grant, `-BR*` and `BG.IN*`, reach the backplane through jumpers to
/// `-BR5*` and `BG5.IN*`, which are the records that carry the pin; and
/// the interface, being the arbiter, ties the grant's in and out pins,
/// `DP2` and `DR2`, so its record has two and the board's one is among
/// them.
#[test]
fn every_shared_unibus_wire_is_on_the_same_backplane_pin_on_both_boards() {
    let (busint, io) = the_two_lists();
    let mut checked = 0;
    for (interface, board) in muir::unibus::wire_pairs() {
        let board_record = match board.as_str() {
            "-BR*" => "-BR5*",
            "BG.IN*" => "BG5.IN*",
            other => other,
        };
        let a = backplane_pins(signal(&busint, &interface));
        let b = backplane_pins(signal(&io, board_record));
        assert_eq!(b.len(), 1, "{board_record}: one backplane pin, {b:?}");
        assert!(
            a.contains(&b[0]),
            "{interface} on {a:?} and {board} on {b:?} are one backplane wire"
        );
        checked += 1;
    }
    assert_eq!(checked, 42, "the pairs {}", checked);
}

/// **The boot line is the one wire the two boards put on different pins,
/// and this holds what the lists say rather than what the machine did.**
///
/// The I/O board decodes the keyboard's boot word itself (IOBCSR) and
/// puts `-BOOT*` on backplane pin `CP1` (`iob.wlr`, page HEXSPC). The bus
/// interface takes `-LM BOOT` on `CR1` (`busint.wlr`, page CUBUS) and
/// passes it, with no part on it, to cable header `J08-12`. `CP1` and
/// `CR1` are different pins, when every other shared wire above is on
/// the same one, and MIT's own list for the backplane has neither of
/// them: they are not bus strips and not in the grant chains, and the
/// list leaves device wiring to the hand (the test below). So either the
/// cage carried a hand wire from the I/O slot's `CP1` to the interface
/// slot's `CR1` --- of the kind `cadrio/iob.eco` records for the video,
/// twisted pairs "on backplane" from slot 2 to slot 15 landing on pins
/// named differently at the two ends --- or one list is wrong about its
/// pin, or the two were never joined. MIT's Xbus specification settles
/// why the pins differ (the last test in this file): `CP1` is taken at
/// the bus interface's slot and `CR1` is a bused line there. The hand
/// wire is what muir assumes. **Unverified**: what would settle it is
/// the wire itself, a photograph of a cage or an installation note. That
/// keyboards did reboot machines is on record --- ECO#3 warns of "the
/// old keyboard rebooting the machine accidentally" --- so the path was
/// live; the wire is what is not shown.
///
/// muir carries the wire all the same, from the board's `-BOOT*` straight
/// to the processor's `-BOOT1`, `FarEnd::boot_line`, as what ECO#3 says
/// happened rather than what a file shows (`docs/keyboard-boot.md`;
/// `tests/keyboard_boot.rs` holds the wire). This test is here so that a
/// corrected list, or a cage document, is noticed: if the two pins ever
/// agree, the sentence above is wrong.
#[test]
fn the_boot_line_is_the_one_wire_the_two_boards_put_on_different_pins() {
    let (busint, io) = the_two_lists();
    let boot = signal(&io, "-BOOT*");
    assert_eq!(backplane_pins(boot), ["CP1"], "the I/O board's boot output");
    let lm_boot = signal(&busint, "-LM BOOT");
    assert_eq!(backplane_pins(lm_boot), ["CR1"], "the bus interface's boot input");
    let cable: Vec<String> = lm_boot
        .pins
        .iter()
        .filter(|p| p.body == "CON" && p.location.starts_with('J'))
        .map(|p| format!("{}-{:02}", p.location, p.number))
        .collect();
    assert_eq!(cable, ["J08-12"], "passed straight through to the processor cable");
    assert!(
        lm_boot.pins.iter().all(|p| p.body == "CON"),
        "no part on the bus interface reads or drives it: {:?}",
        lm_boot.pins
    );
    assert_ne!(backplane_pins(boot), backplane_pins(lm_boot), "and the two pins differ");
}

/// **The bus interface's boot line arrives at the processor as `-BOOT1`,
/// and `-BOOT2` is the light panel's.** `data/cables.txt` pairs the bus
/// interface's `J08-12` with the processor's `1AJ1-12`, which
/// `cadrwd/icmem3.wlr` names `-BOOT1`; MIT's `cadr/busint.erface`
/// documents that cable signal, "Take this low to boot the machine. It
/// has a pullup." `-BOOT2` is on `1AJ2-03`, and the MBCPIN drawing marks
/// connector `1AJ2` "TO LIGHT PANEL" and `1AJ1` "TO BUS INTERFACE J08".
/// So a CADR boots from the keyboard through `-BOOT1`, from the button
/// through `-BOOT2`, and from the other machine through `PROG.BOOT`; on
/// the board the three meet at the 74S02 at OLORD2 1A07 that makes
/// `-BOOT`. The prompt's `boot` is the button and presses `-BOOT2`.
#[test]
fn the_boot_line_reaches_the_processor_as_boot1_and_the_light_panel_is_boot2() {
    let cpu = netlist::parse(include_str!("../data/CADR.netlist")).unwrap();
    let icmem = wire_list(&cpu, &["cadrwd", "icmem3.wlr"]);
    let at = |name: &str| -> Vec<String> {
        signal(&icmem, name)
            .pins
            .iter()
            .filter(|p| p.body == "CON")
            .map(|p| format!("{}-{:02}", p.location, p.number))
            .collect()
    };
    assert_eq!(at("-BOOT1"), ["1AJ1-12"]);
    assert_eq!(at("-BOOT2"), ["1AJ2-03"]);
    let cables =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/data/cables.txt")).unwrap();
    assert!(
        cables
            .lines()
            .any(|l| l.starts_with("1AJ1 12 J08 12 | -BOOT1 |") && l.contains("| -LM BOOT |")),
        "cables.txt carries J08-12 to -BOOT1"
    );
    let erface = mit_text(&["cadr", "busint.erface"]);
    assert!(
        erface.contains("-BOOT1\t\tTake this low to boot the machine.  It has a pullup."),
        "MIT documents the cable signal"
    );
    // On the board, each of the three presses makes -BOOT by itself.
    use muir::chip::Chip;
    use muir::part::Level;
    let net = |name: &str| cpu.by_name_id(name).unwrap_or_else(|| panic!("no net {name}"));
    let mut c = Chip::new(&cpu);
    c.power_on();
    c.settle();
    assert_eq!(c.net(net("-BOOT")), Level::High, "nothing pressed");
    for (input, pressed) in
        [("-BOOT1", Level::Low), ("-BOOT2", Level::Low), ("PROG.BOOT", Level::High)]
    {
        let released = if pressed == Level::Low { Level::High } else { Level::Low };
        c.set_net(net(input), pressed);
        c.settle();
        assert_eq!(c.net(net("-BOOT")), Level::Low, "{input} pressed makes -BOOT");
        c.set_net(net(input), released);
        c.settle();
        assert_eq!(c.net(net("-BOOT")), Level::High, "{input} released");
    }
}

/// A backplane pin as `cadr1/dubspc.wires` writes one: `CS2` on a bus
/// strip, or `C1-S2` at a slot's end of a wire from slot to slot, the
/// slot dropped. A row `A` to `F`, a pin letter, and side 1 or 2 --- so
/// `BR7`, a signal, is not one.
fn spc_pin(token: &str) -> Option<String> {
    let row = |c: u8| (b'A'..=b'F').contains(&c);
    let side = |c: u8| c == b'1' || c == b'2';
    match token.as_bytes() {
        [r, p, n] if row(*r) && p.is_ascii_uppercase() && side(*n) => Some(token.to_string()),
        [r, s, b'-', p, n]
            if row(*r) && s.is_ascii_digit() && p.is_ascii_uppercase() && side(*n) =>
        {
            Some(format!("{}{}{}", *r as char, *p as char, *n as char))
        }
        _ => None,
    }
}

/// MIT's backplane list, read: the bus strips, pin to signal, and the
/// grant chains' pins at the slots, pin to signal.
fn backplane_list(text: &str) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let mut strips = BTreeMap::new();
    let mut chains = BTreeMap::new();
    let mut in_strips = false;
    let mut name = String::new();
    for line in text.lines() {
        if line.starts_with("Bus strip, all the way across") {
            in_strips = true;
            continue;
        }
        if line.starts_with("Extra ground wiring") {
            in_strips = false;
        }
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if in_strips {
            let is_pin = |t: &str| t.len() == 3 && spc_pin(t).is_some();
            let words: Vec<&str> = tokens.iter().copied().take_while(|t| !is_pin(t)).collect();
            if !words.is_empty() {
                name = words.join(" ");
            }
            for pin in tokens.iter().copied().filter(|t| is_pin(t)) {
                strips.insert(pin.to_string(), name.clone());
            }
        } else if let [signal, dir, .., end] = tokens[..]
            && (dir == "(IN)" || dir == "(OUT)")
        {
            chains.insert(spc_pin(end).unwrap(), signal.to_string());
        }
    }
    (strips, chains)
}

/// The backplane list's name for a board's record: `-D0*` is its `D00`,
/// `-A1*` its `A01`, `-MSYN*` its `MSYN`, and `BG5.IN*` the chain `BG5`.
fn strip_name(record: &str) -> String {
    let core = record.trim_start_matches('-').trim_end_matches('*');
    let core = core.split('.').next().unwrap();
    let (letter, digits) = core.split_at(1);
    if matches!(letter, "A" | "D")
        && !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
    {
        format!("{letter}{:02}", digits.parse::<u32>().unwrap())
    } else {
        core.to_string()
    }
}

/// **MIT's own list for the backplane buses every shared wire on the pin
/// both boards give it, and has neither boot pin.** `cadr1/dubspc.wires`,
/// "Wire List for double (9-slot) SPC backplane", is the cage's wiring as
/// MIT specified it: the Unibus as bus strips "all the way across" on
/// DEC's SPC pins, `NPG` and `BG7` to `BG4` as wires from slot to slot,
/// and grant-continuity jumpers to be "installed last" because "some of
/// them will be removed by hand and replaced with grant wiring for
/// specific devices". It is the third copy of the forty-two pins above,
/// by a different route from either board's list. And `CP1` and `CR1`
/// are in it nowhere --- not bused, not chained, not joined --- while
/// `CP2` and `CR2` beside them carry `D05` and `D01`: a wire between them
/// would be device wiring of the kind the list leaves to the hand.
#[test]
fn mits_backplane_list_buses_every_shared_wire_and_has_neither_boot_pin() {
    let text = mit_text(&["cadr1", "dubspc.wires"]);
    assert!(text.starts_with("Wire List for double (9-slot) SPC backplane."));
    let (strips, chains) = backplane_list(&text);
    assert_eq!(strips.len(), 77, "bus strip pins: the supplies, the grounds and the Unibus");
    assert_eq!(chains.len(), 10, "NPG and BG7 to BG4, in at slot 1 and out at slot 9");
    let (_, io) = the_two_lists();
    let mut checked = 0;
    for (_, board) in muir::unibus::wire_pairs() {
        let record = match board.as_str() {
            "-BR*" => "-BR5*",
            "BG.IN*" => "BG5.IN*",
            other => other,
        };
        let pins = backplane_pins(signal(&io, record));
        let pin = &pins[0];
        let want = strip_name(record);
        match (strips.get(pin), chains.get(pin)) {
            (Some(name), _) => assert_eq!(*name, want, "{board} on {pin}: a bus strip"),
            (None, Some(name)) => assert_eq!(*name, want, "{board} on {pin}: a chain pin"),
            (None, None) => panic!("{board} on {pin}: neither a bus strip nor a chain pin"),
        }
        checked += 1;
    }
    assert_eq!(checked, 42, "the pairs");
    for pin in ["CP1", "CR1"] {
        assert!(!text.contains(pin), "{pin} is in the backplane list");
    }
    assert!(!text.to_ascii_uppercase().contains("BOOT"));
    assert_eq!(strips.get("CP2").map(String::as_str), Some("D05"));
    assert_eq!(strips.get("CR2").map(String::as_str), Some("D01"));
    assert!(text.contains("will be removed by hand and replaced with grant"));
    assert!(text.contains("wiring for specific devices"));
}

/// The entries of a line of `cadr1/xspec.text.3`, each with the column
/// it starts at: a tab is a stop of eight and ends an entry, as does a
/// run of two spaces; a single space is inside one, `NPG IN`.
fn spec_entries(line: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut cur: Option<(usize, String)> = None;
    let mut col = 0;
    let mut in_gap = false;
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '\t' => {
                out.extend(cur.take());
                in_gap = true;
                col = (col / 8 + 1) * 8;
            }
            ' ' => {
                let next_blank = chars.get(i + 1).is_none_or(|&n| n == ' ' || n == '\t');
                if in_gap || next_blank {
                    out.extend(cur.take());
                    in_gap = true;
                } else if let Some((_, text)) = cur.as_mut() {
                    text.push(' ');
                }
                col += 1;
            }
            _ => {
                in_gap = false;
                match cur.as_mut() {
                    Some((_, text)) => text.push(c),
                    None => cur = Some((col, c.to_string())),
                }
                col += 1;
            }
        }
    }
    out.extend(cur.take());
    out
}

/// A slot table of MIT's Xbus specification, `cadr1/xspec.text.3`: the
/// pin rows from `header` to the next form feed, each as the row's two
/// letters and its side 1 and side 2 entries. Side 1 starts before
/// column 24 and side 2 at 16 or later; what starts at 40 or later is
/// another slot's column and not read. A `"` is a ditto for the row
/// above, `BG7 IN` over `" OUT`.
fn spec_slot(text: &str, header: &str) -> Vec<(String, String, String)> {
    let start = text.find(header).unwrap_or_else(|| panic!("no table {header}"));
    let body = &text[start..];
    let end = body.find('\x0c').unwrap_or(body.len());
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for line in body[..end].lines() {
        let entries = spec_entries(line);
        let Some((0, letters)) = entries.first() else { continue };
        let b = letters.as_bytes();
        if b.len() != 2 || !(b'A'..=b'F').contains(&b[0]) || !b[1].is_ascii_uppercase() {
            continue;
        }
        let side1 = entries.iter().position(|(c, _)| (3..24).contains(c));
        let side2 = entries
            .iter()
            .enumerate()
            .position(|(k, (c, _))| (16..40).contains(c) && side1.is_none_or(|s| k > s));
        let text_of = |k: Option<usize>| k.map_or(String::new(), |k| entries[k].1.clone());
        let mut side1 = text_of(side1);
        let mut side2 = text_of(side2);
        if let Some(prev) = rows.last() {
            for (side, above) in [(&mut side1, &prev.1), (&mut side2, &prev.2)] {
                if let Some(rest) = side.strip_prefix('"') {
                    let word = above.split(' ').next().unwrap_or("");
                    *side = format!("{word}{rest}");
                }
            }
        }
        rows.push((letters.clone(), side1, side2));
    }
    rows
}

/// The backplane list's name for an entry in the specification's slot
/// tables: `D0` is `D00`, `BUS INIT` is `INIT`, `BG7 IN` the chain `BG7`,
/// `(gnd)` is `GND`, and `-5` is itself.
fn spec_name(entry: &str) -> String {
    let entry = entry.trim_matches(|c| c == '(' || c == ')').to_ascii_uppercase();
    let first = entry.trim_start_matches("BUS ").split(' ').next().unwrap().to_string();
    if first.starts_with(['+', '-']) && first[1..].starts_with(|c: char| c.is_ascii_digit()) {
        first
    } else {
        strip_name(&first)
    }
}

/// **MIT's Xbus specification puts the bus interface's slot on the same
/// pins as the bus interface's own list, and its SPC slot on the
/// backplane list's, and it is where the boot line's two pins are told
/// apart.** `cadr1/xspec.text.3` gives the pinout of "SLOT 11, BUS
/// INTERFACE SLOT": rows A and B the Xbus data and address, "identical to
/// the pin layout of the interface card"; rows C to F the Unibus on side
/// 2 in DEC's positions and the Xbus control on side 1. Every Xbus entry
/// is on that pin in `busint.wlr`, page CXBUS, and every Unibus, power
/// and ground entry in rows C to F is that pin's bus strip or chain in
/// `dubspc.wires`, whose strips are those rows'. Its "OUR MODIFIED SPC
/// SLOT" table, the I/O board's kind of slot, has the same strips on
/// side 2 and nothing on `CP1` or `CR1`.
///
/// At slot 11, `CP1` is `-XBUS.SYNC`: the pin the I/O board sends the
/// boot line out on is taken at the bus interface's end. And `CR1`, where
/// the bus interface takes `-LM BOOT`, is one of the two side-1 pins in
/// rows C to F the table marks `--`, which its legend defines: "-- means
/// bussed through, otherwise pin uncommitted". The other is `CU1`, the
/// bus interface's `-XBUS POWER RESET`, which the display board's list
/// has on `CU1` too. So the boot line's two ends differ because they
/// have to, and its bus-interface end is a bused line.
#[test]
fn the_xbus_specification_puts_slot_11_on_the_bus_interfaces_pins_and_tells_the_boot_pins_apart() {
    let text = mit_text(&["cadr1", "xspec.text.3"]);
    assert!(text.contains("(-- means bussed through,"), "the legend");
    assert!(text.contains(" otherwise pin uncommitted)"), "the legend");
    let (strips, chains) = backplane_list(&mit_text(&["cadr1", "dubspc.wires"]));
    let (busint, _) = the_two_lists();
    let on_busint = |name: &str, pin: &str| -> bool {
        let name = name.replace('.', " ");
        busint.iter().any(|s| {
            s.names.contains(&name) && s.pins.iter().any(|p| p.body == "CON" && p.location == pin)
        })
    };
    let mut xbus = 0;
    let mut unibus = 0;
    let mut power_ab = 0;
    let mut bused_through = Vec::new();
    // Slot 11 is two tables a form feed apart, rows A and B under
    // "SLOT 11" and rows C to F under the header they share with the TV
    // slots; the SPC slot is one table of rows C to F.
    let tables = [
        ("slot 11", vec!["SLOT 11", "SLOT 15-18"], 108),
        ("the SPC slot", vec!["OUR MODIFIED \"SPC\" SLOT"], 72),
    ];
    for (table, headers, count) in tables {
        let rows: Vec<_> = headers.iter().flat_map(|h| spec_slot(&text, h)).collect();
        assert_eq!(rows.len(), count, "{table}: rows");
        for (letters, side1, side2) in &rows {
            for (side, entry) in [("1", side1), ("2", side2)] {
                let pin = format!("{letters}{side}");
                // "[BRACKETED SIGNALS] INDICATE ONES THAT MAY NEED TO BE
                // ADDED FOR COMPLETE SPC COMPATABLILITY": not wired.
                if entry.is_empty() || entry.starts_with('[') {
                    continue;
                }
                if entry == "--" {
                    bused_through.push(pin);
                } else if entry.starts_with("-X") || entry.starts_with("XBUS") {
                    assert_eq!(table, "slot 11");
                    assert!(on_busint(entry, &pin), "{table}: {entry} on {pin} in busint.wlr");
                    xbus += 1;
                } else if letters.starts_with(['A', 'B']) {
                    // The backplane list's rows A and B are the Unibus
                    // connectors of an SPC unit; the Xbus slot's carry
                    // power and ground beside the Xbus.
                    assert!(
                        matches!(entry.as_str(), "+5" | "-5" | "GND"),
                        "{table}: {entry} on {pin}"
                    );
                    power_ab += 1;
                } else {
                    let want = spec_name(entry);
                    let got = strips.get(&pin).or_else(|| chains.get(&pin));
                    assert_eq!(
                        got.map(|n| n.split(' ').next().unwrap()),
                        Some(want.as_str()),
                        "{table}: {entry} on {pin} in dubspc.wires"
                    );
                    unibus += 1;
                }
            }
        }
        let at = |pins: &str| rows.iter().find(|r| r.0 == pins).unwrap().clone();
        if table == "slot 11" {
            assert_eq!(at("CP").1, "-XBUS.SYNC", "CP1 at the bus interface's slot");
            assert_eq!(at("CR").1, "--", "CR1 at the bus interface's slot");
            assert_eq!(at("CU").1, "--", "CU1 at the bus interface's slot");
            assert!(on_busint("-LM BOOT", "CR1") && on_busint("-XBUS POWER RESET", "CU1"));
        } else {
            assert_eq!((at("CP").1.as_str(), at("CP").2.as_str()), ("", "D5"), "CP in an SPC slot");
            assert_eq!((at("CR").1.as_str(), at("CR").2.as_str()), ("", "D1"), "CR in an SPC slot");
        }
    }
    // The counts, read back: every Xbus data, address and control line,
    // and the Unibus with its power and grounds, in both tables.
    assert_eq!((xbus, unibus, power_ab), (70, 161, 12), "entries checked");
    bused_through.sort();
    assert_eq!(bused_through, ["CR1", "CU1"]);
    let tv = wire_list(
        &netlist::parse(include_str!("../data/LISPMTV.netlist")).unwrap(),
        &["cadrtv", "lmtv4b.wlr"],
    );
    let reset = signal(&tv, "-XBUS POWER RESET");
    assert_eq!(backplane_pins(reset), ["CU1"], "the display board's end of the bused line");
}
