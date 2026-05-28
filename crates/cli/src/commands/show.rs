// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit show` runner.
//!
//! Read-only bundle inspection. Reads a `.ikb` evidence bundle
//! and prints its manifest plus per-artefact summary.
//!
//! Pairs with `pack` / `unpack` / `verify` / `replay`:
//!
//! * `pack`   — produce a bundle.
//! * `show`   — inspect it without writing anything to disk.
//! * `verify` — confirm integrity.
//! * `replay` — re-run the pipeline and diff against the bundle.
//! * `unpack` — extract artefacts for deep inspection.
//!
//! Operators reach for `show` first because it answers the
//! cheapest question — *what's in here?* — in one shot.
//!
//! Exit codes:
//!
//! * `0` — bundle shown.
//! * `1` — JSON serialise failure (only with `--json`).
//! * `2` — usage error (bad args, unreadable file, malformed
//!   bundle).

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use invoicekit_evidence::{unpack, EvidenceBundle};
use serde::Serialize;

/// Run `invoicekit show`.
#[must_use]
pub fn run(argv: &[String]) -> ExitCode {
    let parsed = match parse_args(argv) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };

    let bytes = match fs::read(&parsed.bundle) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("show: cannot read {}: {err}", parsed.bundle.display());
            return ExitCode::from(2);
        }
    };
    let total_bytes = bytes.len() as u64;

    let bundle = match unpack(&bytes) {
        Ok(b) => b,
        Err(err) => {
            eprintln!(
                "show: {} is not a valid evidence bundle: {err}",
                parsed.bundle.display()
            );
            return ExitCode::from(2);
        }
    };

    let summary = summarize(&bundle, total_bytes);

    if parsed.json {
        match serde_json::to_string_pretty(&summary) {
            Ok(json) => println!("{json}"),
            Err(err) => {
                eprintln!("show: summary serialise failed: {err}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print_human(&summary);
    }

    ExitCode::SUCCESS
}

#[derive(Debug)]
struct Args {
    bundle: PathBuf,
    json: bool,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut bundle: Option<PathBuf> = None;
    let mut json = false;
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        match arg.as_str() {
            "--help" | "-h" => return Err(usage_help()),
            "--json" => {
                json = true;
                i += 1;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("show: unknown flag {flag:?}\n\n{}", usage_help()));
            }
            positional => {
                if bundle.is_some() {
                    return Err(format!(
                        "show: extra positional argument {positional:?}\n\n{}",
                        usage_help()
                    ));
                }
                bundle = Some(PathBuf::from(positional));
                i += 1;
            }
        }
    }
    let bundle =
        bundle.ok_or_else(|| format!("show: <bundle.ikb> required\n\n{}", usage_help()))?;
    Ok(Args { bundle, json })
}

fn usage_help() -> String {
    "usage: invoicekit show <bundle.ikb> [--json]\n\nPrint a human-readable manifest summary for a .ikb evidence bundle (or a JSON summary with --json). Read-only; never writes to disk.".to_owned()
}

/// Plain-old-data snapshot of a bundle, suitable for JSON
/// serialisation or human pretty-printing.
#[derive(Debug, Serialize)]
struct BundleSummary {
    schema_version: String,
    created_at: String,
    tenant_id: String,
    trace_id: String,
    container_bytes: u64,
    artefact_count: usize,
    artefacts: Vec<ArtefactSummary>,
}

#[derive(Debug, Serialize)]
struct ArtefactSummary {
    id: String,
    size: u64,
    blake3_hex: String,
}

fn summarize(bundle: &EvidenceBundle, container_bytes: u64) -> BundleSummary {
    let artefacts: Vec<ArtefactSummary> = bundle
        .manifest
        .artefacts
        .iter()
        .map(|entry| ArtefactSummary {
            id: entry.id.clone(),
            size: entry.size,
            blake3_hex: entry.blake3_hex.clone(),
        })
        .collect();
    BundleSummary {
        schema_version: bundle.manifest.schema_version.clone(),
        created_at: bundle.manifest.created_at.clone(),
        tenant_id: bundle.manifest.tenant_id.clone(),
        trace_id: bundle.manifest.trace_id.clone(),
        container_bytes,
        artefact_count: artefacts.len(),
        artefacts,
    }
}

fn print_human(summary: &BundleSummary) {
    println!("Schema:        {}", summary.schema_version);
    println!("Created at:    {}", summary.created_at);
    println!("Tenant:        {}", summary.tenant_id);
    println!("Trace:         {}", summary.trace_id);
    println!("Container:     {} bytes", summary.container_bytes);
    println!("Artefacts:     {}", summary.artefact_count);
    println!();
    println!("{:<40}  {:>10}  blake3", "id", "size");
    println!("{:<40}  {:>10}  ------", "--", "----");
    for a in &summary.artefacts {
        // BLAKE3 hex is 64 chars; print first 16 chars only in
        // the human view so the line stays readable. Full hash
        // is in --json output.
        let short = &a.blake3_hex[..16.min(a.blake3_hex.len())];
        println!("{:<40}  {:>10}  {short}…", a.id, a.size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_evidence::{manifest_for, pack};
    use std::collections::BTreeMap;

    fn sample_bytes() -> Vec<u8> {
        let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        artefacts.insert("canonical.json".to_owned(), br#"{"id":"SHOW-1"}"#.to_vec());
        artefacts.insert("formats/ubl.xml".to_owned(), b"<Invoice/>".to_vec());
        let manifest = manifest_for(
            &artefacts,
            "tenant-show",
            "trace-show",
            "2026-05-28T04:00:00Z",
        );
        pack(&EvidenceBundle {
            manifest,
            artefacts,
        })
        .unwrap()
    }

    #[test]
    fn run_with_no_args_returns_usage_error() {
        let code = run(&[]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_unknown_flag_returns_usage_error() {
        let code = run(&["--xyzzy".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_extra_positional_returns_usage_error() {
        let code = run(&["a.ikb".to_owned(), "b.ikb".to_owned()]);
        // Path validation happens before the file is read, but
        // the file may not exist either; we just need a usage-
        // code-2 to come out either way.
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_missing_file_returns_usage_error() {
        let code = run(&["/tmp/does/not/exist.ikb".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_malformed_bundle_returns_usage_error() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.ikb");
        fs::write(&bad, b"not a bundle").unwrap();
        let code = run(&[bad.to_string_lossy().into_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_valid_bundle_returns_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.ikb");
        fs::write(&path, sample_bytes()).unwrap();
        let code = run(&[path.to_string_lossy().into_owned()]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn run_with_json_flag_returns_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.ikb");
        fs::write(&path, sample_bytes()).unwrap();
        let code = run(&[path.to_string_lossy().into_owned(), "--json".to_owned()]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn summarize_carries_manifest_fields_and_artefact_rows() {
        let bytes = sample_bytes();
        let bundle = unpack(&bytes).unwrap();
        let summary = summarize(&bundle, bytes.len() as u64);
        assert_eq!(summary.schema_version, "1.0");
        assert_eq!(summary.tenant_id, "tenant-show");
        assert_eq!(summary.trace_id, "trace-show");
        assert_eq!(summary.created_at, "2026-05-28T04:00:00Z");
        // Manifest entries are payloads only (the manifest does
        // not list itself); the container layer adds
        // manifest.json on top via [`invoicekit_evidence::pack`].
        assert_eq!(summary.artefact_count, 2);
        let ids: Vec<&str> = summary.artefacts.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.contains(&"canonical.json"));
        assert!(ids.contains(&"formats/ubl.xml"));
    }

    #[test]
    fn parse_args_extracts_bundle_and_json() {
        let parsed = parse_args(&["b.ikb".to_owned(), "--json".to_owned()]).unwrap();
        assert_eq!(parsed.bundle, PathBuf::from("b.ikb"));
        assert!(parsed.json);
    }

    #[test]
    fn parse_args_defaults_to_human_output() {
        let parsed = parse_args(&["b.ikb".to_owned()]).unwrap();
        assert!(!parsed.json);
    }
}
