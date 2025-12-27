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

/// Type of API endpoint being called
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointType {
    /// Chat Completions API (/openai/v1/chat/completions)
    ChatCompletions,
    /// Responses API (/openai/v1/responses)
    Responses,
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

    // Per-model request counts
    model_requests: RwLock<HashMap<String, u64>>,

    // Latency tracking (in microseconds)
    /// Total latency for calculating average
    total_latency_us: AtomicU64,
    /// Count of completed requests (for average calculation)
    completed_requests: AtomicU64,
    /// Minimum latency seen
    min_latency_us: AtomicU64,
    /// Maximum latency seen
    max_latency_us: AtomicU64,

    // Rolling window for RPS calculation
    request_times: RwLock<Vec<Instant>>,
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
            request_times: RwLock::new(Vec::new()),
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
        }

        // Track per-model requests
        if let Ok(mut map) = self.model_requests.write() {
            *map.entry(model.to_string()).or_insert(0) += 1;
        }

        // Add to rolling window
        if let Ok(mut times) = self.request_times.write() {
            times.push(Instant::now());
            // Keep only last 60 seconds of requests
            let cutoff = Instant::now() - Duration::from_secs(60);
            times.retain(|t| *t > cutoff);
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

    /// Get the uptime of the server
    pub fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Get requests per second (over the last 60 seconds)
    pub fn requests_per_second(&self) -> f64 {
        if let Ok(times) = self.request_times.read() {
            let now = Instant::now();
            let cutoff = now - Duration::from_secs(60);
            let recent_count = times.iter().filter(|t| **t > cutoff).count();

            if recent_count == 0 {
                return 0.0;
            }

            // Calculate actual time window
            let oldest = times.iter().filter(|t| **t > cutoff).min();
            if let Some(oldest) = oldest {
                let window = now.duration_since(*oldest).as_secs_f64();
                if window > 0.0 {
                    return recent_count as f64 / window;
                }
            }
        }
        0.0
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
            .map(|m| m.clone())
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
