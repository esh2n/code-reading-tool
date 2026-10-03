//! `Explainer` backed by any OpenAI-compatible chat completions endpoint.
//!
//! - Notes and scenarios are two requests: notes when a file opens,
//!   scenarios only when asked for.
//! - Answers are streamed, so notes reach the editor one by one while the
//!   model is still writing.
//! - Structured output is required: the request carries a strict JSON
//!   Schema, and an endpoint that refuses it is an error, never a silent
//!   switch to free-form text.
//! - A malformed answer is asked again of the same endpoint (twice at most).
//! - An unreachable endpoint or a server error moves to the next fallback,
//!   and the reading says which model actually wrote it.

mod client;
mod config;
mod prompt;
mod stream;

pub use client::OpenAiCompatible;
pub use config::{EndpointConfig, LlmConfig};
