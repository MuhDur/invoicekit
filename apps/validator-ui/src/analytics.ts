// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

/// PII-free analytics for validate.invoicekit.org.
///
/// We track three events:
///
/// * `page_view` — fired once on load
/// * `validation_started` — fired when the user clicks Validate
/// * `validation_completed` — fired on a successful result
///
/// All events carry only the mode (`local` / `reference`) and a
/// timestamp. No payload bytes, no IP, no user agent beyond what
/// the analytics endpoint already sees in the request envelope.

export type AnalyticsEvent =
  | { kind: "page_view" }
  | { kind: "validation_started"; mode: "local" | "reference" }
  | {
      kind: "validation_completed";
      mode: "local" | "reference";
      finding_count: number;
    };

export interface AnalyticsConfig {
  /** Endpoint that accepts POSTed JSON events. */
  endpoint?: string;
  /** Disable network sends entirely (default in dev). */
  disabled?: boolean;
}

/// Singleton sink used by the App; tests inject their own.
export class AnalyticsSink {
  constructor(private readonly config: AnalyticsConfig = {}) {}

  emit(event: AnalyticsEvent): void {
    if (this.config.disabled || !this.config.endpoint) {
      return;
    }
    const payload = JSON.stringify({
      ts: new Date().toISOString(),
      ...event,
    });
    if ("sendBeacon" in navigator) {
      const blob = new Blob([payload], { type: "application/json" });
      navigator.sendBeacon(this.config.endpoint, blob);
      return;
    }
    void fetch(this.config.endpoint, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: payload,
      keepalive: true,
    });
  }
}
