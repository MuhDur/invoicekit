-- SPDX-License-Identifier: Apache-2.0
-- Copyright 2026 The InvoiceKit Authors

DROP INDEX IF EXISTS invoicekit_dead_letter_tenant_idx;
DROP TABLE IF EXISTS invoicekit_outbox_dead_letter;
DROP INDEX IF EXISTS invoicekit_outbox_ready_idx;
DROP TABLE IF EXISTS invoicekit_outbox;
