# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 The InvoiceKit Authors
#
# End-to-end example that provisions InvoiceKit into an existing
# AWS EKS cluster. Caller supplies VPC + subnet + security group
# ids; the example wires the module to RDS PostgreSQL + S3 +
# Secrets Manager.

terraform {
  required_version = ">= 1.6.0"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = ">= 5.50.0"
    }
    helm = {
      source  = "hashicorp/helm"
      version = ">= 2.13.0"
    }
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = ">= 2.30.0"
    }
  }
}

provider "aws" {
  region = var.region
}

data "aws_eks_cluster" "this" {
  name = var.eks_cluster_name
}

data "aws_eks_cluster_auth" "this" {
  name = var.eks_cluster_name
}

provider "kubernetes" {
  host                   = data.aws_eks_cluster.this.endpoint
  cluster_ca_certificate = base64decode(data.aws_eks_cluster.this.certificate_authority[0].data)
  token                  = data.aws_eks_cluster_auth.this.token
}

provider "helm" {
  kubernetes {
    host                   = data.aws_eks_cluster.this.endpoint
    cluster_ca_certificate = base64decode(data.aws_eks_cluster.this.certificate_authority[0].data)
    token                  = data.aws_eks_cluster_auth.this.token
  }
}

module "invoicekit" {
  source = "../../invoicekit"

  name   = "invoicekit-prod"
  region = var.region
  tags = {
    Environment = "prod"
    Owner       = "platform"
  }

  kubernetes_namespace = "invoicekit"

  rds_subnet_ids             = var.private_subnet_ids
  rds_vpc_security_group_ids = [var.rds_security_group_id]
  rds_instance_class         = "db.t4g.large"
  rds_allocated_storage_gb   = 100

  archive_bucket_object_lock_days = 3650

  peppol_ap_cert_p12_base64 = var.peppol_ap_cert_p12_base64
  peppol_ap_cert_pass       = var.peppol_ap_cert_pass

  managed_api_replicas = 3
  ingress_enabled      = true
  ingress_class_name   = "alb"
  ingress_host         = var.ingress_host
  ingress_annotations = {
    "alb.ingress.kubernetes.io/scheme"      = "internet-facing"
    "alb.ingress.kubernetes.io/target-type" = "ip"
    "alb.ingress.kubernetes.io/listen-ports" = "[{\"HTTPS\":443}]"
    "alb.ingress.kubernetes.io/ssl-redirect" = "443"
  }

  image_tag     = var.image_tag
  chart_path    = "../../../helm/invoicekit"
  chart_version = ""
}
