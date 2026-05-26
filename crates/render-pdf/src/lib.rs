// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-render-pdf` — InvoiceKit workspace member.
//!
//! This crate owns the deterministic Typst rendering path. It currently exposes
//! the smallest useful surface: compile an in-memory Typst template using only
//! embedded Typst fonts and export PDF bytes with a stable identifier and fixed
//! timestamp.

use std::path::PathBuf;

use thiserror::Error;
use typst::diag::{FileError, FileResult, SourceDiagnostic};
use typst::foundations::{Bytes, Datetime, Smart};
use typst::layout::PagedDocument;
use typst::syntax::{FileId, Source, VirtualPath};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, World};
use typst_kit::fonts::{FontSlot, Fonts};
use typst_pdf::{PdfOptions, PdfStandard, PdfStandards, Timestamp};

/// Minimal deterministic invoice template used by the T-050 smoke render.
///
/// The template intentionally uses only bundled Typst fonts and fixed document
/// metadata, so it is suitable for byte-stability regression tests.
///
/// # Examples
///
/// ```
/// assert!(invoicekit_render_pdf::HELLO_WORLD_INVOICE_TEMPLATE.contains("InvoiceKit"));
/// ```
pub const HELLO_WORLD_INVOICE_TEMPLATE: &str = r#"
#set document(
  title: "InvoiceKit Hello Invoice",
  author: "InvoiceKit",
  date: datetime(year: 2026, month: 1, day: 1),
)
#set page(width: 210mm, height: 297mm, margin: 18mm)
#set text(font: "Libertinus Serif", size: 10pt)

#align(center)[
  = InvoiceKit Hello Invoice
]

#v(12pt)

#grid(
  columns: (1fr, 1fr),
  [*Supplier* \ InvoiceKit Trust Toolkit],
  [*Customer* \ Deterministic Renderer Test],
)

#v(12pt)

#table(
  columns: (1fr, auto, auto),
  [*Description*], [*Qty*], [*Amount*],
  [Trust toolkit render smoke test], [1], [EUR 1.00],
)

#v(12pt)

Total due: *EUR 1.00*
"#;

/// PDF conformance mode requested from Typst's PDF exporter.
///
/// InvoiceKit still verifies PDF/A-3 with veraPDF in T-052. This enum selects
/// the mode Typst should attempt to emit; it is not a replacement for reference
/// conformance verification.
///
/// # Examples
///
/// ```
/// let request = invoicekit_render_pdf::RenderRequest::new("#set page(width: 10mm, height: 10mm)\nHi", "example")
///     .with_profile(invoicekit_render_pdf::PdfProfile::Pdf17);
/// assert_eq!(request.profile(), invoicekit_render_pdf::PdfProfile::Pdf17);
/// ```
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PdfProfile {
    /// Ordinary PDF 1.7 output.
    Pdf17,
    /// PDF/A-3b output as emitted by Typst.
    PdfA3b,
}

/// A request to render an in-memory Typst source to PDF bytes.
///
/// # Examples
///
/// ```
/// let request = invoicekit_render_pdf::RenderRequest::new("Hello", "hello");
/// assert_eq!(request.stable_id(), "hello");
/// ```
#[derive(Debug, Clone, Copy)]
pub struct RenderRequest<'a> {
    source: &'a str,
    stable_id: &'a str,
    profile: PdfProfile,
}

impl<'a> RenderRequest<'a> {
    /// Creates a render request using the default PDF/A-3b profile.
    ///
    /// The `stable_id` should remain identical for identical logical documents;
    /// Typst hashes it into the PDF document identifier.
    ///
    /// # Examples
    ///
    /// ```
    /// let request = invoicekit_render_pdf::RenderRequest::new("Hello", "invoice-1");
    /// assert_eq!(request.profile(), invoicekit_render_pdf::PdfProfile::PdfA3b);
    /// ```
    #[must_use]
    pub const fn new(source: &'a str, stable_id: &'a str) -> Self {
        Self {
            source,
            stable_id,
            profile: PdfProfile::PdfA3b,
        }
    }

    /// Selects a different Typst PDF export profile.
    ///
    /// # Examples
    ///
    /// ```
    /// let request = invoicekit_render_pdf::RenderRequest::new("Hello", "invoice-1")
    ///     .with_profile(invoicekit_render_pdf::PdfProfile::Pdf17);
    /// assert_eq!(request.profile(), invoicekit_render_pdf::PdfProfile::Pdf17);
    /// ```
    #[must_use]
    pub const fn with_profile(mut self, profile: PdfProfile) -> Self {
        self.profile = profile;
        self
    }

    /// Returns the Typst source that will be rendered.
    ///
    /// # Examples
    ///
    /// ```
    /// let request = invoicekit_render_pdf::RenderRequest::new("Hello", "invoice-1");
    /// assert_eq!(request.source(), "Hello");
    /// ```
    #[must_use]
    pub const fn source(&self) -> &'a str {
        self.source
    }

    /// Returns the stable document identifier used for PDF export.
    ///
    /// # Examples
    ///
    /// ```
    /// let request = invoicekit_render_pdf::RenderRequest::new("Hello", "invoice-1");
    /// assert_eq!(request.stable_id(), "invoice-1");
    /// ```
    #[must_use]
    pub const fn stable_id(&self) -> &'a str {
        self.stable_id
    }

    /// Returns the requested PDF profile.
    ///
    /// # Examples
    ///
    /// ```
    /// let request = invoicekit_render_pdf::RenderRequest::new("Hello", "invoice-1");
    /// assert_eq!(request.profile(), invoicekit_render_pdf::PdfProfile::PdfA3b);
    /// ```
    #[must_use]
    pub const fn profile(&self) -> PdfProfile {
        self.profile
    }
}

/// Errors returned by the Typst PDF renderer.
#[derive(Debug, Error)]
pub enum RenderPdfError {
    /// The Typst source failed to compile.
    #[error(
        "Typst compilation failed: {message}. Hint: check the template syntax and supported Typst features."
    )]
    Compile {
        /// Joined Typst diagnostic messages.
        message: String,
    },
    /// The requested PDF profile is not supported by the Typst exporter.
    #[error(
        "PDF profile {profile:?} is not supported by Typst: {message}. Hint: choose a compatible PDF profile or add the missing renderer support before enabling this profile."
    )]
    Profile {
        /// Requested profile.
        profile: PdfProfile,
        /// Typst profile error.
        message: String,
    },
    /// Typst failed while exporting the compiled document to PDF bytes.
    #[error(
        "Typst PDF export failed: {message}. Hint: inspect the template resources and PDF profile settings."
    )]
    Export {
        /// Joined Typst export diagnostic messages.
        message: String,
    },
    /// The fixed deterministic timestamp could not be constructed.
    #[error(
        "internal deterministic timestamp is invalid. Hint: report an invoicekit-render-pdf bug."
    )]
    InvalidFixedTimestamp,
}

/// Renders the built-in hello-world invoice template to PDF bytes.
///
/// # Examples
///
/// ```
/// let pdf = invoicekit_render_pdf::render_hello_world_invoice().unwrap();
/// assert!(pdf.starts_with(b"%PDF-"));
/// ```
///
/// # Errors
///
/// Returns [`RenderPdfError`] if the built-in template fails to compile, the
/// fixed deterministic timestamp cannot be constructed, or Typst cannot export
/// the compiled document to PDF bytes.
pub fn render_hello_world_invoice() -> Result<Vec<u8>, RenderPdfError> {
    render_typst_pdf(RenderRequest::new(
        HELLO_WORLD_INVOICE_TEMPLATE,
        "invoicekit:t-050:hello-world",
    ))
}

/// Renders an in-memory Typst source to PDF bytes.
///
/// The renderer does not read templates from disk and does not load system
/// fonts. This keeps the first Typst integration deterministic and reviewable;
/// richer template loading belongs in T-051.
///
/// # Examples
///
/// ```
/// let request = invoicekit_render_pdf::RenderRequest::new(
///     "#set page(width: 30mm, height: 20mm)\nHello",
///     "example",
/// ).with_profile(invoicekit_render_pdf::PdfProfile::Pdf17);
/// let pdf = invoicekit_render_pdf::render_typst_pdf(request).unwrap();
/// assert!(pdf.starts_with(b"%PDF-"));
/// ```
///
/// # Errors
///
/// Returns [`RenderPdfError`] when Typst rejects the source, when deterministic
/// PDF metadata cannot be constructed, or when PDF export fails for the selected
/// profile.
pub fn render_typst_pdf(request: RenderRequest<'_>) -> Result<Vec<u8>, RenderPdfError> {
    let world = InMemoryWorld::new(request.source);
    let warned = typst::compile::<PagedDocument>(&world);
    let document = warned
        .output
        .map_err(|diagnostics| RenderPdfError::Compile {
            message: join_diagnostics(&diagnostics),
        })?;

    let options = PdfOptions {
        ident: Smart::Custom(request.stable_id),
        timestamp: Some(Timestamp::new_utc(fixed_datetime()?)),
        page_ranges: None,
        standards: pdf_standards(request.profile)?,
    };

    typst_pdf::pdf(&document, &options).map_err(|diagnostics| RenderPdfError::Export {
        message: join_diagnostics(&diagnostics),
    })
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
/// assert_eq!(invoicekit_render_pdf::crate_name(), "invoicekit-render-pdf");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-render-pdf"
}

struct InMemoryWorld {
    main: FileId,
    source: Source,
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<FontSlot>,
}

impl InMemoryWorld {
    fn new(source: &str) -> Self {
        let main = FileId::new(None, VirtualPath::new("invoice.typ"));
        let mut font_searcher = Fonts::searcher();
        let fonts = font_searcher
            .include_system_fonts(false)
            .include_embedded_fonts(true)
            .search();

        Self {
            main,
            source: Source::new(main, source.to_owned()),
            library: LazyHash::new(Library::default()),
            book: LazyHash::new(fonts.book),
            fonts: fonts.fonts,
        }
    }
}

impl World for InMemoryWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main {
            Ok(self.source.clone())
        } else {
            Err(FileError::NotFound(PathBuf::from(
                id.vpath().as_rootless_path(),
            )))
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        Err(FileError::NotFound(PathBuf::from(
            id.vpath().as_rootless_path(),
        )))
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index)?.get()
    }

    fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
        Datetime::from_ymd(2026, 1, 1)
    }
}

fn pdf_standards(profile: PdfProfile) -> Result<PdfStandards, RenderPdfError> {
    let standards = match profile {
        PdfProfile::Pdf17 => &[PdfStandard::V_1_7],
        PdfProfile::PdfA3b => &[PdfStandard::A_3b],
    };

    PdfStandards::new(standards).map_err(|message| RenderPdfError::Profile {
        profile,
        message: message.to_string(),
    })
}

fn fixed_datetime() -> Result<Datetime, RenderPdfError> {
    Datetime::from_ymd_hms(2026, 1, 1, 0, 0, 0).ok_or(RenderPdfError::InvalidFixedTimestamp)
}

fn join_diagnostics(diagnostics: &[SourceDiagnostic]) -> String {
    let mut message = String::new();

    for diagnostic in diagnostics {
        if !message.is_empty() {
            message.push_str("; ");
        }
        message.push_str(diagnostic.message.as_str());
    }

    message
}

#[cfg(test)]
mod tests {
    use super::{
        crate_name, render_hello_world_invoice, render_typst_pdf, PdfProfile, RenderPdfError,
        RenderRequest,
    };

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-render-pdf");
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

    #[test]
    fn hello_world_invoice_renders_pdf_a3_bytes() {
        let pdf = render_hello_world_invoice().expect("hello-world invoice should render");

        assert!(pdf.starts_with(b"%PDF-"));
        assert!(
            pdf.windows(b"pdfaid:part".len())
                .any(|window| window == b"pdfaid:part"),
            "PDF/A identification metadata missing from Typst PDF output"
        );
    }

    #[test]
    fn hello_world_invoice_is_byte_stable() {
        let first = render_hello_world_invoice().expect("first render should succeed");
        let second = render_hello_world_invoice().expect("second render should succeed");

        assert_eq!(first, second);
    }

    #[test]
    fn pdf17_profile_renders_without_pdfa_marker_requirement() {
        let request = RenderRequest::new(
            "#set page(width: 30mm, height: 20mm)\nHello from InvoiceKit",
            "invoicekit:test:pdf17",
        )
        .with_profile(PdfProfile::Pdf17);

        let pdf = render_typst_pdf(request).expect("PDF 1.7 render should succeed");

        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn invalid_template_returns_typed_compile_error() {
        let request = RenderRequest::new("#let broken = )", "invoicekit:test:broken");
        let error = render_typst_pdf(request).expect_err("invalid Typst should fail");

        assert!(matches!(error, RenderPdfError::Compile { .. }));
        assert!(error.to_string().contains("Hint:"));
    }

    #[test]
    fn imported_files_are_rejected_as_missing() {
        let request = RenderRequest::new("#read(\"/etc/passwd\")", "invoicekit:test:read");
        let error = render_typst_pdf(request).expect_err("file access should fail");

        assert!(matches!(error, RenderPdfError::Compile { .. }));
    }

    #[test]
    fn invalid_page_width_is_reported_without_panic() {
        let request = RenderRequest::new(
            "#set page(width: \"wide\")\nHello",
            "invoicekit:test:invalid-page",
        );
        let error = render_typst_pdf(request).expect_err("invalid page width should fail");

        assert!(matches!(error, RenderPdfError::Compile { .. }));
    }
}
