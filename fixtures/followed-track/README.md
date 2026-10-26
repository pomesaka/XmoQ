# The limits and waits of a followed track

A followed track is a kept track of one numbered item a Group, read with what came before (`docs/architecture.md`, "The roles of an endpoint"): the chat's `lines` and a transcript's `speech`. `limits.tsv` has what its readers wait for and hold, in Rust and in TypeScript. All the values are **provisional**, with their reasons at the constants. Each row is read by the tests of the part that holds its value, so that a value changed on one side alone fails a test:

| Rows | Rust | TypeScript |
|---|---|---|
| `rendezvous-ms`, `fetch-timeout-ms` | `xmoq-subscriber` (`follow.rs`) | `@xmoq/tracks` (`FOLLOW_RENDEZVOUS_MS`, `FOLLOW_FETCH_TIMEOUT_MS`) |
| `lines-payload-bytes`, `remembered-lines` | `xmoq-viewer-io` (`chat_tracks.rs`); `xmoq-subscriber` for `remembered-lines` | `@xmoq/chat` (`src/moqt/participant.ts`) |
| `speech-payload-bytes`, `remembered-pieces` | `xmoq-agent` (`attend.rs`); `xmoq-subscriber` for `remembered-pieces` | `@xmoq/transcript` (`src/moqt/follow.ts`) |

## Format

`limits.tsv`, one row per limit, three columns separated by tabs, no header. Every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | Name of the limit; unique |
| 2 | The Rust value; `-` where it has none |
| 3 | The TypeScript value; `-` where it has none |

Bytes are bytes, `-ms` is milliseconds, and a count is a count.

## The rows where the two differ

| Row | Why |
|---|---|
| `fetch-timeout-ms` | The Rust readers bound the Joining FETCH by its bytes (`fetch_most`) and by the session's wait for its answer, not by a time of its own |
| `remembered-lines`, `remembered-pieces` | The Rust readers hand an item on once by its number against the newest had (`Items`), and remember no set of numbers |
