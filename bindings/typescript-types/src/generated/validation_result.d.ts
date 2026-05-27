// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// !!! GENERATED FILE — DO NOT EDIT BY HAND !!!
//
// Re-generate with `pnpm --filter @invoicekit/types run generate`
// from the InvoiceKit workspace root. Source of truth: schemas/.
//
/**
 * Severity assigned by the rule pack.
 * 
 * This interface was referenced by `ValidationResult`'s JSON-Schema
 * via the `definition` "Severity".
 */
export type Severity = ("fatal" | "error" | "warning" | "info")
/**
 * EN 16931 business term (BT) or business group (BG) reference.
 * 
 * This interface was referenced by `ValidationResult`'s JSON-Schema
 * via the `definition` "BusinessTerm".
 */
export type BusinessTerm = ({
/**
 * Term code, e.g. `BT-1`.
 */
code: string
kind: "business_term"
} | {
/**
 * Group code, e.g. `BG-25`.
 */
code: string
kind: "business_group"
})
/**
 * Source-document pointer.
 * 
 *  XML-backed documents use XPath; JSON-backed documents use RFC 6901 JSON
 *  Pointer. The two forms are distinct types so a backend cannot mix them
 *  up by accident.
 * 
 * This interface was referenced by `ValidationResult`'s JSON-Schema
 * via the `definition` "Location".
 */
export type Location = ({
/**
 * Pointer body, beginning with `/`.
 */
pointer: string
kind: "json_pointer"
} | {
/**
 * XPath expression as written by the backend.
 */
expression: string
kind: "x_path"
})

/**
 * One validation finding emitted by a backend.
 */
export interface ValidationResult {
/**
 * Identifier of the rule that produced this finding.
 */
rule_id: string
/**
 * Severity that the rule pack assigned to the finding.
 */
severity: ("fatal" | "error" | "warning" | "info")
/**
 * EN 16931 business term or business group implicated.
 */
term: ({
/**
 * Term code, e.g. `BT-1`.
 */
code: string
kind: "business_term"
} | {
/**
 * Group code, e.g. `BG-25`.
 */
code: string
kind: "business_group"
})
/**
 * Pointer into the source document at which the finding applies.
 */
location: ({
/**
 * Pointer body, beginning with `/`.
 */
pointer: string
kind: "json_pointer"
} | {
/**
 * XPath expression as written by the backend.
 */
expression: string
kind: "x_path"
})
/**
 * Optional remediation hint shown to humans and consumed by autofixers.
 */
suggested_fix?: (SuggestedFix | null)
citation: Citation
/**
 * Optional per-result trace context, owned by the T-032a trace extension.
 */
trace?: (ValidationTrace | null)
}
/**
 * Concrete remediation hint.
 * 
 *  Backends populate this when they can deterministically infer a fix
 *  (e.g. set `currency` to `EUR`, drop the duplicate allowance line); UIs
 *  can then offer the user a one-click apply.
 * 
 * This interface was referenced by `ValidationResult`'s JSON-Schema
 * via the `definition` "SuggestedFix".
 */
export interface SuggestedFix {
/**
 * Short human-readable summary.
 */
summary: string
/**
 * Optional patch body — JSON Patch (RFC 6902) for JSON Pointer
 *  locations, XSLT for XPath locations. Format is owned by the
 *  consuming UI; this crate only stores the bytes.
 */
patch?: (string | null)
}
/**
 * Citation back to the authoritative source for this rule.
 */
export interface Citation {
/**
 * Source document (e.g. `EN 16931`, `Peppol BIS 3.0`, `XRechnung 3.0`).
 */
source: string
/**
 * Section identifier inside the source (e.g. `BR-01`, `§5.2`).
 */
section: string
/**
 * Optional URL that resolves to the cited section online.
 */
url?: (string | null)
}
/**
 * Optional trace context owned by the T-032a extension.
 * 
 *  Carries the backend identifier (`rust-native`, `jvm:kosit`, …), the
 *  trace identifier the backend assigned, and any backend-specific
 *  debug fields. The schema deliberately leaves the inner shape opaque so
 *  new backends can ship without modifying this crate.
 * 
 * This interface was referenced by `ValidationResult`'s JSON-Schema
 * via the `definition` "ValidationTrace".
 */
export interface ValidationTrace {
/**
 * Backend identifier, e.g. `rust-native`, `jvm:kosit`, `rest:official`,
 *  `partner`, `cli:invoicekit`, `none`.
 */
backend: string
/**
 * Trace identifier emitted by the backend; correlated by the
 *  `invoicekit-reconcile` outbox.
 */
trace_id: string
/**
 * Backend-specific debug payload. Schema is opaque to this crate;
 *  the trace consumer (typically the support bundle redactor) owns
 *  any further structure.
 */
details?: {
[k: string]: unknown
}
}
/**
 * Authoritative citation for the rule.
 * 
 * This interface was referenced by `ValidationResult`'s JSON-Schema
 * via the `definition` "Citation".
 */
export interface Citation1 {
/**
 * Source document (e.g. `EN 16931`, `Peppol BIS 3.0`, `XRechnung 3.0`).
 */
source: string
/**
 * Section identifier inside the source (e.g. `BR-01`, `§5.2`).
 */
section: string
/**
 * Optional URL that resolves to the cited section online.
 */
url?: (string | null)
}
