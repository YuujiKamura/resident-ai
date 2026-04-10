# resident-ai

ConPTY で AI CLI を常駐させ、タグベースで応答を抽出するクレート。

## 現状 (2026-04-10)

### ビルド・テスト
```bash
cargo build   # OK
cargo test    # 6 pass, 1 ignored (ConPTY診断テスト)
```

### 構成
```
src/
├── lib.rs       — モジュール宣言
├── conpty.rs    — Windows ConPTY ラッパー (named pipe input + anonymous pipe output)
└── session.rs   — ResidentSession + タグ抽出 (<RESULT>...</RESULT>)
```

### 実装済み
- Named pipe 化（input pipe）: `\\.\pipe\LOCAL\resident-ai-{pid}-{counter}`
- `SetHandleInformation` で全ハンドル non-inheritable
- `ResizePseudoConsole` による flush_render
- `FreeConsole` / `AllocConsole` ヘルパー
- タグ抽出ユニットテスト 6件

### 発見: ConPTY output capture の環境制約

**ConPTY の output pipe にテキスト内容が流れるのは、親プロセスにコンソールがない場合のみ。**

| 環境 | ConPTY output pipe | 実用性 |
|------|-------------------|--------|
| GUI アプリ (ghostty-win) | テキスト含む VT sequences | OK |
| cmd.exe / PowerShell | WriteConsole が親コンソールに流出 | FreeConsole で回避可能 |
| mintty / Git Bash | パイプベースで FreeConsole 無効 | 不可 |
| cargo test (Git Bash経由) | 上記と同じ | 診断テストのみ |

`ResizePseudoConsole` でフラッシュすると制御シーケンスは来るがテキストは空。
子プロセス (cmd.exe, Node.js) の出力が ConPTY スクリーンバッファをバイパスしている。

### 次にやること（優先順）

1. **ghostty-win から ConPTY 出力を検証**
   - ghostty-win は GUI アプリ（コンソールなし）→ ConPTY が正常動作するはず
   - `ResidentSession::new("gemini.cmd")` + `session.query()` のE2Eテスト

2. **Node.js isTTY 検証**
   - GUI ホストから ConPTY 経由で Node.js を起動し `process.stdin.isTTY` が true になるか確認
   - true なら gemini CLI はインタラクティブモードで起動する

3. **代替手段の検討（isTTY=false の場合）**
   - `winpty` を PTY レイヤーとして使う（Git for Windows に同梱）
   - `gemini -p "prompt"` でワンショット実行（resident ではないが確実）

## 設計の背景

- Gemini CLI は `-i`(インタラクティブ) + パイプ入力を明示的にブロック
- パイプ stdin → headless 1ショットで死ぬ（`isatty` チェック）
- ConPTY なら TTY として認識される → インタラクティブモードで常駐（GUI ホスト前提）
- 応答抽出は TUI パースではなく `<RESULT>` タグで指示 → 確実

## 参照

- `~/ghostty-win/src/pty.zig:324-478` — ConPTY 実装リファレンス（named pipe パターン）
- `~/cli-ai-analyzer/` — TimeBasedQuota/PayPerUse の既存実装
- `~/deckpilot/` — セッション管理（将来的に統合候補）
