# deploy/

Deployment artifacts. Not a Cargo workspace member.

- `docker-compose.yml` — single-host managed-stack deployment, owned end-to-end by bead **T-1300**. Layout per `plans/PLAN.md` §4.1. Service blocks declare the bead that owns each component so operators can trace any production service back to its specification.
- `helm/` — Kubernetes Helm chart, owned by bead **T-1301**.
- `terraform/` — Terraform module for managed cloud provisioning, owned by bead **T-1302**.

Scaffolded by bead **invoices-t-001-cargo-workspace-xos**; production-ready content lands with the owning beads above.
