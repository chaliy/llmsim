//! Statistics module for tracking real-time metrics.
//!
//! This module provides thread-safe atomic counters and statistics
//! collection for monitoring LLMSim server performance.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Relaxed ordering for stats - we don't need strict ordering guarantees
const ORDERING: Ordering = Ordering::Relaxed;

/// Width of the rolling RPS window, in 1-second buckets.
///
/// RPS is tracked as a ring of per-second atomic buckets instead of a
/// `Vec<Instant>` behind a write lock. The previous design took a global write
/// lock and did an O(n) `retain` on *every* request, which serialized all
/// worker threads and was the dominant throughput bottleneck under load. Each
/// bucket packs `(second_tag << 32) | count` into one AtomicU64, so recording a
/// request is a single lock-free compare-exchange.
const RPS_WINDOW_SECS: u64 = 60;

/// Maximum bytes kept for a model name in stats.
const MAX_MODEL_NAME_BYTES: usize = 128;
/// Maximum number of distinct model keys tracked before aggregating.
const MAX_TRACKED_MODELS: usize = 128;
/// Bucket for model names beyond tracking limits.
const OTHER_MODELS_BUCKET: &str = "__other__";

/// Type of API endpoint being called
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointType {
    /// Chat Completions API (/openai/v1/chat/completions)
    ChatCompletions,
    /// Responses API (/openai/v1/responses)
    Responses,
    /// WebSocket Responses API (/openai/v1/responses via WS)
    WebSocketResponses,
    /// Anthropic Messages API (/anthropic/v1/messages)
    Messages,
}

/// Global statistics tracker for the LLMSim server.
#[derive(Debug)]
pub struct Stats {
    /// Server start time
    start_time: Instant,

    // Request counters
    /// Total number of requests received
    pub total_requests: AtomicU64,
    /// Currently active (in-flight) requests
    pub active_requests: AtomicU64,
    /// Total streaming requests
    pub streaming_requests: AtomicU64,
    /// Total non-streaming requests
    pub non_streaming_requests: AtomicU64,
    /// Chat Completions API requests
    pub completions_requests: AtomicU64,
    /// Responses API requests
    pub responses_requests: AtomicU64,
    /// WebSocket Responses API requests (individual response.create messages)
    pub websocket_requests: AtomicU64,
    /// Anthropic Messages API requests
    pub messages_requests: AtomicU64,
    /// Currently active WebSocket connections
    pub active_websocket_connections: AtomicU64,

    // Token counters
    /// Total prompt tokens processed
    pub prompt_tokens: AtomicU64,
    /// Total completion tokens generated
    pub completion_tokens: AtomicU64,

    // Error counters
    /// Total errors returned
    pub total_errors: AtomicU64,
    /// Rate limit errors (429)
    pub rate_limit_errors: AtomicU64,
    /// Server errors (500)
    pub server_errors: AtomicU64,
    /// Timeout errors (504)
    pub timeout_errors: AtomicU64,

    // Per-model request counts. The value is an AtomicU64 so the common case
    // (a model that's already been seen) increments under a shared read lock
    // with no serialization; the write lock is only taken to insert a new model
    // key, which is bounded by MAX_TRACKED_MODELS.
    model_requests: RwLock<HashMap<String, AtomicU64>>,

    // Latency tracking (in microseconds)
    /// Total latency for calculating average
    total_latency_us: AtomicU64,
    /// Count of completed requests (for average calculation)
    completed_requests: AtomicU64,
    /// Minimum latency seen
    min_latency_us: AtomicU64,
    /// Maximum latency seen
    max_latency_us: AtomicU64,

    // Rolling window for RPS calculation: one AtomicU64 per second bucket,
    // each packing (second_tag << 32) | count. See RPS_WINDOW_SECS.
    rps_buckets: Vec<AtomicU64>,
}

impl Default for Stats {
    fn default() -> Self {
        Self::new()
    }
}

impl Stats {
    /// Create a new Stats instance
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            total_requests: AtomicU64::new(0),
            active_requests: AtomicU64::new(0),
            streaming_requests: AtomicU64::new(0),
            non_streaming_requests: AtomicU64::new(0),
            completions_requests: AtomicU64::new(0),
            responses_requests: AtomicU64::new(0),
            websocket_requests: AtomicU64::new(0),
            messages_requests: AtomicU64::new(0),
            active_websocket_connections: AtomicU64::new(0),
            prompt_tokens: AtomicU64::new(0),
            completion_tokens: AtomicU64::new(0),
            total_errors: AtomicU64::new(0),
            rate_limit_errors: AtomicU64::new(0),
            server_errors: AtomicU64::new(0),
            timeout_errors: AtomicU64::new(0),
            model_requests: RwLock::new(HashMap::new()),
            total_latency_us: AtomicU64::new(0),
            completed_requests: AtomicU64::new(0),
            min_latency_us: AtomicU64::new(u64::MAX),
            max_latency_us: AtomicU64::new(0),
            rps_buckets: (0..RPS_WINDOW_SECS).map(|_| AtomicU64::new(0)).collect(),
        }
    }

    /// Record the start of a new request
    pub fn record_request_start(&self, model: &str, is_streaming: bool, endpoint: EndpointType) {
        self.total_requests.fetch_add(1, ORDERING);
        self.active_requests.fetch_add(1, ORDERING);

        if is_streaming {
            self.streaming_requests.fetch_add(1, ORDERING);
        } else {
            self.non_streaming_requests.fetch_add(1, ORDERING);
        }

        // Track by endpoint type
        match endpoint {
            EndpointType::ChatCompletions => {
                self.completions_requests.fetch_add(1, ORDERING);
            }
            EndpointType::Responses => {
                self.responses_requests.fetch_add(1, ORDERING);
            }
            EndpointType::WebSocketResponses => {
                self.websocket_requests.fetch_add(1, ORDERING);
            }
            EndpointType::Messages => {
                self.messages_requests.fetch_add(1, ORDERING);
            }
        }

        // Track per-model requests with bounded key size/cardinality.
        // Fast path: a model we've already seen increments under a shared read
        // lock (read locks don't block each other), so concurrent requests for
        // known models don't serialize.
        let model_key = normalize_model_name(model);
        let counted = match self.model_requests.read() {
            Ok(map) => match map.get(&model_key) {
                Some(counter) => {
                    counter.fetch_add(1, ORDERING);
                    true
                }
                None => false,
            },
            Err(_) => false,
        };
        if !counted {
            // Slow path: insert a new key (write lock, bounded by cardinality cap).
            if let Ok(mut map) = self.model_requests.write() {
                let bucket = if map.contains_key(&model_key)
                    || map.len() < MAX_TRACKED_MODELS
                    || model_key == OTHER_MODELS_BUCKET
                {
                    model_key
                } else {
                    OTHER_MODELS_BUCKET.to_string()
                };
                map.entry(bucket)
                    .or_insert_with(|| AtomicU64::new(0))
                    .fetch_add(1, ORDERING);
            }
        }

        // Record into the rolling RPS window: lock-free update of this second's
        // bucket. Packs (second_tag << 32) | count into one AtomicU64.
        let sec = self.start_time.elapsed().as_secs();
        let tag = (sec as u32) as u64;
        let bucket = &self.rps_buckets[(sec % RPS_WINDOW_SECS) as usize];
        let mut cur = bucket.load(ORDERING);
        loop {
            let new = if (cur >> 32) == tag {
                (tag << 32) | ((cur & 0xFFFF_FFFF) + 1)
            } else {
                (tag << 32) | 1
            };
            match bucket.compare_exchange_weak(cur, new, ORDERING, ORDERING) {
                Ok(_) => break,
                Err(x) => cur = x,
            }
        }
    }

    /// Record the completion of a request
    pub fn record_request_end(
        &self,
        latency: Duration,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) {
        self.active_requests.fetch_sub(1, ORDERING);
        self.completed_requests.fetch_add(1, ORDERING);

        // Update token counts
        self.prompt_tokens.fetch_add(prompt_tokens as u64, ORDERING);
        self.completion_tokens
            .fetch_add(completion_tokens as u64, ORDERING);

        // Update latency stats
        let latency_us = latency.as_micros() as u64;
        self.total_latency_us.fetch_add(latency_us, ORDERING);

        // Update min latency
        let mut current_min = self.min_latency_us.load(ORDERING);
        while latency_us < current_min {
            match self.min_latency_us.compare_exchange_weak(
                current_min,
                latency_us,
                ORDERING,
                ORDERING,
            ) {
                Ok(_) => break,
                Err(x) => current_min = x,
            }
        }

        // Update max latency
        let mut current_max = self.max_latency_us.load(ORDERING);
        while latency_us > current_max {
            match self.max_latency_us.compare_exchange_weak(
                current_max,
                latency_us,
                ORDERING,
                ORDERING,
            ) {
                Ok(_) => break,
                Err(x) => current_max = x,
            }
        }
    }

    /// Record an error response
    pub fn record_error(&self, status_code: u16) {
        self.total_errors.fetch_add(1, ORDERING);
        self.active_requests.fetch_sub(1, ORDERING);

        match status_code {
            429 => {
                self.rate_limit_errors.fetch_add(1, ORDERING);
            }
            500 | 503 => {
                self.server_errors.fetch_add(1, ORDERING);
            }
            504 => {
                self.timeout_errors.fetch_add(1, ORDERING);
            }
            _ => {}
        }
    }

    /// Try to reserve capacity for a new WebSocket connection.
    /// Returns true if a slot was successfully reserved, false if the cap is already reached.
    pub fn try_reserve_ws_connection(&self, max_connections: u64) -> bool {
        self.active_websocket_connections
            .fetch_update(ORDERING, ORDERING, |current| {
                (current < max_connections).then_some(current + 1)
            })
            .is_ok()
    }

    /// Release a previously reserved/open WebSocket connection slot.
    pub fn record_ws_disconnect(&self) {
        self.active_websocket_connections.fetch_sub(1, ORDERING);
    }

    /// Get the uptime of the server
    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Get requests per second (over the last RPS_WINDOW_SECS seconds)
    pub fn requests_per_second(&self) -> f64 {
        let now_tag = self.start_time.elapsed().as_secs() as u32;
        let mut total = 0u64;
        let mut oldest_age = 0u32;
        for bucket in &self.rps_buckets {
            let packed = bucket.load(ORDERING);
            let count = packed & 0xFFFF_FFFF;
            if count == 0 {
                continue;
            }
            // Wrapping subtraction so the comparison is correct as the tag wraps.
            let age = now_tag.wrapping_sub((packed >> 32) as u32);
            if (age as u64) < RPS_WINDOW_SECS {
                total += count;
                oldest_age = oldest_age.max(age);
            }
        }
        if total == 0 {
            return 0.0;
        }
        // Span covered, inclusive of the current (possibly partial) second.
        total as f64 / (oldest_age as f64 + 1.0)
    }

    /// Get average latency in milliseconds
    pub fn avg_latency_ms(&self) -> f64 {
        let completed = self.completed_requests.load(ORDERING);
        if completed == 0 {
            return 0.0;
        }
        let total_us = self.total_latency_us.load(ORDERING);
        (total_us as f64 / completed as f64) / 1000.0
    }

    /// Get minimum latency in milliseconds
    pub fn min_latency_ms(&self) -> Option<f64> {
        let min = self.min_latency_us.load(ORDERING);
        if min == u64::MAX {
            None
        } else {
            Some(min as f64 / 1000.0)
        }
    }

    /// Get maximum latency in milliseconds
    pub fn max_latency_ms(&self) -> Option<f64> {
        let max = self.max_latency_us.load(ORDERING);
        if max == 0 {
            None
        } else {
            Some(max as f64 / 1000.0)
        }
    }

    /// Get total tokens (prompt + completion)
    pub fn total_tokens(&self) -> u64 {
        self.prompt_tokens.load(ORDERING) + self.completion_tokens.load(ORDERING)
    }

    /// Get per-model request counts
    pub fn model_requests(&self) -> HashMap<String, u64> {
        self.model_requests
            .read()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.load(ORDERING)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get a snapshot of all stats for serialization
    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            uptime_secs: self.uptime().as_secs(),
            total_requests: self.total_requests.load(ORDERING),
            active_requests: self.active_requests.load(ORDERING),
            streaming_requests: self.streaming_requests.load(ORDERING),
            non_streaming_requests: self.non_streaming_requests.load(ORDERING),
            completions_requests: self.completions_requests.load(ORDERING),
            responses_requests: self.responses_requests.load(ORDERING),
            websocket_requests: self.websocket_requests.load(ORDERING),
            messages_requests: self.messages_requests.load(ORDERING),
            active_websocket_connections: self.active_websocket_connections.load(ORDERING),
            prompt_tokens: self.prompt_tokens.load(ORDERING),
            completion_tokens: self.completion_tokens.load(ORDERING),
            total_tokens: self.total_tokens(),
            total_errors: self.total_errors.load(ORDERING),
            rate_limit_errors: self.rate_limit_errors.load(ORDERING),
            server_errors: self.server_errors.load(ORDERING),
            timeout_errors: self.timeout_errors.load(ORDERING),
            requests_per_second: self.requests_per_second(),
            avg_latency_ms: self.avg_latency_ms(),
            min_latency_ms: self.min_latency_ms(),
            max_latency_ms: self.max_latency_ms(),
            model_requests: self.model_requests(),
        }
    }
}

/// A serializable snapshot of statistics
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StatsSnapshot {
    pub uptime_secs: u64,
    pub total_requests: u64,
    pub active_requests: u64,
    pub streaming_requests: u64,
    pub non_streaming_requests: u64,
    pub completions_requests: u64,
    pub responses_requests: u64,
    pub websocket_requests: u64,
    #[serde(default)]
    pub messages_requests: u64,
    pub active_websocket_connections: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub total_errors: u64,
    pub rate_limit_errors: u64,
    pub server_errors: u64,
    pub timeout_errors: u64,
    pub requests_per_second: f64,
    pub avg_latency_ms: f64,
    pub min_latency_ms: Option<f64>,
    pub max_latency_ms: Option<f64>,
    pub model_requests: HashMap<String, u64>,
}

/// Shared stats handle for use across threads
pub type SharedStats = Arc<Stats>;

/// Create a new shared stats instance
pub fn new_shared_stats() -> SharedStats {
    Arc::new(Stats::new())
}

fn normalize_model_name(model: &str) -> String {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return OTHER_MODELS_BUCKET.to_string();
    }

    if trimmed.len() <= MAX_MODEL_NAME_BYTES {
        return trimmed.to_string();
    }

    let mut end = MAX_MODEL_NAME_BYTES;
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    trimmed[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stats_basic() {
        let stats = Stats::new();

        stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
        assert_eq!(stats.total_requests.load(ORDERING), 1);
        assert_eq!(stats.active_requests.load(ORDERING), 1);
        assert_eq!(stats.non_streaming_requests.load(ORDERING), 1);
        assert_eq!(stats.completions_requests.load(ORDERING), 1);

        stats.record_request_end(Duration::from_millis(100), 50, 100);
        assert_eq!(stats.active_requests.load(ORDERING), 0);
        assert_eq!(stats.prompt_tokens.load(ORDERING), 50);
        assert_eq!(stats.completion_tokens.load(ORDERING), 100);
    }

    #[test]
    fn test_stats_streaming() {
        let stats = Stats::new();

        stats.record_request_start("gpt-4", true, EndpointType::ChatCompletions);
        assert_eq!(stats.streaming_requests.load(ORDERING), 1);
        assert_eq!(stats.non_streaming_requests.load(ORDERING), 0);
    }

    #[test]
    fn test_stats_errors() {
        let stats = Stats::new();

        stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
        stats.record_error(429);

        assert_eq!(stats.total_errors.load(ORDERING), 1);
        assert_eq!(stats.rate_limit_errors.load(ORDERING), 1);
        assert_eq!(stats.active_requests.load(ORDERING), 0);
    }

    #[test]
    fn test_stats_latency() {
        let stats = Stats::new();

        stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
        stats.record_request_end(Duration::from_millis(100), 10, 20);

        stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
        stats.record_request_end(Duration::from_millis(200), 10, 20);

        assert_eq!(stats.avg_latency_ms(), 150.0);
        assert_eq!(stats.min_latency_ms(), Some(100.0));
        assert_eq!(stats.max_latency_ms(), Some(200.0));
    }

    #[test]
    fn test_model_requests() {
        let stats = Stats::new();

        stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
        stats.record_request_start("gpt-4", false, EndpointType::Responses);
        stats.record_request_start("claude-3", true, EndpointType::Responses);

        let model_counts = stats.model_requests();
        assert_eq!(model_counts.get("gpt-4"), Some(&2));
        assert_eq!(model_counts.get("claude-3"), Some(&1));
    }

    #[test]
    fn test_model_requests_cardinality_is_bounded() {
        let stats = Stats::new();

        for i in 0..(MAX_TRACKED_MODELS + 10) {
            stats.record_request_start(
                &format!("attacker-model-{i}"),
                false,
                EndpointType::ChatCompletions,
            );
        }

        let model_counts = stats.model_requests();
        assert!(model_counts.len() <= MAX_TRACKED_MODELS + 1);
        assert!(model_counts.contains_key(OTHER_MODELS_BUCKET));
        assert_eq!(
            model_counts.values().sum::<u64>(),
            (MAX_TRACKED_MODELS + 10) as u64
        );
    }

    #[test]
    fn test_model_name_is_truncated() {
        let stats = Stats::new();
        let long_name = "a".repeat(MAX_MODEL_NAME_BYTES + 64);

        stats.record_request_start(&long_name, false, EndpointType::ChatCompletions);

        let model_counts = stats.model_requests();
        let stored = model_counts.keys().next().expect("model key should exist");
        assert!(stored.len() <= MAX_MODEL_NAME_BYTES);
    }

    #[test]
    fn test_requests_per_second_empty_is_zero() {
        let stats = Stats::new();
        assert_eq!(stats.requests_per_second(), 0.0);
    }

    #[test]
    fn test_requests_per_second_counts_recent() {
        let stats = Stats::new();
        for _ in 0..5 {
            stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
        }
        let rps = stats.requests_per_second();
        // All requests land within the first second(s) of uptime, so the
        // rolling window must report a positive rate reflecting them.
        assert!(rps > 0.0, "expected positive rps, got {rps}");
        assert!(rps.is_finite());
    }

    #[test]
    fn test_endpoint_types() {
        let stats = Stats::new();

        stats.record_request_start("gpt-4", false, EndpointType::ChatCompletions);
        stats.record_request_start("gpt-4", true, EndpointType::ChatCompletions);
        stats.record_request_start("gpt-5", false, EndpointType::Responses);

        assert_eq!(stats.completions_requests.load(ORDERING), 2);
        assert_eq!(stats.responses_requests.load(ORDERING), 1);
        assert_eq!(stats.total_requests.load(ORDERING), 3);
    }
}
