// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The mouse at the desk: a viewer's pointer turned into what MIT's mouse
//! sends the I/O board.
//!
//! MIT's mouse is a relative device --- two quadrature encoders and three
//! switches --- and the board keeps two twelve-bit counts of it that
//! microcode 323's `TRACK-MOUSE` polls every frame and takes deltas from.
//! A VNC viewer sends the opposite, an absolute position with the buttons
//! (RFC 6143 section 7.5.5), so this keeps the last position seen and
//! hands on the differences: one count a pixel, which is the one place
//! this invents anything. The machine then scales those counts by speed,
//! `MOUSE-SPEED-HACK` in `sys/window/mouse.lisp`, half a count a pixel
//! slow and four fast, so its cursor and the viewer's pointer part company
//! as a real mouse and a real screen do; that is the machine's doing and
//! is left to it.
//!
//! The buttons need no translation: RFC 6143's mask is left 1, middle 2,
//! right 4, and MIT's `buttons-down-mask` is the same three numbers
//! (`sys/doc/mouse.text`), which the register carries as tail, middle and
//! head in bits 12 to 14 ([`crate::ioboard::mouse`]).
//!
//! What reaches the board is the same by either route: the counts go to
//! a pair of [`Encoders`], which step the quadrature pairs a phase at a
//! time --- on the netlist board's lines through
//! [`super::cable::MouseOnCable`], and on the behavioural board's through
//! [`IoBoard::mouse_move`], which samples them on its own clock as the
//! board does.

use crate::ioboard::{IoBoard, mouse};

/// How long the mouse holds each quadrature phase: two of the board's 8
/// us clocks, [`crate::ioboard::KB_CLK_NS`], so that every step is
/// latched into `NEW` and then into `OLD` before the next, and none is
/// missed. A real mouse moved briskly steps faster than this, and the
/// board would miss counts; a viewer's motion arrives in bursts that this
/// spreads out.
pub const MOUSE_STEP_NS: u64 = 16_000;

/// The mouse's two encoders as they turn: motion still to be stepped out,
/// the phase each stands at, and when the next step may go. Each phase is
/// a pair of levels, `A` then `B`, in Gray order --- 00, 01, 11, 10 ---
/// so that one step moves one line, which is what the board's decoder on
/// IOBMS2 counts. Phase up is to the right and down.
#[derive(Clone, Copy, Debug)]
pub struct Encoders {
    /// Counts still to go, right and down.
    dx: i32,
    dy: i32,
    /// Each encoder's phase, 0 to 3.
    x_phase: u8,
    y_phase: u8,
    /// When the next step may go out.
    next: u64,
}

impl Default for Encoders {
    /// At power-on at phase 2, both lines of each pair high. An encoder
    /// has no phase of its own to start from and this is the model's
    /// choice: high on the cable is what the board's 74LS14s read with
    /// nothing plugged in, so a mouse that has not yet moved reads as no
    /// mouse, and `tests/cadrio_netlist.rs` can hold the behavioural board
    /// to the netlist one with its mouse lines open. Only until it moves:
    /// a mouse that stops stays at whatever phase it stopped at, as an
    /// encoder does.
    fn default() -> Self {
        Encoders { dx: 0, dy: 0, x_phase: 2, y_phase: 2, next: 0 }
    }
}

impl Encoders {
    /// Motion to step out; it accumulates.
    pub fn send(&mut self, dx: i32, dy: i32) {
        self.dx += dx;
        self.dy += dy;
    }

    pub fn busy(&self) -> bool {
        self.dx != 0 || self.dy != 0
    }

    /// When the next step is due, if there is motion to step out: not
    /// before `now`.
    pub fn next_change(&self, now: u64) -> Option<u64> {
        self.busy().then_some(self.next.max(now))
    }

    /// Moves each encoder one phase toward where it is going, if a step
    /// is due at `now`. Returns whether it did.
    pub fn step(&mut self, now: u64) -> bool {
        if !self.busy() || now < self.next {
            return false;
        }
        if self.dx != 0 {
            self.x_phase = (self.x_phase as i8 + self.dx.signum() as i8).rem_euclid(4) as u8;
            self.dx -= self.dx.signum();
        }
        if self.dy != 0 {
            self.y_phase = (self.y_phase as i8 + self.dy.signum() as i8).rem_euclid(4) as u8;
            self.dy -= self.dy.signum();
        }
        self.next = now + MOUSE_STEP_NS;
        true
    }

    /// A phase as the pair of levels on the cable, `A` then `B`: Gray, so
    /// that one step changes one line.
    pub fn levels(phase: u8) -> (bool, bool) {
        let g = phase ^ (phase >> 1);
        (g & 2 != 0, g & 1 != 0)
    }

    /// The four lines as the mouse drives them: `HORA`, `HORB`, `VERA`,
    /// `VERB`.
    pub fn lines(&self) -> [bool; 4] {
        let (xa, xb) = Self::levels(self.x_phase);
        let (ya, yb) = Self::levels(self.y_phase);
        [xa, xb, ya, yb]
    }

    pub fn save(&self, w: &mut crate::checkpoint::Writer) {
        let Encoders { dx, dy, x_phase, y_phase, next } = self;
        w.u32(*dx as u32);
        w.u32(*dy as u32);
        w.u8(*x_phase);
        w.u8(*y_phase);
        w.u64(*next);
    }

    pub fn load(&mut self, r: &mut crate::checkpoint::Reader) -> std::io::Result<()> {
        self.dx = r.u32()? as i32;
        self.dy = r.u32()? as i32;
        self.x_phase = r.u8()?;
        self.y_phase = r.u8()?;
        self.next = r.u64()?;
        Ok(())
    }
}

/// The pointer as last seen, and what has happened since it was last
/// handed on.
#[derive(Default)]
pub struct Mouse {
    last: Option<(u16, u16)>,
    buttons: u8,
    dx: i32,
    dy: i32,
}

impl Mouse {
    pub fn new() -> Mouse {
        Mouse::default()
    }

    /// A `PointerEvent` from a viewer: the buttons down, and where the
    /// pointer is in screen pixels. The first says where the pointer is
    /// and moves nothing; each after that moves by the difference.
    pub fn pointer(&mut self, mask: u8, x: u16, y: u16) {
        if let Some((lx, ly)) = self.last {
            self.dx += x as i32 - lx as i32;
            self.dy += y as i32 - ly as i32;
        }
        self.last = Some((x, y));
        self.buttons = mask & mouse::BUTTONS;
    }

    /// The motion since it was last taken, in counts, right and down.
    pub fn take_motion(&mut self) -> (i32, i32) {
        (std::mem::take(&mut self.dx), std::mem::take(&mut self.dy))
    }

    /// The buttons as they stand.
    pub fn buttons(&self) -> u8 {
        self.buttons
    }

    /// Whether anything is waiting to go to the board.
    pub fn pending(&self, board_buttons: u8) -> bool {
        self.dx != 0 || self.dy != 0 || self.buttons != board_buttons
    }

    /// Hands the behavioural I/O board what has happened: the counts,
    /// whole, and the buttons.
    pub fn deliver(&mut self, board: &mut IoBoard) {
        let (dx, dy) = self.take_motion();
        board.mouse_move(dx, dy);
        board.mouse_buttons(self.buttons);
    }
}
