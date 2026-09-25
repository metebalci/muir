// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! QUUX's memory port (contract Q6, revision 7), in place of the CADR's bus
//! interface ([`crate::busint`]), as `rtl` times it.
//!
//! The processor's cycle goes one of two ways, by its physical address:
//!
//! - **Main memory**, through the cache ([`crate::cache`], always fitted on
//!   QUUX) to the memory controller. A read hit is answered after the
//!   cache's hit time; a miss fills its line in [`MemoryTiming::read_ns`]
//!   and a write takes [`MemoryTiming::write_ns`], one operation at a time,
//!   the write buffer acknowledging a write after the hit time and running
//!   it behind the processor. No memory boards, no Xbus setup or deskew, no
//!   refresh. The nominal timing is a floor: a board slower on an access
//!   waits, muir answers at it.
//! - **The Xbus**, which holds only devices, never cached: the display,
//!   block-disk, the feature and register page. A device answers as an Xbus
//!   slave does on the CADR --- [`busint::SETUP_NS`] after the request, a
//!   read deskewed [`busint::XBUS_ACK_NS`] more --- and an address nothing
//!   answers, past main memory's end or in the old Unibus window among
//!   them, times out as the CADR's does ([`busint::nxm_timeout_at`]).
//!
//! There is nothing to arbitrate: the processor is the only requester in
//! muir. Block-disk moves its words at START, which its contract allows,
//! and invalidates the cache ([`crate::machine::Machine::dma_written`]).
//!
//! The words themselves come from [`crate::machine::Machine`], as with the
//! bus interface; the port says only when.

use crate::busint::{self, Ack, Responder};
use crate::cache::{Cache, CacheConfig, MemoryTiming};
use crate::clock::TimingModel;

/// Where the port's cycle is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Idle,
    /// Requested, to be taken at the next edge.
    Requested,
    /// Taken: answered at `answered`, acknowledged at `ack`.
    Granted {
        ack: u64,
        answered: u64,
        timed_out: bool,
    },
    /// Acknowledged, until the processor lets the request go.
    Acked {
        at: u64,
        answered: u64,
        timed_out: bool,
    },
}

#[derive(Clone, Debug)]
pub struct MemoryPort {
    state: State,
    write: bool,
    /// The physical address of the cycle, for the cache.
    addr: u32,
    /// The cycle is main memory's: no Xbus time between the answer and the
    /// data paths.
    memory: bool,
    cache: Cache,
    timing: MemoryTiming,
    /// When main memory is free for its next operation.
    memory_free_at: u64,
    /// When the write buffer is free again.
    buffer_free_at: u64,
    /// Whose clock the timeout's oscillator is measured on: the engine's,
    /// [`MemoryPort::keep_timing_model`]. Not in a checkpoint.
    model: TimingModel,
}

impl Default for MemoryPort {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryPort {
    /// QUUX's port: the cache at its shape, main memory at the nominal
    /// timing.
    pub fn new() -> MemoryPort {
        MemoryPort {
            state: State::Idle,
            write: false,
            addr: 0,
            memory: false,
            cache: Cache::new(CacheConfig::QUUX),
            timing: MemoryTiming::NOMINAL,
            memory_free_at: 0,
            buffer_free_at: 0,
            model: TimingModel::Sync { cycle_ticks: 4, ilong_ticks: 0 },
        }
    }

    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    /// Another shape of cache, before the machine runs: `--cache`.
    pub fn set_cache(&mut self, config: CacheConfig) {
        self.cache = Cache::new(config);
    }

    pub fn memory_timing(&self) -> MemoryTiming {
        self.timing
    }

    /// Other figures for main memory, before the machine runs:
    /// `--memory-timing`.
    pub fn set_memory_timing(&mut self, timing: MemoryTiming) {
        self.timing = timing;
    }

    /// Drops everything the cache holds: main memory was written by
    /// something else, the disk.
    pub fn invalidate_cache(&mut self) {
        self.cache.invalidate();
    }

    pub fn keep_timing_model(&mut self, model: TimingModel) {
        self.model = model;
    }

    pub fn timing_model(&self) -> TimingModel {
        self.model
    }

    /// The processor asks for a cycle at physical address `phys`.
    pub fn request_at(&mut self, write: bool, phys: u32) {
        debug_assert_eq!(self.state, State::Idle, "a cycle is already running");
        self.state = State::Requested;
        self.write = write;
        self.addr = phys;
        self.memory = false;
    }

    /// An edge of the processor's clock: a cycle requested is taken here.
    pub fn mclk_edge(&mut self, now: u64, responder: Responder) {
        if self.state != State::Requested {
            return;
        }
        self.memory = matches!(responder, Responder::Memory(_));
        let (at, timed_out) = if self.memory {
            (self.memory_cycle(now), false)
        } else if responder == Responder::Device {
            let answered = now + busint::SETUP_NS + busint::IDEAL_DEVICE_NS;
            let ack = if self.write { answered } else { answered + busint::XBUS_ACK_NS };
            self.state = State::Granted { ack, answered, timed_out: false };
            return;
        } else {
            (self.model.free_running(busint::nxm_timeout_at(now)), true)
        };
        self.state = State::Granted { ack: at, answered: at, timed_out };
    }

    /// Main memory's answer to the cycle taken at `now`: a hit after the
    /// hit time; a miss or a write when main memory has done it, a buffered
    /// write after the hit time or when the buffer is free.
    fn memory_cycle(&mut self, now: u64) -> u64 {
        let hit_ns = self.cache.config.hit_ns;
        if !self.write && self.cache.read(self.addr) {
            return now + hit_ns;
        }
        let start = now.max(self.memory_free_at);
        let done = start + if self.write { self.timing.write_ns } else { self.timing.read_ns };
        self.memory_free_at = done;
        if self.write && self.cache.config.write_buffer {
            let at = (now + hit_ns).max(self.buffer_free_at);
            self.buffer_free_at = done;
            at
        } else {
            done
        }
    }

    /// Advances to `now` and reports the acknowledgement if it has come.
    pub fn poll(&mut self, now: u64, responder: Responder) -> Option<Ack> {
        if let State::Granted { ack, answered, timed_out } = self.state
            && now >= ack
        {
            self.state = State::Acked { at: ack, answered, timed_out };
        }
        match self.state {
            State::Acked { at, answered, timed_out } => Some(Ack {
                timed_out,
                responder,
                at,
                loadmd_at: at,
                answered_at: answered,
                cached: self.memory,
            }),
            _ => None,
        }
    }

    pub fn granted(&self) -> bool {
        matches!(self.state, State::Granted { .. } | State::Acked { .. })
    }

    pub fn busy(&self) -> bool {
        self.state != State::Idle
    }

    pub fn ack_at(&self) -> Option<u64> {
        match self.state {
            State::Granted { ack, .. } => Some(ack),
            State::Acked { at, .. } => Some(at),
            _ => None,
        }
    }

    pub fn answered_at(&self) -> Option<u64> {
        match self.state {
            State::Granted { answered, .. } | State::Acked { answered, .. } => Some(answered),
            _ => None,
        }
    }

    /// The processor lets the request go.
    pub fn finish(&mut self) {
        self.state = State::Idle;
    }

    pub fn save(&self, w: &mut crate::checkpoint::Writer) {
        let MemoryPort {
            state,
            write,
            addr,
            memory,
            cache,
            timing,
            memory_free_at,
            buffer_free_at,
            model: _,
        } = self;
        match *state {
            State::Idle => w.u8(0),
            State::Requested => w.u8(1),
            State::Granted { ack, answered, timed_out } => {
                w.u8(2);
                w.u64(ack);
                w.u64(answered);
                w.bool(timed_out);
            }
            State::Acked { at, answered, timed_out } => {
                w.u8(3);
                w.u64(at);
                w.u64(answered);
                w.bool(timed_out);
            }
        }
        w.bool(*write);
        w.u32(*addr);
        w.bool(*memory);
        cache.save(w);
        w.u64(timing.read_ns);
        w.u64(timing.write_ns);
        w.u64(*memory_free_at);
        w.u64(*buffer_free_at);
    }

    pub fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        self.state = match r.u8()? {
            0 => State::Idle,
            1 => State::Requested,
            k @ (2 | 3) => {
                let (at, answered, timed_out) = (r.u64()?, r.u64()?, r.bool()?);
                if k == 2 {
                    State::Granted { ack: at, answered, timed_out }
                } else {
                    State::Acked { at, answered, timed_out }
                }
            }
            k => return Err(crate::checkpoint::bad(format!("memory port state {k}"))),
        };
        self.write = r.bool()?;
        self.addr = r.u32()?;
        self.memory = r.bool()?;
        self.cache = Cache::load(r)?;
        self.timing = MemoryTiming { read_ns: r.u64()?, write_ns: r.u64()? };
        self.memory_free_at = r.u64()?;
        self.buffer_free_at = r.u64()?;
        Ok(())
    }
}
