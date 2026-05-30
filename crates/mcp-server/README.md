# invoicekit-mcp-server

Type definitions and a deterministic mock for an InvoiceKit Model Context Protocol (MCP) server. It captures the request/response/tool/resource shapes an MCP-aware client (Claude Code, Cursor, etc.) would use; it does not implement a live server or call the engine.

## What it does

MCP is the protocol an editor or agent uses to discover and invoke tools over JSON-RPC. This crate models the InvoiceKit side of that contract: the `initialize` server info and capabilities, the `tools/list` and `tools/call` shapes, the `resources/list` and `resources/read` shapes, and a typed error enum.

It ships `MockMcpServer`, an in-memory implementation of the `McpServer` trait that returns hardcoded, deterministic JSON for four tool names. The mock validates that required arguments are present and are strings; beyond that it does no work — it never reads a bundle, never invokes verify/replay/show/pack, and never touches the filesystem. There is no JSON-RPC transport and no stdio loop in this crate.

The crate is transport-agnostic by design so it can double as a test harness for the eventual live server.

## Capabilities

- `McpServer` trait: `server_info`, `capabilities`, `list_tools`, `call_tool`, `list_resources`, `read_resource`.
- Serde-serializable wire types: `ServerInfo`, `ServerCapabilities` (defaults to `tools: true`, `resources: true`, `prompts: false`), `ToolDescriptor`, `ResourceDescriptor`, `ToolCallResult`, `ResourceReadResult`.
- `McpError`: `UnknownTool`, `UnknownResource`, `MissingArgument`, `InvalidArgument { arg, reason }`.
- `MockMcpServer`: advertises `invoicekit/0.0.0`, lists four tools, and serves one resource.
- Argument shape checking in the mock: missing or non-string required arguments fail with a typed `McpError`.
- `crate_name()` — returns `"invoicekit-mcp-server"`.

The four tool descriptors the mock advertises, with their JSON-Schema argument shapes:

- `verify_bundle` — required `bundle` (string).
- `replay_bundle` — required `bundle` (string); optional `mutate` (array of strings).
- `show_bundle` — required `bundle` (string).
- `pack_bundle` — required `input_dir`, `output` (strings); optional `tenant`, `trace`, `created_at`.

The one resource the mock serves is `invoicekit://docs/CLI.md` (a placeholder markdown stub pointing back at `docs/CLI.md` in the repo).

## Mode / Residuals

This crate is a mock and a set of wire types, not a working MCP server.

- **Mock tool results are placeholder JSON, not engine output.** `verify_bundle` always returns `ok: true` with `{ "content_address": "passed" }` regardless of the bundle. `replay_bundle` returns empty `deltas`. `show_bundle` returns a fixed `schema_version: "1.0"` and `tenant_id: "mock"`. `pack_bundle` returns `artefacts: 0`. The argument value is echoed back; it is never opened or checked.
- **No engine wiring.** The live path is meant to call `invoicekit-verify`, `invoicekit-replay`, `invoicekit-evidence`, and the country signers. None of those are dependencies here (the only dependencies are `serde`, `serde_json`, and `thiserror`), and none are invoked.
- **No transport.** There is no JSON-RPC framing and no stdio glue. The crate is `publish = false`. The doc-comment notes the live stdio glue is expected in a separate `invoicekit-mcp-server-bin` binary; that binary does not exist in this crate.
- **`prompts/*` is not modelled** beyond the `prompts: false` capability flag; there are no prompt types or methods.

## References

- Model Context Protocol (MCP) — the protocol whose request/response, `tools/*`, and `resources/*` shapes this crate models. Referenced by name in the crate documentation; no specification URL appears in the source.

## License

Apache-2.0. See the workspace root for the full text.
