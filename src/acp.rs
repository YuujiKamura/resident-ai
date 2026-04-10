//! ACP (Agent Client Protocol) client — JSON-RPC 2.0 over stdio.
//!
//! Spawns `gemini.cmd --acp` and provides a typed session API.
//!
//! # Protocol flow
//!
//! 1. `initialize` — handshake, get capabilities
//! 2. `session/new` — create session, get sessionId
//! 3. `session/prompt` — send prompt, receive streaming chunks + final result

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};

use serde_json::Value;

/// A JSON-RPC error from the agent.
#[derive(Debug)]
pub struct AcpError(pub String);

impl std::fmt::Display for AcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AcpError {}

// === Pure functions: testable without a process ===

/// Build a JSON-RPC 2.0 request message.
pub fn build_request(id: u64, method: &str, params: Value) -> String {
    let msg = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    });
    serde_json::to_string(&msg).expect("serialize JSON-RPC")
}

/// Build the initialize request params.
pub fn build_initialize_params() -> Value {
    serde_json::json!({
        "protocolVersion": 1,
        "clientInfo": {"name": "resident-ai", "version": "0.1.0"}
    })
}

/// Build the session/new request params.
pub fn build_session_new_params(cwd: &str) -> Value {
    serde_json::json!({
        "cwd": cwd,
        "mcpServers": []
    })
}

/// Build the session/prompt request params.
pub fn build_prompt_params(session_id: &str, text: &str) -> Value {
    serde_json::json!({
        "sessionId": session_id,
        "prompt": [{"type": "text", "text": text}]
    })
}

/// Build prompt text with @file references prepended.
pub fn build_prompt_text(text: &str, files: Option<&[&str]>) -> String {
    match files {
        Some(files) if !files.is_empty() => {
            let refs: String = files.iter().map(|f| format!("@{} ", f)).collect();
            format!("{}{}", refs, text)
        }
        _ => text.to_string(),
    }
}

/// Extract sessionId from a session/new response.
pub fn extract_session_id(response: &Value) -> Option<&str> {
    response.get("result")
        .and_then(|r| r.get("sessionId"))
        .and_then(|v| v.as_str())
}

/// Extract agent_message_chunk text from a session/update notification.
/// Returns None if the line is not an agent_message_chunk.
pub fn extract_chunk_text(line: &Value) -> Option<&str> {
    let update = line.get("params")?.get("update")?;
    if update.get("sessionUpdate")?.as_str()? != "agent_message_chunk" {
        return None;
    }
    update.get("content")?.get("text")?.as_str()
}

/// Check if a JSON-RPC line is a response with the given id.
pub fn is_response(line: &Value, expected_id: u64) -> bool {
    line.get("id").and_then(|v| v.as_u64()) == Some(expected_id)
}

/// Check if a JSON-RPC response contains an error.
pub fn response_error(line: &Value) -> Option<&Value> {
    line.get("error")
}

// === AcpSession: process-dependent ===

/// An active ACP session with a running gemini process.
pub struct AcpSession {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
    session_id: String,
    next_id: u64,
}

impl AcpSession {
    /// Spawn gemini.cmd --acp, perform handshake, return ready session.
    pub fn new(cwd: &Path) -> Result<Self, AcpError> {
        let mut child = Command::new("gemini.cmd")
            .arg("--acp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| AcpError(format!("spawn gemini.cmd --acp: {}", e)))?;

        let stdin = child.stdin.take()
            .ok_or_else(|| AcpError("no stdin".into()))?;
        let stdout = child.stdout.take()
            .ok_or_else(|| AcpError("no stdout".into()))?;

        let mut s = AcpSession {
            child,
            stdin,
            reader: BufReader::new(stdout),
            session_id: String::new(),
            next_id: 1,
        };

        // initialize
        let id = s.send("initialize", build_initialize_params())?;
        let resp = s.read_until_id(id)?;
        if response_error(&resp).is_some() {
            return Err(AcpError(format!("initialize failed: {}", resp)));
        }

        // session/new
        let cwd_str = cwd.to_str()
            .ok_or_else(|| AcpError("cwd not utf-8".into()))?;
        let id = s.send("session/new", build_session_new_params(cwd_str))?;
        let resp = s.read_until_id(id)?;
        s.session_id = extract_session_id(&resp)
            .ok_or_else(|| AcpError(format!("no sessionId: {}", resp)))?
            .to_string();

        Ok(s)
    }

    /// Send a text prompt. Returns the full response text.
    pub fn prompt(&mut self, text: &str) -> Result<String, AcpError> {
        let params = build_prompt_params(&self.session_id, text);
        let id = self.send("session/prompt", params)?;
        self.collect_response(id)
    }

    /// Send a text prompt with @file references prepended.
    pub fn prompt_with_files(&mut self, text: &str, files: &[&str]) -> Result<String, AcpError> {
        self.prompt(&build_prompt_text(text, Some(files)))
    }

    fn send(&mut self, method: &str, params: Value) -> Result<u64, AcpError> {
        let id = self.next_id;
        self.next_id += 1;
        let line = build_request(id, method, params);
        self.stdin.write_all(line.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| AcpError(format!("write stdin: {}", e)))?;
        Ok(id)
    }

    fn read_until_id(&mut self, expected_id: u64) -> Result<Value, AcpError> {
        loop {
            let line = self.read_line()?;
            if is_response(&line, expected_id) {
                return Ok(line);
            }
        }
    }

    fn collect_response(&mut self, expected_id: u64) -> Result<String, AcpError> {
        let mut chunks = Vec::new();
        loop {
            let line = self.read_line()?;
            if is_response(&line, expected_id) {
                if let Some(err) = response_error(&line) {
                    return Err(AcpError(format!("prompt error: {}", err)));
                }
                return Ok(chunks.join("").trim().to_string());
            }
            if let Some(text) = extract_chunk_text(&line) {
                chunks.push(text.to_string());
            }
        }
    }

    fn read_line(&mut self) -> Result<Value, AcpError> {
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = self.reader.read_line(&mut buf)
                .map_err(|e| AcpError(format!("read stdout: {}", e)))?;
            if n == 0 {
                return Err(AcpError("process closed stdout".into()));
            }
            let trimmed = buf.trim();
            if trimmed.is_empty() {
                continue;
            }
            return serde_json::from_str(trimmed)
                .map_err(|e| AcpError(format!("parse JSON: {} in {:?}", e, &trimmed[..trimmed.len().min(100)])));
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

impl Drop for AcpSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // === build_request ===

    #[test]
    fn build_request_initialize() {
        let msg = build_request(1, "initialize", build_initialize_params());
        let parsed: Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 1);
        assert_eq!(parsed["method"], "initialize");
        assert_eq!(parsed["params"]["protocolVersion"], 1);
        assert_eq!(parsed["params"]["clientInfo"]["name"], "resident-ai");
    }

    #[test]
    fn build_request_session_new() {
        let msg = build_request(2, "session/new", build_session_new_params("/tmp/work"));
        let parsed: Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["method"], "session/new");
        assert_eq!(parsed["params"]["cwd"], "/tmp/work");
        assert_eq!(parsed["params"]["mcpServers"], json!([]));
    }

    #[test]
    fn build_request_prompt() {
        let msg = build_request(3, "session/prompt", build_prompt_params("uuid-123", "hello"));
        let parsed: Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["method"], "session/prompt");
        assert_eq!(parsed["params"]["sessionId"], "uuid-123");
        assert_eq!(parsed["params"]["prompt"][0]["type"], "text");
        assert_eq!(parsed["params"]["prompt"][0]["text"], "hello");
    }

    // === build_prompt_text ===

    #[test]
    fn prompt_text_no_files() {
        assert_eq!(build_prompt_text("hello", None), "hello");
    }

    #[test]
    fn prompt_text_empty_files() {
        assert_eq!(build_prompt_text("hello", Some(&[])), "hello");
    }

    #[test]
    fn prompt_text_with_files() {
        assert_eq!(
            build_prompt_text("analyze", Some(&["a.jpg", "b.pdf"])),
            "@a.jpg @b.pdf analyze"
        );
    }

    // === extract_session_id ===

    #[test]
    fn extract_session_id_valid() {
        let resp = json!({"result": {"sessionId": "abc-123"}});
        assert_eq!(extract_session_id(&resp), Some("abc-123"));
    }

    #[test]
    fn extract_session_id_missing() {
        let resp = json!({"result": {}});
        assert_eq!(extract_session_id(&resp), None);
    }

    #[test]
    fn extract_session_id_error_response() {
        let resp = json!({"error": {"code": -32603, "message": "fail"}});
        assert_eq!(extract_session_id(&resp), None);
    }

    // === extract_chunk_text ===

    #[test]
    fn extract_chunk_text_valid() {
        let line = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "uuid",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "4"}
                }
            }
        });
        assert_eq!(extract_chunk_text(&line), Some("4"));
    }

    #[test]
    fn extract_chunk_text_not_a_chunk() {
        let line = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "available_commands_update",
                    "availableCommands": []
                }
            }
        });
        assert_eq!(extract_chunk_text(&line), None);
    }

    #[test]
    fn extract_chunk_text_not_a_notification() {
        let line = json!({"jsonrpc": "2.0", "id": 3, "result": {"stopReason": "end_turn"}});
        assert_eq!(extract_chunk_text(&line), None);
    }

    #[test]
    fn extract_chunk_thought_is_not_message() {
        let line = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "agent_thought_chunk",
                    "content": {"type": "text", "text": "thinking..."}
                }
            }
        });
        assert_eq!(extract_chunk_text(&line), None);
    }

    // === is_response / response_error ===

    #[test]
    fn is_response_matching() {
        let line = json!({"jsonrpc": "2.0", "id": 5, "result": {}});
        assert_eq!(is_response(&line, 5), true);
    }

    #[test]
    fn is_response_wrong_id() {
        let line = json!({"jsonrpc": "2.0", "id": 5, "result": {}});
        assert_eq!(is_response(&line, 3), false);
    }

    #[test]
    fn is_response_notification() {
        let line = json!({"jsonrpc": "2.0", "method": "session/update", "params": {}});
        assert_eq!(is_response(&line, 1), false);
    }

    #[test]
    fn response_error_present() {
        let line = json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32601, "message": "not found"}});
        assert!(response_error(&line).is_some());
    }

    #[test]
    fn response_error_absent() {
        let line = json!({"jsonrpc": "2.0", "id": 1, "result": {}});
        assert!(response_error(&line).is_none());
    }

    // === Live tests (require gemini CLI) ===

    #[test]
    #[ignore]
    fn handshake_and_prompt() {
        let cwd = std::env::current_dir().unwrap();
        let mut session = AcpSession::new(&cwd).expect("handshake failed");
        assert!(!session.session_id().is_empty());

        let result = session.prompt("2+2は？数字だけ答えろ").expect("prompt failed");
        assert!(result.contains('4'), "expected 4, got: {}", result);
    }

    #[test]
    #[ignore]
    fn sequential_prompts() {
        let cwd = std::env::current_dir().unwrap();
        let mut session = AcpSession::new(&cwd).expect("handshake failed");

        let r1 = session.prompt("2+3は？数字だけ").expect("prompt 1 failed");
        assert!(r1.contains('5'), "expected 5, got: {}", r1);

        let r2 = session.prompt("7*8は？数字だけ").expect("prompt 2 failed");
        assert!(r2.contains("56"), "expected 56, got: {}", r2);
    }
}
