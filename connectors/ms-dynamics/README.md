# connectors/ms-dynamics

Microsoft Dynamics 365 Business Central connector for InvoiceKit.

The implementation lives in `extensions/invoicekit-bc/` as an AL extension.
It adds an InvoiceKit setup page, extends the Sales Invoice page with a
`Send via InvoiceKit` action, posts invoice JSON to the InvoiceKit sidecar
`/v1/transmit` endpoint, and records the returned submission id plus
evidence bundle URL on the sales invoice.

## Local checks

Run the static package conformance checks from the repository root:

```bash
python3 -m pytest extensions/invoicekit-bc/tests -q
```

The full Business Central test run is wired in
`.github/workflows/dynamics-connector.yml` through `BcContainerHelper`.
It is conditional because it requires operator-provisioned Business Central
container credentials and a Windows container-capable runner.

## Package layout

- `extensions/invoicekit-bc/app.json` — AppSource package manifest.
- `extensions/invoicekit-bc/src/InvoiceKitSetup.*.al` — sidecar URL and API key setup.
- `extensions/invoicekit-bc/src/SalesInvoiceInvoiceKit.PageExt.al` — Sales Invoice action.
- `extensions/invoicekit-bc/src/InvoiceKitSidecarClient.Codeunit.al` — sidecar HTTP client.
- `extensions/invoicekit-bc/test/InvoiceKitSidecar.Tests.Codeunit.al` — AL test codeunit.

## Install and uninstall behavior

Installing the extension creates its own setup table and adds extension fields
to Sales Header. It does not modify base Business Central tables outside the
normal AL extension field mechanism. Uninstalling the extension removes the
InvoiceKit objects and leaves native sales invoices intact; operators that want
to retain the submission id and evidence URL should export those extension
fields before uninstalling.
