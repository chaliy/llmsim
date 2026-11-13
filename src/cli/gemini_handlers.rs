// Gemini native API (Google AI `v1beta`): generateContent,
// streamGenerateContent, countTokens, embedContent, batchEmbedContents and
// the models list, under `/gemini/v1beta/models/...`.
//
// Decisions:
// - Bodies are read as JSON values, accepting both camelCase and snake_case
//   field names, as Google's protobuf JSON parser does.
// - Thinking models (2.5 except Flash-Lite, 3.x, 4.x) think by default and
//   bill `thoughtsTokenCount`. Thinking counts against `maxOutputTokens`, so a
//   small cap can leave no room for the answer: the candidate then has no
//   parts and `finishReason: MAX_TOKENS`, the trap real clients fall into.
// - `responseSchema` uses OpenAPI-style upper-case types (`OBJECT`); they are
//   lower-cased before generating schema-valid JSON.
// - Streaming sends a few words per chunk (Gemini streams in larger chunks
//   than OpenAI), as SSE with `alt=sse` and as one JSON array otherwise.
// - Implicit caching reports `cachedContentTokenCount` for a repeated prefix
//   of at least 1024 tokens (4096 on Pro models).
// - `gemini-embedding-001` vectors shorter than 3072 are a prefix and not
//   renormalized, as Google documents.

use super::decisions_handlers::gemini_error;
use super::state::AppState;
use crate::{create_generator, EndpointType, ErrorInjector};
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Words per streamed chunk.
const CHUNK_WORDS: usize = 12;
/// Token cost of one inline image or file part.
const MEDIA_TOKENS: usize = 258;
/// Default ceiling for dynamic thinking.
const DYNAMIC_THINKING_CAP: usize = 24_576;

fn get<'a>(v: &'a Value, camel: &str, snake: &str) -> Option<&'a Value> {
    v.get(camel).or_else(|| v.get(snake))
}

fn tokens(text: &str) -> usize {
    crate::count_tokens_default(text).unwrap_or(text.len().div_ceil(4))
}

fn not_found(model: &str, method: &str) -> Response {
    gemini_error(
        404,
        format!(
            "models/{model} is not found for API version v1beta, or is not supported for \
             {method}. Call ListModels to see the list of available models and their supported \
             methods."
        ),
    )
}

/// Whether a model thinks by default.
fn thinks_by_default(model: &str) -> bool {
    let m = model.to_lowercase();
    (m.starts_with("gemini-2.5") && !m.contains("lite"))
        || m.starts_with("gemini-3")
        || m.starts_with("gemini-4")
}

fn is_pro(model: &str) -> bool {
    model.to_lowercase().contains("pro")
}

fn upper_types_to_lower(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let v = match (k.as_str(), v) {
                        ("type", Value::String(t)) => Value::String(t.to_lowercase()),
                        _ => upper_types_to_lower(v),
                    };
                    (k.clone(), v)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(upper_types_to_lower).collect()),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Request parsing
// ---------------------------------------------------------------------------

/// The parts of a generateContent request the simulator reads.
pub(super) struct GeminiRequest {
    pub system: String,
    pub contents: Vec<Value>,
    pub prompt_text: String,
    pub input_tokens: usize,
    pub tools: Vec<(String, Option<Value>)>,
    pub mode: String,
    pub allowed: Vec<String>,
    pub max_output: Option<u64>,
    pub json_mime: bool,
    pub schema: Option<Value>,
    pub thinking_budget: Option<i64>,
    pub include_thoughts: bool,
    pub thinking_level: Option<String>,
    /// The last content is a user message with text (not a function result).
    pub last_is_user_text: bool,
}

fn parts_text(content: &Value) -> String {
    get(content, "parts", "parts")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

fn media_parts(content: &Value) -> usize {
    get(content, "parts", "parts")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter(|p| {
                    get(p, "inlineData", "inline_data").is_some()
                        || get(p, "fileData", "file_data").is_some()
                })
                .count()
        })
        .unwrap_or(0)
}

pub(super) fn parse(body: &Value) -> Result<GeminiRequest, String> {
    let contents: Vec<Value> = match get(body, "contents", "contents") {
        Some(Value::Array(c)) if !c.is_empty() => c.clone(),
        Some(c @ Value::Object(_)) => vec![c.clone()],
        _ => return Err("* GenerateContentRequest.contents: contents is not specified\n".into()),
    };
    for c in &contents {
        match c.get("role").and_then(Value::as_str) {
            None | Some("user") | Some("model") | Some("function") => {}
            Some(_) => return Err("Please use a valid role: user, model.".into()),
        }
    }
    let system = get(body, "systemInstruction", "system_instruction")
        .map(parts_text)
        .unwrap_or_default();
    let mut prompt = vec![system.clone()];
    prompt.extend(contents.iter().map(parts_text));
    let prompt_text = prompt.join("\n");
    let media: usize = contents.iter().map(media_parts).sum();
    let input_tokens = tokens(&prompt_text) + media * MEDIA_TOKENS;

    let mut tools = Vec::new();
    for t in get(body, "tools", "tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for d in get(t, "functionDeclarations", "function_declarations")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(name) = d.get("name").and_then(Value::as_str) else {
                continue;
            };
            let schema = get(d, "parametersJsonSchema", "parameters_json_schema")
                .or_else(|| d.get("parameters"))
                .map(upper_types_to_lower);
            tools.push((name.to_string(), schema));
        }
    }
    let fcc = get(body, "toolConfig", "tool_config")
        .and_then(|t| get(t, "functionCallingConfig", "function_calling_config"));
    let mode = fcc
        .and_then(|f| f.get("mode"))
        .and_then(Value::as_str)
        .unwrap_or("AUTO")
        .to_uppercase();
    let allowed = fcc
        .and_then(|f| get(f, "allowedFunctionNames", "allowed_function_names"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let gc = get(body, "generationConfig", "generation_config");
    let gcf = |camel: &str, snake: &str| gc.and_then(|g| get(g, camel, snake));
    let json_mime = gcf("responseMimeType", "response_mime_type").and_then(Value::as_str)
        == Some("application/json");
    let schema = gcf("responseJsonSchema", "response_json_schema")
        .or_else(|| gcf("responseSchema", "response_schema"))
        .map(upper_types_to_lower);
    let tc = gcf("thinkingConfig", "thinking_config");
    let last = contents.last();
    let last_is_user_text = last.is_some_and(|c| {
        c.get("role").and_then(Value::as_str).unwrap_or("user") == "user"
            && !parts_text(c).is_empty()
    });
    Ok(GeminiRequest {
        system,
        contents: contents.clone(),
        prompt_text,
        input_tokens,
        tools,
        mode,
        allowed,
        max_output: gcf("maxOutputTokens", "max_output_tokens").and_then(Value::as_u64),
        json_mime,
        schema,
        thinking_budget: tc
            .and_then(|t| get(t, "thinkingBudget", "thinking_budget"))
            .and_then(Value::as_i64),
        include_thoughts: tc
            .and_then(|t| get(t, "includeThoughts", "include_thoughts"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        thinking_level: tc
            .and_then(|t| get(t, "thinkingLevel", "thinking_level"))
            .and_then(Value::as_str)
            .map(str::to_lowercase),
        last_is_user_text,
    })
}

// ---------------------------------------------------------------------------
// Response pieces
// ---------------------------------------------------------------------------

fn response_id() -> String {
    let id = crate::ids::prefixed_compact_id("");
    id.chars().take(22).collect()
}

/// `usageMetadata` for a response.
pub(super) fn usage_metadata(input: usize, output: usize, thoughts: usize, cached: usize) -> Value {
    let mut u = json!({
        "promptTokenCount": input,
        "candidatesTokenCount": output,
        "totalTokenCount": input + output + thoughts,
        "promptTokensDetails": [{"modality": "TEXT", "tokenCount": input}],
    });
    if thoughts > 0 {
        u["thoughtsTokenCount"] = json!(thoughts);
    }
    if cached > 0 {
        u["cachedContentTokenCount"] = json!(cached);
        u["cacheTokensDetails"] = json!([{"modality": "TEXT", "tokenCount": cached}]);
    }
    u
}

/// One GenerateContentResponse.
pub(super) fn chunk(
    model: &str,
    id: &str,
    parts: Vec<Value>,
    finish: Option<&str>,
    usage: Option<Value>,
) -> Value {
    let mut content = json!({"role": "model"});
    if !parts.is_empty() {
        content["parts"] = json!(parts);
    }
    let mut candidate = json!({"content": content, "index": 0});
    if let Some(f) = finish {
        candidate["finishReason"] = json!(f);
    }
    let mut v = json!({"candidates": [candidate], "modelVersion": model, "responseId": id});
    if let Some(u) = usage {
        v["usageMetadata"] = u;
    }
    v
}

/// Opaque thought signature for a thinking model's function call.
pub(super) fn thought_signature(seed: u64) -> String {
    let bytes: Vec<u8> = (0..24)
        .map(|i| (seed.rotate_left(i * 7) & 0xff) as u8)
        .collect();
    crate::imagegen::base64_encode(&bytes)
}

/// SSE (`alt=sse`) or JSON-array streaming body from timed chunks.
pub(super) fn stream_body(chunks: Vec<(Duration, Value)>, sse: bool) -> Response {
    let body = async_stream::stream! {
        let n = chunks.len();
        for (i, (delay, c)) in chunks.into_iter().enumerate() {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            let frame = if sse {
                format!("data: {c}\r\n\r\n")
            } else {
                let open = if i == 0 { "[" } else { ",\r\n" };
                let close = if i + 1 == n { "]" } else { "" };
                format!("{open}{c}{close}")
            };
            yield Ok::<_, std::io::Error>(frame);
        }
    };
    let content_type = if sse {
        "text/event-stream"
    } else {
        "application/json"
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(body))
        .unwrap()
}

/// Split text into chunks of `CHUNK_WORDS` words (whitespace kept).
fn word_chunks(text: &str) -> Vec<String> {
    let words = crate::scenario_stream::word_tokens(text);
    words.chunks(CHUNK_WORDS).map(|c| c.concat()).collect()
}

fn generate_text(state: &AppState, model: &str, prompt: &str, max: Option<u64>) -> String {
    let generator = create_generator(
        &state.config.response.generator,
        state.config.response.target_tokens,
    );
    let request = crate::openai::ChatCompletionRequest {
        model: model.to_string(),
        messages: vec![crate::openai::Message::user(prompt)],
        temperature: None,
        top_p: None,
        n: None,
        stream: false,
        stop: None,
        max_tokens: max.map(|m| m as u32),
        max_completion_tokens: None,
        presence_penalty: None,
        frequency_penalty: None,
        logit_bias: None,
        user: None,
        tools: None,
        tool_choice: None,
        response_format: None,
        seed: None,
        reasoning_effort: None,
    };
    generator.generate(&request)
}

fn truncate_words(text: &str, max_tokens: usize) -> String {
    let mut out = String::new();
    for w in crate::scenario_stream::word_tokens(text) {
        if tokens(&format!("{out}{w}")) > max_tokens {
            break;
        }
        out.push_str(&w);
    }
    out
}

/// Implicitly cached prompt tokens (`cachedContentTokenCount`).
fn implicit_cache(state: &AppState, model: &str, req: &GeminiRequest) -> usize {
    if !state.config.response.prompt_caching {
        return 0;
    }
    let min = if is_pro(model) { 4096 } else { 1024 };
    let mut b = super::prompt_cache::PrefixBuilder::new(&format!("gemini:{model}"));
    b.push(&req.system, tokens(&req.system));
    for c in &req.contents {
        let s = c.to_string();
        b.push(&s, tokens(&parts_text(c)) + media_parts(c) * MEDIA_TOKENS);
    }
    let cacheable: Vec<_> = b
        .prefixes
        .iter()
        .copied()
        .filter(|p| p.tokens >= min)
        .collect();
    state
        .prompt_cache
        .lookup_and_store(&cacheable, &cacheable, Instant::now())
        .min(req.input_tokens)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /gemini/v1beta/models/{model}:{action}
pub async fn model_action(
    State(state): State<Arc<AppState>>,
    Path(segment): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    Json(body): Json<Value>,
) -> Response {
    let Some((model, action)) = segment.split_once(':') else {
        return gemini_error(404, format!("Method not found: {segment}"));
    };
    let model = model.to_string();
    match action {
        "generateContent" => generate(state, model, body, None).await,
        "streamGenerateContent" => {
            let sse = query.get("alt").map(String::as_str) == Some("sse");
            generate(state, model, body, Some(sse)).await
        }
        "countTokens" => count_tokens(&model, &body),
        "embedContent" => embed_content(&state, &model, &body),
        "batchEmbedContents" => batch_embed(&state, &model, &body),
        other => gemini_error(404, format!("Method not found: {other}")),
    }
}

async fn generate(
    state: Arc<AppState>,
    model: String,
    body: Value,
    stream: Option<bool>,
) -> Response {
    let start = Instant::now();
    state
        .stats
        .record_request_start(&model, stream.is_some(), EndpointType::Gemini);
    let fail = |status: u16, message: String| {
        state.stats.record_error(status);
        gemini_error(status, message)
    };

    if let Some(error) = ErrorInjector::new(state.config.error_config()).maybe_inject() {
        let mut r = fail(error.status_code(), error.to_error_response().error.message);
        if let Some(retry) = error.retry_after() {
            r.headers_mut().insert(header::RETRY_AFTER, retry.into());
        }
        return r;
    }
    let profile = crate::openai::get_model_profile(&model);
    if profile.is_none()
        && state.config.validation.strict_models
        && !model.starts_with(crate::scenario::MODEL_PREFIX)
    {
        state.stats.record_error(404);
        return not_found(&model, "generateContent");
    }
    let req = match parse(&body) {
        Ok(r) => r,
        Err(m) => return fail(400, m),
    };
    if let (Some(p), true) = (profile, state.config.validation.enforce_limits) {
        if let Some(max) = req.max_output {
            if max > u64::from(p.max_output_tokens) {
                return fail(
                    400,
                    format!(
                        "Unable to submit request because it has a maxOutputTokens value of {max} \
                         but the supported range is from 1 (inclusive) to {} (exclusive). Update \
                         the value and try again.",
                        p.max_output_tokens + 1
                    ),
                );
            }
        }
        if req.input_tokens > p.context_window as usize {
            return fail(
                400,
                format!(
                    "The input token count ({}) exceeds the maximum number of tokens allowed ({}).",
                    req.input_tokens, p.context_window
                ),
            );
        }
    }
    let thinks = thinks_by_default(&model);
    if req.thinking_budget == Some(0) && thinks && is_pro(&model) {
        return fail(
            400,
            "Budget 0 is invalid. This model only works in thinking mode.".into(),
        );
    }

    let latency = state.config.latency_for_model(&model);
    if let Some(r) =
        super::scenario_handlers::gemini(&state, &model, &req, stream, start, latency.clone()).await
    {
        return r;
    }

    let seed = crate::schema_fake::seed_from(req.prompt_text.as_bytes());
    let call = crate::forced::gemini(
        &req.tools,
        &req.mode,
        &req.allowed,
        state.config.response.auto_tool_calls && req.last_is_user_text,
        seed,
    );
    let mut text = match (&call, req.json_mime, &req.schema) {
        (Some(_), _, _) => String::new(),
        (None, true, Some(schema)) => crate::forced::json_for_schema(schema, seed),
        (None, true, None) => json!({
            "response": generate_text(&state, &model, &req.prompt_text, req.max_output)
        })
        .to_string(),
        (None, false, _) => generate_text(&state, &model, &req.prompt_text, req.max_output),
    };
    let call_tokens = call
        .as_ref()
        .map_or(0, |c| tokens(&c.arguments.to_string()) + 1);

    // Thinking: on by default for thinking models, off with budget 0.
    let thinking = thinks && req.thinking_budget != Some(0);
    let answer_tokens = tokens(&text).max(call_tokens);
    let thoughts = if thinking {
        let factor = match req.thinking_level.as_deref() {
            Some("minimal") => 0.25,
            Some("low") => 0.5,
            Some("medium") => 1.0,
            _ => 2.0,
        };
        let cap = match req.thinking_budget {
            Some(b) if b > 0 => b as usize,
            _ => DYNAMIC_THINKING_CAP,
        };
        ((answer_tokens as f64 * factor) as usize).clamp(64, cap)
    } else {
        0
    };

    // Thinking counts against maxOutputTokens; the answer gets what is left.
    let mut finish = "STOP";
    let mut thoughts = thoughts;
    if let Some(max) = req.max_output.map(|m| m as usize) {
        if thoughts >= max {
            thoughts = max;
            text.clear();
            finish = "MAX_TOKENS";
        } else if tokens(&text) > max - thoughts {
            text = truncate_words(&text, max - thoughts);
            finish = "MAX_TOKENS";
        }
    }
    let text_tokens = if text.is_empty() { 0 } else { tokens(&text) };
    let output_tokens = text_tokens + call_tokens;
    let summary = (thinking && req.include_thoughts).then(|| {
        let words: Vec<&str> = text.split_whitespace().take(40).collect();
        format!(
            "**Planning the answer**\n\nI need to address: {}",
            if words.is_empty() {
                "the request".to_string()
            } else {
                words.join(" ")
            }
        )
    });

    let cached = implicit_cache(&state, &model, &req);
    let latency = latency.with_input_tokens(req.input_tokens - cached);
    let usage = usage_metadata(req.input_tokens, output_tokens, thoughts, cached);
    let id = response_id();
    let tbt = Duration::from_millis(latency.tbt_mean_ms);
    // Hidden thinking takes time before the first visible token.
    let hidden = if summary.is_some() {
        Duration::ZERO
    } else {
        tbt * thoughts as u32
    };

    let mut call_part = call.as_ref().map(|c| {
        let mut p = json!({"functionCall": {"name": c.name, "args": c.arguments}});
        if thinking {
            p["thoughtSignature"] = json!(thought_signature(seed));
        }
        p
    });

    let input = req.input_tokens as u32;
    let total_out = (output_tokens + thoughts) as u32;
    if let Some(sse) = stream {
        let mut chunks: Vec<(Duration, Value)> = Vec::new();
        let mut first = latency.sample_ttft() + hidden;
        if let Some(s) = &summary {
            for piece in word_chunks(s) {
                let d = std::mem::take(&mut first) + tbt * CHUNK_WORDS as u32;
                chunks.push((
                    d,
                    chunk(
                        &model,
                        &id,
                        vec![json!({"text": piece, "thought": true})],
                        None,
                        None,
                    ),
                ));
            }
        }
        for piece in word_chunks(&text) {
            let d = std::mem::take(&mut first) + tbt * CHUNK_WORDS as u32;
            chunks.push((
                d,
                chunk(&model, &id, vec![json!({"text": piece})], None, None),
            ));
        }
        let last_parts: Vec<Value> = call_part.take().into_iter().collect();
        chunks.push((
            first,
            chunk(&model, &id, last_parts, Some(finish), Some(usage)),
        ));
        let stats = state.stats.clone();
        let total = chunks.iter().map(|(d, _)| *d).sum::<Duration>();
        tokio::spawn(async move {
            tokio::time::sleep(total).await;
            stats.record_request_end(start.elapsed(), input, total_out);
        });
        return stream_body(chunks, sse);
    }

    let delay = latency.sample_ttft()
        + hidden
        + tbt * (output_tokens + summary.as_ref().map_or(0, |s| tokens(s))) as u32;
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    let mut parts = Vec::new();
    if let Some(s) = summary {
        parts.push(json!({"text": s, "thought": true}));
    }
    if !text.is_empty() {
        parts.push(json!({"text": text}));
    }
    parts.extend(call_part);
    state
        .stats
        .record_request_end(start.elapsed(), input, total_out);
    Json(chunk(&model, &id, parts, Some(finish), Some(usage))).into_response()
}

fn count_tokens(model: &str, body: &Value) -> Response {
    let inner = get(body, "generateContentRequest", "generate_content_request");
    let source = inner.unwrap_or(body);
    if get(source, "contents", "contents").is_none() {
        return gemini_error(
            400,
            "* CountTokensRequest.contents: contents is not specified\n".into(),
        );
    }
    let _ = model;
    match parse(source) {
        Ok(r) => Json(json!({
            "totalTokens": r.input_tokens,
            "promptTokensDetails": [{"modality": "TEXT", "tokenCount": r.input_tokens}],
        }))
        .into_response(),
        Err(m) => gemini_error(400, m),
    }
}

// ---------------------------------------------------------------------------
// Embeddings
// ---------------------------------------------------------------------------

/// (native dimensions, smallest allowed, renormalize when shortened)
fn gemini_embedding_shape(model: &str) -> (usize, usize, bool) {
    match model {
        "gemini-embedding-001" => (3072, 128, false),
        _ => (768, 1, true),
    }
}

fn embed_one(model: &str, content: &Value, dims: Option<u64>) -> Result<Vec<f32>, String> {
    let text = parts_text(content);
    if text.is_empty() {
        return Err("* BatchEmbedContentsRequest.requests[0].content.parts: contents.parts must not be empty.\n".into());
    }
    let (native, min, renormalize) = gemini_embedding_shape(model);
    let d = dims.map_or(native, |d| d as usize);
    if d < min || d > native {
        return Err(format!(
            "outputDimensionality must be between {min} and {native}, got {d}."
        ));
    }
    let mut v = crate::embeddings::embed(&text, native);
    v.truncate(d);
    if renormalize && d < native {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter_mut().for_each(|x| *x /= n);
    }
    Ok(v)
}

fn embed_content(state: &AppState, model: &str, body: &Value) -> Response {
    state
        .stats
        .record_request_start(model, false, EndpointType::Embeddings);
    let content = body.get("content").cloned().unwrap_or(Value::Null);
    let dims = get(body, "outputDimensionality", "output_dimensionality").and_then(Value::as_u64);
    match embed_one(model, &content, dims) {
        Ok(v) => Json(json!({"embedding": {"values": v}})).into_response(),
        Err(m) => {
            state.stats.record_error(400);
            gemini_error(400, m)
        }
    }
}

fn batch_embed(state: &AppState, model: &str, body: &Value) -> Response {
    state
        .stats
        .record_request_start(model, false, EndpointType::Embeddings);
    let Some(requests) = body.get("requests").and_then(Value::as_array) else {
        state.stats.record_error(400);
        return gemini_error(
            400,
            "* BatchEmbedContentsRequest.requests: requests is not specified\n".into(),
        );
    };
    let mut out = Vec::with_capacity(requests.len());
    for r in requests {
        let content = r.get("content").cloned().unwrap_or(Value::Null);
        let dims = get(r, "outputDimensionality", "output_dimensionality").and_then(Value::as_u64);
        match embed_one(model, &content, dims) {
            Ok(v) => out.push(json!({"values": v})),
            Err(m) => {
                state.stats.record_error(400);
                return gemini_error(400, m);
            }
        }
    }
    Json(json!({"embeddings": out})).into_response()
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

const EMBEDDING_MODELS: &[(&str, &str, u32)] = &[
    ("gemini-embedding-001", "Gemini Embedding 001", 2048),
    ("text-embedding-004", "Text Embedding 004", 2048),
];

fn model_json(p: &crate::openai::ModelProfile) -> Value {
    json!({
        "name": format!("models/{}", p.id),
        "baseModelId": p.id,
        "version": "001",
        "displayName": p.name,
        "description": p.name,
        "inputTokenLimit": p.context_window,
        "outputTokenLimit": p.max_output_tokens,
        "supportedGenerationMethods": ["generateContent", "countTokens", "streamGenerateContent"],
        "temperature": 1.0,
        "maxTemperature": 2.0,
        "topP": 0.95,
        "topK": 64,
        "thinking": thinks_by_default(&p.id),
    })
}

fn embedding_model_json(id: &str, name: &str, limit: u32) -> Value {
    json!({
        "name": format!("models/{id}"),
        "version": "001",
        "displayName": name,
        "description": name,
        "inputTokenLimit": limit,
        "outputTokenLimit": 1,
        "supportedGenerationMethods": ["embedContent", "countTextTokens", "countTokens"],
    })
}

/// GET /gemini/v1beta/models
pub async fn list_models() -> Response {
    let mut models: Vec<&crate::openai::ModelProfile> = crate::openai::all_model_profiles()
        .filter(|p| p.id.starts_with("gemini-"))
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    let mut out: Vec<Value> = models.into_iter().map(model_json).collect();
    out.extend(
        EMBEDDING_MODELS
            .iter()
            .map(|(id, name, limit)| embedding_model_json(id, name, *limit)),
    );
    Json(json!({"models": out})).into_response()
}

/// GET /gemini/v1beta/models/{model}
pub async fn get_model(Path(name): Path<String>) -> Response {
    if let Some((id, n, limit)) = EMBEDDING_MODELS.iter().find(|(id, _, _)| *id == name) {
        return Json(embedding_model_json(id, n, *limit)).into_response();
    }
    match crate::openai::get_model_profile(&name).filter(|p| p.id.starts_with("gemini-")) {
        Some(p) => Json(model_json(p)).into_response(),
        None => gemini_error(
            404,
            format!("models/{name} is not found for API version v1beta"),
        ),
    }
}
