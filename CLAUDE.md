# XmoQ

MoQ/WebTransportを使った1対多ライブ配信の実験プロジェクト。構成はブラウザ（TypeScript、WebCodecs）、Rustリレー、Rust CLI（Ghostty）、Cloudflare Durable Objects。

## 現状

Phase 0（既存ライブラリでのObject転送）は2026-10-10に通った。検証用のコードは`spikes/`にあり、アプリ本体の構成ではない。

Phase 0b（MoQ Transportの層をRustとTypeScriptで自前実装する）の終了条件にも、2026-10-10に届いた。自前のブラウザクライアント、Rustクライアント、Rustリレーのあいだで、Objectが1つ通る。版はdraft 18に固定している。理由と範囲は`docs/protocols.md`にある。

Phase 0bの項目は全部終わっている。Phase 1（映像）の終了条件には、2026-10-11にデモ（`mise run demo`）で届いた。Phase 2（音声と品質）からPhase 4（部屋）は、それぞれ最初の版がデモか`spikes/`で動いていて、どれも終わっていない。FETCH、MSFのカタログ、LOCの詰め方、映像の受け手と送り手の中核、映像のトラックを出す部品、ブラウザのエンコーダとデコーダにつなぐ部品、リレーのキャッシュ（トラックごとに最新のGroup）が入っていて、ページが符号化した映像を別のページがリレーを通して再生するところまで、照合で通っている。音声（Opus）も同じ形で通る。再生のバッファ、映像と音の同期、つなぎ直しは`@xmoq/media`の部品で、絵を画面に描く所だけが`demo/`にある。映像はGroupを1本のストリームで送り、LOCはLOC-04の番号をdraft 18の上で使う。どこまでできているかは`docs/roadmap.md`のチェックで見る。リレーはWebTransportだけを受ける。認可は、鍵を渡したとき（`--auth-key-hex`）だけ、部屋のトークンを要求ごととSETUPで確かめる。キャッシュは、配信元がいるあいだだけ持つ。draftに届いていない所は`docs/moqt-implementation-notes.md`の既知の不足にある。

Rustのcrateは次のように分かれている。`xmoq-moqt`（符号化とセッションの状態）、`xmoq-relay-core`（リレーの経路と配信の論理）は入出力に依存しない。`xmoq-moqt-io`が`Connection`トレイトの上でストリームを読み書きし、WebTransportのライブラリに触るのはこのcrateの1ファイルだけ。`xmoq-relay`と`xmoq-moqt-client`がそれらをつなぐ。`xmoq-loc`（LOCの読み取り。入出力に依存せず、版に依る部分はTypeScriptと同じく版の名前のモジュール`draft_04`に置く）を、クライアントが映像のトラックをAnnex Bのファイルに書き出すのに使う（`--annex-b-out`）。`xmoq-watch`（`crates/watch`）は端末の視聴者で、1つのセッションで映像と音のトラックを受け、Kitty Graphicsの絵と機械の音の出力に、LOCのTimestampの時計で出す。部屋につなぐと、配信の場所とトークンを部屋から受け、チャットを絵の下に出す。チャットは集め役のトラックだけで送り受けし、部屋のWebSocketのチャットは使わない（2026-10-24に決定）。判断する部分（出番の時刻、音の出す順、縮める計算、端末へ書くバイト列、トラック選び、キー、部屋のメッセージ、旗）は入出力に依存しないライブラリで、テストはそこにある。入出力はバイナリ（`src/bin/xmoq-watch/`）の側で、種類ごとに1ファイル。映像のデコーダ、絵を縮める計算、PNGへの書き出しは`xmoq-picture`（`crates/picture`）にあり、`xmoq-watch`と`xmoq-agent`が使う。`xmoq-picture`は入出力に依存せず、頼まれたときだけ復号してPNGを返す（`Frames`）。映像のデコーダ（`crates/picture/src/video_decoder.rs`、`openh264`。2026-10-13に決定）と音の出力（`audio_output.rs`、`cpal`。未決）は、差し替えられるように、それぞれそのファイルだけが触る。Unix専用（端末の設定に`termios`を使う）で、macOSでは走らせ、LinuxではCIの照合が音を出さずに走らせる。LinuxのビルドにはALSAのヘッダが要る（`scripts/ci/check-rust.sh`がCIで入れる）。起動は`mise run watch -- <旗>`。文字起こし役の起動は`mise run transcriber -- <旗>`。エージェントが部屋を読むには`mise run agent -- --room <部屋のURL> --out <DIR>`（別のポートでデモを立てる手順が`crates/agent/README.md`の頭にある）。デモ、エージェント、配信のページをまとめて立てて止めるには`mise run agent-demo -- start --out <DIR>`と`mise run agent-demo -- stop --out <DIR>`（`scripts/agent-demo.sh`。起こしたプロセスだけを`<DIR>/pids`で止める）。`xmoq-agent`（`crates/agent`）はエージェント（Claude）が配信を読むためのCLIで、部屋につなぎ、チャット、参加者の出入り、配信の始まりと終わり、頼まれたとき（標準入力の`/frame`）の映像の絵のPNGを、1行1つのJSONで標準出力と`<DIR>/events.jsonl`に書き、標準入力の行をチャットに言う。`xmoq-watch`と`xmoq-agent`が共有する部屋とセッションの部分は、入出力に依存しない`xmoq-viewer`（`crates/viewer`）と、入出力のある`xmoq-viewer-io`（`crates/viewer-io`）にある。`xmoq-chat`（`crates/chat`）はチャットの集め役で、リレーにつながり、参加者の行を受けた順に番号を振って部屋ごとのトラックに出し直し、SQLiteに保存する。トラックの名前とペイロードの形は`xmoq-chat-tracks`（`crates/chat-tracks`）にあり、`xmoq-watch`もそれを使う。`xmoq-chat`はrusqliteの`bundled`でSQLiteをCからビルドするので、Cコンパイラが要る（macOSはXcodeのコマンドラインツールのclang、Linuxはgcc）。`xmoq-chat-tracks`はCコンパイラを要らない形に保ち、`xmoq-watch`はSQLiteに依存させない（`xmoq-watch`は`openh264`をビルドするのにもともとC++コンパイラが要る）。部屋のトークンの形（作る所、照らす所、名前空間の`*`の照らし方）は`xmoq-room-token`（`crates/room-token`）にあり、リレーと集め役が使う。部屋（`spikes/durable-objects-room/worker.js`）も同じ形でトークンを作るが、JavaScriptで別に書いてある。断りや終わりの後にいつ頼み直すかの規則は`xmoq-again`（`crates/again`。入出力も依存も無い）にあり、`xmoq-watch`と`xmoq-chat`と`xmoq-transcriber`が使う。自分のトラックを出してFETCHに答えるクライアントの部品（要求、Groupごとのストリーム、FETCHの答えを書くタスク、FETCHで何を返すか、つなぎ直しの待ち時間）は`xmoq-track-serving`（`crates/track-serving`）にあり、`xmoq-chat`と`xmoq-transcriber`が使う。`xmoq-transcriber`（`crates/transcriber`）は文字起こし役で、集め役と同じくリレーにつながり、prefixの下に告知された配信の音のトラックをwhisper.cppで文字にして、prefixごとのトラック`speech`（`xmoq/transcript/<prefix>`）に出す（2026-10-25に決定）。agentもほかの視聴者も、そのトラックを購読して読む。音を切って読む部品は`xmoq-speech`（`crates/speech`）、トラックの名前とペイロードの形は`xmoq-transcript-tracks`（`crates/transcript-tracks`。Cコンパイラを要らない形に保つ）にある。whisper.cppのビルドにはC++コンパイラとcmakeが要り、cmakeは`mise.toml`で固定している。モデルはリポジトリに入れず、`mise run speech-model`がユーザーのキャッシュ（`~/.cache/xmoq/whisper/`）に取る。`Connection`の契約は、メモリ上の接続と実際のループバックの接続の両方に同じテストを当てて確かめる。テスト用の接続だけで通る挙動を前提にしない。

TypeScriptのパッケージは層になっている。`@xmoq/moqt`（MoQ Transport）と`@xmoq/loc`（LOCの詰め方）と`@xmoq/json`（トラックのペイロードをserde_jsonと同じに読むJSONの読み手）が層0、`@xmoq/msf`（MSFのカタログ）と`@xmoq/chat`（チャットのトラックの参加者）と`@xmoq/transcript`（文字起こしのトラックの読み手）が層1、`@xmoq/media`（映像のトラックの送り受け）が層2で、自分より下の層にだけ依存できる。層の表は`scripts/ci/check-layers.sh`の`package_layer`にあり、表に無いパッケージは検査で落ちる。各パッケージの中核（`src/core`）は入出力に依存しない。LOCは、版に依る部分（プロパティの番号、ペイロードの決まり）を版の名前のフォルダ（`packages/loc/src/draft-04/`）に置き、切り替えを`packages/loc/src/index.ts`の1行にしてある。版に依る値を、そのフォルダの外に書かない。

draftが黙っている箇所と矛盾している箇所で2つの実装がどう動くかは、`docs/moqt-implementation-notes.md`に表がある。片方の実装だけを変えない。表に行を足すときと裁定を出すときは、該当の節を本文で読んでから書く。

検査は`mise run check`で走らせる。道具の版は`mise.toml`と`rust-toolchain.toml`で固定してあり（Bun、Node、Biome、oxlintなど。照合の相手の`moqlivemock`をビルドするGoも）、初回は`mise trust`と`mise install`が要る。個別には`mise run check:rust`（fmt、clippy、test）、`check:ts`（Biome、oxlint、型検査、`bun test`）、`check:layers`（層の依存）、`check:workflows`（actionlint、shellcheck）があり、整形は`mise run fmt`。CIは同じタスクを呼ぶので、検査を足すときはワークフローでなく`scripts/ci/`を直す。

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

## 開発の流れ

本体のコード（`crates/`、`packages/`）は、次の順で`main`に入れる。`spikes/`は検証用で、lintとレビューの対象外にする。本体から`spikes/`に依存させない。

`interop/`は、自前の部品を`moqtail`の相手とつないで確かめるスクリプトの置き場で、lintと型検査の対象にする。`interop/peers/`は`moqtail`のコードの写しなので、整形もlintもしない（元との差分を読めるようにするため）。`interop/`は`packages/`に依存してよいが、逆は禁止する。

照合は`mise run interop`で走らせる。自前と`moqtail`のクライアントの全部の組み合わせで、`moqtail`のリレーと自前のリレーのそれぞれを通してObjectを1つ運び、実物のChromiumで確かめる。FETCH（StandaloneとJoining）の組と、MSFのカタログを`moqlivemock`の配信元と視聴者（`interop/peers/moqlivemock/`。Goでビルドする）とのあいだで読み書きする組もある。加えて、publisherやsubscriberが転送の途中で去る場面（kill、取り消し、読まないsubscriber）を、自前のリレーを通して走らせる。リレーが止まって同じポートで戻り、ページがつなぎ直す場面、答えの来ない要求を購読者が時間切れで取り消す場面、SETUPも答えも送らない相手（`xmoq-quiet-peer`、`crates/quiet-peer`。照合のためだけのcrateで、本体から依存させない）に対してクライアントがSETUP、PUBLISH、FETCHを諦める場面、購読者が購読の途中でREQUEST_UPDATEで配送の期限を変える場面、視聴者が配信より先に来てRENDEZVOUS_TIMEOUTで待つ場面（カタログの読み手が先に来る場面を含む）、端末の視聴者（`xmoq-watch`）が配信を視聴する場面、エージェントの読み手（`xmoq-agent`）が配信の絵を頼み、会議（`--meeting`）の参加者を知ってその人の絵を頼む場面、GOAWAY（`xmoq-quiet-peer`が送るものと、`moqtail`のリレーと自前のリレーがSIGTERMで送るもの）を受けたクライアントが移る場面、相手が誤ったRequest IDで要求を開く場面、SUBSCRIBE_NAMESPACEで名前空間の告知を受ける場面、TRACK_STATUSで両方のリレーにトラックの様子を聞く場面、自前のリレーを別のリレーの下（edge）に立てて、その越しに購読する場面（上のリレーが止まって戻るあいだも再生が続く場面と、配信より先に来たSUBSCRIBEを2台のリレーが保留する場面と、チャットの集め役が上のリレー、参加者と視聴者が下のリレーにいて、下のリレーが告知を上へ渡す場面、2台のリレーが両方トークンを求める場面、チャットのページが下のリレーにいて集め役と2台が同じ鍵を持つ場面を含む）、チャットの集め役（`xmoq-chat`）を通して参加者がトラックで行を交わし、途中から来た参加者が履歴を受ける場面、文字起こし役（`xmoq-transcriber`）がページの声を読んで文字起こしのトラックに出す場面もある（いちばん小さいモデルを`~/.cache/xmoq/whisper/`に取って使う）。場面は、リレーのログの行を待って進むので、その行の書式を変えると落ちる（`interop/disconnect.ts`の冒頭）。`mise run check`には入っていない。同じマシンで2つの作業コピーから同時に走らせるときは、`--base-port`を分ける。CIでは`interop`ジョブが同じタスクを呼び、照合が依存するパスに触れていないPRでは、skipと表示して終わる。`crates/`、`packages/`、`interop/`、`fixtures/`を変えたら、報告の前に`mise run interop`も走らせる。

人がブラウザで見るためのデモは`mise run demo`で立てる（`demo/`。リレーとページのサーバーが立ち、Chromiumで配信のページと視聴のページを開く）。チャットの集め役（`xmoq-chat`）も立ち、ページはチャットをトラックだけで送り受けする（部屋のWebSocketはチャットを運ばない）。`--auth`のデモでは、集め役にリレーと同じ鍵を渡し、集め役が自分でトークンを作る（デモだけの形）。ページは部屋から、自分の名前空間の配信のトークンと部屋の購読のトークンの2つを受け、要求ごとに付ける。文字起こし役（`xmoq-transcriber`、releaseのビルド）も、ビルドがありモデルがキャッシュにあれば立ち（無ければ取り方を出して字幕無しで続ける）、視聴のページはその文字起こしを`@xmoq/transcript`で読んで絵の下に字幕として出す。読む言語は`--transcribe-language`（既定は`auto`）。配信のページは`?audio=mic`でなければ試験音を流し、試験音は声と見なされないので字幕は出ない。`demo/`はlintと型検査の対象外で、本体から依存させない。

1. 実装する。終わったと報告する前に`mise run check`を実際に走らせ、その出力を報告に含める。通っていない状態で終わったと報告しない
2. 実装者とは別のレビュアーがコードを読む（`/review-team-xmoq`）。レビュアーには実装者の報告を渡さず、差分だけを渡す
3. 指摘は実装者が直す。直したことは差分と検査の再実行で確かめる
4. ブランチをPRにする。CIが通ってからマージする。`main`へ直接pushしない

規約は、文章で足す前に、破ると通らない形にできないかを考える。層の境界はcrateやパッケージの依存関係で固定し、lintとCIで検査する。

- 符号化と状態遷移（`xmoq-moqt`、`@xmoq/moqt`の中核）は入出力に依存させない。WebTransportのライブラリ（Rustは`web-transport-quinn`）やブラウザの`WebTransport`に触るのは、クライアントとリレーの側だけにする
- RustとTypeScriptの実装は、名前をdraft本文の用語に合わせ、`fixtures/moqt-18/`の同じ実バイト列をテストで読む。draftの外の契約（セッションを閉じるときの理由の切り方など）も、2つの実装が満たすなら`fixtures/`に共有の固定値を置き、両方のテストで読む。読んで比べるだけで「同じ」としない
- テストの固定値は`moqtail`から採った実バイト列にする。自分で符号化したものを自分で復号するだけのテストは、照合にならない
- 受け取る側のバッファとキューには上限を置く。遅いsubscriber、キャンセル、切断の後始末を書く
- lintを`#[allow]`や設定で黙らせるときは理由を書く。黙らせるより直すほうを先に考える
- `cfg`でOSを分けたコードは、手元のOSでコンパイルされない側がlintも型検査も受けない。書いたら報告にそう書き、CIの結果を見る
- 偽の接続や偽のトランスポートを使うテストは、偽物が実物より寛容だと誤りを通す。ブラウザやライブラリのAPIの呼び方は、実物で1回確かめ、偽物を実物が拒むものを拒む形にする

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
| docs/av-demo.md | 仕様書 | 映像と音のデモで測れたことと限界のまとめ。数字は`docs/experiments/`の記録から取る |
| docs/assets/ | README | 文書に埋め込むSVG図 |
| docs/moq-libraries.md | 仕様書 | ライブラリ調査の記録（終了済み） |
| docs/moqt-implementation-notes.md | 仕様書 | draftが黙っている箇所と矛盾している箇所で、2つの実装がそろえる挙動。既知の不足 |
| docs/scale-and-meetings.md | 仕様書 | 1つの配信をリレーを増やして10,000人に広げる形と、49人までの会議の形の設計の案（proposed） |
| fixtures/moqt-18/README.md | 仕様書 | moqtailから採った実バイト列の出どころと復号後の値 |
| fixtures/webtransport-close/README.md | 仕様書 | セッションを閉じるときの理由の切り方。Chromiumが送った実バイト列の出どころ |
| fixtures/moqt-18-by-hand/README.md | 仕様書 | draft 18の本文から手で書いたバイト列。相手から採ったもの（`fixtures/moqt-18/`）とは分けて置く |
| fixtures/moqt-18-from-libraries/README.md | 仕様書 | 相手のライブラリがスクリプトの中で組み立てたバイト列。回線を流れたものとは分けて置く |
| fixtures/first-bytes/README.md | 仕様書 | 相手の開いたストリームが「何であるかを言った」と見なす時点の表。2つの実装が読む |
| fixtures/ のほかのディレクトリのREADME | 仕様書 | 2つ以上の実装が同じ場面で同じ動きをすることを確かめる共有の表（`joining-request-id`、`peer-subscribe`、`peer-fetch`、`published-fetch`、再生のバッファ、いちばん速く届いたものを覚える窓、音の継ぎ目の`playout`、答えを待つ時間の`request-answer-wait`、断りが言う「いつ出し直せるか」の読み方と付け方の`retry-interval`、チャットを購読し直したときにどの行を取り寄せるかの`chat-history`、Immutable Propertiesの読み方の`immutable-properties`）と、相手から採ったLOCとMSFの実データ（`loc-04`、`msf-01`） |
| packages/moqt/README.md | 仕様書 | ブラウザ用クライアントライブラリの範囲、層の決まり、公開API |
| packages/loc/README.md | 仕様書 | LOCのどの版を実装しているか、プロパティの番号、draftが言っていない所の読み方 |
| packages/msf/README.md | 仕様書 | MSFのカタログを読む・追う・出す部品の範囲と、draftに届いていない所 |
| packages/media/README.md | 仕様書 | 映像のトラックの受け手と送り手の決めごと、仮の値、既知の不足 |
| packages/chat/README.md | 仕様書 | チャットのトラックの参加者の部品の範囲、既定値と上書き、ペイロードの読み方 |
| packages/json/README.md | 仕様書 | トラックのペイロードをserde_jsonと同じに読むJSONの読み手 |
| packages/transcript/README.md | 仕様書 | 文字起こしのトラックの読み手の部品の範囲と仮の値 |
| crates/watch/README.md | 仕様書 | 端末の視聴者の範囲、何がどのファイルにあるか、未決の依存のライセンスとビルドに要るもの、確かめのスクリプト |
| crates/chat/README.md | 仕様書 | チャットの集め役の動き、旗と既定値、受け取りの上限、保存の形 |
| crates/chat-tracks/README.md | 仕様書 | チャットのトラックの名前とペイロードの形 |
| crates/agent/README.md | 仕様書 | エージェントが配信を読むCLIの使い方、旗、書き出す行の形、標準入力のコマンド、既知の不足 |
| crates/picture/README.md | 仕様書 | 映像の絵の復号、縮め方、PNGの書き出し、頼まれたときに何を復号するかの決めごとと依存のライセンス |
| crates/again/README.md | 仕様書 | 断りや終わりの後にいつ頼み直すかの規則と、既定の間隔 |
| crates/track-serving/README.md | 仕様書 | 自分のトラックを出してFETCHに答えるクライアントの部品と、その上限 |
| crates/speech/README.md | 仕様書 | 音を区間に切ってwhisper.cppで読む部品、whisper.cppのビルドに要るもの、モデルの選び方と測った数字 |
| crates/transcriber/README.md | 仕様書 | 文字起こし役の動き、旗と既定値、受け取りの上限 |
| crates/transcript-tracks/README.md | 仕様書 | 文字起こしのトラックの名前とペイロードの形 |
| fixtures/transcript-tracks/README.md | 仕様書 | 文字起こしのペイロードの共有の固定値 |
| fixtures/speech/README.md | 仕様書 | 文字起こしの照合に使う声の出どころ |
| fixtures/chat-tracks/README.md | 仕様書 | チャットのペイロードの共有の固定値。RustとTypeScriptが読む |
| interop/README.md | 運用手順書 | moqtailの相手の用意と、照合の走らせ方 |
| interop/peers/moqlivemock/README.md | 運用手順書 | moqlivemockの版、ビルド、当てているパッチ |
| .github/workflows/、scripts/ci/、mise.toml | コーディングポリシー | CIとローカルの検査。規約を検査で守る |
| .claude/agents/、.claude/skills/ | コーディングポリシー | レビュアーのsubagentと`/review-team-xmoq`。報告は`.claude/tmp/reviews/`に書かれ、コミットされない |

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
| 新しい文書を足した | README.md の一覧、このDocumentation Map |
| エージェントの進め方を変えた | docs/agent-guide.md、CLAUDE.md |
| draftの解釈や、本文に無い挙動を決めた・変えた | docs/moqt-implementation-notes.md。RustとTypeScriptの両方を同じPRで直す |
| 受信の上限やタイムアウトの値を決めた・変えた | docs/open-questions.md、docs/moqt-implementation-notes.md |
| moqtailから固定値を採った・採り直した | fixtures/moqt-18/README.md |
| ブラウザやライブラリから固定値を採った・採り直した | その固定値のディレクトリのREADME（fixtures/webtransport-close/README.md など） |
