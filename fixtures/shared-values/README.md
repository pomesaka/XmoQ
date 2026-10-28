# Values that several parts hold alike

Values that a Rust part and a TypeScript package each hold as a constant of their own, where the sessions' table ([`fixtures/limits/`](../limits/README.md)) has no place for them: they belong to a client, to the relay or to the chat, not to the session. Each part named in a row reads `values.tsv` in its tests and compares the value with its constant. The values are those the constants had when the table was made; why each is what it is, is at the constant.

## Format

`values.tsv`, one row per value, three columns separated by tabs, no header. Every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | Name of the value; unique. `-ms` is milliseconds, a count is a count |
| 2 | The value |
| 3 | The parts that hold it, separated by commas: a Rust crate by its package name, a TypeScript package by its name. Each reads the rows that name it, and no fewer |

The rows and their constants:

| Row | Constants |
|---|---|
| `request-stream-open-wait-ms` | `STREAM_OPEN_WAIT_MS` of `crates/moqt-client`; `DEFAULT_OPEN_STREAM_TIMEOUT_MS` of `@xmoq/moqt` |
| `publish-done-peer-end-wait-ms` | `PUBLISH_DONE_ANSWER_WAIT` of `crates/moqt-client`; `DEFAULT_PEER_STREAM_END_TIMEOUT_MS` of `@xmoq/moqt` |
| `served-fetch-peer-end-wait-ms` | `FETCH_PEER_END_WAIT_MS` of `crates/relay-core`; `PEER_END_WAIT` of `crates/publisher`; `DEFAULT_PEER_STREAM_END_TIMEOUT_MS` of `@xmoq/moqt` |
| `chat-recent-lines` | `RECENT_LINES` of `crates/viewer`; `HISTORY_LINES` of `@xmoq/chat`. The tests of [`fixtures/chat-history/`](../chat-history/README.md) take it from here too |
| `video-hide-after-ms` | `HIDE_AFTER` of `crates/watch` (the default of `--hide-after-ms` of `xmoq-watch`); `DEFAULT_HIDE_AFTER_MS` of `@xmoq/media` (the default of `hideAfterMs` of `createPlayout`, and of `?hideAfterMs=` of the demo's pages) |
