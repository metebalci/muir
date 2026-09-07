// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the CC tests share: two `rtl` machines under [`Lashup`], A booting
//! System 100 from the pack with the Chaosnet server serving the release as
//! `SYS:`, B running the boot PROM with no pack; A typed at through its
//! keyboard; CC loaded over the FILE service; the two screens recorded on
//! one canvas as they go; and a way to run a form on A and get what it
//! printed back over the FILE service.
//!
//! The Chaosnet server's root is `vendor/run/file-root`, and its `tmp` is
//! where a form's output goes: one directory, emptied when a run starts.
//! So the tests built on this harness run one at a time, whatever the test
//! runner's threads: the lock is taken as the machines are built and held
//! as long as the [`Cc`] lives.
//!
//! `MUIR_CHAOS_TRACE`, set to anything, has both machines' Chaosnet
//! interfaces say what they take and drop on their cables, as it does
//! under `muir` itself.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use muir::capture::Recorder;
use muir::chaos::ether::Event;
use muir::chaos::packet::Packet;
use muir::chaos::server::op;
use muir::disk_unit::{Geometry, Unit};
use muir::engine::Engine;
use muir::lashup::Lashup;
use muir::machine::Machine;
use muir::mcr;
use muir::rtl::Rtl;
use muir::simpletv::WIDTH;
use muir::terminal::keyboard::{Keyboard, keysym};

pub use crate::support::{pack_100, vendor};

/// The two machines, A's keyboard, the FILE service's root and the
/// recording of the two screens.
pub struct Cc {
    pub l: Lashup,
    pub k: Keyboard,
    pub root: PathBuf,
    /// Both screens on one canvas: A's, where CC is typed, at the left,
    /// and B's, the machine under test, at the right.  B shows nothing
    /// while nothing writes its frame buffer, and that is what a frame of
    /// it says.
    pub rec: Recorder,
    /// The release A booted, which says where its sources are and what its
    /// Chaosnet numbers were.
    pub release: Release,
    /// Microcycles of A with neither cable moving after which a form asked
    /// of it is given up on.  [`Cc::STALL`] to begin with, which is right
    /// for a form that runs; a form that *compiles* is quiet for as long
    /// as a file takes and raises it.
    pub stall: u64,
    next_sample: u64,
    /// The Chaosnet server's root, held for as long as the machines are up.
    _root_held: MutexGuard<'static, ()>,
}

/// Which release machine A boots.
///
/// The two are different machines and want different Chaosnet numbers, and
/// their sources are in different places: System 100's under
/// `vendor/system-100-0/sys`, System 304's under
/// `vendor/system-304-0/sys-304-0`, each linked into the FILE service's
/// root under the name its own band asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    /// **The target.** Its `SYS: CC;` is shipped compiled, so CC loads
    /// from the release's own QFASLs.
    System100,
    /// The release that continues it.  It ships no QFASL for CC, so CC is
    /// compiled from source before it can be loaded, which is twenty-odd
    /// minutes of the machine's time.
    System304,
}

impl Release {
    /// The pack this release boots from, or `None` with the skip line if
    /// its fetch script has not been run.
    pub fn pack(self) -> Option<PathBuf> {
        match self {
            Release::System100 => pack_100(),
            Release::System304 => crate::support::pack_304(),
        }
    }

    /// The boot PROM's image in this release's `SYS: UBIN;`.  Both
    /// releases carry the same `promh.mcr`, and each is read from its own
    /// rather than from the other's.
    pub fn prom(self) -> Option<PathBuf> {
        match self {
            Release::System100 => vendor(&["system-100-0", "sys", "ubin", "promh.mcr"]),
            Release::System304 => vendor(&["system-304-0", "sys-304-0", "ubin", "promh.mcr"]),
        }
    }

    /// The name the server answers `STATUS` with: `MIT-OZ`, which is
    /// System 100's `sys/site/hosts.text` name for 3060, and `OZ` for
    /// System 304, the name its band resolves to 4403 --- asked at its
    /// listener, `(send (si:parse-host "OZ") :chaos-address)` answers
    /// 2307 decimal.
    ///
    /// **Unverified** that `OZ` is that host's own name there rather than
    /// a nickname of it: System 304's host table is in the pack and not
    /// in the sources, so there is no file here to read it from, and the
    /// name is seen only in a `STATUS` answer, which nothing in these
    /// tests reads. `(si:get-host-from-address #o4403 :chaos)`
    /// (`network/host.lisp`) asked at the band's listener would settle it.
    pub fn server_name(self) -> String {
        match self {
            Release::System100 => muir::chaos::Config::default().server_name,
            Release::System304 => "OZ".to_string(),
        }
    }

    /// This machine's Chaosnet address and its file and time host's, as
    /// this release's band holds them: `MIT-LISPM-1` at 3050 with `MIT-OZ`
    /// at 3060, `AMS-LISPM-1` at 4401 with `OZ` at 4403.  A server
    /// answering anywhere else is a server the band never calls.
    pub fn chaos(self) -> (u16, u16) {
        match self {
            Release::System100 => (
                muir::chaos::Config::default().address,
                muir::chaos::Config::default().server_address,
            ),
            Release::System304 => crate::support::CHAOS_304,
        }
    }
}

/// The one Chaosnet server root the tests share, `vendor/run/file-root`:
/// a run that has it keeps every other waiting.  A test that failed while
/// holding it leaves it poisoned, which says nothing about the directory,
/// so the next run takes it all the same.
static SHARED_ROOT: Mutex<()> = Mutex::new(());

/// Lit pixels in rows `rows` of A's screen.
pub fn lit_rows(l: &Lashup, rows: std::ops::Range<usize>) -> usize {
    let tv = &l.debugger.machine().simpletv;
    rows.flat_map(|y| (0..WIDTH).map(move |x| (x, y))).filter(|&(x, y)| tv.pixel(x, y)).count()
}

impl Cc {
    /// Microcycles of A with nothing on the cable after which a form asked
    /// of it is given up on: a minute and a half of the machine's time.
    pub const STALL: u64 = 600_000_000;

    /// Microcycles of A between screen samples: about a third of a second
    /// of the machine's time.
    const SAMPLE_EVERY: u64 = 2_000_000;

    /// Steps the lashup until A has taken `n` more microcycles, sampling
    /// both screens as it goes.  One sample of the pair, timed by A's
    /// clock: the lashup steps neither machine past the other's promise,
    /// so a frame is one instant on both.
    pub fn run(&mut self, n: u64) {
        let from = self.l.steps.0;
        while self.l.steps.0 < from + n {
            self.l.step().expect("a machine halted");
            if self.l.steps.0 >= self.next_sample {
                self.rec.sample_pair(
                    &self.l.debugger.machine().simpletv,
                    &self.l.debuggee.machine().simpletv,
                    self.l.debugger.ns(),
                    muir::capture::wall_clock(),
                );
                self.next_sample = self.l.steps.0 + Self::SAMPLE_EVERY;
            }
        }
    }

    /// Writes the recording where `path` says, and says how big it came.
    pub fn write_recording(&self, path: &Path) {
        let gif = self.rec.gif();
        std::fs::write(path, &gif).unwrap();
        eprintln!(
            "both screens recorded at {}: {} frames of {} samples, {} bytes",
            path.display(),
            self.rec.frames(),
            self.rec.samples(),
            gif.len()
        );
    }

    /// The recording under `vendor/run` as `<name>.gif`, and its path.
    pub fn save_recording(&self, name: &str) -> PathBuf {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("vendor/run/{name}.gif"));
        self.write_recording(&p);
        p
    }

    /// Types `text` and Return at A, a key at a time, each acknowledged
    /// before the next: taken off the keyboard by the microcode, and then
    /// echoed --- the screen changed --- by Lisp.  Timing alone will not
    /// do: the microcode's keyboard buffer holds 64 characters and Lisp
    /// takes them out only when its process runs, so a long line typed as
    /// fast as the microcode takes it wraps the buffer and arrives
    /// garbled.  A key Lisp does not echo within a while goes on anyway,
    /// so that typing at something that shows nothing still ends.
    pub fn type_line(&mut self, text: &str) {
        self.type_keys(text.bytes().map(|b| b as u32).chain([keysym::RETURN]).collect());
    }

    /// Types `form` at A with no Return after it: the listener runs a form
    /// as soon as its last parenthesis is in, and a Return typed after it
    /// stays in the keyboard buffer as typeahead --- which CC's test loops
    /// take as a request to break.
    pub fn type_form(&mut self, form: &str) {
        self.type_keys(form.bytes().map(|b| b as u32).collect());
    }

    fn type_keys(&mut self, keys: Vec<u32>) {
        for sym in keys {
            let before = self.screen_hash();
            self.k.key(sym, true);
            self.k.key(sym, false);
            let mut waited = 0;
            while self.k.pending() > 0 || self.l.debugger.machine().ioboard.keyboard_ready() {
                self.k.deliver(&mut self.l.debugger.machine_mut().ioboard);
                self.run(1_000);
                waited += 1_000;
                assert!(waited < 50_000_000, "A never read the keyboard");
            }
            let mut echoed = 0;
            while self.screen_hash() == before && echoed < 20_000_000 {
                self.run(50_000);
                echoed += 50_000;
            }
        }
    }

    /// A's screen, folded to a number: changed when it has.
    fn screen_hash(&self) -> u64 {
        self.l
            .debugger
            .machine()
            .simpletv
            .buffer()
            .iter()
            .fold(0xcbf2_9ce4_8422_2325u64, |h, &w| (h ^ w as u64).wrapping_mul(0x100_0000_01b3))
    }

    /// Runs until the screen's rows `rows` change from `was`, within
    /// `limit` microcycles of A's, and a while after for the rest of the
    /// line.
    pub fn until_written(&mut self, rows: std::ops::Range<usize>, was: usize, limit: u64) {
        let from = self.l.steps.0;
        while lit_rows(&self.l, rows.clone()) == was {
            self.run(1_000_000);
            assert!(self.l.steps.0 - from < limit, "nothing appeared in rows {rows:?}");
        }
        self.run(5_000_000);
    }

    /// Data frames that have crossed A's cable so far.
    pub fn data_frames(&self) -> usize {
        let ether = self.l.debugger.machine().ioboard.chaos.as_ref().unwrap().ether().unwrap();
        ether
            .log
            .iter()
            .filter(|e| {
                let buffer = match e {
                    Event::Sent(_, _, buffer) => buffer,
                    Event::Heard(_, f) => &f.buffer,
                    Event::Collision(_) => return false,
                };
                Packet::from_buffer(buffer).is_ok_and(|(p, _)| op::is_data(p.opcode))
            })
            .count()
    }

    /// Runs until no file data has crossed the cable for a while: a load
    /// over the FILE service is done.  The screen will not do, its cursor
    /// blinking.
    pub fn until_cable_quiet(&mut self, limit: u64) -> usize {
        let from = self.l.steps.0;
        let mut quiet = 0;
        let mut last = self.data_frames();
        while quiet < 20 {
            self.run(5_000_000);
            let now = self.data_frames();
            if now == last {
                quiet += 1;
            } else {
                quiet = 0;
                last = now;
            }
            assert!(self.l.steps.0 - from < limit, "the cable never went quiet");
        }
        last
    }

    /// Types `form` at A with its standard output on the screen and on a
    /// file the Chaosnet server keeps, `tmp/<name>.text` under the FILE
    /// service's root, and runs until the file is closed --- within
    /// `limit` microcycles of A's --- then returns what was printed, the
    /// machine's newlines made ours.  The band spells a Unix slash
    /// doubled, as its `sys.translations` does: `OZ://tmp//name.text`.
    ///
    /// Both places, through the band's own `make-broadcast-stream`
    /// (`sys/io/qio.lisp`), which passes every operation to every stream
    /// it was given: the file is what is read back, and the screen is so
    /// that a run of minutes shows what it is doing rather than standing
    /// still.
    pub fn ask(&mut self, name: &str, form: &str, limit: u64) -> String {
        let tmp = self.root.join("tmp");
        let file = tmp.join(format!("{name}.text"));
        let _ = std::fs::remove_file(&file);
        // The previous form's temporary may still be there, its close not
        // yet through, and it ends with the marker too: only temporaries
        // that appear from here on are this form's.
        let before: std::collections::HashSet<PathBuf> = temp_files(&tmp).into_iter().collect();
        // The form prints a marker at the end, so completion can be seen
        // in the FILE service's own temporary before its close renames it
        // into place --- the close is a slow round trip over the model
        // network, and the debug cable, which the diagnostic drove, goes
        // quiet the moment the diagnostic is done, well before the file is
        // there.
        let line = format!(
            "(with-open-file (f \"OZ://tmp//{name}.text\" :direction :output) \
             (let* ((both (make-broadcast-stream terminal-io f)) \
             (standard-output both) (*standard-output* both)) {form}) \
             (format f \"~%~%*DONE*~%\"))"
        );
        self.type_form(&line);
        let from = self.l.steps.0;
        // The answer is done when the file --- placed, or still the
        // service's temporary --- ends with the marker.  Given up on only
        // when the Chaosnet cable itself has gone quiet for a long while,
        // stuck, or at `limit`.
        // Progress is either cable moving: the debug cable while the
        // diagnostic drives B, the Chaosnet while A flushes its output. A
        // is stuck only when neither has moved for a long while.
        let mut reported = 0;
        let progress = |cc: &Cc| (cc.l.debugger.debug_cycles(), cc.data_frames());
        let (mut moved, mut moved_at) = (progress(self), from);
        let done = |tmp: &std::path::PathBuf, file: &std::path::PathBuf| -> Option<Vec<u8>> {
            let fresh = temp_files(tmp).into_iter().filter(|t| !before.contains(t));
            for path in [file.clone()].into_iter().chain(fresh) {
                if let Ok(b) = std::fs::read(&path)
                    && ends_with_marker(&b)
                {
                    return Some(b);
                }
            }
            None
        };
        let bytes = loop {
            if let Some(b) = done(&tmp, &file) {
                break b;
            }
            self.run(5_000_000);
            let gone = self.l.steps.0 - from;
            let now = progress(self);
            if now != moved {
                (moved, moved_at) = (now, self.l.steps.0);
            }
            if gone / 1_000_000_000 > reported {
                reported = gone / 1_000_000_000;
                eprintln!(
                    "  {name}: {gone} microcycles on, {} debug cycles on the cable",
                    self.l.debugger.debug_cycles()
                );
            }
            if self.l.steps.0 - moved_at > self.stall || gone > limit {
                self.screenshot(&format!("cc-{name}-timeout"));
                // Where each machine stands, for whoever reads the failure.
                for (who, m) in [("A", &self.l.debugger), ("B", &self.l.debuggee)] {
                    eprintln!(
                        "  {who}: PC {:o}, FLAG-1 {:o}, disk status {:o}, bus error {:o}, {} ns",
                        m.pc(),
                        m.spy_read(muir::spy::FLAG_1),
                        m.machine().disk.status(),
                        m.machine().bus_error,
                        m.machine().ns
                    );
                    let mm = m.machine();
                    let stack: Vec<String> = (0..=mm.spcptr as usize)
                        .map(|i| format!("{:o}", mm.spc[i] & 0o37777))
                        .collect();
                    eprintln!(
                        "  {who}: M-1 {:o}, A-ERROR-SUBSTATUS {:o}, SPC [{}]",
                        mm.mmem[1],
                        mm.amem[0o30],
                        stack.join(" ")
                    );
                }
                panic!(
                    "{name}: no answer, neither cable moved for {} microcycles, {gone} on",
                    self.l.steps.0 - moved_at
                );
            }
        };
        eprintln!("  {name}: answered after {} microcycles", self.l.steps.0 - from);
        let text: String = bytes
            .iter()
            .map(|&b| if b == 0o215 || b == b'\n' { '\n' } else { (b & 0x7f) as char })
            .collect();
        // Everything up to the marker.
        match text.rfind("*DONE*") {
            Some(i) => text[..i].trim_end().to_string(),
            None => text,
        }
    }

    /// A's screen as a PNG under `vendor/run`.
    pub fn screenshot(&self, name: &str) -> PathBuf {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("vendor/run/{name}.png"));
        std::fs::write(&p, self.l.debugger.machine().simpletv.png()).unwrap();
        eprintln!("A's screen at {}", p.display());
        p
    }
}

/// Boots A to its listener with B on the cable, logs in and loads CC
/// over the FILE service.  `None`, with a word, if the vendored pack or
/// file root is missing.
pub fn boot_and_load_cc() -> Option<Cc> {
    load_cc(boot_and_login()?)
}

/// The same, with B booting A's pack as well: the one image, opened
/// read-only by both drives with each machine's written blocks kept in its
/// own memory, so that B is a machine with a band up rather than a PROM
/// waiting on a drive that is not there.  B has a Chaosnet of its own, so
/// its band takes the time from it and boots on to a listener instead of
/// stopping in the cold-load debugger to ask for the date.
pub fn boot_and_load_cc_with_debuggee_pack() -> Option<Cc> {
    load_cc(boot_and_login_with(true)?)
}

/// Types `make-system` at a booted A and waits the load out.
fn load_cc(mut cc: Cc) -> Option<Cc> {
    cc.type_line("(make-system 'cc :noconfirm :nowarn)");
    // Sixteen files over the FILE service: minutes.
    let frames = cc.until_cable_quiet(6_000_000_000);
    eprintln!("CC loaded by {} microcycles: {frames} data frames on the cable", cc.l.steps.0);
    Some(cc)
}

/// Boots A to its listener with B on the cable and logs in.  B runs the
/// boot PROM with no pack.
pub fn boot_and_login() -> Option<Cc> {
    boot_and_login_with(false)
}

/// The same, `debuggee_pack` saying whether B has A's pack under it too.
///
/// **This runs the target, System 100.**  [`boot_and_login_on`] boots
/// either release; the tests that debug B through CC take this one,
/// because the acceptance test is the target's.
pub fn boot_and_login_with(debuggee_pack: bool) -> Option<Cc> {
    boot_and_login_on(Release::System100, debuggee_pack)
}

/// Boots A on `release` to its listener with B on the cable and logs in,
/// B running the boot PROM with `debuggee_pack` saying whether A's pack is
/// under it as well.
///
/// The release decides three things and nothing else: which pack A boots,
/// which Chaosnet numbers its band calls with, and which tree the FILE
/// service's link points at.  What is typed afterwards is the caller's.
pub fn boot_and_login_on(release: Release, debuggee_pack: bool) -> Option<Cc> {
    let (Some(prom), Some(pack), Some(root)) =
        (release.prom(), release.pack(), vendor(&["run", "file-root"]))
    else {
        return None;
    };
    // One run at a time under the shared root, from here to the end of
    // the test.
    let root_held = SHARED_ROOT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    // The server's `/tmp`, for what A is asked to write: emptied, a run
    // interrupted leaving the service's temporary files behind.
    let tmp = root.join("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    for e in std::fs::read_dir(&tmp).unwrap().flatten() {
        let _ = std::fs::remove_file(e.path());
    }
    let prom = mcr::parse(&std::fs::read(prom).unwrap()).unwrap().imem;
    let mut a = Machine::new();
    a.load_prom(&prom);
    a.disk.attach(0, Unit::open(&pack, Geometry::T300).expect("the release's pack"));
    // The band's own numbers.  At any other pair the machine boots and
    // reaches no server at all: 4403 is off 3050's subnet, so System 304's
    // band, hearing no route to it, never transmits.
    (a.chaos.address, a.chaos.server_address) = release.chaos();
    a.chaos.server_name = release.server_name();
    a.chaos.file_root = Some(root.clone());
    a.chaos.trace = std::env::var_os("MUIR_CHAOS_TRACE").is_some();
    a.chaos.time = Some(muir::chaos::time::TEST_UNIVERSAL);
    a.plug_chaos(0);
    // `Cc::data_frames` counts what has crossed A's cable, and the log is
    // off unless a run asks for it.
    a.ioboard.chaos.as_mut().unwrap().ether_mut().unwrap().keep_log(true);
    let mut b = Machine::new();
    b.load_prom(&prom);
    if debuggee_pack {
        b.disk.attach(0, Unit::open(&pack, Geometry::T300).expect("the release's pack"));
    }
    // B's own Chaosnet, as `muir --debug-in-process` gives it: its own
    // ether with its own server on it, since one cable carries one
    // machine.  The same fixed time as A's, so its band asks the network
    // for the date rather than the screen, and **no file root**: two
    // servers rooted at one directory are two hosts sharing a filesystem,
    // and this run's directory is A's, temporary files and all.
    b.chaos.trace = std::env::var_os("MUIR_CHAOS_TRACE").is_some();
    b.chaos.time = Some(muir::chaos::time::TEST_UNIVERSAL);
    (b.chaos.address, b.chaos.server_address) = release.chaos();
    b.chaos.server_name = release.server_name();
    b.plug_chaos(0);
    let (mut ea, mut eb) = (Rtl::new(a), Rtl::new(b));
    ea.boot();
    eb.boot();
    let l = Lashup::new(ea, eb);
    // The clocks on: a diagnostic prints to the file the FILE service
    // keeps and not to the screen, so a recording of one is minutes of an
    // unchanging frame unless the line below the screens ticks.
    let mut cc = Cc {
        l,
        k: Keyboard::new(),
        root,
        rec: Recorder::pair(true),
        release,
        stall: Cc::STALL,
        next_sample: 0,
        _root_held: root_held,
    };

    // A to its prompt: the listener's `;Reading at top level` line.  Where
    // it lands depends on how tall the herald above it is --- six lines on
    // System 304's band, one fewer on System 100's --- so the rows watched
    // are the band the line falls in for either, as
    // `support::wait_for_the_prompt` watches them.
    while lit_rows(&cc.l, 84..130) < 400 {
        cc.run(500_000);
        assert!(cc.l.steps.0 < 100_000_000, "A never reached its listener");
    }
    cc.run(2_000_000);
    eprintln!("A at its listener after {} microcycles", cc.l.steps.0);
    assert_eq!(cc.l.debugger.debug_cycles(), 0, "nothing on the cable yet");

    cc.type_line("(login 'lispm)");
    // What the login prints, wherever this band's herald has pushed it to:
    // the two land a line apart, so the rows watched cover both.
    cc.until_written(142..170, 0, 60_000_000);
    // A diagnostic's typeout fills the window, and a full window stops at
    // **MORE** and waits for a key that no one will type.  MIT's own file
    // server turns the same switch off on login --- "Allow typeout to
    // continue past the end of screen without hanging",
    // `sys/file/login.lisp` --- and the window scrolls instead.
    cc.type_line("(setq tv:more-processing-global-enable nil)");
    cc.run(2_000_000);
    Some(cc)
}

/// The FILE service's temporary files in `dir`: `#name#`, an interrupted
/// or in-flight write.
fn temp_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('#')))
        .collect()
}

/// Whether `bytes` ends with the `ask` marker, in the machine's newline or
/// ours, trailing whitespace allowed.
fn ends_with_marker(bytes: &[u8]) -> bool {
    let text: String =
        bytes.iter().map(|&b| if b == 0o215 { '\n' } else { (b & 0x7f) as char }).collect();
    text.trim_end().ends_with("*DONE*")
}
