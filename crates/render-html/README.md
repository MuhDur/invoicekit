<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-render-html

Renders an InvoiceKit commercial document to a single, self-contained, accessibility-oriented HTML5 string for email-safe display and archival viewing alongside the PDF/A render.

## What it does

`render_invoice_html(doc, options)` takes a validated `invoicekit_ir::CommercialDocument` and returns one `String` of HTML5. The output is fully self-contained: the stylesheet is inlined in a `<style>` block and there are no external resources, so the result can be dropped into an email body or stored as a standalone file. The renderer first calls `doc.validate()` and refuses to emit HTML for a document that fails its own IR validation.

The HTML is built by hand-rolled string templating (no template engine). Document layout:

```
<main>
  <article aria-label="Invoice F-2026-001">
    <header>            document title, issue/due/tax-point dates
    <section parties>   supplier, customer, optional payee (<dl> rows)
    <section lines>     <table> with <caption>, <thead>, <th scope>
    <section totals>    <dl> of monetary totals + per-category tax + amount due
    <section payment>   payment terms and instructions (omitted if empty)
    <section notes>     localized notes (omitted if empty)
```

## Capabilities

- **Semantic HTML5 structure.** `<main>` landmark, `<article>` with `aria-label`, one `<section>` per logical block keyed by `aria-labelledby`/`<h2 id>`, `<table>` with `<caption>`/`<thead>`/`<th scope="col">`/`<th scope="row">` for line items, and `<dl>` for party and totals key-value rows.
- **Language tag.** `<html lang>` is always set. It is taken from `RenderOptions.language` (BCP 47), falling back to the first localized note's language, then to `en`. Each note paragraph also carries its own `lang` attribute.
- **HTML escaping.** Text content escapes `& < >`; attribute values additionally escape `"` and `'`. Both escapers iterate by `char`, so multibyte UTF-8 (including right-to-left and CJK text) passes through unchanged into the output.
- **No script tags.** The renderer emits no `<script>`; a unit test asserts this. The output is a pure-data document.
- **Caller options.** `RenderOptions { language, title }` — override the language tag and the auto-generated document title ("Invoice <number>", "Credit note <number>", etc., derived from `DocumentType`).
- **Email/contact links.** A supplier/customer email renders as a `mailto:` anchor.
- **Color-contrast palette and helper.** The `palette` module holds the default template colors as constants and exposes `contrast_ratio(a, b)`, a correct implementation of the WCAG 2.1 relative-luminance contrast formula over `#RRGGBB` sRGB inputs.

## Mode / Residuals

- **Accessibility is structural, not certified.** What the code actually provides is a set of accessibility-oriented construction rules (semantic landmarks, table header scoping, language tags, a contrast-targeted default palette, script-free output) plus unit tests for a subset of them. It does **not** run a WCAG conformance checker, and there is no audit against the full set of WCAG 2.1 AA success criteria. Treat the output as accessibility-minded HTML, not as certified-conformant HTML.
- **Default palette only.** `contrast_ratio` is available to verify a candidate palette before swapping the constants, but the renderer always uses the built-in palette constants; there is no API to inject a custom palette. The default `FG_TEXT`/`BG_PAGE`, `FG_MUTED`/`BG_PAGE`, and `ACCENT_FG`/`ACCENT` pairs are unit-tested to clear the 4.5:1 normal-text threshold.
- **No images.** The renderer emits no `<img>`, logos, or icons; the "every image carries `alt`" rule in the doc-comment is forward-looking, not exercised by current output.
- **HTTP headers are out of scope.** The doc-comment mentions `Content-Type` and `X-Content-Type-Options: nosniff`; this crate produces only the HTML string and takes no opinion on the HTTP layer that serves it.
- **Payment instruction kind** is rendered via Rust `Debug` formatting of the enum (e.g. `IbanBic`), not a localized label.

## Public API

- `render_invoice_html(doc: &CommercialDocument, options: &RenderOptions) -> Result<String, RenderError>`
- `RenderOptions { language: Option<String>, title: Option<String> }` (derives `Default`)
- `RenderError` — `InvalidInvoice(IrError)`, returned when `doc.validate()` fails
- `palette` module — color constants (`FG_TEXT`, `BG_PAGE`, `FG_MUTED`, `ACCENT`, `ACCENT_FG`, `BORDER`) and `contrast_ratio(a: &str, b: &str) -> Result<f64, &'static str>`
- `crate_name() -> &'static str` — returns `"invoicekit-render-html"`
- `RENDER_HTML_BEAD_ID` — bead identifier carried on log records

## Where it sits

In the pipeline `engine -> ir -> canonical -> format/profile -> validate -> render/intake -> transmit -> evidence`, this crate is a render leaf. It depends only on `invoicekit-ir` and `rust_decimal`; nothing reaches back up the stack.

## References

- WCAG 2.1 Success Criterion 1.4.3 Contrast (Minimum) — https://www.w3.org/TR/WCAG21/#contrast-minimum (the threshold and luminance formula `palette::contrast_ratio` implements)

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
