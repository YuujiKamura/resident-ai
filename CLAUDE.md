# resident-ai

ConPTY で AI CLI を常駐させ、タグベースで応答を抽出するクレート。

## 現状 (2026-04-10)

### ビルド・テスト
```bash
cargo build   # OK
cargo test    # 6 pass, 1 ignored (ConPTY flush問題)
```

### 構成
```
src/
├── lib.rs       — モジュール宣言
├── conpty.rs    — Windows ConPTY ラッパー (CreatePseudoConsole + バックグラウンドリーダー)
└── session.rs   — ResidentSession + タグ抽出 (<RESULT>...</RESULT>)
```

### 次にやること（優先順）

1. **ConPTY output パイプを named pipe に変更**
   - 現在 `CreatePipe`（匿名パイプ）→ 出力の flush が遅い
   - ghostty-win の `src/pty.zig:370` のように `CreateNamedPipeW` を使う
   - `\\.\pipe\LOCAL\resident-ai-{pid}-{counter}` 形式
   - これで `test_spawn_and_write` の `#[ignore]` を外せる

2. **ライブテスト: gemini.cmd を ConPTY で起動**
   - `ResidentSession::new("gemini.cmd")` で常駐起動
   - `session.query("2+2は？", None)` でタグ抽出が動くか検証
   - 画像付き: `session.query("解析しろ", Some(&["photo.jpg"]))` 

3. **cli-ai-analyzer の Resident モードを resident-ai に切り替え**
   - `cli-ai-analyzer/src/executor.rs:156-175` の Resident 分岐
   - `deckpilot.rs` → `resident-ai` クレート依存に変更
   - photo-ai-rust の `deps/cli-ai-analyzer` も同期

4. **ディレクトリ名リネーム**
   - `~/resident-agent` → `~/resident-ai`
   - パッケージ名は既に `resident-ai`

## 設計の背景

- Gemini CLI は `-i`(インタラクティブ) + パイプ入力を明示的にブロック
- パイプ stdin → headless 1ショットで死ぬ（`isatty` チェック）
- ConPTY なら TTY として認識される → インタラクティブモードで常駐
- 応答抽出は TUI パースではなく `<RESULT>` タグで指示 → 確実

## 参照

- `~/ghostty-win/src/pty.zig:324-478` — ConPTY 実装リファレンス（named pipe パターン）
- `~/cli-ai-analyzer/` — TimeBasedQuota/PayPerUse の既存実装
- `~/deckpilot/` — セッション管理（将来的に統合候補）
