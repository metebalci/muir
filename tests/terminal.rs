// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The terminal, driven by a viewer of our own: a socket that speaks RFC
//! 6143 back at it and checks that what arrives is the screen.
//!
//! The server never blocks and is polled from the engine's loop, so a test
//! has to be both ends at once: write, poll, read. [`Viewer::exchange`] is
//! that, and every test here goes through it.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use muir::terminal::{Frame, Terminal, rfb};
use muir::tv::{self, Tv};

/// A viewer, blocking, with the server it is talking to.
struct Viewer {
    terminal: Terminal,
    stream: TcpStream,
    tv: Tv,
}

impl Viewer {
    /// Binds a terminal to a port the host picks and connects to it, with
    /// nothing said yet.
    fn open() -> Viewer {
        Viewer::open_showing(Tv::default())
    }

    /// The same, showing another board: [`Tv::color`] is the color TV,
    /// whose screen is four bits a pixel through the colour map.
    fn open_showing(tv: Tv) -> Viewer {
        let terminal = Terminal::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
        let stream = TcpStream::connect(terminal.addr().unwrap()).unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        Viewer { terminal, stream, tv }
    }

    /// [`Viewer::open`], then RFC 6143's opening exchange as 3.8 as far as
    /// `ServerInit`, which it returns.
    fn connect() -> (Viewer, Vec<u8>) {
        Viewer::connect_showing(Tv::default())
    }

    /// The same, showing another board.
    fn connect_showing(tv: Tv) -> (Viewer, Vec<u8>) {
        let mut v = Viewer::open_showing(tv);
        let init = v.handshake();
        (v, init)
    }

    /// RFC 6143's opening exchange as 3.8 as far as `ServerInit`, on a
    /// viewer already opened.
    fn handshake(&mut self) -> Vec<u8> {
        assert_eq!(&self.exchange(&[], 12)[..], rfb::VERSION, "the version the server offers");
        // 3.8: one security type on offer, and it is None.
        assert_eq!(self.exchange(rfb::VERSION, 2), vec![1, 1], "one type, and it is None");
        assert_eq!(self.exchange(&[1], 4), vec![0, 0, 0, 0], "SecurityResult, and it is ok");
        // ClientInit's shared flag, then ServerInit: 24 bytes and a name.
        let head = self.exchange(&[1], 24);
        let name = u32::from_be_bytes(head[20..24].try_into().unwrap()) as usize;
        let rest = self.exchange(&[], name);
        [head, rest].concat()
    }

    /// Writes `out`, then polls the server and reads until `want` bytes
    /// have come back.
    fn exchange(&mut self, out: &[u8], want: usize) -> Vec<u8> {
        exchange_with(&mut self.terminal, &mut self.stream, Frame::of(&self.tv), out, want)
    }

    /// Asks for the whole screen and returns the rectangles that come
    /// back, as `(x, y, w, h, pixels)`.
    fn update(
        &mut self,
        incremental: bool,
        bytes_per_pixel: usize,
    ) -> Vec<(u16, u16, u16, u16, Vec<u8>)> {
        let whole = (0, 0, tv::WIDTH as u16, tv::HEIGHT as u16);
        self.update_rect(incremental, whole, bytes_per_pixel)
    }

    /// The same for one rectangle of it.
    fn update_rect(
        &mut self,
        incremental: bool,
        (rx, ry, rw, rh): (u16, u16, u16, u16),
        bytes_per_pixel: usize,
    ) -> Vec<(u16, u16, u16, u16, Vec<u8>)> {
        let mut request = vec![3u8, incremental as u8];
        for v in [rx, ry, rw, rh] {
            request.extend_from_slice(&v.to_be_bytes());
        }
        let head = self.exchange(&request, 4);
        assert_eq!(head[0], 0, "a FramebufferUpdate");
        let count = u16::from_be_bytes([head[2], head[3]]);
        let mut out = Vec::new();
        for _ in 0..count {
            let r = self.exchange(&[], 12);
            let at = |k: usize| u16::from_be_bytes([r[k], r[k + 1]]);
            let (x, y, w, h) = (at(0), at(2), at(4), at(6));
            assert_eq!(
                i32::from_be_bytes(r[8..12].try_into().unwrap()),
                0,
                "Raw encoding, the one every server must have"
            );
            let pixels = self.exchange(&[], w as usize * h as usize * bytes_per_pixel);
            out.push((x, y, w, h, pixels));
        }
        out
    }

    /// `SetPixelFormat`, and the colour map that follows a mapped one:
    /// two entries for the black-and-white screen and sixteen for the
    /// colour one, six bytes each.
    fn set_format(&mut self, f: rfb::PixelFormat) {
        let mut m = vec![0u8, 0, 0, 0];
        m.extend_from_slice(&f.encode());
        if f.true_colour {
            self.exchange(&m, 0);
        } else {
            let entries = if self.tv.strap() == tv::COLOR_TV { tv::COLORS } else { 2 };
            let map = self.exchange(&m, 6 + 6 * entries);
            assert_eq!(map[0], 1, "SetColourMapEntries");
            assert_eq!(
                u16::from_be_bytes([map[4], map[5]]) as usize,
                entries,
                "the screen's colours"
            );
        }
    }
}

/// The formats a viewer may ask for: eight, sixteen and thirty-two bits a
/// pixel, each way round, and the mapped one that the eight-bit case is.
fn formats() -> Vec<rfb::PixelFormat> {
    let mut out = Vec::new();
    for big_endian in [false, true] {
        out.push(rfb::PixelFormat { big_endian, ..rfb::PixelFormat::RGB888 });
        out.push(rfb::PixelFormat {
            bits_per_pixel: 16,
            depth: 16,
            big_endian,
            red_max: 31,
            green_max: 63,
            blue_max: 31,
            red_shift: 11,
            green_shift: 5,
            blue_shift: 0,
            ..rfb::PixelFormat::RGB888
        });
        out.push(rfb::PixelFormat {
            bits_per_pixel: 8,
            depth: 8,
            big_endian,
            true_colour: false,
            ..rfb::PixelFormat::RGB888
        });
    }
    out
}

/// A screen whose bits fall in no pattern, so that a wrong bit anywhere in
/// a byte or a word shows up rather than cancelling out.
fn scramble(tv: &mut Tv) {
    for k in 0..(tv::HEIGHT * tv::WORDS_PER_LINE) as u32 {
        tv.write_buffer(k, k.wrapping_mul(0x9e37_79b9) ^ k.rotate_left(13));
    }
}

/// What [`rfb::PixelFormat::put`] would write for the rectangle `(x, y, w,
/// h)` of `frame`, one pixel at a time.
fn as_put_would(
    frame: Frame,
    f: rfb::PixelFormat,
    (x, y, w, h): (usize, usize, usize, usize),
) -> Vec<u8> {
    let (white, black) = (f.white(), f.black());
    let mut out = Vec::new();
    for row in y..y + h {
        for col in x..x + w {
            f.put(&mut out, if frame.shows_white(col, row) { white } else { black });
        }
    }
    out
}

/// Writes `out` to `stream`, then polls `terminal` with `frame` and reads
/// until `want` bytes have come back.
fn exchange_with(
    terminal: &mut Terminal,
    stream: &mut TcpStream,
    frame: Frame,
    out: &[u8],
    want: usize,
) -> Vec<u8> {
    if !out.is_empty() {
        stream.write_all(out).unwrap();
    }
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while got.len() < want && Instant::now() < deadline {
        terminal.poll(frame);
        let mut buf = vec![0u8; want - got.len()];
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(_) => continue, // the read timed out; poll again
        }
    }
    assert_eq!(got.len(), want, "wanted {want} bytes, got {}: {got:?}", got.len());
    got
}

/// **A viewer is told the screen's size.** `ServerInit` carries
/// `MAIN-SCREEN-WIDTH` and `MAIN-SCREEN-HEIGHT` as `shwarm.lisp` gives
/// them, the format the server offers, and a name for the window.
#[test]
fn a_viewer_is_told_the_screens_size() {
    let (_v, init) = Viewer::connect();
    assert_eq!(u16::from_be_bytes([init[0], init[1]]), tv::WIDTH as u16, "width");
    assert_eq!(u16::from_be_bytes([init[2], init[3]]), tv::HEIGHT as u16, "height");
    assert_eq!(
        rfb::PixelFormat::parse(&init[4..20]),
        rfb::PixelFormat::RGB888,
        "the format offered"
    );
    assert_eq!(
        String::from_utf8_lossy(&init[24..]),
        muir::terminal::NAME,
        "the name of the window"
    );
}

/// **The first update is the whole screen, and it is the frame buffer.**
/// Every pixel of it, against [`Tv::shows_white`] --- so a viewer
/// and the PNG cannot disagree about which way round the screen is.
#[test]
fn the_first_update_is_the_whole_screen() {
    let (mut v, _) = Viewer::connect();
    // Something to see: a word at the top left, a word further down, and
    // the mode register left white-on-black.
    v.tv.write_buffer(0, 0x0f0f_0f0f);
    v.tv.write_buffer(100 * tv::WORDS_PER_LINE as u32 + 3, 0xffff_ffff);

    let rects = v.update(false, 4);
    assert_eq!(rects.len(), 1, "one rectangle for the whole screen");
    let (x, y, w, h, pixels) = &rects[0];
    assert_eq!((*x, *y, *w, *h), (0, 0, tv::WIDTH as u16, tv::HEIGHT as u16));

    let white = rfb::PixelFormat::RGB888.white();
    let mut checked = 0;
    for row in 0..tv::HEIGHT {
        for col in 0..tv::WIDTH {
            let at = (row * tv::WIDTH + col) * 4;
            let got = u32::from_le_bytes(pixels[at..at + 4].try_into().unwrap());
            let want = if v.tv.shows_white(col, row) { white } else { 0 };
            assert_eq!(got, want, "pixel {col},{row}");
            checked += 1;
        }
    }
    assert_eq!(checked, tv::WIDTH * tv::HEIGHT, "every pixel of the screen");
}

/// **An incremental update carries only the rows that changed.** The
/// server keeps what each viewer was last sent and compares, so a write by
/// any route shows up and the hardware model needs no dirty flag.
#[test]
fn an_incremental_update_carries_only_what_changed() {
    let (mut v, _) = Viewer::connect();
    v.update(false, 4); // the whole screen, so there is a baseline

    // Nothing has changed: the request stays outstanding, and asking for
    // the whole screen again is the way to get an answer.
    v.tv.write_buffer(7 * tv::WORDS_PER_LINE as u32 + 2, 0xdead_beef);
    let rects = v.update(true, 4);
    assert_eq!(rects.len(), 1, "one rectangle: {rects:?}");
    let (x, y, w, h, _) = rects[0];
    assert_eq!((x, y, w, h), (0, 7, tv::WIDTH as u16, 1), "the one row that moved");

    // Two rows next to each other come back as one rectangle.
    for row in [20u32, 21] {
        v.tv.write_buffer(row * tv::WORDS_PER_LINE as u32, 1);
    }
    let rects = v.update(true, 4);
    assert_eq!(rects.len(), 1, "two touching rows are one rectangle: {rects:?}");
    assert_eq!((rects[0].1, rects[0].3), (20, 2), "at row 20, two rows high");

    // And two apart come back as two.
    v.tv.write_buffer(30 * tv::WORDS_PER_LINE as u32, 1);
    v.tv.write_buffer(60 * tv::WORDS_PER_LINE as u32, 1);
    let rects = v.update(true, 4);
    assert_eq!(rects.len(), 2, "two rows apart are two rectangles: {rects:?}");
    assert_eq!((rects[0].1, rects[0].3), (30, 1));
    assert_eq!((rects[1].1, rects[1].3), (60, 1));
}

/// **A viewer that wants eight bits a pixel gets them, through a colour
/// map.** The screen has two colours, so `SetColourMapEntries` sends two
/// and white is entry 1.
#[test]
fn a_viewer_can_ask_for_a_colour_map() {
    let (mut v, _) = Viewer::connect();
    v.tv.write_buffer(0, 0xffff_ffff);

    let mut set = vec![0u8, 0, 0, 0];
    let mapped = rfb::PixelFormat {
        bits_per_pixel: 8,
        depth: 8,
        big_endian: false,
        true_colour: false,
        ..rfb::PixelFormat::RGB888
    };
    set.extend_from_slice(&mapped.encode());
    // The map comes unasked, as RFC 6143 section 7.6.2 has it.
    let map = v.exchange(&set, 6 + 12);
    assert_eq!(map[0], 1, "SetColourMapEntries");
    assert_eq!(u16::from_be_bytes([map[4], map[5]]), 2, "two colours");
    assert_eq!(&map[6..12], &[0, 0, 0, 0, 0, 0], "entry 0 is black");
    assert_eq!(&map[12..18], &[0xff; 6], "entry 1 is white");

    let rects = v.update(false, 1);
    let (_, _, _, _, pixels) = &rects[0];
    assert_eq!(pixels[0], rfb::WHITE_INDEX, "the lit pixel is the white entry");
    assert_eq!(pixels[tv::WIDTH - 1], 0, "and the far end of the row is black");
}

/// **Black-on-white swaps every pixel.** `MODE BOW` is the display
/// board's own bit, and the viewer sees what the monitor would.
#[test]
fn the_mode_registers_bow_bit_swaps_the_screen() {
    let (mut v, _) = Viewer::connect();
    v.tv.write_buffer(0, 1);
    let plain = v.update(false, 4)[0].4.clone();
    v.tv.write_control(0, tv::mode::BOW, 0);
    let swapped = v.update(false, 4)[0].4.clone();
    let white = rfb::PixelFormat::RGB888.white().to_le_bytes();
    assert_eq!(&plain[0..4], &white, "the lit bit is white to start with");
    assert_eq!(&swapped[0..4], &[0, 0, 0, 0], "and black once BOW is set");
    assert_eq!(&plain[4..8], &[0, 0, 0, 0], "its neighbour is black");
    assert_eq!(&swapped[4..8], &white, "and white once BOW is set");
}

/// **A frame and the frame buffer agree on which way round the screen
/// is.** The rule lives in [`Tv::shows_white`] and again in
/// [`Frame::shows_white`], because a frame may be a monitor's raster and
/// not that buffer; this is the check that the second copy says the same
/// as the first.
#[test]
fn a_frame_shows_what_the_frame_buffer_shows() {
    let mut tv = Tv::default();
    for (k, word) in [0x0000_0001u32, 0xffff_ffff, 0xaaaa_5555, 0x8000_0000].iter().enumerate() {
        tv.write_buffer(k as u32, *word);
    }
    for bow in [false, true] {
        tv.write_control(0, if bow { tv::mode::BOW } else { 0 }, 0);
        let frame = Frame::of(&tv);
        assert_eq!(frame.black_on_white, bow);
        for y in 0..4 {
            for x in 0..tv::WIDTH {
                assert_eq!(frame.shows_white(x, y), tv.shows_white(x, y), "{x},{y} bow {bow}");
            }
        }
    }
}

/// **Typing at a viewer is taken off the wire and kept.** Nothing consumes
/// it yet --- the keyboard on the I/O board's cable is the next piece ---
/// but the protocol layer has to read those bytes or the stream desyncs,
/// so the events are here to be collected.
#[test]
fn keys_and_the_pointer_come_off_the_wire() {
    let (mut v, _) = Viewer::connect();
    // KeyEvent: down, then up, of X11 keysym 0x0041, "A".
    let mut typed = vec![4u8, 1, 0, 0];
    typed.extend_from_slice(&0x41u32.to_be_bytes());
    typed.extend_from_slice(&[4, 0, 0, 0]);
    typed.extend_from_slice(&0x41u32.to_be_bytes());
    // PointerEvent: button 1 down at 100, 200.
    typed.extend_from_slice(&[5, 1]);
    typed.extend_from_slice(&100u16.to_be_bytes());
    typed.extend_from_slice(&200u16.to_be_bytes());
    v.stream.write_all(&typed).unwrap();
    // Nothing is expected back, so it is polled until the events have
    // arrived --- and for as long as that takes rather than a hundred
    // times, a hundred polls of an idle terminal being over in less time
    // than a keystroke takes to cross the loopback.
    let deadline = Instant::now() + Duration::from_secs(10);
    let (mut keys, mut pointers) = (Vec::new(), Vec::new());
    while (keys.len() < 2 || pointers.is_empty()) && Instant::now() < deadline {
        v.terminal.poll(Frame::of(&v.tv));
        keys.extend(v.terminal.take_keys());
        pointers.extend(v.terminal.take_pointers());
    }
    assert_eq!(keys, vec![(0x41, true), (0x41, false)], "the key, down then up");
    assert_eq!(pointers, vec![(1, 100, 200)], "the pointer");
    assert!(v.terminal.take_keys().is_empty(), "each event is handed out once");
}

/// **A viewer that hangs up is forgotten.**
#[test]
fn a_viewer_that_hangs_up_is_dropped() {
    let (mut v, _) = Viewer::connect();
    assert_eq!(v.terminal.viewers(), 1);
    let dead = v.stream.try_clone().unwrap();
    drop(dead);
    drop(v.stream);
    for _ in 0..100 {
        v.terminal.poll(Frame::of(&v.tv));
        if v.terminal.viewers() == 0 {
            break;
        }
    }
    assert_eq!(v.terminal.viewers(), 0, "the viewer is gone");
}

impl Viewer {
    /// Polls the server until it closes this connection; false if it has
    /// not within ten seconds.
    fn hung_up(&mut self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut buf = [0u8; 64];
        while Instant::now() < deadline {
            self.terminal.poll(Frame::of(&self.tv));
            if let Ok(0) = self.stream.read(&mut buf) {
                return true;
            }
        }
        false
    }
}

/// **A pixel format whose shifts fall off the word is refused.** RFC 6143
/// section 7.4 gives each shift as "the number of shifts needed to get the
/// red value in a pixel to the least significant bit", so one of 32 or
/// more names no bit of any pixel. A viewer that sends one is dropped, as
/// one naming 24 bits a pixel is, rather than shifted by.
#[test]
fn a_pixel_format_with_a_shift_off_the_word_is_refused() {
    let (mut v, _) = Viewer::connect();
    let format = rfb::PixelFormat { red_shift: 40, ..rfb::PixelFormat::RGB888 };
    assert!(!format.fits(), "40 shifts is off a 32-bit pixel");
    assert!(rfb::PixelFormat::RGB888.fits());
    let mut msg = vec![0u8, 0, 0, 0];
    msg.extend_from_slice(&format.encode());
    v.stream.write_all(&msg).unwrap();
    assert!(v.hung_up(), "the viewer is dropped");
}

/// **The clipboard is skipped as it arrives, not collected.** A
/// `ClientCutText` names its length in 32 bits, and a viewer may put four
/// gigabytes there. Nothing on a CADR wants the text, so the message is
/// taken at its eight-byte head and the text is dropped as it comes, in
/// whatever pieces --- and what follows it is read as usual.
#[test]
fn cut_text_is_skipped_as_it_arrives() {
    let mut head = vec![6u8, 0, 0, 0];
    head.extend_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(
        rfb::parse(&head).unwrap(),
        Some((rfb::ClientMessage::CutText { len: u32::MAX as usize }, 8)),
        "the head alone is the message"
    );

    let (mut v, _) = Viewer::connect();
    let mut msg = vec![6u8, 0, 0, 0];
    msg.extend_from_slice(&100_000u32.to_be_bytes());
    v.stream.write_all(&msg).unwrap();
    v.terminal.poll(Frame::of(&v.tv));
    let piece = vec![b'x'; 30_000];
    for _ in 0..3 {
        v.stream.write_all(&piece).unwrap();
        v.terminal.poll(Frame::of(&v.tv));
    }
    v.stream.write_all(&piece[..10_000]).unwrap();
    let mut key = vec![4u8, 1, 0, 0];
    key.extend_from_slice(&0x61u32.to_be_bytes());
    v.stream.write_all(&key).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut keys = Vec::new();
    while keys.is_empty() && Instant::now() < deadline {
        v.terminal.poll(Frame::of(&v.tv));
        keys = v.terminal.take_keys();
    }
    assert_eq!(keys, vec![(0x61, true)], "the key after the text comes off the wire as itself");
}

// --- What a viewer may do to the terminal ---------------------------------------

/// A `KeyEvent`, RFC 6143 section 7.5.4.
fn key_event(down: bool, keysym: u32) -> Vec<u8> {
    let mut b = vec![4u8, down as u8, 0, 0];
    b.extend_from_slice(&keysym.to_be_bytes());
    b
}

/// A `PointerEvent`, RFC 6143 section 7.5.5.
fn pointer_event(buttons: u8, x: u16, y: u16) -> Vec<u8> {
    let mut b = vec![5u8, buttons];
    b.extend_from_slice(&x.to_be_bytes());
    b.extend_from_slice(&y.to_be_bytes());
    b
}

/// A `FramebufferUpdateRequest` for the whole screen the viewer was told of.
fn update_request(incremental: bool) -> Vec<u8> {
    let mut b = vec![3u8, incremental as u8];
    for v in [0u16, 0, tv::WIDTH as u16, tv::HEIGHT as u16] {
        b.extend_from_slice(&v.to_be_bytes());
    }
    b
}

/// **A version RFC 6143 does not publish is spoken to as 3.3.** Section
/// 7.1.1: "Other version numbers are reported by some servers and clients,
/// but should be interpreted as 3.3 since they do not implement the
/// different handshake in 3.7 or 3.8." Apple's Screen Sharing answers `RFB
/// 003.889`, and gets 3.3's handshake: the one security type as a word,
/// no choice read back, no `SecurityResult`, and `ServerInit` on its
/// `ClientInit`.
#[test]
fn an_unpublished_version_is_spoken_to_as_3_3() {
    assert_eq!(rfb::Version::parse(b"RFB 003.889\n"), rfb::Version::V3_3);
    assert_eq!(rfb::Version::parse(b"RFB 003.003\n"), rfb::Version::V3_3);
    assert_eq!(rfb::Version::parse(b"RFB 003.007\n"), rfb::Version::V3_7);
    assert_eq!(rfb::Version::parse(b"RFB 003.008\n"), rfb::Version::V3_8);

    let mut v = Viewer::open();
    assert_eq!(&v.exchange(&[], 12)[..], rfb::VERSION);
    assert_eq!(v.exchange(b"RFB 003.889\n", 4), [0, 0, 0, 1], "3.3: the type None, as a word");
    let head = v.exchange(&[1], 24);
    assert_eq!(u16::from_be_bytes([head[0], head[1]]), tv::WIDTH as u16, "ServerInit");
}

/// **A 3.8 viewer that picks a type not offered is told why before the
/// connection goes.** RFC 6143 section 7.1.3: the `SecurityResult` word is
/// 1 for failed, and "if unsuccessful, the server sends a string
/// describing the reason for the failure, and then closes the connection"
/// --- the string a `U32` length and that many ASCII characters, as
/// section 7.1.2 lays it out.
#[test]
fn a_3_8_viewer_picking_an_unoffered_security_type_is_told_why() {
    let mut v = Viewer::open();
    v.exchange(&[], 12);
    assert_eq!(v.exchange(rfb::VERSION, 2), [1, 1]);
    // 2 is VNC Authentication, which is not on offer.
    assert_eq!(v.exchange(&[2], 4), [0, 0, 0, 1], "SecurityResult: failed");
    let len = u32::from_be_bytes(v.exchange(&[], 4).try_into().unwrap()) as usize;
    let reason = v.exchange(&[], len);
    assert!(!reason.is_empty() && reason.is_ascii(), "a reason in ASCII: {reason:?}");
    eprintln!("the reason given: {}", String::from_utf8_lossy(&reason));
    assert!(v.hung_up(), "and then the connection is closed");
}

/// **Viewers beyond the cap are turned away.** Each viewer costs the
/// terminal a copy of the screen and an outbox of up to a whole update, and
/// each is another screen to encode; [`muir::terminal::MAX_VIEWERS`] is
/// how many the terminal serves at once, and the next connection is closed
/// as it arrives.
#[test]
fn viewers_beyond_the_cap_are_turned_away() {
    use muir::terminal::MAX_VIEWERS;
    let mut terminal = Terminal::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    let addr = terminal.addr().unwrap();
    let tv = Tv::default();
    let streams: Vec<TcpStream> =
        (0..=MAX_VIEWERS).map(|_| TcpStream::connect(addr).unwrap()).collect();
    for _ in 0..20 {
        terminal.poll(Frame::of(&tv));
    }
    assert_eq!(terminal.viewers(), MAX_VIEWERS, "the cap");
    let (mut served, mut turned_away) = (0, 0);
    for mut s in streams {
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut version = [0u8; 12];
        match s.read_exact(&mut version) {
            Ok(()) => {
                assert_eq!(&version, rfb::VERSION);
                served += 1;
            }
            Err(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof, "{e}");
                turned_away += 1;
            }
        }
    }
    assert_eq!((served, turned_away), (MAX_VIEWERS, 1));
}

/// **A viewer that never finishes the handshake is dropped.** A connection
/// that sends nothing would otherwise hold its place among the viewers for
/// the run; [`Terminal::handshake_timeout`] is how long it gets.
#[test]
fn a_viewer_that_never_finishes_the_handshake_is_dropped() {
    let mut v = Viewer::open();
    v.terminal.handshake_timeout = Duration::from_millis(100);
    for _ in 0..100 {
        v.terminal.poll(Frame::of(&v.tv));
        if v.terminal.viewers() == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(v.terminal.viewers(), 1, "accepted");
    std::thread::sleep(Duration::from_millis(150));
    v.terminal.poll(Frame::of(&v.tv));
    assert_eq!(v.terminal.viewers(), 0, "the version never came");
    assert!(v.hung_up());
}

/// **A frame taller than the screen the viewer was told of is clipped to
/// that screen.** `ServerInit` names the size once; a frame that arrives
/// taller afterwards is shown as far as that size, and its extra rows
/// neither break the comparison with what the viewer was last sent nor
/// go out.
#[test]
fn a_taller_frame_is_clipped_to_the_screen_the_viewer_was_told_of() {
    fn frame(words: &[u32]) -> Frame<'_> {
        Frame {
            words,
            width: tv::WIDTH,
            height: words.len() / tv::WORDS_PER_LINE,
            words_per_line: tv::WORDS_PER_LINE,
            black_on_white: false,
            colours: None,
        }
    }
    let (mut v, _) = Viewer::connect();
    let tall = tv::HEIGHT + 16;
    let mut words = vec![0u32; tall * tv::WORDS_PER_LINE];
    let (t, s) = (&mut v.terminal, &mut v.stream);
    let head = exchange_with(t, s, frame(&words), &update_request(false), 4);
    assert_eq!(u16::from_be_bytes([head[2], head[3]]), 1, "one rectangle");
    let r = exchange_with(t, s, frame(&words), &[], 12);
    let at = |k: usize| u16::from_be_bytes([r[k], r[k + 1]]);
    assert_eq!((at(0), at(2), at(4), at(6)), (0, 0, tv::WIDTH as u16, tv::HEIGHT as u16));
    exchange_with(t, s, frame(&words), &[], tv::WIDTH * tv::HEIGHT * 4);

    // A change on the screen and one below it: the one on the screen goes.
    words[5 * tv::WORDS_PER_LINE] = 1;
    words[(tv::HEIGHT + 3) * tv::WORDS_PER_LINE] = 1;
    let head = exchange_with(t, s, frame(&words), &update_request(true), 4);
    assert_eq!(u16::from_be_bytes([head[2], head[3]]), 1, "one rectangle");
    let r = exchange_with(t, s, frame(&words), &[], 12);
    let at = |k: usize| u16::from_be_bytes([r[k], r[k + 1]]);
    assert_eq!((at(2), at(6)), (5, 1), "row 5, and not the row below the screen");
}

/// **A full input queue loses key-downs before key-ups.** The keys wait
/// for the engine's next look; [`muir::terminal::INPUT_BACKLOG`] is how
/// many. Beyond that a key-down goes first, so that a key the machine saw
/// go down still comes up: a modifier whose up was lost would hold for
/// the rest of the run.
#[test]
fn a_full_input_queue_loses_key_downs_before_key_ups() {
    use muir::terminal::INPUT_BACKLOG;
    use muir::terminal::keyboard::keysym::CONTROL_L;
    let (mut v, _) = Viewer::connect();
    v.stream.write_all(&key_event(true, CONTROL_L)).unwrap();
    // Polled until it arrives and not a fixed number of times: a hundred
    // polls of an idle terminal are over in less time than a keystroke
    // takes to cross the loopback.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut got = Vec::new();
    while got.is_empty() && Instant::now() < deadline {
        v.terminal.poll(Frame::of(&v.tv));
        got.extend(v.terminal.take_keys());
    }
    assert_eq!(got, [(CONTROL_L, true)], "the machine saw control go down");

    let mut burst = key_event(false, CONTROL_L);
    for _ in 0..INPUT_BACKLOG {
        burst.extend(key_event(true, 'a' as u32));
        burst.extend(key_event(false, 'a' as u32));
    }
    v.stream.write_all(&burst).unwrap();
    // **Polled until the burst has arrived, and not for a fixed time.**
    // The queue is drained once, after everything is in, because what is
    // being measured is what a full queue kept --- so a burst still on
    // its way through the socket would be counted as a queue that had
    // room. A viewer's messages are read in the order it sent them, so an
    // update asked for after the burst is answered only once the whole
    // burst has been read off the socket: a pixel of the screen coming
    // back is the burst all in, which is delivery saying so rather than a
    // wait guessing at it. A wait is what this did before, and fifty
    // milliseconds of it was enough on an idle machine and not on a
    // loaded one, where it failed.
    v.update_rect(false, (0, 0, 1, 1), 4);
    let keys = v.terminal.take_keys();
    assert!(keys.len() <= INPUT_BACKLOG, "{} kept", keys.len());
    assert!(keys.contains(&(CONTROL_L, false)), "control's key-up is kept");
    let downs = keys.iter().filter(|k| k.1).count();
    assert!(downs < keys.len() - downs, "key-downs went before key-ups: {downs} of {}", keys.len());
}

impl Viewer {
    /// Sends `n` key-downs beyond what the input queue holds and polls
    /// until every one of them has been read off the socket.
    ///
    /// **Bounded by delivery and not by a wait.** A burst of nothing but
    /// key-downs leaves the queue standing at [`muir::terminal::INPUT_BACKLOG`]
    /// from its first loss on --- one event goes for each that arrives ---
    /// so the terminal's own count of what it lost says how much of the
    /// burst has arrived, and nothing has to be drained to ask. The ten
    /// seconds is the socket's outside bound, not a guess at how long it
    /// takes.
    fn lose_key_downs(&mut self, n: usize) {
        use muir::terminal::INPUT_BACKLOG;
        let was = self.terminal.keys_lost();
        // The queue has to be filled before anything can go, and it is
        // still full from the last time if anything has.
        let fill = if was == 0 { INPUT_BACKLOG } else { 0 };
        let mut burst = Vec::new();
        for _ in 0..fill + n {
            burst.extend(key_event(true, 'a' as u32));
        }
        self.stream.write_all(&burst).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.terminal.keys_lost() < was + n && Instant::now() < deadline {
            self.terminal.poll(Frame::of(&self.tv));
        }
        assert_eq!(self.terminal.keys_lost(), was + n, "the burst arrived and {n} of it went");
    }
}

/// **A terminal that loses nothing says nothing.** The count and the line
/// are for the run that misbehaves; nearly every run loses nothing, and a
/// run that loses nothing is told nothing.
#[test]
fn a_terminal_that_loses_nothing_says_nothing() {
    let (mut v, _) = Viewer::connect();
    let mut typed = key_event(true, 'a' as u32);
    typed.extend(key_event(false, 'a' as u32));
    typed.extend(pointer_event(1, 100, 200));
    v.stream.write_all(&typed).unwrap();
    // Until all three have arrived, rather than until the keys have: what
    // is being asked is what a terminal that lost nothing says, and a
    // pointer event still in the socket is a question not yet put.
    let deadline = Instant::now() + Duration::from_secs(10);
    let (mut keys, mut pointers) = (Vec::new(), Vec::new());
    while (keys.len() < 2 || pointers.is_empty()) && Instant::now() < deadline {
        v.terminal.poll(Frame::of(&v.tv));
        keys.extend(v.terminal.take_keys());
        pointers.extend(v.terminal.take_pointers());
    }
    assert_eq!(keys, [('a' as u32, true), ('a' as u32, false)], "the key went through");
    assert_eq!(pointers, [(1, 100, 200)], "and the pointer");
    assert_eq!(v.terminal.keys_lost(), 0, "nothing was lost");
    assert_eq!(v.terminal.pointers_lost(), 0);
    assert_eq!(v.terminal.lost_line(), None, "and there is nothing to say");
}

/// **A full input queue says how many key events it lost.** A keystroke
/// that never reaches the machine is a character that does not type, and
/// the queue's rule --- [`muir::terminal::INPUT_BACKLOG`] --- is right and
/// stays; what was wrong is that it happened in silence, and a person
/// looking at a key that would not type had no way to tell a lost
/// keystroke from a mapping with nothing in it.
///
/// One line a run without `--keyboard-mapping-trace`: a burst under load
/// is hundreds of events, and a run that loses typing is one thing to
/// say, not one thing an event.
#[test]
fn a_full_input_queue_says_how_many_key_events_it_lost() {
    let (mut v, _) = Viewer::connect();
    v.lose_key_downs(20);
    assert_eq!(v.terminal.keys_lost(), 20);
    assert_eq!(
        v.terminal.lost_line().as_deref(),
        Some(
            "terminal: the input queue was full: 20 key events lost; \
             --keyboard-mapping-trace says as more go"
        )
    );
    assert_eq!(v.terminal.lost_line(), None, "and the line is not repeated for the same loss");
    v.lose_key_downs(5);
    assert_eq!(v.terminal.keys_lost(), 25, "the count keeps up");
    assert_eq!(v.terminal.lost_line(), None, "and the run has been told once");
}

/// **The keyboard trace says it every time the count changes.**
/// `--keyboard-mapping-trace` is the flag a person reaches for when a key
/// will not type: it says what every keysym became, and a keysym the
/// queue lost became nothing at all. So under it the count is said as
/// often as it changes --- the count, and not one line an event, a burst
/// being hundreds of them.
#[test]
fn the_keyboard_trace_says_each_time_the_count_changes() {
    let (mut v, _) = Viewer::connect();
    v.terminal.trace_lost_keys = true;
    v.lose_key_downs(1);
    assert_eq!(
        v.terminal.lost_line().as_deref(),
        Some("terminal: the input queue was full: 1 key event lost, 1 in this run")
    );
    assert_eq!(v.terminal.lost_line(), None, "nothing until more go");
    v.lose_key_downs(11);
    assert_eq!(
        v.terminal.lost_line().as_deref(),
        Some("terminal: the input queue was full: 11 key events lost, 12 in this run"),
        "what went this time, and what the run has lost"
    );
}

/// **A run says what its terminal lost without being asked for it.**
/// [`Terminal::trace`] is on for every run muir serves --- it is what
/// prints a viewer coming and going --- and a run that has quietly lost a
/// hundred keystrokes is a run whose behaviour is unexplained. So
/// [`Terminal::poll`] says the line itself, and there is nothing left for
/// a caller to ask for afterwards.
#[test]
fn a_run_says_what_its_terminal_lost_without_being_asked() {
    let (mut v, _) = Viewer::connect();
    v.terminal.trace = true;
    v.lose_key_downs(3);
    assert_eq!(v.terminal.keys_lost(), 3, "three went");
    assert_eq!(v.terminal.lost_line(), None, "and the poll that lost them said so");
}

/// **A full pointer queue is counted and not said.** The pointer is an
/// absolute position and the mouse hands the machine the difference from
/// the last one it saw, so the position a lost event carried is not lost
/// with it: the newest is always kept, and the pointer ends where the
/// viewer put it. Nothing is printed for it --- a line about the mouse
/// under a keyboard flag would say that typing had gone astray when it had
/// not --- and the count is there to be asked for.
#[test]
fn a_full_pointer_queue_is_counted_and_not_said() {
    use muir::terminal::INPUT_BACKLOG;
    let (mut v, _) = Viewer::connect();
    let over = 3;
    let sent = INPUT_BACKLOG + over;
    let mut burst = Vec::new();
    for i in 0..sent {
        burst.extend(pointer_event(0, i as u16, 7));
    }
    v.stream.write_all(&burst).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while v.terminal.pointers_lost() < over && Instant::now() < deadline {
        v.terminal.poll(Frame::of(&v.tv));
    }
    assert_eq!(v.terminal.pointers_lost(), over, "the oldest three went");
    let pointers = v.terminal.take_pointers();
    assert_eq!(pointers.len() + over, sent, "every event sent is one kept or one lost");
    assert_eq!(
        pointers.last(),
        Some(&(0, sent as u16 - 1, 7)),
        "and where the pointer ended is kept"
    );
    assert_eq!(v.terminal.keys_lost(), 0, "no key event went");
    assert_eq!(v.terminal.lost_line(), None, "and a lost position is not worth a line");
}

/// **The keyboard holds a bounded backlog, and never drops a key-up.**
/// `ukbd.lisp`: "both pressing and releasing a key send a code, therefore
/// the central machine knows the status of all keys" --- a key-down the
/// machine has read whose key-up never follows is a key held for the rest
/// of the run. So while the software is not reading the keyboard the
/// words wait, up to [`keyboard::BACKLOG`] of them; a press beyond that is
/// refused whole and leaves nothing down, and a release always goes.
#[test]
fn the_keyboard_holds_a_bounded_backlog_and_never_drops_a_key_up() {
    use muir::terminal::keyboard::{self, Keyboard};
    let mut k = Keyboard::new();
    k.key('b' as u32, true);
    for _ in 0..keyboard::BACKLOG {
        k.key('a' as u32, true);
        k.key('a' as u32, false);
    }
    let full = k.pending();
    assert!((keyboard::BACKLOG..=keyboard::BACKLOG + 2).contains(&full), "{full} words waiting");
    let refused = k.refused();
    k.key('c' as u32, true);
    assert_eq!(k.pending(), full, "a press beyond the backlog is refused");
    assert_eq!(k.refused(), refused + 1, "and counted, each being a character that did not type");
    k.key('c' as u32, false);
    assert_eq!(k.pending(), full, "and leaves nothing to release");
    assert_eq!(k.refused(), refused + 1, "a release is never a refusal");
    k.key('b' as u32, false);
    assert_eq!(k.pending(), full + 1, "a release always goes");

    let mut words = Vec::new();
    while let Some(w) = k.take() {
        words.push(w);
    }
    assert_eq!(words.last(), Some(&keyboard::up_down(0o114, true)), "b's key-up, last");
    assert!(!words.iter().any(|&w| w & 0o177 == 0o164), "no word of c's");
    let mut down: Vec<u32> = Vec::new();
    for &w in &words {
        let position = w & 0o177;
        if w & keyboard::UP == 0 {
            down.push(position);
        } else {
            let k = down.iter().position(|&p| p == position).expect("an up for a key down");
            down.remove(k);
        }
    }
    assert!(down.is_empty(), "every key that went down came up: {down:?}");
}

/// **The cable takes a word only while it has room for it.** The keyboard's
/// firmware has a shift register and no queue, so the queue on the cable
/// is the terminal's; beyond [`keyboard::BACKLOG`] words `send` says no
/// and the caller keeps the word for a later offer, as `muir` does.
#[test]
fn the_cable_refuses_a_word_beyond_its_backlog() {
    use muir::terminal::cable::OnCable;
    use muir::terminal::keyboard;
    let n = muir::netlist::parse(include_str!("../data/CADRIO.netlist")).unwrap();
    let mut k = OnCable::of(&n).expect("the I/O board has the keyboard cable");
    for _ in 0..keyboard::BACKLOG {
        assert!(k.send(keyboard::up_down(0o123, false)));
    }
    assert!(!k.send(keyboard::up_down(0o123, true)), "the word beyond the backlog is refused");
    assert!(k.busy());
}

/// **The machine's beep reaches the viewer as a Bell.** RFC 6143 section
/// 7.6.3: message type 2 and nothing else, so there is no duration and no
/// pitch to send --- one bell for one beep is the whole of what the
/// protocol carries.
#[test]
fn a_beep_reaches_the_viewer_as_a_bell() {
    let (mut v, _) = Viewer::connect();
    v.terminal.ring();
    assert_eq!(v.exchange(&[], 1), vec![2], "Bell, RFC 6143 7.6.3");
}

/// **A bell with nobody listening is not kept for whoever connects next.**
/// A viewer arriving an hour later must not be told the machine beeped.
#[test]
fn a_bell_with_no_viewers_is_not_held() {
    let mut terminal = Terminal::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
    let tv = Tv::default();
    terminal.ring();
    terminal.poll(Frame::of(&tv));

    let mut stream = TcpStream::connect(terminal.addr().unwrap()).unwrap();
    stream.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    let frame = Frame::of(&tv);
    assert_eq!(&exchange_with(&mut terminal, &mut stream, frame, &[], 12)[..], rfb::VERSION);
    assert_eq!(exchange_with(&mut terminal, &mut stream, frame, rfb::VERSION, 2), vec![1, 1]);
    assert_eq!(exchange_with(&mut terminal, &mut stream, frame, &[1], 4), vec![0, 0, 0, 0]);
    // `ServerInit` and nothing before it: no bell arrived first.
    let head = exchange_with(&mut terminal, &mut stream, frame, &[1], 24);
    assert_eq!(u16::from_be_bytes([head[0], head[1]]), tv::WIDTH as u16, "ServerInit");
}

/// **Every pixel format a viewer may ask for is sent as
/// [`rfb::PixelFormat::put`] would write it, byte for byte.** A rectangle
/// goes out eight pixels at a time, out of a table built for the format;
/// `put` is the one-pixel-at-a-time statement of RFC 6143 section 7.4,
/// and this is what holds the one to the other. The screen is scrambled
/// first, so a table entry that is right for a run of equal bits and
/// wrong for a mixed one cannot pass.
#[test]
fn every_pixel_format_is_sent_as_put_would_write_it() {
    for f in formats() {
        for bow in [false, true] {
            let (mut v, _) = Viewer::connect();
            scramble(&mut v.tv);
            v.tv.write_control(0, if bow { tv::mode::BOW } else { 0 }, 0);
            v.set_format(f);
            let n = f.bytes_per_pixel().unwrap();
            let rects = v.update(false, n);
            assert_eq!(rects.len(), 1, "one rectangle for the whole screen");
            let want = as_put_would(Frame::of(&v.tv), f, (0, 0, tv::WIDTH, tv::HEIGHT));
            assert_eq!(rects[0].4.len(), want.len(), "{f:?} bow {bow}: as many bytes");
            assert!(rects[0].4 == want, "{f:?} bow {bow}: the pixels put would write");
        }
    }
}

/// **A rectangle that begins and ends inside a byte of the frame buffer is
/// sent as `put` would write it too.** The table carries eight pixels at a
/// time, so the pixels at either end of such a rectangle go one at a time;
/// this is the check that the two paths meet, and that a viewer entitled
/// to ask for an odd rectangle gets the same pixels as one asking for the
/// screen.
#[test]
fn a_rectangle_that_ends_inside_a_byte_is_sent_as_put_would_write_it() {
    // Between them: a ragged head and a ragged tail; a rectangle short
    // enough to hold no whole byte at all, from a byte's edge and from
    // inside one; one aligned at both ends, which is all table and no
    // ends; the last byte of a row; and a ragged head with an aligned
    // tail, which is what a viewer asking for all but the left edge
    // would send.
    for (x, w) in [(3usize, 13usize), (3, 5), (0, 7), (8, 16), (760, 8), (5, 763)] {
        let (mut v, _) = Viewer::connect();
        scramble(&mut v.tv);
        let (y, h) = (2usize, 4usize);
        let rects = v.update_rect(false, (x as u16, y as u16, w as u16, h as u16), 4);
        assert_eq!(rects.len(), 1, "one rectangle at {x}+{w}");
        assert_eq!(
            (rects[0].0, rects[0].1, rects[0].2, rects[0].3),
            (x as u16, y as u16, w as u16, h as u16)
        );
        let want = as_put_would(Frame::of(&v.tv), rfb::PixelFormat::RGB888, (x, y, w, h));
        assert!(rects[0].4 == want, "the pixels put would write at {x}+{w}");
    }
}

/// What [`rfb::PixelFormat::put`] would write for the rectangle `(x, y, w,
/// h)` of the colour screen, one pixel at a time: the colour of the
/// pixel's four bits, through the map, in the viewer's format --- or the
/// four bits themselves where the viewer asked for a mapped format, the
/// map having gone as `SetColourMapEntries`.
fn as_colour_put_would(
    tv: &Tv,
    f: rfb::PixelFormat,
    (x, y, w, h): (usize, usize, usize, usize),
) -> Vec<u8> {
    let mut out = Vec::new();
    for row in y..y + h {
        for col in x..x + w {
            let colour = tv.pixel4(col, row) as usize;
            let value = if f.true_colour { f.colour(tv.rgb(colour)) } else { colour as u32 };
            f.put(&mut out, value);
        }
    }
    out
}

/// **The colour screen is 576 by 454 at four bits a pixel, through the
/// map.** `ServerInit` says the size `COLOR:MAKE-SCREEN` gives, and every
/// pixel of the update is the colour its nibble names, in whatever format
/// the viewer asked for --- held to [`rfb::PixelFormat::put`] one pixel at
/// a time, as the black-and-white screen is.
#[test]
fn the_colour_screen_is_sent_through_the_map() {
    for f in formats() {
        let (mut v, init) = Viewer::connect_showing(Tv::color());
        assert_eq!(
            (u16::from_be_bytes([init[0], init[1]]), u16::from_be_bytes([init[2], init[3]])),
            (tv::COLOR_WIDTH as u16, tv::COLOR_HEIGHT as u16),
            "the colour screen's size"
        );
        // A map of sixteen different colours, and a picture that uses all
        // of them: `WRITE-COLOR-MAP` writes `377 - value` on channel
        // 0, 1 and 2 for red, green and blue.
        for colour in 0..tv::COLORS as u32 {
            for channel in 0..tv::CHANNELS as u32 {
                let value = (colour * 0o21 + channel * 0o5) & 0o377;
                v.tv.write_control(4, (0o377 - value) << 8 | channel << 6 | colour, 0);
            }
        }
        for k in 0..(tv::COLOR_HEIGHT * tv::COLOR_WORDS_PER_LINE) as u32 {
            v.tv.write_buffer(k, k.wrapping_mul(0x9e37_79b9) ^ k.rotate_left(13));
        }
        v.set_format(f);
        let n = f.bytes_per_pixel().unwrap();
        let whole = (0, 0, tv::COLOR_WIDTH as u16, tv::COLOR_HEIGHT as u16);
        let rects = v.update_rect(false, whole, n);
        assert_eq!(rects.len(), 1, "{f:?}: one rectangle, the whole screen");
        let (x, y, w, h, pixels) = &rects[0];
        assert_eq!((*x, *y, *w, *h), whole);
        assert_eq!(
            pixels,
            &as_colour_put_would(&v.tv, f, (0, 0, tv::COLOR_WIDTH, tv::COLOR_HEIGHT)),
            "{f:?}: the colours through the map"
        );
    }
}

/// **A map written while a viewer is looking repaints the screen.** The
/// buffer has not changed, so nothing the viewer holds says the picture
/// has; the colours it was sent no longer mean what they meant, and a
/// mapped viewer is told the new map as well.
#[test]
fn a_map_written_under_a_viewer_repaints_it() {
    let (mut v, _) = Viewer::connect_showing(Tv::color());
    // Every pixel colour 1, and colour 1 black.
    for k in 0..(tv::COLOR_HEIGHT * tv::COLOR_WORDS_PER_LINE) as u32 {
        v.tv.write_buffer(k, 0x1111_1111);
    }
    let whole = (0, 0, tv::COLOR_WIDTH as u16, tv::COLOR_HEIGHT as u16);
    let f = rfb::PixelFormat::RGB888;
    let rects = v.update_rect(false, whole, 4);
    assert_eq!(rects.len(), 1);
    assert_eq!(v.tv.rgb(1), [255, 255, 255], "an unwritten map shows full white");
    assert_eq!(
        rects[0].4,
        as_colour_put_would(&v.tv, f, (0, 0, tv::COLOR_WIDTH, tv::COLOR_HEIGHT)),
        "and the viewer has it"
    );

    // `WRITE-COLOR-MAP 1 0 0 0`: black, stored as 377 on every channel.
    for channel in 0..tv::CHANNELS as u32 {
        v.tv.write_control(4, 0o377 << 8 | channel << 6 | 1, 0);
    }
    // Incremental, and the whole screen comes back all the same.
    let rects = v.update_rect(true, whole, 4);
    assert_eq!(rects.len(), 1, "the map changed, so the screen did");
    assert_eq!((rects[0].0, rects[0].1, rects[0].2, rects[0].3), whole);
    assert_eq!(v.tv.rgb(1), [0, 0, 0], "and colour 1 is now black");
    assert!(rects[0].4.iter().all(|&b| b == 0), "so every pixel of it is");

    // Nothing changed since, so an incremental request is left outstanding.
    let mut request = vec![3u8, 1];
    for value in [whole.0, whole.1, whole.2, whole.3] {
        request.extend_from_slice(&value.to_be_bytes());
    }
    v.stream.write_all(&request).unwrap();
    for _ in 0..5 {
        v.terminal.poll(Frame::of(&v.tv));
    }
    let mut buf = [0u8; 1];
    assert!(v.stream.read(&mut buf).is_err(), "nothing more to send");
}

/// **A pixels-only terminal drops what a viewer types and points at.** The
/// machine has one keyboard and one mouse, both on the I/O board, and they
/// stay with the terminal that serves the main screen; the colour screen
/// is a second monitor and has neither. The bytes still come off the wire
/// --- the stream would desync otherwise --- and go nowhere, so the queues
/// stay empty and nothing is counted as lost.
#[test]
fn a_pixels_only_terminal_drops_the_keyboard_and_the_mouse() {
    let mut v = Viewer::open_showing(Tv::color());
    v.terminal.pixels_only = true;
    v.handshake();
    let mut typed = vec![4u8, 1, 0, 0];
    typed.extend_from_slice(&0x41u32.to_be_bytes());
    typed.extend_from_slice(&[5, 1]);
    typed.extend_from_slice(&100u16.to_be_bytes());
    typed.extend_from_slice(&200u16.to_be_bytes());
    // A `FramebufferUpdateRequest` after them, so that the poll below can
    // be waited on: the head of its answer is what says the messages
    // before it have all been read.
    typed.extend_from_slice(&update_request(false));
    v.stream.write_all(&typed).unwrap();
    let head = exchange_with(&mut v.terminal, &mut v.stream, Frame::of(&v.tv), &[], 4);
    assert_eq!(head[0], 0, "a FramebufferUpdate, so the messages before it were read");
    assert!(v.terminal.take_keys().is_empty(), "the key went nowhere");
    assert!(v.terminal.take_pointers().is_empty(), "nor did the pointer");
    assert_eq!(v.terminal.keys_lost(), 0, "and nothing was lost: it was never queued");
}
