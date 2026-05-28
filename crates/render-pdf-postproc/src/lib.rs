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
//!    the `fx:` namespace and the chosen ZUGFeRD profile, so the
//!    Factur-X attachment is visible to veraPDF's `--profile=3b` /
//!    `--profile=3u` oracle checks. A real veraPDF run remains
//!    mandatory before closing the conformance gate.
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

use lopdf::{dictionary, Dictionary, Document, Object, Stream, StringFormat};
use thiserror::Error;

pub use invoicekit_intake_pdf::FACTUR_X_ATTACHMENT_NAMES;

/// Number of acceptance fixtures required per Factur-X / ZUGFeRD
/// profile by T-053.
pub const ACCEPTANCE_FIXTURES_PER_PROFILE: usize = 5;

/// veraPDF profile arguments required by the T-053 acceptance gate.
pub const REQUIRED_VERAPDF_PROFILE_ARGS: [&str; 2] = ["3b", "3u"];

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
    /// All profiles covered by the T-053 acceptance matrix.
    #[must_use]
    pub const fn all() -> [Self; 6] {
        [
            Self::Minimum,
            Self::BasicWl,
            Self::Basic,
            Self::En16931,
            Self::Extended,
            Self::Xrechnung,
        ]
    }

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

    let catalog_id = catalog_id(&doc)
        .ok_or_else(|| PostprocError::Parse("PDF has no /Root reference in trailer".to_owned()))?;

    // 3. Add the XMP metadata stream. Preserve Typst's existing
    //    PDF/A identification metadata and append the Factur-X
    //    extension-schema block inside the same RDF packet.
    let xmp = render_xmp(catalog_metadata_xmp(&doc, catalog_id).as_deref(), profile);
    let metadata_stream_id = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "Metadata",
            "Subtype" => Object::Name(b"XML".to_vec()),
        },
        xmp.into_bytes(),
    ));

    // 4. Patch the catalog: add Names.EmbeddedFiles, AF, and the
    //    Metadata pointer without discarding existing name-tree
    //    entries or associated files.
    let names_id = merged_names_id(&mut doc, catalog_id, filename, filespec_id)?;
    let af_value = merged_af_value(&doc, catalog_id, filespec_id);
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

fn catalog_metadata_xmp(doc: &Document, catalog_id: lopdf::ObjectId) -> Option<String> {
    let catalog = doc.get_object(catalog_id).ok()?.as_dict().ok()?;
    let metadata_id = match catalog.get(b"Metadata").ok()? {
        Object::Reference(id) => *id,
        _ => return None,
    };
    let metadata = doc.get_object(metadata_id).ok()?.as_stream().ok()?;
    let content = metadata.get_plain_content().ok()?;
    std::str::from_utf8(&content).ok().map(ToOwned::to_owned)
}

fn merged_names_id(
    doc: &mut Document,
    catalog_id: lopdf::ObjectId,
    filename: &str,
    filespec_id: lopdf::ObjectId,
) -> Result<lopdf::ObjectId, PostprocError> {
    let catalog = doc
        .get_object(catalog_id)
        .map_err(|e| PostprocError::Parse(e.to_string()))?
        .as_dict()
        .map_err(|_| PostprocError::Parse("PDF catalog is not a dictionary".to_owned()))?;

    let mut names = match catalog.get(b"Names") {
        Ok(Object::Reference(id)) => doc
            .get_object(*id)
            .ok()
            .and_then(|object| object.as_dict().ok())
            .cloned()
            .unwrap_or_default(),
        Ok(Object::Dictionary(dict)) => dict.clone(),
        _ => Dictionary::new(),
    };

    let mut embedded_files = match names.get(b"EmbeddedFiles") {
        Ok(Object::Dictionary(dict)) => dict.clone(),
        _ => Dictionary::new(),
    };
    let mut entries = match embedded_files.get(b"Names") {
        Ok(Object::Array(items)) => without_existing_filename(items, filename),
        _ => Vec::new(),
    };
    entries.push(Object::String(
        filename.as_bytes().to_vec(),
        StringFormat::Literal,
    ));
    entries.push(filespec_id.into());
    embedded_files.set("Names", entries);
    names.set("EmbeddedFiles", embedded_files);

    Ok(doc.add_object(names))
}

fn without_existing_filename(items: &[Object], filename: &str) -> Vec<Object> {
    let mut retained = Vec::with_capacity(items.len());
    for pair in items.chunks(2) {
        if let [candidate, _] = pair {
            if is_filename(candidate, filename) {
                continue;
            }
        }
        retained.extend_from_slice(pair);
    }
    retained
}

fn is_filename(object: &Object, filename: &str) -> bool {
    matches!(object, Object::String(value, _) if value == filename.as_bytes())
}

fn merged_af_value(
    doc: &Document,
    catalog_id: lopdf::ObjectId,
    filespec_id: lopdf::ObjectId,
) -> Object {
    let mut entries = doc
        .get_object(catalog_id)
        .ok()
        .and_then(|object| object.as_dict().ok())
        .and_then(|catalog| catalog.get(b"AF").ok())
        .and_then(|object| object.as_array().ok())
        .cloned()
        .unwrap_or_default();
    let reference = Object::Reference(filespec_id);
    if !entries.contains(&reference) {
        entries.push(reference);
    }
    entries.into()
}

/// Render the XMP packet for a given ZUGFeRD profile.
///
/// PDF/A requires an extension schema declaration for the custom
/// `fx:` properties, so the packet contains both the PDF/A
/// extension-schema container and the actual Factur-X values.
fn render_xmp(existing_xmp: Option<&str>, profile: ZugferdProfile) -> String {
    let factur_x = render_factur_x_xmp_descriptions(profile);
    if let Some(existing) = existing_xmp {
        let promoted = promote_pdfa_conformance_to_3u(existing);
        let existing = promoted.as_str();
        if let Some(end) = existing.rfind("</rdf:RDF>") {
            if let Some(with_schema) = insert_factur_x_schema_into_existing_bag(existing) {
                let value_description = render_factur_x_value_description(profile);
                let schema_rdf_end = with_schema
                    .rfind("</rdf:RDF>")
                    .expect("schema merge preserves RDF end marker");
                let (before_rdf_end, after_rdf_start) = with_schema.split_at(schema_rdf_end);
                let mut merged = String::with_capacity(with_schema.len() + value_description.len());
                merged.push_str(before_rdf_end);
                merged.push_str(&value_description);
                merged.push_str(after_rdf_start);
                return merged;
            }
            let (before_rdf_end, after_rdf_start) = existing.split_at(end);
            let mut merged = String::with_capacity(existing.len() + factur_x.len());
            merged.push_str(before_rdf_end);
            merged.push_str(&factur_x);
            merged.push_str(after_rdf_start);
            return merged;
        }
    }

    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n  \
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
         {factur_x}\
         </rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"w\"?>\n"
    )
}

fn promote_pdfa_conformance_to_3u(existing: &str) -> String {
    existing.replace(
        "<pdfaid:conformance>B</pdfaid:conformance>",
        "<pdfaid:conformance>U</pdfaid:conformance>",
    )
}

fn insert_factur_x_schema_into_existing_bag(existing: &str) -> Option<String> {
    if existing.contains("Factur-X PDF/A Extension Schema") {
        return Some(existing.to_owned());
    }
    let schemas_start = existing.find("<pdfaExtension:schemas>")?;
    let schemas = existing.get(schemas_start..)?;
    let bag_relative_start = schemas.find("<rdf:Bag>")?;
    let insert_at = schemas_start + bag_relative_start + "<rdf:Bag>".len();
    let schema = render_factur_x_extension_schema();
    let before_insert = existing.get(..insert_at)?;
    let after_insert = existing.get(insert_at..)?;
    let mut merged = String::with_capacity(existing.len() + schema.len());
    merged.push_str(before_insert);
    merged.push_str(&schema);
    merged.push_str(after_insert);
    Some(merged)
}

fn render_factur_x_xmp_descriptions(profile: ZugferdProfile) -> String {
    let schema = render_factur_x_extension_schema();
    let value_description = render_factur_x_value_description(profile);
    format!(
        "\n    <rdf:Description rdf:about=\"\" \
           xmlns:pdfaExtension=\"http://www.aiim.org/pdfa/ns/extension/\" \
           xmlns:pdfaSchema=\"http://www.aiim.org/pdfa/ns/schema#\" \
           xmlns:pdfaProperty=\"http://www.aiim.org/pdfa/ns/property#\">\n      \
         <pdfaExtension:schemas>\n        \
         <rdf:Bag>{schema}</rdf:Bag>\n      \
         </pdfaExtension:schemas>\n    \
         </rdf:Description>{value_description}  "
    )
}

fn render_factur_x_extension_schema() -> String {
    "\n          \
         <rdf:li rdf:parseType=\"Resource\">\n            \
         <pdfaSchema:schema>Factur-X PDF/A Extension Schema</pdfaSchema:schema>\n            \
         <pdfaSchema:namespaceURI>urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#</pdfaSchema:namespaceURI>\n            \
         <pdfaSchema:prefix>fx</pdfaSchema:prefix>\n            \
         <pdfaSchema:property>\n              \
         <rdf:Seq>\n                \
         <rdf:li rdf:parseType=\"Resource\">\n                  \
         <pdfaProperty:name>DocumentType</pdfaProperty:name>\n                  \
         <pdfaProperty:valueType>Text</pdfaProperty:valueType>\n                  \
         <pdfaProperty:category>external</pdfaProperty:category>\n                  \
         <pdfaProperty:description>Type of the embedded Factur-X document</pdfaProperty:description>\n                \
         </rdf:li>\n                \
         <rdf:li rdf:parseType=\"Resource\">\n                  \
         <pdfaProperty:name>DocumentFileName</pdfaProperty:name>\n                  \
         <pdfaProperty:valueType>Text</pdfaProperty:valueType>\n                  \
         <pdfaProperty:category>external</pdfaProperty:category>\n                  \
         <pdfaProperty:description>Name of the embedded Factur-X XML file</pdfaProperty:description>\n                \
         </rdf:li>\n                \
         <rdf:li rdf:parseType=\"Resource\">\n                  \
         <pdfaProperty:name>Version</pdfaProperty:name>\n                  \
         <pdfaProperty:valueType>Text</pdfaProperty:valueType>\n                  \
         <pdfaProperty:category>external</pdfaProperty:category>\n                  \
         <pdfaProperty:description>Version of the Factur-X data model</pdfaProperty:description>\n                \
         </rdf:li>\n                \
         <rdf:li rdf:parseType=\"Resource\">\n                  \
         <pdfaProperty:name>ConformanceLevel</pdfaProperty:name>\n                  \
         <pdfaProperty:valueType>Text</pdfaProperty:valueType>\n                  \
         <pdfaProperty:category>external</pdfaProperty:category>\n                  \
         <pdfaProperty:description>Profile of the embedded Factur-X data</pdfaProperty:description>\n                \
         </rdf:li>\n              \
         </rdf:Seq>\n            \
         </pdfaSchema:property>\n          \
         </rdf:li>"
        .to_owned()
}

fn render_factur_x_value_description(profile: ZugferdProfile) -> String {
    let conformance = profile.xmp_conformance_level();
    let filename = profile.attachment_filename();
    format!(
        "\n    <rdf:Description rdf:about=\"\" \
           xmlns:fx=\"urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#\">\n      \
         <fx:DocumentType>INVOICE</fx:DocumentType>\n      \
         <fx:DocumentFileName>{filename}</fx:DocumentFileName>\n      \
         <fx:Version>1.0</fx:Version>\n      \
         <fx:ConformanceLevel>{conformance}</fx:ConformanceLevel>\n    \
         </rdf:Description>\n  "
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
    use super::{
        crate_name, embed_factur_x, render_xmp, ACCEPTANCE_FIXTURES_PER_PROFILE,
        REQUIRED_VERAPDF_PROFILE_ARGS,
    };

    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Dictionary, Document, Object, Stream, StringFormat};
    use quick_xml::events::Event;
    use quick_xml::Reader;

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

    fn synthesize_pdf_with_existing_catalog_entries() -> Vec<u8> {
        let mut doc = Document::load_mem(&synthesize_pdf_for_profile(99)).unwrap();
        let catalog_id = super::catalog_id(&doc).unwrap();
        let metadata_id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "Metadata",
                "Subtype" => Object::Name(b"XML".to_vec()),
            },
            existing_pdfa_xmp().as_bytes().to_vec(),
        ));
        let existing_xml_id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "EmbeddedFile",
                "Subtype" => Object::Name(b"text/xml".to_vec()),
            },
            b"<existing/>".to_vec(),
        ));
        let existing_filespec_id = doc.add_object(dictionary! {
            "Type" => "Filespec",
            "F" => Object::String(b"existing.xml".to_vec(), StringFormat::Literal),
            "UF" => Object::String(b"existing.xml".to_vec(), StringFormat::Literal),
            "AFRelationship" => Object::Name(b"Data".to_vec()),
            "EF" => dictionary! {
                "F" => existing_xml_id,
                "UF" => existing_xml_id,
            },
        });
        let names_id = doc.add_object(dictionary! {
            "EmbeddedFiles" => dictionary! {
                "Names" => vec![
                    Object::String(b"existing.xml".to_vec(), StringFormat::Literal),
                    existing_filespec_id.into(),
                ],
            },
        });

        if let Ok(Object::Dictionary(catalog)) = doc.get_object_mut(catalog_id) {
            catalog.set("Names", names_id);
            catalog.set("AF", vec![Object::Reference(existing_filespec_id)]);
            catalog.set("Metadata", metadata_id);
        }

        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    fn synthesize_pdf_with_compressed_catalog_metadata() -> Vec<u8> {
        let mut doc = Document::load_mem(&synthesize_pdf_for_profile(100)).unwrap();
        let catalog_id = super::catalog_id(&doc).unwrap();
        let mut metadata = Stream::new(
            dictionary! {
                "Type" => "Metadata",
                "Subtype" => Object::Name(b"XML".to_vec()),
            },
            existing_pdfa_xmp().as_bytes().to_vec(),
        );
        metadata.compress().unwrap();
        let metadata_id = doc.add_object(metadata);

        if let Ok(Object::Dictionary(catalog)) = doc.get_object_mut(catalog_id) {
            catalog.set("Metadata", metadata_id);
        }

        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    fn existing_pdfa_xmp() -> &'static str {
        "<?xpacket begin=\"\u{feff}\" id=\"existing\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
         <rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\">\n\
         <pdfaid:part>3</pdfaid:part>\n\
         <pdfaid:conformance>B</pdfaid:conformance>\n\
         </rdf:Description>\n\
         </rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"w\"?>\n"
    }

    fn existing_pdfa_xmp_with_extension_schema() -> &'static str {
        "<?xpacket begin=\"\u{feff}\" id=\"existing\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n\
         <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n\
         <rdf:Description rdf:about=\"\" \
           xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" \
           xmlns:pdfaExtension=\"http://www.aiim.org/pdfa/ns/extension/\" \
           xmlns:pdfaSchema=\"http://www.aiim.org/pdfa/ns/schema#\" \
           xmlns:pdfaProperty=\"http://www.aiim.org/pdfa/ns/property#\">\n\
         <pdfaExtension:schemas><rdf:Bag>\
         <rdf:li rdf:parseType=\"Resource\"><pdfaSchema:schema>Existing schema</pdfaSchema:schema></rdf:li>\
         </rdf:Bag></pdfaExtension:schemas>\n\
         <pdfaid:part>3</pdfaid:part>\n\
         <pdfaid:conformance>B</pdfaid:conformance>\n\
         </rdf:Description>\n\
         </rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"w\"?>\n"
    }

    fn assert_well_formed_xml(xmp: &str) {
        let mut reader = Reader::from_str(xmp);
        let mut result = Ok(());
        loop {
            match reader.read_event() {
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(err) => {
                    result = Err(err.to_string());
                    break;
                }
            }
        }
        assert!(result.is_ok(), "XMP should be well-formed XML: {result:?}");
    }

    fn xml_for_profile(profile: super::ZugferdProfile, idx: usize) -> Vec<u8> {
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
    /// validation is an external oracle gate and remains required
    /// before T-053 can close.
    #[test]
    fn embed_factur_x_round_trips_all_six_profiles_with_five_fixtures_each() {
        let mut total = 0;
        for profile in super::ZugferdProfile::all() {
            for idx in 0..ACCEPTANCE_FIXTURES_PER_PROFILE {
                let pdf = synthesize_pdf_for_profile(idx);
                let xml = xml_for_profile(profile, idx);
                let patched = embed_factur_x(&pdf, &xml, profile)
                    .expect("embedding fixture XML should succeed");
                let extracted = extract_factur_x_xml(&patched)
                    .expect("embedded fixture XML should extract")
                    .expect("patched PDF should contain an XML attachment");
                assert_eq!(extracted, xml, "{profile:?} round-trip drift at idx={idx}");
                total += 1;
            }
        }
        assert_eq!(total, 30, "30 fixtures expected; saw {total}");
    }

    #[test]
    fn acceptance_matrix_names_all_profiles_and_verapdf_profiles() {
        assert_eq!(super::ZugferdProfile::all().len(), 6);
        assert_eq!(ACCEPTANCE_FIXTURES_PER_PROFILE, 5);
        assert_eq!(REQUIRED_VERAPDF_PROFILE_ARGS, ["3b", "3u"]);
        assert_eq!(
            super::ZugferdProfile::all().len()
                * ACCEPTANCE_FIXTURES_PER_PROFILE
                * REQUIRED_VERAPDF_PROFILE_ARGS.len(),
            60,
            "T-053 requires 30 fixture PDFs checked across two veraPDF profiles"
        );
    }

    #[test]
    fn xmp_packet_declares_factur_x_namespace_and_chosen_profile() {
        for (profile, conformance) in [
            (super::ZugferdProfile::Minimum, "MINIMUM"),
            (super::ZugferdProfile::BasicWl, "BASIC WL"),
            (super::ZugferdProfile::Basic, "BASIC"),
            (super::ZugferdProfile::En16931, "EN 16931"),
            (super::ZugferdProfile::Extended, "EXTENDED"),
            (super::ZugferdProfile::Xrechnung, "XRECHNUNG"),
        ] {
            let xmp = render_xmp(None, profile);
            assert_well_formed_xml(&xmp);
            assert!(xmp.contains("urn:factur-x:pdfa:CrossIndustryDocument:invoice:1p0#"));
            assert!(xmp.contains("pdfaExtension:schemas"));
            assert!(xmp.contains("Factur-X PDF/A Extension Schema"));
            for property in [
                "DocumentType",
                "DocumentFileName",
                "Version",
                "ConformanceLevel",
            ] {
                assert!(xmp.contains(&format!(
                    "<pdfaProperty:name>{property}</pdfaProperty:name>"
                )));
            }
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
    fn xmp_merge_appends_factur_x_schema_to_existing_extension_bag() {
        let xmp = render_xmp(
            Some(existing_pdfa_xmp_with_extension_schema()),
            super::ZugferdProfile::Basic,
        );
        assert_well_formed_xml(&xmp);
        assert_eq!(xmp.matches("<pdfaExtension:schemas>").count(), 1);
        assert!(xmp.contains("<pdfaSchema:schema>Existing schema</pdfaSchema:schema>"));
        assert!(xmp.contains("Factur-X PDF/A Extension Schema"));
        assert!(xmp.contains("<pdfaid:part>3</pdfaid:part>"));
        assert!(xmp.contains("<pdfaid:conformance>U</pdfaid:conformance>"));
        assert!(xmp.contains("<fx:ConformanceLevel>BASIC</fx:ConformanceLevel>"));
    }

    #[test]
    fn xrechnung_profile_uses_xrechnung_filename() {
        assert_eq!(
            super::ZugferdProfile::Xrechnung.attachment_filename(),
            "xrechnung.xml"
        );
        for profile in [
            super::ZugferdProfile::Minimum,
            super::ZugferdProfile::BasicWl,
            super::ZugferdProfile::Basic,
            super::ZugferdProfile::En16931,
            super::ZugferdProfile::Extended,
        ] {
            assert_eq!(profile.attachment_filename(), "factur-x.xml");
        }
    }

    #[test]
    fn patched_pdf_catalog_points_to_embedded_file_and_xmp_metadata() {
        let pdf = synthesize_pdf_for_profile(0);
        let xml = xml_for_profile(super::ZugferdProfile::En16931, 0);
        let patched = embed_factur_x(&pdf, &xml, super::ZugferdProfile::En16931).unwrap();
        let doc = Document::load_mem(&patched).unwrap();
        let catalog_id = super::catalog_id(&doc).unwrap();
        let catalog = doc.get_object(catalog_id).unwrap().as_dict().unwrap();

        let names_id = match catalog.get(b"Names").unwrap() {
            Object::Reference(id) => Some(*id),
            _ => None,
        }
        .expect("catalog Names should be an indirect object");
        let names = doc.get_object(names_id).unwrap().as_dict().unwrap();
        let embedded_files = names.get(b"EmbeddedFiles").unwrap().as_dict().unwrap();
        let name_entries = embedded_files.get(b"Names").unwrap().as_array().unwrap();
        let filespec_id = match name_entries.as_slice() {
            [Object::String(filename, _), Object::Reference(id)] => {
                assert_eq!(filename, b"factur-x.xml");
                Some(*id)
            }
            _ => None,
        }
        .expect("EmbeddedFiles name tree should point at a Filespec");

        let af_entries = catalog.get(b"AF").unwrap().as_array().unwrap();
        assert_eq!(af_entries, &[Object::Reference(filespec_id)]);

        let filespec = doc.get_object(filespec_id).unwrap().as_dict().unwrap();
        assert!(
            matches!(filespec.get(b"Type").unwrap(), Object::Name(name) if name == b"Filespec")
        );
        assert!(
            matches!(filespec.get(b"AFRelationship").unwrap(), Object::Name(name) if name == b"Alternative")
        );
        assert!(filespec.get(b"EF").unwrap().as_dict().unwrap().has(b"F"));

        let metadata_id = match catalog.get(b"Metadata").unwrap() {
            Object::Reference(id) => Some(*id),
            _ => None,
        }
        .expect("catalog Metadata should be an indirect stream");
        let metadata = doc.get_object(metadata_id).unwrap().as_stream().unwrap();
        let xmp = String::from_utf8(metadata.content.clone()).unwrap();
        assert!(xmp.contains("pdfaExtension:schemas"));
        assert!(xmp.contains("<fx:ConformanceLevel>EN 16931</fx:ConformanceLevel>"));
    }

    #[test]
    fn embed_factur_x_preserves_existing_pdfa_xmp_names_and_af_entries() {
        let pdf = synthesize_pdf_with_existing_catalog_entries();
        let xml = xml_for_profile(super::ZugferdProfile::Basic, 0);
        let patched = embed_factur_x(&pdf, &xml, super::ZugferdProfile::Basic).unwrap();
        let doc = Document::load_mem(&patched).unwrap();
        let catalog_id = super::catalog_id(&doc).unwrap();
        let catalog = doc.get_object(catalog_id).unwrap().as_dict().unwrap();

        let metadata_id = match catalog.get(b"Metadata").unwrap() {
            Object::Reference(id) => Some(*id),
            _ => None,
        }
        .expect("catalog Metadata should remain an indirect stream");
        let metadata = doc.get_object(metadata_id).unwrap().as_stream().unwrap();
        let xmp = String::from_utf8(metadata.content.clone()).unwrap();
        assert_well_formed_xml(&xmp);
        assert!(xmp.contains("<pdfaid:part>3</pdfaid:part>"));
        assert!(xmp.contains("<fx:ConformanceLevel>BASIC</fx:ConformanceLevel>"));

        let names_id = match catalog.get(b"Names").unwrap() {
            Object::Reference(id) => Some(*id),
            _ => None,
        }
        .expect("catalog Names should remain an indirect object");
        let names = doc.get_object(names_id).unwrap().as_dict().unwrap();
        let embedded_files = names.get(b"EmbeddedFiles").unwrap().as_dict().unwrap();
        let name_entries = embedded_files.get(b"Names").unwrap().as_array().unwrap();
        assert_eq!(name_entries.len(), 4);
        assert!(name_entries
            .iter()
            .any(|entry| is_string(entry, b"existing.xml")));
        assert!(name_entries
            .iter()
            .any(|entry| is_string(entry, b"factur-x.xml")));

        let af_entries = catalog.get(b"AF").unwrap().as_array().unwrap();
        assert_eq!(af_entries.len(), 2);
    }

    #[test]
    fn embed_factur_x_preserves_compressed_existing_pdfa_xmp() {
        let pdf = synthesize_pdf_with_compressed_catalog_metadata();
        let xml = xml_for_profile(super::ZugferdProfile::En16931, 4);
        let patched = embed_factur_x(&pdf, &xml, super::ZugferdProfile::En16931).unwrap();
        let doc = Document::load_mem(&patched).unwrap();
        let catalog = doc.catalog().unwrap();
        let metadata_id = catalog.get(b"Metadata").unwrap().as_reference().unwrap();
        let metadata = doc.get_object(metadata_id).unwrap().as_stream().unwrap();
        let xmp = String::from_utf8(metadata.get_plain_content().unwrap()).unwrap();
        assert_well_formed_xml(&xmp);
        assert!(xmp.contains("<pdfaid:part>3</pdfaid:part>"));
        assert!(xmp.contains("<pdfaid:conformance>U</pdfaid:conformance>"));
        assert!(xmp.contains("<fx:ConformanceLevel>EN 16931</fx:ConformanceLevel>"));
    }

    fn is_string(object: &Object, expected: &[u8]) -> bool {
        matches!(object, Object::String(value, _) if value == expected)
    }

    #[test]
    fn embed_factur_x_is_byte_deterministic() {
        let pdf = synthesize_pdf_for_profile(0);
        let xml = xml_for_profile(super::ZugferdProfile::Basic, 0);
        let first = embed_factur_x(&pdf, &xml, super::ZugferdProfile::Basic).unwrap();
        let second = embed_factur_x(&pdf, &xml, super::ZugferdProfile::Basic).unwrap();
        assert_eq!(
            first, second,
            "post-processor must be byte-deterministic so the release pipeline can hash output"
        );
    }

    #[test]
    fn embed_factur_x_rejects_garbage_input() {
        let err = embed_factur_x(b"not a pdf", b"<x/>", super::ZugferdProfile::Basic).unwrap_err();
        assert!(matches!(err, super::PostprocError::Parse(_)));
    }
}
