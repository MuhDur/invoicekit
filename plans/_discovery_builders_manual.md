# InvoiceKit Builder's Manual — Local-Only End-to-End Country Support

Audience: an implementer adding a country report adapter and its offline lifecycle test to the InvoiceKit Rust workspace.
Scope: build a document in code → serialize to a wire format → sign + bundle + verify evidence → wire a national report adapter → write the E2E test and matrix entry, using **only crates already resolved in `Cargo.lock`**.

All Rust snippets below compile against the foundation crates as surveyed; minor identifier consistency has been corrected. Cite the exact `crate::symbol` names when you build.

---

## 1. How to build a valid `CommercialDocument` in code

The intermediate representation (IR) lives in `invoicekit-ir`. There is **no builder** — `CommercialDocumentParts` is a plain struct with all-public fields that you fill in directly, then hand to `CommercialDocument::new`.

### Construction paths
- `invoicekit_ir::CommercialDocument::new(parts: CommercialDocumentParts) -> Result<CommercialDocument, IrError>` — typed in-code path. Moves every field in, then calls `validate()`.
- `invoicekit_ir::CommercialDocument::try_from_value(value: serde_json::Value) -> Result<_, IrError>` — JSON-in path. serde decode errors surface as `IrError::Json`; shape errors as the typed variants.
- `invoicekit_ir::CommercialDocument::to_value(&self) -> Result<serde_json::Value, IrError>` — JSON-out path. The round-trip `to_value → try_from_value → to_value` is byte-stable.
- `invoicekit_ir::CommercialDocument::validate(&self) -> Result<(), IrError>` — runs automatically inside `new()` and `try_from_value()`.

### Newtypes validate at construction (no struct literals)
These have private inner fields, so you **must** go through `::new` (each returns `Result`):
- `DocumentId::new(impl Into<String>)` — non-empty stable id. Blank → `IrError::MissingRequiredField("id")`.
- `DocumentNumber::new(impl Into<String>)` — non-empty human invoice number.
- `DateOnly::new(impl Into<String>)` — strict `YYYY-MM-DD`, real calendar (rejects `2026-02-29`, `2026-13-01`) → `IrError::InvalidDate`.
- `Iso4217Code::new(impl Into<String>)` — 3 UPPERCASE ASCII letters (`"EUR"`); `"eur"`/`"EURO"` → `IrError::InvalidCurrency`. **Shared with `invoicekit-money`.**
- `CountryCode::new(impl Into<String>)` — 2 UPPERCASE ASCII letters (`"DE"`); used in `PostalAddress.country`.

### No floats — `DecimalValue` everywhere
- `DecimalValue::new(value: rust_decimal::Decimal)` is the IR's boundary money/quantity scalar. `MoneyAmount` and `Quantity` are **type aliases** for `DecimalValue`. It carries **no currency**.
- Build the inner `Decimal` with `rust_decimal::Decimal::new(minor_units, scale)` (`Decimal::new(11900, 2) == 119.00`), `Decimal::from(int)`, `Decimal::ONE`, `Decimal::ZERO`, or `Decimal::from_str`. The `dec!` macro is **not** available — `rust_decimal`'s `macros` feature is off.
- Amounts serialize as fixed-scale decimal **STRINGS** (`"119.00"`), not JSON numbers (via `rust_decimal::serde::str`). `try_from_value` expects a string for every amount field; a JSON number fails deserialization. This also dodges canonicalization's I-JSON unsafe-integer limit.

`invoicekit_money::Money::new(amount: Decimal, currency: Iso4217Code)` is a **separate** currency-tagged type for arithmetic (`add`/`sub`/`mul_scalar`/`round`/`allocate`). Compute totals with `Money`, then drop the resulting `Decimal` into `DecimalValue::new(...)` for the IR. `Iso4217Code` is shared (money depends on ir).

### Required, non-blank fields the validator enforces
- `meta.tenant_id` and `meta.trace_id` — both required, non-blank → `IrError::MissingRequiredField("meta.tenant_id" / "meta.trace_id")`. `DocumentMeta` has no constructor; build it literally. `source_system` is optional.
- `lines` MUST be non-empty → `IrError::EmptyCollection("lines")`. Each line validates `lines.id` and `lines.description` non-blank.
- `PostalAddress.lines` must be non-empty with every line non-blank; `city`, `postal_code`, `country` required. `tax_ids` defaults empty, but each present entry needs non-blank `scheme` and `value`.

### What validation does NOT do
It checks shapes, non-emptiness, dates, codes, and URN/payload envelopes — **not** arithmetic. It does not verify that line amounts sum to `monetary_total` or that tax math is correct. That is `invoicekit-tax-calculation` / the validators' job.

### `IrError` variants
`MissingRequiredField(&'static str)`, `EmptyCollection(&'static str)`, `InvalidDate(String)`, `InvalidCurrency(String)`, `InvalidCountryCode(String)`, `InvalidExtensionUrn(String)`, `InvalidExtensionPayload(String)`, `InvalidProfileUrn(String)`, `Json(serde_json::Error)`. Field paths like `"meta.tenant_id"`, `"party.address.city"` identify the offender.

### Optional `JurisdictionExtension`
`JurisdictionExtension::new(urn, payload: serde_json::Value)` — document- and line-level. URN must start with the `urn:` scheme (any case, normalised to lowercase, len > 4); payload must be non-null JSON. Bad URN → `InvalidExtensionUrn`; null payload → `InvalidExtensionPayload`. Not needed for a minimal invoice.

### Compilable example

```rust
// Cargo.toml deps:
//   invoicekit-ir       = { path = "crates/ir" }
//   invoicekit-money    = { path = "crates/money" }      // optional, for arithmetic
//   invoicekit-canonical= { path = "crates/canonical" }  // optional, for signing payload
//   rust_decimal        = { version = "1", default-features = false, features = ["serde-with-str","std"] }
//   serde_json          = "1"
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, Contact, CountryCode, DateOnly,
    DecimalValue, DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentType,
    Iso4217Code, MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion,
    TaxCategorySummary,
};
use rust_decimal::Decimal;

fn build_minimal_b2b_invoice() -> Result<CommercialDocument, invoicekit_ir::IrError> {
    // Fixed-scale decimals, never floats. Decimal::new(value, scale): 10000/2 = 100.00
    let amt = |minor: i64| DecimalValue::new(Decimal::new(minor, 2)); // money at scale 2
    let supplier = Party {
        id: Some("supplier-1".into()),
        name: "InvoiceKit GmbH".into(),
        tax_ids: vec![PartyTaxId { scheme: "vat".into(), value: "DE123456789".into() }],
        address: PostalAddress {
            lines: vec!["Main Street 1".into()],   // must be non-empty, each line non-blank
            city: "Berlin".into(),
            subdivision: None,
            postal_code: "10115".into(),
            country: CountryCode::new("DE")?,
        },
        contact: Some(Contact { name: None, email: Some("billing@example.invalid".into()), phone: None }),
    };
    let customer = Party {
        id: Some("customer-1".into()),
        name: "ACME SAS".into(),
        tax_ids: vec![PartyTaxId { scheme: "vat".into(), value: "FR123456789".into() }],
        address: PostalAddress {
            lines: vec!["Rue de Rivoli 1".into()],
            city: "Paris".into(),
            subdivision: None,
            postal_code: "75001".into(),
            country: CountryCode::new("FR")?,
        },
        contact: None,
    };
    let line = DocumentLine {
        id: "1".into(),
        description: "Validation subscription".into(),
        quantity: DecimalValue::new(Decimal::from(1)), // Quantity == DecimalValue
        unit_code: Some("EA".into()),
        unit_price: amt(10000),            // 100.00
        line_extension_amount: amt(10000), // 100.00
        tax_category: Some("S".into()),
        extensions: vec![],                // optional
    };
    let parts = CommercialDocumentParts {
        schema_version: SchemaVersion::V1_0,         // or Default::default()
        id: DocumentId::new("doc_2026_0001")?,
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-26")?,
        tax_point_date: None,
        due_date: Some(DateOnly::new("2026-06-25")?),
        document_number: DocumentNumber::new("INV-2026-0001")?,
        currency: Iso4217Code::new("EUR")?,
        supplier,
        customer,
        payee: None,
        payment_terms: None,
        payment_instructions: vec![],                // defaults to empty
        lines: vec![line],                           // MUST be non-empty
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".into(),
            taxable_amount: amt(10000), // 100.00
            tax_amount: amt(1900),      // 19.00
            tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))), // 19.00
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amt(10000), // 100.00
            tax_exclusive_amount: amt(10000),  // 100.00
            tax_inclusive_amount: amt(11900),  // 119.00
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: amt(11900),        // 119.00
        },
        attachments: vec![],
        references: vec![],
        notes: vec![],
        extensions: vec![],
        meta: DocumentMeta {
            tenant_id: "tenant_123".into(),   // REQUIRED, non-blank
            trace_id: "trace_abc".into(),     // REQUIRED, non-blank
            source_system: Some("my-app".into()),
        },
    };
    CommercialDocument::new(parts) // builds + validates
}

// Alternate path: build from JSON (amounts are STRINGS, not numbers).
fn from_json() -> Result<CommercialDocument, invoicekit_ir::IrError> {
    let v = serde_json::json!({ /* same shape; "unit_price":"100.00", "currency":"EUR", ... */ });
    CommercialDocument::try_from_value(v)
}
```

To discover field names/shapes programmatically: `invoicekit_ir::commercial_document_schema() -> serde_json::Value` returns the Draft 2020-12 JSON Schema; CI compares it against `schemas/invoicekit-ir-v1.json`.

---

## 2. How to serialize to UBL / CII / profiles

Serialization is per-syntax, dispatching on `document.document_type`. Each function calls `document.validate()` first, builds raw XML, then canonicalizes.

### Base serializers
- `invoicekit_format_ubl::to_xml(&CommercialDocument) -> Result<String, UblError>` — deterministic UBL 2.1. `Invoice` → root `<Invoice>`, `CreditNote` → `<CreditNote>`; any other type → `UblError::UnsupportedDocumentType`.
- `invoicekit_format_ubl::from_xml(&str) -> Result<(CommercialDocument, LossinessLedger), UblError>` — parse → re-serialize → re-parse → diff. Non-core top-level elements round-trip via a `JurisdictionExtension` keyed by `UBL_DOCUMENT_FIELDS_EXTENSION_URN` under a `top_level` array.
- `invoicekit_format_cii::to_xml(&CommercialDocument) -> Result<String, CiiError>` — deterministic CII D16B `<rsm:CrossIndustryInvoice>` (the Factur-X/ZUGFeRD syntax). Only `Invoice` (TypeCode 380) and `CreditNote` (TypeCode 381).
- `invoicekit_format_cii::from_xml(&str) -> Result<(CommercialDocument, LossinessLedger), CiiError>` — CII elements without an IR home are preserved as canonical raw XML fragments.

### Profile crates are pure projections, not new serializers
They clone the document, upsert one or two extension entries, then delegate to a base serializer:
- `invoicekit_profile_peppol_bis::to_peppol_bis_3_0_xml(&CommercialDocument) -> Result<String, PeppolBisError>` — upserts `cbc:CustomizationID` (`PEPPOL_BIS_3_0_CUSTOMIZATION_ID`) and `cbc:ProfileID` (`PEPPOL_BIS_3_0_PROFILE_ID`) into the `top_level` array of the `UBL_DOCUMENT_FIELDS_EXTENSION_URN` extension, then calls `format_ubl::to_xml`.
- `invoicekit_profile_xrechnung::to_xrechnung_3_x_xml(&CommercialDocument, &XRechnungOptions) -> Result<String, XRechnungError>` — same upsert pattern, plus an optional Leitweg-ID written into the `buyer_reference` document-fields key (emitted as `cbc:BuyerReference`; shape-validated ASCII alphanumeric + `-`). Then `format_ubl::to_xml`.
- `invoicekit_profile_factur_x::to_factur_x_cii_xml(&CommercialDocument, FacturXProfile) -> Result<String, FacturXError>` — six profiles (`Minimum`, `BasicWl`, `Basic`, `En16931`, `Extended`, `Xrechnung`). Goes through **CII**: injects the profile guideline URN (`FacturXProfile::guideline_urn`) into `CII_PROFILE_CONTEXT_EXTENSION_URN`, then `format_cii::to_xml`.
- `invoicekit_profile_factur_x::project(&CommercialDocument, FacturXProfile) -> Result<ProjectedDocument, FacturXError>` — same projection but returns `ProjectedDocument { document, ledger }` so you can inspect what each profile drops. Serialize `.document` yourself.

### Public seams
- `invoicekit_format_ubl::UBL_DOCUMENT_FIELDS_EXTENSION_URN = "urn:invoicekit:ubl:2.1:document-fields"` — payload carries the `top_level` override array plus named fields (`accounting_cost`, `buyer_reference`). Also re-exports `INVOICEKIT_METADATA_EXTENSION_URN`, `coverage_for`, `UblDocumentKind`.
- `invoicekit_format_cii::mapping::CII_PROFILE_CONTEXT_EXTENSION_URN` — payload carries `guideline_context_ids`, mapped to `GuidelineSpecifiedDocumentContextParameter`.

### Determinism
Comes from `invoicekit_canonical::canonicalize_xml(&str) -> Result<String, XmlCanonicalizeError>` (a no-comments XML C14N 1.1 invoice profile), which the format crates call internally — **you do not call it yourself**. It drops the XML declaration and comments, expands empty elements, sorts namespace declarations and attributes, normalizes prefixes to stable invoice prefixes (`cac`/`cbc`/`ext`/`ik` for UBL; `rsm`/`ram`/`udt`/`qdt` for CII), and strips inter-element whitespace. Same document in ⇒ identical bytes out. Do not add your own XML declaration or pretty-printing; it is normalized away.

### Gotchas
- `to_xml` returns `String`, not `Vec<u8>`. Call `.into_bytes()` (owned) or `.as_bytes()` (borrowed) for bytes.
- Only `Invoice` and `CreditNote` serialize. UBL `CreditNote` **cannot** carry a top-level `cbc:DueDate` — leave `due_date: None` for credit notes, else `UblError::UnsupportedDocumentField`.
- `unit_code` differs by family: UBL uses `"EA"`, CII/Factur-X uses UN/ECE Rec 20 codes like `"C62"`. Same IR field; pick the right code for the target.
- Output is well-formed canonical XML but **not** XSD/Schematron-validated by `to_xml`. Reference validation is separate (`format-ubl::schema::validate_oasis_ubl_2_1_schema`, or the JVM phive/KoSIT validator workers).
- `from_xml` costs two serializations (parse → re-serialize → re-parse to build the ledger). An empty `ledger.lost` is the signal the IR captured everything.

### Compilable example

```rust
use invoicekit_format_ubl::to_xml;
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue,
    DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code,
    MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use rust_decimal::Decimal;

fn ubl_invoice_bytes() -> anyhow::Result<Vec<u8>> {
    let amount = DecimalValue::new(Decimal::new(10000, 2)); // 100.00
    let party = Party {
        id: Some("party-1".to_owned()),
        name: "Example GmbH".to_owned(),
        tax_ids: vec![PartyTaxId { scheme: "vat".to_owned(), value: "DE123456789".to_owned() }],
        address: PostalAddress {
            lines: vec!["Main Street 1".to_owned()],
            city: "Berlin".to_owned(),
            subdivision: None,
            postal_code: "10115".to_owned(),
            country: CountryCode::new("DE")?,
        },
        contact: None,
    };

    let doc = CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::V1_0,
        id: DocumentId::new("doc-1")?,
        document_type: DocumentType::Invoice,
        issue_date: DateOnly::new("2026-05-29")?,
        tax_point_date: None,
        due_date: None,
        document_number: DocumentNumber::new("INV-1")?,
        currency: Iso4217Code::new("EUR")?,
        supplier: party.clone(),
        customer: party,
        payee: None,
        payment_terms: None,
        payment_instructions: Vec::new(),
        lines: vec![DocumentLine {
            id: "1".to_owned(),
            description: "Service".to_owned(),
            quantity: DecimalValue::new(Decimal::ONE),
            unit_code: Some("EA".to_owned()), // UBL uses EA; CII would use C62
            unit_price: amount.clone(),
            line_extension_amount: amount.clone(),
            tax_category: Some("S".to_owned()),
            extensions: Vec::new(),
        }],
        tax_summary: vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: amount.clone(),
            tax_amount: DecimalValue::new(Decimal::new(1900, 2)),
            tax_rate: Some(DecimalValue::new(Decimal::new(1900, 2))),
        }],
        monetary_total: MonetaryTotal {
            line_extension_amount: amount.clone(),
            tax_exclusive_amount: amount.clone(),
            tax_inclusive_amount: DecimalValue::new(Decimal::new(11900, 2)),
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount: DecimalValue::new(Decimal::new(11900, 2)),
        },
        attachments: Vec::new(),
        references: Vec::new(),
        notes: Vec::new(),
        extensions: Vec::new(),
        meta: DocumentMeta {
            tenant_id: "tenant".to_owned(),
            trace_id: "trace".to_owned(),
            source_system: None,
        },
    })?;

    // to_xml returns a canonicalized String; .into_bytes() gives the UBL XML bytes.
    let xml: String = to_xml(&doc)?;
    Ok(xml.into_bytes())
}

// For CII instead: invoicekit_format_cii::to_xml(&doc) -> Result<String, CiiError>
//   (set line.unit_code to "C62", not "EA").
// For a Peppol BIS UBL document, swap the last two lines for:
//   invoicekit_profile_peppol_bis::to_peppol_bis_3_0_xml(&doc)?.into_bytes()
```

> Italy note: FatturaPA is a **national** XML format (`FatturaElettronica`), not UBL/CII. `format-ubl`/`format-cii` will not emit it. See §4.

---

## 3. How to sign + build + verify an evidence bundle (`.ikb`)

The bundle is content-addressed: a typed BLAKE3 manifest plus artefact payloads. Signing and timestamping run **over the canonical `manifest.json` bytes**, not over your own serde of the manifest.

### Core types and functions (`invoicekit-evidence`)
- `EvidenceBundle { manifest: Manifest, artefacts: BTreeMap<String, Vec<u8>> }` — artefacts keyed by lexicographically sortable id (`canonical.json`, `formats/ubl.xml`, `receipts/peppol.json`). **Never** insert `manifest.json` (`MANIFEST_ARTEFACT_ID`) into `artefacts`; `pack()` injects it.
- `Manifest { schema_version, created_at, tenant_id, trace_id, artefacts: Vec<ArtefactEntry> }` — `schema_version` must equal `SCHEMA_VERSION` (`"1.0"`). `created_at` is the **determinism knob**: caller-pinned RFC-3339 UTC.
- `manifest_for(&BTreeMap<String,Vec<u8>>, tenant_id, trace_id, created_at) -> Manifest` — computes size + blake3 per entry, sorts by id. The normal way to build the manifest.
- `pack(&EvidenceBundle) -> Result<Vec<u8>, BundleError>` — bit-stable `.ikb`: zstd-compressed tar, entries sorted by id, normalized `uid=0/gid=0/mtime=0/mode=0644`, manifest re-emitted. Errors on unsafe artefact paths (absolute, `..`, backslash, empty).
- `unpack(&[u8]) -> Result<EvidenceBundle, BundleError>` — hoists `manifest.json` into `bundle.manifest`, runs `verify()` automatically.
- `verify(&EvidenceBundle) -> Result<(), BundleError>` — re-hashes every artefact; catches `HashDrift` / `MissingArtefact` / `UnknownArtefact` / `UnsupportedSchema`. Allows the reserved `signatures/manifest.dsse` sidecar.

### Verify orchestration (`invoicekit-verify`)
- `verify_packed(&[u8], &VerifyOptions) -> Result<VerifyReport, VerifyError>` — top-level entry: unpack then run the four opt-in checks. Returns `Err` **only** when the container won't parse. Hash/signature/timestamp failures are `Failed` outcomes in the report, not `Err`. **`exit 0 == report.ok == true`**.
- `verify(&EvidenceBundle, &VerifyOptions) -> VerifyReport` — same checks against an already-unpacked bundle.
- `VerifyOptions { signer, signature, manifest_dsse_signer, require_manifest_dsse, timestamp_client, timestamp, timestamp_algorithm }` — each check is independent opt-in. Start from `VerifyOptions::content_only()` then `..` spread to enable more. `content_only` always exits 0 on an untampered bundle.
- `VerifyReport { ok, content_address, signature, manifest_envelope, timestamp: CheckOutcome }`; `CheckOutcome::{Passed, Skipped{reason}, Failed{error}}`. `ok` is `true` iff no check FAILED; `Skipped` checks do not pull it red.
- `sign_bundle(&EvidenceBundle, &dyn Signer, KeyRef) -> Result<Signature, SignBundleError>` — signs over the canonical `manifest.json` bytes.
- `canonical_manifest_bytes(&EvidenceBundle) -> Result<Vec<u8>, BundleError>` — recover the exact bytes a signer/TSA must sign. Needed because manifest bytes are re-derived via a pack/unpack round-trip, not stored in `artefacts`.

### Signing substrate (`invoicekit-signer`)
- `trait Signer { fn sign(&self, &SignRequest) -> Result<Signature, SigningError>; fn list_keys(&self) -> Vec<KeyRef>; }`
- `SignRequest { payload: Vec<u8>, key_ref: KeyRef }`, `Signature { key_ref, algorithm, signature_b64 }`, `KeyRef(pub String)`.
- `SoftwareSigner::new().with_key(key_ref, [u8; 32])` — deterministic in-process test signer, keyed BLAKE3 MAC, `algorithm = "blake3-keyed-256"`. The `[u8;32]` is the deterministic test key (e.g. `[7u8;32]`).
- `MockSigner::new(known_keys)` — records calls; `algorithm = "mock-blake3-256"`.
- `UnixSocketSigner::new(socket_path)` (cfg unix) — production agent client over JSON-RPC; same trait, swap without changing call sites.

### DSSE bundle-level signing (`invoicekit-evidence-dsse`) — the recommended path
`attach_manifest_dsse(&EvidenceBundle, &dyn ManifestSigner) -> Result<EvidenceBundle, DsseError>` wraps a DSSE envelope over the canonical `manifest.json` bytes at the reserved sidecar id `signatures/manifest.dsse` (`MANIFEST_SIGNATURE_ARTEFACT_ID`). Verify with `VerifyOptions.manifest_dsse_signer`. `MockSigner::default()` is the deterministic test `ManifestSigner` (no key material needed).

### Timestamping (`invoicekit-timestamping`)
`MockTimestampClient::with_fixed_time(time, tsa_name)`; build the imprint with `invoicekit_verify::recompute_imprint(algorithm, manifest_bytes)`, request a token, verify with `VerifyOptions.timestamp_client + .timestamp + .timestamp_algorithm`.

### Gotchas
- **Determinism hinges on `Manifest.created_at`** (plus `tenant_id`/`trace_id`). There is **no clock call** inside the crates — the caller pins the time. Hold it constant for byte-stable `pack()`.
- Sign/timestamp over `canonical_manifest_bytes(...)`, not over your own `serde_json` of the `Manifest`, or the signature check fails.
- `require_manifest_dsse = true` forces a `Failed` when the sidecar is absent even with no signer wired.
- **All crypto today is placeholder**: `SoftwareSigner` = keyed BLAKE3 MAC; DSSE `MockSigner` = FNV-derived digest; RFC 3161 is mocked (SHA-2 mapped onto BLAKE3, length-padded); ZATCA `invoice_sha256_hex` is an FNV stand-in. Real RSA/ECDSA, HSM/PKCS#11, ZATCA secp256k1, CFDI RSA-SHA256 land behind future feature flags. Signature comparison uses `subtle::ConstantTimeEq`.
- Artefact ids are validated on `pack`: no absolute paths, no `..`, no backslash, no empty components, valid UTF-8, normalization round-trips unchanged.

### Compilable example (sign + pack + verify → `report.ok` == exit 0)

```rust
use std::collections::BTreeMap;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_verify::{sign_bundle, verify_packed, CheckOutcome, VerifyOptions};
use invoicekit_signer::{KeyRef, SoftwareSigner};

// 1. Assemble artefacts (id -> bytes). Do NOT add "manifest.json"; pack injects it.
let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
artefacts.insert("canonical.json".into(), br#"{"id":"INV-1"}"#.to_vec());
artefacts.insert("formats/ubl.xml".into(), b"<Invoice/>".to_vec());

// 2. Build the BLAKE3 manifest. created_at is caller-pinned -> deterministic pack().
let manifest = manifest_for(&artefacts, "tenant-a", "trace-1", "2026-05-29T00:00:00Z");
let bundle = EvidenceBundle { manifest, artefacts };

// 3. Sign over the bundle's canonical manifest.json bytes (deterministic test key).
let signer = SoftwareSigner::new().with_key("seal", [7u8; 32]);
let signature = sign_bundle(&bundle, &signer, KeyRef::new("seal")).unwrap();

// 4. Pack to portable .ikb (zstd tar, normalized metadata, lexicographic order).
let ikb: Vec<u8> = pack(&bundle).unwrap();

// 5. Verify: content-address always runs; signature check is opt-in here.
let report = verify_packed(&ikb, &VerifyOptions {
    signer: Some(&signer),
    signature: Some(&signature),
    ..VerifyOptions::content_only()   // pass content_only() alone for hash-only verify
}).unwrap();

assert!(report.ok);                                       // -> CLI exit 0
assert_eq!(report.content_address, CheckOutcome::Passed);
assert_eq!(report.signature, CheckOutcome::Passed);
```

---

## 4. The canonical report-adapter pattern (national vs Peppol) and what `report-it-sdi` needs

### Two distinct shapes

**National-clearance** (Spain VeriFactu, Kenya eTIMS, **Italy SDI**): a synchronous `Accepted`/`Rejected` verdict, country authority artifacts (Spain CSV + chained hash; Kenya CU Invoice Number + KRA signature; SDI `IdentificativoSdI` + receipt-kind), **one** verb (register/submit). **Italy is national-clearance — model it on ES/KE, not BE.**

**Peppol / EN 16931** (Belgium): a lifecycle ladder `Submitted → Delivered → Accepted/Rejected/ValidationFailed` reflecting async Message Level Response, an MLR reason, **two** verbs (`deliver` + `poll_status`), receiver lookup keys, payload is Peppol BIS UBL.

### The canonical adapter contract (every report-* crate)
- `pub trait XProvider: Send + Sync { fn <verb>(&self, &XRequest) -> Result<XEnvelope, XError>; }` — always `Send + Sync`. Real backends and the Mock both implement it.
- `XRequest` — typed input. Invariant fields across all adapters: `tenant_id` (mirrored from gateway context), an `Environment` enum (`Sandbox`/`Production`, kebab-case serde), the raw canonical payload as `Vec<u8>`, plus country-specific identity fields.
- `XEnvelope` — typed receipt carrying the **real** country artifacts, an `XStatus` verdict enum, an RFC-3339 timestamp, and an optional reason. Has a dedicated serde round-trip test.
- `XStatus` — `#[serde(rename_all="kebab-case")]` enum, the authority verdict.
- `XError` — `thiserror` enum, three buckets: payload-shape, country-id-shape, transport.
- `MockXProvider` — deterministic mock: fixed timestamp + `std::sync::Mutex<u64>` serial, runs the **same** `validate_*(request)?` validators the real impl runs, synthesizes receipt fields with a country-tagged prefix.

### The two load-bearing rules (anti-slop)
1. **Rejection is NOT an error.** Every adapter returns `Ok(envelope-with-Rejected-status)` when the authority refuses. `Err` is reserved for (a) pre-wire shape validation and (b) transport/TLS/DNS. For SDI, **NS (Notifica di Scarto) is a `SdiReceiptKind`, not an `Err`.** Inverting this breaks the audit-trail contract.
2. **The mock must run the same validators as the real impl, and encode genuinely country-specific content.** 40 near-identical clones = fake parity. Italy must encode real Partita IVA (11 digits) / Codice Fiscale (16 alphanumeric) validators, the real ProgressivoInvio (1..=5 alphanumeric) rule, the real five SDI receipt kinds, and FatturaPA — not the generic `CommercialDocument` verbatim.

### What `report-it-sdi` needs exactly

Today it is a **60-line identity stub**: `crate_name()` plus four trivial tests, **zero `[dependencies]`** in `Cargo.toml`. Per the Coverage Loop §1 it is missing all six honest-bar layers. The asymmetry to exploit: Italy **already has** a built-out `invoicekit-signer-sdi` (the *signer* layer), but the *report* adapter is the stub — the opposite of wave-2/3 countries. **Compose `invoicekit-signer-sdi`; do not re-invent the SDI receipt.**

`invoicekit-signer-sdi` already provides (verified surface):
- `trait SdiProvider { fn submit(&self, &SdiSubmitRequest) -> Result<SdiStampEnvelope, SdiError>; }`
- `SdiStampEnvelope { signature: Signature, identificativo_sdi: String, receipt_kind: SdiReceiptKind, progressivo_invio: String, signed_fattura_xml: Vec<u8>, transport: SdiTransport }`
- `SdiSubmitRequest { fattura_xml: Vec<u8>, certificate: ArubaQualifiedCertificate, transport: SdiTransport, progressivo_invio: String }`
- `enum SdiReceiptKind { RicevutaConsegna, NotificaScarto, MancataConsegna, NotificaEsito, Metadata }` (the five real Agenzia delle Entrate receipts — RC/NS/MC/NE/MT). Method `SdiReceiptKind::is_delivered(self) -> bool`.
- `enum SdiTransport { WebService, Pec }`
- `MockSdiProvider::new(name, Arc<dyn Signer>)`, `.with_forced_receipt(SdiReceiptKind)`, `.submissions()`. Inner `Signer` is **keyed by `certificate.serial_number`** (deterministic test key `SoftwareSigner::with_key(serial, [2u8;32])`). Synthesizes `IdentificativoSdI = IT{nnn}`.

The six honest-bar layers to add to `report-it-sdi`:
1. **Step 1 — serialize**: a real IR → FatturaPA (`FatturaElettronica`) serializer, or at minimum a clearly-labelled "faithful typed payload". Emitting the generic `CommercialDocument` and calling it FatturaPA violates the anti-slop rule.
2. **Step 2 — local validate** (`invoicekit-validate`): pure-Rust `ValidationResult`/`Severity`/`RuleId`; reference-grade stays labelled `requires_external_backend`. Rule set from `invoicekit_rulepack::Registry::seeded()?.pack_for(country, profile_id, date)` (Italy needs a FatturaPA/EN16931-IT rulepack entry).
3. **Steps 3+4 — sign + transmit**: compose `MockSdiProvider` over a `SoftwareSigner` keyed by the cert serial.
4. **Step 5 — evidence**: `manifest_for(...)` over a `BTreeMap` of `{canonical.json, formats/fattura.xml, signed.xml, receipt.json}`, `pack` to `.ikb`, assert `verify(&bundle) == Ok` (exit 0).
5. **Step 6 — matrix entry**: see §5.
6. **Step 7 — E2E test**: `tests/e2e_offline_lifecycle.rs` driving steps 1→7 (no report-* crate has a `tests/` dir yet; you build the first one).

Also add country-specific free-standing validators (load-bearing anti-slop content): `validate_italian_tax_id` (P.IVA 11 digits / CF 16 alphanumeric) and `validate_progressivo` (1..=5 alphanumeric). The `Cargo.toml` must explicitly add deps (it currently has none): `serde`, `serde_json`, `thiserror.workspace = true`, `invoicekit-signer`, `invoicekit-signer-sdi`, plus dev-deps `invoicekit-evidence`, `invoicekit-ir`, `invoicekit-validate`, `invoicekit-rulepack`. (Apache-2.0 header on every file; `.gitignore` blocks `crates/*/src/bin/` — keep logic in `lib.rs`/`tests/`.)

Mock-transmit route choice: for a first honest Italy adapter use **route (a)** — the country-local `MockSdiProvider` (matches the ES/KE/BE in-crate pattern, lowest risk). Route (b) is the shared `invoicekit_transmit_mock::MockGatewayAdapter` driven by a `.vcr` cassette keyed on method+path+BLAKE3-body-fingerprint, plugging into `invoicekit_reconcile`'s `GatewayAdapter`/`GatewayStatus`/`TransmissionBaseState` ladder (heavier).

### Compilable skeleton — `crates/report-it-sdi/src/lib.rs`

```rust
#![allow(clippy::doc_markdown)]
use std::sync::Arc;
use invoicekit_signer::Signer;
use invoicekit_signer_sdi::{
    ArubaQualifiedCertificate, MockSdiProvider, SdiProvider, SdiReceiptKind,
    SdiStampEnvelope, SdiSubmitRequest, SdiTransport,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SdiEnvironment { Sandbox, Production }

/// The operator-facing typed request (mirrors es/ke/be shape).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SdiReportRequest {
    pub tenant_id: String,
    pub environment: SdiEnvironment,
    /// Issuer Partita IVA (11 digits) or Codice Fiscale (16 alphanumeric).
    pub issuer_tax_id: String,
    /// 5-char alphanumeric ProgressivoInvio.
    pub progressivo_invio: String,
    pub transport: SdiTransport,
    pub certificate: ArubaQualifiedCertificate,
    /// Canonical FatturaPA XML bytes (produced by an IR->FatturaPA serializer).
    pub fattura_xml: Vec<u8>,
}

/// Typed report verdict (re-uses SDI's five real receipt kinds).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SdiReportEnvelope {
    pub identificativo_sdi: String,
    pub receipt_kind: SdiReceiptKind,
    pub progressivo_invio: String,
    pub recorded_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Error)]
pub enum SdiReportError {
    #[error("fattura xml rejected: {0}")] BadXml(String),
    #[error("invalid issuer tax id: {0}")] BadTaxId(String),
    #[error("invalid progressivo invio: {0}")] BadProgressivo(String),
    #[error("sdi signer/transport failure: {0}")] Transport(String),
}

pub trait SdiReportProvider: Send + Sync {
    /// serialize-is-upstream; this does validate->sign->transmit(SDI)->typed receipt.
    /// SDI NS (Scarto/rejection) is surfaced as receipt_kind, NOT as Err.
    fn report(&self, req: &SdiReportRequest) -> Result<SdiReportEnvelope, SdiReportError>;
}

/// Deterministic mock: composes the existing MockSdiProvider so the real SDI
/// signature path + IdentificativoSdI synthesis are exercised, not re-faked.
pub struct MockSdiReportProvider { inner: MockSdiProvider, fixed_recorded_at: String }
impl MockSdiReportProvider {
    #[must_use] pub fn new(signer: Arc<dyn Signer>) -> Self {
        Self { inner: MockSdiProvider::new("aruba-test", signer),
               fixed_recorded_at: "2026-07-01T00:00:00Z".to_owned() }
    }
}
impl SdiReportProvider for MockSdiReportProvider {
    fn report(&self, req: &SdiReportRequest) -> Result<SdiReportEnvelope, SdiReportError> {
        validate_italian_tax_id(&req.issuer_tax_id)?;     // country-specific (anti-slop)
        validate_progressivo(&req.progressivo_invio)?;
        if req.fattura_xml.is_empty() { return Err(SdiReportError::BadXml("payload is empty".into())); }
        let stamp: SdiStampEnvelope = self.inner.submit(&SdiSubmitRequest {
            fattura_xml: req.fattura_xml.clone(), certificate: req.certificate.clone(),
            transport: req.transport, progressivo_invio: req.progressivo_invio.clone(),
        }).map_err(|e| SdiReportError::Transport(e.to_string()))?;
        Ok(SdiReportEnvelope {
            identificativo_sdi: stamp.identificativo_sdi,
            receipt_kind: stamp.receipt_kind,
            progressivo_invio: stamp.progressivo_invio,
            recorded_at: self.fixed_recorded_at.clone(), reason: None,
        })
    }
}

/// Partita IVA = 11 digits; Codice Fiscale = 16 alphanumeric. Real, testable shape.
pub fn validate_italian_tax_id(id: &str) -> Result<(), SdiReportError> {
    let piva = id.len() == 11 && id.bytes().all(|b| b.is_ascii_digit());
    let cf   = id.len() == 16 && id.bytes().all(|b| b.is_ascii_alphanumeric());
    if piva || cf { Ok(()) } else {
        Err(SdiReportError::BadTaxId(format!("expected 11-digit P.IVA or 16-char CF, got {id:?}")))
    }
}
pub fn validate_progressivo(p: &str) -> Result<(), SdiReportError> {
    if (1..=5).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()) { Ok(()) }
    else { Err(SdiReportError::BadProgressivo(format!("ProgressivoInvio must be 1..=5 alphanumeric, got {p:?}"))) }
}

#[must_use] pub const fn crate_name() -> &'static str { "invoicekit-report-it-sdi" }
```

The signer wiring at the call site (Italy SDI, deterministic test key keyed by cert serial):

```rust
use std::sync::Arc;
use invoicekit_signer::{Signer, SoftwareSigner};
use invoicekit_signer_sdi::{ArubaQualifiedCertificate, SdiTransport};

let cert = ArubaQualifiedCertificate {
    serial_number: "1234567890ABCDEF".into(),
    codice_fiscale: "RSSMRA80A01H501U".into(),
    subject_dn: "CN=Mario Rossi,O=Acme SRL,C=IT".into(),
    certificate_pem: b"-----BEGIN CERTIFICATE-----\n...stub...".to_vec(),
};
// The inner Signer is keyed by the certificate serial_number.
let inner: Arc<dyn Signer> =
    Arc::new(SoftwareSigner::new().with_key("1234567890ABCDEF", [2u8; 32]));
let provider = MockSdiReportProvider::new(inner);
let env = provider.report(&SdiReportRequest {
    tenant_id: "tenant_123".into(),
    environment: SdiEnvironment::Sandbox,
    issuer_tax_id: "12345678901".into(),       // 11-digit P.IVA
    progressivo_invio: "ABCDE".into(),         // 1..=5 alphanumeric
    transport: SdiTransport::WebService,
    certificate: cert,
    fattura_xml: b"<FatturaElettronica/>".to_vec(),
}).unwrap();
assert!(env.receipt_kind.is_delivered());      // RicevutaConsegna on the happy path
```

---

## 5. E2E test house style + ready-to-adapt Italy matrix entry

### House style
- Integration/E2E tests live **per-crate** in `crates/<crate>/tests/*.rs`. There is **no** top-level integration-test crate (`/tests/` holds only `fuzz_crashes/`). No report-* crate has a `tests/` dir yet — you build the first one at `crates/report-it-sdi/tests/e2e_offline_lifecycle.rs`.
- Every source file starts with the SPDX + copyright header:
  ```rust
  // SPDX-License-Identifier: Apache-2.0
  // Copyright 2026 The InvoiceKit Authors
  ```
  followed by a doc-comment naming the bead / strict-acceptance gate the test backs.
- Resolve fixtures relative to the crate, not CWD: `PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap()` is the repo root.
- Conventions observed across `format-ubl` / `format-detect` / `canonical` tests:
  - Tolerate a missing optional corpus: `if !dir.is_dir() { eprintln!("skipping ..."); return; }`.
  - Assert a minimum fixture **count** (e.g. `>= 20`) so the gate cannot silently pass on an empty dir.
  - Collect failures into a `Vec<String>` and `assert!(failures.is_empty(), "...\n  - {}", join)` for readable diffs.
  - Assert determinism: serialize twice, `assert_eq!` the two outputs.
  - Golden tests freeze output to `tests/golden/*.golden`, refreshed with `UPDATE_GOLDENS=1`; `.actual` files are gitignored. The idiom is **hand-rolled** (manual `.golden` read + env flag), **not** `insta`.
- The §1 step-5 success criterion for the Italy E2E is `verify(&bundle) == Ok` (exit 0) — assert it. The E2E drives steps 1→7: serialize IR→FatturaPA bytes; local validate (non-fatal); `MockSdiReportProvider::report()`; `evidence::pack({canonical.json, formats/fattura.xml, signed.xml, receipt.json})` then `verify()==Ok`; assert a `matrix.json` IT entry exists; the test itself is the deterministic E2E.
- Use the inline serde-round-trip + validator tests in `report-es-verifactu`/`report-ke-etims`/`report-be-peppol` as the unit-test template.

### Matrix CI gates (two, distinct)
1. **Python/jsonschema gate** — `tools/release-checks/test_capabilities_matrix.py` (`.github/workflows/capabilities.yml`): validates `crates/cli/data/capabilities/matrix.json` against `schemas/invoicekit-capabilities-v1.json` (Draft 2020-12) and asserts `schema_version == "1.0"`.
2. **Rust semantic gate** — `invoicekit_cli::commands::capabilities::validate_matrix_semantics()`, exercised when `bundled_matrix()` loads the `include_str!` matrix. `SUPPORTED_MATRIX_SCHEMA_VERSION = "1.0"`.

### The two semantic invariants (easy to get wrong)
- **Invariant #1:** `capabilities.unavailable_in_wasm` MUST equal **exactly** the set of operations among `{serialize, local_validate, reference_validate}` whose level is not `"available"`. If `reference_validate` is `"requires_external_backend"`, then `"reference_validate"` must appear in `unavailable_in_wasm`, and nothing `"available"` may appear. Mismatch → panic at startup.
- **Invariant #2:** any profile with an op at level `"requires_external_backend"` MUST declare a non-empty `requires_service` OR `requires_cli` (e.g. `["jvm:fatturapa-validator"]`).

Schema notes: `additionalProperties:false` everywhere; all six `ProfileRuntimeCapabilities` fields required (empty arrays ok). Closed enums — `format`: `UBL|CII|Factur-X|XRechnung|Peppol BIS|Peppol PINT|FatturaPA|Chorus Pro`; `transport`: `peppol|email|portal|as4-direct|manual`; `CapabilityLevel`: `available|requires_external_backend|unavailable_in_wasm`; `scenario`: `B2B|B2C|B2G`; `confidence`: `authoritative|high|medium|low`. `route_from`/`route_to` match `^[A-Z]{2}$` (alpha-2). `source` requires `name`, `fetched_at`, `confidence` (`url` optional). `stale_after_days` is 180; a too-old `fetched_at` resolves as stale.

> **Conflict warning:** `matrix.json` **already** contains an `IT → IT` `B2B` row (lines 211–238) using `transport: "portal"`, `requires_service: ["partner:sdi"]`, with `local_validate` marked `unavailable_in_wasm` (`unavailable_in_wasm: ["local_validate","reference_validate"]`). The honest variant below routes `reference_validate` through a JVM validator and marks `local_validate` as `available`. **Do not co-list both for the same route+scenario+date window** unless you intend two profiles — decide deliberately whether to replace the `partner:sdi` row.

### Ready-to-adapt Italy capability matrix entry (append to the `entries` array)

```json
{
  "route_from": "IT",
  "route_to": "IT",
  "scenario": "B2B",
  "valid_from": "2019-01-01",
  "valid_until": null,
  "profiles": [
    {
      "id": "fatturapa-1.2.2",
      "format": "FatturaPA",
      "transport": "portal",
      "capabilities": {
        "serialize": "available",
        "local_validate": "available",
        "reference_validate": "requires_external_backend",
        "requires_service": ["jvm:fatturapa-validator"],
        "requires_cli": [],
        "unavailable_in_wasm": ["reference_validate"]
      }
    }
  ],
  "source": {
    "name": "Agenzia delle Entrate Sistema di Interscambio (SDI) FatturaPA specification",
    "url": "https://www.fatturapa.gov.it/",
    "fetched_at": "2026-05-01T00:00:00Z",
    "confidence": "authoritative"
  }
}
```

This entry satisfies both invariants: `unavailable_in_wasm` lists exactly `reference_validate` (the only non-`available` op), and the `requires_external_backend` op declares `requires_service: ["jvm:fatturapa-validator"]`. Run the matrix consumer with `--locked` in CI (`cargo build -p invoicekit-cli --bin invoicekit --locked`) so any unintended `Cargo.lock` drift is a hard failure.

---

## 6. Dependencies already available to reuse (so E2E tests never touch `Cargo.lock`)

### Workspace path crates (add as `[dependencies]` / `[dev-dependencies]` with `path = "../<crate>"`)
- `invoicekit-signer` (`../signer`) — `Signer`, `SoftwareSigner`, `SignRequest`/`Signature`/`KeyRef`.
- `invoicekit-signer-sdi` (`../signer-sdi`) — **ALREADY BUILT**: `SdiProvider`/`SdiSubmitRequest`/`SdiStampEnvelope`/`SdiReceiptKind(RicevutaConsegna,NotificaScarto,MancataConsegna,NotificaEsito,Metadata)`/`SdiTransport(WebService,Pec)`/`ArubaQualifiedCertificate`/`MockSdiProvider`. Compose, don't duplicate.
- `invoicekit-evidence` (`../evidence`) — `EvidenceBundle`/`Manifest`/`manifest_for`/`pack`/`unpack`/`verify`/`blake3_hex`.
- `invoicekit-verify` (`../verify`) — `verify_packed`/`verify`/`sign_bundle`/`canonical_manifest_bytes`/`VerifyOptions`/`VerifyReport`/`CheckOutcome`.
- `invoicekit-ir` (`../ir`) — `CommercialDocument`/`CommercialDocumentParts`/`DocumentId`/`DocumentNumber`/all value objects/`IrError`.
- `invoicekit-money` (`../money`) — `Money` arithmetic (optional).
- `invoicekit-canonical` (`../canonical`) — `canonicalize_value`/`canonicalize`/`canonicalize_xml`.
- `invoicekit-format-ubl`, `invoicekit-format-cii` (`../format-ubl`, `../format-cii`) — `to_xml`/`from_xml`. (FatturaPA needs a national serializer, not these.)
- `invoicekit-profile-peppol-bis`, `invoicekit-profile-xrechnung`, `invoicekit-profile-factur-x` (`../profile-*`) — profile projections.
- `invoicekit-validate` (`../validate`) — `ValidationResult`/`Severity`/`RuleId`/`explain_plan_from_results`.
- `invoicekit-rulepack` (`../rulepack`) — `Registry::seeded()`/`pack_for(country, profile, date)`.
- `invoicekit-reconcile` (`../reconcile`) — `GatewayAdapter`/`GatewayReceipt`/`GatewayStatus`/`SubmitRequest`/`GatewayContext`/`TransmissionBaseState`.
- `invoicekit-transmit-mock` (`../transmit-mock`) — `MockGatewayAdapter` + `.vcr` cassette recorder/scrubber/matcher (BLAKE3 body fingerprint) + `SCENARIO_METADATA_SCHEMA_JSON`.
- `invoicekit-evidence-dsse` (`../evidence-dsse`), `invoicekit-timestamping` (`../timestamping`) — optional DSSE + RFC 3161 substrates.

Workspace path crates pin to version `0.0.0` and most are `publish = false` (workspace-internal). Inherit version/edition/lints via `version.workspace = true`, `edition.workspace = true`, `[lints] workspace = true`.

### Third-party crates ALREADY in `Cargo.lock` (Lock-safe — reuse the same version strings)
- `serde` (`version = "1"`, `features = ["derive"]`) — declared directly by `report-es-verifactu`/`report-gr-mydata`.
- `serde_json` (`version = "1"`) — canonical uses the `preserve_order` variant.
- `thiserror` — workspace-pinned `thiserror.workspace = true` (`= "2"`).
- `tracing` — workspace-pinned `"0.1"`.
- `rust_decimal` (`version = "1"`, `default-features = false`, `features = ["serde-with-str","std"]`) — `Decimal::new` etc.
- `proptest` (`"1"`) — dev-dependency idiom in `format-ubl`/`canonical`.
- `sha2` (`"0.10"`) — dev-dependency of `format-ubl`.

### Lock-unsafe — DO NOT add
`insta` and `pretty_assertions` are **not** in `Cargo.lock`; adding them mutates it. Stay with the repo's hand-rolled golden idiom (manual `.golden` read + `UPDATE_GOLDENS` env). The capability matrix to edit lives at `crates/cli/data/capabilities/matrix.json`; its schema at `schemas/invoicekit-capabilities-v1.json`.
