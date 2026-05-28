// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-095 Peppol Test Bed conformance test (receiver side).
//!
//! Gated behind the `peppol-test-bed` cargo feature. Only runs
//! when the operator has set `PEPPOL_TEST_BED_CREDENTIALS`.

#![cfg(feature = "peppol-test-bed")]

use std::path::PathBuf;

use invoicekit_transmit_peppol_byok::PeppolCredentials;
use invoicekit_transmit_peppol_native_as4_receive::byok::receiver_config_from_byok;

fn credentials_path() -> Option<PathBuf> {
    std::env::var("PEPPOL_TEST_BED_CREDENTIALS")
        .ok()
        .map(PathBuf::from)
}

#[test]
fn loads_test_bed_credentials_and_builds_receiver_config() {
    let Some(path) = credentials_path() else {
        eprintln!("PEPPOL_TEST_BED_CREDENTIALS not set; skipping");
        return;
    };
    let creds = PeppolCredentials::from_json_file(&path).expect("load Test Bed credentials");
    let cfg = receiver_config_from_byok(&creds).expect("byok -> receiver config");
    assert!(cfg.bind_url.starts_with("https://"));
    assert!(!cfg.participant_id_wire.is_empty());
}
