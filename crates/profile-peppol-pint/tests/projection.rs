// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-043 integration test: project ≥10 bbqm UBL fixtures through
//! the PINT projection, varying the country argument across all
//! five strict-gate authorities (AU/NZ joint, SG, JP, AE, MY).
//! Every projection must carry the expected PINT
//! `CustomizationID` for the chosen country.

use std::fs;
use std::path::PathBuf;

use invoicekit_format_ubl::from_xml;
use invoicekit_profile_peppol_pint::{to_peppol_pint_xml, PintCountry};

fn fixture_paths() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance-corpus/synthetic/ubl-2-1");
    let mut paths = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path().join("fixture.xml"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[test]
fn projects_ten_plus_fixtures_across_five_pint_countries() -> Result<(), String> {
    let fixtures = fixture_paths();
    assert!(
        fixtures.len() >= 10,
        "T-043 strict gate: need >=10 fixtures; have {}",
        fixtures.len()
    );

    let countries = [
        PintCountry::AustraliaNewZealand,
        PintCountry::Singapore,
        PintCountry::Japan,
        PintCountry::UnitedArabEmirates,
        PintCountry::Malaysia,
    ];

    let mut total_projections = 0usize;
    // Project the first 10 fixtures, cycling through the 5
    // countries. 10 fixtures * 5 countries = 50 projections.
    for (i, path) in fixtures.iter().take(10).enumerate() {
        let xml = fs::read_to_string(path)
            .map_err(|err| format!("fixture {i} ({path:?}) could not be read: {err}"))?;
        let (document, _) = from_xml(&xml)
            .map_err(|err| format!("fixture {i} ({path:?}) failed to parse: {err}"))?;

        for country in countries {
            let projected = to_peppol_pint_xml(&document, country)
                .map_err(|err| format!("projection {i}/{country:?} ({path:?}) failed: {err}"))?;
            assert!(
                projected.contains(country.customization_id()),
                "fixture {i} country {country:?}: projected XML must carry the PINT CustomizationID"
            );
            assert!(
                projected.contains(country.profile_id()),
                "fixture {i} country {country:?}: projected XML must carry the PINT ProfileID"
            );
            total_projections += 1;
        }
    }
    assert!(
        total_projections >= 50,
        "expected >=50 projections; got {total_projections}"
    );
    Ok(())
}

#[test]
fn projection_replaces_existing_customization_with_country_specific_urn() {
    // Pick a fixture and project to Singapore PINT; the result
    // must NOT carry the Peppol BIS / XRechnung / EN 16931 URNs.
    let path = &fixture_paths()[0];
    let xml = fs::read_to_string(path).unwrap();
    let (document, _) = from_xml(&xml).unwrap();
    let projected = to_peppol_pint_xml(&document, PintCountry::Singapore).unwrap();

    assert!(projected.contains("urn:peppol:pint:billing-1@sg-1"));
    assert!(!projected.contains("urn:xoev-de:kosit"));
    assert!(!projected.contains("urn:fdc:peppol.eu:2017:poacc:billing:3.0"));
}
