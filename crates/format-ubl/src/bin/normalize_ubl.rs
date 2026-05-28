// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Normalize UBL XML through the InvoiceKit IR and serializer.

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;

use invoicekit_format_ubl::{from_xml, to_xml};

fn main() -> ExitCode {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    let input = if args.first().and_then(|arg| arg.to_str()) == Some("--stdin") {
        let label = args
            .get(1)
            .and_then(|arg| arg.to_str())
            .unwrap_or("stdin")
            .to_owned();
        read_stdin(label)
    } else if args.len() == 1 {
        read_path(Path::new(&args[0]))
    } else {
        eprintln!(
            "usage: invoicekit-ubl-normalize <fixture.xml>\n\
             usage: invoicekit-ubl-normalize --stdin <fixture-label>"
        );
        return ExitCode::from(2);
    };

    let (label, xml) = match input {
        Ok(input) => input,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    let normalized = match normalize(&label, &xml) {
        Ok(normalized) => normalized,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = io::stdout().lock().write_all(normalized.as_bytes()) {
        eprintln!("failed to write normalized UBL XML: {error}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn read_stdin(label: String) -> Result<(String, String), String> {
    let mut xml = String::new();
    io::stdin()
        .read_to_string(&mut xml)
        .map_err(|error| format!("failed to read UBL XML from stdin: {error}"))?;
    Ok((label, xml))
}

fn read_path(path: &Path) -> Result<(String, String), String> {
    let label = path.display().to_string();
    let xml =
        fs::read_to_string(path).map_err(|error| format!("failed to read {label}: {error}"))?;
    Ok((label, xml))
}

fn normalize(label: &str, xml: &str) -> Result<String, String> {
    let (document, ledger) =
        from_xml(xml).map_err(|error| format!("failed to parse {label}: {error}"))?;
    if !ledger.lost.is_empty() || !ledger.warnings.is_empty() {
        return Err(format!(
            "refusing lossy UBL normalization for {label}: lost={:?}, warnings={:?}",
            ledger.lost, ledger.warnings
        ));
    }
    to_xml(&document).map_err(|error| format!("failed to serialize {label}: {error}"))
}
