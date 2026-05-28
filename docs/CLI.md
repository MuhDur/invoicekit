# `invoicekit` CLI — the trust-toolkit walkthrough

This doc walks through the operator loop the `invoicekit` binary ships today: **pack → show → verify → replay → unpack → diff**. Plus the environment-diagnostic command `doctor`, the migration tool `migrate-archive`, the capability resolver `capabilities`, and the code-list updater `codelist-update`.

Every subcommand:

- accepts `--help`, prints a one-line usage and exits non-zero,
- emits structured JSON for scripting (`--json` where applicable),
- has stable exit codes: **0 success, 1 substantive failure, 2 usage error**,
- has zero network dependencies unless explicitly noted.

## Install (from source)

```bash
git clone https://github.com/MuhDur/invoicekit.git
cd invoicekit
cargo build --release -p invoicekit-cli --bin invoicekit
# Binary: ./target/release/invoicekit
```

The rest of this doc assumes `invoicekit` is on `$PATH`.

## End-to-end loop in 30 seconds

```bash
# 1. Stage some artefacts (canonical JSON + format renderings).
mkdir -p sample/formats
echo '{"id":"INV-DEMO-1"}' > sample/canonical.json
echo '<Invoice/>'           > sample/formats/ubl.xml
echo '<CrossIndustryInvoice/>' > sample/formats/cii.xml

# 2. Pack a deterministic evidence bundle.
invoicekit pack sample dist.ikb \
  --tenant acme \
  --trace trace-2026-05-28-001 \
  --created-at 2026-05-28T05:00:00Z

# 3. Inspect what's inside without writing anything to disk.
invoicekit show dist.ikb

# 4. Verify the content-address ledger.
invoicekit verify dist.ikb

# 5. Replay through the identity replayer (deterministic
#    byte-equal baseline; once the engine wires in real
#    replayers this will surface drift).
invoicekit replay dist.ikb

# 6. Extract for human inspection.
invoicekit unpack dist.ikb extracted/

# 7. Diff two bundles artefact-by-artefact (useful for audit).
invoicekit pack sample dist-v2.ikb --created-at 2026-05-28T05:00:00Z
invoicekit diff dist.ikb dist-v2.ikb     # → byte-equal, exit 0
```

## Subcommand reference

### `pack`

Pack every regular file under `<input-dir>` as an artefact (id = path relative to the input root, forward slashes regardless of OS) into a deterministic `.ikb` evidence bundle.

```
invoicekit pack <input-dir> <output.ikb> \
    [--tenant ID] [--trace ID] [--created-at RFC3339]
```

Defaults — `tenant=unset-tenant`, `trace=unset-trace`, `created-at=1970-01-01T00:00:00Z` — are chosen so the same directory packed twice without overrides produces byte-identical output.

Exit codes: `0` written, `1` pack failure, `2` usage error.

### `show`

Print a human-readable manifest summary (or JSON with `--json`). Read-only; never writes to disk.

```
invoicekit show <bundle.ikb> [--json]
```

Sample human output:

```
Schema:        1.0
Created at:    2026-05-28T05:00:00Z
Tenant:        acme
Trace:         trace-2026-05-28-001
Container:     449 bytes
Artefacts:     3

id                                              size  blake3
--                                              ----  ------
canonical.json                                    20  700912e58eb800e5…
formats/cii.xml                                   23  1defcab5b08857ad…
formats/ubl.xml                                   10  b7a8182821f0d741…
```

### `verify`

Re-hash every artefact in the bundle and reconcile against the manifest. Today only the **content-address** check runs; signature + RFC 3161 timestamp checks are skipped until the CLI gains a signer / TSA client (T-083a / T-082 follow-ups).

```
invoicekit verify <bundle.ikb>
```

Prints a structured JSON report; exit `0` on pass, `1` on any failed check, `2` on usage error.

### `replay`

Unpack the bundle and re-run each recorded artefact through the **identity replayer**, reporting per-artefact verdicts (`byte-equal`, `drifted`, `not-replayed`, `unexpected`).

```
invoicekit replay <bundle.ikb>
```

Today the identity replayer always replays byte-equal — this is the baseline. The moment a real pipeline replayer is wired in, this same subcommand will start surfacing engine drift without any flag changes.

Exit `0` if every selected artefact replays byte-equal, `1` on any drift, `2` on usage error.

### `unpack`

Extract every artefact from a bundle into `<output-dir>`, preserving artefact ids as path components. Re-serialises the manifest as `manifest.json` alongside the payloads.

```
invoicekit unpack <bundle.ikb> <output-dir> [--force]
```

Refuses to write into a non-empty `<output-dir>` unless `--force` is set. Rejects artefact ids that would escape the output directory (absolute paths, `..` components, rooted paths).

### `diff`

Compare two bundles artefact-by-artefact:

```
invoicekit diff <left.ikb> <right.ikb> [--json]
```

Per-artefact verdicts: `byte-equal`, `changed`, `only-in-left`, `only-in-right`. Human output prints `[EQ|CHG|L  |  R]` tags per id; `--json` emits the full structured report.

Exit `0` on byte-equal across all artefacts, `1` on any diff, `2` on usage error.

### `doctor`

Environment diagnostics. Inspects what's locally observable so the command stays fast and works offline:

```
invoicekit doctor [--workspace PATH] [--json]
```

Checks:

| check | covers |
|---|---|
| `rust-toolchain` | `rustc --version` |
| `workspace-layout` | `Cargo.toml` + `crates/` visible |
| `codelist-data` | code-list data tree present |
| `sidecar-validator-kosit` | TCP probe of `127.0.0.1:7001` (200 ms timeout) |
| `sidecar-validator-phive` | …port 7002 |
| `sidecar-validator-saxon` | …port 7003 |
| `sidecar-validator-verapdf` | …port 7004 |
| `sidecar-validator-phase4` | …port 7005 |
| `sidecar-invoicekit-signer-agent` | …port 7100 |

Human output prints `[PASS|WARN|FAIL] name — detail` with indented remediation lines; `--json` emits a structured report. Exit `0` on no-fail (warnings ok), `1` on any fail, `2` on usage error.

### `capabilities`, `codelist-update`, `migrate-archive`

Pre-existing subcommands; see their respective `--help` output. Briefly:

- **`capabilities`** — resolve accepted e-invoice profiles for a given country / scenario / date.
- **`codelist-update`** — refresh a code-list manifest from a locally-staged upstream payload.
- **`migrate-archive`** — migrate a directory of invoice JSON archives between schema versions.

## Wiring into CI

The repo ships a composite GitHub Action that does the build-and-verify dance for downstream consumers:

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

Full reference: [`.github/actions/verify-bundle/README.md`](../.github/actions/verify-bundle/README.md).
