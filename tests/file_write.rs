// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! **The band writes a file through the Chaosnet server's FILE service,
//! and it goes into place.**
//!
//! The write half of the protocol ends in a handshake across two
//! connections: `sys/doc/chfile.text` has the client send "a SYNC mark on
//! the DATA connection and a CLOSE on the CONTROL connection (in either
//! order)", and the server awaits the mark before it renames its
//! temporary over the real name.  Since it awaits the mark, the CLOSE's
//! own reply waits too, and this is the test that the wait ends: A is
//! booted to its listener, asked for something with its output on a file
//! under `OZ:`, and the file is there afterwards with no temporary beside
//! it.
//!
//! `qfile.lisp`'s `:COMMAND` sends the command packet and then, for an
//! output stream, `(SEND STREAM :WRITE-SYNCHRONOUS-MARK)` before it waits
//! for the response, so holding the reply back cannot hold the mark back
//! with it --- but that is read from the source, and this is the run.
//! Both releases, because the two bands are different machines and the
//! client is the one thing this leans on.
//!
//! Ignored by default: about eight seconds a release at `rtl`'s rate.
//! Needs a pack and the file root, and says it was skipped without them.
//!
//!     cargo test --release --test file_write -- --ignored --nocapture

mod cc_harness;
mod support;

use cc_harness::Release;

#[test]
#[ignore = "boots the band: seconds; run with --ignored"]
fn the_band_writes_a_file_through_the_file_service() {
    a_file_goes_into_place_on(Release::System100);
}

#[test]
#[ignore = "boots the band: seconds; run with --ignored"]
fn the_304_band_writes_a_file_through_the_file_service() {
    a_file_goes_into_place_on(Release::System304);
}

/// Microcycles of A's allowed for the write to be renamed into place
/// after the answer has been read: the CLOSE and its mark are one round
/// trip over the model network, and the answer can be read out of the
/// service's own temporary before that.
const PLACING: u64 = 2_000_000_000;

fn a_file_goes_into_place_on(release: Release) {
    let Some(mut cc) = cc_harness::boot_and_login_on(release, false) else {
        eprintln!("skipped: no pack, or no file root; tools/fetch-system-100.sh");
        return;
    };
    let name = "file-write";
    let who = cc.ask(name, "(princ si:user-id)", 4_000_000_000);
    assert!(who.contains("LISPM"), "logged in, and the answer came back: {who:?}");

    let tmp = cc.root.join("tmp");
    let file = tmp.join(format!("{name}.text"));
    let from = cc.l.steps.0;
    while !file.exists() {
        cc.run(5_000_000);
        assert!(cc.l.steps.0 - from < PLACING, "the write never went into place");
    }
    // The temporary is what the file was written into; the rename is
    // what puts it away. One left behind is a CLOSE that never
    // completed.
    let left: Vec<String> = std::fs::read_dir(&tmp)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('#'))
        .collect();
    assert!(left.is_empty(), "a temporary was left behind: {left:?}");
    eprintln!(
        "placed by {} microcycles: {} bytes",
        cc.l.steps.0,
        std::fs::metadata(&file).unwrap().len()
    );
}
