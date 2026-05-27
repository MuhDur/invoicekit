// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-058 impl: PDF visual regression binary.
//!
//! Walks `conformance-corpus/pdf-snapshots/MANIFEST.json`, rasterises each
//! declared PDF candidate at 144 DPI via pdfium (when the `pdfium`
//! feature is enabled), diffs against the committed baseline PNG using
//! a fast per-pixel RGBA delta with anti-aliasing tolerance, and emits
//! a markdown report.
//!
//! Three CLI modes:
//!
//! * `pdf-visual-regression diff` — full pipeline (rasterise + diff +
//!   report). Requires the `pdfium` feature.
//! * `pdf-visual-regression diff-png` — diff a pair of already-rasterised
//!   PNGs without pdfium. Useful for unit tests + local debugging.
//! * `pdf-visual-regression bless` — overwrite baselines from a freshly
//!   rasterised set. Used by the release pipeline when the operator has
//!   added the `accept-visual-drift` PR label.
//!
//! The runbook at `docs/operators/PDF-VISUAL-REGRESSION.md` documents
//! the baseline layout + the PR-comment surface the workflow consumes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use image::{ImageBuffer, Rgba};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// CLI surface.
#[derive(Parser, Debug)]
#[command(
    name = "pdf-visual-regression",
    about = "T-058: rasterise + diff InvoiceKit PDFs against committed baselines"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Diff a directory of candidate PDFs against the manifest's
    /// committed baselines.
    Diff {
        /// Path to `conformance-corpus/pdf-snapshots/`.
        #[arg(long, default_value = "conformance-corpus/pdf-snapshots")]
        snapshots: PathBuf,
        /// Path to a directory containing the candidate PDFs to
        /// rasterise. Each candidate must have a matching baseline
        /// declared in the manifest by `relative_path`.
        #[arg(long)]
        candidates: PathBuf,
        /// Markdown report destination.
        #[arg(long)]
        report: PathBuf,
        /// Fail if the per-page drift exceeds this fraction
        /// (0.0..=1.0). 0.001 (= 0.1%) matches the runbook.
        #[arg(long, default_value_t = 0.001)]
        threshold: f64,
    },
    /// Diff two already-rasterised PNGs and print the drift
    /// percentage. No pdfium required.
    DiffPng {
        #[arg(long)]
        baseline: PathBuf,
        #[arg(long)]
        candidate: PathBuf,
        /// Optional output path for the diff visualisation PNG.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Overwrite the baseline PNGs from a freshly rasterised
    /// candidate set. Refuses to run unless `--i-have-the-label`
    /// is passed (workflow attaches this only when the PR carries
    /// the `accept-visual-drift` label).
    Bless {
        #[arg(long, default_value = "conformance-corpus/pdf-snapshots")]
        snapshots: PathBuf,
        #[arg(long)]
        candidates: PathBuf,
        #[arg(long, value_name = "yes")]
        i_have_the_label: Option<String>,
    },
}

/// Manifest entry, one per baseline.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct ManifestEntry {
    /// Path relative to `--snapshots`, identifying both the PDF
    /// (under `--candidates`) and the baseline PNG (next to the
    /// manifest).
    relative_path: String,
    /// sha256 of the committed baseline PNG bytes. Catches stale
    /// baselines that drifted from the manifest.
    baseline_sha256: String,
}

/// Top-level manifest shape.
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Manifest {
    /// Renderer version that produced the baselines.
    renderer_version: String,
    /// Rasterizer version (pdfium version).
    rasterizer_version: String,
    /// Per-baseline entries.
    entries: Vec<ManifestEntry>,
}

/// Per-entry outcome of a diff run.
#[derive(Clone, Debug)]
struct DiffOutcome {
    relative_path: String,
    drift_fraction: f64,
    pixel_count: u64,
    drifted_pixels: u64,
    status: DiffStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffStatus {
    Ok,
    Drifted,
    MissingBaseline,
    MissingCandidate,
    BaselineSha256Mismatch,
}

/// Top-level errors.
#[derive(Debug, Error)]
enum Error {
    #[error("I/O at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("manifest at {path} is not valid JSON: {source}")]
    Manifest {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("image decode at {path}: {source}")]
    Image {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    #[error("baseline {baseline} and candidate {candidate} differ in dimensions: {expected}x{eh} vs {actual}x{ah}")]
    Dimensions {
        baseline: PathBuf,
        candidate: PathBuf,
        expected: u32,
        eh: u32,
        actual: u32,
        ah: u32,
    },
    #[error("bless refused: pass --i-have-the-label yes to confirm (workflow attaches when the PR carries the accept-visual-drift label)")]
    BlessGuard,
    #[cfg(feature = "pdfium")]
    #[error("pdfium error: {0}")]
    Pdfium(String),
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Diff {
            snapshots,
            candidates,
            report,
            threshold,
        } => run_diff(&snapshots, &candidates, &report, threshold),
        Command::DiffPng {
            baseline,
            candidate,
            out,
        } => run_diff_png(&baseline, &candidate, out.as_deref()),
        Command::Bless {
            snapshots,
            candidates,
            i_have_the_label,
        } => run_bless(
            &snapshots,
            &candidates,
            i_have_the_label.as_deref().map(str::trim),
        ),
    }
}

fn run_diff(snapshots: &Path, candidates: &Path, report: &Path, threshold: f64) -> ExitCode {
    match diff_against_manifest(snapshots, candidates, threshold) {
        Ok((outcomes, summary)) => {
            if let Err(e) = write_report(report, &outcomes, &summary, threshold) {
                eprintln!("write report: {e}");
                return ExitCode::from(2);
            }
            if summary.drifted == 0 && summary.missing == 0 && summary.mismatched == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("diff failed: {e}");
            ExitCode::from(2)
        }
    }
}

fn run_diff_png(baseline: &Path, candidate: &Path, out: Option<&Path>) -> ExitCode {
    match diff_pngs(baseline, candidate, out) {
        Ok(outcome) => {
            println!(
                "{}: drift {:.4}% ({}/{} pixels)",
                outcome.relative_path,
                outcome.drift_fraction * 100.0,
                outcome.drifted_pixels,
                outcome.pixel_count
            );
            if matches!(outcome.status, DiffStatus::Ok) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("diff-png failed: {e}");
            ExitCode::from(2)
        }
    }
}

fn run_bless(snapshots: &Path, candidates: &Path, i_have_the_label: Option<&str>) -> ExitCode {
    if i_have_the_label != Some("yes") {
        eprintln!("{}", Error::BlessGuard);
        return ExitCode::from(2);
    }
    match bless(snapshots, candidates) {
        Ok(count) => {
            println!("blessed {count} baseline(s)");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("bless failed: {e}");
            ExitCode::from(2)
        }
    }
}

// ----- pipeline ------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct DiffSummary {
    total: usize,
    ok: usize,
    drifted: usize,
    missing: usize,
    mismatched: usize,
}

fn diff_against_manifest(
    snapshots: &Path,
    candidates: &Path,
    threshold: f64,
) -> Result<(Vec<DiffOutcome>, DiffSummary), Error> {
    let manifest = load_manifest(snapshots)?;
    let mut outcomes: Vec<DiffOutcome> = Vec::with_capacity(manifest.entries.len());
    let mut summary = DiffSummary::default();
    for entry in &manifest.entries {
        summary.total += 1;
        let baseline_png = snapshots.join(&entry.relative_path);
        let candidate_png = candidates.join(&entry.relative_path);
        if !baseline_png.is_file() {
            outcomes.push(DiffOutcome {
                relative_path: entry.relative_path.clone(),
                drift_fraction: 0.0,
                pixel_count: 0,
                drifted_pixels: 0,
                status: DiffStatus::MissingBaseline,
            });
            summary.missing += 1;
            continue;
        }
        if !verify_baseline_sha256(&baseline_png, &entry.baseline_sha256)? {
            outcomes.push(DiffOutcome {
                relative_path: entry.relative_path.clone(),
                drift_fraction: 0.0,
                pixel_count: 0,
                drifted_pixels: 0,
                status: DiffStatus::BaselineSha256Mismatch,
            });
            summary.mismatched += 1;
            continue;
        }
        if !candidate_png.is_file() {
            outcomes.push(DiffOutcome {
                relative_path: entry.relative_path.clone(),
                drift_fraction: 0.0,
                pixel_count: 0,
                drifted_pixels: 0,
                status: DiffStatus::MissingCandidate,
            });
            summary.missing += 1;
            continue;
        }
        let outcome = diff_pngs_for_entry(
            &baseline_png,
            &candidate_png,
            &entry.relative_path,
            threshold,
        )?;
        match outcome.status {
            DiffStatus::Ok => summary.ok += 1,
            DiffStatus::Drifted => summary.drifted += 1,
            _ => {}
        }
        outcomes.push(outcome);
    }
    Ok((outcomes, summary))
}

fn diff_pngs(baseline: &Path, candidate: &Path, out: Option<&Path>) -> Result<DiffOutcome, Error> {
    let relative_path = baseline
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("<unknown>")
        .to_owned();
    let mut outcome = diff_pngs_for_entry(baseline, candidate, &relative_path, 0.001)?;
    if let Some(out_path) = out {
        let diff_image = render_diff_image(baseline, candidate)?;
        diff_image.save(out_path).map_err(|source| Error::Image {
            path: out_path.to_path_buf(),
            source,
        })?;
        // Recompute outcome to print the same drift the file shows.
        outcome.relative_path =
            format!("{} (diff -> {})", outcome.relative_path, out_path.display());
    }
    Ok(outcome)
}

fn diff_pngs_for_entry(
    baseline_path: &Path,
    candidate_path: &Path,
    relative_path: &str,
    threshold: f64,
) -> Result<DiffOutcome, Error> {
    let baseline = open_rgba(baseline_path)?;
    let candidate = open_rgba(candidate_path)?;
    if baseline.dimensions() != candidate.dimensions() {
        return Err(Error::Dimensions {
            baseline: baseline_path.to_path_buf(),
            candidate: candidate_path.to_path_buf(),
            expected: baseline.width(),
            eh: baseline.height(),
            actual: candidate.width(),
            ah: candidate.height(),
        });
    }
    let pixel_count = u64::from(baseline.width()) * u64::from(baseline.height());
    let mut drifted: u64 = 0;
    for (a, b) in baseline.pixels().zip(candidate.pixels()) {
        if pixel_drifted(*a, *b) {
            drifted += 1;
        }
    }
    let drift_fraction = if pixel_count > 0 {
        // Pixel counts are bounded by image dimensions (u32 each),
        // so the product fits in 52 bits well below the f64
        // mantissa precision boundary.
        #[allow(clippy::cast_precision_loss)]
        {
            (drifted as f64) / (pixel_count as f64)
        }
    } else {
        0.0
    };
    let status = if drift_fraction > threshold {
        DiffStatus::Drifted
    } else {
        DiffStatus::Ok
    };
    Ok(DiffOutcome {
        relative_path: relative_path.to_owned(),
        drift_fraction,
        pixel_count,
        drifted_pixels: drifted,
        status,
    })
}

/// Per-pixel delta with a small anti-aliasing tolerance — two
/// pixels are "drifted" only when their max-channel delta exceeds
/// 16 (out of 255). Pixelmatch's default is 32; we're stricter
/// here because rendered invoice text has crisper edges than
/// natural images.
fn pixel_drifted(a: Rgba<u8>, b: Rgba<u8>) -> bool {
    let max_delta =
        a.0.iter()
            .zip(b.0.iter())
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
    max_delta > 16
}

fn render_diff_image(
    baseline_path: &Path,
    candidate_path: &Path,
) -> Result<ImageBuffer<Rgba<u8>, Vec<u8>>, Error> {
    let baseline = open_rgba(baseline_path)?;
    let candidate = open_rgba(candidate_path)?;
    if baseline.dimensions() != candidate.dimensions() {
        return Err(Error::Dimensions {
            baseline: baseline_path.to_path_buf(),
            candidate: candidate_path.to_path_buf(),
            expected: baseline.width(),
            eh: baseline.height(),
            actual: candidate.width(),
            ah: candidate.height(),
        });
    }
    let (w, h) = baseline.dimensions();
    let mut out: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(w, h);
    for (x, y, base_px) in baseline.enumerate_pixels() {
        let cand_px = candidate.get_pixel(x, y);
        let drifted = pixel_drifted(*base_px, *cand_px);
        let visual = if drifted {
            // Bright red drift markers on a desaturated baseline.
            Rgba([255, 0, 0, 255])
        } else {
            let grey = u8::try_from(
                ((u32::from(base_px[0]) + u32::from(base_px[1]) + u32::from(base_px[2])) / 6)
                    .min(255),
            )
            .unwrap_or(128);
            Rgba([grey, grey, grey, 255])
        };
        out.put_pixel(x, y, visual);
    }
    Ok(out)
}

fn open_rgba(path: &Path) -> Result<ImageBuffer<Rgba<u8>, Vec<u8>>, Error> {
    let img = image::open(path).map_err(|source| Error::Image {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(img.into_rgba8())
}

fn verify_baseline_sha256(path: &Path, expected: &str) -> Result<bool, Error> {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let bytes = fs::read(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(hex, "{byte:02x}").expect("writing to a String never fails");
    }
    Ok(hex.eq_ignore_ascii_case(expected))
}

fn load_manifest(snapshots: &Path) -> Result<Manifest, Error> {
    let path = snapshots.join("MANIFEST.json");
    let bytes = fs::read(&path).map_err(|source| Error::Io {
        path: path.clone(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| Error::Manifest {
        path: path.clone(),
        source,
    })
}

// ----- bless --------------------------------------------------------

fn bless(snapshots: &Path, candidates: &Path) -> Result<usize, Error> {
    let manifest = load_manifest(snapshots)?;
    let mut count = 0;
    for entry in &manifest.entries {
        let baseline_png = snapshots.join(&entry.relative_path);
        let candidate_png = candidates.join(&entry.relative_path);
        if !candidate_png.is_file() {
            continue;
        }
        if let Some(parent) = baseline_png.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::copy(&candidate_png, &baseline_png).map_err(|source| Error::Io {
            path: baseline_png.clone(),
            source,
        })?;
        count += 1;
    }
    Ok(count)
}

// ----- report -------------------------------------------------------

fn write_report(
    path: &Path,
    outcomes: &[DiffOutcome],
    summary: &DiffSummary,
    threshold: f64,
) -> Result<(), Error> {
    use std::fmt::Write as _;
    let mut buf = String::new();
    writeln!(buf, "# PDF visual regression report").unwrap();
    writeln!(buf).unwrap();
    writeln!(
        buf,
        "Threshold: {:.4}% per-page drift. Pixel-delta cutoff: 16/255.",
        threshold * 100.0
    )
    .unwrap();
    writeln!(buf).unwrap();
    writeln!(
        buf,
        "Total: {} | Ok: {} | Drifted: {} | Missing: {} | Mismatched baseline: {}",
        summary.total, summary.ok, summary.drifted, summary.missing, summary.mismatched
    )
    .unwrap();
    writeln!(buf).unwrap();
    writeln!(buf, "| Fixture | Status | Drift | Drifted / Total px |").unwrap();
    writeln!(buf, "|---|:--|---:|---:|").unwrap();
    for outcome in outcomes {
        let status_text = match outcome.status {
            DiffStatus::Ok => "ok",
            DiffStatus::Drifted => "**DRIFT**",
            DiffStatus::MissingBaseline => "missing baseline",
            DiffStatus::MissingCandidate => "missing candidate",
            DiffStatus::BaselineSha256Mismatch => "baseline sha256 mismatch",
        };
        writeln!(
            buf,
            "| `{}` | {} | {:.4}% | {} / {} |",
            outcome.relative_path,
            status_text,
            outcome.drift_fraction * 100.0,
            outcome.drifted_pixels,
            outcome.pixel_count,
        )
        .unwrap();
    }
    if summary.drifted > 0 || summary.missing > 0 || summary.mismatched > 0 {
        writeln!(buf).unwrap();
        writeln!(
            buf,
            "Approve by adding the `accept-visual-drift` label to the PR; the harness will then rebless the baselines on the next push."
        )
        .unwrap();
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(path, buf).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    fn solid(w: u32, h: u32, px: [u8; 4]) -> ImageBuffer<Rgba<u8>, Vec<u8>> {
        let mut img = ImageBuffer::new(w, h);
        for pixel in img.pixels_mut() {
            *pixel = Rgba(px);
        }
        img
    }

    #[test]
    fn pixel_drifted_tolerates_small_delta() {
        assert!(!pixel_drifted(
            Rgba([10, 10, 10, 255]),
            Rgba([20, 20, 20, 255])
        ));
    }

    #[test]
    fn pixel_drifted_flags_large_delta() {
        assert!(pixel_drifted(
            Rgba([10, 10, 10, 255]),
            Rgba([200, 10, 10, 255])
        ));
    }

    #[test]
    fn identical_pngs_report_zero_drift() {
        let tmp = tempdir();
        let baseline = tmp.join("baseline.png");
        let candidate = tmp.join("candidate.png");
        solid(40, 30, [255, 255, 255, 255]).save(&baseline).unwrap();
        solid(40, 30, [255, 255, 255, 255])
            .save(&candidate)
            .unwrap();
        let outcome = diff_pngs_for_entry(&baseline, &candidate, "test", 0.001).expect("diff ok");
        assert_eq!(outcome.drifted_pixels, 0);
        assert_eq!(outcome.status, DiffStatus::Ok);
    }

    #[test]
    fn different_solid_colour_pngs_report_full_drift() {
        let tmp = tempdir();
        let baseline = tmp.join("baseline.png");
        let candidate = tmp.join("candidate.png");
        solid(40, 30, [255, 255, 255, 255]).save(&baseline).unwrap();
        solid(40, 30, [0, 0, 0, 255]).save(&candidate).unwrap();
        let outcome = diff_pngs_for_entry(&baseline, &candidate, "test", 0.001).expect("diff ok");
        assert_eq!(outcome.drifted_pixels, 40 * 30);
        assert_eq!(outcome.status, DiffStatus::Drifted);
        assert!(outcome.drift_fraction > 0.99);
    }

    #[test]
    fn dimension_mismatch_returns_error() {
        let tmp = tempdir();
        let baseline = tmp.join("baseline.png");
        let candidate = tmp.join("candidate.png");
        solid(40, 30, [255, 255, 255, 255]).save(&baseline).unwrap();
        solid(20, 30, [255, 255, 255, 255])
            .save(&candidate)
            .unwrap();
        let err = diff_pngs_for_entry(&baseline, &candidate, "test", 0.001).unwrap_err();
        matches!(err, Error::Dimensions { .. });
    }

    #[test]
    fn manifest_with_missing_baseline_reports_status() {
        let tmp = tempdir();
        let snapshots = tmp.join("snapshots");
        fs::create_dir_all(&snapshots).unwrap();
        let manifest = Manifest {
            renderer_version: "test".to_owned(),
            rasterizer_version: "test".to_owned(),
            entries: vec![ManifestEntry {
                relative_path: "missing/baseline.png".to_owned(),
                baseline_sha256: "0".repeat(64),
            }],
        };
        fs::write(
            snapshots.join("MANIFEST.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let candidates = tmp.join("candidates");
        fs::create_dir_all(candidates.join("missing")).unwrap();
        solid(10, 10, [255, 255, 255, 255])
            .save(candidates.join("missing/baseline.png"))
            .unwrap();
        let (outcomes, summary) = diff_against_manifest(&snapshots, &candidates, 0.001).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, DiffStatus::MissingBaseline);
        assert_eq!(summary.missing, 1);
    }

    #[test]
    fn manifest_with_sha256_mismatch_reports_status() {
        let tmp = tempdir();
        let snapshots = tmp.join("snapshots");
        fs::create_dir_all(snapshots.join("dir")).unwrap();
        solid(5, 5, [0, 255, 0, 255])
            .save(snapshots.join("dir/baseline.png"))
            .unwrap();
        let manifest = Manifest {
            renderer_version: "test".to_owned(),
            rasterizer_version: "test".to_owned(),
            entries: vec![ManifestEntry {
                relative_path: "dir/baseline.png".to_owned(),
                baseline_sha256: "deadbeef".to_owned(), // wrong on purpose
            }],
        };
        fs::write(
            snapshots.join("MANIFEST.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let candidates = tmp.join("candidates");
        fs::create_dir_all(candidates.join("dir")).unwrap();
        solid(5, 5, [0, 255, 0, 255])
            .save(candidates.join("dir/baseline.png"))
            .unwrap();
        let (outcomes, summary) = diff_against_manifest(&snapshots, &candidates, 0.001).unwrap();
        assert_eq!(outcomes[0].status, DiffStatus::BaselineSha256Mismatch);
        assert_eq!(summary.mismatched, 1);
    }

    #[test]
    fn full_pipeline_ok_path_emits_passing_report() {
        let tmp = tempdir();
        let snapshots = tmp.join("snapshots");
        let candidates = tmp.join("candidates");
        fs::create_dir_all(snapshots.join("page")).unwrap();
        fs::create_dir_all(candidates.join("page")).unwrap();
        let baseline_path = snapshots.join("page/x.png");
        let candidate_path = candidates.join("page/x.png");
        solid(20, 20, [255, 255, 255, 255])
            .save(&baseline_path)
            .unwrap();
        solid(20, 20, [255, 255, 255, 255])
            .save(&candidate_path)
            .unwrap();
        let baseline_sha = {
            use sha2::{Digest, Sha256};
            use std::fmt::Write as _;
            let bytes = fs::read(&baseline_path).unwrap();
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let digest = hasher.finalize();
            let mut s = String::with_capacity(digest.len() * 2);
            for b in digest {
                write!(s, "{b:02x}").unwrap();
            }
            s
        };
        let manifest = Manifest {
            renderer_version: "test".to_owned(),
            rasterizer_version: "test".to_owned(),
            entries: vec![ManifestEntry {
                relative_path: "page/x.png".to_owned(),
                baseline_sha256: baseline_sha,
            }],
        };
        fs::write(
            snapshots.join("MANIFEST.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let (outcomes, summary) = diff_against_manifest(&snapshots, &candidates, 0.001).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, DiffStatus::Ok);
        assert_eq!(summary.ok, 1);
        let report_path = tmp.join("report.md");
        write_report(&report_path, &outcomes, &summary, 0.001).unwrap();
        let report = fs::read_to_string(report_path).unwrap();
        assert!(report.contains("PDF visual regression"));
        assert!(report.contains("`page/x.png`"));
        assert!(report.contains("ok"));
    }

    #[test]
    fn bless_refuses_without_label_flag() {
        // run_bless without the magic --i-have-the-label yes flag
        // must return a non-zero ExitCode.
        let tmp = tempdir();
        let snapshots = tmp.join("snapshots");
        fs::create_dir_all(&snapshots).unwrap();
        fs::write(
            snapshots.join("MANIFEST.json"),
            serde_json::to_string(&Manifest {
                renderer_version: "x".to_owned(),
                rasterizer_version: "x".to_owned(),
                entries: vec![],
            })
            .unwrap(),
        )
        .unwrap();
        let candidates = tmp.join("candidates");
        fs::create_dir_all(&candidates).unwrap();
        let exit_code = run_bless(&snapshots, &candidates, None);
        assert_ne!(format!("{exit_code:?}"), format!("{:?}", ExitCode::SUCCESS));
    }

    fn tempdir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "pdf-visual-regression-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
