#!/usr/bin/env bash
# setup-ssh-mesh.sh
# Runs locally. Generates a shared SSH keypair and pushes it to all client
# machines so that MPI rank spawning works (client-0 → client-1..5 via SSH).
set -uo pipefail

SSH_KEY="${1:?ssh_key_path}"
CLIENT_IPS="${2:?comma_separated_client_public_ips}"

SSH_OPTS="-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=10 -i $SSH_KEY"

IFS=',' read -r -a IPS <<< "$CLIENT_IPS"

# Step 1: generate a shared keypair locally (tempdir)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
ssh-keygen -t ed25519 -N "" -f "$TMP/mpi_key" -q
PUB=$(cat "$TMP/mpi_key.pub")

# Step 2: upload private+public key to every client and append pubkey to authorized_keys
for ip in "${IPS[@]}"; do
    echo "[ssh-mesh] setting up $ip"
    scp $SSH_OPTS "$TMP/mpi_key" "$TMP/mpi_key.pub" "ubuntu@$ip:/tmp/" >/dev/null
    ssh $SSH_OPTS "ubuntu@$ip" "
        mkdir -p ~/.ssh
        cp /tmp/mpi_key ~/.ssh/id_ed25519
        cp /tmp/mpi_key.pub ~/.ssh/id_ed25519.pub
        chmod 600 ~/.ssh/id_ed25519
        chmod 644 ~/.ssh/id_ed25519.pub
        grep -q \"$PUB\" ~/.ssh/authorized_keys 2>/dev/null || echo \"$PUB\" >> ~/.ssh/authorized_keys
        chmod 600 ~/.ssh/authorized_keys
        # turn off strict host checking for the internal subnet so mpirun can ssh into peers
        cat > ~/.ssh/config <<'SSHCONF'
Host 10.0.*
  StrictHostKeyChecking no
  UserKnownHostsFile /dev/null
  LogLevel ERROR
SSHCONF
        chmod 600 ~/.ssh/config
    "
done

# Step 3: verify: from client-0, ssh to every other client
FIRST_IP="${IPS[0]}"
PRIVATE_IPS_ARG="${3:?comma_separated_private_ips}"
IFS=',' read -r -a PRIV_IPS <<< "$PRIVATE_IPS_ARG"

echo "[ssh-mesh] verifying from ${FIRST_IP} (client-0) → all peers..."
for pip in "${PRIV_IPS[@]}"; do
    res=$(ssh $SSH_OPTS "ubuntu@$FIRST_IP" "ssh -o StrictHostKeyChecking=no -o ConnectTimeout=5 ubuntu@$pip hostname" 2>&1)
    echo "  → $pip: $res"
done
echo "[ssh-mesh] done"
