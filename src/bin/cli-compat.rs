//! cli-ai-analyzer 互換 CLI (ACP 経路版)
//!
//! cli-ai-analyzer.exe を呼ぶ既存の Go/Rust callsite を、本バイナリに
//! 差し替えるだけで ACP (gemini.cmd --acp) 経由になる。
//!
//! サポートする呼び出し形式:
//!   cli-compat analyze --prompt "..." --json --model M file1 file2 ...
//!
//! 等価性検証用。プロンプト生成や JSON 後処理は cli-ai-analyzer と同じ。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use resident_ai::acp::AcpSession;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut subcommand: Option<String> = None;
    let mut prompt: Option<String> = None;
    let mut json: bool = false;
    let mut _model: Option<String> = None;
    let mut files: Vec<PathBuf> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "analyze" | "a" => { subcommand = Some("analyze".into()); }
            "prompt" | "p" => { subcommand = Some("prompt".into()); }
            "--prompt" | "-p" => {
                i += 1;
                if i >= args.len() { eprintln!("--prompt requires value"); return ExitCode::from(2); }
                prompt = Some(args[i].clone());
            }
            "--json" => { json = true; }
            "--model" | "-m" => {
                i += 1;
                if i >= args.len() { eprintln!("--model requires value"); return ExitCode::from(2); }
                _model = Some(args[i].clone());
            }
            "--cli-path" | "--backend" | "--pay-per-use" => {
                // accept-and-ignore for compat (ACP は backend=gemini 固定、cli_path 不要)
                if a != "--pay-per-use" {
                    i += 1; // skip value
                }
            }
            other => {
                if other.starts_with("--") {
                    eprintln!("unknown flag: {}", other);
                    return ExitCode::from(2);
                }
                files.push(PathBuf::from(other));
            }
        }
        i += 1;
    }

    let sub = match subcommand {
        Some(s) => s,
        None => { eprintln!("expected subcommand: analyze or prompt"); return ExitCode::from(2); }
    };
    let prompt = match prompt {
        Some(p) => p,
        None => { eprintln!("--prompt is required"); return ExitCode::from(2); }
    };

    let full_prompt = if json {
        format!("{} Respond with ONLY the JSON object.", prompt)
    } else {
        prompt
    };

    let cwd = match std::env::current_dir() {
        Ok(c) => c,
        Err(e) => { eprintln!("current_dir: {}", e); return ExitCode::from(1); }
    };

    let mut session = match AcpSession::new_with_model(&cwd, _model.as_deref()) {
        Ok(s) => s,
        Err(e) => { eprintln!("ACP session init failed: {}", e); return ExitCode::from(1); }
    };

    let result = if sub == "prompt" || files.is_empty() {
        session.prompt(&full_prompt)
    } else {
        let refs: Vec<&Path> = files.iter().map(|p| p.as_path()).collect();
        session.prompt_with_images_inline(&full_prompt, &refs)
    };

    match result {
        Ok(text) => {
            println!("{}", text);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("ACP prompt failed: {}", e);
            ExitCode::from(1)
        }
    }
}
