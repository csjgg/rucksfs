# ============================================================
# IP Addresses
# ============================================================

output "client1_public_ip" {
  description = "Machine A1 (client1) public IP — SSH target"
  value       = tencentcloud_instance.client1.public_ip
}

output "client1_private_ip" {
  description = "Machine A1 (client1) private IP"
  value       = tencentcloud_instance.client1.private_ip
}

output "client2_public_ip" {
  description = "Machine A2 (client2) public IP — SSH target"
  value       = tencentcloud_instance.client2.public_ip
}

output "client2_private_ip" {
  description = "Machine A2 (client2) private IP"
  value       = tencentcloud_instance.client2.private_ip
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
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.client1.public_ip}  # Machine A1 (client1)
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.client2.public_ip}  # Machine A2 (client2)
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.meta.public_ip}     # Machine B  (meta)
    ssh -i ~/.ssh/your-key ubuntu@${tencentcloud_instance.data.public_ip}     # Machine C  (data)

    # After cloud-init completes (~5-10 min), check status:
    #   ssh ubuntu@<ip> 'cloud-init status --wait'

    # Internal IPs (use in config files):
    #   Client1: ${tencentcloud_instance.client1.private_ip}
    #   Client2: ${tencentcloud_instance.client2.private_ip}
    #   Meta:    ${tencentcloud_instance.meta.private_ip}
    #   Data:    ${tencentcloud_instance.data.private_ip}

  EOT
}
