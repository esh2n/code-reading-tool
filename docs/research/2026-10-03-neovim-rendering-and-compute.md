# Neovim での行ごと EOL 仮想テキストの描画と、計算側の実装方式

確認日: 2026-10-03。凡例: [直] = raw ファイル・gh api・クローンした HEAD。[要約] = WebFetch の要約経由。[測] = このセッションでの実測（Apple Silicon の Mac、nvim v0.12.5、LuaJIT 2.1。1 機種のみ）。[推] = 推論。先行記録（cdylib の破損、nvim-oxi の版結合、remote plugin の状態、inlay hint の切り詰め）は再調査していない。

## 答え

1. **200〜2,000 行の EOL 注釈は、描画がボトルネックにならない。** 行ごとに `nvim_buf_set_extmark` を Lua から呼ぶ素朴な形で、2,000 行が 2.3 ms、10 万行でも 124 ms [測]。スクロール時の再描画は extmark が 0 件でも 10 万件でも 0.04〜0.07 ms [測]。
2. **描画の型は 3 つ併存する。**
   - (A) `nvim_set_decoration_provider` の `on_win` で見えている行だけ。Neovim 本体の inlay hint・semantic tokens・codelens、gitsigns、snacks、mini.diff。
   - (B) デバウンスして見えている範囲＋余白に永続 extmark。indent-blankline v3 と render-markdown.nvim（**provider は使っていない**）。
   - (C) 結果が届いた時点で全行に永続 extmark。本体の `vim.diagnostic` と nvim-dap-virtual-text。
   - 編集で行がずれても追従させるには永続 extmark が要る。本体の semantic tokens はこの理由で ephemeral を使わない。
3. **外部プロセスと JSON が、この用途で最も安全な計算側。** 1 回の往復は 0.01 ms 未満、プロセス起動は約 1 ms、313 KB（2,000 行分）の `vim.json.decode` は 0.69 ms [測]。計算が重くないので cdylib や FFI の速さは要らない。
4. **Neovim に wasm のプラグインホストは無い。** #23579 は「Not planned」。
5. **Neovim の方針は Lua 優先。** 外部の解析は LSP が文書上の入口で、Lua 関数を `cmd` に渡せば同一プロセスの LSP サーバにもなる。
6. **tree-sitter で多言語の構造を取れるが、パーサの有無は利用者の環境次第。** 本体が持つパーサは 7 種だけ。

## 根拠

### 描画性能 [測]

| 行数 | 永続 extmark の設定（合計） | 再描画（スクロール） |
|---|---|---|
| 200 | 0.2 ms | 0.02 ms |
| 2,000 | 2.3 ms | 0.05 ms |
| 20,000 | 24.6 ms | 0.06 ms |
| 100,000 | 124 ms | 0.06 ms |

- 外部プロセスから RPC で付ける場合 [測]（2,000 行）: 1 extmark = 1 回の同期 `rpcrequest` で 50.6 ms、`nvim_call_atomic` にまとめて 5.0 ms、`nvim_exec_lua` 1 回でデータを渡して 2.7 ms。1 行ずつは約 20 倍遅い。
- api.txt の注意 [直] https://raw.githubusercontent.com/neovim/neovim/master/runtime/doc/api.txt：provider の `on_line` は非推奨で `on_range` が現行。「It is not allowed to remove or update extmarks in `on_line` or `on_range` callbacks.」「A plugin managing multiple sources of decoration should ideally only set one provider」。
- semantic tokens が ephemeral を使わない理由 [直] https://github.com/neovim/neovim/blob/master/runtime/lua/vim/lsp/semantic_tokens.lua：「the buffer updates are not in sync with the list of semantic tokens. There's a delay between the buffer changing and when the LSP server can respond with updated tokens, and we don't want to "blink"」。外部の解析結果を行に貼る形にそのまま当てはまる。
- codelens は応答の到着時に消すと点滅するので、更新を保留する修正が入った https://github.com/neovim/neovim/pull/38782 [直]
- 各実装 [直]: gitsigns `lua/gitsigns/manager.lua:312`（A）、snacks `lua/snacks/indent.lua:500`（A、ephemeral）、indent-blankline `lua/ibl/config.lua:21-25`（B、`debounce = 200`、`viewport_buffer = {min = 30, max = 500}`）、render-markdown `lua/render-markdown/init.lua:63,80`（B、`debounce = 100`、`max_file_size = 10.0`）、`vim.diagnostic/_handlers.lua`（C）、nvim-dap-virtual-text `virtual_text.lua:252-299`（C）。
- プラグインが README で引用する描画性能の数字は無い。あるのは設定値だけ。

### 計算側の言語

**純 Lua / LuaJIT**
- 「these cannot be assumed to be available, and Lua code ... should check the `jit` global」 https://raw.githubusercontent.com/neovim/neovim/master/runtime/doc/lua.txt [直]。FFI は前提にできない。
- [測] 2,000 行の JSON（313 KB）: `vim.json.decode` 0.69 ms、`vim.json.encode` 0.87 ms、`vim.fn.json_decode` 2.3 ms、`vim.mpack` encode+decode 3.8 ms。JSON の方が msgpack より 4〜5 倍速い。

**FFI で C / Zig / Rust**
| 前例 | 状態 |
|---|---|
| telescope-zf-native.nvim（164★、Zig、`ffi.load`） | 最終 push 2024-09-15 |
| telescope-fzf-native.nvim（1,757★、C） | segfault の報告 #67 #11、ビルド失敗が open #153 #152 |
| blink.cmp（6,607★、Rust の Lua C モジュール） | Lua 実装に戻れる。「~6x the performance of fzf」は 1 万件超の照合の数字 |
| zig-lamp（41★） | 利用者の機械でビルド |
| wasm_nvim（320★、Rust cdylib が wasm を載せる） | 2025-08 push。数字は作者の微小ベンチ |

Zig の FFI の前例は zf と zig-lamp の 2 件で、どちらも軽い計算。

**外部プロセス** [測]
- `vim.system` の起動 1.04 ms。常駐した子との 1 行往復は中央値 0.007 ms。313 KB の往復 0.89 ms。

**WebAssembly**
- #23579「wasm (webassembly) plugins」open、「This is just a tracking issue. Not planned.」 https://github.com/neovim/neovim/issues/23579 [直]。試作 2 件（wasnvim 8★は 2023-09 で停止、wasm_nvim 320★）はどちらも本体に入っていない。
- wasmtime は tree-sitter の wasm パーサ用の任意ビルドオプションのみ [直]
- 対比: Zellij は wasmtime から wasmi へ移した https://github.com/zellij-org/zellij/pull/4449 [直]。Zed は `wasm32-wasip2` [要約]。どれも専用のランタイムを持つ。Neovim は持たない。

### Neovim の方針

- Lua プラグインは登録不要 https://raw.githubusercontent.com/neovim/neovim/master/runtime/doc/lua-plugin.txt [直]。`vim.pack` は experimental [直]。
- in-process の LSP サーバ: 「`vim.lsp.start()` accepts `cmd` as a Lua function」 https://raw.githubusercontent.com/neovim/neovim/master/runtime/doc/lsp.txt [直]
- remote plugin は roadmap で簡素化の対象、#27949 は open（先行記録）。

### tree-sitter

- 本体が持つパーサは C、Diff、Lua、Markdown、Vimscript、Vimdoc、query のみ [直]。理由 #22313「the total file size approaches gigabytes」[直]。計画 #39006 は「top 20」を `vim.pack.add{}` で入れる案。
- `vim.treesitter.get_parser` はパーサが無ければ nil。
- nvim-treesitter main（14,434★）は 0.12 以上、`tree-sitter-cli`、C コンパイラが要り、利用者の機械でビルドする。約 320 言語。「This plugin does not support lazy-loading.」[直]
- 関数の範囲は `@function.outer`（nvim-treesitter-textobjects、52 言語）[直]。本体の 5 種のクエリには無い。
- [測] Lua 2,289 行の初回解析 12.1 ms、`iter_captures` 1.7 ms。

## 注意点

- 測定は 1 機種。再描画は内部の同期更新の時間で、端末への出力を含まない。
- 80 字弱の説明文で測った。ウィンドウ幅より長いときの折り返しと `virt_lines` は未調査。
- indent-blankline と render-markdown が provider を使わない理由は文書に無い [推: 非同期の結果を編集に追従させるため]。
- 型 A は、バッファ編集で行がずれたときの古い結果の扱いを自前で処理することになる。

## 推奨の組み合わせ

| 要件 | 描画 | 計算と受け渡し |
|---|---|---|
| 200〜2,000 行の EOL 注釈 | 永続 extmark を到着時に 1 回の Lua 呼び出しで全行に（型 C）。2.3 ms | 外部プロセス + stdout の JSON（`vim.json.decode`） |
| 数万行以上 | `on_win` で見える行だけ（型 A） | 同上 |
| 外部プロセスが Neovim を直接操作する | 1 回の `nvim_exec_lua` か `nvim_call_atomic` にまとめる | msgpack-RPC |
| wasm | 使わない（ホストが無い） | — |
| LSP の枠に収まる部分（診断、hover） | 本体の機能 | 外部 LSP、または in-process |
| 多言語の関数範囲 | `vim.treesitter.get_parser` + `@function.outer` | パーサは利用者次第。無い言語の代替が要る |

## 前例が見つからなかったもの

- 外部で作った注釈を 1 ファイルの全行へ EOL 仮想テキストで貼る Neovim プラグイン。
- extmark の件数と描画の遅延の関係を測った公開資料。
- 本体に入った wasm プラグインホスト。
- 「外部の解析ツールは LSP で統合せよ」という maintainer の明言。
