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
//! found to be the exception.

mod support;

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
/// the same one. So either MIT's card cage joins them with a backplane
/// wire no file here describes, or one list is wrong about its pin, or
/// the two were never joined. **Unverified**: what would settle it is a
/// backplane wire list or a photograph of a cage. That keyboards did
/// reboot machines is on record --- `cadrio/iob.eco` ECO#3 warns of "the
/// old keyboard rebooting the machine accidentally" --- so the path was
/// live; the pin is what is not established.
///
/// muir carries neither the wire nor the boot word (`docs/keyboard-boot.md`).
/// This test is here so that a corrected list, or a cage document, is
/// noticed: if the two pins ever agree, the sentence above is wrong.
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
