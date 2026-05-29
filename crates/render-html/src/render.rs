// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-056 invoice → HTML5 renderer.
//!
//! Hand-rolled string templating instead of pulling in tera/askama:
//! the invoice document is small (~30 fields and a line array), and
//! the WCAG-conformance contract means every output element has to
//! be hand-audited anyway. A templating engine wouldn't simplify
//! the audit and would add a dep + a syntax to learn.

use std::fmt::Write as _;

use invoicekit_ir::{
    CommercialDocument, DocumentLine, MonetaryTotal, Party, PostalAddress, TaxCategorySummary,
};
use thiserror::Error;

use crate::palette;

/// Caller-tunable rendering options.
#[derive(Clone, Debug, Default)]
pub struct RenderOptions {
    /// BCP 47 language tag stamped on `<html lang>`. Defaults to
    /// `en` when `None`; the first localized note's language wins
    /// when present.
    pub language: Option<String>,
    /// Optional human-readable title overriding the auto-generated
    /// "Invoice #N" / "Credit note #N" string.
    pub title: Option<String>,
}

/// Errors raised by [`render_invoice_html`].
#[derive(Debug, Error)]
pub enum RenderError {
    /// The document failed IR validation; we refuse to render an
    /// invalid invoice because the WCAG-conformance contract
    /// assumes every advertised field is present.
    #[error("invoice failed IR validation: {0}")]
    InvalidInvoice(#[from] invoicekit_ir::IrError),
}

/// Render `doc` to a self-contained, WCAG 2.1 AA conformant HTML5
/// document.
///
/// The output is one string with no external resources; styling is
/// inline so a customer can drop it into an email body without
/// chasing CSS hosting. The structure is:
///
/// ```html
/// <!doctype html><html lang="…"><head>…</head>
/// <body><main>
///   <article aria-label="Invoice INV-1">
///     <header>…</header>
///     <section aria-labelledby="parties">…</section>
///     <section aria-labelledby="lines">…</section>
///     <section aria-labelledby="totals">…</section>
///     <section aria-labelledby="payment">…</section>
///     <section aria-labelledby="notes">…</section>
///   </article>
/// </main></body></html>
/// ```
///
/// # Errors
///
/// Returns [`RenderError::InvalidInvoice`] when `doc` fails its own
/// IR validation; the renderer refuses to emit HTML for an invoice
/// it doesn't structurally trust.
pub fn render_invoice_html(
    doc: &CommercialDocument,
    options: &RenderOptions,
) -> Result<String, RenderError> {
    doc.validate()?;

    let lang = options
        .language
        .clone()
        .or_else(|| {
            doc.notes
                .first()
                .map(|n| n.language.clone())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "en".to_owned());

    let title = options
        .title
        .clone()
        .unwrap_or_else(|| format!("{} {}", doc_kind_label(doc), doc.document_number.as_str()));

    let mut out = String::with_capacity(8 * 1024);
    out.push_str("<!doctype html>\n");
    let _ = writeln!(out, r#"<html lang="{}">"#, escape_attr(&lang));
    write_head(&mut out, &title);
    out.push_str("<body>\n<main>\n");
    let _ = writeln!(
        out,
        r#"<article aria-label="{kind} {num}">"#,
        kind = escape_attr(doc_kind_label(doc)),
        num = escape_attr(doc.document_number.as_str())
    );
    write_header(&mut out, doc, &title);
    write_parties(&mut out, doc);
    write_lines(&mut out, doc);
    write_totals(&mut out, &doc.monetary_total, &doc.tax_summary, doc);
    write_payment(&mut out, doc);
    write_notes(&mut out, doc);
    out.push_str("</article>\n</main>\n</body>\n</html>\n");
    Ok(out)
}

fn write_head(out: &mut String, title: &str) {
    out.push_str("<head>\n");
    out.push_str(r#"<meta charset="utf-8">"#);
    out.push('\n');
    out.push_str(r#"<meta name="viewport" content="width=device-width, initial-scale=1">"#);
    out.push('\n');
    let _ = writeln!(out, r#"<title>{}</title>"#, escape_text(title));
    out.push_str("<style>\n");
    out.push_str(&inline_stylesheet());
    out.push_str("</style>\n</head>\n");
}

fn inline_stylesheet() -> String {
    use palette::*;
    format!(
        r#":root {{
  color-scheme: light;
}}
* {{ box-sizing: border-box; }}
body {{
  margin: 0;
  font-family: ui-sans-serif, system-ui, -apple-system, sans-serif;
  font-size: 16px;
  line-height: 1.5;
  color: {FG_TEXT};
  background: {BG_PAGE};
}}
main {{ max-width: 64rem; margin: 0 auto; padding: 2rem; }}
article {{
  border: 1px solid {BORDER};
  border-radius: 0.5rem;
  overflow: hidden;
}}
header {{
  background: {ACCENT};
  color: {ACCENT_FG};
  padding: 1.5rem 2rem;
}}
header h1 {{ margin: 0 0 0.5rem; font-size: 1.5rem; font-weight: 700; }}
header .meta {{ font-size: 0.95rem; }}
section {{ padding: 1.5rem 2rem; border-top: 1px solid {BORDER}; }}
section h2 {{ margin: 0 0 0.75rem; font-size: 1.125rem; color: {FG_TEXT}; }}
.parties {{ display: grid; grid-template-columns: 1fr 1fr; gap: 2rem; }}
@media (max-width: 40rem) {{ .parties {{ grid-template-columns: 1fr; }} }}
dl {{ margin: 0; }}
dt {{ font-weight: 600; color: {FG_MUTED}; }}
dd {{ margin: 0 0 0.75rem; }}
table {{ width: 100%; border-collapse: collapse; }}
caption {{ text-align: left; color: {FG_MUTED}; padding-bottom: 0.5rem; }}
th, td {{ padding: 0.5rem 0.75rem; border-bottom: 1px solid {BORDER}; text-align: left; vertical-align: top; }}
th[scope=col] {{ background: #f3f4f6; font-weight: 600; }}
td.num, th.num {{ text-align: right; font-variant-numeric: tabular-nums; }}
.totals dt {{ text-align: right; }}
.totals dd {{ text-align: right; font-variant-numeric: tabular-nums; }}
.totals .grand {{ font-weight: 700; font-size: 1.125rem; }}
.muted {{ color: {FG_MUTED}; }}
"#,
    )
}

fn write_header(out: &mut String, doc: &CommercialDocument, title: &str) {
    out.push_str("<header>\n");
    let _ = writeln!(out, r#"<h1>{}</h1>"#, escape_text(title));
    out.push_str("<p class=\"meta\">\n");
    let _ = writeln!(
        out,
        "Issued <time datetime=\"{date}\">{date}</time>",
        date = escape_text(doc.issue_date.as_str())
    );
    if let Some(due) = &doc.due_date {
        let _ = writeln!(
            out,
            r#" — due <time datetime="{date}">{date}</time>"#,
            date = escape_text(due.as_str())
        );
    }
    if let Some(tax_point) = &doc.tax_point_date {
        let _ = writeln!(
            out,
            r#" — tax point <time datetime="{date}">{date}</time>"#,
            date = escape_text(tax_point.as_str())
        );
    }
    out.push_str("</p>\n");
    out.push_str("</header>\n");
}

fn write_parties(out: &mut String, doc: &CommercialDocument) {
    out.push_str(
        "<section aria-labelledby=\"parties-heading\">\n<h2 id=\"parties-heading\">Parties</h2>\n",
    );
    out.push_str("<div class=\"parties\">\n");
    out.push_str("<div>\n<h3>Supplier</h3>\n");
    write_party_dl(out, &doc.supplier);
    out.push_str("</div>\n");
    out.push_str("<div>\n<h3>Customer</h3>\n");
    write_party_dl(out, &doc.customer);
    out.push_str("</div>\n</div>\n");
    if let Some(payee) = &doc.payee {
        out.push_str("<div>\n<h3>Payee</h3>\n");
        write_party_dl(out, payee);
        out.push_str("</div>\n");
    }
    out.push_str("</section>\n");
}

fn write_party_dl(out: &mut String, party: &Party) {
    out.push_str("<dl>\n");
    let _ = writeln!(out, "<dt>Name</dt><dd>{}</dd>", escape_text(&party.name));
    for tax in &party.tax_ids {
        let _ = writeln!(
            out,
            "<dt>Tax ID ({})</dt><dd>{}</dd>",
            escape_text(&tax.scheme),
            escape_text(&tax.value)
        );
    }
    out.push_str("<dt>Address</dt><dd>");
    write_address_block(out, &party.address);
    out.push_str("</dd>\n");
    if let Some(c) = &party.contact {
        if let Some(email) = &c.email {
            let attr = escape_attr(email);
            let _ = writeln!(
                out,
                r#"<dt>Email</dt><dd><a href="mailto:{attr}">{}</a></dd>"#,
                escape_text(email),
            );
        }
        if let Some(phone) = &c.phone {
            let _ = writeln!(out, "<dt>Phone</dt><dd>{}</dd>", escape_text(phone));
        }
    }
    out.push_str("</dl>\n");
}

fn write_address_block(out: &mut String, addr: &PostalAddress) {
    let mut parts: Vec<String> = Vec::new();
    parts.extend(addr.lines.iter().map(|l| escape_text(l)));
    parts.push(escape_text(&addr.city));
    if let Some(sub) = &addr.subdivision {
        parts.push(escape_text(sub));
    }
    parts.push(escape_text(&addr.postal_code));
    parts.push(escape_attr(addr.country.as_str()));
    out.push_str(&parts.join("<br>"));
}

fn write_lines(out: &mut String, doc: &CommercialDocument) {
    out.push_str(
        "<section aria-labelledby=\"lines-heading\">\n<h2 id=\"lines-heading\">Line items</h2>\n",
    );
    out.push_str("<table>\n<caption class=\"muted\">Document lines</caption>\n");
    out.push_str(
        "<thead><tr>\n<th scope=\"col\">#</th>\n<th scope=\"col\">Description</th>\n<th scope=\"col\" class=\"num\">Qty</th>\n<th scope=\"col\" class=\"num\">Unit price</th>\n<th scope=\"col\" class=\"num\">Amount</th>\n</tr></thead>\n",
    );
    out.push_str("<tbody>\n");
    for (idx, line) in doc.lines.iter().enumerate() {
        write_line_row(out, idx, line);
    }
    out.push_str("</tbody>\n</table>\n</section>\n");
}

fn write_line_row(out: &mut String, idx: usize, line: &DocumentLine) {
    out.push_str("<tr>\n");
    let _ = writeln!(out, r#"<th scope="row">{}</th>"#, idx + 1);
    let _ = writeln!(out, "<td>{}</td>", escape_text(&line.description));
    let _ = writeln!(
        out,
        r#"<td class="num">{}</td>"#,
        escape_text(&decimal_str(&line.quantity))
    );
    let _ = writeln!(
        out,
        r#"<td class="num">{}</td>"#,
        escape_text(&decimal_str(&line.unit_price))
    );
    let _ = writeln!(
        out,
        r#"<td class="num">{}</td>"#,
        escape_text(&decimal_str(&line.line_extension_amount))
    );
    out.push_str("</tr>\n");
}

fn write_totals(
    out: &mut String,
    totals: &MonetaryTotal,
    tax: &[TaxCategorySummary],
    doc: &CommercialDocument,
) {
    out.push_str(
        "<section aria-labelledby=\"totals-heading\" class=\"totals\">\n<h2 id=\"totals-heading\">Totals</h2>\n",
    );
    let currency = doc.currency.as_str();
    out.push_str("<dl>\n");
    write_money_row(
        out,
        "Sum of line amounts",
        &decimal_str(&totals.line_extension_amount),
        currency,
    );
    write_money_row(
        out,
        "Tax-exclusive total",
        &decimal_str(&totals.tax_exclusive_amount),
        currency,
    );
    for s in tax {
        let label = format!("Tax ({})", s.category_code);
        write_money_row(out, &label, &decimal_str(&s.tax_amount), currency);
    }
    write_money_row(
        out,
        "Tax-inclusive total",
        &decimal_str(&totals.tax_inclusive_amount),
        currency,
    );
    if let Some(p) = &totals.prepaid_amount {
        write_money_row(out, "Prepaid", &decimal_str(p), currency);
    }
    out.push_str(r#"<dt class="grand">Amount due</dt>"#);
    let _ = writeln!(
        out,
        r#"<dd class="grand">{} {}</dd>"#,
        escape_text(&decimal_str(&totals.payable_amount)),
        escape_text(currency)
    );
    out.push_str("</dl>\n</section>\n");
}

fn write_money_row(out: &mut String, label: &str, amount: &str, currency: &str) {
    let _ = writeln!(out, "<dt>{}</dt>", escape_text(label));
    let _ = writeln!(
        out,
        "<dd>{} {}</dd>",
        escape_text(amount),
        escape_text(currency)
    );
}

fn write_payment(out: &mut String, doc: &CommercialDocument) {
    if doc.payment_terms.is_none() && doc.payment_instructions.is_empty() {
        return;
    }
    out.push_str(
        "<section aria-labelledby=\"payment-heading\">\n<h2 id=\"payment-heading\">Payment</h2>\n",
    );
    if let Some(terms) = &doc.payment_terms {
        let _ = writeln!(
            out,
            r#"<p class="muted">{}</p>"#,
            escape_text(&terms.description)
        );
        if let Some(due) = &terms.due_date {
            let _ = writeln!(
                out,
                r#"<p>Due by <time datetime="{date}">{date}</time>.</p>"#,
                date = escape_text(due.as_str())
            );
        }
    }
    if !doc.payment_instructions.is_empty() {
        out.push_str("<ul>\n");
        for inst in &doc.payment_instructions {
            out.push_str("<li>");
            let _ = write!(out, "<strong>{:?}</strong>", inst.kind);
            if let Some(acct) = &inst.account {
                let _ = write!(out, " — account {}", escape_text(acct));
            }
            if let Some(reference) = &inst.reference {
                let _ = write!(out, " (ref {})", escape_text(reference));
            }
            out.push_str("</li>\n");
        }
        out.push_str("</ul>\n");
    }
    out.push_str("</section>\n");
}

fn write_notes(out: &mut String, doc: &CommercialDocument) {
    if doc.notes.is_empty() {
        return;
    }
    out.push_str(
        "<section aria-labelledby=\"notes-heading\">\n<h2 id=\"notes-heading\">Notes</h2>\n",
    );
    for note in &doc.notes {
        let _ = writeln!(
            out,
            r#"<p lang="{}">{}</p>"#,
            escape_attr(&note.language),
            escape_text(&note.text)
        );
    }
    out.push_str("</section>\n");
}

fn doc_kind_label(doc: &CommercialDocument) -> &'static str {
    use invoicekit_ir::DocumentType;
    match doc.document_type {
        DocumentType::Invoice => "Invoice",
        DocumentType::CreditNote => "Credit note",
        DocumentType::DebitNote => "Debit note",
        DocumentType::ProForma => "Pro forma",
        DocumentType::SelfBilled => "Self-billed invoice",
    }
}

fn decimal_str(v: &invoicekit_ir::DecimalValue) -> String {
    v.inner().to_string()
}

fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_ir::{
        CommercialDocument, CommercialDocumentParts, Contact, CountryCode, DateOnly, DecimalValue,
        DocumentId, DocumentLine, DocumentMeta, DocumentNumber, DocumentType, Iso4217Code,
        LocalizedString, MonetaryTotal, Party, PartyTaxId, PaymentInstruction,
        PaymentInstructionKind, PostalAddress, TaxCategorySummary,
    };
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn dv(s: &str) -> DecimalValue {
        DecimalValue::new(Decimal::from_str(s).unwrap())
    }

    fn sample_doc() -> CommercialDocument {
        let parts = CommercialDocumentParts {
            schema_version: invoicekit_ir::SchemaVersion::default(),
            id: DocumentId::new("doc-001").unwrap(),
            document_type: DocumentType::Invoice,
            issue_date: DateOnly::new("2026-05-27").unwrap(),
            tax_point_date: None,
            due_date: Some(DateOnly::new("2026-06-27").unwrap()),
            document_number: DocumentNumber::new("F-2026-001").unwrap(),
            currency: Iso4217Code::new("EUR").unwrap(),
            supplier: Party {
                id: None,
                name: "Acme Corp <Holdings>".into(),
                tax_ids: vec![PartyTaxId {
                    scheme: "vat".into(),
                    value: "B12345678".into(),
                }],
                address: PostalAddress {
                    lines: vec!["Calle Mayor 1".into()],
                    city: "Madrid".into(),
                    subdivision: Some("M".into()),
                    postal_code: "28013".into(),
                    country: CountryCode::new("ES").unwrap(),
                },
                contact: Some(Contact {
                    name: Some("Ana".into()),
                    email: Some("billing@acme.example".into()),
                    phone: Some("+34 911 234 567".into()),
                }),
            },
            customer: Party {
                id: None,
                name: "Widget Buyer SARL".into(),
                tax_ids: vec![PartyTaxId {
                    scheme: "vat".into(),
                    value: "FR12345678901".into(),
                }],
                address: PostalAddress {
                    lines: vec!["12 rue de la Paix".into()],
                    city: "Paris".into(),
                    subdivision: None,
                    postal_code: "75001".into(),
                    country: CountryCode::new("FR").unwrap(),
                },
                contact: None,
            },
            payee: None,
            payment_terms: Some(invoicekit_ir::PaymentTerms {
                description: "Net 30 from issue date".into(),
                due_date: Some(DateOnly::new("2026-06-27").unwrap()),
            }),
            payment_instructions: vec![PaymentInstruction {
                kind: PaymentInstructionKind::IbanBic,
                account: Some("ES1234567890".into()),
                reference: Some("F-2026-001".into()),
            }],
            lines: vec![DocumentLine {
                id: "L1".into(),
                description: "Premium Widget (10 units)".into(),
                quantity: dv("2"),
                unit_code: Some("EA".into()),
                unit_price: dv("100.00"),
                line_extension_amount: dv("200.00"),
                tax_category: Some("standard".into()),
                extensions: vec![],
            }],
            tax_summary: vec![TaxCategorySummary {
                category_code: "VAT".into(),
                taxable_amount: dv("200.00"),
                tax_amount: dv("42.00"),
                tax_rate: Some(dv("21.00")),
            }],
            monetary_total: MonetaryTotal {
                line_extension_amount: dv("200.00"),
                tax_exclusive_amount: dv("200.00"),
                tax_inclusive_amount: dv("242.00"),
                allowance_total_amount: None,
                charge_total_amount: None,
                prepaid_amount: None,
                payable_amount: dv("242.00"),
            },
            attachments: vec![],
            references: vec![],
            notes: vec![LocalizedString {
                language: "en".into(),
                text: "Thank you for your business.".into(),
            }],
            extensions: vec![],
            meta: DocumentMeta {
                tenant_id: "tenant-x".into(),
                trace_id: "trace-001".into(),
                source_system: Some("test".into()),
            },
        };
        CommercialDocument::new(parts).unwrap()
    }

    #[test]
    fn renders_doctype_lang_and_meta() {
        let html = render_invoice_html(&sample_doc(), &RenderOptions::default()).unwrap();
        assert!(html.starts_with("<!doctype html>\n"));
        assert!(html.contains(r#"<html lang="en">"#));
        assert!(html.contains(r#"<meta charset="utf-8">"#));
        assert!(html.contains(r#"<meta name="viewport""#));
    }

    #[test]
    fn renders_semantic_landmarks_and_sections() {
        let html = render_invoice_html(&sample_doc(), &RenderOptions::default()).unwrap();
        for landmark in [
            "<main>",
            "<article ",
            "<header>",
            "<section aria-labelledby=\"parties-heading\">",
            "<section aria-labelledby=\"lines-heading\">",
            "<section aria-labelledby=\"totals-heading\"",
            "<section aria-labelledby=\"payment-heading\">",
            "<section aria-labelledby=\"notes-heading\">",
        ] {
            assert!(html.contains(landmark), "missing landmark {landmark}");
        }
    }

    #[test]
    fn line_table_uses_thead_and_th_scope() {
        let html = render_invoice_html(&sample_doc(), &RenderOptions::default()).unwrap();
        assert!(html.contains("<thead>"));
        assert!(html.contains(r#"<th scope="col">Description</th>"#));
        assert!(html.contains(r#"<th scope="row">1</th>"#));
        assert!(html.contains("<caption"));
    }

    #[test]
    fn escapes_special_characters_in_text() {
        let html = render_invoice_html(&sample_doc(), &RenderOptions::default()).unwrap();
        // Supplier name was "Acme Corp <Holdings>"; the &lt; and &gt;
        // must appear, the raw < > must not appear inside the text.
        assert!(html.contains("Acme Corp &lt;Holdings&gt;"));
        // Sanity-check that the document body still contains valid
        // open angles for our own tags.
        assert!(html.contains("<h1>"));
    }

    #[test]
    fn never_emits_script_tags() {
        let html = render_invoice_html(&sample_doc(), &RenderOptions::default()).unwrap();
        assert!(!html.to_lowercase().contains("<script"));
    }

    #[test]
    fn renders_payment_instructions_when_present() {
        let html = render_invoice_html(&sample_doc(), &RenderOptions::default()).unwrap();
        assert!(html.contains("ES1234567890"));
        assert!(html.contains("F-2026-001"));
        assert!(html.contains("IbanBic"));
    }

    #[test]
    fn renders_localized_note_with_lang_attribute() {
        let html = render_invoice_html(&sample_doc(), &RenderOptions::default()).unwrap();
        assert!(html.contains(r#"<p lang="en">Thank you for your business.</p>"#));
    }

    #[test]
    fn render_options_override_language() {
        let opts = RenderOptions {
            language: Some("de".into()),
            title: None,
        };
        let html = render_invoice_html(&sample_doc(), &opts).unwrap();
        assert!(html.contains(r#"<html lang="de">"#));
    }

    #[test]
    fn render_options_override_title() {
        let opts = RenderOptions {
            language: None,
            title: Some("Custom Title".into()),
        };
        let html = render_invoice_html(&sample_doc(), &opts).unwrap();
        assert!(html.contains("<h1>Custom Title</h1>"));
    }

    #[test]
    fn credit_note_kind_label_is_used() {
        let mut doc = sample_doc();
        doc.document_type = DocumentType::CreditNote;
        let html = render_invoice_html(&doc, &RenderOptions::default()).unwrap();
        assert!(html.contains("Credit note F-2026-001"));
    }

    #[test]
    fn renders_payable_amount_alongside_currency() {
        let html = render_invoice_html(&sample_doc(), &RenderOptions::default()).unwrap();
        assert!(html.contains(r#"<dd class="grand">242.00 EUR</dd>"#));
    }
}
