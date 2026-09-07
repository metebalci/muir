// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! One interface, several implementations at different fidelity levels.
//!
//! Every engine agrees on the microcycle as the unit of progress and on
//! [`Machine`] as the state.  They differ in how much of the hardware they
//! compute on the way.

use crate::machine::{Halt, Machine};

pub trait Engine {
    /// The boot button: `-BOOT` presets `RUN`, and `-RESET` with it clears
    /// the console's registers, `PROMDISABLE` among them, so the machine
    /// starts from the boot PROM at the bottom of the control store.  It is
    /// what starts a CADR --- the machine comes up with its clock stopped
    /// --- and muir presses it for the engine at the start of a run unless
    /// `--no-auto-boot` says not to, where the prompt's `boot` presses it
    /// instead.
    fn boot(&mut self);

    /// Runs one microcycle.
    fn step(&mut self) -> Result<(), Halt>;

    /// Where the next microinstruction will be fetched from.
    fn pc(&self) -> u16;

    /// What the cpu drives onto `SPY<15:0>` while `-DBREAD` is low with
    /// `EADR<3:0>` at `eadr`: one of the sixteen diagnostic registers,
    /// [`crate::spy::IR_LOW`] to [`crate::spy::STAT_HIGH`], as the console
    /// reads them.  Register 3 has no read select and reads as the open
    /// bus, [`crate::spy::OPEN_READ`].
    ///
    /// The values are the machine's as it stands between two microcycles,
    /// which is what a halted machine shows a console and what a read in
    /// flight sees at the microcycle boundary the interface answers in.
    fn spy_read(&self, eadr: u8) -> u16;

    /// Loads one of the console's registers, as the trailing edge of
    /// `-DBWRITE` does.  The registers are [`Machine`]'s, so this is
    /// [`Machine::spy_write`] for every engine; it is here so that a console
    /// has one interface to hold.
    fn spy_write(&mut self, eadr: u8, v: u16) {
        self.machine_mut().spy_write(eadr, v);
    }

    fn machine(&self) -> &Machine;
    fn machine_mut(&mut self) -> &mut Machine;

    /// The engine and its machine into a checkpoint, [`crate::checkpoint`]:
    /// the machine first, then the engine's own state.
    fn save(&self, w: &mut crate::checkpoint::Writer);

    /// Back from a checkpoint, into an engine built and booted as the flags
    /// say: the pack under it and the Chaosnet on it stay as they are.
    fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()>;

    /// Runs until it halts or `limit` microcycles have passed.  Returns the
    /// number run.
    fn run(&mut self, limit: u64) -> (u64, Option<Halt>) {
        for n in 0..limit {
            if let Err(h) = self.step() {
                return (n, Some(h));
            }
        }
        (limit, None)
    }
}
