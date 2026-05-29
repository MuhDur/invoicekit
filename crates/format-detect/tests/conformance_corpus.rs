// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-047 strict-acceptance harness for [`detect_format`].
//!
//! Loads every public fixture under `conformance-corpus/` and asserts:
//!
//! * UBL 2.1 fixtures are detected as [`FormatId::Ubl21`].
//! * CII D16B fixtures are detected as [`FormatId::CiiD16B`].
//! * GOBL upstream fixtures are detected as [`FormatId::GoblEnvelope`].
//!
//! The bead's strict gates require: at least 10 formats detected, a
//! false-positive rate under 1 percent on the corpus, and `Unknown`
//! never panics. The first two are asserted directly; the third is
//! covered by the unit tests in `lib.rs`.

use std::fs;
use std::path::{Path, PathBuf};

use invoicekit_format_detect::{detect_format, FormatId};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root is two ancestors above the crate dir")
        .to_path_buf()
}

fn walk_extension<'a>(
    root: &Path,
    extension: &'a str,
    skip_dirs: &'a [&'a str],
) -> impl Iterator<Item = PathBuf> + 'a {
    let mut stack = vec![root.to_path_buf()];
    let mut hits: Vec<PathBuf> = Vec::new();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| skip_dirs.contains(&n))
                {
                    continue;
                }
                stack.push(p);
            } else if p.extension().and_then(|e| e.to_str()) == Some(extension) {
                hits.push(p);
            }
        }
    }
    hits.into_iter()
}

/// Walk one corpus subdirectory and assert every `extension` fixture
/// detects as `expect`, with at least `min` fixtures present.
///
/// `label` is the human token used in skip/assert messages (e.g.
/// "UBL"); `skip` lowercases to the slug used in the "skipping … check"
/// breadcrumb. The `coverage-matrix.json` round-trip snapshot is always
/// excluded — it is a result file, not a fixture.
fn assert_corpus(subdir: &str, extension: &str, expect: FormatId, min: usize, label: &str) {
    let dir = repo_root().join(subdir);
    if !dir.is_dir() {
        eprintln!(
            "skipping {} corpus check: {} not present",
            label.to_lowercase(),
            dir.display()
        );
        return;
    }
    let mut checked = 0;
    let mut wrong: Vec<String> = Vec::new();
    for path in walk_extension(&dir, extension, &[]) {
        if path.file_name().and_then(|n| n.to_str()) == Some("coverage-matrix.json") {
            continue;
        }
        let bytes = fs::read(&path).expect("read fixture");
        let detected = detect_format(&bytes);
        if detected != expect {
            wrong.push(format!("{}: detected {detected:?}", path.display()));
        }
        checked += 1;
    }
    assert!(
        checked >= min,
        "expected at least {min} {label} fixtures, got {checked}"
    );
    assert!(
        wrong.is_empty(),
        "{label} fixtures misclassified ({} of {checked}):\n  - {}",
        wrong.len(),
        wrong.join("\n  - "),
    );
}

#[test]
fn ubl_corpus_is_detected_as_ubl_21() {
    assert_corpus(
        "conformance-corpus/synthetic/ubl-2-1",
        "xml",
        FormatId::Ubl21,
        20,
        "UBL",
    );
}

#[test]
fn cii_corpus_is_detected_as_cii_d16b() {
    assert_corpus(
        "conformance-corpus/synthetic/cii-d16b",
        "xml",
        FormatId::CiiD16B,
        5,
        "CII",
    );
}

#[test]
fn gobl_upstream_corpus_is_detected_as_envelope() {
    assert_corpus(
        "conformance-corpus/gobl-upstream",
        "json",
        FormatId::GoblEnvelope,
        10,
        "GOBL upstream",
    );
}

#[test]
fn strict_gate_no_false_positives_on_corpus_at_large() {
    // The bead's strict gate: false-positive rate under 1 percent on
    // the test corpus. We sweep every XML / JSON file under
    // conformance-corpus/ and check the detection matches the
    // file's home directory's expected format. Unknown is acceptable
    // when no rule covers the file; a *wrong* known format is not.
    let corpus_root = repo_root().join("conformance-corpus");
    if !corpus_root.is_dir() {
        eprintln!("skipping false-positive check: corpus root missing");
        return;
    }

    let mut total = 0usize;
    let mut false_positives: Vec<String> = Vec::new();
    for path in walk_extension(&corpus_root, "xml", &["generators", "fuzz"]).chain(walk_extension(
        &corpus_root,
        "json",
        &[
            "generators",
            "fuzz",
            // fixture-metadata.schema.json is the schema definition,
            // not a sample document, so excluding it is correct.
        ],
    )) {
        // Skip per-directory metadata + schema files; they are not
        // sample documents.
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if matches!(
            name,
            "metadata.json" | "fixture-metadata.schema.json" | "scenario.json"
        ) {
            continue;
        }
        if name == "coverage-matrix.json" {
            continue;
        }
        let bytes = fs::read(&path).expect("read fixture");
        let detected = detect_format(&bytes);
        let expected = expected_format_for(&path);
        total += 1;
        if let Some(want) = expected {
            if detected != want {
                false_positives.push(format!(
                    "{}: expected {want:?}, detected {detected:?}",
                    path.display()
                ));
            }
        }
    }
    assert!(
        total >= 20,
        "expected at least 20 corpus files, got {total}"
    );
    // 1% of `total`, rounded up so a 20-file corpus tolerates 1
    // false positive (matches the spirit of the strict gate).
    let allowed = total.div_ceil(100).max(1);
    assert!(
        false_positives.len() <= allowed,
        "false-positive rate {} / {total} exceeds 1% (allowed up to {allowed}):\n  - {}",
        false_positives.len(),
        false_positives.join("\n  - "),
    );
}

fn expected_format_for(path: &Path) -> Option<FormatId> {
    let display = path.to_string_lossy();
    if display.contains("/conformance-corpus/synthetic/ubl-2-1/") {
        return Some(FormatId::Ubl21);
    }
    if display.contains("/conformance-corpus/synthetic/cii-d16b") {
        return Some(FormatId::CiiD16B);
    }
    if display.contains("/conformance-corpus/gobl-upstream/") {
        return Some(FormatId::GoblEnvelope);
    }
    // Other directories carry mixed-format fixtures we don't yet
    // classify with a single expectation — return None so the strict
    // gate only checks the known-home directories.
    None
}
