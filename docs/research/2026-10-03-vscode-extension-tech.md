# VS Code で全行の行末説明をラグ無く出す型（拡張ホスト・wasm・LSP・描画 API）

確認日: 2026-10-03。凡例: [直] = raw ファイル・gh api・vscode ソース。[要約] = WebFetch の要約経由。[推] = 推論。既存記録（inlay hint の 43 文字、typos-lsp、プラットフォーム別 VSIX）は再調査していない。

## 答え

- **搬送**：ネイティブのコアバイナリを子プロセスで起動し、stdio の JSON-RPC（LSP の殻＋独自メソッド）で話す。コアを wasm にして拡張ホストで動かす案は、実測で 15 倍遅い報告があり不採用。
- **描画**：`createTextEditorDecorationType` の `after.contentText` を、**表示中の行（`visibleRanges` ± バッファ）にだけ**付ける。
- **全行・全ファイルに行ごとの別文を一度に付ける設計は最も危ない。** VS Code は `renderOptions` のハッシュごとに CSS のサブタイプを作る。行ごとに文が違えば、行数 = サブタイプ数になる。Error Lens が実際に固まり、2026-09-26 に「1,000 件超なら表示行だけ」へ直した。
- 全文は HoverProvider に回す。CodeLens は行の上に別行で出るので行末には使えない [推]。
- 多言語の構造（関数の範囲）は `vscode.executeDocumentSymbolProvider` を第一にし、言語拡張が無いときはコア側（tree-sitter）で持つ。

## 根拠

### 拡張ホストと描画コスト

- 拡張は別プロセス（Node）か Web Worker で動く https://code.visualstudio.com/api/advanced-topics/extension-host [要約]
- `setDecorations` は `Range[]` なら平坦な配列で送る速い経路、`DecorationOptions[]`（`renderOptions` 付き）なら構造体ごと送る経路の 2 つ https://raw.githubusercontent.com/microsoft/vscode/main/src/vs/workbench/api/common/extHostTextEditor.ts [直]
- レンダラ側は「identify custom render options by a hash code over all keys and values」。`hash(renderOptions)` ごとに型を登録する https://raw.githubusercontent.com/microsoft/vscode/main/src/vs/editor/browser/widget/codeEditor/codeEditorWidget.ts [直]
- Error Lens のコメント「any `renderOptions` object, even an empty one, makes the renderer register a CSS subtype instead of reusing the base decoration type.」 https://raw.githubusercontent.com/usernamehw/vscode-error-lens/master/src/decorations.ts [直]
- `setDecorations` は 1 型につき 1 回の全置換。差分更新はできない [直]

**数値（1 本だけ、未マージ PR の自己計測）**

- microsoft/vscode PR #337313（open、2026-09-22）「I profiled the renderer main thread on a file with ~11k diagnostics...: the extension host was idle, but the renderer was blocked for 66 s, of which 45 s was the scan loop in `removeCSSRulesContainingSelector` ... Inserting the same 36k rules cost 0.4 s.」 https://github.com/microsoft/vscode/pull/337313 [直]
  - 登録は速く、**撤去が二次**で遅い。固まるのは拡張ホストではなくレンダラ。
- Error Lens 3.29.0（2026-09-26）「Render diagnostics only for visible lines when there are many problems in 1 file (>1000)」。設定 `errorLens.maxInlineMessages`（既定 1000）の説明「VSCode creates a separate set of CSS rules for every distinct inline message, so a file with thousands of problems can freeze the editor for a long time.」 https://github.com/usernamehw/vscode-error-lens/blob/master/CHANGELOG.md [直]
  - 実装は `visibleRanges` を `viewportLineBuffer` 行ぶん広げ、範囲外の行から `renderOptions` を外す [直]

| 拡張 | 描画の範囲 | 根拠 |
|---|---|---|
| GitLens inline blame | カーソル行のみ | `lineAnnotationController.ts` [直] |
| Error Lens | 全診断。1,000 件超は表示行のみ | `decorations.ts` [直] |

- 苦情の実例: Error Lens #242「high cpu load」、GitLens #3616「Inline Blame debounce」（上下キー押しっぱなしでラグ、open）[直]

### 非 JS コアを VS Code で動かす

- **ネイティブ Node モジュール**：Electron の ABI で再ビルドが要る。子プロセスのバイナリなら避けられる。Ruff は同梱バイナリを `spawn` する https://github.com/astral-sh/ruff-vscode [直]
- **WebAssembly**：`ms-vscode.wasm-wasi-core`（WASI Preview 1、「WASI is work in progress」）https://github.com/microsoft/vscode-wasm/blob/main/wasm-wasi-core/README.md [直]
  - wasm は別 worker で動き、`SharedArrayBuffer` + `Atomics` で同期する。vscode.dev では cross-origin isolation が要る https://raw.githubusercontent.com/microsoft/vscode-docs/main/blogs/2023/06/05/vscode-wasm-wasi.md [直]
  - 「The VS Code API is only accessible within the extension host worker」「Extensions should avoid performing any long-running synchronous computations on that worker.」 https://raw.githubusercontent.com/microsoft/vscode-docs/main/blogs/2024/05/08/wasm.md [直]
  - Rust から VS Code API を直接呼ぶ案は公式が見送り：「we have decided not to proceed with it for now. The primary reason is the lack of async support in WASM.」[直]
  - **性能**：vscode-wasm #234「it is 15x slower than on wasmtime」。保守者「I can't improve the performance of the WASM runtime that ships in Electron / Browser」 https://github.com/microsoft/vscode-wasm/issues/234 [直]（報告者 1 人の計測）
  - wasm の LSP サーバの前例は testbed のみ https://github.com/microsoft/vscode-wasm/tree/main/testbeds/lsp-rust [直]
- **Web（vscode.dev）**：「Creating child processes or running executables is not possible.」 https://raw.githubusercontent.com/microsoft/vscode-docs/main/api/extension-guides/web-extensions.md [直]。子プロセス型は vscode.dev で動かない。

### LSP と拡張 API の使い分け

- 公式：「While Language Servers have many benefits, they are not the only option」。LSP を使う理由は「implemented in their native programming languages」と「resource intensive」の 2 つ https://code.visualstudio.com/api/language-extensions/language-server-extension-guide [要約]
- LSP でない例：GitLens（9,939★）、Error Lens（841★）、coverage-gutters。LSP の例：Copilot の補完 [直]
- LSP で表せないもの（decoration、webview）はクライアント側で描く。clangd の拡張がカスタム通知 → `setDecorations` の実例 https://raw.githubusercontent.com/clangd/vscode-clangd/master/src/inactive-regions.ts [直]。ただし clangd は `Range[]` のみで、行ごとに別文を送る本件の負荷は前例の範囲外。

### 多言語の構造

- `vscode.executeDocumentSymbolProvider` は結果が空なら `undefined` を返す。**言語拡張が無いと「空」と「プロバイダ無し」を区別できない** https://raw.githubusercontent.com/microsoft/vscode/main/src/vs/workbench/api/common/extHostApiCommands.ts [直]
- anycode（387★）：tree-sitter の wasm で outline を出す。README「_inaccurately_ implements」、対応 7 言語、最終リリース 2025-08-18 [直]

### 起動

- `onStartupFinished` は起動を遅らせない https://raw.githubusercontent.com/microsoft/vscode-docs/main/api/references/activation-events.md [直]。活性化時間の公式予算（ms）は見つからなかった。

## 注意点

- 行数に対する `setDecorations` の遅延を測った公開ベンチは無い。50 / 500 / 5,000 行で自前計測が要る。
- 表示行だけ付け替える方式でも、スクロールのたびにサブタイプの登録と撤去が起きる。Error Lens 3.29.0 の体感の報告は未確認。
- GitLens の「カーソル行のみ」を全行に外挿しない。
- vscode.dev 対応を捨てるなら子プロセス型で足りる。捨てないなら wasm（15 倍遅い）か degrade 動作が要る。

## 前例が見つからなかったもの

- 全行（数千行）の「行ごとに別文」の EOL 装飾を一括で付ける製品。
- Rust・Go・Zig の wasm コアを拡張ホストで本番に載せた製品。
- 公式文書における「decoration type は使い回せ」の明文。
