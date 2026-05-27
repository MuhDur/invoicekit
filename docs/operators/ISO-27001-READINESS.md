# ISO 27001 readiness — engagement plan

This document tracks the InvoiceKit ISO 27001 readiness engagement
that the bead `invoices-t-005` opens. The work is operational and
spans 6–12 months; this file is the per-quarter checkpoint anchor
so future operators (or auditors) can see the chronology.

## Why ISO 27001 and not SOC 2 first

Both certifications get InvoiceKit through procurement at most
EU-mid-market customers. The trust toolkit pitch leans on
public-evidence-friendly process — ISO 27001's published policy
artefacts and Annex A control register fit that better than SOC 2's
auditor-only Type II reports. The plan therefore prioritises ISO
27001, with SOC 2 Type I targeted as a fast follow when a US-only
customer specifically requires it.

## Scope decision (Q1 of the engagement)

In scope:

- The hosted Engine ABI (`services/managed-api-server`).
- The validator JVM sidecars (`services/validator-*`).
- The release pipeline (cosign, CycloneDX SBOM, release.yml).
- The conformance corpus and its provenance metadata.
- Customer-managed Peppol AP credentials (handled via partner AP
  in Year 1 — confirm the partner's ISO 27001 letter is on file).
- The signing substrate and key rotation runbook.

Out of scope (Year 1):

- The reference client SDKs (Python / Node / Java / .NET / Go) —
  customers run these in their own environments.
- The CLI and the Rust core crates as a library — same reason.
- Customer-side hosting (the InvoiceKit IR, format, and validate
  crates compiled into a customer binary).

This split keeps the audit boundary tight and avoids spending
engagement cycles on code paths that never reach our infra.

## Year-1 engagement timeline

The dates below assume a kickoff in early Q3 2026. Update each
checkpoint when the corresponding evidence lands.

| Month | Milestone | Evidence pointer |
| ---: | --- | --- |
| 0 | Pick a registered certification body and a registered consultant. Sign engagement letters. | `docs/operators/iso27001/engagement-letter.pdf` (private) |
| 1 | Statement of Applicability (SoA) draft v0.1. | `docs/operators/iso27001/soa-v0.1.md` |
| 2 | Asset register and risk treatment plan v0.1. | `docs/operators/iso27001/asset-register-v0.1.md` |
| 3 | Policy library v0.5 (acceptable use, access control, cryptography, backup, business continuity, change management, supplier management, data classification, incident response). | `docs/operators/iso27001/policies/` |
| 4 | Internal control validation pass. Capture residual risk. | `docs/operators/iso27001/internal-validation-v0.1.md` |
| 6 | Stage-1 audit. | `docs/operators/iso27001/stage1-report.pdf` (private) |
| 8 | Address Stage-1 findings. | `docs/operators/iso27001/stage1-corrective-actions.md` |
| 10 | Stage-2 audit. | `docs/operators/iso27001/stage2-report.pdf` (private) |
| 12 | Certification issued. | Certificate URL — published in the next quarter's release notes. |

## Hard prerequisites already in place

The following are already shipped and become evidence the
engagement reuses:

- **Cosign-signed releases** with CycloneDX SBOM
  (`docs/RELEASE-SIGNING.md`).
- **Cassette PII scan in CI** (catches accidental personal-data
  ingestion in conformance fixtures).
- **License-header gate** (every source file declares its SPDX
  identifier).
- **Cargo audit + cargo deny on every PR and every release** (CVE
  + license + ban + advisory gates).
- **Conformance corpus provenance metadata schema**
  (`conformance-corpus/fixture-metadata.schema.json`) plus the
  validator in `tools/conformance-corpus/`.
- **Per-bead pull request trail** with `discovered-from` links
  (provides the change-management audit trail Annex A.12.1 asks for).

## Operator responsibilities during the engagement

1. **Quarterly evidence rotation.** Pull the cosign verification
   commands from each release into the policy library so the
   certifying body can run them.
2. **Annual penetration-test commission.** A third-party pen test
   on the hosted Engine ABI is a standard supplementary control.
   Track the engagement under `docs/operators/iso27001/pentest-<year>/`.
3. **Incident response drill.** Run one rehearsed incident per
   quarter using the `tools/status-page/incidents/` ledger as the
   public output (the same artefact the trust toolkit pitches as
   evidence).

## When to revisit the scope

- A new hosted service ships (e.g., an asynchronous webhook bus).
  Add it to the in-scope list and re-run the asset register pass.
- A customer requires SOC 2 alongside ISO 27001. Open a
  follow-up bead that starts the SOC 2 Type I scoping
  conversation; the policy library carries straight over.
- An auditor flags a control as inapplicable that we previously
  declared applicable in the SoA. Record the rationale in
  `docs/operators/iso27001/soa-<version>.md` and bump the SoA
  version pointer above.
