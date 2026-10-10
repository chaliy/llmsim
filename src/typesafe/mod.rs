//! TypeSafe System One API simulation.
//!
//! Implements `POST /typesafe/v1/systemone` and `GET /typesafe/v1/models`,
//! mirroring the TypeSafe API wire format (<https://docs.typesafe.ai/api>) so the
//! official `typesafe-sdk` clients work when pointed at `{base_url}/typesafe`.
//!
//! System One answers typed questions (noul, choice, score) about a `state`
//! with calibrated probabilities instead of generated text, so this module
//! synthesizes answers rather than reusing the text generators.

mod judge;
mod models;
mod types;

pub use judge::*;
pub use models::*;
pub use types::*;
