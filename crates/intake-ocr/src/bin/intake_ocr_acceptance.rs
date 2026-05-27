// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Release acceptance harness for real `PaddleOCR` invoice scans.

use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use invoicekit_intake_ocr::{BoundingBox, OcrDocument, OcrError, PaddleOcrCommand};
use serde::{Deserialize, Serialize};

const EXIT_ACCEPTANCE_FAILED: u8 = 1;
const EXIT_OPERATIONAL_ERROR: u8 = 2;
const DEFAULT_MIN_FIXTURES: usize = 10;
const DEFAULT_MAX_REGRESSION_PCT: f64 = 2.0;
const DEFAULT_MIN_BBOX_IOU: f64 = 0.5;
const DEFAULT_MAX_P95_SECONDS: f64 = 10.0;

macro_rules! reportln {
    ($report:expr) => {
        writeln!($report).map_err(report_error)
    };
    ($report:expr, $($arg:tt)*) => {
        writeln!($report, $($arg)*).map_err(report_error)
    };
}

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(AppError::Acceptance(message)) => {
            eprintln!("acceptance failed: {message}");
            ExitCode::from(EXIT_ACCEPTANCE_FAILED)
        }
        Err(AppError::Operational(message)) => {
            eprintln!("acceptance harness error: {message}");
            ExitCode::from(EXIT_OPERATIONAL_ERROR)
        }
    }
}

fn run(argv: Vec<OsString>) -> Result<(), AppError> {
    let cli = Args::parse(argv)?;
    let corpus = resolve_corpus(&cli)?;
    let thresholds = ThresholdConfig::load(cli.thresholds.as_deref())?;
    let run_root = cli.output_dir.clone().unwrap_or_else(default_output_dir);
    fs::create_dir_all(&run_root).map_err(|err| {
        AppError::Operational(format!(
            "failed to create output directory {}: {err}",
            run_root.display()
        ))
    })?;

    let fixtures = discover_fixtures(&corpus)?;
    if fixtures.len() < thresholds.minimum_fixture_count {
        return Err(AppError::Acceptance(format!(
            "only {} fixture(s) found under {}; need at least {}",
            fixtures.len(),
            corpus.display(),
            thresholds.minimum_fixture_count
        )));
    }

    let command = build_paddle_command(&cli);
    let mut fixture_results = Vec::with_capacity(fixtures.len());
    let mut field_totals = FieldTotals::default();

    for fixture in fixtures {
        let output_dir = run_root.join(&fixture.id);
        fs::create_dir_all(&output_dir).map_err(|err| {
            AppError::Operational(format!(
                "failed to create PaddleOCR output directory {}: {err}",
                output_dir.display()
            ))
        })?;

        let started = Instant::now();
        let document = command
            .run_path(&fixture.artifact_path, &output_dir)
            .map_err(|err| {
                AppError::Operational(format!(
                    "PaddleOCR failed for fixture {} ({}): {}",
                    fixture.id,
                    fixture.artifact_path.display(),
                    format_ocr_error(&err)
                ))
            })?;
        let elapsed = started.elapsed();

        let result = score_fixture(&fixture, &document, thresholds.min_bbox_iou, elapsed);
        field_totals.add(&result);
        fixture_results.push(result);
    }

    let field_scores = field_totals.scores();
    let threshold_results = thresholds.evaluate(&field_scores);
    let performance = thresholds.evaluate_performance(&fixture_results);
    let report = render_report(
        &corpus,
        &run_root,
        &fixture_results,
        &threshold_results,
        &performance,
    )?;

    if let Some(parent) = cli.report.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                AppError::Operational(format!(
                    "failed to create report directory {}: {err}",
                    parent.display()
                ))
            })?;
        }
    }
    fs::write(&cli.report, report).map_err(|err| {
        AppError::Operational(format!(
            "failed to write report {}: {err}",
            cli.report.display()
        ))
    })?;

    if cli.baseline {
        let baseline = ThresholdConfig::baseline_from_scores(&thresholds, &field_scores);
        let Some(path) = cli.thresholds.as_deref() else {
            return Err(AppError::Operational(
                "--baseline requires --thresholds so the baseline has a destination".to_owned(),
            ));
        };
        baseline.write(path)?;
    }

    let field_failures = threshold_results
        .values()
        .filter(|result| !result.passed)
        .count();
    let failures = field_failures + usize::from(!performance.passed);
    if failures > 0 {
        return Err(AppError::Acceptance(format!(
            "{failures} acceptance threshold(s) failed; report written to {}",
            cli.report.display()
        )));
    }

    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
struct Args {
    corpus: String,
    report: PathBuf,
    thresholds: Option<PathBuf>,
    output_dir: Option<PathBuf>,
    timeout: Duration,
    device: Option<String>,
    engine: Option<String>,
    baseline: bool,
}

impl Args {
    fn parse(argv: Vec<OsString>) -> Result<Self, AppError> {
        let mut corpus = None;
        let mut report = None;
        let mut thresholds = None;
        let mut output_dir = None;
        let mut timeout = Duration::from_secs(600);
        let mut device = None;
        let mut engine = None;
        let mut baseline = false;

        let mut iter = argv.into_iter();
        while let Some(arg) = iter.next() {
            let arg = arg
                .into_string()
                .map_err(|_| AppError::Operational("arguments must be valid UTF-8".to_owned()))?;
            if let Some(value) = arg.strip_prefix("--corpus=") {
                corpus = Some(value.to_owned());
                continue;
            }
            if let Some(value) = arg.strip_prefix("--report=") {
                report = Some(PathBuf::from(value));
                continue;
            }
            if let Some(value) = arg.strip_prefix("--thresholds=") {
                thresholds = Some(PathBuf::from(value));
                continue;
            }
            if let Some(value) = arg.strip_prefix("--output-dir=") {
                output_dir = Some(PathBuf::from(value));
                continue;
            }
            if let Some(value) = arg.strip_prefix("--timeout-seconds=") {
                let seconds = value.parse::<u64>().map_err(|err| {
                    AppError::Operational(format!("--timeout-seconds must be an integer: {err}"))
                })?;
                timeout = Duration::from_secs(seconds);
                continue;
            }
            if let Some(value) = arg.strip_prefix("--device=") {
                device = Some(value.to_owned());
                continue;
            }
            if let Some(value) = arg.strip_prefix("--engine=") {
                engine = Some(value.to_owned());
                continue;
            }
            match arg.as_str() {
                "--corpus" => corpus = Some(next_value(&mut iter, "--corpus")?),
                "--report" => report = Some(PathBuf::from(next_value(&mut iter, "--report")?)),
                "--thresholds" => {
                    thresholds = Some(PathBuf::from(next_value(&mut iter, "--thresholds")?));
                }
                "--output-dir" => {
                    output_dir = Some(PathBuf::from(next_value(&mut iter, "--output-dir")?));
                }
                "--timeout-seconds" => {
                    let raw = next_value(&mut iter, "--timeout-seconds")?;
                    let seconds = raw.parse::<u64>().map_err(|err| {
                        AppError::Operational(format!(
                            "--timeout-seconds must be an integer: {err}"
                        ))
                    })?;
                    timeout = Duration::from_secs(seconds);
                }
                "--device" => device = Some(next_value(&mut iter, "--device")?),
                "--engine" => engine = Some(next_value(&mut iter, "--engine")?),
                "--baseline" => baseline = true,
                "-h" | "--help" => return Err(AppError::Operational(usage())),
                other => {
                    return Err(AppError::Operational(format!(
                        "unknown argument {other:?}\n{}",
                        usage()
                    )));
                }
            }
        }

        let Some(corpus) = corpus else {
            return Err(AppError::Operational(format!(
                "missing required --corpus\n{}",
                usage()
            )));
        };
        let Some(report) = report else {
            return Err(AppError::Operational(format!(
                "missing required --report\n{}",
                usage()
            )));
        };

        Ok(Self {
            corpus,
            report,
            thresholds,
            output_dir,
            timeout,
            device,
            engine,
            baseline,
        })
    }
}

fn next_value(
    iter: &mut impl Iterator<Item = OsString>,
    flag: &'static str,
) -> Result<String, AppError> {
    iter.next()
        .ok_or_else(|| AppError::Operational(format!("{flag} requires a value")))?
        .into_string()
        .map_err(|_| AppError::Operational(format!("{flag} value must be valid UTF-8")))
}

fn usage() -> String {
    "usage: intake-ocr-acceptance --corpus <PATH|s3://bucket/prefix> --report <PATH> \
     [--thresholds <PATH>] [--output-dir <PATH>] [--timeout-seconds <N>] \
     [--device <cpu|gpu:N>] [--engine <paddle|transformers>] [--baseline]"
        .to_owned()
}

fn resolve_corpus(args: &Args) -> Result<PathBuf, AppError> {
    if !args.corpus.starts_with("s3://") {
        return Ok(PathBuf::from(&args.corpus));
    }

    let cache_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(default_output_dir)
        .join("s3-corpus");
    fs::create_dir_all(&cache_dir).map_err(|err| {
        AppError::Operational(format!(
            "failed to create S3 corpus cache {}: {err}",
            cache_dir.display()
        ))
    })?;
    let status = Command::new("aws")
        .arg("s3")
        .arg("sync")
        .arg(&args.corpus)
        .arg(&cache_dir)
        .status()
        .map_err(|err| AppError::Operational(format!("failed to run aws s3 sync: {err}")))?;
    if !status.success() {
        return Err(AppError::Operational(format!(
            "aws s3 sync failed with status {status}"
        )));
    }
    Ok(cache_dir)
}

fn default_output_dir() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    PathBuf::from("target")
        .join("intake-ocr-acceptance")
        .join(format!("run-{stamp}"))
}

fn build_paddle_command(args: &Args) -> PaddleOcrCommand {
    let mut command = PaddleOcrCommand::new().with_timeout(args.timeout);
    if let Some(device) = &args.device {
        command = command.with_device(device);
    }
    if let Some(engine) = &args.engine {
        command = command.with_engine(engine);
    }
    command
}

fn discover_fixtures(corpus: &Path) -> Result<Vec<Fixture>, AppError> {
    let mut metadata_paths = Vec::new();
    collect_metadata_paths(corpus, &mut metadata_paths)?;
    metadata_paths.sort();

    let mut fixtures = Vec::new();
    for metadata_path in metadata_paths {
        let metadata_raw = fs::read_to_string(&metadata_path).map_err(|err| {
            AppError::Operational(format!(
                "failed to read fixture metadata {}: {err}",
                metadata_path.display()
            ))
        })?;
        let metadata: FixtureMetadata = serde_json::from_str(&metadata_raw).map_err(|err| {
            AppError::Operational(format!(
                "failed to parse fixture metadata {}: {err}",
                metadata_path.display()
            ))
        })?;
        if metadata.status != "active" || metadata.artifact.media_type != "application/pdf" {
            continue;
        }

        let fixture_dir = metadata_path
            .parent()
            .ok_or_else(|| AppError::Operational("metadata path has no parent".to_owned()))?;
        let ground_truth_path = fixture_dir.join("ocr-ground-truth.json");
        let ground_truth_raw = fs::read_to_string(&ground_truth_path).map_err(|err| {
            AppError::Operational(format!(
                "failed to read OCR ground truth {}: {err}",
                ground_truth_path.display()
            ))
        })?;
        let ground_truth: GroundTruth = serde_json::from_str(&ground_truth_raw).map_err(|err| {
            AppError::Operational(format!(
                "failed to parse OCR ground truth {}: {err}",
                ground_truth_path.display()
            ))
        })?;

        fixtures.push(Fixture {
            id: metadata.fixture_id,
            artifact_path: fixture_dir.join(metadata.artifact.path),
            ground_truth,
        });
    }

    Ok(fixtures)
}

fn collect_metadata_paths(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), AppError> {
    let entries = fs::read_dir(path).map_err(|err| {
        AppError::Operational(format!(
            "failed to read corpus path {}: {err}",
            path.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|err| {
            AppError::Operational(format!(
                "failed to read corpus entry under {}: {err}",
                path.display()
            ))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|err| {
            AppError::Operational(format!(
                "failed to stat corpus entry {}: {err}",
                path.display()
            ))
        })?;
        if file_type.is_dir() {
            collect_metadata_paths(&path, out)?;
        } else if path.file_name().is_some_and(|name| name == "metadata.json") {
            out.push(path);
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct FixtureMetadata {
    fixture_id: String,
    status: String,
    artifact: ArtifactMetadata,
}

#[derive(Debug, Deserialize)]
struct ArtifactMetadata {
    path: PathBuf,
    media_type: String,
}

#[derive(Debug)]
struct Fixture {
    id: String,
    artifact_path: PathBuf,
    ground_truth: GroundTruth,
}

#[derive(Debug, Deserialize)]
struct GroundTruth {
    fields: Vec<ExpectedField>,
}

#[derive(Debug, Deserialize)]
struct ExpectedField {
    name: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    bbox: Option<[f64; 4]>,
    #[serde(default)]
    min_iou: Option<f64>,
    #[serde(default)]
    expected_count: Option<usize>,
    #[serde(default)]
    text_prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ThresholdConfig {
    #[serde(default = "default_minimum_fixture_count")]
    minimum_fixture_count: usize,
    #[serde(default = "default_field_thresholds")]
    fields: BTreeMap<String, f64>,
    #[serde(default)]
    baseline: BTreeMap<String, f64>,
    #[serde(default = "default_max_regression_pct")]
    max_regression_pct: f64,
    #[serde(default = "default_min_bbox_iou")]
    min_bbox_iou: f64,
    #[serde(default = "default_max_p95_seconds")]
    max_p95_seconds: f64,
}

impl ThresholdConfig {
    fn load(path: Option<&Path>) -> Result<Self, AppError> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path).map_err(|err| {
            AppError::Operational(format!(
                "failed to read thresholds {}: {err}",
                path.display()
            ))
        })?;
        serde_json::from_str(&raw).map_err(|err| {
            AppError::Operational(format!(
                "failed to parse thresholds {}: {err}",
                path.display()
            ))
        })
    }

    fn evaluate(&self, scores: &BTreeMap<String, f64>) -> BTreeMap<String, ThresholdResult> {
        self.fields
            .iter()
            .map(|(name, minimum)| {
                let baseline_min = self
                    .baseline
                    .get(name)
                    .map_or(0.0, |score| score - (self.max_regression_pct / 100.0));
                let required = minimum.max(baseline_min).clamp(0.0, 1.0);
                let actual = scores.get(name).copied().unwrap_or(0.0);
                (
                    name.clone(),
                    ThresholdResult {
                        actual,
                        required,
                        passed: actual >= required,
                    },
                )
            })
            .collect()
    }

    fn evaluate_performance(&self, fixtures: &[FixtureResult]) -> PerformanceResult {
        let actual_p95_seconds = p95_seconds(fixtures);
        PerformanceResult {
            actual_p95_seconds,
            required_max_seconds: self.max_p95_seconds,
            passed: actual_p95_seconds <= self.max_p95_seconds,
        }
    }

    fn baseline_from_scores(current: &Self, scores: &BTreeMap<String, f64>) -> Self {
        Self {
            baseline: scores.clone(),
            ..current.clone()
        }
    }

    fn write(&self, path: &Path) -> Result<(), AppError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|err| {
                    AppError::Operational(format!(
                        "failed to create threshold directory {}: {err}",
                        parent.display()
                    ))
                })?;
            }
        }
        let raw = serde_json::to_string_pretty(self).map_err(|err| {
            AppError::Operational(format!("failed to serialize thresholds: {err}"))
        })?;
        fs::write(path, format!("{raw}\n")).map_err(|err| {
            AppError::Operational(format!(
                "failed to write thresholds {}: {err}",
                path.display()
            ))
        })
    }
}

impl Default for ThresholdConfig {
    fn default() -> Self {
        Self {
            minimum_fixture_count: DEFAULT_MIN_FIXTURES,
            fields: default_field_thresholds(),
            baseline: BTreeMap::new(),
            max_regression_pct: DEFAULT_MAX_REGRESSION_PCT,
            min_bbox_iou: DEFAULT_MIN_BBOX_IOU,
            max_p95_seconds: DEFAULT_MAX_P95_SECONDS,
        }
    }
}

#[derive(Debug)]
struct ThresholdResult {
    actual: f64,
    required: f64,
    passed: bool,
}

fn default_minimum_fixture_count() -> usize {
    DEFAULT_MIN_FIXTURES
}

fn default_max_regression_pct() -> f64 {
    DEFAULT_MAX_REGRESSION_PCT
}

fn default_min_bbox_iou() -> f64 {
    DEFAULT_MIN_BBOX_IOU
}

fn default_max_p95_seconds() -> f64 {
    DEFAULT_MAX_P95_SECONDS
}

fn default_field_thresholds() -> BTreeMap<String, f64> {
    BTreeMap::from([
        ("invoice_number".to_owned(), 0.95),
        ("line_item_count".to_owned(), 0.80),
        ("supplier_name".to_owned(), 0.90),
        ("total_amount".to_owned(), 0.95),
        ("vat_amount".to_owned(), 0.90),
    ])
}

#[derive(Debug)]
struct FixtureResult {
    id: String,
    elapsed: Duration,
    field_results: Vec<FieldResult>,
}

#[derive(Debug)]
struct FieldResult {
    name: String,
    passed: bool,
    detail: String,
}

#[derive(Default)]
struct FieldTotals {
    correct: BTreeMap<String, usize>,
    total: BTreeMap<String, usize>,
}

impl FieldTotals {
    fn add(&mut self, fixture: &FixtureResult) {
        for field in &fixture.field_results {
            *self.total.entry(field.name.clone()).or_default() += 1;
            if field.passed {
                *self.correct.entry(field.name.clone()).or_default() += 1;
            }
        }
    }

    fn scores(&self) -> BTreeMap<String, f64> {
        self.total
            .iter()
            .map(|(name, total)| {
                let correct = self.correct.get(name).copied().unwrap_or_default();
                (name.clone(), ratio(correct, *total))
            })
            .collect()
    }
}

fn ratio(correct: usize, total: usize) -> f64 {
    let Ok(correct) = u32::try_from(correct) else {
        return 0.0;
    };
    let Ok(total) = u32::try_from(total) else {
        return 0.0;
    };
    if total == 0 {
        0.0
    } else {
        f64::from(correct) / f64::from(total)
    }
}

fn score_fixture(
    fixture: &Fixture,
    document: &OcrDocument,
    default_min_iou: f64,
    elapsed: Duration,
) -> FixtureResult {
    let field_results = fixture
        .ground_truth
        .fields
        .iter()
        .map(|expected| score_field(expected, document, default_min_iou))
        .collect();
    FixtureResult {
        id: fixture.id.clone(),
        elapsed,
        field_results,
    }
}

fn score_field(
    expected: &ExpectedField,
    document: &OcrDocument,
    default_min_iou: f64,
) -> FieldResult {
    if let Some(expected_count) = expected.expected_count {
        let prefix = expected.text_prefix.as_deref().unwrap_or_default();
        let actual = document
            .spans()
            .filter(|span| span.text.trim().starts_with(prefix))
            .count();
        return FieldResult {
            name: expected.name.clone(),
            passed: actual == expected_count,
            detail: format!("expected count {expected_count}, got {actual}"),
        };
    }

    let Some(expected_text) = expected.text.as_deref() else {
        return FieldResult {
            name: expected.name.clone(),
            passed: false,
            detail: "missing text or expected_count in ground truth".to_owned(),
        };
    };

    let normalized_expected = normalize_text(expected_text);
    for span in document.spans() {
        if normalize_text(&span.text) != normalized_expected {
            continue;
        }
        if let Some(expected_bbox) = expected.bbox {
            let actual_iou = iou(&span.bbox, expected_bbox);
            let required_iou = expected.min_iou.unwrap_or(default_min_iou);
            if actual_iou < required_iou {
                continue;
            }
            return FieldResult {
                name: expected.name.clone(),
                passed: true,
                detail: format!("matched text and bbox iou {actual_iou:.3}"),
            };
        }
        return FieldResult {
            name: expected.name.clone(),
            passed: true,
            detail: "matched text".to_owned(),
        };
    }

    FieldResult {
        name: expected.name.clone(),
        passed: false,
        detail: format!("did not find expected text {expected_text:?}"),
    }
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn iou(actual: &BoundingBox, expected: [f64; 4]) -> f64 {
    let [ex, ey, ew, eh] = expected;
    let ax1 = actual.x;
    let ay1 = actual.y;
    let ax2 = actual.x + actual.width;
    let ay2 = actual.y + actual.height;
    let ex1 = ex;
    let ey1 = ey;
    let ex2 = ex + ew;
    let ey2 = ey + eh;

    let ix1 = ax1.max(ex1);
    let iy1 = ay1.max(ey1);
    let ix2 = ax2.min(ex2);
    let iy2 = ay2.min(ey2);
    let intersection = (ix2 - ix1).max(0.0) * (iy2 - iy1).max(0.0);
    let actual_area = actual.width.max(0.0) * actual.height.max(0.0);
    let expected_area = ew.max(0.0) * eh.max(0.0);
    let union = actual_area + expected_area - intersection;
    if union <= f64::EPSILON {
        0.0
    } else {
        intersection / union
    }
}

#[derive(Debug)]
struct PerformanceResult {
    actual_p95_seconds: f64,
    required_max_seconds: f64,
    passed: bool,
}

fn p95_seconds(fixtures: &[FixtureResult]) -> f64 {
    if fixtures.is_empty() {
        return 0.0;
    }
    let mut elapsed = fixtures
        .iter()
        .map(|fixture| fixture.elapsed.as_secs_f64())
        .collect::<Vec<_>>();
    elapsed.sort_by(f64::total_cmp);
    let rank = (elapsed.len() * 95).div_ceil(100);
    elapsed.get(rank.saturating_sub(1)).copied().unwrap_or(0.0)
}

fn render_report(
    corpus: &Path,
    run_root: &Path,
    fixtures: &[FixtureResult],
    thresholds: &BTreeMap<String, ThresholdResult>,
    performance: &PerformanceResult,
) -> Result<String, AppError> {
    let mut report = String::new();
    reportln!(report, "# PaddleOCR acceptance report")?;
    reportln!(report)?;
    reportln!(report, "- Corpus: `{}`", corpus.display())?;
    reportln!(report, "- Output root: `{}`", run_root.display())?;
    reportln!(report, "- Fixtures processed: {}", fixtures.len())?;
    reportln!(report)?;
    reportln!(report, "## Thresholds")?;
    reportln!(report)?;
    reportln!(report, "| Field | Actual | Required | Status |")?;
    reportln!(report, "|---|---:|---:|---|")?;
    for (field, result) in thresholds {
        let status = if result.passed { "pass" } else { "fail" };
        reportln!(
            report,
            "| {field} | {:.1}% | {:.1}% | {status} |",
            result.actual * 100.0,
            result.required * 100.0
        )?;
    }
    reportln!(report)?;
    reportln!(report, "## Performance")?;
    reportln!(report)?;
    reportln!(report, "| Metric | Actual | Required | Status |")?;
    reportln!(report, "|---|---:|---:|---|")?;
    let performance_status = if performance.passed { "pass" } else { "fail" };
    reportln!(
        report,
        "| end-to-end p95 | {:.3}s | <= {:.3}s | {performance_status} |",
        performance.actual_p95_seconds,
        performance.required_max_seconds
    )?;
    reportln!(report)?;
    reportln!(report, "## Fixtures")?;
    reportln!(report)?;
    for fixture in fixtures {
        reportln!(report, "### {}", fixture.id)?;
        reportln!(report)?;
        reportln!(
            report,
            "- PaddleOCR elapsed: {:.3}s",
            fixture.elapsed.as_secs_f64()
        )?;
        reportln!(report)?;
        reportln!(report, "| Field | Status | Detail |")?;
        reportln!(report, "|---|---|---|")?;
        for field in &fixture.field_results {
            let status = if field.passed { "pass" } else { "fail" };
            reportln!(
                report,
                "| {} | {status} | {} |",
                field.name,
                field.detail.replace('|', "\\|")
            )?;
        }
        reportln!(report)?;
    }
    Ok(report)
}

fn report_error(err: std::fmt::Error) -> AppError {
    AppError::Operational(format!("failed to render markdown report: {err}"))
}

fn format_ocr_error(err: &OcrError) -> String {
    err.to_string()
}

#[derive(Debug)]
enum AppError {
    Acceptance(String),
    Operational(String),
}

#[cfg(test)]
mod tests {
    use super::{
        iou, p95_seconds, score_field, Args, ExpectedField, FieldResult, FixtureResult,
        ThresholdConfig, DEFAULT_MIN_BBOX_IOU,
    };
    use invoicekit_intake_ocr::{
        BoundingBox, OcrDocument, OcrEngine, OcrPage, OcrSpan, OcrSpanKind, Point,
    };
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn parses_required_arguments() {
        let args = Args::parse(os_args(&[
            "--corpus",
            "fixtures",
            "--report",
            "target/report.md",
            "--timeout-seconds=12",
            "--device=cpu",
        ]))
        .unwrap();

        assert_eq!(args.corpus, "fixtures");
        assert_eq!(args.report, PathBuf::from("target/report.md"));
        assert_eq!(args.timeout, Duration::from_secs(12));
        assert_eq!(args.device.as_deref(), Some("cpu"));
    }

    #[test]
    fn parse_rejects_missing_required_flags() {
        let err = Args::parse(Vec::new()).unwrap_err();
        assert!(format!("{err:?}").contains("missing required --corpus"));
    }

    #[test]
    fn text_and_bbox_match_passes() {
        let document = document_with_spans(vec![("INV-001", [10.0, 20.0, 100.0, 25.0])]);
        let field = ExpectedField {
            name: "invoice_number".to_owned(),
            text: Some("inv-001".to_owned()),
            bbox: Some([10.0, 20.0, 100.0, 25.0]),
            min_iou: Some(0.99),
            expected_count: None,
            text_prefix: None,
        };

        let result = score_field(&field, &document, DEFAULT_MIN_BBOX_IOU);

        assert!(result.passed);
    }

    #[test]
    fn text_match_with_bad_bbox_fails() {
        let document = document_with_spans(vec![("INV-001", [500.0, 20.0, 100.0, 25.0])]);
        let field = ExpectedField {
            name: "invoice_number".to_owned(),
            text: Some("INV-001".to_owned()),
            bbox: Some([10.0, 20.0, 100.0, 25.0]),
            min_iou: Some(0.5),
            expected_count: None,
            text_prefix: None,
        };

        let result = score_field(&field, &document, DEFAULT_MIN_BBOX_IOU);

        assert!(!result.passed);
    }

    #[test]
    fn line_item_count_uses_prefix() {
        let document = document_with_spans(vec![
            ("Line item 1", [0.0, 0.0, 10.0, 10.0]),
            ("Line item 2", [0.0, 20.0, 10.0, 10.0]),
            ("Total", [0.0, 40.0, 10.0, 10.0]),
        ]);
        let field = ExpectedField {
            name: "line_item_count".to_owned(),
            text: None,
            bbox: None,
            min_iou: None,
            expected_count: Some(2),
            text_prefix: Some("Line item".to_owned()),
        };

        let result = score_field(&field, &document, DEFAULT_MIN_BBOX_IOU);

        assert!(result.passed);
    }

    #[test]
    fn threshold_uses_stricter_baseline_drift() {
        let config = ThresholdConfig {
            fields: BTreeMap::from([("invoice_number".to_owned(), 0.80)]),
            baseline: BTreeMap::from([("invoice_number".to_owned(), 0.99)]),
            max_regression_pct: 2.0,
            ..ThresholdConfig::default()
        };
        let scores = BTreeMap::from([("invoice_number".to_owned(), 0.965)]);

        let result = config.evaluate(&scores);

        assert!(!result["invoice_number"].passed);
        assert_close(result["invoice_number"].required, 0.97);
    }

    #[test]
    fn performance_threshold_uses_p95_runtime() {
        let config = ThresholdConfig {
            max_p95_seconds: 3.0,
            ..ThresholdConfig::default()
        };
        let fixtures = vec![
            fixture_result("f1", 1),
            fixture_result("f2", 2),
            fixture_result("f3", 4),
        ];

        let result = config.evaluate_performance(&fixtures);

        assert_close(result.actual_p95_seconds, 4.0);
        assert!(!result.passed);
    }

    #[test]
    fn p95_uses_nearest_rank() {
        let fixtures = (1..=20)
            .map(|seconds| fixture_result(&format!("f{seconds}"), seconds))
            .collect::<Vec<_>>();

        assert_close(p95_seconds(&fixtures), 19.0);
    }

    #[test]
    fn identical_boxes_have_full_iou() {
        let bbox = bbox([10.0, 20.0, 100.0, 25.0]);
        assert_close(iou(&bbox, [10.0, 20.0, 100.0, 25.0]), 1.0);
    }

    fn os_args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn document_with_spans(items: Vec<(&str, [f64; 4])>) -> OcrDocument {
        OcrDocument {
            pages: vec![OcrPage {
                index: 0,
                spans: items
                    .into_iter()
                    .enumerate()
                    .map(|(idx, (text, rect))| OcrSpan {
                        id: format!("p0-s{idx}"),
                        page_index: 0,
                        text: text.to_owned(),
                        bbox: bbox(rect),
                        confidence: Some(0.99),
                        kind: OcrSpanKind::Text,
                    })
                    .collect(),
            }],
            engine: OcrEngine::default(),
        }
    }

    fn fixture_result(id: &str, elapsed_secs: u64) -> FixtureResult {
        FixtureResult {
            id: id.to_owned(),
            elapsed: Duration::from_secs(elapsed_secs),
            field_results: vec![FieldResult {
                name: "invoice_number".to_owned(),
                passed: true,
                detail: "matched text".to_owned(),
            }],
        }
    }

    fn bbox([x, y, width, height]: [f64; 4]) -> BoundingBox {
        BoundingBox {
            x,
            y,
            width,
            height,
            polygon: vec![
                Point { x, y },
                Point { x: x + width, y },
                Point {
                    x: x + width,
                    y: y + height,
                },
                Point { x, y: y + height },
            ],
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < f64::EPSILON,
            "expected {expected}, got {actual}"
        );
    }
}
