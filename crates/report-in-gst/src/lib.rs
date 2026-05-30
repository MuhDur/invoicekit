// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! India **GST e-invoicing** via the Invoice Registration Portal (IRP).
//!
//! Under the Goods and Services Tax mandate every notified
//! Indian taxpayer (turnover above the current threshold —
//! ₹5 crore at time of writing) issues B2B invoices through
//! an **IRP** (Invoice Registration Portal). The IRP
//! validates the invoice payload, assigns an **IRN** (Invoice
//! Reference Number — a 64-char SHA-256 hex of the invoice's
//! canonical fields), signs a JWS over the invoice, and
//! returns a base-64 PNG / TLV string for the **signed QR**
//! the issuer prints on the invoice.
//!
//! Multiple IRPs exist (NIC IRP1, NIC IRP2, IRIS IRP, EY
//! GSP, Cygnet GSP, etc.). The shape of every request +
//! response is identical — this crate captures it as a single
//! [`IrpProvider`] trait so operator code never re-derives
//! the IRP wire shape.
//!
//! Mock `MockIrpProvider` ships for tests + cassette-replay.
//! Real backends land in feature-flagged
//! `report-in-gst-http` / `report-in-gst-nic` follow-ups.

#![allow(clippy::doc_markdown)]

use std::fmt::Write as _;

use invoicekit_ir::{CommercialDocument, DocumentLine, DocumentType, Party, ReferenceKindClass};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use thiserror::Error;

// ---------------------------------------------------------------------------
// IRP INV-01 serialization (IR -> national e-invoice JSON)
// ---------------------------------------------------------------------------
//
// India's e-invoice is a JSON document (NOT XML): the IRP "generate IRN"
// request body is the `INV-01` schema published by the Goods and Services Tax
// Network (GSTN) / National Informatics Centre (NIC). The element names below
// are the REAL schema field names — abbreviated PascalCase keys like `TranDtls`,
// `DocDtls`, `SellerDtls`, `ItemList`, `ValDtls` — not UBL relabelled.
//
// Spec: NIC IRP e-Invoice JSON schema `INV-01` (schema version `1.1`),
//   <https://einvoice1.gst.gov.in/Documents/EINVOICE_SCHEMA.xlsx> and the
//   bulk-generation tools at
//   <https://einvoice1.gst.gov.in/Others/BulkGenerationTools>.
//
// Tax-split rule (CGST Act 2017 / IGST Act 2017): the first two GSTIN digits
// are the supplier/buyer state code. An intra-state supply (same state code)
// splits the tax into Central GST (`CgstAmt`) + State GST (`SgstAmt`), each at
// half the headline rate; an inter-state supply (or export) charges Integrated
// GST (`IgstAmt`) at the full rate.

/// IRP `INV-01` schema version this serializer emits (`TranDtls`-level pin).
const INV01_SCHEMA_VERSION: &str = "1.1";

/// Transaction-level context for the `INV-01` `TranDtls` block — the fields
/// that are India-specific and have no jurisdiction-agnostic IR home.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Inv01Context {
    /// `SupTyp` — supply type code: `B2B`, `SEZWP`, `SEZWOP`, `EXPWP`,
    /// `EXPWOP`, or `DEXP`. Defaults to `B2B`.
    pub supply_type: String,
}

impl Default for Inv01Context {
    fn default() -> Self {
        Self {
            supply_type: "B2B".to_owned(),
        }
    }
}

/// Errors raised while serializing an IR document to IRP `INV-01` JSON.
#[derive(Debug, Error)]
pub enum Inv01Error {
    /// The IR `document_type` has no `INV-01` `DocDtls.Typ` mapping.
    #[error("document type {0:?} is not representable as INV-01 DocDtls.Typ")]
    UnsupportedDocumentType(DocumentType),
    /// The supplier (`SellerDtls`) carries no GSTIN usable as `Gstin`.
    #[error("supplier has no GSTIN usable as SellerDtls.Gstin")]
    MissingSellerGstin,
    /// The transaction context was malformed (e.g. blank `SupTyp`).
    #[error("invalid INV-01 transaction context: {0}")]
    BadContext(String),
}

/// Serialize an InvoiceKit [`CommercialDocument`] to deterministic IRP `INV-01`
/// e-invoice JSON (schema version `1.1`).
///
/// This is the REAL Indian national format: a JSON payload with the abbreviated
/// PascalCase keys the NIC Invoice Registration Portal validates (`Version`,
/// `TranDtls`, `DocDtls`, `SellerDtls`, `BuyerDtls`, `ItemList`, `ValDtls`).
/// It is NOT a UBL/EN 16931 XML re-skin — the UBL serializer emits the family
/// format; this emits the country format the IRP actually accepts.
///
/// **Coverage.** The serializer now emits the IRP-mandatory party and item
/// fields mapped from the IR: `SellerDtls`/`BuyerDtls.Addr1` (+ optional
/// `Addr2`), `BuyerDtls.Pos` (place of supply), and per-item `PrdDesc`,
/// `IsServc`, and `Unit` (the IR unit code mapped to the IRP UQC set via
/// `unit_uqc`). The per-line `HsnCd` is sourced from the IR line
/// classification (EN 16931 BT-158, e.g. an `HSN`/`SAC`-scheme classification):
/// the serializer uses the chosen classification's `code` as `HsnCd`, falling
/// back to the generic SAC heading `"9983"` only when the line carries no
/// classification. `IsServc` is derived from the chosen classification (a `SAC`
/// scheme or a chapter-99 code ⇒ service), falling back to the chapter-99 rule
/// on the resolved code, so it always agrees with `HsnCd`.
///
/// Output is byte-stable by construction: a fixed key order via an explicit
/// [`serde_json::Map`] insertion sequence, amounts rendered at fixed scale 2,
/// and no timestamps. The document is expected to have passed IR validation
/// (it has, if built via [`CommercialDocument::new`]).
///
/// # Tax split
///
/// The first two GSTIN digits encode the state code. When supplier and buyer
/// share a state code the supply is intra-state and the per-item tax is split
/// into `CgstAmt` + `SgstAmt` (half the headline rate each); otherwise it is
/// inter-state and charged as `IgstAmt`. A buyer with no GSTIN (export) is
/// treated as inter-state (`IgstAmt`).
///
/// # Errors
///
/// Returns [`Inv01Error::UnsupportedDocumentType`] for document types with no
/// `DocDtls.Typ` mapping, [`Inv01Error::MissingSellerGstin`] when the supplier
/// has no GSTIN, and [`Inv01Error::BadContext`] when the context is malformed.
pub fn to_inv01_json(
    document: &CommercialDocument,
    context: &Inv01Context,
) -> Result<String, Inv01Error> {
    if context.supply_type.is_empty() {
        return Err(Inv01Error::BadContext(
            "SupTyp must not be empty".to_owned(),
        ));
    }
    let doc_typ = doc_type_code(document.document_type)?;
    let seller_gstin = party_gstin(&document.supplier).ok_or(Inv01Error::MissingSellerGstin)?;
    let buyer_gstin = party_gstin(&document.customer);

    // Intra-state when both parties share the leading two-digit state code.
    // The state code is the ASCII GSTIN prefix; `gstin_state_prefix` returns it
    // only when the first two bytes are ASCII, so the byte-index-2 slice can
    // never split a multibyte character on junk input.
    let intra_state = match (
        gstin_state_prefix(seller_gstin.as_str()),
        buyer_gstin.as_deref().and_then(gstin_state_prefix),
    ) {
        (Some(seller), Some(buyer)) => seller == buyer,
        _ => false,
    };

    let mut root = Map::new();
    root.insert("Version".to_owned(), json!(INV01_SCHEMA_VERSION));

    // TranDtls — transaction details.
    let mut tran = Map::new();
    tran.insert("TaxSch".to_owned(), json!("GST"));
    tran.insert("SupTyp".to_owned(), json!(context.supply_type));
    root.insert("TranDtls".to_owned(), Value::Object(tran));

    // DocDtls — document details.
    let mut doc = Map::new();
    doc.insert("Typ".to_owned(), json!(doc_typ));
    doc.insert("No".to_owned(), json!(document.document_number.as_str()));
    doc.insert(
        "Dt".to_owned(),
        json!(fmt_doc_date(document.issue_date.as_str())),
    );
    root.insert("DocDtls".to_owned(), Value::Object(doc));

    // SellerDtls / BuyerDtls — party blocks.
    root.insert(
        "SellerDtls".to_owned(),
        party_block(&document.supplier, Some(seller_gstin.as_str()), None),
    );
    // `BuyerDtls.Pos` (place of supply) is the buyer's GST state code — the same
    // value `intra_state` is derived from above.
    let buyer_pos = state_code(&document.customer, buyer_gstin.as_deref());
    root.insert(
        "BuyerDtls".to_owned(),
        party_block(&document.customer, buyer_gstin.as_deref(), Some(&buyer_pos)),
    );

    // ItemList — one entry per IR line.
    let mut items = Vec::with_capacity(document.lines.len());
    let mut central_total = Decimal::ZERO;
    let mut state_total = Decimal::ZERO;
    let mut integrated_total = Decimal::ZERO;
    let mut assessable_total = Decimal::ZERO;
    for (index, line) in document.lines.iter().enumerate() {
        let item = item_block(document, line, index, intra_state)?;
        // checked_add: summing untrusted per-line amounts can exceed
        // Decimal::MAX. Surface overflow as a typed error rather than panicking
        // on the `+=` accumulator.
        let (cgst, sgst, igst) = line_tax_amounts(document, line, intra_state)?;
        assessable_total = assessable_total
            .checked_add(line.line_extension_amount.inner())
            .ok_or_else(|| Inv01Error::BadContext("AssVal total overflowed".to_owned()))?;
        central_total = central_total
            .checked_add(cgst)
            .ok_or_else(|| Inv01Error::BadContext("CgstVal total overflowed".to_owned()))?;
        state_total = state_total
            .checked_add(sgst)
            .ok_or_else(|| Inv01Error::BadContext("SgstVal total overflowed".to_owned()))?;
        integrated_total = integrated_total
            .checked_add(igst)
            .ok_or_else(|| Inv01Error::BadContext("IgstVal total overflowed".to_owned()))?;
        items.push(item);
    }
    root.insert("ItemList".to_owned(), Value::Array(items));

    // ValDtls — document-level value summary.
    let tax_inclusive = document.monetary_total.tax_inclusive_amount.inner();
    let mut val = Map::new();
    val.insert("AssVal".to_owned(), json!(fmt_amount(assessable_total)));
    val.insert("CgstVal".to_owned(), json!(fmt_amount(central_total)));
    val.insert("SgstVal".to_owned(), json!(fmt_amount(state_total)));
    val.insert("IgstVal".to_owned(), json!(fmt_amount(integrated_total)));
    val.insert("TotInvVal".to_owned(), json!(fmt_amount(tax_inclusive)));
    root.insert("ValDtls".to_owned(), Value::Object(val));

    // RefDtls.PrecDocDtls — preceding-document details: the original invoice(s)
    // a credit/debit note refers to. Mapped verbatim from the IR references
    // classified as a preceding invoice (`InvNo` = id, `InvDt` = its issue date
    // as dd/mm/yyyy); no code-list mapping. Emitted only when such a reference
    // is present, so a document without one serializes exactly as before.
    let prec_docs: Vec<Value> = document
        .references
        .iter()
        .filter(|r| r.kind_class() == ReferenceKindClass::PrecedingInvoice)
        .map(|r| {
            let mut prec = Map::new();
            prec.insert("InvNo".to_owned(), json!(r.id));
            if let Some(issue_date) = &r.issue_date {
                prec.insert("InvDt".to_owned(), json!(fmt_doc_date(issue_date.as_str())));
            }
            Value::Object(prec)
        })
        .collect();
    if !prec_docs.is_empty() {
        let mut ref_dtls = Map::new();
        ref_dtls.insert("PrecDocDtls".to_owned(), Value::Array(prec_docs));
        root.insert("RefDtls".to_owned(), Value::Object(ref_dtls));
    }

    // serde_json's `preserve_order` feature keeps `Map` insertion-ordered, so
    // the INV-01 key sequence (and the whole output) is byte-deterministic.
    serde_json::to_string(&Value::Object(root))
        .map_err(|e| Inv01Error::BadContext(format!("INV-01 JSON serialization failed: {e}")))
}

/// Map an IR [`DocumentType`] to an `INV-01` `DocDtls.Typ` code (`INV` / `CRN`
/// / `DBN` per the NIC IRP document-type codelist).
fn doc_type_code(document_type: DocumentType) -> Result<&'static str, Inv01Error> {
    match document_type {
        DocumentType::Invoice => Ok("INV"),
        DocumentType::CreditNote => Ok("CRN"),
        DocumentType::DebitNote => Ok("DBN"),
        other @ (DocumentType::ProForma | DocumentType::SelfBilled) => {
            Err(Inv01Error::UnsupportedDocumentType(other))
        }
    }
}

/// The party's GSTIN, taken from a `gst`-scheme tax id when present, else the
/// first tax id. Returns `None` for a party with no tax ids (export buyer).
fn party_gstin(party: &Party) -> Option<String> {
    party
        .tax_ids
        .iter()
        .find(|t| t.scheme.eq_ignore_ascii_case("gst"))
        .or_else(|| party.tax_ids.first())
        .map(|t| t.value.clone())
}

/// Build a `SellerDtls` / `BuyerDtls` object: `Gstin`, `LglNm` (legal name),
/// `Loc` (location/city), `Pin` (postal code as integer), `Stcd` (state code).
fn party_block(party: &Party, gstin: Option<&str>, pos: Option<&str>) -> Value {
    let mut block = Map::new();
    // `Gstin` is `URP` ("Unregistered Person") in the IRP schema when the party
    // carries no GSTIN — the canonical placeholder for export/B2C buyers.
    block.insert("Gstin".to_owned(), json!(gstin.unwrap_or("URP")));
    block.insert("LglNm".to_owned(), json!(party.name));
    // `Addr1` is IRP-mandatory and `Addr2` optional; the IR address carries an
    // ordered line list (guaranteed non-empty by IR validation). Map the first
    // line onto `Addr1` and fold any remaining lines into `Addr2` so no address
    // content is dropped on the two-slot IRP shape.
    let mut lines = party.address.lines.iter();
    block.insert(
        "Addr1".to_owned(),
        json!(lines.next().map_or("", String::as_str)),
    );
    let addr2 = lines.cloned().collect::<Vec<_>>().join(", ");
    if !addr2.is_empty() {
        block.insert("Addr2".to_owned(), json!(addr2));
    }
    block.insert("Loc".to_owned(), json!(party.address.city));
    block.insert("Pin".to_owned(), pin_value(&party.address.postal_code));
    // `Stcd` is the two-digit GST state code (the GSTIN prefix). Falls back to
    // the address subdivision when the party has no GSTIN.
    block.insert("Stcd".to_owned(), json!(state_code(party, gstin)));
    // `Pos` (place of supply) is a `BuyerDtls`-only field: the destination state
    // code that determines intra- vs inter-state tax. Sellers carry no `Pos`.
    if let Some(pos) = pos {
        block.insert("Pos".to_owned(), json!(pos));
    }
    Value::Object(block)
}

/// `Pin` as a JSON number when the postal code is all digits (the IRP schema
/// types `Pin` as an integer), else as the raw string.
fn pin_value(postal_code: &str) -> Value {
    if !postal_code.is_empty() && postal_code.bytes().all(|b| b.is_ascii_digit()) {
        postal_code
            .parse::<u64>()
            .map_or_else(|_| json!(postal_code), |n| json!(n))
    } else {
        json!(postal_code)
    }
}

/// The two-character GST state code: the GSTIN prefix when present, else the
/// address subdivision, else `"96"` (the IRP "Other Country" code for foreign
/// parties in export scenarios).
fn state_code(party: &Party, gstin: Option<&str>) -> String {
    if let Some(prefix) = gstin.and_then(gstin_state_prefix) {
        return prefix.to_owned();
    }
    party
        .address
        .subdivision
        .as_deref()
        .map_or_else(|| "96".to_owned(), str::to_owned)
}

/// The two-character GST state-code prefix of a GSTIN, or `None` when the input
/// is shorter than two bytes or its first two bytes are not ASCII.
///
/// A real GSTIN is ASCII alphanumeric, so its leading two characters are always
/// available. Guarding on `is_ascii` keeps the byte-index-2 slice on a UTF-8
/// character boundary, so malformed multibyte input degrades gracefully instead
/// of panicking.
fn gstin_state_prefix(gstin: &str) -> Option<&str> {
    let bytes = gstin.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii() && bytes[1].is_ascii() {
        Some(&gstin[..2])
    } else {
        None
    }
}

/// Build one `ItemList` entry: `SlNo`, `HsnCd`, `Qty`, `UnitPrice`, `TotAmt`,
/// `AssAmt`, `GstRt`, the CGST/SGST or IGST split, and `TotItemVal`.
fn item_block(
    document: &CommercialDocument,
    line: &DocumentLine,
    index: usize,
    intra_state: bool,
) -> Result<Value, Inv01Error> {
    let ass_amt = line.line_extension_amount.inner();
    let rate = line_tax_rate(document, line);
    let (cgst, sgst, igst) = line_tax_amounts(document, line, intra_state)?;
    // checked_add: assessable base + the tax split on untrusted amounts can
    // exceed Decimal::MAX. Surface overflow as a typed error, not a panic.
    let tot_item_val = ass_amt
        .checked_add(cgst)
        .and_then(|x| x.checked_add(sgst))
        .and_then(|x| x.checked_add(igst))
        .ok_or_else(|| Inv01Error::BadContext("TotItemVal overflowed".to_owned()))?;

    let (hsn, is_service) = hsn_code(line);
    let mut item = Map::new();
    // `SlNo` (serial number) is a string in the IRP schema.
    item.insert("SlNo".to_owned(), json!((index + 1).to_string()));
    // `PrdDesc` (product description) is IRP-mandatory; map the IR line text.
    item.insert("PrdDesc".to_owned(), json!(line.description));
    // `IsServc` (Y/N) flags a service line. SAC headings (services) sit in
    // chapter 99; HSN codes (goods) sit elsewhere. When the line carries an IR
    // classification, the flag comes from the chosen classification (SAC scheme
    // or a chapter-99 code); otherwise it falls back to the chapter-99 rule on
    // the resolved code so it always agrees with `HsnCd`.
    item.insert(
        "IsServc".to_owned(),
        json!(if is_service { "Y" } else { "N" }),
    );
    item.insert("HsnCd".to_owned(), json!(hsn));
    item.insert("Qty".to_owned(), json!(fmt_qty(line.quantity.inner())));
    // `Unit` is the IRP unit-quantity code (UQC); map the IR unit code onto the
    // UQC set, defaulting to `OTH` ("Others") when absent or unrecognized.
    item.insert(
        "Unit".to_owned(),
        json!(unit_uqc(line.unit_code.as_deref())),
    );
    item.insert(
        "UnitPrice".to_owned(),
        json!(fmt_amount(line.unit_price.inner())),
    );
    item.insert("TotAmt".to_owned(), json!(fmt_amount(ass_amt)));
    item.insert("AssAmt".to_owned(), json!(fmt_amount(ass_amt)));
    item.insert("GstRt".to_owned(), json!(fmt_amount(rate)));
    if intra_state {
        item.insert("CgstAmt".to_owned(), json!(fmt_amount(cgst)));
        item.insert("SgstAmt".to_owned(), json!(fmt_amount(sgst)));
    } else {
        item.insert("IgstAmt".to_owned(), json!(fmt_amount(igst)));
    }
    item.insert("TotItemVal".to_owned(), json!(fmt_amount(tot_item_val)));
    Ok(Value::Object(item))
}

/// The line's headline GST rate (`GstRt`), looked up from the tax summary entry
/// matching the line's tax category. Defaults to zero.
fn line_tax_rate(document: &CommercialDocument, line: &DocumentLine) -> Decimal {
    line.tax_category
        .as_ref()
        .and_then(|cat| {
            document
                .tax_summary
                .iter()
                .find(|s| &s.category_code == cat)
                .and_then(|s| s.tax_rate.as_ref())
        })
        .map_or(Decimal::ZERO, invoicekit_ir::DecimalValue::inner)
}

/// The per-line tax split `(cgst, sgst, igst)` computed from the line's taxable
/// base and headline rate. Intra-state halves the rate into CGST + SGST;
/// inter-state charges the full rate as IGST.
fn line_tax_amounts(
    document: &CommercialDocument,
    line: &DocumentLine,
    intra_state: bool,
) -> Result<(Decimal, Decimal, Decimal), Inv01Error> {
    let base = line.line_extension_amount.inner();
    let rate = line_tax_rate(document, line);
    let hundred = Decimal::from(100);
    // checked_mul/checked_div: `base * rate` on untrusted amounts can exceed
    // Decimal::MAX. Surface overflow as a typed error rather than panicking.
    let full = base
        .checked_mul(rate)
        .and_then(|product| product.checked_div(hundred))
        .ok_or_else(|| Inv01Error::BadContext("line tax base*rate overflowed".to_owned()))?;
    if intra_state {
        let half = full
            .checked_div(Decimal::TWO)
            .ok_or_else(|| Inv01Error::BadContext("line tax split overflowed".to_owned()))?
            .round_dp(2);
        Ok((half, half, Decimal::ZERO))
    } else {
        Ok((Decimal::ZERO, Decimal::ZERO, full.round_dp(2)))
    }
}

/// The line's `(HsnCd, is_service)`, sourced from the IR line classification
/// (EN 16931 BT-158 *Item classification identifier* + BT-158-1 scheme id).
///
/// When the line carries one or more classifications we pick one — preferring a
/// classification whose `scheme_id` is `HSN`/`SAC` (case-insensitive), else the
/// first — and use its `code` as `HsnCd`. The service flag is `true` when the
/// chosen scheme is `SAC` (case-insensitive) or the code sits in chapter 99
/// (the SAC range), so `IsServc` always agrees with `HsnCd`.
///
/// When the line has NO classification we fall back to the placeholder `"9983"`
/// (the SAC heading for "Other professional, technical and business services"),
/// which keeps the field a valid 4-digit code without inventing line-level data
/// the IR does not carry; its chapter-99 prefix makes it a service.
fn hsn_code(line: &DocumentLine) -> (String, bool) {
    let chosen = line
        .classifications
        .iter()
        .find(|c| {
            c.scheme_id.eq_ignore_ascii_case("HSN") || c.scheme_id.eq_ignore_ascii_case("SAC")
        })
        .or_else(|| line.classifications.first());
    chosen.map_or_else(
        // No classification: the generic SAC services heading (chapter 99 ⇒
        // service) keeps the field schema-valid and matches the prior behavior.
        || ("9983".to_owned(), true),
        |classification| {
            let is_service = classification.scheme_id.eq_ignore_ascii_case("SAC")
                || classification.code.starts_with("99");
            (classification.code.clone(), is_service)
        },
    )
}

/// Map an IR unit code (typically a UN/ECE Recommendation 20 code) onto the IRP
/// unit-quantity-code (UQC) set. A value already in the UQC set passes through;
/// common UN/ECE codes are translated; anything absent or unrecognized falls
/// back to `OTH` ("Others"), a valid UQC, so the field is always schema-valid
/// without inventing a unit the IR did not carry.
fn unit_uqc(unit_code: Option<&str>) -> &'static str {
    let code = unit_code.unwrap_or("").trim().to_ascii_uppercase();
    match code.as_str() {
        "NOS" | "C62" | "NMB" => "NOS",
        "PCS" | "EA" | "PCE" | "H87" => "PCS",
        "KGS" | "KGM" => "KGS",
        "GMS" | "GRM" => "GMS",
        "TON" | "TNE" => "TON",
        "LTR" | "LITRE" => "LTR",
        "MLT" | "MILLILITRE" => "MLT",
        "MTR" | "METRE" => "MTR",
        "CMS" | "CMT" => "CMS",
        "SQM" | "MTK" => "SQM",
        "CBM" | "MTQ" => "CBM",
        "BOX" | "BX" => "BOX",
        "DOZ" | "DZN" => "DOZ",
        "HRS" | "HUR" => "HRS",
        "DAY" | "DAYS" => "DAY",
        "KWH" => "KWH",
        "BAG" | "BAGS" => "BAG",
        "BTL" | "BOTTLES" => "BTL",
        "SET" | "SETS" => "SET",
        _ => "OTH",
    }
}

/// Format an IRP date `dd/mm/yyyy` from an ISO `yyyy-mm-dd` string. Falls back
/// to the input verbatim when it is not the expected ISO shape.
fn fmt_doc_date(iso: &str) -> String {
    let parts: Vec<&str> = iso.split('-').collect();
    if let [y, m, d] = parts.as_slice() {
        if y.len() == 4 && m.len() == 2 && d.len() == 2 {
            return format!("{d}/{m}/{y}");
        }
    }
    iso.to_owned()
}

/// Format a decimal at fixed scale 2 (`100` -> `"100.00"`, `0` -> `"0.00"`),
/// padding trailing zeros so the IRP `*Amt` / `*Val` fields are scale-stable.
fn fmt_amount(value: Decimal) -> String {
    format!("{:.2}", value.round_dp(2))
}

/// Format a quantity at fixed scale 3 (the IRP `Qty` precision), deterministic.
fn fmt_qty(value: Decimal) -> String {
    format!("{:.3}", value.round_dp(3))
}

/// Which IRP backend the engine talks to. Strings stay
/// opaque so new IRPs can plug in without bumping this enum.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IrpBackend {
    /// Government IRP1 (`einvoice1.gst.gov.in`).
    Nic1,
    /// Government IRP2 (`einvoice2.gst.gov.in`).
    Nic2,
    /// Any private GSP / IRP. The string is the operator-side
    /// vendor label so cassettes can pin a recording to one
    /// specific IRP.
    Gsp(String),
}

/// Environment selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IrpEnvironment {
    /// `einv-apisandbox.nic.in` / GSP sandbox tier.
    Sandbox,
    /// Production.
    Production,
}

/// IRP per-invoice verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IrpStatus {
    /// Successfully registered; IRN + signed QR returned.
    Accepted,
    /// Duplicate IRN — IRP returns the existing IRN; engine
    /// should reconcile against it instead of issuing fresh.
    Duplicate,
    /// IRP refused the payload. Fix + resubmit.
    Rejected,
}

/// What the operator passes in to
/// [`IrpProvider::register_invoice`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IrpRegisterRequest {
    /// Tenant identifier mirrored from the gateway context.
    pub tenant_id: String,
    /// Environment selector.
    pub environment: IrpEnvironment,
    /// Which IRP backend handles this request.
    pub backend: IrpBackend,
    /// Issuer's 15-character GSTIN (Goods and Services Tax
    /// Identification Number).
    pub issuer_gstin: String,
    /// Buyer's GSTIN; `None` for export / B2C transactions
    /// that don't carry a buyer GSTIN.
    pub buyer_gstin: Option<String>,
    /// Canonical IRP JSON payload (Schema-1.1 at time of
    /// writing). The provider does NOT pre-sign — the IRP
    /// signs on its side.
    pub invoice_json: Vec<u8>,
}

/// What [`IrpProvider::register_invoice`] returns when the
/// IRP has registered the invoice.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IrpRegisterEnvelope {
    /// IRP verdict.
    pub status: IrpStatus,
    /// 64-char IRN (Invoice Reference Number). `None` only
    /// for `Rejected` status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub irn: Option<String>,
    /// IRP acknowledgement number (numeric string). Engines
    /// quote this in support tickets with the IRP.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ack_no: Option<String>,
    /// RFC-3339 UTC timestamp the IRP recorded.
    pub ack_dt: String,
    /// Base-64 PNG of the signed QR (the engine writes this
    /// straight into the printed invoice).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed_qr_code: Option<String>,
    /// JWS the IRP signed the invoice with. Engines persist
    /// this for offline verification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signed_invoice_jws: Option<String>,
    /// Free-form error text when `status == Rejected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Typed transport / validation / refusal errors.
#[derive(Debug, Error)]
pub enum IrpError {
    /// Invoice JSON failed shape validation before the wire.
    #[error("invoice json rejected: {0}")]
    BadJson(String),
    /// Issuer / buyer GSTIN didn't match the 15-char pattern.
    #[error("invalid GSTIN: {0}")]
    BadGstin(String),
    /// HTTP / TLS / DNS failure talking to the IRP.
    #[error("transport failure: {0}")]
    Transport(String),
}

/// The IRP integration surface. Real backends satisfy this
/// trait; the mock below is what tests + cassette-replay use.
pub trait IrpProvider: Send + Sync {
    /// Register one invoice with the IRP. The provider:
    ///
    /// 1. validates `issuer_gstin` (+ `buyer_gstin` when
    ///    supplied),
    /// 2. POSTs the invoice JSON to the backend endpoint
    ///    selected by `backend` + `environment`,
    /// 3. returns the IRP-issued envelope.
    ///
    /// The IRP-returned `Rejected` verdict is NOT an `Err` —
    /// it's surfaced via `IrpStatus::Rejected` inside the
    /// envelope so the engine persists the rejection
    /// alongside its audit trail.
    ///
    /// # Errors
    ///
    /// Returns [`IrpError`] when local validation fails
    /// before the wire or transport fails on the wire.
    fn register_invoice(
        &self,
        request: &IrpRegisterRequest,
    ) -> Result<IrpRegisterEnvelope, IrpError>;
}

/// Deterministic mock provider.
///
/// Emits a synthesised 64-hex-char IRN derived from the
/// payload length + first 24 bytes so cassette-replay tests
/// stay byte-identical across runs. Returns `Duplicate` when
/// the same IRN would be produced twice — i.e. when the same
/// payload is registered twice with the same provider.
pub struct MockIrpProvider {
    fixed_ack_dt: String,
    seen_irns: std::sync::Mutex<std::collections::BTreeSet<String>>,
    next_ack: std::sync::Mutex<u64>,
}

impl MockIrpProvider {
    /// Build a mock with deterministic timestamps + serials.
    #[must_use]
    pub fn new() -> Self {
        Self::with_fixed_ack_dt("2026-01-01T00:00:00Z")
    }

    /// Build a mock with a custom fixed timestamp (the mock
    /// emits this value verbatim in every `IrpRegisterEnvelope`).
    #[must_use]
    pub fn with_fixed_ack_dt(ack_dt: impl Into<String>) -> Self {
        Self {
            fixed_ack_dt: ack_dt.into(),
            seen_irns: std::sync::Mutex::new(std::collections::BTreeSet::new()),
            next_ack: std::sync::Mutex::new(1),
        }
    }
}

impl Default for MockIrpProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl IrpProvider for MockIrpProvider {
    fn register_invoice(
        &self,
        request: &IrpRegisterRequest,
    ) -> Result<IrpRegisterEnvelope, IrpError> {
        validate_gstin(&request.issuer_gstin)?;
        if let Some(buyer) = &request.buyer_gstin {
            validate_gstin(buyer)?;
        }
        if request.invoice_json.is_empty() {
            return Err(IrpError::BadJson("payload is empty".to_owned()));
        }

        // Synthesise a 64-hex "IRN" so callers can dedup.
        let mut irn = String::with_capacity(64);
        let _ = write!(irn, "{:0>16x}", request.invoice_json.len() as u64);
        for byte in request.invoice_json.iter().take(24) {
            let _ = write!(irn, "{byte:02x}");
        }
        while irn.len() < 64 {
            irn.push('0');
        }
        irn.truncate(64);

        let seen = {
            let mut guard = self.seen_irns.lock().expect("seen IRN mutex poisoned");
            let already = guard.contains(&irn);
            guard.insert(irn.clone());
            already
        };
        let ack_serial = {
            let mut g = self.next_ack.lock().expect("ack mutex poisoned");
            let v = *g;
            *g += 1;
            v
        };
        let ack_no = format!("ACK-{ack_serial:014}");
        Ok(IrpRegisterEnvelope {
            status: if seen {
                IrpStatus::Duplicate
            } else {
                IrpStatus::Accepted
            },
            irn: Some(irn.clone()),
            ack_no: Some(ack_no),
            ack_dt: self.fixed_ack_dt.clone(),
            signed_qr_code: Some(mock_qr_base64(&irn)),
            signed_invoice_jws: Some(mock_jws(&irn)),
            error_message: None,
        })
    }
}

fn mock_qr_base64(irn: &str) -> String {
    // The real IRP returns a base-64 PNG; the mock returns a
    // deterministic placeholder so cassettes stay
    // byte-identical.
    format!("MOCK-QR-{}", &irn[..16])
}

fn mock_jws(irn: &str) -> String {
    format!("eyJhbGciOiJSUzI1NiJ9.{}.MOCK_SIG", &irn[..32])
}

/// Validate that a GSTIN is exactly 15 ASCII alphanumeric chars.
///
/// Real shape: state code + PAN + entity number + check
/// digit. The full IRP modulo-checksum is a separate concern;
/// this helper only catches obviously-wrong shapes before the
/// wire.
///
/// # Errors
///
/// Returns [`IrpError::BadGstin`] when the input isn't 15
/// ASCII alphanumeric characters.
pub fn validate_gstin(gstin: &str) -> Result<(), IrpError> {
    if gstin.len() == 15 && gstin.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(())
    } else {
        Err(IrpError::BadGstin(format!(
            "GSTIN must be 15 ASCII alphanumeric chars, got {gstin:?}"
        )))
    }
}

/// Validate that an HSN (Harmonised System of Nomenclature)
/// or SAC (Services Accounting Code) is 4–8 ASCII digits.
///
/// # Errors
///
/// Returns [`IrpError::BadJson`] when the shape is wrong.
pub fn validate_hsn_sac(code: &str) -> Result<(), IrpError> {
    if (4..=8).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(IrpError::BadJson(format!(
            "HSN/SAC must be 4–8 ASCII digits, got {code:?}"
        )))
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_report_in_gst::crate_name(),
///     "invoicekit-report-in-gst"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-report-in-gst"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> IrpRegisterRequest {
        IrpRegisterRequest {
            tenant_id: "tenant-in-test".to_owned(),
            environment: IrpEnvironment::Sandbox,
            backend: IrpBackend::Nic1,
            issuer_gstin: "29AAAPL2356Q1ZS".to_owned(),
            buyer_gstin: Some("27AAAPL2356Q1ZT".to_owned()),
            invoice_json: br#"{"version":"1.1"}"#.to_vec(),
        }
    }

    #[test]
    fn register_invoice_returns_accepted_with_irn() {
        let p = MockIrpProvider::default();
        let env = p.register_invoice(&sample_request()).unwrap();
        assert_eq!(env.status, IrpStatus::Accepted);
        assert!(env.irn.as_ref().is_some_and(|s| s.len() == 64));
        assert!(env.ack_no.as_ref().is_some_and(|s| s.starts_with("ACK-")));
        assert_eq!(env.ack_dt, "2026-01-01T00:00:00Z");
        assert!(env.signed_qr_code.is_some());
        assert!(env.signed_invoice_jws.is_some());
        assert!(env.error_message.is_none());
    }

    #[test]
    fn register_invoice_detects_duplicate_on_resubmit() {
        let p = MockIrpProvider::default();
        let env1 = p.register_invoice(&sample_request()).unwrap();
        let env2 = p.register_invoice(&sample_request()).unwrap();
        assert_eq!(env1.status, IrpStatus::Accepted);
        assert_eq!(env2.status, IrpStatus::Duplicate);
        // Same IRN returned both times.
        assert_eq!(env1.irn, env2.irn);
    }

    #[test]
    fn register_invoice_rejects_empty_payload() {
        let p = MockIrpProvider::default();
        let mut req = sample_request();
        req.invoice_json.clear();
        let err = p.register_invoice(&req).unwrap_err();
        assert!(matches!(err, IrpError::BadJson(_)));
    }

    #[test]
    fn register_invoice_rejects_bad_issuer_gstin() {
        let p = MockIrpProvider::default();
        let mut req = sample_request();
        req.issuer_gstin = "TOO-SHORT".to_owned();
        let err = p.register_invoice(&req).unwrap_err();
        assert!(matches!(err, IrpError::BadGstin(_)));
    }

    #[test]
    fn register_invoice_rejects_bad_buyer_gstin() {
        let p = MockIrpProvider::default();
        let mut req = sample_request();
        req.buyer_gstin = Some("TOO-SHORT".to_owned());
        let err = p.register_invoice(&req).unwrap_err();
        assert!(matches!(err, IrpError::BadGstin(_)));
    }

    #[test]
    fn register_invoice_accepts_export_without_buyer_gstin() {
        let p = MockIrpProvider::default();
        let mut req = sample_request();
        req.buyer_gstin = None;
        let env = p.register_invoice(&req).unwrap();
        assert_eq!(env.status, IrpStatus::Accepted);
    }

    #[test]
    fn validate_gstin_accepts_well_formed_15_char_string() {
        assert!(validate_gstin("29AAAPL2356Q1ZS").is_ok());
    }

    #[test]
    fn validate_gstin_rejects_wrong_length() {
        assert!(validate_gstin("29AAAPL2356Q1Z").is_err());
        assert!(validate_gstin("29AAAPL2356Q1ZSS").is_err());
    }

    #[test]
    fn validate_gstin_rejects_non_alphanumeric() {
        assert!(validate_gstin("29-AAPL2356Q1ZS").is_err());
        assert!(validate_gstin("29 AAPL2356Q1ZS").is_err());
    }

    #[test]
    fn validate_hsn_sac_accepts_4_to_8_digits() {
        assert!(validate_hsn_sac("8471").is_ok());
        assert!(validate_hsn_sac("84713010").is_ok());
    }

    #[test]
    fn validate_hsn_sac_rejects_wrong_length() {
        assert!(validate_hsn_sac("847").is_err());
        assert!(validate_hsn_sac("847130100").is_err());
    }

    #[test]
    fn validate_hsn_sac_rejects_non_digits() {
        assert!(validate_hsn_sac("84A1").is_err());
    }

    #[test]
    fn backend_serde_round_trips_all_three_variants() {
        for backend in [
            IrpBackend::Nic1,
            IrpBackend::Nic2,
            IrpBackend::Gsp("iris".to_owned()),
        ] {
            let json = serde_json::to_string(&backend).unwrap();
            let parsed: IrpBackend = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, backend);
        }
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = IrpRegisterEnvelope {
            status: IrpStatus::Accepted,
            irn: Some("ab".repeat(32)),
            ack_no: Some("ACK-00000000000007".to_owned()),
            ack_dt: "2026-01-01T00:00:00Z".to_owned(),
            signed_qr_code: Some("MOCK-QR-abababab".to_owned()),
            signed_invoice_jws: Some("eyJhbGciOiJSUzI1NiJ9.x.MOCK_SIG".to_owned()),
            error_message: None,
        };
        let json = serde_json::to_string(&env).unwrap();
        let parsed: IrpRegisterEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, env);
    }
}

#[cfg(test)]
mod inv01_tests {
    use super::*;
    use invoicekit_ir::{
        CommercialDocument, CommercialDocumentParts, CountryCode, DateOnly, DecimalValue,
        DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, Iso4217Code,
        MonetaryTotal, Party, PartyTaxId, PostalAddress, SchemaVersion, TaxCategorySummary,
    };
    use rust_decimal::Decimal;

    fn amt(minor: i64) -> DecimalValue {
        DecimalValue::new(Decimal::new(minor, 2))
    }

    fn indian_party(name: &str, gstin: &str, city: &str, state: &str) -> Party {
        Party {
            id: Some(name.to_lowercase().replace(' ', "-")),
            name: name.to_owned(),
            tax_ids: vec![PartyTaxId {
                scheme: "gst".to_owned(),
                value: gstin.to_owned(),
            }],
            address: PostalAddress {
                lines: vec!["1 MG Road".to_owned()],
                city: city.to_owned(),
                subdivision: Some(state.to_owned()),
                postal_code: "560001".to_owned(),
                country: CountryCode::new("IN").unwrap(),
            },
            contact: None,
        }
    }

    /// Inter-state (Karnataka 29 -> Maharashtra 27) supply at 18% IGST.
    fn inter_state_invoice() -> CommercialDocument {
        CommercialDocument::new(CommercialDocumentParts {
            schema_version: SchemaVersion::default(),
            id: DocumentId::new("doc-in-1").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-25").unwrap()),
            invoice_period: None,
            delivery_date: None,
            document_number: DocumentNumber::new("INV-2026-IN-0001").unwrap(),
            currency: Iso4217Code::new("INR").unwrap(),
            supplier: indian_party(
                "Acme Technologies Pvt Ltd",
                "29AAAPL2356Q1ZS",
                "Bengaluru",
                "KA",
            ),
            customer: indian_party("Beta Solutions Pvt Ltd", "27AAAPL2356Q1ZT", "Mumbai", "MH"),
            payee: None,
            payment_terms: None,
            payment_instructions: Vec::new(),
            lines: vec![DocumentLine {
                id: "1".to_owned(),
                description: "Software consulting & support".to_owned(),
                quantity: DecimalValue::new(Decimal::from(2)),
                unit_code: Some("EA".to_owned()),
                unit_price: amt(500_000),
                line_extension_amount: amt(1_000_000),
                tax_category: Some("S".to_owned()),
                classifications: Vec::new(),
                extensions: Vec::new(),
                allowance_charges: Vec::new(),
            }],
            tax_summary: vec![TaxCategorySummary {
                category_code: "S".to_owned(),
                taxable_amount: amt(1_000_000),
                tax_amount: amt(180_000),
                tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))),
                exemption_reason: None,
                exemption_reason_code: None,
            }],
            monetary_total: MonetaryTotal {
                line_extension_amount: amt(1_000_000),
                tax_exclusive_amount: amt(1_000_000),
                tax_inclusive_amount: amt(1_180_000),
                allowance_total_amount: None,
                charge_total_amount: None,
                prepaid_amount: None,
                payable_amount: amt(1_180_000),
            },
            attachments: Vec::new(),
            references: Vec::new(),
            notes: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
            deliver_to: None,
            meta: DocumentMeta {
                tenant_id: "tenant_123".to_owned(),
                trace_id: "trace_abc".to_owned(),
                source_system: Some("inline".to_owned()),
            },
        })
        .unwrap()
    }

    /// Intra-state (both Karnataka, state code 29) supply: 18% splits 9% CGST + 9% SGST.
    fn intra_state_invoice() -> CommercialDocument {
        let mut doc = inter_state_invoice();
        // Re-build with a same-state buyer (state code 29).
        let parts = CommercialDocumentParts {
            schema_version: SchemaVersion::default(),
            id: DocumentId::new("doc-in-2").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-25").unwrap()),
            invoice_period: None,
            delivery_date: None,
            document_number: DocumentNumber::new("INV-2026-IN-0002").unwrap(),
            currency: Iso4217Code::new("INR").unwrap(),
            supplier: doc.supplier.clone(),
            customer: indian_party("Gamma Pvt Ltd", "29BBBPL6789Q1Z5", "Mysuru", "KA"),
            payee: None,
            payment_terms: None,
            payment_instructions: Vec::new(),
            lines: std::mem::take(&mut doc.lines),
            tax_summary: std::mem::take(&mut doc.tax_summary),
            monetary_total: doc.monetary_total.clone(),
            attachments: Vec::new(),
            references: Vec::new(),
            notes: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
            deliver_to: None,
            meta: DocumentMeta {
                tenant_id: "tenant_123".to_owned(),
                trace_id: "trace_abc".to_owned(),
                source_system: Some("inline".to_owned()),
            },
        };
        CommercialDocument::new(parts).unwrap()
    }

    #[test]
    fn inv01_emits_real_inter_state_field_names() {
        let json = to_inv01_json(&inter_state_invoice(), &Inv01Context::default()).unwrap();
        // Parse back and assert the REAL INV-01 keys + values.
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["Version"], "1.1");
        assert_eq!(v["TranDtls"]["TaxSch"], "GST");
        assert_eq!(v["TranDtls"]["SupTyp"], "B2B");
        assert_eq!(v["DocDtls"]["Typ"], "INV");
        assert_eq!(v["DocDtls"]["No"], "INV-2026-IN-0001");
        assert_eq!(v["DocDtls"]["Dt"], "26/05/2026");
        assert_eq!(v["SellerDtls"]["Gstin"], "29AAAPL2356Q1ZS");
        assert_eq!(v["SellerDtls"]["LglNm"], "Acme Technologies Pvt Ltd");
        assert_eq!(v["SellerDtls"]["Loc"], "Bengaluru");
        assert_eq!(v["SellerDtls"]["Pin"], 560_001);
        assert_eq!(v["SellerDtls"]["Stcd"], "29");
        // `Addr1` is the first IR address line; the seller carries no `Pos`.
        assert_eq!(v["SellerDtls"]["Addr1"], "1 MG Road");
        assert!(
            v["SellerDtls"].get("Pos").is_none(),
            "seller carries no Pos"
        );
        assert_eq!(v["BuyerDtls"]["Gstin"], "27AAAPL2356Q1ZT");
        assert_eq!(v["BuyerDtls"]["Stcd"], "27");
        assert_eq!(v["BuyerDtls"]["Addr1"], "1 MG Road");
        // `Pos` (place of supply) is the buyer's GST state code.
        assert_eq!(v["BuyerDtls"]["Pos"], "27");

        let item = &v["ItemList"][0];
        assert_eq!(item["SlNo"], "1");
        // `PrdDesc` maps the IR line description; `IsServc` is `Y` because the
        // resolved `HsnCd` (9983) is a chapter-99 SAC (service); `Unit` maps the
        // IR unit code `EA` onto the IRP UQC `PCS`.
        assert_eq!(item["PrdDesc"], "Software consulting & support");
        assert_eq!(item["IsServc"], "Y");
        assert_eq!(item["Unit"], "PCS");
        assert_eq!(item["HsnCd"], "9983");
        assert_eq!(item["Qty"], "2.000");
        assert_eq!(item["UnitPrice"], "5000.00");
        assert_eq!(item["TotAmt"], "10000.00");
        assert_eq!(item["AssAmt"], "10000.00");
        assert_eq!(item["GstRt"], "18.00");
        // Inter-state -> IGST at the full 18% (1800.00), no CGST/SGST keys.
        assert_eq!(item["IgstAmt"], "1800.00");
        assert!(
            item.get("CgstAmt").is_none(),
            "inter-state emits no CgstAmt"
        );
        assert!(
            item.get("SgstAmt").is_none(),
            "inter-state emits no SgstAmt"
        );
        assert_eq!(item["TotItemVal"], "11800.00");

        assert_eq!(v["ValDtls"]["AssVal"], "10000.00");
        assert_eq!(v["ValDtls"]["IgstVal"], "1800.00");
        assert_eq!(v["ValDtls"]["CgstVal"], "0.00");
        assert_eq!(v["ValDtls"]["SgstVal"], "0.00");
        assert_eq!(v["ValDtls"]["TotInvVal"], "11800.00");
    }

    #[test]
    fn inv01_hsn_code_comes_from_ir_classification() {
        use invoicekit_ir::ItemClassification;

        // A goods line classified under the HSN scheme: `HsnCd` is the
        // classification code (not the "9983" placeholder) and `IsServc` is "N"
        // because the scheme is HSN and the code (0901, coffee) is not chapter 99.
        let mut doc = inter_state_invoice();
        doc.lines[0].classifications = vec![ItemClassification {
            code: "0901".to_owned(),
            scheme_id: "HSN".to_owned(),
            scheme_version: Some("2017".to_owned()),
        }];
        let json = to_inv01_json(&doc, &Inv01Context::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let item = &v["ItemList"][0];
        assert_eq!(
            item["HsnCd"], "0901",
            "HsnCd sourced from the IR classification"
        );
        assert_eq!(
            item["IsServc"], "N",
            "HSN-scheme non-chapter-99 code is goods"
        );

        // A SAC-scheme classification flags a service even when its code does
        // not start with 99 — the flag derives from the chosen scheme.
        doc.lines[0].classifications = vec![ItemClassification {
            code: "00440406".to_owned(),
            scheme_id: "sac".to_owned(), // case-insensitive
            scheme_version: None,
        }];
        let json = to_inv01_json(&doc, &Inv01Context::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let item = &v["ItemList"][0];
        assert_eq!(item["HsnCd"], "00440406");
        assert_eq!(
            item["IsServc"], "Y",
            "SAC-scheme classification is a service"
        );

        // Preference: an HSN/SAC classification wins over an unrelated leading
        // entry, so `HsnCd` carries the HSN code, not the first (UNSPSC) one.
        doc.lines[0].classifications = vec![
            ItemClassification {
                code: "50161509".to_owned(),
                scheme_id: "UNSPSC".to_owned(),
                scheme_version: None,
            },
            ItemClassification {
                code: "0901".to_owned(),
                scheme_id: "HSN".to_owned(),
                scheme_version: None,
            },
        ];
        let json = to_inv01_json(&doc, &Inv01Context::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            v["ItemList"][0]["HsnCd"], "0901",
            "HSN/SAC scheme is preferred"
        );
    }

    #[test]
    fn inv01_unclassified_line_falls_back_to_placeholder_hsn() {
        // Behavior-preservation proof: a line with NO classification still emits
        // the legacy "9983" placeholder and `IsServc` "Y" (chapter-99 service),
        // byte-identical to the pre-classification behavior.
        let doc = inter_state_invoice();
        assert!(doc.lines[0].classifications.is_empty());
        let json = to_inv01_json(&doc, &Inv01Context::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["ItemList"][0]["HsnCd"], "9983");
        assert_eq!(v["ItemList"][0]["IsServc"], "Y");
    }

    #[test]
    fn unit_uqc_maps_un_ece_codes_and_defaults_to_oth() {
        // UN/ECE Rec 20 codes translate to the IRP UQC set.
        assert_eq!(unit_uqc(Some("EA")), "PCS");
        assert_eq!(unit_uqc(Some("C62")), "NOS");
        assert_eq!(unit_uqc(Some("KGM")), "KGS");
        assert_eq!(unit_uqc(Some("LTR")), "LTR");
        assert_eq!(unit_uqc(Some("MTK")), "SQM");
        // Already-UQC values pass through (case-insensitively).
        assert_eq!(unit_uqc(Some("box")), "BOX");
        // Absent / unrecognized -> the valid `OTH` ("Others") fallback.
        assert_eq!(unit_uqc(None), "OTH");
        assert_eq!(unit_uqc(Some("")), "OTH");
        assert_eq!(unit_uqc(Some("furlong")), "OTH");
    }

    #[test]
    fn inv01_intra_state_splits_into_cgst_and_sgst() {
        let json = to_inv01_json(&intra_state_invoice(), &Inv01Context::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let item = &v["ItemList"][0];
        // 18% on 10000 splits into 9% CGST (900.00) + 9% SGST (900.00), no IGST.
        assert_eq!(item["CgstAmt"], "900.00");
        assert_eq!(item["SgstAmt"], "900.00");
        assert!(
            item.get("IgstAmt").is_none(),
            "intra-state emits no IgstAmt"
        );
        assert_eq!(item["TotItemVal"], "11800.00");
        assert_eq!(v["ValDtls"]["CgstVal"], "900.00");
        assert_eq!(v["ValDtls"]["SgstVal"], "900.00");
        assert_eq!(v["ValDtls"]["IgstVal"], "0.00");
    }

    #[test]
    fn inv01_credit_note_maps_to_crn() {
        let mut doc = inter_state_invoice();
        // Cheap document-type swap for the doc-type mapping assertion.
        let parts = CommercialDocumentParts {
            schema_version: SchemaVersion::default(),
            id: DocumentId::new("doc-in-cn").unwrap(),
            document_type: DocumentType::CreditNote,
            issue_date: DateOnly::new("2026-05-26").unwrap(),
            tax_point_date: None,
            due_date: None,
            invoice_period: None,
            delivery_date: None,
            document_number: DocumentNumber::new("CRN-2026-IN-0001").unwrap(),
            currency: Iso4217Code::new("INR").unwrap(),
            supplier: doc.supplier.clone(),
            customer: doc.customer.clone(),
            payee: None,
            payment_terms: None,
            payment_instructions: Vec::new(),
            lines: std::mem::take(&mut doc.lines),
            tax_summary: std::mem::take(&mut doc.tax_summary),
            monetary_total: doc.monetary_total.clone(),
            attachments: Vec::new(),
            // A credit note refers back to the original invoice it corrects.
            references: vec![DocumentReference {
                kind: "original-invoice".to_owned(),
                id: "INV-2026-IN-0001".to_owned(),
                issue_date: Some(DateOnly::new("2026-04-15").unwrap()),
            }],
            notes: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
            deliver_to: None,
            meta: DocumentMeta {
                tenant_id: "tenant_123".to_owned(),
                trace_id: "trace_abc".to_owned(),
                source_system: Some("inline".to_owned()),
            },
        };
        let cn = CommercialDocument::new(parts).unwrap();
        let json = to_inv01_json(&cn, &Inv01Context::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["DocDtls"]["Typ"], "CRN");
        // RefDtls.PrecDocDtls carries the original-invoice link verbatim
        // (InvNo = id, InvDt = dd/mm/yyyy of the reference issue date).
        assert_eq!(v["RefDtls"]["PrecDocDtls"][0]["InvNo"], "INV-2026-IN-0001");
        assert_eq!(v["RefDtls"]["PrecDocDtls"][0]["InvDt"], "15/04/2026");

        // Behavior-preserving: an invoice with no references emits no RefDtls.
        let inv_json = to_inv01_json(&inter_state_invoice(), &Inv01Context::default()).unwrap();
        let iv: serde_json::Value = serde_json::from_str(&inv_json).unwrap();
        assert!(
            iv.get("RefDtls").is_none(),
            "no references must emit no RefDtls"
        );
    }

    #[test]
    fn inv01_export_buyer_without_gstin_is_urp_and_inter_state() {
        let mut doc = inter_state_invoice();
        // Foreign buyer with no GSTIN.
        doc.customer.tax_ids.clear();
        let ctx = Inv01Context {
            supply_type: "EXPWOP".to_owned(),
        };
        let json = to_inv01_json(&doc, &ctx).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["TranDtls"]["SupTyp"], "EXPWOP");
        assert_eq!(v["BuyerDtls"]["Gstin"], "URP");
        // No buyer GSTIN -> treated as inter-state, so IGST is charged.
        assert!(v["ItemList"][0].get("IgstAmt").is_some());
    }

    #[test]
    fn inv01_rejects_unsupported_document_type() {
        let err = doc_type_code(DocumentType::ProForma).unwrap_err();
        assert!(matches!(err, Inv01Error::UnsupportedDocumentType(_)));
    }

    #[test]
    fn inv01_is_deterministic() {
        let doc = inter_state_invoice();
        let ctx = Inv01Context::default();
        assert_eq!(
            to_inv01_json(&doc, &ctx).unwrap(),
            to_inv01_json(&doc, &ctx).unwrap(),
            "INV-01 serialization must be byte-stable"
        );
    }

    #[test]
    fn inv01_key_order_is_fixed() {
        // Determinism is byte-stable key order; assert the top-level key order
        // matches the INV-01 schema sequence (no map-driven reordering).
        let json = to_inv01_json(&inter_state_invoice(), &Inv01Context::default()).unwrap();
        let order = [
            "Version",
            "TranDtls",
            "DocDtls",
            "SellerDtls",
            "BuyerDtls",
            "ItemList",
            "ValDtls",
        ];
        let mut last = 0;
        for key in order {
            let needle = format!("\"{key}\"");
            let at = json
                .find(&needle)
                .expect("INV-01 emits every top-level key in order");
            assert!(at >= last, "key {key} out of INV-01 order");
            last = at;
        }
    }

    /// Regression: a GSTIN whose leading bytes are a multibyte UTF-8 character
    /// must not panic the INV-01 serializer. Both `to_inv01_json` (the
    /// intra-state state-code comparison) and `state_code` slice the GSTIN at
    /// byte index 2; with a byte-length-only guard that index can fall inside a
    /// multibyte character and panic. A real GSTIN is ASCII alphanumeric, so the
    /// state-code prefix is simply unavailable for such junk input — the
    /// serializer falls back to the address subdivision rather than crashing.
    #[test]
    fn inv01_multibyte_gstin_does_not_panic_and_falls_back_to_subdivision() {
        let mut doc = inter_state_invoice();
        // U+20B9 (₹) is a 3-byte character: byte index 2 lands inside it.
        doc.supplier.tax_ids[0].value = "\u{20b9}9AAAPL2356Q1ZS".to_owned();
        doc.customer.tax_ids[0].value = "\u{20b9}7AAAPL2356Q1ZT".to_owned();
        let json = to_inv01_json(&doc, &Inv01Context::default())
            .expect("multibyte GSTIN must serialize, not panic");
        let v: serde_json::Value =
            serde_json::from_str(&json).expect("serializer emits valid JSON");
        // No ASCII two-char prefix available -> Stcd falls back to the party's
        // address subdivision ("KA" supplier / "MH" buyer), not a panic.
        assert_eq!(v["SellerDtls"]["Stcd"], "KA");
        assert_eq!(v["BuyerDtls"]["Stcd"], "MH");
    }

    /// Build a `DecimalValue` straight from a raw `Decimal` (no scale-2 minor
    /// coercion) so tests can place untrusted amounts near `Decimal::MAX`.
    fn raw(value: Decimal) -> DecimalValue {
        DecimalValue::new(value)
    }

    /// Regression for the accumulation loop in `to_inv01_json`: summing
    /// per-line `line_extension_amount` values that individually fit but
    /// jointly exceed `Decimal::MAX` must surface a typed error, not panic on
    /// the `+=` accumulator. Before the `checked_add` fix this panicked
    /// ("attempt to add with overflow" inside `rust_decimal`).
    #[test]
    fn inv01_assessable_total_overflow_is_error_not_panic() {
        let mut doc = inter_state_invoice();
        // Two lines, each at the maximum representable amount: their sum
        // overflows the accumulator. Both lines carry the zero-rate "Z"
        // category so the tax multiply at site 2 stays in range and we
        // isolate the accumulation overflow.
        let line = DocumentLine {
            id: "1".to_owned(),
            description: "Huge line".to_owned(),
            quantity: DecimalValue::new(Decimal::from(1)),
            unit_code: Some("EA".to_owned()),
            unit_price: raw(Decimal::MAX),
            line_extension_amount: raw(Decimal::MAX),
            tax_category: None,
            classifications: Vec::new(),
            extensions: Vec::new(),
            allowance_charges: Vec::new(),
        };
        doc.lines = vec![line.clone(), line];
        doc.tax_summary.clear();

        let err = to_inv01_json(&doc, &Inv01Context::default())
            .expect_err("two Decimal::MAX line amounts must overflow the assessable total");
        assert!(
            matches!(err, Inv01Error::BadContext(_)),
            "overflow must surface as a typed Inv01Error, got {err:?}"
        );
    }

    /// Regression for the per-line tax computation in `line_tax_amounts`:
    /// `base * rate` on untrusted amounts can exceed `Decimal::MAX`. With a
    /// near-`MAX` base and a non-trivial rate the multiply overflows and must
    /// surface a typed error rather than panic. Before the `checked_mul` fix
    /// this panicked ("attempt to multiply with overflow").
    #[test]
    fn inv01_line_tax_multiply_overflow_is_error_not_panic() {
        let mut doc = inter_state_invoice();
        doc.lines[0].line_extension_amount = raw(Decimal::MAX);
        doc.lines[0].tax_category = Some("S".to_owned());
        // An 18% rate against a Decimal::MAX base overflows `base * rate`.
        doc.tax_summary = vec![TaxCategorySummary {
            category_code: "S".to_owned(),
            taxable_amount: raw(Decimal::MAX),
            tax_amount: amt(0),
            tax_rate: Some(DecimalValue::new(Decimal::new(1800, 2))),
            exemption_reason: None,
            exemption_reason_code: None,
        }];

        let err = to_inv01_json(&doc, &Inv01Context::default())
            .expect_err("Decimal::MAX base times an 18% rate must overflow the tax multiply");
        assert!(
            matches!(err, Inv01Error::BadContext(_)),
            "overflow must surface as a typed Inv01Error, got {err:?}"
        );
    }
}
