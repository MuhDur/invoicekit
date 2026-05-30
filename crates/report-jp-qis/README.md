<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-jp-qis

Japan (National Tax Agency / 国税庁, NTA) — Qualified Invoice System (QIS / 適格請求書発行事業者制度) adapter.

A typed Japan-specific overlay for the Qualified Invoice System that went live in October 2023. It does not build a national wire format and Japan operates no clearance portal — the NTA runs only a registration registry. This crate adds issuer registration validation, the JCT (Japanese Consumption Tax) rate enum, qualified vs simplified invoice kinds, and the registry-lookup trait the engine pings before delivery. Wire delivery is via Peppol-JP and is delegated to `crates/transmit-peppol`.

## Capabilities

- **Validate issuer registration numbers** — `validate_registration_number` enforces the NTA shape: the letter `T` followed by exactly 13 ASCII digits.
- **Look up a registration with the NTA registry** — the `QisRegistryProvider` trait exposes `lookup`, returning a `QisIssuerRegistration` (registration number, registered legal name, effective-from date, and an optional revoked-at date). The engine calls this before delivery so a buyer can confirm the issuer is registered and claim JCT input credit. `NtaEnvironment` selects sandbox vs production.
- **Carry typed JP enums** — `QisInvoiceKind` (`Qualified` vs `Simplified`), `JctCategory` (`Standard10`, `Reduced8`, `Zero`, `Exempt`), and `jct_basis_points`, which maps each category to integer basis points (1000 / 800 / 0 / 0) for float-free arithmetic.
- **Ship a deterministic mock registry** — `MockQisRegistryProvider` resolves any well-formed number to a synthetic record and can flip specific numbers into the revoked state via `revoke`, for cassette-replay tests.

This crate does not serialize an invoice, sign anything, or open any connections.

## Coverage

This crate is a **typed overlay plus registry-lookup surface, not a serializer**. Japan does not operate a clearance portal, so there is no national XML or JSON format emitted here; the wire format is Peppol BIS Billing 3 under the Japanese CIUS (Peppol-JP), produced by the shared UBL/profile crates upstream and transmitted by `crates/transmit-peppol`. The QIS layer's job is the registration-number gate and the JCT typing the buyer needs to claim input credit.

The only registry provider that ships in-crate is `MockQisRegistryProvider`. The live NTA registry lookup and the Peppol-JP delivery integration land in a follow-up `report-jp-qis-http` crate (per the Cargo manifest description).

Documented residuals:

- Registration validation is **structural only** — `T` + 13 ASCII digits. It does not confirm the number exists in the NTA registry or that the registration is active; that requires a live `lookup`.

## References

- National Tax Agency (国税庁, NTA) — the registrar for qualified-invoice issuers; runs the registration registry (`kokuzei.nta.go.jp` production, `kokuzei-test.nta.go.jp` sandbox).
- Qualified Invoice System (適格請求書発行事業者制度) — JCT input-credit regime live since October 2023; qualified invoices (適格請求書) carry the issuer's NTA registration number.
- Peppol BIS Billing 3 with the Japanese CIUS (Peppol-JP) — the wire format for delivery, transmitted via `crates/transmit-peppol`.

(All references above are those named in the crate's module documentation; no external specification URLs are cited in the source.)

## License

Apache-2.0.
