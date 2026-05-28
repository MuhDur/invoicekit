# Validator UI — operator runbook (T-035)

`apps/validator-ui/` is the public single-page app behind
`validate.invoicekit.org`. Dual-mode validation per the bead:

- **Local** — `invoicekit-wasm-browser` runs entirely in the
  client. The XML never leaves the device. Today's scaffold
  emits a `ui.scaffold.wasm-pending` warning until the WASM
  binding wires up; the SPA still renders end-to-end so the
  layout, error handling, and analytics work in production.
- **Reference** — POSTs the XML to the JVM validator sidecar
  service (`validator-kosit` / `validator-phive`). Clearly
  labelled in the UI; no retention by default.

## Files

- `apps/validator-ui/package.json` — Bun + Vite + React 19 SPA.
- `apps/validator-ui/src/{validator,analytics}.ts` — the two
  modes and the PII-free analytics sink.
- `apps/validator-ui/src/App.tsx` — UI shell, mode switch,
  textarea input, findings table. Shows mode + rule pack +
  backend + elapsed per result.
- `apps/validator-ui/tests/*.test.ts` — Bun unit tests for the
  validator dispatch and the analytics sink (7 tests total).
- `apps/validator-ui/Dockerfile` — multi-stage Bun build → nginx
  static-host on port 8080 with `/healthz` and aggressive
  asset caching.
- `apps/validator-ui/nginx.conf` — SPA fallback + cache policy.
- `.github/workflows/validator-ui.yml` — install / typecheck /
  test / build on every push to main and PR.

## Configuration

Vite-time env vars:

| var | default | meaning |
|---|---|---|
| `VITE_REFERENCE_VALIDATOR_URL` | `https://reference.validate.invoicekit.org` | base URL of the JVM sidecar (`/validate` is appended) |
| `VITE_ANALYTICS_ENDPOINT` | (unset → analytics disabled) | endpoint that accepts POSTed JSON events |

## Local dev

```bash
cd apps/validator-ui
bun install
bun run dev          # http://127.0.0.1:5174
bun test             # unit tests
bun run check        # tsc --noEmit
bun run build        # writes dist/
```

## Build the container

```bash
docker build -t invoicekit/validator-ui:scaffold \
  -f apps/validator-ui/Dockerfile .
docker run --rm -p 8080:8080 invoicekit/validator-ui:scaffold
# open http://127.0.0.1:8080
```

## Deploy to validate.invoicekit.org

Two supported topologies:

1. **Static object-store mirror** — sync `apps/validator-ui/dist`
   to `s3://validate.invoicekit.org/` after each main-branch
   build; CloudFront / Bunny in front for TLS. The CI workflow
   already uploads the `validator-ui-dist` artefact; the
   deploy step is one extra `aws s3 sync` line per the host.
2. **Container deploy** — push `invoicekit/validator-ui:<tag>`
   to the registry and run it behind your existing ingress.
   The image self-hosts via nginx and exposes `/healthz`.

Either topology serves the SPA at the apex; the JVM sidecar at
`reference.validate.invoicekit.org` is a separate deployment
(see the existing `services/validator-kosit/` docs).

## Analytics

Events fired (no payload bytes, no PII, no IP beyond the
analytics endpoint's request envelope):

- `page_view` — on initial load.
- `validation_started` — `{ mode }`.
- `validation_completed` — `{ mode, finding_count }`.

Wire to Plausible / Umami / a custom collector via
`VITE_ANALYTICS_ENDPOINT`. The sink uses `navigator.sendBeacon`
when available, with a `fetch keepalive: true` fallback so the
drop-off ping fires even on tab close.

## Findings layout

Every result row shows the four headline fields per the bead
acceptance gate:

- **Mode** — `local` or `reference`.
- **Rule pack** — string, e.g. `en16931-2017+peppol-bis-3.0.18`.
- **Backend** — string, e.g. `validator-kosit-1.5.0` or
  `invoicekit-wasm-browser@<version>`.
- **Elapsed** — round-trip ms (client-side timer).

The findings table renders severity / rule_id / message rows.
Findings shape is symmetric across both modes so the audit UI
can diff local vs. reference output for the same input.
