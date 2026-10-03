# 構成：レイヤーと依存の向き

2026-10-03。決定は `decisions/2026-10-03-architecture.md`。

## 1 つの規則

**依存は内側にだけ向く。** 内側の層は、外側の層の名前・型・データ形式を知らない。Rust では crate の分割でこれを機械的に強制する。内側の crate は外側の crate を `[dependencies]` に持たないので、違反するとビルドが通らない。

```
 外側 ─────────────────────────────────────────────────────────────────┐
  crt-cli / crt-lsp / crt-html      入口と出口（CLI、LSP サーバ、HTML）   │  組み立て
  crt-treesitter  crt-llm  crt-store  crt-wire                          │  アダプタ
  crt-app                           ユースケースとポート（trait）         │  アプリケーション
  crt-domain                        エンティティと規則                    │  ドメイン
 内側 ─────────────────────────────────────────────────────────────────┘
```

| crate | 層 | 中身 | 依存してよいもの |
|---|---|---|---|
| `crt-domain` | ドメイン | 言語 ID、シンボル、関数、構造の事実、内容ハッシュ、注釈と根拠の種類、「根拠の無い注釈は推測に落とす」規則 | 標準ライブラリだけ。serde も tree-sitter も知らない |
| `crt-app` | アプリケーション | ユースケース（ファイルを解析する、関数を読む）と、外側に求める能力の trait（ポート） | `crt-domain` |
| `crt-treesitter` | アダプタ | `StructureSource` ポートの実装。同梱した文法と tags クエリでシンボルを取る | `crt-app`、`crt-domain`、tree-sitter の crate |
| `crt-llm` | アダプタ | `Explainer` ポートの実装。OpenAI 互換 API を呼ぶ | `crt-app`、`crt-domain`、HTTP と JSON の crate |
| `crt-store` | アダプタ | `ReadingStore` ポートの実装。利用者のキャッシュ領域にファイルで保存 | `crt-app`、`crt-domain` |
| `crt-wire` | アダプタ（プレゼンタ） | 外に出す JSON の形（DTO）と、ドメインの型からの変換。LSP の独自メソッド、CLI の出力、HTML の入力が共有する | `crt-domain`、serde |
| `crt-cli`、`crt-lsp`、`crt-html` | 組み立て | 実装をポートに差し込み、ユースケースを呼び、`crt-wire` で出す | すべて |

## 各層の責務

- **ドメイン**：この道具が何を「読む」と呼ぶかの定義。関数とは tags の定義のうち function か method であること、メソッドの外側の型はそれを含む最小の型定義であること、注釈は根拠（構造の事実／LLM の推論／将来は実行で確かめた事実）を 1 つ持つこと、事実を根拠に挙げた注釈はその事実が存在しなければ推測に落ちること。I/O は無い。
- **アプリケーション**：手順。ファイルを解析するには、ポートからシンボルをもらい、ドメインの規則で関数と事実に組み立てる。関数を読むには、キャッシュを引き、無ければ説明を生成し、規則で検査し、保存する。ここに業務規則（何が推測か）は書かない。
- **アダプタ**：翻訳だけ。tree-sitter の tag の種類名（Go は `type`、他は `class`）をドメインの `SymbolKind` に写す、HTTP の応答をドメインの注釈に写す、ドメインの関数を JSON に写す。判断はしない。
- **組み立て**：どの実装を使うかを決めて配線する。

## 境界をまたぐデータ

- ポートの引数と戻り値はドメインの型。外側の形式（tree-sitter の `Tag`、HTTP のレスポンス、JSON）はアダプタの中で閉じる。
- 外に出す JSON は `crt-wire` の DTO。ドメインの型に serde を付けない。理由：JSON の形は LSP の独自メソッドの契約として版を持ち、ドメインの型とは別の理由で変わる。
- ユースケースの入出力は、ドメインの型をそのまま使う。1 つの入口しか無い段階で層ごとに同じ構造体を作ることはしない。

## 言語を足すとき

`crt-treesitter` の同梱リストに文法と tags クエリを 1 つ足す。ドメインとアプリケーションは変わらない。

## 作る順番と層

| 段階 | 足す crate |
|---|---|
| 1 | `crt-domain`（シンボル、関数、事実、ハッシュ）、`crt-app`（`AnalyzeFile`、`StructureSource`）、`crt-treesitter`、`crt-wire`、`crt-cli` |
| 2 | `crt-domain` に注釈と根拠と検査の規則、`crt-app` に `ReadFunction`・`Explainer`・`ReadingStore`、`crt-llm`、`crt-store` |
| 3 | `crt-lsp` |
| 4 | `crt-html`、エディタ拡張（`editors/nvim`、`editors/vscode`） |
