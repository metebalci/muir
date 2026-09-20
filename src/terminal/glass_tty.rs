// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The glass TTY: the screen as text, over telnet, and what is typed there
//! back into the keyboard.
//!
//! **This is not a device.** `--serial` is a real 2651 at J9 that the
//! band's own software must drive, with a driver that has to exist and a
//! rate that has to be programmed. This is neither: it reads the screen
//! the machine already drew --- the frame buffer, through a font --- and
//! puts what is typed on the I/O board's keyboard shift register, which is
//! the cable MIT's keyboard is on. **Nothing in the band knows it is
//! there**, so it works where a band's own software cannot help: on a
//! cold machine with no drivers, in the boot PROM, in PRAID, and in the
//! MIT diagnostics.
//!
//! What it cannot do follows from the same thing. It reads a bitmap back
//! through one font, so a cell that is not a character of that font reads
//! as `?`: under the window system, where much of the screen is not text
//! in the character font, most of it will.
//!
//! Two layers, as [`super::rfb`] and [`super::Terminal`] are two:
//!
//! - this module: the socket, the clients on it, telnet, and the painting;
//! - [`Text`]: what the screen says, which is the font readback's to make
//!   and not this module's. Nothing here knows what a frame buffer is.
//!
//! **The socket is bound to the loopback address unless told otherwise**,
//! for the reason [`super::Terminal`]'s is: telnet offers no
//! authentication at all, and a glass TTY on a routable address hands the
//! machine's keyboard to whoever reaches it. `,ro` is the other half of
//! that --- a client that may watch and not type.
//!
//! The telnet is RFC 854, with `ECHO` (RFC 857), `SUPPRESS-GO-AHEAD` (RFC
//! 858) and `NAWS` (RFC 1073). The painting is ECMA-48's `CUP`, `ED` and
//! `EL`, which is every terminal made since 1976. Nothing in either is a
//! claim about the CADR.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};

use super::Frame;
use super::keyboard::keysym;

/// What the screen says, as characters: a grid of `cols` by `rows` and
/// where the cursor is, which is what a font readback makes of the frame
/// buffer.
///
/// A cell no glyph of the font matched is `?`, and a blank one is a
/// space. The cursor is `None` when the readback found none --- on this
/// machine the cursor is a block drawn into the frame buffer like
/// anything else, so it is found rather than reported.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Text {
    pub cols: usize,
    pub rows: usize,
    /// `rows * cols` characters, row by row.
    pub cells: Vec<char>,
    pub cursor: Option<(usize, usize)>,
}

impl Text {
    /// A blank screen of that size, which is what a client is shown
    /// before any readback has been made.
    pub fn blank(cols: usize, rows: usize) -> Text {
        Text { cols, rows, cells: vec![' '; cols * rows], cursor: None }
    }

    /// Row `y`, with its trailing blanks cut: what is painted of it.
    pub fn line(&self, y: usize) -> &[char] {
        let row = &self.cells[y * self.cols..(y + 1) * self.cols];
        let end = row.iter().rposition(|&c| c != ' ').map_or(0, |k| k + 1);
        &row[..end]
    }

    /// The last row with anything on it, and the cursor's row if that is
    /// lower down: how far the screen is painted.
    ///
    /// **The whole grid is never painted.** This machine's screen is far
    /// taller than a terminal window, and painting all of it scrolls the
    /// interesting lines off the top of a client that is shorter --- the
    /// bytes are right and the person sees nothing.
    pub fn used(&self) -> usize {
        let written = (0..self.rows).rev().find(|&y| !self.line(y).is_empty()).map(|y| y + 1);
        written.unwrap_or(0).max(self.cursor.map_or(0, |(_, y)| y + 1))
    }
}

// --- Telnet, RFC 854 ---------------------------------------------------------

/// RFC 854 section "Command structure": the escape, and the commands this
/// speaks.
mod telnet {
    pub const IAC: u8 = 255;
    pub const SE: u8 = 240;
    pub const SB: u8 = 250;
    pub const WILL: u8 = 251;
    pub const WONT: u8 = 252;
    pub const DO: u8 = 253;
    pub const DONT: u8 = 254;

    /// RFC 857: the server echoes, so the client does not.
    pub const ECHO: u8 = 1;
    /// RFC 858: no line turnaround, so a client sends a character at a
    /// time rather than a line.
    pub const SUPPRESS_GO_AHEAD: u8 = 3;
    /// RFC 1073: the client says how big its window is.
    pub const NAWS: u8 = 31;
}

/// What the server offers as a client connects: it echoes and suppresses
/// go-ahead, which together are what put a real client into
/// character-at-a-time mode, and it asks for the window size.
///
/// Echo is the server's because the machine echoes: a key typed here goes
/// to the keyboard and what comes back is whatever the machine chose to
/// draw, which is the screen this paints. A client echoing for itself
/// would show characters the machine never took.
fn opening() -> Vec<u8> {
    use telnet::*;
    vec![IAC, WILL, ECHO, IAC, WILL, SUPPRESS_GO_AHEAD, IAC, DO, NAWS]
}

/// What a client sent, with telnet taken out of it: the data bytes, and a
/// window size if `NAWS` carried one. Returns how many bytes were used,
/// leaving a part-arrived command for the next look.
fn strip(buf: &[u8], data: &mut Vec<u8>, size: &mut Option<(usize, usize)>) -> usize {
    use telnet::*;
    let mut at = 0;
    while at < buf.len() {
        if buf[at] != IAC {
            data.push(buf[at]);
            at += 1;
            continue;
        }
        let Some(&command) = buf.get(at + 1) else { break };
        match command {
            // A doubled escape is the byte 255 itself.
            IAC => {
                data.push(IAC);
                at += 2;
            }
            // An option the client will or will not do: nothing is
            // refused and nothing is answered, because every option this
            // server cares about it offered first and a client's answer
            // to that needs no answer of its own. Answering would risk
            // the loop RFC 854 warns of.
            WILL | WONT | DO | DONT => {
                let Some(_) = buf.get(at + 2) else { break };
                at += 3;
            }
            SB => {
                // A subnegotiation runs to IAC SE; only NAWS is read.
                let Some(end) =
                    (at + 2..buf.len() - 1).find(|&k| buf[k] == IAC && buf.get(k + 1) == Some(&SE))
                else {
                    break;
                };
                let body = &buf[at + 2..end];
                if body.first() == Some(&NAWS) && body.len() >= 5 {
                    let w = u16::from_be_bytes([body[1], body[2]]) as usize;
                    let h = u16::from_be_bytes([body[3], body[4]]) as usize;
                    // A zero either way is the client saying it does not
                    // know, RFC 1073, and the default stands.
                    if w > 0 && h > 0 {
                        *size = Some((w, h));
                    }
                }
                at = end + 2;
            }
            // Every other command is two bytes and none of them means
            // anything here.
            _ => at += 2,
        }
    }
    at
}

// --- What a client typed -----------------------------------------------------

/// The keysyms a byte from a client stands for, as key events: each is a
/// press and a release, and a control character is the letter with
/// Control held around it.
///
/// A client sends characters and the machine wants key positions, which
/// is the same gap [`super::keyboard`] crosses for a viewer's keysyms ---
/// so this goes no further than the keysym and lets the keyboard do the
/// rest. Control is the one that cannot be a keysym on its own: Control-A
/// arrives as the byte 1, and the machine, which decodes from the stream
/// of positions, has to see Control go down, `a`, and Control come up.
fn keys_of(byte: u8) -> Vec<(u32, bool)> {
    let tap = |sym: u32| vec![(sym, true), (sym, false)];
    match byte {
        // Return: a client may send either, and CR LF must not type twice
        // --- the LF of a CR LF is dropped by the caller.
        b'\r' | b'\n' => tap(keysym::RETURN),
        0x09 => tap(keysym::TAB),
        0x1b => tap(keysym::ESCAPE),
        // Rubout and backspace both rub out: MIT's Rubout is where a
        // host keyboard's Backspace is, and `default.keys` binds both.
        0x08 | 0x7f => tap(keysym::BACKSPACE),
        // The other control characters are a letter with Control held.
        1..=26 => {
            let letter = (b'a' + byte - 1) as u32;
            vec![
                (keysym::CONTROL_L, true),
                (letter, true),
                (letter, false),
                (keysym::CONTROL_L, false),
            ]
        }
        // Printable ASCII is its own keysym, which is how X11 numbers it.
        0x20..=0x7e => tap(byte as u32),
        _ => Vec::new(),
    }
}

// --- The painting ------------------------------------------------------------

/// ECMA-48 `CUP`: the cursor to row `y`, column `x`, both counted from
/// zero here and from one on the wire.
fn cursor_to(out: &mut Vec<u8>, x: usize, y: usize) {
    out.extend_from_slice(format!("\x1b[{};{}H", y + 1, x + 1).as_bytes());
}

/// `ED` with parameter 2: the whole display cleared.
const CLEAR: &[u8] = b"\x1b[2J";

/// `EL` with no parameter: from the cursor to the end of the line.
const TO_END_OF_LINE: &[u8] = b"\x1b[K";

// --- The clients -------------------------------------------------------------

/// How many clients a glass TTY serves at once; the next connection is
/// closed as it arrives. Each holds a copy of the text it was last sent,
/// which is small beside a viewer's copy of the screen, and eight is a
/// room of people watching.
pub const MAX_CLIENTS: usize = 8;

/// How many lines are kept below the anchor when the window moves, and
/// how close the anchor may come to the bottom before it moves again:
/// output walking down the screen would otherwise re-anchor the window on
/// every line it wrote.
const MARGIN: usize = 3;

/// The window a client is assumed to have until `NAWS` says otherwise:
/// the size every terminal has had since the VT100, and the one a client
/// that never negotiates almost certainly is.
const DEFAULT_SIZE: (usize, usize) = (80, 24);

/// The most one poll reads from one client. A glass TTY's traffic is
/// keystrokes, so this is generous rather than tight; it is here so that
/// a client streaming a file into the socket cannot hold the poll.
const FILL_BUDGET: usize = 1 << 16;

/// One client on the socket.
struct Client {
    stream: TcpStream,
    who: SocketAddr,
    inbox: Vec<u8>,
    outbox: VecDeque<u8>,
    /// The window, from `NAWS` or [`DEFAULT_SIZE`].
    size: (usize, usize),
    /// **The whole screen** as this client last had it, line by line, and
    /// not merely the part of it that was painted: the window is anchored
    /// on where the machine last wrote, so what changed has to be known
    /// above and below the window as well as inside it. `None` until the
    /// client has been painted once, which is what makes one joining
    /// mid-run get the whole screen rather than a difference against
    /// something it never saw.
    was: Option<Vec<String>>,
    /// The lines on the client's own rows, so that a row whose line has
    /// not changed is left alone.
    painted: Vec<String>,
    /// The first screen row of the window this client was last painted
    /// in. When it moves every line means a different terminal row, so
    /// the whole window is painted again.
    window: usize,
    /// Whether the last byte taken from this client was a carriage
    /// return, so that the line feed of a CR LF types nothing.
    after_cr: bool,
}

impl Client {
    fn new(stream: TcpStream, who: SocketAddr) -> Client {
        let mut c = Client {
            stream,
            who,
            inbox: Vec::new(),
            outbox: VecDeque::new(),
            size: DEFAULT_SIZE,
            was: None,
            painted: Vec::new(),
            window: 0,
            after_cr: false,
        };
        c.outbox.extend(opening());
        c
    }

    fn drain(&mut self) -> std::io::Result<()> {
        while !self.outbox.is_empty() {
            let (front, _) = self.outbox.as_slices();
            match self.stream.write(front) {
                Ok(0) => return Err(ErrorKind::WriteZero.into()),
                Ok(n) => {
                    self.outbox.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Reads what has arrived. `Ok(false)` is the client having hung up.
    fn fill(&mut self) -> std::io::Result<bool> {
        let mut buf = [0u8; 4096];
        let mut read = 0;
        loop {
            if read >= FILL_BUDGET {
                return Ok(true);
            }
            match self.stream.read(&mut buf) {
                Ok(0) => return Ok(false),
                Ok(n) => {
                    read += n;
                    self.inbox.extend_from_slice(&buf[..n]);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => return Ok(true),
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// Takes the telnet out of what arrived and turns the rest into key
    /// events, which are dropped when it is read-only.
    fn typed(&mut self, keys: &mut VecDeque<(u32, bool)>, read_only: bool) {
        let (mut data, mut size) = (Vec::new(), None);
        let used = strip(&self.inbox, &mut data, &mut size);
        // A poll that carried no `NAWS` leaves the size it had.
        if let Some(s) = size {
            self.size = s;
        }
        self.inbox.drain(..used);
        if read_only {
            self.after_cr = false;
            return;
        }
        for byte in data {
            // The line feed of a CR LF is the same Return, already typed.
            if byte == b'\n' && self.after_cr {
                self.after_cr = false;
                continue;
            }
            self.after_cr = byte == b'\r';
            keys.extend(keys_of(byte));
        }
    }

    /// Paints what this client should now see.
    ///
    /// Nothing is queued while anything is still draining, so a client
    /// slower than the machine is never sent frames faster than it takes
    /// them.
    fn paint(&mut self, text: &Text) {
        if !self.outbox.is_empty() {
            return;
        }
        let (_, height) = self.size;
        let used = text.used();
        let all: Vec<String> = (0..used).map(|y| text.line(y).iter().collect::<String>()).collect();
        let window = self.anchor(&all, text.cursor, height);
        let end = (window + height).min(used);
        let lines = all.get(window..end).unwrap_or_default().to_vec();
        // A window that has moved means every line is on a different row
        // of the client's screen than it was, and a client that has never
        // been painted has nothing to be told the difference from.
        let whole = self.was.is_none() || self.window != window;
        let mut out = Vec::new();
        if whole {
            out.extend_from_slice(CLEAR);
        }
        for (k, line) in lines.iter().enumerate() {
            if !whole && self.painted.get(k).is_some_and(|old| old == line) {
                continue;
            }
            cursor_to(&mut out, 0, k);
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(TO_END_OF_LINE);
        }
        // The cursor last, so it is left where the machine has it and not
        // after whatever line was painted last.
        if let Some((x, y)) = text.cursor
            && y >= window
            && y < end
        {
            cursor_to(&mut out, x.min(self.size.0.saturating_sub(1)), y - window);
        }
        self.painted = lines;
        self.was = Some(all);
        self.window = window;
        if !out.is_empty() {
            self.outbox.extend(out);
        }
    }

    /// The first screen row of the window to show: the one that keeps
    /// **where the machine last wrote** in view.
    ///
    /// Not the cursor. The screen is far taller than a client's window,
    /// so something has to choose which part of it a person sees, and the
    /// cases a glass TTY exists for are exactly the ones where the cursor
    /// is the wrong thing to follow: the MIT diagnostics and PRAID print
    /// without moving a cursor the way a terminal means it, and a
    /// cold-load stream paints lines while the hardware cursor sits
    /// wherever it was left. What a person is watching is the line that
    /// just changed. The cursor is the fallback for a screen where
    /// nothing did.
    ///
    /// The lowest changed line is taken rather than the highest, because
    /// this screen fills downward: with changes spread wider than the
    /// window, the newest of them is at the bottom.
    ///
    /// The window is then left where it is until the anchor leaves it or
    /// comes within [`MARGIN`] of its bottom, so that output walking down
    /// the screen does not move the window on every line.
    fn anchor(&self, all: &[String], cursor: Option<(usize, usize)>, height: usize) -> usize {
        let used = all.len();
        let changed =
            self.was.as_ref().and_then(|was| (0..used).rev().find(|&y| was.get(y) != all.get(y)));
        let anchor = match changed {
            Some(y) => y,
            // **Nothing changed, so there is nothing new to follow and
            // the window stays where it is.** The cursor is the anchor
            // only before any line has ever changed --- on the first
            // paint, where there is no window yet. Falling back to it on
            // every still screen would drag the window off the line the
            // person is watching and back to wherever the cursor was
            // left, which is the whole failure this rule exists to avoid.
            None if self.was.is_some() => return self.window.min(used.saturating_sub(1)),
            None => cursor.map_or(used.saturating_sub(1), |(_, y)| y),
        };
        let anchor = anchor.min(used.saturating_sub(1));
        if anchor >= self.window && anchor + MARGIN < self.window + height && self.was.is_some() {
            return self.window;
        }
        // The anchor with a margin of lines under it, as far as there are
        // lines to show.
        (anchor + 1 + MARGIN).min(used).saturating_sub(height)
    }
}

/// The glass TTYs a run serves, and the one font readback behind them.
///
/// **One readback for all of them.** Which font the machine is drawing
/// with is the machine's business and not a client's, and reading the
/// screen is the expensive half --- 7,680 cells scored against two
/// tables --- so it is done once a poll and every client is shown the
/// same text.
pub struct Glass {
    ttys: Vec<GlassTty>,
    fonts: super::font::Fonts,
}

impl Glass {
    /// The TTYs `--glass-tty` asked for, which may be none.
    pub fn new(ttys: Vec<GlassTty>) -> Glass {
        Glass { ttys, fonts: super::font::Fonts::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.ttys.is_empty()
    }

    /// How many clients are attached, over all of them.
    pub fn clients(&self) -> usize {
        self.ttys.iter().map(GlassTty::clients).sum()
    }

    /// Which font the last screen was read through, by MIT's name for its
    /// file: what a run says when asked what it is doing.
    pub fn font_in_force(&self) -> &'static str {
        self.fonts.in_force()
    }

    /// Reads the screen, shows it to every client, and gives back what
    /// they typed --- as keysyms, so the caller hands them to the one
    /// [`super::keyboard::Keyboard`] a machine has, as it does a viewer's.
    pub fn poll(&mut self, frame: Frame) -> Vec<(u32, bool)> {
        if self.ttys.is_empty() {
            return Vec::new();
        }
        let text = self.fonts.read(&frame);
        let mut keys = Vec::new();
        for tty in &mut self.ttys {
            tty.poll(&text);
            keys.extend(tty.take_keys());
        }
        keys
    }

    /// Where each is listening, for the run to say.
    pub fn addrs(&self) -> Vec<(SocketAddr, bool)> {
        self.ttys.iter().filter_map(|t| t.addr().ok().map(|a| (a, t.read_only))).collect()
    }
}

/// The socket, and the clients on it.
pub struct GlassTty {
    listener: TcpListener,
    clients: Vec<Client>,
    keys: VecDeque<(u32, bool)>,
    /// **Watch and do not type**: `,ro`. What a client sends is dropped
    /// as it arrives rather than queued for the machine.
    pub read_only: bool,
    /// What happens on this glass TTY, printed as it happens: every client
    /// coming and going.
    pub trace: bool,
}

impl GlassTty {
    /// Listens on `addr`, without blocking. Port 0 asks the host for one,
    /// which is what `tests/glass_tty.rs` does; [`GlassTty::addr`] then says
    /// which.
    pub fn bind(addr: SocketAddr) -> std::io::Result<GlassTty> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(GlassTty {
            listener,
            clients: Vec::new(),
            keys: VecDeque::new(),
            read_only: false,
            trace: false,
        })
    }

    pub fn addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn clients(&self) -> usize {
        self.clients.len()
    }

    /// What the clients have typed, oldest first, by X11 keysym with
    /// `true` for a press --- the same form [`super::Terminal::take_keys`]
    /// hands out, so that both reach the machine by the one keyboard.
    pub fn take_keys(&mut self) -> Vec<(u32, bool)> {
        self.keys.drain(..).collect()
    }

    /// Accepts whoever has arrived, reads what was typed, and paints what
    /// `text` now says. Never blocks, so it can be called from an
    /// engine's own loop.
    pub fn poll(&mut self, text: &Text) {
        loop {
            match self.listener.accept() {
                Ok((stream, who)) => {
                    if self.clients.len() >= MAX_CLIENTS {
                        if self.trace {
                            eprintln!(
                                "glass tty: {who} turned away: {MAX_CLIENTS} clients already"
                            );
                        }
                        continue;
                    }
                    if let Err(e) = stream.set_nonblocking(true) {
                        eprintln!("glass tty: {who}: {e}");
                        continue;
                    }
                    let _ = stream.set_nodelay(true);
                    if self.trace {
                        eprintln!("glass tty: {who} connected");
                    }
                    self.clients.push(Client::new(stream, who));
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    eprintln!("glass tty: {e}");
                    break;
                }
            }
        }
        let (keys, trace, read_only) = (&mut self.keys, self.trace, self.read_only);
        self.clients.retain_mut(|c| {
            let outcome = c.fill().and_then(|open| {
                if !open {
                    return Ok(false);
                }
                c.typed(keys, read_only);
                c.paint(text);
                c.drain()?;
                Ok(true)
            });
            match outcome {
                Ok(true) => true,
                Ok(false) => {
                    if trace {
                        eprintln!("glass tty: {} hung up", c.who);
                    }
                    false
                }
                Err(e) => {
                    eprintln!("glass tty: {}: {e}", c.who);
                    false
                }
            }
        });
    }
}
