# validate-action — operator runbook (T-115)

`actions/validate-action/` is the composite GitHub Action
published as `invoicekit/validate-action@v1`. Drop into any
repo's CI to validate UBL / CII / Peppol invoices against EN
16931 + Peppol BIS rules.

## Files

- `actions/validate-action/action.yml` — composite action
  manifest. Installs the InvoiceKit CLI, runs
  `invoicekit validate` over the input glob, prints findings
  as GitHub annotations, uploads a JSON summary as an
  artefact, and exposes finding-count outputs.
- `actions/validate-action/README.md` — public README the
  Marketplace listing renders.
- `actions/validate-action/examples/basic.yml` — minimal
  consumer workflow example.
- `actions/validate-action/tests/test_action_yml.py` —
  5 schema-level unit tests that the manifest is parseable,
  exposes every input/output the README contract documents,
  uses a Marketplace-allowed branding icon + color, and
  invokes the InvoiceKit CLI with the expected flags.
- `.github/workflows/validate-action.yml` — runs the manifest
  tests on every push to main and every PR touching the
  action.

## Publishing to the GitHub Marketplace

1. **Tag a release.** From `main`, tag `vMAJOR.MINOR.PATCH`
   (e.g. `v1.0.0`). The tag must point at a commit whose
   `actions/validate-action/action.yml` parses cleanly under
   the schema tests.

   ```bash
   git tag -a v1.0.0 -m "validate-action v1.0.0"
   git push origin v1.0.0
   ```

2. **Open the Releases page** at
   `https://github.com/MuhDur/invoicekit/releases/new?tag=v1.0.0`.

3. **Tick "Publish this Action to the GitHub Marketplace"**
   in the release form. GitHub validates the action.yml
   against its own Marketplace schema; if the schema tests in
   this repo are green the publish step should not bounce.

4. **Pick the primary category.** Recommended: "Continuous
   integration"; secondary "Code quality".

5. **Submit.** The action becomes available at
   `https://github.com/marketplace/actions/invoicekit-validate`.

6. **Move the floating `v1` tag** to point at the new release
   so consumers using `@v1` get the update automatically:

   ```bash
   git tag -fa v1 v1.0.0 -m "move floating v1 tag"
   git push origin v1 --force
   ```

   Don't move `v1` if the release is a breaking change — cut
   a `v2` floating tag instead.

## Test the action locally

```bash
# Validate the manifest itself
python3 -m unittest discover -s actions/validate-action/tests

# Smoke-test against a real invoice (requires the CLI on PATH)
invoicekit validate \
  --glob "conformance-corpus/synthetic/ubl-2-1/*/fixture.xml" \
  --rule-pack "en16931-2017+peppol-bis-3.0" \
  --fail-on error \
  --json /tmp/summary.json \
  --annotations github
```

## Why composite (not Node / Docker)?

- **Composite** ships in seconds; no container pull, no Node
  bootstrap. The CLI install step is the only network call.
- **Easy to audit.** Consumers can read `action.yml` end to
  end without dropping into a Node toolchain.
- **Matches the InvoiceKit "trust toolkit" stance.** Nothing
  proprietary on the runner; the CLI is the same Apache 2.0
  binary `apt-get install`/Homebrew users already trust.
