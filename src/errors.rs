// Error Injection Module
// Simulates various API error conditions for testing error handling.

use crate::openai::ErrorResponse;
use rand::Rng;
use std::time::Duration;

/// Configuration for error injection
#[derive(Debug, Clone)]
pub struct ErrorConfig {
    /// Probability of rate limit error (0.0-1.0)
    pub rate_limit_rate: f64,
    /// Probability of server error (0.0-1.0)
    pub server_error_rate: f64,
    /// Probability of timeout (0.0-1.0)
    pub timeout_rate: f64,
    /// Delay before timeout error (simulates slow response before failure)
    pub timeout_after_ms: u64,
    /// Probability of invalid request error (0.0-1.0)
    pub invalid_request_rate: f64,
    /// Probability of authentication error (0.0-1.0)
    pub auth_error_rate: f64,
}

impl ErrorConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a config with no errors (for normal operation)
    pub fn none() -> Self {
        Self {
            rate_limit_rate: 0.0,
            server_error_rate: 0.0,
            timeout_rate: 0.0,
            timeout_after_ms: 30000,
            invalid_request_rate: 0.0,
            auth_error_rate: 0.0,
        }
    }

    /// Create a config for chaos testing (high error rates)
    pub fn chaos() -> Self {
        Self {
            rate_limit_rate: 0.1,
            server_error_rate: 0.05,
            timeout_rate: 0.05,
            timeout_after_ms: 5000,
            invalid_request_rate: 0.02,
            auth_error_rate: 0.01,
        }
    }

    /// Create a config that simulates rate limiting
    pub fn rate_limited() -> Self {
        Self {
            rate_limit_rate: 0.5, // 50% chance of rate limit
            server_error_rate: 0.0,
            timeout_rate: 0.0,
            timeout_after_ms: 30000,
            invalid_request_rate: 0.0,
            auth_error_rate: 0.0,
        }
    }

    /// Builder pattern methods
    pub fn with_rate_limit_rate(mut self, rate: f64) -> Self {
        self.rate_limit_rate = rate.clamp(0.0, 1.0);
        self
    }

    pub fn with_server_error_rate(mut self, rate: f64) -> Self {
        self.server_error_rate = rate.clamp(0.0, 1.0);
        self
    }

    pub fn with_timeout_rate(mut self, rate: f64) -> Self {
        self.timeout_rate = rate.clamp(0.0, 1.0);
        self
    }

    pub fn with_timeout_after_ms(mut self, ms: u64) -> Self {
        self.timeout_after_ms = ms;
        self
    }

    /// Get the total probability of any error occurring
    pub fn total_error_rate(&self) -> f64 {
        (self.rate_limit_rate
            + self.server_error_rate
            + self.timeout_rate
            + self.invalid_request_rate
            + self.auth_error_rate)
            .min(1.0)
    }
}

impl Default for ErrorConfig {
    fn default() -> Self {
        Self {
            rate_limit_rate: 0.0,
            server_error_rate: 0.0,
            timeout_rate: 0.0,
            timeout_after_ms: 30000,
            invalid_request_rate: 0.0,
            auth_error_rate: 0.0,
        }
    }
}

/// Types of simulated errors
#[derive(Debug, Clone, PartialEq)]
pub enum SimulatedError {
    /// Rate limit exceeded (HTTP 429)
    RateLimit { retry_after_seconds: u32 },
    /// Internal server error (HTTP 500)
    ServerError,
    /// Service unavailable (HTTP 503)
    ServiceUnavailable,
    /// Request timeout
    Timeout { after: Duration },
    /// Invalid request (HTTP 400)
    InvalidRequest { message: String },
    /// Authentication error (HTTP 401)
    AuthenticationError,
}

impl SimulatedError {
    /// Get the HTTP status code for this error
    pub fn status_code(&self) -> u16 {
        match self {
            SimulatedError::RateLimit { .. } => 429,
            SimulatedError::ServerError => 500,
            SimulatedError::ServiceUnavailable => 503,
            SimulatedError::Timeout { .. } => 504,
            SimulatedError::InvalidRequest { .. } => 400,
            SimulatedError::AuthenticationError => 401,
        }
    }

    /// Convert to OpenAI-style error response
    pub fn to_error_response(&self) -> ErrorResponse {
        match self {
            SimulatedError::RateLimit { .. } => ErrorResponse::rate_limit(),
            SimulatedError::ServerError => ErrorResponse::server_error(),
            SimulatedError::ServiceUnavailable => {
                ErrorResponse::new("Service temporarily unavailable", "service_unavailable")
            }
            SimulatedError::Timeout { .. } => {
                ErrorResponse::new("Request timed out", "timeout_error")
            }
            SimulatedError::InvalidRequest { message } => {
                ErrorResponse::invalid_request(message.clone())
            }
            SimulatedError::AuthenticationError => {
                ErrorResponse::new("Invalid API key provided", "authentication_error")
            }
        }
    }

    /// Get Retry-After header value if applicable
    pub fn retry_after(&self) -> Option<u32> {
        match self {
            SimulatedError::RateLimit {
                retry_after_seconds,
            } => Some(*retry_after_seconds),
            SimulatedError::ServiceUnavailable => Some(60),
            _ => None,
        }
    }
}

/// Error injector that decides whether to return an error
pub struct ErrorInjector {
    config: ErrorConfig,
}

impl ErrorInjector {
    pub fn new(config: ErrorConfig) -> Self {
        Self { config }
    }

    /// Decide whether to inject an error based on configured rates
    /// Returns None if no error should be injected
    pub fn maybe_inject(&self) -> Option<SimulatedError> {
        let mut rng = rand::rng();
        let roll: f64 = rng.random();

        let mut threshold = 0.0;

        // Check rate limit
        threshold += self.config.rate_limit_rate;
        if roll < threshold {
            return Some(SimulatedError::RateLimit {
                retry_after_seconds: rng.random_range(1..60),
            });
        }

        // Check server error
        threshold += self.config.server_error_rate;
        if roll < threshold {
            // Randomly choose between 500 and 503
            return if rng.random_bool(0.7) {
                Some(SimulatedError::ServerError)
            } else {
                Some(SimulatedError::ServiceUnavailable)
            };
        }

        // Check timeout
        threshold += self.config.timeout_rate;
        if roll < threshold {
            return Some(SimulatedError::Timeout {
                after: Duration::from_millis(self.config.timeout_after_ms),
            });
        }

        // Check invalid request
        threshold += self.config.invalid_request_rate;
        if roll < threshold {
            return Some(SimulatedError::InvalidRequest {
                message: "Simulated invalid request error".to_string(),
            });
        }

        // Check auth error
        threshold += self.config.auth_error_rate;
        if roll < threshold {
            return Some(SimulatedError::AuthenticationError);
        }

        None
    }

    /// Check if error injection is enabled (any rate > 0)
    pub fn is_enabled(&self) -> bool {
        self.config.total_error_rate() > 0.0
    }

    /// Get the underlying config
    pub fn config(&self) -> &ErrorConfig {
        &self.config
    }
}

impl Default for ErrorInjector {
    fn default() -> Self {
        Self::new(ErrorConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_config_defaults() {
        let config = ErrorConfig::default();
        assert_eq!(config.rate_limit_rate, 0.0);
        assert_eq!(config.server_error_rate, 0.0);
        assert_eq!(config.total_error_rate(), 0.0);
    }

    #[test]
    fn test_error_config_none() {
        let config = ErrorConfig::none();
        assert_eq!(config.total_error_rate(), 0.0);
    }

    #[test]
    fn test_error_config_chaos() {
        let config = ErrorConfig::chaos();
        assert!(config.total_error_rate() > 0.0);
        assert!(config.rate_limit_rate > 0.0);
    }

    #[test]
    fn test_error_config_builder() {
        let config = ErrorConfig::new()
            .with_rate_limit_rate(0.1)
            .with_server_error_rate(0.05);

        assert_eq!(config.rate_limit_rate, 0.1);
        assert_eq!(config.server_error_rate, 0.05);
    }

    #[test]
    fn test_error_config_clamps_values() {
        let config = ErrorConfig::new()
            .with_rate_limit_rate(1.5) // Should clamp to 1.0
            .with_server_error_rate(-0.5); // Should clamp to 0.0

        assert_eq!(config.rate_limit_rate, 1.0);
        assert_eq!(config.server_error_rate, 0.0);
    }

    #[test]
    fn test_simulated_error_status_codes() {
        assert_eq!(
            SimulatedError::RateLimit {
                retry_after_seconds: 30
            }
            .status_code(),
            429
        );
        assert_eq!(SimulatedError::ServerError.status_code(), 500);
        assert_eq!(SimulatedError::ServiceUnavailable.status_code(), 503);
        assert_eq!(
            SimulatedError::Timeout {
                after: Duration::from_secs(30)
            }
            .status_code(),
            504
        );
        assert_eq!(
            SimulatedError::InvalidRequest {
                message: "test".to_string()
            }
            .status_code(),
            400
        );
        assert_eq!(SimulatedError::AuthenticationError.status_code(), 401);
    }

    #[test]
    fn test_error_response_conversion() {
        let error = SimulatedError::RateLimit {
            retry_after_seconds: 30,
        };
        let response = error.to_error_response();
        assert_eq!(response.error.error_type, "rate_limit_error");
    }

    #[test]
    fn test_retry_after() {
        let error = SimulatedError::RateLimit {
            retry_after_seconds: 45,
        };
        assert_eq!(error.retry_after(), Some(45));

        let error = SimulatedError::ServerError;
        assert_eq!(error.retry_after(), None);
    }

    #[test]
    fn test_error_injector_disabled() {
        let injector = ErrorInjector::new(ErrorConfig::none());
        assert!(!injector.is_enabled());

        // Should never inject errors
        for _ in 0..100 {
            assert!(injector.maybe_inject().is_none());
        }
    }

    #[test]
    fn test_error_injector_always_rate_limit() {
        let injector = ErrorInjector::new(ErrorConfig::new().with_rate_limit_rate(1.0));
        assert!(injector.is_enabled());

        // Should always inject rate limit error
        for _ in 0..10 {
            let error = injector.maybe_inject();
            assert!(error.is_some());
            assert!(matches!(error.unwrap(), SimulatedError::RateLimit { .. }));
        }
    }

    #[test]
    fn test_error_rate_distribution() {
        // Test that error rates approximately match configured rates
        let config = ErrorConfig::new().with_rate_limit_rate(0.5);
        let injector = ErrorInjector::new(config);

        let mut errors = 0;
        let trials = 1000;

        for _ in 0..trials {
            if injector.maybe_inject().is_some() {
                errors += 1;
            }
        }

        // Should be roughly 50% (with some variance)
        let error_rate = errors as f64 / trials as f64;
        assert!(
            error_rate > 0.4 && error_rate < 0.6,
            "Error rate {} not within expected range",
            error_rate
        );
    }
}
