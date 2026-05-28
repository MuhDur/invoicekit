// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! Subcommand runners for the `invoicekit` CLI.
//!
//! Each subcommand exposes a single `run(argv: &[String]) -> ExitCode`
//! function so the same code path drives both the published
//! `invoicekit <subcommand>` invocation and any per-subcommand helper
//! binary (e.g. `invoicekit-cli --bin migrate-archive`).

pub mod capabilities;
pub mod codelist_update;
pub mod doctor;
pub mod migrate_archive;
pub mod pack;
pub mod replay;
pub mod unpack;
pub mod verify;
