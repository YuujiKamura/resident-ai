"""ACP handshake test: initialize → list methods → newSession → prompt"""
import subprocess
import json
import sys
import time

def send(proc, msg):
    """Send JSON-RPC message and read response."""
    line = json.dumps(msg) + "\n"
    print(f">>> {msg['method']} (id={msg['id']})", file=sys.stderr)
    proc.stdin.write(line)
    proc.stdin.flush()

def read_line(proc, timeout=15):
    """Read one line from stdout."""
    import select
    # Windows doesn't have select on pipes, use threading
    import threading
    result = [None]
    def reader():
        result[0] = proc.stdout.readline()
    t = threading.Thread(target=reader, daemon=True)
    t.start()
    t.join(timeout)
    if result[0]:
        return json.loads(result[0])
    return None

def main():
    proc = subprocess.Popen(
        ["gemini.cmd", "--acp"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )

    # 1. initialize
    send(proc, {
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": 1, "clientInfo": {"name": "resident-ai", "version": "0.1.0"}}
    })
    resp = read_line(proc, timeout=15)
    print(f"<<< initialize: {json.dumps(resp, indent=2)}")

    if not resp or "result" not in resp:
        print("FAIL: initialize failed")
        proc.kill()
        return 1

    # 2. session/new (requires cwd and mcpServers)
    import os
    send(proc, {
        "jsonrpc": "2.0", "id": 2, "method": "session/new",
        "params": {"cwd": os.getcwd(), "mcpServers": []}
    })
    resp = read_line(proc, timeout=10)
    print(f"<<< session/new: {json.dumps(resp, indent=2)}")

    if not resp or "result" not in resp:
        print(f"FAIL: session/new failed")
        proc.kill()
        return 1

    session_id = resp["result"]["sessionId"]
    print(f"Session ID: {session_id}")

    # 3. session/prompt
    send(proc, {
        "jsonrpc": "2.0", "id": 3, "method": "session/prompt",
        "params": {
            "sessionId": session_id,
            "prompt": [{"type": "text", "text": "2+2は？数字だけ答えろ"}]
        }
    })

    # Read responses (may be multiple: notifications + final result)
    for _ in range(20):
        resp = read_line(proc, timeout=30)
        if resp is None:
            print("<<< (timeout)")
            break
        method = resp.get("method", "")
        if "id" in resp and resp["id"] == 3:
            print(f"<<< RESPONSE: {json.dumps(resp, indent=2, ensure_ascii=False)}")
            break
        else:
            print(f"<<< notification [{method}]: {json.dumps(resp, ensure_ascii=False)[:200]}")

    proc.kill()
    return 0

if __name__ == "__main__":
    sys.exit(main())
