# `verify-bundle` — composite GitHub Action

Verify an InvoiceKit `.invoicekit` / `.ikb` evidence bundle in CI. The action re-hashes every artefact in the bundle and reconciles against the manifest. If the bundle fails the content-address check, the job fails.

## Usage

```yaml
jobs:
  audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: MuhDur/invoicekit/.github/actions/verify-bundle@main
        with:
          bundle: dist/release.ikb
```

## Inputs

| name | required | default | meaning |
|---|---|---|---|
| `bundle` | yes | — | Path (relative to your repo root) to the `.invoicekit` / `.ikb` bundle. |
| `show` | no | `true` | When `true`, prints a human-readable manifest summary before verifying. |
| `invoicekit-repo` | no | `MuhDur/invoicekit` | Override only if you fork the engine. |
| `invoicekit-ref` | no | `main` | Git ref to check out from `invoicekit-repo`. Pin to a release tag once one exists. |

## Outputs

| name | meaning |
|---|---|
| `verdict` | `pass` if the bundle's content-address check passed, `fail` otherwise. The action also exits non-zero on `fail`, so most workflows can just rely on the step failure. |

## What "verify" checks today

* **content-address** — every artefact's BLAKE3 hash matches the manifest, the manifest itself is intact, and the container round-trips losslessly through `pack` → `unpack`.

Signature and RFC 3161 timestamp checks are **skipped** in this action until the CLI gains a signer/TSA client (T-083a / T-082 follow-ups). The action will start exercising them automatically once that lands.

## Build time

The action builds the `invoicekit` CLI from source on the runner. That keeps the contract simple (`uses:` works without downloads or env tricks) but adds ~one Cargo build per workflow run. Once we cut a release with prebuilt binaries the action will switch to a download path.

For tight CI budgets, add `Swatinem/rust-cache@v2` before the `uses:` step — the action's own cache step will hydrate from your workflow's cache.
