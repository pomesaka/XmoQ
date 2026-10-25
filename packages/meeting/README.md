# @xmoq/meeting

A participant of a meeting in a page. Every participant publishes a broadcast of its own under `xmoq/meet/<room>/<id>`, and finds the others with one SUBSCRIBE_NAMESPACE for `xmoq/meet/<room>` (decided on 2026-10-25, [design](../../docs/scale-and-meetings.md)). This package receives: it reads each one's catalog, and receives the sound of all of them and the small video of the first 8 who came. Publishing the page's own broadcast is done with [`@xmoq/media`](../media/README.md) and [`@xmoq/msf`](../msf/README.md), as any broadcast is, and playing what is received is the caller's.

| Export | What it is |
|---|---|
| `createMeeting(options)` | A meeting of `room` as `self`. `attend(connection, signal)` takes part on one session until it ends; called again with the next session, it keeps who was there. Each decoded frame and piece of sound is handed to `onFrame(id, …)` and `onAudio(id, …)`, and the callee closes it. `onEvent` is told who came and left, and which tracks began and stopped |
| `DEFAULT_MOST_WATCHED`, `DEFAULT_MOST_OTHERS` | 8 videos at once, and 48 others: the room gives no token past 49 participants, and a page keeps 48 others besides |
| `meetingPrefix`, `participantOf`, `MEET` | The names, as `crates/viewer/src/meeting.rs` has them |
| `createPresence`, `SETTLE_MS`, `watchedOf` | Who is there in the order they came, across sessions, as `Presence` of `crates/viewer/src/presence.rs` has it; and whose video is received |
| `chooseTracks` | The video with the fewest pixels, and the first sound, of a catalog's tracks in its own namespace |

What it does, in order:

- NAMESPACE of `xmoq/meet/<room>/<id>` (§6.1): `id` came, unless it is `self`, or 48 others are there already (`too-many`). Its catalog is followed, with a RENDEZVOUS_TIMEOUT of 10 s, since a participant announces before its catalog is published; a catalog that ends or fails is followed again 2 s later.
- With a catalog: the sound of everybody is subscribed to, and the smallest video of the first `mostWatched` who are there, in the order they came. A subscription that ends while its participant is there is made again (`keepTrackOnSession` of `@xmoq/media`). A catalog that changes the chosen tracks has them subscribed anew.
- NAMESPACE_DONE: the participant left. Its subscriptions are cancelled, and its place among the watched goes to the next who came.
- A new session: whoever was there is followed on it at once, and those the relay does not tell of again within `SETTLE_MS` (2 s) have left.

**Provisional values**: the 10 s of the catalog's RENDEZVOUS_TIMEOUT and the 2 s before a catalog is followed again, as `@xmoq/transcript` has them, and `SETTLE_MS`. A sound track without `samplerate` is taken for 48,000 Hz, and one without `channelConfig` for one channel.

Layer 3 (`scripts/ci/check-layers.sh`): it depends on `@xmoq/moqt`, `@xmoq/msf` and `@xmoq/media`. The core (`src/core`) does no input or output.

## Checks

`test/presence.test.ts` reads the shared rows of [`fixtures/meeting-presence/`](../../fixtures/meeting-presence/README.md), which `crates/viewer` reads as well. `test/tracks.test.ts` checks which tracks of a catalog are chosen and the limits. The interop run `own relay: meeting: …` (`interop/meeting-pages.ts`) has three pages meet, one of them with one video, and one leave; not through moqtail's relay, which brings no catalog of a page ([record](../../docs/experiments/2026-10-25-a-meeting-of-pages.md)).
