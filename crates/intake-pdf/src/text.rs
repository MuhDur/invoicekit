// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-061 text extractor.
//!
//! Walks every page's content stream, tracks the text-positioning
//! state (current Text matrix, leading, font size), and emits one
//! [`TextFragment`] per `Tj` / `TJ` / `'` / `"` operator. Fragments
//! are then sorted into reading order per page.
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

use crate::PdfTextError;

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
    /// Text fragments sorted top-to-bottom then left-to-right.
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

    let mut pages = Vec::new();
    for (idx, (_page_no, page_id)) in doc.get_pages().into_iter().enumerate() {
        let (width_pt, height_pt) = page_size(&doc, page_id);
        let fragments = extract_page(&doc, page_id)
            .map_err(|detail| PdfTextError::Page { page: idx, detail })?;
        let fragments = sort_reading_order(fragments);
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
    let content = doc.get_page_content(page_id).map_err(|e| e.to_string())?;
    let content = lopdf::content::Content::decode(&content).map_err(|e| e.to_string())?;

    let mut state = State::default();
    let mut frags: Vec<TextFragment> = Vec::new();
    for op in content.operations {
        apply_operator(&op, &mut state, &mut frags);
    }
    Ok(frags)
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
            if let (Some(tx), Some(ty)) = (
                op.operands.first().and_then(as_number),
                op.operands.get(1).and_then(as_number),
            ) {
                state.translate_text(tx, ty);
            }
        }
        "TD" => {
            if let (Some(tx), Some(ty)) = (
                op.operands.first().and_then(as_number),
                op.operands.get(1).and_then(as_number),
            ) {
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
                if !text.is_empty() && state.in_text {
                    frags.push(TextFragment {
                        x: state.tm[4],
                        y: state.tm[5],
                        font_size: state.font_size,
                        text,
                    });
                }
            }
        }
        _ => {}
    }
}

fn emit(string: Option<&Object>, state: &State, frags: &mut Vec<TextFragment>) {
    if !state.in_text {
        return;
    }
    let Some(obj) = string else { return };
    let Some(text) = decode_string(obj) else {
        return;
    };
    if text.is_empty() {
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
        let mut chars = Vec::with_capacity(bytes.len() / 2);
        let mut i = 2;
        while i + 1 < bytes.len() {
            let cp = u32::from(bytes[i]) << 8 | u32::from(bytes[i + 1]);
            if let Some(c) = char::from_u32(cp) {
                chars.push(c);
            }
            i += 2;
        }
        return Some(chars.into_iter().collect());
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Some(s.to_owned());
    }
    Some(bytes.iter().map(|b| char::from(*b)).collect())
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

fn sort_reading_order(mut frags: Vec<TextFragment>) -> Vec<TextFragment> {
    frags.sort_by(|a, b| {
        // Group fragments into approximate lines: any two fragments
        // whose y-coords are within half a font-height belong to the
        // same line and sort left-to-right; otherwise sort top-to-
        // bottom (larger y first).
        let line_tol = 0.5 * a.font_size.max(b.font_size).max(8.0);
        if (a.y - b.y).abs() <= line_tol {
            a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal)
        } else {
            b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal)
        }
    });
    frags
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
}
