// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-binding-rest-shim` binary.

const DEFAULT_BIND: &str = "127.0.0.1:8081";

#[tokio::main]
async fn main() -> Result<(), invoicekit_binding_rest_shim::ServeError> {
    let bind = std::env::var("INVOICEKIT_REST_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_owned());
    invoicekit_binding_rest_shim::serve(&bind).await
}
