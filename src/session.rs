//! Tag-based response extraction from AI CLI output.
//!
//! Sends prompts with instructions to wrap responses in XML tags,
//! then extracts the content between those tags from the PTY output buffer.

use std::time::{Duration, Instant};

#[cfg(windows)]
use crate::conpty::ConPty;

const DEFAULT_TAG_OPEN: &str = "<RESULT>";
const DEFAULT_TAG_CLOSE: &str = "</RESULT>";
const DEFAULT_TIMEOUT_SECS: u64 = 180;
const POLL_INTERVAL_MS: u64 = 500;

/// A persistent AI CLI session that extracts responses via tag markers.
pub struct ResidentSession {
    #[cfg(windows)]
    conpty: ConPty,
    tag_open: String,
    tag_close: String,
    timeout_secs: u64,
}

#[cfg(windows)]
impl ResidentSession {
    /// Spawn a new resident session with the given CLI command.
    pub fn new(command: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let conpty = ConPty::spawn(command)?;
        // Wait for CLI startup
        std::thread::sleep(Duration::from_secs(3));
        Ok(Self {
            conpty,
            tag_open: DEFAULT_TAG_OPEN.to_string(),
            tag_close: DEFAULT_TAG_CLOSE.to_string(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        })
    }

    /// Send a prompt and extract the tagged response.
    pub fn query(
        &self,
        prompt: &str,
        files: Option<&[&str]>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let baseline_pos = self.conpty.buffer_len();

        // Build message with @file refs and tag instruction
        let mut message = String::new();
        if let Some(files) = files {
            for f in files {
                message.push_str(&format!("@{} ", f));
            }
        }
        message.push_str(prompt);
        message.push_str(&format!(
            " 結果を{}と{}で囲んで返せ",
            self.tag_open, self.tag_close
        ));
        message.push('\r');

        // Send to PTY
        self.conpty.write(message.as_bytes())?;

        // Poll for tag_close in new buffer content
        let deadline = Instant::now() + Duration::from_secs(self.timeout_secs);

        loop {
            std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));

            if Instant::now() >= deadline {
                return Err(format!(
                    "timeout waiting for {} after {}s",
                    self.tag_close, self.timeout_secs
                )
                .into());
            }

            let buffer = self.conpty.read_buffer();
            if buffer.len() <= baseline_pos {
                continue;
            }

            let new_content = &buffer[baseline_pos..];
            if let Some(result) = extract_tagged(new_content, &self.tag_open, &self.tag_close) {
                return Ok(result);
            }
        }
    }

    /// Check if the underlying process is still alive.
    pub fn is_alive(&self) -> bool {
        self.conpty.is_alive()
    }
}

/// Extract content between the last occurrence of tag_open and tag_close.
pub fn extract_tagged(text: &str, tag_open: &str, tag_close: &str) -> Option<String> {
    let close_pos = text.rfind(tag_close)?;
    let search_region = &text[..close_pos];
    let open_pos = search_region.rfind(tag_open)?;
    let start = open_pos + tag_open.len();
    let content = &text[start..close_pos];
    let trimmed = content.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_extract() {
        assert_eq!(
            extract_tagged("noise <RESULT>hello</RESULT> noise", "<RESULT>", "</RESULT>"),
            Some("hello".into())
        );
    }

    #[test]
    fn multiline() {
        assert_eq!(
            extract_tagged(
                "<RESULT>\nline1\nline2\n</RESULT>",
                "<RESULT>",
                "</RESULT>"
            ),
            Some("line1\nline2".into())
        );
    }

    #[test]
    fn no_tags() {
        assert_eq!(
            extract_tagged("no tags here", "<RESULT>", "</RESULT>"),
            None
        );
    }

    #[test]
    fn incomplete() {
        assert_eq!(
            extract_tagged("<RESULT>partial", "<RESULT>", "</RESULT>"),
            None
        );
    }

    #[test]
    fn last_wins() {
        assert_eq!(
            extract_tagged(
                "<RESULT>old</RESULT> junk <RESULT>new</RESULT>",
                "<RESULT>",
                "</RESULT>"
            ),
            Some("new".into())
        );
    }

    #[test]
    fn with_tui_noise() {
        let buf = "▀▀▀▀▀\n > prompt\n▄▄▄▄▄\n✦ <RESULT>\n4\n</RESULT>\n────\n? for shortcuts";
        assert_eq!(
            extract_tagged(buf, "<RESULT>", "</RESULT>"),
            Some("4".into())
        );
    }

    // NOTE: ConPTY live tests (Node.js TTY detection, gemini session) require
    // a GUI host process (no inherited console). They cannot pass from
    // mintty/Git Bash or cargo test. See conpty.rs module docs.
}
