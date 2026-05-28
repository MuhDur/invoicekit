// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Export the REST shim OpenAPI document for release artifacts.

use std::env;
use std::fs;
use std::io::{self, Write as _};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = invoicekit_binding_rest_shim::openapi_document_bytes();
    let hash = invoicekit_binding_rest_shim::openapi_sha256_hex();
    if let Some(path) = env::args_os().nth(1).map(PathBuf::from) {
        fs::write(path, &bytes)?;
        println!("{hash}");
    } else {
        io::stdout().write_all(&bytes)?;
        eprintln!("{hash}");
    }
    Ok(())
}
