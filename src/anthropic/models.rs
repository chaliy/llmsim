// Anthropic Model Profiles
// Realistic Claude model specifications sourced from https://models.dev and the
// Anthropic model documentation (https://docs.anthropic.com/en/docs/about-claude/models).
//
// Unlike the OpenAI registry in `src/openai/models.rs`, these use the *real*
// Anthropic API model IDs (dash-separated, e.g. `claude-opus-4-8`) plus the
// dated snapshot IDs and `-latest` aliases the Anthropic SDKs actually send, so
// requests from the official SDK resolve to a profile.

use crate::openai::{ModelCapabilities, ModelProfile};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

/// Capabilities for modern Claude models with extended thinking (3.7+, 4.x, Fable).
fn reasoning_caps() -> ModelCapabilities {
    ModelCapabilities {
        function_calling: true,
        vision: true,
        json_mode: true,
        reasoning: true,
    }
}

/// Capabilities for older Claude models without extended thinking (3.5 and earlier).
fn standard_caps() -> ModelCapabilities {
    ModelCapabilities {
        function_calling: true,
        vision: true,
        json_mode: true,
        reasoning: false,
    }
}

/// Add a profile plus any aliases (dated snapshot, `-latest`) that map to the
/// same specifications, so SDK-issued IDs all resolve.
fn insert_with_aliases(
    registry: &mut HashMap<String, ModelProfile>,
    profile: ModelProfile,
    aliases: &[&str],
) {
    for alias in aliases {
        let mut p = profile.clone();
        p.id = (*alias).to_string();
        registry.insert(p.id.clone(), p);
    }
    registry.insert(profile.id.clone(), profile);
}

/// Build the static Anthropic model registry.
fn build_registry() -> HashMap<String, ModelProfile> {
    let mut registry = HashMap::new();

    // --- Claude Fable 5 (most capable widely-released model) ---
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-fable-5",
            "Claude Fable 5",
            "anthropic",
            1_000_000,
            128_000,
        )
        .with_created(1780272000) // 2026-06-01
        .with_capabilities(reasoning_caps()),
        &[],
    );

    // --- Opus 4.x family ---
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-opus-4-8",
            "Claude Opus 4.8",
            "anthropic",
            1_000_000,
            128_000,
        )
        .with_created(1779235200) // 2026-05-20 (approximate)
        .with_capabilities(reasoning_caps()),
        &[],
    );
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-opus-4-7",
            "Claude Opus 4.7",
            "anthropic",
            1_000_000,
            128_000,
        )
        .with_created(1776297600) // 2026-04-16
        .with_capabilities(reasoning_caps())
        .with_knowledge_cutoff("2026-01-31"),
        &[],
    );
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-opus-4-6",
            "Claude Opus 4.6",
            "anthropic",
            1_000_000,
            128_000,
        )
        .with_created(1770249600) // 2026-02-05
        .with_capabilities(reasoning_caps())
        .with_knowledge_cutoff("2025-05-31"),
        &[],
    );
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-opus-4-5",
            "Claude Opus 4.5",
            "anthropic",
            200_000,
            64_000,
        )
        .with_created(1761955200) // 2025-11-01
        .with_capabilities(reasoning_caps())
        .with_knowledge_cutoff("2025-03-31"),
        &["claude-opus-4-5-20251101"],
    );
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-opus-4-1",
            "Claude Opus 4.1",
            "anthropic",
            200_000,
            32_000,
        )
        .with_created(1754352000) // 2025-08-05
        .with_capabilities(reasoning_caps())
        .with_knowledge_cutoff("2025-03-31"),
        &["claude-opus-4-1-20250805"],
    );
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-opus-4-0",
            "Claude Opus 4",
            "anthropic",
            200_000,
            32_000,
        )
        .with_created(1747785600) // 2025-05-21
        .with_capabilities(reasoning_caps())
        .with_knowledge_cutoff("2025-03-01"),
        &["claude-opus-4-20250514"],
    );

    // --- Sonnet 4.x family ---
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-sonnet-4-6",
            "Claude Sonnet 4.6",
            "anthropic",
            1_000_000,
            64_000,
        )
        .with_created(1771027200) // 2026-02-15
        .with_capabilities(reasoning_caps())
        .with_knowledge_cutoff("2025-08-31"),
        &[],
    );
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-sonnet-4-5",
            "Claude Sonnet 4.5",
            "anthropic",
            1_000_000,
            64_000,
        )
        .with_created(1759104000) // 2025-09-29
        .with_capabilities(reasoning_caps())
        .with_knowledge_cutoff("2025-07-31"),
        &["claude-sonnet-4-5-20250929"],
    );
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-sonnet-4-0",
            "Claude Sonnet 4",
            "anthropic",
            1_000_000,
            64_000,
        )
        .with_created(1747785600) // 2025-05-21
        .with_capabilities(reasoning_caps())
        .with_knowledge_cutoff("2025-03-01"),
        &["claude-sonnet-4-20250514"],
    );

    // --- Haiku 4.5 ---
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-haiku-4-5",
            "Claude Haiku 4.5",
            "anthropic",
            200_000,
            64_000,
        )
        .with_created(1760486400) // 2025-10-15
        .with_capabilities(reasoning_caps())
        .with_knowledge_cutoff("2025-02-28"),
        &["claude-haiku-4-5-20251001"],
    );

    // --- Claude 3.x family (legacy, still referenced) ---
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-3-7-sonnet",
            "Claude Sonnet 3.7",
            "anthropic",
            200_000,
            64_000,
        )
        .with_created(1740355200) // 2025-02-24
        .with_capabilities(reasoning_caps())
        .with_knowledge_cutoff("2024-11-01"),
        &["claude-3-7-sonnet-20250219", "claude-3-7-sonnet-latest"],
    );
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-3-5-sonnet",
            "Claude Sonnet 3.5",
            "anthropic",
            200_000,
            8_192,
        )
        .with_created(1729555200) // 2024-10-22
        .with_capabilities(standard_caps())
        .with_knowledge_cutoff("2024-04-01"),
        &["claude-3-5-sonnet-20241022", "claude-3-5-sonnet-latest"],
    );
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-3-5-haiku",
            "Claude Haiku 3.5",
            "anthropic",
            200_000,
            8_192,
        )
        .with_created(1729555200) // 2024-10-22
        .with_capabilities(standard_caps())
        .with_knowledge_cutoff("2024-07-01"),
        &["claude-3-5-haiku-20241022", "claude-3-5-haiku-latest"],
    );
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-3-opus",
            "Claude Opus 3",
            "anthropic",
            200_000,
            4_096,
        )
        .with_created(1709164800) // 2024-02-29
        .with_capabilities(standard_caps())
        .with_knowledge_cutoff("2023-08-01"),
        &["claude-3-opus-20240229", "claude-3-opus-latest"],
    );
    insert_with_aliases(
        &mut registry,
        ModelProfile::new(
            "claude-3-haiku",
            "Claude Haiku 3",
            "anthropic",
            200_000,
            4_096,
        )
        .with_created(1709769600) // 2024-03-07
        .with_capabilities(standard_caps())
        .with_knowledge_cutoff("2023-08-01"),
        &["claude-3-haiku-20240307"],
    );

    registry
}

/// Static registry of Anthropic model profiles, keyed by API model ID.
pub static ANTHROPIC_MODEL_REGISTRY: LazyLock<HashMap<String, ModelProfile>> =
    LazyLock::new(build_registry);

/// Look up an Anthropic model profile by API ID (alias or dated snapshot).
pub fn get_anthropic_model_profile(model_id: &str) -> Option<&'static ModelProfile> {
    ANTHROPIC_MODEL_REGISTRY.get(model_id)
}

/// The canonical (alias) model IDs to advertise via the models endpoint, in a
/// stable, human-friendly order (newest/most-capable first).
pub fn default_anthropic_model_ids() -> Vec<&'static str> {
    vec![
        "claude-fable-5",
        "claude-opus-4-8",
        "claude-opus-4-7",
        "claude-opus-4-6",
        "claude-opus-4-5",
        "claude-opus-4-1",
        "claude-opus-4-0",
        "claude-sonnet-4-6",
        "claude-sonnet-4-5",
        "claude-sonnet-4-0",
        "claude-haiku-4-5",
        "claude-3-7-sonnet",
        "claude-3-5-sonnet",
        "claude-3-5-haiku",
        "claude-3-opus",
        "claude-3-haiku",
    ]
}

/// Anthropic model object as returned by `GET /anthropic/v1/models`.
/// Reference: https://docs.anthropic.com/en/api/models-list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicModel {
    #[serde(rename = "type")]
    pub object_type: String,
    pub id: String,
    pub display_name: String,
    /// ISO 8601 release timestamp.
    pub created_at: String,
    /// Context window (input) size in tokens. Exposed since Mar 2026.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    /// Maximum output tokens per request. Exposed since Mar 2026.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

impl AnthropicModel {
    pub fn from_profile(profile: &ModelProfile) -> Self {
        Self {
            object_type: "model".to_string(),
            id: profile.id.clone(),
            display_name: profile.name.clone(),
            created_at: iso8601_utc(profile.created),
            max_input_tokens: Some(profile.context_window),
            max_tokens: Some(profile.max_output_tokens),
        }
    }
}

/// Response body for `GET /anthropic/v1/models` (paginated list envelope).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnthropicModelsResponse {
    pub data: Vec<AnthropicModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_id: Option<String>,
    pub has_more: bool,
}

impl AnthropicModelsResponse {
    pub fn new(models: Vec<AnthropicModel>) -> Self {
        let first_id = models.first().map(|m| m.id.clone());
        let last_id = models.last().map(|m| m.id.clone());
        Self {
            data: models,
            first_id,
            last_id,
            has_more: false,
        }
    }
}

/// Format a Unix timestamp (seconds) as an ISO 8601 UTC string
/// (`YYYY-MM-DDThh:mm:ssZ`) without pulling in a date library. Uses Howard
/// Hinnant's `civil_from_days` algorithm.
fn iso8601_utc(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let secs_of_day = unix_secs.rem_euclid(86_400);
    let (hour, minute, second) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );

    // civil_from_days: days since 1970-01-01 -> (year, month, day)
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if month <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_populated() {
        assert!(ANTHROPIC_MODEL_REGISTRY.len() >= 16);
    }

    #[test]
    fn test_opus_4_8_profile() {
        let p = get_anthropic_model_profile("claude-opus-4-8").unwrap();
        assert_eq!(p.owned_by, "anthropic");
        assert_eq!(p.context_window, 1_000_000);
        assert_eq!(p.max_output_tokens, 128_000);
        assert!(p.capabilities.reasoning);
    }

    #[test]
    fn test_fable_5_profile() {
        let p = get_anthropic_model_profile("claude-fable-5").unwrap();
        assert_eq!(p.context_window, 1_000_000);
        assert_eq!(p.max_output_tokens, 128_000);
    }

    #[test]
    fn test_dated_snapshot_alias_resolves() {
        let alias = get_anthropic_model_profile("claude-haiku-4-5-20251001").unwrap();
        let canonical = get_anthropic_model_profile("claude-haiku-4-5").unwrap();
        assert_eq!(alias.context_window, canonical.context_window);
        assert_eq!(alias.max_output_tokens, canonical.max_output_tokens);
    }

    #[test]
    fn test_latest_alias_resolves() {
        assert!(get_anthropic_model_profile("claude-3-5-sonnet-latest").is_some());
        assert!(get_anthropic_model_profile("claude-3-5-sonnet-20241022").is_some());
    }

    #[test]
    fn test_haiku_3_no_reasoning() {
        let p = get_anthropic_model_profile("claude-3-haiku").unwrap();
        assert!(!p.capabilities.reasoning);
        assert_eq!(p.max_output_tokens, 4_096);
    }

    #[test]
    fn test_sonnet_4_6_profile() {
        let p = get_anthropic_model_profile("claude-sonnet-4-6").unwrap();
        assert_eq!(p.context_window, 1_000_000);
        assert_eq!(p.max_output_tokens, 64_000);
        assert_eq!(p.knowledge_cutoff.as_deref(), Some("2025-08-31"));
    }

    #[test]
    fn test_model_object_from_profile() {
        let p = get_anthropic_model_profile("claude-opus-4-8").unwrap();
        let m = AnthropicModel::from_profile(p);
        assert_eq!(m.object_type, "model");
        assert_eq!(m.id, "claude-opus-4-8");
        assert_eq!(m.display_name, "Claude Opus 4.8");
        assert!(m.created_at.ends_with('Z'));
        assert_eq!(m.max_input_tokens, Some(1_000_000));
    }

    #[test]
    fn test_models_response_envelope() {
        let models: Vec<AnthropicModel> = default_anthropic_model_ids()
            .iter()
            .map(|id| AnthropicModel::from_profile(get_anthropic_model_profile(id).unwrap()))
            .collect();
        let resp = AnthropicModelsResponse::new(models);
        assert_eq!(resp.first_id.as_deref(), Some("claude-fable-5"));
        assert_eq!(resp.last_id.as_deref(), Some("claude-3-haiku"));
        assert!(!resp.has_more);
    }

    #[test]
    fn test_iso8601_known_dates() {
        // 2024-01-01T00:00:00Z = 1704067200
        assert_eq!(iso8601_utc(1_704_067_200), "2024-01-01T00:00:00Z");
        // 1970-01-01T00:00:00Z = 0
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
        // 2026-02-15T00:00:00Z = 1771113600
        assert_eq!(iso8601_utc(1_771_113_600), "2026-02-15T00:00:00Z");
        // 2025-05-21T12:34:56Z = 1747830896
        assert_eq!(iso8601_utc(1_747_830_896), "2025-05-21T12:34:56Z");
    }

    #[test]
    fn test_default_ids_all_resolve() {
        for id in default_anthropic_model_ids() {
            assert!(
                get_anthropic_model_profile(id).is_some(),
                "default id {} should resolve",
                id
            );
        }
    }
}
