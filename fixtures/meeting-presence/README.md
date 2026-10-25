# Who is in a meeting

Every participant of a meeting publishes under `xmoq/meet/<room>/<id>`, and a reader learns who is there from one SUBSCRIBE_NAMESPACE for `xmoq/meet/<room>` (NAMESPACE and NAMESPACE_DONE, draft 18 §6.1; decided on 2026-10-25, `docs/scale-and-meetings.md`). Two implementations keep who is there by the same rule: `Presence` and `participant_of` of `xmoq-viewer` (`crates/viewer/src/presence.rs`, `crates/viewer/src/meeting.rs`), which the reader for agents uses, and `createPresence` and `participantOf` of `@xmoq/meeting` (`packages/meeting/src/core/`), which a page uses. These rows pin it; the reasons are in those files.

## Format

Both files have one row per case, columns separated by tabs, no header. Every row ends with one line feed. `-` for none.

`who-is-there.tsv`:

| Column | Content |
|---|---|
| 1 | Name of the row; unique |
| 2 | The most that are kept (0 is taken for 1) |
| 3 | What happens, in order, separated by spaces: `named:<id>` (a NAMESPACE), `done:<id>` (a NAMESPACE_DONE), `session-ended`, `settled` (the wait after the SUBSCRIBE_NAMESPACE of a new session was answered is over) |
| 4 | The changes of all of it, in order, separated by spaces: `came:<id>`, `left:<id>`, `too-many:<id>` |

`participant-of.tsv`:

| Column | Content |
|---|---|
| 1 | Name of the row; unique |
| 2 | The room |
| 3 | The fields of a namespace, separated by spaces |
| 4 | The participant it is the broadcast of |
