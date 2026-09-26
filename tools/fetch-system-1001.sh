#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mete Balci
# SPDX-License-Identifier: AGPL-3.0-or-later
# Fetch System 1001, the release that continues System 100.
#
# System 1001 continues System 100 with a source cleanup and unattended
# cold-load builds, on microcode 323. It is published as the GitHub release
# `release-1001` of https://github.com/metebalci/muir-sys, and like System
# 100 it is not part of this repository: `vendor/` is fetched material and
# is in .gitignore.
#
# Both of the release's files are used, and this puts them here:
#
#   release-1001-sys.tar.gz   the sources: the muir-sys repository at the
#                             release's tag, with `sys/ubin/` assembled,
#                             unpacked to vendor/system-1001/
#   release-1001-pack.img.gz  the disk pack, decompressed to
#                             vendor/run/release-1001-pack.img
#
# The license is in the sources: the GNU Affero General Public License,
# version 3 or later, muir's own, with `NOTICE` giving their provenance.
#
# Every file is checked against its SHA-256 sum below, whether just
# downloaded or already in place, and the pack again as it is decompressed.
# The sums are the ones the release's own notes publish; SYSTEM_1001_BASE
# names another place to download from.
#
# The band in the pack was built for its site's Chaosnet, not System 100's:
# the machine LISPM-1 at 177201, and its file, time and host table server
# OZ at 177200, which serves the sources' `sys` and `site` and a writable
# `lispm` home. So it is run with ozd at 177200 and muir at 177201, as the
# release's notes give the two commands. Unattended cold-load reporting
# uses MINI opcode 204, supported by ozd 26927a8 or later.
#
# Re-running this is safe: whatever is already in place is left alone, and
# nothing else in vendor/ is touched.
#
# usage: tools/fetch-system-1001.sh

set -eu
muir=$(cd "$(dirname "$0")/.." && pwd)
base=${SYSTEM_1001_BASE:-https://github.com/metebalci/muir-sys/releases/download/release-1001}
rel=$muir/vendor/system-1001
run=$muir/vendor/run

# The SHA-256 of each file, as the release's notes publish them.
sum_of() {
    case $1 in
    release-1001-sys.tar.gz) echo 507d1fe89e8a8890a42c4a8630ef5eddb058fbebebc4c66df7e33ea708a42269 ;;
    release-1001-pack.img.gz) echo 7ddf071b28a6e8b14fe0501683458ee8ae640291751e2c306e35316ee15ba713 ;;
    release-1001-pack.img) echo 279e597891a0d8263a82d52898dbc2194e7c0f19cbca194d8aaf5750226b7298 ;;
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

for f in release-1001-sys.tar.gz release-1001-pack.img.gz; do
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

# The tarball's one top directory, `release-1001/`, is left out, so the
# sources sit beside the tarball as System 100's do: vendor/system-1001/sys.
if [ -d "$rel/sys" ]; then
    echo "have vendor/system-1001/sys"
else
    tar xzf "$rel/release-1001-sys.tar.gz" -C "$rel" --strip-components=1
fi

# The pack is checked as it comes out and not afterwards: a machine writes
# to the pack it runs on, so the image in place is only the release's until
# the first run.
if [ -f "$run/release-1001-pack.img" ]; then
    echo "have vendor/run/release-1001-pack.img"
else
    gunzip -c "$rel/release-1001-pack.img.gz" > "$run/release-1001-pack.img.part"
    if [ "$(sha256 "$run/release-1001-pack.img.part")" != "$(sum_of release-1001-pack.img)" ]; then
        rm -f "$run/release-1001-pack.img.part"
        echo "vendor/system-1001/release-1001-pack.img.gz does not decompress to the pack the release published" >&2
        exit 1
    fi
    mv "$run/release-1001-pack.img.part" "$run/release-1001-pack.img"
    echo "     vendor/run/release-1001-pack.img checks"
fi

echo "done; the release's notes at https://github.com/metebalci/muir-sys/releases/tag/release-1001 say how to run it"
