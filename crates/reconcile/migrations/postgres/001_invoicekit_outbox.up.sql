-- SPDX-License-Identifier: Apache-2.0
-- Copyright 2026 The InvoiceKit Authors

CREATE TABLE IF NOT EXISTS invoicekit_outbox (
    outbox_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    gateway_attempt_id TEXT NOT NULL,
    invoice_fingerprint BYTEA NOT NULL,
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
    payload_json JSONB NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    max_attempts INTEGER NOT NULL DEFAULT 8 CHECK (max_attempts > 0),
    next_attempt_at TIMESTAMPTZ NOT NULL,
    reserved_until TIMESTAMPTZ,
    last_error_code TEXT,
    last_error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT invoicekit_outbox_outbox_id_not_blank
        CHECK (char_length(btrim(outbox_id)) > 0),
    CONSTRAINT invoicekit_outbox_tenant_id_not_blank
        CHECK (char_length(btrim(tenant_id)) > 0),
    CONSTRAINT invoicekit_outbox_trace_id_not_blank
        CHECK (char_length(btrim(trace_id)) > 0),
    CONSTRAINT invoicekit_outbox_idempotency_key_not_blank
        CHECK (char_length(btrim(idempotency_key)) > 0),
    CONSTRAINT invoicekit_outbox_gateway_attempt_id_not_blank
        CHECK (char_length(btrim(gateway_attempt_id)) > 0),
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
    invoice_fingerprint BYTEA NOT NULL,
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
    payload_json JSONB NOT NULL,
    gateway_receipt_hash BYTEA,
    dead_lettered_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT invoicekit_dead_letter_dead_letter_id_not_blank
        CHECK (char_length(btrim(dead_letter_id)) > 0),
    CONSTRAINT invoicekit_dead_letter_outbox_id_not_blank
        CHECK (char_length(btrim(outbox_id)) > 0),
    CONSTRAINT invoicekit_dead_letter_tenant_id_not_blank
        CHECK (char_length(btrim(tenant_id)) > 0),
    CONSTRAINT invoicekit_dead_letter_trace_id_not_blank
        CHECK (char_length(btrim(trace_id)) > 0),
    CONSTRAINT invoicekit_dead_letter_idempotency_key_not_blank
        CHECK (char_length(btrim(idempotency_key)) > 0),
    CONSTRAINT invoicekit_dead_letter_gateway_attempt_id_not_blank
        CHECK (char_length(btrim(gateway_attempt_id)) > 0),
    CONSTRAINT invoicekit_dead_letter_failure_code_not_blank
        CHECK (char_length(btrim(failure_code)) > 0),
    CONSTRAINT invoicekit_dead_letter_failure_message_not_blank
        CHECK (char_length(btrim(failure_message)) > 0),
    CONSTRAINT invoicekit_dead_letter_tenant_idempotency_unique
        UNIQUE (tenant_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS invoicekit_dead_letter_tenant_idx
    ON invoicekit_outbox_dead_letter (tenant_id, dead_lettered_at);
