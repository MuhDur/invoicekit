// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit verify` runner.
//!
//! Verifies a `.invoicekit` / `.ikb` evidence bundle by
//! delegating to [`invoicekit_verify::verify_packed`]. The
//! content-address check is always run; signature + timestamp
//! checks are skipped today because the CLI doesn't yet wire
//! a signer or TSA client (T-100 / T-083a / T-082 follow-ups
//! land those).
//!
//! Exit codes:
//!
//! * `0` — bundle verified (content-address ok; signature +
//!   timestamp skipped, see `--require-*` flags).
//! * `1` — verification produced a `Failed` outcome (bundle
//!   tampered or drifted).
//! * `2` — usage error (bad args / unreadable file).

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use invoicekit_verify::{verify_packed, CheckOutcome, VerifyOptions};

/// Run `invoicekit verify`.
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
            eprintln!("verify: cannot read {}: {err}", parsed.bundle.display());
            return ExitCode::from(2);
        }
    };

    let options = VerifyOptions::content_only();
    let report = match verify_packed(&bytes, &options) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("verify: {} did not unpack: {err}", parsed.bundle.display());
            return ExitCode::FAILURE;
        }
    };

    let json = match serde_json::to_string_pretty(&report) {
        Ok(j) => j,
        Err(err) => {
            eprintln!("verify: report serialise failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    println!("{json}");

    if report.ok {
        eprintln!("verify: {} ok", parsed.bundle.display());
        ExitCode::SUCCESS
    } else {
        let failed: Vec<&str> = [
            ("content_address", &report.content_address),
            ("signature", &report.signature),
            ("timestamp", &report.timestamp),
        ]
        .into_iter()
        .filter_map(|(name, outcome)| {
            matches!(outcome, CheckOutcome::Failed { .. }).then_some(name)
        })
        .collect();
        eprintln!(
            "verify: {} FAILED ({} failed: {})",
            parsed.bundle.display(),
            failed.len(),
            failed.join(", ")
        );
        ExitCode::FAILURE
    }
}

#[derive(Debug)]
struct Args {
    bundle: PathBuf,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut bundle: Option<PathBuf> = None;
    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];
        match arg.as_str() {
            "--help" | "-h" => {
                return Err(usage_help());
            }
            flag if flag.starts_with('-') => {
                return Err(format!("verify: unknown flag {flag:?}\n\n{}", usage_help()));
            }
            positional => {
                if bundle.is_some() {
                    return Err(format!(
                        "verify: extra positional argument {positional:?}\n\n{}",
                        usage_help()
                    ));
                }
                bundle = Some(PathBuf::from(positional));
            }
        }
        i += 1;
    }
    let bundle =
        bundle.ok_or_else(|| format!("verify: <bundle> argument required\n\n{}", usage_help()))?;
    Ok(Args { bundle })
}

fn usage_help() -> String {
    "usage: invoicekit verify <bundle.ikb>\n\nVerify an evidence bundle. Prints a JSON report to stdout; exit code is 0 on pass, 1 on fail.".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn sample_bundle() -> EvidenceBundle {
        let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        artefacts.insert("canonical.json".to_owned(), br#"{"id":"INV-1"}"#.to_vec());
        artefacts.insert("formats/ubl.xml".to_owned(), b"<Invoice/>".to_vec());
        let manifest = manifest_for(
            &artefacts,
            "tenant-cli",
            "trace-cli",
            "2026-05-28T00:00:00Z",
        );
        EvidenceBundle {
            manifest,
            artefacts,
        }
    }

    fn write_bundle(dir: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn run_with_no_args_returns_usage_error() {
        let code = run(&[]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_help_flag_returns_usage_error() {
        // `--help` returns the same code as a usage error
        // (matches the rest of the CLI shape).
        let code = run(&["--help".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_unknown_flag_returns_usage_error() {
        let code = run(&["--xyzzy".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_missing_file_returns_usage_error() {
        let code = run(&["/tmp/this/file/does/not/exist.ikb".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_valid_bundle_returns_success() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = sample_bundle();
        let packed = pack(&bundle).unwrap();
        let path = write_bundle(&dir, "sample.ikb", &packed);
        let code = run(&[path.to_string_lossy().into_owned()]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn run_with_tampered_bundle_returns_failure() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = sample_bundle();
        let mut packed = pack(&bundle).unwrap();
        // Flip a byte well past the header so the magic check
        // passes but a payload hash fails. The manifest entry
        // sits past offset 64; mutate near the end.
        let idx = packed.len() - 6;
        packed[idx] ^= 0xff;
        let path = write_bundle(&dir, "tampered.ikb", &packed);
        let code = run(&[path.to_string_lossy().into_owned()]);
        // The bundle won't unpack cleanly OR the payload hash
        // fails verification; either way `verify` returns
        // FAILURE.
        assert_eq!(code, ExitCode::FAILURE);
    }

    #[test]
    fn parse_args_extracts_bundle_path() {
        let parsed = parse_args(&["bundle.ikb".to_owned()]).unwrap();
        assert_eq!(parsed.bundle, PathBuf::from("bundle.ikb"));
    }

    #[test]
    fn parse_args_rejects_duplicate_positional() {
        let err = parse_args(&["a.ikb".to_owned(), "b.ikb".to_owned()]).unwrap_err();
        assert!(err.contains("extra positional"));
    }
}
