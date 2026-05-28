// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit peppol` runner.
//!
//! Two subcommands:
//!
//! * `invoicekit peppol doctor --credentials <path>` — load a BYOK
//!   credentials JSON file and run the `PeppolDoctor` checks
//!   (cert/key existence, PEM shape, endpoint URL, participant id).
//! * `invoicekit peppol show --credentials <path>` — pretty-print
//!   the credentials bundle (without revealing any passphrases) so
//!   operators can confirm the parse before wiring it through.
//!
//! Exit codes:
//!
//! * `0` — every doctor check passed (or `show` succeeded).
//! * `1` — at least one doctor check failed.
//! * `2` — usage error or credentials file unreadable.

use std::path::PathBuf;
use std::process::ExitCode;

use invoicekit_transmit_peppol_byok::{CheckStatus, PeppolCredentials, PeppolDoctor, StdFs};

/// Run `invoicekit peppol <subcommand>`.
#[must_use]
pub fn run(argv: &[String]) -> ExitCode {
    let Some((first, rest)) = argv.split_first() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let sub = first.as_str();
    match sub {
        "doctor" => run_doctor(rest),
        "show" => run_show(rest),
        "--help" | "-h" | "help" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("unknown peppol subcommand: {other}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

const USAGE: &str = "usage: invoicekit peppol <subcommand>\n\nSubcommands:\n  doctor --credentials <path>   Validate a BYOK credentials JSON bundle\n  show   --credentials <path>   Print the credentials shape (no secrets)";

#[derive(Debug)]
struct Args {
    credentials: PathBuf,
    json: bool,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut credentials: Option<PathBuf> = None;
    let mut json = false;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--credentials" | "-c" => {
                i += 1;
                let v = argv
                    .get(i)
                    .ok_or_else(|| "missing value for --credentials".to_owned())?;
                credentials = Some(PathBuf::from(v));
            }
            "--json" => json = true,
            other => return Err(format!("unknown argument: {other}")),
        }
        i += 1;
    }
    let credentials = credentials.ok_or_else(|| "--credentials is required".to_owned())?;
    Ok(Args { credentials, json })
}

fn run_doctor(argv: &[String]) -> ExitCode {
    let parsed = match parse_args(argv) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let creds = match PeppolCredentials::from_json_file(&parsed.credentials) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to load credentials: {e}");
            return ExitCode::from(2);
        }
    };
    let fs = StdFs;
    let doctor = PeppolDoctor::new(&fs);
    let report = doctor.check(&creds);

    if parsed.json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("failed to serialise report: {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        print_report_human(&report);
    }
    if report.passed() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn print_report_human(report: &invoicekit_transmit_peppol_byok::DoctorReport) {
    for row in &report.rows {
        let (tag, detail) = match &row.status {
            CheckStatus::Ok => ("OK   ", String::new()),
            CheckStatus::Failed(msg) => ("FAIL ", format!(" — {msg}")),
            CheckStatus::Skipped(msg) => ("SKIP ", format!(" — {msg}")),
        };
        println!("  [{tag}] {}{}", row.id, detail);
    }
    println!();
    if report.passed() {
        println!("peppol doctor: all checks passed");
    } else {
        println!("peppol doctor: {} check(s) failed", report.failures().len());
    }
}

fn run_show(argv: &[String]) -> ExitCode {
    let parsed = match parse_args(argv) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let creds = match PeppolCredentials::from_json_file(&parsed.credentials) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to load credentials: {e}");
            return ExitCode::from(2);
        }
    };
    // Never echo passphrases, even if accidentally set. The
    // `key_passphrase_env` field is only the NAME of the env var
    // — the variable contents stay in-process.
    let view = serde_json::json!({
        "participant_id": format!(
            "{}::{}",
            creds.participant_id.scheme, creds.participant_id.value
        ),
        "cert_pem_path": creds.cert_pem_path,
        "key_pem_path": creds.key_pem_path,
        "key_passphrase_env": creds.key_passphrase_env,
        "endpoint_url": creds.endpoint_url,
        "sml_mode": creds.sml_mode.slug(),
        "transport": creds.transport.slug(),
        "labels": creds.labels,
    });
    match serde_json::to_string_pretty(&view) {
        Ok(s) => {
            println!("{s}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("failed to serialise: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir();
        let n: u128 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = base.join(format!("ik-cli-peppol-{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_happy_bundle(tmp: &std::path::Path) -> PathBuf {
        let cert = tmp.join("cert.pem");
        let key = tmp.join("key.pem");
        fs::write(
            &cert,
            "-----BEGIN CERTIFICATE-----\nMIIBLOL\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        fs::write(
            &key,
            "-----BEGIN PRIVATE KEY-----\nMIIBLOL\n-----END PRIVATE KEY-----\n",
        )
        .unwrap();
        let creds = tmp.join("creds.json");
        let body = r#"{
            "participant_id": {"scheme": "iso6523-actorid-upis", "value": "0192:991825827"},
            "cert_pem_path": "cert.pem",
            "key_pem_path": "key.pem",
            "endpoint_url": "https://ap.example.com/as4",
            "sml_mode": "test",
            "transport": "partner"
        }"#;
        fs::write(&creds, body).unwrap();
        creds
    }

    #[test]
    fn no_args_returns_usage_error() {
        assert_eq!(run(&[]), ExitCode::from(2));
    }

    #[test]
    fn unknown_subcommand_returns_usage_error() {
        assert_eq!(run(&["nope".to_owned()]), ExitCode::from(2));
    }

    #[test]
    fn doctor_missing_credentials_arg_is_usage_error() {
        assert_eq!(run(&["doctor".to_owned()]), ExitCode::from(2));
    }

    #[test]
    fn doctor_on_happy_bundle_returns_success() {
        let tmp = tempdir();
        let creds = write_happy_bundle(&tmp);
        let code = run(&[
            "doctor".to_owned(),
            "--credentials".to_owned(),
            creds.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn doctor_on_broken_endpoint_returns_failure_exit_code() {
        let tmp = tempdir();
        let _ = write_happy_bundle(&tmp);
        let bad = tmp.join("bad.json");
        fs::write(
            &bad,
            r#"{
            "participant_id": {"scheme": "iso6523-actorid-upis", "value": "0192:991825827"},
            "cert_pem_path": "cert.pem",
            "key_pem_path": "key.pem",
            "endpoint_url": "http://insecure.example.com/as4",
            "sml_mode": "test",
            "transport": "partner"
        }"#,
        )
        .unwrap();
        let code = run(&[
            "doctor".to_owned(),
            "--credentials".to_owned(),
            bad.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ]);
        assert_eq!(code, ExitCode::from(1));
    }

    #[test]
    fn show_round_trips_bundle_without_secrets() {
        let tmp = tempdir();
        let creds = write_happy_bundle(&tmp);
        let code = run(&[
            "show".to_owned(),
            "--credentials".to_owned(),
            creds.to_string_lossy().into_owned(),
        ]);
        assert_eq!(code, ExitCode::SUCCESS);
    }
}
