// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit doctor` runner.
//!
//! Reports environment diagnostics with per-check pass/warn/fail
//! verdicts and a one-line remediation hint per failed check.
//!
//! v0 covers what's locally inspectable without spinning anything
//! up:
//!
//! * Rust toolchain present.
//! * Workspace layout (Cargo.toml + crates/ visible at the
//!   current working directory or at a `--workspace` override).
//! * Code-list data tree present.
//! * Reachability probes for the validator and signer sidecar
//!   ports (TCP connect against `127.0.0.1:<port>` with a short
//!   timeout so the command stays snappy when nothing's running).
//!
//! Exit codes:
//!
//! * `0` — every check passed (or only warnings).
//! * `1` — at least one check failed.
//! * `2` — usage error.

use std::io::Write;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

use serde::Serialize;

/// Run `invoicekit doctor`.
#[must_use]
pub fn run(argv: &[String]) -> ExitCode {
    let parsed = match parse_args(argv) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };

    let mut report = DoctorReport::default();
    report.push(check_rust_toolchain());
    report.push(check_workspace(&parsed.workspace));
    report.push(check_codelists(&parsed.workspace));
    for port in DEFAULT_SIDECAR_PORTS {
        report.push(check_sidecar_port(port.name, port.port));
    }

    report.finalize();

    if parsed.json {
        match serde_json::to_string_pretty(&report) {
            Ok(json) => {
                println!("{json}");
            }
            Err(err) => {
                eprintln!("doctor: report serialise failed: {err}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print_human(&report);
    }

    if report.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[derive(Debug)]
struct Args {
    workspace: PathBuf,
    json: bool,
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut workspace: Option<PathBuf> = None;
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
            "--workspace" => {
                let v = argv.get(i + 1).ok_or_else(|| {
                    format!("doctor: --workspace needs a path\n\n{}", usage_help())
                })?;
                workspace = Some(PathBuf::from(v));
                i += 2;
            }
            flag if flag.starts_with('-') => {
                return Err(format!("doctor: unknown flag {flag:?}\n\n{}", usage_help()));
            }
            positional => {
                return Err(format!(
                    "doctor: unexpected positional argument {positional:?}\n\n{}",
                    usage_help()
                ));
            }
        }
    }
    let workspace = workspace.unwrap_or_else(|| PathBuf::from("."));
    Ok(Args { workspace, json })
}

fn usage_help() -> String {
    "usage: invoicekit doctor [--workspace PATH] [--json]\n\nReport InvoiceKit environment diagnostics. Defaults to the current working directory as the workspace root. Prints human output unless --json is set.".to_owned()
}

/// One check's verdict.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
enum Verdict {
    Pass { detail: String },
    Warn { detail: String, remediation: String },
    Fail { detail: String, remediation: String },
}

impl Verdict {
    const fn is_fail(&self) -> bool {
        matches!(self, Self::Fail { .. })
    }
}

/// One row in the report.
#[derive(Clone, Debug, Serialize)]
struct Check {
    name: String,
    verdict: Verdict,
}

/// Aggregate report.
#[derive(Default, Debug, Serialize)]
struct DoctorReport {
    ok: bool,
    checks: Vec<Check>,
}

impl DoctorReport {
    fn push(&mut self, check: Check) {
        self.checks.push(check);
    }
    fn finalize(&mut self) {
        self.ok = !self.checks.iter().any(|c| c.verdict.is_fail());
    }
}

fn pass(name: &str, detail: impl Into<String>) -> Check {
    Check {
        name: name.to_owned(),
        verdict: Verdict::Pass {
            detail: detail.into(),
        },
    }
}

fn warn(name: &str, detail: impl Into<String>, remediation: impl Into<String>) -> Check {
    Check {
        name: name.to_owned(),
        verdict: Verdict::Warn {
            detail: detail.into(),
            remediation: remediation.into(),
        },
    }
}

fn fail(name: &str, detail: impl Into<String>, remediation: impl Into<String>) -> Check {
    Check {
        name: name.to_owned(),
        verdict: Verdict::Fail {
            detail: detail.into(),
            remediation: remediation.into(),
        },
    }
}

fn check_rust_toolchain() -> Check {
    match Command::new("rustc").arg("--version").output() {
        Ok(out) if out.status.success() => {
            let v = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            pass("rust-toolchain", v)
        }
        Ok(out) => fail(
            "rust-toolchain",
            format!("rustc exited with status {}", out.status),
            "install rustup from https://rustup.rs",
        ),
        Err(err) => fail(
            "rust-toolchain",
            format!("rustc not on PATH: {err}"),
            "install rustup from https://rustup.rs",
        ),
    }
}

fn check_workspace(root: &Path) -> Check {
    let manifest = root.join("Cargo.toml");
    let crates_dir = root.join("crates");
    if !manifest.is_file() {
        return fail(
            "workspace-layout",
            format!("missing {}", manifest.display()),
            "run `invoicekit doctor --workspace <repo-root>` from / pointing at the repo"
                .to_owned(),
        );
    }
    if !crates_dir.is_dir() {
        return fail(
            "workspace-layout",
            format!("missing {}", crates_dir.display()),
            "run from the repo root or pass --workspace <repo-root>".to_owned(),
        );
    }
    pass(
        "workspace-layout",
        format!("Cargo.toml + crates/ visible at {}", root.display()),
    )
}

fn check_codelists(root: &Path) -> Check {
    let codelist_dir = root.join("crates").join("codelists").join("data");
    if codelist_dir.is_dir() {
        pass(
            "codelist-data",
            format!("codelist data dir at {}", codelist_dir.display()),
        )
    } else {
        warn(
            "codelist-data",
            format!("codelist data dir not found at {}", codelist_dir.display()),
            "run `invoicekit codelist-update` to refresh local code lists",
        )
    }
}

#[derive(Clone, Copy)]
struct SidecarPort {
    name: &'static str,
    port: u16,
}

const DEFAULT_SIDECAR_PORTS: &[SidecarPort] = &[
    SidecarPort {
        name: "validator-kosit",
        port: 7001,
    },
    SidecarPort {
        name: "validator-phive",
        port: 7002,
    },
    SidecarPort {
        name: "validator-saxon",
        port: 7003,
    },
    SidecarPort {
        name: "validator-verapdf",
        port: 7004,
    },
    SidecarPort {
        name: "validator-phase4",
        port: 7005,
    },
    SidecarPort {
        name: "invoicekit-signer-agent",
        port: 7100,
    },
];

fn check_sidecar_port(name: &str, port: u16) -> Check {
    let addr_str = format!("127.0.0.1:{port}");
    // Resolve the address. If resolution fails we treat that as
    // an environment problem rather than an outage, but it's
    // exceedingly unlikely for a literal IP:port.
    let Some(addr) = addr_str
        .to_socket_addrs()
        .ok()
        .and_then(|mut iter| iter.next())
    else {
        return warn(
            &format!("sidecar-{name}"),
            format!("could not resolve {addr_str}"),
            "verify networking stack",
        );
    };
    match probe(addr, Duration::from_millis(200)) {
        Ok(()) => pass(
            &format!("sidecar-{name}"),
            format!("reachable at {addr_str}"),
        ),
        Err(_) => warn(
            &format!("sidecar-{name}"),
            format!("not reachable at {addr_str}"),
            format!("start the {name} sidecar (see deploy/docker-compose.yml)"),
        ),
    }
}

fn probe(addr: SocketAddr, timeout: Duration) -> std::io::Result<()> {
    TcpStream::connect_timeout(&addr, timeout).map(|_| ())
}

fn print_human(report: &DoctorReport) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    for check in &report.checks {
        let (tag, detail, remediation): (&str, &str, Option<&str>) = match &check.verdict {
            Verdict::Pass { detail } => ("PASS", detail, None),
            Verdict::Warn {
                detail,
                remediation,
            } => ("WARN", detail, Some(remediation)),
            Verdict::Fail {
                detail,
                remediation,
            } => ("FAIL", detail, Some(remediation)),
        };
        let _ = writeln!(handle, "[{tag}] {} — {detail}", check.name);
        if let Some(r) = remediation {
            let _ = writeln!(handle, "       remediation: {r}");
        }
    }
    let _ = writeln!(
        handle,
        "\noverall: {}",
        if report.ok { "OK" } else { "FAIL" }
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn run_with_unknown_flag_returns_usage_error() {
        let code = run(&["--xyzzy".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn run_with_unexpected_positional_returns_usage_error() {
        let code = run(&["positional".to_owned()]);
        assert_eq!(code, ExitCode::from(2));
    }

    #[test]
    fn parse_args_extracts_workspace_and_json() {
        let parsed = parse_args(&[
            "--workspace".to_owned(),
            "/tmp/foo".to_owned(),
            "--json".to_owned(),
        ])
        .unwrap();
        assert_eq!(parsed.workspace, PathBuf::from("/tmp/foo"));
        assert!(parsed.json);
    }

    #[test]
    fn parse_args_defaults_workspace_to_cwd_and_human_output() {
        let parsed = parse_args(&[]).unwrap();
        assert_eq!(parsed.workspace, PathBuf::from("."));
        assert!(!parsed.json);
    }

    #[test]
    fn check_workspace_passes_on_real_layout() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), b"[workspace]\n").unwrap();
        fs::create_dir(dir.path().join("crates")).unwrap();
        let check = check_workspace(dir.path());
        assert!(matches!(check.verdict, Verdict::Pass { .. }));
    }

    #[test]
    fn check_workspace_fails_when_cargo_toml_missing() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("crates")).unwrap();
        let check = check_workspace(dir.path());
        assert!(matches!(check.verdict, Verdict::Fail { .. }));
    }

    #[test]
    fn check_workspace_fails_when_crates_dir_missing() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), b"[workspace]\n").unwrap();
        let check = check_workspace(dir.path());
        assert!(matches!(check.verdict, Verdict::Fail { .. }));
    }

    #[test]
    fn check_codelists_warns_when_dir_missing() {
        let dir = tempfile::tempdir().unwrap();
        let check = check_codelists(dir.path());
        assert!(matches!(check.verdict, Verdict::Warn { .. }));
    }

    #[test]
    fn check_codelists_passes_when_dir_present() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("crates/codelists/data")).unwrap();
        let check = check_codelists(dir.path());
        assert!(matches!(check.verdict, Verdict::Pass { .. }));
    }

    #[test]
    fn check_sidecar_port_warns_when_nothing_listening() {
        // Port 1 on loopback is reserved/unused on dev machines;
        // pick a high improbable port to keep the test
        // CI-portable.
        let check = check_sidecar_port("imaginary", 65111);
        assert!(matches!(check.verdict, Verdict::Warn { .. }));
    }

    #[test]
    fn report_finalize_marks_ok_when_no_fails() {
        let mut r = DoctorReport::default();
        r.push(pass("a", "ok"));
        r.push(warn("b", "minor", "fix later"));
        r.finalize();
        assert!(r.ok);
    }

    #[test]
    fn report_finalize_marks_not_ok_when_any_fail() {
        let mut r = DoctorReport::default();
        r.push(pass("a", "ok"));
        r.push(fail("b", "broken", "fix it"));
        r.finalize();
        assert!(!r.ok);
    }
}
