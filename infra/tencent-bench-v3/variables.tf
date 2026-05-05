variable "secret_id" {
  description = "Tencent Cloud API SecretId"
  type        = string
  sensitive   = true
}

variable "secret_key" {
  description = "Tencent Cloud API SecretKey"
  type        = string
  sensitive   = true
}

variable "region" {
  description = "Tencent Cloud region"
  type        = string
  default     = "ap-hongkong"
}

variable "availability_zone" {
  description = "Availability zone (must match region)"
  type        = string
  default     = "ap-hongkong-2"
}

variable "ssh_key_ids" {
  description = "List of SSH key IDs for CVM login (create in Tencent Cloud console first)"
  type        = list(string)
}

variable "project_id" {
  description = "Tencent Cloud project ID (0 = default project)"
  type        = number
  default     = 0
}

# ---------- Instance types ----------

variable "instance_type_client" {
  description = "CVM instance type for client/test-driver"
  type        = string
  default     = "SA3.2XLARGE16" # 8C16G
}

variable "instance_type_server" {
  description = "CVM instance type for both server-1 (RucksFS) and server-2 (NFS) — must be identical"
  type        = string
  default     = "SA3.2XLARGE16" # 8C16G
}

# ---------- Disk sizes (GB) ----------

variable "data_disk_size_client" {
  description = "Data disk size in GB for client"
  type        = number
  default     = 200
}

variable "data_disk_size_server" {
  description = "Data disk size in GB for both servers — must be identical"
  type        = number
  default     = 200
}

# ---------- Image ----------

variable "image_id" {
  description = "Ubuntu 22.04 LTS image ID. Use `tccli cvm DescribeImages` to find it for your region."
  type        = string
  default     = "img-487zeit5" # Ubuntu Server 22.04 LTS 64bit
}

# ---------- Network ----------

variable "existing_vpc_id" {
  description = "Existing VPC ID to reuse. Leave empty to create a new VPC."
  type        = string
  default     = ""
}

variable "existing_subnet_id" {
  description = "Existing subnet ID to reuse. Leave empty to create a new subnet."
  type        = string
  default     = ""
}

variable "vpc_cidr" {
  description = "VPC CIDR block (only used when creating new VPC)"
  type        = string
  default     = "10.0.0.0/16"
}

variable "subnet_cidr" {
  description = "Subnet CIDR block (only used when creating new subnet)"
  type        = string
  default     = "10.0.1.0/24"
}

# ---------- Naming ----------

variable "name_prefix" {
  description = "Prefix for all resource names"
  type        = string
  default     = "rucksfs-bench-v3"
}

# ---------- Round 3: multi-client topology ----------

variable "num_clients" {
  description = "Number of client/MPI-worker VMs to provision. bench-v3 uses 96 to cover N up to 96."
  type        = number
  default     = 96
}
