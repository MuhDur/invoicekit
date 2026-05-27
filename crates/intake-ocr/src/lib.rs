// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Server-side OCR intake using `PaddleOCR` PP-StructureV3.
//!
//! This crate owns the Layer-3/Layer-4 OCR boundary in the InvoiceKit intake
//! stack. It does two things:
//!
//! 1. wraps the external `paddleocr pp_structurev3` executable from Rust, and
//! 2. normalizes `PaddleOCR` JSON output into [`OcrDocument`] / [`OcrSpan`].
//!
//! The wrapper deliberately uses caller-provided input and output paths. That
//! keeps model execution auditable and avoids hidden temporary files. `PaddleOCR`
//! installation, model download policy, GPU selection, and sandboxing are
//! deployment concerns; this crate validates the subprocess contract and the
//! JSON shape InvoiceKit consumes.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tracing::{debug, instrument};

const DEFAULT_PADDLEOCR_BINARY: &str = "paddleocr";

/// Canonical Cargo package name of this crate.
///
/// Used by the InvoiceKit release tooling and by the bead-correlation reports
/// to map runtime log records back to the originating crate without parsing
/// `Cargo.toml` at runtime.
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

/// OCR output for one input document.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OcrDocument {
    /// Pages in input order.
    pub pages: Vec<OcrPage>,
    /// OCR engine metadata attached to the normalized trace.
    pub engine: OcrEngine,
}

impl OcrDocument {
    /// Return every span in page order.
    pub fn spans(&self) -> impl Iterator<Item = &OcrSpan> {
        self.pages.iter().flat_map(|page| page.spans.iter())
    }

    /// Count all OCR spans across every page.
    #[must_use]
    pub fn span_count(&self) -> usize {
        self.spans().count()
    }

    /// Concatenate OCR text in span order, one span per line.
    #[must_use]
    pub fn plain_text(&self) -> String {
        self.spans()
            .map(|span| span.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn extend(&mut self, other: Self) {
        self.pages.extend(other.pages);
    }
}

/// OCR engine metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OcrEngine {
    /// Engine family name.
    pub name: String,
    /// Model or pipeline identifier.
    pub model_id: String,
}

impl Default for OcrEngine {
    fn default() -> Self {
        Self {
            name: "paddleocr".to_owned(),
            model_id: "PP-StructureV3".to_owned(),
        }
    }
}

/// OCR output for one page.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OcrPage {
    /// 0-based page index.
    pub index: usize,
    /// OCR spans in normalized reading order.
    ///
    /// When `PaddleOCR` returns `parsing_res_list`, InvoiceKit preserves that
    /// list because PP-StructureV3 documents it as the recovered reading order.
    /// Otherwise spans remain in the OCR result order for their source object.
    pub spans: Vec<OcrSpan>,
}

/// One OCR text span with source coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OcrSpan {
    /// Stable per-document span identifier.
    pub id: String,
    /// 0-based page index.
    pub page_index: usize,
    /// Recognized text.
    pub text: String,
    /// Bounding box in `PaddleOCR` image pixel coordinates.
    pub bbox: BoundingBox,
    /// Optional OCR confidence score.
    pub confidence: Option<f64>,
    /// High-level source kind.
    pub kind: OcrSpanKind,
}

/// High-level OCR source kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrSpanKind {
    /// Plain OCR text.
    Text,
    /// OCR text sourced from a table-recognition subpipeline.
    Table,
    /// OCR text sourced from seal/stamp recognition.
    Seal,
    /// OCR text from a recognized region that does not fit the kinds above.
    Other,
}

/// Axis-aligned bounds plus the original polygon.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoundingBox {
    /// Left coordinate in pixels.
    pub x: f64,
    /// Top coordinate in pixels.
    pub y: f64,
    /// Width in pixels.
    pub width: f64,
    /// Height in pixels.
    pub height: f64,
    /// Original polygon in `PaddleOCR` order.
    pub polygon: Vec<Point>,
}

impl BoundingBox {
    fn from_polygon(polygon: Vec<Point>) -> Result<Self, OcrError> {
        if polygon.is_empty() {
            return Err(OcrError::InvalidPayload(
                "OCR span has an empty bounding polygon".to_owned(),
            ));
        }

        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for point in &polygon {
            if !point.x.is_finite() || !point.y.is_finite() {
                return Err(OcrError::InvalidPayload(
                    "OCR span contains a non-finite coordinate".to_owned(),
                ));
            }
            min_x = min_x.min(point.x);
            min_y = min_y.min(point.y);
            max_x = max_x.max(point.x);
            max_y = max_y.max(point.y);
        }

        Ok(Self {
            x: min_x,
            y: min_y,
            width: (max_x - min_x).max(0.0),
            height: (max_y - min_y).max(0.0),
            polygon,
        })
    }
}

/// One point in image pixel coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// Horizontal pixel coordinate.
    pub x: f64,
    /// Vertical pixel coordinate.
    pub y: f64,
}

/// Boolean PP-StructureV3 flags surfaced by the Rust wrapper.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaddleBoolFlag {
    /// Load/use the document orientation classifier.
    DocOrientationClassify(bool),
    /// Load/use document unwarping.
    DocUnwarping(bool),
    /// Load/use text-line orientation classification.
    TextlineOrientation(bool),
}

impl PaddleBoolFlag {
    fn as_cli_pair(self) -> (&'static str, &'static str) {
        match self {
            Self::DocOrientationClassify(true) => ("--use_doc_orientation_classify", "True"),
            Self::DocOrientationClassify(false) => ("--use_doc_orientation_classify", "False"),
            Self::DocUnwarping(true) => ("--use_doc_unwarping", "True"),
            Self::DocUnwarping(false) => ("--use_doc_unwarping", "False"),
            Self::TextlineOrientation(true) => ("--use_textline_orientation", "True"),
            Self::TextlineOrientation(false) => ("--use_textline_orientation", "False"),
        }
    }
}

/// Rust subprocess adapter for `paddleocr pp_structurev3`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaddleOcrCommand {
    device: Option<String>,
    engine: Option<String>,
    bool_flags: Vec<PaddleBoolFlag>,
    extra_args: Vec<OsString>,
    timeout: Option<Duration>,
}

impl PaddleOcrCommand {
    /// Build a command wrapper using `paddleocr` from `PATH`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Select a `PaddleOCR` device such as `cpu`, `gpu:0`, or `npu:0`.
    #[must_use]
    pub fn with_device(mut self, device: impl Into<String>) -> Self {
        self.device = Some(device.into());
        self
    }

    /// Select a `PaddleOCR` inference engine such as `paddle` or `transformers`.
    #[must_use]
    pub fn with_engine(mut self, engine: impl Into<String>) -> Self {
        self.engine = Some(engine.into());
        self
    }

    /// Add a supported boolean PP-StructureV3 flag.
    #[must_use]
    pub fn with_bool_flag(mut self, flag: PaddleBoolFlag) -> Self {
        self.bool_flags.push(flag);
        self
    }

    /// Add an implementation-specific CLI argument pair or flag.
    #[must_use]
    pub fn with_extra_arg(mut self, arg: impl Into<OsString>) -> Self {
        self.extra_args.push(arg.into());
        self
    }

    /// Set a hard timeout budget for the `PaddleOCR` child process.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Return the exact argument vector used for `paddleocr`.
    #[must_use]
    pub fn args_for(&self, input_path: &Path, output_dir: &Path) -> Vec<OsString> {
        let mut args = vec![
            OsString::from("pp_structurev3"),
            OsString::from("-i"),
            input_path.as_os_str().to_owned(),
            OsString::from("--save_path"),
            output_dir.as_os_str().to_owned(),
        ];

        if let Some(device) = &self.device {
            args.push(OsString::from("--device"));
            args.push(OsString::from(device));
        }
        if let Some(engine) = &self.engine {
            args.push(OsString::from("--engine"));
            args.push(OsString::from(engine));
        }
        for flag in &self.bool_flags {
            let (name, value) = flag.as_cli_pair();
            args.push(OsString::from(name));
            args.push(OsString::from(value));
        }
        args.extend(self.extra_args.iter().cloned());
        args
    }

    /// Execute `PaddleOCR` against a path and normalize JSON files emitted into
    /// `output_dir`.
    ///
    /// # Errors
    ///
    /// Returns [`OcrError::Io`] for process spawn or output-file reads,
    /// [`OcrError::CommandFailed`] for non-zero `PaddleOCR` exits,
    /// [`OcrError::NoJsonOutput`] if no JSON result files were written, and
    /// JSON/payload errors from [`normalize_paddle_json`].
    #[instrument(skip(self, input_path, output_dir))]
    pub fn run_path(
        &self,
        input_path: impl AsRef<Path>,
        output_dir: impl AsRef<Path>,
    ) -> Result<OcrDocument, OcrError> {
        let input_path = input_path.as_ref();
        let output_dir = output_dir.as_ref();
        let args = self.args_for(input_path, output_dir);
        debug!(
            input = %input_path.display(),
            output = %output_dir.display(),
            "running PaddleOCR PP-StructureV3"
        );
        let binary = Path::new(DEFAULT_PADDLEOCR_BINARY);
        let mut command = Command::new("paddleocr");
        command.args(&args);
        let output = run_paddle_command(&mut command, binary, self.timeout)?;

        if !output.status.success() {
            return Err(OcrError::CommandFailed {
                status: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        normalize_paddle_output_dir(output_dir)
    }
}

fn run_paddle_command(
    command: &mut Command,
    binary: &Path,
    timeout: Option<Duration>,
) -> Result<Output, OcrError> {
    let Some(timeout) = timeout else {
        return command.output().map_err(|source| OcrError::Io {
            path: binary.to_path_buf(),
            source,
        });
    };

    debug!(?timeout, "running PaddleOCR with hard timeout");
    let started = Instant::now();
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| OcrError::Io {
            path: binary.to_path_buf(),
            source,
        })?;

    loop {
        if child
            .try_wait()
            .map_err(|source| OcrError::Io {
                path: binary.to_path_buf(),
                source,
            })?
            .is_some()
        {
            return child.wait_with_output().map_err(|source| OcrError::Io {
                path: binary.to_path_buf(),
                source,
            });
        }

        let elapsed = started.elapsed();
        if elapsed >= timeout {
            let _ = child.kill();
            let output = child.wait_with_output().map_err(|source| OcrError::Io {
                path: binary.to_path_buf(),
                source,
            })?;
            return Err(OcrError::CommandTimedOut {
                timeout,
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        let remaining = timeout.saturating_sub(elapsed);
        thread::sleep(remaining.min(Duration::from_millis(20)));
    }
}

/// Normalize all `PaddleOCR` JSON files in an output directory.
///
/// # Errors
///
/// Returns [`OcrError::Io`] if `output_dir` cannot be read,
/// [`OcrError::NoJsonOutput`] if it contains no `*.json` files, and JSON or
/// payload errors from [`normalize_paddle_json`].
pub fn normalize_paddle_output_dir(output_dir: &Path) -> Result<OcrDocument, OcrError> {
    let mut json_paths = Vec::new();
    for entry in fs::read_dir(output_dir).map_err(|source| OcrError::Io {
        path: output_dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| OcrError::Io {
            path: output_dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.extension() == Some(OsStr::new("json")) {
            json_paths.push(path);
        }
    }
    json_paths.sort();

    if json_paths.is_empty() {
        return Err(OcrError::NoJsonOutput {
            output_dir: output_dir.to_path_buf(),
        });
    }

    let mut document = OcrDocument::default();
    for path in json_paths {
        let raw = fs::read_to_string(&path).map_err(|source| OcrError::Io { path, source })?;
        document.extend(normalize_paddle_json(&raw)?);
    }
    renumber_pages_by_order(&mut document);
    Ok(document)
}

/// Normalize a `PaddleOCR` PP-StructureV3 JSON payload.
///
/// Supports both `save_to_json()` style payloads containing a top-level `res`
/// object and serving payloads containing
/// `result.layoutParsingResults[*].prunedResult`.
///
/// # Errors
///
/// Returns [`OcrError::InvalidJson`] when `raw` is not valid JSON and
/// [`OcrError::InvalidPayload`] when `PaddleOCR` returned text without usable
/// coordinates.
pub fn normalize_paddle_json(raw: &str) -> Result<OcrDocument, OcrError> {
    let value: Value = serde_json::from_str(raw).map_err(OcrError::InvalidJson)?;
    normalize_paddle_value(&value)
}

fn normalize_paddle_value(value: &Value) -> Result<OcrDocument, OcrError> {
    let mut document = OcrDocument::default();

    if let Some(results) = value
        .pointer("/result/layoutParsingResults")
        .or_else(|| value.get("layoutParsingResults"))
        .and_then(Value::as_array)
    {
        for (idx, result) in results.iter().enumerate() {
            let page_value = result.get("prunedResult").unwrap_or(result);
            document.pages.push(normalize_page(page_value, idx)?);
        }
        return Ok(document);
    }

    if let Some(res) = value.get("res") {
        document.pages.push(normalize_page(res, 0)?);
        return Ok(document);
    }

    document.pages.push(normalize_page(value, 0)?);
    Ok(document)
}

fn normalize_page(value: &Value, fallback_index: usize) -> Result<OcrPage, OcrError> {
    let page_index = value
        .get("page_index")
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(fallback_index);

    let mut spans = extract_parsing_result_spans(value, page_index)?;
    if spans.is_empty() {
        let mut ocr_objects = Vec::new();
        collect_ocr_objects(value, "res", &mut ocr_objects);

        for (path, object) in ocr_objects {
            let kind = kind_from_path(&path);
            extract_spans_from_object(&path, object, page_index, kind, spans.len(), &mut spans)?;
        }
    }
    deduplicate_spans(&mut spans);
    renumber_spans(page_index, &mut spans);

    Ok(OcrPage {
        index: page_index,
        spans,
    })
}

fn extract_parsing_result_spans(
    value: &Value,
    page_index: usize,
) -> Result<Vec<OcrSpan>, OcrError> {
    let Some(items) = value.get("parsing_res_list").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };

    let mut spans = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        let Some(text) = item
            .get("block_content")
            .or_else(|| item.get("content"))
            .or_else(|| item.get("text"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        let polygon =
            polygon_for_keys(item, &["block_bbox", "bbox", "coordinate"]).ok_or_else(|| {
                OcrError::InvalidPayload(format!(
                    "res.parsing_res_list[{idx}] has text but no block_bbox/bbox/coordinate"
                ))
            })?;
        let label = item
            .get("block_label")
            .or_else(|| item.get("label"))
            .and_then(Value::as_str)
            .unwrap_or("text");
        let bbox = BoundingBox::from_polygon(polygon)?;

        spans.push(OcrSpan {
            id: String::new(),
            page_index,
            text: text.to_owned(),
            bbox,
            confidence: item.get("score").and_then(Value::as_f64),
            kind: kind_from_label(label),
        });
    }

    Ok(spans)
}

fn collect_ocr_objects<'a>(value: &'a Value, path: &str, out: &mut Vec<(String, &'a Value)>) {
    match value {
        Value::Object(map) => {
            if map.get("rec_texts").and_then(Value::as_array).is_some() {
                out.push((path.to_owned(), value));
                return;
            }
            for key in [
                "overall_ocr_res",
                "text_paragraphs_ocr_res",
                "table_res_list",
                "table_ocr_pred",
                "table_ocr_res",
                "seal_res_list",
            ] {
                if let Some(child) = map.get(key) {
                    collect_ocr_objects(child, &format!("{path}.{key}"), out);
                }
            }
            for (key, child) in map {
                if matches!(
                    key.as_str(),
                    "overall_ocr_res"
                        | "text_paragraphs_ocr_res"
                        | "table_res_list"
                        | "table_ocr_pred"
                        | "table_ocr_res"
                        | "seal_res_list"
                ) {
                    continue;
                }
                collect_ocr_objects(child, &format!("{path}.{key}"), out);
            }
        }
        Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                collect_ocr_objects(child, &format!("{path}[{idx}]"), out);
            }
        }
        _ => {}
    }
}

fn kind_from_label(label: &str) -> OcrSpanKind {
    let lower = label.to_ascii_lowercase();
    if lower.contains("table") {
        OcrSpanKind::Table
    } else if lower.contains("seal") || lower.contains("stamp") {
        OcrSpanKind::Seal
    } else if lower.contains("text")
        || lower.contains("title")
        || lower.contains("paragraph")
        || lower.contains("header")
        || lower.contains("footer")
        || lower.contains("list")
    {
        OcrSpanKind::Text
    } else {
        OcrSpanKind::Other
    }
}

fn kind_from_path(path: &str) -> OcrSpanKind {
    let lower = path.to_ascii_lowercase();
    if lower.contains("table") {
        OcrSpanKind::Table
    } else if lower.contains("seal") {
        OcrSpanKind::Seal
    } else if lower.contains("ocr") || lower.contains("text") {
        OcrSpanKind::Text
    } else {
        OcrSpanKind::Other
    }
}

fn extract_spans_from_object(
    path: &str,
    object: &Value,
    page_index: usize,
    kind: OcrSpanKind,
    first_span_index: usize,
    out: &mut Vec<OcrSpan>,
) -> Result<(), OcrError> {
    let texts = object
        .get("rec_texts")
        .and_then(Value::as_array)
        .ok_or_else(|| OcrError::InvalidPayload(format!("{path} is missing rec_texts")))?;
    let scores = object.get("rec_scores").and_then(Value::as_array);

    for (idx, text_value) in texts.iter().enumerate() {
        let Some(text) = text_value
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        let polygon = polygon_at(object, idx).ok_or_else(|| {
            OcrError::InvalidPayload(format!(
                "{path}.rec_texts[{idx}] has no rec_polys/rec_boxes/dt_polys/dt_boxes entry"
            ))
        })?;
        let confidence = scores
            .and_then(|items| items.get(idx))
            .and_then(Value::as_f64)
            .filter(|score| score.is_finite());
        let bbox = BoundingBox::from_polygon(polygon)?;
        let span_index = first_span_index + out.len();

        out.push(OcrSpan {
            id: format!("p{page_index}-s{span_index}"),
            page_index,
            text: text.to_owned(),
            bbox,
            confidence,
            kind,
        });
    }

    Ok(())
}

fn polygon_at(object: &Value, idx: usize) -> Option<Vec<Point>> {
    for key in [
        "rec_polys",
        "rec_boxes",
        "dt_polys",
        "dt_boxes",
        "polys",
        "boxes",
    ] {
        let Some(item) = object
            .get(key)
            .and_then(Value::as_array)
            .and_then(|items| items.get(idx))
        else {
            continue;
        };
        if let Some(polygon) = parse_polygon(item) {
            return Some(polygon);
        }
    }
    None
}

fn polygon_for_keys(object: &Value, keys: &[&str]) -> Option<Vec<Point>> {
    for key in keys {
        if let Some(polygon) = object.get(*key).and_then(parse_polygon) {
            return Some(polygon);
        }
    }
    None
}

fn parse_polygon(value: &Value) -> Option<Vec<Point>> {
    let array = value.as_array()?;
    if array.len() == 4 && array.iter().all(Value::is_number) {
        let [x0, y0, x1, y1] = array.as_slice() else {
            return None;
        };
        let x0 = number(x0)?;
        let y0 = number(y0)?;
        let x1 = number(x1)?;
        let y1 = number(y1)?;
        return Some(vec![
            Point { x: x0, y: y0 },
            Point { x: x1, y: y0 },
            Point { x: x1, y: y1 },
            Point { x: x0, y: y1 },
        ]);
    }

    let mut points = Vec::with_capacity(array.len());
    for item in array {
        let pair = item.as_array()?;
        let [x, y] = pair.as_slice() else {
            return None;
        };
        points.push(Point {
            x: number(x)?,
            y: number(y)?,
        });
    }
    Some(points)
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite())
}

fn deduplicate_spans(spans: &mut Vec<OcrSpan>) {
    let mut unique = Vec::with_capacity(spans.len());
    for span in spans.drain(..) {
        if unique
            .iter()
            .any(|existing| duplicate_span(existing, &span))
        {
            continue;
        }
        unique.push(span);
    }
    *spans = unique;
}

fn duplicate_span(left: &OcrSpan, right: &OcrSpan) -> bool {
    left.text == right.text && same_bbox(&left.bbox, &right.bbox)
}

fn same_bbox(left: &BoundingBox, right: &BoundingBox) -> bool {
    const TOLERANCE_PX: f64 = 1.0;
    (left.x - right.x).abs() <= TOLERANCE_PX
        && (left.y - right.y).abs() <= TOLERANCE_PX
        && (left.width - right.width).abs() <= TOLERANCE_PX
        && (left.height - right.height).abs() <= TOLERANCE_PX
}

fn renumber_pages_by_order(document: &mut OcrDocument) {
    for (page_index, page) in document.pages.iter_mut().enumerate() {
        page.index = page_index;
        renumber_spans(page_index, &mut page.spans);
    }
}

fn renumber_spans(page_index: usize, spans: &mut [OcrSpan]) {
    for (idx, span) in spans.iter_mut().enumerate() {
        span.page_index = page_index;
        span.id = format!("p{page_index}-s{idx}");
    }
}

/// Errors returned by OCR adapter and normalization APIs.
#[derive(Debug, Error)]
pub enum OcrError {
    /// Input JSON could not be parsed.
    #[error("invalid PaddleOCR JSON: {0}")]
    InvalidJson(serde_json::Error),
    /// `PaddleOCR` JSON parsed but lacked required InvoiceKit OCR fields.
    #[error("invalid PaddleOCR payload: {0}")]
    InvalidPayload(String),
    /// Process or filesystem I/O failed.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// Path being executed or read.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// `PaddleOCR` exited with a non-zero status.
    #[error("paddleocr pp_structurev3 failed with status {status:?}: {stderr}")]
    CommandFailed {
        /// Exit status code, if the process reported one.
        status: Option<i32>,
        /// Standard error emitted by `PaddleOCR`.
        stderr: String,
    },
    /// `PaddleOCR` did not exit before the configured timeout budget elapsed.
    #[error("paddleocr pp_structurev3 timed out after {timeout:?}: {stderr}")]
    CommandTimedOut {
        /// Timeout configured by the caller.
        timeout: Duration,
        /// Standard error captured before termination, if any.
        stderr: String,
    },
    /// `PaddleOCR` succeeded but did not write JSON output files.
    #[error("paddleocr wrote no JSON output files in {output_dir}")]
    NoJsonOutput {
        /// Output directory passed to `PaddleOCR`.
        output_dir: PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        crate_name, normalize_paddle_json, normalize_paddle_value, renumber_pages_by_order,
        OcrDocument, OcrError, OcrSpanKind, PaddleBoolFlag, PaddleOcrCommand,
    };
    use serde_json::{json, Value};
    use std::path::Path;
    use std::time::Instant;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-intake-ocr");
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
    fn normalizes_save_to_json_shape_with_text_scores_and_polygons() {
        let raw = json!({
            "res": {
                "input_path": "scan-01.png",
                "page_index": 0,
                "overall_ocr_res": {
                    "rec_texts": ["Invoice INV-001", "Total EUR 42.00"],
                    "rec_scores": [0.99, 0.96],
                    "rec_polys": [
                        [[10, 20], [210, 20], [210, 50], [10, 50]],
                        [[420, 720], [600, 720], [600, 760], [420, 760]]
                    ]
                }
            }
        });

        let document = normalize_paddle_value(&raw).unwrap();

        assert_eq!(document.pages.len(), 1);
        assert_eq!(document.span_count(), 2);
        assert_eq!(document.plain_text(), "Invoice INV-001\nTotal EUR 42.00");
        let total = &document.pages[0].spans[1];
        assert_eq!(total.id, "p0-s1");
        assert_eq!(total.kind, OcrSpanKind::Text);
        assert_close(total.bbox.x, 420.0);
        assert_close(total.bbox.y, 720.0);
        assert_close(total.bbox.width, 180.0);
        assert_close(total.bbox.height, 40.0);
        assert_close(total.confidence.unwrap(), 0.96);
    }

    #[test]
    fn normalizes_service_layout_parsing_results_shape() {
        let raw = json!({
            "result": {
                "layoutParsingResults": [
                    {
                        "prunedResult": {
                            "overall_ocr_res": {
                                "rec_texts": ["Supplier A"],
                                "rec_scores": [0.88],
                                "dt_polys": [
                                    [[5, 10], [105, 10], [105, 40], [5, 40]]
                                ]
                            }
                        }
                    },
                    {
                        "prunedResult": {
                            "overall_ocr_res": {
                                "rec_texts": ["Customer B"],
                                "rec_scores": [0.91],
                                "dt_polys": [
                                    [[6, 11], [106, 11], [106, 41], [6, 41]]
                                ]
                            }
                        }
                    }
                ]
            }
        });

        let document = normalize_paddle_value(&raw).unwrap();

        assert_eq!(document.pages.len(), 2);
        assert_eq!(document.pages[0].index, 0);
        assert_eq!(document.pages[1].index, 1);
        assert_eq!(document.pages[1].spans[0].id, "p1-s0");
        assert_eq!(document.plain_text(), "Supplier A\nCustomer B");
    }

    #[test]
    fn parsing_res_list_controls_reading_order_when_present() {
        let raw = json!({
            "res": {
                "parsing_res_list": [
                    {
                        "block_label": "text",
                        "block_content": "Right column comes first",
                        "block_bbox": [300, 100, 520, 140]
                    },
                    {
                        "block_label": "table",
                        "block_content": "Left table comes second",
                        "block_bbox": [20, 20, 260, 80]
                    }
                ],
                "overall_ocr_res": {
                    "rec_texts": ["Left table comes second", "Right column comes first"],
                    "rec_scores": [0.91, 0.92],
                    "rec_polys": [
                        [[20, 20], [260, 20], [260, 80], [20, 80]],
                        [[300, 100], [520, 100], [520, 140], [300, 140]]
                    ]
                }
            }
        });

        let document = normalize_paddle_value(&raw).unwrap();

        assert_eq!(document.span_count(), 2);
        assert_eq!(
            document.plain_text(),
            "Right column comes first\nLeft table comes second"
        );
        assert_eq!(document.pages[0].spans[0].id, "p0-s0");
        assert_eq!(document.pages[0].spans[1].kind, OcrSpanKind::Table);
    }

    #[test]
    fn fallback_preserves_ocr_result_order_without_geometric_resort() {
        let raw = json!({
            "res": {
                "overall_ocr_res": {
                    "rec_texts": ["Right column first", "Left column later"],
                    "rec_scores": [0.96, 0.95],
                    "rec_polys": [
                        [[300, 100], [520, 100], [520, 140], [300, 140]],
                        [[20, 20], [260, 20], [260, 80], [20, 80]]
                    ]
                }
            }
        });

        let document = normalize_paddle_value(&raw).unwrap();

        assert_eq!(
            document.plain_text(),
            "Right column first\nLeft column later"
        );
    }

    #[test]
    fn handles_rec_boxes_as_rectangles() {
        let raw = json!({
            "res": {
                "overall_ocr_res": {
                    "rec_texts": ["IBAN DE89"],
                    "rec_scores": [0.93],
                    "rec_boxes": [[100, 200, 340, 235]]
                }
            }
        });

        let document = normalize_paddle_value(&raw).unwrap();
        let span = &document.pages[0].spans[0];

        assert_close(span.bbox.x, 100.0);
        assert_close(span.bbox.y, 200.0);
        assert_close(span.bbox.width, 240.0);
        assert_close(span.bbox.height, 35.0);
        assert_eq!(span.bbox.polygon.len(), 4);
    }

    #[test]
    fn extracts_table_and_seal_subpipeline_spans() {
        let raw = json!({
            "res": {
                "table_res_list": [
                    {
                        "table_ocr_res": {
                            "rec_texts": ["VAT", "8.40"],
                            "rec_scores": [0.82, 0.8],
                            "rec_polys": [
                                [[1, 1], [21, 1], [21, 11], [1, 11]],
                                [[30, 1], [50, 1], [50, 11], [30, 11]]
                            ]
                        }
                    }
                ],
                "seal_res_region1": {
                    "rec_texts": ["PAID"],
                    "rec_scores": [0.7],
                    "rec_polys": [
                        [[300, 300], [360, 300], [360, 340], [300, 340]]
                    ]
                }
            }
        });

        let document = normalize_paddle_value(&raw).unwrap();
        let kinds = document.spans().map(|span| span.kind).collect::<Vec<_>>();

        assert_eq!(
            kinds,
            vec![OcrSpanKind::Table, OcrSpanKind::Table, OcrSpanKind::Seal]
        );
    }

    #[test]
    fn deduplicates_combined_overall_and_table_ocr_spans() {
        let raw = json!({
            "res": {
                "overall_ocr_res": {
                    "rec_texts": ["VAT"],
                    "rec_scores": [0.91],
                    "rec_polys": [
                        [[1, 1], [21, 1], [21, 11], [1, 11]]
                    ]
                },
                "table_res_list": [
                    {
                        "table_ocr_pred": {
                            "rec_texts": ["VAT"],
                            "rec_scores": [0.88],
                            "rec_polys": [
                                [[1, 1], [21, 1], [21, 11], [1, 11]]
                            ]
                        }
                    }
                ]
            }
        });

        let document = normalize_paddle_value(&raw).unwrap();

        assert_eq!(document.span_count(), 1);
        assert_eq!(document.pages[0].spans[0].kind, OcrSpanKind::Text);
    }

    #[test]
    fn ignores_blank_text_spans() {
        let raw = json!({
            "res": {
                "overall_ocr_res": {
                    "rec_texts": ["Invoice", "   ", ""],
                    "rec_scores": [0.9, 0.1, 0.1],
                    "rec_polys": [
                        [[0, 0], [20, 0], [20, 10], [0, 10]],
                        [[30, 0], [40, 0], [40, 10], [30, 10]],
                        [[50, 0], [60, 0], [60, 10], [50, 10]]
                    ]
                }
            }
        });

        let document = normalize_paddle_value(&raw).unwrap();

        assert_eq!(document.span_count(), 1);
        assert_eq!(document.plain_text(), "Invoice");
    }

    #[test]
    fn rejects_text_without_coordinates() {
        let raw = r#"{"res":{"overall_ocr_res":{"rec_texts":["Invoice"]}}}"#;
        let error = normalize_paddle_json(raw).unwrap_err();

        assert!(matches!(error, OcrError::InvalidPayload(_)));
        assert!(error.to_string().contains("no rec_polys"));
    }

    #[test]
    fn rejects_non_finite_or_empty_coordinates() {
        let raw = json!({
            "res": {
                "overall_ocr_res": {
                    "rec_texts": ["Invoice"],
                    "rec_polys": [[]]
                }
            }
        });
        let error = normalize_paddle_value(&raw).unwrap_err();

        assert!(matches!(error, OcrError::InvalidPayload(_)));
    }

    #[test]
    fn reports_invalid_json() {
        let error = normalize_paddle_json("{not json").unwrap_err();

        assert!(matches!(error, OcrError::InvalidJson(_)));
    }

    #[test]
    fn normalizes_ten_invoice_scan_pages() {
        let raw = synthetic_invoice_pages(10, 3);
        let document = normalize_paddle_value(&raw).unwrap();

        assert_eq!(document.pages.len(), 10);
        assert_eq!(document.span_count(), 30);
        for page in &document.pages {
            assert_eq!(page.spans.len(), 3);
            assert!(page.spans[0]
                .text
                .starts_with(&format!("Invoice INV-{:03}", page.index + 1)));
            assert_eq!(page.spans[0].page_index, page.index);
        }
    }

    #[test]
    fn five_page_normalization_is_well_under_p95_budget() {
        let raw = synthetic_invoice_pages(5, 20);
        let started = Instant::now();
        let document = normalize_paddle_value(&raw).unwrap();
        let elapsed = started.elapsed();

        assert_eq!(document.pages.len(), 5);
        assert_eq!(document.span_count(), 100);
        assert!(
            elapsed.as_secs_f64() < 10.0,
            "normalizing 5 synthetic OCR pages took {elapsed:?}"
        );
    }

    #[test]
    fn command_args_match_pp_structurev3_cli_contract() {
        let command = PaddleOcrCommand::new()
            .with_device("cpu")
            .with_engine("paddle")
            .with_bool_flag(PaddleBoolFlag::DocOrientationClassify(false))
            .with_bool_flag(PaddleBoolFlag::DocUnwarping(false))
            .with_bool_flag(PaddleBoolFlag::TextlineOrientation(true))
            .with_extra_arg("--cpu_threads")
            .with_extra_arg("4");

        let args = command
            .args_for(Path::new("scan.pdf"), Path::new("ocr-output"))
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            args,
            vec![
                "pp_structurev3",
                "-i",
                "scan.pdf",
                "--save_path",
                "ocr-output",
                "--device",
                "cpu",
                "--engine",
                "paddle",
                "--use_doc_orientation_classify",
                "False",
                "--use_doc_unwarping",
                "False",
                "--use_textline_orientation",
                "True",
                "--cpu_threads",
                "4",
            ]
        );
    }

    #[test]
    fn renumbers_pages_after_combining_multiple_json_outputs() {
        let mut document = OcrDocument::default();
        document.extend(
            normalize_paddle_value(&json!({
                "res": {
                    "page_index": null,
                    "overall_ocr_res": {
                        "rec_texts": ["First scan"],
                        "rec_polys": [
                            [[0, 0], [20, 0], [20, 10], [0, 10]]
                        ]
                    }
                }
            }))
            .unwrap(),
        );
        document.extend(
            normalize_paddle_value(&json!({
                "res": {
                    "page_index": null,
                    "overall_ocr_res": {
                        "rec_texts": ["Second scan"],
                        "rec_polys": [
                            [[0, 0], [20, 0], [20, 10], [0, 10]]
                        ]
                    }
                }
            }))
            .unwrap(),
        );

        renumber_pages_by_order(&mut document);

        assert_eq!(document.pages[0].index, 0);
        assert_eq!(document.pages[0].spans[0].id, "p0-s0");
        assert_eq!(document.pages[1].index, 1);
        assert_eq!(document.pages[1].spans[0].page_index, 1);
        assert_eq!(document.pages[1].spans[0].id, "p1-s0");
    }

    fn synthetic_invoice_pages(page_count: usize, spans_per_page: usize) -> Value {
        let pages = (0..page_count)
            .map(|page| {
                let texts = (0..spans_per_page)
                    .map(|span| match span {
                        0 => Value::String(format!("Invoice INV-{:03}", page + 1)),
                        1 => Value::String(format!("Supplier {}", page + 1)),
                        2 => Value::String(format!("Total EUR {}.00", 100 + page)),
                        _ => Value::String(format!("Line {span} page {page}")),
                    })
                    .collect::<Vec<_>>();
                let scores = (0..spans_per_page)
                    .map(|span| {
                        let span_index = u32::try_from(span).unwrap();
                        Value::from(f64::from(span_index).mul_add(-0.001, 0.99))
                    })
                    .collect::<Vec<_>>();
                let polys = (0..spans_per_page)
                    .map(|span| {
                        let x0 = 20 + (span % 3) * 180;
                        let y0 = 30 + (span / 3) * 32;
                        json!([[x0, y0], [x0 + 150, y0], [x0 + 150, y0 + 20], [x0, y0 + 20]])
                    })
                    .collect::<Vec<_>>();

                json!({
                    "prunedResult": {
                        "page_index": page,
                        "overall_ocr_res": {
                            "rec_texts": texts,
                            "rec_scores": scores,
                            "rec_polys": polys
                        }
                    }
                })
            })
            .collect::<Vec<_>>();

        json!({ "result": { "layoutParsingResults": pages } })
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < f64::EPSILON,
            "expected {expected}, got {actual}"
        );
    }
}
