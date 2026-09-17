#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mete Balci
# SPDX-License-Identifier: AGPL-3.0-or-later
# Fetch LMZ System 1000, the release that continues System 100.
#
# LMZ 1000 is MIT's System 99.32, as LM-3 brought it up as System 100, fixed
# so that it rebuilds itself. It is published as the GitHub release
# `lmz-1000` of https://github.com/metebalci/lmz, and like System 100 it is
# not part of this repository: `vendor/` is fetched material and is in
# .gitignore.
#
# Both of the release's files are used, and this puts them here:
#
#   lmz-1000-sys.tar.gz   the system sources, `sys/` and `site/` with
#                         `LICENSE` and `NOTICE`, unpacked to
#                         vendor/system-1000/
#   lmz-1000-pack.img.gz  the disk pack, decompressed to
#                         vendor/run/lmz-1000-pack.img
#
# The license is in the sources: the GNU Affero General Public License,
# version 3 or later, muir's own, with `NOTICE` giving their provenance.
#
# Every file is checked against its SHA-256 sum below, whether just
# downloaded or already in place, and the pack again as it is decompressed.
# The sums are the ones the release's own notes publish; SYSTEM_1000_BASE
# names another place to download from.
#
# The band in the pack was built for the Z54 site's Chaosnet, not System
# 100's: the machine LISPM-1 at 177201, and its file, time and host table
# server OZ at 177200, which serves the sources' `sys` and `site` and a
# writable `lispm` home. So it is run with ozd at 177200 and muir at 177201,
# as the release's notes give the two commands. A pack of the release as
# first published looks for OZ at 177201, and these sums refuse it.
#
# Re-running this is safe: whatever is already in place is left alone, and
# nothing else in vendor/ is touched.
#
# usage: tools/fetch-system-1000.sh

set -eu
muir=$(cd "$(dirname "$0")/.." && pwd)
base=${SYSTEM_1000_BASE:-https://github.com/metebalci/lmz/releases/download/lmz-1000}
rel=$muir/vendor/system-1000
run=$muir/vendor/run

# The SHA-256 of each file, as the release's notes publish them.
sum_of() {
    case $1 in
    lmz-1000-sys.tar.gz) echo 9492a0ca770f4f8098e7f04f68ee57c7261c4f5c6b568a26d1416e105b46d954 ;;
    lmz-1000-pack.img.gz) echo d486d5acd29dadfc8110dd5c2dc71668c72542649bd9c3f9a34337f0c83c6383 ;;
    lmz-1000-pack.img) echo 195608a498a54da3eb96ead5b19dd9814465d616e30334750171bfa99b2ddedf ;;
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

for f in lmz-1000-sys.tar.gz lmz-1000-pack.img.gz; do
    if [ -f "$rel/$f" ]; then
        echo "have vendor/system-1000/$f"
    else
        curl -fL -o "$rel/$f.part" "$base/$f"
        mv "$rel/$f.part" "$rel/$f"
    fi
    if [ "$(sha256 "$rel/$f")" != "$(sum_of "$f")" ]; then
        echo "vendor/system-1000/$f is not the file the release published:" >&2
        echo "its SHA-256 is not $(sum_of "$f"); move it away and run this again" >&2
        exit 1
    fi
    echo "     vendor/system-1000/$f checks"
done

# The tarball's one top directory, `lmz-1000/`, is left out, so the
# sources sit beside the tarball as System 100's do: vendor/system-1000/sys.
if [ -d "$rel/sys" ]; then
    echo "have vendor/system-1000/sys"
else
    tar xzf "$rel/lmz-1000-sys.tar.gz" -C "$rel" --strip-components=1
fi

# The pack is checked as it comes out and not afterwards: a machine writes
# to the pack it runs on, so the image in place is only the release's until
# the first run.
if [ -f "$run/lmz-1000-pack.img" ]; then
    echo "have vendor/run/lmz-1000-pack.img"
else
    gunzip -c "$rel/lmz-1000-pack.img.gz" > "$run/lmz-1000-pack.img.part"
    if [ "$(sha256 "$run/lmz-1000-pack.img.part")" != "$(sum_of lmz-1000-pack.img)" ]; then
        rm -f "$run/lmz-1000-pack.img.part"
        echo "vendor/system-1000/lmz-1000-pack.img.gz does not decompress to the pack the release published" >&2
        exit 1
    fi
    mv "$run/lmz-1000-pack.img.part" "$run/lmz-1000-pack.img"
    echo "     vendor/run/lmz-1000-pack.img checks"
fi

echo "done; the release's notes at https://github.com/metebalci/lmz/releases/tag/lmz-1000 say how to run it"
