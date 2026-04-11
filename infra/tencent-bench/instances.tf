# ============================================================
# Machine A1 — Client / Test Driver 1 (8C16G + 200GB SSD)
# ============================================================

resource "tencentcloud_instance" "client1" {
  instance_name              = "${var.name_prefix}-client1"
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
    role    = "client1"
  }
}

# ============================================================
# Machine A2 — Client / Test Driver 2 (8C16G + 200GB SSD)
# ============================================================

resource "tencentcloud_instance" "client2" {
  instance_name              = "${var.name_prefix}-client2"
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
    role    = "client2"
  }
}

# ============================================================
# Machine B — Metadata Server (8C32G + 200GB SSD)
# ============================================================

resource "tencentcloud_instance" "meta" {
  instance_name              = "${var.name_prefix}-meta"
  availability_zone          = var.availability_zone
  image_id                   = var.image_id
  instance_type              = var.instance_type_meta
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
    data_disk_size = var.data_disk_size_meta
  }

  user_data = base64encode(file("${path.module}/scripts/init-meta.sh"))

  tags = {
    billing = "shunjiecui"
    app     = "rucksfs-bench"
    role    = "metadata"
  }
}

# ============================================================
# Machine C — Data Server (4C8G + 500GB SSD)
# ============================================================

resource "tencentcloud_instance" "data" {
  instance_name              = "${var.name_prefix}-data"
  availability_zone          = var.availability_zone
  image_id                   = var.image_id
  instance_type              = var.instance_type_data
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
    data_disk_size = var.data_disk_size_data
  }

  user_data = base64encode(file("${path.module}/scripts/init-data.sh"))

  tags = {
    billing = "shunjiecui"
    app     = "rucksfs-bench"
    role    = "data"
  }
}
