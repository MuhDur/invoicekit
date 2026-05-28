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
/// Bead identifier for the runtime capability matrix.
pub const CAPABILITY_MATRIX_BEAD_ID: &str = "invoices-t-033-browser-edge-capability-matrix-pet";
/// Engine ABI operation for local validation requests that cannot run in WASM.
pub const COMMERCIAL_DOCUMENT_LOCAL_VALIDATE_OPERATION: &str = "commercial_document.local_validate";
/// Engine ABI operation for reference validation requests that require a backend.
pub const COMMERCIAL_DOCUMENT_REFERENCE_VALIDATE_OPERATION: &str =
    "commercial_document.reference_validate";

/// Typed diagnostic used when WebAssembly callers ask for a validator path
/// that requires a JVM sidecar, CLI verifier, partner service, or other
/// non-WASM backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequiresExternalBackend {
    /// Stable machine-readable error code.
    pub code: &'static str,
    /// Profile that needs the backend.
    pub profile_id: String,
    /// Capability that cannot run in-process.
    pub capability: String,
    /// Required backend identifier, for example `jvm:kosit`.
    pub backend: String,
    /// User-facing remediation.
    pub remediation: String,
}

impl RequiresExternalBackend {
    /// Builds the canonical WASM external-backend diagnostic.
    ///
    /// # Examples
    ///
    /// ```
    /// let err = invoicekit_wasm::RequiresExternalBackend::new(
    ///     "xrechnung-3.0",
    ///     "reference_validate",
    ///     "jvm:kosit",
    /// );
    /// assert_eq!(err.code, "requires_external_backend");
    /// assert!(err.remediation.contains("server-assisted validator"));
    /// ```
    #[must_use]
    pub fn new(
        profile_id: impl Into<String>,
        capability: impl Into<String>,
        backend: impl Into<String>,
    ) -> Self {
        let backend = backend.into();
        Self {
            code: "requires_external_backend",
            profile_id: profile_id.into(),
            capability: capability.into(),
            remediation: format!(
                "route this operation to a server-assisted validator with `{backend}`; WebAssembly never silently downgrades to local validation"
            ),
            backend,
        }
    }

    /// Deterministic JSON representation for JavaScript callers.
    ///
    /// # Examples
    ///
    /// ```
    /// let json = invoicekit_wasm::RequiresExternalBackend::new(
    ///     "xrechnung-3.0",
    ///     "reference_validate",
    ///     "jvm:kosit",
    /// )
    /// .to_json();
    /// assert!(json.contains(r#""code":"requires_external_backend""#));
    /// ```
    #[must_use]
    pub fn to_json(&self) -> String {
        format!(
            "{{\"backend\":\"{}\",\"capability\":\"{}\",\"code\":\"{}\",\"profile_id\":\"{}\",\"remediation\":\"{}\"}}",
            escape_json(&self.backend),
            escape_json(&self.capability),
            self.code,
            escape_json(&self.profile_id),
            escape_json(&self.remediation)
        )
    }

    fn to_engine_abi_error_json(&self, operation: &str) -> String {
        format!(
            "{{\"abi_version\":1,\"error\":{{\"backend\":\"{}\",\"capability\":\"{}\",\"code\":\"{}\",\"message\":\"{}\",\"profile_id\":\"{}\",\"remediation\":\"{}\"}},\"operation\":\"{}\",\"status\":\"error\"}}",
            escape_json(&self.backend),
            escape_json(&self.capability),
            self.code,
            escape_json(&self.to_string()),
            escape_json(&self.profile_id),
            escape_json(&self.remediation),
            escape_json(operation)
        )
    }
}

impl std::fmt::Display for RequiresExternalBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {} requires external backend {} for {}",
            self.code, self.profile_id, self.backend, self.capability
        )
    }
}

impl std::error::Error for RequiresExternalBackend {}

/// Always returns a typed external-backend diagnostic for reference
/// validators that cannot run inside the WASM artifact.
///
/// # Errors
///
/// Returns [`RequiresExternalBackend`] with the backend and remediation
/// the browser/edge caller must surface.
pub fn require_external_backend(
    profile_id: impl Into<String>,
    capability: impl Into<String>,
    backend: impl Into<String>,
) -> Result<(), RequiresExternalBackend> {
    Err(RequiresExternalBackend::new(
        profile_id, capability, backend,
    ))
}

fn escape_json(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if c.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(escaped, "\\u{:04x}", c as u32);
            }
            c => escaped.push(c),
        }
    }
    escaped
}

/// Process an Engine ABI JSON request through the WebAssembly delivery
/// wrapper.
///
/// External validator operations that cannot run inside a browser/edge WASM
/// artifact return a typed `requires_external_backend` error before the request
/// reaches the native engine. All other operations use the same byte contract as
/// `invoicekit_engine::process_abi_json`; the wasm-bindgen export below is a
/// thin wrapper that converts to/from `js_sys::Uint8Array`.
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
    if let Some(response) = external_backend_abi_response(request_bytes) {
        return response;
    }
    invoicekit_engine::process_abi_json(request_bytes)
}

fn external_backend_abi_response(request_bytes: &[u8]) -> Option<Vec<u8>> {
    let request_text = std::str::from_utf8(request_bytes).ok()?;
    let request: serde_json::Value = serde_json::from_str(request_text).ok()?;
    if request
        .get("abi_version")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
    {
        return None;
    }
    let operation = request
        .get("operation")
        .and_then(serde_json::Value::as_str)?;
    let capability = match operation {
        COMMERCIAL_DOCUMENT_LOCAL_VALIDATE_OPERATION => "local_validate",
        COMMERCIAL_DOCUMENT_REFERENCE_VALIDATE_OPERATION => "reference_validate",
        _ => return None,
    };
    let payload = request
        .get("payload")
        .and_then(serde_json::Value::as_object)?;
    let profile_id = payload
        .get("profile_id")
        .and_then(serde_json::Value::as_str)?;
    let backend = payload
        .get("backend")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| default_external_backend(profile_id, capability));
    let error = RequiresExternalBackend::new(profile_id, capability, backend);
    Some(error.to_engine_abi_error_json(operation).into_bytes())
}

fn default_external_backend(profile_id: &str, capability: &str) -> &'static str {
    match (profile_id, capability) {
        (profile, "reference_validate") if profile.starts_with("xrechnung") => "jvm:kosit",
        (profile, "reference_validate") if profile.starts_with("peppol") => "jvm:phive",
        (profile, "reference_validate") if profile.starts_with("factur-x") => "verapdf",
        (profile, "reference_validate") if profile.starts_with("fatturapa") => "partner:sdi",
        _ => "external-validator",
    }
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
#[allow(unreachable_pub)]
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

    /// Return a typed external-backend diagnostic as stable JSON.
    #[wasm_bindgen(js_name = requiresExternalBackendJson)]
    #[must_use]
    pub fn requires_external_backend_json(
        profile_id: &str,
        capability: &str,
        backend: &str,
    ) -> String {
        super::RequiresExternalBackend::new(profile_id, capability, backend).to_json()
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
        require_external_backend, RequiresExternalBackend,
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
    fn external_backend_error_is_typed_and_operator_readable() {
        let err = require_external_backend("xrechnung-3.0", "reference_validate", "jvm:kosit")
            .expect_err("wasm reference validation must require a backend");
        assert_eq!(err.code, "requires_external_backend");
        assert_eq!(err.profile_id, "xrechnung-3.0");
        assert_eq!(err.capability, "reference_validate");
        assert_eq!(err.backend, "jvm:kosit");
        assert!(err
            .remediation
            .contains("WebAssembly never silently downgrades"));
        assert!(err.to_string().contains("requires external backend"));
    }

    #[test]
    fn external_backend_json_is_stable_and_escaped() {
        let json = RequiresExternalBackend::new("profile\"id", "reference_validate", "jvm:kosit")
            .to_json();
        assert_eq!(
            json,
            "{\"backend\":\"jvm:kosit\",\"capability\":\"reference_validate\",\"code\":\"requires_external_backend\",\"profile_id\":\"profile\\\"id\",\"remediation\":\"route this operation to a server-assisted validator with `jvm:kosit`; WebAssembly never silently downgrades to local validation\"}"
        );
    }

    #[test]
    fn wasm_reference_validation_abi_returns_typed_backend_error() {
        let response = process_engine_abi_json(
            br#"{"abi_version":1,"operation":"commercial_document.reference_validate","payload":{"profile_id":"xrechnung-3.0","backend":"jvm:kosit"}}"#,
        );
        let text = std::str::from_utf8(&response).expect("response is UTF-8 JSON");
        assert!(text.contains(r#""code":"requires_external_backend""#));
        assert!(text.contains(r#""backend":"jvm:kosit""#));
        assert!(text.contains(r#""profile_id":"xrechnung-3.0""#));
        assert!(text.contains(r#""operation":"commercial_document.reference_validate""#));
        assert!(!text.contains("unsupported_operation"));
    }

    #[test]
    fn wasm_reference_validation_abi_defaults_known_profile_backend() {
        let response = process_engine_abi_json(
            br#"{"abi_version":1,"operation":"commercial_document.reference_validate","payload":{"profile_id":"peppol-bis-3.0"}}"#,
        );
        let text = std::str::from_utf8(&response).expect("response is UTF-8 JSON");
        assert!(text.contains(r#""code":"requires_external_backend""#));
        assert!(text.contains(r#""backend":"jvm:phive""#));
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
