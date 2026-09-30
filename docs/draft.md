# 草案：AI が書いたコードを「動き」で読むための仕組み

2026-09-30 時点の調査にもとづく草案。

## 背景

- AI がコードを書く量が増え、人がレビューして理解するのが追いつかない。
- 差分のレビューは今も大事。ただ、差分だけではコード全体がどう動くかは分からない。
- 欲しいのは次の 5 つ。
  1. 全体の流れを見渡せる俯瞰ビュー
  2. 関数を 1 行ずつ解説し、どう動くかを示すもの
  3. 上の二つと連動するファイルツリー
  4. 具体的なリクエストや値（境界値を含む）が来たときの動き
  5. リクエストが同時に来たときに起きうること（競合、更新の取りこぼし、二重送信など）と、問題になりそうな箇所

## 調べて分かったこと

- **5 つを一つでまかなう道具は無い。** 3 つ（1〜3）に一番近いのは ProjectMind だが、対応言語が Java と Rust だけで、使われた実績もまだ無い（GitHub のスター 5、v0.12）。
- **既存の道具は 3 つのグループに分かれる。**

| グループ | 代表例 | できないこと |
|---|---|---|
| 全体の図・wiki | DeepWiki、Google Code Wiki、CodeBoarding、GitDiagram、CodeFlow | 行ごとの解説 |
| 行の範囲・差分の解説 | CodeTour、CodeRabbit Change Stack、crit、Stage | コード全体の俯瞰 |
| 実際に動かして調べる | VizTracer、rr、Go の race detector、Loom、TLA+、Hypothesis | 人が読める説明文を作らない |

- **4 と 5 を、実際に動かした結果をもとに説明する製品は見つからなかった。**
- **LLM に推測させるだけでは精度が足りない。**
  - 実行結果を当てるベンチマークでは、最大 13 行の短い関数でも正答は 44〜63%（CRUXEval、REval）。
  - データ競合を見つけるベンチマークでは、最良のモデルでも F1 は 75%、ほとんどのモデルは 60% 未満（DRPBench、ICML 2026）。
- **自動の解説がレビューを良くするとは限らない。**
  - 自動生成のレビューコメントを付けても、重大な問題の検出は増えなかった。むしろコメントに引きずられる傾向（アンカリング）が見られた（ICSE 2025、29 人。論文本文は未確認）。
  - AI に概念を質問しながら進めた人は理解度テストで 65% 以上、コードの生成を丸ごと任せた人は 40% 未満だった（Anthropic、52 人の比較実験）。
- **ビューアで理解やレビューが良くなったことを確かめた比較実験は、どの道具についても見つからなかった。**

## 提案

1〜3 は既存の道具を組み合わせ、抜けている 4 と 5 だけを作る。

| 機能 | やり方 |
|---|---|
| 全体の俯瞰・ファイルツリー | CodeBoarding（ローカルで静的解析、MIT、8 言語、VS Code 拡張あり） |
| 行ごとの解説 | CodeTour の形式（ファイルの行範囲に説明を付ける）を使い、agent に書かせる |
| 差分 | crit（ローカルで動くレビュー UI、MIT） |
| 具体的な値・同時リクエスト | **新しく作る。** 以下の流れにする |

新しく作る部分の流れ：

1. agent が、対象のコードに対するシナリオを書く。普通の値、境界値（空、0、最大値、null、壊れた入力）、同時に 2 つ来た場合。
2. シナリオをテストとして実際に実行する（例：`go test -race`、Hypothesis）。
3. 実行結果を根拠にして解説ページを作る。一文ごとに「どのシナリオの結果に基づくか」を付け、推測だけの説明と区別する。

## 進め方

- 最初はビューアのアプリではなく、agent の skill として小さく作る。対象はエンドポイント 1 本か関数 1 つだけ。
- 実際のリポジトリで何度か使い、次の二つを確かめる。
  - 読む時間が減ったか
  - 見落としが減ったか
- 役に立つと分かったら、俯瞰の図やファイルツリーとつなぐビューアへ広げる。

## リスク

- 前例が無い。業界で確立したやり方ではなく、試しながら作ることになる。
- テストを実行するには環境が要る（DB、外部 API など）。その準備の手間が大きいかもしれない。
- 並行の問題は、実行しても起きなければ見つからない。Go の race detector も、実行中に実際に起きた競合しか検出しない。

## 決めること

- 最初に試すリポジトリと言語
- 対象にするエンドポイントか関数
- 「役に立った」をどう測るか

## 出典

- ProjectMind: https://github.com/Plaintext-Gmbh/projectmind
- CodeBoarding: https://github.com/CodeBoarding/CodeBoarding
- crit: https://github.com/tomasz-tomczyk/crit
- CodeTour: https://github.com/microsoft/codetour
- DeepWiki: https://docs.devin.ai/work-with-devin/deepwiki
- Google Code Wiki: https://developers.googleblog.com/introducing-code-wiki-accelerating-your-code-understanding/
- CodeRabbit Change Stack: https://docs.coderabbit.ai/pr-reviews/change-stack.md
- Go の race detector: https://go.dev/doc/articles/race_detector
- CRUXEval: https://arxiv.org/pdf/2401.03065
- DRPBench: https://icml.cc/virtual/2026/poster/66181
- Anthropic の比較実験: https://www.anthropic.com/research/AI-assistance-coding-skills
- ICSE 2025 のレビュー研究: https://www.inf.usi.ch/en/node/11023
- Comprehension Debt（Addy Osmani）: https://addyosmani.com/blog/comprehension-debt/
