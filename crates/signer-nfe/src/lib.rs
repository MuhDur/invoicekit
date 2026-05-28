// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

// NF-e / SEFAZ / ICP-Brasil / CNPJ / IBGE acronyms trip
// doc-markdown; suppress crate-wide.
#![allow(clippy::doc_markdown)]

//! `invoicekit-signer-nfe` — Brazil NF-e federal certificate
//! flow adapter.
//!
//! Layers the Brazil SEFAZ NF-e contract on top of
//! [`invoicekit_signer`]. NF-e is a per-state clearance flow:
//! the taxpayer's ICP-Brasil-chained A1 certificate signs the
//! NF-e XML, the state SEFAZ validates + assigns a `chave de
//! acesso` (44-character access key) + a `protocolo de
//! autorização`, and the buyer pulls the authorised invoice
//! from the state's portal.
//!
//! Public surface:
//!
//! * [`NfeProvider`] — provider trait every SEFAZ
//!   integration implements (one per state in production).
//! * [`UfCode`] — Brazilian state code (2-letter UF).
//! * [`NfeEnvironment`] — `Homologacao` (sandbox) vs
//!   `Producao`.
//! * [`Icp BrasilCertificate`] — typed A1 certificate
//!   reference (serial + CNPJ + chain).
//! * [`NfeStampEnvelope`] — typed envelope: chave de acesso
//!   + protocolo + cStat + status descricao + signed XML.
//! * [`MockNfeProvider`] — deterministic test provider.

use std::sync::Mutex;

use invoicekit_signer::{KeyRef, SignRequest, Signature, Signer, SigningError};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Brazilian state code (Unidade Federativa, ISO 3166-2:BR).
///
/// Modeled as a small enum of the high-volume states; other
/// UFs deserialise via the catch-all [`UfCode::Other`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum UfCode {
    /// São Paulo.
    Sp,
    /// Rio de Janeiro.
    Rj,
    /// Minas Gerais.
    Mg,
    /// Paraná.
    Pr,
    /// Rio Grande do Sul.
    Rs,
    /// Santa Catarina.
    Sc,
    /// Bahia.
    Ba,
    /// Distrito Federal.
    Df,
    /// Catch-all for any other UF, opaque 2-letter code.
    Other,
}

impl UfCode {
    /// Lowercase wire name.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Sp => "SP",
            Self::Rj => "RJ",
            Self::Mg => "MG",
            Self::Pr => "PR",
            Self::Rs => "RS",
            Self::Sc => "SC",
            Self::Ba => "BA",
            Self::Df => "DF",
            Self::Other => "OTHER",
        }
    }
}

/// NF-e environment — Homologação is the SEFAZ sandbox;
/// Produção is the live tax-clearance environment.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NfeEnvironment {
    /// Homologação (sandbox).
    Homologacao,
    /// Produção (live).
    Producao,
}

impl NfeEnvironment {
    /// Lowercase wire name.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Homologacao => "homologacao",
            Self::Producao => "producao",
        }
    }

    /// Numeric `tpAmb` value SEFAZ expects in the NF-e XML
    /// (`1` = produção, `2` = homologação).
    #[must_use]
    pub const fn tp_amb(self) -> u8 {
        match self {
            Self::Producao => 1,
            Self::Homologacao => 2,
        }
    }
}

/// ICP-Brasil A1 certificate reference.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IcpBrasilCertificate {
    /// X.509 serial number.
    pub serial_number: String,
    /// CNPJ (14-digit Brazilian company id) the cert is
    /// bound to.
    pub cnpj: String,
    /// Subject distinguished name.
    pub subject_dn: String,
    /// PEM-encoded X.509 bytes (opaque on substrate).
    pub certificate_pem: Vec<u8>,
}

/// SEFAZ status codes the bridge recognises. Maps cStat
/// values from the NF-e specification.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NfeStatus {
    /// 100 — Autorizado o uso da NF-e.
    Authorized,
    /// 110 — Uso Denegado.
    Denied,
    /// 205 — NF-e está denegada na base de dados da SEFAZ.
    DeniedInDatabase,
    /// 215 — Falha no schema XML.
    SchemaFailure,
    /// 539 — Duplicidade de NF-e.
    Duplicate,
    /// Any other cStat (numeric code surfaced via the
    /// envelope's cStat field).
    Other,
}

impl NfeStatus {
    /// Map from the SEFAZ numeric cStat code.
    #[must_use]
    pub const fn from_c_stat(code: u32) -> Self {
        match code {
            100 => Self::Authorized,
            110 => Self::Denied,
            205 => Self::DeniedInDatabase,
            215 => Self::SchemaFailure,
            539 => Self::Duplicate,
            _ => Self::Other,
        }
    }

    /// True only when the NF-e was authorised.
    #[must_use]
    pub const fn is_authorized(self) -> bool {
        matches!(self, Self::Authorized)
    }
}

/// Typed NF-e stamp envelope.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NfeStampEnvelope {
    /// Underlying [`Signer`] receipt — the XAdES-BES
    /// signature over the canonical NF-e XML.
    pub signature: Signature,
    /// Chave de acesso (44-char SEFAZ access key, includes
    /// UF + AAMM + CNPJ + modelo + série + nNF + cNF + cDV).
    pub chave_acesso: String,
    /// Protocolo de autorização returned by SEFAZ.
    pub protocolo_autorizacao: String,
    /// Numeric SEFAZ cStat code.
    pub c_stat: u32,
    /// Typed status mapping.
    pub status: NfeStatus,
    /// SEFAZ status description (xMotivo).
    pub status_descricao: String,
    /// Signed NF-e XML bytes (XAdES wrapped).
    pub signed_nfe_xml: Vec<u8>,
    /// UF and environment the invoice was submitted to.
    pub uf: UfCode,
    /// Environment used.
    pub environment: NfeEnvironment,
}

/// Submission request shape.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NfeSubmitRequest {
    /// Canonical NF-e XML bytes.
    pub nfe_xml: Vec<u8>,
    /// ICP-Brasil A1 certificate.
    pub certificate: IcpBrasilCertificate,
    /// State UF the invoice is destined for.
    pub uf: UfCode,
    /// Numeric `nNF` (invoice number, 1..=999999999).
    pub n_nf: u64,
}

/// Errors raised by [`NfeProvider`] implementations.
#[derive(Debug, Error)]
pub enum NfeError {
    /// Underlying signer refused.
    #[error("nfe provider's signer refused: {0}")]
    Signer(SigningError),
    /// CNPJ on the certificate mismatched the NF-e issuer.
    #[error("NF-e CNPJ mismatch: certificate={cert}")]
    CnpjMismatch {
        /// CNPJ on the certificate.
        cert: String,
    },
    /// SEFAZ rejected the NF-e.
    #[error("SEFAZ rejected the NF-e: cStat={c_stat}, xMotivo={x_motivo}")]
    SefazRejected {
        /// Numeric cStat code.
        c_stat: u32,
        /// Description.
        x_motivo: String,
    },
    /// SEFAZ portal unreachable.
    #[error("SEFAZ portal unavailable: {0}")]
    Unavailable(String),
    /// Environment mismatch — Homologação NF-e against
    /// Produção request or vice versa.
    #[error("NF-e environment mismatch: cert={cert:?}, request={request:?}")]
    EnvironmentMismatch {
        /// Environment the certificate targets.
        cert: NfeEnvironment,
        /// Environment the request targets.
        request: NfeEnvironment,
    },
}

/// NF-e provider surface.
pub trait NfeProvider: Send + Sync {
    /// Provider display name (e.g. `sefaz-sp`, `sefaz-rj`).
    fn provider_name(&self) -> &str;

    /// Environment this provider targets.
    fn environment(&self) -> NfeEnvironment;

    /// Submit an NF-e invoice.
    ///
    /// # Errors
    ///
    /// Returns [`NfeError`] when the CNPJ mismatches, SEFAZ
    /// rejects the NF-e, the environment mismatches, the
    /// signer refuses, or the portal is unreachable.
    fn submit(
        &self,
        request: &NfeSubmitRequest,
        target_environment: NfeEnvironment,
    ) -> Result<NfeStampEnvelope, NfeError>;
}

/// Mock NF-e provider.
pub struct MockNfeProvider {
    name: String,
    environment: NfeEnvironment,
    signer: std::sync::Arc<dyn Signer>,
    forced_c_stat: u32,
    submissions: Mutex<Vec<NfeSubmitRequest>>,
    next_protocolo: Mutex<u64>,
}

impl MockNfeProvider {
    /// Build a mock NF-e provider.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        environment: NfeEnvironment,
        signer: std::sync::Arc<dyn Signer>,
    ) -> Self {
        Self {
            name: name.into(),
            environment,
            signer,
            forced_c_stat: 100, // Authorized
            submissions: Mutex::new(Vec::new()),
            next_protocolo: Mutex::new(1),
        }
    }

    /// Force the provider to return a specific cStat code.
    #[must_use]
    pub fn with_forced_c_stat(mut self, c_stat: u32) -> Self {
        self.forced_c_stat = c_stat;
        self
    }

    /// Snapshot of recorded submissions.
    ///
    /// # Panics
    ///
    /// Panics if a prior holder of the mutex panicked.
    #[must_use]
    pub fn submissions(&self) -> Vec<NfeSubmitRequest> {
        self.submissions.lock().unwrap().clone()
    }
}

impl NfeProvider for MockNfeProvider {
    fn provider_name(&self) -> &str {
        &self.name
    }

    fn environment(&self) -> NfeEnvironment {
        self.environment
    }

    fn submit(
        &self,
        request: &NfeSubmitRequest,
        target_environment: NfeEnvironment,
    ) -> Result<NfeStampEnvelope, NfeError> {
        if self.environment != target_environment {
            return Err(NfeError::EnvironmentMismatch {
                cert: self.environment,
                request: target_environment,
            });
        }
        if request.certificate.cnpj.is_empty() {
            return Err(NfeError::CnpjMismatch {
                cert: request.certificate.cnpj.clone(),
            });
        }
        let signature = self
            .signer
            .sign(&SignRequest {
                key_ref: KeyRef::new(&request.certificate.serial_number),
                payload: request.nfe_xml.clone(),
            })
            .map_err(NfeError::Signer)?;
        let protocolo = {
            let mut g = self.next_protocolo.lock().expect("mutex poisoned");
            let n = *g;
            *g += 1;
            n
        };
        let chave_acesso = build_chave_acesso(request.uf, &request.certificate.cnpj, request.n_nf);
        let mut signed_nfe_xml: Vec<u8> = b"<XAdES-stub>".to_vec();
        signed_nfe_xml.extend_from_slice(&request.nfe_xml);
        signed_nfe_xml.extend_from_slice(b"</XAdES-stub>");
        self.submissions.lock().unwrap().push(request.clone());
        let c_stat = self.forced_c_stat;
        let status = NfeStatus::from_c_stat(c_stat);
        Ok(NfeStampEnvelope {
            signature,
            chave_acesso,
            protocolo_autorizacao: format!("135{protocolo:015}"),
            c_stat,
            status,
            status_descricao: nfe_status_descricao(c_stat).to_owned(),
            signed_nfe_xml,
            uf: request.uf,
            environment: self.environment,
        })
    }
}

/// Build a 44-character chave de acesso.
///
/// Lays out `UF (2) | AAMM (4) | CNPJ (14) | mod (2) | série
/// (3) | nNF (9) | tpEmis (1) | cNF (8) | cDV (1)`
/// deterministically from the inputs. The real SEFAZ flow
/// computes cNF from the emitting system and cDV via mod-11;
/// the substrate uses fixed `00000000` + `0` so the
/// substrate's keys round-trip stably across runs.
#[must_use]
pub fn build_chave_acesso(uf: UfCode, cnpj: &str, n_nf: u64) -> String {
    let uf_code = uf_numeric_code(uf);
    let cnpj_padded = cnpj_padded_14(cnpj);
    format!("{uf_code:02}260555000000000{cnpj_padded}5500001{n_nf:09}10000000000")
        .chars()
        .take(44)
        .collect::<String>()
}

fn uf_numeric_code(uf: UfCode) -> u8 {
    // IBGE state codes (subset).
    match uf {
        UfCode::Sp => 35,
        UfCode::Rj => 33,
        UfCode::Mg => 31,
        UfCode::Pr => 41,
        UfCode::Rs => 43,
        UfCode::Sc => 42,
        UfCode::Ba => 29,
        UfCode::Df => 53,
        UfCode::Other => 99,
    }
}

fn cnpj_padded_14(cnpj: &str) -> String {
    let digits: String = cnpj.chars().filter(char::is_ascii_digit).collect();
    if digits.len() >= 14 {
        digits.chars().take(14).collect()
    } else {
        let pad = 14 - digits.len();
        format!("{}{digits}", "0".repeat(pad))
    }
}

/// Lookup the canonical xMotivo description for known cStat
/// codes. Catch-all returns `"Other"`.
#[must_use]
pub const fn nfe_status_descricao(c_stat: u32) -> &'static str {
    match c_stat {
        100 => "Autorizado o uso da NF-e",
        110 => "Uso denegado",
        205 => "NF-e está denegada na base de dados da SEFAZ",
        215 => "Falha no schema XML",
        539 => "Duplicidade de NF-e",
        _ => "Other",
    }
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_signer_nfe::crate_name(),
///     "invoicekit-signer-nfe"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-signer-nfe"
}

#[cfg(test)]
mod tests {
    use super::*;
    use invoicekit_signer::SoftwareSigner;
    use std::sync::Arc;

    fn sample_cert() -> IcpBrasilCertificate {
        IcpBrasilCertificate {
            serial_number: "ABCDEF1234567890".to_owned(),
            cnpj: "11222333000181".to_owned(),
            subject_dn: "CN=Acme LTDA,O=Acme,C=BR".to_owned(),
            certificate_pem: b"-----BEGIN CERTIFICATE-----\n...stub...".to_vec(),
        }
    }

    fn build_provider(env: NfeEnvironment) -> MockNfeProvider {
        let signer: Arc<dyn Signer> =
            Arc::new(SoftwareSigner::new().with_key("ABCDEF1234567890", [3_u8; 32]));
        MockNfeProvider::new("sefaz-sp-test", env, signer)
    }

    fn sample_request(uf: UfCode) -> NfeSubmitRequest {
        NfeSubmitRequest {
            nfe_xml: b"<NFe/>".to_vec(),
            certificate: sample_cert(),
            uf,
            n_nf: 4242,
        }
    }

    #[test]
    fn crate_name_matches_cargo() {
        assert_eq!(crate_name(), "invoicekit-signer-nfe");
    }

    #[test]
    fn uf_round_trips_uppercase_json() {
        let json = serde_json::to_string(&UfCode::Sp).unwrap();
        assert_eq!(json, "\"SP\"");
        let back: UfCode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, UfCode::Sp);
    }

    #[test]
    fn environment_tp_amb_matches_sefaz_spec() {
        assert_eq!(NfeEnvironment::Producao.tp_amb(), 1);
        assert_eq!(NfeEnvironment::Homologacao.tp_amb(), 2);
    }

    #[test]
    fn nfe_status_maps_c_stat() {
        assert_eq!(NfeStatus::from_c_stat(100), NfeStatus::Authorized);
        assert_eq!(NfeStatus::from_c_stat(215), NfeStatus::SchemaFailure);
        assert_eq!(NfeStatus::from_c_stat(539), NfeStatus::Duplicate);
        assert_eq!(NfeStatus::from_c_stat(9999), NfeStatus::Other);
        assert!(NfeStatus::Authorized.is_authorized());
        assert!(!NfeStatus::Denied.is_authorized());
    }

    #[test]
    fn build_chave_acesso_produces_44_chars() {
        let chave = build_chave_acesso(UfCode::Sp, "11222333000181", 4242);
        assert_eq!(chave.len(), 44);
        assert!(chave.starts_with("35"));
    }

    #[test]
    fn cnpj_padded_14_strips_punctuation_and_pads() {
        assert_eq!(cnpj_padded_14("11.222.333/0001-81"), "11222333000181");
        assert_eq!(cnpj_padded_14("1234"), "00000000001234");
    }

    #[test]
    fn submit_rejects_environment_mismatch() {
        let provider = build_provider(NfeEnvironment::Homologacao);
        let err = provider
            .submit(&sample_request(UfCode::Sp), NfeEnvironment::Producao)
            .unwrap_err();
        assert!(matches!(err, NfeError::EnvironmentMismatch { .. }));
    }

    #[test]
    fn submit_rejects_empty_cnpj() {
        let provider = build_provider(NfeEnvironment::Homologacao);
        let mut req = sample_request(UfCode::Sp);
        req.certificate.cnpj = String::new();
        let err = provider
            .submit(&req, NfeEnvironment::Homologacao)
            .unwrap_err();
        assert!(matches!(err, NfeError::CnpjMismatch { .. }));
    }

    #[test]
    fn submit_produces_envelope_with_chave_and_protocolo() {
        let provider = build_provider(NfeEnvironment::Homologacao);
        let envelope = provider
            .submit(&sample_request(UfCode::Sp), NfeEnvironment::Homologacao)
            .unwrap();
        assert_eq!(envelope.chave_acesso.len(), 44);
        assert!(envelope.protocolo_autorizacao.starts_with("135"));
        assert_eq!(envelope.c_stat, 100);
        assert!(envelope.status.is_authorized());
        assert_eq!(envelope.status_descricao, "Autorizado o uso da NF-e");
        assert_eq!(envelope.uf, UfCode::Sp);
        assert_eq!(envelope.environment, NfeEnvironment::Homologacao);
        assert_eq!(provider.submissions().len(), 1);
    }

    #[test]
    fn submit_propagates_forced_c_stat() {
        let provider = build_provider(NfeEnvironment::Homologacao).with_forced_c_stat(215);
        let envelope = provider
            .submit(&sample_request(UfCode::Rj), NfeEnvironment::Homologacao)
            .unwrap();
        assert_eq!(envelope.c_stat, 215);
        assert_eq!(envelope.status, NfeStatus::SchemaFailure);
        assert_eq!(envelope.status_descricao, "Falha no schema XML");
        assert!(!envelope.status.is_authorized());
    }
}
