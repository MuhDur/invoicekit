# OASIS UBL 2.1 XSD Provenance

This directory vendors the minimal official OASIS UBL 2.1 OASIS Standard XSD
closure needed to validate InvoiceKit's current `Invoice` and `CreditNote`
serializer outputs.

- Upstream root: https://docs.oasis-open.org/ubl/os-UBL-2.1/
- Upstream release: OASIS Universal Business Language 2.1 OASIS Standard, 04 November 2013.
- Retrieved: 2026-05-27 UTC.
- Retrieval method: direct HTTPS download from `docs.oasis-open.org`.
- Use: offline test/runtime schema validation only. The XSD files are stored
  unchanged so the upstream copyright and permission notice remains attached.

## Validated Fixtures

`invoicekit-format-ubl` currently pins these serializer fixtures as passing the
vendored OASIS UBL 2.1 XSD harness:

- `format-ubl serializer invoice fixture seed=20` against `xsd/maindoc/UBL-Invoice-2.1.xsd`
- `format-ubl serializer credit-note fixture seed=21` against `xsd/maindoc/UBL-CreditNote-2.1.xsd`

Broader real-world fixture diversity is tracked separately by `invoices-bbqm`.

## Local Extension Schema

`invoicekit-extension-v1.xsd` is InvoiceKit-authored, not an OASIS file. It
declares the `urn:invoicekit:ubl:extension` metadata payload that the serializer
places under `ext:ExtensionContent`. UBL's official extension content model uses
a lax foreign-namespace slot, so this supplemental schema validates the private
payload without changing the vendored OASIS XSD files.

- SHA-256: `8bd791dc5c9e3259fd10999aa5a141659ba2e0a1eafd90f060ca73e5e9e9e89a`

## Files

| SHA-256 | Path | Source URL |
| --- | --- | --- |
| `40fae8cb436f3a9506d7acce65ba162caef3b0bed4d5cbc0992b2153ded4edf4` | `xsd/maindoc/UBL-Invoice-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/maindoc/UBL-Invoice-2.1.xsd |
| `a54651b1225052f811bf2ba01346f13f2454e7cc3e0be290c91dd680dc7b7b1a` | `xsd/maindoc/UBL-CreditNote-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/maindoc/UBL-CreditNote-2.1.xsd |
| `939172ad8dd057cd403e7f763f6532184dd5ed4b9de24c42ebb35db4792ba613` | `xsd/common/UBL-CommonAggregateComponents-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/common/UBL-CommonAggregateComponents-2.1.xsd |
| `bd4ad043ee1d9da1c7f8018dabf739cfafdfb59143d0d16b9ef769e6b7c408a7` | `xsd/common/UBL-CommonBasicComponents-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/common/UBL-CommonBasicComponents-2.1.xsd |
| `ad7a4e490978adfbcfc5ec0bb20941cf11ac960ccf0c4de8791a7c731a8dbe87` | `xsd/common/UBL-CommonExtensionComponents-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/common/UBL-CommonExtensionComponents-2.1.xsd |
| `7dcb156e610239c97ae70940cf4653b88e48c3595bf5f56a2204a32e2893e6cf` | `xsd/common/UBL-QualifiedDataTypes-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/common/UBL-QualifiedDataTypes-2.1.xsd |
| `09052d406b4293e2a5f9c2bfee6df10ad4d8d5f0b36e24a6349d7f7936d89eb6` | `xsd/common/UBL-UnqualifiedDataTypes-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/common/UBL-UnqualifiedDataTypes-2.1.xsd |
| `fcee77a11870208e6377ea6311b9f2a050bca24bdad8606ea02d71e9f9e72f8d` | `xsd/common/UBL-ExtensionContentDataType-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/common/UBL-ExtensionContentDataType-2.1.xsd |
| `dd546e4809df86b6445589f69f0d6c9df162840ae386574ddfc1da7638103e15` | `xsd/common/CCTS_CCT_SchemaModule-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/common/CCTS_CCT_SchemaModule-2.1.xsd |
| `3db472305f029bba5c1ae157bfd0178f715c3f9b94bd8e6c557dbce5e88da874` | `xsd/common/UBL-CommonSignatureComponents-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/common/UBL-CommonSignatureComponents-2.1.xsd |
| `17bb6b62d709b4fd81449a37655af36aa6a1276ad4fdb1b2e249a5ed4b7c2172` | `xsd/common/UBL-SignatureAggregateComponents-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/common/UBL-SignatureAggregateComponents-2.1.xsd |
| `cef924d7ba3d1d8ade14469325cde1364f8c174e46f0198ec02da8e9e748a489` | `xsd/common/UBL-SignatureBasicComponents-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/common/UBL-SignatureBasicComponents-2.1.xsd |
| `101909c9f06456d61ddcc4fb982f1d40dc357b439f393b1a2eb46e42acd60809` | `xsd/common/UBL-xmldsig-core-schema-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/common/UBL-xmldsig-core-schema-2.1.xsd |
| `a4f726bcf8cc3f7d9ffa4dab99e005535a8e8b60dced1e5d94578d2e05afa96e` | `xsd/common/UBL-XAdESv132-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/common/UBL-XAdESv132-2.1.xsd |
| `1fa4625e9cefcb7a9abb5ac1b64315547450031eece8a55bd584e4ba4b79dbc1` | `xsd/common/UBL-XAdESv141-2.1.xsd` | https://docs.oasis-open.org/ubl/os-UBL-2.1/xsd/common/UBL-XAdESv141-2.1.xsd |
