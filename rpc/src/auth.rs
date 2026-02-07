use std::sync::Arc;
use tonic::Request;
use tower_http::validate_request::ValidateRequestHeaderLayer;

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

/// Create a middleware layer for API token authentication
pub fn auth_layer(token: Arc<String>) -> ValidateRequestHeaderLayer<impl Fn(&Request<()>) -> Result<(), tonic::Status> + Clone> {
    ValidateRequestHeaderLayer::custom(move |req: &Request<()>| {
        let auth_header = req
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok());

        match auth_header {
            Some(header) if header.starts_with("Bearer ") => {
                let token = &header[7..];
                if constant_time_eq::constant_time_eq(token.as_bytes(), token.as_bytes()) {
                    Ok(())
                } else {
                    Err(tonic::Status::unauthenticated("Invalid API token"))
                }
            }
            _ => Err(tonic::Status::unauthenticated("Missing or invalid authorization header")),
        }
    })
}

pub fn create_auth_layer(token: String) -> ValidateRequestHeaderLayer<impl Fn(&Request<()>) -> Result<(), tonic::Status> + Clone> {
    let token = Arc::new(token);
    ValidateRequestHeaderLayer::custom(move |req: &Request<()>| {
        let auth_header = req
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok());

        match auth_header {
            Some(header) if header.starts_with("Bearer ") => {
                let provided_token = &header[7..];
                if constant_time_eq::constant_time_eq(provided_token.as_bytes(), token.as_bytes()) {
                    Ok(())
                } else {
                    Err(tonic::Status::unauthenticated("Invalid API token"))
                }
            }
            _ => Err(tonic::Status::unauthenticated("Missing or invalid authorization header (expected: Bearer <token>)")),
        }
    })
}
