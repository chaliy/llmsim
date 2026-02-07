// Server Configuration Module
// Handles configuration from files and environment variables.

use crate::{ErrorConfig, LatencyProfile};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
                // GPT-5 family
                "gpt5" | "gpt-5" => LatencyProfile::gpt5(),
                "gpt5-mini" | "gpt-5-mini" => LatencyProfile::gpt5_mini(),
                // O-series reasoning models (o3, o4)
                "o3" | "o4" | "o-series" => LatencyProfile::o_series(),
                // GPT-4 family
                "gpt4" | "gpt-4" => LatencyProfile::gpt4(),
                "gpt4o" | "gpt-4o" => LatencyProfile::gpt4o(),
                // Claude family
                "claude-opus" | "opus" => LatencyProfile::claude_opus(),
                "claude-sonnet" | "sonnet" => LatencyProfile::claude_sonnet(),
                "claude-haiku" | "haiku" => LatencyProfile::claude_haiku(),
                // Gemini
                "gemini" | "gemini-pro" => LatencyProfile::gemini_pro(),
                // Special profiles
                "instant" => LatencyProfile::instant(),
                "fast" => LatencyProfile::fast(),
                _ => LatencyProfile::gpt5(),
            }
        } else if self.latency.ttft_mean_ms.is_some() || self.latency.tbt_mean_ms.is_some() {
            LatencyProfile::new(
                self.latency.ttft_mean_ms.unwrap_or(600),
                self.latency.ttft_stddev_ms.unwrap_or(150),
                self.latency.tbt_mean_ms.unwrap_or(40),
                self.latency.tbt_stddev_ms.unwrap_or(12),
            )
        } else {
            LatencyProfile::gpt5()
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
    // Default model list with profiles from https://models.dev
    // Each model has realistic context_window and max_output_tokens
    vec![
        // GPT-5 family (from models.dev)
        "gpt-5".to_string(),
        "gpt-5-mini".to_string(),
        "gpt-5-nano".to_string(),
        "gpt-5-codex".to_string(),
        "gpt-5.1".to_string(),
        "gpt-5.1-codex".to_string(),
        "gpt-5.1-codex-mini".to_string(),
        "gpt-5.1-codex-max".to_string(),
        "gpt-5.2".to_string(),
        "gpt-5.2-pro".to_string(),
        "gpt-5.2-codex".to_string(),
        "gpt-5.3-codex".to_string(),
        // O-series reasoning models
        "o3".to_string(),
        "o3-mini".to_string(),
        "o4-mini".to_string(),
        // GPT-4 family
        "gpt-4".to_string(),
        "gpt-4-turbo".to_string(),
        "gpt-4o".to_string(),
        "gpt-4o-mini".to_string(),
        "gpt-4.1".to_string(),
        // Claude family
        "claude-3.5-sonnet".to_string(),
        "claude-3.7-sonnet".to_string(),
        "claude-sonnet-4".to_string(),
        "claude-sonnet-4.5".to_string(),
        "claude-opus-4".to_string(),
        "claude-opus-4.1".to_string(),
        "claude-opus-4.5".to_string(),
        "claude-opus-4.6".to_string(),
        "claude-haiku-4.5".to_string(),
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
