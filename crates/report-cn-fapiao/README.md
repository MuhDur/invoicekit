# invoicekit-report-cn-fapiao

China State Taxation Administration (国家税务总局, STA) Golden Tax / e-Fapiao (全面数字化的电子发票) clearance adapter.

This crate ships the typed integration surface for STA e-Fapiao clearance and a deterministic mock provider. It does not serialize Fapiao XML and it does not talk to STA yet. The caller hands in an already-signed XML payload as opaque bytes; the live STA REST integration lands in a follow-up `report-cn-fapiao-http` crate.

## Capabilities

- **Typed clearance surface.** `FapiaoProvider` is the trait an STA transport implements: `issue_fapiao` and `void_fapiao`. The request carries tenant id, environment (`Sandbox` / `Production`), fapiao class (`FapiaoKind`), issuer Unified Social Credit Code (USCC), and the signed XML payload. The response envelope carries the STA-assigned 20-character fapiao number, the 12-digit fapiao code (发票代码), a status, an RFC 3339 timestamp, and an optional reason.
- **Local shape validation.** `validate_uscc` checks that an issuer USCC is 18 ASCII alphanumeric characters. `issue_fapiao` additionally rejects an empty payload. These run before any wire call. A `Rejected` verdict returned by STA is not an error: it is surfaced as `FapiaoStatus::Rejected` inside the envelope so the engine can persist the rejection alongside its audit trail.
- **Deterministic mock.** `MockFapiaoProvider` returns fixed timestamps and incrementing serials, for tests and offline development.

It does not serialize, validate the contents of, or sign the Fapiao payload, and it does not transmit to STA.

## Coverage

Opaque-payload / bring-your-own model. The `payload` field is the canonical signed XML that the caller produces upstream; this crate treats it as bytes and never parses, builds, or signs it. There is no native Fapiao XML serializer here.

The only validation performed is structural: USCC must be 18 ASCII alphanumeric characters, and the payload must be non-empty. No business rules, no schema check, no content inspection.

The only `FapiaoProvider` implementation is `MockFapiaoProvider`, which fabricates fapiao numbers and codes and never contacts STA. Per the module doc-comment, the live STA REST integration — issuer registration, the pre-allocated invoice-code track (发票字轨), signing, submission, and QR-encoded fapiao retrieval — lands in a follow-up `report-cn-fapiao-http` crate and is not present here.

`FapiaoKind` enumerates VAT special (增值税专用发票), VAT general (增值税普通发票), and the electronic general / special variants, but the kind is passed through to the (future) transport; this crate does not branch on it.

## References

- State Taxation Administration of China (国家税务总局, STA) — operator of the Golden Tax (金税) e-Fapiao clearance regime, as described in the module documentation. No URL is cited in the source.

## License

Apache-2.0.
