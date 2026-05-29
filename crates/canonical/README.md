<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-canonical

Byte-stable JSON and XML canonicalization for the InvoiceKit signing and hashing path.

## What it does

Before InvoiceKit signs, hashes, or audits a document, it runs the document through this crate. The output is a deterministic UTF-8 string: two independent implementations should produce the same bytes from the same input. That byte-stability is what makes a signature verifiable and a hash reproducible across machines and across time.

The crate covers two formats:

- **JSON** follows RFC 8785 (JSON Canonicalization Scheme). Object members are sorted lexicographically by UTF-16 code-unit sequence, strings use the minimal ECMAScript `JSON.stringify` escape set (forward slash is **not** escaped), numbers serialize through the ECMAScript `Number.prototype.toString` algorithm via `ryu-js`, and insignificant whitespace is removed.
- **XML** follows InvoiceKit's no-comments XML Canonicalization 1.1 profile plus an invoice overlay: XML declarations and comments are dropped, empty elements are expanded, namespace declarations and attributes are sorted into canonical order, text and attribute escaping is normalized, and whitespace-only text between elements is removed. The overlay rewrites UBL and CII namespace prefixes to stable invoice prefixes (`cac`, `cbc`, `ext`, `qdt`, `udt`, `rsm`, `ram`, `ds`, `xades`), so two documents that differ only in their prefix choice canonicalize to identical bytes.

The crate is strict on purpose. It rejects inputs that would let a signed payload mean two things:

- Duplicate object member names (forbidden by RFC 8785 / I-JSON), caught before `serde_json` can silently collapse them.
- Integers outside the I-JSON interoperable safe range (`-9007199254740991 ..= 9007199254740991`).
- Non-finite numbers (`NaN`, `+Infinity`, `-Infinity`).
- XML DTDs and any entity reference outside the five predefined XML entities.
- Undeclared XML namespace prefixes.

## Public API

JSON:

- `canonicalize(input: &str) -> Result<String, CanonicalizeError>` — parse and canonicalize a JSON string.
- `canonicalize_value(value: &serde_json::Value) -> Result<String, CanonicalizeError>` — canonicalize an already-parsed value. (Duplicate member names are no longer representable in a `Value`, so this path only guards the number domain.)
- `CanonicalizeError` — `InvalidJson`, `DuplicateObjectMember`, `UnsafeInteger`, `NonFiniteNumber`.

XML:

- `canonicalize_xml(input: &str) -> Result<String, XmlCanonicalizeError>` — canonicalize an XML invoice document.
- `XmlCanonicalizeError` — `InvalidXml`, `InvalidAttribute`, `InvalidEncoding`, `InvalidName`, `UnboundPrefix`, `DuplicateAttribute`, `UnclosedElement`, `UnexpectedEndTag`, `UnsupportedDoctype`, `UnsupportedEntityReference`.

Misc:

- `crate_name() -> &'static str` — returns `"invoicekit-canonical"`.

## Where it sits in the pipeline

```
engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence
```

Canonicalization is the step that turns a serialized document into the exact bytes everything downstream signs and hashes. The signer crates, evidence bundles, and replay/verify all depend on this crate producing the same output every time. If two runs disagreed by a single byte, every signature built on top would be unverifiable.

## Usage

JSON:

```rust
let raw = r#"{ "b": 2 ,  "a":1 }"#;
let canonical = invoicekit_canonical::canonicalize(raw)?;
assert_eq!(canonical, r#"{"a":1,"b":2}"#);
```

From an in-memory value:

```rust
use serde_json::json;

let value = json!({"b": [3, 1, 2], "a": null});
let canonical = invoicekit_canonical::canonicalize_value(&value)?;
assert_eq!(canonical, r#"{"a":null,"b":[3,1,2]}"#);
```

XML, showing prefix normalization and empty-element expansion:

```rust
let raw = r#"<Invoice xmlns:x="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2"><x:AccountingSupplierParty/></Invoice>"#;
let canonical = invoicekit_canonical::canonicalize_xml(raw)?;
assert_eq!(
    canonical,
    r#"<Invoice><cac:AccountingSupplierParty xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2"></cac:AccountingSupplierParty></Invoice>"#
);
```

## Testing

The unit tests pin RFC 8785 test vectors (member ordering, number serialization, escape rules, UTF-16 sort) and the XML overlay behavior. A property-test harness in `tests/proptest_canonical.rs` asserts idempotence and determinism for both canonicalizers on generated input; CI runs it with `PROPTEST_CASES=10000`. These tests assert internal stability — they do not yet diff against an external reference implementation's output.

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
