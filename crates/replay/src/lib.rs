// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-replay` — replay-from-bundle audit/debug feature.
//!
//! Re-runs the pipeline recorded in a [`EvidenceBundle`] and
//! reports whether the freshly-produced artefacts are
//! byte-equal to the originally-recorded artefacts, or
//! produces a structured diff.
//!
//! The library is engine-agnostic: it accepts a
//! [`PipelineReplayer`] trait that the eventual T-100
//! `invoicekit replay` subcommand wires up to the real engine
//! crate. Tests use [`IdentityReplayer`] (re-emits each input
//! artefact unchanged) + [`MutatingReplayer`] (deliberately
//! drifts) to exercise both paths without dragging the full
//! engine into the test target.
//!
//! Public surface:
//! [`PipelineReplayer`], [`ReplayReport`], [`ArtefactDelta`],
//! [`ReplayOptions`], [`replay`].
//!
//! Plain-English version of the contract:
//! given an audit `.invoicekit` bundle and the same engine
//! version, replay produces byte-identical output. Any diff
//! means either the engine changed behaviour or the bundle
//! was tampered with after recording — both are operator
//! signals worth alerting on.

use std::collections::{BTreeMap, BTreeSet};

use invoicekit_evidence::{blake3_hex, EvidenceBundle};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Set of artefact ids the replay should diff. Empty means
/// "diff every artefact the bundle carries".
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplayOptions {
    /// Artefact ids to include. Empty means all.
    pub only: BTreeSet<String>,
    /// Artefact ids to skip. Applied after `only`. Useful
    /// when the operator wants to re-render the PDF but
    /// ignore the gateway receipts.
    pub ignore: BTreeSet<String>,
}

impl ReplayOptions {
    /// Build options that replay every artefact in the bundle.
    #[must_use]
    pub fn all() -> Self {
        Self::default()
    }

    /// Build options that replay only the listed artefact ids.
    #[must_use]
    pub fn only(ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            only: ids.into_iter().map(Into::into).collect(),
            ignore: BTreeSet::new(),
        }
    }

    /// Add an ignore-list entry; chains.
    #[must_use]
    pub fn ignoring(mut self, id: impl Into<String>) -> Self {
        self.ignore.insert(id.into());
        self
    }

    fn includes(&self, id: &str) -> bool {
        if !self.only.is_empty() && !self.only.contains(id) {
            return false;
        }
        if self.ignore.contains(id) {
            return false;
        }
        true
    }
}

/// Per-artefact diff verdict.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ArtefactDelta {
    /// Re-emitted bytes are byte-equal to the recorded bytes.
    ByteEqual {
        /// BLAKE3 hash of the bytes (lowercase hex).
        blake3_hex: String,
    },
    /// Re-emitted bytes differ. Hex hashes recorded so the
    /// audit UI can show "expected ABC, got DEF" without
    /// re-hashing on the read path.
    Drifted {
        /// BLAKE3 hash of the originally-recorded bytes.
        expected_blake3_hex: String,
        /// BLAKE3 hash of the re-emitted bytes.
        observed_blake3_hex: String,
        /// Recorded payload length.
        expected_size: u64,
        /// Re-emitted payload length.
        observed_size: u64,
    },
    /// The replayer did not emit a payload for this artefact
    /// id. Typical when the replayer is intentionally narrow
    /// (e.g. re-render only the PDF).
    NotReplayed,
    /// The replayer emitted an artefact whose id is not in the
    /// recorded bundle. Surfaces engine drift toward emitting
    /// new artefact kinds.
    Unexpected {
        /// BLAKE3 hash of the new payload.
        observed_blake3_hex: String,
        /// Length of the new payload.
        observed_size: u64,
    },
}

impl ArtefactDelta {
    /// True only when this delta indicates byte-equality.
    #[must_use]
    pub const fn is_byte_equal(&self) -> bool {
        matches!(self, Self::ByteEqual { .. })
    }
    /// True when this delta indicates the replayer disagreed
    /// with the recorded bundle (drift / unexpected / missing
    /// where one was expected).
    #[must_use]
    pub const fn is_diff(&self) -> bool {
        matches!(self, Self::Drifted { .. } | Self::Unexpected { .. })
    }
}

/// Aggregate replay report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplayReport {
    /// True iff every artefact that was replayed is byte-equal
    /// (or skipped); any diff pulls this to false.
    pub ok: bool,
    /// Per-artefact verdicts keyed by id, in lexicographic
    /// order so the JSON output is stable across runs.
    pub deltas: BTreeMap<String, ArtefactDelta>,
}

impl ReplayReport {
    /// Iterator over the artefact ids that diverged from the
    /// recorded bundle.
    pub fn drifted_ids(&self) -> impl Iterator<Item = &str> {
        self.deltas
            .iter()
            .filter(|(_, d)| d.is_diff())
            .map(|(id, _)| id.as_str())
    }
}

/// Errors that prevent replay from running.
#[derive(Debug, Error)]
pub enum ReplayError {
    /// The injected replayer returned an internal error.
    #[error("replayer error: {0}")]
    Replayer(String),
}

/// Replay surface.
///
/// The library calls [`PipelineReplayer::replay_artefact`]
/// once per recorded artefact id (subject to [`ReplayOptions`])
/// and reconciles the returned bytes with the recorded bytes.
/// The eventual T-100 wiring uses the real engine crate behind
/// this trait; tests inject deterministic stubs.
pub trait PipelineReplayer {
    /// Re-produce the artefact bytes for `artefact_id` given
    /// the bundle as input.
    ///
    /// Return `Ok(Some(bytes))` to surface those bytes for
    /// diff; `Ok(None)` to declare "I don't replay this kind"
    /// (which the library maps to [`ArtefactDelta::NotReplayed`]);
    /// `Err` for transport / engine errors that should fail
    /// the whole replay.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayError::Replayer`] when the replayer's
    /// backing engine refuses or errors.
    fn replay_artefact(
        &self,
        bundle: &EvidenceBundle,
        artefact_id: &str,
    ) -> Result<Option<Vec<u8>>, ReplayError>;

    /// Optionally produce a set of artefact ids the replayer
    /// would emit that the bundle does not record (engine
    /// drift). The default returns the empty set.
    fn extra_artefacts(&self, _bundle: &EvidenceBundle) -> BTreeMap<String, Vec<u8>> {
        BTreeMap::new()
    }
}

/// Run replay and produce a [`ReplayReport`].
///
/// # Errors
///
/// Returns [`ReplayError`] when the replayer's backing engine
/// refuses on any artefact. Drift, missing payloads, and
/// unexpected payloads do **not** raise — they're recorded as
/// [`ArtefactDelta`] entries on the report.
pub fn replay(
    bundle: &EvidenceBundle,
    replayer: &dyn PipelineReplayer,
    options: &ReplayOptions,
) -> Result<ReplayReport, ReplayError> {
    let mut deltas: BTreeMap<String, ArtefactDelta> = BTreeMap::new();
    for (id, recorded) in &bundle.artefacts {
        if !options.includes(id) {
            continue;
        }
        let Some(observed) = replayer.replay_artefact(bundle, id)? else {
            deltas.insert(id.clone(), ArtefactDelta::NotReplayed);
            continue;
        };
        let observed_hex = blake3_hex(&observed);
        let expected_hex = blake3_hex(recorded);
        let delta = if expected_hex == observed_hex {
            ArtefactDelta::ByteEqual {
                blake3_hex: observed_hex,
            }
        } else {
            ArtefactDelta::Drifted {
                expected_blake3_hex: expected_hex,
                observed_blake3_hex: observed_hex,
                expected_size: recorded.len() as u64,
                observed_size: observed.len() as u64,
            }
        };
        deltas.insert(id.clone(), delta);
    }
    // Surface engine-emitted artefacts that the bundle did not
    // record. Subject to the same include/ignore filter so
    // operators can scope the report.
    for (id, observed) in replayer.extra_artefacts(bundle) {
        if !options.includes(&id) {
            continue;
        }
        if bundle.artefacts.contains_key(&id) {
            // Already handled in the recorded loop above.
            continue;
        }
        deltas.insert(
            id.clone(),
            ArtefactDelta::Unexpected {
                observed_blake3_hex: blake3_hex(&observed),
                observed_size: observed.len() as u64,
            },
        );
    }
    let ok = deltas.values().all(|d| !d.is_diff());
    Ok(ReplayReport { ok, deltas })
}

/// Trivial replayer that returns each recorded artefact
/// unchanged. Used by tests + by the cassette-replay sandbox
/// to verify the replay machinery itself.
pub struct IdentityReplayer;

impl PipelineReplayer for IdentityReplayer {
    fn replay_artefact(
        &self,
        bundle: &EvidenceBundle,
        artefact_id: &str,
    ) -> Result<Option<Vec<u8>>, ReplayError> {
        Ok(bundle.artefacts.get(artefact_id).cloned())
    }
}

/// Mutating replayer that drifts the listed artefact ids by
/// appending a deterministic suffix. Used by tests to exercise
/// the drift-detection path without spinning up a real engine.
pub struct MutatingReplayer {
    /// Artefact ids to drift.
    pub drift: BTreeSet<String>,
    /// Extra artefact ids to emit that the bundle did not
    /// record.
    pub extra: BTreeMap<String, Vec<u8>>,
}

impl MutatingReplayer {
    /// Build a mutating replayer that drifts every artefact id
    /// in the supplied iterator.
    #[must_use]
    pub fn drifting(ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            drift: ids.into_iter().map(Into::into).collect(),
            extra: BTreeMap::new(),
        }
    }

    /// Add an extra artefact the replayer will emit even though
    /// the bundle does not record it.
    #[must_use]
    pub fn with_extra(mut self, id: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        self.extra.insert(id.into(), bytes.into());
        self
    }
}

impl PipelineReplayer for MutatingReplayer {
    fn replay_artefact(
        &self,
        bundle: &EvidenceBundle,
        artefact_id: &str,
    ) -> Result<Option<Vec<u8>>, ReplayError> {
        let Some(recorded) = bundle.artefacts.get(artefact_id) else {
            return Ok(None);
        };
        if self.drift.contains(artefact_id) {
            let mut drifted = recorded.clone();
            drifted.extend_from_slice(b"--mutated");
            Ok(Some(drifted))
        } else {
            Ok(Some(recorded.clone()))
        }
    }

    fn extra_artefacts(&self, _bundle: &EvidenceBundle) -> BTreeMap<String, Vec<u8>> {
        self.extra.clone()
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_replay::crate_name(), "invoicekit-replay");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-replay"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_evidence::manifest_for;

    fn sample_bundle() -> EvidenceBundle {
        let mut artefacts: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        artefacts.insert("canonical.json".to_owned(), br#"{"id":"INV-1"}"#.to_vec());
        artefacts.insert("formats/ubl.xml".to_owned(), b"<Invoice/>".to_vec());
        artefacts.insert(
            "formats/cii.xml".to_owned(),
            b"<CrossIndustryInvoice/>".to_vec(),
        );
        artefacts.insert(
            "receipts/peppol.json".to_owned(),
            br#"{"message_id":"msg-1"}"#.to_vec(),
        );
        let manifest = manifest_for(&artefacts, "tenant-a", "trace-1", "2026-05-27T00:00:00Z");
        EvidenceBundle {
            manifest,
            artefacts,
        }
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-replay");
    }

    #[test]
    fn identity_replayer_reports_byte_equal_for_every_artefact() {
        let bundle = sample_bundle();
        let report = replay(&bundle, &IdentityReplayer, &ReplayOptions::all()).unwrap();
        assert!(report.ok);
        for (id, delta) in &report.deltas {
            assert!(delta.is_byte_equal(), "drifted: {id} -> {delta:?}");
        }
        assert_eq!(report.deltas.len(), bundle.artefacts.len());
        assert_eq!(report.drifted_ids().count(), 0);
    }

    #[test]
    fn mutating_replayer_surfaces_drift_for_targeted_ids() {
        let bundle = sample_bundle();
        let replayer = MutatingReplayer::drifting(["formats/ubl.xml".to_owned()]);
        let report = replay(&bundle, &replayer, &ReplayOptions::all()).unwrap();
        assert!(!report.ok);
        let drifted: Vec<&str> = report.drifted_ids().collect();
        assert_eq!(drifted, vec!["formats/ubl.xml"]);
        // Other artefacts should still report byte-equal.
        assert!(report.deltas["canonical.json"].is_byte_equal());
    }

    #[test]
    fn only_filter_scopes_replay_to_named_ids() {
        let bundle = sample_bundle();
        let report = replay(
            &bundle,
            &IdentityReplayer,
            &ReplayOptions::only(["canonical.json"]),
        )
        .unwrap();
        assert_eq!(
            report.deltas.keys().collect::<Vec<_>>(),
            vec!["canonical.json"]
        );
    }

    #[test]
    fn ignore_filter_skips_named_ids() {
        let bundle = sample_bundle();
        let report = replay(
            &bundle,
            &IdentityReplayer,
            &ReplayOptions::all().ignoring("receipts/peppol.json"),
        )
        .unwrap();
        assert!(!report.deltas.contains_key("receipts/peppol.json"));
        assert!(report.deltas.contains_key("canonical.json"));
    }

    #[test]
    fn extra_artefacts_surface_as_unexpected() {
        let bundle = sample_bundle();
        let replayer =
            MutatingReplayer::drifting(Vec::<String>::new()).with_extra("formats/jpk.xml", b"<x/>");
        let report = replay(&bundle, &replayer, &ReplayOptions::all()).unwrap();
        assert!(!report.ok);
        match &report.deltas["formats/jpk.xml"] {
            ArtefactDelta::Unexpected { observed_size, .. } => assert_eq!(*observed_size, 4),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn not_replayed_records_when_replayer_returns_none() {
        struct PartialReplayer;
        impl PipelineReplayer for PartialReplayer {
            fn replay_artefact(
                &self,
                _bundle: &EvidenceBundle,
                _artefact_id: &str,
            ) -> Result<Option<Vec<u8>>, ReplayError> {
                Ok(None)
            }
        }
        let bundle = sample_bundle();
        let report = replay(&bundle, &PartialReplayer, &ReplayOptions::all()).unwrap();
        assert!(report.ok, "report: {report:?}");
        for delta in report.deltas.values() {
            assert!(matches!(delta, ArtefactDelta::NotReplayed));
        }
    }

    #[test]
    fn replayer_error_surfaces_as_replay_error() {
        struct ErroringReplayer;
        impl PipelineReplayer for ErroringReplayer {
            fn replay_artefact(
                &self,
                _bundle: &EvidenceBundle,
                _artefact_id: &str,
            ) -> Result<Option<Vec<u8>>, ReplayError> {
                Err(ReplayError::Replayer("engine offline".to_owned()))
            }
        }
        let bundle = sample_bundle();
        let err = replay(&bundle, &ErroringReplayer, &ReplayOptions::all()).unwrap_err();
        match err {
            ReplayError::Replayer(msg) => assert!(msg.contains("engine offline")),
        }
    }

    #[test]
    fn report_round_trips_through_json() {
        let bundle = sample_bundle();
        let report = replay(&bundle, &IdentityReplayer, &ReplayOptions::all()).unwrap();
        let json = serde_json::to_string(&report).unwrap();
        let back: ReplayReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
    }

    #[test]
    fn drift_delta_is_marked_as_diff_not_byte_equal() {
        let drift = ArtefactDelta::Drifted {
            expected_blake3_hex: "a".to_owned(),
            observed_blake3_hex: "b".to_owned(),
            expected_size: 1,
            observed_size: 2,
        };
        assert!(drift.is_diff());
        assert!(!drift.is_byte_equal());
        let unexpected = ArtefactDelta::Unexpected {
            observed_blake3_hex: "c".to_owned(),
            observed_size: 3,
        };
        assert!(unexpected.is_diff());
        let not_replayed = ArtefactDelta::NotReplayed;
        assert!(!not_replayed.is_diff());
        assert!(!not_replayed.is_byte_equal());
    }
}
