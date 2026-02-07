# Deployment Guide

This guide explains how to configure authentication and TLS for secure communication between the RucksFS server and client.

## Overview

The RucksFS RPC layer uses gRPC over TLS for secure communication. The server and client can be configured with:

- **Authentication**: Bearer token-based authentication
- **Encryption**: TLS 1.3 for encrypted connections
- **Certificate Verification**: Optional CA certificate validation

## Generating TLS Certificates

### Self-Signed Certificates (Development)

For development or testing, you can generate self-signed certificates:

```bash
# Generate CA private key
openssl genrsa -out ca.key 4096

# Generate CA certificate
openssl req -new -x509 -days 365 -key ca.key -out ca.crt -subj "/CN=RucksFS CA"

# Generate server private key
openssl genrsa -out server.key 4096

# Generate server CSR
openssl req -new -key server.key -out server.csr -subj "/CN=localhost"

# Sign server certificate with CA
openssl x509 -req -days 365 -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -out server.crt
```

### Production Certificates

For production, use certificates from a trusted CA (e.g., Let's Encrypt):

```bash
certbot certonly --standalone -d your-server.example.com
```

## Server Configuration

### Basic Server (Insecure - Development Only)

```bash
rucksfs-server --bind 127.0.0.1:50051
```

### Server with Authentication

```bash
rucksfs-server --bind 127.0.0.1:50051 --token "your-secret-api-token"
```

### Server with TLS

```bash
rucksfs-server --bind 0.0.0.0:50051 \
    --tls-cert /path/to/server.crt \
    --tls-key /path/to/server.key
```

### Secure Server (Production Recommended)

```bash
rucksfs-server --bind 0.0.0.0:50051 \
    --token "your-secret-api-token" \
    --tls-cert /etc/rucksfs/server.crt \
    --tls-key /etc/rucksfs/server.key
```

### Server Options

| Option | Description | Required |
|--------|-------------|----------|
| `--bind <addr>` | Bind address (e.g., `0.0.0.0:50051`) | Yes |
| `--token <token>` | API token for Bearer authentication | Recommended |
| `--tls-cert <path>` | Path to TLS certificate file | Optional* |
| `--tls-key <path>` | Path to TLS private key file | Optional* |

* Both `--tls-cert` and `--tls-key` must be provided together.

## Client Configuration

### Basic Client (Insecure)

```bash
rucksfs-client --server http://127.0.0.1:50051
```

### Client with Authentication

```bash
rucksfs-client --server http://127.0.0.1:50051 \
    --token "your-secret-api-token"
```

### Client with TLS

```bash
rucksfs-client --server https://server.example.com:50051
```

### Client with TLS and Authentication (Production Recommended)

```bash
rucksfs-client --server https://server.example.com:50051 \
    --token "your-secret-api-token" \
    --ca-cert /etc/rucksfs/ca.crt \
    --mount /mnt/rucksfs
```

### Client Options

| Option | Description | Required |
|--------|-------------|----------|
| `--server <addr>` | Server address (e.g., `https://server:50051`) | Yes |
| `--token <token>` | API token for Bearer authentication | Recommended |
| `--ca-cert <path>` | Path to CA certificate for verification | Optional |
| `--domain <name>` | Server domain name for TLS verification | Optional |
| `--mount <path>` | Mount point (Linux only) | Optional |

## Security Best Practices

### 1. Always Use TLS in Production

TLS encrypts all traffic between client and server, preventing:

- Data interception and eavesdropping
- Man-in-the-middle attacks
- Credential leakage

### 2. Use Strong Authentication Tokens

Generate strong, random tokens:

```bash
# Generate a secure random token
openssl rand -hex 32
```

### 3. Restrict Network Access

- Use firewall rules to limit access to trusted IPs
- Bind to specific interfaces (`127.0.0.1` for local, `0.0.0.0` for remote)
- Consider using VPNs for additional security

### 4. Secure Certificate Storage

- Set proper file permissions: `chmod 600 server.key`
- Store private keys in secure directories
- Rotate certificates before expiration

### 5. Use Environment Variables for Secrets

```bash
# Server
export RUCKSFS_TOKEN="your-secret-token"
rucksfs-server --bind 0.0.0.0:50051 --token "$RUCKSFS_TOKEN"

# Client
rucksfs-client --server https://server:50051 --token "$RUCKSFS_TOKEN"
```

## Troubleshooting

### Connection Refused

- Check if server is running
- Verify bind address and port
- Check firewall rules

### TLS Handshake Failed

- Verify certificate paths
- Ensure certificate is not expired
- Check certificate chain validity

### Authentication Failed

- Verify token matches server token
- Check for typos in token
- Ensure token is passed correctly

### Certificate Verification Failed

- Ensure CA certificate is provided to client
- Check domain name matches certificate CN/SAN
- Verify certificate is signed by trusted CA

## Example Deployment

### Production Setup with Docker

```yaml
# docker-compose.yml
version: '3.8'
services:
  rucksfs-server:
    image: rucksfs-server:latest
    ports:
      - "50051:50051"
    environment:
      - RUCKSFS_TOKEN=${TOKEN}
    volumes:
      - ./certs:/etc/rucksfs/certs:ro
    command:
      - --bind
      - 0.0.0.0:50051
      - --token
      - ${TOKEN}
      - --tls-cert
      - /etc/rucksfs/certs/server.crt
      - --tls-key
      - /etc/rucksfs/certs/server.key
```

```bash
# Generate .env file
echo "TOKEN=$(openssl rand -hex 32)" > .env

# Start server
docker-compose up -d
```

## Port Configuration

The default gRPC port is `50051`. You can use any available port:

```bash
# Server on custom port
rucksfs-server --bind 0.0.0.0:9100 --token "$TOKEN" --tls-cert cert.crt --tls-key cert.key

# Client connecting to custom port
rucksfs-client --server https://server.example.com:9100 --token "$TOKEN"
```

## Next Steps

- Monitor server logs for authentication failures
- Set up log aggregation for security auditing
- Consider implementing rate limiting for production
- Regularly rotate authentication tokens and certificates
