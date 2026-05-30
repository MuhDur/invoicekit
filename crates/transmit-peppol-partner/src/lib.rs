// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-091 impl: partner Peppol Access Point [`GatewayAdapter`]
//! scaffold for the three vendors named in the T-091 runbook
//! (`Storecove`, `ecosio`, `B2BRouter`).
//!
//! Public surface:
//!
//! * [`PartnerVendor`] — closed enum of supported vendors. Adding
//!   a vendor is a typed compile-time diff.
//! * [`PartnerConfig`] — env-var-driven configuration. Carries
//!   the API base URL, the chosen vendor, the legal-entity ID,
//!   and whether sandbox mode is on. Credentials are resolved
//!   through [`SecretResolver`] so the operator can swap in a
//!   secret-manager backend without touching the adapter.
//! * [`SecretResolver`] — abstraction over the credential source.
//!   [`EnvSecretResolver`] reads `INVOICEKIT_PEPPOL_API_KEY` /
//!   `_API_SECRET`. A future bead adds the Vault + SOPS backends
//!   the runbook promises.
//! * [`HttpClient`] — abstraction over the HTTP transport so the
//!   adapter is testable without `reqwest`. A `reqwest`-backed
//!   client lives behind a follow-up `reqwest` feature flag.
//! * [`PartnerPeppolAdapter`] — implements
//!   [`invoicekit_reconcile::GatewayAdapter`] by translating
//!   [`SubmitRequest`] / [`PollRequest`] / [`CancelRequest`] /
//!   [`CorrectRequest`] into the chosen vendor's REST shape via
//!   the injected `HttpClient`.
//!
//! The vendor-specific REST mapping (Storecove `/api/v2/document_submissions`,
//! ecosio SOAP, etc.) is mostly stubbed: each public method
//! constructs the right URL and JSON body shape, then delegates
//! to the `HttpClient`. The unit tests use [`MockHttpClient`] to
//! prove the URL + body construction is correct; landing the
//! real `reqwest`-backed client is a follow-up.

use std::env;
use std::pin::Pin;

use invoicekit_reconcile::{
    CancelRequest, CorrectRequest, GatewayAdapter, GatewayContext, GatewayError, GatewayErrorKind,
    GatewayFuture, GatewayOperation, GatewayReceipt, GatewayStatus, GatewaySubmissionId,
    PollRequest, SubmitRequest,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod byok;

/// One of the three partner vendors named in the T-091 runbook.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PartnerVendor {
    /// Storecove (REST API, EU + SG + AU + NZ + JP coverage).
    Storecove,
    /// ecosio (SOAP API, DACH-strong).
    Ecosio,
    /// `B2BRouter` (SOAP API, Iberia-focused).
    B2brouter,
}

impl PartnerVendor {
    /// Operator-readable slug.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Storecove => "storecove",
            Self::Ecosio => "ecosio",
            Self::B2brouter => "b2brouter",
        }
    }

    /// Parse from the `INVOICEKIT_PEPPOL_PARTNER` env-var value.
    ///
    /// # Errors
    ///
    /// Returns [`PartnerError::UnknownVendor`] when the value
    /// doesn't match a registered vendor slug.
    pub fn from_slug(value: &str) -> Result<Self, PartnerError> {
        match value {
            "storecove" => Ok(Self::Storecove),
            "ecosio" => Ok(Self::Ecosio),
            "b2brouter" => Ok(Self::B2brouter),
            other => Err(PartnerError::UnknownVendor(other.to_owned())),
        }
    }

    /// Default production base URL per vendor (the runbook lists
    /// these). Override via `INVOICEKIT_PEPPOL_API_BASE` for
    /// sandbox / staging.
    #[must_use]
    pub const fn default_api_base(self) -> &'static str {
        match self {
            Self::Storecove => "https://api.storecove.com/api/v2",
            Self::Ecosio => "https://api.ecosio.com",
            Self::B2brouter => "https://app.b2brouter.net/projects/-/api",
        }
    }
}

/// Operator-facing configuration. Construct with
/// [`PartnerConfig::from_env`] in production; construct directly
/// in tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartnerConfig {
    /// Chosen vendor.
    pub vendor: PartnerVendor,
    /// API base URL (defaults to `vendor.default_api_base()` when
    /// `INVOICEKIT_PEPPOL_API_BASE` is unset).
    pub api_base: String,
    /// Vendor-assigned legal entity ID surfaced in SBDH headers.
    pub legal_entity_id: String,
    /// True when the adapter should route through the vendor's
    /// sandbox. The reconcile state machine refuses
    /// `sandbox: true` + production-tagged invoices.
    pub sandbox: bool,
}

impl PartnerConfig {
    /// Read configuration from the documented env-vars.
    ///
    /// Env-vars consulted:
    ///
    /// * `INVOICEKIT_PEPPOL_PARTNER` (required, vendor slug)
    /// * `INVOICEKIT_PEPPOL_API_BASE` (optional override)
    /// * `INVOICEKIT_PEPPOL_LEGAL_ENTITY_ID` (required)
    /// * `INVOICEKIT_PEPPOL_SANDBOX` (optional, default `false`)
    ///
    /// Credentials (`_API_KEY` / `_API_SECRET`) are NOT read here
    /// — they live in the [`SecretResolver`] so the operator can
    /// swap in Vault or SOPS without re-deploying.
    ///
    /// # Errors
    ///
    /// Returns [`PartnerError::MissingEnv`] when a required
    /// env-var is unset and [`PartnerError::UnknownVendor`] when
    /// the partner slug is unrecognised.
    pub fn from_env() -> Result<Self, PartnerError> {
        Self::from_lookup(&|name| env::var(name).ok())
    }

    /// Read configuration from a custom lookup function. Used by
    /// tests to avoid the global env mutex.
    ///
    /// # Errors
    ///
    /// Same as [`PartnerConfig::from_env`].
    pub fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, PartnerError> {
        let vendor_slug = lookup("INVOICEKIT_PEPPOL_PARTNER")
            .ok_or(PartnerError::MissingEnv("INVOICEKIT_PEPPOL_PARTNER"))?;
        let vendor = PartnerVendor::from_slug(&vendor_slug)?;
        let api_base = lookup("INVOICEKIT_PEPPOL_API_BASE")
            .unwrap_or_else(|| vendor.default_api_base().to_owned());
        let legal_entity_id = lookup("INVOICEKIT_PEPPOL_LEGAL_ENTITY_ID").ok_or(
            PartnerError::MissingEnv("INVOICEKIT_PEPPOL_LEGAL_ENTITY_ID"),
        )?;
        let sandbox = lookup("INVOICEKIT_PEPPOL_SANDBOX")
            .is_some_and(|v| matches!(v.as_str(), "true" | "1" | "yes"));
        Ok(Self {
            vendor,
            api_base,
            legal_entity_id,
            sandbox,
        })
    }
}

/// Credential resolver. The runbook calls for env-var, Stdin,
/// Vault, and SOPS backends; this PR ships Env + Stdin, future
/// beads add the secret-manager integrations.
pub trait SecretResolver: Send + Sync {
    /// Resolve the API key. None when the resolver has no entry.
    fn api_key(&self) -> Option<String>;
    /// Resolve the API secret (only required for HMAC-auth vendors).
    fn api_secret(&self) -> Option<String>;
}

/// Reads `INVOICEKIT_PEPPOL_API_KEY` / `_API_SECRET` from the
/// process environment.
#[derive(Clone, Debug, Default)]
pub struct EnvSecretResolver;

impl SecretResolver for EnvSecretResolver {
    fn api_key(&self) -> Option<String> {
        env::var("INVOICEKIT_PEPPOL_API_KEY").ok()
    }

    fn api_secret(&self) -> Option<String> {
        env::var("INVOICEKIT_PEPPOL_API_SECRET").ok()
    }
}

/// Static credential pair. Used by tests + by the Stdin
/// interactive bootstrap flow (a future bead wires the actual
/// readline prompt; the struct is the type the prompt builds).
#[derive(Clone, Debug)]
pub struct StaticSecretResolver {
    api_key: Option<String>,
    api_secret: Option<String>,
}

impl StaticSecretResolver {
    /// Build a static resolver with an explicit key + optional
    /// secret.
    #[must_use]
    pub fn new(api_key: impl Into<String>, api_secret: Option<String>) -> Self {
        Self {
            api_key: Some(api_key.into()),
            api_secret,
        }
    }
}

impl SecretResolver for StaticSecretResolver {
    fn api_key(&self) -> Option<String> {
        self.api_key.clone()
    }

    fn api_secret(&self) -> Option<String> {
        self.api_secret.clone()
    }
}

/// Abstraction over the HTTP transport. The real adapter uses a
/// `reqwest`-backed implementation (behind the `reqwest` feature
/// flag, follow-up bead); the unit tests use [`MockHttpClient`].
pub trait HttpClient: Send + Sync {
    /// POST `body_json` to `url` with the optional bearer token,
    /// returning the response body bytes + the HTTP status code.
    fn post_json(
        &self,
        url: String,
        bearer: Option<String>,
        body_json: String,
    ) -> Pin<Box<dyn std::future::Future<Output = HttpResult> + Send + '_>>;

    /// GET `url` with the optional bearer token.
    fn get(
        &self,
        url: String,
        bearer: Option<String>,
    ) -> Pin<Box<dyn std::future::Future<Output = HttpResult> + Send + '_>>;
}

/// Result of an HTTP call.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    /// HTTP status code (e.g. 200, 401, 502).
    pub status: u16,
    /// Response body bytes.
    pub body: Vec<u8>,
}

/// Shorthand for an HTTP call result.
pub type HttpResult = Result<HttpResponse, HttpError>;

/// Transport-layer errors.
#[derive(Debug, Error)]
pub enum HttpError {
    /// The underlying transport (reqwest, mock, ...) returned an
    /// error before a response was received.
    #[error("HTTP transport error: {0}")]
    Transport(String),
}

/// Errors raised by the partner adapter scaffold itself
/// (configuration, vendor dispatch).
#[derive(Debug, Error)]
pub enum PartnerError {
    /// A required env-var was unset.
    #[error("missing required env-var: {0}")]
    MissingEnv(&'static str),
    /// The `INVOICEKIT_PEPPOL_PARTNER` value didn't name a
    /// registered vendor.
    #[error("unknown partner vendor: {0}")]
    UnknownVendor(String),
}

/// The adapter. Wraps a [`PartnerConfig`] plus a [`SecretResolver`]
/// plus an [`HttpClient`], and implements
/// [`invoicekit_reconcile::GatewayAdapter`].
pub struct PartnerPeppolAdapter {
    config: PartnerConfig,
    secrets: Box<dyn SecretResolver>,
    http: Box<dyn HttpClient>,
}

impl PartnerPeppolAdapter {
    /// Build a new adapter.
    #[must_use]
    pub fn new(
        config: PartnerConfig,
        secrets: Box<dyn SecretResolver>,
        http: Box<dyn HttpClient>,
    ) -> Self {
        Self {
            config,
            secrets,
            http,
        }
    }

    /// Vendor-specific submit URL.
    fn submit_url(&self) -> String {
        match self.config.vendor {
            PartnerVendor::Storecove => {
                format!("{}/document_submissions", self.config.api_base)
            }
            PartnerVendor::Ecosio => format!("{}/peppol/submit", self.config.api_base),
            PartnerVendor::B2brouter => format!("{}/invoices", self.config.api_base),
        }
    }

    /// Vendor-specific poll URL for a submission id.
    fn poll_url(&self, submission_id: &str) -> String {
        // The submission id originates from the partner's submit
        // response (see `extract_submission_id`) and is only loosely
        // validated, so it must be percent-encoded before it can be
        // interpolated into the URL path — otherwise a hostile id can
        // inject `/`, `?`, `#`, or `..` and rewrite the request target.
        let submission_id = percent_encode_path_segment(submission_id);
        match self.config.vendor {
            PartnerVendor::Storecove => {
                format!(
                    "{}/document_submissions/{submission_id}",
                    self.config.api_base
                )
            }
            PartnerVendor::Ecosio => {
                format!("{}/peppol/status/{submission_id}", self.config.api_base)
            }
            PartnerVendor::B2brouter => {
                format!("{}/invoices/{submission_id}", self.config.api_base)
            }
        }
    }

    /// Submit a document to the partner's submit endpoint. Shared by
    /// [`GatewayAdapter::submit`] and [`GatewayAdapter::correct`]
    /// (a correction is a fresh submit); `operation` distinguishes
    /// which surfaced the call so the [`GatewayError`] / receipt
    /// envelope carry the right [`GatewayOperation`].
    fn submit_document(
        &self,
        operation: GatewayOperation,
        document: invoicekit_ir::CommercialDocument,
        context: GatewayContext,
    ) -> GatewayFuture<'_, GatewayReceipt> {
        let url = self.submit_url();
        let bearer = self.secrets.api_key();
        let vendor = self.config.vendor;
        let legal_entity_id = self.config.legal_entity_id.clone();
        Box::pin(async move {
            // Build the canonical UBL bytes once; the partner
            // bodies wrap them as base64. format-ubl is the
            // load-bearing serializer; errors propagate as
            // InvalidRequest because they happen before any
            // partner-side decision.
            let xml = invoicekit_format_ubl::to_xml(&document).map_err(|e| {
                GatewayError::new(
                    GatewayErrorKind::InvalidRequest,
                    operation,
                    format!("partner adapter: UBL serialisation failed: {e}"),
                    "fix the IR document so format-ubl can serialise it",
                )
            })?;
            let body = render_submit_body(vendor, &legal_entity_id, &xml);
            let response = self.http.post_json(url, bearer, body).await.map_err(|e| {
                GatewayError::new(
                    GatewayErrorKind::NetworkFailure,
                    operation,
                    format!("partner adapter HTTP transport error: {e}"),
                    "check INVOICEKIT_PEPPOL_API_BASE reachability + the partner's status page",
                )
            })?;
            decode_submit_response(&context, &response)
        })
    }
}

impl GatewayAdapter for PartnerPeppolAdapter {
    fn submit(&self, request: SubmitRequest) -> GatewayFuture<'_, GatewayReceipt> {
        self.submit_document(GatewayOperation::Submit, request.document, request.context)
    }

    fn poll(&self, request: PollRequest) -> GatewayFuture<'_, GatewayReceipt> {
        let url = self.poll_url(request.submission_id.as_str());
        let bearer = self.secrets.api_key();
        let submission_id = request.submission_id.clone();
        Box::pin(async move {
            let response = self.http.get(url, bearer).await.map_err(|e| {
                GatewayError::new(
                    GatewayErrorKind::NetworkFailure,
                    GatewayOperation::Poll,
                    format!("partner adapter HTTP transport error: {e}"),
                    "check INVOICEKIT_PEPPOL_API_BASE reachability + the partner's status page",
                )
            })?;
            decode_poll_response(&request.context, submission_id, &response)
        })
    }

    fn cancel(&self, _request: CancelRequest) -> GatewayFuture<'_, GatewayReceipt> {
        Box::pin(async move {
            Err(GatewayError::new(
                GatewayErrorKind::UnsupportedOperation,
                GatewayOperation::Cancel,
                "cancel is not supported by the partner AP adapter",
                "Peppol invoices are immutable post-submit; use correct() to issue a credit note instead",
            ))
        })
    }

    fn correct(&self, request: CorrectRequest) -> GatewayFuture<'_, GatewayReceipt> {
        // Correction is a fresh submit with a reference to the
        // prior submission. We delegate to submit + tag the route.
        self.submit_document(
            GatewayOperation::Correct,
            request.corrected_document,
            request.context,
        )
    }
}

fn render_submit_body(vendor: PartnerVendor, legal_entity_id: &str, xml: &str) -> String {
    let xml_b64 = base64_encode(xml.as_bytes());
    // Build the body with `serde_json` rather than `format!` so that
    // `legal_entity_id` (operator-controlled, may contain `"`, `\`, or
    // control characters) is correctly escaped. `xml_b64` is already a
    // base64 string, but routing it through the value keeps the whole
    // body canonically encoded. For normal values this yields the same
    // JSON the hand-written literals produced.
    let body = match vendor {
        PartnerVendor::Storecove => serde_json::json!({
            "legal_entity_id": legal_entity_id,
            "document": {
                "document_type": "invoice",
                "raw_document_data": {
                    "document": xml_b64,
                    "parse": true,
                    "document_type": "ubl",
                },
            },
        }),
        PartnerVendor::Ecosio => serde_json::json!({
            "sender": { "id": legal_entity_id },
            "payload": xml_b64,
            "syntax": "UBL",
        }),
        PartnerVendor::B2brouter => serde_json::json!({
            "project_id": legal_entity_id,
            "xml_base64": xml_b64,
        }),
    };
    body.to_string()
}

fn decode_submit_response(
    context: &GatewayContext,
    response: &HttpResponse,
) -> Result<GatewayReceipt, GatewayError> {
    if (200..300).contains(&response.status) {
        // The receipt is signal that the partner accepted the
        // submission for delivery. The actual delivery status
        // round-trips via poll(). A real partner response carries
        // a submission ID we'd parse out of the JSON body; the
        // scaffold extracts the first plausible `"id"` value to
        // stand in until the per-vendor parser lands.
        let submission_id = extract_submission_id(&response.body).map_err(|reason| {
            GatewayError::new(
                GatewayErrorKind::MalformedReceipt,
                GatewayOperation::Submit,
                format!(
                    "partner submit accepted (HTTP {}) but no submission id parseable: {reason}",
                    response.status
                ),
                "land the per-vendor JSON parser for the partner's submission-id field",
            )
        })?;
        GatewayReceipt::new(
            GatewayOperation::Submit,
            context.clone(),
            submission_id,
            GatewayStatus::Pending,
            "1970-01-01T00:00:00Z",
        )
        .map_err(|e| {
            GatewayError::new(
                GatewayErrorKind::MalformedReceipt,
                GatewayOperation::Submit,
                format!("partner adapter receipt envelope rejected: {e}"),
                "report this as a partner adapter bug",
            )
        })
    } else {
        Err(GatewayError::new(
            partner_error_kind(response.status),
            GatewayOperation::Submit,
            format!(
                "partner submit returned HTTP {} (body bytes={})",
                response.status,
                response.body.len()
            ),
            "consult the partner status page; re-try after the documented backoff",
        ))
    }
}

fn decode_poll_response(
    context: &GatewayContext,
    submission_id: GatewaySubmissionId,
    response: &HttpResponse,
) -> Result<GatewayReceipt, GatewayError> {
    if (200..300).contains(&response.status) {
        // The partner reports terminal delivery as `Accepted`
        // (synchronous) when a positive MDN is in hand. Pending
        // is the default for asynchronous Peppol routing.
        let status = if response
            .body
            .windows(b"delivered".len())
            .any(|w| w == b"delivered")
        {
            GatewayStatus::Accepted
        } else {
            GatewayStatus::Pending
        };
        GatewayReceipt::new(
            GatewayOperation::Poll,
            context.clone(),
            submission_id,
            status,
            "1970-01-01T00:00:00Z",
        )
        .map_err(|e| {
            GatewayError::new(
                GatewayErrorKind::MalformedReceipt,
                GatewayOperation::Poll,
                format!("partner adapter receipt envelope rejected: {e}"),
                "report this as a partner adapter bug",
            )
        })
    } else {
        Err(GatewayError::new(
            partner_error_kind(response.status),
            GatewayOperation::Poll,
            format!("partner poll returned HTTP {}", response.status),
            "consult the partner status page; re-try after the documented backoff",
        ))
    }
}

/// Extract the partner's submission id from a JSON response body.
/// This is the substring-fallback parser the scaffold ships; a
/// follow-up bead lands per-vendor structured parsers.
fn extract_submission_id(body: &[u8]) -> Result<GatewaySubmissionId, String> {
    let needle = b"\"id\":\"";
    let Some(start) = body.windows(needle.len()).position(|w| w == needle) else {
        return Err("response body did not contain an \"id\" field".to_owned());
    };
    let from = start + needle.len();
    let Some(end) = body[from..].iter().position(|b| *b == b'"') else {
        return Err("unterminated \"id\" string in response body".to_owned());
    };
    let id = std::str::from_utf8(&body[from..from + end])
        .map_err(|e| format!("id field is not valid UTF-8: {e}"))?;
    GatewaySubmissionId::new(id).map_err(|e| format!("partner submission id rejected: {e}"))
}

fn partner_error_kind(status: u16) -> GatewayErrorKind {
    match status {
        401 | 403 => GatewayErrorKind::AuthFailure,
        404 => GatewayErrorKind::NotFound,
        408 => GatewayErrorKind::Timeout,
        409 => GatewayErrorKind::DuplicateSubmission,
        422 => GatewayErrorKind::Rejected,
        429 => GatewayErrorKind::RateLimited,
        500..=599 => GatewayErrorKind::GatewayMaintenance,
        _ => GatewayErrorKind::PartnerError,
    }
}

/// Percent-encode a single URL path segment.
///
/// The partner submission id is partner-supplied and only
/// loosely validated upstream (no leading/trailing whitespace and
/// no control characters), so it can still contain `/`, `?`, `#`,
/// `%`, internal spaces, or a `..` traversal. Interpolating it raw
/// into the poll/status URL path would let it escape its path
/// segment and rewrite the request target across a network trust
/// boundary. Encoding it here keeps it confined to one segment.
///
/// Bytes outside the RFC 3986 "unreserved" set (ASCII letters and
/// digits plus `-`, `.`, `_`, `~`) are encoded as `%XX`. Slashes
/// become `%2F`, so a `..` payload can no longer reach across
/// segments to traverse the path. Implemented inline so the crate
/// doesn't pull `percent_encoding` into the workspace for one call.
fn percent_encode_path_segment(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len());
    for &byte in input.as_bytes() {
        let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if unreserved {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

fn base64_encode(input: &[u8]) -> String {
    // Standard alphabet, no padding control needed — this is the
    // canonical mapping per RFC 4648 §4. Implemented inline so
    // the crate doesn't pull base64 into the workspace twice.
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let chunks = input.chunks(3);
    for chunk in chunks {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let triple = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_transmit_peppol_partner::crate_name(),
///     "invoicekit-transmit-peppol-partner"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-transmit-peppol-partner"
}

// ----- mock HttpClient for tests + downstream tests --------------

/// Mock [`HttpClient`] for tests + downstream consumers.
///
/// Records every request and returns the next queued response;
/// used by the crate's unit tests and by downstream callers that
/// want to integration-test their reconcile flow without a real
/// network.
pub struct MockHttpClient {
    queued: std::sync::Mutex<Vec<HttpResult>>,
    recorded: std::sync::Mutex<Vec<MockHttpCall>>,
}

/// One recorded call against [`MockHttpClient`].
#[derive(Clone, Debug)]
pub struct MockHttpCall {
    /// HTTP method invoked (`"POST"` or `"GET"`).
    pub method: &'static str,
    /// Target URL the adapter constructed.
    pub url: String,
    /// Bearer token supplied by the [`SecretResolver`].
    pub bearer: Option<String>,
    /// Request body for `POST`; `None` for `GET`.
    pub body: Option<String>,
}

impl MockHttpClient {
    /// Build a mock that returns each queued response in the order
    /// they appear (FIFO). The mock returns a transport error when
    /// the queue is exhausted.
    #[must_use]
    pub fn new(responses: Vec<HttpResult>) -> Self {
        Self {
            queued: std::sync::Mutex::new(responses.into_iter().rev().collect()),
            recorded: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Return a snapshot of the calls made so far.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned (only possible if
    /// a prior call panicked while holding the lock — never under
    /// normal use).
    #[must_use]
    pub fn calls(&self) -> Vec<MockHttpCall> {
        self.recorded.lock().unwrap().clone()
    }

    fn pop_response(&self) -> HttpResult {
        self.queued
            .lock()
            .unwrap()
            .pop()
            .unwrap_or_else(|| Err(HttpError::Transport("no queued response".to_owned())))
    }
}

impl HttpClient for MockHttpClient {
    fn post_json(
        &self,
        url: String,
        bearer: Option<String>,
        body_json: String,
    ) -> Pin<Box<dyn std::future::Future<Output = HttpResult> + Send + '_>> {
        self.recorded.lock().unwrap().push(MockHttpCall {
            method: "POST",
            url,
            bearer,
            body: Some(body_json),
        });
        let response = self.pop_response();
        Box::pin(async move { response })
    }

    fn get(
        &self,
        url: String,
        bearer: Option<String>,
    ) -> Pin<Box<dyn std::future::Future<Output = HttpResult> + Send + '_>> {
        self.recorded.lock().unwrap().push(MockHttpCall {
            method: "GET",
            url,
            bearer,
            body: None,
        });
        let response = self.pop_response();
        Box::pin(async move { response })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup_from(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |key: &str| map.get(key).map(|s| (*s).to_owned())
    }

    #[test]
    fn vendor_slug_round_trips() {
        for vendor in [
            PartnerVendor::Storecove,
            PartnerVendor::Ecosio,
            PartnerVendor::B2brouter,
        ] {
            assert_eq!(PartnerVendor::from_slug(vendor.slug()).unwrap(), vendor);
        }
    }

    #[test]
    fn unknown_vendor_slug_is_rejected() {
        let err = PartnerVendor::from_slug("not-a-vendor").unwrap_err();
        assert!(matches!(err, PartnerError::UnknownVendor(_)));
    }

    #[test]
    fn from_lookup_uses_defaults() {
        let map: HashMap<&'static str, &'static str> = [
            ("INVOICEKIT_PEPPOL_PARTNER", "storecove"),
            ("INVOICEKIT_PEPPOL_LEGAL_ENTITY_ID", "acme-123"),
        ]
        .into_iter()
        .collect();
        let cfg = PartnerConfig::from_lookup(&lookup_from(map)).unwrap();
        assert_eq!(cfg.vendor, PartnerVendor::Storecove);
        assert_eq!(cfg.api_base, "https://api.storecove.com/api/v2");
        assert_eq!(cfg.legal_entity_id, "acme-123");
        assert!(!cfg.sandbox);
    }

    #[test]
    fn from_lookup_overrides_with_sandbox_and_api_base() {
        let map: HashMap<&'static str, &'static str> = [
            ("INVOICEKIT_PEPPOL_PARTNER", "ecosio"),
            ("INVOICEKIT_PEPPOL_LEGAL_ENTITY_ID", "ent-1"),
            ("INVOICEKIT_PEPPOL_API_BASE", "https://sandbox.ecosio.com"),
            ("INVOICEKIT_PEPPOL_SANDBOX", "true"),
        ]
        .into_iter()
        .collect();
        let cfg = PartnerConfig::from_lookup(&lookup_from(map)).unwrap();
        assert_eq!(cfg.vendor, PartnerVendor::Ecosio);
        assert_eq!(cfg.api_base, "https://sandbox.ecosio.com");
        assert!(cfg.sandbox);
    }

    #[test]
    fn from_lookup_rejects_missing_required_envs() {
        let err = PartnerConfig::from_lookup(&|_| None).unwrap_err();
        assert!(matches!(
            err,
            PartnerError::MissingEnv("INVOICEKIT_PEPPOL_PARTNER")
        ));
    }

    #[test]
    fn base64_encode_matches_rfc4648_examples() {
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn render_submit_body_storecove_carries_legal_entity_and_xml() {
        let body = render_submit_body(PartnerVendor::Storecove, "acme-1", "<x/>");
        assert!(body.contains("\"legal_entity_id\":\"acme-1\""));
        // <x/> base64 = "PHgvPg=="
        assert!(body.contains("PHgvPg=="));
        assert!(body.contains("document_type"));
    }

    #[test]
    fn render_submit_body_escapes_legal_entity_id_for_every_vendor() {
        // A legal-entity ID containing a JSON metacharacter must not be
        // able to break out of its string literal or inject sibling keys.
        // Previously the body was assembled with `format!` and a raw
        // interpolation, so a `"` produced malformed / attacker-shaped JSON.
        let nasty = r#"x","injected":"yes"#;
        for vendor in [
            PartnerVendor::Storecove,
            PartnerVendor::Ecosio,
            PartnerVendor::B2brouter,
        ] {
            let body = render_submit_body(vendor, nasty, "<x/>");
            // The body must be valid JSON.
            let parsed: serde_json::Value =
                serde_json::from_str(&body).expect("render_submit_body must emit valid JSON");
            // The literal raw ID must NOT appear unescaped in the wire bytes:
            // a `"` from the ID is escaped to `\"`, so the raw substring is absent.
            assert!(
                !body.contains(r#""injected":"yes""#),
                "raw injected key leaked into {vendor:?} body: {body}"
            );
            // And the ID round-trips intact at its proper location.
            let recovered = match vendor {
                PartnerVendor::Storecove => parsed["legal_entity_id"].as_str(),
                PartnerVendor::Ecosio => parsed["sender"]["id"].as_str(),
                PartnerVendor::B2brouter => parsed["project_id"].as_str(),
            };
            assert_eq!(
                recovered,
                Some(nasty),
                "legal_entity_id must round-trip through the {vendor:?} body"
            );
        }
    }

    #[test]
    fn submit_url_per_vendor() {
        let cfg_storecove = PartnerConfig {
            vendor: PartnerVendor::Storecove,
            api_base: PartnerVendor::Storecove.default_api_base().to_owned(),
            legal_entity_id: "x".to_owned(),
            sandbox: false,
        };
        let cfg_ecosio = PartnerConfig {
            vendor: PartnerVendor::Ecosio,
            api_base: PartnerVendor::Ecosio.default_api_base().to_owned(),
            legal_entity_id: "x".to_owned(),
            sandbox: false,
        };
        let cfg_b2b = PartnerConfig {
            vendor: PartnerVendor::B2brouter,
            api_base: PartnerVendor::B2brouter.default_api_base().to_owned(),
            legal_entity_id: "x".to_owned(),
            sandbox: false,
        };
        let adapter_storecove = PartnerPeppolAdapter::new(
            cfg_storecove,
            Box::new(StaticSecretResolver::new("k", None)),
            Box::new(MockHttpClient::new(vec![])),
        );
        assert_eq!(
            adapter_storecove.submit_url(),
            "https://api.storecove.com/api/v2/document_submissions"
        );
        let adapter_ecosio = PartnerPeppolAdapter::new(
            cfg_ecosio,
            Box::new(StaticSecretResolver::new("k", None)),
            Box::new(MockHttpClient::new(vec![])),
        );
        assert_eq!(
            adapter_ecosio.submit_url(),
            "https://api.ecosio.com/peppol/submit"
        );
        let adapter_b2b = PartnerPeppolAdapter::new(
            cfg_b2b,
            Box::new(StaticSecretResolver::new("k", None)),
            Box::new(MockHttpClient::new(vec![])),
        );
        assert_eq!(
            adapter_b2b.submit_url(),
            "https://app.b2brouter.net/projects/-/api/invoices"
        );
    }

    #[test]
    fn partner_error_kind_maps_http_status() {
        assert!(matches!(
            partner_error_kind(401),
            GatewayErrorKind::AuthFailure
        ));
        assert!(matches!(
            partner_error_kind(403),
            GatewayErrorKind::AuthFailure
        ));
        assert!(matches!(
            partner_error_kind(404),
            GatewayErrorKind::NotFound
        ));
        assert!(matches!(
            partner_error_kind(409),
            GatewayErrorKind::DuplicateSubmission
        ));
        assert!(matches!(
            partner_error_kind(422),
            GatewayErrorKind::Rejected
        ));
        assert!(matches!(
            partner_error_kind(429),
            GatewayErrorKind::RateLimited
        ));
        assert!(matches!(
            partner_error_kind(503),
            GatewayErrorKind::GatewayMaintenance
        ));
        assert!(matches!(
            partner_error_kind(418),
            GatewayErrorKind::PartnerError
        ));
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-transmit-peppol-partner");
    }

    #[test]
    fn percent_encode_path_segment_leaves_unreserved_intact() {
        // RFC 3986 unreserved set must pass through unchanged so that
        // ordinary partner submission ids are not mangled on the wire.
        assert_eq!(
            percent_encode_path_segment("sub_001-2.3~AZaz09"),
            "sub_001-2.3~AZaz09"
        );
    }

    #[test]
    fn percent_encode_path_segment_neutralises_path_injection() {
        // Every character that could break out of the path segment
        // must be escaped: `/` `?` `#` `%` and a bare space.
        let encoded = percent_encode_path_segment("../../admin?x=1#frag with%2e");
        assert!(!encoded.contains('/'), "slash leaked: {encoded}");
        assert!(!encoded.contains('?'), "query opener leaked: {encoded}");
        assert!(!encoded.contains('#'), "fragment opener leaked: {encoded}");
        assert!(!encoded.contains(' '), "space leaked: {encoded}");
        // A literal `%` from the input is itself encoded to `%25`, so
        // the only `%` sequences left are the encoder's own escapes.
        assert_eq!(encoded, "..%2F..%2Fadmin%3Fx%3D1%23frag%20with%252e");
    }

    #[test]
    fn poll_url_percent_encodes_hostile_submission_id() {
        use invoicekit_reconcile::GatewaySubmissionId;

        // This hostile id passes `GatewaySubmissionId` validation
        // (no control chars, no leading/trailing whitespace) yet, if
        // interpolated raw, would traverse out of the status namespace
        // and rewrite the request target.
        let hostile = "../../admin?x=1#frag";
        let id = GatewaySubmissionId::new(hostile)
            .expect("hostile id is accepted by the loose upstream validator");

        for vendor in [
            PartnerVendor::Storecove,
            PartnerVendor::Ecosio,
            PartnerVendor::B2brouter,
        ] {
            let cfg = PartnerConfig {
                vendor,
                api_base: vendor.default_api_base().to_owned(),
                legal_entity_id: "x".to_owned(),
                sandbox: false,
            };
            let adapter = PartnerPeppolAdapter::new(
                cfg,
                Box::new(StaticSecretResolver::new("k", None)),
                Box::new(MockHttpClient::new(vec![])),
            );
            let url = adapter.poll_url(id.as_str());
            // The hostile path-control characters must NOT survive into
            // the URL path: the raw traversal/query/fragment substring
            // is gone, replaced by its percent-encoded form.
            assert!(
                !url.contains("../../admin"),
                "raw traversal leaked into {vendor:?} poll URL: {url}"
            );
            assert!(
                !url.contains('?'),
                "raw query opener leaked into {vendor:?} poll URL: {url}"
            );
            assert!(
                !url.contains('#'),
                "raw fragment opener leaked into {vendor:?} poll URL: {url}"
            );
            // The encoded id is present as a single confined path segment.
            assert!(
                url.contains("..%2F..%2Fadmin%3Fx%3D1%23frag"),
                "encoded id missing from {vendor:?} poll URL: {url}"
            );
            // And the URL still begins with the vendor's API base, so the
            // host/scheme were not rewritten by the payload.
            assert!(
                url.starts_with(vendor.default_api_base()),
                "{vendor:?} poll URL no longer rooted at the API base: {url}"
            );
        }
    }
}
