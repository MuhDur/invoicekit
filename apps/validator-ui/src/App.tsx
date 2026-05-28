// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

import React, { useEffect, useMemo, useState } from "react";

import { AnalyticsSink } from "./analytics";
import {
  type ValidationResult,
  type ValidatorMode,
  validateLocal,
  validateReference,
} from "./validator";

const REFERENCE_BASE_URL =
  // Wire-time configurable via Vite's env injection.
  (import.meta as unknown as { env?: Record<string, string> }).env
    ?.VITE_REFERENCE_VALIDATOR_URL ?? "https://reference.validate.invoicekit.org";

const ANALYTICS_ENDPOINT = (import.meta as unknown as { env?: Record<string, string> })
  .env?.VITE_ANALYTICS_ENDPOINT;

const analytics = new AnalyticsSink({
  endpoint: ANALYTICS_ENDPOINT,
  disabled: !ANALYTICS_ENDPOINT,
});

export function App(): JSX.Element {
  const [mode, setMode] = useState<ValidatorMode>("local");
  const [xml, setXml] = useState<string>("");
  const [result, setResult] = useState<ValidationResult | null>(null);
  const [running, setRunning] = useState<boolean>(false);

  useEffect(() => {
    analytics.emit({ kind: "page_view" });
  }, []);

  const onValidate = async (): Promise<void> => {
    setRunning(true);
    analytics.emit({ kind: "validation_started", mode });
    try {
      const r =
        mode === "local"
          ? await validateLocal(xml)
          : await validateReference(xml, { baseUrl: REFERENCE_BASE_URL });
      setResult(r);
      analytics.emit({
        kind: "validation_completed",
        mode,
        finding_count: r.findings.length,
      });
    } finally {
      setRunning(false);
    }
  };

  return (
    <main style={containerStyle}>
      <header>
        <h1>InvoiceKit validator</h1>
        <p style={subtitleStyle}>
          Paste a UBL or CII invoice; pick a mode; check it against EN 16931
          and Peppol BIS.
        </p>
      </header>

      <section>
        <ModeSwitch mode={mode} onChange={setMode} />
        <textarea
          aria-label="XML input"
          value={xml}
          onChange={(e) => setXml(e.target.value)}
          placeholder="<Invoice xmlns=...>..."
          style={textareaStyle}
          spellCheck={false}
        />
        <button
          type="button"
          onClick={onValidate}
          disabled={running}
          style={buttonStyle(running)}
        >
          {running ? "Validating…" : "Validate"}
        </button>
      </section>

      <ResultPanel result={result} />

      <footer style={footerStyle}>
        InvoiceKit is open source under Apache 2.0. Local mode runs entirely
        in your browser; reference mode posts to a JVM sidecar with no
        retention by default. See the runbook at{" "}
        <code>docs/operators/VALIDATOR-UI.md</code> for hosting.
      </footer>
    </main>
  );
}

interface ModeSwitchProps {
  mode: ValidatorMode;
  onChange: (m: ValidatorMode) => void;
}

function ModeSwitch({ mode, onChange }: ModeSwitchProps): JSX.Element {
  return (
    <fieldset style={modeSwitchStyle}>
      <legend style={modeLegendStyle}>Validator mode</legend>
      <label style={radioStyle}>
        <input
          type="radio"
          name="mode"
          value="local"
          checked={mode === "local"}
          onChange={() => onChange("local")}
        />
        <span>
          <strong>Local</strong> — browser-only (WASM). Nothing leaves the device.
        </span>
      </label>
      <label style={radioStyle}>
        <input
          type="radio"
          name="mode"
          value="reference"
          checked={mode === "reference"}
          onChange={() => onChange("reference")}
        />
        <span>
          <strong>Reference</strong> — JVM sidecar (official-parity). Posted
          XML is not retained by default.
        </span>
      </label>
    </fieldset>
  );
}

function ResultPanel({ result }: { result: ValidationResult | null }): JSX.Element | null {
  if (!result) return null;
  const grouped = useMemo(() => {
    const out = new Map<string, number>();
    for (const f of result.findings) {
      out.set(f.severity, (out.get(f.severity) ?? 0) + 1);
    }
    return out;
  }, [result]);
  return (
    <section style={resultStyle}>
      <header style={resultHeaderStyle}>
        <div>
          <strong>Mode:</strong> {result.mode}
        </div>
        <div>
          <strong>Rule pack:</strong> {result.rule_pack_version}
        </div>
        <div>
          <strong>Backend:</strong> <code>{result.backend}</code>
        </div>
        <div>
          <strong>Elapsed:</strong> {result.elapsed_ms.toFixed(1)} ms
        </div>
      </header>
      <ul style={tallyStyle}>
        {[...grouped.entries()].map(([sev, count]) => (
          <li key={sev}>
            <strong>{sev}:</strong> {count}
          </li>
        ))}
      </ul>
      <table style={findingTableStyle}>
        <thead>
          <tr>
            <th>Severity</th>
            <th>Rule</th>
            <th>Message</th>
          </tr>
        </thead>
        <tbody>
          {result.findings.map((f, i) => (
            <tr key={`${f.rule_id}-${i}`}>
              <td>{f.severity}</td>
              <td>
                <code>{f.rule_id}</code>
              </td>
              <td>{f.message}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

const containerStyle: React.CSSProperties = {
  fontFamily: "system-ui, sans-serif",
  margin: "2rem auto",
  maxWidth: "960px",
  padding: "0 1rem",
  color: "#111",
};

const subtitleStyle: React.CSSProperties = { color: "#555" };
const modeSwitchStyle: React.CSSProperties = {
  border: "1px solid #ddd",
  borderRadius: "6px",
  padding: "0.75rem 1rem",
  margin: "1rem 0",
};
const modeLegendStyle: React.CSSProperties = {
  padding: "0 0.5rem",
  color: "#444",
  fontWeight: 600,
};
const radioStyle: React.CSSProperties = {
  display: "flex",
  gap: "0.5rem",
  alignItems: "start",
  margin: "0.5rem 0",
};
const textareaStyle: React.CSSProperties = {
  display: "block",
  width: "100%",
  minHeight: "200px",
  fontFamily: "ui-monospace, SF Mono, Menlo, monospace",
  fontSize: "13px",
  padding: "0.5rem",
  border: "1px solid #ccc",
  borderRadius: "4px",
};
const buttonStyle = (running: boolean): React.CSSProperties => ({
  marginTop: "0.75rem",
  padding: "0.5rem 1.25rem",
  background: running ? "#888" : "#2a4d9b",
  color: "white",
  border: "none",
  borderRadius: "4px",
  cursor: running ? "wait" : "pointer",
  fontWeight: 600,
});
const resultStyle: React.CSSProperties = {
  marginTop: "1.5rem",
  border: "1px solid #ddd",
  borderRadius: "6px",
  padding: "1rem",
  background: "#fafafa",
};
const resultHeaderStyle: React.CSSProperties = {
  display: "grid",
  gridTemplateColumns: "repeat(auto-fit, minmax(180px, 1fr))",
  gap: "0.5rem",
  marginBottom: "0.75rem",
};
const tallyStyle: React.CSSProperties = {
  display: "flex",
  gap: "1rem",
  listStyle: "none",
  padding: 0,
  margin: "0 0 0.75rem 0",
};
const findingTableStyle: React.CSSProperties = {
  width: "100%",
  borderCollapse: "collapse",
  fontSize: "13px",
};
const footerStyle: React.CSSProperties = {
  color: "#888",
  fontSize: "0.85rem",
  marginTop: "2rem",
};
