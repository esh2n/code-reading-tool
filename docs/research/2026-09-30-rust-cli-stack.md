# Rust CLI の技術選定（Go テスト実行 + 出典付き説明ページ）

確認日: 2026-09-30。凡例: [直接] = 一次ソース・API・手元実行。[要約] = WebFetch の要約経由（原文未確認）。[推測] = 推論。

## 答え

1. **Rust CLI の型**: clap（derive）+ anyhow（バイナリ）+ thiserror（呼び出し側が分岐する型付きエラーだけ）+ serde / serde_json / toml + insta + assert_cmd。サブプロセスは `std::process::Command` で足りる。tokio は不要。テンプレートは minijinja（実行時）が最短、askama（コンパイル時）は堅いが摩擦がある。Edition 2024 は Rust 1.85.0 で安定化済み。最新安定は 1.98.1。
2. **`go test -json`**: 1 行 1 JSON の TestEvent。`-race` の DATA RACE は専用 Action ではなく通常の `output` イベント（`Test` 付き）として流れ、その後そのテストが `fail` になる。Go 1.24 以降、ビルド失敗は `build-output` / `build-fail`（`ImportPath` キー、`Time` 無し）が別スキーマで混ざる。保守された Rust の parser crate は無く、serde で自前実装する。
3. **Wasm/WASI**: コアの実行役には無関係。WASI 0.2/0.3 にホストのプロセス起動は見当たらない。関係するのはブラウザ内ビューア、Zellij プラグイン、信頼できないコードの隔離をやるときだけ。
4. **LazyVim**: `lang.rust` extra は rustaceanvim + crates.nvim + codelldb。rust-analyzer は mason でなく PATH のものを使う。rustaceanvim 作者は mason 版を非推奨。
5. **先行事例**: 「シナリオを実行し、説明の各文を実行結果に結びつけ、出典の無い主張を拒否する」ツールは見つからなかった。

## 根拠

### 1. Rust CLI crates（crates.io API [直接]、2026-09-30）

| crate | 最新 | 直近DL | 更新日 |
|---|---|---|---|
| clap | 4.6.7 | 2.4 億 | 2026-09-14 |
| thiserror | 2.0.21 | 3.9 億 | 2026-09-23 |
| anyhow | 1.0.104 | 2.2 億 | 2026-07-18 |
| color-eyre | 0.6.5 | 1,378 万 | 2025-05-30（リポジトリ archived） |
| miette | 7.6.0 | 1,892 万 | 2025-04-27 |
| serde / serde_json | 1.0.229 / 1.0.151 | 3.3 億 | 2026-07 |
| toml | 1.1.6 | 2.3 億 | 2026-09-10 |
| askama | 0.16.1 | 1,280 万 | 2026-09-04 |
| minijinja | 2.24.0 | 1,161 万 | 2026-09-23 |
| tera | 2.4.0 | 615 万 | 2026-09-11 |
| maud | 0.27.0 | 230 万 | 2025-02-02 |
| insta | 1.48.0 | 2,876 万 | 2026-06-11 |
| assert_cmd | 2.2.2 | 1,826 万 | 2026-05-11 |
| proptest | 1.11.0 | 5,090 万 | 2026-03-24 |
| tokio | 1.53.1 | 2.4 億 | 2026-07-20 |

- clap: blessed.rs「Ergonomic, battle-tested, includes the kitchen sink, and is fast at runtime. However compile times can be slow」 https://blessed.rs/crates [要約]。Rust CLI Book も clap derive https://rust-cli.github.io/book/tutorial/errors.html [要約]
- エラー: blessed.rs はアプリに anyhow / color-eyre、ライブラリに thiserror。color-eyre は GitHub で archived [直接]。
- テンプレート: askama は「type-safe compiler for Jinja-like templates」 https://docs.rs/askama/latest/askama/ [要約]。minijinja は「entirely free of dependencies」 https://github.com/mitsuhiko/minijinja [要約]。「出典なしの文を不可能にする」のはテンプレートではなく Rust の型（`Claim { text, evidence }`）でやる [推測]。
- テスト: insta はレンダリング済み Markdown/HTML の回帰確認に合う https://github.com/mitsuhiko/insta [要約]
- サブプロセス: std docs「If you pipe both stdout and stderr and don't read from them promptly, the child process can block ... causing a deadlock」。`output()` は両方を取り込む https://doc.rust-lang.org/std/process/struct.Command.html [要約]
- tokio「designed for IO-bound applications where each individual task spends most of its time waiting for IO」 https://tokio.rs/tokio/tutorial [要約]
- Edition 2024 は 1.85.0 で安定化 https://doc.rust-lang.org/edition-guide/rust-2024/index.html [要約]。最新安定 1.98.1 https://raw.githubusercontent.com/rust-lang/rust/master/RELEASES.md [直接]

### 2. `go test -json`

- TestEvent のフィールドと Action 値 https://pkg.go.dev/cmd/test2json [要約]。ソースには `Key` / `Value` / `Path` も追加されている https://raw.githubusercontent.com/golang/go/master/src/cmd/internal/test2json/test2json.go [直接]
- Go 1.24「go test -json now reports build output and failures in JSON, interleaved with test result JSON. These are distinguished by new Action types ... GODEBUG setting gotestjsonbuildtext=1」 https://go.dev/doc/go1.24 [直接]
- `go help buildjson`: BuildEvent は `ImportPath` / `Action`（`build-output` / `build-fail`）/ `Output`。ImportPath は TestEvent.Package と一致しない。突き合わせは `FailedBuild` 経由 [直接、Go 1.26.2]
- `-race` の実測 [直接、Go 1.26.2]: `WARNING: DATA RACE` から `==================` までの全行が `{"Action":"output","Test":"TestRace",...}`。続いて `testing.go:NNNN: race detected during execution of test`、`Action:"fail"`。検出は「fail したテストの output に `WARNING: DATA RACE` がある」という文字列一致になる。
- 構文エラー時は `build-output` / `build-fail` の後、`FAIL x [setup failed]` と `{"Action":"fail","FailedBuild":"x.test"}`。行の形が 2 種類ある前提でパースする。
- Rust の parser crate は無い（gotest 0.1.1 は 2023-07 以降更新無し）[直接]。Go 側の参考: gotestsum、tparse、go-junit-report。

### 3. Wasm/WASI

- WASI README「WASI 0.3 (Preview 3) is the current preview」 https://raw.githubusercontent.com/WebAssembly/WASI/main/README.md [直接]。wasi.dev/roadmap は 0.3.0 が 2026-06-11 に出たとする [要約]
- wasm32-wasip2 は Tier 2、std を完全サポート [直接]
- ホストのプロセス起動インターフェースの記述は見つからなかった（不在の証明ではない）。
- `go test` はホストの `go` を起動して走るので、コアを wasm にする理由が無い。

### 4. LazyVim の Rust 設定

- extra の中身 https://raw.githubusercontent.com/LazyVim/LazyVim/main/lua/lazyvim/plugins/extras/lang/rust.lua [直接]: crates.nvim、treesitter `rust` / `ron`、mason に codelldb、rustaceanvim。lspconfig 側の `rust_analyzer` は無効（rustaceanvim が担当）。既定で保存時に cargo check が走る。Rust ファイルでは `<leader>cR` が Code Action、`<leader>dr` が Debuggables。
- rust-analyzer が PATH に無いと「rust-analyzer not found in PATH」と通知する。
- rustaceanvim README「I strongly recommend against using rust-analyzer managed by mason.nvim, as version mismatches ... will lead to subtle issues.」 https://github.com/mrcjkb/rustaceanvim [直接]
- 既定の LSP キー https://raw.githubusercontent.com/LazyVim/LazyVim/main/lua/lazyvim/plugins/lsp/init.lua [直接]: `gd` 定義、`gr` 参照、`gI` 実装、`gy` 型定義、`gD` 宣言、`K` ホバー、`gK` シグネチャ、`<leader>ca` コードアクション、`<leader>cr` リネーム、`<leader>cc` Codelens、`]]` / `[[` 次/前の参照。

### 5. 先行事例

- `gh search repos` を 7 種の文言で実行して該当 0 件。近隣は gotestsum / tparse / go-junit-report（表示のみ）、sqz / rtk（エージェント向けの出力要約）。

## 注意点

- 要約経由の項目は逐語引用が要約の範囲に限られる。
- `-race` の実測は単純な 1 ケースのみ。`t.Parallel()`、サブテスト、`TestMain`、テスト終了後のレースは未確認。
- WASI にプロセス起動が無いことは間接根拠のみ。
- LazyVim の診断ジャンプ・シンボル検索のキーはこの調査では未確認。
- 実践者レンズは薄い（blessed.rs、Rust CLI Book、rustaceanvim 作者の README のみ）。コンパイル時間などの測定値は取っていない。
