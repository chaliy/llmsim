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
pub mod anthropic;
pub mod errors;
pub mod generator;
mod ids;
pub mod latency;
pub mod openai;
pub mod openresponses;
pub mod responses_stream;
pub mod script;
pub mod script_stream;
pub mod stats;
pub mod stream;

// Token counting via tiktoken-rs (enabled by the `tokens` feature)
#[cfg(feature = "tokens")]
pub mod tokens;

// CLI module: HTTP server, router, and handlers (enabled by the `server` feature)
#[cfg(feature = "server")]
pub mod cli;

// TUI module (for `llmsim serve --tui`)
#[cfg(feature = "tui")]
pub mod tui;

// Re-export commonly used types
pub use errors::{ErrorConfig, ErrorInjector, SimulatedError};
pub use generator::{
    create_generator, EchoGenerator, FixedGenerator, LoremGenerator, RandomWordGenerator,
    ResponseGenerator, SequenceGenerator,
};
pub use latency::LatencyProfile;
pub use responses_stream::{ResponsesTokenStream, ResponsesTokenStreamBuilder};
pub use script::{
    OnExhausted, Script, ScriptError, ScriptSpec, ScriptedResponse, SimError, SimToolCall, SimTurn,
};
pub use stats::{new_shared_stats, EndpointType, SharedStats, Stats, StatsSnapshot};
pub use stream::{TokenStream, TokenStreamBuilder};
#[cfg(feature = "tokens")]
pub use tokens::{count_tokens, count_tokens_default, TokenCounter, TokenError};
