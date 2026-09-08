// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! MIT's wire lists, the `.wlr` files beside the drawings.
//!
//! A wire list is what MIT's own SUDS tooling made of a board's drawings in
//! 1980, one signal at a time: every pin on the wire, and the drawing each
//! pin is on. It is the drawings by a second route --- the one the boards
//! were wire-wrapped from --- and so it is the cross-check for a netlist read
//! off the same drawings by another reader. Where the two disagree, the wire
//! list is the board.
//!
//! The format, read off `cadr1/busint.wlr` and `cadrwd/cadr4.wlr`: after a
//! `SIGNAL NAME` heading, blocks separated by blank lines. A block opens with
//! one name a line --- a wire the drawings label twice has two, `LMRD` and
//! `-LMWR` --- and goes on with one pin a line: `LOC-PIN(SOCKET)`, loading
//! and length columns, the body's type and DIP type, the drawing it is on
//! and its position there. The first pin may share the last name's line.
//! `H` after a name says the drawings wrote that name with MIT's explicit
//! assertion-level suffix, `NAME L`, which the tooling folded into the
//! leading minus of the name it prints: `EDGE L` on LMDETC comes out
//! `-EDGE`, `-CBLBSY L` on LMMYNM comes out `CBLBSY`. It is a fact about the
//! name and not about the wire, so it is ignored here. Checked over the
//! whole set: every one of the 595 names carrying `H` in the nine wire lists
//! in `mit/` is the fold of an ` L`-suffixed label in the drawings, and
//! `cadrwd/cadr4.wlr`, the processor, whose drawings never write the suffix,
//! carries none at all.
//! `NC` is the block of unconnected pins, and the supplies are
//! blocks like any other. Long lines are cut off in the files, so the
//! drawing is taken as the last column that names a page rather than by
//! position. One control character, `\x19`, stands in for `>`.
//!
//! Two things the format does not say and the files do. A block ends at a
//! blank line **or at the next name**, because the supplies run `+12.0V`
//! straight into `+5.0V` with nothing between them; reading them as one wire
//! puts pins on a rail the board has not got. And a page break is `0xEE`,
//! then a formfeed, then the title, so once the formfeed is gone the line
//! arrives where a name would --- which is why a name that is not ASCII is
//! dropped rather than kept.

use std::collections::HashMap;

use crate::netlist::{NetId, Netlist};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    /// `3E05`, `A25`, `J12`, or `A28@20` when a location holds more than
    /// one body: MIT names the second one at a location with an `@nn`, and
    /// it is not only packs that share one. Three of the disk controller's
    /// are pairs of 75452 drivers, DCTRSG `A02`/`A02@03`, `B01`/`B01@03`
    /// and `B03`/`B03@03`, two devices apiece in one footprint --- and MIT
    /// gives both bodies of a pair the same pin numbers, so nothing but the
    /// `@nn` tells them apart. What the number itself means is
    /// **unverified**; no MIT file found so far defines it.
    pub location: String,
    pub number: u8,
    /// The drawing, from the `FILE` column; empty when it was cut off or
    /// names no page the netlist has.
    pub page: String,
    /// The `BODY` column: `74S08O`, or `CON` for a connector pin.
    pub body: String,
}

impl Pin {
    /// The location without the `@` offset: `A28` for `A28@20`.
    pub fn slot(&self) -> &str {
        self.location.split('@').next().unwrap_or("")
    }
}

#[derive(Clone, Debug)]
pub struct Signal {
    /// MIT's name first, then the other labels the drawings give the wire.
    pub names: Vec<String>,
    pub pins: Vec<Pin>,
}

impl Signal {
    pub fn name(&self) -> &str {
        &self.names[0]
    }

    /// `NC` and the supplies: blocks that are not wires.
    pub fn is_pseudo(&self) -> bool {
        let n = self.name();
        n == "NC"
            || n == "GND"
            || n == "VCC"
            || n.starts_with('+')
            || n.strip_prefix('-').is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
    }
}

/// One pin line: `3E05-04(07)  \  TIS  -2.0  0.5  ...  74S08  74S08O  SOURCE  B7`.
///
/// A connector pin is `J12-14` on a cable header, or a bare Unibus or Xbus
/// pin name --- `CA1`, `DT2` --- with `CON` for its body and no pin number,
/// which is kept as pin 0.
fn pin(line: &str, pages: &[String]) -> Option<Pin> {
    let mut tokens = line.split_whitespace();
    let head = tokens.next()?;
    let rest: Vec<&str> = tokens.collect();
    let (location, number) = match head.split_once('-') {
        Some((location, pin)) => (location, pin.split('(').next()?.parse().ok()?),
        None if rest.contains(&"CON") => (head, 0),
        None => return None,
    };
    if location.is_empty() || !location.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return None;
    }
    let at = rest.iter().rposition(|t| pages.iter().any(|p| p == t));
    let page = at.map(|i| rest[i].to_string()).unwrap_or_default();
    let body = at.filter(|&i| i > 0).map(|i| rest[i - 1].to_string()).unwrap_or_default();
    Some(Pin { location: location.to_string(), number, page, body })
}

pub fn parse(text: &str, pages: &[String]) -> Vec<Signal> {
    let title = text.lines().next().unwrap_or("").split('\t').next().unwrap_or("").trim();
    let mut signals = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut pins: Vec<Pin> = Vec::new();
    let mut started = false;
    for raw in text.lines() {
        let line = raw.replace('\x0c', "");
        if line.starts_with("SIGNAL NAME") {
            started = true;
            continue;
        }
        if !started || (!title.is_empty() && line.starts_with(title)) || line.contains("LOC(PIN#)")
        {
            continue;
        }
        if line.trim().is_empty() {
            if !pins.is_empty() {
                signals.push(Signal {
                    names: std::mem::take(&mut names),
                    pins: std::mem::take(&mut pins),
                });
            }
            names.clear();
            continue;
        }
        if line.starts_with(['\t', ' ']) {
            if let Some(p) = pin(&line, pages) {
                pins.push(p);
            }
            continue;
        }
        let (name, rest) = line.split_once('\t').unwrap_or((&line, ""));
        // A name is ASCII, so a line carrying anything else is dropped
        // rather than taken for a wire. Two kinds of line need that:
        //
        // - **Every page break.** These files put a stray `0xEE` before the
        //   formfeed, so with the formfeed stripped the line no longer opens
        //   with the title and arrives here looking like a name. All five
        //   wire lists do it, 70 to 87 times each.
        // - **Wreckage.** A tape dump can carry binary lines, some of them
        //   starting at column 0; `cadrm/mem.wlr` had thirteen. This guard is
        //   what keeps them out of the names.
        //
        // Control characters are not the test: MIT names one wire on the I/O
        // board `POWER LINE \x01`.
        if !name.is_ascii() {
            continue;
        }
        // A name line once pins have been collected is the next wire, not
        // another label for this one: the alias names of a wire the drawings
        // label twice all come before its first pin. Only the supplies run
        // together without a blank line between them --- `+12.0V` straight
        // into `+5.0V` --- and merging those two would put the bus cable's
        // terminator pins on the wrong rail.
        if !pins.is_empty() {
            signals.push(Signal {
                names: std::mem::take(&mut names),
                pins: std::mem::take(&mut pins),
            });
        }
        names.push(name.trim().replace('\x19', ">"));
        if let Some(p) = pin(rest, pages) {
            pins.push(p);
        }
    }
    if !pins.is_empty() {
        signals.push(Signal { names, pins });
    }
    signals
}

/// A wire the netlist has as more than one net: the wire's names, then
/// each net with how many of the wire's pins it holds.
pub type Split = (Vec<String>, Vec<(String, usize)>);

/// A netlist against a wire list, pin by pin.
#[derive(Debug, Default)]
pub struct Report {
    /// Wires whose pins the netlist puts on more than one net.
    pub split: Vec<Split>,
    /// Nets whose pins the wire list puts on more than one signal.
    pub merged: Vec<(String, Vec<String>)>,
    /// Wire-list pins found on the netlist, and on no part of it ---
    /// connectors, mostly.
    pub placed: usize,
    pub unplaced: usize,
    /// Pins the list places on a part the netlist **has**, at a pin that
    /// part has not got, as `page location-pin = signal`.
    ///
    /// This is the absence the rest of this report cannot see: a pin that
    /// is simply not there is counted in [`Report::unplaced`] along with
    /// every pin on a part the netlist does not model at all, so a
    /// netlist that is quietly short of pins passes a check on
    /// [`Report::split`] and [`Report::merged`] alone, which is why this
    /// field exists: a whole page of a board can be nine pins short and every
    /// other number in the report still agree.
    pub missing: Vec<String>,
}

/// Compares every wire the list has against the nets the netlist has.
///
/// `locate` turns a wire-list slot into the netlist's reference designator:
/// the bus interface's netlist writes `0A25` where its list writes `A25`.
/// A pin two parts could answer for --- two packs at one location --- counts
/// for the signal if either is on its net, and never against it.
pub fn compare(n: &Netlist, signals: &[Signal], locate: impl Fn(&str) -> String) -> Report {
    // Every netlist pin by page, reference designator and number, with the
    // part's kind beside the net.
    type Pins<'a> = HashMap<(String, String, u8), Vec<(&'a str, NetId)>>;
    let mut at: Pins = HashMap::new();
    let mut at_location: std::collections::HashSet<(String, String)> = Default::default();
    for part in &n.parts {
        at_location.insert((part.page.clone(), part.reference.clone()));
        for &(pin, net) in &part.pins {
            at.entry((part.page.clone(), part.reference.clone(), pin))
                .or_default()
                .push((part.kind.as_str(), net));
        }
    }
    let mut report = Report::default();
    let mut on: HashMap<NetId, Vec<String>> = HashMap::new();
    for s in signals.iter().filter(|s| !s.is_pseudo()) {
        let mut sure: Vec<NetId> = Vec::new();
        let mut maybe: Vec<Vec<NetId>> = Vec::new();
        for p in &s.pins {
            if p.page.is_empty() {
                continue;
            }
            let Some(parts) = at.get(&(p.page.clone(), locate(p.slot()), p.number)) else {
                report.unplaced += 1;
                // A pin the netlist has no part for at all is a connector
                // it does not model; a pin missing from a part it *does*
                // have is a hole, and is worth naming.
                if at_location.contains(&(p.page.clone(), locate(p.slot()))) {
                    report.missing.push(format!(
                        "{} {}-{} = {}",
                        p.page,
                        locate(p.slot()),
                        p.number,
                        s.name()
                    ));
                }
                continue;
            };
            report.placed += 1;
            // Two parts at one location: the body column tells them apart
            // when they are different parts, and when they are the same
            // part twice --- two pull-up packs --- either may answer.
            let same: Vec<NetId> =
                parts.iter().filter(|(kind, _)| *kind == p.body).map(|&(_, net)| net).collect();
            match (parts.len(), same.len()) {
                (1, _) => sure.push(parts[0].1),
                (_, 1) => sure.push(same[0]),
                _ => maybe.push(parts.iter().map(|&(_, net)| net).collect()),
            }
        }
        if sure.is_empty() {
            continue;
        }
        let mut count: HashMap<NetId, usize> = HashMap::new();
        for &id in &sure {
            *count.entry(id).or_default() += 1;
        }
        let main = count
            .iter()
            .max_by_key(|(id, c)| (**c, std::cmp::Reverse(**id)))
            .map(|(&id, _)| id)
            .unwrap();
        count.remove(&main);
        for nets in maybe {
            if !nets.contains(&main) {
                *count.entry(nets[0]).or_default() += 1;
            }
        }
        // `@,p0` is the reader's name for a reference it lost: a pin on it
        // is on nothing, so a wire there is broken whatever else it holds.
        if !count.is_empty() || n.net(main) == "@,p0" {
            let mut others: Vec<(String, usize)> =
                count.into_iter().map(|(id, c)| (n.net(id).to_string(), c)).collect();
            others.sort();
            let mut nets =
                vec![(n.net(main).to_string(), sure.iter().filter(|&&x| x == main).count())];
            nets.extend(others);
            report.split.push((s.names.clone(), nets));
        }
        on.entry(main).or_default().push(s.name().to_string());
    }
    for (id, mut names) in on {
        if names.len() > 1 {
            names.sort();
            report.merged.push((n.net(id).to_string(), names));
        }
    }
    report.split.sort();
    report.merged.sort();
    report
}
