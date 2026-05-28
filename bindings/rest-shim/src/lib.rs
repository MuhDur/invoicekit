// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-binding-rest-shim` — REST-sidecar wrapper over the engine ABI.
//!
//! The sidecar exposes the thin REST surface from `plans/PLAN.md` section 5.5
//! and preserves the raw Engine ABI endpoint used by the Go no-cgo fallback.

#![allow(
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::needless_pass_by_value,
    clippy::too_many_lines
)]

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use invoicekit_evidence::{manifest_for, pack, EvidenceBundle};
use invoicekit_ir::CommercialDocument;
use invoicekit_verify::{verify_packed, CheckOutcome, VerifyOptions, VerifyReport};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

const FIXED_BUNDLE_CREATED_AT: &str = "2026-01-01T00:00:00Z";
const CAPABILITIES_MATRIX: &str = include_str!("../../../crates/cli/data/capabilities/matrix.json");

/// Process an Engine ABI JSON request through the REST shim wrapper.
///
/// # Examples
///
/// ```
/// let response = invoicekit_binding_rest_shim::process_engine_abi_json(
///     br#"{"abi_version":1,"operation":"unknown","payload":{}}"#,
/// );
/// assert!(std::str::from_utf8(&response).unwrap().contains(r#""status":"error""#));
/// ```
#[must_use]
pub fn process_engine_abi_json(request_bytes: &[u8]) -> Vec<u8> {
    invoicekit_engine::process_abi_json(request_bytes)
}

/// Shared in-memory state for the REST shim.
///
/// The shim is intentionally small and stateless-at-rest: it exists for
/// conservative runtimes that cannot or do not want to load native bindings.
/// A production managed API can replace this with persistent stores without
/// changing the route contracts.
#[derive(Clone, Debug, Default)]
pub struct RestShimState {
    invoices: Arc<RwLock<BTreeMap<String, StoredInvoice>>>,
    transmissions: Arc<RwLock<BTreeMap<String, TransmissionRecord>>>,
}

/// Build the default Axum router.
pub fn build_router() -> Router {
    build_router_with_state(RestShimState::default())
}

/// Build the Axum router with caller-supplied state.
pub fn build_router_with_state(state: RestShimState) -> Router {
    let v1 = Router::new()
        .route("/engine/process_json", post(process_engine_json))
        .route("/invoices", post(create_invoice))
        .route("/invoices/{id}/validate", post(validate_invoice))
        .route("/invoices/{id}/render", post(render_invoice))
        .route("/invoices/{id}/transmit", post(transmit_invoice))
        .route("/transmissions/{id}", get(get_transmission))
        .route("/reconcile", post(reconcile))
        .route("/bundles/{id}", get(get_bundle))
        .route("/bundles/verify", post(verify_bundle))
        .route("/capabilities", get(get_capabilities))
        .route("/openapi.json", get(openapi_json));

    Router::new()
        .route("/openapi.json", get(openapi_json))
        .nest("/v1", v1)
        .with_state(state)
}

/// Start the REST shim listener on the supplied bind address.
pub async fn serve(bind: &str) -> Result<(), ServeError> {
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|source| ServeError::Bind {
            bind: bind.to_owned(),
            source,
        })?;
    axum::serve(listener, build_router())
        .await
        .map_err(ServeError::Serve)
}

/// Listener startup errors.
#[derive(Debug, Error)]
pub enum ServeError {
    /// The TCP listener could not bind.
    #[error("could not bind REST shim to {bind}: {source}")]
    Bind {
        /// Requested bind address.
        bind: String,
        /// Underlying IO error.
        source: std::io::Error,
    },
    /// Axum returned an IO error while serving.
    #[error("REST shim server failed: {0}")]
    Serve(std::io::Error),
}

#[derive(Clone, Debug)]
struct StoredInvoice {
    engine_request: Vec<u8>,
    document: CommercialDocument,
    bundle: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
struct TransmissionRecord {
    id: String,
    invoice_id: String,
    state: String,
    gateway: String,
}

/// JSON contract returned by `POST /v1/engine/process_json`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct EngineProcessResponse {
    /// C-ABI-compatible status code: 0 for ok, 1 for canonical engine error.
    pub status: u32,
    /// Base64-encoded canonical engine response bytes.
    pub response_base64: String,
}

/// JSON contract returned by `POST /v1/invoices`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct InvoiceResponse {
    /// Stable sidecar invoice identifier.
    pub id: String,
    /// Engine ABI response status.
    pub engine_status: u32,
    /// Base64-encoded canonical engine response bytes.
    pub engine_response_base64: String,
    /// Bundle identifier for `GET /v1/bundles/{id}`.
    pub bundle_id: String,
}

/// JSON contract returned by `POST /v1/invoices/{id}/transmit`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct TransmitResponse {
    /// Transmission tracking identifier.
    pub transmission_id: String,
    /// Initial state-machine state.
    pub state: String,
}

/// JSON body accepted by `POST /v1/reconcile`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
struct ReconcileRequest {
    /// Invoice identifiers to reconcile against the sidecar store.
    invoice_ids: Vec<String>,
}

/// JSON contract returned by `POST /v1/reconcile`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
struct ReconcileResponse {
    /// One result per requested invoice identifier.
    matches: Vec<ReconcileMatch>,
}

/// Reconciliation result for one invoice identifier.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
struct ReconcileMatch {
    /// Requested invoice identifier.
    invoice_id: String,
    /// True when the sidecar currently stores this invoice.
    present: bool,
}

/// Query parameters accepted by `GET /v1/capabilities`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
struct CapabilitiesQuery {
    /// Source country or runtime route code.
    from: Option<String>,
    /// Destination country or runtime route code.
    to: Option<String>,
    /// Effective date used to filter capability windows.
    date: Option<String>,
    /// Scenario such as B2B or B2G.
    scenario: Option<String>,
}

/// JSON contract returned by `POST /v1/bundles/verify`.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct VerifyBundleResponse {
    /// Aggregate verdict.
    pub ok: bool,
    /// Per-artefact re-hashing + manifest reconciliation.
    pub content_address: VerifyCheckOutcome,
    /// Detached signature over `manifest.json`.
    pub signature: VerifyCheckOutcome,
    /// DSSE sidecar bound to canonical `manifest.json` bytes.
    pub manifest_envelope: VerifyCheckOutcome,
    /// RFC 3161 timestamp re-binding to the manifest imprint.
    pub timestamp: VerifyCheckOutcome,
}

/// REST-serializable outcome for one verification check.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum VerifyCheckOutcome {
    /// The check passed.
    Passed,
    /// The check was not requested.
    Skipped {
        /// One-line operator-readable reason.
        reason: String,
    },
    /// The check failed.
    Failed {
        /// One-line operator-readable error.
        error: String,
    },
}

async fn process_engine_json(body: Bytes) -> Json<EngineProcessResponse> {
    Json(engine_process_response(&body))
}

async fn create_invoice(
    State(state): State<RestShimState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<InvoiceResponse>), ApiError> {
    let engine_request = engine_request_from_body(&body)?;
    let engine_response = process_engine_abi_json(&engine_request);
    let engine_status = engine_status(&engine_response);
    if engine_status != 0 {
        return Err(ApiError::UnprocessableEngineResponse {
            response: engine_process_response(&engine_request),
        });
    }
    let document = document_from_engine_response(&engine_response)?;

    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let id = prefixed_hash("inv", &[idempotency_key.as_bytes(), &engine_response]);
    let bundle = bundle_for_invoice(&id, &engine_request, &engine_response)?;
    let response = InvoiceResponse {
        id: id.clone(),
        engine_status,
        engine_response_base64: BASE64_STANDARD.encode(&engine_response),
        bundle_id: id.clone(),
    };
    state
        .invoices
        .write()
        .map_err(|_| ApiError::internal("invoice store lock poisoned"))?
        .insert(
            id,
            StoredInvoice {
                engine_request,
                document,
                bundle,
            },
        );
    Ok((StatusCode::CREATED, Json(response)))
}

async fn validate_invoice(
    State(state): State<RestShimState>,
    Path(id): Path<String>,
) -> Result<Json<EngineProcessResponse>, ApiError> {
    let request = {
        let invoices = state
            .invoices
            .read()
            .map_err(|_| ApiError::internal("invoice store lock poisoned"))?;
        invoices
            .get(&id)
            .ok_or_else(|| ApiError::not_found("invoice", &id))?
            .engine_request
            .clone()
    };
    Ok(Json(engine_process_response(&request)))
}

async fn render_invoice(
    State(state): State<RestShimState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let document = {
        let invoices = state
            .invoices
            .read()
            .map_err(|_| ApiError::internal("invoice store lock poisoned"))?;
        invoices
            .get(&id)
            .ok_or_else(|| ApiError::not_found("invoice", &id))?
            .document
            .clone()
    };
    let pdf = invoicekit_render_pdf::render_commercial_document_invoice(&document)
        .map_err(|err| ApiError::internal(format!("PDF render failed: {err}")))?;
    let mut response = (StatusCode::OK, pdf).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/pdf"),
    );
    headers.insert(
        "x-invoicekit-renderer",
        HeaderValue::from_static("invoicekit-render-pdf:commercial-document"),
    );
    Ok(response)
}

async fn transmit_invoice(
    State(state): State<RestShimState>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<TransmitResponse>), ApiError> {
    ensure_invoice_exists(&state, &id)?;
    let transmission_id = prefixed_hash("tx", &[id.as_bytes()]);
    let record = TransmissionRecord {
        id: transmission_id.clone(),
        invoice_id: id,
        state: "accepted".to_owned(),
        gateway: "mock".to_owned(),
    };
    state
        .transmissions
        .write()
        .map_err(|_| ApiError::internal("transmission store lock poisoned"))?
        .insert(transmission_id.clone(), record);
    Ok((
        StatusCode::ACCEPTED,
        Json(TransmitResponse {
            transmission_id,
            state: "accepted".to_owned(),
        }),
    ))
}

async fn get_transmission(
    State(state): State<RestShimState>,
    Path(id): Path<String>,
) -> Result<Json<TransmissionRecord>, ApiError> {
    let transmissions = state
        .transmissions
        .read()
        .map_err(|_| ApiError::internal("transmission store lock poisoned"))?;
    transmissions
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::not_found("transmission", &id))
}

async fn reconcile(
    State(state): State<RestShimState>,
    Json(request): Json<ReconcileRequest>,
) -> Result<Json<ReconcileResponse>, ApiError> {
    let invoices = state
        .invoices
        .read()
        .map_err(|_| ApiError::internal("invoice store lock poisoned"))?;
    let matches = request
        .invoice_ids
        .into_iter()
        .map(|invoice_id| ReconcileMatch {
            present: invoices.contains_key(&invoice_id),
            invoice_id,
        })
        .collect();
    Ok(Json(ReconcileResponse { matches }))
}

async fn get_bundle(
    State(state): State<RestShimState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let bundle = {
        let invoices = state
            .invoices
            .read()
            .map_err(|_| ApiError::internal("invoice store lock poisoned"))?;
        invoices
            .get(&id)
            .ok_or_else(|| ApiError::not_found("bundle", &id))?
            .bundle
            .clone()
    };
    let mut response = (StatusCode::OK, bundle).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.invoicekit.bundle"),
    );
    Ok(response)
}

async fn verify_bundle(body: Bytes) -> Result<Json<VerifyBundleResponse>, ApiError> {
    let report = verify_packed(&body, &VerifyOptions::content_only())
        .map_err(|err| ApiError::BadBundle(err.to_string()))?;
    Ok(Json(report.into()))
}

async fn get_capabilities(Query(query): Query<CapabilitiesQuery>) -> Result<Json<Value>, ApiError> {
    let matrix = capabilities_matrix()?;
    if query.from.is_none()
        && query.to.is_none()
        && query.date.is_none()
        && query.scenario.is_none()
    {
        return Ok(Json(matrix));
    }
    let entries = matrix
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::internal("capability matrix missing entries array"))?;
    let filtered: Vec<Value> = entries
        .iter()
        .filter(|entry| capability_entry_matches(entry, &query))
        .cloned()
        .collect();
    Ok(Json(json!({
        "schema_version": matrix.get("schema_version").cloned().unwrap_or(Value::Null),
        "generated_at": matrix.get("generated_at").cloned().unwrap_or(Value::Null),
        "query": query_json(&query),
        "entries": filtered,
    })))
}

async fn openapi_json() -> Response {
    let body = openapi_document_bytes();
    let hash = openapi_sha256_hex_for_bytes(&body);
    let mut response = (StatusCode::OK, body).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    if let Ok(value) = HeaderValue::from_str(&hash) {
        headers.insert("x-invoicekit-openapi-sha256", value);
    }
    response
}

fn engine_request_from_body(body: &[u8]) -> Result<Vec<u8>, ApiError> {
    let value: Value = serde_json::from_slice(body).map_err(|err| ApiError::BadJson {
        reason: err.to_string(),
    })?;
    let request = if value.get("abi_version").is_some() && value.get("operation").is_some() {
        value
    } else {
        json!({
            "abi_version": invoicekit_engine::ENGINE_ABI_VERSION,
            "operation": invoicekit_engine::COMMERCIAL_DOCUMENT_CANONICALIZE_OPERATION,
            "payload": value,
        })
    };
    serde_json::to_vec(&request).map_err(|err| ApiError::internal(err.to_string()))
}

fn engine_process_response(request: &[u8]) -> EngineProcessResponse {
    let response = process_engine_abi_json(request);
    EngineProcessResponse {
        status: engine_status(&response),
        response_base64: BASE64_STANDARD.encode(response),
    }
}

fn engine_status(response: &[u8]) -> u32 {
    u32::from(
        !response
            .windows(br#""status":"ok""#.len())
            .any(|window| window == br#""status":"ok""#),
    )
}

fn bundle_for_invoice(
    id: &str,
    engine_request: &[u8],
    engine_response: &[u8],
) -> Result<Vec<u8>, ApiError> {
    let mut artefacts = BTreeMap::new();
    artefacts.insert(
        "request/engine-abi.json".to_owned(),
        engine_request.to_vec(),
    );
    artefacts.insert("responses/engine.json".to_owned(), engine_response.to_vec());
    let manifest = manifest_for(&artefacts, "rest-shim", id, FIXED_BUNDLE_CREATED_AT);
    let bundle = EvidenceBundle {
        manifest,
        artefacts,
    };
    pack(&bundle).map_err(|err| ApiError::internal(format!("bundle pack failed: {err}")))
}

fn document_from_engine_response(response: &[u8]) -> Result<CommercialDocument, ApiError> {
    let value: Value =
        serde_json::from_slice(response).map_err(|err| ApiError::internal(err.to_string()))?;
    let document = value
        .get("payload")
        .and_then(|payload| payload.get("document"))
        .cloned()
        .ok_or_else(|| ApiError::internal("engine response missing payload.document"))?;
    CommercialDocument::try_from_value(document).map_err(|err| {
        ApiError::internal(format!(
            "engine response document failed IR revalidation: {err}"
        ))
    })
}

fn ensure_invoice_exists(state: &RestShimState, id: &str) -> Result<(), ApiError> {
    let invoices = state
        .invoices
        .read()
        .map_err(|_| ApiError::internal("invoice store lock poisoned"))?;
    if invoices.contains_key(id) {
        Ok(())
    } else {
        Err(ApiError::not_found("invoice", id))
    }
}

fn prefixed_hash(prefix: &str, parts: &[&[u8]]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"invoicekit:rest-shim:v1");
    for part in parts {
        hasher.update(&(part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    let hex = hasher.finalize().to_hex().to_string();
    format!("{prefix}_{}", &hex[..24])
}

fn capabilities_matrix() -> Result<Value, ApiError> {
    serde_json::from_str(CAPABILITIES_MATRIX)
        .map_err(|err| ApiError::internal(format!("capability matrix parse failed: {err}")))
}

fn capability_entry_matches(entry: &Value, query: &CapabilitiesQuery) -> bool {
    field_matches(entry, "route_from", query.from.as_deref())
        && field_matches(entry, "route_to", query.to.as_deref())
        && field_matches(entry, "scenario", query.scenario.as_deref())
        && query
            .date
            .as_deref()
            .is_none_or(|date| entry_valid_on(entry, date))
}

fn field_matches(entry: &Value, key: &str, expected: Option<&str>) -> bool {
    expected.is_none_or(|value| entry.get(key).and_then(Value::as_str) == Some(value))
}

fn entry_valid_on(entry: &Value, date: &str) -> bool {
    let from_ok = entry
        .get("valid_from")
        .and_then(Value::as_str)
        .is_none_or(|from| from <= date);
    let until_ok = entry
        .get("valid_until")
        .and_then(Value::as_str)
        .is_none_or(|until| date <= until);
    from_ok && until_ok
}

fn query_json(query: &CapabilitiesQuery) -> Value {
    json!({
        "from": query.from,
        "to": query.to,
        "date": query.date,
        "scenario": query.scenario,
    })
}

impl From<VerifyReport> for VerifyBundleResponse {
    fn from(report: VerifyReport) -> Self {
        Self {
            ok: report.ok,
            content_address: report.content_address.into(),
            signature: report.signature.into(),
            manifest_envelope: report.manifest_envelope.into(),
            timestamp: report.timestamp.into(),
        }
    }
}

impl From<CheckOutcome> for VerifyCheckOutcome {
    fn from(outcome: CheckOutcome) -> Self {
        match outcome {
            CheckOutcome::Passed => Self::Passed,
            CheckOutcome::Skipped { reason } => Self::Skipped { reason },
            CheckOutcome::Failed { error } => Self::Failed { error },
        }
    }
}

/// Build the deterministic OpenAPI 3.1 document for the REST shim.
///
/// The component schemas are generated from the Rust DTOs with `schemars`.
/// Release tooling serializes this document and publishes a sidecar SHA-256
/// hash so SDK generators can pin the exact contract they consumed.
#[must_use]
pub fn openapi_document() -> Value {
    let components = openapi_components();

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "InvoiceKit REST shim",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Thin REST sidecar over the InvoiceKit engine ABI.",
            "x-invoicekit-generated-from": [
                "EngineProcessResponse",
                "InvoiceResponse",
                "TransmitResponse",
                "TransmissionRecord",
                "ReconcileRequest",
                "ReconcileResponse",
                "VerifyBundleResponse",
                "ApiErrorBody"
            ]
        },
        "servers": [
            {"url": "https://api.invoicekit.org", "description": "Hosted API"},
            {"url": "http://127.0.0.1:8081", "description": "Local REST shim"}
        ],
        "paths": {
            "/v1/engine/process_json": {
                "post": {
                    "operationId": "processEngineAbiJson",
                    "summary": "Process raw Engine ABI JSON",
                    "tags": ["engine"],
                    "requestBody": json_request_body("Raw Engine ABI envelope.", object_schema()),
                    "responses": ok_response("Engine ABI processing result.", schema_ref("EngineProcessResponse"))
                }
            },
            "/v1/invoices": {
                "post": {
                    "operationId": "createInvoice",
                    "summary": "Create an invoice through the engine ABI",
                    "tags": ["invoices"],
                    "parameters": [idempotency_key_header()],
                    "requestBody": json_request_body(
                        "Either a raw Engine ABI envelope or a CommercialDocument JSON object.",
                        object_schema()
                    ),
                    "responses": created_response("Stored invoice metadata.", schema_ref("InvoiceResponse"))
                }
            },
            "/v1/invoices/{id}/validate": {
                "post": {
                    "operationId": "validateInvoice",
                    "summary": "Re-run validation for a stored invoice",
                    "tags": ["invoices"],
                    "parameters": [path_id_parameter("Invoice identifier returned by createInvoice.")],
                    "responses": ok_response("Engine ABI validation result.", schema_ref("EngineProcessResponse"))
                }
            },
            "/v1/invoices/{id}/render": {
                "post": {
                    "operationId": "renderInvoice",
                    "summary": "Render a deterministic PDF for a stored invoice",
                    "tags": ["invoices"],
                    "parameters": [path_id_parameter("Invoice identifier returned by createInvoice.")],
                    "responses": binary_response("application/pdf", "Deterministic PDF bytes.")
                }
            },
            "/v1/invoices/{id}/transmit": {
                "post": {
                    "operationId": "transmitInvoice",
                    "summary": "Submit a stored invoice through the mock gateway",
                    "tags": ["transmissions"],
                    "parameters": [path_id_parameter("Invoice identifier returned by createInvoice.")],
                    "responses": accepted_response("Transmission tracker.", schema_ref("TransmitResponse"))
                }
            },
            "/v1/transmissions/{id}": {
                "get": {
                    "operationId": "getTransmission",
                    "summary": "Return current transmission state",
                    "tags": ["transmissions"],
                    "parameters": [path_id_parameter("Transmission identifier returned by transmitInvoice.")],
                    "responses": ok_response("Transmission state.", schema_ref("TransmissionRecord"))
                }
            },
            "/v1/reconcile": {
                "post": {
                    "operationId": "reconcileInvoices",
                    "summary": "Bulk reconcile invoice identifiers",
                    "tags": ["reconcile"],
                    "requestBody": json_request_body("Invoice identifiers to reconcile.", schema_ref("ReconcileRequest")),
                    "responses": ok_response("Reconciliation result.", schema_ref("ReconcileResponse"))
                }
            },
            "/v1/bundles/{id}": {
                "get": {
                    "operationId": "getEvidenceBundle",
                    "summary": "Download the invoice evidence bundle",
                    "tags": ["evidence"],
                    "parameters": [path_id_parameter("Bundle identifier returned by createInvoice.")],
                    "responses": binary_response("application/vnd.invoicekit.bundle", "Packed .ikb evidence bundle bytes.")
                }
            },
            "/v1/bundles/verify": {
                "post": {
                    "operationId": "verifyEvidenceBundle",
                    "summary": "Verify an uploaded evidence bundle",
                    "tags": ["evidence"],
                    "requestBody": binary_request_body("application/vnd.invoicekit.bundle", "Packed .ikb evidence bundle bytes."),
                    "responses": ok_response("Bundle verification report.", schema_ref("VerifyBundleResponse"))
                }
            },
            "/v1/capabilities": {
                "get": {
                    "operationId": "getCapabilities",
                    "summary": "Lookup country/profile/date capabilities",
                    "tags": ["capabilities"],
                    "parameters": capability_query_parameters(),
                    "responses": ok_response("Capability matrix or filtered capability entries.", object_schema())
                }
            },
            "/v1/openapi.json": openapi_path_item("getVersionedOpenApiDocument"),
            "/openapi.json": openapi_path_item("getOpenApiDocument")
        },
        "components": components
    })
}

/// Serialize the OpenAPI document with a stable trailing newline.
///
/// # Panics
///
/// Panics only if `serde_json` cannot serialize the in-memory JSON value
/// returned by [`openapi_document`].
#[must_use]
pub fn openapi_document_bytes() -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(&openapi_document())
        .expect("OpenAPI document is built from serializable JSON values");
    bytes.push(b'\n');
    bytes
}

/// Return the SHA-256 hash for [`openapi_document_bytes`].
#[must_use]
pub fn openapi_sha256_hex() -> String {
    openapi_sha256_hex_for_bytes(&openapi_document_bytes())
}

fn openapi_sha256_hex_for_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn schema_value<T: JsonSchema>() -> Value {
    serde_json::to_value(schema_for!(T)).expect("schemars output is always serializable")
}

fn openapi_components() -> Value {
    let mut schemas = Map::new();
    insert_schema::<ApiErrorBody>(&mut schemas, "ApiErrorBody");
    insert_schema::<CapabilitiesQuery>(&mut schemas, "CapabilitiesQuery");
    insert_schema::<EngineProcessResponse>(&mut schemas, "EngineProcessResponse");
    insert_schema::<InvoiceResponse>(&mut schemas, "InvoiceResponse");
    insert_schema::<ReconcileRequest>(&mut schemas, "ReconcileRequest");
    insert_schema::<ReconcileResponse>(&mut schemas, "ReconcileResponse");
    insert_schema::<TransmissionRecord>(&mut schemas, "TransmissionRecord");
    insert_schema::<TransmitResponse>(&mut schemas, "TransmitResponse");
    insert_schema::<VerifyBundleResponse>(&mut schemas, "VerifyBundleResponse");
    json!({ "schemas": schemas })
}

fn insert_schema<T: JsonSchema>(schemas: &mut Map<String, Value>, name: &str) {
    let mut schema = schema_value::<T>();
    hoist_schema_defs(&mut schema, schemas);
    schemas.insert(name.to_owned(), schema);
}

fn hoist_schema_defs(schema: &mut Value, schemas: &mut Map<String, Value>) {
    rewrite_schema_refs(schema);
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    let Some(defs) = object.remove("$defs").and_then(|value| match value {
        Value::Object(defs) => Some(defs),
        _ => None,
    }) else {
        return;
    };
    for (name, mut def) in defs {
        hoist_schema_defs(&mut def, schemas);
        schemas.entry(name).or_insert(def);
    }
}

fn rewrite_schema_refs(value: &mut Value) {
    match value {
        Value::Object(object) => {
            let replacement = object
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
                .map(ToOwned::to_owned);
            if let Some(def_name) = replacement {
                object.insert(
                    "$ref".to_owned(),
                    Value::String(format!("#/components/schemas/{def_name}")),
                );
            }
            for child in object.values_mut() {
                rewrite_schema_refs(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_schema_refs(item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn schema_ref(name: &str) -> Value {
    json!({"$ref": format!("#/components/schemas/{name}")})
}

fn object_schema() -> Value {
    json!({"type": "object"})
}

fn json_request_body(description: &str, schema: Value) -> Value {
    json!({
        "required": true,
        "description": description,
        "content": {
            "application/json": {"schema": schema}
        }
    })
}

fn binary_request_body(content_type: &str, description: &str) -> Value {
    json!({
        "required": true,
        "description": description,
        "content": {
            content_type: {"schema": {"type": "string", "format": "binary"}}
        }
    })
}

fn ok_response(description: &str, schema: Value) -> Value {
    response_map("200", description, "application/json", schema)
}

fn created_response(description: &str, schema: Value) -> Value {
    response_map("201", description, "application/json", schema)
}

fn accepted_response(description: &str, schema: Value) -> Value {
    response_map("202", description, "application/json", schema)
}

fn response_map(status: &str, description: &str, content_type: &str, schema: Value) -> Value {
    json!({
        status: {
            "description": description,
            "content": {
                content_type: {"schema": schema}
            }
        },
        "400": error_response("Bad request."),
        "404": error_response("Resource not found."),
        "422": error_response("Request was syntactically valid but could not be processed."),
        "500": error_response("Internal REST shim error.")
    })
}

fn binary_response(content_type: &str, description: &str) -> Value {
    json!({
        "200": {
            "description": description,
            "content": {
                content_type: {"schema": {"type": "string", "format": "binary"}}
            }
        },
        "404": error_response("Resource not found."),
        "500": error_response("Internal REST shim error.")
    })
}

fn error_response(description: &str) -> Value {
    json!({
        "description": description,
        "content": {
            "application/json": {"schema": schema_ref("ApiErrorBody")}
        }
    })
}

fn path_id_parameter(description: &str) -> Value {
    json!({
        "name": "id",
        "in": "path",
        "required": true,
        "description": description,
        "schema": {"type": "string"}
    })
}

fn idempotency_key_header() -> Value {
    json!({
        "name": "Idempotency-Key",
        "in": "header",
        "required": false,
        "description": "Optional idempotency key used to derive the stable invoice identifier.",
        "schema": {"type": "string"}
    })
}

fn capability_query_parameters() -> Vec<Value> {
    [
        ("from", "Source country or runtime route code."),
        ("to", "Destination country or runtime route code."),
        ("date", "Effective date in YYYY-MM-DD form."),
        ("scenario", "Scenario such as B2B or B2G."),
    ]
    .into_iter()
    .map(|(name, description)| {
        json!({
            "name": name,
            "in": "query",
            "required": false,
            "description": description,
            "schema": {"type": "string"}
        })
    })
    .collect()
}

fn openapi_path_item(operation_id: &str) -> Value {
    json!({
        "get": {
            "operationId": operation_id,
            "summary": "Return this OpenAPI 3.1 document",
            "tags": ["metadata"],
            "responses": {
                "200": {
                    "description": "OpenAPI 3.1 document generated from Rust DTO schemas.",
                    "headers": {
                        "x-invoicekit-openapi-sha256": {
                            "description": "SHA-256 hash of the exact response body.",
                            "schema": {"type": "string", "pattern": "^[a-f0-9]{64}$"}
                        }
                    },
                    "content": {
                        "application/json": {"schema": object_schema()}
                    }
                }
            }
        }
    })
}

/// Standard REST-shim error envelope.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct ApiErrorBody {
    /// Error payload.
    pub error: ApiErrorInner,
}

/// Stable error payload for SDK consumers.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct ApiErrorInner {
    /// Stable error code.
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// Remediation hint.
    pub remediation: String,
}

#[derive(Debug, Error)]
enum ApiError {
    #[error("request body is not valid JSON: {reason}")]
    BadJson { reason: String },
    #[error("engine rejected the invoice request")]
    UnprocessableEngineResponse { response: EngineProcessResponse },
    #[error("{kind} {id} was not found")]
    NotFound { kind: &'static str, id: String },
    #[error("bundle verification failed: {0}")]
    BadBundle(String),
    #[error("internal REST shim error: {0}")]
    Internal(String),
}

impl ApiError {
    fn internal(reason: impl Into<String>) -> Self {
        Self::Internal(reason.into())
    }

    fn not_found(kind: &'static str, id: &str) -> Self {
        Self::NotFound {
            kind,
            id: id.to_owned(),
        }
    }

    const fn status(&self) -> StatusCode {
        match self {
            Self::BadJson { .. } => StatusCode::BAD_REQUEST,
            Self::UnprocessableEngineResponse { .. } | Self::BadBundle(_) => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    const fn code(&self) -> &'static str {
        match self {
            Self::BadJson { .. } => "bad_json",
            Self::UnprocessableEngineResponse { .. } => "engine_rejected_invoice",
            Self::NotFound { .. } => "not_found",
            Self::BadBundle(_) => "bad_bundle",
            Self::Internal(_) => "internal_error",
        }
    }

    fn message(&self) -> String {
        match self {
            Self::UnprocessableEngineResponse { response } => {
                format!(
                    "engine returned status {}; response_base64={}",
                    response.status, response.response_base64
                )
            }
            _ => self.to_string(),
        }
    }

    const fn remediation(&self) -> &'static str {
        match self {
            Self::BadJson { .. } => "Send a JSON object or raw Engine ABI JSON envelope.",
            Self::UnprocessableEngineResponse { .. } => {
                "Decode response_base64 and fix the engine ABI validation error."
            }
            Self::NotFound { .. } => "Check the identifier returned by the create/transmit call.",
            Self::BadBundle(_) => "Upload bytes returned by GET /v1/bundles/{id}.",
            Self::Internal(_) => "Retry with the same input and report the deterministic failure.",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = ApiErrorBody {
            error: ApiErrorInner {
                code: self.code().to_owned(),
                message: self.message(),
                remediation: self.remediation().to_owned(),
            },
        };
        (status, Json(body)).into_response()
    }
}

/// Canonical Cargo package name of this crate.
///
/// Used by the InvoiceKit release tooling and by the bead-correlation
/// reports to map runtime log records back to the originating crate
/// without parsing `Cargo.toml` at runtime.
///
/// # Examples
///
/// ```
/// assert_eq!(invoicekit_binding_rest_shim::crate_name(), "invoicekit-binding-rest-shim");
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-binding-rest-shim"
}

#[cfg(test)]
mod tests {
    use super::{
        build_router, crate_name, openapi_document, openapi_document_bytes, openapi_sha256_hex,
        process_engine_abi_json, EngineProcessResponse,
    };
    use axum::body::{Body, Bytes};
    use axum::http::{header, Method, Request, StatusCode};
    use http_body_util::BodyExt;
    use serde::Deserialize;
    use serde_json::Value;
    use sha2::{Digest as _, Sha256};
    use tower::ServiceExt;

    const GOLDEN_FIXTURE: &str =
        include_str!("../../../conformance-corpus/golden/engine-abi-v1-commercial-document.json");

    #[derive(Debug, Deserialize)]
    struct GoldenFixture {
        request_bytes: String,
        expected_response_bytes: String,
    }

    fn golden_fixture() -> GoldenFixture {
        serde_json::from_str(GOLDEN_FIXTURE).expect("golden fixture is valid JSON")
    }

    #[test]
    fn crate_name_is_cargo_package_name() {
        assert_eq!(crate_name(), "invoicekit-binding-rest-shim");
    }

    #[test]
    fn crate_name_is_non_empty() {
        assert!(!crate_name().is_empty());
    }

    #[test]
    fn crate_name_is_lowercase_kebab() {
        let n = crate_name();
        for c in n.chars() {
            assert!(
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-',
                "non-kebab char in {n}: {c:?}"
            );
        }
    }

    #[test]
    fn crate_name_carries_invoicekit_prefix() {
        let n = crate_name();
        assert!(
            n == "invoicekit" || n.starts_with("invoicekit-") || n.starts_with("invoicekit_"),
            "crate name does not advertise InvoiceKit family: {n}"
        );
    }

    #[test]
    fn rest_shim_wrapper_matches_engine_abi_golden_fixture() {
        let fixture = golden_fixture();
        assert_eq!(
            process_engine_abi_json(fixture.request_bytes.as_bytes()),
            fixture.expected_response_bytes.as_bytes()
        );
    }

    #[tokio::test]
    async fn engine_process_endpoint_matches_go_fallback_contract() {
        let fixture = golden_fixture();
        let response = build_router()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/engine/process_json")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(fixture.request_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: EngineProcessResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.status, 0);
        assert!(!parsed.response_base64.is_empty());
    }

    #[tokio::test]
    async fn invoice_lifecycle_routes_work_through_http_client() {
        let fixture = golden_fixture();
        let app = build_router();
        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/invoices")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("idempotency-key", "rest-shim-test")
                    .body(Body::from(fixture.request_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_response.status(), StatusCode::CREATED);
        let created: Value = serde_json::from_slice(&response_body(create_response).await).unwrap();
        let invoice_id = created["id"].as_str().unwrap();
        assert_eq!(created["engine_status"], 0);

        let validate_response = app
            .clone()
            .oneshot(empty_request(
                Method::POST,
                &format!("/v1/invoices/{invoice_id}/validate"),
            ))
            .await
            .unwrap();
        assert_eq!(validate_response.status(), StatusCode::OK);

        let render_response = app
            .clone()
            .oneshot(empty_request(
                Method::POST,
                &format!("/v1/invoices/{invoice_id}/render"),
            ))
            .await
            .unwrap();
        assert_eq!(render_response.status(), StatusCode::OK);
        assert!(response_body(render_response).await.starts_with(b"%PDF-"));

        let transmit_response = app
            .clone()
            .oneshot(empty_request(
                Method::POST,
                &format!("/v1/invoices/{invoice_id}/transmit"),
            ))
            .await
            .unwrap();
        assert_eq!(transmit_response.status(), StatusCode::ACCEPTED);
        let transmitted: Value =
            serde_json::from_slice(&response_body(transmit_response).await).unwrap();
        let transmission_id = transmitted["transmission_id"].as_str().unwrap();

        let state_response = app
            .clone()
            .oneshot(empty_request(
                Method::GET,
                &format!("/v1/transmissions/{transmission_id}"),
            ))
            .await
            .unwrap();
        assert_eq!(state_response.status(), StatusCode::OK);

        let reconcile_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/reconcile")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(format!(
                        r#"{{"invoice_ids":["{invoice_id}","missing"]}}"#
                    )))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reconcile_response.status(), StatusCode::OK);
        let reconciled: Value =
            serde_json::from_slice(&response_body(reconcile_response).await).unwrap();
        assert_eq!(reconciled["matches"][0]["present"], true);
        assert_eq!(reconciled["matches"][1]["present"], false);

        let bundle_response = app
            .clone()
            .oneshot(empty_request(
                Method::GET,
                &format!("/v1/bundles/{invoice_id}"),
            ))
            .await
            .unwrap();
        assert_eq!(bundle_response.status(), StatusCode::OK);
        let bundle = response_body(bundle_response).await;
        assert!(!bundle.is_empty());

        let verify_response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/bundles/verify")
                    .body(Body::from(bundle))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(verify_response.status(), StatusCode::OK);
        let verified: Value =
            serde_json::from_slice(&response_body(verify_response).await).unwrap();
        assert_eq!(verified["ok"], true);
    }

    #[tokio::test]
    async fn capabilities_route_filters_by_country_scenario_and_date() {
        let response = build_router()
            .oneshot(empty_request(
                Method::GET,
                "/v1/capabilities?from=DE&to=DE&scenario=B2B&date=2026-01-01",
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(&response_body(response).await).unwrap();
        assert_eq!(body["entries"].as_array().unwrap().len(), 1);
        assert_eq!(body["entries"][0]["route_from"], "DE");
    }

    #[tokio::test]
    async fn openapi_lists_plan_section_5_5_routes() {
        let response = build_router()
            .oneshot(empty_request(Method::GET, "/openapi.json"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = serde_json::from_slice(&response_body(response).await).unwrap();
        let paths = body["paths"].as_object().unwrap();
        for path in [
            "/v1/invoices",
            "/v1/invoices/{id}/validate",
            "/v1/invoices/{id}/render",
            "/v1/invoices/{id}/transmit",
            "/v1/transmissions/{id}",
            "/v1/reconcile",
            "/v1/bundles/{id}",
            "/v1/bundles/verify",
            "/v1/capabilities",
        ] {
            assert!(paths.contains_key(path), "OpenAPI missing {path}");
        }
    }

    #[test]
    fn openapi_document_uses_generated_component_schemas() {
        let document = openapi_document();
        assert_eq!(document["openapi"], "3.1.0");
        let generated_from = document["info"]["x-invoicekit-generated-from"]
            .as_array()
            .unwrap();
        assert!(generated_from.iter().any(|name| name == "InvoiceResponse"));
        let schemas = document["components"]["schemas"].as_object().unwrap();
        for schema in [
            "ApiErrorBody",
            "EngineProcessResponse",
            "InvoiceResponse",
            "ReconcileRequest",
            "ReconcileResponse",
            "TransmissionRecord",
            "TransmitResponse",
            "VerifyBundleResponse",
        ] {
            assert!(schemas.contains_key(schema), "OpenAPI missing {schema}");
        }
        assert_eq!(
            document["paths"]["/v1/invoices"]["post"]["operationId"],
            "createInvoice"
        );
        assert_eq!(
            schemas["InvoiceResponse"]["properties"]["id"]["type"],
            "string"
        );
    }

    #[test]
    fn openapi_document_hash_matches_serialized_bytes() {
        let bytes = openapi_document_bytes();
        assert!(bytes.ends_with(b"\n"));
        assert_eq!(
            openapi_sha256_hex(),
            format!("{:x}", Sha256::digest(&bytes))
        );
    }

    #[tokio::test]
    async fn openapi_route_carries_response_body_sha256() {
        let response = build_router()
            .oneshot(empty_request(Method::GET, "/v1/openapi.json"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let header_hash = response
            .headers()
            .get("x-invoicekit-openapi-sha256")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let body = response_body(response).await;
        assert_eq!(header_hash, format!("{:x}", Sha256::digest(&body)));
    }

    #[tokio::test]
    async fn missing_invoice_returns_stable_error_envelope() {
        let response = build_router()
            .oneshot(empty_request(Method::POST, "/v1/invoices/nope/validate"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body: Value = serde_json::from_slice(&response_body(response).await).unwrap();
        assert_eq!(body["error"]["code"], "not_found");
    }

    fn empty_request(method: Method, uri: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    async fn response_body(response: axum::response::Response) -> Bytes {
        response.into_body().collect().await.unwrap().to_bytes()
    }
}
