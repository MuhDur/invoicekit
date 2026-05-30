<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-report-kr-nts

South Korea — National Tax Service (국세청, NTS) e-Tax Invoice (전자세금계산서) clearance adapter.

Submits a caller-supplied signed e-Tax Invoice XML payload to NTS's Hometax clearance regime and returns the approval number, status, and issuance timestamp. This crate does not build or sign the XML; it carries an opaque payload to the authority and surfaces the verdict.

## Capabilities

- **Transmit (typed surface only).** Defines the `NtsProvider` trait — one `submit` call that takes an `NtsSubmitRequest` (tenant, environment, invoice kind, issuer BRN, signed XML bytes) and returns an `NtsSubmitEnvelope` (approval number, status, issuance timestamp, optional rejection reason).
- **Local validation of the issuer Business Registration Number (사업자등록번호).** `validate_brn` checks the 10-digit shape (hyphenated `NNN-NN-NNNNN` collapses to `NNNNNNNNNN`); `validate_brn_checksum` additionally verifies the NTS modulus-10 check digit (검증번호) using the weights `[1,3,7,1,3,7,1,3,5]` — the same rule Hometax enforces before accepting a filing.
- **Pre-wire payload guard.** `submit` rejects an empty XML payload and an ill-shaped BRN as errors before anything goes on the wire.
- **Deterministic mock provider.** `MockNtsProvider` returns serial approval numbers and a fixed timestamp; `with_forced_rejection` synthesizes an authority-side 전송오류 (transmission error) receipt to exercise the rejection branch.

The crate models three NTS document kinds — `Standard` (일반), `Exempt` (면세), `Correction` (수정) — and two environments (`Test`, `Production`).

## Coverage

Opaque-payload / partner model. The crate does **not** serialize NTS e-Tax Invoice XML from the InvoiceKit intermediate representation and does **not** sign it. The caller supplies an already-signed `invoice_xml: Vec<u8>` blob; this crate validates the issuer BRN, refuses an empty payload, and conveys the payload to NTS.

Documented residuals from the module doc-comment:

- **No live transport.** Only `MockNtsProvider` ships. The live NTS REST integration lands in a follow-up `report-kr-nts-http` crate. Until then nothing in this crate reaches `hometax.go.kr`.
- **Authority rejection is a receipt, not an error.** An NTS 전송오류 verdict surfaces inside the envelope as `NtsStatus::Rejected` with a reason, never as a transport `Err` — so the engine can persist the refusal alongside its audit trail. Only pre-wire shape failures (bad BRN, empty payload) return `Err`.
- **BRN validation is shape + check digit only.** `validate_brn_checksum` confirms the tenth digit is internally consistent; it does not confirm the BRN is registered with NTS.

## References

- National Tax Service Hometax portal: <https://hometax.go.kr>

## License

Apache-2.0.
