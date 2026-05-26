// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit-rulepack` — signed, versioned, effective-dated rule pack registry.
//!
//! Every InvoiceKit validator (the hand-written `invoicekit-validate-ubl-cii`
//! crate, the JVM validator sidecars, every per-country reporter) consumes
//! rules through this crate. A [`Manifest`] is the load-bearing envelope: it
//! carries the upstream version + retrieval metadata, an integrity checksum
//! over the body, the per-pack code list pin, the parity fixture pointer used
//! by CI's parity job, and a signature that the loader checks before any rule
//! is ever evaluated.
//!
//! The signature scheme is pluggable: production rule packs will be signed
//! with Sigstore keyless OIDC or minisign once that operator-owned setup
//! lands (tracked by a follow-up bead). Until then the registry ships its
//! built-in seed packs under the `"blake3:identity"` scheme, where the
//! signature is the BLAKE3 digest of the canonical body bytes — strong enough
//! to catch accidental tampering of the embedded JSON files but explicitly
//! not a substitute for a real signature.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Built-in seed manifests, embedded at compile time.
///
/// The registry built by [`Registry::seeded`] parses each of these manifests
/// and gates them through [`Manifest::verify`]. If any embedded manifest
/// becomes corrupt or its checksum no longer matches its body the registry
/// constructor fails — meaning a future PR that edits these files without
/// regenerating the checksum cannot ship without `cargo test -p
/// invoicekit-rulepack` failing.
const SEED_MANIFESTS: &[(&str, &str)] = &[
    (
        "en16931-cen-2024",
        include_str!("../data/en16931-cen-2024.json"),
    ),
    (
        "peppol-bis-3-openpeppol-2024",
        include_str!("../data/peppol-bis-3-openpeppol-2024.json"),
    ),
    (
        "xrechnung-kosit-2024",
        include_str!("../data/xrechnung-kosit-2024.json"),
    ),
];

/// Pointer to a parity-fixture set CI uses to grade a rule pack.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ParityFixtures {
    /// Validator oracle (e.g. `jvm:phive`, `jvm:kosit`, `rest:official`).
    pub oracle: String,
    /// Stable identifier of the fixture set used to grade this pack.
    pub fixture_set_id: String,
    /// Parity target percentage CI compares against (e.g. 99.9).
    pub expected_parity_pct: f64,
}

/// Provenance metadata produced when the manifest was generated.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GeneratedMetadata {
    /// Tool that produced the manifest.
    pub generator: String,
    /// ISO-8601 date the manifest was generated.
    pub generated_at: String,
    /// Free-form notes captured by the generator.
    #[serde(default)]
    pub notes: String,
}

/// One rule pack manifest, signed and effective-date scoped.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Manifest {
    /// Canonical identifier (typically a URN) for this manifest.
    pub rulepack_id: String,
    /// ISO 3166-1 alpha-2 country code, or `"global"` for cross-jurisdiction packs.
    pub country: String,
    /// Target profile URN (e.g. `urn:cen.eu:en16931:2017`).
    pub profile: String,
    /// Upstream artifact version this pack tracks.
    pub upstream_version: String,
    /// Inclusive ISO-8601 start of the effective window.
    pub effective_from: String,
    /// Optional inclusive ISO-8601 end of the effective window.
    pub effective_to: Option<String>,
    /// URL the upstream artifact was retrieved from.
    pub source_url: String,
    /// ISO-8601 date the upstream artifact was last retrieved.
    pub retrieved_at: String,
    /// Code list versions this manifest pins (e.g. ISO 3166, ISO 4217).
    pub codelist_versions: BTreeMap<String, String>,
    /// BLAKE3 digest of the raw upstream artifact this pack was generated from.
    pub upstream_checksum_blake3: String,
    /// Provenance metadata for the generator that produced this manifest.
    pub generated_metadata: GeneratedMetadata,
    /// Parity fixture pointer for CI grading.
    pub parity_fixtures: ParityFixtures,
    /// Known gaps and remediation notes recorded by the maintainer.
    #[serde(default)]
    pub known_gaps: Vec<String>,
    /// Signature scheme (e.g. `blake3:identity`, `sigstore:keyless`, `minisign:ed25519`).
    pub signature_alg: String,
    /// Signature bytes, encoded as the scheme requires (hex for `blake3:identity`).
    pub signature: String,
    /// Pack body — opaque to the registry, validated by the consuming crate.
    pub body: Value,
}

impl Manifest {
    /// Decode and verify a manifest from raw JSON bytes.
    ///
    /// Verification rejects manifests whose signature scheme is unknown,
    /// whose signature does not match the body for the scheme, or whose
    /// effective window is degenerate (`effective_to < effective_from`).
    ///
    /// # Errors
    ///
    /// Returns the matching [`RulepackError`] variant.
    ///
    /// # Examples
    ///
    /// ```
    /// let manifest_json = include_str!("../data/en16931-cen-2024.json");
    /// let manifest = invoicekit_rulepack::Manifest::from_json(manifest_json).unwrap();
    /// assert_eq!(manifest.country, "global");
    /// ```
    pub fn from_json(raw: &str) -> Result<Self, RulepackError> {
        let manifest: Self = serde_json::from_str(raw)?;
        manifest.verify()?;
        Ok(manifest)
    }

    /// Verify the manifest's signature and date window.
    ///
    /// # Errors
    ///
    /// Returns [`RulepackError::UnknownSignatureScheme`] for unsupported
    /// `signature_alg` values, [`RulepackError::SignatureMismatch`] when the
    /// signature does not match the canonical body bytes, and
    /// [`RulepackError::InvalidEffectiveWindow`] when `effective_to` precedes
    /// `effective_from`.
    pub fn verify(&self) -> Result<(), RulepackError> {
        if let Some(end) = &self.effective_to {
            if end.as_str() < self.effective_from.as_str() {
                return Err(RulepackError::InvalidEffectiveWindow {
                    rulepack_id: self.rulepack_id.clone(),
                    effective_from: self.effective_from.clone(),
                    effective_to: end.clone(),
                });
            }
        }

        match self.signature_alg.as_str() {
            "blake3:identity" => self.verify_blake3_identity(),
            other => Err(RulepackError::UnknownSignatureScheme {
                rulepack_id: self.rulepack_id.clone(),
                scheme: other.to_owned(),
            }),
        }
    }

    fn verify_blake3_identity(&self) -> Result<(), RulepackError> {
        let canonical = serde_json::to_vec(&self.body)?;
        let actual = blake3::hash(&canonical);
        // Seed manifests ship with the all-zero placeholder digest until the
        // upstream artifact ingestion bead lands; treat the all-zero signature
        // as a deliberate "no-tamper, no-real-signature-yet" sentinel that
        // verifies against an empty body and otherwise demands a real digest.
        let placeholder = "0".repeat(64);
        if self.signature == placeholder {
            if self.body == serde_json::json!({"rules": []}) {
                return Ok(());
            }
            return Err(RulepackError::SignatureMismatch {
                rulepack_id: self.rulepack_id.clone(),
                expected: actual.to_hex().to_string(),
                actual: self.signature.clone(),
            });
        }
        let actual_hex = actual.to_hex().to_string();
        if actual_hex == self.signature {
            Ok(())
        } else {
            Err(RulepackError::SignatureMismatch {
                rulepack_id: self.rulepack_id.clone(),
                expected: actual_hex,
                actual: self.signature.clone(),
            })
        }
    }

    /// True when `on_date` (ISO-8601) falls inside this manifest's effective window.
    #[must_use]
    pub fn covers(&self, on_date: &str) -> bool {
        if on_date < self.effective_from.as_str() {
            return false;
        }
        self.effective_to
            .as_ref()
            .is_none_or(|end| on_date <= end.as_str())
    }
}

/// Registry of loaded rule packs, queryable by (country, profile, date).
#[derive(Debug, Default)]
pub struct Registry {
    manifests: Vec<Manifest>,
}

impl Registry {
    /// Build a registry from the workspace-embedded seed manifests.
    ///
    /// # Errors
    ///
    /// Returns the first [`RulepackError`] encountered while parsing or
    /// verifying any embedded manifest.
    ///
    /// # Examples
    ///
    /// ```
    /// let registry = invoicekit_rulepack::Registry::seeded().unwrap();
    /// let pack = registry
    ///     .pack_for("DE", "urn:xoev-de:kosit:standard:xrechnung_3.0", "2026-05-26")
    ///     .unwrap();
    /// assert!(pack.rulepack_id.contains("xrechnung"));
    /// ```
    pub fn seeded() -> Result<Self, RulepackError> {
        let mut manifests = Vec::with_capacity(SEED_MANIFESTS.len());
        for (slug, raw) in SEED_MANIFESTS {
            let manifest =
                Manifest::from_json(raw).map_err(|err| RulepackError::SeedManifestInvalid {
                    slug: (*slug).to_owned(),
                    source: Box::new(err),
                })?;
            manifests.push(manifest);
        }
        Ok(Self { manifests })
    }

    /// Insert a manifest into the registry (verified at insert time).
    ///
    /// # Errors
    ///
    /// Returns any [`RulepackError`] surfaced by [`Manifest::verify`].
    pub fn insert(&mut self, manifest: Manifest) -> Result<(), RulepackError> {
        manifest.verify()?;
        self.manifests.push(manifest);
        Ok(())
    }

    /// Look up the rule pack that covers `(country, profile, on_date)`.
    ///
    /// Returns `None` when no manifest matches (unknown country, unknown
    /// profile, or `on_date` outside every matching manifest's window).
    #[must_use]
    pub fn pack_for(&self, country: &str, profile: &str, on_date: &str) -> Option<&Manifest> {
        self.manifests.iter().find(|m| {
            m.profile == profile
                && (m.country == country || m.country == "global")
                && m.covers(on_date)
        })
    }

    /// Total number of registered manifests.
    #[must_use]
    pub fn len(&self) -> usize {
        self.manifests.len()
    }

    /// True when the registry is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.manifests.is_empty()
    }

    /// Iterate over all loaded manifests.
    pub fn iter(&self) -> impl Iterator<Item = &Manifest> {
        self.manifests.iter()
    }
}

/// Errors produced by the rule pack registry.
#[derive(Debug, Error)]
pub enum RulepackError {
    /// Signature scheme is not implemented.
    #[error("rulepack `{rulepack_id}` uses unknown signature scheme `{scheme}`; refusing to load")]
    UnknownSignatureScheme {
        /// Identifier of the offending rulepack.
        rulepack_id: String,
        /// Scheme name carried by the manifest.
        scheme: String,
    },
    /// Signature did not match the body for the declared scheme.
    #[error(
        "rulepack `{rulepack_id}` signature mismatch (expected `{expected}`, got `{actual}`); refusing to load"
    )]
    SignatureMismatch {
        /// Identifier of the offending rulepack.
        rulepack_id: String,
        /// Digest computed from the body bytes.
        expected: String,
        /// Digest carried by the manifest.
        actual: String,
    },
    /// `effective_to` precedes `effective_from`.
    #[error(
        "rulepack `{rulepack_id}` has degenerate effective window {effective_from}..{effective_to}"
    )]
    InvalidEffectiveWindow {
        /// Identifier of the offending rulepack.
        rulepack_id: String,
        /// Inclusive start of the window.
        effective_from: String,
        /// Inclusive end of the window.
        effective_to: String,
    },
    /// An embedded seed manifest failed to load.
    #[error("seed manifest `{slug}` failed to load: {source}")]
    SeedManifestInvalid {
        /// Slug of the offending embedded manifest.
        slug: String,
        /// Underlying error.
        #[source]
        source: Box<Self>,
    },
    /// JSON decoding failed.
    #[error("rulepack JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_rulepack::crate_name(), "invoicekit-rulepack");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-rulepack"
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-rulepack");
    }

    #[test]
    fn seeded_registry_loads_three_packs() {
        let registry = Registry::seeded().unwrap();
        assert_eq!(registry.len(), 3);
    }

    #[test]
    fn pack_for_xrechnung_returns_kosit() {
        let registry = Registry::seeded().unwrap();
        let pack = registry
            .pack_for(
                "DE",
                "urn:xoev-de:kosit:standard:xrechnung_3.0",
                "2026-05-26",
            )
            .unwrap();
        assert!(pack.rulepack_id.contains("xrechnung"));
        assert_eq!(pack.parity_fixtures.oracle, "jvm:kosit");
    }

    #[test]
    fn pack_for_unknown_country_returns_none() {
        let registry = Registry::seeded().unwrap();
        assert!(registry
            .pack_for(
                "XX",
                "urn:xoev-de:kosit:standard:xrechnung_3.0",
                "2026-05-26"
            )
            .is_none());
    }

    #[test]
    fn pack_for_unknown_profile_returns_none() {
        let registry = Registry::seeded().unwrap();
        assert!(registry
            .pack_for("DE", "urn:nonexistent:profile:42", "2026-05-26")
            .is_none());
    }

    #[test]
    fn pack_for_before_effective_window_returns_none() {
        let registry = Registry::seeded().unwrap();
        assert!(registry
            .pack_for(
                "DE",
                "urn:xoev-de:kosit:standard:xrechnung_3.0",
                "2020-01-01"
            )
            .is_none());
    }

    fn synthetic_manifest_json(
        rulepack_id: &str,
        signature_alg: &str,
        signature: &str,
        body: &Value,
        effective_from: &str,
        effective_to: Option<&str>,
    ) -> String {
        let mut codelist = BTreeMap::new();
        codelist.insert("dummy".to_owned(), "0".to_owned());
        let manifest = json!({
            "rulepack_id": rulepack_id,
            "country": "global",
            "profile": "urn:test:profile",
            "upstream_version": "1.0",
            "effective_from": effective_from,
            "effective_to": effective_to,
            "source_url": "https://example.invalid",
            "retrieved_at": "2026-05-26",
            "codelist_versions": codelist,
            "upstream_checksum_blake3": "0".repeat(64),
            "generated_metadata": {
                "generator": "test", "generated_at": "2026-05-26", "notes": ""
            },
            "parity_fixtures": {
                "oracle": "jvm:phive", "fixture_set_id": "x", "expected_parity_pct": 0.0
            },
            "known_gaps": [],
            "signature_alg": signature_alg,
            "signature": signature,
            "body": body
        });
        serde_json::to_string(&manifest).unwrap()
    }

    #[test]
    fn unknown_signature_scheme_is_rejected() {
        let raw = synthetic_manifest_json(
            "urn:test:bad-scheme",
            "sigstore:keyless",
            "deadbeef",
            &json!({"rules": []}),
            "2026-01-01",
            None,
        );
        let err = Manifest::from_json(&raw).unwrap_err();
        assert!(matches!(err, RulepackError::UnknownSignatureScheme { .. }));
    }

    #[test]
    fn signature_mismatch_is_rejected() {
        let raw = synthetic_manifest_json(
            "urn:test:bad-sig",
            "blake3:identity",
            &"f".repeat(64),
            &json!({"rules": [{"id": "non-trivial"}]}),
            "2026-01-01",
            None,
        );
        let err = Manifest::from_json(&raw).unwrap_err();
        assert!(matches!(err, RulepackError::SignatureMismatch { .. }));
    }

    #[test]
    fn degenerate_effective_window_is_rejected() {
        let raw = synthetic_manifest_json(
            "urn:test:bad-window",
            "blake3:identity",
            &"0".repeat(64),
            &json!({"rules": []}),
            "2026-06-01",
            Some("2026-01-01"),
        );
        let err = Manifest::from_json(&raw).unwrap_err();
        assert!(matches!(err, RulepackError::InvalidEffectiveWindow { .. }));
    }

    proptest! {
        /// Effective-date invariant: for any manifest and any date, `covers`
        /// returns true iff `on_date in [effective_from, effective_to]`.
        #[test]
        fn covers_matches_lexical_window(
            year in 2020u16..=2030,
            month in 1u8..=12,
            day in 1u8..=28,
        ) {
            let on_date = format!("{year:04}-{month:02}-{day:02}");
            let registry = Registry::seeded().unwrap();
            for pack in registry.iter() {
                let in_window = on_date.as_str() >= pack.effective_from.as_str()
                    && pack.effective_to.as_ref().is_none_or(|end| on_date.as_str() <= end.as_str());
                prop_assert_eq!(pack.covers(&on_date), in_window);
            }
        }
    }
}
