#!/bin/sh
# SPDX-FileCopyrightText: 2026 Mete Balci
# SPDX-License-Identifier: AGPL-3.0-or-later
# Check that every netlist in `data/` is what its script in `tools/` makes
# from MIT's drawings today.
#
# `CLAUDE.md` says `data/` holds fixtures made *from* `mit/` by a script
# here, and nothing used to check it: **a fixture whose script no longer
# reproduces it looks exactly like one that does.** It had already happened
# once. `data/CADR.netlist` was a fossil of a `soap4` that could not read
# `cpins.drw` --- the scripts fall back to a bare `page NAME` when a drawing
# will not read, and the committed file had `page CPINS` bare where today's
# reader emits the drawing's banner. Nine comment lines, no non-comment line
# changed, 97 pages and 1243 parts either way, and it was found only because
# a documentation change happened to want a regeneration.
#
# **What is checked is the file and not a summary of it.** A hash would say
# that a netlist had moved; the whole of that finding was in *what* moved,
# and a reader given a mismatched hash has to regenerate anyway to learn
# whether the committed file or the tool is the one that changed. So the
# regenerated file is diffed and the diff is printed.
#
# **The committed files are deleted before the scripts run**, so that a
# script which exits without writing is a missing file rather than a file
# that agrees: "no diff" and "did not write" are the same diff otherwise.
# Whatever happens, the committed files are put back on the way out ---
# `--write` keeps the regenerated ones instead, which is what to use when
# the difference is the intended one.
#
# It wants a C compiler, as the netlist scripts do. What it costs is
# whether `examples/reconcile.rs` is built: seconds when it is, since each
# board is a `soap4` build and a pass over its drawings, and a release
# build of the library first when it is not.
#
# usage: tools/check-netlists.sh [--write]
set -eu
muir=$(cd "$(dirname "$0")/.." && pwd)

write=no
if [ "${1-}" = --write ]; then
    write=yes
elif [ $# -gt 0 ]; then
    echo "usage: tools/check-netlists.sh [--write]" >&2
    exit 2
fi

# **The boards are whatever `data/` holds**, and not a list here, so that a
# ninth netlist is checked the day it is committed rather than the day
# somebody remembers this file. A script's name is the netlist's in lower
# case, and a netlist with no script is what `CLAUDE.md` says cannot exist.
boards=""
for f in "$muir"/data/*.netlist; do
    b=$(basename "$f" .netlist)
    if [ ! -x "$muir/tools/$(echo "$b" | tr 'A-Z' 'a-z')-netlist.sh" ]; then
        echo "data/$b.netlist: no tools/ script makes it" >&2
        exit 1
    fi
    boards="$boards $b"
done
if [ -z "$boards" ]; then
    echo "no netlists in data/" >&2
    exit 1
fi

saved=$(mktemp -d)
restore() {
    if [ "$write" = no ]; then
        for b in $boards; do
            if [ -f "$saved/$b.netlist" ]; then
                cp "$saved/$b.netlist" "$muir/data/$b.netlist"
            fi
        done
    fi
    rm -rf "$saved"
}
trap restore EXIT INT TERM

for b in $boards; do
    cp "$muir/data/$b.netlist" "$saved/$b.netlist"
    rm "$muir/data/$b.netlist"
done

# The scripts report every net the drawings label twice, which is hundreds
# of lines a board and is the reconciliation working rather than anything
# wrong. It is kept and shown only for a script that fails.
failed=""
for b in $boards; do
    s=$(echo "$b" | tr 'A-Z' 'a-z')
    if ! "$muir/tools/$s-netlist.sh" > "$saved/$b.log" 2>&1; then
        echo "tools/$s-netlist.sh failed:" >&2
        sed -n '1,40p' "$saved/$b.log" >&2
        failed="$failed $b"
    fi
done

moved=""
for b in $boards; do
    s=$(echo "$b" | tr 'A-Z' 'a-z')
    if [ ! -f "$muir/data/$b.netlist" ]; then
        echo "$b.netlist: tools/$s-netlist.sh wrote nothing"
        moved="$moved $b"
    elif diff -u "$saved/$b.netlist" "$muir/data/$b.netlist" > "$saved/$b.diff"; then
        echo "$b.netlist: as tools/$s-netlist.sh makes it"
    else
        echo "$b.netlist: NOT what tools/$s-netlist.sh makes"
        sed -n '1,200p' "$saved/$b.diff"
        lines=$(wc -l < "$saved/$b.diff")
        if [ "$lines" -gt 200 ]; then
            echo "... and $((lines - 200)) more lines of diff"
        fi
        moved="$moved $b"
    fi
done

if [ -n "$failed$moved" ]; then
    echo >&2
    if [ -n "$failed" ]; then
        echo "scripts that failed:$failed" >&2
    fi
    if [ -n "$moved" ]; then
        echo "netlists that are not what their script makes:$moved" >&2
    fi
    if [ "$write" = no ]; then
        echo "the committed files are unchanged; --write keeps the regenerated ones" >&2
    fi
    exit 1
fi

# The arguments were read long ago, so the positional parameters are free
# to count with.
set -- $boards
echo "all $# netlists are what tools/ makes from mit/ today"
