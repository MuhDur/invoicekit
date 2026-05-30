// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! Color palette for the default HTML5 invoice template.
//!
//! The WCAG 2.1 AA gate is "contrast ratio ≥ 4.5:1 for normal text
//! and ≥ 3:1 for large text" ([WCAG SC
//! 1.4.3](https://www.w3.org/TR/WCAG21/#contrast-minimum)). The
//! palette below is hand-tuned to clear both thresholds; the
//! [`contrast_ratio`] function lets a downstream caller verify
//! candidate palette swaps before swapping the constants.

/// Foreground text on a light background.
pub const FG_TEXT: &str = "#1a1a1a";

/// Page background.
pub const BG_PAGE: &str = "#ffffff";

/// Subdued text (legends, captions, payment terms).
pub const FG_MUTED: &str = "#4a4a4a";

/// Primary accent (header band; reverse text overlay).
pub const ACCENT: &str = "#0a4d8c";

/// Foreground text used over the [`ACCENT`] background.
pub const ACCENT_FG: &str = "#ffffff";

/// Light separator color (borders + table strokes).
pub const BORDER: &str = "#b8c2cc";

/// Compute the WCAG 2.1 contrast ratio between two hex sRGB colors.
///
/// Returns a value between 1.0 (no contrast) and 21.0 (black on
/// white). 4.5 is the minimum for normal text under WCAG 2.1 AA;
/// 7.0 is the AAA threshold.
///
/// # Errors
///
/// Returns `Err` when either input is not a `#RRGGBB` hex triplet.
pub fn contrast_ratio(a: &str, b: &str) -> Result<f64, &'static str> {
    let la = relative_luminance(parse_hex(a)?);
    let lb = relative_luminance(parse_hex(b)?);
    let (light, dark) = if la > lb { (la, lb) } else { (lb, la) };
    Ok((light + 0.05) / (dark + 0.05))
}

fn parse_hex(s: &str) -> Result<(u8, u8, u8), &'static str> {
    let bytes = s.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' {
        return Err("color must be #RRGGBB");
    }
    // Index the raw bytes rather than slicing the `&str`: a 7-byte input
    // may contain a multibyte UTF-8 char, and slicing on a non-char
    // boundary would panic. Each `hex_pair` validates ASCII hex digits.
    let r = hex_pair(bytes[1], bytes[2]).ok_or("bad red component")?;
    let g = hex_pair(bytes[3], bytes[4]).ok_or("bad green component")?;
    let b = hex_pair(bytes[5], bytes[6]).ok_or("bad blue component")?;
    Ok((r, g, b))
}

/// Parse two ASCII hex-digit bytes into one octet. Returns `None` for any
/// non-hex byte (including the continuation bytes of a multibyte UTF-8 char).
fn hex_pair(hi: u8, lo: u8) -> Option<u8> {
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    Some(nibble(hi)? << 4 | nibble(lo)?)
}

fn relative_luminance((r, g, b): (u8, u8, u8)) -> f64 {
    fn channel(c: u8) -> f64 {
        let v = f64::from(c) / 255.0;
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_on_white_clears_wcag_aaa() {
        let ratio = contrast_ratio("#000000", "#ffffff").unwrap();
        assert!(ratio >= 7.0, "ratio was {ratio}");
    }

    #[test]
    fn default_text_palette_clears_wcag_aa() {
        let ratio = contrast_ratio(FG_TEXT, BG_PAGE).unwrap();
        assert!(ratio >= 4.5, "FG_TEXT on BG_PAGE was {ratio}");
    }

    #[test]
    fn default_muted_palette_clears_wcag_aa() {
        // Muted text is allowed to be lighter but must still hit AA.
        let ratio = contrast_ratio(FG_MUTED, BG_PAGE).unwrap();
        assert!(ratio >= 4.5, "FG_MUTED on BG_PAGE was {ratio}");
    }

    #[test]
    fn accent_band_text_clears_wcag_aa() {
        let ratio = contrast_ratio(ACCENT_FG, ACCENT).unwrap();
        assert!(ratio >= 4.5, "ACCENT_FG on ACCENT was {ratio}");
    }

    #[test]
    fn parse_hex_rejects_bad_inputs() {
        assert!(contrast_ratio("nope", "#fff").is_err());
        assert!(contrast_ratio("#fff", "#fff").is_err());
        assert!(contrast_ratio("#ZZZZZZ", "#ffffff").is_err());
    }

    #[test]
    fn parse_hex_rejects_multibyte_without_panicking() {
        // "#a" + "é" + "ZZZ" is exactly 7 bytes (1 + 1 + 2 + 3) and starts
        // with '#', so the length/prefix check passes. Here 'é' spans bytes
        // 2..4, so the old `&s[1..3]` slice ends at byte 3 — mid-'é', not a
        // char boundary — which used to panic; it must now error instead.
        let input = "#a\u{00e9}ZZZ";
        assert_eq!(input.len(), 7);
        assert!(
            !input.is_char_boundary(3),
            "input must straddle the 1..3 slice cut"
        );
        assert!(parse_hex(input).is_err());
        assert!(contrast_ratio(input, "#ffffff").is_err());
    }

    #[test]
    fn parse_hex_round_trips_valid_components() {
        assert_eq!(parse_hex("#0a4d8c"), Ok((0x0a, 0x4d, 0x8c)));
        assert_eq!(parse_hex("#FFFFFF"), Ok((0xff, 0xff, 0xff)));
        assert_eq!(parse_hex("#000000"), Ok((0x00, 0x00, 0x00)));
    }
}
