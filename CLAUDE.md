# XmoQ

MoQ/WebTransportを使った1対多の配信と会議の実験。目的は、エージェントが配信や会議に参加すること。MoQ Transportの層をRustとTypeScriptで自前に実装し、版はdraft 18に固定している（理由は`docs/protocols.md`）。

## 着手前に読む

- `docs/agent-guide.md`: このファイルに無い、実装エージェント向けの決まり（英語）
- `docs/open-questions.md`: 触ろうとしている範囲に未決事項が無いかを確かめる
- `docs/architecture.md`: 構成要素の責務と境界。どこまでできているかは`docs/requirements.md`、次の作業は`docs/roadmap.md`
- 触る部品のREADMEと、`docs/moqt-implementation-notes.md`の該当の行

## コマンド

初回は`mise trust`と`mise install`。道具の版は`mise.toml`と`rust-toolchain.toml`で固定している。Rustのビルドには、miseが入れないCとC++のコンパイラが要る。

| コマンド | いつ走らせるか |
|---|---|
| `mise run check` | CIと同じ検査の全部。全体はPRのCIが走らせるので、ローカルでは走らせなくてよい |
| `mise run fmt` | 整形した後と、記録か照合の台本を足した後（生成する索引も書き直す） |
| `mise run interop` | `check`には入っていない。全体はPRのCIが走らせる。ローカルでは触った部品の場面を`--only`で走らせる |

タスクの一覧と説明は`mise tasks`が出す。説明は`mise.toml`のdescriptionにだけ書き、ほかの文書に写さない。旗と既定値は各プログラムの`USAGE`（無ければ`Args`のdoc）、旗の組み合わせと手順は各部品のREADMEにある。

## このプロジェクトの決まり

- 未決事項を実装の都合で決めない。`docs/open-questions.md`に載っている項目に当たったら、選択肢と根拠をそろえてユーザーに聞く
- 製品の要件、性能の目標、方針を作らない。認証、ログ、保存期間、デプロイ、ライセンスを黙って決めない
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
- `xmoq-chat-tracks`、`xmoq-transcript-tracks`、`xmoq-meeting-tracks`、`xmoq-viewer`はCコンパイラを要らない形に保ち、`xmoq-watch`をSQLiteに依存させない（`check:layers`が依存の木で検査する）
- whisper.cppのモデルはリポジトリに入れない（`mise run speech-model`がユーザーのキャッシュに取る）
- 本体（`crates/`、`packages/`）から`spikes/`、`demo/`、`interop/`、照合のためだけの`xmoq-quiet-peer`に依存させない（`xmoq-quiet-peer`は`check:layers`が検査する）。`spikes/`はlintと型検査の対象外。`demo/`と`interop/`は対象で、`interop/peers/`は`spikes/moqtail/`の写し（`moqtail`のライブラリで書いた自前の相手役）で、spikeとの差分を読めるように整形もlintもしない。自前のコードなのでレビューの対象に入る
- 断りや終わりの後にいつ頼み直すかの規則は、Rustは`xmoq-again`、TypeScriptは`@xmoq/moqt`（`createAskAgain`、`createNextSession`、GOAWAYで移るループの`keepMoving`）にある。層0のパッケージどうしは依存できないので、TypeScriptの側は`@xmoq/moqt`に置く。表は`fixtures/ask-again/`で、両方のテストが読む。クライアントごとに書き直さず、これを使う
- 部屋のトークンの形は、Rustは`xmoq-room-token`、TypeScriptは`@xmoq/room`の`src/token.ts`にある。表は`fixtures/room-token/`で、両方のテストが読む。部屋は`packages/room`にあり、Miniflareで動かす（`node packages/room/run.ts`）

## 実装の決まり

- RustとTypeScriptの実装は、名前をdraft本文の用語に合わせ、`fixtures/moqt-18/`の同じ実バイト列をテストで読む。draftの外の契約（セッションを閉じるときの理由の切り方など）も、2つの実装が満たすなら`fixtures/`に共有の固定値を置き、両方のテストで読む。読んで比べるだけで「同じ」としない
- テストの固定値は`moqtail`から採った実バイト列にする。自分で符号化したものを自分で復号するだけのテストは、照合にならない
- 受け取る側のバッファとキューには上限を置く。遅いsubscriber、キャンセル、切断の後始末を書く
- lintを`#[allow]`や設定で黙らせるときは理由を書く。黙らせるより直すほうを先に考える
- `cfg`でOSを分けたコードは、手元のOSでコンパイルされない側がlintも型検査も受けない。書いたら報告にそう書き、CIの結果を見る
- 偽の接続や偽のトランスポートを使うテストは、偽物が実物より寛容だと誤りを通す。ブラウザやライブラリのAPIの呼び方は、実物で1回確かめ、偽物を実物が拒むものを拒む形にする

## 開発の流れ

本体のコード（`crates/`、`packages/`）は、次の順で`main`に入れる。

1. 実装する。終わったと報告する前に、触った範囲の検査を実際に走らせ、その出力を報告に含める。触ったcrateの`cargo test -p`と`cargo clippy -p`、触ったパッケージの`bun test`と型検査、照合は触った部品の場面を`--only`で。通っていない状態で終わったと報告しない。検査の全体（`mise run check`と照合の全体）はPRのCIが走らせ、CIが通ってからマージする。WHY: 全体をローカルとCIで2度走らせない。照合の全体は1回10分かかり、同じマシンでは1つずつしか走らせられない
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
| CLAUDE.md、docs/agent-guide.md | エージェントへの指示 | 作業の決まりとコマンド。agent-guideはこのファイルに無い分だけを持つ |
| docs/vision.md | 仕様 | 目指す方向と範囲外 |
| docs/requirements.md | 仕様 | 要件ごとの状態と、状態を表す語の定義 |
| docs/architecture.md、docs/assets/ | 仕様 | 構成要素の責務と境界。図 |
| docs/glossary.md | 仕様 | 配信と会議の語。MoQの語はdraftを指すだけ |
| docs/protocols.md | 仕様 | プロトコルの層、版、一次仕様へのリンク |
| docs/moqt-state-models.md | 仕様 | セッションと要求の状態の移り方。draft 18の状態に節番号を付け、draftが黙っている所は覚え書きの行を名前で指す |
| docs/moqt-implementation-notes.md | 仕様 | draftが黙っている箇所と矛盾している箇所で2つの実装がそろえる挙動。既知の不足 |
| docs/open-questions.md | 仕様 | 未決事項と、仮の値の置き場所 |
| docs/scale-and-meetings.md | 設計の案 | 配信をリレーで広げる形と、会議の形（proposed） |
| docs/roadmap.md | 計画 | Phaseごとの作業と終了条件 |
| docs/validation.md、docs/experiments/ | 記録 | 検証の観点と、走らせた記録 |
| docs/av-demo.md | 記録のまとめ | デモの映像と音で測れたことと限界。数字は記録から取る |
| docs/moq-libraries.md | 記録 | Phase 0のライブラリ調査（終了済み） |
| crates/*/README.md、packages/*/README.md、demo/README.md | 仕様 | 部品の範囲、旗の組み合わせ、旗に閉じない上限、既知の不足。プログラムの旗と既定値は`USAGE`（無ければ`Args`のdoc）にだけ書く。READMEを持つのは、走らせ方、ビルド要件とライセンス、複数の実装が守る契約（共有の固定値の表）のどれかを持つ部品で、ほかはcrateなら`lib.rs`の`//!`に書く |
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
| ビルド・テスト・起動のコマンドを足した・変えた | `mise.toml`のdescription。検査のタスクなら CLAUDE.md と README.md の表も |
| 部品（crate、パッケージ）や共有の固定値を足した | そのREADME、README.md の一覧 |
| 新しい文書を足した | README.md の一覧、このDocumentation Map |
| エージェントの進め方を変えた | docs/agent-guide.md、CLAUDE.md |
| 既知の不足を直した（draftの要件を満たした、上限を足した） | docs/moqt-implementation-notes.md の既知の不足の項目と、それを指す`FIXME:`を消す。その挙動を表の行に書く。部品のREADMEの既知の不足とStatus |
| draftの解釈や、本文に無い挙動を決めた・変えた | docs/moqt-implementation-notes.md。RustとTypeScriptの両方を同じPRで直す。状態の移り方が変わるならdocs/moqt-state-models.mdも |
| 受信の上限やタイムアウトの値を決めた・変えた | 値を置くコードの定数のコメント（provisionalの印を含む）、docs/moqt-implementation-notes.md |
| moqtailから固定値を採った・採り直した | fixtures/moqt-18/README.md |
| ブラウザやライブラリから固定値を採った・採り直した | その固定値のディレクトリのREADME（fixtures/webtransport-close/README.md など） |
