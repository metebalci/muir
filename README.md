# muir

A simulator of the MIT CADR Lisp Machine, down to the chips.

The CADR is the second-generation MIT Lisp Machine, designed around 1978 by
Tom Knight, David Moon, Jack Holloway and Guy Steele. muir models it far
enough down that the software written *for the hardware* runs --- the MIT
diagnostics, the console program CC, and two machines lashed together with
one debugging the other. That lashup is the acceptance test, and it passes.
There are three engines: `micro`, the fastest, with no timing model; `rtl`,
on the machine's own clock, for ordinary use; and `chip`, which runs MIT's
own drawings part by part. Rust, no crate dependencies.

The front page is **[muir.metebalci.com](https://muir.metebalci.com)**, and
[the manual](docs/manual.md) is the rest.

## Quick start

Install `rustup`, then open a new shell so `cargo` is on the path:

    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

Build muir and fetch [System 100](https://tumbleweed.nu/system-100-0-release/),
the release it boots. The compiler version is pinned in `rust-toolchain.toml`,
and the release lands in `vendor/`, every file checked against its SHA-256
sum.

    git clone https://github.com/metebalci/muir
    cd muir
    cargo build --release
    tools/fetch-system-100.sh

A Lisp Machine took its files and the date from a host on the Chaosnet, and
a CADR had no such server in it, so muir has none either.
[ozd](https://github.com/metebalci/ozd) is that host. Build it beside muir
and start it in a shell of its own, serving System 100's sources:

    git clone https://github.com/metebalci/ozd ../ozd
    (cd ../ozd && cargo build --release)
    mkdir -p vendor/run/oz/lispm
    ../ozd/target/release/ozd --address 3060 --name MIT-OZ,OZ,system=UNIX \
        --root $PWD/vendor/run/oz --root tree=$PWD/vendor/system-100-0/sys,ro

Then start the machine. System 100's host table puts the machine at 3050 and
its host at 3060; ozd has UDP port 42042, so the machine takes 42043.

    target/release/muir --disk-pack vendor/run/disk-sys-100-0.img \
        --chaos-address 3050 --chaos-udp 42043 --chaos-udp-peer 3060@127.0.0.1:42042

It says where its terminal is and boots. Point any VNC viewer at
`vnc://127.0.0.1:5900`, with no password: the display, keyboard and mouse
are the machine's only way in or out. In under a minute it is at a Lisp
Listener with the date set. Type `(si:setup-cpt)` there for the mouse, which
[the terminal](docs/manual.md#the-terminal) explains, along with what to type
when a boot with no host stops to ask for the date.

## Documentation

| | |
|---|---|
| [The manual](docs/manual.md) | Running muir: what a run says, every flag, flags in a file, the prompt, the terminal, what a run can write, the Chaosnet and the two-machine lashup |
| [Making a pack](docs/diskpack.md) | `diskpack`, the second binary: a pack of one's own, its partitions, and bands loaded and dumped |
| [How the engines work](docs/engines.md) | `micro`, `rtl` and `chip`: what each computes, how fast each runs, and what each models board by board |
| [The machine it models](docs/machine.md) | The processor, the bus interface, the two buses and every board on them |
| [Where the netlists come from](docs/netlists.md) | MIT's drawings and wire lists, how a board becomes a netlist, and what each is checked against |
| [The Chaosnet board](docs/chaosnet.md) | The interface on the I/O board, its cable, and what muir has of it |
| [The TV board](docs/tv.md) | The SIMPLE TV, the LISPM TV and the color TV |
| [The keyboard boot sequence](docs/keyboard-boot.md) | The chord that boots the machine, from the keyboard's firmware to the processor |
| [Sources and attribution](docs/sources.md) | What this is built on, related projects, how it was written, and the license |

## Layout

    mit/       MIT's own files, unmodified: the drawings and wire lists of
               every board modeled here, and a snapshot of System 100's own
               sys tree, which the boot PROM comes from. mit/README.md
    data/      what is made from mit/ by a script in tools/, each
               cross-checked or labeled: the eight netlists, the disk
               controller's microcode, the cable tables. data/README.md
    src/       the simulator, the muir binary and diskpack
    tests/     the checks
    examples/  development tools; none is part of the simulator.
               examples/README.md
    tools/     fetching, and one script per board to re-extract a netlist
    pages/     the front page and the recording, published by
               .github/workflows/pages.yml
    docs/      the manual, and findings about the machine read from mit/,
               each claim cited to its file and held by a test where one
               can. docs/README.md
    vendor/    fetched material, never committed

## License

Copyright (C) 2026 Mete Balci. Free software under the **GNU Affero General
Public License**, version 3 or, at your option, any later version; see
`LICENSE`. `mit/` is MIT's work and `tools/soap4/` came from `ams/cadr4`:
[the license](docs/sources.md#license) says what is whose.

muir is implemented entirely by [Claude Code](https://claude.com/claude-code),
on Anthropic's Opus and Fable models.
