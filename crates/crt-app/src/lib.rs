//! Use cases and the ports they need. Depends only on `crt-domain`.
//! Adapters implement the ports; composition roots wire them in.

pub mod analyze_file;
pub mod ports;
pub mod read_function;

pub use analyze_file::{AnalyzeError, FileAnalysis, analyze_file};
pub use ports::{
    Caller, ExplainError, ExplainRequest, Explained, Explainer, ReadingKey, ReadingStore,
    StoreError, Structure, StructureError, StructureSource,
};
pub use read_function::{
    FunctionReading, FunctionSelector, ReadError, ReadOptions, Readers, cached_readings,
    read_function,
};
