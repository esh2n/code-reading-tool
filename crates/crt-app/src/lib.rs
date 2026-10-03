//! Use cases and the ports they need. Depends only on `crt-domain`.
//! Adapters implement the ports; composition roots wire them in.

pub mod analyze_file;
pub mod ports;

pub use analyze_file::{analyze_file, AnalyzeError, FileAnalysis};
pub use ports::{StructureError, StructureSource};
