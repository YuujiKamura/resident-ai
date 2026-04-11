# resident-ai

ACP (JSON-RPC 2.0 over stdio) で AI CLI を常駐セッションとして制御するクレート。

## 仕様

### セッションモデル

- **短命セッション**: initialize → session/new → session/prompt → drop
- **長命セッション**: initialize → session/new → session/prompt × N → drop

同一 API。持続時間が違うだけ。

### プロトコル

Gemini CLI `--acp` フラグで起動。stdin/stdout で JSON-RPC 2.0 通信。

## テストの方針

### テストは3層ある

| 層 | 何を証明するか | gemini CLI |
|----|--------------|-----------|
| ユニット | JSON 組み立て・パース・型の正しさ | 不要 |
| ライブ | ACP ハンドシェイク→プロンプト→応答が返る | 必要 |
| リレーション | photo-engine → resident-ai → gemini → 応答 | 必要 |

**リレーションテストが最も重要。** ビルドが通ることと動くことは違う。

### テスト実行

```bash
# ユニットテスト（CI 向け、gemini 不要）
cargo test

# ライブテスト（gemini CLI 認証済み環境で実行）
cargo test -- --ignored --nocapture

# リレーションテスト（photo-engine から resident-ai 経由で解析）
cd ../photo-ai-rust && cargo test -p photo-engine test_resident_ai -- --ignored --nocapture
```

### リレーションテストの書き方

photo-engine 側に resident-ai を実際に呼ぶテストを置く:

```rust
#[test]
#[ignore] // requires gemini CLI
fn test_resident_ai_text_prompt() {
    let opts = AnalyzeOptions::default();
    let result = analyze("2+2は？数字だけ", &[] as &[PathBuf], opts);
    assert!(result.is_ok());
    assert!(result.unwrap().contains('4'));
}

#[test]
#[ignore] // requires gemini CLI + test image
fn test_resident_ai_image_analyze() {
    let opts = AnalyzeOptions::default().json();
    let result = analyze("この画像の内容を説明しろ", &[PathBuf::from("test_image.png")], opts);
    assert!(result.is_ok());
    assert!(!result.unwrap().is_empty());
}
```

## 構成

```
src/
├── lib.rs   — モジュール宣言 + pub use 再エクスポート
├── acp.rs   — AcpSession + JSON-RPC 純粋関数 (18 tests)
└── api.rs   — 互換 API: analyze(), prompt(), types (3 tests)
```

## 依存元

- `photo-ai-rust/photo-engine` — `resident-ai = { path = "../../resident-agent" }` で参照
- `photo-ai-rust/desktop-rust` — 同上

## 変更時の確認手順

resident-ai を変更したら以下を全部通すこと:

```bash
cd ~/resident-agent && cargo test
cd ~/photo-ai-rust && cargo build -p photo-engine
cd ~/photo-ai-rust && cargo test -p photo-engine
```
