// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-055: dump the byte-stable hello-world PDF to stdout. The
//! cross-platform CI job calls this binary on Linux + macOS and
//! compares the sha256 digests; matching digests prove the
//! renderer is byte-stable across operating systems.

use std::io::Write as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    match invoicekit_render_pdf::render_hello_world_invoice() {
        Ok(pdf) => {
            if let Err(err) = std::io::stdout().write_all(&pdf) {
                eprintln!("render_hello_world: failed to write stdout: {err}");
                return ExitCode::from(2);
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("render_hello_world: render failed: {err}");
            ExitCode::FAILURE
        }
    }
}
