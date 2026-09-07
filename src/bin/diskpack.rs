// SPDX-FileCopyrightText: 2026 Mete Balci
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `diskpack <image> [<command>]`: making a disk pack and editing its label.
//!
//! The pack named is opened if it is there and started if it is not.  With a
//! command after it that command is run and the tool is done; without one,
//! commands are read a line at a time.  [`muir::diskpack`] is the language and
//! what each command does; this is the loop around it.
//!
//! Every command takes effect when it runs, so there is nothing to lose by
//! leaving and nothing to remember to write.
//!
//! A line typed at a terminal that is no command says so and the next line is
//! read.  A line piped in that is no command stops the run with status 1: a
//! script that builds a pack must not go on after a step of it failed.

use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

use muir::diskpack::{Command, HELP, PROMPT, Pack, parse};

const USAGE: &str = "usage: diskpack <image> [<command> ...]

  <image>    the pack to edit, made if it is not there yet
  <command>  one command, run instead of the prompt

A pack is a file of blocks, a Trident's worth: 257 MiB for a T-300, which is
sparse until something is written to it.  Commands are read from the terminal
or piped in, and one can also go on the command line, where the shell
completes a file name for you; it does the same thing there as at the prompt.
";

fn usage(msg: &str) -> ! {
    eprintln!("diskpack: {msg}");
    eprint!("{USAGE}");
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (path, command) = match args.as_slice() {
        // The commands are the useful half of the help, and asking for them
        // should not mean naming a pack first.
        [one] if one == "-h" || one == "--help" => {
            print!("{USAGE}\n{HELP}");
            std::process::exit(0);
        }
        [one, ..] if one.starts_with('-') => usage(&format!("{one} is no flag")),
        [] => usage("no pack given"),
        // The rest of the command line is the command, joined with spaces as
        // a typed line would be.  Every command that takes a path takes the
        // whole of what is left as it, so a name with a space in it comes
        // through the shell's quoting intact.
        [one, rest @ ..] => (PathBuf::from(one), rest.join(" ")),
    };

    let (mut pack, said) = Pack::open(&path);
    if command.is_empty() || !pack.has_label() {
        println!("{said}");
    }
    if !command.is_empty() {
        once(&mut pack, &command);
    }

    let terminal = std::io::stdin().is_terminal();
    let mut lines = std::io::stdin().lock().lines();
    loop {
        if terminal {
            print!("{PROMPT}");
            let _ = std::io::stdout().flush();
        }
        let Some(line) = lines.next() else { return };
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                eprintln!("diskpack: {e}");
                std::process::exit(1);
            }
        };
        let command = match parse(&line) {
            Ok(Some(c)) => c,
            Ok(None) => continue,
            Err(e) => {
                eprintln!("{e}");
                if terminal {
                    continue;
                }
                std::process::exit(1);
            }
        };
        if command == Command::Quit {
            return;
        }
        match pack.run(command) {
            Ok(said) => print!("{said}"),
            Err(e) => {
                eprintln!("{e}");
                if !terminal {
                    std::process::exit(1);
                }
            }
        }
    }
}

/// One command off the command line, and then out.
///
/// A word that is no command is a usage error, as a flag muir does not have
/// is; a command that ran and failed is not.
fn once(pack: &mut Pack, line: &str) -> ! {
    let command = match parse(line) {
        Ok(Some(c)) => c,
        Ok(None) => std::process::exit(0),
        Err(e) => usage(&e),
    };
    if command != Command::Quit
        && let Err(e) = pack.run(command).map(|said| print!("{said}"))
    {
        eprintln!("{e}");
        std::process::exit(1);
    }
    std::process::exit(0);
}
