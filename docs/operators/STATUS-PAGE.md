# Status page operator runbook

`tools/status-page/` builds a deterministic single-file HTML status
page from a TOML config and an append-only incident ledger.
`.github/workflows/status-page.yml` re-renders the page on every push
to `tools/status-page/` and publishes it to GitHub Pages on `main`.

This runbook covers the one-time operator setup and the per-incident
operator flow.

## One-time setup

1. **Enable GitHub Pages on the repo.** Settings → Pages → *Source*
   → *GitHub Actions*. The `status-page` workflow already declares
   the `pages: write` permission and uses
   `actions/upload-pages-artifact` + `actions/deploy-pages`, so no
   further wiring is needed once Pages itself is enabled.
2. **Create the `github-pages` environment.** Settings →
   Environments → New environment → `github-pages`. The deploy step
   in the workflow targets this environment by name. Add a deployment
   protection rule that requires a reviewer if the operator wants a
   human gate on every publish.
3. **Point the custom domain.** Configure `status.invoicekit.org` as
   a `CNAME` to `<owner>.github.io`. The deployed page's
   `<link rel=canonical>` already points at the custom domain so
   search engines don't index the github.io alias.

## Adding a new target

A "target" is either a country (`it-sdi`, `fr-ctc`, …) or a gateway
(`peppol-storecove`, …) that the status page tracks uptime for.
Edit `tools/status-page/status.toml`:

```toml
[targets.de-xrechnung]
kind = "country"
label = "Germany — XRechnung partner endpoint"
```

Open a PR; the workflow regenerates the page on merge.

## Recording an incident

Incidents are append-only Markdown files under
`tools/status-page/incidents/`. The filename convention is
`<YYYY-MM-DD>-<short-slug>.md`:

```markdown
---
target: peppol-storecove
title: "Storecove sandbox 503s for 12 minutes"
minutes: 12
status: resolved
---

12-minute partial outage on Storecove's EU sandbox. Affected
documents queued via our retry layer were re-delivered without
caller intervention. Storecove acknowledged the outage in their
status page within 5 minutes.
```

Header keys:

- `target` — must match a key under `[targets]` in `status.toml`.
- `title` — short headline displayed on the page.
- `minutes` — non-negative integer; the rolling-window uptime
  calculation subtracts this from the available time.
- `status` — one of `resolved`, `monitoring`, `investigating`.

The body is rendered as a plain paragraph; multi-paragraph notes
are intentionally collapsed to keep the public page brief. Long
post-mortems belong in a separate `docs/post-mortems/` tree, not
here.

## Re-running the build locally

```
python3 tools/status-page/build_status_page.py \
  --config tools/status-page/status.toml \
  --incidents-dir tools/status-page/incidents \
  --out target/status-page/index.html
```

Open `target/status-page/index.html` in a browser to preview.
Same inputs produce byte-identical output, so a diff against the
committed page is meaningful when reviewing a PR that touches the
config or the ledger.

## Why a static page and not a third-party status service

InvoiceKit's trust toolkit pitch leans on operator-visible
infrastructure that the audience can audit. A third-party status
service (StatusGator, Statuspage, Better Stack) would let us claim
9 nines, hide a real incident, and recall the claim with no
public trace. A repo-resident incident ledger is auditable via
`git log` and survives the company outliving any vendor.
