// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! Script-aware reading-order reconstruction for the digital-PDF
//! intake path.
//!
//! The bare [`crate::text`] extractor emits one fragment per
//! show-text operator at its `(x, y)` origin and orders fragments
//! top-to-bottom then strictly left-to-right. That is correct for
//! left-to-right Latin invoices but wrong for two writing systems
//! that appear on real-world invoices:
//!
//! * **Right-to-left scripts** (Arabic, Hebrew, and the like). A
//!   producer lays an RTL line out so the *visually leftmost* run
//!   has the smallest `x`, but the run that should be read *first*
//!   is the visually rightmost one. Sorting purely by ascending `x`
//!   therefore reverses the run order on every RTL line.
//! * **CJK vertical writing mode**. A producer that writes vertical
//!   Chinese/Japanese/Korean emits each glyph (or short run) as its
//!   own fragment stepping *downward* in a column, with successive
//!   columns marching right-to-left across the page. Sorting
//!   top-to-bottom then left-to-right scrambles that into nonsense.
//!
//! This module post-processes the flat fragment list per page and
//! rebuilds reading order for those two cases while leaving ordinary
//! left-to-right pages untouched.
//!
//! # What is and is not handled
//!
//! Handled:
//!
//! * Detection of right-to-left lines via the Unicode Bidirectional
//!   Algorithm (the `unicode-bidi` crate, the same engine browsers
//!   and `rustybuzz`/`cosmic-text` use) applied to each visual line's
//!   text.
//! * Reordering of the runs on a right-to-left line into logical
//!   (reading) order, with the base direction taken from the line's
//!   strong characters, so a line of Arabic with an embedded Latin
//!   purchase-order number or a Western number comes out in the
//!   order a human reads it.
//! * Detection of CJK vertical columns by clustering fragments into
//!   near-constant-`x` columns of stacked CJK glyphs and emitting
//!   columns right-to-left, each column top-to-bottom.
//!
//! Not handled (honest bounds):
//!
//! * We assume each individual show-text run already stores its
//!   glyphs in **logical** order within the run. This is what every
//!   mainstream producer does (LaTeX `bidi`/`polyglossia`, Typst,
//!   headless-browser/`wkhtmltopdf`, `LibreOffice`, Word/Apache POI):
//!   the bidi reordering for display happens at render time, not in
//!   the stored string. A producer that bakes *visual*-order glyphs
//!   into each run (rare, and indistinguishable from logical order
//!   without per-glyph advances) is not reconstructed; that case
//!   still routes through the OCR / vision fallback.
//! * Right-to-left reconstruction works at run granularity: the runs
//!   on an RTL line are placed into logical order, and each run's own
//!   logically-stored text is kept verbatim. A *multi-run* embedded
//!   left-to-right phrase on an RTL line (e.g. a two-word English
//!   product name a producer happened to split into two runs) has its
//!   runs reversed along with the line; the words land in the right
//!   place on the line but in reversed order among themselves. Single
//!   embedded LTR tokens (the common case: an order number, a SKU, a
//!   date) are unaffected.
//! * Mirrored bracket glyphs (U+0028 vs. its mirror) are left as the
//!   producer stored them; we reorder, we do not substitute glyphs.
//! * Mixed vertical + horizontal text in a single line/column is
//!   classified by majority; a genuinely interleaved layout falls
//!   back to the horizontal path.

use unicode_bidi::{bidi_class, BidiClass};

use crate::text::TextFragment;

/// Re-order one page's fragments into reading order, picking the
/// strategy that fits the page's dominant writing system.
///
/// The flow is:
///
/// 1. If the page is dominated by stacked CJK columns, reconstruct
///    vertical reading order (columns right-to-left, each column
///    top-to-bottom).
/// 2. Otherwise group fragments into visual lines, and for each line
///    decide left-to-right or right-to-left from its own text. LTR
///    lines keep ascending-`x` order; RTL lines are rebuilt into
///    logical order.
///
/// The returned vector is the page's fragments in reading order.
// `pub(crate)` is what we want (sibling `text` module is the only
// caller); `unreachable_pub` forbids a bare `pub` in a private module,
// while `redundant_pub_crate` would prefer `pub`. The two lints pull
// opposite ways, so silence the latter for this one item.
#[allow(clippy::redundant_pub_crate)]
#[must_use]
pub(crate) fn reading_order(frags: Vec<TextFragment>) -> Vec<TextFragment> {
    if frags.len() < 2 {
        return frags;
    }
    if looks_like_cjk_vertical(&frags) {
        return order_cjk_vertical(frags);
    }
    order_horizontal(frags)
}

/// Group fragments into visual lines (top-to-bottom) and order each
/// line by its own base direction.
fn order_horizontal(frags: Vec<TextFragment>) -> Vec<TextFragment> {
    let lines = group_into_lines(frags);
    let mut out = Vec::new();
    for mut line in lines {
        // Within a line the extractor gives us runs in arbitrary
        // emission order; establish visual order (left-to-right by
        // x) first so direction decisions are about geometry, not
        // content-stream happenstance.
        line.sort_by(total_cmp_x);
        if line_is_rtl(&line) {
            // RTL line: each run stores its glyphs in logical order,
            // and the visually-rightmost run reads first, so logical
            // run order is right-to-left, i.e. descending x.
            line.reverse();
        }
        out.extend(line);
    }
    out
}

/// Split fragments into visual lines, largest `y` (top of page)
/// first. Two fragments share a line when their baselines sit within
/// half the larger font height of each other.
fn group_into_lines(mut frags: Vec<TextFragment>) -> Vec<Vec<TextFragment>> {
    // Sort top-to-bottom so we can sweep lines in reading order.
    frags.sort_by(total_cmp_y_desc);
    let mut lines: Vec<Vec<TextFragment>> = Vec::new();
    for frag in frags {
        match lines.last_mut() {
            Some(line) if same_line(line, &frag) => line.push(frag),
            _ => lines.push(vec![frag]),
        }
    }
    lines
}

/// True when `frag` belongs on the same visual line as the fragments
/// already collected in `line` (compared against the line's first,
/// i.e. topmost, member).
fn same_line(line: &[TextFragment], frag: &TextFragment) -> bool {
    let Some(anchor) = line.first() else {
        return false;
    };
    let tol = 0.5 * anchor.font_size.max(frag.font_size).max(8.0);
    (anchor.y - frag.y).abs() <= tol
}

/// Decide whether a visual line's base direction is right-to-left.
///
/// We classify every character with its Unicode Bidirectional
/// Algorithm strong class (UAX #9): `L` is strong left-to-right,
/// `R` and `AL` are strong right-to-left. The line's base direction
/// is right-to-left when it carries at least one strong RTL character
/// and the strong RTL characters are not outnumbered by strong LTR
/// ones.
///
/// Why dominant-class rather than the standard "first strong
/// character" rule (`P2`/`P3`)? Because we feed the algorithm the
/// runs in *visual* (left-to-right) order, so the first strong
/// character belongs to the visually-leftmost — i.e. logically
/// *last* — run. On a mostly-Arabic line that opens with an embedded
/// Latin token (an order number, a SKU), first-strong would
/// mis-resolve the whole line to left-to-right. Counting strong
/// classes keeps the decision about the line's language, not about
/// which token a producer happened to place on the left.
///
/// A purely neutral line (digits, punctuation, whitespace; no strong
/// characters) has no RTL strong character and stays left-to-right.
fn line_is_rtl(line: &[TextFragment]) -> bool {
    let mut rtl = 0usize;
    let mut ltr = 0usize;
    for frag in line {
        for ch in frag.text.chars() {
            match bidi_class(ch) {
                BidiClass::R | BidiClass::AL => rtl += 1,
                BidiClass::L => ltr += 1,
                _ => {}
            }
        }
    }
    rtl > 0 && rtl >= ltr
}

/// Heuristic: does this page read as CJK vertical text?
///
/// Two conditions must hold:
///
/// 1. A clear majority of the non-space character mass is CJK
///    ideographs/kana/hangul (so we never transpose a Latin page).
/// 2. The fragments genuinely *stack* into vertical columns rather
///    than spread across a horizontal line. We cluster fragments by
///    `x` and require that the typical column is deep — has two or
///    more glyphs at distinct `y`. A single horizontal row of CJK
///    glyphs produces many one-glyph columns and is *not* vertical;
///    a vertical column produces one `x` with many stacked glyphs.
fn looks_like_cjk_vertical(frags: &[TextFragment]) -> bool {
    let mut cjk = 0usize;
    let mut total = 0usize;
    for frag in frags {
        for ch in frag.text.chars() {
            if !ch.is_whitespace() {
                total += 1;
                if is_cjk(ch) {
                    cjk += 1;
                }
            }
        }
    }
    if total == 0 || cjk * 5 < total * 4 {
        // Fewer than ~80% CJK glyphs: not a vertical CJK page.
        return false;
    }
    // Cluster into columns and require real vertical depth. A column
    // is "deep" when it stacks two or more fragments at distinct y.
    // Horizontal CJK (one row) yields columns of depth 1 only.
    let columns = cluster_columns(frags.to_vec());
    let deep = columns
        .iter()
        .filter(|col| distinct_buckets(col.iter().map(|f| f.y)) >= 2)
        .count();
    // At least one genuinely-stacked column, and stacked columns are
    // not a rounding-error minority of the columns we found.
    deep >= 1 && deep * 2 >= columns.len()
}

/// Reconstruct vertical CJK reading order: columns right-to-left,
/// each column read top-to-bottom.
fn order_cjk_vertical(frags: Vec<TextFragment>) -> Vec<TextFragment> {
    let mut columns = cluster_columns(frags);
    // Columns read right-to-left: the rightmost column (largest x)
    // comes first.
    columns.sort_by(|a, b| {
        let ax = a.first().map_or(0.0, |f| f.x);
        let bx = b.first().map_or(0.0, |f| f.x);
        bx.partial_cmp(&ax).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut out = Vec::new();
    for mut col in columns {
        // Within a column, read top (large y) to bottom (small y).
        col.sort_by(total_cmp_y_desc);
        out.extend(col);
    }
    out
}

/// Cluster fragments into vertical columns by `x` proximity. Two
/// fragments share a column when their `x` origins are within a
/// glyph-width tolerance.
fn cluster_columns(mut frags: Vec<TextFragment>) -> Vec<Vec<TextFragment>> {
    frags.sort_by(total_cmp_x);
    let mut columns: Vec<Vec<TextFragment>> = Vec::new();
    for frag in frags {
        let tol = 0.6 * frag.font_size.max(8.0);
        match columns.last_mut() {
            Some(col)
                if col
                    .first()
                    .is_some_and(|anchor| (anchor.x - frag.x).abs() <= tol) =>
            {
                col.push(frag);
            }
            _ => columns.push(vec![frag]),
        }
    }
    columns
}

/// Count how many distinct buckets a coordinate stream falls into,
/// where values within 4 user-space units collapse to one bucket.
/// Cheap stand-in for a clustering pass; good enough to compare the
/// column-count against the row-count.
fn distinct_buckets(values: impl Iterator<Item = f32>) -> usize {
    let mut seen: Vec<f32> = Vec::new();
    for v in values {
        if !seen.iter().any(|s| (s - v).abs() <= 4.0) {
            seen.push(v);
        }
    }
    seen.len()
}

fn total_cmp_x(a: &TextFragment, b: &TextFragment) -> std::cmp::Ordering {
    a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal)
}

/// Descending comparator on `y`: larger `y` (higher on the page,
/// since the PDF origin is bottom-left) sorts first, so a sort with
/// this comparator yields top-to-bottom order.
fn total_cmp_y_desc(a: &TextFragment, b: &TextFragment) -> std::cmp::Ordering {
    b.y.partial_cmp(&a.y).unwrap_or(std::cmp::Ordering::Equal)
}

/// True for the CJK ranges that are written vertically on invoices:
/// CJK Unified Ideographs (incl. Extension A), Hiragana, Katakana,
/// Hangul syllables, and the CJK fullwidth/symbol blocks. Latin and
/// other scripts return false.
fn is_cjk(ch: char) -> bool {
    matches!(ch as u32,
        0x3040..=0x30FF      // Hiragana + Katakana
        | 0x3400..=0x4DBF    // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF    // CJK Unified Ideographs
        | 0xF900..=0xFAFF    // CJK Compatibility Ideographs
        | 0xAC00..=0xD7A3    // Hangul syllables
        | 0xFF00..=0xFFEF    // Halfwidth/Fullwidth forms
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frag(x: f32, y: f32, text: &str) -> TextFragment {
        TextFragment {
            x,
            y,
            font_size: 12.0,
            text: text.to_owned(),
        }
    }

    fn joined(frags: &[TextFragment]) -> String {
        frags
            .iter()
            .map(|f| f.text.as_str())
            .collect::<Vec<_>>()
            .join("|")
    }

    #[test]
    fn ltr_line_keeps_left_to_right_order() {
        // "Invoice" then "INV-001" laid out left-to-right.
        let frags = vec![frag(300.0, 700.0, "INV-001"), frag(72.0, 700.0, "Invoice")];
        let out = reading_order(frags);
        assert_eq!(joined(&out), "Invoice|INV-001");
    }

    #[test]
    fn ltr_multiline_keeps_top_to_bottom() {
        let frags = vec![frag(72.0, 100.0, "bottom"), frag(72.0, 700.0, "top")];
        let out = reading_order(frags);
        assert_eq!(joined(&out), "top|bottom");
    }

    #[test]
    fn hebrew_line_reconstructs_right_to_left() {
        // Hebrew word "שלום" (shalom) split across two runs by the
        // producer. Visually the run with the larger x sits on the
        // right and reads first. Logical order is right-run then
        // left-run.
        let right = frag(300.0, 700.0, "של"); // reads first
        let left = frag(72.0, 700.0, "ום"); // reads second
        let out = reading_order(vec![left, right]);
        assert_eq!(joined(&out), "של|ום");
    }

    #[test]
    fn arabic_line_reconstructs_right_to_left() {
        // Arabic "فاتورة رقم" (invoice number) as two runs. The
        // rightmost run reads first.
        let right = frag(320.0, 700.0, "فاتورة"); // "invoice"
        let left = frag(72.0, 700.0, "رقم"); // "number"
        let out = reading_order(vec![left, right]);
        assert_eq!(joined(&out), "فاتورة|رقم");
    }

    #[test]
    fn arabic_line_with_embedded_latin_number_orders_logically() {
        // "فاتورة" (right) ... "INV-7" (left, an embedded LTR token).
        // On an RTL line the Arabic reads first, then the number.
        let arabic = frag(320.0, 700.0, "فاتورة");
        let number = frag(72.0, 700.0, "INV-7");
        let out = reading_order(vec![number, arabic]);
        assert_eq!(joined(&out), "فاتورة|INV-7");
    }

    #[test]
    fn cjk_vertical_reconstructs_columns_right_to_left() {
        // Two vertical columns of Japanese. The producer steps each
        // glyph downward (decreasing y) and lays the second column to
        // the LEFT of the first. Reading order: right column top-to-
        // bottom, then left column top-to-bottom.
        //
        // Right column (x=300): 請 (top) 求 (bottom)
        // Left column  (x=270): 書 (top) 類 (bottom)
        let frags = vec![
            frag(270.0, 700.0, "書"),
            frag(300.0, 680.0, "求"),
            frag(270.0, 680.0, "類"),
            frag(300.0, 700.0, "請"),
        ];
        let out = reading_order(frags);
        assert_eq!(joined(&out), "請|求|書|類");
    }

    #[test]
    fn cjk_vertical_single_column_reads_top_to_bottom() {
        let frags = vec![
            frag(300.0, 660.0, "三"),
            frag(300.0, 700.0, "一"),
            frag(300.0, 680.0, "二"),
        ];
        let out = reading_order(frags);
        // A lone column is still a column; top-to-bottom.
        assert_eq!(joined(&out), "一|二|三");
    }

    #[test]
    fn horizontal_cjk_is_not_treated_as_vertical() {
        // CJK glyphs laid out horizontally (one row, increasing x)
        // must stay in left-to-right order, not be transposed.
        let frags = vec![
            frag(72.0, 700.0, "発"),
            frag(96.0, 700.0, "行"),
            frag(120.0, 700.0, "日"),
        ];
        let out = reading_order(frags);
        assert_eq!(joined(&out), "発|行|日");
    }

    #[test]
    fn mixed_latin_page_with_one_rtl_line() {
        // A Latin header line and an Arabic line below it: each line
        // gets its own direction.
        let frags = vec![
            frag(72.0, 700.0, "Invoice"),
            frag(300.0, 700.0, "2026"),
            frag(320.0, 680.0, "فاتورة"),
            frag(72.0, 680.0, "رقم"),
        ];
        let out = reading_order(frags);
        assert_eq!(joined(&out), "Invoice|2026|فاتورة|رقم");
    }
}
