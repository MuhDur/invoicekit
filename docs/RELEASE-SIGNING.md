# Release signing and SBOM operator runbook

This runbook covers the operator-side setup that the
`.github/workflows/release.yml` workflow expects to be in place
before a `v*` tag will produce a fully signed release with a
verifiable software bill of materials.

The workflow itself is already wired. What follows is the
one-time setup an operator does on the GitHub repo + sigstore
side so the cosign keyless step and the CycloneDX upload
actually carry through.

## What ships today

Every `v*` tag triggers `.github/workflows/release.yml`, which:

1. Runs `cargo audit` and `cargo deny` against the locked workspace.
2. Builds release binaries per target.
3. Generates a CycloneDX SBOM (one `bom.json` per workspace
   crate, packed into `target/sbom/invoicekit.cdx.tar`).
4. Signs every emitted `invoicekit*` binary with
   `cosign sign-blob --yes`, producing a sidecar
   `<binary>.cosign.bundle`.
5. Generates an attestation via
   `actions/attest-build-provenance@v2`.
6. Attaches binaries, cosign bundles, and the SBOM tarball to
   the GitHub release.

`tools/release-checks/verify_release_checks.py` asserts that
the `cosign sign-blob` and `cargo cyclonedx` steps are still
wired in. Its tests (`tools/release-checks/tests/`) cover the
"step was removed" regression in both directions.

## Operator setup checklist

Tick these once per repo. None of them can be done from this
repository's CI alone — they require the GitHub repo admin to
act on the GitHub or sigstore web side.

### 1. Enable OIDC trust for sigstore

`cosign sign-blob` runs without a long-lived key by reading an
OIDC token from the GitHub runner. The runner needs
`id-token: write` permission on the workflow (already declared
in `release.yml`) **and** the sigstore Fulcio policy needs to
trust this repo's OIDC issuer.

- [ ] Confirm `Settings > Actions > General > Workflow permissions`
  allows OIDC token issuance for tagged release runs.
- [ ] If the org enforces an OIDC subject-allowlist, ensure the
  pattern matches `repo:OWNER/invoicekit:ref:refs/tags/v*`.
- [ ] Verify a dry-run sign on a throwaway branch:
  `gh workflow run release.yml --ref test-cosign-tag` against a
  test tag. The job logs should print
  `Successfully verified Fulcio signature` for each binary.

### 2. Publish SBOM artifacts

The CycloneDX upload step is gated on the tag matching `v*`. No
operator action is required to start emitting SBOM tarballs —
they appear on every tagged release automatically.

What is operator-decided:

- [ ] Whether the SBOM is mirrored to a public registry
  (e.g. `docker push <sbom-image> oci.invoicekit.org/sbom`).
  The current workflow uploads to the GitHub release only; a
  mirror is intentionally out of scope until v1.
- [ ] Whether to enable Dependency-Track ingest of the
  `bom.json` artifacts. If so, configure a Dependency-Track
  API token under `Settings > Secrets > Actions` as
  `DEPENDENCY_TRACK_API_KEY` and wire a separate
  `release-sbom-publish.yml` workflow that consumes the
  uploaded artifact and pushes to Dependency-Track.

### 3. Document the verification recipe for downstream users

Every signed release needs a one-paragraph instruction so
downstream consumers can verify the bundle. Suggested copy for
the release notes template:

> Verify a release binary:
> ```
> cosign verify-blob \
>   --bundle invoicekit-x86_64-unknown-linux-gnu.cosign.bundle \
>   --certificate-identity-regexp '^https://github.com/OWNER/invoicekit/.+@refs/tags/v.+$' \
>   --certificate-oidc-issuer https://token.actions.githubusercontent.com \
>   invoicekit-x86_64-unknown-linux-gnu
> ```

## What to do if `verify_release_checks.py` fails locally

The verifier walks `release.yml` and asserts the cosign +
cyclonedx steps are present. If you intentionally remove or
rename a step, update both the workflow and the corresponding
`Requirement` in `tools/release-checks/verify_release_checks.py`,
then re-run:

```
python3 -m pytest tools/release-checks/tests/test_verify_release_checks.py
```

The tests assert that the verifier catches the removal in both
directions, so removing a step without updating the test will
trip the existing regression coverage.

## Why no key-based signing fallback

InvoiceKit ships under Apache 2.0 and targets a developer
audience that already trusts GitHub's OIDC identity for every
other open-source release they consume. Adding a key-based
signing fallback would require an operator-managed HSM, a key
rotation runbook, and a way to publish public keys
out-of-band — none of which adds verifiability beyond what
sigstore already gives us through Fulcio. The trust toolkit
direction document explicitly favours public, audit-friendly
infrastructure over private credentials.

If a downstream consumer can't accept the sigstore root for
operational reasons, the right response is *also* publishing
under a deterministic-build manifest (a follow-up bead), not
adding a second signing path that diverges from the first.
