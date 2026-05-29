// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-intake-pdf` — Layer-1/Layer-2 PDF intake.
//!
//! T-061 owns the Layer-2 path: deterministic text extraction from
//! "digital" PDFs (PDFs that carry an embedded text layer; scanned
//! PDFs are routed to `invoicekit-intake-ocr` instead). The public
//! API is:
//!
//! ```rust,ignore
//! use invoicekit_intake_pdf::{extract_pdf_text, StructuredText};
//! let bytes: Vec<u8> = std::fs::read("invoice.pdf").unwrap();
//! let st: StructuredText = extract_pdf_text(&bytes).unwrap();
//! for page in &st.pages {
//!     for frag in &page.fragments {
//!         println!("p{} ({:.1},{:.1}) {}", page.index, frag.x, frag.y, frag.text);
//!     }
//! }
//! ```
//!
//! Guarantees:
//!
//! 1. **Reading order preserved, script-aware.** Fragments inside a
//!    page are grouped into visual lines top-to-bottom (PDF Y-axis is
//!    bottom-up, so larger `y` is higher) and ordered within each
//!    line by writing system. Left-to-right Latin lines sort
//!    left-to-right. Right-to-left lines (Arabic, Hebrew) are
//!    detected with the Unicode Bidirectional Algorithm and rebuilt
//!    into logical (reading) order. CJK vertical pages are detected
//!    and read column-by-column right-to-left, each column
//!    top-to-bottom. Pages are emitted in PDF page order. The
//!    reconstruction assumes each show-text run stores its glyphs in
//!    logical order (the universal producer behaviour); a run that
//!    bakes in visual-order glyphs still routes through the OCR /
//!    vision fallback, which can re-align using the
//!    `(x, y, font_size)` triplet. See the `script_order` module for
//!    the precise bounds.
//! 2. **Encrypted PDFs are rejected** with [`PdfTextError::Encrypted`].
//!    A future bead can supply credentials.
//! 3. **Position information is in PDF user-space units** (1 unit =
//!    1/72 inch), origin at the lower-left of the page. The OCR
//!    aligner consumes the same coordinate system.

mod factur_x;
mod script_order;
mod text;

use thiserror::Error;

pub use factur_x::{extract_factur_x_xml, FACTUR_X_ATTACHMENT_NAMES};
pub use text::{extract_pdf_text, PageText, StructuredText, TextFragment};

/// Errors returned by [`extract_pdf_text`].
#[derive(Debug, Error)]
pub enum PdfTextError {
    /// The byte stream is not a parseable PDF.
    #[error("not a parseable PDF: {0}")]
    Parse(String),
    /// The PDF declares an `Encrypt` dictionary. T-061 does not
    /// attempt decryption; a future bead can supply credentials.
    #[error("PDF is encrypted; T-061 does not decrypt")]
    Encrypted,
    /// The PDF parsed but a page's content stream could not be
    /// decoded or interpreted.
    #[error("page {page} content stream malformed: {detail}")]
    Page {
        /// 0-based page index that failed.
        page: usize,
        /// Operator-readable reason.
        detail: String,
    },
    /// The PDF exceeds an intake resource ceiling (too many pages, or
    /// too many text fragments). The input is treated as abusive rather
    /// than processed to exhaustion. The bounds are deliberately
    /// generous; a conformant invoice never trips them.
    #[error("PDF exceeds intake limits: {detail}")]
    TooLarge {
        /// Operator-readable reason (which ceiling was crossed).
        detail: String,
    },
}

/// Canonical Cargo package name of this crate.
///
/// Used by the InvoiceKit release tooling and by the bead-correlation
/// reports to map runtime log records back to the originating crate
/// without parsing `Cargo.toml` at runtime.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_intake_pdf::crate_name(), "invoicekit-intake-pdf");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-intake-pdf"
}

#[cfg(test)]
mod tests {
    use super::crate_name;

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-intake-pdf");
    }

    #[test]
    fn crate_name_is_non_empty() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn crate_name_is_lowercase_kebab() {
        let n = crate_name();
        for c in n.chars() {
            assert!(
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
                "non-kebab char in {n}: {c:?}"
            );
        }
    }

    #[test]
    fn crate_name_carries_invoicekit_prefix() {
        let n = crate_name();
        assert!(
            n == "invoicekit" || n.starts_with("invoicekit-") || n.starts_with("invoicekit_"),
            "crate name does not advertise InvoiceKit family: {n}"
        );
    }
}
