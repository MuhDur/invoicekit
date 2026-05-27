// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-053 acceptance binary: render 30 Factur-X PDFs (5 per
//! profile x 6 profiles) into an output directory so the
//! `.github/workflows/pdfa3-verapdf.yml` job can validate them
//! against `verapdf --profile=3b` and `--profile=3u`.
//!
//! Each PDF:
//!
//! 1. Starts as the byte-stable hello-world invoice from
//!    `invoicekit-render-pdf::render_hello_world_invoice` —
//!    Typst-rendered, PDF/A-3b conformant by construction.
//! 2. Has a profile-tagged Factur-X CII XML injected via
//!    `invoicekit-render-pdf-postproc::embed_factur_x`, which
//!    writes the `Names.EmbeddedFiles` name tree, the `AF`
//!    array on the catalog, and a profile-aware XMP packet.
//!
//! Usage:
//!
//! ```bash
//! cargo run --release \
//!     -p invoicekit-render-factur-x-acceptance \
//!     -- --out target/factur-x-acceptance
//! ```
//!
//! Output layout:
//!
//! ```
//! target/factur-x-acceptance/
//!   minimum/{0..4}.pdf
//!   basic-wl/{0..4}.pdf
//!   basic/{0..4}.pdf
//!   en-16931/{0..4}.pdf
//!   extended/{0..4}.pdf
//!   xrechnung/{0..4}.pdf
//! ```
//!
//! Exit codes:
//!
//! - 0 — every PDF rendered + embedded cleanly.
//! - 1 — at least one render or embed failed; the failing
//!   profile + index is printed to stderr.
//! - 2 — bad CLI input (missing `--out`).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use invoicekit_render_pdf::render_hello_world_invoice;
use invoicekit_render_pdf_postproc::{embed_factur_x, ZugferdProfile};

fn main() -> ExitCode {
    let out = match parse_out_arg() {
        Ok(path) => path,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };
    match render_all(&out) {
        Ok(count) => {
            eprintln!("rendered {count} Factur-X PDFs under {}", out.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("render-factur-x-acceptance: {err}");
            ExitCode::FAILURE
        }
    }
}

fn parse_out_arg() -> Result<PathBuf, String> {
    let mut args = env::args().skip(1);
    let mut out: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => {
                out = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "missing value for --out".to_owned())?,
                ));
            }
            other => return Err(format!("unexpected argument: {other}")),
        }
    }
    out.ok_or_else(|| "usage: render-factur-x-acceptance --out <DIR>".to_owned())
}

fn render_all(out: &Path) -> Result<usize, Box<dyn std::error::Error>> {
    let profiles: &[(ZugferdProfile, &str)] = &[
        (ZugferdProfile::Minimum, "minimum"),
        (ZugferdProfile::BasicWl, "basic-wl"),
        (ZugferdProfile::Basic, "basic"),
        (ZugferdProfile::En16931, "en-16931"),
        (ZugferdProfile::Extended, "extended"),
        (ZugferdProfile::Xrechnung, "xrechnung"),
    ];
    let base_pdf = render_hello_world_invoice()?;
    let mut count = 0;
    for (profile, dir_name) in profiles {
        let dir = out.join(dir_name);
        fs::create_dir_all(&dir)?;
        for idx in 0..5 {
            let xml = render_profile_xml(*profile, idx);
            let patched = embed_factur_x(&base_pdf, xml.as_bytes(), *profile)
                .map_err(|e| format!("embed_factur_x failed for {profile:?} idx={idx}: {e}"))?;
            let pdf_path = dir.join(format!("{idx}.pdf"));
            fs::write(&pdf_path, patched)?;
            count += 1;
        }
    }
    Ok(count)
}

fn render_profile_xml(profile: ZugferdProfile, idx: usize) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <rsm:CrossIndustryInvoice \
           xmlns:rsm=\"urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100\">\n  \
         <guideline-id>{name}</guideline-id>\n  \
         <fixture-index>{idx}</fixture-index>\n\
         </rsm:CrossIndustryInvoice>\n",
        name = profile.name()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn render_all_emits_30_pdfs_under_out() {
        let tmp = std::env::temp_dir().join(format!(
            "render-factur-x-acceptance-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let count = render_all(&tmp).expect("render_all");
        assert_eq!(count, 30);
        for sub in [
            "minimum",
            "basic-wl",
            "basic",
            "en-16931",
            "extended",
            "xrechnung",
        ] {
            let dir = tmp.join(sub);
            assert!(dir.is_dir(), "missing dir {sub}");
            let n = fs::read_dir(&dir).unwrap().count();
            assert_eq!(n, 5, "expected 5 PDFs in {sub}, got {n}");
        }
    }

    #[test]
    fn render_profile_xml_contains_profile_name() {
        let xml = render_profile_xml(ZugferdProfile::Minimum, 0);
        assert!(xml.contains("<guideline-id>MINIMUM</guideline-id>"));
        assert!(xml.contains("<fixture-index>0</fixture-index>"));
    }
}
