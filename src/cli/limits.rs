// Provider-style rate limits and overload bursts, applied as router middleware.
//
// Decisions:
// - Limits are token buckets per API key (the `Authorization` bearer,
//   `x-api-key` or `x-goog-api-key`; requests without one share an
//   "anonymous" bucket), refilled continuously over a minute, like the
//   providers' RPM/TPM limits.
// - Token cost is charged when the request arrives, the way OpenAI does it:
//   an estimate of the prompt (body bytes / 4) plus the requested output cap
//   (`max_tokens` / `max_completion_tokens` / `max_output_tokens`, else
//   `DEFAULT_OUTPUT_ESTIMATE`). The real tokenizer runs later in the handler;
//   limits only need to be in the right range.
// - Headers follow each provider: OpenAI `x-ratelimit-*` (durations like
//   `6m0s`), Anthropic `anthropic-ratelimit-*` (RFC 3339 reset times). They
//   are sent only when a limit is configured, so default responses keep their
//   old headers.
// - An overload burst is a window in which every generation request fails,
//   the way provider incidents arrive in clusters rather than as independent
//   coin flips: Anthropic and TypeSafe answer 529 `overloaded_error`, the
//   others 503. A burst starts with probability `overload_burst_rate` per
//   request and lasts `overload_burst_ms`.
// - Only generation endpoints (POST) are limited; model lists, health and
//   stats never are.

use super::state::AppState;
use crate::stats::EndpointType;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use rand::RngExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Output tokens charged when a request names no output cap.
pub const DEFAULT_OUTPUT_ESTIMATE: u64 = 1_000;
/// Largest request body buffered for token estimation (larger bodies are
/// charged by their declared length).
const MAX_BUFFERED_BODY: usize = 64 * 1024 * 1024;

/// Which provider's conventions a path follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAi,
    Anthropic,
    Gemini,
    TypeSafe,
}

impl Provider {
    /// The provider for a generation endpoint path, `None` for anything that
    /// is never limited.
    pub fn for_path(path: &str) -> Option<Self> {
        if path.ends_with("/models") || path.contains("/models/") || path.ends_with("/count_tokens")
        {
            return None;
        }
        if path.starts_with("/openai/") || path.starts_with("/openresponses/") {
            Some(Self::OpenAi)
        } else if path.starts_with("/anthropic/") {
            Some(Self::Anthropic)
        } else if path.starts_with("/gemini/") {
            Some(Self::Gemini)
        } else if path.starts_with("/typesafe/") || path.starts_with("/foundry/") {
            Some(Self::TypeSafe)
        } else {
            None
        }
    }

    /// An error body in this provider's envelope.
    pub fn error(self, status: u16, kind: &str, message: &str) -> Response {
        let body = match self {
            // OpenAI's 429 names the exhausted limit as the type
            // (`requests`/`tokens`) and `rate_limit_exceeded` as the code.
            Self::OpenAi => json!({"error": {
                "message": message,
                "type": kind,
                "param": null,
                "code": if status == 429 { Some("rate_limit_exceeded") } else { None },
            }}),
            Self::Anthropic => {
                json!({"type": "error", "error": {"type": kind, "message": message}})
            }
            Self::Gemini => json!({"error": {
                "code": status,
                "message": message,
                "status": match status {
                    429 => "RESOURCE_EXHAUSTED",
                    503 => "UNAVAILABLE",
                    _ => "INTERNAL",
                },
            }}),
            Self::TypeSafe => json!({"detail": {"error_type": kind, "message": message}}),
        };
        let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (code, Json(body)).into_response()
    }
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    level: f64,
    capacity: f64,
    updated: Instant,
}

impl Bucket {
    fn new(capacity: u64, now: Instant) -> Self {
        Self {
            level: capacity as f64,
            capacity: capacity as f64,
            updated: now,
        }
    }

    fn per_sec(&self) -> f64 {
        self.capacity / 60.0
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.updated).as_secs_f64();
        self.level = (self.level + elapsed * self.per_sec()).min(self.capacity);
        self.updated = now;
    }

    /// Time until `amount` is available.
    fn wait_for(&self, amount: f64) -> Duration {
        let missing = (amount.min(self.capacity) - self.level).max(0.0);
        Duration::from_secs_f64(missing / self.per_sec())
    }

    /// Time until the bucket is full again.
    fn reset(&self) -> Duration {
        Duration::from_secs_f64((self.capacity - self.level).max(0.0) / self.per_sec())
    }

    fn remaining(&self) -> u64 {
        self.level.max(0.0) as u64
    }
}

#[derive(Debug, Clone, Copy)]
struct KeyBuckets {
    requests: Option<Bucket>,
    tokens: Option<Bucket>,
}

/// Snapshot of one key's limits after a request, for response headers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LimitSnapshot {
    pub limit_requests: Option<u64>,
    pub remaining_requests: u64,
    pub reset_requests: Duration,
    pub limit_tokens: Option<u64>,
    pub remaining_tokens: u64,
    pub reset_tokens: Duration,
}

/// Outcome of charging a request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    Allowed(LimitSnapshot),
    Limited {
        snapshot: LimitSnapshot,
        retry_after: Duration,
        /// `true` when the token budget (not the request count) ran out.
        tokens: bool,
    },
}

/// Per-key RPM/TPM buckets plus the overload-burst clock.
#[derive(Debug, Default)]
pub struct RateLimiter {
    requests_per_minute: u64,
    tokens_per_minute: u64,
    keys: Mutex<HashMap<String, KeyBuckets>>,
    overloaded_until: Mutex<Option<Instant>>,
}

impl RateLimiter {
    pub fn new(requests_per_minute: u64, tokens_per_minute: u64) -> Self {
        Self {
            requests_per_minute,
            tokens_per_minute,
            ..Self::default()
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.requests_per_minute > 0 || self.tokens_per_minute > 0
    }

    /// Charge one request costing `tokens` against `key`'s buckets.
    pub fn check(&self, key: &str, tokens: u64, now: Instant) -> Decision {
        let mut keys = self.keys.lock().unwrap_or_else(|e| e.into_inner());
        let entry = keys.entry(key.to_string()).or_insert_with(|| KeyBuckets {
            requests: (self.requests_per_minute > 0)
                .then(|| Bucket::new(self.requests_per_minute, now)),
            tokens: (self.tokens_per_minute > 0).then(|| Bucket::new(self.tokens_per_minute, now)),
        });
        for b in [&mut entry.requests, &mut entry.tokens]
            .into_iter()
            .flatten()
        {
            b.refill(now);
        }
        let request_wait = entry.requests.map_or(Duration::ZERO, |b| b.wait_for(1.0));
        let token_wait = entry
            .tokens
            .map_or(Duration::ZERO, |b| b.wait_for(tokens as f64));
        let limited = !request_wait.is_zero() || !token_wait.is_zero();
        if !limited {
            if let Some(b) = entry.requests.as_mut() {
                b.level -= 1.0;
            }
            if let Some(b) = entry.tokens.as_mut() {
                b.level -= tokens as f64;
            }
        }
        let snapshot = LimitSnapshot {
            limit_requests: entry.requests.map(|b| b.capacity as u64),
            remaining_requests: entry.requests.map_or(0, |b| b.remaining()),
            reset_requests: entry.requests.map_or(Duration::ZERO, |b| b.reset()),
            limit_tokens: entry.tokens.map(|b| b.capacity as u64),
            remaining_tokens: entry.tokens.map_or(0, |b| b.remaining()),
            reset_tokens: entry.tokens.map_or(Duration::ZERO, |b| b.reset()),
        };
        if limited {
            Decision::Limited {
                snapshot,
                retry_after: request_wait.max(token_wait),
                tokens: token_wait > request_wait,
            }
        } else {
            Decision::Allowed(snapshot)
        }
    }

    /// Whether an overload burst is in progress, possibly starting one.
    pub fn overloaded(&self, rate: f64, burst: Duration, now: Instant) -> Option<Duration> {
        let mut until = self
            .overloaded_until
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(end) = *until {
            if now < end {
                return Some(end - now);
            }
            *until = None;
        }
        if rate > 0.0 && rand::rng().random_bool(rate.clamp(0.0, 1.0)) {
            *until = Some(now + burst);
            return Some(burst);
        }
        None
    }
}

/// The caller's API key, or "anonymous".
fn api_key(headers: &HeaderMap) -> String {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    get("authorization")
        .map(|v| v.trim_start_matches("Bearer ").to_string())
        .or_else(|| get("x-api-key").map(str::to_string))
        .or_else(|| get("x-goog-api-key").map(str::to_string))
        .unwrap_or_else(|| "anonymous".to_string())
}

/// Estimated token cost of a request body.
pub fn estimate_tokens(body: &[u8]) -> u64 {
    let prompt = (body.len() as u64).div_ceil(4);
    let output = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| {
            ["max_completion_tokens", "max_tokens", "max_output_tokens"]
                .iter()
                .find_map(|k| v.get(*k).and_then(Value::as_u64))
        })
        .unwrap_or(DEFAULT_OUTPUT_ESTIMATE);
    prompt + output
}

/// Go-style duration as OpenAI prints it: `12ms`, `1s`, `6m0s`.
pub fn openai_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        return format!("{ms}ms");
    }
    let secs = d.as_secs_f64();
    let minutes = (secs / 60.0).floor() as u64;
    let rest = secs - minutes as f64 * 60.0;
    let rest = if rest.fract() == 0.0 {
        format!("{}s", rest as u64)
    } else {
        format!("{:.3}s", rest).replace(".000s", "s")
    };
    if minutes > 0 {
        format!("{minutes}m{rest}")
    } else {
        rest
    }
}

fn rfc3339_after(d: Duration) -> String {
    let at = SystemTime::now() + d;
    let secs = at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    crate::anthropic::iso8601_utc(secs)
}

fn insert(headers: &mut HeaderMap, name: &'static str, value: String) {
    if let Ok(v) = HeaderValue::from_str(&value) {
        headers.insert(name, v);
    }
}

/// Add the provider's rate-limit headers for `s`.
pub fn add_headers(provider: Provider, s: &LimitSnapshot, headers: &mut HeaderMap) {
    match provider {
        Provider::Anthropic => {
            if let Some(limit) = s.limit_requests {
                insert(
                    headers,
                    "anthropic-ratelimit-requests-limit",
                    limit.to_string(),
                );
                insert(
                    headers,
                    "anthropic-ratelimit-requests-remaining",
                    s.remaining_requests.to_string(),
                );
                insert(
                    headers,
                    "anthropic-ratelimit-requests-reset",
                    rfc3339_after(s.reset_requests),
                );
            }
            if let Some(limit) = s.limit_tokens {
                for prefix in [
                    "anthropic-ratelimit-tokens",
                    "anthropic-ratelimit-input-tokens",
                ] {
                    let (l, r, t) = match prefix {
                        "anthropic-ratelimit-tokens" => (
                            "anthropic-ratelimit-tokens-limit",
                            "anthropic-ratelimit-tokens-remaining",
                            "anthropic-ratelimit-tokens-reset",
                        ),
                        _ => (
                            "anthropic-ratelimit-input-tokens-limit",
                            "anthropic-ratelimit-input-tokens-remaining",
                            "anthropic-ratelimit-input-tokens-reset",
                        ),
                    };
                    insert(headers, l, limit.to_string());
                    insert(headers, r, s.remaining_tokens.to_string());
                    insert(headers, t, rfc3339_after(s.reset_tokens));
                }
            }
        }
        _ => {
            if let Some(limit) = s.limit_requests {
                insert(headers, "x-ratelimit-limit-requests", limit.to_string());
                insert(
                    headers,
                    "x-ratelimit-remaining-requests",
                    s.remaining_requests.to_string(),
                );
                insert(
                    headers,
                    "x-ratelimit-reset-requests",
                    openai_duration(s.reset_requests),
                );
            }
            if let Some(limit) = s.limit_tokens {
                insert(headers, "x-ratelimit-limit-tokens", limit.to_string());
                insert(
                    headers,
                    "x-ratelimit-remaining-tokens",
                    s.remaining_tokens.to_string(),
                );
                insert(
                    headers,
                    "x-ratelimit-reset-tokens",
                    openai_duration(s.reset_tokens),
                );
            }
        }
    }
}

fn record_rejected(state: &AppState, status: u16) {
    state
        .stats
        .record_request_start("unknown", false, EndpointType::ChatCompletions);
    state.stats.record_error(status);
}

fn with_retry_after(mut response: Response, wait: Duration) -> Response {
    let secs = wait.as_secs_f64().ceil().max(1.0) as u64;
    insert(response.headers_mut(), "retry-after", secs.to_string());
    if let Ok(v) = HeaderValue::from_str(&wait.as_millis().max(1).to_string()) {
        response.headers_mut().insert("retry-after-ms", v);
    }
    response
}

/// Router middleware: overload bursts, then RPM/TPM limits.
pub async fn middleware(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let provider = match (request.method(), Provider::for_path(request.uri().path())) {
        (&Method::POST, Some(p)) => p,
        _ => return next.run(request).await,
    };
    let now = Instant::now();
    let errors = &state.config.errors;
    if errors.overload_burst_rate > 0.0 {
        let burst = Duration::from_millis(errors.overload_burst_ms);
        if let Some(left) = state
            .limiter
            .overloaded(errors.overload_burst_rate, burst, now)
        {
            let (status, kind) = match provider {
                Provider::Anthropic | Provider::TypeSafe => (529, "overloaded_error"),
                _ => (503, "service_unavailable"),
            };
            record_rejected(&state, status);
            let response = provider.error(status, kind, "Overloaded");
            return with_retry_after(response, left);
        }
    }
    if !state.limiter.is_enabled() {
        return next.run(request).await;
    }

    let key = api_key(request.headers());
    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_BUFFERED_BODY).await {
        Ok(b) => b,
        Err(_) => {
            return provider.error(413, "invalid_request_error", "Request body too large");
        }
    };
    let tokens = estimate_tokens(&bytes);
    let request = Request::from_parts(parts, Body::from(bytes));

    match state.limiter.check(&key, tokens, now) {
        Decision::Allowed(snapshot) => {
            let mut response = next.run(request).await;
            add_headers(provider, &snapshot, response.headers_mut());
            response
        }
        Decision::Limited {
            snapshot,
            retry_after,
            tokens: on_tokens,
        } => {
            record_rejected(&state, 429);
            let what = if on_tokens { "tokens" } else { "requests" };
            let message = format!(
                "Rate limit reached for {what} per minute. Please try again in {}.",
                openai_duration(retry_after)
            );
            let kind = match provider {
                Provider::OpenAi => what,
                _ => "rate_limit_error",
            };
            let mut response = provider.error(429, kind, &message);
            add_headers(provider, &snapshot, response.headers_mut());
            with_retry_after(response, retry_after)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_bucket_limits_then_refills() {
        let limiter = RateLimiter::new(2, 0);
        let t0 = Instant::now();
        assert!(matches!(limiter.check("k", 10, t0), Decision::Allowed(_)));
        assert!(matches!(limiter.check("k", 10, t0), Decision::Allowed(_)));
        let Decision::Limited { retry_after, .. } = limiter.check("k", 10, t0) else {
            panic!("third request should be limited");
        };
        // 2 RPM refills one request every 30 s.
        assert!(
            (29..=30).contains(&retry_after.as_secs()),
            "{retry_after:?}"
        );
        // Other keys have their own buckets.
        assert!(matches!(
            limiter.check("other", 10, t0),
            Decision::Allowed(_)
        ));
        assert!(matches!(
            limiter.check("k", 10, t0 + Duration::from_secs(31)),
            Decision::Allowed(_)
        ));
    }

    #[test]
    fn token_bucket_charges_estimates() {
        let limiter = RateLimiter::new(0, 1_000);
        let t0 = Instant::now();
        let Decision::Allowed(s) = limiter.check("k", 600, t0) else {
            panic!()
        };
        assert_eq!(s.remaining_tokens, 400);
        assert_eq!(s.limit_tokens, Some(1_000));
        assert!(matches!(
            limiter.check("k", 600, t0),
            Decision::Limited { tokens: true, .. }
        ));
    }

    #[test]
    fn estimates_use_output_caps() {
        assert_eq!(estimate_tokens(br#"{"max_tokens": 50}"#), 5 + 50);
        assert_eq!(estimate_tokens(b"{}"), 1 + DEFAULT_OUTPUT_ESTIMATE);
    }

    #[test]
    fn openai_durations() {
        assert_eq!(openai_duration(Duration::from_millis(12)), "12ms");
        assert_eq!(openai_duration(Duration::from_secs(1)), "1s");
        assert_eq!(openai_duration(Duration::from_secs(360)), "6m0s");
        assert_eq!(openai_duration(Duration::from_millis(1500)), "1.500s");
    }

    #[test]
    fn bursts_last_their_window() {
        let limiter = RateLimiter::default();
        let t0 = Instant::now();
        let burst = Duration::from_secs(5);
        assert_eq!(limiter.overloaded(1.0, burst, t0), Some(burst));
        assert_eq!(
            limiter.overloaded(0.0, burst, t0 + Duration::from_secs(2)),
            Some(Duration::from_secs(3))
        );
        assert_eq!(
            limiter.overloaded(0.0, burst, t0 + Duration::from_secs(6)),
            None
        );
    }

    #[test]
    fn only_generation_paths_are_limited() {
        assert_eq!(
            Provider::for_path("/openai/v1/chat/completions"),
            Some(Provider::OpenAi)
        );
        assert_eq!(
            Provider::for_path("/anthropic/v1/messages"),
            Some(Provider::Anthropic)
        );
        assert_eq!(Provider::for_path("/openai/v1/models"), None);
        assert_eq!(
            Provider::for_path("/anthropic/v1/messages/count_tokens"),
            None
        );
        assert_eq!(Provider::for_path("/health"), None);
    }
}
