# XmoQ

MoQ/WebTransportを使った1対多ライブ配信の実験プロジェクト。構成はブラウザ（TypeScript、WebCodecs）、Rustリレー、Rust CLI（Ghostty）、Cloudflare Durable Objects。

## 現状

Phase 0（既存ライブラリでのObject転送）は2026-10-10に通った。検証用のコードは`spikes/`にあり、アプリ本体の構成ではない。

いまの課題は`docs/roadmap.md`のPhase 0bで、MoQ Transportの層をRustとTypeScriptで自前実装する。版はdraft 18に固定する。理由と範囲は`docs/protocols.md`にある。これを通るまで映像などの機能を作らない。

ビルド、テスト、lintのコマンドはまだ無い。最初のコードを足したPRで、この節にコマンドを書く。

## 着手前に読む

`docs/agent-guide.md`が実装エージェント向けの決まりで、このファイルより優先する。あわせて`docs/open-questions.md`を読み、触ろうとしている範囲に未決事項が無いかを確かめる。

## このプロジェクトの決まり

- 未決事項を実装の都合で決めない。`docs/open-questions.md`に載っている項目に当たったら、選択肢と根拠をそろえてユーザーに聞く
- 未決事項が決まったら、同じ変更で`docs/open-questions.md`の該当行を消し、`docs/requirements.md`のStatusを直す
- 決定の背景、却下した代替案、根拠は、その決定が効いている場所に書く。コードならWHYとWHY NOTのコメント、文書ならその話題を扱う文書（構成は`docs/architecture.md`、プロトコルと版は`docs/protocols.md`）に書く。ADRや決定一覧のように「意思決定」でまとめた置き場は作らない
- 検証は`docs/experiments/`に記録する。走らせていない検証を「通った」と書かない。失敗と阻害要因も同じ形式で残す
- MoQの仕様は、モデルの知識で書かずにdraft 18の本文（datatracker）を読んで実装する。本文から自明でない挙動には、コメントに節番号を書く。ライブラリの版も、その時点のリポジトリを読んで確かめる
- 自前の部品は、先に`moqtail`の相手とつないで確かめる。自前どうしだけで通っても、両側が同じ読み違いをしている場合を見つけられない
- 状態を表す語は`docs/requirements.md`の定義にそろえる（discussed、proposed、undecided、validation required、implemented）。図や文書で、未実装のものを実装済みに見える書き方にしない
- `docs/`配下とREADMEは英語、このファイルは日本語で書く

## Documentation Map

プロジェクトで維持すべきドキュメントと、変更種別ごとの更新要件。
`/setup-docs` で生成・更新できる。

### ドキュメント一覧

| ファイル/ディレクトリ | 種別 | 目的 |
|---|---|---|
| README.md | README | 概要と文書への入り口 |
| CLAUDE.md | コーディングポリシー | Claude向けの決まりとコマンド |
| docs/agent-guide.md | コーディングポリシー | 実装エージェント向けの決まりと最初の課題 |
| docs/vision.md | 仕様書 | 目指す方向と範囲外 |
| docs/requirements.md | 仕様書 | 要件ごとの状態 |
| docs/architecture.md | 仕様書 | 構成要素の責務と境界 |
| docs/protocols.md | 仕様書 | プロトコルの層と一次仕様へのリンク |
| docs/roadmap.md | 仕様書 | Phaseごとの作業と終了条件 |
| docs/validation.md | 仕様書 | 検証する観点と記録する項目 |
| docs/open-questions.md | 仕様書 | 未決事項の一覧 |
| docs/experiments/ | 仕様書 | 検証の実行記録 |
| docs/assets/ | README | 文書に埋め込むSVG図 |

### 変更 → ドキュメント更新マッピング

| 変更の種類 | 更新が必要なドキュメント |
|---|---|
| 未決事項を決めた | docs/open-questions.md、docs/requirements.md、その話題を扱う文書かコードのコメント |
| ライブラリやMoQ draftの版を選んだ・変えた | docs/protocols.md |
| 検証を実行した（成功・失敗とも） | docs/experiments/ に追加 |
| roadmapの項目を終えた | docs/roadmap.md のチェックボックス、docs/requirements.md |
| 構成要素の責務や接続を変えた | docs/architecture.md、docs/assets/architecture.svg、docs/assets/broadcast-flow.svg |
| プロトコルの層や運ぶものを変えた | docs/protocols.md、docs/assets/protocol-layers.svg |
| 範囲を広げた・狭めた | docs/vision.md、docs/requirements.md、docs/roadmap.md |
| ビルド・テスト・起動のコマンドを足した・変えた | CLAUDE.md、README.md |
| 新しい文書を足した | README.md の一覧、このDocumentation Map |
| エージェントの進め方を変えた | docs/agent-guide.md、CLAUDE.md |
