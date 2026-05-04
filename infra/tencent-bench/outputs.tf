# ============================================================
# IP Addresses — Round 3 multi-client topology
# ============================================================

output "client_public_ips" {
  description = "List of client public IPs (for orchestrator SSH targets)"
  value       = [for k in sort(keys(tencentcloud_instance.client)) : tencentcloud_instance.client[k].public_ip]
}

output "client_private_ips" {
  description = "List of client private IPs (for MPI hostfile)"
  value       = [for k in sort(keys(tencentcloud_instance.client)) : tencentcloud_instance.client[k].private_ip]
}

output "num_clients" {
  description = "Number of provisioned client machines"
  value       = var.num_clients
}

output "server_rucksfs_public_ip" {
  description = "Server (RucksFS / NFS / JuiceFS) public IP — SSH target"
  value       = tencentcloud_instance.server_rucksfs.public_ip
}

output "server_rucksfs_private_ip" {
  description = "Server (RucksFS / NFS / JuiceFS) private IP"
  value       = tencentcloud_instance.server_rucksfs.private_ip
}

# ============================================================
# Quick-start commands
# ============================================================

output "ssh_commands" {
  description = "SSH commands to connect to each machine"
  value = <<-EOT

    # Server (hosts every SUT under test, rotated by orchestrator):
    ssh -i shunjiecuitest.pem ubuntu@${tencentcloud_instance.server_rucksfs.public_ip}

    # Clients (${var.num_clients} total):
    ${join("\n    ", [for k in sort(keys(tencentcloud_instance.client)) : "ssh -i shunjiecuitest.pem ubuntu@${tencentcloud_instance.client[k].public_ip}  # client-${k}"])}

    # Internal (VPC) IPs:
      Server:  ${tencentcloud_instance.server_rucksfs.private_ip}
      Clients: ${join(", ", [for k in sort(keys(tencentcloud_instance.client)) : tencentcloud_instance.client[k].private_ip])}

  EOT
}
