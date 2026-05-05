terraform {
  required_version = ">= 1.3"

  required_providers {
    tencentcloud = {
      source  = "tencentcloudstack/tencentcloud"
      version = ">= 1.81.0"
    }
  }
}

provider "tencentcloud" {
  secret_id  = var.secret_id
  secret_key = var.secret_key
  region     = var.region
}

# ============================================================
# Locals — resolve VPC/subnet ID (existing or newly created)
# ============================================================

locals {
  use_existing_vpc    = var.existing_vpc_id != ""
  use_existing_subnet = var.existing_subnet_id != ""

  vpc_id    = local.use_existing_vpc ? var.existing_vpc_id : tencentcloud_vpc.bench[0].id
  subnet_id = local.use_existing_subnet ? var.existing_subnet_id : tencentcloud_subnet.bench[0].id
}

# ============================================================
# VPC (only created when not reusing existing)
# ============================================================

resource "tencentcloud_vpc" "bench" {
  count        = local.use_existing_vpc ? 0 : 1
  name         = "${var.name_prefix}-vpc"
  cidr_block   = var.vpc_cidr
  is_multicast = false
}

resource "tencentcloud_subnet" "bench" {
  count             = local.use_existing_subnet ? 0 : 1
  name              = "${var.name_prefix}-subnet"
  vpc_id            = local.vpc_id
  cidr_block        = var.subnet_cidr
  availability_zone = var.availability_zone
  is_multicast      = false
}

# ============================================================
# Security Group — internal full access + external SSH only
# ============================================================

resource "tencentcloud_security_group" "bench" {
  name        = "${var.name_prefix}-sg"
  description = "RucksFS benchmark: internal full access, external SSH only"
  project_id  = var.project_id

  lifecycle {
    ignore_changes = [tags]
  }
}

resource "tencentcloud_security_group_rule_set" "bench" {
  security_group_id = tencentcloud_security_group.bench.id

  ingress {
    action      = "ACCEPT"
    cidr_block  = var.vpc_cidr
    protocol    = "ALL"
    port        = "ALL"
    description = "Allow all from VPC"
  }

  ingress {
    action      = "ACCEPT"
    cidr_block  = "0.0.0.0/0"
    protocol    = "TCP"
    port        = "22"
    description = "SSH"
  }

  ingress {
    action      = "ACCEPT"
    cidr_block  = "0.0.0.0/0"
    protocol    = "ICMP"
    port        = "ALL"
    description = "ICMP ping"
  }

  egress {
    action      = "ACCEPT"
    cidr_block  = "0.0.0.0/0"
    protocol    = "ALL"
    port        = "ALL"
    description = "Allow all outbound"
  }
}
