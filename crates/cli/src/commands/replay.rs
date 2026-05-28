// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit replay` runner.
//!
//! Replays an evidence bundle through
//! [`invoicekit_replay::replay`] using an
//! [`invoicekit_replay::IdentityReplayer`].
//!
//! Today the engine isn't yet hooked into the CLI, so we use
//! the identity replayer. That produces a baseline byte-equal
//! report against the bundle's own artefacts; the moment T-100
//! wires in the real pipeline replayer the same subcommand will
//! start surfacing actual drift without changing flags or exit
//! codes. Plain English: this command answers "would a fresh
//! engine pass produce the exact bytes this bundle claims?".
//!
//! Exit codes:
//!
//! * `0` — every selected artefact byte-equal.
//! * `1` — at least one artefact drifted / went unreplayed /
//!   appeared as an unexpected emit.
//! * `2` — usage error (bad args / unreadable file / malformed
//!   bundle).

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use invoicekit_evidence::unpack;
use invoicekit_replay::{replay, IdentityReplayer, ReplayOptions};

/// Run `invoicekit replay`.
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
            eprintln!("replay: cannot read {}: {err}", parsed.bundle.display());
            return ExitCode::from(2);
        }
    };

    let bundle = match unpack(&bytes) {
        Ok(b) => b,
        Err(err) => {
            eprintln!(
                "replay: {} is not a valid evidence bundle: {err}",
                parsed.bundle.display()
            );
            return ExitCode::from(2);
        }
    };

    let options = ReplayOptions::all();
    let report = match replay(&bundle, &IdentityReplayer, &options) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("replay: {err}");
            return ExitCode::FAILURE;
        }
    };

    let json = match serde_json::to_string_pretty(&report) {
        Ok(j) => j,
        Err(err) => {
            eprintln!("replay: report serialise failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    println!("{json}");

    if report.ok {
        eprintln!(
            "replay: {} ok ({} artefacts byte-equal)",
            parsed.bundle.display(),
            report.deltas.len()
        );
        ExitCode::SUCCESS
    } else {
        let drifted: Vec<&str> = report.drifted_ids().collect();
        eprintln!(
            "replay: {} DRIFTED ({} of {} artefacts diverged: {})",
            parsed.bundle.display(),
            drifted.len(),
            report.deltas.len(),
            drifted.join(", ")
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
            "--help" | "-h" => return Err(usage_help()),
            flag if flag.starts_with('-') => {
                return Err(format!("replay: unknown flag {flag:?}\n\n{}", usage_help()));
            }
            positional => {
                if bundle.is_some() {
                    return Err(format!(
                        "replay: extra positional argument {positional:?}\n\n{}",
                        usage_help()
                    ));
                }
                bundle = Some(PathBuf::from(positional));
            }
        }
        i += 1;
    }
    let bundle =
        bundle.ok_or_else(|| format!("replay: <bundle> argument required\n\n{}", usage_help()))?;
    Ok(Args { bundle })
}

fn usage_help() -> String {
    "usage: invoicekit replay <bundle.ikb>\n\nReplay an evidence bundle through the identity replayer and report per-artefact drift. Prints a JSON report to stdout; exit code is 0 on byte-equal, 1 on drift.".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
    use std::collections::BTreeMap;

    fn sample_bundle() -> EvidenceBundle {
        let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        artefacts.insert(
            "canonical.json".to_owned(),
            br#"{"id":"INV-replay-1"}"#.to_vec(),
        );
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

    fn write_bundle(name: &str, bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    #[test]
    fn run_with_no_args_returns_usage_error() {
        let code = run(&[]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_missing_file_returns_usage_error() {
        let code = run(&["/tmp/this/file/does/not/exist.ikb".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_malformed_bundle_returns_usage_error() {
        let (_dir, path) = write_bundle("junk.ikb", b"not a real bundle");
        let code = run(&[path.to_string_lossy().into_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_valid_bundle_returns_success() {
        let packed = pack(&sample_bundle()).unwrap();
        let (_dir, path) = write_bundle("sample.ikb", &packed);
        let code = run(&[path.to_string_lossy().into_owned()]);
        // Identity replayer always replays byte-equal.
        assert_eq!(code, ExitCode::SUCCESS);
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

    #[test]
    fn parse_args_rejects_unknown_flag() {
        let err = parse_args(&["--xyzzy".to_owned()]).unwrap_err();
        assert!(err.contains("unknown flag"));
    }
}
