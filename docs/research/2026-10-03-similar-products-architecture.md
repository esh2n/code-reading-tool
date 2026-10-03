# エディタ内コード解説・解析ツールの業界調査（複数言語・複数エディタ・ファイルを開いた時点での表示）

確認日: 2026-10-03。凡例: [直] = gh api・raw ファイル・公式ページ。[要約] = WebFetch/WebSearch の要約経由。[推] = 推論。

## 答え

1. **複数エディタ・複数言語で成功しているツールは「エディタ外の別プロセスのバックエンド + stdio の JSON-RPC」に収束する。** 変種は 3 つ。(a) LSP の枠＋独自メッセージ（Copilot Language Server、Sourcery）、(b) LSP ではない独自 JSON-RPC（Cody agent、Continue core binary）、(c) 言語ごとの実行系と共通プロトコル（Jupyter kernel、nREPL）。
2. **最も近い前例の Wallaby（行内に実行時の値を出す）は、拡張のあるエディタにだけ行内表示を出し、Neovim・Vim・Zed・Emacs にはブラウザ UI を渡している。** Neovim の行内表示はしていない。
3. **LSP を選ばなかった Cody と Continue は、理由を文書に書いていない。** 実装上の理由（TS のコアを JetBrains・Neovim から使う）は読み取れるが、「LSP では足りない」という記述は無い。
4. **「ファイルを開いたら出る、事前計算なし、キャッシュ可」に合う前例は、行ごとの LLM 解説では見つからなかった。** 事前計算型（DeepWiki、Code Wiki）は要件に反する。開いたファイルだけを対象にする前例は Sourcery（診断）と GitLens（現在行のみ）。
5. **配布の運用負荷の前例はある。** copilot.lua は Node 同梱をやめ、75〜110 MB の単一バイナリをダウンロードして SHA-256 で検証し、バージョンで鍵づけして置く方式に既定を切り替えた。

## 根拠

### バックエンドの共有方式

| 製品 | 共有方式 | 中核言語 | 出典 |
|---|---|---|---|
| Copilot | LSP + 多数の独自メッセージ。各 OS の単一バイナリ。ACP も話す | TS 由来、ネイティブ化済み | https://github.com/github/copilot-language-server-release [直] |
| Cody | 独自 JSON-RPC。JetBrains と Neovim が使う | TypeScript（Node 実行） | https://github.com/sourcegraph/cody-public-snapshot/blob/main/agent/README.md [直] |
| Continue | 独自メッセージ。TS の core を `pkg` で OS 別バイナリ化。JetBrains は子プロセス、VS Code は同一プロセス | TypeScript | https://github.com/continuedev/continue/blob/main/binary/README.md [直] |
| Windsurf | 独自の `language_server` バイナリ | 未確認 | https://github.com/Exafunction/windsurf.nvim [直] |
| Augment | Node の `dist/server.js`。要 Node | JS | https://github.com/augmentcode/augment.vim [直] |
| Sourcery | `sourcery` CLI が言語サーバ。VS Code と Zed（第三者）が同じ CLI を呼ぶ | Python [推] | https://github.com/sourcery-ai/sourcery-vscode [直] |
| Wallaby | 拡張（VS Code、JetBrains、VS、Sublime）と Standalone Mode（ブラウザ） | Node | https://wallabyjs.com/docs/integration/overview/ [要約] |
| CodeTour | `.tour` JSON を共有。Neovim 版（1★）と JetBrains 版（49★）は別実装 | TS | https://github.com/microsoft/codetour [直] |
| Jupyter / nREPL | 言語非依存のメッセージ仕様。言語ごとの実行系 | — | [要約] |

- Copilot LS「attempts to follow the LSP spec as closely as possible, but many custom messages are employed」。独自: `textDocument/didFocus`、`textDocument/copilotInlineEdit` など [直]
- Cody agent「There's no tool to automatically generate bindings for the Cody agent protocol. Currently, clients have to manually write bindings」「The protocol is subject to breaking changes without notice.」[直]。LSP のライフサイクル（起動、再起動、進捗、キャンセル）を各クライアントが自前実装している。
- Continue はネイティブ依存（sqlite3、lancedb）を「need to download for each platform manually」[直]。配布の手間の実例。
- Mintlify の IntelliJ 版と Neovim 非公式版は archived [直]

### 行内表示の方式

| 製品 | VS Code | Neovim |
|---|---|---|
| Copilot | inline completion API（LSP 3.18 `inlineCompletion`） | copilot.lua / 本体の `vim.lsp.inline_completion` |
| Wallaby / Quokka | 行脇のゲージ・行内値 | 無し（ブラウザ） |
| nvim-dap-virtual-text | — | extmark。変数の発見は tree-sitter の `locals.scm` |
| molten-nvim（1,234★） | — | Jupyter kernel の出力を virtual text で。Python の remote plugin |
| Continue | CodeLens | — |
| GitLens | 現在行の行末注釈 | — |

- LSP 3.17 の `textDocument/inlineValue` はデバッガ用で、Neovim の `lsp.txt` に記述が無い [直]。実行結果や解説を LSP の標準機能で運ぶ前例は無い。

### 多言語の構造の取り方

- avante.nvim: 自前の Rust crate `avante-repo-map` に tree-sitter 文法を直書き、17 言語に固定 https://github.com/yetone/avante.nvim/blob/main/crates/avante-repo-map/Cargo.toml [直]
- Continue: tree-sitter の wasm を core に同梱 [直]
- Cody: autocomplete に「LSP-light context」（#1506、#3690）[直、タイトルのみ]
- nvim-dap-virtual-text: エディタ側の tree-sitter クエリに依存。
- molten / Jupyter: 構造は取らず、言語ごとの実行系に任せる。

### 測定値

- Copilot: 平均応答 200 ms 未満（補完）https://infoq.com/presentations/github-copilot/ [要約]。Cursor Tab: 260 ms（補完）[要約]。
- 解説・解析系の初回表示時間の公表値は、どの製品にも無い。
- DeepWiki と Code Wiki はリポジトリ全体を事前処理する設計 [要約]。

### リポジトリの状態（2026-10-03 [直]）

| リポジトリ | ★ | 状態 |
|---|---|---|
| copilot.vim / copilot.lua | 11,686 / 4,101 | 活発 |
| continuedev/continue | 36,090 | 活発 |
| codecompanion.nvim | 6,886 | 活発。全て Lua、同一プロセス |
| avante.nvim | 18,173 | 活発 |
| molten-nvim | 1,234 | 活発 |
| cody-public-snapshot | 3,803 | archived（2025-08） |
| sg.nvim | 778 | 「not actively being maintained」 |
| tabnine-nvim | 415 | archived |
| supermaven-nvim | 1,458 | 事業終了（2025-11） |
| mintlify/writer | 3,130 | deprecated（2026-06） |
| LightTable | 11,687 | archived |

### ファイルを開いた時点で何が起きるか

| 製品 | open 時の挙動 | キャッシュ（場所・鍵） | 開いている範囲だけ |
|---|---|---|---|
| GitLens | 現在行だけ。250 ms の debounce、CancellationToken で中断 | 内部（鍵は未確認） | 全ファイルを走査しない |
| Error Lens | 診断の到着順に装飾。`delay` 既定 0 | 無し | 開いたファイルの診断のみ |
| Copilot | 起動が遅いので lazy load 推奨。`didFocus` でフォーカスした文書を通知。`debounce = 15` ms | クライアント側に補完をキャッシュ。バイナリは `stdpath("data")`、鍵はバージョン・OS・SHA-256 | 開いた文書のみ |
| Cody | 補完は `RequestManager` の LRU | メモリ内 LRU、鍵はプレフィックス | 開いたファイルの周辺 |
| Continue | `AutocompleteLruCache` 容量 1000、30 秒ごとに SQLite へ。`openedFilesLruCache` は開いたファイル 20 件 | SQLite、鍵はプレフィックス | 開いたファイル 20 件を文脈に |
| Quokka | 自動では走らない。開始は手動か設定 | 文書なし | 開始したファイルのみ |
| Wallaby | 影響を受けるテストだけを実行。「real-time result streaming」 | 「cached execution」（詳細なし） | 影響分のみ |
| Sourcery | 「review all of the Python, JavaScript, and TypeScript files you have open」 | 文書なし | 開いたファイルのみ |
| 行ごとに LLM 解説を書く拡張（ExplainThisCode 27★ など） | 選択範囲をコメントとして挿入。2023〜2024 で停止 | — | — |

読み取れること:

- 確認できたキャッシュ鍵は「リクエスト内容（プレフィックス）」と「バイナリのバージョン・OS・SHA-256」だけ。ファイルの内容ハッシュを鍵にした解説結果のキャッシュは前例なし。
- 開いた分だけ先読みするのは Sourcery、Copilot の `didFocus`、Continue の 20 件、GitLens の現在行。全リポジトリを走査するのは DeepWiki 系だけ。
- 補完系は「1 件ずつ出す + debounce（15〜500 ms）+ キャンセル + プレフィックスキャッシュ」。数秒〜数十秒かかる生成を可視範囲優先やチャンクごとの装飾更新で見せる公開実装は見つからなかった。

### 否定側の証拠

- copilot.lua #667「9GB ram usage with two neovim instances open」、#688「Sqlite jumping up to 100% usage」（open）、#739 社内 Windows 機でバイナリのダウンロード失敗 [直]。配布するツールのバイナリ取得経路の実害例。
- Cody #5329「High CPU Usage」、Continue #9004、Error Lens #242 [直]

## 注意点

- 要約経由で原文を見ていないもの: Wallaby、Quokka、Jupyter、nREPL、ACP、DeepWiki の数字。
- Windsurf のプロトコル、Cody が tree-sitter を使う範囲、GitLens の blame キャッシュの鍵は未確認。
- Cody の撤退と sg.nvim の保守停止は事業判断で、設計の評価には使えない。

## 前例が見つからなかったもの

- ファイルを開いた時に行ごとの LLM 解説を生成して行末に出す製品または OSS。
- 実行結果を根拠にした解説を Neovim と VS Code の両方に同一バックエンドで出す製品。
- Cody・Continue が「LSP ではなく独自 JSON-RPC を選んだ理由」を述べた文書。
- 解説・解析系の「開いてから最初の注釈までの時間」の公表値。
- ファイルの内容ハッシュを鍵にした解説結果のキャッシュの公開実装。
