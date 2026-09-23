// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! MONO TV, QUUX's display: a monochrome 1280 by 1024 frame buffer by
//! default, one bit a pixel, 40 words a line, 40,960 words from Xbus
//! `17000000`, where the
//! CADR's TV buffer starts. Its mode register at `17377760` keeps
//! black-on-white, bit 2, and nothing else; registers 1 to 7 read 0 and
//! take writes to no effect; and it raises no interrupt, the clock being
//! the processor's tick. `--tv-board mono-tv`, QUUX only.

use muir::engine::Engine;
use muir::isa::Insn;
use muir::isa::asm::{ALU, MD, SETM, SRC_MD, START_READ, START_WRITE, a_dest, filler, m_src};
use muir::machine::{Geometry, Machine, bus_error};
use muir::micro::Micro;
use muir::rtl::Rtl;
use muir::terminal::Frame;
use muir::tv::{self, Board};

fn quux_with_mono_tv() -> Machine {
    let mut m = Machine::new();
    m.geometry = Geometry::QUUX;
    m.tv.set_board(Board::MonoTv);
    m
}

/// **The buffer is 40,960 words from `17000000`**: its first and last words
/// keep what is written, and the word after the last is nobody's, a read of
/// it timing out with the Xbus NXM bit, on both engines.
#[test]
fn the_buffer_is_40960_words() {
    // (virtual address, value): written through MD, read back into A.
    let last = 0o17000000 + tv::MONO_TV_WORDS - 1;
    // Page 1 onto the buffer's first page, 2 onto its last word's, 3 onto
    // the page of the word after it.
    let cases = [
        (0o400u32, 0o1234567u32),
        (0o1000 + (last & 0o377), 0o7654321),
        (0o1400 + ((last + 1) & 0o377), 0),
    ];
    let mut prom = Vec::new();
    for (k, &(va, _)) in cases.iter().enumerate() {
        let k = k as u64;
        prom.push(Insn::new(ALU | SETM | m_src(10 + k) | MD));
        prom.push(Insn::new(ALU | SETM | m_src(k + 1) | START_WRITE));
        prom.extend([filler(); 12]);
        prom.push(Insn::new(ALU | SETM | m_src(k + 1) | START_READ));
        prom.extend([filler(); 12]);
        prom.push(Insn::new(ALU | SETM | SRC_MD | a_dest(0o200 + k)));
        let _ = va;
    }
    let make = |prom: &[Insn]| {
        let mut m = quux_with_mono_tv();
        let mut words = vec![filler(); 512];
        words[..prom.len()].copy_from_slice(prom);
        m.load_prom(&words);
        let rw = (1 << 23) | (1 << 22);
        m.l2_map[1] = rw | (0o17000000 >> 8);
        m.l2_map[2] = rw | (last >> 8);
        m.l2_map[3] = rw | ((last + 1) >> 8);
        for (k, &(va, v)) in cases.iter().enumerate() {
            m.mmem[1 + k] = va;
            m.mmem[10 + k] = v;
        }
        m
    };
    // Each case is 28 instructions: the first two alone, then all three.
    for (cases_run, nxm) in [(2, false), (3, true)] {
        let mut e = Micro::new(make(&prom[..28 * cases_run]));
        e.boot();
        let mut r = Rtl::new(make(&prom[..28 * cases_run]));
        r.boot();
        for _ in 0..2000 {
            e.step().unwrap();
            r.step().unwrap();
        }
        for (name, m) in [("micro", e.machine()), ("rtl", r.machine())] {
            assert_eq!(m.amem[0o200], 0o1234567, "{name}: the first word");
            assert_eq!(m.amem[0o201], 0o7654321, "{name}: the last word");
            assert_eq!(m.tv.read_buffer(tv::MONO_TV_WORDS - 1), 0o7654321, "{name}: in the buffer");
            let got = m.bus_error & bus_error::XBUS_NXM != 0;
            assert_eq!(got, nxm, "{name}: NXM after {cases_run} cases");
        }
    }
}

/// **Pixel `x` of line `y` is bit `x mod 32` of word `40 y + x / 32`**, the
/// low bit leftmost as on the CADR's TV, and the frame the terminal draws
/// is 1280 by 1024.
#[test]
fn a_pixel_is_where_the_cadr_would_put_it_at_40_words_a_line() {
    let mut m = quux_with_mono_tv();
    let (x, y) = (1279usize, 1023usize);
    m.bus_write(0o17000000 + (y * 40 + x / 32) as u32, 1 << (x % 32));
    assert!(m.tv.pixel(x, y));
    assert!(!m.tv.pixel(x - 1, y));
    let f = Frame::of(&m.tv);
    assert_eq!((f.width, f.height, f.words_per_line), (1280, 1024, 40));
    assert_eq!(f.visible(), tv::MONO_TV_WORDS as usize);
    assert!(f.shows_white(x, y), "a one shows white while black-on-white is off");
    m.bus_write(tv::CONTROL, tv::mode::BOW);
    assert!(!Frame::of(&m.tv).shows_white(x, y), "and black with it on");
}

/// **The mode register keeps black-on-white alone, register 4 answers and
/// takes no writes yet, and nothing interrupts**, the interrupt enable
/// written or not, over a second of the machine's time. Register 4 is the
/// color map's write, kept for a color display to come; registers 1 to 3
/// and 5 to 7 --- the CADR's sync program and three that did nothing --- are
/// not there, and an access times out with the Xbus NXM bit.
#[test]
fn it_keeps_black_on_white_and_never_interrupts() {
    let mut m = quux_with_mono_tv();
    m.bus_write(tv::CONTROL, 0o377);
    assert_eq!(m.bus_read(tv::CONTROL), tv::mode::BOW);
    m.bus_write(tv::CONTROL + 4, 0o177777);
    assert_eq!(m.bus_read(tv::CONTROL + 4), 0, "register 4");
    assert_eq!(m.bus_error & bus_error::XBUS_NXM, 0, "registers 0 and 4 answer");
    for r in [1, 2, 3, 5, 6, 7] {
        m.bus_error = 0;
        m.bus_read(tv::CONTROL + r);
        assert_ne!(m.bus_error & bus_error::XBUS_NXM, 0, "register {r} read");
        m.bus_error = 0;
        m.bus_write(tv::CONTROL + r, 1);
        assert_ne!(m.bus_error & bus_error::XBUS_NXM, 0, "register {r} written");
    }
    for ms in 0..1000u64 {
        m.ns = ms * 1_000_000;
        assert!(!m.xbus_interrupt(), "an interrupt at {ms} ms");
    }
}

/// **The feature page describes the main screen** in three words: 11 is
/// the width in 31:16 and the height in 15:0, 12 the bits a pixel in 31:16
/// and the words a line in 15:0, and 13 the buffer's first physical address
/// --- MONO TV's, or the CADR's TV's when QUUX has that board. Word 14 is 0.
#[test]
fn the_feature_page_describes_the_main_screen() {
    let page = 0o17377000;
    let words = |m: &mut Machine| [0o11, 0o12, 0o13, 0o14].map(|w| m.bus_read(page + w));
    let packed = |hi: u32, lo: u32| hi << 16 | lo;
    assert_eq!(words(&mut quux_with_mono_tv()), [packed(1280, 1024), packed(1, 40), 0o17000000, 0]);
    let mut m = Machine::new();
    m.geometry = Geometry::QUUX;
    assert_eq!(words(&mut m), [packed(768, 963), packed(1, 24), 0o17000000, 0]);
}

/// **The bus interface acknowledges the whole buffer**: `rtl` takes a cycle
/// to the buffer's last word as a device's, not as a timeout, and one past
/// it as nobody's. What the word is and whether NXM is set are
/// `Machine::bus_read`'s, which the test above holds; this is the decode
/// that makes the cycle's timing, which the CADR's boards' 32K words do not
/// reach.
#[test]
fn the_bus_interface_answers_the_whole_buffer() {
    use muir::busint::{Responder, decode_for};
    let last = 0o17000000 + tv::MONO_TV_WORDS - 1;
    let m = quux_with_mono_tv();
    let words = m.tv.buffer_words();
    assert_eq!(words, tv::MONO_TV_WORDS);
    let all = 0xff;
    assert_eq!(decode_for(last, 1 << 20, false, words, all), Responder::Device);
    assert_eq!(decode_for(last + 1, 1 << 20, false, words, all), Responder::NoXbus);
    assert_eq!(decode_for(last, 1 << 20, false, tv::BUFFER_WORDS, all), Responder::NoXbus);
    let regs = m.tv.control_registers();
    assert_eq!(decode_for(tv::CONTROL, 1 << 20, false, words, regs), Responder::Device);
    assert_eq!(decode_for(tv::CONTROL + 4, 1 << 20, false, words, regs), Responder::Device);
    assert_eq!(decode_for(tv::CONTROL + 1, 1 << 20, false, words, regs), Responder::NoXbus);
}

/// **MONO TV can be another size, `--mono-tv-size`**, and the feature page,
/// the frame, the buffer's end and a checkpoint all follow it: 2560 by
/// 1440 is 80 words a line and 115,200 words, past the color TV's strap.
#[test]
fn another_size_is_followed_everywhere() {
    let mut m = quux_with_mono_tv();
    m.tv.set_mono_tv_size(2560, 1440);
    assert_eq!(m.tv.screen(), (2560, 1440, 80));
    assert_eq!(m.tv.buffer_words(), 115_200);
    let page = 0o17377000;
    assert_eq!(m.bus_read(page + 0o11), 2560 << 16 | 1440);
    assert_eq!(m.bus_read(page + 0o12), 1 << 16 | 80);
    let f = Frame::of(&m.tv);
    assert_eq!((f.width, f.height, f.words_per_line), (2560, 1440, 80));
    let last = 0o17000000 + 115_200 - 1;
    m.bus_write(last, 5);
    assert_eq!(m.bus_read(last), 5);
    assert_eq!(m.bus_error & bus_error::XBUS_NXM, 0);
    m.bus_read(last + 1);
    assert_ne!(m.bus_error & bus_error::XBUS_NXM, 0, "one past the end");

    use muir::checkpoint::{Reader, Writer};
    let mut w = Writer::new();
    m.tv.save(&mut w);
    let body = w.finish();
    let mut back = tv::Tv::default();
    back.load(&mut Reader::new(&body)).unwrap();
    assert_eq!((back.board(), back.screen()), (Board::MonoTv, (2560, 1440, 80)));
    assert_eq!(back.read_buffer(115_200 - 1), 5);
}

/// **A size is refused unless a line is whole words and the buffer fits**:
/// below the feature page, and below the color TV's strap when one is
/// fitted.
#[test]
fn a_size_is_checked() {
    use tv::check_mono_tv_size as check;
    assert!(check(1280, 1024, false).is_ok());
    assert!(check(1280, 1024, true).is_ok());
    assert!(check(1920, 1080, false).is_ok());
    assert!(check(1920, 1080, true).is_ok());
    assert!(check(2560, 1440, false).is_ok());
    assert!(check(2560, 1440, true).is_err(), "over the color TV's buffer");
    assert!(check(1921, 1080, false).is_err(), "not whole words");
    assert!(check(3840, 2160, false).is_err(), "259,200 words");
    assert!(check(0, 1080, false).is_err());
}
