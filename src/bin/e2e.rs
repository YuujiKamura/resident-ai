//! E2E test binary for resident-ai ConPTY.
//!
//! Must be run from a GUI host (no console) like ghostty-win.
//! From mintty/Git Bash, output capture will fail.
//!
//! Usage:
//!   cargo build --bin e2e
//!   # Launch from ghostty-win or a GUI-spawned terminal:
//!   ./target/debug/e2e.exe [test_name]
//!
//! Tests:
//!   cmd_echo       — spawn cmd.exe, send echo, verify output in pipe
//!   node_tty       — spawn node via ConPTY, verify isTTY=true
//!   gemini_session — spawn gemini.cmd, send query, extract <RESULT> tag
//!   all            — run all tests (default)

use std::time::{Duration, Instant};

use resident_ai::conpty::ConPty;
use resident_ai::session::ResidentSession;

fn main() {
    // Detach from inherited console so ConPTY output flows through pipes.
    ConPty::detach_console();

    let args: Vec<String> = std::env::args().collect();
    let test_name = args.get(1).map(|s| s.as_str()).unwrap_or("all");

    let mut passed = 0;
    let mut failed = 0;

    match test_name {
        "cmd_echo" => run_one("cmd_echo", test_cmd_echo, &mut passed, &mut failed),
        "node_tty" => run_one("node_tty", test_node_tty, &mut passed, &mut failed),
        "gemini_session" => run_one("gemini_session", test_gemini_session, &mut passed, &mut failed),
        "all" | _ => {
            run_one("cmd_echo", test_cmd_echo, &mut passed, &mut failed);
            run_one("node_tty", test_node_tty, &mut passed, &mut failed);
            run_one("gemini_session", test_gemini_session, &mut passed, &mut failed);
        }
    }

    println!("\n=== Results: {} passed, {} failed ===", passed, failed);
    std::process::exit(if failed > 0 { 1 } else { 0 });
}

fn run_one(name: &str, f: fn() -> Result<(), String>, passed: &mut u32, failed: &mut u32) {
    print!("test {} ... ", name);
    match f() {
        Ok(()) => {
            println!("ok");
            *passed += 1;
        }
        Err(e) => {
            println!("FAILED: {}", e);
            *failed += 1;
        }
    }
}

/// Test 1: spawn various programs, check which ones produce output through ConPTY pipe.
fn test_cmd_echo() -> Result<(), String> {
    // Test 1a: Python (uses WriteFile for stdout)
    let pty = ConPty::spawn("cmd.exe /c python -c \"print('MARKER_PYTHON')\"")
        .map_err(|e| format!("spawn python failed: {}", e))?;
    std::thread::sleep(Duration::from_secs(3));
    pty.flush_render();
    std::thread::sleep(Duration::from_secs(1));
    let buf = pty.read_buffer();
    println!("  python buffer ({} bytes), has MARKER: {}", buf.len(), buf.contains("MARKER_PYTHON"));

    // Test 1b: cmd.exe echo (uses WriteConsoleW)
    let pty2 = ConPty::spawn("cmd.exe /c echo MARKER_CMD")
        .map_err(|e| format!("spawn cmd failed: {}", e))?;
    std::thread::sleep(Duration::from_secs(3));
    pty2.flush_render();
    std::thread::sleep(Duration::from_secs(1));
    let buf2 = pty2.read_buffer();
    println!("  cmd buffer ({} bytes), has MARKER: {}", buf2.len(), buf2.contains("MARKER_CMD"));

    // Test 1c: PowerShell Write-Output
    let pty3 = ConPty::spawn("powershell.exe -NoProfile -Command \"Write-Output 'MARKER_PS'\"")
        .map_err(|e| format!("spawn ps failed: {}", e))?;
    std::thread::sleep(Duration::from_secs(5));
    pty3.flush_render();
    std::thread::sleep(Duration::from_secs(1));
    let buf3 = pty3.read_buffer();
    println!("  powershell buffer ({} bytes), has MARKER: {}", buf3.len(), buf3.contains("MARKER_PS"));

    if buf.contains("MARKER_PYTHON") || buf2.contains("MARKER_CMD") || buf3.contains("MARKER_PS") {
        Ok(())
    } else {
        Err("No program produced output through ConPTY pipe".into())
    }
}

/// Test 2: spawn node.js via ConPTY, verify isTTY is true.
fn test_node_tty() -> Result<(), String> {
    let pty = ConPty::spawn(
        "cmd.exe /c node -e \"console.log('TTY_STDIN:' + !!process.stdin.isTTY); console.log('TTY_STDOUT:' + !!process.stdout.isTTY)\""
    ).map_err(|e| format!("spawn failed: {}", e))?;

    wait_for(&pty, "TTY_STDIN:", 10)?;

    let buf = pty.read_buffer();
    if !buf.contains("TTY_STDIN:true") {
        return Err(format!("stdin isTTY is not true. Buffer: {:?}", &buf[..buf.len().min(500)]));
    }
    if !buf.contains("TTY_STDOUT:true") {
        return Err(format!("stdout isTTY is not true. Buffer: {:?}", &buf[..buf.len().min(500)]));
    }

    Ok(())
}

/// Test 3: spawn gemini.cmd, send a query, extract <RESULT> tag.
fn test_gemini_session() -> Result<(), String> {
    let session = ResidentSession::new("gemini.cmd")
        .map_err(|e| format!("session spawn failed: {}", e))?;

    if !session.is_alive() {
        return Err("gemini.cmd died immediately".into());
    }

    let result = session
        .query("2+2は？ 数字だけ答えろ", None)
        .map_err(|e| format!("query failed: {}", e))?;

    if !result.contains('4') {
        return Err(format!("Expected '4' in response, got: {:?}", result));
    }

    Ok(())
}

/// Poll the ConPTY buffer until `needle` appears or timeout.
fn wait_for(pty: &ConPty, needle: &str, timeout_secs: u64) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    loop {
        std::thread::sleep(Duration::from_millis(200));

        // Periodically flush to ensure ConPTY emits pending output.
        pty.flush_render();

        let buf = pty.read_buffer();
        if buf.contains(needle) {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting for {:?} ({} secs). Buffer ({} bytes): {:?}",
                needle,
                timeout_secs,
                buf.len(),
                &buf[..buf.len().min(500)]
            ));
        }
    }
}
