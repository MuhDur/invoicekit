// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit diff` runner.
//!
//! Compares two `.ikb` evidence bundles artefact-by-artefact
//! and reports what's different. Used by auditors to compare
//! a recorded bundle against a freshly produced one without
//! running the full replay pipeline (replay catches engine
//! drift; diff catches "did anything change between these two
//! bundles" — a simpler, faster question).
//!
//! Per artefact id the diff is one of:
//!
//! * `byte-equal` — same blake3, same size, both bundles
//!   carry the artefact.
//! * `changed` — both bundles carry the id but the blake3
//!   differs.
//! * `only-in-left` — bundle A carries the artefact, B does
//!   not.
//! * `only-in-right` — bundle B carries the artefact, A does
//!   not.
//!
//! Exit codes:
//!
//! * `0` — bundles compared and are byte-equal across all
//!   artefacts.
//! * `1` — bundles compared and at least one artefact differs.
//! * `2` — usage error (bad args, unreadable file, malformed
//!   bundle).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use invoicekit_evidence::{blake3_hex, unpack, EvidenceBundle};
use serde::Serialize;

/// Run `invoicekit diff`.
#[must_use]
pub fn run(argv: &[String]) -> ExitCode {
    let parsed = match parse_args(argv) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };

    let left = match load(&parsed.left, "left") {
        Ok(b) => b,
        Err(code) => return code,
    };
    let right = match load(&parsed.right, "right") {
        Ok(b) => b,
        Err(code) => return code,
    };

    let report = compute_diff(&left, &right);

    if parsed.json {
        match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(err) => {
                eprintln!("diff: report serialise failed: {err}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print_human(&parsed, &report);
    }

    if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn load(path: &std::path::Path, side: &str) -> Result<EvidenceBundle, ExitCode> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("diff: cannot read {side} bundle {}: {err}", path.display());
            return Err(ExitCode::from(2));
        }
    };
    unpack(&bytes).map_err(|err| {
        eprintln!(
            "diff: {side} bundle {} is not a valid evidence bundle: {err}",
            path.display()
        );
        ExitCode::from(2)
    })
}

#[derive(Debug)]
struct Args {
    left: PathBuf,
    right: PathBuf,
    json: bool,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut left: Option<PathBuf> = None;
    let mut right: Option<PathBuf> = None;
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
                return Err(format!("diff: unknown flag {flag:?}\n\n{}", usage_help()));
            }
            positional => {
                if left.is_none() {
                    left = Some(PathBuf::from(positional));
                } else if right.is_none() {
                    right = Some(PathBuf::from(positional));
                } else {
                    return Err(format!(
                        "diff: extra positional argument {positional:?}\n\n{}",
                        usage_help()
                    ));
                }
                i += 1;
            }
        }
    }
    let left = left.ok_or_else(|| format!("diff: <left.ikb> required\n\n{}", usage_help()))?;
    let right = right.ok_or_else(|| format!("diff: <right.ikb> required\n\n{}", usage_help()))?;
    Ok(Args { left, right, json })
}

fn usage_help() -> String {
    "usage: invoicekit diff <left.ikb> <right.ikb> [--json]\n\nCompare two evidence bundles artefact-by-artefact. Exit 0 on byte-equal across all artefacts, 1 on any diff, 2 on usage error.".to_owned()
}

/// One artefact's diff verdict.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum ArtefactDiff {
    /// Same blake3, same size, present in both bundles.
    ByteEqual {
        blake3_hex: String,
        size: u64,
    },
    /// Present in both bundles but bytes differ.
    Changed {
        left_blake3_hex: String,
        right_blake3_hex: String,
        left_size: u64,
        right_size: u64,
    },
    OnlyInLeft {
        blake3_hex: String,
        size: u64,
    },
    OnlyInRight {
        blake3_hex: String,
        size: u64,
    },
}

impl ArtefactDiff {
    fn is_diff(&self) -> bool {
        !matches!(self, Self::ByteEqual { .. })
    }
}

/// Aggregate diff report.
#[derive(Debug, Serialize)]
pub(crate) struct DiffReport {
    ok: bool,
    left_artefact_count: usize,
    right_artefact_count: usize,
    diffs_by_id: BTreeMap<String, ArtefactDiff>,
}

pub(crate) fn compute_diff(left: &EvidenceBundle, right: &EvidenceBundle) -> DiffReport {
    let mut diffs: BTreeMap<String, ArtefactDiff> = BTreeMap::new();

    // Pass 1: every id in left.
    for (id, l_bytes) in &left.artefacts {
        let l_hex = blake3_hex(l_bytes);
        if let Some(r_bytes) = right.artefacts.get(id) {
            let r_hex = blake3_hex(r_bytes);
            let diff = if l_hex == r_hex {
                ArtefactDiff::ByteEqual {
                    blake3_hex: l_hex,
                    size: l_bytes.len() as u64,
                }
            } else {
                ArtefactDiff::Changed {
                    left_blake3_hex: l_hex,
                    right_blake3_hex: r_hex,
                    left_size: l_bytes.len() as u64,
                    right_size: r_bytes.len() as u64,
                }
            };
            diffs.insert(id.clone(), diff);
        } else {
            diffs.insert(
                id.clone(),
                ArtefactDiff::OnlyInLeft {
                    blake3_hex: l_hex,
                    size: l_bytes.len() as u64,
                },
            );
        }
    }

    // Pass 2: ids only in right.
    for (id, r_bytes) in &right.artefacts {
        if left.artefacts.contains_key(id) {
            continue;
        }
        diffs.insert(
            id.clone(),
            ArtefactDiff::OnlyInRight {
                blake3_hex: blake3_hex(r_bytes),
                size: r_bytes.len() as u64,
            },
        );
    }

    let ok = diffs.values().all(|d| !d.is_diff());
    DiffReport {
        ok,
        left_artefact_count: left.artefacts.len(),
        right_artefact_count: right.artefacts.len(),
        diffs_by_id: diffs,
    }
}

fn print_human(args: &Args, report: &DiffReport) {
    println!(
        "left:  {} ({} artefacts)",
        args.left.display(),
        report.left_artefact_count
    );
    println!(
        "right: {} ({} artefacts)",
        args.right.display(),
        report.right_artefact_count
    );
    println!();
    for (id, diff) in &report.diffs_by_id {
        let tag = match diff {
            ArtefactDiff::ByteEqual { .. } => "EQ",
            ArtefactDiff::Changed { .. } => "CHG",
            ArtefactDiff::OnlyInLeft { .. } => "L  ",
            ArtefactDiff::OnlyInRight { .. } => "  R",
        };
        println!("[{tag}] {id}");
    }
    println!();
    println!(
        "overall: {}",
        if report.ok { "byte-equal" } else { "differs" }
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_evidence::{manifest_for, pack};

    fn bundle_with(artefacts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut map: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for (id, bytes) in artefacts {
            map.insert((*id).to_owned(), (*bytes).to_vec());
        }
        let manifest = manifest_for(&map, "tenant-diff", "trace-diff", "2026-05-28T05:00:00Z");
        pack(&EvidenceBundle {
            manifest,
            artefacts: map,
        })
        .unwrap()
    }

    #[test]
    fn run_with_no_args_returns_usage_error() {
        let code = run(&[]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_only_one_arg_returns_usage_error() {
        let code = run(&["only.ikb".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_unknown_flag_returns_usage_error() {
        let code = run(&["--xyzzy".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_extra_positional_returns_usage_error() {
        let code = run(&["a.ikb".to_owned(), "b.ikb".to_owned(), "c.ikb".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_missing_left_returns_usage_error() {
        let dir = tempfile::tempdir().unwrap();
        let right = dir.path().join("right.ikb");
        fs::write(&right, bundle_with(&[("a", b"x")])).unwrap();
        let code = run(&[
            "/tmp/does/not/exist.ikb".to_owned(),
            right.to_string_lossy().into_owned(),
        ]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_malformed_right_returns_usage_error() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("left.ikb");
        let right = dir.path().join("right.ikb");
        fs::write(&left, bundle_with(&[("a", b"x")])).unwrap();
        fs::write(&right, b"not a bundle").unwrap();
        let code = run(&[
            left.to_string_lossy().into_owned(),
            right.to_string_lossy().into_owned(),
        ]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_identical_bundles_returns_success() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = bundle_with(&[("a", b"x"), ("b/c.xml", b"<x/>")]);
        let left = dir.path().join("l.ikb");
        let right = dir.path().join("r.ikb");
        fs::write(&left, &bytes).unwrap();
        fs::write(&right, &bytes).unwrap();
        let code = run(&[
            left.to_string_lossy().into_owned(),
            right.to_string_lossy().into_owned(),
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn run_with_changed_artefact_returns_failure() {
        let dir = tempfile::tempdir().unwrap();
        let left_bytes = bundle_with(&[("a", b"original")]);
        let right_bytes = bundle_with(&[("a", b"different")]);
        let left = dir.path().join("l.ikb");
        let right = dir.path().join("r.ikb");
        fs::write(&left, &left_bytes).unwrap();
        fs::write(&right, &right_bytes).unwrap();
        let code = run(&[
            left.to_string_lossy().into_owned(),
            right.to_string_lossy().into_owned(),
        ]);
        assert_eq!(code, ExitCode::FAILURE);
    }

    #[test]
    fn compute_diff_classifies_each_artefact_correctly() {
        let left_bytes = bundle_with(&[("same", b"x"), ("changed", b"a"), ("only-left", b"l")]);
        let right_bytes = bundle_with(&[("same", b"x"), ("changed", b"b"), ("only-right", b"r")]);
        let left = unpack(&left_bytes).unwrap();
        let right = unpack(&right_bytes).unwrap();
        let report = compute_diff(&left, &right);
        assert!(!report.ok);
        match report.diffs_by_id.get("same").unwrap() {
            ArtefactDiff::ByteEqual { .. } => {}
            other => panic!("expected ByteEqual, got {other:?}"),
        }
        match report.diffs_by_id.get("changed").unwrap() {
            ArtefactDiff::Changed { .. } => {}
            other => panic!("expected Changed, got {other:?}"),
        }
        match report.diffs_by_id.get("only-left").unwrap() {
            ArtefactDiff::OnlyInLeft { .. } => {}
            other => panic!("expected OnlyInLeft, got {other:?}"),
        }
        match report.diffs_by_id.get("only-right").unwrap() {
            ArtefactDiff::OnlyInRight { .. } => {}
            other => panic!("expected OnlyInRight, got {other:?}"),
        }
    }

    #[test]
    fn parse_args_extracts_both_paths_and_json() {
        let parsed =
            parse_args(&["a.ikb".to_owned(), "b.ikb".to_owned(), "--json".to_owned()]).unwrap();
        assert_eq!(parsed.left, PathBuf::from("a.ikb"));
        assert_eq!(parsed.right, PathBuf::from("b.ikb"));
        assert!(parsed.json);
    }
}
