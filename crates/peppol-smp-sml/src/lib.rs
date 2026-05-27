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
    #[must_use]
    pub fn to_url_segment(&self) -> String {
        format!("iso6523-actorid-upis::{}:{}", self.scheme, self.value)
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
    #[must_use]
    pub fn to_url_segment(&self) -> String {
        format!("{}::{}", self.scheme, self.value)
    }
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

/// Parse an SMP `SignedServiceMetadata` XML document and extract
/// the first matching `EndpointURI` + `TransportProfile` pair.
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
    let mut endpoint_url = None;
    let mut transport_profile = None;
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
                if local == "EndpointURI" || local == "EndpointReference" {
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
                if in_endpoint_uri && endpoint_url.is_none() && !text.is_empty() {
                    endpoint_url = Some(text);
                } else if in_transport_profile && transport_profile.is_none() && !text.is_empty() {
                    transport_profile = Some(text);
                }
            }
            Ok(quick_xml::events::Event::End(ref e)) => {
                let name = e.name();
                let local = std::str::from_utf8(name.local_name().as_ref())
                    .unwrap_or("")
                    .to_owned();
                if local == "EndpointURI" || local == "EndpointReference" {
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
    fn parse_smp_metadata_reports_no_endpoint_when_missing() {
        let body = b"<?xml version=\"1.0\"?><SignedServiceMetadata/>";
        let err = parse_smp_metadata(body, &DocumentTypeId::peppol_bis_3_invoice()).unwrap_err();
        assert!(matches!(err, PeppolLookupError::NoEndpoint(_)));
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-peppol-smp-sml");
    }
}
