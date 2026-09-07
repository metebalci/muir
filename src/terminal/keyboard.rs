// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The keyboard on the I/O board's cable: MIT's "new keyboard" of November
//! 1980, the one with the 24-bit shift register.
//!
//! Two things are modelled and both are MIT's own. **The character on the
//! cable**, from the "Protocol documentation" section at the end of
//! `sys/io1/ukbd.lisp` in the System 100 release --- the 8748 firmware of
//! the keyboard itself: 24 bits, low-order first, a start bit that is low,
//! true-high data on a cable that is high when idle, source ID `001` in
//! bits 18-16 and ones in 23-19, and the up-down, all-keys-up and boot
//! codes laid out below. **The key positions**, from `KBD-MAKE-NEW-TABLE`
//! in `lmio/kbd.123` of the System 46 sources --- the table System 100
//! ships only as qfasl in the band --- cross-checked against the ten
//! positions `ukbd.lisp` names in passing, every one of which agrees:
//! mode lock 3, super 5 and 65, alt lock 15, control 20 and 26, rubout 23,
//! shift 24 and 25, greek 44 and 35, meta 45. `tests/keyboard.rs` holds
//! the table to those ten. This is **not** the Knight keyboard, which is
//! source ID `111` with a different word and is not modelled.
//!
//! **The keyboard sends positions and the machine does the shifting.**
//! `ukbd.lisp`: "All key-encoding, including hacking of shifts, will be
//! done in software in the central machine, not in the keyboard. Note that
//! both pressing and releasing a key send a code, therefore the central
//! machine knows the status of all keys." A VNC viewer sends the opposite
//! --- X11 keysyms with the shift already applied, `A` and not `shift, a`
//! --- so [`Keyboard::key`] has to find the position and plane that make
//! that character and hold or release the shift keys to match. That is
//! the one piece of invention here, and it is confined to one function.
//!
//! What reaches the I/O board is the same 24-bit word by either route:
//! [`Keyboard::deliver`] presses it into the behavioural board, and
//! `crate::terminal::cable` clocks it down the wire into the netlist one.

use std::collections::VecDeque;

use crate::ioboard::IoBoard;

/// Source ID of the new keyboard, bits 18-16 of the word.
pub const SOURCE: u32 = 0o1;

/// The word with the reserved ones and the source in place and the
/// sixteen bits of information still to come: bits 23-19 "Reserved, must
/// be 1's", 18-16 the source.
const FRAME: u32 = 0o37 << 19 | SOURCE << 16;

/// Bit 8 of an up-down code: "1=key up, 0=key down".
pub const UP: u32 = 1 << 8;

/// Bit 15: "1 (indicates not an up-down code)", the all-keys-up word.
pub const ALL_KEYS_UP: u32 = 1 << 15;

/// A key going down or coming up, by position.
pub fn up_down(position: u8, up: bool) -> u32 {
    FRAME | if up { UP } else { 0 } | (position as u32 & 0o177)
}

/// The all-keys-up word: bit 15 set and one bit for each shifting key
/// still down, in the order `ukbd.lisp` gives --- shift 0, greek 1, top
/// 2, caps lock 3, control 4, meta 5, super 6, hyper 7, alt lock 8, mode
/// lock 9, repeat 10.
pub fn all_keys_up(shifts: u16) -> u32 {
    FRAME | ALL_KEYS_UP | (shifts as u32 & 0o3777)
}

/// The boot codes: "15-10 1, 9-6 0, 5-0 46 (octal) if cold, 62 (octal)
/// if warm". The I/O board decodes these itself, `ukbd.lisp` says ---
/// "bits 10-13 = 1, bits 6-9 = 0, and bit 16 = 1" --- and pulls `-BOOT*`.
pub fn boot(cold: bool) -> u32 {
    FRAME | 0o77 << 10 | if cold { 0o46 } else { 0o62 }
}

/// The shifting keys, as the bit each holds in the all-keys-up word and
/// in `KBD-SHIFTS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    Shift = 0,
    Greek = 1,
    Top = 2,
    CapsLock = 3,
    Control = 4,
    Meta = 5,
    Super = 6,
    Hyper = 7,
    AltLock = 8,
    ModeLock = 9,
    Repeat = 10,
}

/// What a position on the keyboard is, in MIT's table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A character key: what it gives unshifted and shifted, planes 0 and
    /// 1 of the table. The other planes --- caps lock, top, greek --- are
    /// not needed to find a position for an ASCII keysym and are left out.
    Char(u8, u8),
    /// A key with a name and no ASCII character: Return, Rubout, Help.
    Named(&'static str),
    /// A shifting key.
    Shift(Shift),
    /// A position with no key, or one MIT's table leaves undefined.
    None,
}

/// MIT's table, by position in octal, transcribed from `KBD-MAKE-NEW-TABLE`
/// with its own comment on each line. A character entry is what the key
/// gives unshifted and with shift; where MIT's file has a character outside
/// ASCII in one of those two planes --- `plus-minus` at 21 --- the position
/// is [`Key::None`] here, because no keysym reaches it.
pub const TABLE: [Key; 128] = {
    use Key::*;
    let mut t = [None; 128];
    t[0o001] = Named("Roman II");
    t[0o002] = Named("Roman IV");
    t[0o003] = Shift(self::Shift::ModeLock);
    t[0o005] = Shift(self::Shift::Super); // Left super
    t[0o011] = Char(b'4', b'$');
    t[0o012] = Char(b'r', b'R');
    t[0o013] = Char(b'f', b'F');
    t[0o014] = Char(b'v', b'V');
    t[0o015] = Shift(self::Shift::AltLock);
    t[0o017] = Named("Hand Right");
    t[0o020] = Shift(self::Shift::Control); // Left control
    t[0o022] = Named("Tab");
    t[0o023] = Named("Rubout");
    t[0o024] = Shift(self::Shift::Shift); // Left Shift
    t[0o025] = Shift(self::Shift::Shift); // Right Shift
    t[0o026] = Shift(self::Shift::Control); // Right control
    t[0o030] = Named("Hold Output");
    t[0o031] = Char(b'8', b'*');
    t[0o032] = Char(b'i', b'I');
    t[0o033] = Char(b'k', b'K');
    t[0o034] = Char(b',', b'<');
    t[0o035] = Shift(self::Shift::Greek); // Right Greek
    t[0o036] = Named("Line");
    t[0o037] = Char(b'\\', b'|');
    t[0o040] = Named("Terminal");
    t[0o042] = Named("Network");
    t[0o044] = Shift(self::Shift::Greek); // Left Greek
    t[0o045] = Shift(self::Shift::Meta); // Left Meta
    t[0o046] = Named("Status");
    t[0o047] = Named("Resume");
    t[0o050] = Named("Clear Screen");
    t[0o051] = Char(b'6', b'^');
    t[0o052] = Char(b'y', b'Y');
    t[0o053] = Char(b'h', b'H');
    t[0o054] = Char(b'n', b'N');
    t[0o061] = Char(b'2', b'@');
    t[0o062] = Char(b'w', b'W');
    t[0o063] = Char(b's', b'S');
    t[0o064] = Char(b'x', b'X');
    t[0o065] = Shift(self::Shift::Super); // Right Super
    t[0o067] = Named("Abort");
    t[0o071] = Char(b'9', b'(');
    t[0o072] = Char(b'o', b'O');
    t[0o073] = Char(b'l', b'L');
    t[0o074] = Char(b'.', b'>');
    t[0o077] = Char(b'`', b'~');
    t[0o100] = Named("Macro");
    t[0o101] = Named("Roman I");
    t[0o102] = Named("Roman III");
    t[0o104] = Shift(self::Shift::Top); // Left Top
    t[0o106] = Named("Up Thumb");
    t[0o107] = Named("Call");
    t[0o110] = Named("Clear Input");
    t[0o111] = Char(b'5', b'%');
    t[0o112] = Char(b't', b'T');
    t[0o113] = Char(b'g', b'G');
    t[0o114] = Char(b'b', b'B');
    t[0o115] = Shift(self::Shift::Repeat);
    t[0o116] = Named("Help");
    t[0o117] = Named("Hand Left");
    t[0o120] = Named("Quote");
    t[0o121] = Char(b'1', b'!');
    t[0o122] = Char(b'q', b'Q');
    t[0o123] = Char(b'a', b'A');
    t[0o124] = Char(b'z', b'Z');
    t[0o125] = Shift(self::Shift::CapsLock);
    t[0o126] = Char(b'=', b'+');
    t[0o131] = Char(b'-', b'_');
    t[0o132] = Char(b'(', b'[');
    t[0o133] = Char(b'\'', b'"');
    t[0o134] = Char(b' ', b' ');
    t[0o136] = Named("Return");
    t[0o137] = Char(b')', b']');
    t[0o141] = Named("System");
    t[0o143] = Named("Alt Mode");
    t[0o145] = Shift(self::Shift::Hyper); // Left Hyper
    t[0o146] = Char(b'}', b'}'); // shifted is undefined in MIT's table
    t[0o151] = Char(b'7', b'&');
    t[0o152] = Char(b'u', b'U');
    t[0o153] = Char(b'j', b'J');
    t[0o154] = Char(b'm', b'M');
    t[0o155] = Shift(self::Shift::Top); // Right Top
    t[0o156] = Named("End");
    t[0o157] = Named("Delete");
    t[0o160] = Named("Overstrike");
    t[0o161] = Char(b'3', b'#');
    t[0o162] = Char(b'e', b'E');
    t[0o163] = Char(b'd', b'D');
    t[0o164] = Char(b'c', b'C');
    t[0o165] = Shift(self::Shift::Meta); // Right Meta
    t[0o166] = Char(b'{', b'{'); // shifted is undefined in MIT's table
    t[0o167] = Named("Break");
    t[0o170] = Named("Stop Output");
    t[0o171] = Char(b'0', b')');
    t[0o172] = Char(b'p', b'P');
    t[0o173] = Char(b';', b':');
    t[0o174] = Char(b'/', b'?');
    t[0o175] = Shift(self::Shift::Hyper); // Right Hyper
    t[0o176] = Named("Down Thumb");
    t
};

/// The position of a named key, if it has one.
pub fn named(name: &str) -> Option<u8> {
    TABLE.iter().position(|k| matches!(k, Key::Named(n) if *n == name)).map(|p| p as u8)
}

/// The positions of a shifting key: left and right where there are two.
pub fn shifting(s: Shift) -> Vec<u8> {
    (0..128u8).filter(|&p| TABLE[p as usize] == Key::Shift(s)).collect()
}

/// X11 keysyms, from `X11/keysymdef.h`, which is what RFC 6143 section
/// 7.5.4 says a `KeyEvent` carries. Only the ones this keyboard has a key
/// for.
pub mod keysym {
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
    pub const LINEFEED: u32 = 0xff0a;
    pub const RETURN: u32 = 0xff0d;
    pub const PAUSE: u32 = 0xff13;
    pub const ESCAPE: u32 = 0xff1b;
    pub const DELETE: u32 = 0xffff;
    pub const END: u32 = 0xff57;
    pub const CANCEL: u32 = 0xff69;
    pub const HELP: u32 = 0xff6a;
    pub const BREAK: u32 = 0xff6b;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const F1: u32 = 0xffbe;
    pub const F12: u32 = 0xffc9;
    pub const SHIFT_L: u32 = 0xffe1;
    pub const SHIFT_R: u32 = 0xffe2;
    pub const CONTROL_L: u32 = 0xffe3;
    pub const CONTROL_R: u32 = 0xffe4;
    pub const CAPS_LOCK: u32 = 0xffe5;
    pub const META_L: u32 = 0xffe7;
    pub const META_R: u32 = 0xffe8;
    pub const ALT_L: u32 = 0xffe9;
    pub const ALT_R: u32 = 0xffea;
    pub const SUPER_L: u32 = 0xffeb;
    pub const SUPER_R: u32 = 0xffec;
    pub const HYPER_L: u32 = 0xffed;
    pub const HYPER_R: u32 = 0xffee;
}

/// The keys a viewer's function keys stand for, F1 to F12. The CADR has
/// no F keys; these are the keys on its top row that a PC keyboard has no
/// name for, and the order is ours. Nothing in MIT's sources says this.
pub const FUNCTION_KEYS: [&str; 12] = [
    "Terminal",
    "System",
    "Network",
    "Status",
    "Resume",
    "Abort",
    "Call",
    "Help",
    "Clear Input",
    "Clear Screen",
    "Break",
    "Quote",
];

/// The position a keysym is on, and whether it wants the shift plane.
/// `None` for a keysym this keyboard has nothing for.
///
/// A printable ASCII keysym is its own code, and is looked for on plane 0
/// and plane 1 of every character key; a key may give it on either, `(`
/// being unshifted at 132 and shifted at 71, and both are returned so
/// that [`Keyboard::key`] can pick the one that fits the shift the viewer
/// is holding.
pub fn positions(keysym: u32) -> Vec<(u8, bool)> {
    use keysym::*;
    if (0x20..=0x7e).contains(&keysym) {
        let c = keysym as u8;
        let mut out = Vec::new();
        for (p, k) in TABLE.iter().enumerate() {
            if let Key::Char(plain, shifted) = k {
                if *plain == c {
                    out.push((p as u8, false));
                }
                if *shifted == c && shifted != plain {
                    out.push((p as u8, true));
                }
            }
        }
        return out;
    }
    let name = match keysym {
        RETURN | KP_ENTER => "Return",
        TAB => "Tab",
        BACKSPACE | DELETE => "Rubout",
        LINEFEED => "Line",
        ESCAPE => "Alt Mode",
        HELP => "Help",
        BREAK => "Break",
        CANCEL => "Abort",
        END => "End",
        PAUSE => "Hold Output",
        F1..=F12 => FUNCTION_KEYS[(keysym - F1) as usize],
        _ => return Vec::new(),
    };
    named(name).map(|p| vec![(p, false)]).unwrap_or_default()
}

/// The shifting key a modifier keysym is, and which of its two positions:
/// left or right.
pub fn modifier(keysym: u32) -> Option<(Shift, usize)> {
    use keysym::*;
    Some(match keysym {
        SHIFT_L => (Shift::Shift, 0),
        SHIFT_R => (Shift::Shift, 1),
        CONTROL_L => (Shift::Control, 0),
        CONTROL_R => (Shift::Control, 1),
        // A PC keyboard's Alt is where a Lisp Machine's Meta is, and every
        // viewer sends it as Alt.
        META_L | ALT_L => (Shift::Meta, 0),
        META_R | ALT_R => (Shift::Meta, 1),
        SUPER_L => (Shift::Super, 0),
        SUPER_R => (Shift::Super, 1),
        HYPER_L => (Shift::Hyper, 0),
        HYPER_R => (Shift::Hyper, 1),
        CAPS_LOCK => (Shift::CapsLock, 0),
        _ => return None,
    })
}

/// How many words wait to go down the cable while the software is not
/// reading the keyboard.  The keyboard's own firmware has a shift register
/// and the `DONE` that says it has been sent, and no queue --- `ukbd.lisp`
/// --- so any queue is the terminal's, and one that grew for as long as
/// the software left `764100` unread would be a leak rather than a
/// feature.  256 words is 128 keystrokes.  Beyond it a press is refused
/// whole and leaves nothing down ([`Keyboard::key`]); a release always
/// goes, because the machine tracks every key from the stream ---
/// `ukbd.lisp`: "both pressing and releasing a key send a code, therefore
/// the central machine knows the status of all keys" --- and a key-down it
/// has read whose key-up never follows is a key held for the rest of the
/// run.  So the queue holds at most this many words of presses, a few more
/// where a shift is worked around a key, and one up-code for each key
/// down.  [`super::cable::OnCable`] holds as many again on the wire.
pub const BACKLOG: usize = 256;

/// The keyboard: the keys a viewer is holding, and the words waiting to go
/// down the cable.
///
/// The keyboard's own firmware sends one character and waits for the I/O
/// board's `DONE` before the next --- `ukbd.lisp`: "DONE is 1 if the shift
/// register has been sent off to the host computer" --- so words queue
/// here and go one at a time as the board takes them, [`BACKLOG`] of them
/// at the most.
#[derive(Default)]
pub struct Keyboard {
    /// Words to send, oldest first.
    queue: VecDeque<u32>,
    /// Positions the viewer has down, so that a key up is sent for each
    /// and a shift the viewer holds is not sent twice.
    down: Vec<u8>,
}

impl Keyboard {
    pub fn new() -> Keyboard {
        Keyboard::default()
    }

    /// Whether a shifting key is down, at either of its positions.
    fn holding(&self, s: Shift) -> bool {
        shifting(s).iter().any(|p| self.down.contains(p))
    }

    /// `position` down, if it is up and the queue has room.  A press the
    /// queue has no room for is refused whole, and the key stays up here
    /// too, so that no release is owed for it.
    fn press(&mut self, position: u8) {
        if self.queue.len() >= BACKLOG || self.down.contains(&position) {
            return;
        }
        self.down.push(position);
        self.queue.push_back(up_down(position, false));
    }

    /// `position` up, if it is down.  Always queued: the machine has read
    /// the key going down, or will.
    fn release(&mut self, position: u8) {
        if let Some(k) = self.down.iter().position(|&p| p == position) {
            self.down.remove(k);
            self.queue.push_back(up_down(position, true));
        }
    }

    /// A key from the viewer, by X11 keysym, going down or coming up.
    ///
    /// A modifier is pressed or released at its own position and nothing
    /// more. A character is found on the keyboard by [`positions`] and
    /// sent as the position whose plane matches the shift the viewer is
    /// holding; when no position does --- `!` with no shift held, or `(`
    /// with it held and only the unshifted key free --- the shift is
    /// pressed or released around the key, so that the machine, which
    /// decodes from the stream of positions, sees the character the
    /// viewer typed. That is the one place this code invents anything.
    pub fn key(&mut self, keysym: u32, down: bool) {
        if let Some((s, side)) = modifier(keysym) {
            let at = shifting(s);
            let position = at.get(side).or(at.first()).copied();
            if let Some(p) = position {
                if down { self.press(p) } else { self.release(p) }
            }
            return;
        }
        let found = positions(keysym);
        if found.is_empty() {
            return;
        }
        let shifted = self.holding(Shift::Shift);
        // The position whose plane the viewer's own shift already gives.
        if let Some(&(p, _)) = found.iter().find(|&&(_, wants)| wants == shifted) {
            if down {
                self.press(p)
            } else {
                self.release(p)
            }
            return;
        }
        // Otherwise the shift has to be worked around the key: held for it
        // when the character is on the shifted plane and the viewer is
        // not holding shift, let go for it when the other way round. The
        // machine sees shift, key, and shift back, which is what a typist
        // would have done.
        let (p, wants) = found[0];
        let shift = shifting(Shift::Shift)[0];
        if !down {
            self.release(p);
            return;
        }
        // A press with the shift worked around it is refused whole beyond
        // the backlog, as a plain press is; it leaves nothing down.
        if self.queue.len() >= BACKLOG {
            return;
        }
        if wants {
            self.queue.push_back(up_down(shift, false));
            self.queue.push_back(up_down(p, false));
            self.queue.push_back(up_down(p, true));
            self.queue.push_back(up_down(shift, true));
        } else {
            // Every shift the viewer holds comes up around the key.
            let held: Vec<u8> =
                shifting(Shift::Shift).into_iter().filter(|q| self.down.contains(q)).collect();
            for &q in &held {
                self.queue.push_back(up_down(q, true));
            }
            self.queue.push_back(up_down(p, false));
            self.queue.push_back(up_down(p, true));
            for &q in &held {
                self.queue.push_back(up_down(q, false));
            }
        }
    }

    /// Words waiting to go down the cable.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// The next word, taken.
    pub fn take(&mut self) -> Option<u32> {
        self.queue.pop_front()
    }

    /// The next word, looked at.
    pub fn peek(&self) -> Option<u32> {
        self.queue.front().copied()
    }

    /// Hands the next word to the behavioural I/O board, if the board has
    /// taken the last: the board's `KBD READY` is the keyboard's `DONE`
    /// the other way up.
    pub fn deliver(&mut self, board: &mut IoBoard) -> bool {
        if board.keyboard_ready() {
            return false;
        }
        match self.queue.pop_front() {
            Some(word) => {
                board.press(word);
                true
            }
            None => false,
        }
    }
}
