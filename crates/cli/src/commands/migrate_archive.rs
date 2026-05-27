// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit migrate-archive` runner.
//!
//! Walks a directory tree, parses every `.json` file, checks the source
//! `schema_version` matches `--from-version`, lifts the document to
//! `--to-version` via [`invoicekit_migration::migrate`], and writes the
//! migrated document back in-place. Prints a per-file summary to stdout
//! and any [`invoicekit_migration::MigrationFinding`]s to stderr. Returns
//! a non-zero [`ExitCode`] on any failure.
//!
//! Both the published `invoicekit migrate-archive` invocation (via
//! `crates/cli/src/main.rs`) and the per-subcommand helper binary
//! (`crates/cli/src/bin/migrate_archive.rs`) dispatch through [`run`] so
//! the published path is exercised end-to-end by CI.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use invoicekit_ir::SchemaVersion;
use invoicekit_migration::{migrate, MigrationReport};

/// Run `invoicekit migrate-archive` with the given subcommand argv
/// (already stripped of the leading `migrate-archive` token).
#[must_use]
pub fn run(argv: &[String]) -> ExitCode {
    let parsed = match parse_args(argv) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    let files = match collect_json_files(&parsed.root) {
        Ok(files) => files,
        Err(err) => {
            eprintln!(
                "migrate-archive: cannot walk {}: {err}",
                parsed.root.display()
            );
            return ExitCode::from(1);
        }
    };

    if files.is_empty() {
        eprintln!(
            "migrate-archive: no .json files found under {}",
            parsed.root.display()
        );
        return ExitCode::SUCCESS;
    }

    let mut had_error = false;
    let mut migrated_count = 0_usize;
    let mut skipped_count = 0_usize;

    for file in &files {
        match migrate_file(file, parsed.from, parsed.to) {
            Ok(Outcome::Migrated(report)) => {
                migrated_count += 1;
                println!(
                    "migrated {}: {} -> {} (reversible={}, findings={})",
                    file.display(),
                    short_version(report.from),
                    short_version(report.to),
                    report.reversible,
                    report.findings.len()
                );
                if !report.is_clean() {
                    for finding in &report.findings {
                        eprintln!(
                            "  finding: path={} kind={} message={}",
                            finding.path, finding.kind, finding.message,
                        );
                    }
                }
            }
            Ok(Outcome::Skipped(reason)) => {
                skipped_count += 1;
                println!("skipped {}: {}", file.display(), reason);
            }
            Err(message) => {
                had_error = true;
                eprintln!("failed {}: {message}", file.display());
            }
        }
    }

    println!(
        "migrate-archive summary: migrated={migrated_count} skipped={skipped_count} errors={}",
        usize::from(had_error)
    );

    if had_error {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

#[derive(Debug)]
struct ParsedArgs {
    from: SchemaVersion,
    to: SchemaVersion,
    root: PathBuf,
}

fn parse_args(argv: &[String]) -> Result<ParsedArgs, String> {
    let mut from: Option<SchemaVersion> = None;
    let mut to: Option<SchemaVersion> = None;
    let mut root: Option<PathBuf> = None;
    let mut iter = argv.iter();
    while let Some(arg) = iter.next() {
        if let Some(value) = arg.strip_prefix("--from-version=") {
            from = Some(parse_version(value)?);
        } else if arg == "--from-version" {
            from = Some(parse_version(iter.next().ok_or_else(|| {
                "missing value for --from-version (hint: use --from-version=1.0)".to_owned()
            })?)?);
        } else if let Some(value) = arg.strip_prefix("--to-version=") {
            to = Some(parse_version(value)?);
        } else if arg == "--to-version" {
            to = Some(parse_version(iter.next().ok_or_else(|| {
                "missing value for --to-version (hint: use --to-version=1.0)".to_owned()
            })?)?);
        } else if arg == "--help" || arg == "-h" {
            return Err(usage());
        } else if root.is_some() {
            return Err(format!(
                "unexpected positional argument `{arg}` (hint: pass the archive directory exactly once)"
            ));
        } else {
            root = Some(PathBuf::from(arg));
        }
    }
    let from = from.ok_or_else(|| usage_with("missing --from-version"))?;
    let to = to.ok_or_else(|| usage_with("missing --to-version"))?;
    let root = root.ok_or_else(|| usage_with("missing archive directory"))?;
    Ok(ParsedArgs { from, to, root })
}

fn usage() -> String {
    "usage: invoicekit migrate-archive --from-version=<vN> --to-version=<vM> <archive-dir>\n\
     where <vN>/<vM> are SchemaVersion tags (e.g. 1.0)"
        .to_owned()
}

fn usage_with(message: &str) -> String {
    format!("migrate-archive: {message}\n{}", usage())
}

fn parse_version(raw: &str) -> Result<SchemaVersion, String> {
    serde_json::from_value::<SchemaVersion>(serde_json::Value::String(raw.to_owned()))
        .map_err(|_| format!("unsupported schema version `{raw}` (hint: try 1.0)"))
}

fn short_version(version: SchemaVersion) -> &'static str {
    match version {
        SchemaVersion::V1_0 => "1.0",
    }
}

enum Outcome {
    Migrated(MigrationReport),
    Skipped(String),
}

fn migrate_file(
    file: &Path,
    expected_from: SchemaVersion,
    to: SchemaVersion,
) -> Result<Outcome, String> {
    let raw = fs::read(file).map_err(|err| format!("cannot read: {err}"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|err| format!("invalid JSON: {err}"))?;
    let source_text = value
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "missing `schema_version` field".to_owned())?;
    let source = parse_version(source_text)?;
    if source != expected_from {
        return Ok(Outcome::Skipped(format!(
            "schema_version is `{source_text}`, expected `{}`",
            short_version(expected_from)
        )));
    }
    let (migrated, report) = migrate(value, to).map_err(|err| err.to_string())?;
    let serialized = serde_json::to_vec_pretty(&migrated)
        .map_err(|err| format!("cannot serialize migrated value: {err}"))?;
    fs::write(file, serialized).map_err(|err| format!("cannot write back: {err}"))?;
    Ok(Outcome::Migrated(report))
}

fn collect_json_files(root: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut out = Vec::new();
    walk(root, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
    if !dir.is_dir() {
        if dir.is_file() && dir.extension().and_then(|s| s.to_str()) == Some("json") {
            out.push(dir.to_owned());
        }
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out)?;
        } else if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_accepts_eq_form() {
        let argv = vec![
            "--from-version=1.0".to_owned(),
            "--to-version=1.0".to_owned(),
            "/tmp/archive".to_owned(),
        ];
        let parsed = parse_args(&argv).unwrap();
        assert_eq!(parsed.from, SchemaVersion::V1_0);
        assert_eq!(parsed.to, SchemaVersion::V1_0);
        assert_eq!(parsed.root, PathBuf::from("/tmp/archive"));
    }

    #[test]
    fn parse_args_accepts_split_form() {
        let argv = vec![
            "--from-version".to_owned(),
            "1.0".to_owned(),
            "--to-version".to_owned(),
            "1.0".to_owned(),
            "/tmp/archive".to_owned(),
        ];
        let parsed = parse_args(&argv).unwrap();
        assert_eq!(parsed.from, SchemaVersion::V1_0);
    }

    #[test]
    fn parse_args_rejects_missing_from_version() {
        let argv = vec!["--to-version=1.0".to_owned(), "/tmp/archive".to_owned()];
        let err = parse_args(&argv).unwrap_err();
        assert!(err.contains("missing --from-version"));
    }

    #[test]
    fn parse_args_rejects_unknown_version() {
        let argv = vec![
            "--from-version=99.0".to_owned(),
            "--to-version=1.0".to_owned(),
            "/tmp/archive".to_owned(),
        ];
        let err = parse_args(&argv).unwrap_err();
        assert!(err.contains("unsupported schema version"));
    }

    #[test]
    fn parse_args_rejects_extra_positional() {
        let argv = vec![
            "--from-version=1.0".to_owned(),
            "--to-version=1.0".to_owned(),
            "/tmp/archive".to_owned(),
            "/tmp/extra".to_owned(),
        ];
        let err = parse_args(&argv).unwrap_err();
        assert!(err.contains("unexpected positional argument"));
    }

    #[test]
    fn run_returns_success_on_empty_directory() {
        let dir = tempdir();
        let argv = vec![
            "--from-version=1.0".to_owned(),
            "--to-version=1.0".to_owned(),
            dir.to_string_lossy().to_string(),
        ];
        let exit = run(&argv);
        // Empty archive is not an error per the runner contract.
        assert_eq!(format!("{exit:?}"), "ExitCode(unix_exit_status(0))");
    }

    #[test]
    fn run_migrates_identity_in_place() {
        let dir = tempdir();
        let file = dir.join("doc.json");
        fs::write(
            &file,
            br#"{"schema_version":"1.0","id":"doc-1","marker":"keep"}"#,
        )
        .expect("write fixture");

        let argv = vec![
            "--from-version=1.0".to_owned(),
            "--to-version=1.0".to_owned(),
            dir.to_string_lossy().to_string(),
        ];
        let exit = run(&argv);
        assert_eq!(format!("{exit:?}"), "ExitCode(unix_exit_status(0))");

        let after = fs::read_to_string(&file).expect("read back");
        assert!(after.contains("\"schema_version\": \"1.0\""));
        assert!(after.contains("\"marker\": \"keep\""));
    }

    fn tempdir() -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nonce = format!(
            "invoicekit-cli-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        );
        dir.push(nonce);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }
}
