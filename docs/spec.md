# 仕様 v2：コードを「動き」で読む道具

2026-10-03。v1（2026-09-30）を置き換える。変更点：Neovim と Rust の練習という制約を外し、作るものから逆算して技術を選び直した。根拠は `research/` の 4 本。画面の案は `mocks/reading-ui-mocks.html`。

## 1. 何を作るか

エディタで関数を 1 つ指すと、次の 3 つを出す。

1. **インライン注釈**：各行の末尾に「この行が何をするか」。実行で確かめた注釈には根拠のシナリオ ID が付き、確かめていない注釈は「推測」と分かる色で出る。
2. **詳細パネル**：行の上でホバーすると、詳しい説明、根拠になったシナリオの結果、race の 2 か所。
3. **シナリオモード**：シナリオを 1 つ選ぶと、通った行だけが明るくなり、別のビューにそのシナリオで起きたことが順に並ぶ。race もシナリオの中の出来事として出る。

シナリオは LLM が書き、**実際にテストとして実行する**。注釈と実行結果を突き合わせ、根拠の無い主張を「推測」に落とす。これがこの道具の芯で、LLM の推測だけでは精度が足りない（`draft.md` の CRUXEval、DRPBench の数字）ことへの答え。

出力先は 3 つ：Neovim、VS Code、静的 HTML。

### 作らないもの

- 全体の俯瞰とファイルツリー。エディタの LSP（call hierarchy、シンボル検索）で足りる。
- デバッガによる変数の値の記録。値はテストが `t.Logf` で出したものだけ。
- 外部の DB や API が要る関数の実行。

## 2. 構成

```
                 ┌──────────────── crt（Go、単一バイナリ）────────────────┐
                 │  analyze   対象の関数・型・呼び出し元を集める（go/packages）│
エディタ ──LSP──▶│  generate  LLM にシナリオと注釈を書かせる（構造化出力）     │
  Neovim (Lua)   │  run       go test -c -overlay で 1 回ビルド、並列に実行    │
  VS Code (TS)   │  check     注釈と結果を突き合わせる                        │
                 │  bundle    読解の束（JSON）を .code-reading/ に保存         │
                 └──────────────────────────┬────────────────────────────┘
                                            └─ crt render --html（束 → 1 ページ）
```

- **バックエンドは Go で 1 つの長期プロセス。** エディタとは LSP（stdio）で話す。Neovim も VS Code も LSP クライアントを持っているので、起動・再起動・進捗・キャンセル・診断がそのまま使える。
- **エディタ側は薄い。** Neovim は Lua、VS Code は TypeScript。どちらも「LSP クライアントを起動し、独自リクエストの結果を描く」だけ。
- **HTML は束をテンプレートに流すだけ。** `crt render` は同じバイナリのサブコマンド。

### 言語の選定（`research/2026-10-03-backend-language.md`）

| 候補 | 判断 | 決め手 |
|---|---|---|
| **Go（採用）** | 最初の対象が Go。`go/packages`・`go/types`・`cover.ParseProfiles`・test2json の型が公式ライブラリにある。単一バイナリ 2.5 MB、起動 1.7 ms | 部品がそろっている。速さではない |
| Rust | 起動 1.2 ms、0.47 MB で僅かに勝るが、Go の型情報と cover/test2json の読み取りを自作する | 性能差は LSP の応答に効かない |
| TypeScript | VS Code 側は最も楽だが、Neovim 利用者に Node を要求するか 63 MB の単一バイナリになる | copilot.lua も Node 同梱をやめた |

**性能の本当の支配項**は `go test -c -race` のビルドと実行、そして LLM の応答で、言語では決まらない。言語差（ms）より、次の設計で桁が変わる。

### 性能設計（実測つき）

| 設計 | 理由 | 実測（`testdata/go/copyrace`、2026-10-03） |
|---|---|---|
| テストバイナリを 1 回ビルド（`go test -c -race -cover -overlay`） | シナリオごとに `go test` を呼ぶとビルドが N 回走る | 初回 0.80 秒、キャッシュ後 0.08 秒 |
| シナリオを**別プロセス**で並列実行 | 同じプロセス内の `-count=N` では race は 1 回しか報告されない | 20 本並列 0.024 秒、race 20/20 回検出（`-count=3` では 1/3） |
| 結果を 1 本ずつエディタへ流す（`$/progress` と通知） | LLM の注釈を待たずに、シナリオの成否と race から先に見せる | — |
| 束をファイルのハッシュで鍵にしてキャッシュ | 開き直しで再実行しない | — |
| LLM 呼び出しは 2 回（シナリオ生成 → 実行 → 注釈生成） | 注釈は実行結果を見てから書かせる方が精度が上がる想定 | 未検証 |

## 3. 流れ

```
1. エディタがカーソル位置の関数を LSP で送る         codeReading/read {uri, position}
2. analyze   関数本体、レシーバ型、直接の呼び出し元 N 件を集める
3. generate  LLM へ: 上の文脈 → シナリオ（Go のテスト関数）を JSON Schema で受ける
4. run       -overlay で差し込み、go test -c、別プロセスで並列実行、test2json と cover を読む
             → 1 本終わるごとに codeReading/scenarioResult を通知
5. generate  LLM へ: 文脈 + 実行結果 → 行ごとの注釈（根拠の ID 付き）
6. check     根拠の ID が存在し、その行がそのシナリオで通っていて、race の行が合うか
             合わなければ「推測」に落とす（消さない）
7. bundle    .code-reading/<pkg>/<func>.json に保存、エディタへ codeReading/bundle
```

### 突き合わせの規則（6）

1. 注釈が根拠に挙げたシナリオ ID が存在する。
2. そのシナリオのカバレッジで、その行が通っている。通っていなければ根拠を外す。
3. race についての主張は、race の報告の行番号と合う。
4. 根拠の無い注釈は「推測」として残す。

## 4. 読解の束（bundle）

```
Bundle
  target    { repo, commit, file, func, range, file_hash }
  llm       { model }                       URL とキーは残さない
  notes     [ { line, text, detail?, evidence: [ScenarioId] } ]   evidence が空なら推測
  scenarios [ Scenario ]

Scenario
  id        "S4"
  kind      normal | boundary | concurrent
  title
  test_source
  result    { status: pass|fail|build_fail, runs, failures,
              covered_lines: [u32],
              races: [ { write: Loc, read: Loc } ],   Loc = { file, line, func }
              logs: [string] }
```

`file_hash` が今のファイルと合わなければ、エディタは「古い」と表示する。

## 5. LSP の面

| 画面 | Neovim | VS Code | LSP |
|---|---|---|---|
| インライン注釈 | extmark（`virt_text`、行末） | decoration（`after`） | 独自 `codeReading/lineNotes` |
| race の 2 か所 | diagnostic。`gf` で相手の場所へ | 波線と Problems | 標準 diagnostics + `relatedInformation` |
| 詳細パネル | hover の浮動ウィンドウ | hover | 標準 `textDocument/hover` |
| シナリオモード | 別バッファ + extmark | 仮想ドキュメント（webview は後） | 独自 `codeReading/scenario` |
| 実行中 | statusline | ステータスバー | 標準 `$/progress` |
| 開始 | `:CrRead` | コマンドパレット | 独自 `codeReading/read` |

inlay hint は使わない（VS Code は 43 文字で切る）。独自メソッドは rust-analyzer に倣い `codeReading/` の名前空間に置き、`experimental` capabilities で宣言する。

ライブラリ：sourcegraph/jsonrpc2 + go.lsp.dev/protocol（Go 製 LSP の多数派。決め手に欠けるのは認識済み）。

## 6. LLM

- 宛先は OpenAI 互換の Chat Completions。設定は `base_url`、`model`、`api_key_env`（キーを読む環境変数の名前）の 3 つ。任意の `[[llm.fallback]]`。特定のプロキシやモデルを既定値に持たない。
- **構造化出力は必須。** JSON Schema の `response_format` を受け付けない宛先はエラーで止める。形が崩れたら同じ宛先に最大 2 回。
- 予備に切り替えたら警告を出し、束には実際に書いたモデル名を残す。
- 送る文脈は関数とその周り（呼び出し元 N 件まで）。上限は `max_context_tokens` で与える。

## 7. 対象言語の拡張

Go の次は Rust → TypeScript → Python。言語ごとに差し替えるのは 3 点だけ：関数の範囲の取得、テストの書き込みと実行、結果の読み取り。

- 関数の範囲は**エディタの LSP から受け取る**（document symbols）。バックエンドが各言語を構文解析しない。
- 実行は各言語のツールチェーンの JSON 出力（`cargo test -- --format json`、vitest の reporter、pytest の JSON）。
- これで Go の tree-sitter バインディングが弱い問題を避ける。

## 8. 作る順番

| 段階 | 作るもの | 終わりの条件 |
|---|---|---|
| 1 | `crt` の analyze / run / check / bundle（LLM 抜き、手書きのシナリオ） | `testdata/go/copyrace` の手書きシナリオから束ができ、S3 が race になる |
| 2 | generate（LLM）と `crt read` CLI | 設定した宛先でシナリオと注釈が生成され、根拠の無い注釈が「推測」に落ちる |
| 3 | `crt render --html` | 束からモックの「5 HTML」と同じ形のページが出る |
| 4 | LSP サーバ + Neovim プラグイン | インライン → 詳細 → シナリオモードの順 |
| 5 | VS Code 拡張 | typos-lsp と同じ `extension.ts` 1 ファイルから |
| 6 | 評価 | 下 |

### 評価

過去に直されたバグの、直す前のコミットで `crt read` を動かし、既知のバグを見つけられるか。

- 境界値：go-humanize の `CustomRelTime`（PR #65）、`FormatFloat`（#157）、ftoa（#158）
- race：logrus `Entry.write`（#1263）、gin `Context.Copy`（#1841）

測るもの：既知のバグに当たるシナリオを作れた割合、実行でバグが表に出た割合、「推測」に落ちた注釈の割合、関数 1 つの所要時間。

## 9. 未決

- コマンド名（`crt` は仮）
- 呼び出し元を何件まで LLM に送るか
- race が確率的な場合の既定の実行回数（題材では 20/20 だが、実物では未測定）
- 注釈の生成を実行の後にする（2 回呼ぶ）か、同時にする（1 回）か
- Go の LSP ライブラリ（jsonrpc2 + protocol か、go-lsp か）
