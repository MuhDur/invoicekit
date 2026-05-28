// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! Signed, versioned, effective-dated code list registry.
//!
//! Code lists are data, not constants baked into validation code. This crate
//! loads embedded manifest snapshots and answers date-pinned lookup queries.
//! T-018 owns the external updater; its atomic output shape is the same
//! [`Manifest`] structure used here:
//!
//! 1. fetch the authoritative upstream list,
//! 2. normalize it into manifest JSON with source and retrieval metadata,
//! 3. verify the manifest signature before it enters a registry,
//! 4. atomically swap the old snapshot for the new one.
//!
//! The seed data is intentionally small but real. It covers the initial list
//! families required by T-015 and keeps the loading, validation, and lookup
//! contract executable before the updater lands.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub mod sources;

const SEED_MANIFESTS: &[&str] = &[
    include_str!("../data/iso-3166-1-alpha2.json"),
    include_str!("../data/iso-3166-2.json"),
    include_str!("../data/iso-4217-2024.json"),
    include_str!("../data/unece-rec20-units.json"),
    include_str!("../data/en16931-vat-categories.json"),
    include_str!("../data/peppol-uncl1001-invoice.json"),
    include_str!("../data/peppol-participant-schemes.json"),
];

/// ISO 3166-1 alpha-2 country code list name.
pub const ISO_3166_1_ALPHA2: &str = "iso-3166-1:alpha2";

/// ISO 3166-2 subdivision code list name.
pub const ISO_3166_2: &str = "iso-3166-2";

/// ISO 4217 active currency code list name.
pub const ISO_4217: &str = "iso-4217";

/// UN/ECE Recommendation 20 unit-code list name.
pub const UNECE_REC20_UNITS: &str = "unece-rec20:units";

/// EN 16931 VAT category code list name.
pub const EN16931_VAT_CATEGORY: &str = "en16931:vat-category";

/// Peppol BIS Billing invoice document type code list name.
pub const PEPPOL_INVOICE_TYPE: &str = "peppol:uncl1001-invoice";

/// Peppol participant identifier scheme code list name.
pub const PEPPOL_PARTICIPANT_SCHEME: &str = "peppol:participant-scheme";

/// Canonical Cargo package name of this crate.
///
/// Used by the InvoiceKit release tooling and by the bead-correlation reports
/// to map runtime log records back to the originating crate without parsing
/// `Cargo.toml` at runtime.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_codelists::crate_name(), "invoicekit-codelists");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-codelists"
}

/// A signed snapshot of one code list for an effective date range.
///
/// # Examples
///
/// ```
/// let registry = invoicekit_codelists::Registry::seeded().unwrap();
/// let manifest = registry.manifest(invoicekit_codelists::ISO_4217, "2024-06-01").unwrap();
/// manifest.verify().unwrap();
/// ```
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Manifest {
    /// Stable list identifier used by lookup callers.
    pub list: String,
    /// Upstream or InvoiceKit-normalized version identifier.
    pub version: String,
    /// First date this manifest can answer for, formatted as `YYYY-MM-DD`.
    pub effective_from: String,
    /// Last date this manifest can answer for, inclusive.
    pub effective_to: Option<String>,
    /// Authoritative upstream source URL.
    pub source_url: String,
    /// Retrieval timestamp or source publication date.
    pub retrieved_at: String,
    /// Signature algorithm for the manifest payload.
    pub signature_alg: String,
    /// Hex-encoded signature or digest according to `signature_alg`.
    pub signature: String,
    /// Entries carried by this manifest.
    pub entries: Vec<Entry>,
}

impl Manifest {
    /// Verify the manifest envelope, effective dates, entries, and signature.
    ///
    /// # Examples
    ///
    /// ```
    /// let registry = invoicekit_codelists::Registry::seeded().unwrap();
    /// let manifest = registry.manifest(invoicekit_codelists::EN16931_VAT_CATEGORY, "2024-06-01").unwrap();
    /// assert!(manifest.verify().is_ok());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest is malformed, has duplicate entries,
    /// carries unsupported signature metadata, or its signature does not match.
    pub fn verify(&self) -> Result<(), CodelistError> {
        self.validate()?;
        match self.signature_alg.as_str() {
            "sha256:identity" => {}
            algorithm => {
                return Err(CodelistError::UnsupportedSignatureAlgorithm {
                    list: self.list.clone(),
                    algorithm: algorithm.to_owned(),
                });
            }
        }

        let expected = self.expected_signature();
        if !constant_time_eq(self.signature.as_bytes(), expected.as_bytes()) {
            return Err(CodelistError::SignatureMismatch {
                list: self.list.clone(),
                expected,
                actual: self.signature.clone(),
            });
        }
        Ok(())
    }

    /// Compute the expected signature for the manifest payload.
    ///
    /// # Examples
    ///
    /// ```
    /// let registry = invoicekit_codelists::Registry::seeded().unwrap();
    /// let manifest = registry.manifest(invoicekit_codelists::ISO_3166_1_ALPHA2, "2024-06-01").unwrap();
    /// assert_eq!(manifest.expected_signature().len(), 64);
    /// ```
    #[must_use]
    pub fn expected_signature(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.signing_payload().as_bytes());
        hex_lower(&hasher.finalize())
    }

    /// True when this manifest covers `on_date`.
    ///
    /// # Examples
    ///
    /// ```
    /// let registry = invoicekit_codelists::Registry::seeded().unwrap();
    /// let manifest = registry.manifest(invoicekit_codelists::ISO_4217, "2024-06-01").unwrap();
    /// assert!(manifest.is_effective_on("2024-06-01"));
    /// ```
    #[must_use]
    pub fn is_effective_on(&self, on_date: &str) -> bool {
        is_within_window(
            on_date,
            Some(&self.effective_from),
            self.effective_to.as_deref(),
        )
    }

    fn validate(&self) -> Result<(), CodelistError> {
        require_non_empty("list", &self.list)?;
        require_non_empty("version", &self.version)?;
        require_non_empty("source_url", &self.source_url)?;
        require_non_empty("retrieved_at", &self.retrieved_at)?;
        require_non_empty("signature_alg", &self.signature_alg)?;
        require_non_empty("signature", &self.signature)?;
        // 522z: reject separator characters that would make the
        // signing-payload format ambiguous.
        require_payload_safe("list", &self.list)?;
        require_payload_safe("version", &self.version)?;
        require_payload_safe("source_url", &self.source_url)?;
        require_payload_safe("retrieved_at", &self.retrieved_at)?;
        if let Some(effective_to) = &self.effective_to {
            require_payload_safe("effective_to", effective_to)?;
        }
        require_payload_safe("effective_from", &self.effective_from)?;
        validate_date(&self.effective_from)?;
        if let Some(effective_to) = &self.effective_to {
            validate_date(effective_to)?;
            if effective_to < &self.effective_from {
                return Err(CodelistError::InvalidDateWindow {
                    list: self.list.clone(),
                    from: self.effective_from.clone(),
                    to: Some(effective_to.clone()),
                });
            }
        }
        if self.entries.is_empty() {
            return Err(CodelistError::EmptyManifest {
                list: self.list.clone(),
            });
        }

        let mut codes = BTreeSet::new();
        for entry in &self.entries {
            entry.validate(&self.list)?;
            if !codes.insert(entry.code.clone()) {
                return Err(CodelistError::DuplicateEntry {
                    list: self.list.clone(),
                    code: entry.code.clone(),
                });
            }
        }
        Ok(())
    }

    fn signing_payload(&self) -> String {
        let mut lines = vec![
            "manifest-v1".to_owned(),
            format!("list={}", self.list),
            format!("version={}", self.version),
            format!("effective_from={}", self.effective_from),
            format!(
                "effective_to={}",
                self.effective_to.as_deref().unwrap_or("")
            ),
            format!("source_url={}", self.source_url),
            format!("retrieved_at={}", self.retrieved_at),
        ];

        for entry in &self.entries {
            let attrs = entry
                .attrs
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(";");
            lines.push(format!(
                "entry={}|{}|{}|{}|{}",
                entry.code,
                entry.label,
                entry.valid_from.as_deref().unwrap_or(""),
                entry.valid_to.as_deref().unwrap_or(""),
                attrs
            ));
        }

        lines.join("\n")
    }
}

/// One code-list entry and its optional validity window.
///
/// # Examples
///
/// ```
/// let registry = invoicekit_codelists::Registry::seeded().unwrap();
/// let entry = registry.lookup(invoicekit_codelists::ISO_4217, "EUR", "2024-06-01").unwrap();
/// assert_eq!(entry.code, "EUR");
/// ```
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Entry {
    /// Code value as it appears in invoices.
    pub code: String,
    /// Human-readable label from the source list or InvoiceKit normalization.
    pub label: String,
    /// Optional first date this entry is valid, formatted as `YYYY-MM-DD`.
    pub valid_from: Option<String>,
    /// Optional last date this entry is valid, inclusive.
    pub valid_to: Option<String>,
    /// Extra source-specific attributes.
    #[serde(default)]
    pub attrs: BTreeMap<String, String>,
}

impl Entry {
    /// True when this entry covers `on_date` within a manifest window.
    ///
    /// # Examples
    ///
    /// ```
    /// let registry = invoicekit_codelists::Registry::seeded().unwrap();
    /// let entry = registry.lookup(invoicekit_codelists::ISO_3166_1_ALPHA2, "DE", "2024-06-01").unwrap();
    /// assert!(entry.is_effective_on("2024-06-01"));
    /// ```
    #[must_use]
    pub fn is_effective_on(&self, on_date: &str) -> bool {
        is_within_window(
            on_date,
            self.valid_from.as_deref(),
            self.valid_to.as_deref(),
        )
    }

    fn validate(&self, list: &str) -> Result<(), CodelistError> {
        require_non_empty("code", &self.code)?;
        require_non_empty("label", &self.label)?;
        // 522z: keep the signing payload unambiguous by rejecting
        // separator characters in entry-shaped fields.
        require_payload_safe("entry.code", &self.code)?;
        require_payload_safe("entry.label", &self.label)?;
        if let Some(valid_from) = &self.valid_from {
            require_payload_safe("entry.valid_from", valid_from)?;
            validate_date(valid_from)?;
        }
        if let Some(valid_to) = &self.valid_to {
            require_payload_safe("entry.valid_to", valid_to)?;
            validate_date(valid_to)?;
        }
        for (key, value) in &self.attrs {
            require_payload_safe("entry.attrs.key", key)?;
            require_payload_safe("entry.attrs.value", value)?;
        }
        if let (Some(valid_from), Some(valid_to)) = (&self.valid_from, &self.valid_to) {
            if valid_to < valid_from {
                return Err(CodelistError::InvalidDateWindow {
                    list: list.to_owned(),
                    from: valid_from.clone(),
                    to: Some(valid_to.clone()),
                });
            }
        }
        Ok(())
    }
}

/// In-memory registry of signed code-list manifests.
///
/// # Examples
///
/// ```
/// let registry = invoicekit_codelists::Registry::seeded().unwrap();
/// assert!(registry.lookup(invoicekit_codelists::ISO_3166_1_ALPHA2, "DE", "2024-06-01").is_some());
/// ```
/// A verified manifest paired with a `code -> entry index` map.
///
/// The index is built once when the registry is constructed so that
/// [`Registry::lookup`] is an O(1) hash plus a single validity-window check,
/// instead of a linear scan over every entry on each call. Codes are unique
/// within a manifest (enforced by [`Manifest::verify`]'s duplicate check), so
/// the map holds exactly one entry index per code and the looked-up entry is
/// byte-for-byte the same one the previous linear `find` would have returned.
#[derive(Clone, Debug, Eq, PartialEq)]
struct IndexedManifest {
    manifest: Manifest,
    code_index: HashMap<String, usize>,
}

impl IndexedManifest {
    fn new(manifest: Manifest) -> Self {
        let mut code_index = HashMap::with_capacity(manifest.entries.len());
        for (idx, entry) in manifest.entries.iter().enumerate() {
            // Codes are unique per manifest (Manifest::verify rejects
            // duplicates), so this never overwrites a smaller index; the map
            // therefore points at the same entry the old linear scan found.
            code_index.entry(entry.code.clone()).or_insert(idx);
        }
        Self {
            manifest,
            code_index,
        }
    }

    /// Look up `code`, assuming `on_date` has already been validated by the
    /// caller. Uses the unchecked window check so the date is not re-parsed.
    fn lookup_validated(&self, code: &str, on_date: &str) -> Option<&Entry> {
        let idx = *self.code_index.get(code)?;
        let entry = &self.manifest.entries[idx];
        is_within_window_unchecked(
            on_date,
            entry.valid_from.as_deref(),
            entry.valid_to.as_deref(),
        )
        .then_some(entry)
    }

    /// True when this manifest's window covers an already-validated `on_date`.
    fn covers_validated(&self, on_date: &str) -> bool {
        is_within_window_unchecked(
            on_date,
            Some(&self.manifest.effective_from),
            self.manifest.effective_to.as_deref(),
        )
    }
}

/// In-memory registry of signed code-list manifests.
///
/// # Examples
///
/// ```
/// let registry = invoicekit_codelists::Registry::seeded().unwrap();
/// assert!(registry.lookup(invoicekit_codelists::ISO_3166_1_ALPHA2, "DE", "2024-06-01").is_some());
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Registry {
    manifests: BTreeMap<String, Vec<IndexedManifest>>,
}

impl Registry {
    /// Build a registry from InvoiceKit's embedded seed manifests.
    ///
    /// # Examples
    ///
    /// ```
    /// let registry = invoicekit_codelists::Registry::seeded().unwrap();
    /// assert!(registry.list_names().any(|name| name == invoicekit_codelists::ISO_4217));
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if any embedded manifest is malformed or fails
    /// signature verification.
    pub fn seeded() -> Result<Self, CodelistError> {
        let manifests = SEED_MANIFESTS
            .iter()
            .map(|raw| Manifest::from_json(raw))
            .collect::<Result<Vec<_>, _>>()?;
        Self::from_manifests(manifests)
    }

    /// Build a registry from caller-supplied manifests.
    ///
    /// # Examples
    ///
    /// ```
    /// let seeded = invoicekit_codelists::Registry::seeded().unwrap();
    /// let manifests = seeded.manifests().cloned().collect::<Vec<_>>();
    /// let rebuilt = invoicekit_codelists::Registry::from_manifests(manifests).unwrap();
    /// assert_eq!(rebuilt.list_names().count(), seeded.list_names().count());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when a manifest is invalid or signature verification
    /// fails.
    pub fn from_manifests(manifests: Vec<Manifest>) -> Result<Self, CodelistError> {
        let mut grouped: BTreeMap<String, Vec<Manifest>> = BTreeMap::new();
        for manifest in manifests {
            manifest.verify()?;
            grouped
                .entry(manifest.list.clone())
                .or_default()
                .push(manifest);
        }
        let mut indexed: BTreeMap<String, Vec<IndexedManifest>> = BTreeMap::new();
        for (list, mut manifests) in grouped {
            manifests.sort_by(|left, right| right.effective_from.cmp(&left.effective_from));
            indexed.insert(
                list,
                manifests.into_iter().map(IndexedManifest::new).collect(),
            );
        }
        Ok(Self { manifests: indexed })
    }

    /// Look up a code in a list for an effective date.
    ///
    /// Returns `None` when the list is unknown, the date is malformed, no
    /// manifest covers the date, the code is unknown, or the entry is outside
    /// its own validity window.
    ///
    /// # Examples
    ///
    /// ```
    /// let registry = invoicekit_codelists::Registry::seeded().unwrap();
    /// let eur = registry.lookup(invoicekit_codelists::ISO_4217, "EUR", "2024-06-01").unwrap();
    /// assert_eq!(eur.label, "Euro");
    /// ```
    #[must_use]
    pub fn lookup(&self, list: &str, code: &str, on_date: &str) -> Option<&Entry> {
        validate_date(on_date).ok()?;
        self.indexed_manifest_validated(list, on_date)?
            .lookup_validated(code, on_date)
    }

    /// Return the manifest that covers `list` on `on_date`.
    ///
    /// # Examples
    ///
    /// ```
    /// let registry = invoicekit_codelists::Registry::seeded().unwrap();
    /// let manifest = registry.manifest(invoicekit_codelists::PEPPOL_INVOICE_TYPE, "2024-06-01").unwrap();
    /// assert_eq!(manifest.list, invoicekit_codelists::PEPPOL_INVOICE_TYPE);
    /// ```
    #[must_use]
    pub fn manifest(&self, list: &str, on_date: &str) -> Option<&Manifest> {
        validate_date(on_date).ok()?;
        Some(&self.indexed_manifest_validated(list, on_date)?.manifest)
    }

    /// Internal: find the indexed manifest covering `list` on an
    /// already-validated `on_date`.
    ///
    /// Same selection as [`Self::manifest`] (first manifest whose window covers
    /// the date, in the construction-time effective-from-descending order), but
    /// returns the wrapper so callers can reach the per-manifest code index, and
    /// uses the unchecked window test since the caller validated `on_date`.
    fn indexed_manifest_validated(&self, list: &str, on_date: &str) -> Option<&IndexedManifest> {
        self.manifests
            .get(list)?
            .iter()
            .find(|indexed| indexed.covers_validated(on_date))
    }

    /// Iterate over all list names in the registry.
    ///
    /// # Examples
    ///
    /// ```
    /// let registry = invoicekit_codelists::Registry::seeded().unwrap();
    /// assert!(registry.list_names().any(|name| name == invoicekit_codelists::EN16931_VAT_CATEGORY));
    /// ```
    pub fn list_names(&self) -> impl Iterator<Item = &str> {
        self.manifests.keys().map(String::as_str)
    }

    /// Iterate over all manifests in the registry.
    ///
    /// # Examples
    ///
    /// ```
    /// let registry = invoicekit_codelists::Registry::seeded().unwrap();
    /// assert!(registry.manifests().all(|manifest| manifest.verify().is_ok()));
    /// ```
    pub fn manifests(&self) -> impl Iterator<Item = &Manifest> {
        self.manifests
            .values()
            .flat_map(|manifests| manifests.iter())
            .map(|indexed| &indexed.manifest)
    }
}

impl Manifest {
    /// Parse one manifest from JSON.
    ///
    /// # Examples
    ///
    /// ```
    /// let registry = invoicekit_codelists::Registry::seeded().unwrap();
    /// let raw = serde_json::to_string(registry.manifests().next().unwrap()).unwrap();
    /// let parsed = invoicekit_codelists::Manifest::from_json(&raw).unwrap();
    /// assert!(parsed.verify().is_ok());
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when JSON parsing fails or the manifest is invalid.
    pub fn from_json(raw: &str) -> Result<Self, CodelistError> {
        let manifest: Self = serde_json::from_str(raw)?;
        manifest.verify()?;
        Ok(manifest)
    }
}

/// Errors emitted while loading or validating code-list manifests.
#[derive(Debug, Error)]
pub enum CodelistError {
    /// JSON parsing failed.
    #[error("code list manifest JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    /// A required text field is empty.
    #[error("code list manifest field `{field}` must not be empty")]
    EmptyField {
        /// Field name.
        field: &'static str,
    },
    /// 522z: a field value contains a character that the
    /// signing-payload format uses as a delimiter.
    #[error(
        "code list manifest field `{field}` contains separator character {character:?} \
         which would make the signing payload ambiguous"
    )]
    AmbiguousSeparator {
        /// Field name.
        field: &'static str,
        /// Offending character.
        character: char,
    },
    /// A date string is not a valid `YYYY-MM-DD` date.
    #[error("date `{date}` must be a valid YYYY-MM-DD date")]
    InvalidDate {
        /// Invalid date.
        date: String,
    },
    /// A validity window has its end before its start.
    #[error("date window for `{list}` is invalid: {from}..{to:?}")]
    InvalidDateWindow {
        /// List name.
        list: String,
        /// Start date.
        from: String,
        /// End date.
        to: Option<String>,
    },
    /// A manifest has no entries.
    #[error("code list `{list}` manifest must contain at least one entry")]
    EmptyManifest {
        /// List name.
        list: String,
    },
    /// A manifest repeats a code.
    #[error("code list `{list}` repeats entry code `{code}`")]
    DuplicateEntry {
        /// List name.
        list: String,
        /// Duplicate code.
        code: String,
    },
    /// Signature algorithm is not supported by this crate.
    #[error("code list `{list}` uses unsupported signature algorithm `{algorithm}`")]
    UnsupportedSignatureAlgorithm {
        /// List name.
        list: String,
        /// Unsupported algorithm.
        algorithm: String,
    },
    /// Signature does not match the manifest payload.
    #[error("code list `{list}` signature mismatch: expected {expected}, got {actual}")]
    SignatureMismatch {
        /// List name.
        list: String,
        /// Expected hex digest.
        expected: String,
        /// Actual hex digest.
        actual: String,
    },
}

fn require_non_empty(field: &'static str, value: &str) -> Result<(), CodelistError> {
    if value.trim().is_empty() {
        Err(CodelistError::EmptyField { field })
    } else {
        Ok(())
    }
}

/// 522z: reject characters that the line-and-pipe-separated
/// signing payload uses as field delimiters. Without this guard
/// a future upstream label containing `|` or a newline would
/// produce a digest payload that collides with a different
/// manifest.
fn require_payload_safe(field: &'static str, value: &str) -> Result<(), CodelistError> {
    for c in value.chars() {
        if matches!(c, '\n' | '\r' | '|' | ';' | '=') {
            return Err(CodelistError::AmbiguousSeparator {
                field,
                character: c,
            });
        }
    }
    Ok(())
}

fn is_within_window(on_date: &str, from: Option<&str>, to: Option<&str>) -> bool {
    if validate_date(on_date).is_err() {
        return false;
    }
    is_within_window_unchecked(on_date, from, to)
}

/// Window check that assumes `on_date` is already a validated `YYYY-MM-DD`
/// string. The bounds comparisons are lexicographic, which is order-equivalent
/// to chronological for the fixed-width ISO format, so this returns exactly what
/// [`is_within_window`] would for a valid date — it just skips the re-parse. The
/// registry lookup path validates the query date once and then uses this for
/// every per-manifest / per-entry window test.
fn is_within_window_unchecked(on_date: &str, from: Option<&str>, to: Option<&str>) -> bool {
    if from.is_some_and(|from| on_date < from) {
        return false;
    }
    if to.is_some_and(|to| on_date > to) {
        return false;
    }
    true
}

fn validate_date(date: &str) -> Result<(), CodelistError> {
    if !is_valid_date(date) {
        return Err(CodelistError::InvalidDate {
            date: date.to_owned(),
        });
    }
    Ok(())
}

fn is_valid_date(date: &str) -> bool {
    if date.len() != 10 {
        return false;
    }
    let bytes = date.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    if !bytes
        .iter()
        .enumerate()
        .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
    {
        return false;
    }

    let Ok(year) = date[0..4].parse::<u16>() else {
        return false;
    };
    let Ok(month) = date[5..7].parse::<u8>() else {
        return false;
    };
    let Ok(day) = date[8..10].parse::<u8>() else {
        return false;
    };

    year > 0 && (1..=12).contains(&month) && (1..=days_in_month(year, month)).contains(&day)
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    let max_len = left.len().max(right.len());

    for index in 0..max_len {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        diff |= usize::from(left_byte ^ right_byte);
    }

    diff == 0
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use proptest::test_runner::TestRunner;

    use super::*;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-codelists");
    }

    #[test]
    fn seeded_registry_contains_required_families() {
        let registry = Registry::seeded().expect("seed data verifies");
        let names = registry.list_names().collect::<BTreeSet<_>>();

        for required in [
            ISO_3166_1_ALPHA2,
            ISO_3166_2,
            ISO_4217,
            UNECE_REC20_UNITS,
            EN16931_VAT_CATEGORY,
            PEPPOL_INVOICE_TYPE,
            PEPPOL_PARTICIPANT_SCHEME,
        ] {
            assert!(
                names.contains(required),
                "{required} missing from seed registry"
            );
        }
    }

    #[test]
    fn lookup_finds_effective_entry() {
        let registry = Registry::seeded().expect("seed data verifies");
        let entry = registry
            .lookup(ISO_3166_1_ALPHA2, "DE", "2024-06-01")
            .expect("DE is seeded");

        assert_eq!(entry.label, "Germany");
        assert_eq!(entry.attrs.get("alpha3").map(String::as_str), Some("DEU"));
    }

    #[test]
    fn unknown_list_returns_none() {
        let registry = Registry::seeded().expect("seed data verifies");
        assert!(registry
            .lookup("missing-list", "DE", "2024-06-01")
            .is_none());
    }

    #[test]
    fn unknown_code_returns_none() {
        let registry = Registry::seeded().expect("seed data verifies");
        assert!(registry
            .lookup(ISO_3166_1_ALPHA2, "ZZ", "2024-06-01")
            .is_none());
    }

    #[test]
    fn date_before_manifest_window_returns_none() {
        let registry = Registry::seeded().expect("seed data verifies");
        assert!(registry.lookup(ISO_4217, "EUR", "2023-12-31").is_none());
    }

    #[test]
    fn date_after_entry_window_returns_none() {
        let manifest = signed_test_manifest(Some("2024-12-31"));
        let registry = Registry::from_manifests(vec![manifest]).expect("manifest verifies");

        assert!(registry.lookup("test-list", "OLD", "2025-01-01").is_none());
    }

    #[test]
    fn bad_manifest_signature_is_rejected() {
        let mut manifest = signed_test_manifest(None);
        manifest.signature = "bad".to_owned();

        let err = Registry::from_manifests(vec![manifest]).expect_err("bad signature rejected");
        assert!(matches!(err, CodelistError::SignatureMismatch { .. }));
    }

    #[test]
    fn unsupported_signature_algorithm_is_rejected() {
        let mut manifest = signed_test_manifest(None);
        manifest.signature_alg = "ed25519:detached".to_owned();

        let err = Registry::from_manifests(vec![manifest]).expect_err("unknown alg rejected");
        assert!(matches!(
            err,
            CodelistError::UnsupportedSignatureAlgorithm { .. }
        ));
    }

    // -------- 522z: ambiguous-separator regression tests --------

    #[test]
    fn label_with_pipe_is_rejected() {
        let mut manifest = signed_test_manifest(None);
        manifest.entries[0].label = "Old|test|label".to_owned();
        let err = manifest.verify().expect_err("label with `|` is ambiguous");
        assert!(
            matches!(
                err,
                CodelistError::AmbiguousSeparator {
                    field: "entry.label",
                    character: '|'
                }
            ),
            "unexpected: {err}"
        );
    }

    #[test]
    fn label_with_newline_is_rejected() {
        let mut manifest = signed_test_manifest(None);
        manifest.entries[0].label = "Old\ntest".to_owned();
        let err = manifest
            .verify()
            .expect_err("label with `\\n` is ambiguous");
        assert!(matches!(
            err,
            CodelistError::AmbiguousSeparator {
                field: "entry.label",
                character: '\n'
            }
        ));
    }

    #[test]
    fn version_with_equals_is_rejected() {
        let mut manifest = signed_test_manifest(None);
        manifest.version = "v=1.0".to_owned();
        let err = manifest
            .verify()
            .expect_err("version with `=` is ambiguous");
        assert!(matches!(
            err,
            CodelistError::AmbiguousSeparator {
                field: "version",
                character: '='
            }
        ));
    }

    #[test]
    fn attrs_value_with_semicolon_is_rejected() {
        let mut manifest = signed_test_manifest(None);
        manifest.entries[0]
            .attrs
            .insert("region".to_owned(), "EU;DACH".to_owned());
        let err = manifest
            .verify()
            .expect_err("attrs value with `;` is ambiguous");
        assert!(matches!(
            err,
            CodelistError::AmbiguousSeparator {
                field: "entry.attrs.value",
                character: ';'
            }
        ));
    }

    #[test]
    fn attrs_key_with_pipe_is_rejected() {
        let mut manifest = signed_test_manifest(None);
        manifest.entries[0]
            .attrs
            .insert("reg|ion".to_owned(), "EU".to_owned());
        let err = manifest
            .verify()
            .expect_err("attrs key with `|` is ambiguous");
        assert!(matches!(
            err,
            CodelistError::AmbiguousSeparator {
                field: "entry.attrs.key",
                character: '|'
            }
        ));
    }

    #[test]
    fn property_lookup_on_valid_seed_windows_returns_entry() {
        let registry = Registry::seeded().expect("seed data verifies");
        let cases = registry
            .manifests()
            .flat_map(|manifest| {
                manifest.entries.iter().map(|entry| {
                    (
                        manifest.list.clone(),
                        manifest.effective_from.clone(),
                        manifest.effective_to.clone(),
                        entry.clone(),
                    )
                })
            })
            .collect::<Vec<_>>();

        let mut runner = TestRunner::default();
        runner
            .run(
                &(0..cases.len(), 2024u16..=2027, 1u8..=12, 1u8..=31),
                |(index, year, month, day)| {
                    let (list, manifest_from, manifest_to, entry) = &cases[index];
                    let date = format!("{year:04}-{month:02}-{day:02}");
                    prop_assume!(is_valid_date(&date));
                    prop_assume!(is_within_window(
                        &date,
                        Some(manifest_from.as_str()),
                        manifest_to.as_deref(),
                    ));
                    prop_assume!(entry.is_effective_on(&date));

                    prop_assert_eq!(
                        registry
                            .lookup(list, &entry.code, &date)
                            .map(|entry| entry.code.as_str()),
                        Some(entry.code.as_str())
                    );
                    Ok(())
                },
            )
            .expect("seed lookup property holds");
    }

    #[test]
    fn property_lookup_outside_valid_seed_windows_returns_none() {
        let registry = Registry::seeded().expect("seed data verifies");
        let cases = registry
            .manifests()
            .flat_map(|manifest| {
                manifest.entries.iter().map(|entry| {
                    (
                        manifest.list.clone(),
                        manifest.effective_from.clone(),
                        manifest.effective_to.clone(),
                        entry.clone(),
                    )
                })
            })
            .collect::<Vec<_>>();

        let mut runner = TestRunner::default();
        runner
            .run(
                &(0..cases.len(), 2023u16..=2023, 1u8..=12, 1u8..=31),
                |(index, year, month, day)| {
                    let (list, _manifest_from, _manifest_to, entry) = &cases[index];
                    let date = format!("{year:04}-{month:02}-{day:02}");
                    prop_assume!(is_valid_date(&date));
                    prop_assert_eq!(
                        registry
                            .lookup(list, &entry.code, &date)
                            .map(|entry| entry.code.as_str()),
                        None
                    );
                    Ok(())
                },
            )
            .expect("seed lookup outside windows property holds");
    }

    fn signed_test_manifest(entry_valid_to: Option<&str>) -> Manifest {
        let mut manifest = Manifest {
            list: "test-list".to_owned(),
            version: "test-2024".to_owned(),
            effective_from: "2024-01-01".to_owned(),
            effective_to: None,
            source_url: "https://example.invalid/test-list".to_owned(),
            retrieved_at: "2024-01-01".to_owned(),
            signature_alg: "sha256:identity".to_owned(),
            signature: String::new(),
            entries: vec![Entry {
                code: "OLD".to_owned(),
                label: "Old test code".to_owned(),
                valid_from: Some("2024-01-01".to_owned()),
                valid_to: entry_valid_to.map(ToOwned::to_owned),
                attrs: BTreeMap::new(),
            }],
        };
        manifest.signature = manifest.expected_signature();
        manifest
    }
}
