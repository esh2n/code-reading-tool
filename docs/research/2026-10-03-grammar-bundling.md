# Rust 単一バイナリで tree-sitter の文法を数十〜100 言語どう積み、どう読み込むか

確認日: 2026-10-03。凡例: [直] = GitHub API・crates.io/npm API・raw ファイル。[測] = Apple Silicon の Mac 1 台での実測（strip 済み、LTO なし）。[要約] = WebFetch の要約経由。[推] = 推論。

## 答え

1. **静的リンクを核にし、足す分だけ実行時ロードする混成**が、実例のある型（ast-grep、Zed）。10 言語前後は公式の文法 crate を静的リンクし、50〜100 は実行時ロードで、文法とクエリを設定で足せるようにする。
2. **静的リンクのコストは言語でまちまちで、長い尾がある。** tree-sitter 0.27 本体 0.36 MB。11 言語（Rust/Go/Python/JS/TS/Java/C/C++/C#/Ruby/PHP）で 17.05 MB [測]。中央値は 0.4〜0.6 MB だが、C# 5.4 MB、C++ 3.4 MB、Ruby 2.1 MB。Helix の 245 言語は 191 MB [測]。100 言語を全部静的に積むのは現実的でない。
3. **wasm 経路**（`tree-sitter` crate の `wasm` feature、Zed が本番採用）は成立するが重い。バイナリ +7.9 MB、ビルドに cmake と wasmtime のコンパイル（約 31 秒）[測]。Zed 自身も字句解析だけを wasm にする混成。
4. **tags クエリ付きで静的リンクできる Rust 製の多言語パックは無い。** tree-sitter-language-pack（Rust crate あり）は実行時ダウンロード方式で tags は 369 言語中 69。arborium は静的だが tags は 2 言語。
5. **tags クエリは上流で揃っていない。** 公式 11 言語の tags.scm のうち `@reference.call` が TS/C/C++/C# で 0 件 [直]。クエリを設定側で足す・上書きする設計が要る。これが「言語追加を設定だけで」の本当の難所。

## 根拠

### ABI と crate の互換

- tree-sitter 0.27.0 の `api.h`: `LANGUAGE_VERSION 15`、`MIN_COMPATIBLE 13`。範囲外は `Incompatible language version` で拒む [直]
- 11 文法すべて 0.27 で `set_language` 成功。ABI は Rust/Go/Python/JS/C/C#/PHP が 15、TS/Java/C++/Ruby が 14 [測]
- 公式の文法 crate は `tree-sitter-language = "0.1"` にだけ依存し、`tree-sitter` 本体の版に縛られない [直]。非公式の古い crate が `tree-sitter` に直接依存すると `links = "tree-sitter"` の衝突で解決に失敗する（fwcd/tree-sitter-kotlin#159）[直]
- Zed#24632（2025-02）「Zed crashes when an extension tries to use a Tree-Sitter with ABI version > 14」。ホスト側が古いと新しい ABI でクラッシュする [直]

### サイズの実測 [測]

| 構成 | バイナリ |
|---|---|
| tree-sitter 0.27 のみ | 0.36 MB |
| + c-sharp / cpp / ruby | +5.37 / +3.44 / +2.11 MB |
| + typescript / rust / php | +1.42 / +1.12 / +1.06 MB |
| + c / python / go / javascript / java | +0.63 / +0.45 / +0.22 / +0.41 / +0.40 MB |
| 11 言語まとめて | 17.05 MB |
| `wasm` feature（文法なし） | 8.24 MB、ビルド約 31 秒、cmake 必須 |

- 11 言語のクリーンビルド約 9 秒（M 系 Mac）。
- Helix 25.07.1 の `runtime/grammars`: 245 個の `.so`、計 191 MB。平均 790 KB、中央値 135 KB。verilog 18.4 MB、lean 15.8 MB [測]。静的リンクの実測と主要言語のサイズがほぼ一致（1 言語のコストは静的でも動的でも同じ）。
- ast-grep 0.45.3 は 51 MB（組み込み 28 文法）、difftastic 0.71.0 は 119 MB（58 文法、リポジトリ 1.6 GiB）[測][直]

### メンテナの発言

- Neovim #22313「Nvim can't ship hundreds of parsers」「total file size approaches gigabytes」[直]
- tree-sitter #5974（2026-09-30）「_Binary_ size, on the other hand, is something we do care about」[直]
- tree-sitter #5888 Fedora の `helix-parsers` が 195 MiB。回答「You can use WASM parsers. (_Any_ compiled binary from a non-trusted source is a security issue.)」[直]
- Zed「WebAssembly … is a great format for distributing parsers, because it's cross-platform, and it's designed for running untrusted code safely.」 https://zed.dev/blog/language-extensions-part-1 [要約]

### 製品の採用形 [直]

| 製品 | 方式 |
|---|---|
| Zed | 組み込み 34 文法を静的 + 拡張は `extension.toml` に `[grammars.x] repository, rev` で wasm |
| Helix | `languages.toml` に 303 の grammar（git URL とリビジョン）。`hx --grammar fetch/build` で `.so` を作って実行時ロード。リリースには事前ビルド済みの `.so` |
| ast-grep | 28 を静的 + `sgconfig.yml` の `customLanguages` で `.so` を動的ロード。静的と動的の橋渡し要望が open（#2934） |
| difftastic | 58 を静的 |
| avante.nvim | 17 を静的 |

### パック系 [直]

| 名前 | 中身 | tags.scm | 方式 |
|---|---|---|---|
| tree-sitter-language-pack（crate 1.20.0） | 369 言語 | 69 | 実行時に download して `libloading` |
| arborium（2.18.2） | 約 70 言語 | 2 | 静的、ハイライト用 |
| syntastica（0.6.1、1 年更新なし） | 複数 | 未確認 | tree-sitter 0.25 固定 |
| inkjet | — | — | archived |

### クエリの入手先

- 公式文法 crate は `TAGS_QUERY` を同梱。`@reference.call` の数: rust 3、go 1、python 1、javascript 2、java 1、ruby 2、php 3、**typescript 0、c 0、cpp 0、c-sharp 0** [直]
- aider（Apache-2.0）の tags.scm 58 本は各文法のライセンス（MIT 多数）に従う旨を README に書く [直]
- Helix（MPL-2.0）の tags.scm 77 本は `@definition` のみで `@reference` が無い [直]

## 推奨

- **(a) 最初の 10 言語前後**: 公式の文法 crate を静的リンク。Rust, Go, Python, JavaScript, TypeScript, Java, C, C++, C#, Ruby, PHP で約 17 MB。
- **(b) 50〜100 への成長**: 設定に「言語名、拡張子、文法の出所（`.so`/`.dylib` のパスか URL + SHA-256）、クエリのパス」を書く。Zed の `extension.toml` と Helix の `languages.toml` と同型。ロードは `libloading` + `tree-sitter-language` の `LanguageFn`。`.so` はリリース CI が OS ごとに `tree-sitter build` して配る。
- **(c) サイズ予算**: 静的は約 17 MB まで。外れ値（C# 5.4 MB、verilog、lean）は積まない。
- **(d) クエリ**: `@reference.call` が無い言語は、自前の tags.scm を同梱して上書きできる設計にする。

## 注意点

- 実測は 1 台、LTO なし。CI の Linux や musl では変わる。
- wasm 文法とネイティブ文法の同条件の速度比較は見つからなかった。
- tree-sitter-language-pack の「tags あり 69」は README を数えた値で、統合報告の「97」とは数え方が異なる。

## 前例が見つからなかったもの

- tags クエリ付きで静的リンクできる Rust の多言語パック。
- 100 言語規模を静的リンクしたまま配布する Rust 製 CLI（difftastic の 58 が最大級）。
- `@reference.call` を全言語に揃えた tags クエリの公開コレクション。
