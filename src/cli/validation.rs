// Request limits the real APIs enforce: context window, output cap, and
// (optionally) unknown models.
//
// Decisions:
// - Limits come from the model profiles (`openai::get_model_profile` for the
//   OpenAI-shaped endpoints, `anthropic::get_anthropic_model_profile` for
//   Messages). A model llmsim has no profile for is never limit-checked, so
//   custom and future model ids keep working.
// - Context and output caps are enforced by default (`[validation]
//   enforce_limits = true`): a request that would fail against the provider
//   should fail against the simulator. Rejecting unknown models is opt-in
//   (`strict_models`), because llmsim has always accepted any id.
// - Messages and error codes mirror each provider's wording
//   (`context_length_exceeded`, "prompt is too long: N tokens > M maximum").

use crate::openai::{ErrorDetail, ErrorResponse};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use super::config::ValidationConfig;

/// A request the provider would reject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitViolation {
    UnknownModel {
        model: String,
    },
    ContextLength {
        model: String,
        limit: u32,
        requested: u32,
    },
    MaxTokens {
        model: String,
        limit: u32,
        requested: u32,
    },
}

/// Profile limits for a model: (context window, max output tokens).
pub type Limits = Option<(u32, u32)>;

/// Check one request against `limits` (`None` = model unknown to llmsim).
pub fn check(
    config: &ValidationConfig,
    model: &str,
    limits: Limits,
    input_tokens: usize,
    max_tokens: Option<u32>,
) -> Result<(), LimitViolation> {
    // Scenario model ids are llmsim's own and always valid.
    if model.starts_with(crate::scenario::MODEL_PREFIX) {
        return Ok(());
    }
    let Some((context, max_output)) = limits else {
        return if config.strict_models {
            Err(LimitViolation::UnknownModel {
                model: model.to_string(),
            })
        } else {
            Ok(())
        };
    };
    if !config.enforce_limits {
        return Ok(());
    }
    if let Some(requested) = max_tokens {
        if requested > max_output {
            return Err(LimitViolation::MaxTokens {
                model: model.to_string(),
                limit: max_output,
                requested,
            });
        }
    }
    let requested = input_tokens as u32 + max_tokens.unwrap_or(0);
    if input_tokens as u32 > context || requested > context {
        return Err(LimitViolation::ContextLength {
            model: model.to_string(),
            limit: context,
            requested,
        });
    }
    Ok(())
}

/// Limits of an OpenAI-catalog model.
pub fn openai_limits(model: &str) -> Limits {
    crate::openai::get_model_profile(model).map(|p| (p.context_window, p.max_output_tokens))
}

/// Limits of an Anthropic-catalog model.
pub fn anthropic_limits(model: &str) -> Limits {
    crate::anthropic::get_anthropic_model_profile(model)
        .map(|p| (p.context_window, p.max_output_tokens))
}

/// The violation as an OpenAI error response.
pub fn openai_response(v: &LimitViolation, max_tokens_param: &str) -> Response {
    let (status, message, param, code) = match v {
        LimitViolation::UnknownModel { model } => (
            StatusCode::NOT_FOUND,
            format!("The model `{model}` does not exist or you do not have access to it."),
            None,
            "model_not_found",
        ),
        LimitViolation::ContextLength {
            limit, requested, ..
        } => (
            StatusCode::BAD_REQUEST,
            format!(
                "This model's maximum context length is {limit} tokens. However, you requested \
                 {requested} tokens. Please reduce the length of the input or the requested \
                 output."
            ),
            Some("messages"),
            "context_length_exceeded",
        ),
        LimitViolation::MaxTokens {
            limit, requested, ..
        } => (
            StatusCode::BAD_REQUEST,
            format!(
                "{max_tokens_param} is too large: {requested}. This model supports at most \
                 {limit} completion tokens, whereas you provided {requested}."
            ),
            Some(max_tokens_param),
            "invalid_value",
        ),
    };
    let body = ErrorResponse {
        error: ErrorDetail {
            message,
            error_type: "invalid_request_error".to_string(),
            param: param.map(str::to_string),
            code: Some(code.to_string()),
        },
    };
    (status, Json(body)).into_response()
}

/// Status and message for the Anthropic error envelope.
pub fn anthropic_parts(v: &LimitViolation) -> (u16, String) {
    match v {
        LimitViolation::UnknownModel { model } => (404, format!("model: {model}")),
        LimitViolation::ContextLength {
            limit, requested, ..
        } => (
            400,
            format!("prompt is too long: {requested} tokens > {limit} maximum"),
        ),
        LimitViolation::MaxTokens {
            model,
            limit,
            requested,
        } => (
            400,
            format!(
                "max_tokens: {requested} > {limit}, which is the maximum allowed number of \
                 output tokens for {model}"
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> ValidationConfig {
        ValidationConfig::default()
    }

    #[test]
    fn within_limits_passes() {
        assert!(check(&on(), "m", Some((1000, 100)), 500, Some(100)).is_ok());
        assert!(check(&on(), "m", Some((1000, 100)), 900, None).is_ok());
    }

    #[test]
    fn over_context_or_output_fails() {
        assert!(matches!(
            check(&on(), "m", Some((1000, 100)), 1001, None),
            Err(LimitViolation::ContextLength {
                requested: 1001,
                ..
            })
        ));
        assert!(matches!(
            check(&on(), "m", Some((1000, 100)), 950, Some(100)),
            Err(LimitViolation::ContextLength {
                requested: 1050,
                ..
            })
        ));
        assert!(matches!(
            check(&on(), "m", Some((1000, 100)), 10, Some(101)),
            Err(LimitViolation::MaxTokens { limit: 100, .. })
        ));
    }

    #[test]
    fn unknown_models_pass_unless_strict() {
        assert!(check(&on(), "my-model", None, 10_000_000, Some(u32::MAX)).is_ok());
        let strict = ValidationConfig {
            strict_models: true,
            ..on()
        };
        assert!(matches!(
            check(&strict, "my-model", None, 1, None),
            Err(LimitViolation::UnknownModel { .. })
        ));
        assert!(check(&strict, "llmsim-scenario-chat-short", None, 1, None).is_ok());
    }

    #[test]
    fn limits_can_be_disabled() {
        let off = ValidationConfig {
            enforce_limits: false,
            ..on()
        };
        assert!(check(&off, "m", Some((10, 10)), 1_000, Some(1_000)).is_ok());
    }
}
