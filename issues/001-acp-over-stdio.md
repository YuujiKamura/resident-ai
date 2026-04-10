# Issue #001: ConPTY を捨てて ACP (JSON-RPC over stdio) に移行する

## 背景

resident-ai は ConPTY で AI CLI を TTY として常駐させ、`<RESULT>` タグで応答を抽出する設計だった。
しかし ConPTY には以下の問題がある:

- **output pipe にテキストが流れない**: mintty/Git Bash/console host からは制御シーケンスしか来ない
- **GUI ホスト前提**: ghostty-win のような非コンソールアプリからのみ動作（未検証）
- **isatty 偽装が目的**: そもそも Gemini CLI の TTY チェックを騙すための手段

## 発見

Gemini CLI は `--acp` フラグで **Agent Communication Protocol** モードを提供している。
これは Google が公式に用意した「プログラムから Gemini CLI を使う正解」。

### ACP の仕様

- **プロトコル**: JSON-RPC 2.0 over stdio（stdin/stdout）
- **TTY 不要**: パイプで動く。ConPTY もタグ抽出も不要
- **構造化応答**: JSON で返ってくる。TUI パースや ANSI 除去が不要
- **セッション管理**: `newSession` / `prompt` / `cancel` メソッドがある
- **ドキュメント**: https://geminicli.com/docs/cli/acp-mode/

### 実証済み

```bash
# JSON-RPC 2.0 で通信できることを確認
echo '{"jsonrpc":"2.0","method":"tasks/send","id":"1",...}' | gemini --acp
# → {"jsonrpc":"2.0","id":"1","error":{"code":-32601,"message":"Method not found: tasks/send"}}
```

メソッド名が違っただけで、JSON-RPC 2.0 の通信自体は成功している。

### ACP の主要メソッド

| メソッド | 説明 |
|---------|------|
| `initialize` | 接続確立、プロトコルバージョン交換 |
| `newSession` | セッション開始 |
| `prompt` | プロンプト送信→応答受信 |
| `cancel` | 実行中のプロンプトをキャンセル |
| `setSessionMode` | ツール承認レベル変更 |

## やるべきこと

### Phase 1: ACP プロトコル実装

1. `src/acp.rs` — JSON-RPC 2.0 メッセージの組み立てとパース
2. `initialize` → `newSession` → `prompt` のハンドシェイクを実装
3. `std::process::Command` + stdin/stdout パイプで十分。ConPTY 不要

### Phase 2: セッションモデル適用

1. **短命セッション**: `initialize` → `newSession` → `prompt` → drop
2. **長命セッション**: `initialize` → `newSession` → `prompt` × N → drop
3. 仕様で定義した2モデルがそのまま適用できる

### Phase 3: ConPTY コードの扱い

- `conpty.rs` は削除するか、ghostty-win 専用のユーティリティとして別クレートに移す
- タグ抽出 (`extract_tagged`) は ACP で不要になるが、非ACP バックエンド用に残す選択肢もある
- `build_message` も ACP では不要（JSON で送るため）

## つまずきポイント

1. **ACP メソッド名**: `tasks/send` ではなく `prompt` 等の固有メソッド。ドキュメントを正確に読むこと
2. **initialize ハンドシェイク**: `protocolVersion` と `clientCapabilities` が必要。省略すると拒否される可能性
3. **改行区切り JSON**: メッセージは改行（`\n`）で区切る。1行1メッセージ
4. **非同期応答**: `prompt` の応答は即座に返らない。id でコリレーションする
5. **MCP サーバー統合**: ACP 側がファイルシステムアクセスをプロキシする設計。ファイルパスの扱いが変わる可能性

## 参照

- ACP ドキュメント: https://geminicli.com/docs/cli/acp-mode/
- ACP プロトコル仕様: https://agentclientprotocol.com/get-started/introduction
- ローカル: `%APPDATA%/npm/node_modules/@google/gemini-cli/bundle/docs/cli/acp-mode.md`
