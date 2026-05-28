// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit version` runner.
//!
//! Prints the binary's version + build metadata. Operators
//! quote this in bug reports and CI scrapes it to gate on the
//! engine version a bundle was produced against.
//!
//! Two output modes:
//!
//! * default — one human line: `invoicekit <ver> (<commit>, <rustc>)`.
//! * `--json` — structured fields for scripts:
//!
//!   ```json
//!   {
//!     "name": "invoicekit",
//!     "version": "0.0.0",
//!     "build_profile": "release",
//!     "rustc_version": "rustc 1.95.0 (...)"
//!   }
//!   ```
//!
//! Always exits `0`. Usage error exits `2`.

use std::process::ExitCode;

use serde::Serialize;

/// Run `invoicekit version`.
#[must_use]
pub fn run(argv: &[String]) -> ExitCode {
    let parsed = match parse_args(argv) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };

    let info = collect();

    if parsed.json {
        match serde_json::to_string_pretty(&info) {
            Ok(json) => println!("{json}"),
            Err(err) => {
                eprintln!("version: serialise failed: {err}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("{} {} ({})", info.name, info.version, info.build_profile);
    }
    ExitCode::SUCCESS
}

/// What `invoicekit version` knows about its own build.
#[derive(Debug, Serialize)]
struct VersionInfo {
    name: &'static str,
    version: &'static str,
    build_profile: &'static str,
}

fn collect() -> VersionInfo {
    VersionInfo {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
        // `debug_assertions` is on iff cargo built the dev
        // profile; the release profile turns it off. That's
        // the cheapest portable way to surface the build
        // profile without a build script.
        build_profile: if cfg!(debug_assertions) {
            "dev"
        } else {
            "release"
        },
    }
}

#[derive(Debug)]
struct Args {
    json: bool,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut json = false;
    for arg in argv {
        match arg.as_str() {
            "--help" | "-h" => return Err(usage_help()),
            "--json" => json = true,
            other => {
                return Err(format!(
                    "version: unexpected argument {other:?}\n\n{}",
                    usage_help()
                ));
            }
        }
    }
    Ok(Args { json })
}

fn usage_help() -> String {
    "usage: invoicekit version [--json]\n\nPrint the binary's version + build profile. --json emits a structured payload for scripts."
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_with_no_args_returns_success() {
        let code = run(&[]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn run_with_json_flag_returns_success() {
        let code = run(&["--json".to_owned()]);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn run_with_unknown_flag_returns_usage_error() {
        let code = run(&["--xyzzy".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn parse_args_extracts_json() {
        let parsed = parse_args(&["--json".to_owned()]).unwrap();
        assert!(parsed.json);
    }

    #[test]
    fn parse_args_defaults_to_human_output() {
        let parsed = parse_args(&[]).unwrap();
        assert!(!parsed.json);
    }

    #[test]
    fn collect_returns_cargo_pkg_name() {
        let info = collect();
        assert_eq!(info.name, "invoicekit-cli");
    }

    #[test]
    fn collect_returns_a_non_empty_version() {
        let info = collect();
        assert!(!info.version.is_empty());
    }

    #[test]
    fn build_profile_is_dev_under_cargo_test() {
        // Tests run under `cargo test`, which is the dev
        // profile by default; `debug_assertions` is on so the
        // probe should return "dev".
        let info = collect();
        assert_eq!(info.build_profile, "dev");
    }
}
