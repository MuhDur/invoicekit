// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Integration tests for the outbox SQL migrations.

use invoicekit_reconcile::{outbox_migration, DatabaseDialect};
use std::env;
use std::error::Error;

const SQLITE_INSERT_OUTBOX: &str = r#"
INSERT INTO invoicekit_outbox (
    outbox_id, tenant_id, trace_id, idempotency_key, gateway_attempt_id,
    invoice_fingerprint, state, payload_json, attempt_count, max_attempts, next_attempt_at
)
VALUES (
    'outbox_001', 'tenant_acme', 'trace_123', 'idem_invoice_123', 'attempt_001',
    X'000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f',
    'reserved', '{"type":"submit"}', 0, 8, '2026-05-26T22:00:00Z'
);
"#;

const SQLITE_INSERT_DUPLICATE_OUTBOX: &str = r#"
INSERT INTO invoicekit_outbox (
    outbox_id, tenant_id, trace_id, idempotency_key, gateway_attempt_id,
    invoice_fingerprint, state, payload_json, attempt_count, max_attempts, next_attempt_at
)
VALUES (
    'outbox_002', 'tenant_acme', 'trace_456', 'idem_invoice_123', 'attempt_002',
    X'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff',
    'reserved', '{"type":"submit"}', 0, 8, '2026-05-26T22:00:00Z'
);
"#;

const SQLITE_INSERT_DEAD_LETTER: &str = r#"
INSERT INTO invoicekit_outbox_dead_letter (
    dead_letter_id, outbox_id, tenant_id, trace_id, idempotency_key, gateway_attempt_id,
    invoice_fingerprint, final_state, failure_code, failure_message, attempt_count, payload_json
)
VALUES (
    'dead_001', 'outbox_001', 'tenant_acme', 'trace_123', 'idem_invoice_123', 'attempt_001',
    X'000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f',
    'dead_letter', 'gateway_maintenance', 'maintenance window exceeded retry budget',
    8, '{"type":"submit"}'
);
"#;

const POSTGRES_INSERT_OUTBOX: &str = r#"
INSERT INTO invoicekit_outbox (
    outbox_id, tenant_id, trace_id, idempotency_key, gateway_attempt_id,
    invoice_fingerprint, state, payload_json, attempt_count, max_attempts, next_attempt_at
)
VALUES (
    'outbox_001', 'tenant_acme', 'trace_123', 'idem_invoice_123', 'attempt_001',
    '\x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f'::bytea,
    'reserved', '{"type":"submit"}'::jsonb, 0, 8, CURRENT_TIMESTAMP
);
"#;

const POSTGRES_INSERT_DUPLICATE_OUTBOX: &str = r#"
INSERT INTO invoicekit_outbox (
    outbox_id, tenant_id, trace_id, idempotency_key, gateway_attempt_id,
    invoice_fingerprint, state, payload_json, attempt_count, max_attempts, next_attempt_at
)
VALUES (
    'outbox_002', 'tenant_acme', 'trace_456', 'idem_invoice_123', 'attempt_002',
    '\xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'::bytea,
    'reserved', '{"type":"submit"}'::jsonb, 0, 8, CURRENT_TIMESTAMP
);
"#;

const POSTGRES_INSERT_DEAD_LETTER: &str = r#"
INSERT INTO invoicekit_outbox_dead_letter (
    dead_letter_id, outbox_id, tenant_id, trace_id, idempotency_key, gateway_attempt_id,
    invoice_fingerprint, final_state, failure_code, failure_message, attempt_count, payload_json
)
VALUES (
    'dead_001', 'outbox_001', 'tenant_acme', 'trace_123', 'idem_invoice_123', 'attempt_001',
    '\x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f'::bytea,
    'dead_letter', 'gateway_maintenance', 'maintenance window exceeded retry budget',
    8, '{"type":"submit"}'::jsonb
);
"#;

const MYSQL_INSERT_OUTBOX: &str = r#"
INSERT INTO invoicekit_outbox (
    outbox_id, tenant_id, trace_id, idempotency_key, gateway_attempt_id,
    invoice_fingerprint, state, payload_json, attempt_count, max_attempts, next_attempt_at
)
VALUES (
    'outbox_001', 'tenant_acme', 'trace_123', 'idem_invoice_123', 'attempt_001',
    UNHEX('000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f'),
    'reserved', '{"type":"submit"}', 0, 8, CURRENT_TIMESTAMP(6)
);
"#;

const MYSQL_INSERT_DUPLICATE_OUTBOX: &str = r#"
INSERT INTO invoicekit_outbox (
    outbox_id, tenant_id, trace_id, idempotency_key, gateway_attempt_id,
    invoice_fingerprint, state, payload_json, attempt_count, max_attempts, next_attempt_at
)
VALUES (
    'outbox_002', 'tenant_acme', 'trace_456', 'idem_invoice_123', 'attempt_002',
    UNHEX('ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'),
    'reserved', '{"type":"submit"}', 0, 8, CURRENT_TIMESTAMP(6)
);
"#;

const MYSQL_INSERT_DEAD_LETTER: &str = r#"
INSERT INTO invoicekit_outbox_dead_letter (
    dead_letter_id, outbox_id, tenant_id, trace_id, idempotency_key, gateway_attempt_id,
    invoice_fingerprint, final_state, failure_code, failure_message, attempt_count, payload_json
)
VALUES (
    'dead_001', 'outbox_001', 'tenant_acme', 'trace_123', 'idem_invoice_123', 'attempt_001',
    UNHEX('000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f'),
    'dead_letter', 'gateway_maintenance', 'maintenance window exceeded retry budget',
    8, '{"type":"submit"}'
);
"#;

#[test]
fn sqlite_outbox_migration_is_executable_idempotent_and_reversible() -> Result<(), Box<dyn Error>> {
    let migration = outbox_migration(DatabaseDialect::Sqlite);
    let connection = rusqlite::Connection::open_in_memory()?;

    connection.execute_batch(migration.up_sql)?;
    connection.execute_batch(migration.up_sql)?;
    connection.execute_batch(SQLITE_INSERT_OUTBOX)?;

    let duplicate = connection.execute_batch(SQLITE_INSERT_DUPLICATE_OUTBOX);
    assert!(duplicate.is_err());
    connection.execute_batch(SQLITE_INSERT_DEAD_LETTER)?;

    connection.execute_batch(migration.down_sql)?;
    connection.execute_batch(migration.down_sql)?;

    let missing = connection.prepare("SELECT outbox_id FROM invoicekit_outbox");
    assert!(missing.is_err());

    Ok(())
}

#[test]
fn postgres_outbox_migration_runs_against_service_when_configured() -> Result<(), Box<dyn Error>> {
    let Some(url) = guarded_database_url("INVOICEKIT_POSTGRES_URL") else {
        return Ok(());
    };
    let migration = outbox_migration(DatabaseDialect::Postgres);
    let mut client = postgres::Client::connect(&url, postgres::NoTls)?;

    client.batch_execute(migration.down_sql)?;
    client.batch_execute(migration.up_sql)?;
    client.batch_execute(migration.up_sql)?;
    client.batch_execute(POSTGRES_INSERT_OUTBOX)?;

    let duplicate = client.batch_execute(POSTGRES_INSERT_DUPLICATE_OUTBOX);
    assert!(duplicate.is_err());
    client.batch_execute(POSTGRES_INSERT_DEAD_LETTER)?;

    client.batch_execute(migration.down_sql)?;
    client.batch_execute(migration.down_sql)?;

    let missing = client.query("SELECT outbox_id FROM invoicekit_outbox", &[]);
    assert!(missing.is_err());

    Ok(())
}

#[test]
fn mysql_outbox_migration_runs_against_service_when_configured() -> Result<(), Box<dyn Error>> {
    let Some(url) = guarded_database_url("INVOICEKIT_MYSQL_URL") else {
        return Ok(());
    };
    let migration = outbox_migration(DatabaseDialect::Mysql);
    let pool = mysql::Pool::new(url.as_str())?;
    let mut connection = pool.get_conn()?;

    mysql_execute_batch(&mut connection, migration.down_sql)?;
    mysql_execute_batch(&mut connection, migration.up_sql)?;
    mysql_execute_batch(&mut connection, migration.up_sql)?;
    mysql::prelude::Queryable::query_drop(&mut connection, MYSQL_INSERT_OUTBOX)?;

    let duplicate =
        mysql::prelude::Queryable::query_drop(&mut connection, MYSQL_INSERT_DUPLICATE_OUTBOX);
    assert!(duplicate.is_err());
    mysql::prelude::Queryable::query_drop(&mut connection, MYSQL_INSERT_DEAD_LETTER)?;

    mysql_execute_batch(&mut connection, migration.down_sql)?;
    mysql_execute_batch(&mut connection, migration.down_sql)?;

    let missing = mysql::prelude::Queryable::query_drop(
        &mut connection,
        "SELECT outbox_id FROM invoicekit_outbox",
    );
    assert!(missing.is_err());

    Ok(())
}

fn guarded_database_url(var_name: &str) -> Option<String> {
    let url = env::var(var_name).ok()?;
    assert!(
        url.contains("invoicekit_test"),
        "{var_name} must point at an invoicekit_test database"
    );
    Some(url)
}

fn mysql_execute_batch(
    connection: &mut mysql::PooledConn,
    sql: &str,
) -> Result<(), Box<dyn Error>> {
    for statement in sql
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        mysql::prelude::Queryable::query_drop(connection, statement)?;
    }
    Ok(())
}
