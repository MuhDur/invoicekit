// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit-migration` — IR schema evolution and forward migration.
//!
//! When the [`invoicekit_ir::SchemaVersion`] family grows a new variant the
//! previously archived invoices on customer disks must keep working. This
//! crate carries the typed forward-migration framework that takes a JSON
//! document tagged with a known old version and lifts it to the requested
//! newer version, recording every field the migration could not lift
//! cleanly in a [`MigrationReport`].
//!
//! ## Today's reality
//!
//! The IR currently exposes exactly one variant — [`SchemaVersion::V1_0`].
//! No two-version pair exists yet, so every well-formed migration call is
//! either a no-op identity (`V1_0 → V1_0`) or an explicit
//! [`MigrationError::UnknownTargetVersion`]. The framework, the typed
//! report, the reversibility marker, and the [`Registry`] ship today so
//! that the day a `V1_1` or `V2_0` variant lands, only a single
//! [`Migration`] implementation needs to be written and registered.
//! This crate is library-only: it ships no command-line interface and no
//! binary.
//!
//! ## API shape
//!
//! ```
//! use invoicekit_migration::migrate;
//! use invoicekit_ir::SchemaVersion;
//! use serde_json::json;
//!
//! let doc = json!({"schema_version": "1.0", "id": "doc-1"});
//! let (migrated, report) = migrate(doc, SchemaVersion::V1_0).unwrap();
//! assert!(report.is_clean());
//! assert_eq!(migrated["schema_version"], "1.0");
//! ```

use invoicekit_ir::SchemaVersion;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Per-field migration outcome appended to a [`MigrationReport`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MigrationFinding {
    /// JSON Pointer to the source field that could not migrate cleanly.
    pub path: String,
    /// Short machine-friendly code (`field-dropped`, `value-coerced`, ...).
    pub kind: String,
    /// Human-readable explanation.
    pub message: String,
    /// Optional remediation hint shown to operators and autofixers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

/// Summary returned by every successful [`migrate`] call.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MigrationReport {
    /// Source IR version of the document.
    pub from: SchemaVersion,
    /// Destination IR version of the document.
    pub to: SchemaVersion,
    /// Did this migration preserve enough information that the inverse
    /// migration would round-trip? Identity migrations are always
    /// reversible.
    pub reversible: bool,
    /// Field-level findings: empty for an identity migration, populated
    /// when the migration drops or coerces information.
    pub findings: Vec<MigrationFinding>,
}

impl MigrationReport {
    /// True when no findings were recorded — the migration round-trips
    /// the document with no information loss.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// Errors emitted by the migration framework.
#[derive(Debug, Error)]
pub enum MigrationError {
    /// The source document does not carry a `schema_version` field.
    #[error(
        "source document is missing `schema_version`; hint: every InvoiceKit invoice tags itself with a SchemaVersion at the root"
    )]
    MissingSourceVersion,
    /// The source document's `schema_version` is not a known [`SchemaVersion`].
    #[error(
        "source document carries unknown schema_version `{0}`; hint: upgrade `invoicekit-ir` or check the document's origin"
    )]
    UnknownSourceVersion(String),
    /// The caller requested a target [`SchemaVersion`] that no migration path can reach.
    #[error(
        "no migration path from `{from:?}` to `{to:?}`; hint: register a `Migration` implementation between these two versions"
    )]
    UnknownTargetVersion {
        /// Source version.
        from: SchemaVersion,
        /// Requested target version.
        to: SchemaVersion,
    },
    /// JSON decoding failed.
    #[error("migration JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// One concrete migration step (`from` → `to`).
///
/// The framework wires migrations into a directed graph keyed on
/// `(from, to)`; [`Registry::migrate`] picks the migration whose
/// `target_version` matches the requested target. The trait is intentionally
/// narrow — it owns the per-field transformation logic only — so that
/// adding a new IR version is a self-contained PR.
pub trait Migration: Send + Sync {
    /// Source version this migration accepts.
    fn source_version(&self) -> SchemaVersion;
    /// Destination version this migration produces.
    fn target_version(&self) -> SchemaVersion;
    /// Whether the produced document can round-trip back to its source
    /// without information loss (no fields were dropped or coerced).
    fn reversible(&self) -> bool;
    /// Lift `value` from `source_version` to `target_version`, appending any
    /// field-level findings to `report`.
    ///
    /// # Errors
    ///
    /// Implementations return [`MigrationError`] on JSON decoding failure
    /// or when the input document does not match the expected shape for
    /// `source_version`.
    fn apply(&self, value: Value, report: &mut MigrationReport) -> Result<Value, MigrationError>;
}

/// Identity migration `V1_0 → V1_0`. Always reversible, never lossy.
pub struct IdentityV1ToV1;

impl Migration for IdentityV1ToV1 {
    fn source_version(&self) -> SchemaVersion {
        SchemaVersion::V1_0
    }
    fn target_version(&self) -> SchemaVersion {
        SchemaVersion::V1_0
    }
    fn reversible(&self) -> bool {
        true
    }
    fn apply(&self, value: Value, _report: &mut MigrationReport) -> Result<Value, MigrationError> {
        Ok(value)
    }
}

/// In-memory registry of known [`Migration`] implementations.
///
/// The built-in registry returned by [`Registry::seeded`] knows the
/// identity migration today; new migrations are added by passing them to
/// [`Registry::register`] before calling [`Registry::migrate`].
///
/// Implementation note: `SchemaVersion` does not derive `Ord` or `Hash`,
/// so the registry stores migrations in insertion order in a `Vec` and
/// scans linearly. The number of distinct migrations is bounded by the
/// number of IR versions ever shipped, which we expect to stay in the
/// low single digits across the lifetime of InvoiceKit.
#[derive(Default)]
pub struct Registry {
    migrations: Vec<Box<dyn Migration>>,
}

impl Registry {
    /// Build a registry pre-loaded with every migration InvoiceKit ships.
    #[must_use]
    pub fn seeded() -> Self {
        let mut registry = Self::default();
        registry.register(Box::new(IdentityV1ToV1));
        registry
    }

    /// Register one migration step in the registry.
    pub fn register(&mut self, migration: Box<dyn Migration>) {
        self.migrations.push(migration);
    }

    /// Migrate `value` from its declared `schema_version` up to `target`.
    ///
    /// Picks the first registered migration whose `target_version` equals
    /// `target`. Returns `(migrated_value, report)`.
    ///
    /// # Errors
    ///
    /// Returns [`MigrationError`] for missing/unknown source version,
    /// no known path, or JSON decoding failure.
    pub fn migrate(
        &self,
        value: Value,
        target: SchemaVersion,
    ) -> Result<(Value, MigrationReport), MigrationError> {
        let source = read_source_version(&value)?;
        let mut report = MigrationReport {
            from: source,
            to: target,
            reversible: true,
            findings: Vec::new(),
        };

        let migration = self
            .pick(source, target)
            .ok_or(MigrationError::UnknownTargetVersion {
                from: source,
                to: target,
            })?;
        let out = migration.apply(value, &mut report)?;
        report.reversible = migration.reversible();
        Ok((out, report))
    }

    fn pick(&self, from: SchemaVersion, to: SchemaVersion) -> Option<&dyn Migration> {
        self.migrations
            .iter()
            .find(|m| m.source_version() == from && m.target_version() == to)
            .map(AsRef::as_ref)
    }
}

/// Convenience: migrate a JSON `Value` using the default seeded [`Registry`].
///
/// # Errors
///
/// Same as [`Registry::migrate`].
///
/// # Examples
///
/// ```
/// use invoicekit_ir::SchemaVersion;
/// use invoicekit_migration::migrate;
/// use serde_json::json;
///
/// let (out, report) = migrate(json!({"schema_version": "1.0"}), SchemaVersion::V1_0).unwrap();
/// assert!(report.is_clean());
/// assert_eq!(out["schema_version"], "1.0");
/// ```
pub fn migrate(
    value: Value,
    target: SchemaVersion,
) -> Result<(Value, MigrationReport), MigrationError> {
    Registry::seeded().migrate(value, target)
}

fn read_source_version(value: &Value) -> Result<SchemaVersion, MigrationError> {
    let raw = value
        .get("schema_version")
        .and_then(Value::as_str)
        .ok_or(MigrationError::MissingSourceVersion)?;
    // SchemaVersion derives Deserialize via serde(rename = "1.0"), so we
    // can route the string through serde_json to get a typed value.
    serde_json::from_value::<SchemaVersion>(Value::String(raw.to_owned()))
        .map_err(|_| MigrationError::UnknownSourceVersion(raw.to_owned()))
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_migration::crate_name(), "invoicekit-migration");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-migration"
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-migration");
    }

    #[test]
    fn identity_migration_round_trips_a_v1_doc() {
        let input = json!({
            "schema_version": "1.0",
            "id": "doc-1",
            "marker": "untouched",
        });
        let (output, report) = migrate(input.clone(), SchemaVersion::V1_0).unwrap();
        assert_eq!(output, input);
        assert!(report.is_clean());
        assert!(report.reversible);
        assert_eq!(report.from, SchemaVersion::V1_0);
        assert_eq!(report.to, SchemaVersion::V1_0);
    }

    #[test]
    fn missing_schema_version_is_rejected() {
        let err = migrate(json!({"id": "no-version"}), SchemaVersion::V1_0).unwrap_err();
        assert!(matches!(err, MigrationError::MissingSourceVersion));
    }

    #[test]
    fn unknown_source_version_is_rejected() {
        let err = migrate(json!({"schema_version": "99.0"}), SchemaVersion::V1_0).unwrap_err();
        assert!(matches!(err, MigrationError::UnknownSourceVersion(_)));
    }

    #[test]
    fn report_is_clean_when_no_findings_recorded() {
        let report = MigrationReport::default();
        assert!(report.is_clean());
    }

    #[test]
    fn finding_marks_report_unclean() {
        let mut report = MigrationReport::default();
        report.findings.push(MigrationFinding {
            path: "/extensions/0/payload/legacy_field".to_owned(),
            kind: "field-dropped".to_owned(),
            message: "Field was removed in v2_0".to_owned(),
            remediation: Some("Re-emit the document using the v2 IR.".to_owned()),
        });
        assert!(!report.is_clean());
    }

    #[test]
    fn registry_picks_registered_migration() {
        let registry = Registry::seeded();
        let (out, report) = registry
            .migrate(json!({"schema_version": "1.0"}), SchemaVersion::V1_0)
            .unwrap();
        assert_eq!(out["schema_version"], "1.0");
        assert!(report.reversible);
    }

    proptest! {
        /// Identity migration on any well-formed v1 document produces the
        /// same document and a clean report.
        #[test]
        fn identity_is_a_function(seed in 0_u32..=10_000) {
            let input = json!({
                "schema_version": "1.0",
                "seed": seed,
                "nested": {"a": 1, "b": [seed, seed.wrapping_add(1)]},
            });
            let (output, report) = migrate(input.clone(), SchemaVersion::V1_0).unwrap();
            prop_assert_eq!(output, input);
            prop_assert!(report.is_clean());
            prop_assert!(report.reversible);
        }
    }
}
