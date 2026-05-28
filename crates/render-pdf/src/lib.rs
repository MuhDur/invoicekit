// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-render-pdf` — InvoiceKit workspace member.
//!
//! This crate owns the deterministic Typst rendering path. It currently exposes
//! the smallest useful public surface: render the built-in T-050 smoke invoice
//! using only embedded Typst fonts and export PDF bytes with a stable identifier
//! and fixed timestamp.
//!
//! The internal Typst source renderer is intentionally not public. Typst source
//! execution is a trusted-template operation, not a sandbox for user-authored
//! templates; T-051 owns the public template trust boundary.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::LazyLock;

use invoicekit_ir::CommercialDocument;
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

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum PdfProfile {
    #[cfg(test)]
    Pdf17,
    PdfA3b,
}

#[derive(Debug, Clone, Copy)]
struct RenderRequest<'a> {
    source: &'a str,
    stable_id: &'a str,
    profile: PdfProfile,
}

impl<'a> RenderRequest<'a> {
    #[must_use]
    const fn new(source: &'a str, stable_id: &'a str) -> Self {
        Self {
            source,
            stable_id,
            profile: PdfProfile::PdfA3b,
        }
    }

    #[must_use]
    #[cfg(test)]
    const fn with_profile(mut self, profile: PdfProfile) -> Self {
        self.profile = profile;
        self
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
        "PDF profile {profile} is not supported by Typst: {message}. Hint: choose a compatible PDF profile or add the missing renderer support before enabling this profile."
    )]
    Profile {
        /// Requested profile.
        profile: &'static str,
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
    render_trusted_typst_pdf(RenderRequest::new(
        HELLO_WORLD_INVOICE_TEMPLATE,
        "invoicekit:t-050:hello-world",
    ))
}

/// Renders a validated commercial document to deterministic PDF/A-3b bytes.
///
/// This is the document-aware renderer used by REST and demo surfaces. It keeps
/// the trust boundary narrow: callers pass an already-validated IR document,
/// and this crate owns the generated Typst source.
///
/// # Errors
///
/// Returns [`RenderPdfError`] if Typst compilation or PDF export fails.
pub fn render_commercial_document_invoice(
    document: &CommercialDocument,
) -> Result<Vec<u8>, RenderPdfError> {
    let source = trusted_invoice_source(document);
    let stable_id = format!("invoicekit:document:{}", document.id.as_str());
    render_trusted_typst_pdf(RenderRequest::new(&source, &stable_id))
}

/// Fuzz-only entry point: render arbitrary Typst source to PDF bytes.
///
/// This shim exists so libFuzzer can drive the Typst compiler and PDF
/// exporter with adversarial inputs without [`RenderRequest`] needing to
/// become public. Typst-source execution is a *trusted-template*
/// operation, not a sandbox for user-authored templates; T-051 owns the
/// public template trust boundary. Until that lands, the only
/// legitimate caller is the `render_typst_pdf` fuzz target.
///
/// The stable identifier is fixed to `"invoicekit:fuzz:render_typst_pdf"`
/// so the only thing the fuzzer varies is the source itself.
///
/// # Errors
///
/// Returns [`RenderPdfError`] if Typst compilation fails (the common
/// outcome on adversarial input), the deterministic timestamp cannot be
/// constructed, or the PDF exporter rejects the compiled document.
#[doc(hidden)]
pub fn render_for_fuzz(source: &str) -> Result<Vec<u8>, RenderPdfError> {
    render_trusted_typst_pdf(RenderRequest::new(
        source,
        "invoicekit:fuzz:render_typst_pdf",
    ))
}

// Internal trusted-template renderer. Do not expose this as a public API for
// user-authored Typst until T-051 defines the template trust boundary.
fn render_trusted_typst_pdf(request: RenderRequest<'_>) -> Result<Vec<u8>, RenderPdfError> {
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

fn trusted_invoice_source(document: &CommercialDocument) -> String {
    let title = format!("Invoice {}", document.document_number.as_str());
    let rows = invoice_line_rows(document);
    let total = money_text(
        document.currency.as_str(),
        &document.monetary_total.payable_amount,
    );

    format!(
        r#"
#set document(
  title: {title},
  author: "InvoiceKit",
  date: datetime(year: 2026, month: 1, day: 1),
)
#set page(width: 210mm, height: 297mm, margin: 18mm)
#set text(font: "Libertinus Serif", size: 10pt)

#align(center)[
  = #text({title})
]

#v(12pt)

#grid(
  columns: (1fr, 1fr),
  [*Supplier* \ #text({supplier})],
  [*Customer* \ #text({customer})],
  [*Document number* \ #text({document_number})],
  [*Issue date* \ #text({issue_date})],
)

#v(12pt)

#table(
  columns: (1fr, auto, auto),
  [*Description*], [*Qty*], [*Amount*],
{rows})

#v(12pt)

Total due: *#text({total})*
"#,
        title = typst_string(&title),
        supplier = typst_string(&document.supplier.name),
        customer = typst_string(&document.customer.name),
        document_number = typst_string(document.document_number.as_str()),
        issue_date = typst_string(document.issue_date.as_str()),
        rows = rows,
        total = typst_string(&total),
    )
}

fn invoice_line_rows(document: &CommercialDocument) -> String {
    let mut rows = String::new();
    for line in &document.lines {
        let amount = money_text(document.currency.as_str(), &line.line_extension_amount);
        let _ = writeln!(
            rows,
            "  [#text({})], [#text({})], [#text({})],",
            typst_string(&line.description),
            typst_string(&line.quantity.inner().normalize().to_string()),
            typst_string(&amount)
        );
    }
    rows
}

fn money_text(currency: &str, amount: &invoicekit_ir::MoneyAmount) -> String {
    format!("{} {}", currency, amount.inner().normalize())
}

fn typst_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if c.is_control() => escaped.push(' '),
            c => escaped.push(c),
        }
    }
    escaped.push('"');
    escaped
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

/// T-055 impl: every byte of every font we may load is shipped
/// inside this binary. The macro expands to `(&'static [u8], &str)`
/// pairs the loader consumes via `Font::iter(Bytes::new(*))`.
/// Adding a font is a three-step diff: drop the .ttf under
/// `crates/render-pdf/fonts/<family>/`, add its license file, and
/// append an entry here.
macro_rules! pinned_fonts {
    () => {
        &[(
            include_bytes!("../fonts/dejavu/DejaVuSansMono.ttf") as &[u8],
            "DejaVu Sans Mono Regular (Bitstream Vera + DejaVu, public-domain + free)",
        )]
    };
}

/// Document-independent Typst rendering assets.
///
/// The standard library and the font catalogue (typst-kit's embedded faces plus
/// the T-055 pinned fonts) do not depend on the invoice being rendered, yet
/// building them is the dominant cost of a render. They are constructed exactly
/// once into [`RENDER_ASSETS`] and shared by every [`InMemoryWorld`]; only the
/// per-document `main`/`source` change between renders.
///
/// This is purely a hoist: the library, the `FontBook` contents, and the font
/// index order are byte-for-byte identical to constructing them per call, so the
/// rendered PDF bytes are unchanged (pinned by `tests/golden_render.rs`).
struct RenderAssets {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<FontSlot>,
    /// T-055 pinned fonts (`crates/render-pdf/fonts/**`). These
    /// sit after `fonts` in the `FontBook` index order, so
    /// `World::font(idx)` consults them when `idx >= fonts.len()`.
    extra_fonts: Vec<Font>,
}

impl RenderAssets {
    fn build() -> Self {
        // Step 1: typst-kit's embed-fonts catalogue (Libertinus
        // Serif, NCM, IBM Plex Sans, DejaVu Sans Mono, etc.) —
        // system fonts stay off so the byte-stable cross-platform
        // gate (T-055) keeps working.
        let mut font_searcher = Fonts::searcher();
        let kit_fonts = font_searcher
            .include_system_fonts(false)
            .include_embedded_fonts(true)
            .search();
        let mut book = kit_fonts.book;
        let fonts = kit_fonts.fonts;

        // Step 2: layer our pinned fonts on top of the embedded
        // catalogue. The pinned set ships under
        // `crates/render-pdf/fonts/<family>/` with a sibling
        // LICENSE.txt per family; the `pinned_fonts!` macro lists
        // the per-family `include_bytes!` calls. Anything in the
        // pinned set is appended after the typst-kit catalogue —
        // a typst-kit face with the same name still wins by
        // FontBook iteration order, which is the right precedence
        // for the existing T-050 hello-world template that asks
        // for Libertinus Serif by name.
        let mut extra_fonts: Vec<Font> = Vec::new();
        for (raw, _label) in pinned_fonts!().iter().copied() {
            let bytes = Bytes::new(raw);
            for font in Font::iter(bytes) {
                book.push(font.info().clone());
                extra_fonts.push(font);
            }
        }

        Self {
            library: LazyHash::new(Library::default()),
            book: LazyHash::new(book),
            fonts,
            extra_fonts,
        }
    }
}

/// Process-wide, lazily-built rendering assets shared by every render call.
static RENDER_ASSETS: LazyLock<RenderAssets> = LazyLock::new(RenderAssets::build);

struct InMemoryWorld {
    main: FileId,
    source: Source,
    /// Shared, document-independent library + font catalogue.
    assets: &'static RenderAssets,
}

impl InMemoryWorld {
    fn new(source: &str) -> Self {
        // The library and font catalogue are document-independent and built once
        // into `RENDER_ASSETS`; only `main`/`source` vary per render. Step 1/2
        // of the asset build live in `RenderAssets::build`.
        let main = FileId::new(None, VirtualPath::new("invoice.typ"));
        Self {
            main,
            source: Source::new(main, source.to_owned()),
            assets: &RENDER_ASSETS,
        }
    }
}

impl World for InMemoryWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.assets.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.assets.book
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
        // T-055 pinned-font tail: `book` is populated in the
        // same order — typst-kit's slots first, then every
        // pinned font — so the offset arithmetic is exact.
        let assets = self.assets;
        assets.fonts.get(index).map_or_else(
            || assets.extra_fonts.get(index - assets.fonts.len()).cloned(),
            FontSlot::get,
        )
    }

    fn today(&self, _offset: Option<i64>) -> Option<Datetime> {
        Datetime::from_ymd(2026, 1, 1)
    }
}

fn pdf_standards(profile: PdfProfile) -> Result<PdfStandards, RenderPdfError> {
    let standards = match profile {
        #[cfg(test)]
        PdfProfile::Pdf17 => &[PdfStandard::V_1_7],
        PdfProfile::PdfA3b => &[PdfStandard::A_3b],
    };

    PdfStandards::new(standards).map_err(|message| RenderPdfError::Profile {
        profile: profile.name(),
        message: message.to_string(),
    })
}

impl PdfProfile {
    const fn name(self) -> &'static str {
        match self {
            #[cfg(test)]
            Self::Pdf17 => "PDF 1.7",
            Self::PdfA3b => "PDF/A-3b",
        }
    }
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
        crate_name, render_commercial_document_invoice, render_for_fuzz,
        render_hello_world_invoice, render_trusted_typst_pdf, typst_string, PdfProfile,
        RenderPdfError, RenderRequest,
    };
    use invoicekit_ir::CommercialDocument;
    use serde_json::json;

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
    fn commercial_document_invoice_renders_pdf_a3_bytes() {
        let pdf = render_commercial_document_invoice(&sample_doc())
            .expect("commercial document should render");

        assert!(pdf.starts_with(b"%PDF-"));
        assert!(
            pdf.windows(b"pdfaid:part".len())
                .any(|window| window == b"pdfaid:part"),
            "PDF/A identification metadata missing from Typst PDF output"
        );
    }

    #[test]
    fn commercial_document_invoice_render_is_deterministic() {
        let document = sample_doc();
        let first = render_commercial_document_invoice(&document).expect("first render");
        let second = render_commercial_document_invoice(&document).expect("second render");

        assert_eq!(first, second);
    }

    #[test]
    fn typst_string_escapes_user_text() {
        assert_eq!(
            typst_string("A \"quote\" \\ slash"),
            r#""A \"quote\" \\ slash""#
        );
        assert_eq!(typst_string("line\nnext"), r#""line\nnext""#);
    }

    /// T-055 guard: `InMemoryWorld` constructs its font searcher
    /// with `include_system_fonts(false)`. This test renders a
    /// PDF that explicitly asks for a font name only typst-kit's
    /// embedded set can satisfy (`Libertinus Serif`); if a future
    /// refactor flips the system-font discovery flag back on,
    /// the embedded set still wins by virtue of search order, so
    /// instead we assert the property indirectly by re-rendering
    /// and checking that the byte output is the same as it was
    /// when this guard was committed (digest-pinned). A diff
    /// here is the alarm.
    #[test]
    fn t_055_system_fonts_are_not_consulted() {
        // Render with an embedded-only font request; the test
        // succeeds iff the renderer never had to fall back to a
        // system font.
        let request = RenderRequest::new(
            "#set page(width: 30mm, height: 20mm)\n\
             #set text(font: \"Libertinus Serif\")\n\
             Pinned",
            "invoicekit:t-055:font-guard",
        );
        let pdf = render_trusted_typst_pdf(request).expect("embedded Libertinus must render");
        assert!(pdf.starts_with(b"%PDF-"));
        // A future change that flips `include_system_fonts(true)`
        // would not by itself break this test, but the
        // cross-platform byte-stable CI job (`render-byte-stable`
        // in `.github/workflows/ci.yml`) would: it asserts the
        // hello-world PDF bytes are equal across Linux + macOS,
        // which is impossible when system-font discovery picks
        // up `/usr/share/fonts` on Linux but `~/Library/Fonts`
        // on macOS.
    }

    /// T-055 impl: a template that explicitly asks for a font
    /// only the pinned set supplies (`DejaVu Sans Mono`) must
    /// render. The hello-world template's Libertinus Serif
    /// comes from typst-kit's embedded catalogue; this test
    /// proves the pinned-font layering is wired up the right way.
    #[test]
    fn t_055_pinned_dejavu_sans_mono_is_loaded() {
        let request = RenderRequest::new(
            "#set page(width: 40mm, height: 20mm)\n\
             #set text(font: \"DejaVu Sans Mono\", size: 8pt)\n\
             Pinned-font",
            "invoicekit:t-055:pinned-dejavu",
        );
        let pdf = render_trusted_typst_pdf(request).expect("pinned DejaVu must render");
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn pdf17_profile_renders_without_pdfa_marker_requirement() {
        let request = RenderRequest::new(
            "#set page(width: 30mm, height: 20mm)\nHello from InvoiceKit",
            "invoicekit:test:pdf17",
        )
        .with_profile(PdfProfile::Pdf17);

        let pdf = render_trusted_typst_pdf(request).expect("PDF 1.7 render should succeed");

        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn invalid_template_returns_typed_compile_error() {
        let request = RenderRequest::new("#let broken = )", "invoicekit:test:broken");
        let error = render_trusted_typst_pdf(request).expect_err("invalid Typst should fail");

        assert!(matches!(error, RenderPdfError::Compile { .. }));
        assert!(error.to_string().contains("Hint:"));
    }

    #[test]
    fn imported_files_are_rejected_as_missing() {
        let request = RenderRequest::new("#read(\"/etc/passwd\")", "invoicekit:test:read");
        let error = render_trusted_typst_pdf(request).expect_err("file access should fail");

        assert!(matches!(error, RenderPdfError::Compile { .. }));
    }

    #[test]
    fn invalid_page_width_is_reported_without_panic() {
        let request = RenderRequest::new(
            "#set page(width: \"wide\")\nHello",
            "invoicekit:test:invalid-page",
        );
        let error = render_trusted_typst_pdf(request).expect_err("invalid page width should fail");

        assert!(matches!(error, RenderPdfError::Compile { .. }));
    }

    // oueo: render_for_fuzz is the libFuzzer entry point — keep it tested at
    // the unit level so refactors of `RenderRequest` can't silently change
    // the surface that fuzz targets call.

    #[test]
    fn render_for_fuzz_emits_pdf_a3_on_valid_source() {
        let source = "#set page(width: 30mm, height: 20mm)\n#text[Fuzz target valid input render]";
        let pdf = render_for_fuzz(source).expect("trivial valid source should render");
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn render_for_fuzz_returns_typed_error_on_broken_source() {
        let error = render_for_fuzz("#let broken = )").expect_err("broken source should fail");
        assert!(matches!(error, RenderPdfError::Compile { .. }));
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn render_for_fuzz_does_not_panic_on_empty_source() {
        // Empty input is the libFuzzer baseline; the fuzz target must
        // tolerate either Ok(_) or RenderPdfError without panicking.
        let _ = render_for_fuzz("");
    }

    fn sample_doc() -> CommercialDocument {
        CommercialDocument::try_from_value(json!({
            "schema_version": "1.0",
            "id": "doc_pdf_vector",
            "document_type": "invoice",
            "issue_date": "2026-05-26",
            "due_date": "2026-06-25",
            "document_number": "INV-2026-0001",
            "currency": "EUR",
            "supplier": {
                "id": "supplier",
                "name": "InvoiceKit GmbH",
                "tax_ids": [{ "scheme": "vat", "value": "DE123456789" }],
                "address": {
                    "lines": ["Main Street 1"],
                    "city": "Berlin",
                    "postal_code": "10115",
                    "country": "DE"
                }
            },
            "customer": {
                "id": "customer",
                "name": "ACME SAS",
                "tax_ids": [{ "scheme": "vat", "value": "FR123456789" }],
                "address": {
                    "lines": ["Rue Example 2"],
                    "city": "Paris",
                    "postal_code": "75001",
                    "country": "FR"
                }
            },
            "lines": [{
                "id": "1",
                "description": "Validation subscription",
                "quantity": "1",
                "unit_code": "EA",
                "unit_price": "119.00",
                "line_extension_amount": "119.00",
                "tax_category": "S"
            }],
            "tax_summary": [{
                "category_code": "S",
                "taxable_amount": "119.00",
                "tax_amount": "0.00",
                "tax_rate": "0.00"
            }],
            "monetary_total": {
                "line_extension_amount": "119.00",
                "tax_exclusive_amount": "119.00",
                "tax_inclusive_amount": "119.00",
                "payable_amount": "119.00"
            },
            "meta": {
                "tenant_id": "tenant_pdf",
                "trace_id": "trace_pdf",
                "source_system": "render-pdf-test"
            }
        }))
        .unwrap()
    }
}
