// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-018 codelist update sources.
//!
//! Houses a typed registry of upstream authorities, per-list CSV
//! normalizers, and a deterministic manifest builder that signs the
//! result using the same `sha256:identity` algorithm as the rest of
//! the crate.
//!
//! This module is intentionally network-free. The CLI driver
//! (`invoicekit codelist-update`) reads the upstream data from a
//! local file path; the nightly CI workflow handles the `curl`. That
//! keeps the Rust unit tests hermetic and the crate offline-buildable.
//!
//! Adding a new list is a two-line change here: add a [`SourceSpec`]
//! to [`BUILTIN_SOURCES`] and point its `normalize` field at a
//! per-list CSV parser. The driver does the rest.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::{Entry, Manifest, ISO_4217};

/// Supported upstream payload formats.
///
/// Only CSV is wired today; XML and JSON are listed so the dispatch
/// table is honest about which surface a future bead will extend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceFormat {
    /// Comma-separated values with a header row.
    Csv,
}

/// Per-list normalizer: takes the raw upstream payload and produces a
/// sorted, deduplicated list of [`Entry`] values. The driver wraps
/// these into a [`Manifest`] before signing.
pub type Normalizer = fn(&str) -> Result<Vec<Entry>, SourceError>;

/// Description of one upstream code-list source.
#[derive(Clone, Copy, Debug)]
pub struct SourceSpec {
    /// Stable list identifier (e.g. [`ISO_4217`]).
    pub list_name: &'static str,
    /// Public URL the nightly workflow uses to fetch upstream data.
    /// Carried into [`Manifest::source_url`] verbatim so consumers can
    /// audit provenance from the manifest alone.
    pub upstream_url: &'static str,
    /// Wire format of the upstream payload.
    pub format: SourceFormat,
    /// Normalizer entry point for [`format`](Self::format).
    pub normalize: Normalizer,
    /// Format string for the manifest `version` field. The token
    /// `{RETRIEVED_AT}` is substituted with the run's retrieved-at
    /// date (so the produced manifest version stays deterministic
    /// per UTC day even if the upstream payload has changed).
    pub version_template: &'static str,
    /// Default `effective_from` for produced manifests.
    pub default_effective_from: &'static str,
}

/// Built-in source registry.
///
/// T-018 ships ISO 4217 end-to-end and lists the other seed manifests
/// under follow-up beads so the dispatch table is honest about
/// coverage. Adding a new list here without also adding the per-list
/// normalizer is a compile error because the `normalize` field is
/// non-optional.
pub const BUILTIN_SOURCES: &[SourceSpec] = &[SourceSpec {
    list_name: ISO_4217,
    upstream_url:
        "https://www.six-group.com/en/products-services/financial-information/data-standards.html",
    format: SourceFormat::Csv,
    normalize: normalize_iso_4217_csv,
    version_template: "iso-4217-{RETRIEVED_AT}",
    default_effective_from: "2024-01-01",
}];

/// Errors raised by the source pipeline.
#[derive(Debug, Error)]
pub enum SourceError {
    /// No [`SourceSpec`] exists for the requested list.
    #[error(
        "no upstream source registered for list {list:?}; add an entry to sources::BUILTIN_SOURCES"
    )]
    UnknownList {
        /// The list name the caller asked for.
        list: String,
    },
    /// The upstream payload was malformed.
    #[error("malformed upstream payload for list {list:?}: {detail}")]
    Malformed {
        /// List the parse was attempted for.
        list: String,
        /// Operator-readable reason.
        detail: String,
    },
    /// The retrieved-at value did not match the documented `YYYY-MM-DD` shape.
    #[error("invalid retrieved_at {value:?}: expected YYYY-MM-DD")]
    BadRetrievedAt {
        /// Offending value.
        value: String,
    },
}

/// Look up a registered source by list name.
///
/// # Errors
///
/// Returns [`SourceError::UnknownList`] when no entry of
/// [`BUILTIN_SOURCES`] matches `list`.
pub fn source_for(list: &str) -> Result<&'static SourceSpec, SourceError> {
    BUILTIN_SOURCES
        .iter()
        .find(|s| s.list_name == list)
        .ok_or_else(|| SourceError::UnknownList {
            list: list.to_owned(),
        })
}

/// Build a fresh [`Manifest`] from a registered source plus the raw
/// upstream payload. The returned manifest is signed and round-trips
/// through [`Manifest::verify`].
///
/// `retrieved_at` is carried into the manifest verbatim and used to
/// expand the source's `version_template`. CI pins it to the
/// workflow's start-of-day UTC date so re-runs are byte-identical.
///
/// # Errors
///
/// Returns [`SourceError::BadRetrievedAt`] when `retrieved_at` is not
/// `YYYY-MM-DD`, [`SourceError::Malformed`] when the upstream payload
/// fails to parse, and [`SourceError::UnknownList`] when `spec.list_name`
/// is not in [`BUILTIN_SOURCES`].
pub fn build_manifest(
    spec: &SourceSpec,
    raw_upstream: &str,
    retrieved_at: &str,
) -> Result<Manifest, SourceError> {
    if !is_iso_date(retrieved_at) {
        return Err(SourceError::BadRetrievedAt {
            value: retrieved_at.to_owned(),
        });
    }
    let entries = (spec.normalize)(raw_upstream)?;
    let version = spec
        .version_template
        .replace("{RETRIEVED_AT}", retrieved_at);
    let mut manifest = Manifest {
        list: spec.list_name.to_owned(),
        version,
        effective_from: spec.default_effective_from.to_owned(),
        effective_to: None,
        source_url: spec.upstream_url.to_owned(),
        retrieved_at: retrieved_at.to_owned(),
        signature_alg: "sha256:identity".to_owned(),
        signature: String::new(),
        entries,
    };
    manifest.signature = manifest.expected_signature();
    Ok(manifest)
}

fn is_iso_date(s: &str) -> bool {
    if s.len() != 10 {
        return false;
    }
    let bytes = s.as_bytes();
    bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

/// Parse an ISO 4217 CSV payload.
///
/// Expected header (case-insensitive, order-insensitive):
///
/// ```text
/// code,label,numeric,minor_units
/// ```
///
/// Each remaining row produces one [`Entry`] with `numeric` and
/// `minor_units` carried as `attrs`. Empty or whitespace-only lines
/// are skipped. Lines beginning with `#` are treated as comments and
/// also skipped. Duplicate codes are rejected up-front so the
/// resulting [`Manifest`] passes [`Manifest::verify`] without
/// surprise.
///
/// # Errors
///
/// Returns [`SourceError::Malformed`] when the header is missing,
/// when a required column is absent, when a row has the wrong column
/// count, when `minor_units` is not an integer, when `numeric` is not
/// a 3-digit numeric, or when two rows declare the same `code`.
pub fn normalize_iso_4217_csv(raw: &str) -> Result<Vec<Entry>, SourceError> {
    let list = ISO_4217;
    let bad = |detail: String| SourceError::Malformed {
        list: list.to_owned(),
        detail,
    };

    let mut rows = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'));

    let header = rows
        .next()
        .ok_or_else(|| bad("missing header row".into()))?;
    let cols: Vec<&str> = header.split(',').map(str::trim).collect();
    let find = |name: &str| {
        cols.iter()
            .position(|c| c.eq_ignore_ascii_case(name))
            .ok_or_else(|| bad(format!("missing required column {name:?}")))
    };
    let i_code = find("code")?;
    let i_label = find("label")?;
    let i_numeric = find("numeric")?;
    let i_minor = find("minor_units")?;

    let mut out: Vec<Entry> = Vec::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (lineno, row) in rows.enumerate() {
        let fields: Vec<&str> = row.split(',').map(str::trim).collect();
        if fields.len() != cols.len() {
            return Err(bad(format!(
                "row {row_num} has {got} columns, header has {want}",
                row_num = lineno + 2,
                got = fields.len(),
                want = cols.len()
            )));
        }
        let code = fields[i_code].to_owned();
        if code.is_empty() {
            return Err(bad(format!("row {} has empty code", lineno + 2)));
        }
        let numeric = fields[i_numeric].to_owned();
        if numeric.len() != 3 || !numeric.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad(format!(
                "row {} numeric {numeric:?} is not 3 digits",
                lineno + 2
            )));
        }
        let minor = fields[i_minor].to_owned();
        if minor.parse::<u8>().is_err() {
            return Err(bad(format!(
                "row {} minor_units {minor:?} is not an integer",
                lineno + 2
            )));
        }
        if let Some(prior) = seen.insert(code.clone(), lineno + 2) {
            return Err(bad(format!(
                "row {} declares duplicate code {code:?} (already seen on row {prior})",
                lineno + 2
            )));
        }
        let mut attrs = BTreeMap::new();
        attrs.insert("numeric".to_owned(), numeric);
        attrs.insert("minor_units".to_owned(), minor);
        out.push(Entry {
            code,
            label: fields[i_label].to_owned(),
            valid_from: None,
            valid_to: None,
            attrs,
        });
    }
    if out.is_empty() {
        return Err(bad("no data rows after header".into()));
    }
    out.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_sources_carries_iso_4217() {
        let s = source_for(ISO_4217).expect("ISO 4217 must be registered");
        assert_eq!(s.list_name, ISO_4217);
        assert!(matches!(s.format, SourceFormat::Csv));
    }

    #[test]
    fn unknown_list_is_rejected() {
        let err = source_for("not-a-real-list").expect_err("must reject unknown lists");
        assert!(matches!(err, SourceError::UnknownList { .. }));
    }

    #[test]
    fn iso_4217_normalizer_accepts_minimal_csv() {
        let raw = "code,label,numeric,minor_units\nEUR,Euro,978,2\nJPY,Yen,392,0\n";
        let entries = normalize_iso_4217_csv(raw).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].code, "EUR");
        assert_eq!(entries[1].code, "JPY");
        assert_eq!(
            entries[1].attrs.get("minor_units").map(String::as_str),
            Some("0")
        );
    }

    #[test]
    fn iso_4217_normalizer_is_order_insensitive_on_columns() {
        let raw = "minor_units,numeric,label,code\n2,978,Euro,EUR\n";
        let entries = normalize_iso_4217_csv(raw).unwrap();
        assert_eq!(entries[0].code, "EUR");
        assert_eq!(
            entries[0].attrs.get("numeric").map(String::as_str),
            Some("978")
        );
    }

    #[test]
    fn iso_4217_normalizer_rejects_missing_column() {
        let raw = "code,label,numeric\nEUR,Euro,978\n";
        let err = normalize_iso_4217_csv(raw).unwrap_err();
        assert!(
            matches!(err, SourceError::Malformed { detail, .. } if detail.contains("minor_units"))
        );
    }

    #[test]
    fn iso_4217_normalizer_rejects_bad_numeric() {
        let raw = "code,label,numeric,minor_units\nEUR,Euro,97A,2\n";
        let err = normalize_iso_4217_csv(raw).unwrap_err();
        assert!(
            matches!(err, SourceError::Malformed { detail, .. } if detail.contains("3 digits"))
        );
    }

    #[test]
    fn iso_4217_normalizer_rejects_duplicate_code() {
        let raw = "code,label,numeric,minor_units\nEUR,Euro,978,2\nEUR,Other,978,2\n";
        let err = normalize_iso_4217_csv(raw).unwrap_err();
        assert!(
            matches!(err, SourceError::Malformed { detail, .. } if detail.contains("duplicate"))
        );
    }

    #[test]
    fn iso_4217_normalizer_rejects_empty_data() {
        let raw = "code,label,numeric,minor_units\n";
        let err = normalize_iso_4217_csv(raw).unwrap_err();
        assert!(
            matches!(err, SourceError::Malformed { detail, .. } if detail.contains("no data rows"))
        );
    }

    #[test]
    fn iso_4217_normalizer_skips_comments_and_blank_lines() {
        let raw = "# Comment\ncode,label,numeric,minor_units\n\nEUR,Euro,978,2\n# trailing\n";
        let entries = normalize_iso_4217_csv(raw).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn build_manifest_signs_round_trip() {
        let raw = "code,label,numeric,minor_units\nEUR,Euro,978,2\nUSD,US Dollar,840,2\n";
        let spec = source_for(ISO_4217).unwrap();
        let m = build_manifest(spec, raw, "2026-05-27").unwrap();
        assert_eq!(m.list, ISO_4217);
        assert_eq!(m.retrieved_at, "2026-05-27");
        assert_eq!(m.version, "iso-4217-2026-05-27");
        assert_eq!(m.source_url, spec.upstream_url);
        assert!(!m.signature.is_empty());
        m.verify().expect("freshly built manifest must verify");
    }

    #[test]
    fn build_manifest_rejects_bad_retrieved_at() {
        let raw = "code,label,numeric,minor_units\nEUR,Euro,978,2\n";
        let spec = source_for(ISO_4217).unwrap();
        let err = build_manifest(spec, raw, "2026/05/27").unwrap_err();
        assert!(matches!(err, SourceError::BadRetrievedAt { .. }));
    }

    #[test]
    fn build_manifest_is_deterministic_for_same_inputs() {
        let raw = "code,label,numeric,minor_units\nEUR,Euro,978,2\nUSD,US Dollar,840,2\n";
        let spec = source_for(ISO_4217).unwrap();
        let a = build_manifest(spec, raw, "2026-05-27").unwrap();
        let b = build_manifest(spec, raw, "2026-05-27").unwrap();
        assert_eq!(a, b, "manifest must be byte-identical across runs");
    }
}
