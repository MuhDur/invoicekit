// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-wasm` — WebAssembly artifact with feature-flagged
//! country and format bundles.
//!
//! The wasm-bindgen exports are conditionally compiled for the
//! `wasm32-*` targets so the crate continues to build and test on
//! native targets as part of `cargo test --workspace`. Browser /
//! Cloudflare Workers / Deno / Bun consumers reach the engine via
//! [`process_engine_abi_json`] (callable both natively and from JS).
//!
//! ## Feature flags
//!
//! Country bundles (`country-de`, `country-fr`, `country-it`,
//! `country-pl`, `country-sa`, etc.) and format bundles (`format-ubl`,
//! `format-cii`, `format-peppol`, `format-factur-x`, `format-xrechnung`,
//! `format-fatturapa`) let a downstream customer compile only what
//! they ship. `default` is intentionally empty so the leanest
//! consumer (e.g. a serverless function that only needs the engine
//! ABI scaffold) gets the smallest artifact.
//!
//! The `full` meta-flag toggles every country and format; the CI
//! workflow uses it to assert the "full bundle" still fits the
//! < 5 MB acceptance gate.
//!
//! Build for a target country set:
//!
//! ```bash
//! cargo build \
//!   --release \
//!   --no-default-features \
//!   --features "country-de,country-fr,country-it,format-peppol" \
//!   --target wasm32-unknown-unknown
//! ```
//!
//! `wasm-pack` is the recommended publisher; the workflow at
//! [`.github/workflows/wasm-artifact.yml`](../../../.github/workflows/wasm-artifact.yml)
//! drives it and uploads the bundle as a release artifact.

#![allow(clippy::doc_markdown, clippy::too_long_first_doc_paragraph)]

/// Bead identifier carried on emitted log records.
pub const WASM_ARTIFACT_BEAD_ID: &str = "invoices-t-025-wasm-artifact-nso";

/// Process an Engine ABI JSON request through the WebAssembly
/// delivery wrapper. Same byte contract as
/// `invoicekit_engine::process_abi_json`; the wasm-bindgen export
/// below is a thin wrapper that converts to/from `js_sys::Uint8Array`.
///
/// # Examples
///
/// ```
/// let response = invoicekit_wasm::process_engine_abi_json(
///     br#"{"abi_version":1,"operation":"unknown","payload":{}}"#,
/// );
/// assert!(std::str::from_utf8(&response).unwrap().contains(r#""status":"error""#));
/// ```
#[must_use]
pub fn process_engine_abi_json(request_bytes: &[u8]) -> Vec<u8> {
    invoicekit_engine::process_abi_json(request_bytes)
}

/// List the feature-gated country bundles compiled into this
/// artifact. Useful for a runtime `invoicekit_capabilities()` JS
/// surface to advertise what's actually shipped — the customer's
/// build flags determine the answer.
#[must_use]
pub fn compiled_country_bundles() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    if cfg!(feature = "country-be") {
        out.push("BE");
    }
    if cfg!(feature = "country-br") {
        out.push("BR");
    }
    if cfg!(feature = "country-de") {
        out.push("DE");
    }
    if cfg!(feature = "country-es") {
        out.push("ES");
    }
    if cfg!(feature = "country-fr") {
        out.push("FR");
    }
    if cfg!(feature = "country-gr") {
        out.push("GR");
    }
    if cfg!(feature = "country-hu") {
        out.push("HU");
    }
    if cfg!(feature = "country-in") {
        out.push("IN");
    }
    if cfg!(feature = "country-it") {
        out.push("IT");
    }
    if cfg!(feature = "country-mx") {
        out.push("MX");
    }
    if cfg!(feature = "country-pl") {
        out.push("PL");
    }
    if cfg!(feature = "country-ro") {
        out.push("RO");
    }
    if cfg!(feature = "country-sa") {
        out.push("SA");
    }
    if cfg!(feature = "country-tr") {
        out.push("TR");
    }
    out
}

/// List the feature-gated format bundles compiled into this artifact.
#[must_use]
pub fn compiled_format_bundles() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    if cfg!(feature = "format-cii") {
        out.push("CII");
    }
    if cfg!(feature = "format-factur-x") {
        out.push("Factur-X");
    }
    if cfg!(feature = "format-fatturapa") {
        out.push("FatturaPA");
    }
    if cfg!(feature = "format-peppol") {
        out.push("Peppol");
    }
    if cfg!(feature = "format-ubl") {
        out.push("UBL");
    }
    if cfg!(feature = "format-xrechnung") {
        out.push("XRechnung");
    }
    out
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_wasm::crate_name(), "invoicekit-wasm");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-wasm"
}

// ─────────────────────── wasm-bindgen surface ───────────────────────
//
// The JS-callable API lives behind `cfg(target_arch = "wasm32")` so
// native `cargo test --workspace` doesn't try to link wasm-bindgen's
// glue code.

#[cfg(target_arch = "wasm32")]
mod browser {
    use wasm_bindgen::prelude::*;

    /// Process an Engine ABI request from JavaScript.
    #[wasm_bindgen(js_name = processEngineAbiJson)]
    #[must_use]
    pub fn process_engine_abi_json(request_bytes: &[u8]) -> Box<[u8]> {
        super::process_engine_abi_json(request_bytes).into_boxed_slice()
    }

    /// Return the compiled-in country bundles as a JSON array.
    #[wasm_bindgen(js_name = compiledCountryBundles)]
    #[must_use]
    pub fn compiled_country_bundles() -> String {
        let bundles: Vec<String> = super::compiled_country_bundles()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut out = String::from("[");
        for (i, b) in bundles.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('"');
            out.push_str(b);
            out.push('"');
        }
        out.push(']');
        out
    }

    /// Return the compiled-in format bundles as a JSON array.
    #[wasm_bindgen(js_name = compiledFormatBundles)]
    #[must_use]
    pub fn compiled_format_bundles() -> String {
        let bundles: Vec<String> = super::compiled_format_bundles()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut out = String::from("[");
        for (i, b) in bundles.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('"');
            out.push_str(b);
            out.push('"');
        }
        out.push(']');
        out
    }

    /// Bead identifier; reachable from JS for diagnostic correlation.
    #[wasm_bindgen(js_name = beadId)]
    #[must_use]
    pub fn bead_id() -> String {
        super::WASM_ARTIFACT_BEAD_ID.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        compiled_country_bundles, compiled_format_bundles, crate_name, process_engine_abi_json,
    };
    use serde::Deserialize;

    const GOLDEN_FIXTURE: &str =
        include_str!("../../../conformance-corpus/golden/engine-abi-v1-commercial-document.json");

    #[derive(Debug, Deserialize)]
    struct GoldenFixture {
        request_bytes: String,
        expected_response_bytes: String,
    }

    fn golden_fixture() -> GoldenFixture {
        serde_json::from_str(GOLDEN_FIXTURE).expect("golden fixture is valid JSON")
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-wasm");
    }

    #[test]
    fn crate_name_is_non_empty() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn crate_name_is_lowercase_kebab() {
        let n = crate_name();
        for c in n.chars() {
            assert!(
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
                "non-kebab char in {n}: {c:?}"
            );
        }
    }

    #[test]
    fn crate_name_carries_invoicekit_prefix() {
        let n = crate_name();
        assert!(
            n == "invoicekit" || n.starts_with("invoicekit-") || n.starts_with("invoicekit_"),
            "crate name does not advertise InvoiceKit family: {n}"
        );
    }

    #[test]
    fn wasm_wrapper_matches_engine_abi_golden_fixture() {
        let fixture = golden_fixture();
        assert_eq!(
            process_engine_abi_json(fixture.request_bytes.as_bytes()),
            fixture.expected_response_bytes.as_bytes()
        );
    }

    #[test]
    fn bundle_lists_are_sorted_and_unique() {
        // Holds regardless of which features are enabled: every
        // bundle entry must be unique (no double-listing under
        // overlapping feature combinations) and the list must be
        // ASCII-sorted so downstream JS code can binary-search it
        // without resorting.
        for list in [compiled_country_bundles(), compiled_format_bundles()] {
            let mut sorted = list.clone();
            sorted.sort_unstable();
            assert_eq!(list, sorted, "bundle list must be ASCII-sorted");
            sorted.dedup();
            assert_eq!(
                sorted.len(),
                list.len(),
                "bundle list must contain no duplicates",
            );
        }
    }

    #[cfg(not(any(
        feature = "country-be",
        feature = "country-br",
        feature = "country-de",
        feature = "country-es",
        feature = "country-fr",
        feature = "country-gr",
        feature = "country-hu",
        feature = "country-in",
        feature = "country-it",
        feature = "country-mx",
        feature = "country-pl",
        feature = "country-ro",
        feature = "country-sa",
        feature = "country-tr",
    )))]
    #[test]
    fn default_features_report_empty_country_bundles() {
        // With no country-* features enabled, the bundle list must
        // be empty — the artifact ships just the engine ABI surface.
        assert_eq!(compiled_country_bundles(), Vec::<&str>::new());
    }

    #[cfg(not(any(
        feature = "format-cii",
        feature = "format-factur-x",
        feature = "format-fatturapa",
        feature = "format-peppol",
        feature = "format-ubl",
        feature = "format-xrechnung",
    )))]
    #[test]
    fn default_features_report_empty_format_bundles() {
        assert_eq!(compiled_format_bundles(), Vec::<&str>::new());
    }

    #[cfg(feature = "country-de")]
    #[test]
    fn enabling_country_de_advertises_de_in_compiled_bundles() {
        assert!(compiled_country_bundles().contains(&"DE"));
    }

    #[cfg(feature = "format-peppol")]
    #[test]
    fn enabling_format_peppol_advertises_peppol_in_compiled_bundles() {
        assert!(compiled_format_bundles().contains(&"Peppol"));
    }

    #[cfg(feature = "full")]
    #[test]
    fn full_meta_feature_enables_every_known_bundle() {
        let countries = compiled_country_bundles();
        for expected in [
            "BE", "BR", "DE", "ES", "FR", "GR", "HU", "IN", "IT", "MX", "PL", "RO", "SA", "TR",
        ] {
            assert!(
                countries.contains(&expected),
                "country {expected} missing under full"
            );
        }
        let formats = compiled_format_bundles();
        for expected in ["CII", "Factur-X", "FatturaPA", "Peppol", "UBL", "XRechnung"] {
            assert!(
                formats.contains(&expected),
                "format {expected} missing under full"
            );
        }
    }
}
