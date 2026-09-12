// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! muir: the simulator. One engine at a time from the boot PROM, a pack on
//! the disk controller's cable, and microcycles per second against the
//! machine's own rate.
//!
//! The reference is the machine's own microcycle, read off the delay-line
//! taps: 145 ns at normal speed, so 6.9 M microcycles/s.
//!
//!     muir [--micro|--rtl|--chip] [--chaos-address <address>]
//!          [--chaos-udp [<endpoint>]] [--chaos-udp-dynamic]
//!          [--chaos-udp-peer <address>@<host>:<port>] [--checkpoint <file>]
//!          [--debug-cable-connect [<endpoint>|0x<address>]]
//!          [--debug-cable-listen [<endpoint>]] [--debug-in-process]
//!          [--debuggee-disk-pack <image>[,<unit>][,ro]]
//!          [--debuggee-terminal [<endpoint>]]
//!          [--disk-controller netlist|model]
//!          [--disk-pack <image>[,<unit>][,ro]] [--io-board netlist|model]
//!          [--main-memory netlist|model] [--main-memory-boards <n>]
//!          [--prom <file>] [--resume <file>] [--serial <endpoint>]
//!          [--stop-after <microcycles>]
//!          [--stop-at <pc>] [--stop-at-prom <pc>] [--terminal [<endpoint>]]
//!          [--tv netlist|model] [--tv-board simple-tv|lispm-tv]
//!          [--tv-capture <gif>] [--tv-capture-no-time]
//!
//! `cargo build --release` leaves it at `target/release/muir`, and `cargo
//! install --path .` puts it on the path. The programs in `examples/` stay
//! `cargo run --example` programs.
//!
//! The engine flags are mutually exclusive and `--rtl` is the default,
//! which is the engine for ordinary use: the models throughout, at about
//! twice the machine's own rate. `chip` is the reference, and it runs
//! every board as a netlist. Each board flag takes `model` instead ---
//! `rtl`'s twin of that board, the same timing, no gates --- which is how
//! one board is taken out of the picture while something else is under
//! investigation, rather than a machine to run for its own sake.
//! `--main-memory` is main memory; `--main-memory-boards` is how many
//! 64K-word boards the machine has --- main memory on every engine, the
//! boards on the Xbus on `chip` --- 32 by default for the two million
//! words. `--io-board` is the I/O board, `--tv` the display, and
//! `--tv-board` which display: the SIMPLE TV, the black-and-white board
//! System 100 drives, or the LISPM TV that replaced it. `--disk-controller`
//! is the disk controller, whose netlist runs its own microcode with the
//! pack on its cable as a drive and takes the drive's time over every
//! block, milliseconds where the model takes none. **A run that touches no
//! pack pays about 3% for that, and a run that reads one pays days**:
//! System 100 booted through the netlist controller on 12 September 2026
//! in 2 days 14 hours and 301 million microcycles, 51 hours of it the cold
//! boot's copy of all 21,342 pages of the band. The controller's sequencer
//! waits on the drive's clocks and on its own delay lines, so it cannot be
//! untimed the way the model is, and `--disk-controller model` is how a
//! `chip` run that does not care about the disk is made quick.
//! `--main-memory model` takes the disk controller down with it, the
//! netlist controller being a second master on the Xbus that the model
//! memory does not answer; asking for that memory and
//! `--disk-controller netlist` together is refused, being a backplane that
//! cannot be built. Either way the start says which controller the run
//! has. The other engines run the models always.
//! `--disk-pack` takes a pack image --- the System 100 release's
//! `disk-sys-100-0.img` --- and there is no default: no flag is a drive
//! with no pack in it. The image is the pack's blocks end to end, 256 words of 32 bits
//! each, in the geometry's order. It is opened read-write and written as a
//! drive writes its pack. After the image, in either order, come the
//! drive's unit --- 0 unless a DISK MULTIPLEXOR is fitted with
//! `--disk-multiplexor`, the netlist controller having one port of its
//! own --- and
//! `ro`, the drive's read-only switch, `STATUS<7>`, with the file opened
//! read-only behind it, and a write then faults as MIT says it does. With
//! no pack at all the engines run the same PROM waiting on a drive that
//! never answers, which measures the wait loop.
//!
//! Every run serves a terminal: the display, the keyboard and the mouse
//! over RFB, RFC 6143, so that any VNC viewer can work the machine, which
//! has no other way to be worked. It is at VNC's display :0 on the loopback,
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
//! elsewhere.
//!
//! **A band wants a file and time host, and muir is not one**: a CADR had
//! no such server in it, and neither has this. The host is another program
//! on the network --- `ozd`, `https://github.com/metebalci/ozd`, is one
//! that boots a band --- and `--chaos-address` is what reaches it, this
//! machine's own sixteen address switches and, with them, Chaosnet over
//! UDP: the cable goes on the network at port 42042 unless `--chaos-udp`
//! says otherwise, and every host `--chaos-udp-peer` names is then a
//! station on the same modelled cable, taking its turn on it. Which
//! numbers a run wants are its band's: `--chaos-address 3050` with
//! `--chaos-udp-peer 3060@<where the host is>` for the System 100 pack,
//! `4401` with `4403@...` for System 304's. Every engine has a Chaosnet.
//! muir stays a leaf: a packet for somewhere else is dropped rather than
//! forwarded, and a `cbridge` beside it is what routes;
//! `--chaos-udp-dynamic` lets a host muir was never told about be answered
//! where its packets came from. A machine that reaches no host --- no
//! `--chaos-address`, or one at a number its band does not call --- stops
//! in the debugger at the initialization that wants a host: `Super-B`
//! there, then the date and time it asks for and `y`, finish it.
//!
//! The machine's other way out is the serial port at J9, the 2651 at
//! IOBSER 0A12, and `--serial <endpoint>` is where it is reached: a TCP
//! port, or address:port, attached to with `nc` or `telnet`. A connection
//! is the device on the null-modem cable plugging in --- `DSR`, `DCD` and
//! `CTS` asserted, which is what the chip needs before it will transmit or
//! receive at all --- and hanging up drops them; one device at a time. The
//! rate and the frame are the machine's, whatever it programmed into the
//! chip, and nothing at this end sets or checks them, so a far end that
//! assumes another rate reads garbage as it would on a real line. The port
//! is off unless the flag is given: nothing needs it to work the machine,
//! and on `chip` a port the machine has opened counts the baud-rate
//! crystal and the I/O board stops idling. It is one machine's, so it is
//! refused with the lashup.
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
//!
//! `--checkpoint` and `--resume` work on all three engines. On `micro`
//! and `rtl` a checkpoint is [`Machine`] and the engine's own state; on
//! `chip` there are no arrays to write, so it is the boards --- every
//! net, every part's cells, every oscillator, one-shot and delay-line
//! transition in flight on the processor, the bus interface, the memory
//! boards, the I/O board and the display --- with the machine behind the
//! buses and what each end of each bus is driving onto the others. It is
//! taken at the first microcycle from the stop with no bus cycle in
//! flight, which is the only kind of instant it does not describe, and
//! those microcycles are counted and said. A netlist disk
//! controller's drives are on its own cable rather than in the machine,
//! and the multiplexor between them on its connector rather than the
//! backplane; both are in a checkpoint too, each where it stood.

use std::io::IsTerminal;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
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
use muir::lashup::{FreeRunning, Lashup, Remote};
use muir::machine::Machine;
use muir::micro::Micro;
use muir::netlist;
use muir::part::Level;
use muir::prompt::{Command, Memory, NetName};
use muir::rtl::Rtl;
use muir::serial::Endpoint;
use muir::terminal::keyboard::{Keyboard, Mapping};
use muir::terminal::mouse::Mouse;
use muir::terminal::{Frame, Terminal};

/// 145 ns per microcycle at normal speed: `Speed::cycle_ns`, and what
/// `tests/clock.rs` pins.
const HARDWARE_CYCLES_PER_S: f64 = 1e9 / 145.0;

/// How often the terminal is given a turn: about thirty times a second.
/// The machine's own raster is 64.7 Hz, so a viewer sees every other frame
/// at best, and the cost does not show in any engine's rate.
///
/// **The terminal has no thread of its own on purpose.** It was measured
/// rather than assumed, release build, one viewer reading as fast as it
/// can, 768 by 963, the screen scrambled so no run of equal bits flatters
/// the encoder: a poll costs 0.7 us with nobody connected, 100 to 200 us
/// on an idle screen, 0.4 ms for a hundred rows changed and 3.0 ms for the
/// whole screen. At thirty polls a second that is about 0.5% of wall time
/// idle, 1.2% scrolling, and 9% only if the whole screen repaints every
/// poll --- which [`terminal::Terminal::FULL_UPDATE_INTERVAL`] already caps
/// at one whole screen a raster frame. The share is the same on every
/// engine, the interval being wall clock rather than microcycles.
///
/// **1.2% does not buy a lock.** The frame buffer is written by the
/// processor, by the disk controller's DMA and by the netlist display's
/// mirror, and none of them has to know a viewer exists; a second thread
/// reading the screen would put a synchronisation point into a part of the
/// machine that has none.
///
/// What would change the answer: a screen that really does repaint whole
/// at 30 Hz for long stretches; a terminal doing more per poll than
/// encoding raw rectangles, such as a compressed encoding or several
/// viewers wanting different pixel formats; this interval dropping to the
/// machine's own 64.7 Hz; or a profile of a real session putting the
/// terminal higher than these figures predict. **Those are one machine on
/// one day: acting on them means measuring again, not quoting them.**
const TERMINAL_INTERVAL: Duration = Duration::from_millis(33);

/// Microcycles between glances at the computer's clock to see whether
/// [`TERMINAL_INTERVAL`] has gone by. `Instant::now` is not free and
/// `micro` runs 66 M microcycles a second.
const TERMINAL_CHECK: u64 = 4_096;

/// How often the serial endpoint is given a turn, when `--serial` has
/// opened one: as often as the terminal.
///
/// A poll is a system call or two and a run reaches a check far more often
/// than a serial line has anything to say --- `micro` sixteen thousand
/// times a second. A character typed at the endpoint waits at most this
/// long to reach the port, which is about one character's own time at 300
/// baud, the rate MIT's `sys/io1/serial.lisp` defaults to.
const SERIAL_INTERVAL: Duration = TERMINAL_INTERVAL;

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

/// Where `--debug-cable-connect` puts the debuggee: at an endpoint on the
/// network, which is another program speaking the cable's frames, or
/// behind a window of memory-mapped registers, which is a CADR in FPGA
/// fabric on the board muir is running on ([`muir::fabric`]).
///
/// One flag rather than two, because it is one concept --- this machine is
/// the debugger and here is the debuggee --- and the argument says which
/// it is: `0x` is unambiguous against a port, a host name and a host with
/// a port, so nothing has to be remembered about which flag takes which. A
/// host genuinely named `0x…` is not supported.
#[derive(Clone, Copy)]
enum Connect {
    Endpoint(SocketAddr),
    Window(u64),
}

/// Whether a debug cable flag's argument names the fabric's register
/// window rather than an endpoint: `0x` or `0X`.
fn names_a_window(spec: Option<&str>) -> bool {
    spec.is_some_and(|v| v.starts_with("0x") || v.starts_with("0X"))
}

/// The physical address the fabric's register window is at, out of a
/// `--debug-cable-connect 0x…`: hexadecimal after the prefix, and a
/// multiple of four, the window being 32-bit registers and every access to
/// it one 32-bit load or store. Page alignment is not asked for, which
/// would be a needless restriction. There is no default: where the window
/// sits is a property of the bitstream and muir holds no opinion about it.
fn window_address(flag: &str, spec: &str) -> u64 {
    let want = format!(
        "{flag} {spec}: the fabric's register window is 0x and a physical address in hexadecimal, \
         a multiple of 4"
    );
    let at = u64::from_str_radix(&spec[2..], 16).unwrap_or_else(|_| usage(&want));
    if !at.is_multiple_of(4) {
        usage(&want);
    }
    at
}

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

/// `--chaos-udp-peer`'s argument, `<address>@<host>[:<port>]`: a
/// Chaosnet host and where it lives. The address is octal or
/// `subnet:host`, as `--chaos-address` takes it; the host is a name or an
/// address, and the port may be left off for the protocol's own.
///
/// **The name is resolved here**, once, before a machine is built, so
/// that a name with no address is a refusal at the start rather than a
/// peer that is never reached. A name that moves afterwards is not
/// followed; naming the address instead, or `--chaos-udp-dynamic`, is
/// what covers that.
fn peer_spec(arg: &str) -> Result<(u16, SocketAddr), String> {
    let (address, lives) = arg.split_once('@').ok_or("wants <address>@<host>:<port>")?;
    let a = muir::chaos::parse_address(address)
        .ok_or_else(|| format!("{address} is not an address in octal or subnet:host"))?;
    let first = |s: String| s.to_socket_addrs().ok().and_then(|mut a| a.next());
    let at = first(lives.to_string())
        .or_else(|| first(format!("{lives}:{}", muir::chaos::udp::PORT)))
        .ok_or_else(|| format!("{lives} has no address this host can reach"))?;
    Ok((a, at))
}

/// A pack flag's argument parsed, or the usage. Which units a run can
/// fill is the controller's business and not the flag's: see
/// `--disk-multiplexor`.
fn pack_flag(flag: &str, arg: Option<String>) -> Pack {
    let arg = arg.unwrap_or_else(|| usage(&format!("{flag} wants <image>[,<unit>][,ro]")));
    pack_spec(&arg).unwrap_or_else(|e| usage(&format!("{flag} {arg}: {e}")))
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

/// `--serial`'s endpoint: a port, on the loopback, or an address and a
/// port.
///
/// **The port has to be named.** The other endpoints have a default to
/// fall back on --- VNC's display :0, the debug cable's 7661 for DBGOUT's
/// Unibus address --- and this one has none: the serial port is off unless
/// the flag is given, so a number here would be muir's own invention and
/// not something a viewer or a convention already knows. A bare address is
/// refused rather than bound where nobody was told to attach.
fn serial_endpoint(spec: &str) -> Option<SocketAddr> {
    if let Ok(a) = spec.parse::<SocketAddr>() {
        return Some(a);
    }
    spec.parse::<u16>().ok().map(|p| SocketAddr::from((Ipv4Addr::LOCALHOST, p)))
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
        if m.ioboard.take_beep() {
            term.ring();
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
const DM: &str = include_str!("../data/DM.netlist");

/// Which display board `--tv-board` puts on the backplane.
#[derive(Clone, Copy, PartialEq)]
enum TvBoard {
    SimpleTv,
    LispmTv,
}

impl TvBoard {
    /// What `--tv-board` calls it, which is also what a checkpoint carries
    /// so that a resume onto the other one is refused by the flag's name.
    fn name(self) -> &'static str {
        match self {
            TvBoard::SimpleTv => "simple-tv",
            TvBoard::LispmTv => "lispm-tv",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Which {
    Micro,
    Rtl,
    Chip,
}

/// What the ratio does **not** include: the disk.
///
/// This is microcycles against microcycles. With the disk controller as a
/// behavioural model --- always on `micro` and `rtl`, and on `chip` when
/// it is given `--disk-controller model` --- a transfer completes inside
/// the store to `START` and a seek takes no time. The machine spent
/// milliseconds on a seek and spent them running the microcode's polling
/// loop, so a 55 ms seek is about 380,000 microcycles the hardware executes
/// and muir does not. A program that seeks therefore finishes further ahead
/// of the hardware than this says, by an amount that depends on the program.
///
/// On the netlist disk controller it inverts: the drive takes its own time,
/// the polling loop runs through every gate on the board, and a seeking
/// program comes out slower than this rather than faster.
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

const USAGE: &str = "usage: muir [--micro|--rtl|--chip] [--chaos-address <address>]
            [--chaos-trace] [--chaos-udp [<endpoint>]] [--chaos-udp-dynamic]
            [--chaos-udp-peer <address>@<host>:<port>] [--checkpoint <file>]
            [-c|--config <file>]
            [--debug-cable-connect [<endpoint>|0x<address>]]
            [--debug-cable-listen [<endpoint>]] [--debug-in-process]
            [--debuggee-chaos-address <address>]
            [--debuggee-disk-pack <image>[,<unit>][,ro]]
            [--debuggee-terminal [<endpoint>]]
            [--disk-controller netlist|model] [--disk-multiplexor]
            [--disk-pack <image>[,<unit>][,ro]]
            [--io-board netlist|model]
            [--keyboard-mapping <file>] [--keyboard-mapping-dump]
            [--keyboard-mapping-trace]
            [--main-memory netlist|model]
            [--main-memory-boards <n>] [--no-auto-boot] [--prom <file>]
            [--resume <file>] [--serial <endpoint>]
            [--stop-after <microcycles>] [--stop-at <pc>]
            [--stop-at-prom <pc>] [--terminal [<endpoint>]]
            [--tv netlist|model] [--tv-board simple-tv|lispm-tv]
            [--tv-capture <gif>] [--tv-capture-no-time]
            [--watch <from>[-<to>]:<net>,<net>,...] [-V|--version]";

/// What `-h` and `--help` print: the usage, then each flag in the order
/// the usage lists them.
const HELP: &str = "\
A simulator of the MIT CADR Lisp Machine.

  --micro | --rtl | --chip     the engine: microinstruction, register
                               transfer, or chip level. [default: --rtl]
  --chaos-address <address>    this machine's Chaosnet address: the sixteen
                               address switches on the I/O board, the bits
                               in octal, 3050, or subnet:host with each in
                               octal, 6:50 --- the same number, subnet in
                               the high byte. Giving it also puts the cable
                               on the network, as --chaos-udp does. Which
                               address to give is the band's own: 3050 on
                               System 100 and 4401 on System 304, whose file
                               and time hosts are 3060 and 4403 --- not
                               muir, but another program on the network,
                               named with --chaos-udp-peer. [default:
                               177001, on subnet 376 and no band's, with the
                               cable off the network]
  --chaos-trace                every Chaosnet packet and frame on the cable,
                               to stderr. [default: off]
  --chaos-udp [<endpoint>]     where the Chaosnet cable is on the network:
                               Chaosnet over UDP, which cbridge, usim, klh10
                               and ozd speak. Nothing, a port, an address or
                               address:port; a bare port is on the loopback,
                               so reaching another host means naming an
                               address to listen on. muir is a leaf: a
                               packet for another host is dropped, not
                               forwarded. [default: 127.0.0.1:42042, the
                               protocol's own port, when --chaos-address is
                               given; off with neither flag]
  --chaos-udp-dynamic          learn where a peer is from the packets it
                               sends, so that a host --chaos-udp-peer never
                               named can still be answered: whatever can
                               reach the port goes in the address table
                               under whatever address it claims. An endpoint
                               --chaos-udp-peer named is not moved by a
                               packet. It needs the link, so --chaos-address
                               or --chaos-udp. [default: off]
  --chaos-udp-peer <address>@<host>:<port>
                               a Chaosnet host reached over UDP and where it
                               lives: 3060@127.0.0.1:42043, the address in
                               octal or subnet:host and the host a name or
                               an address, resolved once here. The port may
                               be left off for 42042. Once per peer, and the
                               address may not be this machine's own. It
                               needs the link, so --chaos-address or
                               --chaos-udp.
  --checkpoint <file>          write the machine's whole state to <file>
                               when the run stops, for --resume to start
                               from. On chip it is the boards themselves,
                               taken at the first microcycle from the stop
                               with no bus cycle in flight. The prompt's
                               checkpoint writes one as the run goes, and
                               the run goes on.
  -c, --config <file>          the file of flags to read before the command
                               line, which must be there. Without it muir
                               reads .muirrc in the directory it was run
                               from, or failing that .muirrc in the home
                               directory --- the first of the three there,
                               not all of them. MUIR_RC names a file in
                               place of the two that are looked for.
  --debug-cable-connect [<endpoint>|0x<address>]
                               rtl: this machine is the debugger: its DBGOUT
                               connects to a debuggee listening at the
                               endpoint, a port, an address or address:port.
                               Either end may be another program that speaks
                               the cable's frames. An argument beginning 0x
                               is no endpoint but a physical address, where
                               the muir-fpga project's CADR presents its
                               DBGIN as a register window, reached through
                               /dev/mem. It is that project's window and no
                               other: one that does not say so is refused,
                               and there is no default for where it sits.
                               [default: 127.0.0.1:7661]
  --debug-cable-listen [<endpoint>]
                               rtl, chip: this machine is the debuggee at
                               the end of a debug cable over TCP: its DBGIN
                               waits at the endpoint for the debugger to
                               connect, and then the two run in step. On
                               chip it is the board's own connector, run an
                               event at a time. [default: 127.0.0.1:7661]
  --debug-in-process           rtl: the two-machine lashup in one process. A
                               second machine runs beside this one with both
                               debug cables between them, each machine's
                               DBGOUT to the other's DBGIN, so that CC here
                               debugs the other, or the other this one. The
                               other boots the same PROM with no pack unless
                               one is named, and its console is CC's alone.
                               The stops are this machine's, and
                               --stop-after counts its microcycles. Both
                               machines get a terminal, the other's one port
                               above.
  --debuggee-chaos-address <address>
                               rtl: the other machine's Chaosnet address, as
                               --chaos-address is this machine's. The other
                               machine has a cable of its own with no link
                               on it, so the address is all it has.
                               [default: the same address as this machine's,
                               the two cables never meeting]
  --debuggee-disk-pack <image>[,<unit>][,ro]
                               rtl: the other machine's pack, as
                               --disk-pack.
  --debuggee-terminal [<endpoint>]
                               rtl: the other machine's terminal, as
                               --terminal is this machine's, somewhere other
                               than the display above this one's: a port, an
                               address or address:port. [default: the
                               display above this machine's, 127.0.0.1:5901
                               when it is at :0]
  --disk-controller netlist|model
                               chip: the disk controller. The netlist takes
                               the drive's real milliseconds over every
                               block: a run that touches no pack pays about
                               3% for that, and System 100's boot through it
                               took two and a half days. model is how a run
                               that does not care about the disk is made
                               quick, and is what --main-memory model
                               leaves. [default: netlist]
  --disk-multiplexor           chip: a DISK MULTIPLEXOR on the netlist
                               controller's cable, which is what gives it
                               eight drive ports instead of one. Without it
                               the one port is unit 0, so a second
                               --disk-pack, or one past unit 0, is refused.
                               It needs --disk-controller netlist; the model
                               controller wants no board. [default: off,
                               with one pack in unit 0; the start says when
                               it is fitted]
  --disk-pack <image>[,<unit>][,ro]
                               the pack in a drive: its blocks end to end.
                               The image is opened read-write, as a drive
                               writes its pack. After the image, in either
                               order: the unit, and ro for the drive's
                               read-only switch --- the status word says so,
                               a write faults, and the image is opened
                               read-only, so a written block reaches a
                               checkpoint rather than the file. Once for
                               each pack, one to a unit, up to the eight the
                               controller addresses. [default: unit 0; no
                               pack unless one is named, which is a drive
                               with no pack in it and a boot that waits on
                               it for ever]
  --io-board netlist|model     chip: the I/O board. [default: netlist]
  --keyboard-mapping <file>    what a viewer's keysyms mean on the Lisp
                               Machine keyboard: `key <keysym> <key>` a
                               line, and `prefix <keysym> <keysym> <key>`
                               for a key reached by pressing one and then
                               another. It goes over the built-in mapping
                               rather than replacing it, and the prompt's
                               `keys` prints what is in force. MUIR_KEYS
                               names a file in place of the two looked for.
                               [default: .muirkeys, looked for where .muirrc
                               is; without one the built-in mapping stands]
  --keyboard-mapping-dump      write the mapping this run would use to
                               stdout, in the format --keyboard-mapping
                               reads, and stop before a machine is built.
                               Fed back in unedited it changes nothing, so
                               it is a copy to edit rather than a report:
                               `muir --keyboard-mapping-dump > my.keys`,
                               edit it, `muir --keyboard-mapping my.keys`.
  --keyboard-mapping-trace     every keysym a viewer sends and what it
                               became, on stderr, alongside the run: the
                               keysym by name and number, whether it went
                               down or up, and the key it became, spelled as
                               --keyboard-mapping-dump spells it so that the
                               line can be pasted into a mapping file.
                               [default: off]
  --main-memory netlist|model  chip: main memory as MIT's board or as rtl's
                               model of it. model takes the disk controller
                               down with it, the netlist controller being a
                               second master the model memory does not
                               answer; asking for it and --disk-controller
                               netlist together is refused. [default:
                               netlist]
  --main-memory-boards <n>     how many 64K-word boards, 1 to 60: main
                               memory on every engine, and on chip the
                               boards on the backplane. [default: 32, the
                               two million words]
  --no-auto-boot               leave the boot button unpressed, as a CADR is
                               when the power comes on: RUN clear and
                               nothing running. The run starts held at the
                               prompt, and boot there presses the button;
                               nothing else starts it, and continue and step
                               say so. [default: muir presses the button for
                               you]
  --prom <file>                the boot PROM to run, an MCR microcode file
                               as MIT's own sys/ubin/promh.mcr is: at most
                               the 512 words the machine fetches before it
                               turns the PROM off, assembled at address 0,
                               with the statistics bit IR<46> nowhere set.
                               Anything else is refused rather than run. The
                               start says how the file stands to MIT's own.
                               [default: MIT's own, built in --- System
                               100's sys/ubin/promh.mcr, version 9]
  --resume <file>              start from a checkpoint instead of cold: the
                               engine that wrote it, the same pack under it,
                               the Chaosnet plugged in afresh, and as many
                               memory boards as it had, which
                               --main-memory-boards may not gainsay. On chip
                               the boards on the backplane have to be the
                               checkpoint's too. The button is not pressed,
                               and the stops count from here.
  --serial <endpoint>          where the serial port at J9 is reached: a TCP
                               port, or address:port. Attach with `nc <host>
                               <port>` or telnet. A connection is the device
                               on the null-modem cable plugging in: it
                               asserts DSR, DCD and CTS, and hanging up
                               drops them. One device at a time, and a
                               second connection is closed as it arrives.
                               The rate and the frame are the machine's to
                               program into the 2651 --- MIT's driver
                               defaults to 300 baud, seven data bits and
                               even parity --- and nothing here sets them.
                               One machine's port, so it is refused with
                               --debug-in-process and the cable flags.
                               [default: off, and J9 empty]
  --stop-after <microcycles>   how many to run, then stop. [default: none;
                               the run goes on until a --stop-at, a halt or
                               ^C]
  --stop-at <pc>               stop when the PC reaches this address with
                               the boot PROM disabled: in microcode loaded
                               into the control store. Octal, as MIT writes
                               it.
  --stop-at-prom <pc>          the same with the PROM enabled: an address in
                               the boot PROM, below 1000. With --stop-after,
                               whichever comes first.
  --terminal [<endpoint>]      where the display, keyboard and mouse are
                               served over RFB, RFC 6143, for any VNC viewer
                               to connect to: a port, an address or
                               address:port. Every run serves one, asked for
                               or not; this says where instead, and the
                               start says where it went. A named port is
                               bound as it stands, and the run stops if it
                               cannot be; an unnamed one is where the first
                               free display is looked for. An address other
                               than the loopback lets another machine in,
                               RFB's None security being the only type
                               offered. [default: 127.0.0.1:5900, VNC's
                               display :0, or the first free display above
                               it]
  --tv netlist|model           chip: the display. [default: netlist]
  --tv-board simple-tv|lispm-tv
                               chip: which display board. [default:
                               simple-tv]
  --tv-capture <gif>           record the display to <gif> as the run goes,
                               an animated GIF timed by the machine's own
                               clock so that it plays at the machine's
                               speed. There is no default path; one must be
                               given. In the lashup it is both machines on
                               one canvas, the debugger at the left and the
                               debuggee at the right; not over the debug
                               cable, where they are two clocks.
  --tv-capture-no-time         leave the clocks off the recording. By
                               default a line below the screen, hiding none
                               of it, shows the machine's simulated time at
                               the left and the local time of day at the
                               right, each hh:mm:ss.
  --watch <from>[-<to>]:<net>,<net>,...
                               chip: record the named nets over microcycles
                               <from> to <to>, or from <from> to the end of
                               the run with no <to>, counted as --stop-after
                               counts them. Each net as the prompt's `net`
                               names one --- `disk:NEW CCW`, `PC/14` for a
                               bus --- and the nets comma separated. The
                               boards are sampled at every instant they move
                               and not once a microcycle, so a pulse shorter
                               than one is seen. One line on stderr,
                               prefixed `watch:`, with the time in
                               nanoseconds, the microcycle and every value,
                               as the range begins and then at every change.
                               The prompt's watch records the next n
                               microcycles the same way, without a restart.
                               [default: off]
  -h, --help                   this.
  -V, --version                what this build calls itself: the version,
                               the commit it was built from --- with -dirty
                               after it if the tree had uncommitted work ---
                               and whether it was built with optimisations
                               off. Every run says it in its first line too.
";

/// What this build calls itself: the crate's version, the commit it was
/// built from, and whether it was built with optimisations off ---
/// `muir 0.1.0-e4d8aeb-release`. `--version` prints it, and every run says
/// it in its first line, so a report of a run says which muir made it.
///
/// **A tree with uncommitted work in it says `-dirty` after the commit**,
/// because the commit alone would name something that was never built.
/// The commit is stamped in at build time by `build.rs`, which asks git
/// once there; nothing here runs git, and a built muir does not need it.
/// Built where there is no repository --- a source archive, a machine
/// with no git --- there is no commit to name and the version is the
/// crate's and the build's alone.
fn version() -> String {
    let build = if cfg!(debug_assertions) { "dev" } else { "release" };
    match option_env!("MUIR_GIT") {
        Some(commit) => format!("muir {}-{commit}-{build}", env!("CARGO_PKG_VERSION")),
        None => format!("muir {}-{build}", env!("CARGO_PKG_VERSION")),
    }
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

/// **A checkpoint this build cannot read is a file to make again, not a
/// mistyped flag**, so it does not get [`usage`]'s sixty lines.
///
/// Two things go stale and both land here. The format is versioned and the
/// version moves --- 10 to 15 on 8 September 2026, three of them in one
/// afternoon --- and a board's [`Chip::fingerprint`] moves under a resume
/// whenever `data/CADR.netlist` is regenerated, which is
/// "saved from a different board or a different build". Neither is the
/// reader's mistake and neither is fixed by reading the flag list; what
/// fixes both is running the machine again to the same place with
/// `--checkpoint`, which is what wrote the file in the first place.
fn stale_checkpoint(path: &Path, err: &dyn std::fmt::Display, ran: Option<u64>) -> ! {
    eprintln!("muir: --resume {}: {err}", path.display());
    eprintln!("  the file is from another build, not a broken one, and making it again is");
    eprintln!("  the fix: run the machine to the same place with --checkpoint, as this was");
    match ran {
        Some(n) => eprintln!("  written --- `--stop-after {n} --checkpoint {}`", path.display()),
        None => eprintln!("  written --- `--stop-after <microcycles> --checkpoint <file>`"),
    }
    std::process::exit(2);
}

fn help() -> ! {
    println!("{USAGE}\n\n{HELP}");
    std::process::exit(0);
}

/// The packs a run gets: the ones `--disk-pack` names, and nothing at all
/// otherwise. There is no default, because which pack a drive holds is not
/// something to guess: no flag is a drive with no pack in it, which the
/// boot waits on for ever. The path, the unit and the drive's read-only
/// switch, in the order the flags came.
fn pack_choice(packs: &[Pack]) -> Vec<(PathBuf, usize, bool)> {
    packs.iter().map(|p| (p.path.clone(), p.unit, p.read_only)).collect()
}

/// What a file of flags is called where muir looks for one.
const RC: &str = ".muirrc";

/// The keyboard mapping this run's terminal uses, settled once from the
/// flags and read by every place that makes a [`Keyboard`].
///
/// One run has one mapping --- it is what a viewer's keysyms mean, and a
/// run serves one viewer's keyboard --- so it is here rather than
/// threaded through the five timing loops and the prompt, none of which
/// would do anything with it but pass it on.
static KEYS_IN_FORCE: std::sync::OnceLock<Mapping> = std::sync::OnceLock::new();

/// Whether `--keyboard-mapping-trace` was given, for every keyboard this
/// run builds: the lashup builds two, and a trace of one machine's keys
/// and not the other's would say less than it appears to.
static KEYS_TRACED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The keyboard mapping in force, as the prompt's `keys` prints it: what
/// each of a viewer's keysyms means, and where the mapping came from.
fn keys_in_force() -> String {
    let m = KEYS_IN_FORCE.get().cloned().unwrap_or_default();
    let from = match m.source() {
        Some(p) => format!("{}, over the built-in mapping", shown(p)),
        None => "the built-in mapping".to_string(),
    };
    format!("keyboard mapping: {from}\n{}", m.show())
}

/// A keyboard on the mapping this run settled on.
fn a_keyboard() -> Keyboard {
    let mut k = Keyboard::with_mapping(KEYS_IN_FORCE.get().cloned().unwrap_or_default());
    k.traced(KEYS_TRACED.load(std::sync::atomic::Ordering::Relaxed));
    k
}

/// The keyboard mapping a run reads, beside its flags: `.muirkeys` in the
/// directory muir was run from, else `.muirkeys` in the home directory.
const KEYS: &str = ".muirkeys";

/// The keyboard mapping file this run reads, and whether it was asked for
/// by name.
///
/// `--keyboard-mapping` if it is given, else `MUIR_KEYS`, else [`KEYS`] in the
/// directory muir was run from, else [`KEYS`] in the home directory ---
/// the same order and the same rule as [`config_path`], the first of them
/// there and not all of them. A file asked for by name must be there; the
/// ones looked for need not be, and most runs have none, which leaves the
/// built-in mapping standing.
fn keyboard_path(named: Option<&Path>) -> Option<(PathBuf, bool)> {
    if let Some(p) = named {
        return Some((p.to_path_buf(), true));
    }
    if let Some(from_env) = std::env::var_os("MUIR_KEYS") {
        return Some((PathBuf::from(from_env), false));
    }
    let here = PathBuf::from(KEYS);
    if here.exists() {
        return Some((here, false));
    }
    let home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(KEYS))?;
    home.exists().then_some((home, false))
}

/// Which board of the netlist machine a net is on: where the prompt's
/// `net` and `--watch` find its chip, [`chip_on`], once a name has been
/// resolved.  The boards' names, and the order they are searched in, are
/// [`boards_named`]'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum On {
    Cpu,
    Busint,
    /// The first memory board on the backplane.
    Memory,
    /// A device on the Xbus, by its place in `Xbus::devices`.
    Device(usize),
    Io,
}

/// A board by the name `net` and `--watch` take, where it is, its chip and
/// its netlist.
type Named<'a> = (&'static str, On, &'a Chip, &'a netlist::Netlist);

/// The boards of a netlist machine that carry nets, by the names the
/// prompt's `net` and `--watch` take, in the order a name is searched:
/// the processor, the interface, a memory board, then the devices.  The
/// devices are built display first, then the disk controller, which is
/// `Xbus::new`'s own order; the I/O board is on the Unibus, last.
fn boards_named<'a>(
    cpu: &'a Chip,
    far: &'a FarEnd,
    boards: &Boards<'a>,
    (cpu_n, bus_n, mem_n): &'a (netlist::Netlist, netlist::Netlist, netlist::Netlist),
) -> Vec<Named<'a>> {
    let mut on: Vec<Named<'a>> =
        vec![("cpu", On::Cpu, cpu, cpu_n), ("busint", On::Busint, &far.board, bus_n)];
    if let Some(m) = far.xbus.boards.first() {
        on.push(("memory", On::Memory, m, mem_n));
    }
    let devices = far.xbus.devices.iter().enumerate();
    for ((what, n), (k, chip)) in [("tv", boards.tv), ("disk", boards.disk)]
        .into_iter()
        .filter_map(|(w, n)| n.map(|n| (w, n)))
        .zip(devices)
    {
        on.push((what, On::Device(k), chip, n));
    }
    if let (Some(u), Some(n)) = (far.unibus.as_ref(), boards.io) {
        on.push(("io", On::Io, &u.board, n));
    }
    on
}

/// The chip a resolved net is on, as the machine stands now.  The boards
/// are where [`boards_named`] found them: a resolved `On` names a board
/// the machine has.
fn chip_on<'a>(cpu: &'a Chip, far: &'a FarEnd, on: On) -> &'a Chip {
    match on {
        On::Cpu => cpu,
        On::Busint => &far.board,
        On::Memory => &far.xbus.boards[0],
        On::Device(k) => &far.xbus.devices[k],
        On::Io => &far.unibus.as_ref().expect("resolved on the I/O board, so it is there").board,
    }
}

/// A name resolved: the board that carries it, the nets --- one, or a
/// bus's from bit 0 up --- and any other boards that carry the name too.
struct Found {
    board: &'static str,
    on: On,
    nets: Vec<netlist::NetId>,
    also: Vec<&'static str>,
}

/// A name as `net` takes one, found on whichever board carries it, or why
/// it was not.
///
/// **A name is unique only within a board.** `-XBUS RQ` is on nearly all of
/// them and `TRIDENT.READY/` on one, so the boards are searched in the
/// order [`boards_named`] gives them and the answer says which one carried
/// it. Where more than one does, the others are named too, so that a
/// reading is never quietly the wrong board's.
///
/// A bus is `NAME/width`, `NAME0` up, which is how `MUIR_WATCH` writes one.
fn resolve_net(on: &[Named], want: &NetName) -> Result<Found, String> {
    let NetName { board: want_board, name, width } = want;
    if let Some(board) = want_board
        && !on.iter().any(|&(b, ..)| b == board)
    {
        let names: Vec<&str> = on.iter().map(|&(b, ..)| b).collect();
        return Err(format!("no board called {board} here; this run has {}", names.join(", ")));
    }
    let named = |n: &netlist::Netlist, what: &str| {
        n.by_name_id(what).or_else(|| n.by_name_id(&format!("'{what}'")))
    };
    let asked = |b: &str| want_board.as_deref().is_none_or(|w| w == b);
    // A bus is carried by `NAME0` and there may be no net called `NAME` at
    // all, so what decides which board carries it is the first bit.
    let first = match width {
        None => name.clone(),
        Some(_) => format!("{name}0"),
    };
    let carries: Vec<&Named> =
        on.iter().filter(|&&(b, _, _, n)| asked(b) && named(n, &first).is_some()).collect();
    let Some(&&(board, at, chip, n)) = carries.first() else {
        let looked: Vec<&str> = on.iter().map(|&(b, ..)| b).filter(|b| asked(b)).collect();
        return Err(format!("no net {first} on {}", looked.join(", ")));
    };
    let nets = match width {
        None => vec![named(n, name).expect("just found")],
        Some(bits) => {
            if let Some(b) = (0..*bits).find(|b| named(n, &format!("{name}{b}")).is_none()) {
                return Err(format!("{board} has no {name}{b} --- a bus is {name}0 up"));
            }
            chip.bus_nets(n, name, *bits)
        }
    };
    let also = carries[1..].iter().map(|&&(b, ..)| b).collect();
    Ok(Found { board, on: at, nets, also })
}

/// A net or a bus read off whichever board carries the name: the prompt's
/// `net` on `chip`, which is [`resolve_net`] and a reading.
///
/// A bus's value is read the way a TTL input reads it, an undriven net as
/// a one, and the count of undriven bits is said beside it, because half
/// the datapath is tri-state and undriven for part of every cycle.
fn say_net(on: &[Named], want: &NetName) -> String {
    use std::fmt::Write;
    let Found { board, on: at, nets, also } = match resolve_net(on, want) {
        Ok(found) => found,
        Err(what) => return format!("prompt: {what}\n"),
    };
    let chip = on.iter().find(|&&(_, o, ..)| o == at).map(|&(_, _, c, _)| c).expect("named");
    let name = &want.name;
    let mut out = String::new();
    match want.width {
        None => writeln!(out, "{name} on {board}: {:?}", chip.net(nets[0])).unwrap(),
        Some(bits) => {
            let word = chip.read(&nets);
            let undriven = nets.iter().filter(|&&id| chip.net(id) == Level::Z).count();
            write!(out, "{name}/{bits} on {board}: {word:o} octal, {word:#x}").unwrap();
            match undriven {
                0 => writeln!(out, ", every bit driven").unwrap(),
                k => writeln!(out, ", {k} of {bits} bits undriven and read as ones").unwrap(),
            }
        }
    }
    if !also.is_empty() {
        writeln!(out, "  ({name} is also on {})", also.join(", ")).unwrap();
    }
    out
}

/// `--watch`'s argument, parsed: the first microcycle, the last or `None`
/// for the end of the run, and the nets, still by name.
type WatchSpec = (u64, Option<u64>, Vec<NetName>);

/// `<from>[-<to>]:<net>,<net>,...`: the microcycles, counted as
/// `--stop-after` counts them, and the nets as `net` names them.
///
/// The range is before the first colon and has none of its own, so the
/// colon that ends it is the first one, and a board prefix or a colon in
/// a name --- `cpu:LM UB: GRANTED` --- is the nets' to parse.
fn watch_spec(arg: &str) -> Result<WatchSpec, String> {
    let Some((range, list)) = arg.split_once(':') else {
        return Err("wants <from>[-<to>]:<net>,<net>,...".to_string());
    };
    let (from, to) = match range.split_once('-') {
        Some((f, "")) => (f, None),
        Some((f, t)) => (f, Some(t)),
        None => (range, Some(range)),
    };
    let from: u64 = from.parse().map_err(|_| format!("{from:?} is not a microcycle"))?;
    let to = match to {
        Some(t) => Some(t.parse::<u64>().map_err(|_| format!("{t:?} is not a microcycle"))?),
        None => None,
    };
    if to.is_some_and(|t| t < from) {
        return Err(format!("the range ends at {} before it begins at {from}", to.unwrap()));
    }
    let nets = muir::prompt::parse_net_names(list, "--watch")?;
    Ok((from, to, nets))
}

/// What one watched net read as the last time a line was printed: a net
/// as `net` prints it, a bus as a word when every bit is driven.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reading {
    Net(Level),
    Bus(Option<u64>),
}

/// One net or bus being recorded: what to call it, where it is, its nets.
struct Watched {
    label: String,
    on: On,
    nets: Vec<netlist::NetId>,
    bus: bool,
}

/// `--watch` and the prompt's `watch`: the named nets recorded over a range
/// of microcycles, one line on stderr at every change.
///
/// **The record is taken at the chip's own step, which is an event and
/// not a fixed interval.** [`FarEnd::tick_with`] moves every board to the
/// next instant anything on any of them is due --- the processor clock's
/// next transition, or a delay-line tap, an oscillator edge or a one-shot
/// on some board, whichever comes first --- and the gates are zero-delay,
/// so between two of those instants no net moves.  Sampled after each,
/// the record has every level a net settled at, however brief: `CCW CLK`
/// on the disk controller is two taps of a delay line 50 ns apart, and
/// once-a-microcycle sampling, 145 to 220 ns, would step over it.  What
/// it does not have is a level a net took and left inside one instant ---
/// a glitch of no width --- which is not a level the model has either.
///
/// The microcycle is the run's own count, as `--stop-after` and the
/// prompt's `pc` count it: a resume counts from the checkpoint.  The one
/// stretch of a run the record does not cover is the walk to a quiet
/// microcycle a checkpoint makes, [`chip_to_quiet`], which ticks the
/// boards itself, up to a thousand microcycles: those are counted and
/// said, and not sampled.
struct Watch {
    from: u64,
    /// `None` runs to the end of the run.
    to: Option<u64>,
    nets: Vec<Watched>,
    /// What was last printed for each, `None` before the first line.
    last: Vec<Option<Reading>>,
}

impl Watch {
    /// The nets resolved against the boards, or which one was not.
    fn new(from: u64, to: Option<u64>, nets: &[NetName], on: &[Named]) -> Result<Watch, String> {
        let nets = nets
            .iter()
            .map(|want| {
                let found = resolve_net(on, want)?;
                Ok(Watched {
                    label: want.to_string(),
                    on: found.on,
                    nets: found.nets,
                    bus: want.width.is_some(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let last = vec![None; nets.len()];
        Ok(Watch { from, to, nets, last })
    }

    /// What is recorded, comma separated, for the line that says so.
    fn labels(&self) -> String {
        self.nets.iter().map(|w| w.label.as_str()).collect::<Vec<_>>().join(", ")
    }

    /// One sample, after one step of the boards: a line if anything
    /// watched has changed since the last one, or at the first sample of
    /// the range, so that a reader has the values it began at.  `false`
    /// once the range is behind, when there is nothing more to sample.
    ///
    /// Before the range this is one comparison, and the run pays nothing
    /// else for a watch still to come.
    fn sample(&mut self, cycle: u64, now: u64, cpu: &Chip, far: &FarEnd) -> bool {
        if cycle < self.from {
            return true;
        }
        if self.to.is_some_and(|to| cycle > to) {
            return false;
        }
        let mut changed = false;
        for (w, last) in self.nets.iter().zip(self.last.iter_mut()) {
            let chip = chip_on(cpu, far, w.on);
            let reading = if w.bus {
                Reading::Bus(chip.read_driven(&w.nets))
            } else {
                Reading::Net(chip.net(w.nets[0]))
            };
            if *last != Some(reading) {
                *last = Some(reading);
                changed = true;
            }
        }
        if changed {
            use std::fmt::Write;
            let mut line = format!("watch: {now} ns, microcycle {cycle}:");
            for (w, last) in self.nets.iter().zip(&self.last) {
                match last.expect("every net has been read") {
                    Reading::Net(l) => write!(line, " {}={l:?}", w.label).unwrap(),
                    Reading::Bus(Some(v)) => write!(line, " {}={v:o}", w.label).unwrap(),
                    Reading::Bus(None) => write!(line, " {}=Z", w.label).unwrap(),
                }
            }
            eprintln!("{line}");
        }
        true
    }
}

/// The mapping this run's terminal uses: the built-in one, with whatever
/// [`keyboard_path`] found over it.  A file that cannot be read or that
/// says something muir does not understand stops the run rather than
/// leaving the user with a keyboard that is quietly not the one they
/// wrote.
fn keyboard_mapping(named: Option<&Path>) -> (Mapping, String) {
    let Some((path, _)) = keyboard_path(named) else {
        return (
            Mapping::default(),
            format!("built in; no {KEYS} found (--keyboard-mapping <file>)"),
        );
    };
    match Mapping::from_file(&path) {
        Ok(m) => {
            let line = format!("{}, over the built-in one", shown(&path));
            (m, line)
        }
        Err(e) => usage(&format!("--keyboard-mapping {e}")),
    }
}

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
/// and the file of flags are looked for under that directory, and their
/// whole paths are long and say nothing a reader does not know.
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

/// Attaches each pack, [`pack_choice`], to its unit.
fn attach(m: &mut Machine, packs: &[Pack]) {
    for (p, unit, read_only) in pack_choice(packs) {
        // A drive writes its pack, so the image is opened read-write and a
        // written block goes into the file. `ro` is the drive's own
        // read-only switch: the file is opened read-only behind it, a
        // written block stays in memory for the run and goes into a
        // checkpoint instead, and the machine sees the write fault as MIT
        // says it does.
        let opened = if read_only {
            Unit::open(&p, Geometry::T300)
        } else {
            Unit::open_rw(&p, Geometry::T300)
        };
        let mut u = match opened {
            Ok(u) => u,
            Err(e) => fail(&format!("{}: {e}", p.display())),
        };
        u.read_only = read_only;
        m.disk.attach(unit, u);
    }
}

fn machine(prom: &[Insn], packs: &[Pack], memory_boards: usize) -> Machine {
    let mut m = Machine::with_memory_boards(memory_boards);
    m.load_prom(prom);
    attach(&mut m, packs);
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
    let (mut keyboard, mut mouse) = (a_keyboard(), Mouse::new());
    let (mut b_keyboard, mut b_mouse) = (a_keyboard(), Mouse::new());
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
    let mut keyboard = a_keyboard();
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
                if e.machine_mut().ioboard.take_beep() {
                    term.ring();
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

/// The debugger's end of the cable to a debuggee in FPGA fabric: this
/// machine stepped and the window polled beside it by
/// [`FreeRunning::step`], with the terminal and the stops as for a machine
/// alone. Nothing is promised either way and there is nothing at the far
/// end to agree with about stopping: the run ends where this machine's
/// own stops say, with any request left at the window lifted as the window
/// goes.
fn time_fabric(
    mut run: FreeRunning<muir::fabric::Fabric<muir::fabric::Mapped>>,
    stop: Stop,
    terminal: Option<&mut Terminal>,
) {
    let t = Instant::now();
    let mut halt = None;
    let mut terminal = terminal;
    let (mut keyboard, mut mouse) = (a_keyboard(), Mouse::new());
    let mut last_poll = Instant::now();
    let mut ran = 0;
    catch_interrupts();
    let mut interrupts_seen = 0;
    loop {
        let e = &run.debugger;
        if ran >= stop.after || stop.reached(e.pc(), !e.machine().mode.prom_disable) {
            break;
        }
        if interrupted(&mut interrupts_seen) {
            break;
        }
        match run.step() {
            Ok(()) => ran += 1,
            Err(muir::lashup::Error::Halt(h)) => {
                halt = Some(h);
                break;
            }
            Err(e) => {
                eprintln!("muir: the debug cable: {e}");
                break;
            }
        }
        // A window that stops answering as the adapter is the end of the
        // run: what it gives after that is not data. An adapter that has
        // lost muir's request is said and not stopped on --- the cycle is
        // one the debugger times out, and the next begins again.
        if let Some(fault) = run.debuggee.fault() {
            eprintln!("muir: the fabric's window: {}", fault.what);
            if fault.fatal {
                break;
            }
        }
        if ran % TERMINAL_CHECK == 0 {
            let e = &mut run.debugger;
            let poll = last_poll.elapsed() >= TERMINAL_INTERVAL;
            attend(terminal.as_deref_mut(), poll, e.machine_mut(), &mut keyboard, &mut mouse);
            if poll {
                last_poll = Instant::now();
            }
        }
    }
    report("rtl, debugger", ran, t.elapsed().as_secs_f64());
    let e = &run.debugger;
    stop.conclude(ran, e.pc(), !e.machine().mode.prom_disable, halt);
    match run.debuggee.taken() {
        Some((count, faults)) => println!(
            "       {} debug cycles on the cable; the window took {count} requests, faults {faults:#x}",
            run.debugger.debug_cycles()
        ),
        None => println!(
            "       {} debug cycles on the cable; the window no longer answers as the adapter",
            run.debugger.debug_cycles()
        ),
    }
    if let Some(term) = terminal {
        serve_last_screen(term, &run.debugger.machine().simpletv);
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

fn time_engine<E: Engine>(
    name: &str,
    mut e: E,
    terminal: Option<&mut Terminal>,
    serial: Option<&mut Endpoint>,
    run: Run,
) {
    let Run { stop, capture, checkpoint, setup, hold, clocks } = run;
    let t = Instant::now();
    let mut ran = 0;
    let mut halt = None;
    let mut terminal = terminal;
    let mut serial = serial;
    let mut keyboard = a_keyboard();
    let mut mouse = Mouse::new();
    let mut last_poll = Instant::now();
    let mut last_serial = Instant::now();
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
            if e.machine_mut().ioboard.take_beep() {
                term.ring();
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
        // The serial port's endpoint, when `--serial` opened one: what the
        // port has finished sending goes to the socket, and what was typed
        // at it goes on the cable. The port takes its own frame time over
        // each character either way, so a burst read in one turn still
        // arrives one frame at a time.
        if check
            && let Some(end) = serial.as_deref_mut()
            && last_serial.elapsed() >= SERIAL_INTERVAL
        {
            let now = e.machine().ns;
            end.poll_cable(&mut e.machine_mut().ioboard.serial.cable, now);
            last_serial = Instant::now();
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
                    // Main memory is an array here and a physical address
                    // is an index into it; on `chip` it is the memory
                    // boards' cells and the same address picks the board.
                    Ok(Some(Command::Mem { from, words })) => {
                        let m = e.machine();
                        let read = |a: usize| m.main.get(a).copied();
                        match muir::prompt::main_dump(from, words, m.main.len(), read) {
                            Ok(dump) => print!("{dump}"),
                            Err(what) => println!("prompt: {what}"),
                        }
                    }
                    Ok(Some(Command::Net(_) | Command::Watch { .. })) => {
                        println!("prompt: nets are the chip engine's --- this machine is");
                        println!("        registers and memories and has no wires to read;");
                        println!("        `reg` gives the registers and `pc` the PC");
                    }
                    Ok(Some(Command::Info)) => print!("{setup}"),
                    Ok(Some(Command::Keys)) => print!("{}", keys_in_force()),
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
    // One last turn, so that what the port sent between the final poll and
    // the stop reaches whoever is attached before the socket closes.
    if let Some(end) = serial {
        let now = e.machine().ns;
        end.poll_cable(&mut e.machine_mut().ioboard.serial.cable, now);
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

/// **How many `SIGUSR1`s have come**: `kill -USR1` on a run asks it where
/// it is, and the run answers between two microcycles and carries on.
///
/// A long `chip` run has no prompt --- the process has a terminal and
/// nothing else --- so before this the only way to know where one was
/// was to infer it from what it had touched. Issue 86 has a run whose
/// state was read from pack mtimes, then from lit pixels, then from a
/// block-by-block comparison, two of the three retracted, over six hours,
/// with `pc` unanswered throughout.
static DUMPS: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_dump(_signal: std::ffi::c_int) {
    DUMPS.fetch_add(1, Ordering::SeqCst);
}

/// `SIGINT`, 2 on every Unix; `SIGUSR1`, 30 on macOS and the BSDs and 10
/// on Linux; and `SIG_DFL`, 0.
const SIGINT: std::ffi::c_int = 2;
#[cfg(target_os = "linux")]
const SIGUSR1: std::ffi::c_int = 10;
#[cfg(not(target_os = "linux"))]
const SIGUSR1: std::ffi::c_int = 30;
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
    // `SIGUSR1` is taken by every engine and acted on by `chip`, which is
    // the one with no prompt. Every engine, because the default action
    // for it is to kill the process: a signal sent to the wrong run of a
    // pair would otherwise end a run that had been going for hours.
    let dump: extern "C" fn(std::ffi::c_int) = on_dump;
    unsafe { signal(SIGUSR1, dump as *const () as usize) };
}

/// This run's own process id, for the banner line that says how to ask it
/// where it is.
fn pid() -> u32 {
    std::process::id()
}

/// Whether `SIGUSR1` has come since this was last asked, as
/// [`interrupted`] is for ^C.
fn asked_where(seen: &mut u32) -> bool {
    let now = DUMPS.load(Ordering::SeqCst);
    let asked = now > *seen;
    *seen = now;
    asked
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
fn say_pc_chip(
    c: &Chip,
    pc_nets: &[netlist::NetId],
    ir_nets: &[netlist::NetId],
    ran: u64,
    prom_enabled: bool,
) {
    let prom = if prom_enabled { " in the PROM" } else { "" };
    println!("PC {:o}{prom}; {ran} microcycles this run", c.read(pc_nets) as u16);
    // **The instruction, which on a halt is the one that halted.** `IR` is
    // held while the machine is stopped, and `HALT-CONS` is `IR<11:10>`,
    // so a machine that stopped itself says here what stopped it --- which
    // the PC does not, `ILLOP` being a `POPJ` whose PC is the address it
    // popped rather than the trap.
    let ir = c.read(ir_nets);
    println!("IR {ir:#014x}; misc function {}", (ir >> 10) & 3);
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
/// [`say_memory`] for `chip`, where a memory is the RAM chips' cells
/// rather than an array: `rams` is in [`Memory`]'s own order, built once
/// when the machine was.
fn say_chip_memory(
    c: &Chip,
    rams: &[muir::chip::Ram],
    memory: Memory,
    from: usize,
    words: Option<usize>,
) -> Result<String, String> {
    let ram = &rams[memory as usize];
    if from >= ram.len() {
        return Err(format!(
            "{} is {:o} words, and {from:o} is past its end",
            memory.name(),
            ram.len()
        ));
    }
    let to = match words {
        Some(n) => from.saturating_add(n).min(ram.len()),
        None => ram.len(),
    };
    let all: Vec<u32> = (from..to).map(|a| ram.word(c, a)).collect();
    Ok(muir::prompt::dump(&all, from))
}

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

/// Writes a netlist machine to `path` and says how big it came, or why
/// it did not: [`muir::cable::write_checkpoint`], which is the one format
/// the cosim harness writes too.  Taken where [`FarEnd::quiet`] says it
/// may be, which is what [`chip_to_quiet`] runs on to.
fn write_chip_checkpoint(
    path: &Path,
    cpu: &Chip,
    clk: &Behavioural,
    far: &FarEnd,
    tv_board: TvBoard,
    ran: u64,
) {
    match muir::cable::write_checkpoint(path, ran, tv_board.name(), cpu, clk, far) {
        Ok(n) => eprintln!("checkpoint: {} at {ran} microcycles, {n} bytes", path.display()),
        Err(err) => eprintln!("checkpoint: could not write {}: {err}", path.display()),
    }
}

/// Loads a `chip` checkpoint onto a netlist machine built as the flags say
/// and not booted, and says which microcycle it resumed at; or says why
/// not and exits.  The cables are joined after it, so that each board
/// holds what the others drive onto it.
fn resume_chip(
    cpu: &mut Chip,
    clk: &mut Behavioural,
    far: &mut FarEnd,
    tv_board: TvBoard,
    (path, c): &(PathBuf, Checkpoint),
) -> u64 {
    let refuse = |err: std::io::Error| -> ! { stale_checkpoint(path, &err, None) };
    let mut it = muir::cable::read_checkpoint(c).unwrap_or_else(|e| refuse(e));
    if it.tv_board != tv_board.name() {
        usage(&format!(
            "--resume {}: a {} checkpoint, and --tv-board is {}",
            path.display(),
            it.tv_board,
            tv_board.name()
        ));
    }
    let ran = it.ran;
    it.processor(cpu)
        .and_then(|()| {
            *clk = it.clock()?;
            it.far_end(far)
        })
        .unwrap_or_else(|e| refuse(e));
    far.join(cpu, clk.time_ns());
    eprintln!(
        "resumed: {} at {ran} microcycles, {} ns, {} memory boards",
        path.display(),
        clk.time_ns(),
        far.buses.machine.memory_boards()
    );
    ran
}

/// Runs on to the first point a netlist machine may be checkpointed at,
/// and says how many microcycles that took, or what the machine was doing
/// instead if it did not come.
///
/// **Not every microcycle boundary is one.** A checkpoint carries no bus
/// cycle in flight --- [`FarEnd::quiet`] is what says so --- and no memory
/// request from the processor, whose answer would be owed to a cycle the
/// checkpoint does not describe.  Between cycles both are true, and a
/// machine reaches such a point within a few microcycles: the longest
/// anything holds them is the bus timeout, about twelve microseconds,
/// which is eighty microcycles.  The bound is well past that, and a
/// machine that never comes quiet is told about rather than checkpointed
/// wrong.
///
/// **A transition on its way down a delay line used to be asked about
/// here too, and is not any more.** The format had no field for one until
/// version 21, so a board could not be saved with one in flight; it has
/// one now, and `tests/checkpoint.rs` holds a machine loaded from a
/// checkpoint taken with taps in flight to being the machine that was
/// never stopped.  That was the half of this that a busy machine could
/// not get past: issue 89, where a run held after two and a half hours
/// could not be banked.
fn chip_to_quiet(
    cpu: &mut Chip,
    clk: &mut Behavioural,
    far: &mut FarEnd,
    memrq: netlist::NetId,
) -> Result<u64, &'static str> {
    let mut why = match chip_busy_with(cpu, far, memrq) {
        None => return Ok(0),
        Some(why) => why,
    };
    let mut ran = 0;
    let mut last = clk.phase_ns();
    while ran < 1000 {
        far.tick_with(cpu, clk);
        let p = clk.phase_ns();
        if p < last {
            ran += 1;
            match chip_busy_with(cpu, far, memrq) {
                None => return Ok(ran),
                // The last boundary's, so a run that never came quiet can
                // say what the machine was doing at one rather than what
                // it happens to be doing between two.
                Some(w) => why = w,
            }
        }
        last = p;
    }
    Err(why)
}

/// What is holding a netlist machine off a checkpoint at this instant, or
/// `None` if nothing is: what [`chip_to_quiet`] runs on until, and what it
/// says the machine was doing instead when it never came.
///
/// The two are told apart because they mean different things to whoever
/// asked: a bus cycle is the boards', and a machine doing nothing else
/// but bus cycles may never be between them, while a memory request is
/// the microcode's and goes as soon as it is answered.
fn chip_busy_with(cpu: &Chip, far: &FarEnd, memrq: netlist::NetId) -> Option<&'static str> {
    if !far.quiet() {
        Some("a bus cycle in flight")
    } else if cpu.net(memrq) == Level::High {
        Some("a memory request up")
    } else {
        None
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
    e.load(&mut r).and_then(|()| r.done()).unwrap_or_else(|err| stale_checkpoint(path, &err, None));
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
    /// `IR<47:0>`, the instruction register.  It holds the instruction the
    /// machine last executed, and on a halt that is the one that halted it
    /// --- `HALT-CONS` is `IR<11:10>` --- which is why the prompt prints it
    /// beside the PC here and does not on the other engines, where it is
    /// not what a halt leaves behind.
    ir_nets: Vec<netlist::NetId>,
    /// The board's five readable memories, in [`Memory`]'s order, each a
    /// map from the RAM chips' cells to a word.  Built once: `Ram::new`
    /// walks every instance, and the prompt would otherwise do it per
    /// command.
    rams: Vec<muir::chip::Ram>,
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
    packs: &[Pack],
    boards: Boards,
    memory_boards: usize,
    chaos: muir::chaos::Config,
    auto_boot: bool,
) -> ChipMachine {
    let n = netlist::parse(NETLIST).unwrap();
    let mut c = Chip::new(&n);
    c.power_on();
    c.load_prom(&n, image);
    c.settle();
    let mut clk = Behavioural::new();
    let mut machine = Machine::with_memory_boards(memory_boards);
    attach(&mut machine, packs);
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
    let ir_nets = c.bus_nets(&n, "IR", 48);
    // In `Memory`'s order, so the prompt indexes by the command's own enum.
    let rams: Vec<muir::chip::Ram> = ["A", "M", "DISPATCH", "PDL", "SPC"]
        .iter()
        .map(|name| {
            let m = muir::chip::MEMS.iter().find(|m| m.name == *name).expect("a memory by name");
            muir::chip::Ram::new(&c, &n, m)
        })
        .collect();
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
        ir_nets,
        rams,
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
        // The behavioural board under `--io-board model`. The netlist
        // board's speaker is `AUDIO+`/`AUDIO-` out of the 75118 at IOBXCV
        // 0F30 and nothing is plugged into that pair, so a beep on it is
        // heard by nobody.
        if far.buses.machine.ioboard.take_beep() {
            term.ring();
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

/// One turn of the serial endpoint for a netlist machine.
///
/// Two far ends, and which one is on J9 is `--io-board`'s: the netlist
/// board's, a bit at a time on the EIA wires, or the behavioural port's
/// cable under `--io-board model`. The model board is advanced by its own
/// register accesses here and by nothing else, so its time is the
/// machine's last access rather than the clock's; MIT's driver polls the
/// status register, so a character leaves within a poll of being written.
fn attend_serial_chip(far: &mut FarEnd, end: &mut Endpoint) {
    match far.unibus.as_mut().and_then(|u| u.serial()) {
        Some(cable) => end.poll_on_cable(cable),
        None => {
            let now = far.buses.machine.ns;
            end.poll_cable(&mut far.buses.machine.ioboard.serial.cable, now);
        }
    }
}

/// Runs a netlist machine: what the command line has to say about one,
/// the memory board count among them, and [`Run`] for the rest.
#[allow(clippy::too_many_arguments)]
fn time_chip(
    image: &[u64],
    packs: &[Pack],
    boards: Boards,
    memory_boards: usize,
    chaos: muir::chaos::Config,
    terminal: Option<&mut Terminal>,
    serial: Option<&mut Endpoint>,
    run: Run,
    resume: Option<(PathBuf, Checkpoint)>,
    tv_board: TvBoard,
    watch: Option<WatchSpec>,
) {
    let Run { stop, capture, checkpoint, setup, hold, clocks } = run;
    let ChipMachine {
        mut cpu,
        mut clk,
        mut far,
        bus: _,
        pc_nets,
        ir_nets,
        rams,
        promdisable,
        srun,
        errhalt,
        stathalt,
        boot,
        // A resume brings the board up but does not press the button: what
        // the button and the power-on set is what the checkpoint replaces.
    } = chip_machine(image, packs, boards, memory_boards, chaos, !hold && resume.is_none());
    // One microcycle is however many clock transitions it takes for the phase
    // to wrap, not a fixed number of them.
    let t = Instant::now();
    // Where the checkpoint left the machine, which the microcycles this
    // run makes are counted from; `ran` is this run's own, as it is on the
    // other two engines, so that `--stop-after` is a window on the run and
    // not on the machine's whole life.
    let resumed_at = match &resume {
        Some(p) => resume_chip(&mut cpu, &mut clk, &mut far, tv_board, p),
        None => 0,
    };
    let mut serial = serial;
    // The far end of the null-modem cable goes on the netlist board's J9
    // only when `--serial` opened an endpoint: without one the board pays
    // nothing for a port nobody is at. After the resume, which brings the
    // board back as the checkpoint left it and carries no far end.
    if serial.is_some()
        && let Some(u) = far.unibus.as_mut()
    {
        u.plug_serial(clk.time_ns());
    }
    let mut ran = 0;
    // `MEMRQ`, for the quiet point a checkpoint is taken at.
    let memrq = netlist::parse(NETLIST).unwrap().by_name_id("MEMRQ").unwrap();
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
    let (mut keyboard, mut mouse) = (a_keyboard(), Mouse::new());
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
    // The processor's, the interface's and a memory board's netlists, for
    // the prompt's `net` and `watch`: parsed on the first one asked for,
    // since most runs ask for none.
    let mut net_netlists: Option<(netlist::Netlist, netlist::Netlist, netlist::Netlist)> = None;
    let parse_netlists = || {
        (
            netlist::parse(NETLIST).unwrap(),
            netlist::parse(BUSINT).unwrap(),
            netlist::parse(CADRM).unwrap(),
        )
    };
    // `--watch`, resolved now that the boards are there: a name no board
    // carries stops the run before it starts, as the prompt's `net` would
    // answer it, rather than recording nothing for hours.
    let mut watch = watch.map(|(from, to, nets)| {
        let named =
            boards_named(&cpu, &far, &boards, net_netlists.get_or_insert_with(parse_netlists));
        Watch::new(from, to, &nets, &named).unwrap_or_else(|what| fail(&format!("--watch: {what}")))
    });
    let mut interrupts_seen = 0;
    let mut asks_seen = 0;
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
                        say_pc_chip(&cpu, &pc_nets, &ir_nets, ran, prom_enabled(&cpu));
                    }
                }
            }
            last = p;
            // The record, at every step: [`Watch`] says why a step and
            // not a microcycle. A run with no watch pays one test here,
            // and one with a range behind it drops the watch and pays the
            // same.
            if let Some(w) = watch.as_mut()
                && !w.sample(ran, clk.time_ns(), &cpu, &far)
            {
                watch = None;
            }
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
        // **`kill -USR1` asks a run where it is**, and it answers here,
        // between two microcycles, and goes on. It does not hold the
        // machine: a reader that stopped the run would be no use for the
        // long timing runs this exists for, and this costs one atomic
        // load against the `Instant::now` above it, which the comment
        // there already calls free at this rate. Issue 86.
        if asked_where(&mut asks_seen) {
            say_pc_chip(&cpu, &pc_nets, &ir_nets, ran, prom_enabled(&cpu));
        }
        let poll = last_poll.elapsed() >= TERMINAL_INTERVAL;
        if poll || !held {
            attend_chip(&mut far, terminal.as_deref_mut(), poll, &mut keyboard, &mut mouse);
            if poll {
                last_poll = Instant::now();
            }
        }
        // The serial port's endpoint, when `--serial` opened one: a check
        // is already the terminal's cadence here, which is as often as a
        // serial line needs.
        if let Some(end) = serial.as_deref_mut() {
            attend_serial_chip(&mut far, end);
        }
        // The machine stopping itself, held on once rather than spun on,
        // exactly as `time_engine` does it off `FLAG-1`.
        if !held && let Some(why) = stopped_itself(&cpu) {
            held = true;
            stepping = None;
            say_machrun_low(why);
            say_pc_chip(&cpu, &pc_nets, &ir_nets, ran, prom_enabled(&cpu));
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
                say_pc_chip(&cpu, &pc_nets, &ir_nets, ran, prom_enabled(&cpu));
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
                        say_pc_chip(&cpu, &pc_nets, &ir_nets, ran, prom_enabled(&cpu));
                        held = false;
                    }
                    Ok(Some(Command::Hold)) => {
                        held = true;
                        say_pc_chip(&cpu, &pc_nets, &ir_nets, ran, prom_enabled(&cpu));
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
                    Ok(Some(Command::Pc)) => {
                        say_pc_chip(&cpu, &pc_nets, &ir_nets, ran, prom_enabled(&cpu))
                    }
                    Ok(Some(Command::Info)) => print!("{setup}"),
                    Ok(Some(Command::Keys)) => print!("{}", keys_in_force()),
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
                    // The scratchpads live in the RAM chips' own cells here
                    // rather than in arrays, so a dump walks those cells:
                    // `muir::chip::Ram`, which is also what
                    // `chip_and_rtl_hold_the_same_memories` holds to `rtl`,
                    // so this prints the same words that comparison checks.
                    Ok(Some(Command::Dump { memory, from, words })) => {
                        match say_chip_memory(&cpu, &rams, memory, from, words) {
                            Ok(dump) => print!("{dump}"),
                            Err(what) => println!("prompt: {what}"),
                        }
                    }
                    // Main memory is the boards on the backplane here, so a
                    // word is a bit off each of the 32 DRAMs of one bank of
                    // one board: `FarEnd::main_word`, which reads the cells
                    // and runs no bus cycle, so this is answered while the
                    // machine runs as `net` is.
                    Ok(Some(Command::Mem { from, words })) => {
                        let read = |a: usize| far.main_word(a as u32);
                        match muir::prompt::main_dump(from, words, far.main_words(), read) {
                            Ok(dump) => print!("{dump}"),
                            Err(what) => println!("prompt: {what}"),
                        }
                    }
                    // A register is a net bundle rather than a memory and
                    // wants naming one at a time; the memories and `IR`,
                    // which is what a halt leaves behind, are here.
                    Ok(Some(Command::Registers)) => {
                        println!("prompt: not on chip yet --- a register here is the nets of the");
                        println!(
                            "        parts driving it, not a word to read off; `pc` gives the"
                        );
                        println!(
                            "        PC and IR, and amem, mmem, dmem, pdl and spc the memories"
                        );
                    }
                    Ok(Some(Command::Checkpoint(path))) => {
                        let path = path.unwrap_or_else(|| timestamped("chk"));
                        match chip_to_quiet(&mut cpu, &mut clk, &mut far, memrq) {
                            Ok(on) => {
                                ran += on;
                                write_chip_checkpoint(
                                    &path,
                                    &cpu,
                                    &clk,
                                    &far,
                                    tv_board,
                                    resumed_at + ran,
                                );
                            }
                            Err(why) => println!(
                                "checkpoint: the machine has {why} and has not come quiet in \
                                 a thousand microcycles; nothing written"
                            ),
                        }
                    }
                    Ok(Some(Command::Net(want))) => {
                        // The netlists are parsed the first time one is
                        // asked for and kept: a run that never asks pays
                        // nothing, and one that asks twice parses once.
                        let netlists = net_netlists.get_or_insert_with(parse_netlists);
                        let on = boards_named(&cpu, &far, &boards, netlists);
                        print!("{}", say_net(&on, &want));
                    }
                    // The next `cycles` microcycles from here: the one in
                    // progress, or the one about to start if held at a
                    // boundary, and the rest. A watch already going, or
                    // one `--watch` set for later, is replaced.
                    Ok(Some(Command::Watch { cycles, nets })) => {
                        let netlists = net_netlists.get_or_insert_with(parse_netlists);
                        let on = boards_named(&cpu, &far, &boards, netlists);
                        let to = ran + cycles - 1;
                        match Watch::new(ran, Some(to), &nets, &on) {
                            // Not `watch: ...`, which is the record's own
                            // prefix and what a reader greps for.
                            Ok(w) => {
                                println!(
                                    "recording {} over microcycles {ran} to {to}; the record is on \
                                     stderr, each line prefixed watch:",
                                    w.labels()
                                );
                                watch = Some(w);
                            }
                            Err(what) => println!("prompt: {what}"),
                        }
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
    // The checkpoint last, and at the first quiet microcycle from here:
    // the stop falls where it falls, and a machine part way through a bus
    // cycle is not a machine a checkpoint describes.  Those microcycles
    // are the run's like any other, so they are counted and said.
    if let Some(path) = &checkpoint {
        match chip_to_quiet(&mut cpu, &mut clk, &mut far, memrq) {
            Ok(on) => {
                if on > 0 {
                    eprintln!("checkpoint: {on} microcycles on to a quiet one");
                }
                write_chip_checkpoint(path, &cpu, &clk, &far, tv_board, resumed_at + ran + on);
            }
            Err(why) => eprintln!(
                "checkpoint: {} not written: the machine has {why} and has not come quiet in \
                 a thousand microcycles",
                path.display()
            ),
        }
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
    let (mut keyboard, mut mouse) = (a_keyboard(), Mouse::new());
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
    let mut packs: Vec<Pack> = Vec::new();
    let mut chaos = muir::chaos::Config::default();
    // Whether `--chaos-address` was given, which is also what asks for
    // the CHUDP link: an address is a run saying which machine on which
    // network this is, and a network it cannot reach is no network.
    let mut address_given = false;
    // The CHUDP link: where it listens, the peers named for it, and
    // whether it learns where an unnamed one lives. The socket is bound
    // after the flags are read, so that what is refused is refused before
    // anything is bound.
    let mut udp_at: Option<SocketAddr> = None;
    let mut udp_peers: Vec<(u16, SocketAddr)> = Vec::new();
    let mut udp_dynamic = false;
    let mut cycles: Option<u64> = None;
    let mut auto_boot = true;
    let mut checkpoint: Option<PathBuf> = None;
    let mut prom_file: Option<PathBuf> = None;
    let mut keyboard_file: Option<PathBuf> = None;
    let mut keyboard_dump = false;
    let mut keyboard_trace = false;
    let mut resume: Option<PathBuf> = None;
    let mut stop_at: Option<u16> = None;
    let mut stop_at_prom: Option<u16> = None;
    let mut boards: usize = 32;
    let mut boards_given = false;
    let mut main_memory_model = false;
    let mut io = true;
    let mut tv = true;
    let mut tv_board = TvBoard::SimpleTv;
    // **The disk controller is a netlist like every other board**, since
    // 12 September 2026, when a boot through it was run to the end ---
    // two and a half days of it, issue 40.  `disk_given` is whether a run
    // said which it wanted, which decides what the model memory does to
    // it below.
    let mut disk_controller = true;
    let mut disk_given = false;
    // The DISK MULTIPLEXOR on the netlist controller's cable, which is
    // what gives it eight drive ports instead of one.
    let mut use_multiplexor = false;
    // A terminal is served whether or not it is asked for: the display,
    // the keyboard and the mouse are the only way the machine is worked.
    // `--serial` opens the other way out, and nothing on that port works
    // the machine.
    let mut listen = TerminalAt::default_display();
    let mut debuggee = false;
    let mut debuggee_pack: Option<Pack> = None;
    // The other machine's Chaosnet, which is its own: its own cable, with
    // nothing else on it. Unset, its address is the debugger's over
    // again, the two cables never meeting.
    let mut debuggee_address: Option<u16> = None;
    // Absent, or present with or without an endpoint.
    let mut debuggee_terminal: Option<Option<String>> = None;
    let mut cable_listen: Option<SocketAddr> = None;
    let mut cable_connect: Option<Connect> = None;
    // The serial port's endpoint: nothing unless `--serial` names one.
    let mut serial_at: Option<SocketAddr> = None;
    let mut capture_tv: Option<PathBuf> = None;
    let mut capture_tv_time = true;
    // `--watch`: the range and the nets, resolved against the boards once
    // the machine is built.
    let mut watch: Option<WatchSpec> = None;

    // The flags in `~/.muirrc` come first, so that a flag on the command
    // line, which is read after, has the last word.  [`muirrc`] drops the
    // ones the command line gives too, for the few that may not be given
    // twice.
    let typed: Vec<String> = std::env::args().skip(1).collect();
    // **Asked what this build is, muir answers before it reads anything
    // else.**  Neither of these runs a machine, so nothing a file of
    // flags configures applies to either, and a file outlives the flags
    // it holds: one still holding a spelling muir has since stopped
    // taking is rightly refused for a run, and would otherwise leave a
    // person with no way to ask which muir they have.  The loop below
    // answers them again, for a file that names one, which is harmless
    // and reaches nothing this has not.
    for a in &typed {
        if a == "-h" || a == "--help" {
            help();
        }
        if a == "-V" || a == "--version" {
            println!("{}", version());
            std::process::exit(0);
        }
    }
    // **Which muir, and which file of flags, before anything is parsed.**
    // A report of a refused run wants those two facts most of all, and a
    // file the person at the keyboard was not asked about is the
    // commonest reason a run will not start: one written when a flag was
    // spelled differently outlives the spelling.  So both are said here,
    // and every refusal below follows them.  The rest of the setup waits
    // until there is a machine to describe.
    let (from_file, rc) = muirrc(&typed);
    let head = {
        let mut head = format!("{} started\n", version());
        if let Some(path) = &rc
            && !from_file.is_empty()
        {
            head.push_str(&format!("flags: {}, from {}\n", from_file.join(" "), shown(path)));
        }
        eprint!("{head}");
        head
    };
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
                let p = pack_flag("--disk-pack", args.next());
                if packs.iter().any(|q: &Pack| q.unit == p.unit) {
                    usage(&format!("--disk-pack: unit {} twice; one pack a drive", p.unit));
                }
                packs.push(p);
            }
            (None, "--disk-multiplexor") => use_multiplexor = true,
            (None, "--chaos-address") => {
                // One address: this machine's sixteen switches, in octal
                // or subnet:host. The file and time host's is not muir's
                // to hold --- it is a peer over CHUDP --- so a comma is
                // the old spelling and is refused by name.
                let want = "--chaos-address wants one address in octal or subnet:host; the file and time host is not muir's, and --chaos-udp-peer is where it lives";
                let arg = args.next().unwrap_or_else(|| usage(want));
                match muir::chaos::parse_address(&arg) {
                    Some(a) => chaos.address = a,
                    None => usage(want),
                }
                address_given = true;
            }
            (None, "--chaos-trace") => chaos.trace = true,
            // The flags of the Chaosnet server muir used to carry. muir
            // has no file or time server in it any more --- a CADR had
            // none --- so there is nothing for them to configure, and
            // they say where the host went rather than reading as an
            // unknown argument.
            (
                None,
                gone @ ("--chaos-file-root" | "--chaos-file-peers" | "--debuggee-chaos-file-root"),
            ) => usage(&format!(
                "{gone} is gone: muir has no file server. A band's file and time host is another program on the network --- ozd, https://github.com/metebalci/ozd --- named with --chaos-udp-peer"
            )),
            (None, "--chaos-udp") => {
                // The endpoint is optional: the next word is it unless it is a flag.
                let spec = args.next_if(|v| !v.starts_with('-'));
                match endpoint(spec.as_deref(), muir::chaos::udp::PORT) {
                    Some(a) => udp_at = Some(a),
                    None => usage("--chaos-udp wants nothing, a port, an address or address:port"),
                }
            }
            (None, "--chaos-udp-dynamic") => udp_dynamic = true,
            (None, "--chaos-udp-peer") => {
                let want = "--chaos-udp-peer wants <address>@<host>:<port>, the address in octal or subnet:host";
                let arg = args.next().unwrap_or_else(|| usage(want));
                match peer_spec(&arg) {
                    Ok(p) => udp_peers.push(p),
                    Err(e) => usage(&format!("--chaos-udp-peer {arg}: {e}")),
                }
            }
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
            (None, "--disk-controller") => {
                disk_given = true;
                match args.next().as_deref() {
                    Some("netlist") => disk_controller = true,
                    Some("model") => disk_controller = false,
                    _ => usage("--disk-controller wants netlist or model"),
                }
            }
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
                let want = "--debuggee-chaos-address wants one address in octal or subnet:host";
                let arg = args.next().unwrap_or_else(|| usage(want));
                match muir::chaos::parse_address(&arg) {
                    Some(a) => debuggee_address = Some(a),
                    None => usage(want),
                }
            }
            (None, "--debuggee-disk-pack") => {
                debuggee_pack = Some(pack_flag("--debuggee-disk-pack", args.next()));
            }
            (None, "--debuggee-terminal") => {
                debuggee_terminal = Some(args.next_if(|v| !v.starts_with('-')));
            }
            (None, "--debug-cable-listen") => {
                // The endpoint is optional: the next word is it unless it is a flag.
                let spec = args.next_if(|v| !v.starts_with('-'));
                // The window is the other flag's. What is to be built in
                // fabric is the debuggee's DBGIN end, so the fabric is
                // always the debuggee and muir always the debugger; there
                // is no listening at a window and none is proposed.
                if names_a_window(spec.as_deref()) {
                    usage(
                        "--debug-cable-listen takes an endpoint and not a window: the fabric is \
                         the debuggee and muir the debugger, so the window is \
                         --debug-cable-connect's",
                    );
                }
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
                if names_a_window(spec.as_deref()) {
                    let at = window_address("--debug-cable-connect", spec.as_deref().unwrap());
                    cable_connect = Some(Connect::Window(at));
                } else {
                    match endpoint(spec.as_deref(), DEBUG_CABLE_PORT) {
                        Some(a) => cable_connect = Some(Connect::Endpoint(a)),
                        None => usage(
                            "--debug-cable-connect wants nothing, a port, an address, \
                             address:port or 0x<address>",
                        ),
                    }
                }
            }
            (None, "--tv-capture") => match args.next() {
                Some(path) => capture_tv = Some(PathBuf::from(path)),
                None => usage("--tv-capture wants a file for the GIF"),
            },
            (None, "--tv-capture-no-time") => capture_tv_time = false,
            (None, "--watch") => {
                const WANT: &str = "--watch wants <from>[-<to>]:<net>,<net>,...: the microcycles to record over, and the nets as `net` names them";
                let arg = args.next().unwrap_or_else(|| usage(WANT));
                match watch_spec(&arg) {
                    Ok(w) => watch = Some(w),
                    Err(e) => usage(&format!("--watch {arg}: {e}")),
                }
            }
            (None, "--checkpoint") => match args.next() {
                Some(path) => checkpoint = Some(PathBuf::from(path)),
                None => usage("--checkpoint wants a file to write"),
            },
            // Read before the loop, by `muirrc`, since the file it names is
            // where the loop's first words come from.
            (None, "-c" | "--config") => {
                args.next();
            }
            (None, "--keyboard-mapping") => match args.next() {
                Some(path) => keyboard_file = Some(PathBuf::from(path)),
                None => usage("--keyboard-mapping wants a file of key bindings"),
            },
            (None, "--keyboard-mapping-dump") => keyboard_dump = true,
            (None, "--keyboard-mapping-trace") => keyboard_trace = true,
            (None, "--no-auto-boot") => auto_boot = false,
            (None, "--prom") => match args.next() {
                Some(path) => prom_file = Some(PathBuf::from(path)),
                None => usage("--prom wants an MCR microcode file"),
            },
            (None, "--resume") => match args.next() {
                Some(path) => resume = Some(PathBuf::from(path)),
                None => usage("--resume wants a checkpoint to start from"),
            },
            (None, "--serial") => {
                const WANT: &str = "--serial wants a port or address:port: the endpoint the serial port at J9 is reached at, which has no default";
                let arg = args.next().unwrap_or_else(|| usage(WANT));
                match serial_endpoint(&arg) {
                    Some(a) => serial_at = Some(a),
                    None => usage(&format!("--serial {arg}: {WANT}")),
                }
            }
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
    // A window muir cannot map is refused by name, and before anything is
    // built: the mapping is `/dev/mem`, which only Linux has, and the
    // debugger has to be a muir running on the board's own processor.
    if let Some(Connect::Window(at)) = cable_connect
        && let Some(why) = muir::fabric::unmappable()
    {
        usage(&format!("--debug-cable-connect {at:#x}: {why}"));
    }
    // The debuggee on a cable is stepped by the debugger's events, in
    // `Remote::step`, and nothing there samples between them; refused
    // rather than quietly recording nothing.
    if which == Which::Chip && watch.is_some() && cable_listen.is_some() {
        usage("--watch is the run's own loop, which a debuggee on a cable does not have");
    }
    if debuggee_address.is_some() && !debuggee {
        usage(
            "--debuggee-chaos-address is the other machine's Chaosnet: it needs --debug-in-process",
        );
    }
    if debuggee_pack.is_some() && !debuggee {
        usage("--debuggee-disk-pack is the other machine's pack: it needs --debug-in-process");
    }
    // The serial port is one machine's. In the lashup there are two, and
    // the run loops that step them through the debug cable reach neither
    // machine's J9, so an endpoint here would be opened for one of them
    // without saying which. Refused rather than quietly the debugger's.
    if serial_at.is_some() && cabled == 1 {
        usage("--serial is one machine's serial port, and the lashup runs two");
    }
    // `--chaos-address` is a run saying which machine on which network
    // this is, so it puts the cable on the network too: the link at the
    // protocol's own port unless `--chaos-udp` said where. A run that
    // gives neither opens no socket at all, which is what most runs
    // want and what lets many of them go at once.
    if address_given && udp_at.is_none() {
        udp_at = endpoint(None, muir::chaos::udp::PORT);
    }
    // The two flags that describe a CHUDP link describe one that has to
    // be there: without a link nothing is listening and the cable carries
    // this machine alone.
    for (flag, given_it) in
        [("--chaos-udp-peer", !udp_peers.is_empty()), ("--chaos-udp-dynamic", udp_dynamic)]
    {
        if given_it && udp_at.is_none() {
            usage(&format!(
                "{flag} is part of the CHUDP link: it needs --chaos-address or --chaos-udp"
            ));
        }
    }
    // A peer is somebody else, and one endpoint an address: this
    // machine's own address is not a host over the network, and an
    // address given twice is two answers to where one host lives.
    for (k, &(a, _)) in udp_peers.iter().enumerate() {
        if a == chaos.address {
            usage(&format!("--chaos-udp-peer {a:o}: that is this machine's own address"));
        }
        if udp_peers[..k].iter().any(|&(b, _)| b == a) {
            usage(&format!("--chaos-udp-peer: {a:o} twice; one endpoint an address"));
        }
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
            "--tv-capture records a machine on its own or the lashup in one process, not an end of the debug cable to another program or to the fabric",
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
    if (checkpoint.is_some() || resume.is_some()) && cabled == 1 {
        usage("--checkpoint and --resume are one machine on its own, not the lashup");
    }
    // A checkpoint is read before the machine is built, so that the machine
    // can be built with as much memory as the checkpoint's had.
    let resume = resume.map(|path| {
        let c =
            muir::checkpoint::read(&path).unwrap_or_else(|err| stale_checkpoint(&path, &err, None));
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
    // `--keyboard-mapping-dump` is the mapping and nothing else: it writes
    // the file a run would read and stops, before a terminal is bound or a
    // machine is built, so that stdout carries the mapping alone. The
    // start banner is on stderr, so nothing else has to be held back.
    if keyboard_dump {
        let (mapping, _) = keyboard_mapping(keyboard_file.as_deref());
        print!("{}", mapping.dump());
        std::process::exit(0);
    }
    // The behavioural memory answers the interface's cycles and no other
    // master's; the netlist controller's DMA needs memory boards.
    // **The model memory takes the disk down with it unless the netlist
    // controller was asked for.**  A run that chose the model memory did
    // not ask for a netlist disk and should not be refused for the
    // default's sake; one that asked for both asked for something the
    // backplane cannot do, and is told so.  Either way the start says
    // which controller the run has, so neither is silent.
    if main_memory_model && disk_controller {
        if disk_given {
            usage(
                "--disk-controller netlist needs --main-memory netlist: the model memory does not answer a second master",
            );
        }
        disk_controller = false;
    }
    // The DISK MULTIPLEXOR hangs off the netlist controller's edge
    // connector, so there has to be one for it to hang off: the `chip`
    // engine's, and not its model. The model controller wants no such
    // board on any engine --- it is behavioural and has had eight units
    // all along, `disk_controller::UNITS` --- and `micro` and `rtl` have
    // no netlist board of any kind. The engine is named here rather than
    // left to the controller's default, which is netlist on `chip` and
    // means nothing on the other two.
    if use_multiplexor && !(which == Which::Chip && disk_controller) {
        usage(
            "--disk-multiplexor is a board on the netlist controller's cable: it needs chip, with --disk-controller netlist",
        );
    }
    // Without it the netlist controller has one drive port --- and not
    // because the unit number is forced to 0. `UNIT<2:0>` reach one
    // 74LS244's inputs at DCDA B17 and nothing else: `cadrdc/dc.wlr` gives
    // a direction per pin and there is no `TO` on any of the three, so the
    // board cannot drive them. The one-board jumpers `EP2:ER2`, `ER2:ES2`
    // and `ES2:ET1` ground them to stop them floating, and unit 0 is the
    // consequence. The multiplexor is what supplies the driver: its
    // 74LS175 at 0F05 latches `XBI<30:28>` and reports the unit back on
    // those three posts.
    //
    // So a second drive, or a drive past unit 0, wants a multiplexor ---
    // and is refused until it is asked for. muir could fit one by
    // implication, and used to; a board that appears because of how a
    // pack was spelled is a board the machine has without anybody
    // choosing it, and which machine is being simulated is the user's to
    // say. The model controller is refused nothing: it wants no board for
    // its eight units.
    // The engine is named for the same reason the multiplexor's refusal
    // names it: the controller's default is netlist on `chip` and means
    // nothing on `micro` and `rtl`, whose behavioural controller has
    // addressed eight units all along.
    if which == Which::Chip
        && disk_controller
        && !use_multiplexor
        && (packs.len() > 1 || packs.iter().any(|p| p.unit != 0))
    {
        usage(
            "the netlist disk controller has one drive port, unit 0: a second --disk-pack, or one past unit 0, wants --disk-multiplexor",
        );
    }
    // The run goes on until a stop, a halt or ^C unless a window was asked for.
    let window = cycles.unwrap_or(u64::MAX);
    let stop = Stop { after: window, at: stop_at, at_prom: stop_at_prom };
    let packs: &[Pack] = &packs;
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
    // The serial port's endpoint, if one was asked for. Its port is always
    // named, so it is bound as it stands and the run stops if it cannot
    // be: it is where someone is being told to attach, and serving that
    // somewhere else would be worse than not serving it.
    let mut serial = serial_at.map(|addr| match Endpoint::bind(addr) {
        Ok(mut end) => {
            end.trace = true;
            end
        }
        Err(e) => usage(&format!("--serial {addr}: {e}")),
    });
    // The CHUDP link, bound here so that a port that cannot be had stops
    // the run rather than leaving a machine that quietly reaches nobody.
    chaos.udp = udp_at.map(|at| {
        muir::chaos::udp::Link::bind(at, udp_peers, udp_dynamic)
            .unwrap_or_else(|e| usage(&format!("--chaos-udp {at}: {e}")))
    });
    // The other machine's Chaosnet: a cable of its own, since muir's cable
    // carries one machine.  Its address is the debugger's over again
    // unless told otherwise --- the two cables never meet, so there is
    // nothing for them to collide with.  The other machine has no CHUDP
    // link: one socket belongs to one cable, and there is no flag that
    // gives the other machine one.
    let debuggee_chaos = muir::chaos::Config {
        address: debuggee_address.unwrap_or(chaos.address),
        udp: None,
        ..chaos.clone()
    };

    // The boot PROM, before the setup: the setup says which one it is.
    let prom = boot_prom(prom_file.as_deref());
    // What a viewer's keysyms mean on the Lisp Machine keyboard, which is
    // the one part of it that is muir's own and so the user's to change.
    let (keyboard_map, keyboard_said) = keyboard_mapping(keyboard_file.as_deref());
    let _ = KEYS_IN_FORCE.set(keyboard_map);
    KEYS_TRACED.store(keyboard_trace, std::sync::atomic::Ordering::Relaxed);

    // What this run is: said once here, and again by the prompt's `info`.
    let setup = {
        use std::fmt::Write;
        let mut s = String::new();
        let engine = match which {
            Which::Micro => "micro",
            Which::Rtl => "rtl",
            Which::Chip => "chip",
        };
        // A flag the chosen engine has no use for is not an error --- one
        // `.muirrc` serves runs of every engine --- but a run that quietly
        // ignored it would look as though it had obeyed.
        for (flag, engines, has) in [
            ("--disk-controller", "chip", which == Which::Chip),
            ("--io-board", "chip", which == Which::Chip),
            ("--main-memory", "chip", which == Which::Chip),
            ("--tv", "chip", which == Which::Chip),
            ("--tv-board", "chip", which == Which::Chip),
            ("--watch", "chip", which == Which::Chip),
        ] {
            if !has && given.iter().any(|w| w == flag) {
                writeln!(s, "warning: {flag} is {engines}, and this run is {engine}: ignored")
                    .unwrap();
            }
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
            let tv_kind = tv_board.name();
            writeln!(
                s,
                "boards: I/O board {}, TV {} {tv_kind}, disk controller {}{}",
                kind(io),
                kind(tv),
                kind(disk_controller),
                if use_multiplexor { " with a multiplexor, eight drive ports" } else { "" }
            )
            .unwrap();
        }
        let chosen = pack_choice(packs);
        if chosen.is_empty() {
            writeln!(s, "pack: none; the boot waits on a drive that never answers").unwrap();
        }
        for (p, unit, ro) in chosen {
            writeln!(
                s,
                "pack: {} in unit {unit}{}",
                shown(&p),
                if ro {
                    ", the read-only switch on: nothing reaches the file"
                } else {
                    ", written as the machine writes it"
                }
            )
            .unwrap();
        }
        let reaches = match &chaos.udp {
            Some(_) => "on the network over UDP".to_string(),
            None => "alone on its cable; --chaos-address puts it on a network".to_string(),
        };
        writeln!(s, "chaosnet: {:o}, {reaches}", chaos.address).unwrap();
        if let Some(link) = &chaos.udp {
            let peers = match link.peers.as_slice() {
                [] => "no peer named, so no file or time host".to_string(),
                p => p.iter().map(|(a, e)| format!("{a:o} at {e}")).collect::<Vec<_>>().join(", "),
            };
            let learning = if link.dynamic { ", learning where others are" } else { "" };
            writeln!(s, "chaosnet udp: listening at {}, {peers}{learning}", link.at).unwrap();
        }
        writeln!(s, "terminal: {}", terminal_line(&terminal, &no_terminal, listen.addr)).unwrap();
        writeln!(s, "keyboard: {keyboard_said}").unwrap();
        if let Some(end) = &serial {
            let at = end.addr().unwrap_or_else(|_| serial_at.expect("the endpoint was asked for"));
            writeln!(s, "serial: tcp://{at} --- the device on the null-modem cable at J9").unwrap();
        }
        if debuggee {
            let pack = match debuggee_pack.as_ref() {
                Some(p) => format!("pack {}", shown(&p.path)),
                None => "no pack, so its boot waits on the drive".to_string(),
            };
            writeln!(s, "debuggee: in this process, {pack}").unwrap();
            writeln!(
                s,
                "debuggee chaosnet: {:o}, alone on a cable of its own",
                debuggee_chaos.address
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
        } else if let Some(Connect::Endpoint(a)) = cable_connect {
            writeln!(s, "debug cable: this machine the debugger, DBGOUT connecting to {a}")
                .unwrap();
        } else if let Some(Connect::Window(a)) = cable_connect {
            writeln!(
                s,
                "debug cable: this machine the debugger, DBGOUT at the fabric's window at {a:#x}"
            )
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
        // `chip` is the engine with no prompt, and the one whose runs go
        // for hours; the person watching one needs to be told this exists
        // or it does not pay. Issue 86.
        if which == Which::Chip {
            writeln!(s, "where: kill -USR1 {} prints the PC and IR, and the run goes on", pid())
                .unwrap();
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
    // **Armed before it is announced.**  The line below says `kill -USR1`
    // asks a run where it is, and the default action for that signal is
    // to kill the process, so a signal sent on the strength of the line
    // must find a handler already installed.  Each engine's loop calls
    // this again, which costs nothing: the same handler, installed twice.
    catch_interrupts();
    eprint!("{setup}");
    let setup = format!("{head}{setup}");

    match which {
        Which::Micro => {
            // The Chaosnet, as under rtl and chip: the interface is the
            // I/O board's and the board is the machine's, so it is the
            // same three lines whatever engine runs it.  What it wants
            // from an engine is a clock, and this one has the machine's
            // periods; `tests/micro_chaos.rs` holds the two engines to
            // the same conversation with the server.
            let mut m = machine(&prom, packs, boards);
            m.chaos = chaos.clone();
            m.plug_chaos(0);
            let mut e = Micro::new(m);
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
            time_engine("micro", e, terminal.as_mut(), serial.as_mut(), run);
        }
        Which::Rtl => {
            let mut m = machine(&prom, packs, boards);
            // The Chaosnet, as under chip: the interface on the I/O board
            // and, if a link was bound, the network on its cable.
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
                    attach(&mut mb, std::slice::from_ref(p));
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
            } else if let Some(Connect::Endpoint(addr)) = cable_connect {
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
            } else if let Some(Connect::Window(at)) = cable_connect {
                // The identity is read before anything is stored, and a
                // window that is not the adapter ends the run here: there
                // is no falling back to the network and no retrying.
                let window = muir::fabric::open(at).unwrap_or_else(|why| {
                    eprintln!("muir: --debug-cable-connect {at:#x}: {why}");
                    std::process::exit(1);
                });
                eprintln!("debug cable: DBGOUT at the fabric's window at {at:#x}");
                time_fabric(FreeRunning::new(e, window), stop, terminal.as_mut());
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
                time_engine("rtl", e, terminal.as_mut(), serial.as_mut(), run);
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
            // With a multiplexor on the controller's cable the six
            // one-board jumpers come off, those nets being the
            // multiplexor's to drive.
            let disk_n = disk_controller.then(|| {
                if use_multiplexor {
                    netlist::parse_with_multiplexor(CADRDC).unwrap()
                } else {
                    netlist::parse(CADRDC).unwrap()
                }
            });
            let dm_n = use_multiplexor.then(|| netlist::parse(DM).unwrap());
            let on_the_buses = Boards {
                memory: netlist_boards,
                io: io_n.as_ref(),
                tv: tv_n.as_ref(),
                disk: disk_n.as_ref(),
                multiplexor: dm_n.as_ref(),
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
                    chip_machine(&image, packs, on_the_buses, boards, chaos, true);
                let (reader, stream) = accept_debugger(&listener, addr);
                let end = DebugIn::new(&bus, cpu, clk, far);
                let remote = Remote::debuggee(end, reader, stream);
                time_chip_debuggee(remote, stop, pc_nets, promdisable, terminal.as_mut());
            } else {
                let run = Run {
                    stop,
                    capture,
                    checkpoint,
                    setup: &setup,
                    hold: !auto_boot,
                    clocks: capture_tv_time,
                };
                time_chip(
                    &image,
                    packs,
                    on_the_buses,
                    boards,
                    chaos,
                    terminal.as_mut(),
                    serial.as_mut(),
                    run,
                    resume,
                    tv_board,
                    watch,
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

    /// **`--serial`'s endpoint names its port or is refused.** The other
    /// endpoint flags have a default port to fall back on and this one has
    /// none, so a bare address --- which for them means "there, on the
    /// usual port" --- is not an endpoint here at all.
    #[test]
    fn the_serial_endpoint_is_a_port_or_an_address_and_a_port() {
        let lo = |p| SocketAddr::from((Ipv4Addr::LOCALHOST, p));
        assert_eq!(serial_endpoint("5962"), Some(lo(5962)));
        assert_eq!(serial_endpoint("0"), Some(lo(0)));
        assert_eq!(serial_endpoint("0.0.0.0:5962"), "0.0.0.0:5962".parse().ok());
        assert_eq!(serial_endpoint("[::1]:5962"), "[::1]:5962".parse().ok());
        assert_eq!(serial_endpoint("127.0.0.1"), None, "no port");
        assert_eq!(serial_endpoint("::1"), None, "no port");
        assert_eq!(serial_endpoint(""), None);
        assert_eq!(serial_endpoint("70000"), None);
        assert_eq!(serial_endpoint("nowhere"), None);
        assert_eq!(serial_endpoint("nowhere:5962"), None);
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
