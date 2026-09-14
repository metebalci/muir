// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The color TV under System 100: the cold boot finds the board, sets its
//! sync going and writes its colour map.
//!
//! Needs the vendored pack and file root; without them the test says it
//! was skipped.

use muir::engine::Engine;
use muir::rtl::Rtl;
use muir::tv::{self, mode};

mod support;
use support::{CHAOS_100, boot_to_the_prompt, machine_with_pack};

/// **A cold boot with the color TV fitted sets the board up and still
/// reaches the listener.**
///
/// What the release does, in order:
///
/// - `WINDOW-INITIALIZE` in `sys/window/shwarm.lisp` sends `:EXPOSE` to
///   every screen, and `COLOR-SCREEN` is one of them: `color.lisp`'s
///   `(ADD-INITIALIZATION "Color Make Screen" '(SETQ COLOR-SCREEN
///   (MAKE-SCREEN)) '(ONCE))` put it there when the file was loaded into
///   the band.
/// - The `:EXPOSE` wrapper calls `COLOR-EXISTS-P`, which writes 1 into the
///   first word of the colour buffer with the error stop off and reads it
///   back (`XBUS-LOCATION-EXISTS-P`, `XBUS-READ-NO-PARITY`). With no board
///   that is an NXM and the boot moves on; with one it reads back and the
///   screen is taken to exist.
/// - `COLOR:SETUP` then runs `SI:STOP-SYNC`, `SI:FILL-SYNC` with
///   `COLOR:SYNC` --- "This is really NTSC standard video" --- and
///   `(SI:START-SYNC 3 0 36.)`, which `CC-TV-START-SYNC` in
///   `sys/cc/dmon.lisp` writes as the mode `(+ (LSH BOW 2) CLOCK)` and
///   register 3 as `(+ 200 VSP)`: clock mode 3, the sync RAM in, vertical
///   spacing 36.
/// - Then `R-G-B-COLOR-MAP`, sixteen `WRITE-COLOR-MAP`s of three
///   `%XBUS-WRITE-SYNC`es each. Each of those spins in the microcode
///   (`XXBWS` in `sys/ucadr/uc-cadr.lisp`) until `MODE HSYNC` goes low and
///   then high before it writes register 4, so the map is only written at
///   all because the model runs the board's sync program.
///
/// The boot reaches the listener in about 13 million microcycles, which is
/// a second and a half of `rtl`: the model disk controller takes no time
/// over the band. So it runs with the rest of the suite rather than under
/// `--ignored`.
#[test]
fn the_cold_boot_sets_up_the_color_tv_and_writes_its_map() {
    let (Some(pack), Some(root)) = (support::pack_100(), support::file_root()) else {
        return;
    };
    let mut m = machine_with_pack(&pack);
    m.fit_color_tv();
    let mut e = Rtl::new(m);
    e.boot();
    // The band is System 100's, and it calls its file and time host at its
    // own host table's numbers.
    let ran = boot_to_the_prompt(&mut e, CHAOS_100, root);
    let colour = e.machine().color_tv.as_ref().expect("the board is on the backplane");
    eprintln!(
        "at the prompt after {ran}: colour mode {:o}, register 3 {:o}, {} sync words loaded",
        colour.mode(),
        colour.sync.enable,
        colour.sync.words().iter().filter(|&&w| w != 0).count()
    );

    // `(SI:START-SYNC 3 0 36.)`: clock mode 3, black-on-white clear, the
    // interrupt enable clear --- which is why the colour board's vertical
    // flag never reaches `-XBUS.INTR`, where `INTRX0` would have nothing
    // to clear it with.
    assert_eq!(colour.mode() & tv::mode::CLOCK, 3, "clock mode 3");
    assert_eq!(colour.mode() & mode::BOW, 0, "BOW clear");
    assert_eq!(colour.mode() & mode::INTERRUPT_ENABLE, 0, "and the interrupt enable clear");
    assert!(colour.sync.enabled(), "the sync RAM is in");
    assert_eq!(colour.sync.enable & 0o177, 36, "vertical spacing 36, MIT's 'for color'");

    // The program loaded is `COLOR:SYNC`, which makes NTSC's 525 lines in
    // two fields --- `tests/sync_program.rs` walks the same program out of
    // the release and gets the same frame.
    let timeline = colour.timeline().expect("the loaded program makes a frame");
    assert_eq!(timeline.lines.len(), 525, "NTSC's lines");
    assert_eq!(timeline.tvma_clr.len(), 2, "a TVMA CLR at the top of each field");
    assert_eq!(
        timeline.lines.iter().filter(|l| l.video_cycles == 37).count(),
        tv::COLOR_HEIGHT,
        "the picture lines of both fields"
    );

    // The main board is where the cold boot left it, which is with its own
    // sync RAM empty: `tests/vertical.rs` holds that, and the colour board
    // being set up does not set the main one up.
    assert!(!e.machine().tv.sync.enabled(), "the main board is untouched");

    // `R-G-B-COLOR-MAP`: colour 0 all off, and colour I the `(NTH (\\ I 3)
    // R-G-B)` of `((ON OFF OFF) (OFF ON OFF) (OFF OFF ON))` with
    // `COLOR-MAP-ON` 377 and `COLOR-MAP-OFF` 0 --- stored by
    // `WRITE-COLOR-MAP` as `377 - value`, so an on channel is 0 and an off
    // one 377.
    const ON: u8 = 0o377;
    const OFF: u8 = 0;
    let written = |value: u8| ON - value;
    let mut want = [[written(OFF); tv::CHANNELS]; tv::COLORS];
    for (i, entry) in want.iter_mut().enumerate().skip(1) {
        entry[i % 3] = written(ON);
    }
    assert_eq!(colour.color_map(), &want, "the map R-G-B-COLOR-MAP wrote");

    // And what the monitor is shown, which is the map inverted back: black
    // at 0, then red, green and blue over and over.
    assert_eq!(colour.rgb(0), [0, 0, 0], "colour 0 is black");
    assert_eq!(colour.rgb(1), [0, 255, 0], "1 mod 3 is green");
    assert_eq!(colour.rgb(2), [0, 0, 255]);
    assert_eq!(colour.rgb(3), [255, 0, 0]);
    assert_eq!(colour.rgb(0o17), colour.rgb(3), "15 mod 3 is red again");
}
