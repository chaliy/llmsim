// Response Generator Module
// Provides different strategies for generating simulated LLM responses.

use crate::openai::ChatCompletionRequest;
use rand::seq::SliceRandom;
use rand::Rng;

/// Trait for generating simulated responses
pub trait ResponseGenerator: Send + Sync {
    /// Generate a response for the given request
    fn generate(&self, request: &ChatCompletionRequest) -> String;

    /// Get a name for this generator (for logging/debugging)
    fn name(&self) -> &str;
}

/// Generates lorem ipsum text
pub struct LoremGenerator {
    target_tokens: usize,
}

impl LoremGenerator {
    const LOREM_WORDS: &'static [&'static str] = &[
        "lorem",
        "ipsum",
        "dolor",
        "sit",
        "amet",
        "consectetur",
        "adipiscing",
        "elit",
        "sed",
        "do",
        "eiusmod",
        "tempor",
        "incididunt",
        "ut",
        "labore",
        "et",
        "dolore",
        "magna",
        "aliqua",
        "enim",
        "ad",
        "minim",
        "veniam",
        "quis",
        "nostrud",
        "exercitation",
        "ullamco",
        "laboris",
        "nisi",
        "aliquip",
        "ex",
        "ea",
        "commodo",
        "consequat",
        "duis",
        "aute",
        "irure",
        "in",
        "reprehenderit",
        "voluptate",
        "velit",
        "esse",
        "cillum",
        "fugiat",
        "nulla",
        "pariatur",
        "excepteur",
        "sint",
        "occaecat",
        "cupidatat",
        "non",
        "proident",
        "sunt",
        "culpa",
        "qui",
        "officia",
        "deserunt",
        "mollit",
        "anim",
        "id",
        "est",
        "laborum",
    ];

    pub fn new(target_tokens: usize) -> Self {
        Self { target_tokens }
    }

    fn generate_text(&self, word_count: usize) -> String {
        let mut rng = rand::thread_rng();
        let words: Vec<&str> = (0..word_count)
            .map(|_| *Self::LOREM_WORDS.choose(&mut rng).unwrap())
            .collect();

        let mut result = String::new();
        for (i, word) in words.iter().enumerate() {
            if i == 0 {
                // Capitalize first letter
                let mut chars = word.chars();
                if let Some(first) = chars.next() {
                    result.push(first.to_ascii_uppercase());
                    result.extend(chars);
                }
            } else {
                result.push(' ');
                result.push_str(word);
            }

            // Add punctuation periodically
            if (i + 1) % 10 == 0 && i < words.len() - 1 {
                result.push('.');
            }
        }
        result.push('.');
        result
    }
}

impl Default for LoremGenerator {
    fn default() -> Self {
        Self::new(100)
    }
}

impl ResponseGenerator for LoremGenerator {
    fn generate(&self, _request: &ChatCompletionRequest) -> String {
        // Rough estimate: 1 token ≈ 0.75 words for English text
        let word_count = (self.target_tokens as f64 * 0.75) as usize;
        self.generate_text(word_count.max(1))
    }

    fn name(&self) -> &str {
        "lorem"
    }
}

/// Echoes back the last user message
pub struct EchoGenerator;

impl EchoGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl Default for EchoGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponseGenerator for EchoGenerator {
    fn generate(&self, request: &ChatCompletionRequest) -> String {
        // Find the last user message
        for message in request.messages.iter().rev() {
            if matches!(message.role, crate::openai::Role::User) {
                if let Some(content) = &message.content {
                    return format!("Echo: {}", content);
                }
            }
        }
        "Echo: (no user message found)".to_string()
    }

    fn name(&self) -> &str {
        "echo"
    }
}

/// Returns a fixed configured response
pub struct FixedGenerator {
    response: String,
}

impl FixedGenerator {
    pub fn new(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
        }
    }
}

impl ResponseGenerator for FixedGenerator {
    fn generate(&self, _request: &ChatCompletionRequest) -> String {
        self.response.clone()
    }

    fn name(&self) -> &str {
        "fixed"
    }
}

/// Generates random words to reach target token count
pub struct RandomWordGenerator {
    target_tokens: usize,
}

impl RandomWordGenerator {
    const COMMON_WORDS: &'static [&'static str] = &[
        "the", "be", "to", "of", "and", "a", "in", "that", "have", "I", "it", "for", "not", "on",
        "with", "he", "as", "you", "do", "at", "this", "but", "his", "by", "from", "they", "we",
        "say", "her", "she", "or", "an", "will", "my", "one", "all", "would", "there", "their",
        "what", "so", "up", "out", "if", "about", "who", "get", "which", "go", "me", "when",
        "make", "can", "like", "time", "no", "just", "him", "know", "take", "people", "into",
        "year", "your", "good", "some", "could", "them", "see", "other", "than", "then", "now",
        "look", "only", "come", "its", "over", "think", "also", "back", "after", "use", "two",
        "how", "our", "work", "first", "well", "way", "even", "new", "want", "because", "any",
        "these", "give", "day", "most", "us",
    ];

    pub fn new(target_tokens: usize) -> Self {
        Self { target_tokens }
    }
}

impl Default for RandomWordGenerator {
    fn default() -> Self {
        Self::new(100)
    }
}

impl ResponseGenerator for RandomWordGenerator {
    fn generate(&self, _request: &ChatCompletionRequest) -> String {
        let mut rng = rand::thread_rng();
        // Approximate: 1 token ≈ 0.75 words
        let word_count = (self.target_tokens as f64 * 0.75) as usize;

        let words: Vec<&str> = (0..word_count.max(1))
            .map(|_| *Self::COMMON_WORDS.choose(&mut rng).unwrap())
            .collect();

        let mut result = String::new();
        for (i, word) in words.iter().enumerate() {
            if i == 0 {
                // Capitalize first letter
                let mut chars = word.chars();
                if let Some(first) = chars.next() {
                    result.push(first.to_ascii_uppercase());
                    result.extend(chars);
                }
            } else {
                result.push(' ');
                result.push_str(word);
            }

            // Add punctuation
            if (i + 1) % rng.gen_range(8..15) == 0 && i < words.len() - 1 {
                result.push('.');
                // Next word should be capitalized (handled in next iteration if we check)
            }
        }
        result.push('.');
        result
    }

    fn name(&self) -> &str {
        "random_word"
    }
}

/// Generates numbered sequence responses (useful for testing streaming)
pub struct SequenceGenerator {
    target_tokens: usize,
}

impl SequenceGenerator {
    pub fn new(target_tokens: usize) -> Self {
        Self { target_tokens }
    }
}

impl Default for SequenceGenerator {
    fn default() -> Self {
        Self::new(100)
    }
}

impl ResponseGenerator for SequenceGenerator {
    fn generate(&self, _request: &ChatCompletionRequest) -> String {
        let count = self.target_tokens.max(1);
        (1..=count)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn name(&self) -> &str {
        "sequence"
    }
}

/// Factory for creating generators from config
pub fn create_generator(name: &str, target_tokens: usize) -> Box<dyn ResponseGenerator> {
    match name.to_lowercase().as_str() {
        "lorem" => Box::new(LoremGenerator::new(target_tokens)),
        "echo" => Box::new(EchoGenerator::new()),
        "random" | "random_word" => Box::new(RandomWordGenerator::new(target_tokens)),
        "sequence" => Box::new(SequenceGenerator::new(target_tokens)),
        fixed if fixed.starts_with("fixed:") => Box::new(FixedGenerator::new(&fixed[6..])),
        _ => Box::new(LoremGenerator::new(target_tokens)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::{ChatCompletionRequest, Message};

    fn sample_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "gpt-4".to_string(),
            messages: vec![
                Message::system("You are a helpful assistant."),
                Message::user("Hello, how are you?"),
            ],
            temperature: None,
            top_p: None,
            n: None,
            stream: false,
            stop: None,
            max_tokens: None,
            max_completion_tokens: None,
            presence_penalty: None,
            frequency_penalty: None,
            logit_bias: None,
            user: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            seed: None,
        }
    }

    #[test]
    fn test_lorem_generator() {
        let gen = LoremGenerator::new(50);
        let response = gen.generate(&sample_request());
        assert!(!response.is_empty());
        assert!(response.ends_with('.'));
    }

    #[test]
    fn test_echo_generator() {
        let gen = EchoGenerator::new();
        let response = gen.generate(&sample_request());
        assert!(response.contains("Hello, how are you?"));
    }

    #[test]
    fn test_fixed_generator() {
        let gen = FixedGenerator::new("This is a fixed response.");
        let response = gen.generate(&sample_request());
        assert_eq!(response, "This is a fixed response.");
    }

    #[test]
    fn test_random_word_generator() {
        let gen = RandomWordGenerator::new(50);
        let response = gen.generate(&sample_request());
        assert!(!response.is_empty());
    }

    #[test]
    fn test_sequence_generator() {
        let gen = SequenceGenerator::new(10);
        let response = gen.generate(&sample_request());
        assert!(response.contains("1"));
        assert!(response.contains("10"));
    }

    #[test]
    fn test_create_generator() {
        let lorem = create_generator("lorem", 100);
        assert_eq!(lorem.name(), "lorem");

        let echo = create_generator("echo", 100);
        assert_eq!(echo.name(), "echo");

        let random = create_generator("random", 100);
        assert_eq!(random.name(), "random_word");
    }

    #[test]
    fn test_generator_names() {
        assert_eq!(LoremGenerator::default().name(), "lorem");
        assert_eq!(EchoGenerator.name(), "echo");
        assert_eq!(FixedGenerator::new("test").name(), "fixed");
        assert_eq!(RandomWordGenerator::default().name(), "random_word");
        assert_eq!(SequenceGenerator::default().name(), "sequence");
    }
}
