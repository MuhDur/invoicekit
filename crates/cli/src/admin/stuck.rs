// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-137 `invoicekit-admin stuck`: list outbox rows that need human attention.
//!
//! A row is "stuck" when it falls into any of the buckets below. We
//! report each bucket separately so an operator can triage the
//! actually-broken transmissions from the ones that just haven't fired
//! their next retry yet.
//!
//! - **dead-letter**: `invoicekit_outbox_dead_letter` rows. Always
//!   stuck, always need a decision (replay or write off).
//! - **retry-overdue**: live `invoicekit_outbox` rows whose
//!   `next_attempt_at` is in the past by more than `--overdue-mins`
//!   (default 15). The worker should already be picking these up; if
//!   they're still here, the worker is wedged or off.
//! - **stale-reserved**: rows that have been held by `reserved_until`
//!   for longer than `--reserved-mins` (default 5). The leasing worker
//!   probably crashed.
//!
//! Output is one JSON object per row (`jsonl`) on `stdout`, or a
//! human-friendly table when `--format=table`.

#![allow(clippy::option_if_let_else)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use rusqlite::Connection;

use super::{open_sqlite, AdminError, OutputFormat};

/// Default `--overdue-mins` for the retry-overdue bucket.
pub const DEFAULT_OVERDUE_MINUTES: i64 = 15;

/// Default `--reserved-mins` for the stale-reserved bucket.
pub const DEFAULT_RESERVED_MINUTES: i64 = 5;

/// Parsed CLI shape for `invoicekit-admin stuck`.
#[derive(Debug, Clone)]
pub struct StuckArgs {
    /// Sqlite database path.
    pub db_path: String,
    /// Optional tenant filter; `None` lists every tenant.
    pub tenant: Option<String>,
    /// Override of the retry-overdue threshold.
    pub overdue_mins: i64,
    /// Override of the stale-reserved threshold.
    pub reserved_mins: i64,
    /// Output format (`jsonl` default; `table` for humans).
    pub format: OutputFormat,
}

/// One stuck row as reported by `invoicekit-admin stuck`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StuckRow {
    /// Origin queue: `dead_letter` | `outbox`.
    pub source: &'static str,
    /// Stuck bucket: `dead_letter` | `retry_overdue` | `stale_reserved`.
    pub bucket: &'static str,
    /// Primary key on the source table.
    pub id: String,
    /// Owning tenant.
    pub tenant_id: String,
    /// Trace identifier for cross-system correlation.
    pub trace_id: String,
    /// Idempotency key carried by the transmission.
    pub idempotency_key: String,
    /// Current state on the live outbox; `dead_letter` for DLQ rows.
    pub state: String,
    /// Last failure-code recorded (live) or `failure_code` (DLQ).
    pub failure_code: Option<String>,
    /// Last failure message recorded (live) or `failure_message` (DLQ).
    pub failure_message: Option<String>,
    /// Number of attempts the worker has made.
    pub attempt_count: i64,
    /// ISO-8601 of the most relevant timestamp (`dead_lettered_at` for
    /// DLQ; `updated_at` for live).
    pub since: String,
}

/// Run the `stuck` subcommand against `db_path`.
///
/// # Errors
///
/// Returns [`AdminError::Sqlite`] when a query fails, [`AdminError::Io`]
/// for filesystem errors, [`AdminError::MissingTable`] when the
/// expected outbox tables aren't there.
pub fn run(args: &StuckArgs) -> Result<Vec<StuckRow>, AdminError> {
    let conn = open_sqlite(Path::new(&args.db_path))?;
    let mut out = Vec::new();

    out.extend(query_dead_letter(&conn, args.tenant.as_deref())?);
    out.extend(query_retry_overdue(
        &conn,
        args.tenant.as_deref(),
        args.overdue_mins,
    )?);
    out.extend(query_stale_reserved(
        &conn,
        args.tenant.as_deref(),
        args.reserved_mins,
    )?);
    Ok(out)
}

/// CLI entry point — parses argv, runs, prints, returns an [`ExitCode`].
///
/// # Panics
///
/// Panics only via the internal `expect` on `serde_json::to_string`,
/// which would indicate that [`StuckRow`] failed to serialize.
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
        Ok(rows) => {
            match parsed.format {
                OutputFormat::Jsonl => {
                    for row in &rows {
                        println!(
                            "{}",
                            serde_json::to_string(row).expect("StuckRow must serialize")
                        );
                    }
                }
                OutputFormat::Table => print_table(&rows),
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("invoicekit-admin stuck: {e}");
            ExitCode::from(3)
        }
    }
}

fn usage() -> String {
    "usage: invoicekit-admin stuck --db=PATH [--tenant=TENANT] \\\n                              [--overdue-mins=N] [--reserved-mins=N] [--format=jsonl|table]\n"
        .to_string()
}

fn parse_argv(argv: &[String]) -> Result<StuckArgs, AdminError> {
    let mut db: Option<String> = None;
    let mut tenant: Option<String> = None;
    let mut overdue = DEFAULT_OVERDUE_MINUTES;
    let mut reserved = DEFAULT_RESERVED_MINUTES;
    let mut format = OutputFormat::Jsonl;
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
        } else if let Some(v) = a.strip_prefix("--tenant=") {
            tenant = Some(v.to_owned());
        } else if let Some(v) = a.strip_prefix("--overdue-mins=") {
            overdue = v
                .parse()
                .map_err(|_| AdminError::Cli(format!("--overdue-mins not an integer: {v:?}")))?;
        } else if let Some(v) = a.strip_prefix("--reserved-mins=") {
            reserved = v
                .parse()
                .map_err(|_| AdminError::Cli(format!("--reserved-mins not an integer: {v:?}")))?;
        } else if let Some(v) = a.strip_prefix("--format=") {
            format = OutputFormat::parse(v)?;
        } else if a == "--help" || a == "-h" {
            print!("{}", usage());
            std::process::exit(0);
        } else {
            return Err(AdminError::Cli(format!("unknown flag: {a}")));
        }
        i += 1;
    }
    Ok(StuckArgs {
        db_path: db.ok_or_else(|| AdminError::Cli("--db is required".into()))?,
        tenant,
        overdue_mins: overdue,
        reserved_mins: reserved,
        format,
    })
}

fn query_dead_letter(conn: &Connection, tenant: Option<&str>) -> Result<Vec<StuckRow>, AdminError> {
    let (sql, args): (&str, Vec<String>) = match tenant {
        Some(t) => (
            "SELECT dead_letter_id, tenant_id, trace_id, idempotency_key, final_state, \
                    failure_code, failure_message, attempt_count, dead_lettered_at \
             FROM invoicekit_outbox_dead_letter \
             WHERE tenant_id = ?1 \
             ORDER BY dead_lettered_at",
            vec![t.to_owned()],
        ),
        None => (
            "SELECT dead_letter_id, tenant_id, trace_id, idempotency_key, final_state, \
                    failure_code, failure_message, attempt_count, dead_lettered_at \
             FROM invoicekit_outbox_dead_letter \
             ORDER BY dead_lettered_at",
            vec![],
        ),
    };
    let mut stmt = conn.prepare(sql).map_err(map_sqlite("query dead-letter"))?;
    let arg_refs: Vec<&dyn rusqlite::ToSql> =
        args.iter().map(|a| a as &dyn rusqlite::ToSql).collect();
    let mut rows = stmt
        .query(arg_refs.as_slice())
        .map_err(map_sqlite("execute dead-letter query"))?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(map_sqlite("read dead-letter row"))? {
        out.push(StuckRow {
            source: "dead_letter",
            bucket: "dead_letter",
            id: r.get::<_, String>(0).map_err(map_sqlite("dlq id"))?,
            tenant_id: r.get::<_, String>(1).map_err(map_sqlite("dlq tenant"))?,
            trace_id: r.get::<_, String>(2).map_err(map_sqlite("dlq trace"))?,
            idempotency_key: r.get::<_, String>(3).map_err(map_sqlite("dlq idemp"))?,
            state: r.get::<_, String>(4).map_err(map_sqlite("dlq state"))?,
            failure_code: r
                .get::<_, Option<String>>(5)
                .map_err(map_sqlite("dlq fc"))?,
            failure_message: r
                .get::<_, Option<String>>(6)
                .map_err(map_sqlite("dlq fm"))?,
            attempt_count: r.get::<_, i64>(7).map_err(map_sqlite("dlq attempt"))?,
            since: r.get::<_, String>(8).map_err(map_sqlite("dlq when"))?,
        });
    }
    Ok(out)
}

fn query_retry_overdue(
    conn: &Connection,
    tenant: Option<&str>,
    overdue_mins: i64,
) -> Result<Vec<StuckRow>, AdminError> {
    let cutoff = format!("-{overdue_mins} minutes");
    let (sql, args): (&str, Vec<String>) = match tenant {
        Some(t) => (
            "SELECT outbox_id, tenant_id, trace_id, idempotency_key, state, \
                    last_error_code, last_error_message, attempt_count, updated_at \
             FROM invoicekit_outbox \
             WHERE tenant_id = ?1 \
               AND state NOT IN ('delivered','acknowledged','archived','rejected','dead_letter') \
               AND next_attempt_at < datetime('now', ?2) \
             ORDER BY updated_at",
            vec![t.to_owned(), cutoff],
        ),
        None => (
            "SELECT outbox_id, tenant_id, trace_id, idempotency_key, state, \
                    last_error_code, last_error_message, attempt_count, updated_at \
             FROM invoicekit_outbox \
             WHERE state NOT IN ('delivered','acknowledged','archived','rejected','dead_letter') \
               AND next_attempt_at < datetime('now', ?1) \
             ORDER BY updated_at",
            vec![cutoff],
        ),
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(map_sqlite("query retry-overdue"))?;
    let arg_refs: Vec<&dyn rusqlite::ToSql> =
        args.iter().map(|a| a as &dyn rusqlite::ToSql).collect();
    let mut rows = stmt
        .query(arg_refs.as_slice())
        .map_err(map_sqlite("execute retry-overdue query"))?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(map_sqlite("read retry-overdue row"))? {
        out.push(StuckRow {
            source: "outbox",
            bucket: "retry_overdue",
            id: r.get::<_, String>(0).map_err(map_sqlite("ob id"))?,
            tenant_id: r.get::<_, String>(1).map_err(map_sqlite("ob tenant"))?,
            trace_id: r.get::<_, String>(2).map_err(map_sqlite("ob trace"))?,
            idempotency_key: r.get::<_, String>(3).map_err(map_sqlite("ob idemp"))?,
            state: r.get::<_, String>(4).map_err(map_sqlite("ob state"))?,
            failure_code: r.get::<_, Option<String>>(5).map_err(map_sqlite("ob fc"))?,
            failure_message: r.get::<_, Option<String>>(6).map_err(map_sqlite("ob fm"))?,
            attempt_count: r.get::<_, i64>(7).map_err(map_sqlite("ob attempt"))?,
            since: r.get::<_, String>(8).map_err(map_sqlite("ob when"))?,
        });
    }
    Ok(out)
}

fn query_stale_reserved(
    conn: &Connection,
    tenant: Option<&str>,
    reserved_mins: i64,
) -> Result<Vec<StuckRow>, AdminError> {
    let cutoff = format!("-{reserved_mins} minutes");
    let (sql, args): (&str, Vec<String>) = match tenant {
        Some(t) => (
            "SELECT outbox_id, tenant_id, trace_id, idempotency_key, state, \
                    last_error_code, last_error_message, attempt_count, updated_at \
             FROM invoicekit_outbox \
             WHERE tenant_id = ?1 \
               AND reserved_until IS NOT NULL \
               AND reserved_until < datetime('now', ?2) \
             ORDER BY updated_at",
            vec![t.to_owned(), cutoff],
        ),
        None => (
            "SELECT outbox_id, tenant_id, trace_id, idempotency_key, state, \
                    last_error_code, last_error_message, attempt_count, updated_at \
             FROM invoicekit_outbox \
             WHERE reserved_until IS NOT NULL \
               AND reserved_until < datetime('now', ?1) \
             ORDER BY updated_at",
            vec![cutoff],
        ),
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(map_sqlite("query stale-reserved"))?;
    let arg_refs: Vec<&dyn rusqlite::ToSql> =
        args.iter().map(|a| a as &dyn rusqlite::ToSql).collect();
    let mut rows = stmt
        .query(arg_refs.as_slice())
        .map_err(map_sqlite("execute stale-reserved query"))?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(map_sqlite("read stale-reserved row"))? {
        out.push(StuckRow {
            source: "outbox",
            bucket: "stale_reserved",
            id: r.get::<_, String>(0).map_err(map_sqlite("sr id"))?,
            tenant_id: r.get::<_, String>(1).map_err(map_sqlite("sr tenant"))?,
            trace_id: r.get::<_, String>(2).map_err(map_sqlite("sr trace"))?,
            idempotency_key: r.get::<_, String>(3).map_err(map_sqlite("sr idemp"))?,
            state: r.get::<_, String>(4).map_err(map_sqlite("sr state"))?,
            failure_code: r.get::<_, Option<String>>(5).map_err(map_sqlite("sr fc"))?,
            failure_message: r.get::<_, Option<String>>(6).map_err(map_sqlite("sr fm"))?,
            attempt_count: r.get::<_, i64>(7).map_err(map_sqlite("sr attempt"))?,
            since: r.get::<_, String>(8).map_err(map_sqlite("sr when"))?,
        });
    }
    Ok(out)
}

fn print_table(rows: &[StuckRow]) {
    if rows.is_empty() {
        println!("no stuck transmissions");
        return;
    }
    let mut by_bucket: BTreeMap<&'static str, Vec<&StuckRow>> = BTreeMap::new();
    for r in rows {
        by_bucket.entry(r.bucket).or_default().push(r);
    }
    for (bucket, rs) in by_bucket {
        let mut out = String::new();
        let _ = writeln!(out, "## {bucket} ({n})", n = rs.len());
        for r in rs {
            let _ = writeln!(
                out,
                "  {id}\ttenant={tenant}\tstate={state}\tattempts={n}\tsince={s}",
                id = r.id,
                tenant = r.tenant_id,
                state = r.state,
                n = r.attempt_count,
                s = r.since,
            );
        }
        print!("{out}");
    }
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
    use crate::admin::sqlite_seed_test_outbox;
    use tempfile::TempDir;

    #[test]
    fn stuck_returns_dead_letter_rows() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("test.db");
        sqlite_seed_test_outbox(&db);
        let args = StuckArgs {
            db_path: db.to_string_lossy().into(),
            tenant: None,
            overdue_mins: 15,
            reserved_mins: 5,
            format: OutputFormat::Jsonl,
        };
        let rows = run(&args).unwrap();
        let has_dlq = rows.iter().any(|r| r.bucket == "dead_letter");
        assert!(has_dlq, "should report dead-letter rows; got {rows:?}");
    }

    #[test]
    fn stuck_filters_by_tenant() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("test.db");
        sqlite_seed_test_outbox(&db);
        let args = StuckArgs {
            db_path: db.to_string_lossy().into(),
            tenant: Some("tenant-other".into()),
            overdue_mins: 15,
            reserved_mins: 5,
            format: OutputFormat::Jsonl,
        };
        let rows = run(&args).unwrap();
        assert!(
            rows.is_empty(),
            "no rows expected for tenant-other; got {rows:?}"
        );
    }

    #[test]
    fn stuck_returns_retry_overdue_rows() {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("test.db");
        sqlite_seed_test_outbox(&db);
        let args = StuckArgs {
            db_path: db.to_string_lossy().into(),
            tenant: None,
            overdue_mins: 5,
            reserved_mins: 5,
            format: OutputFormat::Jsonl,
        };
        let rows = run(&args).unwrap();
        let has_overdue = rows.iter().any(|r| r.bucket == "retry_overdue");
        assert!(
            has_overdue,
            "expected at least one retry-overdue row; got {rows:?}"
        );
    }

    #[test]
    fn parse_argv_requires_db() {
        let err = parse_argv(&[]).unwrap_err();
        assert!(matches!(err, AdminError::Cli(_)));
    }

    #[test]
    fn parse_argv_accepts_split_form() {
        let args = parse_argv(&[
            "--db".into(),
            "/tmp/x.db".into(),
            "--tenant=t-1".into(),
            "--overdue-mins=20".into(),
            "--format=table".into(),
        ])
        .unwrap();
        assert_eq!(args.db_path, "/tmp/x.db");
        assert_eq!(args.tenant.as_deref(), Some("t-1"));
        assert_eq!(args.overdue_mins, 20);
        assert!(matches!(args.format, OutputFormat::Table));
    }

    #[test]
    fn run_fails_cleanly_on_missing_db() {
        let args = StuckArgs {
            db_path: "/nonexistent/path/db.sqlite".into(),
            tenant: None,
            overdue_mins: 15,
            reserved_mins: 5,
            format: OutputFormat::Jsonl,
        };
        let err = run(&args).unwrap_err();
        assert!(matches!(
            err,
            AdminError::Io { .. } | AdminError::Sqlite { .. }
        ));
    }
}
