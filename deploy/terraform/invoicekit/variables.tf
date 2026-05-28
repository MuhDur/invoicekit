# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

variable "name" {
  description = "Logical name prefix stamped on every provisioned resource."
  type        = string
  default     = "invoicekit"
}

variable "region" {
  description = "AWS region to provision into."
  type        = string
}

variable "tags" {
  description = "Tags applied to every AWS resource the module creates."
  type        = map(string)
  default     = {}
}

variable "kubernetes_namespace" {
  description = "Kubernetes namespace the chart is installed into. Module creates the namespace if it does not exist."
  type        = string
  default     = "invoicekit"
}

variable "helm_release_name" {
  description = "Helm release name."
  type        = string
  default     = "invoicekit"
}

variable "chart_path" {
  description = "Path or URL to the InvoiceKit Helm chart. Default points at the repository checkout under deploy/helm/invoicekit."
  type        = string
  default     = "../../helm/invoicekit"
}

variable "chart_version" {
  description = "Pinned chart version. Required for production; defaults pull from chart_path."
  type        = string
  default     = ""
}

variable "image_tag" {
  description = "Image tag for every InvoiceKit-built container."
  type        = string
  default     = "scaffold"
}

variable "rds_instance_class" {
  description = "RDS PostgreSQL instance class. Defaults to db.t4g.micro for dev; bump for production."
  type        = string
  default     = "db.t4g.micro"
}

variable "rds_allocated_storage_gb" {
  description = "Allocated storage for the RDS instance, in gigabytes."
  type        = number
  default     = 20
}

variable "rds_subnet_ids" {
  description = "Subnet IDs the RDS subnet group spans. Must be private subnets in at least two AZs."
  type        = list(string)
}

variable "rds_vpc_security_group_ids" {
  description = "Security group IDs attached to the RDS instance. Must allow inbound 5432 from the cluster's node SG."
  type        = list(string)
}

variable "archive_bucket_object_lock_days" {
  description = "Object Lock Compliance-mode retention period applied to every object written to the archive bucket."
  type        = number
  default     = 3650
}

variable "peppol_ap_cert_p12_base64" {
  description = "Base64-encoded OpenPeppol AP PKCS#12 bundle. Pass an empty string for dev installs that don't transmit live."
  type        = string
  default     = ""
  sensitive   = true
}

variable "peppol_ap_cert_pass" {
  description = "Passphrase for the AP PKCS#12 bundle above."
  type        = string
  default     = ""
  sensitive   = true
}

variable "managed_api_replicas" {
  description = "Replica count for the managed-api Deployment."
  type        = number
  default     = 2
}

variable "ingress_enabled" {
  description = "Render the managed-api Ingress."
  type        = bool
  default     = false
}

variable "ingress_class_name" {
  description = "Ingress class name (e.g. `alb`, `nginx`)."
  type        = string
  default     = ""
}

variable "ingress_host" {
  description = "Hostname for the Ingress rule."
  type        = string
  default     = ""
}

variable "ingress_annotations" {
  description = "Ingress annotations (typically cert-manager + ALB-controller key/values)."
  type        = map(string)
  default     = {}
}
