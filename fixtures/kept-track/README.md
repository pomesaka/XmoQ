# What becomes of a kept track

A kept track is a track of another endpoint kept on a session: subscribed to, and asked for again when its subscription ends or a SUBSCRIBE for it is refused (`docs/architecture.md`, "The roles of an endpoint"). `kept-track.tsv` has what becomes of one after each thing that can happen to it: asked for again after a pause, given up on the session, or the session closed. [`xmoq-subscriber`](../../crates/subscriber/README.md) (`kept::decide`, which `KeptTrack` and the gatherer of the chat go by) reads it in its test, and `@xmoq/tracks` is to read the rows of no limit on the times and of a required track. The pauses are the default rule of [`fixtures/ask-again/same-session.tsv`](../ask-again/README.md), all **provisional**.

## `kept-track.tsv`

No header, six columns separated by tabs, every row ends with one line feed.

| Column | Content |
|---|---|
| 1 | The name of the row, unique |
| 2 | What the track does where its SUBSCRIBE could not be sent or was not answered: `again` (asked for again) or `give-up` (not asked for again on this session; the next session asks for it). It bears on `not-sent` and `not-answered` alone |
| 3 | What happens, in order, separated by spaces (below) |
| 4 | What becomes of the track after each, in order: `subscribed`, `again:<ms>` (asked for again after that pause), `given-up`, `closed:<code>` (the session is closed with that Session Termination Error Code, §3.5) |
| 5 | The most times in a row it is asked again without a subscription between, or `-` for no limit. An end (`malformed` among them), a refusal and a failure that is asked again count; once the times are reached, the next is `given-up` (the gatherer asks again when the participant's namespace comes again, the transcriber its catalog). `0`: never asked again. The times it is found `malformed` in a row are held to the same limit, and are not counted anew by `subscribed`, only by another end |
| 6 | `required`, or `optional` for a track its publisher need not publish (the gatherer's `reacted`): a refusal with `DOES_NOT_EXIST` (`0x10`) gives it up |

What can happen, column 3:

| Step | What it is |
|---|---|
| `subscribed` | Its SUBSCRIBE was answered with SUBSCRIBE_OK; the shortest pause after a refusal begins anew |
| `publish-done` | PUBLISH_DONE on its stream (§10.11) |
| `fin` | The publisher finished its direction of the stream without PUBLISH_DONE, which travels on that direction and can no longer come: the request is over (§3.3.2 cancels by "resetting or sending STOP_SENDING", and §5.1 has PUBLISH_DONE end a subscription; the row on a requester that finishes its direction in [the implementation notes](../../docs/moqt-implementation-notes.md) has the publisher's FIN of a SUBSCRIBE as an ending) |
| `reset` | The publisher reset its direction of the stream (§3.3.2) |
| `lost` | The connection went while the stream was read |
| `goaway` | A GOAWAY on its stream (§10.4: "SHOULD re-issue that specific request"); it is asked for again on this session, where the rest of the session is |
| `malformed` | The track was found malformed (§2.4.2: "it MUST cancel any corresponding subscription"); its stream is reset with `MALFORMED_TRACK`, and it is asked for again, as after any end |
| `refused:<code>` | Its SUBSCRIBE was refused with REQUEST_ERROR of that Error Code (hexadecimal), with no time named |
| `refused:<code>:<ms>` | The same, with the wait its Retry Interval names in milliseconds (`0`: at once). What a Retry Interval on the wire is in milliseconds is in [`fixtures/retry-interval/`](../retry-interval/README.md) |
| `not-sent` | Its SUBSCRIBE could not be sent: no stream in time, or one that broke |
| `not-answered` | Its SUBSCRIBE was not answered in time ([`fixtures/request-answer-wait/`](../request-answer-wait/README.md)) |
| `violation` | A message on its stream, or the answer to its SUBSCRIBE, broke the protocol: the session is closed with `PROTOCOL_VIOLATION` (`0x3`) |
