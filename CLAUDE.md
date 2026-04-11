# resident-ai

ACP (JSON-RPC 2.0 over stdio) で AI CLI を常駐セッションとして制御するクレート。

## 仕様

### セッションモデル

- **短命セッション**: initialize → session/new → session/prompt → drop
- **長命セッション**: initialize → session/new → session/prompt × N → drop

同一 API。持続時間が違うだけ。

### プロトコル

Gemini CLI `--acp` フラグで起動。stdin/stdout で JSON-RPC 2.0 通信。

## ビルド・テスト

```bash
cargo test              # 18 pass (ユニットテスト)
cargo test -- --ignored # + 2 ライブテスト (gemini CLI 必要)
```

## 構成

```
src/
├── lib.rs  — モジュール宣言
└── acp.rs  — AcpSession + 公開純粋関数 + テスト
examples/
└── photo_analyze.rs — 画像解析デモ
tests/
├── acp_handshake.py  — Python 実証コード
└── acp_behavior.py   — 動作特性テスト
```
