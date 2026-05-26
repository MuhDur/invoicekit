// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-021 property-test harness for InvoiceKit's canonicalizers.
//!
//! Asserts two invariants on every synthetic input:
//!
//! 1. **Idempotence**: `canonicalize(canonicalize(x)) == canonicalize(x)`.
//! 2. **Determinism**: two independent runs on the same input produce the
//!    same byte string.
//!
//! Both invariants run against [`canonicalize`] (RFC 8785 JSON) and
//! [`canonicalize_xml`] (XML C14N-style InvoiceKit form). Each proptest is
//! shrunk by `proptest` to a minimal failing case if any assertion fails.
//!
//! The CI workflow `.github/workflows/proptest.yml` runs this test target
//! with `PROPTEST_CASES=10000` on every pull request so a regression in
//! either canonicalizer is caught before it lands.

#![allow(clippy::cast_possible_truncation)]

use invoicekit_canonical::{canonicalize, canonicalize_value, canonicalize_xml};
use proptest::prelude::*;
use serde_json::{json, Map, Value};

/// Build a strategy that generates a JSON `Value` of bounded depth and width.
///
/// The leaves are small numbers, short ASCII strings, booleans, and null.
/// Objects use ASCII keys so we can also exercise the case-sensitive
/// UTF-16 sort that RFC 8785 requires.
fn arb_json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i32>().prop_map(|n| json!(n)),
        (-1_000_000_i64..=1_000_000_i64).prop_map(|n| json!(n)),
        "[a-zA-Z0-9_-]{0,16}".prop_map(Value::String),
    ];
    leaf.prop_recursive(4, 32, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::vec(("[A-Za-z][A-Za-z0-9_]{0,8}", inner), 0..6).prop_map(|pairs| {
                let mut map = Map::new();
                for (k, v) in pairs {
                    map.insert(k, v);
                }
                Value::Object(map)
            }),
        ]
    })
}

/// Build a strategy that generates a minimal but valid XML document.
///
/// The synthesizer constructs balanced element trees with ASCII tag names
/// and text content, mirroring what the InvoiceKit XML canonicalizer
/// will actually see in production (canonical UBL emits ASCII tags).
fn arb_xml_document() -> impl Strategy<Value = String> {
    arb_xml_element(4, 6)
        .prop_map(|element| format!(r#"<?xml version="1.0" encoding="UTF-8"?>{element}"#))
}

fn arb_xml_element(max_depth: u32, max_width: usize) -> BoxedStrategy<String> {
    let tag = "[A-Za-z][A-Za-z0-9]{0,8}";
    let text = "[A-Za-z0-9 ]{0,16}";
    if max_depth == 0 {
        return (tag, text)
            .prop_map(|(t, txt)| format!("<{t}>{txt}</{t}>"))
            .boxed();
    }
    let leaf = (tag, text)
        .prop_map(|(t, txt)| format!("<{t}>{txt}</{t}>"))
        .boxed();
    let recurse = (
        tag,
        prop::collection::vec(arb_xml_element(max_depth - 1, max_width), 1..=max_width),
    )
        .prop_map(|(t, children)| format!("<{t}>{}</{t}>", children.join("")))
        .boxed();
    prop_oneof![leaf, recurse].boxed()
}

proptest! {
    /// Idempotence: canonicalizing a value, then canonicalizing the textual
    /// form of that value, returns the same string. Per RFC 8785 §3.4 the
    /// canonical form is a fixed point of the canonicalization function.
    #[test]
    fn canonicalize_json_is_idempotent(value in arb_json_value()) {
        let once = canonicalize_value(&value).expect("synthetic JSON canonicalizes");
        let twice = canonicalize(&once).expect("canonical JSON re-canonicalizes");
        prop_assert_eq!(once, twice);
    }

    /// Determinism: two independent calls on the same value return the
    /// same bytes. (Catches platform-specific iteration-order leaks.)
    #[test]
    fn canonicalize_json_is_deterministic(value in arb_json_value()) {
        let a = canonicalize_value(&value).expect("synthetic JSON canonicalizes");
        let b = canonicalize_value(&value).expect("synthetic JSON canonicalizes");
        prop_assert_eq!(a, b);
    }

    /// Round-trip: canonicalize, parse, canonicalize again — the second
    /// canonicalization must equal the first. This is the strongest form
    /// of stability we can assert without comparing against an external
    /// reference implementation.
    #[test]
    fn canonicalize_json_round_trips(value in arb_json_value()) {
        let first = canonicalize_value(&value).expect("synthetic JSON canonicalizes");
        let reparsed: Value = serde_json::from_str(&first).expect("canonical JSON parses");
        let second = canonicalize_value(&reparsed).expect("re-canonicalize");
        prop_assert_eq!(first, second);
    }

    /// Idempotence on XML.
    #[test]
    fn canonicalize_xml_is_idempotent(input in arb_xml_document()) {
        let once = canonicalize_xml(&input).expect("synthetic XML canonicalizes");
        let twice = canonicalize_xml(&once).expect("canonical XML re-canonicalizes");
        prop_assert_eq!(once, twice);
    }

    /// Determinism on XML.
    #[test]
    fn canonicalize_xml_is_deterministic(input in arb_xml_document()) {
        let a = canonicalize_xml(&input).expect("synthetic XML canonicalizes");
        let b = canonicalize_xml(&input).expect("synthetic XML canonicalizes");
        prop_assert_eq!(a, b);
    }
}
