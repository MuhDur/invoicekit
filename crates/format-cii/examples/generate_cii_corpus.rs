// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Generate the committed synthetic CII D16B conformance corpus.

use std::collections::BTreeSet;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use invoicekit_format_cii::{mapping, to_xml};
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
const GENERATED_AT: &str = "2026-05-27T10:00:00Z";
const CREATED_DATE: &str = "2026-05-27";
const REVIEW_DUE: &str = "2027-05-27";

const PROFILES: &[Profile] = &[
    Profile {
        name: "Factur-X MINIMUM",
        scenario: "profile-factur-x-minimum",
        guideline_id: "urn:factur-x.eu:1p0:minimum",
    },
    Profile {
        name: "Factur-X BASIC WL",
        scenario: "profile-factur-x-basic-wl",
        guideline_id: "urn:factur-x.eu:1p0:basicwl",
    },
    Profile {
        name: "Factur-X BASIC",
        scenario: "profile-factur-x-basic",
        guideline_id: "urn:cen.eu:en16931:2017#compliant#urn:factur-x.eu:1p0:basic",
    },
    Profile {
        name: "Factur-X EN 16931",
        scenario: "profile-factur-x-en16931",
        guideline_id: "urn:cen.eu:en16931:2017",
    },
    Profile {
        name: "Factur-X EXTENDED",
        scenario: "profile-factur-x-extended",
        guideline_id: "urn:cen.eu:en16931:2017#conformant#urn:factur-x.eu:1p0:extended",
    },
    Profile {
        name: "XRechnung CII",
        scenario: "profile-xrechnung",
        guideline_id: "urn:cen.eu:en16931:2017#compliant#urn:xoev-de:kosit:standard:xrechnung_3.0",
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
    guideline_id: &'static str,
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
    let corpus_root = root.join("conformance-corpus/synthetic/cii-d16b-profiled");
    for number in 1..=FIXTURE_COUNT {
        let fixture = fixture(number)?;
        write_fixture(&corpus_root, &fixture)?;
    }
    println!(
        "Generated or verified {FIXTURE_COUNT} synthetic CII fixtures under {}",
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
    let fixture_name = format!("cii-d16b-{:04}", fixture.number);
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
        "fixture_id": format!("ik-synthetic-cii-d16b-{:04}", fixture.number),
        "corpus_partition": "synthetic",
        "publication": "public",
        "status": "active",
        "title": format!("Synthetic CII D16B {} {:04}", document_type.replace('_', " "), fixture.number),
        "description": format!("Public synthetic CII D16B fixture for {} round-trip conformance, profile coverage, and byte-stable serializer regression.", fixture.profile.name),
        "artifact": {
            "path": "fixture.xml",
            "media_type": "application/xml",
            "sha256": sha256_hex(xml.as_bytes()),
            "size_bytes": xml.len(),
            "format_family": "cii",
            "document_type": document_type,
        },
        "jurisdiction": {
            "countries": fixture_countries(&fixture.document)?,
            "profile": fixture.profile.name,
            "syntax": "UN/CEFACT CII D16B CrossIndustryInvoice",
            "version": "D16B-100",
        },
        "license": {
            "license_id": "CC0-1.0",
            "copyright_holder": "InvoiceKit Authors",
            "redistribution": "public-ok",
        },
        "provenance": {
            "source_kind": "generated",
            "source_name": "InvoiceKit invoices-h4b3 synthetic CII corpus",
            "source_url": "urn:invoicekit:conformance:cii-d16b:synthetic",
            "generated_by": "crates/format-cii/examples/generate_cii_corpus.rs",
            "generator_version": "invoices-h4b3-v1",
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
                    "name": "invoicekit-format-cii parse-serialize-parse",
                    "version": "0.0.0",
                    "result": "pass",
                },
                {
                    "name": "invoicekit-format-cii byte-stability",
                    "version": "0.0.0",
                    "result": "pass",
                },
            ],
            "known_gaps": [
                "Synthetic corpus targets current InvoiceKit CII IR coverage; full D16B schema element coverage is tracked by the CII coverage matrix.",
            ],
        },
        "maintenance": {
            "owner": "InvoiceKit maintainers",
            "created_at": CREATED_DATE,
            "reviewed_at": CREATED_DATE,
            "review_due": REVIEW_DUE,
            "labels": ["synthetic", "cii", "d16b", "roundtrip"],
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
    let document_type = if number % 2 == 0 {
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
    let document_number = format!("IK-CII-D16B-{:04}", config.number);

    Ok(CommercialDocument::new(CommercialDocumentParts {
        schema_version: SchemaVersion::V1_0,
        id: DocumentId::new(document_number.clone())?,
        document_type: config.document_type,
        issue_date: DateOnly::new("2026-05-27")?,
        tax_point_date: Some(DateOnly::new("2026-05-27")?),
        due_date: Some(DateOnly::new("2026-06-26")?),
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
            reference: Some(format!("RF{:04}CII", config.number)),
        }],
        lines,
        tax_summary: tax_summary(config, &amounts),
        monetary_total: monetary_total(config, &amounts),
        attachments: Vec::new(),
        references: Vec::new(),
        notes: notes(config),
        extensions: extensions(config)?,
        meta: DocumentMeta {
            tenant_id: format!("tenant-cii-{:04}", config.number),
            trace_id: format!("trace-cii-{:04}", config.number),
            source_system: Some("invoicekit-cii-corpus-generator".to_owned()),
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
            description: format!("CII conformance service {:04}-{line_number}", config.number),
            quantity: DecimalValue::new(Decimal::new(100 + i64::from(line_number), 2)),
            unit_code: Some(if line_number % 2 == 0 { "HUR" } else { "C62" }.to_owned()),
            unit_price: DecimalValue::new(amount),
            line_extension_amount: DecimalValue::new(amount),
            tax_category: Some(config.vat.code.to_owned()),
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
    Ok(PaymentTerms {
        description: format!(
            "Payable within {} days for {}",
            14 + (config.number % 5) * 7,
            config.profile.name
        ),
        due_date: Some(DateOnly::new("2026-06-26")?),
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
            language: "und".to_owned(),
            text: format!(
                "Synthetic CII fixture {:04} for {}",
                config.number, config.profile.name
            ),
        },
        LocalizedString {
            language: "und".to_owned(),
            text: format!(
                "Buyer reference and payment reference coverage {:04}",
                config.number
            ),
        },
    ]
}

fn cii_document_fields(config: &FixtureConfig) -> Result<JurisdictionExtension, Box<dyn Error>> {
    Ok(JurisdictionExtension::new(
        mapping::CII_DOCUMENT_FIELDS_EXTENSION_URN,
        json!({
            "buyer_reference": format!("BUYER-REF-CII-{:04}", config.number),
            "business_process_context_ids": [
                "urn:invoicekit:conformance:process:cii-d16b",
            ],
        }),
    )?)
}

fn extensions(config: &FixtureConfig) -> Result<Vec<JurisdictionExtension>, Box<dyn Error>> {
    let mut extensions = vec![cii_document_fields(config)?];
    if config.profile.guideline_id != "urn:cen.eu:en16931:2017" {
        extensions.push(JurisdictionExtension::new(
            mapping::CII_PROFILE_CONTEXT_EXTENSION_URN,
            json!({
                "guideline_context_ids": [config.profile.guideline_id],
            }),
        )?);
    }
    Ok(extensions)
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
    scenarios.insert("cii-d16b-cross-industry-invoice".to_owned());
    scenarios.insert("format-cii-round-trip".to_owned());
    scenarios.insert("byte-stable-serializer-output".to_owned());
    scenarios.insert(config.profile.scenario.to_owned());
    scenarios.insert(config.vat.scenario.to_owned());
    scenarios.insert("payment-means-iban".to_owned());
    scenarios.insert("payment-reference".to_owned());
    scenarios.insert("payment-terms".to_owned());
    scenarios.insert("delivery-event".to_owned());
    scenarios.insert("party-tax-registration".to_owned());
    scenarios.insert("party-contact".to_owned());
    scenarios.insert("buyer-reference".to_owned());
    scenarios.insert("business-process-context".to_owned());
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
