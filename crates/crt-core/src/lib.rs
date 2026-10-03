//! Core of the code-reading tool: finds functions and structural facts with
//! tree-sitter, asks an LLM for per-line explanations, checks them, and
//! caches the result per function.
//!
//! See docs/spec.md for the requirements and docs/decisions/ for the rulings.
