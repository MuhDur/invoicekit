# sandbox-prod-parity — nightly sandbox-vs-production response diff canary

A developer/CI canary that replays the same request fixture against a country's
sandbox and production e-invoicing endpoints, normalizes both responses, diffs
them, and reports any drift. It is internal tooling, not part of the shipped
product.

This is the T-074d canary. It is the sibling of `tools/sandbox-drift`
(T-074c, sandbox-only), and reuses that tool's `_normalize_response` and `_diff`
helpers so both report drift in the same shape. The defining difference: this
canary *may* touch production endpoints, so it is gated on recorded customer
consent.

## What it does

`parity_diff.py` reads pair definitions from
`data/sandbox-prod-parity/config.toml`. The config has a `schema_version` (must
be `"1.0"`) and a list of `[[pair]]` stanzas. Each stanza names a country, a
pair id, a sandbox endpoint, a production endpoint, the environment-variable
names that hold each side's auth token, a request fixture path, and a
`consent_signed_at` timestamp.

For each pair, `check_pair` applies two gates before any network call, then
diffs:

1. **Consent gate.** If `consent_signed_at` is empty, the pair is recorded as
   `skipped` with detail "no production-call consent on file" and the function
   returns without calling either endpoint. The test suite asserts the
   production endpoint is never fetched when consent is absent.
2. **Credentials gate.** Both the sandbox and production auth environment
   variables must be set. Either missing yields `skipped` ("missing sandbox
   credentials" / "missing production credentials") with no HTTP call.
3. **Replay both sides.** The request fixture (JSON) is loaded relative to
   `--repo-root`; a missing fixture is `skipped`, an unparseable one is `error`.
   The fixture's `method`, `path`, `body`, and `headers` drive an
   `Authorization: Bearer <token>` request to both endpoints via
   `urllib.request` (30s timeout). HTTP errors are captured as a response rather
   than raised; any other fetch exception yields `error`.
4. **Normalize and diff.** Both responses (status, lowercased header keys, body)
   are passed through `_normalize_response` to strip per-call jitter, then
   compared with `_diff`. Any difference makes the pair `drift`; otherwise `ok`.

`render_report` groups results by status (DRIFT, ERROR, OK, SKIPPED) into a
Markdown summary that includes the bead id. When invoked with
`--report-mode=github-issue` and at least one pair drifted, `open_drift_issue`
shells out to `gh issue create` with the labels `sandbox-prod-parity`,
`track-6`, and `automation`. If `gh` is missing or the call fails, a warning is
written to stderr. `main` always returns `0` — the issue is the page, not the
exit code.

### Config and fixtures

`data/sandbox-prod-parity/config.toml` ships three pairs: Italy (SDI), France
(Chorus Pro), and Spain (VeriFactu). Every stanza is intentionally disabled —
`consent_signed_at` is empty and the auth env vars point at variables that do
not exist by default — so the workflow has a hermetic "every pair skipped"
baseline. Real consent timestamps and credentials are supplied per customer by
the principal; production tokens come from the secret manager, never inlined in
the config. The referenced request fixtures live under
`conformance-corpus/cassette/parity/`.

## Usage / CI

Local dry run (prints the Markdown report to stdout, never opens an issue):

```
python3 tools/sandbox-prod-parity/parity_diff.py \
  --config data/sandbox-prod-parity/config.toml \
  --repo-root . \
  --report-mode stdout
```

Tests:

```
pytest tools/sandbox-prod-parity/tests -q
```

CI is `.github/workflows/sandbox-prod-parity.yml`:

- The `tests` job runs the pytest suite and a `--report-mode stdout` config-load
  check on push/PR touching this tool or its data, and on the nightly schedule.
- The `nightly` job runs only on `schedule` (04:13 UTC) or `workflow_dispatch`.
  It exports per-pair sandbox and production tokens from repository secrets and
  invokes the script with `--report-mode github-issue` (`issues: write`
  permission). With the committed baseline config (no consent, no real
  credentials), every pair is skipped and no production endpoint is touched.

## What it is not

- It is not a live monitor or alerting service; it is a scheduled canary that
  runs at most nightly.
- It is not a validator or conformance check; it diffs two endpoints' raw
  responses and does not assert correctness of either.
- It is not a credential or consent store; it only reads `consent_signed_at`
  from config and tokens from environment variables.
- It does not fail CI on drift; it opens a GitHub issue and exits `0`.

## License

Apache-2.0.
