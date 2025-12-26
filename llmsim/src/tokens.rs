// Token Counter Module
// Uses tiktoken-rs for accurate token counting compatible with OpenAI's tokenizer.

use tiktoken_rs::{cl100k_base, o200k_base, p50k_base, r50k_base, CoreBPE};

/// Error type for token counting operations
#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("Failed to initialize tokenizer: {0}")]
    InitError(String),
}

/// Get the appropriate tokenizer for a model
fn get_tokenizer_for_model(model: &str) -> Result<CoreBPE, TokenError> {
    // Model to encoding mapping based on OpenAI's documentation
    let model_lower = model.to_lowercase();

    // o200k_base: GPT-5, GPT-4o, O-series and newer models
    if model_lower.contains("gpt-5")
        || model_lower.contains("gpt-4o")
        || model_lower.starts_with("o1")
        || model_lower.starts_with("o3")
        || model_lower.contains("chatgpt-4o")
    {
        return o200k_base().map_err(|e| TokenError::InitError(e.to_string()));
    }

    // cl100k_base: GPT-4, text-embedding, Claude, Gemini
    if model_lower.contains("gpt-4")
        || model_lower.contains("text-embedding")
        || model_lower.contains("claude")
        || model_lower.contains("gemini")
    {
        return cl100k_base().map_err(|e| TokenError::InitError(e.to_string()));
    }

    // p50k_base: text-davinci-002, text-davinci-003, code-* models
    if model_lower.contains("davinci") || model_lower.contains("code-") {
        return p50k_base().map_err(|e| TokenError::InitError(e.to_string()));
    }

    // r50k_base: GPT-3 models (ada, babbage, curie, davinci without version)
    if model_lower.contains("ada")
        || model_lower.contains("babbage")
        || model_lower.contains("curie")
    {
        return r50k_base().map_err(|e| TokenError::InitError(e.to_string()));
    }

    // Default to cl100k_base as it's the most common for modern models
    cl100k_base().map_err(|e| TokenError::InitError(e.to_string()))
}

/// Count tokens in a text string for a specific model
///
/// # Arguments
/// * `text` - The text to tokenize
/// * `model` - The model name (e.g., "gpt-5", "gpt-5-mini", "gpt-4", "claude-3-opus")
///
/// # Returns
/// The number of tokens in the text
pub fn count_tokens(text: &str, model: &str) -> Result<usize, TokenError> {
    let bpe = get_tokenizer_for_model(model)?;
    Ok(bpe.encode_with_special_tokens(text).len())
}

/// Count tokens in a text string using default encoding (cl100k_base)
pub fn count_tokens_default(text: &str) -> Result<usize, TokenError> {
    count_tokens(text, "gpt-4")
}

/// Token counter that caches the tokenizer for repeated use
pub struct TokenCounter {
    bpe: CoreBPE,
    model: String,
}

impl TokenCounter {
    /// Create a new TokenCounter for a specific model
    pub fn new(model: &str) -> Result<Self, TokenError> {
        let bpe = get_tokenizer_for_model(model)?;
        Ok(Self {
            bpe,
            model: model.to_string(),
        })
    }

    /// Count tokens in the given text
    pub fn count(&self, text: &str) -> usize {
        self.bpe.encode_with_special_tokens(text).len()
    }

    /// Tokenize text and return the token IDs
    pub fn encode(&self, text: &str) -> Vec<u32> {
        self.bpe.encode_with_special_tokens(text)
    }

    /// Decode token IDs back to text
    pub fn decode(&self, tokens: &[u32]) -> Result<String, TokenError> {
        self.bpe
            .decode(tokens.to_vec())
            .map_err(|e| TokenError::InitError(e.to_string()))
    }

    /// Get the model this counter was created for
    pub fn model(&self) -> &str {
        &self.model
    }
}

/// Estimate tokens for a chat message (includes overhead for message formatting)
/// OpenAI uses ~4 tokens overhead per message for role and formatting
pub fn estimate_message_tokens(
    content: &str,
    role: &str,
    model: &str,
) -> Result<usize, TokenError> {
    let content_tokens = count_tokens(content, model)?;
    let role_tokens = count_tokens(role, model)?;
    // OpenAI adds approximately 4 tokens per message for formatting
    Ok(content_tokens + role_tokens + 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_tokens_gpt4() {
        // "Hello, world!" should be around 4 tokens
        let count = count_tokens("Hello, world!", "gpt-4").unwrap();
        assert!(count > 0);
        assert!(count < 10);
    }

    #[test]
    fn test_count_tokens_empty() {
        let count = count_tokens("", "gpt-4").unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_count_tokens_long_text() {
        let text = "The quick brown fox jumps over the lazy dog. ".repeat(100);
        let count = count_tokens(&text, "gpt-4").unwrap();
        // Should be roughly 1000 tokens (10 tokens per sentence * 100)
        assert!(count > 500);
        assert!(count < 2000);
    }

    #[test]
    fn test_token_counter_reuse() {
        let counter = TokenCounter::new("gpt-4").unwrap();
        let count1 = counter.count("Hello");
        let count2 = counter.count("World");
        assert!(count1 > 0);
        assert!(count2 > 0);
    }

    #[test]
    fn test_encode_decode() {
        let counter = TokenCounter::new("gpt-4").unwrap();
        let text = "Hello, world!";
        let tokens = counter.encode(text);
        let decoded = counter.decode(&tokens).unwrap();
        assert_eq!(decoded, text);
    }

    #[test]
    fn test_different_models() {
        let text = "Testing different models";
        // GPT-5 and GPT-4o use the same encoding (o200k)
        let gpt5_tokens = count_tokens(text, "gpt-5").unwrap();
        let gpt4o_tokens = count_tokens(text, "gpt-4o").unwrap();
        assert_eq!(gpt5_tokens, gpt4o_tokens);
    }

    #[test]
    fn test_unknown_model_fallback() {
        // Unknown models should fallback to cl100k_base
        let count = count_tokens("Hello", "unknown-model-xyz").unwrap();
        assert!(count > 0);
    }

    #[test]
    fn test_gpt5_models() {
        // All GPT-5 variants should work
        let count_gpt5 = count_tokens("Hello", "gpt-5").unwrap();
        let count_mini = count_tokens("Hello", "gpt-5-mini").unwrap();
        let count_nano = count_tokens("Hello", "gpt-5-nano").unwrap();
        assert!(count_gpt5 > 0);
        assert_eq!(count_gpt5, count_mini);
        assert_eq!(count_gpt5, count_nano);
    }

    #[test]
    fn test_o_series_models() {
        let count_o1 = count_tokens("Hello", "o1-preview").unwrap();
        let count_o3 = count_tokens("Hello", "o3-mini").unwrap();
        assert!(count_o1 > 0);
        assert_eq!(count_o1, count_o3);
    }
}
