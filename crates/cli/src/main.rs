// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit` CLI entry point. Dispatches `argv[1]` to the matching
//! `invoicekit_cli::commands::*::run` function so every subcommand
//! shares one library code path and the standalone shim binaries
//! (`migrate-archive`, `gen-schema`, ...) stay in lockstep.

use std::env;
use std::process::ExitCode;

const USAGE: &str = "usage: invoicekit <command> [<args>...]\n\nCommands:\n  capabilities     resolve accepted e-invoice profiles for a route/scenario/date\n  migrate-archive  migrate a directory of invoice JSON archives between schema versions\n\nRun `invoicekit <command> --help` for command-specific flags.\n";

fn main() -> ExitCode {
    let _ = invoicekit_cli::crate_name();
    let mut argv = env::args().skip(1);
    let Some(sub) = argv.next() else {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    };
    let rest: Vec<String> = argv.collect();
    match sub.as_str() {
        "--help" | "-h" => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        "capabilities" => invoicekit_cli::commands::capabilities::run(&rest),
        "migrate-archive" => invoicekit_cli::commands::migrate_archive::run(&rest),
        unknown => {
            eprintln!("invoicekit: unknown subcommand {unknown:?}");
            eprintln!();
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn binary_links() {
        let _ = invoicekit_cli::crate_name();
    }
}
