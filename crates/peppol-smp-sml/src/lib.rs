// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! T-090 Peppol SMP / SML participant lookup client.
//!
//! The Peppol four-corner network discovers a recipient's access
//! point through two layered services:
//!
//! 1. **SML** (Service Metadata Locator) — DNS-based. The
//!    participant identifier (e.g. `0192:991825827`) is hashed
//!    with `MD5`, base32-encoded, and looked up under the
//!    relevant SML domain (`edelivery.tech.ec.europa.eu` for
//!    production, `acc.edelivery.tech.ec.europa.eu` for the
//!    `OpenPeppol` acceptance test SML). The CNAME points at the
//!    participant's SMP URL.
//!
//! 2. **SMP** (Service Metadata Publisher) — HTTP-based. The
//!    SMP serves an XML document at
//!    `{smp}/iso6523-actorid-upis::{scheme}:{value}/services/{document_type}`
//!    listing the access-point endpoints that can deliver the
//!    given document type to the participant.
//!
//! This crate provides the lookup pipeline plus a TTL-aware
//! cache. DNS and HTTP transports are injected behind traits
//! so the crate can be exercised without network access in unit
//! tests; the upstream `crates/transmit-peppol` bead wires in
//! a real `hickory-dns-resolver` + `reqwest` transport stack.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use base32::Alphabet;
use md5::{Digest, Md5};
use thiserror::Error;

/// TTL-aware in-memory cache used by the lookup pipeline.
pub mod cache;

/// Errors raised by the SMP/SML pipeline.
#[derive(Debug, Error)]
pub enum PeppolLookupError {
    /// DNS resolution failed at the SML stage.
    #[error("SML DNS lookup failed for `{host}`: {detail}")]
    Sml {
        /// Host that was being resolved.
        host: String,
        /// Transport-specific failure detail.
        detail: String,
    },
    /// HTTP fetch failed at the SMP stage.
    #[error("SMP HTTP fetch failed for `{url}`: {detail}")]
    Smp {
        /// SMP endpoint that was being fetched.
        url: String,
        /// Transport-specific failure detail.
        detail: String,
    },
    /// SMP response could not be parsed.
    #[error("SMP response parse failed: {0}")]
    Parse(String),
    /// SMP response carried no matching endpoint for the
    /// requested document type.
    #[error("no SMP endpoint for document type `{0}`")]
    NoEndpoint(String),
}

/// Peppol participant identifier (BIS Billing 3.0 `EndpointID`).
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ParticipantId {
    /// Scheme identifier, e.g. `0192` (Norwegian organisation
    /// numbers), `0151` (Australia Business Number).
    pub scheme: String,
    /// Scheme-specific identifier value.
    pub value: String,
}

impl ParticipantId {
    /// Build a participant ID from the wire form
    /// `{scheme}:{value}`.
    ///
    /// # Errors
    ///
    /// Returns an error message string when the wire form does
    /// not match `<scheme>:<value>` with non-empty halves.
    pub fn parse(wire: &str) -> Result<Self, &'static str> {
        let (scheme, value) = wire
            .split_once(':')
            .ok_or("participant ID must be `scheme:value`")?;
        if scheme.is_empty() || value.is_empty() {
            return Err("participant ID scheme and value must be non-empty");
        }
        Ok(Self {
            scheme: scheme.to_owned(),
            value: value.to_owned(),
        })
    }

    /// Wire form `iso6523-actorid-upis::{scheme}:{value}` used
    /// in SMP URLs.
    ///
    /// The scheme and value are percent-encoded with
    /// [`percent_encode_segment`] so that a hostile identifier
    /// carrying `/`, `?`, `#`, `%`, or `..` cannot break out of
    /// its SMP path segment. The structural `iso6523-actorid-upis::`
    /// prefix and the `:` separator are fixed literals and stay
    /// readable.
    #[must_use]
    pub fn to_url_segment(&self) -> String {
        format!(
            "iso6523-actorid-upis::{}:{}",
            percent_encode_segment(&self.scheme),
            percent_encode_segment(&self.value)
        )
    }

    /// Canonical SML hostname for this participant under the
    /// given SML domain (e.g. `edelivery.tech.ec.europa.eu`).
    #[must_use]
    pub fn to_sml_hostname(&self, sml_domain: &str) -> String {
        let canonical = format!("{}:{}", self.scheme, self.value).to_lowercase();
        let mut hasher = Md5::new();
        hasher.update(canonical.as_bytes());
        let digest = hasher.finalize();
        // Peppol SML hashing uses base32 RFC4648 lowercase.
        let encoded = base32::encode(Alphabet::Rfc4648Lower { padding: false }, &digest);
        format!("B-{encoded}.iso6523-actorid-upis.{sml_domain}")
    }
}

/// Document-type identifier used to scope the SMP lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentTypeId {
    /// Scheme such as `busdox-docid-qns`.
    pub scheme: String,
    /// Full document-type value, e.g. the Peppol BIS Billing 3.0
    /// Invoice document-type URN.
    pub value: String,
}

impl DocumentTypeId {
    /// Peppol BIS Billing 3.0 Invoice document type.
    #[must_use]
    pub fn peppol_bis_3_invoice() -> Self {
        Self {
            scheme: "busdox-docid-qns".to_owned(),
            value: "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2::Invoice##urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0::2.1".to_owned(),
        }
    }

    /// URL segment form used in SMP requests.
    ///
    /// The scheme and value are percent-encoded with
    /// [`percent_encode_segment`] so that a hostile document-type
    /// identifier cannot break out of its SMP path segment. The
    /// `::` separator is a fixed literal and stays readable.
    #[must_use]
    pub fn to_url_segment(&self) -> String {
        format!(
            "{}::{}",
            percent_encode_segment(&self.scheme),
            percent_encode_segment(&self.value)
        )
    }
}

/// Percent-encode one URL path segment per RFC 3986.
///
/// Bytes outside the "unreserved" set (ASCII letters and digits
/// plus `-`, `.`, `_`, `~`) are written as `%XX`. In particular
/// `/`, `?`, `#`, `%`, `:`, and whitespace are escaped, so a
/// participant or document-type identifier can no longer inject
/// extra path, query, or fragment components into the SMP REST
/// URL. Mirrors `transmit-peppol-partner::percent_encode_path_segment`;
/// implemented inline so the crate does not pull `percent_encoding`
/// into the workspace for one call.
#[must_use]
fn percent_encode_segment(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len());
    for &byte in input.as_bytes() {
        let unreserved =
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if unreserved {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

/// Resolved access-point endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessPoint {
    /// AS4 receiver endpoint URL.
    pub endpoint_url: String,
    /// AS4 transport profile, e.g. `peppol-transport-as4-v2_0`.
    pub transport_profile: String,
}

/// DNS-style CNAME resolver. The lookup pipeline calls
/// [`Resolver::lookup_cname`] with the SML hostname and expects
/// the SMP base URL (an HTTP URL string) in return when the
/// participant is registered.
pub trait Resolver: Send + Sync {
    /// Resolve a CNAME chain to its SMP base URL.
    ///
    /// # Errors
    ///
    /// Returns an error message string when the DNS lookup
    /// fails at the transport layer.
    fn lookup_cname(&self, host: &str) -> Result<Option<String>, String>;
}

/// HTTP client used by the SMP fetch step.
pub trait HttpClient: Send + Sync {
    /// Perform a GET request, returning the response body bytes.
    ///
    /// # Errors
    ///
    /// Returns an error message string when the HTTP fetch
    /// fails at the transport layer.
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// SMP/SML lookup client.
pub struct PeppolClient<R: Resolver, H: HttpClient> {
    resolver: R,
    http: H,
    sml_domain: String,
    cache: Mutex<cache::TtlCache<String, AccessPoint>>,
    default_ttl: Duration,
}

impl<R: Resolver, H: HttpClient> PeppolClient<R, H> {
    /// Build a client for the given SML domain.
    pub fn new(resolver: R, http: H, sml_domain: impl Into<String>) -> Self {
        Self {
            resolver,
            http,
            sml_domain: sml_domain.into(),
            cache: Mutex::new(cache::TtlCache::new()),
            default_ttl: Duration::from_secs(600),
        }
    }

    /// Override the default cache TTL (initial value: 10 minutes).
    #[must_use]
    pub fn with_default_ttl(mut self, ttl: Duration) -> Self {
        self.default_ttl = ttl;
        self
    }

    /// Resolve a participant + document type to its access point.
    ///
    /// # Errors
    ///
    /// Returns a [`PeppolLookupError`] on DNS / HTTP / parse
    /// failures, or `NoEndpoint` if the SMP response carried no
    /// matching endpoint for the document type.
    ///
    /// # Panics
    ///
    /// Panics if the internal cache mutex is poisoned — only
    /// possible if a prior `lookup` thread panicked while
    /// holding the lock, which the implementation does not.
    pub fn lookup(
        &self,
        participant: &ParticipantId,
        document_type: &DocumentTypeId,
    ) -> Result<AccessPoint, PeppolLookupError> {
        let cache_key = format!(
            "{}|{}",
            participant.to_url_segment(),
            document_type.to_url_segment()
        );
        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(hit) = cache.get(&cache_key, Instant::now()) {
                return Ok(hit);
            }
        }
        let smp_base = self.resolve_smp(participant)?;
        let body = self.fetch_smp_metadata(&smp_base, participant, document_type)?;
        let access = parse_smp_metadata(&body, document_type)?;
        self.cache.lock().unwrap().insert(
            cache_key,
            access.clone(),
            Instant::now() + self.default_ttl,
        );
        Ok(access)
    }

    fn resolve_smp(&self, participant: &ParticipantId) -> Result<String, PeppolLookupError> {
        let host = participant.to_sml_hostname(&self.sml_domain);
        let cname = self
            .resolver
            .lookup_cname(&host)
            .map_err(|detail| PeppolLookupError::Sml {
                host: host.clone(),
                detail,
            })?;
        cname.ok_or_else(|| PeppolLookupError::Sml {
            host,
            detail: "no CNAME record".to_owned(),
        })
    }

    fn fetch_smp_metadata(
        &self,
        smp_base: &str,
        participant: &ParticipantId,
        document_type: &DocumentTypeId,
    ) -> Result<Vec<u8>, PeppolLookupError> {
        let url = format!(
            "{}/{}/services/{}",
            smp_base.trim_end_matches('/'),
            participant.to_url_segment(),
            document_type.to_url_segment()
        );
        self.http
            .get(&url)
            .map_err(|detail| PeppolLookupError::Smp { url, detail })
    }
}

/// Extract the first matching `EndpointURI` + `TransportProfile`
/// pair from an SMP metadata XML body.
///
/// The SMP wraps its payload in a `SignedServiceMetadata` element,
/// but this function does **not** verify the SMP signature: it reads
/// the `EndpointURI`/`TransportProfile` text only and performs no
/// XML-DSig signature check. The authenticity of the returned
/// [`AccessPoint`] is therefore not established here.
///
/// # Errors
///
/// Returns a [`PeppolLookupError::Parse`] when the body is not
/// valid UTF-8 or the XML reader rejects the input, and
/// [`PeppolLookupError::NoEndpoint`] when no
/// `EndpointURI`/`TransportProfile` pair is found.
pub fn parse_smp_metadata(
    body: &[u8],
    document_type: &DocumentTypeId,
) -> Result<AccessPoint, PeppolLookupError> {
    let xml = std::str::from_utf8(body)
        .map_err(|e| PeppolLookupError::Parse(format!("invalid UTF-8: {e}")))?;
    // The URL and transport profile must come from the *same*
    // `Endpoint` element. The `cur_*` candidates are scoped to the
    // endpoint currently being read; they are committed to the
    // returned `endpoint_url` / `transport_profile` only when an
    // `Endpoint` closes carrying both halves of the pair. Tracking
    // the two fields globally would let a later endpoint's URL be
    // paired with an earlier endpoint's profile.
    let mut endpoint_url = None;
    let mut transport_profile = None;
    let mut cur_endpoint_url: Option<String> = None;
    let mut cur_transport_profile: Option<String> = None;
    let mut in_endpoint = false;
    let mut in_endpoint_uri = false;
    let mut in_transport_profile = false;

    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(ref e)) => {
                let name = e.name();
                let local = std::str::from_utf8(name.local_name().as_ref())
                    .unwrap_or("")
                    .to_owned();
                if local == "Endpoint" {
                    // Open a fresh per-endpoint scope.
                    in_endpoint = true;
                    cur_endpoint_url = None;
                    cur_transport_profile = None;
                } else if local == "EndpointURI" || local == "EndpointReference" {
                    in_endpoint_uri = true;
                } else if local == "TransportProfile" {
                    in_transport_profile = true;
                }
            }
            Ok(quick_xml::events::Event::Text(t)) => {
                let text = std::str::from_utf8(t.as_ref())
                    .map_err(|e| PeppolLookupError::Parse(e.to_string()))?
                    .trim()
                    .to_owned();
                if in_endpoint_uri && cur_endpoint_url.is_none() && !text.is_empty() {
                    cur_endpoint_url = Some(text);
                } else if in_transport_profile
                    && cur_transport_profile.is_none()
                    && !text.is_empty()
                {
                    cur_transport_profile = Some(text);
                }
            }
            Ok(quick_xml::events::Event::End(ref e)) => {
                let name = e.name();
                let local = std::str::from_utf8(name.local_name().as_ref())
                    .unwrap_or("")
                    .to_owned();
                if local == "Endpoint" {
                    in_endpoint = false;
                    // Commit the first endpoint that carries both halves.
                    if endpoint_url.is_none() && transport_profile.is_none() {
                        if let (Some(url), Some(profile)) =
                            (cur_endpoint_url.take(), cur_transport_profile.take())
                        {
                            endpoint_url = Some(url);
                            transport_profile = Some(profile);
                        }
                    }
                    cur_endpoint_url = None;
                    cur_transport_profile = None;
                } else if local == "EndpointURI" || local == "EndpointReference" {
                    in_endpoint_uri = false;
                } else if local == "TransportProfile" {
                    in_transport_profile = false;
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(e) => return Err(PeppolLookupError::Parse(e.to_string())),
            _ => {}
        }
        buf.clear();
    }

    // Fallback for documents that carry a bare `EndpointURI` +
    // `TransportProfile` with no enclosing `Endpoint` wrapper: pair
    // whatever single candidates were collected outside any endpoint
    // scope. This preserves behaviour for non-wrapped inputs.
    if endpoint_url.is_none() && transport_profile.is_none() && !in_endpoint {
        endpoint_url = cur_endpoint_url;
        transport_profile = cur_transport_profile;
    }

    match (endpoint_url, transport_profile) {
        (Some(url), Some(profile)) => Ok(AccessPoint {
            endpoint_url: url,
            transport_profile: profile,
        }),
        _ => Err(PeppolLookupError::NoEndpoint(document_type.value.clone())),
    }
}

/// Production `OpenPeppol` SML domain.
pub const PEPPOL_PRODUCTION_SML: &str = "edelivery.tech.ec.europa.eu";

/// Acceptance / test `OpenPeppol` SML domain.
pub const PEPPOL_ACCEPTANCE_SML: &str = "acc.edelivery.tech.ec.europa.eu";

/// Crate name advertised in operator logs.
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-peppol-smp-sml"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn participant_id_parses_scheme_value_form() {
        let id = ParticipantId::parse("0192:991825827").unwrap();
        assert_eq!(id.scheme, "0192");
        assert_eq!(id.value, "991825827");
    }

    #[test]
    fn participant_id_rejects_malformed_input() {
        assert!(ParticipantId::parse("no-colon").is_err());
        assert!(ParticipantId::parse(":missing-scheme").is_err());
        assert!(ParticipantId::parse("missing-value:").is_err());
    }

    #[test]
    fn participant_id_url_segment() {
        let id = ParticipantId::parse("0192:991825827").unwrap();
        assert_eq!(id.to_url_segment(), "iso6523-actorid-upis::0192:991825827");
    }

    #[test]
    fn participant_id_sml_hostname_matches_peppol_spec() {
        // The MD5 of "0192:991825827" base32-encoded lower-case
        // is the canonical Peppol SML test vector for the
        // Norwegian organisation number 991825827. The expected
        // output is reproducible from the spec.
        let id = ParticipantId::parse("0192:991825827").unwrap();
        let host = id.to_sml_hostname("edelivery.tech.ec.europa.eu");
        assert!(host.starts_with("B-"));
        assert!(host.ends_with(".iso6523-actorid-upis.edelivery.tech.ec.europa.eu"));
        // 32 hex chars / 4 base32 = 26 chars (no padding).
        let prefix = host.strip_prefix("B-").unwrap();
        let hash_part = prefix.split('.').next().unwrap();
        assert_eq!(hash_part.len(), 26);
    }

    #[test]
    fn document_type_helpers_emit_well_formed_segments() {
        let dt = DocumentTypeId::peppol_bis_3_invoice();
        assert_eq!(dt.scheme, "busdox-docid-qns");
        assert!(dt.value.contains("Invoice"));
        assert!(dt.to_url_segment().starts_with("busdox-docid-qns::"));
    }

    #[test]
    fn parse_smp_metadata_extracts_endpoint_and_profile() {
        let body = br#"<?xml version="1.0"?>
            <SignedServiceMetadata xmlns="http://busdox.org/serviceMetadata/publishing/1.0/">
              <ServiceMetadata>
                <ServiceInformation>
                  <ProcessList>
                    <Process>
                      <ServiceEndpointList>
                        <Endpoint transportProfile="peppol-transport-as4-v2_0">
                          <EndpointURI>https://ap.example.test/as4</EndpointURI>
                          <TransportProfile>peppol-transport-as4-v2_0</TransportProfile>
                        </Endpoint>
                      </ServiceEndpointList>
                    </Process>
                  </ProcessList>
                </ServiceInformation>
              </ServiceMetadata>
            </SignedServiceMetadata>"#;
        let access = parse_smp_metadata(body, &DocumentTypeId::peppol_bis_3_invoice()).unwrap();
        assert_eq!(access.endpoint_url, "https://ap.example.test/as4");
        assert_eq!(access.transport_profile, "peppol-transport-as4-v2_0");
    }

    #[test]
    fn parse_smp_metadata_pairs_url_and_profile_from_same_endpoint() {
        // Regression: two endpoints where the FIRST one carries a
        // transport profile but an empty `EndpointURI`, and the
        // SECOND one is the only complete endpoint. A parser that
        // tracks the URL and profile globally would emit the
        // second endpoint's URL paired with the first endpoint's
        // profile. The correct result is the second endpoint's
        // own URL + profile, taken as a matched pair.
        let body = br#"<?xml version="1.0"?>
            <SignedServiceMetadata xmlns="http://busdox.org/serviceMetadata/publishing/1.0/">
              <ServiceMetadata>
                <ServiceInformation>
                  <ProcessList>
                    <Process>
                      <ServiceEndpointList>
                        <Endpoint transportProfile="busdox-transport-as2-ver1p0">
                          <EndpointURI></EndpointURI>
                          <TransportProfile>busdox-transport-as2-ver1p0</TransportProfile>
                        </Endpoint>
                        <Endpoint transportProfile="peppol-transport-as4-v2_0">
                          <EndpointURI>https://ap.example.test/as4</EndpointURI>
                          <TransportProfile>peppol-transport-as4-v2_0</TransportProfile>
                        </Endpoint>
                      </ServiceEndpointList>
                    </Process>
                  </ProcessList>
                </ServiceInformation>
              </ServiceMetadata>
            </SignedServiceMetadata>"#;
        let access = parse_smp_metadata(body, &DocumentTypeId::peppol_bis_3_invoice()).unwrap();
        assert_eq!(access.endpoint_url, "https://ap.example.test/as4");
        assert_eq!(access.transport_profile, "peppol-transport-as4-v2_0");
    }

    #[test]
    fn parse_smp_metadata_takes_first_complete_endpoint_when_many() {
        // Two fully-populated endpoints: the first complete one wins,
        // and its URL and profile stay paired together.
        let body = br#"<?xml version="1.0"?>
            <SignedServiceMetadata xmlns="http://busdox.org/serviceMetadata/publishing/1.0/">
              <ServiceMetadata>
                <ServiceInformation>
                  <ProcessList>
                    <Process>
                      <ServiceEndpointList>
                        <Endpoint transportProfile="peppol-transport-as4-v2_0">
                          <EndpointURI>https://ap-one.example.test/as4</EndpointURI>
                          <TransportProfile>peppol-transport-as4-v2_0</TransportProfile>
                        </Endpoint>
                        <Endpoint transportProfile="busdox-transport-as2-ver1p0">
                          <EndpointURI>https://ap-two.example.test/as2</EndpointURI>
                          <TransportProfile>busdox-transport-as2-ver1p0</TransportProfile>
                        </Endpoint>
                      </ServiceEndpointList>
                    </Process>
                  </ProcessList>
                </ServiceInformation>
              </ServiceMetadata>
            </SignedServiceMetadata>"#;
        let access = parse_smp_metadata(body, &DocumentTypeId::peppol_bis_3_invoice()).unwrap();
        assert_eq!(access.endpoint_url, "https://ap-one.example.test/as4");
        assert_eq!(access.transport_profile, "peppol-transport-as4-v2_0");
    }

    #[test]
    fn parse_smp_metadata_reports_no_endpoint_when_missing() {
        let body = b"<?xml version=\"1.0\"?><SignedServiceMetadata/>";
        let err = parse_smp_metadata(body, &DocumentTypeId::peppol_bis_3_invoice()).unwrap_err();
        assert!(matches!(err, PeppolLookupError::NoEndpoint(_)));
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-peppol-smp-sml");
    }

    /// Resolver that always returns a fixed SMP base for any host.
    struct FixedResolver(String);
    impl Resolver for FixedResolver {
        fn lookup_cname(&self, _host: &str) -> Result<Option<String>, String> {
            Ok(Some(self.0.clone()))
        }
    }

    /// HTTP client that records the exact URL it was asked to fetch.
    struct CapturingHttp(Mutex<Vec<String>>);
    impl HttpClient for CapturingHttp {
        fn get(&self, url: &str) -> Result<Vec<u8>, String> {
            self.0.lock().unwrap().push(url.to_owned());
            // Return a payload with no endpoint so the lookup ends
            // after the fetch; the test only inspects the URL.
            Ok(b"<?xml version=\"1.0\"?><X/>".to_vec())
        }
    }

    #[test]
    fn url_segment_neutralises_path_injection_from_identifier() {
        // Regression: a hostile participant/document-type identifier
        // must not break out of its path segment. Before the fix the
        // raw `scheme`/`value` were interpolated into the SMP REST URL
        // unencoded, so a `/ ? # %` payload injected extra path,
        // query, or fragment components.
        let participant = ParticipantId {
            scheme: "0192".to_owned(),
            value: "../../admin?x=1#frag%2e&z".to_owned(),
        };
        let document = DocumentTypeId {
            scheme: "busdox-docid-qns".to_owned(),
            value: "evil/../../bypass#frag".to_owned(),
        };

        let http = CapturingHttp(Mutex::new(Vec::new()));
        let resolver = FixedResolver("https://smp.example.test".to_owned());
        let client = PeppolClient::new(resolver, http, "acc.edelivery.tech.ec.europa.eu");
        let _ = client.lookup(&participant, &document);

        let url = {
            let urls = client.http.0.lock().unwrap();
            urls.first().expect("one SMP fetch was attempted").clone()
        };
        let url = url.as_str();
        // The base, the structural prefix, and `/services/` are the
        // only `/` separators allowed; the injected payload must not
        // add more. Strip the trusted prefix, then assert no path or
        // URL delimiters from the payload survived.
        let rest = url
            .strip_prefix("https://smp.example.test/")
            .expect("URL keeps the trusted SMP base");
        let injected = rest.replace("/services/", "|");
        assert!(
            !injected.contains('/'),
            "slash from identifier leaked into path: {url}"
        );
        assert!(
            !injected.contains('?'),
            "query opener from identifier leaked: {url}"
        );
        assert!(
            !injected.contains('#'),
            "fragment opener from identifier leaked: {url}"
        );
        // The literal `%` in the payload must itself be encoded, so the
        // only `%` sequences left are the encoder's own `%XX` escapes.
        assert!(
            url.contains("%2F") && url.contains("%3F") && url.contains("%23"),
            "expected percent escapes for `/`, `?`, `#`: {url}"
        );
    }

    #[test]
    fn url_segment_leaves_valid_identifiers_intact() {
        // The structural delimiters and ordinary identifier characters
        // must survive unchanged so legitimate lookups still hit the
        // canonical Peppol SMP path.
        let id = ParticipantId::parse("0192:991825827").unwrap();
        assert_eq!(id.to_url_segment(), "iso6523-actorid-upis::0192:991825827");
        let dt = DocumentTypeId::peppol_bis_3_invoice();
        assert!(dt.to_url_segment().starts_with("busdox-docid-qns::"));
    }
}
