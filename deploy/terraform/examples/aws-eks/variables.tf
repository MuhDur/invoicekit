# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors

variable "region" {
  description = "AWS region the EKS cluster lives in."
  type        = string
}

variable "eks_cluster_name" {
  description = "Existing EKS cluster name."
  type        = string
}

variable "private_subnet_ids" {
  description = "Private subnet IDs (must span at least two AZs) for the RDS subnet group."
  type        = list(string)
}

variable "rds_security_group_id" {
  description = "Security group ID attached to the RDS instance; must allow inbound 5432 from the EKS node security group."
  type        = string
}

variable "peppol_ap_cert_p12_base64" {
  description = "Base64-encoded OpenPeppol AP PKCS#12 bundle."
  type        = string
  sensitive   = true
}

variable "peppol_ap_cert_pass" {
  description = "Passphrase for the AP PKCS#12 bundle above."
  type        = string
  sensitive   = true
}

variable "ingress_host" {
  description = "Public hostname for the managed-api Ingress."
  type        = string
}

variable "image_tag" {
  description = "Image tag for every InvoiceKit-built container."
  type        = string
  default     = "scaffold"
}
