# ============================================================
# Machine A — Client / Test Driver (8C16G + 200GB SSD)
# ============================================================

resource "tencentcloud_instance" "client" {
  instance_name              = "${var.name_prefix}-client"
  availability_zone          = var.availability_zone
  image_id                   = var.image_id
  instance_type              = var.instance_type_client
  instance_charge_type       = "POSTPAID_BY_HOUR"
  project_id                 = var.project_id
  vpc_id                     = local.vpc_id
  subnet_id                  = local.subnet_id
  allocate_public_ip         = true
  internet_max_bandwidth_out = 10
  orderly_security_groups    = [tencentcloud_security_group.bench.id]
  key_ids                    = var.ssh_key_ids

  system_disk_type = "CLOUD_BSSD"
  system_disk_size = 50

  data_disks {
    data_disk_type = "CLOUD_SSD"
    data_disk_size = var.data_disk_size_client
  }

  user_data = base64encode(file("${path.module}/scripts/init-client.sh"))

  tags = {
    billing = "shunjiecui"
    app     = "rucksfs-bench"
    role    = "client"
  }
}

# ============================================================
# Server — RucksFS MetadataServer + DataServer (8C16G + 200GB SSD)
# Both services co-located on one machine (same as benchmark setup).
# ============================================================

resource "tencentcloud_instance" "server_rucksfs" {
  instance_name              = "${var.name_prefix}-server-rucksfs"
  availability_zone          = var.availability_zone
  image_id                   = var.image_id
  instance_type              = var.instance_type_server
  instance_charge_type       = "POSTPAID_BY_HOUR"
  project_id                 = var.project_id
  vpc_id                     = local.vpc_id
  subnet_id                  = local.subnet_id
  allocate_public_ip         = true
  internet_max_bandwidth_out = 10
  orderly_security_groups    = [tencentcloud_security_group.bench.id]
  key_ids                    = var.ssh_key_ids

  system_disk_type = "CLOUD_BSSD"
  system_disk_size = 50

  data_disks {
    data_disk_type = "CLOUD_SSD"
    data_disk_size = var.data_disk_size_server
  }

  user_data = base64encode(file("${path.module}/scripts/init-server-rucksfs.sh"))

  tags = {
    billing = "shunjiecui"
    app     = "rucksfs-bench"
    role    = "server-rucksfs"
  }
}
