#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mete Balci
# SPDX-License-Identifier: AGPL-3.0-or-later
# Fetch QUUX's current release, System 2000.
#
# QUUX runs muir-sys's system and no other: System 2000, on microcode 2000
# and boot PROM 2000, runs only on QUUX hardware revision 11 or later. It is
# the GitHub release `release-2000` of https://github.com/metebalci/muir-sys,
# and like System 100 it is not part of this repository: `vendor/` is
# fetched material and is in .gitignore.
#
# The release's files, and where this puts them:
#
#   SHA256SUMS                the release's own sums, kept beside the rest
#   release-2000-sys.tar.gz   the sources: the muir-sys repository at the
#                             release's tag, with `sys/ubin/` assembled,
#                             unpacked to vendor/system-2000/
#   release-2000-disk.vhd.gz  QUUX's disk, a dynamic VHD holding a GPT,
#                             decompressed to vendor/run/release-2000-disk.vhd
#   release-2000-promh.mcr    boot PROM 2000, kept in vendor/system-2000/;
#                             `quux`'s own PROM is the same, byte for byte
#
# and it makes the machine's host folder, its file root,
# vendor/run/release-2000-root/: copies of the sources' `sys/` and `site/`
# and an empty `home/lispm/`, the login's home: a login needs none, but a
# file written to the home needs it to exist. The machine writes, renames
# and deletes under its root, so the root is a directory of its own and the
# checked sources are never in its reach; they are copies and not links
# because the file device refuses a link that leaves its mount. The root is
# made once and left alone afterwards, whatever is in it.
#
# The license is in the sources: the GNU Affero General Public License,
# version 3 or later, muir's own, with `NOTICE` giving their provenance.
#
# The release is pinned: its tag and every file's SHA-256 are below, and a
# newer release is taken by a commit that changes them, never by following
# GitHub's latest release. Every file is checked against its sum, whether
# just downloaded or already in place, and the disk again as it is
# decompressed. SYSTEM_FOR_QUUX_BASE names another place to download from.
#
# The number and the format say the machine, and both are checked before
# anything is unpacked: the sources must unpack to one `release-2NNN/`
# directory, a QUUX system's number, and the disk must be a dynamic VHD
# (`conectix` in its last 512 bytes) holding a GPT (`EFI PART` at the start
# of its sector 1, found through the VHD's block table). The CADR's
# releases, numbered from 1000 and on a LABL pack, are refused.
#
# Re-running this is safe: whatever is already in place is left alone, and
# nothing else in vendor/ is touched.
#
# usage: tools/fetch-system-for-quux.sh
# then:  quux --disk-pack vendor/run/release-2000-disk.vhd \
#             --file-root vendor/run/release-2000-root

set -eu
muir=$(cd "$(dirname "$0")/.." && pwd)
tag=release-2000
base=${SYSTEM_FOR_QUUX_BASE:-https://github.com/metebalci/muir-sys/releases/download/$tag}
rel=$muir/vendor/system-${tag#release-}
vrel=vendor/system-${tag#release-}
run=$muir/vendor/run
sources=$tag-sys.tar.gz
disk=$tag-disk.vhd
prom=$tag-promh.mcr
root=$run/$tag-root
assets="SHA256SUMS $sources $disk.gz $prom"

# The SHA-256 of each file, as the release's SHA256SUMS and notes give them.
sum_of() {
    case $1 in
    SHA256SUMS) echo 99e8f34aeeba038f9072fdd9db517ae231aecca25e84027d35b46dd25e1514fa ;;
    release-2000-sys.tar.gz) echo e953c7d844c787ec43a9d5939badbd31d0f745a792a0b64f78f8932015df9954 ;;
    release-2000-disk.vhd.gz) echo 8dd7a9b353339f7be482a0090449e019170c827f330ac44fd889f9e7285caa6f ;;
    release-2000-disk.vhd) echo 77178ef2c2cda8f00354e81809d87fc4b644c3c342dca9c9f7bed12833dade8e ;;
    release-2000-promh.mcr) echo 1fcb62bc6d8cf8e1170a422e1401a2261043d1d1c1f9fb5f3f9450fdc0a54058 ;;
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

# The unsigned big-endian number of $3 bytes at offset $2 of file $1.
be() {
    n=0
    for b in $(od -An -v -tu1 -j "$2" -N "$3" "$1"); do
        n=$((n * 256 + b))
    done
    echo "$n"
}

# The $3 bytes at offset $2 of file $1, as text.
bytes_at() {
    dd if="$1" bs=1 skip="$2" count="$3" 2>/dev/null
}

# Whether file $1 is a dynamic VHD whose disk holds a GPT, saying why not
# on stderr. Microsoft's VHD specification (October 2006), every field
# big-endian: the footer is the file's last 512 bytes, cookie `conectix` at
# 0, the dynamic header's offset at 16, the disk type at 60 (3, dynamic);
# the dynamic header has cookie `cxsparse` at 0, the block table's offset at
# 16 and the block size at 32; a table entry is the sector where a block's
# sector bitmap begins, the block's data after the bitmap, or FFFFFFFF for a
# block not allocated, which reads as zeros. The GPT's header is the disk's
# sector 1 (UEFI 2.10, 5.3.1), in block 0.
vhd_holds_gpt() {
    name=${1#"$muir"/}
    size=$(wc -c < "$1")
    if [ "$size" -lt 1536 ]; then
        echo "$name is $size bytes, too short for a VHD" >&2
        return 1
    fi
    foot=$((size - 512))
    if [ "$(bytes_at "$1" "$foot" 8)" != conectix ]; then
        echo "$name has no VHD footer (no conectix in its last 512 bytes): not a VHD" >&2
        return 1
    fi
    if [ "$(be "$1" $((foot + 60)) 4)" != 3 ]; then
        echo "$name is a VHD of type $(be "$1" $((foot + 60)) 4), not a dynamic one (3)" >&2
        return 1
    fi
    header=$(be "$1" $((foot + 16)) 8)
    if [ "$(bytes_at "$1" "$header" 8)" != cxsparse ]; then
        echo "$name has no dynamic header (cxsparse) at $header" >&2
        return 1
    fi
    table=$(be "$1" $((header + 16)) 8)
    block=$(be "$1" $((header + 32)) 4)
    first=$(be "$1" "$table" 4)
    if [ "$first" = 4294967295 ]; then
        echo "$name: its disk's first block is not allocated, so sector 1 is zero: no GPT" >&2
        return 1
    fi
    # The sector bitmap: a bit per sector, rounded up to whole sectors.
    bitmap=$(((block / 512 + 4095) / 4096 * 512))
    at=$((first * 512 + bitmap + 512))
    if [ "$(bytes_at "$1" "$at" 8)" != "EFI PART" ]; then
        echo "$name is a VHD whose sector 1 is not a GPT header (no EFI PART): no GPT" >&2
        return 1
    fi
}

mkdir -p "$rel" "$run"

for f in $assets; do
    if [ -f "$rel/$f" ]; then
        echo "have $vrel/$f"
    else
        curl -fL -o "$rel/$f.part" "$base/$f"
        mv "$rel/$f.part" "$rel/$f"
    fi
    if [ "$(sha256 "$rel/$f")" != "$(sum_of "$f")" ]; then
        echo "$vrel/$f is not the file the release published:" >&2
        echo "its SHA-256 is not $(sum_of "$f"); move it away and run this again" >&2
        exit 1
    fi
    echo "     $vrel/$f checks"
done

refused=

# The number: every member of the sources under one `release-2NNN/`.
top=$(tar -tzf "$rel/$sources" | sed 's|/.*||' | sort -u)
case $top in
release-2[0-9][0-9][0-9]) echo "     $vrel/$sources unpacks to $top/, a QUUX system's" ;;
*)
    echo "$vrel/$sources unpacks to $(echo "$top" | tr '\n' ' ')" >&2
    echo "and not to one release-2NNN/: it is not a system for QUUX" >&2
    refused=1
    ;;
esac

# The disk is checked as it comes out and not afterwards: a machine writes
# to the disk it runs on, so the file in place is only the release's until
# the first run.
if [ -f "$run/$disk" ]; then
    echo "have vendor/run/$disk"
else
    gunzip -c "$rel/$disk.gz" > "$run/$disk.part"
    if [ "$(sha256 "$run/$disk.part")" != "$(sum_of "$disk")" ]; then
        rm -f "$run/$disk.part"
        echo "$vrel/$disk.gz does not decompress to the disk the release published" >&2
        exit 1
    fi
    echo "     vendor/run/$disk checks"
    # The format: QUUX's disk is a dynamic VHD holding a GPT.
    if vhd_holds_gpt "$run/$disk.part"; then
        echo "     vendor/run/$disk is a dynamic VHD holding a GPT, QUUX's disk"
    else
        echo "vendor/run/$disk is not QUUX's disk" >&2
        refused=1
    fi
    if [ -n "$refused" ]; then
        rm -f "$run/$disk.part"
    else
        mv "$run/$disk.part" "$run/$disk"
    fi
fi

if [ -n "$refused" ]; then
    echo "refused: nothing unpacked" >&2
    exit 1
fi

# The tarball's one top directory is left out, so the sources sit beside
# the tarball as System 100's do: vendor/system-2NNN/sys.
if [ -d "$rel/sys" ]; then
    echo "have $vrel/sys"
else
    tar xzf "$rel/$sources" -C "$rel" --strip-components=1
    echo "     $vrel/ holds the sources"
fi

# The file root, made once from copies, in a directory of its own that is
# renamed into place only when whole.
if [ -e "$root" ]; then
    echo "have vendor/run/$tag-root"
else
    rm -rf "$root.part"
    mkdir -p "$root.part/home/lispm"
    cp -Rp "$rel/sys" "$rel/site" "$root.part/"
    mv "$root.part" "$root"
    echo "made vendor/run/$tag-root: copies of sys/ and site/, and an empty home/lispm/"
fi

echo "done: $tag; run it with"
echo "  quux --disk-pack vendor/run/$disk --file-root vendor/run/$tag-root"
