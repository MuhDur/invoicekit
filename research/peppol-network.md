# Peppol Network — Deep Technical & Commercial Dive

**Research date:** May 2026
**Scope:** What does it take to become an operational Peppol Access Point (AP) — costs, timeline, protocol stack, open-source options, liabilities, and the per-envelope economic floor.

---

## 1. Peppol Authority Structure

### 1.1 OpenPeppol AISBL

OpenPeppol AISBL is the Brussels-based international not-for-profit association that owns and operates the Peppol network. It defines the specifications, governs the PKI, and signs the Service Provider Agreement (formerly "Transport Infrastructure Agreement", TIA) with each AP.

OpenPeppol's role:
- Maintains the **eDelivery specifications** (AS4 profile, SBDH, SMP, SML).
- Maintains the **Peppol BIS** business interoperability specifications (Billing 3.0, PINT, etc.).
- Operates the **Peppol PKI** (issues AP and SMP certificates via DigiCert One Trust Lifecycle as of the 2025 migration).
- As of 2026 it has **insourced the SML** itself (was previously operated by the EU CEF Digital Building Blocks team). Migration deadlines: SMP registrations by **31 May 2026**; AP lookup (DNS resolution) by **31 August 2026**.

### 1.2 Peppol Authorities (PAs)

A Peppol Authority is a national/regional body that has signed an agreement with OpenPeppol granting it the right to onboard Service Providers in its territory and define overlay rules ("specific national requirements" or "domains"). Examples:

| Country | Peppol Authority | Notes |
|---|---|---|
| Germany | **KoSIT** (Koordinierungsstelle für IT-Standards) | XRechnung is the national CIUS |
| France | **DGFiP** (since July 2025) | PPF + PDP regime; layered on Peppol |
| Belgium | **FOD Financiën / FPS Finance** | Default channel for the 2026 B2B mandate |
| Netherlands | **NPa / Logius / PIANOo** | One of the earliest PAs |
| Italy | **AgID** | Coexists with FatturaPA/SDI |
| Singapore | **IMDA** | First non-EU PA, since May 2018 |
| Australia | **ATO** (Australian Taxation Office) | A-NZ PINT profile |
| New Zealand | **MBIE** | A-NZ PINT profile |
| Japan | **Digital Agency (Cabinet)** | JP PINT |
| Norway | **DFØ / Anskaffelser.no** | Mandatory for B2G; B2B mandate from Jan 2027 (announced March 2026) |
| Sweden | **DIGG** | B2G mandatory since 2019 |
| UK | **HMRC** | PEPPOL UK BIS |
| OpenPeppol AISBL (residual) | Acts as the PA for countries without a national PA | |

Bottom line: **you do not "join Peppol once" and operate everywhere.** Each country's PA must accept you, and many enforce additional rules (insurance, ISO 27001, local validation rules, reporting obligations) before you can route into their participant ID space.

### 1.3 SML, SMPs, and the discovery chain

- **SML (Service Metadata Locator)** — a single, central DNS-based service operated by OpenPeppol. Given a Participant ID (e.g. `0192:991825827` for a Norwegian org), the SML returns the URL of the SMP that publishes that participant's metadata.
  - Implementation: DNS records (historically CNAME; **NAPTR/U-NAPTR became primary 1 Nov 2025; CNAME deprecated 1 Feb 2026**). Lookup key is a SHA-256 hash of the canonicalised participant ID, encoded into a hostname.
- **SMP (Service Metadata Publisher)** — a per-AP (or per-group) HTTPS REST service that publishes, for each participant it serves: supported document types, process IDs, the receiving AP's URL, and the AP's signing certificate. From **1 Feb 2026**, every SMP must run **HTTPS-only with TLS certificates from an approved public CA**.
- **Discovery flow:** Sender AP queries SML (DNS) → gets SMP URL → queries SMP (HTTPS, signed XML response) → gets receiver AP endpoint + receiver's AS4 signing cert → sends AS4 message.

### 1.4 Certificate hierarchy

Two distinct PKI trees:

1. **Peppol PKI (private, OpenPeppol-operated, via DigiCert)** — issues:
   - **AP signing/encryption certificates** (used inside the AS4 SOAP envelope for WS-Security).
   - **SMP signing certificates** (used to sign SMP XML responses).
   - **Test (PILOT) vs Production** certificates are separate trust chains.
   - Migrated in H2 2025 from `DigiCert Managed PKI v8 (MPKI8)` to **`DigiCert One Trust Lifecycle (DOTL)`** roots; all SPs had to roll certificates.
2. **Public TLS PKI** — for the HTTPS endpoint of your AP and your SMP. **Not** issued by OpenPeppol — you use Let's Encrypt, DigiCert, Sectigo, etc.

---

## 2. Becoming a Peppol Service Provider (Access Point)

### 2.1 Step-by-step roadmap with costs

| Stage | Activity | Typical duration | Direct cost (EUR) |
|---|---|---|---|
| 0 | Decide AP-only vs AP+SMP vs AP+SMP+End User. AP-only is cheaper; AP+SMP is required if you want to be a *full* SP and onboard your own customers as participants. | — | — |
| 1 | Establish legal entity, basic security posture, AS4 stack PoC against the **Peppol Test Bed (PILOT)**. | 2-4 months | dev cost only |
| 2 | **OpenPeppol membership application** — Candidate Service Provider. Pay sign-up + first-year annual fee. Sign Member Agreement. | 2-6 weeks | **€1,050–€5,400 sign-up + €1,850–€8,250 annual** (size-dependent, see §2.2) |
| 3 | Obtain **PILOT** Peppol PKI AP certificate (and SMP cert if AP+SMP). Configure test SML/SMP. | 1-2 weeks | included in membership |
| 4 | Sign with a **Peppol Authority** for each country you want to operate in. Pay PA fees if applicable (most EU PAs do not charge SPs directly today, but the OpenPeppol "national domain" fee structure has shifted some cost back to SPs). | 4-12 weeks | €0–€12,500/yr per "national domain", depending on PA |
| 5 | **Conformance / accreditation testing**. Phase 1: basic AS4/SBDH/BIS sending+receiving against the Test Bed (now automated via the Onboarding Platform). Phase 2: country-specific tests (e.g. AgID, IMDA, ATO each have their own Phase 2). | 4-8 weeks | dev cost only |
| 6 | **ISO/IEC 27001 certification** (now mandatory for all APs, with the deadline rolled out per PA — Netherlands, Belgium, France already enforce; Australia gives a 12-month grace from accreditation). | **6-12 months** (parallelisable with steps 1-5) | **€15,000–€60,000** for a small org (auditor + consultancy + tooling), plus ongoing surveillance audits |
| 7 | **Professional indemnity insurance** — mandatory in AU/NZ at AUD 1m+/occurrence; Belgium, Singapore PAs require it implicitly via the SP Agreement. | 2-4 weeks | **€2,000–€10,000/yr** premium |
| 8 | **Certification Fee** to OpenPeppol (one-off at first certification, then re-billed annually as part of the "Certified" tier). | concurrent with step 6 | **€1,500 (AP-only) / €2,500 (AP+SMP)** |
| 9 | Production PKI issuance (AP cert + SMP cert), register in production SML, **go live**. | 1-2 weeks | included |
| 10 | Per-country go-live for each additional Peppol Authority (repeat steps 4-5 for each). | 4-8 weeks per country | depends |

**Realistic total time from zero to live in one country: 6-12 months.** The ISO 27001 audit is almost always the long pole; if you start it in parallel on day 1, you can compress to ~6 months.

**Realistic total direct cost (year 1, AP+SMP, small org, 1 country):**

- OpenPeppol membership (S1-S2 AP+SMP): €1,800 sign-up + €2,750 annual = **€4,550**
- OpenPeppol certification fee: **€2,500**
- TLS certs (commercial): **€500/yr** (or free with Let's Encrypt)
- ISO 27001 (initial cert + consultancy): **€20,000–€40,000** amortised year 1
- PI insurance: **€3,000–€5,000**
- Peppol Authority national fee (varies; many EU PAs do not pass through): **€0–€12,500**
- **Subtotal: ~€30,000–€65,000 hard cost in year 1**, plus engineering.

Engineering effort to build a working AP from scratch (DIY, see §4): **roughly 4-8 person-months** if reusing battle-tested open-source crypto libs; substantially less if you wrap `phase4`.

### 2.2 OpenPeppol fee schedule (effective 1 July 2025 for new members; 1 Jan 2026 for existing) — sign-up / annual

| Tier | S1-S2 (1-50 emp.) | S3 (51-250) | S4 (251-2500) | S5 (>2500) |
|---|---|---|---|---|
| **AP only** (Cat 2b) | €1,050 / €1,850 | €2,200 / €3,100 | €2,550 / €4,250 | €2,950 / €5,500 |
| **AP + SMP** (Cat 2a) | €1,800 / €2,750–€3,700 | €4,250 / €5,250 | €4,850 / €6,750 | €5,400 / €8,250 |
| **SMP only** (Cat 2c) | €1,050 / €5,000 | €2,200 / €5,000 | €2,550 / €5,000 | €2,950 / €5,000 |
| Certification fee (one-off, then annual top-up) | €1,500 AP / €2,500 AP+SMP / €1,000 SMP-only | | | |
| Peppol Authority sign-up/annual (when acting as PA) | €25,000 / €25,000 (Post+Pre+Lookup); €12,500 / €12,500 per national domain | | | |
| End User membership | €650 / €1,250 | €1,650 / €2,500 | €1,650 / €3,650 | €1,650 / €4,500 |
| Observer | €250 / €1,500 | | | |

We would join as **Cat 2a, S1-S2 — €1,800 sign-up + €2,750 annual + €2,500 certification = €7,050 year 1 to OpenPeppol**.

### 2.3 Insurance / financial requirements

- **AU/NZ:** explicit AUD 1m PI insurance, audited annually.
- **EU PAs:** the OpenPeppol Service Provider Agreement obliges you to maintain "adequate insurance" — not quantified, but in practice €1m+ PI is the de facto floor for due diligence by PAs (Belgium, Netherlands, France).
- **No minimum capital** is mandated by OpenPeppol, but PAs perform due diligence (financial statements, beneficial ownership) before signing.

### 2.4 Certificate procurement

- **Peppol AP/SMP certs:** issued by OpenPeppol's PKI administrator, signed by DigiCert DOTL root. Application via OpenPeppol Service Desk on Jira. No per-certificate fee — bundled into membership. Test (PILOT) certs first, then production after PA signs off.
- **Public TLS cert** for your AP endpoint & SMP: any approved public CA. Let's Encrypt works, but operationally many APs prefer paid DV/OV certs with longer validity for stability.

### 2.5 Per-country approvals required

You are accredited **per Peppol Authority**. Practically:

- EU members can typically use the OpenPeppol-residual PA for countries without a national PA. This covers maybe ~10 smaller EU markets implicitly.
- For Germany, France, Belgium, Italy, Netherlands, Nordics, Singapore, Australia/NZ, Japan, UK — **you must sign each PA separately**, and each will impose its own overlay (formats, validation, reporting, insurance, sometimes ISO 27001 specifics).
- France's PDP regime is a special case: from Sept 2026, to be a sender/receiver of B2B invoices for French entities, you need to be a **PDP** (Plateforme de Dématérialisation Partenaire), which is an *additional* registration with DGFiP (ISO 27001 + SecNumCloud-aware hosting + capital requirements). Being a Peppol AP is *necessary but not sufficient*.

---

## 3. AS4 Protocol Stack

The Peppol AS4 profile is layered on **OASIS ebMS3 / AS4** with a strict OpenPeppol profile.

### 3.1 PMode configuration (the "agreement" between two APs)

Peppol uses a **single fixed PMode** for all AP-to-AP traffic:

- `PMode.Agreement` = `urn:fdc:peppol.eu:2017:agreements:tia:ap_provider`
- `PMode[1].BusinessInfo.MPC` = `http://docs.oasis-open.org/ebxml-msg/ebms/v3.0/ns/core/200704/defaultMPC`
- One-way **push** MEP only (no pull, no two-way sync in the core profile).
- **Reception Awareness Feature** enabled — receiver must return a non-repudiation receipt (NRR) referencing the message ID and digest.
- **AS4 Compression** enabled; payload always a MIME attachment (no inline payloads).
- **WS-Security 1.1** signing + encryption with the AP certificates; SHA-256 / RSA-SHA256; AES-128-GCM is the current required cipher.

### 3.2 ebMS3 messaging

- SOAP 1.2 with the ebMS3 `Messaging` header.
- `UserMessage` for business payloads; `SignalMessage` for receipts and errors.
- `MessageInfo`, `PartyInfo` (with AP identity), `CollaborationInfo` (service + action), `PayloadInfo`, `MessageProperties` (where Peppol stamps `originalSender` / `finalRecipient`).

### 3.3 SBDH (Standard Business Document Header)

- **Mandatory** in every Peppol message, immediately wrapping the business document.
- Carries `DocumentIdentification` (type, schema, version), `Sender` and `Receiver` participant IDs in Peppol scheme `iso6523-actorid-upis`, plus business scope and process identifier.
- Standalone SBDH (the "no-payload" CEF variant) is **forbidden** in Peppol.

### 3.4 Payload formats

- **UBL 2.1** is dominant (Invoice, CreditNote, Order, OrderResponse, DespatchAdvice, Catalogue, MLR/BLR).
- **UN/CEFACT CII** (Cross-Industry Invoice) supported for billing (e.g. Germany's ZUGFeRD/XRechnung CII variant, France's Factur-X).
- Always inside Peppol BIS profiles (BIS Billing 3.0 today; PINT for AU/NZ/SG/JP/MY; EU PINT v1.0.0 published 2 Oct 2025; **BIS 4.0 expected to merge BIS+PINT in 2026**).

### 3.5 Reception receipts (non-repudiation)

- Receiver AP returns a **Receipt** SignalMessage containing the `NonRepudiationInformation` element listing the digests of all signed parts of the original UserMessage.
- This receipt is the **legal proof of receipt** between the two APs — must be persisted by both sides for the statute of limitations (typically 7+ years).
- Failure to return a receipt or returning an Error message has explicit semantics in the SP Agreement (sender must retry per the Reception Awareness Feature; after exhaustion, message is considered undeliverable and reported).
- Independent of this, BIS 3.0/Billing also defines **MLR (Message Level Response)** and **BLR (Business Level Response)** as *application-layer* business responses — these are separate Peppol documents sent back end-to-end through the 4-corner model, not AS4 SignalMessages.

---

## 4. Open-Source AS4/Peppol Implementations

### 4.1 Landscape table

| Project | Language | License | Maturity | Scope | Notes |
|---|---|---|---|---|---|
| **phase4** (phax) | Java 11+ | Apache 2.0 | Production, dominant in DACH | AS4 library + Peppol client/servlet | Most actively maintained; Philip Helger personally tracks every Peppol spec change. **De facto reference.** `phase4-peppol-standalone` is a Spring Boot 3 turnkey AP. |
| **Oxalis** | Java 11+ | Apache 2.0 | Production, dominant in Nordics | Full Peppol AP + SBDH + SMP client | Maintained by `OxalisCommunity` (post-Difi/Sendregning fork). Heavier than phase4. |
| **Holodeck B2B** (Chasquis) | Java 8+ | GPLv3 (core) | Production | Generic ebMS3/AS4 gateway + commercial Peppol plug-in | Open core, *Peppol package not free* |
| **mendelson AS4** | Java | Open + commercial dual | Production | Generic AS4 gateway with Peppol adapter | Older codebase, GUI-driven, mostly EDI shops |
| **Domibus** (EU CEF) | Java | EUPL | Production | Generic AS4 gateway used by EU institutions for eDelivery | Not Peppol-profile out of the box; requires configuration. |
| **OpenPEPPOL/edec-as4** | Java + spec | Apache 2.0 | Spec + reference test material | Reference test data, not a runtime impl | |
| **phoss-ap** (phax) | Java | Apache 2.0 | Production | Standalone AP on phase4 + Spring Boot | Newer than `phase4-peppol-standalone`. |
| **ion-SMP** | Java | Apache 2.0 | Production | SMP server | Common SMP choice. |
| **node42 Peppol AS4 sender** | Node.js (TypeScript) | MIT (blog/article) | PoC (March 2026) | Sender only, ~500 LOC | First demonstrated pure-JS AS4+SMP+SML+WS-Security sender. **No production users known yet.** No receiver/servlet side. |
| **(Go)** | — | — | — | — | **No public open-source Peppol AS4 impl in Go as of May 2026.** |
| **(Rust)** | — | — | — | — | **No public open-source Peppol AS4 impl in Rust as of May 2026.** |
| **(Python)** | — | — | — | — | No production-grade Peppol AS4 impl in Python. Some companies have internal Python wrappers around phase4 via JNI/sidecar; nothing publicly maintained. |
| **(.NET)** | — | — | — | — | A few proprietary .NET AS4 implementations (Mendelson, Comarch internal); no notable OSS. |

**Headline:** the Peppol ecosystem is **monoculture Java**. Every open-source production-grade AP is JVM. The Node.js sender from March 2026 is the first credible proof that the protocol itself is implementable in non-JVM stacks "in 500 lines" — but it's a sender only, no receiver/SMP/SML server, no test coverage, no production deployment.

### 4.2 Why Java dominates

Three reasons, in order of weight:

1. **WS-Security / XML-DSIG / XML-Enc** maturity — `Apache WSS4J` and `Apache Santuario` are essentially the only "boring" implementations of these specs that handle the full Peppol-required feature set (canonicalisation, multi-attachment signing, GCM encryption). Replicating their correctness elsewhere is genuinely hard.
2. **Historical inertia** — the original CIPA/CEF eDelivery reference implementations (Domibus, Holodeck) were Java; OpenPeppol itself ships Java reference test tooling.
3. **Network effect** — phase4 is patched within days of any spec change. A non-Java fork would constantly lag.

### 4.3 Is non-JVM feasible?

Yes, but with caveats:

- **Sender side** is the easier half. The node42 article proves it in 500 LOC of TypeScript using only Node's `crypto` + a small XML signing helper. Rust can do the same with `xmlsec` bindings or pure-Rust `rxml` + `rsa` + `aes-gcm`. Go can do it with `goxmldsig` (already used for SAML).
- **Receiver side** (the AS4 servlet that has to validate WS-Security on inbound, return signed receipts, persist NRR evidence, replay-protect, handle PMode validation, error mapping, and remain conformant under the **Peppol Test Bed Phase 1 automated suite**) is substantially harder. Most of the bugs caught in Phase 1 testing are in receiver-side edge cases (compression failure paths, signature reference orderings, attachment digest mismatches).
- **SMP server** is the easiest part — it's a REST/HTTPS service serving signed XML, totally feasible in any language.

**Verdict for our project:** Rust/Go are absolutely viable for sender + SMP. For the receiver, expect 2-3x the engineering effort of phase4 because we'd be writing the WS-Security/XML-DSIG glue ourselves. WASM-native is feasible for the *cryptographic and serialization* parts but not for the long-running AS4 servlet (sockets, persistence, retry loops).

---

## 5. Turnkey AP vs DIY — Commercial Landscape & Pricing

### 5.1 Commercial APs (pricing snapshots, May 2026, public/observed)

| Provider | Country base | Model | Indicative pricing |
|---|---|---|---|
| **Storecove** | Netherlands | Volume-tiered SaaS API | From ~€39/mo bundling 100 docs; €1/doc overage; enterprise tiers from ~€0.04-€0.10/doc at 100k+/mo |
| **B2BRouter** | Spain | Per-document + per-connection | ~€0.05-€0.20/doc depending on tier and country overlays |
| **ecosio** | Austria | Enterprise B2B middleware (Peppol + EDIFACT + X12) | Custom; €0.05-€0.30/doc typical at scale, plus integration fees |
| **Pagero / Thomson Reuters ONESOURCE** | Sweden | Enterprise Peppol + global compliance | Custom; commitment-based contracts, €0.10-€0.50/doc at mid-volume |
| **Comarch** | Poland | Enterprise EDI/e-invoicing | Custom; integration-heavy |
| **Babelway** | Belgium (now part of Quadient) | iPaaS B2B/EDI | Custom; project-based |
| **EDICOM** | Spain | Global Peppol + LATAM CTC | Custom; mid-market and enterprise focus |
| **Qvalia** | Sweden | Self-service Peppol | From ~€25/mo, per-doc above plan |
| **Tickstar (part of Pagero)** | Sweden | **Wholesale AP** — they run the AP, you put your brand on it | ~€0.05-€0.30/doc to the reseller, large minimums |
| **Phoss / Helger** | Austria | Open-source backed consulting | Self-host phase4 + paid support |

### 5.2 What "buying an AP" actually means

Three commercial models:

1. **End-user API** (Storecove, Qvalia, B2BRouter retail tiers) — you use their REST API to send invoices, they handle Peppol entirely. Easy. Your customers' invoices flow through their AP's identity. **You are not an AP.**
2. **Wholesale / white-label AP** (Tickstar, Storecove enterprise) — they run the AP, you appear as the SP to your customers. You bill your customer; they bill you per envelope. Sometimes you can register your own SMP and present your branding in the Peppol Directory.
3. **Embed / co-license** — you self-host phase4 or Oxalis (or pay for a managed deploy from ecosio/EDICOM) and become a Peppol SP yourself.

---

## 6. Embedded AP vs Outsourced AP — Implications for a SaaS

### 6.1 Can a SaaS embed an AP?

Yes — there is **no rule against a SaaS being its own Peppol AP**. Many do (Tickstar themselves, Storecove, Pagero, Visma, several accounting SaaSes in NL/BE/SE). Conditions: pass §2 onboarding, accept SP Agreement liability, maintain ISO 27001 on the production environment.

### 6.2 Liability & compliance implications

| Dimension | Embedded AP | Outsourced AP (you proxy through Storecove etc.) |
|---|---|---|
| Liability for delivery (4-corner SLA) | **You** (per OpenPeppol SP Agreement: deliver reliably, return NRR, retain evidence, respond to PA inquiries) | The upstream AP carries Peppol-layer liability; you only owe your customer the SaaS SLA |
| GDPR data processor role | You are the controller's processor and the network's transport intermediary | Mixed: upstream is sub-processor; document carefully |
| ISO 27001 scope | Production AP infra must be in scope | Your SaaS still needs ISO 27001 for many EU enterprise customers, but the AP infra is out of scope |
| PA per-country onboarding | You repeat it per country | One-and-done — the upstream AP already has it |
| BIS spec churn | You patch your stack within OpenPeppol's deadlines | Upstream handles it |
| Pricing power | You set your own per-envelope price; floor is your true cost | You pay €0.04-€0.30/doc upstream; you can't undercut that |
| Strategic moat | You own the participant relationship; you can offer features upstream can't | You're at the mercy of upstream's roadmap |
| Time to market | 6-12 months | 1-2 weeks |

---

## 7. Document types supported

The Peppol network is **not just for invoices.** Current production document types (BIS 3.0 / PINT specs):

| Domain | Document | Profile |
|---|---|---|
| Post-Award (procurement) | Invoice | BIS Billing 3.0 / PINT Billing |
| | Credit Note | BIS Billing 3.0 |
| | Corrective Invoice (Italy) | PINT-IT |
| | Self-Billing Invoice | BIS Self-Billing 3.0 |
| Pre-Award | Catalogue + Catalogue Response | BIS Catalogue 3.0 |
| | Order + Order Response | BIS Ordering 3.0 |
| | Order Agreement | BIS Order Agreement 3.0 |
| | Advanced Ordering (change/cancel) | BIS Advanced Ordering 3.0 |
| | Despatch Advice | BIS Despatch Advice 3.0 |
| | Punch-Out | BIS Punch-Out 3.0 |
| Response | Message Level Response (MLR) | technical/syntactic ack |
| | Business Level Response (BLR) / Invoice Response | business accept/reject |
| Tender (eTendering domain) | Tender, Tender Question, etc. | EU CEF eTendering BIS |

**Mandatory subset for Billing 3.0** (what an AP claiming "Peppol Billing capability" must support):
- UBL 2.1 Invoice + CreditNote.
- Schematron validation against EN 16931 core rules + Peppol BIS Billing 3.0 customisations + any applicable national CIUS (e.g. XRechnung in DE).
- Generation and consumption of MLR.

---

## 8. Country-Specific Overlays

| Country | Peppol BIS base | National overlay / CIUS | Sender mandate | Receiver mandate | Notes |
|---|---|---|---|---|---|
| **Germany** | BIS Billing 3.0 | **XRechnung** CIUS (KoSIT) | Phased B2B: >€800k turnover from 1 Jan 2027; all others from 1 Jan 2028 | All VAT-registered companies since 1 Jan 2025 | Peppol is *one* of several legal channels; XRechnung is also acceptable over email/portal |
| **France** | BIS Billing 3.0 (with Factur-X allowed) | **PPF + PDP regime** (DGFiP) | All companies from 1 Sept 2026 (issue) | All companies from 1 Sept 2026 (receive) | **Must be a PDP, not just a Peppol AP.** Peppol is the recommended transport between PDPs. |
| **Belgium** | BIS Billing 3.0 | None (pure BIS) | **All B2B for Belgian VAT entities from 1 Jan 2026** | Same | Peppol is the *default* channel; 4-corner; CTC reporting (5-corner) added Jan 2028 |
| **Netherlands** | BIS Billing 3.0 | NL-CIUS | B2G mandatory, B2B voluntary | B2G mandatory | NPa was first non-Nordic PA; strict on ISO 27001 |
| **Italy** | BIS Billing 3.0 | Coexists with **FatturaPA via SDI** | B2B already covered by SDI since 2019 | Same | Peppol B2G; AgID runs its own AP test bed |
| **Singapore** | **PINT-SG** (not BIS Billing 3.0 anymore) | IMDA overlay | GST InvoiceNow phased rollout 2025-2026; mandatory for all GST-registered by 2027 | Same | First non-EU PA; 5-corner model with IRAS |
| **Australia / NZ** | **PINT A-NZ** | ATO/MBIE overlay | B2G receive mandatory; B2B voluntary | B2G receive | AUD 1m PI insurance required |
| **Japan** | **JP PINT** | Digital Agency | Voluntary, growing | Voluntary | |
| **Malaysia** | PINT (under development) | LHDN/MyInvois bridge | Phased B2B mandate active 2024-2026 | Same | MyInvois (CTC) primary; Peppol gateway emerging |
| **UK** | BIS Billing 3.0 | UK NHS BIS extension | B2G NHS use mandated | Same | HMRC consulting on broader B2B mandate |
| **Nordics (SE/NO/DK/FI)** | BIS Billing 3.0 | local CIUSes | B2G mandatory; NO B2B from 1 Jan 2027 | Same | Highest e-invoicing penetration in the world |
| **Poland** | BIS Billing 3.0 (B2G) | **KSeF** is the primary B2B channel (NOT Peppol) | KSeF mandatory phased 2026-2027 | KSeF | Peppol B2G only |

---

## 9. Per-Envelope Economic Floor

What does it *actually* cost the AP, in pure marginal terms, to deliver one Peppol envelope (~50-200 KB typical, up to 25 MB max)?

### 9.1 Component cost stack (one envelope, sender side, AWS-class infra, mid-2026 prices)

| Component | Cost per envelope | Notes |
|---|---|---|
| SML DNS lookup | ~€0.0000005 | One DNS query, cacheable for the TTL |
| SMP HTTPS GET | ~€0.00001 | Egress + ~5KB response |
| AS4 outbound HTTPS POST (signing, encryption, transmit) | ~€0.00005 | ~100KB egress + CPU (signing dominates) |
| Compute for WS-Security + payload validation | ~€0.0001-€0.0005 | Single-thread Java/Rust: ~30-100ms |
| Receipt validation + persistence | ~€0.00002 | |
| **Marginal infra cost** | **~€0.0002-€0.001 (~$0.0002-$0.001)** | |
| Storage for 7-year evidence retention (NRR + payload) | ~€0.00005/envelope (amortised, S3 IA, 200KB avg) | |
| Bandwidth amortisation (assuming bi-directional traffic) | ~€0.00005 | |

**Pure marginal infra floor: well under €0.005 per envelope** — effectively *fractions of a cent*.

### 9.2 Fully-loaded cost (adding fixed costs amortised)

| Fixed cost | Annual € | Break-even envelopes/yr at €0.05/env |
|---|---|---|
| OpenPeppol membership (AP+SMP, S1-S2) | €4,250 (incl. cert. fee amortised) | 85,000 |
| ISO 27001 (annual amortised after initial) | €15,000 | 300,000 |
| PI insurance | €4,000 | 80,000 |
| Per-country PA fees | €0-€12,500 | up to 250,000 |
| 24/7 ops (on-call, monitoring, incident response) — 0.5 FTE | €60,000 | 1,200,000 |
| Engineering maintenance (spec churn, ~0.5 FTE) | €60,000 | 1,200,000 |
| **Total fixed** | **~€140,000-€155,000/yr** | **~2.8-3.1M envelopes/yr to break even at €0.05/env** |

### 9.3 Strategic implication

- The **marginal cost is effectively zero** (~€0.001).
- The **fixed cost is the dragon** — ~€150k/yr just to keep an AP alive and compliant, before you have a single customer.
- Commercial APs charging €0.04-€0.30/envelope are pricing on **gross margin to fund the fixed cost + R&D + sales**, not marginal cost.
- An open-source-backed self-hosted AP serving >3M envelopes/yr is dramatically cheaper than outsourcing. Below that volume, **outsourcing wins on TCO** (and on speed-to-market).

---

## 10. "Can we DIY a Peppol AP?" — Verdict

### Verdict: **Phased DIY — start outsourced, embed AP in year 2.**

| Phase | Approach | Rationale |
|---|---|---|
| **Phase 0 (Month 1-6)** — MVP | Integrate with **Storecove or Qvalia API** as upstream AP. Ship our product end-to-end through them. | Zero Peppol onboarding cost; full BIS support immediately; we focus engineering on our product, not on AS4. |
| **Phase 1 (Month 6-12)** — Open-source AP behind feature flag | Stand up `phase4-peppol-standalone` in PILOT against the Peppol Test Bed. Begin ISO 27001 process. Submit OpenPeppol membership. | Learn the protocol, prove the stack, accumulate test artefacts. |
| **Phase 2 (Month 12-18)** — Production AP, one PA | Go live as our own AP in our home jurisdiction. Route a % of traffic. Keep Storecove as fallback. | We own the participant relationship; we shave the per-envelope margin. |
| **Phase 3 (Month 18+)** — Expand PAs | Onboard with each PA where we have customer volume. | Direct presence in DE/FR/BE/NL once justified. |
| **Phase 4 (year 3+)** — Optional Rust/Go reimplementation | Replace phase4 internals with native-Rust/Go AS4 implementation. Keep phase4 as conformance oracle. | Performance, dependency-shedding, supply-chain control. **Only if volume justifies it.** |

### What we *can* DIY confidently
- The **business document validation** (Schematron, EN 16931, BIS profiles) — straightforward XML/XSL work, easy in any language.
- The **SMP server** — REST + signed XML; trivial in Rust/Go.
- The **SML client** — just DNS + URL construction.
- The **sender-side AS4** in Rust/Go — feasible (node42 proved it for Node.js).
- The **product layer** above the AP (mapping, ERP integration, UI).

### What we should NOT DIY
- **The Peppol PKI itself** — we don't issue certs; OpenPeppol does. Don't build a "shadow PKI."
- **The AS4 receiver-side WS-Security validator from scratch in year 1** — too many edge cases; use phase4 first, port later if needed.
- **ISO 27001 audit work** — hire a consultancy that has done it for SaaS before. Cheaper than reinventing.
- **Public TLS CA** — use Let's Encrypt or paid public CA.
- **National CIUS conformance** — use the official OpenPeppol validation artefacts and Peppol Test Bed; do not write your own validators.

---

## 11. Liability Surface as a Peppol AP

Per the OpenPeppol Service Provider Agreement (and per each PA's local supplement), we would be on the hook for:

1. **Reliable delivery and receipt evidence (NRR)** for every envelope. Failure → escalation by recipient AP → PA inquiry → possible suspension.
2. **Integrity & confidentiality** — payload must arrive unmodified, signed, encrypted in transit. Cert misuse triggers PKI revocation.
3. **Participant identity** — we must verify our customers' Peppol identifiers and not register them improperly. Phantom/fraudulent participant registrations are a major audit area.
4. **GDPR / data protection** — payloads are personal data (invoice line items). Processor obligations under Art. 28 GDPR run with the message.
5. **Peppol Reporting** — quarterly reporting of message volumes to OpenPeppol (Peppol Reporting v1.0.2 onwards). Non-reporting → membership suspension.
6. **PKI/cert rotation** — we must complete migrations (e.g. DOTL, SML insourcing) by deadline or lose connectivity.
7. **Spec conformance** — must remain conformant with each BIS update. BIS 4.0 in 2026 is the next forced upgrade.
8. **ISO 27001 maintenance** — annual surveillance audits.
9. **Insurance maintenance** — PI policy current and at the contractually agreed minimum.
10. **Sanctions/AML** — we cannot knowingly route messages for sanctioned entities; the SP Agreement embeds OFAC/EU sanctions compliance.
11. **Country-specific liabilities** — e.g. France PDP regime adds tax-reporting obligations on top of Peppol; Belgium adds 2028 CTC reporting.

Suspension risk in practice: PAs do suspend SPs (rare but happens) for repeat reporting failures, PKI hygiene failures, or critical security incidents. The reputational cost of suspension is far worse than the operational cost.

---

## 12. The Future of Peppol

### 12.1 Glossary clarifications
- **CIUS** = "Core Invoice Usage Specification". A *restricting* customisation of EN 16931 — a national/sectoral profile that narrows the standard (XRechnung is a CIUS).
- **MIG** = "Message Implementation Guide". A document describing how a specific BIS is implemented in a specific industry/country context.
- **PINT** = "Peppol International". A *generalised* invoice data model, jurisdiction-agnostic, that anchors per-country profiles (PINT-SG, PINT A-NZ, PINT-JP, PINT-MY, EU-PINT).
- **CTC** = "Continuous Transaction Control". Real-time/near-real-time fiscal data reporting to tax authorities (Italy SDI, France PPF, Singapore IRAS, Belgium 2028, Spain Veri*factu).
- **5-corner model** = Peppol's evolution to insert the tax authority as a 5th corner, receiving CTC data alongside the buyer.

### 12.2 What's coming

- **BIS 4.0** (expected 2026) — merges BIS Billing 3.0 and PINT EU into one harmonised spec; cross-border interoperability simplified.
- **EN 16931 mid-2026 update** — incorporates ViDA fields, especially mandatory `BT-` codes for CTC reporting (issued date precision, tax point date, etc.).
- **EU CTC (ViDA Digital Reporting Requirements)** — by 2030 (originally 2028, pushed to 2030 in final ViDA text), all intra-EU B2B must use structured e-invoicing with near-real-time CTC reporting. Peppol is positioning as *the* transport layer for ViDA-compliant exchange.
- **5-corner adoption** — Belgium 2028, more EU PAs following. Tax authorities become subscribers to a Peppol "CTC tier" of metadata.
- **Non-EU expansion** — Saudi Arabia, UAE, Oman exploring Peppol-aligned models; Brazil/Mexico unlikely to adopt (their own CTC regimes are entrenched).
- **OpenPeppol governance** — funding model shifting away from CEF/EU subsidies toward member fees, explaining the 2025-2026 fee restructure.
- **SML insourcing complete by Aug 2026** — OpenPeppol takes full operational control of the discovery layer.
- **Mandatory ISO 27001 for all APs** — being enforced by more PAs each year.

---

## 13. Sources

- [OpenPeppol — Fees page](https://peppol.org/join/fees/)
- [OpenPeppol — For Service Providers](https://peppol.org/about/for-service-providers/)
- [OpenPeppol — Certified Service Providers list](https://peppol.org/members/peppol-certified-service-providers/)
- [OpenPeppol — Country Profiles](https://peppol.org/learn-more/country-profiles/)
- [OpenPeppol — Service Provider Agreement](https://peppol.org/documentation/governance-documentation/service-provider-agreement/)
- [Peppol AS4 Profile spec](https://docs.peppol.eu/edelivery/as4/specification/)
- [OpenPeppol eDEC Specifications](https://docs.peppol.eu/edelivery/)
- [Peppol PINT Billing](https://docs.peppol.eu/poac/pint/pint/)
- [Peppol PKI 2025 — CA Migration Plan](https://openpeppol.atlassian.net/wiki/spaces/OPMA/pages/3977936899/Peppol+PKI+2025+-+Certificate+Authority+Migration+Plan)
- [Peppol PKI — Issuing and Enrolment](https://openpeppol.atlassian.net/wiki/spaces/OPMA/pages/4439080961/Peppol+PKI+2025+-+Issuing+and+Enrolment+Process)
- [Peppol SML Insourcing](https://openpeppol.atlassian.net/wiki/spaces/PTPUB/pages/5059608580/SML+Insourcing)
- [Peppol Practical — SMP/SML interplay (Helger)](https://peppol.helger.com/public/menuitem-docs-smp-sml-interplay)
- [Peppol Practical — Setup AP (Helger)](https://peppol.helger.com/public/menuitem-docs-setup-ap)
- [Peppol Practical — PKI explained](https://peppol.helger.com/public/menuitem-docs-peppol-pki)
- [phase4 (GitHub)](https://github.com/phax/phase4)
- [phase4-peppol-standalone (GitHub)](https://github.com/phax/phase4-peppol-standalone)
- [peppol-commons (GitHub)](https://github.com/phax/peppol-commons)
- [Oxalis (GitHub)](https://github.com/OxalisCommunity/oxalis)
- [Holodeck B2B](http://holodeck-b2b.org/tag/peppol/)
- [node42 — Pure Node.js Peppol AS4 sender in ~500 LOC (Mar 2026)](https://medium.com/@node42-dev/a-fully-working-peppol-as4-sender-in-node-js-in-500-lines-of-code-bad807b0e071)
- [Comparison of AS4 solutions for Peppol (xeinkauf, 2023)](https://xeinkauf.de/app/uploads/2023/09/Comparison_of_AS4_solutions_for_Peppol.pdf)
- [eDelivery AS4 1.15 (CEF)](https://ec.europa.eu/digital-building-blocks/sites/spaces/DIGITAL/pages/467117638/eDelivery+AS4+-+1.15)
- [Storecove — Peppol Access Point](https://www.storecove.com/us/en/solutions/peppol-access-point/)
- [Storecove — ISO 27001 mandatory blog](https://www.storecove.com/blog/en/peppol-standards-iso-certification/)
- [Storecove — Buy vs Make AP](https://www.storecove.com/blog/en/6-considerations-for-becoming-peppol-access-point/)
- [Tickstar — How to become a Peppol AP](https://www.tickstar.com/how-to-become-a-peppol-access-point/)
- [ATO — Australian Accreditation Process for Peppol SPs](https://softwaredevelopers.ato.gov.au/australian-accreditation-process-peppol-service-providers)
- [IMDA — Peppol SP Accreditation Scheme](https://www.imda.gov.sg/how-we-can-help/nationwide-e-invoicing-framework/peppol-service-provider-accreditation-scheme)
- [AgID — Peppol AP/SMP Qualification (IT)](https://peppol.agid.gov.it/en/qualification-ap-smp/)
- [Fonoa — Peppol Adoption in Europe 2026, ViDA & What's Next](https://www.fonoa.com/resources/blog/peppol-adoption-europe-2026-mandates-vida)
- [Qvalia — Peppol global reach 2026 country guide](https://qvalia.com/peppol-global-reach-2026-the-complete-country-guide/)
- [Arratech — EN 16931 mid-2026 update](https://www.arratech.com/blog/en-16931-update-mid-2026-what-it-means-for-your-e-invoicing-infrastructure)
- [Arratech — Peppol PKI migration](https://www.arratech.com/blog/peppol-pki-migration-what-you-need-to-know)
- [Arratech — SMP HTTPS deadline](https://www.arratech.com/blog/peppol-service-metadata-infrastructure-updates-must-do-s-before-the-deadline)
- [Combell — Is Peppol free?](https://www.combell.com/en/blog/is-peppol-free/)
- [EDICOM — Peppol 4-corner vs 5-corner CTC](https://edicomgroup.com/blog/peppol-4-corner-5-corner-ctc)
- [Pagero — Peppol and the French CTC system](https://www.pagero.com/blog/peppol-network-french-continuous-transaction-control)
- [Peppol BIS Billing 3.0 docs](https://docs.peppol.eu/poacc/billing/3.0/bis/)
- [VATupdate — BIS vs PINT](https://www.vatupdate.com/2025/08/17/what-is-the-difference-between-peppol-bis-and-peppol-pint/)
- [Logiq — Belgium 2026 mandate & CTC](https://www.logiqconnect.com/resources/insights/belgium-e-invoicing-mandate-2026-and-continuous-transaction-controls-ctc-what-businesses-need-to-know)
