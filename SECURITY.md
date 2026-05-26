# Security Policy

InvoiceKit handles legally-binding business documents, signed evidence
bundles, and credentials for national e-invoicing gateways. We take security
reports seriously.

## Supported versions

The project is pre-1.0. Until the first `1.0.0` release:

- Only the `main` branch and the most recent tagged release receive security
  fixes.
- Earlier tagged releases will not receive backports.

After `1.0.0` we will publish an explicit support matrix in this file.

## Reporting a vulnerability

**Do not file a public GitHub issue for a security vulnerability.**

Report privately through one of the following channels, in order of
preference:

1. **GitHub Security Advisories.** Open a private advisory in this
   repository at `Security` → `Report a vulnerability`
   (<https://github.com/MuhDur/invoicekit/security/advisories/new>).
   This is the preferred channel; it gives us a private fork to
   coordinate the fix and a CVE assignment path.
2. **Email.** **durakovic.mu@gmail.com** with `[InvoiceKit security]`
   in the subject. Use this if you cannot open a GitHub advisory.

Please include:

- A description of the issue and the impact you believe it has.
- A minimal reproducer (commit hash + commands), if you have one.
- The affected versions / commit ranges.
- Any suggested mitigations or patches.

## What to expect

- **Acknowledgement** within 5 business days.
- **Initial assessment** (severity, scope, affected versions) within 10
  business days.
- **Fix and coordinated disclosure** on a timeline proportionate to severity:
  - Critical / actively exploited: target ≤ 14 days from confirmation.
  - High: target ≤ 30 days.
  - Medium / low: included in the next scheduled release.

We will keep you informed at each step and credit you in the advisory
(opt-out available).

## Scope

In scope:

- The Rust crates in this repository.
- The JVM validator sidecars in `services/`.
- The published bindings (`bindings/`) and the REST shim.
- The deployment manifests in `deploy/` (default configurations).

Out of scope:

- Third-party national clearance gateways. Report those to the gateway
  operator directly.
- Misconfiguration of a downstream deployment that is not represented in
  `deploy/`.
- Social engineering of project maintainers.

## Hardening commitments

- Rust workspace forbids `unsafe_code` at the lint level.
- CI runs `cargo audit` and `cargo deny` on every push and pull request.
- Releases are built from a tagged commit and carry build-provenance
  attestations (`actions/attest-build-provenance`).
- Evidence bundles (`.invoicekit`) are signed; bundle verification never
  executes shell scripts.

## License of this policy

Apache License 2.0, same as the rest of the project.
