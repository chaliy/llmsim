// Latency Profiles Module
// Defines latency profiles for simulating realistic LLM response times.

use rand::Rng;
use rand_distr::{Distribution, Normal};
use std::time::Duration;

/// Latency profile for simulating LLM response timing
#[derive(Debug, Clone)]
pub struct LatencyProfile {
    /// Mean time to first token in milliseconds
    pub ttft_mean_ms: u64,
    /// Standard deviation for time to first token
    pub ttft_stddev_ms: u64,
    /// Mean time between tokens in milliseconds
    pub tbt_mean_ms: u64,
    /// Standard deviation for time between tokens
    pub tbt_stddev_ms: u64,
}

impl LatencyProfile {
    /// Create a new latency profile with custom values
    pub fn new(
        ttft_mean_ms: u64,
        ttft_stddev_ms: u64,
        tbt_mean_ms: u64,
        tbt_stddev_ms: u64,
    ) -> Self {
        Self {
            ttft_mean_ms,
            ttft_stddev_ms,
            tbt_mean_ms,
            tbt_stddev_ms,
        }
    }

    /// GPT-4 profile - slower, higher quality model
    /// Based on typical GPT-4 latency characteristics
    pub fn gpt4() -> Self {
        Self {
            ttft_mean_ms: 800,
            ttft_stddev_ms: 200,
            tbt_mean_ms: 50,
            tbt_stddev_ms: 15,
        }
    }

    /// GPT-4o profile - faster optimized model
    pub fn gpt4o() -> Self {
        Self {
            ttft_mean_ms: 400,
            ttft_stddev_ms: 100,
            tbt_mean_ms: 25,
            tbt_stddev_ms: 8,
        }
    }

    /// GPT-5 profile - flagship model with reasoning capabilities
    /// Designed for logic and multi-step tasks
    pub fn gpt5() -> Self {
        Self {
            ttft_mean_ms: 600,
            ttft_stddev_ms: 150,
            tbt_mean_ms: 40,
            tbt_stddev_ms: 12,
        }
    }

    /// GPT-5-mini profile - lightweight version for cost-sensitive applications
    pub fn gpt5_mini() -> Self {
        Self {
            ttft_mean_ms: 300,
            ttft_stddev_ms: 80,
            tbt_mean_ms: 20,
            tbt_stddev_ms: 6,
        }
    }

    /// O3/O4 reasoning model profile - slower due to chain-of-thought
    pub fn o_series() -> Self {
        Self {
            ttft_mean_ms: 2000,
            ttft_stddev_ms: 500,
            tbt_mean_ms: 30,
            tbt_stddev_ms: 10,
        }
    }

    /// Claude Opus profile - Anthropic flagship model
    pub fn claude_opus() -> Self {
        Self {
            ttft_mean_ms: 1000,
            ttft_stddev_ms: 250,
            tbt_mean_ms: 60,
            tbt_stddev_ms: 20,
        }
    }

    /// Claude Sonnet profile - balanced speed/quality
    pub fn claude_sonnet() -> Self {
        Self {
            ttft_mean_ms: 500,
            ttft_stddev_ms: 120,
            tbt_mean_ms: 30,
            tbt_stddev_ms: 10,
        }
    }

    /// Claude Haiku profile - fastest Claude model
    pub fn claude_haiku() -> Self {
        Self {
            ttft_mean_ms: 200,
            ttft_stddev_ms: 50,
            tbt_mean_ms: 15,
            tbt_stddev_ms: 5,
        }
    }

    /// Gemini Pro profile
    pub fn gemini_pro() -> Self {
        Self {
            ttft_mean_ms: 600,
            ttft_stddev_ms: 150,
            tbt_mean_ms: 35,
            tbt_stddev_ms: 10,
        }
    }

    /// Instant profile - no delay (for fast tests)
    pub fn instant() -> Self {
        Self {
            ttft_mean_ms: 0,
            ttft_stddev_ms: 0,
            tbt_mean_ms: 0,
            tbt_stddev_ms: 0,
        }
    }

    /// Fast profile - minimal delays for quick testing
    pub fn fast() -> Self {
        Self {
            ttft_mean_ms: 10,
            ttft_stddev_ms: 2,
            tbt_mean_ms: 1,
            tbt_stddev_ms: 0,
        }
    }

    /// Get a profile based on model name
    pub fn from_model(model: &str) -> Self {
        let model_lower = model.to_lowercase();

        // GPT-5 family (check specific variants first)
        if model_lower.contains("gpt-5-mini") {
            Self::gpt5_mini()
        } else if model_lower.contains("gpt-5") {
            Self::gpt5()
        // O-series reasoning models (o3, o4)
        } else if model_lower.starts_with("o3") || model_lower.starts_with("o4") {
            Self::o_series()
        // GPT-4 family
        } else if model_lower.contains("gpt-4o") {
            Self::gpt4o()
        } else if model_lower.contains("gpt-4") {
            Self::gpt4()
        // Claude family
        } else if model_lower.contains("opus") {
            Self::claude_opus()
        } else if model_lower.contains("sonnet") {
            Self::claude_sonnet()
        } else if model_lower.contains("haiku") {
            Self::claude_haiku()
        // Gemini
        } else if model_lower.contains("gemini") {
            Self::gemini_pro()
        } else {
            // Default to GPT-5-like latency
            Self::gpt5()
        }
    }

    /// Sample time to first token using normal distribution
    pub fn sample_ttft(&self) -> Duration {
        if self.ttft_mean_ms == 0 {
            return Duration::ZERO;
        }

        let mut rng = rand::rng();
        let sample = if self.ttft_stddev_ms > 0 {
            let normal = Normal::new(self.ttft_mean_ms as f64, self.ttft_stddev_ms as f64)
                .unwrap_or_else(|_| Normal::new(self.ttft_mean_ms as f64, 1.0).unwrap());
            // Ensure minimum of 1ms when mean is non-zero
            normal.sample(&mut rng).max(1.0) as u64
        } else {
            self.ttft_mean_ms
        };

        Duration::from_millis(sample)
    }

    /// Sample time between tokens using normal distribution
    pub fn sample_tbt(&self) -> Duration {
        if self.tbt_mean_ms == 0 {
            return Duration::ZERO;
        }

        let mut rng = rand::rng();
        let sample = if self.tbt_stddev_ms > 0 {
            let normal = Normal::new(self.tbt_mean_ms as f64, self.tbt_stddev_ms as f64)
                .unwrap_or_else(|_| Normal::new(self.tbt_mean_ms as f64, 1.0).unwrap());
            // Ensure minimum of 1ms when mean is non-zero
            normal.sample(&mut rng).max(1.0) as u64
        } else {
            self.tbt_mean_ms
        };

        Duration::from_millis(sample)
    }

    /// Sample a variable delay with jitter (0.5x to 1.5x of base)
    pub fn sample_with_jitter(&self, base_ms: u64) -> Duration {
        let mut rng = rand::rng();
        let factor = rng.random_range(0.5..1.5);
        Duration::from_millis((base_ms as f64 * factor) as u64)
    }
}

impl Default for LatencyProfile {
    fn default() -> Self {
        Self::gpt4()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preset_profiles() {
        let gpt5 = LatencyProfile::gpt5();
        assert!(gpt5.ttft_mean_ms > 0);
        assert!(gpt5.tbt_mean_ms > 0);

        let gpt5_mini = LatencyProfile::gpt5_mini();
        assert!(gpt5_mini.ttft_mean_ms < gpt5.ttft_mean_ms); // GPT-5-mini should be faster

        let instant = LatencyProfile::instant();
        assert_eq!(instant.ttft_mean_ms, 0);
        assert_eq!(instant.tbt_mean_ms, 0);
    }

    #[test]
    fn test_gpt5_family() {
        let gpt5 = LatencyProfile::gpt5();
        let gpt5_mini = LatencyProfile::gpt5_mini();

        // Mini should be faster than standard
        assert!(gpt5_mini.ttft_mean_ms < gpt5.ttft_mean_ms);
    }

    #[test]
    fn test_sample_ttft() {
        let profile = LatencyProfile::gpt4();
        let samples: Vec<Duration> = (0..100).map(|_| profile.sample_ttft()).collect();

        // All samples should be positive
        assert!(samples.iter().all(|d| *d > Duration::ZERO));

        // Samples should vary (not all the same)
        let first = samples[0];
        assert!(samples.iter().any(|d| *d != first));
    }

    #[test]
    fn test_sample_tbt() {
        let profile = LatencyProfile::gpt4();
        let samples: Vec<Duration> = (0..100).map(|_| profile.sample_tbt()).collect();

        // All samples should be positive
        assert!(samples.iter().all(|d| *d > Duration::ZERO));
    }

    #[test]
    fn test_instant_profile_zero_delay() {
        let profile = LatencyProfile::instant();
        let ttft = profile.sample_ttft();
        let tbt = profile.sample_tbt();
        assert_eq!(ttft, Duration::ZERO);
        assert_eq!(tbt, Duration::ZERO);
    }

    #[test]
    fn test_from_model() {
        // GPT-5 family
        let gpt5 = LatencyProfile::from_model("gpt-5");
        assert_eq!(gpt5.ttft_mean_ms, LatencyProfile::gpt5().ttft_mean_ms);

        let gpt5_mini = LatencyProfile::from_model("gpt-5-mini");
        assert_eq!(
            gpt5_mini.ttft_mean_ms,
            LatencyProfile::gpt5_mini().ttft_mean_ms
        );

        // GPT-5.1/5.2 variants
        let gpt51 = LatencyProfile::from_model("gpt-5.1");
        assert_eq!(gpt51.ttft_mean_ms, LatencyProfile::gpt5().ttft_mean_ms);

        let gpt52 = LatencyProfile::from_model("gpt-5.2");
        assert_eq!(gpt52.ttft_mean_ms, LatencyProfile::gpt5().ttft_mean_ms);

        // O-series reasoning models (o3, o4)
        let o3 = LatencyProfile::from_model("o3-mini");
        assert_eq!(o3.ttft_mean_ms, LatencyProfile::o_series().ttft_mean_ms);

        let o4 = LatencyProfile::from_model("o4-mini");
        assert_eq!(o4.ttft_mean_ms, LatencyProfile::o_series().ttft_mean_ms);

        // GPT-4 family
        let gpt4 = LatencyProfile::from_model("gpt-4-turbo");
        assert_eq!(gpt4.ttft_mean_ms, LatencyProfile::gpt4().ttft_mean_ms);

        let gpt4o = LatencyProfile::from_model("gpt-4o-mini");
        assert_eq!(gpt4o.ttft_mean_ms, LatencyProfile::gpt4o().ttft_mean_ms);

        // Claude family
        let claude = LatencyProfile::from_model("claude-3-opus-20240229");
        assert_eq!(
            claude.ttft_mean_ms,
            LatencyProfile::claude_opus().ttft_mean_ms
        );
    }

    #[test]
    fn test_custom_profile() {
        let custom = LatencyProfile::new(100, 10, 5, 1);
        assert_eq!(custom.ttft_mean_ms, 100);
        assert_eq!(custom.tbt_mean_ms, 5);
    }

    #[test]
    fn test_distribution_sanity() {
        let profile = LatencyProfile::gpt4();

        // Take many samples and check they're roughly normally distributed
        let samples: Vec<u64> = (0..1000)
            .map(|_| profile.sample_ttft().as_millis() as u64)
            .collect();

        let mean: f64 = samples.iter().sum::<u64>() as f64 / samples.len() as f64;

        // Mean should be close to ttft_mean_ms (within 20%)
        let expected_mean = profile.ttft_mean_ms as f64;
        let tolerance = expected_mean * 0.2;
        assert!(
            (mean - expected_mean).abs() < tolerance,
            "Mean {} not within tolerance of expected {}",
            mean,
            expected_mean
        );
    }
}
