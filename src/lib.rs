pub mod acp;
pub mod api;

// Re-export public API at crate root for convenience
pub use api::{analyze, prompt, AnalyzeOptions, Backend, UsageMode, OutputFormat};
