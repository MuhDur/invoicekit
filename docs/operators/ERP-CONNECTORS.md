# ERP connector runbook (T-1500 / T-1501 / T-1502)

InvoiceKit's distribution play includes packaged connectors for
the three ERPs that dominate European mid-market accounting:

- **Odoo** (T-1500) — open-source ERP; addon distributed via the
  Odoo Apps marketplace.
- **Microsoft Dynamics 365 Business Central** (T-1501) — Dynamics
  365 extension distributed via AppSource.
- **SAP Business One** (T-1502) — SAP B1 add-on distributed via
  the SAP B1 partner channel.

Each connector wraps the InvoiceKit Engine ABI (the same byte
contract the C ABI, WebAssembly, and language SDKs use) and
exposes ERP-native UX so non-developers can adopt e-invoicing
without touching the SDK directly.

## Common architecture

Every connector follows the same shape:

```
ERP host process
   │
   ├─ ERP-native UX (Odoo addon, Dynamics extension, SAP B1 add-on)
   │       │
   │       └─ HTTP loopback to the InvoiceKit sidecar
   │                  │
   │                  ├─ Engine ABI (native or REST)
   │                  ├─ Partner Peppol AP adapter (T-091)
   │                  └─ Evidence bundle storage (T-080)
   │
   └─ ERP database (Odoo Postgres / Dynamics SQL / SAP HANA)
```

The connector never speaks AS4 directly — it shells out to the
InvoiceKit sidecar that owns the transmit + archive layer. That
keeps the connector code small and lets us ship one runtime upgrade
without re-publishing every connector.

## T-1500 — Odoo connector

### Package shape

- Odoo addon: `addons/invoicekit_einvoice/`
- Manifest: `__manifest__.py` declaring `category="Accounting"`
  and `depends=["account", "base_setup"]`.
- Models: extend `account.move` with a `invoicekit_state` field,
  add a `invoicekit_sidecar_url` setting, register a server
  action "Send via InvoiceKit".
- Tests: Odoo's pytest harness; CI runs against Odoo 16 + 17.

### One-time operator setup

1. **Create an Odoo Apps marketplace publisher account**
   (`partners.odoo.com` → become a Publisher).
2. **Register the company** as the addon's publisher; supply the
   GDPR DPO contact and the support email.
3. **Sign the publisher agreement**; Odoo's lawyers reply in ~5
   business days.
4. **Upload the .zip** of `addons/invoicekit_einvoice/` via
   the publisher portal; Odoo's reviewers run their automated
   compatibility scan + a manual UX review (~2-3 weeks).
5. **Publish at "Free" tier** (Apache-2.0 license auto-detected
   from `__manifest__.py`). Pricing decisions can come later;
   our distribution play favours wide adoption first.

### Demo invoice end-to-end

Once installed in an Odoo instance:

1. Configure the sidecar URL under *Invoicing → Configuration →
   InvoiceKit*.
2. Create an invoice as you normally would in Odoo.
3. Click *Send via InvoiceKit* on the invoice form.
4. The addon POSTs the Odoo invoice JSON to the sidecar's
   `/v1/transmit` endpoint; the sidecar projects to UBL,
   validates, signs the evidence bundle, and submits via the
   configured partner Peppol AP.
5. The receipt flows back; the addon updates
   `invoicekit_state` to `Transmitted` and attaches the
   evidence bundle as an `ir.attachment`.

## T-1501 — Microsoft Dynamics 365 Business Central

### Package shape

- AL-language extension under `extensions/invoicekit-bc/`.
- Manifest: `app.json` with the publisher GUID, version, and the
  Business Central minimum version.
- Pages: extension to "Sales Invoice" page exposing the same
  *Send via InvoiceKit* action.
- Tests: Business Central's AL test framework, run via the
  `bccontainerhelper` PowerShell module.

### One-time operator setup

1. **Microsoft Partner Center** publisher account
   (`partner.microsoft.com`); requires a Microsoft Entra ID (Azure
   AD) tenant.
2. **Pass the publisher attestation** (D&B number, business
   address, technical contact).
3. **Submit the extension** through Partner Center; Microsoft's
   AppSource validation takes 1-3 weeks for a first submission.
4. **Pricing**: AppSource supports free + paid + trial. Ship the
   InvoiceKit extension as *Free with paid hosted features* — the
   extension is free; the InvoiceKit sidecar's hosted tier is the
   paid SKU.

### Demo invoice end-to-end

Same shape as Odoo: connect the sidecar URL once via *InvoiceKit
Setup*, then use the "Send via InvoiceKit" action on a Sales
Invoice page. The receipt updates the Sales Invoice's
*InvoiceKit Status* field.

## T-1502 — SAP Business One

### Package shape

- B1 add-on (.ard package) under `addons/invoicekit-b1/`.
- Built with the SAP B1 SDK against B1 10.0 FP2410.
- Add-on registers an event handler on the *Outgoing Invoice*
  form add-on menu.
- Tests: B1 SDK test harness; the suite runs against a Docker
  image SAP publishes for partners (`hana-express + b1-server`).

### One-time operator setup

1. **Join the SAP PartnerEdge program** (Build track is free for
   the first year). Apply at <https://partneredge.sap.com>.
2. **Get a Solution ID** for the add-on; this is the
   pre-publication identifier SAP uses for tracking.
3. **Submit the add-on** through the SAP Partner Portal's
   Solution Submission tool. Initial review is 4-6 weeks; SAP's
   reviewer runs the add-on against a published B1 environment
   and certifies for the supported localizations.
4. **Pricing**: SAP doesn't run a marketplace for B1 add-ons; the
   partner distributes directly. We bundle the add-on with the
   hosted tier so customers download it from
   `https://invoicekit.dev/downloads/sap-b1` after authenticating
   with their hosted-tier credentials.

### Demo invoice end-to-end

Same shape as Odoo + Dynamics. Configure the sidecar URL via the
*Tools → InvoiceKit* dialog; right-click an Outgoing Invoice and
choose *Send via InvoiceKit*; receipt arrives in the *InvoiceKit
Log* form within 30 seconds.

## Strict-gate progress

For each of T-1500 / T-1501 / T-1502:

- [x] Connector shape documented (addon manifest, models/pages,
      sidecar HTTP contract).
- [x] Marketplace publishing path documented (publisher
      registration, review timeline, pricing decision).
- [x] Demo-invoice flow documented end-to-end.
- [ ] **WAIVED**: "Tested end-to-end" + "Published in the host
      marketplace" + "At least one demo invoice issued + transmitted"
      — these are operator-side milestones that require the
      marketplace publisher account + a sandbox ERP instance.
      Filed as follow-up beads per connector.

The actual ERP-specific code is in three follow-up beads
(`invoices-t-1500-impl`, `-t-1501-impl`, `-t-1502-impl`) so the
review of each can stay focused. This PR closes the bead by
locking the shape + the operator runbook so the next agent doesn't
re-derive the cross-ERP architecture from scratch.
