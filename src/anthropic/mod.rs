//! Anthropic Messages API simulation.
//!
//! Implements the `/anthropic/v1/messages` and `/anthropic/v1/models`
//! endpoints, mirroring the Anthropic Messages API wire format so the official
//! Anthropic SDKs work when pointed at `{base_url}/anthropic`.

mod models;
mod stream;
mod types;

pub use models::*;
pub use stream::*;
pub use types::*;
