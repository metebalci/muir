// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Remote Framebuffer protocol, RFC 6143 --- what a VNC viewer speaks.
//!
//! RFB is the wire protocol; VNC is the name of the software that speaks
//! it. This is the protocol alone: the handshake, the pixel format, the
//! six messages a viewer sends and the two the server sends back. What is
//! on the screen and who is connected is [`super::Terminal`]'s.
//!
//! The authority is RFC 6143, *The Remote Framebuffer Protocol*, T.
//! Richardson and J. Levine, March 2011, which is the protocol MIT never
//! heard of --- nothing here is a claim about the CADR. It is written from
//! the RFC and not from a crate, because the parts of it a black-and-white
//! frame buffer needs are small: a fixed handshake, one encoding, and a
//! rectangle of pixels.
//!
//! **Raw encoding only.** RFC 6143 section 7.7.1: "Every server MUST
//! support" it, so a viewer that offers nothing else still works and one
//! that offers everything is answered in the one encoding both ends are
//! obliged to have. The screen is one bit a pixel and the updates are
//! whole rows, so there is nothing here that a run-length encoding would
//! win enough to pay for.

/// How a viewer wants pixels laid out: RFC 6143 section 7.4.
///
/// The screen is black and white, so of all this only three things
/// matter --- how many bytes a pixel takes, which way round they go, and
/// what value means white. [`PixelFormat::white`] and
/// [`PixelFormat::black`] are the whole of the colour handling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelFormat {
    pub bits_per_pixel: u8,
    pub depth: u8,
    pub big_endian: bool,
    pub true_colour: bool,
    pub red_max: u16,
    pub green_max: u16,
    pub blue_max: u16,
    pub red_shift: u8,
    pub green_shift: u8,
    pub blue_shift: u8,
}

impl PixelFormat {
    /// What the server offers in its `ServerInit`: 32 bits a pixel, eight
    /// each of red, green and blue in the low three bytes, little-endian.
    /// Viewers may ask for something else and this one answers whatever
    /// they ask; it is offered because it is the format every viewer
    /// accepts without asking for another.
    pub const RGB888: PixelFormat = PixelFormat {
        bits_per_pixel: 32,
        depth: 24,
        big_endian: false,
        true_colour: true,
        red_max: 255,
        green_max: 255,
        blue_max: 255,
        red_shift: 16,
        green_shift: 8,
        blue_shift: 0,
    };

    /// The sixteen bytes of the wire form.
    pub fn encode(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0] = self.bits_per_pixel;
        b[1] = self.depth;
        b[2] = self.big_endian as u8;
        b[3] = self.true_colour as u8;
        b[4..6].copy_from_slice(&self.red_max.to_be_bytes());
        b[6..8].copy_from_slice(&self.green_max.to_be_bytes());
        b[8..10].copy_from_slice(&self.blue_max.to_be_bytes());
        b[10] = self.red_shift;
        b[11] = self.green_shift;
        b[12] = self.blue_shift;
        // 13, 14, 15 are padding and stay zero.
        b
    }

    pub fn parse(b: &[u8]) -> PixelFormat {
        PixelFormat {
            bits_per_pixel: b[0],
            depth: b[1],
            big_endian: b[2] != 0,
            true_colour: b[3] != 0,
            red_max: u16::from_be_bytes([b[4], b[5]]),
            green_max: u16::from_be_bytes([b[6], b[7]]),
            blue_max: u16::from_be_bytes([b[8], b[9]]),
            red_shift: b[10],
            green_shift: b[11],
            blue_shift: b[12],
        }
    }

    /// Bytes a pixel takes on the wire. A viewer may name 8, 16 or 32
    /// bits; anything else is refused when it arrives, because writing a
    /// pixel that is not a whole number of bytes is not something RFC 6143
    /// asks for.
    pub fn bytes_per_pixel(&self) -> Option<usize> {
        matches!(self.bits_per_pixel, 8 | 16 | 32).then_some(self.bits_per_pixel as usize / 8)
    }

    /// Whether the format is one a pixel can be written in: a whole number
    /// of bytes, and shifts that name bits of it.  RFC 6143 section 7.4
    /// gives each shift as "the number of shifts needed to get the red
    /// value in a pixel to the least significant bit", so one at or past
    /// the pixel's width names none; a viewer sending such a format is
    /// refused, as one naming 24 bits a pixel is, rather than shifted by.
    pub fn fits(&self) -> bool {
        let width = self.bits_per_pixel as u32;
        let shifts = [self.red_shift, self.green_shift, self.blue_shift];
        self.bytes_per_pixel().is_some()
            && (!self.true_colour || shifts.iter().all(|&s| (s as u32) < width))
    }

    /// The pixel value for white: every colour at its maximum, or colour
    /// map entry 1 where the viewer wants a map.  A shift off the word,
    /// which [`PixelFormat::fits`] refuses on arrival, contributes nothing.
    pub fn white(&self) -> u32 {
        if !self.true_colour {
            return WHITE_INDEX as u32;
        }
        let at = |max: u16, shift: u8| (max as u32).checked_shl(shift as u32).unwrap_or(0);
        at(self.red_max, self.red_shift)
            | at(self.green_max, self.green_shift)
            | at(self.blue_max, self.blue_shift)
    }

    /// The pixel value for black: zero either way, which is colour map
    /// entry 0 as [`colour_map`] sets it.
    pub fn black(&self) -> u32 {
        0
    }

    /// One pixel, in as many bytes and whichever order the viewer asked
    /// for.
    pub fn put(&self, out: &mut Vec<u8>, value: u32) {
        let n = self.bytes_per_pixel().unwrap_or(4);
        let b = value.to_be_bytes();
        if self.big_endian {
            out.extend_from_slice(&b[4 - n..]);
        } else {
            out.extend(b[4 - n..].iter().rev());
        }
    }
}

/// The colour map entry white takes when a viewer asks for a mapped
/// format rather than a true-colour one.
pub const WHITE_INDEX: u8 = 1;

/// `SetColourMapEntries`, RFC 6143 section 7.6.2: the two colours this
/// screen has, black at 0 and white at 1, sent to a viewer that asked for
/// a mapped format.
pub fn colour_map() -> Vec<u8> {
    let mut m = vec![1u8, 0];
    m.extend_from_slice(&0u16.to_be_bytes()); // first colour
    m.extend_from_slice(&2u16.to_be_bytes()); // how many
    for _ in 0..3 {
        m.extend_from_slice(&0u16.to_be_bytes()); // black
    }
    for _ in 0..3 {
        m.extend_from_slice(&u16::MAX.to_be_bytes()); // white
    }
    m
}

/// The version the server offers, RFC 6143 section 7.1.1. A viewer that
/// answers with an older one is spoken to in that one:
/// [`Version::security`] is the whole of the difference here.
pub const VERSION: &[u8; 12] = b"RFB 003.008\n";

/// Which of RFC 6143's three versions a viewer answered with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    /// 3.3, section 7.1.2: the server names the one security type as a
    /// 32-bit word and the viewer does not answer.
    V3_3,
    /// 3.7 and 3.8: the server lists the types, the viewer picks one.
    /// 3.8 adds a `SecurityResult` after the pick, which 3.7 has not.
    V3_7,
    V3_8,
}

impl Version {
    /// Reads a viewer's twelve-byte answer. RFC 6143 section 7.1.1: "The
    /// only published protocol versions at this time are 3.3, 3.7, and
    /// 3.8. Other version numbers are reported by some servers and
    /// clients, but should be interpreted as 3.3 since they do not
    /// implement the different handshake in 3.7 or 3.8." So anything but
    /// 3.7 and 3.8 is 3.3 --- Apple's Screen Sharing, which answers `RFB
    /// 003.889`, gets 3.3's handshake --- and a viewer that then talks
    /// something else fails on its next message rather than here.
    pub fn parse(b: &[u8]) -> Version {
        match b {
            b"RFB 003.007\n" => Version::V3_7,
            b"RFB 003.008\n" => Version::V3_8,
            _ => Version::V3_3,
        }
    }

    /// What the server sends once the version is settled, and whether a
    /// viewer's choice of security type is to be read back.
    ///
    /// The only type offered is 1, None: RFC 6143 section 7.2.1, "no
    /// authentication is needed". `super::Terminal` is bound to the
    /// loopback address unless told otherwise for exactly that reason.
    pub fn security(&self) -> (Vec<u8>, bool) {
        match self {
            // 3.3: the type as a word, and nothing to read.
            Version::V3_3 => (1u32.to_be_bytes().to_vec(), false),
            // 3.7 and 3.8: one type on offer, and the viewer names it.
            _ => (vec![1, 1], true),
        }
    }

    /// Whether a `SecurityResult` follows the viewer's choice. 3.8 only.
    pub fn wants_security_result(&self) -> bool {
        *self == Version::V3_8
    }
}

/// A failed `SecurityResult`, RFC 6143 section 7.1.3: the word 1, "failed",
/// then "a string describing the reason for the failure" --- a `U32` length
/// and that many ASCII characters, as section 7.1.2 lays a string out ---
/// after which the server "closes the connection". For a 3.8 viewer that
/// picks a type not offered; 3.7 has no reason string, and 3.3 makes no
/// choice.
pub fn security_failed(reason: &str) -> Vec<u8> {
    let mut m = 1u32.to_be_bytes().to_vec();
    m.extend_from_slice(&(reason.len() as u32).to_be_bytes());
    m.extend_from_slice(reason.as_bytes());
    m
}

/// A message from the viewer: RFC 6143 section 7.5.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMessage {
    SetPixelFormat(PixelFormat),
    /// The encodings the viewer will take. Kept for the record and not
    /// acted on: Raw is the one every viewer must have and the only one
    /// sent.
    SetEncodings(Vec<i32>),
    FramebufferUpdateRequest {
        incremental: bool,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
    },
    /// A key went down or came up, by X11 keysym.
    Key {
        down: bool,
        keysym: u32,
    },
    Pointer {
        buttons: u8,
        x: u16,
        y: u16,
    },
    /// The viewer's clipboard, which has nowhere to go on a CADR: how
    /// many bytes of text follow the head, which the caller drops as they
    /// arrive.  The length is the viewer's 32 bits, so the text is never
    /// held whole.
    CutText {
        len: usize,
    },
}

/// Takes one message off the front of `buf`, and says how many bytes it
/// used.
///
/// `Ok(None)` is a message that has not all arrived --- the caller keeps
/// the bytes and asks again. `Err` is a message this server does not
/// know, which RFC 6143 gives no way to skip, the length being implied by
/// the type: the connection has to go.
pub fn parse(buf: &[u8]) -> Result<Option<(ClientMessage, usize)>, String> {
    let need = |n: usize| buf.len() >= n;
    let Some(&kind) = buf.first() else { return Ok(None) };
    match kind {
        0 => {
            if !need(20) {
                return Ok(None);
            }
            Ok(Some((ClientMessage::SetPixelFormat(PixelFormat::parse(&buf[4..20])), 20)))
        }
        2 => {
            if !need(4) {
                return Ok(None);
            }
            let count = u16::from_be_bytes([buf[2], buf[3]]) as usize;
            if !need(4 + 4 * count) {
                return Ok(None);
            }
            let list = (0..count)
                .map(|k| i32::from_be_bytes(buf[4 + 4 * k..8 + 4 * k].try_into().unwrap()))
                .collect();
            Ok(Some((ClientMessage::SetEncodings(list), 4 + 4 * count)))
        }
        3 => {
            if !need(10) {
                return Ok(None);
            }
            let at = |k: usize| u16::from_be_bytes([buf[k], buf[k + 1]]);
            Ok(Some((
                ClientMessage::FramebufferUpdateRequest {
                    incremental: buf[1] != 0,
                    x: at(2),
                    y: at(4),
                    w: at(6),
                    h: at(8),
                },
                10,
            )))
        }
        4 => {
            if !need(8) {
                return Ok(None);
            }
            let keysym = u32::from_be_bytes(buf[4..8].try_into().unwrap());
            Ok(Some((ClientMessage::Key { down: buf[1] != 0, keysym }, 8)))
        }
        5 => {
            if !need(6) {
                return Ok(None);
            }
            let at = |k: usize| u16::from_be_bytes([buf[k], buf[k + 1]]);
            Ok(Some((ClientMessage::Pointer { buttons: buf[1], x: at(2), y: at(4) }, 6)))
        }
        6 => {
            if !need(8) {
                return Ok(None);
            }
            let len = u32::from_be_bytes(buf[4..8].try_into().unwrap()) as usize;
            Ok(Some((ClientMessage::CutText { len }, 8)))
        }
        _ => Err(format!("message type {kind}, whose length RFC 6143 does not give")),
    }
}

/// `ServerInit`, RFC 6143 section 7.3.2: the screen's size, the format
/// offered, and a name for the window.
pub fn server_init(width: u16, height: u16, name: &str) -> Vec<u8> {
    let mut m = Vec::with_capacity(24 + name.len());
    m.extend_from_slice(&width.to_be_bytes());
    m.extend_from_slice(&height.to_be_bytes());
    m.extend_from_slice(&PixelFormat::RGB888.encode());
    m.extend_from_slice(&(name.len() as u32).to_be_bytes());
    m.extend_from_slice(name.as_bytes());
    m
}

/// The head of a `FramebufferUpdate`, RFC 6143 section 7.6.1: the message
/// type and how many rectangles follow.
pub fn update_header(rectangles: u16) -> [u8; 4] {
    let mut h = [0u8; 4];
    h[2..].copy_from_slice(&rectangles.to_be_bytes());
    h
}

/// The head of one rectangle in Raw encoding: where it is, how big, and
/// encoding 0.
pub fn rectangle_header(x: u16, y: u16, w: u16, h: u16) -> [u8; 12] {
    let mut r = [0u8; 12];
    r[0..2].copy_from_slice(&x.to_be_bytes());
    r[2..4].copy_from_slice(&y.to_be_bytes());
    r[4..6].copy_from_slice(&w.to_be_bytes());
    r[6..8].copy_from_slice(&h.to_be_bytes());
    // Encoding 0, Raw: the four bytes are already zero.
    r
}
