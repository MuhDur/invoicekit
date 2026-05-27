// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-transmit-mock` — InvoiceKit workspace member.
//!
//! This crate owns the deterministic cassette format used by the mock
//! transmission gateway. T-074 wires these types into the
//! `GatewayAdapter`; T-074a provides the recorder, scrubber, matcher, and
//! scenario metadata contract that downstream country crates can build on.
//!
//! Cassettes are JSON `.vcr` documents. Request matching is keyed by
//! method, path, and a BLAKE3 body fingerprint so a fixture never silently
//! reuses the wrong gateway response.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// JSON Schema document for `scenario.json` files stored beside cassettes.
///
/// The schema is intentionally small: each cassette directory names one
/// scenario, the source sandbox, the route being exercised, and the
/// scrubber profile used before committing the recording.
pub const SCENARIO_METADATA_SCHEMA_JSON: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://invoicekit.dev/schemas/cassette-scenario-v1.json",
  "title": "InvoiceKit cassette scenario metadata",
  "type": "object",
  "additionalProperties": false,
  "required": [
    "schema_version",
    "scenario_id",
    "title",
    "country",
    "route",
    "source",
    "scrubber_profile"
  ],
  "properties": {
    "schema_version": { "const": "1.0" },
    "scenario_id": { "type": "string", "pattern": "^[a-z0-9][a-z0-9._/-]*$" },
    "title": { "type": "string", "minLength": 1 },
    "country": { "type": "string", "pattern": "^[A-Z]{2}$" },
    "route": { "type": "string", "minLength": 1 },
    "source": {
      "type": "string",
      "enum": ["official-sandbox", "partner-sandbox", "synthetic"]
    },
    "scrubber_profile": { "type": "string", "minLength": 1 },
    "description": { "type": "string" },
    "tags": {
      "type": "array",
      "items": { "type": "string", "minLength": 1 },
      "uniqueItems": true
    }
  }
}"#;

/// Errors returned by cassette recording, scrubbing, and matching.
#[derive(Debug, Error)]
pub enum CassetteError {
    /// Required field was blank or absent.
    #[error(
        "{field} is required; hint: cassette records must be complete before they are committed"
    )]
    MissingRequiredField {
        /// Field name.
        field: &'static str,
    },
    /// Field failed a format rule.
    #[error("{field} is invalid: {reason}; hint: normalize the cassette input before recording")]
    InvalidField {
        /// Field name.
        field: &'static str,
        /// Human-readable validation failure.
        reason: String,
    },
    /// Two interactions had the same matcher key.
    #[error(
        "cassette collision for {method} {path} body={body_fingerprint}; hint: split scenarios or change the recorded request body"
    )]
    MatcherCollision {
        /// HTTP method.
        method: String,
        /// Request path.
        path: String,
        /// BLAKE3 fingerprint of the request body.
        body_fingerprint: String,
    },
    /// No interaction matched the requested key.
    #[error(
        "no cassette interaction matched {method} {path} body={body_fingerprint}; hint: record a cassette for this request"
    )]
    NoMatch {
        /// HTTP method.
        method: String,
        /// Request path.
        path: String,
        /// BLAKE3 fingerprint of the request body.
        body_fingerprint: String,
    },
    /// A cassette still contains text that looks like personal data.
    #[error(
        "cassette contains {finding_count} unscrubbed personal-data pattern(s); hint: add scrub rules before committing the cassette"
    )]
    UnscrubbedPii {
        /// Number of matched high-risk patterns.
        finding_count: usize,
    },
    /// JSON serialization or parsing failed.
    #[error("cassette JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Metadata stored in each cassette directory as `scenario.json`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScenarioMetadata {
    /// Scenario schema version.
    pub schema_version: String,
    /// Stable scenario identifier, for example `pl/ksef/accepted`.
    pub scenario_id: String,
    /// Human-readable title.
    pub title: String,
    /// ISO 3166-1 alpha-2 country code.
    pub country: String,
    /// Transmission route or gateway family, for example `peppol`.
    pub route: String,
    /// Recording source.
    pub source: ScenarioSource,
    /// Scrubber profile applied before commit.
    pub scrubber_profile: String,
    /// Optional longer description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Searchable scenario labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl ScenarioMetadata {
    /// Builds validated scenario metadata.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{ScenarioMetadata, ScenarioSource};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "de/ksef/success",
    ///     "KSeF accepted invoice",
    ///     "DE",
    ///     "ksef",
    ///     ScenarioSource::OfficialSandbox,
    ///     "default-de",
    /// )?;
    ///
    /// assert_eq!(scenario.schema_version, "1.0");
    /// assert_eq!(scenario.country, "DE");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError`] when identifiers are blank or when
    /// `country` is not an ISO 3166-1 alpha-2 uppercase code.
    pub fn new(
        scenario_id: impl Into<String>,
        title: impl Into<String>,
        country: impl Into<String>,
        route: impl Into<String>,
        source: ScenarioSource,
        scrubber_profile: impl Into<String>,
    ) -> Result<Self, CassetteError> {
        let scenario_id = non_empty(scenario_id.into(), "scenario_id")?;
        let title = non_empty(title.into(), "title")?;
        let country = country.into();
        validate_country(&country)?;
        let route = non_empty(route.into(), "route")?;
        let scrubber_profile = non_empty(scrubber_profile.into(), "scrubber_profile")?;
        Ok(Self {
            schema_version: "1.0".to_owned(),
            scenario_id,
            title,
            country,
            route,
            source,
            scrubber_profile,
            description: None,
            tags: Vec::new(),
        })
    }
}

/// Source used to create a cassette.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScenarioSource {
    /// Official regulator sandbox.
    OfficialSandbox,
    /// Partner access point or partner gateway sandbox.
    PartnerSandbox,
    /// Fully synthetic scenario.
    Synthetic,
}

/// A full deterministic `.vcr` cassette.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Cassette {
    /// Cassette format version.
    pub schema_version: String,
    /// Scenario metadata.
    pub scenario: ScenarioMetadata,
    /// Recorded request/response interactions.
    pub interactions: Vec<CassetteInteraction>,
}

impl Cassette {
    /// Serializes this cassette to byte-stable JSON `.vcr` bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{
    ///     CassetteRecorder, ScenarioMetadata, ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/accepted",
    ///     "Synthetic accepted invoice",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    /// let cassette = CassetteRecorder::new(scenario).finish();
    ///
    /// assert!(cassette.to_vcr_bytes()?.ends_with(b"\n"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError::Json`] if serialization fails.
    pub fn to_vcr_bytes(&self) -> Result<Vec<u8>, CassetteError> {
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Parses a cassette from JSON `.vcr` bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{
    ///     Cassette, CassetteRecorder, ScenarioMetadata, ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/accepted",
    ///     "Synthetic accepted invoice",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    /// let original = CassetteRecorder::new(scenario).finish();
    /// let parsed = Cassette::from_vcr_bytes(&original.to_vcr_bytes()?)?;
    ///
    /// assert_eq!(parsed, original);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError::Json`] if parsing fails.
    pub fn from_vcr_bytes(bytes: &[u8]) -> Result<Self, CassetteError> {
        Ok(serde_json::from_slice(bytes)?)
    }
}

/// One request/response pair inside a cassette.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CassetteInteraction {
    /// Recorded request.
    pub request: RecordedRequest,
    /// Recorded response.
    pub response: RecordedResponse,
}

/// Recorded gateway request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecordedRequest {
    /// Uppercase HTTP-like method or AS4 operation name.
    pub method: String,
    /// Request path, starting with `/`.
    pub path: String,
    /// Deterministically ordered request headers.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Request body as UTF-8 text.
    pub body: String,
    /// BLAKE3 fingerprint of `body`.
    pub body_fingerprint: String,
}

impl RecordedRequest {
    /// Builds a recorded request and computes its body fingerprint.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// use invoicekit_transmit_mock::{body_fingerprint, RecordedRequest};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let request = RecordedRequest::new(
    ///     "post",
    ///     "/sandbox/invoices",
    ///     BTreeMap::new(),
    ///     "<Invoice />",
    /// )?;
    ///
    /// assert_eq!(request.method, "POST");
    /// assert_eq!(
    ///     request.body_fingerprint,
    ///     body_fingerprint(b"<Invoice />")
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError`] when `method` is blank or `path` does not
    /// start with `/`.
    pub fn new(
        method: impl Into<String>,
        path: impl Into<String>,
        headers: BTreeMap<String, String>,
        body: impl Into<String>,
    ) -> Result<Self, CassetteError> {
        let method = non_empty(method.into(), "method")?.to_ascii_uppercase();
        let path = path.into();
        validate_path(&path)?;
        let body = body.into();
        let body_fingerprint = body_fingerprint(body.as_bytes());
        Ok(Self {
            method,
            path,
            headers,
            body,
            body_fingerprint,
        })
    }

    fn match_key(&self) -> MatchKey {
        MatchKey {
            method: self.method.clone(),
            path: self.path.clone(),
            body_fingerprint: body_fingerprint(self.body.as_bytes()),
        }
    }
}

/// Recorded gateway response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecordedResponse {
    /// HTTP-like status code.
    pub status: u16,
    /// Deterministically ordered response headers.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Response body as UTF-8 text.
    pub body: String,
}

impl RecordedResponse {
    /// Builds a recorded response.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// use invoicekit_transmit_mock::RecordedResponse;
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let response = RecordedResponse::new(202, BTreeMap::new(), r#"{"ok":true}"#)?;
    ///
    /// assert_eq!(response.status, 202);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError`] when `status` is outside `100..=599`.
    pub fn new(
        status: u16,
        headers: BTreeMap<String, String>,
        body: impl Into<String>,
    ) -> Result<Self, CassetteError> {
        if !(100..=599).contains(&status) {
            return Err(CassetteError::InvalidField {
                field: "status",
                reason: "expected HTTP status code in 100..=599".to_owned(),
            });
        }
        Ok(Self {
            status,
            headers,
            body: body.into(),
        })
    }
}

/// Deterministic recorder for `.vcr` cassettes.
pub struct CassetteRecorder {
    scenario: ScenarioMetadata,
    interactions: Vec<CassetteInteraction>,
}

impl CassetteRecorder {
    /// Starts recording a scenario.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{
    ///     CassetteRecorder, ScenarioMetadata, ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/accepted",
    ///     "Synthetic accepted invoice",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    /// let cassette = CassetteRecorder::new(scenario).finish();
    ///
    /// assert!(cassette.interactions.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(scenario: ScenarioMetadata) -> Self {
        Self {
            scenario,
            interactions: Vec::new(),
        }
    }

    /// Appends one request/response interaction.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// use invoicekit_transmit_mock::{
    ///     CassetteRecorder, RecordedRequest, RecordedResponse, ScenarioMetadata,
    ///     ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/accepted",
    ///     "Synthetic accepted invoice",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    /// let mut recorder = CassetteRecorder::new(scenario);
    /// let request = RecordedRequest::new("POST", "/invoices", BTreeMap::new(), "{}")?;
    /// let response = RecordedResponse::new(202, BTreeMap::new(), "{}")?;
    ///
    /// recorder.record(request, response);
    ///
    /// assert_eq!(recorder.finish().interactions.len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn record(&mut self, request: RecordedRequest, response: RecordedResponse) {
        self.interactions
            .push(CassetteInteraction { request, response });
    }

    /// Finishes the recording and returns a cassette.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{
    ///     CassetteRecorder, ScenarioMetadata, ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/accepted",
    ///     "Synthetic accepted invoice",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    ///
    /// let cassette = CassetteRecorder::new(scenario).finish();
    ///
    /// assert_eq!(cassette.schema_version, "1.0");
    /// # Ok(())
    /// # }
    /// ```
    pub fn finish(self) -> Cassette {
        Cassette {
            schema_version: "1.0".to_owned(),
            scenario: self.scenario,
            interactions: self.interactions,
        }
    }
}

/// Part of an interaction a scrub rule applies to.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrubScope {
    /// Request and response bodies.
    Body,
    /// Header values.
    Headers,
    /// Bodies and header values.
    All,
}

/// One country-scoped scrubber rule.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScrubRule {
    /// ISO country code or `*` for all countries.
    pub country: String,
    /// Sensitive literal to remove.
    pub find: String,
    /// Deterministic replacement token.
    pub replacement: String,
    /// Interaction scope.
    pub scope: ScrubScope,
}

impl ScrubRule {
    /// Builds a scrub rule.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{ScrubRule, ScrubScope};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let rule = ScrubRule::new("DE", "DE123456789", "[VAT-DE-1]", ScrubScope::All)?;
    ///
    /// assert_eq!(rule.country, "DE");
    /// assert_eq!(rule.replacement, "[VAT-DE-1]");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError`] when `country`, `find`, or
    /// `replacement` is blank, or when `country` is neither `*` nor an ISO
    /// uppercase alpha-2 code.
    pub fn new(
        country: impl Into<String>,
        find: impl Into<String>,
        replacement: impl Into<String>,
        scope: ScrubScope,
    ) -> Result<Self, CassetteError> {
        let country = country.into();
        if country != "*" {
            validate_country(&country)?;
        }
        Ok(Self {
            country,
            find: non_empty(find.into(), "find")?,
            replacement: non_empty(replacement.into(), "replacement")?,
            scope,
        })
    }

    fn applies_to(&self, country: &str) -> bool {
        self.country == "*" || self.country == country
    }
}

/// Configurable country-aware cassette scrubber.
#[derive(Default)]
pub struct Scrubber {
    rules: Vec<ScrubRule>,
}

impl Scrubber {
    /// Creates a scrubber from pre-validated rules.
    ///
    /// # Examples
    ///
    /// ```
    /// use invoicekit_transmit_mock::{ScrubRule, ScrubScope, Scrubber};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scrubber = Scrubber::new(vec![
    ///     ScrubRule::new("*", "secret@example.com", "[EMAIL-1]", ScrubScope::All)?,
    /// ]);
    ///
    /// let _ = scrubber;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn new(rules: Vec<ScrubRule>) -> Self {
        Self { rules }
    }

    /// Applies all rules for `country` and returns a scrubbed cassette.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// use invoicekit_transmit_mock::{
    ///     CassetteRecorder, RecordedRequest, RecordedResponse, ScenarioMetadata,
    ///     ScenarioSource, ScrubRule, ScrubScope, Scrubber,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/pii",
    ///     "Synthetic PII case",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "default-de",
    /// )?;
    /// let mut recorder = CassetteRecorder::new(scenario);
    /// let request = RecordedRequest::new("POST", "/invoices", BTreeMap::new(), "DE123456789")?;
    /// let response = RecordedResponse::new(202, BTreeMap::new(), "{}")?;
    /// recorder.record(request, response);
    ///
    /// let scrubber = Scrubber::new(vec![
    ///     ScrubRule::new("DE", "DE123456789", "[VAT-DE-1]", ScrubScope::Body)?,
    /// ]);
    /// let scrubbed = scrubber.scrub_cassette("DE", &recorder.finish());
    ///
    /// assert_eq!(scrubbed.interactions[0].request.body, "[VAT-DE-1]");
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn scrub_cassette(&self, country: &str, cassette: &Cassette) -> Cassette {
        let mut out = cassette.clone();
        for interaction in &mut out.interactions {
            for rule in self.rules.iter().filter(|rule| rule.applies_to(country)) {
                match rule.scope {
                    ScrubScope::Body => scrub_bodies(interaction, rule),
                    ScrubScope::Headers => scrub_headers(interaction, rule),
                    ScrubScope::All => {
                        scrub_bodies(interaction, rule);
                        scrub_headers(interaction, rule);
                    }
                }
            }
            interaction.request.body_fingerprint =
                body_fingerprint(interaction.request.body.as_bytes());
        }
        out
    }
}

/// Counts high-risk unsanitized personal-data patterns in a cassette.
///
/// This deliberately errs on the side of false positives for CI use. It
/// catches country-prefixed tax IDs such as `DE123456789`, IBAN-like
/// account numbers, and email addresses in request/response bodies and
/// header values.
///
/// # Examples
///
/// ```
/// use std::collections::BTreeMap;
///
/// use invoicekit_transmit_mock::{
///     count_unscrubbed_pii_patterns, CassetteRecorder, RecordedRequest,
///     RecordedResponse, ScenarioMetadata, ScenarioSource,
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let scenario = ScenarioMetadata::new(
///     "synthetic/pii",
///     "Synthetic PII case",
///     "DE",
///     "mock",
///     ScenarioSource::Synthetic,
///     "none",
/// )?;
/// let mut recorder = CassetteRecorder::new(scenario);
/// recorder.record(
///     RecordedRequest::new("POST", "/invoices", BTreeMap::new(), "DE123456789")?,
///     RecordedResponse::new(202, BTreeMap::new(), "{}")?,
/// );
///
/// assert_eq!(count_unscrubbed_pii_patterns(&recorder.finish()), 1);
/// # Ok(())
/// # }
/// ```
#[must_use]
pub fn count_unscrubbed_pii_patterns(cassette: &Cassette) -> usize {
    cassette
        .interactions
        .iter()
        .map(|interaction| {
            count_text_pii_patterns(&interaction.request.body)
                + count_text_pii_patterns(&interaction.response.body)
                + count_header_pii_patterns(&interaction.request.headers)
                + count_header_pii_patterns(&interaction.response.headers)
        })
        .sum()
}

/// Fails when a cassette still contains high-risk personal-data patterns.
///
/// # Errors
///
/// Returns [`CassetteError::UnscrubbedPii`] when bodies or headers still
/// contain values that look like tax identifiers, IBANs, or email
/// addresses.
///
/// # Examples
///
/// ```
/// use invoicekit_transmit_mock::{
///     assert_no_unscrubbed_pii_patterns, CassetteRecorder, ScenarioMetadata,
///     ScenarioSource,
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let scenario = ScenarioMetadata::new(
///     "synthetic/clean",
///     "Synthetic clean case",
///     "DE",
///     "mock",
///     ScenarioSource::Synthetic,
///     "none",
/// )?;
/// let cassette = CassetteRecorder::new(scenario).finish();
///
/// assert_no_unscrubbed_pii_patterns(&cassette)?;
/// # Ok(())
/// # }
/// ```
pub fn assert_no_unscrubbed_pii_patterns(cassette: &Cassette) -> Result<(), CassetteError> {
    let finding_count = count_unscrubbed_pii_patterns(cassette);
    if finding_count == 0 {
        Ok(())
    } else {
        Err(CassetteError::UnscrubbedPii { finding_count })
    }
}

/// Matcher keyed by method + path + body fingerprint.
pub struct CassetteMatcher<'a> {
    responses: BTreeMap<MatchKey, &'a RecordedResponse>,
}

impl<'a> CassetteMatcher<'a> {
    /// Builds a matcher and rejects duplicate request keys.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// use invoicekit_transmit_mock::{
    ///     CassetteMatcher, CassetteRecorder, RecordedRequest, RecordedResponse,
    ///     ScenarioMetadata, ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/replay",
    ///     "Synthetic replay case",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    /// let mut recorder = CassetteRecorder::new(scenario);
    /// recorder.record(
    ///     RecordedRequest::new("POST", "/invoices", BTreeMap::new(), "{}")?,
    ///     RecordedResponse::new(202, BTreeMap::new(), r#"{"accepted":true}"#)?,
    /// );
    /// let cassette = recorder.finish();
    ///
    /// let matcher = CassetteMatcher::new(&cassette)?;
    ///
    /// assert_eq!(
    ///     matcher.match_request(&cassette.interactions[0].request)?.status,
    ///     202
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError::MatcherCollision`] if two interactions
    /// share method, path, and body fingerprint.
    pub fn new(cassette: &'a Cassette) -> Result<Self, CassetteError> {
        let mut responses = BTreeMap::new();
        for interaction in &cassette.interactions {
            let key = interaction.request.match_key();
            if responses.contains_key(&key) {
                return Err(CassetteError::MatcherCollision {
                    method: key.method,
                    path: key.path,
                    body_fingerprint: key.body_fingerprint,
                });
            }
            responses.insert(key, &interaction.response);
        }
        Ok(Self { responses })
    }

    /// Returns the response for `request`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    ///
    /// use invoicekit_transmit_mock::{
    ///     CassetteMatcher, CassetteRecorder, RecordedRequest, RecordedResponse,
    ///     ScenarioMetadata, ScenarioSource,
    /// };
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let scenario = ScenarioMetadata::new(
    ///     "synthetic/replay",
    ///     "Synthetic replay case",
    ///     "DE",
    ///     "mock",
    ///     ScenarioSource::Synthetic,
    ///     "none",
    /// )?;
    /// let request = RecordedRequest::new("POST", "/invoices", BTreeMap::new(), "{}")?;
    /// let mut recorder = CassetteRecorder::new(scenario);
    /// recorder.record(
    ///     request.clone(),
    ///     RecordedResponse::new(202, BTreeMap::new(), r#"{"accepted":true}"#)?,
    /// );
    /// let cassette = recorder.finish();
    /// let matcher = CassetteMatcher::new(&cassette)?;
    ///
    /// assert_eq!(matcher.match_request(&request)?.body, r#"{"accepted":true}"#);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`CassetteError::NoMatch`] when no recorded interaction has
    /// the same method, path, and body fingerprint.
    pub fn match_request(
        &self,
        request: &RecordedRequest,
    ) -> Result<&'a RecordedResponse, CassetteError> {
        let key = request.match_key();
        self.responses
            .get(&key)
            .copied()
            .ok_or(CassetteError::NoMatch {
                method: key.method,
                path: key.path,
                body_fingerprint: key.body_fingerprint,
            })
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct MatchKey {
    method: String,
    path: String,
    body_fingerprint: String,
}

/// Computes the BLAKE3 body fingerprint used by [`CassetteMatcher`].
///
/// # Examples
///
/// ```
/// let first = invoicekit_transmit_mock::body_fingerprint(b"{}");
/// let second = invoicekit_transmit_mock::body_fingerprint(b"{}");
///
/// assert_eq!(first, second);
/// assert_ne!(first, invoicekit_transmit_mock::body_fingerprint(b"{\"x\":1}"));
/// ```
#[must_use]
pub fn body_fingerprint(body: &[u8]) -> String {
    blake3::hash(body).to_hex().to_string()
}

/// Parses the scenario metadata JSON Schema.
///
/// # Errors
///
/// Returns [`CassetteError::Json`] if the embedded schema is malformed.
///
/// # Examples
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let schema = invoicekit_transmit_mock::scenario_metadata_schema()?;
///
/// assert_eq!(schema["properties"]["schema_version"]["const"], "1.0");
/// # Ok(())
/// # }
/// ```
pub fn scenario_metadata_schema() -> Result<serde_json::Value, CassetteError> {
    Ok(serde_json::from_str(SCENARIO_METADATA_SCHEMA_JSON)?)
}

fn scrub_bodies(interaction: &mut CassetteInteraction, rule: &ScrubRule) {
    interaction.request.body = interaction
        .request
        .body
        .replace(rule.find.as_str(), rule.replacement.as_str());
    interaction.response.body = interaction
        .response
        .body
        .replace(rule.find.as_str(), rule.replacement.as_str());
}

fn scrub_headers(interaction: &mut CassetteInteraction, rule: &ScrubRule) {
    for value in interaction.request.headers.values_mut() {
        *value = value.replace(rule.find.as_str(), rule.replacement.as_str());
    }
    for value in interaction.response.headers.values_mut() {
        *value = value.replace(rule.find.as_str(), rule.replacement.as_str());
    }
}

fn count_header_pii_patterns(headers: &BTreeMap<String, String>) -> usize {
    headers
        .values()
        .map(|value| count_text_pii_patterns(value))
        .sum()
}

fn count_text_pii_patterns(value: &str) -> usize {
    value
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | '-' | '+')))
        .filter(|token| {
            looks_like_country_tax_id(token) || looks_like_iban(token) || looks_like_email(token)
        })
        .count()
}

fn looks_like_country_tax_id(token: &str) -> bool {
    if !(10..=14).contains(&token.len()) {
        return false;
    }
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let Some(second) = chars.next() else {
        return false;
    };
    first.is_ascii_uppercase() && second.is_ascii_uppercase() && chars.all(|c| c.is_ascii_digit())
}

fn looks_like_iban(token: &str) -> bool {
    if !(15..=34).contains(&token.len()) {
        return false;
    }
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let Some(second) = chars.next() else {
        return false;
    };
    let Some(third) = chars.next() else {
        return false;
    };
    let Some(fourth) = chars.next() else {
        return false;
    };
    first.is_ascii_uppercase()
        && second.is_ascii_uppercase()
        && third.is_ascii_digit()
        && fourth.is_ascii_digit()
        && chars.all(|c| c.is_ascii_alphanumeric())
}

fn looks_like_email(token: &str) -> bool {
    let Some((local, domain)) = token.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
}

fn non_empty(value: String, field: &'static str) -> Result<String, CassetteError> {
    if value.trim().is_empty() {
        return Err(CassetteError::MissingRequiredField { field });
    }
    Ok(value)
}

fn validate_country(country: &str) -> Result<(), CassetteError> {
    if country.len() == 2 && country.chars().all(|c| c.is_ascii_uppercase()) {
        return Ok(());
    }
    Err(CassetteError::InvalidField {
        field: "country",
        reason: "expected ISO 3166-1 alpha-2 uppercase code".to_owned(),
    })
}

fn validate_path(path: &str) -> Result<(), CassetteError> {
    if path.starts_with('/') && !path.trim().is_empty() {
        return Ok(());
    }
    Err(CassetteError::InvalidField {
        field: "path",
        reason: "expected an absolute request path starting with `/`".to_owned(),
    })
}

/// Canonical Cargo package name of this crate.
///
/// Used by the InvoiceKit release tooling and by the bead-correlation
/// reports to map runtime log records back to the originating crate
/// without parsing `Cargo.toml` at runtime.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_transmit_mock::crate_name(), "invoicekit-transmit-mock");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-transmit-mock"
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        assert_no_unscrubbed_pii_patterns, body_fingerprint, count_unscrubbed_pii_patterns,
        crate_name, scenario_metadata_schema, CassetteMatcher, CassetteRecorder, RecordedRequest,
        RecordedResponse, ScenarioMetadata, ScenarioSource, ScrubRule, ScrubScope, Scrubber,
    };

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-transmit-mock");
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
    fn recorder_emits_byte_stable_vcr_bytes() {
        let first = sample_cassette_bytes();
        let second = sample_cassette_bytes();

        assert_eq!(first, second);
        assert!(String::from_utf8(first)
            .unwrap()
            .contains("\"schema_version\": \"1.0\""));
    }

    #[test]
    fn scrubber_removes_country_scoped_personal_data() {
        let cassette = sample_cassette();
        let scrubber = Scrubber::new(vec![
            ScrubRule::new("DE", "DE123456789", "[VAT-DE-1]", ScrubScope::All).unwrap(),
            ScrubRule::new("FR", "FR123456789", "[VAT-FR-1]", ScrubScope::All).unwrap(),
        ]);

        let redacted = scrubber.scrub_cassette("DE", &cassette);
        let bytes = String::from_utf8(redacted.to_vcr_bytes().unwrap()).unwrap();

        assert!(!bytes.contains("DE123456789"));
        assert!(bytes.contains("[VAT-DE-1]"));
        assert!(bytes.contains("FR123456789"));
        let interaction = first_interaction(&redacted);
        assert_eq!(
            interaction.request.body_fingerprint,
            body_fingerprint(interaction.request.body.as_bytes())
        );
    }

    #[test]
    fn pii_scan_reports_unscrubbed_patterns() {
        let cassette = sample_cassette();

        assert!(count_unscrubbed_pii_patterns(&cassette) >= 2);
        assert!(assert_no_unscrubbed_pii_patterns(&cassette).is_err());
    }

    #[test]
    fn pii_scan_passes_after_all_sensitive_values_are_scrubbed() {
        let cassette = sample_cassette();
        let scrubber = Scrubber::new(vec![
            ScrubRule::new("*", "DE123456789", "[VAT-DE-1]", ScrubScope::All).unwrap(),
            ScrubRule::new("*", "FR123456789", "[VAT-FR-1]", ScrubScope::All).unwrap(),
        ]);

        let redacted = scrubber.scrub_cassette("DE", &cassette);

        assert_eq!(count_unscrubbed_pii_patterns(&redacted), 0);
        assert!(assert_no_unscrubbed_pii_patterns(&redacted).is_ok());
    }

    #[test]
    fn matcher_routes_by_method_path_and_body_fingerprint() {
        let cassette = sample_cassette();
        let matcher = CassetteMatcher::new(&cassette).unwrap();
        let response = matcher
            .match_request(&first_interaction(&cassette).request)
            .unwrap();

        assert_eq!(response.status, 202);

        let different_body = RecordedRequest::new(
            "POST",
            "/ksef/invoices",
            BTreeMap::new(),
            "<Invoice>different</Invoice>",
        )
        .unwrap();
        assert!(matcher.match_request(&different_body).is_err());
    }

    #[test]
    fn matcher_derives_fingerprint_from_body_not_stored_field() {
        let mut cassette = sample_cassette();
        first_interaction_mut(&mut cassette)
            .request
            .body_fingerprint = body_fingerprint(b"stale");
        let matcher = CassetteMatcher::new(&cassette).unwrap();
        let request = RecordedRequest::new(
            "POST",
            "/ksef/invoices",
            BTreeMap::new(),
            "<Invoice><Seller>DE123456789</Seller></Invoice>",
        )
        .unwrap();

        assert_eq!(matcher.match_request(&request).unwrap().status, 202);
    }

    #[test]
    fn matcher_rejects_colliding_interactions() {
        let mut cassette = sample_cassette();
        let repeated = first_interaction(&cassette).clone();
        cassette.interactions.push(repeated);

        assert!(CassetteMatcher::new(&cassette).is_err());
    }

    #[test]
    fn scenario_metadata_schema_is_valid_json() {
        let schema = scenario_metadata_schema().unwrap();

        assert_eq!(schema["title"], "InvoiceKit cassette scenario metadata");
        assert_eq!(schema["properties"]["schema_version"]["const"], "1.0");
    }

    #[test]
    fn invalid_country_is_rejected() {
        let err = ScenarioMetadata::new(
            "de/ksef/success",
            "KSeF success",
            "de",
            "ksef",
            ScenarioSource::OfficialSandbox,
            "default-de",
        )
        .unwrap_err();

        assert!(err.to_string().contains("country"));
    }

    fn sample_cassette_bytes() -> Vec<u8> {
        sample_cassette().to_vcr_bytes().unwrap()
    }

    fn first_interaction(cassette: &super::Cassette) -> &super::CassetteInteraction {
        cassette.interactions.first().unwrap()
    }

    fn first_interaction_mut(cassette: &mut super::Cassette) -> &mut super::CassetteInteraction {
        cassette.interactions.first_mut().unwrap()
    }

    fn sample_cassette() -> super::Cassette {
        let scenario = ScenarioMetadata::new(
            "de/ksef/success",
            "KSeF accepted invoice",
            "DE",
            "ksef",
            ScenarioSource::OfficialSandbox,
            "default-de",
        )
        .unwrap();
        let mut recorder = CassetteRecorder::new(scenario);
        let mut request_headers = BTreeMap::new();
        request_headers.insert("x-tax-id".to_owned(), "DE123456789".to_owned());
        let request = RecordedRequest::new(
            "post",
            "/ksef/invoices",
            request_headers,
            "<Invoice><Seller>DE123456789</Seller></Invoice>",
        )
        .unwrap();
        let response = RecordedResponse::new(
            202,
            BTreeMap::new(),
            "{\"status\":\"accepted\",\"buyer\":\"FR123456789\"}",
        )
        .unwrap();
        recorder.record(request, response);
        recorder.finish()
    }
}
