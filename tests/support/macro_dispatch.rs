// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **Checkers for QUUX's fused return and operand address** (contract H8a,
//! revision 12, §6 items 4 and 5), outside the engines: they watch a run
//! microcycle by microcycle and count what breaks the contract, and the
//! engines never consult them.
//!
//! - **The rule of §3.3.** The microcycle after a fused return, the one an
//!   XCT-NEXT runs or a nopped one, must not write the location counter,
//!   M 31, INTERRUPT-CONTROL (whose byte mode chooses the halfword) or
//!   functional destinations 5 to 7, and, when the entry has the operand
//!   bit, PDL-INDEX, `A-LOCALP` or `M-AP`: the hardware has chosen the
//!   handler and the operand address by then. The instruction that ran
//!   there is decoded from the control store, independently of the
//!   engines' own decode ([`writes`]).
//! - **What the main loop would have done.** After that microcycle the
//!   handler the MACRO DISPATCH MEMORY names for the halfword the main
//!   loop's dispatch would take --- M 31 by the location counter's `<1>`,
//!   as stepped --- is the next to run, the micro stack's pointer is where
//!   the return left it, and, with the operand bit, PDL-INDEX holds
//!   `A-LOCALP` + delta or `M-AP` + 1 + delta read from A and M memory.
//! - **The base copies** equal A memory at the register's `<23:14>` and
//!   M memory at its `<28:24>`, fourteen bits, after every microcycle.
//!
//! [`Checked`] wraps an engine and runs the checker after every step.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use muir::engine::Engine;
use muir::isa::{Insn, Op};
use muir::machine::{Halt, Machine, macro_dispatch};
use muir::micro::Micro;
use muir::rtl::Rtl;

/// An engine that says which control-store address it executed in its last
/// step, `None` when that microcycle was nopped.
pub trait Executes: Engine {
    fn executed(&self) -> Option<u16>;
}

impl Executes for Micro {
    fn executed(&self) -> Option<u16> {
        Micro::executed(self)
    }
}

impl Executes for Rtl {
    fn executed(&self) -> Option<u16> {
        Rtl::executed(self)
    }
}

/// A write the microcycle after a fused return may not make (contract H8a
/// §3.3). The last three only when the entry has the operand bit.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Write {
    /// Functional destination 1, the location counter.
    Lc,
    /// M memory 31, `M-INST-BUFFER`.
    M31,
    /// Functional destination 2, INTERRUPT-CONTROL, whose `<29>` is the
    /// byte mode.
    InterruptControl,
    /// Functional destinations 5 to 7, the MACRO-DISPATCH register and the
    /// MACRO DISPATCH MEMORY.
    MacroDispatch,
    /// Functional destination 13, PDL-INDEX.
    PdlIndex,
    /// Functional destination 12, the PDL buffer at PDL-INDEX: the word is
    /// written in the next microcycle's write phase, at PDL-INDEX as it
    /// then stands (`PWIDX`, `Rtl::write_phase`), after the operand
    /// address has been loaded.
    PdlAtIndex,
    /// A memory at the register's `<23:14>`, `A-LOCALP`.
    Localp,
    /// M memory at the register's `<28:24>`, `M-AP`.
    Ap,
}

impl Write {
    /// Forbidden only when the entry has the operand bit.
    pub fn operand_only(self) -> bool {
        matches!(self, Write::PdlIndex | Write::PdlAtIndex | Write::Localp | Write::Ap)
    }
}

/// **The writes of [`Write`]'s kinds that instruction `i` makes**, with the
/// MACRO-DISPATCH register at `register` naming the bases. Decoded as
/// `mit/cadr/ir.bits` lays the word out: only the ALU and BYTE classes
/// write; `IR<25>` set is an A destination at `IR<23:14>`, and clear is an
/// M destination at `IR<18:14>`, which writes the shadowing A word too,
/// with the functional destination `IR<23:19>` (`IR<24>` in no decode).
/// Functional destinations 1 and 2 are the location counter and
/// INTERRUPT-CONTROL, 13 PDL-INDEX (`ir.bits`' `FUNCTIONAL DESTINATIONS`),
/// and 5 to 7 QUUX's from revision 12.
pub fn writes(i: Insn, register: u32) -> Vec<Write> {
    let mut w = Vec::new();
    if !matches!(i.op(), Op::Alu | Op::Byte) {
        return w;
    }
    let ir = i.raw();
    let localp = macro_dispatch::localp_address(register) as u64;
    let ap = macro_dispatch::ap_address(register) as u64;
    if ir >> 25 & 1 != 0 {
        if ir >> 14 & 0o1777 == localp {
            w.push(Write::Localp);
        }
        return w;
    }
    match ir >> 19 & 0o37 {
        0o1 => w.push(Write::Lc),
        0o2 => w.push(Write::InterruptControl),
        0o5..=0o7 => w.push(Write::MacroDispatch),
        0o12 => w.push(Write::PdlAtIndex),
        0o13 => w.push(Write::PdlIndex),
        _ => {}
    }
    let m = ir >> 14 & 0o37;
    if m == 0o31 {
        w.push(Write::M31);
    }
    if m == localp {
        w.push(Write::Localp);
    }
    if m == ap {
        w.push(Write::Ap);
    }
    w
}

/// Where a write was made: the return's address and the address of the
/// microcycle after it.
pub type Site = (Write, u16, u16);

/// What the checker has counted.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct Counts {
    /// Fused returns seen.
    pub fused: u64,
    /// Microcycles after a fused return that were nopped.
    pub nopped_after: u64,
    /// Fused returns whose entry had the operand bit and a LOCAL or ARG
    /// register, whose PDL-INDEX was checked.
    pub operand_loads: u64,
    /// Fused returns into one of [`OPERAND_OPCODES`] with a LOCAL or ARG
    /// register, whatever the entry: those the operand bit could serve.
    pub operand_candidates: u64,
    /// Forbidden writes in the microcycle after a fused return, by kind and
    /// site: the rule of §3.3 broken.
    pub violations: BTreeMap<Site, u64>,
    /// Writes of PDL-INDEX, `A-LOCALP` or `M-AP` there whose entry had no
    /// operand bit: allowed, and what a specialised handler with the bit
    /// would break.
    pub unarmed: BTreeMap<Site, u64>,
    /// The micro stack's pointer moved in the microcycle after.
    pub stack_moved: u64,
    /// The next microcycle was not the handler the main loop's dispatch
    /// would have reached.
    pub wrong_handler: u64,
    /// PDL-INDEX was not the operand address after the microcycle after.
    pub wrong_operand: u64,
    /// Microcycles after which a base copy differed from its memory.
    pub copies_differ: u64,
    /// The first problem found, said in words.
    pub first: Option<String>,
}

impl Counts {
    /// Everything that breaks the contract: zero on a run that keeps it.
    pub fn problems(&self) -> u64 {
        self.violations.values().sum::<u64>()
            + self.stack_moved
            + self.wrong_handler
            + self.wrong_operand
            + self.copies_differ
    }

    /// What was counted after `earlier`, a clone of these counts.
    pub fn since(&self, earlier: &Counts) -> Counts {
        let diff = |now: &BTreeMap<Site, u64>, then: &BTreeMap<Site, u64>| {
            now.iter()
                .map(|(k, &n)| (*k, n - then.get(k).copied().unwrap_or(0)))
                .filter(|&(_, n)| n > 0)
                .collect()
        };
        Counts {
            fused: self.fused - earlier.fused,
            nopped_after: self.nopped_after - earlier.nopped_after,
            operand_loads: self.operand_loads - earlier.operand_loads,
            operand_candidates: self.operand_candidates - earlier.operand_candidates,
            violations: diff(&self.violations, &earlier.violations),
            unarmed: diff(&self.unarmed, &earlier.unarmed),
            stack_moved: self.stack_moved - earlier.stack_moved,
            wrong_handler: self.wrong_handler - earlier.wrong_handler,
            wrong_operand: self.wrong_operand - earlier.wrong_operand,
            copies_differ: self.copies_differ - earlier.copies_differ,
            first: if earlier.first.is_none() { self.first.clone() } else { None },
        }
    }

    /// The counts in a few lines, with `label` naming a control-store
    /// address.
    pub fn report(&self, label: impl Fn(u16) -> String) -> String {
        let mut s = format!(
            "fused {}, nopped after {}, operand loads {}, LOCAL or ARG operands {}, violations {}, stack moved {}, wrong handler {}, wrong operand {}, copies differ {}",
            self.fused,
            self.nopped_after,
            self.operand_loads,
            self.operand_candidates,
            self.violations.values().sum::<u64>(),
            self.stack_moved,
            self.wrong_handler,
            self.wrong_operand,
            self.copies_differ
        );
        for (what, map) in [("violation", &self.violations), ("unarmed", &self.unarmed)] {
            for ((w, ret, after), n) in map {
                let _ = write!(
                    s,
                    "\n     {what} {w:?}: return at {ret:o} ({}), after it {after:o} ({}): {n}",
                    label(*ret),
                    label(*after)
                );
            }
        }
        if let Some(f) = &self.first {
            let _ = write!(s, "\n     first: {f}");
        }
        s
    }
}

/// The fused return the next microcycle follows.
#[derive(Clone, Copy)]
struct After {
    /// The return's address, or `u16::MAX` if the engine did not say.
    at: u16,
    /// The micro stack's pointer after it.
    spcptr: u8,
}

/// The checker's state between two microcycles.
#[derive(Clone, Default)]
pub struct Checker {
    cycles: u64,
    fused: u64,
    after: Option<After>,
    /// The handler the next microcycle must run.
    handler: Option<u16>,
    pub counts: Counts,
}

impl Checker {
    /// A checker for `m` as it stands: nothing it did before is counted.
    pub fn new(m: &Machine) -> Checker {
        Checker { cycles: m.cycles, fused: m.macro_dispatch.fused, ..Checker::default() }
    }

    fn problem(&mut self, what: impl FnOnce() -> String) {
        if self.counts.first.is_none() {
            self.counts.first = Some(what());
        }
    }

    /// After a step of `e`: nothing unless it completed a microcycle.
    pub fn after_step<E: Executes>(&mut self, e: &E) {
        let m = e.machine();
        if m.cycles == self.cycles {
            return;
        }
        self.cycles = m.cycles;
        if !m.geometry.macro_dispatch {
            return;
        }
        let d = &m.macro_dispatch;
        let base = macro_dispatch::BASE_BITS;
        let localp_at = macro_dispatch::localp_address(d.register);
        let ap_at = macro_dispatch::ap_address(d.register);
        let (localp, ap) = (m.amem[localp_at] & base, m.mmem[ap_at] & base);
        if (d.localp, d.ap) != (localp, ap) {
            self.counts.copies_differ += 1;
            let (cl, ca) = (d.localp, d.ap);
            self.problem(|| {
                format!(
                    "after microcycle {}: copies {cl:o} {ca:o}, memory {localp:o} {ap:o}",
                    m.cycles
                )
            });
        }
        if let Some(h) = self.handler.take()
            && e.executed() != Some(h)
        {
            self.counts.wrong_handler += 1;
            let ran = e.executed().map_or("nothing".into(), |pc| format!("{pc:o}"));
            self.problem(|| format!("handler {h:o} expected, {ran} ran"));
        }
        if let Some(a) = self.after.take() {
            // The halfword the main loop's dispatch would take: M 31, by
            // the counter as the return's `NEXT INSTR` has stepped it, `<1>`
            // set for the word's `<15:0>` (the profile's decode, held by its
            // `decode agrees` count).
            let word = m.mmem[0o31];
            let half = if e.lc() & 2 != 0 { word & 0xffff } else { word >> 16 };
            let index = (half >> 6) as usize & (macro_dispatch::ENTRIES - 1);
            let entry = d.entries[index];
            let register = half >> 6 & 7;
            let armed = entry & macro_dispatch::OPERAND != 0
                && (register == macro_dispatch::LOCAL || register == macro_dispatch::ARG);
            self.handler = Some((entry & 0o37777) as u16);
            if (register == macro_dispatch::LOCAL || register == macro_dispatch::ARG)
                && OPERAND_OPCODES.contains(&(half >> 9 & 0o37))
            {
                self.counts.operand_candidates += 1;
            }
            match e.executed() {
                None => self.counts.nopped_after += 1,
                Some(pc) => {
                    for w in writes(m.imem[pc as usize], d.register) {
                        let site = (w, a.at, pc);
                        if w.operand_only() && entry & macro_dispatch::OPERAND == 0 {
                            *self.counts.unarmed.entry(site).or_default() += 1;
                        } else {
                            *self.counts.violations.entry(site).or_default() += 1;
                            self.problem(|| {
                                format!(
                                    "{w:?} in the microcycle after the return at {:o}, at {pc:o}",
                                    a.at
                                )
                            });
                        }
                    }
                }
            }
            if m.spcptr != a.spcptr {
                self.counts.stack_moved += 1;
                let now = m.spcptr;
                self.problem(|| {
                    format!("stack pointer {:o} after the return, {now:o} after", a.spcptr)
                });
            }
            if armed {
                self.counts.operand_loads += 1;
                let delta = half & 0o77;
                let want = if register == macro_dispatch::ARG { ap + 1 } else { localp } + delta;
                let want = want as u16 & m.geometry.pdl_mask();
                if m.pdl_index != want {
                    self.counts.wrong_operand += 1;
                    let got = m.pdl_index;
                    self.problem(|| {
                        format!("PDL-INDEX {got:o} for halfword {half:o}, {want:o} expected")
                    });
                }
            }
        }
        if d.fused != self.fused {
            assert_eq!(d.fused, self.fused + 1, "one fused return a microcycle");
            self.fused = d.fused;
            self.counts.fused += 1;
            self.after = Some(After { at: e.executed().unwrap_or(u16::MAX), spcptr: m.spcptr });
        }
    }
}

/// **Microcode 2000's opcodes whose halfword's `<8:0>` is a register and a
/// delta**, which the operand bit is given to by [`fill_generic`]: CALL to
/// ND3, 0 to 13, and ND4, 16, and their twins with `<13>` set, 31 to 33
/// and 36 (`OPDTB`, `uc-macrocode.lisp:91-122`). Not BRANCH (14, 34) or
/// MISC (15, 35), whose `<8:0>` is a displacement and a function number,
/// nor AREFI-NEW (20) or the unused codes.
pub const OPERAND_OPCODES: [u32; 17] =
    [0, 1, 2, 3, 4, 5, 6, 7, 0o10, 0o11, 0o12, 0o13, 0o16, 0o31, 0o32, 0o33, 0o36];

/// **The MACRO DISPATCH MEMORY filled with the generic handlers**, every
/// index `OPDTB`'s entry for its `<13:9>` opcode, and the register enabled
/// with the main loop at `qmlp` and the bases at A memory `localp` and M
/// memory `ap`, the copies loaded from them: what the microcode is to do
/// at every start (contract H8a §4). With `operand`, the entries of
/// [`OPERAND_OPCODES`] have the operand bit, so that a fused return into
/// one of them with a LOCAL or ARG operand loads PDL-INDEX before its
/// generic handler computes the same address itself (`QADLOC1`,
/// `QADARG1`, `uc-macrocode.lisp:237-245`).
pub fn fill_generic(m: &mut Machine, qmlp: u16, opdtb: u16, localp: u16, ap: u8, operand: bool) {
    for (k, e) in m.macro_dispatch.entries.iter_mut().enumerate() {
        let op = (k >> 3 & 0o37) as u32;
        *e = m.dmem[opdtb as usize + op as usize];
        if operand && OPERAND_OPCODES.contains(&op) {
            *e |= macro_dispatch::OPERAND;
        }
    }
    let d = &mut m.macro_dispatch;
    d.register = macro_dispatch::word(qmlp, localp, ap);
    d.reload(&m.amem, &m.mmem);
}

/// **An engine with the checker run after its every step.** Everything
/// else is the engine's own.
pub struct Checked<E> {
    pub engine: E,
    pub checker: Checker,
}

impl<E: Executes> Checked<E> {
    pub fn new(engine: E) -> Checked<E> {
        let checker = Checker::new(engine.machine());
        Checked { engine, checker }
    }
}

impl<E: Executes> Engine for Checked<E> {
    fn boot(&mut self) {
        self.engine.boot();
    }
    fn keyboard_boot(&mut self) -> bool {
        self.engine.keyboard_boot()
    }
    fn step(&mut self) -> Result<(), Halt> {
        let r = self.engine.step();
        self.checker.after_step(&self.engine);
        r
    }
    fn nominal_cycle_ns(&self) -> u64 {
        self.engine.nominal_cycle_ns()
    }
    fn pc(&self) -> u16 {
        self.engine.pc()
    }
    fn pc_is_a_write(&self) -> bool {
        self.engine.pc_is_a_write()
    }
    fn lc(&self) -> u32 {
        self.engine.lc()
    }
    fn spy_read(&self, eadr: u8) -> u16 {
        self.engine.spy_read(eadr)
    }
    fn spy_write(&mut self, eadr: u8, v: u16) {
        self.engine.spy_write(eadr, v)
    }
    fn machine(&self) -> &Machine {
        self.engine.machine()
    }
    fn machine_mut(&mut self) -> &mut Machine {
        self.engine.machine_mut()
    }
    fn save(&self, w: &mut muir::checkpoint::Writer) {
        self.engine.save(w)
    }
    fn load(&mut self, r: &mut muir::checkpoint::Reader) -> std::io::Result<()> {
        self.engine.load(r)
    }
}

impl<E: Executes> Executes for Checked<E> {
    fn executed(&self) -> Option<u16> {
        self.engine.executed()
    }
}
