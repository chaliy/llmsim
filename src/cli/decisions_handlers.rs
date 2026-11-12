// Decisions API handlers: OpenAI and Gemini typed-judgment endpoints, plus
// Microsoft Foundry's System One route.
//
// - `POST /openai/v1/decisions` follows OpenAI's Decisions API (public beta,
//   served by GPT-6 Luna): `input` text and an array of named `predicate`,
//   `choice` and `score` questions; answers come back as an array in the same
//   order, carrying the question's name.
// - `POST /gemini/v1beta/decisions` follows Gemini Decision 1: System One's
//   primitives (`noul`, `choice`, `score`) under a Google envelope (`input`,
//   `true_description`/`false_description`, `total_input_tokens`).
// - `POST /foundry/providers/microsoft/v1/systemone` is TypeSafe's System One
//   wire as Microsoft Foundry serves it at a resource root, so a client's
//   base URL is `{llmsim}/foundry`.
//
// Decisions:
// - All three translate into a System One request and reuse its answer
//   synthesis, so the same input and question always get the same answer,
//   whichever provider shape asked.
// - Validation mirrors limits the providers enforce (probed by everruns'
//   drivers): OpenAI wants unique names, at least two choices and at most ten
//   score levels; Gemini wants keys matching `[A-Za-z0-9_-]{1,64}`. Errors
//   use each provider's own error envelope.
// - `[[llmsim:refuse]]` in an OpenAI input makes every answer a `refusal`
//   (HTTP 200), so clients can test the path where the model declines.

use super::state::AppState;
use super::typesafe_handlers::evaluate;
use crate::typesafe::{Answer, Question, QuestionKind, SystemOneRequest};
use crate::EndpointType;
use axum::{
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

/// Model the OpenAI Decisions API serves when a request names none.
pub const OPENAI_DECISIONS_DEFAULT_MODEL: &str = "gpt-6-luna";
/// Model Gemini's Decisions API serves when a request names none.
pub const GEMINI_DECISIONS_DEFAULT_MODEL: &str = "gemini-decision-1";
/// Most questions one OpenAI decisions request may carry.
pub const OPENAI_MAX_QUESTIONS: usize = 24;
/// Most score levels an OpenAI score question may carry.
pub const OPENAI_MAX_SCORE_LEVELS: usize = 10;
/// Most questions one Gemini decisions request may carry.
pub const GEMINI_MAX_QUESTIONS: usize = 20;
/// Score levels Gemini accepts.
pub const GEMINI_MAX_SCORE_LEVELS: usize = 255;
/// Input marker that makes the OpenAI endpoint refuse every question.
pub const REFUSE_MARKER: &str = "[[llmsim:refuse]]";

fn record_rejected(state: &AppState, model: &str, status: u16) {
    state
        .stats
        .record_request_start(model, false, EndpointType::SystemOne);
    state.stats.record_error(status);
}

// ---------------------------------------------------------------------------
// OpenAI
// ---------------------------------------------------------------------------

fn openai_error_type(status: u16) -> &'static str {
    match status {
        400 | 422 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        429 => "rate_limit_error",
        _ => "server_error",
    }
}

fn openai_error(status: u16, message: String) -> Response {
    let body = json!({"error": {
        "message": message,
        "type": openai_error_type(status),
        "param": null,
        "code": null,
    }});
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (code, Json(body)).into_response()
}

fn text_of(value: &Value) -> Option<String> {
    value.as_str().map(str::to_string)
}

/// Parse an OpenAI decisions body into a System One request, keeping the
/// question order for the answer array.
fn parse_openai(raw: &Value) -> Result<(SystemOneRequest, Vec<String>), String> {
    let obj = raw
        .as_object()
        .ok_or("Request body must be a JSON object")?;
    let model = match obj.get("model") {
        None | Some(Value::Null) => OPENAI_DECISIONS_DEFAULT_MODEL.to_string(),
        Some(Value::String(m)) if !m.trim().is_empty() => m.clone(),
        _ => return Err("'model' must be a non-empty string".into()),
    };
    let input = obj
        .get("input")
        .and_then(Value::as_str)
        .ok_or("'input' is required and must be a string")?;
    let questions = obj
        .get("questions")
        .and_then(Value::as_array)
        .ok_or("'questions' is required and must be an array")?;
    if questions.is_empty() {
        return Err("'questions' must contain at least one question".into());
    }
    if questions.len() > OPENAI_MAX_QUESTIONS {
        return Err(format!(
            "'questions' may contain at most {OPENAI_MAX_QUESTIONS} questions"
        ));
    }
    let mut order = Vec::with_capacity(questions.len());
    let mut parsed = BTreeMap::new();
    for (i, q) in questions.iter().enumerate() {
        let name = q
            .get("name")
            .and_then(Value::as_str)
            .filter(|n| !n.is_empty())
            .ok_or_else(|| format!("questions[{i}].name is required"))?;
        if parsed.contains_key(name) {
            return Err(format!("Duplicate question name '{name}'"));
        }
        let instructions = q.get("instructions").and_then(text_of);
        let kind = match q.get("type").and_then(Value::as_str) {
            Some("predicate") => QuestionKind::Noul { criteria: None },
            Some("choice") => {
                let choices = q
                    .get("choices")
                    .and_then(Value::as_array)
                    .ok_or_else(|| format!("questions[{i}].choices is required"))?;
                if choices.len() < 2 {
                    return Err(format!("questions[{i}].choices needs at least 2 choices"));
                }
                let mut options = Vec::new();
                for c in choices {
                    let value = c
                        .get("value")
                        .and_then(Value::as_str)
                        .ok_or_else(|| format!("questions[{i}].choices[].value is required"))?;
                    let about = c.get("description").cloned().unwrap_or(Value::Null);
                    options.push((value.to_string(), about));
                }
                QuestionKind::Choice { options }
            }
            Some("score") => {
                let levels = q
                    .get("levels")
                    .and_then(Value::as_array)
                    .ok_or_else(|| format!("questions[{i}].levels is required"))?;
                if levels.len() < 2 || levels.len() > OPENAI_MAX_SCORE_LEVELS {
                    return Err(format!(
                        "questions[{i}].levels needs 2 to {OPENAI_MAX_SCORE_LEVELS} levels"
                    ));
                }
                let mut out = Vec::new();
                for l in levels {
                    let label = l
                        .get("label")
                        .and_then(Value::as_str)
                        .ok_or_else(|| format!("questions[{i}].levels[].label is required"))?;
                    out.push(Value::String(label.to_string()));
                }
                QuestionKind::Score { levels: out }
            }
            other => {
                return Err(format!(
                    "questions[{i}].type must be 'predicate', 'choice' or 'score', got {}",
                    other.map_or("nothing".to_string(), |t| format!("'{t}'"))
                ))
            }
        };
        order.push(name.to_string());
        parsed.insert(
            name.to_string(),
            Question {
                instructions: instructions.map(Value::String),
                kind,
            },
        );
    }
    Ok((
        SystemOneRequest {
            state: Value::String(input.to_string()),
            model,
            questions: parsed,
        },
        order,
    ))
}

fn openai_answer(name: &str, answer: &Answer) -> Value {
    match answer {
        Answer::Noul { noul } => json!({"type": "predicate", "name": name, "probability": noul}),
        Answer::Choice {
            choice,
            confidence,
            probabilities,
        } => json!({
            "type": "choice",
            "name": name,
            "choice": choice,
            "confidence": confidence,
            "probabilities": probabilities
                .iter()
                .map(|(value, p)| json!({"value": value, "probability": p}))
                .collect::<Vec<_>>(),
        }),
        Answer::Score {
            score,
            confidence,
            probabilities,
            ..
        } => {
            let mut levels: Vec<(usize, f64)> = probabilities
                .iter()
                .filter_map(|(k, p)| k.parse().ok().map(|i| (i, *p)))
                .collect();
            levels.sort_by_key(|(i, _)| *i);
            json!({
                "type": "score",
                "name": name,
                "score": score,
                "confidence": confidence,
                "probabilities": levels
                    .iter()
                    .map(|(value, p)| json!({"value": value, "probability": p}))
                    .collect::<Vec<_>>(),
            })
        }
    }
}

/// POST /openai/v1/decisions
pub async fn openai_decisions(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let request_start = Instant::now();
    let raw: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            record_rejected(&state, "unknown", 400);
            return openai_error(400, format!("Invalid JSON body: {e}"));
        }
    };
    let (request, order) = match parse_openai(&raw) {
        Ok(parsed) => parsed,
        Err(message) => {
            record_rejected(&state, raw["model"].as_str().unwrap_or("unknown"), 400);
            return openai_error(400, message);
        }
    };
    tracing::info!(model = %request.model, questions = order.len(), "OpenAI decisions request");
    let (answers, usage) = match evaluate(&state, &request, request_start, openai_error).await {
        Ok(result) => result,
        Err(response) => return response,
    };
    let refuse = request
        .state
        .as_str()
        .is_some_and(|s| s.contains(REFUSE_MARKER));
    let answers: Vec<Value> = order
        .iter()
        .map(|name| {
            if refuse {
                json!({"type": "refusal", "name": name})
            } else {
                openai_answer(name, &answers[name])
            }
        })
        .collect();
    Json(json!({
        "model": request.model,
        "answers": answers,
        "usage": {"input_tokens": usage.input_tokens, "output_tokens": usage.output_tokens},
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Gemini
// ---------------------------------------------------------------------------

fn gemini_status(status: u16) -> &'static str {
    match status {
        400 | 422 => "INVALID_ARGUMENT",
        401 => "UNAUTHENTICATED",
        403 => "PERMISSION_DENIED",
        404 => "NOT_FOUND",
        429 => "RESOURCE_EXHAUSTED",
        503 | 529 => "UNAVAILABLE",
        504 => "DEADLINE_EXCEEDED",
        _ => "INTERNAL",
    }
}

fn gemini_error(status: u16, message: String) -> Response {
    let body =
        json!({"error": {"code": status, "message": message, "status": gemini_status(status)}});
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (code, Json(body)).into_response()
}

fn valid_gemini_key(key: &str) -> bool {
    (1..=64).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The text a Gemini `input` bills and seeds answers as: a string, or the
/// text parts of a Content array (media parts are counted by type only).
fn gemini_input_text(input: &Value) -> Result<String, String> {
    match input {
        Value::String(s) => Ok(s.clone()),
        Value::Array(parts) if !parts.is_empty() => {
            let mut text = Vec::new();
            for (i, part) in parts.iter().enumerate() {
                match part.get("type").and_then(Value::as_str) {
                    Some("text") => text.push(
                        part.get("text")
                            .and_then(Value::as_str)
                            .ok_or_else(|| format!("input[{i}].text is required"))?
                            .to_string(),
                    ),
                    Some(kind @ ("image" | "audio" | "video" | "document")) => {
                        if part.get("data").is_none() && part.get("uri").is_none() {
                            return Err(format!("input[{i}] needs 'data' or 'uri'"));
                        }
                        text.push(format!("[{kind}]"));
                    }
                    _ => {
                        return Err(format!(
                            "input[{i}].type must be text, image, audio, video or document"
                        ))
                    }
                }
            }
            Ok(text.join("\n"))
        }
        _ => Err("'input' must be a string or a non-empty array of parts".into()),
    }
}

fn parse_gemini(raw: &Value) -> Result<SystemOneRequest, String> {
    let obj = raw
        .as_object()
        .ok_or("Request body must be a JSON object")?;
    let model = match obj.get("model") {
        None | Some(Value::Null) => GEMINI_DECISIONS_DEFAULT_MODEL.to_string(),
        Some(Value::String(m)) if !m.trim().is_empty() => {
            m.trim_start_matches("models/").to_string()
        }
        _ => return Err("'model' must be a non-empty string".into()),
    };
    let input = gemini_input_text(obj.get("input").unwrap_or(&Value::Null))?;
    let questions = obj
        .get("questions")
        .and_then(Value::as_object)
        .ok_or("'questions' is required and must be an object")?;
    if questions.is_empty() || questions.len() > GEMINI_MAX_QUESTIONS {
        return Err(format!(
            "'questions' must contain 1 to {GEMINI_MAX_QUESTIONS} questions"
        ));
    }
    let mut parsed = BTreeMap::new();
    for (key, q) in questions {
        if !valid_gemini_key(key) {
            return Err(format!(
                "Question key '{key}' must match [A-Za-z0-9_-]{{1,64}}"
            ));
        }
        let instructions = q.get("instructions").cloned().filter(|v| !v.is_null());
        let criteria = q.get("criteria");
        let kind = match q.get("type").and_then(Value::as_str) {
            Some("noul") => {
                let criteria = match criteria {
                    None | Some(Value::Null) => None,
                    Some(Value::Object(c)) => {
                        let mut out = Map::new();
                        for (from, to) in
                            [("true_description", "true"), ("false_description", "false")]
                        {
                            if let Some(v) = c.get(from) {
                                out.insert(to.to_string(), v.clone());
                            }
                        }
                        Some(out)
                    }
                    _ => return Err(format!("questions.{key}.criteria must be an object")),
                };
                QuestionKind::Noul { criteria }
            }
            Some("choice") => {
                let options = criteria
                    .and_then(Value::as_object)
                    .filter(|c| c.len() >= 2)
                    .ok_or_else(|| {
                        format!("questions.{key}.criteria must map at least 2 options")
                    })?;
                QuestionKind::Choice {
                    options: options
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                }
            }
            Some("score") => {
                let levels = criteria
                    .and_then(Value::as_array)
                    .filter(|l| (2..=GEMINI_MAX_SCORE_LEVELS).contains(&l.len()))
                    .ok_or_else(|| {
                        format!(
                            "questions.{key}.criteria must list 2 to {GEMINI_MAX_SCORE_LEVELS} levels"
                        )
                    })?;
                QuestionKind::Score {
                    levels: levels.clone(),
                }
            }
            _ => {
                return Err(format!(
                    "questions.{key}.type must be 'noul', 'choice' or 'score'"
                ))
            }
        };
        parsed.insert(key.clone(), Question { instructions, kind });
    }
    Ok(SystemOneRequest {
        state: Value::String(input),
        model,
        questions: parsed,
    })
}

fn gemini_answer(answer: &Answer) -> Value {
    match answer {
        // Gemini also reports a noul's confidence: how far the probability
        // sits from a coin flip (0.978 for a 0.978 yes, measured live).
        Answer::Noul { noul } => {
            json!({"type": "noul", "noul": noul, "confidence": noul.max(1.0 - noul)})
        }
        other => serde_json::to_value(other).unwrap_or(Value::Null),
    }
}

/// POST /gemini/v1beta/decisions
pub async fn gemini_decisions(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let request_start = Instant::now();
    let raw: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            record_rejected(&state, "unknown", 400);
            return gemini_error(400, format!("Invalid JSON payload received. {e}"));
        }
    };
    let request = match parse_gemini(&raw) {
        Ok(r) => r,
        Err(message) => {
            record_rejected(&state, raw["model"].as_str().unwrap_or("unknown"), 400);
            return gemini_error(400, message);
        }
    };
    tracing::info!(model = %request.model, questions = request.questions.len(), "Gemini decisions request");
    let (answers, usage) = match evaluate(&state, &request, request_start, gemini_error).await {
        Ok(result) => result,
        Err(response) => return response,
    };
    let answers: Map<String, Value> = answers
        .iter()
        .map(|(k, a)| (k.clone(), gemini_answer(a)))
        .collect();
    Json(json!({
        "answers": answers,
        "usage": {
            "total_tokens": usage.input_tokens,
            "total_input_tokens": usage.input_tokens,
            "input_tokens_by_modality": [{"modality": "text", "tokens": usage.input_tokens}],
            "total_output_tokens": 0,
        },
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_body_keeps_question_order() {
        let (req, order) = parse_openai(&json!({
            "input": "x",
            "questions": [
                {"type": "score", "name": "z", "instructions": "?", "levels": [{"label": "lo"}, {"label": "hi"}]},
                {"type": "predicate", "name": "a", "instructions": "?"}
            ]
        }))
        .unwrap();
        assert_eq!(order, ["z", "a"]);
        assert_eq!(req.model, OPENAI_DECISIONS_DEFAULT_MODEL);
    }

    #[test]
    fn openai_limits_are_enforced() {
        let one_choice = json!({"input": "x", "questions": [
            {"type": "choice", "name": "c", "instructions": "?", "choices": [{"value": "a"}]}
        ]});
        assert!(parse_openai(&one_choice)
            .unwrap_err()
            .contains("at least 2"));
        let dup = json!({"input": "x", "questions": [
            {"type": "predicate", "name": "p", "instructions": "?"},
            {"type": "predicate", "name": "p", "instructions": "?"}
        ]});
        assert!(parse_openai(&dup).unwrap_err().contains("Duplicate"));
        let levels: Vec<Value> = (0..11).map(|i| json!({"label": i.to_string()})).collect();
        let many = json!({"input": "x", "questions": [
            {"type": "score", "name": "s", "instructions": "?", "levels": levels}
        ]});
        assert!(parse_openai(&many).unwrap_err().contains("levels"));
        assert!(parse_openai(&json!({"questions": []})).is_err());
    }

    #[test]
    fn gemini_keys_and_criteria_are_validated() {
        let bad_key = json!({"input": "x", "questions": {"is urgent?": {"type": "noul"}}});
        assert!(parse_gemini(&bad_key).unwrap_err().contains("must match"));
        let ok = parse_gemini(&json!({
            "model": "models/gemini-decision-1",
            "input": [{"type": "text", "text": "hi"}, {"type": "image", "mime_type": "image/png", "data": "AA=="}],
            "questions": {"q0": {"type": "noul", "criteria": {"true_description": "yes"}}}
        }))
        .unwrap();
        assert_eq!(ok.model, "gemini-decision-1");
        assert_eq!(ok.state, json!("hi\n[image]"));
        let QuestionKind::Noul { criteria } = &ok.questions["q0"].kind else {
            panic!("noul")
        };
        assert_eq!(criteria.as_ref().unwrap()["true"], "yes");
    }
}
