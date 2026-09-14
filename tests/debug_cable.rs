// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! DBGIN's connector with the debugger elsewhere: a machine that runs on
//! its own until a debugger connects, in step with it while one is on the
//! cable, and on its own again when the cable goes --- as the bus
//! interface's DBGIN is always there and nothing in the machine enables
//! it.  [`muir::lashup::Connector`], on `rtl`; the netlist board's own
//! connector is held to `rtl` in `tests/chip.rs`.

use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use muir::engine::Engine;
use muir::isa::asm::filler;
use muir::lashup::{Connector, DebugProgram, Plug, Remote, Turn};
use muir::machine::Machine;
use muir::rtl::Rtl;
use muir::spy;

/// The debuggee: fillers throughout the control store, so the PC climbs one
/// a microcycle for as long as the test runs and nothing touches the buses.
fn debuggee() -> Rtl {
    let mut m = Machine::new();
    m.amem[3] = 0o123456;
    m.load_prom(&vec![filler(); 512]);
    m.imem.fill(filler());
    let mut r = Rtl::new(m);
    r.boot();
    r
}

fn unibus(eadr: u8) -> u32 {
    spy::BASE + 2 * eadr as u32
}

/// A debugger that reads the PC once and leaves the clock running.
fn reads_the_pc() -> Rtl {
    let mut a = DebugProgram::new();
    a.dbg_read(unibus(spy::PC), 0o101);
    let mut r = Rtl::new(a.finish());
    r.boot();
    r
}

/// CC's first five operations on a debuggee: stop the clock, read the PC,
/// step once, read it again.
fn stops_and_steps() -> Rtl {
    let mut a = DebugProgram::new();
    a.dbg_write(unibus(spy::CLK), 0);
    a.dbg_read(unibus(spy::PC), 0o101);
    a.dbg_write(unibus(spy::CLK), 2);
    a.dbg_write(unibus(spy::CLK), 0);
    a.dbg_read(unibus(spy::PC), 0o102);
    let mut r = Rtl::new(a.finish());
    r.boot();
    r
}

/// The debugger's clock is run to here, which is time enough for the five
/// operations; a run that hangs is failed by [`BOUND`].
const NS: u64 = 120_000;
const BOUND: Duration = Duration::from_secs(60);

/// A debugger on its own thread, connecting to `addr`, running its
/// program to [`NS`] and agreeing to stop; its machine comes back.
fn debugger(addr: std::net::SocketAddr, program: fn() -> Rtl) -> std::thread::JoinHandle<Rtl> {
    std::thread::spawn(move || {
        let stream = TcpStream::connect(addr).unwrap();
        stream.set_nodelay(true).unwrap();
        let reader = stream.try_clone().unwrap();
        let mut end = Remote::debugger(program(), reader, stream);
        end.run_until(NS).unwrap();
        end.machine
    })
}

/// The debuggee's run loop while a debugger is coming: the connector
/// attended until one is on the cable --- the machine stands, so that the
/// PC at the connection is known --- then stepped as the cable allows until
/// the debugger has gone.  Returns whether the cable ever went, and why.
fn serve_one_debugger(end: &mut Connector<Rtl>) -> String {
    let t = Instant::now();
    loop {
        assert!(t.elapsed() < BOUND, "no debugger connected");
        match end.attend() {
            Some(Plug::Connected(_)) => break,
            Some(Plug::Refused(from)) => panic!("a second debugger from {from} with none on"),
            None => std::thread::sleep(Duration::from_millis(1)),
        }
    }
    loop {
        assert!(t.elapsed() < BOUND, "the debugger never went: the two clocks are not in step");
        end.attend();
        match end.step_cabled().unwrap() {
            Turn::Unplugged(why) => return why,
            Turn::Stepped | Turn::Waited => {}
        }
    }
}

/// **A debugger that connects to a machine already running reads it where
/// it stands, and the machine runs on when the debugger has gone.**  The
/// debuggee runs a while on its own before the first debugger comes ---
/// its clock far from zero, where a cable plugged at power-on has both
/// clocks --- and that debugger reads a PC no lower than where the
/// machine stood.  The cable goes, the machine runs on its own, and a
/// second debugger stops it and steps it exactly as the first debugger of
/// a run does in `tests/chip.rs`.
#[test]
fn a_debugger_may_come_late_and_go_and_another_come() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let mut end = Connector::new(debuggee(), Some(listener)).unwrap();
    assert_eq!(end.addr(), Some(addr));
    assert!(end.debugger().is_none());

    // On its own first: no debugger, nothing on the connector.
    for _ in 0..300 {
        assert!(end.attend().is_none());
        end.machine_mut().step().unwrap();
    }
    let pc_before = end.machine().pc();
    let ns_before = end.machine().ns();
    assert!(pc_before >= 250, "fillers climb: PC {pc_before:o}");

    // The first debugger, reading the PC once.
    let near = debugger(addr, reads_the_pc);
    let why = serve_one_debugger(&mut end);
    let a = near.join().unwrap();
    assert!(end.debugger().is_none(), "the cable went: {why}");
    assert_eq!(end.connections(), 1);
    assert_eq!(a.machine().bus_error, 0, "every cycle of the debugger's was answered");
    assert_eq!(a.debug_cycles(), 1);
    let read1 = a.machine().amem[0o101] as u16;
    assert!(
        read1 >= pc_before,
        "the PC read, {read1:o}, is no lower than where the machine stood when the debugger \
         came, {pc_before:o}"
    );
    assert!(end.machine().ns() > ns_before, "the machine ran while the debugger was on");

    // On its own again: the clock was left running and the PC climbs.
    for _ in 0..200 {
        end.machine_mut().step().unwrap();
    }
    let pc_between = end.machine().pc();
    assert!(pc_between > read1, "ran on after the cable went: PC {pc_between:o} > {read1:o}");

    // The second debugger stops it and steps it.
    let near = debugger(addr, stops_and_steps);
    serve_one_debugger(&mut end);
    let a = near.join().unwrap();
    assert_eq!(end.connections(), 2);
    assert_eq!(a.machine().bus_error, 0);
    assert_eq!(a.debug_cycles(), 5);
    let (read2, read3) = (a.machine().amem[0o101] as u16, a.machine().amem[0o102] as u16);
    assert!(read2 >= pc_between, "read where it stood: {read2:o} >= {pc_between:o}");
    assert_eq!(read3, read2 + 1, "one step");
    assert_eq!(end.machine().pc(), read3, "and it stands where the debugger last read it");
    // Stopped by the debugger, it stays stopped when the cable goes.
    for _ in 0..50 {
        end.machine_mut().step().unwrap();
    }
    assert_eq!(end.machine().pc(), read3, "the clock the debugger stopped stays stopped");
}

/// **A cable that goes with a request on it lifts the request.**  The
/// debugger drops off the network without a word while its cycle is on
/// the debuggee's DBGIN --- what an unplugged cable's pull-ups do to
/// `-DEBUG IN REQ` --- and the debuggee, told nothing, runs on with the
/// request lifted and the connector listening, so that the next debugger
/// finds a machine and not a Unibus held by a master that is gone.
#[test]
fn a_cable_that_goes_lifts_the_request_on_it() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let mut end = Connector::new(debuggee(), Some(listener)).unwrap();

    // The debugger puts its first cycle on the cable and is dropped there:
    // the socket shut down with no `Done`, as the death of a process shuts
    // it.  Dropping the `Remote` alone would not do it --- its reader
    // thread holds a handle and sits in a read --- so the socket is shut
    // down by hand, which is what the kernel does for a process that dies.
    let gone = std::thread::spawn(move || {
        let stream = TcpStream::connect(addr).unwrap();
        stream.set_nodelay(true).unwrap();
        let reader = stream.try_clone().unwrap();
        let socket = stream.try_clone().unwrap();
        let mut a = Remote::debugger(stops_and_steps(), reader, stream);
        while a.machine.debug_cycles() < 1 {
            a.step().unwrap();
        }
        // A few more turns, so the request has reached the other end.
        for _ in 0..20 {
            let _ = a.step();
        }
        drop(a);
        socket.shutdown(std::net::Shutdown::Both).unwrap();
    });
    let why = serve_one_debugger(&mut end);
    gone.join().unwrap();
    assert!(why.contains("went away"), "the cable went without a word: {why}");
    assert!(end.debugger().is_none());
    for _ in 0..100 {
        end.machine_mut().step().unwrap();
    }
    assert!(!end.machine().debug_busy(), "the request the debugger left is lifted");

    // The next debugger finds a machine.
    let near = debugger(addr, stops_and_steps);
    serve_one_debugger(&mut end);
    let a = near.join().unwrap();
    assert_eq!(end.connections(), 2);
    assert_eq!(a.machine().bus_error, 0, "every cycle answered");
    assert_eq!(a.debug_cycles(), 5);
    let (read2, read3) = (a.machine().amem[0o101] as u16, a.machine().amem[0o102] as u16);
    assert_eq!(read3, read2 + 1, "one step");
}

/// **One cable per connector.**  A second debugger connecting while one is
/// on the cable is refused --- its connection closed --- and the first is
/// not disturbed.
#[test]
fn a_second_debugger_is_refused_while_one_is_on() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let mut end = Connector::new(debuggee(), Some(listener)).unwrap();
    let near = debugger(addr, stops_and_steps);
    let t = Instant::now();
    loop {
        assert!(t.elapsed() < BOUND);
        if let Some(Plug::Connected(_)) = end.attend() {
            break;
        }
    }
    // The second, refused: its stream ends at once.
    let second = TcpStream::connect(addr).unwrap();
    let refused = loop {
        assert!(t.elapsed() < BOUND);
        match end.attend() {
            Some(Plug::Refused(from)) => break from,
            Some(Plug::Connected(from)) => panic!("a second cable from {from}"),
            None => std::thread::sleep(Duration::from_millis(1)),
        }
    };
    assert_eq!(refused, second.local_addr().unwrap());
    let mut buf = [0u8; 1];
    assert_eq!(std::io::Read::read(&mut &second, &mut buf).unwrap(), 0, "closed at once");
    // The first runs to its end undisturbed.
    loop {
        assert!(t.elapsed() < BOUND);
        end.attend();
        if let Turn::Unplugged(_) = end.step_cabled().unwrap() {
            break;
        }
    }
    let a = near.join().unwrap();
    assert_eq!(a.machine().bus_error, 0);
    assert_eq!(a.debug_cycles(), 5);
    assert_eq!(end.connections(), 1, "the refused one is not a connection");
}

/// **A connector with no listener is the machine alone**: `attend` finds
/// nothing, there is no address, and `finish` has nobody to tell.
#[test]
fn no_listener_is_a_machine_alone() {
    let mut end = Connector::new(debuggee(), None).unwrap();
    assert_eq!(end.addr(), None);
    assert!(end.attend().is_none());
    assert!(end.debugger().is_none());
    end.machine_mut().step().unwrap();
    end.finish().unwrap();
}
