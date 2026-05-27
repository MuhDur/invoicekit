// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! Thin shim for the `migrate-archive` subcommand so callers that wire
//! the binary directly (rather than `invoicekit migrate-archive`) hit
//! the exact same runner. Both paths funnel through
//! [`invoicekit_cli::commands::migrate_archive::run`].

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    invoicekit_cli::commands::migrate_archive::run(&argv)
}
