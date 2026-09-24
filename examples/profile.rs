// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a band's microcycles go, workload by workload.
//!
//! Boots System 1001's pack (`tools/fetch-system-1001.sh`) with the test
//! harness's Chaosnet server at OZ, logs in, defines a set of workloads at
//! the listener and runs them one at a time, counting every control-store
//! address the engine executes. Each workload ends by writing a marker
//! file through the FILE service, which is how the run knows it is over:
//! nothing here reads the screen.
//!
//!     cargo run --release --example profile -- [micro|rtl] [cadr|quux|quux-4k|quux-16k] [workload ...]
//!
//! The machine is the CADR unless `quux` is named: QUUX, `--machine quux`.
//! `quux-4k` and `quux-16k` are QUUX with a PDL buffer of 4K or 16K words,
//! the sizes being measured for its next revision.
//! With no workloads named, all of them run, in the order below. For each,
//! it prints the microcycles, the macroinstructions --- executions of
//! `QMLP+2`, the dispatch on `M-INST-OP` (`uc-macrocode.lisp`) --- and their
//! ratio, where the microinstructions went by the category and source file
//! of the nearest `I-MEM` label in the symbol table, the hottest labels,
//! and the microcode's own meters out of A memory. On `rtl` it adds the
//! time stalled on the bus and the memory cycles.
//!
//! Every count includes the typing: the listener reads the form a
//! character at a time. `nil`, the empty form, measures that overhead on
//! its own.
//!
//! The microcode is whatever the pack's current microload is; `MUIR_UCODE`
//! names a directory holding another microcode's `ucadr.mcr`, `.tbl` and
//! `.sym`, which is loaded into MCR2 of the run's copy of the pack, made
//! current, and served as the error table and read as the symbols.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use muir::diskpack::{Command, Pack};
use muir::engine::Engine;
use muir::machine::Halt;
use muir::micro::Micro;
use muir::rtl::Rtl;
use muir::sym::{Space, Symbols};
use muir::terminal::keyboard::{Keyboard, keysym};

// The Chaosnet server and the typing are the test harness's: a CADR has no
// file server in it, and this program is a test run by hand.
#[path = "../tests/support/mod.rs"]
mod support;

/// LISPM-1 and OZ, as the release's `site/hosts.text` gives them.
const CHAOS_1001: (u16, u16) = (0o177201, 0o177200);

/// The workloads: a name, and the form that runs it. Each is typed as
/// `(progn <form> (w-done "<name>"))`.
const WORKLOADS: &[(&str, &str)] = &[
    ("nil", "nil"),
    (
        "compile",
        "(mapc #'compile '(w-ack w-fib w-cons w-muldiv w-float w-array w-sort w-bignum w-intern))",
    ),
    ("calls-ack", "(w-ack 2 300)"),
    ("calls-fib", "(w-fib 24)"),
    ("cons", "(w-cons)"),
    ("arith-muldiv", "(w-muldiv)"),
    ("float", "(w-float)"),
    ("array", "(w-array)"),
    ("sort", "(w-sort)"),
    ("bignum", "(w-bignum)"),
    ("intern", "(w-intern)"),
    ("print-scroll", "(w-print)"),
    ("compile-again", "(mapc #'compile '(w-ack w-fib w-cons w-muldiv))"),
    // Not run unless named: the cost of switching stack groups, from the
    // listener's own shallow stack and from 500 frames deep, where every
    // switch has the PDL buffer's resident words to write out.
    ("switch-shallow", "(w-switch 0)"),
    ("switch-deep", "(w-switch 500)"),
];

/// The workloads run when none is named: all but the switching ones.
const DEFAULT_WORKLOADS: usize = 13;

/// The definitions, typed once after login. `w-done` writes the marker.
const DEFINITIONS: &[&str] = &[
    "(defun w-done (name) (with-open-file (s (string-append \"OZ: //lispm//\" name \".done\") :direction :output) (print name s)))",
    "(defun w-ack (m n) (cond ((zerop m) (1+ n)) ((zerop n) (w-ack (1- m) 1)) (t (w-ack (1- m) (w-ack m (1- n))))))",
    "(defun w-fib (n) (if (< n 2) n (+ (w-fib (1- n)) (w-fib (- n 2)))))",
    "(defun w-cons () (dotimes (i 10000) (make-list 200)))",
    "(defun w-muldiv () (let ((s 0)) (dotimes (i 150000) (setq s (remainder (+ s (* i 7)) 1000003))) s))",
    "(defun w-float () (let ((x 1.0)) (dotimes (i 50000) (setq x (+ (* x 1.0001) 0.5))) x))",
    "(defun w-array () (let ((a (make-array 1000))) (dotimes (i 186) (dotimes (j 1000) (aset j a j) (aref a j)))))",
    "(defun w-sort () (let ((l nil)) (dotimes (i 3000) (push (random 100000) l)) (sort l #'<)))",
    "(defun w-bignum () (dotimes (i 21) (print (expt 3 300))))",
    "(defun w-intern () (dotimes (i 1500) (intern (format nil \"W-SYM-~D\" i))))",
    "(defun w-print () (dotimes (i 1000) (print i)))",
    "(defun w-switch (n) (if (zerop n) (progn (dotimes (i 2000) (process-allow-schedule)) 0) (1+ (w-switch (1- n)))))",
];

/// The microcode's meters, by their `A-MEM` names in the symbol table.
const METERS: &[&str] = &[
    "A-FIRST-LEVEL-MAP-RELOADS",
    "A-SECOND-LEVEL-MAP-RELOADS",
    "A-META-BITS-MAP-RELOADS",
    "A-PDL-BUFFER-READ-FAULTS",
    "A-PDL-BUFFER-WRITE-FAULTS",
    "A-FRESH-PAGE-COUNT",
    "A-DISK-PAGE-READ-COUNT",
    "A-DISK-PAGE-WRITE-COUNT",
];

/// What the two engines can say that the harness needs beyond [`Engine`].
trait Profiled: Engine {
    fn executed_pc(&self) -> Option<u16>;
    /// Nanoseconds stalled on the bus, memory cycles, the machine's
    /// nanoseconds, and the memory cache's hits and misses, where the
    /// engine keeps them.
    fn bus(&self) -> Option<[u64; 5]>;
}

impl Profiled for Micro {
    fn executed_pc(&self) -> Option<u16> {
        self.executed()
    }
    fn bus(&self) -> Option<[u64; 5]> {
        None
    }
}

impl Profiled for Rtl {
    fn executed_pc(&self) -> Option<u16> {
        self.executed()
    }
    fn bus(&self) -> Option<[u64; 5]> {
        let (h, m) = self.busint().cache().map_or((0, 0), |c| (c.hits, c.misses));
        Some([self.stalled_ns(), self.bus_cycles(), self.ns(), h, m])
    }
}

/// The source file of every label, from the `uc-*.lisp` files: a label is a
/// token at the start of a line inside a file's list.
fn label_files(ucadr: &Path) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(ucadr).expect("sys/ucadr") {
        let path = entry.unwrap().path();
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        if !stem.starts_with("uc-") || path.extension().is_none_or(|e| e != "lisp") {
            continue;
        }
        let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
        for line in text.lines() {
            let Some(c) = line.chars().next() else { continue };
            if c.is_whitespace() || c == '(' || c == ';' || c == ')' {
                continue;
            }
            let label: String =
                line.chars().take_while(|c| !c.is_whitespace() && *c != ';').collect();
            files.entry(label).or_insert_with(|| stem.clone());
        }
    }
    files
}

/// The category the 17 September 2026 profile used for a label in a file:
/// named families first, then the file.
fn category(label: &str, file: &str) -> String {
    let starts = |ps: &[&str]| ps.iter().any(|p| label.starts_with(p));
    if label.starts_with("QMLP") {
        "dispatch".into()
    } else if starts(&["P-R-", "P-B-"]) || label.contains("PDL-BUFFER") {
        "pdl-buffer".into()
    } else if starts(&["CZRR", "PHTDEL", "FINDCORE", "AGER", "SWAP", "PAGE-IN"]) {
        "paging".into()
    } else if starts(&["MPY", "DIV", "DIVIDE-ONCE", "BMPY", "BIDIV", "BFXMPY", "FDIV"]) {
        "mpy/div".into()
    } else {
        match file {
            "uc-page-fault" => "map".into(),
            "uc-macrocode" => "macro-ops".into(),
            "uc-call-return" => "call-return".into(),
            "uc-fctns" => "fctns".into(),
            "uc-array" => "array".into(),
            "uc-arith" => "arith".into(),
            "uc-storage-allocation" => "alloc".into(),
            "uc-transporter" => "transporter".into(),
            "uc-tv" => "tv".into(),
            other => other.trim_start_matches("uc-").into(),
        }
    }
}

struct Phase {
    cycles: u64,
    hist: Vec<u64>,
    meters: Vec<u32>,
    bus: Option<[u64; 5]>,
}

fn meters(e: &impl Engine, syms: &Symbols) -> Vec<u32> {
    METERS
        .iter()
        .map(|m| syms.address(Space::AMem, m).map(|a| e.machine().amem[a as usize]).unwrap_or(0))
        .collect()
}

/// The screen, folded to a number: changed when it has.
fn screen_hash(e: &impl Engine) -> u64 {
    e.machine()
        .tv
        .buffer()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, &w| (h ^ w as u64).wrapping_mul(0x100_0000_01b3))
}

/// Types `text` a key at a time, each taken off the keyboard by the
/// microcode and then echoed by Lisp before the next, as `tests/cc_harness`
/// does: the microcode's keyboard buffer holds 64 characters and Lisp
/// empties it only when its process runs, so a line typed faster wraps it
/// and arrives garbled. A key not echoed within twenty million microcycles
/// goes on anyway. Every microcycle is run by `step`.
fn type_echoed<E: Engine>(e: &mut E, k: &mut Keyboard, text: &str, step: &mut impl FnMut(&mut E)) {
    for ch in text.chars() {
        let before = screen_hash(e);
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
                step(e);
            }
            waited += 1_000;
            assert!(waited < 50_000_000, "the machine never read the keyboard");
        }
        let mut echoed = 0u64;
        while screen_hash(e) == before && echoed < 20_000_000 {
            for _ in 0..50_000 {
                step(e);
            }
            echoed += 50_000;
        }
    }
}

/// Types `form` and runs until `marker` appears, counting what executes.
fn run<E: Profiled>(
    e: &mut E,
    k: &mut Keyboard,
    form: &str,
    marker: &Path,
    syms: &Symbols,
) -> Phase {
    let before = meters(e, syms);
    let bus0 = e.bus();
    let mut hist = vec![0u64; 1 << 14];
    let cycles0 = e.machine().cycles;
    // Typing steps the engine too, so count through it.
    let mut step = |e: &mut E| {
        if let Err(Halt::UnknownDest { pc, dest }) = e.step() {
            panic!("halted at {pc:o} on destination {dest:o}");
        }
        if let Some(pc) = e.executed_pc() {
            hist[pc as usize] += 1;
        }
    };
    // No Return: the listener runs a form when its last parenthesis is in,
    // and a Return after it would wait in the buffer as typeahead.
    type_echoed(e, k, form, &mut step);
    let mut ran = 0u64;
    while !marker.exists() {
        for _ in 0..100_000 {
            step(e);
        }
        ran += 100_000;
        if ran >= 2_000_000_000 {
            // What the listener says is the only account of why.
            // Outside the run's scratch directory, which goes with the panic.
            let shot = std::env::temp_dir().join(format!(
                "muir-profile-{}.gif",
                marker.file_stem().unwrap().to_string_lossy()
            ));
            let mut rec = muir::capture::Recorder::new(false);
            rec.sample(&e.machine().tv, 0, 0);
            std::fs::write(&shot, rec.gif()).unwrap();
            panic!("{} never came; the screen is {}", marker.display(), shot.display());
        }
    }
    let after = meters(e, syms);
    let bus = match (bus0, e.bus()) {
        (Some(a), Some(b)) => Some(std::array::from_fn(|i| b[i] - a[i])),
        _ => None,
    };
    Phase {
        cycles: e.machine().cycles - cycles0,
        hist,
        meters: after.iter().zip(&before).map(|(a, b)| a.wrapping_sub(*b)).collect(),
        bus,
    }
}

fn report(name: &str, p: &Phase, syms: &Symbols, files: &BTreeMap<String, String>, qmlp: u32) {
    let executed: u64 = p.hist.iter().sum();
    let macros = p.hist[(qmlp + 2) as usize];
    println!("== {name}");
    println!(
        "   {} microcycles, {} executed, {} macroinstructions, {:.1} microcycles each",
        p.cycles,
        executed,
        macros,
        p.cycles as f64 / macros.max(1) as f64
    );
    if let Some([stalled, bus, ns, hits, misses]) = p.bus {
        println!("   stalled {stalled} ns, {bus} memory cycles, {ns} ns in all");
        if hits + misses > 0 {
            println!("   cache {hits} hits, {misses} misses");
        }
    }
    let mut by_cat: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_label: BTreeMap<String, u64> = BTreeMap::new();
    for (pc, &n) in p.hist.iter().enumerate() {
        if n == 0 {
            continue;
        }
        let label = syms
            .nearest(Space::IMem, pc as u32)
            .map(|(l, _)| l.to_string())
            .unwrap_or_else(|| "?".into());
        let file = files.get(&label).map(String::as_str).unwrap_or("?");
        *by_cat.entry(category(&label, file)).or_default() += n;
        *by_label.entry(label).or_default() += n;
    }
    let pct = |n: u64| 100.0 * n as f64 / executed.max(1) as f64;
    let mut cats: Vec<_> = by_cat.into_iter().collect();
    cats.sort_by_key(|c| std::cmp::Reverse(c.1));
    let line: Vec<String> = cats.iter().map(|(c, n)| format!("{c} {:.1}", pct(*n))).collect();
    println!("   {}", line.join(", "));
    let mut labels: Vec<_> = by_label.into_iter().collect();
    labels.sort_by_key(|l| std::cmp::Reverse(l.1));
    let top: Vec<String> =
        labels.iter().take(6).map(|(l, n)| format!("{l} {:.1}", pct(*n))).collect();
    println!("   hottest: {}", top.join(", "));
    let m: Vec<String> = METERS.iter().zip(&p.meters).map(|(n, v)| format!("{n} {v}")).collect();
    println!("   {}", m.join(", "));
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let engine = if args.first().is_some_and(|a| a == "micro" || a == "rtl") {
        args.remove(0)
    } else {
        "micro".into()
    };
    use muir::machine::Geometry;
    let geometry = match args.first().map(String::as_str) {
        Some("cadr") => Some(Geometry::CADR),
        Some("quux") => Some(Geometry::QUUX),
        Some("quux-4k") => Some(Geometry { pdl_bits: 12, ..Geometry::QUUX }),
        Some("quux-16k") => Some(Geometry { pdl_bits: 14, ..Geometry::QUUX }),
        _ => None,
    };
    let geometry = match geometry {
        Some(g) => {
            args.remove(0);
            g
        }
        None => Geometry::CADR,
    };
    let wanted: Vec<&(&str, &str)> = if args.is_empty() {
        WORKLOADS[..DEFAULT_WORKLOADS].iter().collect()
    } else {
        args.iter()
            .map(|a| {
                WORKLOADS.iter().find(|(n, _)| n == a).unwrap_or_else(|| panic!("no workload {a}"))
            })
            .collect()
    };
    match engine.as_str() {
        "rtl" => {
            // `MUIR_SYNC_TICKS` runs QUUX's synchronous microcycle of that
            // many ticks, and `MUIR_CACHE` fits a memory cache of that many
            // words (H1a, H2).
            let ticks = std::env::var("MUIR_SYNC_TICKS").ok().and_then(|v| v.parse().ok());
            let cache = std::env::var("MUIR_CACHE")
                .ok()
                .and_then(|v| v.parse().ok())
                .map(muir::cache::CacheConfig::with_words);
            // `MUIR_MEMORY_NS=<read>,<write>` times main memory as QUUX's
            // own, in place of the CADR's boards.
            let memory = std::env::var("MUIR_MEMORY_NS").ok().and_then(|v| {
                match v.as_str() {
                    "arty" => return Some(muir::cache::MemoryTiming::ARTY_Z7_20),
                    "de25" => return Some(muir::cache::MemoryTiming::DE25_NANO),
                    _ => {}
                }
                let (r, w) = v.split_once(',')?;
                Some(muir::cache::MemoryTiming {
                    read_ns: r.parse().ok()?,
                    write_ns: w.parse().ok()?,
                })
            });
            profile(
                move |m| {
                    let mut e = Rtl::new(m);
                    if let Some(cycle_ticks) = ticks {
                        e.set_timing_model(muir::clock::TimingModel::Sync {
                            cycle_ticks,
                            ilong_ticks: 0,
                        });
                    }
                    e.set_cache(cache);
                    e.set_memory_timing(memory);
                    e
                },
                geometry,
                &wanted,
            )
        }
        _ => profile(Micro::new, geometry, &wanted),
    }
}

fn profile<E: Profiled>(
    make: impl Fn(muir::machine::Machine) -> E,
    geometry: muir::machine::Geometry,
    wanted: &[&(&str, &str)],
) {
    let (Some(pack), Some(sources)) =
        (support::vendor(&["run", "release-1001-pack.img"]), support::vendor(&["system-1001"]))
    else {
        return;
    };
    let dir = support::scratch("profile");
    let copy = dir.join("pack.img");
    std::fs::copy(&pack, &copy).unwrap();
    let root = dir.join("root");
    let home = root.join("lispm");
    std::fs::create_dir_all(&home).unwrap();
    std::os::unix::fs::symlink(sources.join("site"), root.join("site")).unwrap();

    // Another microcode, when asked for: into MCR2, current, and its table
    // and symbols the ones served and read.
    let ucode = std::env::var_os("MUIR_UCODE").map(PathBuf::from);
    let sym_file = match &ucode {
        Some(u) => {
            let (mut p, _) = Pack::open(&copy);
            p.run(Command::Load { partition: "MCR2".into(), file: Some(u.join("ucadr.mcr")) })
                .unwrap();
            p.run(Command::Microload("MCR2".into())).unwrap();
            let sys = root.join("sys");
            std::fs::create_dir_all(sys.join("ubin")).unwrap();
            for entry in std::fs::read_dir(sources.join("sys")).unwrap() {
                let entry = entry.unwrap();
                if entry.file_name() != "ubin" {
                    std::os::unix::fs::symlink(entry.path(), sys.join(entry.file_name())).unwrap();
                }
            }
            for entry in std::fs::read_dir(sources.join("sys/ubin")).unwrap() {
                let entry = entry.unwrap();
                if entry.file_name() != "ucadr.tbl" && entry.file_name() != "ucadr.sym" {
                    std::os::unix::fs::symlink(
                        entry.path(),
                        sys.join("ubin").join(entry.file_name()),
                    )
                    .unwrap();
                }
            }
            std::fs::copy(u.join("ucadr.tbl"), sys.join("ubin/ucadr.tbl")).unwrap();
            u.join("ucadr.sym")
        }
        None => {
            std::os::unix::fs::symlink(sources.join("sys"), root.join("sys")).unwrap();
            sources.join("sys/ubin/ucadr.sym")
        }
    };
    let syms = muir::sym::parse(&std::fs::read_to_string(&sym_file).unwrap()).unwrap();
    let qmlp = syms.address(Space::IMem, "QMLP").expect("QMLP in the symbol table");
    let files = label_files(&sources.join("sys/ucadr"));

    let mut m = support::machine_with_pack(&copy);
    m.geometry = geometry;
    // QUUX boots from its own PROM, which a PDL buffer wider than 1K needs.
    if geometry != muir::machine::Geometry::CADR {
        m.load_prom(&muir::prom::quux_boot_prom());
    }
    let mut e = make(m);
    e.boot();
    let ran = support::boot_to_the_prompt_within(&mut e, CHAOS_1001, root.clone(), 400_000_000);
    eprintln!("listener after {ran} microcycles");
    let mut k = Keyboard::new();
    let mut plain = |e: &mut E| {
        e.step().expect("halted");
    };
    type_echoed(&mut e, &mut k, "(login \"LISPM\" \"OZ\" t)", &mut plain);
    // No **MORE** break when the printing workloads fill the screen: it
    // waits for a key that nobody types.
    type_echoed(&mut e, &mut k, "(send terminal-io :set-more-p nil)", &mut plain);
    for d in DEFINITIONS {
        type_echoed(&mut e, &mut k, d, &mut plain);
    }
    // Compiled before any workload runs, so that each is measured compiled
    // whichever are asked for; `compile` and `compile-again` compile them
    // again.
    type_echoed(
        &mut e,
        &mut k,
        "(mapc #'compile '(w-done w-ack w-fib w-cons w-muldiv w-float w-array w-sort w-bignum w-intern w-print w-switch))",
        &mut plain,
    );
    let ready = home.join("ready.done");
    let p = run(&mut e, &mut k, "(w-done \"ready\")", &ready, &syms);
    eprintln!("defined and logged in, {} microcycles", p.cycles);

    // A marker of its own for every run, so that a workload named twice is
    // run twice.
    for (n, (name, form)) in wanted.iter().enumerate() {
        let marker = home.join(format!("{name}-{n}.done"));
        let p =
            run(&mut e, &mut k, &format!("(progn {form} (w-done \"{name}-{n}\"))"), &marker, &syms);
        report(name, &p, &syms, &files, qmlp);
    }
}
