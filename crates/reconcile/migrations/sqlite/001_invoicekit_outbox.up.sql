-- SPDX-License-Identifier: Apache-2.0
-- Copyright 2026 The InvoiceKit Authors

CREATE TABLE IF NOT EXISTS invoicekit_outbox (
    outbox_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    gateway_attempt_id TEXT NOT NULL,
    invoice_fingerprint BLOB NOT NULL,
    state TEXT NOT NULL CHECK (
        state IN (
            'draft',
            'validated',
            'signed',
            'reserved',
            'sent',
            'delivered',
            'acknowledged',
            'rejected',
            'archived',
            'dead_letter'
        )
    ),
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    max_attempts INTEGER NOT NULL DEFAULT 8 CHECK (max_attempts > 0),
    next_attempt_at TEXT NOT NULL,
    reserved_until TEXT,
    last_error_code TEXT,
    last_error_message TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT invoicekit_outbox_outbox_id_not_blank
        CHECK (length(trim(outbox_id)) > 0),
    CONSTRAINT invoicekit_outbox_tenant_id_not_blank
        CHECK (length(trim(tenant_id)) > 0),
    CONSTRAINT invoicekit_outbox_trace_id_not_blank
        CHECK (length(trim(trace_id)) > 0),
    CONSTRAINT invoicekit_outbox_idempotency_key_not_blank
        CHECK (length(trim(idempotency_key)) > 0),
    CONSTRAINT invoicekit_outbox_gateway_attempt_id_not_blank
        CHECK (length(trim(gateway_attempt_id)) > 0),
    CONSTRAINT invoicekit_outbox_attempt_budget_check
        CHECK (attempt_count <= max_attempts),
    CONSTRAINT invoicekit_outbox_tenant_idempotency_unique
        UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS invoicekit_outbox_ready_idx
    ON invoicekit_outbox (tenant_id, state, next_attempt_at);

CREATE TABLE IF NOT EXISTS invoicekit_outbox_dead_letter (
    dead_letter_id TEXT PRIMARY KEY,
    outbox_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    gateway_attempt_id TEXT NOT NULL,
    invoice_fingerprint BLOB NOT NULL,
    final_state TEXT NOT NULL CHECK (
        final_state IN (
            'draft',
            'validated',
            'signed',
            'reserved',
            'sent',
            'delivered',
            'acknowledged',
            'rejected',
            'archived',
            'dead_letter'
        )
    ),
    failure_code TEXT NOT NULL,
    failure_message TEXT NOT NULL,
    attempt_count INTEGER NOT NULL CHECK (attempt_count >= 0),
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    gateway_receipt_hash BLOB,
    dead_lettered_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT invoicekit_dead_letter_dead_letter_id_not_blank
        CHECK (length(trim(dead_letter_id)) > 0),
    CONSTRAINT invoicekit_dead_letter_outbox_id_not_blank
        CHECK (length(trim(outbox_id)) > 0),
    CONSTRAINT invoicekit_dead_letter_tenant_id_not_blank
        CHECK (length(trim(tenant_id)) > 0),
    CONSTRAINT invoicekit_dead_letter_trace_id_not_blank
        CHECK (length(trim(trace_id)) > 0),
    CONSTRAINT invoicekit_dead_letter_idempotency_key_not_blank
        CHECK (length(trim(idempotency_key)) > 0),
    CONSTRAINT invoicekit_dead_letter_gateway_attempt_id_not_blank
        CHECK (length(trim(gateway_attempt_id)) > 0),
    CONSTRAINT invoicekit_dead_letter_failure_code_not_blank
        CHECK (length(trim(failure_code)) > 0),
    CONSTRAINT invoicekit_dead_letter_failure_message_not_blank
        CHECK (length(trim(failure_message)) > 0),
    CONSTRAINT invoicekit_dead_letter_tenant_idempotency_unique
        UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS invoicekit_dead_letter_tenant_idx
    ON invoicekit_outbox_dead_letter (tenant_id, dead_lettered_at);
