// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Shared by the test binaries, each taking what it needs: where the tests
//! find MIT's files and the fetched release, the checks every board's
//! netlist gets, MIT's own census and wire list read back, the I/O board's
//! inputs at rest, and the boot to the listener.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use muir::engine::Engine;
use muir::netlist::Netlist;
use muir::part::{self, Drive, Level};
use muir::rtl::Rtl;
use muir::unibus::{IDLE_CHAOSNET, IDLE_KEYBOARD};
use muir::wirelist;

/// A file under `mit/`. Everything there is committed and always present,
/// so a missing one is a broken checkout and fails; it is never a skip.
pub fn mit(parts: &[&str]) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("mit");
    p.extend(parts);
    assert!(p.exists(), "{} is committed with the repository and should be there", p.display());
    p
}

/// A file under `mit/` as text. MIT wrote ASCII; a byte the tapes carry
/// outside it is read lossily rather than refused.
pub fn mit_text(parts: &[&str]) -> String {
    String::from_utf8_lossy(&std::fs::read(mit(parts)).unwrap()).into_owned()
}

/// The **System 304** pack, put there by `tools/fetch-system-304.sh`. That
/// release boots here and has a test of its own; it is not what the machine
/// tests run, because CC does not load on it --- see `tests/cc_harness`.
pub fn pack_304() -> Option<PathBuf> {
    vendor(&["run", "disk-sys-304-0.img"])
}

/// The pack the machine tests boot: **System 100**, what this project
/// targets, put there by `tools/fetch-system-100.sh`. It is also the
/// fixture the label and band tests are written against --- their
/// partition table, pack name and band comments are its.
pub fn pack_100() -> Option<PathBuf> {
    vendor(&["run", "disk-sys-100-0.img"])
}

/// The Chaosnet numbers the System 304 band holds: this machine
/// `AMS-LISPM-1` at 4401, and `OZ`, its file and time host, at 4403. Read
/// out of the band itself --- `(send (si:parse-host "OZ") :chaos-address)`
/// answers 2307 decimal, and `si:local-host` is `AMS-LISPM-1` when the
/// switches say 4401 --- and enforced by
/// `tests/chaos.rs::the_304_band_reaches_the_server_at_its_own_numbers`.
/// A server answering anywhere else is a server this band never calls.
pub const CHAOS_304: (u16, u16) = (0o4401, 0o4403);

/// A file under `vendor/`, where the fetch scripts put the releases, or
/// `None` with a line saying so: a test that needs a release skips
/// without it, and says that it did.
pub fn vendor(parts: &[&str]) -> Option<PathBuf> {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor");
    p.extend(parts);
    if p.exists() {
        Some(p)
    } else {
        eprintln!("skipped: {} is not present", p.display());
        None
    }
}

/// A source file of the System 100 release, `vendor/system-100-0/sys/<file>`,
/// as text, or `None` with the skip line.
///
/// **The two releases' sources are not interchangeable**, which is why this
/// names one: `window/shwarm.lisp`, for one, carries the display geometry
/// as `DEFCONST`s here and does not there. A test reads whichever release
/// its fact is from, and one that wants the target's takes
/// [`release_304`].
pub fn release(file: &str) -> Option<String> {
    let p = vendor(&["system-100-0", "sys", file])?;
    Some(String::from_utf8_lossy(&std::fs::read(p).unwrap()).into_owned())
}

/// A file of the System 100 release, `vendor/system-100-0/sys/<parts>`, as
/// a path: for `SYS: UBIN;`, where the readers want bytes rather than text.
pub fn release_100_file(parts: &[&str]) -> Option<PathBuf> {
    let mut p = vec!["system-100-0", "sys"];
    p.extend_from_slice(parts);
    vendor(&p)
}

/// A file of the System 304 sources, `vendor/system-304-0/sys-304-0/<file>`,
/// or `None` with the skip line. The upstream release is the pack alone,
/// so these are the project's own Fossil repository at branch
/// `system-304`; `tools/fetch-system-304.sh` says how they are built.
pub fn release_304(parts: &[&str]) -> Option<PathBuf> {
    let mut p = vec!["system-304-0", "sys-304-0"];
    p.extend_from_slice(parts);
    vendor(&p)
}

/// A supply, a pull-up, an unconnected pin or a spare: a net no signal is
/// on, which the driver checks leave out.
pub fn is_power(name: &str) -> bool {
    matches!(name.trim_matches('\''), "GND" | "VCC" | "NC" | "+5" | "-5")
        || name.starts_with("HI")
        || name.starts_with("NC#")
        || name.starts_with("PULLUP")
}

/// **Two totem-pole outputs on one net is an electrical fault**, so a
/// wrongly claimed output pin in `src/part.rs` surfaces here. It is the
/// check that caught a `74S472` entry claiming an output on its ground pin.
pub fn no_net_has_two_push_pull_drivers(n: &Netlist) {
    let mut drivers: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for p in &n.parts {
        let Some(po) = part::pinout(&p.kind) else { continue };
        for &(pin, net) in &p.pins {
            if po.drive_of(pin) == Some(Drive::Totem) && !is_power(n.net(net)) {
                drivers
                    .entry(net)
                    .or_default()
                    .push(format!("{} {} ({}) p{pin}", p.page, p.reference, p.kind));
            }
        }
    }
    let conflicts: Vec<_> = drivers.iter().filter(|(_, v)| v.len() > 1).collect();
    for (net, who) in conflicts.iter().take(12) {
        eprintln!("net {} driven by {}: {:?}", n.net(**net), who.len(), who);
    }
    eprintln!(
        "{} nets driven by a totem-pole output; {} conflicts",
        drivers.len(),
        conflicts.len()
    );
    assert!(conflicts.is_empty(), "{} nets have two push-pull drivers", conflicts.len());
}

/// What `src/part.rs` makes of the kinds a board uses, each kind once and
/// sorted: the kinds with no pinout, the kinds with a pinout and no
/// behaviour, and the kinds whose behaviour computes nothing --- no gate
/// and no update.
pub struct Kinds {
    pub unknown: Vec<String>,
    pub silent: Vec<String>,
    pub empty: Vec<String>,
}

pub fn kinds(n: &Netlist) -> Kinds {
    let (mut unknown, mut silent, mut empty) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for p in &n.parts {
        if part::pinout(&p.kind).is_none() {
            unknown.insert(p.kind.clone());
        } else if let Some(b) = part::behaviour(&p.kind) {
            if b.gates.is_empty() && b.update.is_none() {
                empty.insert(p.kind.clone());
            }
        } else {
            silent.insert(p.kind.clone());
        }
    }
    Kinds {
        unknown: unknown.into_iter().collect(),
        silent: silent.into_iter().collect(),
        empty: empty.into_iter().collect(),
    }
}

/// MIT's census of a board by body, a `.wls` file: under "DIPTYPE  BODY
/// NAME  # SECTION  TOTAL DIPS  #SPARE SECTIONS" a type starts at column 0
/// and its further bodies are indented, and the count taken is each body's
/// `# SECTION` --- how many of that body the drawings place, which is what
/// a netlist's parts count too. `dip_census` below takes the packages
/// instead. The file's own header lines and its closing note are passed
/// over.
pub fn body_census(text: &str) -> BTreeMap<String, usize> {
    let mut census: BTreeMap<String, usize> = BTreeMap::new();
    let mut started = false;
    for line in text.lines() {
        if line.starts_with("DIPTYPE\tBODY NAME") {
            started = true;
            continue;
        }
        if !started || line.starts_with("NUMBER IN PARENS") {
            if started {
                break;
            }
            continue;
        }
        if line.starts_with("LISP") || line.starts_with("FILNAM") {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').filter(|f| !f.trim().is_empty()).collect();
        if fields.is_empty() || fields[0].trim() == "WITH LOCATIONS" {
            continue;
        }
        let (body, count) = if line.starts_with('\t') {
            if fields.len() < 2 {
                continue;
            }
            (fields[0].trim(), fields[1].trim())
        } else {
            if fields.len() < 3 {
                continue;
            }
            (fields[1].trim(), fields[2].trim())
        };
        let Ok(k) = count.parse::<usize>() else { continue };
        *census.entry(body.to_string()).or_default() += k;
    }
    census
}

/// MIT's wire list for a board, `mit/<parts>`, read against the netlist's
/// pages.
pub fn wire_list(n: &Netlist, parts: &[&str]) -> Vec<wirelist::Signal> {
    wirelist::parse(&mit_text(parts), &n.pages)
}

/// What a comparison against the wire list found, printed: how much of the
/// list landed on the board, the wires split over nets, and the nets that
/// hold several wires.
pub fn report(signals: &[wirelist::Signal], r: &wirelist::Report) {
    eprintln!(
        "{} signals, {} pins placed, {} on parts the netlist has not got",
        signals.len(),
        r.placed,
        r.unplaced
    );
    for (names, nets) in &r.split {
        eprintln!("  wire {names:?} is nets {nets:?}");
    }
    for (net, names) in &r.merged {
        eprintln!("  net {net} is wires {names:?}");
    }
}

/// The I/O board's inputs from off the Unibus as the far end holds them at
/// rest: an idle Chaosnet cable and a keyboard with nothing typed.
pub fn quiet() -> Vec<(&'static str, Level)> {
    IDLE_CHAOSNET.iter().chain(IDLE_KEYBOARD).copied().collect()
}

/// Boots an `rtl` machine over the Chaosnet server, serving `file_root`, to
/// the listener: the boot is at its prompt once the rows the `;Reading`
/// line is printed in hold more than 400 lit pixels, and a moment more for
/// the prompt to settle. Returns the microcycles it took.
pub fn boot_to_the_prompt(e: &mut Rtl, file_root: PathBuf) -> u64 {
    // The band this boots is System 100's, whose own Chaosnet numbers are
    // what `chaos::Config` defaults to. A band reached at the wrong pair
    // gets neither the time nor its files and stops to ask for the date;
    // System 304's wants [`CHAOS_304`], which its own test gives it.
    e.machine_mut().chaos.file_root = Some(file_root);
    e.machine_mut().chaos.time = Some(muir::chaos::time::TEST_UNIVERSAL);
    e.machine_mut().plug_chaos(0);
    wait_for_the_prompt(e)
}

/// The wait itself, for a machine whose Chaosnet is already plugged: a
/// test that wants the ether's log on has to turn it on after the plug,
/// so it does that and comes here.
pub fn wait_for_the_prompt(e: &mut Rtl) -> u64 {
    // The `;Reading at top level` line, which is the prompt appearing.
    // Where it lands depends on how tall the herald above it is --- six
    // lines on System 304's band, one fewer on System 100's --- so the
    // rows watched are the band the line falls in for either, and the
    // herald itself is above all of them.
    let reading = |e: &Rtl| {
        let tv = &e.machine().simpletv;
        (84..130usize)
            .flat_map(|y| (0..muir::simpletv::WIDTH).map(move |x| (x, y)))
            .filter(|&(x, y)| tv.pixel(x, y))
            .count()
            > 400
    };
    let mut ran = 0u64;
    while !reading(e) {
        for _ in 0..500_000 {
            e.step().expect("the boot halted");
        }
        ran += 500_000;
        assert!(ran < 100_000_000, "the listener never began reading");
    }
    // And a moment for the prompt to settle.
    for _ in 0..2_000_000 {
        e.step().expect("the boot halted");
    }
    ran + 2_000_000
}

/// A plain GIF reader for two-colour, uninterlaced frames: the rectangle
/// and a byte a pixel.  What the recorder writes is read back here rather
/// than trusted, by the recorder's own tests and by the CC diagnostics'.
pub type Frame = ((usize, usize, usize, usize), Vec<u8>);

pub fn decode_gif(g: &[u8]) -> Vec<Frame> {
    assert_eq!(&g[..6], b"GIF89a");
    let mut i = 13 + 6; // header, screen descriptor, two colours
    let mut frames = Vec::new();
    while i < g.len() {
        match g[i] {
            0x21 => {
                i += 2;
                while g[i] != 0 {
                    i += g[i] as usize + 1;
                }
                i += 1;
            }
            0x2c => {
                let u = |k: usize| u16::from_le_bytes([g[k], g[k + 1]]) as usize;
                let (l, t, w, h) = (u(i + 1), u(i + 3), u(i + 5), u(i + 7));
                i += 10;
                let min = g[i] as u32;
                i += 1;
                let mut data = Vec::new();
                while g[i] != 0 {
                    let n = g[i] as usize;
                    data.extend_from_slice(&g[i + 1..i + 1 + n]);
                    i += n + 1;
                }
                i += 1;
                frames.push(((l, t, w, h), lzw_decode(&data, min, w * h)));
            }
            0x3b => break,
            b => panic!("unexpected block {b:#x} at {i}"),
        }
    }
    frames
}

fn lzw_decode(data: &[u8], min: u32, n: usize) -> Vec<u8> {
    let clear = 1u32 << min;
    let end = clear + 1;
    let mut table: Vec<Vec<u8>> = Vec::new();
    let reset = |table: &mut Vec<Vec<u8>>| {
        table.clear();
        for k in 0..clear {
            table.push(vec![k as u8]);
        }
        table.push(Vec::new());
        table.push(Vec::new());
    };
    reset(&mut table);
    let mut width = min + 1;
    let (mut acc, mut nbits, mut pos) = (0u32, 0u32, 0usize);
    let mut out = Vec::new();
    let mut prev: Option<Vec<u8>> = None;
    loop {
        while nbits < width && pos < data.len() {
            acc |= (data[pos] as u32) << nbits;
            nbits += 8;
            pos += 1;
        }
        if nbits < width {
            break;
        }
        let code = acc & ((1 << width) - 1);
        acc >>= width;
        nbits -= width;
        if code == clear {
            reset(&mut table);
            width = min + 1;
            prev = None;
            continue;
        }
        if code == end {
            break;
        }
        let entry = if (code as usize) < table.len() {
            table[code as usize].clone()
        } else {
            let p = prev.as_ref().expect("a code past the table with nothing before it");
            let mut e = p.clone();
            e.push(p[0]);
            e
        };
        out.extend_from_slice(&entry);
        if let Some(p) = prev {
            let mut e = p;
            e.push(entry[0]);
            table.push(e);
            if table.len() == (1 << width) && width < 12 {
                width += 1;
            }
        }
        prev = Some(entry);
    }
    assert_eq!(out.len(), n, "pixels decoded");
    out
}

/// An MCR file holding these control store words and nothing else: the
/// control store section, then the empty A-memory section that ends a
/// file, in the layout `muir::mcr` reads. Written out here so that a
/// boot PROM this repository has no file of --- one word too long, one
/// carrying a bit the chips cannot hold, one that jumps somewhere else
/// --- can still be handed to the reader, or to `muir --prom`.
pub fn mcr(words: &[u64]) -> Vec<u8> {
    /// 32 bits in PDP-11 word order, as `mcr`'s reader takes them.
    fn u32_pdp(b: &mut Vec<u8>, v: u32) {
        b.extend_from_slice(&[(v >> 16) as u8, (v >> 24) as u8, v as u8, (v >> 8) as u8]);
    }
    let mut b = Vec::new();
    // Section 1, the control store, from address 0.
    u32_pdp(&mut b, 1);
    u32_pdp(&mut b, 0);
    u32_pdp(&mut b, words.len() as u32);
    for w in words {
        // Four 16-bit little-endian halves, most significant first; the
        // top one is zero, a microinstruction being 48 bits.
        for shift in [48, 32, 16, 0] {
            b.push((w >> shift) as u8);
            b.push((w >> (shift + 8)) as u8);
        }
    }
    // Section 4, A memory, empty: it is the one that ends the file.
    u32_pdp(&mut b, 4);
    u32_pdp(&mut b, 0);
    u32_pdp(&mut b, 0);
    b
}

// --- The boards as netlists, a machine with the pack, and typing at it -----

use std::path::Path;

use muir::cable::FarEnd;
use muir::chip::Chip;
use muir::clock::Behavioural;
use muir::disk_unit::{Geometry, Unit};
use muir::machine::Machine;
use muir::terminal::keyboard::{Keyboard, keysym};

/// The I/O board, `data/CADRIO.netlist`, as the model has it:
/// `netlist::parse`, which joins the two ends of every series resistor into
/// one net. Every pin the board was moved on against its drawings is
/// already in the file, so `parse` and `parse_wired` differ in the
/// resistors alone.
pub fn cadrio() -> Netlist {
    muir::netlist::parse(include_str!("../../data/CADRIO.netlist")).unwrap()
}

/// The five boards `muir --chip` boots, each parsed from `data/`: the
/// processor, the bus interface, memory, the I/O board and the display.
pub struct Netlists {
    pub cpu: Netlist,
    pub busint: Netlist,
    pub cadrm: Netlist,
    pub cadrio: Netlist,
    pub simpletv: Netlist,
}

pub fn netlists() -> Netlists {
    let parse = |s: &str| muir::netlist::parse(s).unwrap();
    Netlists {
        cpu: parse(include_str!("../../data/CADR.netlist")),
        busint: parse(include_str!("../../data/BUSINT.netlist")),
        cadrm: parse(include_str!("../../data/CADRM.netlist")),
        cadrio: cadrio(),
        simpletv: parse(include_str!("../../data/SIMPLETV.netlist")),
    }
}

/// The board as `muir --chip` runs it, `benchmark::chip` over
/// [`netlists`]: the processor chip, its clock and the far end of its
/// cables.
pub fn chip(n: &Netlists) -> (Chip, Behavioural, FarEnd) {
    muir::benchmark::chip(&n.cpu, &n.busint, &n.cadrm, &n.cadrio, &n.simpletv)
}

/// A machine with the boot PROM in place and the System 100 pack, `pack`,
/// as a T-300 on unit 0: what every boot off the pack starts from, on
/// whichever engine.
pub fn machine_with_pack(pack: &Path) -> Machine {
    let mut m = Machine::new();
    m.load_prom(&muir::prom::boot_prom());
    m.disk.attach(0, Unit::open(pack, Geometry::T300).expect("the System 100 pack"));
    m
}

/// Lit pixels in rows `rows` of the screen.
pub fn lit_rows(e: &Rtl, rows: std::ops::Range<usize>) -> usize {
    let tv = &e.machine().simpletv;
    rows.flat_map(|y| (0..muir::simpletv::WIDTH).map(move |x| (x, y)))
        .filter(|&(x, y)| tv.pixel(x, y))
        .count()
}

/// Types `text` at the machine as a viewer does: each character a key
/// down and up, Shift held around an upper-case letter or a shifted
/// symbol, and `\n` the Return key. The board takes one word at a time
/// and the microcode reads it within a few thousand microcycles, so the
/// machine runs between words; a word it never reads fails the test
/// instead of running for ever.
pub fn type_at(e: &mut Rtl, k: &mut Keyboard, text: &str) {
    for ch in text.chars() {
        let sym = if ch == '\n' { keysym::RETURN } else { ch as u32 };
        let shifted = ch.is_ascii_uppercase() || "~!@#$%^&*()_+{}|:\"<>?".contains(ch);
        if shifted {
            k.key(keysym::SHIFT_L, true);
        }
        k.key(sym, true);
        k.key(sym, false);
        if shifted {
            k.key(keysym::SHIFT_L, false);
        }
        let mut waited = 0u64;
        while k.pending() > 0 || e.machine().ioboard.keyboard_ready() {
            k.deliver(&mut e.machine_mut().ioboard);
            for _ in 0..1_000 {
                e.step().expect("halted while typing");
            }
            waited += 1_000;
            assert!(waited < 50_000_000, "the machine never read the keyboard");
        }
    }
}

// --- muir as a child of the tests: the command, what it writes, a deadline ---
// --- on it, and a directory of the test's own -------------------------------

/// `muir` itself, the binary Cargo built for these tests, with stdin closed
/// and no flags from the developer's own `~/.muirrc`: `MUIR_RC` names an
/// empty file, so the run is the flags the test gives and nothing else.  A
/// test that types at the prompt opens stdin as a pipe instead.
pub fn muir() -> std::process::Command {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_muir"));
    c.stdin(std::process::Stdio::null());
    c.env("MUIR_RC", "/dev/null");
    c
}

/// What a run wrote, stdout then stderr, as text.
pub fn text(out: &std::process::Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

/// How long a child of the tests is given to end, or to write a line that
/// is waited for, before it is taken to have hung: killed, and the test
/// failed with what it wrote.  The longest run under it is a few seconds of
/// `micro`; the allowance is for a loaded machine, not for the run.
pub const DEADLINE: std::time::Duration = std::time::Duration::from_secs(300);

/// A directory of the test's own, removed when the guard is dropped --- on
/// the way out of a failing test as much as a passing one.  It derefs to
/// its path.
pub struct Scratch(PathBuf);

/// `muir-<name>-<pid>` under the system's temporary directory, made here.
pub fn scratch(name: &str) -> Scratch {
    Scratch::at(std::env::temp_dir().join(format!("muir-{name}-{}", std::process::id())))
}

impl Scratch {
    /// The directory at `path`, made here and removed with the guard.
    pub fn at(path: PathBuf) -> Scratch {
        std::fs::create_dir_all(&path).unwrap();
        Scratch(path)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for Scratch {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One of a child's pipes, read by a thread of its own as the child writes
/// it, so that the pipe never fills and what has come so far can be looked
/// at, and waited for, while the child runs.
pub struct Gathered {
    bytes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Gathered {
    pub fn from(mut pipe: impl std::io::Read + Send + 'static) -> Gathered {
        let bytes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = bytes.clone();
        let reader = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                sink.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        Gathered { bytes, reader: Some(reader) }
    }

    /// What has come so far, as text.
    pub fn so_far(&self) -> String {
        String::from_utf8_lossy(&self.bytes.lock().unwrap()).into_owned()
    }

    /// Whether the writer has closed the pipe: nothing more will come.
    pub fn closed(&self) -> bool {
        self.reader.as_ref().is_none_or(|r| r.is_finished())
    }

    /// Waits until `what` holds of what has come, and says so; or until
    /// `within` is up, or the pipe is closed with `what` still not so, and
    /// says not.
    pub fn wait_for(&self, what: impl Fn(&str) -> bool, within: std::time::Duration) -> bool {
        let until = std::time::Instant::now() + within;
        loop {
            // The reader appends the last bytes before it finishes, so a
            // pipe seen closed is looked at once more.
            let closed = self.closed();
            if what(&self.so_far()) {
                return true;
            }
            if closed || std::time::Instant::now() >= until {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// [`Gathered::wait_for`] under [`DEADLINE`], failing the test with
    /// `why` and what came if `what` never holds.
    pub fn wait_until(&self, what: impl Fn(&str) -> bool, why: &str) {
        assert!(
            self.wait_for(what, DEADLINE),
            "{why}: not within {} seconds{}; what came:\n{}",
            DEADLINE.as_secs(),
            if self.closed() { ", and the pipe closed" } else { "" },
            self.so_far()
        );
    }

    /// The whole of what the pipe carried, once the writer has closed it.
    fn all(mut self) -> Vec<u8> {
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
        std::mem::take(&mut *self.bytes.lock().unwrap())
    }
}

/// A child of the tests --- `muir`, or `script(1)` with `muir` under it ---
/// with its stdout and stderr gathered as they are written, killed when
/// dropped so that a failing test leaves no machine running, and waited
/// for under [`DEADLINE`].
pub struct Child {
    child: std::process::Child,
    stdout: Option<Gathered>,
    stderr: Option<Gathered>,
}

impl Child {
    /// Spawns `c` with stdout and stderr piped and gathered; stdin is as
    /// `c` has it.
    pub fn spawn(c: &mut std::process::Command) -> std::io::Result<Child> {
        let mut child =
            c.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn()?;
        let stdout = Gathered::from(child.stdout.take().unwrap());
        let stderr = Gathered::from(child.stderr.take().unwrap());
        Ok(Child { child, stdout: Some(stdout), stderr: Some(stderr) })
    }

    /// The child's stdin, which the command opened as a pipe; taken once.
    pub fn stdin(&mut self) -> std::process::ChildStdin {
        self.child.stdin.take().expect("stdin opened as a pipe, and not taken before")
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// What the child has written on stdout so far.
    pub fn stdout(&self) -> &Gathered {
        self.stdout.as_ref().unwrap()
    }

    /// What the child has written on stderr so far.
    pub fn stderr(&self) -> &Gathered {
        self.stderr.as_ref().unwrap()
    }

    /// Waits for the child to end, within [`DEADLINE`], and gives what it
    /// wrote and how it ended, as `Command::output` does.  One still going
    /// at the deadline is killed, and the test fails with what it wrote.
    pub fn wait(mut self) -> std::process::Output {
        let until = std::time::Instant::now() + DEADLINE;
        let status = loop {
            match self.child.try_wait().unwrap() {
                Some(status) => break status,
                None if std::time::Instant::now() >= until => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    panic!(
                        "the child had not ended {} seconds on, and was killed; it wrote:\n{}{}",
                        DEADLINE.as_secs(),
                        self.stdout().so_far(),
                        self.stderr().so_far()
                    );
                }
                None => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        };
        let stdout = self.stdout.take().unwrap().all();
        let stderr = self.stderr.take().unwrap().all();
        std::process::Output { status, stdout, stderr }
    }

    /// Ends the child: killed, and reaped, as dropping it does.
    pub fn kill(self) {
        drop(self);
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The two ways a test runs a command: to its end, or as a [`Child`] to be
/// typed at, watched and waited for.
pub trait Run {
    /// Runs it to its end within [`DEADLINE`]: `Command::output`, with the
    /// deadline on it.
    fn run(&mut self) -> std::process::Output;

    /// Starts it as a [`Child`].  The binary is always there, so failing to
    /// start it is a failure.
    fn start(&mut self) -> Child;
}

impl Run for std::process::Command {
    fn run(&mut self) -> std::process::Output {
        self.start().wait()
    }

    fn start(&mut self) -> Child {
        Child::spawn(self).unwrap_or_else(|e| panic!("{self:?}: {e}"))
    }
}

// --- the structural checks every board's netlist gets, and MIT's census ------
// --- by part type -----------------------------------------------------------

/// The nets no part on the board drives, sorted by name: what arrives from
/// off the board, or an input a pinout wrongly calls one. Supplies, spares
/// and the unnamed nets are left out.
pub fn undriven_nets(n: &Netlist) -> Vec<&str> {
    let mut driven = BTreeSet::new();
    for p in &n.parts {
        let Some(po) = part::pinout(&p.kind) else { continue };
        for &(pin, net) in &p.pins {
            if po.drives(pin) {
                driven.insert(net);
            }
        }
    }
    let mut undriven: Vec<&str> = (0..n.nets.len() as u32)
        .filter(|id| !driven.contains(id))
        .map(|id| n.net(id))
        .filter(|name| !is_power(name) && !name.starts_with('@'))
        .collect();
    undriven.sort_unstable();
    undriven
}

/// Prints [`undriven_nets`], for the reports of what plugs into a board.
pub fn report_undriven_nets(n: &Netlist) {
    let undriven = undriven_nets(n);
    eprintln!("{} nets with no driver on this board:", undriven.len());
    eprintln!("{undriven:?}");
}

/// **A pinout must not claim an output on its own supply pin**, or past the
/// end of its package. The two-drivers check cannot catch this on a
/// tri-state or open-collector part, those being exempt from it, which is
/// how a `74S472` entry claiming an output on pin 10, its ground, survived
/// for a while. Every kind the board uses is checked once. A kind declared
/// with no package --- a resistor pack, or a part whose ground is not the
/// middle pin --- has no standard supply pins to check, which is what
/// `Pinout::package` being zero means.
pub fn no_pinout_drives_its_own_supply_pin(n: &Netlist) {
    let kinds: BTreeSet<&str> = n.parts.iter().map(|p| p.kind.as_str()).collect();
    for kind in kinds {
        let Some(po) = part::pinout(kind) else { continue };
        if po.package == 0 {
            continue;
        }
        for &pin in po.outputs {
            assert_ne!(pin, po.gnd(), "{kind} claims an output on pin {pin}, its ground");
            assert_ne!(pin, po.vcc(), "{kind} claims an output on pin {pin}, its supply");
            assert!(
                pin <= po.package,
                "{kind} claims an output on pin {pin} of a DIP{}",
                po.package
            );
        }
    }
}

/// **No part uses its own ground or supply pin.** A pin the netlist puts on
/// a part's ground or supply is a wrong pinout or a wrong package size.
/// Parts declared with no package are passed over as above. Returns how
/// many parts were checked.
pub fn no_part_touches_its_own_supply_pins(n: &Netlist) -> usize {
    let mut checked = 0;
    for p in &n.parts {
        let Some(po) = part::pinout(&p.kind) else { continue };
        if po.package == 0 {
            continue;
        }
        checked += 1;
        for &(pin, _) in &p.pins {
            assert_ne!(
                pin,
                po.gnd(),
                "{} {} ({}) uses pin {pin}, ground on a DIP{}",
                p.page,
                p.reference,
                p.kind,
                po.package
            );
            assert_ne!(
                pin,
                po.vcc(),
                "{} {} ({}) uses pin {pin}, the supply on a DIP{}",
                p.page,
                p.reference,
                p.kind,
                po.package
            );
        }
    }
    checked
}

/// MIT's DIP census by part type: the same `.wls` file [`body_census`] reads
/// for sections, read for devices. Under each type's bodies comes a `WITH
/// LOCATIONS` line whose numbers are the sections the sheets use of the
/// type, the packages they take and the sections left spare; the second is
/// the count of devices, and it is the type's rather than a body's --- in
/// `cadr1/busint.wls` the type `74LS74` is three bodies and twelve sections
/// in six packages. Type names are cut to seven characters there, so
/// `SIP220/330-8` reads `SIP220/`.
pub fn dip_census(text: &str) -> BTreeMap<String, usize> {
    let mut out: BTreeMap<String, usize> = BTreeMap::new();
    let mut kind = String::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split('\t').map(str::trim).filter(|c| !c.is_empty()).collect();
        let Some(&first) = cols.first() else { continue };
        if !line.starts_with('\t') {
            kind = first.to_string();
        } else if first == "WITH LOCATIONS" {
            let n: Vec<usize> = cols[1..].iter().filter_map(|c| c.parse().ok()).collect();
            if let Some(&dips) = n.get(1) {
                *out.entry(kind.clone()).or_default() += dips;
            }
        }
    }
    out
}
