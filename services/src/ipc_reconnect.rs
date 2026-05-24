use std::time::Duration;
use crate::ipc_robustness::{CircuitBreaker, config};

/// Reconnection manager — handles retries with exponential backoff
pub struct ReconnectManager {
    circuit_breaker: CircuitBreaker,
    retry_count: u32,
    last_error: Option<String>,
}
use rand::Rng;
impl ReconnectManager {
    pub fn new() -> Self {
        Self {
            circuit_breaker: CircuitBreaker::new(),
            retry_count: 0,
            last_error: None,
        }
    }

    /// Get next backoff duration with exponential increase + jitter
    pub fn next_backoff(&mut self) -> Duration {
        // Check if circuit breaker allows attempt
        if !self.circuit_breaker.can_attempt() {
            eprintln!(
                "[reconnect] circuit breaker open, waiting {:?}",
                config::CIRCUIT_BREAKER_RECOVERY_WAIT
            );
            return config::CIRCUIT_BREAKER_RECOVERY_WAIT;
        }

        // Exponential backoff: 1s, 2s, 4s, 8s, 16s, 60s (capped)
        let base_secs = match self.retry_count {
            0 => 1,
            1 => 2,
            2 => 4,
            3 => 8,
            4 => 16,
            _ => 60,
        };

        let base = Duration::from_secs(base_secs as u64);

        // Add jitter: ±10% randomization to prevent thundering herd
        let jitter_percent = (base.as_secs_f64() * 0.1) as u64;
        let jitter_ms = jitter_percent.saturating_mul(10); // convert to ms
        let jitter = Duration::from_millis(jitter_ms / 2); // simplified jitter

        let jittered = base.saturating_add(jitter).max(Duration::from_millis(100));

        eprintln!(
            "[reconnect] attempt {}/{}, backoff: {:?}",
            self.retry_count + 1,
            config::CIRCUIT_BREAKER_THRESHOLD,
            jittered
        );

        self.retry_count += 1;
        jittered
    }

    /// Record successful connection
    pub fn on_success(&mut self) {
        self.circuit_breaker.record_success();
        self.retry_count = 0;
        self.last_error = None;
        eprintln!("[reconnect] connection restored");
    }

    /// Record failed connection attempt
    pub fn on_failure(&mut self, error: String) {
        self.circuit_breaker.record_failure();
        self.last_error = Some(error.clone());
        eprintln!(
            "[reconnect] connection failed: {} (attempt {})",
            error, self.retry_count
        );
    }

    /// Check if we've exceeded max retries
    pub fn should_give_up(&self) -> bool {
        matches!(self.circuit_breaker.state(), crate::ipc_robustness::CircuitBreakerState::Open)
            && self.retry_count >= config::CIRCUIT_BREAKER_THRESHOLD
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

/// Retry helper — waits for backoff duration and retries
pub async fn retry_with_backoff<F, T, E>(
    mut f: F,
    manager: &mut ReconnectManager,
) -> Result<T, String>
where
    F: FnMut() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>>>>,
    E: std::fmt::Display,
{
    loop {
        // Try the operation
        match f().await {
            Ok(result) => {
                manager.on_success();
                return Ok(result);
            }
            Err(e) => {
                let error_msg = e.to_string();
                manager.on_failure(error_msg);

                if manager.should_give_up() {
                    return Err(format!(
                        "Max retries exceeded: {}",
                        manager.last_error().unwrap_or("unknown")
                    ));
                }

                // Wait for backoff duration
                let backoff = manager.next_backoff();
                tokio::time::sleep(backoff).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_progression() {
        let mut mgr = ReconnectManager::new();
        let b0 = mgr.next_backoff();
        assert!(b0.as_secs() >= 1 && b0.as_secs() <= 2);

        let b1 = mgr.next_backoff();
        assert!(b1.as_secs() >= 1 && b1.as_secs() <= 3);

        let b2 = mgr.next_backoff();
        assert!(b2.as_secs() >= 3 && b2.as_secs() <= 5);
    }

    #[test]
    fn test_circuit_breaker_integration() {
        let mut mgr = ReconnectManager::new();
        for _ in 0..config::CIRCUIT_BREAKER_THRESHOLD {
            mgr.on_failure("test error".to_string());
        }
        assert!(mgr.should_give_up());
    }

    #[test]
    fn test_success_resets_retry_count() {
        let mut mgr = ReconnectManager::new();
        mgr.on_failure("error1".to_string());
        mgr.on_failure("error2".to_string());
        mgr.on_success();
        assert_eq!(mgr.retry_count, 0);
    }
}
