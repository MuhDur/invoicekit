// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! InvoiceKit intake **VLM** substrate (Layer 5 — Qwen2.5-VL-7B).
//!
//! The vision-language-model layer of the intake pipeline.
//! Layer 5 is the most expensive layer per PLAN.md §3.5;
//! the engine routes to it only when Layers 1, 2, 3, and 4
//! (digital PDF, Factur-X, PaddleOCR, SmolDocling) failed
//! to reach acceptable confidence.
//!
//! Today's substrate captures the typed surface so engine
//! wiring is stable. Live cloud inference lands in a
//! follow-up `intake-vlm-http` crate that talks to the
//! Qwen2.5-VL-7B endpoint at the operator's chosen
//! provider (Together / Replicate / self-hosted).

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Which VLM model the engine targets.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VlmModel {
    /// Qwen2.5-VL-7B (the L5 default).
    Qwen25Vl7b,
    /// Mock — test stub.
    Mock,
}

/// One typed extraction the VLM emits per invoice field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VlmField {
    /// EN16931 BT/BG term id (e.g. `BT-1`, `BG-7`).
    pub term: String,
    /// Extracted value as a UTF-8 string. Numeric fields are
    /// strings to preserve the issuer's formatting.
    pub value: String,
    /// Model self-reported confidence in [0.0, 1.0].
    pub confidence: f32,
}

/// Aggregate VLM extraction result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VlmResult {
    /// Model that produced the result.
    pub model: VlmModel,
    /// Extracted fields in document order.
    pub fields: Vec<VlmField>,
    /// Mean field confidence (0.0 when no fields).
    pub mean_confidence: f32,
    /// Tokens billed by the provider (live impl populates;
    /// mock returns 0).
    pub billed_tokens: u64,
}

/// Typed errors raised by [`VlmProvider`] implementations.
#[derive(Debug, Error)]
pub enum VlmError {
    /// Source bytes were not a parseable PDF/image.
    #[error("source bytes rejected: {0}")]
    BadSource(String),
    /// Cloud inference provider refused / timed out.
    #[error("provider failure: {0}")]
    Provider(String),
    /// Rate-limited.
    #[error("rate limited; retry-after {0}s")]
    RateLimited(u32),
}

/// VLM extraction surface.
pub trait VlmProvider: Send + Sync {
    /// Which model this provider implements.
    fn model(&self) -> VlmModel;

    /// Extract typed invoice fields from `source_bytes`
    /// (PDF/PNG/JPEG).
    ///
    /// # Errors
    ///
    /// Returns [`VlmError::BadSource`] on parse failure,
    /// [`VlmError::Provider`] on cloud-provider failure, or
    /// [`VlmError::RateLimited`] when the operator hits
    /// their per-second quota.
    fn extract(&self, source_bytes: &[u8]) -> Result<VlmResult, VlmError>;
}

/// Live-bound Qwen2.5-VL-7B provider scaffold.
///
/// The live impl POSTs to `endpoint_url` with a
/// base64-encoded image and a structured-output prompt that
/// asks for the EN16931 BT/BG fields. Today's stub returns
/// a fixed three-field result so engine wiring stays
/// exercisable.
pub struct Qwen25Vl7bProvider {
    /// HTTPS endpoint the live impl POSTs to.
    pub endpoint_url: String,
    /// Operator's per-tenant API key (kept opaque).
    pub api_key_ref: String,
}

impl VlmProvider for Qwen25Vl7bProvider {
    fn model(&self) -> VlmModel {
        VlmModel::Qwen25Vl7b
    }
    fn extract(&self, source_bytes: &[u8]) -> Result<VlmResult, VlmError> {
        if source_bytes.is_empty() {
            return Err(VlmError::BadSource("source is empty".to_owned()));
        }
        if self.endpoint_url.is_empty() {
            return Err(VlmError::Provider("endpoint_url is empty".to_owned()));
        }
        if self.api_key_ref.is_empty() {
            return Err(VlmError::Provider("api_key_ref is empty".to_owned()));
        }
        Ok(stub_extract(VlmModel::Qwen25Vl7b))
    }
}

/// Deterministic mock provider.
pub struct MockVlmProvider;

impl VlmProvider for MockVlmProvider {
    fn model(&self) -> VlmModel {
        VlmModel::Mock
    }
    fn extract(&self, source_bytes: &[u8]) -> Result<VlmResult, VlmError> {
        if source_bytes.is_empty() {
            return Err(VlmError::BadSource("source is empty".to_owned()));
        }
        Ok(stub_extract(VlmModel::Mock))
    }
}

fn stub_extract(model: VlmModel) -> VlmResult {
    let fields = vec![
        VlmField {
            term: "BT-1".to_owned(),
            value: "INV-MOCK-1".to_owned(),
            confidence: 0.95,
        },
        VlmField {
            term: "BT-2".to_owned(),
            value: "2026-05-28".to_owned(),
            confidence: 0.92,
        },
        VlmField {
            term: "BT-5".to_owned(),
            value: "EUR".to_owned(),
            confidence: 0.99,
        },
    ];
    let count = u32::try_from(fields.len()).unwrap_or(1).max(1);
    #[allow(clippy::cast_precision_loss)]
    let mean = fields.iter().map(|f| f.confidence).sum::<f32>() / (count as f32);
    VlmResult {
        model,
        fields,
        mean_confidence: mean,
        billed_tokens: 0,
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_intake_vlm::crate_name(), "invoicekit-intake-vlm");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-intake-vlm"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_provider_returns_three_fields() {
        let r = MockVlmProvider.extract(b"%PDF-1.4").unwrap();
        assert_eq!(r.model, VlmModel::Mock);
        assert_eq!(r.fields.len(), 3);
        assert!(r.mean_confidence > 0.9);
    }

    #[test]
    fn mock_provider_rejects_empty_source() {
        let err = MockVlmProvider.extract(b"").unwrap_err();
        assert!(matches!(err, VlmError::BadSource(_)));
    }

    #[test]
    fn qwen_provider_returns_stub_for_well_formed_config() {
        let p = Qwen25Vl7bProvider {
            endpoint_url: "https://api.together.xyz/v1/chat/completions".to_owned(),
            api_key_ref: "secret-ref:tenant-1".to_owned(),
        };
        let r = p.extract(b"%PDF-1.4").unwrap();
        assert_eq!(r.model, VlmModel::Qwen25Vl7b);
        assert_eq!(r.fields.len(), 3);
    }

    #[test]
    fn qwen_provider_rejects_empty_endpoint() {
        let p = Qwen25Vl7bProvider {
            endpoint_url: String::new(),
            api_key_ref: "secret-ref:tenant-1".to_owned(),
        };
        let err = p.extract(b"%PDF-1.4").unwrap_err();
        assert!(matches!(err, VlmError::Provider(_)));
    }

    #[test]
    fn qwen_provider_rejects_empty_api_key_ref() {
        let p = Qwen25Vl7bProvider {
            endpoint_url: "https://api.together.xyz".to_owned(),
            api_key_ref: String::new(),
        };
        let err = p.extract(b"%PDF-1.4").unwrap_err();
        assert!(matches!(err, VlmError::Provider(_)));
    }

    #[test]
    fn qwen_provider_rejects_empty_source() {
        let p = Qwen25Vl7bProvider {
            endpoint_url: "https://api.together.xyz".to_owned(),
            api_key_ref: "x".to_owned(),
        };
        let err = p.extract(b"").unwrap_err();
        assert!(matches!(err, VlmError::BadSource(_)));
    }

    #[test]
    fn vlm_result_round_trips_through_serde() {
        let r = VlmResult {
            model: VlmModel::Qwen25Vl7b,
            fields: vec![VlmField {
                term: "BT-1".to_owned(),
                value: "X".to_owned(),
                confidence: 1.0,
            }],
            mean_confidence: 1.0,
            billed_tokens: 42,
        };
        let json = serde_json::to_string(&r).unwrap();
        let parsed: VlmResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn vlm_error_rate_limited_carries_retry_after() {
        let err = VlmError::RateLimited(30);
        assert!(err.to_string().contains("30"));
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-intake-vlm");
    }
}
