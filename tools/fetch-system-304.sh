#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mete Balci
# SPDX-License-Identifier: AGPL-3.0-or-later
# Fetch the System 304 release, which the engines also boot.
#
# System 304, microcode 323, is the release line that continues System 100,
# developed in the LM-3 project. It boots here, and `tests/chaos.rs` holds it
# to booting; `tools/fetch-system-100.sh` fetches the target, System 100. It
# is not part of this repository --- `vendor/` is fetched material and is in
# .gitignore --- and without it the tests that want it skip cleanly and say
# so.
#
# **CC did not load on this release until 7 September 2026.** `cc/lcadrd.lisp`,
# `cc/diags.lisp` and `cc/ldbg.lisp` called `MAKE-ARRAY` in the old positional
# form --- `(MAKE-ARRAY NIL 'ART-Q '(8))` --- seven times between them, and
# the system had taken that form out, so the load of `LCADRD QFASL` stopped
# at "ART-Q is not a known MAKE-ARRAY keyword". Upstream rewrote all seven
# that day in three check-ins ending `1af7716f24`, which is why the sources
# below are built from a check-in later than the branch the pack was cut
# from, and why `tests/cc_304.rs` compiles and loads CC on this band.
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
# Fossil repository, https://tumbleweed.nu/r/sys/, on `trunk`, at check-in
# 1af7716f24ba298fef22805ac41819bc330dbb6b2885a16d86ab5c55f4e62e2f of
# 7 September 2026, with
#
#     fossil tarball --name sys-304-0 1af7716f24 sys-304-0.tar.gz -R sys.fossil
#
# which gives the same bytes every time it is run --- rebuilding the earlier
# check-in reproduces the earlier tarball's sum exactly, which is how this
# one is known to be the same build.
#
# **These sources are ahead of the pack's own band**, and deliberately.
# The pack is System 304.0, cut at `c576cb3242` on 27 May 2025; this
# check-in is that branch point plus five: the `TV:RH-GET-FUNCTION` hack of
# 3 September 2025 and the System 304.1 patch that ships it, and the three
# of 7 September 2026 that rewrite CC's seven `MAKE-ARRAY` calls. Nothing
# else differs --- five text files and `window/rh.lisp` --- and the CC
# files are the reason: with the branch point's sources CC cannot be
# loaded on this band at all.
#
# Everything in both files is under the GNU Affero General Public License,
# version 3 or later, as the sources' own `doc/sys100.msg` says; muir's own
# licence is the same.
#
# It also makes the directory the tests' Chaosnet server serves files from,
# vendor/run/file-root, with `sys` pointing at the release's sources. This
# band translates `SYS: SYS2; FOO LISP` to `OZ: //sys//sys2//foo.lisp`, so
# the sources belong at `sys` under the root and a link is all it takes for
# `SYS:` to resolve over the network. System 100's band asks for
# `/tree/...` instead, so that release's script makes a `tree` link in the
# same root and the two live side by side.
#
# The band's own host table puts this machine at `AMS-LISPM-1`, 4401 octal,
# and its file and time host `OZ` at 4403. muir is not that host --- a CADR
# has no file or time server in it --- so a run with this pack wants
# `--chaos-address 4401` and `--chaos-udp-peer 4403@<where the host is>`,
# and without them the machine boots but asks for the date and reaches no
# files. The tests put a server of their own on the modelled cable instead.
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
    sys-304-0.tar.gz) echo c945a464bfbed337358e79ba4ace8ef73c331e3a3a6c94c2c053624a463dfab1 ;;
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
