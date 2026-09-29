// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The profile harness's account of time (`examples/profile.rs`): a span
//! of a run as its microcycles, its memory time and its time in all, side
//! by side, since a microcycle count alone leaves the memory out.
//!
//! On `rtl` a wait or a hang for the memory advances the clock without
//! running a microcycle: the microcycle runs once, after it
//! (`Rtl::stall_for`). So the time is the microcycles' own periods plus
//! the time stalled, and on a machine whose microcycle has one period,
//! QUUX's, that is the period times the microcycles plus the time stalled.
//! `micro` has no memory to wait on and charges a fixed time for every
//! memory cycle instead (`Micro::memory_cycle_ns`); that is a stand-in,
//! not a measurement, and the line says so. `tests/profile_time.rs` holds
//! the line to the engines' clocks.

use muir::engine::Engine;
use muir::micro::Micro;
use muir::rtl::Rtl;

/// Where a span's memory time comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Memory {
    /// `rtl`'s waits and hangs, measured (`Rtl::stalled_ns`).
    Stalled,
    /// `micro`'s fixed charge of `each_ns` a memory cycle.
    Charged { each_ns: u64 },
}

/// A span of a run: the engine's clock and counts at its end less those at
/// its start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// Microcycles, `Machine::cycles`: executed and inhibited, and no
    /// stall.
    pub microcycles: u64,
    /// Of [`Span::ns`], the memory's: stalled, or charged.
    pub memory_ns: u64,
    /// Of [`Span::ns`], halted by the console: neither a microcycle nor a
    /// stall. A profile's workloads never halt.
    pub halted_ns: u64,
    /// The machine's clock: the time in all.
    pub ns: u64,
    pub memory: Memory,
}

impl Span {
    /// `rtl`'s clock and counts so far.
    pub fn of_rtl(e: &Rtl) -> Span {
        Span {
            microcycles: e.machine().cycles,
            memory_ns: e.stalled_ns(),
            halted_ns: e.halted_ns(),
            ns: e.ns(),
            memory: Memory::Stalled,
        }
    }

    /// `micro`'s clock and counts so far.
    pub fn of_micro(e: &Micro) -> Span {
        let each_ns = e.memory_cycle_ns;
        Span {
            microcycles: e.machine().cycles,
            memory_ns: e.memory_cycles() * each_ns,
            halted_ns: 0,
            ns: e.machine().ns,
            memory: Memory::Charged { each_ns },
        }
    }

    /// This span less an earlier one of the same run.
    pub fn since(self, earlier: Span) -> Span {
        Span {
            microcycles: self.microcycles - earlier.microcycles,
            memory_ns: self.memory_ns - earlier.memory_ns,
            halted_ns: self.halted_ns - earlier.halted_ns,
            ns: self.ns - earlier.ns,
            memory: self.memory,
        }
    }

    /// Two spans of the same run, one after the other: the workloads'
    /// total.
    pub fn plus(self, other: Span) -> Span {
        Span {
            microcycles: self.microcycles + other.microcycles,
            memory_ns: self.memory_ns + other.memory_ns,
            halted_ns: self.halted_ns + other.halted_ns,
            ns: self.ns + other.ns,
            memory: self.memory,
        }
    }

    /// The microcycles' own periods: the clock less the memory's time and
    /// the time halted.
    pub fn microcycle_ns(&self) -> u64 {
        self.ns - self.memory_ns - self.halted_ns
    }

    /// The span in a line: the microcycles and their time, the memory's
    /// time, and the time in all, which is their sum, and the memory's
    /// share of the time in all.
    pub fn line(&self) -> String {
        let share = 100.0 * self.memory_ns as f64 / self.ns.max(1) as f64;
        let halted = if self.halted_ns > 0 {
            format!(" + halted {} ns", self.halted_ns)
        } else {
            String::new()
        };
        match self.memory {
            Memory::Stalled => format!(
                "time: {} microcycles in {} ns + stalled on memory {} ns{halted} = {} ns in all; stalled {share:.2}% of the time in all",
                self.microcycles,
                self.microcycle_ns(),
                self.memory_ns,
                self.ns
            ),
            Memory::Charged { each_ns } => format!(
                "time: {} microcycles in {} ns + memory charged {} ns ({} ns a memory cycle, fixed; micro does not stall){halted} = {} ns in all; memory charged {share:.2}% of the time in all",
                self.microcycles,
                self.microcycle_ns(),
                self.memory_ns,
                each_ns,
                self.ns
            ),
        }
    }
}
