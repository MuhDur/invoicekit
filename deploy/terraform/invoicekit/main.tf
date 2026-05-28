# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

locals {
  common_tags = merge(
    var.tags,
    {
      "app.kubernetes.io/name"       = "invoicekit"
      "app.kubernetes.io/managed-by" = "terraform"
      "invoicekit.dev/module"        = "deploy/terraform/invoicekit"
    },
  )
}

# ---------------------------------------------------------------------------
# Database — managed RDS PostgreSQL, swap for the in-cluster Postgres the
# chart bundles for dev. Production tenancy requires Multi-AZ + automated
# backups; module exposes both via the rds_* variables.
# ---------------------------------------------------------------------------

resource "random_password" "postgres" {
  length           = 32
  special          = true
  override_special = "_%@!*"
}

resource "aws_db_subnet_group" "postgres" {
  name       = "${var.name}-postgres"
  subnet_ids = var.rds_subnet_ids
  tags       = local.common_tags
}

resource "aws_db_instance" "postgres" {
  identifier              = "${var.name}-postgres"
  engine                  = "postgres"
  engine_version          = "16"
  instance_class          = var.rds_instance_class
  allocated_storage       = var.rds_allocated_storage_gb
  storage_encrypted       = true
  db_name                 = "invoicekit"
  username                = "invoicekit"
  password                = random_password.postgres.result
  db_subnet_group_name    = aws_db_subnet_group.postgres.name
  vpc_security_group_ids  = var.rds_vpc_security_group_ids
  backup_retention_period = 7
  deletion_protection     = true
  skip_final_snapshot     = false
  final_snapshot_identifier = "${var.name}-postgres-final"
  tags = local.common_tags
}

# ---------------------------------------------------------------------------
# Archive — S3 bucket with Object Lock in Compliance mode. Replaces the
# in-cluster MinIO StatefulSet the chart bundles for dev.
# ---------------------------------------------------------------------------

resource "aws_s3_bucket" "archive" {
  bucket              = "${var.name}-archive"
  object_lock_enabled = true
  tags                = local.common_tags
}

resource "aws_s3_bucket_versioning" "archive" {
  bucket = aws_s3_bucket.archive.id
  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_object_lock_configuration" "archive" {
  bucket = aws_s3_bucket.archive.id
  rule {
    default_retention {
      mode = "COMPLIANCE"
      days = var.archive_bucket_object_lock_days
    }
  }
}

resource "aws_s3_bucket_server_side_encryption_configuration" "archive" {
  bucket = aws_s3_bucket.archive.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "AES256"
    }
  }
}

resource "aws_s3_bucket_public_access_block" "archive" {
  bucket                  = aws_s3_bucket.archive.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

# ---------------------------------------------------------------------------
# Secrets — pushed into Secrets Manager so the cluster's IAM role can
# pull them via the AWS Secrets and Configuration Provider (ASCP).
# ---------------------------------------------------------------------------

resource "aws_secretsmanager_secret" "postgres_password" {
  name = "${var.name}/postgres_password"
  tags = local.common_tags
}

resource "aws_secretsmanager_secret_version" "postgres_password" {
  secret_id     = aws_secretsmanager_secret.postgres_password.id
  secret_string = random_password.postgres.result
}

resource "aws_secretsmanager_secret" "peppol_ap_cert" {
  name = "${var.name}/peppol_ap_cert"
  tags = local.common_tags
}

resource "aws_secretsmanager_secret_version" "peppol_ap_cert" {
  secret_id = aws_secretsmanager_secret.peppol_ap_cert.id
  secret_string = jsonencode({
    cert_p12_base64 = var.peppol_ap_cert_p12_base64
    cert_pass       = var.peppol_ap_cert_pass
  })
}

# ---------------------------------------------------------------------------
# Cluster-side resources — namespace + the three Kubernetes secrets the
# chart references by name + the Helm release that installs the chart.
# Caller must wire kubernetes / helm providers against the target cluster.
# ---------------------------------------------------------------------------

resource "kubernetes_namespace" "this" {
  metadata {
    name   = var.kubernetes_namespace
    labels = local.common_tags
  }
}

resource "kubernetes_secret" "postgres" {
  metadata {
    name      = "invoicekit-postgres"
    namespace = kubernetes_namespace.this.metadata[0].name
    labels    = local.common_tags
  }
  data = {
    password = random_password.postgres.result
  }
  type = "Opaque"
}

resource "random_password" "minio_root" {
  length  = 32
  special = false
}

resource "kubernetes_secret" "archive" {
  metadata {
    name      = "invoicekit-archive"
    namespace = kubernetes_namespace.this.metadata[0].name
    labels    = local.common_tags
  }
  data = {
    root_user     = "invoicekit"
    root_password = random_password.minio_root.result
    s3_bucket     = aws_s3_bucket.archive.bucket
    s3_endpoint   = "https://s3.${var.region}.amazonaws.com"
  }
  type = "Opaque"
}

resource "kubernetes_secret" "peppol_ap_cert" {
  count = length(var.peppol_ap_cert_p12_base64) > 0 ? 1 : 0
  metadata {
    name      = "invoicekit-peppol-ap-cert"
    namespace = kubernetes_namespace.this.metadata[0].name
    labels    = local.common_tags
  }
  data = {
    "cert.p12"  = base64decode(var.peppol_ap_cert_p12_base64)
    "cert.pass" = var.peppol_ap_cert_pass
  }
  type = "Opaque"
}

resource "helm_release" "invoicekit" {
  name      = var.helm_release_name
  chart     = var.chart_path
  namespace = kubernetes_namespace.this.metadata[0].name
  version   = var.chart_version != "" ? var.chart_version : null

  values = [yamlencode({
    global = {
      imageTag = var.image_tag
    }
    postgres = {
      # The cluster-side bundled Postgres is disabled; the
      # chart wires the managed API directly to RDS via the
      # invoicekit-postgres secret that this module created.
      enabled = false
      auth = {
        existingSecret = kubernetes_secret.postgres.metadata[0].name
        username       = "invoicekit"
        database       = "invoicekit"
      }
    }
    archive = {
      # In-cluster MinIO disabled; chart points at S3 via
      # INVOICEKIT_ARCHIVE_S3_ENDPOINT.
      enabled        = false
      endpoint       = "https://${aws_s3_bucket.archive.bucket_regional_domain_name}"
      existingSecret = kubernetes_secret.archive.metadata[0].name
    }
    managedApi = {
      replicaCount = var.managed_api_replicas
      ingress = {
        enabled     = var.ingress_enabled
        className   = var.ingress_class_name
        annotations = var.ingress_annotations
        hosts = var.ingress_enabled ? [
          {
            host = var.ingress_host
            paths = [
              {
                path     = "/"
                pathType = "Prefix"
              },
            ]
          },
        ] : []
      }
    }
    validators = {
      phase4 = {
        existingSecret = length(kubernetes_secret.peppol_ap_cert) > 0 ? kubernetes_secret.peppol_ap_cert[0].metadata[0].name : "invoicekit-peppol-ap-cert"
      }
    }
  })]

  depends_on = [
    kubernetes_secret.postgres,
    kubernetes_secret.archive,
  ]
}
