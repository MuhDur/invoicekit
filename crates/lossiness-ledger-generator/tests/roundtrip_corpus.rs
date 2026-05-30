// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Follow-up conformance harness for real UBL/CII fixture round trips.
//!
//! The format crates already assert `parse -> serialize -> parse`
//! equality. This harness adds the missing acceptance boundary for
//! the lossiness ledger: every fixture has an explicit expected set
//! of `lost` paths, and the generated ledger must match it exactly.

use std::fs;
use std::path::{Path, PathBuf};

use invoicekit_format_cii::from_xml as cii_from_xml;
use invoicekit_format_ubl::from_xml as ubl_from_xml;
use invoicekit_ir::CommercialDocument;
use invoicekit_lossiness_ledger_generator::{compute_ledger, TargetFormat};

const MIN_FIXTURES_PER_FORMAT: usize = 20;
const ZERO_LOSS_LOST_PATHS: &[&str] = &[];
const ZERO_LOSS_PRESERVED_PATHS: &[&str] = &[
    "/allowance_charges",
    "/attachments",
    "/currency",
    "/customer",
    "/deliver_to",
    "/delivery_date",
    "/document_number",
    "/document_type",
    "/due_date",
    "/extensions",
    "/id",
    "/invoice_period",
    "/issue_date",
    "/lines",
    "/meta",
    "/monetary_total",
    "/notes",
    "/payee",
    "/payment_instructions",
    "/payment_terms",
    "/references",
    "/schema_version",
    "/supplier",
    "/tax_point_date",
    "/tax_summary",
];

const UBL_ZERO_LOSS_FIXTURE_IDS: &[&str] = &[
    "ubl-2-1-0001",
    "ubl-2-1-0002",
    "ubl-2-1-0003",
    "ubl-2-1-0004",
    "ubl-2-1-0005",
    "ubl-2-1-0006",
    "ubl-2-1-0007",
    "ubl-2-1-0008",
    "ubl-2-1-0009",
    "ubl-2-1-0010",
    "ubl-2-1-0011",
    "ubl-2-1-0012",
    "ubl-2-1-0013",
    "ubl-2-1-0014",
    "ubl-2-1-0015",
    "ubl-2-1-0016",
    "ubl-2-1-0017",
    "ubl-2-1-0018",
    "ubl-2-1-0019",
    "ubl-2-1-0020",
    "ubl-2-1-0021",
    "ubl-2-1-0022",
    "ubl-2-1-0023",
    "ubl-2-1-0024",
    "ubl-2-1-0025",
    "ubl-2-1-0026",
    "ubl-2-1-0027",
    "ubl-2-1-0028",
    "ubl-2-1-0029",
    "ubl-2-1-0030",
    "ubl-2-1-0031",
    "ubl-2-1-0032",
    "ubl-2-1-0033",
    "ubl-2-1-0034",
    "ubl-2-1-0035",
    "ubl-2-1-0036",
    "ubl-2-1-0037",
    "ubl-2-1-0038",
    "ubl-2-1-0039",
    "ubl-2-1-0040",
    "ubl-2-1-0041",
    "ubl-2-1-0042",
    "ubl-2-1-0043",
    "ubl-2-1-0044",
    "ubl-2-1-0045",
    "ubl-2-1-0046",
    "ubl-2-1-0047",
    "ubl-2-1-0048",
    "ubl-2-1-0049",
    "ubl-2-1-0050",
];

const CII_ZERO_LOSS_FIXTURE_IDS: &[&str] = &[
    "cii-d16b-0001",
    "cii-d16b-0002",
    "cii-d16b-0003",
    "cii-d16b-0004",
    "cii-d16b-0005",
    "cii-d16b-0006",
    "cii-d16b-0007",
    "cii-d16b-0008",
    "cii-d16b-0009",
    "cii-d16b-0010",
    "cii-d16b-0011",
    "cii-d16b-0012",
    "cii-d16b-0013",
    "cii-d16b-0014",
    "cii-d16b-0015",
    "cii-d16b-0016",
    "cii-d16b-0017",
    "cii-d16b-0018",
    "cii-d16b-0019",
    "cii-d16b-0020",
    "cii-d16b-0021",
    "cii-d16b-0022",
    "cii-d16b-0023",
    "cii-d16b-0024",
    "cii-d16b-0025",
    "cii-d16b-0026",
    "cii-d16b-0027",
    "cii-d16b-0028",
    "cii-d16b-0029",
    "cii-d16b-0030",
    "cii-d16b-0031",
    "cii-d16b-0032",
    "cii-d16b-0033",
    "cii-d16b-0034",
    "cii-d16b-0035",
    "cii-d16b-0036",
    "cii-d16b-0037",
    "cii-d16b-0038",
    "cii-d16b-0039",
    "cii-d16b-0040",
    "cii-d16b-0041",
    "cii-d16b-0042",
    "cii-d16b-0043",
    "cii-d16b-0044",
    "cii-d16b-0045",
    "cii-d16b-0046",
    "cii-d16b-0047",
    "cii-d16b-0048",
    "cii-d16b-0049",
    "cii-d16b-0050",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CorpusFormat {
    Ubl,
    Cii,
}

impl CorpusFormat {
    const fn target(self) -> TargetFormat {
        match self {
            Self::Ubl => TargetFormat::Ubl,
            Self::Cii => TargetFormat::Cii,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Ubl => "UBL 2.1",
            Self::Cii => "CII D16B",
        }
    }
}

#[derive(Debug)]
struct CorpusCase {
    format: CorpusFormat,
    fixture_id: String,
    fixture: PathBuf,
    expected_lost_paths: &'static [&'static str],
    expected_preserved_paths: &'static [&'static str],
}

/// invoices-ri01 strict gate:
///
/// - covers at least 20 committed XML fixtures per syntax family;
/// - computes the same-format lossiness ledger for each fixture;
/// - compares actual `lost` paths with the fixture's expected set.
#[test]
fn real_ubl_and_cii_fixtures_match_expected_lossiness_ledgers() -> Result<(), String> {
    let cases = corpus_cases()?;
    let ubl_count = count_format(&cases, CorpusFormat::Ubl);
    let cii_count = count_format(&cases, CorpusFormat::Cii);
    if ubl_count < MIN_FIXTURES_PER_FORMAT {
        return Err(format!(
            "invoices-ri01 requires at least {MIN_FIXTURES_PER_FORMAT} UBL fixtures; saw {ubl_count}"
        ));
    }
    if cii_count < MIN_FIXTURES_PER_FORMAT {
        return Err(format!(
            "invoices-ri01 requires at least {MIN_FIXTURES_PER_FORMAT} CII fixtures; saw {cii_count}"
        ));
    }
    if ubl_count != UBL_ZERO_LOSS_FIXTURE_IDS.len() {
        return Err(format!(
            "UBL fixture expectation table drift: table has {} entries but corpus has {ubl_count}",
            UBL_ZERO_LOSS_FIXTURE_IDS.len()
        ));
    }
    if cii_count != CII_ZERO_LOSS_FIXTURE_IDS.len() {
        return Err(format!(
            "CII fixture expectation table drift: table has {} entries but corpus has {cii_count}",
            CII_ZERO_LOSS_FIXTURE_IDS.len()
        ));
    }

    for case in &cases {
        let document = parse_fixture(case)?;
        let ledger = compute_ledger(&document, case.format.target()).map_err(|error| {
            format!(
                "lossiness ledger failed for {}: {error}",
                case.fixture.display()
            )
        })?;
        let mut actual_lost = ledger
            .lost
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();
        actual_lost.sort_unstable();
        let mut expected_lost = case.expected_lost_paths.to_vec();
        expected_lost.sort_unstable();
        let mut actual_preserved = ledger
            .preserved
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();
        actual_preserved.sort_unstable();
        let mut expected_preserved = case.expected_preserved_paths.to_vec();
        expected_preserved.sort_unstable();

        if actual_lost != expected_lost {
            return Err(format!(
                "unexpected lost-path ledger for {} fixture {} at {}; actual={actual_lost:?} expected={expected_lost:?}; preserved paths were {actual_preserved:?}",
                case.format.name(),
                case.fixture_id,
                case.fixture.display()
            ));
        }
        if actual_preserved != expected_preserved {
            return Err(format!(
                "unexpected preserved-path ledger for {} fixture {} at {}; actual={actual_preserved:?} expected={expected_preserved:?}",
                case.format.name(),
                case.fixture_id,
                case.fixture.display()
            ));
        }
    }
    Ok(())
}

fn corpus_cases() -> Result<Vec<CorpusCase>, String> {
    let mut cases = Vec::new();
    cases.extend(
        fixture_paths("../../conformance-corpus/synthetic/ubl-2-1")?
            .into_iter()
            .map(|fixture| corpus_case(CorpusFormat::Ubl, fixture))
            .collect::<Result<Vec<_>, _>>()?,
    );
    cases.extend(
        fixture_paths("../../conformance-corpus/synthetic/cii-d16b-profiled")?
            .into_iter()
            .map(|fixture| corpus_case(CorpusFormat::Cii, fixture))
            .collect::<Result<Vec<_>, _>>()?,
    );
    Ok(cases)
}

fn corpus_case(format: CorpusFormat, fixture: PathBuf) -> Result<CorpusCase, String> {
    let fixture_id = fixture_id(&fixture)?;
    let expected = expected_ledger(format, &fixture_id)?;
    Ok(CorpusCase {
        format,
        fixture_id,
        fixture,
        expected_lost_paths: expected.lost_paths,
        expected_preserved_paths: expected.preserved_paths,
    })
}

fn fixture_paths(relative: &str) -> Result<Vec<PathBuf>, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    let mut paths = Vec::new();
    for entry in fs::read_dir(&root).map_err(|error| {
        format!(
            "could not read fixture directory {}: {error}",
            root.display()
        )
    })? {
        paths.push(
            entry
                .map_err(|error| {
                    format!(
                        "could not read fixture entry in {}: {error}",
                        root.display()
                    )
                })?
                .path()
                .join("fixture.xml"),
        );
    }
    paths.sort();
    Ok(paths)
}

fn fixture_id(fixture: &Path) -> Result<String, String> {
    fixture
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("could not derive fixture id from {}", fixture.display()))
}

struct ExpectedLedger {
    lost_paths: &'static [&'static str],
    preserved_paths: &'static [&'static str],
}

fn expected_ledger(format: CorpusFormat, fixture_id: &str) -> Result<ExpectedLedger, String> {
    let known = match format {
        CorpusFormat::Ubl => UBL_ZERO_LOSS_FIXTURE_IDS,
        CorpusFormat::Cii => CII_ZERO_LOSS_FIXTURE_IDS,
    };
    if !known.contains(&fixture_id) {
        return Err(format!(
            "missing explicit expected-loss entry for {} fixture {fixture_id}",
            format.name()
        ));
    }
    Ok(ExpectedLedger {
        lost_paths: ZERO_LOSS_LOST_PATHS,
        preserved_paths: ZERO_LOSS_PRESERVED_PATHS,
    })
}

fn parse_fixture(case: &CorpusCase) -> Result<CommercialDocument, String> {
    let xml = fs::read_to_string(&case.fixture)
        .map_err(|error| format!("could not read fixture {}: {error}", case.fixture.display()))?;
    match case.format {
        CorpusFormat::Ubl => ubl_from_xml(&xml)
            .map(|(document, _)| document)
            .map_err(|error| {
                format!(
                    "could not parse UBL fixture {}: {error}",
                    case.fixture.display()
                )
            }),
        CorpusFormat::Cii => cii_from_xml(&xml)
            .map(|(document, _)| document)
            .map_err(|error| {
                format!(
                    "could not parse CII fixture {}: {error}",
                    case.fixture.display()
                )
            }),
    }
}

fn count_format(cases: &[CorpusCase], format: CorpusFormat) -> usize {
    cases.iter().filter(|case| case.format == format).count()
}
