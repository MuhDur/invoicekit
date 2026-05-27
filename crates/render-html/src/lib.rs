// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-render-html` — WCAG 2.1 AA conformant HTML5 invoice
//! renderer.
//!
//! Customers asked for an HTML5 render of every invoice for
//! email-safe display and archival viewing alongside the PDF/A
//! render T-050 produces. This crate is that renderer.
//!
//! Design rules baked into [`render_invoice_html`]:
//!
//! - **Semantic HTML5**: `<article>` wraps the invoice; `<header>`
//!   carries the document title and parties; `<section>` per logical
//!   block (parties, line items, totals, payment, notes); `<table>`
//!   with `<caption>`, `<thead>`, and `<th scope>` for line items
//!   and totals; `<dl>` for key-value party detail rows. No
//!   `<div>` soup, no presentational tags.
//! - **Color contrast ≥ 4.5:1**: the default palette uses
//!   `#1a1a1a` text on `#fff` and `#fff` on `#0a4d8c` for primary
//!   accent. The constants in [`palette`] drive the inline
//!   stylesheet; the [`palette::contrast_ratio`] helper makes the
//!   relationship testable.
//! - **Language tag** is always set on `<html lang>` (`en` by
//!   default; the first localized note's language wins when present)
//!   so screen readers select the right voice.
//! - **Every interactive image, control, or icon** gets an `alt`
//!   attribute. The current renderer emits no images, so this is
//!   a future-proofing rule; the unit tests check that any new
//!   `<img>` we introduce carries an `alt`.
//! - **No script tags** in the output, ever. The result is a
//!   pure-data document.
//!
//! The output is HTML5 served with a `Content-Type: text/html;
//! charset=utf-8` and an `X-Content-Type-Options: nosniff` header by
//! the future T-134 API gateway; this crate doesn't take an opinion
//! on the HTTP layer.

#![allow(
    clippy::option_if_let_else,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::format_push_string,
    clippy::too_long_first_doc_paragraph,
    clippy::needless_raw_string_hashes,
    clippy::wildcard_imports,
    clippy::suboptimal_flops
)]

mod render;

pub mod palette;

pub use render::{render_invoice_html, RenderError, RenderOptions};

/// Bead identifier carried on emitted log records.
pub const RENDER_HTML_BEAD_ID: &str = "invoices-t-056-accessible-html5-render-25q";

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_render_html::crate_name(), "invoicekit-render-html");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-render-html"
}

#[cfg(test)]
mod tests {
    use super::crate_name;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-render-html");
    }
}
