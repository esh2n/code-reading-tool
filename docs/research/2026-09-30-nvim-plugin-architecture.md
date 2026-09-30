# Rust コアの Neovim プラグイン構成、neotest の再利用、評価に使う Go の OSS

確認日: 2026-09-30。凡例: [直] = gh api・raw ファイルを直接取得。[要約] = WebFetch の要約経由。[推] = 推論・未検証。star 数と push 日は 2026-09-30 時点。

## 答え

1. **Rust コアの形**: コアを独立した Rust バイナリにし、stdin/stdout の JSON で話す。Neovim 側は薄い Lua が `vim.system` で起動して JSON を読む。同じバイナリを coding agent の skill からも呼べる。この用途でこの形が標準だと明言した一次資料は無く、下の破損報告と「Neovim の外でも動く」要件からの推論 [推]。
   - cdylib を Lua モジュールとして読む形（blink.cmp、avante.nvim）は速いが、クラッシュすると Neovim ごと落ちる。ビルド・配布の破損報告も多い。
   - nvim-oxi は Neovim の版に強く結びつく（0.12.2 で panic、open）。
   - リモートプラグイン（msgpack-RPC のホスト方式）は非推奨ではないが、Neovim 本体の issue で「複雑すぎる」と簡素化が提起され、open のまま。
2. **neotest**: 「ファイル内のテスト位置を見つけて実行し、結果を位置に戻す」枠組み。結果型（`status` / `output` / `short` / `errors`）は、説明の各文を実行結果に結びつけるには粒度が足りない [推]。再利用するなら消費者として。JUnit XML は go test と cargo test が素では出さない。結果の入力ではなく、エクスポート先として扱う。
3. **評価対象**: 外部サービス不要で `go test` だけで動く Go の OSS から、バグ修正の前のコミットを 5 件選んだ（下表）。GoBench は大規模アプリ由来で、関数 1 つを対象にするには粒度が大きい。

## 根拠

### 1. 構成の比較

| 観点 | 外部バイナリ + JSON | cdylib / nvim-oxi | リモートプラグイン |
|---|---|---|---|
| 障害の影響 | バイナリだけが落ちる | Neovim ごと落ちる | ホストだけが落ちる |
| Neovim の版との結びつき | 弱い（`vim.system` と JSON のみ） | 強い | 中 |
| 単体検証 | `cargo test` で足りる | Neovim 内でのテストが要る | ホスト経由 |
| Neovim の外で動く | そのまま動く | 別ビルドが要る | 動かない |
| Rust 学習者向き | 高い | 低い（unsafe、FFI、ABI） | 低い |

- `vim.system`「Runs a system command or throws an error if {cmd} cannot be run.」 https://raw.githubusercontent.com/neovim/neovim/master/runtime/doc/lua.txt [直]
- sniprun（1,714★）は Rust バイナリを `jobstart(..., { rpc = true })` で起動する https://github.com/michaelb/sniprun [直]。cargo の target-dir 設定でバイナリの場所がずれる不具合の修正 PR がある https://github.com/michaelb/sniprun/pull/329 [直]
- blink.cmp は `crate-type = ["cdylib"]` と mlua `module` を使い、プレビルドを取得できなければ純 Lua 実装に戻る https://github.com/Saghen/blink.cmp/blob/main/Cargo.toml 、 https://cmp.saghen.dev/configuration/fuzzy.html [直]
- blink.cmp の破損報告: プレビルドで Neovim が落ちる #1516、古い CPU で落ちる #2453、nightly の cargo でビルド失敗 #2376 https://github.com/saghen/blink.cmp/issues/1516 、 https://github.com/saghen/blink.cmp/issues/2453 、 https://github.com/saghen/blink.cmp/issues/2376 [直]
- avante.nvim は cdylib を 4 つ持つ。macOS の拡張子（.so/.dylib）によるビルド失敗 #3110、Lazy のタイムアウト #3202 https://github.com/avante-corp/avante.nvim/issues/3110 、 https://github.com/avante-corp/avante.nvim/issues/3202 [直]
- nvim-oxi「`set_hl` panics on nvim 0.12.2」 https://github.com/noib3/nvim-oxi/issues/311 [直]
- リモートプラグインの公式文書に「deprecated」の語は無い https://raw.githubusercontent.com/neovim/neovim/master/runtime/doc/remote_plugin.txt [直]。「The "remote plugin" model currently is too complex」 https://github.com/neovim/neovim/issues/27949 （open）[直]
- fff は Rust コアに Neovim・Node・MCP の複数の窓口を付けた実例 https://github.com/dmtrKovalenko/fff [直]

### 2. neotest

- アダプタの仕事は「Parse tests / Construct test commands / Collect results」 https://github.com/nvim-neotest/neotest [直]
- 結果型 https://github.com/nvim-neotest/neotest/blob/master/lua/neotest/types/init.lua [直]
- neotest-golang（268★、活発） https://github.com/fredrikaverpil/neotest-golang 。neotest-rust は archived で、代わりに rustaceanvim のアダプタを使う https://github.com/mrcjkb/rustaceanvim [直]
- JUnit XML: pytest は `--junitxml` を持ち、Vitest は `junit` reporter を持つ [要約]。go test は gotestsum、cargo test は cargo-nextest を通す必要がある [直]

### 3. 評価対象（Go）

| # | リポジトリ | 修正 PR | 対象関数 | 種別 | 親コミット |
|---|---|---|---|---|---|
| 1 | dustin/go-humanize | https://github.com/dustin/go-humanize/pull/65 | `CustomRelTime` | 境界値（1 週間ちょうどでの off-by-one） | 0b19b17f90（go.mod が無い） |
| 2 | dustin/go-humanize | https://github.com/dustin/go-humanize/pull/157 | `FormatFloat` | 境界値（int64 上限での桁あふれ） | 33e8a42f57 |
| 3 | dustin/go-humanize | https://github.com/dustin/go-humanize/pull/158 | ftoa.go の桁数制限 | 境界値 | e0c1e0ea67 |
| 4 | sirupsen/logrus | https://github.com/sirupsen/logrus/pull/1263 | `Entry.write` | データ競合（ロックの範囲の誤り） | 79c5ab66aa |
| 5 | gin-gonic/gin | https://github.com/gin-gonic/gin/pull/1841 | `Context.Copy` | データ競合（`Params` の共有） | 35e33d3638 |

- 追加の候補（未精査）: gin #2675、zap #1511、logrus #1494。
- GoBench: 82 件の実バグと 103 件の bug kernel https://github.com/timmyyuan/gobench [直]

## 注意点

- issue は否定側に偏る。クラッシュ報告の頻度は利用者数に対して不明。
- `vim.system` + JSON で Rust バイナリを制御する大きな実例は特定できていない。
- 評価対象で、親コミットで実際にバグが再現するかは実行して確かめていない。#4 と #5 の `-race` の再現は確率的かもしれない。古いコミットが今の Go でビルドできるかも未確認。
- 「Rust コアを cdylib から外部バイナリへ移した（またはその逆）」という実践者の体験記は見つからなかった。
