// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Golden-artifact tests for InvoiceKit canonicalization.
//!
//! Canonicalization output sits on the signing and hashing path: every evidence
//! bundle signature and content hash is computed over these exact bytes. The
//! contract is byte-stability — semantically equivalent inputs must map to one
//! canonical form, forever. These tests freeze the known-good canonical output
//! for a curated corpus of inputs that exercise each transformation rule, so any
//! future change that moves a single output byte fails loudly and must be
//! reviewed (a real canonicalization change rotates every downstream signature
//! and is never incidental).
//!
//! The output is fully deterministic (RFC 8785 JSON; the no-comments XML C14N
//! 1.1 invoice profile), so these are exact-match goldens — no scrubbing.
//!
//! Workflow:
//!
//! ```text
//! cargo test -p invoicekit-canonical --test golden_canonical
//! UPDATE_GOLDENS=1 cargo test -p invoicekit-canonical --test golden_canonical
//! git diff crates/canonical/tests/golden/   # review EVERY change before commit
//! ```
//!
//! `.actual` files are written next to a mismatched golden for easy diffing and
//! are git-ignored.

use std::fs;
use std::path::{Path, PathBuf};

use invoicekit_canonical::{canonicalize, canonicalize_xml};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

fn assert_golden(name: &str, actual: &str) {
    let path = golden_dir().join(format!("{name}.golden"));

    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        fs::create_dir_all(path.parent().unwrap()).expect("create golden dir");
        fs::write(&path, actual).expect("write golden");
        eprintln!("[GOLDEN] updated {}", path.display());
        return;
    }

    let expected = fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "golden file missing: {}\n\
             run `UPDATE_GOLDENS=1 cargo test -p invoicekit-canonical --test golden_canonical`,\n\
             then review and commit crates/canonical/tests/golden/",
            path.display()
        )
    });

    if actual != expected {
        let actual_path = path.with_extension("actual");
        let _ = fs::write(&actual_path, actual);
        panic!(
            "GOLDEN MISMATCH: {name}\n\
             --- expected (frozen)\n{expected}\n\
             +++ actual (this run)\n{actual}\n\
             A canonicalization change rotates every downstream signature. If this is\n\
             intentional, run UPDATE_GOLDENS=1, review `git diff tests/golden/`, and commit.\n\
             Wrote actual output to {}",
            actual_path.display()
        );
    }
}

// --- JSON canonicalization (RFC 8785) ------------------------------------

#[test]
fn json_sorts_object_keys() {
    let out = canonicalize(r#"{ "b": 2, "a": 1, "c": 3 }"#).unwrap();
    assert_golden("json/sorts_keys", &out);
}

#[test]
fn json_sorts_nested_keys_and_preserves_array_order() {
    let out = canonicalize(r#"{"z":{"y":2,"x":1},"a":[3,1,2]}"#).unwrap();
    assert_golden("json/nested_keys_array_order", &out);
}

#[test]
fn json_strips_insignificant_whitespace() {
    let out = canonicalize("{\n  \"a\" : 1 ,\n  \"b\" : [ 1 , 2 ]\n}").unwrap();
    assert_golden("json/strips_whitespace", &out);
}

#[test]
fn json_normalizes_numbers() {
    // Integers, fractionals, negative zero, exponents — RFC 8785 number form.
    let out =
        canonicalize(r#"{"int":42,"frac":1.5,"negzero":-0,"exp":1e3,"small":0.001}"#).unwrap();
    assert_golden("json/numbers", &out);
}

#[test]
fn json_preserves_unicode_and_escapes() {
    let out = canonicalize(r#"{"text":"a\tb\n€ ☃","quote":"he said \"hi\""}"#).unwrap();
    assert_golden("json/unicode_escapes", &out);
}

#[test]
fn json_handles_scalars_and_null() {
    let out =
        canonicalize(r#"{"t":true,"f":false,"n":null,"empty_obj":{},"empty_arr":[]}"#).unwrap();
    assert_golden("json/scalars_null", &out);
}

#[test]
fn json_realistic_invoice_fragment() {
    let raw = r#"{
        "currency": "EUR",
        "document_number": "INV-2026-0001",
        "lines": [
            {"id": "2", "amount": "200.00", "qty": 2},
            {"id": "1", "amount": "100.00", "qty": 1}
        ],
        "issue_date": "2026-05-26",
        "total": {"payable": "300.00", "tax": "57.00"}
    }"#;
    let out = canonicalize(raw).unwrap();
    assert_golden("json/invoice_fragment", &out);
}

// --- XML canonicalization (no-comments C14N 1.1 invoice profile) ----------

#[test]
fn xml_normalizes_ubl_prefixes_and_sorts_attrs() {
    let raw = r#"<Invoice xmlns:x="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2"><x:AccountingSupplierParty z="2" a="1"/></Invoice>"#;
    let out = canonicalize_xml(raw).unwrap();
    assert_golden("xml/ubl_prefix_and_attr_sort", &out);
}

#[test]
fn xml_strips_declaration_and_expands_empty_elements() {
    let raw = r#"<?xml version="1.0" encoding="UTF-8"?><Invoice><Empty/></Invoice>"#;
    let out = canonicalize_xml(raw).unwrap();
    assert_golden("xml/decl_and_empty_elements", &out);
}

#[test]
fn xml_removes_inter_element_whitespace() {
    let raw = "<Invoice>\n  <A>text</A>\n  <B>more</B>\n</Invoice>";
    let out = canonicalize_xml(raw).unwrap();
    assert_golden("xml/inter_element_whitespace", &out);
}

#[test]
fn xml_realistic_ubl_invoice() {
    let raw = r#"<ubl:Invoice xmlns:ubl="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2" xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2">
  <cbc:ID>INV-1</cbc:ID>
  <cbc:IssueDate>2026-05-27</cbc:IssueDate>
  <cbc:DocumentCurrencyCode>EUR</cbc:DocumentCurrencyCode>
  <cac:InvoiceLine><cbc:ID>1</cbc:ID><cbc:LineExtensionAmount currencyID="EUR">100.00</cbc:LineExtensionAmount></cac:InvoiceLine>
</ubl:Invoice>"#;
    let out = canonicalize_xml(raw).unwrap();
    assert_golden("xml/ubl_invoice", &out);
}

#[test]
fn xml_canonicalization_is_idempotent() {
    // Not a golden, but a load-bearing invariant for a canonical form: running
    // it twice must yield the same bytes. Guards against a golden that is not
    // actually a fixed point.
    let raw = r#"<ubl:Invoice xmlns:ubl="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2"><cac:AccountingSupplierParty z="2" a="1"/></ubl:Invoice>"#;
    let once = canonicalize_xml(raw).unwrap();
    let twice = canonicalize_xml(&once).unwrap();
    assert_eq!(once, twice, "XML canonicalization must be idempotent");
}

#[test]
fn json_canonicalization_is_idempotent() {
    let raw = r#"{"b":2,"a":[3,1,2],"c":{"y":1,"x":2}}"#;
    let once = canonicalize(raw).unwrap();
    let twice = canonicalize(&once).unwrap();
    assert_eq!(once, twice, "JSON canonicalization must be idempotent");
}
