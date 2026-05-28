// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Golden byte-stability tests for the deterministic PDF renderer.
//!
//! These tests pin the exact SHA-256 of the PDF bytes produced by each public
//! render path. They exist as the isomorphism proof for performance work on
//! `InMemoryWorld`: hoisting the document-independent Typst standard library and
//! font catalogue out of the per-call constructor must not change a single
//! output byte. If a render optimization is genuinely behavior-preserving these
//! hashes do not move; if it is not, this test fails loudly.
//!
//! The renderer is already contractually deterministic (fixed document date,
//! pinned fonts, system fonts disabled, stable identifier), so a moving hash
//! here is always a real regression, never flakiness.
//!
//! If a *legitimate* upstream change (e.g. a Typst version bump) changes the
//! bytes, update the constants below in the same commit that bumps the
//! dependency, and call it out in the commit message.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use invoicekit_ir::CommercialDocument;
use invoicekit_render_pdf::{render_commercial_document_invoice, render_hello_world_invoice};
use serde_json::{json, Value};

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn sample_document() -> CommercialDocument {
    let value: Value = json!({
        "schema_version": "1.0",
        "id": "doc_golden_render_0001",
        "document_type": "invoice",
        "issue_date": "2026-05-26",
        "due_date": "2026-06-25",
        "document_number": "INV-GOLDEN-0001",
        "currency": "EUR",
        "supplier": party_json("supplier-1", "Golden Supplier", "DE"),
        "customer": party_json("customer-1", "Golden Customer", "FR"),
        "payment_terms": { "description": "30 days net", "due_date": "2026-06-25" },
        "payment_instructions": [{
            "kind": "iban_bic",
            "account": "DE02100100100006820101",
            "reference": "INV-GOLDEN-0001"
        }],
        "lines": [{
            "id": "1",
            "description": "Golden render line item",
            "quantity": "1",
            "unit_code": "EA",
            "unit_price": "100.00",
            "line_extension_amount": "100.00",
            "tax_category": "S",
            "extensions": []
        }],
        "tax_summary": [{
            "category_code": "S",
            "taxable_amount": "100.00",
            "tax_amount": "19.00",
            "tax_rate": "19.00"
        }],
        "monetary_total": {
            "line_extension_amount": "100.00",
            "tax_exclusive_amount": "100.00",
            "tax_inclusive_amount": "119.00",
            "payable_amount": "119.00"
        },
        "extensions": [],
        "meta": { "tenant_id": "tenant_golden", "trace_id": "trace_golden" }
    });
    CommercialDocument::try_from_value(value).expect("golden fixture must be valid")
}

fn party_json(id: &str, name: &str, country: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "tax_ids": [{ "scheme": "vat", "value": format!("{country}123456789") }],
        "address": {
            "lines": ["Golden Street 1"],
            "city": "Golden City",
            "postal_code": "00000",
            "country": country
        }
    })
}

/// Golden SHA-256 of the T-050 hello-world invoice PDF bytes.
///
/// Captured on the pre-optimization renderer. Any render-path optimization that
/// claims to be behavior-preserving must keep this hash unchanged.
const HELLO_WORLD_PDF_SHA256: &str =
    "8133ea4acca835622b999b2ee24dbf4cf927e592becaa82efd58c406b7becea4";

/// Golden SHA-256 of the rendered commercial-document PDF bytes for
/// [`sample_document`].
const COMMERCIAL_DOCUMENT_PDF_SHA256: &str =
    "dc33a69201c679f5a919014454d64afc49f422b5effc0167742c64182a1c0fd1";

#[test]
fn hello_world_pdf_matches_golden_hash() {
    let pdf = render_hello_world_invoice().expect("hello-world render must succeed");
    assert_eq!(
        sha256_hex(&pdf),
        HELLO_WORLD_PDF_SHA256,
        "hello-world PDF bytes changed; a render optimization broke byte-stability \
         (or a dependency bump legitimately moved the bytes — update the constant in \
         that same commit)"
    );
}

#[test]
fn commercial_document_pdf_matches_golden_hash() {
    let document = sample_document();
    let pdf =
        render_commercial_document_invoice(&document).expect("commercial render must succeed");
    assert_eq!(
        sha256_hex(&pdf),
        COMMERCIAL_DOCUMENT_PDF_SHA256,
        "commercial-document PDF bytes changed; a render optimization broke byte-stability \
         (or a dependency bump legitimately moved the bytes — update the constant in \
         that same commit)"
    );
}
