// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit codelist-update` — refresh a code-list manifest from a
//! locally-staged upstream payload.
//!
//! ```text
//! invoicekit codelist-update --list=iso-4217 --source-file=PATH \
//!     [--output=PATH] [--retrieved-at=YYYY-MM-DD] [--dry-run]
//! ```
//!
//! The Rust side is intentionally network-free: the nightly workflow
//! (`.github/workflows/codelist-update.yml`) handles `curl`, hands a
//! local file path to this command, and uses `git diff` to decide
//! whether to open a refresh PR.
//!
//! Output write is atomic via temp-file + `fsync` + `rename`. If the
//! produced manifest is byte-identical to what's already on disk, the
//! runner reports "no change" and exits 0 without touching the file
//! (which is what lets the nightly job's `git diff --quiet` correctly
//! skip the PR-creation step on drift-free days).

use std::env;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use invoicekit_codelists::sources::{build_manifest, source_for};

/// Bead identifier carried alongside emitted log events for diagnostic correlation.
pub const CODELIST_UPDATE_BEAD_ID: &str = "invoices-t-018-codelist-updater-6s0";

/// CLI entry point. Returns 0 on a successful update (including the
/// "no change" branch), 2 on usage errors, 3 on source/IO errors.
///
/// # Panics
///
/// Panics only via the internal `expect` on `serde_json::to_string_pretty`,
/// which would indicate that a freshly-signed manifest failed to
/// serialize — impossible by construction since every field is
/// `Serialize`.
pub fn run(argv: &[String]) -> ExitCode {
    if argv.iter().any(|a| a == "--help" || a == "-h") {
        print!("{}", usage());
        return ExitCode::SUCCESS;
    }

    let parsed = match parse_argv(argv) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            eprintln!();
            eprint!("{}", usage());
            return ExitCode::from(2);
        }
    };

    let spec = match source_for(&parsed.list) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    let raw = match fs::read_to_string(&parsed.source_file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "failed to read source file {}: {e}",
                parsed.source_file.display()
            );
            return ExitCode::from(3);
        }
    };

    let retrieved_at = parsed
        .retrieved_at
        .clone()
        .unwrap_or_else(|| env::var("INVOICEKIT_TODAY").unwrap_or_else(|_| "2026-05-27".into()));

    let manifest = match build_manifest(spec, &raw, &retrieved_at) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(3);
        }
    };

    let serialized =
        serde_json::to_string_pretty(&manifest).expect("Manifest must serialize to JSON") + "\n";

    let output_path = parsed
        .output
        .clone()
        .unwrap_or_else(|| default_output_for(spec.list_name));

    let prior = fs::read_to_string(&output_path).ok();
    if prior.as_deref() == Some(serialized.as_str()) {
        println!(
            "codelist-update {list}: no change (signature {sig})",
            list = spec.list_name,
            sig = &manifest.signature[..16]
        );
        return ExitCode::SUCCESS;
    }

    if parsed.dry_run {
        println!(
            "codelist-update {list}: would write {bytes} bytes to {path} (signature {sig})",
            list = spec.list_name,
            bytes = serialized.len(),
            path = output_path.display(),
            sig = &manifest.signature[..16]
        );
        return ExitCode::SUCCESS;
    }

    if let Err(e) = atomic_write(&output_path, serialized.as_bytes()) {
        eprintln!("failed to write {}: {e}", output_path.display());
        return ExitCode::from(3);
    }
    println!(
        "codelist-update {list}: wrote {bytes} bytes to {path} (signature {sig})",
        list = spec.list_name,
        bytes = serialized.len(),
        path = output_path.display(),
        sig = &manifest.signature[..16]
    );
    ExitCode::SUCCESS
}

#[derive(Debug)]
struct ParsedArgs {
    list: String,
    source_file: PathBuf,
    output: Option<PathBuf>,
    retrieved_at: Option<String>,
    dry_run: bool,
}

#[derive(Debug)]
enum ParseError {
    Missing(&'static str),
    Unknown(String),
    BadValue(&'static str, String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(name) => write!(f, "missing required flag --{name}"),
            Self::Unknown(name) => write!(f, "unknown flag: {name}"),
            Self::BadValue(name, value) => {
                write!(f, "invalid value for --{name}: {value:?}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

fn parse_argv(argv: &[String]) -> Result<ParsedArgs, ParseError> {
    let mut list: Option<String> = None;
    let mut source_file: Option<String> = None;
    let mut output: Option<String> = None;
    let mut retrieved_at: Option<String> = None;
    let mut dry_run = false;

    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        if let Some(v) = a.strip_prefix("--list=") {
            list = Some(v.to_owned());
        } else if a == "--list" {
            i += 1;
            list = Some(argv.get(i).cloned().ok_or(ParseError::Missing("list"))?);
        } else if let Some(v) = a.strip_prefix("--source-file=") {
            source_file = Some(v.to_owned());
        } else if a == "--source-file" {
            i += 1;
            source_file = Some(
                argv.get(i)
                    .cloned()
                    .ok_or(ParseError::Missing("source-file"))?,
            );
        } else if let Some(v) = a.strip_prefix("--output=") {
            output = Some(v.to_owned());
        } else if a == "--output" {
            i += 1;
            output = Some(argv.get(i).cloned().ok_or(ParseError::Missing("output"))?);
        } else if let Some(v) = a.strip_prefix("--retrieved-at=") {
            retrieved_at = Some(v.to_owned());
        } else if a == "--retrieved-at" {
            i += 1;
            retrieved_at = Some(
                argv.get(i)
                    .cloned()
                    .ok_or(ParseError::Missing("retrieved-at"))?,
            );
        } else if a == "--dry-run" {
            dry_run = true;
        } else {
            return Err(ParseError::Unknown(a.clone()));
        }
        i += 1;
    }

    let list = list.ok_or(ParseError::Missing("list"))?;
    if list.is_empty() {
        return Err(ParseError::BadValue("list", list));
    }
    let source_file = source_file.ok_or(ParseError::Missing("source-file"))?;
    if source_file.is_empty() {
        return Err(ParseError::BadValue("source-file", source_file));
    }
    Ok(ParsedArgs {
        list,
        source_file: PathBuf::from(source_file),
        output: output.map(PathBuf::from),
        retrieved_at,
        dry_run,
    })
}

fn default_output_for(list: &str) -> PathBuf {
    // Mirrors the convention used by the existing seed manifests in
    // `crates/codelists/data/`. Resolved relative to the current
    // working directory so the nightly workflow (which `cd`s into the
    // repo root) lands files in the right place.
    let filename = match list {
        invoicekit_codelists::ISO_4217 => "iso-4217-2024.json",
        other => return PathBuf::from(format!("crates/codelists/data/{other}.json")),
    };
    PathBuf::from(format!("crates/codelists/data/{filename}"))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other(format!("no parent for {}", path.display())))?;
    if !parent.as_os_str().is_empty() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn usage() -> String {
    "usage: invoicekit codelist-update --list=NAME --source-file=PATH \\\n                                  [--output=PATH] [--retrieved-at=YYYY-MM-DD] [--dry-run]\n\nNormalize a locally-staged upstream code-list payload, sign it, and\natomically write the result to crates/codelists/data/<list>.json\n(or the path given by --output).\n\nThe Rust side never fetches over the network; the nightly workflow\nhandles `curl` and feeds this command a local file path.\n\nExit codes:\n  0  successful update OR no change\n  2  invalid CLI usage / unknown list\n  3  source-file read error, normalization error, or write error\n"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parse_argv_requires_list_and_source_file() {
        let err = parse_argv(&argv(&["--list=iso-4217"])).unwrap_err();
        assert!(matches!(err, ParseError::Missing("source-file")));
        let err = parse_argv(&argv(&["--source-file=/tmp/x.csv"])).unwrap_err();
        assert!(matches!(err, ParseError::Missing("list")));
    }

    #[test]
    fn parse_argv_supports_eq_and_split_forms() {
        let a = parse_argv(&argv(&["--list=iso-4217", "--source-file=/tmp/a.csv"])).unwrap();
        let b = parse_argv(&argv(&[
            "--list",
            "iso-4217",
            "--source-file",
            "/tmp/a.csv",
        ]))
        .unwrap();
        assert_eq!(a.list, "iso-4217");
        assert_eq!(b.list, "iso-4217");
        assert_eq!(a.source_file, PathBuf::from("/tmp/a.csv"));
        assert_eq!(b.source_file, PathBuf::from("/tmp/a.csv"));
        assert!(!a.dry_run);
    }

    #[test]
    fn parse_argv_rejects_unknown_flag() {
        let err = parse_argv(&argv(&[
            "--list=iso-4217",
            "--source-file=/tmp/a.csv",
            "--unsupported",
        ]))
        .unwrap_err();
        assert!(matches!(err, ParseError::Unknown(_)));
    }

    #[test]
    fn run_writes_signed_manifest() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("iso-4217.csv");
        fs::write(
            &src,
            "code,label,numeric,minor_units\nEUR,Euro,978,2\nUSD,US Dollar,840,2\n",
        )
        .unwrap();
        let out = dir.path().join("iso-4217.json");
        let code = run(&argv(&[
            "--list=iso-4217",
            "--source-file",
            src.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
            "--retrieved-at=2026-05-27",
        ]));
        assert_eq!(code, ExitCode::SUCCESS);
        let written = fs::read_to_string(&out).unwrap();
        assert!(written.contains("\"list\": \"iso-4217\""));
        assert!(written.contains("\"retrieved_at\": \"2026-05-27\""));
        assert!(written.contains("\"signature_alg\": \"sha256:identity\""));
        assert!(written.ends_with('\n'));
        let parsed: invoicekit_codelists::Manifest = serde_json::from_str(&written).unwrap();
        parsed.verify().unwrap();
    }

    #[test]
    fn run_is_idempotent_for_same_inputs() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("iso-4217.csv");
        fs::write(&src, "code,label,numeric,minor_units\nEUR,Euro,978,2\n").unwrap();
        let out = dir.path().join("iso-4217.json");
        let args = argv(&[
            "--list=iso-4217",
            "--source-file",
            src.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
            "--retrieved-at=2026-05-27",
        ]);
        assert_eq!(run(&args), ExitCode::SUCCESS);
        let first = fs::metadata(&out).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(run(&args), ExitCode::SUCCESS);
        let second = fs::metadata(&out).unwrap().modified().unwrap();
        assert_eq!(first, second, "no-change branch must not touch the file");
    }

    #[test]
    fn run_dry_run_does_not_write() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("iso-4217.csv");
        fs::write(&src, "code,label,numeric,minor_units\nEUR,Euro,978,2\n").unwrap();
        let out = dir.path().join("iso-4217.json");
        let code = run(&argv(&[
            "--list=iso-4217",
            "--source-file",
            src.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
            "--retrieved-at=2026-05-27",
            "--dry-run",
        ]));
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(!out.exists(), "--dry-run must not create the file");
    }

    #[test]
    fn run_rejects_unknown_list_with_exit_2() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("any.csv");
        fs::write(&src, "code,label,numeric,minor_units\nEUR,Euro,978,2\n").unwrap();
        let code = run(&argv(&[
            "--list=not-a-real-list",
            "--source-file",
            src.to_str().unwrap(),
        ]));
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_rejects_unreadable_source_file_with_exit_3() {
        let code = run(&argv(&[
            "--list=iso-4217",
            "--source-file=/nonexistent/path/to/file.csv",
        ]));
        assert_eq!(code, ExitCode::from(3));
    }

    #[test]
    fn run_rejects_malformed_csv_with_exit_3() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("bad.csv");
        fs::write(&src, "code,label,numeric\nEUR,Euro,978\n").unwrap();
        let out = dir.path().join("iso-4217.json");
        let code = run(&argv(&[
            "--list=iso-4217",
            "--source-file",
            src.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ]));
        assert_eq!(code, ExitCode::from(3));
        assert!(!out.exists(), "failed runs must not produce a partial file");
    }

    #[test]
    fn source_error_display_is_meaningful() {
        use invoicekit_codelists::sources::SourceError;
        let err = SourceError::UnknownList { list: "x".into() };
        assert!(format!("{err}").contains("no upstream source"));
    }
}
