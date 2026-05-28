// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! InvoiceKit intake **OCR** substrate.
//!
//! Typed surface for the document-OCR layer of the intake
//! pipeline. The full intake stack ships in layers
//! per PLAN.md §3.5:
//!
//! * Layer 1 — digital PDF text extraction (lives in
//!   `crates/intake-pdf`).
//! * Layer 2 — Factur-X embedded XML extraction (lives in
//!   `crates/intake-pdf`).
//! * **Layer 3 — server-side PaddleOCR** (this crate's
//!   `PaddleOcrProvider`).
//! * **Layer 4 — SmolDocling-256M ONNX** (this crate's
//!   `SmolDoclingProvider`).
//! * Layer 5 — Qwen2.5-VL cloud inference (lives in
//!   `crates/intake-vlm`).
//!
//! Every layer satisfies the same [`OcrProvider`] trait so
//! the engine's intake pipeline picks the cheapest layer
//! that returns acceptable confidence.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Bounding box of a recognised text span (PDF points).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    /// Page index (0-based).
    pub page: u32,
    /// X coordinate of the top-left corner (PDF points).
    pub x: f32,
    /// Y coordinate of the top-left corner (PDF points).
    pub y: f32,
    /// Box width (PDF points).
    pub width: f32,
    /// Box height (PDF points).
    pub height: f32,
}

/// One OCR text token + its bounding box + recogniser
/// confidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OcrToken {
    /// Recognised UTF-8 text.
    pub text: String,
    /// Bounding box on the source page.
    pub bbox: BoundingBox,
    /// Recogniser confidence in [0.0, 1.0].
    pub confidence: f32,
}

/// Which OCR layer produced the result.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OcrLayer {
    /// PaddleOCR — server-side default.
    PaddleOcr,
    /// SmolDocling — 256M ONNX small-VLM hybrid.
    SmolDocling,
    /// Mock — deterministic test stub.
    Mock,
}

/// Aggregate result of an OCR pass over one document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OcrResult {
    /// Layer that produced the result.
    pub layer: OcrLayer,
    /// Tokens in reading order.
    pub tokens: Vec<OcrToken>,
    /// Page count of the source document.
    pub page_count: u32,
    /// Mean token confidence (0.0 when no tokens).
    pub mean_confidence: f32,
}

/// Typed errors raised by [`OcrProvider`] implementations.
#[derive(Debug, Error)]
pub enum OcrError {
    /// Source bytes were not a parseable PDF/image.
    #[error("source bytes rejected: {0}")]
    BadSource(String),
    /// ONNX runtime / Paddle backend failed to load.
    #[error("backend failure: {0}")]
    Backend(String),
}

/// OCR layer surface. Implementations:
///
/// * [`PaddleOcrProvider`] — Layer 3.
/// * [`SmolDoclingProvider`] — Layer 4.
/// * [`MockOcrProvider`] — test/cassette baseline.
pub trait OcrProvider: Send + Sync {
    /// Which layer this provider implements.
    fn layer(&self) -> OcrLayer;

    /// Run OCR over `source_bytes` and return a typed
    /// [`OcrResult`].
    ///
    /// # Errors
    ///
    /// Returns [`OcrError::BadSource`] when the bytes aren't
    /// a parseable PDF/image, or [`OcrError::Backend`] when
    /// the underlying runtime fails to load / run.
    fn recognise(&self, source_bytes: &[u8]) -> Result<OcrResult, OcrError>;
}

/// Layer 3 — PaddleOCR server-side default. The live impl
/// shells out to a sidecar (T-062, in-progress); this
/// substrate captures the typed surface so engine wiring is
/// stable.
pub struct PaddleOcrProvider {
    /// Sidecar URL the live impl will POST to.
    pub sidecar_url: String,
}

impl OcrProvider for PaddleOcrProvider {
    fn layer(&self) -> OcrLayer {
        OcrLayer::PaddleOcr
    }
    fn recognise(&self, source_bytes: &[u8]) -> Result<OcrResult, OcrError> {
        if source_bytes.is_empty() {
            return Err(OcrError::BadSource("source is empty".to_owned()));
        }
        // Live impl POSTs to `sidecar_url`; stub returns a
        // single token so substrate users can exercise the
        // contract.
        Ok(stub_result(OcrLayer::PaddleOcr, source_bytes))
    }
}

/// Layer 4 — SmolDocling-256M ONNX runtime. Same shape as
/// Paddle but with a smaller, faster model suitable for
/// edge deployments.
pub struct SmolDoclingProvider {
    /// Path to the ONNX model file (loaded once at engine
    /// startup by the live impl).
    pub model_path: String,
}

impl OcrProvider for SmolDoclingProvider {
    fn layer(&self) -> OcrLayer {
        OcrLayer::SmolDocling
    }
    fn recognise(&self, source_bytes: &[u8]) -> Result<OcrResult, OcrError> {
        if source_bytes.is_empty() {
            return Err(OcrError::BadSource("source is empty".to_owned()));
        }
        if self.model_path.is_empty() {
            return Err(OcrError::Backend("model_path is empty".to_owned()));
        }
        Ok(stub_result(OcrLayer::SmolDocling, source_bytes))
    }
}

/// Deterministic mock. Returns a fixed `INV-MOCK-1` token at
/// the top-left of page 0 so the rest of the intake pipeline
/// can be exercised without spinning up a real OCR runtime.
pub struct MockOcrProvider;

impl OcrProvider for MockOcrProvider {
    fn layer(&self) -> OcrLayer {
        OcrLayer::Mock
    }
    fn recognise(&self, source_bytes: &[u8]) -> Result<OcrResult, OcrError> {
        if source_bytes.is_empty() {
            return Err(OcrError::BadSource("source is empty".to_owned()));
        }
        Ok(OcrResult {
            layer: OcrLayer::Mock,
            tokens: vec![OcrToken {
                text: "INV-MOCK-1".to_owned(),
                bbox: BoundingBox {
                    page: 0,
                    x: 10.0,
                    y: 10.0,
                    width: 50.0,
                    height: 12.0,
                },
                confidence: 1.0,
            }],
            page_count: 1,
            mean_confidence: 1.0,
        })
    }
}

fn stub_result(layer: OcrLayer, source_bytes: &[u8]) -> OcrResult {
    OcrResult {
        layer,
        tokens: vec![OcrToken {
            text: format!("STUB-{}-len-{}", layer_slug(layer), source_bytes.len()),
            bbox: BoundingBox {
                page: 0,
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 12.0,
            },
            confidence: 0.9,
        }],
        page_count: 1,
        mean_confidence: 0.9,
    }
}

const fn layer_slug(layer: OcrLayer) -> &'static str {
    match layer {
        OcrLayer::PaddleOcr => "paddle",
        OcrLayer::SmolDocling => "smoldocling",
        OcrLayer::Mock => "mock",
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_intake_ocr::crate_name(), "invoicekit-intake-ocr");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-intake-ocr"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_provider_returns_single_token() {
        let r = MockOcrProvider.recognise(b"%PDF-1.4").unwrap();
        assert_eq!(r.layer, OcrLayer::Mock);
        assert_eq!(r.tokens.len(), 1);
        assert_eq!(r.tokens[0].text, "INV-MOCK-1");
        assert!((r.mean_confidence - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn mock_provider_rejects_empty_source() {
        let err = MockOcrProvider.recognise(b"").unwrap_err();
        assert!(matches!(err, OcrError::BadSource(_)));
    }

    #[test]
    fn paddle_provider_returns_stub_token() {
        let p = PaddleOcrProvider {
            sidecar_url: "http://localhost:7001".to_owned(),
        };
        let r = p.recognise(b"%PDF-1.4").unwrap();
        assert_eq!(r.layer, OcrLayer::PaddleOcr);
        assert!(r.tokens[0].text.contains("STUB-paddle"));
    }

    #[test]
    fn smoldocling_provider_rejects_empty_model_path() {
        let p = SmolDoclingProvider {
            model_path: String::new(),
        };
        let err = p.recognise(b"%PDF-1.4").unwrap_err();
        assert!(matches!(err, OcrError::Backend(_)));
    }

    #[test]
    fn smoldocling_provider_returns_stub_token_with_model_path() {
        let p = SmolDoclingProvider {
            model_path: "/var/lib/invoicekit/smoldocling.onnx".to_owned(),
        };
        let r = p.recognise(b"%PDF-1.4").unwrap();
        assert_eq!(r.layer, OcrLayer::SmolDocling);
        assert!(r.tokens[0].text.contains("STUB-smoldocling"));
    }

    #[test]
    fn ocr_result_round_trips_through_serde() {
        let r = OcrResult {
            layer: OcrLayer::Mock,
            tokens: vec![OcrToken {
                text: "x".to_owned(),
                bbox: BoundingBox {
                    page: 0,
                    x: 1.0,
                    y: 2.0,
                    width: 3.0,
                    height: 4.0,
                },
                confidence: 0.5,
            }],
            page_count: 1,
            mean_confidence: 0.5,
        };
        let json = serde_json::to_string(&r).unwrap();
        let parsed: OcrResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn ocr_layer_serde_round_trips_all_variants() {
        for l in [OcrLayer::PaddleOcr, OcrLayer::SmolDocling, OcrLayer::Mock] {
            let json = serde_json::to_string(&l).unwrap();
            let parsed: OcrLayer = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, l);
        }
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-intake-ocr");
    }
}
