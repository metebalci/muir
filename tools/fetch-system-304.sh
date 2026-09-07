#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mete Balci
# SPDX-License-Identifier: AGPL-3.0-or-later
# Fetch the System 304 release, which the engines also boot.
#
# System 304, microcode 323, is the release line that continues System 100,
# developed in the LM-3 project. It boots here, and `tests/chaos.rs` holds it
# to booting, but it is **not** what this project targets: CC does not load
# on it, and the acceptance test runs through CC. `tools/fetch-system-100.sh`
# fetches the target. It is not part of this repository --- `vendor/` is
# fetched material and is in .gitignore --- and without it the tests that
# want it skip cleanly and say so.
#
# Two files are used, and this puts them where the tests look for them:
#
#   sys-304-0.tar.gz       the system sources, unpacked to
#                          vendor/system-304-0/sys-304-0/
#   disk-sys-304-0.img.gz  the disk pack, decompressed to
#                          vendor/run/disk-sys-304-0.img
#
# Both come from muir's own GitHub release `system-304-0`, and every file
# is checked against its SHA-256 sum below, whether just downloaded or
# already in place; SYSTEM_304_BASE names another place to download from.
#
# The pack there is byte for byte the release as published at
# https://tumbleweed.nu/system-304-0-release/ and fetched from there on
# 7 September 2026. That release is the pack alone: no sources are
# published with it. So the sources are built from the project's own
# Fossil repository, https://tumbleweed.nu/r/sys/, at branch `system-304`,
# check-in c576cb32425c8b17e56e908c7f64baa5e2ab47a4 of 27 May 2025 --- the
# branch made at the version bump, the same day the pack was published ---
# with `fossil tarball --name sys-304-0`, which gives the same bytes every
# time it is run. Everything in both files is under the GNU Affero General
# Public License, version 3 or later, as the sources' own `doc/sys100.msg`
# says; muir's own licence is the same.
#
# It also makes the directory the Chaosnet server serves files from,
# vendor/run/file-root, with `sys` pointing at the release's sources. This
# band translates `SYS: SYS2; FOO LISP` to `OZ: //sys//sys2//foo.lisp`, so
# the sources belong at `sys` under the root and a link is all it takes for
# `SYS:` to resolve over the network. System 100's band asks for
# `/tree/...` instead, so that release's script makes a `tree` link in the
# same root and the two live side by side.
#
# The band's own host table puts this machine at `AMS-LISPM-1`, 4401 octal,
# and its file and time host `OZ` at 4403, which is not where muir answers
# unless told: a run with this pack wants `--chaos-address 4401,4403`, and
# without it the machine boots but asks for the date and reaches no files.
#
# Re-running this is safe: whatever is already in place is left alone, and
# nothing else in vendor/ is touched.
#
# usage: tools/fetch-system-304.sh

set -eu
muir=$(cd "$(dirname "$0")/.." && pwd)
base=${SYSTEM_304_BASE:-https://github.com/metebalci/muir/releases/download/system-304-0}
rel=$muir/vendor/system-304-0
run=$muir/vendor/run

# The SHA-256 of each file: the pack as fetched from upstream on
# 7 September 2026, the sources as `fossil tarball` writes them.
sum_of() {
    case $1 in
    sys-304-0.tar.gz) echo 9e9c2982e462383864d2ed393b80247a047298fb107fe7a43bfbf0bfa57ecc7d ;;
    disk-sys-304-0.img.gz) echo 87f0434b54dd86fa1df5251b04e0c48b86c804fa5c5fc6faba6dafa818e2e584 ;;
    esac
}

# The SHA-256 of a file, with whichever tool the system has.
sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

mkdir -p "$rel" "$run"

for f in sys-304-0.tar.gz disk-sys-304-0.img.gz; do
    if [ -f "$rel/$f" ]; then
        echo "have vendor/system-304-0/$f"
    else
        curl -fL -o "$rel/$f.part" "$base/$f"
        mv "$rel/$f.part" "$rel/$f"
    fi
    if [ "$(sha256 "$rel/$f")" != "$(sum_of "$f")" ]; then
        echo "vendor/system-304-0/$f is not the file the tests were written against:" >&2
        echo "its SHA-256 is not $(sum_of "$f"); move it away and run this again" >&2
        exit 1
    fi
    echo "     vendor/system-304-0/$f checks"
done

if [ -d "$rel/sys-304-0" ]; then
    echo "have vendor/system-304-0/sys-304-0"
else
    tar xzf "$rel/sys-304-0.tar.gz" -C "$rel"
fi

if [ -f "$run/disk-sys-304-0.img" ]; then
    echo "have vendor/run/disk-sys-304-0.img"
else
    gunzip -c "$rel/disk-sys-304-0.img.gz" > "$run/disk-sys-304-0.img.part"
    mv "$run/disk-sys-304-0.img.part" "$run/disk-sys-304-0.img"
fi

# Where `muir` serves files from over the Chaosnet when no root is named.
# A directory of its own, not `vendor/` and not the release: the FILE
# service writes, renames and deletes under its root, so a running
# machine should not have the fetched sources in reach by accident. The
# link is what puts them there deliberately, read and written as the band
# would.
#
# Whatever is already at `sys` is left exactly as it is --- a link, a
# real directory, or a link pointing nowhere.  `-L` is tested as well as
# `-e` because `-e` follows the link: a dangling one is false to `-e` and
# still very much there, and `ln` would then fail and, under `set -e`,
# take the whole script down.
root=$run/file-root
mkdir -p "$root"
if [ -e "$root/sys" ] || [ -L "$root/sys" ]; then
    echo "have vendor/run/file-root/sys"
else
    ln -s ../../system-304-0/sys-304-0 "$root/sys"
    echo "made vendor/run/file-root/sys -> the release's sources"
fi

echo "done; see the Install section of README.md"
