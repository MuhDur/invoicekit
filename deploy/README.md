# deploy/

Deployment artifacts. Not a Cargo workspace member.

- `docker-compose.yml` — single-host managed-stack deployment, owned end-to-end by bead **T-1300**. Layout per `plans/PLAN.md` §4.1.
- `secrets/` — operator-managed secret files referenced by `docker-compose.yml`. Not committed (see `.gitignore`).
- `smoke.sh` — health-check probe; run after `docker compose up -d`.
- `helm/` — Kubernetes Helm chart, owned by bead **T-1301**.
- `terraform/` — Terraform module for managed cloud provisioning, owned by bead **T-1302**.

## Quick start

```bash
cd deploy

# 1. Seed the secrets the compose file references. Use real
#    values in production; the lines below are dev defaults.
mkdir -p secrets
printf 'change-me-postgres'           > secrets/postgres_password
printf 'change-me-minio'              > secrets/minio_password
printf '/dev/null'                    > secrets/peppol_ap_cert_p12
printf 'change-me-peppol-cert-pass'   > secrets/peppol_ap_cert_pass
chmod 600 secrets/*

# 2. Bring up the stack.
docker compose up -d

# 3. Probe every service.
./smoke.sh
```

## Services

| Service | Port | Owner bead | Image (scaffold) |
| --- | --- | --- | --- |
| `postgres` | 5432 (internal) | T-130 | `postgres:16-alpine` |
| `managed-api-server` | 8080 (host) | T-130 + T-1300 | `ghcr.io/muhdur/invoicekit/managed-api-server:scaffold` |
| `signer-agent` | (Unix socket) | T-083 | `ghcr.io/muhdur/invoicekit/signer-agent:scaffold` |
| `validator-kosit` | 8080 (internal) | T-030 | `ghcr.io/muhdur/invoicekit/validator-kosit:scaffold` |
| `validator-phive` | 8080 (internal) | T-030 | `ghcr.io/muhdur/invoicekit/validator-phive:scaffold` |
| `validator-saxon` | 8080 (internal) | T-030 | `ghcr.io/muhdur/invoicekit/validator-saxon:scaffold` |
| `validator-verapdf` | 8080 (internal) | T-058 | `ghcr.io/muhdur/invoicekit/validator-verapdf:scaffold` |
| `validator-phase4` | 8090 (internal) | T-092 | `ghcr.io/muhdur/invoicekit/validator-phase4:scaffold` |
| `archive-minio` | 9000 / 9001 (internal) | T-081 | `minio/minio:RELEASE.2025-04-22T22-12-26Z` |

Internal-only ports are reachable from inside the compose network at
`<service-name>:<port>`. The only host-exposed port is `8080` on
`managed-api-server`; expose more via the operator's reverse proxy
(see T-110 `bindings/rest-shim`).

## Secrets contract

Every secret file is required by at least one service:

| File | Used by | Notes |
| --- | --- | --- |
| `secrets/postgres_password` | `postgres`, `managed-api-server` | PostgreSQL `POSTGRES_PASSWORD_FILE` env. |
| `secrets/minio_password` | `archive-minio` | MinIO root password. |
| `secrets/peppol_ap_cert_p12` | `validator-phase4` | OpenPeppol AP PKCS#12 bundle. Production only. |
| `secrets/peppol_ap_cert_pass` | `validator-phase4` | Passphrase for the P12 bundle above. |

For development, point the Peppol AP secrets at `/dev/null` and a throwaway
passphrase — the phase4 sidecar runs in scaffold mode and does not actually
unwrap the certificate.

## Smoke test contract (`smoke.sh`)

The script exits 0 iff every service the stack ships answers a 2xx on its
health probe:

- `postgres`: `pg_isready -h 127.0.0.1 -p 5432 -U invoicekit`
- `managed-api-server`: `GET http://127.0.0.1:8080/health` → 2xx
- `validator-*`: `GET http://<host>:<port>/health` → 2xx (probed from
  inside the compose network via `docker compose exec`)
- `validator-phase4`: JSON-RPC `health` method → `version` + `sml` fields
- `archive-minio`: `GET http://127.0.0.1:9000/minio/health/ready` → 2xx

Exit 1 on any single failure. Run after every `compose up` and before
opening a customer-facing endpoint.

## Production cuts the operator must make

The scaffold values are intentionally insecure so the dev loop is fast.
Before going live, replace at minimum:

- The five secrets above with real values from your secrets manager
  (HashiCorp Vault, AWS Secrets Manager, SOPS-encrypted files).
- The MinIO bucket / region / Object Lock retention policy. Default is
  unencrypted local volume — fine for dev, never for production.
- The Postgres backup policy. Compose does not run pg_basebackup; bolt on
  WAL-G or pgbackrest before traffic.
- The TLS-terminating reverse proxy (T-110); the managed API listens
  plain-HTTP inside the compose network.

Scaffolded by bead **invoices-t-001-cargo-workspace-xos**; T-1300 ships the
operator-readable stack documented above. CI smoke-test integration +
production-ready image build land as T-1300a follow-up.
