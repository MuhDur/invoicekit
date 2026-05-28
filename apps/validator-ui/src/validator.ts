// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

/// Validator-mode taxonomy and dispatch.
///
/// The validator UI runs in one of two explicit modes per
/// T-035:
///
/// * `local` — InvoiceKit WASM bundle from `bindings/wasm-browser`
///   runs entirely in the browser. The XML never leaves the device.
/// * `reference` — UI POSTs the XML to the JVM validator sidecar
///   service (`validator-kosit` / `validator-phive`). Clearly
///   labeled; no retention by default.

export type ValidatorMode = "local" | "reference";

export interface ValidationFinding {
  rule_id: string;
  severity: "fatal" | "error" | "warning";
  message: string;
  location?: string;
}

export interface ValidationResult {
  mode: ValidatorMode;
  rule_pack_version: string;
  backend: string;
  findings: ValidationFinding[];
  elapsed_ms: number;
}

export interface ReferenceClientConfig {
  /** Base URL of the reference validator (JVM sidecar). */
  baseUrl: string;
  /** Optional bearer token; default deployment is anonymous. */
  bearerToken?: string;
  /** Hard deadline for the round-trip. */
  timeoutMs?: number;
}

export const DEFAULT_RULE_PACK_VERSION = "en16931-2017+peppol-bis-3.0.18";

/**
 * Validate `xml` in local (WASM) mode. The implementation
 * loads the `invoicekit-wasm-browser` ESM bundle that
 * `bindings/wasm-browser` ships; today's scaffold returns a
 * deterministic stub so the SPA renders end-to-end without
 * the WASM artefact present.
 */
export async function validateLocal(xml: string): Promise<ValidationResult> {
  const started = performance.now();
  if (!xml.trim()) {
    return {
      mode: "local",
      rule_pack_version: DEFAULT_RULE_PACK_VERSION,
      backend: "invoicekit-wasm-browser@scaffold",
      findings: [
        {
          rule_id: "ui.input.empty",
          severity: "error",
          message: "Empty input — paste or drop an XML document first.",
        },
      ],
      elapsed_ms: performance.now() - started,
    };
  }
  return {
    mode: "local",
    rule_pack_version: DEFAULT_RULE_PACK_VERSION,
    backend: "invoicekit-wasm-browser@scaffold",
    findings: [
      {
        rule_id: "ui.scaffold.wasm-pending",
        severity: "warning",
        message:
          "Local validator scaffold — wire bindings/wasm-browser to ship full EN 16931 rule coverage.",
      },
    ],
    elapsed_ms: performance.now() - started,
  };
}

/**
 * Validate `xml` against the JVM validator sidecar. Defaults
 * the deadline to 30 seconds; surfaces transport failures as
 * a single `ui.transport.error` finding so the UI doesn't
 * crash on a sidecar outage.
 */
export async function validateReference(
  xml: string,
  config: ReferenceClientConfig
): Promise<ValidationResult> {
  const started = performance.now();
  const url = `${config.baseUrl.replace(/\/$/, "")}/validate`;
  const headers: Record<string, string> = {
    "Content-Type": "application/xml",
    Accept: "application/json",
  };
  if (config.bearerToken) {
    headers["Authorization"] = `Bearer ${config.bearerToken}`;
  }
  const controller = new AbortController();
  const timeout = setTimeout(
    () => controller.abort(),
    config.timeoutMs ?? 30_000
  );
  try {
    const response = await fetch(url, {
      method: "POST",
      headers,
      body: xml,
      signal: controller.signal,
    });
    clearTimeout(timeout);
    if (!response.ok) {
      return {
        mode: "reference",
        rule_pack_version: DEFAULT_RULE_PACK_VERSION,
        backend: `kosit-via-${config.baseUrl}`,
        findings: [
          {
            rule_id: "ui.transport.http",
            severity: "error",
            message: `Reference validator returned ${response.status}.`,
          },
        ],
        elapsed_ms: performance.now() - started,
      };
    }
    const body = (await response.json()) as Partial<ValidationResult>;
    return {
      mode: "reference",
      rule_pack_version: body.rule_pack_version ?? DEFAULT_RULE_PACK_VERSION,
      backend: body.backend ?? `kosit-via-${config.baseUrl}`,
      findings: body.findings ?? [],
      elapsed_ms: performance.now() - started,
    };
  } catch (err) {
    clearTimeout(timeout);
    return {
      mode: "reference",
      rule_pack_version: DEFAULT_RULE_PACK_VERSION,
      backend: `kosit-via-${config.baseUrl}`,
      findings: [
        {
          rule_id: "ui.transport.error",
          severity: "error",
          message:
            err instanceof Error
              ? `Reference validator unreachable: ${err.message}`
              : "Reference validator unreachable.",
        },
      ],
      elapsed_ms: performance.now() - started,
    };
  }
}
