// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-137 `invoicekit-admin replay <dead_letter_id>`: re-enqueue a
//! dead-letter row into the live outbox.
//!
//! The dead-letter row stays in `invoicekit_outbox_dead_letter` as an
//! audit trail. The replay inserts a fresh `invoicekit_outbox` row
//! reusing the original tenant/trace/idempotency triple but with a
//! brand-new `outbox_id` and `gateway_attempt_id`, attempt count zero,
//! and `next_attempt_at = now`. If a live row with the same
//! `(tenant_id, idempotency_key)` already exists (the unique
//! constraint), the replay is refused.
//!
//! Operators can run with `--dry-run` to see what would happen without
//! mutating either table.

use std::path::Path;
use std::process::ExitCode;

use rusqlite::{params, OptionalExtension};

use super::{open_sqlite, AdminError};

/// Bead identifier carried for diagnostic correlation.
pub const REPLAY_BEAD_ID: &str = "invoices-t-137-replay-admin-tooling-j6sy";

/// Parsed CLI shape for `invoicekit-admin replay`.
#[derive(Debug, Clone)]
pub struct ReplayArgs {
    /// Sqlite database path.
    pub db_path: String,
    /// `dead_letter_id` of the row to replay.
    pub dead_letter_id: String,
    /// Optional fresh outbox-id; auto-generated when absent.
    pub new_outbox_id: Option<String>,
    /// Optional fresh gateway-attempt-id; auto-generated when absent.
    pub new_attempt_id: Option<String>,
    /// `true` = no mutation, just print what would happen.
    pub dry_run: bool,
}

/// Outcome of a replay attempt.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ReplayResult {
    /// Source dead-letter row id.
    pub dead_letter_id: String,
    /// New live-outbox row id.
    pub new_outbox_id: String,
    /// Original tenant.
    pub tenant_id: String,
    /// Original idempotency key.
    pub idempotency_key: String,
    /// `true` when `--dry-run` was set.
    pub dry_run: bool,
}

/// Re-enqueue the dead-letter row identified by `args.dead_letter_id`.
///
/// # Errors
///
/// Returns [`AdminError::NotFound`] when no dead-letter row matches,
/// [`AdminError::Conflict`] when a live row already has the same
/// `(tenant_id, idempotency_key)`, [`AdminError::Sqlite`] on query
/// failure, [`AdminError::Io`] on filesystem failure.
pub fn run(args: &ReplayArgs) -> Result<ReplayResult, AdminError> {
    let mut conn = open_sqlite(Path::new(&args.db_path))?;

    let tx = conn.transaction().map_err(map_sqlite("begin tx"))?;
    let dlq = tx
        .query_row(
            "SELECT tenant_id, trace_id, idempotency_key, gateway_attempt_id, \
                    invoice_fingerprint, payload_json \
             FROM invoicekit_outbox_dead_letter \
             WHERE dead_letter_id = ?1",
            params![&args.dead_letter_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        )
        .optional()
        .map_err(map_sqlite("lookup dead-letter row"))?;
    let (tenant_id, trace_id, idempotency_key, prior_attempt_id, fingerprint, payload_json) =
        dlq.ok_or_else(|| AdminError::NotFound {
            dead_letter_id: args.dead_letter_id.clone(),
        })?;

    let conflict: Option<String> = tx
        .query_row(
            "SELECT outbox_id FROM invoicekit_outbox \
             WHERE tenant_id = ?1 AND idempotency_key = ?2",
            params![&tenant_id, &idempotency_key],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(map_sqlite("conflict probe"))?;
    if let Some(existing) = conflict {
        return Err(AdminError::Conflict {
            dead_letter_id: args.dead_letter_id.clone(),
            existing_outbox_id: existing,
        });
    }

    let new_outbox_id = args
        .new_outbox_id
        .clone()
        .unwrap_or_else(|| derive_id("replay-outbox-", &args.dead_letter_id));
    let new_attempt_id = args
        .new_attempt_id
        .clone()
        .unwrap_or_else(|| derive_id("replay-attempt-", &prior_attempt_id));

    if args.dry_run {
        // Roll the transaction back without mutating; the caller still
        // gets a fully-populated ReplayResult so it can preview.
        tx.rollback().map_err(map_sqlite("rollback dry-run"))?;
    } else {
        tx.execute(
            "INSERT INTO invoicekit_outbox ( \
                outbox_id, tenant_id, trace_id, idempotency_key, gateway_attempt_id, \
                invoice_fingerprint, state, payload_json, attempt_count, max_attempts, \
                next_attempt_at, created_at, updated_at \
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'draft', ?7, 0, 8, \
                      datetime('now'), datetime('now'), datetime('now'))",
            params![
                &new_outbox_id,
                &tenant_id,
                &trace_id,
                &idempotency_key,
                &new_attempt_id,
                &fingerprint,
                &payload_json,
            ],
        )
        .map_err(map_sqlite("insert replayed outbox row"))?;
        tx.commit().map_err(map_sqlite("commit replay"))?;
    }

    Ok(ReplayResult {
        dead_letter_id: args.dead_letter_id.clone(),
        new_outbox_id,
        tenant_id,
        idempotency_key,
        dry_run: args.dry_run,
    })
}

/// CLI entry point for `invoicekit-admin replay`.
///
/// # Panics
///
/// Panics only via the internal `expect` on `serde_json::to_string`,
/// which would indicate that [`ReplayResult`] failed to serialize.
pub fn cli(argv: &[String]) -> ExitCode {
    let parsed = match parse_argv(argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            eprintln!();
            eprint!("{}", usage());
            return ExitCode::from(2);
        }
    };
    match run(&parsed) {
        Ok(res) => {
            println!(
                "{}",
                serde_json::to_string(&res).expect("ReplayResult must serialize")
            );
            ExitCode::SUCCESS
        }
        Err(e @ AdminError::NotFound { .. }) => {
            eprintln!("invoicekit-admin replay: {e}");
            ExitCode::from(4)
        }
        Err(e @ AdminError::Conflict { .. }) => {
            eprintln!("invoicekit-admin replay: {e}");
            ExitCode::from(5)
        }
        Err(e) => {
            eprintln!("invoicekit-admin replay: {e}");
            ExitCode::from(3)
        }
    }
}

fn usage() -> String {
    "usage: invoicekit-admin replay --db=PATH --id=DEAD_LETTER_ID [--new-outbox-id=ID] \\\n                               [--new-attempt-id=ID] [--dry-run]\n"
        .to_string()
}

fn parse_argv(argv: &[String]) -> Result<ReplayArgs, AdminError> {
    let mut db: Option<String> = None;
    let mut id: Option<String> = None;
    let mut new_outbox_id: Option<String> = None;
    let mut new_attempt_id: Option<String> = None;
    let mut dry_run = false;
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        if let Some(v) = a.strip_prefix("--db=") {
            db = Some(v.to_owned());
        } else if a == "--db" {
            i += 1;
            db = Some(
                argv.get(i)
                    .cloned()
                    .ok_or_else(|| AdminError::Cli("--db requires a value".into()))?,
            );
        } else if let Some(v) = a.strip_prefix("--id=") {
            id = Some(v.to_owned());
        } else if a == "--id" {
            i += 1;
            id = Some(
                argv.get(i)
                    .cloned()
                    .ok_or_else(|| AdminError::Cli("--id requires a value".into()))?,
            );
        } else if let Some(v) = a.strip_prefix("--new-outbox-id=") {
            new_outbox_id = Some(v.to_owned());
        } else if let Some(v) = a.strip_prefix("--new-attempt-id=") {
            new_attempt_id = Some(v.to_owned());
        } else if a == "--dry-run" {
            dry_run = true;
        } else if a == "--help" || a == "-h" {
            print!("{}", usage());
            std::process::exit(0);
        } else {
            return Err(AdminError::Cli(format!("unknown flag: {a}")));
        }
        i += 1;
    }
    Ok(ReplayArgs {
        db_path: db.ok_or_else(|| AdminError::Cli("--db is required".into()))?,
        dead_letter_id: id.ok_or_else(|| AdminError::Cli("--id is required".into()))?,
        new_outbox_id,
        new_attempt_id,
        dry_run,
    })
}

fn derive_id(prefix: &str, seed: &str) -> String {
    // Deterministic, audit-friendly id: `<prefix><blake3-12>` so the
    // same DLQ row replayed twice (with no live conflict in between)
    // produces the same outbox id and the audit log can join across.
    let hash = blake3::hash(seed.as_bytes());
    let mut s = String::with_capacity(prefix.len() + 12);
    s.push_str(prefix);
    for b in &hash.as_bytes()[..6] {
        let _ = std::fmt::Write::write_fmt(&mut s, format_args!("{b:02x}"));
    }
    s
}

fn map_sqlite(stage: &'static str) -> impl Fn(rusqlite::Error) -> AdminError {
    move |e| AdminError::Sqlite {
        stage,
        detail: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::{sqlite_seed_test_outbox, TEST_DLQ_ID};
    use tempfile::TempDir;

    #[test]
    fn replay_reinserts_dlq_row_into_live_outbox() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("test.db");
        sqlite_seed_test_outbox(&db);
        let args = ReplayArgs {
            db_path: db.to_string_lossy().into(),
            dead_letter_id: TEST_DLQ_ID.into(),
            new_outbox_id: None,
            new_attempt_id: None,
            dry_run: false,
        };
        let res = run(&args).unwrap();
        assert_eq!(res.dead_letter_id, TEST_DLQ_ID);
        assert!(res.new_outbox_id.starts_with("replay-outbox-"));
        let conn = open_sqlite(&db).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM invoicekit_outbox WHERE outbox_id = ?1",
                params![&res.new_outbox_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "new outbox row should be present");
    }

    #[test]
    fn replay_dry_run_does_not_mutate() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("test.db");
        sqlite_seed_test_outbox(&db);
        let before: i64 = open_sqlite(&db)
            .unwrap()
            .query_row("SELECT count(*) FROM invoicekit_outbox", params![], |r| {
                r.get(0)
            })
            .unwrap();
        let args = ReplayArgs {
            db_path: db.to_string_lossy().into(),
            dead_letter_id: TEST_DLQ_ID.into(),
            new_outbox_id: None,
            new_attempt_id: None,
            dry_run: true,
        };
        let res = run(&args).unwrap();
        assert!(res.dry_run);
        let after: i64 = open_sqlite(&db)
            .unwrap()
            .query_row("SELECT count(*) FROM invoicekit_outbox", params![], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(before, after, "dry-run must not insert");
    }

    #[test]
    fn replay_rejects_missing_dlq_id() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("test.db");
        sqlite_seed_test_outbox(&db);
        let args = ReplayArgs {
            db_path: db.to_string_lossy().into(),
            dead_letter_id: "not-a-real-id".into(),
            new_outbox_id: None,
            new_attempt_id: None,
            dry_run: false,
        };
        let err = run(&args).unwrap_err();
        assert!(matches!(err, AdminError::NotFound { .. }));
    }

    #[test]
    fn replay_refuses_when_live_row_already_exists() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("test.db");
        sqlite_seed_test_outbox(&db);
        // First replay succeeds.
        let args = ReplayArgs {
            db_path: db.to_string_lossy().into(),
            dead_letter_id: TEST_DLQ_ID.into(),
            new_outbox_id: None,
            new_attempt_id: None,
            dry_run: false,
        };
        run(&args).unwrap();
        // Second replay must hit the (tenant, idempotency) unique
        // constraint via our pre-check.
        let err = run(&args).unwrap_err();
        assert!(matches!(err, AdminError::Conflict { .. }));
    }

    #[test]
    fn parse_argv_requires_db_and_id() {
        let err = parse_argv(&[]).unwrap_err();
        assert!(matches!(err, AdminError::Cli(_)));
        let err = parse_argv(&["--db=/tmp/x.db".into()]).unwrap_err();
        assert!(matches!(err, AdminError::Cli(_)));
    }
}
