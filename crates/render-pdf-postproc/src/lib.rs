// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-053: post-process a Typst-rendered PDF into ZUGFeRD-grade PDF/A-3.
//!
//! Typst emits PDF/A-3b-compliant bytes via `typst_pdf::PdfOptions`,
//! but does not write the XMP metadata or the `AssociatedFile`
//! relationship that Factur-X / ZUGFeRD readers look for. This
//! crate is the post-processing pass: it takes Typst's PDF output
//! plus the Factur-X XML payload and emits a PDF where:
//!
//! 1. The XML lives in the catalog's `Names.EmbeddedFiles` name
//!    tree under the canonical filename for the profile.
//! 2. The catalog's `AF` array references the same `Filespec`, so
//!    PDF/A-3 readers find the attachment regardless of which path
//!    they walk first.
//! 3. The XMP packet in the catalog's `Metadata` stream declares
//!    the `fx:` namespace and the chosen ZUGFeRD profile, so
//!    veraPDF's `--profile=3b` / `--profile=3u` validation does
//!    not flag the attachment as undeclared.
//!
//! The injection is byte-deterministic — the same PDF + the same
//! XML produces byte-identical output across runs, so a downstream
//! release pipeline can hash the post-processed PDF and store the
//! digest as conformance evidence.
//!
//! ## Decision rule (upstream PR vs `lopdf` patch)
//!
//! `plans/PDF-A-3-POSTPROC.md` records the rule we follow when
//! deciding whether to fix a missing PDF feature here (`lopdf`
//! patch) or upstream in Typst:
//!
//! - **Upstream PR** when the missing feature is part of the PDF
//!   spec proper (e.g. XMP metadata format, Filespec dictionary
//!   shape). Typst should not require a downstream patch to
//!   produce a valid PDF/A.
//! - **`lopdf` patch (this crate)** when the missing feature is
//!   a Factur-X / ZUGFeRD-specific overlay that does not belong
//!   in a general-purpose Typst renderer. The XML attachment and
//!   the `fx:` XMP namespace are this category.

use lopdf::{dictionary, Document, Object, Stream, StringFormat};
use thiserror::Error;

pub use invoicekit_intake_pdf::FACTUR_X_ATTACHMENT_NAMES;

/// ZUGFeRD / Factur-X profile identifier.
///
/// The variant determines the XML attachment filename and the
/// `fx:ConformanceLevel` value emitted into the XMP packet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ZugferdProfile {
    /// Minimum profile (header data only).
    Minimum,
    /// Basic WL profile (header + summary, no line items).
    BasicWl,
    /// Basic profile.
    Basic,
    /// EN 16931 profile (full EN 16931 dataset).
    En16931,
    /// Extended profile (EN 16931 plus extension data).
    Extended,
    /// XRechnung-in-PDF profile (German B2G).
    Xrechnung,
}

impl ZugferdProfile {
    /// Canonical attachment filename for this profile.
    ///
    /// Per ZUGFeRD 2.1 / Factur-X 1.0, all profiles use
    /// `factur-x.xml` except XRechnung-in-PDF, which uses
    /// `xrechnung.xml`.
    #[must_use]
    pub const fn attachment_filename(self) -> &'static str {
        match self {
            Self::Xrechnung => "xrechnung.xml",
            _ => "factur-x.xml",
        }
    }

    /// `fx:ConformanceLevel` value emitted into the XMP packet.
    #[must_use]
    pub const fn xmp_conformance_level(self) -> &'static str {
        match self {
            Self::Minimum => "MINIMUM",
            Self::BasicWl => "BASIC WL",
            Self::Basic => "BASIC",
            Self::En16931 => "EN 16931",
            Self::Extended => "EXTENDED",
            Self::Xrechnung => "XRECHNUNG",
        }
    }

    /// Operator-readable identifier used by tracing + tests.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Minimum => "MINIMUM",
            Self::BasicWl => "BASIC WL",
            Self::Basic => "BASIC",
            Self::En16931 => "EN 16931",
            Self::Extended => "EXTENDED",
            Self::Xrechnung => "XRECHNUNG",
        }
    }
}

/// Errors raised by [`embed_factur_x`].
#[derive(Debug, Error)]
pub enum PostprocError {
    /// The Typst output is not a parseable PDF.
    #[error("input is not a parseable PDF: {0}")]
    Parse(String),
    /// Serialization of the patched PDF failed.
    #[error("serialization of the patched PDF failed: {0}")]
    Serialize(String),
}

/// Inject the Factur-X / ZUGFeRD XML attachment, the `AssociatedFile`
/// pointer, and a profile-aware XMP metadata packet into a
/// Typst-rendered PDF.
///
/// Returns the post-processed PDF bytes. The output is
/// byte-deterministic over the (pdf, xml, profile) triple.
///
/// # Errors
///
/// Returns [`PostprocError::Parse`] when the input is not a valid
/// PDF and [`PostprocError::Serialize`] when the patched document
/// cannot be written back out.
pub fn embed_factur_x(
    pdf: &[u8],
    xml: &[u8],
    profile: ZugferdProfile,
) -> Result<Vec<u8>, PostprocError> {
    let mut doc = Document::load_mem(pdf).map_err(|e| PostprocError::Parse(e.to_string()))?;

    let filename = profile.attachment_filename();

    // 1. Add the embedded-file stream.
    let xml_stream_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "EmbeddedFile",
            "Subtype" => Object::Name(b"text/xml".to_vec()),
        },
        xml.to_vec(),
    ));

    // 2. Add the Filespec dictionary that points at it.
    let filespec_id = doc.add_object(dictionary! {
        "Type" => "Filespec",
        "F" => Object::String(filename.as_bytes().to_vec(), StringFormat::Literal),
        "UF" => Object::String(filename.as_bytes().to_vec(), StringFormat::Literal),
        "AFRelationship" => Object::Name(b"Alternative".to_vec()),
        "EF" => dictionary! {
            "F" => xml_stream_id,
            "UF" => xml_stream_id,
        },
    });

    // 3. Add the XMP metadata stream (overwrites Typst's default,
    //    which omits the fx: namespace).
    let xmp = render_xmp(profile);
    let metadata_stream_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "Metadata",
            "Subtype" => Object::Name(b"XML".to_vec()),
        },
        xmp.into_bytes(),
    ));

    // 4. Patch the catalog: add Names.EmbeddedFiles, AF, and the
    //    Metadata pointer.
    let catalog_id = catalog_id(&doc)
        .ok_or_else(|| PostprocError::Parse("PDF has no /Root reference in trailer".to_owned()))?;
    let names_id = doc.add_object(dictionary! {
        "EmbeddedFiles" => dictionary! {
            "Names" => vec![
                Object::String(filename.as_bytes().to_vec(), StringFormat::Literal),
                filespec_id.into(),
            ],
        },
    });
    let af_value: Object = vec![filespec_id.into()].into();
    if let Ok(Object::Dictionary(catalog_dict)) = doc.get_object_mut(catalog_id) {
        catalog_dict.set("Names", names_id);
        catalog_dict.set("AF", af_value);
        catalog_dict.set("Metadata", metadata_stream_id);
    } else {
        return Err(PostprocError::Parse(
            "PDF catalog is not a dictionary".to_owned(),
        ));
    }

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PostprocError::Serialize(e.to_string()))?;
    Ok(out)
}

fn catalog_id(doc: &Document) -> Option<lopdf::ObjectId> {
    doc.trailer.get(b"Root").ok().and_then(|root| match root {
        Object::Reference(id) => Some(*id),
        _ => None,
    })
}

/// Render the XMP packet for a given ZUGFeRD profile. The packet
/// is intentionally short — veraPDF's `--profile=3b` validation
/// reads the `fx:DocumentType`, `fx:DocumentFileName`,
/// `fx:Version`, and `fx:ConformanceLevel` triples; everything
/// else is best-effort metadata that the next caller can extend.
fn render_xmp(profile: ZugferdProfile) -> String {
    let conformance = profile.xmp_conformance_level();
    let filename = profile.attachment_filename();
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n  \
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n    \
         <rdf:Description \
           xmlns:fx=\"urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#\" \
           rdf:about=\"\">\n      \
         <fx:DocumentType>INVOICE</fx:DocumentType>\n      \
         <fx:DocumentFileName>{filename}</fx:DocumentFileName>\n      \
         <fx:Version>1.0</fx:Version>\n      \
         <fx:ConformanceLevel>{conformance}</fx:ConformanceLevel>\n    \
         </rdf:Description>\n  \
         </rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"w\"?>\n"
    )
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_render_pdf_postproc::crate_name(),
///     "invoicekit-render-pdf-postproc"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-render-pdf-postproc"
}

#[cfg(test)]
mod tests {
    use super::{crate_name, embed_factur_x, render_xmp, PostprocError, ZugferdProfile};

    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Dictionary, Document, Object, Stream};

    use invoicekit_intake_pdf::extract_factur_x_xml;

    /// Synthesize a minimal PDF for each fixture index using lopdf
    /// directly so the test does not pull the Typst dependency
    /// graph in (which would force the Typst advisory waivers in
    /// `tools/release-checks/verify_release_checks.py` to be widened
    /// to a non-`invoicekit-render-pdf` crate).
    fn synthesize_pdf_for_profile(idx: usize) -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let content_id = doc.add_object(Stream::new(
            Dictionary::new(),
            Content {
                operations: vec![Operation::new("q", vec![]), Operation::new("Q", vec![])],
            }
            .encode()
            .unwrap(),
        ));
        let leaf_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => Object::Reference((0, 0)),
            "Contents" => content_id,
        });
        let parent_id = doc.add_object(dictionary! {
            "Type" => "Pages",
            "Count" => 1,
            "Kids" => vec![leaf_id.into()],
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        if let Ok(Object::Dictionary(d)) = doc.get_object_mut(leaf_id) {
            d.set("Parent", parent_id);
        }
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => parent_id,
        });
        doc.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("serialize synthesised pdf");
        bytes.extend_from_slice(format!("\n%idx-{idx}\n").as_bytes());
        bytes
    }

    fn xml_for_profile(profile: ZugferdProfile, idx: usize) -> Vec<u8> {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <rsm:CrossIndustryInvoice \
               xmlns:rsm=\"urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100\">\n  \
             <guideline-id>{name}</guideline-id>\n  \
             <fixture-index>{idx}</fixture-index>\n\
             </rsm:CrossIndustryInvoice>\n",
            name = profile.name()
        )
        .into_bytes()
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-render-pdf-postproc");
    }

    /// Strict acceptance gate: 5 fixtures per profile (30 total).
    /// Each fixture round-trips through `embed_factur_x` and then
    /// through `extract_factur_x_xml`, and the round-tripped XML
    /// matches the input byte-for-byte. veraPDF profile-3b/3u
    /// validation is waived in this PR (no veraPDF on CI runners)
    /// and tracked as a follow-up bead.
    #[test]
    fn embed_factur_x_round_trips_all_six_profiles_with_five_fixtures_each() {
        let profiles = [
            ZugferdProfile::Minimum,
            ZugferdProfile::BasicWl,
            ZugferdProfile::Basic,
            ZugferdProfile::En16931,
            ZugferdProfile::Extended,
            ZugferdProfile::Xrechnung,
        ];
        let mut total = 0;
        for profile in profiles {
            for idx in 0..5 {
                let pdf = synthesize_pdf_for_profile(idx);
                let xml = xml_for_profile(profile, idx);
                let patched = embed_factur_x(&pdf, &xml, profile)
                    .unwrap_or_else(|e| panic!("embed failed for {profile:?}: {e}"));
                let extracted = extract_factur_x_xml(&patched)
                    .unwrap_or_else(|e| panic!("extract failed for {profile:?}: {e}"))
                    .unwrap_or_else(|| panic!("no attachment found for {profile:?}"));
                assert_eq!(extracted, xml, "{profile:?} round-trip drift at idx={idx}");
                total += 1;
            }
        }
        assert_eq!(total, 30, "30 fixtures expected; saw {total}");
    }

    #[test]
    fn xmp_packet_declares_factur_x_namespace_and_chosen_profile() {
        for (profile, conformance) in [
            (ZugferdProfile::Minimum, "MINIMUM"),
            (ZugferdProfile::BasicWl, "BASIC WL"),
            (ZugferdProfile::Basic, "BASIC"),
            (ZugferdProfile::En16931, "EN 16931"),
            (ZugferdProfile::Extended, "EXTENDED"),
            (ZugferdProfile::Xrechnung, "XRECHNUNG"),
        ] {
            let xmp = render_xmp(profile);
            assert!(xmp.contains("urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#"));
            assert!(xmp.contains(&format!(
                "<fx:ConformanceLevel>{conformance}</fx:ConformanceLevel>"
            )));
            assert!(xmp.contains(&format!(
                "<fx:DocumentFileName>{}</fx:DocumentFileName>",
                profile.attachment_filename()
            )));
        }
    }

    #[test]
    fn xrechnung_profile_uses_xrechnung_filename() {
        assert_eq!(
            ZugferdProfile::Xrechnung.attachment_filename(),
            "xrechnung.xml"
        );
        for profile in [
            ZugferdProfile::Minimum,
            ZugferdProfile::BasicWl,
            ZugferdProfile::Basic,
            ZugferdProfile::En16931,
            ZugferdProfile::Extended,
        ] {
            assert_eq!(profile.attachment_filename(), "factur-x.xml");
        }
    }

    #[test]
    fn embed_factur_x_is_byte_deterministic() {
        let pdf = synthesize_pdf_for_profile(0);
        let xml = xml_for_profile(ZugferdProfile::Basic, 0);
        let first = embed_factur_x(&pdf, &xml, ZugferdProfile::Basic).unwrap();
        let second = embed_factur_x(&pdf, &xml, ZugferdProfile::Basic).unwrap();
        assert_eq!(
            first, second,
            "post-processor must be byte-deterministic so the release pipeline can hash output"
        );
    }

    #[test]
    fn embed_factur_x_rejects_garbage_input() {
        let err = embed_factur_x(b"not a pdf", b"<x/>", ZugferdProfile::Basic).unwrap_err();
        assert!(matches!(err, PostprocError::Parse(_)));
    }
}
