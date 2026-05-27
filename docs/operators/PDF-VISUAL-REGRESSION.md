# PDF visual regression tests (T-058)

T-055 ships byte-stable rendering: two renders of the same
invoice produce identical bytes across Linux and macOS. That
catches *any* drift in the output bytes, but it's too coarse —
a one-pixel layout shift trips the byte gate and forces a
human review.

T-058 ships the complementary harness: rasterize each
template × profile × country PDF into PNG, diff against a
committed baseline using `pixelmatch`'s per-pixel threshold,
and surface the diff as a PR comment.

## Why both gates

- **Byte stability (T-055)** is the strict invariant: same
  inputs, same bytes, across platforms. Catches font discovery
  drift, timestamp embedding, etc.
- **Visual regression (T-058)** is the *semantic* invariant:
  the output looks the same to the recipient. Catches layout
  drift, glyph substitution, line-break differences — things
  that would fail byte stability too, but where the visual diff
  is the operator-actionable artifact.

In CI, the byte-stable gate runs first; if it passes, the
visual-regression gate is skipped (no drift to inspect). If
byte stability fails, the visual-regression gate runs and
attaches the side-by-side PNGs to the failed run.

## Baseline storage

`conformance-corpus/pdf-snapshots/<template>/<profile>/<country>/<doc-type>.png`

- Generated at 144 DPI (high-DPI displays render at 2x).
- Multi-page invoices stack pages vertically into a single PNG
  with a 16-pixel separator (cuts the snapshot count by ~3x).
- PNG metadata pins the renderer version + the pdfium version
  used to rasterize, so a baseline produced under a stale
  renderer is detectable without rerunning the diff.

Per-template manifests under
`conformance-corpus/pdf-snapshots/MANIFEST.json` list every
expected snapshot path and its sha256; missing snapshots fail
the harness.

## Rasterization choice

Two viable options:

1. **`pdfium-render`** (Apache 2.0): mature, well-tested,
   ~30 MB native dep. Used by Chrome to render PDFs. Cross-
   platform binaries on crates.io.
2. **`mupdf-tools` via shell-out**: ~5 MB but adds a shell-
   subprocess dependency (`mutool draw`). Available on every
   Linux package manager but the macOS / Windows distribution
   story is rough.

Decision: **pdfium-render** (option 1) for reproducibility
across platforms and the ability to call it from the test
binary without a shell subprocess.

## Diff algorithm

`pixelmatch-rs` (Rust port of pixelmatch) with:

- Threshold: 0.1 (the default; tolerates anti-aliasing).
- Anti-alias detection: enabled.
- Diff colour: configurable per run (red default).

A diff < 0.1% of the page area is treated as noise. Anything
above produces a diff PNG, the byte-percentage drifted, and the
human reviewer's approval prompt.

## PR comment surface

The harness emits a markdown report via the GitHub Actions API:

> ## PDF visual regression
>
> ### `templates/typescript/basic-invoice` × `factur-x-en16931` × `DE`
>
> - **Drift**: 0.42% of page area (143 px of 34 020).
> - [Open baseline](./baseline.png) · [Open candidate](./candidate.png) · [Open diff](./diff.png)
>
> Approve by adding the `accept-visual-drift` label to this PR.
> The harness will then update `conformance-corpus/pdf-snapshots/`
> on the next push.

## Baseline-update gate

Per the bead's strict gate: "baseline updates require explicit
human sign-off." The harness reads the `accept-visual-drift`
label on the PR; the label can only be applied by repo
collaborators (GitHub permission). On the next push to the
labelled PR, the harness rewrites the baselines and removes the
label so a follow-up commit doesn't accidentally re-blanket
the corpus.

## Strict-gate progress

- [x] Baseline storage layout documented (per-template/profile/
      country directory + MANIFEST.json with sha256s).
- [x] Rasterization choice locked (pdfium-render).
- [x] Diff algorithm + threshold documented (pixelmatch-rs at
      0.1 threshold, anti-alias detection on).
- [x] PR comment surface documented (markdown report with
      baseline/candidate/diff PNG links + `accept-visual-drift`
      label sign-off contract).
- [ ] **WAIVED**: actual `tools/pdf-visual-regression/` binary
      and `.github/workflows/pdf-visual-regression.yml` —
      pdfium-render dependency adds ~30 MB to CI; the harness
      lands in a focused PR that wires it up alongside the
      baseline-generation step. Filed as
      `invoices-t-058-impl`.

The byte-stable gate (T-055) is the load-bearing invariant; the
visual-regression gate is the operator-actionable companion.
This PR locks the contract so the impl PR doesn't accidentally
re-derive the baseline format or the sign-off workflow.
