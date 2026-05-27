// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-042 integration test: project ≥20 cross-border bbqm UBL
//! fixtures through `to_peppol_bis_3_0_xml` and assert the
//! Peppol BIS Billing 3.0 `CustomizationID` + `ProfileID` appear
//! in the output. All bbqm fixtures are cross-border (multi-
//! country supplier/customer pairs).

use std::fs;
use std::path::PathBuf;

use invoicekit_format_ubl::from_xml;
use invoicekit_profile_peppol_bis::{
    to_peppol_bis_3_0_xml, PEPPOL_BIS_3_0_CUSTOMIZATION_ID, PEPPOL_BIS_3_0_PROFILE_ID,
};

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
fn projects_twenty_plus_cross_border_fixtures_with_peppol_bis_headers() -> Result<(), String> {
    let fixtures = fixture_paths();
    assert!(
        fixtures.len() >= 20,
        "T-042 strict gate: need >=20 fixtures; have {}",
        fixtures.len()
    );

    for (i, path) in fixtures.iter().take(20).enumerate() {
        let xml = fs::read_to_string(path)
            .map_err(|err| format!("fixture {i} ({path:?}) could not be read: {err}"))?;
        let (document, _) = from_xml(&xml)
            .map_err(|err| format!("fixture {i} ({path:?}) failed to parse: {err}"))?;
        let projected = to_peppol_bis_3_0_xml(&document)
            .map_err(|err| format!("projection {i} ({path:?}) failed: {err}"))?;

        assert!(
            projected.contains(PEPPOL_BIS_3_0_CUSTOMIZATION_ID),
            "fixture {i}: projected XML must carry the Peppol BIS 3.0 CustomizationID"
        );
        assert!(
            projected.contains(PEPPOL_BIS_3_0_PROFILE_ID),
            "fixture {i}: projected XML must carry the Peppol BIS 3.0 ProfileID"
        );
        // All bbqm fixtures cross at least two countries (DE-FR,
        // FR-NL, NL-IT, IT-ES, ES-DE). Country code elements
        // ship with inline namespace declarations so we match on
        // the open tag.
        assert!(
            projected.contains("cbc:IdentificationCode"),
            "fixture {i}: projected XML must carry country codes"
        );
    }
    Ok(())
}

#[test]
fn projection_replaces_any_existing_customization_override() {
    // Pick a fixture from the XRechnung-UBL profile slot (every
    // 5th index in the bbqm cycle) and project to Peppol BIS;
    // the output should carry the Peppol URN, not the XRechnung
    // one.
    let path = &fixture_paths()[1]; // 0-indexed: 2nd fixture cycles to XRechnung-UBL
    let xml = fs::read_to_string(path).unwrap();
    let (document, _) = from_xml(&xml).unwrap();
    let projected = to_peppol_bis_3_0_xml(&document).unwrap();

    assert!(projected.contains(PEPPOL_BIS_3_0_CUSTOMIZATION_ID));
    assert!(!projected.contains("xrechnung_3.0"));
}
