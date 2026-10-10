// TypeSafe Model Catalog
// System One models from https://docs.typesafe.ai/models.
//
// Decision: `GET /v1/models` mirrors the real API, which lists the aliases
// (`jev-latest`, `jev-preview`) rather than versioned IDs. Versioned IDs are
// still accepted in requests, and a response always reports the versioned ID
// that answered, as the real API does.
//
// Decision: unknown model IDs are accepted and echoed back unchanged, like the
// other simulated providers, so tests can pin future `jev-*` releases without
// waiting for a simulator update.

use serde::{Deserialize, Serialize};

/// The model the aliases currently resolve to.
pub const JEV_CURRENT: &str = "jev-1.13.0";
/// Release date of [`JEV_CURRENT`] (from the dated `jev-1.13-20260917` snapshot).
pub const JEV_CURRENT_RELEASE_DATE: &str = "2026-09-17";
/// Default model used by the TypeSafe SDKs.
pub const TYPESAFE_DEFAULT_MODEL: &str = "jev-latest";

/// One entry of `GET /v1/models`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeSafeModel {
    /// Model name or alias accepted by a request's `model` field.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Release date, `YYYY-MM-DD`.
    pub release_date: String,
}

/// `GET /v1/models` response body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeSafeModelsResponse {
    pub models: Vec<TypeSafeModel>,
}

/// Models and aliases listed by `GET /typesafe/v1/models`.
pub fn typesafe_models() -> TypeSafeModelsResponse {
    TypeSafeModelsResponse {
        models: vec![
            TypeSafeModel {
                name: "jev-latest".to_string(),
                description: "Jev, TypeSafe's flagship System One model. \
                              Most recent stable release (currently Jev 1.13)."
                    .to_string(),
                release_date: JEV_CURRENT_RELEASE_DATE.to_string(),
            },
            TypeSafeModel {
                name: "jev-preview".to_string(),
                description: "Most recent Jev release, official or not \
                              (currently identical to jev-latest)."
                    .to_string(),
                release_date: JEV_CURRENT_RELEASE_DATE.to_string(),
            },
        ],
    }
}

/// Resolve a requested model name to the versioned ID reported in responses.
///
/// Aliases resolve to [`JEV_CURRENT`]; anything else is echoed unchanged.
pub fn resolve_typesafe_model(model: &str) -> &str {
    match model {
        "jev-latest" | "jev-preview" | "jev-1.13" => JEV_CURRENT,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_resolve_to_the_versioned_id() {
        assert_eq!(resolve_typesafe_model("jev-latest"), "jev-1.13.0");
        assert_eq!(resolve_typesafe_model("jev-preview"), "jev-1.13.0");
        assert_eq!(resolve_typesafe_model("jev-1.13.0"), "jev-1.13.0");
    }

    #[test]
    fn unknown_models_are_echoed() {
        assert_eq!(resolve_typesafe_model("jev-2.0.0"), "jev-2.0.0");
    }

    #[test]
    fn catalog_lists_the_aliases() {
        let names: Vec<_> = typesafe_models()
            .models
            .into_iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(names, ["jev-latest", "jev-preview"]);
    }
}
