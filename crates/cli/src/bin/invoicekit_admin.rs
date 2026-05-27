// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-137 binary entry point for `invoicekit-admin`.

use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    invoicekit_cli::admin::run_dispatch(&argv)
}
