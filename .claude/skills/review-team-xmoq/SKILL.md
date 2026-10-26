---
name: review-team-xmoq
description: XmoQのコードレビュー＆修正ループ。protocol-reviewer-xmoq・rust-reviewer-xmoq・ts-reviewer-xmoq・design-reviewer-xmoq・docs-reviewer-xmoqが並列レビューし、実装したdeckセッションが修正する。
argument-hint: "[レビュー対象のdeckセッション名。省略するとカレントの main..@]"
allowed-tools: Agent, Bash, Read, Edit, Write, SendMessage, ListAgents
---

# review-team-xmoq: レビュー＆修正ループ（Coordinator用）

あなたはCoordinator。subagent を調整してレビュー＆修正サイクルを回す。XmoQ ではCoordinatorはPM役のセッションで、修正は実装した deck セッションに頼む。

## ループ構成

| subagent | 担当 |
|---|---|
| protocol-reviewer-xmoq | draft 18 の本文との一致・固定値・Rust と TypeScript の一致・上限と後始末 |
| rust-reviewer-xmoq | Rust の書き方・エラー処理・テスト・プロジェクト固有ルール |
| ts-reviewer-xmoq | TypeScript の書き方・エラー処理・テスト・プロジェクト固有ルール |
| design-reviewer-xmoq | 凝集度・可読性・設計品質 |
| docs-reviewer-xmoq | ドキュメントの品質・網羅性・状態の書き方 |

fixer の subagent は置かない。実装した deck セッションが文脈を持っているので、そのセッションに直させる。

---

## Step 0: 対象を決める

`$ARGUMENTS` に deck セッション名があれば、そのワークスペースが対象になる。

```bash
"$CLAUDE_DECK_BIN" list
```

で `work_dir` を取り、次で作業コピーを取り込んでから change ID を得る。

```bash
jj -R <work_dir> log -r @ --no-graph -T 'change_id.short()'
```

revset は `main..<change ID>`、作業ディレクトリは `work_dir` になる。引数が無ければ revset は `main..@`、作業ディレクトリはカレント。

差分に含まれるパスで、起動するレビュアーを選ぶ。

| 差分に含まれるもの | 起動する |
|---|---|
| `crates/` か `Cargo.toml` | rust-reviewer-xmoq |
| `packages/`、`interop/`、`demo/` のどれか | ts-reviewer-xmoq |
| `crates/`、`packages/`、`fixtures/`、`interop/` のどれか | protocol-reviewer-xmoq |
| `crates/`、`packages/`、`fixtures/`、`interop/`、`demo/`、`scripts/`、`mise.toml`、`.github/` のどれか | design-reviewer-xmoq |
| `.md` か、上のどれか | docs-reviewer-xmoq |

`spikes/` だけの差分は、docs-reviewer-xmoq だけを起動する。

## Step 1: レビューラウンド（最大5ラウンド）

### 1a. 並列レビュー

Step 0 で選んだレビュアーを**同時に**起動する（並列 Agent 呼び出し）。prompt は全員同じ形にする。実装者の報告は渡さない。

```
ラウンドN のレビューをしてください。
revset: <revset>
作業ディレクトリ: <work_dir>
報告の書き出し先: <work_dir>/.claude/tmp/reviews/xmoq-<対象>-<レビュアー名>-r<N>.md
```

`.claude/tmp/` は `.gitignore` に入っているので、報告はコミットされない。

全てのレビュアーの結果が返るまで待つ。報告は書き出し先のファイルを Read して読む。返答の要約だけで判断しない。止まったレビュアーがいたら、その観点は未実施と明示する。

### 1b. 結果集約・分類

全レビュアーの結果をまとめ、以下に分類する。

- **非Nit**: 仕様不一致・固定値・実装間の不一致・層の依存・上限と後始末・ポリシー違反・整合性・凝集度・可読性・設計・ドキュメント（「提案」も含む）
- **Nit**: 明示的に「Nit:」と書かれているもの

protocol-reviewer の「推測」と付いた指摘は、Coordinator が draft の本文かコードを読んで確かめてから非Nit に入れる。確かめられなければ、未確認として利用者に見せる。

### 1c. 終了判定

| 状態 | 次のアクション |
|---|---|
| 非Nit = 0 | Step 2（完了）へ |
| 非Nit > 0 かつラウンド < 5 | 1d へ |
| ラウンド = 5 到達 | 残存指摘を表示して終了（人間に委ねる） |

### 1d. 実装したセッションに修正を頼む

SendMessage で、対象の deck セッションに `notify_when_idle: true` を付けて送る。

```
レビューの指摘です。全て修正してください。Nitも可能な範囲で一緒に直してください。
直さない指摘があれば、理由を書いて返してください。
修正の後、CLAUDE.md の「開発の流れ」の 1 に従って検査を実際に走らせ、その出力を含めて SendMessage で返してください。

## 非Nit指摘
<一覧>

## Nit指摘（任意）
<一覧>
```

対象が deck セッションでない（Coordinator 自身の変更）ときは、Coordinator が直接修正する。

修正の報告が届いたら、差分とテストの再実行で確かめてから Step 1a へ戻る。

---

## Step 2: 完了

```
## レビュー＆修正ループ完了

- ラウンド数: N
- 修正した指摘数: M件
- 直さなかった指摘と理由: （一覧、なければ「なし」）
- 未実施の観点: （止まったレビュアー、なければ「なし」）
- 残存するNit: （一覧、なければ「なし」）
```

この後、Coordinator がブランチを作って PR にする。

## Gotchas

- **subagent は毎回ゼロから起動**: 前ラウンドの文脈は持ち越さない。これは意図的（ゼロベースレビューのため）
- **並列起動**: Agent ツールを複数同時に呼ぶことでレビュアーが並列実行される
- **別のワークスペースの差分は、取り込んでからでないと古い**: `jj -R <work_dir>` のコマンドを 1 回走らせると、そのワークスペースの作業コピーが取り込まれる。レビューの前と、修正の報告を受けた後に毎回走らせる
- **待機の通知は完了の合図ではない**: 修正を頼んだセッションから報告が届かないときは、`claude-deck list` の `status` を見る
