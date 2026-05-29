// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-signer-agent` — local signer daemon.
//!
//! Listens on a Unix socket (default
//! `/run/invoicekit/signer.sock`, overridable via
//! `INVOICEKIT_SIGNER_SOCK`) and serves the JSON-RPC contract:
//!
//! | Method | Input | Output |
//! | --- | --- | --- |
//! | `list_keys` | `{}` | `{ "keys": ["key-id-1", ...] }` |
//! | `sign` | `{ "key_ref": "...", "payload_b64": "..." }` | `{ "key_ref": "...", "algorithm": "...", "signature_b64": "..." }` |
//! | `ping` | `{}` | `{ "version": "0.1.0" }` |
//!
//! The actual signing happens in `invoicekit-signer` so the
//! engine, the cassette-replay sandbox, and the daemon all share
//! the same [`Signer`] surface. The daemon is the on-host
//! boundary so customer keys never enter the engine's process.

use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::Arc;

use invoicekit_signer::{KeyRef, SignRequest, Signer, SoftwareSigner};
use serde::{Deserialize, Serialize};

const DAEMON_VERSION: &str = "0.1.0";

fn main() {
    let socket_path = env::var("INVOICEKIT_SIGNER_SOCK").map_or_else(
        |_| PathBuf::from("/run/invoicekit/signer.sock"),
        PathBuf::from,
    );

    // The scaffold ships a deterministic in-memory keyring so
    // the daemon is testable without operator key provisioning;
    // the real backend (env-var-driven file paths, HSM slots,
    // KMS key ids) lands in the per-provider follow-ups
    // T-083a / T-083b.
    let signer: Arc<dyn Signer> = Arc::new(
        SoftwareSigner::new()
            .with_key("scaffold/default", [0_u8; 32])
            .with_key("scaffold/test", [1_u8; 32]),
    );

    let socket_str = socket_path.display();
    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(err) => {
            eprintln!("signer-agent: cannot bind {socket_str}: {err}");
            std::process::exit(1);
        }
    };
    eprintln!("signer-agent: listening on {socket_str}");
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let signer = Arc::clone(&signer);
                let spawn_result = std::thread::Builder::new()
                    .name("invoicekit-signer-agent-client".to_owned())
                    .spawn(move || {
                        let reader_stream = match stream.try_clone() {
                            Ok(reader_stream) => reader_stream,
                            Err(err) => {
                                let _ = writeln!(
                                    stream,
                                    r#"{{"error":"stream clone failure: {err}"}}"#
                                );
                                return;
                            }
                        };
                        let mut reader = BufReader::new(reader_stream);
                        let mut line = String::new();
                        if reader.read_line(&mut line).is_err() {
                            return;
                        }
                        let response = dispatch(&signer, &line);
                        let body = serde_json::to_string(&response).unwrap_or_else(|err| {
                            format!(r#"{{"error":"response serialise failure: {err}"}}"#)
                        });
                        let _ = writeln!(stream, "{body}");
                    });
                if let Err(err) = spawn_result {
                    eprintln!("signer-agent: cannot spawn client handler: {err}");
                }
            }
            Err(err) => {
                eprintln!("signer-agent: accept error: {err}");
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum RpcResponse {
    Ok(serde_json::Value),
    Err { error: String },
}

fn dispatch(signer: &Arc<dyn Signer>, body: &str) -> RpcResponse {
    let req: RpcRequest = match serde_json::from_str(body.trim()) {
        Ok(r) => r,
        Err(err) => {
            return RpcResponse::Err {
                error: format!("malformed JSON-RPC body: {err}"),
            }
        }
    };
    match req.method.as_str() {
        "ping" => RpcResponse::Ok(serde_json::json!({ "version": DAEMON_VERSION })),
        "list_keys" => {
            let keys: Vec<String> = signer.list_keys().into_iter().map(|k| k.0).collect();
            RpcResponse::Ok(serde_json::json!({ "keys": keys }))
        }
        "sign" => match decode_sign_params(&req.params) {
            Ok(sign_req) => match signer.sign(&sign_req) {
                Ok(sig) => match serde_json::to_value(&sig) {
                    Ok(v) => RpcResponse::Ok(v),
                    Err(err) => RpcResponse::Err {
                        error: format!("signature serialise failure: {err}"),
                    },
                },
                Err(err) => RpcResponse::Err {
                    error: err.to_string(),
                },
            },
            Err(err) => RpcResponse::Err { error: err },
        },
        other => RpcResponse::Err {
            error: format!("unknown method: {other}"),
        },
    }
}

fn decode_sign_params(params: &serde_json::Value) -> Result<SignRequest, String> {
    let key_ref = params
        .get("key_ref")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing string field: key_ref".to_owned())?
        .to_owned();
    let payload_b64 = params
        .get("payload_b64")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing string field: payload_b64".to_owned())?;
    let payload =
        decode_base64_strict(payload_b64).map_err(|err| format!("payload_b64 invalid: {err}"))?;
    Ok(SignRequest {
        key_ref: KeyRef::new(key_ref),
        payload,
    })
}

fn decode_base64_strict(input: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    let mut padding = 0_u8;
    for byte in input.bytes() {
        let val = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding += 1;
                continue;
            }
            other => return Err(format!("non-base64 byte: {other:#x}")),
        };
        if padding > 0 {
            return Err("non-padding byte after padding".to_owned());
        }
        buf = (buf << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signer() -> Arc<dyn Signer> {
        Arc::new(SoftwareSigner::new().with_key("scaffold/default", [9_u8; 32]))
    }

    fn assert_ok_response(response: RpcResponse) -> serde_json::Value {
        assert!(
            matches!(response, RpcResponse::Ok(_)),
            "expected ok response"
        );
        let RpcResponse::Ok(value) = response else {
            return serde_json::Value::Null;
        };
        value
    }

    fn assert_err_response(response: RpcResponse) -> String {
        assert!(
            matches!(response, RpcResponse::Err { .. }),
            "expected error response"
        );
        let RpcResponse::Err { error } = response else {
            return String::new();
        };
        error
    }

    #[test]
    fn dispatch_ping_returns_version() {
        let signer = signer();
        let response = dispatch(&signer, r#"{"method":"ping"}"#);
        let v = assert_ok_response(response);
        assert_eq!(
            v.get("version").and_then(|x| x.as_str()),
            Some(DAEMON_VERSION)
        );
    }

    #[test]
    fn dispatch_list_keys_returns_registered_keys() {
        let signer = signer();
        let response = dispatch(&signer, r#"{"method":"list_keys"}"#);
        let v = assert_ok_response(response);
        let keys = v
            .get("keys")
            .and_then(|k| k.as_array())
            .expect("keys array");
        let names: Vec<&str> = keys.iter().filter_map(|s| s.as_str()).collect();
        assert_eq!(names, vec!["scaffold/default"]);
    }

    #[test]
    fn dispatch_sign_returns_signature() {
        let signer = signer();
        // payload "hello" base64 = "aGVsbG8="
        let response = dispatch(
            &signer,
            r#"{"method":"sign","params":{"key_ref":"scaffold/default","payload_b64":"aGVsbG8="}}"#,
        );
        let v = assert_ok_response(response);
        assert_eq!(
            v.get("algorithm").and_then(|s| s.as_str()),
            Some("blake3-keyed-256")
        );
        assert!(!v
            .get("signature_b64")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .is_empty());
    }

    #[test]
    fn dispatch_sign_rejects_unknown_key() {
        let signer = signer();
        let response = dispatch(
            &signer,
            r#"{"method":"sign","params":{"key_ref":"nope","payload_b64":"aGVsbG8="}}"#,
        );
        let error = assert_err_response(response);
        assert!(error.contains("unknown key"), "got: {error}");
    }

    #[test]
    fn dispatch_rejects_malformed_json() {
        let signer = signer();
        let response = dispatch(&signer, "not json");
        let error = assert_err_response(response);
        assert!(error.contains("malformed"));
    }

    #[test]
    fn dispatch_rejects_unknown_method() {
        let signer = signer();
        let response = dispatch(&signer, r#"{"method":"evict-cache"}"#);
        let error = assert_err_response(response);
        assert!(error.contains("unknown method"));
    }

    #[test]
    fn decode_base64_strict_round_trips() {
        // "Man" -> "TWFu"
        assert_eq!(decode_base64_strict("TWFu").unwrap(), b"Man");
        // "Ma" -> "TWE="
        assert_eq!(decode_base64_strict("TWE=").unwrap(), b"Ma");
        // empty
        assert_eq!(decode_base64_strict("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn decode_base64_strict_rejects_non_alphabet() {
        let err = decode_base64_strict("Z!").unwrap_err();
        assert!(err.contains("non-base64 byte"));
    }
}
