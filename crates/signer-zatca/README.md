<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-signer-zatca

Saudi Arabia ZATCA Phase 2 cryptographic-stamp adapter: ECDSA secp256k1 invoice stamp plus the Phase 2 QR-code tag-length-value (TLV) envelope, layered on `invoicekit-signer`.

## What it does

This crate adds the ZATCA (Zakat, Tax and Customs Authority) Phase 2 contract on top of the generic `invoicekit-signer` substrate. It models the two Phase 2 flows — `Standard` invoices (business-to-business / business-to-government, cleared by the ZATCA portal before delivery) and `Simplified` invoices (business-to-consumer, reported to the portal after delivery) — and produces a typed `ZatcaStampEnvelope` carrying the signature, the Cryptographic Stamp Identifier (CSID) it was produced under, the QR-code TLV bytes, the invoice hash, and the portal reporting status.

## Capabilities

- `Phase2Provider` trait — bundles the underlying `Signer` with the ZATCA-specific operations: pick the CSID, sign the invoice payload, build the QR TLV envelope, and report a status.
- `MockPhase2Provider` — the only provider that ships. Deterministic outputs; records every sign request for assertion. Used by tests and the cassette-replay sandbox.
- `ZatcaInvoiceMode` — `Standard` vs `Simplified`, with the differing clearance/reporting semantics.
- `ZatcaEnvironment` — `Compliance` (the portal's onboarding sandbox) vs `Production`. `stamp` rejects a CSID/target environment mismatch.
- `CsidRecord` — typed Cryptographic Stamp Identifier issued by the portal after Certificate Signing Request (CSR) submission: the opaque CSID, environment, 15-digit Kingdom of Saudi Arabia VAT number, optional stamp UUID, and the RFC 3339 `notBefore` / `notAfter` window.
- `QrField` — the ZATCA Phase 2 QR TLV tags (1–8): seller name, VAT number, timestamp, total, VAT total, invoice hash, stamp signature value, stamp public key.
- `validate_qr_fields` — enforces the five mandatory tags (1–5) on every invoice, and requires the stamp signature (tag 7) and public key (tag 8) on simplified invoices.
- `encode_qr_tlv` — encodes the QR-field map into the Phase 2 TLV byte envelope (`tag | single-byte length | UTF-8 value`, emitted in ascending tag order).
- `invoice_sha256_hex` — computes the lowercase-hex invoice digest the signature attests to.
- `ReportingStatus` — `Accepted`, `AcceptedWithWarnings`, `Rejected`, `Pending`, with an `is_accepted` predicate.

## Mode

**Mock / offline only.** No real cryptographic stamp ships. The doc-comment names ECDSA secp256k1 as the Phase 2 signature algorithm and SHA-256 over the canonicalized Universal Business Language (UBL) as the hash, but:

- The only provider is `MockPhase2Provider`. It signs through whatever `invoicekit_signer::Signer` it is handed; in-tree tests wire it to `SoftwareSigner`, a keyed BLAKE3 message-authentication-code placeholder, not an ECDSA signer.
- `invoice_sha256_hex` is a deterministic stand-in digest, not a real SHA-256 of canonicalized UBL. It exists so the surface and tests run end-to-end.

The real path — ECDSA secp256k1 signing, real SHA-256 over XML Canonicalization 1.1 UBL, and the live ZATCA portal CSR flow with sandbox/production certificates — lands behind a future `zatca-secp256k1` feature flag (the T-083b1 follow-up), with the ZATCA test vectors as fixtures. Producing a live stamp will need a real ECDSA secp256k1 signer and a CSID issued by the ZATCA portal against a registered taxpayer VAT number.

## Residuals

From the module doc-comment ("Strict-gate scope"):

- The T-083b1 strict gates — an ECDSA secp256k1 implementation, ZATCA documentation test vectors passing, and a test against a real sandbox certificate — are not met here. They need either an ECDSA crate in the workspace dependencies or a real ZATCA portal CSR.
- Only the substrate (types, trait, TLV/validation helpers, mock provider) ships. Real cryptography is deferred to the `zatca-secp256k1` feature flag.
- `encode_qr_tlv` uses single-byte TLV lengths and truncates a value defensively at 255 bytes so the encoded envelope stays well-formed.

## References

Named in the source; no URLs are present in the source, so none are cited here.

- ZATCA Phase 2 §V QR Code Specification — QR-code TLV tag numbers (tags 1–8).
- XML Canonicalization 1.1 (XML C14N 1.1) — the canonicalization the real SHA-256 invoice hash is taken over.
- RFC 3339 — the UTC timestamp format for CSID validity and the invoice timestamp.

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
