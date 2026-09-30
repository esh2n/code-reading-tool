# 仕様：コードを「動き」で読む道具（MVP）

2026-09-30 作成。背景は `draft.md`、画面は `mocks/reading-ui-mocks.html`、根拠は `research/` の 3 本。

## 1. 何を作るか

Go の関数を 1 つ指定すると、次のものを作る。

- 行ごとの説明。確かめた説明には根拠のシナリオを付け、確かめていない説明は「推測」と明示する。
- シナリオと実行結果。シナリオは普通の値、境界値、同時に 2 つ来た場合。実行結果は、成否、通った行、race の 2 か所、テストが出した値。

これらを 1 つの JSON（**読解の束**、bundle）にまとめ、HTML、Neovim、VS Code に出す。

MVP で作るのは次の 3 つ。

- コア crate
- CLI（`crt`。仮の名前）
- HTML 出力

LSP サーバ、Neovim のプラグイン、VS Code の拡張は、MVP の後に作る（6 の順番を参照）。

### MVP で作らないもの

- 全体の俯瞰とファイルツリー。LSP の call hierarchy と、Neovim のシンボル検索で足りる。
- デバッガを使った変数の値の記録。値はテストが `t.Logf` で出したものだけを使う。
- Go 以外の言語。後から言語ごとのアダプタとして足す（Rust → TypeScript → Python の順）。
- 外部の DB や API が要る関数。

## 2. 流れ

```
crt read <package dir> <func>          例: crt read ./ "(*Context).Copy"
  1. 読む      対象の関数と、その周り（型、呼び出し元の一部）を集める
  2. 生成      LLM がシナリオ（Go のテスト）と、行ごとの説明を書く
  3. 実行      シナリオごとに go test を実行する
               go test -race -json -count=N -run '^TestCrtS4$'
                       -overlay=<overlay.json> -coverprofile=<S4.out>
  4. 突き合わせ  説明と実行結果を照らし合わせ、根拠の無い主張を弾く
  5. 書き出し   .code-reading/<func>.json（束）
crt render <bundle> --html out.html
```

- 生成したテストは対象のリポジトリに書かない。`-overlay` で差し込む。2026-09-30 に手元の Go 1.26.2 で試し、次を確かめた。
  - 差し込んだテストがコンパイルされて実行される
  - race が検出される
  - シナリオごとのカバレッジで、通らなかったブロックの回数が 0 になる
- race は、`fail` したテストの `output` に `WARNING: DATA RACE` があるかで判定する。専用のイベントは無い。
- **`-count=N` では race の回数を数えられない。** 2026-09-30 に `testdata/go/copyrace` の S3 を `-count=3` で動かしたところ、失敗したのは 1 回で、残りの 2 回は pass だった。race detector は、同じプロセスの中では同じ競合を 1 回しか報告しないためと考えられる。
  - 回数を数えるには、テストのバイナリを 1 回だけビルドし（`go test -c -race -cover -overlay=...`）、そのバイナリを N 回、別々のプロセスとして動かす（`-test.run`、`-test.coverprofile`）。JSON は `go tool test2json` を通して得る。この手順自体はまだ試していない。
- 通った行は、カバレッジの行（`file:開始行.桁,終了行.桁 文の数 回数`）の回数が 1 以上のブロックから求める。

### 突き合わせの規則（4）

この道具の芯。LLM の推測を、実行した事実で確かめる。

1. 説明が根拠に挙げたシナリオ ID が存在すること。
2. 根拠に挙げたシナリオで、その行が実際に通っていること（カバレッジ）。通っていなければ根拠を外し、「推測」に落とす。
3. race についての主張は、race の報告にある行番号と合うこと。
4. 根拠の無い説明は消さずに「推測」として残す。画面では色を分ける。

## 3. 読解の束（bundle）

形の案。型は Rust 側（コア crate）で serde の構造体として定義し、JSON Schema も出す。

```
Bundle
  target: { repo, commit, file, func, range: [start, end], file_hash }
  llm:    { model }                      どのモデルが書いたか（URL とキーは束に残さない）
  notes:  [LineNote]
  scenarios: [Scenario]

LineNote
  line: u32
  text: String                           行末に出す短い説明
  detail: Option<String>                 詳細パネル用
  evidence: [ScenarioId]                 空なら「推測」

Scenario
  id: "S4"
  kind: normal | boundary | concurrent
  title: String
  test_source: String                    生成した Go のテスト
  result:
    status: pass | fail | build_fail
    runs: u32, failures: u32             -count=N のうち何回落ちたか
    covered_lines: [u32]
    races: [{ write: Loc, read: Loc }]   Loc = { file, line, func }
    logs: [String]                       t.Logf の出力
```

- `file_hash` と `commit` が今のファイルと合わなければ、画面は「古い」と出す（編集で行がずれるため）。

## 4. LLM の呼び出し

- **特定の環境を前提にしない。** 特定のプロキシ、ティアの名前、モデル、ポート番号は、コードにも既定値にも入れない。宛先は使う人が与える。
- **宛先は OpenAI 互換の Chat Completions API に揃える。** 手元で動かす推論サーバもクラウドの API も、多くがこの形で受ける。一つの形に揃えれば、宛先は URL とモデル名だけで決まる。
- 設定は、いつも使う宛先 1 つだけ。予備の宛先（fallback）は、書いたときだけ使う。

```toml
# $XDG_CONFIG_HOME/crt/config.toml
[llm]
base_url = "http://…/v1"
model = "…"
api_key_env = "…"          # キーを読む環境変数の名前。キーそのものは書かない。キーが要らない宛先なら省く

[[llm.fallback]]           # 任意。書いた順に試す
base_url = "http://…/v1"
model = "…"
```

- `base_url` と `model` には既定値を持たせない。どちらかが無ければ、書き方を示すエラーで止める。
- 予備に切り替えるのは、宛先につながらないとき、または 5xx が返ったときだけ。返ってきた形が崩れているときは、予備に回さず同じ宛先で繰り返す（下）。
- 予備に切り替えたことは隠さない。画面に警告を出し、束の `llm.model` には実際に書いたモデルを残す。
- **出力は必ず構造化出力で受ける。** JSON Schema を指定した `response_format` を送る。
  - 宛先が構造化出力を受け付けないときは、形を落として続けずにエラーで止める。「JSON で返して」と指示するだけの形は作らない。
  - 形が崩れていたら、同じ要求を最大 2 回まで繰り返す。それでも駄目なら止める。
- モデルによっては文脈の上限が小さい（数万トークン）。送るのは対象の関数とその周りだけにし、上限は設定（`max_context_tokens`）で与える。

## 5. 構成とスタック

```
code-reading-tool/
  Cargo.toml                 workspace（edition 2024）
  crates/
    crt-core/                束の型、Go の実行、突き合わせ、LLM 呼び出し（library）
    crt-cli/                 crt コマンド（binary）
    crt-lsp/                 LSP サーバ（MVP の後）
  editors/
    nvim/                    Lua のプラグイン（MVP の後）
    vscode/                  TypeScript の拡張（その後）
  testdata/go/               検証用の小さな Go パッケージ
  docs/
```

| 用途 | crate | 理由 |
|---|---|---|
| 引数 | clap（derive） | Rust の CLI の標準 |
| エラー | anyhow（CLI）、thiserror（コアの型付きエラー） | 呼び出し側で分岐するエラーだけ型にする |
| JSON・設定 | serde、serde_json、toml | |
| HTML | minijinja | 実行時のテンプレート。学習中はコンパイル時の型エラーに悩まされない方がよい |
| LLM | reqwest（blocking）+ serde | OpenAI 互換の API は POST 1 本。専用の SDK は入れない |
| サブプロセス | std::process::Command | 数本のテストを順に動かすだけなので tokio は要らない |
| テスト | insta（束と HTML のスナップショット）、assert_cmd（CLI） | |
| LSP（後） | tower-lsp-server | Harper と typos-lsp が採用。本家の tower-lsp は 2023 年から止まっている |

- WASI/Wasm は使わない。コアはホストの `go` を起動する必要があるが、WASI にはプロセスを起動する口が見当たらない。
- Neovim と VS Code には LSP で届ける。行ごとの説明とシナリオモードは独自のリクエスト（`codeReading/lineNotes`、`codeReading/scenario`）で渡す。race は標準の diagnostics で、詳細は標準の hover で出す。

## 6. 作る順番と確かめ方

| 段階 | 作るもの | 終わりの条件 |
|---|---|---|
| 1 | 束の型と、`go test -json` の読み取り | `testdata/go/copyrace` に `testdata/go/scenarios/copyrace` の手書きのシナリオを overlay で差し込み、束ができる。S1 と S2 は pass、S3 は race になる |
| 2 | 突き合わせ | 通っていない行を根拠にした説明が「推測」に落ちる（単体テスト） |
| 3 | LLM の呼び出しと `crt read` | 宛先につながらないと予備に切り替わり、警告が出る。構造化出力を受け付けない宛先ではエラーで止まる（単体テスト）。実際に束ができる |
| 4 | `crt render --html` | HTML のスナップショットテストが通る |
| 5 | 実物で試す | 下の評価 |
| 6 | LSP、Neovim、VS Code | インライン → 詳細パネル → シナリオモードの順 |

### 役に立ったかの評価

過去に直されたバグの、直す前のコミットで `crt read` を動かし、そのバグを見つけられるかを見る（候補は `research/2026-09-30-nvim-plugin-architecture.md` の 5 件）。

- 境界値：go-humanize の `CustomRelTime`、`FormatFloat`、ftoa
- race：logrus の `Entry.write`、gin の `Context.Copy`

測るもの：

- 既知のバグに当たるシナリオを作れたか
- 実行でそのバグが表に出たか
- 束の説明のうち「推測」に落ちた割合

## 7. 未決

- コマンドの名前（`crt` は仮）
- 対象の関数の周りを、どこまで LLM に送るか（呼び出し元を含めるか）
- race が確率的にしか出ない場合の `-count` の既定値
