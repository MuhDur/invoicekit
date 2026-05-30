<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-inbound-peppol

Library half of the Peppol inbound receiver: detect, parse, validate, and archive a received invoice payload behind an injected archive trait.

This crate is the pure-logic core. The eventual `services/inbound-peppol/` axum binary wraps `InboundPipeline::process` in an HTTP handler that accepts either the partner access-point webhook payload or the unwrapped XML from a native AS4 receiver.

## Capabilities

`InboundPipeline::process` runs one received document through four stages:

1. **Format-detect** — calls `invoicekit_format_detect::detect_format` on the raw bytes. Only `FormatId::Ubl21` and `FormatId::CiiD16B` are accepted; anything else returns `InboundError::UnrecognizedFormat`.
2. **Parse** — decodes the bytes as UTF-8, then calls `invoicekit_format_ubl::from_xml` or `invoicekit_format_cii::from_xml`. Yields a `CommercialDocument` plus a `LossinessLedger`. UTF-8 or parse failure returns `InboundError::Parse`.
3. **Validate** — calls `CommercialDocument::validate` (the intermediate-representation's own envelope checks). Failure returns `InboundError::Validate`.
4. **Archive** — builds an `EvidenceBundle` and persists it through the injected `Archive` trait, returning the trait's `EvidenceReceipt`.

The emitted `EvidenceBundle` carries the tenant id, trace id, inbound source, detected format, the raw payload, the parsed `CommercialDocument`, the lossiness ledger, the validator findings vector, and a hex SHA-256 of the payload. `payload_sha256_hex` is a real SHA-256 (`sha2` crate), lowercase hex, used to content-address the archive entry.

`InboundSource` (`PartnerWebhook` | `NativeAs4`) records which lane delivered the bytes and serializes as kebab-case JSON. The bundle and finding types derive `serde` so the schema is stable across the scaffold and the eventual real backends.

## Mode / Residuals

This crate is a scaffold. Two stages named in the pipeline are not yet implemented here:

- **Rule-pack validation is a stub.** The `validator_findings` field is always populated with an empty `Vec`. The T-031 JVM-validator JSON-RPC bridge is a follow-up; today the only validation performed is the intermediate-representation's own `validate()` envelope checks. The `ValidatorFinding` type exists to lock the schema (`rule`, `severity`, `message`) the future bridge will fill. Do not read an empty findings list as "the document passed the EN 16931 / Peppol rule pack" — those rules are not run.
- **Archiving is delegated, no production backend ships here.** `Archive` is a trait. The only implementation in this crate is `MockArchive`, which stores bundles in memory and returns a receipt whose `archive_id` is the first 12 chars of the payload SHA-256. The production WORM / Object-Lock archive (T-081, behind `s3` / `azure` features) is a separate follow-up crate; this crate does not persist to durable storage on its own.

The HTTP service binary, the partner-webhook handler, and the native AS4 receiver are out of scope for this crate — it only exposes `InboundPipeline::process` for those call sites to drive.

## References

- `docs/operators/PEPPOL-INBOUND.md` — the T-093 runbook this crate implements (referenced in the module doc-comment).
- Format support comes from sibling crates: `invoicekit-format-detect`, `invoicekit-format-ubl`, `invoicekit-format-cii`, `invoicekit-ir`.

No external standards URLs are embedded in the source.

## License

Apache-2.0. Workspace member, `publish = false`.
