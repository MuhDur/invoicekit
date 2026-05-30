<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-lsp

Language Server Protocol (LSP) type substrate for InvoiceKit: the request/response/diagnostic shapes plus two pure functions that an IDE extension wires to, ahead of a live server crate.

## What it does

This crate is a typed surface, not a running language server. It defines the LSP-shaped data types InvoiceKit's future server would emit (a subset of `lsp-types`), and provides two pure functions that operate over an open document: a JSON parse pass that yields diagnostics, and a hover lookup that returns a plain-English gloss for an invoice field.

There is no transport, no JSON-RPC, no `tower-lsp` glue, and no `initialize`/`shutdown`/`exit` lifecycle here. The module doc-comment positions that live server as a follow-up crate (referenced as `invoicekit-lsp-server`); this crate is the stable typed core it would delegate to, and doubles as a test harness because it is transport-agnostic.

## Capabilities

Types:

- `Position` — LSP 0-indexed `(line, character)`.
- `Range` — inclusive-start / exclusive-end pair of `Position`.
- `DiagnosticSeverity` — `Error` / `Warning` / `Information` / `Hint`, with `code()` returning the LSP numeric encoding (1–4).
- `Diagnostic` — `range`, `severity`, a stable string `code`, and a human-readable `message`.
- `HoverInfo` — the `term`, `description`, and `range` an IDE renders on hover.
- `InvoiceDocument` — an open document: original `text`, a `parses` bool, and collected `diagnostics`.
- `Capabilities` — advertised server capabilities. `Default` sets `hover_provider: true`, `diagnostic_provider: true`, `completion_provider: false`.
- `LspError` — one variant, `PositionOutOfRange(Position)`.

All types derive `Serialize`/`Deserialize`.

Functions:

- `parse_document(text) -> InvoiceDocument` — parses `text` as JSON via `serde_json`. On success: `parses: true`, empty diagnostics. On failure: `parses: false` and a single `Error` diagnostic with code `invoicekit/json-parse`, pinned to the failing line/column reported by `serde_json` (1-indexed input clamped to 0-indexed LSP positions).
- `hover_at(document, position) -> Result<Option<HoverInfo>, LspError>` — returns a gloss for the field key under the cursor, `None` if there is no key or the key is unknown, and `LspError::PositionOutOfRange` if `position.line` is past the last line of the document.
- `crate_name() -> &'static str` — returns `"invoicekit-lsp"`.

## Mode / Residuals

What this crate does NOT do, plainly:

- **Diagnostics are JSON-syntax only.** `parse_document` runs `serde_json::from_str` and nothing else. It does not validate against EN 16931, a rulepack, or any business rule. A document that is valid JSON but a structurally invalid invoice parses with zero diagnostics. The `Diagnostic.code` field is typed to hold rule identifiers (the doc-comment gives `EN16931 BR-CO-15` as an example), but no code in this crate ever emits such a code — the only code produced is `invoicekit/json-parse`.
- **Hover is a hardcoded 9-entry glossary over a naive key scan.** `hover_at` slices the cursor's line into characters, walks left/right over `[A-Za-z0-9_-]` to find the word under the column, and matches that word against a fixed `match` table of nine field names (`id`/`invoice_number`, `issue_date`, `due_date`, `currency`, `buyer`, `seller`, `tax_total`, `line_extension_amount`, `payable_amount`) mapped to short EN 16931 BT/BG glosses. It does not parse the JSON, walk an abstract syntax tree, or consult a rulepack. The crate's own doc-comment notes the "real implementation walks the parsed AST against the rulepack" — that is unbuilt; only the substrate contract exists. Any field name outside the nine returns `None`.
- **No server.** No `tower-lsp` dependency, no stdio loop, no JSON-RPC, no capability negotiation beyond the `Capabilities` struct's default values.

Dependencies are `serde`, `serde_json`, and `thiserror`.

## References

- Language Server Protocol — diagnostic severity numeric values (LSP §6), as encoded by `DiagnosticSeverity::code()`.
- EN 16931 — Business Term (BT) / Business Group (BG) identifiers used as gloss labels in the hover table (e.g. BT-1 invoice number, BT-5 currency code, BG-4 seller, BG-7 buyer).
- ISO 4217 — referenced in the `currency` gloss text.

No URLs appear in the source.

## License

Apache-2.0.
