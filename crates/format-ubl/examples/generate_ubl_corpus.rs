// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Generate the committed synthetic UBL 2.1 conformance corpus.
//!
//! Mirror of the T-h4b3 CII generator. Emits 50 UBL `Invoice` /
//! `CreditNote` fixtures under
//! `conformance-corpus/synthetic/ubl-2-1/`, each with a
//! `metadata.json` carrying the canonical
//! `fixture-metadata-v1.schema.json` shape. The bbqm release check
//! and rust integration test consume the committed output.

#![allow(clippy::struct_field_names)]

use std::collections::BTreeSet;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use invoicekit_format_ubl::{mapping, to_xml};
use invoicekit_ir::{
    CommercialDocument, CommercialDocumentParts, Contact, CountryCode, DateOnly, DecimalValue,
    DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code,
    JurisdictionExtension, LocalizedString, MonetaryTotal, Party, PartyTaxId, PaymentInstruction,
    PaymentInstructionKind, PaymentTerms, PostalAddress, SchemaVersion, TaxCategorySummary,
};
use rust_decimal::Decimal;
use serde_json::json;
use sha2::{Digest, Sha256};

const FIXTURE_COUNT: u32 = 50;
const GENERATED_AT: &str = "2026-05-27T11:00:00Z";
const CREATED_DATE: &str = "2026-05-27";
const REVIEW_DUE: &str = "2027-05-27";

/// UBL profile presets used to cover Peppol BIS Billing 3.0,
/// XRechnung UBL 3.0, and the PINT family without needing the
/// actual profile crates yet.
const PROFILES: &[Profile] = &[
    Profile {
        name: "Peppol BIS Billing 3.0",
        scenario: "profile-peppol-bis-billing-3",
        customization_id:
            "urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0",
        profile_id: "urn:fdc:peppol.eu:2017:poacc:billing:01:1.0",
    },
    Profile {
        name: "XRechnung UBL 3.0",
        scenario: "profile-xrechnung-ubl",
        customization_id:
            "urn:cen.eu:en16931:2017#compliant#urn:xoev-de:kosit:standard:xrechnung_3.0",
        profile_id: "urn:fdc:peppol.eu:2017:poacc:billing:01:1.0",
    },
    Profile {
        name: "Peppol PINT (international)",
        scenario: "profile-peppol-pint",
        customization_id: "urn:peppol:pint:billing-1@aunz-1",
        profile_id: "urn:peppol:bis:billing",
    },
    Profile {
        name: "EN 16931 (UBL core)",
        scenario: "profile-en16931-ubl",
        customization_id: "urn:cen.eu:en16931:2017",
        profile_id: "urn:fdc:peppol.eu:2017:poacc:billing:01:1.0",
    },
    Profile {
        name: "Peppol BIS Billing 3.0 (CreditNote)",
        scenario: "profile-peppol-bis-credit-note",
        customization_id:
            "urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0",
        profile_id: "urn:fdc:peppol.eu:2017:poacc:billing:01:1.0",
    },
];

const PARTY_PAIRS: &[PartyPair] = &[
    PartyPair {
        supplier_country: "DE",
        supplier_city: "Berlin",
        customer_country: "FR",
        customer_city: "Paris",
    },
    PartyPair {
        supplier_country: "FR",
        supplier_city: "Lyon",
        customer_country: "NL",
        customer_city: "Amsterdam",
    },
    PartyPair {
        supplier_country: "NL",
        supplier_city: "Rotterdam",
        customer_country: "IT",
        customer_city: "Milan",
    },
    PartyPair {
        supplier_country: "IT",
        supplier_city: "Rome",
        customer_country: "ES",
        customer_city: "Madrid",
    },
    PartyPair {
        supplier_country: "ES",
        supplier_city: "Barcelona",
        customer_country: "DE",
        customer_city: "Munich",
    },
];

const VAT_CATEGORIES: &[VatCategory] = &[
    VatCategory {
        code: "S",
        rate_hundredths: 1900,
        scenario: "vat-category-standard",
    },
    VatCategory {
        code: "AA",
        rate_hundredths: 700,
        scenario: "vat-category-reduced",
    },
    VatCategory {
        code: "Z",
        rate_hundredths: 0,
        scenario: "vat-category-zero",
    },
    VatCategory {
        code: "E",
        rate_hundredths: 0,
        scenario: "vat-category-exempt",
    },
    VatCategory {
        code: "AE",
        rate_hundredths: 0,
        scenario: "vat-category-reverse-charge",
    },
];

#[derive(Clone, Copy)]
struct Profile {
    name: &'static str,
    scenario: &'static str,
    customization_id: &'static str,
    profile_id: &'static str,
}

#[derive(Clone, Copy)]
struct PartyPair {
    supplier_country: &'static str,
    supplier_city: &'static str,
    customer_country: &'static str,
    customer_city: &'static str,
}

#[derive(Clone, Copy)]
struct VatCategory {
    code: &'static str,
    rate_hundredths: i64,
    scenario: &'static str,
}

struct Fixture {
    number: u32,
    document: CommercialDocument,
    profile: Profile,
    scenarios: Vec<String>,
}

#[derive(Clone, Copy)]
struct FixtureConfig {
    number: u32,
    profile: Profile,
    parties: PartyPair,
    vat: VatCategory,
    document_type: DocumentType,
    line_count: u32,
    allowance: Option<Decimal>,
    charge: Option<Decimal>,
    prepaid: Option<Decimal>,
}

struct Amounts {
    line_total: Decimal,
    tax_exclusive: Decimal,
    tax_amount: Decimal,
    tax_inclusive: Decimal,
    payable: Decimal,
}

fn main() -> Result<(), Box<dyn Error>> {
    let root = repo_root()?;
    let corpus_root = root.join("conformance-corpus/synthetic/ubl-2-1");
    for number in 1..=FIXTURE_COUNT {
        let fixture = fixture(number)?;
        write_fixture(&corpus_root, &fixture)?;
    }
    println!(
        "Generated or verified {FIXTURE_COUNT} synthetic UBL fixtures under {}",
        corpus_root.display()
    );
    Ok(())
}

fn repo_root() -> Result<PathBuf, Box<dyn Error>> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "could not resolve repository root".into())
}

fn write_fixture(corpus_root: &Path, fixture: &Fixture) -> Result<(), Box<dyn Error>> {
    let fixture_name = format!("ubl-2-1-{:04}", fixture.number);
    let fixture_dir = corpus_root.join(&fixture_name);
    fs::create_dir_all(&fixture_dir)?;

    let xml = to_xml(&fixture.document)?;
    write_new_or_same(&fixture_dir.join("fixture.xml"), xml.as_bytes())?;

    let metadata = metadata(fixture, &xml)?;
    write_new_or_same(&fixture_dir.join("metadata.json"), metadata.as_bytes())?;
    Ok(())
}

fn write_new_or_same(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        let existing = fs::read(path)?;
        if existing == bytes {
            return Ok(());
        }
        return Err(format!("refusing to overwrite changed file {}", path.display()).into());
    }

    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

fn metadata(fixture: &Fixture, xml: &str) -> Result<String, Box<dyn Error>> {
    let document_type = match fixture.document.document_type {
        DocumentType::Invoice => "invoice",
        DocumentType::CreditNote => "credit_note",
        other => return Err(format!("unsupported generated document type {other:?}").into()),
    };
    let metadata = json!({
        "schema_version": "1.0",
        "fixture_id": format!("ik-synthetic-ubl-2-1-{:04}", fixture.number),
        "corpus_partition": "synthetic",
        "publication": "public",
        "status": "active",
        "title": format!("Synthetic UBL 2.1 {} {:04}", document_type.replace('_', " "), fixture.number),
        "description": format!("Public synthetic UBL 2.1 fixture for {} round-trip conformance, profile coverage, and byte-stable serializer regression.", fixture.profile.name),
        "artifact": {
            "path": "fixture.xml",
            "media_type": "application/xml",
            "sha256": sha256_hex(xml.as_bytes()),
            "size_bytes": xml.len(),
            "format_family": "ubl",
            "document_type": document_type,
        },
        "jurisdiction": {
            "countries": fixture_countries(&fixture.document)?,
            "profile": fixture.profile.name,
            "syntax": "OASIS UBL 2.1 Invoice / CreditNote",
            "version": "UBL-2.1",
        },
        "license": {
            "license_id": "CC0-1.0",
            "copyright_holder": "InvoiceKit Authors",
            "redistribution": "public-ok",
        },
        "provenance": {
            "source_kind": "generated",
            "source_name": "InvoiceKit invoices-bbqm synthetic UBL corpus",
            "source_url": "urn:invoicekit:conformance:ubl-2-1:synthetic",
            "generated_by": "crates/format-ubl/examples/generate_ubl_corpus.rs",
            "generator_version": "invoices-bbqm-v1",
            "created_at": GENERATED_AT,
        },
        "pii": {
            "classification": "synthetic",
            "redaction_status": "not-required",
            "contains_personal_data": false,
            "notes": "All parties, identifiers, references, and addresses are fictional.",
        },
        "coverage": {
            "capabilities": ["parse", "serialize", "validate"],
            "scenarios": fixture.scenarios,
            "negative_case": false,
        },
        "validation": {
            "expected_outcome": "valid",
            "validators": [
                {
                    "name": "invoicekit-format-ubl parse-serialize-parse",
                    "version": "0.0.0",
                    "result": "pass",
                },
                {
                    "name": "invoicekit-format-ubl byte-stability",
                    "version": "0.0.0",
                    "result": "pass",
                },
            ],
            "known_gaps": [
                "Synthetic corpus targets current InvoiceKit UBL IR coverage; full UBL 2.1 schema element coverage is tracked by the UBL coverage matrix.",
            ],
        },
        "maintenance": {
            "owner": "InvoiceKit maintainers",
            "created_at": CREATED_DATE,
            "reviewed_at": CREATED_DATE,
            "review_due": REVIEW_DUE,
            "labels": ["synthetic", "ubl", "2-1", "roundtrip"],
        },
    });
    let mut rendered = serde_json::to_string_pretty(&metadata)?;
    rendered.push('\n');
    Ok(rendered)
}

fn fixture(number: u32) -> Result<Fixture, Box<dyn Error>> {
    let config = fixture_config(number)?;
    let document = document(&config)?;

    Ok(Fixture {
        number,
        document,
        profile: config.profile,
        scenarios: scenarios(&config),
    })
}

fn fixture_config(number: u32) -> Result<FixtureConfig, Box<dyn Error>> {
    let index = number - 1;
    let profile = select(PROFILES, index)?;
    let parties = select(PARTY_PAIRS, index)?;
    let vat = select(VAT_CATEGORIES, index)?;
    // CreditNote profile forces document type; otherwise alternate.
    let document_type = if profile.scenario == "profile-peppol-bis-credit-note" || number % 2 == 0 {
        DocumentType::CreditNote
    } else {
        DocumentType::Invoice
    };
    let line_count = 1 + index % 4;
    let allowance = (number % 3 == 0).then(|| Decimal::new(350 + i64::from(number), 2));
    let charge = (number % 4 == 0).then(|| Decimal::new(210 + i64::from(number), 2));
    let prepaid = (number % 7 == 0).then(|| Decimal::new(125 + i64::from(number), 2));

    Ok(FixtureConfig {
        number,
        profile,
        parties,
        vat,
        document_type,
        line_count,
        allowance,
        charge,
        prepaid,
    })
}

fn select<T: Copy>(items: &[T], index: u32) -> Result<T, Box<dyn Error>> {
    if items.is_empty() {
        return Err("selection table must not be empty".into());
    }
    let len = u32::try_from(items.len())?;
    let offset = usize::try_from(index % len)?;
    items
        .get(offset)
        .copied()
        .ok_or_else(|| "selection table must not be empty".into())
}

fn document(config: &FixtureConfig) -> Result<CommercialDocument, Box<dyn Error>> {
    let (lines, line_total) = lines(config);
    let amounts = amounts(config, line_total);
    let document_number = format!("IK-UBL-2-1-{:04}", config.number);

    // UBL 2.1 CreditNote has no top-level cbc:DueDate; the
    // serializer rejects the field for that document type.
    let due_date = match config.document_type {
        DocumentType::Invoice => Some(DateOnly::new("2026-06-26")?),
        _ => None,
    };

    Ok(CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::V1_0,
        id: DocumentId::new(document_number.clone())?,
        document_type: config.document_type,
        issue_date: DateOnly::new("2026-05-27")?,
        tax_point_date: Some(DateOnly::new("2026-05-27")?),
        due_date,
        document_number: DocumentNumber::new(document_number)?,
        currency: Iso4217Code::new("EUR")?,
        supplier: party(
            &format!("supplier-{:04}", config.number),
            &format!("Synthetic Supplier {:04} GmbH", config.number),
            "DE123456789",
            config.parties.supplier_country,
            config.parties.supplier_city,
            config.number,
        )?,
        customer: party(
            &format!("customer-{:04}", config.number),
            &format!("Synthetic Customer {:04} SAS", config.number),
            "FR12345678901",
            config.parties.customer_country,
            config.parties.customer_city,
            config.number + 100,
        )?,
        payee: payee(config)?,
        payment_terms: Some(payment_terms(config)?),
        payment_instructions: vec![PaymentInstruction {
            kind: PaymentInstructionKind::IbanBic,
            account: Some(format!("DE8937040044053201{:04}", config.number)),
            reference: Some(format!("RF{:04}UBL", config.number)),
        }],
        lines,
        tax_summary: tax_summary(config, &amounts),
        monetary_total: monetary_total(config, &amounts),
        attachments: Vec::new(),
        references: Vec::new(),
        notes: notes(config),
        extensions: vec![ubl_document_fields(config)?],
        meta: DocumentMeta {
            tenant_id: format!("tenant-ubl-{:04}", config.number),
            trace_id: format!("trace-ubl-{:04}", config.number),
            source_system: Some("invoicekit-ubl-corpus-generator".to_owned()),
        },
    })?)
}

fn lines(config: &FixtureConfig) -> (Vec<DocumentLine>, Decimal) {
    let mut lines = Vec::new();
    let mut line_total = Decimal::ZERO;
    for line_number in 1..=config.line_count {
        let amount = Decimal::new(
            10_000 + i64::from(config.number * 97) + i64::from(line_number * 713),
            2,
        );
        line_total += amount;
        lines.push(DocumentLine {
            id: line_number.to_string(),
            description: format!("UBL conformance service {:04}-{line_number}", config.number),
            quantity: DecimalValue::new(Decimal::new(100 + i64::from(line_number), 2)),
            unit_code: Some(if line_number % 2 == 0 { "HUR" } else { "C62" }.to_owned()),
            unit_price: DecimalValue::new(amount),
            line_extension_amount: DecimalValue::new(amount),
            tax_category: Some(config.vat.code.to_owned()),
            classifications: Vec::new(),
            extensions: Vec::new(),
        });
    }

    (lines, line_total)
}

fn amounts(config: &FixtureConfig, line_total: Decimal) -> Amounts {
    let tax_exclusive = line_total - config.allowance.unwrap_or(Decimal::ZERO)
        + config.charge.unwrap_or(Decimal::ZERO);
    let rate = Decimal::new(config.vat.rate_hundredths, 2);
    let tax_amount = (tax_exclusive * rate / Decimal::new(100, 0)).round_dp(2);
    let tax_inclusive = tax_exclusive + tax_amount;
    let payable = tax_inclusive - config.prepaid.unwrap_or(Decimal::ZERO);

    Amounts {
        line_total,
        tax_exclusive,
        tax_amount,
        tax_inclusive,
        payable,
    }
}

fn payee(config: &FixtureConfig) -> Result<Option<Party>, Box<dyn Error>> {
    if config.number % 5 != 0 {
        return Ok(None);
    }
    Ok(Some(party(
        &format!("payee-{:04}", config.number),
        &format!("Synthetic Payee {:04} GmbH", config.number),
        "DE987654321",
        "DE",
        "Hamburg",
        config.number + 200,
    )?))
}

fn payment_terms(config: &FixtureConfig) -> Result<PaymentTerms, Box<dyn Error>> {
    // For credit notes the payment-terms due date is also dropped
    // to stay symmetric with the document-level due_date rule.
    let due_date = match config.document_type {
        DocumentType::Invoice => Some(DateOnly::new("2026-06-26")?),
        _ => None,
    };
    Ok(PaymentTerms {
        description: format!(
            "Payable within {} days for {}",
            14 + (config.number % 5) * 7,
            config.profile.name
        ),
        due_date,
    })
}

fn tax_summary(config: &FixtureConfig, amounts: &Amounts) -> Vec<TaxCategorySummary> {
    vec![TaxCategorySummary {
        category_code: config.vat.code.to_owned(),
        taxable_amount: DecimalValue::new(amounts.tax_exclusive),
        tax_amount: DecimalValue::new(amounts.tax_amount),
        tax_rate: Some(DecimalValue::new(Decimal::new(
            config.vat.rate_hundredths,
            2,
        ))),
        exemption_reason: None,
        exemption_reason_code: None,
    }]
}

fn monetary_total(config: &FixtureConfig, amounts: &Amounts) -> MonetaryTotal {
    MonetaryTotal {
        line_extension_amount: DecimalValue::new(amounts.line_total),
        tax_exclusive_amount: DecimalValue::new(amounts.tax_exclusive),
        tax_inclusive_amount: DecimalValue::new(amounts.tax_inclusive),
        allowance_total_amount: config.allowance.map(DecimalValue::new),
        charge_total_amount: config.charge.map(DecimalValue::new),
        prepaid_amount: config.prepaid.map(DecimalValue::new),
        payable_amount: DecimalValue::new(amounts.payable),
    }
}

fn notes(config: &FixtureConfig) -> Vec<LocalizedString> {
    vec![
        LocalizedString {
            language: "en".to_owned(),
            text: format!(
                "Synthetic UBL fixture {:04} for {}",
                config.number, config.profile.name
            ),
        },
        LocalizedString {
            language: "en".to_owned(),
            text: format!(
                "Buyer reference and payment reference coverage {:04}",
                config.number
            ),
        },
    ]
}

fn ubl_document_fields(config: &FixtureConfig) -> Result<JurisdictionExtension, Box<dyn Error>> {
    Ok(JurisdictionExtension::new(
        mapping::UBL_DOCUMENT_FIELDS_EXTENSION_URN,
        json!({
            "accounting_cost": format!("COST-UBL-{:04}", config.number),
            "buyer_reference": format!("BUYER-REF-UBL-{:04}", config.number),
            "customization_id": config.profile.customization_id,
            "profile_id": config.profile.profile_id,
        }),
    )?)
}

fn party(
    id: &str,
    name: &str,
    vat: &str,
    country: &str,
    city: &str,
    seed: u32,
) -> Result<Party, Box<dyn Error>> {
    Ok(Party {
        id: Some(id.to_owned()),
        name: name.to_owned(),
        tax_ids: vec![PartyTaxId {
            scheme: "vat".to_owned(),
            value: vat.to_owned(),
        }],
        address: PostalAddress {
            lines: vec![
                format!("Synthetic Street {}", seed % 97 + 1),
                format!("Suite {}", seed % 31 + 1),
            ],
            city: city.to_owned(),
            subdivision: Some(country.to_owned()),
            postal_code: format!("{:05}", 10_000 + seed % 89_000),
            country: CountryCode::new(country)?,
        },
        contact: Some(Contact {
            name: Some(format!("{name} Contact")),
            email: Some(format!("{id}@example.test")),
            phone: Some(format!("+49-30-{seed:06}")),
        }),
    })
}

fn scenarios(config: &FixtureConfig) -> Vec<String> {
    let mut scenarios = BTreeSet::new();
    scenarios.insert("ubl-2-1-document".to_owned());
    scenarios.insert("format-ubl-round-trip".to_owned());
    scenarios.insert("byte-stable-serializer-output".to_owned());
    scenarios.insert(config.profile.scenario.to_owned());
    scenarios.insert(config.vat.scenario.to_owned());
    scenarios.insert("payment-means-iban".to_owned());
    scenarios.insert("payment-reference".to_owned());
    scenarios.insert("payment-terms".to_owned());
    scenarios.insert("party-tax-registration".to_owned());
    scenarios.insert("party-contact".to_owned());
    scenarios.insert("buyer-reference".to_owned());
    scenarios.insert("accounting-cost".to_owned());
    scenarios.insert("customization-and-profile-ids".to_owned());
    scenarios.insert("included-notes".to_owned());
    if config.document_type == DocumentType::Invoice {
        scenarios.insert("invoice-type-code-380".to_owned());
    } else {
        scenarios.insert("credit-note-type-code-381".to_owned());
    }
    if config.line_count > 1 {
        scenarios.insert("multi-line-document".to_owned());
    }
    if config.allowance.is_some() {
        scenarios.insert("header-allowance-total".to_owned());
    }
    if config.charge.is_some() {
        scenarios.insert("header-charge-total".to_owned());
    }
    if config.prepaid.is_some() {
        scenarios.insert("prepaid-amount".to_owned());
    }
    if config.number % 5 == 0 {
        scenarios.insert("payee-party".to_owned());
    }
    scenarios.into_iter().collect()
}

fn fixture_countries(document: &CommercialDocument) -> Result<Vec<String>, Box<dyn Error>> {
    let mut countries = BTreeSet::new();
    countries.insert(country_string(&document.supplier.address.country)?);
    countries.insert(country_string(&document.customer.address.country)?);
    if let Some(payee) = &document.payee {
        countries.insert(country_string(&payee.address.country)?);
    }
    Ok(countries.into_iter().collect())
}

fn country_string(country: &CountryCode) -> Result<String, serde_json::Error> {
    serde_json::from_value::<String>(serde_json::to_value(country)?)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}
