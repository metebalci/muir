// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The glass TTY: the screen as text over telnet, and what is typed there.
//!
//! **These tests assert what a person sees, not what goes out on the
//! wire.** A byte stream can be right in every particular and still leave
//! somebody looking at a blank terminal --- painting the whole of a screen
//! taller than the window scrolls everything interesting off the top, and
//! the bytes that do it are perfect. So the client here is a small ANSI
//! interpreter: a grid, cursor addressing, erase-to-end-of-line and
//! scrolling past the last row, and the assertions are made against the
//! grid it ends up holding.
//!
//! Nothing here needs `vendor/`: a [`Text`] is made by hand rather than
//! read back off a frame buffer.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use muir::terminal::glass_tty::{GlassTty, Text};
use muir::terminal::keyboard::keysym;

// --- A terminal, as a person's would be --------------------------------------

/// What a client's screen holds after the bytes it was sent: ECMA-48's
/// `CUP`, `ED` and `EL`, printable characters, carriage return, line feed
/// and a scroll when the last row is passed. Telnet is taken out on the
/// way in.
struct Terminal {
    cols: usize,
    rows: usize,
    cells: Vec<char>,
    cx: usize,
    cy: usize,
}

impl Terminal {
    fn new(cols: usize, rows: usize) -> Terminal {
        Terminal { cols, rows, cells: vec![' '; cols * rows], cx: 0, cy: 0 }
    }

    /// Row `y` with its trailing blanks cut.
    fn line(&self, y: usize) -> String {
        let row: String = self.cells[y * self.cols..(y + 1) * self.cols].iter().collect();
        row.trim_end().to_string()
    }

    /// Every row that has anything on it, in order.
    fn written(&self) -> Vec<String> {
        (0..self.rows).map(|y| self.line(y)).filter(|l| !l.is_empty()).collect()
    }

    fn scroll(&mut self) {
        self.cells.drain(..self.cols);
        self.cells.extend(std::iter::repeat_n(' ', self.cols));
        self.cy = self.rows - 1;
    }

    fn put(&mut self, c: char) {
        if self.cx >= self.cols {
            self.cx = 0;
            self.cy += 1;
        }
        if self.cy >= self.rows {
            self.scroll();
        }
        let at = self.cy * self.cols + self.cx;
        self.cells[at] = c;
        self.cx += 1;
    }

    fn feed(&mut self, bytes: &[u8]) {
        let b = strip_telnet(bytes);
        let mut at = 0;
        while at < b.len() {
            match b[at] {
                0x1b if b.get(at + 1) == Some(&b'[') => {
                    let mut end = at + 2;
                    while end < b.len() && !b[end].is_ascii_alphabetic() {
                        end += 1;
                    }
                    if end >= b.len() {
                        break;
                    }
                    let params: Vec<usize> = String::from_utf8_lossy(&b[at + 2..end])
                        .split(';')
                        .map(|p| p.parse().unwrap_or(0))
                        .collect();
                    match b[end] {
                        // CUP: row and column, counted from one.
                        b'H' => {
                            self.cy = params.first().copied().unwrap_or(1).saturating_sub(1);
                            self.cx = params.get(1).copied().unwrap_or(1).saturating_sub(1);
                        }
                        // ED 2: the whole display, and the cursor home.
                        b'J' if params.first() == Some(&2) => {
                            self.cells.iter_mut().for_each(|c| *c = ' ');
                            (self.cx, self.cy) = (0, 0);
                        }
                        // EL: from the cursor to the end of the line.
                        b'K' => {
                            let from = self.cy * self.cols + self.cx;
                            let to = (self.cy + 1) * self.cols;
                            if self.cy < self.rows {
                                self.cells[from..to].iter_mut().for_each(|c| *c = ' ');
                            }
                        }
                        _ => {}
                    }
                    at = end + 1;
                }
                b'\r' => {
                    self.cx = 0;
                    at += 1;
                }
                b'\n' => {
                    self.cy += 1;
                    if self.cy >= self.rows {
                        self.scroll();
                    }
                    at += 1;
                }
                c if (0x20..0x7f).contains(&c) => {
                    self.put(c as char);
                    at += 1;
                }
                _ => at += 1,
            }
        }
    }
}

/// Telnet out of a stream the server sent: this client answers nothing,
/// so the commands are only to be skipped.
fn strip_telnet(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != 255 {
            out.push(bytes[at]);
            at += 1;
            continue;
        }
        match bytes.get(at + 1) {
            Some(&255) => {
                out.push(255);
                at += 2;
            }
            Some(&250) => {
                let end = (at + 2..bytes.len().saturating_sub(1))
                    .find(|&k| bytes[k] == 255 && bytes[k + 1] == 240);
                at = end.map_or(bytes.len(), |e| e + 2);
            }
            Some(&(251..=254)) => at += 3,
            Some(_) => at += 2,
            None => break,
        }
    }
    out
}

// --- The client and the server it talks to -----------------------------------

/// A glass TTY bound to a port the host picks, and one client on it with
/// its own terminal.
struct Client {
    tty: GlassTty,
    stream: TcpStream,
    term: Terminal,
}

impl Client {
    fn connect(cols: usize, rows: usize) -> Client {
        let tty = GlassTty::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
        Client::join(tty, cols, rows)
    }

    fn join(tty: GlassTty, cols: usize, rows: usize) -> Client {
        let stream = TcpStream::connect(tty.addr().unwrap()).unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
        let mut c = Client { tty, stream, term: Terminal::new(cols, rows) };
        // `NAWS`, so the window is the one this terminal actually has.
        let (w, h) = (cols as u16, rows as u16);
        let mut naws = vec![255, 250, 31];
        naws.extend_from_slice(&w.to_be_bytes());
        naws.extend_from_slice(&h.to_be_bytes());
        naws.extend_from_slice(&[255, 240]);
        c.stream.write_all(&naws).unwrap();
        c
    }

    /// Polls the glass TTY with `text` and feeds this terminal whatever came
    /// back, for a while, so that a paint queued on one poll and drained
    /// on the next has arrived.
    fn show(&mut self, text: &Text) {
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut quiet = 0;
        while Instant::now() < deadline && quiet < 3 {
            self.tty.poll(text);
            let mut buf = [0u8; 4096];
            match self.stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    self.term.feed(&buf[..n]);
                    quiet = 0;
                }
                Err(_) => quiet += 1,
            }
        }
    }

    /// Types `s` at the glass TTY and returns the key events it made.
    fn type_bytes(&mut self, s: &[u8], text: &Text) -> Vec<(u32, bool)> {
        self.stream.write_all(s).unwrap();
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut keys = Vec::new();
        while Instant::now() < deadline && keys.is_empty() {
            self.tty.poll(text);
            keys = self.tty.take_keys();
        }
        keys
    }
}

/// A screen of `rows` rows with `lines` written at the top of it, the
/// cursor after the last of them: what a machine that has printed a few
/// lines looks like.
fn screen(lines: &[&str], cols: usize, rows: usize) -> Text {
    let mut text = Text::blank(cols, rows);
    for (y, line) in lines.iter().enumerate() {
        for (x, c) in line.chars().enumerate() {
            text.cells[y * cols + x] = c;
        }
    }
    text.cursor = Some((0, lines.len()));
    text
}

// --- What a person sees ------------------------------------------------------

/// **A client that connects mid-run is shown the whole screen.** The
/// machine has been running and printing for some time before anyone
/// attaches, and a client sent only what changed from then on would show
/// a person nothing at all --- which is what the first version of this
/// did. The rule is [`muir::terminal::Terminal`]'s `seen`: until a client
/// has been painted once there is nothing for a difference to be against.
#[test]
fn a_client_that_connects_mid_run_is_shown_the_whole_screen() {
    let text = screen(&["MIT CADR", "PRAID 3.2", "OK"], 96, 64);
    let mut c = Client::connect(80, 24);
    c.show(&text);
    assert_eq!(c.term.written(), vec!["MIT CADR", "PRAID 3.2", "OK"], "the screen as it stands");
}

/// **The herald is visible on a terminal of 24 rows.** The machine's
/// screen is 64 rows and a person's window is 24, and painting all 64
/// scrolls the first line off the top of the window: the bytes are
/// perfect and the person sees nothing. Only the rows with anything on
/// them are painted.
#[test]
fn the_herald_is_visible_at_twenty_four_rows() {
    let text = screen(&["MIT CADR", "PRAID 3.2"], 96, 64);
    let mut c = Client::connect(80, 24);
    c.show(&text);
    assert_eq!(c.term.line(0), "MIT CADR", "the herald is on the top row of the window");
    assert_eq!(c.term.line(1), "PRAID 3.2");
}

/// **The window follows the line that changed, not the cursor.** This is
/// the case the rule exists for: the MIT diagnostics and PRAID print
/// without moving a cursor the way a terminal means it, and a cold-load
/// stream paints lines while the hardware cursor sits where it was left.
/// Anchoring on the cursor would leave a person watching the top of the
/// screen while the machine wrote thirty lines below it.
#[test]
fn the_window_follows_the_line_that_changed_and_not_the_cursor() {
    let lines: Vec<String> = (0..40).map(|k| format!("line {k}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut first = screen(&refs, 96, 64);
    // The cursor stays at the top throughout, as PRAID's does.
    first.cursor = Some((0, 0));
    let mut c = Client::connect(80, 24);
    c.show(&first);

    let mut later = first.clone();
    for (x, ch) in "DISK ERROR".chars().enumerate() {
        later.cells[30 * 96 + x] = ch;
    }
    for x in "DISK ERROR".len()..7 {
        later.cells[30 * 96 + x] = ' ';
    }
    c.show(&later);
    let seen = c.term.written();
    assert!(
        seen.iter().any(|l| l.starts_with("DISK ERROR")),
        "the line the machine wrote is in view: {seen:?}"
    );
}

/// **More written than the window is tall keeps the last written line in
/// view.** With nothing yet changed, the bottom of what was written is
/// what a person wants: it is where the output stops and the prompt is.
#[test]
fn a_screen_taller_than_the_window_keeps_the_cursor_in_view() {
    let lines: Vec<String> = (0..40).map(|k| format!("line {k}")).collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let text = screen(&refs, 96, 64);
    let mut c = Client::connect(80, 24);
    c.show(&text);
    let seen = c.term.written();
    assert!(seen.contains(&"line 39".to_string()), "the last line written: {seen:?}");
    assert!(!seen.contains(&"line 0".to_string()), "the first has scrolled out of the window");
    assert!(seen.len() <= 24, "no more than the window holds: {}", seen.len());
}

/// **A line that changed is repainted and one that did not is left
/// alone.** The glass TTY is served from the engine's own thread, and a
/// screen that is still should cost nothing.
#[test]
fn only_the_lines_that_changed_are_painted() {
    let first = screen(&["alpha", "beta"], 96, 64);
    let mut c = Client::connect(80, 24);
    c.show(&first);
    let second = screen(&["alpha", "gamma"], 96, 64);
    c.show(&second);
    assert_eq!(c.term.written(), vec!["alpha", "gamma"], "the changed line is the new one");
    // Nothing further to paint: another poll of the same text queues
    // nothing, so the terminal is unchanged.
    let before = c.term.written();
    c.show(&second);
    assert_eq!(c.term.written(), before, "a still screen paints nothing");
}

// --- What is typed -----------------------------------------------------------

/// **A printable character is its own keysym**, which is how X11 numbers
/// it and what [`muir::terminal::keyboard`] takes.
#[test]
fn a_printable_character_is_its_own_keysym() {
    let text = Text::blank(96, 64);
    let mut c = Client::connect(80, 24);
    c.show(&text);
    let keys = c.type_bytes(b"A", &text);
    assert_eq!(keys, vec![('A' as u32, true), ('A' as u32, false)]);
}

/// **A control character is the letter with Control held around it.**
/// Control-A arrives from a terminal as the byte 1, and the machine
/// decodes from the stream of key positions: it has to see Control go
/// down, `a`, and Control come up, which is what a typist would have
/// done.
#[test]
fn a_control_character_is_the_letter_with_control_around_it() {
    let text = Text::blank(96, 64);
    let mut c = Client::connect(80, 24);
    c.show(&text);
    let keys = c.type_bytes(&[1], &text);
    assert_eq!(
        keys,
        vec![
            (keysym::CONTROL_L, true),
            ('a' as u32, true),
            ('a' as u32, false),
            (keysym::CONTROL_L, false),
        ],
        "Control-A"
    );
}

/// **A carriage return and line feed together type one Return.** A
/// client may send either or both, and a Return typed twice is a blank
/// line the person did not ask for.
#[test]
fn a_carriage_return_and_line_feed_type_one_return() {
    let text = Text::blank(96, 64);
    let mut c = Client::connect(80, 24);
    c.show(&text);
    let keys = c.type_bytes(b"\r\n", &text);
    assert_eq!(keys, vec![(keysym::RETURN, true), (keysym::RETURN, false)], "one Return");
}

/// **A read-only glass TTY drops what is typed at it.** `,ro` is a client
/// that may watch the machine and not touch it.
#[test]
fn a_read_only_glass_tty_drops_what_is_typed() {
    let text = screen(&["watching"], 96, 64);
    let mut tty = GlassTty::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    tty.read_only = true;
    let mut c = Client::join(tty, 80, 24);
    c.show(&text);
    assert_eq!(c.term.line(0), "watching", "the screen is still served");
    let keys = c.type_bytes(b"xyz", &text);
    assert!(keys.is_empty(), "nothing reaches the machine: {keys:?}");
}
