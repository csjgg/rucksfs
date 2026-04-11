# RucksFS Benchmark — Tencent Cloud Infrastructure

Terraform configuration to provision 3 CVM instances for RucksFS performance benchmarking.

## Architecture

```
Machine A (client)    Machine B (meta)       Machine C (data)
8C16G + 200G SSD      8C16G + 200G SSD       4C8G + 500G SSD
┌──────────────┐      ┌──────────────┐       ┌──────────────┐
│ mdtest       │─gRPC─│ RucksFS Meta │       │ RucksFS Data │
│ pjdfstest    │─SQL──│ MySQL 8.0    │       │ MinIO (S3)   │
│ FUSE mounts  │─TCP──│ Redis 7.x    │       │ NFS server   │
└──────────────┘      └──────────────┘       └──────────────┘
         All in same VPC / subnet / AZ
```

## Prerequisites

1. [Terraform >= 1.3](https://developer.hashicorp.com/terraform/install)
2. Tencent Cloud account with API credentials ([get SecretId/SecretKey](https://console.cloud.tencent.com/cam/capi))
3. SSH key pair created in Tencent Cloud console ([create here](https://console.cloud.tencent.com/cvm/sshkey))

## Quick Start

```bash
cd infra/tencent-bench

# 1. Configure
cp terraform.tfvars.example terraform.tfvars
# Edit terraform.tfvars: fill in secret_id, secret_key, ssh_key_ids

# 2. Initialize
terraform init

# 3. Review
terraform plan

# 4. Create (takes ~3-5 minutes)
terraform apply

# 5. Wait for cloud-init (~5-10 minutes after apply)
ssh -i ~/.ssh/your-key ubuntu@<client_public_ip> 'cloud-init status --wait'

# 6. Run benchmarks (see outputs for mount commands)
terraform output mount_commands

# 7. Destroy when done (IMPORTANT — stops billing!)
terraform destroy
```

## Outputs

After `terraform apply`, you'll see:

| Output | Description |
|--------|-------------|
| `client_public_ip` | SSH into Machine A |
| `meta_private_ip` | Use in MetadataServer/MySQL/Redis connection strings |
| `data_private_ip` | Use in DataServer/MinIO/NFS connection strings |
| `ssh_commands` | Ready-to-use SSH commands |
| `mount_commands` | Ready-to-use mount commands for all filesystems |

## Cloud-init Details

Each machine runs an initialization script on first boot:

| Machine | Script | Installs |
|---------|--------|----------|
| A (client) | `scripts/init-client.sh` | mdtest, pjdfstest, FUSE, JuiceFS, NFS client, Rust |
| B (meta) | `scripts/init-meta.sh` | MySQL 8.0, Redis 7.x |
| C (data) | `scripts/init-data.sh` | MinIO, NFS server |

Check init status: `cat /var/log/bench-init.log`

## Cost Estimate

~288 CNY for 3 days (POSTPAID_BY_HOUR in Guangzhou region).

**Remember to run `terraform destroy` when done!**

## Customization

Edit `terraform.tfvars` to change:
- Region/AZ (update `image_id` accordingly)
- Instance types (see [CVM specs](https://cloud.tencent.com/document/product/213/11518))
- Disk sizes

## Troubleshooting

```bash
# Check cloud-init progress
ssh ubuntu@<ip> 'tail -f /var/log/bench-init.log'

# Verify network between machines
ssh ubuntu@<client_ip> 'ping -c 5 <meta_private_ip>'
ssh ubuntu@<client_ip> 'iperf3 -c <meta_private_ip> -t 10'

# Verify services on Machine B
ssh ubuntu@<meta_ip> 'systemctl status mysql redis-server'

# Verify services on Machine C
ssh ubuntu@<data_ip> 'systemctl status minio nfs-kernel-server'
```
