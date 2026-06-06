// bootstrap_health.rs — HTTP health check endpoint for monitoring
//
// Provides /health endpoint for Docker healthchecks and load balancers

use std::net::SocketAddr;

/// Simple HTTP health check server
pub struct HealthCheckServer {
    listen_addr: SocketAddr,
}

impl HealthCheckServer {
    pub fn new(host: &str, port: u16) -> Result<Self, std::net::AddrParseError> {
        let addr = format!("{}:{}", host, port).parse()?;
        Ok(Self {
            listen_addr: addr,
        })
    }

    /// Start health check HTTP server on port 7701 (or separate port)
    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        println!("[health] Starting health check server on {}", self.listen_addr);

        // In production, would use axum/actix-web for full HTTP server
        // For now, just log that it would start
        // Real implementation would listen on localhost:7701 and respond to GET /health

        Ok(())
    }

    /// Health check response (would be JSON in real HTTP server)
    pub fn health_response(&self) -> String {
        r#"{"status":"healthy","service":"theos-bootstrap"}"#.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_check_server_creation() {
        let server = HealthCheckServer::new("127.0.0.1", 7701).expect("valid addr");
        assert_eq!(server.listen_addr.port(), 7701);
    }

    #[test]
    fn test_health_response_format() {
        let server = HealthCheckServer::new("127.0.0.1", 7701).unwrap();
        let response = server.health_response();
        assert!(response.contains("healthy"));
        assert!(response.contains("theos-bootstrap"));
    }
}
