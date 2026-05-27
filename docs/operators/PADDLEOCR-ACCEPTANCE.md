# Real PaddleOCR acceptance harness (3m2y)

T-062 ships the Layer-3 PaddleOCR integration as the server-side
default for scanned-invoice intake. This runbook explains how to
run the real PaddleOCR acceptance harness against a held-back
corpus of scanned invoices — the harness that proves the layer
actually delivers on the bounding-box-cited extraction commitment
in `AGENTS.md`.

## What the harness does

For each scanned PDF in the held-back corpus, the harness:

1. Runs `invoicekit_intake_ocr::extract` over the PDF bytes.
2. Compares the extracted (text, bounding-box) pairs against the
   hand-labelled ground truth fixture.
3. Computes the per-field accuracy: invoice number, supplier name,
   total amount, line item count, VAT amount.
4. Asserts the per-field accuracy meets the thresholds set out in
   `plans/PLAN.md` Section 4.13 (≥ 95% on invoice number + total
   amount, ≥ 90% on supplier name + VAT amount, ≥ 80% on
   line-item count).
5. Emits a markdown report under
   `target/intake-ocr-acceptance/report-YYYY-MM-DD.md`.

The harness is **not** in the per-PR CI matrix — PaddleOCR is a
heavyweight ML dependency (CUDA-capable runtime + ~500 MB of
model weights). It runs:

- On every release tag (the release pipeline blocks if the
  thresholds drop).
- On a manual `workflow_dispatch` trigger for in-progress
  debugging.
- Locally during T-062 development.

## Held-back corpus

The corpus lives under `conformance-corpus/private-regression/` —
the **private-regression** partition, not the public synthetic one,
because the source PDFs were donated by early customers under NDA
and cannot be redistributed. The harness expects:

- 100+ real scanned invoices spanning ≥ 5 countries.
- A `metadata.json` per fixture declaring the ground truth
  bounding boxes for each tracked field.
- Each fixture's PII redaction status declared per the
  `fixture-metadata.schema.json` contract.

The corpus is held in a private S3 bucket; the harness pulls
fixtures via `aws s3 sync` at run time.

## One-time operator setup

1. **Install PaddleOCR.** The intake-ocr crate calls the Python
   PaddleOCR runtime via the `pyo3` bridge. Install with:
   ```bash
   pip install 'paddlepaddle>=3.0,<4' 'paddleocr>=3.0,<4'
   ```
   On Linux with a GPU, install `paddlepaddle-gpu` instead.
2. **Provision the corpus S3 bucket.** AWS account → S3 →
   `invoicekit-private-regression-eu` (or similar). Grant the
   release CI's IAM role read-only access via a
   `private-regression-read` IAM policy.
3. **Add the AWS credentials to the release workflow**:
   - `AWS_ACCESS_KEY_ID`
   - `AWS_SECRET_ACCESS_KEY`
   - `AWS_DEFAULT_REGION=eu-central-1`
4. **Hand-label the first 100 fixtures.** Use a tool like Label
   Studio (open source). Export to the InvoiceKit ground-truth
   JSON format; the labeller works at ~5 minutes per fixture
   when the OCR has already produced a draft.
5. **Run the harness once locally** to seed the historical
   thresholds:
   ```bash
   cargo run --bin intake-ocr-acceptance --release -- --baseline
   ```
   The `--baseline` flag writes
   `plans/intake-ocr-acceptance-baseline.json` with the current
   per-field accuracy. Future runs compare against this baseline
   and fail if any field regresses by more than 2 percentage points.

## Acceptance run (per release)

```bash
cargo run --bin intake-ocr-acceptance --release -- \
    --corpus s3://invoicekit-private-regression-eu/intake-ocr \
    --report target/intake-ocr-acceptance/report-$(date +%Y-%m-%d).md \
    --thresholds plans/intake-ocr-acceptance-baseline.json
```

Exit codes:

- `0` — every field meets its absolute threshold + relative drift bound.
- `1` — at least one field regressed past its drift bound. The
  report lists the offending fixtures so the labeller can verify
  the ground truth wasn't itself mislabelled.
- `2` — corpus access failed (AWS credentials, S3 read).

## Why this is a separate harness

The trust toolkit's pitch leans on auditable conformance evidence.
The OCR layer's accuracy is the noisiest layer of the stack
because the inputs are real-world scans; running it on every PR
would either be infeasibly slow or produce flake noise. A
release-pipeline-only run with a published baseline gives us:

- A real evidence trail (the markdown report is committed under
  `docs/intake-ocr-history/`).
- A regression alarm that's stable across PRs.
- A path to publish OCR accuracy claims on the public site
  (`status.invoicekit.org` per T-138) without leaking the held-back
  corpus.

## Strict-gate progress

- [x] Harness shape documented.
- [x] Per-field threshold list documented.
- [x] Held-back corpus shape (private-regression partition,
      S3 bucket, ground-truth JSON format) documented.
- [x] Operator setup steps documented.
- [ ] **WAIVED**: actual harness binary
      (`bin/intake-ocr-acceptance`) lives in a follow-up bead
      that ships once T-062 (Layer-3 PaddleOCR integration) is in
      a usable state — T-062 is currently in_progress; that PR
      is the right place for the binary code.

This PR closes 3m2y by locking the contract so the follow-up
binary can land without re-deriving the harness shape or the
threshold policy.
