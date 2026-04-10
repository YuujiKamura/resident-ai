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

/// An active ACP session with a running gemini process.
pub struct AcpSession {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
    session_id: String,
    next_id: u64,
}

/// A JSON-RPC error from the agent.
#[derive(Debug)]
pub struct AcpError(pub String);

impl std::fmt::Display for AcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AcpError {}

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
        let id = s.send("initialize", serde_json::json!({
            "protocolVersion": 1,
            "clientInfo": {"name": "resident-ai", "version": "0.1.0"}
        }))?;
        let resp = s.read_until_id(id)?;
        if resp.get("error").is_some() {
            return Err(AcpError(format!("initialize failed: {}", resp)));
        }

        // session/new
        let cwd_str = cwd.to_str()
            .ok_or_else(|| AcpError("cwd not utf-8".into()))?;
        let id = s.send("session/new", serde_json::json!({
            "cwd": cwd_str,
            "mcpServers": []
        }))?;
        let resp = s.read_until_id(id)?;
        s.session_id = resp
            .get("result")
            .and_then(|r| r.get("sessionId"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| AcpError(format!("no sessionId: {}", resp)))?
            .to_string();

        Ok(s)
    }

    /// Send a text prompt. Returns the full response text.
    pub fn prompt(&mut self, text: &str) -> Result<String, AcpError> {
        let sid = self.session_id.clone();
        let id = self.send("session/prompt", serde_json::json!({
            "sessionId": sid,
            "prompt": [{"type": "text", "text": text}]
        }))?;
        self.collect_response(id)
    }

    /// Send a text prompt with @file references prepended.
    pub fn prompt_with_files(&mut self, text: &str, files: &[&str]) -> Result<String, AcpError> {
        let refs: String = files.iter().map(|f| format!("@{} ", f)).collect();
        self.prompt(&format!("{}{}", refs, text))
    }

    fn send(&mut self, method: &str, params: serde_json::Value) -> Result<u64, AcpError> {
        let id = self.next_id;
        self.next_id += 1;
        let msg = serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let line = serde_json::to_string(&msg)
            .map_err(|e| AcpError(format!("serialize: {}", e)))?;
        self.stdin.write_all(line.as_bytes())
            .and_then(|_| self.stdin.write_all(b"\n"))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| AcpError(format!("write stdin: {}", e)))?;
        Ok(id)
    }

    /// Read lines until one has matching id. Skip notifications.
    fn read_until_id(&mut self, expected_id: u64) -> Result<serde_json::Value, AcpError> {
        loop {
            let line = self.read_line()?;
            if line.get("id").and_then(|v| v.as_u64()) == Some(expected_id) {
                return Ok(line);
            }
        }
    }

    /// Read lines until matching id. Collect agent_message_chunk text along the way.
    fn collect_response(&mut self, expected_id: u64) -> Result<String, AcpError> {
        let mut chunks = Vec::new();
        loop {
            let line = self.read_line()?;

            // Final response
            if line.get("id").and_then(|v| v.as_u64()) == Some(expected_id) {
                if let Some(err) = line.get("error") {
                    return Err(AcpError(format!("prompt error: {}", err)));
                }
                return Ok(chunks.join("").trim().to_string());
            }

            // Notification: extract agent_message_chunk text
            // Structure: params.update.sessionUpdate == "agent_message_chunk"
            //            params.update.content.text == "..."
            if let Some(update) = line.get("params").and_then(|p| p.get("update")) {
                if update.get("sessionUpdate").and_then(|v| v.as_str()) == Some("agent_message_chunk") {
                    if let Some(text) = update.get("content").and_then(|c| c.get("text")).and_then(|v| v.as_str()) {
                        chunks.push(text.to_string());
                    }
                }
            }
        }
    }

    fn read_line(&mut self) -> Result<serde_json::Value, AcpError> {
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
        // stdin is dropped here, which closes the pipe → gemini exits
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // requires gemini CLI
    fn handshake_and_prompt() {
        let cwd = std::env::current_dir().unwrap();
        let mut session = AcpSession::new(&cwd).expect("handshake failed");
        assert!(!session.session_id().is_empty());

        let result = session.prompt("2+2は？数字だけ答えろ").expect("prompt failed");
        assert_eq!(result.contains('4'), true, "expected 4, got: {}", result);
    }

    #[test]
    #[ignore] // requires gemini CLI
    fn sequential_prompts() {
        let cwd = std::env::current_dir().unwrap();
        let mut session = AcpSession::new(&cwd).expect("handshake failed");

        let r1 = session.prompt("2+3は？数字だけ").expect("prompt 1 failed");
        assert!(r1.contains('5'), "expected 5, got: {}", r1);

        let r2 = session.prompt("7*8は？数字だけ").expect("prompt 2 failed");
        assert!(r2.contains("56"), "expected 56, got: {}", r2);
    }
}
