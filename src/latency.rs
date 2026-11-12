// Latency Profiles Module
// Defines latency profiles for simulating realistic LLM response times.
//
// Decisions:
// - Time to first token is drawn from a log-normal distribution fitted to the
//   profile's mean and stddev. Real TTFT is right-skewed: most calls land near
//   the median and a few take several times longer. A normal draw clipped at
//   1 ms had no tail at all. Inter-token gaps stay normal (they are small and
//   averaged over hundreds of tokens).
// - Prefill: `with_input_tokens` adds `prefill_us_per_token` per prompt token
//   to the mean TTFT, so a 200k-token context starts noticeably later than a
//   one-line prompt. The default 50 us/token (about 20k tokens/s) is a typical
//   hosted prefill rate; `instant` and `fast` have none.
// - GPT-5.6+/GPT-6 tier profiles (Sol, Terra, Luna, Astra) and Muse Spark come
//   from everruns production timings measured on 2026-10-28 for full agent
//   turns (TTFT includes default reasoning): Luna 2.9 s and 108 tokens/s, Sol
//   2.0 s and 46 tokens/s, Muse Spark 12.7 s and 136 tokens/s. Terra and Astra
//   were not measured; they are placed between and above Sol by tier.

use rand::RngExt;
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
    /// Prefill cost per input token in microseconds, added to the mean TTFT
    /// by [`LatencyProfile::with_input_tokens`].
    pub prefill_us_per_token: u64,
}

/// Default prefill cost: about 20k input tokens per second.
pub const DEFAULT_PREFILL_US_PER_TOKEN: u64 = 50;

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
            prefill_us_per_token: DEFAULT_PREFILL_US_PER_TOKEN,
        }
    }

    /// Set the prefill cost per input token (microseconds).
    pub fn with_prefill_us_per_token(mut self, us: u64) -> Self {
        self.prefill_us_per_token = us;
        self
    }

    /// This profile with prefill for `input_tokens` added to the mean TTFT.
    /// Profiles with a zero TTFT (`instant`) stay instant.
    pub fn with_input_tokens(mut self, input_tokens: usize) -> Self {
        if self.ttft_mean_ms == 0 {
            return self;
        }
        let prefill_ms = (input_tokens as u64).saturating_mul(self.prefill_us_per_token) / 1000;
        self.ttft_mean_ms = self.ttft_mean_ms.saturating_add(prefill_ms);
        self
    }

    /// This profile with TTFT scaled for a reasoning effort. Hidden reasoning
    /// happens before the first visible token, so more effort means a later
    /// start. Profiles are calibrated at the default (`medium`) effort.
    pub fn with_reasoning_effort(mut self, effort: &str) -> Self {
        let factor = match effort {
            "none" => 0.35,
            "minimal" => 0.5,
            "low" => 0.7,
            "high" => 1.8,
            "xhigh" => 3.0,
            "max" => 4.5,
            _ => 1.0, // medium / unknown
        };
        self.ttft_mean_ms = (self.ttft_mean_ms as f64 * factor) as u64;
        self.ttft_stddev_ms = (self.ttft_stddev_ms as f64 * factor) as u64;
        self
    }

    /// GPT-4 profile - slower, higher quality model
    /// Based on typical GPT-4 latency characteristics
    pub fn gpt4() -> Self {
        Self {
            ttft_mean_ms: 800,
            ttft_stddev_ms: 200,
            tbt_mean_ms: 50,
            tbt_stddev_ms: 15,
            prefill_us_per_token: DEFAULT_PREFILL_US_PER_TOKEN,
        }
    }

    /// GPT-4o profile - faster optimized model
    pub fn gpt4o() -> Self {
        Self {
            ttft_mean_ms: 400,
            ttft_stddev_ms: 100,
            tbt_mean_ms: 25,
            tbt_stddev_ms: 8,
            prefill_us_per_token: DEFAULT_PREFILL_US_PER_TOKEN,
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
            prefill_us_per_token: DEFAULT_PREFILL_US_PER_TOKEN,
        }
    }

    /// GPT-5-mini profile - lightweight version for cost-sensitive applications
    pub fn gpt5_mini() -> Self {
        Self {
            ttft_mean_ms: 300,
            ttft_stddev_ms: 80,
            tbt_mean_ms: 20,
            tbt_stddev_ms: 6,
            prefill_us_per_token: DEFAULT_PREFILL_US_PER_TOKEN,
        }
    }

    /// O3/O4 reasoning model profile - slower due to chain-of-thought
    pub fn o_series() -> Self {
        Self {
            ttft_mean_ms: 2000,
            ttft_stddev_ms: 500,
            tbt_mean_ms: 30,
            tbt_stddev_ms: 10,
            prefill_us_per_token: DEFAULT_PREFILL_US_PER_TOKEN,
        }
    }

    /// Claude Opus profile - Anthropic flagship model
    pub fn claude_opus() -> Self {
        Self {
            ttft_mean_ms: 1000,
            ttft_stddev_ms: 250,
            tbt_mean_ms: 60,
            tbt_stddev_ms: 20,
            prefill_us_per_token: DEFAULT_PREFILL_US_PER_TOKEN,
        }
    }

    /// Claude Sonnet profile - balanced speed/quality
    pub fn claude_sonnet() -> Self {
        Self {
            ttft_mean_ms: 500,
            ttft_stddev_ms: 120,
            tbt_mean_ms: 30,
            tbt_stddev_ms: 10,
            prefill_us_per_token: DEFAULT_PREFILL_US_PER_TOKEN,
        }
    }

    /// Claude Haiku profile - fastest Claude model
    pub fn claude_haiku() -> Self {
        Self {
            ttft_mean_ms: 200,
            ttft_stddev_ms: 50,
            tbt_mean_ms: 15,
            tbt_stddev_ms: 5,
            prefill_us_per_token: DEFAULT_PREFILL_US_PER_TOKEN,
        }
    }

    /// Gemini Pro profile
    pub fn gemini_pro() -> Self {
        Self {
            ttft_mean_ms: 600,
            ttft_stddev_ms: 150,
            tbt_mean_ms: 35,
            tbt_stddev_ms: 10,
            prefill_us_per_token: DEFAULT_PREFILL_US_PER_TOKEN,
        }
    }

    /// Gemini Flash profile - fast inference
    pub fn gemini_flash() -> Self {
        Self {
            ttft_mean_ms: 200,
            ttft_stddev_ms: 50,
            tbt_mean_ms: 15,
            tbt_stddev_ms: 5,
            prefill_us_per_token: DEFAULT_PREFILL_US_PER_TOKEN,
        }
    }

    /// DeepSeek profile - general chat model
    pub fn deepseek() -> Self {
        Self {
            ttft_mean_ms: 500,
            ttft_stddev_ms: 120,
            tbt_mean_ms: 30,
            tbt_stddev_ms: 10,
            prefill_us_per_token: DEFAULT_PREFILL_US_PER_TOKEN,
        }
    }

    /// DeepSeek Reasoner profile - reasoning model with chain-of-thought
    pub fn deepseek_reasoner() -> Self {
        Self {
            ttft_mean_ms: 1500,
            ttft_stddev_ms: 400,
            tbt_mean_ms: 25,
            tbt_stddev_ms: 8,
            prefill_us_per_token: DEFAULT_PREFILL_US_PER_TOKEN,
        }
    }

    /// GPT-5.6+/GPT-6 Sol (frontier tier). Measured: TTFT about 2.0 s,
    /// 46 tokens/s.
    pub fn gpt_sol() -> Self {
        Self::new(2000, 500, 22, 6)
    }

    /// GPT-5.6+/GPT-6 Terra (balanced tier). Not measured; between Sol and
    /// Luna.
    pub fn gpt_terra() -> Self {
        Self::new(2400, 600, 14, 4)
    }

    /// Embedding models (text-embedding-3 family).
    pub fn embedding() -> Self {
        // Embedding calls return in roughly 100-300 ms; prefill dominates
        // for large batches. No streaming, so no token gap.
        Self::new(120, 40, 0, 0).with_prefill_us_per_token(2)
    }

    /// GPT-5.6+/GPT-6 Luna (cost-efficient tier). Measured: TTFT about 2.9 s
    /// (2-4.5 s), 108 tokens/s. Luna thinks before it answers, so its first
    /// token is later than Sol's even though it streams faster.
    pub fn gpt_luna() -> Self {
        Self::new(2900, 800, 9, 3)
    }

    /// GPT-6 Astra (most capable tier). Not measured; slower than Sol.
    pub fn gpt_astra() -> Self {
        Self::new(3500, 1000, 30, 8)
    }

    /// Muse Spark. Measured: TTFT about 12.7 s, 136 tokens/s.
    pub fn muse_spark() -> Self {
        Self::new(12700, 3000, 7, 2)
    }

    /// TypeSafe Jev (System One) profile. A verdict is one forward pass with
    /// no generated text, so latency is a single short wait with no tokens.
    pub fn jev() -> Self {
        Self {
            ttft_mean_ms: 150,
            ttft_stddev_ms: 40,
            tbt_mean_ms: 0,
            tbt_stddev_ms: 0,
            prefill_us_per_token: DEFAULT_PREFILL_US_PER_TOKEN,
        }
    }

    /// Instant profile - no delay (for fast tests)
    pub fn instant() -> Self {
        Self {
            ttft_mean_ms: 0,
            ttft_stddev_ms: 0,
            tbt_mean_ms: 0,
            tbt_stddev_ms: 0,
            prefill_us_per_token: 0,
        }
    }

    /// Fast profile - minimal delays for quick testing
    pub fn fast() -> Self {
        Self {
            ttft_mean_ms: 10,
            ttft_stddev_ms: 2,
            tbt_mean_ms: 1,
            tbt_stddev_ms: 0,
            prefill_us_per_token: 0,
        }
    }

    /// Get a profile based on model name
    pub fn from_model(model: &str) -> Self {
        let model_lower = model.to_lowercase();

        let gpt = model_lower.starts_with("gpt-");
        if model_lower.contains("embedding") {
            return Self::embedding();
        }
        // GPT-5.6+/GPT-6 tiers (check before the generic GPT-5 family)
        if gpt && model_lower.contains("luna") {
            Self::gpt_luna()
        } else if gpt && model_lower.contains("terra") {
            Self::gpt_terra()
        } else if gpt && model_lower.contains("astra") {
            Self::gpt_astra()
        } else if gpt
            && (model_lower.contains("sol")
                || model_lower.starts_with("gpt-5.6")
                || model_lower.starts_with("gpt-6"))
        {
            // Bare `gpt-5.6` / `gpt-6.1` are Sol aliases.
            Self::gpt_sol()
        } else if model_lower.starts_with("muse-spark") {
            Self::muse_spark()
        } else if model_lower.contains("gpt-5-mini") {
            Self::gpt5_mini()
        } else if model_lower.contains("gpt-5") || model_lower.contains("gpt-6") {
            Self::gpt5()
        // O-series reasoning models (o1, o3, o4)
        } else if model_lower.starts_with("o1")
            || model_lower.starts_with("o3")
            || model_lower.starts_with("o4")
        {
            Self::o_series()
        // GPT-4 family
        } else if model_lower.contains("gpt-4o") {
            Self::gpt4o()
        } else if model_lower.contains("gpt-4") {
            Self::gpt4()
        // Claude family
        } else if model_lower.contains("opus") || model_lower.contains("fable") {
            Self::claude_opus()
        } else if model_lower.contains("sonnet") {
            Self::claude_sonnet()
        } else if model_lower.contains("haiku") {
            Self::claude_haiku()
        // Gemini family
        } else if model_lower.contains("gemini") && model_lower.contains("flash") {
            Self::gemini_flash()
        } else if model_lower.contains("gemini") {
            Self::gemini_pro()
        // DeepSeek family
        } else if model_lower.contains("deepseek-reasoner") || model_lower.contains("deepseek-r") {
            Self::deepseek_reasoner()
        } else if model_lower.contains("deepseek") {
            Self::deepseek()
        // TypeSafe System One (Jev)
        } else if model_lower.starts_with("jev-") {
            Self::jev()
        } else {
            // Default to GPT-5-like latency
            Self::gpt5()
        }
    }

    /// Sample time to first token from a log-normal distribution with the
    /// profile's mean and stddev (right-skewed: a long tail of slow starts).
    pub fn sample_ttft(&self) -> Duration {
        if self.ttft_mean_ms == 0 {
            return Duration::ZERO;
        }

        let mut rng = rand::rng();
        let sample = sample_lognormal_ms(self.ttft_mean_ms, self.ttft_stddev_ms, &mut rng);

        Duration::from_millis(sample)
    }

    /// Sample time between tokens using normal distribution
    pub fn sample_tbt(&self) -> Duration {
        if self.tbt_mean_ms == 0 {
            return Duration::ZERO;
        }

        let mut rng = rand::rng();
        let sample = sample_normal_ms(self.tbt_mean_ms, self.tbt_stddev_ms, &mut rng);

        Duration::from_millis(sample)
    }

    /// Sample a variable delay with jitter (0.5x to 1.5x of base)
    pub fn sample_with_jitter(&self, base_ms: u64) -> Duration {
        let mut rng = rand::rng();
        let factor = rng.random_range(0.5..1.5);
        Duration::from_millis((base_ms as f64 * factor) as u64)
    }
}

fn standard_normal(rng: &mut impl rand::Rng) -> f64 {
    let u1: f64 = rng.random_range(f64::MIN_POSITIVE..1.0);
    let u2: f64 = rng.random_range(0.0..1.0);
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

/// Log-normal sample whose mean and stddev match the arguments.
fn sample_lognormal_ms(mean_ms: u64, stddev_ms: u64, rng: &mut impl rand::Rng) -> u64 {
    if stddev_ms == 0 {
        return mean_ms;
    }
    let mean = mean_ms as f64;
    let cv = stddev_ms as f64 / mean;
    let sigma2 = (1.0 + cv * cv).ln();
    let mu = mean.ln() - sigma2 / 2.0;
    (mu + sigma2.sqrt() * standard_normal(rng)).exp().max(1.0) as u64
}

fn sample_normal_ms(mean_ms: u64, stddev_ms: u64, rng: &mut impl rand::Rng) -> u64 {
    if stddev_ms == 0 {
        return mean_ms;
    }

    let z = standard_normal(rng);

    (mean_ms as f64 + stddev_ms as f64 * z).max(1.0) as u64
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

        // GPT-5.6 / GPT-6 tiers use measured per-tier profiles
        for (model, expected) in [
            ("gpt-6-sol", LatencyProfile::gpt_sol()),
            ("gpt-6.1-sol", LatencyProfile::gpt_sol()),
            ("gpt-5.6", LatencyProfile::gpt_sol()),
            ("gpt-6-luna", LatencyProfile::gpt_luna()),
            ("gpt-5.6-luna", LatencyProfile::gpt_luna()),
            ("gpt-6.1-terra", LatencyProfile::gpt_terra()),
            ("gpt-6-astra", LatencyProfile::gpt_astra()),
            ("muse-spark-1.3", LatencyProfile::muse_spark()),
        ] {
            let got = LatencyProfile::from_model(model);
            assert_eq!(got.ttft_mean_ms, expected.ttft_mean_ms, "{model}");
            assert_eq!(got.tbt_mean_ms, expected.tbt_mean_ms, "{model}");
        }
        // Luna starts later than Sol but streams faster (measured)
        let (luna, sol) = (LatencyProfile::gpt_luna(), LatencyProfile::gpt_sol());
        assert!(luna.ttft_mean_ms > sol.ttft_mean_ms);
        assert!(luna.tbt_mean_ms < sol.tbt_mean_ms);

        // Claude Fable uses the Opus-tier profile
        let fable = LatencyProfile::from_model("claude-fable-5-1");
        assert_eq!(
            fable.ttft_mean_ms,
            LatencyProfile::claude_opus().ttft_mean_ms
        );

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

    #[test]
    fn ttft_is_right_skewed_with_matching_mean() {
        let profile = LatencyProfile::gpt_luna();
        let mut samples: Vec<u64> = (0..20_000)
            .map(|_| profile.sample_ttft().as_millis() as u64)
            .collect();
        samples.sort_unstable();
        let mean = samples.iter().sum::<u64>() as f64 / samples.len() as f64;
        let median = samples[samples.len() / 2] as f64;
        let p99 = samples[samples.len() * 99 / 100] as f64;
        let expected = profile.ttft_mean_ms as f64;
        assert!((mean - expected).abs() < expected * 0.05, "mean {mean}");
        // Log-normal: median below the mean, and a long upper tail.
        assert!(median < mean, "median {median} mean {mean}");
        assert!(p99 > expected * 1.6, "p99 {p99}");
    }

    #[test]
    fn input_tokens_add_prefill_to_ttft() {
        let base = LatencyProfile::gpt_sol();
        let long = base.clone().with_input_tokens(200_000);
        // 200k tokens at the default 50 us/token = 10 s of prefill.
        assert_eq!(long.ttft_mean_ms, base.ttft_mean_ms + 10_000);
        let none = base
            .clone()
            .with_prefill_us_per_token(0)
            .with_input_tokens(200_000);
        assert_eq!(none.ttft_mean_ms, base.ttft_mean_ms);
        // Instant and fast profiles never gain prefill.
        assert_eq!(
            LatencyProfile::instant()
                .with_input_tokens(1_000_000)
                .ttft_mean_ms,
            0
        );
        assert_eq!(
            LatencyProfile::fast()
                .with_input_tokens(1_000_000)
                .ttft_mean_ms,
            LatencyProfile::fast().ttft_mean_ms
        );
    }

    #[test]
    fn reasoning_effort_scales_ttft() {
        let base = LatencyProfile::gpt_sol();
        assert_eq!(
            base.clone().with_reasoning_effort("medium").ttft_mean_ms,
            base.ttft_mean_ms
        );
        assert!(base.clone().with_reasoning_effort("xhigh").ttft_mean_ms > base.ttft_mean_ms * 2);
        assert!(base.clone().with_reasoning_effort("none").ttft_mean_ms < base.ttft_mean_ms / 2);
        assert_eq!(
            LatencyProfile::instant()
                .with_reasoning_effort("max")
                .ttft_mean_ms,
            0
        );
    }
}
