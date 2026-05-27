// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import * as basicInvoice from "./basic-invoice.ts";
import * as creditNote from "./credit-note.ts";
import * as facturXSummary from "./factur-x-summary.ts";
import * as paymentReminder from "./payment-reminder.ts";
import * as taxBreakdown from "./tax-breakdown.ts";

export const examples = {
  "basic-invoice": basicInvoice,
  "credit-note": creditNote,
  "factur-x-summary": facturXSummary,
  "payment-reminder": paymentReminder,
  "tax-breakdown": taxBreakdown,
} as const;
