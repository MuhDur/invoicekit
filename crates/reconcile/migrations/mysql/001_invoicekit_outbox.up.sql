-- SPDX-License-Identifier: Apache-2.0
-- Copyright 2026 The InvoiceKit Authors

CREATE TABLE IF NOT EXISTS invoicekit_outbox (
    outbox_id VARCHAR(128) PRIMARY KEY,
    tenant_id VARCHAR(128) NOT NULL,
    trace_id VARCHAR(128) NOT NULL,
    idempotency_key VARCHAR(191) NOT NULL,
    gateway_attempt_id VARCHAR(128) NOT NULL,
    invoice_fingerprint VARBINARY(32) NOT NULL,
    state VARCHAR(32) NOT NULL,
    payload_json JSON NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 8,
    next_attempt_at TIMESTAMP(6) NOT NULL,
    reserved_until TIMESTAMP(6) NULL,
    last_error_code VARCHAR(128) NULL,
    last_error_message TEXT NULL,
    created_at TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    updated_at TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6) ON UPDATE CURRENT_TIMESTAMP(6),
    CONSTRAINT invoicekit_outbox_state_check CHECK (
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
    CONSTRAINT invoicekit_outbox_attempt_count_check CHECK (attempt_count >= 0),
    CONSTRAINT invoicekit_outbox_max_attempts_check CHECK (max_attempts > 0),
    CONSTRAINT invoicekit_outbox_outbox_id_not_blank CHECK (TRIM(outbox_id) <> ''),
    CONSTRAINT invoicekit_outbox_tenant_id_not_blank CHECK (TRIM(tenant_id) <> ''),
    CONSTRAINT invoicekit_outbox_trace_id_not_blank CHECK (TRIM(trace_id) <> ''),
    CONSTRAINT invoicekit_outbox_idempotency_key_not_blank CHECK (TRIM(idempotency_key) <> ''),
    CONSTRAINT invoicekit_outbox_gateway_attempt_id_not_blank CHECK (TRIM(gateway_attempt_id) <> ''),
    CONSTRAINT invoicekit_outbox_attempt_budget_check CHECK (attempt_count <= max_attempts),
    CONSTRAINT invoicekit_outbox_tenant_idempotency_unique UNIQUE (tenant_id, idempotency_key),
    INDEX invoicekit_outbox_ready_idx (tenant_id, state, next_attempt_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;

CREATE TABLE IF NOT EXISTS invoicekit_outbox_dead_letter (
    dead_letter_id VARCHAR(128) PRIMARY KEY,
    outbox_id VARCHAR(128) NOT NULL,
    tenant_id VARCHAR(128) NOT NULL,
    trace_id VARCHAR(128) NOT NULL,
    idempotency_key VARCHAR(191) NOT NULL,
    gateway_attempt_id VARCHAR(128) NOT NULL,
    invoice_fingerprint VARBINARY(32) NOT NULL,
    final_state VARCHAR(32) NOT NULL,
    failure_code VARCHAR(128) NOT NULL,
    failure_message TEXT NOT NULL,
    attempt_count INTEGER NOT NULL,
    payload_json JSON NOT NULL,
    gateway_receipt_hash VARBINARY(32) NULL,
    dead_lettered_at TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    CONSTRAINT invoicekit_dead_letter_state_check CHECK (
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
    CONSTRAINT invoicekit_dead_letter_attempt_count_check CHECK (attempt_count >= 0),
    CONSTRAINT invoicekit_dead_letter_dead_letter_id_not_blank CHECK (TRIM(dead_letter_id) <> ''),
    CONSTRAINT invoicekit_dead_letter_outbox_id_not_blank CHECK (TRIM(outbox_id) <> ''),
    CONSTRAINT invoicekit_dead_letter_tenant_id_not_blank CHECK (TRIM(tenant_id) <> ''),
    CONSTRAINT invoicekit_dead_letter_trace_id_not_blank CHECK (TRIM(trace_id) <> ''),
    CONSTRAINT invoicekit_dead_letter_idempotency_key_not_blank CHECK (TRIM(idempotency_key) <> ''),
    CONSTRAINT invoicekit_dead_letter_gateway_attempt_id_not_blank CHECK (TRIM(gateway_attempt_id) <> ''),
    CONSTRAINT invoicekit_dead_letter_failure_code_not_blank CHECK (TRIM(failure_code) <> ''),
    CONSTRAINT invoicekit_dead_letter_failure_message_not_blank CHECK (TRIM(failure_message) <> ''),
    CONSTRAINT invoicekit_dead_letter_tenant_idempotency_unique UNIQUE (tenant_id, idempotency_key),
    INDEX invoicekit_dead_letter_tenant_idx (tenant_id, dead_lettered_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;
