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

output "server1_public_ip" {
  description = "Server-1 (RucksFS) public IP — SSH target"
  value       = tencentcloud_instance.server1.public_ip
}

output "server1_private_ip" {
  description = "Server-1 (RucksFS) private IP"
  value       = tencentcloud_instance.server1.private_ip
}

output "server2_public_ip" {
  description = "Server-2 (NFS) public IP — SSH target"
  value       = tencentcloud_instance.server2.public_ip
}

output "server2_private_ip" {
  description = "Server-2 (NFS) private IP"
  value       = tencentcloud_instance.server2.private_ip
}

# ============================================================
# Quick-start commands
# ============================================================

output "ssh_commands" {
  description = "SSH commands to connect to each machine"
  value = <<-EOT

    # Symmetric 2-server benchmark cluster:
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.client.public_ip}   # Client
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.server1.public_ip}  # Server-1 (RucksFS MDS+DS)
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.server2.public_ip}  # Server-2 (NFS)

    # Internal IPs:
    #   Client:   ${tencentcloud_instance.client.private_ip}
    #   Server-1: ${tencentcloud_instance.server1.private_ip}  (RucksFS)
    #   Server-2: ${tencentcloud_instance.server2.private_ip}  (NFS)

  EOT
}
