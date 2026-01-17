// OpenAI Model Profiles
// Model specifications sourced from https://models.dev
// These profiles contain realistic context windows, output limits, and capabilities
// for use in simulating LLM API behavior.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

/// Capabilities that a model may support
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ModelCapabilities {
    /// Supports function/tool calling
    #[serde(default)]
    pub function_calling: bool,
    /// Supports vision (image input)
    #[serde(default)]
    pub vision: bool,
    /// Supports JSON mode / structured output
    #[serde(default)]
    pub json_mode: bool,
    /// Extended reasoning capabilities (o-series models)
    #[serde(default)]
    pub reasoning: bool,
}

/// A model profile containing realistic specifications from models.dev
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfile {
    /// Model identifier (e.g., "gpt-5")
    pub id: String,
    /// Human-readable model name
    pub name: String,
    /// Organization that owns the model
    pub owned_by: String,
    /// Maximum context window size in tokens
    pub context_window: u32,
    /// Maximum output tokens per request
    pub max_output_tokens: u32,
    /// Unix timestamp when the model was released
    pub created: i64,
    /// Model capabilities
    pub capabilities: ModelCapabilities,
    /// Knowledge cutoff date (YYYY-MM-DD format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff: Option<String>,
}

impl ModelProfile {
    /// Create a new model profile
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        owned_by: impl Into<String>,
        context_window: u32,
        max_output_tokens: u32,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            owned_by: owned_by.into(),
            context_window,
            max_output_tokens,
            created: chrono::Utc::now().timestamp(),
            capabilities: ModelCapabilities::default(),
            knowledge_cutoff: None,
        }
    }

    /// Builder method to set created timestamp
    pub fn with_created(mut self, timestamp: i64) -> Self {
        self.created = timestamp;
        self
    }

    /// Builder method to set capabilities
    pub fn with_capabilities(mut self, capabilities: ModelCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Builder method to set knowledge cutoff
    pub fn with_knowledge_cutoff(mut self, cutoff: impl Into<String>) -> Self {
        self.knowledge_cutoff = Some(cutoff.into());
        self
    }
}

/// Standard capabilities for GPT-5 series models
fn gpt5_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        function_calling: true,
        vision: true,
        json_mode: true,
        reasoning: true,
    }
}

/// Standard capabilities for GPT-4o series models
fn gpt4o_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        function_calling: true,
        vision: true,
        json_mode: true,
        reasoning: false,
    }
}

/// Standard capabilities for GPT-4 series models
fn gpt4_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        function_calling: true,
        vision: false,
        json_mode: true,
        reasoning: false,
    }
}

/// Standard capabilities for O-series reasoning models
fn o_series_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        function_calling: true,
        vision: true,
        json_mode: true,
        reasoning: true,
    }
}

/// Standard capabilities for Claude models
fn claude_capabilities() -> ModelCapabilities {
    ModelCapabilities {
        function_calling: true,
        vision: true,
        json_mode: true,
        reasoning: false,
    }
}

/// Build the static model registry with profiles from models.dev
fn build_model_registry() -> HashMap<String, ModelProfile> {
    let mut registry = HashMap::new();

    // GPT-5 family (from models.dev)
    // Released August 2025, 400K context window
    let gpt5_models = vec![
        ModelProfile::new("gpt-5", "GPT-5", "openai", 400_000, 128_000)
            .with_created(1754524800) // 2025-08-07
            .with_capabilities(gpt5_capabilities())
            .with_knowledge_cutoff("2024-09-30"),
        ModelProfile::new("gpt-5-mini", "GPT-5 Mini", "openai", 400_000, 128_000)
            .with_created(1754524800)
            .with_capabilities(gpt5_capabilities())
            .with_knowledge_cutoff("2024-05-30"),
        ModelProfile::new("gpt-5-nano", "GPT-5 Nano", "openai", 400_000, 128_000)
            .with_created(1754524800)
            .with_capabilities(gpt5_capabilities())
            .with_knowledge_cutoff("2024-05-30"),
        ModelProfile::new("gpt-5-codex", "GPT-5 Codex", "openai", 400_000, 128_000)
            .with_created(1754524800)
            .with_capabilities(gpt5_capabilities())
            .with_knowledge_cutoff("2024-09-30"),
        // GPT-5.1 variants
        ModelProfile::new("gpt-5.1", "GPT-5.1", "openai", 400_000, 128_000)
            .with_created(1762387200) // 2025-11-06
            .with_capabilities(gpt5_capabilities())
            .with_knowledge_cutoff("2025-03-31"),
        ModelProfile::new("gpt-5.1-codex", "GPT-5.1 Codex", "openai", 400_000, 128_000)
            .with_created(1762387200)
            .with_capabilities(gpt5_capabilities())
            .with_knowledge_cutoff("2025-03-31"),
        ModelProfile::new(
            "gpt-5.1-codex-mini",
            "GPT-5.1 Codex Mini",
            "openai",
            400_000,
            128_000,
        )
        .with_created(1762387200)
        .with_capabilities(gpt5_capabilities())
        .with_knowledge_cutoff("2025-03-31"),
        ModelProfile::new(
            "gpt-5.1-codex-max",
            "GPT-5.1 Codex Max",
            "openai",
            400_000,
            128_000,
        )
        .with_created(1762387200)
        .with_capabilities(gpt5_capabilities())
        .with_knowledge_cutoff("2025-03-31"),
        // GPT-5.2
        ModelProfile::new("gpt-5.2", "GPT-5.2", "openai", 400_000, 128_000)
            .with_created(1765411200) // 2025-12-11
            .with_capabilities(gpt5_capabilities())
            .with_knowledge_cutoff("2025-08-31"),
    ];

    // O-series reasoning models
    let o_series_models = vec![
        ModelProfile::new("o3", "O3", "openai", 200_000, 100_000)
            .with_created(1765411200) // 2025-12-11
            .with_capabilities(o_series_capabilities())
            .with_knowledge_cutoff("2024-12-31"),
        ModelProfile::new("o3-mini", "O3 Mini", "openai", 200_000, 100_000)
            .with_created(1765411200)
            .with_capabilities(o_series_capabilities())
            .with_knowledge_cutoff("2024-12-31"),
        ModelProfile::new("o4-mini", "O4 Mini", "openai", 200_000, 100_000)
            .with_created(1768003200) // 2026-01-10
            .with_capabilities(o_series_capabilities())
            .with_knowledge_cutoff("2025-06-30"),
    ];

    // GPT-4 family
    let gpt4_models = vec![
        // GPT-4o (May 2024)
        ModelProfile::new("gpt-4o", "GPT-4o", "openai", 128_000, 16_384)
            .with_created(1715558400) // 2024-05-13
            .with_capabilities(gpt4o_capabilities())
            .with_knowledge_cutoff("2023-10-01"),
        ModelProfile::new("gpt-4o-mini", "GPT-4o Mini", "openai", 128_000, 16_384)
            .with_created(1721692800) // 2024-07-23
            .with_capabilities(gpt4o_capabilities())
            .with_knowledge_cutoff("2023-10-01"),
        // GPT-4 Turbo (April 2024)
        ModelProfile::new("gpt-4-turbo", "GPT-4 Turbo", "openai", 128_000, 4_096)
            .with_created(1712620800) // 2024-04-09
            .with_capabilities(gpt4_capabilities())
            .with_knowledge_cutoff("2023-12-01"),
        // GPT-4 (March 2023)
        ModelProfile::new("gpt-4", "GPT-4", "openai", 8_192, 8_192)
            .with_created(1678838400) // 2023-03-15
            .with_capabilities(gpt4_capabilities())
            .with_knowledge_cutoff("2023-04-01"),
        // GPT-4.1 (April 2025)
        ModelProfile::new("gpt-4.1", "GPT-4.1", "openai", 1_047_576, 32_768)
            .with_created(1744675200) // 2025-04-14
            .with_capabilities(gpt4o_capabilities())
            .with_knowledge_cutoff("2024-06-01"),
    ];

    // Claude family (for completeness when using simulator)
    let claude_models = vec![
        // Claude 3.5 Sonnet
        ModelProfile::new(
            "claude-3.5-sonnet",
            "Claude 3.5 Sonnet",
            "anthropic",
            200_000,
            8_192,
        )
        .with_created(1718841600) // 2024-06-20
        .with_capabilities(claude_capabilities())
        .with_knowledge_cutoff("2024-04-01"),
        // Claude 3.7 Sonnet
        ModelProfile::new(
            "claude-3.7-sonnet",
            "Claude 3.7 Sonnet",
            "anthropic",
            200_000,
            64_000,
        )
        .with_created(1740355200) // 2025-02-24
        .with_capabilities(ModelCapabilities {
            function_calling: true,
            vision: true,
            json_mode: true,
            reasoning: true, // Extended thinking
        })
        .with_knowledge_cutoff("2024-11-01"),
        // Claude Sonnet 4
        ModelProfile::new(
            "claude-sonnet-4",
            "Claude Sonnet 4",
            "anthropic",
            200_000,
            64_000,
        )
        .with_created(1747958400) // 2025-05-23
        .with_capabilities(claude_capabilities())
        .with_knowledge_cutoff("2025-03-01"),
        // Claude Sonnet 4.5
        ModelProfile::new(
            "claude-sonnet-4.5",
            "Claude Sonnet 4.5",
            "anthropic",
            200_000,
            64_000,
        )
        .with_created(1756684800) // 2025-09-01
        .with_capabilities(claude_capabilities())
        .with_knowledge_cutoff("2025-05-01"),
        // Claude Opus 4
        ModelProfile::new(
            "claude-opus-4",
            "Claude Opus 4",
            "anthropic",
            200_000,
            64_000,
        )
        .with_created(1747958400) // 2025-05-23
        .with_capabilities(claude_capabilities())
        .with_knowledge_cutoff("2025-03-01"),
        // Claude Opus 4.5
        ModelProfile::new(
            "claude-opus-4.5",
            "Claude Opus 4.5",
            "anthropic",
            200_000,
            64_000,
        )
        .with_created(1756684800) // 2025-09-01
        .with_capabilities(claude_capabilities())
        .with_knowledge_cutoff("2025-05-01"),
        // Claude Haiku 4.5
        ModelProfile::new(
            "claude-haiku-4.5",
            "Claude Haiku 4.5",
            "anthropic",
            200_000,
            64_000,
        )
        .with_created(1756684800) // 2025-09-01
        .with_capabilities(claude_capabilities())
        .with_knowledge_cutoff("2025-05-01"),
    ];

    // Add all models to registry
    for model in gpt5_models
        .into_iter()
        .chain(o_series_models)
        .chain(gpt4_models)
        .chain(claude_models)
    {
        registry.insert(model.id.clone(), model);
    }

    registry
}

/// Static registry of all known model profiles
/// Sourced from https://models.dev
pub static MODEL_REGISTRY: LazyLock<HashMap<String, ModelProfile>> =
    LazyLock::new(build_model_registry);

/// Get a model profile by ID
pub fn get_model_profile(model_id: &str) -> Option<&'static ModelProfile> {
    MODEL_REGISTRY.get(model_id)
}

/// Get all available model profiles
pub fn all_model_profiles() -> impl Iterator<Item = &'static ModelProfile> {
    MODEL_REGISTRY.values()
}

/// Get all model IDs sorted alphabetically
pub fn all_model_ids() -> Vec<&'static str> {
    let mut ids: Vec<&str> = MODEL_REGISTRY.keys().map(|s| s.as_str()).collect();
    ids.sort();
    ids
}

/// Determine the owner of a model by ID (fallback for unknown models)
pub fn infer_model_owner(model_id: &str) -> &'static str {
    // First check the registry
    if let Some(profile) = get_model_profile(model_id) {
        // Return a static str by matching known owners
        return match profile.owned_by.as_str() {
            "openai" => "openai",
            "anthropic" => "anthropic",
            "google" => "google",
            _ => "llmsim",
        };
    }

    // Fallback to pattern matching for custom models
    let model_lower = model_id.to_lowercase();
    if model_lower.contains("gpt") || model_lower.starts_with("o3") || model_lower.starts_with("o4")
    {
        "openai"
    } else if model_lower.contains("claude") {
        "anthropic"
    } else if model_lower.contains("gemini") {
        "google"
    } else {
        "llmsim"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_registry_populated() {
        assert!(!MODEL_REGISTRY.is_empty());
        assert!(MODEL_REGISTRY.len() >= 20);
    }

    #[test]
    fn test_gpt5_profile() {
        let profile = get_model_profile("gpt-5").expect("gpt-5 should exist");
        assert_eq!(profile.id, "gpt-5");
        assert_eq!(profile.owned_by, "openai");
        assert_eq!(profile.context_window, 400_000);
        assert_eq!(profile.max_output_tokens, 128_000);
        assert!(profile.capabilities.reasoning);
        assert!(profile.capabilities.function_calling);
        assert!(profile.capabilities.vision);
    }

    #[test]
    fn test_gpt4o_profile() {
        let profile = get_model_profile("gpt-4o").expect("gpt-4o should exist");
        assert_eq!(profile.context_window, 128_000);
        assert_eq!(profile.max_output_tokens, 16_384);
        assert!(!profile.capabilities.reasoning);
    }

    #[test]
    fn test_o_series_profile() {
        let profile = get_model_profile("o3").expect("o3 should exist");
        assert_eq!(profile.owned_by, "openai");
        assert!(profile.capabilities.reasoning);
    }

    #[test]
    fn test_claude_profile() {
        let profile = get_model_profile("claude-opus-4.5").expect("claude-opus-4.5 should exist");
        assert_eq!(profile.owned_by, "anthropic");
        assert_eq!(profile.context_window, 200_000);
    }

    #[test]
    fn test_infer_model_owner() {
        assert_eq!(infer_model_owner("gpt-5"), "openai");
        assert_eq!(infer_model_owner("claude-opus-4.5"), "anthropic");
        assert_eq!(infer_model_owner("o3-mini"), "openai");
        assert_eq!(infer_model_owner("custom-model"), "llmsim");
    }

    #[test]
    fn test_all_model_ids_sorted() {
        let ids = all_model_ids();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn test_model_capabilities_serialize() {
        let caps = gpt5_capabilities();
        let json = serde_json::to_string(&caps).unwrap();
        assert!(json.contains("\"function_calling\":true"));
        assert!(json.contains("\"reasoning\":true"));
    }

    #[test]
    fn test_model_profile_serialize() {
        let profile = get_model_profile("gpt-5").unwrap();
        let json = serde_json::to_string(profile).unwrap();
        assert!(json.contains("\"context_window\":400000"));
        assert!(json.contains("\"max_output_tokens\":128000"));
        assert!(json.contains("\"knowledge_cutoff\""));
    }
}
