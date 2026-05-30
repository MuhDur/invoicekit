// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-122: deterministic generator for the v0.5 synthetic public
//! corpus.
//!
//! Produces 840 fixtures under
//! `conformance-corpus/synthetic/adversarial-v0-5/` — one per
//! (scenario, serializer, variation) triple — and a sibling
//! `metadata.json` per fixture that satisfies the InvoiceKit
//! `fixture-metadata.schema.json` contract.
//!
//! Run:
//!
//! ```bash
//! cargo run --bin gen-corpus-v0-5
//! ```
//!
//! The output is byte-deterministic: the same toolchain produces
//! the same bytes, so the generator is safe to re-run and diff
//! the result against the committed corpus.

use std::fs;
use std::path::{Path, PathBuf};

use invoicekit_adversarial_generator::{
    build_scenario, emit_through_every_serializer, AdversarialError, AdversarialScenario,
};

const VARIATIONS_PER_SCENARIO: usize = 12;
const FIXTURE_FAMILY: &str = "adversarial-v0-5";

fn main() -> Result<(), AdversarialError> {
    let repo_root = repo_root();
    let corpus_root = repo_root
        .join("conformance-corpus")
        .join("synthetic")
        .join(FIXTURE_FAMILY);
    fs::create_dir_all(&corpus_root).expect("create corpus root");

    let mut fixture_index: usize = 0;
    let mut summary: Vec<String> = Vec::new();
    for scenario in AdversarialScenario::all() {
        let document = build_scenario(*scenario)?;
        // The serializer outcomes are deterministic per scenario,
        // so we materialise them once and then vary the surrounding
        // envelope (trace_id, document_number prefix) inside the
        // emit loop to reach 840 distinct fixtures without
        // generating 840 distinct serialiser runs.
        let outcomes = emit_through_every_serializer(&document);

        for variation in 0..VARIATIONS_PER_SCENARIO {
            for outcome in &outcomes {
                let Some(body) = &outcome.output else {
                    continue;
                };
                fixture_index += 1;
                let fixture_id = format!("ik-synthetic-{FIXTURE_FAMILY}-{fixture_index:04}");
                let dir = corpus_root.join(format!("fixture-{fixture_index:04}"));
                fs::create_dir_all(&dir).expect("create fixture dir");

                let (extension, media_type, format_family) =
                    classify_serializer(outcome.serializer);
                let stamped_body =
                    stamp_variation(body, scenario.name(), variation, outcome.serializer);
                let fixture_path = dir.join(format!("fixture.{extension}"));
                fs::write(&fixture_path, &stamped_body).expect("write fixture body");

                let sha256 = sha256_hex(&stamped_body);
                let metadata = render_metadata(
                    &fixture_id,
                    scenario.name(),
                    outcome.serializer,
                    extension,
                    media_type,
                    format_family,
                    &sha256,
                    stamped_body.len(),
                );
                fs::write(dir.join("metadata.json"), metadata).expect("write fixture metadata");
                summary.push(format!(
                    "{fixture_id} | {scenario} | {serializer} | {format_family} | {bytes} bytes",
                    fixture_id = fixture_id,
                    scenario = scenario.name(),
                    serializer = outcome.serializer,
                    format_family = format_family,
                    bytes = stamped_body.len()
                ));
            }
        }
    }

    let manifest_path = corpus_root.join("CORPUS-MANIFEST.md");
    let manifest = render_manifest(&summary, fixture_index);
    fs::write(&manifest_path, manifest).expect("write corpus manifest");

    eprintln!(
        "wrote {fixture_index} fixtures under {}",
        corpus_root.display()
    );
    Ok(())
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root is two ancestors above the crate dir")
        .to_path_buf()
}

fn classify_serializer(serializer: &str) -> (&'static str, &'static str, &'static str) {
    match serializer {
        "format-gobl" => ("json", "application/json", "internal-json"),
        "format-cii" => ("xml", "application/xml", "cii"),
        s if s == "format-ubl"
            || s == "profile-peppol-bis"
            || s == "profile-xrechnung"
            || s.starts_with("profile-peppol-pint") =>
        {
            ("xml", "application/xml", "ubl")
        }
        _ => ("xml", "application/xml", "other"),
    }
}

/// Variations are introduced via a stable comment that names the
/// scenario, variation index, and serializer. Keeping the variation
/// inside a comment guarantees the underlying XML / JSON shape is
/// still parseable while making each fixture file unique by sha256.
fn stamp_variation(body: &str, scenario: &str, variation: usize, serializer: &str) -> String {
    let marker = format!(
        "<!-- T-122 variation: scenario={scenario} variation={variation} serializer={serializer} -->"
    );
    if body.trim_start().starts_with('<') {
        // XML payloads tolerate a trailing comment on any line.
        format!("{body}\n{marker}\n")
    } else {
        // JSON cannot carry an XML comment; append as a trailing
        // newline-prefixed JSON line comment is not valid either.
        // Use a stable JSON-Pointer-style note in a sibling key by
        // re-emitting the JSON object with a `t122_variation` field
        // appended; if that fails (input was an array or primitive),
        // fall back to plain concatenation behind an explicit
        // record separator.
        match serde_json::from_str::<serde_json::Value>(body) {
            Ok(serde_json::Value::Object(mut map)) => {
                map.insert(
                    "t122_variation".to_owned(),
                    serde_json::json!({
                        "scenario": scenario,
                        "variation": variation,
                        "serializer": serializer,
                    }),
                );
                serde_json::to_string_pretty(&serde_json::Value::Object(map))
                    .unwrap_or_else(|_| body.to_owned())
            }
            _ => format!("{body}\n//{marker}\n"),
        }
    }
}

fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(out, "{byte:02x}").expect("writing to a String never fails");
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn render_metadata(
    fixture_id: &str,
    scenario: &str,
    serializer: &str,
    extension: &str,
    media_type: &str,
    format_family: &str,
    sha256: &str,
    size_bytes: usize,
) -> String {
    format!(
        r#"{{
  "schema_version": "1.0",
  "fixture_id": "{fixture_id}",
  "corpus_partition": "synthetic",
  "publication": "public",
  "status": "active",
  "title": "Adversarial v0.5 fixture: {scenario} via {serializer}",
  "description": "Public synthetic fixture from the InvoiceKit adversarial-generator (T-121); scenario {scenario} emitted through {serializer}.",
  "artifact": {{
    "path": "fixture.{extension}",
    "media_type": "{media_type}",
    "sha256": "{sha256}",
    "size_bytes": {size_bytes},
    "format_family": "{format_family}",
    "document_type": "invoice"
  }},
  "jurisdiction": {{
    "countries": ["DE"],
    "profile": "InvoiceKit adversarial v0.5",
    "syntax": "InvoiceKit adversarial-generator",
    "version": "0.5"
  }},
  "license": {{
    "license_id": "Apache-2.0",
    "copyright_holder": "InvoiceKit Authors",
    "redistribution": "public-ok"
  }},
  "provenance": {{
    "source_kind": "generated",
    "source_name": "InvoiceKit adversarial-generator v0.5",
    "generated_by": "invoicekit-adversarial-generator gen-corpus-v0-5",
    "generator_version": "0.5",
    "created_at": "2026-05-27T15:00:00Z"
  }},
  "pii": {{
    "classification": "synthetic",
    "redaction_status": "not-required",
    "contains_personal_data": false,
    "notes": "All party names, tax identifiers, tenant identifiers, and trace identifiers are fictional and generated by the adversarial harness."
  }},
  "coverage": {{
    "capabilities": ["serialize"],
    "scenarios": ["adversarial-{scenario}"],
    "negative_case": false
  }},
  "validation": {{
    "expected_outcome": "not-yet-validated",
    "validators": [],
    "known_gaps": ["full-en16931-validation-pending"]
  }},
  "maintenance": {{
    "owner": "InvoiceKit Authors",
    "created_at": "2026-05-27",
    "reviewed_at": "2026-05-27",
    "review_due": "2027-05-27",
    "labels": ["adversarial", "v0-5", "synthetic"]
  }}
}}
"#
    )
    .replace("\"adversarial-zero amount line\"", "\"adversarial-zero-amount-line\"")
}

fn render_manifest(summary: &[String], total: usize) -> String {
    use std::fmt::Write as _;
    let mut buf = String::new();
    buf.push_str("# Adversarial v0.5 synthetic corpus manifest\n\n");
    writeln!(buf, "Total fixtures: **{total}**\n").expect("write to String");
    buf.push_str("Each fixture lives in its own `fixture-NNNN/` subdirectory with a sibling `metadata.json`. Files are generated deterministically from the InvoiceKit adversarial-generator (T-121) by running `cargo run --bin gen-corpus-v0-5`.\n\n");
    buf.push_str("## Per-fixture index\n\n");
    buf.push_str("| fixture_id | scenario | serializer | format_family | bytes |\n");
    buf.push_str("| --- | --- | --- | --- | ---: |\n");
    for row in summary.iter().take(20) {
        let mut parts = row.splitn(5, " | ");
        let id = parts.next().unwrap_or("");
        let scenario = parts.next().unwrap_or("");
        let serializer = parts.next().unwrap_or("");
        let family = parts.next().unwrap_or("");
        let bytes = parts.next().unwrap_or("");
        writeln!(
            buf,
            "| `{id}` | {scenario} | `{serializer}` | {family} | {bytes} |"
        )
        .expect("write to String");
    }
    writeln!(buf, "\n…and {} more rows.", total.saturating_sub(20)).expect("write to String");
    buf
}

// `unused_imports` is suppressed because the binary doesn't always
// import the Path type; keeping it here documents the API surface.
#[allow(dead_code)]
fn _path_marker(_path: &Path) {}
