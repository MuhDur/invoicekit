# status-page — static HTML generator for the InvoiceKit public status site

A single Python script that renders a one-file `index.html` from an operator-edited
config and an append-only Markdown incident ledger. It is developer/CI tooling, not
part of the shipped product, and it does no live probing: uptime numbers are computed
from the recorded incidents, not from real-time checks against any gateway or country
endpoint.

## What it does

`build_status_page.py` reads two inputs and writes one HTML file:

- `status.toml` — site config. A `[site]` table (`title`, `url`, `window_days`,
  default 90) and a `[targets.<key>]` table per target. Each target declares a
  `kind` (`country` or `gateway`) and a `label`. Both `[site]` and `[targets]` are
  required; `[targets]` and each target entry must be tables.
- `incidents/` — append-only ledger. One Markdown file per incident, named
  `YYYY-MM-DD-<slug>.md`, with a `---`-delimited front-matter header carrying
  `target`, `title`, `minutes`, and `status` (all required), followed by free-text
  notes. `minutes` must be a non-negative integer. `target` is matched literally
  against a target key from the config (e.g. `it-sdi`, `peppol-storecove`).

For each target, over the rolling `window_days` window, it sums the `minutes` of
incidents whose `target` matches and renders a row with computed uptime percent,
incident count, and total downtime minutes:

    uptime = max(0, 100 - total_minutes / (window_days * 24 * 60) * 100)

It then renders each in-window incident as a block (date, title, target, status,
duration, notes) and appends a 12-character SHA-256 build digest over the rendered
content. All text from config and incidents is HTML-escaped. Output is deterministic:
the same inputs produce byte-identical HTML.

Invalid input — missing config, malformed TOML, a missing `[site]`/`[targets]` table,
an incident filename that breaks the naming convention, missing front-matter, a
missing required header key, or a non-integer/negative `minutes` — prints to stderr
and exits `2`. A clean run exits `0`.

The repository ships one sentinel fixture, `incidents/2026-05-26-bootstrap.md`
(`minutes: 0`), so the renderer always has at least one incident to walk.

## Usage / CI

Run directly:

    python3 tools/status-page/build_status_page.py \
        --config tools/status-page/status.toml \
        --incidents-dir tools/status-page/incidents \
        --out target/status-page/index.html

All three flags (`--config`, `--incidents-dir`, `--out`) are required; the output
parent directory is created if missing.

In CI, `.github/workflows/status-page.yml` runs this command on every push and pull
request that touches `tools/status-page/**` (and on `workflow_dispatch`). The build
job uploads the rendered page as an artifact so reviewers can preview it. On `main`,
a separate deploy job publishes to GitHub Pages — but only when Pages and the
`github-pages` environment are configured (the one-time setup is documented in
`docs/operators/STATUS-PAGE.md`).

## License

Apache-2.0.
