// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-061 text extractor.
//!
//! Walks every page's content stream, tracks the text-positioning
//! state (current Text matrix, leading, font size), and emits one
//! [`TextFragment`] per `Tj` / `TJ` / `'` / `"` operator. Fragments
//! are then sorted into reading order per page by the
//! `script_order` module, which is script-aware: left-to-right
//! Latin lines sort left-to-right, right-to-left lines (Arabic,
//! Hebrew) are rebuilt into logical order via the Unicode
//! Bidirectional Algorithm, and CJK vertical columns are read
//! right-to-left, top-to-bottom.
//!
//! The implementation is intentionally simple: no character mapping
//! (`CMap`) reverse-lookups, no font kerning, no rendering-mode
//! awareness. For embedded `WinAnsiEncoding` / `MacRomanEncoding` /
//! standard Type1 fonts (the common shape of invoice PDFs rendered
//! by Typst, LaTeX, wkhtmltopdf, Apache POI / Word) this produces
//! human-readable text out of the box. Custom-encoded subset fonts
//! produce mangled text — that case routes through OCR via the
//! Layer-3 path and is the documented gap for a follow-up bead.

use std::fmt;

use lopdf::content::Operation;
use lopdf::{Document, Object, ObjectId};

use crate::factur_x::decode_embedded_capped;
use crate::PdfTextError;

/// Maximum number of pages [`extract_pdf_text`] will process from an
/// untrusted PDF before treating the input as abusive. A real invoice
/// is a handful of pages; even a verbose multi-line-item document with
/// supporting annexes stays in the low dozens, so a 4096-page ceiling
/// is generous while still refusing a PDF that declares millions of
/// pages purely to exhaust intake.
const MAX_PAGES: usize = 4096;

/// Maximum number of text fragments [`extract_pdf_text`] will
/// accumulate across the whole document before bailing. Each fragment
/// is an allocation and feeds the per-page reading-order pass, so an
/// unbounded fragment count is a denial-of-service amplifier on a
/// hostile content stream. A dense real page holds a few thousand
/// fragments; a million across the document is a generous ceiling no
/// conformant invoice approaches.
const MAX_FRAGMENTS_TOTAL: usize = 1_000_000;

/// Hard ceiling on the *decompressed* size of a single page's content
/// stream(s), in bytes. A real page's content stream is a few kilobytes
/// to a few megabytes (a graphics-heavy page); even a verbose invoice
/// page stays well under this, so a 64 MiB ceiling is generous for any
/// conformant page while refusing a decompression bomb.
///
/// Without this cap, a hostile PDF can name a page content stream whose
/// `FlateDecode` input is a few kilobytes but inflates to gigabytes, and
/// force an unbounded allocation on intake. lopdf's `get_page_content`
/// helper calls `Stream::decompressed_content()` with no cap, so we walk
/// the page's content-stream object ids ourselves and decode each one
/// through the same size-capped path the embedded-file extractor uses.
const MAX_PAGE_CONTENT_SIZE: u64 = 64 * 1024 * 1024;

/// The full text surface of a single PDF, page by page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StructuredText {
    /// Pages in PDF page-tree order.
    pub pages: Vec<PageText>,
}

impl StructuredText {
    /// Concatenate every fragment's text in reading order. Useful for
    /// quick comparisons and tests; downstream code should prefer the
    /// per-fragment view so it keeps the positions.
    #[must_use]
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for (page_idx, page) in self.pages.iter().enumerate() {
            if page_idx > 0 {
                out.push_str("\n\u{000C}\n");
            }
            for (i, frag) in page.fragments.iter().enumerate() {
                if i > 0 {
                    out.push('\n');
                }
                out.push_str(&frag.text);
            }
        }
        out
    }
}

/// One page's reading-order-sorted fragment list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageText {
    /// 0-based page index in the PDF's page tree.
    pub index: usize,
    /// Page width in PDF user-space units (1/72 inch). Sourced from
    /// the page's `MediaBox` and carried so downstream OCR can compute
    /// relative positions without re-parsing the PDF.
    pub width_pt: f32,
    /// Page height in PDF user-space units.
    pub height_pt: f32,
    /// Text fragments in reading order. Left-to-right lines sort
    /// top-to-bottom then left-to-right; right-to-left lines are
    /// reconstructed into logical order; CJK vertical columns are
    /// emitted right-to-left, each column top-to-bottom. See the
    /// `script_order` module.
    pub fragments: Vec<TextFragment>,
}

/// One run of text emitted by a single `Tj`/`TJ`/`'`/`"` operator.
///
/// `(x, y)` is the origin of the first glyph in PDF user-space units
/// with the origin at the lower-left of the page (so a fragment at
/// the top of a US-Letter page has `y ≈ 720`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextFragment {
    /// Horizontal position of the fragment's first glyph.
    pub x: f32,
    /// Vertical position of the fragment's first glyph (origin
    /// bottom-left, so larger values are higher on the page).
    pub y: f32,
    /// Current font size in points, as set by the most-recent `Tf`.
    pub font_size: f32,
    /// Decoded text for this run.
    pub text: String,
}

impl fmt::Display for TextFragment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({:.1},{:.1}) {}", self.x, self.y, self.text)
    }
}

/// Public entry point for the bead.
///
/// # Errors
///
/// Returns [`PdfTextError::Parse`] when `bytes` isn't a valid PDF,
/// [`PdfTextError::Encrypted`] when the PDF carries an `Encrypt`
/// dictionary (T-061 doesn't decrypt), and [`PdfTextError::Page`]
/// when a specific page's content stream fails to parse.
pub fn extract_pdf_text(bytes: &[u8]) -> Result<StructuredText, PdfTextError> {
    let doc = Document::load_mem(bytes).map_err(|e| PdfTextError::Parse(e.to_string()))?;
    if doc.trailer.get(b"Encrypt").is_ok() {
        return Err(PdfTextError::Encrypted);
    }

    let all_pages = doc.get_pages();
    if all_pages.len() > MAX_PAGES {
        return Err(PdfTextError::TooLarge {
            detail: format!(
                "page count {} exceeds ceiling {MAX_PAGES}",
                all_pages.len()
            ),
        });
    }

    let mut pages = Vec::new();
    let mut fragments_so_far = 0usize;
    for (idx, (_page_no, page_id)) in all_pages.into_iter().enumerate() {
        let (width_pt, height_pt) = page_size(&doc, page_id);
        let fragments = extract_page(&doc, page_id)
            .map_err(|detail| PdfTextError::Page { page: idx, detail })?;
        // Cap the *total* fragment count across the document before the
        // per-page reading-order pass runs, so a single hostile content
        // stream that emits millions of show-text operators cannot force
        // unbounded allocation or super-linear sorting/clustering work.
        fragments_so_far = fragments_so_far.saturating_add(fragments.len());
        if fragments_so_far > MAX_FRAGMENTS_TOTAL {
            return Err(PdfTextError::TooLarge {
                detail: format!(
                    "text-fragment count exceeds ceiling {MAX_FRAGMENTS_TOTAL}"
                ),
            });
        }
        let fragments = crate::script_order::reading_order(fragments);
        pages.push(PageText {
            index: idx,
            width_pt,
            height_pt,
            fragments,
        });
    }
    Ok(StructuredText { pages })
}

fn page_size(doc: &Document, page_id: ObjectId) -> (f32, f32) {
    // MediaBox can be inherited from a parent Pages node, so search
    // up the tree if the page itself doesn't declare one.
    let mut id = page_id;
    for _ in 0..16 {
        let Ok(obj) = doc.get_dictionary(id) else {
            break;
        };
        if let Ok(mb) = obj.get(b"MediaBox") {
            if let Some(b) = box_dims(mb) {
                return b;
            }
        }
        if let Ok(Object::Reference(parent)) = obj.get(b"Parent") {
            id = *parent;
            continue;
        }
        break;
    }
    // US-Letter default; harmless for downstream code that only
    // wants the relative layout.
    (612.0, 792.0)
}

fn box_dims(obj: &Object) -> Option<(f32, f32)> {
    let arr = obj.as_array().ok()?;
    if arr.len() != 4 {
        return None;
    }
    let x0 = as_number(&arr[0])?;
    let y0 = as_number(&arr[1])?;
    let x1 = as_number(&arr[2])?;
    let y1 = as_number(&arr[3])?;
    Some(((x1 - x0).abs(), (y1 - y0).abs()))
}

fn extract_page(doc: &Document, page_id: ObjectId) -> Result<Vec<TextFragment>, String> {
    let content = page_content_capped(doc, page_id)?;
    let content = lopdf::content::Content::decode(&content).map_err(|e| e.to_string())?;

    let mut state = State::default();
    let mut frags: Vec<TextFragment> = Vec::new();
    for op in content.operations {
        apply_operator(&op, &mut state, &mut frags);
    }
    Ok(frags)
}

/// Concatenate a page's content stream(s) with a hard cap on the total
/// decompressed size.
///
/// lopdf's `Document::get_page_content` calls
/// `Stream::decompressed_content()` on every page content stream with no
/// size cap, which is a decompression-bomb sink on an untrusted PDF: a
/// hostile page can name a `FlateDecode` content stream whose few
/// kilobytes of input inflate to gigabytes. Instead we walk the page's
/// content-stream object ids and decode each one through
/// [`decode_embedded_capped`] — the same size-capped path the
/// embedded-file extractor uses — accumulating against
/// [`MAX_PAGE_CONTENT_SIZE`]. A page whose decoded content crosses the
/// ceiling is refused with an error rather than materialised.
fn page_content_capped(doc: &Document, page_id: ObjectId) -> Result<Vec<u8>, String> {
    let mut content: Vec<u8> = Vec::new();
    let mut budget = MAX_PAGE_CONTENT_SIZE;
    for object_id in doc.get_page_contents(page_id) {
        let Ok(stream) = doc.get_object(object_id).and_then(Object::as_stream) else {
            // A content-stream reference that does not resolve to a
            // stream is not text we can read; skip it, mirroring lopdf's
            // own lenient handling.
            continue;
        };
        // `budget` is whatever ceiling remains after earlier streams on
        // this page, so the *total* decoded content is capped, not just
        // each stream individually.
        let Some(decoded) = decode_embedded_capped(stream, budget) else {
            return Err(format!(
                "page content stream exceeds {MAX_PAGE_CONTENT_SIZE}-byte decompression ceiling"
            ));
        };
        budget = budget.saturating_sub(decoded.len() as u64);
        content.extend_from_slice(&decoded);
        // lopdf inserts no separator between concatenated content
        // streams; the PDF spec treats them as one logical stream and a
        // token cannot straddle the boundary, so a newline is harmless
        // and matches `get_page_content`'s `write_all` concatenation.
    }
    Ok(content)
}

#[derive(Debug, Default)]
struct State {
    in_text: bool,
    // 6-element affine for the current text matrix.
    tm: [f32; 6],
    // Same for the line matrix (anchor for the current line).
    lm: [f32; 6],
    font_size: f32,
    leading: f32,
}

impl State {
    fn set_tm(&mut self, m: [f32; 6]) {
        self.tm = m;
        self.lm = m;
    }
    fn translate_text(&mut self, tx: f32, ty: f32) {
        // PDF Td: lm := lm * [[1,0],[0,1],[tx,ty]]; tm := lm
        self.lm = matmul(&[1.0, 0.0, 0.0, 1.0, tx, ty], &self.lm);
        self.tm = self.lm;
    }
}

fn apply_operator(op: &Operation, state: &mut State, frags: &mut Vec<TextFragment>) {
    match op.operator.as_str() {
        "BT" => {
            state.in_text = true;
            state.set_tm([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        }
        "ET" => {
            state.in_text = false;
        }
        "Tf" => {
            // Tf font size: name + number. We ignore the font name
            // for now and just remember the size in points.
            if let Some(size) = op.operands.get(1).and_then(as_number) {
                state.font_size = size;
            }
        }
        "TL" => {
            if let Some(l) = op.operands.first().and_then(as_number) {
                state.leading = l;
            }
        }
        "Td" => {
            if let Some((tx, ty)) = operand_pair(op) {
                state.translate_text(tx, ty);
            }
        }
        "TD" => {
            if let Some((tx, ty)) = operand_pair(op) {
                state.leading = -ty;
                state.translate_text(tx, ty);
            }
        }
        "Tm" if op.operands.len() == 6 => {
            let mut m = [0.0f32; 6];
            for (i, o) in op.operands.iter().enumerate() {
                m[i] = as_number(o).unwrap_or(0.0);
            }
            state.set_tm(m);
        }
        "T*" => {
            state.translate_text(0.0, -state.leading);
        }
        "Tj" => emit(op.operands.first(), state, frags),
        "'" => {
            state.translate_text(0.0, -state.leading);
            emit(op.operands.first(), state, frags);
        }
        "\"" => {
            // " operator takes aw ac string; we ignore aw/ac and emit.
            state.translate_text(0.0, -state.leading);
            emit(op.operands.get(2), state, frags);
        }
        "TJ" => {
            // TJ takes one array of (string|number); numbers shift
            // glyphs but for text-only extraction we concatenate the
            // strings.
            if let Some(Object::Array(items)) = op.operands.first() {
                let mut text = String::new();
                for it in items {
                    if let Some(s) = decode_string(it) {
                        text.push_str(&s);
                    }
                }
                push_run(text, state, frags);
            }
        }
        _ => {}
    }
}

fn emit(string: Option<&Object>, state: &State, frags: &mut Vec<TextFragment>) {
    let Some(obj) = string else { return };
    let Some(text) = decode_string(obj) else {
        return;
    };
    push_run(text, state, frags);
}

/// Record one decoded run as a [`TextFragment`] at the current text
/// position. Drops the run when text positioning is inactive (no open
/// `BT`/`ET` block) or when the decoded text is empty.
fn push_run(text: String, state: &State, frags: &mut Vec<TextFragment>) {
    if !state.in_text || text.is_empty() {
        return;
    }
    frags.push(TextFragment {
        x: state.tm[4],
        y: state.tm[5],
        font_size: state.font_size,
        text,
    });
}

fn decode_string(obj: &Object) -> Option<String> {
    let bytes = match obj {
        Object::String(b, _) => b.as_slice(),
        _ => return None,
    };
    // Most invoice PDFs from common renderers ship strings in
    // PDFDocEncoding / WinAnsiEncoding which align with Latin-1 for
    // the printable range. Try UTF-8 first (some renderers emit
    // BOM + UTF-16BE; we strip that special-case), fall back to a
    // best-effort Latin-1 mapping that's harmless for ASCII payloads.
    if bytes.starts_with(&[0xFE, 0xFF]) {
        // Decode UTF-16BE code units, combining surrogate pairs. A high
        // surrogate (U+D800..=U+DBFF) followed by a low surrogate
        // (U+DC00..=U+DFFF) encodes a single astral scalar value
        // (U+10000..=U+10FFFF). Decoding each 16-bit unit on its own
        // would drop both halves (surrogates are not scalar values),
        // losing emoji and other supplementary-plane characters.
        let units = (bytes.len() - 2) / 2;
        let mut utf16 = Vec::with_capacity(units);
        let mut i = 2;
        while i + 1 < bytes.len() {
            utf16.push((u16::from(bytes[i]) << 8) | u16::from(bytes[i + 1]));
            i += 2;
        }
        // Lossy decode keeps the well-formed prefix/suffix around any
        // unpaired surrogate (replaced with U+FFFD) instead of bailing
        // on the whole run.
        return Some(String::from_utf16_lossy(&utf16));
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Some(s.to_owned());
    }
    Some(bytes.iter().map(|b| char::from(*b)).collect())
}

/// Extract the first two operands of `op` as numbers. Returns `None`
/// when either operand is missing or not a numeric object. Used by the
/// `Td`/`TD` text-positioning operators, which both take an `(tx, ty)`
/// displacement pair.
fn operand_pair(op: &Operation) -> Option<(f32, f32)> {
    let tx = op.operands.first().and_then(as_number)?;
    let ty = op.operands.get(1).and_then(as_number)?;
    Some((tx, ty))
}

#[allow(clippy::cast_precision_loss)]
fn as_number(obj: &Object) -> Option<f32> {
    match obj {
        Object::Integer(i) => Some(*i as f32),
        Object::Real(r) => Some(*r),
        _ => None,
    }
}

#[inline]
fn matmul(a: &[f32; 6], b: &[f32; 6]) -> [f32; 6] {
    // Both matrices in 3x3 form with the bottom row [0,0,1]:
    // [a0 a1 0]   [b0 b1 0]
    // [a2 a3 0] x [b2 b3 0]
    // [a4 a5 1]   [b4 b5 1]
    [
        a[0].mul_add(b[0], a[1] * b[2]),
        a[0].mul_add(b[1], a[1] * b[3]),
        a[2].mul_add(b[0], a[3] * b[2]),
        a[2].mul_add(b[1], a[3] * b[3]),
        a[4].mul_add(b[0], a[5].mul_add(b[2], b[4])),
        a[4].mul_add(b[1], a[5].mul_add(b[3], b[5])),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fmt::Write as _;

    fn make_pdf(content: &str) -> Vec<u8> {
        // Hand-assemble a minimal PDF with one page and a content
        // stream we control. Avoids pulling in a PDF-writer dep just
        // for tests.
        let stream = content.as_bytes();
        let len = stream.len();
        let mut pdf = String::new();
        pdf.push_str("%PDF-1.4\n");
        let mut offsets = vec![0usize];
        let body_start = pdf.len();

        let push_obj = |pdf: &mut String, offsets: &mut Vec<usize>, body: &str| {
            offsets.push(pdf.len() - body_start);
            pdf.push_str(body);
        };

        push_obj(
            &mut pdf,
            &mut offsets,
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        );
        push_obj(
            &mut pdf,
            &mut offsets,
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        );
        push_obj(
            &mut pdf,
            &mut offsets,
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        );
        push_obj(
            &mut pdf,
            &mut offsets,
            "4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        );
        let stream_obj =
            format!("5 0 obj\n<< /Length {len} >>\nstream\n{content}endstream\nendobj\n");
        push_obj(&mut pdf, &mut offsets, &stream_obj);

        let xref_offset = pdf.len();
        let _ = writeln!(pdf, "xref\n0 {n}", n = offsets.len());
        pdf.push_str("0000000000 65535 f \n");
        for off in offsets.iter().skip(1) {
            let _ = writeln!(pdf, "{:010} 00000 n ", *off + body_start);
        }
        let _ = writeln!(
            pdf,
            "trailer\n<< /Size {n} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF",
            n = offsets.len()
        );
        pdf.into_bytes()
    }

    #[test]
    fn extracts_simple_text_at_known_position() {
        let bytes = make_pdf("BT\n/F1 12 Tf\n72 720 Td\n(Hello, world) Tj\nET\n");
        let st = extract_pdf_text(&bytes).expect("digital PDF should parse");
        assert_eq!(st.pages.len(), 1);
        let page = &st.pages[0];
        assert_eq!(page.index, 0);
        assert!((page.width_pt - 612.0).abs() < 0.1);
        assert!((page.height_pt - 792.0).abs() < 0.1);
        assert_eq!(page.fragments.len(), 1);
        let f = &page.fragments[0];
        assert!((f.x - 72.0).abs() < 0.1, "x was {}", f.x);
        assert!((f.y - 720.0).abs() < 0.1, "y was {}", f.y);
        assert!((f.font_size - 12.0).abs() < 0.1);
        assert_eq!(f.text, "Hello, world");
    }

    #[test]
    fn preserves_reading_order_top_to_bottom() {
        let bytes = make_pdf(
            "BT\n/F1 12 Tf\n72 100 Td\n(bottom) Tj\nET\nBT\n/F1 12 Tf\n72 700 Td\n(top) Tj\nET\n",
        );
        let st = extract_pdf_text(&bytes).unwrap();
        let texts: Vec<&str> = st.pages[0]
            .fragments
            .iter()
            .map(|f| f.text.as_str())
            .collect();
        assert_eq!(texts, vec!["top", "bottom"]);
    }

    #[test]
    fn preserves_reading_order_left_to_right_within_line() {
        let bytes = make_pdf(
            "BT\n/F1 12 Tf\n300 600 Td\n(right) Tj\nET\nBT\n/F1 12 Tf\n72 600 Td\n(left) Tj\nET\n",
        );
        let st = extract_pdf_text(&bytes).unwrap();
        let texts: Vec<&str> = st.pages[0]
            .fragments
            .iter()
            .map(|f| f.text.as_str())
            .collect();
        assert_eq!(texts, vec!["left", "right"]);
    }

    #[test]
    fn handles_tm_operator() {
        let bytes = make_pdf("BT\n/F1 12 Tf\n1 0 0 1 100 200 Tm\n(matrixed) Tj\nET\n");
        let st = extract_pdf_text(&bytes).unwrap();
        let f = &st.pages[0].fragments[0];
        assert!((f.x - 100.0).abs() < 0.1);
        assert!((f.y - 200.0).abs() < 0.1);
        assert_eq!(f.text, "matrixed");
    }

    #[test]
    fn handles_tj_array_with_kerning_numbers() {
        // TJ array: ["He" -100 "llo"] should concatenate to "Hello".
        let bytes = make_pdf("BT\n/F1 12 Tf\n72 720 Td\n[(He) -100 (llo)] TJ\nET\n");
        let st = extract_pdf_text(&bytes).unwrap();
        assert_eq!(st.pages[0].fragments[0].text, "Hello");
    }

    #[test]
    fn rejects_encrypted_pdf() {
        // Hand-craft a PDF whose trailer carries an Encrypt entry. We
        // do not need a valid encryption dictionary — the entry's
        // mere presence is what T-061 keys on.
        let mut bytes = make_pdf("BT\nET\n");
        let needle = b"/Size";
        if let Some(pos) = bytes.windows(needle.len()).position(|w| w == needle) {
            let injection = b"/Encrypt 99 0 R ";
            bytes.splice(pos..pos, injection.iter().copied());
        }
        let err = extract_pdf_text(&bytes).expect_err("encrypted PDFs must be rejected");
        assert!(matches!(err, PdfTextError::Encrypted));
    }

    #[test]
    fn rejects_non_pdf_bytes() {
        let err = extract_pdf_text(b"not a pdf").expect_err("non-PDF must be rejected");
        assert!(matches!(err, PdfTextError::Parse(_)));
    }

    #[test]
    fn plain_text_round_trips_through_pages() {
        let bytes =
            make_pdf("BT\n/F1 12 Tf\n72 720 Td\n(line one) Tj\n0 -14 Td\n(line two) Tj\nET\n");
        let st = extract_pdf_text(&bytes).unwrap();
        assert_eq!(st.plain_text(), "line one\nline two");
    }

    #[test]
    fn handles_utf16be_bom_strings() {
        // UTF-16BE "Hi" with BOM = FE FF 00 48 00 69.
        let bytes = make_pdf("BT\n/F1 12 Tf\n72 720 Td\n<FEFF00480069> Tj\nET\n");
        let st = extract_pdf_text(&bytes).unwrap();
        assert_eq!(st.pages[0].fragments[0].text, "Hi");
    }

    #[test]
    fn combines_utf16be_surrogate_pairs_for_astral_code_points() {
        // U+1F600 GRINNING FACE encodes in UTF-16BE as the surrogate
        // pair D83D DE00. Decoding each 16-bit unit on its own drops
        // both halves (surrogates are not scalar values); the pair must
        // be combined into the single astral character. BOM = FE FF.
        let bytes = make_pdf("BT\n/F1 12 Tf\n72 720 Td\n<FEFFD83DDE00> Tj\nET\n");
        let st = extract_pdf_text(&bytes).expect("digital PDF should parse");
        assert_eq!(st.pages[0].fragments[0].text, "\u{1F600}");
    }

    #[test]
    fn utf16be_mixes_bmp_and_astral_code_points() {
        // "A" (00 41) + U+1F4B0 MONEY BAG (D83D DCB0) + "Z" (00 5A).
        // The astral glyph between two BMP glyphs must not corrupt
        // the surrounding text. BOM = FE FF.
        let bytes = make_pdf("BT\n/F1 12 Tf\n72 720 Td\n<FEFF0041D83DDCB0005A> Tj\nET\n");
        let st = extract_pdf_text(&bytes).expect("digital PDF should parse");
        assert_eq!(st.pages[0].fragments[0].text, "A\u{1F4B0}Z");
    }

    #[test]
    fn handles_multiple_pages_in_order() {
        // Two pages, each one fragment.
        use lopdf::Dictionary;
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let mut font = Dictionary::new();
        font.set("Type", "Font");
        font.set("Subtype", "Type1");
        font.set("BaseFont", "Helvetica");
        let font_id = doc.add_object(Object::Dictionary(font));

        let make_page = |doc: &mut Document, text: &str| -> ObjectId {
            let content_bytes = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET\n").into_bytes();
            let content_id = doc.add_object(lopdf::Stream::new(Dictionary::new(), content_bytes));
            let mut fonts = Dictionary::new();
            fonts.set("F1", Object::Reference(font_id));
            let mut resources = Dictionary::new();
            resources.set("Font", Object::Dictionary(fonts));
            let mut page = Dictionary::new();
            page.set("Type", "Page");
            page.set("Parent", Object::Reference(pages_id));
            page.set(
                "MediaBox",
                Object::Array(vec![
                    Object::Integer(0),
                    Object::Integer(0),
                    Object::Integer(612),
                    Object::Integer(792),
                ]),
            );
            page.set("Resources", Object::Dictionary(resources));
            page.set("Contents", Object::Reference(content_id));
            doc.add_object(Object::Dictionary(page))
        };
        let p1 = make_page(&mut doc, "page-one");
        let p2 = make_page(&mut doc, "page-two");
        let mut pages = Dictionary::new();
        pages.set("Type", "Pages");
        pages.set(
            "Kids",
            Object::Array(vec![Object::Reference(p1), Object::Reference(p2)]),
        );
        pages.set("Count", Object::Integer(2));
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let mut catalog = Dictionary::new();
        catalog.set("Type", "Catalog");
        catalog.set("Pages", Object::Reference(pages_id));
        let catalog_id = doc.add_object(Object::Dictionary(catalog));
        doc.trailer.set("Root", Object::Reference(catalog_id));

        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();

        let st = extract_pdf_text(&bytes).unwrap();
        assert_eq!(st.pages.len(), 2);
        assert_eq!(st.pages[0].fragments[0].text, "page-one");
        assert_eq!(st.pages[1].fragments[0].text, "page-two");
        assert_eq!(st.pages[0].index, 0);
        assert_eq!(st.pages[1].index, 1);
    }

    /// Build a PDF with `n` empty pages hung off one `Pages` node. Used
    /// to exercise the page-count ceiling without any per-page text
    /// work — the cap is checked from the page list alone.
    fn build_pdf_with_n_pages(n: usize) -> Vec<u8> {
        use lopdf::Dictionary;
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let mut kids = Vec::with_capacity(n);
        for _ in 0..n {
            let mut page = Dictionary::new();
            page.set("Type", "Page");
            page.set("Parent", Object::Reference(pages_id));
            page.set(
                "MediaBox",
                Object::Array(vec![
                    Object::Integer(0),
                    Object::Integer(0),
                    Object::Integer(612),
                    Object::Integer(792),
                ]),
            );
            kids.push(Object::Reference(doc.add_object(Object::Dictionary(page))));
        }
        let mut pages = Dictionary::new();
        pages.set("Type", "Pages");
        let count = i64::try_from(kids.len()).expect("page count fits i64");
        pages.set("Kids", Object::Array(kids));
        pages.set("Count", Object::Integer(count));
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let mut catalog = Dictionary::new();
        catalog.set("Type", "Catalog");
        catalog.set("Pages", Object::Reference(pages_id));
        let catalog_id = doc.add_object(Object::Dictionary(catalog));
        doc.trailer.set("Root", Object::Reference(catalog_id));
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("serialize many-page pdf");
        bytes
    }

    /// A PDF at exactly the page ceiling still extracts; one page over
    /// is refused with [`PdfTextError::TooLarge`]. Without the cap an
    /// attacker could declare an unbounded page list and force intake
    /// to walk every one.
    #[test]
    fn rejects_pdf_over_page_ceiling() {
        let at_cap = build_pdf_with_n_pages(MAX_PAGES);
        let st = extract_pdf_text(&at_cap).expect("page count at cap is accepted");
        assert_eq!(st.pages.len(), MAX_PAGES);

        let over_cap = build_pdf_with_n_pages(MAX_PAGES + 1);
        let err = extract_pdf_text(&over_cap).expect_err("over-cap page count must be refused");
        assert!(
            matches!(err, PdfTextError::TooLarge { .. }),
            "expected TooLarge, got {err:?}"
        );
    }

    /// Build a one-page PDF whose content stream emits `n` `Tj`
    /// show-text operators, each producing one fragment.
    fn build_pdf_with_n_fragments(n: usize) -> Vec<u8> {
        use lopdf::Dictionary;
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();

        let mut font = Dictionary::new();
        font.set("Type", "Font");
        font.set("Subtype", "Type1");
        font.set("BaseFont", "Helvetica");
        let font_id = doc.add_object(Object::Dictionary(font));

        // BT ... ET with n `(x)Tj` runs. Each `Tj` emits its own
        // fragment (position is irrelevant to the fragment count), so
        // we keep the stream as short as possible to bound test cost.
        let mut content = String::with_capacity(n * 6 + 32);
        content.push_str("BT /F1 12 Tf 72 780 Td\n");
        for _ in 0..n {
            content.push_str("(x)Tj\n");
        }
        content.push_str("ET\n");
        let content_id =
            doc.add_object(lopdf::Stream::new(Dictionary::new(), content.into_bytes()));

        let mut fonts = Dictionary::new();
        fonts.set("F1", Object::Reference(font_id));
        let mut resources = Dictionary::new();
        resources.set("Font", Object::Dictionary(fonts));
        let mut page = Dictionary::new();
        page.set("Type", "Page");
        page.set("Parent", Object::Reference(pages_id));
        page.set(
            "MediaBox",
            Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(612),
                Object::Integer(792),
            ]),
        );
        page.set("Resources", Object::Dictionary(resources));
        page.set("Contents", Object::Reference(content_id));
        let leaf_id = doc.add_object(Object::Dictionary(page));

        let mut pages = Dictionary::new();
        pages.set("Type", "Pages");
        pages.set("Kids", Object::Array(vec![Object::Reference(leaf_id)]));
        pages.set("Count", Object::Integer(1));
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let mut catalog = Dictionary::new();
        catalog.set("Type", "Catalog");
        catalog.set("Pages", Object::Reference(pages_id));
        let catalog_id = doc.add_object(Object::Dictionary(catalog));
        doc.trailer.set("Root", Object::Reference(catalog_id));
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("serialize many-fragment pdf");
        bytes
    }

    /// `FlateDecode`-compress `plain` into a zlib stream the way a PDF
    /// writer would, returning the on-wire compressed bytes.
    fn flate_compress(plain: &[u8]) -> Vec<u8> {
        use std::io::Write as _;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(plain).expect("zlib write");
        encoder.finish().expect("zlib finish")
    }

    /// Build a one-page PDF whose page content stream carries `plain`
    /// `FlateDecode`-compressed. The on-wire bytes are the compressed
    /// form (tiny even when `plain` is enormous — a decompression bomb);
    /// the page's *decompressed* content is `plain`.
    fn build_pdf_with_flate_page_content(plain: &[u8]) -> Vec<u8> {
        use lopdf::Dictionary;
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();

        let mut font = Dictionary::new();
        font.set("Type", "Font");
        font.set("Subtype", "Type1");
        font.set("BaseFont", "Helvetica");
        let font_id = doc.add_object(Object::Dictionary(font));

        let mut content_stream = lopdf::Stream::new(Dictionary::new(), flate_compress(plain));
        // Mark the on-wire bytes as FlateDecode so the extractor decodes
        // them through the size-capped path. `Stream::new` already set
        // `/Length` to the compressed size.
        content_stream
            .dict
            .set("Filter", Object::Name(b"FlateDecode".to_vec()));
        let content_id = doc.add_object(content_stream);

        let mut fonts = Dictionary::new();
        fonts.set("F1", Object::Reference(font_id));
        let mut resources = Dictionary::new();
        resources.set("Font", Object::Dictionary(fonts));
        let mut page = Dictionary::new();
        page.set("Type", "Page");
        page.set("Parent", Object::Reference(pages_id));
        page.set(
            "MediaBox",
            Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                Object::Integer(612),
                Object::Integer(792),
            ]),
        );
        page.set("Resources", Object::Dictionary(resources));
        page.set("Contents", Object::Reference(content_id));
        let leaf_id = doc.add_object(Object::Dictionary(page));

        let mut pages = Dictionary::new();
        pages.set("Type", "Pages");
        pages.set("Kids", Object::Array(vec![Object::Reference(leaf_id)]));
        pages.set("Count", Object::Integer(1));
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let mut catalog = Dictionary::new();
        catalog.set("Type", "Catalog");
        catalog.set("Pages", Object::Reference(pages_id));
        let catalog_id = doc.add_object(Object::Dictionary(catalog));
        doc.trailer.set("Root", Object::Reference(catalog_id));
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("serialize flate-content pdf");
        bytes
    }

    /// Regression for the page-content decompression bomb: a hostile PDF
    /// names a page content stream whose `FlateDecode` input is a few
    /// kilobytes but inflates to far more than the
    /// [`MAX_PAGE_CONTENT_SIZE`] ceiling.
    ///
    /// Before the fix, `extract_page` called lopdf's
    /// `Document::get_page_content`, which runs the unbounded
    /// `Stream::decompressed_content()` on every page content stream and
    /// materialises the whole bomb. With the size-capped walk in place
    /// the over-cap page content is refused and extraction surfaces a
    /// typed [`PdfTextError::Page`] error instead of exhausting memory.
    #[test]
    fn rejects_page_content_decompression_bomb() {
        // 256 MiB of a benign content operator repeated compresses to a
        // few kilobytes of DEFLATE but inflates well past the 64 MiB
        // page-content ceiling.
        let bomb = b" 0 0 0 0 0 0 cm\n".repeat(256 * 1024 * 1024 / 15);
        let pdf = build_pdf_with_flate_page_content(&bomb);
        // The on-wire PDF stays tiny — proof the danger is the
        // *decompressed* size, not the input size.
        assert!(
            pdf.len() < 1024 * 1024,
            "compressed bomb PDF should be small, got {} bytes",
            pdf.len()
        );
        let err = extract_pdf_text(&pdf)
            .expect_err("over-cap page content must be refused, not materialised");
        assert!(
            matches!(err, PdfTextError::Page { .. }),
            "expected Page error, got {err:?}"
        );
    }

    /// A normally-sized, `FlateDecode`-compressed page content stream
    /// must still extract through the size-capped path. This proves the
    /// cap is behaviour-preserving for conformant input.
    #[test]
    fn extracts_flate_compressed_page_content() {
        let pdf = build_pdf_with_flate_page_content(
            b"BT /F1 12 Tf 72 720 Td (Compressed page) Tj ET\n",
        );
        let st = extract_pdf_text(&pdf).expect("flate-compressed page extracts");
        assert_eq!(st.pages.len(), 1);
        assert_eq!(st.pages[0].fragments[0].text, "Compressed page");
    }

    /// A single page that emits more than [`MAX_FRAGMENTS_TOTAL`]
    /// show-text operators is refused with [`PdfTextError::TooLarge`]
    /// rather than accumulating an unbounded fragment vector and feeding
    /// it to the per-page reading-order pass. A modestly-sized page is
    /// still extracted normally.
    #[test]
    fn rejects_pdf_over_fragment_ceiling() {
        // Comfortably under the ceiling: extracts every fragment.
        let small = build_pdf_with_n_fragments(1000);
        let st = extract_pdf_text(&small).expect("small fragment count is accepted");
        assert_eq!(st.pages[0].fragments.len(), 1000);

        // Just over the ceiling: refused.
        let over = build_pdf_with_n_fragments(MAX_FRAGMENTS_TOTAL + 1);
        let err = extract_pdf_text(&over).expect_err("over-cap fragment count must be refused");
        assert!(
            matches!(err, PdfTextError::TooLarge { .. }),
            "expected TooLarge, got {err:?}"
        );
    }
}
