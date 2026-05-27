// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

/**
 * @invoicekit/core — pure-TypeScript builder API on top of the
 * generated @invoicekit/types interfaces.
 *
 * Goal: a TypeScript consumer can build a CommercialDocument shape
 * with type-safety and basic invariant checks (totals add up,
 * required fields present) without bundling the wasm engine. The
 * builder produces JSON that the @invoicekit/wasm or
 * @invoicekit/managed packages can hand to the engine for
 * validation + canonicalization.
 *
 * This package is the leanest entry point in the SDK trio:
 * pure TS, no runtime dependencies, ~5 KB minified.
 */

export const SDK_CORE_BEAD_ID = "invoices-t-103-typescript-sdk-bhkn";

/** Document type the builder accepts. Mirrors the IR enum. */
export type DocumentType =
  | "invoice"
  | "credit_note"
  | "debit_note"
  | "pro_forma"
  | "self_billed";

/** Minimal party shape. */
export interface PartyInput {
  name: string;
  taxId?: { scheme: string; value: string };
  address: {
    lines: string[];
    city: string;
    postalCode: string;
    country: string;
    subdivision?: string;
  };
  contact?: { email?: string; phone?: string; name?: string };
}

/** One line item the builder accepts. */
export interface LineInput {
  id?: string;
  description: string;
  quantity: string;
  unitCode?: string;
  unitPrice: string;
  taxCategory?: string;
}

/** Builder input shape. Sparser than the full IR; the builder fills
 * derived fields (totals, IDs) before emitting. */
export interface InvoiceInput {
  id: string;
  documentNumber: string;
  documentType: DocumentType;
  issueDate: string;
  dueDate?: string;
  currency: string;
  supplier: PartyInput;
  customer: PartyInput;
  lines: LineInput[];
  tenantId: string;
  traceId: string;
}

/** Output of buildCommercialDocument: a plain JSON object the
 * downstream engine accepts as `CommercialDocument`. The shape is
 * intentionally loose-typed at this layer — the engine validates the
 * full shape and the generated @invoicekit/types interfaces are the
 * compile-time contract. */
export type CommercialDocumentJson = Record<string, unknown>;

/** Errors the builder rejects with. */
export class InvalidInvoiceError extends Error {
  constructor(public readonly field: string, message: string) {
    super(`invalid invoice: ${field}: ${message}`);
    this.name = "InvalidInvoiceError";
  }
}

/**
 * Convert a builder-friendly `InvoiceInput` into a
 * CommercialDocument JSON object suitable for the engine.
 *
 * The builder runs three checks before emitting:
 *
 *  1. Required strings (id, document number, dates, party names,
 *     line descriptions) are non-empty.
 *  2. Currency is a 3-letter ISO 4217 uppercase code.
 *  3. Quantities and unit prices parse as decimal strings.
 *
 * Derived fields filled by the builder:
 *
 *  - Line `id` defaults to `L1`, `L2`, ... when absent.
 *  - `line_extension_amount` per line = `quantity * unit_price`
 *    (decimal-string math via simple BigInt scaling).
 *  - `monetary_total.line_extension_amount` =
 *    sum of per-line line_extension_amount.
 *  - Tax-exclusive total = sum of lines (no tax yet).
 *  - Tax-inclusive total + payable amount mirror tax-exclusive
 *    when no tax_category was set; downstream tax engine fills
 *    real tax math.
 */
export function buildCommercialDocument(
  input: InvoiceInput,
): CommercialDocumentJson {
  requireNonEmpty(input.id, "id");
  requireNonEmpty(input.documentNumber, "documentNumber");
  requireNonEmpty(input.issueDate, "issueDate");
  requireNonEmpty(input.tenantId, "tenantId");
  requireNonEmpty(input.traceId, "traceId");
  requireCurrency(input.currency);
  requireParty(input.supplier, "supplier");
  requireParty(input.customer, "customer");
  if (input.lines.length === 0) {
    throw new InvalidInvoiceError("lines", "must contain at least one line");
  }

  const lines = input.lines.map((line, idx) => {
    requireNonEmpty(line.description, `lines[${idx}].description`);
    requireDecimal(line.quantity, `lines[${idx}].quantity`);
    requireDecimal(line.unitPrice, `lines[${idx}].unitPrice`);
    const lineExtension = multiplyDecimals(line.quantity, line.unitPrice);
    return {
      id: line.id ?? `L${idx + 1}`,
      description: line.description,
      quantity: line.quantity,
      ...(line.unitCode !== undefined ? { unit_code: line.unitCode } : {}),
      unit_price: line.unitPrice,
      line_extension_amount: lineExtension,
      ...(line.taxCategory !== undefined ? { tax_category: line.taxCategory } : {}),
      extensions: [],
    };
  });

  const subtotal = lines.reduce(
    (acc, l) => addDecimals(acc, l.line_extension_amount),
    "0",
  );

  const doc: CommercialDocumentJson = {
    schema_version: "1.0",
    id: input.id,
    document_type: input.documentType,
    issue_date: input.issueDate,
    ...(input.dueDate !== undefined ? { due_date: input.dueDate } : {}),
    document_number: input.documentNumber,
    currency: input.currency,
    supplier: party(input.supplier),
    customer: party(input.customer),
    payment_instructions: [],
    lines,
    tax_summary: [],
    monetary_total: {
      line_extension_amount: subtotal,
      tax_exclusive_amount: subtotal,
      tax_inclusive_amount: subtotal,
      payable_amount: subtotal,
    },
    extensions: [],
    meta: {
      tenant_id: input.tenantId,
      trace_id: input.traceId,
    },
  };
  return doc;
}

function requireNonEmpty(value: string, field: string): void {
  if (typeof value !== "string" || value.trim() === "") {
    throw new InvalidInvoiceError(field, "must be a non-empty string");
  }
}

function requireCurrency(value: string): void {
  if (!/^[A-Z]{3}$/.test(value)) {
    throw new InvalidInvoiceError(
      "currency",
      `must be a 3-letter uppercase ISO 4217 code (got ${JSON.stringify(value)})`,
    );
  }
}

function requireDecimal(value: string, field: string): void {
  if (!/^-?\d+(?:\.\d+)?$/.test(value)) {
    throw new InvalidInvoiceError(
      field,
      `must parse as a decimal string (got ${JSON.stringify(value)})`,
    );
  }
}

function requireParty(party_: PartyInput, name: string): void {
  requireNonEmpty(party_.name, `${name}.name`);
  if (party_.address.lines.length === 0) {
    throw new InvalidInvoiceError(`${name}.address.lines`, "must contain at least one line");
  }
  requireNonEmpty(party_.address.city, `${name}.address.city`);
  requireNonEmpty(party_.address.postalCode, `${name}.address.postalCode`);
  if (!/^[A-Z]{2}$/.test(party_.address.country)) {
    throw new InvalidInvoiceError(
      `${name}.address.country`,
      `must be ISO 3166-1 alpha-2 (got ${JSON.stringify(party_.address.country)})`,
    );
  }
}

function party(p: PartyInput): Record<string, unknown> {
  const out: Record<string, unknown> = {
    name: p.name,
    tax_ids: p.taxId ? [{ scheme: p.taxId.scheme, value: p.taxId.value }] : [],
    address: {
      lines: p.address.lines,
      city: p.address.city,
      ...(p.address.subdivision !== undefined ? { subdivision: p.address.subdivision } : {}),
      postal_code: p.address.postalCode,
      country: p.address.country,
    },
  };
  if (p.contact) {
    out["contact"] = {
      ...(p.contact.name !== undefined ? { name: p.contact.name } : {}),
      ...(p.contact.email !== undefined ? { email: p.contact.email } : {}),
      ...(p.contact.phone !== undefined ? { phone: p.contact.phone } : {}),
    };
  }
  return out;
}

// Simple decimal-string arithmetic. We don't pull in big.js / decimal.js
// because the engine is the source of truth for full decimal math —
// the builder just needs enough precision for the line subtotals to
// agree with the engine on integer-cent inputs.
function multiplyDecimals(a: string, b: string): string {
  const [aWhole, aFrac = ""] = a.split(".");
  const [bWhole, bFrac = ""] = b.split(".");
  const aScaled = BigInt(`${aWhole}${aFrac}`);
  const bScaled = BigInt(`${bWhole}${bFrac}`);
  const scale = aFrac.length + bFrac.length;
  const product = aScaled * bScaled;
  return formatDecimal(product, scale);
}

function addDecimals(a: string, b: string): string {
  const [aWhole, aFrac = ""] = a.split(".");
  const [bWhole, bFrac = ""] = b.split(".");
  const maxFrac = Math.max(aFrac.length, bFrac.length);
  const aScaled = BigInt(`${aWhole}${aFrac.padEnd(maxFrac, "0")}`);
  const bScaled = BigInt(`${bWhole}${bFrac.padEnd(maxFrac, "0")}`);
  return formatDecimal(aScaled + bScaled, maxFrac);
}

function formatDecimal(value: bigint, scale: number): string {
  if (scale === 0) {
    return value.toString();
  }
  const negative = value < 0n;
  const abs = negative ? -value : value;
  const str = abs.toString().padStart(scale + 1, "0");
  const whole = str.slice(0, -scale);
  const frac = str.slice(-scale);
  const out = `${whole}.${frac}`;
  return negative ? `-${out}` : out;
}
