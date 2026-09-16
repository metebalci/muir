// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Two Chaosnet interfaces on one cable: the netlist I/O board and the
//! behavioral interface, each at its own address, so that AIM-628 §2.5's
//! hardware flow control can be watched from both ends at once.
//!
//! Every other test puts one interface on the cable and model stations
//! beside it, and a model station has no Command/Status Register: what a
//! *sender* reads when a busy receiver aborts its frame cannot be asked
//! of one. Here the sender is an interface both times.
//!
//! **One thing a cable with two interfaces on it does not do here.** The
//! behavioral interface's turn timer is fed the frames model
//! transmitters start (`Ether::board_cable_frame`), and the netlist
//! board's frame is not one of those: it is its transceiver's level and
//! the cells the decoder takes off it. So that interface does not know
//! the cable is busy with the board's frame, and the two tests below
//! have one sender at a time. A test that wanted them contending for the
//! cable would want the turn timer fed from that frame's first edge as
//! the receiver now is.

use muir::chaos::cable::Transceiver;
use muir::chaos::ether::{Ether, Event};
use muir::chaos::interface::{self as chaos, csr};
use muir::chaos::packet::{Packet, op};
use muir::chaos::wire;
use muir::ioboard::IoBoard;
use muir::netlist::Netlist;
use muir::part::Level;
use muir::unibus::UnibusMaster;

mod support;
use support::{cadrio, quiet};

/// The netlist board's switches.
const BOARD: u16 = 0o3050;
/// The behavioral interface's.
const MODEL: u16 = 0o3060;

fn rfc_time(from: u16, to: u16) -> Vec<u16> {
    Packet {
        opcode: op::RFC,
        forward: 0,
        dest: to,
        dest_index: 0,
        source: from,
        source_index: 0o21,
        number: 1,
        ack: 0,
        data: b"TIME".to_vec(),
    }
    .to_buffer(to)
}

/// The netlist board and the behavioral interface on one cable.
///
/// **The cable is the behavioral interface's**, because an [`Ether`] is
/// owned by whatever it is plugged into and there is only one of it here.
/// The netlist board reaches it through a [`Transceiver`], which is the
/// transceiver half of the [`muir::chaos::cable::OnCable`] every other
/// netlist run uses, against a cable it does not own.
struct Two<'a> {
    b: UnibusMaster<'a>,
    t: Transceiver,
    m: IoBoard,
}

impl<'a> Two<'a> {
    fn new(n: &'a Netlist) -> Two<'a> {
        let mut b = UnibusMaster::new(n, 10_000, &quiet());
        for (reference, closed) in chaos::switches(BOARD) {
            b.chip.set_switches(reference, closed);
        }
        b.run(b.now + 2_000);
        let mut ether = Ether::new();
        ether.keep_log(true);
        let mut m = IoBoard::default();
        m.plug_chaos(MODEL, Some(ether), 0, false);
        let t = Transceiver::new(n);
        let mut two = Two { b, t, m };
        two.run(two.b.now);
        two
    }

    fn now(&self) -> u64 {
        self.b.now
    }

    /// The cable, which the behavioral interface holds.
    fn ether(&mut self) -> &mut Ether {
        self.m.chaos.as_mut().unwrap().ether_mut().unwrap()
    }

    /// The behavioral interface's CSR, without a Unibus cycle: a cycle
    /// takes microseconds, and these are read while a frame is coming in.
    fn model_csr(&self) -> u16 {
        self.m.chaos.as_ref().unwrap().csr()
    }

    /// The netlist board's four `LSTCNT` bits, the 74LS161 at LMMYNM 0F04,
    /// read off the nets for the same reason.
    fn board_lost(&self) -> u16 {
        (0..4).map(|k| ((self.b.level(&format!("LSTCNT{k}")) == Level::High) as u16) << k).sum()
    }

    /// Whether the netlist board's line driver is on, off `TRANS.DATA+`.
    fn board_tx(&self) -> bool {
        self.t.board_tx(&self.b.chip)
    }

    /// Whether the behavioral interface's line driver is on: its
    /// busy-receiver abort signal, which is all that board drives of its
    /// own accord.
    fn model_tx(&mut self) -> bool {
        self.ether().board_driving()
    }

    /// Both to `until`, stopping at every tap of the board and every
    /// instant the cable has something to do --- [`UnibusMaster::run`]
    /// with the cable in another board's keeping.
    fn run(&mut self, until: u64) {
        while self.b.now < until {
            let now = self.b.now;
            let cable = self.ether().next_due().map(|t| t.max(now));
            let next = [self.b.chip.next_tap(), cable]
                .into_iter()
                .flatten()
                .min()
                .map_or(until, |t| t.min(until));
            self.b.now = next.max(now);
            let at = self.b.now;
            self.b.chip.transition(at);
            // The interface first: it is the cable's keeper, and it takes
            // what the cable has for it at the cable's own due instants
            // ([`muir::chaos::board::Interface::advance`]). Then the
            // board's transceiver, which gives the cable what the board
            // drives now and reads the cable back onto its receive pairs.
            let Two { b, t, m } = self;
            m.advance(at);
            t.apply(&mut b.chip, m.chaos.as_mut().unwrap().ether_mut().unwrap(), at);
            if next >= until {
                break;
            }
        }
    }

    /// A register cycle on the netlist board. **Only with the cable
    /// quiet**: a Unibus cycle is microseconds of the board's own time,
    /// and the cable is not stepped through it.
    fn cycle(&mut self, uaddr: u32, write: Option<u16>) -> u16 {
        let now = self.now();
        assert!(!self.ether().busy(now), "a register cycle with a frame on the cable");
        let (_, w) = self.b.cycle(uaddr, write);
        let at = self.b.now;
        let Two { b, t, m } = self;
        m.advance(at);
        t.apply(&mut b.chip, m.chaos.as_mut().unwrap().ether_mut().unwrap(), at);
        w
    }

    /// The same on the behavioral interface, which has no bus timing.
    fn model(&mut self, uaddr: u32, write: Option<u16>) -> u16 {
        let now = self.now();
        match write {
            Some(v) => {
                self.m.write(uaddr, v, now);
                0
            }
            None => self.m.read(uaddr, now),
        }
    }

    /// Both interfaces reset and their receivers cleared.
    fn reset(&mut self) {
        self.cycle(chaos::CSR, Some(csr::RESET));
        self.run(self.now() + 2_000);
        self.cycle(chaos::CSR, Some(csr::CLEAR_RECEIVER));
        self.model(chaos::CSR, Some(csr::RESET));
        self.model(chaos::CSR, Some(csr::CLEAR_RECEIVER));
    }

    /// `words` written into the netlist board's transmitter and started.
    fn board_sends(&mut self, words: &[u16]) {
        for &w in words {
            self.cycle(chaos::WRITE_BUFFER, Some(w));
        }
        self.cycle(chaos::START, None);
    }

    /// The same for the behavioral interface.
    fn model_sends(&mut self, words: &[u16]) {
        for &w in words {
            self.model(chaos::WRITE_BUFFER, Some(w));
        }
        self.model(chaos::START, None);
    }

    /// When each frame started on the cable, in order.
    fn sent(&mut self) -> Vec<u64> {
        self.ether()
            .log
            .iter()
            .filter_map(|e| if let Event::Sent(t, _, _) = e { Some(*t) } else { None })
            .collect()
    }

    /// Whether each run of bits the cable carried checked good.
    fn heard(&mut self) -> Vec<bool> {
        self.ether()
            .log
            .iter()
            .filter_map(|e| if let Event::Heard(_, f) = e { Some(f.check_ok) } else { None })
            .collect()
    }

    /// Runs until `done` or `for_ns` have passed, stepping as [`Two::run`]
    /// does. Returns the instant `done` first held, if it did.
    fn until(&mut self, for_ns: u64, mut done: impl FnMut(&mut Two<'a>) -> bool) -> Option<u64> {
        let end = self.now() + for_ns;
        while self.now() < end {
            let next = self.now() + 5;
            self.run(next.min(end));
            if done(self) {
                return Some(self.now());
            }
        }
        None
    }
}

/// **A busy netlist board stops a sending interface, which reads Transmit
/// Abort.** AIM-628 §2.5, memo page 5: "When a receiving interface
/// determines that an incoming packet is addressed to it, but its receive
/// buffer already contains a packet, it sends an abort signal which causes
/// the transmitter to stop"; §7 gives Transmit Abort as "the last
/// transmission was aborted, by a collision or because the receiver was
/// busy", and §2.6, memo page 8, that "the transmitter does not
/// distinguish receiver-busy aborts from real collisions".
///
/// The receiver here is the board as MIT wired it --- `-LOST.ONE` out of
/// the 74S10 at LMMYNM 0D02 presetting the `ABORT` flip-flop at LMMODU
/// 0A09, whose driver is the 26LS31 at LMLNDR 0A02 --- and the sender is
/// the behavioral interface, whose CSR is read. That is the memo's flow
/// control carried end to end between two interfaces, which no other test
/// has: the sender everywhere else is a model station with no registers.
///
/// Measured, from the second frame's first edge on the cable: the board's
/// line driver on 12,250 ns in, the cell after the destination word ---
/// 48 cells, the three hardware words --- and off 1,125 ns later, four
/// bit cells of abort signal; the sender reads **Transmit Abort 375 ns
/// after the abort signal starts**, which is three edges of the 8 MHz
/// clock a transmitter looks for interference on, since it finds it only
/// at an edge and only while its own driver is high. Its CSR is then
/// 040300, Transmit Done with Transmit Abort and CRC Error while the
/// cable is still busy; **Lost Count at the receiver reads 1**, the
/// packet already in its buffer is untouched and still checks good, and
/// the cable carried the first frame whole and the second stopped before
/// its end.
#[test]
fn a_busy_netlist_board_stops_the_sending_interface_which_reads_transmit_abort() {
    let n = cadrio();
    let mut x = Two::new(&n);
    x.reset();
    // The first frame fills the board's buffer and is not read out.
    x.model_sends(&rfc_time(MODEL, BOARD));
    let landed = x
        .until(1_000_000, |x| x.b.level("RDONE") == Level::High)
        .expect("the first frame landed in the board's buffer");
    eprintln!("the board's buffer is full at {landed}");
    assert_eq!(x.board_lost(), 0, "nothing lost yet");
    // The second, addressed to the board by name while its buffer is full.
    x.model_sends(&rfc_time(MODEL, BOARD));
    let mut on = None;
    let mut off = None;
    let mut abort_at = None;
    let mut done_at = None;
    x.until(600_000, |x| {
        let driving = x.board_tx();
        if driving {
            on.get_or_insert(x.now());
        } else if on.is_some() {
            off.get_or_insert(x.now());
        }
        let c = x.model_csr();
        if c & csr::TRANSMIT_ABORT != 0 {
            abort_at.get_or_insert(x.now());
        }
        if abort_at.is_some() && c & csr::TRANSMIT_DONE != 0 {
            done_at.get_or_insert(x.now());
        }
        off.is_some() && done_at.is_some()
    });
    let sent = x.sent();
    let second = *sent.get(1).unwrap_or_else(|| panic!("a second frame went: {sent:?}"));
    let on = on.expect("the board drove the cable back");
    let off = off.expect("and let it go again");
    let abort_at = abort_at.expect("the sender read Transmit Abort");
    eprintln!(
        "the second frame's first edge at {second}; the board's abort signal on at {on} (+{}, \
         {} cells in) and off at {off} (+{}); the sender read Transmit Abort at {abort_at} (+{} \
         from the abort signal)",
        on - second,
        (on - second) / wire::CELL_NS,
        off - on,
        abort_at - on
    );
    assert!(
        (48 * wire::CELL_NS..50 * wire::CELL_NS).contains(&(on - second)),
        "the abort signal starts in the cell after the destination word"
    );
    assert!((1_000..1_250).contains(&(off - on)), "four bit cells of abort signal");
    // Within two bit cells: a transmitter looks for interference on its
    // own clock ([`muir::chaos::ether::ABORT_NS`]) and only while its own
    // driver is high, so an abort signal that comes up while it is
    // driving low is not found until it drives high again --- at most a
    // cell --- and then at the next edge of that clock.
    assert!(
        abort_at >= on && abort_at - on <= 2 * wire::CELL_NS,
        "the sender stops within two cells of the abort signal: {} ns",
        abort_at - on
    );
    let c = x.model_csr();
    eprintln!("the sender's CSR: {c:#08o}");
    assert!(c & csr::TRANSMIT_ABORT != 0, "Transmit Abort: {c:#08o}");
    assert!(c & csr::TRANSMIT_DONE != 0, "with Transmit Done: {c:#08o}");
    assert_eq!(x.board_lost(), 1, "the receiver counted the frame in Lost Count");
    // The cable let go and the run of bits over: what the stopped frame
    // left on it, and the packet already in the buffer untouched.
    x.run(x.now() + 20_000);
    let checks = x.heard();
    eprintln!("runs of bits the cable carried, check good: {checks:?}");
    assert_eq!(checks, [true, false], "the first frame whole, the second stopped before its end");
    let c = x.cycle(chaos::CSR, None);
    eprintln!("the receiver's CSR: {c:#08o}");
    assert_eq!((c & csr::LOST_COUNT) >> 9, 1, "Lost Count reads 1: {c:#08o}");
    assert!(c & csr::RECEIVE_DONE != 0, "the first packet is still there: {c:#08o}");
    assert!(c & csr::CRC_ERROR == 0, "and its check is still good: {c:#08o}");
}

/// **A busy behavioral interface stops the netlist board, which reads
/// Transmit Abort.** The same claim the other way round: the receiver is
/// the behavioral interface and the sender the board, whose detector is
/// the `ABORT` flip-flop at LMMODU 0A09 taking `COLLISION` --- `TBUSY`
/// with `INTERFERENCE`, the 74S02 at 0B08 --- on `-FCLK^`. AIM-628 §2.3:
/// a transceiver "detects interference (another transceiver transmitting
/// at the same time as this one) and informs the interface", so the
/// receiver's abort signal is interference to the sender like any other
/// driver, which is §2.6's "the transmitter does not distinguish
/// receiver-busy aborts from real collisions".
///
/// Measured, from the board's frame's first edge: the interface's abort
/// signal on 12,260 ns in, again the cell after the destination word, and
/// off 1,000 ns later, four bit cells; `INTERFERENCE IN` at the board's
/// transceiver 240 ns after the abort signal starts --- the first cell in
/// which its own driver is high while the signal stands --- and
/// `TABORTED` 125 ns after that, one edge of `-FCLK^`, so **Transmit
/// Abort 365 ns after the abort signal**. The board's CSR is then 000300,
/// Transmit Done with Transmit Abort, and the receiver's 101200: Receive
/// Done still up with its first packet, no CRC error, and **Lost Count
/// 1**.
#[test]
fn a_busy_model_receiver_stops_the_netlist_board_which_reads_transmit_abort() {
    let n = cadrio();
    let mut x = Two::new(&n);
    x.reset();
    // The first frame fills the behavioral interface's buffer.
    x.board_sends(&rfc_time(BOARD, MODEL));
    let landed = x
        .until(1_000_000, |x| x.model_csr() & csr::RECEIVE_DONE != 0)
        .expect("the first frame landed in the interface's buffer");
    eprintln!("the interface's buffer is full at {landed}, CSR {:#08o}", x.model_csr());
    // The second, addressed to it by name while its buffer is full.
    x.run(x.now() + 5_000);
    let started = x.now();
    x.board_sends(&rfc_time(BOARD, MODEL));
    let mut first = None;
    let mut on = None;
    let mut off = None;
    let mut interfered = None;
    let mut aborted = None;
    x.until(900_000, |x| {
        if x.board_tx() {
            first.get_or_insert(x.now());
        }
        if x.model_tx() {
            on.get_or_insert(x.now());
        } else if on.is_some() {
            off.get_or_insert(x.now());
        }
        if x.b.level("'INTERFERENCE IN'") == Level::High {
            interfered.get_or_insert(x.now());
        }
        if x.b.level("TABORTED") == Level::High {
            aborted.get_or_insert(x.now());
        }
        off.is_some() && aborted.is_some()
    });
    let first = first.expect("the board put its frame on the cable");
    let on = on.expect("the interface drove the cable back");
    let off = off.expect("and let it go again");
    let interfered = interfered.expect("the board's transceiver reported interference");
    let aborted = aborted.expect("the board's TABORTED came up");
    eprintln!(
        "the board's frame started at {first} ({} ns after START); the interface's abort signal          on at {on} (+{}, {} cells in) and off at {off} (+{}); INTERFERENCE IN at {interfered}          (+{}), TABORTED at {aborted} (+{})",
        first - started,
        on - first,
        (on - first) / wire::CELL_NS,
        off - on,
        interfered - on,
        aborted - on
    );
    assert!(
        (48 * wire::CELL_NS..50 * wire::CELL_NS).contains(&(on - first)),
        "the abort signal starts in the cell after the destination word"
    );
    assert!((1_000..1_250).contains(&(off - on)), "four bit cells of abort signal");
    assert!(
        interfered >= on && interfered - on <= wire::CELL_NS,
        "the abort signal is interference to the transceiver at once: {} ns",
        interfered - on
    );
    // The `ABORT` flip-flop at LMMODU 0A09 takes `COLLISION` on `-FCLK^`,
    // so interference is found at the next edge of an 8 MHz clock.
    assert!(
        aborted >= interfered && aborted - interfered <= wire::CELL_NS,
        "the board stops within a cell of the interference: {} ns",
        aborted - interfered
    );
    x.run(x.now() + 20_000);
    let c = x.cycle(chaos::CSR, None);
    eprintln!("the sender's CSR: {c:#08o}");
    assert!(c & csr::TRANSMIT_ABORT != 0, "Transmit Abort: {c:#08o}");
    assert!(c & csr::TRANSMIT_DONE != 0, "with Transmit Done: {c:#08o}");
    let cm = x.model_csr();
    eprintln!("the receiver's CSR: {cm:#08o}");
    assert_eq!((cm & csr::LOST_COUNT) >> 9, 1, "Lost Count reads 1: {cm:#08o}");
    assert!(cm & csr::RECEIVE_DONE != 0, "the first packet is still there: {cm:#08o}");
    assert!(cm & csr::CRC_ERROR == 0, "and its check is still good: {cm:#08o}");
}
