# ============================================================
# IP Addresses
# ============================================================

output "client_public_ip" {
  description = "Machine A (client) public IP — SSH target"
  value       = tencentcloud_instance.client1.public_ip
}

output "client_private_ip" {
  description = "Machine A (client) private IP"
  value       = tencentcloud_instance.client1.private_ip
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

    # SSH into machines (3-node controlled benchmark cluster):
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.client1.public_ip}  # Machine A (client)
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.meta.public_ip}     # Machine B (meta)
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.data.public_ip}     # Machine C (data/nfs)

    # After cloud-init completes (~5-10 min), check status:
    #   ssh ubuntu@<ip> 'cloud-init status --wait'

    # Internal IPs (use in config files):
    #   Client: ${tencentcloud_instance.client1.private_ip}
    #   Meta:   ${tencentcloud_instance.meta.private_ip}
    #   Data:   ${tencentcloud_instance.data.private_ip}

  EOT
}
