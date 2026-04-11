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
  default     = "ap-guangzhou"
}

variable "availability_zone" {
  description = "Availability zone (must match region)"
  type        = string
  default     = "ap-guangzhou-3"
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
  description = "CVM instance type for client/test-driver (Machine A)"
  type        = string
  default     = "SA3.2XLARGE16" # 8C16G
}

variable "instance_type_meta" {
  description = "CVM instance type for metadata server (Machine B)"
  type        = string
  default     = "SA3.2XLARGE16" # 8C16G
}

variable "instance_type_data" {
  description = "CVM instance type for data server (Machine C)"
  type        = string
  default     = "SA3.XLARGE8" # 4C8G
}

# ---------- Disk sizes (GB) ----------

variable "data_disk_size_client" {
  description = "Data disk size in GB for Machine A"
  type        = number
  default     = 200
}

variable "data_disk_size_meta" {
  description = "Data disk size in GB for Machine B"
  type        = number
  default     = 200
}

variable "data_disk_size_data" {
  description = "Data disk size in GB for Machine C"
  type        = number
  default     = 500
}

# ---------- Image ----------

variable "image_id" {
  description = "Ubuntu 22.04 LTS image ID. Use `tccli cvm DescribeImages` to find it for your region."
  type        = string
  default     = "img-487zeit5" # Ubuntu Server 22.04 LTS 64bit (guangzhou)
}

# ---------- Network ----------

variable "vpc_cidr" {
  description = "VPC CIDR block"
  type        = string
  default     = "10.0.0.0/16"
}

variable "subnet_cidr" {
  description = "Subnet CIDR block"
  type        = string
  default     = "10.0.1.0/24"
}

# ---------- Naming ----------

variable "name_prefix" {
  description = "Prefix for all resource names"
  type        = string
  default     = "rucksfs-bench"
}
