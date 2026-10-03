# 多言語のコード構造をどこから取るか（tree-sitter・エディタの LSP・SCIP）

確認日: 2026-10-03。前提: v1 はテストを実行しない。凡例: [直] = GitHub API・raw ファイル。[要約] = WebFetch の要約経由。[推] = 推論。

## 答え

1. 多言語を最初から扱う構造の取得は、**tree-sitter を土台**にして「定義の範囲・外側の型・名前ベースの呼び出し元」を取るのが現実的。言語ごとの追加コストは tags クエリ（`.scm`）1 本。ただし tags は名前の一致で、型解決を伴う呼び出し解決ではない。
2. 精度が要る部分（呼び出し元・呼び出し先の確定）は、エディタの LSP の call hierarchy を「あれば使う」任意の強化層に置く。サーバーによる有無の差が大きく、全言語には揃わない。
3. SCIP と stack-graphs は土台に向かない。SCIP は索引器が約 9 系統で多言語に足りず、stack-graphs は GitHub が保守を終了した。
4. 実行を後で足す場合、言語ごとのアダプタ（テスト発見・コマンド構築・結果収集）は避けられない。並行処理の検証が安く成立するのは Go だけ。

## 根拠

### tree-sitter

- 公式の束縛: C#, Go, Haskell, Java, JavaScript (Node / Wasm), Kotlin, Python, Rust, Swift, Zig https://tree-sitter.github.io/tree-sitter/ [要約]。Rust は本体クレートが第一級。
- 保守 [直] 2026-10-03: tree-sitter 27,111★（v0.27.0、2026-08-30）。py-tree-sitter 1,507★、node-tree-sitter 877★、zig-tree-sitter 125★（2026-06-27）、**go-tree-sitter 298★は 2025-11-16 で停止**。
- 文法の品質は言語ごとにばらつく。tree-sitter-swift は archived（2022-01）。
- wasm 配布: CLI の `--wasm` と web-tree-sitter。Continue は `web-tree-sitter` を使う https://raw.githubusercontent.com/continuedev/continue/main/core/autocomplete/context/root-path-context/RootPathContextService.ts [直]
- tree-sitter-language-pack: 371 言語、パーサは初回使用時に取得。**tags クエリがあるのは 97 本** https://github.com/Goldziher/tree-sitter-language-pack [直]。文法がある数と「関数と呼び出し元を取れる」数は別。
- tags の capture は `@definition.*` と `@reference.call` など。名前解決は主張していない https://tree-sitter.github.io/tree-sitter/4-code-navigation.html [要約]

### aider の repo-map（LLM 文脈ツールの実例）

- ctags から tree-sitter へ移行 https://aider.chat/2023/10/22/repomap.html [要約]。tags ファイルは 31 本 https://github.com/Aider-AI/aider/tree/main/aider/queries [直]
- 参照は名前一致。`repomap.py` は pygments で識別子を拾って補う（"# Use pygments to backfill refs"）[直]
- `@reference.call` の数 [直]: go 5、python 3、rust 3、javascript 4、**c 0、cpp 0、csharp 0、swift 0、zig 0**。これらの言語では tags だけでは呼び出しが取れない。

### 他の基盤

- difftastic 25,966★（30 言語超）、ast-grep 16,106★、semgrep 16,843★（LGPL）、universal-ctags 7,293★ [直]
- **github/stack-graphs は archived**（"no longer supported or updated by GitHub"、最終 push 2025-09-09）https://github.com/github/stack-graphs [直]
- SCIP: 索引器は scip-java / typescript / clang / ruby / python / dotnet / dart / php / go の約 9 系統、各 18〜135★ https://github.com/scip-code/scip [直]。Sourcegraph 自身が精密索引と名前ベース検索の二層 https://sourcegraph.com/docs/code-search/code-navigation/precise_code_navigation [要約]
- CodeQL: 12 言語だが CLI のライセンスが私有コードの自動解析を禁じる（有償契約を除く）https://raw.githubusercontent.com/github/codeql-cli-binaries/main/LICENSE.md [直]
- Joern 3,542★（Apache-2.0）。JDK 21 が要件で、1 関数の文脈取得には重い [推]

### LLM 文脈ツールが実際に使うもの

- aider: tree-sitter の定義・参照 + グラフランク。
- Continue: tree-sitter と IDE の定義ジャンプの両方。LSP は IDE ごとの任意経路で、遅ければ捨てる（`racePromise`）https://raw.githubusercontent.com/continuedev/continue/main/core/autocomplete/snippets/getAllSnippets.ts [直]
- Cody: embeddings を撤退して BM25 系検索 https://sourcegraph.com/blog/how-cody-understands-your-codebase [要約]。cody-public-snapshot は archived（2025-08）[直]
- Cursor: 構文のチャンク + embeddings https://cursor.com/blog/secure-codebase-indexing [要約]
- 「関数 + 呼び出し元」を多言語で公開しているのは aider だけで、方式は名前一致。精度を数値で述べた一次情報は無い。

### エディタの LSP の call hierarchy（ソースで確認 [直]）

| サーバー | call hierarchy |
|---|---|
| gopls、rust-analyzer、pyright、basedpyright、clangd | あり |
| typescript-language-server | あり（クライアントの宣言と TS 3.8 以上が条件） |
| jdtls | import はあり、提供の最終確認は未 |
| ruby-lsp | **なし**。#340 は not_planned（型検査なしには確定できない） https://github.com/Shopify/ruby-lsp/issues/340 |
| Sorbet | **なし**。#2130 open |
| zls | 記述なし |

- 「全言語で揃う」前提は成り立たない。動的言語は呼び出し元を型検査なしに確定できない。
- 名前一致は取りすぎ、LSP は取りこぼす方向 [推]。

### Serena（LSP を主経路にした前例）

- 40 言語超を謳うが、機能表に call hierarchy は無い https://raw.githubusercontent.com/oraios/serena/main/README.md [直]
- multilspy から自前の solidlsp へ移行 [直]。運用の不満: jdtls の資源暴走と索引破損（#1944）、起動完了しない（#937）、pyright で implementations が空（#2019）。
- エディタの既存サーバーを再利用せず、独自に起動する設計 [推]。

### 実行を後で足す場合（短く）

- 共通の型は neotest の 3 関数。コミュニティアダプタ 47 件 [要約]
- Rust の libtest-json は experimental https://raw.githubusercontent.com/nextest-rs/nextest/main/site/src/docs/machine-readable/libtest-json.md [直]
- Go race detector: メモリ 5〜10 倍、時間 2〜20 倍、実行された経路のみ https://go.dev/doc/articles/race_detector [要約]
- Rust TSan は nightly + build-std、5 ターゲット [要約]。他言語は未調査。

## 注意点

- 名前一致と LSP の呼び出し元の再現率を比べた測定は見つからなかった。
- 各 tags クエリが関数の外側の型を取るかは未確認。
- LSP の起動・初回索引の時間の一次数値は未取得。
- GitHub のコード検索 API がレート制限に当たり、一部は raw ファイルの grep で代替。

## 判定

v1（実行なし）には、tree-sitter + 任意の LSP の二層が証拠で支えられる。LLM に渡すのは「確定した呼び出し元」ではなく「候補の呼び出し元 + 確からしさの印（名前一致か解決済みか）」にするのが、証拠と矛盾しない [推]。
