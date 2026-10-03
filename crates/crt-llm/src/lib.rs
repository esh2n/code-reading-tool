//! `Explainer` backed by any OpenAI-compatible chat completions endpoint.
//!
//! - Structured output is required: the request carries a strict JSON
//!   Schema, and an endpoint that refuses it is an error, never a silent
//!   switch to free-form text.
//! - A malformed answer is asked again of the same endpoint (twice at most).
//! - An unreachable endpoint or a server error moves to the next fallback,
//!   and the reading says which model actually wrote it.

mod client;
mod config;
mod prompt;

pub use client::OpenAiCompatible;
pub use config::{EndpointConfig, LlmConfig};
