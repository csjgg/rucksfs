# ============================================================
# IP Addresses
# ============================================================

output "client_public_ip" {
  description = "Client public IP — SSH target"
  value       = tencentcloud_instance.client.public_ip
}

output "client_private_ip" {
  description = "Client private IP"
  value       = tencentcloud_instance.client.private_ip
}

output "server_jfs_public_ip" {
  description = "Server (JuiceFS+TiKV) public IP — SSH target"
  value       = tencentcloud_instance.server_jfs.public_ip
}

output "server_jfs_private_ip" {
  description = "Server (JuiceFS+TiKV) private IP"
  value       = tencentcloud_instance.server_jfs.private_ip
}

# ============================================================
# Quick-start commands
# ============================================================

output "ssh_commands" {
  description = "SSH commands to connect to each machine"
  value = <<-EOT

    # JuiceFS+Redis benchmark cluster:
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.client.public_ip}      # Client
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.server_jfs.public_ip}  # Server-JFS (Redis)

    # Internal IPs:
    #   Client:     ${tencentcloud_instance.client.private_ip}
    #   Server-JFS: ${tencentcloud_instance.server_jfs.private_ip}  (Redis)

  EOT
}
