// Streaming engine for the simulated image-generation endpoint.
//
// Emits OpenAI-compatible `image_generation.partial_image` events followed by a
// final `image_generation.completed` event, with progressively sharper preview
// frames and realistic inter-frame timing.

use crate::ids::unix_timestamp;
use crate::imagegen::{base64_encode, render_png, PlaceholderSpec};
use crate::latency::LatencyProfile;
use crate::openai::images::{
    completed_image_sse, image_total_duration, partial_image_sse, CompletedImageEvent, ImagesUsage,
    PartialImageEvent, ResolvedImageParams,
};
use async_stream::stream;
use futures_core::Stream;
use std::pin::Pin;
use std::time::Duration;
use tokio::time::sleep;

type OnCompleteCallback = Box<dyn FnOnce() + Send>;

/// Ensures the completion callback runs exactly once, even on early drop.
struct CompletionGuard {
    callback: Option<OnCompleteCallback>,
}

impl CompletionGuard {
    fn new(callback: Option<OnCompleteCallback>) -> Self {
        Self { callback }
    }
    fn complete(&mut self) {
        if let Some(cb) = self.callback.take() {
            cb();
        }
    }
}

impl Drop for CompletionGuard {
    fn drop(&mut self) {
        self.complete();
    }
}

/// A streaming simulated image generation.
pub struct ImageStream {
    model: String,
    prompt: String,
    params: ResolvedImageParams,
    latency: LatencyProfile,
    usage: ImagesUsage,
    on_complete: Option<OnCompleteCallback>,
}

impl ImageStream {
    pub fn new(
        model: impl Into<String>,
        prompt: impl Into<String>,
        params: ResolvedImageParams,
        latency: LatencyProfile,
        usage: ImagesUsage,
    ) -> Self {
        Self {
            model: model.into(),
            prompt: prompt.into(),
            params,
            latency,
            usage,
            on_complete: None,
        }
    }

    pub fn with_on_complete<F>(mut self, callback: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        self.on_complete = Some(Box::new(callback));
        self
    }

    pub fn into_stream(self) -> Pin<Box<dyn Stream<Item = String> + Send>> {
        let ImageStream {
            model,
            prompt,
            params,
            latency,
            usage,
            on_complete,
        } = self;

        Box::pin(stream! {
            let mut guard = CompletionGuard::new(on_complete);

            let total = image_total_duration(
                &latency,
                &params.quality,
                params.width,
                params.height,
                params.n,
            );

            // p partials + 1 completed frame, evenly spaced across `total`.
            let partials = params.partial_images;
            let frames = partials + 1;
            let per_frame = if frames > 0 {
                total / frames
            } else {
                total
            };

            // Partial frames get progressively finer (smaller block size).
            for i in 0..partials {
                if !per_frame.is_zero() {
                    sleep(per_frame).await;
                }
                let blockiness = 1u32 << (partials - i); // e.g. 8,4,2 for p=3
                let png = render_png(&PlaceholderSpec {
                    width: params.width,
                    height: params.height,
                    prompt: &prompt,
                    model: &model,
                    quality: &params.quality,
                    blockiness,
                });
                let event = PartialImageEvent {
                    event_type: "image_generation.partial_image".to_string(),
                    b64_json: base64_encode(&png),
                    created_at: unix_timestamp(),
                    size: params.size.clone(),
                    quality: params.quality.clone(),
                    background: params.background.clone(),
                    output_format: params.output_format.clone(),
                    partial_image_index: i,
                };
                yield partial_image_sse(&event);
            }

            // Final, crisp frame.
            if !per_frame.is_zero() {
                sleep(per_frame).await;
            }
            let png = render_png(&PlaceholderSpec {
                width: params.width,
                height: params.height,
                prompt: &prompt,
                model: &model,
                quality: &params.quality,
                blockiness: 1,
            });
            let completed = CompletedImageEvent {
                event_type: "image_generation.completed".to_string(),
                b64_json: base64_encode(&png),
                created_at: unix_timestamp(),
                size: params.size.clone(),
                quality: params.quality.clone(),
                background: params.background.clone(),
                output_format: params.output_format.clone(),
                usage,
            };
            yield completed_image_sse(&completed);

            guard.complete();
        })
    }
}

/// Convenience: a near-instant TTFT delay used by the non-streaming handler so
/// it can `sleep` a single bounded duration without monopolising the runtime.
pub fn capped(total: Duration) -> Duration {
    // Guard against an absurd configured profile producing minutes-long waits.
    total.min(Duration::from_secs(120))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::images::ImageInputTokensDetails;
    use futures_util::StreamExt;

    fn usage() -> ImagesUsage {
        ImagesUsage {
            total_tokens: 100,
            input_tokens: 10,
            output_tokens: 90,
            input_tokens_details: ImageInputTokensDetails {
                text_tokens: 10,
                image_tokens: 0,
            },
        }
    }

    fn params(partials: u32) -> ResolvedImageParams {
        ResolvedImageParams {
            width: 256,
            height: 256,
            size: "256x256".to_string(),
            quality: "low".to_string(),
            output_format: "png".to_string(),
            background: "opaque".to_string(),
            n: 1,
            partial_images: partials,
        }
    }

    #[tokio::test]
    async fn streams_partials_then_completed() {
        let stream = ImageStream::new(
            "gpt-image-1",
            "a red apple",
            params(2),
            LatencyProfile::instant(),
            usage(),
        );
        let events: Vec<String> = stream.into_stream().collect().await;

        let partial = events
            .iter()
            .filter(|e| e.contains("image_generation.partial_image"))
            .count();
        assert_eq!(partial, 2);
        assert!(events
            .last()
            .unwrap()
            .contains("image_generation.completed"));
        // Completed event carries usage.
        assert!(events.last().unwrap().contains("\"usage\""));
    }

    #[tokio::test]
    async fn zero_partials_emits_only_completed() {
        let stream = ImageStream::new(
            "gpt-image-1",
            "a blue car",
            params(0),
            LatencyProfile::instant(),
            usage(),
        );
        let events: Vec<String> = stream.into_stream().collect().await;
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("image_generation.completed"));
    }

    #[tokio::test]
    async fn partial_indices_increase() {
        let stream = ImageStream::new(
            "gpt-image-1",
            "a tree",
            params(3),
            LatencyProfile::instant(),
            usage(),
        );
        let events: Vec<String> = stream.into_stream().collect().await;
        assert!(events[0].contains("\"partial_image_index\":0"));
        assert!(events[1].contains("\"partial_image_index\":1"));
        assert!(events[2].contains("\"partial_image_index\":2"));
    }
}
