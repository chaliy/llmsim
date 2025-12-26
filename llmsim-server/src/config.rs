// Server Configuration Module
// Handles configuration from files and environment variables.

use llmsim::{ErrorConfig, LatencyProfile};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub latency: LatencyConfig,
    #[serde(default)]
    pub response: ResponseConfig,
    #[serde(default)]
    pub errors: ErrorsConfig,
    #[serde(default)]
    pub models: ModelsConfig,
}


impl Config {
    /// Load configuration from a YAML file
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let content =
            std::fs::read_to_string(path.as_ref()).map_err(|e| ConfigError::Io(e.to_string()))?;
        Self::from_yaml(&content)
    }

    /// Parse configuration from YAML string
    pub fn from_yaml(yaml: &str) -> Result<Self, ConfigError> {
        serde_yaml::from_str(yaml).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    /// Create a latency profile from the configuration
    pub fn latency_profile(&self) -> LatencyProfile {
        if let Some(ref profile) = self.latency.profile {
            match profile.to_lowercase().as_str() {
                "gpt4" | "gpt-4" => LatencyProfile::gpt4(),
                "gpt4o" | "gpt-4o" => LatencyProfile::gpt4o(),
                "gpt35" | "gpt-3.5-turbo" => LatencyProfile::gpt35_turbo(),
                "claude-opus" | "opus" => LatencyProfile::claude_opus(),
                "claude-sonnet" | "sonnet" => LatencyProfile::claude_sonnet(),
                "claude-haiku" | "haiku" => LatencyProfile::claude_haiku(),
                "gemini" | "gemini-pro" => LatencyProfile::gemini_pro(),
                "instant" => LatencyProfile::instant(),
                "fast" => LatencyProfile::fast(),
                _ => LatencyProfile::gpt4(),
            }
        } else if self.latency.ttft_mean_ms.is_some() || self.latency.tbt_mean_ms.is_some() {
            LatencyProfile::new(
                self.latency.ttft_mean_ms.unwrap_or(800),
                self.latency.ttft_stddev_ms.unwrap_or(200),
                self.latency.tbt_mean_ms.unwrap_or(50),
                self.latency.tbt_stddev_ms.unwrap_or(15),
            )
        } else {
            LatencyProfile::gpt4()
        }
    }

    /// Create an error config from the configuration
    pub fn error_config(&self) -> ErrorConfig {
        ErrorConfig {
            rate_limit_rate: self.errors.rate_limit_rate,
            server_error_rate: self.errors.server_error_rate,
            timeout_rate: self.errors.timeout_rate,
            timeout_after_ms: self.errors.timeout_after_ms,
            invalid_request_rate: 0.0,
            auth_error_rate: 0.0,
        }
    }
}

/// Server network configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
}

fn default_port() -> u16 {
    8080
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
        }
    }
}

/// Latency simulation configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LatencyConfig {
    /// Named latency profile (e.g., "gpt4", "claude-sonnet", "instant")
    pub profile: Option<String>,
    /// Custom time to first token mean (ms)
    pub ttft_mean_ms: Option<u64>,
    /// Custom time to first token stddev (ms)
    pub ttft_stddev_ms: Option<u64>,
    /// Custom time between tokens mean (ms)
    pub tbt_mean_ms: Option<u64>,
    /// Custom time between tokens stddev (ms)
    pub tbt_stddev_ms: Option<u64>,
}

/// Response generation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseConfig {
    /// Generator type: "lorem", "echo", "random", "sequence", "fixed:..."
    #[serde(default = "default_generator")]
    pub generator: String,
    /// Target number of tokens in response
    #[serde(default = "default_target_tokens")]
    pub target_tokens: usize,
}

fn default_generator() -> String {
    "lorem".to_string()
}

fn default_target_tokens() -> usize {
    100
}

impl Default for ResponseConfig {
    fn default() -> Self {
        Self {
            generator: default_generator(),
            target_tokens: default_target_tokens(),
        }
    }
}

/// Error injection configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ErrorsConfig {
    /// Rate of 429 rate limit errors (0.0-1.0)
    #[serde(default)]
    pub rate_limit_rate: f64,
    /// Rate of 500 server errors (0.0-1.0)
    #[serde(default)]
    pub server_error_rate: f64,
    /// Rate of timeout errors (0.0-1.0)
    #[serde(default)]
    pub timeout_rate: f64,
    /// Milliseconds before timeout (default 30000)
    #[serde(default = "default_timeout")]
    pub timeout_after_ms: u64,
}

fn default_timeout() -> u64 {
    30000
}

/// Models configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    /// List of available model IDs
    #[serde(default = "default_models")]
    pub available: Vec<String>,
}

fn default_models() -> Vec<String> {
    vec![
        "gpt-4".to_string(),
        "gpt-4-turbo".to_string(),
        "gpt-4o".to_string(),
        "gpt-4o-mini".to_string(),
        "gpt-3.5-turbo".to_string(),
        "claude-3-opus-20240229".to_string(),
        "claude-3-sonnet-20240229".to_string(),
        "claude-3-haiku-20240307".to_string(),
    ]
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            available: default_models(),
        }
    }
}

/// Configuration errors
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Failed to read configuration file: {0}")]
    Io(String),
    #[error("Failed to parse configuration: {0}")]
    Parse(String),
    #[error("Invalid configuration: {0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.response.generator, "lorem");
        assert_eq!(config.response.target_tokens, 100);
    }

    #[test]
    fn test_parse_yaml() {
        let yaml = r#"
server:
  port: 9000
  host: "127.0.0.1"

latency:
  profile: "gpt4"

response:
  generator: "echo"
  target_tokens: 50

errors:
  rate_limit_rate: 0.01
"#;
        let config = Config::from_yaml(yaml).unwrap();
        assert_eq!(config.server.port, 9000);
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.latency.profile, Some("gpt4".to_string()));
        assert_eq!(config.response.generator, "echo");
        assert_eq!(config.response.target_tokens, 50);
        assert_eq!(config.errors.rate_limit_rate, 0.01);
    }

    #[test]
    fn test_custom_latency() {
        let yaml = r#"
latency:
  ttft_mean_ms: 500
  tbt_mean_ms: 25
"#;
        let config = Config::from_yaml(yaml).unwrap();
        let profile = config.latency_profile();
        assert_eq!(profile.ttft_mean_ms, 500);
        assert_eq!(profile.tbt_mean_ms, 25);
    }

    #[test]
    fn test_latency_profile_from_name() {
        let yaml = r#"
latency:
  profile: "instant"
"#;
        let config = Config::from_yaml(yaml).unwrap();
        let profile = config.latency_profile();
        assert_eq!(profile.ttft_mean_ms, 0);
    }

    #[test]
    fn test_error_config() {
        let yaml = r#"
errors:
  rate_limit_rate: 0.1
  server_error_rate: 0.05
"#;
        let config = Config::from_yaml(yaml).unwrap();
        let error_config = config.error_config();
        assert_eq!(error_config.rate_limit_rate, 0.1);
        assert_eq!(error_config.server_error_rate, 0.05);
    }
}
