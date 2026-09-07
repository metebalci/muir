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
//! What reaches the board is the same by either route: [`Mouse::deliver`]
//! adds the counts to the behavioural board, and `crate::terminal::cable`
//! turns them into quadrature on the netlist one.

use crate::ioboard::{IoBoard, mouse};

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
