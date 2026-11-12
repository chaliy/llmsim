// Simulated embeddings (OpenAI `/v1/embeddings`).
//
// Decisions:
// - Vectors are bag-of-words: each distinct word maps to a fixed
//   pseudo-random direction and a text's vector is the normalized sum, plus a
//   small per-text component. Texts that share words land close together, so
//   retrieval and dedup code sees meaningful cosine similarity, and the same
//   text always gives the same vector.
// - Coordinates come from a per-word stream, so a shorter `dimensions`
//   request is a prefix of the full vector, renormalized: the same property
//   OpenAI's v3 models have.
// - Model limits follow OpenAI: 8192 tokens per input, 2048 inputs and 300k
//   tokens per request, `dimensions` only on v3 models.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Tokens allowed in one input.
pub const MAX_INPUT_TOKENS: usize = 8192;
/// Inputs allowed in one request.
pub const MAX_INPUTS: usize = 2048;
/// Tokens allowed across one request.
pub const MAX_REQUEST_TOKENS: usize = 300_000;
/// Weight of the per-text component that separates texts with equal words.
const TEXT_NOISE: f32 = 0.15;

/// Shape of an embedding model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddingModel {
    pub dimensions: usize,
    /// Whether the `dimensions` parameter is accepted.
    pub shortenable: bool,
}

/// Known embedding models. Unknown ids get the `text-embedding-3-small`
/// shape so custom names keep working.
pub fn embedding_model(id: &str) -> Option<EmbeddingModel> {
    match id {
        "text-embedding-3-small" => Some(EmbeddingModel {
            dimensions: 1536,
            shortenable: true,
        }),
        "text-embedding-3-large" => Some(EmbeddingModel {
            dimensions: 3072,
            shortenable: true,
        }),
        "text-embedding-ada-002" => Some(EmbeddingModel {
            dimensions: 1536,
            shortenable: false,
        }),
        _ => None,
    }
}

/// Default shape for models llmsim has no profile for.
pub const DEFAULT_MODEL: EmbeddingModel = EmbeddingModel {
    dimensions: 1536,
    shortenable: true,
};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [-1, 1).
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }
}

fn add_direction(acc: &mut [f32], seed: u64, weight: f32) {
    let mut rng = Rng(seed);
    for x in acc.iter_mut() {
        *x += rng.unit() * weight;
    }
}

/// Deterministic unit vector of `dims` coordinates for `text`.
pub fn embed(text: &str, dims: usize) -> Vec<f32> {
    let mut words: BTreeMap<String, f32> = BTreeMap::new();
    for w in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        *words.entry(w.to_lowercase()).or_default() += 1.0;
    }
    let mut v = vec![0.0f32; dims];
    for (word, count) in &words {
        // Sub-linear weight: repeating a word moves the vector less each time.
        add_direction(
            &mut v,
            crate::schema_fake::seed_from(word.as_bytes()),
            count.sqrt(),
        );
    }
    // Each direction has expected squared norm proportional to its weight
    // squared, so this keeps the per-text share fixed whatever `dims` is.
    let total: f32 = words.values().sum();
    let noise = if total > 0.0 {
        total.sqrt() * TEXT_NOISE
    } else {
        1.0
    };
    add_direction(
        &mut v,
        crate::schema_fake::seed_from(text.as_bytes()) ^ 0x5eed,
        noise,
    );
    normalize(&mut v);
    v
}

fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        for x in v.iter_mut() {
            *x /= n;
        }
    }
}

/// Cosine similarity of two vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// `encoding_format: base64`: little-endian f32 bytes, base64.
pub fn to_base64(v: &[f32]) -> String {
    let bytes: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
    crate::imagegen::base64_encode(&bytes)
}

/// The `input` field: a string, strings, or token arrays.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmbeddingInput {
    Text(String),
    Texts(Vec<String>),
    Tokens(Vec<u32>),
    TokenArrays(Vec<Vec<u32>>),
}

impl EmbeddingInput {
    /// Each input as text (token arrays become `t<id>` words) with its
    /// token count.
    pub fn items(&self) -> Vec<(String, usize)> {
        let text = |s: &str| (s.to_string(), count_tokens(s));
        let tokens = |t: &[u32]| {
            (
                t.iter()
                    .map(|id| format!("t{id}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                t.len(),
            )
        };
        match self {
            EmbeddingInput::Text(s) => vec![text(s)],
            EmbeddingInput::Texts(v) => v.iter().map(|s| text(s)).collect(),
            EmbeddingInput::Tokens(t) => vec![tokens(t)],
            EmbeddingInput::TokenArrays(v) => v.iter().map(|t| tokens(t)).collect(),
        }
    }
}

/// Tokens in `s`: the tokenizer when built in, else about 4 bytes a token.
fn count_tokens(s: &str) -> usize {
    #[cfg(feature = "tokens")]
    {
        crate::count_tokens_default(s).unwrap_or(s.len().div_ceil(4))
    }
    #[cfg(not(feature = "tokens"))]
    {
        s.len().div_ceil(4)
    }
}

/// `POST /v1/embeddings` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    pub model: String,
    pub input: EmbeddingInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_unit_vectors() {
        let a = embed("the quick brown fox", 256);
        assert_eq!(a, embed("the quick brown fox", 256));
        assert_eq!(a.len(), 256);
        let n: f32 = a.iter().map(|x| x * x).sum();
        assert!((n - 1.0).abs() < 1e-4);
    }

    #[test]
    fn shared_words_are_closer() {
        let a = embed("refund policy for damaged items", 512);
        let b = embed("what is the refund policy for items", 512);
        let c = embed("kubernetes pod scheduling latency", 512);
        assert!(cosine(&a, &b) > 0.4, "{}", cosine(&a, &b));
        assert!(cosine(&a, &c).abs() < 0.2, "{}", cosine(&a, &c));
        assert!(cosine(&a, &b) > cosine(&a, &c) + 0.3);
        // Same words in another order are near duplicates, not identical.
        let d = embed("damaged items refund policy for", 512);
        assert!(cosine(&a, &d) > 0.95 && cosine(&a, &d) < 1.0);
    }

    #[test]
    fn shorter_dimensions_are_a_renormalized_prefix() {
        let full = embed("hello world", 1536);
        let short = embed("hello world", 256);
        let mut prefix = full[..256].to_vec();
        normalize(&mut prefix);
        assert!(cosine(&prefix, &short) > 0.999);
    }

    #[test]
    fn empty_text_still_gives_a_unit_vector() {
        let v = embed("", 8);
        let n: f32 = v.iter().map(|x| x * x).sum();
        assert!((n - 1.0).abs() < 1e-4);
    }

    #[test]
    fn input_forms() {
        let parse = |j: &str| serde_json::from_str::<EmbeddingInput>(j).unwrap().items();
        assert_eq!(parse(r#""hi""#).len(), 1);
        assert_eq!(parse(r#"["a","b"]"#).len(), 2);
        assert_eq!(parse("[1,2,3]"), vec![("t1 t2 t3".to_string(), 3)]);
        assert_eq!(parse("[[1],[2,3]]").len(), 2);
    }

    #[test]
    fn base64_round_trips() {
        let v = vec![1.0f32, -0.5];
        assert_eq!(to_base64(&v), "AACAPwAAAL8=");
    }
}
