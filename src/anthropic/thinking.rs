// Anthropic extended-thinking simulation.
//
// Decides whether a Messages response carries a `thinking` block, how many
// thinking tokens it bills, and whether its text is visible, from the request's
// `thinking` / `output_config.effort` fields and the model's defaults.
//
// Decisions:
// - Model rules are keyed on ID prefixes rather than profile flags because they
//   are API-surface rules (which `thinking` types a model accepts, its default
//   display), not capabilities. Unknown/custom model IDs get the permissive
//   legacy behavior: nothing is rejected, thinking only when requested.
// - Thinking tokens are billed inside `usage.output_tokens`, like the real API.
// - The raw chain of thought is never returned; `display: "summarized"` yields a
//   short synthetic summary, `"omitted"` an empty string, as on current models.

use super::types::{OutputConfig, ThinkingConfig};
use rand::RngExt;

/// Effort levels accepted by `output_config.effort`.
pub const EFFORT_LEVELS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// The thinking block to emit for one response.
#[derive(Debug, Clone, PartialEq)]
pub struct ThinkingPlan {
    /// Thinking tokens billed in `usage.output_tokens`.
    pub tokens: usize,
    /// Visible thinking text (empty when display is `"omitted"`).
    pub text: String,
    /// Opaque signature clients echo back on the next turn.
    pub signature: String,
}

/// Models where thinking is always on unless explicitly turned off
/// (Fable 5.x, Opus 5.x, Sonnet 5.x, Haiku 5.5).
fn thinks_by_default(model: &str) -> bool {
    model.starts_with("claude-fable-")
        || model.starts_with("claude-opus-5")
        || model.starts_with("claude-sonnet-5")
        || model.starts_with("claude-haiku-5")
}

/// Models whose default `display` is `"omitted"` (Opus 4.7 onward).
fn omits_by_default(model: &str) -> bool {
    thinks_by_default(model)
        || model.starts_with("claude-opus-4-7")
        || model.starts_with("claude-opus-4-8")
}

/// Models where the fixed `budget_tokens` mode was removed.
fn rejects_budget_tokens(model: &str) -> bool {
    omits_by_default(model)
}

/// Models where `{type: "disabled"}` is rejected (thinking cannot be turned off).
fn rejects_disabled(model: &str) -> bool {
    model.starts_with("claude-fable-")
        || model.starts_with("claude-opus-5-5")
        || model.starts_with("claude-sonnet-5-5")
}

/// Default effort when `output_config.effort` is omitted.
fn default_effort(model: &str) -> &'static str {
    if model.starts_with("claude-opus-5-5") || model.starts_with("claude-haiku-5") {
        "medium"
    } else {
        "high"
    }
}

/// Validate the thinking/effort fields the way the real API does, returning an
/// `invalid_request_error` message on failure.
pub fn validate(
    model: &str,
    max_tokens: Option<u32>,
    thinking: Option<&ThinkingConfig>,
    output_config: Option<&OutputConfig>,
) -> Result<(), String> {
    if let Some(effort) = output_config.and_then(|o| o.effort.as_deref()) {
        if !EFFORT_LEVELS.contains(&effort) {
            return Err(format!(
                "output_config.effort: Input should be one of {}; got '{effort}'",
                EFFORT_LEVELS.join(", ")
            ));
        }
    }

    let Some(thinking) = thinking else {
        return Ok(());
    };

    match thinking.thinking_type.as_str() {
        "adaptive" | "between_tools" => Ok(()),
        "disabled" if rejects_disabled(model) => Err(format!(
            "thinking: type 'disabled' is not supported for {model}; \
             lower output_config.effort instead"
        )),
        "disabled" => Ok(()),
        "enabled" if rejects_budget_tokens(model) => Err(format!(
            "thinking: type 'enabled' is not supported for {model}; \
             use thinking: {{type: \"adaptive\"}} with output_config.effort"
        )),
        "enabled" => {
            let budget = thinking
                .budget_tokens
                .ok_or_else(|| "thinking.budget_tokens: Field required".to_string())?;
            if budget < 1024 {
                return Err("thinking.budget_tokens: must be at least 1024".to_string());
            }
            if let Some(max_tokens) = max_tokens {
                if budget >= max_tokens {
                    return Err(format!(
                        "`max_tokens` must be greater than `thinking.budget_tokens` \
                         ({max_tokens} <= {budget})"
                    ));
                }
            }
            Ok(())
        }
        other => Err(format!(
            "thinking.type: Input tag '{other}' found using 'type' does not match any \
             of the expected tags: 'adaptive', 'enabled', 'disabled'"
        )),
    }
}

/// Decide the thinking block (if any) for a response whose visible text is
/// `output_tokens` long. Assumes [`validate`] already passed.
pub fn plan(
    model: &str,
    thinking: Option<&ThinkingConfig>,
    output_config: Option<&OutputConfig>,
    output_tokens: usize,
) -> Option<ThinkingPlan> {
    let enabled = match thinking.map(|t| t.thinking_type.as_str()) {
        Some("adaptive") | Some("enabled") => true,
        Some(_) => false, // disabled / between_tools
        None => thinks_by_default(model),
    };
    if !enabled {
        return None;
    }

    let effort = output_config
        .and_then(|o| o.effort.as_deref())
        .unwrap_or_else(|| default_effort(model));
    let multiplier = match effort {
        "low" => 0.5,
        "medium" => 1.5,
        "xhigh" => 5.0,
        "max" => 8.0,
        _ => 3.0, // high
    };
    let mut tokens = ((output_tokens as f64 * multiplier) as usize).max(16);
    if let Some(budget) = thinking.and_then(|t| t.budget_tokens) {
        tokens = tokens.min(budget as usize);
    }

    let summarized = match thinking.and_then(|t| t.display.as_deref()) {
        Some("summarized") => true,
        Some(_) => false, // omitted / updates
        None => !omits_by_default(model),
    };
    let text = if summarized {
        summary_text((tokens / 10).clamp(8, 120))
    } else {
        String::new()
    };

    Some(ThinkingPlan {
        tokens,
        text,
        signature: random_signature(),
    })
}

/// Plausible thinking-summary prose of roughly `word_count` words.
fn summary_text(word_count: usize) -> String {
    const PHRASES: &[&str] = &[
        "Let me work through what the user is asking.",
        "First I should identify the key constraints.",
        "There are a few possible approaches here.",
        "Weighing them, the simplest one fits best.",
        "I should double-check the edge cases.",
        "That looks consistent, so I can write the answer.",
    ];
    let mut out = String::new();
    let mut words = 0;
    for phrase in PHRASES.iter().cycle() {
        if words >= word_count {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(phrase);
        words += phrase.split_whitespace().count();
    }
    out
}

/// An opaque base64-looking signature, like the ones the real API returns.
fn random_signature() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut rng = rand::rng();
    let body: String = (0..96)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect();
    format!("Eq{body}==")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(kind: &str, budget: Option<u32>, display: Option<&str>) -> ThinkingConfig {
        ThinkingConfig {
            thinking_type: kind.to_string(),
            budget_tokens: budget,
            display: display.map(str::to_string),
        }
    }

    fn effort(level: &str) -> OutputConfig {
        OutputConfig {
            effort: Some(level.to_string()),
            format: None,
        }
    }

    #[test]
    fn test_thinking_on_by_default_for_5x_models() {
        for model in [
            "claude-fable-5-1",
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "claude-haiku-5-5",
        ] {
            let p = plan(model, None, None, 100).expect(model);
            assert!(p.tokens > 0);
            assert!(p.text.is_empty(), "{model} should default to omitted");
            assert!(!p.signature.is_empty());
        }
    }

    #[test]
    fn test_no_thinking_by_default_for_4x_models() {
        assert!(plan("claude-opus-4-8", None, None, 100).is_none());
        assert!(plan("claude-sonnet-4-6", None, None, 100).is_none());
        assert!(plan("my-custom-model", None, None, 100).is_none());
    }

    #[test]
    fn test_adaptive_summarized_returns_text() {
        let t = cfg("adaptive", None, Some("summarized"));
        let p = plan("claude-opus-4-8", Some(&t), None, 100).unwrap();
        assert!(!p.text.is_empty());
    }

    #[test]
    fn test_older_models_default_to_summarized() {
        let t = cfg("enabled", Some(2048), None);
        let p = plan("claude-sonnet-4-5", Some(&t), None, 100).unwrap();
        assert!(!p.text.is_empty());
        assert!(p.tokens <= 2048);
    }

    #[test]
    fn test_disabled_and_between_tools_turn_thinking_off() {
        let t = cfg("disabled", None, None);
        assert!(plan("claude-opus-5", Some(&t), None, 100).is_none());
        let t = cfg("between_tools", None, None);
        assert!(plan("claude-sonnet-5-5", Some(&t), None, 100).is_none());
    }

    #[test]
    fn test_effort_scales_thinking_tokens() {
        let low = plan("claude-opus-5", None, Some(&effort("low")), 1000).unwrap();
        let max = plan("claude-opus-5", None, Some(&effort("max")), 1000).unwrap();
        assert!(max.tokens > low.tokens);
        // Opus 5.5 defaults to medium, Opus 5 to high.
        let o55 = plan("claude-opus-5-5", None, None, 1000).unwrap();
        let o5 = plan("claude-opus-5", None, None, 1000).unwrap();
        assert!(o5.tokens > o55.tokens);
    }

    #[test]
    fn test_validate_rejects_budget_tokens_on_new_models() {
        let t = cfg("enabled", Some(2048), None);
        assert!(validate("claude-fable-5-1", Some(4096), Some(&t), None).is_err());
        assert!(validate("claude-opus-4-8", Some(4096), Some(&t), None).is_err());
        assert!(validate("claude-sonnet-4-5", Some(4096), Some(&t), None).is_ok());
    }

    #[test]
    fn test_validate_budget_bounds() {
        let small = cfg("enabled", Some(512), None);
        assert!(validate("claude-sonnet-4-5", Some(4096), Some(&small), None).is_err());
        let too_big = cfg("enabled", Some(4096), None);
        assert!(validate("claude-sonnet-4-5", Some(4096), Some(&too_big), None).is_err());
        let missing = cfg("enabled", None, None);
        assert!(validate("claude-sonnet-4-5", Some(4096), Some(&missing), None).is_err());
    }

    #[test]
    fn test_validate_disabled() {
        let t = cfg("disabled", None, None);
        assert!(validate("claude-opus-5-5", None, Some(&t), None).is_err());
        assert!(validate("claude-fable-5", None, Some(&t), None).is_err());
        assert!(validate("claude-opus-5", None, Some(&t), None).is_ok());
    }

    #[test]
    fn test_validate_effort_values() {
        for level in EFFORT_LEVELS {
            assert!(validate("claude-opus-5-5", None, None, Some(&effort(level))).is_ok());
        }
        assert!(validate("claude-opus-5-5", None, None, Some(&effort("extreme"))).is_err());
    }

    #[test]
    fn test_validate_unknown_type() {
        let t = cfg("sometimes", None, None);
        assert!(validate("claude-opus-5-5", None, Some(&t), None).is_err());
    }
}
