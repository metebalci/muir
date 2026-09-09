// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A Unibus master for one board alone: the harness that measures a
//! Unibus slave --- the I/O board --- without the machine, the counterpart
//! of [`crate::xbus::XbusMaster`] for a board on the other bus.
//!
//! The Unibus as the bus interface drives it (`src/busint.rs`, measured on
//! the interface netlist): the address and `C1` go out, `-MSYN` follows
//! them [`crate::busint::UNIBUS_ADDRESS_NS`] later, the slave answers
//! `-SSYN` with the word for a read or having taken it for a write, and the
//! master lifts `-MSYN` [`crate::busint::UNIBUS_STROBE_NS`] after `-SSYN`,
//! whereupon the slave lifts `-SSYN`. Every line is open collector, low
//! when asserted, and pulled up at rest: `A0` is not on this board, which
//! takes words, and the board's own drawings name its bus pins with a `*`.

use crate::busint::{UNIBUS_ADDRESS_NS, UNIBUS_STROBE_NS};
use crate::chip::Chip;
use crate::netlist::{NetId, Netlist};
use crate::part::Level;

/// The Unibus between the bus interface and the I/O board, wire for wire:
/// the interface's name for a wire and the I/O board's, which end it in
/// `*`. `A0` is not on the board, which takes words; the board interrupts
/// on `BR5` and sits in the `BG5` grant chain, which the interface, the
/// arbiter, drives into it, and the other grant levels pass the board by
/// on jumpers. Read off `data/busint-connectors.txt` and the board's
/// wire list, `cadrio/iob.wlr`, whose connector column names the
/// backplane pins. `-BOOT*`, the keyboard's boot key, is not here: it
/// runs to the processor board's `-BOOT` past the interface, which has no
/// net for it (see `Cables::new`), and nothing presses the key.
pub fn wire_pairs() -> Vec<(String, String)> {
    let mut w: Vec<(String, String)> =
        (1..18).map(|b| (format!("-UB ADR{b}"), format!("-A{b}*"))).collect();
    w.extend((0..16).map(|b| (format!("-UBD{b}"), format!("-D{b}*"))));
    for (a, b) in [
        ("-UB C1", "-C1*"),
        ("-UB MSYN", "-MSYN*"),
        ("-UB SSYN", "-SSYN*"),
        ("-UB BBSY", "-BBSY*"),
        ("-UB INIT", "-INIT*"),
        ("-UB INTR", "-INTR*"),
        ("-UB SACK", "-SACK*"),
        ("-UB BR5", "-BR*"),
        ("UB BG5 IN", "BG.IN*"),
    ] {
        w.push((a.to_string(), b.to_string()));
    }
    w
}

/// The Chaosnet transmit clock's speed jumpers, as MIT set them.
///
/// The 74S163 at LMTCLK 0B03 divides the 32 MHz crystal down to `FCLK^`
/// by reloading itself from its own carry: the carry, inverted through
/// the 74S04 at 0D03, is its `-LOAD`, so it runs from the load value to
/// 15 and starts again, and `FCLK^` is one crystal period in every
/// `16 - N`. `C`, `D`, `ENP` and `ENT` are tied to the `HI.C` pull-up,
/// and **`A` and `B` are left open on the drawing and in `iob.wlr` alike**
/// --- pins 3 and 4 are `NC` in both. A TTL input left open floats high,
/// which loads 15, and a counter reloaded to 15 from its own carry never
/// leaves it: `FCLK^` sat high and nothing on the transmit side ever
/// moved, `diagnose_the_transmitter` in `tests/chaos_netlist.rs` counting
/// 1,908 edges of `MCLK^` against none of `FCLK^`.
///
/// The two are the speed jumpers. `cadrio/iob.eco` carries their history:
/// a 5.35 MHz and an 8 MHz setting in the 1/9/80 consolidation, ECO #5 of
/// 8/3/80 "Change cable speed to 4 MHz" with its "FCLK/2 fix to allow
/// UNIBUS readout at 8 MHz while cable rate is 4 MHz", and the 12/81
/// installation list's "jumpers to set timing ... these two determine
/// transmit clock speed". 8 MHz from 32 is a load value of 12, `A` and
/// `B` low; and with them low the transmitter shifts a bit every 250 ns,
/// which is the bit cell AIM-628 §2.5 gives and 4 Mbit/s. ECO #12's
/// "increase duty cycle of fclk ... to 50%" says the same from the other
/// side: a carry one period in four is 25%. So these two pins are held
/// low here, as the jumpers held them. Which pins the ECO's `B3-3` and
/// `B3-4` runs went to is not recoverable from it; the frequency is.
/// **Verified by the loopback**, `a_packet_loops_back_through_the_board`.
pub fn chaosnet_speed_jumpers(io: &Netlist) -> Vec<(NetId, Level)> {
    io.parts
        .iter()
        .filter(|p| p.page == "LMTCLK" && p.reference == "0B03")
        .flat_map(|p| p.pins.iter().filter(|&&(pin, _)| pin == 3 || pin == 4))
        .map(|&(_, net)| (net, Level::Low))
        .collect()
}

/// What an idle Chaosnet holds the board's two receive pairs at.
///
/// The Chaosnet half is on the board now, so nothing has to be held in
/// its place --- `CHAOS SSYN`, `CHAOS.IREQ` and `UB>TSR` are driven by
/// its own logic. What is still off the
/// board is the **cable**: the Am26LS33 at LMLNDR A01 reads the receive
/// pair on 1 and 2 and the interference pair on 6 and 7, and a
/// differential receiver with nothing across its inputs has no difference
/// to read. `src/part.rs` answers that honestly with an unknown, which
/// then spreads, so the far end holds both pairs at rest instead --- as
/// [`IDLE_KEYBOARD`] holds the keyboard's.
///
/// Which way round rest is: both signals inactive, because the ether at
/// rest carries nothing. AIM-628 §2.3, "When the cable is idle it is held
/// at 0 volts by the terminations", and §2.5, "if no transceivers are
/// active, the terminations will hold the ether low" --- and a low ether is
/// a zero, so `-RCVR.DATA.IN` is not asserted and `INTERFERENCE IN` is not
/// asserted.
///
/// Which levels on the pairs those are is the Am26LS33's, and MIT's wire
/// list gives its pins by name: `cadrio/iob.wlr` puts `RCVR.DATA+` on
/// A01-01 with the `USE` code `-IN` and `RCVR.DATA-` on A01-02 with `IN`,
/// `INTERFERE+` on A01-06 with `IN` and `INTERFERE-` on A01-07 with `-IN`.
/// That is the part's own pinout --- `1B` on pin 1 is the inverting input
/// and `1A` on pin 2 the non-inverting, `2A` on 6 and `2B` on 7 the other
/// way about (`am26ls33.pdf`) --- so MIT crossed the data pair into the
/// receiver on purpose, which is why its output is named active-low. Rest
/// is therefore `RCVR.DATA-` above `RCVR.DATA+`, and `INTERFERE+` below
/// `INTERFERE-`, which is what is below.
///
/// These levels and the receiver's pinout only mean anything together:
/// turn both round and the board still comes up quiet, because the two
/// errors cancel. `tests/behaviour.rs` checks the pair of them against
/// each other, and against the loop through the line driver, so that
/// neither can move on its own.
pub const IDLE_CHAOSNET: &[(&str, Level)] = &[
    ("RCVR.DATA+", Level::Low),
    ("RCVR.DATA-", Level::High),
    ("INTERFERE+", Level::Low),
    ("INTERFERE-", Level::High),
];

/// What an idle keyboard holds the board's keyboard data receiver at. The
/// board clocks the keyboard itself, `KB CLK^` at 125 kHz out through the
/// SN75118 at IOBKBD 0E30, and takes the data back through the same
/// part's receiver as `-KBDIN`, whose high is a start bit: the 74LS109 at
/// 0C26 goes busy on it, the three 74LS164s shift twenty-four bits in,
/// and `KBD READY` follows. The sheet leaves the receiver's output with
/// open inputs undefined, so the far end holds the pair as a keyboard
/// with nothing typed does, `-KBDIN` low, rather than trust what the
/// model makes of a float.
pub const IDLE_KEYBOARD: &[(&str, Level)] = &[("KBDIN+", Level::High), ("KBDIN-", Level::Low)];

/// One wire of the bus: its net on the interface and on the I/O board.
struct Wire {
    interface: NetId,
    board: NetId,
}

/// The Unibus backplane with the I/O board on it, carried the way
/// [`crate::xbus::Xbus`] carries the Xbus: what either board's drivers
/// hold, the other is given, and a wire nobody holds is pulled up.
/// The mains, as the I/O board's 60 Hz input takes it.
///
/// One cycle in nanoseconds, as a fraction for [`crate::chip::toggle_at`]:
/// **60 Hz is MIT's own figure**, written on `cadrio/clk60h.drw` --- the
/// page titled "I/O BOARD 60 CYCLE CLOCK & GPIO" --- as the comment on the
/// network at C20, `2.5 VAC, 60 Hz`, `150 ohm series`. The same comment is
/// in `cadrio/clk60h.wd`.
///
/// What it reaches: `cadrio/iob.wlr` has `POWER LINE ^` arriving at the
/// board's edge pin `FV2` and going to C20 pins 12 and 13. C20 pins 7 and
/// 8 are the input of the 74LS14 Schmitt trigger at 0D20, whose output is
/// `60 Hz`, which clocks the two 74393s at CLKTOD 0D22 and 0D23 --- the
/// free-running counter of mains cycles the `CLOCK` register reads.
///
/// **The model drives the trigger's side of C20 and not the edge pin**,
/// which is worth saying plainly. `cadrio/bodies.drw` defines `20DUMMY` as
/// a twenty-pin body of DIPTYPE `DUMMY`: no internal connections at all,
/// the 150 ohm resistor living only in the comment above. So nothing in
/// MIT's files says which pins of C20 the resistor joins, and a part model
/// that paired them would be inventing a connection rather than reading
/// one. What the drawings do settle is that the mains reaches the trigger,
/// and that is what is modelled; the conditioning between is analogue and
/// has no logic level to carry.
pub const MAINS_PERIOD: (u64, u64) = (1_000_000_000, 60);

pub struct Unibus {
    wires: Vec<Wire>,
    /// The I/O board.
    pub board: Chip,
    /// What each end was last given, per wire: the interface first.
    given: Vec<[Option<Level>; 2]>,
    changed: [bool; 2],
    /// Board transitions made, for measuring the cost of the board.
    pub transitions: u64,
    /// Whether the board, asleep on its own clock, may skip its edges; see
    /// [`Chip::asleep`].
    pub sleep: bool,
    /// The interface's and the board's [`Chip::generation`]s as of the
    /// last exchange: while neither has moved, no wire has.
    seen: [u64; 2],
    /// Whether that skips the exchange. Off, every wire is carried every
    /// time, the slow way: [`crate::cable::FarEnd::unoptimised`].
    pub skip_unchanged: bool,
    /// The Chaosnet cable, with the Chaosnet server on it, if plugged in:
    /// [`Unibus::plug`]. Not in a checkpoint, like the drive: a resume
    /// brings it up fresh.
    chaos: Option<crate::chaos::cable::OnCable>,
    /// When the ether next moves of its own accord.
    chaos_next: Option<u64>,
    /// The transceiver's nets, for plugging a cable in later.
    chaos_nets: crate::chaos::cable::Nets,
    /// The keyboard on the board's keyboard cable, if one is plugged in:
    /// [`Unibus::plug_keyboard`]. Until it is, the far end holds the pair
    /// at [`IDLE_KEYBOARD`]; plugged in, the keyboard holds it there
    /// itself between words.
    keyboard: Option<crate::terminal::cable::OnCable>,
    keyboard_nets: Option<crate::terminal::cable::Nets>,
    /// The mouse on the board's mouse lines, if one is plugged in:
    /// [`Unibus::plug_mouse`]. Its steps are timed, like the ether's
    /// edges, so it has a next time of its own.
    mouse: Option<crate::terminal::cable::MouseOnCable>,
    mouse_nets: Option<crate::terminal::cable::MouseNets>,
    mouse_next: Option<u64>,
    /// The far end of the serial port's null-modem cable on J9, if one is
    /// on it: [`Unibus::plug_serial`], which only `muir --serial` asks
    /// for. Its frames are timed, like the ether's edges, so it has a next
    /// time of its own. Without it J9 is empty, which is what a CADR with
    /// nothing plugged into its serial port is.
    serial: Option<crate::serial::OnCable>,
    serial_nets: Option<crate::serial::Nets>,
    serial_next: Option<u64>,
    /// The 74LS14's input on the mains network at C20, and how many
    /// toggles of [`MAINS_PERIOD`] have been given to it. The line is high
    /// at zero, as the oscillators are.
    mains: Option<NetId>,
    mains_toggles: u64,
}

impl Unibus {
    /// Builds the I/O board on the backplane beside the interface netlist
    /// `busint`: powered, its Chaosnet address switches set to
    /// `chaos_address`, the cable's inputs held at idle until an ether is
    /// plugged in, and settled. The reset comes over `-UB INIT` from the
    /// interface.
    pub fn new(busint: &Netlist, io: &Netlist, powered_at: u64, chaos_address: u16) -> Unibus {
        let mut wires = Vec::new();
        for (a, b) in wire_pairs() {
            let interface = find(busint, &a).unwrap_or_else(|| panic!("the interface has no {a}"));
            let board = find(io, &b).unwrap_or_else(|| panic!("the I/O board has no {b}"));
            wires.push(Wire { interface, board });
        }
        let mut board = Chip::new_unclocked(io);
        board.power_on();
        for w in &wires {
            board.pull_up(w.board);
        }
        // The boot line, terminated on the bus and pressed by nobody.
        board.pull_up(find(io, "-BOOT*").expect("the I/O board has no -BOOT*"));
        for &(name, level) in IDLE_CHAOSNET.iter().chain(IDLE_KEYBOARD) {
            board.drive(
                find(io, name).unwrap_or_else(|| panic!("the I/O board has no {name}")),
                level,
            );
        }
        for (net, level) in chaosnet_speed_jumpers(io) {
            board.drive(net, level);
        }
        for (reference, kind, image) in crate::xbus::roms(io) {
            board.load_rom(reference, kind, image);
        }
        for (reference, closed) in crate::chaos::interface::switches(chaos_address) {
            if io.parts.iter().any(|p| p.reference == reference && p.kind == "SWITCH") {
                board.set_switches(reference, closed);
            }
        }
        // Reset at power-on, as the memory boards are over `-XBUS INIT`
        // and as the machine's power-on reset would over `-UB INIT`, which
        // the processor's RC network makes and `chip` does not model.
        // Without it the keyboard receiver's busy flop came up busy, the
        // shift register filled with twenty-four ones off the idle line,
        // and `KBD READY` was up 196 µs after power-on for the microcode
        // to read at `(LOC 6)` as a key typed during the boot.
        let chaos_nets = crate::chaos::cable::Nets::of(io);
        let keyboard_nets = crate::terminal::cable::Nets::of(io);
        let mouse_nets = crate::terminal::cable::MouseNets::of(io);
        let serial_nets = crate::serial::Nets::of(io);
        let init = find(io, "-INIT*").expect("the I/O board has no -INIT*");
        board.drive(init, Level::Low);
        board.settle_all();
        // The crystal starts at the board's first transition: power-on.
        board.transition(powered_at);
        board.pull_up(init);
        board.settle();
        let given = vec![[None; 2]; wires.len()];
        Unibus {
            wires,
            board,
            given,
            changed: [false; 2],
            transitions: 0,
            sleep: true,
            seen: [u64::MAX; 2],
            skip_unchanged: true,
            chaos: None,
            chaos_next: None,
            chaos_nets,
            keyboard: None,
            keyboard_nets,
            mouse: None,
            mouse_nets,
            mouse_next: None,
            serial: None,
            serial_nets,
            serial_next: None,
            mains: find(io, "@0C20,p8").or_else(|| find(io, "@0C20,p7")),
            mains_toggles: 0,
        }
    }

    /// The mains at `now`: every toggle due is taken and the line left
    /// where they leave it. The board is transitioned by the caller, as it
    /// is for the mouse and the ether.
    fn apply_mains(&mut self, now: u64) {
        let Some(net) = self.mains else { return };
        while crate::chip::toggle_at(MAINS_PERIOD, self.mains_toggles) <= now {
            self.mains_toggles += 1;
        }
        self.board.drive(net, Level::from(self.mains_toggles % 2 == 1));
    }

    /// When the mains next moves.
    fn mains_next(&self) -> Option<u64> {
        self.mains.map(|_| crate::chip::toggle_at(MAINS_PERIOD, self.mains_toggles))
    }

    /// Puts an ether on the board's Chaosnet transceiver at `now`, in
    /// place of the idle cable the far end holds it at.
    pub fn plug(&mut self, ether: crate::chaos::ether::Ether, now: u64) {
        let mut cable = crate::chaos::cable::OnCable::from_nets(self.chaos_nets, ether);
        cable.apply(&mut self.board, now);
        self.chaos_next = cable.next_change(now);
        self.chaos = Some(cable);
    }

    /// The ether on the cable, if one is plugged in.
    pub fn ether(&self) -> Option<&crate::chaos::ether::Ether> {
        self.chaos.as_ref().map(|c| &c.ether)
    }

    /// Puts a keyboard on the board's keyboard cable at `now`, in place of
    /// the idle pair the far end holds. It has nothing to say until a
    /// word is given to it, [`Unibus::keyboard`].
    pub fn plug_keyboard(&mut self, now: u64) {
        if let Some(mut cable) = self.keyboard_nets.map(crate::terminal::cable::OnCable::new) {
            cable.apply(&mut self.board, now);
            self.keyboard = Some(cable);
        }
    }

    /// The keyboard on the cable, to give words to.
    pub fn keyboard(&mut self) -> Option<&mut crate::terminal::cable::OnCable> {
        self.keyboard.as_mut()
    }

    /// Lets the keyboard see the board's clock at `now` and move its line
    /// on the edge; the board transitions if that moved a net.
    fn apply_keyboard(&mut self, now: u64) {
        if let Some(k) = self.keyboard.as_mut() {
            k.apply(&mut self.board, now);
        }
    }

    /// Puts a mouse on the board's mouse lines at `now`, at rest. It has
    /// nothing to say until motion is given to it, [`Unibus::mouse`].
    pub fn plug_mouse(&mut self, now: u64) {
        if let Some(mut m) = self.mouse_nets.map(crate::terminal::cable::MouseOnCable::new) {
            m.apply(&mut self.board, now);
            self.mouse_next = m.next_change(now);
            self.mouse = Some(m);
        }
    }

    /// The mouse on the cable, to give motion and buttons to.
    pub fn mouse(&mut self) -> Option<&mut crate::terminal::cable::MouseOnCable> {
        self.mouse.as_mut()
    }

    /// Lets the mouse step its lines if a step is due at `now`; the board
    /// transitions if that moved a net.
    fn apply_mouse(&mut self, now: u64) {
        if let Some(m) = self.mouse.as_mut() {
            m.apply(&mut self.board, now);
            self.mouse_next = m.next_change(now);
        }
    }

    /// Puts the far end of a null-modem cable on the board's J9 at `now`,
    /// with nothing connected to the other side of it: it holds the port's
    /// three control inputs off, which is where the MC1489's open inputs
    /// sit with J9 empty, until [`crate::serial::Endpoint`] plugs a device
    /// in.
    ///
    /// Only `muir --serial` asks for one. A run without it has no far end
    /// here and pays nothing for the port at all: no rate to read off the
    /// 2651 and no wires to look at on every transition of the board.
    pub fn plug_serial(&mut self, now: u64) {
        let Some(nets) = self.serial_nets else { return };
        // The rate and the frame are the port's own, read off the 2651
        // here and at every step after; these are what the mode registers
        // hold from `RESET` and last until the machine writes them.
        let mut far = crate::serial::OnCable::new(nets, 0, crate::serial::Framing::of(0));
        far.follow(&self.board);
        far.apply(&mut self.board, now);
        self.serial_next = far.next_change(now);
        self.serial = Some(far);
    }

    /// The far end of the serial cable, to give characters to and take
    /// them from.
    pub fn serial(&mut self) -> Option<&mut crate::serial::OnCable> {
        self.serial.as_mut()
    }

    /// Lets the far end of the serial cable take the port's rate, move its
    /// line if a bit is due at `now` and sample the board's; the board
    /// transitions if that moved a net.
    fn apply_serial(&mut self, now: u64) {
        if let Some(s) = self.serial.as_mut() {
            s.follow(&self.board);
            s.apply(&mut self.board, now);
            self.serial_next = s.next_change(now);
        }
    }

    /// Lets the ether see what the board has on the cable at `now` and
    /// answer; the board transitions if that moved a net.
    fn apply_chaos(&mut self, now: u64) {
        if let Some(c) = self.chaos.as_mut() {
            c.apply(&mut self.board, now);
            self.chaos_next = c.next_change(now);
        }
    }

    /// Carries every wire across once: the wired-AND of what the interface
    /// and the board hold, given to both. Returns whether anything changed;
    /// [`Unibus::interface_changed`] says whether on the interface.
    pub fn exchange(&mut self, interface: &mut Chip) -> bool {
        self.changed = [false; 2];
        let gens = [interface.generation(), self.board.generation()];
        if self.skip_unchanged && gens == self.seen {
            return false;
        }
        let since = self.seen;
        self.seen = gens;
        for (k, w) in self.wires.iter().enumerate() {
            // Only a wire an end's drivers have moved on since the last
            // exchange, by the boards' own stamps; the first exchange reads
            // every wire.
            if self.skip_unchanged
                && since != [u64::MAX, u64::MAX]
                && interface.net_stamp(w.interface) <= since[0]
                && self.board.net_stamp(w.board) <= since[1]
            {
                continue;
            }
            // Open collector: a low anywhere is a low everywhere, and only
            // a driver counts, not what a board was given.
            let (mut low, mut high) = (false, false);
            for (level, strong) in
                [interface.board_level(w.interface), self.board.board_level(w.board)]
            {
                if strong {
                    match level {
                        Level::Low => low = true,
                        Level::High => high = true,
                        _ => {}
                    }
                }
            }
            let level = if low {
                Some(Level::Low)
            } else if high {
                Some(Level::High)
            } else {
                None
            };
            for (end, (chip, net)) in
                [(&mut *interface, w.interface), (&mut self.board, w.board)].into_iter().enumerate()
            {
                if self.given[k][end] != level {
                    match level {
                        Some(l) => chip.drive(net, l),
                        None => chip.pull_up(net),
                    }
                    self.given[k][end] = level;
                    self.changed[end] = true;
                }
            }
        }
        self.changed.iter().any(|&c| c)
    }

    pub fn interface_changed(&self) -> bool {
        self.changed[0]
    }

    /// Lets the board settle if the last exchange changed a net on it.
    pub fn transition_changed(&mut self, now: u64) {
        if self.changed[1] {
            self.board.transition(now);
            self.transitions += 1;
            self.apply_chaos(now);
            self.apply_keyboard(now);
            self.apply_mouse(now);
            self.apply_serial(now);
        }
    }

    fn due(&self) -> Option<u64> {
        if self.sleep && self.board.asleep() {
            self.board.next_wake()
        } else {
            self.board.next_tap()
        }
    }

    /// When the board, or the ether on its cable, will next do something
    /// of its own accord.
    pub fn next_tap(&self) -> Option<u64> {
        [self.due(), self.chaos_next, self.mouse_next, self.serial_next, self.mains_next()]
            .into_iter()
            .flatten()
            .min()
    }

    /// Lets the board have every event due by `now`, each at its own time.
    /// One transition for all of them would fold the oscillator edges
    /// passed into one, which a sleeping board can afford and a running
    /// counter cannot: the microsecond clock lost seven MCLK edges at the
    /// button, where the processor is ticked without the far end, and
    /// answered 438 ns late for the rest of the run.
    pub fn transition_due(&mut self, now: u64) {
        let mut n = 0;
        while let Some(t) = self.due()
            && t <= now
        {
            // The ether's edges that come before this tap reach the board
            // first, each at its own time.
            while let Some(e) = self.chaos_next
                && e < t
            {
                self.apply_chaos(e);
                if self.chaos_next == Some(e) {
                    break;
                }
            }
            // And the mouse's steps due before it.
            while let Some(e) = self.mouse_next
                && e < t
            {
                self.apply_mouse(e);
                if self.mouse_next == Some(e) {
                    break;
                }
            }
            // And the serial cable's bits.
            while let Some(e) = self.serial_next
                && e < t
            {
                self.apply_serial(e);
                if self.serial_next == Some(e) {
                    break;
                }
            }
            // The mains' own edges, each with a transition of its own: the
            // counter behind `CLOCK` is clocked by them and would lose any
            // that were folded into the next tap.
            while let Some(e) = self.mains_next()
                && e < t
            {
                self.apply_mains(e);
                self.board.transition(e);
                self.transitions += 1;
            }
            self.board.transition(t);
            self.transitions += 1;
            self.apply_chaos(t);
            self.apply_keyboard(t);
            self.apply_mouse(t);
            self.apply_serial(t);
            n += 1;
            assert!(n < 1_000_000, "the I/O board's events never run out at {now}");
        }
        while let Some(e) = self.chaos_next
            && e <= now
        {
            self.apply_chaos(e);
            if self.chaos_next == Some(e) {
                break;
            }
        }
        while let Some(e) = self.mouse_next
            && e <= now
        {
            self.apply_mouse(e);
            if self.mouse_next == Some(e) {
                break;
            }
        }
        while let Some(e) = self.serial_next
            && e <= now
        {
            self.apply_serial(e);
            if self.serial_next == Some(e) {
                break;
            }
        }
        while let Some(e) = self.mains_next()
            && e <= now
        {
            self.apply_mains(e);
            self.board.transition(e);
            self.transitions += 1;
        }
    }

    /// Whether the board has a delay-line tap in flight.
    pub fn taps_pending(&self) -> bool {
        self.board.taps_pending()
    }

    pub fn save(&self, w: &mut impl std::io::Write) -> std::io::Result<()> {
        self.board.save(w)
    }

    /// What each end of the Unibus was last given, and the mains network's
    /// phase, into a checkpoint.  The wires are carried for the reason
    /// [`crate::cable::Cables::save`] gives; the mains is here because it
    /// cannot be worked out from the time --- the count is what says where
    /// the line is, and a board whose count started at zero would be owed
    /// every half cycle of the 60 Hz line since power-on and would take
    /// the first of them at time 0, before the resume's own instant.
    ///
    /// Not here, as they are not on the other two engines: the Chaosnet
    /// cable and the mouse, which a resume plugs in afresh and which say
    /// then when they next move.
    pub fn save_exchange(&self, w: &mut crate::checkpoint::Writer) {
        w.u64(self.given.len() as u64);
        for g in &self.given {
            for &e in g {
                w.opt(e, crate::checkpoint::Writer::level);
            }
        }
        for &c in &self.changed {
            w.bool(c);
        }
        w.u64(self.mains_toggles);
    }

    /// Back from a checkpoint, onto a Unibus with the same board on it.
    /// Both ends are marked unread, as [`crate::xbus::Xbus::load_exchange`]
    /// marks them and for the same reason.
    pub fn load_exchange(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        let n = r.u64()? as usize;
        if n != self.given.len() {
            return Err(crate::checkpoint::bad(format!(
                "{n} wires on the Unibus, and this one has {}",
                self.given.len()
            )));
        }
        for g in self.given.iter_mut() {
            for e in g.iter_mut() {
                *e = r.opt(crate::checkpoint::Reader::level)?;
            }
        }
        for c in self.changed.iter_mut() {
            *c = r.bool()?;
        }
        self.mains_toggles = r.u64()?;
        self.seen = [u64::MAX; 2];
        Ok(())
    }

    pub fn load(&mut self, r: &mut impl std::io::Read) -> std::io::Result<()> {
        if let Some(c) = self.chaos.as_mut() {
            c.reattach();
        }
        self.board.load(r)
    }
}

/// A net by name, quoted in the netlist or not.
fn find(n: &Netlist, name: &str) -> Option<NetId> {
    n.by_name_id(name).or_else(|| n.by_name_id(&format!("'{name}'")))
}

/// The Unibus wires a slave board has, as its drawings name them.
pub fn wire_names() -> Vec<String> {
    let mut w: Vec<String> = (1..18).map(|b| format!("-A{b}*")).collect();
    w.extend((0..16).map(|b| format!("-D{b}*")));
    w.extend(
        [
            "-C1*", "-MSYN*", "-SSYN*", "-BBSY*", "-INIT*", "-INTR*", "-SACK*", "-BR*", "BG.IN*",
            "BG.OUT*", "-BOOT*",
        ]
        .map(String::from),
    );
    w
}

pub struct UnibusMaster<'a> {
    pub chip: Chip,
    pub now: u64,
    /// The Chaosnet cable, if one is plugged in: [`UnibusMaster::plug_chaos`].
    pub chaos: Option<crate::chaos::cable::OnCable>,
    netlist: &'a Netlist,
    addr: Vec<NetId>,
    data: Vec<NetId>,
    c1: NetId,
    msyn: NetId,
    ssyn: NetId,
}

impl<'a> UnibusMaster<'a> {
    /// How long a slave may take before the harness gives up on a cycle;
    /// the interface's own timeout is under 6 µs
    /// ([`crate::busint::nxm_timeout_at`]).
    pub const TIMEOUT_NS: u64 = 20_000;

    /// Powers a board, holds every Unibus wire it has up and each net in
    /// `held` at the level given --- what a section of the board that is
    /// not in the netlist would hold its outputs at --- resets it over
    /// `-INIT*`, and lets it run `settle_ns`.
    pub fn new(board: &'a Netlist, settle_ns: u64, held: &[(&str, Level)]) -> UnibusMaster<'a> {
        let mut chip = Chip::new_unclocked(board);
        chip.power_on();
        for name in wire_names() {
            if let Some(net) = find(board, &name) {
                chip.pull_up(net);
            }
        }
        let net = |name: &str| find(board, name).unwrap_or_else(|| panic!("no net {name}"));
        // The grant coming in is not a pulled-up line but the arbiter's, or
        // the previous device's, output: low, no grant. The board terminates
        // it to ground itself.
        if let Some(bg) = find(board, "BG.IN*") {
            chip.drive(bg, Level::Low);
        }
        for &(name, level) in held {
            chip.drive(net(name), level);
        }
        for (net, level) in chaosnet_speed_jumpers(board) {
            chip.drive(net, level);
        }
        for (reference, kind, image) in crate::xbus::roms(board) {
            chip.load_rom(reference, kind, image);
        }
        let init = net("-INIT*");
        chip.drive(init, Level::Low);
        chip.settle_all();
        chip.transition(0);
        chip.pull_up(init);
        let mut m = UnibusMaster {
            chip,
            now: 0,
            chaos: None,
            netlist: board,
            addr: (1..18).map(|b| net(&format!("-A{b}*"))).collect(),
            data: (0..16).map(|b| net(&format!("-D{b}*"))).collect(),
            c1: net("-C1*"),
            msyn: net("-MSYN*"),
            ssyn: net("-SSYN*"),
        };
        m.run(settle_ns);
        m
    }

    /// A net on the board, by name.
    pub fn net(&self, name: &str) -> NetId {
        find(self.netlist, name).unwrap_or_else(|| panic!("no net {name}"))
    }

    /// A net's level, by name.
    pub fn level(&self, name: &str) -> Level {
        self.chip.net(self.net(name))
    }

    /// Advances to `until`, stopping at every tap and oscillator edge.
    pub fn run(&mut self, until: u64) {
        while self.now < until {
            let cable = self.chaos.as_ref().and_then(|c| c.next_change(self.now));
            let next = [self.chip.next_tap(), cable]
                .into_iter()
                .flatten()
                .min()
                .map_or(until, |t| t.min(until));
            self.now = next.max(self.now);
            self.chip.transition(self.now);
            if let Some(c) = self.chaos.as_mut() {
                c.apply(&mut self.chip, self.now);
            }
            if next >= until {
                break;
            }
        }
    }

    /// Puts an ether on the board's Chaosnet transceiver, in place of the
    /// idle cable the far end holds it at.
    pub fn plug_chaos(&mut self, ether: crate::chaos::ether::Ether) {
        let mut cable = crate::chaos::cable::OnCable::new(self.netlist, ether);
        cable.apply(&mut self.chip, self.now);
        self.chaos = Some(cable);
    }

    fn put(&mut self, nets: &[NetId], word: u32) {
        for (k, &net) in nets.iter().enumerate() {
            if (word >> k) & 1 != 0 {
                self.chip.drive(net, Level::Low);
            } else {
                self.chip.pull_up(net);
            }
        }
    }

    /// The word on the data lines, as the slave drives them.
    pub fn word(&self) -> u16 {
        !(self.chip.read(&self.data) as u16)
    }

    /// Whether the slave has answered: `-SSYN*` is down.
    pub fn answered(&self) -> bool {
        self.chip.net(self.ssyn) == Level::Low
    }

    /// One cycle at Unibus address `uaddr` from `now`: a read, or a write
    /// of `write`. Returns how long the slave took to answer `-MSYN*` with
    /// `-SSYN*`, and the word it put up for a read.
    pub fn cycle(&mut self, uaddr: u32, write: Option<u16>) -> (u64, u16) {
        self.try_cycle(uaddr, write).unwrap_or_else(|| panic!("the board never answered {uaddr:o}"))
    }

    /// [`UnibusMaster::cycle`], with `None` for an address nothing on the
    /// board answers: the master gives up `TIMEOUT_NS` after `-MSYN*` and
    /// lets the lines go, as a master's timeout does.
    pub fn try_cycle(&mut self, uaddr: u32, write: Option<u16>) -> Option<(u64, u16)> {
        let addr = self.addr.clone();
        let data = self.data.clone();
        // `A1..A17` are the address's bits 1 to 17.
        self.put(&addr, uaddr >> 1);
        match write {
            Some(w) => {
                self.chip.drive(self.c1, Level::Low);
                self.put(&data, w as u32);
            }
            None => {
                self.chip.pull_up(self.c1);
                self.put(&data, 0);
            }
        }
        self.chip.transition(self.now);
        self.run(self.now + UNIBUS_ADDRESS_NS);
        let t0 = self.now;
        self.chip.drive(self.msyn, Level::Low);
        self.chip.transition(self.now);
        // To the board's next event each time, so that the answer is timed
        // when it happens: five nanoseconds at a time reported the counter's
        // low half at 315 ns where the board answers at 313, and the twin
        // carried the grid's number into the band.
        while !self.answered() {
            if self.now >= t0 + Self::TIMEOUT_NS {
                self.chip.pull_up(self.msyn);
                self.put(&addr, 0);
                self.put(&data, 0);
                self.chip.pull_up(self.c1);
                self.chip.transition(self.now);
                self.run(self.now + 200);
                return None;
            }
            let next = self.chip.next_tap().map_or(self.now + 5, |t| t.max(self.now + 1));
            self.run(next);
        }
        let (took, word) = (self.now - t0, self.word());
        self.run(self.now + UNIBUS_STROBE_NS);
        self.chip.pull_up(self.msyn);
        self.chip.transition(self.now);
        let t1 = self.now;
        while self.answered() {
            assert!(
                self.now < t1 + Self::TIMEOUT_NS,
                "the board never lifted -SSYN* after {uaddr:o}"
            );
            let next = self.chip.next_tap().map_or(self.now + 5, |t| t.max(self.now + 1));
            self.run(next);
        }
        self.put(&addr, 0);
        self.put(&data, 0);
        self.chip.pull_up(self.c1);
        self.chip.transition(self.now);
        self.run(self.now + 200);
        Some((took, word))
    }
}
