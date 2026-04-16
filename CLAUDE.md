# resident-agent (Go version)

ACP (JSON-RPC 2.0 over stdio) で AI CLI を常駐セッションとして制御する。
Rust 版から Go 1.24.4 に書き換え済み。

## 仕様

### セッションモデル

- **短命セッション**: initialize → session/new → session/prompt → Close()
- **長命セッション**: initialize → session/new → session/prompt × N → Close()

### プロトコル

Gemini CLI `--acp` フラグで起動。stdin/stdout で JSON-RPC 2.0 通信。

## テスト

### ユニットテスト

```bash
go test ./...
```

### ライブテスト

```bash
go run main.go "2+2は？"
```

## 構成

```
pkg/
├── acp/     — Session + JSON-RPC ロジック
└── api/     — 互換 API: Analyze(), Prompt(), types
main.go      — CLI エントリポイント
```

## 互換性

`AnalyzeOptions` を使用して、バックエンドや出力形式を制御可能。
`api.Analyze()` または `api.Prompt()` を使用して、既存の `cli-ai-analyzer` と同等の機能を提供。
