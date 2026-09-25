#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mete Balci
# SPDX-License-Identifier: AGPL-3.0-or-later
# Make QUUX's disk fixtures, data/quux-disk-*: one small GPT disk, raw and
# as a fixed and a dynamic VHD, and the dynamic VHD again after writes that
# made it grow, with its raw twin.
#
# **Made by standard tools only** --- sgdisk, dd, qemu-img and qemu-io, the
# recipe of docs/manual.md --- and never by muir, because they are what
# muir's own reader and any other implementation's are checked against.
#
# The disk is 8 MiB, 16,384 sectors: four of a dynamic VHD's 2 MiB blocks.
# Sectors are 512 bytes and QUUX's blocks 1,024, so every partition starts
# on an even sector and ends on an odd one (`-a 2`). Every GUID is given,
# so that sgdisk's part of this makes the same bytes every time; the VHD
# footers carry qemu's timestamp and a random UUID, which differ.
#
# What goes where, in sectors:
#
#   1 TEMP  2048-2175   empty
#   2 MCR1  2176-2687   blocks 0-19 "MCR1 block 0000 " ...; current (bit 48)
#   3 MCR2  2688-3199   blocks 0-3  "MCR2 block 0000 " ...
#   4 LOD1  3200-5247   blocks 0-63 "LOD1 block 0000 " ...; current (bit 48)
#   5 LOD2  5248-7295   empty; a comment of the full 31 characters
#   6 PAGE  7296-9343   empty
#   7 FILE  9344-16349  empty
#
# so everything written is in the VHD's first 2 MiB block, the backup GPT
# is in its last, and the two between are left unallocated in the dynamic
# VHD. The grown pair then has three writes from qemu-io into those two
# blocks: 1,024 bytes of 0x4c at LOD2's first sector, 4,096 of 0x50 at
# PAGE's, and 8,192 of 0x46 at FILE's.
#
# usage: tools/quux-disk-fixtures.sh
#   QEMU_IMG and QEMU_IO name the two qemu tools if they are not on PATH.
set -eu
muir=$(cd "$(dirname "$0")/.." && pwd)
data=$muir/data
qemu_img=${QEMU_IMG:-qemu-img}
qemu_io=${QEMU_IO:-qemu-io}
for t in sgdisk "$qemu_img" "$qemu_io"; do
    command -v "$t" >/dev/null || { echo "no $t" >&2; exit 1; }
done
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

raw=$work/quux-disk.img
microcode=9e318cf5-a95b-4b3b-b2ad-9ae306b0e2da
band=a3b30470-c5d4-41c1-87a8-d26590424cb8
page=4652bea5-06af-4bd9-b2bb-3541370151c8
file=7afa9532-75de-409f-8dc8-fef9763511d5
temp=445976f2-34e4-4583-b750-75d28a080cba

"$qemu_img" create -q -f raw "$raw" 8M
sgdisk -a 2 -U 00000000-0000-4000-8000-000000000000 \
    -n 1:2048:2175 -t 1:$temp -c 1:"TEMP" -u 1:00000000-0000-4000-8000-000000000001 \
    -n 2:2176:2687 -t 2:$microcode -c 2:"MCR1 UCADR 1000" -A 2:set:48 \
    -u 2:00000000-0000-4000-8000-000000000002 \
    -n 3:2688:3199 -t 3:$microcode -c 3:"MCR2 UCADR 999" \
    -u 3:00000000-0000-4000-8000-000000000003 \
    -n 4:3200:5247 -t 4:$band -c 4:"LOD1 System 1002.1" -A 4:set:48 \
    -u 4:00000000-0000-4000-8000-000000000004 \
    -n 5:5248:7295 -t 5:$band -c 5:"LOD2 A comment that is 31 characters" \
    -u 5:00000000-0000-4000-8000-000000000005 \
    -n 6:7296:9343 -t 6:$page -c 6:"PAGE" -u 6:00000000-0000-4000-8000-000000000006 \
    -n 7:9344:16349 -t 7:$file -c 7:"FILE" -u 7:00000000-0000-4000-8000-000000000007 \
    "$raw" >/dev/null

# Each block of a filled partition says what it is, sixteen bytes
# `<name> block <nnnn> ` sixty-four times, so a block read from the wrong
# place is told apart from the right one.
fill() { # name blocks first-sector
    i=0
    while [ "$i" -lt "$2" ]; do
        line=$(printf '%s block %04d ' "$1" "$i")
        j=0
        while [ "$j" -lt 64 ]; do printf '%s' "$line"; j=$((j + 1)); done
        i=$((i + 1))
    done > "$work/$1.part"
    dd if="$work/$1.part" of="$raw" bs=512 seek="$3" conv=notrunc status=none
}
fill MCR1 20 2176
fill MCR2 4 2688
fill LOD1 64 3200
sgdisk -v "$raw" | grep -q 'No problems found' || { sgdisk -v "$raw" >&2; exit 1; }

cp "$raw" "$data/quux-disk.img"
"$qemu_img" convert -f raw -O vpc -o subformat=fixed,force_size=on \
    "$raw" "$data/quux-disk-fixed.vhd"
"$qemu_img" convert -f raw -O vpc -o subformat=dynamic,force_size=on \
    "$raw" "$data/quux-disk-dynamic.vhd"

cp "$data/quux-disk-dynamic.vhd" "$work/grown.vhd"
cp "$raw" "$work/grown.img"
for f in "vpc $work/grown.vhd" "raw $work/grown.img"; do
    set -- $f
    "$qemu_io" -f "$1" \
        -c "write -P 0x4c $((5248 * 512)) 1024" \
        -c "write -P 0x50 $((7296 * 512)) 4096" \
        -c "write -P 0x46 $((9344 * 512)) 8192" \
        "$2" >/dev/null
done
cp "$work/grown.vhd" "$data/quux-disk-dynamic-grown.vhd"
cp "$work/grown.img" "$data/quux-disk-grown.img"

# Each VHD holds its raw twin, as qemu reads it.
"$qemu_img" compare -q -f vpc -F raw "$data/quux-disk-fixed.vhd" "$data/quux-disk.img"
"$qemu_img" compare -q -f vpc -F raw "$data/quux-disk-dynamic.vhd" "$data/quux-disk.img"
"$qemu_img" compare -q -f vpc -F raw \
    "$data/quux-disk-dynamic-grown.vhd" "$data/quux-disk-grown.img"
ls -l "$data"/quux-disk*
