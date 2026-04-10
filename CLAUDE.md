# resident-ai

ConPTY 上で AI CLI を動かし、タグベースで応答を抽出するクレート。

## 仕様

### セッションモデル

resident-ai は ConPTY 上の AI CLI セッションを提供する。セッションの持続時間によって2つの使い方がある:

- **短命セッション**: 1回の query で起動→応答→終了。呼び出し側からはワンショットに見える
- **長命セッション**: 起動したまま複数 query を受ける。起動コストを償却する

どちらも同じ基盤（ConPTY + タグ抽出）の上にある。持続時間が違うだけで、API は同一。

### なぜ ConPTY が必要か

AI CLI（gemini, claude 等）は `isatty(stdin)` でインタラクティブモードを判定する。
`std::process::Command` のパイプ stdin では非TTY扱いになり、ワンショットで終了するか起動を拒否する。
ConPTY は OS レベルで TTY を提供し、CLI がインタラクティブモードで起動する。

### 応答抽出

TUI 出力をパースせず、プロンプトに「`<RESULT>` タグで囲め」と指示し、タグ間のテキストを抽出する。
TUI の装飾・ANSI エスケープ・Unicode は無視される。

## 現状

### ビルド・テスト
```bash
cargo build   # OK
cargo test -- --test-threads=1   # 51 pass
```

### 構成
```
src/
├── lib.rs       — モジュール宣言
├── conpty.rs    — Windows ConPTY ラッパー (named pipe input + anonymous pipe output)
├── session.rs   — ResidentSession + build_message + extract_tagged
└── bin/e2e.rs   — GUI ホスト用 E2E テストバイナリ（未検証）
```

### テストカバレッジ（51テスト）
1. CLI パス解決（spawn 成功/失敗/空文字列）
2. プロンプト構築（ファイルなし/あり/タグ指示）
3. モデル選択（プロンプト保持/空/Unicode）
4. 出力フォーマット（カスタムタグ/不一致タグ）
5. ファイルパス（単一/複数/スペース含む）
6. サブプロセス起動（OK/alive/引数付き）
7. バッファキャプチャ（初期サイズ/len一致/write OK）
8. タイムアウト（エラー形式/None-Some パス）
9. ANSI ノイズ耐性（エスケープ/カーソル/混合Unicode）
10. セッション起動（alive/無効コマンド/デフォルトタグ）
11. 逐次クエリ（ベースライン追跡/再抽出防止）
12. エラーハンドリング（死んだプロセス/空文字列）
13. リソース管理（パイプカウンタ/一意性/drop）
14. メトリクス（バッファ成長/is_alive/タイムアウト定数）
15. Drop クリーンアップ（生存/死亡/サイクル）

### 未検証事項

- ConPTY output pipe 経由のテキスト受信（GUI ホストからの実行が必要）
- Node.js の `process.stdin.isTTY` が ConPTY 下で true になるか
- gemini CLI のインタラクティブモード起動
- `detach_console` / `flush_render` の実効性（mintty からは無効だった）

## 参照

- `~/ghostty-win/src/pty.zig:324-478` — ConPTY 実装リファレンス（named pipe パターン）
- `~/cli-ai-analyzer/` — 既存の AI CLI ラッパー（std::process::Command ベース）
