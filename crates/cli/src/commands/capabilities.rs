// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! `invoicekit capabilities` — resolve which e-invoicing profiles and
//! transports a given route/scenario/date accepts.
//!
//! The matrix is bundled at compile time from
//! `crates/cli/data/capabilities/matrix.json` (validated against
//! `schemas/invoicekit-capabilities-v1.json` by a CI gate). Each entry
//! advertises a sender country, recipient country, commercial scenario,
//! validity window, the accepted profiles, and the source provenance
//! (name + fetched-at timestamp + confidence) the entry was derived
//! from.
//!
//! Resolution is intentionally rigid:
//!
//! 1. Filter the matrix on `route_from`, `route_to`, `scenario`, and
//!    `valid_from <= query_date <= valid_until` (open-ended if
//!    `valid_until` is null).
//! 2. If anything matches, return it; the freshness of each match is
//!    derived from `today - source.fetched_at` against the manifest's
//!    `stale_after_days`. A stale match returns the entry **and** a
//!    `warnings[]` entry advertising the staleness so the caller can
//!    decide whether to act on it.
//! 3. If nothing matches, try **auto-downgrade**: relax the scenario
//!    (B2B -> B2G falls back to B2G if only B2G matches the route+date)
//!    and re-run filter (1). When a downgrade match exists, the
//!    response sets `status: "downgraded"` with a warning naming the
//!    scenario we fell back to. The caller can still act on it but
//!    should not assume regulator-grade certainty.
//! 4. If even the downgrade returns nothing, the response is
//!    `status: "no_data"` with `matched: []` and a warning.
//!
//! The result envelope is stable JSON, defined by
//! [`ResolutionEnvelope`]. A side-by-side pretty printer is available
//! via `--format=pretty` for terminal use.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use serde::{Deserialize, Serialize};

const BUNDLED_MATRIX: &str = include_str!("../../data/capabilities/matrix.json");

/// Bead tag of the implementing initiative.
pub const CAPABILITIES_BEAD_ID: &str = "invoices-t-006a-capabilities-spec-b1g";

/// Frozen schema version the bundled matrix advertises. Bumped only
/// alongside a real migration in `invoicekit-migration`.
pub const SUPPORTED_MATRIX_SCHEMA_VERSION: &str = "1.0";

/// Country code in ISO 3166-1 alpha-2 form.
pub type Country = String;

/// Calendar date in `YYYY-MM-DD`. Validated at parse time.
pub type IsoDate = String;

/// Commercial scenario the query covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Scenario {
    /// Business-to-business commercial invoices.
    B2B,
    /// Business-to-consumer commercial invoices.
    B2C,
    /// Business-to-government invoices.
    B2G,
}

impl Scenario {
    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "B2B" => Some(Self::B2B),
            "B2C" => Some(Self::B2C),
            "B2G" => Some(Self::B2G),
            _ => None,
        }
    }

    /// Auto-downgrade chain. B2B falls back to B2G (most regulators
    /// publish B2G profiles even before B2B mandates kick in). B2C has
    /// no fallback because consumer-facing flows are intentionally
    /// distinct. B2G never downgrades — it is already the most
    /// established lane and downgrading would be misleading.
    fn fallbacks(self) -> &'static [Self] {
        match self {
            Self::B2B => &[Self::B2G],
            Self::B2C | Self::B2G => &[],
        }
    }
}

/// Top-level capability matrix file format.
///
/// Mirrors `schemas/invoicekit-capabilities-v1.json`.
/// Top-level capability matrix file format.
///
/// Mirrors `schemas/invoicekit-capabilities-v1.json`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CapabilityMatrix {
    /// Frozen schema version of this manifest. Matched against [`SUPPORTED_MATRIX_SCHEMA_VERSION`].
    pub schema_version: String,
    /// Wall-clock timestamp at which this matrix was assembled.
    pub generated_at: String,
    /// Number of days after `source.fetched_at` an entry is considered stale.
    pub stale_after_days: u32,
    /// Individual route/scenario rows.
    pub entries: Vec<CapabilityEntry>,
}

/// One row of the capability matrix.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CapabilityEntry {
    /// Sender country (ISO 3166-1 alpha-2).
    pub route_from: Country,
    /// Recipient country (ISO 3166-1 alpha-2).
    pub route_to: Country,
    /// Commercial scenario the row covers.
    pub scenario: Scenario,
    /// First date the row is in force.
    pub valid_from: IsoDate,
    /// Last date the row is in force; `None` means open-ended.
    pub valid_until: Option<IsoDate>,
    /// Profiles/transports the row advertises.
    pub profiles: Vec<AcceptedProfile>,
    /// Where the row was derived from.
    pub source: SourceProvenance,
}

/// One accepted profile (format + transport pair).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AcceptedProfile {
    /// Stable profile identifier (e.g. `xrechnung-3.0`).
    pub id: String,
    /// Format family the profile belongs to.
    pub format: String,
    /// Delivery channel (`peppol`, `email`, `portal`, `as4-direct`, `manual`).
    pub transport: String,
}

/// Provenance of a capability row.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourceProvenance {
    /// Human-readable source name (e.g. `KoSIT XRechnung specification`).
    pub name: String,
    /// Canonical URL the row was derived from, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Wall-clock timestamp when the source was last consulted.
    pub fetched_at: String,
    /// Confidence in the source: `authoritative` | `high` | `medium` | `low`.
    pub confidence: String,
}

/// What the user asked.
#[derive(Debug, Clone, Serialize)]
pub struct Query {
    /// Sender country (ISO 3166-1 alpha-2).
    pub from: Country,
    /// Recipient country (ISO 3166-1 alpha-2).
    pub to: Country,
    /// Commercial scenario.
    pub scenario: Scenario,
    /// Query date in `YYYY-MM-DD` form.
    pub date: IsoDate,
}

/// High-level outcome of a [`resolve`] call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Exact match on route/scenario, source data is fresh.
    Ok,
    /// Exact match on route/scenario, but source data is older than the
    /// matrix's `stale_after_days`. Caller should refresh.
    Stale,
    /// No exact match; result comes from auto-downgrade (e.g. fell back
    /// to a B2G entry for a B2B query).
    Downgraded,
    /// Nothing matched, not even after downgrade.
    NoData,
}

/// A single resolved capability entry plus its freshness annotation.
#[derive(Debug, Clone, Serialize)]
pub struct MatchedEntry {
    /// Underlying capability row.
    #[serde(flatten)]
    pub entry: CapabilityEntry,
    /// Whether the underlying source is still within the staleness window.
    pub freshness: Freshness,
    /// How many days past the staleness threshold the source data is.
    /// Zero or negative when fresh.
    pub stale_for_days: i64,
}

/// Per-match freshness classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    /// Source was fetched within the manifest's staleness window.
    Fresh,
    /// Source was fetched outside the manifest's staleness window.
    Stale,
}

/// Stable JSON output envelope returned by [`resolve`] and serialized by
/// the CLI's `--format=json` output.
#[derive(Debug, Clone, Serialize)]
pub struct ResolutionEnvelope {
    /// Bead identifier carried for diagnostic correlation.
    pub bead: &'static str,
    /// Schema version of the matrix that was consulted.
    pub matrix_schema_version: String,
    /// Generation timestamp of the matrix that was consulted.
    pub matrix_generated_at: String,
    /// Echo of the input query.
    pub query: Query,
    /// High-level outcome.
    pub status: Status,
    /// Matched rows with freshness annotations.
    pub matched: Vec<MatchedEntry>,
    /// Human-readable warnings (staleness, downgrade notes, etc.).
    pub warnings: Vec<String>,
}

/// Errors surfaced by [`resolve`] and the CLI parser.
#[derive(Debug)]
pub enum CapabilityError {
    /// A required CLI flag was not supplied.
    MissingFlag(&'static str),
    /// An unknown or malformed CLI flag was supplied.
    UnknownFlag(String),
    /// Country code did not match ISO 3166-1 alpha-2.
    BadCountry(String),
    /// Date did not parse as `YYYY-MM-DD`.
    BadDate(String),
    /// Scenario did not match `B2B|B2C|B2G`.
    BadScenario(String),
    /// Matrix JSON failed to parse.
    MatrixParse(String),
    /// Matrix declared a `schema_version` this binary does not understand.
    MatrixSchemaVersionMismatch {
        /// Schema version this binary was built against.
        expected: String,
        /// Schema version the matrix file declared.
        found: String,
    },
    /// Reading the matrix file from disk failed.
    MatrixRead {
        /// Path that was attempted.
        path: PathBuf,
        /// Underlying I/O error.
        source: String,
    },
}

impl std::fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingFlag(name) => write!(f, "missing required flag: --{name}"),
            Self::UnknownFlag(name) => write!(f, "unknown flag: {name}"),
            Self::BadCountry(c) => write!(
                f,
                "invalid country code {c:?} (expected ISO 3166-1 alpha-2, e.g. DE)"
            ),
            Self::BadDate(d) => {
                write!(f, "invalid date {d:?} (expected YYYY-MM-DD)")
            }
            Self::BadScenario(s) => {
                write!(f, "invalid scenario {s:?} (expected B2B|B2C|B2G)")
            }
            Self::MatrixParse(m) => write!(f, "failed to parse capability matrix: {m}"),
            Self::MatrixSchemaVersionMismatch { expected, found } => {
                write!(
                    f,
                    "capability matrix schema_version mismatch: this binary expects {expected}, matrix declares {found}"
                )
            }
            Self::MatrixRead { path, source } => {
                write!(f, "failed to read matrix {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for CapabilityError {}

/// Validate that `s` is an ISO 3166-1 alpha-2 code (two ASCII uppercase
/// letters). Uppercases lowercase input to be forgiving.
fn parse_country(s: &str) -> Result<Country, CapabilityError> {
    let s = s.trim();
    if s.len() != 2 || !s.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(CapabilityError::BadCountry(s.to_string()));
    }
    Ok(s.to_ascii_uppercase())
}

/// Parse `YYYY-MM-DD` without pulling chrono. Only checks lexical shape
/// and field ranges; downstream comparisons are string-based because
/// ISO dates sort lexicographically.
fn parse_iso_date(s: &str) -> Result<IsoDate, CapabilityError> {
    let s = s.trim();
    let bad = || CapabilityError::BadDate(s.to_string());
    if s.len() != 10 {
        return Err(bad());
    }
    let bytes = s.as_bytes();
    if bytes[4] != b'-' || bytes[7] != b'-' {
        return Err(bad());
    }
    let year: u16 = std::str::from_utf8(&bytes[0..4])
        .map_err(|_| bad())?
        .parse()
        .map_err(|_| bad())?;
    let month: u8 = std::str::from_utf8(&bytes[5..7])
        .map_err(|_| bad())?
        .parse()
        .map_err(|_| bad())?;
    let day: u8 = std::str::from_utf8(&bytes[8..10])
        .map_err(|_| bad())?
        .parse()
        .map_err(|_| bad())?;
    if !(1900..=2300).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(bad());
    }
    Ok(s.to_string())
}

/// Parse the matrix from a JSON string and verify its declared schema
/// version matches what this binary understands.
///
/// # Errors
///
/// Returns [`CapabilityError::MatrixParse`] when `raw` is not valid JSON
/// or does not match the [`CapabilityMatrix`] shape, and
/// [`CapabilityError::MatrixSchemaVersionMismatch`] when the parsed
/// `schema_version` differs from [`SUPPORTED_MATRIX_SCHEMA_VERSION`].
pub fn parse_matrix(raw: &str) -> Result<CapabilityMatrix, CapabilityError> {
    let m: CapabilityMatrix =
        serde_json::from_str(raw).map_err(|e| CapabilityError::MatrixParse(e.to_string()))?;
    if m.schema_version != SUPPORTED_MATRIX_SCHEMA_VERSION {
        return Err(CapabilityError::MatrixSchemaVersionMismatch {
            expected: SUPPORTED_MATRIX_SCHEMA_VERSION.to_string(),
            found: m.schema_version,
        });
    }
    Ok(m)
}

/// Load the bundled capability matrix shipped with this binary.
///
/// # Panics
///
/// Panics if the embedded `data/capabilities/matrix.json` fails to
/// parse or declares a `schema_version` other than
/// [`SUPPORTED_MATRIX_SCHEMA_VERSION`]. The CI gate
/// (`tools/release-checks/validate_capabilities_matrix.py`) prevents
/// such a mismatch from ever reaching a release.
pub fn bundled_matrix() -> CapabilityMatrix {
    parse_matrix(BUNDLED_MATRIX)
        .expect("bundled capability matrix must parse and match supported schema version")
}

/// Days between two ISO dates (`YYYY-MM-DD`), `a - b`, treating each
/// month as 30 days and year as 365 — adequate for the staleness window
/// (180 days default), avoids pulling chrono.
fn days_between(a: &str, b: &str) -> Option<i64> {
    let (ay, am, ad) = split_date(a)?;
    let (by, bm, bd) = split_date(b)?;
    let a_days = i64::from(ay) * 365 + i64::from(am) * 30 + i64::from(ad);
    let b_days = i64::from(by) * 365 + i64::from(bm) * 30 + i64::from(bd);
    Some(a_days - b_days)
}

fn split_date(s: &str) -> Option<(u16, u8, u8)> {
    if s.len() < 10 {
        return None;
    }
    let bytes = s.as_bytes();
    let y: u16 = std::str::from_utf8(&bytes[0..4]).ok()?.parse().ok()?;
    let m: u8 = std::str::from_utf8(&bytes[5..7]).ok()?.parse().ok()?;
    let d: u8 = std::str::from_utf8(&bytes[8..10]).ok()?.parse().ok()?;
    Some((y, m, d))
}

/// Pure resolution: applies the query to the matrix using `today` as
/// the freshness reference clock. Separated from any I/O so tests can
/// pin the clock.
pub fn resolve(matrix: &CapabilityMatrix, query: &Query, today: &str) -> ResolutionEnvelope {
    let mut envelope = ResolutionEnvelope {
        bead: CAPABILITIES_BEAD_ID,
        matrix_schema_version: matrix.schema_version.clone(),
        matrix_generated_at: matrix.generated_at.clone(),
        query: query.clone(),
        status: Status::NoData,
        matched: Vec::new(),
        warnings: Vec::new(),
    };

    let exact = filter(matrix, query.scenario, query);
    if !exact.is_empty() {
        let (entries, any_stale) = attach_freshness(
            exact,
            matrix.stale_after_days,
            today,
            &mut envelope.warnings,
        );
        envelope.matched = entries;
        envelope.status = if any_stale { Status::Stale } else { Status::Ok };
        return envelope;
    }

    for fallback in query.scenario.fallbacks() {
        let downgraded = filter(matrix, *fallback, query);
        if !downgraded.is_empty() {
            envelope.warnings.push(format!(
                "no {orig:?} entries matched; auto-downgraded to {fb:?} per fallback policy",
                orig = query.scenario,
                fb = fallback
            ));
            let (entries, _) = attach_freshness(
                downgraded,
                matrix.stale_after_days,
                today,
                &mut envelope.warnings,
            );
            envelope.matched = entries;
            envelope.status = Status::Downgraded;
            return envelope;
        }
    }

    envelope.warnings.push(format!(
        "no capability entries match route {from}->{to}, scenario {sc:?}, date {d}",
        from = query.from,
        to = query.to,
        sc = query.scenario,
        d = query.date,
    ));
    envelope
}

fn filter(matrix: &CapabilityMatrix, scenario: Scenario, query: &Query) -> Vec<CapabilityEntry> {
    matrix
        .entries
        .iter()
        .filter(|e| {
            e.route_from == query.from
                && e.route_to == query.to
                && e.scenario == scenario
                && e.valid_from.as_str() <= query.date.as_str()
                && e.valid_until
                    .as_deref()
                    .is_none_or(|until| query.date.as_str() <= until)
        })
        .cloned()
        .collect()
}

fn attach_freshness(
    raw: Vec<CapabilityEntry>,
    stale_after_days: u32,
    today: &str,
    warnings: &mut Vec<String>,
) -> (Vec<MatchedEntry>, bool) {
    let mut any_stale = false;
    let entries = raw
        .into_iter()
        .map(|e| {
            let fetched = e.source.fetched_at.as_str();
            let age = days_between(today, fetched).unwrap_or(0);
            let stale_for = age - i64::from(stale_after_days);
            let fresh = stale_for <= 0;
            if !fresh {
                any_stale = true;
                warnings.push(format!(
                    "source {src:?} (fetched {fetched}) is stale by {days} day(s); refresh recommended",
                    src = e.source.name,
                    days = stale_for
                ));
            }
            MatchedEntry {
                entry: e,
                freshness: if fresh {
                    Freshness::Fresh
                } else {
                    Freshness::Stale
                },
                stale_for_days: stale_for.max(0),
            }
        })
        .collect();
    (entries, any_stale)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Json,
    Pretty,
}

#[derive(Debug)]
struct CliArgs {
    from: Country,
    to: Country,
    date: IsoDate,
    scenario: Scenario,
    format: OutputFormat,
    matrix_path: Option<PathBuf>,
    today: Option<String>,
}

fn parse_argv(argv: &[String]) -> Result<CliArgs, CapabilityError> {
    let mut from: Option<String> = None;
    let mut to: Option<String> = None;
    let mut date: Option<String> = None;
    let mut scenario: Option<String> = None;
    let mut format = OutputFormat::Json;
    let mut matrix_path: Option<PathBuf> = None;
    let mut today: Option<String> = None;

    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        if let Some(v) = a.strip_prefix("--from=") {
            from = Some(v.to_string());
        } else if a == "--from" {
            i += 1;
            from = Some(
                argv.get(i)
                    .cloned()
                    .ok_or(CapabilityError::MissingFlag("from"))?,
            );
        } else if let Some(v) = a.strip_prefix("--to=") {
            to = Some(v.to_string());
        } else if a == "--to" {
            i += 1;
            to = Some(
                argv.get(i)
                    .cloned()
                    .ok_or(CapabilityError::MissingFlag("to"))?,
            );
        } else if let Some(v) = a.strip_prefix("--date=") {
            date = Some(v.to_string());
        } else if a == "--date" {
            i += 1;
            date = Some(
                argv.get(i)
                    .cloned()
                    .ok_or(CapabilityError::MissingFlag("date"))?,
            );
        } else if let Some(v) = a.strip_prefix("--scenario=") {
            scenario = Some(v.to_string());
        } else if a == "--scenario" {
            i += 1;
            scenario = Some(
                argv.get(i)
                    .cloned()
                    .ok_or(CapabilityError::MissingFlag("scenario"))?,
            );
        } else if let Some(v) = a.strip_prefix("--format=") {
            format = match v {
                "json" => OutputFormat::Json,
                "pretty" => OutputFormat::Pretty,
                _ => return Err(CapabilityError::UnknownFlag(format!("--format={v}"))),
            };
        } else if let Some(v) = a.strip_prefix("--matrix=") {
            matrix_path = Some(PathBuf::from(v));
        } else if let Some(v) = a.strip_prefix("--today=") {
            today = Some(v.to_string());
        } else {
            return Err(CapabilityError::UnknownFlag(a.clone()));
        }
        i += 1;
    }

    Ok(CliArgs {
        from: parse_country(&from.ok_or(CapabilityError::MissingFlag("from"))?)?,
        to: parse_country(&to.ok_or(CapabilityError::MissingFlag("to"))?)?,
        date: parse_iso_date(&date.ok_or(CapabilityError::MissingFlag("date"))?)?,
        scenario: Scenario::parse(&scenario.ok_or(CapabilityError::MissingFlag("scenario"))?)
            .ok_or_else(|| CapabilityError::BadScenario("(empty)".into()))?,
        format,
        matrix_path,
        today,
    })
}

fn usage() -> String {
    "usage: invoicekit capabilities --from=CC --to=CC --date=YYYY-MM-DD --scenario=B2B|B2C|B2G \\\n                                  [--format=json|pretty] [--matrix=PATH] [--today=YYYY-MM-DD]\n\nResolves accepted e-invoice profiles/transports for a sender->receiver\nroute on a given date and commercial scenario, using the bundled\ncapability matrix (or a caller-supplied one via --matrix).\n\nExit codes:\n  0  resolution succeeded (status: ok | stale | downgraded | no_data)\n  2  invalid CLI usage (missing flag, bad country code, bad date)\n  3  matrix load or schema-version error\n"
        .to_string()
}

/// CLI entry point. Returns 0 on a successful resolution regardless of
/// `status` — `no_data` is still a *successful* answer.
///
/// # Panics
///
/// Panics only via the internal `expect` on `serde_json::to_string_pretty`,
/// which would indicate that [`ResolutionEnvelope`] failed to round-trip
/// to JSON — impossible by construction since every field is `Serialize`.
pub fn run(argv: &[String]) -> ExitCode {
    if argv.iter().any(|a| a == "--help" || a == "-h") {
        print!("{}", usage());
        return ExitCode::SUCCESS;
    }
    let parsed = match parse_argv(argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            eprintln!();
            eprint!("{}", usage());
            return ExitCode::from(2);
        }
    };

    let matrix = match &parsed.matrix_path {
        Some(p) => match fs::read_to_string(p) {
            Ok(raw) => match parse_matrix(&raw) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(3);
                }
            },
            Err(e) => {
                eprintln!(
                    "{}",
                    CapabilityError::MatrixRead {
                        path: p.clone(),
                        source: e.to_string(),
                    }
                );
                return ExitCode::from(3);
            }
        },
        None => bundled_matrix(),
    };

    let today = parsed
        .today
        .clone()
        .unwrap_or_else(|| env::var("INVOICEKIT_TODAY").unwrap_or_else(|_| "2026-05-27".into()));
    let query = Query {
        from: parsed.from,
        to: parsed.to,
        scenario: parsed.scenario,
        date: parsed.date,
    };
    let envelope = resolve(&matrix, &query, &today);

    match parsed.format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&envelope)
                .expect("ResolutionEnvelope must serialize to JSON")
        ),
        OutputFormat::Pretty => print!("{}", render_pretty(&envelope)),
    }
    ExitCode::SUCCESS
}

fn render_pretty(env: &ResolutionEnvelope) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "Route   : {} -> {}", env.query.from, env.query.to);
    let _ = writeln!(out, "Date    : {}", env.query.date);
    let _ = writeln!(out, "Scenario: {:?}", env.query.scenario);
    let _ = writeln!(out, "Status  : {:?}", env.status);
    let _ = writeln!(
        out,
        "Matrix  : v{} generated {}",
        env.matrix_schema_version, env.matrix_generated_at
    );
    out.push('\n');
    if env.matched.is_empty() {
        out.push_str("No accepted profiles.\n");
    } else {
        out.push_str("Accepted profiles:\n");
        for m in &env.matched {
            let _ = writeln!(
                out,
                "  - {scenario:?} {from}->{to} (valid {from_d} .. {to_d}, source {src} [{conf}], {fresh:?})",
                scenario = m.entry.scenario,
                from = m.entry.route_from,
                to = m.entry.route_to,
                from_d = m.entry.valid_from,
                to_d = m.entry.valid_until.as_deref().unwrap_or("open"),
                src = m.entry.source.name,
                conf = m.entry.source.confidence,
                fresh = m.freshness
            );
            for p in &m.entry.profiles {
                let _ = writeln!(
                    out,
                    "      * {} ({}, transport={})",
                    p.id, p.format, p.transport
                );
            }
        }
    }
    if !env.warnings.is_empty() {
        out.push_str("\nWarnings:\n");
        for w in &env.warnings {
            let _ = writeln!(out, "  ! {w}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> CapabilityMatrix {
        bundled_matrix()
    }

    #[test]
    fn bundled_matrix_parses_and_matches_schema_version() {
        let m = fixture();
        assert_eq!(m.schema_version, SUPPORTED_MATRIX_SCHEMA_VERSION);
        assert!(!m.entries.is_empty());
    }

    #[test]
    fn schema_version_mismatch_is_rejected() {
        let raw = r#"{"schema_version":"99.0","generated_at":"2026-01-01T00:00:00Z","stale_after_days":180,"entries":[]}"#;
        let err = parse_matrix(raw).expect_err("expected mismatch error");
        assert!(matches!(
            err,
            CapabilityError::MatrixSchemaVersionMismatch { .. }
        ));
    }

    #[test]
    fn exact_match_returns_ok_when_source_is_fresh() {
        let m = fixture();
        let q = Query {
            from: "DE".into(),
            to: "FR".into(),
            scenario: Scenario::B2B,
            date: "2027-01-01".into(),
        };
        let env = resolve(&m, &q, "2026-06-01");
        assert_eq!(env.status, Status::Ok);
        assert_eq!(env.matched.len(), 1);
        assert_eq!(env.matched[0].entry.route_from, "DE");
        assert_eq!(env.matched[0].entry.route_to, "FR");
        assert_eq!(env.matched[0].freshness, Freshness::Fresh);
        assert!(env.warnings.is_empty());
    }

    #[test]
    fn match_outside_validity_window_returns_no_data() {
        let m = fixture();
        let q = Query {
            from: "DE".into(),
            to: "FR".into(),
            scenario: Scenario::B2B,
            // Before 2026-09-01 valid_from.
            date: "2025-01-01".into(),
        };
        let env = resolve(&m, &q, "2026-06-01");
        assert_eq!(env.status, Status::NoData);
        assert!(env.matched.is_empty());
        assert!(env
            .warnings
            .iter()
            .any(|w| w.contains("no capability entries match")));
    }

    #[test]
    fn b2b_query_auto_downgrades_to_b2g_when_only_b2g_exists() {
        let m = fixture();
        // NL only has B2G in the seed matrix.
        let q = Query {
            from: "NL".into(),
            to: "NL".into(),
            scenario: Scenario::B2B,
            date: "2027-01-01".into(),
        };
        let env = resolve(&m, &q, "2026-06-01");
        assert_eq!(env.status, Status::Downgraded);
        assert_eq!(env.matched.len(), 1);
        assert_eq!(env.matched[0].entry.scenario, Scenario::B2G);
        assert!(env
            .warnings
            .iter()
            .any(|w| w.contains("auto-downgraded to B2G")));
    }

    #[test]
    fn b2c_does_not_downgrade() {
        let m = fixture();
        let q = Query {
            from: "NL".into(),
            to: "NL".into(),
            scenario: Scenario::B2C,
            date: "2027-01-01".into(),
        };
        let env = resolve(&m, &q, "2026-06-01");
        assert_eq!(env.status, Status::NoData);
        assert!(env.matched.is_empty());
    }

    #[test]
    fn stale_source_flips_status_and_adds_warning() {
        let m = fixture();
        let q = Query {
            from: "IT".into(),
            to: "IT".into(),
            scenario: Scenario::B2B,
            date: "2027-01-01".into(),
        };
        // IT source fetched 2025-09-15; with 180-day stale window it is
        // stale by 2026-06-01.
        let env = resolve(&m, &q, "2026-06-01");
        assert_eq!(env.status, Status::Stale);
        assert_eq!(env.matched.len(), 1);
        assert_eq!(env.matched[0].freshness, Freshness::Stale);
        assert!(env.matched[0].stale_for_days > 0);
        assert!(env.warnings.iter().any(|w| w.contains("is stale by")));
    }

    #[test]
    fn argv_supports_eq_and_split_forms() {
        let a1 = parse_argv(&[
            "--from=DE".into(),
            "--to=FR".into(),
            "--date=2027-01-01".into(),
            "--scenario=B2B".into(),
        ])
        .unwrap();
        let a2 = parse_argv(&[
            "--from".into(),
            "de".into(),
            "--to".into(),
            "fr".into(),
            "--date".into(),
            "2027-01-01".into(),
            "--scenario".into(),
            "b2b".into(),
        ])
        .unwrap();
        assert_eq!(a1.from, "DE");
        assert_eq!(a2.from, "DE");
        assert_eq!(a1.to, "FR");
        assert_eq!(a2.to, "FR");
        assert_eq!(a1.scenario, Scenario::B2B);
        assert_eq!(a2.scenario, Scenario::B2B);
    }

    #[test]
    fn argv_rejects_bad_country() {
        let err = parse_argv(&[
            "--from=DEU".into(),
            "--to=FR".into(),
            "--date=2027-01-01".into(),
            "--scenario=B2B".into(),
        ])
        .expect_err("must reject ISO 3166-1 alpha-3");
        assert!(matches!(err, CapabilityError::BadCountry(_)));
    }

    #[test]
    fn argv_rejects_bad_date() {
        let err = parse_argv(&[
            "--from=DE".into(),
            "--to=FR".into(),
            "--date=2027/01/01".into(),
            "--scenario=B2B".into(),
        ])
        .expect_err("must reject slashed date");
        assert!(matches!(err, CapabilityError::BadDate(_)));
    }

    #[test]
    fn argv_rejects_bad_scenario() {
        let err = parse_argv(&[
            "--from=DE".into(),
            "--to=FR".into(),
            "--date=2027-01-01".into(),
            "--scenario=B2X".into(),
        ])
        .expect_err("must reject unknown scenario");
        assert!(matches!(err, CapabilityError::BadScenario(_)));
    }

    #[test]
    fn argv_requires_all_four_flags() {
        let err = parse_argv(&["--from=DE".into()]).expect_err("must require all flags");
        assert!(matches!(err, CapabilityError::MissingFlag(_)));
    }

    #[test]
    fn pretty_output_renders_matched_profiles() {
        let m = fixture();
        let q = Query {
            from: "DE".into(),
            to: "DE".into(),
            scenario: Scenario::B2G,
            date: "2027-01-01".into(),
        };
        let env = resolve(&m, &q, "2026-06-01");
        let out = render_pretty(&env);
        assert!(out.contains("Route   : DE -> DE"));
        assert!(out.contains("Status  : Ok"));
        assert!(out.contains("xrechnung-3.0"));
    }

    #[test]
    fn fallback_chain_b2b_includes_b2g() {
        assert_eq!(Scenario::B2B.fallbacks(), &[Scenario::B2G]);
        assert_eq!(Scenario::B2C.fallbacks(), &[] as &[Scenario]);
        assert_eq!(Scenario::B2G.fallbacks(), &[] as &[Scenario]);
    }

    #[test]
    fn days_between_handles_known_intervals() {
        // 30-day month approximation; check it's directionally correct.
        assert!(days_between("2026-06-01", "2025-09-15").unwrap() > 180);
        assert!(days_between("2026-06-01", "2026-05-01").unwrap() > 0);
        assert!(days_between("2026-05-01", "2026-06-01").unwrap() < 0);
    }
}
