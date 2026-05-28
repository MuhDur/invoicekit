# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

output "postgres_endpoint" {
  description = "DNS endpoint of the RDS PostgreSQL instance."
  value       = module.invoicekit.postgres_endpoint
}

output "archive_bucket_arn" {
  description = "ARN of the evidence archive bucket."
  value       = module.invoicekit.archive_bucket_arn
}

output "ingress_host" {
  description = "Hostname the managed-api Ingress responds on."
  value       = var.ingress_host
}
