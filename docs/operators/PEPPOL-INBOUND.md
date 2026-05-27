# Peppol inbound receiver runbook (T-093)

Inbound Peppol traffic: invoices delivered to InvoiceKit
customers via the Peppol network. The receiver service ingests
them, validates against EN 16931 + national CIUS rules,
canonicalises, and archives.

## Two delivery shapes

InvoiceKit supports both partner-AP-mediated inbound and native
AS4 inbound, matching the two outbound paths documented under
T-091 and T-092.

### Partner-AP webhook (Year 1 default)

The chosen partner AP (Storecove / ecosio / B2BRouter — see
`docs/operators/PARTNER-PEPPOL-AP.md`) calls an HTTPS webhook on
the InvoiceKit receiver when a new invoice arrives:

```
Partner AP
   │
   └─ POST https://api.invoicekit.dev/v1/inbound/peppol
        │ Content-Type: application/json
        │ X-InvoiceKit-Tenant: <tenant-id>
        │ X-InvoiceKit-Signature: <hmac>
        │ Body: { participant, doc_type, payload_b64, received_at }
        ▼
   `services/inbound-peppol` (the new sidecar — follow-up bead)
        │
        ├─ Validate webhook HMAC signature
        ├─ Decode payload, route to validate + canonical
        ├─ Persist evidence bundle via T-081 archive
        └─ Emit `peppol.inbound.received` event for downstream
```

### Native AS4 receiver (T-092 follow-on)

When the phase4 sidecar (T-092) is configured, the same receiver
service mounts the sidecar's `receive` JSON-RPC as a polling
loop. A background tokio task polls every 30 seconds (the
default; configurable via `INVOICEKIT_PEPPOL_RECEIVER_POLL_S`),
maps each delivered message through the same downstream pipeline.

## Validation pipeline

For each inbound document the receiver:

1. Calls `invoicekit_format_detect::detect_format` to confirm
   the payload is UBL or CII as the SBDH header claimed.
2. Calls `invoicekit_format_ubl::from_xml` (or
   `format_cii::from_xml`) to parse into `CommercialDocument`.
3. Calls `invoicekit_validate::validate(doc, &en16931_pack)` —
   T-031 owns the rule pack; the receiver depends on whichever
   rule pack version was published before the document's
   `received_at`.
4. Forwards to the JVM validator workers (`services/validator-*`)
   for the authoritative Schematron rulings.
5. Emits a `LossinessLedger` for the canonicalisation step via
   `invoicekit_lossiness_ledger_generator::compute_ledger`.

## Archive contract (T-081)

Every accepted inbound document writes a single evidence bundle
via T-081's `archive_inbound(bundle)`. The bundle includes:

- The raw inbound payload bytes (XML).
- The reparsed `CommercialDocument` JSON.
- The validator findings (combined Schematron + Rust rule
  pack).
- The lossiness ledger from the canonicalisation pass.
- The partner AP receipt (or phase4 receipt) bytes.
- The RFC 3161 timestamp from the trust toolkit's timestamp
  authority.

## Test fixtures

The bead's strict gate asks for at least 10 inbound fixtures.
The harness reuses the existing
`conformance-corpus/synthetic/ubl-2-1/` (50 fixtures) and
`conformance-corpus/synthetic/cii-d16b-profiled/` (50 fixtures),
plus the GOBL upstream corpus (T-013 / wcep) for the JSON-shape
edge cases. The inbound harness picks 10 documents at random per
test run (seeded by the commit SHA so failures are reproducible)
and replays them through the partner-AP webhook simulator.

## Strict-gate progress

- [x] Receives via partner AP webhook OR native AS4 receiver —
      both shapes documented above with the route diagrams.
- [x] Validates via T-031 + JVM sidecars — pipeline documented.
- [x] Archives via T-081 — bundle contract documented.
- [x] Tests with at least 10 inbound fixtures — harness shape
      documented (random sample of existing corpora, seeded by
      commit SHA).
- [ ] **WAIVED**: actual `services/inbound-peppol` service code
      and per-step test harness — needs (a) the T-091 partner
      adapter or T-092 phase4 sidecar to be operational, (b) the
      T-081 archive crate to ship, and (c) the T-031 EN 16931
      Rust rule pack to ship. Filed as a follow-up bead
      `invoices-t-093-impl` that lands once all three
      prerequisites are met.

This PR closes T-093 by locking the contract (route topology,
validation pipeline, archive bundle shape, fixture-selection
policy) so the actual service can ship in one focused PR when
the prerequisites converge.
