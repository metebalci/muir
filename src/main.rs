// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! muir: the simulator. One engine at a time from the boot PROM, a pack on
//! the disk controller's cable, and microcycles per second against the
//! machine's own rate.
//!
//! The reference is the machine's own microcycle, read off the delay-line
//! taps: 145 ns at normal speed, so 6.9 M microcycles/s.
//!
//!     muir [--micro|--rtl|--chip] [--chaos-address <this>[,<server>]]
//!          [--chaos-file-root <dir>] [--checkpoint <file>]
//!          [--debug-cable-connect [<endpoint>]]
//!          [--debug-cable-listen [<endpoint>]] [--debug-in-process]
//!          [--debuggee-disk-pack <image>[,<unit>][,ro]]
//!          [--debuggee-terminal [<endpoint>]]
//!          [--disk-controller netlist|model]
//!          [--disk-pack <image>[,<unit>][,ro]] [--io-board netlist|model]
//!          [--main-memory netlist|model] [--main-memory-boards <n>]
//!          [--prom <file>] [--resume <file>] [--stop-after <microcycles>]
//!          [--stop-at <pc>] [--stop-at-prom <pc>] [--terminal [<endpoint>]]
//!          [--tv netlist|model] [--tv-board simple-tv|lispm-tv]
//!          [--tv-capture <gif>] [--tv-capture-no-time]
//!
//! `cargo build --release` leaves it at `target/release/muir`, and `cargo
//! install --path .` puts it on the path. The programs in `examples/` stay
//! `cargo run --example` programs.
//!
//! The engine flags are mutually exclusive and `--rtl` is the default.
//! `chip` runs main memory and the I/O board as netlists unless told
//! `model`, which is `rtl`'s twin of each --- the same timing, no gates ---
//! and `--main-memory-boards` is how many 64K-word boards the machine has
//! --- main memory on every engine, the boards on the Xbus on `chip` ---
//! 32 by default for the two million words. `--tv` is the
//! display, a netlist on the backplane unless told `model`, and
//! `--tv-board` which display: the SIMPLE TV, the black-and-white board
//! System 100 drives, or the LISPM TV that replaced it. `--disk-controller`
//! is the disk controller, whose netlist runs its own microcode with the
//! pack on its cable as a drive and takes the drive's time over every
//! block, milliseconds where the model takes none; `model` is its default,
//! alone among the boards, until a boot through it has been run to the
//! end. The other engines run the models always.
//! `--disk-pack` takes a pack image --- the System 100 release's
//! `disk-sys-100-0.img` --- and without it the vendored copy is used when it
//! is there. The image is the pack's blocks end to end, 256 words of 32 bits
//! each, in the geometry's order. It is opened read-write and written as a
//! drive writes its pack. After the image, in either order, come the
//! drive's unit --- only 0 until the disk multiplexor is modelled --- and
//! `ro`, the drive's read-only switch, `STATUS<7>`, with the file opened
//! read-only behind it, and a write then faults as MIT says it does. With
//! no pack at all the engines run the same PROM waiting on a drive that
//! never answers, which measures the wait loop.
//!
//! Every run serves a terminal: the display, the keyboard and the mouse
//! over RFB, RFC 6143, so that any VNC viewer can work the machine, which
//! has no other way in or out. It is at VNC's display :0 on the loopback,
//! `vnc://127.0.0.1:5900`, and the start says where it is; a display
//! already taken --- a second muir on the host, which is what the lashup
//! over TCP is --- moves it up to the first free one. `--terminal` says
//! where instead: a port, and an address before it to listen anywhere but
//! the loopback --- `--terminal 5900` for this machine only, `--terminal
//! 0.0.0.0:5900` to let another one in, which is worth meaning, because
//! RFB's `None` security is the only type offered and a viewer needs no
//! password. A port that is named is bound as it stands and the run stops
//! if it cannot be, rather than serving a viewer somewhere it was not told
//! to look. What it shows is the frame buffer, which
//! is the screen on every engine; the monitor on the netlist board's video
//! cable is a separate thing and not built. It keeps serving the last
//! screen for as long as a viewer is looking at it once the run has
//! stopped. In the lashup the other machine is served a terminal too, the
//! display above this machine's, and `--debuggee-terminal` puts that
//! elsewhere. On an engine with no Chaosnet the boot stops in the debugger
//! at the initialization that wants a host: `Super-B` there, then the date
//! and time it asks for and `y`, finish it.
//!
//! Separately, and on every engine: the band's cold boot leaves the
//! display's vertical interrupt off, so the mouse is not tracked until
//! `(si:setup-cpt)` is typed at the listener. That is the band's own ---
//! `LISP-REINITIALIZE` guards its `SETUP-CPT` block with `(UNLESS (NOT
//! CALLED-BY-USER) ...)`, which the cold boot's `(LISP-REINITIALIZE NIL)`
//! does not satisfy --- so it is wanted just as much on a boot that
//! reached the listener with no trouble. `tests/vertical.rs` holds that:
//! it boots with a Chaosnet, gets to the prompt, and finds the interrupt
//! still off.
//!
//! The prompt is muir's own line on stdin while a machine runs on its
//! own: `boot`, `hold`, `continue`, `step`, `pc`, `reg`, `amem`, `mmem`,
//! `dmem`, `pdl`, `spc`, `screenshot`, `startcapture`, `endcapture`,
//! `info`, `checkpoint`, `quit` and `help`,
//! [`muir::prompt`], read from a pipe or from a terminal muir is in the
//! foreground of, and acted on between two microcycles. `muir: ` is
//! written while the machine is held, to a terminal and not to a pipe;
//! a line typed while it runs is acted on all the same. ^C holds the
//! machine at the prompt; ^C while held, or with no prompt to go on
//! from, ends the run as `quit` does. `--no-auto-boot` leaves the boot
//! button unpressed, as a CADR is when the power comes on, and starts the
//! run held for the prompt's `boot` to press it --- which starts the
//! machine, since the button is all that does; a hold nothing can run on
//! --- stdin having ended --- ends the run rather than standing there.
//!
//! A machine that stops itself is held at the prompt and says so, rather
//! than being run on through: `HALT-CONS` under `ERRSTOP` --- what System
//! 100's `(si:%halt)` runs --- and the statistics counter under `STATHENB`
//! both drop `MACHRUN` with `RUN` still set, and no microcycle runs from
//! there. Nothing about stepping says so, the screen simply stops, so the
//! run loop reads it off `FLAG-1` where a console would. `boot` presses
//! the button that starts it again. `chip` holds at the prompt the same
//! way, reading the nets the spy registers are buffered from, since it is
//! not an `Engine` and has no registers to read.
//!
//! A run goes on until a stop, a halt or ^C. `--stop-after` ends it after
//! that many microcycles, `--stop-at` when the PC reaches an address with
//! the boot PROM disabled, `--stop-at-prom` with it enabled --- the PROM and
//! the control store share their low addresses, so a PC alone names two
//! places. Addresses are octal, as MIT writes them, and whichever stop
//! comes first wins. The rate reported at the end is over the whole run,
//! and past the boot the work is cheaper, so a longer run reports a higher
//! one.
//!
//! The boot PROM is MIT's own `mit/sys/ubin/promh.mcr`, so nothing here
//! needs `vendor/`. `--prom` runs another one instead, out of an MCR
//! microcode file as MIT's own is; the start says how the file stands to
//! MIT's own, because recovered copies of the boot PROM are not all the
//! same program. A checkpoint carries the 512 words it ran, so `--resume`
//! brings its own and the two flags are refused together.

use std::io::IsTerminal;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use muir::cable::{Boards, DebugIn, FarEnd};
use muir::capture::{Recorder, local_time, wall_clock};
use muir::checkpoint::Checkpoint;
use muir::chip::Chip;
use muir::clock::{Behavioural, Clock};
use muir::disk_unit::{Geometry, Unit};
use muir::engine::Engine;
use muir::isa::Insn;
use muir::lashup::{Lashup, Remote};
use muir::machine::Machine;
use muir::micro::Micro;
use muir::netlist;
use muir::part::Level;
use muir::prompt::{Command, Memory};
use muir::rtl::Rtl;
use muir::terminal::keyboard::Keyboard;
use muir::terminal::mouse::Mouse;
use muir::terminal::{Frame, Terminal};

/// 145 ns per microcycle at normal speed: `Speed::cycle_ns`, and what
/// `tests/clock.rs` pins.
const HARDWARE_CYCLES_PER_S: f64 = 1e9 / 145.0;

/// How often the terminal is given a turn: about thirty times a second.
/// The machine's own raster is 64.7 Hz, so a viewer sees every other frame
/// at best, and the cost does not show in any engine's rate.
const TERMINAL_INTERVAL: Duration = Duration::from_millis(33);

/// Microcycles between glances at the computer's clock to see whether
/// [`TERMINAL_INTERVAL`] has gone by. `Instant::now` is not free and
/// `micro` runs 66 M microcycles a second.
const TERMINAL_CHECK: u64 = 4_096;

/// The port a terminal is served at unless `--terminal` says another:
/// VNC's display :0, RFB's convention.
const TERMINAL_PORT: u16 = 5900;

/// How many displays up from where a terminal was asked for a free one is
/// looked for when the port was not named: VNC's :0 to :99, which is 5900
/// to 5999 from the default port.
const TERMINAL_DISPLAYS: u16 = 100;

/// Where a terminal is served, and how hard: the endpoint; whether the
/// port in it was named, since a named port is bound as it stands and an
/// unnamed one is only where the search for a free display starts; and
/// whether the flag was given at all, since a terminal that was asked for
/// and cannot be served stops the run, and one nobody asked for leaves the
/// run without a terminal and says so.
#[derive(Clone, Copy)]
struct TerminalAt {
    addr: SocketAddr,
    port_named: bool,
    asked: bool,
}

impl TerminalAt {
    /// The terminal nobody asked for: VNC's display :0 on the loopback,
    /// which is where an unauthenticated server belongs unless someone
    /// says otherwise in as many words.
    fn default_display() -> TerminalAt {
        TerminalAt {
            addr: SocketAddr::from((Ipv4Addr::LOCALHOST, TERMINAL_PORT)),
            port_named: false,
            asked: false,
        }
    }
}

/// The terminal `at` says: bound where it says when the port was named,
/// and otherwise at the first free display from there up, over
/// [`TERMINAL_DISPLAYS`] of them --- a second muir on one host, which is
/// what the lashup over TCP is, then has a display of its own rather than
/// the first one's port. `Err` is why there is no terminal, which the
/// caller makes fatal or not.
fn bind_terminal(at: TerminalAt) -> Result<Terminal, String> {
    let last = at.addr.port().saturating_add(TERMINAL_DISPLAYS - 1);
    let mut addr = at.addr;
    loop {
        match Terminal::bind(addr) {
            Ok(mut t) => {
                t.trace = true;
                return Ok(t);
            }
            Err(e) if at.port_named => return Err(format!("{addr}: {e}")),
            Err(e) if e.kind() != std::io::ErrorKind::AddrInUse => {
                return Err(format!("{addr}: {e}"));
            }
            Err(_) if addr.port() < last => addr.set_port(addr.port() + 1),
            Err(e) => {
                return Err(format!(
                    "{}: no free display in {}-{}: {e}",
                    addr.ip(),
                    at.addr.port(),
                    last
                ));
            }
        }
    }
}

/// The port the debug cable meets at: 7661 for DBGOUT's Unibus address
/// 766100. IANA leaves 7649-7662 unassigned (its registry, read 6 Sep 2026)
/// and it is below the ranges macOS and Linux hand out to clients.
const DEBUG_CABLE_PORT: u16 = 7661;

/// A pack flag's argument: the image, and after commas in either order
/// the drive's unit and `ro`, its read-only switch.
#[derive(Debug, PartialEq, Eq, Clone)]
struct Pack {
    path: PathBuf,
    unit: usize,
    read_only: bool,
}

/// `<image>[,<unit>][,ro]`, the parts after the image in either order:
/// unit 0 and read-write unless said, `rw` allowed for saying so.
fn pack_spec(arg: &str) -> Result<Pack, String> {
    let mut parts = arg.split(',');
    let path = match parts.next() {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => return Err("wants an image".into()),
    };
    let (mut unit, mut read_only) = (None, None);
    for part in parts {
        if part == "ro" || part == "rw" {
            if read_only.replace(part == "ro").is_some() {
                return Err("ro or rw twice".into());
            }
        } else if !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) {
            let u = part.parse().ok().filter(|&u| u < muir::disk_controller::UNITS);
            let u = u.ok_or_else(|| format!("unit {part} is not one of 0 to 7"))?;
            if unit.replace(u).is_some() {
                return Err("the unit twice".into());
            }
        } else {
            return Err(format!("{part:?} is neither a unit nor ro"));
        }
    }
    Ok(Pack { path, unit: unit.unwrap_or(0), read_only: read_only.unwrap_or(false) })
}

/// A pack flag's argument parsed, or the usage: with the disk controller
/// alone on the bus there is one drive, unit 0.
fn pack_flag(flag: &str, arg: Option<String>) -> Pack {
    let arg = arg.unwrap_or_else(|| usage(&format!("{flag} wants <image>[,<unit>][,ro]")));
    let pack = pack_spec(&arg).unwrap_or_else(|e| usage(&format!("{flag} {arg}: {e}")));
    if pack.unit != 0 {
        usage(&format!(
            "{flag} {arg}: the disk multiplexor is not modelled yet; one pack, as unit 0"
        ));
    }
    pack
}

/// An endpoint from a flag's argument, against a default: nothing is the
/// default, a port is that port at the default's address, an address is
/// the default's port there, and address:port is itself. A bare port is
/// on the loopback address, which is where an unauthenticated server
/// belongs unless someone says otherwise in as many words.
fn endpoint_at(spec: Option<&str>, default: SocketAddr) -> Option<SocketAddr> {
    let Some(v) = spec else {
        return Some(default);
    };
    if let Ok(a) = v.parse::<SocketAddr>() {
        return Some(a);
    }
    if let Ok(p) = v.parse::<u16>() {
        return Some(SocketAddr::new(default.ip(), p));
    }
    v.parse::<IpAddr>().ok().map(|ip| SocketAddr::new(ip, default.port()))
}

/// The other machine's display against this machine's: the display above
/// it unless `--debuggee-terminal` says another. A free display is looked
/// for by moving the port up, so this against the endpoint that was asked
/// for and this against the one that was bound differ only in the port
/// each already carries.
fn debuggee_endpoint(spec: Option<&str>, base: SocketAddr) -> SocketAddr {
    let above = base.port().checked_add(1).unwrap_or_else(|| {
        usage("--terminal: no port above this one for the other machine's display")
    });
    endpoint_at(spec, SocketAddr::new(base.ip(), above)).unwrap_or_else(|| {
        usage("--debuggee-terminal wants nothing, a port, an address or address:port")
    })
}

/// Whether a flag's endpoint names a port: nothing and a bare address do
/// not, a bare port and address:port do.
fn names_a_port(spec: Option<&str>) -> bool {
    spec.is_some_and(|v| v.parse::<SocketAddr>().is_ok() || v.parse::<u16>().is_ok())
}

/// [`endpoint_at`] the loopback at `port`.
fn endpoint(spec: Option<&str>, port: u16) -> Option<SocketAddr> {
    endpoint_at(spec, SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
}

/// What the start says a terminal is: where a viewer connects to it, or
/// why there is none.
fn terminal_line(terminal: &Option<Terminal>, why: &Option<String>, at: SocketAddr) -> String {
    match (terminal, why) {
        (Some(t), _) => format!("vnc://{} --- RFB, no password", t.addr().unwrap_or(at)),
        (None, Some(why)) => format!("none --- {why}"),
        (None, None) => "none".to_string(),
    }
}

/// Serves the screen as it was left, while anyone is still looking.
fn serve_last_screen(terminal: &mut Terminal, tv: &muir::simpletv::SimpleTv) {
    serve_last_screens(&mut [(terminal, tv)]);
}

/// Serves each screen as it was left, while anyone is still looking at
/// any of them.
fn serve_last_screens(screens: &mut [(&mut Terminal, &muir::simpletv::SimpleTv)]) {
    let looking = |screens: &[(&mut Terminal, &muir::simpletv::SimpleTv)]| {
        screens.iter().any(|(t, _)| t.viewers() > 0)
    };
    if !looking(screens) {
        return;
    }
    eprintln!("terminal: serving the last screen while a viewer is on it; ^C to stop");
    while looking(screens) {
        for (terminal, tv) in screens.iter_mut() {
            terminal.poll(Frame::of(tv));
        }
        std::thread::sleep(TERMINAL_INTERVAL);
    }
}

/// One turn of a machine's terminal: the screen out and the keys and the
/// pointer in when it is time to poll, and whatever the keyboard and mouse
/// hold delivered to the I/O board as it takes them.
fn attend(
    terminal: Option<&mut Terminal>,
    poll: bool,
    m: &mut Machine,
    keyboard: &mut Keyboard,
    mouse: &mut Mouse,
) {
    if poll && let Some(term) = terminal {
        term.poll(Frame::of(&m.simpletv));
        for (keysym, down) in term.take_keys() {
            keyboard.key(keysym, down);
        }
        for (buttons, x, y) in term.take_pointers() {
            mouse.pointer(buttons, x, y);
        }
    }
    let board = &mut m.ioboard;
    if keyboard.pending() > 0 {
        keyboard.deliver(board);
    }
    if mouse.pending(board.mouse_buttons_held()) {
        mouse.deliver(board);
    }
}

const NETLIST: &str = include_str!("../data/CADR.netlist");
const BUSINT: &str = include_str!("../data/BUSINT.netlist");
const CADRM: &str = include_str!("../data/CADRM.netlist");
const CADRIO: &str = include_str!("../data/CADRIO.netlist");
const SIMPLETV: &str = include_str!("../data/SIMPLETV.netlist");
const LISPMTV: &str = include_str!("../data/LISPMTV.netlist");
const CADRDC: &str = include_str!("../data/CADRDC.netlist");

/// Which display board `--tv-board` puts on the backplane.
#[derive(Clone, Copy, PartialEq)]
enum TvBoard {
    SimpleTv,
    LispmTv,
}

/// The pack the cosim tests run, used when `--disk-pack` names none: under
/// the directory `muir` is run from.
const VENDORED_PACK: &str = "vendor/run/disk-sys-100-0.img";

/// Where the Chaosnet server's FILE service serves from when no root is
/// named, beside the vendored pack and used the same way: taken when it
/// is there, and without it there is no FILE service.
///
/// A directory of its own rather than `vendor/` or the release, because
/// the service writes, renames and deletes under its root and fetched
/// material should not be in reach of a running machine by accident.
/// The band asks its file host for `/tree/...`, and the release's own
/// `sys` directory is what that host had there, so
///
/// ```text
/// mkdir -p vendor/run/file-root
/// ln -s ../../system-100-0/sys vendor/run/file-root/tree
/// ```
///
/// makes `SYS: SYS2; FOO LISP` resolve. Left empty it is harmless.
const VENDORED_FILE_ROOT: &str = "vendor/run/file-root";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Which {
    Micro,
    Rtl,
    Chip,
}

fn report(name: &str, cycles: u64, secs: f64) {
    let rate = cycles as f64 / secs;
    let ratio = rate / HARDWARE_CYCLES_PER_S;
    // Far below real time the reciprocal is the readable number:
    // "hardware/2800" says what "0.00x" does not. Near it, the ratio does.
    let against = if ratio >= 0.1 {
        format!("{ratio:.2}x hardware")
    } else {
        format!("hardware/{:.0}", 1.0 / ratio)
    };
    println!(
        "{name:6} {cycles:>9} microcycles  {secs:7.3} s  {rate:>11.0} microcycles/s  {against:>16}"
    );
}

const USAGE: &str = "usage: muir [--micro|--rtl|--chip] [--chaos-address <this>[,<server>]]
            [--chaos-file-root <dir>] [--chaos-trace] [--checkpoint <file>]
            [-c|--config <file>] [--debug-cable-connect [<endpoint>]]
            [--debug-cable-listen [<endpoint>]] [--debug-in-process]
            [--debuggee-chaos-address <this>[,<server>]]
            [--debuggee-chaos-file-root <dir>]
            [--debuggee-disk-pack <image>[,<unit>][,ro]]
            [--debuggee-terminal [<endpoint>]]
            [--disk-controller netlist|model]
            [--disk-pack <image>[,<unit>][,ro]] [--io-board netlist|model]
            [--main-memory netlist|model] [--main-memory-boards <n>]
            [--no-auto-boot] [--prom <file>] [--resume <file>]
            [--stop-after <microcycles>] [--stop-at <pc>]
            [--stop-at-prom <pc>] [--terminal [<endpoint>]]
            [--tv netlist|model] [--tv-board simple-tv|lispm-tv]
            [--tv-capture <gif>] [--tv-capture-no-time] [-V|--version]";

/// What `-h` and `--help` print: the usage, then each flag in the order
/// the usage lists them.
const HELP: &str = "\
A simulator of the MIT CADR Lisp Machine.

  --micro | --rtl | --chip     the engine: microinstruction, register
                               transfer, or chip level. [default: --rtl]
  --chaos-address <this>[,<server>]
                               rtl, chip: this machine's Chaosnet address,
                               always, and after a comma the Chaosnet
                               server's --- the address muir's server on
                               the other end of the cable answers at, for
                               STATUS, TIME and UPTIME, and FILE when given
                               a root. Each address is the sixteen bits in
                               octal, 3050, or subnet:host with each in
                               octal, 6:50 --- the same number, subnet in
                               the high byte. [default: 3050,3060]
  --chaos-file-root <dir>
                               rtl, chip: the directory the Chaosnet server
                               serves as its /; the band asks it for
                               /tree/sys/... It may not be /: the service
                               writes, renames and deletes under it.
                               [default: vendor/run/file-root when present;
                               with no root the server answers no FILE at
                               all]
  --checkpoint <file>          micro, rtl: write the machine's whole state
                               to <file> when the run stops, for --resume
                               to start from: the engine, the processor's
                               memories and registers, main memory, the
                               display, the I/O board, and the drive with
                               every block written to its pack. The
                               prompt's checkpoint writes one as the run
                               goes, and the run goes on.
  -c, --config <file>          the file of flags to read before the command
                               line, which must be there. Without it muir
                               reads .muirrc in the directory it was run
                               from, or failing that .muirrc in the home
                               directory --- the first of the three there,
                               not all of them. MUIR_RC names a file in
                               place of the two that are looked for.
  --debug-cable-connect [<endpoint>]
                               rtl: this machine is the debugger: its DBGOUT
                               connects to a debuggee listening at the
                               endpoint, a port, an address or
                               address:port. Either end may be another
                               program that speaks the cable's frames.
                               [default: 127.0.0.1:7661]
  --debug-cable-listen [<endpoint>]
                               rtl, chip: this machine is the debuggee at
                               the end of a debug cable over TCP: its DBGIN
                               waits at the endpoint for the debugger to
                               connect, and then the two run in step. On
                               chip it is the board's own connector, run an
                               event at a time. [default: 127.0.0.1:7661]
  --debug-in-process           rtl: the two-machine lashup in one process.
                               A second machine runs beside this one with
                               both debug cables between them, each
                               machine's DBGOUT to the other's DBGIN, so
                               that CC here debugs the other --- or the
                               other this one --- as MIT ran two CADRs.
                               The other boots the same PROM with no pack
                               unless one is named, and its console is
                               CC's alone. The window and the stops are
                               this machine's; with --terminal both
                               machines get a terminal, the other's one
                               port above.
  --debuggee-chaos-address <this>[,<server>]
                               rtl: the other machine's Chaosnet, as
                               --chaos-address is this machine's. The other
                               machine has a Chaosnet of its own: its own
                               cable with its own server on it, since
                               muir's cable carries one machine. The two
                               cannot hear each other over it, and the only
                               wire between them is the debug cable.
                               [default: the same addresses as this
                               machine's, which collide with nothing, the
                               two cables never meeting]
  --debuggee-chaos-file-root <dir>
                               rtl: the directory the other machine's
                               Chaosnet server serves, as
                               --chaos-file-root is this machine's. Two
                               servers rooted at one directory are two
                               hosts sharing a filesystem, with nothing
                               between them to keep one from writing what
                               the other is reading. [default: none, so its
                               server answers STATUS, TIME and UPTIME but
                               no FILE]
  --debuggee-disk-pack <image>[,<unit>][,ro]
                               rtl: the other machine's pack, as
                               --disk-pack.
  --debuggee-terminal [<endpoint>]
                               rtl: the other machine's terminal --- its
                               display, keyboard and mouse over RFB, as
                               --terminal is this machine's. In the lashup
                               both machines are served a terminal, the
                               other machine's the display above this
                               one's; this puts it elsewhere, a port, an
                               address or address:port. [default: the
                               display above this machine's,
                               127.0.0.1:5901 when it is at :0]
  --disk-controller netlist|model
                               chip: the disk controller. [default: model]
  --disk-pack <image>[,<unit>][,ro]
                               the pack in a drive: its blocks end to end.
                               The file is only ever read; a block the
                               machine writes is kept in memory for the
                               run, and goes into a checkpoint, so the
                               image stays as fetched. After the image, in
                               either order: the unit, only 0 until the
                               disk multiplexor is modelled, and ro for the
                               drive's read-only switch --- the status word
                               says so, and a write faults. [default: unit
                               0; vendor/run/disk-sys-100-0.img when
                               present, and with no pack the boot waits on
                               a drive that never answers]
  --io-board netlist|model     chip: the I/O board. [default: netlist]
  --main-memory netlist|model  chip: main memory as MIT's board or as
                               rtl's model of it. [default: netlist]
  --main-memory-boards <n>     how many 64K-word boards, 1 to 60: main
                               memory on every engine, and on chip the
                               boards on the backplane. [default: 32, the
                               two million words]
  --no-auto-boot               leave the boot button unpressed, as a CADR
                               is when the power comes on: RUN is
                               clear, the machine is halted, and nothing
                               runs. The run starts held at the prompt, so
                               that the machine can be looked at as it came
                               up; boot there presses the button, and the
                               machine runs from that. Nothing else starts
                               it: continue and step say so. [default: muir
                               presses the button for you]
  --prom <file>                the boot PROM to run, an MCR microcode file
                               as MIT's own sys/ubin/promh.mcr is: the 512
                               words the machine fetches before it turns
                               the PROM off. A program longer than that, or
                               assembled somewhere other than address 0, or
                               setting the statistics bit IR<46>, which a
                               burned word has nowhere to hold, is refused
                               rather than run. The start says how the file
                               stands to MIT's own, which matters: recovered
                               copies of the boot PROM are not all the same
                               program. [default: MIT's own, built in ---
                               System 100's sys/ubin/promh.mcr, version 9]
  --resume <file>              micro, rtl: start from a checkpoint instead
                               of cold: the engine that wrote it, the same
                               pack under it, the Chaosnet plugged in
                               afresh, and as many memory boards as it had,
                               which --main-memory-boards may not gainsay.
                               The stops count from here.
  --stop-after <microcycles>   how many to run, then stop. [default:
                               none; the run goes on until a --stop-at, a
                               halt or ^C]
  --stop-at <pc>               stop when the PC reaches this address with
                               the boot PROM disabled: in microcode loaded
                               into the control store. Octal, as MIT
                               writes it.
  --stop-at-prom <pc>          the same with the PROM enabled: an address
                               in the boot PROM, below 1000.
                               With --stop-after, whichever comes first.
  --terminal [<endpoint>]      where the display, keyboard and mouse are
                               served over RFB, RFC 6143, for any VNC
                               viewer to connect to: a port, an address or
                               address:port. Every run serves a terminal,
                               asked for or not --- the machine has no
                               other way in or out --- and this says where
                               instead. A named port is bound as it
                               stands, and the run stops if it cannot be;
                               an unnamed one is where the first free
                               display is looked for. An address other
                               than the loopback lets another machine in
                               --- RFB's None security is the only type
                               offered, so a viewer needs no password.
                               Without a Chaosnet the boot stops in the
                               debugger: Super-B, the date and y finish
                               it. On any engine the band's cold boot
                               leaves the vertical interrupt off, so
                               (si:setup-cpt) at the listener is what
                               turns the mouse on. [default:
                               127.0.0.1:5900, VNC's display
                               :0, or the first free display above it]
  --tv netlist|model           chip: the display. [default: netlist]
  --tv-board simple-tv|lispm-tv
                               chip: which display board. [default:
                               simple-tv]
  --tv-capture <gif>           record the display to <gif> as the run goes,
                               an animated GIF: each frame the rectangle
                               that changed since the last, over the one
                               before it, two colours and LZW, timed by the
                               machine's own clock so it plays at the
                               machine's speed. It stays small while the
                               screen stays still. There is no default path;
                               one must be given. In the lashup it is both
                               machines on one canvas, the debugger's screen
                               at the left and the debuggee's at the right
                               with a rule between them, so that a frame is
                               one instant on both: the two machines are one
                               clock there, which two files could not keep.
                               Not over the debug cable, where they are
                               two.
  --tv-capture-no-time         leave the clocks off the recording. By
                               default a line below the screen, hiding no
                               part of the display, shows the machine's
                               simulated time at the left and the wall
                               clock, the local time of day, at the right,
                               each hh:mm:ss; this drops that line.
  --chaos-trace                rtl, chip: every Chaosnet packet and frame on
                               the cable, to stderr. What to reach for when a
                               lashup goes quiet: it shows whether the machine
                               is still talking.
  -h, --help                   this.
  -V, --version                what this build calls itself: the version,
                               and whether it was built with optimisations
                               off. Every run says it in its first line too.
";

/// What this build calls itself: the crate's version, and whether it was
/// built with optimisations off. `--version` prints it, and every run says
/// it in its first line, so a report of a run says which muir made it.
fn version() -> String {
    let build = if cfg!(debug_assertions) { "dev" } else { "release" };
    format!("muir {}-{build}", env!("CARGO_PKG_VERSION"))
}

/// A run that cannot start, for a reason that is not the command line's
/// shape: a pack that will not open, a port already taken. The flags are no
/// help, so they are not printed.
fn fail(msg: &str) -> ! {
    eprintln!("muir: {msg}");
    std::process::exit(1);
}

fn usage(msg: &str) -> ! {
    eprintln!("muir: {msg}");
    eprintln!("{USAGE}");
    eprintln!("muir --help says more");
    std::process::exit(2);
}

fn help() -> ! {
    println!("{USAGE}\n\n{HELP}");
    std::process::exit(0);
}

/// The directory the Chaosnet server's FILE service serves as its `/`.
///
/// Given explicitly or not at all: there is no default, because the
/// service writes, renames and deletes under whatever it is handed. It
/// is resolved to an absolute path so that a relative one cannot be
/// re-read against a different working directory, it must already be a
/// directory, and **it may not be the filesystem root** --- the band
/// asks for paths like `/tree/sys/...`, and rooted at `/` those would be
/// the real ones.
fn file_root(flag: &str, given: &str) -> PathBuf {
    let path = match std::fs::canonicalize(given) {
        Ok(p) => p,
        Err(e) => usage(&format!("{flag} {given}: {e}")),
    };
    if !path.is_dir() {
        usage(&format!("{flag} {given} is not a directory"));
    }
    if path.parent().is_none() {
        usage(&format!("{flag} may not be /: the FILE service writes under it"));
    }
    path
}

/// Whether the run was left to find its own file root.
fn machine_chaos_wants_default(chaos: &muir::chaos::Config) -> bool {
    chaos.file_root.is_none()
}

/// The root to serve from when none was named: `vendor/run/file-root`
/// under the current directory if it is there, and otherwise none, which
/// leaves the Chaosnet server with no FILE service. The setup says which.
fn default_file_root() -> Option<PathBuf> {
    let vendored = PathBuf::from(VENDORED_FILE_ROOT);
    match std::fs::canonicalize(&vendored) {
        Ok(p) if p.is_dir() => Some(p),
        _ => None,
    }
}

/// The pack a run gets: the one named, else `vendor/run/disk-sys-100-0.img`
/// under the current directory if it is there, else none. The path, the
/// unit and the drive's read-only switch.
fn pack_choice(pack: Option<&Pack>) -> Option<(PathBuf, usize, bool)> {
    let vendored = PathBuf::from(VENDORED_PACK);
    match pack {
        Some(p) => Some((p.path.clone(), p.unit, p.read_only)),
        None if vendored.exists() => Some((vendored, 0, false)),
        None => None,
    }
}

/// What a file of flags is called where muir looks for one.
const RC: &str = ".muirrc";

/// The file of flags this run reads, and whether it was asked for by name.
///
/// `--config` if it is given, else `.muirrc` in the directory muir was run
/// from, else `.muirrc` in the user's home directory: **the first of those
/// there, not all of them**, so a file in the directory is the whole of
/// the run's flags and not an addition to the home one.  `MUIR_RC` stands
/// in for the two that are looked for, which is how a test gives a run a
/// file of its own.
///
/// A file asked for by name must be there; the ones looked for need not
/// be, and most runs have none.
fn config_path(typed: &[String]) -> Option<(PathBuf, bool)> {
    let mut typed = typed.iter();
    while let Some(word) = typed.next() {
        if word == "-c" || word == "--config" {
            let Some(path) = typed.next() else { usage("--config wants a file of flags") };
            return Some((PathBuf::from(path), true));
        }
    }
    if let Some(named) = std::env::var_os("MUIR_RC") {
        return Some((PathBuf::from(named), false));
    }
    let here = PathBuf::from(RC);
    if here.exists() {
        return Some((here, false));
    }
    std::env::var_os("HOME").map(|home| (PathBuf::from(home).join(RC), false))
}

/// The flags in the file, [`config_path`], and which file that was.
///
/// A line is a flag and, after a space, whatever it takes, which is the
/// rest of the line --- so a path with a space in it is one word, as it is
/// in a shell's quotes.  A line whose first word is blank, or begins with
/// `#`, is a comment.
///
/// A flag the command line gives too is left out, and so is the engine
/// when the command line names one: the command line is what the person
/// at the keyboard means this time, and `--micro`, `--rtl` and `--chip`
/// are exclusive of each other, not last-wins.  A file cannot name
/// another; that is the command line's to say.
fn muirrc(typed: &[String]) -> (Vec<String>, Option<PathBuf>) {
    const ENGINES: [&str; 3] = ["--micro", "--rtl", "--chip"];
    let Some((path, named)) = config_path(typed) else { return (Vec::new(), None) };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if named => usage(&format!("--config {}: {e}", path.display())),
        // One that is not there is one a run has none of.
        Err(_) => return (Vec::new(), None),
    };
    let engine_typed = typed.iter().any(|a| ENGINES.contains(&a.as_str()));
    let mut flags = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (flag, value) = match line.split_once(char::is_whitespace) {
            Some((flag, value)) => (flag, value.trim()),
            None => (line, ""),
        };
        if flag == "-c" || flag == "--config" {
            usage(&format!("{}: a file of flags cannot name another", shown(&path)));
        }
        if typed.iter().any(|a| a == flag) || (engine_typed && ENGINES.contains(&flag)) {
            continue;
        }
        flags.push(flag.to_string());
        if !value.is_empty() {
            flags.push(value.to_string());
        }
    }
    (flags, Some(path))
}

/// A path as the setup writes it: what it is under the directory muir was
/// run from when it is under it, and the whole path otherwise.  The pack
/// and the Chaosnet server's file root are looked for under that
/// directory, and their whole paths are long and say nothing a reader
/// does not know.
fn shown(path: &Path) -> String {
    let Ok(here) = std::env::current_dir() else {
        return path.display().to_string();
    };
    match path.strip_prefix(&here) {
        Ok(p) if p.as_os_str().is_empty() => ".".to_string(),
        Ok(p) => p.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

/// Main memory's size for `boards` of 64K words, in KW, or in MW when it
/// comes to whole ones.
fn memory_size(boards: usize) -> String {
    let kw = boards * 64;
    if kw.is_multiple_of(1024) { format!("{} MW", kw / 1024) } else { format!("{kw} KW") }
}

/// Attaches the pack, [`pack_choice`], to its unit.
fn attach(m: &mut Machine, pack: Option<&Pack>) {
    let Some((p, unit, read_only)) = pack_choice(pack) else { return };
    // A drive writes its pack, so the image is opened read-write and a
    // written block goes into the file. `ro` is the drive's own read-only
    // switch: the file is opened read-only behind it, a written block stays
    // in memory for the run and goes into a checkpoint instead, and the
    // machine sees the write fault as MIT says it does.
    let opened =
        if read_only { Unit::open(&p, Geometry::T300) } else { Unit::open_rw(&p, Geometry::T300) };
    let mut u = match opened {
        Ok(u) => u,
        Err(e) => fail(&format!("{}: {e}", p.display())),
    };
    u.read_only = read_only;
    m.disk.attach(unit, u);
}

fn machine(prom: &[Insn], pack: Option<&Pack>, memory_boards: usize) -> Machine {
    let mut m = Machine::with_memory_boards(memory_boards);
    m.load_prom(prom);
    attach(&mut m, pack);
    m
}

/// The boot PROM this run loads: MIT's own, built in, unless `--prom`
/// names an MCR microcode file of one's own.
///
/// A file muir cannot read stops the run before it starts. It is the
/// program the machine is about to execute, so there is nothing to fall
/// back on: 512 zero words are not a boot PROM.
fn boot_prom(file: Option<&Path>) -> Vec<Insn> {
    let Some(path) = file else { return muir::prom::boot_prom() };
    let bytes =
        std::fs::read(path).unwrap_or_else(|e| usage(&format!("--prom {}: {e}", shown(path))));
    muir::prom::parse_mcr(&bytes).unwrap_or_else(|e| usage(&format!("--prom {}: {e}", shown(path))))
}

/// What the setup says the boot PROM is: which file, and how it stands to
/// MIT's own.
///
/// Recovered copies of the boot PROM are not all the same program --- two
/// builds of "version 9" exist that differ in 214 of their 454 words, and
/// nothing about a copy announces which it is --- so a run on a file of
/// one's own says how the file stands to MIT's: word for word, or how
/// many words apart.
fn prom_shown(file: Option<&Path>, prom: &[Insn]) -> String {
    let Some(path) = file else {
        return "built in, System 100's own sys/ubin/promh.mcr, version 9".to_string();
    };
    let mits = muir::prom::boot_prom();
    match prom.iter().zip(&mits).filter(|(a, b)| a != b).count() {
        0 => format!("{}, MIT's own word for word", shown(path)),
        n => format!("{}, {n} of its {} words differing from MIT's own", shown(path), mits.len()),
    }
}

/// Where a run stops: after so many microcycles, or when the PC reaches
/// an address --- one for the boot PROM enabled, one for it disabled, the
/// two sharing their low addresses --- whichever comes first.
#[derive(Clone, Copy)]
struct Stop {
    after: u64,
    at: Option<u16>,
    at_prom: Option<u16>,
}

impl Stop {
    /// Whether the PC, with the PROM as it is, is where the run stops.
    fn reached(&self, pc: u16, prom_enabled: bool) -> bool {
        if prom_enabled { self.at_prom == Some(pc) } else { self.at == Some(pc) }
    }

    /// Says how the run ended, after the rate.
    fn conclude(&self, ran: u64, pc: u16, prom_enabled: bool, halt: Option<muir::machine::Halt>) {
        let prom = if prom_enabled { " in the PROM" } else { "" };
        if self.reached(pc, prom_enabled) {
            println!("       stopped at PC {pc:o}{prom} after {ran}");
        } else if let Some(h) = halt {
            println!("       stopped after {ran}: {h:?}");
        } else if self.at.is_some() || self.at_prom.is_some() {
            println!("       stop not reached in {ran}; PC {pc:o}{prom}");
        } else {
            // A run that simply reaches its cycle count still has to say
            // where it got to. A nine-hour `--stop-after` through the
            // boot's microcode load reported nothing but a rate, and
            // whether it had reached microcode 323 at all could not be
            // told from its output.
            println!("       ran out at {ran}; PC {pc:o}{prom}");
        }
    }
}

/// The lashup in one process: this machine, the debugger, with the
/// terminal and the stops, and the debuggee stepped beside it as the cable
/// allows --- [`Lashup::step`] moves whichever may move, so a microcycle
/// counted here is one of the debugger's.
fn time_lashup(
    mut lashup: Lashup,
    stop: Stop,
    terminal: Option<&mut Terminal>,
    debuggee_terminal: Option<&mut Terminal>,
    capture: Option<(PathBuf, bool)>,
) {
    let t = Instant::now();
    let mut halt = None;
    let (mut terminal, mut debuggee_terminal) = (terminal, debuggee_terminal);
    // Both screens on one canvas: the debugger's at the left and the
    // debuggee's at the right, timed by the debugger's clock, which the
    // lashup holds the debuggee's to within a generator cycle.
    let mut capture = capture.map(|(path, time)| (path, Recorder::pair(time)));
    let (mut keyboard, mut mouse) = (Keyboard::new(), Mouse::new());
    let (mut b_keyboard, mut b_mouse) = (Keyboard::new(), Mouse::new());
    let mut last_poll = Instant::now();
    let mut ran = 0;
    let (mut a_cycles, mut b_cycles) = (0u64, 0u64);
    catch_interrupts();
    let mut interrupts_seen = 0;
    loop {
        let e = &lashup.debugger;
        if ran >= stop.after || stop.reached(e.pc(), !e.machine().mode.prom_disable) {
            break;
        }
        if interrupted(&mut interrupts_seen) {
            break;
        }
        let before = lashup.steps;
        if let Err(h) = lashup.step() {
            halt = Some(h);
            break;
        }
        if lashup.steps.0 == before.0 {
            continue;
        }
        ran += 1;
        a_cycles = lashup.debugger.machine().cycles;
        b_cycles = lashup.debuggee.machine().cycles;
        if ran % TERMINAL_CHECK == 0 {
            if let Some((_, rec)) = capture.as_mut() {
                rec.sample_pair(
                    &lashup.debugger.machine().simpletv,
                    &lashup.debuggee.machine().simpletv,
                    lashup.debugger.ns(),
                    wall_clock(),
                );
            }
            let poll = last_poll.elapsed() >= TERMINAL_INTERVAL;
            let m = lashup.debugger.machine_mut();
            attend(terminal.as_deref_mut(), poll, m, &mut keyboard, &mut mouse);
            let m = lashup.debuggee.machine_mut();
            attend(debuggee_terminal.as_deref_mut(), poll, m, &mut b_keyboard, &mut b_mouse);
            if poll {
                last_poll = Instant::now();
            }
        }
    }
    report("rtl, debugger", ran, t.elapsed().as_secs_f64());
    let e = &lashup.debugger;
    stop.conclude(ran, e.pc(), !e.machine().mode.prom_disable, halt);
    println!(
        "       debuggee: {b_cycles} microcycles to {} ns, PC {:o}{}; debugger {a_cycles} to {} ns; \
         {} debug cycles on the cable",
        lashup.debuggee.ns(),
        lashup.debuggee.pc(),
        if lashup.debuggee.machine().mode.prom_disable { "" } else { " in the PROM" },
        lashup.debugger.ns(),
        lashup.debugger.debug_cycles()
    );
    if let Some((path, rec)) = capture.as_mut() {
        rec.sample_pair(
            &lashup.debugger.machine().simpletv,
            &lashup.debuggee.machine().simpletv,
            lashup.debugger.ns(),
            wall_clock(),
        );
        write_capture(path, rec);
    }
    let mut screens = Vec::new();
    if let Some(term) = terminal {
        screens.push((term, &lashup.debugger.machine().simpletv));
    }
    if let Some(term) = debuggee_terminal {
        screens.push((term, &lashup.debuggee.machine().simpletv));
    }
    serve_last_screens(&mut screens);
}

/// One end of the cable over TCP: this machine, debugger or debuggee, run
/// in step with the other program by [`Remote::step`], with the terminal
/// and the stops as for a machine alone; at the end the two agree to stop.
fn time_remote(name: &str, mut remote: Remote<Rtl>, stop: Stop, terminal: Option<&mut Terminal>) {
    let t = Instant::now();
    let mut halt = None;
    let mut terminal = terminal;
    let mut keyboard = Keyboard::new();
    let mut mouse = Mouse::new();
    let mut last_poll = Instant::now();
    let mut ran = 0;
    catch_interrupts();
    let mut interrupts_seen = 0;
    loop {
        let e = &remote.machine;
        if ran >= stop.after || stop.reached(e.pc(), !e.machine().mode.prom_disable) {
            break;
        }
        if interrupted(&mut interrupts_seen) {
            break;
        }
        match remote.step() {
            Ok(true) => ran += 1,
            Ok(false) => continue,
            Err(muir::lashup::Error::Halt(h)) => {
                halt = Some(h);
                break;
            }
            Err(e) => {
                eprintln!("muir: the debug cable: {e}");
                break;
            }
        }
        if ran % TERMINAL_CHECK == 0 {
            let e = &mut remote.machine;
            if last_poll.elapsed() >= TERMINAL_INTERVAL
                && let Some(term) = terminal.as_deref_mut()
            {
                term.poll(Frame::of(&e.machine().simpletv));
                for (keysym, down) in term.take_keys() {
                    keyboard.key(keysym, down);
                }
                for (buttons, x, y) in term.take_pointers() {
                    mouse.pointer(buttons, x, y);
                }
                last_poll = Instant::now();
            }
            let board = &mut e.machine_mut().ioboard;
            if keyboard.pending() > 0 {
                keyboard.deliver(board);
            }
            if mouse.pending(board.mouse_buttons_held()) {
                mouse.deliver(board);
            }
        }
    }
    report(name, ran, t.elapsed().as_secs_f64());
    let e = &remote.machine;
    stop.conclude(ran, e.pc(), !e.machine().mode.prom_disable, halt);
    println!("       {} debug cycles on the cable", remote.machine.debug_cycles());
    if let Err(e) = remote.finish() {
        eprintln!("muir: the debug cable at the end: {e}");
    }
    if let Some(term) = terminal {
        serve_last_screen(term, &remote.machine.machine().simpletv);
    }
}

/// What a run is to do besides run the machine: where it stops, what it
/// records as it goes, what it writes when it is over, what it says it is
/// when `info` asks, and whether it starts held.
struct Run<'a> {
    stop: Stop,
    capture: Option<(PathBuf, bool)>,
    checkpoint: Option<PathBuf>,
    setup: &'a str,
    /// The run starts held, with the machine as `--no-auto-boot` left it:
    /// the button unpressed, and nothing to run until the prompt's `boot`.
    hold: bool,
    /// Whether a recording carries the clocks below the screen, which
    /// `--tv-capture-no-time` turns off: the prompt's `startcapture` makes
    /// its recorder with it.
    clocks: bool,
}

fn time_engine<E: Engine>(name: &str, mut e: E, terminal: Option<&mut Terminal>, run: Run) {
    let Run { stop, capture, checkpoint, setup, hold, clocks } = run;
    let t = Instant::now();
    let mut ran = 0;
    let mut halt = None;
    let mut terminal = terminal;
    let mut keyboard = Keyboard::new();
    let mut mouse = Mouse::new();
    let mut last_poll = Instant::now();
    let mut capture = capture.map(|(path, time)| (path, Recorder::new(time)));
    let prompt = Prompt::open();
    // The prompt's hold: no microcycle runs while it is on. `step` takes it
    // off for so many microcycles, and no further line is read until they
    // have run, so that lines act in the order they were typed.
    // `--no-auto-boot` starts the run with it on, the machine halted and
    // its button unpressed, before the first microcycle.
    let mut held = hold;
    let mut stepping: Option<u64> = None;
    let mut quit = false;
    catch_interrupts();
    let mut interrupts_seen = 0;
    while !quit && ran < stop.after && !stop.reached(e.pc(), !e.machine().mode.prom_disable) {
        if held {
            // Nothing runs, and the terminal and the prompt are attended at
            // the terminal's pace.
            std::thread::sleep(TERMINAL_INTERVAL / 4);
        } else {
            if let Err(h) = e.step() {
                halt = Some(h);
                break;
            }
            ran += 1;
            if let Some(left) = stepping.as_mut() {
                *left -= 1;
                if *left == 0 {
                    stepping = None;
                    held = true;
                    say_pc(&e, ran);
                }
            }
        }
        let check = held || ran % TERMINAL_CHECK == 0;
        if check
            && !held
            && let Some((_, rec)) = capture.as_mut()
        {
            rec.sample(&e.machine().simpletv, e.machine().ns, wall_clock());
        }
        if check
            && last_poll.elapsed() >= TERMINAL_INTERVAL
            && let Some(term) = terminal.as_deref_mut()
        {
            term.poll(Frame::of(&e.machine().simpletv));
            for (keysym, down) in term.take_keys() {
                keyboard.key(keysym, down);
            }
            for (buttons, x, y) in term.take_pointers() {
                mouse.pointer(buttons, x, y);
            }
            last_poll = Instant::now();
        }
        // The keyboard hands the board one word at a time, as the board
        // takes them; a glance every check is far more often than the
        // machine reads it. The mouse's counts go in whole.
        if check {
            let board = &mut e.machine_mut().ioboard;
            if keyboard.pending() > 0 {
                keyboard.deliver(board);
            }
            if mouse.pending(board.mouse_buttons_held()) {
                mouse.deliver(board);
            }
        }
        // The machine stopping itself --- `(si:%halt)`, or the statistics
        // counter --- looks like nothing at all from `step`, which goes on
        // returning `Ok` and running no microcycle. So it is read off
        // `FLAG-1` here and held on, once, rather than spun on: the screen
        // has stopped, and without this the run says nothing about why.
        if check
            && !held
            && let Some(why) = machrun_low(&e)
        {
            held = true;
            stepping = None;
            say_machrun_low(why);
            say_pc(&e, ran);
        }
        // ^C: with a prompt to go on from, the first holds the machine
        // there and one more while held quits; with none, one quits. A
        // quit is the run's own end, so what it was to write gets written.
        if check {
            let seen = INTERRUPTS.load(Ordering::SeqCst);
            while interrupts_seen < seen {
                interrupts_seen += 1;
                let at_prompt = prompt.as_ref().is_some_and(|p| !p.ended());
                if held || !at_prompt {
                    quit = true;
                } else {
                    held = true;
                    stepping = None;
                    if let Some(prompt) = prompt.as_ref() {
                        prompt.past_interrupt();
                    }
                    println!("held at ^C; continue runs on, ^C again quits");
                    say_pc(&e, ran);
                }
            }
        }
        if check
            && stepping.is_none()
            && let Some(prompt) = prompt.as_ref()
        {
            // Read before the lines are: the reader thread sets it after
            // the last line it will ever send, so a hold left standing
            // when this was already true is one nothing can run on.
            let ending = prompt.ended();
            while let Some(line) = prompt.line() {
                match muir::prompt::parse(&line) {
                    Ok(None) => {}
                    Ok(Some(Command::Boot)) => {
                        // The button starts the machine: it presets RUN,
                        // and a finger on it is all a CADR is given.  So
                        // the hold comes off with it.
                        e.boot();
                        say_pc(&e, ran);
                        held = false;
                    }
                    Ok(Some(Command::Hold)) => {
                        held = true;
                        say_pc(&e, ran);
                    }
                    Ok(Some(Command::Continue)) => {
                        if halted(&e) {
                            say_halted();
                        } else if let Some(why) = machrun_low(&e) {
                            say_machrun_low(why);
                        } else {
                            held = false;
                        }
                    }
                    Ok(Some(Command::Step(n))) => {
                        if halted(&e) {
                            say_halted();
                        } else if let Some(why) = machrun_low(&e) {
                            say_machrun_low(why);
                        } else {
                            held = false;
                            stepping = Some(n);
                            break;
                        }
                    }
                    Ok(Some(Command::Pc)) => say_pc(&e, ran),
                    Ok(Some(Command::Registers)) => print!("{}", say_registers(&e)),
                    Ok(Some(Command::Dump { memory, from, words })) => {
                        match say_memory(e.machine(), memory, from, words) {
                            Ok(dump) => print!("{dump}"),
                            Err(what) => println!("prompt: {what}"),
                        }
                    }
                    Ok(Some(Command::Info)) => print!("{setup}"),
                    Ok(Some(Command::Screenshot(path))) => {
                        let path = path.unwrap_or_else(|| timestamped("png"));
                        write_screenshot(&path, &e.machine().simpletv);
                    }
                    Ok(Some(Command::StartCapture(path))) => match capture.as_ref() {
                        Some((going, _)) => println!(
                            "capture: one is going already, to {}; endcapture closes it",
                            going.display()
                        ),
                        None => {
                            let path = path.unwrap_or_else(|| timestamped("gif"));
                            println!(
                                "capture: recording the display to {}{}; endcapture writes it, and so does the stop",
                                path.display(),
                                if clocks { "" } else { ", no clocks" }
                            );
                            capture = Some((path, Recorder::new(clocks)));
                        }
                    },
                    Ok(Some(Command::EndCapture)) => match capture.take() {
                        Some((path, mut rec)) => {
                            rec.sample(&e.machine().simpletv, e.machine().ns, wall_clock());
                            write_capture(&path, &rec);
                        }
                        None => println!("capture: none is going; startcapture begins one"),
                    },
                    Ok(Some(Command::Checkpoint(path))) => {
                        write_checkpoint(name, &e, &path.unwrap_or_else(|| timestamped("chk")));
                    }
                    Ok(Some(Command::Quit)) => {
                        quit = true;
                        break;
                    }
                    Ok(Some(Command::Help)) => print!("{}", muir::prompt::HELP),
                    Err(what) => println!("prompt: {what}"),
                }
            }
            // A hold with no one left to type `continue` is a run that
            // would never end: stdin has ended, and ^C is the only thing
            // that could still reach it.  The run ends here instead, as a
            // quit does, with what it was to write written.
            if held && ending && !quit {
                println!("held, and stdin has ended: there is nothing to run the machine on");
                quit = true;
            }
            // The prompt is the held machine's: it is there while muir is
            // waiting to be told what to do next, and not while the
            // machine is running --- a line typed then is acted on all the
            // same, there is just nothing waiting for it.  Not while a
            // step is in flight either: the microcycles it asked for run
            // first, and the prompt comes back with where they left the
            // machine.
            if held && !quit && stepping.is_none() {
                prompt.show();
            }
        }
    }
    if let Some(prompt) = prompt.as_ref() {
        prompt.done();
    }
    report(name, ran, t.elapsed().as_secs_f64());
    if quit {
        let prom = if e.machine().mode.prom_disable { "" } else { " in the PROM" };
        println!("       quit at PC {:o}{prom} after {ran}", e.pc());
    } else {
        stop.conclude(ran, e.pc(), !e.machine().mode.prom_disable, halt);
    }
    if let Some((path, rec)) = capture.as_mut() {
        rec.sample(&e.machine().simpletv, e.machine().ns, wall_clock());
        write_capture(path, rec);
    }
    if let Some(path) = &checkpoint {
        write_checkpoint(name, &e, path);
    }
    release_interrupts();
    if !quit && let Some(term) = terminal {
        serve_last_screen(term, &e.machine().simpletv);
    }
}

/// How many ^Cs have come since the run began: `SIGINT`'s handler counts
/// them, and the run loop acts on each between two microcycles.
static INTERRUPTS: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_interrupt(_signal: std::ffi::c_int) {
    INTERRUPTS.fetch_add(1, Ordering::SeqCst);
}

/// `SIGINT`, 2 on every Unix, and `SIG_DFL`, 0.
const SIGINT: std::ffi::c_int = 2;
const SIG_DFL: usize = 0;

// POSIX `signal`, declared here as `localtime_r` is: the handler is a
// function's address, or `SIG_DFL`. The C libraries this builds against,
// macOS's and glibc, keep a handler installed after a signal, so one call
// serves the run.
unsafe extern "C" {
    fn signal(sig: std::ffi::c_int, handler: usize) -> usize;
}

/// Takes ^C for the run: counted, not fatal, until [`release_interrupts`].
fn catch_interrupts() {
    // SAFETY: installing a handler that does nothing but an atomic add,
    // which is safe to do in a signal handler.
    let handler: extern "C" fn(std::ffi::c_int) = on_interrupt;
    unsafe { signal(SIGINT, handler as *const () as usize) };
}

/// Whether ^C has been pressed since this was last asked.
///
/// A run that has no prompt to hold the machine from --- the lashup, either
/// end of a cable, the netlist engine --- ends on the first one, so that
/// what the run was to write is written: the recording, and the checkpoint.
/// [`time_engine`] wants more than this, a first ^C holding the machine and
/// a second quitting, and does it inline.
fn interrupted(seen: &mut u32) -> bool {
    let now = INTERRUPTS.load(Ordering::SeqCst);
    let asked = now > *seen;
    *seen = now;
    asked
}

/// Gives ^C back its meaning: the run is over, and what comes after it,
/// serving the last screen, ends the old way.
fn release_interrupts() {
    // SAFETY: putting the default back.
    unsafe { signal(SIGINT, SIG_DFL) };
}

/// The prompt's answer to `pc`, and to `hold` and `step`: where the
/// machine is.
fn say_pc<E: Engine>(e: &E, ran: u64) {
    let m = e.machine();
    let prom = if m.mode.prom_disable { "" } else { " in the PROM" };
    println!("PC {:o}{prom}; {} microcycles, {} ns; {ran} this run", e.pc(), m.cycles, m.ns);
}

/// Whether the machine is halted: `RUN` is clear, so the clock does not
/// reach the datapath and no microcycle can run.  It is what a CADR is
/// when the power comes on, and what `--no-auto-boot` leaves it as.
fn halted<E: Engine>(e: &E) -> bool {
    !e.machine().clock_control.run
}

/// Why `continue` and `step` do nothing for a halted machine.  Only the
/// button starts one, on the board and here.
fn say_halted() {
    println!("the machine is halted, its RUN clear: boot presses the button that starts it");
}

/// Where a netlist machine is, for the prompt.  [`say_pc`]'s counterpart:
/// `Chip` is not an [`Engine`] and keeps no microcycle count of its own ---
/// `time_chip` counts them by the clock phase wrapping --- so this says the
/// PC and the run's count and leaves the rest out.
fn say_pc_chip(c: &Chip, pc_nets: &[netlist::NetId], ran: u64, prom_enabled: bool) {
    let prom = if prom_enabled { " in the PROM" } else { "" };
    println!("PC {:o}{prom}; {ran} microcycles this run", c.read(pc_nets) as u16);
}

/// **The machine has stopped itself with `RUN` still set**, and why, or
/// `None` if it is running.  `MACHRUN` is low because `ERR` is up under
/// `ERRSTOP` --- which is what `HALT-CONS` does, and so what System 100's
/// `(si:%halt)` does --- or because the statistics counter ran out under
/// `STATHENB`.  A `WAIT` is neither: the machine comes out of a bus wait
/// by itself.
///
/// This is read from `FLAG-1`, where a console reads it, because nothing
/// in [`Engine::step`] says it has happened: a stopped machine's `step`
/// goes on returning `Ok`, advancing the master clock and running no
/// microcycle, so a run loop that watched only the return would spin here
/// for as long as it was left to.  `tests/halt.rs` holds both engines to
/// that.
fn machrun_low<E: Engine>(e: &E) -> Option<&'static str> {
    let f = muir::spy::Flag1::of(e.spy_read(muir::spy::FLAG_1));
    if !f.srun {
        // A cleared RUN is the other halt, and `halted` is its name.
        return None;
    }
    if f.err && e.machine().mode.errstop {
        return Some("ERR is up under ERRSTOP, which is what HALT-CONS does: (si:%halt)");
    }
    if f.stathalt {
        return Some("the statistics counter ran out under STATHENB");
    }
    None
}

/// What a machine that stopped itself says, once, as it drops to the
/// prompt.  `boot` is the way on: the button presets `RUN` and resets the
/// console's registers, `ERRSTOP` among them, which is what a CADR's
/// operator does here too.
fn say_machrun_low(why: &str) {
    println!("the machine stopped itself: {why}");
    println!("it will not run on by itself; boot presses the button that starts it again");
}

/// The prompt's answer to `reg`: the machine's registers, in hex and as
/// characters.  The names are the hardware's: PC, OPC, Q, VMA, MD, LC,
/// SPCPTR, PDLPTR and PDLIDX are all names the boards' own nets carry, and
/// PDLPTR, SPC, VMA, MD, Q and LC are pages of MIT's drawing set besides.
///
/// OPC here is [`Machine::opc`], the PC of the microinstruction that just
/// executed, which every engine keeps.  The eight-deep OPCS shift register
/// the board holds behind it is `rtl`'s own, and a console reads it a word
/// at a time through the OPC control register.
fn say_registers<E: Engine>(e: &E) -> String {
    let m = e.machine();
    muir::prompt::registers(&[
        ("PC", e.pc() as u32),
        ("OPC", m.opc as u32),
        ("Q", m.q),
        ("VMA", m.vma),
        ("MD", m.md),
        ("LC", m.lc),
        ("SPCPTR", m.spcptr as u32),
        ("PDLPTR", m.pdl_pointer as u32),
        ("PDLIDX", m.pdl_index as u32),
        ("DISPATCH CONSTANT", m.dispatch_constant as u32),
        ("INTERRUPT CONTROL", m.interrupt_control),
    ])
}

/// The prompt's answer to `amem`, `mmem`, `dmem`, `pdl` and `spc`: so many
/// words from an address, or the rest of the memory, or why there are
/// none there.
fn say_memory(
    m: &Machine,
    memory: Memory,
    from: usize,
    words: Option<usize>,
) -> Result<String, String> {
    let all: &[u32] = match memory {
        Memory::Amem => &m.amem,
        Memory::Mmem => &m.mmem,
        Memory::Dmem => &m.dmem,
        Memory::Pdl => &m.pdl,
        Memory::Spc => &m.spc,
    };
    if from >= all.len() {
        return Err(format!(
            "{} is {:o} words, and {from:o} is past its end",
            memory.name(),
            all.len()
        ));
    }
    // A count past the end is the rest of the memory, however far past:
    // the largest count the prompt reads, added to the address, would
    // otherwise overflow.
    let to = match words {
        Some(n) => from.saturating_add(n).min(all.len()),
        None => all.len(),
    };
    Ok(muir::prompt::dump(&all[from..to], from))
}

/// The prompt: a line on stdin is a command to muir itself,
/// [`muir::prompt`], read on a thread of its own and acted on between two
/// microcycles, where the terminal is attended.
struct Prompt {
    lines: std::sync::mpsc::Receiver<String>,
    /// Stdin has ended: no line will come, and there is no one to type
    /// `continue` at a hold.
    ended: std::sync::Arc<AtomicBool>,
    /// Someone is typing at a terminal, rather than a script feeding a
    /// pipe: the `muir: ` at a hold is for them, and so is the newline past
    /// what the terminal echoes of a ^C.
    terminal: bool,
    /// A `muir: ` is on the screen with nothing written past it yet.
    showing: std::cell::Cell<bool>,
}

impl Prompt {
    /// Opens the prompt on stdin when it can be read: a pipe or a file, or
    /// a terminal that muir is in the foreground of. A terminal muir is in
    /// the background of is left alone, since a read from it would stop the
    /// process, and there is no prompt.
    fn open() -> Option<Prompt> {
        use std::io::BufRead;
        if !Prompt::possible() {
            return None;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let ended = std::sync::Arc::new(AtomicBool::new(false));
        let over = ended.clone();
        std::thread::Builder::new()
            .name("prompt".into())
            .spawn(move || {
                for line in std::io::stdin().lock().lines() {
                    let Ok(line) = line else { break };
                    if tx.send(line).is_err() {
                        break;
                    }
                }
                over.store(true, Ordering::SeqCst);
            })
            .ok()?;
        let terminal = std::io::stdin().is_terminal();
        Some(Prompt { lines: rx, ended, terminal, showing: std::cell::Cell::new(false) })
    }

    /// Whether stdin can be read for a prompt: anything but a terminal
    /// muir is in the background of.
    fn possible() -> bool {
        !(std::io::stdin().is_terminal() && !stdin_is_foreground())
    }

    /// The next line typed, if one is waiting.
    fn line(&self) -> Option<String> {
        let line = self.lines.try_recv().ok();
        if line.is_some() {
            // The terminal echoed the Enter that ended it, so what muir
            // writes next starts on a line of its own and the `muir: ` the
            // line was typed after is behind.
            self.showing.set(false);
        }
        line
    }

    /// Writes `muir: `, muir's own line, while the machine is held and one
    /// is not already on the screen.  Only to a terminal: a pipe or a file
    /// gets muir's answers alone, as a script wants them.
    fn show(&self) {
        use std::io::Write;
        if self.showing.get() || !self.terminal {
            return;
        }
        print!("{}", muir::prompt::PROMPT);
        let _ = std::io::stdout().flush();
        self.showing.set(true);
    }

    /// Past what the terminal echoed of a ^C: it came after a `muir: `,
    /// with no Enter to end the line.
    fn past_interrupt(&self) {
        self.showing.set(false);
        if self.terminal {
            println!();
        }
    }

    /// No further command will be typed: the line a `muir: ` is on is
    /// ended, so the run's last words start on one of their own.
    fn done(&self) {
        if self.showing.replace(false) {
            println!();
        }
    }

    /// Whether stdin has ended, so that no line will come.
    fn ended(&self) -> bool {
        self.ended.load(Ordering::SeqCst)
    }
}

/// Whether this process is in its terminal's foreground process group, so
/// that reading the terminal does not stop it with `SIGTTIN`: POSIX's
/// `tcgetpgrp` on stdin against `getpgrp`, declared here as `localtime_r`
/// is; both take and give a `pid_t`, an `int`.
fn stdin_is_foreground() -> bool {
    use std::ffi::c_int;
    unsafe extern "C" {
        fn tcgetpgrp(fd: c_int) -> c_int;
        fn getpgrp() -> c_int;
    }
    // SAFETY: two calls that take no pointers, on file descriptor 0.
    unsafe { tcgetpgrp(0) == getpgrp() }
}

/// Writes the engine and its machine to `path` and says how big it came.
fn write_checkpoint<E: Engine>(name: &str, e: &E, path: &Path) {
    let mut w = muir::checkpoint::Writer::new();
    e.save(&mut w);
    match muir::checkpoint::write(path, name, e.machine().memory_boards(), &w.finish()) {
        Ok(n) => eprintln!(
            "checkpoint: {} at {} microcycles, {n} bytes",
            path.display(),
            e.machine().cycles
        ),
        Err(err) => eprintln!("checkpoint: could not write {}: {err}", path.display()),
    }
}

/// `muir-yyyymmdd-hhmmss.chk` in the current directory, by the wall clock.
fn timestamped(extension: &str) -> PathBuf {
    let t = local_time();
    PathBuf::from(format!(
        "muir-{:04}{:02}{:02}-{:02}{:02}{:02}.{extension}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    ))
}

/// The screen as it stands, as a PNG: [`SimpleTv::png`], which is the
/// frame buffer as the monitor shows it.
fn write_screenshot(path: &Path, tv: &muir::simpletv::SimpleTv) {
    match std::fs::write(path, tv.png()) {
        Ok(()) => eprintln!(
            "screenshot: {}, {} bytes",
            path.display(),
            std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
        ),
        Err(e) => eprintln!("screenshot: could not write {}: {e}", path.display()),
    }
}

/// Loads the checkpoint read from `path` into `e`, built and booted as the
/// flags say, or says why not and exits.
fn resume_engine<E: Engine>(name: &str, e: &mut E, (path, c): &(PathBuf, Checkpoint)) {
    if c.engine != name {
        usage(&format!(
            "--resume {}: a {} checkpoint, and this is {name}",
            path.display(),
            c.engine
        ));
    }
    let mut r = muir::checkpoint::Reader::new(&c.body);
    e.load(&mut r)
        .and_then(|()| r.done())
        .unwrap_or_else(|err| usage(&format!("--resume {}: {err}", path.display())));
    let m = e.machine();
    eprintln!(
        "resumed: {} at {} microcycles, {} ns, {} memory boards",
        path.display(),
        m.cycles,
        m.ns,
        m.memory_boards()
    );
}

/// Writes a recording to `path` and says how big it came.
fn write_capture(path: &Path, rec: &Recorder) {
    match std::fs::write(path, rec.gif()) {
        Ok(()) => eprintln!(
            "capture: {} frames of the display at {}, {} bytes",
            rec.frames(),
            path.display(),
            std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
        ),
        Err(e) => eprintln!("capture: could not write {}: {e}", path.display()),
    }
}

/// DBGIN's listener, for the debugger in the other program to connect to.
fn listen_for_debugger(addr: SocketAddr) -> std::net::TcpListener {
    let listener = match std::net::TcpListener::bind(addr) {
        Ok(l) => l,
        Err(err) => usage(&format!("--debug-cable-listen {addr}: {err}")),
    };
    eprintln!("debug cable: DBGIN listening on {}", listener.local_addr().unwrap_or(addr));
    listener
}

/// The debugger's connection: a handle to read the cable and one to write it.
fn accept_debugger(
    listener: &std::net::TcpListener,
    addr: SocketAddr,
) -> (std::net::TcpStream, std::net::TcpStream) {
    let (stream, from) = match listener.accept() {
        Ok(s) => s,
        Err(err) => usage(&format!("--debug-cable-listen {addr}: {err}")),
    };
    eprintln!("debug cable: the debugger connected from {from}");
    let reader = stream.try_clone().expect("a second handle on the cable");
    (reader, stream)
}

/// A netlist machine as `--chip` runs it: the processor with the boot PROM
/// loaded and the button pressed, its clock, the far end with the boards
/// asked for, and the interface netlist the far end's board was built from;
/// run to the first microcycle whose PC is not zero.
struct ChipMachine {
    cpu: Chip,
    clk: Behavioural,
    far: FarEnd,
    bus: netlist::Netlist,
    pc_nets: Vec<netlist::NetId>,
    promdisable: netlist::NetId,
    /// `SRUN`, `-ERRHALT` and `-STATHALT`: three of the six inputs of the
    /// 9S42 at OLORD1 1A15 that makes `MACHRUN`, which the drawing has as
    /// `MACHRUN = (SSTEP AND -SSDONE) OR (SRUN AND -ERRHALT AND -WAIT AND
    /// -STATHALT)`.  What [`machrun_low`] reads off `FLAG-1` on the other
    /// two engines is read off these nets here, and `-WAIT` is left out of
    /// it for the same reason: a machine waiting on the bus comes out of
    /// it by itself.
    srun: netlist::NetId,
    errhalt: netlist::NetId,
    stathalt: netlist::NetId,
    /// `-BOOT1`, the button, for the prompt's `boot` to press again.
    boot: netlist::NetId,
}

fn chip_machine(
    image: &[u64],
    pack: Option<&Pack>,
    boards: Boards,
    memory_boards: usize,
    mut chaos: muir::chaos::Config,
    auto_boot: bool,
) -> ChipMachine {
    let n = netlist::parse(NETLIST).unwrap();
    let mut c = Chip::new(&n);
    c.power_on();
    c.load_prom(&n, image);
    c.settle();
    let mut clk = Behavioural::new();
    let mut machine = Machine::with_memory_boards(memory_boards);
    attach(&mut machine, pack);
    if machine_chaos_wants_default(&chaos) {
        chaos.file_root = default_file_root();
    }
    machine.chaos = chaos;
    let bus_n = netlist::parse(BUSINT).unwrap();
    let mem_n = netlist::parse(CADRM).unwrap();
    let mut far = FarEnd::new(&n, &bus_n, &mem_n, boards, 0, machine);
    far.join(&mut c, clk.time_ns());

    // The button, then the few start-up microcycles before the PC moves.
    // `--no-auto-boot` leaves it unpressed, as a CADR is when the power
    // comes on, for the prompt's `boot` to press.
    let boot = n.by_name_id("-BOOT1").unwrap();
    if auto_boot {
        press_boot(&mut c, &mut clk, boot);
    }
    let pc_nets = c.bus_nets(&n, "PC", 14);
    // The mode register's bit, as `Machine::mode` has it on the other
    // engines.
    let promdisable = n.by_name_id("PROMDISABLE").unwrap();
    let srun = n.by_name_id("SRUN").unwrap();
    let errhalt = n.by_name_id("-ERRHALT").unwrap();
    let stathalt = n.by_name_id("-STATHALT").unwrap();
    let mut skipped = 0;
    while auto_boot && c.read(&pc_nets) == 0 && skipped < 40 {
        c.microcycle(&mut clk);
        skipped += 1;
    }
    ChipMachine {
        cpu: c,
        clk,
        far,
        bus: bus_n,
        pc_nets,
        promdisable,
        srun,
        errhalt,
        stathalt,
        boot,
    }
}

/// **The boot button on a netlist machine**: `-BOOT1` held down, the
/// board settled with it down, and twenty master clock cycles before it
/// comes back up.  It is all that starts a CADR, so the prompt's `boot`
/// presses this and nothing else, as it does on the other two engines.
fn press_boot(c: &mut Chip, clk: &mut Behavioural, boot: netlist::NetId) {
    c.set_net(boot, Level::Low);
    c.settle();
    for _ in 0..20 {
        c.tick(clk);
    }
    c.set_net(boot, Level::High);
}

/// One turn of the terminal for a netlist machine: the screen out and the
/// keys and the pointer in when it is time to poll, and both delivered as
/// the I/O board takes them.
fn attend_chip(
    far: &mut FarEnd,
    terminal: Option<&mut Terminal>,
    poll: bool,
    keyboard: &mut Keyboard,
    mouse: &mut Mouse,
) {
    if poll && let Some(term) = terminal {
        term.poll(Frame::of(&far.buses.machine.simpletv));
        for (keysym, down) in term.take_keys() {
            keyboard.key(keysym, down);
        }
        for (buttons, x, y) in term.take_pointers() {
            mouse.pointer(buttons, x, y);
        }
    }
    match far.unibus.as_mut().and_then(|u| u.mouse()) {
        // The netlist I/O board takes its motion as quadrature stepped
        // down the lines, and its buttons as levels.
        Some(cable) => {
            let (dx, dy) = mouse.take_motion();
            if dx != 0 || dy != 0 || cable.buttons() != mouse.buttons() {
                cable.send(dx, dy, mouse.buttons());
            }
        }
        None => {
            let board = &mut far.buses.machine.ioboard;
            if mouse.pending(board.mouse_buttons_held()) {
                mouse.deliver(board);
            }
        }
    }
    if keyboard.pending() > 0 {
        match far.unibus.as_mut().and_then(|u| u.keyboard()) {
            // The netlist I/O board takes its keys down the cable, one
            // word at a time as the cable frees.
            Some(cable) => {
                if let Some(word) = keyboard.peek()
                    && cable.send(word)
                {
                    keyboard.take();
                }
            }
            // The behavioural board under `--io-board model` takes the
            // word, and `Buses` runs its interrupt cycle against the
            // netlist bus interface --- request, grant, `SACK`, `INTR`
            // with vector 260 --- as the netlist board would run its own.
            None => {
                keyboard.deliver(&mut far.buses.machine.ioboard);
            }
        }
    }
}

/// Runs a netlist machine: the eight things the command line has to say
/// about one, the memory board count among them.
#[allow(clippy::too_many_arguments)]
fn time_chip(
    image: &[u64],
    stop: Stop,
    pack: Option<&Pack>,
    boards: Boards,
    memory_boards: usize,
    chaos: muir::chaos::Config,
    terminal: Option<&mut Terminal>,
    capture: Option<(PathBuf, bool)>,
    setup: &str,
    clocks: bool,
    hold: bool,
) {
    let ChipMachine {
        mut cpu,
        mut clk,
        mut far,
        pc_nets,
        promdisable,
        srun,
        errhalt,
        stathalt,
        boot,
        ..
    } = chip_machine(image, pack, boards, memory_boards, chaos, !hold);
    // One microcycle is however many clock transitions it takes for the phase
    // to wrap, not a fixed number of them.
    let t = Instant::now();
    let mut ran = 0;
    let mut last = clk.phase_ns();
    let prom_enabled = |c: &Chip| c.net(promdisable) != Level::High;
    // As [`machrun_low`] is on the other two engines, off the nets rather
    // than off `FLAG-1`: `Chip` is not an `Engine` and has no spy registers
    // to read, but it has the nets those registers are buffered from.
    let stopped_itself = |c: &Chip| {
        if c.net(srun) != Level::High {
            return None;
        }
        if c.net(errhalt) == Level::Low {
            return Some("ERR is up under ERRSTOP, which is what HALT-CONS does: (si:%halt)");
        }
        if c.net(stathalt) == Level::Low {
            return Some("the statistics counter ran out under STATHENB");
        }
        None
    };
    let mut terminal = terminal;
    let (mut keyboard, mut mouse) = (Keyboard::new(), Mouse::new());
    let mut last_poll = Instant::now();
    let mut capture = capture.map(|(path, time)| (path, Recorder::new(time)));
    let mut last_check = Instant::now();
    let prompt = Prompt::open();
    // The same hold the other two engines have: nothing is ticked while it
    // is on, so the netlist stands where it stopped and can be looked at.
    // `chip` is slow enough that this matters --- a run that has spent an
    // hour getting somewhere should not have to be started again to be
    // asked where it is.
    let mut held = hold;
    let mut stepping: Option<u64> = None;
    let mut quit = false;
    catch_interrupts();
    let mut interrupts_seen = 0;
    while !quit && ran < stop.after && !stop.reached(cpu.read(&pc_nets) as u16, prom_enabled(&cpu))
    {
        let mut wrapped = false;
        if held {
            std::thread::sleep(TERMINAL_INTERVAL / 4);
        } else {
            far.tick_with(&mut cpu, &mut clk);
            let p = clk.phase_ns();
            if p < last {
                ran += 1;
                wrapped = true;
                if let Some(left) = stepping.as_mut() {
                    *left -= 1;
                    if *left == 0 {
                        stepping = None;
                        held = true;
                        say_pc_chip(&cpu, &pc_nets, ran, prom_enabled(&cpu));
                    }
                }
            }
            last = p;
        }
        // The capture keeps its own cadence, in microcycles.
        if wrapped
            && ran % TERMINAL_CHECK == 0
            && let Some((_, rec)) = capture.as_mut()
        {
            rec.sample(&far.buses.machine.simpletv, clk.time_ns(), wall_clock());
        }
        // Everything else goes by the wall clock, not by a microcycle
        // count. `chip` runs about 1,800 microcycles a second, so
        // `TERMINAL_CHECK` of them is two and a half seconds --- too long
        // to wait on a typed line, and a run shorter than that never
        // reaches a check at all. `Instant::now` once a microcycle costs
        // nothing at this rate, which is the only reason the other engines
        // count microcycles instead.
        let check = held || (wrapped && last_check.elapsed() >= TERMINAL_INTERVAL);
        if !check {
            continue;
        }
        last_check = Instant::now();
        let poll = last_poll.elapsed() >= TERMINAL_INTERVAL;
        if poll || !held {
            attend_chip(&mut far, terminal.as_deref_mut(), poll, &mut keyboard, &mut mouse);
            if poll {
                last_poll = Instant::now();
            }
        }
        // The machine stopping itself, held on once rather than spun on,
        // exactly as `time_engine` does it off `FLAG-1`.
        if !held && let Some(why) = stopped_itself(&cpu) {
            held = true;
            stepping = None;
            say_machrun_low(why);
            say_pc_chip(&cpu, &pc_nets, ran, prom_enabled(&cpu));
        }
        // ^C: the first holds the machine at the prompt, one more while
        // held quits; with no prompt to go on from, one quits.
        let seen = INTERRUPTS.load(Ordering::SeqCst);
        while interrupts_seen < seen {
            interrupts_seen += 1;
            let at_prompt = prompt.as_ref().is_some_and(|p| !p.ended());
            if held || !at_prompt {
                quit = true;
            } else {
                held = true;
                stepping = None;
                if let Some(prompt) = prompt.as_ref() {
                    prompt.past_interrupt();
                }
                println!("held at ^C; continue runs on, ^C again quits");
                say_pc_chip(&cpu, &pc_nets, ran, prom_enabled(&cpu));
            }
        }
        if stepping.is_none()
            && let Some(prompt) = prompt.as_ref()
        {
            let ending = prompt.ended();
            while let Some(line) = prompt.line() {
                match muir::prompt::parse(&line) {
                    Ok(None) => {}
                    Ok(Some(Command::Boot)) => {
                        press_boot(&mut cpu, &mut clk, boot);
                        say_pc_chip(&cpu, &pc_nets, ran, prom_enabled(&cpu));
                        held = false;
                    }
                    Ok(Some(Command::Hold)) => {
                        held = true;
                        say_pc_chip(&cpu, &pc_nets, ran, prom_enabled(&cpu));
                    }
                    Ok(Some(Command::Continue)) => match stopped_itself(&cpu) {
                        Some(why) => say_machrun_low(why),
                        None => held = false,
                    },
                    Ok(Some(Command::Step(n))) => match stopped_itself(&cpu) {
                        Some(why) => say_machrun_low(why),
                        None => {
                            held = false;
                            stepping = Some(n);
                            break;
                        }
                    },
                    Ok(Some(Command::Pc)) => say_pc_chip(&cpu, &pc_nets, ran, prom_enabled(&cpu)),
                    Ok(Some(Command::Info)) => print!("{setup}"),
                    Ok(Some(Command::Screenshot(path))) => {
                        let path = path.unwrap_or_else(|| timestamped("png"));
                        write_screenshot(&path, &far.buses.machine.simpletv);
                    }
                    Ok(Some(Command::StartCapture(path))) => match capture.as_ref() {
                        Some((going, _)) => println!(
                            "capture: one is going already, to {}; endcapture closes it",
                            going.display()
                        ),
                        None => {
                            let path = path.unwrap_or_else(|| timestamped("gif"));
                            println!(
                                "capture: recording the display to {}{}; endcapture writes it, and so does the stop",
                                path.display(),
                                if clocks { "" } else { ", no clocks" }
                            );
                            capture = Some((path, Recorder::new(clocks)));
                        }
                    },
                    Ok(Some(Command::EndCapture)) => match capture.take() {
                        Some((path, mut rec)) => {
                            rec.sample(&far.buses.machine.simpletv, clk.time_ns(), wall_clock());
                            write_capture(&path, &rec);
                        }
                        None => println!("capture: none is going; startcapture begins one"),
                    },
                    // The scratchpads live in the RAM chips' own cells
                    // here, not in arrays, and reading one back means
                    // walking those cells and putting the board's word
                    // order right. Until that is written these say so
                    // rather than printing something that is not the
                    // machine's.
                    Ok(Some(Command::Registers | Command::Dump { .. })) => {
                        println!("prompt: not on chip yet --- the registers and the scratchpads");
                        println!("        are the parts' own cells here, not arrays to read off");
                    }
                    Ok(Some(Command::Checkpoint(_))) => {
                        println!("prompt: not on chip yet --- checkpoints are the other engines'");
                    }
                    Ok(Some(Command::Quit)) => {
                        quit = true;
                        break;
                    }
                    Ok(Some(Command::Help)) => print!("{}", muir::prompt::HELP),
                    Err(what) => println!("prompt: {what}"),
                }
            }
            if held && ending && !quit {
                println!("held, and stdin has ended: there is nothing to run the machine on");
                quit = true;
            }
            if held && !quit && stepping.is_none() {
                prompt.show();
            }
        }
    }
    if let Some(prompt) = prompt.as_ref() {
        prompt.done();
    }
    report("chip", ran, t.elapsed().as_secs_f64());
    if quit {
        let prom = if prom_enabled(&cpu) { " in the PROM" } else { "" };
        println!("       quit at PC {:o}{prom} after {ran}", cpu.read(&pc_nets) as u16);
    } else {
        stop.conclude(ran, cpu.read(&pc_nets) as u16, prom_enabled(&cpu), None);
    }
    if let Some((path, rec)) = capture.as_mut() {
        rec.sample(&far.buses.machine.simpletv, clk.time_ns(), wall_clock());
        write_capture(path, rec);
    }
    if let Some(term) = terminal {
        serve_last_screen(term, &far.buses.machine.simpletv);
    }
}

/// The debuggee's end of the cable over TCP on the netlist: the board's
/// DBGIN answering the debugger in the other program, run in step with it
/// by [`Remote::step`], with the terminal and the stops as for `--chip`
/// alone; at the end the two agree to stop.
fn time_chip_debuggee(
    mut remote: Remote<DebugIn>,
    stop: Stop,
    pc_nets: Vec<netlist::NetId>,
    promdisable: netlist::NetId,
    terminal: Option<&mut Terminal>,
) {
    let t = Instant::now();
    let prom_enabled = |c: &Chip| c.net(promdisable) != Level::High;
    let mut terminal = terminal;
    let (mut keyboard, mut mouse) = (Keyboard::new(), Mouse::new());
    let mut last_poll = Instant::now();
    let mut checked = 0;
    catch_interrupts();
    let mut interrupts_seen = 0;
    loop {
        let m = &remote.machine;
        if interrupted(&mut interrupts_seen) {
            break;
        }
        if m.microcycles >= stop.after
            || stop.reached(m.cpu.read(&pc_nets) as u16, prom_enabled(&m.cpu))
        {
            break;
        }
        if let Err(e) = remote.step() {
            eprintln!("muir: the debug cable: {e}");
            break;
        }
        let ran = remote.machine.microcycles;
        if ran / TERMINAL_CHECK != checked {
            checked = ran / TERMINAL_CHECK;
            let poll = last_poll.elapsed() >= TERMINAL_INTERVAL;
            let m = &mut remote.machine;
            attend_chip(&mut m.far, terminal.as_deref_mut(), poll, &mut keyboard, &mut mouse);
            if poll {
                last_poll = Instant::now();
            }
        }
    }
    let m = &remote.machine;
    let ran = m.microcycles;
    report("chip, debuggee", ran, t.elapsed().as_secs_f64());
    stop.conclude(ran, m.cpu.read(&pc_nets) as u16, prom_enabled(&m.cpu), None);
    println!("       {} debug cycles on the cable", m.debug_cycles);
    if let Err(e) = remote.finish() {
        eprintln!("muir: the debug cable at the end: {e}");
    }
    if let Some(term) = terminal {
        serve_last_screen(term, &remote.machine.far.buses.machine.simpletv);
    }
}

fn main() {
    let mut which: Option<Which> = None;
    let mut pack: Option<Pack> = None;
    let mut chaos = muir::chaos::Config::default();
    let mut cycles: Option<u64> = None;
    let mut auto_boot = true;
    let mut checkpoint: Option<PathBuf> = None;
    let mut prom_file: Option<PathBuf> = None;
    let mut resume: Option<PathBuf> = None;
    let mut stop_at: Option<u16> = None;
    let mut stop_at_prom: Option<u16> = None;
    let mut boards: usize = 32;
    let mut boards_given = false;
    let mut main_memory_model = false;
    let mut io = true;
    let mut tv = true;
    let mut tv_board = TvBoard::SimpleTv;
    let mut disk_controller = false;
    // A terminal is served whether or not it is asked for: the display,
    // the keyboard and the mouse are the machine's only way in and out.
    let mut listen = TerminalAt::default_display();
    let mut debuggee = false;
    let mut debuggee_pack: Option<Pack> = None;
    // The other machine's Chaosnet, which is its own: its ether, its own
    // server on it. Unset, it is the debugger's addresses over again, and
    // a server with no FILE service.
    let mut debuggee_address: Option<u16> = None;
    let mut debuggee_server_address: Option<u16> = None;
    let mut debuggee_file_root: Option<PathBuf> = None;
    // Absent, or present with or without an endpoint.
    let mut debuggee_terminal: Option<Option<String>> = None;
    let mut cable_listen: Option<SocketAddr> = None;
    let mut cable_connect: Option<SocketAddr> = None;
    let mut capture_tv: Option<PathBuf> = None;
    let mut capture_tv_time = true;

    // The flags in `~/.muirrc` come first, so that a flag on the command
    // line, which is read after, has the last word.  [`muirrc`] drops the
    // ones the command line gives too, for the few that may not be given
    // twice.
    let typed: Vec<String> = std::env::args().skip(1).collect();
    let (from_file, rc) = muirrc(&typed);
    let words: Vec<String> = from_file.iter().chain(&typed).cloned().collect();
    // Kept to say afterwards which flags the chosen engine has no use for.
    let given = words.clone();
    let mut args = words.into_iter().peekable();
    while let Some(a) = args.next() {
        if a == "-h" || a == "--help" {
            help();
        }
        if a == "-V" || a == "--version" {
            println!("{}", version());
            std::process::exit(0);
        }
        let engine = match a.as_str() {
            "--micro" => Some(Which::Micro),
            "--rtl" => Some(Which::Rtl),
            "--chip" => Some(Which::Chip),
            _ => None,
        };
        match (engine, a.as_str()) {
            (Some(e), _) => match which {
                Some(had) if had != e => usage("--micro, --rtl and --chip are exclusive"),
                _ => which = Some(e),
            },
            (None, "--disk-pack") => {
                if pack.is_some() {
                    usage(
                        "--disk-pack twice: the disk multiplexor is not modelled yet; one pack, as unit 0",
                    );
                }
                pack = Some(pack_flag("--disk-pack", args.next()));
            }
            (None, "--chaos-address") => {
                // "<this>" or "<this>,<server>": this machine's
                // address, always, and the Chaosnet server's after a
                // comma. Each in octal or subnet:host.
                let want = "--chaos-address wants <this>[,<server>], each an address in octal or subnet:host";
                let arg = args.next().unwrap_or_else(|| usage(want));
                let mut halves = arg.splitn(2, ',');
                match halves.next().and_then(muir::chaos::parse_address) {
                    Some(a) => chaos.address = a,
                    None => usage(want),
                }
                if let Some(associated) = halves.next() {
                    match muir::chaos::parse_address(associated) {
                        Some(a) => chaos.server_address = a,
                        None => usage(want),
                    }
                }
            }
            (None, "--chaos-trace") => chaos.trace = true,
            (None, "--chaos-file-root") => match args.next() {
                Some(d) => chaos.file_root = Some(file_root("--chaos-file-root", &d)),
                None => usage("--chaos-file-root wants a directory"),
            },
            (None, "--main-memory") => match args.next().as_deref() {
                Some("netlist") => main_memory_model = false,
                Some("model") => main_memory_model = true,
                _ => usage("--main-memory wants netlist or model"),
            },
            // Main memory on every engine, and the backplane on chip: from
            // one board to the sixty the Xbus I/O space leaves room for
            // (`busint::MAX_MEMORY_BOARDS`). Zero is no memory; the model
            // memory on chip is `--main-memory model`.
            (None, "--main-memory-boards") => match args.next().and_then(|v| v.parse().ok()) {
                Some(b) if (1..=muir::busint::MAX_MEMORY_BOARDS).contains(&b) => {
                    boards = b;
                    boards_given = true;
                }
                _ => usage(&format!(
                    "--main-memory-boards wants a count from 1 to {}",
                    muir::busint::MAX_MEMORY_BOARDS
                )),
            },
            (None, "--tv") => match args.next().as_deref() {
                Some("netlist") => tv = true,
                Some("model") => tv = false,
                _ => usage("--tv wants netlist or model"),
            },
            (None, "--tv-board") => match args.next().as_deref() {
                Some("simple-tv") => tv_board = TvBoard::SimpleTv,
                Some("lispm-tv") => tv_board = TvBoard::LispmTv,
                _ => usage("--tv-board wants simple-tv or lispm-tv"),
            },
            (None, "--disk-controller") => match args.next().as_deref() {
                Some("netlist") => disk_controller = true,
                Some("model") => disk_controller = false,
                _ => usage("--disk-controller wants netlist or model"),
            },
            (None, "--io-board") => match args.next().as_deref() {
                Some("netlist") => io = true,
                Some("model") => io = false,
                _ => usage("--io-board wants netlist or model"),
            },
            (None, "--terminal") => {
                // The endpoint is optional: the next word is it unless it is a flag.
                let spec = args.next_if(|v| !v.starts_with('-'));
                match endpoint(spec.as_deref(), TERMINAL_PORT) {
                    Some(addr) => {
                        listen = TerminalAt {
                            addr,
                            port_named: names_a_port(spec.as_deref()),
                            asked: true,
                        };
                    }
                    None => usage("--terminal wants nothing, a port, an address or address:port"),
                }
            }
            (None, "--debug-in-process") => debuggee = true,
            (None, "--debuggee-chaos-address") => {
                let want = "--debuggee-chaos-address wants <this>[,<server>], each an address in octal or subnet:host";
                let arg = args.next().unwrap_or_else(|| usage(want));
                let mut halves = arg.splitn(2, ',');
                match halves.next().and_then(muir::chaos::parse_address) {
                    Some(a) => debuggee_address = Some(a),
                    None => usage(want),
                }
                if let Some(server) = halves.next() {
                    match muir::chaos::parse_address(server) {
                        Some(a) => debuggee_server_address = Some(a),
                        None => usage(want),
                    }
                }
            }
            (None, "--debuggee-chaos-file-root") => match args.next() {
                Some(d) => {
                    debuggee_file_root = Some(file_root("--debuggee-chaos-file-root", &d));
                }
                None => usage("--debuggee-chaos-file-root wants a directory"),
            },
            (None, "--debuggee-disk-pack") => {
                debuggee_pack = Some(pack_flag("--debuggee-disk-pack", args.next()));
            }
            (None, "--debuggee-terminal") => {
                debuggee_terminal = Some(args.next_if(|v| !v.starts_with('-')));
            }
            (None, "--debug-cable-listen") => {
                // The endpoint is optional: the next word is it unless it is a flag.
                let spec = args.next_if(|v| !v.starts_with('-'));
                match endpoint(spec.as_deref(), DEBUG_CABLE_PORT) {
                    Some(a) => cable_listen = Some(a),
                    None => usage(
                        "--debug-cable-listen wants nothing, a port, an address or address:port",
                    ),
                }
            }
            (None, "--debug-cable-connect") => {
                // The endpoint is optional: the next word is it unless it is a flag.
                let spec = args.next_if(|v| !v.starts_with('-'));
                match endpoint(spec.as_deref(), DEBUG_CABLE_PORT) {
                    Some(a) => cable_connect = Some(a),
                    None => usage(
                        "--debug-cable-connect wants nothing, a port, an address or address:port",
                    ),
                }
            }
            (None, "--tv-capture") => match args.next() {
                Some(path) => capture_tv = Some(PathBuf::from(path)),
                None => usage("--tv-capture wants a file for the GIF"),
            },
            (None, "--tv-capture-no-time") => capture_tv_time = false,
            (None, "--checkpoint") => match args.next() {
                Some(path) => checkpoint = Some(PathBuf::from(path)),
                None => usage("--checkpoint wants a file to write"),
            },
            // Read before the loop, by `muirrc`, since the file it names is
            // where the loop's first words come from.
            (None, "-c" | "--config") => {
                args.next();
            }
            (None, "--no-auto-boot") => auto_boot = false,
            (None, "--prom") => match args.next() {
                Some(path) => prom_file = Some(PathBuf::from(path)),
                None => usage("--prom wants an MCR microcode file"),
            },
            (None, "--resume") => match args.next() {
                Some(path) => resume = Some(PathBuf::from(path)),
                None => usage("--resume wants a checkpoint to start from"),
            },
            (None, "--stop-after") => match args.next().and_then(|v| v.parse().ok()) {
                Some(n) => cycles = Some(n),
                None => usage("--stop-after wants a count of microcycles"),
            },
            (None, "--stop-at") => {
                match args.next().and_then(|v| u16::from_str_radix(&v, 8).ok()) {
                    Some(pc) if pc < 1 << 14 => stop_at = Some(pc),
                    _ => usage("--stop-at wants a PC in octal, below 40000"),
                }
            }
            (None, "--stop-at-prom") => {
                match args.next().and_then(|v| u16::from_str_radix(&v, 8).ok()) {
                    Some(pc) if pc < 512 => stop_at_prom = Some(pc),
                    _ => usage("--stop-at-prom wants a PC in octal, below 1000"),
                }
            }
            (None, v) => usage(&format!("unknown argument {v}")),
        }
    }

    let which = which.unwrap_or(Which::Rtl);
    let cabled = debuggee as u8 + cable_listen.is_some() as u8 + cable_connect.is_some() as u8;
    if cabled > 1 {
        usage(
            "one of --debug-in-process, --debug-cable-listen and --debug-cable-connect: one lashup at a time",
        );
    }
    if cabled == 1 {
        match which {
            Which::Micro => usage("micro has no timing model, and no end of the debug cable"),
            Which::Chip if cable_listen.is_none() => {
                usage("on chip the debug cable is the board's DBGIN only: --debug-cable-listen")
            }
            _ => {}
        }
    }
    if (debuggee_address.is_some() || debuggee_server_address.is_some()) && !debuggee {
        usage(
            "--debuggee-chaos-address is the other machine's Chaosnet: it needs --debug-in-process",
        );
    }
    if debuggee_file_root.is_some() && !debuggee {
        usage(
            "--debuggee-chaos-file-root is the other machine's file service: it needs --debug-in-process",
        );
    }
    if debuggee_pack.is_some() && !debuggee {
        usage("--debuggee-disk-pack is the other machine's pack: it needs --debug-in-process");
    }
    // The other machine's display: it takes another machine, and it is not
    // this machine's endpoint. Both are settled here, before anything is
    // bound, so that what is refused is refused whatever the host has
    // going on at the port.
    if let Some(spec) = debuggee_terminal.as_ref() {
        if !debuggee {
            usage(
                "--debuggee-terminal is the other machine's display: it needs --debug-in-process",
            );
        }
        // Port 0 is the host's choice, and two of them are never the same
        // port.
        if debuggee_endpoint(spec.as_deref(), listen.addr) == listen.addr && listen.addr.port() != 0
        {
            usage("--debuggee-terminal: the same endpoint as --terminal");
        }
    }
    if capture_tv.is_some() && (cable_listen.is_some() || cable_connect.is_some()) {
        usage(
            "--tv-capture records a machine on its own or the lashup in one process, not an end of the debug cable over TCP",
        );
    }
    // The prompt's `startcapture` makes a recorder too, so the flag means
    // something on the engines that have one even with no --tv-capture.
    if capture_tv.is_none() && !capture_tv_time && (which == Which::Chip || cabled == 1) {
        usage("--tv-capture-no-time only means anything with --tv-capture");
    }
    let capture = capture_tv.map(|path| (path, capture_tv_time));
    if !auto_boot {
        if cabled == 1 {
            usage("--no-auto-boot is one machine on its own, not the lashup");
        }
        if !Prompt::possible() {
            usage(
                "--no-auto-boot has nothing to press the button: stdin is a terminal muir is in the background of",
            );
        }
    }
    // A checkpoint holds the whole machine, the PROM's 512 words with it,
    // so resuming one loads the PROM it ran and `--prom` would be a second
    // answer the checkpoint quietly overruled.
    if prom_file.is_some() && resume.is_some() {
        usage("--prom and --resume: the checkpoint carries the PROM it ran");
    }
    if checkpoint.is_some() || resume.is_some() {
        if which == Which::Chip {
            usage("--checkpoint and --resume are micro and rtl for now, not chip");
        }
        if cabled == 1 {
            usage("--checkpoint and --resume are one machine on its own, not the lashup");
        }
    }
    // A checkpoint is read before the machine is built, so that the machine
    // can be built with as much memory as the checkpoint's had.
    let resume = resume.map(|path| {
        let c = muir::checkpoint::read(&path)
            .unwrap_or_else(|err| usage(&format!("--resume {}: {err}", path.display())));
        (path, c)
    });
    if let Some((path, c)) = &resume {
        if boards_given && boards != c.memory_boards {
            usage(&format!(
                "--resume {}: the checkpoint has {} memory boards, --main-memory-boards {boards}",
                path.display(),
                c.memory_boards
            ));
        }
        boards = c.memory_boards;
    }
    // The behavioural memory answers the interface's cycles and no other
    // master's; the netlist controller's DMA needs memory boards.
    if disk_controller && main_memory_model {
        usage(
            "--disk-controller netlist needs --main-memory netlist: the model memory does not answer a second master",
        );
    }
    // The run goes on until a stop, a halt or ^C unless a window was asked for.
    let window = cycles.unwrap_or(u64::MAX);
    let stop = Stop { after: window, at: stop_at, at_prom: stop_at_prom };
    let pack = pack.as_ref();
    // The backplane's netlist boards; the model memory, `main`, is `boards`
    // long on every engine.
    let netlist_boards = if main_memory_model { 0 } else { boards };

    // A terminal that was asked for and cannot be served stops the run;
    // the display nobody asked for, when there is no free one or the host
    // will not have a listener at all, leaves the run without a terminal
    // and the start says why.
    let (mut terminal, no_terminal) = match bind_terminal(listen) {
        Ok(t) => (Some(t), None),
        Err(e) if listen.asked => usage(&format!("--terminal {e}")),
        Err(e) => (None, Some(e)),
    };
    // The other machine's display: the one above this machine's, or where
    // the flag says. In the lashup both machines are served, neither
    // being workable without a terminal.
    let debuggee_listen = debuggee.then(|| {
        let base = terminal.as_ref().and_then(|t| t.addr().ok()).unwrap_or(listen.addr);
        let spec = debuggee_terminal.clone().flatten();
        TerminalAt {
            addr: debuggee_endpoint(spec.as_deref(), base),
            port_named: names_a_port(spec.as_deref()),
            asked: debuggee_terminal.is_some(),
        }
    });
    let (mut debuggee_terminal, no_debuggee_terminal) = match debuggee_listen {
        None => (None, None),
        Some(at) => match bind_terminal(at) {
            Ok(t) => (Some(t), None),
            Err(e) if at.asked => usage(&format!("--debuggee-terminal {e}")),
            Err(e) => (None, Some(e)),
        },
    };
    // The Chaosnet server's file root, on the engines with a Chaosnet.
    if which != Which::Micro && machine_chaos_wants_default(&chaos) {
        chaos.file_root = default_file_root();
    }
    // The other machine's Chaosnet: its own ether with its own server on
    // it, since muir's cable carries one machine.  The addresses are the
    // debugger's over again unless told otherwise --- the two cables never
    // meet, so there is nothing for them to collide with --- and the file
    // root is only what is named for it: two servers rooted at one
    // directory are two hosts sharing a filesystem, with nothing between
    // them to keep one from writing what the other is reading.
    let debuggee_chaos = muir::chaos::Config {
        address: debuggee_address.unwrap_or(chaos.address),
        server_address: debuggee_server_address.unwrap_or(chaos.server_address),
        file_root: debuggee_file_root,
        ..chaos.clone()
    };

    // The boot PROM, before the setup: the setup says which one it is.
    let prom = boot_prom(prom_file.as_deref());

    // What this run is: said once here, and again by the prompt's `info`.
    let setup = {
        use std::fmt::Write;
        let mut s = String::new();
        let engine = match which {
            Which::Micro => "micro",
            Which::Rtl => "rtl",
            Which::Chip => "chip",
        };
        writeln!(s, "{} started", version()).unwrap();
        // A flag the chosen engine has no use for is not an error --- one
        // `.muirrc` serves runs of every engine --- but a run that quietly
        // ignored it would look as though it had obeyed.
        for (flag, engines, has) in [
            ("--disk-controller", "chip", which == Which::Chip),
            ("--io-board", "chip", which == Which::Chip),
            ("--main-memory", "chip", which == Which::Chip),
            ("--tv", "chip", which == Which::Chip),
            ("--tv-board", "chip", which == Which::Chip),
            ("--chaos-address", "rtl and chip", which != Which::Micro),
            ("--chaos-file-root", "rtl and chip", which != Which::Micro),
            ("--chaos-trace", "rtl and chip", which != Which::Micro),
        ] {
            if !has && given.iter().any(|w| w == flag) {
                writeln!(s, "warning: {flag} is {engines}, and this run is {engine}: ignored")
                    .unwrap();
            }
        }
        if let Some(path) = &rc
            && !from_file.is_empty()
        {
            writeln!(s, "flags: {}, from {}", from_file.join(" "), shown(path)).unwrap();
        }
        writeln!(s, "engine: {engine}").unwrap();
        writeln!(s, "prom: {}", prom_shown(prom_file.as_deref(), &prom)).unwrap();
        let memory_kind = match which {
            Which::Chip if main_memory_model => ", model",
            Which::Chip => ", netlist boards on the Xbus",
            _ => "",
        };
        writeln!(s, "memory: {boards} boards, {}{memory_kind}", memory_size(boards)).unwrap();
        if which == Which::Chip {
            let kind = |netlist: bool| if netlist { "netlist" } else { "model" };
            let tv_kind =
                if matches!(tv_board, TvBoard::SimpleTv) { "simple-tv" } else { "lispm-tv" };
            writeln!(
                s,
                "boards: I/O board {}, TV {} {tv_kind}, disk controller {}",
                kind(io),
                kind(tv),
                kind(disk_controller)
            )
            .unwrap();
        }
        match pack_choice(pack) {
            Some((p, unit, ro)) => writeln!(
                s,
                "pack: {} in unit {unit}{}",
                shown(&p),
                if ro {
                    ", the read-only switch on: nothing reaches the file"
                } else {
                    ", written as the machine writes it"
                }
            )
            .unwrap(),
            None => {
                writeln!(s, "pack: none; the boot waits on a drive that never answers").unwrap()
            }
        }
        if which != Which::Micro {
            let root = match &chaos.file_root {
                Some(r) => format!("file root {}", shown(r)),
                None => format!(
                    "no file root, so STATUS, TIME and UPTIME but no FILE (--chaos-file-root, or make {VENDORED_FILE_ROOT})"
                ),
            };
            writeln!(
                s,
                "chaosnet: {:o}, the server at {:o}, {root}",
                chaos.address, chaos.server_address
            )
            .unwrap();
        }
        writeln!(s, "terminal: {}", terminal_line(&terminal, &no_terminal, listen.addr)).unwrap();
        if debuggee {
            let pack = match debuggee_pack.as_ref() {
                Some(p) => format!("pack {}", shown(&p.path)),
                None => "no pack, so its boot waits on the drive".to_string(),
            };
            writeln!(s, "debuggee: in this process, {pack}").unwrap();
            let root = match &debuggee_chaos.file_root {
                Some(r) => format!("file root {}", shown(r)),
                None => "no file root, so STATUS, TIME and UPTIME but no FILE".to_string(),
            };
            writeln!(
                s,
                "debuggee chaosnet: {:o}, its own server at {:o}, {root}",
                debuggee_chaos.address, debuggee_chaos.server_address
            )
            .unwrap();
            if let Some(at) = debuggee_listen {
                writeln!(
                    s,
                    "debuggee terminal: {}",
                    terminal_line(&debuggee_terminal, &no_debuggee_terminal, at.addr)
                )
                .unwrap();
            }
        } else if let Some(a) = cable_listen {
            writeln!(s, "debug cable: this machine the debuggee, DBGIN listening at {a}").unwrap();
        } else if let Some(a) = cable_connect {
            writeln!(s, "debug cable: this machine the debugger, DBGOUT connecting to {a}")
                .unwrap();
        }
        let clocks = if capture_tv_time { "" } else { ", no clocks" };
        if let Some((p, _)) = &capture {
            writeln!(s, "capture: {}{clocks}", p.display()).unwrap();
        }
        if let Some(p) = &checkpoint {
            writeln!(s, "checkpoint: {} at the stop", p.display()).unwrap();
        }
        if let Some((p, c)) = &resume {
            writeln!(s, "resume: {}, {} with {} boards", p.display(), c.engine, c.memory_boards)
                .unwrap();
        }
        let mut stops = Vec::new();
        if let Some(n) = cycles {
            stops.push(format!("after {n} microcycles"));
        }
        if let Some(pc) = stop_at {
            stops.push(format!("at PC {pc:o}"));
        }
        if let Some(pc) = stop_at_prom {
            stops.push(format!("at PC {pc:o} in the PROM"));
        }
        if stops.is_empty() {
            writeln!(s, "stop: none; a halt or ^C").unwrap();
        } else {
            writeln!(s, "stop: {}", stops.join(", ")).unwrap();
        }
        if !auto_boot {
            writeln!(
                s,
                "start: held, and the boot button not pressed; boot at the prompt presses it"
            )
            .unwrap();
        }
        if cabled == 0 && which != Which::Chip {
            if Prompt::possible() {
                writeln!(s, "^C holds the machine at the prompt; help lists muir's commands")
                    .unwrap();
            } else {
                writeln!(s, "prompt: none; stdin is a terminal muir is in the background of")
                    .unwrap();
            }
        }
        s
    };
    eprint!("{setup}");

    match which {
        Which::Micro => {
            let mut e = Micro::new(machine(&prom, pack, boards));
            if auto_boot {
                e.boot();
            }
            if let Some(p) = &resume {
                resume_engine("micro", &mut e, p);
            }
            let run = Run {
                stop,
                capture,
                checkpoint,
                setup: &setup,
                hold: !auto_boot,
                clocks: capture_tv_time,
            };
            time_engine("micro", e, terminal.as_mut(), run);
        }
        Which::Rtl => {
            let mut m = machine(&prom, pack, boards);
            // The Chaosnet, as under chip: the interface on the I/O board
            // and the Chaosnet server on its cable.
            if machine_chaos_wants_default(&chaos) {
                chaos.file_root = default_file_root();
            }
            m.chaos = chaos.clone();
            m.plug_chaos(0);
            let mut e = Rtl::new(m);
            if auto_boot {
                e.boot();
            }
            if debuggee {
                // The other machine's pack is only the one named: CC's
                // debuggee usually has none --- CC loads it over the cable.
                let mut mb = Machine::with_memory_boards(boards);
                mb.load_prom(&prom);
                if let Some(p) = debuggee_pack.as_ref() {
                    attach(&mut mb, Some(p));
                }
                // Its own Chaosnet, on a cable of its own: the two
                // machines cannot hear each other over it, and the only
                // wire between them is the debug cable.
                mb.chaos = debuggee_chaos.clone();
                mb.plug_chaos(0);
                let mut b = Rtl::new(mb);
                b.boot();
                time_lashup(
                    Lashup::new(e, b),
                    stop,
                    terminal.as_mut(),
                    debuggee_terminal.as_mut(),
                    capture,
                );
            } else if let Some(addr) = cable_listen {
                let listener = listen_for_debugger(addr);
                let (reader, stream) = accept_debugger(&listener, addr);
                time_remote(
                    "rtl, debuggee",
                    Remote::debuggee(e, reader, stream),
                    stop,
                    terminal.as_mut(),
                );
            } else if let Some(addr) = cable_connect {
                // The debuggee may still be starting: try for five seconds.
                let mut tries = 0;
                let stream = loop {
                    match std::net::TcpStream::connect(addr) {
                        Ok(s) => break s,
                        Err(err)
                            if tries < 50
                                && err.kind() == std::io::ErrorKind::ConnectionRefused =>
                        {
                            tries += 1;
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        Err(err) => usage(&format!("--debug-cable-connect {addr}: {err}")),
                    }
                };
                eprintln!("debug cable: DBGOUT connected to the debuggee at {addr}");
                let reader = stream.try_clone().expect("a second handle on the cable");
                time_remote(
                    "rtl, debugger",
                    Remote::debugger(e, reader, stream),
                    stop,
                    terminal.as_mut(),
                );
            } else {
                if let Some(p) = &resume {
                    resume_engine("rtl", &mut e, p);
                }
                let run = Run {
                    stop,
                    capture,
                    checkpoint,
                    setup: &setup,
                    hold: !auto_boot,
                    clocks: capture_tv_time,
                };
                time_engine("rtl", e, terminal.as_mut(), run);
            }
        }
        Which::Chip => {
            let image: Vec<u64> = prom.iter().copied().map(muir::prom::programming).collect();
            let io_n = io.then(|| netlist::parse(CADRIO).unwrap());
            let tv_n = tv.then(|| {
                netlist::parse(match tv_board {
                    TvBoard::SimpleTv => SIMPLETV,
                    TvBoard::LispmTv => LISPMTV,
                })
                .unwrap()
            });
            let disk_n = disk_controller.then(|| netlist::parse(CADRDC).unwrap());
            let on_the_buses = Boards {
                memory: netlist_boards,
                io: io_n.as_ref(),
                tv: tv_n.as_ref(),
                disk: disk_n.as_ref(),
            };
            if let Some(addr) = cable_listen {
                // The port first, so that the debugger's connect finds it
                // while the netlists are built.
                let listener = listen_for_debugger(addr);
                // The debuggee's stops are the debugger's to notice over
                // the cable, which is what CC is for, so the self-halt
                // check `time_chip` makes is not made here.
                let ChipMachine { cpu, clk, far, bus, pc_nets, promdisable, .. } =
                    // The debuggee's button is the debugger's to press over
                    // the cable, so this end always boots itself.
                    chip_machine(&image, pack, on_the_buses, boards, chaos, true);
                let (reader, stream) = accept_debugger(&listener, addr);
                let end = DebugIn::new(&bus, cpu, clk, far);
                let remote = Remote::debuggee(end, reader, stream);
                time_chip_debuggee(remote, stop, pc_nets, promdisable, terminal.as_mut());
            } else {
                time_chip(
                    &image,
                    stop,
                    pack,
                    on_the_buses,
                    boards,
                    chaos,
                    terminal.as_mut(),
                    capture,
                    &setup,
                    capture_tv_time,
                    !auto_boot,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A machine that stopped itself is seen in `FLAG-1`, never in
    /// `step`.** `HALT-CONS` under `ERRSTOP` --- misc function 1, which is
    /// what System 100's `(si:%halt)` runs --- leaves `RUN` set and `ERR`
    /// up, and every `step` after it returns `Ok` having run no
    /// microcycle. So the run loop cannot learn this from stepping, and
    /// [`machrun_low`] is what it reads instead.
    ///
    /// `tests/halt.rs` holds both engines to the halt itself; this holds
    /// the run loop's reading of it.
    #[test]
    fn a_machine_that_stopped_itself_reads_low_on_machrun() {
        // `HALT-CONS`, `1_10.` in `sys/sys/cadsym.lisp`: `IR<11:10>` = 1.
        const HALT_CONS: u64 = 1 << 10;
        let halting = || {
            let mut m = Machine::new();
            let mut prom = vec![muir::isa::asm::filler(); 512];
            prom[5] = Insn::new(muir::isa::asm::filler().raw() | HALT_CONS);
            m.load_prom(&prom);
            m
        };

        // Running, with nothing to report.
        let mut e = Rtl::new(halting());
        e.boot();
        assert_eq!(machrun_low(&e), None, "just booted and running");

        // `ERRSTOP` is set after the boot, which resets the console's
        // registers. Forty microcycles is well past the halt at 5.
        e.machine_mut().mode.errstop = true;
        for _ in 0..40 {
            e.step().expect("no halt this engine raises");
        }
        let why = machrun_low(&e).expect("stopped by HALT-CONS under ERRSTOP");
        assert!(why.contains("ERRSTOP"), "and says why: {why}");

        // Without `ERRSTOP` the same program runs straight through it, so
        // there is nothing for the run loop to hold on.
        let mut e = Rtl::new(halting());
        e.boot();
        for _ in 0..40 {
            e.step().expect("no halt this engine raises");
        }
        assert_eq!(machrun_low(&e), None, "HALT-CONS without ERRSTOP runs on");

        // And the same on `micro`, which the run loop treats alike.
        let mut e = Micro::new(halting());
        e.boot();
        e.machine_mut().mode.errstop = true;
        for _ in 0..40 {
            e.step().expect("no halt this engine raises");
        }
        assert!(machrun_low(&e).is_some(), "micro stops the same way");
    }

    /// **The wall clock reads the C library's `struct tm` where the hours,
    /// minutes and seconds are.** It is a time of day, and it stands from
    /// UTC's by a time zone's offset: a whole number of quarter hours,
    /// within fourteen hours either way.
    #[test]
    fn the_wall_clock_is_the_local_time_of_day() {
        let utc =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
                % 86_400;
        let local = wall_clock() / 1_000_000_000;
        assert!(local < 86_400, "a time of day: {local}");
        let offset = (local as i64 - utc as i64).rem_euclid(86_400);
        let offset = if offset > 43_200 { offset - 86_400 } else { offset };
        assert!(offset.abs() <= 14 * 3600, "within a zone's reach: {offset} s");
        assert_eq!(offset.rem_euclid(900), 0, "a whole number of quarter hours: {offset} s");
    }

    #[test]
    fn a_pack_is_an_image_then_its_unit_and_ro_in_either_order() {
        let pack = |unit, read_only| Pack { path: PathBuf::from("a.img"), unit, read_only };
        assert_eq!(pack_spec("a.img"), Ok(pack(0, false)));
        assert_eq!(pack_spec("a.img,ro"), Ok(pack(0, true)));
        assert_eq!(pack_spec("a.img,rw"), Ok(pack(0, false)));
        assert_eq!(pack_spec("a.img,3"), Ok(pack(3, false)));
        assert_eq!(pack_spec("a.img,3,ro"), Ok(pack(3, true)));
        assert_eq!(pack_spec("a.img,ro,3"), Ok(pack(3, true)));
        assert!(pack_spec("").is_err());
        assert!(pack_spec(",ro").is_err());
        assert!(pack_spec("a.img,ro,ro").is_err());
        assert!(pack_spec("a.img,ro,rw").is_err());
        assert!(pack_spec("a.img,1,2").is_err());
        assert!(pack_spec("a.img,8").is_err());
        assert!(pack_spec("a.img,x").is_err());
        assert!(pack_spec("a.img,").is_err());
    }

    /// **A named port is bound as it stands and an unnamed one is where
    /// the search for a free display starts**, so which is which is what
    /// tells a refusal from a display one up.
    #[test]
    fn an_endpoint_names_a_port_or_leaves_it_to_the_default() {
        assert!(!names_a_port(None));
        assert!(!names_a_port(Some("0.0.0.0")));
        assert!(!names_a_port(Some("::1")));
        assert!(names_a_port(Some("5901")));
        assert!(names_a_port(Some("0")));
        assert!(names_a_port(Some("127.0.0.1:5901")));
        assert!(names_a_port(Some("[::1]:5900")));
    }

    /// **A display that is taken moves an unnamed port up and refuses a
    /// named one.** The port taken here is one the host picked and is
    /// held for the whole test, so nothing else on the machine is in the
    /// way of it.
    #[test]
    fn a_taken_display_moves_up_unless_its_port_was_named() {
        let held = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = held.local_addr().unwrap();
        if addr.port() == u16::MAX {
            return;
        }
        let named = TerminalAt { addr, port_named: true, asked: true };
        assert!(bind_terminal(named).is_err(), "a named port that is taken is not moved");
        let unnamed = TerminalAt { addr, port_named: false, asked: false };
        match bind_terminal(unnamed) {
            // The next display up, or one above that if the host had it
            // taken too.
            Ok(t) => assert!(t.addr().unwrap().port() > addr.port(), "the next free display"),
            Err(e) => panic!("no free display above {addr}: {e}"),
        }
    }

    #[test]
    fn an_endpoint_is_nothing_a_port_an_address_or_both() {
        let lo = |p| SocketAddr::from((Ipv4Addr::LOCALHOST, p));
        assert_eq!(endpoint(None, DEBUG_CABLE_PORT), Some(lo(7661)));
        assert_eq!(endpoint(None, TERMINAL_PORT), Some(lo(5900)));
        assert_eq!(endpoint(Some("5901"), TERMINAL_PORT), Some(lo(5901)));
        assert_eq!(endpoint(Some("0.0.0.0"), TERMINAL_PORT), "0.0.0.0:5900".parse().ok());
        assert_eq!(endpoint(Some("10.0.0.2:7000"), DEBUG_CABLE_PORT), "10.0.0.2:7000".parse().ok());
        assert_eq!(endpoint(Some("::1"), TERMINAL_PORT), "[::1]:5900".parse().ok());
        assert_eq!(endpoint(Some("70000"), TERMINAL_PORT), None);
        assert_eq!(endpoint(Some("nowhere"), TERMINAL_PORT), None);
        assert_eq!(endpoint(Some("nowhere:5900"), TERMINAL_PORT), None);
    }

    #[test]
    fn an_endpoint_against_a_default_keeps_what_is_not_said() {
        let base: SocketAddr = "10.0.0.2:5900".parse().unwrap();
        assert_eq!(endpoint_at(None, base), Some(base));
        assert_eq!(endpoint_at(Some("5901"), base), "10.0.0.2:5901".parse().ok());
        assert_eq!(endpoint_at(Some("0.0.0.0"), base), "0.0.0.0:5900".parse().ok());
        assert_eq!(endpoint_at(Some("127.0.0.1:7"), base), "127.0.0.1:7".parse().ok());
        assert_eq!(endpoint_at(Some("x"), base), None);
    }
}
