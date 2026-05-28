// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// !!! GENERATED FILE — DO NOT EDIT BY HAND !!!
//
// Re-generate with `bun run generate` from
// bindings/typescript-types/. Source of truth: schemas/.
//
/**
 * Decision emitted for one rule evaluation.
 *
 * This interface was referenced by `ValidationExplainPlan`'s JSON-Schema
 * via the `definition` "RuleEvaluationDecision".
 */
export type RuleEvaluationDecision = ("pass" | "info" | "warning" | "fail")

/**
 * Complete ordered explanation of a validator run.
 *
 *  The plan is intentionally backend-neutral: a pure Rust validator, JVM
 *  sidecar, or partner validator can all emit the same ordered rule steps.
 *  Each step records where the rule evaluated, the machine-readable inputs the
 *  backend considered safe to expose, its decision, and citations for audit.
 */
export interface ValidationExplainPlan {
/**
 * Schema version for the explain-plan wire contract.
 */
schema_version: string
/**
 * Backend identifier, e.g. `rust-native`, `jvm:kosit`, or `partner`.
 */
backend: string
/**
 * Deterministic trace identifier assigned by the caller.
 */
trace_id: string
/**
 * Ordered rule-evaluation steps.
 */
steps: RuleEvaluationStep[]
}
/**
 * One rule evaluation inside a [`ValidationExplainPlan`].
 *
 * This interface was referenced by `ValidationExplainPlan`'s JSON-Schema
 * via the `definition` "RuleEvaluationStep".
 */
export interface RuleEvaluationStep {
/**
 * Identifier of the evaluated rule.
 */
rule_id: string
/**
 * Document path at which the rule evaluated.
 */
evaluated_at_path: string
/**
 * Machine-readable inputs the backend considered safe to expose.
 */
inputs: {
[k: string]: unknown
}
/**
 * Decision produced by the rule.
 */
decision: ("pass" | "info" | "warning" | "fail")
/**
 * Citations that justify the rule.
 */
citations: ExplainPlanCitation[]
}
/**
 * Citation form embedded in explain-plan steps.
 *
 *  This intentionally has a distinct type name from [`Citation`] so generated
 *  TypeScript bindings can re-export validation-result and explain-plan types
 *  from one flat package without duplicate `Citation` symbols.
 *
 * This interface was referenced by `ValidationExplainPlan`'s JSON-Schema
 * via the `definition` "ExplainPlanCitation".
 */
export interface ExplainPlanCitation {
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
