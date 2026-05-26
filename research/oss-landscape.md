# OSS E-Invoicing Landscape Audit — May 2026

Scope: open-source projects relevant to a "ffmpeg-of-invoicing" library — format conversion/generation, validation, PDF/A-3 embedding, OCR/extraction, PDF rendering, Peppol AS4 transmission, and national e-invoicing wrappers.

All stars/last-commit dates from direct GitHub fetches in May 2026. Where I could not directly fetch the repo (rate limits, etc.), figures are flagged as approximate.

---

## 1. Format conversion / generation libraries

The EN 16931 core is dominated by two ecosystems: **Helger's Java stack** (phax/*) and **Mustang** (Java). Outside Java, the picture is fragmented — many "good enough for one language" projects, no canonical implementation.

| Project | URL | Lang/Runtime | Stars | Last activity | License | Formats | Strengths | Weaknesses |
|---|---|---|---|---|---|---|---|---|
| **mustangproject** | github.com/ZUGFeRD/mustangproject | Java (Maven/CLI/REST server) | 422 | v2.23.0, Apr 2026 | Apache-2.0 | ZUGFeRD 1/2, Factur-X, CII XRechnung 3.0.2, UBL | Most complete OSS Factur-X/ZUGFeRD lib; includes CLI, REST server, visual viewer | Java-only; XSLT-heavy (96% of code); poor API docs; difficult to embed outside JVM |
| **ph-en16931 / phive-rules / ph-ubl / phase4** (Helger stack) | github.com/phax | Java | phase4: 219, phive: ~80 | All very active May 2026 | Apache-2.0 | Everything EN 16931 / Peppol BIS / CII / UBL / XRechnung; full Peppol AS4 + SMP | Reference-quality. Used by most commercial Peppol AP vendors. Validation rules updated within days of OpenPEPPOL releases. | Java-only; sprawling (40+ repos); steep learning curve; one-man maintainer risk (Philip Helger) |
| **ZUGFeRD-csharp** | github.com/stephanstapel/ZUGFeRD-csharp | C#/.NET | 380 | v18.0.0, Mar 2026 | Apache-2.0 | ZUGFeRD, Factur-X, XRechnung | Mature; widely used in DACH .NET shops | **Going commercial** — v18 is the last OSS feature release; new features now in paid "FactoorSharp" |
| **akretion/factur-x** | github.com/akretion/factur-x | Python | 294 | v4.2, Mar 2026 | BSD | Factur-X, Order-X, ZUGFeRD 1/2.2 | Cleanest Python lib; CLI tools (`facturx-pdfgen`, etc.); schematron support | Cannot generate ZUGFeRD 1.x; XSD-only validation by default; small surface |
| **invoice-x/factur-x-ng** | github.com/invoice-x/factur-x-ng | Python | ~80 | Stale ~2023 | BSD | Factur-X | Dict-based abstraction across XML flavors | Largely abandoned; superseded by akretion fork |
| **horstoeko/zugferd** | github.com/horstoeko/zugferd | PHP | 408 | Active 2026 | MIT | ZUGFeRD/XRechnung/Factur-X (Min, Basic, EN16931, Extended) | Profile-agnostic API; 4.7M Packagist installs; Laravel adapter | PHP-only; no extraction (read+write XML+attach to PDF only) |
| **josemmo/einvoicing** | github.com/josemmo/einvoicing | PHP | 177 | v0.3.1 Jun 2025 (slowing) | MIT | UBL, CII, Peppol BIS, XRechnung, Factur-X | Best-designed PHP OO model; aims at 100% EN16931 | Slowing activity; PHP-only; no Peppol transmission |
| **easybill/zugferd-php** | github.com/easybill/zugferd-php | PHP | 101 | v6.0.0 Apr 2026 | MIT | ZUGFeRD v1/v2, Factur-X, XRechnung | Object↔XML mapping; clean DX | PHP-only |
| **easybill/e-invoicing** | github.com/easybill/e-invoicing | PHP | ~50 | Active 2026 | MIT | EN16931 UBL+CII, XRechnung, Peppol BIS, ZUGFeRD/Factur-X | Generates & reads structured XML cleanly | PHP-only; no PDF embedding |
| **atgp/factur-x** | github.com/atgp/factur-x | PHP | ~60 | Active | MIT | Factur-X, ZUGFeRD 2.0 | Pure PHP PDF/A-3 generation | Single-format focus |
| **Youniwemi/digital-invoice** | github.com/Youniwemi/digital-invoice | PHP | ~30 | Active | MIT | Wraps atgp + josemmo + easybill | "Just works" facade over three libs | Brittle: depends on three upstreams |
| **node-zugferd** (jslno) | github.com/jslno/node-zugferd | TypeScript/Node | <50 | Active 2026 | MIT | ZUGFeRD, Factur-X | Modern TS; custom profile defs; embeds in PDF/A | Single-author; small; Node-only (no browser) |
| **@stackforge-eu/factur-x** | jsr.io/@stackforge-eu/factur-x | TypeScript (Node+Deno) | n/a (JSR) | Active 2026 | n/a | Factur-X, ZUGFeRD, XRechnung CII | Uses `libxml2-wasm` for XSD validation in JS! Closest existing thing to "WASM e-invoicing" | Bun/browser/CF Workers compatibility "unknown"; small; PDF/A-3 generation only |
| **@e-invoice-eu/core** (gflohr) | github.com/gflohr/e-invoice-eu | TypeScript | 192 | Apr 2026 | WTFPL | Factur-X levels (Min/Basic/EN16931/Extended/XRechnung), UBL, CII, XRechnung-UBL, XRechnung-CII | **Runs in browser** as ESM/UMD; CLI; REST API; mapping from Excel/CSV/ODS; large parts auto-generated from PEPPOL-UBL docs | PDF generation in browser requires LibreOffice (so blocked); no Peppol transmission; permissive license (WTFPL) can scare enterprises |
| **@stafyniaksacha/facturx** | npmjs.com/@stafyniaksacha/facturx | TypeScript | n/a | 2024-2025 | n/a | Factur-X, Order-X | Uses pdf-lib + libxmljs | Limited scope; Node only |
| **invopop/gobl** | github.com/invopop/gobl | Go | **277** (+ 53 forks) | v0.403.0 May 13 2026 — **very active** | Apache-2.0 | UBL, CII, FatturaPA, CFDI, KSeF, VeriFactu, FacturaE, TicketBAI, Stripe | **The closest thing to "ffmpeg of invoicing" today.** Universal JSON intermediate format. CLI, HTTP server with MCP, JSON Web Signatures. Maintained by Invopop (commercial co.) with strong velocity. | Go-only consumer story (CLI/server fine, but not embeddable in JS/Python/JVM without subprocess); no first-class PDF/A-3 embedding; no Peppol AS4 transmission; no OCR; commercial backer could pivot |
| **invopop/gobl.cfdi**, **gobl.verifactu**, **gobl.ksef** etc. | github.com/invopop/* | Go | Sub-libs of GOBL | Active | Apache-2.0 | Per-country | Modular per-regime conversion | Lock-in to GOBL JSON schema |
| **CenPC434/java-tools** | github.com/CenPC434/java-tools | Java | <50 | Stale-ish | Apache-2.0 | EN 16931 reference (UBL/CII/EDIFACT) | Official CEN reference code | Reference, not production-grade |
| **phax/en16931-cii2ubl** | github.com/phax/en16931-cii2ubl | Java | ~30 | Active | Apache-2.0 | CII→UBL conversion | Quality conversion logic | Java; one direction |
| **AgID/EeISI-mapper** | github.com/AgID/EeISI-mapper | Java | ~30 | Largely dormant | Apache-2.0 | UBL ↔ CII ↔ FatturaPA via intermediate XMLCEN | Italy-government-backed semantic mapping | Java; abandoned-ish; eIGOR successor |
| **2015-EU-IA-0050/eIgor** | github.com/2015-EU-IA-0050/eIgor | Java | n/a | Archived | EUPL | UBL ↔ CII ↔ FatturaPA | Original EU-funded mapper | Archived |
| **ubl-builder** (pipesanta) | github.com/pipesanta/ubl-builder | TypeScript | small | Active 2025 | MIT | UBL 2.0/2.1 | Pure UBL builder | UBL-only; no EN16931 conformance |
| **Xalgorithms/lib-ubl-js** | github.com/Xalgorithms/lib-ubl-js | JavaScript | small | Stale | n/a | UBL parser | One of few pure-JS UBL parsers | Stale |

### Conversion / mapping highlights

- The **EeISI/eIGOR** semantic intermediate XMLCEN was an excellent idea (and EU-funded), but is now dormant. Worth studying for the data model.
- **Helger's `en16931-cii2ubl`** is the most reliable production-grade CII↔UBL conversion code — Java only.
- **GOBL's JSON schema** is the only modern, language-agnostic intermediate format currently shipping. It is effectively a competitor to whatever intermediate we design.

---

## 2. Validation libraries

| Project | URL | Lang | Stars | Last activity | License | What | Gap |
|---|---|---|---|---|---|---|---|
| **KoSIT validator** | github.com/itplr-kosit/validator | Java | 157 | v1.6.2 Feb 2026 | Apache-2.0 | Generic schema+Schematron engine | Java-only; not config-bundled |
| **KoSIT validator-configuration-xrechnung** | github.com/itplr-kosit/validator-configuration-xrechnung | XML/Schematron | n/a | v2026-01-31 | Apache-2.0 | The official XRechnung 3.0.x rules | Requires running KoSIT validator |
| **OpenPEPPOL/peppol-bis-invoice-3** | github.com/OpenPEPPOL/peppol-bis-invoice-3 | XML/Schematron | n/a | Nov 2025 + Mar 2026 hotfix | Open | Authoritative Peppol BIS Billing 3.0 rules | Rules only — need an engine |
| **phax/phive** + **phive-rules** | github.com/phax/phive | Java | ~80 | Active 2026 | Apache-2.0 | All-in-one validation engine. Pre-bundles Peppol, EN16931, XRechnung, Italian SDI, BDEW etc. | Java only |
| **veraPDF-library** | github.com/veraPDF/veraPDF-library | Java | n/a | Active (Open Preservation Foundation) | MPL-2.0/GPL+ | The de-facto open PDF/A-1/2/3 validator | Java only; heavy |
| **AgID Italian SDI validator** | Various forks | Java/Schematron | n/a | Maintained by AgID/Italia | EUPL | FatturaPA validation | Italy-only |

**Key observation:** **No production-grade Schematron engine exists in JS, Rust, Go, or Python** that can run the official EN16931/Peppol/XRechnung rule packs natively. Everyone shells out to Java (KoSIT, phive) or skips Schematron entirely. This is a real gap.

---

## 3. PDF embedding / Factur-X PDF/A-3 libraries

Embedding XML into a PDF/A-3 compliant container is the single most fiddly mechanical task in the stack. Few libs do it correctly.

| Project | URL | Lang | Stars | Notes |
|---|---|---|---|---|
| **mustangproject** | (above) | Java | 422 | Reference quality; uses Apache PDFBox under the hood. |
| **iText (community)** | github.com/itext/itext-java | Java | ~1.5k | AGPL/commercial dual; Factur-X examples ship in docs. AGPL kills most commercial use. |
| **akretion/factur-x** (Python) | (above) | Python | 294 | Uses pypdf — known to occasionally produce non-strict PDF/A-3 output. |
| **atgp/factur-x** (PHP) | (above) | PHP | ~60 | Uses TCPDF; PDF/A-3 conformance is "best effort". |
| **pdf-lib** (Hopding) | github.com/Hopding/pdf-lib | TS/JS | 7k+ | Attachments supported via `attach()`; **but PDF/A-3 conformance is not guaranteed** — must post-process. This is a known limitation requested in issue #229. |
| **PDFBox / iText 7** | Apache PDFBox | Java | n/a | Mature, but Java. |
| **veraPDF** | (above) | Java | n/a | Validate output of any embedding step. |
| **node-zugferd, @stackforge-eu/factur-x** | (above) | TS | small | Build PDF/A-3 using pdf-lib + custom OutputIntent injection. Quality varies. |

**Key observation:** **No widely-used Rust, Go, or browser-WASM library reliably produces strict PDF/A-3b/PDF/A-3u compliant containers with valid XMP, OutputIntent, and embedded XML AFRelationship.** Several attempts exist in TS but none have been independently verified via veraPDF in CI. The "PDF/A-3 in WASM" niche is wide open.

---

## 4. OCR / extraction libraries

| Project | URL | Lang | Stars | Last activity | License | Invoice-specific? | Gap |
|---|---|---|---|---|---|---|---|
| **Docling** (IBM) | github.com/docling-project/docling | Python | **60.4k** | v2.95.0 May 2026 | MIT | General doc AI; "structured information extraction [beta]"; XBRL but not native invoice schema | No EN16931/Peppol output; PDF/DOCX/PPTX/XLSX/HTML/audio; needs schema-driven post-processing for invoices |
| **marker** (datalab-to/VikParuchuri) | github.com/VikParuchuri/marker | Python | **35.4k** | v1.10.2 Jan 2026 | **GPL-3.0** | No | License kills commercial use; PDF→markdown only |
| **donut** (Clova) | github.com/clovaai/donut | Python | 6.9k | **Stale Nov 2022** | MIT | OCR-free SROIE-style receipts | Effectively abandoned |
| **LayoutLMv3** | github.com/microsoft/unilm/tree/master/layoutlmv3 | Python | 21k+ (unilm) | unilm active | MIT (non-commercial in fine print for some models) | Fine-tunable for invoices | Requires fine-tuning per layout; license caveats |
| **PaddleOCR** | github.com/PaddlePaddle/PaddleOCR | Python | ~45k | Active 2026 | Apache-2.0 | PP-StructureV3 for tables/KIE; not invoice-schema aware | CJK-centric training; needs invoice post-processing |
| **invoice2data** | github.com/invoice-x/invoice2data | Python | 2.2k | v0.5.0 May 23 2026 | MIT | **Yes** — YAML template-driven | Template-per-supplier doesn't scale; no LLM/VLM path; no EN16931 mapping |
| **Tesseract + pytesseract** | github.com/tesseract-ocr/tesseract | C++ | 70k+ | Active | Apache-2.0 | No | Pure OCR; you build pipeline on top |
| **docTR** (Mindee) | github.com/mindee/doctr | Python | ~4.5k | Active | Apache-2.0 | Detection+recognition for documents | Not invoice-schema aware |
| **Unstructured-IO/unstructured** | github.com/Unstructured-IO/unstructured | Python | ~12k | Active | Apache-2.0 | General doc partitioning + a `pipeline-invoices` reference repo | Reference is thin; relies on inference models |
| **Mindee open-source bits** | github.com/mindee (org) | Python/JS | n/a | docTR + SDK clients | various | SDKs call hosted API (paid for invoice OCR) | Real invoice extraction model is **not** open-source — only docTR primitives |

**Key observation:** No open-source extractor goes **directly from PDF/image → validated EN16931/UBL/CII XML**. Docling is the obvious foundation (huge community, IBM-backed, MIT) but stops at structured extraction — you still need to map to invoice schemas and validate. invoice2data is the only invoice-aware lib but its template approach is dated. **The PDF→EN16931 pipeline is the single biggest greenfield in the OSS landscape.**

---

## 5. PDF template / rendering libraries

These are general PDF generators developers reach for when they need to *create* the human-readable invoice (then embed structured XML on top).

| Project | URL | Lang | Stars | Notes |
|---|---|---|---|---|
| **pdfmake** | github.com/bpampuch/pdfmake | JS | 11k+ | Declarative JSON; works in browser+Node; no PDF/A out of the box |
| **jsPDF** | github.com/parallax/jsPDF | JS | 30k+ | Imperative canvas API; widely used; no PDF/A |
| **react-pdf / @react-pdf/renderer** | github.com/diegomura/react-pdf | TS/React | 16k+ | Renders React → PDF; popular for invoices; no PDF/A |
| **WeasyPrint** | github.com/Kozea/WeasyPrint | Python | ~7k | CSS Paged Media; **supports PDF/A and PDF/UA via `--pdf-variant`**; lightweight | Python-only |
| **wkhtmltopdf** | wkhtmltopdf.org | C++ | unmaintained | Effectively dead since 2022 |
| **Headless Chrome / Puppeteer** | various | JS | huge | No native PDF/A; needs Gotenberg/QPDF post-processing |
| **Gotenberg** | github.com/gotenberg/gotenberg | Go | ~8.5k | Wraps Chromium+LibreOffice+QPDF; **does produce PDF/A-1a/2b/3b** | Container-only; not embeddable |
| **PDFKit (Node)** | github.com/foliojs/pdfkit | JS | 10k+ | No PDF/A |
| **Typst** | github.com/typst/typst | Rust | 35k+ | Newer; PDF output; no PDF/A-3 attachments yet |

**Key observation:** PDF/A-3 generation in the browser is essentially **unsolved**. WeasyPrint + Python is the cleanest open-source path today for PDF/A-3-ready hybrid invoices, with embedding done by a separate library (e.g. akretion/factur-x). A WASM-native PDF/A-3 builder is a real gap.

---

## 6. Peppol AS4 / access point libraries

Peppol AS4 is almost entirely a Java-only world today. This is the deepest moat in OSS-vs-vendor.

| Project | URL | Lang | Stars | License | Last activity | Notes |
|---|---|---|---|---|---|---|
| **phase4** | github.com/phax/phase4 | Java | 219 | Apache-2.0 | v4.5.1 May 22 2026 (very active) | **The reference open-source AS4 library**. Supports Peppol, CEF eDelivery, BDEW, DBNAlliance, ENTSOG, EUDAMED, EUCTP. Used everywhere. |
| **phoss-ap** | github.com/phax/phoss-ap | Java/Spring Boot | small | Apache-2.0 | Active | Standalone Peppol AP built on phase4 |
| **phoss-smp** | github.com/phax/phoss-smp | Java | n/a | Apache-2.0 | v8.1.5 Apr 2026 | SMP server; SQL/Mongo/XML backends |
| **Oxalis** (original) | github.com/OxalisCommunity/oxalis | Java | 150 | LGPL-3.0 | **Frozen Nov 2025** — bug fixes only | Once the dominant AP; now in maintenance |
| **Oxalis-NG** | github.com/OxalisCommunity/oxalis-ng | Java | 45 | LGPL-3.0 | v1.3.0 May 10 2026 | The successor — actively developed but lower star count; community migrating |
| **vefa-peppol** | github.com/OxalisCommunity/vefa-peppol | Java | n/a | Apache-2.0 | Maintained | Supporting library |
| **peppol-commons / peppol-smp-client / peppol-sml-client** (Helger) | github.com/phax/peppol-commons | Java | n/a | Apache-2.0 | Very active | Shared identifier/codelist/SBDH handling. Universal in JVM Peppol stacks. |
| **mendelson AS4** | sourceforge.net/projects/mendelson-as4 | Java | n/a | GPL | Active | Standalone AS4 with GUI; covers Peppol, eSENS, BDEW, ICS2, etc. **Not on GitHub.** |
| **recommand/recommand-peppol** | github.com/brbxai/recommand-peppol | TypeScript | 33 | **AGPL-3.0** | Active 2026 | Rare modern TS Peppol AP project. Claims certified provider status. AGPL limits commercial integration. |
| **getpeppr** | getpeppr.dev | TypeScript SDK | n/a | **Closed source** (SDK is free; AP is hosted on Storecove) | Commercial | Not OSS but shows market shape |
| **e-invoice-be/e-invoice-ts** | github.com/e-invoice-be/e-invoice-ts | TypeScript | n/a | n/a | Active | Client SDK for e-invoice.be hosted Peppol API |

**Key observation:** Outside Java (phase4 + Oxalis-NG), there is **no production-grade open-source Peppol AS4 access point**. Everyone in TS/Python/Go uses a hosted gateway (Storecove, e-invoice.be, Pagero, getpeppr). This is structurally the hardest moat to cross — AS4 is heavy, certificates are bureaucratic, OpenPEPPOL certification is paid and political. **But it is also where the most leverage is** if we can ship even a "certifiable-in-progress" AS4 client.

---

## 7. National e-invoicing libraries

### Italy — FatturaPA / SDI
| Project | Lang | Stars | Notes |
|---|---|---|---|
| italia/fatturapa-php-sdk | PHP | small | Official Italia repo; SOAP+PEC client |
| italia/fatturapa-python | Python | small | CLI-only invoice generator |
| Truelite/python-a38 | Python | 48 | Best-known FatturaPA Python lib; XML in/out |
| taocomp/php-sdicoop-client | PHP | small | SDI web services client |
| s2software/fatturapa | PHP | small | Quick XML generation |
| invopop/gobl (it regime) | Go | — | Modern alternative |

### Mexico — CFDI
| Project | Lang | Stars | Notes |
|---|---|---|---|
| phpcfdi/* | PHP | 37 repos | The dominant CFDI ecosystem (sat-ws-descarga-masiva, credentials, etc.) |
| SAT-CFDI/python-satcfdi | Python | 142 | Most complete OSS CFDI lib; CFDI 3.2/3.3/4.0, Retenciones, Contabilidad, PAC integrations, FIEL renewal, SAT portal automation. MIT. |
| Angle/sat-cfdi | PHP | small | Pure PHP create+parse+validate |
| invopop/gobl.cfdi | Go | — | Modern alt |

### Spain — Verifactu / TicketBAI / FacturaE
| Project | Lang | Notes |
|---|---|---|
| Eseperio/verifactu | Meta | Living list of all Verifactu libs |
| Eseperio/verifactu-php | PHP | Active 2026 |
| squareetlabs/verifactu-sdk | Java | Chained invoices, NIF validation, QR |
| invopop/gobl.verifactu | Go | — |
| OCA/l10n-spain | Python/Odoo | Verifactu module being merged in |
| (TicketBAI) various Basque-region forks | Mixed | Fragmented |

### Poland — KSeF
| Project | Lang | Notes |
|---|---|---|
| artpods56/ksef2 | Python | **100% of KSeF 2.0 endpoints (73/73)**; Python 3.12+ |
| pprzetacznik/ksef-utils | Python | Utilities and examples |
| samupl/python-ksef | Python | XAdES + token auth |
| m32/ksef | Python | Scripts only |
| ArturSkowronski/ksef-cli | Python | CLI wrapper |
| invopop/gobl.ksef | Go | — |

### India — IRP (GST e-invoicing)
| Project | Lang | Stars | Notes |
|---|---|---|---|
| Mittal-Analytics/gst-e-invoicing | Python | small | Generate IRN via IRP portal |
| IamRamgarhia/Free-GST-Billing-Software | PWA | small | Full app, includes IRN gen |

### Saudi Arabia — ZATCA
| Project | Lang | Notes |
|---|---|---|
| aljbri/Zatca.Net | C# | Phase 2 |
| GeeSuth/GeeSuthSoft.KSA.ZATCA | C# | Unofficial helpers |
| ERPGulf/zatca_erpgulf | Python | MIT; FrappeCloud/ERPNext app |
| Beveren-Software-Inc/ZATCA_Integration | Python | ERPNext app |

### France — Chorus Pro / PPF
- **No official open-source French SDK exists.** AIFE publishes API docs on Piste, but no canonical client library.
- Community: tiny ad-hoc wrappers (vosfactures/API, David-IABB/facturepro-mcp).
- French B2B mandate is Sept 2026 — there will be a scramble. Real opportunity.

### Belgium — Peppol mandate (live Jan 2026)
- Recommand/recommand-peppol AGPL (above).
- Mostly Helger/phase4 underneath.

### Germany — XRechnung / E-Rechnung B2B mandate (Jan 2025 receive)
- KoSIT validator + mustangproject + horstoeko/zugferd cover this.

---

## 8. "All-in-one" attempts (developer-shaped tools)

| Project | Shape | Notes |
|---|---|---|
| **GOBL (Invopop)** | Library (Go) + hosted Invopop service | The dominant "developer-first universal invoice" play. Open-core. Already covers UBL/CII/FatturaPA/CFDI/KSeF/Verifactu/FacturaE/TicketBAI + Peppol BIS. **This is our closest direct conceptual competitor.** |
| **e-invoice-eu (gflohr)** | Library (TS) + CLI + REST server | Mature, runs in browser; Excel/CSV/ODS → EN16931. No Peppol AS4. |
| **Storecove** | Closed-source hosted SaaS + SDKs | Not OSS. Dominant in EU SaaS market. SDK code is permissive but core is closed. |
| **e-invoice.be** | Hosted API + open TS SDK | Closed core, open client. |
| **getpeppr** | Hosted on Storecove + open-feeling TS SDK | Marketing as "Stripe for Peppol". Closed core. |
| **B2BRouter** | Closed hosted | Not OSS. |
| **Pagero / Tungsten / Basware** | Closed enterprise | Not OSS. |

**Key observation:** **GOBL is the only serious developer-first OSS attempt at the breadth we're targeting.** It's currently the leader by a wide margin. Anything we ship needs to be honest about how it compares — either complement it (e.g. JS/WASM where GOBL is Go-only) or do something fundamentally different (e.g. OCR + AS4 in a single library).

---

## 9. Adjacent OSS (full-stack invoicing apps)

These aren't libraries but they ship invoicing logic that's relevant for cross-checking.

| Project | URL | Lang | Stars | License | Notes |
|---|---|---|---|---|---|
| **Invoice Ninja** | github.com/invoiceninja/invoiceninja | PHP/Laravel | 9.8k | **Elastic License** (not real OSS) | Most-used self-hosted invoicing app; has e-invoicing topic tag but limited Peppol/Factur-X depth |
| **Crater** | github.com/crater-invoice-inc/crater | PHP/Laravel + Vue + RN | ~7k | AGPL | Modern stack; no native e-invoicing standards |
| **InvoicePlane** | github.com/InvoicePlane/InvoicePlane | PHP | ~2k | MIT | Simple legacy |
| **InvoiceShelf** | github.com/InvoiceShelf/InvoiceShelf | PHP/Vue/RN | 1.7k | AGPL | Crater fork; v2.3.3 Apr 2026 |
| **SolidInvoice** | github.com/SolidInvoice/SolidInvoice | PHP/Symfony | ~1k | MIT | Cleaner architecture |
| **rossaddison/invoice** | github.com/rossaddison/invoice | PHP/Yii3 | small | n/a | Includes UBL 2.1 + Peppol + Storecove API connector |

None of these expose a clean library API for use in other applications. They are all platforms.

---

## 10. WASM / browser-native invoice processing

This is the area where almost nothing exists.

- **@e-invoice-eu/core (gflohr)** runs in browser (ESM/UMD) — but cannot generate PDFs from spreadsheets because that requires LibreOffice. Generates XML fine.
- **@stackforge-eu/factur-x** uses `libxml2-wasm` for XSD validation in JS — this is the closest existing "WASM in invoicing" data point. PDF/A-3 generation in pure JS.
- **node-zugferd** Node-only (no browser).
- **No project** ships browser-native validation against the official EN 16931/Peppol BIS/XRechnung **Schematron** rule packs. Schematron typically requires XSLT2/XSLT3 (Saxon-HE in Java). Saxon-JS exists but no one has bundled the rule packs against it.
- **No project** ships browser-native AS4 client (TLS+S/MIME+XMLDsig+EBMS3 over HTTP).
- **No project** ships browser-native strict PDF/A-3 builder + veraPDF-validated.

The "why not" is purely engineering effort — every primitive exists somewhere (XSLT3 in JS via Saxon-JS, libxml2-wasm, WebCrypto for signing, pdf-lib for PDF assembly). Nobody has glued them together because the Java stack already exists and the commercial vendors are happy hosting the work server-side.

This is **the single clearest moat for a WASM-first library**.

---

## Gaps in the OSS landscape

This is the highest-value section. Ordered roughly by leverage.

### 1. **Browser/WASM-native validation against EN 16931 / Peppol BIS / XRechnung Schematron rule packs**
   No JS/Rust/Go library can run the official rule sets natively. Everyone shells out to Java (KoSIT, phive) or skips Schematron. Building a portable Schematron engine (Saxon-JS bundled with the OpenPEPPOL/itplr-kosit/CEN-TC434 rule packs, kept in sync via CI) would be unique.

### 2. **A non-Java open-source Peppol AS4 client**
   phase4 + Oxalis-NG dominate. There is **zero** production-grade Rust/Go/TS AS4 library. Everyone outside JVM pays Storecove/Pagero/getpeppr. The cryptographic primitives exist everywhere now (WebCrypto, ring, BoringSSL); nobody has done the AS4 framing + Peppol SBDH + SMP lookup work outside Java. This is the **biggest commercial wedge** in the entire ecosystem.

### 3. **A complete OCR → EN16931 pipeline**
   Docling extracts. invoice2data extracts (template-based). None of them output validated UBL/CII/Factur-X XML. The full pipeline `PDF/image → structured fields → EN16931-conformant XML → embedded in PDF/A-3 → validated by Schematron` does not exist as a single OSS chain. Each link exists separately. Gluing them together is unique value, especially if VLM-driven (no templates).

### 4. **Browser-native PDF/A-3 builder with veraPDF-verified output**
   pdf-lib supports attachments but PDF/A-3 conformance is "best effort, please run veraPDF afterward". A pure-WASM library that produces strictly compliant PDF/A-3b/3u (correct XMP packets, OutputIntent, sRGB/PDF/X color profiles, AFRelationship metadata, MarkInfo, etc.) and ships its own continuous PDF/A-3 conformance harness using veraPDF-WASM is currently a hole.

### 5. **A maintained, language-agnostic intermediate representation that isn't tied to one commercial vendor**
   GOBL is excellent but it's Apache-2.0 owned by Invopop, a commercial company. The only EU-funded neutral attempt (EeISI/eIGOR XMLCEN) is dormant. There is room for a community-governed JSON schema that mirrors EN 16931 semantics directly, with parallel implementations in every popular language — or a structured codegen approach from a single OpenAPI/JSON Schema spec.

### Honorable-mention gaps (smaller but real)

6. **No open French PPF/Chorus Pro SDK** — the entire French B2B mandate (Sept 2026) currently lacks a canonical OSS client. Massive timing opportunity.
7. **Schematron in Rust** — the closest is xee, very early. Nobody has done the EN16931 rule pack on it.
8. **CLI ergonomics across ecosystems** — Mustang CLI is the best "ffmpeg-style" CLI today and it's clunky. There is room for a much friendlier `iv` / `inv` / similar binary that subsumes Mustang+akretion+phive functionality cross-platform.
9. **No serious open-source SMP/SML client outside Java** — Helger's peppol-smp-client is Java-only.
10. **No production-grade browser-native Schematron-aware UBL/CII editor** — useful for invoice prep UIs.

---

## Forking candidates

Projects that are permissively licensed, well-architected, and could realistically be foundations rather than starting from scratch.

| Project | License | Why it's foundational | Caveats |
|---|---|---|---|
| **invopop/gobl** | Apache-2.0 | The data model is already excellent: JSON intermediate, regime-per-country, signing built in, MCP server, broad country coverage. Tens of `gobl.*` sub-libs are individually small and clean. | Go-only consumers; need bindings or porting. Active commercial steward (Invopop) — they may not love a hard fork. Could contribute upstream instead. |
| **@e-invoice-eu/core** | WTFPL | Runs in browser, large parts auto-generated from PEPPOL-UBL docs, very modern TS code. Best starting point for a JS/TS+WASM toolkit. | WTFPL is uncomfortable for enterprise — would need relicensing or contributor agreement. Single maintainer. PDF generation gap (LibreOffice). |
| **mustangproject** | Apache-2.0 | Reference quality Factur-X/ZUGFeRD/XRechnung in Java; massive XSLT asset base for conversion. | Java-only, hard to port, mostly XSLT (96%). Better used as a *test oracle* than a base. |
| **akretion/factur-x** | BSD | Cleanest Python PDF/A-3 embedding code. Easy to port. | Limited scope. |
| **horstoeko/zugferd** | MIT | Cleanly designed profile-agnostic API. Excellent design reference even if you don't take the PHP code. | PHP only. |
| **josemmo/einvoicing** | MIT | The cleanest object model for EN 16931 in any OSS library — worth studying as schema reference. | PHP only; slowing. |
| **phax/phase4** | Apache-2.0 | The AS4 reference. If we want a non-Java AS4 client, phase4 is the spec implementation to mirror, not fork. | Java-only; would need to be re-implemented in Rust/TS, not literally forked. |
| **invoice-x/invoice2data** | MIT | Clean template-based extractor — a good starting point for the deterministic side of an OCR pipeline (with LLM/VLM added on top). | Template-per-supplier is dated. |
| **Docling (IBM)** | MIT | The PDF/image → structured extraction engine to build on. Already huge community + IBM sponsorship. | Needs invoice-schema and EN16931 mapping bolted on; Python only (but has ONNX-exportable models). |
| **veraPDF-library** | MPL-2.0 / GPL+ | The PDF/A-3 conformance oracle. Use it (don't fork) in CI to validate everything we produce. | Heavy Java; consider veraPDF-rest service. |
| **invopop/gobl.cfdi, gobl.verifactu, gobl.ksef** | Apache-2.0 | Per-country mapping logic; well-engineered. Even if we don't reuse the Go code, the mapping tables are gold. | Tied to GOBL schema. |

**Recommended posture:** Treat **GOBL as the de-facto OSS reference for cross-country invoice semantics** and either (a) ship a JS/WASM/Rust front-end that interops via the GOBL JSON schema, or (b) fork the GOBL JSON schema as our own and reimplement around it. Treat **mustangproject + akretion/factur-x + veraPDF** as conformance oracles in CI. Treat **phase4** as the AS4 spec implementation to mirror in our target language.

---

## Appendix: license cheat-sheet for would-be forks

- **MIT / BSD / Apache-2.0 / WTFPL** → safe to fork.
- **MPL-2.0** (veraPDF) → file-level copyleft only; usable.
- **LGPL-3.0** (Oxalis, Oxalis-NG) → can be linked dynamically; modifications to the library itself must be open.
- **GPL-3.0** (marker) → contaminates downstream.
- **AGPL-3.0** (Recommand Peppol, InvoiceShelf) → network-use triggers source disclosure — usually fatal for SaaS products.
- **Elastic License** (Invoice Ninja) → not OSI-approved; commercial restrictions.

The most commercially safe foundations are: **GOBL, mustangproject, akretion/factur-x, horstoeko/zugferd, josemmo/einvoicing, phase4, phive, veraPDF, Docling, invoice2data, e-invoice-eu**.
