# オンボーディング：Neovim と Rust で段階 1 を書く

分担：足場（Cargo の workspace、Go の題材、入力ファイル）は用意済み。中身のロジックは自分で書く。詰まったら、Claude にヒントかレビューを頼む。

## 0. 環境を直す（最初の 1 回だけ）

Rust 1.95 には、今 rust-analyzer が入っていない。そのため rustup と mise のあいだで呼び出しがループし、`infinite recursion detected` で止まる。このままだと Neovim の `gd` などが効かない。

```sh
rustup component add rust-analyzer rust-src   # rust-src が無いと std の中へ飛べない
rust-analyzer --version                        # バージョンが出れば直っている
```

LazyVim で Rust と Go の設定を有効にする。

1. `nvim` を開き、`:LazyExtras` を実行する。
2. `lang.rust` と `lang.go` の行へ移り、それぞれ `x` で有効にする。rustaceanvim、crates.nvim、gopls が入る。
3. Neovim を開き直し、`:checkhealth rustaceanvim` を実行して、rust-analyzer が見つかっていることを確かめる。

有効にした結果は dotfiles の `home/shared/nvim/lazyvim/lazyvim.json` に書かれる（リポジトリへの symlink になっている）。dotfiles 側でコミットしておく。

## 1. 覚えるキー（コードジャンプ）

LSP のキーは LazyVim のソース（`lua/lazyvim/plugins/lsp/init.lua`）で確かめてある。それ以外の行は未確認なので、`<leader>` を押して少し待ち、which-key の一覧で確かめる。

| キー | 動き |
|---|---|
| `gd` | 定義へ飛ぶ |
| `gr` | 参照の一覧 |
| `gI` | 実装へ |
| `gy` | 型の定義へ |
| `K` | ホバー（型とドキュメント） |
| `<C-o>` / `<C-i>` | 飛ぶ前の場所へ戻る／進む（ジャンプリスト） |
| `]]` / `[[` | カーソル下の名前の、次／前の出現 |
| `<leader>ca` | コードアクション（`use` の追加など。Rust では `<leader>cR` も同じ） |
| `<leader>cr` | 名前を変える |
| `]d` / `[d` | 次／前の診断（コンパイルエラー）※未確認 |
| `<leader>ss` | ファイル内のシンボルへ ※未確認 |
| `<leader>ff` | ファイルを探す ※未確認 |

練習の型：`gd` で飛び、読んだら `<C-o>` で戻る。`hjkl` で探さない。tobira.nvim が、より良い操作を提案してくれる。

## 2. 段階 1 の練習（仕様書の 6 章）

入力ファイルは `testdata/fixtures/` にある。どちらも `testdata/go/copyrace` に、手書きのシナリオ（`testdata/go/scenarios/copyrace/`）を overlay で差し込んで実行した本物の出力。

- `copyrace_s1.jsonl` / `.cover.out`：S1。pass
- `copyrace_s3.jsonl` / `.cover.out`：S3。race で fail

コードは `crates/crt-core/src/` に書く。1 つ終わるごとに `cargo test -p crt-core` を通し、コミットする。

### 練習 1：`go test -json` の 1 行を読む

1. `cargo add -p crt-core serde --features derive` と `cargo add -p crt-core serde_json` を実行する。
2. `gotest.rs` を作り、`lib.rs` に `pub mod gotest;` を書く。
3. 1 行を表す構造体 `TestEvent` を書く。
   - フィールドは `Action`、`Package`、`Test`、`Output`。
   - JSON のキーは大文字始まり。`#[serde(rename_all = "PascalCase")]` を使う。
   - 無いことがあるフィールドは `Option<String>` にする。
4. `fn parse_events(input: &str) -> Result<Vec<TestEvent>, serde_json::Error>` を書く。1 行ずつ `serde_json::from_str` する。
5. `#[cfg(test)]` のテストで `include_str!("../../../testdata/fixtures/copyrace_s1.jsonl")` を読み、`pass` のイベントがあることを確かめる。

Neovim での練習：`serde_json::from_str` の上で `gd` を押し、serde_json の中へ飛んで戻ってくる。`K` で型を見る。

### 練習 2：race を見つける

- `fn races(events: &[TestEvent]) -> Vec<String>` を書く。race が起きたテストの名前を返す。
- 判定：`Action == "fail"` のテストのうち、その `Output` に `WARNING: DATA RACE` を含む行があるもの。
- S1 では空になり、S3 では `TestCrtS3` になる。

余裕があれば、race の報告から 2 か所の `file:line` も取り出す（S3 なら `context.go:34` と `context.go:52`）。

### 練習 3：通った行を求める

- `coverage.rs` を作る。カバレッジの 1 行（`example.com/copyrace/context.go:44.27,46.3 1 0`）を読んで、ファイル、開始行、終了行、回数を取り出す。
- 回数が 1 以上のブロックの行を集め、`BTreeSet<u32>` で返す。
- S3 では、`Copy` の中の `for` の本体（44〜46 行目のブロック）の回数が 0 になっていることを確かめる。

### 練習 4：束の型

- 仕様書の 3 章の形を、serde の構造体で書く。
- S1 と S3 の結果から `Bundle` を組み立て、`serde_json::to_string_pretty` で出す。

ここまでできたら段階 1 は終わり。次は突き合わせ（段階 2）。
