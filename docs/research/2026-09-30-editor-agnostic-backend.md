# 1 つのバックエンドを Neovim・VS Code・静的 HTML に出す型

確認日: 2026-09-30。凡例: [直] = gh api・raw ファイル・crates.io API。[要約] = WebFetch の要約経由。[推] = 推論・未検証。

## 答え

- 推奨は、コア・LSP・HTML の 3 つに分けた形。
  - コア（Rust の library crate）が「読解の束（JSON）」を作る。
  - 静的 HTML は、その束をテンプレートで描くだけ。
  - LSP サーバは同じコアを呼ぶ薄い殻にして、ライブ更新と、エディタに標準である面（diagnostics、hover）を担う。
- シナリオ画面と行ごとの長い説明は、LSP の標準機能では出せない。カスタムリクエストでデータを渡し、エディタごとの UI で描く（Neovim は extmark と buffer、VS Code は decoration と webview）。
- 近い実例は Harper。1 つの Rust コア（`harper-core`）に `harper-ls`（LSP）と `harper-wasm`（Web）を付け、Neovim・VS Code・Web に出している。
- 非言語の道具を LSP に載せた実例（harper-ls、typos-lsp、SonarLint、Snyk、Copilot）は、どれも診断・補完・コードアクションの枠に収まる用途。実行結果のビューを LSP で出す実例は見つからなかった。
- Rust の LSP crate は `tower-lsp-server` を使う。本家の `tower-lsp` は 2023-08 から公開が止まっている。

## 根拠

- Harper（16,053★）。`tower-lsp-server = "0.22.1"` https://github.com/Automattic/harper [直]
- typos-lsp（588★）。`tower-lsp-server = "0.23"`。VS Code 拡張は `extension.ts` 1 ファイルで、9 ターゲット分のバイナリを同梱する https://github.com/tekumara/typos-lsp [直]
- Copilot LS「many custom messages are employed to support the unique features of Copilot.」 https://github.com/github/copilot-language-server-release [直]
- rust-analyzer は標準外の要求を `experimental/` と `rust-analyzer/` の名前空間に置き、`experimental` の capabilities で宣言する https://raw.githubusercontent.com/rust-lang/rust-analyzer/master/docs/book/src/contributing/lsp-extensions.md [直]
- inlay hint は長文に向かない。VS Code は `editor.inlayHints.maximumLength` の既定が 43 で、1 行の合計がこれを超えると切れる https://raw.githubusercontent.com/microsoft/vscode/main/src/vs/editor/common/config/editorOptions.ts [直]。Neovim の切り詰め要望 #27240 は open https://github.com/neovim/neovim/issues/27240 [直]
- Neovim は `DiagnosticRelatedInformation` に対応し、`gf` で関連位置へ飛べる https://raw.githubusercontent.com/neovim/neovim/master/runtime/doc/lsp.txt [直]。race の 2 か所はそのまま載せられる。
- Neovim 0.12 の codelens は表示位置が変わり、設定できない（#39477、open）。空の仮想行が残る不具合もある（#41074、open） [直]
- カスタムメソッドは、Neovim の `Client:request` でも、VS Code の `vscode-languageclient` の `sendRequest` でも送れる [要約]
- ファイル型の実例: vscode-coverage-gutters（lcov を監視して読む）https://github.com/ryanluker/vscode-coverage-gutters [直]。nvim-coverage は 2024-12 から更新が無い [直]。Neovim 向けの SARIF ビューアと CodeTour リーダーは検索に出なかった。

| crate | 最新 | 最終公開 | メモ |
|---|---|---|---|
| tower-lsp | 0.20.0 | 2023-08-11 | 保守の継続を問う #427 が open |
| tower-lsp-server | 0.23.0 | 2026-09-11 | コミュニティの fork。Harper と typos-lsp が採用 |
| lsp-server | 0.10.0 | 2026-07-16 | rust-analyzer の一部。同期で動き tokio が要らない |
| lsp-types | 0.97.0 | 2024-06-04 | LSP 3.16 まで。保守者不在を問う #312 が open |

## 注意点

- LSP を使っても、シナリオ画面は Lua と TypeScript で 2 回書くことになる。
- テストの実行は数秒〜数分かかるので、診断が古くなる。「実行中」の通知と、結果に版番号を持たせる設計は自前で作る必要がある [推]。
- ファイル型の束は、編集すると行がずれる。対象のコミットとファイル内容のハッシュを持たせ、一致しなければ古いと表示するのが最小の対策 [推]。
- VS Code が relatedInformation を Problems パネルに出すことは、公式文書では確かめていない。
- tower-lsp-server は 0.x なので、API はまだ変わる。
