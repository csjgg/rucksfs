use std::sync::Arc;
use tonic::{Request, Status, service::Interceptor};

/// API Token authentication for gRPC
#[derive(Debug, Clone)]
pub struct ApiToken {
    pub token: String,
}

impl ApiToken {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }

    pub fn validate(&self, provided: &str) -> bool {
        constant_time_eq::constant_time_eq(self.token.as_bytes(), provided.as_bytes())
    }
}

/// Create an authentication interceptor for Bearer token authentication
pub fn create_auth_interceptor(token: String) -> impl Interceptor {
    let token = Arc::new(token);
    
    move |req: Request<()>| {
        let token = token.clone();
        let auth_header = req
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok());

        match auth_header {
            Some(header) if header.starts_with("Bearer ") => {
                let provided_token = &header[7..];
                if constant_time_eq::constant_time_eq(provided_token.as_bytes(), token.as_bytes()) {
                    Ok(req)
                } else {
                    Err(Status::unauthenticated("Invalid API token"))
                }
            }
            _ => Err(Status::unauthenticated("Missing or invalid authorization header (expected: Bearer <token>)")),
        }
    }
}
