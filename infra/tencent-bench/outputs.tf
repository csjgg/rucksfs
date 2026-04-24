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

output "server_rucksfs_public_ip" {
  description = "Server (RucksFS) public IP — SSH target"
  value       = tencentcloud_instance.server_rucksfs.public_ip
}

output "server_rucksfs_private_ip" {
  description = "Server (RucksFS) private IP"
  value       = tencentcloud_instance.server_rucksfs.private_ip
}

# ============================================================
# Quick-start commands
# ============================================================

output "ssh_commands" {
  description = "SSH commands to connect to each machine"
  value = <<-EOT

    # RucksFS pjdfstest cluster:
    ssh -i shunjiecuitest.pem ubuntu@${tencentcloud_instance.client.public_ip}          # Client
    ssh -i shunjiecuitest.pem ubuntu@${tencentcloud_instance.server_rucksfs.public_ip}  # Server (RucksFS)

    # Internal IPs:
    #   Client:          ${tencentcloud_instance.client.private_ip}
    #   Server-RucksFS:  ${tencentcloud_instance.server_rucksfs.private_ip}

  EOT
}
