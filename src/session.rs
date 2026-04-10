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
        let message = build_message(prompt, files, &self.tag_open, &self.tag_close);

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

/// Build the message sent to the AI CLI, with @file refs and tag instruction.
pub fn build_message(prompt: &str, files: Option<&[&str]>, tag_open: &str, tag_close: &str) -> String {
    let mut message = String::new();
    if let Some(files) = files {
        for f in files {
            message.push_str(&format!("@{} ", f));
        }
    }
    message.push_str(prompt);
    message.push_str(&format!(" 結果を{}と{}で囲んで返せ", tag_open, tag_close));
    message.push('\r');
    message
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

    // === extract_tagged: exact input → exact output ===

    #[test]
    fn extract_simple() {
        assert_eq!(
            extract_tagged("noise <RESULT>hello</RESULT> noise", "<RESULT>", "</RESULT>"),
            Some("hello".into())
        );
    }

    #[test]
    fn extract_multiline() {
        assert_eq!(
            extract_tagged("<RESULT>\nline1\nline2\n</RESULT>", "<RESULT>", "</RESULT>"),
            Some("line1\nline2".into())
        );
    }

    #[test]
    fn extract_no_tags() {
        assert_eq!(extract_tagged("no tags here", "<RESULT>", "</RESULT>"), None);
    }

    #[test]
    fn extract_incomplete_open_only() {
        assert_eq!(extract_tagged("<RESULT>partial", "<RESULT>", "</RESULT>"), None);
    }

    #[test]
    fn extract_last_occurrence_wins() {
        assert_eq!(
            extract_tagged("<RESULT>old</RESULT> junk <RESULT>new</RESULT>", "<RESULT>", "</RESULT>"),
            Some("new".into())
        );
    }

    #[test]
    fn extract_empty_content_returns_none() {
        assert_eq!(extract_tagged("<RESULT>  </RESULT>", "<RESULT>", "</RESULT>"), None);
    }

    #[test]
    fn extract_empty_string() {
        assert_eq!(extract_tagged("", "<RESULT>", "</RESULT>"), None);
    }

    #[test]
    fn extract_custom_tags() {
        assert_eq!(
            extract_tagged("xxx<OUT>data</OUT>yyy", "<OUT>", "</OUT>"),
            Some("data".into())
        );
    }

    #[test]
    fn extract_with_ansi_escapes() {
        assert_eq!(
            extract_tagged("\x1b[32m<RESULT>green</RESULT>\x1b[0m", "<RESULT>", "</RESULT>"),
            Some("green".into())
        );
    }

    #[test]
    fn extract_with_cursor_and_clear() {
        assert_eq!(
            extract_tagged("\x1b[H\x1b[2J<RESULT>cleared</RESULT>", "<RESULT>", "</RESULT>"),
            Some("cleared".into())
        );
    }

    #[test]
    fn extract_with_tui_decorations() {
        assert_eq!(
            extract_tagged("▀▀▀\n✦ \x1b[1m<RESULT>\n4\n</RESULT>\x1b[0m ────", "<RESULT>", "</RESULT>"),
            Some("4".into())
        );
    }

    // === build_message: exact input → exact output ===

    #[test]
    fn build_no_files() {
        assert_eq!(
            build_message("hello", None, "<R>", "</R>"),
            "hello 結果を<R>と</R>で囲んで返せ\r"
        );
    }

    #[test]
    fn build_with_two_files() {
        assert_eq!(
            build_message("analyze", Some(&["a.jpg", "b.pdf"]), "<R>", "</R>"),
            "@a.jpg @b.pdf analyze 結果を<R>と</R>で囲んで返せ\r"
        );
    }

    #[test]
    fn build_single_file() {
        assert_eq!(
            build_message("p", Some(&["photo.jpg"]), "<R>", "</R>"),
            "@photo.jpg p 結果を<R>と</R>で囲んで返せ\r"
        );
    }

    #[test]
    fn build_five_files_order_preserved() {
        assert_eq!(
            build_message("go", Some(&["1.jpg", "2.jpg", "3.jpg", "4.jpg", "5.jpg"]), "<T>", "</T>"),
            "@1.jpg @2.jpg @3.jpg @4.jpg @5.jpg go 結果を<T>と</T>で囲んで返せ\r"
        );
    }

    #[test]
    fn build_file_with_spaces() {
        assert_eq!(
            build_message("p", Some(&["my file.jpg"]), "<R>", "</R>"),
            "@my file.jpg p 結果を<R>と</R>で囲んで返せ\r"
        );
    }

    #[test]
    fn build_empty_prompt() {
        assert_eq!(
            build_message("", None, "<R>", "</R>"),
            " 結果を<R>と</R>で囲んで返せ\r"
        );
    }

    #[test]
    fn build_unicode_prompt() {
        assert_eq!(
            build_message("日本語テスト", None, "<R>", "</R>"),
            "日本語テスト 結果を<R>と</R>で囲んで返せ\r"
        );
    }

    #[test]
    fn build_custom_tags() {
        assert_eq!(
            build_message("test", None, "<OUT>", "</OUT>"),
            "test 結果を<OUT>と</OUT>で囲んで返せ\r"
        );
    }

    #[test]
    fn build_prompt_with_model_flag() {
        assert_eq!(
            build_message("--model flash analyze this", None, "<R>", "</R>"),
            "--model flash analyze this 結果を<R>と</R>で囲んで返せ\r"
        );
    }

    // === baseline tracking: sequential query simulation ===

    #[test]
    fn baseline_extracts_second_result() {
        let buf = "noise <RESULT>first</RESULT> gap <RESULT>second</RESULT> end";
        // After first query, baseline is set past the first result
        let first_end = buf.find("gap").unwrap();
        let new_content = &buf[first_end..];
        assert_eq!(extract_tagged(new_content, "<RESULT>", "</RESULT>"), Some("second".into()));
    }

    #[test]
    fn baseline_at_end_returns_none() {
        let buf = "<RESULT>data</RESULT>";
        let new_content = &buf[buf.len()..];
        assert_eq!(extract_tagged(new_content, "<RESULT>", "</RESULT>"), None);
    }

    #[test]
    fn baseline_skips_old_result() {
        let buf = "<RESULT>old</RESULT>---<RESULT>new</RESULT>";
        let baseline = buf.find("---").unwrap();
        assert_eq!(extract_tagged(&buf[baseline..], "<RESULT>", "</RESULT>"), Some("new".into()));
        // Verify "old" is not accessible from new baseline
        assert!(!buf[baseline..].contains("<RESULT>old</RESULT>"));
    }

    // === constants ===

    #[test]
    fn default_timeout_is_180() {
        assert_eq!(DEFAULT_TIMEOUT_SECS, 180);
    }

    #[test]
    fn default_tags() {
        assert_eq!(DEFAULT_TAG_OPEN, "<RESULT>");
        assert_eq!(DEFAULT_TAG_CLOSE, "</RESULT>");
    }

    #[test]
    fn poll_interval_is_500ms() {
        assert_eq!(POLL_INTERVAL_MS, 500);
    }

    // === ConPTY-dependent tests ===

    #[test]
    #[cfg(windows)]
    fn spawn_valid_command_succeeds() {
        assert!(crate::conpty::ConPty::spawn("cmd.exe /c exit 0").is_ok());
    }

    #[test]
    #[cfg(windows)]
    fn spawn_nonexistent_command_fails() {
        assert!(crate::conpty::ConPty::spawn("nonexistent_program_xyz.exe").is_err());
    }

    #[test]
    #[cfg(windows)]
    fn spawn_empty_command_fails() {
        assert!(crate::conpty::ConPty::spawn("").is_err());
    }

    #[test]
    #[cfg(windows)]
    fn short_lived_process_is_dead_after_wait() {
        let session = ResidentSession::new("cmd.exe /c exit 0").unwrap();
        // ResidentSession::new waits 3s. cmd.exe /c exit 0 exits immediately.
        assert_eq!(session.is_alive(), false);
    }

    #[test]
    #[cfg(windows)]
    fn interactive_cmd_is_alive() {
        // ping runs for ~100s, outlasting ResidentSession::new's 3s startup wait
        let session = ResidentSession::new("ping -n 100 127.0.0.1").unwrap();
        assert_eq!(session.is_alive(), true);
    }

    #[test]
    #[cfg(windows)]
    fn spawn_invalid_session_fails() {
        assert!(ResidentSession::new("nonexistent_xyz.exe").is_err());
    }
}
