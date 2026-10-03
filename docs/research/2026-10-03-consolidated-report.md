# 調査報告（統合版）：エディタ内でコードの「動き」を解説する道具の技術選定

作成日: 2026-10-03。8 本の調査記録（`docs/research/` の 2026-09-30 の 3 本と 2026-10-03 の 5 本）を 1 つにまとめたもの。個々の記録には URL 付きの根拠があり、この報告は結論と主要な根拠だけを載せる。

証拠の印: **[直]** = 一次ソース（公式文書・ソースコード・GitHub API・crates.io/npm の API）を直接読んだ。**[要約]** = Web 取得の要約を通した（原文の言い換えの可能性がある）。**[測]** = この調査で実際に測った（Apple Silicon の Mac 1 台）。**[推]** = 証拠からの推論で、未検証。

---

## 0. 用語

この報告で使う用語。初出でも説明するが、まとめて先に置く。

| 用語 | 意味 |
|---|---|
| エディタ拡張（プラグイン） | エディタの中で動く追加プログラム。VS Code では TypeScript で書く「拡張（extension）」、Neovim では Lua で書く「プラグイン」。 |
| バックエンド | エディタの外で動く別プロセスのプログラム。エディタ拡張から呼ばれ、重い仕事（解析、LLM 呼び出し）を担う。この報告では「エンジン」とも書く。 |
| JSON-RPC | プログラム同士が「関数名と引数」を JSON で送り合う通信の形式。 |
| stdio | 標準入出力。子プロセスとして起動したプログラムと、パイプで文字列をやり取りする経路。JSON-RPC over stdio は「子プロセスと stdio 経由で JSON-RPC を話す」こと。 |
| LSP（Language Server Protocol） | エディタと「言語サーバー」（gopls、rust-analyzer、pyright など）が話すための、JSON-RPC 上の標準仕様。定義ジャンプ、補完、診断（エラーの下線）、hover（マウスを乗せたときの説明窓）などの要求と応答の形が決まっている。VS Code と Neovim は、この仕様で話す「クライアント」を本体に持つ。 |
| 独自メソッド（カスタムリクエスト） | LSP の仕様に無い要求を、同じ JSON-RPC の経路に足すこと。Copilot や rust-analyzer がやっている。 |
| call hierarchy | LSP の機能の 1 つ。ある関数を「誰が呼んでいるか（incoming）」「何を呼んでいるか（outgoing）」を返す。 |
| tree-sitter | プログラミング言語の構文解析ライブラリ。言語ごとの「文法」を読み込むと、コードを構文木（関数、引数、呼び出し、などの入れ子の木）にする。多言語対応で、Neovim 本体、GitHub、多くの AI コーディング製品が使う。 |
| tags クエリ | tree-sitter の構文木から「定義（関数、クラス）」と「参照（呼び出し）」の位置を抜き出すための、言語ごとの問い合わせファイル（`.scm`）。 |
| 構文（syntax）と型（types） | tree-sitter が返すのは構文（どこが関数か）まで。「この呼び出しが実際にどの関数へ飛ぶか」を確定するには型の情報が要り、それは各言語のコンパイラや言語サーバーしか持たない。 |
| virtual text / extmark | Neovim で、ファイルの中身を変えずに画面だけに文字を足す仕組み。extmark は位置の目印で、そこに virtual text（仮想の文字列）を付けられる。行末に付けるのが `eol`。 |
| decoration | VS Code で、同じく画面だけに装飾（行末の文字、下線、色）を足す仕組み。 |
| inlay hint | LSP の標準機能で、型や引数名を行の途中に薄く表示するもの。 |
| wasm（WebAssembly）/ WASI | wasm はブラウザなどで動く中間コードの形式。WASI は wasm がファイルやプロセスなど OS の機能を使うための標準。Rust や Go を wasm に変換して、別の環境で動かせる。 |
| cdylib / FFI | cdylib は Rust などで作った共有ライブラリ（.so / .dylib）。FFI は別の言語（ここでは Lua）からそれを直接呼ぶ仕組み。同じプロセスの中で動くので速いが、落ちるとエディタごと落ちる。 |
| LuaJIT | Neovim が使う Lua の実行系。FFI 機能を持つ。 |
| 構造化出力（Structured Outputs） | LLM に「この JSON Schema に従った JSON で返せ」と強制する機能。 |
| キャッシュの鍵 | 保存した結果を後で引き出すための識別子。鍵が同じなら同じ結果を返す。 |
| race（データ競合） | 複数の処理が同時に同じメモリに触り、片方が書き込むときに起きる不具合。Go の race detector は実行中にこれを検出する道具。 |

---

## 1. 調査の問いと範囲

### 要件（所有者と揃えたもの）

- 対象言語は多岐にわたる。固定の一覧ではない。言語を足すのに新しいコードを書きたくない。
- 解説の中身は「各行が何をするか」「具体的な値（境界値を含む）でどう動くか」「同時に来たら何が起きうるか」。**推論であり、実際には実行しない。**
- ファイルを開いたときに出る。リポジトリ全体の事前計算はしない。キャッシュは可。
- 出力先は VS Code、Neovim、静的 HTML。
- 配布する。他の人の環境で動く。

### 調べた問い

| 記号 | 問い | 記録 |
|---|---|---|
| A | 似た製品はどう作られているか。複数エディタ・複数言語をどう扱い、開いたときに何をしているか | similar-products-architecture |
| B | 多言語のコード構造（関数の範囲、呼び出し元）をどこから取るのが標準か | multi-language-structure |
| C | Neovim の拡張をどう作ると速いか（Lua、FFI、外部プロセス、wasm） | neovim-rendering-and-compute、nvim-plugin-architecture |
| D | VS Code の拡張をどう作ると速いか（描画、wasm、LSP か直接 API か） | vscode-extension-tech、editor-agnostic-backend |
| E | バックエンドを何の言語で書くか | backend-language、rust-cli-stack |
| F | 実行して裏付けるとしたら何が要るか（要件から外れたが、情報として残す） | rust-cli-stack、nvim-plugin-architecture、multi-language-structure |
| G | LLM の推論はどれくらい当たるか | draft.md（元の草案の調査） |

### 証拠の集め方

公式文書、名前の知れた実践者、測定値、公開リポジトリの状態（スター数、最終更新）の 4 つの角度で集め、否定側の証拠（放棄、撤退、不具合報告）も同じ力で集めた。issue トラッカーは否定に偏り、ベンダーの文書は肯定に偏ることを前提に読む。

---

## 2. 問い A：似た製品はどう作られているか

### 2.1 複数エディタに対応する製品は「バックエンドをエディタの外に出す」形に収束している

| 製品 | バックエンドの共有方式 | バックエンドの言語 | 出典 |
|---|---|---|---|
| GitHub Copilot | LSP + 多数の独自メッセージ。各 OS の単一バイナリ | TypeScript 由来、ネイティブ化済み | [直] github/copilot-language-server-release |
| Sourcegraph Cody | 独自の JSON-RPC（LSP ではない）。JetBrains と Neovim が同じプロセスを使う | TypeScript（Node で実行） | [直] cody-public-snapshot の agent/README |
| Continue | 独自メッセージ。TypeScript のコアを OS 別のバイナリにして、JetBrains は子プロセスとして起動 | TypeScript | [直] continuedev/continue の binary/README |
| Sourcery | `sourcery` CLI が言語サーバー。VS Code と Zed が同じ CLI を呼ぶ | Python [推] | [直] sourcery-vscode |
| Windsurf | 独自の `language_server` バイナリ | 未確認 | [直] windsurf.nvim |
| Wallaby（行内に実行時の値を出す） | 拡張のあるエディタ（VS Code、JetBrains、Visual Studio、Sublime）だけ。**Neovim・Vim・Zed・Emacs にはブラウザ UI** | Node | [要約] wallabyjs.com |
| CodeTour | `.tour` という JSON ファイルを共有し、読む側はエディタごとに別実装 | TypeScript | [直] microsoft/codetour |

Copilot の README の原文: 「The Copilot Language Server attempts to follow the LSP spec as closely as possible, but many custom messages are employed to support the unique features of Copilot.」[直]

Cody の agent の README の原文: 「There's no tool to automatically generate bindings for the Cody agent protocol. Currently, clients have to manually write bindings」「The protocol is subject to breaking changes without notice.」[直] — 独自方式にすると、エディタごとに接続のコードを手書きすることになる、という実例。

Continue はネイティブ依存（sqlite3、lancedb）を「need to download for each platform manually」と書いている [直]。配布の手間の実例。

### 2.2 LSP を使わなかった製品は理由を書いていない

Cody と Continue は LSP ではなく独自の JSON-RPC を選んだが、「LSP では足りないから」と書いた文書は見つからなかった。TypeScript で書いたコアを JetBrains や Neovim から使うため、という実装上の事情は読み取れる [推]。

### 2.3 「ファイルを開いたときに行ごとの LLM 解説を出す」製品は見つからなかった

近い用途の製品が開いたときに何をしているか:

| 製品 | 開いたときの挙動 | 何をどこにキャッシュするか | 対象の範囲 |
|---|---|---|---|
| GitLens（行末に git blame） | **カーソル行だけ**に注釈。250 ms の待ち（debounce）の後に更新。途中でキャンセルできる | 内部キャッシュ（鍵は未確認） | 全ファイルは走査しない |
| Error Lens（行末にエラー） | 言語サーバーから届いた診断を、届いた順に描く | 無し | 開いたファイルだけ |
| Copilot | 起動が遅いので遅延読み込みを推奨。`didFocus` で「今見ている文書」をサーバーへ通知 | 補完の結果をクライアント側に保持。バイナリは版・OS・SHA-256 で管理 | 開いた文書だけ |
| Continue | 補完の結果を SQLite に保存（鍵は入力のプレフィックス）。開いたファイル 20 件を文脈に使う | SQLite | 開いたファイル 20 件 |
| Sourcery | 「review all of the Python, JavaScript, and TypeScript files you have open」[直] | 文書なし | 開いたファイルだけ |
| DeepWiki、Google Code Wiki | リポジトリ全体を事前に処理する | — | 全リポジトリ（要件に反する） |

読み取れること:

- 「開いているファイルだけを対象にする」のが、事前計算型（DeepWiki）以外の共通の範囲。
- キャッシュの鍵として確認できたのは「リクエストの内容（プレフィックス）」と「バイナリの版」だけ。**ファイルや関数の内容のハッシュを鍵にした解説結果のキャッシュは、前例が無い。**
- 数秒〜数十秒かかる生成を、画面に見えている範囲から順に、できた部分から描く実装は、公開されたものの中に見つからなかった。

### 2.4 配布の運用（否定側の証拠を含む）

- copilot.lua（Neovim 用、4,101★）は 2026-08 に、Node.js 同梱をやめて「75〜110 MB の単一バイナリをダウンロードし SHA-256 で検証する」方式を既定にした [直]。Node を利用者に要求する方式から離れた実例。
- 同じ copilot.lua に、社内 Windows 機でバイナリのダウンロードに失敗した報告（#739）、2 つの Neovim で 9 GB のメモリを使った報告（#667）がある [直]。配布するツールの実害の例。
- 撤退した製品: Cody の無料版と Pro は 2025-07 に終了、公開リポジトリは archived。Supermaven は 2025-11 に事業終了。Mintlify Writer は 2026-06 に deprecated。LightTable は archived。いずれも事業判断で、設計の失敗を示すものではない。

### 2.5 行内表示の方式（製品ごと）

| 製品 | VS Code での描き方 | Neovim での描き方 |
|---|---|---|
| Copilot | ゴースト文字（inline completion） | 本体の `vim.lsp.inline_completion` |
| Wallaby / Quokka | 行脇のゲージと行内の値 | 無し（ブラウザ） |
| nvim-dap-virtual-text（デバッガの変数の値） | — | extmark の virtual text。変数の発見は tree-sitter の `locals.scm` |
| Continue | CodeLens（行の上に別行で出る） | — |
| GitLens | decoration（行末） | — |

LSP の標準には「解説や実行結果を行末に出す」機能は無い。`textDocument/inlineValue` はデバッガ用で、Neovim は対応していない [直]。よって独自メソッドは避けられない。

---

## 3. 問い B：多言語のコード構造をどこから取るか

必要なのは、対象の関数について「本体の範囲」「外側の型（メソッドならそのクラス）」「直接の呼び出し元と呼び出し先」を、言語を問わず取ること。候補は 4 つ。

### 3.1 tree-sitter（構文のみ）

- 対応言語: tree-sitter-language-pack が 371 言語の文法を束ねる。ただし **tags クエリ（定義と参照を抜き出す問い合わせ）があるのは 97 言語** [直]。文法があることと、関数と呼び出しを取れることは別。
- 公式のバインディング（各言語からの使い方）: C#、Go、Haskell、Java、JavaScript（Node と wasm）、Kotlin、Python、Rust、Swift、Zig [要約]。Rust は本体のクレートが第一級。**Go 用（go-tree-sitter、298★）は 2025-11 で更新が止まっている** [直]。Zig 用は 125★ [直]。
- 精度の限界: tags の「参照」は**名前の一致**で取る。同名の別の関数も拾う（取りすぎる）。型の情報は無い。
- aider（LLM に渡す「リポジトリの地図」を tree-sitter で作る、代表的な実例）の実装を直接読んだ結果: 参照は名前の一致で、足りない分は pygments（字句解析）で補っている [直]。さらに、標準の tags クエリに「呼び出し（`@reference.call`）」が無い言語がある: **C、C++、C#、Swift、Zig は 0 件** [直]。これらの言語では呼び出し元を tags からは取れない。
- 1 言語を足すコスト: 文法 1 つと tags クエリ 1 本。

### 3.2 エディタの言語サーバー（LSP の call hierarchy）

- 型を解決した正確な呼び出し元が取れる。ただし**言語サーバーごとに有無が違う**（ソースを直接確認 [直]）:

| 言語サーバー | call hierarchy |
|---|---|
| gopls、rust-analyzer、pyright、clangd | あり |
| typescript-language-server | あり（条件つき） |
| jdtls（Java） | 型の import はあり、提供の最終確認は未 |
| ruby-lsp | **無し**。要望は「型検査なしには完全な階層は不可能」として not_planned で閉鎖 |
| Sorbet（Ruby） | **無し**。要望は open のまま |
| zls（Zig） | 記述なし |

- つまり、LSP に頼ると「言語によって品質が変わる道具」になる。動的言語では構造的に無理。
- Serena（LSP を使って 40 言語超を謳う MCP サーバー、29,952★）ですら、機能一覧に call hierarchy を載せていない。利用者のエディタの言語サーバーを再利用するのではなく、自分で起動し直す設計。Java の言語サーバーの資源暴走（#1944）、起動が終わらない（#937）などの運用の不満がある [直]。
- Continue は tree-sitter と IDE の定義ジャンプの両方を使い、**遅ければ LSP の結果を捨てる**（`racePromise`）[直]。LSP を「あれば使う任意の経路」として扱っている。

### 3.3 SCIP / stack-graphs / CodeQL（索引を作る方式）

- SCIP（Sourcegraph の索引形式）: 索引を作るプログラムは言語ごとに別で、約 9 系統（Java、TypeScript、C/C++、Ruby、Python、.NET、Dart、PHP、Go）。多言語に足りない [直]。
- stack-graphs（GitHub が作った、tree-sitter 上で名前解決をする仕組み）: **archived**。README に「This repository is no longer supported or updated by GitHub」[直]。業界の本命だった方式が撤退した。
- CodeQL: 12 言語だが、CLI のライセンスが「私有コードの自動解析」を禁じる（有償契約を除く）[直]。個人の道具の土台にならない。

### 3.4 LLM に文脈を渡す製品が実際に使っているもの

| 製品 | 使っているもの |
|---|---|
| aider | tree-sitter の定義と参照（名前一致）+ グラフのランク付け |
| Continue | tree-sitter（wasm 版を同梱）+ IDE の定義ジャンプ（遅ければ捨てる） |
| avante.nvim | 自前の Rust クレートに tree-sitter の文法を直接組み込み、17 言語に固定 |
| Cody | 検索（BM25）。embeddings は撤退 |
| Cursor | 構文の塊ごとの embeddings。方式の詳細は非公開 |

「関数 + 呼び出し元」を多言語で正面から扱っているのは aider だけで、方式は名前一致。精度を数値で述べた一次情報はどの製品にも無い。

### 3.5 結論

- 多言語を最初から扱うなら、**tree-sitter の文法をバックエンドに同梱する**のが実例（Continue、avante.nvim）のある形。エディタの言語サーバーや、利用者が入れた tree-sitter のパーサに頼ると、言語と環境で結果が変わる。
- 呼び出し元は「候補 + 確からしさの印（名前一致か、言語サーバーで解決済みか）」として扱う。確定した呼び出し元が要る場面では、エディタの call hierarchy を「あれば使う」。

---

## 4. 問い C：Neovim の拡張をどう作ると速いか

### 4.1 描画の性能 [測]（nvim 0.12.5、1 機種）

行ごとに `nvim_buf_set_extmark`（extmark を 1 つ置く API）を Lua から呼ぶ、いちばん素朴な方法で測った。各行に 80 字弱の文字列を行末（`eol`）に付けた。

| 行数 | 全行に付けるのにかかった時間 | スクロール時の再描画 |
|---|---|---|
| 200 | 0.2 ms | 0.02 ms |
| 2,000 | 2.3 ms | 0.05 ms |
| 20,000 | 24.6 ms | 0.06 ms |
| 100,000 | 124 ms | 0.06 ms |

- 1 件あたり 1.2 µs。件数に比例する。スクロールの再描画は、extmark が 0 件でも 10 万件でも変わらない。
- **結論: 数千行の行末注釈は、Neovim では描画の問題にならない。**
- 注意: 再描画の値は Neovim 内部の画面更新の時間で、端末に文字を送る時間は含まない。

外部のプロセスから Neovim の API を呼んで付ける場合（2,000 行）[測]: 1 行ずつ同期で呼ぶと 50.6 ms、1 回の呼び出しにまとめると 2.7〜5.0 ms。**まとめて 1 回で渡すのが約 20 倍速い。**

### 4.2 Neovim 本体とプラグインが採っている描画の型（ソースを直接確認 [直]）

| 型 | やり方 | 採っているもの |
|---|---|---|
| A | `nvim_set_decoration_provider` で、画面に見えている行だけにその都度付ける | 本体の inlay hint、semantic tokens、codelens。gitsigns、snacks.nvim、mini.diff |
| B | 待ち時間（100〜200 ms）を置いて、見えている範囲 + 余白に付ける | indent-blankline v3、render-markdown.nvim |
| C | 結果が届いた時点で全行に付ける | 本体の `vim.diagnostic`、nvim-dap-virtual-text |

- 編集で行がずれても注釈を追従させたいなら、消えない（永続の）extmark が要る。本体の semantic tokens のソースの原文: 「the buffer updates are not in sync with the list of semantic tokens. There's a delay between the buffer changing and when the LSP server can respond with updated tokens, and we don't want to "blink"」[直]。外部で作った解説を行に貼るこの道具に、そのまま当てはまる。
- 型 C（全行に一度に付ける）で 2,000 行が 2.3 ms なので、この道具には型 C で足りる。

### 4.3 Neovim 側で重い処理をどこでやるか

選択肢と実測:

| 選択肢 | 実測・前例 | 判断 |
|---|---|---|
| 純 Lua（LuaJIT） | 2,000 行分の JSON（313 KB）の読み取りが 0.69 ms [測] | 十分速い |
| Lua から FFI で Zig / Rust / C の共有ライブラリを呼ぶ | 前例は telescope-fzf-native（C）、telescope-zf-native（Zig、2024-09 で更新停止）、blink.cmp（Rust）。blink.cmp は 1 万件超の候補を毎回照合する処理で「fzf の約 6 倍」を主張。fzf-native には segfault の報告が複数、ビルド失敗が open のまま [直] | この道具の計算量（JSON を受けて行に貼る）には当てはまらない。クラッシュがエディタごと起きる |
| 外部プロセス（`vim.system` / `jobstart`） | プロセス起動 1.04 ms、常駐した子との 1 往復 0.007 ms、313 KB の往復 0.89 ms [測] | 十分速い。落ちてもエディタは無事 |
| wasm | **Neovim に wasm のプラグインを動かす仕組みが無い。** 本体の issue #23579「wasm (webassembly) plugins」は「This is just a tracking issue. Not planned.」[直]。試作が 2 件あるが本体に入っていない。Zellij や Zed は専用のランタイムを持つが、Neovim は持たない | 使えない |

JSON と msgpack（バイナリ形式）の比較 [測]: Neovim の Lua では JSON の方が 4〜5 倍速い。msgpack が要るのは、外部プロセスが Neovim の API を直接呼ぶときだけ。

### 4.4 Neovim 本体の方針 [直]

- プラグインは Lua が第一。登録不要で、置くだけで動く。
- 外部の解析ツールの入口は LSP。`vim.lsp.start()` は `cmd` に Lua の関数も受けるので、同じプロセスの中で LSP サーバーを動かすこともできる。
- 古い「remote plugin」の仕組みは、本体の issue で「too complex」とされ、簡素化が計画中。

### 4.5 Neovim の中の tree-sitter

- Neovim 本体が同梱するパーサは C、Diff、Lua、Markdown、Vimscript、Vimdoc、query の 7 種だけ [直]。他の言語は利用者が nvim-treesitter などで入れる。理由は「数百のパーサを同梱すると GB 級になる」[直]。
- プラグインから `vim.treesitter.get_parser` で使えるが、パーサが無い言語では nil が返る。
- 「関数の範囲」を言語横断で取るクエリ（`@function.outer`）は nvim-treesitter-textobjects が 52 言語分持つ [直]。
- 結論: Neovim の中の tree-sitter に頼ると、利用者の環境で結果が変わる。多言語で同じ結果を出すなら、バックエンド側に文法を持つ方が確実。

### 4.6 以前の記録（2026-09-30）からの補足

- Rust を cdylib にして Neovim に読み込ませる形（blink.cmp、avante.nvim）は、プレビルドの不一致や古い CPU で **Neovim ごと落ちる**報告が複数ある [直]。
- nvim-oxi（Rust から Neovim の C API を直接呼ぶ）は Neovim の版に強く結びつき、0.12.2 で panic する issue が open [直]。

---

## 5. 問い D：VS Code の拡張をどう作ると速いか

### 5.1 拡張の構造

- VS Code の拡張は、画面を描くプロセスとは別の「拡張ホスト」（Node.js のプロセス、またはブラウザ版では Web Worker）で動く [要約]。描画の命令は拡張ホストから描画側へ送られる。
- 行末に文字を出すには `createTextEditorDecorationType` で decoration を作り、`setDecorations` で範囲を渡す。

### 5.2 行ごとに違う文章を全行に付けると固まる（重要）

- VS Code の描画側のソースの原文: 「identify custom render options by a hash code over all keys and values」[直]。つまり、**行末の文字列（renderOptions）が違うごとに、CSS の型が 1 つ作られる。** 行ごとに違う解説を付けると、行数ぶん CSS の型ができる。
- 実測（microsoft/vscode の PR #337313、未マージ、報告者 1 人）: 約 11,000 件の診断を付けたファイルで、描画側が 66 秒止まった。うち 45 秒は CSS の撤去（二次の走査）。挿入は 36,000 ルールで 0.4 秒 [直]。
- Error Lens（行末にエラーを出す拡張、841★）は 2026-09-26 の 3.29.0 で「1 ファイルに 1,000 件超のときは、見えている行だけに描く」に直した。設定の説明の原文: 「VSCode creates a separate set of CSS rules for every distinct inline message, so a file with thousands of problems can freeze the editor for a long time.」[直]
- GitLens は最初から**カーソル行だけ**に行末注釈を出す [直]。
- **結論: VS Code では、画面に見えている行 ± 余白にだけ付け、スクロールに合わせて付け替える。** 長い全文は hover（マウスを乗せたときの説明窓）に回す。inlay hint は VS Code の既定で 1 行 43 文字で切られるので、解説には使えない（2026-09-30 の記録 [直]）。

### 5.3 VS Code の中で Rust などのコアを動かす方法

| 方法 | 事実 | 判断 |
|---|---|---|
| ネイティブ Node モジュール | Electron の ABI に合わせた再ビルドが要る | 子プロセスにすれば避けられる |
| wasm（`@vscode/wasm-wasi`） | 公式の仕組みはある（WASI Preview 1）。しかし **同じバイナリが wasmtime より 15 倍遅い**という報告があり、保守者は「I can't improve the performance of the WASM runtime that ships in Electron / Browser」と回答 [直]。公式も「Rust から VS Code の API を直接呼ぶ」案を async 対応が無いため見送り [直] | 不採用 |
| 子プロセスのバイナリ | Ruff（Python の linter、1,675★）の拡張が同梱バイナリを起動する形 [直]。Copilot も同じ | 採用 |

- ブラウザ版の VS Code（vscode.dev）では「Creating child processes or running executables is not possible」[直]。子プロセス型は動かない。対応するなら wasm 版（15 倍遅い）が要る。

### 5.4 LSP を使うか、拡張の API を直接使うか

- 公式の指針: LSP の利点は「サーバーを別の言語で書ける」「重い処理を別プロセスに逃がせる」の 2 つ。「they are not the only option」[要約]。
- LSP を使わない例: GitLens、Error Lens、coverage-gutters（いずれも拡張の API を直接使う）。LSP を使う例: Copilot の補完。
- LSP で表せない画面（行末の装飾、独自のビュー）は、サーバーから独自の通知を受けて、拡張側で描く。clangd の拡張がこの形の実例 [直]。

### 5.5 多言語の関数の範囲を VS Code 側で取る方法

- `vscode.executeDocumentSymbolProvider` で、入っている言語拡張のシンボル情報を使える。ただし**言語拡張が無いと `undefined` が返り、「空」と区別できない** [直]。
- anycode（Microsoft の、tree-sitter の wasm で構造を出す拡張）は 7 言語で、README 自身が「_inaccurately_ implements」と書く [直]。
- 結論: VS Code 側の仕組みに頼らず、バックエンドが tree-sitter で取る方が、言語と環境によらない。

---

## 6. 問い E：バックエンドを何の言語で書くか

### 6.1 起動時間と大きさ [測]（最小の LSP サーバー、hyperfine 50 回）

| 言語 | バイナリの大きさ | 起動から最初の応答まで | メモリ |
|---|---|---|---|
| Rust | 0.47 MB | 1.2 ms | 1.7 MB |
| Go | 2.5 MB | 1.7 ms | 4.4 MB |
| Bun の単一バイナリ（TypeScript） | 63 MB | 11.9 ms | 28 MB |
| Node（TypeScript） | 111 MB（Node 本体） | 19.3 ms | 43 MB |

Rust と Go の差はミリ秒以下で、体感に乗らない。TypeScript は配布物が大きい。

### 6.2 言語ごとの部品の有無

| 必要な部品 | Rust | Go | TypeScript |
|---|---|---|---|
| tree-sitter | 本体のクレート（第一級） | go-tree-sitter は 2025-11 で停止。CGO が要りクロスコンパイルが面倒 | web-tree-sitter（wasm）。Continue が使う |
| LSP サーバーのライブラリ | tower-lsp-server 0.23（2026-09）。Harper（16,000★）と typos-lsp が採用。本家の tower-lsp は 2023-08 で停止 | sourcegraph/jsonrpc2 + go.lsp.dev/protocol。多数派だが保守者が少ない。glsp は 15 か月停止 | vscode-languageserver（最も整っている） |
| OpenAI 互換の HTTP クライアント | reqwest + serde | 公式 SDK あり | 公式 SDK あり |
| 単一バイナリの配布 | 容易 | 容易 | Bun で 63 MB、Node SEA は「Active development」でネイティブ依存に制限 |
| Neovim の利用者への要求 | 無し | 無し | Node を要求するか、巨大バイナリ |

### 6.3 Zig について

tree-sitter の Zig 用バインディング（125★）はある。しかし、LSP サーバー・HTTP・JSON の各ライブラリで、実績のある組み合わせをこの調査では確認していない。Neovim 側で Zig を FFI で使う前例は 2 件で、どちらも軽い計算（4.3）。**Zig をバックエンドの本命にするなら、Zig だけの調査が別に要る。**

### 6.4 結論

多言語の構造を tree-sitter で取り、単一バイナリで配り、両エディタに LSP で話す、という条件では **Rust** が、部品の有無・配布・前例（Harper、typos-lsp）の 3 点で最も整っている。Go は tree-sitter のバインディングが止まっている点で落ち、TypeScript は配布で落ちる。

---

## 7. 「ファイルを開いたときに出る」をどう実現するか

### 7.1 物理的な制約

LLM に行ごとの解説を書かせる以上、生成には時間がかかる。200 行の関数に 1 行 15 語の解説を付けると、出力は約 3,000〜4,000 トークン。

| LLM の速さ | かかる時間 |
|---|---|
| 毎秒 50 トークン（手元のモデル） | 60〜80 秒 |
| 毎秒 150 トークン（速いクラウド） | 20〜30 秒 |

**初回に「開いた瞬間に全行」は不可能。** これは言語やエディタの技術では解決しない。

### 7.2 前例から取れる部品

- 開いているファイルだけを対象にする（Sourcery、Copilot の `didFocus`、Continue の 20 件）。
- 1 件ずつ出す、待ち時間（debounce）を置く、途中でキャンセルする（補完系の共通の慣行）。
- 結果を利用者のキャッシュ領域に保存する（Continue は SQLite、copilot.lua は `stdpath("data")`）。
- 編集に追従させるなら、Neovim は永続 extmark、VS Code は `rangeBehavior: ClosedClosed`。

### 7.3 前例が無い部分（設計で埋める）

- 解説結果を「関数本体のハッシュ」で鍵づけして保存する。前例は無いが、ファイル単位だと 1 行直すたびに全部無効になるので、関数単位の方が細かい。
- 見えている範囲の関数から順に生成し、関数 1 つ終わるごとに貼る。

---

## 8. 問い F：実行して裏付けるとしたら（要件から外れたが、情報として）

- Go なら、`go test -race -json` でデータ競合の報告と、シナリオごとに通った行（カバレッジ）が取れる。生成したテストはリポジトリに書かず `-overlay` で差し込める。テストバイナリを 1 回ビルドし（0.8 秒）、別プロセスで 20 回走らせると race を 20 回とも検出した（同じプロセス内の `-count=3` では 1 回だけ）[測]。
- 他の言語は、テストの書き方・実行・結果の読み取りが全部違い、言語ごとにアダプタを書くことになる。Rust のテストの JSON 出力は「experimental」で不安定。並行処理の検証が安く成立するのは Go だけ。
- 以上から、実行は「多言語」「開いたら出る」の 2 つの要件と衝突する。

---

## 9. 問い G：LLM の推論はどれくらい当たるか（元の草案の調査より）

- 実行結果を当てるベンチマーク（CRUXEval）: 最大 13 行の短い関数でも正答は 44〜63%。
- データ競合を見つけるベンチマーク（DRPBench、ICML 2026）: 最良のモデルで F1 75%、ほとんどは 60% 未満。
- 自動生成のレビューコメントを付けても重大な問題の検出は増えず、コメントに引きずられる傾向（アンカリング）が見られた（ICSE 2025、29 人）。
- 含意: 実行しない以上、解説の 4 割前後は間違っている前提で設計する。「構造から機械的に分かる事実」と「LLM の推論」を分けて見せ、推論には前提を付けることで、読む側が疑う場所を分かるようにする。

---

## 10. 結論の対応表

| 要件 | 選んだもの | 主な根拠 | 選ばなかったものが失うこと |
|---|---|---|---|
| 複数エディタ | バックエンドをエディタの外に出し、stdio の JSON-RPC で話す | 2.1 の収束 | エディタごとに全部書き直す |
| 接続の方式 | LSP の枠 + 独自メソッド（Copilot 型） | 両エディタにクライアントが組み込み。Cody の手書きの実例 | 独自方式は起動・再起動・進捗・キャンセル・診断・hover を自分で書く |
| 多言語 | tree-sitter の文法をバックエンドに同梱 | 3.5。エディタの LSP は言語で揃わない。Neovim のパーサは利用者次第 | 言語と環境で結果が変わる |
| バックエンドの言語 | Rust | 6.4 | Go は tree-sitter が止まっている。TS は配布が重い |
| Neovim 側 | Lua だけ。全行に一度に付ける | 4.1 の実測。4.3 の前例 | FFI はクラッシュの代償に見合う処理が無い。wasm は不可 |
| VS Code 側 | TypeScript。見えている行だけに付け、全文は hover | 5.2 | 全行に付けると固まる |
| ブラウザ版 VS Code | 対応しない | 5.3 | 対応するなら wasm（15 倍遅い） |
| 配布 | Neovim は Release のバイナリを取得して SHA-256 検証、VS Code は OS 別 VSIX に同梱 | 2.4、Ruff の例 | — |
| 開いたら出る | 構造の事実と保存済みの解説を即時に、新しい解説は見えている関数から順に | 7 | 初回を待つしかない |

---

## 11. 見つからなかった前例（全体）

- ファイルを開いた時に行ごとの LLM 解説を生成して行末に出す製品または OSS。
- 実行結果を根拠にした解説を Neovim と VS Code の両方に同じバックエンドで出す製品（Wallaby は Neovim には行内表示を出さない）。
- Cody と Continue が LSP ではなく独自 JSON-RPC を選んだ理由を書いた文書。
- 解説系ツールの「開いてから最初の注釈までの時間」の公表値。
- ファイルや関数の内容のハッシュを鍵にした解説結果のキャッシュの公開実装。
- tree-sitter の名前一致と LSP の call hierarchy の精度（再現率・適合率）を比べた測定。
- Neovim の extmark の件数と描画の遅延を測った公開資料（今回の実測のみ）。
- VS Code の decoration の件数別の公開ベンチマーク（PR #337313 の 1 例のみ）。
- Rust・Go・Zig の wasm コアを VS Code の拡張ホストで本番運用している製品。
- Zig をバックエンドにした LSP サーバーの実績（調査していない）。

---

## 12. 裁定事項

1. 接続の方式: LSP の枠 + 独自メソッドか、独自 JSON-RPC か。速度に差は無く、エディタ側に書く量の差。
2. バックエンドの言語: Rust か。Zig を本命にするなら追加調査。
3. キャッシュの鍵: 関数本体のハッシュ + モデル名 + プロンプトの版、でよいか。
4. 「構造から機械的に分かる事実」の層（LLM を使わず、開いた直後に出せる唯一の層）を作るか。
5. ブラウザ版 VS Code を捨ててよいか。
6. 同時実行の解説を推論のまま出すか。将来、Go など安く裏付けられる言語だけ実行を足す余地を残すか。
