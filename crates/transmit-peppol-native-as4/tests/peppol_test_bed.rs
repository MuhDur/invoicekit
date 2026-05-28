// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-094 Peppol Test Bed conformance test (sender side).
//!
//! Gated behind the `peppol-test-bed` cargo feature. Only runs
//! when the operator has set `PEPPOL_TEST_BED_CREDENTIALS` to a
//! credentials JSON file (Test Bed certificate + key + endpoint).
//! The Peppol Test Bed is free and requires no partner contract;
//! see `docs/operators/PEPPOL-BYOK.md` for how to apply.

#![cfg(feature = "peppol-test-bed")]

use std::path::PathBuf;

use invoicekit_transmit_peppol_byok::PeppolCredentials;
use invoicekit_transmit_peppol_native_as4::byok::native_as4_config_from_byok;

fn credentials_path() -> Option<PathBuf> {
    std::env::var("PEPPOL_TEST_BED_CREDENTIALS")
        .ok()
        .map(PathBuf::from)
}

#[test]
fn loads_test_bed_credentials_and_builds_native_as4_config() {
    let Some(path) = credentials_path() else {
        eprintln!("PEPPOL_TEST_BED_CREDENTIALS not set; skipping");
        return;
    };
    let creds = PeppolCredentials::from_json_file(&path).expect("load Test Bed credentials");
    let cfg = native_as4_config_from_byok(&creds).expect("byok -> native-as4 config");
    assert!(cfg.ap_cert_pem.contains("BEGIN CERTIFICATE"));
}
