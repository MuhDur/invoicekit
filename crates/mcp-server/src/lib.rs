// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! InvoiceKit **MCP server** substrate.
//!
//! Captures the Model Context Protocol (MCP) request /
//! response / tool / resource shapes so an MCP-aware client
//! (Claude Code, Cursor, etc.) can drive the invoicekit
//! engine over JSON-RPC. The live stdio JSON-RPC glue lands
//! in a follow-up `invoicekit-mcp-server-bin` binary; this
//! crate stays transport-agnostic so it doubles as a test
//! harness.
//!
//! Tools shipped today (mock implementations all return
//! deterministic placeholder JSON; the live impl wires
//! through to `invoicekit-verify`, `invoicekit-replay`,
//! `invoicekit-evidence`, and the country signers):
//!
//! * `verify_bundle` — wraps `invoicekit verify`.
//! * `replay_bundle` — wraps `invoicekit replay`.
//! * `show_bundle` — wraps `invoicekit show`.
//! * `pack_bundle` — wraps `invoicekit pack`.

#![allow(clippy::doc_markdown)]

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use thiserror::Error;

/// MCP server-info advertised in the `initialize` response.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Server name.
    pub name: String,
    /// Server version (semver).
    pub version: String,
}

/// MCP server capabilities advertised in the `initialize`
/// response.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServerCapabilities {
    /// Whether the server publishes `tools/*` methods.
    pub tools: bool,
    /// Whether the server publishes `resources/*` methods.
    pub resources: bool,
    /// Whether the server publishes `prompts/*` methods.
    pub prompts: bool,
}

impl Default for ServerCapabilities {
    fn default() -> Self {
        Self {
            tools: true,
            resources: true,
            prompts: false,
        }
    }
}

/// One tool the server exposes via `tools/list`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    /// Stable name (used in `tools/call`).
    pub name: String,
    /// One-line summary.
    pub description: String,
    /// JSON-Schema describing the tool's `arguments`.
    pub input_schema: JsonValue,
}

/// One resource the server exposes via `resources/list`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourceDescriptor {
    /// Resource URI (`invoicekit://...` scheme).
    pub uri: String,
    /// Human-readable name.
    pub name: String,
    /// Optional MIME type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// What `tools/call` returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolCallResult {
    /// Whether the call succeeded.
    pub ok: bool,
    /// Output payload (typed JSON the caller renders).
    pub content: JsonValue,
}

/// What `resources/read` returns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourceReadResult {
    /// Resource URI echoed back.
    pub uri: String,
    /// Resource MIME type.
    pub mime_type: String,
    /// Resource bytes (UTF-8 text for JSON, base64 for
    /// binary).
    pub text: String,
}

/// Typed transport / validation errors.
#[derive(Debug, Error)]
pub enum McpError {
    /// Unknown tool name in `tools/call`.
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    /// Unknown resource URI in `resources/read`.
    #[error("unknown resource: {0}")]
    UnknownResource(String),
    /// Required tool argument missing.
    #[error("missing required argument {0:?}")]
    MissingArgument(String),
    /// Tool argument failed shape validation.
    #[error("invalid argument {arg:?}: {reason}")]
    InvalidArgument {
        /// Argument name.
        arg: String,
        /// Reason text.
        reason: String,
    },
}

/// MCP server surface. Real stdio JSON-RPC servers wrap a
/// type that implements this trait + glue the methods to
/// JSON-RPC requests.
pub trait McpServer: Send + Sync {
    /// Server info / capabilities advertised in `initialize`.
    fn server_info(&self) -> ServerInfo;
    /// Server capabilities advertised in `initialize`.
    fn capabilities(&self) -> ServerCapabilities;
    /// Handle `tools/list`.
    fn list_tools(&self) -> Vec<ToolDescriptor>;
    /// Handle `tools/call`.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] when the tool is unknown or its
    /// arguments fail validation.
    fn call_tool(&self, name: &str, arguments: &JsonValue) -> Result<ToolCallResult, McpError>;
    /// Handle `resources/list`.
    fn list_resources(&self) -> Vec<ResourceDescriptor>;
    /// Handle `resources/read`.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::UnknownResource`] when the URI is
    /// not recognised.
    fn read_resource(&self, uri: &str) -> Result<ResourceReadResult, McpError>;
}

/// Deterministic mock MCP server implementing the invoicekit
/// tool set.
pub struct MockMcpServer {
    info: ServerInfo,
}

impl MockMcpServer {
    /// Build a mock with default `invoicekit/0.0.0` info.
    #[must_use]
    pub fn new() -> Self {
        Self {
            info: ServerInfo {
                name: "invoicekit".to_owned(),
                version: "0.0.0".to_owned(),
            },
        }
    }
}

impl Default for MockMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

impl McpServer for MockMcpServer {
    fn server_info(&self) -> ServerInfo {
        self.info.clone()
    }

    fn capabilities(&self) -> ServerCapabilities {
        ServerCapabilities::default()
    }

    fn list_tools(&self) -> Vec<ToolDescriptor> {
        vec![
            ToolDescriptor {
                name: "verify_bundle".to_owned(),
                description: "Run invoicekit verify against a .ikb bundle path".to_owned(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": { "bundle": { "type": "string" } },
                    "required": ["bundle"],
                }),
            },
            ToolDescriptor {
                name: "replay_bundle".to_owned(),
                description: "Run invoicekit replay against a .ikb bundle path".to_owned(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "bundle": { "type": "string" },
                        "mutate": { "type": "array", "items": { "type": "string" } },
                    },
                    "required": ["bundle"],
                }),
            },
            ToolDescriptor {
                name: "show_bundle".to_owned(),
                description: "Print a .ikb bundle's manifest summary".to_owned(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": { "bundle": { "type": "string" } },
                    "required": ["bundle"],
                }),
            },
            ToolDescriptor {
                name: "pack_bundle".to_owned(),
                description: "Pack a directory of artefacts into a .ikb bundle".to_owned(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "input_dir": { "type": "string" },
                        "output": { "type": "string" },
                        "tenant": { "type": "string" },
                        "trace": { "type": "string" },
                        "created_at": { "type": "string" },
                    },
                    "required": ["input_dir", "output"],
                }),
            },
        ]
    }

    fn call_tool(&self, name: &str, arguments: &JsonValue) -> Result<ToolCallResult, McpError> {
        match name {
            "verify_bundle" => {
                let bundle = require_string(arguments, "bundle")?;
                Ok(ToolCallResult {
                    ok: true,
                    content: serde_json::json!({
                        "ok": true,
                        "bundle": bundle,
                        "checks": { "content_address": "passed" },
                    }),
                })
            }
            "replay_bundle" => {
                let bundle = require_string(arguments, "bundle")?;
                Ok(ToolCallResult {
                    ok: true,
                    content: serde_json::json!({
                        "ok": true,
                        "bundle": bundle,
                        "deltas": {},
                    }),
                })
            }
            "show_bundle" => {
                let bundle = require_string(arguments, "bundle")?;
                Ok(ToolCallResult {
                    ok: true,
                    content: serde_json::json!({
                        "bundle": bundle,
                        "schema_version": "1.0",
                        "tenant_id": "mock",
                    }),
                })
            }
            "pack_bundle" => {
                let input_dir = require_string(arguments, "input_dir")?;
                let output = require_string(arguments, "output")?;
                Ok(ToolCallResult {
                    ok: true,
                    content: serde_json::json!({
                        "input_dir": input_dir,
                        "output": output,
                        "artefacts": 0,
                    }),
                })
            }
            other => Err(McpError::UnknownTool(other.to_owned())),
        }
    }

    fn list_resources(&self) -> Vec<ResourceDescriptor> {
        vec![ResourceDescriptor {
            uri: "invoicekit://docs/CLI.md".to_owned(),
            name: "invoicekit CLI walkthrough".to_owned(),
            mime_type: Some("text/markdown".to_owned()),
        }]
    }

    fn read_resource(&self, uri: &str) -> Result<ResourceReadResult, McpError> {
        if uri == "invoicekit://docs/CLI.md" {
            Ok(ResourceReadResult {
                uri: uri.to_owned(),
                mime_type: "text/markdown".to_owned(),
                text: "# invoicekit CLI\n\nSee docs/CLI.md in the repo.\n".to_owned(),
            })
        } else {
            Err(McpError::UnknownResource(uri.to_owned()))
        }
    }
}

fn require_string<'a>(args: &'a JsonValue, key: &str) -> Result<&'a str, McpError> {
    args.get(key)
        .ok_or_else(|| McpError::MissingArgument(key.to_owned()))
        .and_then(|v| {
            v.as_str().ok_or_else(|| McpError::InvalidArgument {
                arg: key.to_owned(),
                reason: format!("must be a string, got {v}"),
            })
        })
}

/// Canonical Cargo package name of this crate.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     invoicekit_mcp_server::crate_name(),
///     "invoicekit-mcp-server"
/// );
/// ```
#[must_use]
pub const fn crate_name() -> &'static str {
    "invoicekit-mcp-server"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_info_and_capabilities() {
        let s = MockMcpServer::default();
        let info = s.server_info();
        assert_eq!(info.name, "invoicekit");
        let caps = s.capabilities();
        assert!(caps.tools);
        assert!(caps.resources);
        assert!(!caps.prompts);
    }

    #[test]
    fn list_tools_exposes_four_invoicekit_tools() {
        let s = MockMcpServer::default();
        let tools = s.list_tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "verify_bundle",
                "replay_bundle",
                "show_bundle",
                "pack_bundle"
            ]
        );
    }

    #[test]
    fn call_tool_verify_returns_ok_envelope() {
        let s = MockMcpServer::default();
        let res = s
            .call_tool(
                "verify_bundle",
                &serde_json::json!({ "bundle": "/tmp/x.ikb" }),
            )
            .unwrap();
        assert!(res.ok);
        assert_eq!(res.content["bundle"], "/tmp/x.ikb");
    }

    #[test]
    fn call_tool_replay_returns_ok_envelope() {
        let s = MockMcpServer::default();
        let res = s
            .call_tool(
                "replay_bundle",
                &serde_json::json!({ "bundle": "/tmp/x.ikb" }),
            )
            .unwrap();
        assert!(res.ok);
    }

    #[test]
    fn call_tool_show_returns_ok_envelope() {
        let s = MockMcpServer::default();
        let res = s
            .call_tool(
                "show_bundle",
                &serde_json::json!({ "bundle": "/tmp/x.ikb" }),
            )
            .unwrap();
        assert!(res.ok);
        assert_eq!(res.content["schema_version"], "1.0");
    }

    #[test]
    fn call_tool_pack_returns_ok_envelope() {
        let s = MockMcpServer::default();
        let res = s
            .call_tool(
                "pack_bundle",
                &serde_json::json!({
                    "input_dir": "/tmp/in",
                    "output": "/tmp/out.ikb",
                }),
            )
            .unwrap();
        assert!(res.ok);
    }

    #[test]
    fn call_tool_rejects_unknown_name() {
        let s = MockMcpServer::default();
        let err = s
            .call_tool("does-not-exist", &serde_json::json!({}))
            .unwrap_err();
        assert!(matches!(err, McpError::UnknownTool(_)));
    }

    #[test]
    fn call_tool_rejects_missing_argument() {
        let s = MockMcpServer::default();
        let err = s
            .call_tool("verify_bundle", &serde_json::json!({}))
            .unwrap_err();
        assert!(matches!(err, McpError::MissingArgument(_)));
    }

    #[test]
    fn call_tool_rejects_non_string_argument() {
        let s = MockMcpServer::default();
        let err = s
            .call_tool("verify_bundle", &serde_json::json!({ "bundle": 42 }))
            .unwrap_err();
        assert!(matches!(err, McpError::InvalidArgument { .. }));
    }

    #[test]
    fn read_resource_returns_known_doc() {
        let s = MockMcpServer::default();
        let r = s.read_resource("invoicekit://docs/CLI.md").unwrap();
        assert_eq!(r.mime_type, "text/markdown");
        assert!(r.text.contains("invoicekit CLI"));
    }

    #[test]
    fn read_resource_rejects_unknown_uri() {
        let s = MockMcpServer::default();
        let err = s.read_resource("invoicekit://does/not/exist").unwrap_err();
        assert!(matches!(err, McpError::UnknownResource(_)));
    }

    #[test]
    fn tool_descriptor_serde_round_trips() {
        let t = ToolDescriptor {
            name: "x".to_owned(),
            description: "y".to_owned(),
            input_schema: serde_json::json!({"type":"object"}),
        };
        let json = serde_json::to_string(&t).unwrap();
        let parsed: ToolDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, t);
    }
}
