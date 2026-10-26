# @xmoq/transcript

A reader of the transcripts over MoQ tracks: the track `speech` of `xmoq/transcript/<prefix>` that the transcriber ([`xmoq-transcriber`](../../crates/transcriber/README.md)) publishes, a piece of speech a Group. The demo's watching page shows it as captions (`showCaptions` of `@xmoq/transcript`).

| Export | What it is |
|---|---|
| `followTranscript(connection, options)` | Subscribes to the transcript of `listened` and hands on each piece once, `live` or `history`. When each subscription begins, it fetches the latest `latest` pieces with a Relative Joining FETCH (§10.12.2.1; `LATEST_PIECES`, 3, where not given; none where the SUBSCRIBE_OK says nothing was published). A subscription that ends or is refused is made again after 1 s, or after the Retry Interval of a refusal (`createAskAgain` of `@xmoq/moqt`), for as long as the connection lasts or until `stop` |
| `keepFollowingTranscript(options)` | `followTranscript` across sessions: `connect` is called anew for each session, and the next `connect` is handed the GOAWAY that ended a session, so that the caller connects to its New Session URI with the certificate it had; the pause between sessions and what a GOAWAY (§10.4) does are `keepMoving`'s ([`@xmoq/moqt`](../moqt/README.md), "Asking again"). Each session fetches the latest pieces anew, so the caller passes over a number it had ([record](../../docs/experiments/2026-10-25-the-captions-follow-a-goaway.md)) |
| `showCaptions(element, options)` | A page's captions (`src/web`): each piece in `element` when it comes, for `shownMs` or until the next, on `keepFollowingTranscript`; a piece of the history older than `shownMs` is not shown. The page gives the element, the connection and the time |
| `transcriptNamespace`, `TRANSCRIPT_PREFIX`, `SPEECH_TRACK` | The names, as `crates/transcript-tracks/src/names.rs` has them |
| `Speech`, `Malformed` | A piece (`number`, `from`, `text`, `startUs`, `endUs`, `at`), and why a payload is no piece |

The payload is read as `xmoq-transcript-tracks` reads it, with the JSON reader of [`@xmoq/json`](../json/README.md). Both read the shared rows of [`fixtures/transcript-tracks/`](../../fixtures/transcript-tracks/README.md) in their tests. A payload that is no piece is passed over (`onMalformed`).

**Provisional values**, as `@xmoq/chat` has them for a room's lines: the SUBSCRIBE asks the relay to wait 10 s for the transcript to be published (RENDEZVOUS_TIMEOUT), the FETCH has 15 s, a piece's payload 64 KiB at the most, and the last 200 piece numbers are remembered so that a piece is handed on once.

Layer 1 (`scripts/ci/check-layers.sh`): it depends on `@xmoq/moqt` and `@xmoq/json`. The core (`src/core`) does no input or output.

## Checks

`test/payloads.test.ts` reads the shared table. `test/follow.test.ts` checks the SUBSCRIBE it makes, and `test/keep-following.test.ts` where each session after a GOAWAY goes. The interop run `own relay: transcriber: ...` (`interop/transcriber.ts`) has a page read the transcript with `followTranscript` and be handed the same two pieces as a Rust reader, live.
