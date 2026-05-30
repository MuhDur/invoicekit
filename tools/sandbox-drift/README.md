# sandbox-drift

Nightly canary that checks whether government / regulator e-invoicing sandboxes have drifted from a recorded response. Developer/CI tooling, not part of the shipped product.

`sandbox_drift.py` reads a list of sandbox gateways from a TOML config, replays one recorded request per gateway against the live endpoint, and reports structural differences between the live response and a stored cassette. It is a drift detector, not a conformance validator and not a live uptime monitor: it runs on a schedule, compares one fixed request per gateway, and never fails the workflow.

## What it does

For each `[[gateway]]` stanza in `data/sandbox-drift/config.toml` the canary runs four steps (see `sandbox_drift.py`):

1. **Credentials check.** If the stanza's `auth_env_var` is unset or empty in the environment, the gateway is recorded as `skipped: no credentials`. A missing env var is the contract for "no sandbox account here yet." Every stanza in the committed config points at an absent env var, so a clean checkout reports everything skipped and zero drift.
2. **Cassette check.** If the recorded cassette at `cassette_path` (or the `request_fixture`) does not exist under the repo root, the gateway is recorded as `skipped: cassette missing`. Absence is not a drift event.
3. **Live replay.** The `request_fixture` JSON (method, path, body, headers) is sent to `endpoint` with a `Bearer` token from the env var. The live response and the cassette's expected response are both passed through `_normalize_response` — which decodes JSON bodies and drops per-call jitter (headers like `date`, `x-request-id`, `etag`, `set-cookie`; body keys like `timestamp`, `requestId`, `trace_id`) — then compared by `_diff`. Any difference in status code, surviving header keys, or JSON body structure/values is a drift point.
4. **Reporting.** Results are bucketed by status (`drift` / `error` / `ok` / `skipped`) into one text report per run. A fetch that raises (network failure, etc.) is recorded as `error`, not drift.

The config (`data/sandbox-drift/config.toml`, `schema_version = "1.0"`) declares each gateway's country, id, description, endpoint, auth env var, cassette path, and request-fixture path. The current stanzas are Italy SDI, France Chorus Pro, Spain VeriFactu, and Saudi ZATCA — all intentionally disabled (env vars that don't exist) to give a hermetic skipped baseline. Cassettes live under `conformance-corpus/cassette/sandbox/`.

The tool always exits 0, by design. A network flake on one country is recorded as a structured `error` event rather than paging the run. When `--report-mode=github-issue` is set and at least one gateway drifted, it shells out to `gh issue create` to open one rolled-up triage issue labelled `sandbox-drift`, `track-6`, `automation`; a missing `gh` CLI or a failed `gh` call prints a warning and is otherwise ignored. The run still exits 0.

## Usage / CI

Local dry-run:

```
python3 tools/sandbox-drift/sandbox_drift.py \
  --config data/sandbox-drift/config.toml \
  --repo-root . \
  --report-mode stdout
```

Flags: `--config` (config TOML path), `--repo-root` (root that cassette + fixture paths resolve against), `--report-mode` (`stdout` or `github-issue`), `--github-repo` (owner/repo for the issue, defaults to `$GITHUB_REPOSITORY`).

CI is `.github/workflows/sandbox-drift.yml`:

- **tests** job (on push / pull request touching the tool or config, plus schedule/dispatch): `pytest tools/sandbox-drift/tests -q`, then runs the canary with `--report-mode stdout` to verify `config.toml` loads.
- **nightly** job (schedule `47 3 * * *` UTC and `workflow_dispatch` only): runs the canary with `--report-mode github-issue`, passing the four `INVOICEKIT_SANDBOX_*` secrets through as optional env vars and `GH_TOKEN` for issue creation. Requires `issues: write`.

Tests (`tests/test_sandbox_drift.py`) exercise the pure-Python core with an injected fake fetcher (no real HTTP): config parsing, schema-version rejection, the skip branches, ok/drift/error outcomes, report grouping, and a smoke test that the committed config loads and every stanza uses an `INVOICEKIT_SANDBOX_*` env var.

## License

Apache-2.0
