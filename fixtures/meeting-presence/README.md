# Who is in a meeting

Every participant of a meeting publishes under `xmoq/meet/<room>/<id>`, and a reader learns who is there from one SUBSCRIBE_NAMESPACE for `xmoq/meet/<room>` (NAMESPACE and NAMESPACE_DONE, draft 18 §6.1; the meetings of [`docs/architecture.md`](../../docs/architecture.md)). Two implementations keep who is there by the same rule: `xmoq-viewer` with `xmoq-meeting-tracks`, which the reader for agents uses, and `@xmoq/meeting`, which a page uses. These rows pin it; the reasons are at that code.

`namespaces.tsv` pins the namespaces themselves: `xmoq-meeting-tracks` and `@xmoq/meeting` read it, and so does the room, whose seats give the namespaces of its rows `participant` and `seat-subscribe`.

## Format

Every file has one row per case, columns separated by tabs, no header. Every row ends with one line feed. `-` for none.

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

`namespaces.tsv`:

| Column | Content |
|---|---|
| 1 | Name of the row; unique |
| 2 | The room |
| 3 | The participant's id; `-` where the namespace is of no participant |
| 4 | Which namespace: `prefix` (`xmoq/meet/<room>`), `participant` (the one the participant publishes under), `seat-subscribe` (the one of a seat's token to subscribe) |
| 5 | Its fields, separated by spaces |
