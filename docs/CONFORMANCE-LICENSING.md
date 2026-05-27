# Conformance Corpus Licensing

InvoiceKit's conformance corpus is part of the trust surface. A fixture may be
small, but it can still carry copyright, personal data, tax identifiers, trade
secrets, gateway credentials, or country-specific legal context. This policy
defines which fixtures can enter the repository, how they must be described, and
which corpus partition they belong to.

This policy applies to every file under `conformance-corpus/`, including XML,
JSON, PDF, images, gateway cassettes, OCR inputs, redaction reports, and derived
golden outputs.

## Corpus Partitions

`conformance-corpus/synthetic/`

Contains generated or hand-authored fixtures whose contents are fictional and
safe to publish. Public synthetic fixtures use `CC0-1.0` unless the generator or
fixture file must inherit `Apache-2.0` from project code. Synthetic fixtures must
not be derived from customer invoices, production gateway traffic, or documents
found on the public web unless the source license is recorded as licensed-real.

`conformance-corpus/licensed-real/`

Contains explicitly licensed real or official sample invoices that have been
reviewed for redistribution and redacted when needed. Every fixture in this
partition must include license evidence in the metadata. A URL alone is not
enough unless the upstream page itself grants the needed redistribution rights.

`conformance-corpus/private-regression/`

Contains private support, customer, partner, or incident fixtures. These files
must not be published in the public repository. If a private fixture is needed to
explain a public bug, create a synthetic minimization or a redacted licensed-real
fixture instead.

`conformance-corpus/generators/`

Contains source code and data used to generate synthetic fixtures. Generator code
is Apache-2.0 unless a file states otherwise. Generated outputs must still carry
fixture metadata.

## Metadata Requirement

Every fixture directory must contain a `metadata.json` file that validates
against `conformance-corpus/fixture-metadata.schema.json`. The metadata is the
review contract for a fixture. It must identify:

- the corpus partition and publication status;
- the artifact path, media type, byte size, and SHA-256 digest;
- the format family, document type, jurisdiction, and profile;
- the license, redistribution status, and license evidence when required;
- provenance, including generator or upstream source;
- PII classification and redaction status;
- expected validation outcome and known gaps;
- owner, review dates, and labels.

Metadata must be updated in the same pull request as any fixture content change.
Changing fixture bytes without changing `sha256` and `size_bytes` is a failed
review.

## Licensing Rules

Public synthetic fixtures may use `CC0-1.0` or `Apache-2.0`. Use `CC0-1.0` for
plain generated invoice data. Use `Apache-2.0` when the fixture is generated
from project source code or includes nontrivial project-authored template text.

Licensed-real fixtures require written evidence that InvoiceKit can store,
test, and, if marked public, redistribute the fixture. Acceptable evidence
includes an upstream license file, an official sample page with explicit reuse
terms, a signed contributor statement, or a tracked permission record. Record the
evidence path in `license.evidence_path`.

Private-regression fixtures must use `license_id` `PRIVATE-REGRESSION`, must set
`publication` to `private`, and must not appear in public releases or public
test artifacts.

Do not use copyleft-only fixture licenses that would force InvoiceKit code or
the full public corpus to be redistributed under a different license. If a
fixture is useful but its licensing is unclear, do not commit it. File a bead
for source review instead.

## Redaction Rules

Redaction is required for any real fixture containing personal data, tax
identifiers, bank details, customer names, addresses, order numbers, access
tokens, QR payloads, gateway receipts, or other information that could identify
an entity or transaction.

A redacted fixture must keep a redaction report next to the fixture or in a
reviewed private evidence store. The report records what was changed and why,
without storing the original secret value in the public repository.

Synthetic fixtures must set `pii.classification` to `synthetic` and
`contains_personal_data` to `false`. Private fixtures may contain personal data
only when the support or compliance workflow explicitly permits it, and they
must remain in `private-regression/`.

## Intake Workflow

1. Choose the correct corpus partition before adding files.
2. Create the fixture artifact and `metadata.json`.
3. Run `python3 tools/conformance-corpus/validate_fixture_metadata.py`.
4. Run the applicable parser, serializer, validator, renderer, or gateway tests.
5. Review the metadata against this policy before opening the pull request.

If a fixture cannot pass the metadata validator, it is not ready to enter the
corpus.

## Publication Rules

Only fixtures with `publication` `public`, `license.redistribution`
`public-ok`, and `pii.contains_personal_data` `false` may be published in the
public repository, public docs, release artifacts, benchmark dashboards, or
public validator examples.

Private fixtures may be used in local or protected CI only when the storage and
access controls are documented. They must not be copied into support bundles,
PR comments, issue attachments, or generated public reports.

## Review Cadence

Every fixture must have `maintenance.review_due`. Synthetic fixtures can use a
12-month review window. Licensed-real and private fixtures should use a shorter
window when source permissions, redaction rules, or jurisdictional formats are
likely to change.

Retire a fixture when its license evidence expires, redaction is insufficient,
or it no longer exercises a useful behavior. Quarantined fixtures remain in the
repository only to reproduce a known failure and must not be treated as passing
conformance evidence.
