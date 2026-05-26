# InvoiceKit

> One toolkit for the full B2B invoicing lifecycle — generate, check, render, read, send, and store legally-correct e-invoices for Germany, France, Belgium, Italy, Poland, and the Peppol network. Runs on whatever stack you already use (Node, Python, Go, Java, .NET, browser, edge).

**Status: planning complete, implementation about to begin.**

## What this is

Every country in Europe (and most of the world) is rolling out mandatory electronic invoicing. Today, every existing tool forces a tradeoff a developer should not have to make:

- Use Java, or it won't work.
- Sign up for a hosted service, give up control, and pay €15,000 a year minimum.
- Glue together five separate libraries and a Java sidecar to send one invoice.

InvoiceKit replaces all of that with one open-source package you install in seconds.

## What it does

- **Writes** legally-correct invoices in every required format (Factur-X, ZUGFeRD, XRechnung, Peppol BIS, FatturaPA, KSeF FA(3), and more).
- **Checks** invoices against each country's rules with clear, explained error messages.
- **Renders** to PDF/A-3 with the machine-readable data embedded inside.
- **Reads** invoices back — from PDFs, scans, or XML — and pulls out the structured data, with every extracted field carrying proof of where it came from.
- **Sends** invoices (optional, hosted) through Peppol and national gateways, with full delivery proof.
- **Archives** every operation as a signed, verifiable bundle that holds up in audit.

## Who it is for

Any developer who needs to make their software issue or accept e-invoices in any country we support. That includes:

- Accounting and billing tool builders.
- E-commerce platforms.
- Any business with a custom in-house billing system.
- Freelancers who want a clean script.
- AI agents that need to issue compliant invoices as part of their work.

No account needed. No per-invoice fee for the free core. Apache 2.0 forever.

## How it is different from everything else

| | Existing tools | InvoiceKit |
|---|---|---|
| Runtime | Java, or a hosted service | Node, Python, Go, Java, .NET, browser, edge — same engine |
| Cost to start | €0 to €15,000/year | €0, no signup |
| Coverage | One country or one format | Whole portfolio under one install |
| Reading invoices | Separate paid service | Built-in |
| Audit trail | DIY | Signed evidence bundle for every operation |
| License | Often locked or AGPL | Apache 2.0 |

## What we deliberately don't do

- We do not file taxes.
- We do not run an accounting ledger.
- We do not process payments — we describe how an invoice should be paid; the payment happens elsewhere.
- We do not replace ERPs; we feed them.

## Project background

InvoiceKit was scoped after a deep research pass covering 57 jurisdictions, 60+ existing tools, ~50 commercial competitors, ~490 candidate ideas, and adversarial reviews from multiple foundation models. The full reasoning is open:

- [Master report](research/MASTER_REPORT.md) — single-page executive summary of the whole effort
- [Implementation plan](plans/PLAN.md) — what we are building, how, in what order
- [Revisions log](plans/PLAN_v0.2_revisions.md) — what changed after first review round
- [Research files](research/) — every market research stream, idea-generation phase, and adversarial critique

## License

Apache 2.0.

## Country coverage

Global. The engine architecture is layered so it covers two kinds of countries:

- **The 35+ countries that use European-style formats** (Universal Business Language, Cross Industry Invoice, EN 16931 European norm, Peppol BIS, Peppol PINT, Factur-X, ZUGFeRD) come for free once the core engine is shipped. That includes every European Union state, the United Kingdom, Australia, New Zealand, Singapore, Japan, Norway, Iceland, Switzerland, and the United Arab Emirates.
- **The countries with their own national format or government portal** each get a dedicated package built on top of the same engine: Germany (XRechnung, ZUGFeRD), France (Chorus Pro / PA-PDP), Italy (FatturaPA via SDI), Poland (KSeF FA(3)), Spain (VeriFactu, FacturaE, TicketBAI), Saudi Arabia (ZATCA Phase 2), India (IRP / GST), Mexico (CFDI 4.0), Brazil (NF-e and NFS-e), Malaysia (MyInvois), Greece (myDATA), Romania (RO e-Factura), Hungary (NAV), Portugal, Turkey (e-Fatura), Israel, Egypt, the rest of Latin America (Chile, Colombia, Peru, Argentina, Ecuador, Costa Rica, Dominican Republic), the rest of Asia-Pacific (Indonesia, Philippines, Vietnam, Thailand, South Korea, Japan Qualified Invoice System, China Golden Tax, Taiwan), and Africa (Kenya eTIMS, Nigeria FIRS, South Africa SARS).

That is roughly sixty jurisdictions in total. Each carries an honest maturity label per capability (serialize, validate, render, sandbox, live-delivery, inbound, archive, correction) so claims of "support" are always backed by the matrix in [the plan](plans/PLAN.md).

## Status

Plan complete. Implementation has not started.
