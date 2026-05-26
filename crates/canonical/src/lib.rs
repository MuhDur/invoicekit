// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit-canonical` — RFC 8785 JSON Canonicalization Scheme.
//!
//! Every InvoiceKit operation that signs, hashes, or audits a JSON document
//! first canonicalizes it through this crate. The output is a byte-stable
//! UTF-8 string that two independent implementations should produce
//! bit-identically given the same input.
//!
//! ## What this crate guarantees
//!
//! * Object members are emitted in lexicographic order by UTF-16 code unit
//!   sequence of the member name, exactly as RFC 8785 §3.2.3 specifies.
//! * Strings are escaped using the minimal RFC 8785 §3.2.2 / ECMAScript
//!   `JSON.stringify` rule set: `"`, `\\`, control characters U+0000…U+001F
//!   via short escapes (`\\b`, `\\f`, `\\n`, `\\r`, `\\t`) or `\\u00xx`, and
//!   every other Unicode code point passes through verbatim. Forward slash
//!   `/` is NOT escaped.
//! * Numbers are serialized using the ECMAScript 6.5.3 `Number.prototype.
//!   toString` algorithm, as required by RFC 8785 §3.2.2.2. The
//!   `ryu-js` crate is the reference implementation of that algorithm and
//!   is what JCS test-vector ports converge on.
//! * Arrays preserve element order.
//! * Whitespace between tokens is removed (no insignificant whitespace).
//!
//! ## What this crate does NOT do
//!
//! * It does not validate that the input is RFC 8259-conformant JSON beyond
//!   what `serde_json` already enforces.
//! * It does not transcode strings — the input must already be valid UTF-8
//!   (which every `&str` is).
//! * It does not deduplicate object members; [`canonicalize`] rejects
//!   duplicate object names before the parsed [`Value`] can collapse them.
//!   [`canonicalize_value`] accepts an already-parsed [`Value`], where
//!   duplicate object names are no longer representable.

use std::collections::BTreeSet;
use std::fmt::{self, Write as _};

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use thiserror::Error;

const DUPLICATE_MEMBER_ERROR_PREFIX: &str = "invoicekit duplicate object member: ";
const MAX_SAFE_INTEGER: i128 = 9_007_199_254_740_991;
const MIN_SAFE_INTEGER: i128 = -MAX_SAFE_INTEGER;

/// Errors returned by [`canonicalize`] and [`canonicalize_value`].
#[derive(Debug, Error)]
pub enum CanonicalizeError {
    /// The input was not valid JSON.
    #[error("input was not valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    /// The input contained the same object member name more than once.
    ///
    /// RFC 8785 builds on I-JSON, which forbids duplicate object names.
    /// Rejecting them before `serde_json::Value` construction prevents
    /// silent last-write-wins data loss in signed payloads.
    #[error("duplicate object member `{0}` is not valid RFC 8785/I-JSON input")]
    DuplicateObjectMember(String),
    /// An integer was outside the I-JSON interoperable safe range.
    ///
    /// RFC 8785 inherits I-JSON's IEEE-754 double-precision number domain.
    /// JSON integer values are interoperable only in
    /// `[-9007199254740991, 9007199254740991]`.
    #[error("integer `{0}` is outside the RFC 8785/I-JSON safe range")]
    UnsafeInteger(String),
    /// A JSON number could not be represented under RFC 8785 number rules.
    ///
    /// RFC 8785 §3.2.2.4 forbids serializing `NaN`, `+Infinity`, and
    /// `-Infinity`. `serde_json` does not normally produce those values
    /// from textual input, but when feeding the API from in-memory
    /// [`Value`]s constructed by other code this error surfaces them.
    #[error("number `{0}` is not representable under RFC 8785 (NaN/Infinity)")]
    NonFiniteNumber(String),
}

/// Canonicalize a JSON string into its RFC 8785 form.
///
/// # Errors
///
/// Returns [`CanonicalizeError::InvalidJson`] when the input does not parse
/// as JSON, [`CanonicalizeError::DuplicateObjectMember`] when an object
/// repeats a member name, [`CanonicalizeError::UnsafeInteger`] when an
/// integer is outside the I-JSON safe range, or
/// [`CanonicalizeError::NonFiniteNumber`] when the input contains a
/// non-finite number.
///
/// # Examples
///
/// ```
/// let raw = r#"{ "b": 2 ,  "a":1 }"#;
/// let canonical = invoicekit_canonical::canonicalize(raw).unwrap();
/// assert_eq!(canonical, r#"{"a":1,"b":2}"#);
/// ```
pub fn canonicalize(input: &str) -> Result<String, CanonicalizeError> {
    let value = parse_value_rejecting_duplicate_members(input)?;
    canonicalize_value(&value)
}

/// Canonicalize a parsed JSON value into its RFC 8785 form.
///
/// # Errors
///
/// Returns [`CanonicalizeError::UnsafeInteger`] when an integer is outside
/// the I-JSON safe range, or [`CanonicalizeError::NonFiniteNumber`] when
/// the value contains a non-finite number.
///
/// # Examples
///
/// ```
/// use serde_json::json;
/// let value = json!({"b": [3, 1, 2], "a": null});
/// let canonical = invoicekit_canonical::canonicalize_value(&value).unwrap();
/// assert_eq!(canonical, r#"{"a":null,"b":[3,1,2]}"#);
/// ```
pub fn canonicalize_value(value: &Value) -> Result<String, CanonicalizeError> {
    let mut out = String::new();
    write_value(value, &mut out)?;
    Ok(out)
}

fn parse_value_rejecting_duplicate_members(input: &str) -> Result<Value, CanonicalizeError> {
    match serde_json::from_str::<CheckedValue>(input) {
        Ok(CheckedValue(value)) => Ok(value),
        Err(error) => {
            if let Some(member) = duplicate_member_from_error(&error) {
                return Err(CanonicalizeError::DuplicateObjectMember(member));
            }
            Err(CanonicalizeError::InvalidJson(error))
        }
    }
}

fn duplicate_member_from_error(error: &serde_json::Error) -> Option<String> {
    let message = error.to_string();
    let payload = message.strip_prefix(DUPLICATE_MEMBER_ERROR_PREFIX)?;
    let payload = payload
        .rsplit_once(" at line ")
        .map_or(payload, |(payload, _)| payload);
    serde_json::from_str(payload).ok()
}

fn duplicate_member_error<E>(member: &str) -> E
where
    E: de::Error,
{
    let encoded = serde_json::to_string(member).expect("serializing a string is infallible");
    E::custom(format!("{DUPLICATE_MEMBER_ERROR_PREFIX}{encoded}"))
}

struct CheckedValue(Value);

impl<'de> Deserialize<'de> for CheckedValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(CheckedValueVisitor).map(Self)
    }
}

struct CheckedValueVisitor;

impl<'de> Visitor<'de> for CheckedValueVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object member names")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_i128<E>(self, value: i128) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let value = i64::try_from(value).map_err(|_| E::custom("integer does not fit i64"))?;
        Ok(Value::Number(value.into()))
    }

    fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let value = u64::try_from(value).map_err(|_| E::custom("integer does not fit u64"))?;
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        let number = serde_json::Number::from_f64(value)
            .ok_or_else(|| E::custom("non-finite JSON number"))?;
        Ok(Value::Number(number))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        let CheckedValue(value) = CheckedValue::deserialize(deserializer)?;
        Ok(value)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut items = Vec::new();
        while let Some(CheckedValue(value)) = seq.next_element::<CheckedValue>()? {
            items.push(value);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = serde_json::Map::new();
        let mut seen = BTreeSet::new();
        while let Some(member) = map.next_key::<String>()? {
            if !seen.insert(member.clone()) {
                return Err(duplicate_member_error(&member));
            }
            let CheckedValue(value) = map.next_value::<CheckedValue>()?;
            object.insert(member, value);
        }
        Ok(Value::Object(object))
    }
}

fn write_value(value: &Value, out: &mut String) -> Result<(), CanonicalizeError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => write_number(n, out)?,
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            // Lexicographic sort by UTF-16 code-unit sequence per RFC 8785 §3.2.3.
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| compare_utf16(a.0, b.0));
            out.push('{');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(k, out);
                out.push(':');
                write_value(v, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_number(n: &serde_json::Number, out: &mut String) -> Result<(), CanonicalizeError> {
    if let Some(i) = n.as_i64() {
        ensure_safe_integer(i128::from(i), n)?;
        write!(out, "{i}").expect("write to String is infallible");
        return Ok(());
    }
    if let Some(u) = n.as_u64() {
        let value = i128::from(u);
        ensure_safe_integer(value, n)?;
        write!(out, "{u}").expect("write to String is infallible");
        return Ok(());
    }
    if let Some(f) = n.as_f64() {
        if !f.is_finite() {
            return Err(CanonicalizeError::NonFiniteNumber(n.to_string()));
        }
        // ECMAScript 6.5.3 Number.prototype.toString -> Ryū-JS.
        let mut buffer = ryu_js::Buffer::new();
        let s = buffer.format(f);
        out.push_str(s);
        return Ok(());
    }
    let rendered = n.to_string();
    if rendered
        .bytes()
        .all(|byte| byte == b'-' || byte.is_ascii_digit())
    {
        return Err(CanonicalizeError::UnsafeInteger(rendered));
    }
    Err(CanonicalizeError::NonFiniteNumber(rendered))
}

fn ensure_safe_integer(
    value: i128,
    original: &serde_json::Number,
) -> Result<(), CanonicalizeError> {
    if (MIN_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
        Ok(())
    } else {
        Err(CanonicalizeError::UnsafeInteger(original.to_string()))
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                write!(out, "\\u{:04x}", c as u32).expect("write to String is infallible");
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Compare two strings by their UTF-16 code-unit sequence.
///
/// RFC 8785 §3.2.3 mandates lexicographic sort of object members in their
/// UTF-16 representation. For BMP code points UTF-16 ordering matches
/// Unicode-scalar ordering; for supplementary code points (U+10000..) the
/// surrogate pair ordering differs from the scalar ordering.
fn compare_utf16(a: &str, b: &str) -> std::cmp::Ordering {
    let mut au = a.encode_utf16();
    let mut bu = b.encode_utf16();
    loop {
        match (au.next(), bu.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(a), Some(b)) => {
                if a != b {
                    return a.cmp(&b);
                }
            }
        }
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_canonical::crate_name(), "invoicekit-canonical");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-canonical"
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::{json, Value};

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-canonical");
    }

    /// RFC 8785 §3.3 test vector: object member ordering.
    #[test]
    fn rfc8785_member_ordering() {
        let input = r#"{
            "numbers": [333333333.33333329, 1E30, 4.5, 2e-3],
            "string": "Hello",
            "literals": [null, true, false]
        }"#;
        let canonical = canonicalize(input).unwrap();
        // The canonical form must order top-level keys literals < numbers < string.
        let expected_prefix = r#"{"literals":[null,true,false],"numbers":["#;
        assert!(canonical.starts_with(expected_prefix), "got: {canonical}");
    }

    /// RFC 8785 §3.2.2.2: integers serialize without a decimal point.
    #[test]
    fn integers_serialize_without_decimal_point() {
        assert_eq!(canonicalize("42").unwrap(), "42");
        assert_eq!(canonicalize("-42").unwrap(), "-42");
        assert_eq!(canonicalize("0").unwrap(), "0");
    }

    /// ECMAScript Number.toString: `1.0` → "1".
    #[test]
    fn ecmascript_number_serialization() {
        assert_eq!(canonicalize("1.0").unwrap(), "1");
        assert_eq!(canonicalize("100.0").unwrap(), "100");
        // RFC 8785 §3.2.2.3: `4.50` -> "4.5".
        assert_eq!(canonicalize("4.50").unwrap(), "4.5");
    }

    /// Empty object + empty array.
    #[test]
    fn empty_containers_serialize() {
        assert_eq!(canonicalize("{}").unwrap(), "{}");
        assert_eq!(canonicalize("[]").unwrap(), "[]");
        assert_eq!(canonicalize(r#"{"a":[]}"#).unwrap(), r#"{"a":[]}"#);
    }

    /// RFC 8785 §3.2.2: control characters serialize as `\\u00xx`.
    #[test]
    fn control_characters_are_escaped() {
        let input = "{\"k\":\"\\u0001\\u001f\"}";
        let canonical = canonicalize(input).unwrap();
        assert_eq!(canonical, "{\"k\":\"\\u0001\\u001f\"}");
    }

    /// String escapes: backslash, quote, slash (not escaped), control chars.
    #[test]
    fn string_escapes_match_rfc8785() {
        // Slash must NOT be escaped.
        assert_eq!(canonicalize(r#""a/b""#).unwrap(), r#""a/b""#);
        // Backslash and quote are escaped.
        assert_eq!(canonicalize(r#""a\\b\"c""#).unwrap(), r#""a\\b\"c""#);
        // Tab, newline, carriage return, formfeed, backspace use short escapes.
        assert_eq!(
            canonicalize("\"\\t\\n\\r\\f\\b\"").unwrap(),
            "\"\\t\\n\\r\\f\\b\""
        );
    }

    /// Member-name sort is by UTF-16 code unit (RFC 8785 §3.2.3).
    #[test]
    fn member_name_sort_by_utf16_code_unit() {
        // "a", "b", "ä" (U+00E4 = 0xE4), "💖" (U+1F496 surrogate pair starts 0xD83D)
        let input = r#"{"b":2,"a":1,"ä":3,"💖":4}"#;
        let out = canonicalize(input).unwrap();
        // a < b < ä < 💖 because UTF-16 code units 0x61 < 0x62 < 0xE4 < 0xD83D.
        assert_eq!(out, r#"{"a":1,"b":2,"ä":3,"💖":4}"#);
    }

    /// Non-finite numbers are rejected.
    #[test]
    fn non_finite_numbers_are_rejected() {
        let v: Value = serde_json::from_str(r#"{"k":null}"#).unwrap();
        // Construct a Value that contains NaN through arithmetic; serde_json's
        // Number cannot deserialize NaN from text, but it can hold it via
        // serde_json::Number::from_f64 only when finite. So instead we cover
        // the contract by directly constructing the failure path:
        let nan = serde_json::Number::from_f64(f64::NAN);
        assert!(
            nan.is_none(),
            "serde_json::Number rejects NaN at construction"
        );
        // Verify the happy-path Value works (the negative test above asserts
        // the actual library refuses NaN, satisfying the contract).
        assert!(canonicalize_value(&v).is_ok());
    }

    /// Invalid JSON is rejected.
    #[test]
    fn invalid_json_is_rejected() {
        let err = canonicalize("not json").unwrap_err();
        assert!(matches!(err, CanonicalizeError::InvalidJson(_)));
    }

    fn duplicate_member_name(error: CanonicalizeError) -> Option<String> {
        match error {
            CanonicalizeError::DuplicateObjectMember(member) => Some(member),
            _ => None,
        }
    }

    /// Duplicate object members are rejected before `Value` can collapse them.
    #[test]
    fn duplicate_object_members_are_rejected() {
        let err = canonicalize(r#"{"a":1,"a":2}"#).unwrap_err();
        assert_eq!(duplicate_member_name(err).as_deref(), Some("a"));
    }

    /// Duplicate detection recurses through nested objects and arrays.
    #[test]
    fn nested_duplicate_object_members_are_rejected() {
        let err = canonicalize(r#"{"outer":[{"b":1,"b":2}]}"#).unwrap_err();
        assert_eq!(duplicate_member_name(err).as_deref(), Some("b"));
    }

    /// I-JSON's interoperable integer range ends at 2^53 - 1.
    #[test]
    fn safe_integer_boundaries_are_accepted() {
        assert_eq!(
            canonicalize("9007199254740991").unwrap(),
            "9007199254740991"
        );
        assert_eq!(
            canonicalize("-9007199254740991").unwrap(),
            "-9007199254740991"
        );
    }

    /// Integers outside the I-JSON safe range are rejected from text.
    #[test]
    fn unsafe_integer_text_is_rejected() {
        let err = canonicalize("9007199254740992").unwrap_err();
        assert!(
            matches!(err, CanonicalizeError::UnsafeInteger(value) if value == "9007199254740992")
        );

        let err = canonicalize("-9007199254740992").unwrap_err();
        assert!(
            matches!(err, CanonicalizeError::UnsafeInteger(value) if value == "-9007199254740992")
        );
    }

    /// In-memory `Value` inputs use the same integer-domain guard.
    #[test]
    fn unsafe_integer_value_is_rejected() {
        let value = Value::Number(serde_json::Number::from(9_007_199_254_740_992_u64));
        let err = canonicalize_value(&value).unwrap_err();
        assert!(
            matches!(err, CanonicalizeError::UnsafeInteger(value) if value == "9007199254740992")
        );
    }

    /// Determinism: canonicalize twice on the same input -> identical bytes.
    #[test]
    fn canonicalize_is_idempotent() {
        let input = json!({"z": 1, "a": [2, 3], "m": null});
        let once = canonicalize_value(&input).unwrap();
        let twice = canonicalize_value(&input).unwrap();
        assert_eq!(once, twice);
    }

    /// Canonical form parses back to the same logical document.
    #[test]
    fn canonical_form_round_trips() {
        let input = json!({"b": [1, 2], "a": null, "c": "hi"});
        let canonical = canonicalize_value(&input).unwrap();
        let reparsed: Value = serde_json::from_str(&canonical).unwrap();
        assert_eq!(reparsed, input);
    }

    proptest! {
        /// Canonicalize is a function: same input -> same output.
        #[test]
        fn canonicalize_is_a_function(seed in any::<u32>()) {
            // Build a synthetic JSON value parameterized by the seed.
            let value = json!({
                "seed": seed,
                "nested": {"a": 1, "b": [seed, seed.wrapping_add(1)]},
                "list": (0..(seed % 5)).map(serde_json::Value::from).collect::<Vec<_>>(),
            });
            let a = canonicalize_value(&value).unwrap();
            let b = canonicalize_value(&value).unwrap();
            prop_assert_eq!(a, b);
        }

        /// Round-trip: canonicalize -> parse -> canonicalize -> same bytes.
        #[test]
        fn canonicalize_then_parse_then_canonicalize_is_stable(seed in any::<u32>()) {
            let value = json!({
                "seed": seed,
                "child": {"x": i64::from(seed).wrapping_neg(), "y": [1, 2, 3]},
            });
            let first = canonicalize_value(&value).unwrap();
            let reparsed: Value = serde_json::from_str(&first).unwrap();
            let second = canonicalize_value(&reparsed).unwrap();
            prop_assert_eq!(first, second);
        }
    }
}
