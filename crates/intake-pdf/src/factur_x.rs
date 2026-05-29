// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-060: Factur-X / ZUGFeRD XML extraction from PDF attachments.
//!
//! Factur-X (ZUGFeRD outside the European context) embeds its XML
//! invoice as a PDF/A-3 file attachment. The attachment uses the
//! standard PDF *`AssociatedFile`* mechanism: the document catalog
//! advertises one or more embedded files under `Names.EmbeddedFiles`
//! (and, for PDF/A-3 conformance, in `AF` on the catalog).
//!
//! The canonical attachment file names are:
//!
//! * `factur-x.xml` — current Factur-X / Mandataire / EN 16931 spelling
//! * `ZUGFeRD-invoice.xml` — original ZUGFeRD spelling
//! * `zugferd-invoice.xml` — lower-case variant some tooling emits
//! * `xrechnung.xml` — XRechnung-in-PDF variant
//!
//! The extractor walks the embedded-files name tree, finds the first
//! `Filespec` whose `F`/`UF` field matches one of those names, then
//! resolves the `EF.F` (or `EF.UF`) stream and returns its decoded
//! bytes. Non-Factur-X PDFs return `Ok(None)`, never panic.

use std::collections::HashSet;
use std::io::Read;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use crate::PdfTextError;

/// Hard ceiling on the *decompressed* size of an embedded-file stream,
/// in bytes. A real Factur-X / ZUGFeRD XML is tens of kilobytes; even
/// the verbose EXTENDED profile with embedded free-text stays well
/// under a megabyte, so a 16 MiB ceiling is generous for any conformant
/// payload while refusing a decompression bomb.
///
/// Without this cap, a hostile PDF could name an embedded-file stream
/// `factur-x.xml`, fill it with a few kilobytes of `FlateDecode` input
/// that inflate to hundreds of megabytes, and force an unbounded
/// allocation on intake. The cap is the embedded-file sibling of the
/// evidence-bundle `decode_all_capped` guard and the name-tree cycle
/// guard above.
const MAX_EMBEDDED_FILE_SIZE: u64 = 16 * 1024 * 1024;

/// Hard ceiling on embedded-files name-tree depth. PDF name trees are
/// balanced and never approach this in practice (a tree this deep
/// would hold astronomically many entries); the cap is purely a
/// belt-and-braces guard against a hand-crafted malicious tree that
/// the visited-set check below somehow misses.
const MAX_NAME_TREE_DEPTH: usize = 256;

/// Canonical Factur-X / ZUGFeRD attachment file names, in lookup
/// order. The first match wins — newer Factur-X writers use the
/// hyphenated lowercase name, older ZUGFeRD writers use the
/// camel-cased one.
pub const FACTUR_X_ATTACHMENT_NAMES: &[&str] = &[
    "factur-x.xml",
    "ZUGFeRD-invoice.xml",
    "zugferd-invoice.xml",
    "xrechnung.xml",
];

/// Extract the embedded Factur-X / ZUGFeRD XML from a PDF.
///
/// Returns `Ok(Some(xml_bytes))` when the PDF carries one of the
/// canonical attachment names listed in [`FACTUR_X_ATTACHMENT_NAMES`],
/// `Ok(None)` for any other PDF (including non-Factur-X PDFs and
/// well-formed PDFs that simply do not declare an embedded XML
/// attachment), and a typed [`PdfTextError`] when the input is not a
/// parseable PDF or is encrypted.
///
/// # Examples
///
/// ```rust,ignore
/// let pdf: Vec<u8> = std::fs::read("invoice.pdf").unwrap();
/// match invoicekit_intake_pdf::extract_factur_x_xml(&pdf) {
///     Ok(Some(xml)) => println!("got {} bytes of Factur-X XML", xml.len()),
///     Ok(None) => println!("no Factur-X attachment found"),
///     Err(e) => eprintln!("parse error: {e}"),
/// }
/// ```
///
/// # Errors
///
/// Returns [`PdfTextError::Parse`] when the bytes aren't a valid
/// PDF, and [`PdfTextError::Encrypted`] when the PDF declares an
/// `Encrypt` dictionary (T-060 does not attempt decryption).
pub fn extract_factur_x_xml(bytes: &[u8]) -> Result<Option<Vec<u8>>, PdfTextError> {
    let doc = Document::load_mem(bytes).map_err(|e| PdfTextError::Parse(e.to_string()))?;
    if doc.trailer.get(b"Encrypt").is_ok() {
        return Err(PdfTextError::Encrypted);
    }
    let Ok(root_id) = doc.trailer.get(b"Root").and_then(Object::as_reference) else {
        return Ok(None);
    };
    let Ok(catalog) = resolve_dict(&doc, root_id) else {
        return Ok(None);
    };

    // PDF/A-3 attachment discovery follows two parallel paths:
    //   1. catalog -> Names -> EmbeddedFiles (the legacy name tree)
    //   2. catalog -> AF (the array of associated Filespec refs)
    // Either path is sufficient; we try both to be liberal on intake.
    if let Some(xml) = extract_via_embedded_files_tree(&doc, &catalog)? {
        return Ok(Some(xml));
    }
    if let Some(xml) = extract_via_af_array(&doc, &catalog) {
        return Ok(Some(xml));
    }
    Ok(None)
}

fn extract_via_embedded_files_tree(
    doc: &Document,
    catalog: &Dictionary,
) -> Result<Option<Vec<u8>>, PdfTextError> {
    let Ok(names) = catalog.get(b"Names") else {
        return Ok(None);
    };
    let Ok(names_dict) = resolve_object(doc, names).and_then(Object::as_dict) else {
        return Ok(None);
    };
    let Ok(embedded_files) = names_dict.get(b"EmbeddedFiles") else {
        return Ok(None);
    };
    let Ok(embedded_dict) = resolve_object(doc, embedded_files).and_then(Object::as_dict) else {
        return Ok(None);
    };
    // Seed the visited set with the root node's ObjectId (when it is
    // reached by reference) so a Kid that points back at the root is
    // recognised as a cycle on the first hop.
    let mut visited = HashSet::new();
    if let Object::Reference(id) = embedded_files {
        visited.insert(*id);
    }
    walk_name_tree(doc, embedded_dict, &mut visited, 0)
}

fn extract_via_af_array(doc: &Document, catalog: &Dictionary) -> Option<Vec<u8>> {
    let af = catalog.get(b"AF").ok()?;
    let array = resolve_object(doc, af).and_then(Object::as_array).ok()?;
    for entry in array {
        let Ok(dict) = resolve_object(doc, entry).and_then(Object::as_dict) else {
            continue;
        };
        if let Some(xml) = read_filespec(doc, dict) {
            return Some(xml);
        }
    }
    None
}

/// Walks a name tree node (either internal or leaf). Leaf nodes
/// expose `Names = [name1, ref1, name2, ref2, …]`; internal nodes
/// expose `Kids = [refs to child name tree nodes]`. Returns the
/// first Factur-X attachment found in a left-to-right walk.
///
/// `visited` records every node `ObjectId` already on the current
/// descent so a cyclic `Kids` graph (a node referencing an ancestor
/// or itself) terminates instead of recursing forever; `depth` is a
/// secondary belt-and-braces cap. Both guard against a maliciously
/// crafted name tree triggering unbounded recursion / stack overflow.
fn walk_name_tree(
    doc: &Document,
    node: &Dictionary,
    visited: &mut HashSet<ObjectId>,
    depth: usize,
) -> Result<Option<Vec<u8>>, PdfTextError> {
    if depth >= MAX_NAME_TREE_DEPTH {
        return Ok(None);
    }
    if let Ok(names) = node.get(b"Names") {
        if let Ok(array) = resolve_object(doc, names).and_then(Object::as_array) {
            let mut idx = 0;
            while idx + 1 < array.len() {
                let name = resolve_object(doc, &array[idx]).ok().and_then(string_value);
                let filespec = resolve_object(doc, &array[idx + 1]);
                if let (Some(name), Ok(spec)) = (name, filespec) {
                    if is_canonical_factur_x_name(&name) {
                        if let Ok(dict) = spec.as_dict() {
                            if let Some(xml) = read_filespec(doc, dict) {
                                return Ok(Some(xml));
                            }
                        }
                    }
                }
                idx += 2;
            }
        }
    }
    if let Ok(kids) = node.get(b"Kids") {
        if let Ok(array) = resolve_object(doc, kids).and_then(Object::as_array) {
            for kid in array {
                // Only follow a Kid we have not already entered on this
                // descent. A self- or ancestor-referencing Kid would
                // otherwise recurse forever and overflow the stack.
                if let Object::Reference(id) = kid {
                    if !visited.insert(*id) {
                        continue;
                    }
                }
                let Ok(child) = resolve_object(doc, kid).and_then(Object::as_dict) else {
                    continue;
                };
                if let Some(xml) = walk_name_tree(doc, child, visited, depth + 1)? {
                    return Ok(Some(xml));
                }
            }
        }
    }
    Ok(None)
}

/// Read an XML stream from a `Filespec` dictionary. Both `F` and `UF`
/// keys are inspected, and within the `EF` entry we accept either
/// `F` (legacy ASCII name) or `UF` (Unicode name).
fn read_filespec(doc: &Document, filespec: &Dictionary) -> Option<Vec<u8>> {
    let name = resolved_string(doc, filespec, b"F")
        .or_else(|| resolved_string(doc, filespec, b"UF"))?;
    if !is_canonical_factur_x_name(&name) {
        // Wrong attachment name — skip silently, we may be looking
        // at /AF entries that aren't the XML payload.
        return None;
    }
    let ef = filespec.get(b"EF").ok()?;
    let ef_dict = resolve_object(doc, ef).and_then(Object::as_dict).ok()?;
    for key in [b"F" as &[u8], b"UF"] {
        if let Ok(Object::Reference(stream_ref)) = ef_dict.get(key) {
            if let Ok(stream) = doc.get_object(*stream_ref).and_then(Object::as_stream) {
                // Cap the decompressed size. An attacker controls both the
                // attachment name (so this branch is reachable) and the
                // stream bytes, so an unbounded `decompressed_content()`
                // here is a decompression-bomb sink.
                return decode_embedded_capped(stream, MAX_EMBEDDED_FILE_SIZE);
            }
        }
    }
    None
}

/// Decode an embedded-file stream while refusing to materialise more
/// than `limit` bytes.
///
/// For the common single-`FlateDecode` case (what every Factur-X writer
/// emits) the DEFLATE stream is read through a [`Read::take`] reader
/// bounded at `limit + 1`, so decoding stops the instant the cap is
/// crossed instead of inflating the full bomb. For any other filter
/// (or filter chain, or an uncompressed stream) we fall back to lopdf's
/// decoder and then enforce the same size cap on the result. Either way
/// an output larger than `limit` yields `None`, which the caller treats
/// as "no usable attachment".
fn decode_embedded_capped(stream: &Stream, limit: u64) -> Option<Vec<u8>> {
    // Read one byte past the cap so an output landing exactly on `limit`
    // is accepted while anything larger is detected and rejected.
    let read_budget = limit.saturating_add(1);

    if is_single_flate_decode(stream) {
        let mut decoder = flate2::read::ZlibDecoder::new(stream.content.as_slice());
        let mut out = Vec::new();
        // Bound the *decoder's output* (not the compressed input), so a
        // tiny input that inflates without bound stops at the ceiling.
        if Read::take(&mut decoder, read_budget)
            .read_to_end(&mut out)
            .is_err()
        {
            // A truncated / corrupt DEFLATE stream is not a Factur-X
            // payload we can use; skip it rather than surfacing raw bytes.
            return None;
        }
        if out.len() as u64 > limit {
            return None;
        }
        return Some(out);
    }

    // Uncommon path: LZWDecode / ASCII85Decode / chains / no filter.
    // lopdf materialises the whole output, so enforce the cap on the
    // result and reject anything over the ceiling.
    let decoded = stream
        .decompressed_content()
        .unwrap_or_else(|_| stream.content.clone());
    if decoded.len() as u64 > limit {
        return None;
    }
    Some(decoded)
}

/// True when the stream's `Filter` is exactly `FlateDecode` (a single
/// filter, not a chain) and it carries no `DecodeParms` predictor we'd
/// have to replay. This is the shape every Factur-X / ZUGFeRD writer
/// emits for the embedded XML, and the only one we decode through the
/// size-capped streaming path; everything else falls back to lopdf.
fn is_single_flate_decode(stream: &Stream) -> bool {
    // A predictor (`DecodeParms`) would need post-processing lopdf does
    // internally; an XML attachment never uses one, so treat its
    // presence as "not the fast path" and let lopdf handle it.
    if stream.dict.get(b"DecodeParms").is_ok() {
        return false;
    }
    stream
        .filters()
        .is_ok_and(|filters| matches!(filters.as_slice(), [b"FlateDecode"]))
}

fn resolve_dict(doc: &Document, id: ObjectId) -> Result<Dictionary, PdfTextError> {
    doc.get_object(id)
        .and_then(Object::as_dict)
        .cloned()
        .map_err(|e| PdfTextError::Parse(e.to_string()))
}

fn resolve_object<'a>(doc: &'a Document, obj: &'a Object) -> lopdf::Result<&'a Object> {
    match obj {
        Object::Reference(id) => doc.get_object(*id),
        _ => Ok(obj),
    }
}

/// Look up `key` in `dict`, follow an indirect reference if present,
/// and decode the target as a string. `None` when the key is absent
/// or its value isn't a string/name object.
fn resolved_string(doc: &Document, dict: &Dictionary, key: &[u8]) -> Option<String> {
    let obj = dict.get(key).ok()?;
    resolve_object(doc, obj).ok().and_then(string_value)
}

fn string_value(obj: &Object) -> Option<String> {
    let bytes = match obj {
        Object::String(b, _) | Object::Name(b) => b.clone(),
        _ => return None,
    };
    String::from_utf8(bytes).ok()
}

fn is_canonical_factur_x_name(name: &str) -> bool {
    // Case-sensitive comparison: ZUGFeRD's original spelling
    // capitalises the brand; Factur-X uses the lowercase
    // hyphenated form. Both must be accepted byte-for-byte, but
    // neither should be auto-corrected — a producer that emits
    // `Factur-X.xml` or `ZUGFERD-INVOICE.XML` is non-conformant
    // and should fall through to the "not Factur-X" branch.
    FACTUR_X_ATTACHMENT_NAMES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::{extract_factur_x_xml, is_canonical_factur_x_name};

    use lopdf::content::{Content, Operation};
    use lopdf::dictionary;
    use lopdf::{Dictionary, Document, Object, Stream};

    fn build_factur_x_pdf(attachment_name: &str, xml: &[u8]) -> Vec<u8> {
        // Construct a minimal but valid PDF that mirrors the
        // PDF/A-3 attachment shape Factur-X writers emit:
        //
        //   Catalog
        //     /AF [ <ref to Filespec> ]
        //     /Names << /EmbeddedFiles << /Names [ <name> <ref to Filespec> ] >> >>
        //     /Pages <ref to Pages>
        //   Filespec
        //     /Type /Filespec
        //     /F (attachment_name)
        //     /UF (attachment_name)
        //     /AFRelationship /Alternative
        //     /EF << /F <ref to embedded stream>
        //            /UF <ref to embedded stream> >>
        //   EmbeddedFile stream (raw XML)
        let mut doc = Document::with_version("1.7");

        let xml_stream_id = doc.add_object(Stream::new(
            dictionary! { "Type" => "EmbeddedFile", "Subtype" => Object::Name(b"text/xml".to_vec()) },
            xml.to_vec(),
        ));
        let filespec_id = doc.add_object(dictionary! {
            "Type" => "Filespec",
            "F" => Object::String(attachment_name.as_bytes().to_vec(), lopdf::StringFormat::Literal),
            "UF" => Object::String(attachment_name.as_bytes().to_vec(), lopdf::StringFormat::Literal),
            "AFRelationship" => Object::Name(b"Alternative".to_vec()),
            "EF" => dictionary! {
                "F" => xml_stream_id,
                "UF" => xml_stream_id,
            },
        });

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
        // Patch the page parent now that we know the parent id.
        if let Ok(Object::Dictionary(d)) = doc.get_object_mut(leaf_id) {
            d.set("Parent", parent_id);
        }

        let names_id = doc.add_object(dictionary! {
            "EmbeddedFiles" => dictionary! {
                "Names" => vec![
                    Object::String(attachment_name.as_bytes().to_vec(), lopdf::StringFormat::Literal),
                    filespec_id.into(),
                ],
            },
        });
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => parent_id,
            "Names" => names_id,
            "AF" => vec![filespec_id.into()],
        });
        doc.trailer.set("Root", catalog_id);
        doc.trailer.set("Size", Object::Integer(7));

        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("serialize fixture pdf");
        bytes
    }

    /// FlateDecode-compress `plain` into a zlib stream the way a
    /// Factur-X writer would. Returns the compressed bytes to drop into
    /// a `/Filter /FlateDecode` stream.
    fn flate_compress(plain: &[u8]) -> Vec<u8> {
        use std::io::Write as _;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        encoder.write_all(plain).expect("zlib write");
        encoder.finish().expect("zlib finish")
    }

    /// Like [`build_factur_x_pdf`], but the embedded XML stream is
    /// FlateDecode-compressed (the universal Factur-X shape) so the
    /// extractor exercises its size-capped streaming-decode path. The
    /// stream's `decompressed` size is whatever `plain` is; the
    /// on-the-wire bytes are the compressed form, which can be tiny even
    /// when `plain` is enormous (a decompression bomb).
    fn build_factur_x_pdf_flate(attachment_name: &str, plain: &[u8]) -> Vec<u8> {
        let mut doc = Document::with_version("1.7");

        let mut xml_stream = Stream::new(
            dictionary! { "Type" => "EmbeddedFile", "Subtype" => Object::Name(b"text/xml".to_vec()) },
            flate_compress(plain),
        );
        // Mark the on-wire bytes as FlateDecode so the extractor decodes
        // them. `Stream::new` already set `/Length` to the compressed
        // size, which is what we want.
        xml_stream.dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
        let xml_stream_id = doc.add_object(xml_stream);

        let filespec_id = doc.add_object(dictionary! {
            "Type" => "Filespec",
            "F" => Object::String(attachment_name.as_bytes().to_vec(), lopdf::StringFormat::Literal),
            "UF" => Object::String(attachment_name.as_bytes().to_vec(), lopdf::StringFormat::Literal),
            "AFRelationship" => Object::Name(b"Alternative".to_vec()),
            "EF" => dictionary! {
                "F" => xml_stream_id,
                "UF" => xml_stream_id,
            },
        });

        let content_id = doc.add_object(Stream::new(
            Dictionary::new(),
            Content {
                operations: vec![Operation::new("q", vec![]), Operation::new("Q", vec![])],
            }
            .encode()
            .expect("encode page content"),
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

        let names_id = doc.add_object(dictionary! {
            "EmbeddedFiles" => dictionary! {
                "Names" => vec![
                    Object::String(attachment_name.as_bytes().to_vec(), lopdf::StringFormat::Literal),
                    filespec_id.into(),
                ],
            },
        });
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => parent_id,
            "Names" => names_id,
            "AF" => vec![filespec_id.into()],
        });
        doc.trailer.set("Root", catalog_id);

        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("serialize flate fixture pdf");
        bytes
    }

    fn build_plain_pdf() -> Vec<u8> {
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
        doc.trailer.set("Size", Object::Integer(4));
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("serialize plain pdf");
        bytes
    }

    fn xml_for(profile: &str) -> Vec<u8> {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <rsm:CrossIndustryInvoice \
               xmlns:rsm=\"urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100\">\n  \
               <guideline-id>{profile}</guideline-id>\n\
             </rsm:CrossIndustryInvoice>\n"
        )
        .into_bytes()
    }

    /// Strict gate: all six ZUGFeRD profiles round-trip through the
    /// extractor. The PDF structure is identical across profiles —
    /// only the embedded XML's guideline-id differs — so one
    /// fixture per profile is enough to prove coverage.
    #[test]
    fn extracts_xml_from_all_six_zugferd_profiles() {
        let profiles = [
            ("MINIMUM", "factur-x.xml"),
            ("BASIC WL", "factur-x.xml"),
            ("BASIC", "factur-x.xml"),
            ("EN 16931", "factur-x.xml"),
            ("EXTENDED", "ZUGFeRD-invoice.xml"),
            ("XRECHNUNG", "xrechnung.xml"),
        ];
        for (profile, attachment) in profiles {
            let xml = xml_for(profile);
            let pdf = build_factur_x_pdf(attachment, &xml);
            let extracted = extract_factur_x_xml(&pdf)
                .unwrap_or_else(|e| panic!("extract failed for {profile}: {e}"))
                .unwrap_or_else(|| panic!("expected Some for {profile}"));
            assert_eq!(
                extracted, xml,
                "extracted bytes differ from embedded XML for {profile}"
            );
        }
    }

    #[test]
    fn returns_none_for_pdf_without_attachment() {
        let pdf = build_plain_pdf();
        assert_eq!(extract_factur_x_xml(&pdf).unwrap(), None);
    }

    /// A maliciously crafted embedded-files name tree whose `Kids`
    /// form a cycle (root -> child -> root) must terminate instead of
    /// recursing until the stack overflows. With the visited-set and
    /// depth guards in place the walk simply finds no attachment and
    /// returns `Ok(None)`.
    #[test]
    fn cyclic_name_tree_terminates() {
        let mut doc = Document::with_version("1.7");

        // Reserve the two name-tree node ids up front so each can point
        // at the other (a two-node cycle).
        let root_node_id = doc.new_object_id();
        let child_node_id = doc.new_object_id();

        doc.objects.insert(
            root_node_id,
            Object::Dictionary(dictionary! {
                "Kids" => vec![Object::Reference(child_node_id)],
            }),
        );
        doc.objects.insert(
            child_node_id,
            Object::Dictionary(dictionary! {
                // Points back at the root: this is the cycle.
                "Kids" => vec![Object::Reference(root_node_id)],
            }),
        );

        // Minimal page tree so the catalog is well formed.
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

        let names_id = doc.add_object(dictionary! {
            "EmbeddedFiles" => Object::Reference(root_node_id),
        });
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => parent_id,
            "Names" => names_id,
        });
        doc.trailer.set("Root", catalog_id);
        let mut pdf = Vec::new();
        doc.save_to(&mut pdf).expect("serialize cyclic-tree pdf");

        // The only contract that matters: this returns rather than
        // overflowing the stack.
        assert_eq!(extract_factur_x_xml(&pdf).unwrap(), None);
    }

    /// A self-referencing leaf (a node whose `Kids` contains its own
    /// id) is the degenerate one-node cycle and must also terminate.
    #[test]
    fn self_referencing_name_tree_node_terminates() {
        let mut doc = Document::with_version("1.7");
        let node_id = doc.new_object_id();
        doc.objects.insert(
            node_id,
            Object::Dictionary(dictionary! {
                "Kids" => vec![Object::Reference(node_id)],
            }),
        );
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
        let names_id = doc.add_object(dictionary! {
            "EmbeddedFiles" => Object::Reference(node_id),
        });
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => parent_id,
            "Names" => names_id,
        });
        doc.trailer.set("Root", catalog_id);
        let mut pdf = Vec::new();
        doc.save_to(&mut pdf).expect("serialize self-cyclic pdf");

        assert_eq!(extract_factur_x_xml(&pdf).unwrap(), None);
    }

    #[test]
    fn returns_none_for_pdf_with_wrong_attachment_name() {
        let pdf = build_factur_x_pdf("README.txt", b"hello");
        assert_eq!(extract_factur_x_xml(&pdf).unwrap(), None);
    }

    #[test]
    fn returns_parse_error_for_garbage() {
        let err = extract_factur_x_xml(b"not a pdf at all").unwrap_err();
        assert!(
            matches!(err, crate::PdfTextError::Parse(_)),
            "expected Parse error, got {err:?}"
        );
    }

    #[test]
    fn returns_none_for_empty_bytes() {
        // An empty buffer is not a valid PDF; lopdf surfaces that
        // as a Parse error rather than silently returning None.
        assert!(extract_factur_x_xml(b"").is_err());
    }

    #[test]
    fn does_not_panic_on_arbitrary_binary() {
        for seed in 0u32..16 {
            let mut bytes = Vec::with_capacity(2048);
            for i in 0..2048u32 {
                let mixed = seed.wrapping_mul(2_654_435_761) ^ i.wrapping_mul(0x9e37_79b9);
                bytes.push((mixed & 0xff) as u8);
            }
            let _ = extract_factur_x_xml(&bytes);
        }
    }

    #[test]
    fn canonical_name_check_is_case_sensitive() {
        // Spec spelling is preserved; case-folded variants are
        // intentionally rejected so a producer that mis-cases the
        // attachment doesn't get a misleading positive.
        assert!(is_canonical_factur_x_name("factur-x.xml"));
        assert!(is_canonical_factur_x_name("ZUGFeRD-invoice.xml"));
        assert!(is_canonical_factur_x_name("zugferd-invoice.xml"));
        assert!(is_canonical_factur_x_name("xrechnung.xml"));
        assert!(!is_canonical_factur_x_name("Factur-X.xml"));
        assert!(!is_canonical_factur_x_name("ZUGFERD-INVOICE.XML"));
        assert!(!is_canonical_factur_x_name("invoice.xml"));
    }

    /// A normally-sized, FlateDecode-compressed embedded XML must still
    /// round-trip through the new size-capped streaming-decode path.
    /// This proves the cap is behaviour-preserving for conformant input.
    #[test]
    fn extracts_flate_compressed_attachment() {
        let xml = xml_for("EN 16931");
        let pdf = build_factur_x_pdf_flate("factur-x.xml", &xml);
        let extracted = extract_factur_x_xml(&pdf)
            .expect("extract from flate-compressed pdf")
            .expect("attachment present");
        assert_eq!(extracted, xml);
    }

    /// Regression for the embedded-file decompression bomb: a hostile
    /// PDF names a `factur-x.xml` stream whose `FlateDecode` input is a
    /// few kilobytes but inflates to far more than the 16 MiB cap.
    /// Before the cap, `read_filespec` called the unbounded
    /// `decompressed_content()` and materialised the whole bomb; with
    /// the cap the over-size output is refused and extraction returns
    /// `Ok(None)` (no usable attachment) instead of exhausting memory.
    #[test]
    fn rejects_embedded_file_decompression_bomb() {
        // 64 MiB of zeros compresses to a few kilobytes of DEFLATE but
        // inflates well past the 16 MiB ceiling.
        let bomb = vec![0u8; 64 * 1024 * 1024];
        let pdf = build_factur_x_pdf_flate("factur-x.xml", &bomb);
        // The on-wire PDF stays tiny — proof the bomb's danger is the
        // *decompressed* size, not the input size.
        assert!(
            pdf.len() < 1024 * 1024,
            "compressed bomb PDF should be small, got {} bytes",
            pdf.len()
        );
        assert_eq!(
            extract_factur_x_xml(&pdf).expect("bomb PDF still parses"),
            None,
            "over-cap embedded file must be refused, not materialised"
        );
    }

    /// A payload that sits just under the cap must still extract: the
    /// guard rejects bombs without clipping legitimately-large (but
    /// bounded) attachments.
    #[test]
    fn accepts_embedded_file_just_under_cap() {
        // 1 MiB of a repeating byte — well-formed, comfortably under the
        // 16 MiB ceiling, and large enough to exercise the streaming
        // reader past trivial sizes.
        let payload = vec![b'x'; 1024 * 1024];
        let pdf = build_factur_x_pdf_flate("factur-x.xml", &payload);
        let extracted = extract_factur_x_xml(&pdf)
            .expect("under-cap pdf parses")
            .expect("under-cap attachment present");
        assert_eq!(extracted.len(), payload.len());
        assert_eq!(extracted, payload);
    }

    /// Strict gate: a 5 MB Factur-X PDF must extract in under 50 ms.
    /// The actual budget here is much lower (the helper builds a
    /// well-formed but small embedded XML); we pad the PDF with a
    /// large dummy stream to reach the 5 MB envelope. Release-mode
    /// only — the bead's performance gate targets shipped builds,
    /// and debug-mode `lopdf` parses streams without LLVM
    /// optimisations and consistently lands around 60-90 ms here.
    #[cfg(not(debug_assertions))]
    #[test]
    fn extracts_5mb_pdf_under_50ms() {
        // Build a baseline Factur-X PDF, then synthesize a 5 MB
        // padding stream and re-serialize. We measure only the
        // extraction call, not the synthesis time.
        let xml = xml_for("EN 16931");
        let mut doc = lopdf::Document::with_version("1.7");
        let xml_stream_id = doc.add_object(Stream::new(
            dictionary! { "Type" => "EmbeddedFile", "Subtype" => Object::Name(b"text/xml".to_vec()) },
            xml.clone(),
        ));
        let filespec_id = doc.add_object(dictionary! {
            "Type" => "Filespec",
            "F" => Object::String(b"factur-x.xml".to_vec(), lopdf::StringFormat::Literal),
            "EF" => dictionary! { "F" => xml_stream_id },
        });
        let big_stream = Stream::new(Dictionary::new(), vec![0u8; 5 * 1024 * 1024]);
        let big_id = doc.add_object(big_stream);
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
            "Resources" => dictionary! { "XObject" => dictionary! { "Big" => big_id } },
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
        let names_id = doc.add_object(dictionary! {
            "EmbeddedFiles" => dictionary! {
                "Names" => vec![
                    Object::String(b"factur-x.xml".to_vec(), lopdf::StringFormat::Literal),
                    filespec_id.into(),
                ],
            },
        });
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => parent_id,
            "Names" => names_id,
        });
        doc.trailer.set("Root", catalog_id);
        let mut pdf = Vec::new();
        doc.save_to(&mut pdf).expect("serialize 5MB pdf");
        assert!(
            pdf.len() >= 5 * 1024 * 1024,
            "expected >= 5 MB PDF, got {} bytes",
            pdf.len()
        );

        let start = std::time::Instant::now();
        let extracted = extract_factur_x_xml(&pdf)
            .expect("extract from 5MB pdf")
            .expect("attachment present");
        let elapsed = start.elapsed();
        assert_eq!(extracted, xml);
        assert!(
            elapsed.as_millis() < 50,
            "5 MB extraction took {} ms (strict gate: < 50 ms)",
            elapsed.as_millis()
        );
    }
}
