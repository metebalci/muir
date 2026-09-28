// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! System 2000 on QUUX with the video controller: muir-sys's development band on
//! microcode 2000, which sizes its main screen from the feature page.
//!
//! It is in the gitignored `ref/band-2000` (muir-sys's hand-over of its
//! main `11a1a85` for contract H8a, on revision 12; contracts Q8, Q11, Q13
//! and H8a): a GPT disk as a dynamic VHD, which QUUX boots as it is,
//! with microcode 2000 in its current `MCR1`, "MCR1 UCADR 2000", and the
//! band, "LOD4 System 2000", in its current `LOD4`; PROM 2000, the PROM it
//! was built and tested with, which is muir's built-in
//! `data/quux-promh.mcr` byte for byte (`tests/quux_prom.rs`); and the
//! tree it was built from, `release-2000/`. No TV sync program, no speed
//! bits, and no CADR disk controller: QUUX's disk is block-disk. The band
//! takes the screen's size from the feature page at every boot. Its
//! microcode fills the MACRO DISPATCH MEMORY and writes the MACRO-DISPATCH
//! register at `RESET-MACHINE`, at every start, with specialised handlers
//! for some entries. The band before, microcode 2000 with nothing of H8a
//! (muir-sys's Q13 hand-over for revision 11, whose microcode and PROM
//! sources are muir-sys `02c0bb3`'s), is `ref/band-2000-q13`: the tests of
//! a microcode that never writes the register boot it. Without a band the
//! tests skip and say so.

use std::path::PathBuf;

use muir::engine::Engine;
use muir::machine::{Geometry, Machine};
use muir::micro::Micro;
use muir::rtl::Rtl;
use muir::tv::Board;
use support::macro_dispatch::{Checked, Executes, fill_generic, set_register};

mod support;

/// LISPM-1 and OZ, as the release's `site/hosts.text` gives them.
const CHAOS: (u16, u16) = (0o177201, 0o177200);

/// A copy of the disk, which the machine writes, and the served tree, in a
/// scratch directory.
fn band_2000(name: &str) -> Option<(support::Scratch, PathBuf, PathBuf)> {
    band_in(BAND, name)
}

/// The same for the band in `band`, a directory of the tree.
fn band_in(band: &str, name: &str) -> Option<(support::Scratch, PathBuf, PathBuf)> {
    let from = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(band);
    if !from.join(PACK).exists() {
        eprintln!("skipped: {} is not present", from.display());
        return None;
    }
    let dir = support::scratch(name);
    let pack = dir.join("pack.vhd");
    std::fs::copy(from.join(PACK), &pack).unwrap();
    // Booted as it is: a dynamic VHD of a T-300's 263,245 blocks, the
    // hand-over's README says.
    let (format, bytes) = muir::disk_image::probe(&pack).unwrap();
    assert_eq!((format, bytes / 1024), (muir::disk_image::Format::DynamicVhd, 263_245));
    let untar = std::process::Command::new("tar")
        .arg("xzf")
        .arg(from.join(TREE))
        .arg("-C")
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(untar.success(), "the tree unpacks");
    let root = dir.join("root");
    std::fs::create_dir_all(root.join("lispm")).unwrap();
    for part in ["sys", "site"] {
        std::os::unix::fs::symlink(dir.join(RELEASE).join(part), root.join(part)).unwrap();
    }
    Some((dir, pack, root))
}

/// The band, muir-sys's hand-over: its disk, its tree, and the directory
/// the tree unpacks to.
const BAND: &str = "ref/band-2000";
/// Microcode 2000 and System 2000 as they were before contract H8a: the
/// microcode writes none of destinations 5 to 7.
const BAND_Q13: &str = "ref/band-2000-q13";
const PACK: &str = "pack-2000.vhd";
const TREE: &str = "tree-2000.tar.gz";
const RELEASE: &str = "release-2000";

/// The size muir-sys checked `band-2000` at (its hand-over's screens); it
/// takes whatever size the feature page says at boot
/// ([`system_2000_sizes_its_screen_at_boot`]).
const BAND_SIZE: (usize, usize) = (1280, 1024);

fn quux(pack: &std::path::Path, root: &std::path::Path) -> Machine {
    quux_at(pack, root, BAND_SIZE)
}

/// QUUX with its own boot PROM at 36000 (`data/quux-promh.mcr`), the disk
/// on block-disk, the video controller at `w` by `h`, and the file device serving
/// `root` as HOST's `/` and the tree's `sys` and `site` as `/sys` and
/// `/site`, where the band's `SYS:` is (`site/sys.translations`).
fn quux_at(pack: &std::path::Path, root: &std::path::Path, (w, h): (usize, usize)) -> Machine {
    use muir::block_disk::{BLOCK_NS, BlockDisk};
    let mut m = Machine::new();
    m.load_prom(&muir::prom::quux_boot_prom());
    let mut d = BlockDisk::new(BLOCK_NS);
    d.attach(muir::disk_image::Disk::open_rw(pack).expect("the pack"));
    m.block_disk = Some(d);
    m.geometry = Geometry::QUUX;
    m.tv.set_board(Board::Video);
    m.tv.set_video_size(w, h);
    serve(&mut m, root);
    m
}

/// The file device serving `root` as HOST's `/`, and its `sys` and `site`
/// by name.
fn serve(m: &mut Machine, root: &std::path::Path) {
    m.file_device.mounts.add(&root.display().to_string()).unwrap();
    for part in ["sys", "site"] {
        m.file_device.mounts.add(&format!("{part}={}", root.join(part).display())).unwrap();
    }
}

/// Whether the listener is framed at the screen's own size, and not at any
/// other width a band might have drawn at: its border lights the first and
/// the last pixel of every row through the middle half of the screen, read
/// at the screen's words a line, and read at any other it does not.
fn drawn_at_its_words_a_line(e: &impl Engine) -> bool {
    let (_, h, own) = e.machine().tv.screen();
    framed(e, own, h)
        && [24, 40, 60, 80].into_iter().filter(|&w| w != own).all(|w| !framed(e, w, h))
}

/// Whether, read `words_per_line` words a line, the first and the last
/// pixel of every row in the middle half of `h` rows are lit.
fn framed(e: &impl Engine, words_per_line: usize, h: usize) -> bool {
    let buf = e.machine().tv.buffer();
    let lit = |bit: usize| buf.get(bit / 32).is_some_and(|w| w >> (bit % 32) & 1 != 0);
    let width = words_per_line * 32;
    (h / 4..3 * h / 4).all(|y| lit(y * width) && lit(y * width + width - 1))
}

/// The screen as a GIF in the temporary directory, for a failure message.
fn shot(e: &impl Engine, name: &str) -> String {
    let path = std::env::temp_dir().join(format!("muir-system-2000-{name}.gif"));
    let mut rec = muir::capture::Recorder::new(false);
    rec.sample(&e.machine().tv, 0, 0);
    std::fs::write(&path, rec.gif()).unwrap();
    path.display().to_string()
}

/// `%MICROCODE-VERSION-NUMBER`, A memory's word 40 (`mcr::Mcr::version`
/// has where that is from), as the running machine holds it.
fn microcode_version(e: &impl Engine) -> u32 {
    e.machine().amem[0o40] & 0o77777777
}

/// **The band is System 2000 on microcode 2000**, as the disk says: its
/// current `MCR1` is named "MCR1 UCADR 2000" and holds the hand-over's
/// `ucadr.mcr`, whose `A-VERSION` is 2000, and its current `LOD4` is named
/// "LOD4 System 2000". No boot; the running band's own word is
/// `tests/system_2000_timers.rs`'s M10.
#[test]
fn band_2000_is_system_2000_on_microcode_2000() {
    let from = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(BAND);
    if !from.join(PACK).exists() {
        eprintln!("skipped: {} is not present", from.display());
        return;
    }
    let bytes = std::fs::read(from.join("ucadr.mcr")).unwrap();
    let mcr = muir::mcr::parse_partition_order(&bytes).unwrap();
    assert_eq!(mcr.version(), Some(2000), "ucadr.mcr's A-VERSION");
    let mut d = muir::disk_image::Disk::open(from.join(PACK)).unwrap();
    let parts = support::gpt_partitions(&mut d);
    let current = |lisp: &str| {
        parts
            .iter()
            .find(|p| p.current && p.name.starts_with(lisp))
            .unwrap_or_else(|| panic!("no current {lisp}: {parts:?}"))
    };
    assert_eq!(current("MCR").name, "MCR1 UCADR 2000");
    assert_eq!(current("LOD").name, "LOD4 System 2000");
    let mcr1 = current("MCR");
    for (k, block) in bytes.chunks(1024).enumerate() {
        let on_disk: Vec<u8> = d
            .read_block(mcr1.first + k as u32)
            .unwrap()
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        assert!(on_disk == block, "MCR1's block {k} is the hand-over's ucadr.mcr");
    }
}

/// **System 2000 reaches its listener on QUUX with the video controller, drawn at the
/// screen's words a line**, on both engines, at the size muir-sys checked the
/// band at ([`BAND_SIZE`]), with microcode 2000 in A memory: its listener is
/// framed at the video controller's words a line and at no other width, the CADR's 24
/// among them, which is the band drawing for the screen it was given.
#[test]
fn system_2000_runs_on_the_video_controller() {
    for engine in ["micro", "rtl"] {
        let Some((_dir, pack, root)) = band_2000(&format!("system-2000-{engine}")) else {
            return;
        };
        let m = quux(&pack, &root);
        let (ran, drawn, version) = match engine {
            "micro" => {
                let mut e = Micro::new(m);
                e.boot();
                let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
                (ran, drawn_at_its_words_a_line(&e), microcode_version(&e))
            }
            _ => {
                let mut e = Rtl::new(m);
                e.boot();
                let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
                (ran, drawn_at_its_words_a_line(&e), microcode_version(&e))
            }
        };
        eprintln!("{engine}: listener after {ran} microcycles, microcode {version}");
        assert!(drawn, "{engine}: drawn at the screen's words a line");
        assert_eq!(version, 2000, "{engine}: the microcode's version");
    }
}

/// **System 2000 sizes its screen at boot**: the same band, checked at
/// [`BAND_SIZE`], booted at other sizes, draws its listener at each size's
/// own words a line. 1920 by 1080 is the largest video controller screen QUUX supports.
#[test]
fn system_2000_sizes_its_screen_at_boot() {
    for size in [(1024, 768), (1920, 1080)] {
        let Some((_dir, pack, root)) = band_2000(&format!("system-2000-{}x{}", size.0, size.1))
        else {
            return;
        };
        let mut e = Micro::new(quux_at(&pack, &root, size));
        e.boot();
        let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
        eprintln!("{size:?}: listener after {ran} microcycles");
        assert_eq!(e.machine().tv.screen().2, size.0 / 32, "{size:?}: the screen's words a line");
        assert!(
            drawn_at_its_words_a_line(&e),
            "{size:?}: drawn at the screen's words a line; the screen is {}",
            shot(&e, &format!("{}x{}", size.0, size.1))
        );
    }
}

/// **System 2000 runs at its ticks**: QUUX drops the delay lines, and its
/// microcycle is `sync`'s K ticks of 10 ns. The same microcode and band
/// reach the same listener at four ticks and at three, and the time to it
/// is shorter at three by less than the microcycles' ratio, the bus keeping
/// its own time.
#[test]
fn system_2000_runs_at_its_ticks() {
    use muir::clock::TimingModel;
    let mut times = Vec::new();
    for ticks in [4, 3] {
        let model = TimingModel::Sync { cycle_ticks: ticks, ilong_ticks: 0 };
        let Some((_dir, pack, root)) = band_2000(&format!("system-2000-sync-{ticks}")) else {
            return;
        };
        let mut e = Rtl::new(quux(&pack, &root));
        e.set_timing_model(model);
        e.boot();
        let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
        assert!(
            drawn_at_its_words_a_line(&e),
            "{ticks} ticks: at the screen's words a line; the screen is {}",
            shot(&e, &format!("sync-{ticks}"))
        );
        eprintln!("{ticks} ticks: listener after {ran} microcycles, {} ns", e.ns());
        times.push(e.ns());
    }
    let ratio = times[0] as f64 / times[1] as f64;
    assert!(ratio > 1.0 && ratio < 4.0 / 3.0, "{ratio:.3} times faster at three ticks");
}

/// `DISK-AWAIT-READY`, where microcode 2000 waits for block-disk to be
/// ready, from the band's `ucadr.sym` ([`band_symbol`]): it moves as the
/// microcode before it grows.
const DISK_AWAIT_READY: &str = "DISK-AWAIT-READY";

/// Block-disk's registers as the microcode addresses them, virtual
/// (`DISK-REGS-ADDRESS-BASE`, `77777600` since contract Q13), and where
/// they are, physical: word 200 of the register page, `17777600`.
const DISK_REGS: (u32, u32) = (0o77777600, 0o17777600);

/// **System 2000 restores its own band and comes back to the listener**:
/// booted at 1280 by 1024, `(si:disk-restore 4)` answered `yes` reads LOD4
/// back in and boots it to the listener again, on `micro`. The microcode's
/// cold boot maps the disk registers and the run light with
/// `COLD-FAKE-L2-MAP`, and when the two took the same level-2 slot the
/// disk registers' virtual address reached the run light instead: the
/// restore sat in `DISK-AWAIT-READY` for ever, block-disk's disk address
/// never moving on (found by muir, fixed in muir-sys's microcode before
/// dev11). So while the band is read, every microcycle at
/// `DISK-AWAIT-READY` holds the disk registers' virtual address to their
/// physical one, and every million microcycles block-disk has to have moved
/// on. The restore takes about 26 million microcycles to read the band and
/// 165 million to the listener (measured). The test catches the collision:
/// on the band before the fix, dev9 (QUUX's microcode for Q5, then
/// numbered 1000, with the PROM it booted on, which read MIT's label), the
/// disk registers' virtual
/// address reaches 17117774 at `DISK-AWAIT-READY`; with that check taken
/// out, block-disk stands still within 2 million microcycles of the answer
/// (measured).
#[test]
fn system_2000_restores_its_band_to_the_listener() {
    use muir::terminal::keyboard::Keyboard;
    let Some((_dir, pack, root)) = band_2000("system-2000-restore") else {
        return;
    };
    let await_ready = band_symbol(DISK_AWAIT_READY, "I-MEM");
    let lod4 = support::gpt_partition(&mut muir::disk_image::Disk::open(&pack).unwrap(), "LOD4");
    let mut m = quux(&pack, &root);
    m.block_disk.as_mut().unwrap().log = Some(Vec::new());
    let mut e = Micro::new(m);
    e.boot();
    let ran = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
    eprintln!("listener after {ran} microcycles");
    let mut k = Keyboard::new();
    support::type_at(&mut e, &mut k, "(si:disk-restore 4)");
    // Time for the question, whether to reload LOD4, before its answer.
    for _ in 0..20_000_000 {
        e.step().unwrap();
    }
    support::type_at(&mut e, &mut k, "yes\n");
    let transfers =
        |e: &Micro| e.machine().block_disk.as_ref().unwrap().log.as_ref().unwrap().len();
    let from = transfers(&e);
    // The band read, until the screen goes dark for the boot.
    let (mut n, mut waited) = (0u64, 0u64);
    while support::lit_rows(&e, 84..130) > 400 {
        let (before, mut waiting) = (transfers(&e), 0);
        for _ in 0..1_000_000 {
            e.step().unwrap();
            if e.pc() == await_ready {
                waiting += 1;
                let at = e.machine().translate(DISK_REGS.0).physical;
                assert_eq!(
                    at, DISK_REGS.1,
                    "at DISK-AWAIT-READY the disk registers' {:o} reach {at:o}",
                    DISK_REGS.0
                );
            }
        }
        n += 1_000_000;
        waited += waiting;
        assert!(
            transfers(&e) > before,
            "block-disk still after {n} microcycles, {waiting} of the last million at \
             DISK-AWAIT-READY; the screen is {}",
            shot(&e, "restore")
        );
        assert!(n < 100_000_000, "the screen never went dark for the boot");
    }
    let log = &e.machine().block_disk.as_ref().unwrap().log.as_ref().unwrap()[from..];
    let band_reads = log
        .iter()
        .filter(|t| !t.write && (lod4.first..lod4.first + lod4.blocks).contains(&t.block))
        .count();
    eprintln!(
        "band read, {band_reads} blocks of LOD4, after {n} microcycles, {waited} at \
         DISK-AWAIT-READY ({await_ready:o})"
    );
    assert!(waited > 0, "DISK-AWAIT-READY, {await_ready:o}, never ran: the check held nothing");
    assert!(band_reads > 0, "LOD4 read");
    let fused = e.machine().macro_dispatch.fused;
    let again = support::wait_for_the_prompt_within(&mut e, 400_000_000);
    eprintln!("listener again after {} microcycles more", again);
    assert!(
        drawn_at_its_words_a_line(&e),
        "the listener again, at the screen's words a line; the screen is {}",
        shot(&e, "restored")
    );
    // The restore writes the control store, which clears the enable, and
    // the restored microcode's `RESET-MACHINE` writes the register again.
    let d = &e.machine().macro_dispatch;
    assert_eq!(d.register >> 31, 1, "the register enabled again after the restore");
    assert!(d.fused > fused, "returns fused again after the restore");
}

/// The control store or dispatch memory address of a symbol of the band's
/// microcode, from its `ucadr.sym`: `QMLP I-MEM 124`.
fn band_symbol(name: &str, space: &str) -> u16 {
    band_symbol_in(BAND, name, space)
}

/// The same for the band in `band`.
fn band_symbol_in(band: &str, name: &str, space: &str) -> u16 {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(band).join("ucadr.sym");
    let text = std::fs::read_to_string(&path).unwrap();
    text.lines()
        .find_map(|l| {
            let mut w = l.split_whitespace();
            (w.next() == Some(name) && w.next() == Some(space))
                .then(|| u16::from_str_radix(w.next().unwrap(), 8).unwrap())
        })
        .unwrap_or_else(|| panic!("{}: no {name} {space}", path.display()))
}

/// The state a boot leaves that the fused return could change: the
/// memories, the stacks and their pointers, the location counter, the
/// microcycles and the time, and where the machine is.
fn machine_state<E: Engine>(e: &E) -> impl PartialEq + std::fmt::Debug + use<E> {
    let m = e.machine();
    let digest = |w: &[u32]| {
        w.iter().fold(0xcbf29ce484222325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x100000001b3))
    };
    let imem: Vec<u64> = m.imem.iter().map(|i| i.raw()).collect();
    (
        (m.cycles, m.ns, e.pc(), m.lc, m.spcptr, m.pdl_pointer, m.pdl_index),
        (m.amem.to_vec(), m.mmem, m.spc, m.q, m.vma, m.md),
        (digest(&m.dmem), digest(&m.pdl), digest(&m.main), digest(m.tv.buffer())),
        imem.iter().fold(0u64, |h, &x| (h ^ x).wrapping_mul(0x100000001b3)),
    )
}

/// Boots microcode 2000 as it was before contract H8a (`BAND_Q13`) on
/// revision 12 and on revision 11 (`Geometry::QUUX_11`) with `make`, to
/// the listener, and holds the two to each other.
fn revision_12_runs_as_revision_11<E: Engine>(engine: &str, make: impl Fn(Machine) -> E) {
    let mut ran = Vec::new();
    let mut states = Vec::new();
    for geometry in [Geometry::QUUX_11, Geometry::QUUX] {
        let revision = geometry.machine_id.unwrap() >> 4 & 0o7777;
        let Some((_dir, pack, root)) =
            band_in(BAND_Q13, &format!("system-2000-rev-{engine}-{revision}"))
        else {
            return;
        };
        let mut m = quux(&pack, &root);
        m.geometry = geometry;
        // The real-time clock counted from the harness's time, 1 September
        // 2026, as the time server answers it: the band sets its clock from
        // it at boot, and the host's would differ between the two runs.
        m.rtc = muir::machine::Rtc::Counted {
            start: support::time::TEST_UNIVERSAL - support::time::UNIX_EPOCH_UNIVERSAL as u32,
            base_ns: 0,
        };
        let mut e = make(m);
        e.boot();
        let n = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
        assert!(drawn_at_its_words_a_line(&e), "{engine}, revision {revision}: the listener");
        assert_eq!(e.machine().macro_dispatch.register, 0, "{engine}: never written");
        assert_eq!(e.machine().macro_dispatch.fused, 0, "{engine}: nothing fused");
        eprintln!(
            "{engine}, revision {revision}: listener after {n} steps, {} microcycles, {} ns",
            e.machine().cycles,
            e.machine().ns
        );
        ran.push(n);
        states.push(machine_state(&e));
    }
    assert_eq!(ran[1], ran[0], "{engine}: the listener at the same step");
    assert!(states[1] == states[0], "{engine}: revision 12 leaves revision 11's state");
}

/// **Revision 12 runs a microcode that never writes the register as
/// revision 11 does** (contract H8a §6 item 1), on `rtl`: microcode 2000
/// as it was before H8a (`BAND_Q13`) writes none of destinations 5 to 7,
/// so the MACRO-DISPATCH register stays as -RESET leaves it, disabled, and
/// the band reaches its listener in the same microcycles and nanoseconds
/// with the same memories, stacks and screen. The MACHINE-ID differs, and
/// microcode 2000 asks only for 11 or more (`uc-cold-disk.lisp:16-23`).
#[test]
fn revision_12_runs_microcode_2000_as_revision_11_does_on_rtl() {
    revision_12_runs_as_revision_11("rtl", Rtl::new);
}

/// The same on `micro`.
#[test]
fn revision_12_runs_microcode_2000_as_revision_11_does_on_micro() {
    revision_12_runs_as_revision_11("micro", Micro::new);
}

/// Steps `e` until the microcode's main loop, `QMLP`, has run once: the
/// microcode is loaded, and no main-loop return has been made. Not while
/// the PROM loads the microcode: on `rtl` a control-store write's second
/// microcycle stands at the address written, `QMLP`'s among them, and the
/// PC goes back into the PROM, at 36000 up, after it. With `enabled`, also
/// not before the MACRO-DISPATCH register is enabled, which the band's
/// microcode does at `RESET-MACHINE`, before its first main-loop return.
fn to_the_main_loop(e: &mut impl Engine, qmlp: u16, enabled: bool) {
    use muir::machine::macro_dispatch::ENABLE;
    for _ in 0..100_000_000 {
        if e.machine().opc == qmlp
            && e.pc() < muir::machine::QUUX_PROM_BASE
            && (!enabled || e.machine().macro_dispatch.register & ENABLE != 0)
        {
            return;
        }
        e.step().unwrap();
    }
    panic!("QMLP never ran");
}

/// Who fills the MACRO DISPATCH MEMORY for [`boots_with_the_fused_return`].
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fill {
    /// The band's microcode, at `RESET-MACHINE`: the generic handlers and
    /// its specialised ones.
    Microcode,
    /// The test, once the microcode has filled it: every entry the generic
    /// handler, `OPDTB`'s, with the operand bit on every halfword whose
    /// `<8:0>` is a register and a delta (`fill_generic`).
    Generic,
}

/// The entries whose handler, `<13:0>`, is not `OPDTB`'s for their
/// opcode: the specialised ones.
fn specialised_entries(m: &Machine, opdtb: u16) -> usize {
    m.macro_dispatch
        .entries
        .iter()
        .enumerate()
        .filter(|&(k, &x)| x & 0o37777 != m.dmem[opdtb as usize + (k >> 3 & 0o37)] & 0o37777)
        .count()
}

/// Boots the band on `make`'s engine with the MACRO DISPATCH MEMORY as
/// `fill` says, under the checkers of `support::macro_dispatch`, from the
/// first main-loop return with the register enabled; and says how many
/// returns fused.
fn boots_with_the_fused_return<E: Executes>(engine: &str, make: impl Fn(Machine) -> E, fill: Fill) {
    let Some((_dir, pack, root)) = band_2000(&format!("system-2000-fused-{engine}-{fill:?}"))
    else {
        return;
    };
    // The symbols after the band: without it the test skips.
    let (qmlp, opdtb) = (band_symbol("QMLP", "I-MEM"), band_symbol("OPDTB", "D-MEM"));
    let (localp, ap) = (band_symbol("A-LOCALP", "A-MEM"), band_symbol("M-AP", "M-MEM"));
    let mut e = make(quux(&pack, &root));
    e.boot();
    to_the_main_loop(&mut e, qmlp, true);
    let word = muir::machine::macro_dispatch::word(qmlp, localp, ap as u8);
    assert_eq!(e.machine().macro_dispatch.register, word, "{engine}: the microcode's register");
    let specialised = specialised_entries(e.machine(), opdtb);
    if fill == Fill::Generic {
        fill_generic(e.machine_mut(), qmlp, opdtb, localp, ap as u8, true);
    }
    let mut e = Checked::new(e);
    let n = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
    let m = e.machine();
    let c = &e.checker.counts;
    eprintln!(
        "{engine}, {fill:?}: listener after {n} steps, {} fused returns; the microcode's fill \
         had {specialised} specialised entries",
        m.macro_dispatch.fused
    );
    eprintln!("{engine}, {fill:?}: {}", c.report(|pc| format!("{pc:o}")));
    assert!(drawn_at_its_words_a_line(&e), "{engine}, {fill:?}: the listener, fused");
    assert!(m.macro_dispatch.fused > 0, "{engine}, {fill:?}: returns fused");
    assert_eq!(c.fused, m.macro_dispatch.fused, "{engine}, {fill:?}: every one checked");
    assert!(c.operand_loads > 0, "{engine}, {fill:?}: operand addresses loaded");
    assert_eq!(c.problems(), 0, "{engine}, {fill:?}: the rule of §3.3 kept and every check met");
    assert_eq!(m.macro_dispatch.register, word, "{engine}, {fill:?}: still enabled");
    let now = specialised_entries(m, opdtb);
    match fill {
        Fill::Microcode => {
            assert!(specialised > 0, "{engine}: the microcode's specialised entries");
            assert_eq!(now, specialised, "{engine}: the entries as the microcode left them");
        }
        Fill::Generic => assert_eq!(now, 0, "{engine}: the generic entries kept"),
    }
    assert_eq!(microcode_version(&e), 2000);
}

/// **System 2000 boots with the fused return** (contract H8a §6 items 4-6),
/// on `rtl`: its microcode fills the MACRO DISPATCH MEMORY, with its
/// specialised handlers, and enables the register with its `QMLP`,
/// `A-LOCALP` and `M-AP` before its first main-loop return, and it reaches
/// the listener with returns fused and the register still enabled; the
/// microcycle after every fused return keeps the rule of §3.3, the
/// handler's first microinstruction reads no PDL word that microcycle
/// writes, the handler the main loop would have reached runs next, the
/// operand address is right wherever the entry has the operand bit, the
/// base copies equal `A-LOCALP` and `M-AP` after every microcycle, and
/// M 31 is main memory's word after every return fused on the prefetched
/// word.
#[test]
fn system_2000_boots_with_the_fused_return_on_rtl() {
    boots_with_the_fused_return("rtl", Rtl::new, Fill::Microcode);
}

/// The same on `micro`.
#[test]
fn system_2000_boots_with_the_fused_return_on_micro() {
    boots_with_the_fused_return("micro", Micro::new, Fill::Microcode);
}

/// **System 2000 boots with the generic handlers and the operand bit on
/// every LOCAL and ARG entry** (contract H8a §6 item 5), on `rtl`: the
/// test fills every entry with `OPDTB`'s, the operand bit on every halfword
/// whose `<8:0>` is a register and a delta, once the microcode has
/// enabled the register, and the checkers find nothing, the stores to a
/// local or an argument having left the microcycle after a main-loop
/// return.
#[test]
fn system_2000_boots_with_the_generic_operand_fill_on_rtl() {
    boots_with_the_fused_return("rtl", Rtl::new, Fill::Generic);
}

/// The same on `micro`.
#[test]
fn system_2000_boots_with_the_generic_operand_fill_on_micro() {
    boots_with_the_fused_return("micro", Micro::new, Fill::Generic);
}

/// Boots the band in `band` on `micro` with the register enabled for its
/// own main loop and every entry poisoned to `ILLOP`, left over from before
/// the boot, to the listener; returns the engine and `ILLOP`'s address.
fn boots_from_a_stale_memory(band: &str, name: &str) -> Option<(Micro, u16)> {
    let (_dir, pack, root) = band_in(band, name)?;
    // The symbols after the band: without it the test skips.
    let (qmlp, illop) =
        (band_symbol_in(band, "QMLP", "I-MEM"), band_symbol_in(band, "ILLOP", "I-MEM"));
    let mut e = Micro::new(quux(&pack, &root));
    e.boot();
    // After -BOOT's reset, which clears the enable too.
    e.machine_mut().macro_dispatch.entries.fill(illop as u32);
    set_register(e.machine_mut(), muir::machine::macro_dispatch::word(qmlp, 0, 0));
    let n = support::boot_to_the_prompt_within(&mut e, CHAOS, root, 400_000_000);
    eprintln!("{band}: listener after {n} steps");
    assert!(drawn_at_its_words_a_line(&e), "{band}: the listener");
    Some((e, illop))
}

/// **A stale MACRO DISPATCH MEMORY never runs** (contract H8a §6 item 3):
/// with the register enabled for the main loop of microcode 2000 as it was
/// before H8a (`BAND_Q13`), which never writes it, and every entry
/// poisoned to `ILLOP`, left over from before the boot, the PROM's load of
/// the microcode --- control-store writes --- clears the enable, and the
/// band reaches its listener with nothing fused. Left enabled, the first
/// main-loop return would go to `ILLOP`.
#[test]
fn a_stale_macro_dispatch_memory_never_runs() {
    let Some((e, illop)) = boots_from_a_stale_memory(BAND_Q13, "system-2000-stale") else {
        return;
    };
    let d = &e.machine().macro_dispatch;
    assert_eq!(d.fused, 0, "nothing fused");
    assert_eq!(d.register >> 31, 0, "the enable cleared");
    assert!(d.entries.iter().all(|&x| x == illop as u32), "the entries kept");
}

/// **The band's microcode fills a stale MACRO DISPATCH MEMORY again**: the
/// same start with the band's own microcode, which writes every entry and
/// the register at `RESET-MACHINE`, reaches its listener with returns
/// fused, no entry left poisoned and the register its own.
#[test]
fn a_stale_macro_dispatch_memory_is_filled_again() {
    let Some((e, illop)) = boots_from_a_stale_memory(BAND, "system-2000-stale-filled") else {
        return;
    };
    let (qmlp, localp, ap) = (
        band_symbol("QMLP", "I-MEM"),
        band_symbol("A-LOCALP", "A-MEM"),
        band_symbol("M-AP", "M-MEM"),
    );
    let d = &e.machine().macro_dispatch;
    assert!(d.fused > 0, "returns fused");
    assert_eq!(d.register, muir::machine::macro_dispatch::word(qmlp, localp, ap as u8));
    let poisoned = d.entries.iter().filter(|&&x| x == illop as u32).count();
    assert_eq!(poisoned, 0, "entries left poisoned");
}
