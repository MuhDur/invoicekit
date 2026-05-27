// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

"use client";

import { useState } from "react";

type FixtureName = "basic" | "with-allowance" | "reverse-charge";

const FIXTURES: { id: FixtureName; label: string }[] = [
  { id: "basic", label: "German XRechnung — basic invoice" },
  { id: "with-allowance", label: "German XRechnung — with a 10% allowance" },
  { id: "reverse-charge", label: "German XRechnung — reverse charge (BR-AE)" },
];

export default function Home() {
  const [selected, setSelected] = useState<FixtureName>("basic");
  const [result, setResult] = useState<string>("");
  const [error, setError] = useState<string>("");
  const [busy, setBusy] = useState(false);

  async function issue() {
    setBusy(true);
    setError("");
    setResult("");
    try {
      const resp = await fetch(`/api/issue?fixture=${selected}`, { method: "POST" });
      const text = await resp.text();
      if (!resp.ok) {
        setError(`HTTP ${resp.status}: ${text}`);
      } else {
        setResult(text);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <main style={{ maxWidth: 900 }}>
      <h1>InvoiceKit · Next.js demo</h1>
      <p>
        Pick a fixture, click <strong>Issue</strong>, and the server-side
        route uses <code>@invoicekit/core</code> to construct a validated{" "}
        <code>CommercialDocument</code> JSON. The same shape feeds the wasm
        engine + XML serializer in production.
      </p>

      <fieldset style={{ marginTop: "1.5rem" }}>
        <legend>Fixture</legend>
        {FIXTURES.map((f) => (
          <label key={f.id} style={{ display: "block", margin: "0.4rem 0" }}>
            <input
              type="radio"
              name="fixture"
              value={f.id}
              checked={selected === f.id}
              onChange={() => setSelected(f.id)}
            />{" "}
            {f.label}
          </label>
        ))}
      </fieldset>

      <button
        onClick={issue}
        disabled={busy}
        style={{
          marginTop: "1.5rem",
          padding: "0.6rem 1.2rem",
          fontSize: "1rem",
          cursor: busy ? "wait" : "pointer",
        }}
      >
        {busy ? "Issuing..." : "Issue"}
      </button>

      {error && (
        <pre
          style={{
            background: "#fee",
            color: "#900",
            padding: "1rem",
            marginTop: "1rem",
            whiteSpace: "pre-wrap",
          }}
        >
          {error}
        </pre>
      )}

      {result && (
        <pre
          style={{
            background: "#f4f4f4",
            padding: "1rem",
            marginTop: "1rem",
            overflow: "auto",
            maxHeight: "60vh",
          }}
        >
          {result}
        </pre>
      )}
    </main>
  );
}
