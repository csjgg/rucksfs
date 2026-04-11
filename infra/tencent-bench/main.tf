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
# VPC
# ============================================================

resource "tencentcloud_vpc" "bench" {
  name       = "${var.name_prefix}-vpc"
  cidr_block = var.vpc_cidr
}

resource "tencentcloud_subnet" "bench" {
  name              = "${var.name_prefix}-subnet"
  vpc_id            = tencentcloud_vpc.bench.id
  cidr_block        = var.subnet_cidr
  availability_zone = var.availability_zone
}

# ============================================================
# Security Group — internal full access + external SSH only
# ============================================================

resource "tencentcloud_security_group" "bench" {
  name        = "${var.name_prefix}-sg"
  description = "RucksFS benchmark: internal full access, external SSH only"
  project_id  = var.project_id
}

# Allow all traffic within the VPC CIDR (inbound)
resource "tencentcloud_security_group_lite_rule" "bench" {
  security_group_id = tencentcloud_security_group.bench.id

  ingress = [
    # Internal: allow all from VPC
    "ACCEPT#${var.vpc_cidr}#ALL#ALL",
    # External: SSH only
    "ACCEPT#0.0.0.0/0#22#TCP",
    # ICMP for ping
    "ACCEPT#0.0.0.0/0#ALL#ICMP",
  ]

  egress = [
    # Allow all outbound
    "ACCEPT#0.0.0.0/0#ALL#ALL",
  ]
}
