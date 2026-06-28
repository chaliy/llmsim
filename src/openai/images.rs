// OpenAI Image Generation API types.
//
// Models the `/v1/images/generations` endpoint used by the gpt-image family
// (the "ChatGPT Images" capability). Covers both the non-streaming JSON
// response and the streaming SSE events (`image_generation.partial_image` and
// `image_generation.completed`).
//
// Reference: https://platform.openai.com/docs/api-reference/images

use crate::latency::LatencyProfile;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Default model when a request omits `model`.
fn default_image_model() -> String {
    "gpt-image-1".to_string()
}

/// Request body for `POST /openai/v1/images/generations`.
#[derive(Debug, Clone, Deserialize)]
pub struct ImageGenerationRequest {
    /// Text description of the desired image(s).
    pub prompt: String,
    /// Model id (e.g. "gpt-image-1"). Defaults to gpt-image-1.
    #[serde(default = "default_image_model")]
    pub model: String,
    /// Number of images to generate (1-10).
    pub n: Option<u32>,
    /// Image size, e.g. "1024x1024", "1024x1536", "1536x1024", or "auto".
    pub size: Option<String>,
    /// Quality: "low" | "medium" | "high" | "auto".
    pub quality: Option<String>,
    /// DALL·E response format ("url" | "b64_json"). gpt-image always returns b64.
    pub response_format: Option<String>,
    /// Output encoding for gpt-image models ("png" | "jpeg" | "webp").
    pub output_format: Option<String>,
    /// Background: "transparent" | "opaque" | "auto".
    pub background: Option<String>,
    /// Whether to stream partial images via SSE.
    #[serde(default)]
    pub stream: bool,
    /// Number of progressive partial images to stream (0-3).
    pub partial_images: Option<u32>,
    /// Moderation strictness ("auto" | "low").
    pub moderation: Option<String>,
    /// End-user identifier for abuse tracking (ignored by the simulator).
    pub user: Option<String>,
}

/// Resolved, validated parameters derived from a request.
#[derive(Debug, Clone)]
pub struct ResolvedImageParams {
    pub width: u32,
    pub height: u32,
    pub size: String,
    pub quality: String,
    pub output_format: String,
    pub background: String,
    pub n: u32,
    pub partial_images: u32,
}

impl ImageGenerationRequest {
    /// Resolve defaults and clamp values to the simulator's supported ranges.
    pub fn resolve(&self) -> ResolvedImageParams {
        let size = self
            .size
            .clone()
            .filter(|s| s != "auto")
            .unwrap_or_else(|| "1024x1024".to_string());
        let (width, height) = parse_size(&size);
        let quality = match self.quality.as_deref() {
            Some("low") => "low",
            Some("medium") => "medium",
            Some("high") => "high",
            _ => "high", // "auto"/None map to high, gpt-image's default feel
        }
        .to_string();
        let output_format = match self.output_format.as_deref() {
            Some("jpeg") => "jpeg",
            Some("webp") => "webp",
            _ => "png",
        }
        .to_string();
        let background = match self.background.as_deref() {
            Some("transparent") => "transparent",
            Some("opaque") => "opaque",
            _ => "opaque",
        }
        .to_string();
        let n = self.n.unwrap_or(1).clamp(1, 10);
        let partial_images = self.partial_images.unwrap_or(0).min(3);

        ResolvedImageParams {
            width,
            height,
            size,
            quality,
            output_format,
            background,
            n,
            partial_images,
        }
    }
}

/// Parse a "WxH" size string into pixel dimensions, defaulting to 1024x1024.
pub fn parse_size(size: &str) -> (u32, u32) {
    let mut parts = size.split(['x', 'X']);
    let w = parts.next().and_then(|s| s.trim().parse::<u32>().ok());
    let h = parts.next().and_then(|s| s.trim().parse::<u32>().ok());
    match (w, h) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w.clamp(16, 4096), h.clamp(16, 4096)),
        _ => (1024, 1024),
    }
}

/// One generated image in a non-streaming response.
#[derive(Debug, Clone, Serialize)]
pub struct ImageData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b64_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
}

/// Detailed breakdown of input tokens for image usage.
#[derive(Debug, Clone, Serialize)]
pub struct ImageInputTokensDetails {
    pub text_tokens: u32,
    pub image_tokens: u32,
}

/// Token usage for an image generation.
#[derive(Debug, Clone, Serialize)]
pub struct ImagesUsage {
    pub total_tokens: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub input_tokens_details: ImageInputTokensDetails,
}

/// Non-streaming response for `POST /openai/v1/images/generations`.
#[derive(Debug, Clone, Serialize)]
pub struct ImageGenerationResponse {
    pub created: i64,
    pub data: Vec<ImageData>,
    pub usage: ImagesUsage,
    pub size: String,
    pub quality: String,
    pub output_format: String,
    pub background: String,
}

/// Streaming `image_generation.partial_image` event payload.
#[derive(Debug, Clone, Serialize)]
pub struct PartialImageEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub b64_json: String,
    pub created_at: i64,
    pub size: String,
    pub quality: String,
    pub background: String,
    pub output_format: String,
    pub partial_image_index: u32,
}

/// Streaming `image_generation.completed` event payload.
#[derive(Debug, Clone, Serialize)]
pub struct CompletedImageEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub b64_json: String,
    pub created_at: i64,
    pub size: String,
    pub quality: String,
    pub background: String,
    pub output_format: String,
    pub usage: ImagesUsage,
}

/// Build the SSE wire frame for a partial image event.
pub fn partial_image_sse(event: &PartialImageEvent) -> String {
    let data = serde_json::to_string(event).unwrap_or_default();
    format!("event: image_generation.partial_image\ndata: {}\n\n", data)
}

/// Build the SSE wire frame for the completed image event.
pub fn completed_image_sse(event: &CompletedImageEvent) -> String {
    let data = serde_json::to_string(event).unwrap_or_default();
    format!("event: image_generation.completed\ndata: {}\n\n", data)
}

/// Estimate the number of output (image) tokens for a generation, scaled by
/// size and quality. Baselines are anchored to gpt-image-1 at 1024x1024.
pub fn estimate_image_tokens(width: u32, height: u32, quality: &str) -> u32 {
    let base = match quality {
        "low" => 272.0,
        "medium" => 1056.0,
        _ => 4160.0, // high / auto
    };
    let area_ratio = (width as f64 * height as f64) / (1024.0 * 1024.0);
    (base * area_ratio).round() as u32
}

/// Total wall-clock time to "generate" the image(s).
///
/// Anchored to the configured latency profile: gpt-5's 600 ms TTFT baseline
/// maps to realistic image-generation times, while `instant`/`fast` profiles
/// (used by tests and load runs) collapse the wait to near zero.
pub fn image_total_duration(
    latency: &LatencyProfile,
    quality: &str,
    width: u32,
    height: u32,
    n: u32,
) -> Duration {
    if latency.ttft_mean_ms == 0 {
        return Duration::ZERO;
    }
    let speed_factor = latency.ttft_mean_ms as f64 / 600.0;
    let base_ms = match quality {
        "low" => 4_000.0,
        "medium" => 10_000.0,
        _ => 22_000.0, // high / auto
    };
    let area_ratio = ((width as f64 * height as f64) / (1024.0 * 1024.0)).sqrt();
    let total = base_ms * speed_factor * area_ratio * n as f64;
    Duration::from_millis(total.round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_size_variants() {
        assert_eq!(parse_size("1024x1024"), (1024, 1024));
        assert_eq!(parse_size("1536x1024"), (1536, 1024));
        assert_eq!(parse_size("1024X1536"), (1024, 1536));
        assert_eq!(parse_size("bogus"), (1024, 1024));
        assert_eq!(parse_size("auto"), (1024, 1024));
    }

    #[test]
    fn resolve_defaults() {
        let req = ImageGenerationRequest {
            prompt: "a fox".to_string(),
            model: "gpt-image-1".to_string(),
            n: None,
            size: None,
            quality: None,
            response_format: None,
            output_format: None,
            background: None,
            stream: false,
            partial_images: None,
            moderation: None,
            user: None,
        };
        let r = req.resolve();
        assert_eq!(r.size, "1024x1024");
        assert_eq!(r.quality, "high");
        assert_eq!(r.output_format, "png");
        assert_eq!(r.n, 1);
        assert_eq!(r.partial_images, 0);
    }

    #[test]
    fn partial_images_clamped() {
        let req = ImageGenerationRequest {
            prompt: "x".to_string(),
            model: "gpt-image-1".to_string(),
            n: Some(50),
            size: Some("auto".to_string()),
            quality: Some("medium".to_string()),
            response_format: None,
            output_format: Some("webp".to_string()),
            background: Some("transparent".to_string()),
            stream: true,
            partial_images: Some(9),
            moderation: None,
            user: None,
        };
        let r = req.resolve();
        assert_eq!(r.n, 10);
        assert_eq!(r.partial_images, 3);
        assert_eq!(r.output_format, "webp");
        assert_eq!(r.background, "transparent");
        assert_eq!(r.size, "1024x1024"); // "auto" resolved to default
    }

    #[test]
    fn tokens_scale_with_quality_and_size() {
        let low = estimate_image_tokens(1024, 1024, "low");
        let high = estimate_image_tokens(1024, 1024, "high");
        assert!(high > low);
        let big = estimate_image_tokens(2048, 2048, "high");
        assert!(big > high);
    }

    #[test]
    fn duration_zero_for_instant() {
        let d = image_total_duration(&LatencyProfile::instant(), "high", 1024, 1024, 1);
        assert_eq!(d, Duration::ZERO);
    }

    #[test]
    fn duration_scales_with_quality() {
        let profile = LatencyProfile::gpt5();
        let low = image_total_duration(&profile, "low", 1024, 1024, 1);
        let high = image_total_duration(&profile, "high", 1024, 1024, 1);
        assert!(high > low);
    }
}
