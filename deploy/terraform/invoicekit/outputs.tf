# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

output "postgres_endpoint" {
  description = "DNS name of the RDS PostgreSQL instance."
  value       = aws_db_instance.postgres.endpoint
}

output "archive_bucket_name" {
  description = "Name of the Object-Lock-enabled S3 bucket used for evidence archive."
  value       = aws_s3_bucket.archive.bucket
}

output "archive_bucket_arn" {
  description = "ARN of the archive bucket (use to scope tenant IAM policies)."
  value       = aws_s3_bucket.archive.arn
}

output "secrets_manager_postgres_arn" {
  description = "ARN of the Secrets Manager entry holding the RDS password."
  value       = aws_secretsmanager_secret.postgres_password.arn
}

output "secrets_manager_peppol_arn" {
  description = "ARN of the Secrets Manager entry holding the AP PKCS#12 + passphrase."
  value       = aws_secretsmanager_secret.peppol_ap_cert.arn
}

output "kubernetes_namespace" {
  description = "Kubernetes namespace the chart is installed into."
  value       = kubernetes_namespace.this.metadata[0].name
}

output "helm_release_name" {
  description = "Helm release name (use to address resources via `helm`/`kubectl`)."
  value       = helm_release.invoicekit.name
}
