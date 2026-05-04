terraform {
  required_version = ">= 1.0"
  required_providers {
    tencentcloud = {
      source  = "tencentcloudstack/tencentcloud"
      version = "~> 1.81"
    }
  }
}

provider "tencentcloud" {
  secret_id  = var.secret_id
  secret_key = var.secret_key
  region     = var.region
}

# --- Variables ---

variable "secret_id" {
  type      = string
  sensitive = true
}

variable "secret_key" {
  type      = string
  sensitive = true
}

variable "ssh_key_ids" {
  type = list(string)
}

variable "region" {
  type    = string
  default = "ap-hongkong"
}

variable "availability_zone" {
  type    = string
  default = "ap-hongkong-2"
}

variable "image_id" {
  type = string
}

variable "instance_type" {
  type    = string
  default = "SA5.2XLARGE16" # 8C16G — same as Phase 1
}

variable "existing_vpc_id" {
  type = string
}

variable "existing_subnet_id" {
  type = string
}

variable "name_prefix" {
  type    = string
  default = "rucksfs-grpc"
}

# --- Security group: SSH + RPC ports inside VPC ---

resource "tencentcloud_security_group" "this" {
  name        = "${var.name_prefix}-sg"
  description = "rucksfs gRPC direct-press benchmark"
}

# SSH from anywhere
resource "tencentcloud_security_group_rule" "ssh" {
  security_group_id = tencentcloud_security_group.this.id
  type              = "ingress"
  cidr_ip           = "0.0.0.0/0"
  ip_protocol       = "tcp"
  port_range        = "22"
  policy            = "accept"
}

# All TCP within VPC (lazy but fine for an isolated bench VPC)
resource "tencentcloud_security_group_rule" "vpc_internal" {
  security_group_id = tencentcloud_security_group.this.id
  type              = "ingress"
  cidr_ip           = "10.0.0.0/16"
  ip_protocol       = "tcp"
  port_range        = "1-65535"
  policy            = "accept"
}

# Egress all
resource "tencentcloud_security_group_rule" "egress_all" {
  security_group_id = tencentcloud_security_group.this.id
  type              = "egress"
  cidr_ip           = "0.0.0.0/0"
  ip_protocol       = "tcp"
  port_range        = "1-65535"
  policy            = "accept"
}

# --- Server: MDS + DS co-located, like Phase 1 ---

resource "tencentcloud_instance" "server" {
  instance_name              = "${var.name_prefix}-server"
  availability_zone          = var.availability_zone
  image_id                   = var.image_id
  instance_type              = var.instance_type
  instance_charge_type       = "POSTPAID_BY_HOUR"
  vpc_id                     = var.existing_vpc_id
  subnet_id                  = var.existing_subnet_id
  allocate_public_ip         = true
  internet_max_bandwidth_out = 10
  orderly_security_groups    = [tencentcloud_security_group.this.id]
  key_ids                    = var.ssh_key_ids

  system_disk_type = "CLOUD_BSSD"
  system_disk_size = 50

  data_disks {
    data_disk_type = "CLOUD_SSD"
    data_disk_size = 200
  }

  # Same init as Phase 1 server (mounts /data, installs base tools)
  user_data = base64encode(file("${path.module}/init-server.sh"))

  tags = {
    billing = "shunjiecui"
    app     = "rucksfs-grpc-direct"
    role    = "server"
  }
}

# --- Client: bench driver ---

resource "tencentcloud_instance" "client" {
  instance_name              = "${var.name_prefix}-client"
  availability_zone          = var.availability_zone
  image_id                   = var.image_id
  instance_type              = var.instance_type
  instance_charge_type       = "POSTPAID_BY_HOUR"
  vpc_id                     = var.existing_vpc_id
  subnet_id                  = var.existing_subnet_id
  allocate_public_ip         = true
  internet_max_bandwidth_out = 10
  orderly_security_groups    = [tencentcloud_security_group.this.id]
  key_ids                    = var.ssh_key_ids

  system_disk_type = "CLOUD_BSSD"
  system_disk_size = 50

  # Match Phase 1 client exactly (same data-disk spec, even though bench doesn't use much)
  data_disks {
    data_disk_type = "CLOUD_SSD"
    data_disk_size = 200
  }

  user_data = base64encode(file("${path.module}/init-client.sh"))

  tags = {
    billing = "shunjiecui"
    app     = "rucksfs-grpc-direct"
    role    = "client"
  }
}

# --- Outputs ---

output "server_public_ip" {
  value = tencentcloud_instance.server.public_ip
}

output "server_private_ip" {
  value = tencentcloud_instance.server.private_ip
}

output "client_public_ip" {
  value = tencentcloud_instance.client.public_ip
}

output "client_private_ip" {
  value = tencentcloud_instance.client.private_ip
}
