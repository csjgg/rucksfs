# ============================================================
# IP Addresses
# ============================================================

output "client_public_ip" {
  description = "Machine A (client) public IP — SSH target"
  value       = tencentcloud_instance.client.public_ip
}

output "client_private_ip" {
  description = "Machine A (client) private IP"
  value       = tencentcloud_instance.client.private_ip
}

output "meta_public_ip" {
  description = "Machine B (metadata) public IP — SSH target"
  value       = tencentcloud_instance.meta.public_ip
}

output "meta_private_ip" {
  description = "Machine B (metadata) private IP — use this in mount/connect commands"
  value       = tencentcloud_instance.meta.private_ip
}

output "data_public_ip" {
  description = "Machine C (data) public IP — SSH target"
  value       = tencentcloud_instance.data.public_ip
}

output "data_private_ip" {
  description = "Machine C (data) private IP — use this in mount/connect commands"
  value       = tencentcloud_instance.data.private_ip
}

# ============================================================
# Quick-start commands
# ============================================================

output "ssh_commands" {
  description = "SSH commands to connect to each machine"
  value = <<-EOT

    # SSH into machines:
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.client.public_ip}   # Machine A (client)
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.meta.public_ip}     # Machine B (meta)
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.data.public_ip}     # Machine C (data)

    # After cloud-init completes (~5-10 min), check status:
    #   ssh ubuntu@<ip> 'cloud-init status --wait'

    # Internal IPs (use in config files):
    #   Meta:  ${tencentcloud_instance.meta.private_ip}
    #   Data:  ${tencentcloud_instance.data.private_ip}

  EOT
}

output "mount_commands" {
  description = "Example mount commands to run on Machine A (client)"
  value = <<-EOT

    # === On Machine A (client) ===

    # 1. RucksFS embedded (local, no network)
    sudo mkdir -p /mnt/rucksfs-embedded
    sudo ./rucksfs --mount /mnt/rucksfs-embedded --data-dir /data/rucksfs-local

    # 2. RucksFS distributed
    sudo mkdir -p /mnt/rucksfs-dist
    sudo ./rucksfs-remote-client \
        --mount /mnt/rucksfs-dist \
        --meta-addr http://${tencentcloud_instance.meta.private_ip}:8001 \
        --data-addr http://${tencentcloud_instance.data.private_ip}:8002

    # 3. JuiceFS + MySQL
    sudo mkdir -p /mnt/juicefs-mysql
    juicefs format \
        --storage minio \
        --bucket http://${tencentcloud_instance.data.private_ip}:9000/jfs-mysql \
        --access-key minioadmin --secret-key minioadmin \
        "mysql://juicefs:juicefs_bench@(${tencentcloud_instance.meta.private_ip}:3306)/juicefs" \
        jfs-mysql
    sudo juicefs mount "mysql://juicefs:juicefs_bench@(${tencentcloud_instance.meta.private_ip}:3306)/juicefs" /mnt/juicefs-mysql -d

    # 4. JuiceFS + Redis
    sudo mkdir -p /mnt/juicefs-redis
    juicefs format \
        --storage minio \
        --bucket http://${tencentcloud_instance.data.private_ip}:9000/jfs-redis \
        --access-key minioadmin --secret-key minioadmin \
        "redis://${tencentcloud_instance.meta.private_ip}:6379/1" \
        jfs-redis
    sudo juicefs mount "redis://${tencentcloud_instance.meta.private_ip}:6379/1" /mnt/juicefs-redis -d

    # 5. NFS
    sudo mkdir -p /mnt/nfs
    sudo mount -t nfs ${tencentcloud_instance.data.private_ip}:/data/nfs-export /mnt/nfs

    # 6. ext4 local (already available)
    mkdir -p /data/ext4-bench

  EOT
}
