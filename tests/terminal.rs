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

use muir::simpletv::{self, SimpleTv};
use muir::terminal::{Frame, Terminal, rfb};

/// A viewer, blocking, with the server it is talking to.
struct Viewer {
    terminal: Terminal,
    stream: TcpStream,
    tv: SimpleTv,
}

impl Viewer {
    /// Binds a terminal to a port the host picks and connects to it, with
    /// nothing said yet.
    fn open() -> Viewer {
        let terminal = Terminal::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
        let stream = TcpStream::connect(terminal.addr().unwrap()).unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        Viewer { terminal, stream, tv: SimpleTv::default() }
    }

    /// [`Viewer::open`], then RFC 6143's opening exchange as 3.8 as far as
    /// `ServerInit`, which it returns.
    fn connect() -> (Viewer, Vec<u8>) {
        let mut v = Viewer::open();
        assert_eq!(&v.exchange(&[], 12)[..], rfb::VERSION, "the version the server offers");
        // 3.8: one security type on offer, and it is None.
        assert_eq!(v.exchange(rfb::VERSION, 2), vec![1, 1], "one type, and it is None");
        assert_eq!(v.exchange(&[1], 4), vec![0, 0, 0, 0], "SecurityResult, and it is ok");
        // ClientInit's shared flag, then ServerInit: 24 bytes and a name.
        let head = v.exchange(&[1], 24);
        let name = u32::from_be_bytes(head[20..24].try_into().unwrap()) as usize;
        let rest = v.exchange(&[], name);
        (v, [head, rest].concat())
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
        let mut request = vec![3u8, incremental as u8];
        for v in [0u16, 0, simpletv::WIDTH as u16, simpletv::HEIGHT as u16] {
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
    assert_eq!(u16::from_be_bytes([init[0], init[1]]), simpletv::WIDTH as u16, "width");
    assert_eq!(u16::from_be_bytes([init[2], init[3]]), simpletv::HEIGHT as u16, "height");
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
/// Every pixel of it, against [`SimpleTv::shows_white`] --- so a viewer
/// and the PNG cannot disagree about which way round the screen is.
#[test]
fn the_first_update_is_the_whole_screen() {
    let (mut v, _) = Viewer::connect();
    // Something to see: a word at the top left, a word further down, and
    // the mode register left white-on-black.
    v.tv.write_buffer(0, 0x0f0f_0f0f);
    v.tv.write_buffer(100 * simpletv::WORDS_PER_LINE as u32 + 3, 0xffff_ffff);

    let rects = v.update(false, 4);
    assert_eq!(rects.len(), 1, "one rectangle for the whole screen");
    let (x, y, w, h, pixels) = &rects[0];
    assert_eq!((*x, *y, *w, *h), (0, 0, simpletv::WIDTH as u16, simpletv::HEIGHT as u16));

    let white = rfb::PixelFormat::RGB888.white();
    let mut checked = 0;
    for row in 0..simpletv::HEIGHT {
        for col in 0..simpletv::WIDTH {
            let at = (row * simpletv::WIDTH + col) * 4;
            let got = u32::from_le_bytes(pixels[at..at + 4].try_into().unwrap());
            let want = if v.tv.shows_white(col, row) { white } else { 0 };
            assert_eq!(got, want, "pixel {col},{row}");
            checked += 1;
        }
    }
    assert_eq!(checked, simpletv::WIDTH * simpletv::HEIGHT, "every pixel of the screen");
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
    v.tv.write_buffer(7 * simpletv::WORDS_PER_LINE as u32 + 2, 0xdead_beef);
    let rects = v.update(true, 4);
    assert_eq!(rects.len(), 1, "one rectangle: {rects:?}");
    let (x, y, w, h, _) = rects[0];
    assert_eq!((x, y, w, h), (0, 7, simpletv::WIDTH as u16, 1), "the one row that moved");

    // Two rows next to each other come back as one rectangle.
    for row in [20u32, 21] {
        v.tv.write_buffer(row * simpletv::WORDS_PER_LINE as u32, 1);
    }
    let rects = v.update(true, 4);
    assert_eq!(rects.len(), 1, "two touching rows are one rectangle: {rects:?}");
    assert_eq!((rects[0].1, rects[0].3), (20, 2), "at row 20, two rows high");

    // And two apart come back as two.
    v.tv.write_buffer(30 * simpletv::WORDS_PER_LINE as u32, 1);
    v.tv.write_buffer(60 * simpletv::WORDS_PER_LINE as u32, 1);
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
    assert_eq!(pixels[simpletv::WIDTH - 1], 0, "and the far end of the row is black");
}

/// **Black-on-white swaps every pixel.** `MODE BOW` is the display
/// board's own bit, and the viewer sees what the monitor would.
#[test]
fn the_mode_registers_bow_bit_swaps_the_screen() {
    let (mut v, _) = Viewer::connect();
    v.tv.write_buffer(0, 1);
    let plain = v.update(false, 4)[0].4.clone();
    v.tv.write_control(0, simpletv::mode::BOW, 0);
    let swapped = v.update(false, 4)[0].4.clone();
    let white = rfb::PixelFormat::RGB888.white().to_le_bytes();
    assert_eq!(&plain[0..4], &white, "the lit bit is white to start with");
    assert_eq!(&swapped[0..4], &[0, 0, 0, 0], "and black once BOW is set");
    assert_eq!(&plain[4..8], &[0, 0, 0, 0], "its neighbour is black");
    assert_eq!(&swapped[4..8], &white, "and white once BOW is set");
}

/// **A frame and the frame buffer agree on which way round the screen
/// is.** The rule lives in [`SimpleTv::shows_white`] and again in
/// [`Frame::shows_white`], because a frame may be a monitor's raster and
/// not that buffer; this is the check that the second copy says the same
/// as the first.
#[test]
fn a_frame_shows_what_the_frame_buffer_shows() {
    let mut tv = SimpleTv::default();
    for (k, word) in [0x0000_0001u32, 0xffff_ffff, 0xaaaa_5555, 0x8000_0000].iter().enumerate() {
        tv.write_buffer(k as u32, *word);
    }
    for bow in [false, true] {
        tv.write_control(0, if bow { simpletv::mode::BOW } else { 0 }, 0);
        let frame = Frame::of(&tv);
        assert_eq!(frame.black_on_white, bow);
        for y in 0..4 {
            for x in 0..simpletv::WIDTH {
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
    // Nothing is expected back, so poll until the events have arrived.
    for _ in 0..100 {
        v.terminal.poll(Frame::of(&v.tv));
    }
    assert_eq!(v.terminal.take_keys(), vec![(0x41, true), (0x41, false)], "the key, down then up");
    assert_eq!(v.terminal.take_pointers(), vec![(1, 100, 200)], "the pointer");
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

/// A `FramebufferUpdateRequest` for the whole screen the viewer was told of.
fn update_request(incremental: bool) -> Vec<u8> {
    let mut b = vec![3u8, incremental as u8];
    for v in [0u16, 0, simpletv::WIDTH as u16, simpletv::HEIGHT as u16] {
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
    assert_eq!(u16::from_be_bytes([head[0], head[1]]), simpletv::WIDTH as u16, "ServerInit");
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
    let tv = SimpleTv::default();
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
            width: simpletv::WIDTH,
            height: words.len() / simpletv::WORDS_PER_LINE,
            words_per_line: simpletv::WORDS_PER_LINE,
            black_on_white: false,
        }
    }
    let (mut v, _) = Viewer::connect();
    let tall = simpletv::HEIGHT + 16;
    let mut words = vec![0u32; tall * simpletv::WORDS_PER_LINE];
    let (t, s) = (&mut v.terminal, &mut v.stream);
    let head = exchange_with(t, s, frame(&words), &update_request(false), 4);
    assert_eq!(u16::from_be_bytes([head[2], head[3]]), 1, "one rectangle");
    let r = exchange_with(t, s, frame(&words), &[], 12);
    let at = |k: usize| u16::from_be_bytes([r[k], r[k + 1]]);
    assert_eq!(
        (at(0), at(2), at(4), at(6)),
        (0, 0, simpletv::WIDTH as u16, simpletv::HEIGHT as u16)
    );
    exchange_with(t, s, frame(&words), &[], simpletv::WIDTH * simpletv::HEIGHT * 4);

    // A change on the screen and one below it: the one on the screen goes.
    words[5 * simpletv::WORDS_PER_LINE] = 1;
    words[(simpletv::HEIGHT + 3) * simpletv::WORDS_PER_LINE] = 1;
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
    let mut got = Vec::new();
    for _ in 0..100 {
        v.terminal.poll(Frame::of(&v.tv));
        got.extend(v.terminal.take_keys());
        if !got.is_empty() {
            break;
        }
    }
    assert_eq!(got, [(CONTROL_L, true)], "the machine saw control go down");

    let mut burst = key_event(false, CONTROL_L);
    for _ in 0..INPUT_BACKLOG {
        burst.extend(key_event(true, 'a' as u32));
        burst.extend(key_event(false, 'a' as u32));
    }
    v.stream.write_all(&burst).unwrap();
    for _ in 0..50 {
        v.terminal.poll(Frame::of(&v.tv));
        std::thread::sleep(Duration::from_millis(1));
    }
    let keys = v.terminal.take_keys();
    assert!(keys.len() <= INPUT_BACKLOG, "{} kept", keys.len());
    assert!(keys.contains(&(CONTROL_L, false)), "control's key-up is kept");
    let downs = keys.iter().filter(|k| k.1).count();
    assert!(downs < keys.len() - downs, "key-downs went before key-ups: {downs} of {}", keys.len());
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
    k.key('c' as u32, true);
    assert_eq!(k.pending(), full, "a press beyond the backlog is refused");
    k.key('c' as u32, false);
    assert_eq!(k.pending(), full, "and leaves nothing to release");
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
