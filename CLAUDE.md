# XmoQ

MoQ/WebTransportを使った1対多の配信と会議の実験。目的は、エージェントが配信や会議に参加すること。MoQ Transportの層をRustとTypeScriptで自前に実装し、版はdraft 18に固定している（理由は`docs/protocols.md`）。

## 着手前に読む

- `docs/agent-guide.md`: 実装エージェント向けの決まり。このファイルより優先する
- `docs/open-questions.md`: 触ろうとしている範囲に未決事項が無いかを確かめる
- `docs/architecture.md`: 構成要素の責務と境界。どこまでできているかは`docs/requirements.md`、次の作業は`docs/roadmap.md`
- 触る部品のREADMEと、`docs/moqt-implementation-notes.md`の該当の行

## コマンド

初回は`mise trust`と`mise install`。道具の版は`mise.toml`と`rust-toolchain.toml`で固定している。Rustのビルドには、miseが入れないCとC++のコンパイラが要る。

| コマンド | 何をするか |
|---|---|
| `mise run check` | CIと同じ検査の全部 |
| `mise run check:rust` | fmt、clippy、test |
| `mise run check:ts` | Biome、oxlint、型検査、`bun test` |
| `mise run check:layers`、`check:workflows`、`check:docs` | 層の依存。actionlintとshellcheck。生成した記録の索引と照合のファイルの一覧が古くないか |
| `mise run fmt` | 整形と安全な直し。記録の索引と照合のファイルの一覧も書き直す（記録か照合の台本を足したら走らせる） |
| `mise run interop` | `moqtail`の相手とつなぐ照合。`check`には入っていない |
| `mise run demo` | リレーとページを立て、Chromiumで配信と視聴のページを開く |
| `mise run watch -- <旗>` | 端末の視聴者 |
| `mise run agent -- --room <部屋のURL> --out <DIR>` | エージェントが部屋を読むCLI |
| `mise run agent-demo -- start --out <DIR>`、`stop --out <DIR>` | デモとエージェントと配信のページをまとめて立てて止める。`--meeting N`で会議 |
| `mise run transcriber -- <旗>`、`mise run speech-model` | 文字起こし役と、そのモデルの取得 |

旗と手順は各READMEと`README.md`のChecksにある。

## このプロジェクトの決まり

- 未決事項を実装の都合で決めない。`docs/open-questions.md`に載っている項目に当たったら、選択肢と根拠をそろえてユーザーに聞く
- 未決事項が決まったら、同じ変更で`docs/open-questions.md`の該当行を消し、`docs/requirements.md`のStatusを直す
- 決定の背景、却下した代替案、根拠は、その決定が効いている場所に書く。コードならWHYとWHY NOTのコメント、文書ならその話題を扱う文書（構成は`docs/architecture.md`、プロトコルと版は`docs/protocols.md`）に書く。ADRや決定一覧のように「意思決定」でまとめた置き場は作らない
- 検証は`docs/experiments/`に記録する。走らせていない検証を「通った」と書かない。失敗と阻害要因も同じ形式で残す
- MoQの仕様は、モデルの知識で書かずにdraft 18の本文（datatracker）を読んで実装する。本文から自明でない挙動には、コメントに節番号を書く。ライブラリの版も、その時点のリポジトリを読んで確かめる
- 自前の部品は、先に`moqtail`の相手とつないで確かめる。自前どうしだけで通っても、両側が同じ読み違いをしている場合を見つけられない
- draftが黙っている箇所と矛盾している箇所での動きは`docs/moqt-implementation-notes.md`の表にそろえ、片方の実装だけを変えない。表に行を足すときと裁定を出すときは、該当の節を本文で読んでから書く
- 状態を表す語は`docs/requirements.md`の定義にそろえる。図や文書で、未実装のものを実装済みに見える書き方にしない
- `docs/`配下とREADMEは英語、このファイルは日本語で書く

## 層と依存

規約は、文章で足す前に、破ると通らない形にできないかを考える。層の境界はcrateやパッケージの依存関係で固定し、lintとCIで検査する。CIは`mise run`の同じタスクを呼ぶので、検査を足すときはワークフローでなく`scripts/ci/`を直す。

- 符号化と状態遷移（`xmoq-moqt`、`xmoq-relay-core`、各パッケージの`src/core`）は入出力に依存させない。WebTransportのライブラリ（`web-transport-quinn`）に触るのは`xmoq-moqt-io`の1ファイルだけ、ブラウザの`WebTransport`に触るのはクライアントの側だけにする
- `Connection`の契約は、メモリ上の接続と実際のループバックの接続の両方に同じテストを当てて確かめる。テスト用の接続だけで通る挙動を前提にしない
- TypeScriptのパッケージは自分より下の層にだけ依存できる。層の表は`scripts/ci/check-layers.sh`の`package_layer`にあり、表に無いパッケージは検査で落ちる
- LOCの版に依る値（プロパティの番号、ペイロードの決まり）は、版の名前の場所（`packages/loc/src/draft-04/`、Rustは`xmoq-loc`の`draft_04`）の外に書かない
- 差し替えるかもしれない依存は1ファイルだけが触る: 映像のデコーダ（`crates/picture/src/video_decoder.rs`）、音の出力（`crates/watch/src/bin/xmoq-watch/audio_output.rs`）
- `xmoq-chat-tracks`と`xmoq-transcript-tracks`はCコンパイラを要らない形に保ち、`xmoq-watch`をSQLiteに依存させない
- whisper.cppのモデルはリポジトリに入れない（`mise run speech-model`がユーザーのキャッシュに取る）
- 本体（`crates/`、`packages/`）から`spikes/`、`demo/`、`interop/`、照合のためだけの`xmoq-quiet-peer`に依存させない。`spikes/`と`demo/`はlintと型検査の対象外。`interop/`は対象で、`interop/peers/`は`spikes/moqtail/`の写し（`moqtail`のライブラリで書いた自前の相手役）で、spikeとの差分を読めるように整形もlintもしない。自前のコードなのでレビューの対象に入る
- 断りや終わりの後にいつ頼み直すかの規則は、Rustは`xmoq-again`、TypeScriptは`@xmoq/moqt`（`createAskAgain`、`createNextSession`、GOAWAYで移るループの`keepMoving`）にある。層0のパッケージどうしは依存できないので、TypeScriptの側は`@xmoq/moqt`に置く。表は`fixtures/ask-again/`で、両方のテストが読む。クライアントごとに書き直さず、これを使う
- 部屋のトークンの形は`xmoq-room-token`にあり、部屋（`spikes/durable-objects-room/worker.js`）がJavaScriptで同じ形を別に書いている。変えるときは両方を直す

## 実装の決まり

- RustとTypeScriptの実装は、名前をdraft本文の用語に合わせ、`fixtures/moqt-18/`の同じ実バイト列をテストで読む。draftの外の契約（セッションを閉じるときの理由の切り方など）も、2つの実装が満たすなら`fixtures/`に共有の固定値を置き、両方のテストで読む。読んで比べるだけで「同じ」としない
- テストの固定値は`moqtail`から採った実バイト列にする。自分で符号化したものを自分で復号するだけのテストは、照合にならない
- 受け取る側のバッファとキューには上限を置く。遅いsubscriber、キャンセル、切断の後始末を書く
- lintを`#[allow]`や設定で黙らせるときは理由を書く。黙らせるより直すほうを先に考える
- `cfg`でOSを分けたコードは、手元のOSでコンパイルされない側がlintも型検査も受けない。書いたら報告にそう書き、CIの結果を見る
- 偽の接続や偽のトランスポートを使うテストは、偽物が実物より寛容だと誤りを通す。ブラウザやライブラリのAPIの呼び方は、実物で1回確かめ、偽物を実物が拒むものを拒む形にする

## 開発の流れ

本体のコード（`crates/`、`packages/`）は、次の順で`main`に入れる。

1. 実装する。終わったと報告する前に`mise run check`を実際に走らせ、その出力を報告に含める。通っていない状態で終わったと報告しない。`crates/`、`packages/`、`interop/`、`fixtures/`を変えたら`mise run interop`も走らせる
2. 実装者とは別のレビュアーがコードを読む（`/review-team-xmoq`）。レビュアーには実装者の報告を渡さず、差分だけを渡す
3. 指摘は実装者が直す。直したことは差分と検査の再実行で確かめる
4. ブランチをPRにする。CIが通ってからマージする。`main`へ直接pushしない

照合の場面と走らせ方は`interop/README.md`にある。場面はリレーのログの行を待って進むので、その行の書式を変えると落ちる（`interop/disconnect.ts`の冒頭）。同じマシンで2つの作業コピーから同時に走らせるときは、`--base-port`を分ける。

## Documentation Map

プロジェクトで維持すべきドキュメントと、変更種別ごとの更新要件。`/setup-docs`で生成・更新できる。

### ドキュメント一覧

| ファイル/ディレクトリ | 種別 | 目的 |
|---|---|---|
| README.md | 入り口 | 概要、文書と部品の一覧、検査 |
| CLAUDE.md、docs/agent-guide.md | エージェントへの指示 | 作業の決まりとコマンド。agent-guideが優先する |
| docs/vision.md | 仕様 | 目指す方向と範囲外 |
| docs/requirements.md | 仕様 | 要件ごとの状態と、状態を表す語の定義 |
| docs/architecture.md、docs/assets/ | 仕様 | 構成要素の責務と境界。図 |
| docs/protocols.md | 仕様 | プロトコルの層、版、一次仕様へのリンク |
| docs/moqt-implementation-notes.md | 仕様 | draftが黙っている箇所と矛盾している箇所で2つの実装がそろえる挙動。既知の不足 |
| docs/open-questions.md | 仕様 | 未決事項と、仮の値の置き場所 |
| docs/scale-and-meetings.md | 設計の案 | 配信をリレーで広げる形と、会議の形（proposed） |
| docs/roadmap.md | 計画 | Phaseごとの作業と終了条件 |
| docs/validation.md、docs/experiments/ | 記録 | 検証の観点と、走らせた記録 |
| docs/av-demo.md | 記録のまとめ | デモの映像と音で測れたことと限界。数字は記録から取る |
| docs/moq-libraries.md | 記録 | Phase 0のライブラリ調査（終了済み） |
| crates/*/README.md、packages/*/README.md、demo/README.md | 仕様 | 部品の範囲、旗、既定値、既知の不足 |
| fixtures/*/README.md | 仕様 | 2つ以上の実装がテストで読む固定値と表の出どころ |
| interop/README.md、interop/peers/*/README.md | 手順 | 照合の相手の用意と走らせ方 |
| .github/workflows/、scripts/ci/、mise.toml | 検査 | CIとローカルの検査。規約を検査で守る |
| .claude/agents/、.claude/skills/ | エージェントへの指示 | レビュアーと`/review-team-xmoq`。報告は`.claude/tmp/reviews/`に書かれ、コミットされない |

### 変更 → ドキュメント更新マッピング

| 変更の種類 | 更新が必要なドキュメント |
|---|---|
| 未決事項を決めた | docs/open-questions.md、docs/requirements.md、その話題を扱う文書かコードのコメント |
| ライブラリやMoQ draftの版を選んだ・変えた | docs/protocols.md |
| 検証を実行した（成功・失敗とも） | docs/experiments/ に追加。デモの映像と音に関わる結果なら docs/av-demo.md も |
| roadmapの項目を終えた | docs/roadmap.md のチェックボックス、docs/requirements.md |
| 構成要素の責務や接続を変えた | docs/architecture.md、docs/assets/architecture.svg、docs/assets/broadcast-flow.svg |
| プロトコルの層や運ぶものを変えた | docs/protocols.md、docs/assets/protocol-layers.svg |
| 範囲を広げた・狭めた | docs/vision.md、docs/requirements.md、docs/roadmap.md |
| ビルド・テスト・起動のコマンドを足した・変えた | CLAUDE.md、README.md |
| 部品（crate、パッケージ）や共有の固定値を足した | そのREADME、README.md の一覧 |
| 新しい文書を足した | README.md の一覧、このDocumentation Map |
| エージェントの進め方を変えた | docs/agent-guide.md、CLAUDE.md |
| draftの解釈や、本文に無い挙動を決めた・変えた | docs/moqt-implementation-notes.md。RustとTypeScriptの両方を同じPRで直す |
| 受信の上限やタイムアウトの値を決めた・変えた | 値を置くコードの定数のコメント（provisionalの印を含む）、docs/moqt-implementation-notes.md |
| moqtailから固定値を採った・採り直した | fixtures/moqt-18/README.md |
| ブラウザやライブラリから固定値を採った・採り直した | その固定値のディレクトリのREADME（fixtures/webtransport-close/README.md など） |
