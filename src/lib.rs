//! # LLMSim - LLM Traffic Simulator
//!
//! A lightweight, high-performance LLM API simulator that replicates
//! the traffic shape of real LLM APIs without running actual models.
//!
//! ## Features
//!
//! - Realistic latency simulation (time-to-first-token, inter-token delays)
//! - Streaming support (Server-Sent Events)
//! - Accurate token counting using tiktoken-rs
//! - Error injection for testing error handling
//! - Multiple response generators (lorem, echo, fixed, random)
//! - Realistic model profiles from [models.dev](https://models.dev) with context windows
//!
//! ## Usage
//!
//! ### As a CLI
//!
//! ```bash
//! # Start the server
//! llmsim serve --port 8080
//! ```
//!
//! ### As a Library
//!
//! ```rust,no_run
//! use llmsim::{
//!     openai::{ChatCompletionRequest, Message},
//!     generator::LoremGenerator,
//!     latency::LatencyProfile,
//! };
//!
//! // Create a generator
//! let generator = LoremGenerator::new(100);
//!
//! // Create a latency profile
//! let latency = LatencyProfile::gpt5();
//!
//! // Count tokens
//! let tokens = llmsim::tokens::count_tokens("Hello, world!", "gpt-5").unwrap();
//! ```

// Core library modules
pub mod errors;
pub mod generator;
pub mod latency;
pub mod openai;
pub mod openresponses;
pub mod responses_stream;
pub mod stats;
pub mod stream;
pub mod tokens;

// CLI module (for `llmsim serve` command)
pub mod cli;

// TUI module (for `llmsim stats` command)
pub mod tui;

// Re-export commonly used types
pub use errors::{ErrorConfig, ErrorInjector, SimulatedError};
pub use generator::{
    create_generator, EchoGenerator, FixedGenerator, LoremGenerator, RandomWordGenerator,
    ResponseGenerator, SequenceGenerator,
};
pub use latency::LatencyProfile;
pub use responses_stream::{
    ReasoningStreamConfig, ResponsesTokenStream, ResponsesTokenStreamBuilder,
};
pub use stats::{new_shared_stats, EndpointType, SharedStats, Stats, StatsSnapshot};
pub use stream::{TokenStream, TokenStreamBuilder};
pub use tokens::{count_tokens, count_tokens_default, TokenCounter, TokenError};
