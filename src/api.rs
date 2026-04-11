//! cli-ai-analyzer compatible public API.
//! Internal implementation uses ACP (JSON-RPC over stdio).

use std::path::Path;
use crate::acp::{AcpSession, AcpError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backend {
    #[default]
    Gemini,
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsageMode {
    PayPerUse,
    #[default]
    TimeBasedQuota,
    Resident,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Debug, Clone)]
pub struct AnalyzeOptions {
    pub model: String,
    pub output_format: OutputFormat,
    pub backend: Backend,
    pub usage_mode: UsageMode,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self {
            model: String::new(),
            output_format: OutputFormat::Text,
            backend: Backend::default(),
            usage_mode: UsageMode::default(),
        }
    }
}

impl AnalyzeOptions {
    pub fn json(mut self) -> Self {
        self.output_format = OutputFormat::Json;
        self
    }

    pub fn with_backend(mut self, backend: Backend) -> Self {
        self.backend = backend;
        self
    }

    pub fn with_usage_mode(mut self, usage_mode: UsageMode) -> Self {
        self.usage_mode = usage_mode;
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }
}

/// Analyze files with a prompt. Compatible with cli_ai_analyzer::analyze().
pub fn analyze<P: AsRef<Path>>(
    prompt: &str,
    files: &[P],
    options: AnalyzeOptions,
) -> Result<String, AcpError> {
    let cwd = std::env::current_dir()
        .map_err(|e| AcpError(format!("current_dir: {}", e)))?;
    let mut session = AcpSession::new(&cwd)?;

    // Build prompt with JSON instruction if needed
    let full_prompt = if options.output_format == OutputFormat::Json {
        format!("{} Respond with ONLY the JSON object.", prompt)
    } else {
        prompt.to_string()
    };

    if files.is_empty() {
        session.prompt(&full_prompt)
    } else {
        // For image files, use prompt_with_image for the first file
        // For non-image files, use @file references
        let first = files[0].as_ref();
        if is_image(first) {
            session.prompt_with_image(&full_prompt, first)
        } else {
            let file_strs: Vec<&str> = files.iter()
                .filter_map(|f| f.as_ref().to_str())
                .collect();
            session.prompt_with_files(&full_prompt, &file_strs)
        }
    }
}

/// Prompt without files. Compatible with cli_ai_analyzer::prompt().
pub fn prompt(prompt: &str, options: AnalyzeOptions) -> Result<String, AcpError> {
    let empty: Vec<std::path::PathBuf> = vec![];
    analyze(prompt, &empty, options)
}

fn is_image(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options() {
        let opts = AnalyzeOptions::default();
        assert_eq!(opts.backend, Backend::Gemini);
        assert_eq!(opts.usage_mode, UsageMode::TimeBasedQuota);
        assert_eq!(opts.output_format, OutputFormat::Text);
    }

    #[test]
    fn options_builder_chain() {
        let opts = AnalyzeOptions::default()
            .json()
            .with_backend(Backend::Claude)
            .with_usage_mode(UsageMode::Resident);
        assert_eq!(opts.output_format, OutputFormat::Json);
        assert_eq!(opts.backend, Backend::Claude);
        assert_eq!(opts.usage_mode, UsageMode::Resident);
    }

    #[test]
    fn is_image_detection() {
        assert_eq!(is_image(Path::new("photo.jpg")), true);
        assert_eq!(is_image(Path::new("photo.png")), true);
        assert_eq!(is_image(Path::new("doc.pdf")), false);
        assert_eq!(is_image(Path::new("data.json")), false);
    }

    #[test]
    #[ignore] // requires gemini CLI
    fn analyze_text_prompt() {
        let result = prompt("2+2は？数字だけ", AnalyzeOptions::default());
        assert!(result.is_ok(), "failed: {:?}", result.err());
        assert!(result.unwrap().contains('4'));
    }
}
