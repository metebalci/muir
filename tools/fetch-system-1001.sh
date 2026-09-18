#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mete Balci
# SPDX-License-Identifier: AGPL-3.0-or-later
# Fetch LMZ System 1001, the release that continues System 100.
#
# LMZ 1001 continues System 100 with a source cleanup and unattended
# cold-load builds. It is published as the GitHub release
# `lmz-1001` of https://github.com/metebalci/lmz, and like System 100 it is
# not part of this repository: `vendor/` is fetched material and is in
# .gitignore.
#
# Both of the release's files are used, and this puts them here:
#
#   lmz-1001-sys.tar.gz   the system sources, `sys/` and `site/` with
#                         `LICENSE` and `NOTICE`, unpacked to
#                         vendor/system-1001/
#   lmz-1001-pack.img.gz  the disk pack, decompressed to
#                         vendor/run/lmz-1001-pack.img
#
# The license is in the sources: the GNU Affero General Public License,
# version 3 or later, muir's own, with `NOTICE` giving their provenance.
#
# Every file is checked against its SHA-256 sum below, whether just
# downloaded or already in place, and the pack again as it is decompressed.
# The sums are the ones the release's own notes publish; SYSTEM_1001_BASE
# names another place to download from.
#
# The band in the pack was built for the LMZ site's Chaosnet, not System
# 100's: the machine LISPM-1 at 177201, and its file, time and host table
# server OZ at 177200, which serves the sources' `sys` and `site` and a
# writable `lispm` home. So it is run with ozd at 177200 and muir at 177201,
# as the release's notes give the two commands. Unattended cold-load
# reporting uses MINI opcode 204, supported by ozd 8c816e5 or later.
#
# Re-running this is safe: whatever is already in place is left alone, and
# nothing else in vendor/ is touched.
#
# usage: tools/fetch-system-1001.sh

set -eu
muir=$(cd "$(dirname "$0")/.." && pwd)
base=${SYSTEM_1001_BASE:-https://github.com/metebalci/lmz/releases/download/lmz-1001}
rel=$muir/vendor/system-1001
run=$muir/vendor/run

# The SHA-256 of each file, as the release's notes publish them.
sum_of() {
    case $1 in
    lmz-1001-sys.tar.gz) echo dc83a333f2f2703ef224c1551406e5d208c94b0ced6d44827fe0b35caf4c6378 ;;
    lmz-1001-pack.img.gz) echo 70a620e28feade762f27a0b4d6408e942e4d9495b3c81a6f0530dd7e28070b05 ;;
    lmz-1001-pack.img) echo 35b15e7e947bdcd0e3b3994ca107c247d599ac6127b13b5d8d9029281de1364c ;;
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

for f in lmz-1001-sys.tar.gz lmz-1001-pack.img.gz; do
    if [ -f "$rel/$f" ]; then
        echo "have vendor/system-1001/$f"
    else
        curl -fL -o "$rel/$f.part" "$base/$f"
        mv "$rel/$f.part" "$rel/$f"
    fi
    if [ "$(sha256 "$rel/$f")" != "$(sum_of "$f")" ]; then
        echo "vendor/system-1001/$f is not the file the release published:" >&2
        echo "its SHA-256 is not $(sum_of "$f"); move it away and run this again" >&2
        exit 1
    fi
    echo "     vendor/system-1001/$f checks"
done

# The tarball's one top directory, `lmz-1001/`, is left out, so the
# sources sit beside the tarball as System 100's do: vendor/system-1001/sys.
if [ -d "$rel/sys" ]; then
    echo "have vendor/system-1001/sys"
else
    tar xzf "$rel/lmz-1001-sys.tar.gz" -C "$rel" --strip-components=1
fi

# The pack is checked as it comes out and not afterwards: a machine writes
# to the pack it runs on, so the image in place is only the release's until
# the first run.
if [ -f "$run/lmz-1001-pack.img" ]; then
    echo "have vendor/run/lmz-1001-pack.img"
else
    gunzip -c "$rel/lmz-1001-pack.img.gz" > "$run/lmz-1001-pack.img.part"
    if [ "$(sha256 "$run/lmz-1001-pack.img.part")" != "$(sum_of lmz-1001-pack.img)" ]; then
        rm -f "$run/lmz-1001-pack.img.part"
        echo "vendor/system-1001/lmz-1001-pack.img.gz does not decompress to the pack the release published" >&2
        exit 1
    fi
    mv "$run/lmz-1001-pack.img.part" "$run/lmz-1001-pack.img"
    echo "     vendor/run/lmz-1001-pack.img checks"
fi

echo "done; the release's notes at https://github.com/metebalci/lmz/releases/tag/lmz-1001 say how to run it"
