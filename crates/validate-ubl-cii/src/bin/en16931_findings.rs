// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Emit pure-Rust EN 16931 validation findings for parity harnesses.

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;

use invoicekit_validate_ubl_cii::{validate_xml, DocumentSyntax, En16931Report};
use serde_json::{json, Value};

fn main() -> ExitCode {
    let paths = env::args_os().skip(1).collect::<Vec<_>>();
    if paths.is_empty() {
        eprintln!(
            "usage: invoicekit-en16931-findings <fixture.xml> [<fixture.xml> ...]\n\
             usage: invoicekit-en16931-findings --stdin <fixture-label>"
        );
        return ExitCode::from(2);
    }

    let reports = if paths.first().and_then(|arg| arg.to_str()) == Some("--stdin") {
        let label = paths
            .get(1)
            .and_then(|arg| arg.to_str())
            .unwrap_or("stdin")
            .to_owned();
        let mut xml = String::new();
        if let Err(error) = io::stdin().read_to_string(&mut xml) {
            vec![json!({
                "path": label,
                "valid": false,
                "rule_ids": [],
                "findings": [],
                "error": {
                    "kind": "stdin",
                    "message": error.to_string(),
                },
            })]
        } else {
            vec![report_for_xml(&label, &xml)]
        }
    } else {
        paths
            .iter()
            .map(|path| report_for_path(Path::new(path)))
            .collect::<Vec<_>>()
    };

    let stdout = io::stdout();
    let mut lock = stdout.lock();
    if let Err(error) = serde_json::to_writer_pretty(&mut lock, &reports)
        .and_then(|()| writeln!(lock).map_err(serde_json::Error::io))
    {
        eprintln!("failed to write findings JSON: {error}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn report_for_path(path: &Path) -> Value {
    let display_path = path.display().to_string();
    let xml = match fs::read_to_string(path) {
        Ok(xml) => xml,
        Err(error) => {
            return json!({
                "path": display_path,
                "valid": false,
                "rule_ids": [],
                "findings": [],
                "error": {
                    "kind": "read",
                    "message": error.to_string(),
                },
            });
        }
    };

    match validate_xml(&xml) {
        Ok(report) => report_for_success(&display_path, &report),
        Err(error) => json!({
            "path": display_path,
            "valid": false,
            "rule_ids": [],
            "findings": [],
            "error": {
                "kind": "validate",
                "message": error.to_string(),
            },
        }),
    }
}

fn report_for_xml(label: &str, xml: &str) -> Value {
    match validate_xml(xml) {
        Ok(report) => report_for_success(label, &report),
        Err(error) => json!({
            "path": label,
            "valid": false,
            "rule_ids": [],
            "findings": [],
            "error": {
                "kind": "validate",
                "message": error.to_string(),
            },
        }),
    }
}

fn report_for_success(display_path: &str, report: &En16931Report) -> Value {
    let findings = report
        .findings
        .iter()
        .map(|finding| {
            json!({
                "rule_id": finding.rule_id.as_str(),
                "severity": finding.severity,
                "term": finding.term.code(),
                "location": finding.location,
            })
        })
        .collect::<Vec<_>>();
    let mut rule_ids = report
        .findings
        .iter()
        .map(|finding| finding.rule_id.as_str().to_owned())
        .collect::<Vec<_>>();
    rule_ids.sort_unstable();
    rule_ids.dedup();

    json!({
        "path": display_path,
        "valid": rule_ids.is_empty(),
        "syntax": syntax_name(report.syntax),
        "coverage": {
            "total": report.coverage.total,
            "implemented": report.coverage.implemented,
            "deferred_ir_gap": report.coverage.deferred_ir_gap,
        },
        "rule_ids": rule_ids,
        "findings": findings,
    })
}

fn syntax_name(syntax: DocumentSyntax) -> &'static str {
    match syntax {
        DocumentSyntax::Ubl => "ubl",
        DocumentSyntax::Cii => "cii",
    }
}
