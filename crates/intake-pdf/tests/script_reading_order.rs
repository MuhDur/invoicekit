// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! End-to-end tests for script-aware reading-order reconstruction.
//!
//! These drive *real* PDFs through the public [`extract_pdf_text`]
//! API rather than poking the internal ordering helper. Each test
//! hand-builds a one-page PDF whose content stream places non-Latin
//! text runs at controlled positions, then asserts the extracted
//! plain text comes out in human reading order.
//!
//! Strings are emitted as UTF-16BE hex literals (`<FEFF…>`) so the
//! Arabic, Hebrew, and CJK code points survive the content stream
//! unchanged — the extractor decodes the byte-order mark and reads
//! them back as Unicode.

use std::fmt::Write as _;

use invoicekit_intake_pdf::extract_pdf_text;

/// One positioned text run: place the text at `(x, y)` via a `Tm`
/// operator, then show it with `Tj`. `text` is encoded UTF-16BE with
/// a leading byte-order mark so any script round-trips.
struct Run {
    x: i32,
    y: i32,
    text: &'static str,
}

fn utf16be_hex(text: &str) -> String {
    // Byte-order mark, then each UTF-16 code unit big-endian.
    let mut hex = String::from("FEFF");
    for unit in text.encode_utf16() {
        let _ = write!(hex, "{unit:04X}");
    }
    hex
}

/// Build a minimal single-page PDF whose content stream shows each
/// run at its position. Mirrors the in-crate `make_pdf` test helper
/// but takes positioned, UTF-16BE-encoded runs.
fn build_pdf(runs: &[Run]) -> Vec<u8> {
    let mut content = String::from("BT\n/F1 12 Tf\n");
    for run in runs {
        let _ = write!(
            content,
            "1 0 0 1 {x} {y} Tm\n<{hex}> Tj\n",
            x = run.x,
            y = run.y,
            hex = utf16be_hex(run.text),
        );
    }
    content.push_str("ET\n");

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
    let stream_obj = format!("5 0 obj\n<< /Length {len} >>\nstream\n{content}endstream\nendobj\n");
    push_obj(&mut pdf, &mut offsets, &stream_obj);

    let xref_offset = pdf.len();
    let _ = writeln!(pdf, "xref\n0 {n}", n = offsets.len());
    pdf.push_str("0000000000 65535 f \n");
    for off in offsets.iter().skip(1) {
        let _ = writeln!(pdf, "{:010} 00000 n ", *off + body_start);
    }
    let _ = write!(
        pdf,
        "trailer\n<< /Size {n} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF",
        n = offsets.len()
    );
    pdf.into_bytes()
}

fn plain_text(runs: &[Run]) -> String {
    let bytes = build_pdf(runs);
    let st = extract_pdf_text(&bytes).expect("hand-built PDF should parse");
    st.plain_text()
}

#[test]
fn ltr_invoice_unchanged_end_to_end() {
    // Two-line Latin invoice; ordinary behaviour must be preserved.
    let runs = [
        Run {
            x: 72,
            y: 700,
            text: "Invoice",
        },
        Run {
            x: 300,
            y: 700,
            text: "INV-2026-1",
        },
        Run {
            x: 72,
            y: 680,
            text: "Total",
        },
    ];
    assert_eq!(plain_text(&runs), "Invoice\nINV-2026-1\nTotal");
}

#[test]
fn hebrew_line_logical_order_end_to_end() {
    // Hebrew "חשבונית מס" (tax invoice) as two runs on one line. The
    // visually-rightmost run reads first.
    let runs = [
        Run {
            x: 72,
            y: 700,
            text: "מס",
        }, // reads second
        Run {
            x: 320,
            y: 700,
            text: "חשבונית",
        }, // reads first
    ];
    assert_eq!(plain_text(&runs), "חשבונית\nמס");
}

#[test]
fn arabic_line_logical_order_end_to_end() {
    // Arabic "فاتورة ضريبية" (tax invoice) as two runs.
    let runs = [
        Run {
            x: 72,
            y: 700,
            text: "ضريبية",
        }, // reads second
        Run {
            x: 320,
            y: 700,
            text: "فاتورة",
        }, // reads first
    ];
    assert_eq!(plain_text(&runs), "فاتورة\nضريبية");
}

#[test]
fn arabic_with_embedded_number_logical_order_end_to_end() {
    // Mostly-Arabic line that places a Latin order number on the
    // left. Reading order: Arabic first, number second.
    let runs = [
        Run {
            x: 72,
            y: 700,
            text: "INV-7",
        },
        Run {
            x: 320,
            y: 700,
            text: "فاتورة",
        },
    ];
    assert_eq!(plain_text(&runs), "فاتورة\nINV-7");
}

#[test]
fn cjk_vertical_reading_order_end_to_end() {
    // Two vertical columns of Japanese. Producer steps glyphs
    // downward; the second column sits to the LEFT of the first.
    // Reading order: right column top-to-bottom, then left column.
    //
    //   right column (x=300): 請 / 求
    //   left  column (x=270): 書 / 類
    let runs = [
        Run {
            x: 270,
            y: 700,
            text: "書",
        },
        Run {
            x: 300,
            y: 680,
            text: "求",
        },
        Run {
            x: 270,
            y: 680,
            text: "類",
        },
        Run {
            x: 300,
            y: 700,
            text: "請",
        },
    ];
    assert_eq!(plain_text(&runs), "請\n求\n書\n類");
}

#[test]
fn horizontal_cjk_stays_left_to_right_end_to_end() {
    // CJK glyphs in a single horizontal row must not be transposed
    // into vertical order.
    let runs = [
        Run {
            x: 72,
            y: 700,
            text: "発",
        },
        Run {
            x: 96,
            y: 700,
            text: "行",
        },
        Run {
            x: 120,
            y: 700,
            text: "日",
        },
    ];
    assert_eq!(plain_text(&runs), "発\n行\n日");
}
