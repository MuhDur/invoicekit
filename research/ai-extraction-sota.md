# AI-Driven Invoice Extraction: State of the Art (May 2026)

> Survey scope: commercial APIs, open models, small VLMs, pure OCR, hybrid pipelines, benchmarks, auditability, and per-page economics. Compiled for an open-source e-invoicing toolkit that needs a progressive intake pipeline (cheap deterministic → local small model → cloud LLM fallback).
>
> Treat numbers as vendor-claimed unless explicitly tagged as third-party. SOTA shifts every ~3 months in this space.

---

## TL;DR

- **Strong digital-PDF case is solved.** If the PDF carries embedded text or a Factur-X/ZUGFeRD XML attachment, no AI is needed. ~30–50% of inbound B2B invoices in EU markets already qualify; this share rises every quarter as e-invoicing mandates roll out.
- **Scanned/photo invoices remain hard.** Even SOTA systems plateau around 95–98% header field accuracy and 85–92% line-item accuracy on noisy real-world inputs. The remaining errors are concentrated in line items, multi-page table continuations, and ambiguous alphanumeric strings (IBANs, tax IDs).
- **Open models caught up.** As of OmniDocBench v1.7 (April 2026), the open-source frontier (GLM-OCR, PaddleOCR-VL-1.5, DeepSeek-OCR-2, dots.ocr) is within a couple of points of proprietary cloud APIs on document parsing. The expensive part is now invoice-specific post-processing, not OCR.
- **Browser-side extraction is real but constrained.** Tesseract-WASM, PaddleOCR-WASM, and SmolDocling-256M run client-side today; full reasoning over a noisy scan still needs server or cloud assist.
- **The dominant failure mode is silent line-item hallucination** — LLMs fabricate plausible quantities, unit prices, or descriptions when a row is occluded or split across pages, and confidently emit them with no warning.

---

## 1. Commercial API benchmark table

Pricing is published list rate; enterprise volume contracts routinely cut 40–70%. Accuracy figures are vendor-claimed unless marked `[3p]` (third-party benchmark).

| Vendor | Header field acc. | Line-item acc. | List price (page) | Languages | Deployment | Notes |
|---|---|---|---|---|---|---|
| **Rossum** | ~98% (claimed; field-level after learning loop) | "Industry-leading"; no public number | From $18k/yr enterprise base; usage-tiered | 276 (incl. handwriting) | Cloud SaaS primary; some private-cloud | Strong post-correction learning loop; per-invoice cost-effective at 100k+/yr |
| **Mindee** | >95% most fields (claimed) | >90% (claimed) | $0.10/page (first 25/mo free) | 60+ | Cloud-only API; limited on-prem for enterprise | Developer-friendly REST; lowest friction prototype |
| **Klippa DocHorizon** | 95–98% claimed | Strong on EU formats | Quote-based | 150+ | Cloud + on-prem option | EU-data-residency friendly; no self-serve trial |
| **Hypatos** | 90%+ claimed; "remarkable" on non-standard | Confidence-gated automation | Enterprise quote | 30+ | Cloud + private-cloud | Specialist in complex EU formats, multi-currency/multi-tax |
| **Veryfi** | 98.7% `[3p, own benchmark]` (vendor) | 99.56% on expense receipts (vendor) | ~$0.45/doc (published bench) | 110+ | Cloud + on-device SDK (iOS/Android) | Receipts-first, but invoice OCR also strong; deterministic models claimed |
| **Nanonets** | ~92% on financial docs `[3p customer case]` | Mixed — some columns dropped in dense tables `[3p test]` | First 100 free, then volume-tiered | 200+ | Cloud + on-prem | Trainable custom models, low-touch UX |
| **AWS Textract AnalyzeExpense** | 78% `[3p, businesswaretech 2025]` | 82% `[3p]` | $0.01–$0.10/page depending on tier | English-heavy; partial EU | Cloud (AWS) | Cheapest tier; weakest accuracy on variable formats |
| **Azure Document Intelligence (prebuilt invoice)** | 93% `[3p, businesswaretech 2025]` | 87% `[3p]` | ~$10/1k pages prebuilt; $30/1k custom | 100+ | Cloud + Disconnected Containers (true on-prem) | Best combination of accuracy + on-prem option for EU compliance |
| **Google Document AI — Invoice Processor** | ~92% (Gemini-powered, claimed) | Strong; Gemini Layout Parser (Nov 2026) handles complex tables | $0.10 per 10 pages prebuilt; $10–30/1k custom + $0.05/hr hosting | 200+ | Cloud (GCP) | Recently re-platformed onto Gemini 2.0 Flash; fast catch-up |
| **Sensible.so** | High; field-level rules + LLM hybrid | Strong | **Per-document** (not per-page) — flat rate regardless of length | 30+ | Cloud-only | SenseML config-as-code; flat per-doc pricing huge for multi-page packets |
| **Tabscanner** | Receipt-focused, less for invoices | n/a (receipts) | $0.05–$0.10/page | 50+ | Cloud + mobile SDK | Best for retail receipts, not full B2B invoices |
| **Affinda** | High (200+ field schema) | Per-field confidence in JSON | Quote-based | 50+ | Cloud + on-prem | Widest field coverage out of the box; deep ERP integrations |
| **Parashift** | "50% better than competitors" (vendor) | High on DACH formats | Quote-based | EU-focused | Cloud + on-prem | GDPR-first; strong CH/DE/AT invoice handling |
| **ABBYY Vantage** | 90%+ out of the box (vendor) | Strong table extraction | Enterprise quote | 200+ | Cloud + on-prem + containers | Most mature enterprise IDP; expensive |
| **IRIS Xtract / Xtract for Invoices** | Mature; legacy enterprise | Strong | Enterprise quote | EU-strong | On-prem heavy | Canon-owned; banking and PSP backbone |
| **Reducto** | High; multi-pass vision-first | Best-in-class on complex tables `[3p ParseBench]` | Enterprise quote | 30+ | Cloud | Newer entrant; financial-doc specialist |
| **Extend** | Strong on complex docs | Strong | Enterprise quote | English-heavy | Cloud | YC-backed; tight API ergonomics |
| **LlamaParse** | Good on text-PDF; weaker on scans | OK | $0.003/page (basic); $0.045/page premium | 30+ | Cloud | RAG-oriented; not invoice-specialized |
| **Mistral OCR 3** | SOTA; 88.9% on handwriting `[3p]` | Strong | ~$1/1000 pages | 30+ | Cloud (Mistral platform) | Commodity-priced SOTA; Dec 2025 release |

### Key takeaways from the commercial landscape

1. **Two-stage market**: Document-AI primitives (Textract, Doc AI, Azure DI, Mistral OCR) are commodity at $1–10 per 1000 pages. Invoice-specialist platforms (Rossum, Hypatos, Parashift, ABBYY) charge 10–50× more and bundle workflow + learning loops.
2. **Per-document vs per-page pricing matters.** Sensible's per-document model is dramatically cheaper for multi-page packets; everyone else penalizes long docs.
3. **On-prem availability is uneven.** Azure Disconnected Containers, ABBYY Vantage, Parashift, and Hypatos offer real on-prem. Mindee, Rossum, Veryfi, AWS, Google are cloud-first.
4. **Independent third-party benchmarks rank Azure DI > Google Doc AI > AWS Textract on invoices**, with Azure leading on irregular or historical formats (78% vs 87% vs 93% on a 2025 BusinesswareTech test).

---

## 2. Open-model benchmark table

OmniDocBench v1.7 (April 2026) is the most-cited cross-model document-parsing benchmark. Scores below are composite OmniDocBench unless noted.

| Model | Params | License | OmniDocBench | Invoice/KIE notes | Hardware floor |
|---|---|---|---|---|---|
| **GLM-OCR** | 0.9B | Apache-2.0 | **94.62** (top open) | Fastest (1.25 s/page); good general purpose | 8 GB GPU; CPU usable |
| **PaddleOCR-VL-1.5** | 0.9B | Apache-2.0 | 94.50 | Multilingual; strong tables via PP-StructureV3 | 4–8 GB GPU; runs CPU |
| **HunyuanOCR** | 1B | Custom community | 94.10 | Best grounded output (bounding boxes); coords accurate | 8 GB GPU |
| **FireRed-OCR** | 2B | Apache-2.0 | 92.94 | Formula/table integrity via Format-Constrained GRPO | 12 GB GPU |
| **DeepSeek-OCR-2** | 3B MoE (~500M active) | MIT | 91.09 | Best blank-page handling; strong handwriting | 12 GB GPU |
| **dots.ocr-1.5** | 3B | Apache-2.0 | ~88.4 (1.0); 1.5 expected higher | Multi-task: doc + web/SVG/scene; broader than just OCR | 16 GB GPU |
| **Qwen2.5-VL-7B** | 7B | Apache-2.0 | DocVQA 96.4; OmniDocBench EN edit-distance 0.226 | Designed for "structured outputs for invoices/forms/tables" | 16 GB GPU (fp16) or 8 GB (int4) |
| **Qwen2.5-VL-72B** | 72B | Apache-2.0 | Matches GPT-4o / Claude 3.5 on document tasks | Heaviest open option; needs A100/H100 | 80 GB GPU |
| **Granite-Docling-258M** (IBM, Sep 2025) | 258M | Apache-2.0 | n/a; tuned for Docling pipeline | Tight integration with Docling toolkit | Runs on CPU |
| **Docling (IBM toolkit)** | n/a (pipeline) | MIT | Strong general parse | Open-source champion; Linux Foundation-donated; banks deployment via OpenShift Operator | CPU-viable for layout, GPU for VLM |
| **Marker (Datalab)** | n/a (pipeline) | GPL-3 / commercial | High accuracy on PDF→Markdown | 30.9k GitHub stars; uses Surya under the hood | CPU acceptable; GPU 3–10× faster |
| **Surya OCR (Datalab)** | n/a | GPL-3 / commercial | 90+ languages; layout + reading order + tables | 19.1k stars; CPU-slow but accurate; required by Marker | GPU recommended |
| **LayoutLMv3** | 113M base / 368M large | CC-BY-NC (research) | SOTA on FUNSD/CORD/SROIE at release | Best classical KIE model; needs OCR upstream | CPU usable; GPU preferred |
| **LiLT** | ~140M | MIT | Below LayoutLMv3 at page-level; lighter | Language-agnostic; great for multilingual fine-tunes | CPU viable |
| **Donut** (Naver) | 200M | MIT | Higher recall than LayoutLMv3 on some invoices | OCR-free; end-to-end image→JSON | GPU recommended |
| **UDOP** | 794M | CC-BY-NC | Strong unified doc/text/image | Heavier; less production-deployed | GPU required |
| **BROS** | ~110M | Apache-2.0 | Strong on SROIE/CORD | Older; superseded by LayoutLMv3 for most use cases | CPU usable |
| **Florence-2** (Microsoft) | 230M–770M | MIT | Mid-tier on OCR; strong vision generalist | Good as cheap detect+caption layer in a pipeline | CPU/edge viable |
| **GOT-OCR2** | 580M | Apache-2.0 | Strong on plain text + tables + formulas + charts | "Unified end-to-end" — useful for mixed content | 8 GB GPU |
| **SmolDocling-256M** | 256M | Apache-2.0 | n/a (specialist); designed for end-to-end doc conversion | **Runs in-browser via Transformers.js + WebGPU** | CPU + WebGPU |
| **SmolVLM / SmolVLM-2** | 256M / 500M | Apache-2.0 | n/a | Browser-viable; weaker on noisy scans | CPU + WebGPU |

### State-of-the-art on classical benchmarks (Nov 2025)

ARIAL (an agentic framework wrapping LLM + vision grounding) currently holds top scores:

- **DocVQA**: 88.7 ANLS / 50.1 mAP localization
- **FUNSD**: 90.0 ANLS / 50.3 mAP
- **CORD**: 85.5 ANLS / 60.2 mAP
- **SROIE**: 93.1 ANLS

These benchmarks are largely saturated for retrieval; the bar has moved to **answer localization** (pixel-level citation), where ARIAL beats prior DLaVA by +3.9 mAP on DocVQA.

---

## 3. Small VLMs and edge / WASM feasibility matrix

The headline question: can we put serious invoice extraction in the browser, on a phone, or on an edge worker?

| Model | Params | ONNX export | Browser via Transformers.js + WebGPU | Phone (iOS/Android) | Edge worker (CF/Vercel) | Realistic invoice quality |
|---|---|---|---|---|---|---|
| Tesseract.js | n/a | Pure WASM | **Yes** (SIMD WASM) | Yes (via WebView) | Yes | Clean digital → 95–99%; phone photo → poor |
| PaddleOCR.js (PP-OCRv5 mobile) | ~10–40 MB ONNX | **Yes** | **Yes** (WASM or WebGPU, 2–5× faster) | Yes | Yes | Good on clean scans; competitive with cloud OCR for text extraction |
| Surya (ONNX subset) | ~200–500 MB | Partial | Heavy but possible on desktop browsers | Marginal | No (size) | Strong, but too heavy for typical edge |
| Florence-2-base (230M) | ~450 MB | **Yes** | **Yes** (demoed) | Marginal | No | Good for detection / cropping; weaker on full extraction |
| SmolDocling-256M | ~500 MB | **Yes** | **Yes** (HF demo runs on consumer laptop) | Marginal | No | Reasonable on clean docs; struggles on noisy scans |
| SmolVLM-256M / -500M | 500 MB–1 GB | Yes | Yes (slow) | Marginal | No | OK for simple Q&A; not for line-item tables |
| Moondream2 (~1.9B) | ~2 GB | Yes | Borderline on WebGPU desktop | No | No | OK for headers; weak on dense tables |
| Phi-3.5-vision (4.15B) | ~8 GB fp16 / 2 GB int4 | Yes | Desktop only via WebGPU; slow | No | No | Mid-tier on documents |
| MiniCPM-V 2.6 (8B) | 5.5 GB | Partial | Desktop possible; impractical for production | No | No | Strong on documents; too heavy for browser users |
| Llama 3.2-Vision 11B | ~22 GB fp16 | Partial | No | No | No | Mid-tier on documents; needs server GPU |
| Qwen2.5-VL 3B / 7B | ~6 / 14 GB fp16 | Yes | 3B borderline on WebGPU; 7B no | Server-only | No | Best small-VLM for invoices today |
| GLM-OCR (0.9B) | ~2 GB | Yes | Possible; not yet a turnkey demo | No | No | SOTA accuracy in tiny footprint |

### Browser-side reality check (May 2026)

- **Transformers.js v4** (Feb 2026) with C++-rewritten ONNX-Runtime-Web + WebGPU achieves 20–60 tokens/sec on consumer laptops for ~1B models. WebGPU is now on all major browsers (Chrome 113+, Firefox 141+, Safari 26+).
- A **realistic browser pipeline** today: PaddleOCR.js (ONNX-WASM) for text + bounding boxes → SmolDocling-256M (WebGPU) for layout → ship JSON to server for VLM reasoning only on failure.
- **What still does not work in-browser**: dense multi-page line-item tables on noisy scans; anything requiring 7B+ VLM reasoning; long-context document QA.

---

## 4. Pure OCR comparison

| Engine | Best for | Speed (clean page) | License | Install footprint | Invoice notes |
|---|---|---|---|---|---|
| Tesseract 5 | Clean printed text, Latin scripts | <1 sec/page CPU | Apache-2.0 | ~10 MB | Bad at dense tables; needs heavy preprocessing |
| PaddleOCR PP-OCRv5 | Tabular invoices, multilingual, rotated text, low-quality scans | 1–3 sec/page CPU | Apache-2.0 | 30–100 MB | Best free OCR for invoices; PP-Structure handles line-item tables |
| EasyOCR | Handwriting, mixed scripts | ~3× slower than Tesseract | Apache-2.0 | ~500 MB | Convenient but heavy; rarely the right choice for invoices |
| docTR | Latin-alphabet documents, fixed schemas | Fast on GPU | Apache-2.0 | 100–300 MB | Cleanly designed pipeline; limited multilingual |
| Surya | 90+ languages, layout + reading order + tables | Slow CPU; 2–5× faster GPU | GPL-3 / commercial | ~500 MB | Most accurate open OCR; the one Marker ships |

**Recommendation for a progressive pipeline**: PaddleOCR for the main extraction path (Apache, multilingual, table-aware, WASM-portable), Tesseract for the trivial English digital-PDF fallback, Surya when you need maximum accuracy and are willing to spend GPU.

---

## 5. Hybrid pipeline recommendation

Designed for the project's stated goal: cheap deterministic first, escalate only when needed.

### Layer 1 — Cheap deterministic (target: 30–50% of inbound, near-zero AI cost)

1. **Sniff for Factur-X / ZUGFeRD XML attachment** (PDF/A-3 associated files, `factur-x.xml` or `zugferd-invoice.xml`). If found, parse XML directly. No OCR, no AI. **This catches all e-invoicing-mandate-compliant EU invoices and will be the dominant input within 2–3 years.**
2. **Sniff for UBL / PEPPOL XML attachments** (same approach).
3. **Sniff for digital text in the PDF** via pdfplumber / pdfminer.six / pdf.js. If `>80%` of expected fields can be located via regex + spatial heuristics on the digital text layer, emit canonical invoice with `extraction_method=digital_text` and confidence=1.0.

### Layer 2 — Local small model (target: most remaining cases, sub-second/page on CPU)

1. **Layout detection** with PP-StructureV3 (PaddleOCR) or DocLayout-YOLO — find header, table, footer regions.
2. **Targeted OCR** on each region with PaddleOCR PP-OCRv5 (table cells get table-aware decoding).
3. **Field assignment**: rule-based + LayoutLMv3 fine-tuned on invoices (CC-BY-NC) **or** a small instruct-tuned VLM like SmolDocling-256M or Qwen2.5-VL-3B (Apache-2.0).
4. **Output** must include per-field bounding boxes and confidence.

### Layer 3 — Heavier local model (escalation when Layer 2 confidence < threshold)

1. **Qwen2.5-VL-7B (Apache-2.0)** or **DeepSeek-OCR-2 / GLM-OCR / PaddleOCR-VL-1.5** on a server GPU (1× L4 or A10 sufficient).
2. **Pass cropped regions, not full pages** — keeps token cost down and forces the model to attend to the right part.
3. **Constrained JSON output** via JSON schema or grammar-constrained decoding.

### Layer 4 — Cloud LLM (last-resort fallback for the ~5% the open stack misses)

1. **Gemini 2.5 Pro** or **Mistral OCR 3** for hardest cases (vendor-claimed best invoice accuracy as of Feb 2026).
2. **Batch API** for non-real-time flows — half the cost.
3. **Always** capture and store the bounding boxes / token spans returned, so the audit trail survives the API call.

### Fallback ladder cost expectations

| Layer | Cost per page | Coverage | Latency |
|---|---|---|---|
| 1 (XML attachment / digital text) | ~$0 | 30–60% (EU 2026 mix) | <100 ms |
| 2 (local PaddleOCR + small VLM) | ~$0.0001 (compute amortized) | +30–40% | 300–800 ms CPU; <100 ms GPU |
| 3 (local 7B VLM on owned GPU) | ~$0.001–$0.003 | +5–10% | 1–3 sec |
| 4 (cloud LLM) | ~$0.005–$0.02 | +1–5% (and the ones humans would also fail on) | 2–6 sec |

---

## 6. Benchmarks and where SOTA still fails

### Public benchmarks worth tracking

- **OmniDocBench v1.7** (April 2026) — broadest, most-cited; OCR + layout + tables + formulas. Top scores now 94+ (composite).
- **SROIE** — receipts; saturated (93+ ANLS).
- **FUNSD** — forms; saturated for KIE (90+ ANLS).
- **CORD** — receipts; saturated.
- **DocVQA** — document VQA; ARIAL leads at 88.7 ANLS, 50.1 mAP localization.
- **RVL-CDIP** — document classification; saturated.
- **ParseBench** (LlamaIndex, 2026) — newer benchmark across 14 parsers; useful for **document-parsing** rather than narrow KIE.
- **Document Haystack** — long-context (5–200 pp) VLM benchmark; useful for stress-testing whether a model can find the right field in a multi-page invoice packet.

### Where even the best systems still fail in 2026

1. **Multi-page line-item continuation.** "Continued from previous page" tables and per-page subtotals routinely lose rows or duplicate them.
2. **Unstructured alphanumerics.** IBANs, BIC, complex tax IDs, customer references — even top VLMs misread a digit when font/scan quality is mediocre.
3. **Mixed summary+detail layouts.** Invoices that show summary first and itemized later trip detection of which table is the "real" line items.
4. **Handwritten annotations or stamps** over the printed text.
5. **Faxed or 200 DPI scans** — Mistral OCR 3 leads at 88.9% on handwriting but everyone collapses below ~150 DPI.
6. **Non-Latin scripts in line-item descriptions** (Arabic, Chinese, Thai) embedded in otherwise Latin invoices.
7. **Decimal-separator ambiguity** in mixed-locale environments (`1,234.56` vs `1.234,56`).
8. **Tax line consolidation.** Multi-rate VAT splits, reverse-charge flags, and EU exemption codes are extracted poorly without dedicated post-processing rules.

---

## 7. Auditable AI: how to cite source pixels

This is the most underrated dimension. Real auditability requires every extracted field to carry:

1. **Page index** + **bounding box (x, y, w, h)** in document coordinates.
2. **The exact OCR/decoded substring** that produced the value.
3. **A confidence score** with documented calibration.
4. **Provenance**: which layer of the pipeline (XML attachment, digital text, OCR+rules, VLM) emitted the field.

### Vendor support for source-region citations

| Vendor / Model | Per-field bounding box | Per-field confidence | Source token span | Notes |
|---|---|---|---|---|
| Azure Document Intelligence | Yes | Yes | Yes (page+span) | Best-in-class metadata; layout maps included |
| AWS Textract AnalyzeExpense | Yes | Yes | Yes | Geometry on every block |
| Google Document AI | Yes (anchor → page tokens) | Yes | Yes | Anchors map directly to token spans |
| Rossum | Yes | Yes | Yes | Source-region tracked through learning loop |
| Mindee | Yes | Yes | Partial | Less granular than Azure |
| Affinda | Yes (most fields) | Yes (per-field) | Partial | 200+ schema fields with confidence |
| Reducto | Yes | Yes | Yes | Vision-first multi-pass preserves layout |
| HunyuanOCR | Yes (1,517 visual anchors in workflow test) | Implicit | Yes | Best **open** grounded output |
| ARIAL (research) | Yes (locked to pixel coords) | Yes | Yes | SOTA on **answer localization** metric (mAP) |
| Raw cloud LLM vision (GPT-4o / Claude / Gemini) | **No** (must reconstruct) | Self-reported (unreliable) | No reliable mapping | Will hallucinate confidence; **never trust unaided** |

### Pattern recommendations

1. **Never accept LLM self-reported confidence as a triage signal.** Multiple 2025–2026 studies confirm LLMs overestimate certainty and produce non-uniform numerical confidence distributions.
2. **Force grounding.** Either (a) restrict the LLM to selecting from OCR-detected text spans (so every output is a span ID with a known bbox), or (b) post-validate every emitted value by searching for it on the page and recording its bbox.
3. **Build explicit uncertainty signals into the schema** (allow `"unknown"` / `null` with a reason field) rather than asking the model to score itself.
4. **Two-source agreement**: if Layer 2 OCR+rules and Layer 3 VLM disagree, route to human review. This is the single highest-leverage QA mechanism.
5. **Render the bbox back onto the PDF** in the user UI. Auditors and AP clerks accept AI output an order of magnitude more readily when they can click a field and see the highlighted region.

---

## 8. Per-page cost model at different volumes

All figures are list prices; assume 25–60% enterprise discounts at >100k pages/month.

### Cost per page by extraction method

| Method | Per-page cost | Throughput assumption |
|---|---|---|
| **Digital text + regex** (pdfplumber) | $0 | 50–100 pages/sec on 1 CPU core |
| **Factur-X XML parse** | $0 | ~1000 invoices/sec |
| **Local Tesseract CPU** | ~$0.00005 (electricity-only on owned hw) | 1–3 pages/sec |
| **Local PaddleOCR CPU** | ~$0.0001 | 0.3–1 page/sec |
| **Local PaddleOCR GPU** (1× L4 amortized) | ~$0.0002 | 10–30 pages/sec |
| **Local 7B VLM on owned GPU** (1× L4/A10 amortized) | ~$0.001–$0.003 | 1–3 pages/sec |
| **AWS Textract AnalyzeExpense** | ~$0.01–$0.10 (tiered) | API-bound |
| **Azure Doc Intelligence (prebuilt invoice)** | ~$0.01 | API-bound |
| **Google Doc AI invoice processor** | ~$0.01 | API-bound |
| **Mindee** | $0.10 | API-bound |
| **Rossum** | $0.05–$0.30 (volume-tiered) | API-bound |
| **Mistral OCR 3** | ~$0.001 | API-bound |
| **GPT-4o-mini vision** | ~$0.001–$0.003 per page (depends on resolution) | API-bound |
| **Gemini 2.5 Flash vision** | ~$0.001–$0.003 per page | API-bound |
| **Gemini 2.5 Flash-Lite** | ~$0.0007 per page | API-bound |
| **Claude 4.x Haiku vision** | ~$0.002–$0.005 per page | API-bound |
| **GPT-4o / Claude Opus / Gemini 2.5 Pro vision** | ~$0.02–$0.10 per page | API-bound |

### Break-even thresholds

- **Cloud LLM vs owned GPU**: A single L4 GPU on a $500/month VPS amortized over 30 days × 86,400 sec × 2 pages/sec at 30% utilization = ~1.5M pages/month. At Gemini Flash $0.002/page = $3000/month. So **owned GPU beats cloud Flash above ~250k pages/month** for the Layer 3 use case. For Layer 4 SOTA models ($0.02–$0.10/page), owned GPU dominates from ~25k pages/month.
- **Cloud OCR (Azure DI) vs owned PaddleOCR**: At $0.01 vs $0.0002, the gap is 50×. Even at 5k pages/month the owned-stack saves $480/year, but you absorb engineering overhead. Real break-even is around **100k pages/month** when ops cost is included.
- **Invoice-specialist platforms (Rossum/Hypatos)** typically only economic above 100k+ pages/year, AND when their built-in workflow/approval/learning loop replaces internal tooling. As pure extraction engines, they are not cost-competitive.

### Mixed-routing economics (realistic)

For a 100k pages/month workload, an optimized progressive pipeline costs roughly:

| Layer | % traffic | Cost/page | Sub-total |
|---|---|---|---|
| 1 (Factur-X / digital text) | 45% | $0 | $0 |
| 2 (local PaddleOCR + small VLM) | 40% | $0.0002 | $8/mo |
| 3 (local 7B VLM) | 12% | $0.002 | $24/mo |
| 4 (cloud LLM SOTA) | 3% | $0.05 | $150/mo |
| **Total** | 100% | **~$0.0018 blended** | **~$182/mo + infra** |

Same volume on a single cloud API like Mindee: ~$10,000/month. On Rossum: ~$15,000–30,000/month.

---

## 9. Open-source ecosystem notes

- **Docling (IBM)** is the clearest open-source champion right now: Linux Foundation-donated (Agentic AI Foundation), 58.6k stars, Apache-2.0, OpenShift Operator for banks, native LangChain/LlamaIndex/Haystack integrations, ships Granite-Docling-258M VLM (Apache-2.0). For an open-source invoicing project, building on Docling = "buy" the parsing problem cheaply.
- **Marker + Surya (Datalab)** are the highest-quality general PDF→Markdown pipeline (30.9k + 19.1k stars). License is GPL-3 with commercial option — viable for open-source apps but check the dual-license terms before bundling in a non-GPL product.
- **PaddleOCR + PP-Structure** is the strongest fully permissive (Apache-2.0) end-to-end stack including layout + table extraction, with mature WASM/ONNX ports for browser use.
- **Hugging Face Transformers.js v4** + WebGPU is now production-grade for ~1B-parameter VLMs in the browser. The bar for a "no-server" client-side invoice tool has dropped dramatically.
- **invoice2data** (PyPI) is the classic template-based open-source invoice parser. Still useful as a Layer 1 fallback for known-vendor templates.

---

## 10. Recommended canonical-invoice extraction stack (for this project)

```
PDF / image input
  │
  ▼
[L1] Detect Factur-X/ZUGFeRD/UBL → parse XML directly, done
  │ (no match)
  ▼
[L1] Extract digital text (pdf.js / pdfplumber)
  │   → if structured text + matching template → done
  │ (otherwise)
  ▼
[L2] PaddleOCR PP-StructureV3 (layout + OCR + tables)
  │   → rule-based field extraction with bbox
  │   → emit invoice with per-field confidence + bbox
  │ (low-confidence fields only)
  ▼
[L3] Qwen2.5-VL-3B or SmolDocling-256M on cropped regions
  │   → JSON-schema-constrained output
  │   → ground every value back to OCR span (no free generation)
  │ (still low confidence)
  ▼
[L4] Gemini 2.5 Pro or Mistral OCR 3 batch API
  │   → only on hard residual; cache aggressively
  ▼
Canonical Invoice + Audit Trail (page, bbox, source span, layer, confidence)
```

License-clean stack (all Apache-2.0 / MIT, suitable for any open-source license): PaddleOCR, Qwen2.5-VL, SmolDocling, Granite-Docling, Docling toolkit. Avoid LayoutLMv3 (CC-BY-NC), UDOP (CC-BY-NC), and Marker/Surya if your project license is incompatible with GPL-3.

---

## Sources

Top-cited public sources used in this survey (full URLs in chat history; here grouped for the writer):

- Veryfi Aug 2025 benchmark (Veryfi vs Google Cloud Vision vs Mindee, 500 invoices)
- BusinesswareTech AI invoice processing benchmark 2025 (AWS Textract vs Azure DI vs Google Doc AI vs GPT-4o)
- ArXiv 2509.04469 "Multi-Modal Vision vs Text-Based Parsing: Benchmarking LLM Strategies for Invoice Processing" (GPT-5 / Gemini 2.5 / Gemma 3 on receipts + invoices)
- ArXiv 2511.18192 ARIAL — current SOTA on DocVQA/FUNSD/CORD/SROIE answer-localization
- ArXiv 2502.13923 Qwen2.5-VL Technical Report
- ArXiv 2503.11576 SmolDocling
- ArXiv 2501.17887 Docling
- OmniDocBench v1.7 (opendatalab/OmniDocBench, April 2026)
- instavar.com OCR Model Leaderboard Feb 2026 (GLM-OCR, PaddleOCR-VL-1.5, HunyuanOCR, FireRed-OCR, DeepSeek-OCR-2 rankings)
- HuggingFace Transformers.js v4 release notes (Feb 2026)
- Vendor docs: Rossum, Mindee, Klippa, Hypatos, Veryfi, Nanonets, AWS Textract, Azure Document Intelligence, Google Document AI, Sensible.so, Affinda, Parashift, ABBYY Vantage, Reducto, Extend, LlamaParse, Mistral OCR
- pdflib, textcontrol, formx.ai documentation on Factur-X/ZUGFeRD detection
