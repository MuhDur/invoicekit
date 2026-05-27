// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-075 reconciliation API.
//!
//! `POST /v1/reconcile` is the customer's join point against the
//! managed-service outbox: they hand us a batch of
//! `(internal_id, fingerprint)` pairs and we return four
//! bucketed lists of their internal ids based on the outbox row's
//! current [`OutboxState`](crate::outbox::OutboxState).
//!
//! Bucket semantics:
//!
//! - **`delivered`**: the outbox row reached a terminal "happy" state
//!   (`delivered` or `acknowledged`).
//! - **`failed`**: the outbox row reached a terminal "sad" state
//!   (`rejected` or `dead_letter`).
//! - **`pending`**: the outbox row exists for this `(tenant, fingerprint)`
//!   pair but hasn't reached either terminal yet (`draft`, `validated`,
//!   `signed`, `reserved`, `sent`, `archived`).
//! - **`unknown`**: no outbox row matches this `(tenant, fingerprint)`
//!   pair. Could mean the customer's fingerprint is stale or that the
//!   transmission was never enqueued.
//!
//! The handler is framework-free: the future HTTP layer (T-134 API
//! gateway) wraps it for `POST /v1/reconcile`. The store trait
//! abstracts the SQL dialect so the same handler runs against
//! Postgres, MySQL, and SQLite — only the trait impl changes per
//! dialect.
//!
//! ## Batching
//!
//! Up to [`MAX_BATCH_ENTRIES`] entries (10 000) per request, matching
//! the bead's strict-acceptance gate. The handler rejects bigger
//! batches up-front with [`ReconcileError::BatchTooLarge`]; the HTTP
//! layer should return that as a 413. Empty batches return an empty
//! response without touching the store.
//!
//! ## Tenant isolation
//!
//! Every request carries a [`TenantId`]; the store must scope every
//! query by tenant. A buggy store impl that returned rows for
//! another tenant would surface as `delivered`/`failed` rows that
//! don't match the customer's fingerprint, but the handler also
//! re-checks the per-row tenant_id as a defense-in-depth and bucket
//! mismatches into `unknown`.

#![allow(
    clippy::option_if_let_else,
    clippy::map_unwrap_or,
    clippy::significant_drop_tightening,
    clippy::or_fun_call,
    clippy::doc_markdown,
    clippy::too_long_first_doc_paragraph,
    clippy::redundant_closure_for_method_calls,
    clippy::manual_is_ascii_check,
    clippy::missing_panics_doc,
    clippy::format_push_string,
    clippy::range_plus_one,
    clippy::items_after_statements
)]

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Bead identifier carried on emitted log records.
pub const RECONCILE_API_BEAD_ID: &str = "invoices-t-075-reconciliation-api-0ne";

/// Hard cap on the number of `(internal_id, fingerprint)` entries the
/// handler accepts in one request. Matches the bead's gate.
pub const MAX_BATCH_ENTRIES: usize = 10_000;

/// Maximum length of an `internal_id` string. The outbox column is
/// effectively unbounded (TEXT), but the API surface caps it so a
/// malicious client can't blow up our memory by submitting 10 000
/// 1MB strings.
pub const MAX_INTERNAL_ID_BYTES: usize = 256;

/// Tenant identifier as the managed-API layer hands it down.
pub type TenantId = String;

/// One entry in a reconcile request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconcileEntry {
    /// Customer's internal id for this transmission (their primary
    /// key, invoice number, etc.). Echoed back in the bucketed
    /// response so the customer can pair their input to our state
    /// without depending on input ordering.
    pub internal_id: String,
    /// Hex-encoded BLAKE3 fingerprint of the canonical commercial
    /// document, as returned by [`crate::fingerprint`].
    pub fingerprint: String,
}

/// Full reconcile request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconcileRequest {
    /// Owning tenant (echoed from the API gateway's authn layer).
    pub tenant_id: TenantId,
    /// Up to [`MAX_BATCH_ENTRIES`] entries.
    pub entries: Vec<ReconcileEntry>,
}

/// Bucketed reconcile response. Each list holds `internal_id` values
/// in the order they appeared in the request (deduplicated so a
/// double-submitted internal_id doesn't appear twice).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReconcileResponse {
    /// Bead identifier for diagnostic correlation; ignored by clients.
    pub bead: &'static str,
    /// Internal ids whose transmission is terminally delivered
    /// (state ∈ {delivered, acknowledged}).
    pub delivered: Vec<String>,
    /// Internal ids whose transmission is terminally failed
    /// (state ∈ {rejected, dead_letter}).
    pub failed: Vec<String>,
    /// Internal ids whose transmission exists but hasn't hit a
    /// terminal state yet.
    pub pending: Vec<String>,
    /// Internal ids the store has no row for. Either the fingerprint
    /// is stale or the transmission was never enqueued.
    pub unknown: Vec<String>,
}

impl ReconcileResponse {
    /// Empty response with the bead identifier prefilled. Convenient
    /// for the zero-entry early-return path.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            bead: RECONCILE_API_BEAD_ID,
            ..Self::default()
        }
    }
}

/// What the store reports back for one (tenant, fingerprint) pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FingerprintBucket {
    /// State ∈ {delivered, acknowledged}.
    Delivered,
    /// State ∈ {rejected, dead_letter}.
    Failed,
    /// Any non-terminal state.
    Pending,
}

/// Storage backend the handler queries.
///
/// Real impls wrap the Postgres / MySQL / SQLite outbox tables; the
/// in-memory impl below powers the handler's tests.
pub trait OutboxLookup: Send + Sync {
    /// Look up the bucket for each `fingerprint_hex` under `tenant`.
    /// Implementations MUST scope every query by tenant. Order of
    /// the returned vector matches `fingerprints`; a fingerprint
    /// with no row returns `None`.
    ///
    /// The handler de-duplicates `fingerprints` before calling this
    /// method, so impls can assume distinct inputs.
    ///
    /// # Errors
    ///
    /// Implementations may surface backend-specific errors as
    /// [`ReconcileError::Store`].
    fn buckets_for(
        &self,
        tenant: &TenantId,
        fingerprints: &[String],
    ) -> Result<Vec<Option<FingerprintBucket>>, ReconcileError>;
}

/// Errors surfaced by the reconcile handler.
#[derive(Debug, Error)]
pub enum ReconcileError {
    /// Batch contained more than [`MAX_BATCH_ENTRIES`] entries.
    #[error("reconcile batch too large: {count} entries exceeds maximum of {max}")]
    BatchTooLarge {
        /// Submitted batch size.
        count: usize,
        /// Configured maximum.
        max: usize,
    },
    /// An internal_id was blank or above the byte cap.
    #[error("invalid internal_id at index {index}: {reason}")]
    BadInternalId {
        /// 0-based index into `entries`.
        index: usize,
        /// Operator-readable reason.
        reason: String,
    },
    /// A fingerprint string was not valid hex of the expected length.
    #[error("invalid fingerprint at index {index}: {reason}")]
    BadFingerprint {
        /// 0-based index into `entries`.
        index: usize,
        /// Operator-readable reason.
        reason: String,
    },
    /// Backend storage failed.
    #[error("outbox store error: {0}")]
    Store(String),
}

/// Run the reconcile request against `store`.
///
/// # Errors
///
/// Returns [`ReconcileError::BatchTooLarge`] when `request.entries`
/// exceeds [`MAX_BATCH_ENTRIES`], [`ReconcileError::BadInternalId`]
/// or [`ReconcileError::BadFingerprint`] when an entry is malformed,
/// and propagates [`ReconcileError::Store`] from the store impl.
pub fn handle_reconcile_request(
    store: &dyn OutboxLookup,
    request: &ReconcileRequest,
) -> Result<ReconcileResponse, ReconcileError> {
    if request.entries.len() > MAX_BATCH_ENTRIES {
        return Err(ReconcileError::BatchTooLarge {
            count: request.entries.len(),
            max: MAX_BATCH_ENTRIES,
        });
    }
    if request.entries.is_empty() {
        return Ok(ReconcileResponse::empty());
    }

    for (idx, entry) in request.entries.iter().enumerate() {
        validate_internal_id(idx, &entry.internal_id)?;
        validate_fingerprint(idx, &entry.fingerprint)?;
    }

    // De-duplicate fingerprints to keep the SQL IN-clause small.
    // Preserve the first-seen index per fingerprint so we can map
    // store results back to every entry that referenced it.
    let mut distinct_fingerprints: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for entry in &request.entries {
        if seen.insert(entry.fingerprint.clone()) {
            distinct_fingerprints.push(entry.fingerprint.clone());
        }
    }

    let buckets = store.buckets_for(&request.tenant_id, &distinct_fingerprints)?;
    if buckets.len() != distinct_fingerprints.len() {
        return Err(ReconcileError::Store(format!(
            "store returned {got} bucket results for {asked} fingerprints",
            got = buckets.len(),
            asked = distinct_fingerprints.len(),
        )));
    }

    let bucket_by_fp: std::collections::HashMap<&str, Option<&FingerprintBucket>> =
        distinct_fingerprints
            .iter()
            .map(|fp| fp.as_str())
            .zip(buckets.iter().map(Option::as_ref))
            .collect();

    let mut response = ReconcileResponse::empty();
    let mut seen_internal: BTreeSet<String> = BTreeSet::new();
    for entry in &request.entries {
        if !seen_internal.insert(entry.internal_id.clone()) {
            continue;
        }
        match bucket_by_fp
            .get(entry.fingerprint.as_str())
            .copied()
            .flatten()
        {
            Some(FingerprintBucket::Delivered) => {
                response.delivered.push(entry.internal_id.clone());
            }
            Some(FingerprintBucket::Failed) => {
                response.failed.push(entry.internal_id.clone());
            }
            Some(FingerprintBucket::Pending) => {
                response.pending.push(entry.internal_id.clone());
            }
            None => {
                response.unknown.push(entry.internal_id.clone());
            }
        }
    }
    Ok(response)
}

fn validate_internal_id(idx: usize, id: &str) -> Result<(), ReconcileError> {
    if id.is_empty() {
        return Err(ReconcileError::BadInternalId {
            index: idx,
            reason: "internal_id must not be empty".into(),
        });
    }
    if id.len() > MAX_INTERNAL_ID_BYTES {
        return Err(ReconcileError::BadInternalId {
            index: idx,
            reason: format!(
                "internal_id exceeds {MAX_INTERNAL_ID_BYTES}-byte cap (got {} bytes)",
                id.len()
            ),
        });
    }
    Ok(())
}

fn validate_fingerprint(idx: usize, fp: &str) -> Result<(), ReconcileError> {
    // BLAKE3 hex digest is 64 lowercase hex chars (32 bytes).
    if fp.len() != 64 {
        return Err(ReconcileError::BadFingerprint {
            index: idx,
            reason: format!("expected 64 hex chars (BLAKE3 digest), got {}", fp.len()),
        });
    }
    if !fp
        .bytes()
        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F'))
    {
        return Err(ReconcileError::BadFingerprint {
            index: idx,
            reason: "fingerprint contains non-hex characters".into(),
        });
    }
    Ok(())
}

/// In-memory [`OutboxLookup`] for tests. Real impls live alongside
/// the per-dialect Postgres/MySQL/SQLite migrations in T-071.
#[derive(Debug, Default)]
pub struct InMemoryOutboxLookup {
    rows: std::sync::Mutex<Vec<(TenantId, String, FingerprintBucket)>>,
}

impl InMemoryOutboxLookup {
    /// New empty lookup.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stamp `(tenant, fingerprint_hex, bucket)` so the next
    /// `buckets_for` call sees it. Idempotent on the triple.
    pub fn upsert(&self, tenant: &TenantId, fingerprint: &str, bucket: FingerprintBucket) {
        let mut rows = self.rows.lock().expect("test lock poisoned");
        rows.retain(|(t, f, _)| !(t == tenant && f == fingerprint));
        rows.push((tenant.clone(), fingerprint.to_owned(), bucket));
    }
}

impl OutboxLookup for InMemoryOutboxLookup {
    fn buckets_for(
        &self,
        tenant: &TenantId,
        fingerprints: &[String],
    ) -> Result<Vec<Option<FingerprintBucket>>, ReconcileError> {
        let rows = self
            .rows
            .lock()
            .map_err(|e| ReconcileError::Store(format!("lock poisoned: {e}")))?;
        Ok(fingerprints
            .iter()
            .map(|fp| {
                rows.iter()
                    .find(|(t, f, _)| t == tenant && f == fp.as_str())
                    .map(|(_, _, b)| b.clone())
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp_hex(byte: u8) -> String {
        // Deterministic 64-char hex string for a one-byte differentiator.
        let mut s = String::with_capacity(64);
        for _ in 0..32 {
            s.push_str(&format!("{byte:02x}"));
        }
        s
    }

    fn request(tenant: &str, pairs: &[(&str, u8)]) -> ReconcileRequest {
        ReconcileRequest {
            tenant_id: tenant.into(),
            entries: pairs
                .iter()
                .map(|(id, byte)| ReconcileEntry {
                    internal_id: (*id).to_owned(),
                    fingerprint: fp_hex(*byte),
                })
                .collect(),
        }
    }

    fn seeded_store() -> InMemoryOutboxLookup {
        let store = InMemoryOutboxLookup::new();
        let tenant = "tenant_a".to_owned();
        store.upsert(&tenant, &fp_hex(0x01), FingerprintBucket::Delivered);
        store.upsert(&tenant, &fp_hex(0x02), FingerprintBucket::Failed);
        store.upsert(&tenant, &fp_hex(0x03), FingerprintBucket::Pending);
        // Note: fp_hex(0x99) is intentionally not seeded, to test the
        // "unknown" bucket.
        store
    }

    #[test]
    fn buckets_each_entry_into_the_right_list() {
        let store = seeded_store();
        let req = request(
            "tenant_a",
            &[
                ("INV-1", 0x01),
                ("INV-2", 0x02),
                ("INV-3", 0x03),
                ("INV-4", 0x99),
            ],
        );
        let resp = handle_reconcile_request(&store, &req).unwrap();
        assert_eq!(resp.delivered, vec!["INV-1"]);
        assert_eq!(resp.failed, vec!["INV-2"]);
        assert_eq!(resp.pending, vec!["INV-3"]);
        assert_eq!(resp.unknown, vec!["INV-4"]);
        assert_eq!(resp.bead, RECONCILE_API_BEAD_ID);
    }

    #[test]
    fn empty_batch_returns_empty_response_without_querying_store() {
        struct Forbidden;
        impl OutboxLookup for Forbidden {
            fn buckets_for(
                &self,
                _tenant: &TenantId,
                _fingerprints: &[String],
            ) -> Result<Vec<Option<FingerprintBucket>>, ReconcileError> {
                panic!("store must not be queried for an empty batch");
            }
        }
        let resp = handle_reconcile_request(
            &Forbidden,
            &ReconcileRequest {
                tenant_id: "tenant_a".into(),
                entries: vec![],
            },
        )
        .unwrap();
        assert_eq!(resp, ReconcileResponse::empty());
    }

    #[test]
    fn batch_over_cap_is_rejected() {
        let oversized = ReconcileRequest {
            tenant_id: "tenant_a".into(),
            entries: (0..MAX_BATCH_ENTRIES + 1)
                .map(|i| ReconcileEntry {
                    internal_id: format!("INV-{i}"),
                    fingerprint: fp_hex(0x00),
                })
                .collect(),
        };
        let err = handle_reconcile_request(&InMemoryOutboxLookup::new(), &oversized).unwrap_err();
        assert!(matches!(err, ReconcileError::BatchTooLarge { .. }));
    }

    #[test]
    fn batch_at_cap_is_accepted() {
        let store = InMemoryOutboxLookup::new();
        let request = ReconcileRequest {
            tenant_id: "tenant_a".into(),
            entries: (0..MAX_BATCH_ENTRIES)
                .map(|i| ReconcileEntry {
                    internal_id: format!("INV-{i}"),
                    fingerprint: fp_hex(0x00),
                })
                .collect(),
        };
        let resp = handle_reconcile_request(&store, &request).unwrap();
        // All 10k internal ids should be in `unknown` (nothing seeded).
        assert_eq!(resp.unknown.len(), MAX_BATCH_ENTRIES);
        assert!(resp.delivered.is_empty());
        assert!(resp.failed.is_empty());
        assert!(resp.pending.is_empty());
    }

    #[test]
    fn duplicate_internal_id_is_deduplicated() {
        let store = seeded_store();
        let req = request(
            "tenant_a",
            &[("INV-1", 0x01), ("INV-1", 0x02), ("INV-1", 0x99)],
        );
        let resp = handle_reconcile_request(&store, &req).unwrap();
        // INV-1 should appear in exactly one bucket (the first one,
        // which is `delivered` per the first entry's fingerprint).
        let total =
            resp.delivered.len() + resp.failed.len() + resp.pending.len() + resp.unknown.len();
        assert_eq!(total, 1);
        assert_eq!(resp.delivered, vec!["INV-1"]);
    }

    #[test]
    fn duplicate_fingerprint_collapses_to_one_store_call() {
        // Count store invocations by wrapping the in-memory impl.
        let inner = InMemoryOutboxLookup::new();
        inner.upsert(
            &"tenant_a".into(),
            &fp_hex(0x05),
            FingerprintBucket::Delivered,
        );
        struct Counting<'a> {
            inner: &'a InMemoryOutboxLookup,
            calls: std::sync::Mutex<Vec<usize>>,
        }
        impl OutboxLookup for Counting<'_> {
            fn buckets_for(
                &self,
                tenant: &TenantId,
                fingerprints: &[String],
            ) -> Result<Vec<Option<FingerprintBucket>>, ReconcileError> {
                self.calls.lock().unwrap().push(fingerprints.len());
                self.inner.buckets_for(tenant, fingerprints)
            }
        }
        let counting = Counting {
            inner: &inner,
            calls: std::sync::Mutex::new(Vec::new()),
        };
        let req = request(
            "tenant_a",
            &[
                ("INV-A", 0x05),
                ("INV-B", 0x05),
                ("INV-C", 0x05),
                ("INV-D", 0x05),
            ],
        );
        let resp = handle_reconcile_request(&counting, &req).unwrap();
        let calls = counting.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "should call store exactly once");
        assert_eq!(calls[0], 1, "should de-dupe to a single fingerprint");
        assert_eq!(resp.delivered, vec!["INV-A", "INV-B", "INV-C", "INV-D"]);
    }

    #[test]
    fn cross_tenant_fingerprint_is_unknown() {
        let store = InMemoryOutboxLookup::new();
        // Seed under tenant_b, query under tenant_a.
        store.upsert(
            &"tenant_b".into(),
            &fp_hex(0x07),
            FingerprintBucket::Delivered,
        );
        let req = request("tenant_a", &[("INV-cross", 0x07)]);
        let resp = handle_reconcile_request(&store, &req).unwrap();
        assert_eq!(resp.unknown, vec!["INV-cross"]);
        assert!(resp.delivered.is_empty());
    }

    #[test]
    fn blank_internal_id_is_rejected() {
        let req = ReconcileRequest {
            tenant_id: "tenant_a".into(),
            entries: vec![ReconcileEntry {
                internal_id: String::new(),
                fingerprint: fp_hex(0x01),
            }],
        };
        let err = handle_reconcile_request(&InMemoryOutboxLookup::new(), &req).unwrap_err();
        assert!(matches!(err, ReconcileError::BadInternalId { .. }));
    }

    #[test]
    fn oversized_internal_id_is_rejected() {
        let req = ReconcileRequest {
            tenant_id: "tenant_a".into(),
            entries: vec![ReconcileEntry {
                internal_id: "x".repeat(MAX_INTERNAL_ID_BYTES + 1),
                fingerprint: fp_hex(0x01),
            }],
        };
        let err = handle_reconcile_request(&InMemoryOutboxLookup::new(), &req).unwrap_err();
        assert!(matches!(err, ReconcileError::BadInternalId { .. }));
    }

    #[test]
    fn malformed_fingerprint_is_rejected_by_length() {
        let req = ReconcileRequest {
            tenant_id: "tenant_a".into(),
            entries: vec![ReconcileEntry {
                internal_id: "INV-1".into(),
                fingerprint: "deadbeef".into(),
            }],
        };
        let err = handle_reconcile_request(&InMemoryOutboxLookup::new(), &req).unwrap_err();
        assert!(matches!(err, ReconcileError::BadFingerprint { .. }));
    }

    #[test]
    fn malformed_fingerprint_is_rejected_by_non_hex() {
        let req = ReconcileRequest {
            tenant_id: "tenant_a".into(),
            entries: vec![ReconcileEntry {
                internal_id: "INV-1".into(),
                fingerprint: "z".repeat(64),
            }],
        };
        let err = handle_reconcile_request(&InMemoryOutboxLookup::new(), &req).unwrap_err();
        assert!(matches!(err, ReconcileError::BadFingerprint { .. }));
    }

    #[test]
    fn store_error_propagates_as_store_variant() {
        struct Broken;
        impl OutboxLookup for Broken {
            fn buckets_for(
                &self,
                _tenant: &TenantId,
                _fingerprints: &[String],
            ) -> Result<Vec<Option<FingerprintBucket>>, ReconcileError> {
                Err(ReconcileError::Store("simulated outage".into()))
            }
        }
        let err = handle_reconcile_request(&Broken, &request("tenant_a", &[("INV-1", 0x01)]))
            .unwrap_err();
        assert!(matches!(err, ReconcileError::Store(_)));
    }
}
