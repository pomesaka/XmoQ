# When a client asks again

The pauses a client waits before it asks again: for a track on the same session (`same-session.tsv`), and for the next session (`next-session.tsv`). All the values are **provisional**.

| File | Read by |
|---|---|
| `same-session.tsv` | [`xmoq-again`](../../crates/again/README.md) (`AskAgain`) and `@xmoq/moqt` (`createAskAgain`, `packages/moqt/src/core/again.ts`) |
| `next-session.tsv` | `@xmoq/moqt` (`createNextSession`, the same file) and [`xmoq-again`](../../crates/again/README.md) (`NextSession`, which `xmoq-watch`, `xmoq-agent`, `xmoq-chat` and `xmoq-transcriber` go by) |

What a Retry Interval on the wire means in milliseconds is in [`fixtures/retry-interval/`](../retry-interval/README.md); these rows begin from the milliseconds.

Both files: no header, columns separated by tabs, every row ends with one line feed. Column 1 is the name of the row, unique in its file. The last column is what happens, in order, separated by spaces, each with the pause expected after it.

## `same-session.tsv`

A track asked for again on one session, after its subscription ended or was refused.

| Column | Content |
|---|---|
| 2 | The pause after an end, and after a refusal that names no time; `-` for the default, 1,000 ms |
| 3 | The shortest pause after a refusal that names a time; `-` for 100 ms |
| 4 | What the shortest pause grows to; `-` for 5,000 ms |
| 5 | The longest pause a refusal is let name; `-` for 30,000 ms |
| 6 | The steps |

| Step | What happened | The pause |
|---|---|---|
| `ended=N` | The subscription ended while the session goes on | `N` |
| `refused:-=N` | A refusal that names no time | `N` |
| `refused:M=N` | A refusal whose Retry Interval names `M` ms | `N` |
| `subscribed` | The track was subscribed to: the shortest pause begins again | none |

## `next-session.tsv`

The pause before the next try at a session.

| Column | Content |
|---|---|
| 2 | The pause, `F,M,W`: the first `F` ms, doubling from one try to the next up to `M` ms, and `F` again after a session that lasted `W` ms from its try (`-`: never) |
| 3 | The steps: `moved` (a GOAWAY ended the session), `ended` (it ended), `failed` (it could not be made, or broke), each as `ending@L=N`: the session lasted `L` ms from its try (`@L` left out: 0), and `N` is the pause before the next try |

**The pause is counted from the try before**: what is left of it since that try began, and 0 where the session lasted that long or longer. After a GOAWAY it is 0, unless the try before was itself one made at once after a GOAWAY; then it is counted as after any other end. A try made at once leaves the pause as it was. The roles have their own: the viewers `1000,1000,-`, the gatherer and the transcriber `1000,30000,60000` (`Pause` of `xmoq-again`).

How the rows are read: the first try is at 0. For each step the clock moves by `L`, the session ends, the pause is compared with `N`, and the clock moves by the pause; the next try begins there.
