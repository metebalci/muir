#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mete Balci
# SPDX-License-Identifier: AGPL-3.0-or-later
# Fetch the System 100 release, which the engines boot from.
#
# System 100, microcode 323, is what this project targets: the restored last
# MIT release, pulled from TID/671 in the MIT Tapes of Tech Square project.
# It is not part of this repository --- `vendor/` is fetched material and is
# in .gitignore --- and without it every test and example that needs a pack
# skips cleanly and says so.
#
# Two of the release's files are used, and this puts them where the tests
# look for them:
#
#   sys-100-0.tar.gz       the system sources, unpacked to
#                          vendor/system-100-0/sys/
#   disk-sys-100-0.img.gz  the disk pack, decompressed to
#                          vendor/run/disk-sys-100-0.img
#
# with the release's README beside them, which carries its licence, the
# GNU Affero General Public License, version 3 or later --- muir's own.
#
# The files come from muir's own GitHub release `system-100-0`, a mirror of
# the release as published at https://tumbleweed.nu/system-100-0-release/
# and fetched from there on 31 August 2026, byte for byte under the
# upstream names. The mirror is there so that the bytes the tests were
# written against stay the bytes: upstream publishes no checksums and
# carries a patch directory, and a CI job should not depend on one
# private server being up. Every file is checked against its SHA-256 sum
# below, whether just downloaded or already in place; SYSTEM_100_BASE
# names another place to download from, the upstream directory for one.
#
# It also makes the directory the Chaosnet server serves files from,
# vendor/run/file-root, with `tree` pointing at the release's sources.
# The band's site file makes its file host OZ the SYS host and translates
# `SYS: SYS2; FOO LISP` to `/tree/sys2/foo.lisp` there, so the release's
# own `sys` directory is exactly what OZ had at `/tree`, and a link is
# all it takes for `SYS:` to resolve over the network.
#
# The release also carries two load bands, LOD1 and LOD2, and an emulator.
# Nothing here reads them yet, so they are not fetched.
#
# Re-running this is safe: whatever is already in place is left alone, and
# nothing else in vendor/ is touched.
#
# usage: tools/fetch-system-100.sh

set -eu
muir=$(cd "$(dirname "$0")/.." && pwd)
base=${SYSTEM_100_BASE:-https://github.com/metebalci/muir/releases/download/system-100-0}
rel=$muir/vendor/system-100-0
run=$muir/vendor/run

# The SHA-256 of each file, as fetched from upstream on 31 August 2026.
sum_of() {
    case $1 in
    sys-100-0.tar.gz) echo 349e5fc0ea25cdfcfd594c72ffc061b0c1f7ead65d567c4c57c743d1171b7a2d ;;
    disk-sys-100-0.img.gz) echo bab08874cc35ab129b40daf1602dbaf28e9fe818a81042148c3deb8d70a465a0 ;;
    README) echo 53dc494918d43cf2460d15ae9a38bf6b16264a1c02bc13798fdd4bad80c08937 ;;
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

for f in README sys-100-0.tar.gz disk-sys-100-0.img.gz; do
    if [ -f "$rel/$f" ]; then
        echo "have vendor/system-100-0/$f"
    else
        curl -fL -o "$rel/$f.part" "$base/$f"
        mv "$rel/$f.part" "$rel/$f"
    fi
    if [ "$(sha256 "$rel/$f")" != "$(sum_of "$f")" ]; then
        echo "vendor/system-100-0/$f is not the file the tests were written against:" >&2
        echo "its SHA-256 is not $(sum_of "$f"); move it away and run this again" >&2
        exit 1
    fi
    echo "     vendor/system-100-0/$f checks"
done

if [ -d "$rel/sys" ]; then
    echo "have vendor/system-100-0/sys"
else
    tar xzf "$rel/sys-100-0.tar.gz" -C "$rel"
fi

if [ -f "$run/disk-sys-100-0.img" ]; then
    echo "have vendor/run/disk-sys-100-0.img"
else
    gunzip -c "$rel/disk-sys-100-0.img.gz" > "$run/disk-sys-100-0.img.part"
    mv "$run/disk-sys-100-0.img.part" "$run/disk-sys-100-0.img"
fi

# Where `muir` serves files from over the Chaosnet when no root is named.
# A directory of its own, not `vendor/` and not the release: the FILE
# service writes, renames and deletes under its root, so a running
# machine should not have the fetched sources in reach by accident. The
# link is what puts them there deliberately, read and written as the band
# would.
#
# Whatever is already at `tree` is left exactly as it is --- a link, a
# real directory, or a link pointing nowhere.  `-L` is tested as well as
# `-e` because `-e` follows the link: a dangling one is false to `-e` and
# still very much there, and `ln` would then fail and, under `set -e`,
# take the whole script down.
root=$run/file-root
mkdir -p "$root"
if [ -e "$root/tree" ] || [ -L "$root/tree" ]; then
    echo "have vendor/run/file-root/tree"
else
    ln -s ../../system-100-0/sys "$root/tree"
    echo "made vendor/run/file-root/tree -> the release's sources"
fi

echo "done; see the Install section of README.md"
