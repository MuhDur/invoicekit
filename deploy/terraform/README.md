# deploy/terraform — managed-cloud provisioning

Terraform module + AWS example. Owned end-to-end by bead **T-1302**.

The module `invoicekit/` provisions the cloud-side of an InvoiceKit deployment and hands it off to the `deploy/helm/invoicekit` chart for the cluster-side install. Same `values.yaml` shape as the standalone Helm chart, but with `postgres.enabled = false`, `archive.enabled = false`, and the secrets pre-populated by Secrets Manager / RDS.

## Topology

```
                 (operator runs `terraform apply`)
                                │
        ┌───────────────────────┼───────────────────────┐
        ▼                       ▼                       ▼
  RDS PostgreSQL          S3 (Object Lock)        Secrets Manager
  (managed DB)            (archive bucket)        (postgres + AP cert)
        │                       │                       │
        └───────────┬───────────┴───────────┬───────────┘
                    ▼                       ▼
            Kubernetes Secrets       helm_release.invoicekit
            (synced into ns)         (chart from deploy/helm)
                    │                       │
                    └───────┬───────────────┘
                            ▼
              InvoiceKit pods (managed-api + sidecars)
              talking to RDS / S3 over the cluster's VPC.
```

## What the module does

- Provisions an **RDS PostgreSQL 16** instance (managed subnet group, encryption at rest, deletion protection, 7-day backup retention).
- Provisions an **S3 bucket with Object Lock Compliance mode** for the evidence archive (`object_lock_enabled = true`, SSE-S3, public-access blocked).
- Stores the random RDS password + the Peppol AP P12 bundle + passphrase in **AWS Secrets Manager**.
- Creates the target **Kubernetes namespace** + the three secrets (`invoicekit-postgres`, `invoicekit-archive`, `invoicekit-peppol-ap-cert`) the Helm chart references.
- **Installs the Helm chart** with `postgres.enabled=false` + `archive.enabled=false` so the chart points at the cloud-managed equivalents instead of bundling its own.

## Provider wiring

The module declares `aws`, `kubernetes`, `helm`, and `random` providers — the **caller wires their actual provider configuration** (e.g. against an existing EKS cluster). See `examples/aws-eks/` for the canonical wiring.

## Inputs

The full input contract lives in `invoicekit/variables.tf`. Required:

- `region` — AWS region.
- `rds_subnet_ids` — private subnet IDs across at least two AZs.
- `rds_vpc_security_group_ids` — security groups allowing inbound 5432 from the cluster's node SG.

Optional but recommended for production:

- `peppol_ap_cert_p12_base64` + `peppol_ap_cert_pass` — OpenPeppol AP credentials.
- `ingress_enabled = true` with `ingress_class_name`, `ingress_host`, `ingress_annotations` for the managed-api ALB / Ingress.
- `image_tag = "v0.X.Y"` — pin to a release rather than the default `scaffold` tag.
- `chart_version` — pin the chart too.

## Example

```bash
cd deploy/terraform/examples/aws-eks
terraform init
terraform plan \
  -var region=eu-central-1 \
  -var eks_cluster_name=my-cluster \
  -var 'private_subnet_ids=["subnet-aaa","subnet-bbb"]' \
  -var rds_security_group_id=sg-12345 \
  -var ingress_host=invoicekit.example.com \
  -var peppol_ap_cert_p12_base64="$(base64 -w0 < ap-cert.p12)" \
  -var peppol_ap_cert_pass='change-me'
```

After `terraform apply`, the outputs surface the RDS endpoint, archive bucket ARN, and the ingress host. The bundled `helm_release.invoicekit` resource brings the pods up; verify with the smoke loop documented in `deploy/helm/README.md` §Smoke test.

## Production cuts

1. Use a remote backend (`backend "s3"`) with `dynamodb_table` lock + `encrypt = true`.
2. Replace the auto-generated RDS password with a managed rotation policy.
3. Wire AWS Backup against the RDS instance for cross-region snapshots.
4. Add a CloudWatch alarm on the RDS `CPUUtilization` + `FreeStorageSpace` metrics.
5. Add an S3 Inventory + bucket-level metrics to track archive growth.
6. Front the managed-api Ingress with WAF if your TLS-terminator (ALB) supports it.

CI workflow that runs `terraform fmt -check`, `terraform validate`, and `terraform plan` against a dummy backend is filed as the T-1302a follow-up.
