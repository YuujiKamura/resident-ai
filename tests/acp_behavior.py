"""ACP behavior tests: readiness, concurrent prompts, cancel, session persistence"""
import subprocess
import json
import sys
import os
import time
import threading

# Force UTF-8 stdout on Windows
if sys.platform == "win32":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

def start_gemini():
    return subprocess.Popen(
        ["gemini.cmd", "--acp"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, bufsize=1, encoding="utf-8", errors="replace",
    )

def send(proc, msg):
    line = json.dumps(msg) + "\n"
    proc.stdin.write(line)
    proc.stdin.flush()

def read_line(proc, timeout=30):
    result = [None]
    def reader():
        result[0] = proc.stdout.readline()
    t = threading.Thread(target=reader, daemon=True)
    t.start()
    t.join(timeout)
    if result[0]:
        return json.loads(result[0])
    return None

def read_all(proc, timeout=30):
    """Read all lines until response with matching id or timeout."""
    lines = []
    deadline = time.time() + timeout
    while time.time() < deadline:
        resp = read_line(proc, timeout=max(1, deadline - time.time()))
        if resp is None:
            break
        lines.append(resp)
        if "id" in resp and "result" in resp:
            break
        if "id" in resp and "error" in resp:
            break
    return lines

def handshake(proc):
    """initialize + session/new, return sessionId"""
    send(proc, {"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":1,"clientInfo":{"name":"test","version":"0.1.0"}
    }})
    resp = read_line(proc, timeout=15)
    assert resp and "result" in resp, f"initialize failed: {resp}"

    send(proc, {"jsonrpc":"2.0","id":2,"method":"session/new","params":{
        "cwd": os.getcwd(), "mcpServers": []
    }})
    resp = read_line(proc, timeout=10)
    assert resp and "result" in resp, f"session/new failed: {resp}"
    return resp["result"]["sessionId"]

def extract_text(lines):
    """Extract agent text from session/update notifications."""
    text = ""
    for l in lines:
        if l.get("method") == "session/update":
            update = l.get("params", {}).get("update", {})
            if update.get("sessionUpdate") == "agent_message_chunk":
                content = update.get("content", {})
                if content.get("type") == "text":
                    text += content.get("text", "")
    return text

# ============================================================

def test_1_readiness_timing():
    """Q: initialize前にsession/promptを送るとどうなる？"""
    print("\n=== Test 1: Readiness — prompt before initialize ===")
    proc = start_gemini()
    time.sleep(1)

    # Send prompt BEFORE initialize
    send(proc, {"jsonrpc":"2.0","id":99,"method":"session/prompt","params":{
        "sessionId":"fake","prompt":[{"type":"text","text":"hello"}]
    }})
    resp = read_line(proc, timeout=5)
    print(f"  prompt before init: {json.dumps(resp)}")

    proc.kill()
    return "error" in (resp or {})

def test_2_prompt_before_session():
    """Q: initialize後、session/new前にpromptを送ると？"""
    print("\n=== Test 2: Readiness — prompt before session/new ===")
    proc = start_gemini()

    send(proc, {"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":1,"clientInfo":{"name":"test","version":"0.1.0"}
    }})
    read_line(proc, timeout=15)  # consume initialize response

    send(proc, {"jsonrpc":"2.0","id":99,"method":"session/prompt","params":{
        "sessionId":"fake","prompt":[{"type":"text","text":"hello"}]
    }})
    resp = read_line(proc, timeout=5)
    print(f"  prompt before session: {json.dumps(resp)}")

    proc.kill()
    return "error" in (resp or {})

def test_3_concurrent_prompts():
    """Q: 1つ目のpromptが完了する前に2つ目を送ると？キューされる？エラー？"""
    print("\n=== Test 3: Concurrent prompts ===")
    proc = start_gemini()
    sid = handshake(proc)

    # Send two prompts rapidly
    send(proc, {"jsonrpc":"2.0","id":10,"method":"session/prompt","params":{
        "sessionId":sid,"prompt":[{"type":"text","text":"1から10まで数えろ"}]
    }})
    time.sleep(0.1)
    send(proc, {"jsonrpc":"2.0","id":11,"method":"session/prompt","params":{
        "sessionId":sid,"prompt":[{"type":"text","text":"100+200は？"}]
    }})

    # Collect all responses
    lines = read_all(proc, timeout=30)
    # Check if we got response for id:11
    got_10 = any(l.get("id") == 10 for l in lines)
    got_11 = any(l.get("id") == 11 for l in lines)
    errors = [l for l in lines if "error" in l]

    print(f"  got id:10 response: {got_10}")
    print(f"  got id:11 response: {got_11}")
    print(f"  errors: {len(errors)}")
    for e in errors:
        print(f"    {json.dumps(e)[:200]}")

    # Read more to see if id:11 comes later
    if not got_11:
        more = read_all(proc, timeout=30)
        got_11 = any(l.get("id") == 11 for l in more)
        print(f"  got id:11 after waiting: {got_11}")

    proc.kill()
    return True

def test_4_cancel():
    """Q: session/cancelでプロンプトを中断できるか？"""
    print("\n=== Test 4: Cancel ===")
    proc = start_gemini()
    sid = handshake(proc)

    # Send a slow prompt
    send(proc, {"jsonrpc":"2.0","id":10,"method":"session/prompt","params":{
        "sessionId":sid,"prompt":[{"type":"text","text":"フィボナッチ数列の最初の50項を1つずつ改行して出力しろ"}]
    }})
    time.sleep(2)

    # Cancel it
    send(proc, {"jsonrpc":"2.0","id":11,"method":"session/cancel","params":{
        "sessionId":sid
    }})

    lines = read_all(proc, timeout=10)
    cancel_resp = [l for l in lines if l.get("id") == 11]
    prompt_resp = [l for l in lines if l.get("id") == 10]

    print(f"  cancel response: {json.dumps(cancel_resp)[:300]}")
    print(f"  prompt response after cancel: {json.dumps(prompt_resp)[:300]}")

    proc.kill()
    return True

def test_5_sequential_prompts():
    """Q: 1つ目完了後に2つ目を送ると正常に動くか？（セッション持続）"""
    print("\n=== Test 5: Sequential prompts (session persistence) ===")
    proc = start_gemini()
    sid = handshake(proc)

    # First prompt
    send(proc, {"jsonrpc":"2.0","id":10,"method":"session/prompt","params":{
        "sessionId":sid,"prompt":[{"type":"text","text":"2+3は？数字だけ"}]
    }})
    lines1 = read_all(proc, timeout=30)
    text1 = extract_text(lines1)
    print(f"  prompt 1 response: {repr(text1)}")

    # Second prompt
    send(proc, {"jsonrpc":"2.0","id":11,"method":"session/prompt","params":{
        "sessionId":sid,"prompt":[{"type":"text","text":"7*8は？数字だけ"}]
    }})
    lines2 = read_all(proc, timeout=30)
    text2 = extract_text(lines2)
    print(f"  prompt 2 response: {repr(text2)}")

    proc.kill()
    has_5 = "5" in text1
    has_56 = "56" in text2
    print(f"  prompt 1 correct (contains 5): {has_5}")
    print(f"  prompt 2 correct (contains 56): {has_56}")
    return has_5 and has_56

def test_6_process_lifetime():
    """Q: stdinを閉じるとプロセスは終了するか？"""
    print("\n=== Test 6: Process lifetime after stdin close ===")
    proc = start_gemini()
    _ = handshake(proc)

    proc.stdin.close()
    try:
        exit_code = proc.wait(timeout=10)
        print(f"  process exited with code: {exit_code}")
        return True
    except subprocess.TimeoutExpired:
        print(f"  process still alive after stdin close")
        proc.kill()
        return False

# ============================================================

if __name__ == "__main__":
    tests = [
        ("readiness_before_init", test_1_readiness_timing),
        ("readiness_before_session", test_2_prompt_before_session),
        ("concurrent_prompts", test_3_concurrent_prompts),
        ("cancel", test_4_cancel),
        ("sequential_prompts", test_5_sequential_prompts),
        ("process_lifetime", test_6_process_lifetime),
    ]

    results = {}
    for name, fn in tests:
        try:
            ok = fn()
            results[name] = "PASS" if ok else "FAIL"
        except Exception as e:
            results[name] = f"ERROR: {e}"

    print("\n" + "=" * 50)
    for name, result in results.items():
        print(f"  {name}: {result}")
