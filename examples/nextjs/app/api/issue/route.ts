// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// T-1400 reference Next.js API route: builds a German XRechnung
// CommercialDocument JSON from one of three named fixtures using
// the @invoicekit/core builder. The fixture choice is in the
// `?fixture=` query param; default is "basic".

import { NextRequest, NextResponse } from "next/server";

import { buildCommercialDocument, InvalidInvoiceError } from "@invoicekit/core";

import { fixtures } from "../../../fixtures";

export const runtime = "nodejs";

export async function POST(req: NextRequest) {
  const fixture = req.nextUrl.searchParams.get("fixture") ?? "basic";
  const input = fixtures[fixture as keyof typeof fixtures];
  if (!input) {
    return NextResponse.json(
      { error: { code: "UNKNOWN_FIXTURE", available: Object.keys(fixtures) } },
      { status: 400 },
    );
  }

  try {
    const doc = buildCommercialDocument(input);
    return NextResponse.json(doc, { status: 200 });
  } catch (err) {
    if (err instanceof InvalidInvoiceError) {
      return NextResponse.json(
        { error: { code: "INVALID_INVOICE", field: err.field, message: err.message } },
        { status: 422 },
      );
    }
    throw err;
  }
}
