// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! Subcommand runners for the `invoicekit` CLI.
//!
//! Each subcommand exposes a single `run(argv: &[String]) -> ExitCode`
//! function so the same code path drives both the published
//! `invoicekit <subcommand>` invocation and any per-subcommand helper
//! binary (e.g. `invoicekit-cli --bin migrate-archive`).

use std::process::ExitCode;

pub mod capabilities;
pub mod codelist_update;
pub mod diff;
pub mod doctor;
pub mod init;
pub mod migrate_archive;
pub mod pack;
pub mod repl;
pub mod replay;
pub mod show;
pub mod timestamp;
pub mod unpack;
pub mod validate;
pub mod verify;
pub mod version;

/// Dispatch a named `invoicekit` subcommand.
///
/// Returns [`None`] when `subcommand` is unknown, letting callers own
/// their context-specific unknown-command error text.
#[must_use]
pub fn dispatch(subcommand: &str, argv: &[String]) -> Option<ExitCode> {
    let code = match subcommand {
        "capabilities" => capabilities::run(argv),
        "codelist-update" => codelist_update::run(argv),
        "diff" => diff::run(argv),
        "doctor" => doctor::run(argv),
        "init" => init::run(argv),
        "migrate-archive" => migrate_archive::run(argv),
        "pack" => pack::run(argv),
        "replay" => replay::run(argv),
        "repl" => repl::run(argv),
        "show" => show::run(argv),
        "timestamp" => timestamp::run(argv),
        "unpack" => unpack::run(argv),
        "validate" => validate::run(argv),
        "verify" => verify::run(argv),
        "version" | "--version" | "-V" => version::run(argv),
        _ => return None,
    };
    Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatches_known_command() {
        assert_eq!(dispatch("version", &[]), Some(ExitCode::SUCCESS));
    }

    #[test]
    fn unknown_command_returns_none() {
        assert_eq!(dispatch("does-not-exist", &[]), None);
    }
}
