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

## Release-pipeline gates (T-002 + T-133)

Two distinct tag namespaces produce signed artifacts, each with its
own gated pipeline. Both gates run **before** any build slot is
burned, so a newly-disclosed CVE in a transitive dependency cannot
ride a deploy.

| Namespace | Pipeline | Produces |
| --- | --- | --- |
| `v*` | [`.github/workflows/release.yml`](.github/workflows/release.yml) | Library crates + CLI binaries (cross-target) |
| `hosted-v*` | [`.github/workflows/hosted-release.yml`](.github/workflows/hosted-release.yml) | Hosted-layer binaries: `invoicekit-managed-api-server`, `invoicekit-signer-agent` |

Both pipelines run, in order:

1. `cargo audit --deny warnings` against the RustSec advisory DB.
2. `cargo deny check` for licenses, banned crates, sources, and
   advisories (per `deny.toml`).
3. Strict-mode `cargo build --release --locked`.
4. CycloneDX SBOM via `cargo cyclonedx`.
5. Keyless cosign sign-blob over every produced artifact + SBOM.
6. SLSA-shape provenance attestation via
   `actions/attest-build-provenance`.

Verifying a downloaded artifact:

```sh
cosign verify-blob \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp '^https://github.com/MuhDur/invoicekit/.+@refs/tags/.*' \
  --bundle invoicekit-managed-api-server.cosign.bundle \
  invoicekit-managed-api-server
```

## SBOM consumption

The CycloneDX 1.4 JSON SBOMs uploaded to each GitHub release are the
canonical inventory of every crate baked into the corresponding
binary. Downstream operators can feed them into Dependency-Track,
Trivy, Grype, or their preferred SBOM scanner. The hosted-release
pipeline uploads one SBOM per hosted binary so a scanner sees the
exact dependency tree for the service it's about to deploy.

## Advisory process

When a vulnerability is confirmed:

1. The maintainer opens a private GitHub Security Advisory and
   begins the coordinated-disclosure clock.
2. The fix lands as a normal PR against `main`, including a
   regression test pinned in the advisory body.
3. A patch tag is cut (`vX.Y.Z+1` for the library or `hosted-vX.Y.Z+1`
   for the hosted layer). The release-pipeline gates re-run, so the
   tag cannot ship if the fix accidentally regressed another
   dependency.
4. The advisory is published with the fix commit hash, the CVE, the
   affected version range, and (when available) the reporter
   acknowledgement.
5. Downstream operators are notified via the GitHub Releases RSS
   feed and via the `security` discussion category.

## License of this policy

Apache License 2.0, same as the rest of the project.
