// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-intake-citation` — bounding-box citation
//! taxonomy.
//!
//! Schema every InvoiceKit intake layer (digital PDF text
//! extraction, server-side OCR, vision-language model) emits
//! alongside the extracted [`CommercialDocument`] field
//! values. The audit UI uses citations to highlight the exact
//! source region for any extracted field; the evidence bundle
//! archives the citations next to the canonical document so
//! the highlight survives replay.
//!
//! [`CommercialDocument`]: https://docs.rs/invoicekit-ir
//!
//! # Layered intake
//!
//! InvoiceKit's intake pipeline runs three layers in order
//! (PLAN.md §3.4 / §4.4):
//!
//! 1. **Digital-PDF text** — extract from a PDF's embedded
//!    text layer. Bounding boxes come straight from the PDF's
//!    content stream. Zero OCR, zero ML.
//! 2. **Server-side OCR** — `PaddleOCR` (Layer 3, T-062) for
//!    scanned PDFs. Bounding boxes come from the OCR
//!    engine's word-box output.
//! 3. **Vision-language model** — `SmolDocling`-256M ONNX
//!    (Layer 4, T-063) for low-confidence regions left over
//!    after Layers 1+2. Bounding boxes come from the VLM's
//!    attention map.
//!
//! Each layer emits citations with the same [`BoundingBoxCitation`]
//! shape but a different [`ExtractionLayer`] tag, so the
//! audit UI can colour-code provenance.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Page rectangle in PDF user-space coordinates.
///
/// PDF user-space is origin-bottom-left, in points (1/72 inch).
/// Coordinates are positive floats; the citation is content
/// even if the page is rotated, because rotation belongs in
/// the page's `/Rotate` field, not the bounding box.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    /// 1-indexed page number (PDF convention; `1` is the
    /// first page).
    pub page: u32,
    /// Left edge in PDF points.
    pub x: f32,
    /// Bottom edge in PDF points.
    pub y: f32,
    /// Width in PDF points.
    pub width: f32,
    /// Height in PDF points.
    pub height: f32,
}

impl BoundingBox {
    /// Right edge in PDF points.
    #[must_use]
    pub fn right(self) -> f32 {
        self.x + self.width
    }
    /// Top edge in PDF points.
    #[must_use]
    pub fn top(self) -> f32 {
        self.y + self.height
    }
    /// True when this box and `other` are on the same page
    /// and overlap by any amount.
    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        self.page == other.page
            && self.x < other.right()
            && other.x < self.right()
            && self.y < other.top()
            && other.y < self.top()
    }
}

/// Intake layer that produced a citation. The audit UI uses
/// this to colour-code provenance and to surface a confidence
/// hint to the operator ("OCR" reads less authoritative than
/// "Digital-PDF text").
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtractionLayer {
    /// Layer 1 — text lifted from a digital PDF's embedded
    /// text layer (no OCR).
    DigitalPdfText,
    /// Layer 2 — text lifted from a hybrid PDF's embedded
    /// XML attachment (Factur-X, ZUGFeRD).
    DigitalPdfXml,
    /// Layer 3 — server-side OCR (default: `PaddleOCR`).
    ServerOcr,
    /// Layer 4 — vision-language model fallback.
    VisionLanguageModel,
    /// Operator-edited override; bounding box comes from a
    /// human who corrected an upstream layer's mistake.
    HumanOverride,
}

impl ExtractionLayer {
    /// Lowercase wire name.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::DigitalPdfText => "digital-pdf-text",
            Self::DigitalPdfXml => "digital-pdf-xml",
            Self::ServerOcr => "server-ocr",
            Self::VisionLanguageModel => "vision-language-model",
            Self::HumanOverride => "human-override",
        }
    }

    /// Default confidence floor associated with this layer.
    /// The audit UI uses this when the extractor didn't
    /// supply a per-citation confidence.
    #[must_use]
    pub fn default_confidence(self) -> Confidence {
        Confidence(match self {
            Self::DigitalPdfText | Self::DigitalPdfXml | Self::HumanOverride => 1.0,
            Self::ServerOcr => 0.75,
            Self::VisionLanguageModel => 0.55,
        })
    }
}

/// Confidence score in `[0.0, 1.0]`. Defensive constructor
/// clamps out-of-range inputs to the valid interval.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Confidence(pub f32);

impl Confidence {
    /// Build a confidence, clamping to `[0.0, 1.0]`.
    #[must_use]
    pub fn new(value: f32) -> Self {
        let clamped = value.clamp(0.0, 1.0);
        Self(if clamped.is_nan() { 0.0 } else { clamped })
    }

    /// Underlying f32 value.
    #[must_use]
    pub const fn value(self) -> f32 {
        self.0
    }
}

/// One bounding-box citation for a single extracted field.
///
/// `path` is the canonical JSON-pointer-style path into the
/// extracted [`CommercialDocument`] (e.g. `/supplier/name`,
/// `/lines/0/unit_price`). The bounding box names the source
/// region the extractor read.
///
/// [`CommercialDocument`]: https://docs.rs/invoicekit-ir
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundingBoxCitation {
    /// JSON-pointer-style path of the extracted field.
    pub path: String,
    /// Source bounding box.
    pub bounding_box: BoundingBox,
    /// Layer that produced this citation.
    pub layer: ExtractionLayer,
    /// Verbatim text the layer read from the source region.
    /// Surfaced in the audit UI so operators can spot OCR
    /// errors without re-running the extractor.
    pub source_text: String,
    /// Confidence in `[0.0, 1.0]`.
    pub confidence: Confidence,
    /// Stable identifier of the extractor that produced this
    /// citation (e.g. `paddle-ocr-v2.7.0`,
    /// `smol-docling-256m-int8`). Lets the audit UI show
    /// "this extracted field came from extractor X version Y"
    /// without re-running.
    pub extractor_id: String,
}

/// All citations a single intake run emitted, indexed by
/// canonical field path.
///
/// One field can have multiple citations when a fall-back
/// layer disagreed with an earlier layer; the `Vec` preserves
/// emission order so the audit UI can show the chain
/// ("digital-pdf-text said X; server-ocr said Y; we kept Y").
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CitationLedger {
    /// Citations grouped by canonical field path. The
    /// `BTreeMap` makes iteration order stable for tests +
    /// deterministic bundle output.
    pub by_path: BTreeMap<String, Vec<BoundingBoxCitation>>,
}

impl CitationLedger {
    /// Build an empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a single citation, appending under its path.
    pub fn record(&mut self, citation: BoundingBoxCitation) {
        self.by_path
            .entry(citation.path.clone())
            .or_default()
            .push(citation);
    }

    /// True when the ledger is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    /// Total citation count across all paths.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_path.values().map(Vec::len).sum()
    }

    /// Iterator over every citation, in stable order.
    pub fn iter(&self) -> impl Iterator<Item = &BoundingBoxCitation> {
        self.by_path.values().flat_map(|v| v.iter())
    }

    /// Citations for a specific field path, in emission order.
    #[must_use]
    pub fn for_path(&self, path: &str) -> &[BoundingBoxCitation] {
        self.by_path.get(path).map_or(&[], Vec::as_slice)
    }

    /// The most-recently-recorded citation for a path, which
    /// is the one the engine committed to (later layers
    /// override earlier ones).
    #[must_use]
    pub fn winner_for(&self, path: &str) -> Option<&BoundingBoxCitation> {
        self.for_path(path).last()
    }

    /// True when at least one citation under `path` came from
    /// a layer with a typed confidence below `floor`. The
    /// review UI uses this to surface low-confidence extractions.
    #[must_use]
    pub fn has_low_confidence(&self, path: &str, floor: Confidence) -> bool {
        self.for_path(path)
            .iter()
            .any(|c| c.confidence.value() < floor.value())
    }
}

/// Errors raised by the schema validators.
#[derive(Debug, Error)]
pub enum CitationError {
    /// Bounding-box page is `0` (PDF pages are 1-indexed).
    #[error("bounding box page must be >= 1; got 0")]
    InvalidPage,
    /// Bounding-box has zero or negative width / height.
    #[error("bounding box has non-positive width or height: {0}x{1}")]
    NonPositiveExtent(f32, f32),
    /// Bounding-box coordinates are NaN or infinite.
    #[error("bounding box coordinate is not finite")]
    NonFiniteCoordinate,
    /// Citation `path` is empty.
    #[error("citation path must be a non-empty JSON-pointer-style string")]
    EmptyPath,
    /// Citation `extractor_id` is empty.
    #[error("citation extractor_id must be non-empty")]
    EmptyExtractorId,
    /// [`FieldCitation::value`] is empty.
    #[error("field citation value must be non-empty")]
    EmptyValue,
    /// [`CitationSource::OcrSpan::span_id`] is empty.
    #[error("ocr span_id must be non-empty")]
    EmptyOcrSpanId,
    /// [`CitationSource::Model::model_id`] is empty.
    #[error("model_id must be non-empty")]
    EmptyModelId,
}

/// Return `err` when `value` is empty, otherwise `Ok(())`.
///
/// Shared guard for the non-empty-string checks the validated
/// constructors run. The error is only yielded on the empty
/// branch, so passing a constructed variant is free (the unit
/// variants carry no payload).
fn require_non_empty(value: &str, err: CitationError) -> Result<(), CitationError> {
    if value.is_empty() {
        Err(err)
    } else {
        Ok(())
    }
}

/// Discriminated taxonomy of source pointers the intake
/// layers attach to an extracted value.
///
/// The audit UI walks one of these per [`FieldCitation`] to
/// reconstruct *exactly* where the value came from:
///
/// * [`CitationSource::PdfObject`] — index into the source
///   PDF's object table. Layer 1 (digital-PDF text) uses this
///   when it lifts a string from a `/Contents` stream.
/// * [`CitationSource::BoundingBox`] — page rectangle. Layers
///   1, 2, 3 use this on top of (or instead of) the PDF object
///   id; Layer 4 (VLM) uses it on its own.
/// * [`CitationSource::OcrSpan`] — span id that points into a
///   prior OCR run's structured output. Layer 3 emits this so
///   a re-run can correlate audit edits with the OCR text.
/// * [`CitationSource::Model`] — stable model id (e.g.
///   `smol-docling-256m-int8`). Layer 4 emits this on every
///   citation; lower layers can attach it as the *extractor*
///   that produced a confidence score.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CitationSource {
    /// Source is a PDF object id in the original document.
    PdfObject {
        /// 0-indexed object number (PDF spec convention).
        object_id: u32,
        /// Optional bounding box co-emitted with the object.
        bounding_box: Option<BoundingBox>,
    },
    /// Source is a page rectangle.
    BoundingBox {
        /// The rectangle.
        bounding_box: BoundingBox,
    },
    /// Source is a span id in a prior OCR run.
    OcrSpan {
        /// Opaque span id from the OCR engine.
        span_id: String,
        /// Optional bounding box co-emitted with the span.
        bounding_box: Option<BoundingBox>,
    },
    /// Source is a VLM model output; the model id is
    /// authoritative when no spatial pointer is available.
    Model {
        /// Stable model id (e.g. `smol-docling-256m-int8`).
        model_id: String,
        /// Optional bounding box from the model's attention map.
        bounding_box: Option<BoundingBox>,
    },
}

impl CitationSource {
    /// Returns the bounding box this source carries, if any.
    /// Convenient for the audit UI's "click to highlight" path
    /// which works against the rectangle regardless of which
    /// taxonomy variant emitted it.
    #[must_use]
    pub fn bounding_box(&self) -> Option<BoundingBox> {
        match self {
            Self::BoundingBox { bounding_box } => Some(*bounding_box),
            Self::PdfObject { bounding_box, .. }
            | Self::OcrSpan { bounding_box, .. }
            | Self::Model { bounding_box, .. } => *bounding_box,
        }
    }
}

/// One extracted field paired with its typed citation.
///
/// This is the shape T-066 names in the spec
/// (`{value, source, confidence}`) and the carrier the engine
/// hands to the evidence bundle so an auditor can reconstruct
/// every extracted field's provenance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldCitation {
    /// JSON-pointer-style path of the extracted field.
    pub path: String,
    /// Verbatim extracted value (numeric values are stringified
    /// to preserve the issuer's formatting).
    pub value: String,
    /// Typed source pointer.
    pub source: CitationSource,
    /// Confidence in `[0.0, 1.0]`.
    pub confidence: Confidence,
    /// Intake layer that emitted this citation.
    pub layer: ExtractionLayer,
}

impl FieldCitation {
    /// Build a validated field citation.
    ///
    /// # Errors
    ///
    /// Returns [`CitationError::EmptyPath`] when `path` is
    /// empty, [`CitationError::EmptyValue`] when `value` is
    /// empty, [`CitationError::EmptyOcrSpanId`] when an
    /// [`CitationSource::OcrSpan`] carries an empty span id,
    /// or [`CitationError::EmptyModelId`] when a
    /// [`CitationSource::Model`] carries an empty model id.
    pub fn validated(
        path: impl Into<String>,
        value: impl Into<String>,
        source: CitationSource,
        confidence: Confidence,
        layer: ExtractionLayer,
    ) -> Result<Self, CitationError> {
        let path = path.into();
        require_non_empty(&path, CitationError::EmptyPath)?;
        let value = value.into();
        require_non_empty(&value, CitationError::EmptyValue)?;
        match &source {
            CitationSource::OcrSpan { span_id, .. } if span_id.is_empty() => {
                return Err(CitationError::EmptyOcrSpanId);
            }
            CitationSource::Model { model_id, .. } if model_id.is_empty() => {
                return Err(CitationError::EmptyModelId);
            }
            _ => {}
        }
        Ok(Self {
            path,
            value,
            source,
            confidence,
            layer,
        })
    }
}

impl BoundingBox {
    /// Build a validated bounding box.
    ///
    /// # Errors
    ///
    /// Returns [`CitationError`] when `page` is zero, the
    /// width / height are non-positive, or any coordinate is
    /// non-finite.
    pub fn validated(
        page: u32,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Result<Self, CitationError> {
        if page == 0 {
            return Err(CitationError::InvalidPage);
        }
        if !(x.is_finite() && y.is_finite() && width.is_finite() && height.is_finite()) {
            return Err(CitationError::NonFiniteCoordinate);
        }
        if width <= 0.0 || height <= 0.0 {
            return Err(CitationError::NonPositiveExtent(width, height));
        }
        Ok(Self {
            page,
            x,
            y,
            width,
            height,
        })
    }
}

impl BoundingBoxCitation {
    /// Build a validated citation.
    ///
    /// # Errors
    ///
    /// Returns [`CitationError`] when the path or extractor
    /// id is empty.
    pub fn validated(
        path: impl Into<String>,
        bounding_box: BoundingBox,
        layer: ExtractionLayer,
        source_text: impl Into<String>,
        confidence: Confidence,
        extractor_id: impl Into<String>,
    ) -> Result<Self, CitationError> {
        let path = path.into();
        require_non_empty(&path, CitationError::EmptyPath)?;
        let extractor_id = extractor_id.into();
        require_non_empty(&extractor_id, CitationError::EmptyExtractorId)?;
        Ok(Self {
            path,
            bounding_box,
            layer,
            source_text: source_text.into(),
            confidence,
            extractor_id,
        })
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_intake_citation::crate_name(),
///     "invoicekit-intake-citation"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-intake-citation"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_box() -> BoundingBox {
        BoundingBox::validated(1, 24.0, 700.0, 200.0, 16.0).unwrap()
    }

    fn sample_citation(path: &str, layer: ExtractionLayer) -> BoundingBoxCitation {
        BoundingBoxCitation::validated(
            path,
            sample_box(),
            layer,
            "Acme GmbH",
            layer.default_confidence(),
            "test-extractor-v1",
        )
        .unwrap()
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-intake-citation");
    }

    #[test]
    fn bounding_box_helpers_compute_edges_and_overlap() {
        let a = BoundingBox::validated(1, 0.0, 0.0, 10.0, 10.0).unwrap();
        let b = BoundingBox::validated(1, 5.0, 5.0, 10.0, 10.0).unwrap();
        let c = BoundingBox::validated(1, 20.0, 20.0, 10.0, 10.0).unwrap();
        let d_other_page = BoundingBox::validated(2, 5.0, 5.0, 10.0, 10.0).unwrap();
        assert!(a.overlaps(b));
        assert!(b.overlaps(a));
        assert!(!a.overlaps(c));
        assert!(!a.overlaps(d_other_page));
        assert!((a.right() - 10.0).abs() < f32::EPSILON);
        assert!((a.top() - 10.0).abs() < f32::EPSILON);
    }

    #[test]
    fn bounding_box_validated_rejects_invalid_inputs() {
        assert!(matches!(
            BoundingBox::validated(0, 0.0, 0.0, 10.0, 10.0),
            Err(CitationError::InvalidPage)
        ));
        assert!(matches!(
            BoundingBox::validated(1, 0.0, 0.0, 0.0, 10.0),
            Err(CitationError::NonPositiveExtent(_, _))
        ));
        assert!(matches!(
            BoundingBox::validated(1, 0.0, 0.0, 10.0, -1.0),
            Err(CitationError::NonPositiveExtent(_, _))
        ));
        assert!(matches!(
            BoundingBox::validated(1, f32::NAN, 0.0, 10.0, 10.0),
            Err(CitationError::NonFiniteCoordinate)
        ));
    }

    #[test]
    fn extraction_layer_slug_round_trips_kebab_json() {
        let json = serde_json::to_string(&ExtractionLayer::ServerOcr).unwrap();
        assert_eq!(json, "\"server-ocr\"");
        let back: ExtractionLayer = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ExtractionLayer::ServerOcr);
        for layer in [
            ExtractionLayer::DigitalPdfText,
            ExtractionLayer::DigitalPdfXml,
            ExtractionLayer::ServerOcr,
            ExtractionLayer::VisionLanguageModel,
            ExtractionLayer::HumanOverride,
        ] {
            let s = layer.slug();
            assert!(!s.is_empty());
        }
    }

    #[test]
    fn extraction_layer_default_confidence_orders_correctly() {
        assert!(
            ExtractionLayer::DigitalPdfText.default_confidence().value()
                > ExtractionLayer::ServerOcr.default_confidence().value()
        );
        assert!(
            ExtractionLayer::ServerOcr.default_confidence().value()
                > ExtractionLayer::VisionLanguageModel
                    .default_confidence()
                    .value()
        );
        assert!(
            (ExtractionLayer::HumanOverride.default_confidence().value() - 1.0).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn confidence_clamps_out_of_range_input() {
        assert!((Confidence::new(1.5).value() - 1.0).abs() < f32::EPSILON);
        assert!(Confidence::new(-0.2).value().abs() < f32::EPSILON);
        assert!(Confidence::new(f32::NAN).value().abs() < f32::EPSILON);
        assert!((Confidence::new(0.42).value() - 0.42).abs() < f32::EPSILON);
    }

    #[test]
    fn citation_validated_rejects_empty_path_or_extractor() {
        let err = BoundingBoxCitation::validated(
            "",
            sample_box(),
            ExtractionLayer::ServerOcr,
            "x",
            Confidence::new(1.0),
            "id",
        )
        .unwrap_err();
        assert!(matches!(err, CitationError::EmptyPath));
        let err = BoundingBoxCitation::validated(
            "/x",
            sample_box(),
            ExtractionLayer::ServerOcr,
            "x",
            Confidence::new(1.0),
            "",
        )
        .unwrap_err();
        assert!(matches!(err, CitationError::EmptyExtractorId));
    }

    #[test]
    fn ledger_records_and_indexes_citations() {
        let mut ledger = CitationLedger::new();
        assert!(ledger.is_empty());
        ledger.record(sample_citation(
            "/supplier/name",
            ExtractionLayer::DigitalPdfText,
        ));
        ledger.record(sample_citation(
            "/lines/0/unit_price",
            ExtractionLayer::ServerOcr,
        ));
        ledger.record(sample_citation(
            "/supplier/name",
            ExtractionLayer::HumanOverride,
        ));
        assert_eq!(ledger.len(), 3);
        assert_eq!(ledger.for_path("/supplier/name").len(), 2);
        assert_eq!(
            ledger.winner_for("/supplier/name").unwrap().layer,
            ExtractionLayer::HumanOverride
        );
        assert!(ledger.for_path("/missing").is_empty());
    }

    #[test]
    fn ledger_low_confidence_predicate_flags_below_floor() {
        let mut ledger = CitationLedger::new();
        ledger.record(sample_citation(
            "/supplier/name",
            ExtractionLayer::DigitalPdfText, // 1.0
        ));
        ledger.record(sample_citation(
            "/lines/0/description",
            ExtractionLayer::VisionLanguageModel, // 0.55
        ));
        let floor = Confidence::new(0.8);
        assert!(!ledger.has_low_confidence("/supplier/name", floor));
        assert!(ledger.has_low_confidence("/lines/0/description", floor));
    }

    #[test]
    fn ledger_iter_visits_every_citation_in_stable_order() {
        let mut ledger = CitationLedger::new();
        ledger.record(sample_citation("/b", ExtractionLayer::DigitalPdfText));
        ledger.record(sample_citation("/a", ExtractionLayer::ServerOcr));
        ledger.record(sample_citation("/a", ExtractionLayer::HumanOverride));
        let paths: Vec<&str> = ledger.iter().map(|c| c.path.as_str()).collect();
        // BTreeMap ordering: /a (twice, in insertion order) then /b
        assert_eq!(paths, vec!["/a", "/a", "/b"]);
    }

    #[test]
    fn ledger_round_trips_through_json() {
        let mut ledger = CitationLedger::new();
        ledger.record(sample_citation(
            "/supplier/name",
            ExtractionLayer::DigitalPdfText,
        ));
        let json = serde_json::to_string(&ledger).unwrap();
        let back: CitationLedger = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ledger);
    }

    #[test]
    fn five_sample_extractions_carry_distinct_layers_and_paths() {
        // Acceptance gate from T-066: at least 5 sample
        // extractions verified to carry the right citation.
        let mut ledger = CitationLedger::new();
        let samples = [
            ("/supplier/name", ExtractionLayer::DigitalPdfText),
            ("/customer/name", ExtractionLayer::DigitalPdfText),
            ("/document_number", ExtractionLayer::ServerOcr),
            ("/issue_date", ExtractionLayer::ServerOcr),
            ("/lines/0/unit_price", ExtractionLayer::VisionLanguageModel),
        ];
        for (path, layer) in samples {
            ledger.record(sample_citation(path, layer));
        }
        assert_eq!(ledger.len(), 5);
        // Each unique path has exactly one citation.
        for (path, layer) in samples {
            let cite = ledger.winner_for(path).expect(path);
            assert_eq!(cite.layer, layer);
            assert!(cite.confidence.value() > 0.0);
            assert_eq!(cite.bounding_box.page, 1);
        }
    }

    #[test]
    fn citation_source_round_trips_each_kebab_tag() {
        for src in [
            CitationSource::PdfObject {
                object_id: 42,
                bounding_box: Some(sample_box()),
            },
            CitationSource::BoundingBox {
                bounding_box: sample_box(),
            },
            CitationSource::OcrSpan {
                span_id: "ocr-span-7".to_owned(),
                bounding_box: Some(sample_box()),
            },
            CitationSource::Model {
                model_id: "smol-docling-256m-int8".to_owned(),
                bounding_box: None,
            },
        ] {
            let json = serde_json::to_string(&src).unwrap();
            let back: CitationSource = serde_json::from_str(&json).unwrap();
            assert_eq!(back, src);
        }
    }

    #[test]
    fn citation_source_bounding_box_helper_returns_inner_box() {
        let bb = sample_box();
        assert_eq!(
            CitationSource::BoundingBox { bounding_box: bb }.bounding_box(),
            Some(bb)
        );
        assert_eq!(
            CitationSource::PdfObject {
                object_id: 1,
                bounding_box: Some(bb),
            }
            .bounding_box(),
            Some(bb)
        );
        assert_eq!(
            CitationSource::Model {
                model_id: "x".to_owned(),
                bounding_box: None,
            }
            .bounding_box(),
            None
        );
    }

    #[test]
    fn field_citation_validated_rejects_empty_inputs() {
        let bb_src = CitationSource::BoundingBox {
            bounding_box: sample_box(),
        };
        assert!(matches!(
            FieldCitation::validated(
                "",
                "Acme",
                bb_src.clone(),
                Confidence::new(1.0),
                ExtractionLayer::DigitalPdfText,
            ),
            Err(CitationError::EmptyPath)
        ));
        assert!(matches!(
            FieldCitation::validated(
                "/supplier/name",
                "",
                bb_src,
                Confidence::new(1.0),
                ExtractionLayer::DigitalPdfText,
            ),
            Err(CitationError::EmptyValue)
        ));
        assert!(matches!(
            FieldCitation::validated(
                "/x",
                "v",
                CitationSource::OcrSpan {
                    span_id: String::new(),
                    bounding_box: None,
                },
                Confidence::new(0.5),
                ExtractionLayer::ServerOcr,
            ),
            Err(CitationError::EmptyOcrSpanId)
        ));
        assert!(matches!(
            FieldCitation::validated(
                "/x",
                "v",
                CitationSource::Model {
                    model_id: String::new(),
                    bounding_box: None,
                },
                Confidence::new(0.5),
                ExtractionLayer::VisionLanguageModel,
            ),
            Err(CitationError::EmptyModelId)
        ));
    }

    #[test]
    fn five_field_citations_one_per_taxonomy_variant_round_trip() {
        // Acceptance gate from T-066: at least 5 sample
        // extractions verified to carry the right citation;
        // this set exercises every CitationSource variant so
        // the taxonomy is genuinely covered, not just the
        // BoundingBoxCitation legacy shape.
        let citations = vec![
            FieldCitation::validated(
                "/supplier/name",
                "Acme GmbH",
                CitationSource::PdfObject {
                    object_id: 42,
                    bounding_box: Some(sample_box()),
                },
                Confidence::new(1.0),
                ExtractionLayer::DigitalPdfText,
            )
            .unwrap(),
            FieldCitation::validated(
                "/customer/name",
                "Globex Corp",
                CitationSource::BoundingBox {
                    bounding_box: sample_box(),
                },
                Confidence::new(1.0),
                ExtractionLayer::DigitalPdfXml,
            )
            .unwrap(),
            FieldCitation::validated(
                "/document_number",
                "INV-2026-0001",
                CitationSource::OcrSpan {
                    span_id: "paddle-line-17".to_owned(),
                    bounding_box: Some(sample_box()),
                },
                Confidence::new(0.91),
                ExtractionLayer::ServerOcr,
            )
            .unwrap(),
            FieldCitation::validated(
                "/issue_date",
                "2026-05-28",
                CitationSource::OcrSpan {
                    span_id: "paddle-line-18".to_owned(),
                    bounding_box: None,
                },
                Confidence::new(0.88),
                ExtractionLayer::ServerOcr,
            )
            .unwrap(),
            FieldCitation::validated(
                "/lines/0/unit_price",
                "120.00",
                CitationSource::Model {
                    model_id: "smol-docling-256m-int8".to_owned(),
                    bounding_box: Some(sample_box()),
                },
                Confidence::new(0.62),
                ExtractionLayer::VisionLanguageModel,
            )
            .unwrap(),
        ];
        assert_eq!(citations.len(), 5);
        let json = serde_json::to_string(&citations).unwrap();
        let back: Vec<FieldCitation> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, citations);
        // Each variant survived the round-trip.
        assert!(matches!(
            back[0].source,
            CitationSource::PdfObject { object_id: 42, .. }
        ));
        assert!(matches!(back[1].source, CitationSource::BoundingBox { .. }));
        assert!(matches!(back[2].source, CitationSource::OcrSpan { .. }));
        assert!(matches!(back[3].source, CitationSource::OcrSpan { .. }));
        assert!(matches!(back[4].source, CitationSource::Model { .. }));
    }
}
