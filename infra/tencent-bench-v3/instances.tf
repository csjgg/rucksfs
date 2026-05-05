# ============================================================
# Machine A — Client / Test Driver fleet (2C2G)
# Round 3+: scaled to var.num_clients for multi-node MPI.
# ============================================================

resource "tencentcloud_instance" "client" {
  for_each = toset([for i in range(var.num_clients) : tostring(i)])

  instance_name              = "${var.name_prefix}-client-${each.key}"
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

  # No data disk — clients only need binaries and mount point

  user_data = base64encode(file("${path.module}/scripts/init-client.sh"))

  tags = {
    billing = "shunjiecui"
    app     = "rucksfs-bench"
    role    = "client"
  }
}

# ============================================================
# Server — RucksFS MetadataServer + DataServer (+ NFS + TiKV)
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
  system_disk_size = 200

  # No separate data disk — use enlarged system disk for /data

  user_data = base64encode(file("${path.module}/scripts/init-server-rucksfs.sh"))

  tags = {
    billing = "shunjiecui"
    app     = "rucksfs-bench"
    role    = "server-rucksfs"
  }
}

# ============================================================
# Bench driver — for gRPC direct pressure test (not a mdtest client)
# 8C16G, matches historical appendix A config (SA5.2XLARGE16)
# ============================================================

resource "tencentcloud_instance" "bench_driver" {
  instance_name              = "${var.name_prefix}-bench-driver"
  availability_zone          = var.availability_zone
  image_id                   = var.image_id
  instance_type              = "SA5.2XLARGE16"  # 8C16G
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

  # No cloud-init — we manually scp the rucksfs-bench binary
  user_data = base64encode("#!/bin/bash\necho 'bench-driver ready'\n")

  tags = {
    billing = "shunjiecui"
    app     = "rucksfs-bench"
    role    = "bench-driver"
  }
}
