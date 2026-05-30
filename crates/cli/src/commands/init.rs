// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit init` runner.
//!
//! Scaffolds a starter `invoicekit/` directory in the caller's
//! project with sensible defaults plus a generated invoice
//! draft the operator can iterate from. Detects the host
//! language/framework so the scaffold matches the surrounding
//! repo (Node, Python, Go, Java, .NET, Rust).
//!
//! Today's flow:
//!
//! 1. Walk the caller's cwd for marker files
//!    (`package.json`, `pyproject.toml`, `go.mod`,
//!    `pom.xml` / `build.gradle*`, `*.csproj`, `Cargo.toml`)
//!    and pick the strongest signal.
//! 2. Read `--country` (default `DE` for EU readiness) and
//!    record it.
//! 3. Write `invoicekit/draft.json` (a typed starter
//!    invoice) and `invoicekit/config.toml` (engine
//!    settings: tenant, country, framework).
//!
//! VIES lookup of the supplier VAT is stubbed today (no
//! network) — the printed report flags it as
//! `Skipped { reason: "no VIES client wired yet" }` so the
//! operator knows it's not silently passing.
//!
//! Exit codes:
//!
//! * `0` — scaffold written.
//! * `1` — write failure mid-scaffold.
//! * `2` — usage error (bad args, output already exists
//!   without `--force`).

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use serde::Serialize;

/// Run `invoicekit init`.
#[must_use]
pub fn run(argv: &[String]) -> ExitCode {
    let parsed = match parse_args(argv) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };

    let cwd = match std::env::current_dir() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("init: cannot read cwd: {err}");
            return ExitCode::FAILURE;
        }
    };
    let target_dir = cwd.join("invoicekit");

    if target_dir.exists() && !parsed.force {
        eprintln!(
            "init: {} already exists — pass --force to overwrite",
            target_dir.display()
        );
        return ExitCode::from(2);
    }

    let framework = detect_framework(&cwd);
    let vies_outcome = stub_vies_check(parsed.supplier_vat.as_deref());

    if let Err(err) = fs::create_dir_all(&target_dir) {
        eprintln!("init: cannot create {}: {err}", target_dir.display());
        return ExitCode::FAILURE;
    }

    // Write the typed draft + config.
    let draft = sample_draft_json(&parsed.country);
    if let Err(err) = fs::write(target_dir.join("draft.json"), &draft) {
        eprintln!("init: cannot write draft.json: {err}");
        return ExitCode::FAILURE;
    }
    let config = sample_config_toml(&parsed, framework);
    if let Err(err) = fs::write(target_dir.join("config.toml"), &config) {
        eprintln!("init: cannot write config.toml: {err}");
        return ExitCode::FAILURE;
    }

    let report = InitReport {
        target_dir: target_dir.display().to_string(),
        country: parsed.country.clone(),
        framework: framework.label(),
        vies: vies_outcome,
        files_written: vec!["draft.json".to_owned(), "config.toml".to_owned()],
    };

    if parsed.json {
        match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(err) => {
                eprintln!("init: report serialise failed: {err}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print_human(&report);
    }

    ExitCode::SUCCESS
}

/// Detected framework / language flavour.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Framework {
    /// `package.json` present.
    Node,
    /// `pyproject.toml` / `setup.py` / `requirements.txt` present.
    Python,
    /// `go.mod` present.
    Go,
    /// `pom.xml` / `build.gradle[.kts]` present.
    Java,
    /// `*.csproj` present.
    DotNet,
    /// `Cargo.toml` present (and none of the above match first).
    Rust,
    /// No marker file matched.
    Unknown,
}

impl Framework {
    /// Stable label for the JSON/human report.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Python => "python",
            Self::Go => "go",
            Self::Java => "java",
            Self::DotNet => "dotnet",
            Self::Rust => "rust",
            Self::Unknown => "unknown",
        }
    }
}

/// Detect the framework signature in `dir`. Priority order
/// matches the marker-strength of each tooling ecosystem.
#[must_use]
pub fn detect_framework(dir: &Path) -> Framework {
    if dir.join("package.json").is_file() {
        return Framework::Node;
    }
    if dir.join("pyproject.toml").is_file()
        || dir.join("setup.py").is_file()
        || dir.join("requirements.txt").is_file()
    {
        return Framework::Python;
    }
    if dir.join("go.mod").is_file() {
        return Framework::Go;
    }
    if dir.join("pom.xml").is_file()
        || dir.join("build.gradle").is_file()
        || dir.join("build.gradle.kts").is_file()
    {
        return Framework::Java;
    }
    if fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.path().extension().is_some_and(|ext| ext == "csproj"))
    }) {
        return Framework::DotNet;
    }
    if dir.join("Cargo.toml").is_file() {
        return Framework::Rust;
    }
    Framework::Unknown
}

/// Outcome of the VIES supplier-VAT check. Today it never
/// hits the wire — the live HTTP client lands in a follow-up
/// `invoicekit-vies-client` crate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum ViesOutcome {
    /// Operator didn't supply `--supplier-vat`.
    Skipped {
        /// One-line operator-readable reason.
        reason: String,
    },
    /// Supplier VAT failed the local shape check.
    InvalidShape {
        /// The VAT id as supplied.
        vat: String,
        /// One-line operator-readable reason.
        reason: String,
    },
    /// Supplier VAT passed the local shape check; the live
    /// VIES backend is not yet wired so we report
    /// "shape ok, network skipped".
    ShapeOk {
        /// The VAT id as supplied (echoed for confirmation).
        vat: String,
    },
}

#[derive(Debug, Serialize)]
struct InitReport {
    target_dir: String,
    country: String,
    framework: &'static str,
    vies: ViesOutcome,
    files_written: Vec<String>,
}

fn print_human(r: &InitReport) {
    println!("invoicekit init complete");
    println!("  target:    {}", r.target_dir);
    println!("  country:   {}", r.country);
    println!("  framework: {}", r.framework);
    print!("  vies:      ");
    match &r.vies {
        ViesOutcome::Skipped { reason } => println!("skipped — {reason}"),
        ViesOutcome::InvalidShape { vat, reason } => {
            println!("invalid-shape — {vat:?}: {reason}");
        }
        ViesOutcome::ShapeOk { vat } => println!("shape-ok — {vat}"),
    }
    println!("  files:");
    for f in &r.files_written {
        println!("    + {f}");
    }
}

/// Heuristic VIES check used today.
///
/// Production version delegates to a `vies::Client` shipped
/// in a follow-up crate. Today we just shape-check the VAT
/// string so the report can be honest about what ran.
pub(crate) fn stub_vies_check(supplier_vat: Option<&str>) -> ViesOutcome {
    let Some(vat) = supplier_vat else {
        return ViesOutcome::Skipped {
            reason: "no supplier VAT supplied; pass --supplier-vat <ISO2-prefix + digits>"
                .to_owned(),
        };
    };
    if vat.len() < 4 || vat.len() > 14 {
        return ViesOutcome::InvalidShape {
            vat: vat.to_owned(),
            reason: "VAT must be 4-14 chars (ISO2 country prefix + digits)".to_owned(),
        };
    }
    // The length gate counts bytes; a multibyte character could otherwise land
    // mid-codepoint at the `split_at(2)` boundary below and panic. A valid VAT
    // is ASCII-only, so reject anything else up front.
    if !vat.is_ascii() {
        return ViesOutcome::InvalidShape {
            vat: vat.to_owned(),
            reason: "VAT must be ASCII (ISO2 country prefix + ASCII alphanumeric digits)"
                .to_owned(),
        };
    }
    let (prefix, digits) = vat.split_at(2);
    let prefix_ok = prefix
        .chars()
        .all(|c| c.is_ascii_alphabetic() && c.is_ascii_uppercase());
    let digits_ok = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_alphanumeric());
    if !(prefix_ok && digits_ok) {
        return ViesOutcome::InvalidShape {
            vat: vat.to_owned(),
            reason: "VAT must be uppercase ISO2 country prefix + ASCII alphanumeric digits"
                .to_owned(),
        };
    }
    ViesOutcome::ShapeOk {
        vat: vat.to_owned(),
    }
}

#[derive(Debug)]
struct Args {
    country: String,
    supplier_vat: Option<String>,
    tenant: String,
    force: bool,
    json: bool,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut country = "DE".to_owned();
    let mut supplier_vat: Option<String> = None;
    let mut tenant = "unset-tenant".to_owned();
    let mut force = false;
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
            "--force" => {
                force = true;
                i += 1;
            }
            "--country" => {
                let v = argv
                    .get(i + 1)
                    .ok_or_else(|| format!("init: --country needs a value\n\n{}", usage_help()))?;
                country.clone_from(v);
                i += 2;
            }
            "--supplier-vat" => {
                let v = argv.get(i + 1).ok_or_else(|| {
                    format!("init: --supplier-vat needs a value\n\n{}", usage_help())
                })?;
                supplier_vat = Some(v.clone());
                i += 2;
            }
            "--tenant" => {
                let v = argv
                    .get(i + 1)
                    .ok_or_else(|| format!("init: --tenant needs a value\n\n{}", usage_help()))?;
                tenant.clone_from(v);
                i += 2;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("init: unknown flag {flag:?}\n\n{}", usage_help()));
            }
            positional => {
                return Err(format!(
                    "init: unexpected positional {positional:?}\n\n{}",
                    usage_help()
                ));
            }
        }
    }
    Ok(Args {
        country,
        supplier_vat,
        tenant,
        force,
        json,
    })
}

fn usage_help() -> String {
    "usage: invoicekit init [--country DE] [--tenant ID] [--supplier-vat VATID] [--force] [--json]\n\nScaffold an invoicekit/ directory in the current working directory with a starter invoice draft + config. Detects the host language/framework. VIES supplier-VAT shape-check is stubbed today (no network)."
        .to_owned()
}

fn sample_draft_json(country: &str) -> String {
    format!(
        r#"{{
  "schema_version": "1.0",
  "country": "{country}",
  "id": "INV-DEMO-1",
  "issue_date": "2026-05-28",
  "currency": "EUR",
  "seller": {{
    "name": "Acme GmbH",
    "vat_id": "DE123456789",
    "address": {{ "country": "{country}" }}
  }},
  "buyer": {{
    "name": "Beispiel AG",
    "vat_id": "DE987654321",
    "address": {{ "country": "DE" }}
  }},
  "lines": [
    {{
      "id": "1",
      "description": "Consulting services",
      "quantity": "1.000",
      "unit_code": "HUR",
      "net_unit_price": "100.00",
      "tax_category": "S",
      "tax_rate_percent": "19.00"
    }}
  ]
}}
"#
    )
}

fn sample_config_toml(args: &Args, framework: Framework) -> String {
    format!(
        r#"# Generated by `invoicekit init`. Edit freely.

[engine]
country = "{country}"
tenant  = "{tenant}"

[engine.framework]
detected = "{framework_label}"

[engine.draft]
path = "draft.json"
"#,
        country = args.country,
        tenant = args.tenant,
        framework_label = framework.label(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // Tests that mutate the process-global current directory must not run
    // concurrently with each other (cargo runs tests multi-threaded), or one
    // test's `set_current_dir` races another's `run(&[])` and the scaffold
    // lands in the wrong directory. Serialize them on this lock.
    static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn run_with_unknown_flag_returns_usage_error() {
        let code = run(&["--xyzzy".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn parse_args_extracts_all_flags() {
        let parsed = parse_args(&[
            "--country".to_owned(),
            "FR".to_owned(),
            "--tenant".to_owned(),
            "T".to_owned(),
            "--supplier-vat".to_owned(),
            "DE123456789".to_owned(),
            "--force".to_owned(),
            "--json".to_owned(),
        ])
        .unwrap();
        assert_eq!(parsed.country, "FR");
        assert_eq!(parsed.tenant, "T");
        assert_eq!(parsed.supplier_vat.as_deref(), Some("DE123456789"));
        assert!(parsed.force);
        assert!(parsed.json);
    }

    #[test]
    fn parse_args_defaults_to_de_country() {
        let parsed = parse_args(&[]).unwrap();
        assert_eq!(parsed.country, "DE");
        assert!(parsed.supplier_vat.is_none());
        assert!(!parsed.force);
    }

    #[test]
    fn stub_vies_check_skipped_when_no_vat() {
        match stub_vies_check(None) {
            ViesOutcome::Skipped { .. } => {}
            other => panic!("expected Skipped, got {other:?}"),
        }
    }

    #[test]
    fn stub_vies_check_shape_ok_for_well_formed_vat() {
        match stub_vies_check(Some("DE123456789")) {
            ViesOutcome::ShapeOk { vat } => assert_eq!(vat, "DE123456789"),
            other => panic!("expected ShapeOk, got {other:?}"),
        }
    }

    #[test]
    fn stub_vies_check_invalid_for_lowercase_prefix() {
        match stub_vies_check(Some("de123456789")) {
            ViesOutcome::InvalidShape { .. } => {}
            other => panic!("expected InvalidShape, got {other:?}"),
        }
    }

    #[test]
    fn stub_vies_check_invalid_for_too_short() {
        match stub_vies_check(Some("DE")) {
            ViesOutcome::InvalidShape { .. } => {}
            other => panic!("expected InvalidShape, got {other:?}"),
        }
    }

    #[test]
    fn stub_vies_check_invalid_for_multibyte_vat() {
        // `é` is two UTF-8 bytes, so a byte-length check can pass while
        // byte index 2 lands mid-character. The split must not panic.
        let vat = "Aé12";
        assert!(vat.len() >= 4, "input must clear the byte-length gate");
        match stub_vies_check(Some(vat)) {
            ViesOutcome::InvalidShape { .. } => {}
            other => panic!("expected InvalidShape, got {other:?}"),
        }
    }

    #[test]
    fn detect_framework_picks_node_when_package_json_present() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("package.json"), b"{}").unwrap();
        assert_eq!(detect_framework(dir.path()), Framework::Node);
    }

    #[test]
    fn detect_framework_picks_rust_when_only_cargo_toml() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), b"[package]\n").unwrap();
        assert_eq!(detect_framework(dir.path()), Framework::Rust);
    }

    #[test]
    fn detect_framework_returns_unknown_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_framework(dir.path()), Framework::Unknown);
    }

    #[test]
    fn detect_framework_picks_dotnet_when_csproj_present() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("App.csproj"), b"<Project />").unwrap();
        assert_eq!(detect_framework(dir.path()), Framework::DotNet);
    }

    #[test]
    fn run_in_empty_dir_writes_scaffold_files() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let code = run(&[]);
        std::env::set_current_dir(prev).unwrap();
        assert_eq!(code, ExitCode::SUCCESS);
        assert!(dir.path().join("invoicekit").join("draft.json").is_file());
        assert!(dir.path().join("invoicekit").join("config.toml").is_file());
    }

    #[test]
    fn run_refuses_to_overwrite_existing_dir_without_force() {
        let _cwd = CWD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("invoicekit")).unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let code = run(&[]);
        std::env::set_current_dir(prev).unwrap();
        assert_eq!(code, ExitCode::from(2));
    }
}
