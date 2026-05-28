# Docs site — operator runbook (T-113)

`apps/docs-site/` is the Nextra-themed Next.js site behind
`docs.invoicekit.org`. Pages under `rules/`, `countries/`, and
`operators/` are **generated** on every build by
`tools/docs-generate/generate.py`; nothing per-page is checked
into git.

## Pages shipped

- `/` — landing page (hand-written: `pages/index.mdx`).
- `/rules/` — 81 per-rule pages, one per EN 16931 rule in
  `crates/rulepack/data/en16931-br-co-coverage.json`. Each
  page lists the rule's business terms, validator-testability
  flags, IR paths exercised, and source XSLT line citations.
- `/countries/` — 34 per-country pages, one per
  `crates/report-*` adapter crate. Each page links the cargo
  crate and surfaces its description.
- `/operators/` — every `docs/operators/*.md` runbook mirrored
  in for public reference (one MDX page each).

## Search

Nextra ships with built-in `flexsearch` indexing. The header
search bar indexes every MDX page automatically; no extra
configuration. Try `BR-CO-10` or `Saudi Arabia`.

## Files

- `apps/docs-site/package.json` — Bun + Next.js 15 + Nextra 3.
- `apps/docs-site/{next.config.mjs,theme.config.tsx}` — Nextra
  setup; `output: "export"` so `bun run build` writes static
  HTML into `out/`.
- `apps/docs-site/pages/{index,rules/index,countries/index,operators/index}.mdx`
  — hand-written entry pages (checked in).
- `apps/docs-site/.gitignore` — keeps the generated MDX out
  of git so a rule rename doesn't churn the diff.
- `tools/docs-generate/generate.py` — the MDX fan-out script.
- `tools/docs-generate/tests/test_generate.py` — 5 unit tests
  (run via `pytest tools/docs-generate/tests -q`).
- `apps/docs-site/Dockerfile` — multi-stage build: Python
  generator → Bun + Next.js build → nginx static-host on port
  8080 with `/healthz`.
- `.github/workflows/docs-site.yml` — generator tests + full
  build + static-export artefact upload on every main push
  and PR.

## Local dev

```bash
cd apps/docs-site
bun install
bun run generate     # writes pages/{rules,countries,operators}/
bun run dev          # http://127.0.0.1:3001
bun run build        # static export into out/
```

The `dev` and `build` scripts both call the generator first,
so you don't need to remember to re-run it after touching the
rulepack or a country crate.

## Build the container

```bash
docker build -t invoicekit/docs-site:scaffold \
  -f apps/docs-site/Dockerfile .
docker run --rm -p 8080:8080 invoicekit/docs-site:scaffold
# open http://127.0.0.1:8080
```

## Deploy to docs.invoicekit.org

Two supported topologies:

1. **Static object-store mirror** — sync `apps/docs-site/out/`
   to `s3://docs.invoicekit.org/`; CloudFront / Bunny in front
   for TLS. The CI workflow already uploads the `docs-site-out`
   artefact; the deploy step is one extra `aws s3 sync` line.
2. **Container deploy** — push
   `ghcr.io/muhdur/invoicekit/docs-site:<tag>` and run it
   behind your existing ingress. The image self-hosts via
   nginx and exposes `/healthz`.

Either topology serves the site at the apex.

## Why generate on build?

- Single source of truth: the rule list lives in
  `crates/rulepack/data/`; the country list lives in
  `crates/report-*/Cargo.toml`. A rename or a new adapter
  shows up in docs without a separate doc PR.
- No drift: there's no chance for the doc page and the
  underlying crate description to diverge.
- Tiny diff footprint: the generated MDX is gitignored, so
  PRs that add a country crate show one Cargo.toml diff,
  not 80 hand-rolled markdown files.
