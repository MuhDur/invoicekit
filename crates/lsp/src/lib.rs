// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! InvoiceKit **language server** substrate.
//!
//! Captures the Language Server Protocol (LSP) request /
//! response / diagnostic shapes the future `tower-lsp`-glued
//! server emits, so IDE extensions (VS Code, Cursor, Neovim,
//! Helix — see T-112) can integrate against a stable typed
//! surface before the live JSON-RPC server lands.
//!
//! Public surface:
//!
//! * [`InvoiceDocument`] — a parsed invoice JSON document
//!   the IDE has open.
//! * [`Diagnostic`] / [`DiagnosticSeverity`] / [`Position`]
//!   / [`Range`] — minimal LSP-shaped diagnostic types
//!   (subset of `lsp-types`).
//! * [`HoverInfo`] — what the IDE renders on hover over an
//!   invoice field.
//! * [`Capabilities`] — the server's advertised capabilities.
//! * [`parse_document`] — JSON-parse + collect syntactic
//!   diagnostics.
//! * [`hover_at`] — return [`HoverInfo`] for the BT/BG term
//!   at a cursor position.
//!
//! The future server crate wraps this substrate in a
//! `tower-lsp::LanguageServer` impl that delegates to these
//! pure functions; the LSP rituals (initialize / shutdown /
//! exit) become trivial. Substrate is deliberately
//! transport-agnostic so it doubles as a test harness.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// LSP 0-indexed `(line, character)` position.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Position {
    /// Zero-indexed line number.
    pub line: u32,
    /// Zero-indexed character offset within `line`.
    pub character: u32,
}

/// LSP inclusive-start / exclusive-end character range.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Range {
    /// Inclusive start.
    pub start: Position,
    /// Exclusive end.
    pub end: Position,
}

/// Diagnostic severity. Numeric values match LSP §6 spec.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticSeverity {
    /// 1 = error.
    Error,
    /// 2 = warning.
    Warning,
    /// 3 = information.
    Information,
    /// 4 = hint.
    Hint,
}

impl DiagnosticSeverity {
    /// LSP §6 spec numeric encoding.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Error => 1,
            Self::Warning => 2,
            Self::Information => 3,
            Self::Hint => 4,
        }
    }
}

/// Diagnostic the server publishes to the IDE.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Range the diagnostic applies to.
    pub range: Range,
    /// Severity.
    pub severity: DiagnosticSeverity,
    /// Stable identifier (e.g. EN16931 BR-CO-15).
    pub code: String,
    /// Human-readable message.
    pub message: String,
}

/// Hover info the server renders.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HoverInfo {
    /// EN16931 BT / BG term identifier at the cursor.
    pub term: String,
    /// Plain English description of the term.
    pub description: String,
    /// Range the hover applies to (highlighted in the IDE).
    pub range: Range,
}

/// Parsed invoice document the LSP server keeps in memory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InvoiceDocument {
    /// Original text the IDE sent.
    pub text: String,
    /// Whether `text` parses as JSON.
    pub parses: bool,
    /// Diagnostics collected during the parse pass.
    pub diagnostics: Vec<Diagnostic>,
}

/// Capabilities advertised in the LSP `initialize` response.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Whether the server provides hover info.
    pub hover_provider: bool,
    /// Whether the server publishes diagnostics on
    /// `didOpen` / `didChange`.
    pub diagnostic_provider: bool,
    /// Whether the server provides code completion.
    pub completion_provider: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            hover_provider: true,
            diagnostic_provider: true,
            completion_provider: false,
        }
    }
}

/// Typed errors callers care about.
#[derive(Debug, Error)]
pub enum LspError {
    /// Position pointed past the document end.
    #[error("position {0:?} is past the end of the document")]
    PositionOutOfRange(Position),
}

/// Parse JSON text into an [`InvoiceDocument`].
///
/// Successful parse → `parses: true`, empty diagnostics.
/// Failed parse → `parses: false`, single Error diagnostic
/// pinned to the failing line/column.
#[must_use]
pub fn parse_document(text: impl Into<String>) -> InvoiceDocument {
    let text = text.into();
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(_) => InvoiceDocument {
            text,
            parses: true,
            diagnostics: Vec::new(),
        },
        Err(err) => {
            let line = u32::try_from(err.line().saturating_sub(1)).unwrap_or(0);
            let column = u32::try_from(err.column().saturating_sub(1)).unwrap_or(0);
            let pos = Position {
                line,
                character: column,
            };
            let range = Range {
                start: pos,
                end: Position {
                    line: pos.line,
                    character: pos.character.saturating_add(1),
                },
            };
            let diag = Diagnostic {
                range,
                severity: DiagnosticSeverity::Error,
                code: "invoicekit/json-parse".to_owned(),
                message: err.to_string(),
            };
            InvoiceDocument {
                text,
                parses: false,
                diagnostics: vec![diag],
            }
        }
    }
}

/// Return [`HoverInfo`] for the BT/BG term at `position`.
///
/// Today's implementation looks up the JSON key the cursor
/// sits in and returns a plain-English EN16931 term gloss.
/// The real implementation walks the parsed AST against the
/// rulepack — but the substrate's contract is what matters
/// for IDE-side wiring.
///
/// # Errors
///
/// Returns [`LspError::PositionOutOfRange`] when `position`
/// points past the document end.
pub fn hover_at(
    document: &InvoiceDocument,
    position: Position,
) -> Result<Option<HoverInfo>, LspError> {
    let lines: Vec<&str> = document.text.lines().collect();
    let line_idx = position.line as usize;
    if line_idx >= lines.len() {
        return Err(LspError::PositionOutOfRange(position));
    }
    let line = lines[line_idx];
    // Naive lookup: find the JSON key (between `"`s) closest
    // to the cursor column. Production impl walks the AST.
    let chars: Vec<char> = line.chars().collect();
    let col = (position.character as usize).min(chars.len().saturating_sub(1));
    let (start, end) = key_around(&chars, col);
    if start == end {
        return Ok(None);
    }
    let key: String = chars[start..end].iter().collect();
    let gloss = en16931_gloss(&key);
    Ok(gloss.map(|description| HoverInfo {
        term: key,
        description: description.to_owned(),
        range: Range {
            start: Position {
                line: position.line,
                character: u32::try_from(start).unwrap_or(position.character),
            },
            end: Position {
                line: position.line,
                character: u32::try_from(end).unwrap_or(position.character),
            },
        },
    }))
}

fn key_around(chars: &[char], col: usize) -> (usize, usize) {
    if chars.is_empty() {
        return (0, 0);
    }
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let mut start = col;
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    (start, end)
}

fn en16931_gloss(key: &str) -> Option<&'static str> {
    match key {
        "id" | "invoice_number" | "invoiceNumber" => {
            Some("BT-1 — Invoice number assigned by the issuer.")
        }
        "issue_date" | "issueDate" => Some("BT-2 — Date the invoice was issued."),
        "due_date" | "dueDate" => Some("BT-9 — Payment due date."),
        "currency" => Some("BT-5 — Invoice currency code (ISO 4217)."),
        "buyer" => Some("BG-7 — Buyer party group."),
        "seller" => Some("BG-4 — Seller party group."),
        "tax_total" | "taxTotal" => Some("BT-110 — Invoice total VAT amount."),
        "line_extension_amount" | "lineExtensionAmount" => {
            Some("BT-106 — Sum of invoice line net amounts.")
        }
        "payable_amount" | "payableAmount" => Some("BT-115 — Amount due for payment."),
        _ => None,
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_lsp::crate_name(), "invoicekit-lsp");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-lsp"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_document_succeeds_on_valid_json() {
        let doc = parse_document(r#"{"id":"INV-1"}"#);
        assert!(doc.parses);
        assert!(doc.diagnostics.is_empty());
    }

    #[test]
    fn parse_document_fails_on_invalid_json_with_diagnostic() {
        let doc = parse_document("{ bad json");
        assert!(!doc.parses);
        assert_eq!(doc.diagnostics.len(), 1);
        assert_eq!(doc.diagnostics[0].severity, DiagnosticSeverity::Error);
        assert_eq!(doc.diagnostics[0].code, "invoicekit/json-parse");
    }

    #[test]
    fn diagnostic_severity_codes_match_lsp_spec() {
        assert_eq!(DiagnosticSeverity::Error.code(), 1);
        assert_eq!(DiagnosticSeverity::Warning.code(), 2);
        assert_eq!(DiagnosticSeverity::Information.code(), 3);
        assert_eq!(DiagnosticSeverity::Hint.code(), 4);
    }

    #[test]
    fn default_capabilities_advertise_hover_and_diagnostics() {
        let caps = Capabilities::default();
        assert!(caps.hover_provider);
        assert!(caps.diagnostic_provider);
        assert!(!caps.completion_provider);
    }

    #[test]
    fn hover_at_returns_gloss_for_known_term() {
        let doc = parse_document(r#"{"id":"INV-1"}"#);
        // Cursor on `id` at line 0 column 2 ("{" "\"" "i" "d").
        let info = hover_at(
            &doc,
            Position {
                line: 0,
                character: 2,
            },
        )
        .unwrap()
        .expect("expected hover for `id`");
        assert_eq!(info.term, "id");
        assert!(info.description.starts_with("BT-1"));
    }

    #[test]
    fn hover_at_returns_none_for_unknown_term() {
        let doc = parse_document(r#"{"random_field":42}"#);
        let info = hover_at(
            &doc,
            Position {
                line: 0,
                character: 5,
            },
        )
        .unwrap();
        assert!(info.is_none());
    }

    #[test]
    fn hover_at_rejects_position_past_document_end() {
        let doc = parse_document(r#"{"id":1}"#);
        let err = hover_at(
            &doc,
            Position {
                line: 5,
                character: 0,
            },
        )
        .unwrap_err();
        assert!(matches!(err, LspError::PositionOutOfRange(_)));
    }

    #[test]
    fn position_and_range_round_trip_through_serde() {
        let r = Range {
            start: Position {
                line: 1,
                character: 2,
            },
            end: Position {
                line: 3,
                character: 4,
            },
        };
        let json = serde_json::to_string(&r).unwrap();
        let parsed: Range = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn diagnostic_round_trips_through_serde() {
        let d = Diagnostic {
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 1,
                },
            },
            severity: DiagnosticSeverity::Warning,
            code: "test/diag".to_owned(),
            message: "test".to_owned(),
        };
        let json = serde_json::to_string(&d).unwrap();
        let parsed: Diagnostic = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, d);
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-lsp");
    }
}
