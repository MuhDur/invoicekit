<!-- SPDX-License-Identifier: Apache-2.0 -->

# invoicekit-peppol-smp-sml

Peppol participant lookup: turn a participant identifier plus a document-type identifier into the access-point endpoint that can receive it, via the SML (Service Metadata Locator, DNS) then SMP (Service Metadata Publisher, HTTP) stages.

## What it does

The Peppol four-corner network discovers a recipient's access point in two layered steps, and this crate runs both:

1. **SML stage (DNS).** The participant id (e.g. `0192:991825827`) is lowercased, MD5-hashed, and base32-encoded (RFC 4648 lowercase, unpadded) to build the canonical SML hostname `B-<hash>.iso6523-actorid-upis.<sml-domain>`. A CNAME lookup on that hostname yields the participant's SMP base URL. MD5 here is the algorithm the Peppol SML specification mandates for this hostname construction; it is not a hash placeholder and is not used as a security primitive.
2. **SMP stage (HTTP).** A GET on `{smp-base}/{participant-segment}/services/{document-type-segment}` returns an XML metadata document. The crate parses it for the first `Endpoint` carrying both an `EndpointURI` (or `EndpointReference`) and a `TransportProfile`, and returns that pair as an `AccessPoint`.

DNS and HTTP are injected behind the `Resolver` and `HttpClient` traits, so the pipeline runs without network access in tests. This crate ships no real transport; the module doc-comment notes the upstream `crates/transmit-peppol` bead wires in a real `hickory-dns-resolver` + `reqwest` stack. Results are held in a per-client TTL cache (default 10 minutes).

## Capabilities

- `ParticipantId` — `{scheme, value}`. `parse(wire)` accepts `scheme:value` (both halves non-empty), `to_url_segment()` emits `iso6523-actorid-upis::{scheme}:{value}` with scheme and value percent-encoded, and `to_sml_hostname(sml_domain)` builds the MD5/base32 SML hostname.
- `DocumentTypeId` — `{scheme, value}`. `peppol_bis_3_invoice()` returns the Peppol BIS Billing 3.0 Invoice document type (`busdox-docid-qns` scheme). `to_url_segment()` emits `{scheme}::{value}`, both halves percent-encoded.
- `AccessPoint` — the resolved `{endpoint_url, transport_profile}` pair.
- `Resolver` trait — `lookup_cname(host) -> Result<Option<String>, String>`, returning the SMP base URL (an HTTP URL string) or `None` when unregistered.
- `HttpClient` trait — `get(url) -> Result<Vec<u8>, String>`, returning response body bytes.
- `PeppolClient<R, H>` — `new(resolver, http, sml_domain)`, optional `with_default_ttl(ttl)`, and `lookup(participant, document_type) -> Result<AccessPoint, PeppolLookupError>`. Runs SML then SMP, caches the hit, and serves subsequent lookups from the TTL cache.
- `parse_smp_metadata(body, document_type)` — standalone SMP XML parser (uses `quick-xml`). Scopes `EndpointURI`/`TransportProfile` to the enclosing `Endpoint` so a later endpoint's URL is never paired with an earlier endpoint's profile; falls back to bare unwrapped pairs.
- `cache::TtlCache<K, V>` — `HashMap`-backed cache with absolute per-entry expiry: `insert`, `get(key, now)` (drops on expiry), `prune(now)`, `len`, `is_empty`. Single-threaded; the client wraps it in a `Mutex`.
- `PeppolLookupError` — `Sml`, `Smp`, `Parse`, `NoEndpoint` variants.
- Constants `PEPPOL_PRODUCTION_SML`, `PEPPOL_ACCEPTANCE_SML`; `crate_name()`.

### Security of the path construction

`percent_encode_segment` (RFC 3986 unreserved set) escapes scheme and value before they enter the SMP REST URL, so a hostile participant or document-type identifier carrying `/`, `?`, `#`, `%`, or `..` cannot inject extra path, query, or fragment components. The `iso6523-actorid-upis::`, `::`, and `/services/` literals stay readable. Covered by a regression test.

## Mode / Residuals

**Lookup logic is real; transport is injected and ships only as test doubles.**

What this crate does and exercises with tests: SML hostname construction, the SMP REST URL shape, percent-encoding against path injection, SMP XML endpoint extraction (including the same-endpoint pairing rule and the first-complete-endpoint rule), and TTL cache expiry.

What it does **not** do:

- **No bundled DNS or HTTP transport.** Only the `Resolver` / `HttpClient` traits and in-test doubles ship here. Nothing leaves the process until an upstream crate supplies real implementations.
- **No DNSSEC validation.** The SML stage trusts whatever CNAME the injected resolver returns; this crate performs no DNSSEC chain verification.
- **No SMP signature or TLS verification.** Despite the `SignedServiceMetadata` element name, `parse_smp_metadata` reads endpoint text only — it does **not** verify the SMP document's XML-DSig signature, nor any TLS certificate. Authenticity of the returned `AccessPoint` is not established by this crate.
- **First-match endpoint selection.** `lookup` returns the first `Endpoint` carrying both a URL and a transport profile. It does not filter by `TransportProfile` value (e.g. it does not prefer `peppol-transport-as4-v2_0`), does not check process identifiers, and does not honour `ServiceActivationDate` / `ServiceExpirationDate`.
- **No certificate extraction.** The SMP `Certificate` element (the access point's signing certificate) is not parsed or returned.

## References

Standards and identifiers named in the source:

- Peppol SML hostname scheme — MD5 + base32 (RFC 4648 lowercase, unpadded), `B-<hash>.iso6523-actorid-upis.<sml-domain>`.
- SML domains — production `edelivery.tech.ec.europa.eu`, acceptance `acc.edelivery.tech.ec.europa.eu`.
- RFC 3986 — percent-encoding of URL path segments (unreserved set).
- Peppol BIS Billing 3.0 — Invoice document type `urn:oasis:names:specification:ubl:schema:xsd:Invoice-2::Invoice##urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0::2.1`, scheme `busdox-docid-qns`.
- SMP namespace `http://busdox.org/serviceMetadata/publishing/1.0/` (in test fixtures).

## License

Apache-2.0. Part of the InvoiceKit workspace; this crate is `publish = false`.
