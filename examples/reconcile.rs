// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reconciles a netlist read off the drawings with MIT's wire list for the
//! same board.
//!
//!     cargo run --release --example reconcile -- <netlist> <wlr> <prefix> > out
//!
//! `prefix` is what the netlist puts before a wire-list slot: `0` for the
//! bus interface, whose `A25` is `0A25` in the netlist. `wlr` is `-` for a
//! board MIT left no wire list of, and then only the pin moves below are
//! made. Where a wire of the
//! list falls on more than one net of the netlist --- a wire the drawings
//! label twice, a name spelt with and without a space, a pin the reader lost
//! to `@,p0` --- every pin of that wire is put on one net under MIT's name.
//! A wire the list leaves unnamed is left to the parser's own rule for
//! them unless the reader broke it, and is then named the netlist's way,
//! after one of its pins. The other way round happens too: two unnamed
//! wires the reader gave one name --- a location holding two bodies with
//! the same pin number, so that `@0E08,p1` is two wires on the LISPM TV's
//! ECLVID --- and then each is named after its own pin, as the list names
//! it. Then the pins the board had elsewhere than the drawing puts them
//! are moved, [`MOVES`] --- MIT's change orders, and the drawings' own
//! errors --- each one cited, and each announced in the file's banner.
//! Nothing else in the file is touched, and `tests/*_netlist.rs` then check
//! the result against the list.

use std::collections::{BTreeMap, BTreeSet};

use muir::wirelist;

/// Why a pin is not where the drawing puts it.
enum Reason {
    /// MIT changed the built board after drawing it: the change order, as
    /// its `.eco` file records it.
    Eco(&'static str),
    /// The drawing is wrong: the numbered discrepancy that shows it.
    Drawing(u32),
}

/// A pin the board had on another wire than the drawing's.
struct Move {
    page: &'static str,
    /// The slot as the wire list writes it, `E09`; the netlist's prefix goes
    /// in front.
    slot: &'static str,
    pin: u8,
    from: &'static str,
    to: &'static str,
    why: Reason,
}

/// Every way a board differs from its drawings, made when the netlist is
/// made from them.
///
/// The drawing stays the source, whichever the reason --- the December 1980
/// `reqlm.drw` is read as it is, not swapped for the older revision --- and
/// the move is made here, once, so that `data/*.netlist` is the board that
/// ran, its banner says how that differs from the sheets, and the model and
/// the wire-list tests read the same file. Each move requires the pin to be
/// on `from`: a drawing redrawn to include the change fails the build, and
/// is not moved twice. A page the netlist has not got is skipped, since one
/// reconciler serves every board, and the page names tell the boards apart.
///
/// **NXBCTL F11 pin 8, ECO 2 of `cadrtv/lmtv.eco`, 18 June 1980, on the
/// SIMPLE TV**: *"new window system not initializing tv properly at
/// original power-up; on old TV boards the check if TV is in PROM mode
/// (extant only on new TV boards) reads an unused input"* --- `GND` added
/// `F11-8 : F11-10`. The drawing of 17 May 1980 has that pin, the read
/// buffer's input for `XDO 7`, on `SYNC PROM ENB`; the ECO takes it to
/// ground, and System 100's window system reads the bit to tell the boards
/// apart. `src/simpletv.rs` has bit 7 reading zero for the same reason.
///
/// **REQLM E09 pin 2, discrepancy 68.** The 74S08 at E09
/// gates a Unibus master's Xbus request, `UBXRQ`, with the Xbus grant, and
/// the 10 December 1980 drawing writes that grant as `LMX GRANT A` --- the
/// processor's, which a mapped Unibus cycle never has. As drawn the cycle
/// is granted and never requested, and times out; CC reads and writes the
/// debuggee's memory through exactly that cycle. The 3 October 1978
/// revision of the page (ITS tape 7008105) has `UBX GRANT A` there, the
/// grant every other gate of the mapped path takes, and the redrawing two
/// years on changed one letter. The wire list of 11 December 1980 was made
/// from the redrawn sheet and repeats it; `tests/busint_netlist.rs` moves
/// the list's one pin the same way before comparing.
///
/// **IOBSER A12 pin 15, ECO 10 of `cadrio/iob.eco`, 13 December 1981, on
/// the I/O board**: *"Output interrupts from 2651 for system 78.14.
/// Enables interupts on output for serial I/O. Add wire from A12-8 to
/// C20-1. Apply this only to IOB's wired for a 2651 on an adaptor at
/// A12."* The 2651 sits on a 28-pin adaptor over sockets A12, A13 and A14
/// --- `iob.prt`, "PACKAGE:28 PIN MIT ADAPTOR" --- and the wire list of 11
/// August 1981 gives each chip pin's socket position: `A12-15(08)`, the
/// chip's pin 15, `-TXRDY`, at socket pin A12-8, and `C20-01` on `-SER
/// RRDY` with the chip's `-RxRDY` and the pull-up. So the ECO wires the
/// transmitter's ready onto the receiver's net, both open drain, and
/// `SER.IREQ` follows either. The drawing of 11 December 1980 and the
/// wire list both predate the change and leave the pin unconnected;
/// System 100's `sys/io1/serial.lisp` runs an output channel on the
/// port's vector, which only this wire makes interrupt.
const MOVES: &[Move] = &[
    Move {
        page: "NXBCTL",
        slot: "F11",
        pin: 8,
        from: "'SYNC PROM ENB'",
        to: "GND",
        why: Reason::Eco("ECO 2 of cadrtv/lmtv.eco, 18 June 1980"),
    },
    Move {
        page: "REQLM",
        slot: "E09",
        pin: 2,
        from: "'LMX GRANT A'",
        to: "'UBX GRANT A'",
        why: Reason::Drawing(68),
    },
    Move {
        page: "IOBSER",
        slot: "A12",
        pin: 15,
        from: "NC",
        to: "'-SER RRDY'",
        why: Reason::Eco("ECO 10 of cadrio/iob.eco, 13 December 1981"),
    },
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [netlist, wlr, prefix] = args.as_slice() else {
        eprintln!("usage: reconcile <netlist> <wlr> <prefix>");
        std::process::exit(2);
    };
    let text = std::fs::read_to_string(netlist).unwrap();
    let list =
        (wlr != "-").then(|| String::from_utf8_lossy(&std::fs::read(wlr).unwrap()).into_owned());
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let pages: Vec<String> = lines
        .iter()
        .filter_map(|l| l.strip_prefix("page ").map(|p| p.trim().to_string()))
        .collect();

    // Every pin line, by page, reference designator, kind and pin.
    let mut at: BTreeMap<(String, String, String, u8), Vec<usize>> = BTreeMap::new();
    let (mut page, mut reference, mut kind) = (String::new(), String::new(), String::new());
    for (i, l) in lines.iter().enumerate() {
        let l = l.trim();
        if let Some(p) = l.strip_prefix("page ") {
            page = p.trim().to_string();
        } else if let Some(p) = l.strip_prefix("part ") {
            let (r, k) = p.split_once(',').unwrap();
            reference = r.trim().to_string();
            kind = k.trim().to_string();
        } else if let Some((pin, _)) = l.strip_prefix('p').and_then(|r| r.split_once('='))
            && let Ok(pin) = pin.parse::<u8>()
        {
            at.entry((page.clone(), reference.clone(), kind.clone(), pin)).or_default().push(i);
        }
    }
    let name_of = |l: &str| l.split_once('=').map(|(_, n)| n.trim().to_string()).unwrap();

    if let Some(list) = &list {
        reconcile(&mut lines, &at, &pages, list, prefix, &name_of);
    }

    // The board's own departures from its drawings, last, so that a wire
    // the list settled is moved as one net.
    let mut moved: Vec<String> = Vec::new();
    for m in MOVES {
        if !pages.iter().any(|p| p == m.page) {
            continue;
        }
        let reference = format!("{prefix}{}", m.slot);
        let hits: Vec<usize> = at
            .iter()
            .filter(|((page, r, _, pin), _)| *page == m.page && *r == reference && *pin == m.pin)
            .flat_map(|(_, v)| v.iter().copied())
            .collect();
        let [i] = hits.as_slice() else {
            eprintln!("{} {}-{}: {} pin lines, expected one", m.page, reference, m.pin, hits.len());
            std::process::exit(1);
        };
        let (pin, was) = lines[*i].split_once('=').unwrap();
        if was.trim() != m.from {
            eprintln!(
                "{} {}-{} is on {}, not {}: the drawing has changed, and the move \
                 may no longer apply",
                m.page,
                reference,
                m.pin,
                was.trim(),
                m.from
            );
            std::process::exit(1);
        }
        lines[*i] = format!("{pin}={}", m.to);
        let why = match m.why {
            Reason::Eco(eco) => eco.to_string(),
            Reason::Drawing(d) => format!("drawing error, discrepancy {d}"),
        };
        eprintln!("moved {} {}-{}: {} -> {} ({why})", m.page, reference, m.pin, m.from, m.to);
        moved.push(format!(
            "#   {} {}-{}: {} -> {} ({why})",
            m.page, reference, m.pin, m.from, m.to
        ));
    }
    if !moved.is_empty() {
        // Into the banner, which is the run of `#` lines the generating
        // script puts first.
        let end = lines.iter().position(|l| !l.starts_with('#')).unwrap_or(lines.len());
        let mut banner =
            vec!["# pins moved off the drawings by examples/reconcile.rs:".to_string()];
        banner.extend(moved);
        banner.push("#".to_string());
        lines.splice(end..end, banner);
    }

    print!("{}", lines.join("\n"));
    println!();
}

/// Puts every pin of a wire the list has on one net under MIT's name; see
/// the module comment.
fn reconcile(
    lines: &mut [String],
    at: &BTreeMap<(String, String, String, u8), Vec<usize>>,
    pages: &[String],
    list: &str,
    prefix: &str,
    name_of: &dyn Fn(&str) -> String,
) {
    let signals = wirelist::parse(list, pages);
    // The pin lines each wire lands on, and the names they carry.
    let placed: Vec<(Vec<usize>, BTreeSet<String>)> = signals
        .iter()
        .map(|s| {
            let mut on: Vec<usize> = Vec::new();
            for p in &s.pins {
                if p.page.is_empty() {
                    continue;
                }
                let key =
                    (p.page.clone(), format!("{prefix}{}", p.slot()), p.body.clone(), p.number);
                // Two of the same part at one location cannot be told apart,
                // and are left alone.
                if let Some(v) = at.get(&key)
                    && v.len() == 1
                {
                    on.push(v[0]);
                }
            }
            let names: BTreeSet<String> = on.iter().map(|&i| name_of(&lines[i])).collect();
            (on, names)
        })
        .collect();
    // How many wires claim each anonymous name: two is one net that should
    // be two.
    let mut claims: BTreeMap<&str, usize> = BTreeMap::new();
    for (s, (_, names)) in signals.iter().zip(&placed) {
        if s.is_pseudo() {
            continue;
        }
        for n in names {
            if n.starts_with('@') {
                *claims.entry(n.as_str()).or_default() += 1;
            }
        }
    }

    let mut changed = 0;
    for (s, (on, names)) in signals.iter().zip(&placed).filter(|(s, _)| !s.is_pseudo()) {
        let on = on.clone();
        // An unnamed wire is named from each end and joined by the parser;
        // only one the reader broke, or one sharing its name with another
        // wire, needs touching.
        let broken = names.contains("@,p0");
        let shared = names.iter().any(|n| claims.get(n.as_str()).copied().unwrap_or(0) > 1);
        if names.len() < 2 && !broken && !shared || s.name().starts_with('%') && !broken && !shared
        {
            continue;
        }
        let canonical = if let Some(unnamed) = s.name().strip_prefix('%') {
            let (slot, pin) = unnamed.split_once('-').unwrap();
            format!("@{prefix}{slot},p{}", pin.trim_start_matches('0'))
        } else if s.name().chars().all(|c| c.is_ascii_alphanumeric() || "-=._/".contains(c)) {
            s.name().to_string()
        } else {
            format!("'{}'", s.name())
        };
        eprintln!("{:?}: {:?} -> {canonical}", s.names, names);
        for &i in &on {
            let (pin, _) = lines[i].split_once('=').unwrap();
            lines[i] = format!("{pin}={canonical}");
            changed += 1;
        }
    }
    eprintln!("{changed} pins renamed");
}
