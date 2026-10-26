# @xmoq/transcript

A reader of the transcripts over MoQ tracks: the track `speech` of `xmoq/transcript/<prefix>` that the transcriber ([`xmoq-transcriber`](../../crates/transcriber/README.md)) publishes, a piece of speech a Group. The demo's watching page shows it as captions (`showCaptions` of `@xmoq/transcript`).

| Export | What it is |
|---|---|
| `followTranscript(connection, options)` | Subscribes to the transcript of `listened` and hands on each piece once, `live` or `history`. `speech` is a followed track (`followTrack` of [`@xmoq/tracks`](../tracks/README.md)): when it begins, it fetches the latest `latest` pieces (`LATEST_PIECES`, 3, where not given), or with `lastHad`, the pieces after that one; a subscription made again fetches those after the newest piece had, 16 at the most, by an Absolute Joining FETCH (§10.12.2.1), as the agent does. A subscription that ends or is refused is made again after 1 s, or after the Retry Interval of a refusal, for as long as the connection lasts or until `stop`. `had()` is the newest piece that came |
| `keepFollowingTranscript(options)` | `followTranscript` across sessions: `connect` is called anew for each session, and the next `connect` is handed the GOAWAY that ended a session, so that the caller connects to its New Session URI with the certificate it had; the pause between sessions and what a GOAWAY (§10.4) does are `keepMoving`'s ([`@xmoq/moqt`](../moqt/README.md), "Asking again"). Each session catches up from the newest piece the session before had (`lastHad`) ([record](../../docs/experiments/2026-10-25-the-captions-follow-a-goaway.md)) |
| `showCaptions(element, options)` | A page's captions (`src/web`): each piece in `element` when it comes, for `shownMs` or until the next, on `keepFollowingTranscript`; a piece of the history older than `shownMs` is not shown. The page gives the element, the connection and the time |
| `transcriptNamespace`, `TRANSCRIPT_PREFIX`, `SPEECH_TRACK` | The names, as `crates/transcript-tracks/src/names.rs` has them |
| `Speech`, `Malformed` | A piece (`number`, `from`, `text`, `startUs`, `endUs`, `at`), and why a payload is no piece |

The payload is read as `xmoq-transcript-tracks` reads it, with the JSON reader of [`@xmoq/json`](../json/README.md). Both read the shared rows of [`fixtures/transcript-tracks/`](../../fixtures/transcript-tracks/README.md) in their tests. A payload that is no piece is passed over (`onMalformed`).

**Provisional values**, as `@xmoq/chat` has them for a room's lines: the SUBSCRIBE asks the relay to wait 30 s for the transcript to be published (RENDEZVOUS_TIMEOUT), the FETCH has 15 s, a piece's payload 64 KiB at the most, and the last 200 piece numbers are remembered so that a piece is handed on once. Both sides read them from [`fixtures/followed-track/`](../../fixtures/followed-track/README.md).

Layer 2 (`scripts/ci/check-layers.sh`): it depends on `@xmoq/moqt`, `@xmoq/json` and `@xmoq/tracks`, whose `followTrack` it follows `speech` by. The core (`src/core`) does no input or output.

## Checks

`test/payloads.test.ts` and `test/names.test.ts` read the shared tables. `test/follow.test.ts` checks the SUBSCRIBE it makes, and `test/keep-following.test.ts` where each session after a GOAWAY goes and that the next session catches up from the session before. The interop run `own relay: transcriber: ...` (`interop/transcriber.ts`) has a page read the transcript with `followTranscript` and be handed the same two pieces as a Rust reader, live.
