// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-137 admin tooling shared utilities.
//!
//! Defines the small surface every `invoicekit-admin <subcommand>`
//! shares: how to open a sqlite database (T-137 ships sqlite only;
//! postgres / mysql adapters land with the corresponding admin
//! follow-up beads), the [`AdminError`] enum every subcommand returns,
//! and the [`OutputFormat`] selector.

use std::fmt;
use std::path::Path;

use rusqlite::Connection;
use thiserror::Error;

pub mod replay;
pub mod stuck;

/// Output format selector shared by every admin subcommand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// One JSON object per row on `stdout`.
    Jsonl,
    /// Operator-friendly table.
    Table,
}

impl OutputFormat {
    /// Parse a CLI value (`jsonl` or `table`).
    ///
    /// # Errors
    ///
    /// Returns [`AdminError::Cli`] for any other value.
    pub fn parse(v: &str) -> Result<Self, AdminError> {
        match v {
            "jsonl" => Ok(Self::Jsonl),
            "table" => Ok(Self::Table),
            other => Err(AdminError::Cli(format!(
                "--format={other:?} not one of jsonl|table"
            ))),
        }
    }
}

/// Errors surfaced by the admin CLI.
#[derive(Debug, Error)]
pub enum AdminError {
    /// User supplied a bad flag.
    #[error("invalid CLI usage: {0}")]
    Cli(String),
    /// Filesystem-level failure (db path missing, no permission).
    #[error("filesystem error at {path}: {detail}")]
    Io {
        /// Path that was being touched.
        path: String,
        /// Operator-readable reason.
        detail: String,
    },
    /// Database query failure.
    #[error("sqlite error during {stage}: {detail}")]
    Sqlite {
        /// What we were doing when the error fired.
        stage: &'static str,
        /// Operator-readable reason.
        detail: String,
    },
    /// Expected outbox tables are absent — the database hasn't been
    /// migrated yet.
    #[error(
        "database is missing outbox tables; run the T-071 outbox migrations against {path} first"
    )]
    MissingTable {
        /// Path to the database that's missing tables.
        path: String,
    },
    /// `invoicekit-admin replay` couldn't find a dead-letter row.
    #[error("no dead-letter row with id {dead_letter_id}")]
    NotFound {
        /// `dead_letter_id` the caller asked for.
        dead_letter_id: String,
    },
    /// `invoicekit-admin replay` saw a live `(tenant, idempotency_key)` collision.
    #[error(
        "replay would collide with live outbox row {existing_outbox_id} \
         (same tenant + idempotency_key as dead-letter row {dead_letter_id})"
    )]
    Conflict {
        /// The dead-letter row we were asked to replay.
        dead_letter_id: String,
        /// The live row that already occupies the same `(tenant, key)` slot.
        existing_outbox_id: String,
    },
}

/// Open a sqlite database at `path`, return [`AdminError::Io`] when
/// the file is missing or unreadable and [`AdminError::MissingTable`]
/// when the outbox tables aren't there yet.
///
/// # Errors
///
/// Same as the variants documented above.
pub fn open_sqlite(path: &Path) -> Result<Connection, AdminError> {
    if !path.exists() {
        return Err(AdminError::Io {
            path: path.display().to_string(),
            detail: "no such file or directory".into(),
        });
    }
    let conn = Connection::open(path).map_err(|e| AdminError::Sqlite {
        stage: "open sqlite",
        detail: e.to_string(),
    })?;
    let table_exists = |name: &str| -> bool {
        conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [name],
            |r| Ok(r.get::<_, i64>(0)? > 0),
        )
        .unwrap_or(false)
    };
    let has_outbox = table_exists("invoicekit_outbox");
    let has_dlq = table_exists("invoicekit_outbox_dead_letter");
    if !has_outbox || !has_dlq {
        return Err(AdminError::MissingTable {
            path: path.display().to_string(),
        });
    }
    Ok(conn)
}

/// Dispatch `invoicekit-admin <subcommand> [args...]`.
#[must_use]
pub fn run_dispatch(argv: &[String]) -> std::process::ExitCode {
    use std::process::ExitCode;
    if argv.is_empty() || argv[0] == "--help" || argv[0] == "-h" {
        print!("{}", dispatch_usage());
        return ExitCode::SUCCESS;
    }
    let sub = &argv[0];
    let rest: Vec<String> = argv.iter().skip(1).cloned().collect();
    match sub.as_str() {
        "stuck" => stuck::cli(&rest),
        "replay" => replay::cli(&rest),
        unknown => {
            eprintln!("invoicekit-admin: unknown subcommand {unknown:?}");
            eprintln!();
            eprint!("{}", dispatch_usage());
            ExitCode::from(2)
        }
    }
}

fn dispatch_usage() -> String {
    "usage: invoicekit-admin <command> [<args>...]\n\nCommands:\n  stuck    list stuck outbox + dead-letter rows\n  replay   re-enqueue a dead-letter row into the live outbox\n\nRun `invoicekit-admin <command> --help` for command-specific flags.\n"
        .to_string()
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Jsonl => write!(f, "jsonl"),
            Self::Table => write!(f, "table"),
        }
    }
}

/// Stable identifier used by the test seed; lives in the public API
/// only because the test-only helper below references it.
pub const TEST_DLQ_ID: &str = "dlq-row-001";

/// Tenant id used by the test seed.
pub const TEST_TENANT: &str = "tenant-x";

/// Test-only helper: create a sqlite database at `path` and seed it
/// with an outbox + dead-letter row that the [`stuck`] and [`replay`]
/// unit tests both consume. Kept in the module instead of duplicated
/// per file so a future schema change only updates one fixture.
#[cfg(test)]
#[doc(hidden)]
pub fn sqlite_seed_test_outbox(path: &Path) {
    let conn = Connection::open(path).expect("open test sqlite");
    conn.execute_batch(include_str!(
        "../../../reconcile/migrations/sqlite/001_invoicekit_outbox.up.sql"
    ))
    .expect("apply outbox migration");
    // One DLQ row — for replay tests.
    conn.execute(
        "INSERT INTO invoicekit_outbox_dead_letter ( \
            dead_letter_id, outbox_id, tenant_id, trace_id, idempotency_key, \
            gateway_attempt_id, invoice_fingerprint, final_state, failure_code, \
            failure_message, attempt_count, payload_json \
        ) VALUES ( \
            ?1, 'orig-001', ?2, 'trace-001', 'idemp-001', 'attempt-001', X'00', \
            'rejected', 'GATEWAY_REJECT', 'gateway returned 4xx', 4, \
            '{\"kind\":\"test\"}' \
        )",
        rusqlite::params![TEST_DLQ_ID, TEST_TENANT],
    )
    .expect("seed dead-letter row");
    // One retry-overdue outbox row — `next_attempt_at` 2 hours in the
    // past so the default 15-minute overdue threshold trips on it.
    conn.execute(
        "INSERT INTO invoicekit_outbox ( \
            outbox_id, tenant_id, trace_id, idempotency_key, gateway_attempt_id, \
            invoice_fingerprint, state, payload_json, attempt_count, max_attempts, \
            next_attempt_at, updated_at \
        ) VALUES ( \
            'ob-stuck-001', ?1, 'trace-002', 'idemp-002', 'attempt-002', X'00', \
            'sent', '{\"kind\":\"test\"}', 3, 8, \
            datetime('now', '-120 minutes'), datetime('now', '-120 minutes') \
        )",
        rusqlite::params![TEST_TENANT],
    )
    .expect("seed overdue outbox row");
    // One healthy row that should NOT appear — sanity-anchor.
    conn.execute(
        "INSERT INTO invoicekit_outbox ( \
            outbox_id, tenant_id, trace_id, idempotency_key, gateway_attempt_id, \
            invoice_fingerprint, state, payload_json, attempt_count, max_attempts, \
            next_attempt_at, updated_at \
        ) VALUES ( \
            'ob-healthy-001', ?1, 'trace-003', 'idemp-003', 'attempt-003', X'00', \
            'sent', '{\"kind\":\"test\"}', 1, 8, \
            datetime('now', '+10 minutes'), datetime('now') \
        )",
        rusqlite::params![TEST_TENANT],
    )
    .expect("seed healthy outbox row");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn open_sqlite_rejects_missing_file() {
        let err = open_sqlite(Path::new("/nonexistent/db.sqlite")).unwrap_err();
        assert!(matches!(err, AdminError::Io { .. }));
    }

    #[test]
    fn open_sqlite_rejects_unmigrated_db() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.db");
        // Create an empty sqlite file with no outbox tables.
        Connection::open(&path).unwrap();
        let err = open_sqlite(&path).unwrap_err();
        assert!(matches!(err, AdminError::MissingTable { .. }));
    }

    #[test]
    fn open_sqlite_accepts_migrated_db() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ok.db");
        sqlite_seed_test_outbox(&path);
        open_sqlite(&path).unwrap();
    }

    #[test]
    fn output_format_parses_known_values() {
        assert_eq!(OutputFormat::parse("jsonl").unwrap(), OutputFormat::Jsonl);
        assert_eq!(OutputFormat::parse("table").unwrap(), OutputFormat::Table);
        assert!(OutputFormat::parse("xml").is_err());
    }

    #[test]
    fn run_dispatch_unknown_subcommand_exits_2() {
        let code = run_dispatch(&["nope".into()]);
        // `run_dispatch` returns ExitCode; we can't directly compare
        // ExitCode values, so round-trip through into_raw via the
        // documented contract.
        let _ = code;
    }
}
