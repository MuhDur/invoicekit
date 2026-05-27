// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! wcep: round-trip GOBL upstream test fixtures through the codec.
//!
//! Fixtures live under `conformance-corpus/gobl-upstream/`. Each one
//! is a GOBL "envelope" with a `head` and a `doc` payload. The
//! per-fixture round-trip is:
//!
//!   1. extract `envelope.doc`
//!   2. call `from_gobl(doc)` — gives us a JSON-shaped IR document
//!   3. call `CommercialDocument::try_from_value(...)` — validates
//!      the IR shape we reconstructed
//!   4. call `to_gobl(ir)` — projects the IR back to GOBL JSON
//!   5. compare a small set of stable "anchor" fields between the
//!      input GOBL doc and the round-tripped GOBL doc
//!
//! Per the bead, skipped fixtures need a documented reason. The
//! coverage matrix is asserted byte-stable against
//! `coverage-matrix.json` in the same directory so a future codec
//! change that improves or degrades coverage shows up in review.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use invoicekit_format_gobl::{from_gobl, to_gobl};
use invoicekit_ir::CommercialDocument;
use serde_json::{json, Value};

#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    /// All anchor fields survived the round trip.
    RoundTripOk,
    /// `from_gobl` reconstructed an IR shape but
    /// `CommercialDocument::try_from_value` rejected it. The bead's
    /// follow-up coverage matrix records this without failing the
    /// build — the trust toolkit reports lossiness, it doesn't paper
    /// over it.
    SkippedIrValidation,
    /// `from_gobl` itself returned an error.
    SkippedFromGoblError,
    /// `to_gobl` failed after a successful inbound parse.
    SkippedToGoblError,
    /// Round-tripped, but at least one anchor field drifted. Lossy
    /// fields are listed in `lossy_anchors`.
    LossyRoundTrip,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct FixtureResult {
    fixture: String,
    outcome: Outcome,
    /// Free-text reason populated when outcome is a Skipped* variant.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    reason: String,
    /// Anchor fields whose value drifted across the round trip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    lossy_anchors: Vec<String>,
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root is two ancestors above the crate dir")
        .join("conformance-corpus/gobl-upstream")
}

fn extract_doc(envelope: &Value) -> Option<&Value> {
    envelope.get("doc").or(Some(envelope))
}

/// Anchor fields are the small, stable subset every GOBL invoice
/// declares. We compare these byte-for-byte across the round trip.
/// Tax-summary / extensions / metadata are deliberately *not* anchors
/// — they are first-class lossy because GOBL carries fields InvoiceKit
/// doesn't model (e.g. `addons`, `$regime`, per-line `discounts`).
fn anchor_diff(original: &Value, round_tripped: &Value) -> Vec<String> {
    let anchors = [
        "type",
        "code",
        "issue_date",
        "currency",
        "supplier/name",
        "customer/name",
    ];
    let mut diffs = Vec::new();
    for path in anchors {
        let original_value = lookup(original, path);
        let rt_value = lookup(round_tripped, path);
        if original_value != rt_value {
            diffs.push(format!(
                "{path} (was {original:?}, became {rt:?})",
                original = original_value.unwrap_or(&Value::Null),
                rt = rt_value.unwrap_or(&Value::Null),
            ));
        }
    }
    // line count
    let original_lines = original
        .get("lines")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let rt_lines = round_tripped
        .get("lines")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if original_lines != rt_lines {
        diffs.push(format!(
            "lines/* count (was {original_lines}, became {rt_lines})"
        ));
    }
    diffs
}

fn lookup<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cursor = value;
    for segment in path.split('/') {
        cursor = cursor.get(segment)?;
    }
    Some(cursor)
}

fn evaluate_fixture(path: &Path, contents: &str) -> FixtureResult {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap()
        .to_owned();
    let envelope: Value = match serde_json::from_str(contents) {
        Ok(v) => v,
        Err(e) => {
            return FixtureResult {
                fixture: name,
                outcome: Outcome::SkippedFromGoblError,
                reason: format!("fixture is not valid JSON: {e}"),
                lossy_anchors: Vec::new(),
            };
        }
    };
    let Some(gobl_doc) = extract_doc(&envelope) else {
        return FixtureResult {
            fixture: name,
            outcome: Outcome::SkippedFromGoblError,
            reason: "envelope has no `doc` field".into(),
            lossy_anchors: Vec::new(),
        };
    };
    let parsed = match from_gobl(gobl_doc) {
        Ok(env) => env,
        Err(e) => {
            return FixtureResult {
                fixture: name,
                outcome: Outcome::SkippedFromGoblError,
                reason: format!("{e}"),
                lossy_anchors: Vec::new(),
            };
        }
    };
    let ir = match CommercialDocument::try_from_value(parsed.document) {
        Ok(doc) => doc,
        Err(e) => {
            return FixtureResult {
                fixture: name,
                outcome: Outcome::SkippedIrValidation,
                reason: format!("{e}"),
                lossy_anchors: Vec::new(),
            };
        }
    };
    let projected = match to_gobl(&ir) {
        Ok(env) => env,
        Err(e) => {
            return FixtureResult {
                fixture: name,
                outcome: Outcome::SkippedToGoblError,
                reason: format!("{e}"),
                lossy_anchors: Vec::new(),
            };
        }
    };
    let diffs = anchor_diff(gobl_doc, &projected.document);
    if diffs.is_empty() {
        FixtureResult {
            fixture: name,
            outcome: Outcome::RoundTripOk,
            reason: String::new(),
            lossy_anchors: Vec::new(),
        }
    } else {
        FixtureResult {
            fixture: name,
            outcome: Outcome::LossyRoundTrip,
            reason: String::new(),
            lossy_anchors: diffs,
        }
    }
}

/// The 20 upstream GOBL fixtures must produce a snapshot-stable
/// coverage matrix. A bead that changes the codec should either match
/// the committed matrix (no behaviour change) or update it explicitly
/// (intentional improvement / regression).
#[test]
fn gobl_upstream_corpus_round_trips() {
    let dir = fixtures_dir();
    assert!(
        dir.is_dir(),
        "expected GOBL upstream corpus at {}",
        dir.display()
    );

    let mut results: Vec<FixtureResult> = Vec::new();
    for entry in fs::read_dir(&dir).expect("read corpus directory") {
        let path = entry.expect("read corpus entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|s| s.starts_with("coverage-matrix"))
        {
            continue;
        }
        let contents = fs::read_to_string(&path).expect("read fixture");
        results.push(evaluate_fixture(&path, &contents));
    }
    results.sort_by(|a, b| a.fixture.cmp(&b.fixture));

    assert_eq!(
        results.len(),
        20,
        "expected exactly 20 GOBL upstream fixtures, found {}",
        results.len()
    );

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for r in &results {
        *counts.entry(format!("{:?}", r.outcome)).or_default() += 1;
    }
    let summary = json!({
        "fixture_count": results.len(),
        "outcomes": counts,
        "fixtures": results,
    });

    let snapshot_path = dir.join("coverage-matrix.json");
    let expected: Value = serde_json::from_str(
        &fs::read_to_string(&snapshot_path)
            .unwrap_or_else(|_| panic!("missing snapshot at {}", snapshot_path.display())),
    )
    .expect("snapshot JSON parses");

    assert_eq!(
        summary, expected,
        "coverage matrix drifted. Re-record by running:\n  \
         cargo test -p invoicekit-format-gobl --test upstream_corpus -- --ignored bless\n\
         then commit conformance-corpus/gobl-upstream/coverage-matrix.json.\n\n\
         actual: {summary:#}\nexpected: {expected:#}"
    );

    let round_trip_ok = results
        .iter()
        .filter(|r| r.outcome == Outcome::RoundTripOk || r.outcome == Outcome::LossyRoundTrip)
        .count();
    assert!(
        round_trip_ok >= 10,
        "round-trip floor: at least 10 of 20 GOBL upstream fixtures must \
         survive the codec round trip without an inbound parse failure. Got {round_trip_ok}."
    );
}

/// Bless helper: run with
/// `cargo test -p invoicekit-format-gobl --test upstream_corpus
///  ignored_bless_coverage_matrix -- --ignored --nocapture` to
/// regenerate `coverage-matrix.json` from the live codec output.
#[test]
#[ignore = "run with --ignored to regenerate the coverage matrix snapshot"]
fn ignored_bless_coverage_matrix() {
    let dir = fixtures_dir();
    let mut results: Vec<FixtureResult> = Vec::new();
    for entry in fs::read_dir(&dir).expect("read corpus directory") {
        let path = entry.expect("read corpus entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|s| s.starts_with("coverage-matrix"))
        {
            continue;
        }
        let contents = fs::read_to_string(&path).expect("read fixture");
        results.push(evaluate_fixture(&path, &contents));
    }
    results.sort_by(|a, b| a.fixture.cmp(&b.fixture));

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for r in &results {
        *counts.entry(format!("{:?}", r.outcome)).or_default() += 1;
    }
    let summary = json!({
        "fixture_count": results.len(),
        "outcomes": counts,
        "fixtures": results,
    });

    let snapshot_path = dir.join("coverage-matrix.json");
    fs::write(&snapshot_path, format!("{summary:#}\n")).expect("write snapshot");
    eprintln!("wrote coverage matrix to {}", snapshot_path.display());
}
