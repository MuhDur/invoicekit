// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//
// !!! GENERATED FILE — DO NOT EDIT BY HAND !!!
//
// Re-generate with `bun run generate` from
// bindings/typescript-types/. Source of truth: schemas/.
//
/**
 * This interface was referenced by `InvoiceKitCapabilityMatrix`'s JSON-Schema
 * via the `definition` "CapabilityLevel".
 */
export type CapabilityLevel = ("available" | "requires_external_backend" | "unavailable_in_wasm")

/**
 * Per-route, per-scenario, per-date capability advertisements for sending compliant electronic invoices. Each entry describes which profiles/transports are accepted, sourced from named jurisdictional manifests, with an explicit confidence rating and a validity window. Consumers must honor `stale_after_days` and downgrade results when a query date falls inside a stale window.
 */
export interface InvoiceKitCapabilityMatrix {
/**
 * Frozen schema version of this manifest. Bumped only with a real migration.
 */
schema_version: "1.0"
/**
 * Timestamp when this matrix was assembled from its sources.
 */
generated_at: string
/**
 * Number of days after `source.fetched_at` an entry is considered stale. Stale entries are still returned but flagged.
 */
stale_after_days: number
entries: CapabilityEntry[]
}
/**
 * This interface was referenced by `InvoiceKitCapabilityMatrix`'s JSON-Schema
 * via the `definition` "CapabilityEntry".
 */
export interface CapabilityEntry {
/**
 * ISO 3166-1 alpha-2 of the sender's country.
 */
route_from: string
/**
 * ISO 3166-1 alpha-2 of the recipient's country.
 */
route_to: string
/**
 * Commercial scenario.
 */
scenario: ("B2B" | "B2C" | "B2G")
valid_from: string
valid_until?: (string | null)
/**
 * @minItems 1
 */
profiles: AcceptedProfile[]
source: SourceProvenance
}
/**
 * This interface was referenced by `InvoiceKitCapabilityMatrix`'s JSON-Schema
 * via the `definition` "AcceptedProfile".
 */
export interface AcceptedProfile {
/**
 * Stable identifier of the accepted profile (e.g. 'xrechnung-3.0', 'peppol-bis-3.0', 'factur-x-1.0.06').
 */
id: string
format: ("UBL" | "CII" | "Factur-X" | "XRechnung" | "Peppol BIS" | "Peppol PINT" | "FatturaPA" | "Chorus Pro" | "CFDI" | "NF-e" | "KSeF")
transport: ("peppol" | "email" | "portal" | "as4-direct" | "manual")
capabilities: ProfileRuntimeCapabilities
}
/**
 * This interface was referenced by `InvoiceKitCapabilityMatrix`'s JSON-Schema
 * via the `definition` "ProfileRuntimeCapabilities".
 */
export interface ProfileRuntimeCapabilities {
serialize: CapabilityLevel
local_validate: CapabilityLevel
reference_validate: CapabilityLevel
/**
 * Service backends required for reference validation, for example 'jvm:kosit'.
 */
requires_service: string[]
/**
 * CLI backends required for reference validation, for example 'verapdf'.
 */
requires_cli: string[]
/**
 * Operations that browser/edge WebAssembly callers cannot run in-process. This list must match every non-'available' operation level on the same profile.
 */
unavailable_in_wasm: ("serialize" | "local_validate" | "reference_validate")[]
}
/**
 * This interface was referenced by `InvoiceKitCapabilityMatrix`'s JSON-Schema
 * via the `definition` "SourceProvenance".
 */
export interface SourceProvenance {
name: string
url?: string
fetched_at: string
/**
 * Confidence in the source. 'authoritative' = official regulator publication; 'low' = community-maintained or inferred.
 */
confidence: ("authoritative" | "high" | "medium" | "low")
}
