use std::{fs::File, io::BufReader, path::Path, sync::Arc};
use tokio_rustls::rustls::{self, pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer}};
use tokio_rustls::rustls::ServerConfig as RustlsServerConfig;
use tonic::transport::ServerTlsConfig;

/// TLS configuration for the server
#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub cert_path: String,
    pub key_path: String,
}

impl TlsConfig {
    pub fn new(cert_path: impl Into<String>, key_path: impl Into<String>) -> Self {
        Self {
            cert_path: cert_path.into(),
            key_path: key_path.into(),
        }
    }

    /// Load TLS configuration from PEM files
    pub fn load_server_config(&self) -> Result<RustlsServerConfig, Box<dyn std::error::Error>> {
        let cert_file = File::open(&self.cert_path)?;
        let mut cert_reader = BufReader::new(cert_file);
        let certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut cert_reader)
            .collect::<Result<_, _>>()
            .map_err(|e| format!("Failed to load certificate: {}", e))?;

        let key_file = File::open(&self.key_path)?;
        let mut key_reader = BufReader::new(key_file);
        
        // Try different key formats
        let key = if let Some(key) = rustls_pemfile::private_key(&mut key_reader).map_err(|e| format!("Failed to load private key: {}", e))? {
            key
        } else {
            return Err("No private key found in file".into());
        };

        let config = RustlsServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;

        Ok(config)
    }

    /// Create tonic TLS config
    pub fn create_server_tls_config(&self) -> Result<ServerTlsConfig, Box<dyn std::error::Error>> {
        let rustls_config = self.load_server_config()?;
        Ok(ServerTlsConfig::new().with_rustls_server_config(rustls_config))
    }
}

/// TLS configuration for the client
#[derive(Debug, Clone)]
pub struct ClientTlsConfig {
    pub ca_cert_path: Option<String>,
    pub domain: Option<String>,
}

impl ClientTlsConfig {
    pub fn new() -> Self {
        Self {
            ca_cert_path: None,
            domain: None,
        }
    }

    pub fn with_ca_cert(mut self, path: impl Into<String>) -> Self {
        self.ca_cert_path = Some(path.into());
        self
    }

    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Load client TLS configuration
    pub fn load(&self) -> Result<Option<tonic::transport::ClientTlsConfig>, Box<dyn std::error::Error>> {
        if let Some(ca_path) = &self.ca_cert_path {
            let ca_file = File::open(ca_path)?;
            let mut ca_reader = BufReader::new(ca_file);
            let ca_certs: Vec<CertificateDer> = rustls_pemfile::certs(&mut ca_reader)
                .collect::<Result<_, _>>()
                .map_err(|e| format!("Failed to load CA certificate: {}", e))?;

            let mut builder = tonic::transport::ClientTlsConfig::new()
                .ca_certificate(tonic::transport::Certificate::from_pem(
                    ca_certs.iter().flat_map(|cert| cert.as_ref()).collect::<Vec<_>>()
                ));

            if let Some(domain) = &self.domain {
                builder = builder.domain_name(domain);
            }

            Ok(Some(builder))
        } else {
            // Use system trust store without CA validation
            Ok(Some(tonic::transport::ClientTlsConfig::new()))
        }
    }
}
