//! Use cases and the ports they need. Depends only on `crt-domain`.
//! Adapters implement the ports; composition roots wire them in.

pub mod analyze_file;
pub mod ports;

pub use analyze_file::{AnalyzeError, FileAnalysis, analyze_file};
pub use ports::{Structure, StructureError, StructureSource};
