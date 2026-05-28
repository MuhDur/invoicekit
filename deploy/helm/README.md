# deploy/helm — InvoiceKit Helm chart

Kubernetes counterpart to `deploy/docker-compose.yml`. Owned end-to-end by bead **T-1301**.

The chart `invoicekit/` ships every service the compose file does (`postgres`, `managed-api-server`, `signer-agent`, four JVM validator sidecars, the `validator-phase4` AS4 sidecar, and an in-cluster `minio` archive backend) behind a single `values.yaml` contract.

## Quick start

```bash
# 1. Provision the secrets the chart references.
kubectl create secret generic invoicekit-postgres \
  --from-literal=password='change-me-postgres'
kubectl create secret generic invoicekit-archive \
  --from-literal=root_user=invoicekit \
  --from-literal=root_password='change-me-minio'
kubectl create secret generic invoicekit-peppol-ap-cert \
  --from-file=cert.p12=/dev/null \
  --from-literal=cert.pass='change-me-peppol-cert-pass'

# 2. Install.
helm install ik deploy/helm/invoicekit

# 3. Probe.
kubectl get pods -l app.kubernetes.io/instance=ik
kubectl port-forward svc/ik-invoicekit-managed-api 8080:8080
curl http://127.0.0.1:8080/health
```

## Topology

Every service is rendered as its own `Deployment` / `StatefulSet` + `Service`:

| Component | Workload kind | Image (default) | Notes |
| --- | --- | --- | --- |
| `postgres` | `StatefulSet` (1 replica) | `postgres:16-alpine` | Swap for managed PostgreSQL in production. |
| `managed-api` | `Deployment` (2 replicas) | `ghcr.io/.../managed-api-server:scaffold` | HTTP API. Front with an Ingress for external traffic. |
| `signer-agent` | `DaemonSet` | `ghcr.io/.../signer-agent:scaffold` | Per-node Unix socket; keys never leave the node. |
| `validator-kosit` | `Deployment` (1 replica) | `ghcr.io/.../validator-kosit:scaffold` | JVM sidecar. |
| `validator-phive` | `Deployment` (1 replica) | `ghcr.io/.../validator-phive:scaffold` | JVM sidecar. |
| `validator-saxon` | `Deployment` (1 replica) | `ghcr.io/.../validator-saxon:scaffold` | JVM sidecar. |
| `validator-verapdf` | `Deployment` (1 replica) | `ghcr.io/.../validator-verapdf:scaffold` | JVM sidecar (T-058). |
| `validator-phase4` | `Deployment` (1 replica) | `ghcr.io/.../validator-phase4:scaffold` | AS4 sidecar (T-092); needs Peppol AP P12 secret. |
| `archive` | `StatefulSet` (1 replica) | `minio/minio:RELEASE.2025-04-22T22-12-26Z` | Swap for managed S3 bucket with Object Lock in production. |

The chart stamps standard `app.kubernetes.io/{name,instance,component,version,managed-by}` labels on every resource so an operator can filter by component (`-l app.kubernetes.io/component=validator-kosit`) or by release (`-l app.kubernetes.io/instance=ik`).

## Required secrets

| Secret | Keys | Used by |
| --- | --- | --- |
| `invoicekit-postgres` | `password` | `postgres`, `managed-api` |
| `invoicekit-archive` | `root_user`, `root_password` | `archive`, `managed-api` |
| `invoicekit-peppol-ap-cert` | `cert.p12`, `cert.pass` | `validator-phase4` |

Override the names via `.Values.postgres.auth.existingSecret`, `.Values.archive.existingSecret`, `.Values.validators.phase4.existingSecret`.

## Production cuts

Same checklist as `deploy/docker-compose.yml`:

1. Replace `postgres` + `archive` with managed equivalents.
2. Provision a real Peppol AP P12 certificate.
3. Enable `.Values.managedApi.ingress` with a TLS issuer (`cert-manager` annotations).
4. Pin every `:scaffold` tag to a versioned release (`global.imageTag`).
5. Set `imagePullSecrets` if the registry is private.
6. Tune `resources` per validator — JVM sidecars need >512Mi heap headroom for the larger national rule packs.

## Smoke test

After `helm install`:

```bash
release=ik
ns=default
for c in postgres managed-api signer-agent validator-kosit validator-phive validator-saxon validator-verapdf validator-phase4 archive; do
  if kubectl --namespace "$ns" get pods -l "app.kubernetes.io/instance=$release,app.kubernetes.io/component=$c" \
       --field-selector=status.phase=Running --no-headers 2>/dev/null | grep -q .; then
    echo "  $c: ok"
  else
    echo "  $c: NOT RUNNING"
  fi
done
```

CI integration (a `kind`-based test job that installs the chart + runs the smoke loop above) lands as the T-1301a follow-up.
