// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit migrate-archive` — bulk-migrate every invoice JSON file in a
//! directory tree to a requested target [`invoicekit_ir::SchemaVersion`].
//!
//! Usage:
//!
//! ```text
//! invoicekit migrate-archive --from-version=1.0 --to-version=1.0 path/to/archive
//! ```
//!
//! Walks the directory tree, parses every file with a `.json` suffix,
//! checks the source version matches `--from-version`, and lifts the
//! document to `--to-version` via `invoicekit_migration::migrate`. Writes
//! the migrated document back in-place. Prints a per-file summary to
//! stdout and a typed `MigrationReport` to stderr for any non-clean
//! migration. Exits non-zero on any failure.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use invoicekit_ir::SchemaVersion;
use invoicekit_migration::migrate;

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    let parsed = match parse_args(&argv) {
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
    let mut migrated = 0_usize;
    let mut skipped = 0_usize;

    for file in &files {
        match migrate_file(file, parsed.from, parsed.to) {
            Ok(Outcome::Migrated(report)) => {
                migrated += 1;
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
                skipped += 1;
                println!("skipped {}: {}", file.display(), reason);
            }
            Err(message) => {
                had_error = true;
                eprintln!("failed {}: {message}", file.display());
            }
        }
    }

    println!(
        "migrate-archive summary: migrated={migrated} skipped={skipped} errors={}",
        usize::from(had_error)
    );

    if had_error {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

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
    Migrated(invoicekit_migration::MigrationReport),
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
