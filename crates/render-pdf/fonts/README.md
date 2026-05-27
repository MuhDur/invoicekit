# Pinned fonts for byte-stable rendering (T-055)

This directory holds the only fonts the InvoiceKit PDF renderer
is allowed to consult. Typst's font loader is constrained to read
only from here; system fonts (`/usr/share/fonts`, `~/Library/Fonts`,
`C:\Windows\Fonts`) are intentionally not searched.

Reasons:

1. **Byte-stable output across platforms.** A render on a fresh
   Linux x86_64 runner must produce the same bytes as a render on
   a fresh macOS aarch64 runner. System-font discovery is the
   #1 source of cross-platform drift in any embedded-PDF
   pipeline.
2. **Auditable provenance.** Every font in a rendered PDF is one
   the trust toolkit can name and license. No surprises if a
   downstream user re-renders the same invoice years later.
3. **Sub-second cold-start.** The font catalogue is fixed at
   compile time via `include_bytes!`, so the first render in a
   fresh process does not need to walk `/usr/share/fonts`.

## Planned font set

| Family | Variant | License | Use |
| --- | --- | --- | --- |
| **Inter** | Regular + Italic + Bold + Bold-Italic, subsetted to Latin-1 + Latin-Ext-A + currency symbols | SIL OFL 1.1 | Default body + heading face. |
| **DejaVu Sans Mono** | Regular only, subsetted | DejaVu license (permissive, GPL-compatible) | Reference / IBAN / code-style spans. |
| **Noto Sans CJK** | Regular subset (~3 000 most-frequent CJK glyphs) | SIL OFL 1.1 | CJK invoice content. |

Each font ships as the smallest subset that satisfies the
InvoiceKit conformance corpus. Sub-setting is done with
`pyftsubset` (FontTools); the recipe is in `subset.sh` (filed
under a follow-up bead that installs FontTools in CI; the actual
font bytes ship in a follow-up because Inter alone is ~150 KB
and the licensing review for each font is a separate step).

## What ships today

This PR ships the **contract** (the README you are reading,
plus the test that asserts no system fonts are ever loaded, plus
the cross-platform byte-stable CI job). The actual font files
land in `crates/render-pdf/fonts/inter/`, `dejavu/`, and `noto/`
in a follow-up bead.

Until that bead lands, the renderer relies on the fonts that
`typst-kit`'s `embed-fonts` feature embeds (Libertinus Serif,
New Computer Modern, DejaVu Sans Mono, IBM Plex Sans). Those
covered our Year-1 European fixtures. The Inter/DejaVu/Noto
pinning is the upgrade path; the contract here is what the
follow-up must satisfy.

## How the loader is locked down

The renderer constructs `typst_kit::fonts::Fonts` with
`include_system_fonts: false` and `include_embedded_fonts: true`.
A guard test in `crates/render-pdf/src/lib.rs` asserts that the
loaded font book contains *only* the embedded faces — a future
change that flips the system-font discovery flag back on will
trip the test.

## Operator workflow for adding a new font

1. Identify the license — must be SIL OFL 1.1 or equivalent
   permissive license that allows redistribution under Apache
   2.0 alongside our code.
2. Run `pyftsubset` to produce the minimum subset satisfying the
   conformance corpus.
3. Commit the .ttf under `crates/render-pdf/fonts/<family>/`.
4. Update this README's table.
5. Update the `pinned_fonts!()` macro in `src/lib.rs` (a future
   bead introduces the macro).
6. Re-run the cross-platform byte-stable CI job to confirm the
   output hash is updated everywhere.
