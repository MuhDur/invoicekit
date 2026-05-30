# cii-coverage — CII D16B element-edge coverage artifact generator

A developer/CI tool that reads the UN/CEFACT Cross Industry Invoice (CII) D16B
XSD bundle and emits a deterministic JSON matrix classifying how each schema
element is handled by `format-cii`. It is a coverage-bookkeeping generator, not
a validator and not a parser.

## What it does

`generate_coverage.py` parses two XSD files from the CEN EN16931 validation
schema bundle (pinned to repository `ConnectingEurope/eInvoicing-EN16931`, tag
`validation-1.3.16`, the `D16B SCRDM (Subset)/uncoupled clm/CII` subset):

- `CrossIndustryInvoice_100pD16B.xsd` (root)
- `CrossIndustryInvoice_ReusableAggregateBusinessInformationEntity_100pD16B.xsd`

It collects every `complexType`, walks the type graph reachable from
`CrossIndustryInvoiceType`, and emits one row per reachable
`complexType`/`element` edge — rows are schema element edges, not sample
documents. Each row records the declaring type, element name, XSD type,
cardinality (`minOccurs..maxOccurs`), and a classification into one of six
fixed classes:

- `current_ir` — mapped by the current `invoicekit-format-cii`
  parser/serializer; the row names the target intermediate-representation paths.
- `cii_document_field_extension` — preserved as a named CII document-field
  extension (for example `BuyerReference`, business-process context).
- `profile_extension_payload` — preserved in the CII profile-context extension.
- `invoicekit_metadata_extension` — InvoiceKit's own application-context
  parameter that carries `tenant_id`/`trace_id`/`source_system`.
- `cii_preserved_xml_extension` — no typed IR field yet; preserved as a
  replayable XML fragment.
- `lossiness_ledger_preserved` — recognized CII business surface that needs an
  explicit semantic field or a lossiness-ledger pass before full-fidelity
  claims.

Every edge is classified; the default for an unrecognized element is
`cii_preserved_xml_extension`, so nothing is silently dropped. The output also
includes class counts, the source repository/tag/commit/subset, SHA-256 hashes
of all four pinned schema files (root, RAM, QualifiedDataType,
UnqualifiedDataType), and a list of named mapping decisions documenting the
metadata-overload boundaries (for example, that `BuyerReference` is never
`tenant_id`). JSON is written sorted and indented for stable diffs.

The classification table and per-element strategies are hand-maintained inside
the script. The tool reflects those decisions; it does not infer or enforce
runtime behavior.

## Usage / CI

Run the generator against a local checkout of the CEN schema bundle:

```
python3 tools/cii-coverage/generate_coverage.py \
  --schema-root <path-to-CII-D16B-XSD-directory> \
  --output crates/format-cii/data/cii-d16b-element-coverage.json
```

Both `--schema-root` and `--output` are required. The XSD bundle is supplied at
run time; it is not vendored in this directory.

The committed artifact lives at
`crates/format-cii/data/cii-d16b-element-coverage.json`. In CI it is guarded by
`tools/release-checks/test_cii_coverage.py`, run via
`pytest tools/release-checks/test_cii_coverage.py -q` (wired into the
`license-header.yml` workflow). That gate re-reads the committed artifact and
asserts the source constants match the generator, the pinned schema SHA-256
hashes are unchanged, the class set and counts hold, every edge is uniquely
classified with a non-empty strategy and cardinality, and the named
metadata-overload boundaries are explicit. The gate checks the artifact; it does
not re-run the generator to regenerate it.

## License

Apache-2.0.
