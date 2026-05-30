// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-013 codec: GOBL JSON <-> [`CommercialDocument`].
//!
//! `clippy::too_many_lines` is allowed at module scope because the
//! happy-path codec inherently enumerates the GOBL schema in one
//! function each direction; breaking it up into named helpers would
//! obscure the field-by-field correspondence reviewers care about.
//! `clippy::or_fun_call` is allowed for the same reason — the
//! many `ok_or(GoblError::MissingField { path: "/x".into() })?`
//! chains are clearer than the equivalent `ok_or_else(|| ...)`.

#![allow(clippy::too_many_lines, clippy::or_fun_call)]

use std::str::FromStr;

use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, Contact, CountryCode, DateOnly, DecimalValue,
    DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentReference, DocumentType,
    IrError, Iso4217Code, JurisdictionExtension, LocalizedString, LossinessEntry, LossinessLedger,
    MonetaryTotal, Party, PartyTaxId, PaymentInstruction, PaymentInstructionKind, PaymentTerms,
    PostalAddress, TaxCategorySummary,
};
use rust_decimal::Decimal;
use serde_json::{json, Map, Value};
use thiserror::Error;

use crate::GOBL_BILL_SCHEMA_PREFIX;

/// Bundle returned by [`to_gobl`] and [`from_gobl`]: the projected
/// document plus a [`LossinessLedger`] explaining what didn't survive.
#[derive(Clone, Debug)]
pub struct GoblEnvelope {
    /// The GOBL JSON payload (root object) on the forward path, or the
    /// reconstructed IR document JSON on the reverse path.
    pub document: Value,
    /// Per-call lossiness report.
    pub ledger: LossinessLedger,
}

/// Errors raised by [`to_gobl`] / [`from_gobl`].
#[derive(Debug, Error)]
pub enum GoblError {
    /// The GOBL JSON wasn't an object at the root.
    #[error("GOBL document must be a JSON object at the root")]
    NotAnObject,
    /// A required GOBL field is missing.
    #[error("GOBL document missing required field {path}")]
    MissingField {
        /// JSON-Pointer-shaped path to the missing field.
        path: String,
    },
    /// A GOBL field has an unsupported value (out-of-enum, wrong type).
    #[error("GOBL document field {path} has unsupported value: {detail}")]
    BadValue {
        /// JSON-Pointer-shaped path.
        path: String,
        /// Operator-readable reason.
        detail: String,
    },
    /// The produced IR didn't pass [`CommercialDocument::validate`].
    #[error("produced CommercialDocument failed IR validation: {0}")]
    Ir(#[from] IrError),
    /// A decimal string did not parse.
    #[error("invalid decimal value at {path}: {value:?}")]
    BadDecimal {
        /// JSON pointer to the offending field.
        path: String,
        /// Offending value.
        value: String,
    },
}

/// Project an IR [`CommercialDocument`] into GOBL JSON.
///
/// # Errors
///
/// Returns [`GoblError::Ir`] when the input IR fails its own
/// validation. Otherwise infallible: any IR field that doesn't have a
/// GOBL home is recorded in `envelope.ledger.lost`.
pub fn to_gobl(doc: &CommercialDocument) -> Result<GoblEnvelope, GoblError> {
    doc.validate()?;
    let mut lost = Vec::new();
    let mut payload = Map::new();
    let type_str = gobl_type_string(doc.document_type);

    payload.insert(
        "$schema".into(),
        Value::String(format!("{GOBL_BILL_SCHEMA_PREFIX}{type_str}")),
    );
    payload.insert("type".into(), Value::String(type_str.to_owned()));
    payload.insert("id".into(), Value::String(doc.id.as_str().to_owned()));
    payload.insert(
        "code".into(),
        Value::String(doc.document_number.as_str().to_owned()),
    );
    payload.insert(
        "issue_date".into(),
        Value::String(doc.issue_date.as_str().to_owned()),
    );
    if let Some(d) = &doc.due_date {
        payload.insert("due_date".into(), Value::String(d.as_str().to_owned()));
    }
    if let Some(d) = &doc.tax_point_date {
        payload.insert("tax_date".into(), Value::String(d.as_str().to_owned()));
    }
    payload.insert(
        "currency".into(),
        Value::String(transparent_str(&doc.currency)),
    );

    payload.insert("supplier".into(), party_to_gobl(&doc.supplier));
    payload.insert("customer".into(), party_to_gobl(&doc.customer));
    if let Some(payee) = &doc.payee {
        lost.push(LossinessEntry {
            path: "/payee".into(),
            reason: format!(
                "GOBL has no first-class payee distinct from supplier; emitted as auxiliary `payee` (name {})",
                payee.name
            ),
        });
        payload.insert("payee".into(), party_to_gobl(payee));
    }

    let mut payment = Map::new();
    if let Some(terms) = &doc.payment_terms {
        let mut t = Map::new();
        t.insert(
            "description".into(),
            Value::String(terms.description.clone()),
        );
        if let Some(d) = &terms.due_date {
            t.insert("due_date".into(), Value::String(d.as_str().to_owned()));
        }
        payment.insert("terms".into(), Value::Object(t));
    }
    if !doc.payment_instructions.is_empty() {
        let arr: Vec<Value> = doc
            .payment_instructions
            .iter()
            .map(|p| {
                json!({
                    "kind": payment_kind_to_gobl(p.kind),
                    "account": p.account.as_deref().unwrap_or(""),
                    "reference": p.reference.as_deref().unwrap_or(""),
                })
            })
            .collect();
        payment.insert("instructions".into(), Value::Array(arr));
    }
    if !payment.is_empty() {
        payload.insert("payment".into(), Value::Object(payment));
    }

    payload.insert(
        "lines".into(),
        Value::Array(
            doc.lines
                .iter()
                .enumerate()
                .map(|(idx, line)| line_to_gobl(idx, line))
                .collect(),
        ),
    );

    let mut totals = Map::new();
    totals.insert(
        "sum".into(),
        Value::String(decimal_str(&doc.monetary_total.line_extension_amount)),
    );
    totals.insert(
        "total".into(),
        Value::String(decimal_str(&doc.monetary_total.tax_exclusive_amount)),
    );
    totals.insert(
        "total_with_tax".into(),
        Value::String(decimal_str(&doc.monetary_total.tax_inclusive_amount)),
    );
    totals.insert(
        "payable".into(),
        Value::String(decimal_str(&doc.monetary_total.payable_amount)),
    );
    if !doc.tax_summary.is_empty() {
        totals.insert("tax".into(), tax_summary_to_gobl(&doc.tax_summary));
    }
    payload.insert("totals".into(), Value::Object(totals));

    if !doc.references.is_empty() {
        payload.insert(
            "preceding".into(),
            Value::Array(doc.references.iter().map(reference_to_gobl).collect()),
        );
    }
    if !doc.notes.is_empty() {
        payload.insert(
            "notes".into(),
            Value::Array(
                doc.notes
                    .iter()
                    .map(|n| json!({"text": n.text, "lang": n.language}))
                    .collect(),
            ),
        );
    }
    if !doc.attachments.is_empty() {
        payload.insert(
            "attachments".into(),
            Value::Array(
                doc.attachments
                    .iter()
                    .map(|a| {
                        json!({
                            "kind": a.kind,
                            "digest": a.digest,
                            "media_type": a.media_type,
                        })
                    })
                    .collect(),
            ),
        );
    }
    if !doc.extensions.is_empty() {
        let mut ext = Map::new();
        for e in &doc.extensions {
            ext.insert(e.urn.clone(), e.payload.clone());
        }
        payload.insert("ext".into(), Value::Object(ext));
    }

    payload.insert(
        "meta".into(),
        json!({
            "tenant_id": doc.meta.tenant_id,
            "trace_id": doc.meta.trace_id,
            "source_system": doc.meta.source_system,
        }),
    );

    Ok(GoblEnvelope {
        document: Value::Object(payload),
        ledger: LossinessLedger {
            lost,
            ..Default::default()
        },
    })
}

/// Parse a GOBL JSON object back into an IR [`CommercialDocument`].
///
/// The returned [`GoblEnvelope::document`] is the serialized IR JSON
/// (not the GOBL payload) so callers can pipe it straight into
/// [`CommercialDocument::try_from_value`] if needed.
///
/// # Errors
///
/// Returns [`GoblError::NotAnObject`] / [`GoblError::MissingField`] /
/// [`GoblError::BadValue`] for shape errors, [`GoblError::BadDecimal`]
/// when an amount string isn't parseable, and [`GoblError::Ir`] when
/// the parsed-out IR fails its own validation.
pub fn from_gobl(payload: &Value) -> Result<GoblEnvelope, GoblError> {
    let obj = payload.as_object().ok_or(GoblError::NotAnObject)?;
    let mut lost = Vec::new();

    let document_type = match obj.get("type").and_then(Value::as_str) {
        Some(t) => parse_gobl_type(t)?,
        None => DocumentType::Invoice,
    };
    let id_value = obj
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| obj.get("code").and_then(Value::as_str))
        .ok_or(GoblError::MissingField { path: "/id".into() })?
        .to_owned();
    let document_number = obj
        .get("code")
        .and_then(Value::as_str)
        .ok_or(GoblError::MissingField {
            path: "/code".into(),
        })?
        .to_owned();
    let issue_date = required_str(obj, "issue_date")?;
    let currency = required_str(obj, "currency")?;
    let supplier = party_from_gobl(
        obj.get("supplier").ok_or(GoblError::MissingField {
            path: "/supplier".into(),
        })?,
        "/supplier",
    )?;
    let customer = party_from_gobl(
        obj.get("customer").ok_or(GoblError::MissingField {
            path: "/customer".into(),
        })?,
        "/customer",
    )?;
    let payee = match obj.get("payee") {
        Some(v) => Some(party_from_gobl(v, "/payee")?),
        None => None,
    };

    let lines_array =
        obj.get("lines")
            .and_then(Value::as_array)
            .ok_or(GoblError::MissingField {
                path: "/lines".into(),
            })?;
    let mut lines = Vec::with_capacity(lines_array.len());
    for (idx, raw) in lines_array.iter().enumerate() {
        lines.push(line_from_gobl(raw, idx)?);
    }

    let totals = obj.get("totals").and_then(Value::as_object);
    let total_decimal = |key: &str, path: &str| {
        decimal_from(
            totals.and_then(|t| t.get(key)).and_then(Value::as_str),
            path,
            "0",
        )
    };
    let line_extension_amount = total_decimal("sum", "/totals/sum")?;
    let tax_exclusive_amount = total_decimal("total", "/totals/total")?;
    let tax_inclusive_amount = total_decimal("total_with_tax", "/totals/total_with_tax")?;
    let payable_amount = total_decimal("payable", "/totals/payable")?;

    let tax_summary = totals
        .and_then(|t| t.get("tax"))
        .map(tax_summary_from_gobl)
        .unwrap_or_default();

    let references = obj
        .get("preceding")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(reference_from_gobl).collect())
        .unwrap_or_default();

    let extensions = extensions_from_gobl(obj);

    let notes = obj
        .get("notes")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|n| {
                    let text = n.get("text")?.as_str()?.to_owned();
                    let lang = n
                        .get("lang")
                        .and_then(Value::as_str)
                        .unwrap_or("en")
                        .to_owned();
                    Some(LocalizedString {
                        language: lang,
                        text,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let meta_obj = obj.get("meta").and_then(Value::as_object);
    let tenant_id = meta_obj
        .and_then(|m| m.get("tenant_id"))
        .and_then(Value::as_str)
        .unwrap_or("gobl-import")
        .to_owned();
    let trace_id = meta_obj
        .and_then(|m| m.get("trace_id"))
        .and_then(Value::as_str)
        .unwrap_or("gobl-import-trace")
        .to_owned();
    if meta_obj.is_none() {
        lost.push(LossinessEntry {
            path: "/meta".into(),
            reason: "GOBL document carried no meta; synthesized placeholder tenant/trace IDs"
                .into(),
        });
    }
    let source_system = meta_obj
        .and_then(|m| m.get("source_system"))
        .and_then(Value::as_str)
        .map(str::to_owned);

    let payment = obj.get("payment").and_then(Value::as_object);
    let payment_terms = payment
        .and_then(|p| p.get("terms"))
        .and_then(Value::as_object)
        .and_then(|t| {
            let desc = t.get("description")?.as_str()?.to_owned();
            let due_date = t
                .get("due_date")
                .and_then(Value::as_str)
                .and_then(|d| DateOnly::new(d.to_owned()).ok());
            Some(PaymentTerms {
                description: desc,
                due_date,
            })
        });
    let payment_instructions = payment
        .and_then(|p| p.get("instructions"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    let kind = p.get("kind").and_then(Value::as_str)?;
                    Some(PaymentInstruction {
                        kind: parse_payment_kind(kind),
                        account: p
                            .get("account")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned),
                        reference: p
                            .get("reference")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let attachments = obj
        .get("attachments")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    Some(invoicekit_ir::Attachment {
                        kind: a.get("kind")?.as_str()?.to_owned(),
                        digest: a.get("digest")?.as_str()?.to_owned(),
                        media_type: a.get("media_type")?.as_str()?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let parts = CommercialDocumentParts {
        schema_version: invoicekit_ir::SchemaVersion::default(),
        id: DocumentId::new(id_value)?,
        document_type,
        issue_date: DateOnly::new(issue_date)?,
        tax_point_date: obj
            .get("tax_date")
            .and_then(Value::as_str)
            .and_then(|d| DateOnly::new(d.to_owned()).ok()),
        due_date: obj
            .get("due_date")
            .and_then(Value::as_str)
            .and_then(|d| DateOnly::new(d.to_owned()).ok()),
        document_number: DocumentNumber::new(document_number)?,
        currency: Iso4217Code::new(currency)?,
        supplier,
        customer,
        payee,
        payment_terms,
        payment_instructions,
        lines,
        tax_summary,
        monetary_total: MonetaryTotal {
            line_extension_amount,
            tax_exclusive_amount,
            tax_inclusive_amount,
            allowance_total_amount: None,
            charge_total_amount: None,
            prepaid_amount: None,
            payable_amount,
        },
        attachments,
        references,
        notes,
        extensions,
        meta: DocumentMeta {
            tenant_id,
            trace_id,
            source_system,
        },
    };
    let doc = CommercialDocument::new(parts)?;
    Ok(GoblEnvelope {
        document: doc.to_value()?,
        ledger: LossinessLedger {
            lost,
            ..Default::default()
        },
    })
}

fn party_to_gobl(party: &Party) -> Value {
    let mut p = Map::new();
    if let Some(id) = &party.id {
        p.insert("id".into(), Value::String(id.clone()));
    }
    p.insert("name".into(), Value::String(party.name.clone()));
    if let Some(tax) = party.tax_ids.first() {
        p.insert(
            "tax_id".into(),
            json!({
                "country": transparent_str(&party.address.country),
                "code": tax.value,
                "scheme": tax.scheme,
            }),
        );
    }
    p.insert(
        "addresses".into(),
        Value::Array(vec![json!({
            "street": party.address.lines.first().cloned().unwrap_or_default(),
            "locality": party.address.city,
            "region": party.address.subdivision,
            "code": party.address.postal_code,
            "country": transparent_str(&party.address.country),
        })]),
    );
    if let Some(c) = &party.contact {
        if let Some(email) = &c.email {
            p.insert("emails".into(), Value::Array(vec![json!({"addr": email})]));
        }
        if let Some(phone) = &c.phone {
            p.insert(
                "telephones".into(),
                Value::Array(vec![json!({"num": phone})]),
            );
        }
        if let Some(name) = &c.name {
            p.insert("people".into(), Value::Array(vec![json!({"name": name})]));
        }
    }
    Value::Object(p)
}

fn party_from_gobl(value: &Value, path_prefix: &str) -> Result<Party, GoblError> {
    let obj = value.as_object().ok_or(GoblError::BadValue {
        path: path_prefix.into(),
        detail: "expected an object".into(),
    })?;
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .ok_or(GoblError::MissingField {
            path: format!("{path_prefix}/name"),
        })?
        .to_owned();
    let tax_ids = obj
        .get("tax_id")
        .and_then(Value::as_object)
        .map(|t| {
            vec![PartyTaxId {
                scheme: t
                    .get("scheme")
                    .and_then(Value::as_str)
                    .unwrap_or("vat")
                    .to_owned(),
                value: t
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }]
        })
        .unwrap_or_default();
    let addrs = obj.get("addresses").and_then(Value::as_array);
    let addr_obj = addrs.and_then(|a| a.first()).and_then(Value::as_object);
    let country = addr_obj
        .and_then(|a| a.get("country"))
        .and_then(Value::as_str)
        .or_else(|| {
            obj.get("tax_id")
                .and_then(Value::as_object)
                .and_then(|t| t.get("country"))
                .and_then(Value::as_str)
        })
        .ok_or(GoblError::MissingField {
            path: format!("{path_prefix}/addresses/0/country"),
        })?
        .to_owned();
    let address = PostalAddress {
        lines: addr_obj
            .and_then(|a| a.get("street"))
            .and_then(Value::as_str)
            .map_or_else(|| vec!["unknown".to_owned()], |s| vec![s.to_owned()]),
        city: addr_obj
            .and_then(|a| a.get("locality"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
        subdivision: addr_obj
            .and_then(|a| a.get("region"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        postal_code: addr_obj
            .and_then(|a| a.get("code"))
            .and_then(Value::as_str)
            .unwrap_or("00000")
            .to_owned(),
        country: CountryCode::new(country)?,
    };
    let contact = {
        let email = obj
            .get("emails")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|e| e.get("addr"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let phone = obj
            .get("telephones")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|t| t.get("num"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let name = obj
            .get("people")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if email.is_some() || phone.is_some() || name.is_some() {
            Some(Contact { name, email, phone })
        } else {
            None
        }
    };
    Ok(Party {
        id: obj.get("id").and_then(Value::as_str).map(str::to_owned),
        name,
        tax_ids,
        address,
        contact,
    })
}

fn line_to_gobl(idx: usize, line: &DocumentLine) -> Value {
    let mut item = Map::new();
    item.insert("name".into(), Value::String(line.description.clone()));
    item.insert("price".into(), Value::String(decimal_str(&line.unit_price)));
    if let Some(u) = &line.unit_code {
        item.insert("unit".into(), Value::String(u.clone()));
    }
    let mut out = Map::new();
    out.insert("i".into(), Value::Number((idx + 1).into()));
    out.insert("code".into(), Value::String(line.id.clone()));
    out.insert(
        "quantity".into(),
        Value::String(decimal_str(&line.quantity)),
    );
    out.insert("item".into(), Value::Object(item));
    out.insert(
        "sum".into(),
        Value::String(decimal_str(&line.line_extension_amount)),
    );
    if let Some(c) = &line.tax_category {
        out.insert(
            "taxes".into(),
            Value::Array(vec![json!({"cat": "VAT", "rate": c})]),
        );
    }
    if !line.extensions.is_empty() {
        let mut ext = Map::new();
        for e in &line.extensions {
            ext.insert(e.urn.clone(), e.payload.clone());
        }
        out.insert("ext".into(), Value::Object(ext));
    }
    Value::Object(out)
}

fn line_from_gobl(value: &Value, idx: usize) -> Result<DocumentLine, GoblError> {
    let obj = value.as_object().ok_or(GoblError::BadValue {
        path: format!("/lines/{idx}"),
        detail: "expected an object".into(),
    })?;
    let item = obj.get("item").and_then(Value::as_object);
    let description = item
        .and_then(|i| i.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let unit_price = decimal_from(
        item.and_then(|i| i.get("price")).and_then(Value::as_str),
        &format!("/lines/{idx}/item/price"),
        "0",
    )?;
    let unit_code = item
        .and_then(|i| i.get("unit"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let quantity = decimal_from(
        obj.get("quantity").and_then(Value::as_str),
        &format!("/lines/{idx}/quantity"),
        "1",
    )?;
    let sum = decimal_from(
        obj.get("sum").and_then(Value::as_str),
        &format!("/lines/{idx}/sum"),
        "0",
    )?;
    let id = obj
        .get("code")
        .and_then(Value::as_str)
        .map_or_else(|| format!("LINE-{n}", n = idx + 1), str::to_owned);
    let tax_category = obj
        .get("taxes")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|t| t.get("rate"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let extensions = extensions_from_gobl(obj);
    Ok(DocumentLine {
        id,
        description,
        quantity,
        unit_code,
        unit_price,
        line_extension_amount: sum,
        tax_category,
        classifications: Vec::new(),
        extensions,
    })
}

fn tax_summary_to_gobl(summary: &[TaxCategorySummary]) -> Value {
    let categories: Vec<Value> = summary
        .iter()
        .map(|s| {
            json!({
                "code": s.category_code,
                "rates": [{
                    "key": "standard",
                    "amount": decimal_str(&s.tax_amount),
                    "base": decimal_str(&s.taxable_amount),
                    "percent": s.tax_rate.as_ref().map(decimal_str).unwrap_or_default(),
                }],
            })
        })
        .collect();
    // checked_add via try_fold: summing many bounded tax amounts can still
    // exceed Decimal::MAX. On overflow, emit "0" (this codec's default-when-
    // unrepresentable convention) rather than panicking the adapter. Normal
    // values are unaffected.
    let sum = summary
        .iter()
        .try_fold(Decimal::ZERO, |acc, s| acc.checked_add(s.tax_amount.inner()))
        .unwrap_or(Decimal::ZERO);
    json!({
        "categories": categories,
        "sum": sum.to_string(),
    })
}

fn tax_summary_from_gobl(value: &Value) -> Vec<TaxCategorySummary> {
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    let Some(cats) = obj.get("categories").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for c in cats {
        let Some(code) = c.get("code").and_then(Value::as_str) else {
            continue;
        };
        let Some(rate) = c
            .get("rates")
            .and_then(Value::as_array)
            .and_then(|r| r.first())
        else {
            continue;
        };
        let taxable = rate
            .get("base")
            .and_then(Value::as_str)
            .and_then(|s| Decimal::from_str(s).ok())
            .map(DecimalValue::new);
        let amount = rate
            .get("amount")
            .and_then(Value::as_str)
            .and_then(|s| Decimal::from_str(s).ok())
            .map(DecimalValue::new);
        let percent = rate
            .get("percent")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .and_then(|s| Decimal::from_str(s).ok())
            .map(DecimalValue::new);
        if let (Some(t), Some(a)) = (taxable, amount) {
            out.push(TaxCategorySummary {
                category_code: code.to_owned(),
                taxable_amount: t,
                tax_amount: a,
                tax_rate: percent,
                exemption_reason: None,
                exemption_reason_code: None,
            });
        }
    }
    out
}

fn reference_to_gobl(r: &DocumentReference) -> Value {
    let mut o = Map::new();
    o.insert("type".into(), Value::String(r.kind.clone()));
    o.insert("code".into(), Value::String(r.id.clone()));
    if let Some(d) = &r.issue_date {
        o.insert("issue_date".into(), Value::String(d.as_str().to_owned()));
    }
    Value::Object(o)
}

fn reference_from_gobl(value: &Value) -> Option<DocumentReference> {
    let obj = value.as_object()?;
    Some(DocumentReference {
        kind: obj
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("preceding")
            .to_owned(),
        id: obj.get("code").and_then(Value::as_str)?.to_owned(),
        issue_date: obj
            .get("issue_date")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .and_then(|d| DateOnly::new(d.to_owned()).ok()),
    })
}

fn gobl_type_string(t: DocumentType) -> &'static str {
    match t {
        DocumentType::Invoice => "invoice",
        DocumentType::CreditNote => "credit-note",
        DocumentType::DebitNote => "debit-note",
        DocumentType::ProForma => "proforma",
        DocumentType::SelfBilled => "self-billed",
    }
}

fn parse_gobl_type(s: &str) -> Result<DocumentType, GoblError> {
    match s {
        "invoice" | "standard" => Ok(DocumentType::Invoice),
        "credit-note" | "credit_note" => Ok(DocumentType::CreditNote),
        "debit-note" | "debit_note" => Ok(DocumentType::DebitNote),
        "proforma" | "pro-forma" => Ok(DocumentType::ProForma),
        "self-billed" | "self_billed" => Ok(DocumentType::SelfBilled),
        other => Err(GoblError::BadValue {
            path: "/type".into(),
            detail: format!("unknown GOBL invoice type {other:?}"),
        }),
    }
}

fn payment_kind_to_gobl(kind: PaymentInstructionKind) -> &'static str {
    match kind {
        PaymentInstructionKind::Sepa => "sepa",
        PaymentInstructionKind::IbanBic => "iban_bic",
        PaymentInstructionKind::SwissQr => "swiss_qr",
        PaymentInstructionKind::EpcQr => "epc_qr",
        PaymentInstructionKind::ZatcaQr => "zatca_qr",
        PaymentInstructionKind::Other => "other",
    }
}

fn parse_payment_kind(s: &str) -> PaymentInstructionKind {
    match s {
        "sepa" => PaymentInstructionKind::Sepa,
        "iban_bic" | "iban-bic" | "credit-transfer" => PaymentInstructionKind::IbanBic,
        "swiss_qr" | "swiss-qr" => PaymentInstructionKind::SwissQr,
        "epc_qr" | "epc-qr" => PaymentInstructionKind::EpcQr,
        "zatca_qr" | "zatca-qr" => PaymentInstructionKind::ZatcaQr,
        _ => PaymentInstructionKind::Other,
    }
}

fn required_str(obj: &Map<String, Value>, key: &str) -> Result<String, GoblError> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(GoblError::MissingField {
            path: format!("/{key}"),
        })
}

fn decimal_str(v: &DecimalValue) -> String {
    v.inner().to_string()
}

fn decimal_from(
    maybe: Option<&str>,
    path: &str,
    default_if_missing: &str,
) -> Result<DecimalValue, GoblError> {
    let raw = maybe.unwrap_or(default_if_missing);
    Decimal::from_str(raw)
        .map(DecimalValue::new)
        .map_err(|_| GoblError::BadDecimal {
            path: path.to_owned(),
            value: raw.to_owned(),
        })
}

/// Parse a GOBL `ext` URN->payload map (read from `parent["ext"]`) into
/// the IR jurisdiction-extension list, dropping entries the IR rejects.
fn extensions_from_gobl(parent: &Map<String, Value>) -> Vec<JurisdictionExtension> {
    parent
        .get("ext")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(urn, payload)| {
                    JurisdictionExtension::new(urn.clone(), payload.clone()).ok()
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Serialize a `#[serde(transparent)]` newtype that wraps a String into
/// that String, so we don't depend on each type carrying an `as_str`.
fn transparent_str<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|val| val.as_str().map(str::to_owned))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dv(s: &str) -> DecimalValue {
        DecimalValue::new(Decimal::from_str(s).unwrap())
    }

    fn sample_doc() -> CommercialDocument {
        let parts = CommercialDocumentParts {
            schema_version: invoicekit_ir::SchemaVersion::default(),
            id: DocumentId::new("doc-001").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-27").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-27").unwrap()),
            document_number: DocumentNumber::new("F-2026-001").unwrap(),
            currency: Iso4217Code::new("EUR").unwrap(),
            supplier: Party {
                id: Some("sup-1".into()),
                name: "Acme Corp".into(),
                tax_ids: vec![PartyTaxId {
                    scheme: "vat".into(),
                    value: "B12345678".into(),
                }],
                address: PostalAddress {
                    lines: vec!["Calle Mayor 1".into()],
                    city: "Madrid".into(),
                    subdivision: Some("M".into()),
                    postal_code: "28013".into(),
                    country: CountryCode::new("ES").unwrap(),
                },
                contact: Some(Contact {
                    name: Some("Ana".into()),
                    email: Some("billing@acme.example".into()),
                    phone: Some("+34911234567".into()),
                }),
            },
            customer: Party {
                id: None,
                name: "Widget Buyer SARL".into(),
                tax_ids: vec![PartyTaxId {
                    scheme: "vat".into(),
                    value: "FR12345678901".into(),
                }],
                address: PostalAddress {
                    lines: vec!["12 rue de la Paix".into()],
                    city: "Paris".into(),
                    subdivision: None,
                    postal_code: "75001".into(),
                    country: CountryCode::new("FR").unwrap(),
                },
                contact: None,
            },
            payee: None,
            payment_terms: None,
            payment_instructions: vec![PaymentInstruction {
                kind: PaymentInstructionKind::IbanBic,
                account: Some("ES1234567890".into()),
                reference: Some("F-2026-001".into()),
            }],
            lines: vec![DocumentLine {
                id: "L1".into(),
                description: "Widget".into(),
                quantity: dv("2"),
                unit_code: Some("EA".into()),
                unit_price: dv("100.00"),
                line_extension_amount: dv("200.00"),
                tax_category: Some("standard".into()),
                classifications: Vec::new(),
                extensions: vec![],
            }],
            tax_summary: vec![TaxCategorySummary {
                category_code: "VAT".into(),
                taxable_amount: dv("200.00"),
                tax_amount: dv("42.00"),
                tax_rate: Some(dv("21.00")),
                exemption_reason: None,
                exemption_reason_code: None,
            }],
            monetary_total: MonetaryTotal {
                line_extension_amount: dv("200.00"),
                tax_exclusive_amount: dv("200.00"),
                tax_inclusive_amount: dv("242.00"),
                allowance_total_amount: None,
                charge_total_amount: None,
                prepaid_amount: None,
                payable_amount: dv("242.00"),
            },
            attachments: vec![],
            references: vec![],
            notes: vec![],
            extensions: vec![],
            meta: DocumentMeta {
                tenant_id: "tenant-x".into(),
                trace_id: "trace-001".into(),
                source_system: Some("test".into()),
            },
        };
        CommercialDocument::new(parts).unwrap()
    }

    #[test]
    fn to_gobl_emits_schema_and_core_fields() {
        let env = to_gobl(&sample_doc()).unwrap();
        let obj = env.document.as_object().unwrap();
        assert_eq!(
            obj.get("$schema").and_then(Value::as_str),
            Some("https://gobl.org/draft-0/bill/invoice")
        );
        assert_eq!(obj.get("type").and_then(Value::as_str), Some("invoice"));
        assert_eq!(obj.get("currency").and_then(Value::as_str), Some("EUR"));
        assert_eq!(obj.get("code").and_then(Value::as_str), Some("F-2026-001"));
        assert_eq!(
            obj.get("issue_date").and_then(Value::as_str),
            Some("2026-05-27")
        );
    }

    #[test]
    fn to_gobl_maps_supplier_tax_id_and_address() {
        let env = to_gobl(&sample_doc()).unwrap();
        let sup = env.document.get("supplier").unwrap();
        assert_eq!(sup.get("name").and_then(Value::as_str), Some("Acme Corp"));
        let tax = sup.get("tax_id").unwrap();
        assert_eq!(tax.get("country").and_then(Value::as_str), Some("ES"));
        assert_eq!(tax.get("code").and_then(Value::as_str), Some("B12345678"));
        let addr = sup
            .get("addresses")
            .and_then(Value::as_array)
            .unwrap()
            .first()
            .unwrap();
        assert_eq!(addr.get("country").and_then(Value::as_str), Some("ES"));
        assert_eq!(addr.get("locality").and_then(Value::as_str), Some("Madrid"));
    }

    #[test]
    fn to_gobl_maps_line_item_and_totals() {
        let env = to_gobl(&sample_doc()).unwrap();
        let lines = env.document.get("lines").and_then(Value::as_array).unwrap();
        assert_eq!(lines.len(), 1);
        let item = lines[0].get("item").unwrap();
        assert_eq!(item.get("name").and_then(Value::as_str), Some("Widget"));
        assert_eq!(item.get("price").and_then(Value::as_str), Some("100.00"));
        let totals = env.document.get("totals").unwrap();
        assert_eq!(totals.get("sum").and_then(Value::as_str), Some("200.00"));
        assert_eq!(
            totals.get("total_with_tax").and_then(Value::as_str),
            Some("242.00")
        );
    }

    #[test]
    fn round_trip_preserves_core_fields() {
        let original = sample_doc();
        let forward = to_gobl(&original).unwrap();
        let backward = from_gobl(&forward.document).unwrap();
        let rt: CommercialDocument = serde_json::from_value(backward.document).unwrap();
        assert_eq!(rt.id, original.id);
        assert_eq!(rt.document_type, original.document_type);
        assert_eq!(rt.issue_date, original.issue_date);
        assert_eq!(rt.due_date, original.due_date);
        assert_eq!(rt.currency, original.currency);
        assert_eq!(rt.supplier.name, original.supplier.name);
        assert_eq!(rt.customer.name, original.customer.name);
        assert_eq!(rt.lines.len(), original.lines.len());
        assert_eq!(rt.lines[0].description, original.lines[0].description);
        assert_eq!(
            rt.monetary_total.payable_amount,
            original.monetary_total.payable_amount
        );
    }

    #[test]
    fn round_trip_credit_note_type() {
        let mut doc = sample_doc();
        doc.document_type = DocumentType::CreditNote;
        let forward = to_gobl(&doc).unwrap();
        assert_eq!(
            forward.document.get("type").and_then(Value::as_str),
            Some("credit-note")
        );
        let backward = from_gobl(&forward.document).unwrap();
        let rt: CommercialDocument = serde_json::from_value(backward.document).unwrap();
        assert_eq!(rt.document_type, DocumentType::CreditNote);
    }

    #[test]
    fn round_trip_debit_note_type() {
        let mut doc = sample_doc();
        doc.document_type = DocumentType::DebitNote;
        let forward = to_gobl(&doc).unwrap();
        assert_eq!(
            forward.document.get("type").and_then(Value::as_str),
            Some("debit-note")
        );
        let backward = from_gobl(&forward.document).unwrap();
        let rt: CommercialDocument = serde_json::from_value(backward.document).unwrap();
        assert_eq!(rt.document_type, DocumentType::DebitNote);
    }

    #[test]
    fn from_gobl_rejects_non_object() {
        let err = from_gobl(&Value::Array(vec![])).unwrap_err();
        assert!(matches!(err, GoblError::NotAnObject));
    }

    #[test]
    fn from_gobl_requires_code() {
        let payload = json!({
            "type": "invoice",
            "issue_date": "2026-05-27",
            "currency": "EUR",
            "supplier": {"name": "S", "addresses": [{"country":"ES","locality":"Madrid","code":"28013"}]},
            "customer": {"name": "C", "addresses": [{"country":"FR","locality":"Paris","code":"75001"}]},
            "lines": [{"item": {"name": "x", "price": "1"}, "sum": "1", "quantity": "1"}],
            "totals": {"sum": "1", "total": "1", "total_with_tax": "1", "payable": "1"},
        });
        let err = from_gobl(&payload).unwrap_err();
        assert!(matches!(err, GoblError::MissingField { .. }));
    }

    #[test]
    fn from_gobl_rejects_unknown_type() {
        let payload = json!({
            "type": "not-a-thing",
            "id": "x", "code": "x", "issue_date": "2026-05-27", "currency": "EUR",
            "supplier": {"name": "S", "addresses": [{"country":"ES","locality":"Madrid","code":"28013"}]},
            "customer": {"name": "C", "addresses": [{"country":"FR","locality":"Paris","code":"75001"}]},
            "lines": [{"item": {"name": "x", "price": "1"}, "sum": "1", "quantity": "1"}],
            "totals": {"sum": "1", "total": "1", "total_with_tax": "1", "payable": "1"},
        });
        let err = from_gobl(&payload).unwrap_err();
        assert!(matches!(err, GoblError::BadValue { .. }));
    }

    #[test]
    fn ledger_records_payee_loss() {
        let mut doc = sample_doc();
        doc.payee = Some(doc.supplier.clone());
        let env = to_gobl(&doc).unwrap();
        assert!(env
            .ledger
            .lost
            .iter()
            .any(|e| e.path == "/payee" && e.reason.contains("payee")));
    }

    #[test]
    fn tax_summary_to_gobl_does_not_panic_on_sum_overflow() {
        // Two tax amounts that each sit at Decimal::MAX overflow when summed.
        // Before the checked-sum fix this panicked ("Addition overflowed").
        let near_max = DecimalValue::new(Decimal::MAX);
        let summary = vec![
            TaxCategorySummary {
                category_code: "VAT".into(),
                taxable_amount: dv("0"),
                tax_amount: near_max.clone(),
                tax_rate: None,
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "VAT2".into(),
                taxable_amount: dv("0"),
                tax_amount: near_max,
                tax_rate: None,
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ];
        let value = tax_summary_to_gobl(&summary);
        // Categories are still emitted; the unrepresentable sum falls back to "0".
        assert_eq!(
            value.get("categories").and_then(Value::as_array).map(Vec::len),
            Some(2)
        );
        assert_eq!(value.get("sum").and_then(Value::as_str), Some("0"));
    }

    #[test]
    fn tax_summary_to_gobl_sums_normal_values_unchanged() {
        let summary = vec![
            TaxCategorySummary {
                category_code: "VAT".into(),
                taxable_amount: dv("100.00"),
                tax_amount: dv("21.00"),
                tax_rate: Some(dv("21.00")),
                exemption_reason: None,
                exemption_reason_code: None,
            },
            TaxCategorySummary {
                category_code: "VAT-RED".into(),
                taxable_amount: dv("50.00"),
                tax_amount: dv("5.00"),
                tax_rate: Some(dv("10.00")),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        ];
        let value = tax_summary_to_gobl(&summary);
        assert_eq!(value.get("sum").and_then(Value::as_str), Some("26.00"));
    }

    #[test]
    fn from_gobl_synthesizes_meta_with_ledger_note_when_missing() {
        let env = to_gobl(&sample_doc()).unwrap();
        let mut payload = env.document;
        payload.as_object_mut().unwrap().remove("meta");
        let env2 = from_gobl(&payload).unwrap();
        assert!(env2.ledger.lost.iter().any(|e| e.path == "/meta"));
    }
}
