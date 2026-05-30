<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-be-peppol

Belgium (FOD/SPF Financiën, federal Mercurius / Hermes portals) — Peppol BIS Billing 3 delivery overlay.

A thin Belgium-specific overlay on the shared Peppol BIS Billing 3 substrate. It does not build the wire format: the caller passes in an already-serialized Peppol UBL payload, and this crate adds Belgian mandate routing, receiver-key typing, and pre-transport BTW/TVA checks on top.

## Capabilities

- **Route** an invoice to the correct Belgian endpoint per mandate and environment: Mercurius for B2G, Hermes (or any Peppol access point) for B2B, with a reserved B2C reporting tier. Sandbox vs production is selected explicitly.
- **Locally validate, before transport:**
  - receiver shape — KBO/BCE enterprise number (10 ASCII digits), VAT id (`BE` + 10 digits), or Peppol participant id (`scheme:value`), via `validate_receiver`;
  - BTW/TVA categorisation — rejects an empty category vector and rejects mixing `Exempt` with `Standard` on the same invoice, via `validate_vat_categories`;
  - a non-empty payload.
- **Carry a typed Belgian envelope** over the Peppol receipt: submission id, lifecycle status (`Submitted`, `Delivered`, `Accepted`, `Rejected`, `ValidationFailed`), the Peppol Message Level Response (MLR) reason text on failure, and a recorded UTC timestamp. Status transitions are polled through `poll_status`.

The delivery surface is the `BePeppolProvider` trait. A deterministic `MockBePeppolProvider` ships for tests and cassette-replay; the live Mercurius/Hermes implementation lands in `crates/report-be-peppol-http` behind a feature flag.

## Coverage

This crate is a **delivery and routing overlay, not a serializer**. It accepts the canonical Peppol BIS Billing 3 UBL as an opaque `Vec<u8>` (`BePeppolDeliverRequest::peppol_ubl_xml`) and does not parse, build, or canonicalize it — that is the job of the shared UBL/profile crates upstream. The only inspection it does on the payload is an emptiness check.

Live AS4 transmission is **delegated**, not implemented here: the doc-comment routes signing through `crates/signer-eidas` (or a Belgian QTSP such as Cybertrust / QuoVadis) and transport through `crates/transmit-peppol` (partner access point or `phase4`). This crate signs nothing and opens no connections; the only provider that ships in-crate is the deterministic mock.

Documented residuals:

- VAT categorisation is checked structurally (non-empty, no `Exempt`/`Standard` mix). The full set of Mercurius's stricter business rules beyond plain Peppol BIS is not enforced here.
- The `B2cReporting` mandate variant is a pre-routing placeholder; its reporting flow (RD/AR) is forthcoming and distinct from classic Peppol Billing.

## References

- Peppol BIS Billing 3 — the Belgian B2G and (from 2026) B2B wire format.
- Mercurius — Belgian federal e-invoicing portal, B2G intake (`mercurius.fedict.be`, `mercurius-test.fedict.be`).
- Hermes — Peppol access point routing B2B invoices to the receiver's chosen access point.
- KBO/BCE — Kruispuntbank van Ondernemingen / Banque-Carrefour des Entreprises enterprise number.

(All references above are those named in the crate's module documentation; no external specification URLs are cited in the source.)

## License

Apache-2.0.
