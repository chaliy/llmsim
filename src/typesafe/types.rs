// TypeSafe System One wire types
// Mirrors the official OpenAPI schema (https://api.typesafe.ai/openapi.json).
//
// Decision: requests are parsed from raw JSON and validated by hand rather
// than through serde derives, so a malformed request gets the same FastAPI
// `422 {"detail": [{"type", "loc", "msg", "input"}]}` body the real API
// returns, with `loc` pointing at the offending field. Clients (and the
// official SDKs) surface that path to the user.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Most options a choice question may carry (<https://docs.typesafe.ai/api>).
pub const MAX_CHOICE_OPTIONS: usize = 255;
/// Most levels a score question may carry (<https://docs.typesafe.ai/api>).
pub const MAX_SCORE_LEVELS: usize = 10;

/// A validated `POST /v1/systemone` request.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemOneRequest {
    /// The content the questions refer to: a string, object, or array.
    pub state: Value,
    /// Model name or alias.
    pub model: String,
    /// Questions keyed by caller-chosen names.
    pub questions: BTreeMap<String, Question>,
}

/// One typed question.
#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    /// Optional instructions: string, object, array, or absent.
    pub instructions: Option<Value>,
    pub kind: QuestionKind,
}

/// The three System One primitives.
#[derive(Debug, Clone, PartialEq)]
pub enum QuestionKind {
    /// Yes/no, answered as the probability of yes.
    Noul {
        /// Optional `{"true": ..., "false": ...}` descriptions.
        criteria: Option<Map<String, Value>>,
    },
    /// One option out of a set, keyed by option name.
    Choice { options: Vec<(String, Value)> },
    /// A position along ordered levels, lowest first.
    Score { levels: Vec<Value> },
}

impl QuestionKind {
    /// The primitive name as it appears on the wire.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Noul { .. } => "noul",
            Self::Choice { .. } => "choice",
            Self::Score { .. } => "score",
        }
    }
}

/// One FastAPI-style validation error entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationErrorItem {
    /// Machine-readable error code, e.g. `missing`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Path to the invalid value, starting with `body`.
    pub loc: Vec<Value>,
    /// Human-readable explanation.
    pub msg: String,
    /// The value that failed validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
}

/// `422` response body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HttpValidationError {
    pub detail: Vec<ValidationErrorItem>,
}

/// Non-validation error body, e.g. `401`/`429`:
/// `{"detail": {"error_type": "...", "message": "..."}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeSafeErrorResponse {
    pub detail: TypeSafeErrorDetail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeSafeErrorDetail {
    pub error_type: String,
    pub message: String,
}

impl TypeSafeErrorResponse {
    pub fn new(error_type: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            detail: TypeSafeErrorDetail {
                error_type: error_type.into(),
                message: message.into(),
            },
        }
    }

    /// The `error_type` the API uses for a status code.
    pub fn type_for_status(status: u16) -> &'static str {
        match status {
            400 => "invalid_request_error",
            401 => "authentication_error",
            403 => "permission_error",
            404 => "not_found_error",
            429 => "rate_limit_error",
            529 => "overloaded_error",
            _ => "api_error",
        }
    }
}

/// One answer, matching the type of its question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    Noul {
        /// Probability of yes, 0..=1.
        noul: f64,
    },
    Choice {
        /// The highest-probability option.
        choice: String,
        /// Concentration of the distribution, 0..=1.
        confidence: f64,
        /// Every option mapped to its probability; sums to 1.
        probabilities: BTreeMap<String, f64>,
    },
    Score {
        /// Probability-weighted level; may fall between levels.
        score: f64,
        /// Concentration of the distribution, 0..=1.
        confidence: f64,
        /// Level index (as a string) to its description.
        legend: BTreeMap<String, Value>,
        /// Level index (as a string) to its probability; sums to 1.
        probabilities: BTreeMap<String, f64>,
    },
}

/// Token usage. Output tokens are free on the real API.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemOneUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// `POST /v1/systemone` response body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemOneResponse {
    /// The versioned model that answered.
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
    pub usage: SystemOneUsage,
}

/// Is `value` one of the JSON shapes the API accepts as free-form content
/// (`state`, `instructions`, criteria descriptions)?
fn is_content(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Object(_) | Value::Array(_))
}

fn loc(path: &[&str]) -> Vec<Value> {
    std::iter::once("body")
        .chain(path.iter().copied())
        .map(|s| Value::String(s.to_string()))
        .collect()
}

fn err(
    kind: &str,
    path: &[&str],
    msg: impl Into<String>,
    input: Option<&Value>,
) -> ValidationErrorItem {
    ValidationErrorItem {
        kind: kind.to_string(),
        loc: loc(path),
        msg: msg.into(),
        input: input.cloned(),
    }
}

impl SystemOneRequest {
    /// Parse and validate a request body, collecting every validation error
    /// the way FastAPI does instead of stopping at the first.
    pub fn parse(body: &Value) -> Result<Self, HttpValidationError> {
        let mut errors = Vec::new();
        let Some(obj) = body.as_object() else {
            return Err(HttpValidationError {
                detail: vec![err(
                    "model_attributes_type",
                    &[],
                    "Input should be a valid dictionary or object to extract fields from",
                    Some(body),
                )],
            });
        };

        let state = match obj.get("state") {
            None => {
                errors.push(err("missing", &["state"], "Field required", Some(body)));
                None
            }
            Some(v) if is_content(v) => Some(v.clone()),
            Some(v) => {
                errors.push(err(
                    "content_type",
                    &["state"],
                    "Input should be a valid string, dictionary or list",
                    Some(v),
                ));
                None
            }
        };

        let model = match obj.get("model") {
            None => {
                errors.push(err("missing", &["model"], "Field required", Some(body)));
                None
            }
            Some(Value::String(s)) => Some(s.clone()),
            Some(v) => {
                errors.push(err(
                    "string_type",
                    &["model"],
                    "Input should be a valid string",
                    Some(v),
                ));
                None
            }
        };

        let mut questions = BTreeMap::new();
        match obj.get("questions") {
            None => errors.push(err("missing", &["questions"], "Field required", Some(body))),
            Some(Value::Object(map)) if map.is_empty() => errors.push(err(
                "too_short",
                &["questions"],
                "Dictionary should have at least 1 item after validation, not 0",
                Some(&Value::Object(map.clone())),
            )),
            Some(Value::Object(map)) => {
                for (name, raw) in map {
                    if let Some(q) = parse_question(name, raw, &mut errors) {
                        questions.insert(name.clone(), q);
                    }
                }
            }
            Some(v) => errors.push(err(
                "dict_type",
                &["questions"],
                "Input should be a valid dictionary",
                Some(v),
            )),
        }

        if !errors.is_empty() {
            return Err(HttpValidationError { detail: errors });
        }
        Ok(Self {
            state: state.expect("validated"),
            model: model.expect("validated"),
            questions,
        })
    }
}

fn parse_question(
    name: &str,
    raw: &Value,
    errors: &mut Vec<ValidationErrorItem>,
) -> Option<Question> {
    let Some(obj) = raw.as_object() else {
        errors.push(err(
            "model_attributes_type",
            &["questions", name],
            "Input should be a valid dictionary or object to extract fields from",
            Some(raw),
        ));
        return None;
    };
    let kind = match obj.get("type").and_then(Value::as_str) {
        Some(k @ ("noul" | "choice" | "score")) => k,
        Some(_) => {
            errors.push(err(
                "union_tag_invalid",
                &["questions", name],
                "Input tag found using 'type' does not match any of the expected tags: 'noul', 'choice', 'score'",
                Some(raw),
            ));
            return None;
        }
        None => {
            errors.push(err(
                "union_tag_not_found",
                &["questions", name],
                "Unable to extract tag using discriminator 'type'",
                Some(raw),
            ));
            return None;
        }
    };
    let before = errors.len();
    let path = |field: &'static str| -> [&str; 4] { ["questions", name, kind, field] };

    let instructions = match obj.get("instructions") {
        None | Some(Value::Null) => None,
        Some(v) if is_content(v) => Some(v.clone()),
        Some(v) => {
            errors.push(err(
                "content_type",
                &path("instructions"),
                "Input should be a valid string, dictionary or list",
                Some(v),
            ));
            None
        }
    };

    let question_kind = match kind {
        "noul" => {
            let criteria = match obj.get("criteria") {
                None | Some(Value::Null) => None,
                Some(Value::Object(map)) => {
                    for key in ["true", "false"] {
                        if let Some(v) = map.get(key) {
                            if !v.is_null() && !is_content(v) {
                                errors.push(err(
                                    "content_type",
                                    &["questions", name, kind, "criteria", key],
                                    "Input should be a valid string, dictionary or list",
                                    Some(v),
                                ));
                            }
                        }
                    }
                    Some(map.clone())
                }
                Some(v) => {
                    errors.push(err(
                        "model_type",
                        &path("criteria"),
                        "Input should be a valid dictionary or instance of NoulCriteria",
                        Some(v),
                    ));
                    None
                }
            };
            let described = criteria
                .as_ref()
                .is_some_and(|c| c.values().any(|v| !v.is_null()));
            if instructions.is_none() && !described {
                errors.push(err(
                    "value_error",
                    &["questions", name, kind],
                    "Value error, a noul question needs instructions or criteria",
                    Some(raw),
                ));
            }
            QuestionKind::Noul { criteria }
        }
        "choice" => match obj.get("criteria") {
            None => {
                errors.push(err(
                    "missing",
                    &path("criteria"),
                    "Field required",
                    Some(raw),
                ));
                QuestionKind::Choice { options: vec![] }
            }
            Some(Value::Object(map)) => {
                if map.is_empty() {
                    errors.push(err(
                        "too_short",
                        &path("criteria"),
                        "Dictionary should have at least 1 item after validation, not 0",
                        Some(&Value::Object(map.clone())),
                    ));
                } else if map.len() > MAX_CHOICE_OPTIONS {
                    errors.push(err(
                        "too_long",
                        &path("criteria"),
                        format!(
                            "Dictionary should have at most {MAX_CHOICE_OPTIONS} items after validation, not {}",
                            map.len()
                        ),
                        None,
                    ));
                }
                for (option, v) in map {
                    if !v.is_null() && !is_content(v) {
                        errors.push(err(
                            "content_type",
                            &["questions", name, kind, "criteria", option],
                            "Input should be a valid string, dictionary or list",
                            Some(v),
                        ));
                    }
                }
                QuestionKind::Choice {
                    options: map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                }
            }
            Some(v) => {
                errors.push(err(
                    "dict_type",
                    &path("criteria"),
                    "Input should be a valid dictionary",
                    Some(v),
                ));
                QuestionKind::Choice { options: vec![] }
            }
        },
        _ => match obj.get("criteria") {
            None => {
                errors.push(err(
                    "missing",
                    &path("criteria"),
                    "Field required",
                    Some(raw),
                ));
                QuestionKind::Score { levels: vec![] }
            }
            Some(Value::Array(levels)) => {
                if levels.is_empty() {
                    errors.push(err(
                        "too_short",
                        &path("criteria"),
                        "List should have at least 1 item after validation, not 0",
                        Some(&Value::Array(levels.clone())),
                    ));
                } else if levels.len() > MAX_SCORE_LEVELS {
                    errors.push(err(
                        "too_long",
                        &path("criteria"),
                        format!(
                            "List should have at most {MAX_SCORE_LEVELS} items after validation, not {}",
                            levels.len()
                        ),
                        Some(&Value::Array(levels.clone())),
                    ));
                }
                for (i, v) in levels.iter().enumerate() {
                    if !is_content(v) {
                        let index = i.to_string();
                        errors.push(err(
                            "content_type",
                            &["questions", name, kind, "criteria", &index],
                            "Input should be a valid string, dictionary or list",
                            Some(v),
                        ));
                    }
                }
                QuestionKind::Score {
                    levels: levels.clone(),
                }
            }
            Some(v) => {
                errors.push(err(
                    "list_type",
                    &path("criteria"),
                    "Input should be a valid list",
                    Some(v),
                ));
                QuestionKind::Score { levels: vec![] }
            }
        },
    };

    (errors.len() == before).then_some(Question {
        instructions,
        kind: question_kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn locs(e: &HttpValidationError) -> Vec<String> {
        e.detail
            .iter()
            .map(|d| {
                d.loc
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect::<Vec<_>>()
                    .join(".")
            })
            .collect()
    }

    #[test]
    fn parses_all_three_primitives() {
        let req = SystemOneRequest::parse(&json!({
            "state": {"message": "help"},
            "model": "jev-latest",
            "questions": {
                "a": {"type": "noul", "instructions": "Urgent?"},
                "b": {"type": "choice", "criteria": {"x": "X", "y": null}},
                "c": {"type": "score", "instructions": {"task": "rate"}, "criteria": ["lo", "hi"]}
            }
        }))
        .unwrap();
        assert_eq!(req.questions.len(), 3);
        assert_eq!(req.questions["b"].kind.name(), "choice");
        assert!(req.questions["b"].instructions.is_none());
    }

    #[test]
    fn missing_fields_are_all_reported() {
        let e = SystemOneRequest::parse(&json!({})).unwrap_err();
        assert_eq!(locs(&e), ["body.state", "body.model", "body.questions"]);
        assert!(e.detail.iter().all(|d| d.kind == "missing"));
    }

    #[test]
    fn rejects_non_text_state_and_empty_questions() {
        let e = SystemOneRequest::parse(&json!({"state": 5, "model": "m", "questions": {}}))
            .unwrap_err();
        assert_eq!(locs(&e), ["body.state", "body.questions"]);
    }

    #[test]
    fn rejects_bad_question_shapes() {
        let e = SystemOneRequest::parse(&json!({
            "state": "s",
            "model": "m",
            "questions": {
                "tag": {"type": "maybe"},
                "notag": {"instructions": "x"},
                "bare": {"type": "noul"},
                "nochoice": {"type": "choice", "criteria": {}},
                "noscore": {"type": "score"},
                "toomany": {"type": "score", "criteria": ["0","1","2","3","4","5","6","7","8","9","10"]},
                "nulllevel": {"type": "score", "criteria": ["a", null]}
            }
        }))
        .unwrap_err();
        let l = locs(&e);
        assert!(l.contains(&"body.questions.tag".to_string()));
        assert!(l.contains(&"body.questions.notag".to_string()));
        assert!(l.contains(&"body.questions.bare.noul".to_string()));
        assert!(l.contains(&"body.questions.nochoice.choice.criteria".to_string()));
        assert!(l.contains(&"body.questions.noscore.score.criteria".to_string()));
        assert!(l.contains(&"body.questions.toomany.score.criteria".to_string()));
        assert!(l.contains(&"body.questions.nulllevel.score.criteria.1".to_string()));
    }

    #[test]
    fn noul_with_only_criteria_is_valid() {
        SystemOneRequest::parse(&json!({
            "state": "s", "model": "m",
            "questions": {"q": {"type": "noul", "criteria": {"true": "spam"}}}
        }))
        .unwrap();
    }

    #[test]
    fn answers_serialize_with_type_tags() {
        let v = serde_json::to_value(Answer::Noul { noul: 0.5 }).unwrap();
        assert_eq!(v, json!({"type": "noul", "noul": 0.5}));
    }
}
