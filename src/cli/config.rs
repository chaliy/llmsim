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
    #[serde(default)]
    pub rate_limits: RateLimitsConfig,
    #[serde(default)]
    pub validation: ValidationConfig,
}

/// Request limits the real APIs enforce.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationConfig {
    /// Reject prompts over the model's context window and output caps over
    /// its max output tokens (models without a profile are never checked)
    #[serde(default = "default_true")]
    pub enforce_limits: bool,
    /// Reject model ids llmsim has no profile for (404), like the real APIs
    #[serde(default)]
    pub strict_models: bool,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            enforce_limits: true,
            strict_models: false,
        }
    }
}

/// Provider-style rate limits per API key (0 = unlimited). When set,
/// responses carry `x-ratelimit-*` / `anthropic-ratelimit-*` headers and an
/// exhausted key gets 429 with Retry-After.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RateLimitsConfig {
    /// Requests per minute per key
    #[serde(default)]
    pub requests_per_minute: u64,
    /// Estimated tokens (prompt + output cap) per minute per key
    #[serde(default)]
    pub tokens_per_minute: u64,
}

impl Config {
    /// Load configuration from a TOML file
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let content =
            std::fs::read_to_string(path.as_ref()).map_err(|e| ConfigError::Io(e.to_string()))?;
        Self::from_toml(&content)
    }

    /// Parse configuration from TOML string
    pub fn from_toml(toml_str: &str) -> Result<Self, ConfigError> {
        toml::from_str(toml_str).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    /// Return a copy of this configuration with `overrides` deep-merged on top.
    ///
    /// `overrides` is a JSON object shaped like the TOML file (`{"server":
    /// {"port": 0}, "response": {"generator": "echo"}}`). Objects merge key by
    /// key; any other value (including arrays and `null`) replaces the base
    /// value. Unknown keys are ignored, as in TOML files. Used by the Python
    /// and Node bindings, which accept plain dicts/objects instead of TOML.
    pub fn merged_with_json(&self, overrides: &serde_json::Value) -> Result<Self, ConfigError> {
        if !overrides.is_object() {
            return Err(ConfigError::Validation(
                "config overrides must be an object".to_string(),
            ));
        }
        let mut base = serde_json::to_value(self).map_err(|e| ConfigError::Parse(e.to_string()))?;
        merge_json(&mut base, overrides);
        serde_json::from_value(base).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    /// Latency for a request to `model`: the configured profile or custom
    /// timings when set, otherwise the model's own profile. A configured
    /// `prefill_us_per_token` overrides the profile's prefill rate.
    pub fn latency_for_model(&self, model: &str) -> LatencyProfile {
        let profile = if self.latency.profile.is_some()
            || self.latency.ttft_mean_ms.is_some()
            || self.latency.tbt_mean_ms.is_some()
        {
            self.latency_profile()
        } else {
            LatencyProfile::from_model(model)
        };
        match self.latency.prefill_us_per_token {
            Some(us) => profile.with_prefill_us_per_token(us),
            None => profile,
        }
    }

    /// Create a latency profile from the configuration
    pub fn latency_profile(&self) -> LatencyProfile {
        if let Some(ref profile) = self.latency.profile {
            match profile.to_lowercase().as_str() {
                // GPT-5 family
                "gpt5" | "gpt-5" => LatencyProfile::gpt5(),
                "gpt5-mini" | "gpt-5-mini" => LatencyProfile::gpt5_mini(),
                // GPT-5.6+/GPT-6 tiers
                "sol" | "gpt-sol" => LatencyProfile::gpt_sol(),
                "terra" | "gpt-terra" => LatencyProfile::gpt_terra(),
                "luna" | "gpt-luna" => LatencyProfile::gpt_luna(),
                "astra" | "gpt-astra" => LatencyProfile::gpt_astra(),
                "muse-spark" | "muse" => LatencyProfile::muse_spark(),
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
                "gemini-flash" => LatencyProfile::gemini_flash(),
                // DeepSeek
                "deepseek" | "deepseek-chat" => LatencyProfile::deepseek(),
                "deepseek-reasoner" => LatencyProfile::deepseek_reasoner(),
                // TypeSafe System One
                "jev" => LatencyProfile::jev(),
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

fn merge_json(base: &mut serde_json::Value, overrides: &serde_json::Value) {
    match (base, overrides) {
        (serde_json::Value::Object(base), serde_json::Value::Object(overrides)) => {
            for (key, value) in overrides {
                match base.get_mut(key) {
                    Some(slot) => merge_json(slot, value),
                    None => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (slot, value) => *slot = value.clone(),
    }
}

/// Server network configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
    /// Maximum number of active WebSocket connections allowed
    #[serde(default = "default_max_websocket_connections")]
    pub max_websocket_connections: u64,
}

fn default_port() -> u16 {
    8080
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_max_websocket_connections() -> u64 {
    100
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
            max_websocket_connections: default_max_websocket_connections(),
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
    /// Prefill cost per input token (microseconds) added to TTFT; 0 disables
    pub prefill_us_per_token: Option<u64>,
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
    /// Optional path to a scripted response JSON file. When set, the
    /// server replays the script across requests and ignores
    /// `generator`. See `knowledge/simulation/scripted-mode.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_path: Option<String>,
    /// Optional path to a scenario JSON file or a directory of them. Requests
    /// whose last user message carries `[[llmsim:<name>]]` (or whose model is
    /// `llmsim-scenario-<name>`) play that scenario; other requests keep the
    /// script / generator behaviour. See `knowledge/simulation/scenarios.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenarios_path: Option<String>,
    /// Load the built-in starter scenarios (`examples/scenarios/`, compiled
    /// in). Scenarios from `scenarios_path` replace built-ins of the same
    /// name. Default `true`; a scenario only plays when a request names it.
    #[serde(default = "default_true")]
    pub builtin_scenarios: bool,
    /// With `tool_choice` auto (or unset), answer a user message with a call
    /// to one of the offered tools (schema-valid arguments), then answer in
    /// text once the tool result comes back. Default `false`: text only.
    #[serde(default)]
    pub auto_tool_calls: bool,
    /// Report cached prompt tokens when a prompt prefix repeats within five
    /// minutes (OpenAI automatic caching, Anthropic `cache_control`).
    /// Default `true`.
    #[serde(default = "default_true")]
    pub prompt_caching: bool,
}

fn default_true() -> bool {
    true
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
            script_path: None,
            scenarios_path: None,
            builtin_scenarios: true,
            auto_tool_calls: false,
            prompt_caching: true,
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
    /// Chance per request (0.0-1.0) that an overload burst starts: every
    /// generation request then fails (529 on Anthropic/TypeSafe, 503
    /// elsewhere) until `overload_burst_ms` passes
    #[serde(default)]
    pub overload_burst_rate: f64,
    /// Length of an overload burst in milliseconds (default 10000)
    #[serde(default = "default_burst_ms")]
    pub overload_burst_ms: u64,
}

fn default_burst_ms() -> u64 {
    10_000
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
        "gpt-5-pro".to_string(),
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
        "gpt-5.3-codex-spark".to_string(),
        "gpt-5.3-chat-latest".to_string(),
        "gpt-5.4".to_string(),
        "gpt-5.4-pro".to_string(),
        "gpt-5.4-mini".to_string(),
        "gpt-5.4-nano".to_string(),
        "gpt-5.5".to_string(),
        "gpt-5.5-pro".to_string(),
        "gpt-5.6".to_string(),
        "gpt-5.6-sol".to_string(),
        "gpt-5.6-terra".to_string(),
        "gpt-5.6-luna".to_string(),
        // GPT-6 family
        "gpt-6-astra".to_string(),
        "gpt-6-sol".to_string(),
        "gpt-6-luna".to_string(),
        "gpt-6.1-sol".to_string(),
        "gpt-6.1-terra".to_string(),
        // O-series reasoning models
        "o1".to_string(),
        "o1-mini".to_string(),
        "o3".to_string(),
        "o3-mini".to_string(),
        "o4-mini".to_string(),
        // GPT-4 family
        "gpt-4".to_string(),
        "gpt-4-turbo".to_string(),
        "gpt-4o".to_string(),
        "gpt-4o-mini".to_string(),
        "gpt-4.1".to_string(),
        "gpt-4.1-mini".to_string(),
        "gpt-4.1-nano".to_string(),
        // OpenAI image generation models (gpt-image / "ChatGPT Images")
        "gpt-image-1".to_string(),
        "gpt-image-1-mini".to_string(),
        "gpt-image-1.5".to_string(),
        "gpt-image-2".to_string(),
        "gpt-image-2.5".to_string(),
        "gpt-image-2.6".to_string(),
        // OpenAI embedding models
        "text-embedding-3-small".to_string(),
        "text-embedding-3-large".to_string(),
        "text-embedding-ada-002".to_string(),
        // Claude family
        "claude-3.5-sonnet".to_string(),
        "claude-3.7-sonnet".to_string(),
        "claude-sonnet-4".to_string(),
        "claude-sonnet-4.5".to_string(),
        "claude-sonnet-4.6".to_string(),
        "claude-opus-4".to_string(),
        "claude-opus-4.1".to_string(),
        "claude-opus-4.5".to_string(),
        "claude-opus-4.6".to_string(),
        "claude-opus-4.7".to_string(),
        "claude-opus-4.8".to_string(),
        "claude-haiku-4.5".to_string(),
        "claude-opus-5".to_string(),
        "claude-opus-5.5".to_string(),
        "claude-sonnet-5".to_string(),
        "claude-sonnet-5.5".to_string(),
        "claude-haiku-5.5".to_string(),
        "claude-fable-5".to_string(),
        "claude-fable-5.1".to_string(),
        "claude-fable-5.2".to_string(),
        "claude-mythos-5.2".to_string(),
        // Gemini family
        "gemini-2.0-flash".to_string(),
        "gemini-2.5-flash".to_string(),
        "gemini-2.5-pro".to_string(),
        "gemini-3-pro-preview".to_string(),
        "gemini-3-flash-preview".to_string(),
        "gemini-3.1-pro-preview".to_string(),
        "gemini-3.1-flash-lite".to_string(),
        "gemini-3.5-flash".to_string(),
        "gemini-3.5-flash-lite".to_string(),
        "gemini-3.6-flash".to_string(),
        "gemini-3.7-flash".to_string(),
        "gemini-3.8-flash".to_string(),
        "gemini-4-argon".to_string(),
        // DeepSeek family
        "deepseek-chat".to_string(),
        "deepseek-reasoner".to_string(),
        "deepseek-v4-pro".to_string(),
        "deepseek-flash".to_string(),
        // Other providers
        "grok-4.7".to_string(),
        "kimi-k3".to_string(),
        "qwen3.8-max".to_string(),
        "mistral-large-4".to_string(),
        "muse-spark-1.3".to_string(),
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
    fn test_merged_with_json_overrides_nested_fields() {
        let base =
            Config::from_toml("[response]\ngenerator = \"lorem\"\ntarget_tokens = 7\n").unwrap();
        let merged = base
            .merged_with_json(&serde_json::json!({
                "server": {"port": 0, "host": "127.0.0.1"},
                "response": {"generator": "echo"},
                "latency": {"profile": "instant"}
            }))
            .unwrap();
        assert_eq!(merged.server.port, 0);
        assert_eq!(merged.server.host, "127.0.0.1");
        assert_eq!(merged.response.generator, "echo");
        // Untouched sibling keys survive the merge.
        assert_eq!(merged.response.target_tokens, 7);
        assert_eq!(merged.latency.profile.as_deref(), Some("instant"));
        assert_eq!(merged.models.available, default_models());
    }

    #[test]
    fn test_merged_with_json_replaces_arrays() {
        let merged = Config::default()
            .merged_with_json(&serde_json::json!({"models": {"available": ["gpt-5"]}}))
            .unwrap();
        assert_eq!(merged.models.available, vec!["gpt-5".to_string()]);
    }

    #[test]
    fn test_merged_with_json_rejects_non_object_and_bad_types() {
        let base = Config::default();
        assert!(matches!(
            base.merged_with_json(&serde_json::json!([1, 2])),
            Err(ConfigError::Validation(_))
        ));
        assert!(matches!(
            base.merged_with_json(&serde_json::json!({"server": {"port": "nope"}})),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn test_default_models_all_have_profiles() {
        // Every advertised default model must resolve to a registry profile.
        for id in default_models() {
            assert!(
                crate::openai::get_model_profile(&id).is_some(),
                "{id} has no profile"
            );
        }
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.response.generator, "lorem");
        assert_eq!(config.response.target_tokens, 100);
    }

    #[test]
    fn test_parse_toml() {
        let toml_str = r#"
[server]
port = 9000
host = "127.0.0.1"

[latency]
profile = "gpt4"

[response]
generator = "echo"
target_tokens = 50

[errors]
rate_limit_rate = 0.01
"#;
        let config = Config::from_toml(toml_str).unwrap();
        assert_eq!(config.server.port, 9000);
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.latency.profile, Some("gpt4".to_string()));
        assert_eq!(config.response.generator, "echo");
        assert_eq!(config.response.target_tokens, 50);
        assert_eq!(config.errors.rate_limit_rate, 0.01);
    }

    #[test]
    fn test_custom_latency() {
        let toml_str = r#"
[latency]
ttft_mean_ms = 500
tbt_mean_ms = 25
"#;
        let config = Config::from_toml(toml_str).unwrap();
        let profile = config.latency_profile();
        assert_eq!(profile.ttft_mean_ms, 500);
        assert_eq!(profile.tbt_mean_ms, 25);
    }

    #[test]
    fn test_latency_profile_from_name() {
        let toml_str = r#"
[latency]
profile = "instant"
"#;
        let config = Config::from_toml(toml_str).unwrap();
        let profile = config.latency_profile();
        assert_eq!(profile.ttft_mean_ms, 0);
    }

    #[test]
    fn test_error_config() {
        let toml_str = r#"
[errors]
rate_limit_rate = 0.1
server_error_rate = 0.05
"#;
        let config = Config::from_toml(toml_str).unwrap();
        let error_config = config.error_config();
        assert_eq!(error_config.rate_limit_rate, 0.1);
        assert_eq!(error_config.server_error_rate, 0.05);
    }
}
