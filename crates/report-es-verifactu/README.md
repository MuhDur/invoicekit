<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-es-verifactu — Spain / AEAT / VeriFactu

Typed registration surface for Spain's VeriFactu anti-fraud reporting regime (Real Decreto 1007/2023), administered by the Agencia Estatal de Administración Tributaria (AEAT). The crate models the per-invoice register request/receipt, the SHA-256 hash chain, NIF shape validation, and the verification-QR builder. It does not talk to the AEAT — the live SOAP/REST transport lands in a follow-up `report-es-verifactu-http` crate.

## Capabilities

- **Validate (shape only).** `validate_nif` checks the issuer's NIF / DNI / NIE is 9 ASCII alphanumeric characters (the Spanish modulo-23 checksum is explicitly out of scope). `validate_sha256_hex` checks a previous-hash string is 64 lowercase hex characters.
- **Hash-chain plumbing.** The register request carries `previous_hash_hex` and the receipt returns `recorded_hash_hex`; the engine pins one invoice's recorded hash as the next invoice's previous hash. The provider computes the chain hash over the supplied invoice payload.
- **QR builder.** `qr_payload` glues the issuer NIF, invoice number, issuance date, and gross total into the AEAT `ValidarQR` verification URL the printed/PDF invoice carries.
- **Provider trait + mock.** `VeriFactuProvider::register_invoice` is the registration contract a live AEAT integration implements. `MockVeriFactuProvider` is a deterministic in-crate implementation for cassette-replay tests; `with_forced_status` drives the `AcceptedWithWarnings` (`AceptadoConErrores`) and `Rejected` (`Incorrecto`) verdicts the happy path never reaches.

The crate does **not** serialize, sign, or transmit. There is no native VeriFactu XML serializer here.

## Coverage

**Opaque payload (bring-your-own envelope).** The crate does not build the AEAT VeriFactu XML. `VeriFactuRegisterRequest::invoice_xml` is an opaque `Vec<u8>` the caller supplies — the engine produces the canonical XML upstream (Spain rides the EN 16931 / UBL 2.1 family path; the offline lifecycle test serializes via `invoicekit-format-ubl`) and this crate registers those bytes and computes the chain hash over them.

Documented residuals from the module doc-comment:

- **No live AEAT transport.** Only `MockVeriFactuProvider` ships. The mock synthesises a SHA-256-*shaped* hash (a deterministic expansion of the payload byte length plus a payload-byte prefix — not a real hash) and a `MOCK-CSV-` serial so replay tests stay byte-identical. The real SHA-256 and the SOAP/REST wire arrive in `report-es-verifactu-http`.
- **NIF is shape-checked only**, not checksum-validated.
- **Both operating modes are modeled** — `VeriFactu` (real-time reporting) and `NoVeriFactu` (local SIF hash chain, AEAT inspection on demand). They share the same hash-chain shape; only the transport differs, and the live transport does not exist yet.
- **AEAT verdicts are receipts, not errors.** A `Rejected` / `AcceptedWithWarnings` verdict is an `Ok(VeriFactuRegisterEnvelope)` so the engine persists it in the audit trail. `Err(VeriFactuError)` is reserved for pre-wire shape failures (`BadNif`, `BadPreviousHash`, `BadXml`) and transport faults.

## References

- AEAT VeriFactu portal — Sistemas Informáticos de Facturación: <https://sede.agenciatributaria.gob.es/Sede/iva/sistemas-informaticos-facturacion-verifactu.html>
- Real Decreto 1007/2023 (cited in the module doc-comment as the governing regime).
- AEAT QR specification, chapter 4 (`ValidarQR` verification-URL shape, cited in `qr_payload`).

## License

Apache-2.0.
