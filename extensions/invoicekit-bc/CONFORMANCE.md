# InvoiceKit Business Central Connector Conformance

This package implements the T-1501 runbook contract from
`docs/operators/ERP-CONNECTORS.md`.

| Requirement | Evidence |
| --- | --- |
| AL extension lives under `extensions/invoicekit-bc/` | `app.json` and `src/*.al` |
| App manifest declares publisher, version, and Business Central minimum version | `app.json` |
| Sales Invoice page exposes `Send via InvoiceKit` | `src/SalesInvoiceInvoiceKit.PageExt.al` |
| Connector posts to the sidecar `/v1/transmit` endpoint | `src/InvoiceKitSidecarClient.Codeunit.al` |
| Receipt updates InvoiceKit status, submission id, and evidence URL | `src/InvoiceKitSidecarClient.Codeunit.al` and `src/SalesHeaderInvoiceKit.TableExt.al` |
| Configuration page stores sidecar URL and API key | `src/InvoiceKitSetup.Table.al` and `src/InvoiceKitSetup.Page.al` |
| Uninstall path does not corrupt host ERP data | `connectors/ms-dynamics/README.md` |
| Static conformance checks run on every PR | `.github/workflows/dynamics-connector.yml` |
| Business Central host tests are wired through bccontainerhelper | `.github/workflows/dynamics-connector.yml` |

The real Business Central sandbox run needs operator-provisioned container
credentials and Windows container support. The workflow keeps that job
conditional and always runs the package-shape conformance checks.
