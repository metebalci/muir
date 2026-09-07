// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The terminal: the display, the keyboard and the mouse at the operator's
//! desk, reached from anywhere by a VNC viewer.
//!
//! MIT's machine had a CPT monitor on the display board's video cable, a
//! keyboard on the I/O board's serial pair and a mouse on its quadrature
//! lines. **The terminal is all three**, and `terminal` is its name here
//! because `console` is taken twice over --- CC, the console program, and
//! the diagnostic console on the SPY bus, which the two-machine lashup
//! turns on.
//!
//! Two layers:
//!
//! - [`rfb`]: the Remote Framebuffer protocol, RFC 6143, which is what a
//!   VNC viewer speaks. Nothing in it is a claim about the CADR.
//! - this module: the socket, the viewers on it, and the [`Frame`] each of
//!   them is shown.
//!
//! **Where the pixels come from is the caller's choice, and there are two
//! sources.** `micro` and `rtl` have no video timing --- see
//! [`crate::simpletv`] --- so they hand over the frame buffer itself,
//! [`Frame::of`]. `chip` with the netlist display board scans out for
//! real, 966 lines at 16.000 us with 912 of them carrying 768 dots, and
//! the faithful frame is the raster a monitor accumulates off `MECL VIDEO
//! OUT` with `HSYNC OUT` and `VSYNC OUT`. Both arrive here in the same
//! layout, one bit a pixel and 24 words to a line, so this module does not
//! know which it has.
//!
//! **The socket is bound to the loopback address unless told otherwise.**
//! RFC 6143's `None` security type is the only one offered, so a viewer
//! needs no password; a terminal on a routable address would hand anyone
//! who can reach it the keyboard of the machine. `muir --terminal 5900`
//! listens on `127.0.0.1` and `--terminal 0.0.0.0:5900` is how one says
//! otherwise, out loud.

pub mod cable;
pub mod keyboard;
pub mod monitor;
pub mod mouse;
pub mod rfb;

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use crate::simpletv::{self, SimpleTv};
use rfb::{ClientMessage, PixelFormat, Version};

/// What the viewer's window is called.
pub const NAME: &str = "muir: CADR";

/// A screen for the terminal to draw: one bit a pixel, `words_per_line`
/// words to a line, which is how the frame buffer holds it and how a
/// monitor's raster is accumulated.
#[derive(Clone, Copy)]
pub struct Frame<'a> {
    pub words: &'a [u32],
    pub width: usize,
    pub height: usize,
    pub words_per_line: usize,
    /// Whether a one bit shows black rather than white: the display
    /// board's [`simpletv::mode::BOW`].
    pub black_on_white: bool,
}

impl<'a> Frame<'a> {
    /// The frame buffer as it stands, for the engines with no video
    /// timing. On `chip` with the netlist board this is still the truth,
    /// because every write to the board is mirrored into the model
    /// (`crate::buses`), but it is not what the monitor sees: the monitor
    /// sees what the board scans out.
    pub fn of(tv: &'a SimpleTv) -> Frame<'a> {
        Frame {
            words: tv.buffer(),
            width: simpletv::WIDTH,
            height: simpletv::HEIGHT,
            words_per_line: simpletv::WORDS_PER_LINE,
            black_on_white: tv.black_on_white(),
        }
    }

    /// How many words of `words` the visible screen uses. The frame buffer
    /// is longer than the screen --- 32,768 words against 23,112 --- and
    /// the rest is not drawn.
    pub fn visible(&self) -> usize {
        self.height * self.words_per_line
    }

    /// Whether the monitor shows this pixel white.
    ///
    /// The same rule as [`SimpleTv::shows_white`], stated again because a
    /// frame may be a raster and not that buffer;
    /// `tests/terminal.rs` holds the two to each other pixel for pixel.
    pub fn shows_white(&self, x: usize, y: usize) -> bool {
        let bit = y * self.words_per_line * 32 + x;
        let lit = self.words[bit / 32] >> (bit % 32) & 1 != 0;
        lit != self.black_on_white
    }
}

/// A rectangle of the screen, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

/// Which of the first `height` rows of `frame` differ from `was`, as runs of
/// consecutive rows.  `was` is a copy of `frame`'s words, as long.
///
/// A row is 24 words, so a run of changed rows is one rectangle the full
/// width of the screen. The comparison is against what a viewer was last
/// sent rather than against a flag on the frame buffer, so a write by any
/// route shows up --- the processor's, the disk controller's DMA, or the
/// netlist board's mirror --- and nothing in the hardware model has to
/// know a viewer exists.
fn changed_rows(frame: Frame, was: &[u32], height: usize) -> Vec<(usize, usize)> {
    let words_per_line = frame.words_per_line;
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for row in 0..height {
        let at = row * words_per_line;
        let now = &frame.words[at..at + words_per_line];
        let then = &was[at..at + words_per_line];
        if now == then {
            continue;
        }
        match runs.last_mut() {
            Some((start, len)) if *start + *len == row => *len += 1,
            _ => runs.push((row, 1)),
        }
    }
    runs
}

/// What a viewer has asked for and not yet been sent.
#[derive(Clone, Copy)]
struct Request {
    incremental: bool,
    rect: Rect,
}

/// Where a viewer is in RFC 6143's opening exchange.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    /// Waiting for the viewer's twelve-byte version.
    Version,
    /// Waiting for its choice of security type, in 3.7 and 3.8.
    Security,
    /// Waiting for `ClientInit`.
    Init,
    /// Exchanging messages.
    Running,
}

/// One viewer on the socket.
struct Viewer {
    stream: TcpStream,
    who: SocketAddr,
    /// When it connected, for [`Terminal::handshake_timeout`].
    connected: Instant,
    inbox: Vec<u8>,
    outbox: VecDeque<u8>,
    stage: Stage,
    version: Version,
    format: PixelFormat,
    /// The screen's width and height as `ServerInit` gave them: RFC 6143
    /// gives a viewer no rectangle outside that screen short of a change
    /// of size, which is not sent, so a frame beyond it is shown as far as
    /// it.
    told: (usize, usize),
    /// The screen as this viewer last had it, in frame-buffer words: as
    /// many as the last frame had, remade when a frame of another size
    /// comes.
    was: Vec<u32>,
    /// Whether `was` has ever been filled: until it has, an incremental
    /// request is answered with the whole screen.
    seen: bool,
    request: Option<Request>,
    /// The pixels of [`Viewer::format`], made ready to copy out: remade
    /// whenever the viewer changes the format.
    pixels: Pixels,
    /// When the whole screen last went, for [`FULL_UPDATE_INTERVAL`].
    full_at: Option<Instant>,
    /// Bytes of a `ClientCutText` still to arrive, dropped as they do.
    skip: usize,
}

/// The pixels of a viewer's format, made ready to copy rather than
/// computed one at a time: every value a byte of the frame buffer can
/// take, as the eight pixels it stands for.
///
/// The screen is one bit a pixel and a pixel is one of two values, so a
/// rectangle can go out a byte of the frame buffer at a time --- a copy
/// out of a table --- instead of a pixel at a time, which is a branch on
/// each bit and a fresh look at the format's width and byte order. On a
/// screen whose bits do not fall in a pattern a branch predictor can
/// follow, that measured about seven times faster over a whole 768 x 963
/// screen, and it is the engine's own thread that pays for the encoding.
///
/// The table is 8 KB at 32 bits a pixel, beside the 92 KB copy of the
/// screen and the outbox each viewer already holds.
///
/// [`rfb::PixelFormat::put`] is the statement of what RFC 6143 section
/// 7.4 asks for, one pixel at a time, and every entry here is built with
/// it; `tests/terminal.rs` holds what a viewer is sent to what `put`
/// would have written for the same pixels, in every format and across a
/// rectangle whose ends fall inside a byte.
struct Pixels {
    /// Bytes a pixel takes on the wire.
    n: usize,
    /// 256 entries of eight pixels, `8 * n` bytes each: entry `b` is the
    /// frame-buffer byte `b`, its bit 0 first, bit 0 being the leftmost
    /// pixel.
    table: Vec<u8>,
    /// One pixel each, for the ends of a rectangle that begins or ends
    /// inside a byte of the frame buffer.
    white: Vec<u8>,
    black: Vec<u8>,
}

impl Pixels {
    fn new(format: PixelFormat) -> Pixels {
        // As `put` does. `PixelFormat::fits` refuses any width but 8, 16
        // or 32 as it arrives, so the four is never the one taken.
        let n = format.bytes_per_pixel().unwrap_or(4);
        let (mut white, mut black) = (Vec::new(), Vec::new());
        format.put(&mut white, format.white());
        format.put(&mut black, format.black());
        let mut table = Vec::with_capacity(256 * 8 * n);
        for byte in 0..256u32 {
            for bit in 0..8 {
                table.extend_from_slice(if byte >> bit & 1 != 0 { &white } else { &black });
            }
        }
        Pixels { n, table, white, black }
    }

    /// The eight pixels frame-buffer byte `b` stands for.
    fn eight(&self, b: u8) -> &[u8] {
        let at = b as usize * 8 * self.n;
        &self.table[at..at + 8 * self.n]
    }

    /// One pixel.
    fn one(&self, white: bool) -> &[u8] {
        if white { &self.white } else { &self.black }
    }

    /// Row `y` of `frame`, from pixel `x` for `w` of them, appended to
    /// `out`.
    ///
    /// The middle goes eight pixels at a time out of the table; the ends,
    /// where the rectangle begins or stops inside a byte of the frame
    /// buffer, go one at a time through [`Frame::shows_white`], which is
    /// where the rule about which way round the screen is lives. A viewer
    /// normally asks for the whole screen, whose 768 pixels are 96 whole
    /// bytes, and then there are no ends.
    fn put_row(&self, out: &mut Vec<u8>, frame: Frame, y: usize, x: usize, w: usize) {
        let row = &frame.words[y * frame.words_per_line..];
        // A pixel is a bit of the row, counting from the low end of the
        // first word, so pixel `p` is bit `p % 8` of byte `p / 8`, and
        // byte `k` is the `k % 4`th of word `k / 4`.  `BOW` swaps every
        // pixel, which is the word inverted before it is taken apart.
        let byte = |k: usize| {
            let word = row[k / 4];
            let word = if frame.black_on_white { !word } else { word };
            (word >> (k % 4 * 8)) as u8
        };
        let end = x + w;
        let mut p = x;
        while p < end && !p.is_multiple_of(8) {
            out.extend_from_slice(self.one(frame.shows_white(p, y)));
            p += 1;
        }
        while p + 8 <= end {
            out.extend_from_slice(self.eight(byte(p / 8)));
            p += 8;
        }
        while p < end {
            out.extend_from_slice(self.one(frame.shows_white(p, y)));
            p += 1;
        }
    }
}

/// The most one poll reads from one viewer: a viewer streaming faster
/// than the engine polls --- a clipboard the size of its length field ---
/// must not keep the poll in [`Viewer::fill`].
const FILL_BUDGET: usize = 1 << 20;

impl Viewer {
    fn new(stream: TcpStream, who: SocketAddr, visible: usize) -> Viewer {
        let mut v = Viewer {
            stream,
            who,
            connected: Instant::now(),
            inbox: Vec::new(),
            outbox: VecDeque::new(),
            stage: Stage::Version,
            version: Version::V3_8,
            format: PixelFormat::RGB888,
            told: (0, 0),
            was: vec![0; visible],
            seen: false,
            request: None,
            pixels: Pixels::new(PixelFormat::RGB888),
            full_at: None,
            skip: 0,
        };
        v.outbox.extend(rfb::VERSION);
        v
    }

    /// Writes what it can of the outbox, and stops at the first refusal.
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

    /// Reads what has arrived, up to [`FILL_BUDGET`]. `Ok(false)` is the
    /// viewer having hung up.
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

    /// Takes `n` bytes off the front of the inbox, if they are there.
    fn take(&mut self, n: usize) -> Option<Vec<u8>> {
        (self.inbox.len() >= n).then(|| self.inbox.drain(..n).collect())
    }

    /// Works through everything that has arrived, and answers it.
    fn step(
        &mut self,
        frame: Frame,
        keys: &mut VecDeque<(u32, bool)>,
        pointers: &mut VecDeque<(u8, u16, u16)>,
    ) -> Result<(), String> {
        loop {
            match self.stage {
                Stage::Version => {
                    let Some(v) = self.take(12) else { return Ok(()) };
                    self.version = Version::parse(&v);
                    let (offer, reads_choice) = self.version.security();
                    self.outbox.extend(offer);
                    self.stage = if reads_choice { Stage::Security } else { Stage::Init };
                }
                Stage::Security => {
                    let Some(c) = self.take(1) else { return Ok(()) };
                    if c[0] != 1 {
                        // A 3.8 viewer is told why before the connection
                        // goes: RFC 6143 section 7.1.3's failed
                        // `SecurityResult`, with its reason. 3.7 has no such
                        // message, and is closed alone.
                        if self.version.wants_security_result() {
                            self.outbox.extend(rfb::security_failed(
                                "only the security type None is offered",
                            ));
                        }
                        return Err(format!("security type {}, and only None is offered", c[0]));
                    }
                    if self.version.wants_security_result() {
                        self.outbox.extend(0u32.to_be_bytes());
                    }
                    self.stage = Stage::Init;
                }
                Stage::Init => {
                    // `ClientInit`'s one byte says whether other viewers
                    // may stay; they may either way: every viewer sees the
                    // screen, and every viewer's keys and pointer reach
                    // the machine.
                    let Some(_) = self.take(1) else { return Ok(()) };
                    self.told = (frame.width, frame.height);
                    self.outbox.extend(rfb::server_init(
                        frame.width as u16,
                        frame.height as u16,
                        NAME,
                    ));
                    self.stage = Stage::Running;
                }
                Stage::Running => break,
            }
        }
        loop {
            // A clipboard on its way: dropped as it comes, and the next
            // message read once it has all gone by.
            if self.skip > 0 {
                let n = self.skip.min(self.inbox.len());
                self.inbox.drain(..n);
                self.skip -= n;
                if self.skip > 0 {
                    return Ok(());
                }
            }
            let Some((message, used)) = rfb::parse(&self.inbox)? else { return Ok(()) };
            self.inbox.drain(..used);
            match message {
                ClientMessage::SetPixelFormat(f) => {
                    if !f.fits() {
                        return Err(format!(
                            "{} bits a pixel with shifts {}, {} and {}",
                            f.bits_per_pixel, f.red_shift, f.green_shift, f.blue_shift
                        ));
                    }
                    self.format = f;
                    self.pixels = Pixels::new(f);
                    if !f.true_colour {
                        self.outbox.extend(rfb::colour_map());
                    }
                    // The viewer has changed what a pixel means, so what
                    // it was sent before says nothing about what it now
                    // holds.
                    self.seen = false;
                }
                ClientMessage::SetEncodings(_) => {}
                ClientMessage::CutText { len } => self.skip = len,
                ClientMessage::FramebufferUpdateRequest { incremental, x, y, w, h } => {
                    // As asked; clipped to the screen when answered.  One
                    // outstanding request: a viewer that asks again before
                    // being answered gets one answer, and the later ask is
                    // the one honoured.
                    let rect = Rect { x: x as usize, y: y as usize, w: w as usize, h: h as usize };
                    self.request = Some(Request { incremental, rect });
                }
                ClientMessage::Key { down, keysym } => push_key(keys, (keysym, down)),
                ClientMessage::Pointer { buttons, x, y } => push_pointer(pointers, (buttons, x, y)),
            }
        }
    }

    /// Answers the outstanding request, if there is one and the last
    /// answer has gone out.
    ///
    /// Nothing is queued while anything is still draining: a full screen
    /// is 2.9 MB at 32 bits a pixel, and a viewer slower than the engine
    /// would otherwise be sent frames faster than it takes them until the
    /// memory ran out.
    fn answer(&mut self, frame: Frame) {
        if self.stage != Stage::Running || !self.outbox.is_empty() {
            return;
        }
        let Some(request) = self.request else { return };
        // A frame of another size than the last: what the viewer was sent
        // says nothing about it, and the copy is remade to its size.
        if self.was.len() != frame.visible() {
            self.was = vec![0; frame.visible()];
            self.seen = false;
        }
        // The screen is what `ServerInit` said, and the frame is shown as
        // far as that.
        let width = frame.width.min(self.told.0);
        let height = frame.height.min(self.told.1);
        // Clipped to the screen, then to what was asked for; a viewer
        // normally asks for the whole screen, and is entitled not to.
        let asked = Rect {
            x: request.rect.x.min(width),
            y: request.rect.y.min(height),
            w: request.rect.w.min(width.saturating_sub(request.rect.x)),
            h: request.rect.h.min(height.saturating_sub(request.rect.y)),
        };
        let whole = !(request.incremental && self.seen);
        // A viewer asking for the whole screen at every poll would have the
        // engine encode it at every poll; one whole screen per
        // [`FULL_UPDATE_INTERVAL`] to a viewer, the request outstanding
        // meanwhile.
        if whole && self.full_at.is_some_and(|t| t.elapsed() < FULL_UPDATE_INTERVAL) {
            return;
        }
        let rows: Vec<(usize, usize)> =
            if whole { vec![(0, height)] } else { changed_rows(frame, &self.was, height) };
        let rects: Vec<Rect> = rows
            .iter()
            .filter_map(|&(row, len)| {
                let top = row.max(asked.y);
                let bottom = (row + len).min(asked.y + asked.h);
                (bottom > top && asked.w > 0).then_some(Rect {
                    x: asked.x,
                    y: top,
                    w: asked.w,
                    h: bottom - top,
                })
            })
            .collect();
        // An incremental request with nothing to say is left outstanding,
        // which is what RFC 6143 expects: the answer comes when the screen
        // next changes.
        if rects.is_empty() {
            if !request.incremental {
                self.outbox.extend(rfb::update_header(0));
                self.request = None;
            }
            return;
        }
        self.outbox.extend(rfb::update_header(rects.len() as u16));
        let mut pixels = Vec::new();
        for r in &rects {
            self.outbox
                .extend(rfb::rectangle_header(r.x as u16, r.y as u16, r.w as u16, r.h as u16));
            pixels.clear();
            pixels.reserve(r.w * r.h * self.pixels.n);
            for y in r.y..r.y + r.h {
                self.pixels.put_row(&mut pixels, frame, y, r.x, r.w);
            }
            self.outbox.extend(pixels.iter().copied());
        }
        if whole {
            self.full_at = Some(Instant::now());
        }
        self.was.copy_from_slice(&frame.words[..frame.visible()]);
        self.seen = true;
        self.request = None;
    }
}

/// How many unread input events are kept for the engine's next look ---
/// [`Terminal::take_keys`] and [`Terminal::take_pointers`] --- before some
/// go: a queue that grew for the length of a run would be a leak rather
/// than a feature.  Of the keys, the oldest keystroke goes whole, its
/// key-down and the key-up queued for it, and a key-up whose key-down has
/// been handed out is kept: the machine tracks every key from the stream,
/// and a key it saw go down whose key-up never comes is held for the rest
/// of the run, a modifier most visibly.  Only when nothing but key-ups is
/// queued does a key-down arriving go instead, and a key-up arriving
/// displace the oldest.  Of the pointer, the oldest goes: the newest
/// position is the one that matters.
pub const INPUT_BACKLOG: usize = 256;

/// How many viewers the terminal serves at once; the next connection is
/// closed as it arrives.  Each viewer holds a copy of the screen, 92 KB,
/// and an outbox of up to a whole update, 2.9 MB at 32 bits a pixel, and
/// is another screen for the engine to encode.  Eight is a room of people
/// watching, and a bound.
pub const MAX_VIEWERS: usize = 8;

/// The most often a viewer is given the whole screen: one frame of the
/// board's own raster, [`simpletv::FRAME_NS`], which is 15.456 ms.
///
/// A viewer asks for an update either incrementally, meaning what has
/// changed, or not, meaning the whole screen. A well-behaved one asks
/// incrementally after its first frame, but nothing obliges it to, and the
/// terminal is served from the engine's own thread: a viewer asking for the
/// whole screen at every poll would have the engine encode 740,000 pixels
/// instead of running the machine. So a whole screen goes at most once a
/// frame, the request outstanding meanwhile, and an incremental update is
/// never held back.
///
/// A frame is the right interval because the board cannot produce a new
/// picture faster than it scans one, so nothing is lost by it. In an
/// ordinary run it never bites: `main` attends the terminal every 33 ms,
/// which is longer.
const FULL_UPDATE_INTERVAL: Duration = Duration::from_nanos(simpletv::FRAME_NS);

/// A key event onto the queue, as [`INPUT_BACKLOG`] says.
fn push_key(q: &mut VecDeque<(u32, bool)>, item: (u32, bool)) {
    if q.len() >= INPUT_BACKLOG {
        match q.iter().position(|&(_, down)| down) {
            // The oldest keystroke goes whole: its key-down, and its
            // key-up if that is queued too.
            Some(k) => {
                if let Some((sym, _)) = q.remove(k)
                    && let Some(j) = q.iter().skip(k).position(|&(s, down)| s == sym && !down)
                {
                    q.remove(k + j);
                }
            }
            // Nothing but key-ups, each owed to a key the machine saw go
            // down: a key-down arriving is the one to lose, and only a
            // key-up displaces the oldest.
            None if item.1 => return,
            None => {
                q.pop_front();
            }
        }
    }
    q.push_back(item);
}

/// A pointer event onto the queue: full, the oldest goes.
fn push_pointer(q: &mut VecDeque<(u8, u16, u16)>, item: (u8, u16, u16)) {
    if q.len() >= INPUT_BACKLOG {
        q.pop_front();
    }
    q.push_back(item);
}

/// The socket, and the viewers on it.
pub struct Terminal {
    listener: TcpListener,
    viewers: Vec<Viewer>,
    keys: VecDeque<(u32, bool)>,
    pointers: VecDeque<(u8, u16, u16)>,
    /// A `Bell` to send every viewer on the next poll.  One flag and not a
    /// count: a viewer told twice that the machine beeped, when the polls
    /// are further apart than the beeps, is worse off than one told once.
    bell: bool,
    /// Every viewer coming and going, printed as it happens.
    pub trace: bool,
    /// How long a viewer has, from connecting, to finish the opening
    /// exchange before it is dropped: ten seconds unless set otherwise.  A
    /// connection that says nothing would otherwise hold a place among the
    /// viewers for the run.
    pub handshake_timeout: Duration,
}

impl Terminal {
    /// Listens on `addr`, without blocking.
    ///
    /// Port 0 asks the host for one, which is what `tests/terminal.rs`
    /// does; [`Terminal::addr`] then says which.
    pub fn bind(addr: SocketAddr) -> std::io::Result<Terminal> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(Terminal {
            listener,
            viewers: Vec::new(),
            keys: VecDeque::new(),
            pointers: VecDeque::new(),
            bell: false,
            trace: false,
            handshake_timeout: Duration::from_secs(10),
        })
    }

    /// Where it is listening.
    pub fn addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn viewers(&self) -> usize {
        self.viewers.len()
    }

    /// Keys the viewers have pressed and released, oldest first, by X11
    /// keysym with `true` for a press.
    ///
    /// Taken rather than read: each is handed out once. `muir` hands them
    /// to [`keyboard::Keyboard`] after each poll, which finds each one's
    /// position on MIT's keyboard and sends it down the cable.
    pub fn take_keys(&mut self) -> Vec<(u32, bool)> {
        self.keys.drain(..).collect()
    }

    /// The same for the mouse: the button mask and where the viewer put
    /// the pointer, in screen pixels.
    pub fn take_pointers(&mut self) -> Vec<(u8, u16, u16)> {
        self.pointers.drain(..).collect()
    }

    /// The machine beeped: every viewer gets RFC 6143's `Bell` on the next
    /// poll.  Nobody looking, nobody told --- it is not kept for a viewer
    /// who connects afterwards.
    ///
    /// `muir` calls this when [`crate::ioboard::IoBoard::take_beep`] says
    /// the speaker started up.
    pub fn ring(&mut self) {
        self.bell = true;
    }

    /// Accepts whoever has arrived, reads what has been said, and answers
    /// with what `frame` now shows. Never blocks, and never takes longer
    /// than the sockets are ready for, so it can be called from an
    /// engine's own loop.
    pub fn poll(&mut self, frame: Frame) {
        loop {
            match self.listener.accept() {
                Ok((stream, who)) => {
                    // One over [`MAX_VIEWERS`] is closed as it arrives.
                    if self.viewers.len() >= MAX_VIEWERS {
                        if self.trace {
                            eprintln!("terminal: {who} turned away: {MAX_VIEWERS} viewers already");
                        }
                        continue;
                    }
                    if let Err(e) = stream.set_nonblocking(true) {
                        eprintln!("terminal: {who}: {e}");
                        continue;
                    }
                    // The updates are large and the answers small; waiting
                    // to coalesce either helps nothing.
                    let _ = stream.set_nodelay(true);
                    if self.trace {
                        eprintln!("terminal: {who} connected");
                    }
                    self.viewers.push(Viewer::new(stream, who, frame.visible()));
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    eprintln!("terminal: {e}");
                    break;
                }
            }
        }
        let (keys, pointers, trace) = (&mut self.keys, &mut self.pointers, self.trace);
        // Rung or not, it is spent on this poll: a viewer still in the
        // opening exchange has no message stream to put it in, and holding
        // it would tell whoever connects next about a beep they missed.
        let bell = std::mem::take(&mut self.bell);
        let handshake_timeout = self.handshake_timeout;
        self.viewers.retain_mut(|v| {
            // A viewer that has not finished the opening exchange in its
            // time would otherwise hold its place for the run.
            if v.stage != Stage::Running && v.connected.elapsed() > handshake_timeout {
                if trace {
                    eprintln!("terminal: {} never finished the handshake", v.who);
                }
                return false;
            }
            let outcome = v.fill().map_err(|e| e.to_string()).and_then(|open| {
                if !open {
                    return Ok(false);
                }
                v.step(frame, keys, pointers)?;
                v.answer(frame);
                // Behind the frame, so a beep never delays what it is
                // about: `Viewer::answer` queues nothing while anything is
                // still draining.
                if bell && v.stage == Stage::Running {
                    v.outbox.extend(rfb::BELL);
                }
                v.drain().map_err(|e| e.to_string())?;
                Ok(true)
            });
            match outcome {
                Ok(true) => true,
                Ok(false) => {
                    if trace {
                        eprintln!("terminal: {} hung up", v.who);
                    }
                    false
                }
                Err(e) => {
                    eprintln!("terminal: {}: {e}", v.who);
                    // What was queued for the viewer before the refusal ---
                    // a 3.8 viewer's `SecurityResult` and its reason ---
                    // goes as far as the socket takes it now.
                    let _ = v.drain();
                    false
                }
            }
        });
    }
}
